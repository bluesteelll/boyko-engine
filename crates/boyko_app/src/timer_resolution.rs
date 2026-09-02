//! The Windows timer resolution — the process-wide knob that decides what a short
//! `park_timeout` actually waits (KE16 App-12).
//!
//! # The quantity this changes
//!
//! Nothing here makes running code faster. It makes a **timed wait** expire close to when it was
//! asked to, and nothing else.
//!
//! Two of the engine's waits ask for microseconds and are answered in milliseconds. The threadpool
//! parks a worker with a 50 µs lost-wakeup backstop and the ECS scheduler's dispatcher parks with a
//! 100 µs one (`boyko_ecs`'s `schedule.rs` names the mechanism in its `PARK_TIMEOUT` doc). Neither
//! is a wait on this target: `std`'s `park_timeout` reaches `WaitOnAddress` through `dur2timeout`,
//! which rounds nanoseconds UP to whole milliseconds, and the kernel then quantises the expiry to
//! the system timer interrupt — ~15.6 ms by default. So a 50 µs backstop expires one to fifteen
//! milliseconds late: up to a whole frame at 60 Hz, from a wait that asked for a twentieth of a
//! millisecond.
//!
//! `timeBeginPeriod(1)` asks the OS for a 1 ms interrupt period for the whole process, which is the
//! only lever that moves the second of those two roundings.
//!
//! # What it costs, stated rather than implied
//!
//! A higher interrupt rate means the CPU wakes more often and cannot settle into its deep idle
//! C-states: higher idle draw and shorter battery life for as long as the request is held. That is
//! Microsoft's documented cost, it is real, and it is why `docs/threadpool/KE16-DESIGN.md` (the
//! "decided against" row in its §6 Q4 table) originally chose NOT to make this call. **The owner
//! weighed that cost and ruled on 2026-09-02 to take it**; this module is that ruling.
//!
//! # Why it lives in the host and nowhere else
//!
//! A library may not silently change process-wide state — a game that links `boyko_ecs` for its
//! editor tooling did not ask for a raised interrupt rate. So neither the pool nor the ECS calls
//! this; only `boyko_app`, the host layer, does, at the one seam whose lifetime is the run
//! (`plugins.rs`'s runner closure). This is the same rule `boot_and_enable_logging_from_env`
//! follows one module over: the host owns the process, the libraries own their data structures.

/// The period the engine asks for, in milliseconds.
///
/// 1 ms is the finest period Windows accepts through this API and the one every value in the
/// 50 µs / 100 µs pair rounds up to anyway — asking for less buys nothing, because `dur2timeout`
/// has already rounded the request to a whole millisecond before the kernel sees it.
pub const ENGINE_PERIOD_MS: u32 = 1;

/// The `winmm` surface, hand-declared in this repository's raw-FFI style (`boyko_rhi_vulkan`'s
/// `ffi.rs`, `boyko_ecs`'s `vm.rs`): no `windows`/`winapi` dependency, one block, one ABI note.
#[cfg(windows)]
mod win {
    // SAFETY: the signatures match the Win64 winmm ABI exactly — `timeBeginPeriod` and
    // `timeEndPeriod` are both `MMRESULT WINAPI f(UINT uPeriod)`, i.e. `u32 -> u32` under the
    // `"system"` convention. Unlike kernel32 and user32, `winmm` is NOT linked by std or by any
    // other crate in this workspace (one grep: this is the tree's only `winmm` reference), so the
    // `#[link]` attribute is load-bearing rather than decorative.
    #[link(name = "winmm")]
    unsafe extern "system" {
        pub fn timeBeginPeriod(uPeriod: u32) -> u32;
        pub fn timeEndPeriod(uPeriod: u32) -> u32;
    }

    /// `TIMERR_NOERROR` — the request was granted.
    pub const TIMERR_NOERROR: u32 = 0;

    /// `TIMERR_NOCANDO` — the period is outside the range this system supports. Named rather than
    /// compared against a bare `97` because a reader of the failure branch has to be able to tell
    /// which documented constant is being tested without leaving the file.
    pub const TIMERR_NOCANDO: u32 = 97;
}

