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
//! Instead, it polls the local injector (if on a worker), the global
//! injector, and sibling stealers. Without this, nested scopes can
//! deadlock when every worker is itself blocked inside its own
//! `Scope::Drop`.
//!
//! Note (plan §4.5.5 / Round 3 W-NEW-2, corrected by KE16), describing the
//! DEFAULT build: the joiner never reaches the calling worker's own Chase-Lev
//! deque as its OWNER — that deque is a local of `worker_main` and no owning
//! handle is in scope here. It does reach its contents as a THIEF:
//! `try_steal_any` walks every registered stealer including the calling worker's
//! own, so a batch of up to 33 of the joiner's own tasks lands in the stack-local
//! `scratch` deque and is run serially from there (KE16 defect B — `scratch` is
//! unregistered, so no sibling can take any of that batch back). Progress itself
//! rests on the injectors and the sibling steals.
//!
//! KE16 axis B (this paragraph and its features are deleted once the verdict
//! lands): under `ke16-b1` / `ke16-b3` the join dispatches on the ONE identity
//! predicate instead. A joining WORKER of this pool DOES reach its own deque as
//! the owner — `tls::worker_lane_for` hands it the lane `worker_main` deposited —
//! so it pops its own newest chunk, batch-steals into that same REGISTERED deque
//! (the residue stays stealable; `scratch` is gone) and re-checks the scope
//! between tasks; when it must park it parks idle-marked (rule B1-P), which makes
//! a parked joiner a claimable lane for any other wave. An EXTERNAL joiner — the
//! dispatcher, an unattached thread, a worker of another pool, a worker inside an
//! `install` frame of this pool — has no lane here: under `ke16-b1` it steals one
//! task at a time (no unregistered sink), under `ke16-b3` it only parks, EXCEPT
//! when it is a worker of this pool inside an `install` frame, whose refusal
//! would be a hang rather than a lost lane (that frame's pushes are reachable
//! only through this pool's own workers, and at `num_threads(1)` this thread is
//! all of them). Nested scopes still cannot deadlock: a joiner either runs a
//! ready task or parks with a wake guaranteed by its own scope's last completer,
//! by a foreign wave's claim (B1-P), or by the backstop.

use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::any::Any;
use std::panic::resume_unwind;
use std::time::Duration;

// === KE16 B switch: ke16-b1 / ke16-b3 === The B0 joiner's `scratch` is the only
// deque this module owns, and both B arms delete it (`KE16-DESIGN-B.md` §2.2):
// their worker arm steals into the lane's REGISTERED deque and their external
// arm takes one task at a time, so neither has a sink to receive a batch.
#[cfg(not(any(feature = "ke16-b1", feature = "ke16-b3")))]
use crossbeam_deque::Worker;
use crossbeam_deque::Steal;
use crossbeam_utils::{Backoff, CachePadded};

use crate::sync::{AtomicPtr, AtomicUsize, Ordering, Thread};
use crate::task::Task;
use crate::thread_pool::PoolInner;
// === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 === The joiner's ONE use
// of the identity predicate is step 1's own-slot lookup, which those arms have
// no queue for; the B axis brings the next use (a worker joiner that pops its
// own deque).
// === KE16 B switch: ke16-b1 / ke16-b3 === adds the SECOND use of the predicate:
// the joiner's own dispatch (`KE16-DESIGN-B.md` §2.1). The B arms exist only on
// `ke16-a1` / `ke16-a1-fifo` (the `compile_error!` matrix in `lib.rs`), so that
// arm of the `any` below is what makes `tls` reachable there.
#[cfg(any(
    not(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3")),
    feature = "ke16-b1",
    feature = "ke16-b3"
))]
use crate::tls;
use crate::worker::{push_task, unpark_one_idle};
// === KE16 C switch: ke16-c-batch === The batch spawn is the one caller that
// separates placement (`push_task_no_wake` for the wave's first push,
// `push_task_silent` for the rest) from the wave's single wake decision
// (`wake_after_push`, or `wake_up_to` under `ke16-w-fanout`), so it reaches the
// module rather than a fixed set of names.
#[cfg(feature = "ke16-c-batch")]
use crate::worker;
// === KE16 B switch: ke16-b1 / ke16-b3 === The B worker arm joins through the
// worker loop's OWN poll helpers rather than a second copy of the same scan; the
// two idle-bitset primitives are rule B1-P's park (`KE16-DESIGN-B.md` §2.2).
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
use crate::worker::{
    XorShift64Star, mark_idle, pop_any, pop_global_injector, run_task, splitmix64,
    try_steal_random, unmark_idle, unpark_one_idle_excluding,
};

/// The joiner's park backstop — a RE-POLL trigger, not a wait.
///
/// 50 us is the SOURCE constant; `park_timeout` rounds it up to the OS timer
/// quantum (measured ~15.3 ms unguarded and ~1.0 ms while `boyko_app`'s
/// `TimerResolutionGuard` holds 1 ms — KE16 App-7 / App-12), which is why every
/// arm's real wake is a completer's `unpark` and this value only bounds the rare
/// lost-wakeup window. Named once so the B0 arm and the two B arms cannot drift
/// apart (`KE16-DESIGN-B.md` §2.2, §3).
///
/// Under `ke16-w-count` it stops being load-bearing on route (b): the count gate
/// makes a worker joiner's check-then-park race-free against its own last
/// completer, so this timeout costs nothing when the wake arrives and is kept
/// purely as a defensive bound (`KE16-DESIGN-W.md` §3.4). On the external arm,
/// which keeps the unpark-before-decrement order, it is still the only thing
/// that ends a lost wake — at a full timer quantum.
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
/// (loop exit, `scratch` drop, `take_panic`, `Box::from_raw`). Left to Miri's
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
/// Cost: `cfg(miri)` only, so the shipped artifact contains none of it (symbol
/// census identical to the pre-fix build, zero probe symbols); and even under
/// Miri it is taken only on `prev == 1`, i.e. once per `pending -> 0` transition
/// rather than once per task.
#[cfg(miri)]
const MIRI_RELEASE_PROBE_YIELDS: usize = 16;

