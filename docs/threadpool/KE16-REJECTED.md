# KE16 — the freeze register: every candidate that did not ship

Companion to `KE16-RESULTS.md` (the measured record) and `KE16-DESIGN.md` §2 (the candidate table).
Decision rules are quoted from `KE16-DESIGN-MEASUREMENT.md` §7; band arithmetic from its §4.

**Why this file exists.** The owner's standing rule: *a losing candidate is not deleted, it is
frozen* — a tag plus a register row carrying the CONDITION under which it should be reconsidered.
A shipped crate carries none of the KE16 feature flags (`Cargo.toml`: *"one per candidate … DELETED
in the same pass once the verdict lands"*), so the flags and their arms will be removed from the
tree. This register is how the work survives that removal: it names the mechanism, the flag, the
code state, the number that eliminated the candidate, and the fact about the world that would make
it worth measuring again.

---

## ⚠ THE REGISTER'S OWN DECAY — read before any row

**A frozen candidate is a SNAPSHOT, not a part on a shelf.** Every row below describes the tree at
the moment its numbers were taken, and the tree moved *during* the tournament — twice:

| Tag | Code state | What was measured on it |
|---|---|---|
| **CS-1** | after the `complete_task` protector fix (`complete_task` takes `*const Self`), **before** Stage 1 of the task representation | Step-0 baseline; axis A's five arms; the `a3`-vs-`a1f` head-to-head that closed axis A |
| **CS-2** | after Stage 1 — the queue element is now `Task { payload: *const (), execute: unsafe fn(*const ()) }` | the axis-W/C reference `a3+b0+w0+c0`, 49 rows |
| **CS-3** | the unconditional code after feature removal | **nothing — Step App has not been run** |

Stage 1's measured effect on the task path, from the disassembly rather than a timing: **instruction
count at the call UNCHANGED**; one level of the dependency chain removed (a load from a second
object — the closure's vtable) and the call becomes tail-callable; spawn pays **+3 instructions and
+8 payload bytes** (22 → 25; allocation 16 → 24 B). Every axis-A row below is CS-1. A revived
candidate is measured against a substrate that has changed underneath it at least once.

**Therefore: a revived candidate must be RE-MEASURED, never trusted.** The numbers here rank the
arms *as they stood*; they do not price them today.

Three further properties of every absolute in this file:

* **Bench profile, not the shipped profile.** `[profile.bench]` is pinned to `codegen-units=1,
  lto=false` and the shipped profile is fat-LTO. Ratios between arms transfer; **absolutes do not.**
* **`W = 16` throughout**, from `available_parallelism()` on a Ryzen 9 5900HS (8C/16T laptop part).
  `W` was never varied.
* **The physics harness is not session-comparable in absolutes and the pool grid is.** Measured, not
  assumed: the same code with no features read `in_scheduled_system` 32.530 ms in one session against
  29.983 ms in another (+8.5 %), while `bench_thread_install` moved +23.7 % — the shift is not a
  single scalar, so no one normaliser repairs it. The pool grid's control reproduced the Step-0 band
  file to < 1 % on 22 of 24 cells and to 0.2 % on the deciding cell.

## ✅ THE FREEZE IS DONE — and this section used to say the opposite

The owner's rule is **tag + register row**. Both halves exist.

* **Tag:** `ke16/candidates-frozen-2026-09-07`, annotated, pointing at `897c812f` — *"the KE16
  candidate arms, and a task representation that cannot outlive its own release"*. Every axis-A, B,
  W and C candidate builds from that commit behind its own feature.
* **Row:** this file.

⚠ **What stood here until 2026-09-08 was written before either existed** and said so in detail: that
`git tag --list 'ke16*'` returned nothing, and that every arm lived in 76 uncommitted worktree
entries at HEAD `4a363678`. Both facts were true when written and both are now false — the arms were
committed as `897c812f` and tagged the same day. The old text is summarised rather than deleted
because a register whose value is that it records what was not yet done must not quietly erase the
moment it stopped being true.

⚠ **One thing the tag does NOT cover, and it matters for a revival.** `897c812f` predates stage 3b
(`d51b4ced`), which moved the scoped task cell out of its own heap allocation and into a per-scope
bump block. An arm recovered from the tag is therefore written against the OLD task path. That is
exactly the decay this register's header describes — a snapshot, not a part on a shelf — and it is
why every return condition below ends in a re-measurement rather than a switch. **When the flags are
removed, the removal commit's parent gets its own tag**, so the last state in which every candidate
built against the SHIPPED task path is also recoverable.

## How to read a row

* **Was** — the mechanism, not the label.
* **Flag / measured at** — the cargo feature that builds it, the HEAD and the code state.
* **Eliminated by** — the number and the cell it came from, with the rule quoted.
* **Return condition** — a **fact about the world that could change**. A row with no such fact says
  the candidate is **CLOSED**, and says so plainly.

---

# §1. Axis A — four arms measured and rejected

Axis A closed on **`a3`** (`ke16-a3`, every spawn to `injector_global` — the design's own *"control /
reachability floor"*). All four rows below are **CS-1**, HEAD `4a363678`.

Two sessions produced them: the five-arm cross-session pass (`a0/a2/a3/a5` in one session, `a1` and
`a1f` each in their own), and then the `a3`-vs-`a1f` **head-to-head interleaved pass by pass in ONE
session**, which is what actually closed the axis.

---

## `a1f` — A1 placement with `new_fifo()` deques

> **The most useful row in this file for whoever revisits axis A.** It is the arm the design
> predicted would win the deciding cell, and it lost that cell by **1.58×**.

**Was.** A1's placement — a same-pool worker spawn goes to the worker's own **registered** Chase-Lev
deque, reached through the one identity predicate `tls::worker_lane_for(inner)` — with `new_fifo()`
deques, i.e. today's end discipline. The owner pops with a `front.fetch_add(SeqCst)` (one
multi-writer RMW per executed own task) but a thief's batch steal is **one CAS per ≤ 33 elements**,
against A1-LIFO's one `SeqCst` CAS **per stolen element**.

**Flag / measured at.** `ke16-a1-fifo`. CS-1, HEAD `4a363678`. Two passes: a stand-alone
cross-session pass (4 runs, plus the only in-session `a0` control any arm built), and the
interleaved head-to-head (`CARGO_TARGET_DIR=D:/tmp/ke16_h2h_a1f`, 4 passes, p1 discarded).

**Eliminated by — §7 Step A rule 2, quoted:**

> *"Physics ranks. Order the survivors by physics `in_scheduled_system` median (lower is better). A
> candidate that is worse on physics beyond 2× the band than another is behind it, whatever the grid
> says — the owner's criterion is throughput on the real consumer, and a variant that wins every 1 µs
> cell but loses the consumer loses."*

| | `a3` | `a1f` | ratio | 2×band | verdict |
|---|---|---|---|---|---|
| physics `in_scheduled_system`/29751, kept p2/p3/p4 | **11.709 ms** (spread 4.477 %) | **13.640 ms** (2.776 %) | 1.1649 | 1.0895 | `a1f` REGRESSES |
| same, normalised by each pass's own `single_threaded_O5` | 0.41731 (2.70 %) | 0.48892 (3.76 %) | 1.1716 | 1.0800 | REGRESSES, by more |
| worker `body_10us_tasks_4W` (the deciding cell) | **114 960 ns** (6.09 %) | **159 470 ns** (18.23 %) | 1.3872 | 1.3646 | REGRESSES |

**The pre-registered abort rule did not fire.** Margin / worst-arm spread = **3.68×** raw and
**4.56×** normalised, against the rule that a margin inside the pass-to-pass spread must be reported
as *unresolved*. Stronger and independent of any statistic: **the ranges of the six kept passes do
not overlap** — `a3` [11.504, 12.019] ms against `a1f` [13.402, 13.774] ms; `a3`'s slowest kept pass
is **11.5 %** faster than `a1f`'s fastest. The two discarded p1 passes fall on the same side.

**⚠ The prediction, and its refutation.** `KE16-DESIGN.md` §2 predicted `a1f` *"to win 1–10 µs × 64W
by RMW count unless the same-end contention mode materialises at W=16"*, pricing the ends at ≈ 0.94
(LIFO) against ≈ 0.09 (FIFO) shared RMWs per task on the one-spawner shape. Measured:

* `a1f` **ties `a1` on all six worker decision cells** — 1.0792 / 1.0742 / 1.0068 / 1.0449 / 1.0215 /
  1.0323, every one inside its 2×band. An **order-of-magnitude difference in shared RMW count
  produced a 2.15 % wall-clock difference** on the deciding cell, i.e. inside the noise.
* On the cell it was built to attack it **loses to the shared injector**: worker `10us_4W`
  **176 364 ns vs `a3`'s 111 278.65 ns = 1.5849×** cross-session (threshold 9.46 %, exceeded 6.2×),
  and **1.387×** interleaved.
* The mechanism the record names is **batch granularity, not CAS count**: `a3` reaches all sixteen
  lanes at once from a shared injector, while a deque arm is drained in ≤ 33-element batches. Every
  per-worker-queue arm lands 173–183 µs on that cell (`a1` 172.6, `a1f` 176.4, `a5` 181.2, `a2`
  183.0) and only the shared-injector arm reaches 111.3 µs.

**⚠ A second, independent defect, recorded because the two instruments disagree.** `a1f`'s ECS ratio
`par_in_system / par_from_dispatcher` at N=65536 reads **1.226 / 1.228 / 1.333 / 1.354 — above the
rule-5 limit of 1.15 in all four passes** on the criterion medians, while the protocol wall
(best-of-3, the source `KE16-DESIGN-MEASUREMENT.md` §3 names) reads 1.00–1.02. The mechanism is
visible: `a1f` **has** `a3`'s fast mode (its minimum sample, 90.4–91.9 ms, is indistinguishable from
its own `par_from_dispatcher`) but its median sits at 117–125 ms with MAD/median 19–20 % on three of
four passes. `a3` shows no such split (p10 91.7 → p50 93.0). Best-of-3 reports the mode it reaches;
criterion reports the mode it lives in. **Which source rule 5 is entitled to use was left undecided**
— it did not bear on this verdict, and it is live for any future candidate.

**Also true, and reported because a one-sided row is not a row.** `a1f` converts acceptance clause
(1) from FAIL to PASS on all four of its runs (0.408–0.435×) and is **2.74× faster than its own
in-session `a0` control** (11.891 vs 32.530 ms). It wins the consumer it was built for and loses the
ranking. It also leads `a3` on exactly one worker decision cell, `10us_W` (28 360.9 vs 29 386.8 ns,
a tie at 2×band = 1.0978).

**How thin the first elimination was, and why the head-to-head existed.** Cross-session, `a1f` lost
by **1.20 percentage points** over its threshold (14.84 % against 13.64 %) on a harness whose
arm-independent control had moved 17 % between two runs of one arm. The judge reported `a3` as the
winner **while refusing to call it safe**, and named the one measurement that would settle it. That
measurement was run, and it gave 6.3× the headroom (7.54 pp).

**RETURN CONDITION — RETURNABLE, and it is the first thing to re-measure.**
Its loss is carried by two things: the worker route at 10 µs × 4W, and `par_in_system` falling out of
its own fast mode. **The separator is batch granularity.** Therefore: *any change to steal
granularity — a smaller batch cap, a steal-half policy, or the C-axis batch spawn (`ke16-c-batch`) —
makes `a1f` a different candidate over a different substrate and voids this measurement.* That is a
scheduled change to the world: `ke16-c-batch` is built and **not yet measured** (§3 below). A second
route: if the consumers' chunk shape changes so the ≤ 33-element drain stops mattering. `a1f` is also
**the arm that reopens axis B** — see §2.

---

## `a1` — A1 placement with `new_lifo()` deques

**Was.** The same registered-own-deque placement as `a1f`, with LIFO deques: the owner's pop is a
local fence plus a shared read instead of FIFO's contended `front.fetch_add`, but a thief's batch
steal costs **one `SeqCst` CAS plus one fence per stolen element**, up to 33 per batch.

**Flag / measured at.** `ke16-a1`. CS-1, HEAD `4a363678`; runs r6/r7 with an independent cooled
cross-check r8.

**Eliminated by — §7 Step A rule 2** (same quotation as `a1f`).

* physics `in_scheduled_system` **16.411 ms** (pair mean of r6 17.448 / r7 15.375) against `a3`'s
  **10.354 ms** = **1.5850 → 58.50 %**, applied band 13.48 %, threshold **26.96 %** — behind by
  **2.2× the threshold**. Behind every other arm too: `a2` 61.68 %, `a5` 39.26 %, `a1f` 38.01 %.
* Normalised by its own run's `O5` it stays last on every pairing (0.5635 against `a3`'s 0.3735).
* **Even its faster single run** (r7, 15.375 ms) is 48.5 % behind `a3` — still beyond the threshold.
* Rule 3 **cannot** separate it from `a1f`: all six worker decision cells tie.

