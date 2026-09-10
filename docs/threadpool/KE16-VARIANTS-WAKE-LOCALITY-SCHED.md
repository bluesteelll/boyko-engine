# KE16 — Variants, group W: wake protocol and idle registry; group L: locality; group S: scheduler level

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`.
Negative results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. Cost anchors as in
`KE16-VARIANTS-PLACEMENT.md`. **W15–W19** (throttled wake, fan-out count, the completion-path
signal, topology-nearest wake target, user-mode wait) and **L11–L13** (PDF scheduling,
heterogeneous cores, sub-LLC affinity scopes), added in refutation round 1, are in
`KE16-VARIANTS-ADDENDA.md`.

Group W is the owner's mechanism 3 in its *shipped* form — every runtime has an idle registry and
uses it to choose whom to wake; the cost table at the end of the group is how the acceptance
criterion is applied. Group L is the axis the owner named as deciding mechanism 1. Group S is
mechanism 1 itself.

---

## Group W — wake protocol and idle registry

### W0 — Unconditional wake-one-per-push, rotor RMW before the idle load (boyko today)

**Axis cell.** 12: every push — **and** every task completion (`complete_task`'s unconditional
`waker.unpark()`, `scope.rs:157-160`) **and** at the join before every `park_timeout`
(`unpark_one_idle` at `scope.rs:511`) — three triggers, of which the prior synthesis recorded one
(→W17) · 13: `AtomicU64` bitmap + `wake_rotor`, rotating order, CAS-claim · 14: 50 µs backstop +
post-`mark_idle` re-poll · 26: `std` park/unpark · 10: one shared RMW per push even when nobody is
parked; two shared RMWs per completion.

**Implemented by.** boyko `worker.rs::push_task:375` → `unpark_one_idle:320-349`;
`mark_idle`/`unmark_idle:287-300` `[L]`.

**Documented rationale.** `worker.rs:302-319`: the rotor exists because lowest-bit bias "starves
high-id workers"; "exactly one worker is claimed per successful call" (loom M2/M2b) `[L]`.

**Evidence.**
- `wake_rotor.fetch_add(1, Relaxed)` at `:324` precedes `idle.load(Acquire)` at `:326` `[L]` —
  its own comment says the rotor "only spreads the wake target; no data is published through it".
- Go's rejected approach #3, verbatim: "Unpark an additional thread whenever we ready a goroutine
  and there is an idle P, but don't do handoff. This would lead to excessive thread parking/unparking
  as the additional threads will instantly park without discovering any work to do." `[S]`
- Tokio measured the value of removing no-op wakes: hyper hello 118,495 → 135,261 req/s (+14 %),
  mini-redis SET 96,711 → 120,627 (+25 %) — maintainer, official harness, in the PR description `[D]`.
- Rust `std` `unpark`: `state.swap(NOTIFIED, Release)`; `futex_wake` **only if the previous state was
  PARKED** `[S]` — so an ungated wake is nearly free when the target is running and ~1–10 µs when it is
  parked. On a frame-locked load shape (axis 27) workers are parked at almost every wave boundary.

**Failure modes.** Park/unpark churn (N3); a shared line touched on every spawn by every spawner;
under P0 the woken worker cannot reach the work at all.

**Negative results.** N3, N13.

**Cost.** SPAWN: `wake_rotor` RMW + `idle` load unconditionally; CAS + unpark only if a bit is set.
PARK: `fetch_or` + re-poll + `park`; the joiner additionally issues `unpark_one_idle` before each
`park_timeout`. WAKE: CAS loop + `unpark` + on wake `fetch_and`. COMPLETE: `waker.unpark()` swap on
the joiner's parker line + `futex_wake` if the joiner is parked + `pending.fetch_sub` — two shared
RMWs per task (W17).

**Fit.** M3: the defect cell of the wake axis. **Candidate W-a** (E16): move the rotor RMW inside
`mask != 0` — zero behavioural change, one shared RMW per spawn removed.

---

### W1 — Wake gated on the empty→non-empty transition

**Axis cell.** 12: push into an EMPTY queue only · 10: spawn = local push + `is_empty()` load; the
signal CAS only on the transition.

**Implemented by.** Java FJP `signalWork` (activate one worker "when it pushes a task into an empty
queue, resulting in O(log(#threads)) steps to full activation") `[S]`; rayon `WorkerThread::push`
(`queue_was_empty`) → `new_jobs(num_jobs, queue_was_empty)`: wake `min(num_jobs − idle, sleeping)`
when empty, else `min(num_jobs, sleeping)` `[S]`; oneTBB `arena::out_of_work` `my_pool_state
.try_clear_if(!has_tasks())` — one notification per transition `[S]`.

**Documented rationale.** FJP: "signals may be (and often are) unnecessary because active workers
continue scanning after running tasks without the need to be signalled (which is one reason work
stealing is often faster than alternatives)" `[S]`.

**Evidence.** FJP's activation cascade is O(log W) `[S]`. rayon's exact wake-count rule `[S]`. TBB's
state machine `[S]`.

**Failure modes.** A missed signal during deactivation must be closed by a rescan (FJP) or a final
injected-job check (rayon) — W4.

**Cost.** SPAWN: one thread-local `is_empty()` (an Acquire load of the deque's own front/back) in
place of a shared RMW.

**Fit.** M3: **candidate W-b**. Under P1 the gate is natural (the worker's own deque was empty); under
P2/P3 the Injector has `is_empty()` too. Composes with the rotor bitmap unchanged.

---

### W2 — Wake gated on "no searcher exists", with a searcher cap

**Axis cell.** 12: no searcher · 6: a bounded count of searching/spinning workers; the last searcher
to find work must wake a replacement · 10: spawn = local push + StoreLoad barrier + one LOAD of the
counter; CAS only when a wake is needed.

**Implemented by.** Go `wakep` (go1.16.15 verbatim: `if npidle == 0 return; if nmspinning != 0 ||
!Cas(&nmspinning, 0, 1) return; startm(nil, true)`), spinning admission `2*nmspinning < gomaxprocs −
npidle` (master; from search, not read verbatim — U) `[S]`; Tokio `idle.rs::transition_worker_to_
searching` (`2 * num_searching < num_workers`), `worker_to_notify` only when `num_searching == 0 &&
num_unparked < num_workers`, `transition_worker_from_searching` returns "was the last searcher" `[S]`;
Taskflow `_num_thieves`/`_num_actives` `[S]`; .NET `SpuriousDispatchNoSpinThreshold` (2/3 of
processors) `[S]`.

**Documented rationale.** Go, verbatim: "We unpark an additional thread when we submit work if… 1.
There is an idle P, and 2. There are no 'spinning' worker threads… if the last spinning thread finds
work and stops spinning, it must unpark a new spinning thread. This approach smooths out unjustified
spikes of thread unparking, but at the same time guarantees eventual maximal CPU parallelism
utilization." and the required pattern: "Submit work… #StoreLoad-style memory barrier… Check
sched.nmspinning." `[S]` Tokio: "Limiting searchers is only an optimization to prevent too much
contention." `[S]`

**Evidence.** Tokio +14 %/+25 % (W0) `[D]`. Go: "If they both fail to do that, we can end up with
semi-persistent CPU underutilization." `[S]` Go exempts the global queue: "we are not sloppy about
thread unparking when submitting to global queue" `[S]`.

**Failure modes.** The last-searcher race (Tokio closes it by unparking on the transition, accepting
false wakes); needs a StoreLoad barrier on the submit path (a `lock`-prefixed op or `mfence` on x86 —
what TBB refused, W5); the invariant "last spinner must replace itself" hangs the pool if broken.

**Negative results.** N14, N50.

**Cost.** SPAWN: barrier + one load in the busy case. IDLE: a searching state per worker (one more
shared counter). WAKE: CAS + unpark only when no searcher.

**Fit.** M3: **candidate W-b** (alternative to W1). Extends loom M2/M2b (axis 28). Interacts with
W11: a "searching" worker is one that is scanning, so the cap also bounds the O(W²) idle scan.

---

### W3 — Jobs-event counter / sleepy handshake

**Axis cell.** 6: idle → 32 yield rounds → sleepy (announce via JEC) → 1 more round → sleep on a
per-worker `Mutex+Condvar` · 10: the poster increments the JEC only when its low bit says a worker
is sleepy.

**Implemented by.** rayon-core `sleep/mod.rs` (`ROUNDS_UNTIL_SLEEPY = 32`, `ROUNDS_UNTIL_SLEEPING =
33`, `announce_sleepy`, `sleep`, `new_jobs`), `sleep/counters.rs` (sleeping 16 bits, inactive 16
bits, JEC, SeqCst RMWs) `[S]`; `sleep/README.md` `[D]`.

**Documented rationale.** README: the simpler always-increment form "turns out to be too expensive in
practice"; even JEC = "no new work since the last thread got sleepy", odd = "new work posted". For
INTERNAL work a rollover-induced miss means "fewer workers processing the work then we should, but we
won't deadlock"; for EXTERNAL work it can deadlock, hence the final injected-job check `[D]`.
`counters.rs`: the WAKER decrements the sleeping counter "because this reflects state changes faster
to other work-posting threads" `[S]`.

**Evidence.** Old protocol woke ALL threads on any job arrival OR completion; rayon #642 measured
~30 % CPU at 10 ms idle spawns, ~200 % at 1 ms on 4C/8T `[D]` (N15).

**Failure modes.** Per-worker Mutex + Condvar on the wake path; the counters word is one shared line
CASed by every idle transition; 32 yield rounds burn scheduler round-trips on every short gap.

**Negative results.** N15, N16.

**Fit.** M3: the most elaborate shipped answer to "no RMW on the spawn path"; more state than W1/W2
buy. Not shortlisted — W1 gets the same spawn-path property with our existing bitmap.

---

### W4 — Park-path re-check / two-phase-commit notifier

**Axis cell.** 6: prepare_wait → re-check all queues → commit_wait / cancel_wait · 10: moves the
synchronisation from the SPAWN path (every push) to the PARK path (rare).

**Implemented by.** Taskflow `_wait_for_task` with `prepare_wait/commit_wait/cancel_wait` ("any
notify that arrives after this check but before commit_wait will be caught by the 2PC guarantee")
`[S]`; Tokio `transition_to_parked` final `has_tasks()` check ("Workers should not park if they have
work to do") `[S]`; FJP `deactivate` rescan `[S]`; rayon's sleepy-state final check `[S]`; boyko's
post-`mark_idle` re-poll (`worker.rs:101-108`, "load-bearing against Race C") `[L]`.

**Documented rationale.** Put the burden on the thread about to sleep, not the thread about to spawn.

**Evidence.** Taskflow's fast path skips exploration when `_num_topologies == 0` `[S]`.

**Failure modes.** 2PC costs a lock or a versioned counter on the park path; not free if workers park
often — which on a frame-locked load they do.

**Fit.** M3: we are here (the re-poll). Any W1/W2 change must keep it.

---

### W5 — Tolerated lost wakeup, with or without a backstop

**Axis cell.** 14: throughput-only miss tolerated by contract (TBB) / closed by a timeout (boyko) /
correctness-critical (rayon external, Go).

**Implemented by.** oneTBB `arena.h::advertise_new_work` — verbatim: "Double-check idiom that, in
case of spawning, is deliberately sloppy about memory fences… adding such a fence might hurt overall
performance more than it helps, because the fence would be executed on every task pool release, even
when stealing does not occur. Since TBB allows parallelism, but never promises parallelism, the missed
wakeup is not a correctness problem." `[S]` (the fence is skipped only for `work_spawned`;
`work_enqueued` executes `atomic_fence_seq_cst` `[R:S]`); boyko `scope.rs:504-513` `park_timeout(50 µs)`, doc
`:430-431` "the timeout is also the backstop for the rare lost-wakeup window" `[L]`; rayon's
internal/external split `[D]`.

**Evidence.** TBB's contract; boyko's contract (a `Scope` must drain) needs the backstop.

**Failure modes.** A timeout backstop converts a lost wake into a latency spike — under a 16 ms frame
budget a 50 µs spike is 0.3 % of the frame per occurrence; the dispatcher's 100 µs `park_timeout` is
the same class.

**Fit.** M3: we are here. Transplanting TBB's elision is not possible without the backstop we
already have.

---

### W6 — Idle-registry data structure, maintainer, and wake ORDER (boyko's bitmap is here)

**Axis cell.** 13 (all values) · 7: LIFO wake order (warmest thread) vs rotating (fairness).

**Implemented by.** boyko `thread_pool.rs:139-148` (`idle: CachePadded<AtomicU64>`, `wake_rotor`),
`worker.rs:287-349` `[L]`; Java FJP `ctl` — RC (16 bits) / TC (16) / SS+ID (32): idle workers form a
**Treiber stack** threaded through `WorkQueue.stackPred`, popped by one `compareAndExchangeCtl`
`[S]`; Kotlin `parkedWorkersStack` — "intrusive versioned Treiber stack", version bits against ABA,
`tryUnpark` CAS PARKED→CLAIMED `[S]`; .NET `LowLevelLifoSemaphore` — packed `Counts` (SignalCount /
WaiterCount / CountOfWaitersSignaledToWake, one CAS), `_blockerStack` LIFO under a lock `[S]`; Folly
`LifoSem`/`ThrottledLifoSem` `[S]`; Tokio `Mutex<Synced{sleepers: Vec<usize>}>` `[S]`; rayon per-
worker `Mutex<bool>+Condvar` + packed counters `[S]`; Go `pidle` under `sched.lock` + `nmspinning`
`[S]`; async-executor `Mutex<Sleepers{Vec<(id, Waker)>}>` + `notified: AtomicBool` `[S]`; Eigen
`EventCount` (waiter stack 14 bits, prewait 14, signal 14, epoch 22, in one u64: "Notify is cheap if
there are no waiting threads") `[S]`; enkiTS semaphore + `m_NumThreadsWaitingForNewTasks`
(incremented BEFORE the final check) `[S]`; Godot `_notify_threads` (idle first, then awaiting) `[S]`;
X10 lifelines (distributed per victim) `[P]`; flecs, id Tech 5: none.

**Documented rationale.** Kotlin: LIFO "improves both performance and locality" `[S]`. Folly: LIFO for
cache warmth; `ThrottledLifoSem`'s `wakeUpInterval` batches context switches `[S]` — this is the
E3 occupant the prior synthesis missed (→W15). boyko: rotation against low-id
starvation `[L]`. Eigen: "If there are enough active threads with empty pending-task queues, a
thread that runs out of work can just be parked without spinning." `[S]`

**Evidence.**
- **None of the surveyed production runtimes uses a bitmap**; all but boyko wake LIFO or
  OS-chosen `[S]`/`[I]`; a third order — topology-nearest (Linux `select_idle_sibling`) or
  recently-spinning (marl) — exists and composes with the bitmap (→W18). boyko's rotation is an
  unexamined divergence, not a known error.
- The bitmap makes "is anyone idle?" one Acquire load — cheaper than Tokio's packed SeqCst load and
  far cheaper than a sleepers lock `[L]`/`[S]`.
- `MAX_WORKERS = 64` is the bitmap's width; `MAX_EVENT_THREADS = 65` in `boyko_ecs` is tied to it by
  a const-assert and bounds nothing about dispatch `[L]` (probe).
- Every park and every wake is an RMW on ONE shared line: at W=16 with per-wave idling the line
  ping-pongs W times per wave boundary each way `[I]`.

**Failure modes.** Shared line on wake storms; no recency (LIFO) without extra state; 64 cap; the
rotor RMW paid when the bitmap is zero (W0).

**Cost.** PARK: `fetch_or`. WAKE: CAS loop + `unpark`. QUERY: one load.

**Fit.** M3: **the registry exists** — what the owner proposed is changing what the pusher *does*
with it (→P6). Candidate **W-a** lives here; E5 (a most-recently-parked hint) is the cheap way to
recover LIFO order if it is shown to matter.

---

### W7 — Wake-all / broadcast

**Implemented by.** rayon pre-RFC5 ("as soon as any work arrives (or — in fact — even any work
completes) all threads awaken") — **abandoned** `[D]`; flecs `flecs_signal_workers`
(`ecs_os_cond_broadcast` at each sync point) — **by design**, because the wave is a barrier `[S]`;
id Tech 5 `SignalWork()` per selected thread `[S]`.

**Evidence.** rayon #642 (30 % / 200 % CPU) `[D]`.

**Failure modes.** Thundering herd proportional to the job-event rate; doubled in fork-join
(completions too).

**Negative results.** N15.

**Fit.** M3: the far end of the axis; bounded only when the trigger is a barrier (flecs).

---

### W8 — Single spinner / relay ("whipping boy")

**Axis cell.** 6: exactly one worker spins on behalf of the pool; it wakes the rest · 10: moves the
kernel wake cost off the spawn path and off the main thread.

**Implemented by.** Eigen `StartSpinning` (`kMaxSpinningThreads = 1`; spin budget divided by thread
count and spinner count) `[S]`; Schöner's relay worker on Windows (`WaitOnAddress`) `[B]`; Unity
2022.2 per-worker futex chain (blog returned 403 — U) `[B]`.

**Documented rationale.** Eigen: "The time spent in NonEmptyQueueIndex() is proportional to
num_threads_ and we assume that new work is scheduled at a constant rate, so we divide kSpinCount by
number of threads and number of spinning threads." `[S]` Schöner: a profiled game spent "double-digit
percentages of the frame time just unblocking worker threads"; Windows semaphore release grows with
the number of waiters `[B]`.

**Evidence.** Releasing 32 threads from a Windows semaphore is substantially slower than one; violin
plots, no tabulated numbers `[B]`.

**Failure modes.** One core permanently unavailable or burning; pure waste on a fully loaded frame;
the win exists when the pool oscillates between empty and full — which is the ECS lane-drain pattern.

**Fit.** M3: the opposite attack on "who pays the wake" from push-to-idle. Not shortlisted; W2's
searcher cap is the bounded-spinner half of it.

---

### W9 — Silent worker-generated spawn: no notify, the spawner consumes it

**Axis cell.** 12: never, for worker-generated work · 25: nothing crosses until a thief arrives.

**Implemented by.** cbloom's delayed-semaphore rule `[B]`; Eigen `IsNotifyParkedThreadRequired()`
(notify only if no spinner) `[S]`; chili/spice (publish only on a beat) `[S]`/`[B]`.

**Documented rationale.** cbloom: "the work popper can shortcut the delayed sem inc… the delay does
not apply to the work being available to already running worker threads"; for worker-generated work
let the same worker claim it locally `[B]`. FJP: active workers "continue scanning after running
tasks without the need to be signalled" `[S]`.

**Evidence.** cbloom's failure narrative: push A, inc sem, worker wakes, pops A, sees empty, sleeps;
push B — the wake was spent on nothing `[B]`.

**Failure modes.** A wave spawned while every sibling is parked stays on the spawner until the
spawner's own `Scope::drop`; the joiner then wakes one (`scope.rs:511`) — latency, not loss.

**Fit.** A/M3: with P1/P2, the *steal path* is what makes inner-spawn work reachable and the wake is
the expensive half — cbloom's rule says do not pay it per spawn. Compose with W1: wake only on the
first push of a wave (the empty→non-empty transition). Worth measuring in the A-fixed configuration.

---

### W10 — Heartbeat-gated publication / wake

**Axis cell.** 12: a timer · 25: one job per beat.

**Implemented by.** chili `join_heartbeat` (publication gated) `[S]`; spice `[B]`. **Sub-variant
(E3 — occupied, →W15):** publish eagerly to a reachable queue, but let the *unpark* fire at most once
per interval — Folly `ThrottledLifoSem` ships exactly this.

**Documented rationale.** spice: "even spending hundreds of nanoseconds during heartbeat processing
adds minimal cost when occurring only every ~100 microseconds" `[B]`.

**Evidence.** chili: fast path is a Relaxed `AtomicBool` load, checked 1-in-64 `[S]`.

**Failure modes.** Needs a heartbeat thread or timer; latency floor = one beat (~100 µs) per
promotion step; on a 16 ms frame a short wave never fans out.

**Fit.** M3: the wake-only gate bounds the syscall rate without touching reachability and has a
shipped occupant (W15); measure only if W1/W2 leave no-op wakes on the table.

---

### W11 — Spin budget policy

**Axis cell.** 6: how long to spin, and whether the spin is thread-local or a scan.

**Implemented by.** boyko: crossbeam `Backoff` (`SPIN_LIMIT = 6`, `YIELD_LIMIT = 10`: 2⁰..2⁶ = 127
pause hints + 4 `yield_now`, `is_completed = step > 10`) with **`pop_any` inside every round**
(`worker.rs:81-125`) `[L]`/`[S]`; rayon 32 `yield_now` rounds `[S]`; FJP `SPIN_WAITS = 1<<7`, sleeps
1 µs..1 s `[S]`; .NET 1024 calibrated spins ≈ 35 µs ("chosen to be in the range of typical thread wake
latency… a single spin is calibrated to around 35 nanoseconds"), `DefaultWakeCooldown = 4 µs`,
`WaitNoSpin` for probable no-op wakes `[S]`; enkiTS `gc_SpinCount = 10` with backoff multiplier
`[S]`; Go 4 passes then park `[S]`; Taskflow `MAX_STEALS = ((MAX_VICTIM+1) << 1)` then yield then
~150 more `[S]`; OpenMP `KMP_BLOCKTIME` (200 ms cited; page 403 — U).

**Evidence.**
- boyko's idle episode: ≤11 rounds × (2 Injector probes + W−1 Stealer probes); each `Stealer` probe
  pins the epoch (`lock cmpxchg` on x86, `try_advance` every 128 pins) and each `Injector` probe
  pays a full `SeqCst` fence with **no pin** (0.8.7 `deque.rs:650` vs `:1821` `[L]`; U44 closed) —
  ~187 probes per episode at W=16 (**arithmetic from two verified constants, not a measurement** —
  U) `[L]`/`[S]`.
- rayon's `wait_until_cold` calls `find_work()` — `take_local_job` → full random-start steal sweep →
  `pop_injected_job` — on **every** iteration, and `no_work_found` yields once per round for 32
  rounds `[S]` (re-read this session). That is boyko's per-round scan shape, not a decoupled spin;
  the prior E18 text ("every runtime spins thread-locally and scans every k-th round") was an
  untagged inference and is withdrawn. The decoupled form has **no occupant** among the runtimes
  read.
- Go #28808: findrunnable 60.58 s → 115.47 s at GOMAXPROCS=56, `runqsteal` 19.21 → 47.49 s; "the
  work stealing algorithm degenerates to O(N²)"; GOMAXPROCS=12 restored it `[D]`. Go #18237: 26.8–
  35.1 % of cycles in `findrunnable` under frequent wakeups `[D]`.
- `thread::yield_now` on Windows is `SwitchToThread`/`Sleep(0)` — a syscall per yield `[I]`.

**Failure modes.** Coupling the spin to a full steal scan turns a thread-local spin into O(W)
coherence traffic per round; a fixed budget cannot adapt (too short → churn between waves; too long →
burns the frame).

**Negative results.** N5.

**Cost.** IDLE: today O(W) shared probes × 11.

**Fit.** M3: **candidate W-c** (E18) — spin thread-locally, scan every k-th round; W2's searcher cap
bounds the same traffic from the other side. E18 is an **empty cell**, not the field's practice —
it must be justified by measurement, and the prior for it is Go #28808 (N5), not a precedent.

---

### W12 — Worker-count / parallelism feedback

**Implemented by.** A-STEAL/A-GREEDY (steal-cycle ratio classifies each quantum; desire ×ρ or ÷ρ)
`[P]`; BWS (OS-disclosed running status; yield the core to a peer; cap awake thieves) `[P]`; CLR 4.0
(hill climbing → DFT over an injected concurrency wave; "always be off by at least one thread";
works best under 10 ms items) `[D]`; FJP compensation (J5); TBB `adjust_demand` `[S]`.

**Documented rationale.** A-STEAL: when a job gets "a huge number of processors… just when the job
has little instantaneous parallelism… no adaptive scheduling algorithm can effectively utilize the
available processors" `[P]`. CLR: the CPU-utilisation controller was abandoned because utilisation
rose while work fell (paging; lock contention) `[D]` (N34) — **the most direct published support for
the owner's criterion**.

**Evidence.** A-STEAL Theorem 12: `W ≤ ((1+ρ−δ)/δ + (1+ρ)²/(δ(Lδ−1−ρ)))·T1` — parametric (2.75·T1
for the first term alone at ρ=2, δ=0.8; ≈1.3·T1 at ρ=1.2, δ=0.95); "≈2·T1" is a Section-5
instantiation ("generally less than 2T1"), corrected here (N58) `[P]`; "completed jobs about twice
as fast" than non-feedback ABP in simulation `[P]`. BWS: CG +144 %/MM +37 % at 32+32;
halving CG improved both; +12.5 % throughput, unfairness 124 % → 20 % `[P]`.

**Fit.** M1/Sched: irrelevant to a fixed-worker frame-synchronous engine; the transferable content is
the measured **negative** — more awake thieves lost (N47) — which prices E4 and P6.

---

### W13 — OS blocking primitive

**Implemented by.** Rust `std` futex parker (`park`: `fetch_sub(1, Acquire)` then `futex_wait`;
`unpark`: `swap(NOTIFIED, Release)`, `futex_wake` only if PARKED; "even NOTIFIED⇒NOTIFIED results in a
write… to make sure every unpark() has a release-acquire ordering with park()") `[S]`; parking_lot
(global sharded hash table; bucket `WordLock` on EVERY park and `unpark_one`, even with no waiters;
~1 ms eventual fairness) `[S]`; Win32 `WaitOnAddress`/`WakeByAddressSingle` (no kernel object;
spurious wakes permitted — every protocol must loop on a re-checked predicate) `[D]`; rayon
`Mutex+Condvar`; .NET LIFO semaphore; FJP `LockSupport`; Go `note`.

**Evidence.** Downs cost anchors `[B]`. Bevy 10–70+ µs per OS wake `[D]`.

**Fit.** M3: `std` park is the right primitive for a gated wake (no syscall unless parked); parking_lot
would make "wake nobody" cost a lock. We are here.

---

### W14 — Spawn-path allocation

**Axis cell.** 16: heap `Box` per spawn (boyko `scope.rs:339`; rayon `spawn` `HeapJob`) · job in the
caller's frame (rayon `join` `StackJob`, Cilk, chili `Future`) · pooled cell (Tokio, Go `g`) · proxy
in addition (TBB) · fixed-size pooled job 64→128 B (Molecule `[B]`) · fixed rings, no runtime alloc
(Our Machinery, enkiTS `[B]`/`[S]`).

**Evidence.** Kumar et al.: heap-allocated frame/state objects were "just under half" of X10's 4.1×
sequential overhead; moving everything to the steal path cut it to 15 % `[P]`. Mul-T: eager task
creation 0.83× — slower than serial `[P]`. boyko's own doc: "a single `scope.spawn` costs ~120 ns
(plan §10.3)" `[L]`.

**Failure modes.** A `Box` per chunk is an allocation in the hot path (principle 5); a caller-frame job
needs the spawner to stay alive until the join — which `Scope` already guarantees.

**Negative results.** N38, N49.

**Fit.** A/B: with P1 (own deque + TLS pointer) a `par_iter` wave can be a caller-frame array of chunk
descriptors referenced by word-sized handles — the rayon `join` shape. Not on the shortlist by
itself; it becomes available under A1.

---

### The cost table — how the acceptance criterion is applied (all rows from sources read)

| Runtime | SPAWN (busy pool, nobody idle) | PARK (one idle episode) | WAKE (one decision) | COMPLETE (one task) |
|---|---|---|---|---|
| **boyko today** `[L]` | `Box` + `pending.fetch_add` (shared RMW) + `Injector::push` (SeqCst CAS on the tail + slot `fetch_or`, **no epoch pin**) + `wake_rotor.fetch_add` (shared RMW) + `idle.load` — **1 alloc, 4 shared RMWs, unconditional** | ≤11 rounds × (2 injector probes, each a `SeqCst` fence + W−1 stealer probes, each an epoch pin) → `idle.fetch_or` → re-poll → `park`; the joiner also `unpark_one_idle`s before each `park_timeout` | `idle` CAS loop + `unpark` (futex only if PARKED) + `idle.fetch_and` on wake | `waker.unpark()` swap on the joiner's parker line + futex if the joiner is parked, then `pending.fetch_sub` — **2 shared RMWs + a conditional syscall, unconditional** (W17) |
| rayon `[S]` | Chase-Lev push (Acquire load + Release store, no RMW) + `is_empty()` + one counters load; JEC CAS only if a sleepy/idle thread exists; `join` allocates nothing | 32 × `yield_now` (each round a full `find_work` scan) → sleepy CAS → fence → sleeping CAS → per-worker Mutex + Condvar | Mutex + `notify_one` + `sub_sleeping_thread` | latch set: swap + wake only when the owner is SLEEPING (`latch.rs`, `[R:S]`, not re-read) |
| Tokio `[S]` | LIFO slot store or bounded push (Release store); overflow = 1 CAS + 128-task move; batch injection = one lock per batch (`push_batch`, `[R:S]`); notify = one SeqCst load of the packed idle word; sleepers Mutex only on a real wake | Mutex + push id + fetch_sub | Mutex + pop sleeper + unpark | n/a (no join) |
| Go `[S]` | `runqput` stores; `wakep` = one load of `npidle`; CAS on `nmspinning` only if `npidle > 0`; `runqputbatch` moves a batch under one lock (`[R:D]`) | 4-pass scan → `sched.lock` → `pidle` push → `notesleep` | CAS `nmspinning` + `startm` + `notewakeup` | n/a (no join) |
| Java FJP `[S]` | array store + Release; `signalWork` only when the queue appeared empty = one CAS on `ctl` | IDLE phase + CAS onto the `ctl` stack + rescan + `LockSupport.park` | one CAS on `ctl` + `unpark` | status CAS on the task; joiners helped, not signalled per task (not read in detail) |
| .NET `[S]` | enqueue + CAS on packed `Counts` + LIFO semaphore signal | ≤1024 calibrated spins (~35 µs) then block | CAS on `Counts` + pop LIFO stack under a lock | n/a |
| chili / spice `[S]`/`[B]` | local push + one Relaxed bool load, 1 in 64 calls | condvar in a global Mutex context | only every ~100 µs: global Mutex + move one job + notify | a stolen job signals its join via the shared context; an unstolen one is run inline at join (no signal) |
| oneTBB affinity `[S]` | deque push + proxy push to a foreign line; `advertise_new_work` gated on an empty→non-empty transition, fence omitted for `work_spawned` only | monitor wait | demand adjust + `notify(predicate)` | reference-count decrement on the parent task (not read in detail) |
| async-executor / Bevy `[S]` | global-queue push + `notify()` = one `AtomicBool` check; sleepers Mutex only on a real wake | Mutex + register waker | Mutex + take waker + `wake()` | task-state RMW + waker call (not read in detail) |
| flecs `[S]` | none (static partition) | condvar at the sync point | `cond_broadcast` — wake all, bounded by sync-point frequency | one `workers_waiting` increment per worker per sync point |
| Folly `ThrottledLifoSem` `[S]` | semaphore post (one RMW) | LIFO semaphore wait | at most one waiter woken per `wakeUpInterval`; immediate when posts are spaced ≥ the interval (W15) | n/a |
| Jolt `[S]` | O(W) minimum over per-thread head cursors + `Release(min(jobs, threads))` | semaphore `Acquire(max(1, GetValue()))` | semaphore release, count = `min(jobs, threads)` | barrier counter; a waiter runs only its own barrier's jobs (P21) |

Reading: (1) every runtime gates the wake behind something cheap; boyko gates it behind nothing.
(2) boyko's spawn path is the most expensive in the table — the only one with an allocation and the
only one with four shared RMWs. (3) Push-to-idle as *non-stealable* placement is absent from the
table because no production runtime implements it; the stealable form (P17) costs P0's push on a
foreign line and is measurable. (4) Over-waking a RUNNING thread is nearly free (`std` parker);
over-waking a PARKED thread is ~1–10 µs — and on a frame-locked load shape the target is usually
parked. (5) boyko's spin is the shortest in instructions and the most expensive in coherence
traffic — and rayon's per-round scan has the same shape, so the decoupled spin (W-c) is an empty
cell. (6) **The COMPLETE column is new**: for a flat wave it runs as often as SPAWN, and boyko's is
the only row paying two unconditional shared RMWs plus a conditional syscall per completion (W17).

---

## Group L — locality and victim selection

### L1 — LIFO owner / FIFO thief

**Implemented by.** Universal (Cilk, rayon, FJP, TBB, libomp, Molecule, enkiTS, Eigen `PushFront`).
**boyko uses `Worker::new_fifo()`** (`thread_pool.rs:595`) — FIFO at both ends `[L]`.

**Documented rationale.** oneTBB: depth-first execution "strikes when the cache is hot, minimizes
space, and creates only a linear number of nodes"; breadth-first stealing "converts potential
parallelism into actual parallelism" `[D]` (near-verbatim via search index — U). Molecule: "The
private end can work in LIFO fashion for better utilization of the cache, while the public end works
in FIFO fashion for better work balancing." `[B]`

**Evidence.** ABB's mailbox measurements presuppose it `[P]`; Tokio's LIFO slot is its capacity-1
extreme (→P10).

**Failure modes.** The owner's LIFO end is the hot end; a thief takes the cold end (fine); for a flat
wave of equal chunks the order is immaterial to locality, so the effect is small there and real for
nested physics scopes.

**Fit.** Loc: **App-2** — one constructor argument; uncosted here (E19).

---

### L2 — Victim order: random start + rotation vs fixed 0..n vs coprime stride

**Implemented by.** rayon (XorShift64Star, `(start..n).chain(0..start).filter(i != self)`) `[S]`;
boyko worker loop (`worker.rs:245-250`, randomised, self-skipped) `[L]`; **boyko joiner
(`scope.rs:535`, fixed 0..n, self included)** `[L]`; Go (randomised order, 4 passes) `[S]`; Eigen
(random start + coprime stride "we will cover all threads without repetitions") `[S]`; enkiTS
(`Hash32(rndSeed*threadNum)`) `[S]`; FJP (pseudorandom stride guaranteeing coverage) `[S]`.

**Documented rationale.** enkiTS: "To prevent many threads checking the same task pipe for work we
pseudorandomly distribute the starting thread" `[S]`. Blumofe & Leiserson's bound is proved for
uniform random `[P]`.

**Evidence.** The dispatcher-side helper always drains worker 0 first `[L]`. Suksompong et al.: biasing
victim choice has a proven asymptotic price (→L5).

**Failure modes.** Random wastes probes on empty victims at the tail; fixed order is biased and, in
our joiner, reaches the joiner's own deque (A.4).

**Fit.** Loc/B: **App-3** — randomise and self-skip the joiner's sweep (E20).

---

### L3 — Sticky / last-victim / re-poll the same queue

**Implemented by.** Taskflow `_explore_task` (sticky victims) `[S]`; FJP ("re-poll from the same queue
after a successful poll… reduces bookkeeping, cache traffic, and scanning overhead") `[S]`; FTL
`HiPriLastSuccessfulSteal` "to improve cache locality" `[S]`.

**Failure modes.** Two thieves lock onto one victim.

**Fit.** Loc: cheap; an addition to L2, not a candidate on its own.

---

### L4 — Hierarchical / NUMA-aware victim selection and arenas

**Implemented by.** HotSLAW HVS `[P]`; NUMA-WS (socket-biased steals, lazy cross-socket mailbox
pushes, Z-Morton layout) `[P]`; HPX radial victim list (same core → same NUMA domain → nearest
neighbouring domain only) `[S]`; oneTBB `task_arena::constraints` (`numa_id`, `core_type`,
`max_threads_per_core`) `[D]`; Quintin & Wagner, Qthreads Sherwood, LAWS (not opened — U).

**Documented rationale.** NUMA-WS: randomized stealing "doesn't distinguish between nearby and
distant work items", causing WORK INFLATION `[P]`. HPX: "limit cross-NUMA traffic" `[S]`.

**Evidence.** HotSLAW steal latency intra-socket ≈2 µs, inter-socket ≈3.8 µs, inter-node ≈36 µs; up to
+52 % vs random at 256 cores `[P]`. NUMA-WS 32-core 4-socket: cg inflation 2.33× → 1.21×, speedup
13.1× → 25.8×; heat 5.24× → 2.25×, 6.0× → 14.0×; hull1 only 7.71× → 9.04×; strassen unchanged `[P]`.
**"Confining stealing within one socket reduced performance versus whole-node stealing" on an
8-core Nehalem** `[P]`.

**Failure modes.** Buys nothing without matching data placement; on a single-socket desktop the
relevant hierarchy is SMT pairs / L2 clusters / per-CCD L3s — **unrecorded for the target** (axis
30), so the prior text's "collapses to L3-shared" was an assumption, not a measurement (→L13);
P/E-core heterogeneity can make a static hierarchy wrong (→L12).

**Negative results.** N44.

**Fit.** Loc: the relevant hierarchy for us is L1/L2/L3 sharing and sub-LLC clusters, not sockets;
the kernel measured strict L3-scope affinity losing ~17 % of bandwidth under light load on a
12-core / 4-L3 Ryzen (L13, N54). Not a candidate until axis 30 is recorded; then an L3-scope
*preference* with a non-strict escape is the shape to measure.

---

### L5 — Steal-back / localized work stealing

**Implemented by.** Analysis only: Suksompong, Leiserson, Schardl arXiv:1804.04773 `[P]` (ar5iv /
abstract).

**Evidence.** Unconditional bound `T1/P + O(T∞·P)` — a factor P worse in the span term; recovers to
`T1/P + O(T∞·lg P)` only under an even-distribution assumption; linear speedup requires
`P ≪ √(T1/T∞)` `[P]`; "up to 80 %" improvement claimed in experiments `[P]`.

**Fit.** Loc: the proven price of restricting victims for locality — any L-variant pays some version
of it. Not a candidate.

---

### L6 — Space-bounded / footprint-to-cache anchoring

**Implemented by.** Simhadri, Blelloch, Fineman, Gibbons, Kyrola SPAA'14 `[P]`.

**Evidence.** 25–65 % fewer L3 misses on most of 7 benchmarks; running time: memory-intensive up to
+25 % (+50 % under constrained bandwidth); **compute-intensive: work stealing remains faster**; "up
to 7 % additional scheduler and load-imbalance overhead"; work stealing "split[s]" a shared cache
rather than sharing it `[P]`.

**Fit.** M1/Loc: the direct answer to "handing a whole system to another lane moves its working
set", and the record says it wins only on memory-bound work. Whether an ECS system's working set is
on that side is an empirical question about our archetypes — the instrument does not exist (L10).

---

### L7 — Cache-sized split

**Implemented by.** Molecule `DataSizeSplitter(32*1024)` ("ranges are only split as long as their
working set no longer fits into the L1 cache") `[B]`; oneTBB `affinity_partitioner` caveats `[D]`.

**Fit.** Loc/application: a byte-sized grain needs the per-item footprint — free for an archetype
query (sum of accessed column widths), unavailable to a generic library. A `par_iter` knob, not a
pool variant.

---

### L8 — Stable entity→thread partition across sync points

**Implemented by.** flecs (P13) `[S]`/`[D]`.

**Documented rationale.** "the same entity is always processed by the same thread, until the next
sync point"; commands queue to a per-thread stage "without locks" `[D]`.

**Failure modes.** Correctness of flecs's stages depends on the invariant, so a steal fallback
cannot be retrofitted there; skew idles N−1 threads.

**Fit.** M1/Loc: the ECS-native form of the locality objective; E9 (partition + steal fallback) is
**shipped at loop level** by libomp `static_steal` (→G18) and originates in Markatos & LeBlanc's
affinity scheduling — "blocked on L10's instrument" was a choice, not an absence of precedent.

---

### L9 — Cache-line padding and false sharing

**Implemented by.** crossbeam `CachePadded` (128 B on x86_64: the spatial prefetcher pulls line
pairs) `[D]`; boyko pads `injector_global`, each `injector_local`, `idle`, `wake_rotor`,
`active_scopes`, `shutdown`, each `WorkerHandle` `[L]`; TBB `mail_outbox` "Padded to occupy a cache
line" `[S]`; arXiv:1103.4142: block misses cost O(B) per stolen task, total delay O(S·B) `[P]`.

**Fit.** Already have; note `idle` and `wake_rotor` are padded from each other but every spawner
writes the same `wake_rotor` line (W0).

---

### L10 — Working-set migration cost of handing over a system (the mechanism-1 axis)

**Axis cell.** 7 × 8: what moving a whole system between lanes costs in L1/L2.

**Evidence that exists.** ProWS deviation bounds `Ω(C·P·T∞ + …)` extra misses for deque adoption
`[P]` (J7); space-bounded 25–65 % fewer L3 misses vs work stealing's cache splitting `[P]` (L6); TBB:
affinity pays only when the same loop re-runs over the same in-cache data `[D]`; Go: "we want to
preserve dependent goroutines on the same thread" `[S]`; BEAM migrates rarely and on a measured
metric `[D]` (S5).

**Evidence that does not exist.** In this tree: `pin_workers` is a stub (`thread_pool.rs:551-557`),
`self.affinity` is touched only to silence an unused-field warning (probe), and there is no
instrument for L1/L2 residency of a system's working set. **Any claim that handing system S to lane
B costs a migration is unfalsifiable at this checkout.**

**Fit.** M1: the owner named this axis as deciding mechanism 1. It cannot be decided; it can only be
instrumented. §H.3.

---

## Group S — scheduler level (mechanism 1)

### S0 — Dynamic hand-out via the global-injector race, gated by conflicts and the apply window (boyko today)

**Axis cell.** 8: scheduler · 2: receiver (first worker to reach `worker.rs:68`) · 24: conflict
bitsets at build · 21: `WORKER_ID_DISPATCHER` spawns, workers race.

**Implemented by.** boyko `schedule_builder.rs:396-403` (kind only, no lane), `schedule.rs:992-1045`
(ready scan), `:1128` (exclusive inline), `:1272` (`scope.spawn` → `injector_global`), `:602`
(apply-window gate), `:722` (`running.set(i, false)` only in the drain) `[L]`.

**Evidence.** Assignment is **dynamic** — no per-system lane binding exists (probe, re-read `[L]`).
The drain fires only when `pending == running.count_ones() || running == 0`; a finished system stays
"running" until the round drains (index A.5) `[L]`. Inter-system: 25.1 % top lane for four
conflict-free systems `[L]`.

**Failure modes.** The apply-window barrier idles a free worker whose only runnable successor waits
on a predecessor that has finished but not been drained; the dispatcher is a parked core (J11).

**Fit.** M1: the owner's precondition is answered (dynamic). The binding gate is the apply window,
not a missing hand-out.

---

### S1 — Ready-set executor spawning each system as a task

**Implemented by.** Bevy `multi_threaded.rs` (`spawn` / `spawn_on_scope` / `spawn_on_external`;
`is_apply_deferred` applied inline "reducing overhead") `[S]`; conflicts precomputed once from
`Access` `[D]`; PR #11801 (poll the executor once inline before spawning it: helped
AssetEvents/UpdateAssets/PreUpdate, "hurts a little because the main thread is more likely to go to
sleep") `[D]`; discussion #8304 (10–70+ µs OS wakes; "death by a thousand cuts"; a mutex-based
coordination was 5–10 % slower and dropped) `[D]`; issue #4718 (10–15 % CPU vs 0.3 % on a near-idle
app, closed unresolved) `[D]`.

**Fit.** M1: what we are, with the apply-window difference. Bevy has no round barrier — each
completion decrements successors — which is the shape S6 proposes.

---

### S2 — Cost-weighted static partition of systems to lanes

**Implemented by.** **No game engine.** HEFT/CPOP list scheduling (Topcuoglu, Hariri, Wu IEEE TPDS
2002 — abstract only, U): upward-rank priority + earliest-finish-time processor selection `[P*]`.

**Failure modes.** Needs a per-system cost estimate (nobody surveyed measures system cost at runtime);
static partition reintroduces the P13 imbalance; moves working sets by design (L10); Bevy #4718 shows
per-system spawn overhead can dominate when systems are tiny, and flecs refuses the primitive
outright.

**Fit.** M1: empty cell E8. Not before an instrument.

---

### S3 — Systems are not the scheduling primitive; parallelise inside each system

**Implemented by.** flecs (#1590: "individual systems are a bad scheduling primitive since they are
too small & add too much scheduling overhead in large applications") `[D]` (N25).

**Fit.** M1: the contrary position, stated by the maintainer of the closest peer ECS. Both flecs's and
Bevy's positions are stated without a shared benchmark.

---

### S4 — Main-thread-only dependency-graph scheduling; nested spawn forbidden

**Implemented by.** Unity Jobs / DOTS: "You can only call Schedule and Complete from the main thread"
`[D]`; "If you were to call JobHandle.Complete that leads to impossible to solve job scheduler
deadlocks… every single case has resulted in tears" `[D]` (N23); scheduling a job from a job forbidden
for determinism; replacement "schedule conservatively, exit early" with `IJobParallelForBatch` `[D]`.

**Fit.** Sched: the second contrary position — the largest shipped ECS forbids the nested case we are
fixing. The reasons are determinism and deadlock, not throughput.

---

### S5 — Periodic migration / load compaction, distinct from stealing

**Implemented by.** Erlang/BEAM `check_balance` (migration paths against an average max queue length;
every 2000·CONTEXT_REDS reductions; load compaction vs utilisation balancing since OTP 17) — theBeamBook,
not ERTS source (U) `[D]`; .NET queue reassignment above 32 processors (`(ProcessorCount+15)/16`
assignable queues, `TryReassignWorkItemQueue`) `[S]`.

**Documented rationale.** theBeamBook: stealing is "quite fast and can be done on every iteration";
migration is "a more elaborate" strategy at a frequency orders of magnitude lower `[D]`.

**Fit.** M1: the closest production precedent for "an idle lane takes another system" is deliberately
low-frequency and metric-driven, running *underneath* per-iteration stealing. Linux CFS is the
receiver-initiated form — hierarchical PULL by the idle/periodic balancer with a 500 µs cache-hot
threshold (`sysctl_sched_migration_cost`, →L13 `[R:S]`) — distinct from BEAM's third-party balancer.
Nothing here does opportunistic system-granularity handoff.

---

### S6 — Apply-window early release

**Axis cell.** 8: scheduler · 20: readiness updated per completion, deferred application still per
round.

**Implemented by.** **Nobody in this exact form** (Bevy has no round barrier to relax). Proposed here
from the probe's finding (index A.5): when a system completes, clear its `running` bit and decrement
its successors' `pred_remaining` immediately, while `apply_window_drain` (the `&mut world` exclusive
apply of deferred commands, SCH7) keeps its current gate.

**Evidence.** The gate at `schedule.rs:602` and the single site of `running.set(i, false)` at `:722`
`[L]`; the SAFETY block at `:604-611` ties the exclusive borrow to the drain, not to readiness `[L]`.

**Failure modes.** A successor that reads what its predecessor *deferred* (commands applied at the
drain) would observe the pre-drain world — readiness must be gated on "no pending deferred writes the
successor can see", i.e. the conflict graph must include deferred-command targets, or early release
applies only to successors whose dependency is on direct component access. This is the design
question the architect owns.

**Fit.** M1: **candidate M1-b**, measured after M1-a establishes the barrier's share.

---

### S7 — Heartbeat at the system level

**Implemented by.** Nobody (HBC promotes C loops; TPAL an assembly machine). Empty cell E7.

**Fit.** M1: promote a running system to parallel only when a beat coincides with an idle lane. Needs
a promotable representation of a system body; ours are opaque closures. Not now.

---

### S8 — Emit a DAG, host schedules

**Implemented by.** EnTT `organizer` ("The resulting tasks are not executed in any case"); the
author's recommended intra-system parallelism is a static range split; thread-local registries
merged later "not a good idea" `[D]`.

**Fit.** Sched: pushes the whole problem to the host; no data for the decision.

---

### S9 — Caller-aware lane count for the physics sites (application level)

**Axis cell.** 21: the routing key the pool already has, used by the consumer.

**Implemented by.** Nobody yet. Today: `lanes = pool.num_threads() + 1` at `solver/colored.rs:2639`,
`soft/colored.rs:995`, `resources.rs:1494`; `if lanes < 2 { return false; }` dead at
`resources.rs:1495` `[L]`.

**Evidence.** On the production caller the lane pool is **1** today and **W** after any A-fix — never
`W + 1`, because the frame-path dispatcher is parked (J11) and the route-(b) joiner is one of the W
workers `[L]`. On the bench path (route (a)) the joiner is an extra lane, which is where the `+ 1`
came from. The guard was written for "one effective lane", which is exactly the production condition,
and it asks `num_threads()` instead of `current_worker_id()`.

**Fit.** A (application): **App-1** — `lanes = if on_worker { W } else { W + 1 }` or simply `W`;
delete or re-aim the guard; then `n_chunks = clamp(lanes × CHUNKS_PER_WORKER, 1, n)` is right on both
routes. Independent of which A-candidate wins.
