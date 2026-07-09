> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9.3 — ThreadPool Shutdown: break the worker↔pool Arc cycle so Drop runs and workers join

**Status:** PLAN (architect delivered; pending critic → developer). Read-only investigation complete.
**Scope:** `crates/boyko_threadpool` construction + shutdown + worker-held reference only. The
steal/run inner loop, `push_task`, `unpark_one_idle`, the idle-bitset protocol,
`Scope::spawn`/`Scope::drop` (the Phase 9.2 Candidate-U `NonNull<ScopeShared>` protocol), and
every ECS hot path stay byte-identical or provably equivalent.

## 1. Goal
Make `ThreadPool` shutdown (`shutdown.store` + unpark-all + join-all) **actually execute** when
the user's handle is dropped, so the N worker OS threads + the pool allocation are reclaimed
deterministically instead of leaking until process exit.

**Functional invariant (the property the current design lacks):** dropping the last user-facing
handle runs shutdown + join **exactly once**; no worker thread runs after `Drop` returns; no
double-join; the pool allocation is freed exactly once.

**Performance invariant (inviolable):** the worker hot loop and the push/wake paths keep
**one-deref** field access. No per-steal atomic added (no `Weak::upgrade` on the hot path).
Native codegen of the steal/run loop unchanged modulo the base pointer it offsets from
(`Arc<PoolInner>` instead of `Arc<ThreadPool>` — same shape, same offsets).

## 2. Root cause (confirmed in code)
- `ThreadPoolBuilder::build` (`thread_pool.rs:358-480`) returns `Arc<ThreadPool>`.
- Each worker takes an **owned `Arc<ThreadPool>` clone** (`thread_pool.rs:430`), passed by value
  into `worker_main(pool: Arc<ThreadPool>, …)` (`worker.rs:20`), held for the worker's life.
- Refcount after build = `1 (caller) + N (workers)`.
- `ThreadPool::drop` (`thread_pool.rs:246-274`) is the ONLY writer of `shutdown=true`; workers
  exit ONLY when they observe `shutdown` (`worker.rs:86-89`).
- Caller drops its `Arc` → count → N (workers still hold theirs) → `ThreadPool::drop` never runs
  → `shutdown` never set → workers park forever. **Cycle: Drop unreachable → shutdown dead → N
  threads + allocation leak.** Surfaced under Miri as "main thread terminated without waiting for
  all remaining threads" (suppressed today by `-Zmiri-ignore-leaks`).

## 3. Approach (decision)
**CHOSEN — Approach 1: split into a user-facing `ThreadPool` handle + `Arc<PoolInner>`; workers
hold `Arc<PoolInner>`, NOT the handle.** Exactly rayon's structure (`WorkerThread { registry:
Arc<Registry> }` strong; `ThreadPool` handle's `Drop` triggers `Registry::terminate()`).

- **Rejected #2 (`Weak<ThreadPool>` in workers):** a single top-of-`worker_main` `upgrade()` held
  for the worker's life re-creates the exact cycle; `upgrade()`-per-task drops the strong ref
  between steals, but then a parked worker holds none → the handle can free the pool while a
  worker is parked on `pool.workers[id].thread` → UAF on wake. Bounding it reinvents `PoolInner`
  with extra per-task refcount churn. Strictly worse.
- **Rejected #3 (explicit `shutdown()` + separate join-handle owner):** keeps the cycle; a user
  clone of the (`Clone`) `Arc<ThreadPool>` lets the guard's Drop fire while another clone +
  workers still reference a half-torn-down pool. #1 makes "who triggers shutdown" a type-enforced
  singleton. We DO expose an optional `join()` on top of #1 (§5.6), but it is not the
  cycle-breaking mechanism.

## 4. Data structures
### 4.1 New `PoolInner` (private; worker-shared state) — `#[repr(C)]`, field order + `CachePadded` preserved
Moves every field workers/`Scope` cross-reference: `injector_global: CachePadded<Injector>`,
`injector_local: Arc<[CachePadded<Injector>]>`, `stealers: Arc<[Stealer]>`,
`workers: Arc<[CachePadded<WorkerHandle>]>`, `idle: CachePadded<AtomicU64>`,
`active_scopes: CachePadded<AtomicUsize>`, `shutdown: CachePadded<AtomicBool>`,
`worker_count: u32`, plus (§5.4 decision D) `handle_ptr: AtomicPtr<ThreadPool>`.