**What it did win, so the row is not a caricature.** On the deciding cell against the *baseline* it
is a large improvement: worker `10us_4W` **172 645.9 ns against `a0`'s 676 193.1 ns = 3.92× faster**,
clearing the improvement threshold by 3.1×, with r8 falling between r6 and r7.

**⚠ Its number is also the least trustworthy in the pass, and that does not save it.** The
arm-independent serial control `single_threaded_O5` — a row no axis-A candidate can touch — moved
**16.84 %** between the two runs `a1` was banded on (26.862 → 31.387 ms), `bench_thread_install`
moved **45.41 %** and `bench_thread_install_Wminus1` **21.88 %**. Against 0.19 % / 0.48 % / 0.96 % /
1.21 % on the other four arms, `a1` is the only arm whose own machine witness failed.

**RETURN CONDITION — RETURNABLE, second in priority behind `a1f`.**
**Re-measure in a session whose `O5` spread is ≤ 1 %, with a same-session `a0` control.** That single
re-measure settles whether 16.411 ms is `a1` or the box. Priority is second because `a1` and `a1f`
tie on all six grid cells and `a1f` wins the physics between them under both readings — so
re-measuring `a1f` answers the axis-B question for the whole A1 family. Note the asymmetry if it
does return: unlike the `a1f` pair, `a1` regresses `a3` **unilaterally** on today's numbers (worker
`10us_4W` 1.5515×, with `a3` regressing none of `a1`'s six), so rule 3 would put `a1` behind `a3`
rather than producing the tie that rule 4 hands to `a1f`.

