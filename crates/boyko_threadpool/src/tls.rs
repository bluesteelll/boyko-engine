//! Thread-local state for the pool.
//!
//! Phase 9 plan §2.7 (allocation discipline ALLOC1/ALLOC6) and §2.8 (event
//! lane TLS EVT1) both rely on per-thread flags maintained here. The ECS
//! crate consumes these helpers without depending on internal pool types.
//!
//! Phase 9.3b (decision E): the active-pool TLS pointer is a
//! `*const PoolInner` (the worker-shared state behind `Arc<PoolInner>`),
//! NOT a `*const ThreadPool` (the user-facing handle). A worker holds only
//! `Arc<PoolInner>` and must never resurrect the handle, so it deposits the
//! `PoolInner` pointer. `PoolInner` is opaque `pub`; consumers
//! (`par_iter`/`par_chunk`) only call its `num_threads()`/`scope()`.
//!
//! # The deque-lane invariants (`KE16-DESIGN-A.md` §1.7)
//!
//! The five keep the design's own names, `D1`–`D5`, and TWO other `D`-series
//! already run through this crate's comments: the diagnostics rung `D1` that
//! `worker_main` names at its `boyko_diag::lane::set_lane` call, and the
//! Phase 9 plan's `D2`–`D5` cited in the `loom_pool.rs` / `miri_scope.rs`
//! headers. These five are neither, so every citation of one carries
//! `KE16-DESIGN-A.md` beside it and the three series stay separable.
//!
//! `D1`, `D2`, `D4` and `D5` govern [`LANE_DEPOSIT`], the thread-local slot
//! through which a worker reaches its own registered deque. `D3` is the
//! reachability property that placement exists to serve. `D6`–`D8` are the
//! three the MERGE of the deque slot and the worker-id slot added (below).
//!
//! - **D1** [`LANE_DEPOSIT`]'s `pool` / `deque` fields hold
//!   `(Arc::as_ptr(&inner), &raw const deque)` for the whole of `worker_main`'s
//!   loop and `(null, null)` on every other thread and at every other time.
//!   Upheld by `WorkerDequeDeposit`, which writes and clears EXACTLY those two
//!   fields and which `worker_main` declares AFTER the `deque` parameter, so
//!   the clear happens before the pointee dies.
//! - **D2** worker `wid`'s deque in pool `P` is pushed to only by worker
//!   `wid`'s own thread — crossbeam's single-owner `Worker` contract, which
//!   here reduces to: only by `push_task` on a `Some` answer from
//!   [`worker_lane_for`], i.e. the calling thread IS that worker, the target
//!   pool IS `P`, and the thread is acting as that worker rather than inside
//!   an `install` frame.
//! - **D3** every task pushed to a worker deque is reachable by every sibling
//!   through `inner.stealers`, and by any joiner through the same stealers.
//!   An unreachable destination is what makes a worker-spawned wave run
//!   serially, and closing that is what this placement is for
//!   (`KE16-DESIGN-A.md`, defect A).
//! - **D4** `WorkerLane::deque` is the ONLY dereference of the deposited
//!   pointer; there is no other way to form a `&Worker` from the slot, so the
//!   `unsafe` obligation is discharged in one place.
//! - **D5** no `&Worker` minted from the slot is a field of a by-value
//!   argument, a parameter whose activation spans a task body, or a value used
//!   after a task body has run: each is consumed by one method call in its own
//!   statement (`let popped = lane.deque().pop();` — never an `if let`
//!   scrutinee, whose temporaries live through the THEN block in Rust 2024),
//!   or handed to a helper that runs no task body. This is what keeps a
//!   protected tag from spanning a nested spawn through the same slot.
//! - **D6** the two halves of [`LANE_DEPOSIT`] have DISJOINT writers.
//!   `{pool, deque}` is written only by [`WorkerDequeDeposit::new`] and its
//!   `drop`; `wid` only by [`set_current_worker_id`] and
//!   [`clear_current_worker_id`] (`ThreadPool::install` saves the old id with
//!   [`current_worker_id`] and restores it through the former; there is no
//!   separate swap entry point). No writer of
//!   one half writes the other, and that is what makes "an `install` frame on a
//!   worker of this pool" a STATE — tag present, `wid` = dispatcher sentinel —
//!   rather than a contradiction: the frame rewrites the id, the deposit
//!   survives it, and the worker regains its lane when the frame ends with no
//!   restore code anywhere. A whole-struct write from either guard breaks one
//!   of those two properties SILENTLY (a null tag written by the id guard makes
//!   every later spawn from that worker fall through to `injector_global` —
//!   locality lost, no test red).
//! - **D7** every read of the slot on the spawn path is exactly ONE
//!   `LANE_DEPOSIT.with`, through [`lane_deposit`]. [`worker_lane_for`] must
//!   never call [`current_worker_id`]: that restores the two-read shape the
//!   merge exists to remove and is invisible to every behavioural test: every
//!   answer stays correct and only the cost doubles. Gated by the source-shape
//!   rows of `tests/tls_lane_merge.rs`, which parse THIS file's
//!   `worker_lane_for` body and count its TLS touches, because no behavioural
//!   assertion can see the difference.
//! - **D8** the predicate rejects on the POOL TAG before the id.
//!   `worker_main` publishes the id before the deque, so between those two
//!   lines the slot reads `{null, null, wid}`; an id-first predicate would mint
//!   a lane over a null deque in that window.
//!
//! # Why ONE slot rather than two (`docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`)
//!
//! Under rustc >= 1.98.0 on `x86_64-pc-windows-gnu` EVERY `thread_local!` read
//! executes two lock-prefixed RMWs on one process-global cache line plus a
//! `kernel32!FlsSetValue` call, because std's `guard::windows::enable()` runs
//! on each access and the OS-key backend gives every key a destructor —
//! `const {}` initialisation does not help (that document, §3). The spawn path
//! asked the TLS two questions (which deque, and which worker am I), so it paid
//! that price twice per task. Merging the two slots makes it one; nothing else
//! about the predicate changes.

