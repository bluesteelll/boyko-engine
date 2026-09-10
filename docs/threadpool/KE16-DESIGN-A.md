# KE16 design — axis A: placement (defect A)

Part of the KE16 design; index and candidate table in `KE16-DESIGN.md`. Catalogue entries P0–P25 in
`KE16-VARIANTS-PLACEMENT.md` and `KE16-VARIANTS-ADDENDA*.md`. Line numbers are read at this checkout
(`b6c41237` + the untracked harness) and identify sites, not coordinates. Revision 4: the LIFO/FIFO
sub-choice is restated as a two-sided trade — the owner's local fence is bought with ONE thief-side
`SeqCst` CAS per stolen element (`deque.rs:1077-1142`), so on the one-spawner shape FIFO has the
lighter steal path and the pair is decided by measurement, symmetrically (§1.4; the critic's round-3
blocking item 1); D5 is worded around statements and protectors, with `let popped = …;` as the
required shape (§1.3, §1.7); the second Miri shape's receipt has a third red kind (§1.3); the
consumer-side fence citation names the empty-check path (§1.8); `claim_one_idle` has one signature
(§4.1); the B0-under-A1 rows price the LIFO source (§6). Revision 3: the lane carries a raw pointer
(§1.1, §1.3); the producer-side StoreLoad barrier that A1 loses is put back as production code
(§1.8); the B0 description under A1 is corrected (§0, §6); the Miri gate gains the
inline-nested-spawn shape and a red-classification rule (§1.3).

Built: **A1** (`ke16-a1`), **A1-fifo** (`ke16-a1-fifo`), **A2** (`ke16-a2`), **A3** (`ke16-a3`),
**A5** (`ke16-a5`). Argued away: A2′ (P18), A4 (P4), P6, and the rest of group P (§7).

## 0. What every A candidate has in common

- The defect site is one function: `push_task` at `crates/boyko_threadpool/src/worker.rs:365-376`.
  Every candidate changes its placement arm; nothing else about the spawn path (`Scope::spawn`,
  `scope.rs:288-365`: `register_task` → `Box` → transmute → `push_task`) changes on axis A.
- Every arm ends with the same call, `wake_after_push(inner, pre_len)` (`KE16-DESIGN-W.md` §2.2),
  where `pre_len` is the destination queue's length read immediately before the push. Under no
  `ke16-w-gate` that helper is `unpark_one_idle(inner)` unconditionally; the W switch lives inside
  the helper, never in an A arm. `unpark_one_idle` itself begins with the production StoreLoad
  barrier `publish_fence()` (§1.8; `KE16-DESIGN-W.md` §1) — so every wake decision, on every arm,
  is `fence(SeqCst)` → `idle.load(Acquire)` → claim. The fence is what makes a plain-store transport
  (A1's Chase-Lev push, the thief's residue store) safe against the SB litmus; on the injector arms
  it is redundant on x86 with the `lock`-prefixed ops the `Injector` already issues and is kept so
  the protocol has one shape.
- The route-(b) joiner (`join_workers_until_drained`, `scope.rs:440-518`) is NOT changed on axis A:
  every A candidate is measured with the joiner in its B0 form first (`KE16-DESIGN-B.md` §0).
  Under A1 that means the joiner's `try_steal_any` (`scope.rs:534-541`) steals from the joining
  worker's OWN deque via `inner.stealers[wid]` into `scratch` — and because `scratch` is a
  DIFFERENT deque, crossbeam does NOT degrade this to a pop (the degradation at `deque.rs:987-991`
  fires only when `Arc::ptr_eq(self.inner, dest.inner)`): it is a real batch of
  `min((len − 1)/2, 32) + 1 ≤ 33` of its own wave (`deque.rs:1018`), run serially from `scratch` —
  the full defect-B shape, promoted onto the production route on purpose. Under `ke16-a1` (LIFO
  source) that self-steal is crossbeam's per-element path — one `SeqCst` CAS + one `SeqCst` fence
  per stolen chunk (`deque.rs:1077-1142`) — followed by the reversal loop because `scratch` is
  FIFO (`:1145-1156`); under `ke16-a1-fifo` it is one CAS for the batch (`:1034-1071`). The A step
  measures both with that B0 cost included (§6).
- Anti-vacuity receipt on every measurement of a worker-route number: `outer_worker_id ∈ [0, W)`
  and `outer_same_pool == true` (`tests/ke16_nested_scope_occupancy.rs:257-333`); the bench gains the
  same receipt (`KE16-DESIGN-MEASUREMENT.md` §6).

## 1. A1 — spawn to the spawner's own registered deque (P1), LIFO owner end (L1)

### 1.1 Data and the ONE identity predicate

`crates/boyko_threadpool/src/tls.rs`, beside `ACTIVE_POOL` (`tls.rs:36`):

