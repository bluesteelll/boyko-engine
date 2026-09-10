//! [`Scope`] — borrow-erased fork/join over the pool.
//!
//! Plan §4.5. `Scope::spawn` accepts a closure with lifetime `'scope`;
//! we transmute the lifetime to `'static` for the duration of the task
//! body. The `'scope` correctness is upheld by [`Scope::drop`]: it
//! blocks (via work-stealing, not parking) until every spawned task has
//! completed, so no body outlives the `'scope` borrow.
//!
//! ## Work-stealing wait (plan §4.5.5)
//!
//! `Scope::Drop` does NOT call `std::thread::park()` unconditionally.
//! Instead, it polls every source it can act through — a worker joiner its own
//! registered deque, then the global injector, then its siblings; an external
//! joiner the global injector and then its siblings — and runs what it finds.
//! Without this, nested scopes can deadlock when every worker is itself blocked
//! inside its own `Scope::Drop`.
//!
//! The join dispatches on the ONE identity predicate (`tls::worker_lane_for`,
//! `KE16-DESIGN-B.md` §2.1). A joining WORKER of this pool reaches its own deque
//! as the OWNER — the predicate hands it the lane `worker_main` deposited — so it
//! pops its own oldest chunk from the FIFO owner end, batch-steals into that same
//! REGISTERED deque (the residue it does not run stays stealable by every
//! sibling) and re-checks the scope between tasks; when it must park it parks
//! idle-marked (rule B1-P), which makes a parked joiner a claimable lane for any
//! other wave. An EXTERNAL joiner — the dispatcher, an unattached thread, a
//! worker of another pool, a worker inside an `install` frame of this pool — has
//! no lane here and steals one task at a time, because it has no registered sink
//! to receive a batch. Nested scopes cannot deadlock: a joiner either runs a
//! ready task or parks with a wake guaranteed by its own scope's last completer,
//! by a foreign wave's claim (B1-P), or by the backstop.

use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::any::Any;
use std::panic::resume_unwind;
use std::time::Duration;

use crossbeam_deque::Steal;
use crossbeam_utils::{Backoff, CachePadded};

use crate::block::ScopeBlock;
use crate::sync::{AtomicPtr, AtomicUsize, Ordering, Thread};
use crate::task::Task;
use crate::thread_pool::PoolInner;
// The join's dispatch is the ONE identity predicate (`KE16-DESIGN-B.md` §2.1),
// and `join_on_worker` acts through the lane it hands back.
use crate::tls;
use crate::worker::{push_task, unpark_one_idle};
// The worker joiner joins through the worker loop's OWN poll helpers rather than
// a second copy of the same scan; the two idle-bitset primitives are rule B1-P's
// park (`KE16-DESIGN-B.md` §2.2).
use crate::worker::{
    XorShift64Star, mark_idle, pop_any, pop_global_injector, run_task, splitmix64,
    try_steal_random, unmark_idle, unpark_one_idle_excluding,
};

/// The joiner's park backstop — a RE-POLL trigger, not a wait.
///
/// 50 us is the SOURCE constant; `park_timeout` rounds it up to the OS timer
/// quantum (measured ~15.3 ms unguarded and ~1.0 ms while `boyko_app`'s
/// `TimerResolutionGuard` holds 1 ms — KE16 App-7 / App-12), which is why every
/// route's real wake is a completer's `unpark` and this value only bounds the
/// rare lost-wakeup window. Named once so the worker route and the external
/// route cannot drift apart (`KE16-DESIGN-B.md` §2.2, §2.3).
///
/// It is NOT load-bearing on route (b), the worker joiner: the count gate makes
/// that joiner's check-then-park race-free against its own last completer, so
/// this timeout costs nothing when the wake arrives and is kept purely as a
/// defensive bound (`KE16-DESIGN-W.md` §3.4). On the external route, which keeps
/// the unpark-before-decrement order, it is still the only thing that ends a lost
/// wake — at a full timer quantum.
const JOIN_BACKSTOP: Duration = Duration::from_micros(50);

/// Miri-only: how many times the LAST completer yields while still inside
/// [`ScopeShared::complete_task`], immediately after the decrement that
/// authorises `Scope::drop` to free the allocation.
///
/// This exists so that
/// `tests/miri_scope_completion_protector.rs` can FAIL. The property it gates —
/// no thread holds a live Tree-Borrows protector over `ScopeShared` at the
/// instant of the free — has a window only a couple of MIR steps wide (the
/// decrement, then the frame's return) against a joiner tail of order a hundred
/// (loop exit, `ScopeBlock::free_all`, `take_panic`, `Box::from_raw`). Left to Miri's
/// random preemption the window is a coin flip on the seed, which is how a
/// 10 975-window probe reported nothing. Holding the completer's frame open for
/// a few scheduling rounds turns that into a decision, and it forces exactly the
/// interleaving a real OS produces whenever it deschedules a thread right after
/// the decrement (`lock decq` on this target) and before that thread's frame
/// returns.
///
/// # THIS VALUE IS A TUNING KNOB, NOT A GUARANTEE — read before changing it
///
/// A previous revision carried a compile-time `assert!(YIELDS >= 2)` and claimed
/// the inequality kept the gate armed. BOTH HALVES WERE WRONG, and the KE16
/// tester measured it:
///
/// * THE VALUE. The threshold is 4, not 2, on miri 2026-08-20 / nightly
///   1.100.0-gnu: yields of 2 and 3 pass such an assert and leave the gate GREEN
///   over the defective receiver, deterministically (5/5 and 3/3). The
///   "0 green, 1 green, 2 UB" this doc used to record does not reproduce.
/// * THE PREMISE, which is the bigger half. ARMED-NESS IS NOT MONOTONIC IN THIS
///   NUMBER. Two reconstructions of the completion body differing by ONE MIR
///   STATEMENT: reconstruction A is armed at 8, reconstruction B is DISARMED at
///   8 (0/5 UB, a full green census over the defective receiver) while armed at
///   4, 5, 6, 7, 10, 12, 16, 24 and 32. The mechanism is round-robin ALIGNMENT
///   between the completer's yields and the joiner's tail, not "hold the frame
///   open long enough". So `yields >= floor` implies armed-ness neither below
///   the floor nor above it, and NO constant threshold can certify it.
///
/// What certifies it instead is an OBSERVATION, not an inequality:
/// [`MIRI_FREES_INSIDE_A_RELEASE_WINDOW`] counts the frees that actually landed
/// while a completer was inside its post-decrement window, and the gate asserts
/// that count. A mis-tuned value — too small, too large, or merely misaligned —
/// makes that count fall short and the gate FAILS LOUDLY instead of printing a
/// green census over a defect. The shipped 16 is the value that observation is
/// green at today; if you change it, the gate tells you whether the new one
/// works, which is the whole point of replacing the inequality with a
/// measurement.
///
/// # THE SWEEP THAT WAS OWED, AND WHAT IT REFUTED (MEASURED 2026-09-10)
///
/// The paragraph above promises that a changed value makes the gate say whether
/// the new one works. Nobody had ever asked it across more than one value, and
/// in the meantime a six-seed run reported the gate ARMED on five seeds and
/// DISARMED on one (`overlaps=0/2`, W-d′ arm, seed 3). Read literally that says
/// armed-ness is a coin flip on the seed and this constant is mis-tuned. It is
/// neither. THE FLAPPING WAS THE RECIPE, NOT THE VALUE.
///
/// Six values x six seeds = 36 runs, each under this gate's own documented
/// recipe (`-Zmiri-tree-borrows -Zmiri-disable-isolation
/// -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0
/// -Zmiri-seed=N`), nightly-x86_64-pc-windows-gnu / miri 2026-08-20. Each cell
/// is the WORST `overlaps` over the six seeds for that context, as
/// `n/expected` — the margin, not the typical reading. In all 36 runs
/// `block_overlaps` EQUALLED `overlaps` and `slot_exhaustions` was 0:
///
/// | yields | body-environment | external-joiner | W-d′  | armed |
/// |--------|------------------|-----------------|------|-------|
/// | **16** | 3/4              | 3/4             | 1/2  | 6/6   |
/// | 24     | 3/4              | 3/4             | 1/2  | 6/6   |
/// | 32     | 4/4              | 4/4             | 1/2  | 6/6   |
/// | 48     | 3/4              | 3/4             | 1/2  | 6/6   |
/// | 64     | 3/4              | 2/4             | 1/2  | 6/6   |
/// | 96     | 3/4              | 3/4             | 2/2  | 6/6   |
///
/// EVERY value armed on EVERY seed, in all three contexts. There is no disarmed
/// point in this range to tune away from, so the SMALLEST is kept — which is the
/// value that was already here. Confirmed at 16 on five further seeds (0, 7, 8,
/// 9, 10), armed on all five: 11 distinct seeds, 0 disarmed.
///
/// Note what the table does NOT show, because it is why a sweep of this constant
/// cannot terminate in a verdict on its own: the columns do not improve
/// monotonically. 64 carries the WORST external-joiner margin in the set and 32
/// the best, with 48 between them. That is the round-robin ALIGNMENT effect this
/// doc has recorded since the floor assert was removed, and it is why "raise it
/// until it arms" is not a procedure.
///
/// # The discriminating experiment, which names the knob that WAS broken
///
/// SAME tree, SAME 16, SAME seeds 1..6; only the flag set varied:
///
/// | recipe                                    | armed |
/// |-------------------------------------------|-------|
/// | the documented one, above                 | 6/6   |
/// | `-Zmiri-tree-borrows -Zmiri-seed=N` only  | 4/6   |
///
/// Under the reduced one, seeds 2 and 4 go RED with `overlaps=0/2` on the W-d′
/// arm — the exact reading whose assert message sends a reader here to re-tune
/// this number. And seeds 3 and 6 print `block_overlaps` BELOW `overlaps` (1
/// against 2 on W-d′; 3 against 4 on body-environment), which is the inversion of
/// `block_overlaps >= overlaps` that `assert_probe_armed` documents as the
/// signature of a MISSING `-Zmiri-preemption-rate=0`. The monitor that exists
/// only to report that precondition reported it, on the first runs where the
/// precondition was actually violated.
///
/// Two reduced-recipe runs exist and they do NOT agree per seed: the first
/// (`miri_seeds_tb.log`, taken in `D:/wt/threadpool` on the uncommitted tree
/// that became `e6115223`) read seed 3 as the disarmed one, the second (this
/// tree, a different worktree path and target dir) reads seeds 2 and 4. Miri is
/// deterministic per seed only for an IDENTICAL binary; a different path string
/// or target dir is a different binary and shifts the RNG stream, so the two
/// runs are not seed-comparable — and that non-comparability is the point: a
/// receipt taken without `-Zmiri-preemption-rate=0` is a property of one build,
/// not of the constant.
///
/// The reduced recipe is markedly faster (the harness's own `finished in` read
/// 3-4 s against 6-26 s for the full one, on a LOADED box; a range, not a
/// benchmark), which is exactly what makes it tempting; it buys the speed by
/// not forcing the schedule. So
/// `-Zmiri-preemption-rate=0` is load-bearing for ARMED-NESS and not only for
/// the verdict, and a sweep of this constant taken without it measures the
/// preemption RNG rather than the yield count.
///
/// # The margin is thin, and printed rather than asserted for that reason
///
/// Seed 0 at 16 reads `overlaps=1/4` on the external-joiner arm — one scope
/// from red. That is precisely the drift from 3/4 toward 1/4 that
/// `assert_probe_armed` tells its reader to watch. It is a property of the SEED:
/// seed 0 is the thin one, and seeds 1..6 never went below 2/4 at any value.
/// Whether another value would widen SEED 0's margin was NOT measured — the
/// sweep above ran seeds 1..6 — so do not read the 32 row as a recommendation.
///
/// # Falsifiability, re-established with the value confirmed (2026-09-10)
///
/// A sweep that only ever produces green proves the constant is not the problem;
/// it does not prove the gate can still fail. Measured at 16 under the full
/// recipe, with `complete_task`'s receiver changed back to `&Self` (the pre-fix
/// shape) and its two `task/scoped.rs` call sites to `&*shared`: RED on seeds 1,
/// 2 and 3, every one of them
/// `error: Undefined Behavior: deallocation through <TAG> ... is forbidden`,
/// protected tag born at the mutated `complete_task` argument and accessed tag
/// at `Scope::drop`'s `Box::from_raw`. Zero `KE16-PROTECTOR-GATE-ARMED` lines
/// and zero arming asserts in all three — the gate reds on the PROPERTY, not on
/// its own armed-ness check.
///
/// Cost: `cfg(miri)` only, so the shipped artifact contains none of it (symbol
/// census identical to the pre-fix build, zero probe symbols); and even under
/// Miri it is taken only on `prev == 1`, i.e. once per `pending -> 0` transition
/// rather than once per task.
#[cfg(miri)]
const MIRI_RELEASE_PROBE_YIELDS: usize = 16;

