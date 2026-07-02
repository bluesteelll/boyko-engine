> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9.1 — Soundness Verification of the Parallel Executor

**Status:** Architecture plan, ready for implementation.
**Type:** Verification phase — no new runtime features. Every production-code edit must be byte-identical on the native (non-loom, non-miri) build.
**Inputs consumed:** `docs/PHASE-9.1-RESEARCH.md` (ground truth); `boyko_threadpool/src/{scope,worker,thread_pool,tls,lib}.rs`; `boyko_ecs/src/ecs/core/schedule/schedule.rs`; both `Cargo.toml`s; existing `crates/boyko_ecs/tests/miri_*.rs` conventions; `.cargo/config.toml`.

---

## 0. Ground-truth findings (verified in source)

- **`scope.rs`**: `ScopeShared { pending: CachePadded<AtomicUsize>, panic_payload: Mutex<Option<Box<dyn Any+Send>>>, waker: Thread }`. `spawn`: `pending.fetch_add(1, AcqRel)` (:158); body wrapper `catch_unwind` → `fetch_sub(1, AcqRel)` → `if prev==1 { waker.unpark() }` (:193-196); lifetime erasure `core::mem::transmute::<Box<dyn FnOnce()+Send+'scope>, Box<dyn FnOnce()+Send+'static>>` (:223). `join_workers_until_drained` (:263-322): `pending.load(Acquire)` poll, work-steal from `injector_local`/`injector_global`/`stealers`, on backoff exhaustion `unpark_one_idle` + `std::thread::park_timeout(50µs)`. `drain_one` (:344-356) loops `Steal::Retry` **with no yield**. `SharedPtr(*const ScopeShared)` + `unsafe impl Send` (:117); `as_ref` raw deref (:103-106).
- **`worker.rs`**: `worker_main` — pop local → steal → `Backoff::snooze` → **`mark_idle` then re-poll all sources ("Race C") then `park()`**. `mark_idle`/`unmark_idle`: `idle.fetch_or/fetch_and(bit, Release)`. `unpark_one_idle`: `idle.load(Acquire)` → `compare_exchange_weak(…, AcqRel, Acquire)` → `thread.unpark()`. A second `drain_one` `Steal::Retry` loop, **no yield**.
- **`thread_pool.rs`**: `ThreadPool { workers, stealers, injector_global, injector_local, idle: CachePadded<AtomicU64>, active_scopes: CachePadded<AtomicUsize>, shutdown: CachePadded<AtomicBool>, … }`. `WorkerHandle { thread: Thread, park_state: CachePadded<AtomicU64> }` — **`park_state` is `#[allow(dead_code)]`, never read/written** (dormant). `Drop`: `shutdown.store(true, Release)` → unpark all → join. Bootstrap `Mutex<Option<…>> + Condvar` (cold). All atomics imported directly from `std::sync::atomic` — **zero loom scaffolding**. `ThreadPoolBuilder::num_threads(n)` clamps `[1, MAX_WORKERS]`, always spawns `n` OS threads.
- **`schedule.rs`**: `Schedule::run` → `self.pool.install(|scope| { … scope.spawn(…) … })`. The executor body is **not** blanket `#[cfg(not(miri))]` in source; the **integration tests** that drive the full multithreaded schedule are gated `#![cfg(not(miri))]` (the `miri_*.rs` files exercise single-threaded paths only).
- **Conventions**: Miri tests in `crates/boyko_ecs/tests/miri_*.rs`; `.cargo/config.toml` sets `-Zmiri-tree-borrows` (TB is the project default). `boyko_threadpool` has in-module `#[cfg(test)]` unit tests, **no `tests/` dir, no Miri tests**.

This confirms the research: deque = crossbeam (loom-opaque), join counter = boyko's own (loom-able), transmute needs Miri-TB on real threads, native imports `std` atomics directly (shim = pure additive `use`-swap).

---

## 1. Scope

**In (proven this phase):**
1. **loom** exhaustive models of boyko's *own* primitives (deque abstracted out): (a) `pending`-counter fork/join no-lost-wakeup, (b) idle-bitset park/unpark "Race C" re-poll, (c) `shutdown` handshake.
2. **Miri** (Tree Borrows + data-race + `-Zmiri-many-seeds`) on the **real pool at 2 workers**, driving `Scope::spawn` with bodies that borrow stack data — proving the lifetime-erasure `transmute` + cross-thread `SharedPtr` deref are TB-clean & race-free, and 2-worker `Schedule::run` is UB-free.
3. **Stress** (native, randomized, Drop-accounting) proving deque **exactly-once / no-loss / no-double-run / no-leak** across steal races.
4. Any **real soundness bug** found → fixed and re-verified (expect none; the code mirrors `std::thread::scope`/rayon).

