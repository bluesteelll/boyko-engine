// `Layout` is always needed: the public `allocate_*` signatures use it. The
// global-allocator `alloc`/`dealloc` are pulled in only for the fallback
// backing (Miri / wasm32 / exotic targets); the OS-syscall arms do not use
// them.
use std::alloc::Layout;
#[cfg(any(miri, not(any(windows, unix))))]
use std::alloc::{alloc, dealloc};
use std::cell::{Cell, UnsafeCell};
use std::ptr::NonNull;

use crate::ecs::constants::{
    ARENA_COMMIT_GRANULE, ARENA_MAX_SLAB, ARENA_MIN_SLAB, CACHE_LINE_SIZE, DEFAULT_ARENA_RESERVE,
};
use crate::ecs::memory::free_mem_block::{MemFreeBlock, MemFreeBlockMaster};

/// Cold-path `align_up` with overflow checking (R2-N2). The unchecked
/// `utils::align_up` stays on hot/setup paths where inputs are bounded; every
/// cold-path rounding (reservation sizing, slab sizing) goes through this so
/// pathological inputs (`with_reserve(usize::MAX, ..)`, degenerate layouts)
/// panic loudly instead of wrapping.
fn checked_align_up(value: usize, granule: usize) -> usize {
    debug_assert!(granule.is_power_of_two(), "granule must be a power of two");
    value
        .checked_add(granule - 1)
        .expect("Arena: align_up overflow (value too close to usize::MAX)")
        & !(granule - 1)
}

/// Pure slab-sizing policy for one arena growth event (Phase X.F, D4).
///
/// Inputs are all granule-aligned by construction (`committed` starts at a
/// granule multiple and only ever advances by granule-aligned steps; `needed`
/// is `align_up`'d by the caller; `os_reserve` is the granule-rounded
/// reservation length). The result is therefore granule-aligned too:
/// `clamp`/`max` of aligned values is aligned, and `min(.., os_reserve -
/// committed)` subtracts two aligned values (R2-W1 induction).
///
/// Returns 0 exactly when the frontier is at the reservation ceiling
/// (`committed == os_reserve`) or the remaining headroom is smaller than any
/// positive aligned step would be — the caller's sufficiency check (GROW1)
/// folds that case into the single reserve-exhausted panic surface.
fn grow_step(committed: usize, needed: usize, os_reserve: usize) -> usize {
    debug_assert!(
        committed <= os_reserve,
        "grow_step: committed {committed} past os_reserve {os_reserve}"
    );
    // Geometric doubling (clamped to [MIN_SLAB, MAX_SLAB]) unless the request
    // itself is larger — a single huge request must be a single event, so
    // `needed` is deliberately NOT clamped by ARENA_MAX_SLAB.
    let step = committed.clamp(ARENA_MIN_SLAB, ARENA_MAX_SLAB).max(needed);
    // Never step past the reservation ceiling.
    step.min(os_reserve - committed)
}

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
    pub const PAGE_NOACCESS: u32 = 0x01;
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
/// # Backing store (Phase X.C reserve + Phase X.F lazy frontier growth)
///
/// The arena holds ONE contiguous OS-level virtual-address reservation
/// (`VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on Windows,
/// `mmap(PROT_NONE)` on Unix) of `os_reserve = align_up(reserve,
/// ARENA_COMMIT_GRANULE)` bytes. Pages are committed lazily in slabs at the
/// frontier (`commit_frontier`) when the free-block tracker cannot satisfy a
/// request (`grow_then_retry`); physical pages then fault in demand-zero on
/// first touch. Growth NEVER moves the base pointer: previously returned
/// blocks are never remapped or relocated, so every `NonNull` handed out
/// stays valid for the arena's lifetime. Miri / wasm32 / exotic targets fall
/// back to one eager global-allocator allocation of the full reserve (commit
/// is a watermark bump there). The deallocator in `Drop` is chosen per
/// cfg-arm to match the acquisition (audit finding M-001: free exactly once,
/// with the matching deallocator). See [`Backing`], `with_reserve`, and
/// `grow_then_retry`.
///
/// # Supported alignment (R3-1)
///
/// `allocate_*` aligns OFFSETS; the pointer alignment a caller observes is
/// the base alignment ∧ the offset alignment. The per-arm base guarantees
/// are: Windows reservation 64 KiB, Unix `mmap` 4 KiB, fallback
/// `CACHE_LINE_SIZE` (64 B). The arena's documented + debug-asserted
/// supported alignment bound is therefore **`CACHE_LINE_SIZE` (64 B)** — the
/// honest cross-arm bound (production's largest request is 32,
/// `SIMD_BUFFER_ALIGN`).
pub struct Arena {
    /// Backing buffer pointer. Acquired in `with_reserve` (write-once base of
    /// the single reservation), freed in `Drop`. Never changes afterwards —
    /// growth only commits fresh pages inside the same reservation.
    ptr: NonNull<u8>,

    /// Logical allocation ceiling in bytes (cache-line aligned) — what
    /// `capacity()` reports and what the reserve-exhausted panic is measured
    /// against. Plain immutable `usize`: it never changes after construction.
    /// The OS-level reservation length is the granule-rounded `os_reserve`,
    /// recomputed where needed (1 ALU op on the cold path).
    reserve: usize,

