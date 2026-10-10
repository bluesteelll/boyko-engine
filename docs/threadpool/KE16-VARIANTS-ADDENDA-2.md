# KE16 — Variants added in refutation round 2: P23–P25, G21–G24, J14, W20–W21, S10; axes 36–38 evidence

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`.
Negative results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. Cost anchors as in
`KE16-VARIANTS-PLACEMENT.md`. Round-1 additions (P17–P22, G18–G20, J12–J13, W15–W19, L11–L13)
are in `KE16-VARIANTS-ADDENDA.md`.

These are the designs the second pair of refuters found that neither the lenses nor round 1 had
opened, the cells whose "empty" or "unbuilt" verdict they overturned, and the evidence behind the
three new axes (36 timed-wait resolution, 37 analytical model, 38 contention class). Each keeps its
group letter so cross-references stay by id. A plain `[S]`/`[D]`/`[P]` in this file means this
synthesis re-read the source in this session; `[R:*]` means the refuter read it and this synthesis
did not re-open it (listed in §U).

---

## Group P additions — placement

### P23 — An owner-only per-worker queue INSIDE a stealing pool, by design (rayon's broadcast deque)

**Axis cell.** 1: a second per-worker deque whose `Stealer` half is held only by its **owner** ·
2: sender pushes to EVERY worker under a lock · 11: **not** in the steal scan set, by design · 33:
wake **all** · 39: chosen per call (`broadcast`, not `spawn`).

**Implemented by.** rayon-core `registry.rs` — `WorkerThread` field, verbatim doc: `/// the "stealer"
half of the worker's broadcast deque` `stealer: Stealer<JobRef>`; `Registry::inject_broadcast`:
`let broadcasts = self.broadcasts.lock().unwrap(); assert_eq!(broadcasts.len(),
injected_jobs.len()); for (worker, job_ref) in broadcasts.iter().zip(injected_jobs) {
worker.push(job_ref); }` then `for i in 0..self.num_threads() { self.sleep
.notify_worker_latch_is_set(i); }`; `take_local_job` pops the worker's own deque, then
`self.stealer.steal()` on its OWN broadcast deque (order re-read; body summarised by the fetcher)
`[S]`.

**Documented rationale.** A broadcast job must run once on every worker, so a per-worker queue that
only the owner drains is the correct data shape for that call and only that call — it is fed by a
dedicated API, never by `spawn`/`join`. The wake is all-worker because every worker has a job.

**Evidence.** P0's exact data shape (a per-worker queue polled only by its owner, after the local
pop and before stealing) ships in rayon `[S]`. The axis-1 sentence "no precedent where such a queue
is fed by a stealing pool's ordinary spawn path" survives only because the feeder is `broadcast`;
the prior "rayon = closed by construction: only registered deques" (axis 11) was wrong for this
queue, and rayon has a wake-all value on axis 33 the prior list omitted.

**Failure modes.** N pushes under a `Mutex` per broadcast; W wakes per broadcast; a job that never
gets its worker's turn waits for that worker.

**Cost.** SPAWN (broadcast): a lock + W Chase-Lev pushes + W latch notifies. STEAL: none by design.
IDLE: one extra owner-only probe per `take_local_job`.

**Fit.** Not a candidate for `par_iter` waves (mandatory tasks that must NOT run once per worker).
It sharpens the P0 record: the field ships owner-only per-worker queues when the *semantics* say
"this worker, and only this worker" — and never when the semantics say "whoever is free".

---

### P24 — Round-robin sender placement into registered per-thread queues; fan-out conditioned on push kind (Wicked Engine)

**Axis cell.** 1: per-thread queues, all polled by all workers (sweep from own id) · 2: sender picks
the target by a **rotating counter** · 34: rotating counter (not spawner identity, idleness, data
owner, first-accepting, explicit id, random trylock, affinity, last-scheduler or LLC — a ninth key)
· 33: **one for a single task, all for a wave** · 23: three pools by priority.

**Implemented by.** WickedEngine `wiJobSystem.cpp` (re-read; code summarised by the fetcher, symbols
quoted): `Execute`/`Dispatch` choose the queue by `uint8_t idx = nextQueue.fetch_add(1,
std::memory_order_relaxed); idx = mod_lut[idx];` (a precomputed modulo table); workers sweep all
queues starting at their own (`startingQueue`, incremented per queue exhausted); `Wait()` helps from
`next_queue_index()`; wake = `sleepingCondition.notify_one()` for `Execute` and `notify_all()` for a
`Dispatch` wave when `numThreads > 1`, plus `notify_all()` on `Wait()`; pools High (`numCores − 1`),
Low (`numCores − 2`), Streaming (1) `[S]`.

**Documented rationale.** None beyond the code; the shape is "spread producers across queues so no
single line is hot, let every consumer see every queue" — a P14-to-P1 midpoint.

**Evidence.** A shipped open-source game engine not in the round-1 catalogue; the only surveyed
pool whose wake COUNT depends on the KIND of push (axis 33 value added) `[S]`.