use core::cell::Cell;
use core::ptr;

use crossbeam_deque::Worker;

use crate::task::Task;
use crate::thread_pool::PoolInner;

/// Sentinel for the dispatcher thread (the calling thread inside
/// [`ThreadPool::install`]). Distinct from worker ids so that EVT1's lane
/// router can place dispatcher writes on an extra lane (`worker_count`).
///
/// [`ThreadPool::install`]: crate::ThreadPool::install
pub const WORKER_ID_DISPATCHER: u32 = u32::MAX - 1;

/// Sentinel for "no associated worker / dispatcher". Default state for
/// threads that never entered an install scope.
pub const WORKER_ID_UNATTACHED: u32 = u32::MAX;

/// The raw published lane state of the calling thread: everything
/// [`worker_lane_for`] needs, in ONE thread-local slot, so the spawn path pays
/// ONE `thread_local!` access instead of two (see the module header's cost
/// note and `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`).
///
/// `#[repr(C)]` fixes the field order so that the pool tag — the field whose
/// null-ness defines "this thread deposited no deque", and which the
/// predicate's fast reject reads — sits at offset 0 on every build, and so the
/// layout asserts below mean the same thing everywhere.
///
/// The two halves are INDEPENDENT (D6): `{pool, deque}` is owned by
/// [`WorkerDequeDeposit`] and `wid` by the id writers.
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct LaneDeposit {
    /// The pool whose registered deque this thread owns, or null. Compared by
    /// `ptr::eq` against the TARGET pool — never against [`ACTIVE_POOL`], which
    /// an `install` of another pool rewrites for the frame.
    pool: *const PoolInner,
    /// A raw borrow (`&raw const deque`) of `worker_main`'s by-value `deque`
    /// parameter, or null. Null iff `pool` is null (D1); dereferenced in
    /// exactly one place, [`WorkerLane::deque`] (D4).
    deque: *const Worker<Task>,
    /// The thread's ROLE: `0..worker_count-1` on a worker acting as itself,
    /// [`WORKER_ID_DISPATCHER`] inside an `install` frame (on ANY thread,
    /// worker or not), [`WORKER_ID_UNATTACHED`] otherwise.
    wid: u32,
    /// Explicit tail padding, so `size_of` is readable without re-deriving
    /// `repr(C)` rules and so `Cell::set` writes no indeterminate byte.
    _pad: u32,
}

impl LaneDeposit {
    /// The state of every thread that is not a worker and has entered no
    /// frame: no tag, no deque, no role. The `thread_local!` `const {}` seed.
    pub(crate) const DETACHED: Self = Self {
        pool: ptr::null(),
        deque: ptr::null(),
        wid: WORKER_ID_UNATTACHED,
        _pad: 0,
    };
}

// The slot lives in one thread's own TLS block: never shared, so no
// false-sharing question arises and no alignment is bought. Written against
// the pointer width rather than a hard 64-bit constant so the assert stays
// meaningful (and does not become one more wasm32 blocker) on a 32-bit build.
const _: () = assert!(size_of::<LaneDeposit>() == 2 * size_of::<*const ()>() + 8);
const _: () = assert!(align_of::<LaneDeposit>() == align_of::<*const ()>());
const _: () = assert!(core::mem::offset_of!(LaneDeposit, pool) == 0);

thread_local! {
    /// Active pool pointer for ambient `par_iter` dispatch. Set by
    /// [`ThreadPool::install`](crate::ThreadPool::install) entry / cleared on
    /// exit. Worker threads also have this set on `worker_main` entry. Null
    /// when no pool is attached. Points at the shared [`PoolInner`], never the
    /// handle (decision E).
    pub(crate) static ACTIVE_POOL: Cell<*const PoolInner> = const { Cell::new(ptr::null()) };

    /// THE lane slot: this thread's pool tag, its own Chase-Lev deque, and its
    /// worker id, in one cell. Replaces the separate `WORKER_DEQUE` and
    /// `CURRENT_WORKER_ID` slots so that [`worker_lane_for`] — the ONE identity
    /// predicate, and the thing every spawn asks — is ONE `thread_local!`
    /// access instead of two (D7; the module header prices the difference).
    ///
    /// - `wid` is `0..MAX_WORKERS-1` on a worker acting as itself,
    ///   [`WORKER_ID_DISPATCHER`] inside a `ThreadPool::install` frame, and
    ///   [`WORKER_ID_UNATTACHED`] on a thread attached to no pool.
    /// - `{pool, deque}` are the worker's own registered deque and the pool it
    ///   belongs to; null / null on every thread that is not a worker.
    ///   Deposited by `worker_main` after the active-pool deposit and cleared by
    ///   [`WorkerDequeDeposit::drop`] before `worker_main` returns.
    ///
    /// The deque pointer is minted with `&raw const deque` — a raw borrow of
    /// the place, NOT a reference — so no reference tag to the deque outlives
    /// any single method call (discipline D5, `KE16-DESIGN-A.md` §1.3).
    ///
    /// The pool tag is what [`worker_lane_for`] compares against its target,
    /// NOT [`ACTIVE_POOL`]: an `install` of pool B running on a pool-A worker
    /// swaps [`ACTIVE_POOL`] to B for that frame, and a B task must not land in
    /// A's deque. That is also why the two pointers are NOT in one struct.
    ///
    /// No field has a destructor, so the slot carries no TLS destructor of its
    /// own — the same status the two slots it replaces had.
    pub(crate) static LANE_DEPOSIT: Cell<LaneDeposit>
        = const { Cell::new(LaneDeposit::DETACHED) };

    /// Allocation discipline guard (ALLOC1), as a NESTING DEPTH (KE16 App-8).
    /// Incremented by the worker's run-system RAII guard, decremented on its
    /// drop; the ECS crate's context-restricted paths `debug_assert!` the
    /// boolean [`is_in_system_run`] (or its negation). It is a counter rather
    /// than a flag because a helping joiner may run a sibling conflict-free
    /// system INLINE inside a system body, so guards nest.
    pub(crate) static IN_SYSTEM_RUN: Cell<u32> = const { Cell::new(0) };
}

