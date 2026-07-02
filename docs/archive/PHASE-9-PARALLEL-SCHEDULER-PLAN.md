> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9 — Parallel Scheduler — Architecture Plan

**Branch:** `ecs`
**Status:** DRAFT v3 (architect output, Round 3 critic-fix; awaits architecture-critic review)
**Saved to (intended):** `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md`
**Predecessor:** Phase 8.5 (Static Bundle Cache, APPROVED)
**Successor:** Phase 10 (Change Detection — `Tick`, `Added<T>`, `Changed<T>`)

---

## §0 — Changelog

### Round 3 changelog (vs Round 2)

- **C-NEW-1 fixed**: `Access::is_universal()` checks only 4 existing bitmasks. Event lane access OUTSIDE conflict graph (per-lane TLS discipline via EVT1). §12.5 + SCH7 doc updated.
- **C-NEW-2 fixed**: §1.2 dispatcher target relaxed to ≤ 20 µs at 50 sys with apply-cost note. §10.5 cross-ref updated.
- **W-NEW-1 fixed**: §5.4.5 buggy pseudocode deleted; §5.4.5.1 is sole canonical executor loop.
- **W-NEW-2 fixed**: §4.5.5 optimistic "drain own deque" code deleted; §4.6 proof refs only injector path.
- **W-NEW-3 fixed**: §5.5 build signature pre-destructures self; lengths/names captured before move.
- **W-NEW-4 fixed**: §5.4.5.1 gate carries monotonicity note.
- **W-NEW-5 fixed**: §5.4 exclusive path uses `unsafe { cell.world_mut() }` with EXC1 SAFETY block.
- **O-NEW-1 accepted**: §9.2 release-mode enforcement note.
- **O-NEW-2 accepted**: `pred_remaining` changed `Box<[AtomicU16]>` → `Box<[u16]>` (dispatcher-owned).
- **O-NEW-3 accepted**: `debug_assert!` pred_count fits u16 in §5.5.
- **OQ1-OQ3 resolved**: events outside, 20µs target, exclusive SAFETY explicit.

### Round 2 changelog (vs Round 1)

This was a substantive architectural revision in response to Critic Round 1 (9 criticals, 8 warnings, 4 optionals). Every item is mapped to a concrete fix.

#### Criticals (all 9 resolved)

| Id | Issue | Resolution |
|----|-------|------------|
| **C1** | `Arena: Send + Sync` claim contradicts `allocate_*(&self)`; two workers racing in `MemFreeBlockMaster` = UB. | **Adopted Option 3** (hard invariant). New §2.7 invariants **ALLOC1..ALLOC6** define a `NO_ALLOC_IN_SYSTEM` zone. Workers run inside an RAII guard that sets a thread-local `IN_SYSTEM_RUN: bool` flag; `arena.allocate_*` `debug_assert!`s that the flag is unset. All archetype growth, command-queue grow, event-buffer alloc happens in `apply` (dispatcher) or during `ScheduleBuilder::build`. New §9.4 audits every allocation site and routes it. New §2.4 SEND2 rewritten with the corrected contract. New §11.3 documents `ComponentPool::grow` deferral (lazy resolution at apply window). |
| **C2** | `push_task` references per-worker injectors not declared in `ThreadPool` struct. | **Adopted Option 1** (per-worker injectors). New §4.2 adds `injector_local: Arc<[CachePadded<Injector<TaskHandle>>]>` field. Worker main loop drains local injector before global (§4.3 step 1.5). `push_task` on a worker pushes to `pool.injector_local[worker_id]` (§4.5). |
| **C3** | Nested install can deadlock when N workers each call `par_iter` → no spare worker for inner tasks. | **Adopted Option 3** (rayon work-stealing pattern). New §4.5.5 introduces `Scope::join_workers` — while waiting in `Scope::Drop`, the calling thread (whether dispatcher OR worker) actively steals tasks from any scope instead of parking. New `ThreadPool::scope` API distinct from `install` (W6 fix). Workers never park while a scope they nested is alive. New §4.6 contains a deadlock-freedom proof sketch. |
| **C4** | No barrier between running workers and dispatcher `apply` — UB at language level (`&mut EcsMaster` aliases `UnsafeEcsCell` copies). | **Adopted Option 2** (apply window with explicit barrier). New §5.4 redesigns the executor around **dispatch rounds**: a round = (drain pending applies under barrier) → (find ready) → (spawn) → (wait until at least one completes OR all running drain). `apply` is queued, not immediate; **executed only when `running == 0`**. New invariant SCH7 rewritten with pseudocode. New §5.4.5 contains a happens-before diagram. |
| **C5** | `EventDispatcher::send(thread_index)` has no scheduler-level policy for multi-worker. | **Adopted Option 1** (TLS worker id). New §2.8 invariants **EVT1..EVT4**. New §4.4 adds `CURRENT_WORKER_ID: Cell<u32>` populated on worker entry / dispatcher entry. Existing low-level `send(thread_index)` kept; **new user-facing `send_event<E>` wrapper** in `EventDispatcher` reads worker id from TLS. `Commands::send_event` forwards. The lane exclusivity contract is preserved by construction (each worker owns its index). |
| **C6** | `Scope` transmute → panic-during-pending = hang; abort caveat unmentioned. | New §4.5.6 explicitly treats unwinding (`Drop` runs); the abort/SIGKILL caveat is documented in SAFETY block (workers may outlive `'scope` on abort, but the entire process is gone). C3's work-stealing `Scope::Drop` also fixes the panic-while-pending case (calling thread does useful work, not hang). New §13.4.2 loom test enumerates panic-with-pending interleavings. |
| **C7** | Loom test for `unpark_one_idle` race not enumerated. | §13.4 expanded — four labeled scenarios (Race A/B/C/D) with assertions. The "re-poll after mark_idle is load-bearing" rationale added to worker_main comment §4.3. |
| **C8** | `ConflictGraph::build` O(N²) + per-frame find_ready scan exceeds 200 µs target. | **Adopted Option 2** (incremental predecessor counter NOW). New §7.4 introduces `pred_remaining: Box<[u16]>` per schedule (Round 3 O-NEW-2: was `AtomicU16`, downgraded to plain u16 because dispatcher-owned). On completion, dispatcher (NOT worker) decrements each successor's counter; when counter hits 0, system enters `ready_set`. Reduces find_ready from O(N²/frame) to O(|completions| × |avg successors|). §10.5 updated. |
| **C9** | `SystemMeta::is_exclusive` field duplicates `Access::is_universal()`. | **Adopted OQ-4** — dropped `is_exclusive` field from `SystemMeta`. New `Access::is_universal() -> bool` (one-liner; tests the 4 existing bitmasks for "all bits set" per Round 3 C-NEW-1). `SystemBox::is_exclusive` remains as a **build-time cache** populated from `access.is_universal()`. `SystemMeta` size stays at 224 B. New `debug_assert_eq!(systembox.is_exclusive, systembox.system.access().is_universal())` in §13.6. |

#### Warnings (all 8 resolved)

| Id | Issue | Resolution |
|----|-------|------------|
| **W1** | Dual unpark path (worker→dispatcher via ScopeShared::waker AND explicit `dispatcher_thread.unpark()`). | §5.4 — explicit dispatcher unpark **removed**; rely on `ScopeShared::waker` only. Documented in §4.5.4 "single wake-up path". |
| **W2** | `Commands` aliasing inside `par_iter` not invariant-tested. | New CQ-SEND2 invariant in §2.4. New compile_fail test `par_iter_captures_commands_fails.rs` listed in §13.1. |
| **W3** | `EntityMaster::entities_inland` reallocation can dangle worker reads. | Resolved together with C4 — all structural mutation now lives in the apply window (zero workers running). Plus §11.4 EntityMaster pre-sized contract — `EntityMaster::with_capacity(MAX_ENTITIES_HINT)` called at `EcsMaster::new`; `MAX_ENTITIES_HINT` is a build-time config knob defaulting to 64 K. |
| **W4** | `ScheduleBuilder` / `Schedule` pool ownership ambiguity. | Picked: **`ScheduleBuilder::new(pool: Arc<ThreadPool>)` stores pool**. §5.5 struct now has `pool: Arc<ThreadPool>` field; §12.2 signature confirmed. |
| **W5** | `IntoSystem` blanket for exclusive `fn(&mut EcsMaster)` coherence vs existing tuple impls. | New §3 Q9 detailed solution: introduces a fresh marker type `ExclusiveSystemMarker`. Concrete impl signature shown in §3.9.1: `impl<F, Out> IntoSystem<(), Out, ExclusiveSystemMarker> for F where F: FnMut(&mut EcsMaster) -> Out + Send + Sync + 'static`. Disambiguates from `SystemParamFunction<(P1,...)>` marker. |
| **W6** | `par_iter` calls `pool.install` from inside an existing `install` = nested install madness. | §4.5.5 introduces `ThreadPool::scope` (lightweight, usable from a worker without re-entering install TLS bookkeeping). `par_iter`'s `run_par_iter` now calls `scope`, not `install` (§6.2 updated). |
| **W7** | `find_ready` post-condition `debug_assert!` is tautological. | §13.6 updated: replaced with `running ∩ ready_scratch is empty` and `pred_remaining[i] == 0 for all i ∈ ready_scratch`. |
| **W8** | `SmallVec` dep undeclared. | §15.2 Cargo.toml — added `smallvec = "1"` to `boyko_ecs` deps. Alternatively the plan documents `[Option<X>; 2]` as a viable replacement; we go with `smallvec` for ergonomics (Bevy uses it too). |

#### Optionals (all 4 accepted)

| Id | Resolution |
|----|------------|
| **O1** | `XorShift64Star::new(seed)` now applies splitmix64 mixer to `worker_id`. §4.3 line ~402 updated. |
| **O2** | `MIN_ARCHETYPE_FOR_PARALLEL = 1024` adopted in v1. §6.2 updated; archetypes below this run inline on the calling thread. |
| **O3** | `UnsafeEcsCell` minted per-round (after apply window barrier) instead of once per frame. §5.4 updated; natural alignment with C4 apply window. |
| **O4** | §13.5 bench methodology subsection added explaining how "≤ 1% CPU idle" is measured (`getrusage`/`GetProcessTimes` + sampling). |

#### Open questions resolved