---

## `a2` — keep `injector_local`, add it to the sibling scan set

**Was.** The minimal reachability fix that changes no spawn path: `injector_local[wid]` stays where
it is and joins the **sibling scan set**, so a task spawned from inside a worker becomes stealable by
the other fifteen. Spawn cost is unchanged from today (two single-writer RMWs plus `pending` plus the
fenced wake decision); the idle path pays **W−1 extra `SeqCst` fences per scan round**.

**Flag / measured at.** `ke16-a2` (`ke16-a5` implies it; the pure arm is selected by
`all(feature="ke16-a2", not(feature="ke16-a5"))`). CS-1, HEAD `4a363678`, runs r2/r3.

**Eliminated by — §7 Step A rule 3, quoted:**

> *"The grid vetoes, symmetrically, within the physics band. Among candidates within 2× the band of
> each other on physics, compare their 1 µs and 10 µs WORKER-route cells (all three task counts)
> PAIRWISE and in BOTH directions: if V regresses (§4 rule) any such cell against U while U regresses
> none against V, V is behind U."*

Physics could not separate the pair — which is exactly the condition rule 3 exists to resolve:
`a2` 10.151 ms against `a3` 10.354 ms = **1.0201**, applied band 4.09 %, 2×band 8.18 % → **TIE**; and
normalised, 0.3782 against 0.3735 = 1.26 % → **TIE**. Rule 2 orders them in **neither** direction.
The grid then separates them unilaterally:

