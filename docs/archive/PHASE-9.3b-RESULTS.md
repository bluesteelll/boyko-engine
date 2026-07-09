> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9.3b — Results: ThreadPool shutdown / break the worker↔pool Arc cycle

Branch `ecs`. Committed locally as **`b094936`** (author Celtokisa, no `Co-Authored-By`,
NOT pushed). Plan: [`docs/PHASE-9.3-POOL-SHUTDOWN-PLAN.md`](PHASE-9.3-POOL-SHUTDOWN-PLAN.md).

## Status: COMPLETE — `ThreadPool::drop` now genuinely shuts down + joins all workers

### The bug
`ThreadPoolBuilder::build()` returned `Arc<ThreadPool>` and every worker held an **owned
`Arc<ThreadPool>` clone** for its whole life (`worker_main(pool: Arc<ThreadPool>, …)`). The
sole writer of `shutdown=true` (then unpark + join) was `ThreadPool::drop` — which was
therefore **unreachable** while workers were alive (refcount `1 + N` never reached 0). The
shutdown/join machinery was dead code; the N worker OS threads + the pool allocation leaked
to process exit. Under Miri this surfaced as "the main thread terminated without waiting for
all remaining threads," masked by `-Zmiri-ignore-leaks`.

### The fix — decision E (split handle + `Arc<PoolInner>`)
Split the type:
- `PoolInner` (new, opaque-`pub`, `#[repr(C)]`, field order + `CachePadded` preserved): all
  worker-shared state (`injector_global`, `injector_local`, `stealers`, `workers`, `idle`,
  `active_scopes`, `shutdown`, `worker_count`). No `pub` fields; methods only.
- `ThreadPool` (handle, same public name): `{ inner: Arc<PoolInner>, join_handles:
  Mutex<Option<Vec<WorkerJoin>>> }`.

Workers now hold `Arc<PoolInner>` (NOT the handle). The handle's `Arc<ThreadPool>` refcount
is the user's alone, so dropping the last one runs `Drop` → `shutdown.store(Release)` →
unpark-all → join-all. `Mutex<Option<Vec>>` + `take()` in both `Drop` and the new public
`join()` makes shutdown+join run **exactly once** (the second is a no-op).

### Why decision E, not D (the critical call)
The architect's first proposal (decision D) added a `handle_ptr: AtomicPtr<ThreadPool>`
self-pointer in `PoolInner` so a worker could deposit the ambient-pool TLS without holding
an `Arc<ThreadPool>`. The architecture-critic flagged this **CRITICAL**: a worker forming
`&ThreadPool` (via `try_with_active_pool`) concurrent with `ThreadPool::drop`'s `&mut self`
protector is the **exact Tree-Borrows protected-tag class that already bit Phase 9.2
(`ScopeShared`) and Phase 9.3c (`completion_queue`)**. Decision E eliminates the class **by
construction**: `PoolInner` is **never** borrowed `&mut` anywhere (it lives behind
`Arc<PoolInner>`, dropped only via `Arc`'s internal `&mut` at refcount 0 = after all workers
have joined; no `Arc::get_mut`/`try_unwrap`), so the `*const PoolInner` TLS deref to
`&PoolInner` can never alias a `&mut PoolInner`. TLS became `*const PoolInner`;
`try_with_active_pool` hands out `&PoolInner`; `Scope` borrows `&'scope PoolInner`.

### Supporting changes
- **`InstallGuard` RAII** — folds the per-frame `active_scopes` decrement + TLS restore into
  a guard whose `Drop` runs on unwind too, so a panicking system no longer leaves
  `active_scopes > 0` (which would otherwise trip the new `Drop` debug-assert during unwind).
- **Hot path unchanged** — the worker steal/run/push/wake loop is a pure `pool`→`inner`
  rename, same indirection depth (`Arc<_>` deref + the pre-existing inner `Arc<[…]>` slice
  deref), **no new per-task `Arc::clone`/`Weak::upgrade`**. The Phase-9.3a `#[cfg(miri)]
  yield_now` sites are preserved.
- **Candidate-U `NonNull<ScopeShared>` protocol (Phase 9.2) untouched** — `scope.rs` change
  is only the `pool: &ThreadPool` → `inner: &PoolInner` field/param retype.
- **No new `unsafe impl`** — `PoolInner`/`ThreadPool` are `Send + Sync` by auto-derive.
- **Zero `boyko_ecs`/`boyko_demo` source edits** — `build()` still returns
  `Arc<ThreadPool>`, and the `try_with_active_pool` closure param type is inferred from the
  new `FnOnce(&PoolInner)` bound.

## Verification gate (all run by the orchestrator)

