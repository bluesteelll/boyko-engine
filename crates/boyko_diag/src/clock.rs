//! Substrate A1 — the ONE clock, and the ONE [`SessionId`], that every diagnostics consumer
//! shares.
//!
//! # The consequence of not sharing this
//!
//! A suspend/resume produces a profiler window quarantined as an epoch break and, in the same
//! seconds, log lines whose printed wall times are wrong by the suspend duration with no marker
//! — two artifacts that disagree, neither of which says why. One clock, one epoch counter and
//! one session id remove that by construction rather than by two subsystems agreeing to keep
//! two copies in step.
//!
//! # No boot work
//!
//! Every static here is all-zero at const init and **nothing writes one at process start**.
//! [`calibrate`] runs on the *enable* path — whichever of the two consumers arms first — and is
//! idempotent and CAS-guarded, so "whichever thread calls first wins and the rest observe DONE"
//! is the contract at any call site whatsoever. Which call site that is, is not this module's
//! property to decide; the module is correct under either placement. With both subsystems off,
//! the clock is never calibrated and never read, and [`ticks_per_ns`]'s uncalibrated arm is
//! never taken because nothing stamps.
//!
//! # The mute leaf
//!
//! A condition observed here is **raised**, never emitted: [`ticks_per_ns`] on an uncalibrated
//! clock and [`note_forward_jump`] each set a sticky [`DiagFlag`] bit that a consumer folds at
//! its next opportunity. There is no `Debug` or `Display` impl on any type in this module, and
//! that is deliberate — a derive would pull `core::fmt` formatting into a crate whose whole
//! rule is that it formats nothing.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::loss::{raise, DiagFlag};

// ---------------------------------------------------------------------------------------------
// State layout
// ---------------------------------------------------------------------------------------------

/// The five read-mostly words the clock publishes, plus the two publication words that order
/// them.
///
/// One 64-byte line, because `ticks_per_ns` and `clock_epoch` are read *together* on every
/// record and every window fold — splitting them across two lines would double the read-side
/// footprint of the most frequent operation in either subsystem.
///
/// `DIAG_FLAGS` (see [`crate::loss`]) is deliberately **not** a field here: `raise` dirties it,
/// and a dirtied line shared with the clock would invalidate a line every hot reader touches.
#[repr(C, align(64))]
struct ClockGlobals {
    /// `f64::to_bits` of the probed scale — `f64` has no atomic type, so the bits are carried in
    /// an integer and reinterpreted on read.
    ticks_per_ns_bits: AtomicU64,
    /// Low half of the 128-bit session id. Ordered by `session_state`, never on its own.
    session_lo: AtomicU64,
    /// High half of the 128-bit session id. Ordered by `session_state`, never on its own.
    session_hi: AtomicU64,
    /// Bumped once per detected discontinuity; the only monotone counter in the struct.
    epoch: AtomicU32,
    /// Publication word for `ticks_per_ns_bits`. `PHASE_IDLE` reads as UNCALIBRATED here.
    state: AtomicU32,
    /// Result of the invariant-TSC probe: `INVARIANT_UNPROBED` / `_NO` / `_YES`.
    invariant: AtomicU32,
    /// Publication word for the `session_lo`/`session_hi` pair. `PHASE_IDLE` reads as UNMINTED.
    ///
    /// Without a third word there is nothing for a reader to acquire against, and a half-written
    /// id `(lo, 0)` — a 128-bit value equal to no mint — is observable. It is paid for out of
    /// padding that was already there, so no second word of state crosses the line.
    session_state: AtomicU32,
    /// Fills the line out to 64 B. 8 + 8 + 8 + 4 + 4 + 4 + 4 = 40 B of state; 40 + 24 = 64.
    _pad: [u8; 24],
}