/// An RAII hold on a raised system timer resolution: raises on construction, restores on `Drop`.
///
/// Windows refcounts the requests **per process** and documents that every `timeBeginPeriod` must
/// be matched by a `timeEndPeriod` **with the identical period**, so a guard that dropped without
/// releasing, or that released a period it never acquired, would leave the whole machine's
/// interrupt rate raised until the process exited. That mismatch is the entire bug this type
/// exists to make unrepresentable: the acquired period is stored, and `Drop` passes back exactly
/// what was stored.
///
/// # A refused raise is not an error
///
/// If `timeBeginPeriod` answers anything but success — `TIMERR_NOCANDO` is the documented refusal —
/// the guard records that it holds **nothing** and `Drop` calls nothing. Boot does not fail over
/// this: the engine runs, its timed waits are just as imprecise as they were before, and every
/// other subsystem is unaffected. Aborting a launch because a power-management hint was declined
/// would trade a working run for a dead one.
///
/// # Threading, stated rather than left to the reader
///
/// `Send` and `Sync` are both auto-derived and both correct. The API is process-wide and
/// thread-agnostic — the refcount belongs to the process, not to the calling thread — so a guard
/// may be created on one thread and dropped on another, and two threads may hold `&` to the same
/// guard. It is deliberately **not** `Clone`: a copy would duplicate the release without
/// duplicating the acquisition, which is precisely the imbalance above.
///
/// # Two guards at once are legal
///
/// The OS refcounts, so a nested or overlapping pair of guards is well defined and each one
/// releases its own request. No global "already raised" flag is imposed here, because such a flag
/// would make the second guard a silent no-op whose `Drop` then released the FIRST guard's hold at
/// the wrong time — a worse bug than the one it would be preventing.
#[derive(Debug)]
pub struct TimerResolutionGuard {
    /// The period `timeBeginPeriod` accepted, or `None` when nothing is held (the request was
    /// refused, or this is a non-Windows build). `Drop` is a no-op exactly when this is `None`.
    held_period_ms: Option<u32>,
}

impl TimerResolutionGuard {
    /// Requests `period_ms` milliseconds of timer resolution for the process.
    ///
    /// Never fails: a refused request yields a guard that holds nothing, which
    /// [`granted`](Self::granted) reports.
    #[cfg(windows)]
    pub fn new(period_ms: u32) -> Self {
        // A zero period is rejected by the OS with TIMERR_NOCANDO, so the release path stays
        // correct either way; the assert exists because asking for it is a caller bug, not a
        // configuration, and in release it vanishes.
        debug_assert!(
            period_ms >= 1,
            "invariant: timeBeginPeriod(0) is never grantable"
        );

        // SAFETY: `timeBeginPeriod` takes one `UINT` by value and returns one `MMRESULT` by value.
        // It touches no memory this process owns, so there is no pointer, lifetime or aliasing
        // invariant to uphold — the only obligation the API imposes is the BALANCE one, and that is
        // discharged by `Drop` below, which passes back exactly the value recorded here.
        let result = unsafe { win::timeBeginPeriod(period_ms) };
        let granted = result == win::TIMERR_NOERROR;

        // The API documents exactly two results for this entry point, so a third would mean the
        // ABI declaration above no longer matches the function being called — which is worth
        // knowing at the moment it starts being true rather than after some later release. Held to
        // `debug_assert!` deliberately: a shipped host must degrade to "unraised" on an
        // undocumented result, never abort a launch over a power-management hint.
        debug_assert!(
            granted || result == win::TIMERR_NOCANDO,
            "invariant: timeBeginPeriod returns TIMERR_NOERROR or TIMERR_NOCANDO; got {result}"
        );

        Self {
            held_period_ms: granted.then_some(period_ms),
        }
    }

    /// The non-Windows arm: POSIX timed waits (`futex(2)`, `pthread_cond_timedwait`) already carry
    /// nanosecond-resolution deadlines, bounded only by the scheduler's timer slack, so there is no
    /// process-wide resolution to raise and nothing for this constructor to do. It exists so the
    /// host's boot seam is one unconditional line on every target rather than a `cfg` fork.
    #[cfg(not(windows))]
    pub fn new(_period_ms: u32) -> Self {
        Self {
            held_period_ms: None,
        }
    }

    /// Requests [`ENGINE_PERIOD_MS`] — the engine's own period, and the call the host boot makes.
    #[inline]
    pub fn new_1ms() -> Self {
        Self::new(ENGINE_PERIOD_MS)
    }

    /// The period actually acquired, or `None` when the guard holds nothing.
    #[inline]
    pub fn held_period_ms(&self) -> Option<u32> {
        self.held_period_ms
    }

    /// Whether the request was granted. `false` on every non-Windows target, where there is
    /// nothing to grant.
    #[inline]
    pub fn granted(&self) -> bool {
        self.held_period_ms.is_some()
    }
}

impl Drop for TimerResolutionGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(period_ms) = self.held_period_ms {
            // SAFETY: same ABI argument as the acquisition above — a by-value `UINT` in, a
            // by-value `MMRESULT` out, no memory touched. `period_ms` is `Some` only on the path
            // where `timeBeginPeriod` returned `TIMERR_NOERROR` for that exact value, so this is
            // the matching half of a granted request and cannot release a hold the process does
            // not have. The result is discarded deliberately: the only documented failure is a
            // period that was never acquired, which the `Some` witness rules out, and a release
            // path has no caller left to report to.
            let _ = unsafe { win::timeEndPeriod(period_ms) };
        }
    }
}
