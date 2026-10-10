# KE16 — Evidence registers: negative results, unverified claims, sources

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`. The
variant files cite entries here by `N…` and `U…`.

Two rules, binding on every entry: (1) a design documents, a blog claims — the two are never merged;
(2) nothing a lens marked unverified is silently promoted. Where two lenses disagreed on a
verification level, the lower level is recorded.

## N. Negative-results register — what was tried, capped, or abandoned, and the documented reason

Grouped by who abandoned it. `ref` is the primary source a lens read; the tag is its verification
kind.

**Go runtime (`go/src/runtime/proc.go`, "Worker thread parking/unparking" comment — read verbatim at go1.16.15 and master `[S]`)**

| # | Design | Why (documented) |
|---|---|---|
| N1 | Centralise all scheduler state | "would inhibit scalability"; scheduler state is "intentionally distributed… so it is not possible to compute global predicates on fast paths" |
| N2 | **Direct goroutine handoff** — unpark a thread and hand it the work (push-to-idle as placement) | "thread state thrashing, as the thread that readied the goroutine can be out of work the very next moment… it would destroy locality of computation as we want to preserve dependent goroutines on the same thread; and introduce additional latency" |
| N3 | Unpark an extra thread on every ready when an idle P exists, no handoff (**boyko's `push_task` today**) | "excessive thread parking/unparking as the additional threads will instantly park without discovering any work to do" |
| N4 | Pre-1.1 single global runqueue under `Sched.Lock` | Vyukov design doc `[D]`: global mutex contention; handoff inefficiency; poor locality ("G's often move between M's unnecessarily") |
| N5 | Unbounded O(W) work stealing at high GOMAXPROCS — a measured regression, not an abandonment | #28808 `[D]`: findrunnable 60.58 s → 115.47 s at GOMAXPROCS=56, `runqsteal` 19.21 → 47.49 s, "degenerates to O(N²)", GOMAXPROCS=12 restored; #18237 `[D]`: 26.8–35.1 % of cycles in `findrunnable` |
| N50 | Unbounded spinning-thread sets | capped at GOMAXPROCS (and by Tokio at half): "parking excessive running worker threads to conserve CPU resources and power" |

**Java ForkJoinPool (`ForkJoinPool.java` Implementation Overview `[S]`)**

| # | Design | Why |
|---|---|---|
| N6 | **Work dealing** — producers assign tasks to idle threads | "Work-stealing based on randomized scans generally leads to better throughput than 'work dealing'… in part because threads that have finished other tasks before the signalled thread wakes up can take the task instead" |
| N7 | Eager compensation thread on every blocked join | "the vast majority of blockages are transient byproducts of GC and other JVM or OS activities that are made worse by replacement by causing longer-term oversubscription"; compensation only when the pool could stall |

**Tokio (`tokio.rs/blog/2019-10-scheduler` `[B]` maintainer post; source `[S]`; issues/PRs `[D]`)**

| # | Design | Why |
|---|---|---|
| N8 | crossbeam Chase-Lev deque with epoch reclamation as the local queue (**boyko's substrate**) | "expensive memory reclamation overhead" — atomic RMW in the hot path; replaced by a fixed ring with a packed head; "I was over eager in using atomics in code paths where a mutex would have done just fine" |
| N9 | Bounded MPMC queue as the local queue (tried during the rewrite) | "underperformed due to excessive synchronization in both push/pop" |
| N10 | One lock-free MPMC queue for everything | "the overhead needed to correctly avoid locks is greater than just using a mutex" |
| N11 | Unbounded / growable per-worker queues | replaced by fixed 256 + overflow half to the global queue |
| N12 | **Non-stealable LIFO slot** — acknowledged defect, NOT removed; opt-out shipped | #4941 "make the LIFO slot… stealable" (open); PR #4936 `disable_lifo_slot` "a stop-gap"; docs: "can result in lower total throughput when tasks tend to have longer poll times"; #4323 ping-pong starvation → `MAX_LIFO_POLLS_PER_TICK = 3` |
| N13 | A worker entering "searching" when woken by the I/O driver | PR #4383: no-op wakeups under partial load; fix reduced them up to 50 %; hyper hello +14 %, mini-redis SET +25 % (official harness, maintainer-reported) |
| N14 | "Wake only if no searcher" with no compensating wake | the last searcher can leave without seeing skipped-notify work; kept with an unpark on the searching→not-searching transition, accepting false wakes |

**rayon (RFC 5 `[D]`; `sleep/README.md` `[D]`; source `[S]`; #642 `[D]`)**

| # | Design | Why |
|---|---|---|
| N15 | **Wake all threads on any job arrival or completion** ("tickle") | RFC 5: "as soon as any work arrives (or — in fact — even any work completes) all threads awaken"; "does not track which thread is waiting on what latch"; "Threads can only go to sleep one at a time"; #642: ~30 % CPU at 10 ms idle spawns, ~200 % at 1 ms on 4C/8T |
| N16 | Unconditional atomic increment of a jobs counter per posted job | README: "turns out to be too expensive in practice" → JEC incremented only when its low bit says a sleepy thread exists |
| N17 | Unconditional push of a `Scope::spawn` job to the local deque | "Since `Scope` implements `Sync`, we can't be sure that we're still in a thread of this pool" → `inject_or_push` (identity check, then local push) — the same check boyko performs, with the opposite destination |

**async-executor / Bevy (source `[S]`; issues `[D]`)**

| # | Design | Why |
|---|---|---|
| N18 | Local-queue push for nested spawns | never implemented — "TODO: If possible, push into the current local queue and notify the ticker"; Bevy therefore has no nested-spawn locality path and no defect-A hole |
| N26 | rayon as Bevy's task backend (#318, PR #384) | "long-standing performance issues with smaller workloads on machines with more than a few cores"; "not async-friendly"; "somewhat of a closed box… more difficult to upstream changes that are tuned for games" |
| N27 | (logged gaps, not abandonments) bevy_tasks vs rayon (#10064); ParallelExecutor idle overhead (#4718); nested scopes (#4466) | #10064: executor "spins a little more", "wakes new threads slower as async executor limits to waking one thread at a time", no work-first split; #4718: 10–15 % CPU vs 0.3 % single-threaded, closed unresolved; #4466: many_cubes ~2 % slower, many_lights ~2 % faster, "workload dependent" |
| N28 | Mutex-based coordination inside the multithreaded executor (#8304) | "5-10% slower, so I ended up not pursuing it much further" (hymm); the pursued alternative avoids a 10–70+ µs OS wake |

**oneTBB (source `[S]`; user guide / migration guide `[D]`)**

| # | Design | Why |
|---|---|---|
| N19 | A full fence between releasing a spawned task and reading the wake state | `advertise_new_work`: "the fence would be executed on every task pool release, even when stealing does not occur. Since TBB allows parallelism, but never promises parallelism, the missed wakeup is not a correctness problem" |
| N20 | The public `tbb::task` API incl. `set_affinity`/`note_affinity` (oneTBB 2021) | "considered complex and hence error-prone, which was the primary reason it had been removed"; the internal mailbox (`src/tbb/mailbox.h`) survives in master |
| N21 | Unrestricted helping from a waiting thread (mitigated, not removed) | `this_task_arena::isolate` added because "a thread waiting for a group of tasks to complete might execute other available tasks" and break thread-local state — `assert(ets.local()==i); // May fail!` |