/// Miri-only: slots recording which allocations currently have a completer
/// inside its post-decrement release window.
///
/// One `AtomicUsize` per concurrently-open window, holding the address of the
/// `ScopeShared` whose completer is inside the burst, or 0 for a free slot. Eight
/// is far more than the two or three a gate run ever opens at once; a completer
/// that finds none simply does not record its window, which can only LOSE an
/// observation and never invent one.
///
/// Directionality is the point: a slot holds an address only its own scope's
/// completers ever write, so a free that finds its own address there has
/// certainly caught one of its own completers mid-window. A clobbered slot
/// yields a MISS, which makes the gate red, never green.
#[cfg(miri)]
static MIRI_OPEN_RELEASE_WINDOWS: [core::sync::atomic::AtomicUsize; 8] =
    [const { core::sync::atomic::AtomicUsize::new(0) }; 8];

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

/// Miri-only: the release probe itself — one definition, called from both arms
/// of [`ScopeShared::complete_task`], so the two cannot drift apart.
///
/// Must be called ONLY on the `prev == 1` path and ONLY after the release RMW:
/// its whole purpose is to hold the caller's frame open across the window in
/// which the joiner may free the allocation, and to make that window OBSERVABLE
/// to the free site so the gate can assert it happened.
///
/// `addr` is the allocation this completer was handed, as an integer. An integer
/// deliberately, not a pointer: it is only ever compared for equality with the
/// free site's own address, never dereferenced, so the allocation may be freed
/// under it — which is precisely the situation being recorded.
///
/// `SeqCst` throughout: this is instrumentation whose whole value is that the
/// free site sees the freshest state, and its cost is irrelevant because none of
/// it exists outside `cfg(miri)`.
#[cfg(miri)]
#[inline(never)]
fn miri_release_probe(addr: usize) {
    use core::sync::atomic::Ordering::SeqCst;

    MIRI_RELEASE_PROBE_FIRINGS.fetch_add(1, SeqCst);

    // Claim a slot for the duration of the burst. Failure to find one loses an
    // observation and can only make the gate redder.
    let mut claimed: Option<usize> = None;
    for (i, slot) in MIRI_OPEN_RELEASE_WINDOWS.iter().enumerate() {
        if slot.compare_exchange(0, addr, SeqCst, SeqCst).is_ok() {
            claimed = Some(i);
            break;
        }
    }

    for _ in 0..MIRI_RELEASE_PROBE_YIELDS {
        std::thread::yield_now();
    }

    if let Some(i) = claimed {
        MIRI_OPEN_RELEASE_WINDOWS[i].store(0, SeqCst);
    }
}