### 4.2 Changed `ThreadPool` (handle; same name, crate-public type)
`pub struct ThreadPool { inner: Arc<PoolInner>, join_handles: Mutex<Option<Vec<WorkerJoin>>> }`.
One deref reaches every hot-path field. `join_handles` owned by the handle ALONE (workers cannot
reach/double-join them); `Option` so an explicit `join()` can take them, leaving `Drop` a no-op
(join-exactly-once). `Mutex` is cold teardown only.

### 4.3 `worker_main(inner: Arc<PoolInner>, worker_id, deque)` — was `Arc<ThreadPool>`.

## 5. Changes by file
- **`thread_pool.rs`:** define `PoolInner` + new `ThreadPool`; getters/`install`/`scope`/`spawn`
  project via `self.inner`; `install`/`scope` keep TLS deposit `self as *const ThreadPool`
  (unchanged); `build()` constructs `Arc<PoolInner>`, stores `handle_ptr` after the handle `Arc`
  exists and BEFORE publishing `inner` to workers, returns `Arc<ThreadPool>` (no API churn); `Drop`
  takes-handles-then-`shutdown.store(Release)`-then-unpark-all-then-join; private
  `shutdown_and_join` + public `join()` (§5.6).
- **`worker.rs`:** `worker_main(inner: Arc<PoolInner>)`; deposit
  `tls::swap_active_pool(inner.handle_ptr.load(Acquire))` (SAFETY-commented, cold, once per
  startup); mechanical `pool.X` → `inner.X`; `push_task`/`unpark_one_idle`/poll helpers retyped to
  `&PoolInner`; `mark_idle`/`unmark_idle` unchanged (`&AtomicU64`).
- **`scope.rs`:** `Scope { inner: &'scope PoolInner, … }`; `new(&'scope PoolInner, …)`; `spawn`
  body `push_task(self.inner, …)`; `join_workers_until_drained(&PoolInner)` +
  `try_steal_any(&PoolInner)`. The Candidate-U `NonNull<ScopeShared>` free protocol is UNTOUCHED.
  Public `Scope::spawn` signature unchanged. `install`/`scope` build `Scope::new(&self.inner, …)`;
  `&self.inner: &'scope PoolInner` valid because the handle (which `install` borrows for `'scope`)
  keeps `inner` alive.
- **`tls.rs`:** UNCHANGED signatures; TLS stays `*const ThreadPool`.
- **`lib.rs`:** `pub use` unchanged; `PoolInner` NOT exported (`pub(crate)`); `loom_exports`
  untouched.
- **§5.6 optional `ThreadPool::join()`:** cold convenience; shares the take-once handles body with
  `Drop` so exactly one of {`join()`, `Drop`} joins.