**Game engines**

| # | Design | Who / why |
|---|---|---|
| N22 | **Busy-waiting** (a waiting thread runs unrelated pool tasks) | Epic, UE 5.5 `[D]`: "spinning if no task were available, wasting precious CPU resources and battery lifetime"; "caused deadlocks regularly because it was picking unrelated tasks to run that could themselves have a dependency on the task currently waiting"; "Special care was needed to exclude long running tasks… otherwise it would cause stutters"; "prone to stack overflow". Replaced by oversubscription |
| N23 | `JobHandle.Complete` from inside a job; scheduling a job from a job | Unity `[D]`: "impossible to solve job scheduler deadlocks… every single case has resulted in tears and us reverting such patterns"; "no way to guarantee determinism"; replacement: schedule conservatively, exit early (`IJobParallelForBatch`) |
| N24 | Per-thread queues + work stealing | Bitsquid `[B]`: not adopted — "at our current level of task granularity… the global task queue should not be a bottleneck"; kept one priority heap under a critical section with helping waiters |
| N25 | Systems as the scheduling primitive | flecs, Mertens #1590 `[D]`: "individual systems are a bad scheduling primitive since they are too small & add too much scheduling overhead in large applications" |
| N29 | Lock-free MPMC FIFO under a fiber job system | Our Machinery `[B]`: "failed miserably" → spin-lock |
| N30 | Waiting on a task from inside another pool task | Godot `[S]`/`[D]`: returns `ERR_BUSY` when "there's potential for deadlocking (e.g., the task to await may be at a lower level in the call stack)" |
| N31 | Quiescence by helping alone | enkiTS `WaitforAll` `[S]`: posts a dummy pinned task to another worker; "Otherwise, we have to busy wait" |
| N32 | Naive semaphore-per-pushed-item wake | cbloom `[B]`: wake spent on a worker that finds nothing; remedy: delayed increment, and no post at all for worker-generated work |
| N33 | Waking all workers from the spawning thread on Windows | Schöner `[B]`: "double-digit percentages of the frame time just unblocking worker threads"; remedy: one spinning relay worker, or `WaitOnAddress`/`WakeByAddress` |

**.NET CLR (MSDN Magazine 2010-09 `[D]`)**

| # | Design | Why |
|---|---|---|
| N34 | **Concurrency control driven by observed CPU utilisation** | "wasn't appropriate because the criteria to evaluate the metric can be misleading": under paging CPU% drops and adding threads lowers it further; under lock contention "the CPU time is really being spent doing synchronization, not doing actual work, so adding more threads would just make the situation worse" — the strongest external support for *throughput, not occupancy* |
| N35 | Raw throughput hill climbing | "the throughput constitutes only a small part of what the real observed output is — most of it is noise"; replaced by a DFT over an injected concurrency wave, itself "always… off by at least one thread" |

**Rust (RFC 230 `[D]`)**

| # | Design | Why |
|---|---|---|
| N36 | The M:N green-thread runtime in std (libgreen/librustrt) | segmented stacks "significant performance and complexity cost"; trait-object I/O bloat; TLS penalised by supporting both models; "std::io objects created on a native task cannot safely be used within a green task". Lightweight M:N for data parallelism was explicitly endorsed |

**HPC / academic (papers `[P]` unless marked)**

