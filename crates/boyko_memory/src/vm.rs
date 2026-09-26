//! Shared virtual-memory reservation primitive (Phase X.G, D1).
//!
//! `VmReservation` is the bare reserve/commit/release mechanism extracted
//! from the historical shared `Arena`'s per-target backing arms (the Arena
//! itself was retired in Phase X.J once Phase X.I gave every
//! `ComponentPool` its own per-pool reservation): one contiguous OS-level
//! address-space reservation, committed lazily in caller-defined ranges, and
//! released as a whole on `Drop`. ALL policy (commit watermark, slab sizing,
//! element length) lives in the OWNER — this type is a dumb `(base, os_len)`
//! wrapper.
//!
//! # Single source of truth (Phase X.H)
//!
//! These cfg arms are THE per-OS backing implementation for the whole
//! engine: `ComponentPool` (Phase X.I) and `InlandStore` (Phase X.G) build
//! on them. The historical W-RES/U-RES/F-RES / W-CMT/U-CMT SAFETY lineage
//! from the retired `arena.rs` lives here now.
//!
//! # Zero-fill contract (Phase X.G, D1)
//!
//! **Freshly committed memory reads zero on first access, on every arm.**
//! - Windows: `VirtualAlloc(MEM_COMMIT)` pages are documented zero-fill.
//! - Unix: anonymous `mmap` pages are zero-fill; `mprotect` does not alter
//!   contents.
//! - Fallback (Miri / wasm32 / exotic): the WHOLE reservation is eagerly
//!   acquired with [`std::alloc::alloc_zeroed`] (NOT `alloc` — the X.G/X.I
//!   consumers READ never-program-written memory by design, see
//!   `InlandStore`'s I-Z invariant and the pool's J-XI tick contract).
//!
//! De-jure status of the syscall arms (plan R2-W1): the Rust abstract machine
//! does not model raw-syscall memory; the justification is equivalence with
//! `alloc_zeroed` itself — production allocators' calloc /
//! `HeapAlloc(HEAP_ZERO_MEMORY)` fresh-page paths hand back untouched
//! kernel-zero pages and the `GlobalAlloc` contract calls them initialized.
//! Treating OS zero-fill as an external write of zeros is exactly as
//! official. Miri can validate only the fallback arm; the syscall arms are
//! validated natively by the U-V3 / U-S3 zero-read witnesses.

#[cfg(any(miri, not(any(windows, unix))))]
use std::alloc::{Layout, alloc_zeroed, dealloc};
#[cfg(all(debug_assertions, not(miri), windows))]
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use crate::constants::{COMMIT_GRANULE, COMMIT_PAGE};
use crate::owner::{self, ColumnOwner, CommitOwner};

/// Cold-path checked `align_up` — twin of `arena.rs::checked_align_up`
/// (kept private per module until the X.H unification).
fn checked_align_up(value: usize, granule: usize) -> usize {
    debug_assert!(granule.is_power_of_two(), "granule must be a power of two");
    value
        .checked_add(granule - 1)
        .expect("VmReservation: align_up overflow (value too close to usize::MAX)")
        & !(granule - 1)
}

/// The OS page size, for the debug-only `COMMIT_PAGE` belt in
/// [`VmReservation::reserve`].
#[cfg(all(debug_assertions, not(miri), windows))]
fn os_page_size() -> usize {
    let mut info = MaybeUninit::<win::SystemInfo>::uninit();
    // SAFETY: `GetSystemInfo` has no failure mode and writes the whole
    // `SYSTEM_INFO` it is handed; `info` is a writable buffer of exactly that
    // (const-asserted, 48 B) layout.
    unsafe { win::GetSystemInfo(info.as_mut_ptr()) };
    // SAFETY: fully initialised by `GetSystemInfo` above.
    let info = unsafe { info.assume_init() };
    info.page_size as usize
}

/// The OS page size, for the debug-only `COMMIT_PAGE` belt in
/// [`VmReservation::reserve`].
#[cfg(all(debug_assertions, not(miri), unix, not(windows)))]
fn os_page_size() -> usize {
    // SAFETY: `sysconf` has no preconditions, and `_SC_PAGESIZE` is a name
    // every unix supports.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(page).expect("sysconf(_SC_PAGESIZE) returned a negative value")
}

