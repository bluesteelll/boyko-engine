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
//!   acquired with [`alloc_zeroed`] (NOT `alloc` — the X.G/X.I consumers
//!   READ never-program-written memory by design, see `InlandStore`'s I-Z
//!   invariant and the pool's J-XI tick contract).
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
use std::ptr::NonNull;

use crate::ecs::constants::COMMIT_GRANULE;

/// Cold-path checked `align_up` — twin of `arena.rs::checked_align_up`
/// (kept private per module until the X.H unification).
fn checked_align_up(value: usize, granule: usize) -> usize {
    debug_assert!(granule.is_power_of_two(), "granule must be a power of two");
    value
        .checked_add(granule - 1)
        .expect("VmReservation: align_up overflow (value too close to usize::MAX)")
        & !(granule - 1)
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
    }

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
pub(crate) struct VmReservation {
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
    pub(crate) fn reserve(len: usize) -> Self {
        assert!(len > 0, "VmReservation: reserve length must be non-zero");
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
            Self { base, os_len }
        }

        #[cfg(any(miri, not(any(windows, unix))))]
        {
            let layout = Layout::from_size_align(os_len, COMMIT_GRANULE.min(4096))
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
    pub(crate) fn base(&self) -> NonNull<u8> {
        self.base
    }

    /// Granule-rounded reservation length in bytes.
    #[inline]
    pub(crate) fn os_len(&self) -> usize {
        self.os_len
    }

    /// Commits (makes readable/writable, zero-filled) the byte range
    /// `[old, new)` of the reservation. Granule-aligned, in-bounds,
    /// monotonic-frontier use only (debug-asserted). No-op on the fallback
    /// arm (the whole reservation is eagerly RW + zeroed).
    #[cold]
    pub(crate) fn commit(&self, old: usize, new: usize) {
        debug_assert!(new > old, "VmReservation::commit: empty or backwards range");
        debug_assert!(
            old.is_multiple_of(COMMIT_GRANULE) && new.is_multiple_of(COMMIT_GRANULE),
            "VmReservation::commit: range [{old}, {new}) not granule-aligned"
        );
        debug_assert!(
            new <= self.os_len,
            "VmReservation::commit: range end {new} overruns the reservation ({})",
            self.os_len
        );

        #[cfg(all(not(miri), windows))]
        {
            // SAFETY (V-CMT-W, twin of arena W-CMT): the range lies inside
            // our own reservation (`new <= os_len`, asserted), is
            // granule-aligned (=> page-aligned), and re-committing an already
            // committed page is documented-idempotent (contents untouched).
            // NULL result = commit charge exhausted — the loud genuine-OOM
            // surface.
            let raw = unsafe {
                win::VirtualAlloc(
                    self.base.as_ptr().add(old) as *mut core::ffi::c_void,
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
            // SAFETY (V-CMT-U, twin of arena U-CMT): the range lies inside
            // our own mapping (`new <= os_len == munmap length`) and is
            // granule-aligned (granule is a multiple of the page size), so
            // mprotect gets a page-aligned base and length. ENOMEM here is
            // the overcommit-mode-2 failure surface.
            let ret = unsafe {
                libc::mprotect(
                    self.base.as_ptr().add(old) as *mut core::ffi::c_void,
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
    }
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
    use crate::ecs::constants::COMMIT_GRANULE as G;

    /// U-V1 — reserve/commit/drop round trip ×50, including
    /// partially-committed reservations (native syscall exercise; pure
    /// bookkeeping on the fallback arm).
    #[test]
    fn reserve_commit_drop_round_trip() {
        for i in 0..50 {
            let vm = VmReservation::reserve(4 * G);
            if i % 2 == 0 {
                vm.commit(0, G);
                if i % 4 == 0 {
                    vm.commit(G, 3 * G);
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
        vm.commit(0, 2 * G);
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
        vm.commit(0, G);
        // SAFETY: [0, G) committed RW; in-bounds write/read.
        unsafe {
            *vm.base().as_ptr().add(100) = 0xAB;
        }
        vm.commit(G, 4 * G);
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
        vm.commit(0, G);
        // SAFETY: committed above; single in-bounds byte.
        unsafe {
            assert_eq!(*vm.base().as_ptr(), 0);
        }
    }
}