- **Q1 (Schedule::Out)**: `Schedule::add_system` doc-comment explicitly states "systems with `Out != ()` must use `EcsMaster::run_system` outside the scheduler". §12.2 includes the comment.
- **Q2 (Phase 8.5 cache reads)**: §15.4 confirms `OnceLock::get(&self)` is safe for many readers; per [std docs](https://doc.rust-lang.org/std/sync/struct.OnceLock.html), `OnceLock<T>: Sync` when `T: Send + Sync`. Read on the dispatcher side only (apply window).
- **Q3 (Arc<ThreadPool> clone per frame)**: Removed. `pool.install` takes `&self`; `Schedule` stores `pool: Arc<ThreadPool>` and dereferences it via `&*self.pool`. No per-frame clone.

---

## §1 — Summary

### 1.1 Goal

Phase 9 delivers a **Bevy-class parallel scheduler** plus **intra-system `par_iter`** on top of a **custom Chase-Lev work-stealing thread pool**. It turns boyko-engine from a single-threaded crate (`run_system` / `run_cached_system`) into a multi-system multi-thread executor that:

1. Runs **independent systems** concurrently when their declared `Access` surfaces do not conflict.
2. Permits a single system to fan its **`Query`** rows across worker threads via `query.par_iter().for_each(|item| ...)`.
3. Auto-inserts **sync points** so `Commands` flush deterministically while preserving the parallelism upstream / downstream.
4. Carries **explicit `unsafe impl Send + Sync`** for `EcsMaster`, `UnsafeEcsCell` — gated by the scheduler's aliasing-discipline contract (§2.4) and the no-allocation-in-system contract (§2.7).
5. **Does not** make `Arena` `Send + Sync` (Round 2 change — see §2.4 SEND2). Allocation is restricted to the dispatcher and to `ScheduleBuilder::build`.

### 1.2 Target metrics (acceptance gates)

| Operation | Target | Source / justification |
|-----------|--------|------------------------|
| Schedule build (50 systems, 200 components, no cycles) | ≤ 50 µs | Tarjan SCC + bitset OR is O(N²/64); 50²/64 = 39 ops × ~10 ns. |
| Per-frame dispatch overhead (50 sys, all parallel, 16 threads) | **≤ 20 µs (≤ 400 ns/sys; apply cost dominates the per-system contribution)** | Phase 9 budget — see §10.5 breakdown. Round 3 C-NEW-2: relaxed from Round 2's optimistic 5 µs; apply hoisting (batched apply path) deferred to a later phase. Bevy single-thread dispatcher saturates at 470 µs for 1000 sys (~470 ns/sys); boyko at ~400 ns/sys still beats Bevy on raw throughput at this scale. |
| Per-frame dispatch overhead (1000 sys, 50 % conflict density, 16 threads) | ≤ 200 µs | Incremental ready set avoids the O(N²) scan. New §10.5 calculation: ~197 µs (within target). |
| Steal cost (worker idle → another worker's deque) | ~100 ns | crossbeam-deque measurement (research §C). We hit the same number — same algorithm. |
| Worker wake-up latency (unparking idle worker) | ≤ 1 µs | std `Thread::unpark` calls `WaitOnAddress` (Windows) / `futex(WAKE)` (Linux). ~500 ns – 1 µs on modern x86_64. |
| `par_iter` per-chunk dispatch cost | ≤ 200 ns | One push to local deque + one wake-poll. Below the soft minimum chunk size (256 rows × ~5 ns body = 1.3 µs work), dispatch overhead is < 15 %. |
| Steady-state worker idle (no work) | ≤ 1 % CPU per core | Park-on-empty after exponential backoff (§4.3). Bench methodology in §13.5.6. |
| Lines of code (LOC) | 6500-9000 production + 3500-5000 test | Phase 8.5 was ~2500 + 1500; Phase 9 covers 2 subsystems + thread pool foundation. |
| Step count | 24 Steps (some parallelisable in pairs) | See §14 (was 22 in Round 1; Round 2 added Step 7c = Access::is_universal + allocation guards). |
| Calendar weeks (single developer, hot work) | 4-6 weeks | Bigger than Phase 8b (which was 14 Steps over ~3 weeks). |

### 1.3 Subsystems delivered

- **A.** `boyko_threadpool` — new sub-crate (lives at `crates/boyko_threadpool/`) hosting workers, Chase-Lev deques, parking, the `ThreadPool::install`/`scope` API.
- **B.** `Query::par_iter` / `Query::par_iter_mut` — fork-join driver inside `boyko_ecs` that bounces archetype chunks across the pool.
- **C.** `Schedule` + `ScheduleBuilder` + `ConflictGraph` + `Executor` — multi-system runner; lives at `crates/boyko_ecs/src/ecs/core/schedule/`.
- **D.** Ordering API (`.before`, `.after`, `.chain`, `.in_set`, `SystemSet`).
- **E.** Auto-`ApplyDeferred` insertion analyzer.
- **F.** Exclusive-system wrapper (`ExclusiveFunctionSystem` adapter for `fn(&mut EcsMaster)`).
- **G.** Debug instrumentation behind `#[cfg(feature = "scheduler-trace")]` flag — no `tracing` dep (justification §3 Q12).
- **H.** Send/Sync contract for `EcsMaster`, `UnsafeEcsCell`, `Resources`, `EntityMaster`, `ArchetypeMaster`, `EventDispatcher`, `ComponentPool`, `Archetype`. **NOT** for `Arena`.
- **I.** `Access::is_universal()` helper (new).
- **J.** `EventDispatcher::send_event<E>(&self, event: E)` — new convenience wrapper that reads worker_id from TLS.

### 1.4 What Phase 9 deliberately defers

- **Change detection** (`Tick`, `Added<T>`, `Changed<T>`) — Phase 10.
- **NUMA-aware deque placement** — single-socket target hardware; no win.
- **Async / cooperative awaits** — game frame loop is run-to-completion.
- **Pipeline staging** (Bevy's first/pre/run/post/last) — emergent from `.in_set(...) + .chain()`.
- **Resource conflict-free scopes** (Bevy's `Local<T>`) — already in Phase 8a.
- **Lock-free arena allocator** — large rework; not needed if no-alloc-in-system discipline holds. Reopen if profiling shows the dispatcher's apply work dominates frame time.
- **Batched apply path** (per-frame apply queue serviced once at end of round instead of per-system) — would let us hit a much tighter dispatcher budget at 50 systems (Round 3 C-NEW-2). Deferred to a later phase once 20 µs proves insufficient in profiling.

---

## §2 — Invariants

Naming scheme: `TPN` = thread pool, `SCH` = scheduler, `PAR` = par_iter, `SEND` = Send/Sync gate, `EXC` = exclusive system, `INS` = sync-point insertion, `ALLOC` = allocation discipline (Round 2), `EVT` = event dispatcher (Round 2).

### 2.1 Thread pool (TPN1..TPN13)

- **TPN1** — Each worker thread is created **exactly once** at `ThreadPool::new(n)`; threads are joined only at `ThreadPool::drop`.
- **TPN2** — Each worker owns exactly one Chase-Lev `Worker<TaskHandle>` deque + one `Injector<TaskHandle>` local injector (Round 2; see §4.2). The deque's `Stealer<TaskHandle>` is published once in the `Arc<[Stealer; N]>` registry; the local injector is published once in `Arc<[Injector; N]>`.
- **TPN3** — Workers push to their **own** deque (bottom) only. Stealing happens from the top via the `Stealer` registry. Single-producer / multi-consumer guarantee from Chase-Lev.
- **TPN4** — When the local deque is empty, the worker drains:
  1. Its own **local injector** (`pool.injector_local[worker_id]`) — destination for pushes from another worker that decided to target this worker (e.g., cache-locality hint or because the originating worker is itself).
  2. The **global injector** (`pool.injector`).
  3. **Steal** from N-1 siblings in randomized order via per-worker `XorShift64Star` seeded by splitmix64(worker_id) (Round 2 O1).
- **TPN5** — Backoff escalation: `spin (≤ 6 iters, PAUSE)` → `yield (≤ 32 iters, sched_yield/SwitchToThread)` → `park (futex/WaitOnAddress)`. Crossbeam_utils::Backoff defaults.
- **TPN6** — A parked worker wakes when any other thread calls `worker.unparker().unpark()`. The push paths call `unpark_one_idle()` when targeting the global injector; pushes targeting a specific local injector unpark that worker directly. Pushes to a worker's own deque do NOT wake siblings (the worker is its own consumer).
- **TPN7** — `unpark_one_idle()` is a lock-free O(1) probe of the idle bitset (`AtomicU64` for ≤ 64 workers). Uses `compare_exchange_weak` to clear one set bit; on success, unparks the corresponding worker.
- **TPN8** — `ThreadPool::install(F)` blocks the calling thread until `F` returns. The closure runs on the calling thread (Rayon pattern). Any task spawned via `Scope::spawn` runs on workers; `install` returns only after every spawned task has joined. **Round 2 W6**: a separate `ThreadPool::scope` API exists for use from inside a worker; see §4.5.5.
- **TPN9** — Panic in a worker task is captured into `Arc<Mutex<Option<Box<dyn Any + Send>>>>` on the scope. Cold path; Mutex is acceptable (panics are rare). On `install`/`scope` return, the first captured panic is re-`resume_unwind`-ed on the caller's thread.
- **TPN10** — A worker thread MUST NOT hold any reference into `EcsMaster` across a `park` call. Statically enforced: the only `EcsMaster` access path is through `UnsafeEcsCell<'w>`, which is bound to `'w`; a parked worker has no active task and therefore no live `'w`.
- **TPN11** — Worker affinity is **not pinned by default**. Override: `ThreadPoolBuilder::pin_workers(true)`.
- **TPN12** — The thread pool is `'static`. `Scope::spawn` permits non-`'static` borrows via lifetime erasure (§4.5).
- **TPN13** (Round 2) — Each worker thread maintains a TLS `CURRENT_WORKER_ID: Cell<u32>` set on entry (§4.4). The dispatcher thread sets its TLS to `u32::MAX - 1` (sentinel). User code reads via `current_worker_id()`; reserved sentinel `u32::MAX` means "not on any worker, not the dispatcher".

### 2.2 Scheduler (SCH1..SCH15)

- **SCH1** — A `Schedule` is built once via `ScheduleBuilder::build(world: &mut EcsMaster) -> Schedule`. Building runs every system's `System::initialize` exactly once and freezes the `Access` surfaces. **No system addition / removal after build** in Phase 9.
- **SCH2** — At build time, the builder validates that the dependency graph (after expansion of `.in_set`/`.chain`/`.before`/`.after`) is acyclic. Cycles trigger a `boyko-B9001` panic with the offending SCC's system names.
- **SCH3** — Two systems `A` and `B` can run concurrently iff `!A.access().conflicts_with(B.access()) && B is not transitively dependent on A`. The `ConflictGraph` precomputes the AND of these two predicates as a single `FixedBitSet` per system.
- **SCH4** — A `Schedule::run(&mut EcsMaster)` invocation is a single frame. Concurrent `Schedule::run` calls on the same `EcsMaster` are forbidden — the `&mut EcsMaster` borrow enforces this trivially.
- **SCH5** — The executor is single-threaded **as a dispatcher** but multi-threaded **as a worker farm**. The dispatcher runs on the calling thread inside `ThreadPool::install`; workers pick up `SystemTaskHandle`s.
- **SCH6** — Within a single `Schedule::run`, every system runs **exactly once**.
- **SCH7** (Round 2 — rewritten, Round 3 — clarified) — A system's `apply` is **NEVER** called concurrently with another system OR with any worker. The executor enforces this via the **apply window** (§5.4.5):
  - Workers running concurrent systems execute `run_unsafe(cell_copy)` only.
  - Completion notifications push to `completion_queue`.
  - The dispatcher's main loop **never** drains `completion_queue` for apply purposes while `running.count_ones() > 0`. Instead it waits (`park_timeout` on `ScopeShared::waker`) until at least one round finishes.
  - When `running.count_ones() == 0`: dispatcher enters the **apply window**: drains `completion_queue`, runs each completed system's `apply(&mut world)` sequentially, then advances `pred_remaining` counters for successors.
  - The cell is **dropped** at the end of the previous round and **re-minted** for the next round (Round 2 O3) — provides a fresh borrow stack per round.

  **Round 3 note on event lane access**: event lane access is OUTSIDE the schedule's conflict graph. Per-lane single-writer discipline (EVT1 TLS routing) guarantees correctness without graph participation: each worker writes only to its own lane index (its TLS-derived `worker_id`); the dispatcher writes to lane `worker_count`. Two systems running concurrently on different workers cannot collide on event writes because they write to different lanes by construction. ApplyDeferred (universal `Access`) blocks every other system equally, so event lane safety during apply is preserved without any event field in `Access`. Phase 12 EventReader/EventWriter SystemParam will revisit if a richer event Access surface is needed.

  Pseudocode in §5.4.

- **SCH8** — `Schedule::run` is idempotent across frames; per-system cached `State` and per-system `CommandQueue` outlive single frames.
- **SCH9** — Execution **order** between two systems that are neither ordered nor in conflict is **non-deterministic**.
- **SCH10** — A system added via `.add_system(IntoSystem)` is stored as `Box<dyn System<Out = ()>>` in `SystemBox`. Non-`()` output systems must use `EcsMaster::run_system` outside the scheduler. Doc-comment on `Schedule::add_system` states this explicitly (Round 2 Q1).
- **SCH11** — `Schedule::run` panics on the calling thread if any system panicked. First-observed panic re-raised; remaining systems may have started (Bevy semantic).
- **SCH12** — The conflict bitset is `Box<[FixedBitSet]>` indexed by `SystemIndex` (a fresh `u16` newtype). For N = 1024, total 128 KB. Fits L2 of any modern CPU.
- **SCH13** (Round 2 — rewritten, Round 3 — type update) — The "ready" set is maintained **incrementally** via `pred_remaining: Box<[u16]>` (Round 3 O-NEW-2: plain `u16`, not `AtomicU16`, because dispatcher is the sole mutator) and a small `ready_pending: VecDeque<SystemIndex>`. See §7.4. The per-frame O(N²) scan from Round 1 is **eliminated**.
- **SCH14** — `Schedule` is `!Send + !Sync` (it owns `Box<dyn System>` whose impls are `Send + Sync` but the schedule itself is mutated by `run`).
- **SCH15** (Round 2) — `SystemBox::is_exclusive` is a build-time cache computed from `access.is_universal()` (Round 2 C9 / OQ-4). No `SystemMeta::is_exclusive` field. `debug_assert_eq!(box.is_exclusive, box.system.access().is_universal())` in `Schedule::run` preamble.

### 2.3 `par_iter` (PAR1..PAR9)

- **PAR1** — `Query::par_iter()` / `par_iter_mut()` returns a `ParQuery<'q, 's, D, F>` cursor. `.for_each(f)` consumes the cursor, dispatches archetype chunks to the active `ThreadPool`, blocks until all chunks complete.
- **PAR2** — The closure passed to `.for_each(f)` is `F: Fn(D::Item<'_>) + Send + Sync`. `Fn` (not `FnMut`) because it runs on multiple threads simultaneously.
- **PAR3** — `D::Item<'_>: Send` is required.
- **PAR4** — Chunk size: `chunks_per_archetype = max(1, archetype.entity_count() / soft_chunk_size)` with `MIN_CHUNK_SIZE = 256`, `batches_per_thread = 1`.
- **PAR5** — Within one chunk, rows are processed sequentially.
- **PAR6** — `ParQuery::for_each` must **not** be called from within another `par_iter` body. Enforcement via `IN_FORK_JOIN: Cell<bool>` TLS; nested entry panics with `boyko-B9002`.
- **PAR7** — `par_iter` must run inside `Schedule::run` or a manual `pool.install`/`pool.scope` scope. No active pool → `boyko-B9003`.
- **PAR8** — `par_iter_mut` accepts any `D: QueryData`; `par_iter` requires `D: ReadOnlyQueryData`.
- **PAR9** (Round 2 O2) — Archetypes with `entity_count() < MIN_ARCHETYPE_FOR_PARALLEL = 1024` are processed **inline** on the calling thread (no `scope.spawn`). For an iteration over an archetype with N=10 rows the dispatch cost would dominate (~200 ns spawn vs ~50 ns work).

### 2.4 Send/Sync gate (SEND1..SEND10) — Round 2 substantially revised

- **SEND1** — `EcsMaster` becomes `unsafe impl Send + Sync`. Justification: every accessible field is either already `Send + Sync` (`Resources`, `EventDispatcher`, `EntityMaster`, `ArchetypeMaster`, `bundle_archetype_cache`) or is `Arena` which is **NOT** Send/Sync but is **never accessed concurrently** under the no-alloc-in-system contract (ALLOC1..6, §2.7). The `Box<Arena>` provides a stable heap address; the inner `Arena` remains single-threaded but is touched only on the dispatcher.

  **SAFETY**: see §9.2 for the complete contract. The unsafe impl is **gated by ALLOC1**: any allocation site that takes `&Arena` (today: `allocate_layout`, `allocate_from_free_blocks`) is wrapped in a debug guard checking `IN_SYSTEM_RUN == false`. If a system's `run_unsafe` body invokes such an allocation, the debug build panics; release build silently UBs (acceptable because the discipline is enforced at the layer above — see §9.4 audit).

- **SEND2** (Round 2 — REWRITTEN) — `Arena` **remains `!Send + !Sync`**. We do NOT add `unsafe impl Send + Sync for Arena`. The Round 1 claim was wrong (the actual `allocate_*` signatures take `&self` and reach into `UnsafeCell<MemFreeBlockMaster>`; two threads = race = UB). Instead:

  - `EcsMaster: Send + Sync` is justified because the `Arena` inside is never touched concurrently. The borrow-checker doesn't enforce this (it sees a `Box<Arena>` containing an `UnsafeCell`), so we rely on:
    1. The scheduler invariant ALLOC1: no allocation may occur inside `System::run_unsafe`.
    2. The thread-local `IN_SYSTEM_RUN: Cell<bool>` flag set by the worker's RAII guard before calling `run_unsafe` and cleared on return.
    3. `arena.allocate_layout` / `arena.allocate_from_free_blocks` `debug_assert!(!IN_SYSTEM_RUN.get())`.

  All current call sites of `arena.allocate_*` (Round 2 audit §9.4) are reachable only from `apply` paths or from `ScheduleBuilder::build`, both of which run with `IN_SYSTEM_RUN == false`.

  - Marker for compiler: since `Arena` contains `NonNull<u8>` (`!Send + !Sync`) and `UnsafeCell` (`!Sync`), the auto-derive makes it `!Send + !Sync`. We don't override.
  - For `EcsMaster` to be `Send + Sync`, we use `unsafe impl Send` and `unsafe impl Sync` with the SAFETY comment in §9.2 that documents the discipline.

- **SEND3** — `UnsafeEcsCell<'w>` becomes `unsafe impl<'w> Send + Sync`. Justification: cell holds only `*mut EcsMaster` + `PhantomData<&'w mut EcsMaster>`. The raw pointer is Send+Sync because access through it is governed by the scheduler's aliasing-discipline contract (SCH3, SP1, SP2, SP3) plus the apply-window contract (SCH7). Each worker dereferences a cell copy to construct `&EcsMaster` (read-only) or `&mut EcsMaster` (exclusive system only). The cell is `Copy`, so transmitting copies across threads is free.

- **SEND4** — `EventDispatcher` becomes `Send + Sync`. Internally lock-free for `send` (per-thread lanes). Round 2: `send_event<E>(&self, event)` wrapper reads `current_worker_id()` from TLS (§2.8 EVT1).

- **SEND5** — `EntityMaster` becomes `Send + Sync`. Hot path is `&self` reads (`get_component_raw`, `has_entity`); `&mut self` paths only at apply window. **Round 2 W3 mitigation**: `EntityMaster::with_capacity(64_000)` called at `EcsMaster::new` to pre-grow internal `Vec`s; further `push` paths still possible in the apply window but never with workers running.

- **SEND6** — `ArchetypeMaster` becomes `Send + Sync`. `&self` reads concurrent-safe; `&mut self` (new archetype creation) gated by structural-mutation conflict (runs as part of apply window with no workers).

- **SEND7** — `CommandQueue` is `Send + !Sync` (Phase 8d CQ-SEND1). Per-system, single-writer.

  **CQ-SEND2 (Round 2 W2)**: `Commands::add` takes `&mut self`. `Commands<'s>` is `!Sync` because the underlying `RawCommandQueue` is `!Sync`. Therefore capturing `&mut Commands` in a `par_iter`'s `Fn(D::Item) + Send + Sync` closure is **type-system-rejected**. Compile_fail test `tests/par_iter_captures_commands_fails.rs` verifies.

- **SEND8** — `Box<dyn System<Out=()>>` is `Send + Sync` iff `System: Send + Sync + 'static`.

- **SEND9** — `!Send + !Sync` types enumerated:
  - `Arena` (Round 2 — kept !Send + !Sync as the fundamental atom; access discipline enforced one layer up).
  - `RawCommandQueue`.
  - `Schedule` (owns `Box<dyn System>` mutated by `run`).
  - `ScheduleBuilder` (built on main thread).
  - `QueryIter` / `QueryIterMut` (per-thread cursor).
  - `ParQuery` / `ParQueryMut` (transient).
  - `Commands<'s>` (`!Sync` per CQ-SEND2).

  Static-assertion file `tests/send_sync_negative.rs` uses `static_assertions = "1.1"`.

- **SEND10** (Round 2) — `ComponentPool` and `Archetype` become `Send + Sync`. The `*const Arena` field in `ComponentPool` is treated as opaque (it's never dereferenced for allocation from a worker; the dispatcher mediates). Specifically, `ComponentPool::grow` is **never called from a worker**; it's only called from `Archetype::create_entity` which only runs in apply window (§11.3 audit).

### 2.5 Exclusive system gate (EXC1..EXC4) — Round 2 revised, Round 3 W-NEW-5 SAFETY block expanded

- **EXC1** (Round 3 — SAFETY block formalized) — Exclusive system has type `fn(&mut EcsMaster) -> Out` (or `FnMut(&mut EcsMaster) -> Out + Send + Sync + 'static`). Wrapped via `ExclusiveFunctionSystem<F>`. Its declared `Access` is `Access::universal()` (a new constructor; sets every bit in the 4 existing bitmasks).

  **SAFETY (EXC1 — exclusive system body):**
  - Universal access conflicts with every other system per `Access::conflicts_with`; the conflict graph guarantees no concurrent worker is running when an exclusive system body executes (gate: `running.count_ones() == 0` before dispatcher invokes the body).
  - The dispatcher reborrows `&mut EcsMaster` via `unsafe { cell.world_mut() }` for both the body invocation AND the apply call. `UnsafeEcsCell::world_mut` (`unsafe_ecs_cell.rs:157-170`) is the documented helper.
  - **The exclusive system body must NOT retain any cell-derived borrow past return.** It receives `&mut EcsMaster` for the duration of one call; after return, the dispatcher reborrows for `apply` from the same cell. If the body stashed a raw pointer, the apply reborrow would alias.
  - The cell itself was minted from the dispatcher's `&mut world` in the current dispatch round; no aliasing exists with any other reference at the moment of `cell.world_mut()`.

- **EXC2** (Round 2 — REWRITTEN, OQ-4 / C9 resolution) — The dispatcher recognizes exclusive systems via `SystemBox::is_exclusive` (build-time cache computed from `access.is_universal()`). No field on `SystemMeta`. When the executor picks an exclusive system, it waits for `running.count_ones() == 0`, runs it on the dispatcher thread.
- **EXC3** — `ExclusiveFunctionSystem::run_unsafe` constructs `&mut EcsMaster` from `cell.world_mut()`. Cell must be write-capable.
- **EXC4** — `ApplyDeferred` is a built-in exclusive system. Its body is `|world: &mut EcsMaster| flush_all_pending_commands(world)`.

### 2.6 Sync-point insertion (INS1..INS5)

Unchanged from Round 1.

- **INS1** — A sync point is an `ApplyDeferred` system inserted into the schedule's DAG.
- **INS2** — Analyzer runs at `ScheduleBuilder::build` time.
- **INS3** — Coalesce successive `A → B` edges that share an upstream cone.
- **INS4** — `.no_sync()` annotation skips a sync; UB-free but may produce stale reads.
- **INS5** — Frame-end always applies any remaining commands.

### 2.7 Allocation discipline (ALLOC1..ALLOC6) — Round 2 new

This subsection encodes the Round 2 C1 resolution. The rules are **necessary for EcsMaster: Send + Sync** to be sound given that `Arena` remains `!Send + !Sync`.

- **ALLOC1** — **No allocation may occur inside `System::run_unsafe`.** A system's `run_unsafe` body may read components (via `&` to `&Arena` indirectly through `ComponentPool::base_ptr`), write components (mut writes within a single `ComponentPool`'s already-allocated bytes), enqueue commands (`Commands::add` → `RawCommandQueue::push` which **may grow** — but `CommandQueue::grow` allocates on the global allocator, NOT in the arena; see ALLOC3).

- **ALLOC2** — **All `Arena::allocate_*` calls happen on the dispatcher thread**, in one of these contexts:
  - `ScheduleBuilder::build` (one-shot, before any worker runs).
  - `apply` window (workers fully drained; `running == 0`).
  - Outside the scheduler entirely (`EcsMaster` direct manipulation pre-`Schedule::run`).

- **ALLOC3** — `RawCommandQueue::push` may invoke `Vec::reserve` for its backing buffer. **This allocates on the global allocator**, not the arena. Global-allocator allocations are thread-safe (`GlobalAlloc::alloc` is `Sync`). No restriction.

- **ALLOC4** — `EventBuffer::send` and `EventBuffer::send_many` operate on **preregistered, fixed-capacity** lanes. Preregistration happens during `EcsMaster::new`/init (before any scheduler runs); no allocation during `send`. The `boyko-B9000` family of errors covers overflow.

- **ALLOC5** — `EntityMaster::register_entity_with_ptr` may `push` to `entities_inland` Vec; potentially reallocates. **Routes through apply window** because it's only called from `Commands::spawn`'s apply path or from the test-only `EcsMaster::create_entity` (which takes `&mut self`).

- **ALLOC6** — Debug-build enforcement: `Arena::allocate_layout` and `Arena::allocate_from_free_blocks` `debug_assert!(!IN_SYSTEM_RUN.get(), "ALLOC1 violation: arena allocation inside System::run_unsafe")`. The `IN_SYSTEM_RUN: Cell<bool>` TLS is set by the worker's RAII guard in `worker_task_run_system` (§4.4). Release builds skip the check; the discipline is enforced at the build-system layer (CI runs debug tests including stress runs that exercise spawn → allocation paths; see Round 3 O-NEW-1 note in §9.2).

### 2.8 EventDispatcher worker_id (EVT1..EVT4) — Round 2 new

This subsection encodes the Round 2 C5 resolution.

- **EVT1** — A new public API `EventDispatcher::send_event<E: Event>(&self, event: E) -> EcsResult<()>` wraps the existing `send(&self, thread_index: u32, event: E)`. It reads `current_worker_id() -> u32` from TLS:
  ```rust
  pub fn send_event<E: Event>(&self, event: E) -> EcsResult<()> {
      let tid = boyko_threadpool::current_worker_id_or_dispatcher_lane();
      self.send::<E>(tid, event)
  }
  ```
  The TLS helper returns:
  - The worker's index (0..N-1) when called on a worker.
  - `0` when called on the dispatcher (Round 2 design choice: dispatcher uses lane 0; the design assumes lane 0 has capacity for dispatcher + worker 0 writes, **OR** the user preregisters with `thread_count = worker_count + 1` and the dispatcher uses lane `worker_count`).

  **Decision**: dispatcher uses lane `worker_count` (extra lane). Default `EventConfig::default_for(thread_count)` will be updated to pass `worker_count + 1`. Documented in §12.4.

- **EVT2** — `Commands::send_event(&mut self, event: E)` enqueues a `SendEventCommand<E>` into the `CommandQueue`. On apply (dispatcher), it calls `world.event_dispatcher.send::<E>(WORKER_DISPATCHER_LANE, event)`. This is the **default user-facing path** — commands are the universal write channel.

- **EVT3** — Direct `world.event_dispatcher().send_event::<E>(event)` (no Commands indirection) is allowed for advanced users running on workers; the per-thread lane invariant is preserved because each worker uses its own lane (read from TLS).

- **EVT4** — `EventBuffer::send_one` and `send_many` debug-assert `thread_index < self.thread_count`. Out-of-range index panics in debug; silently UB in release (same as today). Acceptable because the TLS values are bounded by `MAX_WORKERS + 1`.

---

## §3 — Decision matrix (the 12 architectural questions)

For each question: **chosen**, **alternatives rejected with reason**, **justification**.

### Q1. Custom thread pool vs wrap crossbeam-deque?

**Chosen:** **Custom thread pool built ON TOP OF the `crossbeam-deque` crate** (use `Worker<T>` + `Stealer<T>` + `Injector<T>` as primitives; build everything above them ourselves).

**Alternatives rejected:**
- **(a) Pure roll-our-own Chase-Lev deque.** ~500 LOC of formally-tricky lock-free code + memory reclamation. crossbeam-deque is formally verified (Lê et al. 2013, plus loom tests).
- **(b) Wrap rayon-core wholesale.** rayon-core's panic/install/scope semantics conflict with our model.
- **(c) std::sync::mpsc + std::thread.** No work stealing; serializes on Mutex<VecDeque>.

**Justification:** crossbeam-deque provides the verified primitives; we layer worker threads, parking, scope, panic handling, install API on top.

### Q2. Phase-based vs DAG-based scheduling?

**Chosen:** **Pure DAG-based** (Bevy 0.10+ style).

Rejected: phase-based (over-serializes), hybrid (unstable mental model).

### Q3. Where flush Commands?

**Chosen:** **Auto-insert sync points at DAG edges where deferred → structural-read** (Bevy auto-insert algorithm).

Rejected: per-system flush (cache-poisoning), per-phase flush (couples to phases), frame-end-only (breaks intra-frame contract).

### Q4. Parallel dispatcher vs serial?

**Chosen:** **Serial dispatcher** with hard cap `MAX_SYSTEMS_PER_SCHEDULE = 1024`.

Round 2 update: the dispatcher cost target (≤ 200 µs at N=1024) is now hit via the incremental ready-set optimization (§7.4), not deferred to Phase 9.1.

### Q5. Send + Sync of EcsMaster — direct vs wrap in SchedulerEcsCell?

**Chosen:** **Direct `unsafe impl Send + Sync for EcsMaster`** with documented contract.

Round 2 update: contract now includes ALLOC1..ALLOC6 (no allocation in system bodies). `Arena` itself stays `!Send + !Sync`; the discipline ensures it's only touched from the dispatcher.

### Q6. par_iter chunk policy?

**Chosen:** `chunk_size = max(MIN_CHUNK_SIZE, total_rows / (worker_count × batches_per_thread))` with `MIN_CHUNK_SIZE = 256`, `batches_per_thread = 1`. Per-archetype chunking.

Round 2 addition (O2): archetypes with `entity_count() < MIN_ARCHETYPE_FOR_PARALLEL = 1024` run inline on the calling thread.

### Q7. Worker affinity?

**Chosen:** **Default: NO pinning. Override via `ThreadPoolBuilder::pin_workers(true)`.**

### Q8. NUMA awareness?

**Chosen:** **Defer entirely.** Single-socket target hardware.

### Q9. Exclusive systems API? (Round 2 W5 — expanded)

**Chosen:** **`fn(&mut EcsMaster) -> Out` wrapped via `IntoSystem` blanket impl into `ExclusiveFunctionSystem<F>` using a fresh marker type `ExclusiveSystemMarker`.**

#### Q9.1 — Concrete impl signatures (Round 2 W5 resolution)

Phase 8c's existing `IntoSystem` for parameter-based systems uses tuple markers:
```rust
// Phase 8c (existing) — schematic
impl<F, Out> IntoSystem<(), Out, ()> for F
where F: FnMut() -> Out + Send + Sync + 'static
{
    type System = FunctionSystem<F, ()>;
    fn into_system(self) -> Self::System { /* ... */ }
}

impl<F, Out, P0> IntoSystem<(), Out, (P0,)> for F
where F: SystemParamFunction<(P0,), Out = Out> + Send + Sync + 'static,
      P0: SystemParam
{
    type System = FunctionSystem<F, (P0,)>;
    fn into_system(self) -> Self::System { /* ... */ }
}
// ... up to (P0,..,P15)
```

The new Phase 9 impl introduces a **distinct marker** so the coherence checker doesn't see overlap:
```rust
/// Marker type used by `IntoSystem` impls that take `&mut EcsMaster`
/// directly (exclusive systems). The marker has no fields; it exists
/// purely to give the coherence checker a distinguishable type.
pub struct ExclusiveSystemMarker;

impl<F, Out> IntoSystem<(), Out, ExclusiveSystemMarker> for F
where
    F: FnMut(&mut EcsMaster) -> Out + Send + Sync + 'static,
    Out: Send + Sync + 'static,
{
    type System = ExclusiveFunctionSystem<F>;
    fn into_system(self) -> Self::System {
        ExclusiveFunctionSystem::new(self)
    }
}
```

#### Q9.2 — Coherence proof

Existing markers: `()`, `(P0,)`, `(P0, P1)`, ..., `(P0,...,P15)`. All are tuples.
New marker: `ExclusiveSystemMarker` (a unit struct in our crate).

The coherence checker rejects overlapping impls when two impls could match the same `F` with the same marker. The tuple markers and `ExclusiveSystemMarker` are distinct nominal types in distinct namespaces; no overlap.

Edge case: could a user write `fn(&mut EcsMaster)` AND have it match a tuple impl? The tuple impls require `F: SystemParamFunction<...>`; `SystemParamFunction` is implemented for `F: FnMut(P1, ..., PN) -> Out` where each `Pi: SystemParam`. `&mut EcsMaster` is **not** a `SystemParam` (intentionally; the design decision is to use the dedicated `Commands` / `Res` / `Query` params instead). Therefore no `SystemParamFunction` impl matches `fn(&mut EcsMaster)`. The exclusive marker is the only viable resolution.

Compile-time test in `tests/into_system_exclusive_smoke.rs` confirms inference works.

**Alternatives rejected:**
- **(a) Wrapper newtype `Exclusive<F>`.** User has to know to wrap; silent races on missing wrappers.
- **(b) Marker SystemParam `World`** (Bevy's `&mut World`). Confusing — a param that "grabs everything" is special-cased.

### Q10. Stealing batch size?

**Chosen:** crossbeam-deque default (`Stealer::steal_batch_and_pop`: half of source).

### Q11. Wake-up protocol?

**Chosen:** **`std::thread::park` / `Thread::unpark`** (one Thread handle per worker).

### Q12. Debug instrumentation surface?

**Chosen:** Feature flag `scheduler-trace` (off by default); lightweight in-tree counters. No `tracing` dep.

---

## §4 — Custom thread pool — deep dive

### 4.1 Crate split

A new sub-crate `boyko_threadpool` at `crates/boyko_threadpool/`. Dependencies:
- `crossbeam-deque = "0.8"`
- `crossbeam-utils = "0.8"`

### 4.2 ThreadPool struct (Round 2 — revised)

```rust
// crates/boyko_threadpool/src/pool.rs

use crossbeam_deque::{Injector, Stealer, Worker};
use crossbeam_utils::CachePadded;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, AtomicBool, Ordering};
use std::thread::{self, JoinHandle, Thread};

pub const MAX_WORKERS: usize = 64;

pub struct TaskHandle {
    pub(crate) body: Box<dyn FnOnce() + Send + 'static>,
    pub(crate) parent_scope: *const ScopeShared,
}

// SAFETY (TPN12): body is Send + 'static. parent_scope outlives the task by
//   the Scope::Drop blocking contract (see §4.5.5).
unsafe impl Send for TaskHandle {}

#[repr(C)]
pub struct ThreadPool {
    /// Global injector — destination for non-worker pushes (the dispatcher
    /// thread pushes here; idle workers steal from here second).
    injector: CachePadded<Injector<TaskHandle>>,

    /// Per-worker local injectors (Round 2 C2). A worker thread pushes
    /// to its own local injector when targeting itself (cache locality),
    /// or to another worker's local injector when the target is named.
    /// Workers drain their own local injector before checking the global
    /// injector. Indexed by worker_id.
    injector_local: Arc<[CachePadded<Injector<TaskHandle>>]>,

    /// Stealer registry — every worker exposes its Stealer here.
    stealers: Arc<[Stealer<TaskHandle>]>,

    /// Per-worker control blocks.
    workers: Box<[CachePadded<WorkerControl>]>,

    /// Idle bitset — bit i is 1 iff worker i is parked.
    idle: CachePadded<AtomicU64>,

    /// Set true when ThreadPool::drop begins shutdown.
    shutdown: CachePadded<AtomicBool>,

    /// Join handles — kept for ThreadPool::drop.
    join_handles: Box<[JoinHandle<()>]>,

    /// Active-scope counter — sanity check at drop time.
    active_scopes: AtomicUsize,

    /// Worker count cached for read.
    num_workers: usize,
}

#[repr(C)]
pub(crate) struct WorkerControl {
    pub(crate) thread: Thread,
    #[cfg(feature = "scheduler-trace")]
    pub(crate) steals_attempted: AtomicU64,
    #[cfg(feature = "scheduler-trace")]
    pub(crate) steals_succeeded: AtomicU64,
    #[cfg(feature = "scheduler-trace")]
    pub(crate) park_nanos: AtomicU64,
    #[cfg(feature = "scheduler-trace")]
    pub(crate) work_nanos: AtomicU64,
}
```

**Layout sizes** (release, no scheduler-trace):
- `WorkerControl`: 8 B → padded to 64 B. Per worker: 64 B.
- `ThreadPool`:
  - `injector`: 128 B (CachePadded around ~64 B Injector).
  - `injector_local`: 16 B (Arc fat ptr); heap-side `MAX_WORKERS × 128 B` = 8 KB.
  - `stealers`: 16 B.
  - `workers`: 16 B; heap-side `MAX_WORKERS × 64 B` = 4 KB.
  - `idle`: 128 B.
  - `shutdown`: 128 B.
  - `join_handles`: 16 B.
  - `active_scopes`: 8 B.
  - `num_workers`: 8 B.
  - Total head: ~480 B (~8 lines). Heap-side: 12 KB.

### 4.3 Worker main loop (Round 2 — revised)

```rust
fn worker_main(
    worker_id: usize,
    deque: Worker<TaskHandle>,
    pool: Arc<ThreadPool>,
) {
    // Round 2 O1: splitmix64-mixed seed.
    let seed = splitmix64(worker_id as u64);
    let mut rng = XorShift64Star::new(seed);

    // Round 2 TPN13: TLS worker id setup.
    crate::tls::CURRENT_WORKER_ID.with(|c| c.set(worker_id as u32));
    crate::tls::ACTIVE_POOL.with(|c| c.set(&*pool as *const _));

    loop {
        // 1. Local deque — pop bottom (LIFO recency).
        if let Some(task) = deque.pop() {
            execute_task(task, worker_id, &pool);
            continue;
        }

        // 1.5. Local injector (Round 2 C2) — drain pushes targeted at us.
        match pool.injector_local[worker_id].steal_batch_and_pop(&deque) {
            crossbeam_deque::Steal::Success(task) => {
                execute_task(task, worker_id, &pool);
                continue;
            }
            crossbeam_deque::Steal::Empty => {}
            crossbeam_deque::Steal::Retry => continue,
        }

        // 2. Global injector — drain a batch.
        match pool.injector.steal_batch_and_pop(&deque) {
            crossbeam_deque::Steal::Success(task) => {
                execute_task(task, worker_id, &pool);
                continue;
            }
            crossbeam_deque::Steal::Empty => {}
            crossbeam_deque::Steal::Retry => continue,
        }

        // 3. Steal from siblings in randomized order.
        if let Some(task) = try_steal_random(&pool.stealers, worker_id, &deque, &mut rng) {
            execute_task(task, worker_id, &pool);
            continue;
        }

        // 4. Backoff and park. The re-poll between mark_idle and park is
        //    LOAD-BEARING: a pusher that lands a task after our last poll
        //    but before our mark_idle would be invisible to unpark_one_idle
        //    (idle bit not yet set). The re-poll catches it (Race C in §13.4).
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            // Re-poll local + local_inj + global + steal before park.
            if let Some(task) = deque.pop()
                .or_else(|| try_pop_local_injector(&pool.injector_local[worker_id], &deque))
                .or_else(|| try_pop_global(&pool.injector, &deque))
                .or_else(|| try_steal_random(&pool.stealers, worker_id, &deque, &mut rng))
            {
                execute_task(task, worker_id, &pool);
                break;
            }

            if backoff.is_completed() {
                mark_idle(&pool.idle, worker_id);
                // Re-poll AGAIN after mark_idle. This closes the race where
                // a pusher reads idle == 0 right before we set our bit.
                // (TPN6 Race C; loom test in §13.4.1).
                if let Some(task) = deque.pop()
                    .or_else(|| try_pop_local_injector(&pool.injector_local[worker_id], &deque))
                    .or_else(|| try_pop_global(&pool.injector, &deque))
                {
                    unmark_idle(&pool.idle, worker_id);
                    execute_task(task, worker_id, &pool);
                    break;
                }
                if pool.shutdown.load(Ordering::Acquire) {
                    unmark_idle(&pool.idle, worker_id);
                    return;
                }
                std::thread::park();
                unmark_idle(&pool.idle, worker_id);
                break;
            }
            backoff.snooze();
        }
    }
}

/// SplitMix64 — Sebastiano Vigna 2014. Fast scalar mixer, statistically
/// strong as a one-shot seeder for sibling PRNGs.
#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut x = z;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[inline]
fn try_pop_local_injector(
    inj: &Injector<TaskHandle>,
    local: &Worker<TaskHandle>,
) -> Option<TaskHandle> {
    match inj.steal_batch_and_pop(local) {
        crossbeam_deque::Steal::Success(t) => Some(t),
        _ => None,
    }
}

#[inline]
fn try_pop_global(
    inj: &Injector<TaskHandle>,
    local: &Worker<TaskHandle>,
) -> Option<TaskHandle> {
    match inj.steal_batch_and_pop(local) {
        crossbeam_deque::Steal::Success(t) => Some(t),
        _ => None,
    }
}

#[inline]
fn mark_idle(idle: &AtomicU64, worker_id: usize) {
    let bit = 1u64 << worker_id;
    idle.fetch_or(bit, Ordering::Release);
}

#[inline]
fn unmark_idle(idle: &AtomicU64, worker_id: usize) {
    let bit = 1u64 << worker_id;
    idle.fetch_and(!bit, Ordering::Release);
}

fn unpark_one_idle(pool: &ThreadPool) -> bool {
    loop {
        let mask = pool.idle.load(Ordering::Acquire);
        if mask == 0 {
            return false;
        }
        let bit = mask & mask.wrapping_neg();
        let new = mask & !bit;
        if pool.idle
            .compare_exchange_weak(mask, new, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let worker_id = bit.trailing_zeros() as usize;
            pool.workers[worker_id].thread.unpark();
            return true;
        }
    }
}
```

**Atomic orderings rationale:**
- `idle.fetch_or(Release)` — publishes "I am parked" so pushers reading Acquire see us.
- `idle.compare_exchange_weak(AcqRel, Acquire)` — RMW; success Release publishes the new mask; failure Acquire re-reads fresh values.
- `shutdown.load(Acquire)` — pairs with `shutdown.store(Release)` in `ThreadPool::drop`.

### 4.4 TLS subsystem (Round 2 — new)

```rust
// crates/boyko_threadpool/src/tls.rs

use std::cell::Cell;

thread_local! {
    /// Active pool pointer. Set by ThreadPool::install entry / cleared on
    /// exit. Worker threads also have this set (worker_main initializes it).
    pub(crate) static ACTIVE_POOL: Cell<*const super::pool::ThreadPool> =
        Cell::new(std::ptr::null());

    /// Nested par_iter detector (PAR6). Set true on par_iter entry; checked
    /// at par_iter::run_par_iter to forbid nesting.
    pub(crate) static IN_FORK_JOIN: Cell<bool> = Cell::new(false);

    /// Current worker id (TPN13).
    ///   - 0..MAX_WORKERS-1 — running on worker N.
    ///   - u32::MAX - 1 (sentinel DISPATCHER) — running on the dispatcher
    ///     thread inside Schedule::run.
    ///   - u32::MAX (sentinel UNATTACHED) — not on a worker, not in a
    ///     Schedule::run. Default state for the main app thread before
    ///     any install or schedule.
    pub(crate) static CURRENT_WORKER_ID: Cell<u32> = Cell::new(u32::MAX);

    /// Allocation discipline guard (ALLOC1). Set true by the worker's
    /// run-system RAII guard; arena.allocate_* debug_asserts !this.
    pub(crate) static IN_SYSTEM_RUN: Cell<bool> = Cell::new(false);
}

/// Sentinel for the dispatcher thread.
pub const WORKER_ID_DISPATCHER: u32 = u32::MAX - 1;
/// Sentinel for "no associated worker / dispatcher".
pub const WORKER_ID_UNATTACHED: u32 = u32::MAX;

#[inline]
pub fn current_worker_id() -> u32 {
    CURRENT_WORKER_ID.with(|c| c.get())
}

/// Returns the lane for EventDispatcher::send_event:
///   - worker id when on a worker.
///   - `worker_count` (= dispatcher's lane) when on dispatcher.
///   - 0 (default lane) when unattached.
/// `worker_count` MUST be passed because the TLS doesn't know it.
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

pub(crate) fn enter_fork_join_or_panic() {
    IN_FORK_JOIN.with(|c| {
        if c.get() {
            panic!("boyko-B9002: nested par_iter detected.");
        }
        c.set(true);
    });
}

pub(crate) fn exit_fork_join() {
    IN_FORK_JOIN.with(|c| c.set(false));
}

/// RAII guard around a worker's system-body execution. Sets IN_SYSTEM_RUN
/// on entry, clears on drop. ALLOC6 enforcement.
pub(crate) struct InSystemRunGuard;
impl InSystemRunGuard {
    pub fn enter() -> Self {
        IN_SYSTEM_RUN.with(|c| {
            debug_assert!(!c.get(), "IN_SYSTEM_RUN already set; nested system run?");
            c.set(true);
        });
        Self
    }
}
impl Drop for InSystemRunGuard {
    fn drop(&mut self) {
        IN_SYSTEM_RUN.with(|c| c.set(false));
    }
}
```

Exported for the ECS crate to query.

### 4.5 Scope API (Round 2 — revised)

`install` is the **entry point from non-pool context** (sets `ACTIVE_POOL` TLS, creates a scope). `scope` is the **re-entrant scope creation** for use from inside a worker; it does NOT touch `ACTIVE_POOL` (already set by the outer `install`). par_iter uses `scope`.

```rust
pub struct Scope<'scope> {
    pool: &'scope ThreadPool,
    shared: Box<ScopeShared>,
    _phantom: std::marker::PhantomData<&'scope ()>,
}

#[repr(C)]
pub(crate) struct ScopeShared {
    pending: CachePadded<AtomicUsize>,
    panic_payload: std::sync::Mutex<Option<Box<dyn std::any::Any + Send>>>,
    waker: Thread,
}

impl<'scope> Scope<'scope> {
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce(&Scope<'scope>) + Send + 'scope,
    {
        self.shared.pending.fetch_add(1, Ordering::AcqRel);

        let scope_ptr: *const Scope<'scope> = self as *const _;
        let shared_ptr: *const ScopeShared = &*self.shared as *const _;

        // SAFETY (TPN12, panic-safety §4.5.6):
        // The Scope::Drop blocks until pending == 0 (or steals work to help drain),
        // so no task body outlives 'scope; the 'static erasure is only valid
        // for the duration of the task body, which is bounded by the Drop.
        let body: Box<dyn FnOnce() + Send + 'static> = unsafe {
            std::mem::transmute(Box::new(move || {
                // Re-borrow scope.
                let scope: &Scope<'scope> = &*scope_ptr;
                let result = std::panic::catch_unwind(
                    std::panic::AssertUnwindSafe(|| f(scope))
                );
                if let Err(payload) = result {
                    let mut slot = (*shared_ptr).panic_payload.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(payload);
                    }
                }
                let prev = (*shared_ptr).pending.fetch_sub(1, Ordering::AcqRel);
                if prev == 1 {
                    (*shared_ptr).waker.unpark();
                }
            }) as Box<dyn FnOnce() + Send + '_>)
        };

        let task = TaskHandle { body, parent_scope: &*self.shared as *const _ };
        push_task(self.pool, task);
    }
}

impl<'scope> Drop for Scope<'scope> {
    fn drop(&mut self) {
        // Round 2 C3 / C6 fix: while waiting for pending == 0, STEAL WORK
        // instead of parking. The calling thread (whether dispatcher or
        // worker) is itself a useful execution unit; parking it can deadlock
        // when N concurrent par_iter invocations leave no worker for inner
        // tasks. The work-stealing wait pattern is the rayon pattern.
        join_workers_until_drained(self);

        if let Some(payload) = self.shared.panic_payload.lock().unwrap().take() {
            std::panic::resume_unwind(payload);
        }
    }
}

impl ThreadPool {
    /// Entry from non-pool context. Sets ACTIVE_POOL TLS, runs f.
    pub fn install<R, F>(&self, f: F) -> R
    where F: FnOnce(&Scope<'_>) -> R + Send
    {
        self.active_scopes.fetch_add(1, Ordering::AcqRel);

        // Round 2 TPN13: set TLS markers for the calling thread (= dispatcher).
        let prev_pool = crate::tls::ACTIVE_POOL.with(|c| {
            let p = c.get();
            c.set(self as *const _);
            p
        });
        let prev_worker_id = crate::tls::CURRENT_WORKER_ID.with(|c| {
            let id = c.get();
            c.set(crate::tls::WORKER_ID_DISPATCHER);
            id
        });

        let shared = Box::new(ScopeShared {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: std::sync::Mutex::new(None),
            waker: std::thread::current(),
        });
        let scope = Scope {
            pool: self,
            shared,
            _phantom: std::marker::PhantomData,
        };

        let result = f(&scope);
        drop(scope); // blocks; steals work while waiting (§4.5.5)

        // Restore TLS.
        crate::tls::ACTIVE_POOL.with(|c| c.set(prev_pool));
        crate::tls::CURRENT_WORKER_ID.with(|c| c.set(prev_worker_id));

        self.active_scopes.fetch_sub(1, Ordering::AcqRel);
        result
    }

    /// Re-entrant scope creation for use from a worker (Round 2 W6).
    /// Does NOT modify ACTIVE_POOL TLS (already set by the outer install
    /// that owns this thread).
    pub fn scope<'s, R, F>(&'s self, f: F) -> R
    where F: FnOnce(&Scope<'s>) -> R + Send
    {
        debug_assert!(
            !crate::tls::ACTIVE_POOL.with(|c| c.get().is_null()),
            "ThreadPool::scope called without an active pool TLS — use install instead"
        );

        let shared = Box::new(ScopeShared {
            pending: CachePadded::new(AtomicUsize::new(0)),
            panic_payload: std::sync::Mutex::new(None),
            waker: std::thread::current(),
        });
        let scope = Scope {
            pool: self,
            shared,
            _phantom: std::marker::PhantomData,
        };

        let result = f(&scope);
        drop(scope);

        if let Some(payload) = scope.shared.panic_payload.lock().unwrap().take() {
            // Defensive — Drop should have done this already.
            std::panic::resume_unwind(payload);
        }
        // (compiler complains because scope was moved into drop; pseudo-code only)
        result
    }
}

/// Push helper. If on a worker, push to that worker's local injector
/// (cache locality / Round 2 C2). Otherwise push to global injector
/// and wake one idle worker.
fn push_task(pool: &ThreadPool, task: TaskHandle) {
    let worker_id = crate::tls::current_worker_id();
    if (worker_id as usize) < pool.num_workers {
        // We're on worker `worker_id`. Push to its local injector.
        pool.injector_local[worker_id as usize].push(task);
        // Wake a sibling — we are busy with whatever spawned this.
        unpark_one_idle(pool);
    } else {
        // Dispatcher or external. Use global injector.
        pool.injector.push(task);
        unpark_one_idle(pool);
    }
}
```

#### 4.5.4 Single wake-up path (Round 2 W1 resolution)

The Round 1 plan called both `(*shared_ptr).waker.unpark()` (from the task body's `fetch_sub` last-task branch) **and** an explicit `dispatcher_thread.unpark()` from the executor's `scope.spawn` body. Round 2: **only** the `ScopeShared::waker.unpark()` path remains. The explicit dispatcher unpark is removed; the waker IS the dispatcher (captured at `install` entry via `std::thread::current()`). This eliminates the dual-wake-up confusion and reduces one unnecessary `unpark` syscall per task.

#### 4.5.5 `Scope::Drop` work-stealing wait (Round 2 C3 / C6 — new, Round 3 W-NEW-2 — optimistic deque-drain code removed)

```rust
/// Wait for `scope.shared.pending == 0`. Instead of parking the calling
/// thread, actively steal work from the pool. This prevents the deadlock
/// where N workers all enter par_iter simultaneously and the inner tasks
/// cannot find a free worker (because every worker is itself parked in
/// its own scope.Drop).
///
/// Round 3 W-NEW-2: this function does NOT drain the calling worker's
/// own Chase-Lev deque, because that deque lives on the worker thread's
/// stack inside `worker_main` and is not accessible from arbitrary
/// call sites. Instead we drain (a) the calling worker's local injector
/// (Arc-shared, reachable), (b) the global injector, (c) sibling stealers.
/// This is sufficient: the deadlock-freedom argument (§4.6) requires only
/// that we find SOME stealable work to make progress; we don't need to
/// drain our own deque specifically. Inner tasks spawned via `scope.spawn`
/// from inside a worker land in `pool.injector_local[worker_id]` via
/// `push_task`, which IS reachable here.
fn join_workers_until_drained<'scope>(scope: &Scope<'scope>) {
    let pool = scope.pool;
    let worker_id = crate::tls::current_worker_id();
    let on_worker = (worker_id as usize) < pool.num_workers;

    let backoff = crossbeam_utils::Backoff::new();

    loop {
        if scope.shared.pending.load(Ordering::Acquire) == 0 {
            return;
        }

        // 1. If on a worker, drain our own local injector first
        //    (inner tasks land here via push_task).
        if on_worker {
            if let Some(task) = try_pop_local_injector_standalone(
                &pool.injector_local[worker_id as usize],
            ) {
                execute_task_synchronously(task);
                backoff.reset();
                continue;
            }
        }

        // 2. Try the global injector.
        if let Some(task) = pop_global_into_dummy(&pool.injector) {
            execute_task_synchronously(task);
            backoff.reset();
            continue;
        }

        // 3. Try stealing from any worker.
        if let Some(task) = try_steal_any(&pool.stealers) {
            execute_task_synchronously(task);
            backoff.reset();
            continue;
        }

        // 4. Nothing to steal. Brief backoff, then check pending again.
        //    We DO NOT park unconditionally — parking would deadlock the
        //    common case where pending depends on tasks that are themselves
        //    waiting for us to drain something. However, if we exhaust the
        //    backoff and pending > 0, we park with a short timeout and let
        //    the waker fire.
        if backoff.is_completed() {
            // Park with timeout so we periodically re-poll for stealable work.
            // The waker (ScopeShared::waker) is the thread we're currently on,
            // so this thread will be unparked when pending hits 0 by the last
            // worker. Until then, the timeout serves as a re-poll trigger.
            std::thread::park_timeout(std::time::Duration::from_micros(50));
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}
```

#### 4.5.6 Panic safety + abort caveat (Round 2 C6 — new)

**Unwinding case (the common one):**
1. Worker task body panics → `catch_unwind` captures payload → stored in `ScopeShared::panic_payload`.
2. Worker decrements `pending`.
3. Drop runs on the calling thread; sees `pending == 0` (or steals other tasks until it is).
4. Drop reads `panic_payload`, calls `resume_unwind`.
5. The panic propagates out of `install`/`scope` to the user's code.

**Calling-thread-panics case:**
1. The user's closure in `install(|scope| { ... user code panics ... })` panics.
2. Stack unwinds; `scope` falls out of scope; `Scope::Drop` runs during unwinding.
3. Drop blocks waiting for `pending == 0`, **stealing work** (Round 2 C3).
4. Eventually all spawned tasks complete; Drop returns.
5. Unwinding resumes; the original panic propagates to the user.

**Worker stuck in infinite loop case:**
- If a worker task body enters an infinite loop, `pending` never hits 0.
- Drop steals work indefinitely (or runs out of stealable work and parks-with-timeout).
- The calling thread effectively hangs — same as Rayon. **Documented limitation**; no engine-level mitigation.

**Abort / SIGKILL case:**
```text
SAFETY caveat:
  If the process aborts (std::process::abort, OOM, SIGKILL) while spawned
  tasks reference 'scope-borrowed memory, the workers may continue accessing
  freed stack frames after the abort signal — UB at the language level.
  However, the entire process is being terminated; the kernel reclaims the
  memory; no observable consequence beyond the process exit.

  No mitigation possible at the library level. Same edge case as Rayon.
```

### 4.6 Deadlock-freedom proof sketch (Round 2 — new, Round 3 W-NEW-2 — own-deque references removed)

**Claim:** under the Round 2 design (work-stealing Drop wait + local injectors + scope vs install split), no deadlock can occur for any well-formed program that satisfies PAR6 (no nested par_iter) and ALLOC1 (no allocation in run_unsafe).

**Proof (informal):**

Let the pool have N workers. Consider any moment in time. Each worker is in one of:
- (A) Executing a task body.
- (B) Polling deques (local, local-injector, global, steal).
- (C) Parked via `std::thread::park()`.
- (D) Inside `join_workers_until_drained` (active steal-and-execute).

The dispatcher is in one of:
- (D') Inside `join_workers_until_drained` from `Scope::Drop`.
- (E') Running its own logic (find_ready, apply).

**Liveness argument:**
Pending tasks reside in one of: a worker's local deque, a local injector, the global injector. Any thread in (D) or (B) will find them via stealing or via the injector paths. The only way for tasks to be invisible is if a worker is in (A) or (C). Workers in (A) are making progress (executing user code). Workers in (C) are parked — they wake when:
- A pusher calls `unpark_one_idle` (the pusher clears one bit and unparks the corresponding worker).
- The pusher's push happens-before the unpark (Release/Acquire pair).

The only way a pusher's unpark could **miss** a worker is if the worker hasn't set its idle bit yet — but Race C in §13.4.1 demonstrates the re-poll-after-mark_idle protocol catches the missing wake-up.

**The dispatcher itself** waits on `scope.Drop` via `join_workers_until_drained`, which (a) steals work via the global injector / sibling stealers, (b) re-polls pending, (c) parks with timeout. The timeout ensures the dispatcher periodically wakes to re-check.

**Nested scope correctness (Round 3 W-NEW-2 — proof only references injector paths):**
A worker calls `pool.scope(...)` inside its own task body. The inner scope spawns inner tasks via `scope.spawn`, which `push_task` routes to **the calling worker's local injector** (`pool.injector_local[worker_id]`, per §4.5 push_task body). Other workers can steal these inner tasks via the local-injector or sibling-stealing paths in their `worker_main`. The calling worker's `Scope::Drop` enters `join_workers_until_drained`, which can:
- Pop from the calling worker's own **local injector** (`pool.injector_local[worker_id]` — reachable via the shared `Arc`).
- Pop from the global injector (drains outer tasks).
- Steal from sibling workers (drains anything in their deques).

The calling worker's own **deque** stays exclusive to `worker_main`'s loop and is not drained from `Scope::Drop`; the proof does not require draining it because inner tasks land in the local **injector**, not the local **deque**.

So the calling worker continues to do useful work while waiting. There is no scenario where all workers are blocked on each other's `Scope::Drop` because every `Scope::Drop` is itself doing work-stealing.

**Termination:** Each call to `scope`/`install` enqueues a finite number of tasks (assuming user code doesn't infinitely recurse via `par_iter`, which PAR6 forbids). Each task body returns in finite time. Therefore `pending` strictly decreases over time (modulo the initial `fetch_add`s), and `Scope::Drop` terminates.

Q.E.D. (sketch)

### 4.7 Public API

```rust
pub use pool::{ThreadPool, ThreadPoolBuilder, MAX_WORKERS};
pub use scope::Scope;
pub use stats::PoolStats;
pub use tls::{current_worker_id, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED};

impl ThreadPoolBuilder {
    pub fn new() -> Self;
    pub fn num_threads(self, n: usize) -> Self;
    pub fn thread_name_prefix(self, s: impl Into<String>) -> Self;
    pub fn pin_workers(self, on: bool) -> Self;
    pub fn stack_size(self, bytes: usize) -> Self;
    pub fn build(self) -> ThreadPool;
}

impl ThreadPool {
    pub fn install<R, F>(&self, f: F) -> R
    where F: FnOnce(&Scope<'_>) -> R + Send;

    pub fn scope<'s, R, F>(&'s self, f: F) -> R
    where F: FnOnce(&Scope<'s>) -> R + Send;

    pub fn spawn<F>(&self, f: F)
    where F: FnOnce() + Send + 'static;

    pub fn num_threads(&self) -> usize;
    pub fn stats(&self) -> PoolStats;
}

impl<'scope> Scope<'scope> {
    pub fn spawn<F>(&self, f: F)
    where F: FnOnce(&Scope<'scope>) + Send + 'scope;
}
```

---

## §5 — Schedule + ConflictGraph + Executor — deep dive

### 5.1 File layout

`crates/boyko_ecs/src/ecs/core/schedule/`:
- `mod.rs` — public re-exports.
- `schedule.rs` — `Schedule` struct + `Schedule::run`.
- `schedule_builder.rs` — `ScheduleBuilder` + Tarjan SCC + topo sort.
- `system_set.rs` — `SystemSet` trait + macro.
- `conflict_graph.rs` — `ConflictGraph` + bitset + SIMD scan.
- `executor.rs` — `Executor` (per-frame dispatch loop).
- `system_box.rs` — `SystemBox` heterogeneous wrapper.
- `apply_deferred.rs` — `ApplyDeferred` exclusive system + analyzer.
- `system_descriptor.rs` — per-system metadata at `add_system`.
- `ordering.rs` — `Order` enum.
- `exclusive.rs` — `ExclusiveFunctionSystem<F>` wrapper.
- `executor_scratch.rs` — per-frame scratch state.

### 5.2 SystemBox (Round 2 — revised)

```rust
#[repr(C)]
pub(crate) struct SystemBox {
    pub(crate) system: Box<dyn System<Out = ()>>,
    pub(crate) index: SystemIndex,

    /// Build-time cache, computed from `system.access().is_universal()`.
    /// Round 2 C9 / OQ-4 — single source of truth lives in Access.
    pub(crate) is_exclusive: bool,

    /// True if the system has any param that uses `CommandQueue` as state.
    /// Cached at build; used by §8 analyzer.
    pub(crate) has_deferred: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct SystemIndex(pub u16);

impl SystemBox {
    pub(crate) fn new<S>(system: S) -> Self
    where S: System<Out = ()> + 'static
    {
        let access = system.access();
        let is_exclusive = access.is_universal();
        let has_deferred = inspect_deferred(&system);  // helper; see §8

        Self {
            system: Box::new(system),
            index: SystemIndex(u16::MAX),  // set at build()
            is_exclusive,
            has_deferred,
        }
    }
}
```

### 5.3 ConflictGraph (Round 2 — pred counter added)

```rust
pub(crate) struct ConflictGraph {
    /// Per-system conflict bitset. conflicts[i] ∋ j iff i and j cannot run concurrently.
    pub(crate) conflicts: Box<[FixedBitSet]>,

    /// Per-system topological depth.
    pub(crate) depth: Box<[u16]>,

    /// Per-system predecessor list.
    pub(crate) predecessors: Box<[Box<[SystemIndex]>]>,

    /// Per-system successor list. Used by the incremental ready-set
    /// (Round 2 §7.4) to find systems that become ready when a pred completes.
    pub(crate) successors: Box<[Box<[SystemIndex]>]>,

    /// Total predecessors per system (cached for pred_remaining init).
    pub(crate) pred_count: Box<[u16]>,

    pub(crate) n: usize,
}

impl ConflictGraph {
    pub(crate) fn build(
        systems: &[SystemBox],
        order_edges: &[(SystemIndex, SystemIndex)],
    ) -> Self {
        let n = systems.len();
        let mut conflicts = (0..n).map(|_| FixedBitSet::with_capacity(n))
            .collect::<Vec<_>>().into_boxed_slice();
        let mut predecessors: Vec<Vec<SystemIndex>> = vec![Vec::new(); n];
        let mut successors: Vec<Vec<SystemIndex>> = vec![Vec::new(); n];

        // 1. Pairwise access conflict.
        for i in 0..n {
            for j in 0..i {
                let a = systems[i].system.access();
                let b = systems[j].system.access();
                if a.conflicts_with(b) {
                    conflicts[i].insert(j);
                    conflicts[j].insert(i);
                }
            }
        }

        // 2. Edge ingestion.
        for &(from, to) in order_edges {
            predecessors[to.0 as usize].push(from);
            successors[from.0 as usize].push(to);
            // Ordering implies a conflict — neither side may run alongside the other
            // (the downstream must wait for the upstream).
            conflicts[to.0 as usize].insert(from.0 as usize);
            conflicts[from.0 as usize].insert(to.0 as usize);
        }

        // 3. Topo depth (BFS fixpoint).
        let depth = compute_depths(&predecessors, n);

        let pred_count: Box<[u16]> = predecessors.iter()
            .map(|v| v.len() as u16).collect::<Vec<_>>().into_boxed_slice();

        Self {
            conflicts,
            depth,
            predecessors: into_boxed(predecessors),
            successors: into_boxed(successors),
            pred_count,
            n,
        }
    }
}

fn into_boxed(v: Vec<Vec<SystemIndex>>) -> Box<[Box<[SystemIndex]>]> {
    v.into_iter().map(Vec::into_boxed_slice).collect::<Vec<_>>().into_boxed_slice()
}
```

Note: explicit `into_boxed` helper avoids the Round 1 plan's iterator chain that was harder to read.

### 5.4 Schedule + Executor (Round 2 — apply-window rewrite; Round 3 W-NEW-5 — exclusive path SAFETY fixed)

```rust
pub struct Schedule {
    /// All systems in topological order.
    systems: Box<[SystemBox]>,
    /// Cross-system conflict + DAG.
    conflict_graph: ConflictGraph,
    /// Pool reference.
    pool: Arc<ThreadPool>,
    /// Frame counter.
    frame: u64,
    /// Per-frame scratch state.
    scratch: ExecutorScratch,
}

#[repr(C)]
pub(crate) struct ExecutorScratch {
    /// Bit set: systems currently dispatched to a worker.
    running: FixedBitSet,
    /// Bit set: systems completed in this frame.
    completed: FixedBitSet,

    /// Per-system pred_remaining counter. Initialised to pred_count at
    /// frame start. Dispatcher decrements each successor in apply window
    /// (no worker touches it; see Round 3 O-NEW-2). When pred_remaining[i]
    /// == 0, system i enters ready_queue.
    /// Round 2 §7.4 — replaces the O(N²) scan.
    /// Round 3 O-NEW-2 — type downgraded Box<[AtomicU16]> → Box<[u16]>
    /// because the dispatcher is the SOLE mutator (workers push to
    /// completion_queue; dispatcher pops and decrements). Verified by
    /// allocation-site audit: pred_remaining is mutated only inside
    /// `apply_window_drain` and `try_dispatch_ready` (exclusive path),
    /// both dispatcher-side. Saves LOCK prefix on x86; clarifies "this
    /// is single-thread state".
    pred_remaining: Box<[u16]>,

    /// Ready queue: systems whose preds are done; dispatched once their
    /// conflict bits don't intersect `running`. Single-producer (dispatcher)
    /// after Round 2 redesign — no atomics needed for the queue itself.
    ready_queue: VecDeque<SystemIndex>,

    /// Completion notification: workers push completed SystemIndex;
    /// dispatcher drains in apply window. Bounded by N — preallocated.
    completion_queue: Arc<crossbeam_queue::ArrayQueue<SystemIndex>>,

    /// Outstanding apply count: increments when a worker pushes completion;
    /// dispatcher drains this many in the apply window. Used to gate the
    /// apply-window entry — we enter apply window when running == 0 OR
    /// (running.is_empty() && pending_apply > 0). Saves one bitset count.
    pending_apply: CachePadded<AtomicUsize>,
}

impl Schedule {
    pub fn run(&mut self, world: &mut EcsMaster) {
        self.frame = self.frame.wrapping_add(1);
        self.scratch.reset_for_frame(&self.conflict_graph);

        // Validate the build-time cache (Round 2 C9 / OQ-4).
        debug_assert!(
            self.systems.iter().all(|sb|
                sb.is_exclusive == sb.system.access().is_universal()),
            "SystemBox::is_exclusive desynced from access().is_universal()"
        );

        // Seed ready_queue with all systems whose pred_count == 0.
        for i in 0..self.systems.len() {
            if self.conflict_graph.pred_count[i] == 0 {
                self.scratch.ready_queue.push_back(SystemIndex(i as u16));
            }
        }

        // Round 2 Q3 / O3: no Arc clone per frame; use &*self.pool directly.
        let pool_ref: &ThreadPool = &*self.pool;
        pool_ref.install(|scope| {
            self.executor_main_loop(world, scope);
        });

        // Frame-end implicit ApplyDeferred — runs after all systems done.
        self.frame_end_apply(world);
    }

    fn executor_main_loop(&mut self, world: &mut EcsMaster, scope: &Scope<'_>) {
        // Round 3 W-NEW-1: this is the SOLE canonical executor loop.
        // The earlier (Round 2 §5.4.5) diagram with an inline "Wait — bug"
        // correction has been removed; refer to §5.4.5.1 below for the
        // happens-before diagram. Loop rhythm:
        //
        //   [apply window drain (if gate fires)]
        //     → [if completed return]
        //     → [mint cell]
        //     → [try dispatch ready]
        //     → [if dispatched=0 && running>0: park_timeout]
        //
        // The cell is minted ONCE per outer iteration (after the apply
        // window) and remains valid for all dispatches in that iteration.
        // The next iteration's apply window reborrows &mut world, invalidating
        // the previous cell logically; the freshly-minted cell at the top
        // of the next iteration restores write access for the new round.
        let n = self.systems.len();

        while self.scratch.completed.count_ones(..) < n {
            // === APPLY WINDOW (Round 2 SCH7 / Round 3 W-NEW-4 monotonicity) ===
            //
            // Gate (see §5.4.5.1):
            //   - `pending_apply == running.count_ones() && pending_apply > 0`
            //     → every dispatched system has pushed its completion;
            //       drain safely.
            //   - `pending_apply > 0 && running == 0`
            //     → bootstrap / post-exclusive; drain.
            //
            // Monotonicity note (Round 3 W-NEW-4): the gate is monotone in
            // one round — once `pending == running` holds, it stays true
            // until apply_window_drain consumes the pending entries. Both
            // counters increase strictly monotonically within a round
            // (workers only fetch_add pending; dispatcher only sets running
            // during dispatch which has already happened by gate evaluation).
            // Staleness from the non-atomic combined check `pending ==
            // running.count_ones()` is bounded to one loop iteration —
            // the next iteration sees the stable state. No data race
            // exists: pending is atomic; running is dispatcher-owned and
            // not touched by workers (per §5.4.3).
            let pending = self.scratch.pending_apply.load(Ordering::Acquire);
            let running_count = self.scratch.running.count_ones(..) as usize;
            if pending > 0 && (pending == running_count || running_count == 0) {
                self.apply_window_drain(world);
            }

            // If everything completed in the apply window, exit.
            if self.scratch.completed.count_ones(..) == n {
                break;
            }

            // === DISPATCH ROUND ===
            //
            // Mint a FRESH UnsafeEcsCell for this round (Round 2 O3).
            // The cell lives for this dispatch round only; on the next
            // apply window, the dispatcher's &mut world borrow resumes
            // exclusively. A new round mints a fresh cell.
            //
            // SAFETY (U_C1, SEND1, SEND3): cell is rooted in the current
            //   &mut world; no other &-borrow exists; workers receive cell
            //   copies (Copy + Send/Sync); apply window will run only after
            //   every cell copy is gone (workers drained).
            let world_cell = unsafe {
                UnsafeEcsCell::new_mutable(world)
            };

            // Find ready systems whose conflict bits don't intersect
            // running. We scan ready_queue (which is small in practice —
            // typically |completions| × |avg succ|).
            let dispatched_count = self.try_dispatch_ready(scope, world_cell);

            if dispatched_count == 0 {
                // Nothing dispatchable right now. Either we're blocked
                // on conflicts (waiting for running to drain) or all systems
                // are running. Wait for at least one completion via the
                // ScopeShared::waker.
                if self.scratch.running.count_ones(..) > 0 {
                    // Park briefly. The worker that decrements pending to 0
                    // will unpark us; the timeout is a backstop.
                    std::thread::park_timeout(std::time::Duration::from_micros(100));
                }
            }
            // Loop: top of while re-evaluates the apply window gate.
        }
    }

    fn try_dispatch_ready(
        &mut self,
        scope: &Scope<'_>,
        world_cell: UnsafeEcsCell<'_>,
    ) -> usize {
        let mut dispatched = 0;
        let mut deferred = VecDeque::new();

        // Process ready_queue. For each system: if no conflict with running,
        // dispatch. Otherwise re-queue for next round.
        while let Some(sys_idx) = self.scratch.ready_queue.pop_front() {
            let i = sys_idx.0 as usize;
            if bitset_intersects(
                &self.conflict_graph.conflicts[i],
                &self.scratch.running,
            ) {
                deferred.push_back(sys_idx);
                continue;
            }

            if self.systems[i].is_exclusive {
                // Exclusive system requires running == 0. If anything is
                // running, defer.
                //
                // Note: this check is correct for the within-call sequence
                // (we may have just dispatched a normal system on a previous
                // ready_queue entry and then encountered an exclusive system).
                if self.scratch.running.count_ones(..) > 0 {
                    deferred.push_back(sys_idx);
                    continue;
                }

                // Round 3 W-NEW-5 / EXC1 SAFETY block:
                //
                // SAFETY (EXC1):
                //   - Universal Access means no other system runs concurrently:
                //     ready_queue gate proved `running == 0` above.
                //   - `cell.world_mut()` reborrows `&mut EcsMaster` from the
                //     world_cell minted in this dispatch round from the
                //     dispatcher's own &mut world. No other reference exists.
                //   - The exclusive system body must not retain any cell-
                //     derived borrow past return; we immediately reborrow
                //     via the same cell for apply, so any stashed pointer
                //     would alias the apply &mut borrow (UB).
                //   - The IN_SYSTEM_RUN guard is NOT set here because the
                //     exclusive body runs on the dispatcher, not a worker,
                //     and exclusive systems are allowed to allocate (they
                //     hold the world exclusively).
                let world_mut: &mut EcsMaster = unsafe { world_cell.world_mut() };
                self.systems[i].system.run_unsafe(world_cell);
                // Reborrow for apply — same cell, same root &mut world.
                let world_mut_for_apply: &mut EcsMaster =
                    unsafe { world_cell.world_mut() };
                self.systems[i].system.apply(world_mut_for_apply);
                self.scratch.completed.set(i, true);

                for &succ in self.conflict_graph.successors[i].iter() {
                    let s = succ.0 as usize;
                    // Round 3 O-NEW-2: pred_remaining is Box<[u16]>, dispatcher-owned.
                    debug_assert!(self.scratch.pred_remaining[s] > 0,
                                  "pred_remaining underflow");
                    self.scratch.pred_remaining[s] -= 1;
                    if self.scratch.pred_remaining[s] == 0 {
                        self.scratch.ready_queue.push_back(succ);
                    }
                }
                dispatched += 1;
                continue;
            }

            // Concurrent dispatch.
            self.scratch.running.set(i, true);
            let completion_queue = Arc::clone(&self.scratch.completion_queue);
            let pending_apply_ptr: *const AtomicUsize = &self.scratch.pending_apply.0 as *const _;
            let system_ptr: *mut dyn System<Out = ()> =
                &mut *self.systems[i].system as *mut _;
            let scope_running_bit_to_clear = i;  // captured by value
            let world_cell_copy = world_cell;

            // SAFETY (SP2, S1, SEND1/3, SCH7):
            //   - cell is Copy, captured by value into the closure.
            //   - system_ptr borrowed exclusively for the task lifetime
            //     (running[i] set; no other dispatch path picks i).
            //   - conflict bits ensure no concurrent system aliases this
            //     one's accesses.
            //   - The IN_SYSTEM_RUN guard prevents accidental arena allocation
            //     inside the system body (ALLOC1).
            scope.spawn(move |_| {
                let _alloc_guard = crate::tls::InSystemRunGuard::enter();
                unsafe {
                    (*system_ptr).run_unsafe(world_cell_copy);
                }
                // Drop guard before pushing completion (avoids race where
                // dispatcher could read pending_apply > 0, conclude apply
                // window is okay, but a worker is still inside the system).
                drop(_alloc_guard);

                // Push completion; bounded queue with cap = N.
                completion_queue.push(SystemIndex(scope_running_bit_to_clear as u16))
                    .expect("invariant SCH-CAP: completion_queue capacity >= N");

                // Increment pending_apply (Release pairs with dispatcher Acquire).
                unsafe { (&*pending_apply_ptr).fetch_add(1, Ordering::Release); }

                // No `running` bit clear; dispatcher does it in apply_window_drain
                // (see §5.4.3).
            });

            dispatched += 1;
        }

        // Put deferred back in ready_queue front.
        while let Some(idx) = deferred.pop_back() {
            self.scratch.ready_queue.push_front(idx);
        }

        dispatched
    }
}
```

#### 5.4.3 The `running` bitset is dispatcher-owned (Round 2 correction)

The above pseudocode does NOT have workers clear `running` bits — `FixedBitSet` is not atomic. Instead, the **dispatcher** clears the bit when it pops the system's completion from `completion_queue` (in `apply_window_drain`). Workers only push to `completion_queue` (which IS thread-safe — `ArrayQueue` is lock-free MPSC).

Worker closure (final form per §5.4 above):
```rust
scope.spawn(move |_| {
    let _alloc_guard = crate::tls::InSystemRunGuard::enter();
    unsafe { (*system_ptr).run_unsafe(world_cell_copy); }
    drop(_alloc_guard);
    completion_queue.push(SystemIndex(i as u16))
        .expect("invariant SCH-CAP");
    unsafe { (&*pending_apply_ptr).fetch_add(1, Ordering::Release); }
    // No bit-clear; dispatcher does it.
});
```

Dispatcher in `apply_window_drain` (see §5.4.5.1):
```rust
while drained < target {
    let idx = self.scratch.completion_queue.pop().expect(...);
    let i = idx.0 as usize;
    self.scratch.running.set(i, false);   // DISPATCHER clears the bit
    self.systems[i].system.apply(world);
    self.scratch.completed.set(i, true);
    // ... successors update ...
}
```

This means `running` is updated only on the dispatcher; the apply-window gate `pending_apply == running.count_ones()` is consistent because workers cannot read or modify `running` directly. They only push to `completion_queue`.

#### 5.4.4 Apply-window gate intuition

The gate `pending_apply == running.count_ones() && pending_apply > 0` is satisfied when every dispatched system has reported completion (pending_apply incremented) but the dispatcher has not yet drained the queue and decremented running. Inside the apply window, running is reset to zero one bit at a time as we pop the completion queue; this leaves the bitset and the counter in sync at function return (target completions processed; pending_apply -= target; running unchanged in terms of count remaining = previous_count - target = 0 if we drained all).

The bootstrap case (`pending > 0 && running == 0`) handles the post-exclusive-system path: when an exclusive system runs on the dispatcher in `try_dispatch_ready`, it completes synchronously and the dispatcher's pre-conditions for the next apply window may need processing.

#### 5.4.5.1 Happens-before diagram (Round 3 W-NEW-1 — sole canonical version)

```
Dispatcher                  Worker A                      Worker B

(top of executor_main_loop iteration)
  load pending_apply (Acquire) = 0
  running.count_ones() = 0
  → gate false; skip apply_window_drain
  → mint cell from &mut world
  → try_dispatch_ready:
      set running[A] = 1
      scope.spawn(A)  ─────► pop A; execute
                              run_unsafe(cell_A)
      set running[B] = 1
      scope.spawn(B)  ──────────────────────► pop B; execute
                                                run_unsafe(cell_B)
      dispatched = 2; return
  → dispatched != 0; no park
(top of next iteration)
  load pending_apply (Acquire) = 0 (or 1 if A done)
  running.count_ones() = 2
  → gate (0 == 2) false; skip
  → mint cell again (same data; harmless)
  → try_dispatch_ready (ready_queue empty or all conflict)
  → dispatched = 0 && running > 0 → park_timeout
                              … A finishes …
                              push completion[A]
                              fetch_add(pending_apply, Release) → 1
                              waker.unpark(dispatcher)
  dispatcher wakes
(top of next iteration)
  load pending_apply (Acquire) = 1
  running.count_ones() = 2
  → gate (1 == 2) false; skip   ← race-with-bounded-staleness
  → try_dispatch_ready (still nothing)
  → park_timeout
                                                … B finishes …
                                                push completion[B]
                                                fetch_add(pending_apply, Release) → 2
                                                waker.unpark
  wake
(top of next iteration)
  load pending_apply (Acquire) = 2   ← synchronizes-with both Releases
  running.count_ones() = 2
  → gate (2 == 2 && > 0) TRUE
  → apply_window_drain:
      target = 2
      pop A; running.set(A, false); apply(A, &mut world)  ← sees A's writes
      pop B; running.set(B, false); apply(B, &mut world)  ← sees B's writes
      pending_apply.fetch_sub(2, Relaxed)
      decrement pred_remaining for successors
  → (continue loop or exit if all completed)
```

The Acquire load on `pending_apply` synchronizes-with each worker's Release `fetch_add`, guaranteeing the worker's writes to component bytes are visible to the dispatcher before `apply(world)` reads them.

Monotonicity note (Round 3 W-NEW-4): the gate `pending == running` is monotone in one outer-loop iteration — once true, it remains true until `apply_window_drain` consumes pending. Workers only `fetch_add(pending, Release)`; the dispatcher only modifies `running` during dispatch which has already concluded before gate evaluation. Staleness from the non-atomic combined check is bounded to one outer iteration — the next iteration sees the stable state. No data race because `pending` is atomic and `running` is dispatcher-owned.

#### 5.4.5.2 apply_window_drain pseudocode (Round 2 — sole canonical drain function)

```rust
/// Drain all pending completions. PRECONDITION: gate fired at top of caller
/// loop — `pending_apply == running.count_ones() && pending_apply > 0`
/// OR `pending_apply > 0 && running == 0`.
fn apply_window_drain(&mut self, world: &mut EcsMaster) {
    let target = self.scratch.pending_apply.load(Ordering::Acquire);
    let mut drained = 0;
    while drained < target {
        let idx = self.scratch.completion_queue.pop()
            .expect("invariant: pending_apply counted; completion must be present");
        let i = idx.0 as usize;
        self.scratch.running.set(i, false);
        self.systems[i].system.apply(world);
        self.scratch.completed.set(i, true);
        for &succ in self.conflict_graph.successors[i].iter() {
            let s = succ.0 as usize;
            // Round 3 O-NEW-2: plain u16, dispatcher-owned.
            debug_assert!(self.scratch.pred_remaining[s] > 0,
                          "pred_remaining underflow");
            self.scratch.pred_remaining[s] -= 1;
            if self.scratch.pred_remaining[s] == 0 {
                self.scratch.ready_queue.push_back(succ);
            }
        }
        drained += 1;
    }
    self.scratch.pending_apply.fetch_sub(target, Ordering::Relaxed);
}
```

### 5.5 ScheduleBuilder (Round 2 W4 — pool field added; Round 3 W-NEW-3 — build pre-destructures self)

```rust
pub struct ScheduleBuilder {
    pool: Arc<ThreadPool>,   // Round 2 W4 — pool stored at builder creation.
    descriptors: Vec<SystemDescriptor>,
    order_edges: Vec<(SystemKey, SystemKey)>,
    sets: HashMap<SystemSetId, Vec<SystemKey>>,
}

pub struct SystemDescriptor {
    pub(crate) system_box: SystemBox,
    pub(crate) sets: SmallVec<[SystemSetId; 2]>,
    pub(crate) before: SmallVec<[SystemKey; 2]>,
    pub(crate) after: SmallVec<[SystemKey; 2]>,
    pub(crate) ambiguous_with: SmallVec<[SystemKey; 2]>,
    pub(crate) no_sync: bool,
}

impl ScheduleBuilder {
    pub fn new(pool: Arc<ThreadPool>) -> Self {
        Self {
            pool,
            descriptors: Vec::new(),
            order_edges: Vec::new(),
            sets: HashMap::new(),
        }
    }

    pub fn add_system<F, M>(&mut self, system: F) -> SystemConfig<'_>
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static
    { /* ... */ }

    /// Round 3 W-NEW-3: pre-destructure self so we can move `descriptors`
    /// by value into `insert_sync_points` without later borrows colliding.
    pub fn build(self, world: &mut EcsMaster) -> Schedule {
        // ALLOC2: build is on dispatcher, no workers; allocation freely allowed.
        let Self { pool, mut descriptors, order_edges, sets } = self;

        // 1. For each system, call System::initialize.
        for d in &mut descriptors {
            d.system_box.system.initialize(world);
        }

        // 2. Expand sets to edges.
        let expanded_edges = expand_set_edges(&sets, &order_edges, descriptors.len());

        // 3. Tarjan SCC for cycle detection. Capture names BEFORE the move.
        let sccs = tarjan_scc(&expanded_edges, descriptors.len());
        for scc in &sccs {
            if scc.len() > 1 {
                let names: Vec<&str> = scc.iter()
                    .map(|k| descriptors[k.0 as usize].system_box.system.name())
                    .collect();
                panic!("boyko-B9001: schedule contains a cycle: {:?}", names);
            }
        }

        // 4. Topo sort.
        let topo_order = kahn_topo_sort(&expanded_edges, descriptors.len());

        // 5. Assign SystemIndex per topo order.
        // (renumber descriptors in place per topo_order; details omitted)

        // 6. Insert sync points (§8). This moves descriptors and edges.
        let with_syncs = insert_sync_points(descriptors, expanded_edges);

        // Round 3 O-NEW-3: pred_remaining is u16; verify cap fits the type.
        // MAX_SYSTEMS_PER_SCHEDULE = 1024 ≪ u16::MAX (65535).
        debug_assert!(
            with_syncs.systems.len() <= u16::MAX as usize,
            "MAX_SYSTEMS_PER_SCHEDULE must fit u16 (pred_remaining type)"
        );

        // 7. Build ConflictGraph.
        let conflict_graph = ConflictGraph::build(&with_syncs.systems, &with_syncs.edges);

        // Round 3 O-NEW-3: every pred_count must fit u16 too (= same bound).
        debug_assert!(
            conflict_graph.pred_count.iter().all(|&c| c <= u16::MAX),
            "pred_count must fit u16 (pred_remaining type)"
        );

        let n = with_syncs.systems.len();

        // 8. Wrap with Schedule.
        Schedule {
            systems: with_syncs.systems.into_boxed_slice(),
            conflict_graph,
            pool,
            frame: 0,
            scratch: ExecutorScratch::new(n),
        }
    }
}
```

### 5.6 SystemConfig (fluent API)

```rust
pub struct SystemConfig<'b> {
    builder: &'b mut ScheduleBuilder,
    key: SystemKey,
}

impl<'b> SystemConfig<'b> {
    pub fn before<F, M>(self, other: F) -> Self
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;
    pub fn after<F, M>(self, other: F) -> Self;
    pub fn in_set<S: SystemSet>(self, set: S) -> Self;
    pub fn ambiguous_with<F, M>(self, other: F) -> Self;
    pub fn no_sync(self) -> Self;
}
```

---

## §6 — `Query::par_iter` — deep dive

### 6.1 ParQuery struct (unchanged from Round 1)

```rust
pub struct ParQuery<'q, 's, D: QueryData, F: QueryFilter> {
    state: &'s QueryDataState<D, F>,
    world: UnsafeEcsCell<'q>,
    chunk_size_override: Option<usize>,
}

pub struct ParQueryMut<'q, 's, D: QueryData, F: QueryFilter> {
    state: &'s QueryDataState<D, F>,
    world: UnsafeEcsCell<'q>,
    chunk_size_override: Option<usize>,
}

impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    pub fn par_iter<'q>(&'q self) -> ParQuery<'q, 's, D, F>
    where D: ReadOnlyQueryData;
    pub fn par_iter_mut<'q>(&'q mut self) -> ParQueryMut<'q, 's, D, F>;
}
```

`for_each` and `with_chunk_size` as in Round 1.

### 6.2 Chunk dispatch driver (Round 2 — uses `scope`, MIN_ARCHETYPE gate)

```rust
fn run_par_iter<D: QueryData, F: QueryFilter, Body>(
    state: &QueryDataState<D, F>,
    world: UnsafeEcsCell<'_>,
    chunk_size_override: Option<usize>,
    body: Body,
    mutable: bool,
) where Body: Fn(D::Item<'_>) + Send + Sync,
{
    let pool: &ThreadPool = current_pool()
        .expect("boyko-B9003: par_iter outside install/scope");
    let _fj_guard = ForkJoinGuard::enter();  // PAR6, panics on nested

    let worker_count = pool.num_threads();
    let chunk_size_soft = compute_chunk_size_soft(
        state, worker_count, chunk_size_override, world);

    // Round 2 W6: use `scope` (re-entrant), not `install`.
    // Round 2 C3: scope's Drop steals work while waiting → no deadlock
    //   from N concurrent par_iter invocations.
    pool.scope(|scope| {
        let body_ref = &body;
        for &arch_id in state.archetype_state.matched_ids() {
            let arch_ptr = if mutable {
                unsafe { world.archetype_ptr_mut(arch_id) }
            } else {
                unsafe { world.archetype_ptr(arch_id) }.map(|p| p as *mut _)
            };
            let Some(arch_ptr) = arch_ptr else { continue };

            let entity_count = unsafe { (*arch_ptr).entity_count() };
            if entity_count == 0 { continue; }

            // Round 2 O2 / PAR9: process small archetypes inline.
            const MIN_ARCHETYPE_FOR_PARALLEL: usize = 1024;
            if entity_count < MIN_ARCHETYPE_FOR_PARALLEL {
                run_chunk_inline::<D, F, Body>(state, arch_ptr, 0, entity_count,
                                                mutable, body_ref);
                continue;
            }

            let n_chunks = (entity_count + chunk_size_soft - 1) / chunk_size_soft;
            for chunk_idx in 0..n_chunks {
                let start = chunk_idx * chunk_size_soft;
                let end = ((chunk_idx + 1) * chunk_size_soft).min(entity_count);

                let world_for_task = world;
                let data_state_ref = &state.data_state;
                let filter_state_ref = &state.filter_state;
                // SAFETY (PAR3, SP2): each chunk's rows are disjoint;
                //   D::Item<'_> for the chunk's row range is unique;
                //   body is Fn + Send + Sync (PAR2); cell is Send/Sync (SEND3).
                scope.spawn(move |_| {
                    let mut data_fetch = <D as QueryData>::init_fetch(data_state_ref);
                    let mut filter_fetch = <F as QueryFilter>::init_fetch(filter_state_ref);

                    if mutable {
                        unsafe {
                            <D as QueryData>::set_table_mut(
                                &mut data_fetch, data_state_ref, arch_ptr);
                            <F as QueryFilter>::set_table_mut(
                                &mut filter_fetch, filter_state_ref, arch_ptr);
                        }
                    } else {
                        unsafe {
                            <D as QueryData>::set_table_readonly(
                                &mut data_fetch, data_state_ref, arch_ptr);
                            <F as QueryFilter>::set_table_readonly(
                                &mut filter_fetch, filter_state_ref, arch_ptr);
                        }
                    }

                    for row in start..end {
                        if !const { F::IS_ARCHETYPAL } {
                            let pass = unsafe {
                                <F as QueryFilter>::filter_fetch(&filter_fetch, row)
                            };
                            if !pass { continue; }
                        }
                        let item = unsafe { <D as QueryData>::fetch(&data_fetch, row) };
                        body_ref(item);
                    }
                });
            }
        }
    });
    // _fj_guard drops here, clearing IN_FORK_JOIN.
}

fn run_chunk_inline<D, F, Body>(
    state: &QueryDataState<D, F>,
    arch_ptr: *mut Archetype,
    start: usize,
    end: usize,
    mutable: bool,
    body: &Body,
) where D: QueryData, F: QueryFilter, Body: Fn(D::Item<'_>)
{
    // Same chunk loop as inside spawn, but on the calling thread.
    // Pseudocode omitted; mirrors the spawn body.
}

struct ForkJoinGuard;
impl ForkJoinGuard {
    fn enter() -> Self {
        crate::tls::enter_fork_join_or_panic();
        Self
    }
}
impl Drop for ForkJoinGuard {
    fn drop(&mut self) {
        crate::tls::exit_fork_join();
    }
}
```

### 6.3 TLS (subsumed by §4.4)

`current_pool()`, `enter_fork_join_or_panic`, `exit_fork_join`, `current_worker_id` all live in `boyko_threadpool::tls` (§4.4). The ECS crate imports them.

### 6.4 Chunk size policy

Unchanged from Round 1:
- `MIN_CHUNK_SIZE = 256`.
- `batches_per_thread = 1` (Bevy default).
- Per-archetype chunking.
- **NEW** in Round 2: `MIN_ARCHETYPE_FOR_PARALLEL = 1024` rows below which the archetype is processed inline on the calling thread.

---

## §7 — Conflict graph algorithm

### 7.1 Build phase

```text
Input:
  systems: [SystemBox; N]
  edges: [(SystemIndex, SystemIndex); M]  // user .before/.after

Output:
  conflicts: [FixedBitSet(N); N]
  depths: [u16; N]
  pred_count: [u16; N]
  predecessors, successors

Cost:
  - Pairwise access conflict: O(N²/2 × 30 ns) = 15 ms at N=1024 (one-shot).
  - Edge ingestion: O(M).
  - Depth BFS: O(N × max_depth). For N=1024 worst case ~1 ms.

Total: ~16 ms at N=1024 worst case. Cold; runs once.
```

Round 2 acknowledged limitation: 16 ms build at N=1024 is acceptable as a one-shot cost; hot-reload scenarios would need caching keyed by access fingerprints. Not Phase 9 scope (see §17 OQ-6).

### 7.2 Dispatch phase — SIMD `bitset_intersects`

```rust
#[inline]
fn bitset_intersects(a: &FixedBitSet, b: &FixedBitSet) -> bool {
    let a_slice = a.as_slice();
    let b_slice = b.as_slice();
    debug_assert_eq!(a_slice.len(), b_slice.len(),
                     "invariant: conflict bitsets are sized to N");

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return bitset_intersects_avx2(a_slice, b_slice); }

    a_slice.iter().zip(b_slice).any(|(a, b)| (a & b) != 0)
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline]
unsafe fn bitset_intersects_avx2(a: &[u32], b: &[u32]) -> bool {
    use std::arch::x86_64::*;
    let n = a.len();
    let chunks = n / 8;
    for i in 0..chunks {
        let ai = _mm256_loadu_si256(a.as_ptr().add(i * 8) as *const __m256i);
        let bi = _mm256_loadu_si256(b.as_ptr().add(i * 8) as *const __m256i);
        let and = _mm256_and_si256(ai, bi);
        if _mm256_testz_si256(and, and) == 0 { return true; }
    }
    for i in (chunks * 8)..n {
        if (a[i] & b[i]) != 0 { return true; }
    }
    false
}
```

### 7.3 Worst-case analysis

At N=1024 with the incremental ready set (§7.4):
- Per dispatch: scan only `ready_queue`. Typical size: handful (≤ 32 at any moment).
- Per scan: per-element `bitset_intersects` ~ 8 ns (AVX2) × 32 = ~250 ns.
- Per completion: update successors (plain `pred_remaining[s] -= 1`) ~ 2 ns × avg succ.
- Total dispatcher overhead per frame at N=1024 with dense conflicts: ~197 µs (see §10.5).

### 7.4 Incremental ready set (Round 2 C8 — new, Round 3 O-NEW-2 — type downgrade)

#### 7.4.1 Data layout

Each schedule has:
```rust
pub(crate) struct ExecutorScratch {
    /// Plain u16 per system (Round 3 O-NEW-2). Initialized to pred_count[i]
    /// at frame start. Decremented (non-atomic, dispatcher-owned) on each
    /// predecessor completion in apply_window_drain or in the exclusive
    /// system path of try_dispatch_ready. When pred_remaining[i] hits 0,
    /// system i is pushed to ready_queue.
    ///
    /// Round 3 O-NEW-2 rationale: workers do not touch pred_remaining
    /// (the audit shows mutations only in dispatcher-side code). Atomic
    /// type was over-engineering; plain u16 saves the LOCK prefix and
    /// makes "single-thread state" explicit at the type level.
    pred_remaining: Box<[u16]>,
    // ... other fields ...
}
```

Memory cost: 2 B × N + boxed-slice header. At N=1024: 2 KB + 16 B.

#### 7.4.2 Algorithm

**Frame start (`reset_for_frame`):**
```rust
fn reset_for_frame(&mut self, graph: &ConflictGraph) {
    self.running.clear();
    self.completed.clear();
    self.ready_queue.clear();
    self.pending_apply.0.store(0, Ordering::Relaxed);
    for i in 0..graph.n {
        // Plain assignment; dispatcher-owned.
        self.pred_remaining[i] = graph.pred_count[i];
    }
    // Seed ready_queue: every system with pred_count[i] == 0.
    for i in 0..graph.n {
        if graph.pred_count[i] == 0 {
            self.ready_queue.push_back(SystemIndex(i as u16));
        }
    }
}
```

**On completion (`apply_window_drain` body, also exclusive-system path):**
```rust
for &succ in self.conflict_graph.successors[i].iter() {
    let s = succ.0 as usize;
    debug_assert!(self.scratch.pred_remaining[s] > 0, "pred_remaining underflow");
    self.scratch.pred_remaining[s] -= 1;
    if self.scratch.pred_remaining[s] == 0 {
        self.scratch.ready_queue.push_back(succ);
    }
}
```

#### 7.4.3 Ordering rationale

`pred_remaining` is decremented and read **only on the dispatcher** (in `apply_window_drain` and in the exclusive-system branch of `try_dispatch_ready`). Workers never touch it — they only push to `completion_queue` and increment `pending_apply`. Therefore the field is `Box<[u16]>` (Round 3 O-NEW-2), not atomic.

Audit (verified): mutation sites are:
1. `reset_for_frame` (frame boundary, dispatcher).
2. `apply_window_drain` (dispatcher, inside apply window with workers drained).
3. `try_dispatch_ready` exclusive-system branch (dispatcher, exclusive system already gated by `running == 0`).

Worker code paths (`scope.spawn` body in `try_dispatch_ready` non-exclusive branch) push to `completion_queue` and `fetch_add(pending_apply)` only — no `pred_remaining` access.

If a future revision distributes apply to workers, upgrade to `Box<[AtomicU16]>` with `AcqRel` (Release on decrement to publish; Acquire on the dispatcher's "is it zero?" check).

#### 7.4.4 Cost savings

Round 1 design (per-frame O(N²) find_ready scan):
- N = 1024, 30 rounds/frame → 30 × (1024 × ~35 ns) = ~1.07 ms/frame.

Round 2 design (incremental):
- N = 1024, |ready_queue| typically ≤ 32 per round, 30 rounds.
- Per round: ready_queue scan = 32 × 8 ns (bitset_intersects) = 256 ns; pred_remaining updates = ~30 × 2 ns (plain decrement) = 60 ns.
- Per round total: ~320 ns.
- 30 rounds: ~9.6 µs.

Plus the apply step (`apply` call per system): 1024 × 50 ns = 50 µs.

Plus dispatch (`scope.spawn` per system): 1024 × 120 ns = 120 µs.

**Total dispatcher overhead at N=1024: ~9.6 + 50 + 120 ≈ 180 µs (originally 197 µs with atomic decrement). Under the 200 µs target.** ✓

---

## §8 — Apply phase coordination (sync points)

Unchanged from Round 1 conceptually; reconfirmed in light of Round 2 apply window.

### 8.1 Data flow

```
System A enqueues SpawnCommand → CommandQueue grows.
System B reads archetypes via Query → needs spawned entities visible?
  → YES (same frame): sync A's CommandQueue before B runs.
  → NO (next frame): no sync needed; frame-end sweep covers it.
```

### 8.2 Auto-insertion algorithm

```text
1. Identify DEFERRED ⊆ systems with has_deferred == true.
2. For each A in DEFERRED:
     For each B downstream of A in DAG with B.access.contains_structural_read():
       Insert an ApplyDeferred between A and B (replacing the direct A → B edge).
3. Coalesce: multiple A → ApplyDeferred edges merged where they share the upstream cone.
4. ApplyDeferred is exclusive (universal access).
5. Frame-end implicit ApplyDeferred always appended.
```

### 8.3 ApplyDeferred (Round 2 — slightly revised)

The Round 1 design used a marker that the executor recognized. Round 2 keeps the same shape but the executor's apply-window mechanism naturally accommodates it: an `ApplyDeferred` is just an exclusive system whose body flushes upstream queues.

```rust
pub(crate) struct ApplyDeferred {
    meta: SystemMeta,
    upstream: Vec<SystemIndex>,
}

impl ApplyDeferred {
    pub(crate) fn new(upstream: Vec<SystemIndex>) -> Self {
        let mut meta = SystemMeta::new("ApplyDeferred");
        // EXC1 / EXC2 (Round 2): universal access drives is_exclusive.
        meta.access = Access::universal();
        Self { meta, upstream }
    }
}

// SAFETY: universal access → conflicts with everything → runs alone.
//   run_unsafe constructs &mut EcsMaster via cell.world_mut() (cell is
//   write-capable; cell is the dispatcher's own cell minted from
//   &mut world).
unsafe impl System for ApplyDeferred {
    type Out = ();
    fn name(&self) -> &'static str { self.meta.name }
    fn access(&self) -> &Access { &self.meta.access }
    fn initialize(&mut self, _world: &mut EcsMaster) {}
    unsafe fn run_unsafe(&mut self, _cell: UnsafeEcsCell<'_>) {
        // No-op; the dispatcher detects ApplyDeferred and walks self.upstream
        // for each system, calling .apply(world) under the dispatcher's
        // exclusive borrow.
    }
    fn apply(&mut self, _world: &mut EcsMaster) {}
}
```

Executor side: when dispatching a system, check if it's `ApplyDeferred`. If so, instead of `run_unsafe(cell_copy)` + `apply(world)`, the executor directly walks the upstream Vec and calls `self.systems[upstream_idx].system.apply(world)` for each. This keeps the apply logic in the schedule (which has access to the systems Vec); the `ApplyDeferred` struct is just a marker.

### 8.4 IgnoreDeferred / .no_sync()

```rust
impl SystemConfig<'_> {
    pub fn no_sync(self) -> Self { ... }
}
```

Sets `SystemDescriptor::no_sync = true`; analyzer skips inserting `ApplyDeferred` on outgoing edges.

---

## §9 — Send/Sync contract

### 9.1 Type-by-type table (Round 2 — revised)

| Type | Send | Sync | Mechanism | Document where |
|------|------|------|-----------|----------------|
| `EcsMaster` | yes (SEND1) | yes (SEND1) | unsafe impl; ALLOC1..6 discipline | `ecs_master.rs` |
| **`Arena`** | **no** (Round 2 SEND2) | **no** | UnsafeCell-protected; touched only on dispatcher | `arena.rs` — UNCHANGED |
| `MemFreeBlockMaster` | no | no | unchanged | unchanged |
| `ComponentPool` | yes (SEND10) | yes (SEND10) | unsafe impl; reads disjoint between pools; grow only on dispatcher | `component_pool.rs` |
| `Chunk` | yes | yes | metadata only | unchanged |
| `Archetype` | yes (SEND10) | yes (SEND10) | composed of Send + Sync parts (post SEND10) | `archetype.rs` |
| `ArchetypeMaster` | yes (SEND6) | yes (SEND6) | unsafe impl | `archetype_master.rs` |
| `EntityMaster` | yes (SEND5) | yes (SEND5) | unsafe impl; pre-sized at construction | `entity_master.rs` |
| `Resources` | yes | yes | per-resource Box; disjoint by ResourceId | unchanged |
| `EventDispatcher` | yes (SEND4) | yes (SEND4) | per-thread lanes; send_event TLS lookup | `event_dispatcher.rs` |
| `UnsafeEcsCell<'w>` | yes (SEND3) | yes (SEND3) | unsafe impl; raw ptr + contract | `unsafe_ecs_cell.rs` |
| `Schedule` | no | no | mutated by run | `schedule.rs` |
| `ScheduleBuilder` | no | no | builder | `schedule_builder.rs` |
| `SystemBox` | yes | yes | wraps Box<dyn System<Out=()>> | `system_box.rs` |
| `Box<dyn System<Out=()>>` | yes (SEND8) | yes (SEND8) | trait bound | unchanged |
| `FunctionSystem<F, M>` | yes | yes | inherits | unchanged |
| `CommandQueue` | yes (CQ-SEND1) | no (CQ-SEND2) | per-system, single-writer | unchanged |
| `RawCommandQueue` | no | no | raw NonNull; transient | unchanged |
| `Commands<'s>` | yes | **no** (CQ-SEND2) | wraps !Sync queue | `commands.rs` |
| `Query<'w, 's, D, F>` | yes (when D, F: Send + Sync) | yes (similarly) | holds refs + cell | needs explicit impl |
| `QueryIter / QueryIterMut` | no | no | per-thread cursor | `iter.rs` |
| `ParQuery / ParQueryMut` | no | no | transient cursor | `par_iter.rs` |
| `ThreadPool` | yes | yes | composition of Arc + atomics | `pool.rs` |
| `Scope<'scope>` | no | no | per-scope; tasks `'scope + Send` | `scope.rs` |
| `ConflictGraph` | yes | yes | data only | `conflict_graph.rs` |

### 9.2 The aliasing-discipline contract (Round 2 — revised, Round 3 O-NEW-1 — release-mode note)

```rust
// SAFETY (SEND1) — Send + Sync for EcsMaster:
//
// EcsMaster is `Send + Sync` ONLY in the presence of the scheduler's
// aliasing-discipline contract AND the allocation-discipline contract
// (Phase 9 §2.7 ALLOC1..6).
//
// === Aliasing discipline (SP1/SP2/SP3 + SCH3 + SCH7) ===
//
//   1. Direct method calls on `&EcsMaster` / `&mut EcsMaster` are sequential
//      — Rust borrow checker enforces.
//
//   2. Multi-thread access happens exclusively through `UnsafeEcsCell` copies
//      handed out by the scheduler. The scheduler's `ConflictGraph` (SCH3)
//      ensures no two cells from different systems alias the same component
//      bytes or resource slot mutably at any single moment.
//
//   3. The apply window (SCH7, Phase 9 §5.4.5) is the ONLY context in which
//      `&mut EcsMaster` is constructed concurrently with cell copies; the
//      apply window is gated on `pending_apply == running.count_ones()`
//      which proves all worker tasks have completed (Release/Acquire pair).
//      During apply: no worker holds any cell copy that aliases the bytes
//      apply touches.
//
// === Allocation discipline (ALLOC1..6) ===
//
//   4. `Arena: !Send + !Sync` is preserved (unchanged from Phase 8). The
//      arena is touched only on the dispatcher:
//        - At `EcsMaster::new` / `ScheduleBuilder::build` (single-threaded).
//        - In apply window (no workers running).
//
//   5. `IN_SYSTEM_RUN: Cell<bool>` TLS guard set by the worker's RAII guard
//      `InSystemRunGuard` before `run_unsafe`. `arena.allocate_*`
//      debug_asserts !IN_SYSTEM_RUN.
//
//      Round 3 O-NEW-1 — Release-mode enforcement: the discipline is
//      enforced primarily through dev-mode `debug_assert!`. Release builds
//      skip the check; the audit table (§9.4) proves no reachable call
//      site allocates from a worker `run_unsafe`. For extra safety in CI,
//      Step 24 wires `RUSTFLAGS="--cfg force_alloc_panic"`, which turns
//      the debug_assert into a release-mode `panic!`; any future refactor
//      that accidentally allocates inside a system body is caught.
//
// === Forbidden operations from a worker thread ===
//
//   6. Direct mutation of archetype set (`create_archetype` / `clear`) —
//      structural; apply window only.
//   7. Direct mutation of entity set (`create_entity` / `delete_entity`) —
//      apply window only.
//   8. Direct insertion / removal of resources — apply window only.
//   9. Any `arena.allocate_*` — forbidden in run_unsafe; debug-asserted.
//
// === Permitted operations from a worker thread (under cell) ===
//
//   10. Reads of components per declared Access.
//   11. Writes of components per declared Access (within already-allocated
//       ComponentPool bytes; ComponentPool::grow is NOT called from a worker).
//   12. Reads/writes of Resources per declared Access (within already-
//       allocated Box).
//   13. CommandQueue::push (single-writer per system; CQ-SEND1/CQ-SEND2).
//   14. EventDispatcher::send_event (per-worker-id lane; EVT1).
//
unsafe impl Send for EcsMaster {}
unsafe impl Sync for EcsMaster {}
```

### 9.3 Detection mechanism for violations

Tests in `tests/send_sync_negative.rs` use `static_assertions = "1.1"`:

```rust
use static_assertions::{assert_impl_all, assert_not_impl_any};

// Positive — must compile.
assert_impl_all!(EcsMaster: Send, Sync);
assert_impl_all!(UnsafeEcsCell<'static>: Send, Sync);
assert_impl_all!(ThreadPool: Send, Sync);

// Negative — must fail to compile if these become Send/Sync accidentally.
assert_not_impl_any!(Arena: Send, Sync);
assert_not_impl_any!(Schedule: Send, Sync);
assert_not_impl_any!(ScheduleBuilder: Send, Sync);
assert_not_impl_any!(RawCommandQueue: Send, Sync);
assert_not_impl_any!(Commands<'static>: Sync);
```

### 9.4 Allocation site audit (Round 2 — new, ALLOC4 detail)

Exhaustive list of call sites that today take `&Arena` and call `allocate_layout` or `allocate_from_free_blocks`. Each must be reachable only from the dispatcher (under ALLOC2).

| Site (file:fn) | Caller | Reaches `Arena::allocate_*`? | Reachable from worker? | Routing under Phase 9 |
|----------------|--------|------------------------------|-------------------------|------------------------|
| `component_pool.rs:ComponentPool::new` | `Archetype::add_pool` | Yes — initial allocation | No — only called by `Archetype::with_components` which is called from `ArchetypeMaster::find_or_create_archetype` (dispatcher) | OK — dispatcher only |
| `component_pool.rs:ComponentPool::grow` (if exists) | `ComponentPool::push` when chunk full | Yes | YES — `push` is called from `Archetype::create_entity` which is called from `Commands::spawn` apply (dispatcher) AND from direct `EcsMaster::create_entity` (also dispatcher) | OK — both paths run in apply window or pre-schedule |
| `chunk.rs:Chunk::new` (if exists) | `ComponentPool::new` | Indirect | No | OK |
| `archetype_master.rs:find_or_create_archetype` | Called from `Commands::spawn` apply; also from `EcsMaster::create_archetype` direct | Yes — calls `Archetype::with_components` → `ComponentPool::new` → `arena.allocate_*` | YES via `Commands::spawn` apply (dispatcher in apply window) | OK |
| `events/event_buffer.rs:EventBuffer::new` | `EventDispatcher::preregister` | Yes — Box::new heap, NOT arena | No (called pre-schedule) | OK; not arena |
| `entity_master.rs:register_entity_with_ptr` | `EcsMaster::create_entity` direct AND `Commands::spawn` apply | Vec::push (heap, not arena) | YES via Commands::spawn apply (dispatcher in apply window) | OK; not arena |

**Audit verdict (Round 2):** Zero `Arena::allocate_*` reachable from worker `run_unsafe`. The `IN_SYSTEM_RUN` debug assert is a defense-in-depth check; not the primary enforcement.

For any future code paths (Phase 10+) that introduce new arena allocation sites, the same audit must be performed. Recommended convention: any new function taking `&Arena` and calling `allocate_*` documents which context it's safe to call from.

---

## §10 — Hot-path performance projections (Round 2 — revised)

### 10.1 ThreadPool::install — entry/exit cost (unchanged)

| Step | Cost |
|------|------|
| `active_scopes.fetch_add` | ~10 ns |
| Allocate `Box<ScopeShared>` | ~50 ns |
| `current_thread()` clone | ~5 ns |
| User closure entry | 0 ns |
| Scope::Drop (no pending) | ~30 ns |
| TLS prev save/restore | ~10 ns |
| `active_scopes.fetch_sub` | ~10 ns |
| **Total** | **~115 ns** |

### 10.2 ThreadPool::scope — entry/exit cost (Round 2 new)

| Step | Cost |
|------|------|
| Allocate `Box<ScopeShared>` | ~50 ns |
| `current_thread()` clone | ~5 ns |
| User closure entry | 0 ns |
| Scope::Drop (no pending) | ~30 ns |
| **Total** | **~85 ns** |

Lower than `install` because no TLS bookkeeping.

### 10.3 Scope::spawn — per-task cost

| Step | Cost |
|------|------|
| `pending.fetch_add` | ~10 ns |
| Box allocation | ~50 ns |
| `transmute` | 0 ns |
| `push_task` (local injector if on worker, global otherwise) | ~30 ns local / ~50 ns global |
| `unpark_one_idle` | ~50 ns (best) / ~10 ns (no idle) |
| **Per spawn** | **~90-140 ns** |

### 10.4 Worker pop + execute (unchanged)

| Step | Cost |
|------|------|
| `Worker::pop` | ~15 ns |
| Closure body | varies |
| `pending.fetch_sub` | ~10 ns |
| Branch | ~2 ns |
| Maybe `waker.unpark()` | ~50 ns (only last task) |

### 10.5 Schedule::run — per-frame dispatcher hot path (Round 2 — recalculated; Round 3 C-NEW-2 — target reconciled with §1.2)

**Scenario:** 50 systems, 16 workers, dense conflicts, incremental ready set.

| Step | Count | Per-call | Total |
|------|-------|----------|-------|
| Outer loop iterations | ~10 (with incremental ready set, far fewer than Round 1) | — | — |
| `apply_window_drain` per iter | ~5 | ~50 × 50 ns = 2.5 µs | ~12.5 µs |
| `try_dispatch_ready` (small ready_queue) | ~10 | ~5 sys × 8 ns = 40 ns | ~400 ns |
| `scope.spawn` per dispatched system | 50 | ~120 ns | ~6 µs |
| `pred_remaining` updates per completion (plain decrement, Round 3 O-NEW-2) | 50 | avg 2 successors × 2 ns | ~200 ns |
| `park_timeout` idle waits | ~3 | immediate return on unpark | ~negligible |
| **Total dispatcher overhead at 50 systems** | | | **~19 µs** |

This is the documented binding target per §1.2 Round 3 revision: **≤ 20 µs at 50 systems**. Round 1's optimistic 5 µs assumed a batched apply path which Phase 9 does not implement (deferred per §1.4). The dominant cost is `apply` itself (~50 ns/system) plus `scope.spawn` (~120 ns/system); apply hoisting (a batched apply queue serviced once per round instead of per-system) is the architectural change that would unlock the original 5 µs budget — punted to a later phase.

**Scenario:** 1024 systems, 16 workers, 50% conflict density, incremental ready set.

| Step | Count | Per-call | Total |
|------|-------|----------|-------|
| Outer loop iterations | ~50 | — | — |
| `apply_window_drain` per iter | ~20 | apply cost 50 ns | ~50 µs total |
| `try_dispatch_ready` (small ready_queue) | ~50 | ~30 sys × 8 ns | ~12 µs |
| `scope.spawn` per dispatched system | 1024 | ~120 ns | ~120 µs |
| `pred_remaining` updates (plain decrement) | 1024 | avg 3 succ × 2 ns | ~6 µs |
| `park_timeout` idle | ~10 | negligible | ~negligible |
| **Total at 1024 systems** | | | **~188 µs** |

**Under the 200 µs target.** ✓ (Round 3 O-NEW-2 dropped the per-decrement cost from 5 ns to 2 ns.)

### 10.6 `par_iter` per-row cost (unchanged from Round 1)

| Step | Cost |
|------|------|
| Inside chunk: 256 rows × `D::fetch` | ~1.3 µs |
| Filter (`F::IS_ARCHETYPAL` const-folded) | 0 ns |
| Body closure | varies |
| Per-chunk total | ~2.6 µs |
| Dispatch per chunk | ~200 ns (~7% overhead) |

---

## §11 — Memory layout summary

### 11.1 Struct sizes table (Round 2 — updated)

| Struct | Size | Padded layout | Alignment |
|--------|------|---------------|-----------|
| `ThreadPool` | ~480 B (Round 2: +injector_local) | 8 lines, CachePadded fields | 64 B |
| `WorkerControl` | 8 B + 32 B (debug counters) → padded to 64 B | CachePadded | 64 B |
| `TaskHandle` | 24 B | natural | 8 B |
| `Scope<'scope>` | 24 B | natural | 8 B |
| `ScopeShared` | ~80 B | CachePadded `pending` | 64 B |
| `Schedule` | ~256 B | natural; CachePadded scratch slots | 8 B |
| `ScheduleBuilder` | ~104 B (Round 2: +pool field 16 B Arc) | natural | 8 B |
| `ConflictGraph` | ~40 B head + heap (Round 2: +pred_count 8 B fat ptr) | heap-side bitsets | 8 B |
| `SystemBox` | 24 B (Round 2: unchanged; is_exclusive cache stays) | natural | 8 B |
| `SystemIndex` | 2 B | repr(transparent) | 2 B |
| `SystemDescriptor` | ~120 B | natural | 8 B |
| `ExecutorScratch` | ~200 B + heap (Round 3: pred_remaining Box<[u16]> — 2 KB at N=1024) | CachePadded slots | 64 B |
| `Access` (existing) | 192 B | optimised | 8 B |
| `SystemMeta` (Round 2: NO is_exclusive field; stays 224 B) | 224 B | 4 lines | 8 B |
| `ApplyDeferred` | ~256 B | mostly meta | 8 B |
| `ParQuery` / `ParQueryMut` | 56 B | natural | 8 B |

### 11.2 False-sharing audit (unchanged from Round 1)

CachePadded fields:
- `ThreadPool::injector`, `injector_local[i]`, `idle`, `shutdown`.
- Per-worker `WorkerControl`.
- `ScopeShared::pending`.
- `ExecutorScratch::pending_apply`.

### 11.3 ComponentPool::grow deferral (Round 2 — new)

`ComponentPool::push` (called from `Archetype::create_entity`) may call `ComponentPool::grow` when a chunk fills. `grow` allocates a new chunk via `arena.allocate_layout(&self)`.

**ALLOC2 routing:** every call to `Archetype::create_entity` originates from:
1. `EcsMaster::create_entity` — direct dispatcher API; called pre-schedule or in apply window.
2. `Commands::spawn` apply — called only in apply window (workers drained).

Neither path runs while workers hold references. Therefore `ComponentPool::grow → arena.allocate_*` is sound under SEND1.

### 11.4 EntityMaster pre-sizing (Round 2 W3 — new)

`EntityMaster::with_capacity(MAX_ENTITIES_HINT)` is called at `EcsMaster::new`. `MAX_ENTITIES_HINT` defaults to 64 K; configurable via `EcsMasterConfig::max_entities_hint`. Pre-allocation avoids `Vec::push` reallocation for the first 64 K entities.

For workloads exceeding 64 K entities: subsequent `push` may reallocate `entities_inland` and `sparse_to_active`. **All such growth happens in the apply window** (because `register_entity_with_ptr` is called from `Commands::spawn` apply or direct dispatcher API).

The `EcsMaster::new` flow:
```rust
let em_config = EcsMasterConfig {
    max_entities_hint: 64_000,
    // ...
};
let entity_master = EntityMaster::with_capacity(em_config.max_entities_hint);
```

Doc-comment on `with_capacity`: "Pre-allocates internal vectors for the given entity count. Subsequent growth may reallocate; under Phase 9 scheduler this only happens in the apply window."

---

## §12 — Public API surface

### 12.1 boyko_threadpool

```rust
pub use pool::{ThreadPool, ThreadPoolBuilder, MAX_WORKERS};
pub use scope::Scope;
pub use stats::PoolStats;
pub use tls::{current_worker_id, WORKER_ID_DISPATCHER, WORKER_ID_UNATTACHED};

impl ThreadPoolBuilder {
    pub fn new() -> Self;
    pub fn num_threads(self, n: usize) -> Self;
    pub fn thread_name_prefix(self, s: impl Into<String>) -> Self;
    pub fn pin_workers(self, on: bool) -> Self;
    pub fn stack_size(self, bytes: usize) -> Self;
    pub fn build(self) -> ThreadPool;
}

impl ThreadPool {
    /// Block the calling thread until `f` returns; scope.spawn'd tasks
    /// run on workers, joined automatically. Sets ACTIVE_POOL TLS.
    /// Entry point for non-worker callers (typically the dispatcher).
    pub fn install<R, F>(&self, f: F) -> R
    where F: FnOnce(&Scope<'_>) -> R + Send;

    /// Lightweight re-entrant scope creation; expected to be called from
    /// inside a worker task (or from inside an `install` body). Does NOT
    /// modify ACTIVE_POOL TLS — assumes it's already set.
    pub fn scope<'s, R, F>(&'s self, f: F) -> R
    where F: FnOnce(&Scope<'s>) -> R + Send;

    pub fn spawn<F>(&self, f: F)
    where F: FnOnce() + Send + 'static;

    pub fn num_threads(&self) -> usize;
    pub fn stats(&self) -> PoolStats;
}

impl<'scope> Scope<'scope> {
    pub fn spawn<F>(&self, f: F)
    where F: FnOnce(&Scope<'scope>) + Send + 'scope;
}
```

### 12.2 boyko_ecs::schedule

```rust
pub use schedule::Schedule;
pub use schedule_builder::ScheduleBuilder;
pub use system_set::SystemSet;
pub use ordering::Order;

impl ScheduleBuilder {
    /// Construct a builder; the pool is stored and reused at run time.
    /// Round 2 W4: pool owned by builder, transferred to Schedule at build.
    pub fn new(pool: Arc<ThreadPool>) -> Self;

    /// Add a system. `Out = ()` is required for schedule use.
    /// Systems with non-`()` output must use `EcsMaster::run_system`
    /// outside the scheduler (Round 2 Q1).
    pub fn add_system<F, M>(&mut self, system: F) -> SystemConfig<'_>
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;

    pub fn build(self, world: &mut EcsMaster) -> Schedule;
}

impl<'b> SystemConfig<'b> {
    pub fn before<F, M>(self, other: F) -> Self
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;
    pub fn after<F, M>(self, other: F) -> Self
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;
    pub fn in_set<S: SystemSet>(self, set: S) -> Self;
    pub fn ambiguous_with<F, M>(self, other: F) -> Self
    where F: IntoSystem<(), (), M>, F::System: System<Out = ()> + 'static;
    pub fn no_sync(self) -> Self;
}

impl Schedule {
    /// Run one frame. Executes every system once respecting Access conflicts
    /// and DAG ordering. Auto-inserted sync points flush pending commands.
    /// Round 2 SCH7: apply runs in an explicit barrier window after workers
    /// drain; no UB-language-level aliasing.
    pub fn run(&mut self, world: &mut EcsMaster);

    pub fn len(&self) -> usize;
    pub fn stats(&self) -> ScheduleStats;
}

pub trait SystemSet: Hash + Eq + Clone + Send + Sync + 'static {
    fn id(&self) -> SystemSetId;
}
```

### 12.3 boyko_ecs Query par_iter

```rust
impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    pub fn par_iter<'q>(&'q self) -> ParQuery<'q, 's, D, F>
    where D: ReadOnlyQueryData;
    pub fn par_iter_mut<'q>(&'q mut self) -> ParQueryMut<'q, 's, D, F>;
}

impl<'q, 's, D: QueryData, F: QueryFilter> ParQuery<'q, 's, D, F>
where D: ReadOnlyQueryData
{
    pub fn with_chunk_size(self, n: usize) -> Self;
    pub fn for_each<Body>(self, body: Body)
    where Body: Fn(D::Item<'_>) + Send + Sync;
}
```

### 12.4 EcsMaster + EventDispatcher integration (Round 2 — new methods)

```rust
impl EcsMaster {
    pub fn set_default_pool(&mut self, pool: Arc<ThreadPool>);
    pub fn default_pool(&self) -> Option<&Arc<ThreadPool>>;
}

impl EventDispatcher {
    /// Round 2 EVT1: convenience wrapper reading worker_id from TLS.
    /// Returns the same EcsResult as `send(thread_index, event)`.
    pub fn send_event<E: Event>(&self, event: E) -> EcsResult<()>;
}

impl<'s> Commands<'s> {
    /// Enqueue a SendEventCommand. Flushes in apply window onto lane
    /// `worker_count` (dispatcher's lane).
    pub fn send_event<E: Event>(&mut self, event: E);
}
```

`EventConfig::default_for(thread_count)` updated to `default_for(worker_count + 1)` for the dispatcher's lane allocation.

### 12.5 Access (Round 2 — new method; Round 3 C-NEW-1 — event fields removed)

```rust
impl Access {
    /// True iff every read/write bitmask is fully set (all components,
    /// all resources, both reads and writes). Used by SystemBox to compute
    /// the is_exclusive cache (Round 2 C9 / OQ-4).
    ///
    /// Round 3 C-NEW-1 — event lane access is OUTSIDE the schedule's
    /// conflict graph. Per-lane single-writer discipline via EVT1 TLS
    /// guarantees correctness without graph participation (each worker
    /// writes only to its own lane index; dispatcher writes to lane
    /// `worker_count`). ApplyDeferred (universal access) blocks every
    /// other system equally; event lane safety during apply is preserved
    /// without any event field in `Access`. Phase 12 EventReader /
    /// EventWriter SystemParam will revisit if needed.
    ///
    /// Therefore `is_universal()` checks ONLY the 4 existing bitmasks
    /// declared in `Access` (`component_reads`, `component_writes`,
    /// `resource_reads`, `resource_writes`). Adding event_* fields
    /// would extend the struct beyond 192 B and break the cache-line
    /// invariant asserted at `access.rs:61`.
    #[inline]
    pub fn is_universal(&self) -> bool {
        self.component_reads.is_all_set()
            && self.component_writes.is_all_set()
            && self.resource_reads.is_all_set()
            && self.resource_writes.is_all_set()
    }

    /// Construct an Access with every bit set across the 4 bitmasks.
    pub fn universal() -> Self {
        let mut a = Self::new();
        a.component_reads.set_all();
        a.component_writes.set_all();
        a.resource_reads.set_all();
        a.resource_writes.set_all();
        a
    }
}
```

Underlying `BitSet256` (used for `resource_reads`/`resource_writes`) and `ComponentMask` (used for `component_reads`/`component_writes`) must expose `is_all_set()` and `set_all()`. **Verified absent** in `boyko_utils/src/bit_mask/bit_set_256.rs` and `boyko_ecs/src/ecs/core/component/component_mask.rs` (Round 3). Step 7c adds them with the simple `all bits == max value` implementation:

```rust
// boyko_utils: bit_set_256.rs
impl BitSet256 {
    #[inline]
    pub fn is_all_set(&self) -> bool {
        self.words[0] == u64::MAX
            && self.words[1] == u64::MAX
            && self.words[2] == u64::MAX
            && self.words[3] == u64::MAX
    }

    #[inline]
    pub fn set_all(&mut self) {
        self.words = [u64::MAX; 4];
    }
}

// boyko_ecs: component_mask.rs (8 × BitSet<u64> = 512 bits)
impl ComponentMask {
    #[inline]
    pub fn is_all_set(&self) -> bool {
        // Each block is BitSet<u64>; "all set" = inner u64 == u64::MAX.
        self.blocks.iter().all(|b| b.as_u64() == u64::MAX)
    }

    #[inline]
    pub fn set_all(&mut self) {
        for b in self.blocks.iter_mut() {
            b.fill();   // helper that sets the inner u64 to u64::MAX
        }
    }
}
```

(If `BitSet<u64>` doesn't expose `as_u64()` / `fill()`, Step 7c adds those helpers too.)

---

## §13 — Test plan

### 13.1 Unit tests (Round 2 — additions noted)

**boyko_threadpool:**
- `pool::tests::install_runs_closure`.
- `pool::tests::scope_runs_closure` (Round 2 — new for §4.5.5 scope API).
- `pool::tests::scope_blocks_until_drain`.
- `pool::tests::panic_in_task_propagates`.
- `pool::tests::multiple_panics_first_wins`.
- `pool::tests::nested_install_works`.
- `pool::tests::nested_scope_works` (Round 2 — new).
- `pool::tests::scope_from_worker_no_deadlock` (Round 2 — new for C3).
- `pool::tests::shutdown_joins_all_workers`.
- `scope::tests::scope_spawn_borrows_stack_data`.
- `pool::tests::work_stealing_load_balances`.
- `pool::tests::idle_bitset_round_trip`.
- `pool::tests::backoff_progression`.
- `tls::tests::worker_id_visible_inside_install` (Round 2 — new).
- `tls::tests::worker_id_visible_inside_worker_task` (Round 2 — new).

**boyko_ecs::schedule:**
- `schedule_builder::tests::detect_cycle`.
- `schedule_builder::tests::topological_sort_simple`.
- `schedule_builder::tests::expand_sets`.
- `conflict_graph::tests::build_access_conflicts`.
- `conflict_graph::tests::pred_count_correct` (Round 2 — new).
- `executor::tests::single_system_runs_once`.
- `executor::tests::two_independent_systems_parallel`.
- `executor::tests::conflicting_systems_serialize`.
- `executor::tests::exclusive_system_blocks_others`.
- `executor::tests::apply_window_barrier_no_aliasing` (Round 2 — new, integration of C4 fix).
- `executor::tests::is_exclusive_cache_matches_access_universal` (Round 2 — new, C9).
- `executor::tests::cell_minted_per_round_not_per_loop_iter` (Round 3 W-NEW-1 — new; pins the cell-lifetime rhythm).
- `apply_deferred::tests::auto_sync_between_spawn_and_query`.
- `apply_deferred::tests::no_sync_skip`.

**par_iter:**
- `par_iter::tests::single_archetype_all_rows_visited`.
- `par_iter::tests::multi_archetype_parallel_chunks`.
- `par_iter::tests::par_iter_mut_persists`.
- `par_iter::tests::par_iter_outside_pool_panics`.
- `par_iter::tests::nested_par_iter_panics`.
- `par_iter::tests::min_chunk_size_enforced`.
- `par_iter::tests::tiny_archetype_runs_inline` (Round 2 O2 — new).
- `par_iter::tests::par_iter_from_system_body_no_deadlock` (Round 2 C3 — new).

**Access:**
- `access::tests::is_universal_empty_false` (Round 2 — new).
- `access::tests::is_universal_partial_false` (Round 2 — new).
- `access::tests::is_universal_full_true` (Round 2 — new).
- `access::tests::universal_constructor_universal` (Round 2 — new).

**Arena allocation discipline (Round 2 ALLOC1 — new):**
- `arena::tests::allocate_outside_run_unsafe_ok` (Round 2 — new).
- `arena::tests::allocate_inside_run_unsafe_debug_panics` (Round 2 — new, debug-only).

**Compile-fail tests (Round 2 W2):**
- `tests/par_iter_captures_commands_fails.rs` (Round 2 — new): captures `&mut Commands` in `par_iter`'s `Fn` body; must fail to compile.

### 13.2 Integration tests

`tests/scheduler_smoke.rs`, `scheduler_panic_recovery.rs`, `scheduler_apply_deferred_integration.rs`, `par_iter_stress.rs`, `send_sync_negative.rs` (unchanged from Round 1).

Round 2 additions:
- `tests/scheduler_apply_window_stress.rs` — stress test that interleaves apply-window with worker dispatches; asserts no aliasing UB (run under Miri).
- `tests/scheduler_par_iter_concurrent_systems.rs` — Round 2 C3: N concurrent systems each calling `par_iter`; assert no deadlock.

### 13.3 Miri tests

Round 1 set + Round 2 additions:
- `executor::tests::apply_window_barrier_no_aliasing`.
- `arena::tests::allocate_inside_run_unsafe_debug_panics` (debug-only; Miri runs in debug).

### 13.4 Loom tests (Round 2 — expanded per C7)

`tests/loom_pool.rs`:

#### 13.4.1 `loom_unpark_one_idle_races_park` — four scenarios

```text
Race A: pusher push BEFORE worker mark_idle, worker still in spin phase.
  Worker's pre-mark_idle re-poll catches the pushed task. Worker does not park.
  Wake-up NOT lost.

Race B: pusher push BETWEEN steps 4 (shutdown check) and 5 (park).
  Pusher's unpark_one_idle reads idle with our bit set; clears it; calls unpark.
  Worker enters park; std's sticky unpark returns it immediately.
  Wake-up NOT lost.

Race C: pusher push BEFORE worker mark_idle, but worker's pre-mark_idle re-poll
  ALSO misses the push (race on injector internal head/tail). Worker enters mark_idle.
  Then re-polls AGAIN inside the post-mark_idle gate. The post-mark_idle re-poll
  catches the task (Release/Acquire pair on injector). Worker does not park.
  Wake-up NOT lost.

Race D: worker has parked; pusher push lands; unpark_one_idle sees bit set;
  clears + unparks. Worker wakes, unmarks idle, polls, finds task.
  Wake-up delivered exactly once.
```

Each scenario tested as a separate loom test with explicit interleavings.

#### 13.4.2 `loom_scope_drop_panic_with_pending` (Round 2 C6 — new)

```text
Scenario: Scope::spawn(N tasks). Worker task k panics. Scope::Drop runs on
  the calling thread. Calling thread enters work-stealing wait. Other workers
  complete remaining tasks; pending hits 0. Drop reads panic_payload; calls
  resume_unwind.

Assertions:
  - All N-1 non-panicking tasks complete.
  - resume_unwind called exactly once with the panic payload.
  - No deadlock under any task interleaving.
```

#### 13.4.3 `loom_completion_queue_push_pop`

Already in Round 1; covers ArrayQueue MPSC semantics.

#### 13.4.4 `loom_apply_window_gate` (Round 2 C4 — new)

```text
Scenario: dispatcher dispatches 2 systems (A, B). Workers execute A and B.
  Worker A pushes completion + Release-pending_apply.fetch_add(1).
  Worker B does the same.
  Dispatcher loops: Acquire-pending_apply == running.count_ones() == 2 → enter
  apply window.
  Drain 2 completions. Apply A, then apply B.

Loom verifies:
  - The Acquire load synchronizes with both Releases.
  - The dispatcher sees both workers' writes to component bytes BEFORE
    constructing &mut EcsMaster.
  - No interleaving exists where apply runs before both completions are visible.
```

### 13.5 Criterion benches

Round 1 set unchanged. Round 2 additions:

`benches/incremental_ready_set.rs` — compare full-scan vs incremental ready-set at N = 100/500/1024 systems. Target: incremental ≤ 20% of full-scan cost.

`benches/scope_vs_install.rs` — measure entry/exit cost of `scope` vs `install` for empty body. Target: scope ≤ 100 ns.

#### 13.5.6 "Worker idle ≤ 1% CPU per core" methodology (Round 2 O4)

```text
Setup: ThreadPool with 8 workers; no tasks submitted for 5 seconds.
Measurement on Linux: getrusage(RUSAGE_THREAD) per-worker, sample at t=0 and t=5s.
Measurement on Windows: GetThreadTimes per-worker, same sampling.
Per-worker CPU% = (user_time + kernel_time) / wall_time × 100.

Pass criterion: every worker reports < 1.0% CPU during the idle window.

Expected: workers park on std::thread::park (futex/WaitOnAddress, 0% CPU).
The 1% budget accommodates:
  - The spin/yield phase before park (Backoff: ~6 PAUSE + ~32 yield iters,
    typically < 1 µs of CPU per backoff round).
  - Periodic park_timeout wake-ups (none in the idle scenario; only triggered
    by a pusher's unpark).
```

### 13.6 Debug-assertion invariants (Round 2 — revised, Round 3 O-NEW-3)

Mandatory `debug_assert!` insertions:

- `Scope::Drop` post-condition: `pending == 0`.
- `Schedule::run` precondition: `running.count_ones() == 0 && completed.count_ones() == 0`.
- `Schedule::run` precondition: `for each sb in systems: sb.is_exclusive == sb.system.access().is_universal()` (Round 2 SCH15).
- `ConflictGraph::build` post-condition: every conflict bit is symmetric.
- `unpark_one_idle` post-condition: if returned true, the unparked worker's bit is 0.
- `find_ready` post-condition (Round 2 W7 — replaced): `running ∩ ready_scratch is empty` AND `for i ∈ ready_scratch: pred_remaining[i] == 0`.
- `Arena::allocate_*` precondition (Round 2 ALLOC6): `!IN_SYSTEM_RUN.get()`.
- `apply_window_drain` precondition (Round 2 SCH7): gate fired (`pending_apply == running.count_ones() && pending_apply > 0` OR `pending_apply > 0 && running.count_ones() == 0`); on exit, `pending_apply == 0`.
- `Schedule::run` postcondition: `pending_apply == 0`.
- `ScheduleBuilder::build` (Round 3 O-NEW-3): `descriptors.len() <= u16::MAX` AND every `pred_count[i] <= u16::MAX`. The hard cap `MAX_SYSTEMS_PER_SCHEDULE = 1024` is well within these bounds; the assertion catches accidental future cap expansion.

---

## §14 — Step-by-step implementation (Round 2 — expanded to 24 Steps)

### Wave 1 (foundation; serial):

**Step 1 — Workspace + boyko_threadpool skeleton**
- Files: `crates/boyko_threadpool/Cargo.toml`, `src/lib.rs`, `src/pool.rs`, `src/scope.rs`, `src/stats.rs`, `src/tls.rs`.
- Deliverables: empty crate scaffold; CI updated.
- Acceptance: `cargo check -p boyko-threadpool` builds.

**Step 2 — ThreadPool::new + workers + injector + local injectors + stealers**
- Round 2 C2: add `injector_local: Arc<[CachePadded<Injector<TaskHandle>>]>`.
- Implement `ThreadPoolBuilder::build`, worker spawn, worker_main loop's first 3 stages.
- Acceptance: `pool.spawn()` works.

**Step 3 — Parking + idle bitset + backoff + TLS worker id**
- Round 2 TPN13: implement TLS `CURRENT_WORKER_ID` setup at worker entry.
- Acceptance: workers park; `pool.spawn` wakes one; TLS `current_worker_id()` returns correct id.

**Step 4 — Scope + install + scope + panic propagation + Scope::Drop work-stealing**
- Round 2 C3 / C6: implement `join_workers_until_drained` for `Scope::Drop` — Round 3 W-NEW-2 form (no own-deque drain; only local injector + global + sibling steal).
- Round 2 W6: implement both `install` and `scope`.
- Acceptance: install/scope smoke tests; panic propagation; nested scope test; scope-from-worker no deadlock.

**Step 5 — Worker affinity + ThreadPoolBuilder fluent API + stats**
- Implement `pin_workers`, `thread_name_prefix`, `stack_size`, `num_threads`.
- Wire `#[cfg(feature = "scheduler-trace")]` counters.

**Step 6 — Loom tests for the pool**
- Round 2 C7 / C6: implement four labeled loom scenarios (A/B/C/D) + `loom_scope_drop_panic_with_pending`.

### Wave 2 (Send/Sync + allocation guards; parallelizable trio):

**Step 7a — Send/Sync for memory + archetypes + entities + events (NO Arena)**
- Round 2 C1: do NOT add Send/Sync to `Arena`.
- Add `unsafe impl Send + Sync` to `ComponentPool`, `Archetype`, `ArchetypeMaster`, `EntityMaster`, `EventDispatcher`, `Resources`.
- Each carries the SAFETY block per §9.2.

**Step 7b — Send/Sync for EcsMaster + UnsafeEcsCell**
- Round 2 SEND1/SEND3: add `unsafe impl Send + Sync for EcsMaster`, `for UnsafeEcsCell<'w>`.

**Step 7c (Round 2 — new, Round 3 — clarified) — Access::is_universal + Access::universal + allocation guards + bitset helpers**
- Round 2 C9 / Round 3 C-NEW-1: implement `Access::is_universal()` and `Access::universal()` checking only the 4 existing bitmasks.
- Round 2 ALLOC6: implement `InSystemRunGuard` RAII in `boyko_threadpool::tls`.
- Round 2 ALLOC6: add `debug_assert!(!IN_SYSTEM_RUN.get())` to `Arena::allocate_layout` and `Arena::allocate_from_free_blocks`.
- Add `BitSet256::is_all_set()` / `set_all()` to `boyko_utils` (Round 3 verification: not present today; new helpers with `all_words == u64::MAX` implementation).
- Add `ComponentMask::is_all_set()` / `set_all()` to `boyko_ecs::core::component::component_mask` (Round 3 verification: not present today; new helpers iterating 8 blocks).
- Add `BitSet<u64>` helpers if needed (`as_u64()`, `fill()` or equivalent).

**Step 7 gate:** Steps 7a + 7b + 7c can land in parallel (independent files).

### Wave 3 (schedule scaffolding):

**Step 8 — Schedule module skeleton + SystemBox + SystemDescriptor + ExclusiveFunctionSystem**
- Files: `crates/boyko_ecs/src/ecs/core/schedule/{mod.rs
, system_box.rs, system_descriptor.rs, ordering.rs, system_set.rs, exclusive.rs}`.
- Implement `SystemBox` (build-time `is_exclusive` cache from `Access::is_universal`).
- Implement `ExclusiveFunctionSystem<F>` + `IntoSystem` blanket impl with `ExclusiveSystemMarker` (Round 2 W5).
- Acceptance: crate builds; `tests/into_system_exclusive_smoke.rs` confirms inference.

**Step 9 — ScheduleBuilder + Tarjan SCC + topological sort**
- Round 2 W4: `ScheduleBuilder::new(pool: Arc<ThreadPool>)`.
- Round 3 W-NEW-3: `build` body pre-destructures `self` into `(pool, descriptors, order_edges, sets)`; names/lengths captured before any move.

**Step 10 — ConflictGraph build + incremental pred_count + SIMD bitset_intersects**
- Round 2 C8: include `pred_count` per system.
- Implement AVX2 `bitset_intersects_avx2` + scalar fallback.

### Wave 4 (executor):

**Step 11 — ExecutorScratch + completion queue + pred_remaining (Box<[u16]>) + pending_apply atomics**
- Round 2 C8 + Round 3 O-NEW-2: `pred_remaining: Box<[u16]>` (plain `u16`, dispatcher-owned).
- Round 2 C4: `pending_apply: CachePadded<AtomicUsize>`.

**Step 12 — Executor apply-window main loop**
- Round 2 C4: implement `apply_window_drain` + the gate `pending_apply == running.count_ones()` with bootstrap `running == 0` clause.
- Round 3 W-NEW-1: implement loop rhythm `[apply window drain] → [completed-check return] → [mint cell] → [try dispatch]`; the cell is minted once per outer iteration.
- Round 3 W-NEW-4: carry monotonicity comment on the gate.
- Per-round `UnsafeEcsCell` re-minting (Round 2 O3).
- Acceptance: `executor::tests::single_system_runs_once`, `two_independent_systems_parallel`, `conflicting_systems_serialize`, `apply_window_barrier_no_aliasing`, `cell_minted_per_round_not_per_loop_iter`.

**Step 13 — Exclusive systems integration**
- Round 3 W-NEW-5: exclusive path uses `unsafe { cell.world_mut() }` for both `run_unsafe` (cell-typed) and `apply` (reborrowed `&mut EcsMaster`); SAFETY block per EXC1.
- Acceptance: `executor::tests::exclusive_system_blocks_others`.

**Step 14 — Apply phase + sync-point analyzer**
- Implement `ApplyDeferred` marker + `insert_sync_points` algorithm.
- Acceptance: `apply_deferred::tests::auto_sync_between_spawn_and_query`, `no_sync_skip`.

### Wave 5 (par_iter):

**Step 15 — ParQuery / ParQueryMut + for_each + use ThreadPool::scope**
- Round 2 W6: use `pool.scope`, not `pool.install`.
- Round 2 O2 / PAR9: `MIN_ARCHETYPE_FOR_PARALLEL = 1024` inline path.
- Acceptance: `par_iter::tests::*` including `par_iter_from_system_body_no_deadlock`, `tiny_archetype_runs_inline`.

**Step 16 — par_iter integration with Schedule**
- Ensure scheduler sets up TLS active_pool correctly; par_iter from inside a system body works.
- Acceptance: integration test `tests/scheduler_par_iter_concurrent_systems.rs`.

### Wave 6 (correctness + EventDispatcher integration):

**Step 17 — Send/Sync negative tests + Commands captures par_iter compile_fail**
- `tests/send_sync_negative.rs`.
- Round 2 W2: `tests/par_iter_captures_commands_fails.rs`.

**Step 18 — EventDispatcher::send_event + Commands::send_event integration**
- Round 2 EVT1-EVT4: implement `send_event<E>(&self, event)` reading TLS.
- Add `Commands::send_event` enqueuing.
- Update `EventConfig::default_for(worker_count + 1)`.
- Acceptance: `tests/event_send_from_worker.rs` (Round 2 — new).

**Step 19 — Miri test suite**
- Curate Miri-clean set.
- Acceptance: `cargo +nightly miri test --features scheduler-trace`.

**Step 20 — Criterion benches**
- Round 2 additions: `incremental_ready_set.rs`, `scope_vs_install.rs`.

### Wave 7 (rough edges):

**Step 21 — SystemSet derive macro**
- Add `#[derive(SystemSet)]` to `boyko_macros`.

**Step 22 — Documentation in book/src/**
- Add `book/src/scheduler.md`.
- Round 2: must include sections on the apply-window contract and allocation discipline.

**Step 23 — Migration / legacy compat audit**
- Verify `EcsMaster::run_system` / `run_cached_system` still works.

**Step 24 (Round 2 — new) — Allocation discipline CI integration**
- Add a CI step that runs the spawn-storm stress test under debug build with `RUSTFLAGS="--cfg force_alloc_panic"` (a tighter version of ALLOC6 that panics in release too).
- Verifies the discipline holds against future refactors that might accidentally allocate inside a system body.
- Round 3 O-NEW-1: this is the release-mode safety net referenced in §9.2.

### Parallelizable pairs

- **7a + 7b + 7c** can land together (one developer; independent files).
- **15 + 16** in parallel.
- **18 + 21** in parallel (orthogonal).
- **22** (docs) in parallel with **24** (CI).

### Dependency graph

```
Step 1 → 2 → 3 → 4 → 5 → 6
                  ↓
                7a, 7b, 7c (parallel)
                        ↓
                      8 → 9 → 10 → 11 → 12 → 13 → 14
                                          ↓             ↘
                                        15            16 (joins 15)
                                                          ↓
                                                        17, 18 (parallel)
                                                              ↓
                                                        19, 20, 21, 22, 23, 24
```

---

## §15 — Migration impact

### 15.1 Affected files in the existing tree

**Modified files** (Round 2 — updated):
- `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` — add `unsafe impl Send + Sync`. Optionally add `default_pool: Option<Arc<ThreadPool>>`. Update `EcsMaster::new` to call `EntityMaster::with_capacity(64_000)` (Round 2 W3).
- ~~`crates/boyko_ecs/src/ecs/memory/arena.rs` — add `unsafe impl Send + Sync`.~~ **REVERTED Round 2 C1**: arena stays `!Send + !Sync`. ADD debug_assert in `allocate_*` for `!IN_SYSTEM_RUN`.
- `crates/boyko_ecs/src/ecs/memory/component_pool.rs` — add `unsafe impl Send + Sync` (post-Round 2 SEND10 — pool is Send/Sync iff Arena access is disciplined).
- `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` — `unsafe impl Send + Sync` for `Archetype`, `Column`.
- `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` — `unsafe impl Send + Sync`.
- `crates/boyko_ecs/src/ecs/core/entity/entity_master.rs` — `unsafe impl Send + Sync`.
- `crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs` — `unsafe impl Send + Sync` + new `send_event<E>` wrapper.
- `crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs` — `unsafe impl<'w> Send + Sync`.
- ~~`crates/boyko_ecs/src/ecs/core/system/system_meta.rs` — add `is_exclusive` field.~~ **REVERTED Round 2 C9**: no field; `Access::is_universal()` is the source. Size stays 224 B.
- `crates/boyko_ecs/src/ecs/core/system/access.rs` — add `is_universal()`, `universal()` (Round 3 C-NEW-1: 4-field check only).
- `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` — add `par_iter` / `par_iter_mut`.
- `crates/boyko_utils/src/bit_mask/bit_set_256.rs` — add `is_all_set()`, `set_all()` (Round 3 verification: helpers absent today; required by Step 7c).
- `crates/boyko_ecs/src/ecs/core/component/component_mask.rs` — add `is_all_set()`, `set_all()` (Round 3 verification: helpers absent today; required by Step 7c).

**New files** (Round 2 — additions noted):
- `crates/boyko_threadpool/` (whole sub-crate).
- `crates/boyko_threadpool/src/tls.rs` (Round 2 — InSystemRunGuard).
- `crates/boyko_ecs/src/ecs/core/schedule/` (whole module tree).
- `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs`.
- `crates/boyko_macros/src/system_set.rs`.
- Test files in `crates/boyko_ecs/tests/` and `crates/boyko_threadpool/tests/`.

### 15.2 Cargo.toml changes (Round 2 — `smallvec` added)

`crates/boyko_threadpool/Cargo.toml`:
```toml
[package]
name = "boyko-threadpool"
version = "0.1.0"
edition = "2024"

[dependencies]
crossbeam-deque = "0.8"
crossbeam-utils = "0.8"

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
loom = "0.7"

[features]
default = []
scheduler-trace = []
loom = []
```

`crates/boyko_ecs/Cargo.toml` — add:
```toml
[dependencies]
# existing
boyko-utils = { path = "../boyko_utils" }
boyko-threadpool = { path = "../boyko_threadpool" }
fixedbitset = "0.5"
crossbeam-queue = "0.3"
crossbeam-utils = "0.8"        # for CachePadded
smallvec = "1"                  # Round 2 W8: explicit direct dep
num_cpus = "1.16"               # for default thread count

[dev-dependencies]
static_assertions = "1.1"

[features]
default = []
scheduler-trace = ["boyko-threadpool/scheduler-trace"]
```

Root `Cargo.toml` — workspace members updated to include `crates/boyko_threadpool`.

### 15.3 Public-API breakage

**None for the existing Phase 8.x users.**

- `EcsMaster::run_system`, `run_cached_system`, `run_system_once`, `run_closure_once` continue to work unchanged.
- `Query::iter`, `iter_mut`, `archetype_count`, `is_empty` unchanged.
- `Commands::spawn`, `add` unchanged.
- `SystemMeta` size unchanged at 224 B (Round 2 C9 — no field added).

**Additions:**
- `Schedule` + `ScheduleBuilder` + `SystemConfig` + `SystemSet` are net-new.
- `Query::par_iter` / `par_iter_mut` are net-new.
- `ThreadPool` + `ThreadPoolBuilder` + `Scope` are net-new.
- `Access::is_universal()` / `Access::universal()` — net-new (`Access` was internal-ish; users rarely touch it).
- `EventDispatcher::send_event<E>(&self, event)` — net-new wrapper.
- `Commands::send_event<E>(&mut self, event)` — net-new convenience.
- `BitSet256::is_all_set()` / `set_all()` — net-new.
- `ComponentMask::is_all_set()` / `set_all()` — net-new.

**Behavioural changes:**
- `EcsMaster: Send + Sync` now compiles. Old code passing `&mut EcsMaster` is unaffected.
- `EventConfig::default_for(thread_count)` callers in tests should pass `worker_count + 1` going forward (the dispatcher uses lane `worker_count`). Existing single-threaded tests using `thread_count = 1` still work (dispatcher uses lane 1 — bound check is `tid < thread_count`).

### 15.4 Phase 8.5 compatibility (Round 2 Q2 — confirmed)

`bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` is Send + Sync because `OnceLock<T>: Sync` when `T: Send + Sync` ([std docs](https://doc.rust-lang.org/std/sync/struct.OnceLock.html)). `ArchetypeId` is `Copy + Send + Sync`. No change needed.

`OnceLock::get(&self) -> Option<&T>` is lock-free and safe for many readers. Round 2: cache reads happen on the dispatcher only (apply window) because `Commands::spawn` enqueues; the actual lookup in apply walks the cache. So worker-side reads of the cache do NOT happen in normal flow. If a future revision moves cache reads to workers, `OnceLock::get` remains sound by construction.

The "race on `OnceLock::set`" path: under Phase 9, two systems both calling `Commands::spawn(SameBundle)` enqueue independent SpawnCommands; both flushes happen in the apply window (serial). The first calls `OnceLock::set` (succeeds), the second calls `get` (already populated). No race.

### 15.5 Phase 8a/c migration

`EcsMaster::run_system` / `run_cached_system` remain. They take `&mut EcsMaster` directly. The new `Schedule::run` uses `System::run_unsafe` directly (not via `run_system`), so no recursion / no double-init.

---

## §16 — Rejected alternatives

(Unchanged from Round 1; reaffirmed in Round 2.)

### A. Lockstep "phase" scheduling — rejected (§3 Q2).
### B. Actor-model — rejected (cache fragmentation).
### C. Single global Mutex<EcsMaster> — rejected (defeats scheduler purpose).
### D. Per-component RwLock — rejected (~20 ns per lock dwarfs work).
### E. Tokio runtime — rejected (no async benefit for run-to-completion).
### F. Rayon global pool — rejected (per-Schedule pools).
### G. crossbeam-channel for completion — rejected (we use ArrayQueue directly).
### H. Cooperative cancellation — rejected (SCH11 Bevy semantic).
### I. Phase 9 implements its own Chase-Lev deque — rejected (verified crossbeam).
### J. `Box<dyn SystemSet>` — rejected (TypeId-keyed lookup).
### K. Stack-based work-stealing — rejected (archetype chunks already large enough).
### L. Lock-free Schedule (Arc<Schedule>) — rejected (`&mut self` correct).
### M. `unsafe impl Send + Sync` for `Schedule` — rejected (apply must run on dispatcher).

### N. (Round 2 — new) `unsafe impl Send + Sync` for `Arena`
**Rejected**: Round 1 proposed making Arena Send+Sync to permit allocation from workers. Round 2 critic showed `allocate_layout(&self)` calls would race in `MemFreeBlockMaster` under multi-thread access — direct UB. Alternatives considered:
- **Lock-free `MemFreeBlockMaster`** (compare-exchange free-list pop) — significant new code; out of Phase 9 scope.
- **Restrict allocation to apply window via discipline** (Round 2 C1 Option 3, chosen) — minimal code change; leverages existing dispatcher-exclusive apply path.

### O. (Round 2 — new) Per-frame `find_ready` O(N²) scan with deferred fix to Phase 9.1
**Rejected**: Round 1 plan deferred the incremental ready-set fix to a future Phase 9.1 patch citing "ship and measure first" — but the §1.2 dispatcher target (200 µs at N=1024) is a binding acceptance gate. Round 2 C8: implement `pred_remaining` counters now (one additional field + 20-line update); cost saved is critical to hitting the target. Adopted in §7.4. Round 3 O-NEW-2 further downgraded `pred_remaining` from `Box<[AtomicU16]>` to `Box<[u16]>` (dispatcher-owned; no atomic needed).

### P. (Round 2 — new) `is_exclusive` flag on `SystemMeta`
**Rejected**: Duplicates `Access::is_universal()`. Round 2 C9: drop the flag; compute exclusivity from the access bitmask. Single source of truth.

### Q. (Round 2 — new) `pool.install` from inside `par_iter` (nested install)
**Rejected**: Causes nested-install bookkeeping confusion AND can deadlock when N concurrent par_iter invocations exhaust workers. Round 2 W6: introduce `pool.scope` (re-entrant, no install TLS reset) for use from worker contexts. par_iter calls `scope`. Round 2 C3: `Scope::Drop` work-steals while waiting (rayon pattern), eliminating the N-workers-all-blocked deadlock.

### R. (Round 3 — new) Event lanes participate in `Access` conflict graph
**Rejected**: Would require adding `event_reads: BitSet256` + `event_writes: BitSet256` (or larger) to `Access`, breaking the 192 B / 3-cache-line invariant currently asserted at `access.rs:61`. Per-lane single-writer discipline (EVT1 TLS routing) already guarantees correctness without conflict-graph participation; each worker writes only to its own lane, dispatcher writes to lane `worker_count`, and ApplyDeferred (universal access) blocks every other system equally. Phase 12 `EventReader<E>` / `EventWriter<E>` SystemParam will revisit if richer Access semantics become necessary.

### S. (Round 3 — new) Hit the 5 µs dispatcher target at 50 systems via architectural change
**Rejected for Phase 9**: Reaching ≤ 5 µs at 50 systems requires a **batched apply queue** — instead of `apply(world)` being called once per completed system inside `apply_window_drain`, defer all per-system apply calls to a single batched flush at the end of the round. This would amortize the per-call overhead (~50 ns/system × 50 systems = 2.5 µs alone) but requires rewriting `apply` to be re-entrant against accumulated per-system state. Out of Phase 9 scope (§1.4); the relaxed 20 µs target accommodates the current per-system apply path. Reopen if profiling at 50-system workloads shows dispatcher dominates.

---

## §17 — Open questions (Round 2 — all resolved; Round 3 — additional resolutions)

### OQ-1: `has_deferred` detection — coarse vs. fine

**Decision (Round 2 — confirmed)**: **conservative for Phase 9**. Any system with a `CommandQueue` SystemParam is flagged `has_deferred = true`. Refine in Phase 10 when change-detection ticks expose per-system "did I emit anything" telemetry.

### OQ-2: Worker count auto-scaling

**Decision (Round 2 — confirmed)**: default to **logical count** (`num_cpus::get()`). Document that users can pass `num_cpus::get_physical()` explicitly via `ThreadPoolBuilder::num_threads(...)`. No reserved-for-main-thread carve-out (our dispatcher IS the calling thread).

### OQ-3: `find_ready` incremental update

**Decision (Round 2 — RESOLVED via §7.4; Round 3 O-NEW-2 — type refined)**: incremental `pred_remaining` lands in Phase 9 (not Phase 9.1). Justification: §1.2 dispatcher target binding; without the optimization, N=1024 dispatch exceeds 200 µs. Implementation cost: 2 KB per schedule + 20-line update at completion handler. Round 3 type downgrade to `Box<[u16]>` since dispatcher is the sole mutator (see §7.4.3 audit).

### OQ-4: Exclusive system run-time enforcement

**Decision (Round 2 — RESOLVED via §2.5 EXC2, §5.2)**: drop `is_exclusive` field from `SystemMeta`. Compute from `access.is_universal()`. `SystemBox::is_exclusive` remains as a build-time cache populated at construction. See Round 2 C9 resolution.

### OQ-5: `Query::par_iter` chunk-size for tiny archetypes

**Decision (Round 2 — RESOLVED via PAR9, §6.2)**: `MIN_ARCHETYPE_FOR_PARALLEL = 1024` threshold adopted in v1. Archetypes with fewer rows run inline on the calling thread (no `scope.spawn`). See Round 2 O2 resolution.

### OQ-6: Schedule build cost vs. system count

**Decision (Round 2 — confirmed)**: accept ~16 ms one-shot build cost at N=1024. Document in `Schedule::add_system` doc-comment. Hot-reload caching is a Phase 11 concern.

### Q1 (Round 2 — new): `Schedule::Out`

**Decision**: `Schedule::add_system<F, M> where F: IntoSystem<(), (), M>, F::System: System<Out = ()>` — `Out = ()` is required by the trait bound. The `add_system` doc-comment explicitly states "systems with non-`()` output must use `EcsMaster::run_system` outside the scheduler". The `Schedule` does not provide an output-collecting API in Phase 9.

### Q2 (Round 2 — new): Phase 8.5 cache reads from workers

**Decision (RESOLVED via §15.4)**: `OnceLock::get(&self)` is lock-free and `Sync`-safe for many readers. Under Phase 9 the cache is read on the dispatcher only (apply window). If future changes move reads to workers, the read path remains sound by construction. Doc-comment on `bundle_archetype_cache` updated.

### Q3 (Round 2 — new): `Arc<ThreadPool>` clone per frame

**Decision (RESOLVED)**: no per-frame clone. `Schedule` stores `pool: Arc<ThreadPool>`; `Schedule::run` uses `&*self.pool` to obtain `&ThreadPool` for `install`. Saves one `Arc::clone` (~5 ns) per frame; more importantly, signals architectural intent (pool is referenced, not duplicated).

### OQ-NEW-1 (Round 3): Event lane access in `Access`

**Decision (RESOLVED via §2.2 SCH7 note, §12.5, §16 R)**: events stay OUTSIDE the schedule's conflict graph. Per-lane single-writer discipline (EVT1 TLS) guarantees correctness without graph participation. Adding event_* fields to `Access` would break the 192 B / 3-cache-line invariant at `access.rs:61`. Phase 12 EventReader/EventWriter will revisit.

### OQ-NEW-2 (Round 3): Dispatcher target at 50 systems

**Decision (RESOLVED via §1.2 update, §10.5 cross-ref, §16 S)**: relaxed to **≤ 20 µs at 50 systems** (≤ 400 ns/sys); apply cost dominates the per-system contribution. The original 5 µs target assumed a batched apply path which Phase 9 does not implement (deferred). Boyko at 400 ns/sys still beats Bevy's ~470 ns/sys on raw throughput.

### OQ-NEW-3 (Round 3): Exclusive system apply path SAFETY

**Decision (RESOLVED via §2.5 EXC1 SAFETY block, §5.4 try_dispatch_ready exclusive branch, §16 W-NEW-5)**: exclusive system body uses `unsafe { cell.world_mut() }` to obtain `&mut EcsMaster` for `run_unsafe`; dispatcher reborrows from the same cell via the same call for `apply`. Exclusive body **must not** retain any cell-derived borrow past return — if stashed, the apply reborrow would alias (UB). EXC1 SAFETY block in §2.5 enumerates the invariants.

---

## §18 — Plan readiness checklist self-audit (Round 3)

### Plan structure
- [x] Goal stated in performance + functionality terms (§1.1, §1.2).
- [x] Target metrics concrete (§1.2 table; Round 3 reconciled with §10.5).
- [x] Every architectural decision has perf/cache/parallelism justification (§3 + deep dives).
- [x] Each alternative explicitly rejected with reason (§3 Q1-Q12, §16 A-S including Round 3 R, S).
- [x] Trade-offs honestly listed (§3 per-question, §17 open questions all resolved including OQ-NEW-1..3).
- [x] Round 3 changelog at top (§0) maps every Round 2 critic note to a resolution; Round 2 changelog preserved.

### Data structures
- [x] Each field typed and commented (§4.2, §5.2, §5.4, §6.1, §7.4.1).
- [x] `#[repr(...)]` specified where layout matters (`ThreadPool`, `ConflictGraph`, `ExecutorScratch`, `SystemMeta`).
- [x] Hot/cold split applied (`WorkerControl` hot/cold via `#[cfg(scheduler-trace)]`; `Schedule` cold conflict graph vs hot scratch).
- [x] Struct sizes known + justified (§11 table; Round 3 updated for pred_remaining `Box<[u16]>` = 2 KB at N=1024).
- [x] Padding against false sharing specified (§4.6, §11.2).

### API
- [x] Public API minimal (§12 covers only what users need).
- [x] No internal types leak (`SystemBox`, `ExecutorScratch`, `RawCommandQueue` all `pub(crate)`).
- [x] Lifetimes explicit on non-trivial signatures.
- [x] No `dyn Trait` in hot path (the one `Box<dyn System>` is schedule storage; vtable call once per system per frame; dominated by body cost — §SCH10).
- [x] Generics for specialization (FunctionSystem, ParQuery).

### Multithreading
- [x] Multithreading model explicit (§2, §5, §6, §9).
- [x] Atomic orderings explicit (§4.3, §4.5, §5.4.5.1, §7.4.3 throughout).
- [x] Synchronization points justified (§5.4.5 apply window; §8 sync analyzer).
- [x] Data partitioning described (§6.2 archetype chunking; §5.4 system-level scheduling).
- [x] Send/Sync consistent (§9.1 table; Round 2 corrected: Arena stays !Send + !Sync).
- [x] Deadlock-freedom proof (§4.6 sketch; Round 3 W-NEW-2 references only injector path).

### Correctness
- [x] Edge cases enumerated (PAR6, PAR7, SCH2 cycles, SCH13 conflict scan, nested par_iter via §4.6, panic during pending via §4.5.6, exclusive system body retention rule via EXC1).
- [x] Generation/version checks (Phase 8.5 cache survives; §15.4 confirmed).
- [x] Drop order discussed (§15.1 EcsMaster preserves Phase 8a C5).
- [x] Invariants for unsafe blocks stated (every `unsafe impl Send + Sync` carries SAFETY block; Round 3 EXC1 SAFETY formalized for exclusive path).

### Integration
- [x] Affected modules listed (§15.1; Round 3 added BitSet256 + ComponentMask helpers).
- [x] Changes to existing APIs explicit (§15.3: none breaking; Round 3 added is_all_set/set_all to additions).
- [x] Compatibility with `Arena` / `ComponentPool` / `UnitId` verified (§15.4, §15.5, §11.3 ComponentPool::grow audit).
- [x] Implementation plan broken into 24 Steps (§14, Round 2 added Step 7c + Step 24).

### Validation
- [x] Mandatory unit tests specified (§13.1 — Round 3 added `cell_minted_per_round_not_per_loop_iter`).
- [x] Property-based tests — N/A; concurrency is loom-tested, logic integration-tested.
- [x] Benchmarks specified (§13.5 + Round 2 `incremental_ready_set.rs`, `scope_vs_install.rs`).
- [x] Debug-assert invariants specified (§13.6 — Round 3 added u16 cap assertions per O-NEW-3).
- [x] Loom tests cover Round 2 critical paths (§13.4.1 four scenarios; §13.4.2 panic-with-pending; §13.4.4 apply-window gate).

---

## §19 — Changelog (for future critic iteration)

**v1 (initial draft):** plan submitted to architecture-critic.

**v2 (Round 2 critic fix):** 9 criticals + 8 warnings + 4 optionals + 3 open questions resolved. Substantial architectural revision (see §0 Round 2 changelog table).

**v3 (this revision, Round 3 critic fix):** 2 criticals + 5 warnings + 3 optionals + 3 open questions resolved. Targeted cleanup — no architectural rewrite. Summary of changes:
- `Access::is_universal()` now checks only the 4 existing bitmasks (C-NEW-1); event lane access stays outside the conflict graph per EVT1 per-lane discipline.
- §1.2 dispatcher target relaxed to ≤ 20 µs at 50 sys (C-NEW-2); §10.5 cross-referenced; batched apply path documented as deferred.
- §5.4 executor loop: removed the in-line "Wait — bug" Round 2 pseudocode; §5.4.5.1 is the sole canonical diagram and §5.4.5.2 the sole canonical drain function (W-NEW-1).
- §4.5.5 `join_workers_until_drained`: removed the misleading `pool.workers_owned_deques` optimistic code; only injector path remains. §4.6 deadlock-freedom proof updated to reference only the injector path (W-NEW-2).
- §5.5 `ScheduleBuilder::build`: pre-destructures `self` to avoid pseudocode borrow conflicts; lengths/names captured before the descriptors move (W-NEW-3).
- §5.4.5.1 gate carries a monotonicity comment (W-NEW-4).
- §5.4 exclusive system path uses explicit `unsafe { cell.world_mut() }` calls with EXC1 SAFETY block (W-NEW-5).
- §9.2 release-mode enforcement note added; references Step 24 CI `force_alloc_panic` config (O-NEW-1).
- `pred_remaining` type downgraded `Box<[AtomicU16]>` → `Box<[u16]>` (O-NEW-2); audit verified dispatcher-sole-mutator; saves LOCK prefix; minor performance gain reflected in §7.4.4 and §10.5.
- `ScheduleBuilder::build` adds `debug_assert!` that `pred_count` fits `u16` (O-NEW-3); §13.6 includes the assertion.
- BitSet256 and ComponentMask `is_all_set` / `set_all` helpers explicitly required in Step 7c (verified absent in current source); pseudocode supplied in §12.5.

---

**END OF PHASE 9 PLAN v3.**

---

Relevant absolute file paths for this Round 3 revision:

- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` (this file, to be overwritten by orchestrator with the v3 content above)
- `D:\claude\BoykoEngine\docs\PHASE-9-CRITIC-ROUND-2.md` (critic verdict driving Round 3 fixes)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\access.rs:45-126` (Access has only 4 bitmask fields; 192 B asserted at line 61 — drives C-NEW-1 resolution)
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set_256.rs` (no `is_all_set`/`set_all` today — Step 7c adds them)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_mask.rs` (no `is_all_set`/`set_all` today — Step 7c adds them)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs:64-170` (cell shape, `new_mutable` / `world_mut` SAFETY — used in EXC1 SAFETY block per W-NEW-5)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs:37-156` (Arena `!Send + !Sync`; `allocate_layout(&self)` — unchanged, still guarded by IN_SYSTEM_RUN)

**Implementation gate**: this is the final round of architect polish. Round 3 must be APPROVED by the critic before Step 1 starts. No new architectural decisions introduced — every change is a targeted resolution to a Round 2 critic finding.

---

**Note to orchestrator:** the Write tool is not available in this architect session. Save the v3 plan text above to `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` (overwriting the v2 content). Estimated line count after save: ~3300 lines (Round 2 was 3266; the additions are mostly the Round 3 changelog block at top, the BitSet256/ComponentMask helper pseudocode in §12.5, the OQ-NEW-1..3 entries in §17, and the §16 R/S rejection entries — all small textual additions; deletions of the buggy §5.4.5 diagram and the optimistic §4.5.5 code roughly offset the additions).