```rust
thread_local! {
    /// KE16 A1. The calling worker's own Chase-Lev deque (`worker_main`'s `deque` parameter,
    /// `worker.rs:21`), reachable from `push_task` on the SAME thread, and the pool it belongs to.
    /// Null / null on every thread that is not a worker. Deposited by `worker_main` after the
    /// active-pool deposit and cleared by `WorkerDequeDeposit::drop` before `worker_main` returns.
    ///
    /// The pointer is minted with `&raw const deque` — a raw borrow of the place, NOT a reference —
    /// so no reference tag to the deque outlives any single method call (§1.3).
    ///
    /// The pool tag is what the predicate compares against its target, NOT `ACTIVE_POOL`: an
    /// `install` of pool B running on a pool-A worker swaps `ACTIVE_POOL` to B for the frame
    /// (`thread_pool.rs:196`), and a B task must not land in A's deque.
    pub(crate) static WORKER_DEQUE: Cell<(*const PoolInner, *const Worker<TaskHandle>)>
        = const { Cell::new((ptr::null(), ptr::null())) };
}

/// The calling thread's lane in `inner`, when it is a registered worker of `inner` acting AS that
/// worker: `Some` iff the deposited deque belongs to `inner` AND `CURRENT_WORKER_ID` is a worker id
/// (`< inner.worker_count`). `None` on the dispatcher, on an unattached thread, on a worker of
/// another pool, and inside an `install` frame on a worker of THIS pool — `install` rewrites
/// `CURRENT_WORKER_ID` to `WORKER_ID_DISPATCHER` for its frame (`thread_pool.rs:205`), which is
/// exactly how today's `push_task` treats that frame (`worker.rs:370`: the `wid < len` half of its
/// test fails on the sentinel). `scope` does not rewrite the id (`thread_pool.rs:247-276`), so a
/// `par_iter` scope on a worker IS that worker's lane.
///
/// `Copy`, and it holds a RAW pointer, not a reference: a `WorkerLane` passed by value into a
/// function (B1's `join_on_worker`) therefore carries NO reference field for Miri to retag and
/// protect for the call's duration. The only way to touch the deque is `deque()`, which mints a
/// `&Worker` consumed by one method call in its own statement (D5, §1.3).
///
/// This is the ONLY way to reach the deque: the push arm, the joiner dispatch and the W-d′ wake
/// target all call `worker_lane_for`, so the three cannot disagree.
#[derive(Clone, Copy)]
pub(crate) struct WorkerLane {
    pub(crate) wid: u32,
    // === KE16 A switch: ke16-a1 / ke16-a1-fifo === (the field exists only when a deque TLS exists)
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    deque: *const Worker<TaskHandle>,
}

#[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
impl WorkerLane {
    /// A shared reference to the lane's deque, to be consumed by ONE method call in its own
    /// statement (`let popped = lane.deque().pop();`). Callers MUST NOT bind the result to a local
    /// that lives across `run_task`, nor use it as an `if let` scrutinee whose THEN block runs a
    /// task body (§1.3, invariant D5).
    #[inline]
    pub(crate) fn deque(&self) -> &Worker<TaskHandle> {
        // SAFETY: see §1.3 — deposited on THIS thread from `&raw const deque` inside
        //   `worker_main`, cleared before `deque` drops; every caller runs inside `worker_main`'s
        //   loop; the reference is bounded to the calling expression by the D5 discipline.
        unsafe { &*self.deque }
    }
}

#[inline]
pub(crate) fn worker_lane_for(inner: &PoolInner) -> Option<WorkerLane>
```

Under A1/A1-fifo the body is: read the pair; `if !ptr::eq(pool, inner) { return None }`; read
`current_worker_id()`; `if (wid as usize) >= inner.worker_count as usize { return None }`; return
`Some(WorkerLane { wid, deque })` — no dereference happens in the predicate. Under A2/A3/A5 (no
deque TLS) the body is `ptr::eq(tls::active_pool_ptr(), inner) && (wid as usize) <
inner.worker_count as usize` — the check today's `push_task` performs at `worker.rs:369-370`,
factored so the joiner (App-6, `KE16-DESIGN-APP.md` §5) and the W-d′ target use the same one. There
is no `worker_deque_for`; a caller cannot obtain the deque without the id check.

`worker_main` (`worker.rs:21-45`): after `tls::swap_active_pool(Arc::as_ptr(&inner))` at `:37`,

```rust
// KE16 A1: publish the deque to this thread's `push_task`. The guard clears the slot on every
// exit path; it is declared AFTER `deque` (a parameter) so it drops BEFORE `deque` does. The
// pointer is a raw borrow of the place — no reference to `deque` is created here.
let _deque_deposit = tls::WorkerDequeDeposit::new(Arc::as_ptr(&inner), &raw const deque);
```

`WorkerDequeDeposit` is a zero-field-plus-marker RAII type in `tls.rs`: `new` stores the pair,
`drop` stores `(null, null)`. The `Worker<TaskHandle>` is `!Sync`; the pointer never leaves the
thread (the TLS cell is per-thread and only the owning thread reads its own cell), so no `Sync` is
needed or claimed. `tls.rs` gains `use crossbeam_deque::Worker; use crate::thread_pool::TaskHandle;`
(the crate already depends on both).

### 1.2 The switch point in `push_task`

Replace the body at `worker.rs:365-376` with (the `#[cfg]`s mark the A switch; W-a's fence and
reordering are inside `unpark_one_idle`; the W gate is inside `wake_after_push`):

```rust
pub(crate) fn push_task(inner: &PoolInner, task: TaskHandle) {
    // === KE16 A switch: ke16-a1 / ke16-a1-fifo ===
    #[cfg(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))]
    {
        if let Some(lane) = tls::worker_lane_for(inner) {
            let pre_len = lane.deque().len();   // back (Relaxed, own line) + front (SeqCst load, the thieves' line — a load `push` makes anyway at deque.rs:398)
            lane.deque().push(task);            // Chase-Lev owner push: loads + Release fence + Relaxed store, no RMW (deque.rs:395-429)
            wake_after_push(inner, pre_len);    // -> [gate] -> unpark_one_idle: fence(SeqCst) -> idle.load -> claim (§1.8)
            return;
        }
        let pre_len = inner.injector_global.len();
        inner.injector_global.push(task);
        wake_after_push(inner, pre_len);
        return;
    }
    // (A2 / A3 / A5 / today's arms follow, each under its own cfg, each ending in wake_after_push)
}
```

Each `lane.deque()` is a separate expression-bounded reborrow (D5); nothing about `push_task`
holds a `&Worker` across another call. `pre_len` is read only when `ke16-w-gate` is on (the
helper's non-gated arm ignores it and the compiler removes the dead load); it is read before the
push in every arm so the gate's semantics are the same on a deque and on an injector.
`Injector::len()` is a loop of three `SeqCst` loads until the tail is stable (`deque.rs:1978-2010`)
— on the frame path that is one per SYSTEM push and immaterial; under A3 at 1 µs bodies it is part
of A3's cost, and A3 is the control.

Under A1 `injector_local` is **never pushed to**. For the tournament the field stays (empty,
constructed as today at `thread_pool.rs:601-606`) so the A2/A3/A5 rows compile against the same
struct; the polls of it are cfg'd out under A1 and A3 (three sites: `worker.rs:55-58`,
`worker.rs:201-203`, `scope.rs:477-485`) so the acquisition path does not pay an empty-`Injector`
probe (two Acquire loads + a `SeqCst` fence, `deque.rs:1795-1830`) per task for a queue that is
never fed. The removal step deletes the field, its construction, its polls and its doc comments.

