//! A2 — the lane topology: the ONE per-thread integer both diagnostics subsystems index by.
//!
//! # The consequence of not sharing this
//!
//! The same worker is lane 5 to the profiler and lane 37 to the logger, so no reader can place a
//! log line inside the zone it happened in — the one joint question the pair exists to answer
//! becomes unanswerable by construction. What this module owns is the topology and its two
//! names, [`lane`] and [`LANE_COUNT`]. The renames each subsystem must make to arrive at them
//! are that subsystem's own work: a bottom-layer module that enumerated its consumers' edits
//! would go stale on the next rename of a type it does not own.
//!
//! # Why a second TLS slot exists at all
//!
//! The obvious design — derive the lane from `boyko_threadpool::tls::current_worker_id()` and
//! hold no state — is **impossible**: `boyko_diag` sits *below* `boyko_threadpool` and cannot
//! name it. So the slot is written from above, and a worker thread carries **two** TLS `Cell`s
//! after D1: the pool's own worker id and this crate's `LANE`.
//!
//! The divergence risk that creates is closed by **co-location**, not by a runtime check: every
//! [`set_lane`] call sits immediately beside an existing `set_current_worker_id` call, so an edit
//! that moves one and not the other shows up in a two-line diff. Gate `DG3` asserts the two
//! agree; gate `DG4` — a `const` assert that lives in `boyko_threadpool`, because this crate
//! cannot name `MAX_WORKERS` — asserts [`LANE_WORKER_MAX`] still equals it.
//!
//! # The write sites are supplied by D1, not by this module
//!
//! [`set_lane`] has three production callers, all in `boyko_threadpool`: `worker_main`, the
//! `install` entry, and `InstallGuard::drop`. The third is the one an implementer working from
//! the decision record's "two sites" would miss, and it is load-bearing: it restores the lane on
//! the **unwinding** path, so a dispatcher thread that panics inside `install` does not stay
//! labelled [`LANE_DISPATCHER`] for the rest of the process with every later diagnostic from it
//! misattributed.
//!
//! # No `Drop`
//!
//! `LANE` is a `Cell<u16>` with a `const` initialiser, so the `thread_local!` expansion has no
//! lazy-init flag and **registers no destructor**. That is the mechanism that turns logging's
//! "at most one allocation on first emit" into zero, and it is why [`release_lane`] is explicit
//! rather than automatic. Reinstating a `Drop`-carrying TLS guard here is the showable RED on
//! logging's zero-allocation leg.
//!
//! MEASURED consequence, for whoever writes a consumer: on `x86_64-pc-windows-gnu`, a *different*
//! thread-local that does register a destructor can read `LANE` as [`LANE_UNCLAIMED`] from inside
//! that destructor even though the thread set it earlier. The result is deterministic per binary
//! and differs between binaries, so it is not a race — it is TLS teardown order, and it means a
//! consumer must not attribute end-of-thread work by calling [`lane`] from a TLS destructor. It
//! must capture the lane while the thread is live.
//!
//! # No boot work
//!
//! Nothing here runs at process start. `SPARE_OWNER` is all-zero, so it is `.bss` and no
//! initialiser touches it; `LANE`'s `const` initialiser is a compile-time value, not code. The
//! first byte this module writes is written by the first [`set_lane`] or [`claim_lane`] call.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::loss::{DiagFlag, raise};

/// Number of worker lanes, `0..LANE_WORKER_MAX`. Equals `boyko_threadpool::MAX_WORKERS`.
///
/// The equality is asserted where it can be: a `const` assert in `boyko_threadpool` (gate `DG4`),
/// because the bottom crate cannot name the constant it must match. A comment there would not be
/// a gate.
pub const LANE_WORKER_MAX: u16 = 64;

/// The lane of the thread currently inside `ThreadPool::install` — the dispatcher.
pub const LANE_DISPATCHER: u16 = 64;

/// The lane of the host thread that drives the frame loop.
pub const LANE_HOST: u16 = 65;

/// First claimable lane. Lanes `LANE_SPARE_BASE..LANE_COUNT` are handed out by [`claim_lane`].
pub const LANE_SPARE_BASE: u16 = 66;