### 5.4 The one real design call (OQ1) — keep TLS = `*const ThreadPool`, add `handle_ptr` (decision D)
`try_with_active_pool(|pool: &ThreadPool| …)`, `current_pool`, `swap_active_pool` are public and
consumed by `par_iter`/`par_chunk` in `boyko_ecs`. The dispatcher deposits `self as *const
ThreadPool` (unchanged). But a worker holds only `Arc<PoolInner>` and must NOT keep the handle
alive (would re-create the cycle), so it cannot deposit a `*const ThreadPool` — UNLESS it reads
one from a non-refcounting source. **Decision D:** `PoolInner.handle_ptr: AtomicPtr<ThreadPool>`,
a RAW self-pointer (0 refcount impact), stored once in `build()` (Release) before publishing
`inner`, read once at `worker_main` entry (Acquire) to deposit into TLS. Valid because the handle
joins all workers before dropping `inner` (and thus `handle_ptr`'s pointee), so every worker's
deposited `*const ThreadPool` is valid for its whole life. **Keeps `PoolInner` private + zero
`boyko_ecs` diff.** Alternative E (widen TLS to `*const PoolInner`, make `PoolInner` pub, edit 3
`boyko_ecs` sites) rejected — leaks `PoolInner` into the public API. Critic to confirm D's
self-pointer validity proof or direct E.

## 6. Cycle-broken proof
- After build: `strong(inner) = 1 + N`; `strong(Arc<ThreadPool>) = 1` (workers hold NO
  `Arc<ThreadPool>`). `handle_ptr` is raw → 0 refcount.
- Only `Arc<ThreadPool>` is the caller's → dropping it → count 0 → `ThreadPool::drop` runs. ∎
- `Drop` `take()`s handles (Some on first), sets `shutdown` (Release), unparks all, joins. Each
  worker wakes, `Acquire`-loads `shutdown` (M3 loom edge, unchanged), `unmark_idle`s, returns →
  drops its `Arc<PoolInner>`. After all joins, `strong(inner) = 1` (handle's). ∎
- `PoolInner` freed once (handle's `inner` drops at end of `Drop`). No worker touches `PoolInner`
  after its `worker_main` returns (last access `unmark_idle` happens-before the joiner's `join()`
  return via std `JoinHandle` happens-before). ∎
- No double-join (`Option::take` under `Mutex`). No worker after Drop (Drop returns after all
  `join()`). Handle clones: shutdown defers to last clone drop (std `Arc`); `handle_ptr` (one
  allocation, `Arc::as_ptr` same for all clones) valid until last clone drops = when Drop runs. ∎

## 7. Hot-path equivalence
Steal/run loop, `push_task`, `unpark_one_idle`, `mark_idle`/`unmark_idle`, `Scope::spawn`,
`Scope::drop` join wait: all **one deref**, byte-identical modulo the base pointer's static type
(`&PoolInner` vs `&ThreadPool` — same offsets, field order + `CachePadded` preserved). Cold deltas
only: `install`/`scope` add one `self.inner` Deref per frame (not per task); `worker_main` startup
adds one cold `handle_ptr` Acquire-load (never in the loop). 0%-regression on `phase9_scheduler`.

## 8. Blast radius — `boyko_ecs` + `boyko_demo` net diff = ZERO
Because `build()` still returns `Arc<ThreadPool>`, `ThreadPool` is still the named type, and
`try_with_active_pool`/`Scope`/`install`/`scope`/`current_pool` keep public signatures, every
consumer (`Schedule.pool`, `ScheduleBuilder`, `par_iter`/`par_chunk` `try_with_active_pool`
closures, `boyko_demo` `_pool`/`SimRunner`, all tests, the bench) is UNCHANGED. All edits confined
to `crates/boyko_threadpool/src/{thread_pool,worker,scope}.rs` (+ internal `pool.X`→`inner.X`).
`boyko_demo`'s `_pool` now genuinely joins workers at `DemoApp` drop — behavior improvement, not
API change.

## 9. Send/Sync/loom/Miri
- `PoolInner`/`ThreadPool` Send+Sync by auto-derive (same field set + `AtomicPtr` + `Mutex`); no
  new `unsafe impl`. `handle_ptr` self-pointer never races (Release store before publish / Acquire
  load at startup).
- loom models (`loom_pool.rs`) drive `ScopeShared` + `mark_idle`/`unmark_idle` + idle CAS; none
  construct `ThreadPool`/`PoolInner`. Split touches none of those → **loom passes unchanged**; M3
  models the same `shutdown` Release/Acquire edge.
- Miri: `Drop` now joins workers → no thread leak → **`-Zmiri-ignore-leaks` can be DROPPED** from
  the gate once every Miri-run pool is dropped/`join()`ed before the test returns. Update
  `miri_phase9.rs:63-70` + `miri_scope.rs` notes. OQ3: confirm `miri_scope` terminates without the
  flag; if a Drop-join liveness stall appears under Miri, add `#[cfg(miri)] yield_now()` to the
  Drop unpark loop (native no-op) — only if observed. (Interacts with the Phase-9.3 Bug-1 executor
  yields — those must land first so workers stay Miri-cooperative.)

## 10. Developer steps
1. `thread_pool.rs`: define `PoolInner` (+`handle_ptr`); redefine `ThreadPool`.
2. project getters/`install`/`scope`/`spawn` via `self.inner`; `Scope::new(&self.inner, …)`.
3. rewrite `build()` (Arc<PoolInner>, store handle_ptr before publish, return Arc<ThreadPool>).
4. `Drop` + private `shutdown_and_join` + public `join()` (take-once handles).
5. `worker.rs`: `worker_main(Arc<PoolInner>)`, TLS deposit via `handle_ptr.load(Acquire)`,
   `pool.X`→`inner.X`, retype helpers to `&PoolInner`. (Independent file after step 1.)
6. `scope.rs`: `Scope { inner: &'scope PoolInner }`, joiner retarget; Candidate-U untouched.
   (Independent file after step 1; 5 & 6 may batch.)
7. `tls.rs`/`lib.rs`: confirm unchanged.
8. tests: `tests/shutdown.rs` (`pool_drop_joins_all_workers`, `pool_double_shutdown_is_safe`,
   `pool_drop_without_join_terminates`); drop `-Zmiri-ignore-leaks` from gate + update notes.

## 11. Validation gate
- New `tests/shutdown.rs`: assert `pool.join()` returns + each worker ran (Drop-tracker barrier
  task) + double-shutdown safe + drop-without-join doesn't hang.
- Existing: `smoke`, `stress` (exactly-once Drop-accounting), `loom_pool` 4/4, `miri_scope`
  (TB+race, **drop `-Zmiri-ignore-leaks`**), boyko_ecs schedule/par tests, boyko_demo smokes.
- Bench: `phase9_scheduler` `50_systems` + `par_iter_4096` within ±3% (one-deref equivalence).
- `debug_assert!`: `active_scopes==0` in Drop (kept); `!handle_ptr.is_null()` at worker TLS
  deposit (new).

## 13. Open questions
- **OQ1 (the call):** D (`handle_ptr` self-pointer, `PoolInner` private, 0 `boyko_ecs` diff) vs E
  (TLS `*const PoolInner`, `PoolInner` pub, 3-site `boyko_ecs` edit). Recommend **D**.
- **OQ2:** `join_handles` on the handle behind `Mutex<Option<…>>` (chosen) — handle is the unique
  owner; clone semantics correctly defer to last clone.
- **OQ3:** confirm `miri_scope` terminates with `-Zmiri-ignore-leaks` removed; add a Miri-only
  Drop-loop yield only if a stall is observed.

---
*Severity note: this is a BENIGN leak (not UB) for the singleton-process-lifetime pool the engine
actually uses; it matters for multi-pool/tooling correctness and lets the Miri gate drop
`-Zmiri-ignore-leaks`. Sequence AFTER Phase-9.3 Bug-1 (executor Miri-yields), which is its
prerequisite for the Drop-join to terminate under Miri.*

---

## CRITIC AMENDMENT (architecture-critic, APPROVED WITH CHANGES) — supersedes OQ1

**OQ1 RESOLVED: use decision E, NOT D.** Decision D (`handle_ptr: AtomicPtr<ThreadPool>`
self-pointer, dereffed as `&ThreadPool` on workers) re-introduces the protected-tag TB
class that already produced the Phase 9.2 and 9.3c findings: a worker forming `&ThreadPool`
via `try_with_active_pool` while the main thread is inside `ThreadPool::drop(&mut self)`
(whose `&mut self` protector spans the join) is a foreign-access-vs-protected-tag conflict.
CRITICAL — D is rejected.

**Decision E (mandated):**
- TLS `ACTIVE_POOL` becomes `*const PoolInner` (was `*const ThreadPool`).
- `PoolInner` is made **opaque `pub`** — NO public fields; expose only the methods
  `par_iter`/`par_chunk` need: `num_threads()`, `scope()`/`install()`. The worker-shared
  fields stay `pub(crate)`.
- `try_with_active_pool` hands out `&PoolInner`; `current_pool` returns `*const PoolInner`
  (or `NonNull<PoolInner>`). `Scope` borrows `&'scope PoolInner` (as already planned).
- DELETE `handle_ptr` and all of D's store-before-publish/Acquire-load machinery. The
  existing bootstrap `Mutex`+`Condvar` already provides the publication happens-before for
  `Arc<PoolInner>`; no new atomic ordering is introduced.
- Why E is sound where D is not: `PoolInner` is NEVER borrowed `&mut` anywhere — it lives
  behind `Arc<PoolInner>` and is dropped only via `Arc`'s internal `&mut` at refcount 0,
  i.e. AFTER every worker has joined (same safety as today's `Arc<ThreadPool>`). No
  `&mut PoolInner` protector ever spans a worker access ⇒ the protected-tag class is gone
  by construction (the same reasoning that made the Phase 9.2 `NonNull` fix work). The
  handle's `Drop`/`join_handles`/`&mut self` are over the `ThreadPool` allocation, which no
  worker pointer ever references.

**boyko_ecs diff under E (M1 — small, mechanical, type-only, NO semantic change):** exactly
3 sites, each only calling `.num_threads()`/`.scope()`:
`crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:261` (the `try_with_active_pool`
closure `&ThreadPool` → `&PoolInner`), `…/par_chunk.rs:111` (same), and `current_pool`'s
return-type consumer if any. boyko_demo: unchanged (`build()` still returns
`Arc<ThreadPool>`).

**M2 (hot path):** state the invariant as "same indirection depth as today (`Arc<_>` outer
deref + the pre-existing inner `Arc<[…]>` slice deref); NO new pointer hop; **NO per-task
`Arc::clone`/`Weak::upgrade`**." That no-atomic-on-hot-path invariant is the bench-gate
acceptance criterion (`phase9_scheduler` 50_systems + par_iter_4096 within ±3%).

**M3 (`-Zmiri-ignore-leaks`):** drop the flag ONLY for `miri_scope.rs` (its sole leak was
the un-joined pool) and the new native `tests/shutdown.rs` (no leak concern). `miri_phase9
/15/16/17.rs` and `miri_zst_resource.rs` STILL need the flag for unrelated
`EcsMaster`/`Box::leak` cache leaks — do NOT attribute their leak to the pool, do NOT drop
their flag. 9.3b is verifiable WITHOUT fixing 9.3c: primary gate = native `tests/shutdown.rs`
(join-happened + double-shutdown-safe + no-hang) + `miri_scope.rs` (TB + droppable leak
flag). `miri_schedule_parallel` stays `#[ignore]`d on the separate 9.3c TB error — OUT OF
SCOPE for verifying 9.3b.

**M4 (9.3c interaction):** 9.3b is provenance-NEUTRAL w.r.t. the 9.3c TB error — the 9.3c
protector source is `&self.executor_scratch.completion_queue as *const _` (schedule.rs:917,
a child of `executor_main_loop`'s ECS `&mut self`), which 9.3b does NOT touch. 9.3b only
retypes the cross-thread TRANSPORT (`scope.rs`/`worker.rs`). Adopting E keeps the global
"cross-thread state reached only via non-`&mut` sources" invariant consistent, which the
later 9.3c fix (NonNull-derive the completion-queue ptr) will lean on. Keep the Candidate-U
`NonNull<ScopeShared>` protocol UNTOUCHED.

**O4 (panic path):** `Drop` now actually runs, so a pool dropped while unwinding after a
panic-in-`install` could trip the `active_scopes == 0` debug_assert (the `active_scopes`
decrement + TLS restore at thread_pool.rs:207-210 are skipped when `drop(scope)` at :205
itself `resume_unwind`s). Fix: fold the `active_scopes` decrement + TLS restore into an
unwind-safe guard (RAII) so they run on unwind too; OR document the debug-assert as
expected-benign. Pick the guard (cleaner). Pre-existing, but 9.3b surfaces it.

**O5:** invariant — the handle must never be dropped from inside an `install`/`scope` frame
(self-join deadlock now that `Drop` blocks). It cannot today; state it.

**O1:** `worker_count: u32` lives in `PoolInner` (workers read it via `&PoolInner` under E).
**O3:** update the stale bootstrap comment block (thread_pool.rs:382-395) to say
`Arc<PoolInner>`.