Cost on the spawn path under A1 (busy pool, nobody idle), per spawn: `Box` (unchanged, W14 is not
on this axis) + `pending.fetch_add(1, AcqRel)` (one multi-writer RMW, `scope.rs:132`) + the
Chase-Lev push (no RMW; the `back` index line is single-writer; a `resize` once per wave when the
wave exceeds the buffer — `KE16-DESIGN-W.md` §0) + the wake decision: `fence(SeqCst)` (one LOCAL
full barrier — `mfence`, ~30–40 cycles, no cache-line transfer; §1.8) + `idle.load(Acquire)`.
Today: `pending` RMW + two single-writer RMWs on the injector line + the `wake_rotor` multi-writer
RMW + the idle load. Delta: −1 multi-writer RMW, −2 single-writer RMWs, +1 local fence (axis 38).
Under `ke16-w-gate` a push with `pre_len ≥ 2` skips the fence and the load both. That is the
SPAWNER's side only; the THIEF's side depends on the deque flavour (§1.4) and is the larger term on
the one-spawner shape. The full accounting is the table in `KE16-DESIGN-W.md` §0.

### 1.3 The Tree-Borrows obligation, and the SAFETY comment the deref must carry

Two facts about Miri fix the discipline. (i) Miri retags reference FIELDS of by-value arguments and
protects them for the callee's duration — the installed toolchain's Miri README
(`nightly-2026-08-20`, `share/doc/miri/README.md`) no longer even lists a `-Zmiri-retag-fields`
knob, so field retagging is unconditional — hence a `WorkerLane { deque: &Worker }` passed into
`join_on_worker` would carry a protected shared tag across every task the joiner runs, and a push
through the TLS pointer from inside such a task would be a foreign write under a live protector.
(ii) A protector is attached to a function-argument tag for the callee's activation only; a
reference VALUE held in a caller's local has no protector once the callee returns.

**The discipline (D5).** No `&Worker<TaskHandle>` reaching the TLS deque is ever a function
parameter whose activation spans a task body, a field of a by-value argument, or a value that is
USED after a task body has run. Concretely: the deposit stores `&raw const deque` (no reference
created); `WorkerLane.deque` is a raw pointer; `WorkerLane::deque()` mints a `&Worker` that is
consumed by ONE method call in ITS OWN STATEMENT — `let popped = lane.deque().pop();`,
`lane.deque().push(t);`, `let pre_len = lane.deque().len();` — or passed as an argument to a helper
that runs no task body (`pop_global_injector(inner, wid, lane.deque())`, `try_steal_random(inner,
wid, lane.deque(), rng)` — both return before `run_task`); the body then runs in a SEPARATE
statement (`if let Some(t) = popped { run_task(t); … }`). The statement form is required, not a
style: in Rust 2024 the temporaries of an `if let` scrutinee live through the THEN block (the
edition moved only the else-block drop point), so `if let Some(t) = lane.deque().pop() {
run_task(t) }` keeps the `&Worker` temporary syntactically alive while the body runs (the critic's
round-3 non-blocking item 1). That form is NOT UB — a reference VALUE in a temporary carries no
protector, and it is never used after `pop()` returns — but D5's guarantee must rest on what the
model actually checks, which is protectors and uses, so the SAFETY text below argues from "no
protector spans a body; no use after the call", and the code takes the statement shape so the
argument and the syntax say the same thing. The worker loop already has this shape today
(`worker.rs:55-77`: `&deque` per call, `run_task` in a separate statement). The protectors that DO
exist — the `&self` of each `Worker` method for that method's duration, and the `local: &Worker`
argument of the two helpers for their duration — span no code that could access the deque through
another tag: the owning thread is inside the method, no task body runs inside it, and thieves never
touch the `Worker` allocation at all (a `Stealer` holds its own `Arc` clone to the heap `Inner`,
`deque.rs:139-143`; everything a thief writes is on the heap).

**Why the raw deposit's provenance survives the owner's own writes.** The only bytes of the
`Worker<TaskHandle>` allocation that are ever WRITTEN after construction are the
`buffer: Cell<Buffer<T>>` field (`deque.rs:59`), rewritten by `resize` through `Cell::replace`
(`deque.rs:304`) from a `&self` that is a child of whatever tag called `push`. Tree Borrows tracks
interior-mutable bytes at byte level by default (the README's
`-Zmiri-tree-borrows-no-precise-interior-mut` flag exists to turn that OFF), and a shared tag's
permission on `UnsafeCell` bytes tolerates foreign writes — it must, or `let a = &cell; let b =
&cell; b.set(1); a.get()` would be UB. The `inner: Arc<…>` and `flavor` bytes are never written. So
every tag ever minted on the deque — the deposit's raw provenance, its expression-bounded
children, `worker_main`'s own transient `&deque` reborrows (siblings of those children) — keeps
its permission through every push, pop and resize. The pointee is never moved (a by-value
parameter; the deposit happens after the last move). The pointer is dead before the pointee drops
(guard order, §1.1).

Every deref site (there is exactly one: `WorkerLane::deque`) carries:

```rust
// SAFETY: `self.deque` was deposited by `WorkerDequeDeposit::new` on THIS thread as
//   `&raw const deque` (a raw borrow of `worker_main`'s by-value parameter, no reference
//   created) and is cleared by the guard's `drop` before that parameter drops. Every caller runs
//   inside `worker_main`'s loop (a task body, a join inside a task body, or the loop itself), so
//   the pointee is alive. The `&Worker` minted here is consumed by one method call in its own
//   statement, or passed to a helper that runs no task body (D5): the only protectors ever
//   attached to a tag on this deque are a `Worker` method's `&self` and the helpers' `local`
//   argument, and no task body runs inside either, so no protected tag spans a foreign access;
//   and the reference is never used after the call that consumed it, so a body that later pushes
//   through this same slot (a nested scope run inline) finds no live use to conflict with. It is
//   never a by-value-argument field (Miri retags and protects those unconditionally). The only bytes
//   of the pointee written after construction are its `Cell<Buffer<T>>` (interior-mutable,
//   byte-precise under Tree Borrows), written only by this thread through children of this
//   provenance; thieves touch the heap `Inner` behind the `Arc`, never this allocation. The cell
//   is thread-local, so no other thread can observe the pointer; `Worker<T>` is `!Sync` and is
//   only ever used from its owning thread.
```

**Miri gates (index §8), both under the FULL flag string** (`KE16-DESIGN-MEASUREMENT.md` §8):