/// The ONE read of the lane slot (D7).
///
/// Private to this module so that no caller can take the deposit and then ask
/// a second question of the slot. `#[inline]` is load-bearing rather than
/// decorative: it must fold into both readers, or the merge trades two TLS
/// accesses for one access plus a call.
#[inline]
fn lane_deposit() -> LaneDeposit {
    LANE_DEPOSIT.with(|c| c.get())
}

/// The calling thread's lane in a pool: a registered worker id and the raw
/// pointer to that worker's own deque.
///
/// `Copy`, and it holds a RAW pointer, not a reference: a `WorkerLane` passed
/// by value into a function therefore carries no reference field for Miri to
/// retag and protect for the callee's duration. The only way to touch the
/// deque is [`WorkerLane::deque`], whose result is consumed by ONE method call
/// in its own statement (discipline D5, `KE16-DESIGN-A.md` §1.7).
#[derive(Clone, Copy)]
pub(crate) struct WorkerLane {
    /// The registered worker id of the calling thread in the pool this lane was
    /// minted for. Always `< inner.worker_count()`.
    ///
    /// The push arm addresses the lane by its DEQUE and never by index; the id
    /// is read by the worker joiner (`scope::join_on_worker`, which skips its
    /// own stealer by it) and by `PoolInner::joiner_wake_target`.
    pub(crate) wid: u32,
    deque: *const Worker<Task>,
}

impl WorkerLane {
    /// A shared reference to the lane's deque, to be consumed by ONE method
    /// call in its own statement (`let popped = lane.deque().pop();`).
    ///
    /// Callers MUST NOT bind the result to a local that lives across a task
    /// body, nor use it as an `if let` scrutinee whose THEN block runs a task
    /// body — in Rust 2024 such a temporary lives through that block (D5,
    /// `KE16-DESIGN-A.md` §1.3).
    // The production consumers are the push arm (`worker::push_on_lane_no_wake`)
    // and the worker joiner (`scope::join_on_worker`).
    #[inline]
    pub(crate) fn deque(&self) -> &Worker<Task> {
        // SAFETY (`KE16-DESIGN-A.md` §1.3, D5 — four facts, each of which the
        //   code below actually relies on):
        //
        //   1. LIVENESS. `self.deque` was deposited on THIS thread by
        //      `WorkerDequeDeposit::new` as `&raw const deque` — a raw borrow of
        //      `worker_main`'s by-value parameter, no reference created — and the
        //      guard is declared after that parameter, so its `drop` clears the
        //      slot before the parameter drops. `worker_lane_for` handed out this
        //      lane only after matching the deposited pool tag against the target
        //      pool, and every caller runs inside `worker_main`'s loop (the loop
        //      itself, a task body, or a join inside a task body), so the pointee
        //      is alive. The cell is thread-local and `Worker<T>` is `!Sync`, so
        //      no other thread can observe the pointer; the pointee is never
        //      moved after the deposit. `WorkerDequeDeposit` writes and clears
        //      the `{pool, deque}` fields of `LANE_DEPOSIT` and NO OTHERS (D6),
        //      so an `install` frame that runs between the deposit and this
        //      deref — which rewrites only `wid` — can neither dangle this
        //      pointer nor resurrect a cleared one; and for the frame's
        //      duration it makes the predicate answer `None`, so no lane is
        //      minted from a slot whose id is a sentinel.
        //   2. CONSUMPTION FORM. The `&Worker` minted here is consumed by ONE
        //      method call in its own statement (`lane.deque().push(task);`) OR
        //      passed as an argument to a helper that runs no task body
        //      (`P::deque(lane.deque())` in `worker::push_on_lane_no_wake`), and it
        //      is never used after the call that consumed it — so a body that
        //      later pushes through this same slot (a nested scope run inline)
        //      finds no live use of an older tag to conflict with.
        //   3. PROTECTORS. The only protectors ever attached to a tag on this
        //      deque are a `Worker` method's `&self` and the helpers' `local`
        //      argument, and no task body runs inside either — so no protected
        //      tag spans a foreign access. The reference is never a
        //      by-value-argument FIELD: `WorkerLane` carries a raw pointer
        //      precisely because Miri retags and protects reference fields of
        //      by-value arguments unconditionally, which would put a protected
        //      shared tag across every task a joiner runs.
        //   4. PROVENANCE under the owner's own writes. The only bytes of the
        //      pointee written after construction are its `buffer:
        //      Cell<Buffer<T>>` (crossbeam-deque 0.8.7 `deque.rs:198`), rewritten
        //      by `resize` through `Cell::replace` (`deque.rs:304`) from a
        //      `&self` that is a child of whatever tag called `push`. Those bytes
        //      are interior-mutable and Tree Borrows tracks interior mutability
        //      byte-precisely by default, so a shared tag's permission tolerates
        //      them; they are written only by this thread, only through children
        //      of this provenance. Thieves hold their own `Arc` clone of the heap
        //      `Inner` and touch that allocation, never this one.
        unsafe { &*self.deque }
    }
}