/// Miri-only: mints one identity per [`ScopeShared`], so a release window is
/// matched to the SCOPE that opened it rather than to a machine address.
///
/// Starts at 1 because 0 is [`MIRI_WINDOW_SLOT_EMPTY`], the free-slot sentinel
/// of [`MIRI_OPEN_RELEASE_WINDOWS`]: a key must never be mistakable for a free
/// slot, by the claim CAS or by the free site's scan.
///
/// # Why an epoch, and not the address this replaced
///
/// The slot array's stated invariant was address UNIQUENESS — "a slot holds an
/// address only its own scope's completers ever write". That is an ASSUMPTION
/// about the allocator, not a fact about the data. Every `Box<ScopeShared>` is
/// allocated and freed on the SAME THREAD (`Scope::new`'s `Box::into_raw` ->
/// `Scope::drop`'s `Box::from_raw`), Miri's default same-thread heap
/// address-reuse rate is 0.5, and this gate's published recipe pins no reuse
/// rate. So scope k's stale slot can hold an address that scope k+1's
/// allocation is subsequently handed, and k+1's free then certifies an overlap
/// that never happened — an ADDITIVE error, which is the FALSE-GREEN direction
/// for a gate whose threshold is `overlaps >= 1`. A monotonic counter is unique
/// by construction and needs no allocator behaviour to be true.
///
/// `Relaxed` is the whole ordering requirement: this counter's only job is to
/// hand out distinct values, and every value that reaches a slot is published
/// there by a `SeqCst` CAS.
#[cfg(miri)]
static MIRI_SCOPE_EPOCH: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(1);

/// Miri-only: the free-slot sentinel of [`MIRI_OPEN_RELEASE_WINDOWS`], and the
/// one value [`MIRI_SCOPE_EPOCH`] never mints.
#[cfg(miri)]
const MIRI_WINDOW_SLOT_EMPTY: usize = 0;

/// Miri-only: slots recording which SCOPES currently have a completer inside
/// their post-decrement release window.
///
/// One `AtomicUsize` per concurrently-open window, holding the
/// [`MIRI_SCOPE_EPOCH`] key of the scope whose completer is inside the burst, or
/// [`MIRI_WINDOW_SLOT_EMPTY`] for a free slot. Eight is far more than the two or
/// three a gate run ever opens at once; a completer that finds none does not
/// record its window, which can only LOSE an observation and never invent one —
/// but a lost observation resurfaces as `overlaps = 0` under an assert that
/// blames the yield count, so the shortage is counted separately in
/// [`MIRI_WINDOW_SLOT_EXHAUSTIONS`] and asserted by the gate instead of being
/// left to be misdiagnosed.
///
/// Directionality is the point: a slot holds a key only its own scope's
/// completers ever write, so a free that finds its own key there has certainly
/// caught one of its own completers mid-window. A clobbered slot yields a MISS,
/// which makes the gate red, never green.
///
/// EVERY MUTATION IS A `compare_exchange` WHOSE EXPECTED VALUE IS THE MUTATOR'S
/// OWN KEY, so a slot can only ever be cleared by a party holding that key. ⚠
/// The shipped tree does NOT have a bug here, and this is not the repair of one:
/// with no closer, slot `i` has a unique writer between its claim and its clear,
/// so the unconditional `store(0)` the release CAS replaced is safe today. The
/// CAS is what makes ADDING a closer later — a `Scope::drop` that clears its own
/// scope's slots after the last reclamation — a two-line change rather than a
/// correctness re-argument.
#[cfg(miri)]
static MIRI_OPEN_RELEASE_WINDOWS: [core::sync::atomic::AtomicUsize; 8] =
    [const { core::sync::atomic::AtomicUsize::new(MIRI_WINDOW_SLOT_EMPTY) }; 8];

/// Miri-only: how many completers found NO free slot in
/// [`MIRI_OPEN_RELEASE_WINDOWS`] and so could not record their window.
///
/// A DIAGNOSIS, not a property — and it exists because the gate had no way to
/// state this one. `MIRI_OPEN_RELEASE_WINDOWS`'s own doc is right that a failed
/// claim "can only LOSE an observation and never invent one", so soundness is
/// never at stake. What it could not do is say WHY the gate went red: the lost
/// observation reappears as `overlaps = 0`, under an assert whose message tells
/// the reader to re-tune [`MIRI_RELEASE_PROBE_YIELDS`] against an observation
/// that was never taken at all. A RED WITH THE WRONG DIAGNOSIS is the failure
/// mode this repository keeps cataloguing, so a slot shortage is made to say
/// that it is a slot shortage: the gate prints this delta as
/// `slot_exhaustions=` and asserts it is zero.
#[cfg(miri)]
pub(crate) static MIRI_WINDOW_SLOT_EXHAUSTIONS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Miri-only: how many `Scope::drop` frees landed while a completer of THAT
/// SAME allocation was inside its post-decrement release window.
///
/// THE GATE'S ARMED-NESS OBSERVATION, and the only one of the three locks that
/// checks the property rather than a proxy for it. The other two guard inputs:
/// the firing counter proves the probe was CALLED, and the (now removed) floor
/// assert guarded the CONSTANT. Neither can see the thing that actually decides
/// the gate — whether the free and the completer's open frame overlapped in
/// time. The KE16 tester defeated both at once by keeping the `fetch_add` and
/// deleting only the yield loop: both locks green, gate disarmed, defect
/// shipped. This counter is what that mutation cannot pass, because with no
/// burst the window closes before the joiner can reach the free and the overlap
/// is never observed.
///
/// It is also exactly the condition under which a reference-typed receiver would
/// be UB — the free happening while the completer's frame is still open — so
/// asserting it is asserting that the gate exercised the interleaving it claims
/// to decide.
#[cfg(miri)]
pub(crate) static MIRI_FREES_INSIDE_A_RELEASE_WINDOW: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Miri-only: how many `Scope::drop` CHUNK frees landed while a completer of
/// that same scope was inside its post-decrement release window.
///
/// The KE16 M2w observation, and the reason it is a SECOND counter rather than
/// a second increment of [`MIRI_FREES_INSIDE_A_RELEASE_WINDOW`]: THE TWO COUNT
/// DIFFERENT EVENTS, over different allocations, at different points in
/// `Scope::drop`.
///
/// * M1's counter observes the `Box::from_raw` of the `ScopeShared` — the
///   allocation a completer's own `*const Self` names.
/// * this one observes `ScopeBlock::free_all`, which hands back the CHUNKS the
///   scope's task cells live in. Those are allocations no completer holds a
///   pointer to at all: what a completer names is its own cell, and the chunk
///   it sits in is reclaimed wholesale.
///
/// Keeping them apart is what stops this column from inheriting the other's
/// verdict. A shared counter would read `>= 1` from either free, so an M2w
/// green could be produced entirely by M1's event — the "green from
/// elsewhere" failure this repository keeps cataloguing — and the two are not
/// interchangeable evidence: they are freed by different calls, and only this
/// one is the free that Stage 3b introduced.
///
/// Incremented ONCE PER `Scope::drop`, not once per chunk, and only when the
/// block actually grew one. Per-chunk counting would make the number a
/// function of the payload bytes a scope happened to spawn rather than of the
/// event being observed, and the threshold the gate asserts (`>= 1`, as M1's
/// is) would then be met by a single large scope while a hundred small ones
/// went unobserved.
#[cfg(miri)]
pub(crate) static MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Miri-only: how many times [`miri_release_probe`] has run — one per
/// `pending -> 0` transition.
///
/// The weaker of the two counters: it proves the probe was CALLED, which catches
/// a probe that was deleted, `cfg`'d away or moved off the `prev == 1` path. It
/// does NOT prove the probe still opens a window; see
/// [`MIRI_FREES_INSIDE_A_RELEASE_WINDOW`] for that. Both are asserted, because
/// they fail differently and their messages point at different mistakes.
///
/// `core`'s atomic, not [`crate::sync`]'s: this is `cfg(miri)`-only and
/// `cfg(miri)` never co-occurs with `cfg(loom)`, whose shimmed atomics cannot
/// initialise a `static`.
#[cfg(miri)]
pub(crate) static MIRI_RELEASE_PROBE_FIRINGS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Miri-only: the release probe itself — one definition, called from both routes
/// of [`ScopeShared::complete_task`], so the two cannot drift apart.
///
/// Must be called ONLY on the `prev == 1` path and ONLY after the release RMW:
/// its whole purpose is to hold the caller's frame open across the window in
/// which the joiner may free the allocation, and to make that window OBSERVABLE
/// to the free site so the gate can assert it happened.
///
/// `key` is the [`MIRI_SCOPE_EPOCH`] identity of the scope this completer was
/// handed, read out of the allocation BEFORE the decrement by both callers —
/// after the decrement the joiner may already have freed it, so the field is no
/// longer there to read. It is a plain integer, and deliberately so: it is only
/// ever compared for equality with the free site's own key, never dereferenced,
/// so the allocation may be freed under it, which is precisely the situation
/// being recorded.
///
/// `SeqCst` throughout: this is instrumentation whose whole value is that the
/// free site sees the freshest state, and its cost is irrelevant because none of
/// it exists outside `cfg(miri)`.
#[cfg(miri)]
#[inline(never)]
fn miri_release_probe(key: usize) {
    use core::sync::atomic::Ordering::SeqCst;

    MIRI_RELEASE_PROBE_FIRINGS.fetch_add(1, SeqCst);

    // CLAIM. The CAS, not the scan, is what makes the claim exclusive: the
    // winner of `EMPTY -> key` at index `i` is the only party that wrote `key`
    // there, which is what entitles it — and nobody else — to clear `i` below.
    let mut claimed: Option<usize> = None;
    for (i, slot) in MIRI_OPEN_RELEASE_WINDOWS.iter().enumerate() {
        if slot
            .compare_exchange(MIRI_WINDOW_SLOT_EMPTY, key, SeqCst, SeqCst)
            .is_ok()
        {
            claimed = Some(i);
            break;
        }
    }
    if claimed.is_none() {
        MIRI_WINDOW_SLOT_EXHAUSTIONS.fetch_add(1, SeqCst);
    }

    for _ in 0..MIRI_RELEASE_PROBE_YIELDS {
        std::thread::yield_now();
    }

    // RELEASE-OWN. `key -> EMPTY` at the REMEMBERED index, so this clear can
    // never erase a window that is not this one's. A failed CAS means the slot
    // no longer holds this key and there is nothing of ours to clear: write
    // nothing. Today the CAS cannot fail — nobody else can write `i` between the
    // claim and here — and that is exactly why this is a substrate change and
    // not a bug fix; see `MIRI_OPEN_RELEASE_WINDOWS`.
    if let Some(i) = claimed {
        let _ = MIRI_OPEN_RELEASE_WINDOWS[i].compare_exchange(
            key,
            MIRI_WINDOW_SLOT_EMPTY,
            SeqCst,
            SeqCst,
        );
    }
}

/// Miri-only: called by `Scope::drop` immediately before the free, to record
/// whether this deallocation landed inside one of its own completers' release
/// windows.
///
/// See [`MIRI_FREES_INSIDE_A_RELEASE_WINDOW`] for why this, and not the probe's
/// firing count, is what certifies the gate.
///
/// OBSERVE, in the slot protocol's terms: `key` is the freeing scope's own
/// [`ScopeShared::miri_key`], so a match means a completer OF THIS SCOPE is
/// mid-window. It was the allocation's ADDRESS until Stage 3y, which made the
/// match depend on the allocator not recycling an address across two scopes —
/// see [`MIRI_SCOPE_EPOCH`].
#[cfg(miri)]
#[inline(never)]
fn miri_note_free_against_open_windows(key: usize) {
    use core::sync::atomic::Ordering::SeqCst;

    if MIRI_OPEN_RELEASE_WINDOWS
        .iter()
        .any(|slot| slot.load(SeqCst) == key)
    {
        MIRI_FREES_INSIDE_A_RELEASE_WINDOW.fetch_add(1, SeqCst);
    }
}

/// Miri-only: the sibling of [`miri_note_free_against_open_windows`] for the
/// BLOCK free, called by `Scope::drop` immediately before `free_all`.
///
/// Same slot protocol and same `key` — the freeing scope's own
/// [`ScopeShared::miri_key`], so a match means a completer OF THIS SCOPE is
/// mid-window — and a deliberately separate counter, because the event is a
/// different free of a different allocation class. See
/// [`MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW`] for why the two must not share
/// one.
///
/// A separate function rather than a parameter on the existing one: the two
/// notes are called from adjacent lines of the same `Drop`, and a boolean
/// discriminant would put the choice of counter at the call site, where a
/// copy-paste can silently point the block's free at M1's column and produce a
/// green in the column that was not exercised.
///
/// `#[inline(never)]` for the reason its sibling carries it: the gate's failure
/// diagnoses are read off symbol names.
#[cfg(miri)]
#[inline(never)]
fn miri_note_block_free_against_open_windows(key: usize) {
    use core::sync::atomic::Ordering::SeqCst;

    if MIRI_OPEN_RELEASE_WINDOWS
        .iter()
        .any(|slot| slot.load(SeqCst) == key)
    {
        MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW.fetch_add(1, SeqCst);
    }
}

/// Shared state between [`Scope`] and its spawned tasks.
///
/// Allocated on the heap so that every spawned task's payload cell can hold a
/// raw pointer to it that remains stable across `Scope` moves (the
/// `Scope` itself is small and `!Unpin` via the `Box<ScopeShared>`
/// indirection).
///
/// `#[repr(C)]` fixes the field ORDER, and that order is load-bearing twice
/// over. For the D-cache: `pending` is the contended field and leads the
/// struct, `CachePadded` so that completion traffic does not false-share with
/// the three cold fields behind it. For SOUNDNESS REVIEW: it makes the
/// allocation's frozen (non-`UnsafeCell`) ranges predictable, which is what a
/// Tree-Borrows protector violation is reported against — the completion-gate
/// UB report names `alloc…[0x8]`, i.e. inside `CachePadded`'s padding rather
/// than inside the atomic, and it is `repr(C)` that makes that offset mean the
/// same thing on every build. A `repr(Rust)` reordering would not be unsound,
/// but it would make the report's offset unstable and the frozen/interior-mutable
/// split a matter of compiler whim. See `ScopeShared::complete_task` for why
/// that split decides whether a protector is strong or weak.
#[repr(C)]
pub(crate) struct ScopeShared {
    /// Count of outstanding spawned tasks. Increments on spawn, decrements
    /// on task completion (success or panic). `Scope::Drop` waits until
    /// this reaches zero.
    ///
    /// `CachePadded` because workers all decrement this on completion;
    /// the dispatcher's `Drop` thread reads it. Without padding, the
    /// completion traffic would false-share with neighbouring atomics.
    pub(crate) pending: CachePadded<AtomicUsize>,