1. `tests/miri_scope.rs::nested_scope_from_worker_is_stolen_by_sibling` — the H4 forced-interleave
   pattern (`miri_scope.rs:16-31`) one level down. The test thread does `pool.spawn(outer)`
   (fire-and-forget, so the outer body MUST run on a worker — the U1 hazard) and then spin-waits on
   an `AtomicBool` with `spin_until` (NO scope join on the test thread, so the only join in the
   test is the worker's — what makes the same test the W-d′ route-(b) liveness gate,
   `KE16-DESIGN-W.md` §3.7); the outer body opens `pool.scope`, spawns A and B with the flag
   handshake so the joiner alone cannot finish them, records `current_worker_id()` in each body,
   joins, sets the flag. Assert: one body ran on a worker id different from the spawner's. This is
   the test that exercises a sibling's `Stealer::steal_batch_and_pop` against a deque the owner
   reaches through the TLS pointer.
2. `tests/miri_scope.rs::nested_scope_inline_body_spawns_through_tls_deque_under_live_join` — the
   protector-bearing shape the critic asked for. Same outer setup (`pool.spawn`, `AtomicBool`
   wait, W = 2). The outer body opens `pool.scope` and spawns TWO bodies; EACH body opens its own
   `pool.scope` and spawns `2 × 64 + 1 = 129` trivial tasks (crossbeam `MIN_CAP = 64`,
   `deque.rs:16`, so the pushes force at least one `resize` — the one write to the `Worker`
   allocation's bytes — while the OUTER join is live on this thread) and records
   `current_worker_id()`. Assert: every counter reaches 129 and Miri reports no UB (the
   obligation); and, as the RECEIPT that the protector-bearing shape was exercised, at least one
   nested-spawning body reports `wid == outer_wid`. **The receipt is deterministic only under
   B1 + LIFO** (`ke16-a1,ke16-b1`): the outer joiner's first act is to pop its OWN deque from the
   `back`, i.e. the LAST-pushed body, while a sibling's batch from a 2-element deque takes exactly
   one — the FRONT (`(2 − 1)/2 = 0`, plus the popped one; `deque.rs:1018`) — so the back body is
   the joiner's in every interleaving. Under B1 + FIFO both ends are the front and the sibling can
   take both bodies if it finishes the first's 129-task wave before the joiner pops; under B0 the
   joiner's fixed `0..n` sweep (`scope.rs:534-541`) can visit the SIBLING's deque first, batch-steal
   that sibling's NESTED tasks into `scratch` and run those inline while the sibling finishes and
   takes the second outer body — a miss with no UB and no timeout (the critic's round-3 non-blocking
   item 2). So the test repeats the whole shape (fresh pool each time) up to 4 times per seed and
   passes on the first observed receipt; if no attempt observes it, it panics with the literal
   message `receipt not observed` — the THIRD red kind, classified in `KE16-DESIGN-MEASUREMENT.md`
   §5 item 14: not a defect; re-run with `-Zmiri-many-seeds=0..64`, and the gate is "zero UB across
   every seed AND the receipt observed in at least one seed"; the tester records in how many seeds
   it was observed. Run under `ke16-a1`/`ke16-a1-fifo` with B0 AND with B1.

**Classifying a red on these gates — before any fallback is taken.** A Tree-Borrows violation is
an `error: Undefined Behavior: …` report naming a tag and an access; it is never a timeout. A
`spin_until timed out` panic is a LIVENESS defect in the wake protocol — first suspect the
producer-side barrier (§1.8): confirm `publish_fence()` is the first statement of
`unpark_one_idle` and that the residue cascade reaches it, then run loom M2 with the production
fence and its calibration copy (`KE16-DESIGN-W.md` §1.3). A liveness red is NEVER grounds to
discard A1 for A2. Only a UB report that survives the code-reviewer's re-derivation of D5 against
the actual code is; then A2 is the fallback (index §3). Miri's weak-memory emulation is itself
incomplete (README: "there are legal behaviors that Miri will never produce … use specialized tools
such as loom"), so loom M2 — not Miri — is the formal gate for the fence.

### 1.4 The end discipline: `ke16-a1` = LIFO, `ke16-a1-fifo` = FIFO

`thread_pool.rs:595`: `Worker::new_fifo()` → `Worker::new_lifo()` under `ke16-a1`; unchanged under
`ke16-a1-fifo`. One constructor. Under LIFO the owner pops its NEWEST entry and a thief takes the
OLDEST (`Worker::pop` on a LIFO deque takes the back; `Stealer::steal*` always takes the front).

**The trade, both sides (revision 4 — the critic's round-3 blocking item 1).** Revision 3 priced
only the owner's side ("the LIFO pop trades a contended RMW for a local fence"); that was true and
incomplete, because crossbeam's THIEF path differs by source flavour in the opposite direction:

- **Owner's pop.** FIFO: `front.fetch_add(1, SeqCst)` (`deque.rs:463`) — one `lock`-prefixed RMW
  per executed own task on the `front` line, which every successful thief's CAS also writes. LIFO:
  `back.store` + `fence(SeqCst)` + `front.load(Relaxed)` (`:488-494`), a `front` CAS only on the
  LAST element (`:500-517`) — one local `mfence` plus one shared READ of `front` per own task.
- **Thief's batch steal** (`steal_batch_and_pop`, the call at `worker.rs:252` and `scope.rs:536`).
  FIFO source: copy the batch, then ONE `front.compare_exchange(SeqCst)` for the whole batch
  (`deque.rs:1034-1071`, CAS at `:1061`). LIFO source: a CAS for the first element (`:1082`), then
  PER ADDITIONAL ELEMENT a `fence(SeqCst)` (`:1101`) + a `back` load + a buffer check + a
  `front.compare_exchange(SeqCst)` (`:1121`), stopping early on a failed CAS or an emptied queue
  (`:1077-1142`) — up to 33 contended CASes and 32 fences per batch, one per stolen element, and a
  reversal loop when the destination is FIFO (`:1145-1156`: B0's `scratch`). This is why every
  LIFO runtime (rayon) steals ONE task per CAS and every batch-stealing runtime (Tokio, Go) is
  FIFO: under LIFO the owner can shrink the deque from the other end between any two elements, so
  each element needs its own CAS; a batch limit does not change the per-element count
  (`KE16-DESIGN-W.md` §0).

Per task of a wave, with `s` the stolen fraction: LIFO ≈ `s` shared RMWs on `front`, FIFO ≈
`(1 − s) + s/33`. On this pass's shapes — ONE spawner and W−1 thieves, `s ≈ 0.94` at W=16 — that
is ≈ 0.94 vs ≈ 0.09 per task: **by RMW count FIFO has the lighter steal path on the consumer
shape, by an order of magnitude.** Revision 3's "A1 is the best of the five at 1 µs × 64W" and its
rule-4 ordering `a1 < a1f` are retracted; the index's predictions are rewritten accordingly.

What the count does not settle, and the measurement must — the reasons A1 (LIFO) is still built
and could still win:

- **Contention mode.** Under FIFO the owner's `fetch_add` and the thieves' batch CAS meet on the
  SAME end: a batch CAS fails whenever the owner popped between the copy and the CAS, and the
  thief re-copies and retries with no backoff (`drain_one`, `worker.rs:262-279`). With 15 thieves
  on one victim at 1 µs bodies this can turn "one CAS per batch" into a CAS storm; LIFO's ends meet
  only on the last element. Whether the storm materialises at W=16 is exactly what the
  1–10 µs × 64W worker cells measure.
- **Locality** (L1): the spawner's own newest chunk is the one whose descriptor and first rows are
  hottest in its cache; small on a flat `par_iter` wave, real on nested physics scopes (a color's
  chunks spawned by the worker that just solved the previous color's chunk on the same bodies).
  FJP ships FIFO-local only for "forked tasks that are never joined" (`asyncMode` `[D]`); ours are
  joined — that is the documented exclusion.
- **The joiner's priority under B1**: with LIFO the joining worker pops its OWN wave first; with
  FIFO it pops the oldest entry, which on the frame path is a sibling system batch-stolen from
  `injector_global` earlier — running a whole other system before its own chunks. Not a soundness
  matter (App-8) but a latency one for the scope, visible on physics, not on the grid.

**Decision rule.** The pair is decided SYMMETRICALLY on the 1–10 µs × 64W worker-route cells
(where the steal path dominates) and on physics `in_scheduled_system`, per
`KE16-DESIGN-MEASUREMENT.md` §7 Step A rules 3–4: whichever of the two regresses the other beyond
2× the band on those cells loses; a physics difference beyond the band decides over the grid; a
tie everywhere is broken toward FIFO (the smaller diff: today's constructor, no reversal path
reachable). Nothing about this decision is count-derived.

Order dependence audit: no test or production caller asserts an execution order across tasks of one
deque (`grep -rn -i fifo crates/boyko_threadpool/tests` hits only the harness header). The ECS
executor's systems within a round are conflict-free by construction (`schedule.rs:996-1011`), so
LIFO popping of a stolen batch of systems is legal.

### 1.5 The worker loop under A1

`worker.rs:46-126`: stage 1 (`pop_local_injector`) cfg'd out; stages 2–4 unchanged (own deque →
global injector → random self-skipped sibling steal); `pop_any` (`:195-211`) likewise loses stage 1.
`try_steal_random` (`:233-257`) is unchanged and is what makes A1 reachable: `inner.stealers[wid]`
(`thread_pool.rs:596`) is the stealer of the very deque `push_task` now feeds. Under `ke16-w-gate`
the thief-residue cascade is added inside `pop_global_injector`/`try_steal_random`
(`KE16-DESIGN-W.md` §2.2) — shared by the loop and the B1 joiner — and it EXCLUDES the caller's own
idle bit (`unpark_one_idle_excluding(inner, 1 << wid)`): the helpers are also reached from the
post-`mark_idle` re-poll (`worker.rs:104`) BEFORE `unmark_idle` (`:105`), where the caller's bit is
still set and an unmasked claim could pick the caller itself (a stale token on its own parker, the
chain broken at hop one — the critic's non-blocking item 2).

### 1.6 What `scope.rs:17-21` becomes

The paragraph "we do NOT drain the calling worker's own Chase-Lev deque … not accessible here" is
false under A1. It is replaced by the B-axis description (`KE16-DESIGN-B.md` §2.4) — under B0 the
paragraph reads "the joining worker reaches its own deque only through its registered stealer, as a
batch into `scratch`", under B1 "the joining worker pops its own deque directly".

### 1.7 Invariants added by A1 (state them in `tls.rs`'s module doc)

- **D1** `WORKER_DEQUE.0 == Arc::as_ptr(inner)` and `WORKER_DEQUE.1 == &raw const deque` for the
  whole of `worker_main`'s loop, `(null, null)` otherwise.
- **D2** the deque of worker `wid` of pool `P` is pushed to only by worker `wid`'s own thread
  (crossbeam's `Worker` contract), which under A1 means: only by `push_task` when
  `worker_lane_for(P)` is `Some` — i.e. the calling thread IS that worker, the target IS `P`, and
  the thread is acting as that worker (not inside an `install` frame).
- **D3** every task pushed to a worker deque is reachable by every sibling through
  `inner.stealers` (closure of axis 11); by every external joiner through the same stealers.
- **D4** `WorkerLane::deque` is the only deref of `WORKER_DEQUE.1`; there is no other way to form
  a `&Worker` from the slot.
- **D5** no `&Worker` minted from the slot is a by-value-argument field, a parameter of a function
  whose activation spans a task body, or a value used after a task body has run; each is consumed
  by one method call in its own statement (`let popped = lane.deque().pop();` — never an `if let`
  scrutinee whose temporaries outlive the body, §1.3); helpers that take `local: &Worker`
  (`pop_global_injector`, `try_steal_random`, `pop_any`) run no task body.

### 1.8 The producer-side StoreLoad barrier (the critic's blocking item 1), and where it lives

**The race.** The wake protocol is the store-buffer (SB) litmus: the producer does (publish work;
read `idle`), the parking worker does (`fetch_or` its bit; re-poll the queues). Today the
producer's publish is `Injector::push` — a `SeqCst` CAS on the tail (`deque.rs:1409-1414`) and a
`slot.state.fetch_or(WRITE, Release)` (`:1431`), both `lock`-prefixed — followed by
`wake_rotor.fetch_add` (`worker.rs:324`, `lock xadd`) before the `idle.load` at `:326`: three full
barriers on x86 between the publish and the load, none of them placed there on purpose. Under A1 the
publish is `Worker::push` = `fence(Release)` (a no-op instruction on x86) + `back.store(Relaxed)`
(a plain `MOV`, `deque.rs:419-428`), and W-a moves the rotor RMW behind the mask load. The A1 spawn
path would then be: `lock xadd pending` (BEFORE the push, `scope.rs:132`); `MOV back`; `MOV idle`
— and x86-TSO lets the load pass the store. Interleaving: worker P `lock or idle` → (fence in the
steal path) → loads X's `back` → stale, parks untimed (`worker.rs:117`); X's `idle` load executed
before P's `lock or` landed → 0 → no wake. The wave is invisible to every parked sibling; X's join
pops its own deque serially. Not a liveness bug (the owner never parks with a non-empty deque and
always pops it — rayon documents exactly this fallback for its own un-fenced internal pushes,
rayon-core 1.13.0 `sleep/mod.rs:225-233`: "under certain race conditions, the function may fail to
wake any new threads; in that case the existing thread should eventually pop the job") but a
THROUGHPUT defect of the exact shape KE16 exists to remove: one lost race = one serial physics
color. The same shape recurs at the thief-residue cascade, whose residue is published by
`fence(Release)` + `dest.inner.back.store(Relaxed)` (`deque.rs:1160-1170`) and then followed by
`len()`/`is_empty()` (`SeqCst` LOADS of `front`, `deque.rs:359-382` — a `MOV`, no barrier) and the
`idle` load.

**The fix — one site.** `unpark_one_idle` begins with the barrier:

```rust
/// KE16. The producer-side StoreLoad barrier of the wake protocol (the SB litmus against a parking
/// worker's `fetch_or` + steal-path fence + re-poll). Required whenever the work was published by
/// a plain store — a Chase-Lev `Worker::push` (`deque.rs:419-428`) or a batch steal's residue
/// store (`deque.rs:1160-1170`); redundant on x86 after an `Injector::push` (its `lock`-prefixed
/// CAS and `fetch_or`), kept unconditional so the protocol has ONE shape and the loom M2 model
/// drives production code for it. On x86-64 this is one `mfence` (~30-40 cycles, no cache-line
/// transfer); it replaces the `wake_rotor` `lock xadd` that supplied the barrier by accident.
#[inline]
pub(crate) fn publish_fence() {
    crate::sync::fence(Ordering::SeqCst);
}

pub(crate) fn unpark_one_idle(inner: &PoolInner) -> bool { unpark_one_idle_excluding(inner, 0) }

pub(crate) fn unpark_one_idle_excluding(inner: &PoolInner, exclude: u64) -> bool {
    publish_fence();
    let mut mask = inner.idle.load(Ordering::Acquire) & !exclude;
    if mask == 0 { return false; }                 // busy pool: one fence + one load, no RMW (W-a)
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    loop { /* today's rotate / lowest-bit / CAS(AcqRel, Acquire) claim; on failure re-load & !exclude */ }
}
```

`crate::sync` gains `fence` (`loom::sync::atomic::fence` under `cfg(loom)`, `core::sync::atomic::
fence` otherwise); the `sync.rs:39` note "no production code uses `fence`" becomes false and is
corrected (index §7). Every wake decision — the push arms via `wake_after_push`, the residue
cascade, the joiner's pre-park wake, A5's fallback arm, W-f's `wake_up_to` — goes through this one
prologue. Under `ke16-w-gate` a push with `pre_len ≥ 2` returns before it (no fence, no load), which
the weak invariant W-b-1 permits (`KE16-DESIGN-W.md` §2.3: only the 0→1 and 1→2 pushes carry the
wake decision, and those DO fence). A5's claim-then-place-then-unpark arm does not rely on the SB
litmus (the claimed worker is unparked explicitly after the push; program order + the parker's
`Release` swap / `Acquire` park order the two) and its fallback arm goes through the prologue.

**The consumer side keeps its barrier from the transport** — that is not what A1 changes: the
parking worker's re-poll after `fetch_or` reaches `Injector::steal_batch_and_pop`, whose explicit
`fence(SeqCst)` is at `deque.rs:1821`, INSIDE `if new_head & HAS_NEXT == 0` — executed only on the
path that can conclude "empty" (head and tail in the same block), which is exactly the path the SB
litmus needs (the block-boundary spin loop at `:1801-1813` is not it; the critic's round-3
non-blocking item 4) — and `Stealer::steal_batch_and_pop` (`epoch::pin` at `:1006`, which on x86
is a `compare_exchange(SeqCst)` — `lock cmpxchg` — and elsewhere a `fence(SeqCst)`, crossbeam-epoch
0.9.20 `internal.rs:394-440`; plus the explicit `fence(SeqCst)` at `:1003` when already pinned)
before it loads any sibling's `back` (`:1011`). The owner's own empty `pop` (`deque.rs:446-456`,
Relaxed loads, no fence when empty) is not part of any cross-thread SB pair: only the owner pushes
to it.

**Precedent, read at this checkout.** rayon-core 1.13.0: `WorkerThread::push` = `is_empty` →
`worker.push` → `sleep.new_internal_jobs` (`registry.rs:728-732`); `new_injected_jobs` puts an
explicit `fence(SeqCst)` before the counter read ("needed to guarantee that threads as they are
about to fall asleep, observe any new jobs that may have been injected", `sleep/mod.rs:214-220`);
`new_internal_jobs` does NOT and documents the missed-wake fallback quoted above (`:225-237`). This
design fences BOTH: the critic's remark that rayon's shape is "a SeqCst RMW on its jobs counter"
is half right — `new_jobs`' `increment_jobs_event_counter_if(is_sleepy)` is a `SeqCst` LOAD with a
CAS only when a sleeper announced itself (`:242-250`) — and the un-fenced internal case is precisely
the case whose fallback (the owner pops its own wave) is KE16's defect.

**Accounting and gates.** The fence is one row of the `KE16-DESIGN-W.md` §0 table and is charged
to EVERY arm's wake decision, so the A1-vs-A3 delta on the 1 µs cells is pure transport. W-a is no
longer "zero behavioural change": it is "one contended-line RMW replaced by one local full barrier"
(`KE16-DESIGN-W.md` §1). loom M2's producer-side `fence(SeqCst)` is re-attributed from "the
crossbeam injector push transport" (`tests/loom_pool.rs:143-166, 208-217`) to the production
`publish_fence()` (exported through `loom_exports`, one line, C1), and a calibration copy of M2 with
the producer fence deleted MUST go red — the lost-wake window the file's own fidelity note says it
reproduced (`:147-152`) — recorded by the tester at Step 0 (`KE16-DESIGN-MEASUREMENT.md` §8).

## 2. A2 — `injector_local` in the sibling scan set (P2)

One switch point: `try_steal_random` (`worker.rs:233-257`) probes, for each visited sibling `idx`,
first `inner.stealers[idx].steal_batch_and_pop(local)` (as today) and then, under
`#[cfg(feature = "ke16-a2")]` (A5 shares this probe), `inner.injector_local[idx].steal_batch_and_pop
(local)`. Self is skipped for both (the owner drains its own injector at stage 1). `push_task`'s
placement is unchanged from today under `#[cfg(all(feature = "ke16-a2", not(feature = "ke16-a5")))]`
(A5 has its own arm, §4); it ends in `wake_after_push(inner, pre_len)` with `pre_len =
injector_local[wid].len()`. The joiner's `try_steal_any` (`scope.rs:534-541`) gains the same second
probe under `feature = "ke16-a2"` (it is the B0 joiner; B1 does not exist under A2).

Cost: spawn path unchanged (two single-writer RMWs on `injector_local[wid]` + `pending` + the wake
decision's fence and load — the injector line becomes multi-reader as soon as a sibling probes it,
axis 38). Idle path: each `try_steal_random` sweep pays up to W−1 extra empty-`Injector` probes,
each two Acquire loads + a `SeqCst` fence (`mfence` on x86) — per acquisition, per round of the
backoff loop (`worker.rs:81-125`, ≤11 rounds). That is the cost that should show on
`empty_schedule_control` and on the 1 µs cells, on BOTH routes (the dispatcher's pushes are
unchanged, but every idle worker's scan is longer), which is why A2 is one of the two candidates
subject to the dispatcher-route 1 µs veto (`KE16-DESIGN-MEASUREMENT.md` §7).

Reachability closure: D3 holds because `injector_local[i]` is now in every sibling's scan set; the
external joiner reaches it through `try_steal_any`'s second probe. Doc comment `thread_pool.rs:127-129`
becomes TRUE under A2 (the "stage 1.5" it describes is this probe) and is rewritten to name the site.

No new soundness obligation: `Injector` is MPMC by contract (crossbeam 0.8.7).

## 3. A3 — every spawn to the global injector (P3, the control)

One line: `worker.rs:369-374` → `inner.injector_global.push(task)` unconditionally (with `pre_len =
injector_global.len()` before it); stage-1 polls cfg'd out as under A1 (an unfed queue is not
probed). The two Injector RMWs move from a single-writer line to the one multi-writer line every
spawner and every stealing worker touches (axis 38). This is the reachability floor: a variant that
does not beat A3 beyond the band has bought nothing with its locality, and the one-line change
ships. The dispatcher's pushes are byte-identical to today under A3; its dispatcher-route IDLE path
differs from today only by the removed stage-1 empty-`Injector` probe (one fewer fence + two loads
per acquisition), so a dispatcher-route improvement there is real but shared with A1/A1-fifo, and
the dispatcher 1 µs cells are not a veto criterion for it.

## 4. A5 — idle-keyed placement into a sibling's stealable injector (P17, the owner's mechanism 3)

Requires A2's scan set (Cargo: `ke16-a5 = ["ke16-a2"]`); the destination must be stealable or this
is P6, which the record rejects (N2, N6, N37).

### 4.1 The spawn path

`push_task` under `#[cfg(feature = "ke16-a5")]`:

```rust
// === KE16 A switch: ke16-a5 (on top of ke16-a2) ===
let mask = inner.idle.load(Ordering::Acquire);
if mask != 0 {
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    if let Some(target) = claim_one_idle(inner, mask, start) {   // THE claim core, ONE CAS attempt (KE16-DESIGN-W.md §1.1)
        inner.injector_local[target as usize].push(task);        // a FOREIGN line: SeqCst CAS + slot fetch_or
        inner.workers[target as usize].thread.unpark();          // the claimed worker MUST be woken (A5-1)
        return;
    }
}
// mask == 0 or the claim lost its CAS: A2's placement (own injector via worker_lane_for, global
// otherwise) + wake_after_push(inner, pre_len) — through the fenced prologue of §1.8.
```

`claim_one_idle(inner, mask, start) -> Option<u32>` — ONE signature, defined in
`KE16-DESIGN-W.md` §1.1 (revision 4; the critic's round-3 non-blocking item 3) — is
`unpark_one_idle`'s CAS claim factored out with the unpark removed and bounded to ONE CAS attempt
(`Some(id)` claimed, `None` lost — the fallback arm re-reads the mask through `unpark_one_idle`).
`unpark_one_idle_excluding` is `publish_fence` + load + rotor + `claim_one_idle` + unpark, so there
is one claim core, not two (loom M2/M2b/M2c/M4 transcribe that one shape). The A5 arm applies to the dispatcher's pushes as well
(`worker_lane_for` is `None` there, but the idle-keyed branch precedes the lane test), so A5 is the
other candidate subject to the dispatcher-route 1 µs veto.

The registry it reads: the existing `idle` bitmap (`thread_pool.rs:139-141`) — the same Acquire load
`unpark_one_idle` already performs, now BEFORE placement instead of after. The placement key is a set
bit at or above the rotor; the wake and the placement agree for the first time (the catalogue's P17
observation).

### 4.2 What it costs at 1 µs

Per spawn while any worker is parked: one `Acquire` load (shared line) + one CAS on `idle`
(multi-writer) + an `Injector::push` on ANOTHER core's line (a transfer of the tail-index line and a
block-slot line, `deque.rs:1383-1441`) + one `unpark` (an atomic swap on the target's parker line +
a `WakeByAddressSingle` syscall, since the target IS parked by construction —
`std/src/sys/sync/thread_parking/futex.rs:88-97`). That is ≥ 1 µs of spawner-side cost per task on
a 1 µs body — the cell it is expected to lose (NA-RP measured > 100 ns per pushed task for the
foreign write alone `[P]`; the syscall dominates on Windows). The win it can show: at 100 µs–1 ms
bodies with W−1 siblings parked at a wave boundary, the first W−1 tasks land directly on parked
workers with a targeted wake each, versus A1's "spawn locally, wake one or two, wait for the
O(log W) steal chain".

### 4.3 What A5 must never do

- **A5-1** claim an idle bit without unparking that worker: a worker parks with untimed
  `std::thread::park()` (`worker.rs:117`) and is woken only by `unpark_one_idle`'s claim path or
  shutdown; a claimed-but-not-woken worker is a lost core until shutdown. The claim and the unpark
  are one function, never split by an early return.
- **A5-2** push into `injector_local[target]` without the target queue being in every sibling's
  scan set (A2). Otherwise the task is reachable only by `target`, and a `target` that is woken but
  then loses the race for other work parks again with the task stranded — P0 with an extra syscall.
- **A5-3** push into a foreign pool's injector: the `inner` the spawn targets is the pool whose
  `idle` was read and whose `injector_local` is indexed; there is no cross-pool arm here (the
  fallback for a cross-pool spawn is `injector_global` of `inner`, as today at `worker.rs:373`).
- **A5-4** loop on the mask: one CAS attempt, then the fallback. A spawner must not spin on the
  idle line against W parking workers.
- **A5-5** apply to a wave with an ordering contract: none exists (§1.4), so no restriction —
  recorded so a future caller does not assume same-worker execution for a same-scope wave.

### 4.4 Obligations

loom M2c (index §8): the transcribed claim loop + placement + unpark against one worker running the
real `mark_idle` → re-poll → `park` (loom 0.7.2's real `park`/`unpark`, whose token persists across
an early unpark — `loom-0.7.2/src/rt/thread.rs:153-165`); assert: whenever the bit was claimed, the
worker's `park` returns (the M2 SB-litmus fence discipline applies unchanged). Stress: W−1 workers
parked, N spawns from the one running worker, `claimed == unparked` and every task completes.

## 5. Placement of the dispatcher's pushes (the ECS frame path) under each A

The dispatcher is never a worker (`WORKER_ID_DISPATCHER`, `tls.rs:24`; `worker_lane_for` is `None`
on it), so under A1/A1-fifo/A2/A3 its pushes (`schedule.rs:1272`) go to `injector_global` exactly as
today — the frame path's dispatch is unchanged and `empty_schedule_control` measures only the
idle-path cost differences (A2's extra probes; the removed stage-1 probe under A1/A3). Under A5 the
dispatcher's pushes take the idle-keyed arm too: each ready system lands on a distinct parked worker
with a targeted wake. That is a real change to the frame path and `empty_schedule_control` +
`ke16_par_iter_in_system` are where it shows.

## 6. What each A candidate does to the joiner (why B is decided second)

| A | route-(b) joiner under B0 (as is) |
|---|---|
| A1 (LIFO) | drains nothing at stage 1 (no injector), drains `injector_global` into `scratch` (`scope.rs:488`), then `try_steal_any` from `stealers[0..n]` INCLUDING `stealers[wid]` — its own wave, a real batch of `min((len−1)/2, 32) + 1` into the unregistered `scratch` (no degradation: `scratch` is a different deque, `deque.rs:987-991`) — taken by the LIFO-source path (one `SeqCst` CAS + one fence PER element, `deque.rs:1077-1142`) and reversed into the FIFO `scratch` (`:1145-1156`); serial, no `is_drained` re-check inside the batch |
| A1-fifo | as A1, but the self-steal and every sibling steal into `scratch` is one CAS per batch (`deque.rs:1034-1071`), no reversal (FIFO → FIFO) |
| A2 | drains its own `injector_local[wid]` into `scratch` (`scope.rs:479`), ≤33 per grab, serial |
| A3 | drains `injector_global` into `scratch`, ≤33 per grab, serial |
| A5 | the wave was scattered to idle siblings' injectors; the spawner's own injector holds only what was pushed while nobody was idle; the joiner drains that, then `injector_global`, then siblings' deques AND injectors (A2's scan) into `scratch` |

Every row is defect B on the production route. The A step measures them anyway, on equal footing
(all B0), so the A decision is not contaminated by a joiner design that exists for only one of them.

## 7. Argued away, with the number that would reopen them

| Variant | Why not built | Reopened if |
|---|---|---|
| A2′ (P18, rayon `JobFifo`) | prerequisite: A1's TLS pointer; on top of A1 it adds an `Injector::push` CAS + slot `fetch_or` + a placeholder deque push per spawn and an `Injector::steal` (fence + CAS + epoch pin) per execution; its only distinct property — spawns are executed FIFO behind the deque's older entries — is the A1-fifo row | A1-fifo beats A1 on a consumer beyond the band AND the FIFO joiner's inline-sibling-system behaviour is judged undesirable: then P18 gives FIFO-for-spawns with LIFO deques. Not expected |
| A4 (P4, Tokio/Go bounded ring + spill) | the same spawn-path cost as A1 (plain stores, and the same producer fence) for a rewrite of the substrate (a fixed ring, a packed head, a spill protocol) and a new overflow policy to specify; its motivation in Tokio was crossbeam's epoch reclamation on the STEAL path (N8), which A1 pays once per ≤32 tasks | A1 fails its Miri gate with a UB REPORT (not a timeout — §1.3) that the D5 discipline cannot remove |
| P6 (non-stealable push-to-idle) | rejected by Go (N2: thread-state thrashing, locality destroyed, latency) and FJP (N6: threads that finish other work first can take the task instead) and measured as a loss by XGOMP NA-RP (N37) | never; its stealable form is A5 |
| P5 (TBB dual-residency mailbox) | a proxy allocation + a foreign-line write per spawn + a CAS per extraction, for a locality gain measured on a 2002 14-processor machine and unfalsifiable here without L10 | an L10 instrument exists and shows working-set migration dominating |
| P7 / P8 (private deques + requests; split deque) | the victim polls on its working path / must reach a release point; single-digit % gains at best, −102 % worst (LCWS); over crossbeam for µs chunks at W=16 not worth a second protocol; Weave's author abandoned P7 for shared memory (N63) | a fence-cost profile on the owner's pop path shows the Chase-Lev fence as a top-3 cost on a consumer |
| P24 (Wicked rotating-counter placement), P25 (stlab rotating trylock) | a foreign-line push per spawn (P24: a `fetch_add` on a shared counter, then another worker's queue) with no idle key — strictly A5's cost without A5's targeting | never on this grid |
| P23 (rayon broadcast deque) | the owner-only per-worker queue is correct only for "run once on EVERY worker" semantics; ours are "whoever is free" | not a placement for waves |
| P9, P12, P13, P14, P15, P16, P19–P22 | scale-out, static, central, unbuildable, isolation-by-design, thread-per-core, lock-based or nesting-banned designs — each closes reachability at a higher per-task cost than A1 or forbids the nested case being fixed (`KE16-VARIANTS-PLACEMENT.md`, addenda) | not by a number on this grid |
| E26 per-scope registered queue | theory-only; a live-scope registry probed on every steal is one more shared structure than `inner.stealers` | not this round |

`ThreadPool::spawn` (fire-and-forget, `thread_pool.rs:390-395`) has no production caller found
(index §5); it takes whichever A arm wins. Axis 39 (per-call placement) is a design note, not built.