| # | Design | Who / why |
|---|---|---|
| N37 | **NUMA-aware remote PUSH of tasks applied uniformly** (sender-initiated, the measured instance of P6) | XGOMP arXiv:2502.05293 `[P]` (HTML): "pushes more tasks away, incurring a cost of more than 100 ns for each task that could otherwise be self-executed within nanoseconds" on Fib (10–80-cycle tasks); retained only for tasks > 10⁴ cycles (~4×) |
| N38 | Eager task creation; load-based inlining | Mohr, Kranz, Halstead: eager fib-20 0.83× on 16 processors (slower than serial); load-based inlining: T hand-tuned, starvation when an inlined task blocks, actual deadlock (find-primes), 20–30 % of tasks still created, useless on iterative parallelism — the decision is irrevocable |
| N39 | Eager binary splitting with a per-loop stop-splitting threshold; naive lazy scheduling under nesting | Tzannes et al.: sst depends on thread count, iteration count and calling context, "extremely tedious"; LBS +38.9 %/+16.2 %/+56.7 %/+19.5 % vs the TBB partitioners. DF-LS "pushes deeply nested tasks…", "fails to scale beyond ~8 workers" → BF-LS, DF2-LS (hysteresis). Lens 4 could not open the PDF; lens 1 read it via proxy; Unity's official "start at 1 and increase" `[D]` is the independently verified form of the tuning burden |
| N40 | A fixed work-first policy; a fixed help-first policy | Guo et al. IPDPS'09 / SLAW: work-first overflows stacks (spanning tree beyond 62.5K nodes) and serialises flat distribution (FJ(1024) 4.6× slower); help-first has no space bound and loses on fine fib(35) by 10.2× → adaptive per-site switching |
| N41 | ABP's fixed array; sequentially-consistent atomics throughout Chase-Lev | Chase & Lev: the original "failed to complete due to an overflow" at no performance cost to fix; Lê et al.: SC everywhere ≥1.5× slower than barrier-optimised on x86 and ARM |
| N42 | A store-load fence on the owner's pop path | attacked by four independent lines: Morrison & Afek (bounded TSO; fence ≈ up to 25 % of single-threaded time), Michael/Vechev/Saraswat (idempotence), Acar/Charguéraud/Rainey (private deques; "Cilk's work-stealing protocol spends half of its time executing the memory fence"), Dinan/Lace/LCWS (split deques) |
| N43 | Leapfrogging / depth-restricted helping alone | Lace: uts T3L 20× with leapfrogging alone, 36× with random stealing added; Sukha SPAA'09 proves a lower bound (could not open — U) |
| N44 | Confining steals to one socket; large chunks on structured graphs; steal-one at scale | HotSLAW: "Confining stealing within one socket reduced performance versus whole-node stealing" on 8-core Nehalem; "performance degrades severely with ChunkSize ≥ 8" on fib/nqueens/UTS-T1L/T2L; Dinan: steal-one "degrad[es] past 128 processors" |
| N45 | Biasing victim choice toward locality (steal-back) without an extra assumption | Suksompong, Leiserson, Schardl: `T1/P + O(T∞·P)` unconditionally — a factor P worse in the span term |
| N46 | Private deques, split deques, space-bounded scheduling, place-partitioned locality — as universal wins | Acar et al.: matmul −18 % vs concurrent deques; LCWS: +3.8 %/+1 %/+1.3 % average, worst −102 %; Simhadri et al.: 25–65 % fewer L3 misses and still slower on compute-bound (7 % overhead); SLAW disabled cross-place stealing "to prevent counterproductive theft" |
| N47 | **Maximising the number of awake worker/thief threads** | BWS: CG at 32 workers alongside MM: CG +144 %, MM +37 %; CG at 16 "improve[d] the performance of both"; A-STEAL: waste of over-allotment bounded parametrically by Theorem 12 (the "≈ 2·T1" of the first synthesis was a Section-5 instantiation — N58); "no adaptive scheduling algorithm can effectively utilize the available processors" when instantaneous parallelism is low |
| N48 | Sender-initiated deals without a randomised delay; receiver-initiated when transfers are expensive | Acar et al.: fairness breaks → Poisson-distributed deal attempts (a knob receiver-initiated does not need); Eager/Lazowska/Zahorjan: sender-initiated "uniformly better" only when a *running* task must migrate — not our case (scanned PDF, partially verified — U) |
| N49 | Heartbeat / dynamic promotion on regular balanced workloads; heap-allocated frames on the spawn path | Su et al. ASPLOS'24: "HBC underperforms on balanced workloads; OpenMP static scheduling remains superior"; polling +58.46 % on spmv-arrowhead. Kumar et al. OOPSLA'12: heap frames "just under half of the total overhead" of X10's 4.1× |

**Added in refutation round 1 (2026-09-01)**