    /// First panic payload observed by a task; consumed by `Scope::Drop` and re-raised via
    /// `resume_unwind`. Null means no task panicked.
    ///
    /// A `Box<dyn Any + Send>` is a FAT pointer and cannot live in an atomic, so the payload is
    /// boxed once more: the cell holds `*mut Box<dyn Any + Send>`, a thin pointer.
    ///
    /// 2026-07 audit: this was a `Mutex<Option<Box<dyn Any + Send>>>` documented as "cold-path
    /// only (panics are rare)". The WRITE is indeed panic-only, but `Scope::drop` read the slot
    /// through an unconditional `lock()` on every scope teardown and `ScopeShared::new` built a
    /// fresh `Mutex` per scope — and the parallel scheduler creates a scope per system run. The
    /// CAS-once slot keeps the same protocol (first panic wins, later payloads are dropped) with
    /// a null load on the path that does not panic.
    pub(crate) panic_payload: AtomicPtr<Box<dyn Any + Send + 'static>>,

    /// Thread to unpark when `pending` reaches zero. Captured at
    /// `Scope::new` time (typically `std::thread::current()` inside the
    /// surrounding `install`/`scope` frame).
    pub(crate) waker: Thread,

    /// KE16 W-d′ — the count-gated wake target, or null.
    ///
    /// Non-null iff the thread that opened this scope was a registered worker of
    /// this scope's pool ACTING AS that worker: `tls::worker_lane_for(inner)`
    /// was `Some` at scope creation, the ONE identity predicate
    /// (`KE16-DESIGN-A.md` §1.1). It then points at `inner.workers[wid].thread`,
    /// which `PoolInner` owns and which is alive independently of this
    /// allocation — so the last completer may unpark it AFTER the decrement that
    /// lets the joiner free `*self`.
    ///
    /// Null for the dispatcher, an unattached thread, a worker of ANOTHER pool,
    /// and a worker inside an `install` frame of this pool (whose id is the
    /// dispatcher sentinel for that frame): for those [`waker`](Self::waker) is
    /// the only target, it lives in THIS allocation, and the unpark must
    /// therefore precede the decrement, as it does today
    /// (`KE16-DESIGN-W.md` §3.5).
    ///
    /// Written once in [`ScopeShared::new`] and never again; read only by
    /// [`complete_task`](Self::complete_task), which dispatches on it.
    pub(crate) joiner_wake: *const crate::sync::WakeHandle,

    /// Miri-only: this scope's identity in the completion gate's window slots —
    /// a [`MIRI_SCOPE_EPOCH`] draw, minted once in [`ScopeShared::new`] and
    /// never written again.
    ///
    /// APPENDED LAST, AND THAT PLACEMENT IS LOAD-BEARING. `#[repr(C)]` fixes the
    /// field order, and this struct's doc records why: the completion-gate's UB
    /// report names `alloc…[0x8]`, an offset inside `CachePadded`'s padding, and
    /// `repr(C)` is what makes that offset mean the same thing on every build.
    /// Appending leaves every existing field at the offset it already had, so
    /// every recorded report stays readable against this struct. Prepending
    /// would invalidate them all.
    ///
    /// It is read by [`complete_task`](Self::complete_task) BEFORE the
    /// decrement, as a place expression through `*const Self` — a `Copy` read
    /// that forms no reference and so installs no protector over the very
    /// allocation the gate is judging.
    #[cfg(miri)]
    pub(crate) miri_key: usize,
}

impl ScopeShared {
    /// Initialises the panic-payload slot to "no panic" — a null store, no allocation.
    ///
    /// `joiner_wake` is the KE16 W-d′ target: `PoolInner::joiner_wake_target`'s
    /// answer for the joining thread, null for every external joiner.
    /// (`KE16-DESIGN-W.md` §3.2 calls that function `worker_wake_handle`; it was
    /// renamed with its signature — it takes no `lane`, it asks the predicate
    /// itself, so the W switch is ONE site.)
    #[inline]
    pub(crate) fn new(waker: Thread, joiner_wake: *const crate::sync::WakeHandle) -> Self {
        // Miri-only: mint this scope's window-slot identity. `Relaxed` because
        // the counter owes nothing but distinctness — the value is published to
        // other threads by the `SeqCst` claim CAS that writes it into a slot.
        #[cfg(miri)]
        let miri_key = {
            let key = MIRI_SCOPE_EPOCH.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            // The only guard the key needs: 0 is the slot array's EMPTY
            // sentinel, and a wrapped counter would hand out a key that reads as
            // a free slot. Reaching it takes 2^64 scopes in one process.
            debug_assert!(
                key != MIRI_WINDOW_SLOT_EMPTY,
                "MIRI_SCOPE_EPOCH wrapped onto the EMPTY sentinel; window keys are no longer \
                 distinguishable from free slots"
            );
            key
        };

        Self {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: AtomicPtr::new(ptr::null_mut()),
            waker,
            joiner_wake,
            #[cfg(miri)]
            miri_key,
        }
    }

    /// Publishes `payload` as THE panic of this scope, keeping the first one seen.
    ///
    /// Called only from the `Err` arm of a task body's `catch_unwind`. A racing second panic
    /// loses the CAS and drops its own payload, matching the previous `Mutex` protocol
    /// ("first wins; subsequent payloads dropped") without a lock.
    #[cold]
    #[inline(never)]
    pub(crate) fn capture_panic(&self, payload: Box<dyn Any + Send + 'static>) {
        let raw = Box::into_raw(Box::new(payload));
        if self
            .panic_payload
            .compare_exchange(ptr::null_mut(), raw, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // SAFETY: `raw` was minted from `Box::into_raw` on the line above and the failed CAS
            //   means it was never published, so this thread is still its unique owner and no
            //   other thread can observe it. Reclaiming it here is the "later panics are
            //   dropped" half of the protocol.
            drop(unsafe { Box::from_raw(raw) });
        }
    }

