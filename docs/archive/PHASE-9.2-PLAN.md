# Architecture: Phase 9.2 REVISION — multi-drain-safe `Scope` free (Candidate U: unpark-before-decrement)

The following is the complete replacement content for `docs/PHASE-9.2-PLAN.md` (English). The orchestrator must persist it verbatim, overwriting the current stale/broken-handshake plan. My agent tool set exposes no file-write capability; the content is delivered in full below.

I recommend **Candidate U (unpark-before-decrement)**. It is a net deletion of machinery: it removes the entire `free_state` handshake, the `spawned_any` `Cell`, and the worker-side `Box::from_raw` that the current (crashing) 9.2 design added, and frees the box unconditionally at the single `Scope::drop` site — which is multi-drain-safe by construction (the box is freed once, when the scope truly ends, never on an intermediate wave's `pending -> 0`). The UAF is eliminated because the worker's `pending.fetch_sub` becomes its last byte-access to the allocation. K is specified as the perf-clean fallback; H2 is rejected.

```markdown
# Phase 9.2 (REVISION) — Multi-Drain-Safe `Scope` Free: unpark-before-decrement (Candidate U)

**Status:** PLAN (architect → critic → developer). REVISION of the prior 9.2 plan.
This **supersedes and DELETES** the `free_state` "second-swapper-frees" handshake design,
which passed `miri_scope`/loom/stress (all SINGLE-drain) but **deterministically
double-frees** (`0xc0000374 STATUS_HEAP_CORRUPTION`) in `phase9_schedule_run_two_disjoint`,
whose executor drives `ScopeShared.pending` to 0 **multiple times per scope** (once per
dispatch wave). The handshake assumed "pending hits 0 exactly once per scope" (its broken
"Lemma 0"); under multi-drain, wave 2's last completer swaps `JOINED -> JOINED`, observes
`JOINED`, and frees a box that is still alive mid-closure ⇒ `Scope::drop` frees it again.

**Recommended design: Candidate U — unpark-BEFORE-decrement.** The worker's
`pending.fetch_sub` becomes its FINAL byte-access to the `ScopeShared` allocation; the box
is then freed UNCONDITIONALLY at the single `Scope::drop` site after the join. This is a NET
SIMPLIFICATION: it removes the `free_state` atomic, the `FREE_RUNNING`/`FREE_JOINED`
constants, the `spawned_any` `Cell`, the worker-side `Box::from_raw`, and the
`complete_task -> bool` return. It keeps the landed `NonNull<ScopeShared>` field refactor
(the Tree-Borrows-protector fix — DO NOT relitigate) untouched.

This file is for the orchestrator to persist verbatim to `docs/PHASE-9.2-PLAN.md` (English;
the architect's tool set has no file-write capability).

---

## 1. Goal

Make `boyko_threadpool`'s `Scope` fork/join **data-race-clean AND Tree-Borrows-clean** under
Miri **and** crash-free under the real multi-wave ECS executor, keeping:
- the landed `NonNull<ScopeShared>` field (the TB-protector fix — proven GREEN by
  `tests/miri_scope.rs`, 16 seeds);
- the prompt `waker.unpark()` wake edge at every wave join (the executor's between-wave wake
  depends on it — removing it is a REJECTED perf regression);
- native steady-state cost within ±5% of the clean-machine baseline (§10).

**Functional invariant (the property the handshake lacked).** The box is freed **EXACTLY
ONCE**, at `Scope::drop` (when `'scope` truly ends), **NEVER** on an intermediate wave's
`pending -> 0`. No worker reads **any byte** of the allocation after the instant the
dispatcher may free it.

---

## 2. Context and constraints

**Subsystems affected:** `crates/boyko_threadpool/src/scope.rs` (logic). `src/lib.rs`
`loom_exports` (revert `complete_task` to `()` + drop the `dispatcher_swap_frees` shim).
`tests/loom_pool.rs` M1 (drop the exactly-one-freer assertions; keep no-lost-wakeup). Test
attribution in `tests/miri_scope.rs`, `crates/boyko_ecs/tests/{miri_schedule_parallel.rs,
miri_phase9.rs}`. **No** change to `worker.rs`, `thread_pool.rs`, `sync.rs` logic, or any ECS
hot path. `src/sync.rs`'s `AtomicU8` re-export may stay (harmless, unused) or be removed
(§14 step 1) — removal is cleaner.

**Invariants preserved (inviolable; user-mandated):**
1. No `Arc`/`Rc` in spawn/complete/join; no new `Mutex`/`RwLock`/spinlock on the hot path
   (the cold-path `panic_payload: Mutex` stays).
2. `waker.unpark()` MUST still fire on each wave's `pending -> 0` (the executor's between-wave
   wake; §3 Decision 2 / §9).
3. The box is freed EXACTLY ONCE at `Scope::drop`, NEVER on an intermediate wave (§5 proof).
4. Data-race-clean AND Tree-Borrows-clean under Miri (§5 / §6 / §12).
5. Zero/near-zero native steady-state cost; baselines preserved within ±5% (§10).
6. Remain loom-modelable for liveness (no-lost-wakeup), like the existing M1 model (§9).

**The bug being fixed (original, precise).** `ScopeShared::complete_task` reads `self.waker`
(non-atomic) AFTER `pending.fetch_sub(AcqRel) -> 0` and calls `unpark()`. `Scope::drop` (after
`join_workers_until_drained` observes `pending == 0`) frees the box. The last worker's
post-decrement `waker` read is **not ordered-before** the free ⇒ data-race UAF (Miri's
data-race checker flagged it). This is structurally identical to `std::thread::scope`'s
finisher — `if num_running_threads.fetch_sub(1, Release) == 1 { self.main_thread.unpark() }` —
which reads `main_thread` AFTER the decrement and is sound **only because** `ScopeData` lives
in an `Arc` whose last clone (main's or the finisher's) frees it (std comment: *"We put the
`ScopeData` into an `Arc` so that other threads can finish their
`decrement_num_running_threads` even after this function returns."*). boyko removed the Arc
(for the TB win) and thereby removed that lifetime extension. **Candidate U re-supplies
safety WITHOUT an Arc by making the decrement the worker's LAST allocation access** (unpark
moves *before* it), so no post-decrement read exists to race the free.

**The bug the broken 9.2 introduced (multi-drain double-free).** The ECS executor
(`schedule.rs::executor_main_loop`, one `pool.install(|scope| …)` per frame, §3 below) drives
systems in **waves** gated by `apply_window_drain` barriers + conflict-graph dependencies.
Between waves the dispatcher **parks** and is woken by `complete_task`'s `unpark` at each
wave's `pending -> 0`. So `pending` oscillates to 0 **once per wave**, not once per scope. The
`free_state` handshake's free-election fires on EVERY `pending -> 0`; on wave 2 the last
completer observes the already-`JOINED` state and frees the live box ⇒ double-free. (The
single-drain `miri_scope`/stress/`par_iter` tests never exposed this — they drain to 0 once.)
**Candidate U is immune: it ties the free to `Scope::drop` alone (scope END), not to any
`pending -> 0` event.**

**Target metrics:**
- Added atomics on any contended location: **0** (the `pending` RMW is byte-identical; the
  only motion is the `unpark()` call site moving 2 lines up).
- Box size: **shrinks** by `free_state` (removes one `AtomicU8`; the current 9.2 added a
  `CachePadded<AtomicU8>` per the original plan — U deletes it).
- `phase9_scheduler` ALL 5 benches: complete exit 0 (no `0xc0000374`), within ±5% of the §5
  baseline (§10).
- Wave-join wake latency: unchanged in the common case; bounded by the existing 50 µs /
  100 µs backstops in the rare lost-wakeup window (§9.2 proof).

---

## 3. The executor's wave model (why multi-drain is the governing constraint)

`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`:
- `Schedule::run` (l.279): **one** `pool.install(|scope| self.executor_main_loop(world, scope))`
  per frame. The scope's `ScopeShared` is created once per frame.
