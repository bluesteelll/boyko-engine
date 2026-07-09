> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

Now I have a comprehensive understanding. Let me write the architecture-critic review.

---

# Architecture review: Phase 9 — Parallel Scheduler

## Verdict
[ ] APPROVED
[X] **CHANGES REQUESTED** — needs significant revision (10 criticals, 8 warnings, 4 optionals)

The plan is impressively thorough (2732 lines, all 12 architectural decisions justified, deep dives on three subsystems). However, **Round 1 surfaces several soundness and architectural issues** that must be fixed before implementation can start. The Send/Sync contract has a load-bearing factual error about `Arena`; the dispatcher deadlocks on a re-entrancy case; the par_iter contains a nested-install bug; and several SAFETY claims overstate the guarantees the foundations actually provide.

---

## 🔴 Critical (blockers — implementation must NOT start)

### C1. SEND2 contract for `Arena` contradicts the actual code
**Where**: §2.4 SEND2, §9.1 Arena row, §9.2 contract block.
**Problem**: The plan asserts `Arena` is `Send + Sync` "gated by the documented contract: allocations (`alloc_aligned`) require `&mut self`". This is **factually wrong**. In `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs` lines 90 and 106, both `allocate_layout(&self, ...)` and `allocate_from_free_blocks(&self, ...)` take `&self`. The `free_blocks: UnsafeCell<MemFreeBlockMaster>` is mutated through `&self`. The module-level doc says verbatim: "**`Arena` is intentionally single-threaded**: it contains an `UnsafeCell` (free-block tracker), and there is no `Send`/`Sync` `impl`." Concurrent calls from two threads "would be UB — protected against by the non-`Sync` marker."

If you make `Arena: Sync` without changing the allocator signature, **two worker threads can race inside `allocate_from_free_blocks` mutating the same `MemFreeBlockMaster`** — direct UB. The "no allocations during gameplay" line in CLAUDE.md is an aspiration, not an enforcement; the Schedule cannot prove that no system calls into an `Archetype::create_entity` path that triggers a new `ComponentPool::new` → `arena.allocate_layout`.

**Why critical**: Direct data race in `MemFreeBlockMaster::allocate_*`, which is `UnsafeCell`-protected against single-thread aliasing only. Miri and TSan will scream; in release the corruption is silent.
**What is needed**: Three options for the architect to pick from:
1. Re-architect `Arena` so allocations require `&mut self` and document that all in-frame `ComponentPool::new` must be funneled through an exclusive system (or the dispatcher between rounds). Audit every call site that today takes `&Arena`.
2. Wrap `MemFreeBlockMaster` in a lock-free allocator (compare_exchange free-list pop). Significant work; probably out of Phase 9 scope.
3. Hard invariant: "no allocation may occur inside `System::run_unsafe`; all archetype creation happens during `ScheduleBuilder::build` or in dedicated exclusive systems." Document and enforce with a `debug_assert!` in `arena.allocate_*` that records a thread-local "in-system" flag. Existing `Commands::spawn` cold path that creates new archetypes lives in `apply` (dispatcher-only) per §SCH7 — that part already works. But `EventBuffer` heap allocation, `RawCommandQueue::push`, and `ComponentPool` grow may need audit.

This must be resolved before SEND1 (which transitively requires SEND2) can land.

### C2. Worker threads cannot push to other workers' deques — `push_task` is broken
**Where**: §4.5 lines 690-712 (`push_task` helper).
**Problem**: The plan acknowledges the issue in a comment: "we don't have a direct handle to worker N's `Worker` here — that lives on its stack in `worker_main`. The clean solution is a per-worker `injector_local: Injector<TaskHandle>` that worker_main drains first." But then the plan only **mentions** this; nowhere in §4.2 (`ThreadPool` struct definition) is `injector_local` actually declared. The `Scope::spawn` code on line 637 calls `push_task(self.pool, task)`, which on the **dispatcher** thread pushes to the global injector and unparks one worker. But the same `Scope::spawn` is the one used by `par_iter::for_each` (§6.2) when called from **inside a system that runs on a worker** — at that point `push_task` correctly identifies "we're on a worker" but cannot deliver the task because the structure does not exist.