/// The calling thread's lane in `inner`, when it is a registered worker of
/// `inner` acting AS that worker.
///
/// `Some` iff the pool tag deposited on this thread is `inner` AND the
/// thread's role is a worker id (`< inner.worker_count()`). `None` on the
/// dispatcher, on an unattached thread, on a worker of another pool, and
/// inside an `install` frame on a worker of THIS pool — `install` rewrites the
/// slot's `wid` to [`WORKER_ID_DISPATCHER`], which is exactly how `push_task`
/// has always treated that frame. `scope` does not rewrite the id, so a
/// `par_iter` scope on a worker IS that worker's lane.
///
/// This is the ONE identity predicate (KE16 App-6): the push arm, the joiner's
/// dispatch and the count-gated completion target all ask it, so the three
/// cannot disagree — "a pool-A worker joining a pool-B scope drains B's
/// `injector_local[wid_A]`" stops being expressible.
///
/// Exactly ONE thread-local access (D7), and the pool tag is tested FIRST (D8).
#[inline]
pub(crate) fn worker_lane_for(inner: &PoolInner) -> Option<WorkerLane> {
    let d = lane_deposit();
    debug_assert!(
        d.pool.is_null() == d.deque.is_null(),
        "D1: the deque half of the lane slot is written and cleared as a PAIR"
    );
    // D8: the tag first. `inner` is a reference, so it is never null, and a
    // thread that deposited nothing rejects here rather than on its role.
    if !ptr::eq(d.pool, inner) {
        return None;
    }
    // Discharges BOTH sentinels: `worker_count` is clamped to `[1, 64]` at
    // build time and both sentinels are `>= u32::MAX - 1`.
    if (d.wid as usize) >= inner.worker_count() as usize {
        return None;
    }
    // The deque pointer is only carried: no dereference happens here.
    Some(WorkerLane {
        wid: d.wid,
        deque: d.deque,
    })
}

/// RAII deposit of the calling worker's deque into [`LANE_DEPOSIT`].
///
/// Constructed by `worker_main` immediately after the active-pool deposit and
/// declared AFTER the `deque` parameter it points at, so it drops — clearing
/// the slot — before the deque itself does.
///
/// Writes and clears the `{pool, deque}` half of the merged slot and NOTHING
/// else (D6): the `wid` half belongs to the id writers, and an `install` frame
/// that rewrites it must not lose this deposit.
pub(crate) struct WorkerDequeDeposit {
    /// Prevents construction outside `new`, and keeps the guard `!Send`: the
    /// deposit describes THIS thread and must not travel.
    _not_send: core::marker::PhantomData<*const ()>,
}

impl WorkerDequeDeposit {
    /// Publish `(pool, deque)` for this thread. `deque` must be a raw borrow
    /// (`&raw const deque`) of a place that outlives the guard.
    #[inline]
    pub(crate) fn new(pool: *const PoolInner, deque: *const Worker<Task>) -> Self {
        debug_assert!(
            !pool.is_null() && !deque.is_null(),
            "D1: the deposit publishes a real pair"
        );
        LANE_DEPOSIT.with(|c| {
            let mut d = c.get();
            d.pool = pool;
            d.deque = deque;
            c.set(d);
        });
        Self {
            _not_send: core::marker::PhantomData,
        }
    }
}

impl Drop for WorkerDequeDeposit {
    #[inline]
    fn drop(&mut self) {
        LANE_DEPOSIT.with(|c| {
            let mut d = c.get();
            d.pool = ptr::null();
            d.deque = ptr::null();
            c.set(d);
        });
    }
}

/// Returns the current worker id, or [`WORKER_ID_DISPATCHER`] /
/// [`WORKER_ID_UNATTACHED`] sentinel when not on a worker.
#[inline]
pub fn current_worker_id() -> u32 {
    lane_deposit().wid
}

/// Returns the lane index for `EventDispatcher::send_event`:
/// - The worker's own id when on a worker (`0..worker_count-1`).
/// - `worker_count` when on the dispatcher (an extra lane reserved at
///   `EventConfig::default_for(worker_count + 1)` time — see plan §2.8 EVT1).
/// - `0` when unattached (default lane for non-scheduler call sites).
///
/// `worker_count` MUST be passed by the caller because the TLS doesn't know
/// the pool's worker count (and we deliberately avoid a TLS pool pointer
/// dereference on the event hot path).
#[inline]
pub fn current_worker_id_or_dispatcher_lane(worker_count: u32) -> u32 {
    let id = current_worker_id();
    if id == WORKER_ID_DISPATCHER {
        worker_count
    } else if id == WORKER_ID_UNATTACHED {
        0
    } else {
        id
    }
}

