> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9.1 — Research digest: loom + Miri verification of the parallel executor

Goal: turn the `boyko_threadpool` Chase-Lev work-stealing pool + `Scope` fork/join
from "sound by design" (currently `#[cfg(not(miri))]`) into "sound by proof."

## Load-bearing findings

1. **The crossbeam-deque CANNOT be loom-modeled.** It is a dependency compiled
   without loom shims and uses `SeqCst` heavily (loom mismodels `SeqCst` as
   `AcqRel` → false alarms). crossbeam-deque itself is NOT loom-tested
   (crossbeam#846 open; jonhoo's loom PR #487 covers epoch/utils, *excludes*
   deque). The Chase-Lev algorithm boyko depends on is **formally verified
   upstream** (Coq linearizability proof, arXiv 2309.03642). **Therefore: do NOT
   try to loom the deque. loom-model boyko's OWN atomics only**, with the deque
   abstracted away.

2. **boyko's `Scope` join is structurally identical to `std::thread::scope`** (the
   gold-standard, Miri-tested-upstream template). std: `num_running_threads`
   `fetch_add(Relaxed)` / `fetch_sub(Release)`+`unpark` / `load(Acquire)` park
   loop. boyko `ScopeShared` (scope.rs): `pending.fetch_add(1, AcqRel)` /
   `fetch_sub(1, AcqRel)`+`if prev==1 {waker.unpark()}` / `load(Acquire)` poll
   loop — same shape, **slightly stronger** (AcqRel vs Relaxed add). The
   lifetime-erasure `transmute` (scope.rs:222) is the **rayon pattern** (store
   type-erased body, rely on the join), also Miri-tested upstream. The transmute
   soundness is a **happens-before + provenance concern**, NOT a loom concern:
   loom proves the *join ordering* (no task outlives the wait); Miri-TB proves the
   *reborrow/provenance* of captured stack data on the worker thread.

3. **Miri CAN run boyko's real pool at 1-2 workers.** Miri runs real threads
   cooperatively (one schedule/run) + weak-memory fuzz via
   `-Zmiri-many-seeds=0..N`. This is the **highest-value target**: a `cfg(miri)`
   2-worker test driving `install(|s| s.spawn(...))` + a tiny `Schedule::run`
   exercises the transmute, the cross-thread `SharedPtr::as_ref` deref, the
   `pending` join, and `ThreadPool::drop` shutdown — all under Tree Borrows +
   data-race detection.