/// Total lane count, **in every build profile — there is deliberately no profile axis**.
///
/// The arithmetic is the topology's own: 64 workers (`0..63`) + dispatcher (64) + host (65) = 66,
/// plus 14 claimable spares = 80.
///
/// An earlier design made this profile-dependent (80 in dev, 32 in shipping) while the quantity
/// it indexes — `boyko_threadpool::MAX_WORKERS` — is **unconditional**. That is unsound twice
/// over: a shipping build on a machine with more than 32 hardware threads produces lane indices
/// past the end of every array sized by this constant, and [`LANE_HOST`] is already out of range
/// at 32 on *every* machine. Shrinking `MAX_WORKERS` per profile instead would cap the shipped
/// engine's worker count to save memory for a subsystem that is off by default — the engine's
/// first principle inverted for a disabled feature. A single constant closes the whole class by
/// construction, which per-profile values cannot: they re-open it the moment one side is edited.
///
/// The cost is `LANE_COUNT × LossClass::COUNT × 64 B` of loss-cell `.bss` in every profile. What
/// makes that affordable in shipping is that an unclaimed lane is never touched, so the pages
/// cost reserved address space rather than resident memory — see `storage` for the limit of what
/// this crate can actually prove about that.
pub const LANE_COUNT: u16 = 80;

/// The lane of a thread that has none: not a pool thread, not the host, and holding no spare.
///
/// Deliberately `u16::MAX` rather than a value inside the topology, so that a stale or
/// uninitialised lane can never be mistaken for a real one and can never index an array.
pub const LANE_UNCLAIMED: u16 = u16::MAX;

/// Number of claimable spare lanes.
const SPARE_COUNT: usize = (LANE_COUNT - LANE_SPARE_BASE) as usize;

/// A spare slot nobody holds. **Must stay 0**: it is what puts `SPARE_OWNER` in `.bss` and keeps
/// the no-boot-work property (`DG12`) true without an initialiser.
const SPARE_FREE: u32 = 0;

/// A spare slot somebody holds.
const SPARE_CLAIMED: u32 = 1;

// The topology is contiguous and every lane index is a valid array index. These are `const`
// asserts rather than unit tests on purpose: a unit test for a property the compiler can decide
// is a test that cannot fail, because the build would have failed first.
const _: () = assert!(LANE_DISPATCHER == LANE_WORKER_MAX);
const _: () = assert!(LANE_HOST == LANE_DISPATCHER + 1);
const _: () = assert!(LANE_SPARE_BASE == LANE_HOST + 1);
const _: () = assert!(LANE_SPARE_BASE < LANE_COUNT);
// NO assert that the sentinel is outside the id range. `LANE_UNCLAIMED` IS `u16::MAX` and every
// lane id is a `u16`, so `LANE_UNCLAIMED >= LANE_COUNT` is decided by the TYPE and cannot fail
// for any value of `LANE_COUNT` — clippy's `absurd_extreme_comparisons` says so, and it is
// right. Writing it anyway would have added a fifth line to a block whose whole point is that
// each line CAN fail, and a check that cannot fail next to four that can is how a reader learns
// to skim the block. The property holds; the guarantee is the type, not an assertion.

/// Occupancy of the claimable spare lanes, one word per spare, `SPARE_FREE` or `SPARE_CLAIMED`.
///
/// Deliberately **packed**, not padded apart: all 14 words are read by the same cold scan, so
/// padding them onto separate lines would spend 14 cache lines to prevent false sharing on a path
/// that runs once per thread.
///
/// The array stores occupancy, not owner identity. A thread id here would be a second copy of
/// "who am I" to keep in sync with the TLS slot, and nothing reads it.
static SPARE_OWNER: [AtomicU32; SPARE_COUNT] = [const { AtomicU32::new(SPARE_FREE) }; SPARE_COUNT];

thread_local! {
    /// This thread's lane. `Cell<u16>` with a `const` initialiser — see the module docs on why
    /// there is no `Drop` here and why that is load-bearing rather than incidental.
    static LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) };
}

/// This thread's lane, or [`LANE_UNCLAIMED`].
///
/// One TLS read. Both subsystems index their per-thread state by this single integer, which is
/// the whole point of the module.
#[inline]
pub fn lane() -> u16 {
    LANE.get()
}