// The doc above states 64 B by arithmetic. Arithmetic over a `#[repr(C)]` layout is a claim the
// compiler can settle, so it settles it — a field added to the struct without shrinking `_pad`
// fails the build here rather than silently costing every reader a second cache line.
const _: () = assert!(core::mem::size_of::<ClockGlobals>() == 64);
const _: () = assert!(core::mem::align_of::<ClockGlobals>() == 64);

/// All-zero at const init, so the linker emits a virtual size with **no raw data**.
///
/// That is the whole of the residency claim. Whether the OS leaves the page uncommitted until it
/// is touched is UNPROVEN and is not asserted here.
static CLOCK: ClockGlobals = ClockGlobals {
    ticks_per_ns_bits: AtomicU64::new(0),
    session_lo: AtomicU64::new(0),
    session_hi: AtomicU64::new(0),
    epoch: AtomicU32::new(0),
    state: AtomicU32::new(0),
    invariant: AtomicU32::new(0),
    session_state: AtomicU32::new(0),
    _pad: [0; 24],
};

// `state` and `session_state` are two instances of one shape: a one-shot publication word whose
// writer transitions IDLE -> BUSY -> DONE and whose readers acquire on DONE. The spec names the
// phases twice — UNCALIBRATED/RUNNING/DONE for `state`, UNMINTED/MINTING/DONE for
// `session_state` — but the values and the protocol are identical, which is why one `await_done`
// serves both. Naming them once keeps the two protocols from drifting apart.

/// UNCALIBRATED for `state`, UNMINTED for `session_state`. Zero, so the static is `.bss`.
const PHASE_IDLE: u32 = 0;
/// RUNNING for `state`, MINTING for `session_state`: a winner is between the CAS and the store.
const PHASE_BUSY: u32 = 1;
/// The published state. Everything the winner wrote before storing this is visible to any
/// reader that loads it with `Acquire`.
const PHASE_DONE: u32 = 2;

/// The probe has not run. Distinct from `INVARIANT_NO` so that "we asked and the answer was no"
/// is not confused with "nobody asked".
const INVARIANT_UNPROBED: u32 = 0;
/// The probe ran and the CPU does not advertise an invariant TSC.
const INVARIANT_NO: u32 = 1;
/// The probe ran and CPUID.80000007H:EDX[8] was set.
const INVARIANT_YES: u32 = 2;

// ---------------------------------------------------------------------------------------------
// Session identity
// ---------------------------------------------------------------------------------------------

/// A 128-bit identifier minted **once per process**, here, and stamped into both artifact
/// headers so a reader can prove two files came from the same run.
///
/// Neither subsystem mints its own; a per-crate `session.rs` is the shape this crate exists to
/// prevent. There is deliberately no `Debug` impl — see the module docs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionId(
    /// Low half — derived from the wall clock and the tick counter.
    pub u64,
    /// High half — derived from address entropy. Always odd, see [`mint_bits`].
    pub u64,
);

// ---------------------------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------------------------

/// The raw tick counter: `rdtsc` on x86-64, a monotone `Instant` delta everywhere else.
///
/// Ticks are only comparable within one [`clock_epoch`]; across an epoch bump the counter may
/// have jumped and the difference means nothing.
#[inline]
pub fn ticks() -> u64 {
    backend::ticks()
}

/// The tick-to-nanosecond scale published by [`calibrate`].
///
/// On an uncalibrated clock this returns `1.0` and raises [`DiagFlag::ClockUncalibrated`] — a
/// stamp taken before the enable path ran is still a number, and the flag is what tells the
/// consumer not to trust its magnitude.
#[inline]
pub fn ticks_per_ns() -> f64 {
    // The `Acquire` here is what the `Release` in `calibrate` pairs with: it is what makes the
    // probed scale visible to every later reader on every thread.
    if CLOCK.state.load(Ordering::Acquire) != PHASE_DONE {
        return uncalibrated_scale();
    }
    // `Relaxed` suffices because the `Acquire` on `state` already ordered this load.
    f64::from_bits(CLOCK.ticks_per_ns_bits.load(Ordering::Relaxed))
}