**Failure modes.** A rotating counter is itself one shared RMW per spawn (the rotor cost W0 pays);
every worker sweeps every queue on every acquisition (O(W) shared probes per task, the P14 cost
spread over W lines); `notify_all` per wave is W syscalls on the spawner.

**Cost.** SPAWN: one shared RMW (counter) + a lock-protected queue push on a foreign line + one or W
wakes. STEAL: an O(W) sweep. IDLE: condvar.

**Fit.** Not a candidate. Two transferable values: the **wave-conditioned fan-out** (E27 → W-f: a
batch spawn knows N at the one push App-4 leaves) and a ninth axis-34 key.

---

### P25 — Rotating start + first-accepting `try_push`, one `try_pop` rotation then block on the OWN queue (stlab)

**Axis cell.** 1: per-thread queues under per-queue locks · 34: a HYBRID key — rotating start,
first lock-acquirable queue · 20: steal budget = exactly one rotation of `try_pop`, then a
**blocking** pop on the own queue · 13: **no** idle registry at all · 22: elastic (thread limit
"the maximum of 9 or hardware concurrency multiplied by 4 plus 1").

**Implemented by.** stlab `default_executor.hpp`, verbatim: push `auto i = _index++; for (unsigned n
= 0; n != _count; ++n) { if (_q[(i + n) % _count].try_push(std::forward<F>(f), P)) return; }
_q[i % _count].push(std::forward<F>(f), P);`; pop `for (unsigned n = 0; n != _count && !f; ++n) {
f = _q[(i + n) % _count].try_pop(); } if (!f) { bool done; std::tie(done, f) = _q[i].pop(); if
(done) break; }`; `_count` = "the maximum of 1 and hardware concurrency minus 1" (or
`STLAB_TASK_POOL_MAXIMUM()`) `[S]`.

**Documented rationale.** Sean Parent's "portable task system" reference design: contention is
avoided by *trying* locks, not by lock-free structures; the worker never parks on a registry — it
blocks on its own queue's condvar, so the producer's `push` fallback IS the wake.

**Evidence.** Distinct from Julia (random start, priority heaps, P20) and PhysX (index-0 start,
bounded, P17); widely copied `[S]`.

**Failure modes.** A worker blocked on its own queue's condvar is invisible to a producer whose
`try_push` succeeded elsewhere; the fallback `push(i % _count)` targets a queue by counter, not by
idleness, so a parked worker may stay parked while a busy one's queue grows.

**Cost.** SPAWN: one shared RMW (counter) + up to W `try_lock`s + one push. STEAL: up to W `try_lock`s
per acquisition. IDLE: a condvar on the own queue, no registry.

**Fit.** Not a candidate; fills axis 34's hybrid value and marks the "no registry" end of axis 13
with a widely copied design.

---

## Group G additions — granularity, promotion, and the data structure

### G21 — Thief-splitting: an adaptive split budget halved per split and RESET on migration (rayon `Splitter`)

**Axis cell.** 3: split-in-half ranges · 17: a value between G8 (eager to a fixed grain) and G9
(lazy on demand) — **"my job was observed to have migrated"** is the promotion trigger · 16:
caller-frame jobs (`join` `StackJob`).