/// Windows kernel32 surface — twin of `arena.rs::win` (X.H unifies).
#[cfg(all(not(miri), windows))]
mod win {
    use core::ffi::c_void;

    // SAFETY: signatures match the Win64 kernel32 ABI exactly (LPVOID ->
    // *mut c_void, SIZE_T -> usize, DWORD -> u32, BOOL -> i32). kernel32 is
    // linked transitively by std.
    unsafe extern "system" {
        pub fn VirtualAlloc(
            lpAddress: *mut c_void,
            dwSize: usize,
            flAllocationType: u32,
            flProtect: u32,
        ) -> *mut c_void;
        pub fn VirtualFree(lpAddress: *mut c_void, dwSize: usize, dwFreeType: u32) -> i32;
        #[cfg(debug_assertions)]
        pub fn GetSystemInfo(lpSystemInfo: *mut SystemInfo);
    }

    /// `SYSTEM_INFO` on Win64, for the debug-only `COMMIT_PAGE` belt in
    /// `VmReservation::reserve`. Every field is declared so the layout is the
    /// kernel's; the belt reads one of them.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    #[repr(C)]
    pub struct SystemInfo {
        pub oem_id: u32,
        pub page_size: u32,
        pub minimum_application_address: *mut c_void,
        pub maximum_application_address: *mut c_void,
        pub active_processor_mask: usize,
        pub number_of_processors: u32,
        pub processor_type: u32,
        pub allocation_granularity: u32,
        pub processor_level: u16,
        pub processor_revision: u16,
    }
    #[cfg(debug_assertions)]
    const _: () = assert!(size_of::<SystemInfo>() == 48);

    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;
    pub const PAGE_NOACCESS: u32 = 0x01;
    pub const PAGE_READWRITE: u32 = 0x04;
}

/// One contiguous virtual-address reservation with caller-driven lazy commit.
///
/// `!Send`/`!Sync` via `NonNull` — owners that are shared across threads opt
/// in with their own `unsafe impl` and their own exclusivity argument
/// (`EntityMaster`'s SEND5), matching the `Arena` discipline.
pub struct VmReservation {
    /// Write-once base of the single reservation; never reassigned, so every
    /// pointer derived from it stays valid for the reservation's lifetime.
    base: NonNull<u8>,
    /// Granule-rounded reservation length (`<= isize::MAX`, asserted in
    /// `reserve`). On the unix arm this is the exact `munmap` length; on the
    /// fallback arm it equals `layout.size()`.
    os_len: usize,
    /// Fallback release descriptor (M-001: the matching deallocator is
    /// statically selected by the cfg-gated field set).
    #[cfg(any(miri, not(any(windows, unix))))]
    layout: Layout,
}