/// Returns `true` when the current thread is executing inside a system body
/// (between [`InSystemRunGuard::enter`] and the guard's drop).
///
/// The predicate is depth-insensitive: with guards nested (a sibling system
/// run inline by a helping joiner, KE16 App-8) it stays `true` until the
/// outermost guard drops, which is what every caller means by "in a system
/// body".
#[inline]
pub fn is_in_system_run() -> bool {
    IN_SYSTEM_RUN.with(|c| c.get() > 0)
}

/// Set the worker id for the current thread. Called once on `worker_main`
/// entry, and by `ThreadPool::install` to enter and to leave a dispatcher
/// frame; not intended for user code.
///
/// Writes the `wid` field of [`LANE_DEPOSIT`] and NOTHING else (D6). A
/// whole-struct write here would null the `{pool, deque}` half, and because a
/// worker keeps its deposit across an `install` frame, that would silently
/// route every later spawn from that worker to `injector_global` — every test
/// still green and the locality this placement exists for gone.
#[inline]
pub(crate) fn set_current_worker_id(id: u32) {
    LANE_DEPOSIT.with(|c| {
        let mut d = c.get();
        d.wid = id;
        c.set(d);
    });
}

/// Clear the worker id on the current thread back to
/// [`WORKER_ID_UNATTACHED`], and the `boyko_diag` lane with it.
///
/// Reserved for use by code paths that must detach a thread from the pool
/// (e.g. test teardown); production `install` restores the previous value via
/// `set_current_worker_id` so it does not call this directly.
///
/// This clears the diagnostics lane while its sibling
/// [`set_current_worker_id`] deliberately does not write one. The asymmetry is
/// the honest shape: "detached" has exactly one lane answer
/// (`LANE_UNCLAIMED`), while "attached" has several — worker `k` on
/// `worker_main`, `LANE_DISPATCHER` inside `install` — which is why the
/// attaching sites each write their own lane beside the id rather than through
/// this module.
///
/// **Precondition**: the caller is detaching a thread whose lane the *pool*
/// wrote. A thread holding a spare from `boyko_diag::lane::claim_lane` must
/// call `release_lane` instead; clearing the TLS here would strand that spare
/// for the process, because `release_lane` reads the TLS to find the slot to
/// free. No production path can reach this — the function is `pub(crate)` with
/// zero callers today — but the precondition is what a future caller must
/// check.
#[inline]
#[allow(dead_code)]
pub(crate) fn clear_current_worker_id() {
    LANE_DEPOSIT.with(|c| {
        let mut d = c.get();
        d.wid = WORKER_ID_UNATTACHED;
        c.set(d);
    });
    boyko_diag::lane::set_lane(boyko_diag::lane::LANE_UNCLAIMED);
}

/// Replace the active-pool pointer; returns the previous value (so the
/// caller can restore it on exit — see `install`).
#[inline]
pub(crate) fn swap_active_pool(new: *const PoolInner) -> *const PoolInner {
    ACTIVE_POOL.with(|c| {
        let prev = c.get();
        c.set(new);
        prev
    })
}

/// Read the current active-pool pointer without modifying it.
#[inline]
pub(crate) fn active_pool_ptr() -> *const PoolInner {
    ACTIVE_POOL.with(|c| c.get())
}

/// Borrow the current active pool for the duration of `f`. Returns `None`
/// when no pool is attached to the current thread.
///
/// The borrow is bracketed by the closure call — the `&PoolInner` reference
/// MUST NOT escape `f` (the function signature prevents this at compile
/// time). Wave 6 `Query::par_iter` uses this to discover the ambient pool
/// without an explicit pool argument on every cursor.
#[inline]
pub fn try_with_active_pool<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&PoolInner) -> R,
{
    let p = active_pool_ptr();
    if p.is_null() {
        None
    } else {
        // SAFETY: `ACTIVE_POOL` is set by `ThreadPool::install`/`scope`
        //   immediately before `f(&scope)` runs and restored after
        //   `Scope::Drop` returns (which itself blocks until every spawned
        //   task completes). On a worker thread it is set at `worker_main`
        //   entry from the worker's own `Arc<PoolInner>` and lives until the
        //   worker returns (the handle joins every worker before dropping its
        //   `Arc<PoolInner>`, so the pointee outlives the deposit). Any thread
        //   that observes a non-null pointer is therefore inside a frame on
        //   the same thread (TLS is per-thread) whose `PoolInner` is live for
        //   the duration of this closure. `PoolInner` is reached only behind
        //   `Arc` and is never borrowed `&mut` (it is dropped via `Arc`'s
        //   internal `&mut` at refcount 0, after all workers join), so no
        //   `&mut` protector ever spans this shared `&PoolInner`. The closure
        //   cannot capture the borrow because of the `FnOnce(&PoolInner) -> R`
        //   signature; no aliasing escape.
        Some(f(unsafe { &*p }))
    }
}

