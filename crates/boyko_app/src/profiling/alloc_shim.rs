//! Profiling rung 8 — the **opt-in** allocation-counting shim.
//!
//! # The profiler installs no global allocator, and this is the exception that proves it
//!
//! A `#[global_allocator]` is process-wide: it counts every allocation in the process, including
//! the ones the measurement itself makes, and it cannot be scoped to a subsystem. That is why the
//! nineteen zero-alloc gates in this tree are **test binaries** — each installs its own shim, and
//! each measures a window it fully controls. The corpus's disposition is explicit: *"The profiler
//! installs no global allocator. An opt-in `profiling-alloc` feature in `boyko_app` installs a
//! counting shim feeding the `Counter` channel; off by default … and **its perturbation is stated
//! in the artifact when on**."*
//!
//! So this module is **entirely absent** unless `profiling-alloc` is on, and
//! [`snapshot`] is the only thing that compiles either way — returning zeros with
//! [`AllocCounts::armed`] `false`, so the artifact writer needs no `#[cfg]` and a reader can tell
//! *"nothing was counted"* from *"nothing allocated"*.
//!
//! # What it CANNOT claim, said once here
//!
//! * **Not per-frame, and not per-subsystem.** The counters are process totals from start-up. An
//!   allocation made by the window's message pump, by a driver's thread, or by the profiler's own
//!   artifact writer is in them. Attributing them costs a per-call-site mechanism this feature
//!   deliberately does not have — the nineteen gates are the precise instrument, and this one is
//!   the cheap always-on-when-asked one.
//! * **Its numbers are not comparable with an unarmed run's.** The shim adds two atomic
//!   read-modify-writes per allocation on a path the engine's own principle 5 exists to keep off
//!   the hot loop. An artifact written under it is a diagnostic-mode artifact and says so.
//! * **`#[cfg]`-exclusion at the retail tier is NOT implemented here**, because the tier axis does
//!   not exist yet: the single `BOYKO_PROFILE` axis with its five legs is rung 14. Today the
//!   exclusion is the feature flag being off, which is weaker — a build could turn it on. Recorded
//!   rather than claimed.

use core::sync::atomic::{AtomicU64, Ordering};

/// Allocations counted since process start.
static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Deallocations counted since process start.
static DEALLOCS: AtomicU64 = AtomicU64::new(0);
/// Bytes REQUESTED by those allocations — `Layout::size`, not what the allocator reserved.
///
/// The distinction matters for a reader: a size-class allocator rounds up, so the true resident
/// figure is at least this and usually more. Reporting the requested size is the honest choice
/// because it is the only one this shim can observe.
static BYTES: AtomicU64 = AtomicU64::new(0);

/// What the shim has counted, plus whether it was there to count.
///
/// `armed` is the field that makes the other three readable. Without it a zero is ambiguous between
/// *"this build has no shim"* and *"this run allocated nothing"* — and one of those is a claim about
/// the engine while the other is a claim about the build. This campaign has now met that confusion
/// in the label census, in the `content_tag`, and in the present mode; it is the same shape each
/// time, and the fix is always a field rather than a convention.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AllocCounts {
    /// `true` iff the counting shim is installed in this build.
    pub armed: bool,
    /// Allocations since process start.
    pub allocs: u64,
    /// Deallocations since process start.
    pub deallocs: u64,
    /// Bytes requested by those allocations.
    pub bytes: u64,
}

/// Reads the counters. Compiles in both configurations; `armed` says which one this is.
#[must_use]
pub fn snapshot() -> AllocCounts {
    AllocCounts {
        armed: cfg!(feature = "profiling-alloc"),
        // `Relaxed`: these are counters, and no reader orders anything against them. The
        // three loads are independent, so a concurrent allocation can put its count in this
        // snapshot and its bytes in the next — the same eventual consistency `boyko_diag`'s
        // `delta_since` documents, for the same reason and with the same one-event bound.
        allocs: ALLOCS.load(Ordering::Relaxed),
        deallocs: DEALLOCS.load(Ordering::Relaxed),
        bytes: BYTES.load(Ordering::Relaxed),
    }
}

#[cfg(feature = "profiling-alloc")]
mod armed {
    use std::alloc::{GlobalAlloc, Layout, System};

    use super::{ALLOCS, BYTES, DEALLOCS, Ordering};

    /// Delegates every operation to [`System`] and counts it.
    ///
    /// Counting is PROCESS-GLOBAL and not thread-local, unlike the zero-alloc gates' shims. Those
    /// measure one thread's window inside a binary whose other threads must not be folded in; this
    /// one measures the process, because the question it answers — *"how much did this run
    /// allocate"* — has no per-thread reading, and a thread-local would silently omit the render
    /// thread's allocations from a count taken on the main one.
    pub struct CountingAlloc;

    // SAFETY: every method delegates to `System` with the pointer, layout and size arguments
    // forwarded UNCHANGED, so every contract `GlobalAlloc` imposes is `System`'s to keep. The added
    // work is a `Relaxed` counter bump, which touches no allocator state and can neither fail nor
    // unwind.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            // SAFETY: forwarded verbatim.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            // SAFETY: forwarded verbatim.
            unsafe { System.alloc_zeroed(layout) }
        }

        /// A realloc counts as ONE allocation and adds the NEW size.
        ///
        /// Not as a dealloc plus an alloc: that would make `allocs - deallocs` — the figure a
        /// reader uses for "still held" — correct only by cancellation, and it would attribute a
        /// growth of 8 bytes as a full new allocation of the whole buffer in `bytes`. Counting the
        /// new size overstates cumulative bytes for a growing `Vec`, which is stated here rather
        /// than hidden: a `Vec` doubling to 1 MiB reports ~2 MiB across its growth.
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            // SAFETY: forwarded verbatim.
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            DEALLOCS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: forwarded verbatim.
            unsafe { System.dealloc(ptr, layout) };
        }
    }
}

/// The installed shim. **Present only under `profiling-alloc`**, which is what keeps the shipped
/// build's allocator the platform's own.
///
/// ⚠️ A crate that declares a `#[global_allocator]` forces every binary linking it to use that one,
/// and a binary may declare only ONE. Turning this feature on therefore CONFLICTS with any test
/// binary in the workspace that installs its own — `boyko_app`'s own `runner.rs` test module does,
/// as do the nineteen zero-alloc gates. That is measured, not predicted, and it is the concrete
/// reason this feature can never be added to a test sweep's flag set.
#[cfg(feature = "profiling-alloc")]
#[global_allocator]
static SHIM: armed::CountingAlloc = armed::CountingAlloc;

#[cfg(test)]
mod tests {
    use super::*;

    /// `armed` tracks the build, and the counters are readable in both configurations.
    ///
    /// The counts themselves are NOT asserted: they are process totals, every test in this binary
    /// contributes to them, and a threshold here would be a gate on how many other tests ran.
    #[test]
    fn a_snapshot_states_whether_anything_was_counting() {
        let s = snapshot();
        assert_eq!(
            s.armed,
            cfg!(feature = "profiling-alloc"),
            "`armed` must describe THIS build -- it is the field that tells a zero count from an \
             absent counter, and every other reading of the three numbers depends on it"
        );
        if !s.armed {
            assert_eq!(
                (s.allocs, s.deallocs, s.bytes),
                (0, 0, 0),
                "with no shim installed the counters can never move; a non-zero here would mean \
                 something else is writing them"
            );
        }
    }
}