| # | Design | Who / why |
|---|---|---|
| N51 | Pool-wide `breadth_first` FIFO configuration (boyko's `Worker::new_fifo` deques are this configuration) | rayon `[R:D]`: `#[deprecated(note = "use scope_fifo and spawn_fifo for similar effect")]`; RFC 0001: "quite surprising behavior when one intermingles scope and join" including stack overflows (rayon #590); replaced by a per-worker FIFO reached through a placeholder on the deque (→P18), measured "performs equivalently" |
| N52 | O(`jl_n_threads`) broadcast wake | Julia `src/scheduler.c` `[R:S]`: replaced by "at most one sleeping thread" — a third shipped wake-all abandonment beside rayon (N15) and Unity (N53) |
| N53 | Job-completion wakes that "wake all other threads, all the time" | Unity 2022.3.62f1 / 6000.0.48f1 (Dale Kim, Unity staff, discussions.unity.com) `[R:D]`: removing redundant wakes gave "up to a 1.15x speedup" on internal and one customer project; users had been REDUCING the worker count as a workaround — a shipped-engine instance of N47. Harness not published; post not re-opened here |
| N54 | Strict cache-scope affinity; `numa` as the unbound-workqueue default | Linux `docs.kernel.org/core-api/workqueue.html` `[D]` (re-read): on a Ryzen 9 3900x (12 cores, 4 L3s) `cache (strict)` delivered 828.20 MiBps vs `system` 993.60 under light load (4 issuers) — "work-conservation" loss ≈17 %; at saturation `cache` 1166.40 ≈ `system` 1159.40; the default moved from `numa` to `cache_shard` (non-strict) |
| N55 | Immediate compensation on `MAY_BLOCK` | Chromium `thread_group_impl.cc` `[S]`: replaced by a threshold (`kBackgroundMayBlockThreshold = Seconds(10)`) observed by a poll (`Seconds(12)`) — "execution throughput should not be reduced forever if a task blocks forever"; foreground values are runtime params |
| N56 | Nested parallelism in a Rust fork-join pool | ForkUnion README `[R:B]`: banned outright — the second shipped Rust instance of Unity's S4 position |
| N57 | One condvar for all idle workers | Halide `thread_pool_common.h` `[S]`: split into A-team / B-team with a target size plus a separate semaphore-waiter channel, "without a thundering herd of genuinely-idle workers waking only to rescan and go back to sleep" |
| N58 | (correction, not an abandonment) A-STEAL "waste ≈ 2·T1" | Theorem 12 is parametric: `W ≤ ((1+ρ−δ)/δ + (1+ρ)²/(δ(Lδ−1−ρ)))·T1`; "generally less than 2T1" is a Section-5 instantiation of ρ, δ, L `[P]` (re-read via proxy). The first synthesis used "≈2·T1" three times as if fixed |

**What the register does NOT contain, and why it matters:** no entry anywhere abandons or criticises
"put spawned work where only its producer can find it" — because nobody built it as a design. Its
nearest published relatives are N12 (Tokio's capacity-1 slot, filed as a defect), P16 (HPX/TBB
isolation, where nothing is expected to cross) and P19 (thread-per-core, where the queue is
owner-only by design and fed by data-owner-routed inboxes, never by a stealing pool's spawn path).

## U. Unverified-claims register — never silently dropped

**U-0 — Method caveat applying to every `[P]` number in these files.** The fetch tooling could not
extract PDF text directly; lens 1 read papers through a text-extraction proxy (r.jina.ai) and a
summariser, so every paper number is a **second-hand transcription** of the primary text. Lenses 2, 4
and 5 could not open most PDFs at all (no PDF renderer in the environment — "pdftoppm is not
installed") and report those papers at abstract level or not at all. Any single `[P]` number a
decision hinges on must be re-checked in the named section of the typeset PDF. The already-downloaded
files sit under the session's `tool-results/*.pdf` (lens 5 note) and become readable once
poppler-utils is installed.

| # | Claim | Status |
|---|---|---|
| U1 | libfork's child-stealing memory bound "Mp ≤ P·M_1²" (their Eq. 4) | unusual form; standard statement is "no P·S_1 bound"; unconfirmed |
| U2 | Nowa "486.93× faster than libgomp" | implausible as a scheduler effect; unconfirmed |
| U3 | HotSLAW HCS vs StealHalf: "27 %" and "122 %" in the same extraction | inconsistent; both unconfirmed |
| U4 | Cilk-5 "fence ≈ 50 % of Pentium Pro overhead" | ambiguous whether of THE or of total; the corroborating Acar/Charguéraud/Rainey statement and Morrison & Afek's ≈25 % ARE `[P]`-verified |
| U5 | Lifeline mechanism details (w, hypercube, deferred requests) | from GLB arXiv:1312.5691 (read), not the PPoPP'11 original (not opened); "87 % on 2048 nodes" abstract-only |
| U6 | Fibril's design and lock-based join | not opened; described only via Nowa and libfork |
| U7 | Sukha's depth-restricted lower bound (SPAA'09) | could not open; abstract elided; rests on a search description |
| U8 | Quintin & Wagner HWS/PWS; Qthreads Sherwood; LAWS; HotSLAW PDF (lens 4) | not opened / not extractable; designs corroborated by NUMA-WS only |
| U9 | TBB's actual affinity implementation as read by lens 1 (Robison/Voss/Kukanov IPDPS'08 not opened) | lens 2 and lens 4 DID read `src/tbb/mailbox.h` — the mailbox is `[S]`-verified; the IPDPS'08 rationale is not |
| U10 | Wool "2–3× on fine-grained workloads, over 50 in extreme cases" | abstract only |
| U11 | Hood (Blumofe & Papadopoulos SIGMETRICS'98) | not opened; multiprogramming argument carried by ABP and BWS |
| U12 | Yang & He IJPP 2018 survey taxonomy | paywalled; the axis set here is assembled from primaries, not adopted from a published taxonomy |
| U13 | ADM "40× over Carbon on gtfold" | slides, not the paper |
| U14 | rayon's sleep state machine as seen by lens 1 | lens 1's `wait_until_cold` quote was truncated; lens 2 and lens 5 DID read `sleep/mod.rs`, `sleep/counters.rs`, `sleep/README.md` — rayon's idle policy is `[S]`-verified through them |
| U15 | "Defect A destroys the `T1/P + O(T∞)` guarantee" | `[I]` — inference from the hypotheses of Blumofe & Leiserson Theorem 13; no paper analyses a design where spawned work is placed where nobody reads |
| U16 | Heartbeat PLDI'18 theorem statement and experimental numbers as seen by lenses 2/4/5 | PDF not readable there; lens 4's degraded extraction produced figures ("3–15 %", an "H capacity factor") that are **excluded**; lens 1's proxy read supplies the `[P]` figures used |
| U17 | Guo et al. IPDPS'09 measured stack/steal numbers | lens 2/4/5 could not open; lens 1's proxy read supplies FJ(1024) 4.6× and fib(35) 10.2× `[P]` |
| U18 | Eager/Lazowska/Zahorjan 1986 crossover load and verbatim conclusions | scanned PDF, not OCR'd; direction reported from paraphrase only |
| U19 | Doug Lea, "A Java Fork/Join Framework" (2000) | could not open; FJP source read instead |
| U20 | Cilk-5 "2 to 6 times the cost of a C function call" | abstract via search index (lens 2); lens 1 proxy-read the paper (27–115 ns/spawn) |
| U21 | Intel "How Task Scheduler Works" wording | page returned 403; wording via search index — near-verbatim, not byte-exact |
| U22 | Go's modern `runqgrab` `stealRunNextG` delay; modern `findRunnable`/`stealWork` body; spinning admission `2*nmspinning < gomaxprocs − npidle` | go1.5.4/go1.4 read verbatim (`usleep(100)`, `n − n/2`, `runqputslow`, `globrunqget`); master truncated by the fetcher's window; the admission expression comes from search, not a verbatim read. The parking/unparking comment, `wakep` (go1.16.15) and the three rejected approaches ARE `[S]` |
| U23 | OpenCilk cheetah runtime source | 404 on both branches; all Cilk claims are paper/secondary |
| U24 | Weave "3×–10× less overhead than TBB/OpenMP", "~2000 cycles" | README, no harness |
| U25 | spice/chili numbers (sub-ns/node; rayon ~15 ns; 60× at 1000 nodes; chili 3.51× on M1, 7.83× on Ryzen; slower than serial at 1023 nodes) | authors' own harnesses, single benchmark kind; spice self-declares "zero testing coverage" |
| U26 | Tokio 2019 benchmark numbers (11.9× chained_spawn, +34 % hyper) | project-official but vendor-run; the PR #4383 +14 %/+25 % are likewise maintainer-reported on the official harness |
| U27 | BEAM "every 2000·CONTEXT_REDS" and check_balance mechanics | theBeamBook (community book), ERTS source not opened |
| U28 | Why oneTBB removed the public affinity API specifically | the removal is `[D]`; no primary source states the reason for affinity in particular |
| U29 | libomp `__kmp_steal_task` body; libdispatch `_dispatch_root_queue_poke` | outside the fetched windows |
| U30 | Kotlin `WORK_STEALING_TIME_RESOLUTION_NS`, `MIN_STEAL_SIZE` values | **CLOSED (rev. 1)**: `WORK_STEALING_TIME_RESOLUTION_NS = systemProp("kotlinx.coroutines.scheduler.resolution.ns", 100000L)` = 100 µs; `IDLE_WORKER_KEEP_ALIVE_NS` default 60 s; no `MIN_STEAL_SIZE` exists in `Tasks.kt` `[S]` |
| U31 | The 7.69× / 1.01× / "4–5 of 16" / 25.1 % figures | from the 2026-08-30 census (OPEN-QUESTIONS); the code paths were re-verified at this checkout `[L]`, the measurements were **not re-run** by any lens or by this synthesis |
| U32 | Naughty Dog: 160 fibers (128×64 KiB + 32×512 KiB), 3 priority queues, no stealing, ~6 workers, main thread as a job | GDC 2015 slides not extractable; secondary summaries; only the fiber-swap-on-wait mechanism is corroborated by FTL's source |
| U33 | Unity 2022.2 per-worker futex chain / "single jobs wake a single worker" | unity.com returned 403; secondary summary |
| U34 | Unity "uses work stealing… to even out the amount of tasks" (strong phrasing) | the manual page opened states the weaker form ("looks at the other worker threads' queues"); the steal-half-for-locality sentence IS `[D]` |
| U35 | Destiny (Genova GDC 2015) job-fibers / resource tracking; Frostbite slide quotes | session abstract and a third-party transcription (silo.tips); deck not public. Note: the brief attributed the Destiny job talk to "Barrett/Tatarchuk"; Tatarchuk's talk is the renderer |
| U36 | Insomniac's job system | no published source found |
| U37 | Molecule numbers (6.31×, 3.38×, 18.5 vs 5.3 ms, 256, 32 KiB, 64→128 B) | author's blog, no harness |
| U38 | Bitsquid "~20 enqueued tasks → high lock contention" | prior art as described in the post |
| U39 | UE's local-queue placement rule (LIFO own push, affinity mailbox behind `TryLaunchAffinity`), `EQueuePreference` semantics, standby threads | inferred from "based on code from the Eigen library"; Epic source requires an authenticated account; API page rendered empty |
| U40 | "Unity worker count = cores − 1" | not stated on the pages opened |
| U41 | Vyukov bounded MPMC scaling "32/65/120/265 cycles at 1/3/5/15 cores"; LCRQ/MS-queue "peaks at two threads", ">3× from eight threads", "~40M ops/s" | search snippets; the 1024cores page holds only "75 cycles on a dual-core"; the PPoPP'13 PDF not extractable |
| U42 | crossbeam `Injector` "single-producer" (a summariser claim seen by lens 4) | almost certainly a summariser error — the crate documents it as a shared FIFO; `[L]` read of `deque.rs` confirms MPMC |
| U43 | boyko idle-episode cost "~187 probes at W=16" | arithmetic from two verified constants (`Backoff` step>10; W−1 stealers), **not a measurement** |
| U44 | Whether `Injector::push` pins the epoch on the boyko spawn path | **CLOSED (rev. 1)**: it does NOT — crossbeam-deque 0.8.7 pins only in `Worker::resize` and `Stealer::steal*` (`deque.rs:301,650,764,1006`); `Injector::push` is two Acquire loads + SeqCst CAS + slot `fetch_or`; an empty Injector probe is two loads + `fence(SeqCst)` + a Relaxed load `[L]`. The first synthesis asserted the pin on the steal side as `[S]`; that was wrong and is corrected in every cost row |
| U45 | Bevy nested scopes "execute on the outer scope's thread" | **CLOSED (rev. 1) — FALSE**: `Scope::spawn` → `self.executor.spawn(…)` (the pool-wide executor, "Spawns a scoped future onto the thread pool"); only `spawn_on_scope` → `scope_executor: ThreadExecutor` pins to the scope's thread; `spawn_on_external` → `external_executor` `[S]` (`task_pool.rs` re-read). A nested `spawn` in Bevy is a P3 global-injector spawn reachable by every worker; the closest ECS peer has no defect-A hole |
| U46 | Rust `std` parker memory-ordering claims; `ScopeShared::register_task/complete_task` orderings | parker: `[S]`; boyko's orderings inferred from doc comments, the RMW *count* on the spawn path is `[L]` |
| U47 | HEFT algorithm details | abstract-level only |
| U48 | Charm++ seed-balancer strategy names | manual truncated |
| U49 | OpenMP `KMP_BLOCKTIME` default 200 ms | Intel page 403 |
| U50 | Downs concurrency cost anchors (~10 ns / ~110 ns / ~1 µs / ~10 µs) | blog with a published harness and named hardware; not peer-reviewed |
| U51 | forte (heartbeat pool): 5 µs beat, 32-slot per-worker queue, 16-job spawn batches, 32-thread cap | README `[D]`, re-read: **no benchmark numbers**; "named as the bevy_tasks 0.17 direction" is a search snippet of bevy #18510, comment not opened |
| U52 | Seastar `smp_message_queue` (`queue_length = 128`, `batch_size = 16`) and glommio's `!Send` per-executor queues | `[R:S]`/`[R:D]` — read by refuter 2, not re-opened here |
| U53 | ForkUnion design and numbers (54/86 µs vs rayon 483/739 µs on a 128× Xeon; TPAUSE/WFET parking; nesting banned) | README `[R:B]` — author-run, not re-opened here |
| U54 | Unity "up to a 1.15x speedup" from removing redundant completion wakes; users reducing worker counts as a workaround | discussions.unity.com staff post `[R:D]` — harness not published; not re-opened here |
| U55 | Linux CFS `select_idle_sibling` behaviour; `sysctl_sched_migration_cost = 500000UL`; `task_hot` body; `sched_balance_newidle` definition line; `CONFIG_SCHED_CLUSTER` help text | `fair.c` / `Kconfig` `[R:S]` — the constant's declaration was read by the refuter, `task_hot`'s body and the newidle definition line were not; nothing re-opened here |
| U56 | Parallel depth-first scheduling: 1.3–1.6× over WS, 13–41 % less off-chip traffic (simulated CMP) | Liaskovitis et al. SPAA'06 brief `[R:P]` via text proxy; the SPAA'07 full paper and Blelloch & Gibbons SPAA'04 (`M_1(C + P·D)`) `[P*]` not opened |
| U57 | Bender & Rabin heterogeneous scheduling; Mitzenmacher two-choice; Rudolph/Slivkin-Allalouf/Upfal symmetric exchange | `[P*]` — titles only; recorded to close axes 2, 4 and 30 |
| U58 | Markatos & LeBlanc affinity scheduling (1994) | `[P*]` — abstract only ("simultaneously balance the workload, minimize synchronization, and co-locate loop iterations with the necessary data"); libomp `static_steal` is the `[S]`-verified occupant |
| U59 | Linux workqueue affinity-scope CPU-utilisation columns (75.49 % / 66.84 %) | `[R:D]` — the bandwidth columns were re-read here (`[D]`, N54); the CPU-% columns were not |
| U60 | Chromium foreground `MayBlockThreshold` / `BlockedWorkersPoll` values; the MAY_BLOCK vs WILL_BLOCK max-tasks comment | foreground values come from runtime params (`…Param.Get()`), not constants `[S]`; the quoted comment is not present verbatim in `thread_group_impl.cc` `[S]` |
| U61 | A-STEAL "waste ≈ 2·T1" | **CORRECTED (rev. 1)** — N58; the numeric coefficient is a Section-5 example, the theorem is parametric `[P]` |
| U62 | HPX `thread_schedule_hint_mode::thread` semantics ("prefer scheduling a task on the local thread number associated with this hint") | `[R:D]` — not re-opened here |
| U63 | Tokio `push_batch` (link first, one lock, one `len.store`); Go `runqputbatch`/`globrunqputbatch` under one `sched.lock` (#40457) | `[R:S]`/`[R:D]` — not re-opened here |
| U64 | FJP `externalPush` / submission-queue index choice by `ThreadLocalRandom` probe; BEAM last-scheduler placement | `[R:S]`/`[R:D]` — the Implementation Overview quotes are `[S]` (lens 2 and the refuter); `externalPush`'s body was not re-read here |
| U65 | Intel WAITPKG `umonitor`/`umwait`/`tpause` semantics and the OS-settable deadline MSR | `[R:D]` — intrinsics guide, not re-opened here |
| U66 | Halide `wake_a_team` / `wake_b_team` broadcast sites and `target_a_team_size` update rule | the quoted comments and the `target_a_team_size` field are `[S]` (re-read); the broadcast bodies were not quoted back by the fetcher |
| U67 | Jolt `JobSystemWithBarrier.cpp::BarrierImpl::Wait` own-jobs-only loop; `Acquire(max(1, GetValue()))` | `[R:S]` — `JobSystemThreadPool.cpp` was re-read here (`[S]`), the barrier file was not |
| U68 | rayon RFC 0001 quotes ("we actually push two items…", "performs equivalently to today's code"); rayon `latch.rs` wake-only-when-SLEEPING; rayon #590 stack overflows | `[R:D]`/`[R:S]` — `job.rs::JobFifo` and `registry.rs::push_fifo` were re-read here (`[S]`); the RFC, `latch.rs` and #590 were not |
| U69 | marl `Worker::steal` trylock body | the spinning-worker comment and the 1 ms / 256-iteration / 32-nop spin constants are `[S]` (re-read); the steal body was summarised, not quoted |
| U70 | O3DE `JobManagerWorkStealing` (`maxStealAttempts = workers×3`, first-available `ActivateWorker`, sticky victim) | `[R:S]` — cited as an occupant of P1 + L3 + J1, not re-opened here |

## S. Sources index, by kind

Only the kinds and the load-bearing items; each lens's full list is in its report. **Source read
verbatim `[S]`:** rayon-core (`registry.rs`, `scope/mod.rs`, `join/mod.rs`, `job.rs`, `latch.rs`,
`sleep/{mod,counters}.rs`), crossbeam-deque `deque.rs` (0.8.6 and 0.8.7 locally), crossbeam-utils
`backoff.rs`, crossbeam-epoch `internal.rs`, tokio `multi_thread/{worker,idle,queue}.rs`, Go
`proc.go` (go1.4 `proc.c`, go1.5.4 `proc1.go`, go1.16.15, master partial), OpenJDK
`ForkJoinPool.java`, oneTBB `src/tbb/{mailbox.h,arena.h,arena.cpp,task_dispatcher.h,
task_dispatcher.cpp,scheduler_common.h}`, .NET `ThreadPoolWorkQueue.cs`,
`PortableThreadPool.{WorkerThread,HillClimbing}.cs`, `LowLevelLifoSemaphore.cs`, LLVM libomp
`kmp_tasking.cpp` (partial), Taskflow `executor.hpp`, HPX `local_priority_queue_scheduler.hpp`,
`local_workrequesting_scheduler.hpp`, async-executor `lib.rs`, bevy `task_pool.rs`,
`multi_threaded.rs`, `batching.rs`, `par_iter.rs`, Folly `CPUThreadPoolExecutor.cpp`, libdispatch
`queue.c` (partial), Kotlin `CoroutineScheduler.kt`, Eigen `NonBlockingThreadPool.h`, `RunQueue.h`,
`EventCount.h`, enkiTS `TaskScheduler.cpp`, FiberTaskingLib `task_scheduler.cpp`, RBDOOM-3-BFG
`ParallelJobList.cpp`, Godot `worker_thread_pool.cpp`, flecs `worker.c`, chili `lib.rs`, Rust std
`thread_parking/futex.rs`, parking_lot `parking_lot.rs`, and boyko_threadpool / boyko_ecs /
boyko_physics at this checkout `[L]`.

**Official docs / design docs / maintainer statements `[D]`:** rayon RFC 5, `sleep/README.md`, FAQ;
Tokio #4941, #4936, #4323, PR #4383; Go scheduler design doc (Vyukov); oneTBB user guide (How Task
Scheduler Works — via index; Work Isolation; Guiding Task Scheduler Execution; Bandwidth and Cache
Affinity; Migration Guide); Unity Manual (job system overview, parallel jobs, `JobHandle.Complete`,
`JobsUtility`, troubleshooting, "Scheduling a job from a job — why not?"); Unity Entities 1.0;
Unreal Tasks Systems, `FScheduler`, `ELocalQueueType`, `TryRetractAndExecute`; Godot
`WorkerThreadPool` reference; flecs Systems manual, FAQ, #1590, #334; EnTT `entity.md`, #1300; Bevy
#318, #384, #4466, #4301, #10064, #4718, #8304, PR #11801; HPX manual; crossbeam docs; 1024cores
bounded MPMC; MSDN Magazine 2010-09; MSDN "Work-Stealing in .NET 4.0"; Rust RFC 230; Win32
`WaitOnAddress`; N3872 (ISO C++); Weave README; theBeamBook; erlang.org ERTS optimisations;
Charm++ manual; LLNL OpenMP tutorial; specs `DispatcherBuilder`.

**Papers read via proxy `[P]`:** Blumofe & Leiserson JACM'99; Frigo/Leiserson/Randall PLDI'98; Arora/
Blumofe/Plaxton TOCS'01; Chase & Lev SPAA'05; Lê et al. PPoPP'13; Morrison & Afek ASPLOS'14; Michael/
Vechev/Saraswat PPoPP'09; Hendler & Shavit PODC'02; Dinan et al. SC'09; van Dijk & van de Pol
Euro-Par'14; Custódio/Paulino/Rito SPAA'23 + arXiv:1810.10615; Acar/Charguéraud/Rainey PPoPP'13;
Guo/Barik/Raman/Sarkar IPDPS'09; SLAW IPDPS'10; Acar/Blelloch/Blumofe TOCS'02; NUMA-WS
arXiv:1806.11128 (ar5iv, full); Simhadri et al. SPAA'14; HotSLAW PGAS'11; GLB arXiv:1312.5691;
Mohr/Kranz/Halstead TPDS'91; Tzannes et al. PPoPP'10 and TOPLAS'14; Acar et al. PLDI'18 (lens 1
only); Acar et al. PPoPP'19; TPAL PLDI'21; HBC ASPLOS'24; Wagner & Calder PPoPP'93; Cilk-M PACT'10;
Nowa IPDPS'21; libfork arXiv:2402.18480; Shiina et al. Cluster'22; ProWS PPoPP'19; Tardieu/Wang/Lin
PPoPP'12; Kumar et al. OOPSLA'12 and VEE'14; A-STEAL; BWS EuroSys'12; Spoonhower et al. SPAA'09;
arXiv:1309.5301; arXiv:1103.4142; arXiv:1305.6474 (ar5iv); arXiv:1805.00857; arXiv:1804.04773
(ar5iv/abstract); XGOMP arXiv:2502.05293 (HTML, full); Prokopec work-stealing iterators; arXiv:
2105.07902.

**Re-read by this synthesis in refutation round 1 `[S]`/`[D]`:** crossbeam-deque 0.8.7 `deque.rs`
(local registry copy: `Injector::push`, `steal_batch_with_limit_and_pop`, `Stealer::steal`, all
`epoch::pin` sites); boyko `scope.rs:125-175,440-531`; Folly `ThrottledLifoSem.h`; LLVM libomp
`kmp_dispatch.cpp` (`static_steal`, `guided_*`); rayon `job.rs` (`JobFifo`) and `registry.rs`
(`push_fifo`, `wait_until_cold`, `find_work`); forte README; Jolt `JobSystemThreadPool.cpp`; PhysX
`ExtDefaultCpuDispatcher.cpp`; `docs.kernel.org` workqueue; Julia `partr.jl`; GHC `Sparks.c`; Halide
`thread_pool_common.h`; bevy `task_pool.rs` (`Scope` fields and `spawn*`); marl `scheduler.cpp`;
Chromium `thread_group_impl.cc`; Microsoft Learn IOCP; Kotlin `Tasks.kt`; A-STEAL (Theorem 12) and
Blumofe & Leiserson (Lemma 12, Theorem 13, Section 4 rule, "fully strict") via the r.jina.ai text
proxy `[P]`.

**Read by a refuter, not re-opened here `[R:*]`:** FJP `externalPush`; HPX `thread_enums.html`;
theBeamBook scheduling; Linux `kernel/sched/fair.c`, `arch/Kconfig`, `sched-domains.html`; Tokio
`inject/rt_multi_thread.rs`; golang/go #40457; Seastar `smp.hh` + tutorial; glommio docs; ForkUnion
README; Liaskovitis et al. SPAA'06 (proxy); Bender & Rabin, Mitzenmacher, Rudolph et al., Markatos
& LeBlanc (abstracts); Unity staff forum post; rayon RFC 0001, `latch.rs`, #590; Jolt
`JobSystemWithBarrier.cpp`; Julia `src/scheduler.c`; Intel intrinsics guide (WAITPKG); O3DE
`JobManagerWorkStealing.cpp`.

**Could not open / abstract-only `[P*]`:** Sukha SPAA'09; Fibril SPAA'16; Saraswat et al. PPoPP'11;
Yang & He IJPP'18; Robison/Voss/Kukanov IPDPS'08; Faxén ICPP'10; Quintin & Wagner Euro-Par'10; Blumofe &
Papadopoulos SIGMETRICS'98; Eager/Lazowska/Zahorjan 1986 (scan); LCRQ PPoPP'13; SCQ DISC'19; LAWS;
Qthreads Sherwood; Prell's thesis; Lea 2000; HEFT TPDS'02; Gyrling GDC 2015; OpenCilk cheetah
source; Epic's engine source; Unity engineering blog 2022.2.

**Blogs / READMEs / talks `[B]`:** tokio.rs 2019 scheduler post; Molecular Musings Parts 1, 3, 4, 5;
Bitsquid "Task Management"; Narkowicz "Job System and ParallelFor"; Schöner "Whipping Boy";
cbloomrants 2012-03-06; Our Machinery fiber job system; spice and chili READMEs; Downs "Concurrency
Cost Hierarchy"; ADM ASPLOS'10 slides; Frostbite slide transcription; Bevy Cheat Book; unreal_source_
explained; Unreal Community Wiki; swedishcoding.com (Gyrling deck posting).