    /// Takes the captured panic payload, leaving the slot empty.
    ///
    /// `Acquire` pairs with [`capture_panic`](Self::capture_panic)'s release half, so the
    /// payload's contents are visible to the taker.
    #[inline]
    pub(crate) fn take_panic(&self) -> Option<Box<dyn Any + Send + 'static>> {
        let raw = self.panic_payload.swap(ptr::null_mut(), Ordering::Acquire);
        if raw.is_null() {
            return None;
        }
        // SAFETY: a non-null pointer here was published by exactly one `capture_panic` CAS, and
        //   the `swap` that produced it atomically removed it from the cell — so this thread is
        //   now its unique owner. The pointer came from `Box::into_raw(Box::new(..))`, matching
        //   this `Box::from_raw`.
        Some(*unsafe { Box::from_raw(raw) })
    }

    /// Register one outstanding task. Called by [`Scope::spawn`] before the
    /// task body is enqueued.
    ///
    /// `AcqRel` so that the increment is ordered against the matching
    /// `complete_task` decrements on worker threads (the join wait reads the
    /// resulting count with `Acquire` in [`Self::is_drained`]).
    #[inline]
    pub(crate) fn register_task(&self) {
        self.register_tasks(1);
    }

    /// Register `n` outstanding tasks in ONE RMW. Its one caller is
    /// [`register_task`](Self::register_task), which registers a single task
    /// immediately before that task's push.
    ///
    /// `AcqRel` for the reason stated there: the count this increment publishes
    /// is the same count the `complete_task` decrements consume, and the join
    /// wait reads the result with `Acquire` in [`Self::is_drained`].
    #[inline]
    pub(crate) fn register_tasks(&self, n: usize) {
        self.pending.fetch_add(n, Ordering::AcqRel);
    }

    /// Mark one spawned task complete. Its one production caller is
    /// `task::run_scoped`, which calls it as its LAST statement — after the
    /// `catch_unwind` activation that received the body has returned, and after
    /// any panic payload has been stored. (The other caller is the `cfg(loom)`
    /// shim in `lib.rs`, which drives this same body from the models.)
    ///
    /// # The receiver is a RAW POINTER, and that IS the soundness fix
    ///
    /// This is an associated function over `*const Self`, not a method over
    /// `&self`, for exactly the reason
    /// [`join_workers_until_drained`] takes its `shared` by value — Phase 9.2
    /// Candidate U's joiner-side half of this same fix. A reference-typed
    /// function ARGUMENT carries a Tree-Borrows *protector* that lives for the
    /// whole call, i.e. to the closing brace, and TB forbids deallocating any
    /// range a live STRONG protector covers — regardless of happens-before, and
    /// regardless of whether a single byte is touched. The decrement below is
    /// precisely the event that authorises the joiner to free this allocation
    /// (`Scope::drop`, the single free site, which frees the instant its
    /// `Acquire` load sees zero). With a `&self` receiver the freeing thread
    /// therefore raced a protector that had not yet expired. Passing the
    /// pointer BY VALUE means no frame on this thread holds a reference into
    /// the allocation at the instant of the release.
    ///
    /// WHAT WAS BELIEVED, kept because the correction is the point: the earlier
    /// text argued that the `fetch_sub` is this thread's LAST BYTE-ACCESS to
    /// the allocation, so the joiner may free the moment it observes zero. That
    /// claim is true, it is still true, and it answers the DATA-RACE rule. The
    /// rule that judges this code is the protector rule, and under it `&self`
    /// is UB-relevant for as long as the CALL lives, not for as long as its
    /// accesses live. The two coincide for almost every function in this crate;
    /// they come apart for exactly the ones whose last act publishes permission
    /// to free their own argument.
    ///
    /// AND IT IS NOT ONLY A MODEL QUESTION. A `&ScopeShared` parameter carries
    /// LLVM `dereferenceable(256)` (the type's size — `crossbeam`'s
    /// `CachePadded` is 128-byte aligned on x86_64, so `ScopeShared` rounds to
    /// 256), which licenses the optimiser to hoist or speculate loads ANYWHERE
    /// in the function, including after the decrement, from memory the joiner is
    /// by then entitled to have freed. That never materialised — but only
    /// because `complete_task` is `#[inline]` and was in fact always inlined
    /// into its caller of the day (the `Box`ed task-body wrapper, since
    /// deleted — see the `task` module), where the attribute does not survive.
    /// The old code was therefore safe by an inlining DECISION rather than by any
    /// guarantee, and a future `#[inline(never)]`, a codegen-unit change or a
    /// PGO run that declined to inline it would have been enough. A raw pointer
    /// parameter carries no such licence.
    ///
    /// MEASURED, because it explains why this hid: a payload that is ENTIRELY
    /// interior-mutable gives `&self` only a WEAK protector, which does not
    /// forbid deallocation at all. `ScopeShared` is not such a payload — it
    /// carries frozen (non-`UnsafeCell`) bytes in `waker: Thread`, in
    /// `joiner_wake`, and in `CachePadded`'s padding — so the free is a foreign
    /// write to a strongly protected frozen range ("protected tags must never
    /// be Disabled"), which is deterministic UB rather than a near miss.
    ///
    /// # What now establishes the invariant
    ///
    /// At the instant either route's release RMW commits, the only protector
    /// covering this allocation is the one `AtomicUsize::fetch_sub` installs on
    /// its own `&self`: a range wholly inside an `UnsafeCell`, hence weak.
    /// Every strongly protected borrow this function takes —
    /// `&(*shared).waker` for the `unpark`, and the `&CachePadded<_>` that
    /// `Deref::deref` consumes on the way to `pending` — has already returned
    /// its frame, and every one of them ran while `pending >= 1` (this task's
    /// own registration was still outstanding), so the joiner could not have
    /// observed zero and could not have freed. Nothing in this function, and
    /// nothing in its caller, holds a reference into the allocation across the
    /// RMW.
    ///
    /// That is a CHECKED claim, not an argued one:
    /// `tests/miri_scope_completion_protector.rs` forces the interleaving and is
    /// red on a reference receiver, green on this one, on every seed tried. The
    /// `#[cfg(miri)]` yield bursts below are what give it the power to fail —
    /// see `MIRI_RELEASE_PROBE_YIELDS` (a `cfg(miri)` item, hence not linked).
    /// Removing either the by-value receiver
    /// or a burst is a soundness change, not a cleanup.
    ///
    /// # Order (unchanged by the fix)
    ///
    /// WHICH ROUTE RUNS IS DECIDED BY THE TARGET POINTER, not by any build
    /// configuration: a non-null [`joiner_wake`](Self::joiner_wake) means this
    /// scope's joiner is a registered worker of this pool, and the count-gated
    /// route runs (decrement FIRST, unpark only on `prev == 1`); a null one means
    /// an external joiner, and the route below runs (unpark FIRST, then
    /// decrement).
    ///
    /// ORDER IS LOAD-BEARING (Phase 9.2 Candidate U) on the route whose wake
    /// target lives in the allocation: `waker.unpark()` happens BEFORE
    /// `pending.fetch_sub`. While this task has not yet decremented,
    /// `pending >= 1`, so `Scope::drop`'s join cannot have observed zero and
    /// therefore cannot have freed the allocation — the `waker` read is sound.
    /// Multi-drain-safe: the free is tied to scope END, never to an
    /// intermediate wave's `pending -> 0` (the ECS executor drains `pending` to
    /// zero once per dispatch wave).
    ///
    /// The count-gated route below (KE16 W-d′) satisfies the same rule by a
    /// different means: its target is `PoolInner`-owned, so it is read out of
    /// `*shared` BEFORE the decrement and used AFTER it.
    ///
    /// The unpark is UNCONDITIONAL on the external route (no `prev == 1` gate).
    /// NOT because being last is unknowable — `fetch_sub` returns the previous
    /// value, so `prev == 1` costs nothing to learn (the earlier claim that it
    /// "would require reading `pending` after the sub" was simply wrong, KE16
    /// N66). The binding constraint is the WAKE TARGET's lifetime: `waker`
    /// lives inside the allocation this decrement releases, so a gated wake has
    /// to read it after the sub, when the joiner may already have freed it.
    /// Gating it needs a target owned elsewhere — which is exactly what
    /// [`joiner_wake`](Self::joiner_wake) is for a WORKER joiner (W-d′,
    /// `KE16-DESIGN-W.md` §3) and what no external joiner has (§3.5, recorded
    /// for KE17). The unconditional
    /// pre-decrement unpark is a cheap token store when the joiner runs; a
    /// spurious wake when it is parked is harmless (it re-checks `pending`). Its
    /// rare lost-wakeup window — the joiner consumes the token, re-checks a
    /// `pending` the completer has not yet decremented, parks again, and the
    /// decrement wakes nobody — is covered only by the `park_timeout` backstop,
    /// i.e. by a full OS timer quantum. W-d′ closes it on route (b) by making
    /// the decrement FIRST: a check that sees `pending >= 1` precedes the last
    /// decrement in the RMW total order, and that decrement's unpark either
    /// finds the joiner parked or leaves a token its next park consumes.
    ///
    /// `AcqRel` on the decrement is unchanged (loom-proven M1); the `unpark` is
    /// `std`'s `swap(NOTIFIED, Release)`, which pairs with the joiner's
    /// `park_timeout` `Acquire` on its own parker.
    ///
    /// # Safety
    ///
    /// `shared` must point at a live `ScopeShared` whose `pending` STILL COUNTS
    /// this task — i.e. the caller has not completed it already, and never
    /// completes it twice. That is what keeps the allocation alive for the
    /// duration of the call: the single free site (`Scope::drop`) frees only
    /// after its join observes `pending == 0`, which cannot happen while this
    /// task's own registration is outstanding. The caller must perform no
    /// access to `*shared` after this call returns, and must hold no reference
    /// into the allocation across it.
    #[inline]
    pub(crate) unsafe fn complete_task(shared: *const Self) {
        // The count-gated route (KE16 W-d′), taken when this scope's joiner is a
        // registered worker of this pool: its wake target is owned by
        // `PoolInner` rather than by this allocation, so the decrement may come
        // first. Scoped in a block so `target` cannot outlive the route that
        // dispatched on it.
        {
            // Copied out BEFORE the decrement: after it the joiner may free this
            // allocation, so `*shared` must not be touched again. A null target
            // is an external joiner and falls through to the
            // unpark-before-decrement order below.
            //
            // SAFETY: `*shared` is live per this function's contract — this
            //   task is still counted in `pending`, and the only free site runs
            //   after a join that observed zero. A place-read through a raw
            //   pointer forms NO reference and therefore installs no protector.
            let target = unsafe { (*shared).joiner_wake };
            if !target.is_null() {
                // Miri-only, and copied out here for the SAME reason as `target`
                // above: the probe's key must be read while the allocation is
                // certainly live, i.e. before the decrement that authorises the
                // joiner to free it. It is `Copy` and read as a place expression
                // through the raw pointer, so it forms NO reference and installs
                // no protector — the property this whole function exists to
                // keep.
                //
                // SAFETY: `*shared` is live per this function's contract — this
                //   task is still counted in `pending`, so no join can have
                //   observed zero and the single free site cannot have run.
                #[cfg(miri)]
                let miri_key = unsafe { (*shared).miri_key };

                // SAFETY: live for the reason above. The only protector alive
                //   when this RMW commits is `AtomicUsize::fetch_sub`'s own
                //   `&self`, whose range is entirely `UnsafeCell` and therefore
                //   WEAK — it does not forbid the deallocation this very RMW
                //   authorises. The strong protector over `CachePadded`'s frozen
                //   padding belongs to `Deref::deref` and expired when that
                //   frame returned, while `pending` was still >= 1.
                let prev = unsafe { (*shared).pending.fetch_sub(1, Ordering::AcqRel) };

                if prev == 1 {
                    // SAFETY: `target` points at `inner.workers[wid].thread`,
                    //   owned by the `PoolInner` of the pool this task belongs
                    //   to, and that `PoolInner` is alive: the thread running
                    //   this completion is either a worker of that pool (holding
                    //   its own `Arc<PoolInner>` for its whole life,
                    //   `worker::worker_main`) or a joiner running this task
                    //   inline inside an `install` / `scope` frame of that pool,
                    //   whose `&'scope PoolInner` outlives the frame. No other
                    //   thread ever executes a task of the pool — tasks live
                    //   only in its queues. The pointee is a `Thread` handle (an
                    //   `Arc` inside `std`), valid even if that worker thread has
                    //   since exited. It was written once in `ScopeShared::new`
                    //   and is non-null only because `worker_lane_for` answered
                    //   `Some` there, so `wid` indexes a worker of THIS pool.
                    //   The allocation this function was handed may already be
                    //   freed at this point; nothing below touches it, and the
                    //   protector this call installs covers `*target`, which is
                    //   a different allocation with a different owner.
                    unsafe { (*target).unpark() };

                    // THE RELEASE POINT on this route, placed AFTER the unpark — and the
                    // reason is now stated at the strength it was actually
                    // established, which is weaker than the first draft claimed.
                    //
                    // The draft said the gate is "RED here and GREEN with the
                    // burst moved above the unpark, because this route's joiner
                    // may be parked and a burst before the wake would hold the
                    // frame open while the only thread that could free was still
                    // blocked". MEASURED 2026-09-05 during KE16, on a build
                    // carrying this route, at `-Zmiri-preemption-rate=0`, with a
                    // deliberately reference-typed receiver:
                    // `worker_joiner_completer_holds_no_protector_when_the_joiner_frees`
                    // is RED at BOTH placements. The disarming the draft
                    // predicted did not reproduce — this route's joiner is polling
                    // its nested join, not parked, so the burst reaches it
                    // either way. The gate does NOT decide the placement.
                    //
                    // After the unpark is kept because it DOMINATES rather than
                    // because it was shown to differ: it makes the probe's
                    // effect independent of the joiner's park state, whereas
                    // before the unpark the burst delays the wake and would be
                    // disarmed by any future shape whose joiner really is
                    // parked. Both are red today; only one stays red for a
                    // reason that does not depend on scheduling luck.
                    // `cfg(miri)` only.
                    #[cfg(miri)]
                    miri_release_probe(miri_key);
                }
                return;
            }
        }
        // External joiner (a null target): unpark BEFORE the decrement, because
        // `waker` lives in this allocation.
        //
        // SAFETY: `*shared` is live per this function's contract: this task has
        //   not decremented yet, so `pending >= 1` and no join can have observed
        //   zero. The `&Thread` protector `unpark` installs covers frozen bytes
        //   of THIS allocation and is STRONG — which is sound precisely because
        //   it expires before the decrement below, not after it.
        unsafe { (*shared).waker.unpark() };
        // Miri-only: the probe's key, read while the allocation is certainly
        // live. It must precede the decrement below, which is what lets the
        // joiner free — the W-d′ route reads it in the same position, next to
        // its own pre-decrement copy-out. A `Copy` place expression through the raw
        // pointer, forming no reference and installing no protector.
        //
        // SAFETY: `*shared` is live per this function's contract: this task has
        //   not decremented yet, so `pending >= 1` and no join can have observed
        //   zero.
        #[cfg(miri)]
        let miri_key = unsafe { (*shared).miri_key };
        // SAFETY: live until this RMW commits, for the reason above. When it
        //   commits, the only protector over this allocation is `fetch_sub`'s
        //   own all-`UnsafeCell` (weak) `&self`; this function holds `shared` by
        //   value and its caller holds no reference either, so the joiner may
        //   free the instant it observes zero.
        let _prev = unsafe { (*shared).pending.fetch_sub(1, Ordering::AcqRel) };

        // THE RELEASE POINT. Miri-only; compiles to nothing natively, the same
        // device as the yields in `join_on_worker`, `join_external_helping` and
        // `worker::worker_main` (Phase 9.1 H2). Here it is a GATE rather than a
        // scheduling aid: the decrement above is what lets the joiner free this
        // allocation, so holding this frame open across a few scheduling rounds
        // is the interleaving that decides whether any frame of this thread
        // still holds a protected reference into it. With the burst,
        // `tests/miri_scope_completion_protector.rs` goes red DETERMINISTICALLY
        // the moment a reference-typed receiver comes back, and stays green
        // while the receiver is a raw pointer; without it that verdict is left
        // to Miri's random preemption, which is how this window survived a
        // 10 975-window probe. Do not delete it without its twin on the W-d′
        // route: they are one gate, one probe per route.
        //
        // NATIVE COST: none, and for a narrower reason than the first draft of
        // this comment claimed. The whole `cfg(miri)` block is absent natively,
        // so NOTHING READS `_prev`, and an unread binding of a value the
        // instruction already computes costs nothing. The draft justified it
        // with "`lock xadd` produces the old value in a register whether or not
        // anything reads it", which is FALSE on this target: MEASURED at 1.97.1
        // / opt-level 3, all three forms of this statement compile to
        // `lock decq` and no `lock xadd` is emitted at all; a native READ of the
        // previous value would be synthesised from the flags with a `sete`, i.e.
        // it would cost an instruction. That matters to whoever later moves the
        // `_prev` read out from under `cfg(miri)` — the freedom here is "nothing
        // reads it", not "reading it is free".
        #[cfg(miri)]
        if _prev == 1 {
            miri_release_probe(miri_key);
        }
    }

    /// Returns `true` once every spawned task has completed. Polled by the
    /// work-stealing join wait in [`join_workers_until_drained`].
    ///
    /// `Acquire` pairs with the `AcqRel` decrement in [`Self::complete_task`]
    /// so that, when this observes zero, the completing tasks' writes are
    /// visible to the joiner.
    #[inline]
    pub(crate) fn is_drained(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }
}

