# KE16 — Refutation round 2, re-read as data: the integration audit and the missing register rows N59–N67

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`. Negative
results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. Cost anchors as in
`KE16-VARIANTS-PLACEMENT.md`.

**What this file holds.** The two refutation-round-2 reports (refuter 3, *theory and empty cells*;
refuter 4, *shipped runtimes and negative results*) taken as raw input a second time, checked
finding-by-finding against the corpus at this checkout, and the parts of them that are **not**
present anywhere in the corpus written out.

**Provenance.** This pass **did NOT re-open any source** — not a paper, not an upstream repository,
not a vendor page. Every claim below is carried at the kind the refuter stated and tagged
`[R:S]` / `[R:D]` / `[R:P]` / `[R:B]` exactly as the index legend defines those tags: *read by a
refuter at the stated kind, not re-opened by this synthesis*. Two exceptions are marked `[L]`: they
are reads of files **in this worktree** made while auditing the integration (`ls`/`grep` over
`docs/threadpool/`), not of any external source. A number here that carries no tag is a defect.

**What the audit found, and why no variant ids are minted.** The brief for this file assumed round 2
was never integrated. At this checkout it **is**: `KE16-VARIANTS-ADDENDA-2.md` carries
**P23–P25, G21–G24, J14, W20–W21, S10** and the axis-36/37/38 evidence; `KE16-DESIGN-SPACE.md` §B
carries **axes 36–42**; §F carries **E26, E28, E29**; §I logs both refuters as *"Accepted, all five
wrong claims, all fourteen items, and all five axes"* and *"Accepted, all nine…"*-shaped verdicts for
every finding below `[L]`. Every design in both reports already owns an id. Minting a second id for
the same design would break the one discipline the whole corpus rests on — *cross-references are by
variant id, never by line* — so **this file mints no `P`/`G`/`J`/`W`/`L`/`S` id and adds no axis.**
§1 is the cross-walk that proves the claim per finding.

**What the audit found missing.** The negative results round 2 supplied were promised numbers in
§I (`N59–N63`, `N65`, `N66`, `N67`) and **were never written into `KE16-EVIDENCE.md`** — that file's
§N stops at `N58` and its §U at `U70`, while §I of the index states the registers hold *"67 entries"*
and *"87 entries"* `[L]`. §3 writes those rows, in the §N table shape, at the numbers §I already
cites, so the existing cross-references from §I, `E13`, `P7` and `J12` resolve. That is the only
substantive addition in this file.

---

## 1. Integration cross-walk — every round-2 finding against the corpus at this checkout

`[L]` for every "catalogue location" cell: located by grep over `docs/threadpool/` at this checkout.

### 1.1 Refuter 3 — missing variants and cells

| Refuter's finding | Catalogue location | State |
|---|---|---|
| Count-gated completion signal: only the LAST completer signals (rayon `CountLatch` + `CoreLatch`; std `ScopeData`) `[R:S]` | `W20` in `KE16-VARIANTS-ADDENDA-2.md`; shortlist row `W-d′`; `E28` in §F | INTEGRATED |
| Thief-splitting: split budget halved per split, RESET on migration (rayon `Splitter`) `[R:S]` | `G21` in `KE16-VARIANTS-ADDENDA-2.md` | INTEGRATED |
| Self-replicating task: each replica that STARTS queues one more (.NET `TaskReplicator`) `[R:S]` | `G22` in `KE16-VARIANTS-ADDENDA-2.md` | INTEGRATED |
| Decoupled idle spin on ONE signal word, queues scanned only after acquisition (.NET `LowLevelLifoSemaphore.WaitSlow`) `[R:S]` | `W21` in `KE16-VARIANTS-ADDENDA-2.md`; `E18`/`W-c` re-verdicted | INTEGRATED |
| Per-SCOPE queue registered in the scan set — theory-only empty cell (axis 1 × 11 × 19 × 29) | `E26` in `KE16-DESIGN-SPACE.md` §F, recorded as an EMPTY cell | INTEGRATED |

### 1.2 Refuter 3 — missing axes

| Refuter's axis | Catalogue location | State |
|---|---|---|
| 36. Timed-wait RESOLUTION of the target OS `[R:S]`/`[R:D]` | axis **36** in §B; axis-36 evidence section in `KE16-VARIANTS-ADDENDA-2.md`; §C row 36; `App-7`, `H.10` | INTEGRATED |
| 37. Analytical model whose hypotheses the workload satisfies (ABP Th. 9; Tchiboukdjian Th. 1–3, 6; Gast Th. 4.1; Karlin Th. 6–7; Kruskal–Weiss / GSS) `[R:P]` | axis **37** in §B; axis-37 evidence section in `KE16-VARIANTS-ADDENDA-2.md` | INTEGRATED |
| 38. Contention class of each RMW (single-writer line vs multi-writer line vs foreign line) `[R:I]`/`[R:B]` | axis **38** in §B; axis-38 evidence section in `KE16-VARIANTS-ADDENDA-2.md`; every cost row annotated | INTEGRATED |

### 1.3 Refuter 4 — missing variants

| Refuter's finding | Catalogue location | State |
|---|---|---|
| BWoS block-based deque (OSDI'23; NVIDIA stdexec `static_thread_pool`) `[R:P]`/`[R:S]` | `G23` in `KE16-VARIANTS-ADDENDA-2.md`; axis 41 | INTEGRATED |
| rayon per-worker BROADCAST deque — owner-only queue inside a stealing pool `[R:S]` | `P23` in `KE16-VARIANTS-ADDENDA-2.md`; axes 11 and 33 corrected | INTEGRATED |
| libomp Task Scheduling Constraint: helper/thief restricted to descendants of the last suspended TIED task `[R:S]` | `J2` in `KE16-VARIANTS-GRANULARITY-JOIN.md` (libomp moved out of `J1`); axis 19 | INTEGRATED |
| Java FJP `asyncMode` — shipped FIFO-local mode with the stated condition "tasks that are never joined" `[R:D]` | axis 7; `L1`; `App-2`; `E19`; the `N51` record re-framed | INTEGRATED |
| Wicked Engine — rotating-counter placement into registered per-thread queues; fan-out by push kind `[R:S]` | `P24` in `KE16-VARIANTS-ADDENDA-2.md`; axis 34; axis 33 extended | INTEGRATED |
| stlab — rotating start + first-accepting `try_push`, one `try_pop` rotation then block on the own queue `[R:S]` | `P25` in `KE16-VARIANTS-ADDENDA-2.md`; axes 20, 22, 34 | INTEGRATED |
| Nanos6/OmpSs-2 "immediate successor" — the completing CPU runs the successor, probability p `[R:D]` | `S10` in `KE16-VARIANTS-ADDENDA-2.md`; axis 40; `M1-c` | INTEGRATED |
| oneTBB task-scheduler BYPASS — the body returns the next task `[R:D]` | `G24` in `KE16-VARIANTS-ADDENDA-2.md`; axis 1; axis 35 | INTEGRATED |
| rayon `in_worker_cross` — a worker of pool A helps in pool A while awaiting pool B `[R:S]` | `J14` in `KE16-VARIANTS-ADDENDA-2.md`; axis 42; `App-6` | INTEGRATED |
| Five NEGATIVE RESULTS (Windows UMS; NetBSD scheduler activations; Linux UMCG; Microsoft ConcRT; Weave→Constantine) `[R:D]` | **numbers reserved in §I as `N59`–`N63`; rows absent from `KE16-EVIDENCE.md` §N** | **MISSING → written in §3** |

### 1.4 Refuter 4 — missing axes

| Refuter's axis | Catalogue location | State |
|---|---|---|
| Placement-policy SELECTION POINT (per pool / per call / per task class) `[R:D]`/`[R:S]` | axis **39** in §B; `E29` in §F | INTEGRATED |
| Successor hand-off on COMPLETION `[R:D]` | axis **40** in §B; `S10` | INTEGRATED |
| Deque synchronisation GRANULARITY (per task vs per BLOCK) `[R:P]`/`[R:S]` | axis **41** in §B; `G23` | INTEGRATED |
| Cross-POOL reachability and helper set `[R:S]`/`[L]` | axis **42** in §B; `J14`; `App-6` | INTEGRATED |
| Wake fan-out CONDITIONED ON PUSH KIND `[R:S]` | folded into axis **33** rather than given its own number (§I, refuter 4, *Axes*) | INTEGRATED, as an extension |

---

## 2. Disputed claims — recorded as claims, never silently applied

The ten `wrong_claims` of round 2 were **adjudicated by the synthesis** (§I, revision 2: *"Accepted,
all five wrong claims"* for refuter 4 and the itemised list (1)–(7) for refuter 3) `[L]`, so they are
not open disputes and are not re-litigated here; the first table records where each landed so a
reader of this file can check the adjudication rather than take it on trust. The second table holds
what round 2 raised and the corpus does **not** resolve: those are the live disputes.

### 2.1 Adjudicated in revision 2 — pointer only

| Refuter's claim | Its source (as stated by the refuter) | Catalogue location it disputed | Adjudication |
|---|---|---|---|
| Theory is absent for a flat help-first wave ("Blumofe–Leiserson does not cover it; Guo et al. applies") is wrong — ABP Th. 9 covers arbitrary DAGs and "either choice" | ABP TOCS'01 §3, §3.4, §4.3 `[R:P]`; Tchiboukdjian Th. 6 `[R:P]` | `P1`, shortlist `A1`, §I round-1 note | ACCEPTED; `P1`/`A1` rewritten, axis 37 added |
| `scope.rs:149-151`'s stated reason for the unconditional unpark is wrong; `fetch_sub` returns the previous value, the real constraint is the wake target's lifetime | rayon `latch.rs`, std `scoped.rs` `[R:S]`; `scope.rs:138-160` `[L]` | `W17`, `W-d`, §H.8 | ACCEPTED; `W-d′ = W20`; §A.2 row added; `N66` promised (see §3) |
| The cost table's rayon COMPLETE column understates rayon by the wave size; the "unbuilt parked flag" cell is occupied by `CoreLatch` | rayon `latch.rs`, `scope/mod.rs` `[R:S]` | cost table, axis 29 | ACCEPTED; corrected, tag raised to `[S]` |
| Every backstop / latency-floor number (50 µs, 100 µs) is a Linux number; on this toolchain the wait is ≥1 ms | toolchain `std` parking path `[R:S]`; `timeBeginPeriod` page `[R:D]`; `PR_SET_TIMERSLACK` `[R:D]` | axis 14, `W5`, `J11`, `W-d`, §A.7 | ACCEPTED; axis 36, `App-7`, `H.10`; A.2/A.3/A.7/W5/J11/G3/B0/B3/W-b/M1-a corrected |
| `E18` / `W-c` "no occupant" is wrong — .NET's `WaitSlow` is exactly that form | .NET `LowLevelLifoSemaphore.cs`, `PortableThreadPool.WorkerThread.cs` `[R:S]` | `E18`, `W-c`, cost-table reading (5) | ACCEPTED; `W21` added, `W-c` re-verdicted |
| "4 shared RMWs — the most expensive spawn path in the survey" conflates lock-prefixed instructions with cache-line transfers | cost anchors `[R:B]`; §A.2 single-writer rows `[L]`; classification `[R:I]` | axis 10, §C row 10, cost table | ACCEPTED; axis 38 added, cost rows annotated |
| Steal-granularity theory recorded as "measurements only" — closed-form bounds exist | Tchiboukdjian Th. 1–3, 6; Gast Th. 4.1 `[R:P]` | `E24`, `G1`/`G2` | ACCEPTED; attached to G2, G20, G12, G18, P17, W-f; `E24` re-verdicted |
| `J1` lists libomp `taskwait` as an unrestricted helper | `kmp_global.cpp`, `kmp_tasking.cpp` `[R:S]` | `J1`, axes 5 and 19 | ACCEPTED; libomp moved to `J2` |
| Axis 11 "rayon closed by construction" and axis 33 "rayon = min(new_jobs − idle, sleeping)" omit the broadcast deque and its wake-all | rayon `registry.rs` `[R:S]` | axis 11, axis 33, §C row 1 | ACCEPTED; `P23` added, both axes corrected |
| §A.2's "`injector_local[i]` is read only with the caller's own id" holds within one pool only | `scope.rs:440-485`, `worker.rs:365-376` `[L]` | §A.2, `P0` | ACCEPTED; `J14`, axis 42, `App-6` added |
| `N51` presents FIFO-both-ends as the field's deprecation verdict | FJP javadoc `asyncMode` `[R:D]` | `N51`, `E19`, `App-2`, §C row 7 | ACCEPTED; `L1`/`App-2` strengthened |
| `J12`'s "not available to a user-space pool" stated as a design fact rather than a triple abandonment | UMS `[R:D]`, NetBSD SA `[R:D]`, UMCG `[R:D]` | `J12`, §N (no axis-32 entry) | ACCEPTED in prose (axis 32 extended); **the register rows are still missing** — §3 |

### 2.2 Live disputes — raised by round 2, unresolved in the corpus at this checkout

| Claim | Source | Catalogue location it disputes | State |
|---|---|---|---|
| §I states the registers hold "67 entries" (§N) and "87 entries plus the U-0 caveat" (§U); the file holds **58** and **70** `[L]` | `KE16-EVIDENCE.md` vs `KE16-DESIGN-SPACE.md` §I `[L]` | `KE16-EVIDENCE.md` §N, §U | **DISPUTED** — the count is a claim about a file that does not support it; §3 closes the §N half at the numbers §I cites, the §U half (`U71`–`U87`, incl. the promised `U80`) is **not** closed here because this pass cannot reconstruct which claims the synthesis meant |
| "No surveyed runtime uses a bitmap" (`W6`) — Linux `select_idle_cpu` scans a per-LLC cpumask, the Windows kernel keeps an idle summary bitmask; the refuter could NOT fetch `fair.c` | refuter 3's own note, explicitly downgraded to a pointer `[R:B]` | `W6`; the promised narrowing `U80` | **DISPUTED** — §I says the claim was "narrowed to user-space pools (U80)", and `U80` does not exist in `KE16-EVIDENCE.md` `[L]` |
| The Windows default system timer resolution (commonly quoted 15.625 ms) is **not** on the Microsoft page the refuter read | refuter 3's note `[R:B]` | axis 36's magnitude argument | **DISPUTED** — axis 36's ≥1 ms floor does not depend on it, but any figure larger than 1 ms does |
| Kruskal–Weiss's FSC formula is quoted from Hagerup 1997's restatement, not from the 1985 paper | refuter 3's note `[R:P]` | axis 37's chunking row | **DISPUTED** — a transcription of a restatement, two removes from the source |
| NVIDIA stdexec's per-thread `remote_queue` inbox: whether it is **stealable** was not established; the refuter asks that it be read before it is placed on axis 11 | refuter 4's note, summarised not verbatim `[R:S]` | `G23`, axis 11, axis 41 | **DISPUTED** — `G23` is placed on axis 41 by the deque design, not by the inbox; the inbox's reachability is unrecorded |
| Godot: whether an unstarted awaited task is **retracted** (`J9`) was NOT confirmed on the re-open | refuter 4's note `[R:S]` | `J9` | **DISPUTED** — `J9`'s UE occupant is untouched; the Godot line is unverified |
| Intel GTS (archived 2023-01-03, "Intel has discontinued development") carries **no design rationale** in its README | refuter 4's note `[R:D]` | `N65` (promised), axes 22 and 27 | **DISPUTED as evidence class** — an abandonment without a stated reason is a data point for the axes, not a negative result with a documented "why"; recorded in §3 as `N65` at that reduced strength, per §I's own reservation of the number |

---

## 3. Negative results N59–N67 — the rows the synthesis promised and never wrote

Same table shape as `KE16-EVIDENCE.md` §N. Every entry `[R:*]`: **not re-opened by this synthesis.**
The numbers are the ones §I already cites (`N59`–`N61` for the axis-32 triple, `N62` ConcRT, `N63`
Constantine, `N65` GTS, `N66` the completion-path record, `N67` Hagerup), so the existing
cross-references from §I, `E13`, `P7` and `J12` resolve to these rows. **`N64` is left unassigned**:
§I reserves the number by skipping it and names no finding for it; guessing a design to fill it would
invent a record, so it stays free and no future entry may take it without saying so.

**Added in refutation round 2 (2026-09-02), never integrated by revision 2's edit to the registers**

| # | Design | Who / why |
|---|---|---|
| N59 | **Kernel-delivered block notification to a user-space scheduler** (axis 32's user-space cell), as a shipped Win32 API | Windows User-Mode Scheduling `[R:D]`: shipped Windows 7 – Windows 10 21H2 — "regain control of the processor if a UMS thread blocks in the kernel"; "The system calls the entry point function when … a worker thread blocks on a system call". Withdrawn: "As of Windows 11, user-mode scheduling is not supported. All calls fail with the error ERROR_NOT_SUPPORTED". **No reason stated by the vendor** — an abandonment on record without a documented "why" |
| N60 | **M:N threading with kernel upcalls on block** (scheduler activations) | NetBSD 5.0 `[R:D]`: "The SA implementation was complicated, scaled poorly on multiprocessor systems and had no support for real-time applications"; replaced by 1:1 ("The threading system was rewritten and is now based on a 1:1 model"). The second independent abandonment of axis 32's user-space cell, and the one with a stated reason |
| N61 | **User-Managed Concurrency Groups / `futex_swap`** — the same notification for Linux | Linux `[R:D]`: Google's RFC v0.1 (2021-05, Oskolkov) through v0.7 (2021-10) and Zijlstra's re-implementation (2021-12); LWN records it unmerged and "still clearly in an early state". Third non-adoption in the same cell — the reason `J12` gives ("needs a kernel hook") is the *consequence* of three abandonments, not a design fact |
| N62 | **A vendor's own user-mode work-stealing scheduler with cooperative blocking**, kept as the default task scheduler | Microsoft ConcRT `[R:D]`: "In Visual Studio 2015 and later, the Concurrency Runtime Task Scheduler is no longer the scheduler for the `task` class and related types in ppltasks.h. Those types now use the Windows ThreadPool for better performance and interoperability with Windows synchronization primitives." Scheduler Policies: "SchedulerKind … ThreadScheduler (use normal threads). **This is the only valid value for this key.**" — the former `UmsThreadDefault` option is gone. A production vendor replacing user-mode work-stealing with an OS pool: the reverse direction of every other entry in this register |
| N63 | **Channel-based work-REQUESTING** (`P7`) for the shared-memory case | Constantine's threadpool, by Weave's own author `[R:D]`: "The threadpool is not backed by Weave but by an inspired runtime that has been significantly simplified for ease of auditing. In particular it uses **shared-memory based work-stealing instead of channel-based work-requesting** for load balancing as distributed computing is not a target"; and "overhead lower than Weave's default (and as low as Weave lazy + alloca)". `P7` lists Weave as an occupant on a "3×–10×" README claim `[R:B]`; the same author's later pool drops the mechanism, with the reason on record. Cited from `E13` and `P7` |
| N64 | *(unassigned — §I reserves the number and names no finding; do not reuse without saying so)* | — |
| N65 | *(an abandonment WITHOUT a documented reason — recorded at reduced strength, see §2.2)* A shipped game-oriented micro-scheduler with a worker pool plus affinity and priority features | Intel GTS (`GameTechDev/GTS-GamesTaskScheduler`) `[R:D]`: "archived on January 3, 2023 … Intel has discontinued development". The README gives **no design rationale**, so this is a data point for axes 22 and 27 and nothing stronger — it does not say any mechanism failed |
| N66 | **An unconditional per-task wake on the completion path, justified by "learning we are last is too late"** | rayon and Rust `std` `[R:S]`: both gate on the count and stay sound by keeping the wake target alive independently of the scope — rayon `CountLatch::set` (`if counter.fetch_sub(1, SeqCst) == 1 { … }`, registry and worker index read **before** the swap, with the note "Once we `set`, the target may proceed and invalidate `this`!") and std `ScopeData::decrement_num_running_threads` (`if fetch_sub(1, Release) == 1 { self.main_thread.unpark() }`, inside an `Arc` "so that other threads can finish their decrement … even after this function returns"). boyko's `scope.rs:149-151` states the opposite as an impossibility `[L]`. Not an abandonment by a third party — a **documented near-miss of boyko's own use-after-free concern, solved by ordering and lifetime rather than by an extra wake** (→ `W20`, `W-d′`) |
| N67 | **GSS as "the" shrinking chunk rule** (`G19`'s only shrinking occupant) | Hagerup 1997, experimental comparison of FSC / GSS / factoring / trapezoid `[R:P]`: "the Bold strategy performed well across the entire gamut of experiments; no other strategy ever achieved a significantly lower average wasted time". GSS is not the best known shrinking rule; `G19`'s occupant list is one point of a family the literature ranks differently |

---

## 4. What this file mints

- **Variant ids:** none. Every round-2 design already carries one (§1).
- **Axes:** none. Round 2's nine axes are axes 36–42 plus the axis-33 extension (§1.2, §1.4).
- **Negative results:** `N59`, `N60`, `N61`, `N62`, `N63`, `N65`, `N66`, `N67` (§3). `N64` stays
  unassigned by construction.
- **Open, for the owner or the next revision:** the §U half of the count mismatch (`U71`–`U87`,
  including the promised `U80`) is **not** reconstructed here — see §2.2, first two rows.