| Oracle | Result |
|--------|--------|
| **`tests/shutdown.rs`** (NEW, the 9.3b primary gate) | **4/4 pass** — `pool_drop_joins_all_workers`, `pool_double_shutdown_is_safe`, `pool_drop_without_join_terminates`, `pool_drop_joins_with_per_task_tracker` |
| **loom** (`loom_pool`) M1/M2/M2b/M3 | **4/4 pass** (worker↔pool split touches no loom-modeled atomic) |
| **`boyko-ecs --lib`** | **494 pass, 0 failed** |
| **`miri_scope`** (TB + data-race) | **3/3 pass**; residual "memory leaked" = crossbeam-epoch GC at-exit bags (workers now genuinely join — their `crossbeam_epoch::LocalHandle` TLS destructors run), NOT the pool. `-Zmiri-ignore-leaks` retained for that third-party reason only. |
| **`phase9_scheduler` bench** | **0% regression**: empty 5.4 ns, 50-sys 4.10 µs, par_iter 19.3 µs, two_disjoint 1.13 µs, one_excl 250 ns (criterion: two_disjoint "improved", others "no change") |
| `cargo clippy --workspace --all-targets -D warnings` | clean (after W1 fix) |
| `cargo build --workspace` (incl. boyko_demo) | clean |

## Pipeline + the bugs the process caught

architect plan → **critic** (APPROVED WITH CHANGES: resolved OQ1 → decision **E**, rejected
D as CRITICAL; M1–M4 + O1–O5) → **developer** (decision E, ~773/−170 across 6 files) →
orchestrator gate run → **code-review** (CHANGES REQUESTED: W1) → fix → commit.

Three real issues were caught before landing (none reached the commit):
1. **Critic — decision D was a TB hazard.** The self-pointer would have re-introduced the
   protected-tag class for the third time this phase. Switched to E.
2. **Gate — `tests/shutdown.rs` hung.** Two tests used `std::sync::Barrier(WORKERS)` across
   scope tasks; boyko's `Scope::drop` joiner batch-steals and drains the batch **inline** on
   the dispatcher, so it can pull several tasks into its scratch deque and run them
   one-at-a-time — a task blocking on a barrier-of-N wedges the dispatcher while its siblings
   sit unrun → deadlock. A **test bug** (the no-blocking-barrier-across-work-stealing-tasks
   rule), fixed to non-blocking independent work.
3. **Code-review — W1: stale "clippy clean".** My handoff claimed clippy `-D warnings` clean,
   but it failed on `tests/shutdown.rs:25` (`empty_line_after_doc_comments` — a `///` module
   NOTE followed by a blank line). Reviewer caught the real exit 101. Fixed `///`→`//`;
   re-verified `--workspace --all-targets -D warnings` exit 0. (Re-affirms: verify the real
   exit code, never trust a prior "green".)

## Remaining (OPEN, deferred — needs a fresh go-ahead)

**Phase 9.3c — the completion-queue Tree-Borrows fix.** Classified (commit `250cd3f`) as the
same removable-protector class: the worker foreign-writes `executor_scratch.completion_queue`
through a raw ptr derived `&self.executor_scratch.completion_queue as *const _`
(`schedule.rs:916`), a child of `executor_main_loop`/`try_dispatch_ready`'s `&mut self`
protector. 9.3b is **provenance-neutral** to it (per critic M4 — 9.3b retyped only the
transport, not the completion-queue source). Native is clean (494 tests, no UB); the
integrated `miri_schedule_parallel` stays `#[ignore]`d. Fix family = mirror the Phase 9.2
NonNull derivation (derive the completion-queue/pending-apply pointers off a non-`&mut self`
source). Invasive (architect→critic→dev→Miri); not urgent.

## Files
- `crates/boyko_threadpool/src/thread_pool.rs` — `PoolInner`/`ThreadPool` split, `build`,
  `Drop` + `join()` + `shutdown_and_join`, `InstallGuard`, getters forwarding.
- `crates/boyko_threadpool/src/worker.rs` — `worker_main(Arc<PoolInner>)`, TLS deposit,
  `pool`→`inner` rename, helpers retyped to `&PoolInner`.
- `crates/boyko_threadpool/src/scope.rs` — `Scope { inner: &'scope PoolInner }` (Candidate-U
  untouched).
- `crates/boyko_threadpool/src/tls.rs` — `*const PoolInner` TLS + `&PoolInner` accessor.
- `crates/boyko_threadpool/src/lib.rs` — `pub use` adds opaque `PoolInner`.
- `crates/boyko_threadpool/tests/shutdown.rs` — NEW (4 tests).
- `crates/boyko_threadpool/tests/miri_scope.rs` — leak-flag retention note (crossbeam-epoch).