/// Label this thread with `id`.
///
/// **`boyko_threadpool` only**, at the three sites named in the module docs. It writes the TLS
/// `Cell` and nothing else — it does not calibrate, does not claim a spare, does not touch a loss
/// cell — so a worker thread starting with diagnostics off leaves every shared static untouched.
///
/// Threads outside the pool go through [`claim_lane`] instead; they must not pick a lane
/// themselves.
#[inline]
pub fn set_lane(id: u16) {
    debug_assert!(
        id < LANE_COUNT || id == LANE_UNCLAIMED,
        "invariant: a lane is a topology index below LANE_COUNT, or LANE_UNCLAIMED"
    );
    LANE.set(id);
}

/// Claim a spare lane for a thread the pool did not create — asset I/O, a script VM, a mod, an
/// OS or driver callback.
///
/// On success the thread's lane is set and the id returned. **On exhaustion this returns `None`,
/// the caller stays [`LANE_UNCLAIMED`], and [`DiagFlag::LaneExhausted`] is raised — never a
/// panic, never a block.** Exhaustion is non-terminal by design: losing attribution for one
/// thread is a diagnostics degradation, and a diagnostics degradation may not become an engine
/// failure.
///
/// Call at most once per thread and pair it with [`release_lane`]; a second claim without a
/// release strands a spare for the process.
///
/// # Cost
///
/// The scan no longer spreads claimants by thread-id hash, so concurrent claimants **convoy** on
/// the first free slot. Bounded by `LANE_COUNT - LANE_SPARE_BASE` compare-exchanges on a path
/// taken once per thread, which is why it is `#[cold]` and why the convoy is acceptable.
#[cold]
#[inline(never)]
pub fn claim_lane() -> Option<u16> {
    debug_assert_eq!(
        lane(),
        LANE_UNCLAIMED,
        "invariant: claim_lane is called once per thread; a second claim strands a spare"
    );

    for (i, slot) in SPARE_OWNER.iter().enumerate() {
        // Load first so a claimed slot is skipped without dirtying its line. The scan is cold,
        // but a store to every occupied word would invalidate them for their holders.
        if slot.load(Ordering::Relaxed) != SPARE_FREE {
            continue;
        }
        loop {
            // The `Acquire` on success pairs with the `Release` in `release_lane`. That pairing
            // is what guarantees a new claimant of a recycled lane observes the retiring owner's
            // final writes to that lane's loss cells before it starts writing them itself.
            match slot.compare_exchange_weak(
                SPARE_FREE,
                SPARE_CLAIMED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let id = LANE_SPARE_BASE + i as u16;
                    set_lane(id);
                    return Some(id);
                }
                // Spurious failure: the slot is still free, so retry *this* slot. The corpus
                // justifies the weak form with "the scan is a loop anyway", which does not hold
                // — the outer scan moves on rather than retrying, so a spurious failure would
                // report exhaustion with spares still free. This inner retry is what makes the
                // weak form correct.
                Err(SPARE_FREE) => continue,
                // Genuinely taken by another claimant while we looked. Next slot.
                Err(_) => break,
            }
        }
    }

    // The mute-leaf rule: observe here, report above. No code literal, no print, no format — the
    // consumer's next fold turns this bit into a diagnostic.
    raise(DiagFlag::LaneExhausted);
    None
}

/// Return this thread's spare lane to the pool of spares.
///
/// A no-op unless this thread holds a spare: worker, dispatcher and host lanes are owned by
/// whoever wrote them, and an unclaimed thread has nothing to give back.
///
/// The TLS slot is cleared **before** the slot is published as free, so no thread is ever
/// labelled with a lane another thread may already have claimed.
#[cold]
#[inline(never)]
pub fn release_lane() {
    let id = lane();
    if !(LANE_SPARE_BASE..LANE_COUNT).contains(&id) {
        return;
    }
    set_lane(LANE_UNCLAIMED);
    // `Release` pairs with the `Acquire` in `claim_lane`: it publishes this owner's final writes
    // to the lane's loss cells to whoever claims the recycled lane next.
    SPARE_OWNER[(id - LANE_SPARE_BASE) as usize].store(SPARE_FREE, Ordering::Release);
}