* worker `body_10us_tasks_4W`: **`a2` 182 993.5 ns vs `a3` 111 278.65 ns = 1.6445×**, applied band
  4.00 %, threshold 1.08 — **exceeded by 8×**.
* `a3` regresses **none** of `a2`'s six decision cells (1us_W 1.0360, 1us_4W 1.0733 in `a2`'s favour
  but inside 8.00 %, 1us_64W 1.0631, 10us_W 1.0497, 10us_64W 1.0116 — all ties).
* The verdict survives a 2.7× widening of the band.

**Recorded, but not the ground of the elimination:** `a2`'s ECS ratio read from the **criterion** rows
at N=65536 is **1.2337** (r2 1.3653 / r3 1.1024) — a rule-5 breach — while its **protocol wall** reads
0.996. §3 sources the metric from the wall, so rule 5 did not eliminate it; but `a2` could not have
been the winner without that dispute being settled first.

**RETURN CONDITION — RETURNABLE, conditionally, on two facts that could change.**
`a2` is the design's **named fallback**: *"it survives only as the fallback if A1's Miri gate reports
UB that D5 cannot remove."* That condition was **undecidable** at CS-1, because the Tree-Borrows red
was shared: `a2` red at the identical site. At CS-2 the shared UB is fixed (Stage 1 removed
`Scope::prepare`'s `let wrapped = move ||`, and a scratch mutant that restores exactly that block
reds again — the gate discriminates). So `a2` returns if **either**:

1. a residual UB ever proves **A1-specific** *and* `a3` also fails some later gate — `a3` has no TLS
   deque and no D5 discipline to violate, so it is the cheaper escape from an A1-specific UB and
   would be tried first; **or**
2. the ECS **criterion-versus-wall** dispute is settled in the wall's favour **and** a re-measure in
   `a3`'s own session closes the 1.6445× on worker `10us_4W`.

---

## `a5` — idle-keyed placement into a sibling's `injector_local` (implies `a2`)

> **This row exists mainly to record that a candidate was NOT eliminated by its predicted failure
> mode.**

**Was.** The owner's mechanism 3 in buildable form: when the idle mask is non-zero, claim one idle
bit, push into **that** worker's `injector_local[target]` (stealable, because `a2`'s scan set covers
it), and unpark it; fall back to the own injector when the mask is zero. It applies to worker and
dispatcher pushes alike.

**Flag / measured at.** `ke16-a5` (Cargo: `ke16-a5 = ["ke16-a2"]`). CS-1, HEAD `4a363678`, runs r2/r3.

**Eliminated by — §7 Step A rule 2** (quoted under `a1f`).

* physics `in_scheduled_system` **11.784 ms** against `a3`'s **10.354 ms** = 1.1381 → **13.81 %**,
  applied band 4.00 %, threshold **8.00 %** → behind `a3`. Against `a2`: 1.1610 → 16.10 % against
  8.18 % → behind `a2`.
* Normalised by its own run's `O5`: 0.4232 against `a3`'s 0.3735 = 13.28 %, and against `a2`'s
  0.3782 = 11.87 %. **Both readings agree in both directions.**
* Grid corroboration: `a5` regresses worker `10us_4W` at 181 169.5 vs 111 278.65 ns = **1.6281×**;
  `a3` regresses none of `a5`'s.