impl Drop for ScopeShared {
    /// Reclaims an uncollected panic payload.
    ///
    /// `Scope::drop` normally hands the payload off via [`take_panic`](ScopeShared::take_panic)
    /// before freeing the allocation, so this fires only when a `ScopeShared` is dropped without
    /// that hand-off — the `loom_exports` test wrapper constructs one directly. Without it a
    /// captured payload would leak; the previous `Mutex<Option<..>>` got this for free from
    /// `Option`'s drop glue, and dropping to a raw pointer gives up that guarantee unless it is
    /// restated here.
    #[inline]
    fn drop(&mut self) {
        // A `swap`, not `get_mut()`: the loom `AtomicPtr` this shim resolves to under `--cfg loom`
        // has no `get_mut`, and the doc above names the loom wrapper as this path's one caller —
        // so spelling it `get_mut` made the whole `--cfg loom` LIB unbuildable and took every loom
        // model with it. `Acquire` pairs with `capture_panic`'s release half, exactly as
        // `take_panic` does, so the payload's contents are visible to this drop.
        let raw = self.panic_payload.swap(ptr::null_mut(), Ordering::Acquire);
        if !raw.is_null() {
            // SAFETY: `&mut self` proves exclusive access. A non-null value here was published
            //   by one `capture_panic` CAS and never taken, so this is its unique owner; the
            //   pointer came from `Box::into_raw(Box::new(..))`, matching this `Box::from_raw`.
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

/// Fork/join scope over a [`ThreadPool`](crate::ThreadPool).
///
/// Constructed by [`ThreadPool::install`] or [`ThreadPool::scope`].
/// Spawn child tasks via [`Scope::spawn`]; the scope blocks at drop time
/// until every spawned task has completed.
///
/// `#[repr(C, align(64))]` is bought deliberately, and the field ORDER is the
/// purchase: a spawn touches `inner`, `shared` and the block's `cur` / `end` —
/// bytes 0..32 under this order — so the alignment makes that span ONE cache
/// line always, rather than a straddle whose probability is set by where the
/// joiner's frame happened to land. The price is 56 bytes of stack padding once
/// per scope, paid for one guaranteed line per spawn. The order is PINNED below
/// by `offset_of!`, because the size and alignment pins there do not cover it —
/// see the comment on those pins for the edit that keeps both of them green
/// while breaking this paragraph.
///
/// [`ThreadPool::install`]: crate::ThreadPool::install
/// [`ThreadPool::scope`]: crate::ThreadPool::scope
#[repr(C, align(64))]
pub struct Scope<'scope> {
    /// The pool's worker-shared state this scope spawns into. Borrowed for
    /// `'scope` (Phase 9.3b decision E: `&PoolInner`, not the handle — the
    /// handle keeps `inner` alive across the `install`/`scope` frame that
    /// borrows it, so this reference is valid for `'scope`).
    inner: &'scope PoolInner,
    /// Owned `ScopeShared` held as a raw `NonNull` (Phase 9.2 Candidate U —
    /// the Tree-Borrows-protector fix). `NonNull::as_ptr` is a `Copy` that
    /// copies the pointer WITHOUT retagging the pointee, so `Scope::drop`'s
    /// `&mut self` protector covers only this 8-byte field, never the heap
    /// allocation that worker threads concurrently write `pending` into.
    /// Created by `Box::into_raw` in [`Scope::new`], freed by `Box::from_raw`
    /// in [`Scope::drop`] (the single free site).
    pub(crate) shared: NonNull<ScopeShared>,
    /// This scope's cell storage: every task spawned into the scope has its
    /// payload cell emplaced here by [`Scope::prepare`], and the whole block is
    /// returned to the allocator by ONE `free_all` in [`Scope::drop`],
    /// immediately after the join.
    ///
    /// Held INLINE and third. Inline because a `Box` would put the bump cursor
    /// one dependent load away from the spawn path and add an allocation to
    /// every scope, including the ones that spawn nothing (the block itself is
    /// lazy — `ScopeBlock::new` allocates nothing). Third because the block's
    /// own two hot words lead its layout, so they land in bytes 16..32 and
    /// share this struct's first cache line with the two fields above.
    block: ScopeBlock,
    /// `PhantomData<&'scope mut &'scope ()>` makes the scope invariant in
    /// `'scope`, which is what we want — `'scope` is a borrow window, not
    /// a covariant lifetime.
    _phantom: PhantomData<&'scope mut &'scope ()>,
}

// 8 (`inner`) + 8 (`shared`) + 312 (`ScopeBlock`, pinned in `block.rs`) + 0
// (`PhantomData`) = 328 bytes of content, rounded up to the 64-byte alignment
// above = 384. The size is what the padding cost is stated over, and the
// ALIGNMENT is the thing the one-cache-line claim rests on — a size pin alone
// would survive the alignment being dropped, which is exactly the edit that
// silently turns the guaranteed line back into a probability.
//
// NEITHER OF THOSE TWO COVERS THE FIELD ORDER, and the field order is what the
// doc comment above says was bought. MEASURED, by compiling the reordering
// against this pin set: declaring `block` FIRST gives `block`@0, `inner`@312,
// `shared`@320, `_phantom`@328 — still 328 bytes of content, still
// `size_of == 384`, still `align_of == 64`, so the two pins above stay SILENT
// while the four hot fields end up on THREE different cache lines (`cur`/`end`
// at 0..16 in the first, `inner` at 312..320 in the fifth, `shared` at 320..328
// in the sixth). The property the alignment was paid for is then gone with no
// build failure at all. That reordering is the edit the three `offset_of!` pins
// below catch — all three fired on the probe — and the two above cannot.
//
// The pins state one half of a composition. The other half is `cur`@0 /
// `end`@8 INSIDE `ScopeBlock`, pinned beside that struct's field list in
// `block.rs` — it has to be stated there, because `offset_of!` respects field
// visibility and both fields are private to that module. Composed:
// `block`@16 with `cur`@0 / `end`@8 puts the block's two hot words at bytes
// 16..32 of this struct, and `inner`@0 / `shared`@8 puts the scope's own two
// ahead of them, so all four lie in bytes 0..32 of a 64-aligned allocation.
//
// `inner` and `shared` are pinned for the reader rather than for the property:
// swapping just those two would preserve the one-line claim, and `block`@16 is
// the load-bearing line. They are stated because the byte span in the doc
// comment is read off them, and a reader re-deriving 0..32 should not have to
// reconstruct it from `repr(C)` plus two sizes.
const _: () = assert!(size_of::<Scope<'static>>() == 384);
const _: () = assert!(align_of::<Scope<'static>>() == 64);
const _: () = assert!(core::mem::offset_of!(Scope<'static>, inner) == 0);
const _: () = assert!(core::mem::offset_of!(Scope<'static>, shared) == 8);
const _: () = assert!(core::mem::offset_of!(Scope<'static>, block) == 16);

impl<'scope> Scope<'scope> {
    #[inline]
    pub(crate) fn new(inner: &'scope PoolInner, shared: Box<ScopeShared>) -> Self {
        // Cold path: runs single-threaded before any task is spawned, so the
        // conversion never races a worker. `Box::into_raw` hands ownership of
        // the allocation to this `NonNull`; `Scope::drop` reclaims it.
        let shared = NonNull::new(Box::into_raw(shared))
            .expect("invariant: Box::into_raw never yields null");
        Self {
            inner,
            shared,
            // Lazy: no chunk is allocated until the first spawn, so a scope
            // that spawns nothing still makes no allocator call for its block.
            block: ScopeBlock::new(),
            _phantom: PhantomData,
        }
    }

    /// Spawn a child task. The closure may borrow data with lifetime
    /// `'scope`; the scope blocks at drop until the task completes, so
    /// the borrow remains valid.
    ///
    /// # Panic semantics
    /// A panic inside `f` is captured into the scope's panic payload and
    /// re-raised on the calling thread when the scope drops. The first
    /// panic wins; subsequent panics are dropped.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'scope,
    {
        // SAFETY: `self.shared` points to the live `ScopeShared` allocation
        //   owned by this `Scope` (created in `new`, freed only by `drop`).
        //   Stated against the PROTECTOR rule, which is the one that judges a
        //   reborrow whose allocation another thread might free: the protector
        //   this reborrow installs cannot span a free, because THIS thread is
        //   the only thread that can free. `Scope` holds a `NonNull`, so
        //   `Scope: !Sync`, so `&Scope: !Send` — no other thread can hold this
        //   `&self`, and the single free site is this thread's own
        //   `Scope::drop`, which cannot run while `&self` is borrowed. (The
        //   older reading — "runs on the owner thread before the task is
        //   enqueued, so it does not race any worker" — is true and answers the
        //   DATA-RACE rule; it would not have covered a free, which is what
        //   `complete_task` had to be reshaped for.)
        unsafe { self.shared.as_ref() }.register_task();
        push_task(self.inner, self.prepare(f));
    }

    /// Spawn a wave of at most `n` tasks (KE16 App-4, `KE16-DESIGN-W.md` §4).
    ///
    /// One [`spawn`](Self::spawn) per body: every body is registered and pushed
    /// on its own, and every push takes its own wake decision. `n` is therefore
    /// not a count that is registered — it is the caller's promise of an UPPER
    /// BOUND on how many bodies `bodies` will yield, and the API exists so that a
    /// caller whose chunk count is only an upper bound has one call to make.
    ///
    /// `bodies` must yield at most `n` closures (debug-asserted); fewer is
    /// allowed and costs nothing, because nothing is registered for a body that
    /// was never produced. The same property covers an iterator that panics
    /// part-way: it leaves no registration behind for the scope's join — which
    /// runs during that very unwind — to wait on.
    ///
    /// Panic semantics of the bodies are [`spawn`](Self::spawn)'s.
    pub fn spawn_batch<I, F>(&self, n: usize, bodies: I)
    where
        I: IntoIterator<Item = F>,
        F: FnOnce() + Send + 'scope,
    {
        let mut k = 0usize;
        for f in bodies {
            self.spawn(f);
            k += 1;
        }
        debug_assert!(
            k <= n,
            "invariant: `spawn_batch` bodies yielded more than the `n` it was promised"
        );
    }

    /// Build the queue element for one body: the payload cell, the address of
    /// the scope's shared state, and the lifetime erasure.
    ///
    /// Registration is the CALLER's — [`Scope::spawn`] registers each task on the
    /// statement immediately above its push, and [`Scope::spawn_batch`] routes
    /// every body through it — so this is the one seam every spawn takes, and
    /// there is no second copy of the erasure argument below.
    ///
    /// # What used to be here, and why nothing inside it could have saved it
    ///
    /// This function built `move || { catch_unwind(AssertUnwindSafe(f)); ...;
    /// complete_task(shared) }` and boxed it as a `dyn FnOnce`. That made the
    /// user's body — and every `&T` nested inside it — a by-value ARGUMENT of
    /// the same `call_once` activation that performed the release, so at the
    /// instant the joiner became free to reclaim the borrowed memory, a live
    /// protector still covered it. Tree Borrows judges a protector by the
    /// ACTIVATION it belongs to, to the closing brace, not by the last access;
    /// no reordering INSIDE that wrapper could fix it, because the release was
    /// in the wrapper. The body had to leave the releasing frame's argument list
    /// entirely, and it now has: `task::run_scoped` reads the body out of the
    /// cell and hands it to a `catch_unwind` that RETURNS before
    /// [`ScopeShared::complete_task`] is entered.
    fn prepare<F>(&self, f: F) -> Task
    where
        F: FnOnce() + Send + 'scope,
    {
        // `NonNull::as_ptr` copies the pointer WITHOUT retagging the pointee
        // (Phase 9.2 Candidate U), so producing this address forms no tag over
        // the allocation and no protector over it exists here either.
        let shared = self.shared.as_ptr() as *const ScopeShared;

        // SAFETY (`new_scoped`'s first clause — the registration): the caller
        //   registered this task before reaching here (`Scope::spawn`, on the
        //   statement immediately above its push; `Scope::spawn_batch` routes
        //   every body through it), so `pending` already counts it and the
        //   single free site cannot have run. The element's `run_scoped` completes that
        //   registration exactly once: a `Task` is `!Copy`, the queues hand each
        //   element to exactly one consumer, and `Task::run` forgets the element
        //   so the teardown thunk cannot also fire.
        //
        //   The `ScopeShared` ADDRESS crosses a thread boundary inside the cell,
        //   so the pointee has to be sound to SHARE and not merely to send.
        //   Field by field, because `ScopeShared` is not auto-`Sync` (this is
        //   the clause the deleted `SharedPtr` wrapper used to carry):
        //   - `pending` (`CachePadded<AtomicUsize>`) and `panic_payload`
        //     (`AtomicPtr`) are atomics — `Sync` by construction;
        //   - `waker` (`std::thread::Thread`) is `Send + Sync`;
        //   - `joiner_wake` (KE16 W-d′) is a RAW POINTER, which is what costs the
        //     type its auto-`Sync`. Its clause: it is written exactly once, in
        //     `ScopeShared::new`, before the allocation is published and
        //     therefore before any `register_task` can hand this address to
        //     another thread; it is never written again by anyone; and the only
        //     read of it is the by-value copy at the top of `complete_task`. A
        //     by-value read of a never-mutated pointer races with nothing, and
        //     the one dereference — inside `complete_task`, after the decrement —
        //     carries its own SAFETY block for the pointee. A later pass that
        //     makes the field mutable, or that writes it after the first spawn,
        //     breaks THIS clause and with it the soundness of every task that
        //     carries the address, not merely a style rule.
        //
        // SAFETY (`new_scoped`'s second clause — the `'scope` -> `'static`
        //   erasure): the body borrows data with lifetime `'scope`, and coercing
        //   `run_scoped::<F>` to a plain `unsafe fn(*const ())` erases that
        //   lifetime — the coercion replaces the `mem::transmute` of a
        //   `Box<dyn FnOnce + 'scope>` this function used to perform, and
        //   inherits its obligation unchanged. Sound because:
        //     - `Scope::drop` blocks until `pending == 0` via the work-stealing
        //       wait (`join_workers_until_drained`);
        //     - the blocking happens BEFORE any `'scope` borrow can expire
        //       (`Scope::drop` runs while `'scope` is still live; the user's
        //       `install`/`scope` call frame still holds the borrow);
        //     - even when the calling thread panics, `Drop` runs during
        //       unwinding (Rust's stack-unwinding semantics).
        //   The only edge case is `std::process::abort` / SIGKILL: if the
        //   process is terminated mid-task, workers may continue accessing freed
        //   stack frames in the brief window before the kernel reclaims memory.
        //   This is observable only at the language level; no real program can
        //   observe the UB because the process is gone. Same edge case as
        //   rayon's `scope`.
        //
        // SAFETY (`new_scoped`'s third clause — the block that holds the cell):
        //   `&self.block` is THIS scope's block, so it is the block of the
        //   `ScopeShared` named above, and its single `free_all` is in
        //   `Scope::drop` behind the same join as the free of that
        //   `ScopeShared` — so the cell outlives the task's execution for
        //   exactly the reason the registration clause gives. `&self` is what
        //   makes this a shared borrow rather than a unique one, and the
        //   protector it installs covers the 312-byte block STRUCT in this
        //   frame, never the chunks: `emplace` writes through a `*mut T` and
        //   hands back a `BlockPtr` whose only exit is `erase`, so no reference
        //   into chunk memory is formed here (D1/D2 in `block.rs`'s header).
        unsafe { Task::new_scoped(&self.block, shared, f) }
    }
}

impl<'scope> Drop for Scope<'scope> {
    fn drop(&mut self) {
        // `NonNull::as_ptr` is a `Copy` that copies the pointer WITHOUT
        // retagging the pointee, so this Drop's `&mut self` protector covers
        // only the 8-byte `shared` field, never the heap allocation that worker
        // threads write `pending` into (the Phase 9.2 Candidate U TB fix).
        let raw: *mut ScopeShared = self.shared.as_ptr();

        // SAFETY: `raw` is live for the whole join — it is freed only by the
        //   single `Box::from_raw` below, which runs after this returns. The
        //   join reborrows `*raw` only per-poll for one `Acquire` load, never
        //   forming a reference that spans a worker's `pending` write (the
        //   raw-pointer / NonNull design). The `*mut` coerces to the `*const`
        //   parameter.
        unsafe { join_workers_until_drained(self.inner, raw) };

        // Miri-only: the window key of the scope being torn down, read ONCE
        // here and used by both notes below. It has to be read before the first
        // reclamation either way — after the `Box::from_raw` the field is no
        // longer there — and one read makes it evident on sight that the block
        // note and the `ScopeShared` note are keyed by the SAME scope identity,
        // which is the whole basis for comparing their two counts.
        //
        // SAFETY: `*raw` is live and this thread is its sole owner: the join
        //   above observed `pending == 0`, no task of this scope exists, and the
        //   single free is the `Box::from_raw` below, which has not run. The key
        //   is a `Copy` place read through the raw pointer, so it forms no
        //   reference and leaves no protector behind for either free to trip
        //   over. `cfg(miri)` only.
        #[cfg(miri)]
        let miri_key = unsafe { (*raw).miri_key };

        // Miri-only: record whether the CHUNK free below lands inside one of
        // this scope's own release windows (KE16 M2w). A DIFFERENT event from
        // the note further down — that one observes the `ScopeShared` free —
        // and it is counted separately for that reason; see
        // `MIRI_BLOCK_FREES_INSIDE_A_RELEASE_WINDOW`. Once per `Scope::drop`
        // rather than once per chunk, because the property is about the free
        // SITE and not about how many chunks it happens to hand back, and
        // guarded by `is_empty` so a scope that never grew a chunk does not
        // report a free it never performed.
        #[cfg(miri)]
        if !self.block.is_empty() {
            miri_note_block_free_against_open_windows(miri_key);
        }

        // FIRST reclamation of the scope, immediately after the join and before
        // anything that could unwind — which is what makes the block's
        // leak-safety structural instead of a `Drop` impl (`block.rs`'s header).
        //
        // SAFETY (`free_all`'s two preconditions):
        //   - EVERY EMPLACED VALUE IS ALREADY DEAD, and the join is what
        //     establishes it. Each cell in the block belongs to a task that was
        //     registered in `pending`; the join returned, so `pending == 0`, so
        //     every one of them either RAN — `run_scoped` moved its body out of
        //     the cell by `ptr::read` before completing the registration — or
        //     was dropped unrun, in which case `Task::drop`'s thunk dropped the
        //     body in place. A task that had done neither would still be
        //     counted, and this line would not have been reached.
        //   - NO `BlockPtr` OR ERASED COPY IS DEREFERENCED AFTERWARDS. The only
        //     copy that ever leaves this thread is `Task.payload`, and the
        //     element carrying it is consumed by whichever of the two thunks
        //     ran; `emplace` keeps no handle, and `BlockPtr::erase` consumed the
        //     typed one at the spawn. Nothing on this thread names a chunk
        //     either: `free_all` reads the table out of this frame.
        //   Called once — `Drop` runs once, and the call is unconditional.
        unsafe { self.block.free_all() };

        // The join returned ⇒ `pending == 0` (the final wave's decrement). No
        // worker will start a new `complete_task`, and every worker that ran
        // has completed its `fetch_sub`, which happens-before the join's
        // `Acquire` load. The dispatcher is now the sole owner.
        //
        // SAFETY: pre-free shared access through the raw pointer. `is_drained`
        //   is an `Acquire` load and `panic_payload` is an `AtomicPtr` (Sync). The
        //   payload is taken BEFORE the free so that no `*raw` access follows
        //   the deallocation.
        debug_assert!(
            unsafe { (*raw).is_drained() },
            "Scope::Drop returned with pending tasks still in flight"
        );
        // No lock on the common path: a scope whose tasks all completed reads a null pointer.
        let payload = unsafe { (*raw).take_panic() };

        // SAFETY (the single free site — Phase 9.2 Candidate U, completed here):
        //   - `raw` is the `Box::into_raw` address minted in `Scope::new`.
        //
        //   - NO LIVE PROTECTOR COVERS THIS ALLOCATION. This, not the ordering
        //     clause below, is the rule that judges a deallocation: Tree
        //     Borrows forbids freeing memory that a live protector covers, and
        //     a protector is live for as long as the CALL whose reference-typed
        //     ARGUMENT it guards, whether or not that call touches a byte. The
        //     three threads that can be inside this allocation's API at this
        //     instant are covered:
        //       * this one — the join took `*const ScopeShared` BY VALUE and
        //         `Scope::drop` reborrows only per statement (`is_drained`,
        //         `take_panic`), each expired before this line;
        //       * every completer — `ScopeShared::complete_task` is an
        //         associated function over `*const Self`, so the frame that
        //         performs the releasing `pending.fetch_sub` holds NO reference
        //         into the allocation; the only protector alive at that RMW is
        //         `AtomicUsize::fetch_sub`'s own all-`UnsafeCell` (weak) `&self`,
        //         which does not forbid deallocation;
        //       * every task body still running — it cannot be, because its own
        //         registration would still be counted in `pending`.
        //     WHAT WAS BELIEVED until KE16: that "every worker's last allocation
        //     ACCESS — its `pending.fetch_sub` — happens-before the join's
        //     `Acquire` load" was the whole argument. It is true and it is the
        //     DATA-RACE argument; it says nothing about protectors, and a
        //     `complete_task(&self)` satisfied it while still holding a strong
        //     protector over this allocation's frozen bytes (`waker`,
        //     `joiner_wake`, `CachePadded` padding) past the decrement.
        //
        //   - The ordering clause still holds and is still needed for the
        //     CONTENTS: the join observed `pending == 0`, and every completer's
        //     `fetch_sub` happens-before that `Acquire` load, so the payload
        //     this thread took above is the fully published one.
        //
        //   - The payload was taken above, before this free; no `*raw` access
        //     follows. Reached once (Drop runs once), unconditionally, so the
        //     allocation is freed exactly once — no double-free, multi-drain-safe
        //     (the free is tied to scope END, never to an intermediate wave's
        //     `pending -> 0`).
        // Miri-only: record whether this free landed inside one of THIS SCOPE's
        // own release windows. That overlap is the interleaving the
        // completion-protector gate exists to force, and the only thing that
        // certifies the gate is armed - see
        // `MIRI_FREES_INSIDE_A_RELEASE_WINDOW`. The key was the allocation's
        // ADDRESS until Stage 3y; an address makes the match depend on the
        // allocator never recycling one across two scopes, which Miri's
        // same-thread reuse rate of 0.5 does not promise (`MIRI_SCOPE_EPOCH`).
        //
        // POSITION UNCHANGED by Stage 3b: this note stays the last thing before
        // the `ScopeShared` free, because it is that free it observes. The key
        // it reads was hoisted above the block free, where it is read once for
        // both notes.
        #[cfg(miri)]
        miri_note_free_against_open_windows(miri_key);

        unsafe { drop(Box::from_raw(raw)) };

        // Re-raise OUTSIDE any `*raw` access (the payload is a moved-out stack
        // local that no longer aliases the freed allocation).
        if let Some(p) = payload {
            resume_unwind(p);
        }
    }
}

/// Block (with work stealing) until `shared.pending` is zero. Plan §4.5.5,
/// `KE16-DESIGN-B.md` §2.1.
///
/// Called from `Scope::Drop`. It does not park unconditionally: it polls every
/// source it can act through and runs what it finds inline, and parks only on the
/// [`JOIN_BACKSTOP`] timeout — a re-poll trigger, not a wait anyone should read as
/// 50 µs, since `park_timeout` rounds up to the OS timer quantum (`dur2timeout`,
/// `std/src/sys/pal/windows/mod.rs`; ~15.3 ms unguarded and ~1.0 ms while
/// `boyko_app`'s `TimerResolutionGuard` holds 1 ms, KE16 App-7 / App-12). The
/// real wake-up arrives from a completing task
/// ([`ScopeShared::complete_task`]), so the timeout only bounds the rare
/// lost-wakeup window — and a backstop that costs a millisecond, not 50 µs, is
/// why that window is priced rather than dismissed.
///
/// One dispatch on the ONE identity predicate (App-6, [`tls::worker_lane_for`]):
///
/// - `Some(lane)` — a registered worker of THIS pool, acting as that worker,
///   inside a task body. It owns a deque every sibling can steal from, so it
///   joins through that deque ([`join_on_worker`]).
/// - `None` — the dispatcher, an unattached thread, a worker of ANOTHER pool, or
///   a worker inside an `install` frame of this pool (whose id is the dispatcher
///   sentinel for that frame). No deque to act through in this pool
///   ([`join_external`]).
///
/// The predicate is the dispatch because `wid < worker_count` is not an identity
/// test: it is true for a worker of another pool, which would then act on
/// per-worker structures of `inner` that belong to somebody else (J14).
///
/// # Safety
/// `shared` must point to a live `ScopeShared` that stays valid for the whole
/// call. The caller (`Scope::drop`) upholds this: the allocation is freed only by
/// the single `Box::from_raw` that runs after this function returns. Both routes
/// reborrow `*shared` only per-poll for one `Acquire` load and never form a
/// reference that spans a worker's `pending` write.
unsafe fn join_workers_until_drained(inner: &PoolInner, shared: *const ScopeShared) {
    match tls::worker_lane_for(inner) {
        // SAFETY: the caller's contract is handed through unchanged — `shared`
        //   is live for the whole of this call, hence for the whole of the
        //   callee's.
        Some(lane) => unsafe { join_on_worker(inner, shared, lane) },
        // SAFETY: as above.
        None => unsafe { join_external(inner, shared) },
    }
}

/// The worker joiner: run one task per `is_drained` check, taking it from the
/// lane's own deque or stealing INTO that deque; park idle-marked when there is
/// nothing to run (rule B1-P).
///
/// Three properties (`KE16-DESIGN-B.md` §2.2):
///
/// - **No sink.** Every batch lands in the deque `inner.stealers[wid]` publishes,
///   so the residue this joiner does not run is stealable by every sibling and by
///   an external joiner — nothing it takes is hidden from the pool.
/// - **A re-check per task.** The scope is consulted between tasks, so the joiner
///   stops helping the instant its own wave completes rather than running a whole
///   stolen residue first.
/// - **A parked joiner is a lane** (B1-P): its idle bit is set while it parks, so
///   ANY other wave's wake decision can claim it through the same
///   `claim_one_idle` that claims a worker. Cost: one `fetch_or` + one
///   `fetch_and` per PARK, never per task. A claim is an OBLIGATION as much as a
///   wake — it spent the pool's single wake on this thread — so NO exit from
///   that park sequence drops it: an exit that runs a task carries the
///   obligation to the next `is_drained` check, and one that returns into the
///   task body without scanning hands the wake to a sibling.
///
/// No `&Worker` spans a task body (discipline D5, `KE16-DESIGN-A.md` §1.3):
/// `lane` is `Copy` over a RAW pointer, each `lane.deque()` is consumed by one
/// call in its own statement (never an `if let` scrutinee, whose temporary would
/// live through the THEN block in Rust 2024), and the helpers hold their `local`
/// argument only while they run — none of them runs a body. That is what lets a
/// body run INLINE here and push through the same TLS deque.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
unsafe fn join_on_worker(inner: &PoolInner, shared: *const ScopeShared, lane: tls::WorkerLane) {
    let wid = lane.wid;
    debug_assert!(
        (wid as usize) < inner.worker_count() as usize,
        "worker_lane_for handed out a lane whose id is not a worker of this pool"
    );
    let self_bit = 1u64 << wid;
    let mut rng = XorShift64Star::new(splitmix64((wid as u64) ^ (shared as usize as u64)));
    let backoff = Backoff::new();
    // A foreign wave's wake decision took this lane's idle bit — while this
    // thread was parked, or during the post-`mark_idle` re-poll — and this
    // joiner has not scanned since. The pool issues ONE wake per push
    // (`claim_one_idle` clears one bit and unparks that thread), so an exit
    // taken while this is set must pass the wake to a lane that will scan. Every
    // `unmark_idle` site below reads the claim off its own RMW: the drained exit
    // hands it on immediately, the other two record it here and the top of the
    // loop then either scans (which clears it) or returns (which hands it on).
    let mut owes_wake = false;

    loop {
        // Miri-only cooperative yield, as in `worker_main`: a joiner that keeps
        // finding work would otherwise starve its siblings under Miri's
        // cooperative scheduler. Native: compiles to nothing.
        #[cfg(miri)]
        std::thread::yield_now();

        // SAFETY: per the function contract `shared` is live for this whole
        //   call; this is a transient `Acquire` load that does not span a
        //   worker's `pending` write.
        if unsafe { (*shared).is_drained() } {
            // This return goes back into a task body WITHOUT looking at any
            // source, so a claim taken while this thread was parked would die
            // here: the claimer's wake was spent on a thread that then did not
            // scan, and its work can sit visible while every other lane sleeps
            // in `worker_main`'s UNTIMED park. Hand it to a lane that scans.
            if owes_wake {
                unpark_one_idle_excluding(inner, self_bit);
            }
            return;
        }
        // Everything below this line is a full scan of every source, which is
        // all a claim asks of the lane it wakes.
        owes_wake = false;

        // 1. Own deque, owner end — a FIFO owner end, so this takes the OLDEST
        //    entry of the wave being joined, the one pushed first. The `&Worker`
        //    is consumed by `pop()` in THIS statement; the body runs in the next
        //    one (D5).
        let popped = lane.deque().pop();
        if let Some(t) = popped {
            run_task(t);
            backoff.reset();
            continue;
        }

        // 2. Global injector, batch INTO the own registered deque: what this
        //    joiner does not get to is still stealable. On the frame path that
        //    batch can hold conflict-free sibling SYSTEMS, which then run inline
        //    inside this system's body — legal since App-8 turned the
        //    `InSystemRunGuard` into a depth counter (`KE16-DESIGN-B.md` §2.5).
        let popped = pop_global_injector(inner, wid, lane.deque());
        if let Some(t) = popped {
            run_task(t);
            backoff.reset();
            continue;
        }

        // 3. Random-start, self-skipped sibling sweep (App-3), batch into the own
        //    deque. Self is skipped by the PREDICATE's `wid`, so the lane skipped
        //    is exactly the lane being fed.
        let popped = try_steal_random(inner, wid, lane.deque(), &mut rng);
        if let Some(t) = popped {
            run_task(t);
            backoff.reset();
            continue;
        }

        // 4. Nothing stealable: snooze, then park the way `worker_main` parks
        //    (rule B1-P).
        if backoff.is_completed() {
            mark_idle(&inner.idle, wid);

            // Post-`mark_idle` re-poll, load-bearing for the same reason as in
            // `worker_main` (Race C): a pusher that read `idle == 0` just before
            // this bit was set must not have its wake fall between the chairs.
            let popped = pop_any(inner, wid, lane.deque(), &mut rng);
            if let Some(t) = popped {
                // A claim that landed BEFORE this scan is honoured by it. One
                // that lands after its last probe and before the clear below is
                // not — that scan ran while the claimer's push was still
                // invisible — so the claim is read off the clear here exactly as
                // it is at the park below, and CARRIED: this thread runs the task
                // it already holds and then either scans at the top of the loop
                // (obligation discharged) or returns and hands the wake to a
                // sibling. What this exit guarantees is that the obligation
                // survives it, not that it is already met.
                owes_wake = unmark_idle(&inner.idle, wid) & self_bit == 0;
                run_task(t);
                backoff.reset();
                continue;
            }

            // Re-check the SCOPE with the bit set: the last completer's unpark
            // may have landed between step 3 and `mark_idle`, and parking on a
            // token already consumed would sleep to the backstop. One `Acquire`
            // load removes even that.
            //
            // SAFETY: as the load at the top of the loop — `shared` is live for
            //   the whole call and this is a transient `Acquire` read.
            if unsafe { (*shared).is_drained() } {
                // Same exit hazard as the top of the loop, one window earlier: a
                // claim may have taken this bit between `mark_idle` and here.
                // `unmark_idle` returns the word it cleared, so "the bit was no
                // longer mine" is read atomically rather than from a load the
                // claim could slip past.
                if unmark_idle(&inner.idle, wid) & self_bit == 0 {
                    unpark_one_idle_excluding(inner, self_bit);
                }
                return;
            }

            // Hand work that is VISIBLE but unclaimed to a sibling before
            // sleeping on it; `self_bit` is excluded because a claim that picked
            // this thread would be a wake spent on the thread that issued it.
            unpark_one_idle_excluding(inner, self_bit);
            std::thread::park_timeout(JOIN_BACKSTOP);
            // Two wakes can land on this one parker and they COALESCE into a
            // single token: this scope's last completer (`complete_task`'s
            // `unpark`) and a foreign wave's `claim_one_idle`. They are told
            // apart by their trace, not by the park — only the claim clears this
            // bit — so the claim is read off the clear and carried to the top of
            // the loop, which either scans (obligation discharged) or returns
            // (obligation passed on).
            owes_wake = unmark_idle(&inner.idle, wid) & self_bit == 0;
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}

/// The HELPING external joiner: one task at a time, then park
/// (`KE16-DESIGN-B.md` §2.3).
///
/// One task at a time because the joiner has no registered deque *to act
/// through in this pool*: a batch would have to land either in an unregistered
/// sink, where no sibling could reach the residue, or in a registry that does not
/// exist (B1(i)). One CAS per task on the joiner, no residue, nothing hidden from
/// the pool.
///
/// The design's §2.2 bullet says an external joiner "has no deque". That is true
/// of the route it was written for (the dispatcher, an unattached thread, a
/// worker of another pool) and FALSE of one caller: a worker of THIS pool inside
/// an `install` frame, whose deque is still registered in `inner.stealers` — it
/// is simply not this frame's lane, which is why the sweep below is allowed to
/// probe it like any other. Refusing to help that one caller would be a hang
/// rather than a lost lane: at `num_threads(1)`, or whenever every worker is
/// inside such a frame, it is the only thread that could run what the frame
/// spawned.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
unsafe fn join_external_helping(inner: &PoolInner, shared: *const ScopeShared) {
    let mut rng = XorShift64Star::new(splitmix64(shared as usize as u64));
    let backoff = Backoff::new();

    loop {
        #[cfg(miri)]
        std::thread::yield_now();

        // SAFETY: per the function contract `shared` is live for this whole call;
        //   a transient `Acquire` load that does not span a worker write.
        if unsafe { (*shared).is_drained() } {
            return;
        }

        if let Some(t) = drain_one(|| inner.injector_global.steal()) {
            run_task(t);
            backoff.reset();
            continue;
        }
        if let Some(t) = steal_one_random(inner, &mut rng) {
            run_task(t);
            backoff.reset();
            continue;
        }

        if backoff.is_completed() {
            // A sibling that pushed a wave may not have raced through its wake
            // yet, and the work IS visible: let a parked worker take it.
            unpark_one_idle(inner);
            std::thread::park_timeout(JOIN_BACKSTOP);
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}

/// The external joiner: [`join_external_helping`], unconditionally — an external
/// joiner of this pool always helps.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
#[inline]
unsafe fn join_external(inner: &PoolInner, shared: *const ScopeShared) {
    // SAFETY: the caller's contract is handed through unchanged — `shared` is
    //   live for the whole of this call, hence for the whole of the callee's.
    unsafe { join_external_helping(inner, shared) }
}

/// Steal ONE task from a random sibling; no self-skip, because an external
/// joiner has no lane in this pool to skip.
///
/// `worker::try_steal_random`'s sweep with `Stealer::steal()` in place of
/// `steal_batch_and_pop`: one task, one CAS and one epoch pin per successful
/// probe, and no destination deque to receive a residue.
///
/// The sweep probes EVERY registered deque, this thread's own included when the
/// caller happens to own one (a worker of `inner` inside an `install` frame):
/// that deque is not this frame's lane, so it is a victim like any other, and
/// crossbeam's `Stealer::steal` is correct from the owning thread.
///
/// The registered deques are the whole scan set OF THIS SWEEP, and not of the
/// join: a worker's own spawns land on its own registered deque, but a wave
/// pushed by a thread with no lane in `inner` goes to `injector_global`
/// (`worker::place_task`), which the single caller
/// [`join_external_helping`] probes on the statement above this one.
///
/// Gated on `Stealer::is_empty` for the same reason as
/// [`worker::try_steal_random`], whose doc comment carries the argument:
/// `Stealer::steal` pins the epoch (crossbeam-deque 0.8.7 `deque.rs:650`)
/// BEFORE loading `back` and deciding the deque is empty (`:653`), so an
/// external joiner sweeping an idle pool paid a thread-local access per empty
/// victim. Skipping a victim that a concurrent push fills a moment later is
/// benign here too: this joiner loops until the scope's count reaches zero, so
/// the next iteration of the sweep sees it, and the pusher's own wake decision
/// covers a joiner that has since parked.
///
/// [`worker::try_steal_random`]: crate::worker::try_steal_random
fn steal_one_random(inner: &PoolInner, rng: &mut XorShift64Star) -> Option<Task> {
    let n = inner.stealers.len();
    if n == 0 {
        return None;
    }
    let start = (rng.next() as usize) % n;
    for k in 0..n {
        let idx = (start + k) % n;
        // The empty-victim gate (see this function's doc comment).
        if inner.stealers[idx].is_empty() {
            continue;
        }
        if let Some(t) = drain_one(|| inner.stealers[idx].steal()) {
            return Some(t);
        }
    }
    None
}

/// Resolve one `Steal` result, retrying only the `Retry` answer.
///
/// Its callers are the external joiner's two probes: the global injector in
/// [`join_external_helping`], and the sibling sweep in [`steal_one_random`].
#[inline]
fn drain_one<F>(mut f: F) -> Option<Task>
where
    F: FnMut() -> Steal<Task>,
{
    loop {
        match f() {
            Steal::Success(t) => return Some(t),
            Steal::Empty => return None,
            Steal::Retry => {
                // Miri-only cooperative yield in this unbounded steal-retry
                // loop (Phase 9.1 H2). Byte-identical native: compiles away.
                #[cfg(miri)]
                std::thread::yield_now();
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThreadPoolBuilder;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize};

    #[test]
    fn scope_drain_with_no_tasks_is_noop() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        pool.install(|_scope| {
            // no-op
        });
    }

    #[test]
    fn scope_spawn_can_borrow_stack_data() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let counter = AtomicU32::new(0);
        pool.install(|scope| {
            for _ in 0..32 {
                scope.spawn(|| {
                    counter.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        assert_eq!(counter.load(Ordering::Acquire), 32);
    }

    #[test]
    fn scope_propagates_panic() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let arc_pool = pool;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            arc_pool.install(|scope| {
                scope.spawn(|| panic!("planned"));
            });
        }));
        assert!(result.is_err(), "panic should propagate out of install");
    }

    #[test]
    fn nested_scope_does_not_deadlock() {
        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let counter = Arc::new(AtomicU32::new(0));
        let pool_for_outer = Arc::clone(&pool);

        pool.install(|outer| {
            for _ in 0..4 {
                let c = Arc::clone(&counter);
                let inner_pool = Arc::clone(&pool_for_outer);
                outer.spawn(move || {
                    inner_pool.scope(|inner| {
                        for _ in 0..8 {
                            let c2 = Arc::clone(&c);
                            inner.spawn(move || {
                                c2.fetch_add(1, Ordering::Relaxed);
                            });
                        }
                    });
                });
            }
        });

        assert_eq!(counter.load(Ordering::Acquire), 32);
    }

    /// Phase 9.2 Candidate U multi-drain regression (§11.4).
    ///
    /// Drives a SINGLE scope through several waves where `ScopeShared.pending`
    /// returns toward zero between waves *before* the scope drops — the pattern
    /// the deleted `free_state` handshake mis-freed on (it elected a freer on
    /// every `pending -> 0`, double-freeing on wave 2). Candidate U ties the
    /// single `Box::from_raw` to `Scope::drop` (scope END) alone, so repeated
    /// intermediate drains are harmless.
    ///
    /// Each wave's tasks are made to finish before the next wave spawns by
    /// spinning a bounded number of yields until the wave's `done` counter
    /// reaches `PER_WAVE` — by which point those tasks have called
    /// `complete_task` (unpark + `fetch_sub`), returning `pending` toward 0.
    /// If the scope leaked or double-freed, the native allocator (and the
    /// stress-test Drop accounting / Miri under the orchestrator) catch it.
    ///
    /// `done` is an `Arc<AtomicUsize>` (heap, `'static`-capable) rather than a
    /// per-wave stack local: `scope.spawn` requires the body to outlive
    /// `'scope`, so a fresh borrow created inside the `install` closure cannot
    /// be captured by spawned tasks — the `Arc` clone is the correct in-scope
    /// approximation of the executor's between-wave drive.
    #[test]
    fn scope_multi_drain_frees_once() {
        const WAVES: usize = 8;
        const PER_WAVE: usize = 4;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let done = Arc::new(AtomicUsize::new(0));
        let done_main = Arc::clone(&done);

        pool.install(move |scope| {
            for wave in 0..WAVES {
                // Alternate the two spawn ENTRY POINTS so the multi-drain
                // property is asserted over both: a direct `spawn` per task, and
                // a whole wave handed to `spawn_batch`. A wave that registered
                // wrongly would show up here as a wave that never drains or as a
                // scope freed mid-life.
                if wave % 2 == 0 {
                    for _ in 0..PER_WAVE {
                        let d = Arc::clone(&done_main);
                        scope.spawn(move || {
                            d.fetch_add(1, Ordering::Relaxed);
                        });
                    }
                } else {
                    let d = Arc::clone(&done_main);
                    scope.spawn_batch(
                        PER_WAVE,
                        (0..PER_WAVE).map(move |_| {
                            let d = Arc::clone(&d);
                            move || {
                                d.fetch_add(1, Ordering::Relaxed);
                            }
                        }),
                    );
                }

                // Let THIS wave drain (its tasks reach `complete_task`, driving
                // `pending` back toward 0) before spawning the next wave, so the
                // scope sees several `pending -> 0` transitions over its life.
                // Bounded yields — never an unbounded spin — so a stuck wave
                // fails fast instead of hanging.
                let target = (wave + 1) * PER_WAVE;
                let mut spins = 0u32;
                while done_main.load(Ordering::Acquire) < target && spins < 10_000_000 {
                    std::thread::yield_now();
                    spins += 1;
                }
            }
        });
        // <-- the ONLY free, here at Scope::drop. A per-wave free would have
        //     double-freed above.

        assert_eq!(
            done.load(Ordering::Acquire),
            WAVES * PER_WAVE,
            "every wave's tasks must have run exactly once"
        );
    }

    /// KE16 App-4, `k == n`: the promised count is the count that runs, and the
    /// scope drains.
    #[test]
    fn spawn_batch_runs_every_promised_body() {
        const N: usize = 64;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            scope.spawn_batch(
                N,
                (0..N).map(move |_| {
                    let r = Arc::clone(&ran_main);
                    move || {
                        r.fetch_add(1, Ordering::Relaxed);
                    }
                }),
            );
        });

        assert_eq!(ran.load(Ordering::Acquire), N, "every promised body ran");
    }

    /// KE16 App-4, `k < n`: an iterator shorter than its promise is simply fewer
    /// spawns, and the join ends.
    ///
    /// The failure mode this guards against is a HANG, not an assertion: a
    /// `spawn_batch` that took `n` as a count to register up front, and did not
    /// give back the `n - k` registrations no task will ever decrement, would
    /// leave `Scope::drop` waiting for completions that cannot come. The
    /// assertion below is therefore about the bodies; reaching it at all is the
    /// property.
    #[test]
    fn spawn_batch_with_fewer_bodies_than_promised_drains() {
        const PROMISED: usize = 32;
        const YIELDED: usize = 5;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            scope.spawn_batch(
                PROMISED,
                (0..YIELDED).map(move |_| {
                    let r = Arc::clone(&ran_main);
                    move || {
                        r.fetch_add(1, Ordering::Relaxed);
                    }
                }),
            );
        });

        assert_eq!(
            ran.load(Ordering::Acquire),
            YIELDED,
            "exactly the yielded bodies ran"
        );
    }

    /// KE16 App-4, `k > n`: yielding more bodies than promised is a contract
    /// violation and a debug panic.
    ///
    /// Release builds do not panic — they register the surplus instead, because
    /// an unregistered task can drive `pending` to zero while it still runs —
    /// so the test itself is debug-only rather than carrying a release arm that
    /// would assert nothing.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "yielded more than the `n` it was promised")]
    fn spawn_batch_with_more_bodies_than_promised_is_a_debug_panic() {
        const PROMISED: usize = 2;
        const YIELDED: usize = 8;

        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        pool.install(|scope| {
            scope.spawn_batch(PROMISED, (0..YIELDED).map(move |_| || {}));
        });
    }

    /// KE16 App-4's receipt: the wave's wake decision is taken after its FIRST
    /// push, not after its last.
    ///
    /// The iterator itself is the instrument. While it is still producing
    /// bodies — the spawner has pushed exactly one — it waits, bounded, for a
    /// sibling to have STARTED the wave. That can only happen if the first push
    /// already carried the wake decision; a wave that woke nobody until its last
    /// push would leave the flag clear until this thread stopped producing, and
    /// the receipt below would be false.
    ///
    /// Every push takes its own wake decision, so the wave's first push takes
    /// one: the property asserted here is one both spawn entry points have, and
    /// the test is not a way to tell them apart.
    #[test]
    fn spawn_batch_wake_decision_precedes_the_rest_of_the_wave() {
        const N: usize = 64;
        // Generous: it bounds a scheduler wake-up, and a miss here is a hard
        // failure rather than a retry, so the bound must not be the thing that
        // fails on a loaded box.
        const RECEIPT_BOUND: Duration = Duration::from_secs(5);

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let started = Arc::new(AtomicU32::new(0));
        let observed = Arc::new(AtomicU32::new(0));
        let started_main = Arc::clone(&started);
        let observed_main = Arc::clone(&observed);

        pool.install(move |scope| {
            scope.spawn_batch(
                N,
                (0..N).map(move |i| {
                    if i == 1 {
                        let deadline = std::time::Instant::now();
                        while started_main.load(Ordering::Acquire) == 0
                            && deadline.elapsed() < RECEIPT_BOUND
                        {
                            std::thread::yield_now();
                        }
                        observed_main
                            .store(started_main.load(Ordering::Acquire), Ordering::Release);
                    }
                    let s = Arc::clone(&started_main);
                    move || {
                        s.fetch_add(1, Ordering::Relaxed);
                    }
                }),
            );
        });

        assert_eq!(
            started.load(Ordering::Acquire),
            N as u32,
            "every body of the wave ran"
        );
        assert!(
            observed.load(Ordering::Acquire) > 0,
            "no sibling had started the wave while the spawner was still pushing it: \
             the wake decision did not precede the rest of the wave"
        );
    }
    /// The live `pending` count of `scope`, read from the owner thread.
    ///
    /// The tests below are the only readers of the wave's accounting from
    /// outside `spawn_batch`, and each of them reads it either before any task
    /// of the wave exists or after every task of the wave has finished, so the
    /// value it sees is not a snapshot of a race.
    fn pending_of(scope: &Scope<'_>) -> usize {
        // SAFETY: `scope.shared` names the live `ScopeShared` allocation of this
        //   scope — created in `Scope::new` and freed only by `Scope::drop`,
        //   which cannot have run while `scope` is borrowed here. Against the
        //   PROTECTOR rule: the `&ScopeShared` this mints is protected for the
        //   duration of the `load` call, and no free can occur inside it because
        //   `&Scope: !Send` (the `NonNull` field costs `Scope` its `Sync`) puts
        //   the only freeing thread here, holding this borrow. The atomic /
        //   `UnsafeCell` clause below is a separate and still-true statement: it
        //   answers why the concurrent worker RMWs are not a data race, which is
        //   a different rule from the one that would have judged a free.
        unsafe { scope.shared.as_ref() }
            .pending
            .load(Ordering::Acquire)
    }

    /// KE16 App-4's accounting receipt: WHEN the wave's registrations are taken,
    /// observed at the instant the iterator yields body 0.
    ///
    /// `spawn_batch` registers per task, immediately before each push, so at that
    /// instant nothing of the wave has been registered and `pending` is exactly
    /// 0. The reading is not a race — the wave's first push has not happened yet.
    #[test]
    fn spawn_batch_registers_nothing_before_its_first_body() {
        const N: usize = 16;
        // No reading of `pending` can produce this, so a wave whose iterator was
        // never pulled fails instead of matching a real reading.
        const UNSET: usize = usize::MAX;

        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let seen = Arc::new(AtomicUsize::new(UNSET));
        let seen_main = Arc::clone(&seen);

        pool.install(move |scope| {
            scope.spawn_batch(
                N,
                (0..N).map(move |i| {
                    if i == 0 {
                        seen_main.store(pending_of(scope), Ordering::Release);
                    }
                    || {}
                }),
            );
        });

        let observed = seen.load(Ordering::Acquire);
        assert_ne!(observed, UNSET, "the wave's iterator was never pulled");
        assert_eq!(
            observed, 0,
            "`spawn_batch` must have registered nothing before its first body is pulled"
        );
    }

    /// KE16 App-4's `k < n` accounting MEASURED rather than inferred: after a
    /// wave whose iterator was shorter than its promise, `pending` is exactly
    /// zero.
    ///
    /// `spawn_batch_with_fewer_bodies_than_promised_drains` proves only that the
    /// join ends, and it proves it by hanging when it does not. This reads the
    /// count itself, from the owner thread, while the scope is still alive and
    /// after every spawned body has completed: a wave that registered more than
    /// it pushed leaves `pending` above zero, and one that gave back more than it
    /// registered wraps it to a huge value — both fail the bounded wait below
    /// with a message instead of stopping the suite.
    #[test]
    fn spawn_batch_settles_pending_to_zero_after_a_short_wave() {
        const PROMISED: usize = 32;
        const YIELDED: usize = 5;
        const SPIN_BOUND: u32 = 10_000_000;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            scope.spawn_batch(
                PROMISED,
                (0..YIELDED).map(move |_| {
                    let r = Arc::clone(&ran_main);
                    move || {
                        r.fetch_add(1, Ordering::Relaxed);
                    }
                }),
            );

            let mut spins = 0u32;
            while pending_of(scope) != 0 && spins < SPIN_BOUND {
                std::thread::yield_now();
                spins += 1;
            }
            assert_eq!(
                pending_of(scope),
                0,
                "`pending` never returned to zero: the wave accounted for a different number \
                 of tasks than it spawned"
            );
        });

        assert_eq!(
            ran.load(Ordering::Acquire),
            YIELDED,
            "exactly the yielded bodies ran"
        );
    }

    /// KE16 App-4 edge, `n == 0`: an empty wave registers nothing, pushes
    /// nothing and leaves the scope drained.
    ///
    /// Reachable in production: `boyko_physics`'s cell-chunk dispatch computes
    /// its wave count from the cell count, which is zero for an empty pass.
    #[test]
    fn spawn_batch_of_zero_runs_nothing_and_leaves_the_scope_drained() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            let bodies: std::iter::Empty<fn()> = std::iter::empty();
            scope.spawn_batch(0, bodies);
            assert_eq!(
                pending_of(scope),
                0,
                "an empty wave must leave `pending` untouched"
            );
            ran_main.fetch_add(0, Ordering::Relaxed);
        });

        assert_eq!(ran.load(Ordering::Acquire), 0, "no body existed to run");
    }

    /// KE16 App-4 edge, `n == 1`: the minimal wave, whose first push IS its last,
    /// so the loop over the remaining bodies runs zero times.
    #[test]
    fn spawn_batch_of_one_runs_its_only_body() {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            scope.spawn_batch(
                1,
                std::iter::once(move || {
                    ran_main.fetch_add(1, Ordering::Relaxed);
                }),
            );
        });

        assert_eq!(ran.load(Ordering::Acquire), 1, "the single body ran once");
    }

    /// KE16 App-4: an iterator that PANICS part-way through the wave reports
    /// that panic instead of hanging the scope's join.
    ///
    /// Registering per task is what buys this: a body that was never produced was
    /// never registered, so the unwind leaves nothing on `pending` for
    /// `Scope::drop` — which runs during that very unwind — to wait for. Any
    /// future shape that registers the wave up front owes a give-back on the
    /// unwind path too (`KE16-DESIGN-W.md` §4.1's literal trailing
    /// `if k < n { … }` does not run on an unwind), and this is the test that
    /// catches its absence. The observable difference is termination, so what
    /// this asserts is that it finishes at all, with the iterator's own message
    /// rather than a hang.
    #[test]
    #[should_panic(expected = "spawn_batch test iterator: deliberate mid-wave panic")]
    fn spawn_batch_whose_iterator_panics_part_way_does_not_hang_the_join() {
        const PROMISED: usize = 32;
        const PANIC_AT: usize = 4;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        pool.install(|scope| {
            scope.spawn_batch(
                PROMISED,
                (0..PROMISED).map(|i| {
                    assert!(
                        i < PANIC_AT,
                        "spawn_batch test iterator: deliberate mid-wave panic"
                    );
                    || {}
                }),
            );
        });
    }

    /// KE16 App-4, `k > n` in RELEASE: the surplus is registered rather than left
    /// unaccounted, so every body still runs and the scope still drains.
    ///
    /// The debug arm of this contract is
    /// `spawn_batch_with_more_bodies_than_promised_is_a_debug_panic`; the release
    /// path has a different property and so needs its own test. Registering per
    /// task is what gives it: with the `debug_assert!` compiled out, the surplus
    /// goes through `spawn` like every other body and is counted. Leaving a
    /// spawned task unregistered would let a still-running task drive `pending`
    /// to zero and the joiner free `ScopeShared` under it, which is what this
    /// asserts cannot happen at `k > n`. Runs only under `cargo test --release`;
    /// in a debug build the code under test panics by design.
    #[test]
    #[cfg(not(debug_assertions))]
    fn spawn_batch_with_more_bodies_than_promised_still_runs_every_body_in_release() {
        const PROMISED: usize = 2;
        const YIELDED: usize = 8;

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_main = Arc::clone(&ran);

        pool.install(move |scope| {
            scope.spawn_batch(
                PROMISED,
                (0..YIELDED).map(move |_| {
                    let r = Arc::clone(&ran_main);
                    move || {
                        r.fetch_add(1, Ordering::Relaxed);
                    }
                }),
            );
        });

        assert_eq!(
            ran.load(Ordering::Acquire),
            YIELDED,
            "every body ran even though the wave was promised fewer"
        );
    }

    /// KE16 App-4, the `(n, k)` contract over its whole legal DOMAIN rather
    /// than the three points the design's obligation list names.
    ///
    /// `spawn_batch_runs_every_promised_body` (`k == n`, one `n`),
    /// `spawn_batch_with_fewer_bodies_than_promised_drains` (`k < n`, one pair)
    /// and the two edges (`n == 0`, `n == 1`) are four samples of a
    /// two-dimensional domain. The grid pins the contract over the whole of it
    /// instead: `n` is a promise the implementation is free to use for
    /// accounting, and an accounting error that is exact at `k == n` and off by
    /// one everywhere else — a wave registered up front and given back
    /// `n - k - 1`, say — survives all four samples and fails somewhere in here.
    ///
    /// Exhaustive rather than randomised: the interesting region is small
    /// (`n <= 12`), the failure is a HANG rather than a wrong value, and a
    /// hanging case must be REPRODUCIBLE by name. 91 `(n, k)` pairs at each of
    /// two worker counts — 182 waves, the first of which has no siblings at all
    /// (`W == 1`, where the spawner is the only lane and the wave is drained
    /// entirely by `Scope::drop`'s helping join).
    ///
    /// The assertion is on the bodies; TERMINATING is the property. Any
    /// accounting that leaves `pending` above zero makes `Scope::drop` wait
    /// forever, so a defect here stops the suite at a named case instead of
    /// reporting a number.
    #[test]
    fn spawn_batch_drains_every_legal_promise_and_yield_pair() {
        const MAX_N: usize = 12;

        for workers in [1usize, 4] {
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();

            for n in 0..=MAX_N {
                for k in 0..=n {
                    let ran = Arc::new(AtomicUsize::new(0));
                    let ran_main = Arc::clone(&ran);

                    pool.install(move |scope| {
                        scope.spawn_batch(
                            n,
                            (0..k).map(move |_| {
                                let r = Arc::clone(&ran_main);
                                move || {
                                    r.fetch_add(1, Ordering::Relaxed);
                                }
                            }),
                        );
                    });

                    assert_eq!(
                        ran.load(Ordering::Acquire),
                        k,
                        "wave (n = {n}, k = {k}) at {workers} worker(s) ran the wrong number \
                         of bodies"
                    );
                }
            }
        }
    }
}