/// The uncalibrated arm, kept out of line so the calibrated read is a load, a compare and a
/// second load with nothing else in the instruction stream.
#[cold]
#[inline(never)]
fn uncalibrated_scale() -> f64 {
    raise(DiagFlag::ClockUncalibrated);
    1.0
}

/// The current epoch. Incremented by [`note_forward_jump`] on a detected discontinuity.
///
/// Two ticks are comparable only if they were taken in the same epoch; a consumer that records
/// the epoch alongside a stamp can quarantine exactly the window that straddles a suspend.
#[inline]
pub fn clock_epoch() -> u32 {
    // Pairs with the `Release` in `note_forward_jump`: a consumer that observes the incremented
    // epoch also observes everything the detector recorded before bumping it. That is what makes
    // a record straddling the bump legible on both sides.
    CLOCK.epoch.load(Ordering::Acquire)
}

/// Whether the CPU advertises an invariant TSC (CPUID.80000007H:EDX[8]), probed once and cached.
///
/// `false` on every non-x86-64 target and under Miri, where the backend is an `Instant` delta
/// rather than a tick counter and the question does not arise.
#[inline]
pub fn invariant_tsc() -> bool {
    // `Relaxed` is correct and the asymmetry with `session_state` is principled: this is ONE
    // word, so it cannot tear, and CPUID is deterministic, so a thread that loses the race
    // stores exactly the value the winner stored. Neither property holds for a 128-bit id split
    // across two words, which is why that pair has a publication word and this does not.
    match CLOCK.invariant.load(Ordering::Relaxed) {
        INVARIANT_UNPROBED => probe_invariant(),
        INVARIANT_YES => true,
        _ => false,
    }
}

/// The process's session id, minted on first touch.
#[inline]
pub fn session_id() -> SessionId {
    // Every reader — winner or loser — loads the two halves only AFTER this `Acquire`, which
    // pairs with the `Release` in `mint_session`. Without that pairing the halves are two
    // independent atomics and a reader concurrent with the mint can observe `(lo, 0)`.
    if CLOCK.session_state.load(Ordering::Acquire) == PHASE_DONE {
        return read_session();
    }
    mint_session()
}