**⚠ ITS OWN PREDICTED VETO DID NOT FIRE.** The design expected `a5` to lose the **1 µs cells** to a
foreign-line push plus an `idle` CAS plus an **unpark syscall per spawn**, and rule 3 reserves the
dispatcher-route 1 µs veto for exactly `a2` and `a5`. Measured against `a3`, those three cells read
**1.0393 / 1.0361 / 0.971 — three TIES**; and one of the three (`dispatcher/1us_64W`) is one of the
cells whose band is so wide (6.76 %) that it **cannot fire at all**, so the veto was evaluable on
only 2 of its 3 cells. The predicted per-spawn cost did not resolve above the noise **in the median**.
It shows in the **tail** instead: `a5`'s `in_scheduled_system` maximum samples are **15.296 ms** and
**17.190 ms** against p50s of 11.62 / 11.95 ms — and 15.296 ms is the box's unguarded ~15.6 ms park
tick to three figures. `a5` also **creates** a bimodality the baseline did not have (worker `1ms_4W`
MAD/median 0.310, where `a0`'s worst worker cell was 0.059) and **removes** the ECS one
(`par_from_dispatcher/4096`: 0.255 → 0.005).

So `a5` was eliminated **on the consumer**, on the same distribution cell as everyone else — not on
the spawner-side cost the design predicted would kill it.

**RETURN CONDITION — RETURNABLE, and the fact is a workload/host change, not a wish: THE TIMER GUARD.**
`a5`'s entire cost model is one unpark syscall per spawn while any worker is parked, and on this box
**an unguarded `park_timeout` of any nominal duration below ~15.6 ms costs milliseconds**, measured
three ways at CS-1 (Step 0: 15.585 / 15.596 / 15.574 ms for 50 µs / 1 ms / 2 ms — the 50 µs request
overshot **312×**; `a5`'s own three runs 15.517–15.540 ms; the head-to-head 15.505–15.550 ms on both
arms). The bench's own configuration line reads `configuration=unguarded` in every session, with the
App-12 reference **1021 µs guarded against 15 296 µs unguarded**. Owner ruling 4 (`KE16-DESIGN.md`
§6, *"Timer resolution — RAISE IT"*) puts App-12's raise into the host layer, to be measured both
ways. **That is a scheduled change to the world, and `a5`'s syscall price is exactly what it
changes.** A re-measure of `a5` under the **guarded** configuration is warranted; **nothing else
about `a5` should be reconsidered without it.**

*(One caveat travels with that floor: it is itself session-dependent. At CS-2 the same three
unguarded rows read **11.160 / 11.479 / 11.921 ms** and were **bimodal on all three**, against
CS-1's tight ~15.5 ms. Only the ORDER — milliseconds, not microseconds — is stable across sessions.)*

---

# §2. Axis B — `b1` and `b3` are UNREACHABLE, not defeated

**Neither was ever measured, and neither CAN be measured over `a3`.** This is not a defeat; it is an
unreachability, and the register must not file it as a loss.

**What they were.**

* **`b1`** — the worker joiner stops using the private `scratch`: identified by `worker_lane_for`, it
  pops its **own registered deque** one task at a time, batch-steals from `injector_global` and from
  random self-skipped siblings **into that registered deque**, and re-checks `is_drained` between
  tasks; no `&Worker` is live while a body runs. **B1-P**: when it must park it parks the way
  `worker_main` parks (`mark_idle` → post-mark re-poll → `park_timeout` → `unmark_idle`), so a parked
  joiner becomes a **claimable lane** for any other wave. The external joiner steals one task at a
  time with a Backoff snooze and has no `scratch`.
* **`b3`** — `b1`'s worker arm (B1-P included) plus an external joiner that **never helps**:
  `is_drained` → `unpark_one_idle` → Backoff snooze → timed park. Under `b3` the acceptance line's
  reference row changes meaning (`bench_thread_install` becomes the W-lane row).

**Flags.** `ke16-b1`, `ke16-b3`.

**Why they cannot be built over the winner — the refusal, quoted from `crates/boyko_threadpool/src/lib.rs`:**

```
KE16 axis B: `ke16-b1` / `ke16-b3` require `ke16-a1` or `ke16-a1-fifo` — the worker joiner
needs a REGISTERED destination deque, which only the A1 arms give it; A2/A3/A5 leave the
joiner at B0
```

This was **verified mechanically, not assumed**: `cargo check -p boyko-threadpool --features
ke16-a3,ke16-b1` exits **101** with that message. Under `a3` the worker joiner has no registered
deque, so B1 has no destination.

**The design pre-committed the response and it was followed.** `KE16-DESIGN.md` §3 and the B1(i) row
name this exact trigger — *"A2/A3/A5 wins Step A beyond the band"* — and classify it as *"a re-plan
point, not a fallback taken silently"*, with the index's imperative: **"stop and report, do not
improvise B1(i)"**. §7 Step A rule 6 fired accordingly.

**⚠ THE CONSEQUENCE, STATED RATHER THAN HIDDEN.** `KE16-DESIGN.md` §1: *"every A-fix promotes B onto
the production path"* — defect B never fires on the ECS frame path today only because `Scope::drop`
returns on its first `is_drained()`. **Shipping `a3` therefore puts defect B live on the frame path**:
the joiner batch-steals into an **unregistered** `scratch` and runs **~33 of 64 bodies inline**. This
pass supplies **no remedy**, and `b1`/`b3` cannot supply one over the `a3` substrate.

**The cost of that is bounded, and it is already inside the winner's numbers** — every `a3` figure in
this campaign is `a3+b0+w0+c0`, i.e. **defect B live**. On that configuration `a3` clears acceptance
clause (1) with 2.4× of headroom (11.709 ms against `single_threaded_O5` 28.096 ms) and clause (2) at
93.5 % of budget; against the Step-0 shipping-route reference of 36.02 ms it is **3.08×** faster.
(Bench-profile, indicative; the formal take is Step App, which has not run.)

**RETURN CONDITION — concrete, and it is an axis-A event, not an axis-B one.**
`b1`/`b3` become reachable **if and only if an A arm with a registered worker deque wins axis A** —
i.e. `a1` or `a1f` — or if defect B is fixed by some other route that gives the worker joiner a
registered destination deque. The registered-`scratch` alternative (B1(i)) is **explicitly not to be
reached for**: it would require publishing a stack-lived deque's `Stealer` into `inner.stealers` for
the join's duration and withdrawing it while a sibling may be mid-`steal_batch_and_pop` — a
hazard-pointer or epoch protocol on the registry.

Because `a1f`'s return condition (§1) is the first thing to re-measure, **`b1`/`b3` are downstream of
that single measurement.** Until then, defect B on the frame path is an **open re-plan item**, not a
closed question.

---


# §3. Axes W and C — four arms measured and rejected (2026-09-08, CS-4)

Axis W closed on **`wc`** (`ke16-w-count`, W-d′ count-gated completion) and axis C on **`c0`** — no
batch spawn. The shipped configuration is **`a3+b0+wc+c0`**.

All four rows below are **CS-4** — after stage 3b (`d51b4ced`) moved the scoped task cell into a
per-scope bump block. They were taken with `scripts/ke16_measure.sh`, three interleaved passes per
step, after `--prebuild` so that no timed region followed a compile.

**One sentence covers all four eliminations, and it is the finding rather than the verdict: on this
pool the wake is not overhead, it is the parallelism.** Every rejected arm here trades wake BREADTH
for wake COUNT. The saving each one makes is real and microscopic — a fence, an `idle` load,
sometimes a CAS per push — and it is worth nothing against a lane that stayed parked. The measured
loss tracks the remaining width almost exactly: 2 lanes give 2.00×, 10 lanes give 8.00×, 16 give 15×.

⚠ **The winner is the exception that proves it.** `wc` is the only candidate of the five that does
not touch the WORKER wake at all — it changes the JOINER's (one unpark per scope instead of one per
completion). It measured a tie on all 35 comparable cells and was kept for a soundness reason, not a
speed one.

---

## `wg` — W-b: wake iff the pre-push length was ≤ 1, plus the thief-residue cascade

**Was.** Two mechanisms under one flag. The push-side gate: a push wakes one worker only if the
destination held at most one task immediately before it (`worker::wake_after_push`). The cascade: a
thief left with residue in its own registered deque wakes one worker other than itself
(`worker::wake_after_residue`, from `pop_global_injector` and `try_steal_random`). The cascade is
where the O(log W) fan-out was supposed to live.

**Flag / measured at.** `ke16-w-gate`. CS-4, HEAD `d51b4ced`, three interleaved passes.

**Eliminated by.** Step W rule 1 on BOTH clauses: it regresses `worker/body_1us_tasks_64w` by
**× 1.42** (a 1 µs cell, clause (a)) and improves **nothing anywhere** (clause (b)). 13 of 38 cells
regress, 0 improve.

**The number that actually decided it is an occupancy integer, not a time.** The ECS protocol pass
reports `max_in_flight` = **5–6 of 16** with the speedup pinned at **2.00× in every pass**, against
the reference's 16 and 13.9–15.7×. W-b does not make the pool slower by adding work; it leaves ten
lanes parked. On a wave pushed to one destination only the first two pushes see a `pre_len` of 0 and
1, so two workers are woken, and the cascade restores width one unpark-round at a time — too late
for the wave. Worst on the `× W` cells: `dispatcher/body_1ms_tasks_w` × 5.51,
`dispatcher/body_100us_tasks_w` × 4.69, `worker/body_100us_tasks_w` × 3.66.

⚠ **`KE16-DESIGN-W.md` §2.4 named this outcome as its own risk clause, on exactly these cells** —
*"the 100 µs–1 ms × W cells, where the chain's 4 hops of Windows wake latency plus the one-body stall
may exceed the W serial unparks the spawner paid before"* — and predicted the GAIN at
1 µs × {4W, 64W}, which measured a tie and a regression. Both halves of the prediction failed in the
same direction.

**RETURN CONDITION.** Reconsider **if the pool ever gains a wake mechanism whose width does not
depend on a serial cascade** — an eventcount or a futex-style broadcast that wakes `k` lanes in one
call. W-b's arithmetic is sound; its cascade is what cannot keep up. A second, independent trigger:
**if the body sizes the consumers produce ever move below ~1 µs**, where the per-push fence and
`idle` load stop being negligible against the body. Neither is a wish: both are facts about the
world that could change.

---

## `wgc` — W-b and W-d′ together

**Was.** `ke16-w-gate` + `ke16-w-count`, measured because the design declares the axes composable.

**Eliminated by.** 14 of 38 cells regress, 0 improve — `wg`'s profile essentially unchanged, which is
the expected reading given `wc` measured a tie on its own. It additionally regresses
`dispatcher/body_1us_tasks_64w` × 1.51.

**RETURN CONDITION.** **CLOSED as a pair.** It has no independent existence: it returns if and only
if `wg` returns, and then only as `wg` composed with the shipped `wc`.

---

## `c1` — App-4: `Scope::spawn_batch`, one `pending.fetch_add(n)` per wave

**Was.** One `pending` RMW per wave instead of `n`; `n` insertions of which the first is
`push_task_no_wake` and the rest `push_task_silent`; the wave's single wake decision taken right
after the first push. Callers reach it through the `KE16_SPAWN_BATCH` const — `Query::par_iter` /
`par_chunk` and the physics colored / soft / emit dispatches.

**Flag / measured at.** `ke16-c-batch`. CS-4, HEAD `7ccab360`, three interleaved passes over
`{c0, c1, c1f}`.

**Eliminated by.** Rule 1 clause (b): **0 improvements** on 38 cells, 6 regressions. Physics
`in_scheduled_system` **× 1.52** against a 4.3 % band; ECS `par_in_system/65536` **× 7.38**.

**The occupancy integer is this arm's own source comment read back.**
`src/scope.rs::Scope::wake_for_wave` says a build with `ke16-c-batch` alone takes a wave *"down to
the spawner plus ONE, with the rest asleep for the whole wave"*. Measured: **`max_in_flight` = 2 of
16**, in all three passes, speedup pinned at 2.00×. Not about two — two.

⚠ **The `pending` saving is real and was never the problem.** One `fetch_add` per wave against `n`
is a genuine reduction in multi-writer RMW traffic. It is invisible because the same change collapses
the wave's wake width, and a parked lane costs orders of magnitude more than a contended increment.

**RETURN CONDITION.** Reconsider **if the wave's wake width is ever decoupled from the wave's push
count** — i.e. if the pool gains a wake primitive that activates `min(n, idle)` lanes in ONE
operation, without a per-push decision and without a cascade. `c1f` below is the closest attempt in
this tree and it reaches only ~10 of 16. Concretely: **if `max_in_flight` under `c1f` ever reads 16 at
N = 65536**, this row is worth re-measuring, because at that point App-4's accounting would be the
only thing left in the delta.

---

## `c1f` — W-f: fan-out over one snapshot of the idle mask at the batch's first push

**Was.** `worker::wake_up_to` — one `publish_fence` then at most `n` claim+unpark rounds over ONE
entry snapshot of the idle mask, i.e. `min(n, popcount(idle))` unparks instead of one wake plus
W-b's cascade. Implies `ke16-c-batch`; obeys the W gate rather than being exempt from it.

**Flag / measured at.** `ke16-w-fanout`. CS-4, HEAD `7ccab360`, same three passes.

**Eliminated by.** Rule 1 clause (b): **0 improvements**, 5 regressions. Physics
`in_scheduled_system` × 1.24; ECS `par_in_system/65536` × 1.86.

**It works, and it is still not enough.** `max_in_flight` goes 2 → **9–10 of 16**, speedup 2.00 →
**7.95–8.00×**. The single snapshot is the limit: a fan-out taken at the wave's first push reaches
only the siblings already parked at that instant, which on these shapes is about half of them.

⚠ **THIS ROW EXISTS BECAUSE THE PROTOCOL'S OWN RULE 3 WOULD HAVE PREVENTED IT.** §7 Steps W/C rule 3
says *"`c1` not kept ⇒ `c1f` is not measured"*, on the premise that `c1` is the mechanism and `c1f` a
width refinement. That premise is false here — `c1` alone is App-4's accounting MINUS up to W/2
lanes — and the design had escalated the resulting fork as a design call, offering two ways to make a
`c1` row interpretable: over `ke16-w-gate`'s cascade, or over `ke16-w-fanout`. **The W step deleted
the first.** Applying rule 3 literally would have filed *"App-4 rejected"*; the pair shows
*"App-4's accounting is invisible under every wake width this tree can build"*, which is the
statement a future reader needs.

**RETURN CONDITION.** Same as `c1`'s and inseparable from it: reconsider **if a wake primitive
appears that reaches all idle lanes from one operation**. A cheaper partial trigger, testable without
new primitives: **if re-reading the idle mask per round is ever shown not to double-wake** — the
single snapshot is there to bound the unpark count, and a correct re-read would raise the ceiling
above ~10. That is a soundness question with a definite answer, not a preference.



---

# §4. The methodological finding that any revival must inherit

**This matters more than any single verdict in this file, and a revived candidate that ignores it
will produce a number nobody can rank.**

The head-to-head that closed axis A was run **with a game running, at the owner's instruction**, and
it was **TIGHTER than the quiet cross-session runs**:

| | interleaved, under load | across sessions, quiet |
|---|---|---|
| `single_threaded_O5` (arm-independent by construction) — within an arm | 1.86 % / 0.99 % | up to **17 %** |
| — between arms | **0.68 %** | 3.8 % |
| headroom of the deciding physics margin | 7.54 pp | 1.20 pp |

**The dominant error term in this campaign was SESSION DRIFT, not ambient load.** Interleaving cancels
drift; a steady load does not survive as a bias in a ratio. The line worth keeping: **the ratio that
ranks the arms reproduced to within 0.7 % across a session change that moved both absolutes by
12–13 %** (cross-session `a1f`/`a3` = 1.173, same-session = 1.165). The quantity the decision depends
on is the quantity that survived; the quantity that did not survive (absolute ms) is the one nothing
depends on.

The one load asymmetry was named and refuted rather than buried: the game ran 8–10 % hotter during
`a1f`'s physics windows, which favours `a3` — and `a1f`'s p4 window ran **lighter than every one of
`a3`'s** and still measured 13.640 ms against `a3`'s worst pass of 12.019 ms.

**⚠ Occupancy and the wall clock DISAGREED.** In the occupancy printout on the 200 µs shape `a1f`
spreads the wave better — `top_lane` **15–22 of 64** against `a3`'s **17–33** — and it loses every
timed comparison. The owner's criterion is **throughput, not core
occupancy**: the wall clock ranks, occupancy is a diagnostic. A revival that argues from occupancy
argues from the instrument that lost.

---

# §5. Summary — one line per candidate

| Candidate | Flag | Code state | Status | The number | Return condition |
|---|---|---|---|---|---|
| `a1f` | `ke16-a1-fifo` | CS-1 → **CS-4** | ⚠ **RETURNED 2026-09-08** | ALONE it still loses (0 improved, 5 regressed at CS-4). But it is the only placement `b1` builds over, and `a1f+b1` beats the shipped `a3+b0+wc+c0` on 10 of 35 cells — deciding cell 55.6 vs 117.8 µs | **CONDITION MET.** It returned exactly as written: on the arm below becoming reachable |
| `a1` | `ke16-a1` | CS-1 | REJECTED (rule 2) | physics 16.411 vs 10.354 ms = 58.50 % against a 26.96 % threshold | **RETURNABLE, second** — re-measure in a session with `O5` spread ≤ 1 % and a same-session `a0` control |
| `a2` | `ke16-a2` | CS-1 | REJECTED (rule 3) | worker `10us_4W` 182 993.5 vs 111 278.65 ns = 1.6445×, 8× the threshold | **RETURNABLE, conditional** — an A1-specific UB **and** an `a3` gate failure; or the ECS wall-vs-criterion dispute settled and the 1.6445× closed in `a3`'s session |
| `a5` | `ke16-a5` | CS-1 | REJECTED (rule 2) | physics 11.784 vs 10.354 ms = 13.81 % against 8.00 % | **RETURNABLE** — **if the App-12 timer guard ships**; nothing else about `a5` should be reconsidered without it |
| `b1` | `ke16-b1` | **CS-4** | ⚠ **RETURNED AND WINNING 2026-09-08** | over `a1f`: the deciding cell 2.12× faster than shipped, 4W cells 2.4–3.8×, both consumers a tie, `top_lane` 4 against 13/24/21 — defect B's signature GONE. Full §8 gate ladder green | **CONDITION MET, by the second clause.** `KE16-DESIGN-B4.md` BLOCKING 3 predicted this fires; it was tested BEFORE writing the remedy and it fired |
| `b3` | `ke16-b3` | — | **NOT MEASURED — now reachable** | same refusal lifted with `a1f`; it differs from `b1` only in the external joiner's policy (park vs help) | **OWED**: rule 2 of Step B decides `b3` vs `b1` on the dispatcher-route 100 µs and 1 ms × 4W cells. `b1` is measured; `b3` is not |
| `wg` | `ke16-w-gate` | CS-4 | REJECTED (rule 1, both clauses) | 13 of 38 cells regress, 0 improve; `max_in_flight` **5–6 of 16**, speedup pinned 2.00× | a wake mechanism whose width does not depend on a serial cascade (eventcount / futex broadcast); or consumer bodies below ~1 µs |
| `wgc` | `ke16-w-gate,ke16-w-count` | CS-4 | REJECTED (rule 1) | 14 of 38 regress, 0 improve — `wg`'s profile, `wc` being a tie | **CLOSED as a pair** — returns iff `wg` returns |
| `wc` | `ke16-w-count` | CS-4 | ✅ **KEPT — W\*** | tie on all 35 comparable cells, re-tested on clean data: 0 regressions; both gates green | — (shipped) |
| `c1` | `ke16-c-batch` | CS-4 | REJECTED (rule 1, clause b) | 0 improvements, 6 regressions; `max_in_flight` **2 of 16** — the arm's own source comment, measured | the wave's wake width decoupled from its push count; concretely, `c1f` reading `max_in_flight` = 16 at N = 65536 |
| `c1f` | `ke16-w-fanout` | CS-4 | REJECTED (rule 1, clause b) | 0 improvements, 5 regressions; recovers 2 → **9–10 of 16**, 8.00×, still × 1.86 | same as `c1`, inseparable; or a per-round idle re-read shown not to double-wake |

**No row in this file is CLOSED.** Every one carries a fact about the world that could change — and
on 2026-09-08 two of them CHANGED. `b1`'s return condition named its own trigger and the trigger
fired; `a1f` came back with it, because it is the only placement `b1` builds over. **A register
whose rows can return is worth keeping only if somebody eventually tests the condition. This one was
tested, and it cost the campaign its axis-A verdict.**

---

# §6. What is owed before the flags are deleted

1. ✅ **Commit and tag the arms — DONE.** `897c812f`, tagged `ke16/candidates-frozen-2026-09-07`.
   ⚠ That commit predates stage 3b, so a recovered arm is written against the old task path; the
   removal commit's parent gets its own tag for the shipped one.
2. ✅ **Axes W and C — MEASURED 2026-09-08 at CS-4.** W closed on `wc`, C on `c0`; the four rejected
   arms are §3 above.
3. **The App step — STILL OWED.** A re-take on the **unconditional** code after feature removal
   (CS-3). The design requires the numbers on record to come from the code that ships, and **no
   number anywhere in this campaign does**. It has not been run.
4. **Defect B on the frame path — STILL LIVE** under the shipped winner, at 192.1 ms against a 60 ms
   budget. ⚠ Its remedy is not local: `KE16-DESIGN-B4.md` BLOCKING 3 shows that fixing B by any route
   giving the worker joiner a registered destination deque fires the `b1`/`b3` return row above, and
   those build only over `a1`/`a1f` — so the comparison becomes `a3+b4` vs **`a1f+b1`**, and the
   axis-A closure is re-opened until that is measured.
5. **A physics ranking for axis W — owed but NOT on the critical path.** W\* was fixed by gates
   rather than by numbers, and Step App re-takes every consumer row anyway.