/// Miri-only: called by `Scope::drop` immediately before the free, to record
/// whether this deallocation landed inside one of its own completers' release
/// windows.
///
/// See [`MIRI_FREES_INSIDE_A_RELEASE_WINDOW`] for why this, and not the probe's
/// firing count, is what certifies the gate.
#[cfg(miri)]
#[inline(never)]
fn miri_note_free_against_open_windows(addr: usize) {
    use core::sync::atomic::Ordering::SeqCst;

    if MIRI_OPEN_RELEASE_WINDOWS
        .iter()
        .any(|slot| slot.load(SeqCst) == addr)
    {
        MIRI_FREES_INSIDE_A_RELEASE_WINDOW.fetch_add(1, SeqCst);
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
    /// [`complete_task`](Self::complete_task), and only under `ke16-w-count`.
    // Kept unconditional so one struct serves every row of the tournament; the
    // arms that never read it also never write anything but null through it.
    #[cfg_attr(not(feature = "ke16-w-count"), allow(dead_code))]
    pub(crate) joiner_wake: *const crate::sync::WakeHandle,
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
        Self {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: AtomicPtr::new(ptr::null_mut()),
            waker,
            joiner_wake,
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

    /// Register `n` outstanding tasks in ONE RMW. Called by
    /// [`Scope::spawn_batch`] before the wave's first task is enqueued (KE16
    /// App-4: one `pending` RMW per wave instead of one per task).
    ///
    /// Ordering as [`register_task`](Self::register_task) — the count the
    /// increment publishes is the same count the completions decrement.
    #[inline]
    pub(crate) fn register_tasks(&self, n: usize) {
        self.pending.fetch_add(n, Ordering::AcqRel);
    }

    /// Give back `n` registrations that will never be spawned — the KE16 App-4
    /// correction for a `spawn_batch` whose iterator yielded fewer than `n`
    /// bodies (including the case where it unwound part-way).
    ///
    /// Sound because the ONLY caller is the scope's owner thread inside
    /// `spawn_batch`, which has not begun joining: no other thread ever reads
    /// `pending` to decide the allocation is free (the single free site is
    /// `Scope::drop`, on this thread), so a transient `pending == 0` here
    /// cannot free anything, and this thread performs no wake, so nothing
    /// observes the correction as a completion.
    // === KE16 C switch: ke16-c-batch ===
    #[cfg(feature = "ke16-c-batch")]
    #[inline]
    pub(crate) fn unregister_tasks(&self, n: usize) {
        self.pending.fetch_sub(n, Ordering::AcqRel);
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
    /// deleted — see `task.rs`), where the attribute does not survive. The old
    /// code was therefore safe by an inlining DECISION rather than by any
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
    /// At the instant either arm's release RMW commits, the only protector
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
    /// ORDER IS LOAD-BEARING (Phase 9.2 Candidate U) on the arm whose wake
    /// target lives in the allocation: `waker.unpark()` happens BEFORE
    /// `pending.fetch_sub`. While this task has not yet decremented,
    /// `pending >= 1`, so `Scope::drop`'s join cannot have observed zero and
    /// therefore cannot have freed the allocation — the `waker` read is sound.
    /// Multi-drain-safe: the free is tied to scope END, never to an
    /// intermediate wave's `pending -> 0` (the ECS executor drains `pending` to
    /// zero once per dispatch wave).
    ///
    /// The KE16 W-d′ arm below satisfies the same rule by a different route:
    /// its target is `PoolInner`-owned, so it is read out of `*shared` BEFORE
    /// the decrement and used AFTER it.
    ///
    /// The unpark is UNCONDITIONAL on the external arm (no `prev == 1` gate).
    /// NOT because being last is unknowable — `fetch_sub` returns the previous
    /// value, so `prev == 1` costs nothing to learn (the earlier claim that it
    /// "would require reading `pending` after the sub" was simply wrong, KE16
    /// N66). The binding constraint is the WAKE TARGET's lifetime: `waker`
    /// lives inside the allocation this decrement releases, so a gated wake has
    /// to read it after the sub, when the joiner may already have freed it.
    /// Gating it needs a target owned elsewhere — which is exactly what
    /// `ke16-w-count` supplies for a WORKER joiner
    /// ([`joiner_wake`](Self::joiner_wake), W-d′, `KE16-DESIGN-W.md` §3) and
    /// what no external joiner has (§3.5, recorded for KE17). The unconditional
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
        // === KE16 W switch: ke16-w-count ===
        #[cfg(feature = "ke16-w-count")]
        {
            // Copied out BEFORE the decrement: after it the joiner may free this
            // allocation, so `*shared` must not be touched again. A null target
            // is an external joiner and falls through to today's order below.
            //
            // SAFETY: `*shared` is live per this function's contract — this
            //   task is still counted in `pending`, and the only free site runs
            //   after a join that observed zero. A place-read through a raw
            //   pointer forms NO reference and therefore installs no protector.
            let target = unsafe { (*shared).joiner_wake };
            if !target.is_null() {
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

                    // THE RELEASE POINT on this arm, placed AFTER the unpark — and the
                    // reason is now stated at the strength it was actually
                    // established, which is weaker than the first draft claimed.
                    //
                    // The draft said the gate is "RED here and GREEN with the
                    // burst moved above the unpark, because this arm's joiner
                    // may be parked and a burst before the wake would hold the
                    // frame open while the only thread that could free was still
                    // blocked". MEASURED 2026-09-05 under
                    // `--features ke16-w-count,ke16-a2` at
                    // `-Zmiri-preemption-rate=0`, with a deliberately
                    // reference-typed receiver:
                    // `worker_joiner_completer_holds_no_protector_when_the_joiner_frees`
                    // is RED at BOTH placements. The disarming the draft
                    // predicted did not reproduce — this arm's joiner is polling
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
                    miri_release_probe(shared as usize);
                }
                return;
            }
        }
        // External joiner (or the feature off): today's order, for today's
        // reason — `waker` lives in this allocation.
        //
        // SAFETY: `*shared` is live per this function's contract: this task has
        //   not decremented yet, so `pending >= 1` and no join can have observed
        //   zero. The `&Thread` protector `unpark` installs covers frozen bytes
        //   of THIS allocation and is STRONG — which is sound precisely because
        //   it expires before the decrement below, not after it.
        unsafe { (*shared).waker.unpark() };
        // SAFETY: live until this RMW commits, for the reason above. When it
        //   commits, the only protector over this allocation is `fetch_sub`'s
        //   own all-`UnsafeCell` (weak) `&self`; this function holds `shared` by
        //   value and its caller holds no reference either, so the joiner may
        //   free the instant it observes zero.
        let _prev = unsafe { (*shared).pending.fetch_sub(1, Ordering::AcqRel) };

        // THE RELEASE POINT. Miri-only; compiles to nothing natively, the same
        // device as the yields in `join_workers_until_drained` and
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
        // arm: they are one gate, one probe per arm.
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
            miri_release_probe(shared as usize);
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
/// [`ThreadPool::install`]: crate::ThreadPool::install
/// [`ThreadPool::scope`]: crate::ThreadPool::scope
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
    /// `PhantomData<&'scope mut &'scope ()>` makes the scope invariant in
    /// `'scope`, which is what we want — `'scope` is a borrow window, not
    /// a covariant lifetime.
    _phantom: PhantomData<&'scope mut &'scope ()>,
}

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

    /// Spawn `n` tasks as ONE wave (KE16 App-4, `KE16-DESIGN-W.md` §4).
    ///
    /// One `pending` RMW for the wave instead of one per task, `n` queue
    /// insertions, and the wake decision taken ONCE — right after the FIRST
    /// push, so a parked sibling is already on its way while the remaining
    /// pushes land. The remaining pushes are silent.
    ///
    /// The single decision is the *wave's own*, and under `ke16-a5` it is
    /// PRE-EMPTED PER PUSH rather than removed: a first push whose placement
    /// actually CLAIMED an idle sibling unparks that sibling itself and reports
    /// that no decision is due, so `Scope::wake_for_wave` — the C arm's
    /// mechanism — is skipped for that wave; when the idle mask was empty or the
    /// claim CAS lost, A5 falls through to its A2 placement and the wave's
    /// decision is taken as normal (and finds nothing to wake). The `pending`
    /// saving is unaffected either way. That interaction is recorded on
    /// `wake_for_wave` and in the `ke16-w-fanout` feature row.
    ///
    /// `bodies` must yield at most `n` closures (debug-asserted); fewer is
    /// allowed and corrected, so a caller whose chunk count is an upper bound
    /// stays exact. The correction also covers an iterator that panics
    /// part-way: without it the scope's join would wait forever for bodies that
    /// were never spawned.
    ///
    /// Panic semantics of the bodies are [`spawn`](Self::spawn)'s.
    ///
    /// KE16 (deleted with the features): without `ke16-c-batch` this is one
    /// `spawn` per body — today's per-task registration and per-push wake
    /// decision — so the two arms differ in accounting and wake count, never in
    /// which tasks run.
    pub fn spawn_batch<I, F>(&self, n: usize, bodies: I)
    where
        I: IntoIterator<Item = F>,
        F: FnOnce() + Send + 'scope,
    {
        // === KE16 C switch: ke16-c-batch ===
        #[cfg(not(feature = "ke16-c-batch"))]
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

        #[cfg(feature = "ke16-c-batch")]
        {
            // SAFETY: `self.shared` points to the live `ScopeShared` allocation
            //   owned by this `Scope` (created in `new`, freed only by `drop`).
            //   Protector-rule statement, as in `spawn` above: this reborrow's
            //   protector cannot span a free because `Scope: !Sync` (it holds a
            //   `NonNull`) makes `&Scope: !Send`, so the owner thread is the only
            //   thread that can reach the single free site, and it is here
            //   instead. Note this reborrow OUTLIVES the wave's pushes, so the
            //   argument has to be about the free and not merely about the two
            //   accounting RMWs: workers are concurrently decrementing `pending`
            //   under it, and one of them may drive it to zero mid-wave. That is
            //   sound for the same reason — they cannot free.
            let shared = unsafe { self.shared.as_ref() };
            shared.register_tasks(n);
            // Holds the registrations no body has claimed yet, and gives back
            // whatever is left when this returns OR unwinds.
            let mut wave = WaveRegistration {
                shared,
                unpushed: n,
            };

            let mut bodies = bodies.into_iter();
            if let Some(first) = bodies.next() {
                wave.charge_one();
                // `None` means the placement already carried its own targeted
                // wake (the A5 arm), so the wave owes no decision.
                if let Some(pre_len) = worker::push_task_no_wake(self.inner, self.prepare(first)) {
                    Self::wake_for_wave(self.inner, pre_len, n);
                }
                for f in bodies {
                    // The wave's remaining pushes are silent: the first push
                    // carried the fenced wake decision, and these land in a
                    // queue an awake thief is already draining or about to
                    // (`KE16-DESIGN-W.md` §2.3 item 3). Silent also means they
                    // do not COMPUTE a decision's input: `push_task_silent`
                    // skips the pre-push length probe, which under
                    // `ke16-w-gate` is a `SeqCst` load of a contended line and
                    // would otherwise be paid `n` times for a value read once
                    // (`KE16-DESIGN-W.md` §4.3, "N−1 `len()` loads").
                    wave.charge_one();
                    worker::push_task_silent(self.inner, self.prepare(f));
                }
            }
        }
    }

    /// The wave's ONE wake decision, taken right after its first push.
    ///
    /// `pre_len` is that push's destination length and gates the decision on
    /// both arms; `n` is the wave size and is read only by W-f, which widens the
    /// wake from one worker to `min(n, popcount(idle))` without touching whether
    /// a wake is owed. This is the W-f switch's only site.
    ///
    /// ⚠ **ONE decision activates ONE sibling, and only the W axis re-fans it
    /// out.** Without `ke16-w-fanout` this call ends in
    /// [`worker::wake_after_push`] → `unpark_one_idle`, which claims exactly one
    /// idle bit and unparks exactly one worker; a parked worker waits on an
    /// UNTIMED `std::thread::park()` (`worker.rs`, `worker_main`'s park arm) and
    /// moves only on an explicit unpark. What turns that one wake into a wave's
    /// worth of lanes is [`worker::wake_after_residue`], W-b's thief-residue
    /// cascade — and that helper is `#[cfg(feature = "ke16-w-gate")]`, a no-op
    /// without it. So a build with `ke16-c-batch` ALONE takes a wave from the
    /// `min(n, popcount(idle))` siblings the per-task baseline wakes (every push
    /// wakes when the gate is absent) down to the spawner plus ONE, with the
    /// rest asleep for the whole wave. That is a lane-count difference of up to
    /// W/2, not the `pending`-accounting saving the arm is named for, and it
    /// lands hardest at 100 µs–1 ms × W, where the siblings are parked at the
    /// wave boundary — the very cells `KE16-DESIGN-W.md` §4.3 predicts as
    /// "invisible". A `c1` row is therefore interpretable ONLY relative to the
    /// wake configuration it was taken under: `ke16-c-batch` measured on top of
    /// `ke16-w-gate` (the cascade) or `ke16-w-fanout` isolates App-4's
    /// accounting; `ke16-c-batch` alone against a `c0` baseline does not.
    /// Whether the batch's single decision should become a fan-out
    /// unconditionally, or `ke16-c-batch` should only ever be measured on top of
    /// a fan-out mechanism, is a DESIGN call and is escalated, not decided here
    /// (`KE16-DESIGN.md` §2 App-4 says "so parked *siblings* start stealing",
    /// plural; §4.1's soundness argument for the silent pushes borrows §2.3
    /// item 3, which is W-b's).
    ///
    /// **Under `ke16-a5` this is CONDITIONAL, not absent.** A5's placement
    /// answers `None` — "the placement already carried its wake" — only when it
    /// claimed an idle bit; when the mask was zero or the single claim CAS lost
    /// (`worker::try_place_on_idle_sibling`, both `Err(task)` exits) the push
    /// falls through to A5's A2 placement, answers `Some(pre_len)`, and this
    /// runs. So an `a5 + c-batch` build takes one A5 wake per push for as long
    /// as siblings are parked and the wave's own decision on the pushes where
    /// none was claimable; an `a5 + w-fanout` build reaches
    /// [`worker::wake_up_to`] on exactly those waves and pays one
    /// `publish_fence` (a local `mfence`) plus one `Acquire` load of the
    /// contended `idle` line, almost always to return 0 — the mask A5 read a
    /// few instructions earlier was empty, or its one CAS lost to a claimer
    /// that took the bit. The fan-out is therefore inert where
    /// it would help (siblings parked ⇒ A5 pre-empts it) and live as pure
    /// overhead where it would not — the 1 µs busy cells. `KE16_C` still reads
    /// `c1` / `c1f` throughout. The design declares the axes composable
    /// (`KE16-DESIGN.md` §4, "W/C: no exclusions") and simultaneously defines
    /// C's mechanism as the single wave decision this function is; resolving
    /// that is a design call, so the code states the interaction and the
    /// measurement protocol must not file a `c1`/`c1f` row from a build that
    /// also enables `ke16-a5`.
    // === KE16 C switch: ke16-w-fanout ===
    #[cfg(feature = "ke16-c-batch")]
    #[inline]
    fn wake_for_wave(inner: &PoolInner, pre_len: usize, n: usize) {
        #[cfg(feature = "ke16-w-fanout")]
        {
            // Both arms consult the SAME rule — `worker::wake_is_due(pre_len)`,
            // inside the two helpers — and differ only in how wide the wake is.
            // The fan-out is deliberately NOT gate-exempt: were it, a `c1f` row
            // would carry "the ≤1 rule lifted on the wave's first push" on top
            // of the fan-out count that is supposed to decide W-f, and the
            // `pre_len` the first push already paid a `SeqCst` probe for would
            // be discarded. The full argument, and the design clause it departs
            // from, are on `worker::wake_after_push_up_to`.
            //
            // The count is the arm's bound made assertable in the pool's own
            // tests; nothing on the spawn path reads it.
            let _woken = worker::wake_after_push_up_to(inner, pre_len, n);
        }
        #[cfg(not(feature = "ke16-w-fanout"))]
        {
            let _ = n;
            worker::wake_after_push(inner, pre_len);
        }
    }

    /// Build the queue element for one body: the payload cell, the address of
    /// the scope's shared state, and the lifetime erasure.
    ///
    /// Registration is the CALLER's (`spawn` per task, `spawn_batch` once per
    /// wave), so this is the one seam both spawn paths share and no arm of the
    /// tournament carries a second copy of the erasure argument below.
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
        //   registered this task before reaching here (`Scope::spawn` per task,
        //   `Scope::spawn_batch` once per wave, both immediately above their
        //   push), so `pending` already counts it and the single free site
        //   cannot have run. The element's `run_scoped` completes that
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
        unsafe { Task::new_scoped(shared, f) }
    }
}