    /// Commit frontier in bytes: `[0, committed)` is committed (readable/
    /// writable), `[committed, os_reserve)` is reserved-only. Granule-aligned,
    /// monotonically non-decreasing, `<= os_reserve`. Interior-mutable because
    /// the cold grow path advances it through `&self`; the M-003
    /// single-threaded argument covers the `Cell` (no concurrent reader can
    /// exist while grow mutates — `Arena` is `!Send`/`!Sync`).
    committed: Cell<usize>,

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
    /// Reserves a contiguous `reserve`-byte address range (rounded up to
    /// cache-line granularity for the logical ceiling and to
    /// `ARENA_COMMIT_GRANULE` for the OS mapping) and eagerly commits the
    /// first `initial_commit` bytes (rounded up to the granule, clamped to
    /// the reservation). Panics on acquisition failure or `reserve == 0`.
    ///
    /// # Backing store (Phase X.F)
    ///
    /// The acquisition is reserve-ONLY: `VirtualAlloc(MEM_RESERVE,
    /// PAGE_NOACCESS)` on Windows, `mmap(PROT_NONE, MAP_PRIVATE |
    /// MAP_ANONYMOUS)` on Unix — no commit charge is paid for the reserved
    /// range (a `PROT_NONE` mapping is not private-writable, so even
    /// overcommit-mode-2 Linux does not account it). Commit happens in slabs
    /// at the frontier: eagerly here for `initial_commit > 0`, lazily in
    /// `grow_then_retry` otherwise. Miri / wasm32 / exotic targets fall back
    /// to one eager `std::alloc::alloc` of the full (granule-rounded) reserve
    /// — commit is a pure watermark bump there.
    ///
    /// # Free-list state machine
    ///
    /// `initial_commit == 0` seeds the free-block tracker EMPTY (the first
    /// allocation takes the cold grow path); `initial_commit > 0` seeds
    /// `[0, min(committed, reserve))` exactly as the pre-X.F eager arena did.
    pub fn with_reserve(reserve: usize, initial_commit: usize) -> Self {
        // R2-N3: a zero-length reservation is degenerate (zero-size
        // VirtualAlloc/mmap edge); reject it loudly on this cold path.
        assert!(reserve > 0, "Arena reserve must be non-zero");

        // Logical ceiling: cache-line rounded (what `capacity()` reports, the
        // span the free list may ever offer). OS mapping length: additionally
        // granule-rounded so a frontier commit can never overrun the kernel's
        // page-rounded mapping. Both roundings are checked (R2-N2).
        let reserve = checked_align_up(reserve, CACHE_LINE_SIZE);
        let os_reserve = checked_align_up(reserve, ARENA_COMMIT_GRANULE);

        // Review F1: every offset later fed to `ptr.add(..)` must fit `isize`
        // (the `pointer::add` contract; allocated objects are bounded by
        // `isize::MAX`). The fallback arm gets this from
        // `Layout::from_size_align`, but the syscall arms pass `os_reserve`
        // straight to VirtualAlloc/mmap — on 32-bit hosts with a large
        // address split a ~2.5-3 GiB reservation could otherwise succeed and
        // make `base + block.start` UB. Cold path: one compare.
        assert!(
            os_reserve <= isize::MAX as usize,
            "Arena reserve {os_reserve} B exceeds isize::MAX (pointer-offset contract)"
        );

        // Acquire `(ptr, backing)` from the per-target backing store. Exactly
        // one of these three arms is compiled (the cfg matrix is total and
        // disjoint by construction).
        #[cfg(all(not(miri), windows))]
        let (ptr, backing) = {
            // SAFETY (W-RES): a NULL base lets the OS choose the address;
            // `dwSize` is `os_reserve` (> 0 because `reserve > 0` is asserted
            // above and `align_up` only grows). `MEM_RESERVE` + `PAGE_NOACCESS`
            // reserves address space WITHOUT committing — no charge, no
            // access until `commit_frontier` re-protects slabs. The result is
            // null-checked by `NonNull::new` before any use.
            let raw = unsafe {
                win::VirtualAlloc(
                    core::ptr::null_mut(),
                    os_reserve,
                    win::MEM_RESERVE,
                    win::PAGE_NOACCESS,
                )
            };
            let ptr = NonNull::new(raw as *mut u8)
                .expect("VirtualAlloc failed to reserve the arena address space");
            (ptr, Backing {})
        };

        #[cfg(all(not(miri), unix, not(windows)))]
        let (ptr, backing) = {
            // SAFETY (U-RES): a NULL base lets the OS choose the address;
            // `len` is `os_reserve` (> 0, see the Windows arm). `PROT_NONE` +
            // `MAP_PRIVATE | MAP_ANONYMOUS` reserves a private anonymous
            // range with no access (and no overcommit accounting) until
            // `commit_frontier` mprotects slabs RW (fd = -1, offset = 0). The
            // return is validated below.
            let raw = unsafe {
                libc::mmap(
                    core::ptr::null_mut(),
                    os_reserve,
                    libc::PROT_NONE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            // `MAP_FAILED` is `(void*)-1` — NON-NULL — so it must be checked
            // BEFORE `NonNull::new`, which would otherwise wrongly accept it.
            assert!(raw != libc::MAP_FAILED, "mmap failed to reserve the arena address space");
            let ptr = NonNull::new(raw as *mut u8).expect("mmap returned null");
            // `map_len` stores the FULL reservation length passed to `mmap`
            // (NOT the committed frontier) — `munmap` in `Drop` must replay
            // the original mapping length. Single assignment site.
            (ptr, Backing { map_len: os_reserve })
        };

        #[cfg(any(miri, not(any(windows, unix))))]
        let (ptr, backing) = {
            // R3-3: the fallback backing is sized to `os_reserve` (not the
            // cache-line-rounded reserve) for watermark/backing uniformity
            // with the syscall arms.
            let layout = Layout::from_size_align(os_reserve, CACHE_LINE_SIZE)
                .expect("Invalid layout for arena");

            // SAFETY (F-RES): layout has non-zero size (`reserve > 0` asserted
            // above, `align_up` only grows) and alignment is a valid power of
            // two (`CACHE_LINE_SIZE` is 64).
            let raw = unsafe { alloc(layout) };
            let ptr = NonNull::new(raw).expect("Failed to allocate memory for arena");
            (ptr, Backing { layout })
        };

        // R2-W1: the eager frontier rounds UP to the granule and clamps to
        // the reservation, so every later `commit_frontier` base stays
        // granule-aligned by induction.
        let committed = checked_align_up(initial_commit, ARENA_COMMIT_GRANULE).min(os_reserve);

        let arena = Self {
            ptr,
            reserve,
            committed: Cell::new(committed),
            backing,
            // O4: the free-block tracker spans at most the logical
            // cache-line-rounded reserve, never the (potentially larger)
            // granule-rounded mapping length.
            free_blocks: UnsafeCell::new(if committed > 0 {
                MemFreeBlockMaster::new_init(committed.min(reserve))
            } else {
                MemFreeBlockMaster::new()
            }),
        };

        if committed > 0 {
            // Eager arm (`with_capacity` back-compat): commit `[0, committed)`
            // through the same per-arm primitive the grow path uses.
            arena.commit_frontier(0, committed);
        }

        arena
    }

    /// Allocates a fresh EAGER arena: reserve == commit == `capacity`
    /// (rounded up to cache-line granularity). Growth past `capacity` is
    /// impossible — this keeps the exact historical "this much usable memory,
    /// then panic" semantics that fixtures and tests rely on (Phase X.F D3).
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_reserve(capacity, capacity)
    }

    /// Reserves the default arena address range (`DEFAULT_ARENA_RESERVE` —
    /// 4 GiB on 64-bit OS-syscall arms) with ZERO initial commit: no commit
    /// syscall, no commit charge, free list seeded empty. The first pool
    /// allocation takes the cold grow path and commits the first slab.
    pub fn new() -> Self {
        Self::with_reserve(DEFAULT_ARENA_RESERVE, 0)
    }

    /// Logical allocation ceiling in bytes (cache-line aligned) — the
    /// RESERVE, i.e. what the reserve-exhausted panic is measured against.
    ///
    /// Phase X.F: default worlds report the full multi-GB reservation
    /// (4 GiB), of which almost nothing is resident — use [`committed()`]
    /// for resident-memory expectations.
    ///
    /// [`committed()`]: Arena::committed
    #[inline]
    pub fn capacity(&self) -> usize {
        self.reserve
    }

    /// Commit frontier in bytes: how much of the reservation has been made
    /// readable/writable (granule-aligned, monotonic, `<= os_reserve`).
    /// Diagnostics/tests accessor — resident memory is bounded by this, not
    /// by `capacity()`.
    #[inline]
    pub fn committed(&self) -> usize {
        self.committed.get()
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
        // Fast sanity check: a request that can never fit even an EMPTY
        // arena. Sub-reserve requests that still cannot be satisfied
        // (alignment slack, fragmentation at the ceiling) are handled by the
        // cold exhaustion panic in `grow_then_retry` with full diagnostics.
        debug_assert!(
            layout.size() <= self.reserve,
            "allocation request {} B exceeds arena reserve {} B",
            layout.size(),
            self.reserve
        );
        // R3-1: the arena's supported alignment bound is CACHE_LINE_SIZE
        // (64 B) — the honest cross-arm base-alignment guarantee (see the
        // type-level doc). A contract check, not a hot-path branch: compiled
        // out in release.
        debug_assert!(
            layout.align() <= CACHE_LINE_SIZE,
            "allocation align {} exceeds the arena's supported alignment bound {} B",
            layout.align(),
            CACHE_LINE_SIZE
        );

        match self.allocate_from_free_blocks(layout) {
            Some(ptr) => ptr,
            // Phase X.F: no suitable free block — commit more of the
            // reservation at the frontier and retry (cold, out-of-line).
            None => self.grow_then_retry(layout),
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

        // SAFETY: `block.start` is within `[0, self.reserve)` and
        // `block.start + size <= self.reserve` — that is the contract of
        // `allocate_aligned`. The resulting pointer is therefore inside the
        // single allocated object that `self.ptr` heads. Phase X.F: block
        // offsets are < reserve AND below the committed frontier (free-list
        // ranges are only ever seeded from committed space — `with_reserve`
        // seeds `[0, min(committed, reserve))`, `grow_then_retry` inserts
        // `[lo, hi)` with `hi <= committed`), so the pointer is into
        // committed, RW memory.
        let ptr = unsafe { self.ptr.as_ptr().add(block.start) };
        NonNull::new(ptr)
    }

    /// Cold growth path (Phase X.F): commits more of the reservation at the
    /// frontier, offers the fresh range to the free-block tracker, and
    /// retries the allocation. Called by `allocate_layout` exclusively, AFTER
    /// the ALLOC1 guards and only when best-fit failed — growth therefore
    /// inherits the ALLOC1 restriction (apply-window / setup only) for free.
    ///
    /// # GROW1 (why the retry cannot fail)
    ///
    /// Grow runs only after best-fit failed: every free block is
    /// `< required_size = size + align - 1`; in particular the tail block
    /// ending at `lo` has `tail < required_size`. Grow commits `step` bytes
    /// and offers `[lo, hi)` (`usable = hi - lo` bytes), which left-coalesces
    /// with the tail into a single free block of EXACTLY `tail + usable`
    /// bytes ending at `hi` (right-coalescing is impossible at the frontier:
    /// all previously offered space is a subset of `[0, lo)`). Grow proceeds
    /// only when `tail + usable >= required_size` — checked BEFORE any state
    /// change against the ACTUAL `required_size`, never against the
    /// granule-rounded `needed` (false exhaustion) — else it panics as
    /// legitimate reserve exhaustion with zero state change. Therefore
    /// post-grow a free block `>= required_size` EXISTS, the retry's best-fit
    /// returns `Some`, and a retry-`None` is a GENUINE logic bug (panic,
    /// debug AND release).
    #[cold]
    #[inline(never)]
    fn grow_then_retry(&self, layout: Layout) -> NonNull<u8> {
        let size = layout.size();
        let align = layout.align();

        // Step 0 (D7 guard): a zero-size request can never be satisfied by
        // growth (`allocate_aligned` returns `None` for size 0) — panic
        // immediately, do not commit anything.
        if size == 0 {
            panic!("Arena: zero-size allocation request cannot be satisfied");
        }

        // GROW1 comparand. COUPLING: this is EXACTLY the best-fit search size
        // `MemFreeBlockMaster::allocate_aligned` uses (`size + align - 1`) —
        // if that formula changes, update this in lockstep (R3-3). Checked:
        // free on a cold path, airtight against pathological layouts (R2-N2).
        let required_size = size
            .checked_add(align - 1)
            .expect("Arena: allocation size + align - 1 overflows usize");

        // Granule-rounded request — a STEP-SIZING input ONLY, never the
        // sufficiency comparand (R2 C1: a `needed`-based check would falsely
        // exhaust satisfiable requests).
        let needed = checked_align_up(required_size, ARENA_COMMIT_GRANULE);

        let os_reserve = checked_align_up(self.reserve, ARENA_COMMIT_GRANULE);
        let old = self.committed.get();
        let step = grow_step(old, needed, os_reserve);

        // The slab is committed up to `old + step`, but only the part below
        // the LOGICAL reserve is ever offered to the free list (the granule
        // tail past `reserve` stays committed-but-unoffered, <= 60 KiB).
        let lo = old.min(self.reserve);
        let hi = (old + step).min(self.reserve);
        let usable = hi - lo;

        // Length of the free block ending exactly at `lo`, if any — the only
        // block the frontier insert can coalesce with. Read-only probe; the
        // shared borrow is taken and dropped HERE, before the exclusive
        // borrows below (preserves the verified-sound borrow structure).
        let tail = {
            // SAFETY: M-003 — the arena is single-threaded (`!Send`/`!Sync`)
            // and no other reference into `free_blocks` exists at this point
            // (`allocate_from_free_blocks` dropped its `&mut` before
            // returning `None`). The `&` is dropped at the end of this block.
            let free_blocks = unsafe { &*self.free_blocks.get() };
            free_blocks.free_block_len_ending_at(lo)
        };

        // R2 C1: sufficiency check BEFORE any state change, against the
        // ACTUAL best-fit criterion. Single exhaustion surface: the
        // `step == 0` (frontier at ceiling) case folds into this check
        // (`usable == 0` and `tail < required_size` because best-fit just
        // failed). `checked_add` per R2-N2 (cold path, free).
        let available = tail
            .checked_add(usable)
            .expect("Arena: free tail + growable bytes overflows usize");
        if available < required_size {
            panic!(
                "Arena reserve exhausted: request {size} B (align {align}, required \
                 {required_size} B contiguous) cannot be satisfied: free tail {tail} B + \
                 growable {usable} B = {available} B; committed {old} B of reserve {} B",
                self.reserve
            );
        }

        // Corollaries of a passing check after a failed best-fit (R2 C1) —
        // proven, not runtime branches:
        //   best-fit failed => tail < required_size => usable > 0 => step > 0.
        debug_assert!(
            tail < required_size,
            "grow_then_retry entered although a sufficient free tail existed"
        );
        debug_assert!(usable > 0, "frontier insert would be empty");
        debug_assert!(step > 0, "commit step must be positive when usable > 0");
        debug_assert!(
            old + step <= os_reserve,
            "commit step overruns the OS reservation"
        );

        self.commit_frontier(old, old + step);
        // Monotonic advance: `step > 0` and `old + step <= os_reserve` per
        // the corollaries above.
        self.committed.set(old + step);

        {
            // SAFETY: M-003 — same single-threaded argument as above; this
            // exclusive `&mut` is the only live reference into `free_blocks`
            // and is dropped at the end of this block, before the retry takes
            // its own fresh borrow. (`MemFreeBlockMaster::insert` allocates
            // from the GLOBAL allocator — no re-entrancy into the arena.)
            let free_blocks = unsafe { &mut *self.free_blocks.get() };
            // Left-coalesces with the free tail at `lo` automatically;
            // right-coalescing is impossible at the frontier (GROW1).
            free_blocks.insert(MemFreeBlock::new(lo, hi));
        }

        match self.allocate_from_free_blocks(layout) {
            Some(ptr) => ptr,
            // GROW1 proves a covering block exists — reaching this arm is a
            // GENUINE logic bug, reported in debug AND release.
            None => panic!(
                "Arena grow logic bug (GROW1 violated): post-grow retry found no block for \
                 {size} B (align {align}); committed {} B of reserve {} B",
                self.committed.get(),
                self.reserve
            ),
        }
    }

    /// Commits (makes readable/writable) the byte range `[old, new)` of the
    /// reservation — the per-arm growth primitive (Phase X.F D1). `old` and
    /// `new` are granule-aligned and `new <= os_reserve` (W1 induction,
    /// debug-asserted); the granule is a multiple of the commit/`mprotect`
    /// page size on every supported target.
    #[cold]
    fn commit_frontier(&self, old: usize, new: usize) {
        debug_assert!(new > old, "commit_frontier: empty or backwards range");
        debug_assert!(
            old.is_multiple_of(ARENA_COMMIT_GRANULE) && new.is_multiple_of(ARENA_COMMIT_GRANULE),
            "commit_frontier: range [{old}, {new}) not granule-aligned"
        );
        debug_assert!(
            new <= checked_align_up(self.reserve, ARENA_COMMIT_GRANULE),
            "commit_frontier: range end {new} overruns the OS reservation"
        );

        #[cfg(all(not(miri), windows))]
        {
            // SAFETY (W-CMT): `self.ptr + old .. self.ptr + new` lies inside
            // our own reservation (`new <= os_reserve`, asserted above), is
            // granule-aligned (=> page-aligned), and re-committing an already
            // committed page is documented-idempotent for `VirtualAlloc`
            // (contents and addresses of committed pages are untouched). The
            // result is null-checked: NULL here means the OS commit charge is
            // exhausted — the loud failure surface for genuine OOM.
            let raw = unsafe {
                win::VirtualAlloc(
                    self.ptr.as_ptr().add(old) as *mut core::ffi::c_void,
                    new - old,
                    win::MEM_COMMIT,
                    win::PAGE_READWRITE,
                )
            };
            assert!(
                !raw.is_null(),
                "VirtualAlloc(MEM_COMMIT) failed committing [{old}, {new}) of the arena \
                 reservation (commit charge exhausted?)"
            );
        }

        #[cfg(all(not(miri), unix, not(windows)))]
        {
            // SAFETY (U-CMT): `self.ptr + old .. self.ptr + new` lies inside
            // our own mapping (`new <= os_reserve == map_len`, asserted
            // above) and is granule-aligned (the granule is a multiple of the
            // page size), so `mprotect` gets a page-aligned base and length.
            // The return is checked: ENOMEM here is the overcommit-mode-2
            // failure surface (the commit charge is accounted when pages
            // become private-writable).
            let ret = unsafe {
                libc::mprotect(
                    self.ptr.as_ptr().add(old) as *mut core::ffi::c_void,
                    new - old,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            assert!(
                ret == 0,
                "mprotect(PROT_READ | PROT_WRITE) failed committing [{old}, {new}) of the \
                 arena mapping (ENOMEM = overcommit limit)"
            );
        }

        // Fallback arm (Miri / wasm32 / exotic): no-op — the whole reserve
        // was eagerly allocated readable/writable in `with_reserve`; commit
        // is a pure watermark bump (the `committed.set` in the caller).
    }

    /// Convenience wrapper around `allocate_layout` for a Sized type `T`.
    pub fn allocate<T: Sized>(&self) -> NonNull<T> {
        let layout = Layout::new::<T>();
        let ptr = self.allocate_layout(layout);
        ptr.cast()
    }
}

impl Drop for Arena {
    /// Releases the backing reservation (audit finding M-001 — without this,
    /// every `Arena::new()` leaks its address-space reservation plus any
    /// committed slabs).
    ///
    /// Note: type-erased component `Drop` is **not** invoked here. The
    /// components live inside this buffer, but their drop is the
    /// responsibility of `ComponentPool::Drop` (tracked separately as a
    /// Phase 1b refactor; today, components with non-trivial `Drop` will
    /// still leak their inner heap data when their `ComponentPool` is
    /// destroyed, which is a known limitation).
    fn drop(&mut self) {
        // Release with the deallocator matching the acquisition arm in
        // `with_reserve` (M-001). Exactly one arm is compiled.
        #[cfg(all(not(miri), windows))]
        {
            // SAFETY: `self.ptr` is the exact base returned by `VirtualAlloc`
            // in `with_reserve`, freed exactly once (`Drop` runs once).
            // `MEM_RELEASE` requires `dwSize == 0` together with the original
            // base address — both are satisfied. `VirtualFree(MEM_RELEASE)`
            // releases the ENTIRE reservation regardless of commit state
            // (partially-committed reservations are released in full — Phase
            // X.F). After this point `self.ptr` must not be used, which is
            // fine because `Drop` is the last call on `self`.
            let ok =
                unsafe { win::VirtualFree(self.ptr.as_ptr() as *mut core::ffi::c_void, 0, win::MEM_RELEASE) };
            debug_assert!(ok != 0, "VirtualFree(MEM_RELEASE) failed");
        }

        #[cfg(all(not(miri), unix, not(windows)))]
        {
            // SAFETY: `self.ptr` and `self.backing.map_len` are the exact base
            // and length passed to `mmap` in `with_reserve` (`map_len ==
            // os_reserve`, the FULL reservation length, NOT the committed
            // frontier — single assignment site), unmapped exactly once
            // (`Drop` runs once). `munmap` unmaps irrespective of per-page
            // protection, so a partially-committed (PROT_NONE tail) mapping
            // is released in full (Phase X.F). After this point `self.ptr`
            // must not be used — fine, because `Drop` is the last call on
            // `self`.
            let ret = unsafe {
                libc::munmap(self.ptr.as_ptr() as *mut core::ffi::c_void, self.backing.map_len)
            };
            debug_assert_eq!(ret, 0, "munmap failed");
        }

        #[cfg(any(miri, not(any(windows, unix))))]
        {
            // SAFETY: `self.ptr` was returned by `alloc(self.backing.layout)`
            // in `with_reserve` (the layout spans the full granule-rounded
            // `os_reserve`) and has not been freed yet (`Drop` runs once).
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
    use crate::ecs::memory::utils::align_up;

    /// Number of live blocks in the free-block tracker (test-only probe).
    fn free_block_count(arena: &Arena) -> usize {
        // SAFETY: M-003 — single-threaded test, no other borrow of
        // `free_blocks` is live across this call; the shared borrow ends
        // inside the expression.
        unsafe { (*arena.free_blocks.get()).len() }
    }

    /// Offset of an allocation from the arena base (test-only probe).
    fn offset_of(arena: &Arena, ptr: NonNull<u8>) -> usize {
        ptr.as_ptr() as usize - arena.ptr.as_ptr() as usize
    }

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
        // DEFAULT_ARENA_RESERVE is itself a multiple of CACHE_LINE_SIZE, so
        // after align_up the capacity (Phase X.F: the logical RESERVE — the
        // OOM ceiling) must equal it exactly.
        let expected = align_up(DEFAULT_ARENA_RESERVE, CACHE_LINE_SIZE);
        assert_eq!(
            arena.capacity(),
            expected,
            "Arena::new() capacity should match align_up(DEFAULT_ARENA_RESERVE, CACHE_LINE_SIZE)"
        );
        // Phase X.F: `new()` is reserve-only — ZERO commit charge up front.
        assert_eq!(
            arena.committed(),
            0,
            "Arena::new() must not commit anything up front"
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
        // In debug mode the debug_assert fires first ("allocation request ... exceeds arena reserve").
        // In release mode the cold reserve-exhausted panic in `grow_then_retry`
        // fires (Phase X.F: `with_capacity(64)` means reserve == 64, so growth
        // past it is impossible — the historical OOM surface is preserved).
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
        // Phase X.F re-spec (R2-W2) — the ONLY test exercising the DEFAULT
        // multi-GB reservation round trip: reserve-only acquire (zero commit)
        // -> one small allocation (the first-alloc grow path commits exactly
        // ARENA_MIN_SLAB) -> Drop releasing a PARTIALLY-committed
        // reservation. x50 loop. U8 (below) covers the same shape for small
        // `with_reserve` arenas; this one pins the production-default sizing.
        //
        // A mismatched deallocator, a double free, or a leak of the large
        // reservation would surface here as a crash (native), a borrow/leak
        // error (Miri fallback arm), or address-space/commit exhaustion if a
        // prior iteration's mapping were not released.
        let expected = align_up(DEFAULT_ARENA_RESERVE, CACHE_LINE_SIZE);
        for _ in 0..50 {
            let arena = Arena::new();
            assert_eq!(
                arena.capacity(),
                expected,
                "each default arena must report the rounded logical reserve"
            );
            assert_eq!(arena.committed(), 0, "fresh default arena must be uncommitted");
            // First allocation: takes the cold grow path, commits the first
            // slab (MIN_SLAB dominates this tiny request).
            let p = arena.allocate_layout(
                Layout::from_size_align(CACHE_LINE_SIZE, CACHE_LINE_SIZE)
                    .expect("valid layout"),
            );
            assert_eq!(
                arena.committed(),
                ARENA_MIN_SLAB,
                "first small alloc must commit exactly ARENA_MIN_SLAB"
            );
            // SAFETY: `p` is a fresh CACHE_LINE_SIZE-byte block inside the
            // freshly committed slab; writing one byte at offset 0 is in
            // bounds and proves the page is genuinely writable.
            unsafe {
                p.as_ptr().write_volatile(0xABu8);
            }
            // `arena` drops at end of scope — must release the whole
            // reservation, committed slab included.
        }
    }

    // --- grow_step (Phase X.F U1) ---

    #[test]
    fn grow_step_table() {
        const G: usize = ARENA_COMMIT_GRANULE;
        const MIB: usize = 1024 * 1024;
        // (committed, needed, os_reserve, expected, label)
        let table: &[(usize, usize, usize, usize, &str)] = &[
            // First step from an empty arena: MIN_SLAB dominates a small need.
            (0, G, 256 * MIB, ARENA_MIN_SLAB, "min-slab first step"),
            // Geometric doubling: step == committed while inside the clamp.
            (2 * MIB, G, 256 * MIB, 2 * MIB, "double at 2 MiB"),
            (4 * MIB, G, 256 * MIB, 4 * MIB, "double at 4 MiB"),
            (32 * MIB, G, 256 * MIB, 32 * MIB, "double at 32 MiB"),
            // Max clamp: committed past MAX_SLAB no longer doubles.
            (128 * MIB, G, 1024 * MIB, ARENA_MAX_SLAB, "max-slab clamp"),
            // Request-dominant: needed exceeds the clamped double.
            (0, 10 * MIB + G, 256 * MIB, 10 * MIB + G, "request-dominant cold"),
            (2 * MIB, 3 * MIB + G, 256 * MIB, 3 * MIB + G, "request-dominant warm"),
            // A single huge request is NOT MAX_SLAB-clamped (one event).
            (128 * MIB, 100 * MIB, 1024 * MIB, 100 * MIB, "huge request unclamped"),
            // Ceiling clamp: remaining reserve truncates the step.
            (2 * MIB, G, 3 * MIB, MIB, "ceiling clamp"),
            // Frontier at the ceiling: step == 0.
            (4 * MIB, G, 4 * MIB, 0, "step 0 at ceiling"),
            (G, G, G, 0, "step 0 at tiny ceiling"),
        ];
        for &(committed, needed, os_reserve, expected, label) in table {
            assert_eq!(
                grow_step(committed, needed, os_reserve),
                expected,
                "grow_step table case failed: {label}"
            );
            // R2-W1 induction: a granule-aligned input triple yields a
            // granule-aligned step.
            assert_eq!(
                grow_step(committed, needed, os_reserve) % G,
                0,
                "grow_step result must stay granule-aligned: {label}"
            );
        }
    }

    // --- Phase X.F growth (U2-U10) ---

    const MIB: usize = 1024 * 1024;

    // U2: the first allocation on a lazy arena commits exactly MIN_SLAB and
    // succeeds.
    #[test]
    fn grow_first_alloc_commits_min_slab() {
        let arena = Arena::with_reserve(16 * MIB, 0);
        assert_eq!(arena.capacity(), 16 * MIB);
        assert_eq!(arena.committed(), 0, "lazy arena starts uncommitted");

        let layout = Layout::from_size_align(64, 64).expect("valid layout");
        let p = arena.allocate_layout(layout);
        assert_eq!(
            arena.committed(),
            ARENA_MIN_SLAB,
            "small first alloc must commit exactly ARENA_MIN_SLAB"
        );
        // SAFETY: `p` is a fresh 64-byte block inside the freshly committed
        // slab; the write is in bounds and proves the page is writable.
        unsafe { p.as_ptr().write_volatile(0xCDu8) };
    }

    // U3: the frontier insert left-coalesces with the free tail — after a
    // grow + retry the tracker holds a SINGLE block (a non-coalescing insert
    // would leave two).
    #[test]
    fn grow_frontier_insert_coalesces_with_free_tail() {
        let arena = Arena::with_reserve(64 * MIB, 0);

        // Commit the first slab (2 MiB) and take 64 B of it: free tail is
        // [64, 2 MiB) — one block.
        let small = Layout::from_size_align(64, 64).expect("valid layout");
        let p0 = arena.allocate_layout(small);
        assert_eq!(arena.committed(), ARENA_MIN_SLAB);
        assert_eq!(free_block_count(&arena), 1);

        // 3 MiB request: best-fit fails, grow commits a covering slab whose
        // insert merges with the tail into [64, ...). The retry then carves
        // from the merged block's 64-aligned start.
        let big = Layout::from_size_align(3 * MIB, 64).expect("valid layout");
        let p1 = arena.allocate_layout(big);
        assert_eq!(
            free_block_count(&arena),
            1,
            "frontier insert must coalesce with the free tail (single remainder block)"
        );
        assert_eq!(
            offset_of(&arena, p1),
            64,
            "retry must allocate from the MERGED block (old-tail start), not the new slab"
        );
        assert!(offset_of(&arena, p0) < offset_of(&arena, p1));
    }

    // U4: an oversized request (> MIN_SLAB doubling) is covered by a SINGLE
    // request-dominant step — `needed` is not MAX_SLAB-clamped.
    #[test]
    fn grow_oversized_request_single_covering_step() {
        let arena = Arena::with_reserve(64 * MIB, 0);
        let layout = Layout::from_size_align(10 * MIB, 64).expect("valid layout");
        let p = arena.allocate_layout(layout);
        // needed = align_up(10 MiB + 63, GRANULE) = 10 MiB + one granule;
        // step = max(clamp(0) = MIN_SLAB, needed) = needed — ONE event.
        assert_eq!(
            arena.committed(),
            10 * MIB + ARENA_COMMIT_GRANULE,
            "one covering step must commit exactly the granule-rounded request"
        );
        assert_eq!(offset_of(&arena, p), 0);
        // SAFETY: head and tail of the 10 MiB block are inside the committed
        // slab; the writes prove both ends are genuinely writable.
        unsafe {
            p.as_ptr().write_volatile(1u8);
            p.as_ptr().add(10 * MIB - 1).write_volatile(2u8);
        }
    }

    // U5: a request past the reserve panics (debug: the reserve
    // debug_assert; release: the cold exhaustion panic — both mention the
    // reserve).
    #[test]
    fn grow_reserve_exhausted_panics() {
        let result = std::panic::catch_unwind(|| {
            let arena = Arena::with_reserve(4 * MIB, 0);
            let layout = Layout::from_size_align(8 * MIB, 64).expect("valid layout");
            arena.allocate_layout(layout);
        });
        let err = result.expect_err("8 MiB request on a 4 MiB reserve must panic");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            msg.contains("reserve"),
            "panic must name the reserve as the ceiling, got: {msg}"
        );
    }

    // U6: `with_capacity` back-compat — eager commit, capacity == committed
    // for granule-multiple requests; sub-granule requests keep the logical
    // capacity while committing one granule.
    #[test]
    fn with_capacity_back_compat_eager_commit() {
        // Granule multiple: reserve == os_reserve == committed.
        let a = Arena::with_capacity(2 * MIB);
        assert_eq!(a.capacity(), 2 * MIB);
        assert_eq!(a.committed(), 2 * MIB, "eager arm commits the whole reserve");
        // No growth on an in-capacity alloc.
        let layout = Layout::from_size_align(MIB, 64).expect("valid layout");
        let _p = a.allocate_layout(layout);
        assert_eq!(a.committed(), 2 * MIB, "in-capacity alloc must not grow");

        // Sub-granule: logical capacity stays cache-line-rounded (O4); the
        // commit rounds up to the granule-rounded mapping length.
        let b = Arena::with_capacity(64);
        assert_eq!(b.capacity(), 64, "logical capacity must stay 64 B");
        assert_eq!(
            b.committed(),
            ARENA_COMMIT_GRANULE,
            "sub-granule eager commit rounds to one granule"
        );
    }

    // U7: a 64 B-aligned request straddling a growth event returns an
    // aligned pointer whose block spans the old frontier.
    #[test]
    fn grow_alignment_at_grown_frontier() {
        let arena = Arena::with_reserve(64 * MIB, 0);

        // First alloc: commits 2 MiB, occupies [0, 1 MiB + 32) — the free
        // tail starts UNaligned at 1 MiB + 32.
        let first = Layout::from_size_align(MIB + 32, 1).expect("valid layout");
        let _p0 = arena.allocate_layout(first);
        assert_eq!(arena.committed(), 2 * MIB);

        // 1.5 MiB @ 64: does not fit the ~1 MiB tail, grows by 2 MiB; the
        // merged block starts at 1 MiB + 32, so the aligned carve lands at
        // 1 MiB + 64 — INSIDE the old slab — and ends past the 2 MiB frontier.
        let second = Layout::from_size_align(3 * MIB / 2, 64).expect("valid layout");
        let p1 = arena.allocate_layout(second);
        assert_eq!(arena.committed(), 4 * MIB);
        let off = offset_of(&arena, p1);
        assert_eq!(p1.as_ptr() as usize % 64, 0, "pointer must be 64 B aligned");
        assert_eq!(off, MIB + 64, "aligned carve from the merged block");
        assert!(off < 2 * MIB, "block must start below the old frontier");
        assert!(off + 3 * MIB / 2 > 2 * MIB, "block must end past the old frontier");
        // SAFETY: head and tail of the block are inside committed space
        // ([0, 4 MiB)); the writes prove both sides of the old frontier are
        // writable.
        unsafe {
            p1.as_ptr().write_volatile(3u8);
            p1.as_ptr().add(3 * MIB / 2 - 1).write_volatile(4u8);
        }
    }

    // U8: Drop with a PARTIALLY-committed reserve, x50 (native syscall
    // round trip: reserve-only acquire -> partial commit -> full release).
    #[test]
    fn arena_drop_partially_committed_loop() {
        let layout = Layout::from_size_align(64, 64).expect("valid layout");
        for _ in 0..50 {
            let arena = Arena::with_reserve(16 * MIB, 0);
            let p = arena.allocate_layout(layout);
            assert_eq!(arena.committed(), ARENA_MIN_SLAB);
            // SAFETY: 64-byte block inside the committed slab; in-bounds write.
            unsafe { p.as_ptr().write_volatile(0xEEu8) };
            // Drop releases a 16 MiB reservation of which 2 MiB is committed.
        }
    }

    // U9 (R2 C1, the critic's trace verbatim): legitimate exhaustion when
    // tail + usable < required_size — the EXHAUSTION panic fires (NOT the
    // "logic bug" one) and NO state changed.
    #[test]
    fn grow_exhaustion_check_before_state_change() {
        let arena = Arena::with_reserve(3 * MIB + 32 * 1024, 0);

        // 64 KiB alloc: commits the first 2 MiB slab.
        let first = Layout::from_size_align(64 * 1024, 8).expect("valid layout");
        let _p = arena.allocate_layout(first);
        assert_eq!(arena.committed(), 2 * MIB);
        let blocks_before = free_block_count(&arena);

        // 3 MiB request: step clamps to the reservation remainder
        // (1,114,112 B < needed), usable truncates at the logical reserve,
        // and merged tail + usable < required_size => exhaustion.
        let big = Layout::from_size_align(3 * MIB, 64).expect("valid layout");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            arena.allocate_layout(big);
        }));
        let err = result.expect_err("3 MiB request must exhaust the 3 MiB + 32 KiB reserve");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            msg.contains("Arena reserve exhausted"),
            "must be the EXHAUSTION surface, got: {msg}"
        );
        assert!(
            !msg.contains("logic bug"),
            "legitimate exhaustion must NOT be mislabeled a logic bug, got: {msg}"
        );

        // No-state-change witness: the frontier and the free list are
        // exactly as before the failed request.
        assert_eq!(
            arena.committed(),
            2 * MIB,
            "exhaustion must not commit anything"
        );
        assert_eq!(
            free_block_count(&arena),
            blocks_before,
            "exhaustion must not touch the free list"
        );
    }

    // U10 (R3-1 re-spec): false-exhaustion regression net — the sufficiency
    // check uses required_size (102,363 <= usable 102,400), NOT the
    // granule-rounded needed (131,072 > usable). MUST succeed, with a
    // genuinely aligned pointer.
    #[test]
    fn grow_no_false_exhaustion_on_required_size() {
        let arena = Arena::with_reserve(100 * 1024, 0);
        let layout = Layout::from_size_align(102_300, 64).expect("valid layout");
        let p = arena.allocate_layout(layout);
        assert_eq!(p.as_ptr() as usize % 64, 0, "pointer must be 64 B aligned");
        assert_eq!(
            arena.committed(),
            2 * ARENA_COMMIT_GRANULE,
            "the whole (granule-rounded) reservation gets committed"
        );
        // SAFETY: head and tail of the 102,300-byte block are inside the
        // committed 128 KiB; the writes prove the pages are writable.
        unsafe {
            p.as_ptr().write_volatile(5u8);
            p.as_ptr().add(102_299).write_volatile(6u8);
        }
    }

    // committed(): monotonic, granule-aligned, bounded by the granule-rounded
    // reserve — across several growth events.
    #[test]
    fn committed_monotonic_granule_aligned_bounded() {
        let arena = Arena::with_reserve(32 * MIB, 0);
        let os_reserve = align_up(arena.capacity(), ARENA_COMMIT_GRANULE);
        let layout = Layout::from_size_align(3 * MIB, 64).expect("valid layout");
        let mut last = arena.committed();
        assert_eq!(last, 0);
        for _ in 0..5 {
            let _p = arena.allocate_layout(layout);
            let now = arena.committed();
            assert!(now >= last, "committed must be monotonic");
            assert_eq!(now % ARENA_COMMIT_GRANULE, 0, "committed must stay granule-aligned");
            assert!(now <= os_reserve, "committed must never pass os_reserve");
            last = now;
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