If you fall back to "push to global injector even when on a worker", you lose cache locality (the chunk you just produced is hot in your L1, but goes to a random sibling) AND you introduce mandatory contention on the single global injector.
**Why critical**: This is the load-bearing path for `par_iter` inside a Schedule (Steps 15 + 16). Without it, par_iter from a system body either deadlocks (Drop on Scope spins forever because workers can't get tasks) or silently degrades to single-injector contention.
**What is needed**: Decide and write down the actual structure. Two options:
1. Add per-worker `injector_local: [CachePadded<Injector<TaskHandle>>; MAX_WORKERS]` next to the existing global `injector`. Worker main loop drains its own local injector before the global. `push_task` on a worker pushes to `pool.injector_local[worker_id]`. Wake-up uses the same `unpark_one_idle` against the same idle bitset.
2. Use only the global injector; document the cache-locality loss and remove the "if on worker, push local" branch. Simpler. Probably fine for Phase 9.
Either way, the struct definition in §4.2 must match the code that uses it.

### C3. Schedule executor deadlocks when an apply path calls `par_iter`
**Where**: §5.4 lines 1010-1106 (`executor_main_loop`), interacting with §6.3 `current_pool()`.
**Problem**: The executor calls `pool.install(...)` at the start of `Schedule::run` (line 1000). Inside that install scope, the executor runs serially on the dispatcher thread. When a system completes and the executor calls `self.systems[idx].system.apply(world)` on line 1031 (`SCH7` — apply on dispatcher), nothing stops the apply path from invoking a query that calls `par_iter().for_each(...)`. The par_iter calls `pool.install` **again** inside the outer install (line 1351 of §6.2: `pool.install(|scope| { ... })`). 

Now we have a **nested install on the same pool from the dispatcher thread**. The plan's PAR6 ("no nested fork-join") is enforced via the `IN_FORK_JOIN` thread-local, but the test runs in the par_iter wrapper, not in `ThreadPool::install` itself. So nested install is *allowed* but the dispatcher is now blocked inside the inner install's `Scope::Drop`, which spins/parks waiting for inner tasks. Workers can pick the inner tasks up — fine. But if the apply path is, say, a `Commands` that called `world.send_event(...)` and the dispatcher was preparing to call the next system's apply, then no progress can be made by the dispatcher until the inner par_iter completes. That can be acceptable... **but the same issue arises inside `executor_main_loop`'s `park_timeout` on line 1103**: if the dispatcher parks at the same time a worker is about to push a completion and an apply ends up calling `pool.install` to par_iter, the second install's `Scope::Drop` parks the dispatcher *again* with a different waker (`std::thread::current()` of the dispatcher), and the worker's `dispatcher_thread.unpark()` (line 1145) targets the outer `park_timeout` thread, which is the same thread but the unpark is one-shot, sticky — so far so good. The bigger issue is:

**The `Schedule::run` outer `pool.install` already increments `active_scopes` and creates a `Scope`. Workers running outer-scope tasks see them via `parent_scope`. If a worker spawns an inner-scope task (because the worker IS running a system body that does par_iter), the inner Scope's `pending` counter is independent. So far OK. BUT: there's no test in the plan and no analysis of "exclusive system blocked by inner par_iter".** Specifically:
- Dispatcher executes a concurrent system → wraps in install + scope.spawn → worker executes → worker calls par_iter (inner install on same pool).
- Inner install creates inner Scope. Worker spawns inner tasks via inner scope.
- Inner tasks need workers; **the worker that called par_iter is itself blocked in the inner scope.Drop spin/park**.
- If the inner par_iter needs N parallel chunks and only N-1 workers are free (the calling worker is parked in scope.Drop), progress depends on the calling worker noticing the unpark from inner completion. But that worker is parked inside `Scope::Drop` line 650 (`std::thread::park()`). The unparker is on line 629 of the inner task: `(*shared_ptr).waker.unpark();` — but `waker` was set at install entry to `std::thread::current()`, which IS the calling worker. OK, that works.

**But re-reading §4.5 line 674: `waker: std::thread::current()`** captures the thread that called `install`. If that's a *worker* (because the worker called `par_iter`), the worker is now parked on its own waker. When the last inner task completes and unparks the worker, the worker resumes — fine for that case. But during the park, **the worker is unavailable to steal other work**. If the inner par_iter has N chunks and the pool has N workers total, the worker calling par_iter parks, so only N-1 workers do the inner work. That's only a perf hit, not a deadlock.

**The actual deadlock**: when a closure spawned by the **outer** scope calls `par_iter`, which calls `pool.install`, and the inner install's `f(&scope)` immediately returns (synchronous body), then `Scope::Drop` waits for `pending == 0`. Now if **all** workers happen to be blocked inside their own outer-scope tasks each waiting for inner par_iter to finish (e.g., 16 workers, each running a system, each system calls par_iter at the same point), **the inner tasks have no workers to run on**. Outer tasks are blocked on Scope::Drop. Pool deadlocks.

**Why critical**: A real-world scheduler hits this when ≥ workers concurrent systems all use par_iter. Bevy fixes this by `block_on(...)` cooperatively yielding (their `TaskPool` integrates with `async-executor`). PAR6 in the plan (no nested fork-join) is the right rule, but it is enforced inside par_iter, not at the boundary system-body↔par_iter. The plan never analyzes "what if 16 concurrent systems each call par_iter?"
**What is needed**: One of:
1. PAR6 should be: "a system body that calls par_iter must declare itself exclusive". Forces such systems to run alone — no deadlock possible. Probably too restrictive.
2. PAR6 strengthened: "a system body must not call par_iter; par_iter is only allowed from the dispatcher's exclusive context (e.g., a special `ExclusiveDispatcherTask`)." Drops a major feature.
3. Allow nested but require the worker calling par_iter to **steal work while waiting** instead of parking in Scope::Drop. This is the rayon pattern (`scope` workers steal between joins). Requires the worker to know it's mid-task and continue stealing — a significant complication.
4. Cap concurrent par_iter invocations to `worker_count - 1` via a semaphore. Slow workers but guaranteed live.

Architect to decide; in any case, the plan must include the analysis and the resolution. Currently it has neither.

### C4. SEND3 `UnsafeEcsCell: Send + Sync` does not survive Tree Borrows analysis as stated
**Where**: §2.4 SEND3; §5.4 line 1085 (`let world_cell_copy = world_cell; scope.spawn(move |_| ... )`).
**Problem**: The plan says `UnsafeEcsCell` becomes `Send + Sync` because it holds only a raw pointer + PhantomData. Technically the unsafe impl compiles. But the more interesting question is **what each thread sees as its fresh provenance tag** when it dereferences the copy. Tree Borrows tracks per-thread borrow stacks via `release_thread` / `acquire_thread` syscalls; `*mut EcsMaster` is fine to clone, but **each `world_mut()` and `world()` call on different threads creates fresh tags rooted at the original allocation**. If the dispatcher (thread A) calls `world_cell.world_mut()` and a worker (thread B) calls `world_cell_copy.world()` overlapping in time, both create `&[mut] EcsMaster` references to overlapping byte ranges — UB even with Tree Borrows' liberal model, because the byte ranges are not disjoint.

The plan assumes the scheduler will never call both, but the executor flow shows otherwise: `apply` on the dispatcher is called (line 1031) **while workers may still be running concurrent systems that share `world_cell_copy` and access disjoint components**. The dispatcher's `apply` takes `&mut world` from the outer borrow, while workers hold `UnsafeEcsCell` copies pointing to the same `EcsMaster`. Worker calls `world_cell.world()` → constructs `&EcsMaster` → walks `archetype_master` → reads bytes. Dispatcher's `apply` constructs `&mut EcsMaster` (line 1031 is `self.systems[idx.0 as usize].system.apply(world);` where `world: &mut EcsMaster`). Two threads, one with `&EcsMaster`, one with `&mut EcsMaster`, simultaneously — UB at the language level, irrespective of whether the byte accesses overlap.

Compare to §SCH7 "A system's `apply` is **NEVER** called concurrently with another system": the plan says this is the contract. But the executor loop (line 1022) is:
```
while not all completed {
    drain completion_queue → apply  // dispatcher
    find_ready
    scope.spawn → workers run        // workers
    park_timeout
}
```
There is **no barrier between "workers are still running concurrent systems" and "dispatcher does apply"**. The completion drain is non-blocking; the only thing that prevents the apply from racing with running workers is that systems that completed-and-pushed-to-completion_queue are out, but **other systems are still running**.

The plan's mental model is "apply runs in a quiescent window when no system is in flight." Code says otherwise.
**Why critical**: This is the central correctness issue for the scheduler. Either the contract or the executor design must change.
**What is needed**:
1. Modify executor to drain completion_queue only when `running.count_ones() == 0`, OR
2. Distinguish "apply" (which needs `&mut EcsMaster`) from completion tracking (which only updates bitsets); accumulate pending applies in a queue and execute them in a barrier when running drains, OR
3. Have apply work through the cell too (but then apply cannot take `&mut EcsMaster` — major rework of Phase 8d `Commands::apply`).

Whatever the choice, **plan must include a clear "apply window" definition with an explicit barrier**, and SCH7 must match what the code actually does.

### C5. `EventDispatcher::send` requires a thread_index — Phase 9 has no policy
**Where**: §2.4 SEND4 ("`EventDispatcher` becomes `Send + Sync`. It is internally lock-free for `send` — per-thread lanes"); also §9.1 row.
**Problem**: `pub fn send<E: Event>(&self, thread_index: u32, event: E)` requires the caller to specify which **per-thread lane** to push to. The EventDispatcher's per-thread design (one cache-line `ThreadLaneWriter` per index) is correct ONLY if every thread always writes to its own index. In Phase 8a/c, `thread_index` was hardcoded to 0 (single-threaded). Under Phase 9:
- The pool has `worker_count` workers each with index `0..worker_count`.
- The dispatcher is the calling thread, not a worker.
- The user's system body calls `commands.send_event(...)` somewhere — what `thread_index` does it pass?
- If the system body knows its worker_id, it must read TLS — adds a TLS load per send.
- Worse: a system spawned via `Scope::spawn` runs on **whichever worker steals it**, not a fixed one. The user cannot statically pick a thread_index.
- If two workers both send to the same lane (because the user picked thread_index = 0 always), the `ThreadLaneWriter` doc says only "the worker pinned to the corresponding `thread_index` writes" — multi-worker writes to the same lane violate the lane's exclusivity invariant.

This is not a corner case — `Commands::send_event` / direct `world.send_event` are common APIs in Bevy-style ECS, and Phase 9 makes parallelism the default. Plan doesn't even mention this.
**Why critical**: Either the EventDispatcher needs a different write API, or the scheduler needs to provide a "current worker index" service, or events become broken under parallelism.
**What is needed**: Choose one:
1. Add `current_worker_id() -> u32` TLS lookup (set on worker_main entry to the worker index, 0 for the dispatcher), and document that `send_event` reads from TLS internally. Drop the explicit `thread_index` from the user-facing API; keep it internal.
2. Forbid `send_event` from non-dispatcher contexts; force users to enqueue an event via `Commands` (which is per-system queue, single-writer); the dispatcher's apply flushes them onto the lane 0 (dispatcher's lane). Slower but simpler.
3. EventDispatcher uses lock-free MPSC instead of per-thread SPSC for the writer side (significant rework).