/// Spare lanes currently outstanding — claimed and not yet released.
///
/// A thread that never calls [`release_lane`] holds its spare for the process. That is bounded by
/// `LANE_COUNT - LANE_SPARE_BASE` and is not an error, which is why it is reported as a number in
/// the census rather than raised as a condition.
pub fn lanes_leaked() -> u32 {
    SPARE_OWNER
        .iter()
        .filter(|slot| slot.load(Ordering::Relaxed) != SPARE_FREE)
        .count() as u32
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    #[test]
    fn fresh_thread_starts_unclaimed() {
        // Reds if the TLS initialiser is ever changed to a value inside the topology, which would
        // silently attribute every non-pool thread to lane 0.
        let observed = thread::spawn(lane).join().expect("invariant: probe cannot panic");
        assert_eq!(observed, LANE_UNCLAIMED);
    }

    #[test]
    fn lane_is_per_thread() {
        set_lane(LANE_HOST);
        let child = thread::spawn(|| {
            let before = lane();
            set_lane(LANE_DISPATCHER);
            (before, lane())
        })
        .join()
        .expect("invariant: probe cannot panic");

        assert_eq!(child, (LANE_UNCLAIMED, LANE_DISPATCHER));
        // Reds if `LANE` is ever demoted from a thread-local to a shared static.
        assert_eq!(lane(), LANE_HOST);
        set_lane(LANE_UNCLAIMED);
    }

    #[test]
    fn release_ignores_lanes_it_does_not_own() {
        // A worker lane is below LANE_SPARE_BASE, so the subtraction inside `release_lane` would
        // underflow and index far out of bounds without the range guard: this reds by panicking.
        set_lane(7);
        release_lane();
        assert_eq!(lane(), 7);

        set_lane(LANE_UNCLAIMED);
        release_lane();
        assert_eq!(lane(), LANE_UNCLAIMED);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "invariant: a lane is a topology index")]
    fn set_lane_rejects_an_out_of_range_id() {
        set_lane(LANE_COUNT);
    }

    /// The only test that touches `SPARE_OWNER`. Spares are process-global and never reset, so a
    /// second claiming test in this binary would race this one for them.
    #[test]
    fn spare_claims_are_exclusive_and_recycle() {
        let gate = Arc::new(Barrier::new(SPARE_COUNT + 1));
        let (tx, rx) = mpsc::channel();

        let holders: Vec<_> = (0..SPARE_COUNT)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let tx = tx.clone();
                thread::spawn(move || {
                    let claimed = claim_lane();
                    assert_eq!(claimed, Some(lane()), "claim must label its own thread");
                    tx.send(claimed).expect("invariant: receiver outlives every holder");
                    gate.wait();
                    release_lane();
                    assert_eq!(lane(), LANE_UNCLAIMED);
                })
            })
            .collect();
        drop(tx);

        // Every holder has claimed and is parked on the gate before anything below runs.
        let mut ids: Vec<u16> = (0..SPARE_COUNT)
            .map(|_| {
                rx.recv()
                    .expect("invariant: every holder sends before it parks")
                    .expect("invariant: the first SPARE_COUNT claims all succeed")
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        // Reds if the compare-exchange is replaced by a load-then-store: two threads then claim
        // the same spare and the deduplicated count drops.
        assert_eq!(ids.len(), SPARE_COUNT, "spare ids must be pairwise distinct");
        assert_eq!(ids[0], LANE_SPARE_BASE);
        assert_eq!(ids[SPARE_COUNT - 1], LANE_COUNT - 1);
        assert_eq!(lanes_leaked(), SPARE_COUNT as u32);

        // Exhaustion is non-terminal: no panic, no block, and the caller keeps its own lane.
        assert_eq!(claim_lane(), None);
        assert_eq!(lane(), LANE_UNCLAIMED);

        gate.wait();
        for holder in holders {
            holder.join().expect("invariant: no holder panics");
        }

        // Reds if `release_lane` fails to publish the slot as free.
        assert_eq!(lanes_leaked(), 0);
        let recycled = claim_lane().expect("invariant: every spare was released");
        assert_eq!(lane(), recycled);
        release_lane();
        assert_eq!(lanes_leaked(), 0);
        assert_eq!(lane(), LANE_UNCLAIMED);
    }
}