/// RAII guard around a worker's system-body execution. Raises the
/// `IN_SYSTEM_RUN` depth on entry, lowers it on drop. The ECS scheduler wraps
/// `System::run_unsafe` in `let _g = InSystemRunGuard::enter();` so
/// context-restricted paths can `debug_assert!` whether they run inside a
/// system body (ALLOC6).
///
/// **Nesting is legal** (KE16 App-8): a joiner that helps while a scope is
/// open may run a sibling conflict-free system inline inside another system's
/// body — reachable today through the global-injector drain — so the guard
/// counts depth instead of asserting a flag was clear. SCH7's apply window is
/// unaffected: both systems are in `running` and the drain gate counts
/// completions, not lanes.
pub struct InSystemRunGuard {
    /// Prevents the guard from being constructible outside `enter`.
    _private: (),
}

impl InSystemRunGuard {
    /// Enter a system run, raising this thread's system-body depth by one.
    ///
    /// Panics in debug builds if the depth would exceed 64 — the conflict
    /// graph admits at most one inline sibling per open scope, so a deeper
    /// stack is a guard that was leaked rather than dropped.
    #[inline]
    pub fn enter() -> Self {
        IN_SYSTEM_RUN.with(|c| {
            let depth = c.get();
            debug_assert!(depth < 64, "InSystemRunGuard depth runaway");
            c.set(depth + 1);
        });
        Self { _private: () }
    }
}