4. **Two concrete hazards to guard:**
   - **loom park/unpark token divergence** (loom#246): bare `park`/`unpark` under
     loom can false-positive deadlock, and **there is no `loom park_timeout`**.
     boyko's `join_workers_until_drained` uses `park_timeout(50µs)` + work-steal —
     no loom equivalent. Under `cfg(loom)` the wait must become a Condvar/flag
     loop, OR (preferred) model a *separate minimal join primitive* mirroring the
     production ordering, deque abstracted out.
   - **Miri non-preemptive scheduler hangs on yield-free spin loops**
     (crossbeam#829). boyko's `drain_one` `Steal::Retry` loop (scope.rs:349,
     worker.rs) has **no yield** → Miri livelock risk under weak memory. Guard
     with a `cfg(miri)` `yield_now`. boyko's other spins use
     `Backoff::snooze()` (yields after spinning) — OK.
   - **`park_timeout` masks a lost-wakeup as a latency spike, not a hang** — so
     "it works in production" is weak evidence; the ordering must be proven (loom)
     so the timeout is purely a backstop.

## The verification matrix (recommended combo)

| Tool | Role in Phase 9.1 |
|------|-------------------|
| **loom** | Exhaustively prove boyko's OWN primitives in isolation: the pending-counter join (no-lost-wakeup), the idle/park/unpark bitset (the post-`mark_idle` re-poll "Race C"), the shutdown handshake. Deque abstracted. 2 tasks, `LOOM_MAX_PREEMPTIONS=2-3`. |
| **Miri** (`-Zmiri-many-seeds -Zmiri-disable-isolation`) | Prove the `Scope::spawn` transmute + cross-thread deref sound on REAL 2 worker threads (Tree Borrows + data races). Lift the blanket `#[cfg(not(miri))]` for a clamped 2-worker path. |
| **stress tests** (crossbeam-style) | Deque exactly-once / no-loss / no-double-drop the loom can't reach: mirror crossbeam's `spsc`/`stampede`/`stress`/`no_starvation`/`destructors` (Drop-accounting `TaskHandle`). |
| **shuttle** (optional) | Randomized, scales to 4-8 workers where loom explodes. Same cfg-shim as loom. Defer unless cheap. |

## The loom cfg-shim pattern
A new internal module (e.g. `boyko_threadpool/src/sync.rs`) re-exporting under cfg:
```rust
#[cfg(loom)]  pub(crate) use loom::sync::atomic::{AtomicU64, AtomicUsize, AtomicBool};
#[cfg(not(loom))] pub(crate) use std::sync::atomic::{AtomicU64, AtomicUsize, AtomicBool};
// + loom::sync::{Arc,Mutex,Condvar}, loom::cell::UnsafeCell (.with()/.with_mut()),
//   loom::thread::{park,current,yield_now,Thread}
```
`Cargo.toml`: `[target.'cfg(loom)'.dependencies] loom = "0.7"`.
Run: `RUSTFLAGS="--cfg loom" cargo test --release --test <loom_test>`.
Every `use std::sync::atomic::*` in thread_pool.rs/worker.rs/scope.rs routes
through the shim. **Architect decision:** shim the whole crate, OR factor the
join+idle protocols into a small `cfg(loom)` inner module that never pulls in
crossbeam (avoids the deque-opacity problem entirely — preferred).

## boyko's actual verification surface (file:line)
- `scope.rs`: `ScopeShared::pending` (`fetch_add` :158, `fetch_sub`+unpark :193-196),
  `SharedPtr::as_ref` raw deref :103-106 + `unsafe impl Send` :117, the
  `transmute` :222, `join_workers_until_drained` poll+`park_timeout(50µs)` :263-322,
  `drain_one` `Steal::Retry` (no yield) :349.
- `worker.rs`: `mark_idle`/`unmark_idle` (`fetch_or`/`fetch_and` Release),
  `unpark_one_idle` (`load(Acquire)`→`compare_exchange_weak`→`unpark`), `worker_main`
  post-`mark_idle` re-poll ("Race C") + bare `park()`.
- `thread_pool.rs`: `idle`/`active_scopes`/`shutdown` atomics, `ThreadPool::drop`
  shutdown handshake, dormant `WorkerHandle::park_state` (decide: wire or drop).
- `tls.rs`: `Cell` TLS — not a concurrency target.
- **No loom scaffolding exists today** — every atomic imports `std::sync::atomic`
  directly.

## Decisions for the architect
1. loom shim scope: whole crate vs a small crossbeam-free join/idle inner module
   (preferred: the latter avoids deque opacity).
2. loom join wait: rewrite `join_workers_until_drained` under `cfg(loom)` to a
   Condvar/flag (no `park_timeout`), OR model a separate minimal primitive that
   mirrors the production ordering (risk: model/prod divergence — mitigate by
   sharing the counter+notify code, abstracting only the deque steal).
3. Miri: lift the blanket `#[cfg(not(miri))]` for a `cfg(miri)` 2-worker
   `Schedule::run` path; add `cfg(miri)` `yield_now` to the `Steal::Retry` loops.
4. Wire or drop the dormant `WorkerHandle::park_state`.
5. shuttle: in scope now, or defer? (default defer — loom+Miri are the must-haves.)
6. Exactly-once: adopt a Drop-accounting `TaskHandle` test type (crossbeam
   `destructors` style) to prove no task dropped twice / leaked across steal races.

## Safe to defer
Miri+GenMC / RustMC (experimental, custom Miri build, no-unbounded-loops — boyko's
worker loop is unbounded). shuttle (additive). Multi-thread Miri on the FULL
`Schedule::run` at >2 workers.

## Sources
loom docs/book; crossbeam#846/#487/#829 (deque-not-loomed, Miri spin-hang);
std `thread/scoped.rs` (join template); Miri README + Ralf Jung 2025-12 blog
(many-seeds, C++20 weak memory); rayon#938 (Miri on a real pool); awslabs/shuttle;
matklad "Properly Testing Concurrent Data Structures" (2024); Chase-Lev Coq proof
(arXiv 2309.03642).