/// The registrations of one [`Scope::spawn_batch`] wave that no body has
/// claimed yet (KE16 App-4's `k < n` correction).
///
/// A guard rather than a trailing `if k < n`: the correction must also run when
/// `bodies` unwinds part-way through the wave, or the scope's join — which
/// `Scope::drop` performs during that same unwind — would wait forever for
/// tasks that were never spawned.
// === KE16 C switch: ke16-c-batch ===
#[cfg(feature = "ke16-c-batch")]
struct WaveRegistration<'a> {
    shared: &'a ScopeShared,
    unpushed: usize,
}

#[cfg(feature = "ke16-c-batch")]
impl WaveRegistration<'_> {
    /// Charge one registration to the body that is about to be pushed.
    ///
    /// BEFORE the push, not after, because the debug panic below unwinds
    /// through `Scope::drop`, which joins: a surplus task that was already in a
    /// queue when the panic fired would be a completion `pending` never counted,
    /// and the join would wait forever instead of reporting the contract
    /// violation. MEASURED — charging after the push hung the k > n test.
    #[inline]
    fn charge_one(&mut self) {
        debug_assert!(
            self.unpushed > 0,
            "invariant: `spawn_batch` bodies yielded more than the `n` it was promised"
        );
        if self.unpushed > 0 {
            self.unpushed -= 1;
        } else {
            // The contract is broken (more bodies than `n`), which is the debug
            // panic above. In release the surplus is registered instead of being
            // left unaccounted: an unregistered task can drive `pending` to zero
            // while it is still running, and the joiner then frees
            // `ScopeShared` under it.
            self.shared.register_tasks(1);
        }
    }
}