impl Drop for InSystemRunGuard {
    #[inline]
    fn drop(&mut self) {
        IN_SYSTEM_RUN.with(|c| {
            let depth = c.get();
            debug_assert!(depth > 0, "InSystemRunGuard dropped at depth 0");
            c.set(depth.saturating_sub(1));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattached_thread_sees_sentinel() {
        assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    }

    #[test]
    fn dispatcher_lane_for_unattached_is_zero() {
        // The "0" mapping for unattached threads is documented in EVT1.
        assert_eq!(current_worker_id_or_dispatcher_lane(8), 0);
    }

    #[test]
    fn in_system_run_guard_round_trip() {
        assert!(!is_in_system_run());
        {
            let _g = InSystemRunGuard::enter();
            assert!(is_in_system_run());
        }
        assert!(!is_in_system_run());
    }

    /// App-6: inside an `install` frame the pool tag matches but the worker id
    /// is the dispatcher sentinel, so the calling thread is NOT a lane of the
    /// pool — the answer that keeps a sentinel out of every per-worker index.
    #[test]
    fn worker_lane_for_is_none_inside_an_install_frame() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build();
        pool.install(|_scope| {
            let is_lane = try_with_active_pool(|inner| worker_lane_for(inner).is_some());
            assert_eq!(
                is_lane,
                Some(false),
                "the install frame's dispatcher sentinel must not read as a worker lane"
            );
        });
    }

    /// App-6, the design's white-box row: with the pool tag deposited AND the
    /// worker id rewritten to the dispatcher sentinel — i.e. inside
    /// `pool.install` running ON a worker of that same pool — the predicate must
    /// answer `None`, so no per-worker array is ever indexed with a sentinel.
    ///
    /// The sibling `worker_lane_for_is_none_inside_an_install_frame` runs the
    /// frame on the test thread, where the tag half already fails.
    #[test]
    fn worker_lane_for_is_none_in_an_install_frame_on_a_worker_of_the_same_pool() {
        use core::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build();
        // u32::MAX = the probe never ran; otherwise bit 0 = the lane BEFORE the
        // install frame, bit 1 = the lane INSIDE it, bit 2 = the lane AFTER it.
        // Required: 0b101.
        //
        // Bit 2 costs one line and is the whole of D6's observable content.
        // Without it the test cannot tell "the frame rewrote `wid` and put it
        // back" from "the frame wrote the WHOLE struct and destroyed the deque
        // deposit": both answer `false` inside the frame, which is all bits 0-1
        // see. Under the whole-struct mutation the worker never regains its
        // lane, every later `Scope::spawn` from it falls through to
        // `injector_global`, and nothing else in this crate goes red.
        let answered = Arc::new(AtomicU32::new(u32::MAX));
        let answered_cl = Arc::clone(&answered);
        let pool_cl = Arc::clone(&pool);
        pool.spawn(move || {
            let outer = try_with_active_pool(|inner| worker_lane_for(inner).is_some());
            let framed =
                pool_cl.install(|_scope| try_with_active_pool(|i| worker_lane_for(i).is_some()));
            // After `InstallGuard::drop` has restored the id.
            let after = try_with_active_pool(|inner| worker_lane_for(inner).is_some());
            let bits = u32::from(outer == Some(true))
                | (u32::from(framed == Some(true)) << 1)
                | (u32::from(after == Some(true)) << 2);
            answered_cl.store(bits, Ordering::Release);
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while answered.load(Ordering::Acquire) == u32::MAX && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            answered.load(Ordering::Acquire),
            0b101,
            "expected a lane on the worker, none inside its own install frame, and the lane BACK \
             once the frame ended (u32::MAX = the probe never ran)"
        );
    }

    /// App-6: on a registered worker of the pool the predicate answers `Some`
    /// with that worker's own id.
    #[test]
    fn worker_lane_for_is_some_on_a_registered_worker() {
        use core::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build();
        // A fire-and-forget task never runs on the calling thread, so the body
        // is guaranteed to observe a worker's TLS.
        let seen = Arc::new(AtomicU32::new(u32::MAX));
        let mismatched = Arc::new(AtomicU32::new(0));
        let seen_cl = Arc::clone(&seen);
        let mismatched_cl = Arc::clone(&mismatched);
        pool.spawn(move || {
            let lane_wid = try_with_active_pool(|inner| worker_lane_for(inner).map(|l| l.wid));
            match lane_wid {
                Some(Some(wid)) if wid == current_worker_id() => {
                    seen_cl.store(wid, Ordering::Release);
                }
                _ => {
                    mismatched_cl.store(1, Ordering::Release);
                    seen_cl.store(u32::MAX - 2, Ordering::Release);
                }
            }
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while seen.load(Ordering::Acquire) == u32::MAX && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            mismatched.load(Ordering::Acquire),
            0,
            "a worker's own lane must be Some(its own id)"
        );
        assert!(
            seen.load(Ordering::Acquire) < 2,
            "the detached body did not run on a worker of the pool"
        );
    }

    /// App-6, the white-box half of `cross_pool_routing.rs`'s routing receipt: a
    /// worker of pool A asking for its lane in pool B is told `None`, whatever
    /// its own id is. This is the fact an integration test cannot see — a task
    /// drained from `B.injector_local[wid_A]` and a task stolen from a B
    /// worker's deque look the same from outside.
    #[test]
    fn worker_lane_for_is_none_on_a_worker_of_another_pool() {
        use core::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let pool_a = crate::ThreadPoolBuilder::new().num_threads(2).build();
        let pool_b = crate::ThreadPoolBuilder::new().num_threads(2).build();
        // B's shared-state address, read through the public accessor while B's
        // own install frame holds it in this thread's TLS.
        let b_addr = pool_b.install(|_scope| {
            crate::ThreadPool::current_pool()
                .expect("invariant: install deposits the active pool")
                .as_ptr() as usize
        });

        // u32::MAX = the body never ran; 0 = None (required); 1 = Some.
        let answered = Arc::new(AtomicU32::new(u32::MAX));
        let answered_cl = Arc::clone(&answered);
        let keep_b = Arc::clone(&pool_b);
        pool_a.spawn(move || {
            // Keeps B alive for the whole body (see the SAFETY note below).
            let _ = &keep_b;
            // SAFETY: `b_addr` came from B's own `ThreadPool::current_pool()`
            //   accessor, so it is the address of a live `PoolInner`; the
            //   `Arc<ThreadPool>` captured above keeps that allocation alive for
            //   the whole body. `PoolInner` is never borrowed `&mut` (it lives
            //   behind `Arc` and is dropped only at refcount 0, after every
            //   worker has joined), so no `&mut` protector can span this shared
            //   reference.
            let b: &PoolInner = unsafe { &*(b_addr as *const PoolInner) };
            answered_cl.store(u32::from(worker_lane_for(b).is_some()), Ordering::Release);
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while answered.load(Ordering::Acquire) == u32::MAX && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            answered.load(Ordering::Acquire),
            0,
            "a pool-A worker must have no lane in pool B (u32::MAX = the probe never ran)"
        );
    }

    /// A1 (`KE16-DESIGN.md` §8, the A1/A1-fifo row's last gate clause): the
    /// deposit publishes exactly the pair it was given, and the guard's `drop`
    /// clears it back to `(null, null)`.
    ///
    /// What it pins is the GUARD'S CONTRACT, on which clause 1 of
    /// [`WorkerLane::deque`]'s SAFETY block rests: `new` publishes the pair
    /// unaltered, `drop` restores `(null, null)`. Nothing else in the tree
    /// asserts on that pair, so a `Drop` impl that stopped clearing would go
    /// unnoticed here.
    ///
    /// It does NOT observe the production deposit — `worker_main`'s
    /// `let _deque_deposit = ...`, declared after the `deque` parameter so that
    /// local-before-parameter drop order clears the slot first: this test builds
    /// its own guard on the test thread. That line can be broken two ways, and
    /// they fail DIFFERENTLY:
    ///
    /// - A `worker_main` restructure that drops or returns the deque while the
    ///   loop still runs (or a `mem::forget` of the guard followed by any later
    ///   use of the slot on that thread) leaves a DANGLING pair:
    ///   `worker_lane_for` keeps handing out a lane whose `deque()` derefs freed
    ///   stack, reachable from safe code through `Scope::spawn` and silent in
    ///   release. That is the use-after-free clause 1 exists against, and its
    ///   gates are the two Miri shapes of `tests/miri_scope.rs` (which run the
    ///   real worker loop under Tree Borrows) plus invariant D1 on
    ///   `worker_main`'s shape — not an assertion in this file.
    /// - `let _ = WorkerDequeDeposit::new(..)` compiles, but `let _ = expr;`
    ///   drops the value at the end of that statement, so the slot is CLEARED
    ///   AT ONCE and `worker_lane_for` answers `None` for the rest of the worker
    ///   loop. There is no live pair, no deref and no use-after-free: every
    ///   worker spawn falls through to `push_global_no_wake` instead, so the
    ///   pool still runs every task and only the locality this placement exists
    ///   for is silently lost — a quiet regression, not a soundness defect. The
    ///   gate for it is
    ///   `worker::tests::a1_a_spawn_from_a_worker_body_lands_on_its_own_deque`,
    ///   which reads the production deposit from inside a worker body; measured
    ///   against that mutation, it goes red while THIS test stays green. (The
    ///   predicate rows below go red on it too — `worker_lane_for` reads this
    ///   very deposit — but only that row covers the PLACEMENT, i.e. that the
    ///   push arm reached the lane's deque rather than the global injector.)
    #[test]
    fn worker_deque_deposit_publishes_the_pair_and_clears_it_on_drop() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build();
        let pool_ptr = std::sync::Arc::as_ptr(&pool.inner);
        let deque: Worker<Task> = Worker::new_fifo();
        // The production mint: a raw borrow of the place, never a reference.
        let deque_ptr = &raw const deque;
        let pair = || LANE_DEPOSIT.with(|c| (c.get().pool, c.get().deque));
        let id = || LANE_DEPOSIT.with(|c| c.get().wid);

        // The `wid` half is this guard's NON-target, and W2's mutation is a
        // whole-struct write in `new` or in `drop`. Impersonating worker 7
        // first is what makes such a clobber observable: against the
        // `DETACHED` seed, a reset TO `DETACHED` reads as no change at all.
        set_current_worker_id(7);

        assert_eq!(
            pair(),
            (ptr::null(), ptr::null()),
            "a thread that is not a worker starts with an empty deposit"
        );
        {
            let _deposit = WorkerDequeDeposit::new(pool_ptr, deque_ptr);
            assert_eq!(
                pair(),
                (pool_ptr, deque_ptr),
                "the deposit must publish the pair it was given, unaltered"
            );
            assert_eq!(
                id(),
                7,
                "D6: `new` writes the deque half ONLY; a whole-struct write here resets the \
                 worker id and the thread stops being its own lane"
            );
        }
        assert_eq!(
            pair(),
            (ptr::null(), ptr::null()),
            "the guard's drop must clear the slot; a deposit that outlives its guard leaves \
             `worker_lane_for` handing out a lane over a dangling deque"
        );
        assert_eq!(
            id(),
            7,
            "D6: `drop` clears the deque half ONLY; a whole-struct clear would detach the \
             thread from its own id every time a deposit ends"
        );
        clear_current_worker_id();
        drop(deque);
    }

    /// The same fact pinned through the PREDICATE rather than through the
    /// raw cell — after the guard drops, `worker_lane_for` answers `None`, so no
    /// caller can obtain a `WorkerLane` (and hence no `deque()` deref) over a
    /// deque whose deposit has ended.
    ///
    /// The `Some` half first, because a test that only ever observes `None`
    /// would pass over a predicate that is broken in the other direction.
    #[test]
    fn worker_lane_for_answers_none_once_the_deposit_guard_has_dropped() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build();
        let pool_ptr = std::sync::Arc::as_ptr(&pool.inner);
        let deque: Worker<Task> = Worker::new_fifo();

        // The id half of the predicate: impersonate worker 0 of this pool. The
        // test thread pushes nothing while it holds the id, so no task can land
        // in a slot it does not own.
        set_current_worker_id(0);
        {
            let _deposit = WorkerDequeDeposit::new(pool_ptr, &raw const deque);
            assert!(
                worker_lane_for(&pool.inner).is_some(),
                "a deposited deque of this pool plus a worker id IS a lane"
            );
        }
        assert!(
            worker_lane_for(&pool.inner).is_none(),
            "the worker id is unchanged, so only the cleared deposit can deny the lane — and it \
             must, or `deque()` would deref a pointer whose pointee is about to drop"
        );
        clear_current_worker_id();
    }

    #[test]
    fn set_clear_worker_id_round_trip() {
        set_current_worker_id(3);
        assert_eq!(current_worker_id(), 3);
        // Lane for worker 3 with 8 workers => 3.
        assert_eq!(current_worker_id_or_dispatcher_lane(8), 3);
        clear_current_worker_id();
        assert_eq!(current_worker_id(), WORKER_ID_UNATTACHED);
    }

    /// App-8: the guard counts DEPTH, so a helping joiner that runs a sibling
    /// system inline inside a system body leaves the predicate true until the
    /// outer body ends.
    ///
    /// The boolean shape this replaced could not express that: the inner guard's
    /// drop cleared the flag and the rest of the OUTER system then ran with
    /// `is_in_system_run() == false`, which is the answer every consumer keys
    /// its behaviour off. Depth 2 is the reachable case (the inline run is one
    /// level); the loop past it pins that nothing saturates early.
    #[test]
    fn in_system_run_guard_stays_true_until_the_outermost_guard_drops() {
        assert!(
            !is_in_system_run(),
            "the test thread starts outside a system"
        );
        let outer = InSystemRunGuard::enter();
        {
            let inner = InSystemRunGuard::enter();
            assert!(is_in_system_run(), "depth 2 is inside a system run");
            drop(inner);
        }
        assert!(
            is_in_system_run(),
            "the inner guard's drop must not end the OUTER system's run"
        );
        drop(outer);
        assert!(!is_in_system_run(), "depth 0 is outside a system run");
    }

    /// App-8: the counter is a counter, not a saturating flag — eight nested
    /// entries need eight drops. One `enter` short of the top must still read
    /// true, which is what distinguishes a depth counter from a
    /// "set on first enter, clear on any drop" bug.
    #[test]
    fn in_system_run_guard_needs_one_drop_per_enter() {
        let mut guards = Vec::new();
        for _ in 0..8 {
            guards.push(InSystemRunGuard::enter());
        }
        for _ in 0..7 {
            guards.pop();
            assert!(
                is_in_system_run(),
                "a guard remains, so the thread is still inside a system run"
            );
        }
        guards.pop();
        assert!(!is_in_system_run(), "every guard dropped; the run is over");
    }
}