impl VmReservation {
    /// Reserves `len` bytes of address space (granule-rounded), committing
    /// NOTHING on the syscall arms. Panics loudly on a zero/over-`isize`
    /// request or OS failure — reservation failure is unrecoverable
    /// misconfiguration, mirroring the arena's contract.
    ///
    /// The fallback arm eagerly allocates ZEROED memory (the X.G zero-fill
    /// contract — see the module doc). The historical `reserve_unzeroed`
    /// variant (a fallback-arm `alloc` for write-before-read consumers) was
    /// deleted with its sole client, the shared Arena (Phase X.J).
    #[doc(hidden)]
    pub fn reserve(len: usize) -> Self {
        assert!(len > 0, "VmReservation: reserve length must be non-zero");
        // Packing plan D1 belt: `COMMIT_PAGE` is a compile-time assumption
        // about the OS page size (it must stay `const` for the layout
        // proofs). A kernel whose page does not divide it would reject a
        // legal frontier commit, so say so loudly at the first reservation
        // instead. Debug-only and stateless — no `static` caches the answer;
        // both queries are user-mode reads.
        #[cfg(all(debug_assertions, not(miri), any(windows, unix)))]
        {
            let page = os_page_size();
            debug_assert!(
                page > 0 && COMMIT_PAGE.is_multiple_of(page),
                "VmReservation: the OS page ({page} B) does not divide COMMIT_PAGE \
                 ({COMMIT_PAGE} B); every frontier commit would be misaligned"
            );
        }
        let os_len = checked_align_up(len, COMMIT_GRANULE);
        // Twin of arena.rs review-F1: every offset later fed to
        // `base.add(..)` must fit `isize` (pointer::add contract). The
        // fallback arm gets this from `Layout`; the syscall arms need the
        // explicit cold compare.
        assert!(
            os_len <= isize::MAX as usize,
            "VmReservation: {os_len} B exceeds isize::MAX (pointer-offset contract)"
        );

        #[cfg(all(not(miri), windows))]
        {
            // SAFETY (V-RES-W, twin of arena W-RES): NULL base lets the OS
            // choose the address; `dwSize = os_len` (> 0, asserted above).
            // MEM_RESERVE + PAGE_NOACCESS reserves address space WITHOUT
            // commit charge or access; `commit` re-protects ranges later.
            // Result is null-checked before use.
            let raw = unsafe {
                win::VirtualAlloc(
                    core::ptr::null_mut(),
                    os_len,
                    win::MEM_RESERVE,
                    win::PAGE_NOACCESS,
                )
            };
            let base = NonNull::new(raw as *mut u8)
                .expect("VirtualAlloc failed to reserve address space");
            // A reservation base is aligned to the allocation granularity
            // (64 KiB on every Windows arch), above the floor `VmColumn`'s
            // alignment proof rests on: checked here, where that claim is made.
            debug_assert!(
                base.as_ptr()
                    .addr()
                    .is_multiple_of(crate::constants::RESERVATION_BASE_ALIGN),
                "VmReservation: VirtualAlloc base {base:p} is below the {} B alignment floor",
                crate::constants::RESERVATION_BASE_ALIGN
            );
            Self { base, os_len }
        }

        #[cfg(all(not(miri), unix, not(windows)))]
        {
            // SAFETY (V-RES-U, twin of arena U-RES): NULL base, `len =
            // os_len` (> 0). PROT_NONE + MAP_PRIVATE | MAP_ANONYMOUS reserves
            // a private anonymous range with no access and no overcommit
            // accounting until `commit` mprotects ranges RW (fd = -1,
            // offset = 0).
            let raw = unsafe {
                libc::mmap(
                    core::ptr::null_mut(),
                    os_len,
                    libc::PROT_NONE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            // MAP_FAILED is (void*)-1 — NON-NULL — so it must be checked
            // BEFORE NonNull::new (the X.C trap, preserved).
            assert!(
                raw != libc::MAP_FAILED,
                "mmap failed to reserve address space"
            );
            let base = NonNull::new(raw as *mut u8).expect("mmap returned null");
            // `mmap` promises only page alignment, so the floor `VmColumn`'s
            // alignment proof rests on is a claim about the kernel's page
            // size: checked here, where that claim is made.
            debug_assert!(
                base.as_ptr()
                    .addr()
                    .is_multiple_of(crate::constants::RESERVATION_BASE_ALIGN),
                "VmReservation: mmap base {base:p} is below the {} B alignment floor",
                crate::constants::RESERVATION_BASE_ALIGN
            );
            Self { base, os_len }
        }

        #[cfg(any(miri, not(any(windows, unix))))]
        {
            let layout = Layout::from_size_align(os_len, crate::constants::RESERVATION_BASE_ALIGN)
                .expect("VmReservation: invalid fallback layout");
            // SAFETY (V-RES-F, twin of arena F-RES + the X.G zero-fill
            // contract): non-zero size (asserted above), power-of-two align.
            // `alloc_zeroed` — NOT `alloc` — because X.G consumers READ
            // never-program-written memory (module doc: zero-fill contract).
            let raw = unsafe { alloc_zeroed(layout) };
            let base = NonNull::new(raw).expect("VmReservation: fallback allocation failed");
            Self { base, os_len, layout }
        }
    }

    /// Base of the reservation (write-once; stable for the lifetime).
    #[inline]
    pub fn base(&self) -> NonNull<u8> {
        self.base
    }

    /// Granule-rounded reservation length in bytes.
    #[inline]
    pub fn os_len(&self) -> usize {
        self.os_len
    }

    /// Commits (makes readable/writable, zero-filled) the byte range
    /// `[old, new)` of the reservation, counted under [`ColumnOwner`]: the
    /// route of every `ComponentPool`, `InlandStore` and the profiling store. A
    /// store owned by another owner calls [`commit_at`] directly. Page-aligned
    /// (`COMMIT_PAGE`) and monotonic-frontier use only (debug-asserted in
    /// `commit_at`). `MEM_COMMIT` inside a reservation and `mprotect` are
    /// page-granular; only the reservation itself is bound by the 64 KiB
    /// granularity (packing plan D1). Page alignment plus `new <= os_len` is
    /// what keeps a frontier commit inside the kernel's mapping: `os_len` is
    /// granule-rounded and a granule is a whole number of pages. No-op on the
    /// fallback arm (the whole reservation is eagerly RW + zeroed).
    ///
    /// Never inlined: the Column route stays one out-of-line cold call, so the
    /// callers' bodies (`ComponentPool::commit_subregion` jumps here) do not
    /// change with what the commit path counts.
    ///
    /// # Safety
    /// `old < new` and `new <= self.os_len()` (debug-asserted). The syscall
    /// arms offset the base by `old` and change the protection of `new - old`
    /// bytes from there, which is sound only inside the reservation.
    ///
    /// The fn is `unsafe` since rung C1 made it public in another crate. Inside
    /// `boyko_ecs` it was a safe `pub(crate)` fn whose range was checked only in
    /// debug builds, sound because every caller was in the crate. A release
    /// range check instead would move the pinned callers' codegen (UG-15
    /// P29-1, P29-2: reading `os_len` here stops LLVM promoting `&self` to its
    /// `base`), so each caller proves the range at its own site.
    ///
    /// # Panics
    /// The OS refuses the commit (commit charge or overcommit exhausted).
    #[cold]
    #[inline(never)]
    #[doc(hidden)]
    pub unsafe fn commit(&self, old: usize, new: usize) {
        debug_assert!(
            old < new && new <= self.os_len,
            "VmReservation::commit: range [{old}, {new}) is empty, backwards or overruns the \
             reservation ({})",
            self.os_len
        );
        // SAFETY: the caller upholds this fn's `# Safety`, `old < new <= os_len`,
        // so `new - old` does not wrap and `old + (new - old) = new <= os_len`:
        // exactly `commit_at`'s contract.
        unsafe { commit_at::<ColumnOwner>(self, old, new - old) }
    }
}

/// Commits `[off, off + len)` of `res` and counts the `len` bytes under owner
/// `O`: the one choke point every kernel commit goes through (unified plan
/// KC-01; gate UG-04 reads the counter). [`VmReservation::commit`] is the
/// [`ColumnOwner`] route; a store owned by another owner (a KC-18 table as
/// `VmColumn<T, TableOwner>`, the scope `ChunkArena`) calls this directly.
///
/// `len` should be non-zero and `off`, `off + len` should be `COMMIT_PAGE`
/// multiples (debug-asserted). Neither is a soundness condition: a misaligned
/// range is rounded out to whole pages on Windows and refused with a panic on
/// unix. Idempotent over already committed pages. A no-op on the fallback arm
/// (the whole reservation is eagerly RW + zeroed), and counted there too, so
/// Miri reads the same numbers as a native run.
///
/// Never inlined: every commit, whatever its route, pays one cold call beside
/// a syscall, and the counter's add exists in exactly this function.
///
/// # Safety
/// `off + len` must not overflow and must be `<= res.os_len()`. The syscall
/// arms offset the reservation's base by `off` and change the protection of
/// `len` bytes from there, which is sound only inside the reservation.
///
/// # Panics
/// The OS refuses the commit (commit charge or overcommit exhausted).
#[doc(hidden)]
#[cold]
#[inline(never)]
pub unsafe fn commit_at<O: CommitOwner>(res: &VmReservation, off: usize, len: usize) {
    let (old, new) = (off, off + len);
    debug_assert!(new > old, "commit_at: empty range");
    debug_assert!(
        old.is_multiple_of(COMMIT_PAGE) && new.is_multiple_of(COMMIT_PAGE),
        "commit_at: range [{old}, {new}) not page-aligned"
    );
    debug_assert!(
        new <= res.os_len,
        "commit_at: range end {new} overruns the reservation ({})",
        res.os_len
    );

    #[cfg(all(not(miri), windows))]
    {
        // SAFETY (V-CMT-W, twin of arena W-CMT): the range lies inside
        // the reservation (`off + len <= os_len`, this fn's `# Safety`
        // contract), so `base + old` stays inside the reservation object. It
        // is `COMMIT_PAGE`-aligned on both ends (debug-asserted; the base is
        // granule-aligned), and a misaligned range would only be rounded out
        // to whole pages, which the granule-rounded `os_len` keeps inside the
        // reservation. Re-committing an already committed page is
        // documented-idempotent (contents untouched). NULL result = commit
        // charge exhausted — the loud genuine-OOM surface.
        let raw = unsafe {
            win::VirtualAlloc(
                res.base.as_ptr().add(old) as *mut core::ffi::c_void,
                new - old,
                win::MEM_COMMIT,
                win::PAGE_READWRITE,
            )
        };
        assert!(
            !raw.is_null(),
            "VirtualAlloc(MEM_COMMIT) failed committing [{old}, {new}) \
             (commit charge exhausted?)"
        );
    }

    #[cfg(all(not(miri), unix, not(windows)))]
    {
        // SAFETY (V-CMT-U, twin of arena U-CMT): the range lies inside the
        // mapping (`off + len <= os_len == munmap length`, this fn's
        // `# Safety` contract), so `base + old` stays inside it, and both ends
        // are `COMMIT_PAGE`-aligned (debug-asserted) from a page-aligned base.
        // The OS page divides `COMMIT_PAGE` — 4 KiB is the x86_64 base page
        // and every other arch commits by the granule (packing plan D1; the
        // debug belt in `reserve` checks it at the first reservation) — so
        // mprotect gets a page-aligned base and length; a misaligned one is
        // refused with EINVAL and panics at the release assert below, never
        // touching another mapping. ENOMEM here is the overcommit-mode-2
        // failure surface.
        let ret = unsafe {
            libc::mprotect(
                res.base.as_ptr().add(old) as *mut core::ffi::c_void,
                new - old,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        assert!(
            ret == 0,
            "mprotect(PROT_READ | PROT_WRITE) failed committing [{old}, {new}) \
             (ENOMEM = overcommit limit)"
        );
    }

    // Fallback arm: no-op — eagerly RW + zero-filled in `reserve`.
    #[cfg(any(miri, not(any(windows, unix))))]
    {
        let _ = (old, new);
    }

    owner::record::<O>(len);
}

impl Drop for VmReservation {
    /// Releases the whole reservation with the deallocator matching the
    /// acquisition arm (M-001). Exactly one arm is compiled.
    fn drop(&mut self) {
        #[cfg(all(not(miri), windows))]
        {
            // SAFETY (V-DROP-W, twin of arena Drop): `base` is the exact base
            // returned by VirtualAlloc in `reserve`, freed exactly once.
            // MEM_RELEASE requires `dwSize == 0` with the original base;
            // partially-committed reservations are released in full.
            let ok = unsafe {
                win::VirtualFree(self.base.as_ptr() as *mut core::ffi::c_void, 0, win::MEM_RELEASE)
            };
            debug_assert!(ok != 0, "VirtualFree(MEM_RELEASE) failed");
        }

        #[cfg(all(not(miri), unix, not(windows)))]
        {
            // SAFETY (V-DROP-U, twin of arena Drop): `base`/`os_len` are the
            // exact base and FULL length passed to mmap in `reserve`, unmapped
            // exactly once. munmap unmaps irrespective of per-page protection
            // (PROT_NONE tails are released in full).
            let ret =
                unsafe { libc::munmap(self.base.as_ptr() as *mut core::ffi::c_void, self.os_len) };
            debug_assert_eq!(ret, 0, "munmap failed");
        }

        #[cfg(any(miri, not(any(windows, unix))))]
        {
            // SAFETY (V-DROP-F, twin of arena Drop): `base` was returned by
            // `alloc_zeroed(self.layout)` in `reserve`, freed exactly once
            // with the identical Layout (GlobalAlloc contract).
            unsafe { dealloc(self.base.as_ptr(), self.layout) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::COMMIT_GRANULE as G;

    /// U-V1 — reserve/commit/drop round trip ×50, including
    /// partially-committed reservations (native syscall exercise; pure
    /// bookkeeping on the fallback arm).
    #[test]
    fn reserve_commit_drop_round_trip() {
        for i in 0..50 {
            let vm = VmReservation::reserve(4 * G);
            if i % 2 == 0 {
                // SAFETY: `0 < G <= 4 * G = os_len` (`4 * G` is a granule multiple).
                unsafe { vm.commit(0, G) };
                if i % 4 == 0 {
                    // SAFETY: `G < 3 * G <= 4 * G = os_len`.
                    unsafe { vm.commit(G, 3 * G) };
                }
            }
            // Drop releases partially-committed reservations in full.
        }
    }

    /// U-V3 — zero-on-first-access witness: freshly committed bytes read 0 at
    /// the head and tail of the slab (the I-Z keystone at the vm level).
    #[test]
    fn committed_memory_reads_zero() {
        let vm = VmReservation::reserve(2 * G);
        // SAFETY: `0 < 2 * G = os_len` (`2 * G` is a granule multiple).
        unsafe { vm.commit(0, 2 * G) };
        // SAFETY: [0, 2G) was just committed RW on every arm; head/tail are
        // in-bounds; u8 reads of zero-fill memory per the module contract.
        unsafe {
            assert_eq!(*vm.base().as_ptr(), 0, "head byte of fresh slab");
            assert_eq!(*vm.base().as_ptr().add(G), 0, "slab-boundary byte");
            assert_eq!(*vm.base().as_ptr().add(2 * G - 1), 0, "tail byte of fresh slab");
        }
    }

    /// U-V3b — written bytes survive a subsequent frontier commit untouched
    /// (idempotent-commit / mprotect-does-not-clear witness).
    #[test]
    fn committed_writes_survive_further_commits() {
        let vm = VmReservation::reserve(4 * G);
        // SAFETY: `0 < G <= 4 * G = os_len` (`4 * G` is a granule multiple).
        unsafe { vm.commit(0, G) };
        // SAFETY: [0, G) committed RW; in-bounds write/read.
        unsafe {
            *vm.base().as_ptr().add(100) = 0xAB;
        }
        // SAFETY: `G < 4 * G = os_len`.
        unsafe { vm.commit(G, 4 * G) };
        // SAFETY: still committed; the X.F idempotent-commit/W-CMT contract
        // says earlier contents are untouched by later frontier commits.
        unsafe {
            assert_eq!(*vm.base().as_ptr().add(100), 0xAB);
        }
    }

    /// U-V4 — degenerate requests panic loudly.
    #[test]
    fn reserve_zero_panics() {
        let r = std::panic::catch_unwind(|| VmReservation::reserve(0));
        assert!(r.is_err(), "reserve(0) must panic");
    }

    /// U-V4b — the isize::MAX pointer-offset guard fires before any syscall.
    #[test]
    fn reserve_over_isize_panics() {
        let r = std::panic::catch_unwind(|| VmReservation::reserve(usize::MAX - G));
        assert!(r.is_err(), "over-isize reserve must panic (align_up overflow or isize guard)");
    }

    /// U-V5 — fallback-arm Layout round trip (the arm Miri actually runs);
    /// on syscall arms this is just another small round trip.
    #[test]
    fn small_reserve_round_trip() {
        let vm = VmReservation::reserve(1);
        assert_eq!(vm.os_len(), G, "1-byte request rounds to one granule");
        // SAFETY: `0 < G = os_len`, asserted on the line above.
        unsafe { vm.commit(0, G) };
        // SAFETY: committed above; single in-bounds byte.
        unsafe {
            assert_eq!(*vm.base().as_ptr(), 0);
        }
    }
}