#[cfg(feature = "ke16-c-batch")]
impl Drop for WaveRegistration<'_> {
    #[inline]
    fn drop(&mut self) {
        if self.unpushed > 0 {
            self.shared.unregister_tasks(self.unpushed);
        }
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
        // Miri-only: record whether this free landed inside one of THIS
        // allocation's own release windows. That overlap is the interleaving the
        // completion-protector gate exists to force, and the only thing that
        // certifies the gate is armed - see
        // `MIRI_FREES_INSIDE_A_RELEASE_WINDOW`. It touches no memory of the
        // allocation, only the integer address, so it is sound at the instant
        // before the free. `cfg(miri)` only.
        #[cfg(miri)]
        miri_note_free_against_open_windows(raw as usize);

        unsafe { drop(Box::from_raw(raw)) };

        // Re-raise OUTSIDE any `*raw` access (the payload is a moved-out stack
        // local that no longer aliases the freed allocation).
        if let Some(p) = payload {
            resume_unwind(p);
        }
    }
}

/// Block (with work stealing) until `shared.pending` is zero. Plan
/// §4.5.5.
///
/// This function is called from `Scope::Drop`. It steals work from any
/// stealable source (the calling worker's local injector if on a worker;
/// the global injector; any sibling stealer) and runs the stolen tasks
/// inline on the calling thread. When no work is stealable, it parks with a
/// timeout — a re-poll trigger, not a wait anyone should read as 50 µs: 50 µs is
/// the SOURCE CONSTANT, and `park_timeout` rounds up to the OS timer quantum
/// (`dur2timeout`, `std/src/sys/pal/windows/mod.rs`), which on this box measured
/// ~15.3 ms unguarded and ~1.0 ms while `boyko_app`'s `TimerResolutionGuard`
/// holds 1 ms (KE16 App-7 / App-12). The real wake-up arrives via
/// `shared.waker.unpark()` from a completing task (Phase 9.2 Candidate U: unpark
/// precedes the decrement), so the timeout is only the backstop for the rare
/// lost-wakeup window — and a backstop that costs a millisecond, not 50 µs, is
/// why that window is priced rather than dismissed.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` allocation that remains
/// valid for the entire duration of this call. The caller (`Scope::drop`)
/// upholds this: the allocation is freed only by the single `Box::from_raw`
/// that runs after this function returns. The function reborrows `*shared`
/// only per-poll for one `Acquire` load (`is_drained`), never forming a
/// reference that spans a worker's `pending` write.
// === KE16 B switch: ke16-b1 / ke16-b3 === The B arms replace this whole body
// (two adjacent functions with one name, `KE16-DESIGN.md` §4): a joiner that owns
// a registered deque must not steal into an unregistered `scratch`.
#[cfg(not(any(feature = "ke16-b1", feature = "ke16-b3")))]
unsafe fn join_workers_until_drained(inner: &PoolInner, shared: *const ScopeShared) {
    // KE16 App-6: the ONE identity predicate decides whether this joiner may
    // touch per-worker structures of `inner`. The bare `wid < len` test it
    // replaces was true for a worker of ANOTHER pool joining this scope, which
    // then drained `inner.injector_local[wid_of_the_other_pool]` — a slot owned
    // by a worker of `inner` that is the only thread allowed to drain it.
    // === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 === (those arms never
    // feed `injector_local`, so the joiner has no own slot to drain either)
    #[cfg(not(any(
        feature = "ke16-a1",
        feature = "ke16-a1-fifo",
        feature = "ke16-a3"
    )))]
    let own_local_inj: Option<usize> = tls::worker_lane_for(inner).map(|lane| lane.wid as usize);
    // Under A2 / A5 the sweep skips this joiner's own slot, which step 1 drains;
    // under the arms above there is no slot and the sweep skips nothing.
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo", feature = "ke16-a3"))]
    let own_local_inj: Option<usize> = None;

    // A temporary deque to receive batches stolen from injectors/sibling
    // stealers. Not exposed; lives on the dispatcher's stack frame for
    // the duration of this wait.
    let scratch: Worker<Task> = Worker::new_fifo();

    let backoff = Backoff::new();

    loop {
        // Under Miri the scheduler is cooperative: a dispatcher that steals,
        // runs, and `continue`s without ever reaching the backoff/park branch
        // would starve the workers. Yield each iteration so Miri can advance
        // the other threads. Compiles to nothing natively (Phase 9.1 H2).
        #[cfg(miri)]
        std::thread::yield_now();

        // SAFETY: per the function contract, `shared` is live for this whole
        //   call; this is a transient `Acquire` load that does not span a
        //   worker write.
        if unsafe { (*shared).is_drained() } {
            return;
        }

        // 1. If we're on a worker, drain inner-spawn tasks targeted at us.
        // Every stolen task below runs through `worker::run_task`, NOT a bare
        // `t.run()` — 2026-07 audit finding. These queues carry BOTH scope-spawned
        // tasks (whose body `task::run_scoped` already catches into the scope, so
        // the guard is inert for them) AND fire-and-forget `ThreadPool::spawn`
        // tasks, which `task::run_detached` catches NOWHERE — it hands the body
        // straight to the caller. A bare unwind here escapes `Drop for Scope` and abandons
        // the join that the `'scope -> 'static` transmute below depends on, freeing
        // the caller's frame while spawned bodies still borrow it (UAF from safe
        // code). `run_task` applies the same abort-on-fire-and-forget-panic policy
        // the worker loop already applies to the identical task type.
        // === KE16 A switch: ke16-a1 / ke16-a1-fifo / ke16-a3 === (step 1 has no
        // queue to drain under those arms; the sibling sweep below is the only
        // place a joiner meets a worker-spawned wave there)
        #[cfg(not(any(
            feature = "ke16-a1",
            feature = "ke16-a1-fifo",
            feature = "ke16-a3"
        )))]
        if let Some(idx) = own_local_inj
            && let Some(t) = drain_one(|| inner.injector_local[idx].steal_batch_and_pop(&scratch))
        {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 2. Global injector.
        if let Some(t) = drain_one(|| inner.injector_global.steal_batch_and_pop(&scratch)) {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 3. Sibling steal — any worker, any deque.
        let stolen = try_steal_any(inner, &scratch, own_local_inj);
        if let Some(t) = stolen {
            crate::worker::run_task(t);
            drain_scratch(&scratch);
            backoff.reset();
            continue;
        }

        // 4. Nothing to do. Either we exhaust backoff and park-with-
        //    timeout, or we snooze and loop.
        if backoff.is_completed() {
            // Wake one idle worker before parking: a sibling that just
            // pushed inner tasks into its own local injector may not
            // have raced through unpark_one_idle yet, but the work IS
            // visible — letting that worker grab it is also valid.
            //
            // This joiner does NOT mark its own idle bit (KE16 B0), so while it
            // sleeps here it is invisible to every other wave's wake decision:
            // no spawner can claim it, and it resumes only on its own scope's
            // next completer or on the timeout below. Making the parked joiner a
            // claimable lane is rule B1-P, which arrives with the B axis
            // (`KE16-DESIGN-B.md` §2.6).
            unpark_one_idle(inner);
            // 50 µs is the source constant; the real expiry is the OS timer
            // quantum (~15.3 ms unguarded, ~1.0 ms under `boyko_app`'s
            // `TimerResolutionGuard`), which is why this is a backstop and never
            // the wake path.
            std::thread::park_timeout(JOIN_BACKSTOP);
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}

/// Block (with work stealing) until `shared.pending` is zero — the B1 / B3 shape
/// (`KE16-DESIGN-B.md` §2.1).
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
/// The B0 arm's contract, unchanged: `shared` must point to a live `ScopeShared`
/// that stays valid for the whole call. Both arms reborrow `*shared` only
/// per-poll for one `Acquire` load and never form a reference that spans a
/// worker's `pending` write.
// === KE16 B switch: ke16-b1 / ke16-b3 ===
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
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
/// Three properties this has and the B0 `scratch` joiner does not
/// (`KE16-DESIGN-B.md` §2.2):
///
/// - **No sink.** Every batch lands in the deque `inner.stealers[wid]` publishes,
///   so the residue this joiner does not run is stealable by every sibling and by
///   an external joiner. A batch in `scratch` was reachable by nobody.
/// - **A re-check per task.** The scope is consulted between tasks, so the joiner
///   stops helping the instant its own wave completes; the B0 joiner ran a whole
///   residue of up to 33 first.
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
// === KE16 B switch: ke16-b1 / ke16-b3 === (B3 is this arm plus a PARKING
// external joiner, so the worker arm is shared by both features)
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
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
        // Miri-only cooperative yield, as in the B0 arm and `worker_main`: a
        // joiner that keeps finding work would otherwise starve its siblings
        // under Miri's cooperative scheduler. Native: compiles to nothing.
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

        // 1. Own deque, owner end — under `ke16-a1`'s LIFO end that is the
        //    NEWEST chunk of the wave being joined (the `ke16-a1-fifo` arm
        //    inverts the order, which is the A1-vs-A1-fifo question). The
        //    `&Worker` is consumed by `pop()` in THIS statement; the body runs in
        //    the next one (D5).
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
/// sink (B0's `scratch`) or in a registry that does not exist (B1(i)). One CAS
/// per task on the joiner, no residue, nothing hidden from the pool.
///
/// The design's §2.2 bullet says an external joiner "has no deque". That is true
/// of the route it was written for (the dispatcher, an unattached thread, a
/// worker of another pool) and FALSE of one caller: a worker of THIS pool inside
/// an `install` frame, whose deque is still registered in `inner.stealers` — it
/// is simply not this frame's lane, which is why the sweep below is allowed to
/// probe it like any other. Under `ke16-b3` that caller is also the reason this
/// body is not exclusive to `ke16-b1` (see [`join_external`]'s B3 arm).
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
// === KE16 B switch: ke16-b1 / ke16-b3 ===
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
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

/// The external joiner under B1: [`join_external_helping`], unconditionally.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
// === KE16 B switch: ke16-b1 ===
#[cfg(feature = "ke16-b1")]
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
/// The design's shape for this helper carries `try_steal_random`'s second,
/// `ke16-a2` probe of `injector_local[idx]` "so the code has one shape". It is
/// not written here: `ke16-b1` with `ke16-a2` is a `compile_error!` (`lib.rs`),
/// so that probe could never be compiled, let alone type-checked, and an
/// un-buildable mirror of a live scan is documentation wearing code's clothes.
/// Under A2 the sweep would need the probe — and under A2 there is no B1 to need
/// the sweep.
// === KE16 B switch: ke16-b1 / ke16-b3 ===
#[cfg(any(feature = "ke16-b1", feature = "ke16-b3"))]
fn steal_one_random(inner: &PoolInner, rng: &mut XorShift64Star) -> Option<Task> {
    let n = inner.stealers.len();
    if n == 0 {
        return None;
    }
    let start = (rng.next() as usize) % n;
    for k in 0..n {
        let idx = (start + k) % n;
        if let Some(t) = drain_one(|| inner.stealers[idx].steal()) {
            return Some(t);
        }
    }
    None
}

/// The external joiner under B3: never help — snooze, then park
/// (`KE16-DESIGN-B.md` §3) — with ONE caller excepted, because for it "never
/// help" is not a lost lane but a hang.
///
/// The snooze is KEPT and it is load-bearing on the frame path: a system's worker
/// pushes its ECS completion BEFORE the pool's `complete_task` decrement, so the
/// dispatcher can reach this loop a few nanoseconds before that decrement lands.
/// With the external arm's unpark-before-decrement order, a check(false) → park
/// (consumes a pending token, returns at once) → re-check (the decrement still
/// has not landed) → park sequence would have no wake left and would sleep the
/// whole OS-quantum backstop, once per frame that hits the window. The snooze
/// rounds outlast that window by orders of magnitude.
///
/// **The exception (round-4 review, blocking item 2).** The identity predicate
/// classifies a WORKER OF THIS POOL inside an `install` frame as external —
/// `install` rewrites the frame's worker id to the dispatcher sentinel, and that
/// classification is right for placement: the frame's pushes go to
/// `injector_global`, not to this thread's deque. It is wrong as a licence to
/// refuse: at `num_threads(1)`, or whenever every worker is inside such a frame,
/// the refusing thread is the ONLY one that could run what the frame spawned, so
/// §2.6's deadlock argument loses its case (a) *and* its completer, and the join
/// sleeps forever. B0 and B1 both help there. This arm therefore asks
/// [`tls::is_worker_thread_of`] first and helps exactly those callers; the routes
/// B3 is measured on — the bench dispatcher, the ECS frame dispatcher, the
/// fontbake bake thread, a worker of ANOTHER pool — are unattached to `inner`'s
/// worker set and still never help, so no measured cell moves.
///
/// # Safety
/// `shared` must point to a live `ScopeShared` for the whole call — the caller's
/// contract, upheld by `Scope::drop`.
// === KE16 B switch: ke16-b3 ===
#[cfg(feature = "ke16-b3")]
unsafe fn join_external(inner: &PoolInner, shared: *const ScopeShared) {
    if tls::is_worker_thread_of(inner) {
        // SAFETY: the caller's contract is handed through unchanged — `shared`
        //   is live for the whole of this call, hence for the whole of the
        //   callee's.
        return unsafe { join_external_helping(inner, shared) };
    }

    let backoff = Backoff::new();

    loop {
        #[cfg(miri)]
        std::thread::yield_now();

        // SAFETY: per the function contract `shared` is live for this whole call;
        //   a transient `Acquire` load that does not span a worker write.
        if unsafe { (*shared).is_drained() } {
            return;
        }

        if backoff.is_completed() {
            // This thread will not run the work, so the only way it moves is a
            // worker taking it: a sibling that pushed a wave may not have raced
            // through its own wake yet.
            unpark_one_idle(inner);
            std::thread::park_timeout(JOIN_BACKSTOP);
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}

/// Drain anything left in our local scratch deque (we may have stolen a
/// batch where only the first task is returned and the rest live in
/// `scratch`). We run them all inline.
// === KE16 B switch: ke16-b1 / ke16-b3 === no `scratch`, no serial residue.
#[cfg(not(any(feature = "ke16-b1", feature = "ke16-b3")))]
#[inline]
fn drain_scratch(scratch: &Worker<Task>) {
    while let Some(t) = scratch.pop() {
        // Same abort-guard reason as the steal sites above: a batch stolen from
        // `injector_global` can contain fire-and-forget tasks, and an unwind out
        // of here abandons the join (2026-07 audit finding).
        crate::worker::run_task(t);
    }
}

/// Try to steal a task from any sibling deque, ignoring nothing.
///
/// KE16 (deleted with the features): under `ke16-a2` — and `ke16-a5`, which
/// implies it — each index is probed TWICE, its registered deque and then its
/// `injector_local` slot, mirroring the second probe the worker loop's
/// `try_steal_random` gains: those arms leave a worker's own spawns in that
/// slot, so a joiner that scanned deques alone would walk past the very wave it
/// is waiting for. `own_local_inj` names the joiner's own slot, drained by step
/// 1 of the join loop and therefore skipped here; it is `None` for an external
/// joiner and under every arm that does not feed those queues.
// === KE16 A switch: ke16-a2 === Only the A2 probe below reads the sweep index
// and the joiner's own slot; without that feature both bindings are dead, and
// this narrow, feature-scoped allow is what keeps ONE body serving every arm
// instead of two copies of the sweep. It goes with the probe at removal time.
// === KE16 B switch: ke16-b1 / ke16-b3 === replaced by `worker::try_steal_random`
// (App-3: random start, self skipped by the lane's own `wid`, batch into the
// caller's REGISTERED deque).
#[cfg(not(any(feature = "ke16-b1", feature = "ke16-b3")))]
#[cfg_attr(not(feature = "ke16-a2"), allow(unused_variables))]
fn try_steal_any(
    inner: &PoolInner,
    scratch: &Worker<Task>,
    own_local_inj: Option<usize>,
) -> Option<Task> {
    for (idx, stealer) in inner.stealers.iter().enumerate() {
        if let Some(t) = drain_one(|| stealer.steal_batch_and_pop(scratch)) {
            return Some(t);
        }
        #[cfg(feature = "ke16-a2")]
        if own_local_inj != Some(idx)
            && let Some(t) = drain_one(|| inner.injector_local[idx].steal_batch_and_pop(scratch))
        {
            return Some(t);
        }
    }
    None
}

/// Resolve one `Steal` result, retrying only the `Retry` answer.
///
/// Every arm has a caller: the B0 joiner's own sweep, B1's steal-one external
/// arm, and — since the B3 external arm helps a worker of its own pool caught in
/// an `install` frame ([`join_external`]) — B3's too.
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
                // Alternate the two spawn shapes so the multi-drain property is
                // asserted over BOTH accounting paths (KE16 App-4): per task,
                // and one `pending` RMW for the whole wave. A batch wave that
                // registered or corrected wrongly would show up here as a wave
                // that never drains or as a scope freed mid-life.
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

    /// KE16 App-4, `k < n`: an iterator shorter than its promise is corrected,
    /// so the join ends.
    ///
    /// The failure mode is a HANG, not an assertion: `pending` would keep the
    /// `n - k` registrations that no task will ever decrement and `Scope::drop`
    /// would wait for completions that cannot come. The assertion below is
    /// therefore about the bodies; reaching it at all is the property.
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
    /// Sound in both arms: without `ke16-c-batch` every push wakes, so the
    /// property this asserts is one both spawn shapes have, and the test is not
    /// a way to tell them apart.
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

    /// KE16 App-4's accounting receipt, and the ONE test that tells the two arms
    /// apart.
    ///
    /// `KE16-DESIGN-W.md` §4.1 defines the candidate as "one `pending` RMW for
    /// the wave", taken before the first body is pulled. That is directly
    /// observable at the instant the iterator yields body 0: the batched arm has
    /// already run `register_tasks(n)` and no task exists yet to decrement it,
    /// so `pending` is exactly `n`; the fallback arm has registered nothing, so
    /// it is 0. Neither reading is a race — the wave's first push has not
    /// happened in either arm.
    ///
    /// Without it `ke16-c-batch` changes no test outcome anywhere in the crate:
    /// every other `spawn_batch` test asserts a property both arms have, so the
    /// feature's own mechanism would be certified by the witness string alone,
    /// and `KE16_EXPECT` only says the features were ENABLED
    /// (`KE16-DESIGN-MEASUREMENT.md` §5 item 3), never that the arm does
    /// anything.
    #[test]
    fn spawn_batch_registers_the_whole_wave_before_its_first_body() {
        const N: usize = 16;
        // Neither arm can produce this, so a wave whose iterator was never
        // pulled fails instead of matching a real reading.
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
        if crate::KE16_SPAWN_BATCH {
            assert_eq!(
                observed, N,
                "the batched arm must register the whole wave in one RMW before its first body \
                 is pulled"
            );
        } else {
            assert_eq!(
                observed, 0,
                "the per-task arm must have registered nothing before its first body is pulled"
            );
        }
    }

    /// KE16 App-4, the `k < n` correction MEASURED rather than inferred: the
    /// wave gives back exactly the registrations no body claimed.
    ///
    /// `spawn_batch_with_fewer_bodies_than_promised_drains` proves only that the
    /// join ends, and it proves it by hanging when it does not. This reads the
    /// count itself, from the owner thread, while the scope is still alive and
    /// after every spawned body has completed: a correction that gives back too
    /// few leaves `pending` above zero, and one that gives back too many wraps
    /// it to a huge value — both fail the bounded wait below with a message
    /// instead of stopping the suite.
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
                "`pending` never returned to zero: the wave gave back the wrong number of \
                 unspawned registrations"
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

    /// KE16 App-4 edge, `n == 1`: the minimal wave, whose first push IS its
    /// last, so the arm's "the remaining pushes are silent" loop runs zero times
    /// and the wave's single wake decision is the only one taken.
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
    /// This is the property the RAII `WaveRegistration` guard buys over the
    /// design's literal trailing `if k < n { … }` correction
    /// (`KE16-DESIGN-W.md` §4.1). The trailing form never executes on an unwind,
    /// so the `n - k` registrations no body will ever claim stay on `pending`,
    /// and `Scope::drop` — which runs during that same unwind — waits for
    /// completions that cannot come. The observable difference is termination,
    /// so what this test asserts is that it finishes at all, with the iterator's
    /// own message rather than a hang.
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

    /// KE16 App-4, `k > n` in RELEASE: the surplus is registered rather than
    /// left unaccounted, so every body still runs and the scope still drains.
    ///
    /// The debug arm of this contract is
    /// `spawn_batch_with_more_bodies_than_promised_is_a_debug_panic`. The
    /// release arm is different code — `WaveRegistration::charge_one`'s `else`
    /// branch — with a different property, so it needs its own test: leaving the
    /// surplus unregistered would let a still-running task drive `pending` to
    /// zero and the joiner free `ScopeShared` under it. Runs only under
    /// `cargo test --release`; in a debug build the code under test panics by
    /// design.
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
    /// two-dimensional domain in which the arm's own arithmetic changes shape
    /// three times: `n == 0` takes the `bodies.next()` early exit, `k == n`
    /// leaves `WaveRegistration::unpushed` at zero so `Drop` is a no-op, and
    /// every `k < n` has `Drop` give back a DIFFERENT count. An off-by-one in
    /// the give-back — `unpushed` decremented after the push instead of before,
    /// or `n - k - 1` given back — survives all four samples and fails
    /// somewhere in this grid.
    ///
    /// Exhaustive rather than randomised: the interesting region is small
    /// (`n <= 12`), the failure is a HANG rather than a wrong value, and a
    /// hanging case must be REPRODUCIBLE by name. 91 `(n, k)` pairs at each of
    /// two worker counts — 182 waves, the first of which has no siblings at all
    /// (`W == 1`, where the spawner is the only lane and the wave is drained
    /// entirely by `Scope::drop`'s helping join).
    ///
    /// The assertion is on the bodies; TERMINATING is the property. A
    /// give-back that returns too little leaves `pending` above zero and
    /// `Scope::drop` waits forever, so a defect here stops the suite at a named
    /// case instead of reporting a number.
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