/// Loads the published halves. Callers must have acquired `session_state == PHASE_DONE` first.
#[inline]
fn read_session() -> SessionId {
    debug_assert_eq!(
        CLOCK.session_state.load(Ordering::Relaxed),
        PHASE_DONE,
        "invariant: the session halves are only legible after session_state reads DONE"
    );
    SessionId(
        CLOCK.session_lo.load(Ordering::Relaxed),
        CLOCK.session_hi.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------------------------------
// Writes — all of them cold, all of them one-shot
// ---------------------------------------------------------------------------------------------

/// Probes the tick-to-nanosecond scale and publishes it. Idempotent; call it from the enable
/// path.
///
/// Costs ~20 ms of wall time **on the one thread that wins the CAS**; every other caller returns
/// as soon as that thread publishes. Calling it a second time is free.
#[cold]
#[inline(never)]
pub fn calibrate() {
    if CLOCK
        .state
        .compare_exchange(
            PHASE_IDLE,
            PHASE_BUSY,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
    {
        #[cfg(test)]
        CALIBRATE_PROBES.fetch_add(1, Ordering::Relaxed);

        let bits = backend::probe_ticks_per_ns().to_bits();
        CLOCK.ticks_per_ns_bits.store(bits, Ordering::Relaxed);
        // The `Release` that `ticks_per_ns`'s `Acquire` pairs with. It is what makes the probed
        // scale visible; the `Relaxed` store above rides on it.
        CLOCK.state.store(PHASE_DONE, Ordering::Release);
        return;
    }
    await_done(&CLOCK.state);
}

/// Records a detected forward discontinuity: bumps the epoch and raises the sticky flag.
///
/// `observed` is the tick value the detector saw. It is **accepted and not stored**, and the
/// reason is structural rather than an oversight: [`ClockGlobals`] is exactly one cache line
/// with no free word (the const assert above pins that), and the mute-leaf rule forbids this
/// crate from emitting the value itself. The detector — the only caller — already holds it and
/// is the only party that can attribute it to a window, so nothing is lost today; the parameter
/// is in the published signature so a consumer-side sink can be added without breaking callers.
#[cold]
#[inline(never)]
pub fn note_forward_jump(observed: u64) {
    let _ = observed;
    // `Release` so that a consumer which observes the incremented epoch also observes the
    // counters the detector wrote before bumping it. On x86-64 this lowers to a plain locked
    // add either way; it is written correctly rather than relying on the ISA.
    CLOCK.epoch.fetch_add(1, Ordering::Release);
    raise(DiagFlag::ClockEpochBreak);
}

/// Runs the invariant-TSC probe once and caches the answer.
#[cold]
#[inline(never)]
fn probe_invariant() -> bool {
    let yes = backend::invariant_tsc_raw();
    CLOCK.invariant.store(
        if yes { INVARIANT_YES } else { INVARIANT_NO },
        Ordering::Relaxed,
    );
    yes
}

/// Mints the session id, or waits for the thread that is minting it.
#[cold]
#[inline(never)]
fn mint_session() -> SessionId {
    if CLOCK
        .session_state
        .compare_exchange(
            PHASE_IDLE,
            PHASE_BUSY,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
    {
        #[cfg(test)]
        SESSION_MINTS.fetch_add(1, Ordering::Relaxed);

        let (lo, hi) = mint_bits();
        CLOCK.session_lo.store(lo, Ordering::Relaxed);
        CLOCK.session_hi.store(hi, Ordering::Relaxed);
        // The publication the whole pair hangs on. "Minted once" binds the COUNT, not the
        // publication: a CAS on `session_lo` alone satisfies "once" and still leaves
        // `session_hi` unordered with respect to it.
        CLOCK.session_state.store(PHASE_DONE, Ordering::Release);
        return SessionId(lo, hi);
    }
    await_done(&CLOCK.session_state);
    read_session()
}

/// Spins until `word` publishes, yielding between attempts.
///
/// **No `Mutex`** (clippy `disallowed-types`, Principle 4): a CAS plus a yielding spin is the
/// compliant shape here, and it is only ever reached on the enable path by a thread that lost a
/// one-shot race. `spin_loop` keeps the retry cheap while the winner runs on another core;
/// `yield_now` is what stops a loser from burning a core for the winner's whole 20 ms probe on a
/// machine that has fewer cores than enablers.
#[cold]
#[inline(never)]
fn await_done(word: &AtomicU32) {
    while word.load(Ordering::Acquire) != PHASE_DONE {
        core::hint::spin_loop();
        thread::yield_now();
    }
}

/// Derives the 128-bit session id.
///
/// Three independent sources, because no one of them separates every pair of runs: the wall
/// clock separates two runs of one machine, the tick counter separates two runs starting inside
/// one wall-clock nanosecond, and the two addresses separate two processes started in the same
/// nanosecond on any host with ASLR.
fn mint_bits() -> (u64, u64) {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let tsc = ticks();
    let static_addr = (&raw const CLOCK) as usize as u64;
    let stack_addr = (&raw const wall) as usize as u64;

    let lo = splitmix64(wall ^ tsc.rotate_left(32));
    // Bit 0 is forced so that no minted id can ever equal a half-published `(lo, 0)`. The
    // publication word already makes that pair unobservable; forcing the bit makes it
    // unrepresentable, so a future reader that loads the halves without the `Acquire` still
    // cannot mistake a torn read for a real id. One bit of entropy is a cheap price for turning
    // an ordering obligation into a structural one.
    let hi = splitmix64(static_addr ^ stack_addr.rotate_left(17) ^ tsc.rotate_left(48)) | 1;
    (lo, hi)
}

/// SplitMix64's finalizer — a bijection, so it cannot collide two distinct inputs, and it
/// diffuses the low-entropy high bits of a timestamp across the whole word.
#[inline]
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ---------------------------------------------------------------------------------------------
// Backends — exactly two arms, and NEITHER is FFI. That is what makes the zero-dependency claim
// hold: a hand-declared `QueryPerformanceCounter` would be a second per-OS backing
// implementation, which is the breach the never-freed-storage boundary exists to prevent. On
// Windows `Instant` IS QPC internally, so the fallback is honoured without a `windows-sys` edge.
//
// Miri is excluded from the intrinsic arm because it has no x86 intrinsic support. The arm's
// correctness therefore rests on the SAFETY argument below, NOT on a test — stated rather than
// papered over.
// ---------------------------------------------------------------------------------------------

#[cfg(all(target_arch = "x86_64", not(miri)))]
mod backend {
    use std::thread;
    use std::time::{Duration, Instant};

    /// Number of independent probes. The **median** is taken rather than the mean so that one
    /// probe preempted by the scheduler is outvoted instead of averaged in.
    const PROBES: usize = 16;
    /// 16 probes over 20 ms total.
    const PROBE_SPAN: Duration = Duration::from_nanos(20_000_000 / PROBES as u64);

    #[inline]
    pub(super) fn ticks() -> u64 {
        // SAFETY: the `#[cfg(target_arch = "x86_64")]` gate guarantees the RDTSC instruction
        // exists (architectural on x86-64 since its introduction; no CPUID feature bit gates its
        // PRESENCE, only its invariance). The intrinsic has no memory operands, reads no pointer
        // and has no side effects, so it cannot violate any aliasing or initialisation invariant.
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    /// Reads CPUID.80000007H:EDX[8].
    ///
    /// The two-step probe is mandatory, not defensive: a single-step read of leaf `0x8000_0007`
    /// on a CPU that does not implement it returns the highest leaf it *does* implement, and bit
    /// 8 of that unrelated word is then reported as "invariant TSC". `__cpuid` is a safe `fn` on
    /// this toolchain — it writes no memory and takes no pointer — so the guard below is a
    /// CORRECTNESS obligation rather than a soundness one, and carries no `unsafe` block.
    pub(super) fn invariant_tsc_raw() -> bool {
        let max_extended_leaf = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        if max_extended_leaf < 0x8000_0007 {
            return false;
        }
        core::arch::x86_64::__cpuid(0x8000_0007).edx & (1 << 8) != 0
    }

    /// Measures ticks per nanosecond against `Instant`, which is the OS's own timebase.
    pub(super) fn probe_ticks_per_ns() -> f64 {
        let mut rates = [0.0f64; PROBES];
        for slot in rates.iter_mut() {
            // `Instant` brackets the tick reads on the outside, so the measured nanosecond span
            // is a superset of the measured tick span and the ratio errs low by the cost of two
            // `rdtsc` reads — tens of nanoseconds against a 1.25 ms span.
            let t0 = Instant::now();
            let c0 = ticks();
            thread::sleep(PROBE_SPAN);
            let c1 = ticks();
            let ns = t0.elapsed().as_nanos();
            // A probe that observed a zero-width or backwards span measured nothing. Scoring it
            // `0.0` sorts it below every real sample, where the median cannot see it unless MOST
            // probes failed — and if most failed, the guarded fallback below is the honest answer
            // rather than a number derived from garbage.
            *slot = if ns > 0 && c1 > c0 {
                (c1 - c0) as f64 / ns as f64
            } else {
                0.0
            };
        }
        rates.sort_unstable_by(f64::total_cmp);
        let median = (rates[PROBES / 2 - 1] + rates[PROBES / 2]) * 0.5;
        if median.is_finite() && median > 0.0 {
            median
        } else {
            1.0
        }
    }
}

#[cfg(not(all(target_arch = "x86_64", not(miri))))]
mod backend {
    use std::sync::OnceLock;
    use std::time::Instant;

    /// Minted on the first tick read, never at process start — the `OnceLock` is all-zero at
    /// const init and stays untouched in a process that enables no diagnostics.
    static BASE: OnceLock<Instant> = OnceLock::new();

    #[inline]
    pub(super) fn ticks() -> u64 {
        // `Instant` is monotone by std's own guarantee, so the subtraction cannot go backwards.
        BASE.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }

    pub(super) fn invariant_tsc_raw() -> bool {
        // There is no tick counter on this arm, so there is nothing whose invariance to claim.
        false
    }

    /// This arm's tick **is** a nanosecond, so the scale is exact by construction. Probing it
    /// would replace an exact constant with a sampled estimate of the same constant.
    pub(super) fn probe_ticks_per_ns() -> f64 {
        1.0
    }
}

// ---------------------------------------------------------------------------------------------
// Test-only instrumentation
//
// These count the ONE-SHOT paths, not the entry points: a counter on entry would only tell a
// test how many times it called a function it just called, whereas a counter inside the CAS
// winner's branch is exactly the quantity the idempotence claim is about, and a test asserting
// it is 1 reds the moment the guard is removed. They are process-global and monotone, so they
// are immune to the order the test harness happens to run tests in.
// ---------------------------------------------------------------------------------------------

/// Number of times a thread won the calibration CAS and actually probed.
#[cfg(test)]
static CALIBRATE_PROBES: AtomicU32 = AtomicU32::new(0);

/// Number of times a thread won the session CAS and actually minted.
#[cfg(test)]
static SESSION_MINTS: AtomicU32 = AtomicU32::new(0);

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    use super::*;

    /// The published scale must reconstruct a duration the OS measured independently.
    ///
    /// This is the assertion that catches an inverted ratio (ns-per-tick published as
    /// ticks-per-ns), which no plausibility band can catch: on a 3 GHz TSC the inverse is 0.33,
    /// comfortably inside any honest band, but it reconstructs the interval ~9x wrong.
    #[test]
    fn published_scale_reconstructs_an_independently_measured_interval() {
        calibrate();

        let t0 = Instant::now();
        let c0 = ticks();
        thread::sleep(Duration::from_millis(5));
        let c1 = ticks();
        let measured_ns = t0.elapsed().as_nanos() as f64;

        let scale = ticks_per_ns();
        let reconstructed_ns = (c1 - c0) as f64 / scale;
        let relative_error = (reconstructed_ns - measured_ns).abs() / measured_ns;

        // A non-invariant TSC may be rescaled by the CPU mid-interval or read from a different
        // core, so the tight band is only claimed where the hardware says it is claimable. The
        // loose band still fails an inversion (~9x) and a missing calibration (~1000x).
        let band = if invariant_tsc() { 0.05 } else { 0.25 };
        assert!(
            relative_error < band,
            "scale {scale} reconstructs {reconstructed_ns} ns for a {measured_ns} ns interval \
             (relative error {relative_error}, band {band})"
        );
    }

    /// The CAS guard, not the caller, is what makes repeated calibration free — and the guard's
    /// whole question is whether two threads can both fall through it, which a serial test
    /// cannot ask.
    #[test]
    fn calibrate_probes_exactly_once_under_a_concurrent_race() {
        const RACERS: usize = 8;

        thread::scope(|scope| {
            for _ in 0..RACERS {
                scope.spawn(calibrate);
            }
        });
        // A later serial call must also be free, which is the other half of idempotence.
        calibrate();

        assert_eq!(
            CALIBRATE_PROBES.load(Ordering::Relaxed),
            1,
            "calibration probed more than once: the CAS guard is not holding"
        );
        assert_eq!(CLOCK.state.load(Ordering::Relaxed), PHASE_DONE);
        let scale = ticks_per_ns();
        assert!(
            scale.is_finite() && scale > 0.0,
            "published scale {scale} is not a usable number"
        );
    }

    /// The publication word — not the CAS — is what a racing reader depends on: "minted once"
    /// binds the COUNT, and a `session_lo` CAS alone satisfies "once" while leaving `session_hi`
    /// unordered with respect to it. Every racer must come back with the same 128-bit value.
    ///
    /// **This is the ONLY test in this module that touches `session_id()`, and that is
    /// load-bearing.** The mint is one-shot per process, so a sibling test that read the id
    /// first would leave these threads racing nothing but the already-published fast path — the
    /// gate would be green because it could not fail. Measured: with a second serial reader in
    /// the binary, a deliberately mis-ordered publication went undetected.
    ///
    /// **The spin barrier is load-bearing too, and was added because the test without it did not
    /// work.** Measured: with the threads merely spawned in a loop, a deliberately mis-ordered
    /// publication (store DONE, yield, then store `session_hi`) was detected in **0 of 20** runs
    /// — thread creation costs microseconds and the winner had always finished before the second
    /// racer arrived, so nobody was ever inside `await_done` when the flawed publication landed.
    /// Releasing all racers from one barrier puts the losers in that spin at the instant the
    /// winner publishes, which is the only state from which the defect is observable.
    ///
    /// It is still a probabilistic red, not a deterministic one; the deterministic instrument is
    /// a loom model, and this crate carries no loom wiring at D0.
    #[test]
    fn session_id_is_identical_across_racing_threads() {
        const RACERS: usize = 8;

        let mut ids = [SessionId(0, 0); RACERS];
        let arrived = AtomicUsize::new(0);
        thread::scope(|scope| {
            for slot in ids.iter_mut() {
                let arrived = &arrived;
                scope.spawn(move || {
                    arrived.fetch_add(1, Ordering::AcqRel);
                    while arrived.load(Ordering::Acquire) < RACERS {
                        core::hint::spin_loop();
                    }
                    *slot = session_id();
                });
            }
        });

        assert_eq!(
            SESSION_MINTS.load(Ordering::Relaxed),
            1,
            "the session was minted more than once: the CAS guard is not holding"
        );
        for id in &ids {
            assert!(
                *id == ids[0],
                "two racing threads observed different session ids"
            );
            assert_ne!(
                id.1, 0,
                "a racer observed a half-published (lo, 0): the publication word is not ordering \
                 the two halves"
            );
        }
        // The forced low bit, checked once: it is what makes `(lo, 0)` unrepresentable rather
        // than merely unobservable.
        assert_eq!(ids[0].1 & 1, 1, "the high half of a minted id must be odd");
        // A later read must return the same value from the published fast path.
        assert!(session_id() == ids[0], "the session id changed after the mint");
    }

    /// Monotonicity is the one property both backends must share, and it is the property a
    /// window duration is computed from.
    #[test]
    fn ticks_never_goes_backwards_on_one_thread() {
        let mut previous = ticks();
        for _ in 0..4096 {
            let current = ticks();
            assert!(
                current >= previous,
                "tick counter went backwards: {previous} -> {current}"
            );
            previous = current;
        }
    }

    /// The probe must *cache*, not merely answer: an uncached probe leaves the word UNPROBED and
    /// pays CPUID on every call from every hot stamp site that asks.
    #[test]
    fn invariant_tsc_is_probed_once_and_answers_consistently() {
        let first = invariant_tsc();
        assert_ne!(
            CLOCK.invariant.load(Ordering::Relaxed),
            INVARIANT_UNPROBED,
            "the probe answered without caching its answer"
        );
        assert_eq!(first, invariant_tsc());
    }

    /// The only test in this module that calls `note_forward_jump`, so the delta is exact
    /// without coordinating with the harness's thread pool.
    #[test]
    fn note_forward_jump_bumps_the_epoch_by_exactly_one() {
        let before = clock_epoch();
        note_forward_jump(ticks());
        assert_eq!(clock_epoch(), before + 1);
    }
}