The plan must address this before EventDispatcher gets `unsafe impl Sync`.

### C6. The `transmute` lifetime erasure on `Box<dyn FnOnce + 'scope> → Box<dyn FnOnce + 'static>` is too aggressive
**Where**: §4.5 lines 612-632 (`Scope::spawn` body).
**Problem**: The code does:
```rust
let body: Box<dyn FnOnce() + Send + 'static> = unsafe {
    std::mem::transmute(Box::new(move || { ... }) as Box<dyn FnOnce() + Send + '_>)
};
```
This pattern *is* used by rayon (https://github.com/rayon-rs/rayon/blob/main/rayon-core/src/scope/mod.rs), but rayon's invariant is "Scope::Drop blocks ONLY THROUGH a `JoinHandle`-equivalent that prevents the FnOnce from outliving 'scope EVEN ON PANIC."

The plan's `Scope::Drop` (§4.5 lines 641-659):
```rust
loop {
    if self.shared.pending.load(Ordering::Acquire) == 0 { break; }
    if backoff.is_completed() { std::thread::park(); }
    else { backoff.snooze(); }
}
```
**There is no panic-safety here.** If the dispatcher thread panics while inside the `install` closure's body (between `let result = f(&scope);` line 682 and `drop(scope);` line 683), the panic unwinds through `drop(scope)`, which calls `Scope::Drop`, which **spins forever waiting for pending=0**. The unwinding thread is the dispatcher (the one that called `install`), and the spawned tasks are running on workers. Workers' `(*shared_ptr).pending.fetch_sub` will eventually drain it, **unless one of the workers panicked too** and was suppressed by `catch_unwind` (good — pending still decremented). But what if a worker is stuck in an infinite loop in user code? Drop spins forever → unwinding hangs → deadlock during panic propagation.

More subtly: the SAFETY claim is "the scope's Drop blocks until pending == 0, so no task outlives 'scope." This is true ONLY IF `Drop` is actually called. Rust's `Drop` runs during stack unwinding. If a worker panics inside the task body (line 617 `catch_unwind`), the panic is captured (good). If the **dispatcher thread is aborted** (`std::process::abort()`, OOM, SIGKILL), `Drop` never runs and tasks **DO** outlive 'scope. The `'scope` borrow was promoted to `'static`; the workers continue accessing freed stack memory.

This is rayon's same edge case; rayon documents it but boyko's plan claims unconditional safety.
**Why critical**: A panic inside the install closure body (between scope creation and scope drop) interacting with a long-running worker task = hang. Plus the abort-during-spawned-task UB window. Tree Borrows / Stacked Borrows can detect the latter under Miri.
**What is needed**:
1. Add explicit treatment of "Drop runs during unwinding" — confirm correct behavior under nested panics.
2. Document the abort/SIGKILL caveat in SAFETY block.
3. Consider rayon's `block_on` pattern: while waiting in Drop, the calling thread itself **steals work** instead of parking. This shortens the wait and avoids the "Drop spins while workers idle" pathology.
4. Add a loom test for `pool.install panics inside f after some tasks spawned, before all complete` (currently §13.4 doesn't cover this).

### C7. `unpark_one_idle` has a race window that may lose wakeups
**Where**: §4.3 lines 480-498 (`unpark_one_idle`) interacting with §4.3 lines 442-461 (worker park sequence).
**Problem**: The worker's park sequence is:
1. Spin/yield budget exhausted (line 444 `is_completed()`).
2. `mark_idle(&pool.idle, worker_id)` (line 447).
3. Re-poll local + injector + steal (lines 448-450).
4. Check `shutdown` (line 453).
5. `std::thread::park()` (line 457).
6. `unmark_idle(&pool.idle, worker_id)` (line 458).

The pusher (`push_task`) does (line 710): `injector.push(task); unpark_one_idle(pool);`

**Race A**: Pusher pushes BEFORE step 2 mark_idle but AFTER step 1's last poll. Worker enters step 2 (mark_idle, fetch_or Release), steps 3-4 (re-polls the injector — sees the pushed task), step 5 NOT taken (returns to outer loop via `break`). Worker unmarks idle at step 6. Looks fine.

**Race B**: Pusher pushes BETWEEN steps 4 and 5. Step 4 sees `shutdown=false`, then immediately push happens, then `unpark_one_idle` reads the idle bitset, sees this worker's bit (set in step 2), clears it (CAS), calls `worker.thread.unpark()`. The worker then enters `std::thread::park()` on step 5; std's park is "sticky" — if an unpark was issued before park returns immediately. **OK so far.**

**Race C** — the actual bug: Pusher push lands BEFORE step 2 mark_idle but worker is mid-step-1 backoff snooze. Pusher's unpark_one_idle reads `idle = 0` (worker hasn't set its bit yet); returns false. **Wakeup lost.** Worker enters step 2 mark_idle, steps 3-4 re-poll — IF the injector push from the pusher happened-before the worker's re-poll, the re-poll sees the task. But the pusher's push is `injector.push(task)` (crossbeam internal; let's assume Release on tail). The worker's re-poll in step 3 is `pool.injector.steal_batch_and_pop(&deque)` (Acquire). Acquire/Release synchronizes correctly → worker sees the task → does not park. **Wakeup recovered.**

So the algorithm relies on the worker's re-poll AFTER mark_idle to catch concurrent pushes. Looking again, step 3 IS the re-poll. **OK, the protocol works correctly.**

**But there's a SUBTLER issue at step 6 (`unmark_idle` after park returns)**: If a second pusher arrives between step 5's park return and step 6's `unmark_idle(... Release)`, the second pusher reads `idle` with our bit STILL SET. It picks us as wake target, calls `unpark`. We're already woken; our next park call (in the next outer loop iter when we exhaust spin again) will return immediately due to std's sticky unpark. **This wastes one wake but doesn't lose work.** OK.

**The real race**: The `mark_idle` in step 2 uses `fetch_or(Release)`. `Release` publishes prior writes to anyone reading with Acquire — but nothing in our code prior to mark_idle needs publishing. The pairing is `unpark_one_idle::load(Acquire)` reading our `Release` `mark_idle` — that's correct. **But** what about the worker's step 3 re-poll (`injector.steal_batch_and_pop`)? It needs to observe a Push that happened-before. The Push's Release in crossbeam-deque's internal head/tail synchronizes with `steal_batch_and_pop`'s Acquire — yes, this works. **OK.**

After careful analysis, the algorithm appears correct. **But** the plan doesn't include the loom test that validates Race C: §13.4 has `loom_unpark_one_idle_races_park` but the scenario described is the simple race, not the "push lands before mark_idle" → "worker re-polls after mark_idle" double-check protocol.
**Why critical**: This isn't a UB bug; it's a documentation/test bug. But Round 1 critic flags it because losing a wakeup in a production thread pool is a hard-to-reproduce hang that loom can prevent.
**What is needed**: Expand §13.4's loom test plan: list four scenarios explicitly (Race A/B/C/D) and assert each runs without losing a wake-up under all interleavings. Also document the "re-poll after mark_idle is load-bearing" rationale in the worker_main comment.

### C8. `ConflictGraph::build` is O(N²) on `Access::conflicts_with` calls — at N=1024 the cost is severely underestimated
**Where**: §7.1 build phase ("Cost: O(N²/2 × access_conflict_cost) = O(N²/2 × ~30 ns) = ~15 ms at N=1024.")
**Problem**: `Access::conflicts_with` (existing) walks 6 bitmasks. At 192 B per Access, two systems → 384 B compared. Six bitmask intersections at ~5 ns each = 30 ns sounds right. But there are **two** other costs the plan forgets:
1. Vec allocations: `vec![FixedBitSet::with_capacity(n); n]` allocates N FixedBitSets each holding a `Vec<u32>` of length `ceil(N/32)`. At N=1024 that's 1024 Vecs × 32 u32s = 32 KB of small allocations (plus per-Vec heap metadata). ~50 ns per Vec alloc × 1024 = 50 µs. Not 15 ms, but the plan omits it.
2. `Box<[Box<[SystemIndex]>]>` for `predecessors` / `successors`: also N small allocations. ~25 µs each.
3. `compute_depths` is "O(N × max_depth)" but the inner loop walks `predecessors[i].iter()` for each i, AND does it until fixpoint. With N=1024 and max_depth = O(N), the worst case is **O(N² × avg_predecessors)**, easily 100 ms at N=1024 with deep chains. The plan says "≤ 1 ms" — wrong by 2 orders of magnitude.

The §1.2 target "Schedule build (50 systems, 200 components, no cycles) ≤ 50 µs" is plausible. The §7.1 target "16 ms at N=1024" is optimistic. The §10.5 dispatcher estimate "**1.07 ms per frame**" already EXCEEDS the §1.2 target of "≤ 200 µs per frame at N=1024" by 5×, and the plan acknowledges this in OQ-3 by deferring the fix.

**The plan acknowledges this exceeds the target but ships anyway** (OQ-3 "ship Phase 9 with the O(N²) scan and bench it; if benches show > 200 µs at N=1024 the optimisation lands as a Phase 9.1 patch"). This is the "deferred optimization" anti-pattern flagged in CLAUDE.md (§4 anti-pattern wording).
**Why critical**: A scheduler that fails its own performance target out the gate is a structural problem. The §1.2 table is binding (acceptance gate). If we cannot hit 200 µs at N=1024, either the cap moves to 256 (and that becomes the only supported regime), or the find_ready algorithm needs the predecessors-completed counter NOW, not in 9.1.
**What is needed**: Pick a number you can hit and defend it. Either:
1. Lower the supported cap to N=256 (where the math works: 134 µs per frame).
2. Implement the per-system `AtomicU16 predecessors_completed_count` from OQ-3 in Phase 9 (it adds 2 KB per schedule + correct atomic ordering for the increment-on-completion path). The plan currently spends 5+ pages on the dispatcher; this optimization is one new field and a 20-line update at the completion handler. There is no good reason to defer it.

### C9. `is_exclusive` flag duplicates `Access` universality (OQ-4 already flags this, but plan doesn't resolve)
**Where**: §2.5 EXC2 ("SystemMeta::is_exclusive: bool, new field; size grows from 224 B to 232 B aligned to 240 B"); §17 OQ-4.
**Problem**: The plan defines `ExclusiveFunctionSystem` as a system whose `Access` is the universal set. The executor reads `is_exclusive` (cached in `SystemBox`) to gate exclusion. This is **two sources of truth** for the same property — UB-free but a maintenance hazard. OQ-4 proposes dropping `is_exclusive` and computing it from `access.is_universal()`. **The plan should adopt this proposal in v1, not defer it to a future "decision proposal".** §13.6 debug-assert invariants don't include "is_exclusive == access.is_universal()", so the inconsistency could go undetected.

Furthermore: growing `SystemMeta` from 224 to 240 B (an additional cache line on the 192-byte-aligned SystemMeta because Access is 192 B and the existing fields fill the rest) is a real cost on the hot path. Every `system.access()` returns a pointer to a SystemMeta field; the offset and total size affect cache packing of `Vec<SystemBox>`.
**Why critical**: Direct contradiction between OQ-4's proposed resolution and §2.5's "this is the design". A plan must contain decisions, not a smorgasbord (CLAUDE.md anti-pattern wording).
**What is needed**: Adopt OQ-4's proposal in §2.5 — drop `is_exclusive`. Compute exclusivity from `access.is_universal()`. Update `SystemBox` so `is_exclusive: bool` is a build-time cache (still valid, but populated from access universality, not from a redundant flag). Update §11 SystemMeta size to remain 224 B.

### C10. `Scope::Drop` parks the install caller, but `waker = std::thread::current()` is captured INSIDE `install`, not when `Scope::Drop` runs
**Where**: §4.5 lines 671-687 (`ThreadPool::install`), specifically line 674 (`waker: std::thread::current()`).
**Problem**: The `ScopeShared::waker` is captured at `install` entry. `install` runs the user closure `f(&scope)` (line 682), then `drop(scope)` (line 683). Drop calls Scope::Drop which spins/parks on **its own waker** — the calling thread of `install`. The `pending.fetch_sub` path on workers (line 627) calls `(*shared_ptr).waker.unpark()` (line 629) when `prev == 1` — unparking the install caller. That's correct.

But there's a subtle issue: `install` is called from the dispatcher (`Schedule::run`). In the executor loop (§5.4 line 1000) the dispatcher does `pool.install(|scope| { self.executor_main_loop(world, scope); })`. The executor_main_loop spawns tasks via `scope.spawn` (line 1086). Workers `unpark` the dispatcher via `dispatcher_thread.unpark()` (line 1145) directly — bypassing the waker mechanism. AND the worker also decrements `pending.fetch_sub` which **also** calls `waker.unpark()`. **Two unparks on the same thread**. std's `unpark` is one-shot — the second unpark just sets the flag, the next `park()` returns immediately.

Now consider: dispatcher is in `executor_main_loop` (line 1102 `park_timeout(100µs)`). A worker pushes completion AND calls `dispatcher_thread.unpark()`. Park returns. Dispatcher does another iteration. **Meanwhile** another worker is finishing the last spawned task, decrements pending to 0, calls `waker.unpark()`. The unpark is consumed by an earlier `park_timeout` call but not yet the next one. Now `executor_main_loop` returns (all systems completed). `pool.install` calls `drop(scope)` which enters the `loop` (line 645). First iteration: `pending.load(Acquire) == 0` → break. No park needed. **Fine.**

But what if `executor_main_loop` returns **while a worker is still in flight on a task** (it shouldn't, but in a buggy executor)? `pending` > 0; Drop enters `std::thread::park()` (line 650). Worker eventually completes, decrements to 0, calls `waker.unpark()`. Dispatcher resumes. **Fine.**

**The actual problem**: §5.4 line 1086 `scope.spawn(move |_| { ... })` — the task body does NOT call `pending.fetch_sub` because the task is run BY THE POOL'S OWN BODY LOGIC, not by the user's `f`. Wait — looking at §4.5 line 612, the `Box<dyn FnOnce>` body wraps the user's body in the catch_unwind + pending.fetch_sub wrapping. So when the worker runs the boxed body (line 612-631), it DOES call `pending.fetch_sub`. **OK that's covered.**

**However**: There's still a clearer issue. **The waker is `Thread`, which is `Clone` (it's an `Arc<Inner>`)**. The plan correctly clones it in §5.4 line 1141 (`dispatcher_thread.clone()`). But `ScopeShared::waker: Thread` (line 594) is **not in a `Box` or `Mutex`**; it's accessed via `(*shared_ptr).waker.unpark()` (line 629). Reading a `Thread` field through a raw pointer requires the field to be initialized and not concurrently mutated. It's initialized at install (line 674) and never mutated — fine. But `unpark` takes `&self`; the borrow checker sees `&(*shared_ptr).waker`; that's a `&Thread` to the field through the raw pointer. **The struct is heap-allocated** (`Box<ScopeShared>` line 671) so the address is stable. **OK that works.**

After this analysis, no concrete bug. **What IS missing**: the plan doesn't explain why `waker: Thread` doesn't need atomicity or a Mutex despite being read from many worker threads via `unpark`. The answer is "`Thread` is internally `Arc<ThreadInner>`, so cloning is cheap and accessing through a `&Thread` from many threads is safe because `Thread: Sync` internally." The plan should document this rather than assume the reader knows.

Demoting from C10 to a W (warning) — see below. **Removing from criticals**.

---

## 🟡 Important (must be resolved before APPROVE)

### W1. Worker → dispatcher unpark path duplicates `dispatcher_thread.unpark()` and `ScopeShared::waker.unpark()`
**Where**: §5.4 line 1145 vs. §4.5 line 629.
**Problem**: When a worker completes a task spawned via `scope.spawn`, the boxed body calls `pending.fetch_sub` and possibly `waker.unpark()` (the scope's waker, which is the dispatcher in `Schedule::run`'s install). Additionally, the task body (the system-running closure in §5.4 line 1142) calls `dispatcher_thread.unpark()` explicitly. So every worker completion potentially unparks the dispatcher **twice**.

Functionally correct (unparks are idempotent on the receiver side), but wastes one syscall per task and obscures the protocol. Worse, if the user reads `Schedule::run` source code, they cannot tell which unpark is "the real one" — invites accidental removal during refactoring.
**Solution options**:
1. Remove the explicit `dispatcher_thread.unpark()` in §5.4 line 1145; rely on `ScopeShared::waker` for all wakeups. But then the dispatcher needs to know which thread is the scope's waker — which is itself the dispatcher. Works but requires the dispatcher to grab a `Thread` handle to itself and pass it through some channel. Awkward.
2. Drop `ScopeShared::waker.unpark()` from `pending.fetch_sub`'s last-task path; rely on the explicit per-task unpark from the executor. Doesn't generalize beyond Schedule (other `install` users wouldn't get any wakeup).
3. Document the redundancy: "wake-up is doubly-delivered; the second is a no-op."

Option 3 is fine but the plan must include it.

### W2. `CommandQueue: !Sync` invariant + per-system queue assumption breaks if same system runs concurrently
**Where**: §2.4 SEND7 ("`CommandQueue` is already `Send + !Sync` (Phase 8d CQ-SEND1). One queue per system; queues never cross system boundaries.")
**Problem**: SEND7 asserts "single-writer access" because "Phase 9 single-system-per-thread assignment automatically gives single-writer access — see §7.5." But §7.5 is about the bitset scan, not about per-system queue ownership.

A `FunctionSystem<F, M>` stores `Option<P::State>` (Phase 8c). `CommandQueue` lives inside that state. Each system has exactly one state — so single-writer holds *as long as the same FunctionSystem is not dispatched twice concurrently*. The scheduler ensures each system runs once per frame (SCH6), so this holds.

**But**: `par_iter` from inside a system body spawns N tasks running the user's closure. If the closure captures `&mut Commands`, the N tasks all push to the same CommandQueue. Aliasing UB. The plan never analyzes this.

Looking at §6.1 line 1311 (`Body: Fn(D::Item<'_>) + Send + Sync`) — closure is `Fn`, not `FnMut`, so it cannot capture `&mut`. Cannot capture `Commands` either (Commands::add takes `&mut self`). So users **cannot** push commands from inside par_iter. **OK, the design accidentally protects against this** — but the plan should make it an explicit invariant: "Commands::add inside par_iter is type-system-rejected because Commands is not Sync." Document this for users who might wonder.

Additionally: §2.4 SEND7's wording "one queue per system; queues never cross system boundaries" — that holds, but what about systems that run on different threads in different frames? FunctionSystem stores P::State in `Option<P::State>`. If the system runs on worker A in frame 1 (CommandQueue mutated there) and worker B in frame 2, the state's CommandQueue moves from thread A to thread B — that's `Send`, fine. **OK.**
**Solution options**:
1. Add an invariant CQ-SEND2: "CommandQueue is read/written only by one thread at a time; the system scheduler enforces this by running each system serially; par_iter type-system-rejects Commands as a captured variable because Commands is !Sync."
2. Add a test that captures `Commands` in a par_iter closure to confirm it doesn't compile (compile_fail test).

### W3. `EntityMaster::has_entity`, `get_entity` on `&self` will race with structural mutation on another thread
**Where**: §2.4 SEND5 ("`EntityMaster` becomes `Send + Sync`. Hot paths are `&self` reads ... `&mut self` paths are `create_entity` / `delete_entity` and the Phase 9 scheduler **never** runs two structural-mutation systems concurrently (they conflict on the implicit 'Spawn/Despawn' access — see §8 sync-point analyzer).")
**Problem**: The "implicit Spawn/Despawn access" is mentioned but **not defined anywhere in §8**. §8 talks about deferred commands, not about declaring structural-mutation access. How does a system declare "I spawn entities" so the conflict graph knows to serialize it with reads? Phase 8d's `Commands::spawn` enqueues the spawn; the apply runs on the dispatcher (SCH7) — fine. But what about a system that takes `&mut EcsMaster` directly (an exclusive system)? Its Access is universal — exclusive — fine.

The real worry: `EntityMaster::entities_inland: Vec<EntityInland>` (line 34 of entity_master.rs). If the `&mut self` path `register_entity_with_ptr` (called from `EcsMaster::create_entity` on the dispatcher during a flush) reallocates the Vec (capacity grow), all `&self`-readers on other worker threads now hold pointers to freed memory.

Phase 9 says structural mutations happen during apply (dispatcher only, no workers running). But SCH7 just says "apply is never concurrent with another system"; it doesn't say "Vec reallocations are barriered". The drain protocol in executor_main_loop (line 1024-1031) calls apply on EACH completion immediately, but other workers are still running concurrent systems and may be reading `entities_inland` via `world_cell.world().entity_master().get_entity(id)`.

If `apply` triggers a `Commands::spawn` flush → `create_entity` → `EntityMaster::register_entity_with_ptr` → `entities_inland.push(...)` → potential reallocation → workers reading freed Vec.
**Solution options**:
1. Pre-allocate `entities_inland` to a max size at world construction (the Phase 8 plans hinted at this; verify and document).
2. Move all structural mutation into a barrier (no workers running during any apply that may mutate Vecs).
3. Use a stable-address data structure (`SparseSlotMap` already exists in `boyko_utils`).
4. Per C4: define an explicit "apply window" where no workers run, and document that ALL `apply` calls happen in that window.

This is **same problem as C4** but specific to EntityMaster. Resolve together.

### W4. `Schedule` and `ScheduleBuilder` ownership of `pool: Arc<ThreadPool>` is duplicated
**Where**: §5.4 (`Schedule.pool`), §5.5 (`ScheduleBuilder` does not have a pool field but §15.3 ScheduleBuilder takes `Arc<ThreadPool>`).
**Problem**: §12.2 `ScheduleBuilder::new(pool: Arc<ThreadPool>)` — builder takes pool. §5.4 `Schedule.pool: Arc<ThreadPool>` — schedule stores it. Where does the builder's pool live? Plan doesn't say. Need a field on ScheduleBuilder, or the builder takes pool at `build()` time. The §5.5 struct definition (lines 1154-1158) doesn't include the pool. **Inconsistency.**
**Solution**: Pick one. Either `ScheduleBuilder::new(pool: Arc<ThreadPool>)` stores it in a field, or `ScheduleBuilder::build(self, pool: Arc<ThreadPool>, world: &mut EcsMaster) -> Schedule` takes it at build time. Update both §5.5 struct and §12.2 signature to match.

### W5. `IntoSystem` blanket impl for `fn(&mut EcsMaster) -> Out` conflicts with existing tuple-based IntoSystem blanket
**Where**: §3 Q9 ("The `IntoSystem` blanket-impl dispatch is the cleanest spot — Phase 8c's existing `IntoSystem<In, Out, M>` framework already supports it, we just add one more impl.")
**Problem**: Phase 8c's `IntoSystem` is implemented for `F: SystemParamFunction<M>` via a marker `M`. Adding a NEW blanket impl for `F: FnMut(&mut EcsMaster) -> Out` requires a marker that **doesn't overlap** with the existing one. The existing tuple impls use markers like `(P1,)`, `(P1, P2)`, ..., and an empty `()` marker for the no-params case. The exclusive impl needs a different marker, e.g., `ExclusiveMarker`. Plan doesn't specify this; just claims "one more impl" — but Rust's coherence rules will reject overlapping blanket impls without a distinguishing marker type.

Look at the existing `function_system_impls.rs` (the new file in the working tree, recently added per Phase 8c). Verify the marker structure before claiming "just add one more impl."
**Solution options**:
1. Define a fresh marker type (e.g., `ExclusiveSystemMarker`) and document the IntoSystem variant. Show the impl signature in §3 Q9.
2. Or use the `Exclusive<F>` newtype wrapper (Q9 alternative (a)) — has been rejected but might be revisited if the IntoSystem coherence is messy.

### W6. ParQuery's `for_each` calls `pool.install` inside an existing install scope — design choice unclear
**Where**: §6.2 line 1351 (`pool.install(|scope| { ... })`).
**Problem**: When `par_iter` is called from a system body, the calling thread is **either** the dispatcher (if the system is exclusive) or a worker. Either way, the thread is already inside the outer `pool.install` scope. The par_iter then calls `pool.install` **again**.

What does a nested `install` actually do?
- Increments `active_scopes` again (line 669).
- Creates a new `ScopeShared` (line 671) with `waker: std::thread::current()` — this captures the CURRENT thread (worker, not dispatcher).
- Runs `f(&scope)` synchronously.
- Drops the inner scope → spins/parks on `waker` (the worker).
- Workers running inner-scope tasks call `pending.fetch_sub` → unpark worker.

So the calling worker BLOCKS until inner par_iter completes. Workers complete inner tasks. **The calling worker is unavailable for stealing** during this period (it's parked in inner Drop). If `par_iter_chunk_count > worker_count - 1`, the chunks can't all run in parallel — calling worker is dead weight.

The plan doesn't explain whether par_iter "doubles" the install or uses a different mechanism. Compare to rayon: rayon's `scope.spawn` inside another scope reuses the outer pool naturally; rayon's `Pool::install` always uses the existing pool with the calling thread as a worker (rayon installs a TLS pool reference but doesn't create a new scope structure per install call).

**Recommendation**: `par_iter` should create a `Scope` (lightweight; new `ScopeShared`) but NOT call `pool.install` (no new active_scopes increment, no nested install structure). Instead, par_iter creates a `Scope<'a>` directly, spawns tasks, drops the scope. Drop blocks the calling thread. This is the rayon scope pattern.

**What is needed**: Distinguish `ThreadPool::install` (entry from non-pool context, sets TLS) from `ThreadPool::scope` (lightweight scope creation, no TLS setup, usable from a worker). Add a `pub fn scope<'scope, R>(&'scope self, f: impl FnOnce(&Scope<'scope>) -> R + Send) -> R` to the API. par_iter calls `scope`, not `install`.

### W7. The `find_ready` post-condition debug-assert is intractable for the proposed scheme
**Where**: §13.6 ("find_ready post-condition: every readied system's predecessors are completed AND its conflict bits don't intersect running.")
**Problem**: This invariant is checked **inside** find_ready as the loop condition — the debug_assert! at post-condition would just re-do the work. It's tautological. The valuable invariant would be **negative**: "any system NOT in ready_scratch fails at least one of (running, completed, all preds completed, no conflict with running)." That's exhaustive and expensive.
**Solution options**:
1. Drop this debug_assert as not useful.
2. Replace with a small spot-check: for the first 4 systems flagged in ready_scratch, re-validate from scratch.
3. Add the more useful invariant: "running ∩ ready_scratch is empty" — a single bitand.

### W8. SystemDescriptor uses `SmallVec` but no Cargo dep is declared
**Where**: §5.5 (`SystemDescriptor` uses `SmallVec<[SystemSetId; 2]>`, `SmallVec<[SystemKey; 2]>`).
**Problem**: `smallvec` is a transitive dep through `fixedbitset = "0.5"` (it may or may not pull it in), but the plan's §15.2 Cargo.toml additions list `fixedbitset = "0.5"` and `crossbeam-queue = "0.3"` but not `smallvec`. If used as a public API element, must be a direct dep with version pin.
**Solution**: Either add `smallvec = "1"` to deps OR replace `SmallVec<[X; 2]>` with `[Option<X>; 2]` (no dep, less ergonomic but enough for the typical 0-2 ordering hints per system).

---

## 🟢 Optional (improvements, not blockers)

### O1. `XorShift64Star` initialization can use `worker_id` directly without the xor
**Where**: §4.3 line 402 (`XorShift64Star::new(0x1234_5678 ^ (worker_id as u64))`).
**Problem**: 16 workers seeded as `0x1234_5678..0x1234_5588` are 16 sequential xorshift sequences. The xorshift's mixing is good enough that they don't visibly correlate, but it's cleaner to seed with `splitmix64(worker_id)` or similar one-shot mixer. Cosmetic.

### O2. The §17 OQ-5 "MIN_ARCHETYPE_FOR_PARALLEL" heuristic should be in the design, not deferred
**Problem**: OQ-5 proposes a 1024-row threshold for skipping par_iter dispatch on tiny archetypes. This is a 5-line change in `run_par_iter` (`if entity_count < MIN_ARCHETYPE_FOR_PARALLEL { /* run sequentially */ continue; }`). No reason to defer. Adopt in v1.

### O3. `Schedule::run` mints `UnsafeEcsCell::new_mutable(world)` once per frame and re-uses across the entire frame — this is wider than necessary
**Where**: §5.4 line 1018.
**Problem**: Once the dispatcher calls apply (line 1031), it has `&mut world` from the outer borrow. The `world_cell` minted in line 1018 was minted under the same borrow. So the cell is "live" but is it being passed to two places at once? The `apply` call uses `&mut world` directly; workers use `world_cell_copy`. From Rust's perspective, the dispatcher's `&mut world` borrow extends across the entire `executor_main_loop` body, and `world_cell_copy.world()` is reborrowed inside worker tasks. The aliasing is the entire executor_main_loop body — too coarse for static analysis but technically allowed under `unsafe impl Send + Sync`.

Cleaner: drop the cell at the end of each scheduling round (after workers complete), re-mint for the next round. But each round of "find_ready → spawn → wait → drain" already implies a barrier in the plan (per the fix to C4); minting per-round would be natural. Optional polish.

### O4. The §1.2 metric "Steady-state worker idle ≤ 1 % CPU per core" needs methodology
**Problem**: 1% CPU per core during idle is reasonable for a parked thread, but the way it's measured matters. Are we measuring during `pool.install` waiting for tasks, or with no install active? `std::thread::park` is supposed to be 0% CPU (kernel-side block on futex/WaitOnAddress). If we measure 1%, the issue is the spin/yield budget before park (Backoff). Add the methodology to §13.5 bench section.

---

## Positive

- **Decision matrix (§3)** is the model architectural document. Every Q lists chosen / rejected / why. This is exactly what a critic wants to read.
- **Invariant naming scheme (TPN/SCH/PAR/SEND/EXC/INS)** is excellent — gives the critic precise pointers.
- **§16 rejected alternatives** is unusually thorough. Rejecting tokio, rayon, mpsc, lockstep, actor model, fine-grained RwLock — each with a one-line reason — saves rounds of "have you considered X?" critic remarks.
- **Memory layout table (§11)** with sizes, alignments, CachePadded placement is the right level of detail. The §11.2 false-sharing audit is exemplary.
- **§3 Q1 decision to build on crossbeam-deque rather than roll our own** is the correct call given the formal verification of crossbeam's Chase-Lev. The 500-LOC-saved estimate is plausible.
- **§3 Q4 hard cap of 1024 systems** is the correct architectural call — saves us the dispatcher scalability nightmare Bevy hit. The plan is brave to take the position.
- **§4.5 Scope mechanism** mirrors rayon's well-tested pattern. Even with the C6 concerns, the shape is right.
- **§7.2 SIMD `bitset_intersects` with AVX2 + scalar fallback** is the right approach — Bevy uses scalar; we leapfrog.
- **The 22 Steps with parallelisable pairs (§14)** are realistically sized. Wave 2 parallelism (7a+7b) is appropriate.
- **The §18 self-audit checklist** demonstrates the architect is checking their own work. Useful as a debugging tool for iteration.

---

## Open questions for the architect

1. **OQ-1 to OQ-6** are listed but mostly with "decision proposal: defer". Plan should include final decisions. Critic recommendations: OQ-4 adopt (drop is_exclusive), OQ-5 adopt (min archetype threshold), OQ-3 reconsider given C8.

2. The plan declares `Box<dyn System<Out = ()>>` in `SystemBox` (SCH10). But Phase 8c's `FunctionSystem<F, M>` has `Out` as an associated type. Does the schedule support systems with `Out != ()` via some adapter, or is `Out = ()` a hard schedule requirement? Plan §SCH10 says "non-() output systems must be run via `EcsMaster::run_system` outside the scheduler" — fine, but doc-comment must say this on `Schedule::add_system`.

3. Phase 8.5 cache (`bundle_archetype_cache: Box<[OnceLock<ArchetypeId>; 1024]>`) is accessed from `Commands::spawn` apply on the dispatcher. Under Phase 9, two workers running different systems both call `Commands::spawn(SameBundle)` — Commands enqueues, apply on dispatcher processes both. `OnceLock::set` races handled (Phase 8.5 §17). **But what about reads from workers**? §15.4 says "OnceLock::get safe." Confirm: `OnceLock::get` is `&self` and lock-free; reading from many threads is safe; first write wins. OK if writes only happen on dispatcher.

4. Does `Schedule::run` actually need to clone `Arc<ThreadPool>` per frame? Line 997 (`let pool = Arc::clone(&self.pool);`). One refcount increment per frame = ~5 ns. Negligible but check whether the clone is even needed (`install` takes `&self`, not `Self`).

5. **Most important architectural question**: is the SCH7 ("apply NEVER concurrent with another system") an actual invariant the executor enforces, or a wish? If actual, where is the barrier? See C4.

---

Sources:
- [crossbeam-deque Chase-Lev verification PR (Crossbeam RFCs)](https://github.com/crossbeam-rs/rfcs/pull/26/files)
- [Formal Verification of Chase-Lev Deque in Concurrent Separation Logic (arXiv 2309.03642)](https://arxiv.org/pdf/2309.03642)
- [crossbeam-deque LIFO vs FIFO discussion (GitHub issue)](https://github.com/crossbeam-rs/crossbeam-deque/issues/11)

Relevant absolute file paths I consulted:

- `D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md` (the plan under review)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\arena.rs` (C1 evidence — `allocate_layout(&self)`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` (C1 evidence — `arena.allocate_layout` called with `&Arena`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\unsafe_ecs_cell.rs` (C4 — the cell's current `!Send + !Sync` discipline)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (C4 — drop order and field structure for SEND1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_dispatcher.rs` (C5 — `send(&self, thread_index: u32, ...)` signature)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_buffer.rs` (C5 — per-thread lane exclusivity comment)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs` (W3 — `entities_inland: Vec<EntityInland>` reallocation concern)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\access.rs` (C9 — `conflicts_with` and absence of `is_universal`)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_registry.rs` (positive — OnceLock-based registry is already Send+Sync)
- `D:\claude\BoykoEngine\CLAUDE.md` (project principles — measured inlining, no Mutex on hot path, deferred-optimization anti-pattern)

Total criticals: 9 (C10 demoted to W). Total warnings: 8. Total optionals: 4. The plan is high quality but needs another round before implementation begins.