- `executor_main_loop` (l.361): a loop. Each iteration: Step 1 `apply_window_drain` (gated on
  `pending_apply == running`), Step 1.5 conditions, Step 2 termination, Step 3
  `try_dispatch_ready` (spawns this wave's concurrent systems via `scope.spawn`, l.926), Step 5
  **`std::thread::park_timeout(PARK_TIMEOUT=100µs)`** (l.461) when `dispatched == 0 && running
  > 0`.
- The systems spawned in one wave each `complete_task` on finishing. When the wave's last
  system completes, `complete_task` observes `prev == 1` and `waker.unpark()`s the parked
  dispatcher (the `ScopeShared.pending` counter — distinct from the ECS `pending_apply` — is
  the scope's task counter). The dispatcher wakes, drains, dispatches the next wave. So
  **`ScopeShared.pending` goes `…→0→…→0→…` once per wave; the final 0 is at the last wave**,
  and only then does the install closure return and `Scope::drop` run.

Two independent backstops already exist for the wake:
- The executor's Step-5 `park_timeout(100µs)` (`schedule.rs:461`).
- `join_workers_until_drained`'s own `park_timeout(50µs)` (`scope.rs:519`), reached when the
  dispatcher is INSIDE `Scope::drop` waiting for the final wave's stragglers.

These backstops are the liveness guarantee for Candidate U's lost-wakeup window (§9.2).

---

## 4. Key decisions

### Decision 1: Candidate U — unpark-BEFORE-decrement; free unconditionally at `Scope::drop`

**What.** Rewrite `complete_task` so the LAST allocation access by any worker is the atomic
`pending.fetch_sub`:

```rust
fn complete_task(&self) {
    // Read the box (waker) FIRST, while pending is still >= 1 ⇒ box guaranteed alive.
    self.waker.unpark();                          // last *byte* read of the allocation
    self.pending.fetch_sub(1, Ordering::AcqRel);  // last *atomic* access; box may free after
}
```

DELETE the entire `free_state` handshake, the `FREE_RUNNING`/`FREE_JOINED` constants, the
`spawned_any: Cell<bool>`, the `complete_task -> bool` return, the worker-side `Box::from_raw`
in the spawn wrapper, and the `dispatcher_frees`/`prev_state` logic in `Scope::drop`. Free
the box UNCONDITIONALLY at the single site at the end of `Scope::drop`, after the join (as the
pre-9.2 baseline did).

**Why (perf/cache/parallelism).**
- **Multi-drain-safe by construction (the governing requirement, §3).** The free is tied to
  `Scope::drop` — the scope END — not to any `pending -> 0` event. `pending` may oscillate to
  0 across N waves; none of those triggers a free. There is exactly one `Box::from_raw`, at one
  site, once per scope. Constraint 3 satisfied.
- **UAF-free (constraint 4).** The worker's `fetch_sub` is its final access to the allocation
  (`unpark` moved before it). After `fetch_sub`, the worker touches nothing in the box. The
  free at `Scope::drop` is ordered-after every worker's `fetch_sub` by the standard
  counter+Acquire pattern (§5). No post-decrement read exists. This is the same discipline
  rayon documents for latches (*"read all the fields you will need before a latch is set… the
  target may proceed and invalidate `this`"*) — `unpark` is the only field read, so it goes
  first.
- **0 added contended atomics (constraint 5).** The hot `pending.fetch_sub(AcqRel)` is
  byte-identical (same op, same ordering, same `prev` test removed — see Decision 2). No new
  atomic anywhere. The box SHRINKS (no `free_state`).
- **Net deletion of machinery.** Removes the handshake, the `Cell`, the second free site, and
  three proof obligations (double-free-freedom, leak-freedom, the `!spawned_any` lone-swapper
  case) that the broken design needed. Fewer lines, fewer states for Miri/critic to cover.
- **No Arc, no Mutex, no spinlock (constraint 1).** One pointer alloc/free, no refcount.

**Alternatives rejected:**
- **The `free_state` handshake (the crashing 9.2 design).** Disqualified: not multi-drain-safe
  (§3) ⇒ deterministic double-free in the executor bench. Irreparable without per-wave reset
  machinery (that is H2, also rejected).
- **Candidate K (stable waker out of the box).** Sound and perf-clean, but materially more
  machinery (a pool-stable waiter slot + publish/observe sync for the dispatcher's `Thread`
  handle) for behavior identical to U on the current workload. Specified as the fallback (§11)
  if U's spurious-unpark cost proves measurable; NOT the primary.
- **Candidate H2 (multi-drain-safe handshake with per-wave reset).** Most complex, most
  bug-prone (a `register_task` 0->1 reset racing the dispatcher's drop-signal). Rejected:
  U/K both clear every hard constraint without it.

**Trade-off.** Two costs, both quantified and accepted (§9, §10):
(a) the `unpark()` is now **unconditional** (every completion, not just the last) ⇒ spurious
wakeups; (b) because `unpark` precedes the decrement, a rare **lost-wakeup window** exists
where the dispatcher parks on a stale `pending > 0` after consuming the token and misses the
final decrement, falling back to the 50 µs/100 µs `park_timeout` backstop. Both are shown
acceptable in §9.

### Decision 2: drop the `prev == 1` gate — unpark unconditionally

**What.** `complete_task` no longer branches on `prev == 1`. It unparks on EVERY completion,
then decrements. The `pending.fetch_sub` return value is discarded.

**Why.** With unpark BEFORE the decrement, gating on `prev == 1` is impossible without reading
`pending` *after* the sub (which re-introduces a post-decrement box access — the very UAF we
are removing; `pending` is inside the allocation). An unconditional unpark before the decrement
is the simplest UAF-free shape. The spurious-unpark cost is cheap (§10): for the inner
`par_iter` scope the dispatcher is busy-stealing (not parked), so `Thread::unpark` on a
non-parked thread is a single atomic token-set (NO syscall); for the executor scope there are
few completions/wave and the wake is needed anyway. Crucially, unconditional unpark also
**minimizes** the lost-wakeup window (§9.2): more unpark tokens ⇒ the dispatcher is less likely
to be stranded on a stale repark with no pending token.

**Alternative rejected:** a peek-gated unpark `if pending.load(Relaxed) == 1 { unpark() }
fetch_sub(AcqRel)` (Candidate **U'**, §11) restores last-only unparks (near-zero spurious) and
is race-free (the peek-load is before `fetch_sub`, hence before the free), but it trades the
spurious-unpark cost for a slightly LARGER missed-wake latency window (a wrong peek ⇒ no unpark
on the true-last decrement ⇒ rely on the timeout). U' is documented as the drop-in optimization
to apply ONLY if the unconditional unpark shows measurable cost in `phase9_par_iter` (§10/§11);
it needs no new design round.

**Trade-off.** Spurious unparks (cheap, §10) in exchange for the simplest UAF-free shape and
the smallest lost-wakeup window.

### Decision 3: `Scope::drop` frees unconditionally after the join — single free site

**What.** `Scope::drop`: (1) join via `join_workers_until_drained`; (2) read+take
`panic_payload` (a `Mutex`, Sync) and `debug_assert!(is_drained())`; (3) **unconditionally**
`Box::from_raw(raw)` (the single free); (4) `resume_unwind` the payload last (outside any
`*raw` access). No swap, no `dispatcher_frees`, no `spawned_any`.

**Why.** After the join returns, `pending == 0` (Acquire-observed in `is_drained`), so no
worker will START a new `complete_task`, and every worker that ran has completed its
`fetch_sub` (its last box access) — which happens-before the join's Acquire load (§5). The
dispatcher is then the sole owner; an unconditional free is correct and is the ONLY free. The
zero-task case is automatic: `pending` was never raised, `is_drained()` is trivially true, the
dispatcher frees the box it solely owns. No special-case flag needed (the `spawned_any` `Cell`
the handshake required is DELETED).

**Subtlety (inline-on-dispatcher).** `join_workers_until_drained` steals and runs tasks inline
on the dispatcher. An inline task's `complete_task` runs on the dispatcher thread: it
`unpark`s (itself — benign no-op token) then `fetch_sub`. When the inline task is the last, the
dispatcher's own next `is_drained()` poll (straight-line after the inline `run()`) observes 0
and returns from the join; then frees. Still exactly one free, on the dispatcher, after the
join. Single-threaded program order trivially orders the inline `fetch_sub` before the free.
(Covered by `scope_inline_drain_frees_once`, 1-worker pool, §11.)

**Trade-off.** None — this is the pre-9.2 baseline `Scope::drop` shape (unconditional free),
restored. The only retained 9.2 element is the `NonNull` raw-pointer access (the TB fix).

### Decision 4: keep `panic_payload` read BEFORE the free; `resume_unwind` last

**What.** Read+take the payload before `Box::from_raw`; `resume_unwind` after the free on the
stack-local payload.

**Why.** The payload lives in the box; it must be extracted before the box is freed. Reading it
before the (unconditional) free means no `*raw` access follows the free. `resume_unwind`
operates on a moved-out `Box<dyn Any>` that no longer aliases the allocation. (This ordering
was already correct in both the baseline and the broken 9.2; U keeps it, minus the swap that
used to sit between.)

**Trade-off.** None.

---

## 5. The core soundness proof — free is ALWAYS ordered after every worker's last byte-access, across the MULTI-WAVE pattern

Let **A** = the `ScopeShared` allocation. Two atomics matter: `pending` (per-task RMW) and the
implicit happens-before of `Box::from_raw` at `Scope::drop`. There is now exactly ONE free site.

**Definitions.** Within one scope, `register_task` does `pending.fetch_add(1, AcqRel)` (on the
dispatcher, in `spawn`, before the task is enqueued). `complete_task` does `waker.unpark()`
then `pending.fetch_sub(1, AcqRel)`. The dispatcher's `Scope::drop` calls
`join_workers_until_drained`, which loops `pending.load(Acquire) == 0` and returns on true,
then `Box::from_raw(raw)`.

**Lemma U1 (each worker's last A-access is its `fetch_sub`).** A spawned task's body wrapper
touches A only via: (i) `panic_payload.lock()` (on the panic path, before `complete_task`);
(ii) `complete_task`'s `waker.unpark()` then `pending.fetch_sub`. After `fetch_sub` the wrapper
returns without dereferencing `shared_ptr` again (the worker-side `Box::from_raw` is DELETED).
So every worker's last A-access is its `pending.fetch_sub`. ∎

**Lemma U2 (multi-wave: the dispatcher's free is ordered after the LAST decrement of the FINAL
wave).** The `pending` modification order is a single total order over all `fetch_add` and
`fetch_sub` across ALL waves. `Scope::drop` runs only after the install closure returns, i.e.
after the executor's main loop exits (Step 2 termination: `completed == n`), i.e. after the
FINAL wave's systems have all completed. The join loop's `pending.load(Acquire)` returns only
when it reads 0. The value 0 in `pending`'s modification order is produced by the final
`fetch_sub` of the final wave (the unique decrement from 1 to 0 that is not followed by any
later `fetch_add` — there are no more `register_task` calls after the loop exits). The
Acquire load that reads this 0 **synchronizes-with** that final `fetch_sub`'s Release. Every
PRIOR `fetch_sub` (all earlier completions, all earlier waves) is ordered-before the final one
in the RMW total order, hence ordered-before the joiner's Acquire load. Therefore EVERY
worker's `fetch_sub` (Lemma U1: its last A-access) happens-before the joiner's Acquire load,
which happens-before (program order on the dispatcher) the `Box::from_raw`. ∎

> Why intermediate `pending -> 0` events are harmless: an intermediate wave drives `pending`
> to 0, but the join loop is NOT running then (the dispatcher is in `executor_main_loop`'s
> Step-5 `park_timeout`, not in `Scope::drop`). No free is attempted on an intermediate 0.
> `Scope::drop` runs exactly once, at scope end, and its join observes the FINAL 0. This is the
> precise property the `free_state` handshake lacked: it fired a free-election on every 0;
> U fires a free only at `Scope::drop`.

**Lemma U3 (no intra-wave reuse hazard).** Could a later wave's `fetch_add` make the joiner's
"read 0" ambiguous? No: the joiner only runs in `Scope::drop`, after the loop exited, after the
LAST `register_task`. So when the joiner observes 0, no further `fetch_add` can occur (the
install closure has returned; `spawn` is unreachable). The observed 0 is final and stable. ∎

**Theorem (no use-after-free; freed exactly once; multi-drain-safe).** By Lemmas U1–U3, every
A-access by every worker across every wave happens-before the single `Box::from_raw` at
`Scope::drop`. There is exactly one free site, reached once per scope (Drop runs once),
unconditionally. ⇒ no use-after-free, no double-free, no leak — including under the multi-wave
oscillation of `pending`. The original post-decrement-`waker`-read race is eliminated because
no A-access follows any `fetch_sub` (Lemma U1). ∎

**Tree-Borrows cleanliness (unchanged from the landed `NonNull` fix).** TB cleanliness is
supplied by the landed `NonNull<ScopeShared>` field: the joiner takes a by-value `*const
ScopeShared`, so `Scope::drop`'s `&mut self` protector covers only the 8-byte field, never A;
no `&mut self`-derived reference to A spans the workers' `pending` writes. Candidate U adds NO
new reference over A and REMOVES one atomic (`free_state`): `complete_task` takes `&self` (a
transient shared reborrow for the `pending` atomic + the `waker` field read — TB permits
foreign reads/writes against non-protected shared tags); `Scope::drop` accesses `*raw` only
through the raw pointer (`(*raw).field`), never forming a retained `&ScopeShared`; the
`Box::from_raw` is a single owning consume. So TB sees strictly LESS than the landed-clean
state (one fewer atomic, no handshake). The data-race checker is satisfied by the Theorem.

---

## 6. Edge cases — all resolved

### 6.1 Zero tasks spawned
`complete_task` never runs; `pending` stays 0; `register_task` never called.
`join_workers_until_drained` reads 0 immediately and returns. `Scope::drop` frees the box it
solely owns. Exactly one free. ✓ (No `spawned_any` flag needed — the unconditional free is
correct in all cases; the flag the handshake required is DELETED. Covered by
`scope_zero_tasks_frees_no_leak`.)

### 6.2 Tasks run INLINE on the dispatcher (§4 Decision 3 subtlety)
`join_workers_until_drained` steals + runs tasks on the dispatcher. The inline task's
`complete_task` runs on the dispatcher thread: `unpark` (self; benign) then `fetch_sub`. If it
is the last, the dispatcher's next `is_drained()` poll (straight-line after `run()`) reads 0,
returns from the join, frees. One free, on the dispatcher, after the join; single-threaded
program order trivially orders the inline `fetch_sub` before the free. ✓ (Covered by
`scope_inline_drain_frees_once`, 1-worker pool.)

### 6.3 Single free site (no double-free possible)
The worker-side `Box::from_raw` is DELETED. The ONLY `from_raw` is in `Scope::drop`, reached
once. Double-free is structurally impossible (one site, one Drop). ✓

### 6.4 Panic path — payload read before the free
Wrapper stores the panic payload before `complete_task` (unchanged). `Scope::drop` reads+takes
the payload before `Box::from_raw`. A panicking worker's `panic_payload.lock()` store is before
its `fetch_sub` (program order), which happens-before the joiner's Acquire (Lemma U2), which is
before the dispatcher's payload read — so the store is visible. `resume_unwind` runs last on the
moved-out payload, never touching `*raw`. First-panic-wins is unchanged (`slot.is_none()` gate
in the wrapper). ✓ The transmute premise ("no task body outlives `'scope`, even on unwind") is
preserved — the join site did not move; the only deletions are post-join machinery.

### 6.5 Nested scopes
An inner `pool.scope(...)` (e.g. `par_iter` from a system body) has its OWN A and OWN `pending`.
The inner `Scope::drop` runs on the worker thread; its free is independent. The proof (§5) is
per-allocation, so nested scopes compose. ✓ (Covered by `stress_nested_scopes_exactly_once`,
`nested_scope_does_not_deadlock`.)

### 6.6 Multi-wave executor scope (the regression case)
The frame's single scope drives `pending` to 0 once per wave (§3). No free fires on any
intermediate 0 (Lemma U2: free is only at `Scope::drop`, after the loop exits on the FINAL
wave). Exactly one free at frame end. ✓ This is the case the broken handshake double-freed; U
is immune. **Covered by the NEW `scope_multi_drain_frees_once` test (§11.4) AND the full
`phase9_scheduler` bench (§12.5).**

---

## 7. Data structures and exact bodies

All edits in `crates/boyko_threadpool/src/scope.rs`. Native cost: the box SHRINKS; `pending`
RMW byte-identical.

### 7.1 `ScopeShared` — remove `free_state`; revert to the pre-handshake shape

```rust
/// Shared state between [`Scope`] and its spawned tasks. Heap-allocated; the
/// `Scope` holds it as a raw `NonNull` (the landed TB-protector fix — §5).
#[repr(C)]
pub(crate) struct ScopeShared {
    /// Outstanding spawned-task count. `register_task` increments; `complete_task`
    /// decrements; `Scope::drop`'s join waits for 0. `CachePadded` to keep the
    /// per-task completion traffic off neighbouring atomics.
    pub(crate) pending: CachePadded<AtomicUsize>,

    /// First panic payload (cold; `Mutex` is panic-path only). Unchanged.
    pub(crate) panic_payload: Mutex<Option<Box<dyn Any + Send + 'static>>>,

    /// Thread to unpark when a wave's `pending` hits a low watermark. Read by
    /// `complete_task` BEFORE its `pending.fetch_sub` (Decision 1 / §5 Lemma U1),
    /// so the read is ordered-before any free.
    pub(crate) waker: Thread,
    // NOTE (Phase 9.2 REVISION): `free_state: AtomicU8` is REMOVED (Candidate U).
    // The free is unconditional at `Scope::drop`; no per-scope handshake.
}

impl ScopeShared {
    #[inline]
    pub(crate) fn new(waker: Thread) -> Self {
        Self {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: Mutex::new(None),
            waker,
        }
    }

    /// register_task — UNCHANGED (fetch_add AcqRel). Loom-frozen (M1).
    #[inline]
    pub(crate) fn register_task(&self) {
        self.pending.fetch_add(1, Ordering::AcqRel);
    }

    /// is_drained — UNCHANGED (load Acquire). Loom-frozen (M1).
    #[inline]
    pub(crate) fn is_drained(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }
}
```

Remove the `FREE_RUNNING` / `FREE_JOINED` consts entirely. Remove the `AtomicU8` import from
the `crate::sync` use line in `scope.rs`.

### 7.2 `complete_task` — unpark BEFORE decrement; return `()`

```rust
/// Mark one task complete. Unparks the scope's waker, THEN decrements `pending`.
///
/// ORDER IS LOAD-BEARING (Candidate U, §5 Lemma U1): `waker.unpark()` is this
/// thread's FINAL read of the allocation; `pending.fetch_sub` is its FINAL atomic
/// access, after which the dispatcher's `Scope::drop` may free the box. No box
/// access may follow the decrement (rayon's read-before-signal discipline).
///
/// The unpark is UNCONDITIONAL (Decision 2): gating on `prev == 1` would require
/// reading `pending` AFTER the sub (a post-decrement box access = the UAF we are
/// removing). The executor's between-wave wake still fires — every completion
/// unparks; the dispatcher's `is_drained()` re-poll filters spurious wakes.
///
/// Orderings: `pending.fetch_sub(AcqRel)` — UNCHANGED; the join's `is_drained`
/// Acquire load synchronizes-with the FINAL wave's decrement, ordering the free
/// after every worker's last access (§5 Lemma U2). loom-proven (M1).
#[inline]
pub(crate) fn complete_task(&self) {
    // Final read of the allocation (`waker`) while `pending >= 1` ⇒ box alive.
    self.waker.unpark();
    // Final atomic access; the box may be freed by `Scope::drop` after this.
    self.pending.fetch_sub(1, Ordering::AcqRel);
}
```

### 7.3 The `spawn` wrapper tail — drop the bool branch + the worker-side free

```rust
let wrapped = move || {
    let result = catch_unwind(AssertUnwindSafe(f));

    // SAFETY: `shared_ptr` names the live `ScopeShared` allocation (created in
    //   `Scope::new`, freed only by `Scope::drop` after the join). This is a
    //   transient shared reborrow on the worker thread; it MUST NOT outlive the
    //   `complete_task` call (after the decrement the dispatcher may free A).
    let shared = unsafe { shared_ptr.as_ref() };

    if let Err(payload) = result
        && let Ok(mut slot) = shared.panic_payload.lock()
        && slot.is_none()
    {
        *slot = Some(payload);
    }
    // First panic wins; later payloads dropped; poisoned mutex ⇒ drop payload,
    // the scope still completes via `pending` reaching 0.

    // Last action: unpark-then-decrement. After this the worker performs NO
    // further access to A (the worker-side `Box::from_raw` is DELETED, Candidate
    // U). `shared` MUST NOT be dereferenced after this line.
    shared.complete_task();
    // NB: no `must_free` branch, no `Box::from_raw` here. The box is freed solely
    // by `Scope::drop` (§5 / §6.3).
};
```

`SharedPtr` keeps `as_ref`; its `as_ptr` accessor is no longer needed by the wrapper (it is
still used nowhere else) — remove `as_ptr` if it becomes dead (clippy `-D warnings`), or keep
with `#[allow(dead_code)]` if a future use is anticipated. Recommend: REMOVE it (cleaner).

### 7.4 `Scope` struct — remove `spawned_any`

```rust
pub struct Scope<'scope> {
    pool: &'scope ThreadPool,
    /// Owned `ScopeShared` as a raw `NonNull` (the landed TB-protector fix — §5).
    /// Created by `Box::into_raw` in `new`, freed by `Box::from_raw` in `drop`.
    pub(crate) shared: NonNull<ScopeShared>,
    // NOTE (Phase 9.2 REVISION): `spawned_any: Cell<bool>` is REMOVED — the
    // unconditional free at `Scope::drop` is correct for zero-task scopes too
    // (§6.1), so the lone-swapper flag the handshake needed is gone.
    _phantom: PhantomData<&'scope mut &'scope ()>,
}
```

`Scope::new` drops the `spawned_any: Cell::new(false)` initializer (otherwise UNCHANGED — still
`Box::into_raw` → `NonNull`). `spawn` drops the `self.spawned_any.set(true)` line. Remove the
`use core::cell::Cell;` import if it becomes unused.

### 7.5 `Scope::drop` — unconditional free after the join (baseline shape, NonNull access)

```rust
impl<'scope> Drop for Scope<'scope> {
    fn drop(&mut self) {
        // NonNull (Copy): `as_ptr` copies the pointer WITHOUT retagging the
        // pointee, so this Drop's `&mut self` protector covers only the 8-byte
        // field, never the heap allocation (the landed TB fix — KEEP).
        let raw: *mut ScopeShared = self.shared.as_ptr();

        // SAFETY: `raw` is live for the whole join (freed only by the single
        //   `Box::from_raw` below, after this returns). The join reborrows `*raw`
        //   only per-poll for one Acquire load, never spanning a worker write
        //   (the NonNull/raw-pointer design).
        unsafe { join_workers_until_drained(self.pool, raw) };

        // The join returned ⇒ `pending == 0` (the FINAL wave's decrement, §5
        // Lemma U2). No worker will start a new `complete_task`, and every worker
        // that ran has completed its `fetch_sub` (its last A-access), which
        // happens-before the join's Acquire load. The dispatcher is now the SOLE
        // owner of A.
        //
        // SAFETY: pre-free shared access; `panic_payload` is a `Mutex` (Sync);
        //   `is_drained()` is an Acquire load. Read the payload BEFORE the free so
        //   no `*raw` access follows the deallocation (§6.4).
        debug_assert!(
            unsafe { (*raw).is_drained() },
            "Scope::Drop returned with pending tasks still in flight"
        );
        let payload = {
            let mut slot = unsafe { (*raw).panic_payload.lock() }
                .expect("invariant: panic_payload mutex never poisoned by us");
            slot.take()
        };

        // SAFETY (single free site — §5 Theorem / §6.3):
        //   - `raw` is the `Box::into_raw` address from `Scope::new`.
        //   - The dispatcher is the unique remaining owner (join observed
        //     `pending == 0`; every worker's last A-access — its `fetch_sub` —
        //     happens-before the join's Acquire load, §5 Lemma U2).
        //   - The payload was read ABOVE, before this free; no `*raw` access
        //     follows. Reached once (Drop runs once), unconditionally ⇒ freed
        //     exactly once, no double-free, multi-drain-safe.
        unsafe { drop(Box::from_raw(raw)); }

        // Re-raise OUTSIDE any `*raw` access (payload is a moved-out stack local).
        if let Some(p) = payload {
            resume_unwind(p);
        }
    }
}
```

### 7.6 `join_workers_until_drained` — UNCHANGED
Signature stays `unsafe fn(pool: &ThreadPool, shared: *const ScopeShared)`. The `#[cfg(miri)]`
yields, the steal sources, the `park_timeout(50µs)` backstop, and the per-poll `(*shared).
is_drained()` are all unchanged. (The `park_timeout` is now ALSO the liveness backstop for
Candidate U's lost-wakeup window — §9.2.)

---

## 8. Public API and panic-safety

**No public API change.** `Scope`, `ThreadPool::{install, scope}`, `Scope::spawn` signatures
unchanged. `complete_task` reverts from `bool` to `()` (internal; the only callers are the
spawn wrapper and the loom shim). `ScopeShared`/`Scope` field changes are `pub(crate)`/private.

**Panic-safety (highest-stakes; unchanged).** The join stays in `Scope::drop`. A worker
closure panic is caught by the wrapper's `catch_unwind`, stored, and the wrapper still calls
`complete_task` (so `pending` reaches 0 and the join terminates). A dispatcher-body panic still
triggers `Scope::drop` during unwinding, which still blocks on the join before `'scope` borrows
die. The transmute premise is structurally preserved (the join site did not move). The only
deletions are post-join machinery (the swap, the second free, the flag), none panic-capable.
`Box::from_raw` + drop of `ScopeShared` (drops `Mutex`, `Thread`, `AtomicUsize` — none panic)
do not panic; the sole `resume_unwind` is intentional and last. ✓

---

## 9. Lost-wakeup / liveness analysis (the crux of Candidate U)

### 9.1 The wake edge still exists and still fires per wave
`complete_task` still calls `waker.unpark()` on EVERY completion, including the wave's last.
The executor's between-wave wake (`schedule.rs`: dispatcher parks at Step 5, woken by the
wave-last completer) and the in-`Scope::drop` join wake both still receive an unpark. Constraint
2 satisfied. Latency in the COMMON case is unchanged (the unpark fires; the dispatcher wakes
promptly).

### 9.2 The lost-wakeup window and why the backstop covers it
Because `unpark` now precedes `fetch_sub`, a narrow window exists:
1. dispatcher reads `pending > 0` (`is_drained()` false or executor `running > 0`);
2. the last worker `unpark()`s (sets/keeps the dispatcher's park token) THEN `fetch_sub -> 0`;
3. dispatcher `park`s → returns immediately (std persists the token — confirmed against std);
4. dispatcher re-polls `pending`; if the `fetch_sub` from step 2 is **not yet visible** to this
   load, it reads stale `> 0` and `park`s AGAIN — but the token was already consumed in step 3,
   and the worker issues NO further unpark (its single unpark fired in step 2). ⇒ the dispatcher
   is parked with the final decrement pending and no token.

This is the ONLY lost-wakeup shape, and it is bounded by the **`park_timeout` backstop already
present on BOTH dispatcher park sites**:
- `join_workers_until_drained`: `park_timeout(50µs)` (`scope.rs:519`) — the dispatcher re-polls
  every ≤50 µs and observes `pending == 0`.
- `executor_main_loop` Step 5: `park_timeout(100µs)` (`schedule.rs:461`) — same, ≤100 µs.

So liveness is GUARANTEED (no permanent hang): the timeout re-poll always observes the final
decrement (it is a single, monotone, eventually-visible store). The penalty is **at most one
timeout interval of added latency, only in the window**, and only when the window is actually
hit. The existing code comments already document the timeout as exactly this backstop ("the
backstop for the case where the wake-up raced ahead of our park call").

**Why the window is rare (probability argument).** The window requires the dispatcher to thread
a needle: consume the token (step 3), then issue a `pending` load (step 4) that lands in the
nanosecond-scale reorder gap between the worker's `unpark` and its `fetch_sub`-visibility, then
re-park before the store drains. On x86-TSO the store buffer drains in a handful of cycles; the
`fetch_sub` (a `lock xadd`, fully fenced) makes its result globally visible essentially
immediately after retiring. The dispatcher's re-poll `pending.load(Acquire)` after waking from
`park` involves a syscall-return path (hundreds of ns to µs) — far longer than the worker's
unpark→sub gap — so by the time the dispatcher re-reads `pending`, the decrement is
overwhelmingly already visible and it observes 0 and returns WITHOUT re-parking. The window is
therefore a deep-tail event; expected added latency ≈ P(window) × timeout ≪ 1 ns amortized.
Unconditional unpark (Decision 2) further shrinks it: every completion issues a token, so the
dispatcher accumulates wake opportunities across the wave, not just on the last decrement.

**Mitigation if ever needed (U', §11).** The peek-gated unpark does NOT shrink THIS window (a
correct peek still unparks before the sub); it only reduces spurious unparks. Neither variant
eliminates the window without moving the waker out of the box (Candidate K). Given the
backstop + the deep-tail probability, the window is accepted; K is the escalation only if a
latency-sensitive workload ever measures it.

### 9.3 loom impact (M1) — keep no-lost-wakeup; drop the freer election
The M1 model (`tests/loom_pool.rs::loom_m1_fork_join_no_lost_wakeup`) drives the real
`register_task`/`complete_task`/`is_drained`. Changes:
- `complete_task` now returns `()` — DELETE the `if shared_cl.complete_task() {
  worker_frees_cl.fetch_add(...) }` and the entire `worker_frees` / `dispatcher_swap_frees` /
  `total_freers == 1` block (the handshake it modeled is gone). The `LoomScopeShared::
  complete_task` shim reverts to `()`; DELETE `dispatcher_swap_frees`.
- KEEP the no-lost-wakeup core: register N, spawn N worker threads each calling
  `complete_task()`, the join loop `while !is_drained() { park() }`, and the invariants
  `is_drained()` at exit + `completed == N`.
- **loom modeling note (loom #246).** loom does NOT persist an unpark-before-park token (it
  reports a deadlock if `unpark` precedes `park`). With Candidate U's unpark-BEFORE-decrement,
  a model that lets a worker `unpark()` before the joiner reaches `park()` could trip loom's
  false deadlock — NOT a real bug, a loom limitation. Two model-faithful options, in order of
  preference:
  - **(Preferred) Model the wake edge as the `pending` store the joiner re-polls, exactly as
    M1 already does.** M1's join loop is `while !is_drained() { park() }`: loom explores the
    interleaving where each worker's `complete_task` (hence its `pending.fetch_sub`) is observed
    by the joiner's `is_drained()` re-poll. The `unpark` itself need not be loom-the-deadlock-
    detector's wake signal — loom advances the joiner thread and re-polls `is_drained()` across
    interleavings regardless of park/unpark (loom's scheduler is exhaustive over thread steps,
    not driven by unpark tokens). M1 ALREADY relies on this (its comment: "A lost wakeup here =
    a loom deadlock, not a hang"). Since U keeps the same `unpark` + `fetch_sub` pair (only the
    ORDER swaps), and loom does not model `park_timeout`, the model proves the protocol-level
    liveness: the joiner's `is_drained()` eventually observes 0 in every interleaving (the
    `fetch_sub` RMW is total-ordered; no interleaving leaves `pending > 0` permanently). If
    loom's deadlock detector trips on the order swap, switch the model's joiner to a bounded
    `for _ in 0..K { if is_drained() { break } thread::yield_now() }` (mirroring the production
    `park_timeout` re-poll as a loom-visible yield, the SAME technique M2/M2b/M3 use for the
    deque transport) — this models the timeout backstop loom cannot represent and proves
    termination without depending on the unpark token.
  - **(Fallback) Keep `park()` but assert termination via the bounded-yield form above.** This
    is the §9.2 backstop expressed in loom terms.
- M2/M2b/M3 UNCHANGED (idle bitset + shutdown; they don't touch `pending`/`waker`).

The decisive point vs the rejected Candidate B (remove waker entirely): U KEEPS a real
`unpark` edge, so the COMMON-case wake is prompt and loom can still model liveness without
relying solely on a timeout it cannot represent. The backstop is the SAFETY net for the rare
window, not the primary wake — exactly the existing design intent.

---

## 10. Native-cost analysis vs the baseline

**Clean-machine baselines to preserve (±5%):**
- `phase9_schedule_run_50_exclusive_systems` ≈ 4.1 µs
- `phase9_par_iter_4096_entities` ≈ 20 µs
- `phase9_schedule_run_two_disjoint` ≈ 1.25 µs
- `phase9_schedule_run_empty` ≈ 6 ns
- `phase9_schedule_run_one_exclusive` ≈ 242 ns

**Per task:** the hot `pending.fetch_sub(AcqRel)` is byte-identical (same op, same ordering).
The `prev == 1` branch is REMOVED (one fewer compare+branch — a micro-win). The `waker.unpark()`
moves 2 lines earlier and becomes UNCONDITIONAL (was gated on `prev == 1`). Net per-completion
delta: **−1 branch, +(unpark on the N−1 non-last completers that previously skipped it)**.

**Cost of the extra unparks (Decision 2).** `Thread::unpark` on a thread that is NOT parked is
a single atomic token-set on the parker's inner state (no syscall, no context switch). The two
scope kinds:
- **`par_iter` inner scope** (thousands of chunk completions): the inner-scope dispatcher is the
  worker that called `par_iter`; while waiting it is BUSY-STEALING inside
  `join_workers_until_drained` (it parks only after exhausting `Backoff` with no stealable work
  — rare under a full chunk queue). So nearly every extra `unpark` targets a non-parked thread
  ⇒ a cheap atomic token-set, not a syscall. Expected impact on `phase9_par_iter_4096_entities`:
  within noise (the chunk loop and the `pending` RMW are untouched; the added cost is N cheap
  token-sets amortized over the whole scope). **Target: ±5%, expected ≈ noise.**
- **executor scope** (few systems/wave): the dispatcher IS often parked between waves, but
  there the wake is NEEDED (the wave-last `unpark` was already firing). The extra unparks are
  the non-last completers within a wave (typically 1–7) — negligible. **Target: ±5%.**

**Per scope:** the box SHRINKS (no `free_state` `AtomicU8` / its `CachePadded`). One fewer
atomic swap on the dispatcher (the handshake swap is DELETED). One fewer `Cell` write per
`spawn` (the `spawned_any.set(true)` is DELETED). All wins.

**`phase9_schedule_run_empty` (6 ns):** zero tasks ⇒ `complete_task` never runs; `Scope::drop`
frees immediately. No change vs baseline (the deleted `spawned_any` read is a micro-win).

**Verification (§12):** A/B the full `phase9_scheduler` (ALL 5) pre/post within ±5%; confirm
ZERO `0xc0000374`; diff-inspect that the only hot-loop change is `complete_task`'s two-line
reorder + branch removal; `bench_bevy_vs_boyko` GROUP 3 (`par_iter` 10k) within noise to catch
the spurious-unpark cost at scale.

---

## 11. Candidate ranking + the U' / K fallbacks

**Primary: Candidate U (unconditional unpark-before-decrement).** Minimal, multi-drain-safe by
construction, UAF-free, a NET DELETION of machinery, keeps the `NonNull` fix and a real
loom-visible wake edge. Satisfies all six constraints.

**Optional optimization: Candidate U' (peek-gated unpark)** — apply ONLY if §10's
`phase9_par_iter` A/B shows the unconditional unpark costs >5%:
```rust
fn complete_task(&self) {
    // Peek (a box load, BEFORE the decrement ⇒ ordered-before the free, §5).
    if self.pending.load(Ordering::Relaxed) == 1 {
        self.waker.unpark();   // likely-last ⇒ near-zero spurious unparks
    }
    self.pending.fetch_sub(1, Ordering::AcqRel);
}
```
Race-freedom: the peek-`load` is a box access strictly before `fetch_sub` (this worker's last
access); the free is ordered after every worker's `fetch_sub` (§5), hence after the peek.
Correctness does NOT depend on the peek being right (a wrong peek ⇒ no unpark ⇒ the §9.2 timeout
backstop recovers). Trade-off vs U: fewer spurious unparks, but a slightly larger missed-wake
latency window (a wrong peek skips the unpark on the true-last decrement). Drop-in, no new
design round.

**Perf-clean fallback: Candidate K (stable waker out of the box)** — apply ONLY if BOTH U and
U' show measurable cost OR a latency-sensitive workload measures the §9.2 window. Move the wake
target into pool-stable storage so the worker performs NO box access after the decrement AND the
unpark stays last-only after the decrement targeting the stable waker. Sketch: a per-pool
bounded set of "joiner waiter slots" (a joiner = a dispatcher inside `install`/`scope` or a
worker inside a nested `Scope::drop`; bounded by `worker_count + external_callers`). The
dispatcher registers its `std::thread::Thread` into its slot before parking (publish with
Release); `complete_task`, on the wave-last decrement (`prev == 1`, now safe because the waker
is NOT in the box), reads the slot (Acquire) and unparks. The box no longer holds a `waker` at
all. More machinery (slot allocation, publish/observe sync, slot lifecycle) for behavior
identical to U on boyko's workload — DEFERRED, documented as the long-term answer if a global
eventcount/sleep ever lands.

**Rejected: the `free_state` handshake** (multi-drain double-free, §3) and **Candidate H2**
(per-wave reset; most complex/bug-prone). Both unnecessary given U.

---

## 12. Verification gate (CRITICAL — the gap that let the double-free through)

The single-drain `miri_scope`/loom/stress suite PASSED the broken handshake; only the
**multi-drain `phase9_scheduler` bench** exposed the double-free. The gate MUST therefore add a
multi-drain reproducer AND run the full bench. The fix is correct iff ALL pass:

### 12.1 `miri_scope` — 16-seed TB + data-race (boyko surface)
```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-many-seeds=0..16 \
  -Zmiri-permissive-provenance" \
  cargo +nightly miri test -p boyko-threadpool --test miri_scope
```
All 3 existing tests + the NEW `miri`-runnable multi-drain unit test (§11.4 / §12.4) GREEN. Re-doc
the module header: the data race is fixed by **Candidate U (unpark-before-decrement; single
unconditional free at `Scope::drop`)**, NOT the `free_state` handshake.

### 12.2 loom — M1 no-lost-wakeup (4/4)
```bash
RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test loom_pool
LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" \
  cargo test --release -p boyko-threadpool --test loom_pool
```
M1 reverted to no-lost-wakeup-only (§9.3); M2/M2b/M3 unchanged. 4/4.

### 12.3 stress — exactly-once (×5 stable)
```bash
cargo test --release -p boyko-threadpool --test stress
```
The Drop-accounting + post-join sweep is the native double-free/leak backstop; ×5 stable.

### 12.4 NEW multi-drain test — `scope_multi_drain_frees_once` (native AND Miri)
A scope whose `pending` is driven to 0 SEVERAL times before drop — the exact pattern the
handshake double-freed. Shape (add to `scope::tests`, runnable natively and under Miri):
```text
pool.install(|scope| {
    for wave in 0..4 {
        let done = AtomicUsize::new(0);          // borrowed by the wave's tasks
        for _ in 0..W { scope.spawn(|| { done.fetch_add(1, AcqRel); }); }
        // Drive THIS wave's `pending` to 0 via the real wake, mirroring the
        // executor's between-wave barrier: spin until `done == W`, then a brief
        // yield so the wave-last `complete_task` (unpark + fetch_sub) lands and
        // the scope's `pending` returns to 0 before the next wave's spawns.
        spin_until(|| done.load(Acquire) == W);
    }
    // Several `pending -> 0` transitions occurred; the scope is STILL alive here.
});  // <-- the ONLY free, at Scope::drop. A per-wave free would double-free here.
```
Assert every wave's tasks ran (`W*4` total). Under Miri (`-Zmiri-tree-borrows`,
`-Zmiri-many-seeds=0..16`) this proves no double-free/UAF ACROSS waves on the boyko surface.
This is the unit-level analogue of the bench regression — it would have caught the handshake
double-free.

> Note: a `scope`-only multi-drain test cannot perfectly reproduce the executor's park/unpark
> rhythm (the scope has no apply-window barrier), but driving `pending` to 0 repeatedly before
> drop IS the load-bearing invariant; combined with §12.5 (the real executor) the coverage is
> complete.

### 12.5 FULL `phase9_scheduler` bench — the regression oracle (ALL 5, exit 0)
```bash
cargo bench -p boyko-ecs --bench phase9_scheduler
```
**MUST complete exit 0 with NO `0xc0000374 STATUS_HEAP_CORRUPTION`.** This is THE oracle the
single-drain Miri/loom/stress missed — `phase9_schedule_run_two_disjoint` is the multi-drain
case that deterministically double-freed under the handshake. Record each bench's number;
compare to §10 baselines within ±5%.

### 12.6 native clean + regression
```bash
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo test -p boyko-threadpool
cargo test -p boyko-ecs --lib
cargo bench -p bench_bevy_vs_boyko    # GROUP 1 (50 systems) + GROUP 3 (par_iter): ±5% / noise
```

### 12.7 ECS 2-worker integrated executor under Miri
```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance" \
  cargo +nightly miri test -p boyko-ecs --test miri_schedule_parallel
```
Un-ignore ONLY if boyko-frame-clean. **Carried-forward policy (Phase 9.1):** the
`miri_schedule_parallel` test is ALSO `#[ignore]`d for a SEPARATE pre-existing reason — the ECS
executor wait-loop is not Miri-cooperative (no `#[cfg(miri)]` yield), so it livelocks under
Miri's deterministic scheduler (a Phase 9.3 follow-up, NOT a 9.2 gap). If it trips inside a
`crossbeam-*` frame (exposed-provenance), keep it `#[ignore]`d with a crossbeam reason and treat
`miri_scope` as the boyko-surface gate. A `scope.rs`/`schedule.rs`-frame failure means NOT done.

**`#[ignore]`s removed on green:** `miri_scope.rs` ×3 (unconditional, already removed by the
prior 9.2 edit — keep removed); `miri_schedule_parallel.rs` ×1 stays ignored (Miri-livelock
reason, §12.7). Module docs in `miri_scope.rs`, `miri_schedule_parallel.rs`, `miri_phase9.rs`
updated to cite **Candidate U** (unpark-before-decrement) instead of the `free_state` handshake.

---

## 13. Implementation plan (for the developer)

Work SEQUENTIALLY on `scope.rs`; re-run the gate yourself (Phase 9.1 lesson: compiler + Miri +
bench exit codes are the only oracles; never parallel-batch git+build+edit on these files).

1. **`src/scope.rs`** — DELETE `FREE_RUNNING`/`FREE_JOINED` consts; remove `free_state` from
   `ScopeShared` + its `new` initializer; remove `AtomicU8` from the `crate::sync` import (§7.1).
2. **`src/scope.rs`** — rewrite `complete_task` to `()` with `waker.unpark()` BEFORE
   `pending.fetch_sub(AcqRel)` (§7.2). Remove the `prev == 1` branch.
3. **`src/scope.rs`** — `spawn` wrapper tail: `shared.complete_task();` with NO `must_free`
   branch and NO worker-side `Box::from_raw` (§7.3). Remove `SharedPtr::as_ptr` if now dead.
4. **`src/scope.rs`** — remove `spawned_any: Cell<bool>` from `Scope`, its `new` init, and the
   `spawn` set; remove the `use core::cell::Cell;` import if unused (§7.4).
5. **`src/scope.rs`** — rewrite `Scope::drop`: join → read+take payload + `debug_assert!
   (is_drained())` via raw deref → UNCONDITIONAL `Box::from_raw(raw)` → `resume_unwind` last
   (§7.5). `join_workers_until_drained` UNCHANGED (§7.6).
6. **`src/sync.rs`** — remove the `AtomicU8` re-export from both cfg arms (no longer used) — or
   leave it (harmless); recommend REMOVE for cleanliness. If removed, confirm `cargo build
   --release` + `--cfg loom` both still compile (no other `AtomicU8` user).
7. **`src/lib.rs` `loom_exports`** — `LoomScopeShared::complete_task` reverts to `()`; DELETE
   `dispatcher_swap_frees` and the `FREE_JOINED` reference (§9.3).
8. **`tests/loom_pool.rs`** — M1: delete the `worker_frees` counter, the
   `if complete_task() {…}`, the `dispatcher_swap_frees` call, and the `total_freers == 1`
   assert; keep register-N + N workers calling `complete_task()` + the join loop + the
   `is_drained()`/`completed == N` invariants (§9.3). If loom's deadlock detector trips on the
   unpark-before-decrement order, switch the joiner to the bounded-yield re-poll form (§9.3).
9. **`scope::tests`** — add `scope_multi_drain_frees_once` (§12.4); keep
   `scope_zero_tasks_frees_no_leak`, `scope_single_task_completes`,
   `scope_inline_drain_frees_once`, `nested_scope_does_not_deadlock`, `scope_propagates_panic`,
   `scope_spawn_can_borrow_stack_data`, `scope_drain_with_no_tasks_is_noop`. Update their
   doc-comments to drop "handshake"/"second swapper"/"free_state" wording → "single
   unconditional free at `Scope::drop` (Candidate U)".
10. **Doc-update** `tests/miri_scope.rs`, `crates/boyko_ecs/tests/miri_schedule_parallel.rs`,
    `crates/boyko_ecs/tests/miri_phase9.rs` module headers: cite Candidate U
    (unpark-before-decrement; single free at `Scope::drop`), NOT the `free_state` handshake.
11. **Run the FULL gate (§12) yourself** — in particular §12.5 (the full `phase9_scheduler`
    bench, ALL 5, exit 0, NO `0xc0000374`) is the regression oracle that the single-drain
    Miri/loom/stress missed. A/B vs §10 baselines within ±5%.

---

## 14. Metrics, tests, and `debug_assert!` invariants

**`debug_assert!`s:**
- KEEP `debug_assert!((*raw).is_drained(), "Scope::Drop returned with pending tasks still in
  flight")` in `Scope::drop` (now via raw deref, before the free).
- No double-free guard needed: there is a SINGLE free site (§6.3); Miri's own dealloc checker +
  the stress Drop-accounting catch any regression. (The handshake needed a freer-election
  invariant; U does not.)

**Mandatory tests:** §12.4 (`scope_multi_drain_frees_once`, native + Miri) is the NEW
load-bearing test. Existing `scope::tests`, `stress.rs`, loom M1 (revised, §9.3), `miri_scope`
(3 + new), all green.

**Mandatory benches:** §12.5 (`phase9_scheduler` ALL 5, exit 0, ±5%) — the regression oracle;
`bench_bevy_vs_boyko` GROUP 1 + GROUP 3 within ±5%/noise.

---

## 15. Open questions for the critic

1. **Unconditional unpark (U) vs peek-gated (U', §11).** I recommend U (smallest lost-wakeup
   window + simplest + smallest loom surface; the spurious-unpark cost is a cheap non-syscall
   token-set because the inner-scope dispatcher is busy-stealing). U' is the drop-in if §10's
   `phase9_par_iter` A/B shows >5%. Confirm U is acceptable as primary, with U' as the
   measured-fallback (no new design round).
2. **loom M1 under unpark-before-decrement (§9.3).** loom #246 means unpark-before-park is not
   token-persisted; I argue M1 still proves liveness because loom's exhaustive scheduler
   advances the joiner and re-polls `is_drained()` independent of the unpark token (M1 already
   relies on this), and the bounded-yield re-poll form is the documented fallback if the
   detector trips. Confirm this division (loom proves protocol liveness via the `pending` RMW
   order + re-poll; the native `park_timeout` backstop covers the rare window loom cannot
   represent) is acceptable.
3. **The §9.2 lost-wakeup window acceptance.** I accept a bounded ≤50 µs/100 µs added latency in
   a deep-tail window (backstopped by the EXISTING `park_timeout`s, no new code), rather than
   escalate to Candidate K's pool-stable waiter slots now. Confirm this is the right
   cost/complexity trade, or whether K should be in-scope for 9.2 (I argue NO — K is identical
   behavior for materially more `unsafe`/machinery, deferred until a workload measures the
   window).

---

## Plan readiness checklist

**Structure:** goal in perf+functional terms ✓; concrete metrics (0 added contended atomics,
box shrinks, ±5%, exit-0 bench) ✓; every decision justified via perf/cache/parallelism ✓;
alternatives (handshake, H2, K, U', `prev==1` gate, remove-waker) rejected with reasons ✓;
trade-offs (spurious unpark + bounded lost-wakeup window) honestly listed ✓.
**Data structures:** every field typed + role-commented ✓; `#[repr(C)]` kept; `free_state`
REMOVED (box shrinks) ✓; hot `pending` line unchanged ✓; the multi-wave constraint (§3) drives
the whole design ✓.
**API:** no public change ✓; `complete_task` reverts to `()` (internal) ✓; lifetimes unchanged
(`'scope`) ✓; no `dyn` in hot path ✓.
**Multithreading:** model explicit (multi-writer `pending`; single free at `Scope::drop`) ✓;
every ordering stated + justified (`pending.fetch_sub(AcqRel)` unchanged; `is_drained` Acquire
synchronizes-with the FINAL-wave decrement) ✓; happens-before free-after-last-access proof
covering the MULTI-WAVE oscillation (§5 Lemmas U1–U3 + Theorem) ✓; lost-wakeup/liveness analysis
(§9) ✓; `Send`/`Sync` consistent (`SharedPtr: Send` unchanged; `Scope` loses `Cell`, stays
`!Sync` via `NonNull`) ✓.
**Correctness:** edge cases enumerated + resolved (zero tasks, inline-on-dispatcher, single free
site, panic-path payload-before-free, nested scopes, MULTI-WAVE) ✓; Drop order (payload read
before free; `resume_unwind` last) ✓; every `unsafe` has SAFETY invariants (§7.3/§7.5) ✓.
**Integration:** affected modules listed (`scope.rs` logic; `sync.rs` symbol; `lib.rs` shim;
loom M1; 3 test-doc files) ✓; existing API unchanged ✓; landed `NonNull` fix preserved (provides
TB cleanliness; U adds NO new reference + REMOVES one atomic) ✓; step-by-step plan (§13) ✓.
**Validation:** unit tests incl. the NEW multi-drain reproducer (§12.4) ✓; loom no-lost-wakeup
(§9.3) ✓; the FULL `phase9_scheduler` bench as the regression oracle (§12.5) ✓; exact gate
commands + which `#[ignore]`s change (§12) ✓; `debug_assert!`s (§14) ✓.

---

**Sources consulted:**
- [std `thread::scoped.rs` — finisher reads `main_thread` AFTER `fetch_sub(Release)`; the `Arc`
  comment "so that other threads can finish their decrement … even after this function returns"
  is the lifetime-extension U re-supplies without an Arc](https://doc.rust-lang.org/nightly/src/std/thread/scoped.rs.html)
- [Data race in `thread::scope` (rust-lang/rust#98498) — why the Arc is load-bearing](https://github.com/rust-lang/rust/issues/98498)
- [rayon `latch.rs` — "read all the fields you will need before a latch is set… the target may
  proceed and invalidate `this`" (the read-before-signal discipline U applies to `unpark`)](https://github.com/rayon-rs/rayon/blob/main/rayon-core/src/latch.rs)
- [loom #246 — `unpark`-before-`park` is NOT token-persisted (loom reports deadlock); informs the
  M1 modeling choice in §9.3](https://github.com/tokio-rs/loom/issues/246)
- [rayon `sleep/mod.rs` — pool-lifetime sleep states (Candidate K's model)](https://github.com/rayon-rs/rayon/blob/main/rayon-core/src/sleep/mod.rs)
```

---

## Summary for the orchestrator

**Decision: Candidate U (unpark-before-decrement).** It is the simplest fix and the only one that is multi-drain-safe *by construction* — it ties the single `Box::from_raw` to `Scope::drop` (scope END), never to a `pending -> 0` event, so the executor's per-wave oscillation of `pending` (the confirmed root cause of the `0xc0000374` double-free) cannot trigger a premature free. It is a net **deletion** of the crashing machinery (the `free_state` `AtomicU8`, `FREE_RUNNING`/`FREE_JOINED`, `spawned_any: Cell`, the `complete_task -> bool` return, and the worker-side `Box::from_raw`), keeping the landed `NonNull<ScopeShared>` TB-fix untouched.

**Why it is sound (the property the handshake lacked):** moving `waker.unpark()` *before* `pending.fetch_sub` makes the decrement each worker's LAST byte-access to the allocation; the free at `Scope::drop` (after the join's Acquire load reads the FINAL wave's 0) is ordered after every worker's last access via the `pending` RMW total order. I proved this explicitly across the multi-wave pattern (§5 Lemmas U1–U3 + Theorem).

**The two costs, both accepted with analysis:** (a) unconditional unpark — cheap non-syscall token-set because the inner `par_iter` dispatcher is busy-stealing, not parked; (b) a deep-tail lost-wakeup window — backstopped by the EXISTING `park_timeout` (50µs in the scope join, 100µs in the executor), no new code, bounded latency, never a hang.

**Verification gate closes the exact gap that let the bug through:** I added (1) a NEW native+Miri `scope_multi_drain_frees_once` reproducer that drives `pending` to 0 several times before drop, and (2) the mandatory FULL `cargo bench -p boyko-ecs --bench phase9_scheduler` (ALL 5, exit 0, no `0xc0000374`) as the regression oracle — the single-drain Miri/loom/stress suite that passed the broken handshake is explicitly called out as insufficient.

The complete plan content above must be persisted verbatim to `D:\claude\BoykoEngine\docs\PHASE-9.2-PLAN.md` (overwriting the current broken-handshake plan). My tool set has no Write capability, so the orchestrator must perform the write.

Relevant files (absolute paths):
- `D:\claude\BoykoEngine\crates\boyko_threadpool\src\scope.rs` — all logic edits (`ScopeShared` l.52-80, `complete_task` l.126-137, `spawn` wrapper tail l.296-338, `Scope` struct l.209-236, `Scope::drop` l.370-435, `join_workers_until_drained` l.457-525 unchanged).
- `D:\claude\BoykoEngine\crates\boyko_threadpool\src\sync.rs` — remove the now-unused `AtomicU8` re-export (l.52, l.62).
- `D:\claude\BoykoEngine\crates\boyko_threadpool\src\lib.rs` — `loom_exports` shim: revert `complete_task` to `()` (l.122-124), delete `dispatcher_swap_frees` (l.138-144).
- `D:\claude\BoykoEngine\crates\boyko_threadpool\tests\loom_pool.rs` — M1 (l.73-158): drop the freer-election, keep no-lost-wakeup.
- `D:\claude\BoykoEngine\crates\boyko_threadpool\tests\stress.rs`, `D:\claude\BoykoEngine\crates\boyko_threadpool\tests\miri_scope.rs` — gate + new multi-drain test + doc updates.
- `D:\claude\BoykoEngine\crates\boyko_ecs\benches\phase9_scheduler.rs` — the regression oracle (run all 5).
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\schedule.rs` — `executor_main_loop` (l.361), the per-wave `park_timeout(100µs)` at l.461, `PARK_TIMEOUT` at l.61 (the wave model + backstop, read-only context).
- `D:\claude\BoykoEngine\crates\boyko_ecs\tests\miri_schedule_parallel.rs`, `D:\claude\BoykoEngine\crates\boyko_ecs\tests\miri_phase9.rs` — doc updates to cite Candidate U.
- `D:\claude\BoykoEngine\docs\PHASE-9.2-PLAN.md` — overwrite target.