**Out (§9):** shuttle; Miri+GenMC/RustMC; Miri >2 workers; loom on the crossbeam deque (impossible); any perf/throughput change.

---

## 2. Decisions

### D1 — loom shim: a small, crossbeam-free `sync` module (not whole-crate)
Add `crates/boyko_threadpool/src/sync.rs`:
```rust
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU64, AtomicUsize, AtomicBool, Ordering, fence};
#[cfg(loom)]
pub(crate) use loom::sync::{Arc, Mutex, Condvar};
#[cfg(loom)]
pub(crate) use loom::thread::{self, Thread};
// NOTE (H1): no `UnsafeCell` in the shim — `boyko_threadpool` uses none
// (only `std::cell::Cell` in tls.rs, thread-local, not a loom target). If a
// future loom toy-queue needs a cell, use `loom::cell::UnsafeCell` with its
// `.with()`/`.with_mut()` API (NOT a raw deref) — that is NOT a pure use-swap.

#[cfg(not(loom))]
pub(crate) use core::sync::atomic::{AtomicU64, AtomicUsize, AtomicBool, Ordering, fence};
#[cfg(not(loom))]
pub(crate) use std::sync::{Arc, Mutex, Condvar};
#[cfg(not(loom))]
pub(crate) use std::thread::{self, Thread};
```
The loom **models** (in `tests/loom_pool.rs`, `#![cfg(loom)]`) drive the **real** production synchronization methods (C1, see D2/D3) over loom primitives, substituting only a trivial loom-visible stand-in queue for the crossbeam deque transport. This sidesteps deque opacity (research #1). Production files swap `use std::sync::atomic::X` → `use crate::sync::X` (pure rename). **Zero-native-cost:** when `cfg(loom)` is off, `crate::sync::AtomicUsize` *is* `core::sync::atomic::AtomicUsize` — a compile-time alias, byte-identical codegen, no indirection. **Rejected** whole-crate shim: it would drag crossbeam (loom-opaque, SeqCst false alarms) into the loom build and bloat production files.

**Which shimmed types each model touches (H1):** M1 touches `AtomicUsize` (the `pending` counter via `complete_task`/`is_drained`) + `loom::sync::{Mutex, Condvar}` (the toy queue + the wait). M2a/M2b touch `AtomicU64` (the real `idle` bitset via `mark_idle`/`unpark_one_idle`). M3 touches `AtomicBool` (`shutdown`). **No model touches `UnsafeCell`** (none exists in the crate). Production has **no raw `UnsafeCell` deref**, so the shim is a genuine pure `use`-swap for atomics + `thread` — byte-identical native codegen. The `ScopeShared::panic_payload` `Mutex` is **cold** (panics only) and is **omitted from the M1 model**: it is not on the wakeup happens-before (`complete_task` does the counter+unpark before any panic-payload interaction at `Scope::drop`), so omitting it cannot hide a wakeup race.

**`cfg(loom)` lint registration (M3).** The `unexpected_cfgs` `check-cfg` entry for `cfg(loom)` goes in **`boyko_threadpool`'s** `Cargo.toml` `[lints.rust]` (or `[workspace.lints]` if the workspace centralizes lints — match the existing convention) so native `-D warnings` stays clean on the `#[cfg(loom)]` attributes. Verify `cargo build` / `cargo tree` **without** `--cfg loom` shows **no** `loom` dependency (the `[target.'cfg(loom)'.dependencies] loom` is cfg-gated and must never enter the normal build graph).

### D2 — loom join model: drive the REAL synchronization methods (shared code, not a re-implementation)
Do **not** `cfg(loom)`-rewrite `join_workers_until_drained` whole (it is wedded to crossbeam + `park_timeout`, neither loom-able). But do **not** re-implement the synchronization in the loom test either — a model that re-implements the `pending`/`waker` protocol proves the *model*, not `scope.rs`, and the two silently drift when production orderings change. Instead, **extract the synchronization primitives into tiny `pub(crate)` methods that BOTH production and the loom models call** (C1). The work-stealing *transport* (the crossbeam deque) is the only thing the model legitimately abstracts — it is loom-opaque and Coq-verified upstream.

**Extraction (zero-native-cost, `#[inline]`).** Add to `ScopeShared` three methods wrapping the exact existing atomics, routed through the `crate::sync` shim:
- `#[inline] pub(crate) fn register_task(&self)` → `self.pending.fetch_add(1, AcqRel)` (was scope.rs:158 inline).
- `#[inline] pub(crate) fn complete_task(&self)` → `let prev = self.pending.fetch_sub(1, AcqRel); if prev == 1 { self.waker.unpark() }` (was scope.rs:193-196 inline). The panic-payload capture stays in the body wrapper (it is cold and NOT on the wakeup happens-before — see H1); `complete_task` is only the counter+wakeup.
- `#[inline] pub(crate) fn is_drained(&self) -> bool` → `self.pending.load(Acquire) == 0` (was scope.rs:276 inline).

Production `Scope::spawn` calls `register_task()` / (in the body wrapper) `complete_task()`; `join_workers_until_drained` calls `is_drained()` in its poll. Because these are `#[inline]` `pub(crate)` wrappers over the identical atomic ops, native codegen is byte-identical (the compiler inlines them away — verified by §6).

**The loom M1 model calls these exact methods** over the `crate::sync` shim (so loom sees the real `AcqRel`/`Acquire` orderings), substituting only: (a) a trivial loom-visible toy queue (`Vec`/`VecDeque` behind a `loom::sync::Mutex`) for the deque transport, and (b) a `loom`-blocking wait (`loom::sync::Condvar` or `loom::thread::park`) for the production `park_timeout` (loom has no `park_timeout`; the production timeout is a backstop that masks a lost wakeup as latency, not a hang — so the *ordering*, which loom proves via the shared `complete_task`/`is_drained`, is the real correctness condition). This makes "same orderings" a structural guarantee (shared code), not a comment. A future weakening of `fetch_sub(AcqRel)` would change `complete_task` and the loom proof would re-run against it automatically.

### D3 — what loom proves (3 models, 2-3 threads, `LOOM_MAX_PREEMPTIONS=2-3`)
In `crates/boyko_threadpool/tests/loom_pool.rs`:
- **M1 fork/join no-lost-wakeup (drives `ScopeShared::register_task`/`complete_task`/`is_drained`):** the model constructs a real `ScopeShared` over loom atomics; main calls `register_task()` N=2 times then runs the join-wait loop using `is_drained()` (blocking on `loom::thread::park`, the transport being a toy loom queue); 2 task threads pop a toy task + call `complete_task()`. A shadow `completed` counter sits beside the real atomics. *Invariants:* the join loop exits ⟺ `is_drained()` (i.e. real `pending==0`); `completed==N` at exit (no task outlives join → the transmute's premise); always terminates (a lost wakeup in `complete_task`'s `prev==1` branch = loom deadlock report). Because the model calls the same methods production calls, a future ordering regression in those methods fails M1.
- **M2 idle-bitset "Race C" (drives the real `mark_idle`/`unmark_idle`/`unpark_one_idle`):** the model uses the real `worker.rs` functions over a loom `AtomicU64` (and, for `unpark_one_idle`, either a 1-worker `ThreadPool` or the extracted `claim_one_idle` core — H4). **Thread count = 2** (1 worker + 1 producer) — sufficient for the single-bit lost-wakeup property: worker does re-poll→`mark_idle`→re-poll→park vs producer push-work-flag+`unpark_one_idle`. *Invariant:* the worker never ends parked with unclaimed work present (the post-`mark_idle` re-poll closes the window). **M2b variant, thread count = 3** (2 idle workers + 1 producer): exercises the real `compare_exchange_weak` *which-worker* contention in `unpark_one_idle` (`worker.rs:233-235`) — a genuine concurrency path, so it is **in scope**. *Invariant (M2b):* exactly one worker is unparked per `unpark_one_idle` success (the CAS loser retries and observes the cleared bit; no double-wake, no lost-wake). `LOOM_MAX_PREEMPTIONS=2` for M2, `=3` for M2b (state space grows; raise only if the model under-explores).
- **M3 shutdown handshake:** `shutdown.store(Release)`+unpark vs worker `shutdown.load(Acquire)` after re-poll. *Invariant:* every worker exits after shutdown (none parks forever).

### D4 — Miri strategy: a `cfg(miri)` 2-worker integrated path
- **Miri-1** `crates/boyko_threadpool/tests/miri_scope.rs` (`#![cfg(miri)]`): 2-worker pool; `pool.install(|scope| { … scope.spawn(borrows distinct stack slots) … })` (the std::thread::scope doctest shape) + a spawn reading-then-writing a stack `AtomicUsize`. Drives the `transmute` (:223), cross-thread `SharedPtr::as_ref` (:103-106), `pending` join, `ThreadPool::drop` shutdown+join on **real threads under TB**. *Asserts:* no Miri UB, no data race, no UAF (a buggy early-return join → UAF on freed `ScopeShared`/stack), correct results.
- **Miri-1 must FORCE + ASSERT genuine worker-thread execution (H4).** Because `Scope::drop`'s wait steals and runs tasks **inline on the dispatcher**, a small task set can be drained entirely by the dispatcher — so the worker-side `SharedPtr::as_ref` write (the TB target) is never exercised and the test passes *vacuously*. To force a real cross-thread run: a task body **blocks on a second atomic/flag set by a different task** (or an N-task `Barrier`) so the dispatcher cannot complete it alone; AND the test **records the executing worker id** (`tls::current_worker_id()`) per body and **asserts at least one body ran on a worker thread distinct from the dispatcher** — a vacuous all-inline pass is then a *test failure*, not a false green.
- **Miri-2** `crates/boyko_ecs/tests/miri_schedule_parallel.rs` (`#![cfg(miri)]`): kept — it covers a surface Miri-1 does **not**, namely the ECS `UnsafeEcsCell` shared across two workers inside the real `Schedule::run` executor (distinct from Miri-1's pool-only surface). Scoped to the **absolute minimum** so CI does not time out: **exactly 2 systems, 1-2 entities, 1 frame**, over disjoint components (so the two systems run concurrently — the point is the cross-worker cell sharing, not throughput). Seeds: **single-seed on first landing**, then `-Zmiri-many-seeds=0..8` once green. **Runtime budget: target < ~3 min wall under Miri** on CI; if it exceeds, drop to 1 system + the 2-worker pool spin-up alone (still exercises the cross-worker `UnsafeEcsCell` handshake) before cutting seeds further. *Asserts:* the integrated executor's `UnsafeEcsCell` sharing across 2 workers is TB-clean, no UB/race.

**Production edits — Miri-visible yields in EVERY Miri-reachable unbounded loop (all zero-native-cost, H2).** Miri's scheduler is cooperative: a thread that spins without an explicit yield or a blocking syscall can starve the others into an apparent hang. Every loop a Miri test thread can sit in gets a `#[cfg(miri)] crate::sync::thread::yield_now();` (compiles to nothing in native). Grep-verified enumeration:
- `drain_one` `Steal::Retry` (scope) `scope.rs:349-353` — insert in the `Retry` arm.
- `drain_one` `Steal::Retry` (worker) `worker.rs:183-187` — insert in the `Retry` arm.
- `join_workers_until_drained` outer loop `scope.rs:275-321` — insert at the **top of the loop body** (a steal-run-`continue` cycle must yield even when it never reaches the backoff/park branch).
- `worker_main` `'outer` loop `worker.rs:29-93` — insert at the **top of `'outer`** (uniform guarantee; task-running arms make progress, the no-work fall-through enters the inner backoff loop which already yields via `snooze`/`park`).

So **four** `#[cfg(miri)] yield_now()` inserts. The two inner backoff loops already yield (`Backoff::snooze`/`park`); the `unpark_one_idle` CAS loop is bounded by parked-worker count — no yield. **No** `ThreadPoolBuilder` clamp `cfg` (tests pass `num_threads(2)` explicitly — avoid a behavioral `cfg`-divergence).

**`park_timeout(50µs)` under Miri (`scope.rs:316`) — resolved.** Assumption (confirmed on first Miri run, recorded in results): Miri does not advance wall-clock, so it treats `park_timeout` as park-until-unparked (or returns spuriously — both safe here). The join's real wake-up is `complete_task`'s `prev==1` → `waker.unpark()` (the timeout is only a native latency backstop). Termination is guaranteed regardless of timer semantics: (a) the finite workload drives `pending→0`, (b) `complete_task` unparks the joiner, (c) `ThreadPool::drop` sets `shutdown`+unparks all. Fallback if `park_timeout` is unsupported under Miri: a `#[cfg(miri)]` swap to plain `park()` in that one branch (zero native cost, documented).

**MIRIFLAGS:** `-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-many-seeds=0..32` for `miri_scope`; `miri_schedule_parallel` runs single-seed first then `0..8` (M1 budget). TB stays the default. Tests do finite work then drop the pool (deterministic shutdown so Miri terminates).

### D5 — the `Scope::spawn` transmute: keep it, prove it
**Keep** the transmute. It's the rayon pattern (Miri-tested upstream), boyko's join is structurally identical to (and slightly stronger than) `std::thread::scope`, and the transmute changes only the (MIR-erased) lifetime — the data provenance is unchanged. The sole soundness question (no body outlives `'scope`) is **exactly** what loom-M1 (ordering) + Miri-1 (reborrow on a real worker) jointly prove. Avoiding it would need a non-erased or GAT-threaded deque (defeats the word-sized `TaskHandle` Phase-9 perf design; not zero-cost).

#### D5.1 — Miri-1 TB outcome decision tree (the phase MUST be green-able either way)
Miri-1's pass is **not** unconditionally guaranteed. The Phase-9 closeout deferred a *Tree-Borrows protected-tag conflict* — and per `crates/boyko_ecs/tests/miri_phase9.rs:16-26` the conflict is on **`ScopeShared` aliasing**, not the transmute per se: `Scope::drop` passes `&self.shared` (a real `&ScopeShared`, **protected** for the call's duration under TB) into `join_workers_until_drained`, while each worker reborrows `&ScopeShared` via `SharedPtr::as_ref` and **foreign-writes** `pending` (`fetch_sub`) — a foreign write to a location the dispatcher holds as a protected `Reserved`. TB may over-approximate this rayon/`std::thread::scope`-family pattern. So the plan defines "done" for **both** outcomes up front.

Run order for `miri_scope` (and the protected-tag is the **expected first result**, not a tail risk):
1. **Run under Tree Borrows first** (`-Zmiri-tree-borrows`, the project default).
   - **(i) Green** → the deferred note is **RESOLVED**; TB is authoritative; record the passing command+seeds. Done.
   - **(ii) Reproduces the protected-tag conflict** → discriminate real-bug vs TB-over-approximation:
     - **(a)** Re-run under **Stacked Borrows** (`-Zmiri-stacked-borrows`). SB-green + TB-red is the classic TB-strictness signature, not UB.
     - **(b)** **Minimize to a `std::thread::scope`-equivalent harness** (same spawn-borrows-stack shape, std's scope) under the same TB flags. **If STD'S OWN scope triggers the same TB flag, it is a TB limitation, not a boyko bug** (boyko's join is structurally identical to / slightly stronger than std's). Decisive test.
     - **(c)** Only a genuine UAF/aliasing violation (SB *also* red, or std clean while boyko red under SB) escalates to §7.
   - **Candidate real fix if (c) or if we choose to harden:** change `join_workers_until_drained` to take `*const ScopeShared` (raw) instead of `&ScopeShared`, so the dispatcher holds **no protected borrow** across worker writes (mirrors how std/rayon avoid a protected parent borrow). Zero-cost-checkable.

**Authoritative model:** boyko mandates TB project-wide → **TB authoritative when green (i)**. When TB is red *and* (a)+(b) show std's own scope is equally TB-red (ii), **SB becomes authoritative for this one transmute**, the TB result documented as a known over-approximation; the rest of the crate stays TB-unconditional.

**Phase "done" (both outcomes):**
- **(i) Green-under-TB:** all `miri_scope` passes under TB → soundness proven, deferred note closed.
- **(ii) TB-limitation-documented:** SB green + std-scope-equivalent equally TB-red → soundness proven under SB; the TB invocation is `#[ignore]`-with-reason (or split to an `_sb`-suffixed test run under SB in CI) so CI stays green; results doc records the over-approximation + the minimized std harness as evidence; the deferred Phase-9 note updated to "TB over-approximates; SB-clean; std::thread::scope equally flagged."
Only outcome **(c)** (a genuine violation) blocks the phase — and it has a defined fix path (§7). The first Miri run merely decides *which* documented "done" applies.

### D6 — exactly-once stress with Drop-accounting
`crates/boyko_threadpool/tests/stress.rs` (native, randomized). Each task carries an id; the body does run-once `ran[id].swap(true)==false`, and a Drop-counting payload does dropped-once `dropped[id].fetch_add(1)==0`. Mirror crossbeam-deque's **spsc / stampede / stress / destructors** via `pool.install`+`scope.spawn` under steal contention. **Mandatory post-join FULL-ARRAY sweep:** after `install` returns (all joined), assert for **every** `i in 0..N`: `ran[i]==true` **and** `dropped[i]==1`. The in-body asserts catch double-run/double-drop *when they happen*; only the post-join sweep catches **loss** (a task that never ran or never dropped). Both required. Add a **panic-path variant** (a fraction of tasks panic, caught at scope level) asserting survivors+panicked all satisfy run-once+drop-once and the first panic propagates — that is where wrapper double-drop/leak bugs hide (`scope.rs:183-188` unwind path). *Asserts:* exactly-once run + exactly-once drop + no loss + correct panic propagation. Covers the deque path loom can't reach and Miri can't scale to. Extend (don't duplicate) `spawn_100_tasks_via_scope_all_run` / `nested_scope_does_not_deadlock`. **`no_starvation` is best-effort, `#[ignore]`-by-default, NOT a CI gate** (crossbeam's own is timing-flaky); when run it is `Barrier`-synchronized and asserts only "every worker makes some progress," never a tight fairness ratio.

### D7 — drop the dormant `WorkerHandle::park_state`
Remove the never-used `#[allow(dead_code)] park_state: CachePadded<AtomicU64>` (`thread_pool.rs:66`) — 64 B/worker wasted + reader-confusion. Zero-cost removal. **Before removing, the developer (a) re-confirms zero `park_state` references crate-wide (grep), and (b) greps for any `size_of::<WorkerHandle>()` / `align_of` / layout assertion on `WorkerHandle` and updates/removes it** — none exists today (only `#[repr(C)]`, no size assert), but the check is mandatory because shrinking the struct would silently break such an assert. Also drop the `WorkerHandle::new` `park_state` initializer (`:74`) and any now-unused `CachePadded`/`AtomicU64` imports. If any live reference is found, keep the field and downgrade D7 to "documented dormant."

### D8b — `active_scopes` is diagnostic-only (closed surface, L2)
`active_scopes` (`thread_pool.rs`) is **not soundness-critical**: source-confirmed it is only `fetch_add`/`fetch_sub`-ed around `install`/`scope` and read once in `ThreadPool::drop`'s `debug_assert_eq!(... == 0 ...)` to catch "pool dropped with a live scope." It **never gates a wakeup/park/steal** → no loom/Miri model needs it; explicitly out of the proof surface.

### D8 — shuttle: deferred
loom+Miri+stress meet "sound by proof." `sync.rs` is shuttle-ready (future `#[cfg(shuttle)]` arm + `tests/shuttle_pool.rs`). Defer.

---

## 3. File-by-file plan

**New:**
- `crates/boyko_threadpool/src/sync.rs` [W1, independent] — D1 shim.
- `crates/boyko_threadpool/tests/loom_pool.rs` [W3, after shim] — `#![cfg(loom)]`, M1/M2/M3, crossbeam-free.
- `crates/boyko_threadpool/tests/miri_scope.rs` [W3, after cfg(miri) yields] — `#![cfg(miri)]`, D4 Miri-1.
- `crates/boyko_ecs/tests/miri_schedule_parallel.rs` [W4] — `#![cfg(miri)]`, D4 Miri-2.
- `crates/boyko_threadpool/tests/stress.rs` [W2, independent] — D6.

**Modified:**
- `crates/boyko_threadpool/src/lib.rs` [W1] — `mod sync;`.
- `crates/boyko_threadpool/src/scope.rs` [W2] — imports → `crate::sync`; **extract `ScopeShared::{register_task, complete_task, is_drained}`** (`#[inline]`, byte-identical codegen — C1) and rewrite the four call sites (`:158`, `:193-196`, `:234`, `:276`) to use them; `cfg(miri)` yields in EVERY Miri-reachable unbounded loop here (`drain_one` `:349`, `join_workers_until_drained` `:275` loop top — H2).
- `crates/boyko_threadpool/src/worker.rs` [W2] — imports → `crate::sync`; keep `mark_idle`/`unmark_idle`/`unpark_one_idle` as the real shimmed functions M2 calls; **optionally extract `claim_one_idle(idle: &AtomicU64) -> Option<u32>`** (lowest-bit + `compare_exchange_weak` core) so M2b drives it without a full pool (`#[inline]`, zero native cost); `cfg(miri)` yields in EVERY Miri-reachable unbounded loop here (`drain_one` `:183`, `worker_main` `'outer` loop `:29` top — H2).
- `crates/boyko_threadpool/src/thread_pool.rs` [W2] — imports → `crate::sync`; **remove `park_state`** + its initializer + a layout-assert grep (D7).
- `crates/boyko_threadpool/Cargo.toml` [W1] — move `loom` from a plain `[dev-dependencies]` to **`[target.'cfg(loom)'.dev-dependencies] loom = "0.7"`** (only under `--cfg loom`); **remove the stale unconditional `loom` dev-dep + the unused `loom = []` feature**; add `[lints] workspace = true` (H1/M3).
- Root `Cargo.toml` [W1] — add **`[workspace.lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(loom)'] }`** (the canonical home — a manifest lint table, NOT `RUSTFLAGS`/`.cargo/config.toml`); do NOT register `cfg(miri)` (built-in). Confirm `.cargo/config.toml` still carries `-Zmiri-tree-borrows`. **Verify loom isolation:** `cargo tree -p boyko-threadpool` (no `--cfg loom`) MUST NOT list `loom`; a `--cfg loom` build MUST (M3/M4).

**Parallelism:** W1 (shim+Cargo+lib) ∥ W2 stress authoring. W2 production edits = one small sequential pass. W3 loom ∥ miri_scope. W4 miri_schedule. Loom and Miri authoring are mutually independent.

---

## 4. Invariants & assertions

| Check | Invariant | A bug looks like |
|---|---|---|
| loom-M1 | join returns ⟺ `pending==0`; `completed==N`; terminates | loom deadlock (lost wakeup) or `completed<N` (task outlived join → transmute unsound) |
| loom-M2 (2 thr) | worker never parks with unclaimed work present (drives real `mark_idle`/re-poll/`unpark_one_idle`) | loom interleaving: worker parked + idle bit set + work flag set + no pending unpark |
| loom-M2b (3 thr) | exactly one worker unparked per `unpark_one_idle` success; CAS loser observes cleared bit (drives real `compare_exchange_weak`) | double-wake (two workers for one push) or lost-wake (CAS loser exits without retry) |
| loom-M3 | every worker exits after `shutdown` | a worker parked forever post-shutdown |
| Miri-1 | transmute+cross-thread deref+join TB-clean, race-free | TB error on `as_ref`; race on `pending`/stack; UAF on `ScopeShared` |
| Miri-2 | 2-worker `Schedule::run` UB-free | TB/race in executor `UnsafeEcsCell` sharing |
| stress | run-once + dropped-once, none lost/leaked | `ran.swap==true` (double-run); `dropped.fetch_add>0` (double-drop); total<N (loss) |
| stress (post-join sweep) | ∀ i: `ran[i]==true` ∧ `dropped[i]==1` after join | any `ran[i]==false`/`dropped[i]!=1` → task lost/leaked/double-dropped (loss is invisible to in-body asserts) |
| stress (`no_starvation`, `#[ignore]`) | best-effort: every worker makes some progress | not a gate — flaky by nature on shared CI |

---

## 5. How to run

```bash
# loom (exhaustive; release for speed)
RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test loom_pool
LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test loom_pool
# debug a failing model:
LOOM_CHECKPOINT_FILE=loom.json LOOM_CHECKPOINT_INTERVAL=1 LOOM_LOG=trace LOOM_LOCATION=1 \
  RUSTFLAGS="--cfg loom" cargo test --release -p boyko-threadpool --test loom_pool -- <name>

# Miri (TB is the .cargo/config.toml default) — Miri-1, pool-only:
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-many-seeds=0..32" \
  cargo +nightly miri test -p boyko-threadpool --test miri_scope
# D5.1 outcome (ii) cross-check if TB reproduces the protected-tag flag:
MIRIFLAGS="-Zmiri-stacked-borrows -Zmiri-disable-isolation" \
  cargo +nightly miri test -p boyko-threadpool --test miri_scope   # SB authoritative if std-scope equally TB-red
# Miri-2 is the slow one — single-seed first landing, then a small sweep (M1):
MIRIFLAGS="-Zmiri-disable-isolation" \
  cargo +nightly miri test -p boyko-ecs --test miri_schedule_parallel
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-many-seeds=0..8" \
  cargo +nightly miri test -p boyko-ecs --test miri_schedule_parallel

# stress (native)
cargo test --release -p boyko-threadpool --test stress

# zero-native-cost regression
cargo build --release && cargo bench -p boyko-ecs --bench phase9_scheduler -- 50
```

---

## 6. Zero-native-cost contract
- **Shim:** `#[cfg(not(loom))] pub(crate) use core::sync::atomic::X` — transparent alias; identical type & codegen.
- **Extracted `ScopeShared` methods:** `#[inline]` wrappers over the identical atomic ops → the compiler inlines them away; same MIR/asm as the pre-9.1 inline code.
- **`cfg(miri)` yields (four sites — D4/H2):** both `drain_one` `Steal::Retry` arms + the `join_workers_until_drained` loop top + the `worker_main` `'outer` top. All compile to nothing when miri is off → affected loop bodies byte-identical natively. (The bounded `unpark_one_idle` CAS loop gets none.)
- **`park_state` removal:** removing never-read state cannot change behavior; shrinks `WorkerHandle`.
- **No `cfg` branches in hot loops** (loom join lives in a test, not production).
- **Verify (primary = byte-identity, backstop = bench):** the strongest guarantee is structural — the `use`-alias swap + `#[inline]` extraction + vanishing `cfg(miri)` yields change zero native logic, asserted by inspecting the import/extraction diff (no logic change). **There is no dedicated pool-throughput bench today** (grep-confirmed: only `phase9_scheduler` exercises the pool, via the 50-systems schedule), so the bench backstop is `phase9_scheduler -- 50` A/B (`git stash` pre-9.1 vs post), expected within the Phase-9 ±2% noise band — a *regression backstop*, with byte-identity as the actual proof. A standalone threadpool bench is out of scope. Native `cargo build --release` must be clean, no new warnings; loom/Miri are test-only (never in the shipped artifact).

---

## 7. Found-bug protocol
1. **Triage** tooling artifacts: loom `SeqCst`-as-AcqRel (boyko uses none); loom park/unpark token false-deadlock (use Condvar in model, D2); Miri spin-livelock (add cfg(miri) yield, D4); Miri weak-memory false positive (re-run `-Zmiri-disable-weak-memory-emulation` to confirm).
2. **Fix** in production; **re-run** the failing loom model / Miri seed; the fix must be zero-native-cost or benchmarked (an ordering strengthening justified + bench-checked; a structural fix must not regress the Phase-9 5× headroom).
3. **Record** the bug + the reproducing loom seed / Miri command in the results doc.

---

## 8. CI
Two **nightly + manual-dispatch** jobs (loom/Miri are minutes-slow — not per-PR): a `loom` job (`RUSTFLAGS="--cfg loom" … --test loom_pool`, `LOOM_MAX_PREEMPTIONS=3`) and a `miri` job (`miri_scope` + `miri_schedule_parallel`, `-Zmiri-many-seeds=0..64`). The **stress** test joins the normal per-PR `cargo test --release` (fast, native). Additive GitHub Actions jobs; don't touch the docs/Pages deploy.

---

## 9. Explicitly deferred

| Deferred | Why | Re-entry |
|---|---|---|
| shuttle | loom+Miri+stress meet the goal; additive | `sync.rs` shuttle-ready; add `tests/shuttle_pool.rs` |
| Miri >2 workers | exponential runtime; 2 workers exercises every cross-thread path | bump `num_threads`+seeds later |
| loom on crossbeam deque | impossible (opaque, SeqCst); Chase-Lev Coq-verified upstream | covered by D6 stress |
| Miri+GenMC / RustMC | experimental, custom Miri, no-unbounded-loops (worker loop is unbounded) | revisit when GenMC stabilizes |
| **Phase-9 "multi-thread Miri deferred (TB protected-tag in `Scope::spawn` transmute)" note** | **9.1 addresses it directly:** D4-Miri-1 drives the transmute on real threads under TB. If the protected-tag conflict reproduces → §7 found-bug (fix by mirroring std::thread::scope); if not → 9.1 confirms it resolved and updates the note. | the `miri_scope` test is the probe |

---

## 10. Open questions for the critic
1. **D2 fidelity:** is reconstructing the join protocol in the loom test (vs `cfg(loom)`-shimming the real function) acceptable given the same-orderings + cross-referenced-lines mitigation?
2. **D7:** confirm `park_state` is truly dead before removal.
3. **Miri-2 placement:** real `Schedule::run` (`boyko_ecs` test, chosen) vs a lighter threadpool-level stub — runtime trade-off.
4. **CI cadence:** nightly + manual-dispatch for loom/Miri acceptable, or a per-PR subset gate?

---

**Implementation order:** W1 (shim + Cargo + lib) → W2 (production import-swap + `cfg(miri)` yields + `park_state` removal + stress) → W3 (loom ∥ miri_scope) → W4 (miri_schedule_parallel) → run all suites → results doc + roadmap + memory. Expected outcome: all green (proof of soundness); otherwise §7.