**Implemented by.** rayon `src/iter/plumbing/mod.rs`, verbatim: "Thief-splitting is an adaptive
policy that starts by splitting into enough jobs for every worker thread, and then resets itself
whenever a job is actually stolen into a different thread."; `Splitter::new` → `splits:
crate::current_num_threads()`; `try_split(stolen)`: on `stolen` — "This job was stolen! Reset the
number of desired splits to the thread count, if that's more than we had remaining anyway." →
`max(current_num_threads(), splits/2)`, else halve; `LengthSplitter::try_split`: "If splitting
wouldn't make us too small, try the inner splitter. `len / 2 >= self.min && self.inner
.try_split(stolen)`"; `bridge_producer_consumer`'s helper passes `context.migrated()` `[S]`.

**Documented rationale.** Eager splitting to W pieces is enough when nobody steals; a steal is
evidence of hunger, so the budget resets — without polling a deque or a timer.

**Evidence.** The shipped Rust `par_iter` shape, absent from G8's occupant list (which cited only
`with_min_len`) `[S]`. It uses no queue emptiness proxy (G9's inverted-under-defect-A problem) and
no timer (G10).

**Failure modes.** Presupposes the child can be stolen at all (P1–P4); `migrated()` is known only
when the job runs, so the reset is one level late; the budget halves down the recursion, so a deep
imbalance late in the tree is under-split.

**Cost.** SPAWN: one local integer per split; a `StackJob` per `join`, no heap. STEAL: unchanged.

**Fit.** A/B: **design note (App-5′)** for the `par_iter` rework — the trigger "my chunk migrated"
is available for free from the worker id at run time, and composes with G18/G19. Not a pool variant.

---

### G22 — Self-replicating task: one queued task per wave; each replica that STARTS queues one more (.NET `Parallel.For` `TaskReplicator`)

**Axis cell.** 31: one descriptor per wave · 33: a **task** cascade — "as many as actually start,
+1 outstanding" — distinct from FJP's wake cascade (each activated worker signals another) · 12:
the wake is the scheduler running the replica.

**Implemented by.** dotnet/runtime `TaskReplicator.cs`, verbatim: "TaskReplicator runs a delegate
inside of one or more Tasks, concurrently. The idea is to exploit 'available' parallelism, where
'available' is determined by the TaskScheduler."; "We always keep one Task queued to the scheduler,
and if it starts running we queue another one, etc., up to some (potentially) user-defined limit.";
`Replica.Execute`: `if (!_replicator._stopReplicating && _remainingConcurrency > 0) {
CreateNewReplica(); _remainingConcurrency = 0; }` `[S]`.

**Documented rationale.** Fan-out proportional to the scheduler's ACTUAL free capacity: a replica
runs only if a worker was free, and it is the running replica — not the spawner — that pays the next
queue insertion. Replication stops when the user action completes without yielding.

**Evidence.** A shipped fourth value on axis 33 beside one / rule / cascade / all `[S]`; one queue
insertion per replica, no up-front N wakes.

**Failure modes.** Ramp-up is serial in the scheduler's dispatch latency (one replica per dispatch
round); the last replica's insertion is wasted work when the loop is about to end.

**Cost.** SPAWN: one descriptor + one queue op; each replica: one queue op + one claim. STEAL: the
pool's. IDLE: the pool's.

**Fit.** M3: a **W-f grid value** — `{1, FJP cascade, TaskReplicator cascade, min(N, sleepers)}`;
composes with G12 (batch claiming) naturally: the replica IS the batch claimer.

---

### G23 — Block-based work-stealing deque: owner and thieves synchronise per BLOCK, not per task (BWoS)

**Axis cell.** 15: fixed-size blocks inside the deque; the owner puts/gets in its current block with
no thief interference; thieves sample random OTHER blocks · 3: block granularity, "probabilistic
stealing" toward longer queues · 10: the owner's per-task synchronisation is removed · **41
(deque synchronisation granularity): per block** — the only non-disqualified data-structure-level
row (G15–G17 are all disqualified or already had).

**Implemented by.** Wang et al., "BWoS: Formally Verified Block-based Work Stealing for Parallel
Processing", OSDI'23 (read via proxy) `[P]`; NVIDIA stdexec `exec/static_thread_pool.hpp`
(`bwos_params{numBlocks 32, blockSize 8}`, `near_victims_`/`all_victims_`, `max_steals_ =
thread_count + 1`, per-thread `remote_queue` inboxes, states running/stealing/sleeping/notified)
`[R:S]`.

**Documented rationale (abstract, verbatim via proxy).** "Thieves and owners rarely operate on the
same blocks, greatly removing interferences and enabling aggressive optimizations on the owner's
synchronization with thieves."; "a novel probabilistic stealing policy that guarantees thieves steal
from longer queues with higher probability" `[P]`.

**Evidence.** "using BWoS improves performance by up to 1.25 x in the Renaissance macrobenchmark when
applied to Java G1GC, provides an average 1.26 x speedup in JSON processing when applied to Go
runtime, and improves maximum throughput of Hyper HTTP server by 1.12 x when applied to Rust Tokio
runtime. In microbenchmarks, it provides 8-11 x better performance than state-of-the-art designs."
`[P]` (the refuter's body figures 25.3 % / 25.8 % / 12.3 % throughput and 6.74 % latency are the
same numbers at higher precision — U71). Integrated into HotSpot G1GC, Go 1.18, Tokio 1.17.0,
Kotlin coroutines, Eigen per the paper; **no upstream adoption by Tokio or Go is documented** —
unverified.

**Failure modes.** A block that is neither full nor empty cannot be stolen from at the same
granularity as a Chase-Lev deque — the owner's last few tasks are private until a block hand-over;
fixed block size is a knob; the formal model is for a specific ownership protocol.

**Cost.** SPAWN: an owner put inside its block — no CAS on a line thieves write. STEAL: a block
sample + a per-block takeover. IDLE: the pool's.

**Fit.** Not a candidate this round: it changes the substrate (crossbeam) rather than the placement,
and nothing in A/B/M3 requires it. Recorded because axis 41 had no live row and because the stdexec
integration is a P19-style sender-routed per-worker inbox inside a stealing pool — whether that
inbox is stealable must be read before it is placed on axis 11 (U71).

---

### G24 — Task-scheduler BYPASS: the body returns the next task; it is neither spawned nor queued (oneTBB)

**Axis cell.** 1: **not queued** — returned by the body as the next task · 40: successor hand-off =
"returned, no queue op" · 35: mandatory, yet deliberately unreachable by thieves · 10: zero
spawn-path synchronisation for that task.

**Implemented by.** oneTBB user guide "Task Scheduler Bypass" (uxlfoundation.github.io, re-read):
"an optimization where you directly specify the next task to run"; "directly point the preferable
task to be executed next instead of spawning it"; "almost guarantees that the task is executed by
the current thread and not by any other thread"; "at the moment the only way to use this
optimization is to use preview feature of `oneapi::tbb::task_group`" `[D]` (the macro name
`TBB_PREVIEW_TASK_GROUP_EXTENSIONS` and `task_handle` are from oneTBB issue #1523 `[R:D]`).

**Documented rationale.** The returned task is "the first candidate for execution by the current
thread", skipping the spawn and the dequeue of the normal cycle — the cheapest possible hand-off
of a dependent successor, and the only one that keeps the predecessor's working set on the core
without any queue.

**Evidence.** Distinct from G10/P15 (NOT promoted later) and from `par_iter`'s < 1024-row inline
path (the caller's frame RETURNS first — no stack growth) `[D]`.

**Failure modes.** A bypassed task is invisible to thieves for its whole lifetime (P0's property,
chosen); a long chain of bypasses is a serial lane by construction; only the preview API exposes it.

**Cost.** SPAWN: none. STEAL: impossible. IDLE: n/a.

**Fit.** M1/Loc: the zero-sync cell the criterion asks for, occupied. The ECS analogue is S10's
"run the released successor on the completing worker" (axis 40); recorded as the shipped extreme.

---

## Group J addition — the cross-pool helper

### J14 — What a thread may drain while joining a FOREIGN pool's scope (rayon `in_worker_cross` vs boyko's joiner)

**Axis cell.** 42 (cross-pool reachability and helper set): rayon — a pool-A worker awaiting pool
B's job helps in **its own pool A**, never in B; boyko — a pool-A worker joining a pool-B scope
drains **pool B's `injector_local[own id]`**, a slot owned by B's worker of the same index · 19:
"own pool only while awaiting a foreign pool" (rayon) vs "anything, including a foreign owner-only
slot" (boyko).

**Implemented by.** rayon-core `registry.rs::in_worker_cross` — "This thread is a member of a
different pool, so let it process other work while waiting for this `op` to complete." → `let latch
= SpinLatch::cross(current_thread); … self.inject(job.as_job_ref()); current_thread.wait_until
(&job.latch);` `[S]` (comment verbatim; body as summarised). boyko `scope.rs::join_workers_until_
drained` — `let wid = tls::current_worker_id(); let on_worker = (wid as usize) < inner
.injector_local.len(); let local_inj_idx = wid as usize;` then `inner.injector_local[local_inj_idx]
.steal_batch_and_pop(&scratch)` at `:479` — with **no** `active_pool_ptr == inner` check; the only
callers of `tls::active_pool_ptr` in the crate are `worker.rs:39,369` and `thread_pool.rs:253,375`
`[L]` (grep this session). `push_task` HAS the check (`worker.rs:369`) `[L]`.

**Documented rationale.** rayon: a member of another pool has a worker context there, so it helps
there; it has none in the target pool, so it never touches the target's per-worker structures.
boyko: `push_task`'s FIX-2 rationale (`worker.rs:355-364`) states the cross-pool hazard for the
PUSH side and closes it; the JOIN side was not given the same check.

**Evidence.** Within one pool the invariant "`injector_local[i]` is read only by worker i" holds
(A.2). Across pools it fails in the other direction: `PoolInner::scope` (`thread_pool.rs:247-276`)
does not touch TLS and can be called from a pool-A worker holding a handle to pool B; on drop, the
join drains B's `injector_local[wid_A]` — tasks pool B's worker `wid_A` pushed for itself are run
on a pool-A thread `[L]`. Whether any production caller joins cross-pool is unknown (same class as
§H.1; boyko has multi-world and a cross-pool spawn arm, `worker.rs:373`).

**Failure modes.** A foreign worker consuming an owner-only slot: correctness of pool B's locality
intent is silently broken; a pool-B task that assumed it runs on pool B (diag lane, TLS) runs on
pool A; the joiner's `scratch` then serialises it (G3).

**Cost.** None to fix: one pointer comparison already computed by `push_task`.

**Fit.** **App-6** — add `std::ptr::eq(tls::active_pool_ptr(), inner)` to `on_worker` in
`join_workers_until_drained`; independent of every A/B/W candidate; extends the cross-pool test
(`tests/cross_pool_routing.rs`) to the join side.

---

## Group W additions — the completion path and the idle spin

### W20 — Count-gated completion: only the LAST completer signals; the wake target outlives the scope (rayon `CountLatch`; std `thread::scope`)

**Axis cell.** 29: **count-gated** — one `fetch_sub` per task, one swap + conditional wake per SCOPE
· 12: the completion trigger fires once per scope, not per task · 10: COMPLETE = 1 RMW per task ·
28: the join counter is exactly what loom M1 models.

**Implemented by.** rayon-core `latch.rs` (re-read, verbatim): `CountLatch::set`: `if (*this)
.counter.fetch_sub(1, Ordering::SeqCst) == 1 { match (*this).kind { CountLatchKind::Stealing {
ref latch, ref registry, worker_index } => { let registry = Arc::clone(registry); if CoreLatch::
set(latch) { registry.notify_worker_latch_is_set(worker_index); } } CountLatchKind::Blocking { ref
latch } => LockLatch::set(latch), } }`; `CoreLatch::set`: `let old_state = (*this).state.swap(SET,
Ordering::SeqCst); old_state == SLEEPING`; states `UNSET = 0, SLEEPY = 1, SLEEPING = 2, SET = 3`;
`SpinLatch::set` doc: "NOTE: Once we `set`, the target may proceed and invalidate `this`!" — the
registry and worker index are read BEFORE the swap `[S]`; `scope/mod.rs::ScopeBase {
job_completed_latch: CountLatch }`, set from `execute_job_closure` `[S]` (lens 2 / round 1). Rust
`std` `library/std/src/thread/scoped.rs` (re-read, verbatim): `pub(super) struct ScopeData {
num_running_threads: Atomic<usize>, a_thread_panicked: Atomic<bool>, main_thread: Thread }`;
`decrement_num_running_threads`: `if self.num_running_threads.fetch_sub(1, Ordering::Release) == 1
{ self.main_thread.unpark(); }`; "We put the `ScopeData` into an `Arc` so that other threads can
finish their `decrement_num_running_threads` even after this function returns." `[S]`.

**Documented rationale.** `fetch_sub` returns the previous value atomically, so "am I last" is free.
The hazard is not the counter but the LIFETIME of the wake target: the scope may be freed the
instant the count hits zero, so a post-decrement access to anything the scope owns is a
use-after-free. rayon solves it by reading the registry-owned target before the swap (the registry
outlives every scope); std by putting `ScopeData` in an `Arc` shared by the completers.

**Evidence.** boyko `scope.rs:149-151` states the opposite: "The unpark is UNCONDITIONAL (no `prev
== 1` gate): learning we are last would require reading `pending` after the sub — too late." `[L]`
— **refuted**: two shipped Rust designs gate on `fetch_sub(…) == 1` and stay sound by the lifetime
decision above (N66). The round-1 axis-29 cell "gated by a joiner-published parked flag (design,
unbuilt)" is **occupied**: rayon's `CoreLatch` UNSET/SLEEPY/SLEEPING/SET IS a joiner-published
parked flag, and `CoreLatch::set` wakes only when `old_state == SLEEPING` `[S]`. Per TASK rayon pays
one SeqCst `fetch_sub`; the swap and the conditional wake happen once per scope — the round-1 cost
table's rayon COMPLETE column ("swap + wake only when SLEEPING", `[R:S]`) understated rayon's
advantage by the wave size and is corrected.

**Failure modes.** The last completer must be able to reach the wake target after the count hits
zero: either the target lives in a structure that outlives the scope (a registry slot indexed by
the joiner's id, as rayon) or the shared block is reference-counted (as std — an `Arc` is one more
RMW per task on a multi-writer line, which defeats the purpose unless the count and the refcount are
the same word); a spurious-wake-free design must also handle the joiner that is NOT parked (the
swap is then a token store, no syscall).

**Cost.** COMPLETE: 1 shared RMW per task (`fetch_sub`) + once per scope a swap and a conditional
`unpark`. Today (W17): 2 shared RMWs + a conditional syscall per task.

**Fit.** M3: **candidate W-d′** — replaces round 1's flag design (W17) as the primary form: gate on
`fetch_sub(1) == 1`, and move the wake target out of `ScopeShared` into a registry-owned parker slot
(the joiner's `WorkerHandle` already exists for workers; the dispatcher needs one slot) so the
unpark-before-decrement order is no longer needed for memory safety. The round-1 flag remains the
fallback if the target must stay inside `ScopeShared`. Must extend loom M1 (the count gate is
exactly what M1 models).

---

### W21 — Decoupled idle spin on ONE signal word; queues scanned only after acquisition (.NET) — the occupant of E18 / W-c

**Axis cell.** 6: spin reads one shared word (`_separated._counts`), touches no queue; queues are
scanned only after the semaphore is acquired · 10: zero coherence traffic while spinning until a
post · 12: presupposes a sender-side semaphore `Release` per enqueue (a push-side signal — W1/W2's
kin), so this is **W1-plus-decoupled-spin**, not a free-standing spin knob.

**Implemented by.** dotnet/runtime `LowLevelLifoSemaphore.cs::WaitSlow` (re-read): `while
(spinsRemaining > 0) { spinsRemaining -= Backoff.Exponential(iteration++); Counts counts =
_separated._counts; if (counts.SignalCount != 0) { /* attempt to decrement signal count */ } }`,
then `return WaitNoSpin(timeoutMs);`; `DefaultSemaphoreSpinCountLimit = 1024`, `DefaultWakeCooldown
= 4`; header: the spin targets "about 35 microseconds" assuming "a single spin is calibrated to
around 35 nanoseconds" `[S]`; `PortableThreadPool.WorkerThread.cs::WorkerThreadStart`: `while
(semaphore.Wait(timeoutMs)) { WorkerDoWork(...) }` — `ThreadPoolWorkQueue.Dispatch()` runs only
after the semaphore is acquired `[S]` (round 1 had read the same file for the 35 µs constant
without placing it here).

**Documented rationale.** The spin is calibrated to the wake latency it is trying to beat (Karlin's
spin-block rule, axis 37: spin for the block cost, 2-competitive); the semaphore word is the only
thing polled, so W idle workers cost the coherence fabric nothing until a post.

**Evidence.** E18 ("spin budget decoupled from the steal scan") was recorded in round 1 as having
"no occupant among the runtimes read" after rayon was found to scan every round; .NET is the
occupant, and its source was already in the `[S]` list `[S]`.

**Failure modes.** Works only because every enqueue posts the semaphore — the spin sees new work
through the signal, never by looking; a pool that gates the push-side wake (W1/W2) must keep a
"work exists" word for the spinner to read, or the decoupled spin sees nothing.

**Cost.** IDLE: a load per spin iteration of one word; O(W) queue probes only on a real wake.
SPAWN: one semaphore post (a shared RMW).

**Fit.** M3: **W-c is occupied upstream and unbuilt here**; the buildable form for boyko is "spin
on the idle bitmap's line or a per-pool `epoch` word, scan every k-th round or on a change" — a
composite of W21 and W1. Justify by measurement; the prior remains Go #28808 for the coupled form.

---

## Group S addition — successor hand-off on completion

### S10 — Immediate successor: the CPU that completes a task runs its dependency successor, with probability p (Nanos6 / OmpSs-2)

**Axis cell.** 40 (successor hand-off on completion): **run inline by the completing thread** ·
34: the placement key is "the CPU that completed the predecessor" · 12: no queue op and no wake for
that successor · 7: cache reuse between dependent tasks by construction.

**Implemented by.** Nanos6 README (re-read): `scheduler.immediate_successor` — "Probability of
enabling the immediate successor feature to improve cache data reutilization between successor
tasks."; when active, on task completion a CPU begins executing the successor determined by data
dependencies; default **0.75**; `scheduler.policy` fifo/lifo (fifo default); `scheduler.priority`
(on by default) `[D]`.

**Documented rationale.** The successor's inputs are the predecessor's outputs, which are in the
completing core's cache; a probability < 1 keeps some successors flowing to the pool so a long
chain does not serialise on one CPU.

**Evidence.** Distinct from J8 (Molecule/UE continuations are ENQUEUED), from TBB affinity (an id
from a previous RUN, P5), from S1/S0 (a released successor is spawned as a task into a queue) and
from G24 (returned by the body, never queued) `[D]`. It is the task-level form of exactly the event
mechanism 1 turns on in the ECS executor: `pred_remaining` reaching zero.

**Failure modes.** A probability knob; the successor may be conflicting or exclusive (must be
checked against the conflict bitset before inlining); a chain of inlined successors is a serial lane
by construction; the successor's other predecessors must all be done (the ready test is the same as
the executor's).

**Cost.** COMPLETE: one ready test + a branch; no queue op, no wake, no working-set migration.

**Fit.** M1/Loc: **candidate M1-c (design note)** — when a worker finishes system S, decrements
`pred_remaining[S′]` to zero, and S′ is conflict-free against the running set, run S′ on the same
worker without returning to `injector_global` and without a dispatcher round trip (which today costs
a `park_timeout` of 100 µs → ≥1 ms on Windows, axis 36). This is the cheapest possible answer to the
owner's locality objection to mechanism 1 — the successor's inputs are what S just wrote. Presupposes
M1-b (readiness released per completion, S6) and must respect the apply-window gate for deferred
commands. Nobody does it at the ECS-system level (E30).

---

## Axis 36 — Timed-wait RESOLUTION of the target OS (the truth behind every backstop number)

**Values.** Windows: `park_timeout(d)` → `WaitOnAddress(…, dur2timeout(d))`, `dur2timeout` rounds
nanoseconds UP to whole milliseconds; expiry is then bounded by the system timer resolution, which
Windows 10 2004+ does not raise for a process that never called `timeBeginPeriod`. Linux:
`futex(2)` timeouts carry the default 50,000 ns timer slack; real-time policies exempt. A caller can
buy resolution (`timeBeginPeriod(1)`, documented power and scheduler cost) or spin until the
deadline (no wait at all). **boyko's cell:** `park_timeout(Duration::from_micros(50))` at
`scope.rs:512` and `park_timeout(100 µs)` at `schedule.rs:683` are constants in the source, not the
waits that occur.

**Evidence.**
- Toolchain path, read at `C:\Users\flint\.rustup\toolchains\nightly-x86_64-pc-windows-gnu\lib\
  rustlib\src\rust\library\std\src\sys\` `[S]`: `sync/thread_parking/mod.rs:1-16` — `all(target_os
  = "windows", not(target_vendor = "win7"))` → `mod futex; pub use futex::Parker;`;
  `sync/thread_parking/futex.rs:68-85` — `park_timeout` → `futex_wait(&self.state, PARKED,
  Some(timeout))`; `sync/futex/windows.rs:66-67` — `let timeout = timeout.map(dur2timeout)
  .unwrap_or(c::INFINITE); c::WaitOnAddress(addr, compare_addr, size, timeout) == c::TRUE`;
  `pal/windows/mod.rs:240-254` — "timeouts in windows APIs are typically u32 milliseconds …
  Nanosecond precision is rounded up": `ms + (if dur.subsec_nanos() % 1_000_000 > 0 { 1 } else
  { 0 })`. **50 µs → 1 ms; 100 µs → 1 ms.**
- `grep -rnE 'timeBeginPeriod|NtSetTimerResolution|timer_slack|PR_SET_TIMERSLACK'` over
  `D:/wt/threadpool`: no files `[L]`.
- Microsoft Learn `timeBeginPeriod` (re-read) `[D]`: "Starting with Windows 10, version 2004, this
  function no longer affects global timer resolution. For processes which call this function,
  Windows uses the lowest value (that is, highest resolution) requested by any process. For processes
  which have not called this function, Windows does not guarantee a higher resolution than the
  default system resolution."; "Setting a higher resolution can improve the accuracy of time-out
  intervals in wait functions. However, it can also reduce overall system performance, because the
  thread scheduler switches tasks more often. High resolutions can also prevent the CPU power
  management system from entering power-saving modes." The default system resolution's VALUE is not
  on the page (commonly quoted 15.625 ms — `[B]`, U79).
- man7 `PR_SET_TIMERSLACK` (re-read) `[D]`: "The timer slack values of init(1) (PID 1), the ancestor
  of all processes, are 50,000 nanoseconds (50 microseconds)."; applies to "select(2), pselect(2),
  poll(2), ppoll(2), epoll_wait(2), epoll_pwait(2), clock_nanosleep(2), nanosleep(2), and
  futex(2)"; "Timer slack is not applied to threads that are scheduled under a real-time scheduling
  policy".

**Consequences for the catalogue (all corrected in place).** Axis 14 "closed by a timeout backstop
(50 µs)" → ≥1 ms on the Windows target; W5's "a 50 µs spike is 0.3 % of the frame" → ≥1 ms is ≥6 %
of a 16 ms frame per occurrence, 20× the stated figure; J11's "100 µs latency floor" on the apply
window → ≥1 ms per dispatcher park; W17/W-d's "the race loses one wake covered by the 50 µs backstop
— latency, not loss" → the latency is ≥1 ms; A.7's harness numbers were all taken on this path.
The actual expiry latency on the bench machine is **unmeasured** (H.10) — the rounding is verified,
the timer resolution in effect is not.

**Fit.** **App-7**: measure `park_timeout(50 µs)`'s real latency on the bench machine; then choose
between `timeBeginPeriod(1)` at boot (documented cost), a spin-until-deadline backstop, or removing
the timeout dependence (W-b/W-d′ make the backstop rarer, not shorter).

---

## Axis 37 — Which analytical model's hypotheses the workload actually satisfies

Round 1 stated (P1) that the theory was absent for a help-first flat wave. It is not; the wrong
theorem was being consulted. Every entry below was read this session via the r.jina.ai text proxy
(`[P]`, U-0 applies) unless marked `[R:P]`.

| Model | Hypotheses | Bound (as read) | Applies to |
|---|---|---|---|
| Blumofe & Leiserson JACM'99 (round 1) | fully strict; work-first ("Ga is placed on the bottom of the ready deque, and the processor commences work on Gb") | `T1/P + O(T∞)`; steal attempts `O(P·T∞)` | NOT our shape (help-first, flat) |
| **Arora, Blumofe, Plaxton TOCS'01** | "we consider arbitrary multithreaded computations as opposed to the special case of 'fully strict' computations"; "Of the two ready threads (the assigned thread and the newly ready thread), the process pushes one onto the bottom of its deque and continues executing the other … **The bounds proven in this paper hold for either choice.**" | Theorem 9: "Consider any multithreaded computation with work W and critical-path length D being executed by the non-blocking work stealer with p processes in a dedicated environment. The expected execution time is O(W/p + D)."; Theorems 10–12 with `P_A` under multiprogramming | **P1/A1** (help-first covered; the spawn chain sits inside D, as round 1 computed); every A-candidate that lands work in a deque any thief can reach |
| **Tchiboukdjian, Gast, Trystram, "Decentralized List Scheduling" (arXiv:1107.3734; ISAAC'10)** | W unit independent tasks; steal-HALF; uniformly random victim; all tasks initially on one processor in the worst case | Theorem 2: `E[Cmax] ≤ W/m + 3.65·(log2 W + 1/(2 ln 2)) + 1`; Theorem 3 (ν ≈ 2.94): `E[Cmax] ≤ W/m + 3.24·(log2 W + 1/(2 ln 2)) + 1`, "optimal up to a constant factor in log2 W"; Theorem 1: steal requests `E[R] ≤ m·λ·log2 Φ(0) + m(1 + λ/ln 2)`; Theorem 6 (DAG of depth D): `E[Cmax] ≤ W/m + 5.5·D + 1`, "improves upon Arora et al. (2001), which achieved 32·D" (the refuter states it as steal requests ≤ 5.5·m·D vs 32·m·D — the same bound up to the factor m; exact form U81) | **G2** (steal half), **G20/App-4** (a wave published as a pile fills in O(log2 W) steal rounds, not O(N) — the quantitative argument for batch spawn), **G12/G18** (descriptor claims), **B0–B2** (the joiner's half) |
| **Gast, Khatiri, Trystram, Wagner, "A new analysis of Work Stealing with latency" (arXiv:1805.00857)** | W unit independent tasks; steal half; each steal costs a latency λ | Theorem 4.1: `E[Cmax] ≤ W/p + 16.12·λ·log2(W/2λ) + 3λ`; Lemma 4.3: `E[R] ≤ 2pγ·log2(W/λ)`, γ < 4.03; simulations sit 4–5.5× below the bound; empirical fit `W/p + 3.8λ·log2(W/λ)` (simulation only, U82) | **P17/A5** ("beats A1 by one steal latency" is priced by the λ term), **W-f** (a wake chain adds a λ per hop), **G23** |
| **Karlin, Manasse, McGeoch, Owicki, Algorithmica 1994 §3.1** | spin-vs-block with context-switch cost C; strong / weak adversary | Theorem 6: "There is no c-competitive algorithm for the spin-block problem for c < 2 against a strong adversary" — spinning for exactly C is 2-competitive and optimal among deterministic strategies; Theorem 7: a randomised strategy with density `r(t) = 1/C for 0 ≤ t ≤ C` is "strongly e/(e − 1)-competitive against a weak adversary" (≈1.58) | **W11/W-c/W21**: .NET's "calibrated to wake latency" is Karlin's rule; boyko's 127-pause budget is not calibrated to anything; a randomised spin length on [0, C] is the theory's best |
| Kruskal & Weiss IEEE TSE 1985 (fixed-size chunking), Polychronopoulos & Kuck IEEE TC 1987 (GSS), Hummel (factoring), Tzen & Ni (trapezoid), Hagerup 1997 (experimental comparison; "the Bold strategy performed well across the entire gamut of experiments") | per-iteration cost variance σ, p processors, overhead h | FSC `K = (√2·n·h / (σ·p·√ln p))^(2/3)`; GSS chunk = `R/p` | **G8/G19** (grain and shrinking batch) — `[R:P]`, U78; GSS is not the best known shrinking rule per Hagerup (N67) |

**Fit.** Every `[I]` prognosis in the round-1 shortlist for A1, A5, App-4, W-f and W-c now has a
theorem naming its hypotheses; the harness grid remains the arbiter, but the *direction* each cell
is expected to move is no longer an inference.

---

## Axis 38 — Contention class of each RMW on the spawn / steal / idle / complete paths

Axis 10 counts lock-prefixed instructions; the acceptance criterion turns on CACHE-LINE TRANSFERS.
Two classes, from the anchors already used (`[B]`, Downs — U50): a lock-prefixed op on a line with
ONE writer stays Modified in that core's L1 (~10 ns class); an op on a line written by many cores
is a transfer (40–400 ns class). A write to a FOREIGN line (another core's queue) is the transfer
class by construction. Classification is inference `[I]` over verified single-writer facts `[L]`.

| Path (boyko today) | RMW | Line | Class |
|---|---|---|---|
| SPAWN | `pending.fetch_add(1, AcqRel)` (`scope.rs:132`) | `ScopeShared.pending` — written by every completer | **multi-writer** |
| SPAWN | `Injector::push` SeqCst CAS on the tail index | `injector_local[wid]` tail — under P0 route (b) written by the owner only, read by nobody until the join | **single-writer** (uncontended) |
| SPAWN | `slot.state.fetch_or(WRITE, Release)` | the block slot — same | **single-writer** |
| SPAWN | `wake_rotor.fetch_add(1, Relaxed)` (`worker.rs:324`) | written by every spawner | **multi-writer** |
| SPAWN | `idle.load(Acquire)` | a load; the line is written by every parking worker | shared read |
| COMPLETE | `waker.unpark()` swap | the joiner's parker line — written by every completer | **multi-writer** |
| COMPLETE | `pending.fetch_sub(1, AcqRel)` | as above | **multi-writer** |
| STEAL | `Stealer::steal*` CAS on the victim's front + epoch pin | the victim's deque line + the thread-local epoch | transfer + local |
| IDLE | `idle.fetch_or` / `fetch_and` | one line for all W workers | **multi-writer** |

**Consequences.** Round 1's "boyko spawn = 4 shared RMWs, the most expensive in the survey" is true
as "four lock-prefixed instructions" and false as "four cache-line transfers": two are single-writer
under P0. The P0→P1 spawn delta is therefore two uncontended ops (plus the wake), not two
transfers; P17 differs from P0 by exactly this class (its push targets a FOREIGN line); P3 differs
from P0 by moving the two single-writer ops onto a multi-writer line. The completion path's two
RMWs are both multi-writer, which is why W20/W-d′ matters more than the spawn-side count suggested.
Every cost-table row in `KE16-VARIANTS-WAKE-LOCALITY-SCHED.md` now carries the class where it is
known.
