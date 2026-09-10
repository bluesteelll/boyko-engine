# KE16 — Pool occupancy: the design (index)

**Status:** design, revision 4 (after the critic's third REVISE verdict), 2026-09-02, branch
`feat/threadpool-ke16`, HEAD `b6c41237` plus the untracked harness, worktree `D:/wt/threadpool`.
**This is a decision about what to BUILD AND MEASURE, not a recommendation of a winner.** The
catalogue (`KE16-DESIGN-SPACE.md` and its variant files) is the map; this file and its five
companions are the plan the implementers execute literally and the testers measure literally. The
number decides. Every code-level claim carries a `file:line` read at this checkout.

| File | Holds |
|---|---|
| `KE16-DESIGN.md` (this file) | the problem (§1), the candidate table (§2), the tournament order (§3), the feature-gate scheme (§4), out-of-scope items (§5), owner questions (§6), the doc comments the pass corrects (§7), the soundness obligations (§8), the revision record against the critic's blocking items (§9) |
| `KE16-DESIGN-A.md` | placement — defect A: A1 / A1-fifo / A2 / A3 / A5 at code level; the ONE joiner/spawner identity predicate; the raw-pointer lane discipline D5; the producer-side StoreLoad barrier; A2′, A4, P6 argued away |
| `KE16-DESIGN-B.md` | the joiner — defect B in the A-fixed configuration: B0 / B1 / B3; B2 and B1(i) argued away |
| `KE16-DESIGN-W.md` | wake and completion: W-a (base: fenced prologue, rotor behind the mask), W-b (the ≤1 push gate + self-excluding cascade), W-d′ (count-gated completion with a `WakeHandle` target), App-4 batch spawn, W-f fan-out; W-c, W17 argued away |
| `KE16-DESIGN-APP.md` | caller-side items shipped regardless of the tournament; S6/S10 scoping; the L10 instrument |
| `KE16-DESIGN-MEASUREMENT.md` | the tournament: order, commands (with full Miri flag strings), metrics, decision rules, noise, receipts, vacuous-pass shapes, red classification |

Cross-references are by **variant id** (`P1`, `G4`, `W20`, …) and **file**, never by a plan file's
line number. Code line numbers refer to `D:/wt/threadpool` at this checkout and will move once the
pass edits the files; each is given so the developer can find the site, not as a coordinate to be
preserved.

The owner's criteria bind every line: **"Simply the most performant variant."** Throughput on the
real consumers decides; occupancy is a diagnostic. **"If it needs synchronisation heavier than the
gain from maximum core loading, it should not be done."** Any per-spawn or per-completion shared-line
RMW must pay for itself on the 1 µs cells of the grid or be removed. **"Keep defect B as is"** is a
reportable outcome (B0 is built and measured).

## 1. The problem, in one paragraph

A task spawned from inside a worker of pool `P` lands in `P.injector_local[wid]`
(`crates/boyko_threadpool/src/worker.rs:365-376`); only worker `wid` polls that queue
(`worker.rs:216-223`), the sibling scan walks `inner.stealers` only (`worker.rs:233-257`), and the
worker that spawned the wave is inside a task body, not in `worker_main` — so the wave is **exactly
serial** on that worker (defect A): pool harness `max_in_flight = 1` at W=4 and W=16, wall-clock at
the `tasks × body` serial floor within 1 % for every body ≥ 10 µs; ECS harness `par_iter` inside a
scheduled system 1311.99 ms vs 121.27 ms from the bench thread at N=65536 (10.82×). On the real
consumer — the colored solve over a 10 k-body pyramid, 29751 contacts, 6 colors — the shipping route
(`physics_solve_colored` as a scheduled system, body on a worker) costs **36.02 ms** against
**11.57 ms** for the identical step from the bench thread and **31.52 ms** single-threaded: enabling
parallel physics as wired today costs 14 % MORE than leaving it off, and every O-series number on
record was taken on the healthy route. Independently, the joining thread steals into an
**unregistered** private `scratch` deque and runs the batch inline (`scope.rs:448,479,488,496,524-531`,
defect B): not an in-flight cap (the dispatcher route reaches 15–17 live at W=16) but a
work-distribution defect — the joiner executes 33 of 64 bodies itself in most runs (bimodal: also 21,
14, 8; the ≤33 batch of crossbeam's `steal_batch_and_pop`) and wall-clock tracks that count. B never
fires on the ECS frame path today (`Scope::drop` returns on its first `is_drained()` at
`scope.rs:463`) and is subsumed by A on the nested path; **every A-fix promotes B onto the
production path**, so B is decided only in the A-fixed configuration. Two further per-task costs sit
on the same path: an unconditional `waker.unpark()` + `pending.fetch_sub` per completion
(`scope.rs:157-160`) and a `wake_rotor` RMW before the idle-mask load on every push
(`worker.rs:324-326`) — the latter being, by accident, the only producer-side StoreLoad barrier of
the wake protocol once the publish is a plain store. Dispatcher-route speedups never reach W
(1.45×–13.5× over the grid) — the shortfall is the joiner's inline share; at 1 µs × W the wave costs
4.82 µs against a 1 µs ideal, so that cell is where added synchronisation shows.

## 2. Candidate table

`build` = a cargo feature exists for it and it goes through the tournament. `predicted` is what the
grid should show and by roughly how much, with the evidence tag from the catalogue legend; `loses if`
is the measured outcome that eliminates it. Cells: the pool grid is `{dispatcher, worker} × body
{1 µs, 10 µs, 100 µs, 1 ms} × tasks {W, 4W, 64W}` at W = `available_parallelism` (16 on the bench
box); "the consumers" are `ke16_solve_in_system` (physics) and `ke16_par_iter_in_system` (ECS).

### Axis A — placement (defect A). Detail: `KE16-DESIGN-A.md`

| id | catalogue | build | summary | predicted | loses if |
|---|---|---|---|---|---|
| **A1** | P1 + L1 (E19) | **yes** `ke16-a1` | same-pool worker spawn → the worker's own registered Chase-Lev deque, reached through ONE predicate `tls::worker_lane_for(inner)` (pool-tagged TLS raw pointer AND `current_worker_id() < worker_count`, so an `install` frame on a worker is external, as `push_task` treats it today); the lane is `Copy` with a RAW pointer and every `&Worker` is consumed by one method call in its own statement (D5 — no protected tag spans a task body, no use after a body); deques `new_lifo()`; `injector_local` no longer fed; the spawn path's queue op is a plain store (no RMW) followed by the fenced wake decision (`publish_fence`, one local `mfence`, shared by every arm). **The LIFO trade is two-sided** (`KE16-DESIGN-A.md` §1.4): the owner's pop is a local fence + a shared read instead of FIFO's contended `front.fetch_add`, BUT a thief's batch steal from a LIFO deque is one `SeqCst` CAS + one fence PER STOLEN ELEMENT (crossbeam `deque.rs:1077-1142`), up to 33 per batch, against FIFO's one CAS per batch (`:1034-1071`) | worker route reaches W at every cell; on the ONE-spawner shape (a wave of W or 4W–6W chunks, stolen fraction `s ≈ 0.94`) LIFO pays ≈ `s` shared RMWs per task on the victim's `front` line vs FIFO's ≈ `(1 − s) + s/33` — so by count A1-fifo, not A1, is the lighter steal path at 1–10 µs × 64W; A1 wins there only if FIFO's same-end contention (owner `fetch_add` vs thief batch CAS, retried without backoff) costs more than LIFO's per-element CASes, and wins physics if own-wave-first order and chunk locality show (ABP Thm 9 `[P]`; rayon LIFO + `steal()` `[S]`; Tokio/Go FIFO + steal-half `[S]`; axis 38 `[I]`); physics `in_scheduled_system` ≈ `REF` once B1 is under it | decided SYMMETRICALLY against A1-fifo on the 1–10 µs × 64W worker cells and on physics (`KE16-DESIGN-MEASUREMENT.md` §7 Step A rules 2–3): behind A1-fifo if it regresses those cells beyond 2× the band while A1-fifo regresses none of its, or if physics is worse beyond the band; behind A3 if any 1 µs worker cell is worse than A3's by more than 2× the band (per-element deque CASes costing more than the shared injector — now a real possibility, not "not expected"); a Miri UB REPORT (never a timeout, never a receipt miss — §8) that D5 cannot remove |
| **A1-fifo** | P1 alone | **yes** `ke16-a1-fifo` | A1 with `new_fifo()` deques (today's end discipline) — the LIFO/FIFO sub-choice isolated: owner-pop is `front.fetch_add(SeqCst)` (one multi-writer RMW per executed own task, `deque.rs:463`) but a thief's batch is ONE CAS per ≤33 (`:1034-1071`), so on the one-spawner shape it has the lighter steal path by an order of magnitude in RMW count; its unpriced risk is same-end contention (the owner's `fetch_add` and the thieves' batch CAS both hit `front`; a failed batch CAS is a full re-copy retried with no backoff, `worker.rs:262-279`) and, under B1, a joiner that pops a batch-stolen sibling system before its own chunks | predicted to win 1–10 µs × 64W by RMW count unless the same-end contention mode materialises at W=16; ≥100 µs cells equal to A1; on physics/ECS ≤ A1 only if the B1 joiner's oldest-first order costs latency (FJP ships FIFO-local only for tasks never joined `[D]`) | the mirror of A1's rule: behind A1 if it regresses a 1–10 µs × 64W worker cell beyond 2× the band while A1 regresses none, or physics is worse beyond the band; a tie everywhere goes to A1-fifo (today's constructor, smaller diff, no reversal path reachable) |
| A2 | P2 (E1) | **yes** `ke16-a2` | keep `injector_local`, add it to the sibling scan set; cfg shape `all(ke16-a2, not(ke16-a5))` for the pure arm | reaches W; spawn path unchanged from today (two single-writer RMWs + `pending` + the fenced wake decision); idle path +W−1 `SeqCst` fences per round (`deque.rs:1795-1830` `[L]`); loses to A1 at 1 µs × 64W and on `empty_schedule_control`; N5 (Go #28808) is the prior `[D]` | that is the expectation; it survives only as the fallback if A1's Miri gate reports UB that D5 cannot remove |
| A2′ | P18 | no | rayon `JobFifo`: keep `injector_local`, close it by a placeholder on the registered deque | — | argued away: it REQUIRES A1's TLS pointer and then adds an `Injector` CAS + a placeholder push per spawn and an `Injector::steal` per execution on top of A1; its only benefit (FIFO for spawns behind the deque's older work) is the row A1-fifo already occupies. rayon RFC 0001 "performs equivalently" `[R:D]` is relative to rayon's own deque path, which is A1 |
| **A3** | P3 | **yes** `ke16-a3` | every spawn → `injector_global` (one line at `worker.rs:369-374`); **the control / reachability floor**; pays the same fenced wake decision as A1, so the A1-vs-A3 delta is pure transport; its STEAL side is one head CAS per batch (`deque.rs:1851`) — lighter than A1's per-element deque CASes, heavier than nothing | reaches W; per task on the one-spawner shape ≈ 2 multi-writer RMWs on the spawner (the injector tail CAS + slot `fetch_or`, on the one line every spawner and thief touches) + 1/33 on the thief — expected to lose to A1-fifo (≈ 0.09) at 1–10 µs × 64W, and to sit near A1 (≈ 0.94) there, the order between those two being the measured question (axis 38; Bevy/async-executor complaints `[D]`); indistinguishable at 1 ms | it is the control — it "wins" only if neither deque arm beats it beyond the band, in which case locality buys nothing at this granularity and the one-line fix ships |
| A4 | P4 | no | Tokio/Go bounded ring + spill half to global | — | fallback only if A1's TLS pointer is unsound under Tree Borrows (a UB report); the obligation is discharged by construction (D5: raw pointer, per-call reborrows, §8) and by two Miri shapes; same spawn cost as A1 for a larger rewrite (N8–N11 `[S]`/`[B]`) |
| **A5** | P17 (M3-s) | **yes** `ke16-a5` (implies `ke16-a2`) | the owner's mechanism 3 in its buildable form: when the idle mask is non-zero, claim one idle bit, push into THAT worker's `injector_local[target]` (stealable — A2's scan set), unpark it; own injector when the mask is zero; applies to worker and dispatcher pushes alike | loses at 1–10 µs × {4W, 64W}: a foreign-line `Injector::push` (SeqCst CAS + slot `fetch_or` on another core's line) + an `idle` CAS + an `unpark` syscall per spawn while any worker is parked (NA-RP > 100 ns per pushed task `[P]`; Bevy 10–70 µs per OS wake `[D]`; Gast λ term `[P]`); may beat A1 at 100 µs–1 ms × W where siblings are parked at the wave boundary | any 1 µs cell (either route — A5 changes the dispatcher's pushes too) worse than A1's by more than 2× the band — expected; it survives only if it also beats A1 on physics beyond the band |
| P6 | P6 (E17) | no | push-to-idle, NON-stealable | — | rejected in writing by Go (N2) and FJP (N6) and measured as a loss by XGOMP NA-RP (N37); its stealable form IS A5 |
| P5, P7–P16, P19–P25 | — | no | affinity mailbox, private deques + channels, split deque, lifelines, LIFO slot, time window, push-to-all, static, central, lazy task creation, isolation, thread-per-core, Julia, Jolt, ForkUnion, broadcast, Wicked (P24), stlab (P25) | — | each has its reason in `KE16-VARIANTS-PLACEMENT.md` / the addenda; none closes reachability at a lower spawn cost than A1 or is buildable on crossbeam in this pass (P24/P25 are a foreign-line push per spawn without even A5's idle key; P7's own author abandoned it, N63) |

### Axis B — the joiner (defect B), in the A-fixed configuration. Detail: `KE16-DESIGN-B.md`

| id | catalogue | build | summary | predicted | loses if |
|---|---|---|---|---|---|
| **B0** | G3 | **yes** (the default: any `ke16-a*` without `ke16-b*`) | keep `scratch` as is; under A1 the joining worker batch-steals its OWN registered deque via `stealers[wid]` (`scope.rs:534-541`, no self-skip) into `scratch` — a REAL batch of `min((len−1)/2, 32) + 1 ≤ 33` (crossbeam degrades to a pop only when the destination IS the source, `deque.rs:987-991`; `scratch` is not), taken by the per-element LIFO path + the reversal loop into the FIFO `scratch` under `ke16-a1` and by one CAS under `ke16-a1-fifo` — and runs it serially; the parked B0 joiner is never idle-marked, so it is invisible to every other wave's wake decision (a residual if B0 wins, `KE16-DESIGN-B.md` §1) | route (b): `top_lane` ≈ 33 of 64 at 4W (the batch), wall-clock tracks it; physics `in_scheduled_system` above its W-lane reference by the serial residue of the widest colors | B1 beats it on physics beyond 2× the band — expected. It is the owner's reportable outcome if B1 does not |
| **B1** | G4(ii) + J1 (rayon `wait_until_cold` shape) + L2 (App-3) + **J15 (B1-P, new)** | **yes** `ke16-b1` | worker joiner (identified by `worker_lane_for`, a `Copy` lane with a raw pointer): pop its own deque one task at a time through a per-statement `&Worker` (`let popped = lane.deque().pop();` — LIFO end = its own newest chunk), batch-steal from `injector_global` and from random self-skipped siblings INTO its own registered deque, re-check `is_drained` between tasks; no `&Worker` is live while a task body runs; **B1-P: when it must park it parks the way `worker_main` parks — `mark_idle` → post-mark re-poll → `park_timeout` → `unmark_idle` — so a parked joiner is a CLAIMABLE lane for any other wave** (one `fetch_or` + one `fetch_and` per park, never per task); external joiner (dispatcher, fontbake, cross-pool worker, `install` on a worker): steal ONE task at a time, Backoff snooze then the timed park, no `scratch` | route (b): `top_lane` ≈ tasks/W; physics `in_scheduled_system` within the band of `REF` (= `bench_thread_install_Wminus1` under B1); dispatcher route: `top_lane` falls from ~33 to ≈ 64/(W+1), bimodality gone; B1-P's effect shows on physics (16 systems each joining a `par_iter`; a parked joiner now takes a sibling's chunks instead of sleeping to the ≥1 ms backstop), receipt `parked_joiner_is_claimed_by_a_foreign_wave` | worse than B0 on any consumer beyond 2× the band (would mean the residue batch was cheaper than one-at-a-time popping — not expected on ≥10 µs bodies) |
| B2 | G1 on the worker joiner | no | steal one at a time on the worker joiner too | — | dominated by B1 by construction: under A1 the worker joiner owns a REGISTERED deque, so a batch stolen into it is already stealable; steal-one only adds a CAS per task (Dinan `[P]`; HotSLAW `[P]`). B2's shape is kept where it is the only registry-free option: the external joiner inside B1 |
| B1(i) | G4(i) | no | register a `Stealer` for a stack-lived `scratch` | — | needs a dynamic stealer registry with an unregister-before-return hazard; B1's external arm gets "no sink" with zero registry |
| **B3** | J3 (rayon `in_worker_cold`) | **yes** `ke16-b3` (implies B1's worker arm, B1-P included) | external joiner never helps: `is_drained` → `unpark_one_idle` → Backoff snooze (kept, so the frame-path ns window does not fall into the ≥1 ms park) → timed park | dispatcher-route cells lose the joiner's lane: ≤ 1/(W+1) slower at 4W–64W; route (b) unaffected; fontbake's bake (`crates/boyko_fontbake/src/msdf/distance.rs:433`) loses the same lane; **the physics reference rows change meaning**: `bench_thread_install` becomes exactly W lanes and IS the acceptance line's reference under B3, while `bench_thread_install_Wminus1` becomes W−1 lanes (the two do NOT coincide — they differ by one lane; `KE16-DESIGN-APP.md` §11) | dispatcher 100 µs / 1 ms × 4W cells regress beyond 2× the band with no gain elsewhere — expected; wins on code size if those cells are ties (owner question 2) |

### Axis W — wake and completion. Detail: `KE16-DESIGN-W.md`

| id | catalogue | build | summary | predicted | loses if |
|---|---|---|---|---|---|
| **W-a** | W6 (E16) + the fenced prologue | **ships in the base, no feature** | `unpark_one_idle` becomes `publish_fence()` (`fence(SeqCst)`, one local `mfence`) → `idle.load(Acquire) & !exclude` → return if zero → `wake_rotor.fetch_add` → claim. NOT zero behavioural change: it replaces one contended-line RMW (the rotor, which supplied the producer's StoreLoad barrier by accident) with one local full barrier, and makes the SB-litmus argument formal for plain-store transports (A1's push; the thief's residue store). Identical wake-target sequence when a wake happens. `exclude` lets the residue cascade skip the caller's own bit | one multi-writer RMW per spawn removed when nobody is parked, at the price of one local fence; loom M2 drives the production `publish_fence` and a calibration copy without it must go red | nothing measurable on its own — part of every configuration; charged to every arm's wake decision so the A comparison stays transport-only |
| **W-b** | W1 + FJP `signalWork` (the JDK ≤1 rule `[R:S]`) + the thief-residue cascade | **yes** `ke16-w-gate` | a push wakes one worker iff the destination queue held **≤ 1** task before the push (not "was empty": the ≤1 threshold is what covers a thief taking the last task between the length read and the push); a thief left with residue in its own registered deque after a batch steal wakes one worker OTHER THAN ITSELF (the helpers are reached from the post-`mark_idle` re-poll before `unmark_idle`, where an unmasked claim could pick the caller and break the chain at hop one); ONE helper `wake_after_push(inner, pre_len)` at every push arm; a gated-out push pays neither the fence nor the mask load; the joiner's pre-park `unpark_one_idle` and the post-`mark_idle` re-poll stay. Invariant claimed: the WEAK one — no task remains while every worker is parked; the strong "every non-empty queue issued a wake" is NOT claimed | on a wave of N with siblings parked: ≤ 2 claims on the spawner instead of N; fan-out becomes an O(log₂ W) wake chain; the documented cost is a one-body stall when ≥ 2 awake thieves drain a queue between the spawner's length read and its push (no wake; the thieves re-scan after their bodies) — visible, if at all, on 100 µs–1 ms × W | any 100 µs–1 ms × W cell slower than W0 beyond 2× the band (the stall or the chain latency exceeding the saved syscalls), or the physics consumer slower, or any 1 µs cell slower; loom M4 red, or either M4 calibration copy staying green |
| **W-d′** | W20 (E28) | **yes** `ke16-w-count` | count-gated completion for a joiner that `worker_lane_for` identifies as a registered worker of the scope's pool: `ScopeShared.joiner_wake: *const crate::sync::WakeHandle` (null = external), copied out BEFORE `pending.fetch_sub(1, AcqRel)`; `if prev == 1 { (*joiner_wake).unpark() }` — the target is `inner.workers[wid].thread`, `PoolInner`-owned, alive independently of `ScopeShared`; `WakeHandle` = `std::thread::Thread` under `cfg(not(loom))` and a counting newtype over `loom::thread::Thread` under `cfg(loom)` so loom M1c drives the REAL method; an external joiner keeps today's unpark-before-decrement (its window persists on that arm and is stated) | route (b): per task one multi-writer RMW instead of two + a conditional syscall; the joiner's park becomes exact on route (b); no change on the dispatcher route; ECS frame path untouched | any 1 µs cell slower beyond 2× the band (impossible by RMW count; would indicate a bug), or loom M1c red, or the ROUTE-(b) Miri many-seeds gate showing a liveness timeout — a red on either is a defect in the gated arm, to be fixed or the feature dropped; a count-only M1c (yield re-poll fallback) does not count as the liveness gate; W17 is NOT a fallback (same order, same window) |
| W-c | W11/W21 (E18/E31) | no | spin on one word, scan every k-th round | — | argued away: .NET's form presupposes a semaphore post on every enqueue `[S]`, which W-b removes; the idle scan is not on any measured consumer's critical path at W=16; Go #28808 (N5) is a 56-core datum. Revisit only if, after A1+B1+W-b, physics `in_scheduled_system` still trails beyond the band and a profile shows the idle loop |
| W-e | W15 | no | throttle the wake to one per interval | — | a knob with a latency floor; only if W-b over-wakes (a regression at 10 µs × 4W with no gain elsewhere) |
| W9 | W9 | no | never wake for worker-generated work | — | subsumed: W-b's ≤1 wake is the minimum a wave needs; the fully silent form leaves a wave serial until the spawner's own join — rayon's un-fenced internal push is the measured shape of that loss (`KE16-DESIGN-A.md` §1.8) |
| W17 | W17 | no | joiner-published parked flag inside `ScopeShared` | — | keeps the unpark-before-decrement order and hence the lost-wakeup window W-d′ closes; it cannot be the fallback for a W-d′ gate failure because it would fail the same gate identically |
| **App-4** | G20 (E23) | **yes** `ke16-c-batch` | `Scope::spawn_batch(n, bodies)`: one `pending.fetch_add(n)` per wave, n queue insertions, the wake decision taken ONCE right after the FIRST push (so parked siblings start stealing while the spawner is still pushing), the rest silent; callers: `par_iter`, `par_chunk` (`par_chunk.rs:139-260`, the second production `par_*` driver), the three physics sites; fontbake deliberately stays per-task | N−1 multi-writer RMWs and N−1 fenced wake decisions per wave removed on the spawner (~16 × 40–100 ns at W tasks vs the 4.82 µs cell); the publish/steal overlap is kept; Tchiboukdjian Thm 3 `[P]` | no cell better than per-task spawn beyond 2× the band — then the API is not shipped (smaller surface wins a tie) |
| **W-f** | W16/E27 (P24, G22) | **yes** `ke16-w-fanout` (implies `ke16-c-batch`) | at the batch's first push, after one `publish_fence`, wake `min(n, popcount(idle))` workers (one CAS-claim + unpark each) instead of one + cascade | wins at 100 µs–1 ms × W where W−1 siblings are parked and the chain's hop latency (Windows unpark) exceeds W serial unparks on the spawner; loses at 1 µs × W | any 1 µs cell regresses vs W-b + batch beyond 2× the band with no ≥100 µs cell winning beyond 2× the band — expected; measured |

### Axis S / L / App — scheduler, locality, caller side. Detail: `KE16-DESIGN-APP.md`

| id | catalogue | build | summary |
|---|---|---|---|
| App-1 | S9 | **ships regardless** | `lanes = pool.num_threads()` at the three physics sites (never `+1`); the dead `lanes < 2` guard becomes live and correct (W == 1 ⇒ serial) |
| App-2 | L1 | inside A1 | `Worker::new_lifo()` |
| App-3 | L2 | inside B1; the one-line call change ships even if B0 wins | the joiner's sweep randomised and self-skipped (it reuses `try_steal_random`) |
| App-6 | J14 | **ships regardless** | ONE identity predicate for the push arm, the joiner and the W-d′ target: `tls::worker_lane_for(inner)`; a cross-pool joiner and an `install` frame on a worker are EXTERNAL joiners of the target pool; the foreign-install test asserts on a `(pool address, worker id)` receipt |
| App-7 | axis 36 | **ships regardless** (a bench row) | measure `park_timeout(50 µs)` real expiry on the bench box, now in BOTH configurations — App-12 raises the resolution for the shipped host, so every row states whether the guard was held (§6 ruling 4) |
| App-8 | new | **ships regardless** | `InSystemRunGuard` becomes a depth counter: a helping joiner may run a sibling system inline (already reachable today through `injector_global` at `scope.rs:488`; likely under B1) |
| App-9 | M1-0 | **ships regardless** | record the bench box's topology (cores, SMT, L2/L3) in the results file |
| App-10 | new (the critic's round-2 non-blocking item 5; round-3 blocking item 2) | **ships regardless** (a bench row) | `bench_thread_install_Wminus1`: the physics bench-thread route over a `num_threads(W − 1)` pool. The acceptance line's PRIMARY reference `REF` is chosen by LANE COUNT, not by row name: this row (W−1 workers + a HELPING external joiner = W lanes) when the shipped B is B0 or B1; `bench_thread_install` (W workers + a PARKED joiner = W lanes) when it is B3, under which this row is only W−1 lanes and would loosen the line by ≈ 6.7 % at W=16 (`KE16-DESIGN-APP.md` §11; `KE16-DESIGN-MEASUREMENT.md` §7 Step B rule 4) |
| App-11 | new (B1-P's instrument) | **ships regardless** (a diagnostics accessor) | `ThreadPool::parked_mask() -> u64`: one `Acquire` load of `inner.idle`, public, read-only; used by the B1-P receipt test and available to the occupancy harness (`KE16-DESIGN-APP.md` §10) |
| S6 / S10 / M1-a | E15 / E30 | **out of this pass** | separate ticket (§5) |
| L10 | E10 | **not built** | what it would take is stated in `KE16-DESIGN-APP.md` §4; what IS measurable about locality here: the A1-vs-A3 delta at equal reachability and equal wake-decision cost |

## 3. Tournament order (summary — commands and rules in `KE16-DESIGN-MEASUREMENT.md`)

One axis at a time, the others held at the best-known setting; every configuration on the pool grid,
the ECS harness and the physics harness; every configuration run twice.

1. **Step 0 — base and calibration.** Today's code + W-a (with `publish_fence`) + App-1/6/7/8/9/10
   + harness receipts. Records the reference grid, the consumers' reference numbers
   (36.02 / 11.57 / 31.52 ms), the `park_timeout` expiry, the topology, the noise band per cell,
   and the two loom calibration readings (M2 without the producer fence, M4's copies) — a model
   that cannot go red is not a gate.
2. **Step A** — `{A1, A1-fifo, A2, A3, A5}` each with B0 and W0. PHYSICS RANKS (a candidate worse
   on `in_scheduled_system` beyond 2× the band is behind, whatever the grid says); THE GRID VETOES
   SYMMETRICALLY within the physics band (the 1 µs and 10 µs WORKER-route cells compared pairwise
   in both directions; the 64W column breaks a split, because that is where the per-element vs
   per-batch steal CAS shows); the DISPATCHER-route 1 µs cells veto only A2 and A5 — the only
   candidates whose dispatcher-side code differs from today's in a way that can cost; a residual
   tie → the smaller diff / fewer spawner RMWs, with `a1` vs `a1f` going to `a1f`. The LIFO/FIFO
   pair is NEVER separated by an RMW count: its two sides point opposite ways
   (`KE16-DESIGN-A.md` §1.4). If a deque arm is within the band of the winner it is taken, because
   B1 exists only on `a1`/`a1f`. A Miri red on an A1 gate is CLASSIFIED (UB report / liveness
   timeout / receipt miss) before it counts against A1.
3. **Step B** — under the Step-A winner: `{B0, B1, B3}`. Rank on physics and the 100 µs–1 ms × 4W
   dispatcher cells (the fontbake shape); B0 stays if B1 does not beat it beyond 2× the band. The
   Step-B verdict fixes the physics reference row `REF` by lane count: `bench_thread_install_Wminus1`
   under B0/B1 (the external joiner helps: W lanes), `bench_thread_install` under B3 (it parks:
   W lanes).
4. **Step W/C** — under A+B: `W-b`; then `W-d′`; then `App-4`; then `W-f` (only with batch). Each is
   kept only if no 1 µs cell regresses beyond 2× the band and at least one consumer or one cell
   improves beyond 2× the band; W-d′ is kept on the RMW count alone if it is neutral and BOTH its
   gates (loom M1c, the route-(b) many-seeds gate) are green.
5. **Step App** — un-ignore the two red-first gates, re-take the O-series colored-solve numbers on the
   shipping route (with `REF`, the W-lane reference named in Step B, and the other row raw beside
   it), delete every feature and losing branch, run the full workspace gates.

If Step A's winner is not A1 by more than the band, Step B is a re-plan point (B1 needs a registered
destination that A2/A3/A5 do not give the joiner): stop and report, do not improvise B1(i).

## 4. Feature-gate scheme

All features live on `boyko-threadpool` (`crates/boyko_threadpool/Cargo.toml`); nothing else declares
one. **Default = today's behaviour** (plus W-a and the App items that ship regardless), so the tree
stays green with no feature.

```toml
[features]
default = []
scheduler-trace = []
# KE16 tournament switches -- one per candidate, mutually exclusive per axis (enforced by
# `compile_error!` in src/lib.rs), DELETED FROM THE SHIPPED CRATE in the same pass once the
# verdict lands — but FROZEN, not destroyed. Owner ruling, 2026-09-02: a losing candidate is
# kept as a spare, "so the code is recorded but not present in the project". The mechanism is
# in `KE16-DESIGN-MEASUREMENT.md` Step App: one annotated tag at the last commit where every
# candidate still builds, plus a register of what lost, by what number, and under what
# condition it would be worth reaching for again. A shipped crate still carries none of them.
ke16-a1 = []                 # A1: own registered deque, LIFO owner end
ke16-a1-fifo = []            # A1: own registered deque, FIFO owner end (today's end discipline)
ke16-a2 = []                 # A2: injector_local joins the sibling scan set
ke16-a3 = []                 # A3: every spawn to injector_global (the control)
ke16-a5 = ["ke16-a2"]        # A5: idle-keyed placement into a sibling's injector (needs A2's scan)
ke16-b1 = []                 # B1: worker joiner uses its own deque and parks idle-marked (B1-P); external joiner steals one, no scratch
ke16-b3 = []                 # B3: as B1 for workers; external joiner parks and never helps
ke16-w-gate = []             # W-b: wake iff pre-push length <= 1, plus the self-excluding thief-residue cascade
ke16-w-count = []            # W-d': count-gated completion for worker joiners
ke16-c-batch = []            # App-4: Scope::spawn_batch, one pending RMW per wave
ke16-w-fanout = ["ke16-c-batch"]  # W-f: wake min(n, idle) at a batch push
```

Mutual exclusion, in `crates/boyko_threadpool/src/lib.rs` directly below the module list, one
`compile_error!` per illegal pair, each with the message naming the two features and the axis:

- A: every pair among `{ke16-a1, ke16-a1-fifo, ke16-a2, ke16-a3}`; `ke16-a5` with each of
  `{ke16-a1, ke16-a1-fifo, ke16-a3}` (`ke16-a5` with `ke16-a2` is legal — it implies it).
- B: `ke16-b1` with `ke16-b3`; `any(ke16-b1, ke16-b3)` without `any(ke16-a1, ke16-a1-fifo)`
  (B1 needs the TLS deque; A2/A3/A5 leave the joiner at B0).
- W/C: no exclusions (`ke16-w-gate`, `ke16-w-count`, `ke16-c-batch`, `ke16-w-fanout` compose);
  `ke16-w-fanout` pulls `ke16-c-batch` through Cargo.

**The cfg shape for the implied feature.** Because `ke16-a5` enables `ke16-a2`, every site that is
"pure A2" — the A2 arm of `push_task`, the A2 joiner, the witness `KE16_A = "a2"` — is gated
`#[cfg(all(feature = "ke16-a2", not(feature = "ke16-a5")))]`, and the A5 arm is
`#[cfg(feature = "ke16-a5")]`. Sites that A2 and A5 SHARE (the second probe in `try_steal_random`
and in the joiner's sweep) are gated `#[cfg(feature = "ke16-a2")]` alone. Written once here so the
developer does not meet the duplicate-definition error.

**Witness.** `src/lib.rs` exports, under `#[cfg]`, one `&'static str` per axis —
`KE16_A ∈ {"a0","a1","a1f","a2","a3","a5"}`, `KE16_B ∈ {"b0","b1","b3"}`,
`KE16_W ∈ {"w0","wg","wc","wgc"}`, `KE16_C ∈ {"c0","c1","c1f"}` — and
`pub fn ke16_variant() -> String` = `"{A}+{B}+{W}+{C}"`. Every KE16 bench and the two red-first gates
print it on entry and, when the env var `KE16_EXPECT` is set, **panic on mismatch**. The tester's
protocol sets `KE16_EXPECT` on every run, so "features not actually enabled" cannot produce a number.

**Enabling from the consumer crates.** Both `boyko-ecs` (`crates/boyko_ecs/Cargo.toml:8`) and
`boyko-physics` (`crates/boyko_physics/Cargo.toml:24`) depend on `boyko-threadpool` directly, so the
`pkg/feature` syntax applies to a single-package invocation:

```powershell
cargo bench -p boyko-threadpool --bench ke16_nested_scope --features ke16-a1,ke16-b1 -- --save-baseline a1+b1+w0+c0-r1
cargo bench -p boyko-ecs --bench ke16_par_iter_in_system --features boyko-threadpool/ke16-a1,boyko-threadpool/ke16-b1
cargo bench -p boyko-physics --bench ke16_solve_in_system --features boyko-threadpool/ke16-a1,boyko-threadpool/ke16-b1
```

**Keeping the diff reviewable.** One switch point per behavioural difference, marked
`// === KE16 <axis> switch: <feature> ===`, so `grep -n 'KE16' crates/boyko_threadpool/src/*.rs`
enumerates every one. A switch is a `#[cfg]` on the smallest expression or statement that differs;
where two alternatives need whole bodies (`join_workers_until_drained` under B0 vs B1) they are two
adjacent `#[cfg]`-gated functions with the same signature and a shared name. **The W gate is ONE
site**: every push arm ends in `wake_after_push(inner, pre_len)` (`KE16-DESIGN-W.md` §2.2), and the
`#[cfg(feature = "ke16-w-gate")]` lives inside that helper, not in the A arms — so `grep KE16` lists
one W switch and the removal step deletes one site. **The StoreLoad barrier is ONE site**:
`publish_fence()` is the first statement of `unpark_one_idle_excluding`, which every wake decision
calls (`KE16-DESIGN-W.md` §1); no A arm carries a fence of its own. Helpers shared by several
variants (`try_steal_random`, `pop_global_injector`, `drain_one`, `run_task`, `claim_one_idle`,
`unpark_one_idle_excluding`, `publish_fence`, `wake_after_push`, `Scope::prepare`,
`tls::worker_lane_for`, `WorkerLane::deque`) are edited once and called from every arm. No function
body is duplicated beyond the switch points. Doc comments describe the shipped (post-verdict)
behaviour once, with the tournament alternatives named in a single `KE16` paragraph that the removal
step deletes.

**Removal after the verdict.** The winner becomes unconditional code: delete every `ke16-*` feature,
every `#[cfg(feature = "ke16-…")]` and `#[cfg(not(…))]` site, the `compile_error!` block, the
`KE16_*` consts, `ke16_variant()`, and the `KE16_EXPECT` check; the benches keep their route
receipts. Gate: `grep -rn 'feature = "ke16' crates` returns nothing, and
`cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets -- -D warnings`
are green with no feature. The losing branches leave no `#[allow(dead_code)]` behind.

## 5. Out of scope (recorded, not built here)

| Item | Why not this pass | Where recorded |
|---|---|---|
| **S6** apply-window early release (shortlist row "mechanism 1, item b") | an ECS-scheduler change with its own soundness question (SCH7: a successor must not observe a predecessor's deferred commands before the drain, `schedule.rs:603-611`); its share of the observed lane idling is unmeasured (shortlist row "mechanism 1, item a") and independent of every pool candidate; the owner's mechanism-1 precondition is already met (assignment is dynamic, `schedule.rs:967-1326`) | `KE16-DESIGN-APP.md` §3; new backlog row to open in `docs/aether-v2/KERNEL-BACKLOG.md` (KE17) by the orchestrator at pass end |
| **S10** inline successor (M1-c) | presupposes S6 | same |
| **M1-a** the apply-window-share instrument | the ECS `RoundProbe` (`zones.rs:222-260`) already records round widths; sizing the barrier's share needs a conflicting schedule with unequal costs — a separate measurement campaign | same |
| **A lifetime-independent wake target for EXTERNAL joiners** (a leaked per-thread `'static` `WakeHandle` slot, decrement-then-unpark UNCONDITIONAL for the frame scope) | would change the frame path's completion order (an ECS per-system-wake decision); the external arm's window is masked on the frame path by the Backoff snooze and costs at most one ≥1 ms backstop on the fontbake bake; W-d′ ships route (b) only and states the residual | `KE16-DESIGN-W.md` §3.5; KE17 row |
| **L10 / E10** cache-residency instrument | nothing in the tree can measure L1/L2 residency of a system's working set; `pin_workers` is a stub (`thread_pool.rs:551-557`); what it would take is in `KE16-DESIGN-APP.md` §4 | `KE16-DESIGN-APP.md` §4; `KE16-DESIGN-SPACE.md` §H.3 |
| L4/L13/W18 topology-aware victim/wake order | blocked on axis 30 (App-9 records it this pass; the design is a later pass) | `KE16-DESIGN-SPACE.md` E21 |
| G23 BWoS substrate; G10 heartbeat pool; G12/G18/G19/G21 `par_iter` reshapes | substrate or `par_iter` rework, not a fix of A/B; the KE15 chunk-runner rework is where G18/G21 notes go | `KE16-DESIGN-SPACE.md` E9, E26, G23 |
| E26 per-scope registered queue | theory-only cell; its price is a live-scope registry on every steal | `KE16-DESIGN-SPACE.md` E26 |
| A steal LIMIT under LIFO deques (`steal_batch_with_limit_and_pop(local, L)`, `L < 33`; rayon's `L = 1`) | not a lever for the per-element CAS cost: crossbeam's LIFO-source path pays one `SeqCst` CAS per stolen ELEMENT whatever `L` is (`deque.rs:1077-1142`); `L` only changes how many tasks a thief holds per probe and how many probes it makes — the `top_lane` receipt would show hoarding if that were the problem | `KE16-DESIGN-W.md` §0; `KE16-DESIGN-A.md` §1.4 |
| W19 WAITPKG idle primitive | not before the wake protocol is settled | `KE16-DESIGN-SPACE.md` E22 |
| `ThreadPool::spawn` per-call placement (axis 39, E29) | no production caller found (H.5); it rides A's discipline | `KE16-DESIGN-A.md` §7 |
| ~~`timeBeginPeriod(1)` at boot~~ | **MOVED INTO SCOPE by the owner ruling of 2026-09-02 (§6 ruling 4): "ну давай повысим тогда".** Ships as **App-12** in `crates/boyko_app` (host layer, not the pool: a library must not change process-wide state) — an RAII `winmm` guard raising 1 ms at boot and restoring on drop, measured both ways in `crates/boyko_app/tests/app12_timer_resolution.rs` | §6 ruling 4; `KE16-DESIGN-APP.md` §6 |
| A cheaper StoreLoad barrier than `mfence` for `publish_fence` | no portable Rust spelling; ~10–20 cycles per wake decision, below the 1 µs band; rayon-core 1.13.0 uses `fence(SeqCst)` for the same purpose | `KE16-DESIGN-W.md` §6 |
| ~~`spawn_batch` at the fontbake bake~~ | **MOVED INTO SCOPE by the owner ruling of 2026-09-02 (§6): "if it can be optimised somehow, optimise it".** If `ke16-c-batch` wins its step, the bake is converted in the same pass and re-measured on its own bench row | `KE16-DESIGN-W.md` §4.2; §6 ruling 2 |

## 6. Owner rulings — ALL SIX ANSWERED 2026-09-02 (the questions are kept below each ruling; the record of why outlives the call)

**Read this section before implementing the B axis or the C axis.** Two rulings change the design;
four confirm it. The owner's channel copy is `docs/OPEN-QUESTIONS.md` (§RESOLVED 2026-09-02) and its
Russian twin.

| # | ruling | effect on this design |
|---|---|---|
| 1 | **Commit the harness — yes** | unchanged; the documentation corpus is committed ahead of the implementation, the instruments follow with the code they measure |
| 2 | **The fontbake bake — OPTIMISE IT** ("если можно как-то оптимизировать — оптимизируй") | **CHANGES THE DESIGN, twice — see below** |
| 3 | **KE17 split — confirmed** ("надо будет исправить отдельно") | unchanged; the row is open in `docs/aether-v2/KERNEL-BACKLOG.md` since 2026-09-02 |
| 4 | **Timer resolution — RAISE IT** ("ну давай повысим тогда") | **CHANGES THE DESIGN:** §5's refusal is withdrawn; **App-12** ships the raise in the host layer, measured both ways. App-7 still measures the pool-side expiry and now has a second configuration to compare against |
| 5 | **Nested system execution — acceptable** | unchanged; App-8 ships |
| 6 | **O-series retake — annotate, do not rewrite** | unchanged; `KE16-RESULTS.md` carries the shipping-route numbers, the O6 tables get one dated pointer line |

### Ruling 2 in full, because it changes two things

Reading `generate_distance_field` (`crates/boyko_fontbake/src/msdf/distance.rs`) reframed the
question the design had asked. The bake calls `pool.install` from an application thread and spawns
`4 × workers` disjoint row bands — **64 tasks at W=16**. That is the external-joiner route, and it is
the **one route where defect B is live in production today**: the joining bake thread batch-steals up
to 33 of those bands into the private unregistered `scratch` deque and runs them one at a time. So
the bake is not choosing between "keep a helper" and "lose a lane"; it is paying the defect in full
today, and **both** B arms improve it.

1. **The bake gets its own bench row, and that row decides B1 vs B3.** `KE16-DESIGN-MEASUREMENT.md`
   step 2 must no longer settle the external arm on code size or on the dispatcher-route proxy cells
   alone: `crates/boyko_fontbake` gains a criterion row over a real glyph, run under both arms, and
   the faster arm wins the external joiner. The proxy cells stay as corroboration.
2. **`spawn_batch` at the bake moves from §5 into the pass.** If `ke16-c-batch` wins step 5, the bake
   is converted in the same pass and re-measured on the same row. (`KE16-DESIGN-W.md` §4.2's
   "deliberately per-task" note for fontbake is superseded by this ruling.)

### The questions as they were asked (kept for the record)


1. **Commit the harness.** The five untracked instrument files and the three `Cargo.toml` hunks are
   the only carriers of every number this campaign produces; this design assumes they are committed
   with the pass (the tournament cannot run from a clean clone otherwise). Confirm.
2. **The fontbake route.** `crates/boyko_fontbake/src/msdf/distance.rs:433` is a production
   non-worker `install` + spawn + join (the MSDF bake). The external-joiner policy (B1's steal-one
   helper vs B3's park) changes that bake's lane count by one. The design keeps the helper (B1) and
   measures B3; if the bake is tool-time only and its throughput is not a value, say so and B3 wins
   on code size if the dispatcher rows are within the band. (A consequence, decided in the design,
   not a question: B3 also changes which physics bench row is the W-lane reference —
   `bench_thread_install` instead of the W−1 row — `KE16-DESIGN-APP.md` §11.)
3. **S6/S10 as a separate ticket (KE17).** This pass fixes the pool; the scheduler-level lever
   (the apply-window barrier) is recorded as the next ticket with M1-a as its first rung. Confirm
   the split.
4. **Timer resolution — ASKED, THEN RULED AND SHIPPED.** The question was whether to leave
   `park_timeout(50 µs)` as the ≥1 ms wait Windows makes of it (axis 36). The design had decided
   against `timeBeginPeriod(1)` on the documented power cost, relying on W-d′ to make the backstop
   non-load-bearing on route (b). **The owner overrode that on 2026-09-02 and the measurement
   vindicates the override decisively.** App-12 measured the real expiry on this box over 200
   samples: a 50 µs park expires after a median of **15 296 µs** unguarded and **1 021 µs** with the
   guard held — a **15.0× reduction**, and the same figures for a 100 µs park. Dropping the guard
   restores 15 333 µs, so the release is genuine. At the default quantum a lost wakeup costs most of
   a 16 ms frame; under the guard it costs a millisecond. Shipped as
   `boyko_app::timer_resolution::TimerResolutionGuard`, held for the run in the host runner closure
   — the host layer, not the pool, because a library must not change process-wide state.
   ⚠ **One run in five granted the request and moved nothing** (15 371 → 15 368 µs). CPU saturation
   and Windows 11 EcoQoS were both tested and refuted as the cause, which is why App-12's gate
   asserts only "not worse", and why the honest reading is that the win is large but not guaranteed
   on every boot.
5. **Nested system execution.** A helping joiner can run a sibling conflict-free system inline
   inside another system's body (already reachable today via `injector_global`; more likely under
   B1). The design makes this legal (App-8 depth counter) and documents it. It changes nothing the
   conflict graph guarantees; it does change what a profiler sees on one lane (two overlapping
   `SystemSpan`s). Confirm this is acceptable behaviour rather than something to forbid.
6. **Scope of the O-series retake.** The pass re-takes the colored-solve numbers on the shipping
   route (`in_scheduled_system`) and records them beside the O6 numbers. Whether every O-series
   table is rewritten or only annotated is a scope call.

(The critic's proposed seventh question — whether `par_chunk` is in the batch-spawn scope — is a
design fork, not a VALUES call, and is decided here: it is in scope, `KE16-DESIGN-W.md` §4.2.)

## 7. Doc comments the pass corrects (in the same commit as the code they describe)

| Site | What is false | Becomes |
|---|---|---|
| `thread_pool.rs:123-124` | "workers drain it in stage 2 of `worker_main`" — the global injector is stage 3 | the shipped poll order |
| `thread_pool.rs:127-129` | "siblings still see them via the local-injector poll in stage 1.5" — no such stage exists | deleted with `injector_local` under A1; corrected under A2 |
| `worker.rs:355-357` | same "stage 1.5" claim | the shipped placement rule |
| `worker.rs:302-319` | `unpark_one_idle`'s algorithm comment: no barrier named; the rotor RMW precedes the load | the fenced prologue (`publish_fence`), the `exclude` mask, the rotor behind the load |
| `lib.rs:47-48` | "4-source poll loop (local injector → global injector → sibling steal → backoff/park)" omits the own deque | the shipped order (own deque → global injector → sibling steal → park under A1) |
| `scope.rs:17-21` | "we do NOT drain the calling worker's own Chase-Lev deque … not accessible here" | false under A1 (the TLS raw pointer); rewritten to describe B1 |
| `scope.rs:149-151` | "learning we are last would require reading `pending` after the sub — too late" | refuted (N66): `fetch_sub` returns the previous value; the constraint is the wake target's lifetime — rewritten under W-d′ |
| `scope.rs:426-431`, `:512`, `schedule.rs:665-671` (the comment above the park at `:683`) | "50 µs" / "100 µs" backstops described as waits | "a source constant; ≥1 ms on Windows (`dur2timeout` rounds up, `std/src/sys/pal/windows/mod.rs:240-254`; no `timeBeginPeriod`); defensive only on route (b) under W-d′" |
| `sync.rs:39` | "`fence` — no production code uses `core::sync::atomic::fence`" | false after W-a: `publish_fence` is production code; `fence` joins the shim (`loom::sync::atomic::fence` under loom) |
| `sync.rs:43-48` | "the two are separate fields and never assigned across" (WorkerHandle.thread vs the shimmed waker) | still true; gains the `WakeHandle` alias paragraph (`KE16-DESIGN-W.md` §3.2): under `cfg(not(loom))` the two ARE the same type and W-d′ points at the former |
| `solver/colored.rs:2628-2639`, `soft/colored.rs:991-995` | "the dispatcher lane … work-steals … so the lane pool is `num_threads + 1`" | "the lane pool is `num_threads()`: on the production route the joiner is one of the W workers; on the bench route the external joiner's share is measured, not counted" |
| `resources.rs:1470-1477`, `:1494-1496` | "`num_threads() + 1 == 1`, i.e. zero worker threads" — unreachable (`num_threads() ≥ 1`, `thread_pool.rs:586`) | the guard asks `num_threads() < 2` and means "one worker ⇒ serial" |
| `tests/loom_pool.rs:12-16`, `:103-114` | "loom (issue #246) does NOT persist an unpark issued before the matching `park`" — loom 0.7.2 (the pinned version) stores `Runnable { unparked: true }` in `set_unparked` and `rt::park` consumes it (`loom-0.7.2/src/rt/thread.rs:153-165`, `src/rt/mod.rs:87-107`) | the M1 yield re-poll is kept as the transport-agnostic shape; the sentence is rewritten to say what the pinned loom does, and M1c uses a real `park()` (`KE16-DESIGN-W.md` §3.7) |
| `tests/loom_pool.rs:143-166`, `:208-217` | "In PRODUCTION that fence is supplied by the crossbeam injector transport … the producer's publish is `Injector::push`" | the PRODUCER's fence is production code (`publish_fence` in `unpark_one_idle`, called through the exported shim); the CONSUMER's fence is the crossbeam STEAL path's (`Injector::steal_batch_and_pop`'s explicit fence; `epoch::pin` in `Stealer::steal_batch_and_pop`) — under A1 the push is a plain store and supplies nothing |
| `tests/loom_pool.rs:22-24` | "line-for-line copy of `worker.rs:239-257`" | the claim loop is `claim_one_idle` (moves again after W-a) |
| `tests/loom_pool.rs:140` | "post-`mark_idle` re-poll (`worker.rs:78`)" | `worker.rs:104` |
| `tests/loom_pool.rs:343` | "`worker.rs:86`'s `shutdown.load`" | `worker.rs:112` |
| `tests/miri_scope.rs:6-7` | "`scope.rs:256`" (the transmute) and "`scope.rs:140`" (`as_ref`) | `scope.rs:361-362` and `:223` |
| `tests/miri_scope.rs:56-59` | "~1/16 adversarial seeds hit a liveness timeout … Candidate U's lost-wakeup window" stated for the whole suite | stays TRUE for these tests (all join from the test thread = the external arm, unchanged by W-d′); the sentence gains "external-arm only; the worker-joiner arm is count-gated and gated at zero timeouts by `nested_scope_from_worker_is_stolen_by_sibling`" |
| `tests/shutdown.rs:23-30` | "drains the batch INLINE on the dispatcher … scratch deque" | rewritten for the shipped joiner (B1: one task at a time, no scratch; the liveness note about blocking tasks still applies to a helper) |
| `tls.rs:185-192` | "nested system runs are a contract violation under SCH7" | App-8: nesting through a helping joiner is legal; the counter bounds depth |
| `crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs:269-272` | "EVT1 — per-thread lane single-writer. Each worker is the sole writer of its lane" — true, and silent about nesting | gains one sentence: a sibling system run inline by a helping joiner (App-8) appends to the SAME lane sequentially; events of the two systems interleave within the lane; no reader relies on per-system contiguity (`KE16-DESIGN-B.md` §2.5) |
| `scope.rs:504-513` (the joiner's park comment) | "Wake one idle worker before parking" — says nothing about the joiner's OWN idle bit, which stays clear | under B1/B3: the joiner marks its bit and parks the way `worker_main` does (B1-P), so it is claimable by any wave; under B0 the comment gains "this joiner is not idle-marked and is invisible to other waves' wake decisions until its own last completer or the backstop" |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:72` | "nothing in the tree calls `timeBeginPeriod`, so the real expiry is ≥1 ms" — false since **App-12** | the host now holds `timeBeginPeriod(1)` for the run, so under the shipped host the expiry is ~1 ms (measured 1 021 µs); a process without the host still sees the quantum, bounded by whatever another process requested |
| `crates/boyko_threadpool/benches/ke16_nested_scope.rs:34` (header) and its `ke16_park_timeout` group description | same false claim, and it matters more here | same correction, **plus**: the `park_timeout_*` medians now depend on whether the host guard is held, so every published row must say which configuration it was taken in (App-12 measured 15 296 µs unguarded against 1 021 µs guarded on this box) |

## 8. Soundness obligations and the gates that discharge them

Every Miri command below is written with its FULL flag string: `$env:MIRIFLAGS` REPLACES the
`[env]` default of `.cargo/config.toml:14-15` (`-Zmiri-tree-borrows`), it does not merge with it.
A Miri red is classified by KIND before it is acted on (`KE16-DESIGN-MEASUREMENT.md` §5 item 14):
an `Undefined Behavior` report is an aliasing/data-race defect; a `spin_until` timeout is a
wake-protocol liveness defect; the two are never confused and only the former can ever send A1 to
its fallback.

| Candidate | Obligation | Gate |
|---|---|---|
| A1 / A1-fifo — aliasing (D5) | The TLS raw pointer (`&raw const deque`, no reference minted at the deposit) is dereferenced only on its owning thread, only into `&Worker` values consumed by ONE method call in their own statement (`let popped = lane.deque().pop();`, never an `if let` scrutinee — whose temporaries live through the THEN block in Rust 2024) or passed to a helper that runs no task body; the argument rests on what Tree Borrows checks: no protector spans a body (the only protectors are a `Worker` method's `&self` and the helpers' `local`, and no body runs inside either) and no `&Worker` is used after a body has run; no `&Worker` is a field of a by-value argument (Miri retags and protects those unconditionally — the installed README lists no `-Zmiri-retag-fields` knob); the only bytes of the `Worker` allocation written after construction are its `Cell<Buffer<T>>` (byte-precise interior mutability under Tree Borrows), written only by this thread; thieves touch only the heap `Inner`; the pointer is cleared before the deque drops (`KE16-DESIGN-A.md` §1.3) | two Miri shapes in `tests/miri_scope.rs`, both with `pool.spawn` + `AtomicBool` wait (NO external join): `nested_scope_from_worker_is_stolen_by_sibling` (H4 handshake; a sibling steals from the TLS-reached deque) and `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` (each of two bodies opens a nested scope and pushes ≥129 tasks — forcing a `resize`, the one write to the `Worker` bytes — while the outer join is live; RECEIPT: one such body ran on the outer worker's id — deterministic under B1 + LIFO, otherwise retried up to 4× per seed and a miss is the THIRD red kind, "receipt not observed": not a defect, re-run at 64 seeds, gate = zero UB on every seed AND the receipt on ≥1 seed), under `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..16"`, with B0 and with B1; the existing suite under the same flags; a unit test that the deposit is `(null, null)` after `worker_main` returns |
| A1 identity | The deque arm, the B1 joiner arm and the W-d′ target are taken only when `worker_lane_for(inner)` is `Some`: the TLS deque's pool is `inner` AND `current_worker_id() < worker_count`. An `install` of pool B on a pool-A worker (ACTIVE_POOL swapped, `thread_pool.rs:196`) and an `install` of pool A on a pool-A worker (`CURRENT_WORKER_ID = WORKER_ID_DISPATCHER`, `:205`) are both external | `tests/cross_pool_routing.rs`: `install_on_foreign_worker_routes_to_global` (receipt = `(pool address, worker id)` per body: `(B, wid < W_B)` or `(B, DISPATCHER)`, never `(A, _)`) and `install_on_same_pool_worker_is_external` (a worker task calls `pool.install`, spawns N, joins; asserts completion within a bound; under `ke16-w-count` additionally that the process did not abort); `tls.rs` unit test: a deposited deque + `CURRENT_WORKER_ID = WORKER_ID_DISPATCHER` → `worker_lane_for` is `None` |
| W-a — the producer-side barrier (base) | Every wake decision is `publish_fence()` (`fence(SeqCst)`) → `idle.load(Acquire)` → claim, so the fenced SB litmus holds against a parking worker's `fetch_or` → steal-path fence → re-poll, for plain-store transports (A1's `Worker::push`, the thief's residue store) as for the injector; the consumer's fence is the crossbeam STEAL path's — `Injector::steal_batch_and_pop`'s `fence(SeqCst)` at `deque.rs:1821`, inside the `new_head & HAS_NEXT == 0` empty-check path (the one the litmus needs), and `epoch::pin` at `:1006` in `Stealer::steal_batch_and_pop` | loom M2 (`tests/loom_pool.rs:200-265`) with the producer's fence being the exported production `publish_fence()`; the calibration copy `loom_m2_calibration_no_producer_fence_is_lost` (fence deleted), annotated `#[should_panic(expected = "M2: lost wake")]` — its own oracle's message, never a bare `#[should_panic]` — MUST go red for that reason, recorded at Step 0 |
| A5 | A claimed idle bit is ALWAYS followed by an unpark of that worker (a worker parks with untimed `park()` at `worker.rs:117`); the foreign push happens at most once per spawn; the placement never targets a queue outside the scan set | loom M2c: the claim+placement+unpark sequence over the transcribed claim loop with one parked worker running the real `mark_idle`/`unmark_idle` (asserts the worker's `park()` returns in every interleaving where its bit was claimed); stress test with W−1 workers parked: `claimed == unparked` counters |
| B1 | No deadlock: a joiner that helps keeps executing ready tasks or parks with a guaranteed wake (W-d′ on route (b); the backstop otherwise; and, under B1-P, any wave's claim — which only wakes it earlier); a task never blocks except in its own nested join, which helps; no `&Worker` spans a task body (D5) | existing `nested_scope_does_not_deadlock` (`scope.rs:605-628`), `tests/stress.rs`, both Miri shapes above (the inline-nested-spawn one re-run under `ke16-b1`); `tests/shutdown.rs` liveness note re-verified |
| B1-P (the idle-marked joiner park, J15) | The parked joiner's `mark_idle` → post-mark re-poll → `park_timeout` → `unmark_idle` is `worker_main`'s own sequence (`worker.rs:98-121`), so the worker-side W-b-1 argument applies verbatim; a joiner never parks with a non-empty own deque (only its owner pushes there, and the owner is the joiner, which popped first); a claimed bit is always unparked (A5-1: the claimer unparks `inner.workers[wid].thread`, this thread); W-d′'s unpark and a claim land on one parker — a spurious return re-checks and re-scans; the joiner does not check `shutdown` (a thread inside a task body cannot exit; today's behaviour for a scope open at drop) | loom M2's parked-thread role IS this sequence — no new model; native receipt `parked_joiner_is_claimed_by_a_foreign_wave` (`KE16-DESIGN-B.md` §2.7: the parked joiner's id is the foreign task's receipt; compiled only under `ke16-b1`/`ke16-b3`); `ThreadPool::parked_mask()` is the instrument |
| B1 + App-8 | Running a sibling system inline inside a system body: conflict-free by co-dispatch (both are in `running`), completion accounting unchanged (`pending == running.count_ones()` still gates the drain, `schedule.rs:600-602`), `InSystemRunGuard` nesting legal | new ECS test `boyko_ecs/tests/ke16_nested_system_inline.rs`: two conflict-free systems, one with a ≥1024-row `par_iter` and a spin body, W=2; asserts both complete, no debug panic, and (receipt) that the nested run happened at least once across repeats; `tests/miri_phase9.rs` guard tests extended to depth 2 |
| W-b | The WEAK invariant W-b-1: no task remains in the scan set while every worker of the pool is parked and no wake is pending; the strong form is not claimed (`KE16-DESIGN-W.md` §2.3); the residue cascade never claims the cascading thief's own bit | loom M4 (new): one owner doing 3 gated pushes into a toy queue whose length is read as a SEPARATE snapshot before the push (so the check-then-push race is expressible), two thieves running the real `mark_idle`/`unmark_idle` + real loom `park`/`unpark` + the transcribed one-attempt `claim_one_idle` + the residue rule with self-exclusion, cascading from inside the post-`mark_idle` re-poll with their own bit set; oracles: a thief woken only by the model's shutdown unpark (its idle bit still set on wake) that finds work remaining = lost wake = `panic!("M4: lost wake …")`; a cascade that claims its own bit = `panic!("cascade claimed self …")`; loom's deadlock detection covers the rest; `LOOM_MAX_PREEMPTIONS=3`. TWO calibration copies must go RED FOR THEIR OWN REASON: the pure-empty gate (the critic's interleaving), `#[should_panic(expected = "M4: lost wake")]`, and the cascade without self-exclusion, `#[should_panic(expected = "cascade claimed self")]` |
| W-d′ | (i) the last completer's post-decrement access touches only `PoolInner`-owned memory that is alive: `inner.workers[wid].thread` is kept alive by the completer's own `Arc<PoolInner>` (a worker of the pool) or by the install/scope frame of a joiner running the task inline; (ii) `joiner_wake` is non-null only when `worker_lane_for(inner)` was `Some` at scope creation; (iii) `*self` (`ScopeShared`) is not touched after the `fetch_sub` | loom M1c (new): the REAL gated `complete_task` via `LoomScopeShared::new_worker_joined(waker, target)` with a model-owned `Box<WakeHandle>` (the loom counting newtype) as target; the joiner uses a real loom `park()`; asserts termination, `unparks() == 1`, `completed == N`; if the joiner must fall back to the yield re-poll the row is recorded as "M1c-count: green (count only)" and the Miri gate carries liveness alone. Miri: the ROUTE-(b) test `nested_scope_from_worker_is_stolen_by_sibling` under `-Zmiri-tree-borrows … -Zmiri-many-seeds=0..32` with `ke16-w-count` must show ZERO liveness timeouts (its only join is the worker's, count-gated); the existing `miri_scope.rs` tests keep their documented external-arm ~1/16 and are NOT the gate. A native test where a pool-A worker joins a pool-B scope and is B's last completer inline (exercises the external arm on a cross-pool joiner) |
| App-4 | `pending` accounting: `spawn_batch(n, it)` with `it` yielding k < n corrects by `fetch_sub(n − k)` before returning; k > n is a debug panic; the scope never returns with `pending ≠ 0`; the one wake decision happens after the FIRST push | unit tests for k < n, k == n, k > n (debug); `scope_multi_drain_frees_once` (`scope.rs:652`) extended with a batch wave; the physics `{1, N}` oracles green; `cargo test -p boyko-ecs --lib` for the `par_chunk` driver's tests |
| App-6 | A cross-pool joiner never touches the target pool's per-worker structures with its own id | `tests/cross_pool_routing.rs::cross_pool_join_does_not_drain_foreign_local_slot` (pre-fix: a pool-A worker joining a B scope drains B's `injector_local[wid_A]`; post-fix under A2/A3 it cannot; under A1 the slot no longer exists — the test asserts the routing receipt) |
| Feature scheme | illegal pairs fail to build; the witness matches the build | `cargo check -p boyko-threadpool --features ke16-a1,ke16-a2` (and `ke16-b1,ke16-b3`; `ke16-a3,ke16-b1`) must FAIL with the `compile_error!` message; a success is a red result; never piped. `KE16_EXPECT` set on every measured run |
| Whole pass | every `unsafe` block carries a `// SAFETY:` naming the invariant; no `Mutex`/`RwLock`/`HashMap`/`Vec::new()` on the spawn, steal, complete or idle paths; no cfg residue; no `#[allow(dead_code)]` from losing branches | `cargo clippy --workspace --all-targets -- -D warnings` (touch the edited sources first — the stale-fingerprint false-fresh hazard), `code-reviewer` pass over every switch point and over D5 at every `WorkerLane::deque()` call site; `grep -rn 'feature = "ke16' crates` empty at Step App |

## 9. Revision record

### 9.1 Round 4 — the critic's third REVISE (this revision)

| # | Blocking item | Decision | Where |
|---|---|---|---|
| 1 | The RMW table priced STEAL as "one CAS per ≤32 tasks" for every A, but under `Worker::new_lifo()` crossbeam's batch steal takes ONE `front.compare_exchange(SeqCst)` PLUS one `fence(SeqCst)` PER STOLEN ELEMENT (`deque.rs:1077-1142`; a reversal loop into a FIFO destination at `:1145-1156`), against one CAS per batch from a FIFO source (`:1034-1071`); §1.4 priced the LIFO choice on the owner's side only; Step-A rule 4 broke an `a1`/`a1f` physics tie toward `a1` "by the cheaper path" with that count, and rule 3's veto was one-directional | Verified at the cited lines and adopted in full. The STEAL row is split by SOURCE FLAVOUR, with the injector's one-CAS-per-batch and `Stealer::steal()` rows beside it (`KE16-DESIGN-W.md` §0); the per-task accounting is written out both ways — LIFO ≈ `s`, FIFO ≈ `(1 − s) + s/33` shared RMWs per task, `s ≈ 0.94` on the one-spawner consumer shape — so BY COUNT A1-fifo has the lighter steal path by an order of magnitude, which reverses revision 3's prediction "A1 best of the five at 1 µs × 64W" (retracted); what the count cannot price (FIFO's same-end contention with no-backoff retries; LIFO's own-wave-first joiner order and locality) is named as the reason A1 is still built; §1.4 is restated as "owner-side local fence bought with thief-side per-element CASes"; a steal LIMIT under LIFO is argued away (the CAS count is per element regardless of `L`); Step A is rewritten as "physics ranks, the grid vetoes SYMMETRICALLY within the physics band, pairwise in both directions, the 64W column breaking a split, a residual tie → smaller diff (`a1f`)" — never an RMW count; the same symmetric rule applies to every pair (non-blocking item 10); the B0-under-A1 rows in `KE16-DESIGN-A.md` §6 and `KE16-DESIGN-B.md` §1 carry the per-element cost and the reversal | `KE16-DESIGN-W.md` §0; `KE16-DESIGN-A.md` §0, §1.2, §1.4, §6; `KE16-DESIGN-B.md` §1; `KE16-DESIGN-MEASUREMENT.md` §7 Step A rules 2–4; §2 (A1, A1-fifo, A3 rows) and §3 above |
| 2 | Under B3 the external joiner parks, so `bench_thread_install_Wminus1` becomes W−1 lanes and `bench_thread_install` exactly W; the two do not "coincide" and the PRIMARY acceptance line would pass with the scheduled route up to 1/(W−1) ≈ 6.7 % slower than a true W-lane reference | The primary reference `REF` is selected by LANE COUNT, not by row name: `bench_thread_install_Wminus1` when the shipped B keeps the external helper (B0/B1), `bench_thread_install` when it parks (B3); a lane-count table is in `KE16-DESIGN-APP.md` §11; Step B rule 4 fixes `REF` at the Step-B verdict and requires the B3 receipt `Wminus1 / bench_thread_install ≈ W/(W−1)`; every "coincide" sentence is deleted (App-10 row, B3 row, APP §11, MEASUREMENT §7, §9) | `KE16-DESIGN-APP.md` §11; `KE16-DESIGN-B.md` §3; `KE16-DESIGN-MEASUREMENT.md` §3, §7, §9; §2 (B3, App-10 rows), §3 and §6 Q2 above |

Non-blocking items, all adopted: (1) every `&Worker` from the lane is consumed in its own
statement — `let popped = lane.deque().pop();` — because in Rust 2024 an `if let` scrutinee's
temporaries live through the THEN block; D5 and the SAFETY text now argue from "no protector spans
a body; no use after the call", which is what Tree Borrows checks (`KE16-DESIGN-A.md` §1.3, §1.7;
`KE16-DESIGN-B.md` §2.2); (2) the second Miri shape's receipt is deterministic only under
B1 + LIFO (the joiner pops the BACK; a sibling's batch from two takes the FRONT); elsewhere a miss
is the THIRD red kind, "receipt not observed" — retried 4× per seed, then 64 seeds, gate = zero UB
everywhere AND the receipt on ≥1 seed (`KE16-DESIGN-A.md` §1.3; `KE16-DESIGN-MEASUREMENT.md` §5
item 14(c), §8); (3) ONE signature, `claim_one_idle(inner, mask, start) -> Option<u32>`, one CAS
attempt, defined in `KE16-DESIGN-W.md` §1.1 and used by W-a, A5, W-f and the loom transcriptions
(`KE16-DESIGN-A.md` §4.1); (4) the consumer-side fence is cited at `deque.rs:1821`, inside
`if new_head & HAS_NEXT == 0` — the path that can conclude "empty", which is the one the litmus
needs (`KE16-DESIGN-A.md` §1.8; `KE16-DESIGN-W.md` §1.3; §8 above); (5) every calibration copy is
`#[should_panic(expected = "<its oracle's message prefix>")]` — `"M2: lost wake"`, `"M4: lost
wake"`, `"cascade claimed self"` — and a bare `#[should_panic]` is itself a refused shape
(`KE16-DESIGN-W.md` §1.3, §2.5; `KE16-DESIGN-MEASUREMENT.md` §3, §5 item 15); (6) the SPAWN (A1)
row carries the `resize` note — `deque.rs:405-411`, `MIN_CAP = 64`, the two 96-chunk physics
waves grow once per wave, the LIFO `pop` shrinks at `:530-534`, the `Injector` allocates a `Block`
per `BLOCK_CAP = 63` pushes (`:1201-1203`; the critic's "31" is 0.8.7's `LAP − 1` with `LAP = 64`)
(`KE16-DESIGN-W.md` §0); (7) the parked joiner's invisibility to foreign waves is stated as a
residual under B0 AND CLOSED under B1/B3 by rule B1-P (J15, new): the worker joiner parks the way
`worker_main` parks — `mark_idle` → post-mark re-poll → `park_timeout` → `unmark_idle` — so it is a
claimable lane; one `fetch_or` + one `fetch_and` per park; loom M2's parked-thread role already
models the sequence; receipt test `parked_joiner_is_claimed_by_a_foreign_wave` with the new
`ThreadPool::parked_mask()` accessor (App-11) (`KE16-DESIGN-B.md` §1, §2.2, §2.6, §2.7;
`KE16-DESIGN-W.md` §3.5; §2, §7, §8 above); (8) EVT1's per-THREAD single writer means an inline
sibling system appends to the same lane sequentially and its events interleave within it — stated
beside the `SystemSpan` caveat and added to the `event_dispatcher.rs:269-272` doc comment
(`KE16-DESIGN-B.md` §2.5; `KE16-DESIGN-APP.md` §7; §7 above); (9) `worker_wake_handle` returns
`&self.workers[wid].thread as *const WakeHandle` (`KE16-DESIGN-W.md` §3.2); (10) the symmetric
grid rule is stated for every pair, and "physics ranks, the grid vetoes" is written in those words
(`KE16-DESIGN-MEASUREMENT.md` §7 Step A rules 2–3).

### 9.2 Round 3 — the critic's second REVISE (revision 3, kept for the record)

| # | Blocking item | Decision | Where |
|---|---|---|---|
| 1 | Under A1 + W-a the producer side of the Race-C protocol loses its StoreLoad barrier (today it is supplied by accident by the `Injector::push` `lock`-prefixed ops and the rotor `lock xadd`; `Worker::push` is a plain store; W-a moves the rotor behind the load); M2/M4 stay green only because the model inserts the fence "for the transport"; a Miri liveness red would be misread as a Tree-Borrows failure; the RMW table under-counts A1 | `publish_fence()` (`fence(SeqCst)`, through the `sync` shim) is the first statement of `unpark_one_idle_excluding`, which every wake decision on every arm calls — the push arms via `wake_after_push`, the residue cascade, the joiner's pre-park wake, A5's fallback, W-f; a gated-out push (`pre_len ≥ 2`) skips both the fence and the load, which the weak invariant permits; the fence is charged to every arm in the §0 table (one local `mfence`, no line transfer) so the A1-vs-A3 delta is pure transport; W-a is retracted as "zero behavioural change" and restated as "one contended-line RMW replaced by one local full barrier"; M2's producer fence is re-attributed to the exported production `publish_fence()` and a calibration copy without it must go red at Step 0; the consumer's fence stays with the crossbeam STEAL path, which still has it under A1; rayon-core 1.13.0 read at this checkout: `new_injected_jobs` fences (`sleep/mod.rs:214-220`), `new_internal_jobs` does not and documents the owner-pops fallback (`:225-237`) — the un-fenced case is exactly KE16's defect, so both are fenced here; a Miri red is classified by kind (UB report vs timeout) and a timeout never sends A1 to A2 | `KE16-DESIGN-A.md` §0, §1.2, §1.3, §1.8; `KE16-DESIGN-W.md` §0, §1, §2.3; `KE16-DESIGN-MEASUREMENT.md` §5 items 14–15, §7, §8; §7 and §8 above |
| 2 | The A1 SAFETY invariant "no protector spans the pointee" is false under B1: Miri retags and protects the `&Worker` FIELD of the by-value `WorkerLane` argument for the whole join, and a body run inline by the joiner that spawns through the TLS deque is a foreign write under that protector; the Miri gate never exercises this shape | `WorkerLane` is `Copy` and holds a RAW pointer (the deposit itself is `&raw const deque` — no reference minted); `WorkerLane::deque()` mints a `&Worker` bounded to one expression or to a helper that runs no task body; discipline D5 states that no `&Worker` is a by-value-argument field, a parameter spanning a body, or a local across `run_task`; the SAFETY text is rewritten around D5 and the byte-precise interior mutability of the one written field (`Cell<Buffer<T>>`), with the installed Miri README cited for both facts (no `-Zmiri-retag-fields` knob; `-Zmiri-tree-borrows-no-precise-interior-mut` exists to turn byte-level tracking OFF); a second Miri shape `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` (two nested-spawning bodies of ≥129 tasks each — a forced `resize` — with the inline receipt) is added and run under B0 and B1 | `KE16-DESIGN-A.md` §1.1, §1.3, §1.7 (D5); `KE16-DESIGN-B.md` §2.2, §2.7; `KE16-DESIGN-MEASUREMENT.md` §6, §8; §8 above |

Non-blocking items, all adopted: (1) B0 under A1 is a REAL batch into `scratch` (the degradation
at `deque.rs:987-991` needs `dest` to be the source; `scratch` is not) — the "one task per probe"
sentence is retracted everywhere and the `top_lane ≈ 33` prediction is now consistent with its
mechanism (`KE16-DESIGN-A.md` §0, §6; `KE16-DESIGN-B.md` §1; §1–§2 above); (2) the thief-residue
cascade excludes the caller's own idle bit (`unpark_one_idle_excluding(inner, 1 << wid)`;
`pop_global_injector` gains `wid`; M4 gains the post-mark re-poll cascade case and a
no-self-exclusion calibration copy) (`KE16-DESIGN-W.md` §2.1, §2.2, §2.5); (3) `par_chunk.rs:139`
is a `spawn_batch` caller, fontbake deliberately stays per-task (`KE16-DESIGN-W.md` §4.2); (4) an
M1c that falls back to the yield re-poll is recorded as "M1c-count: green (count only)" and the
route-(b) many-seeds run is then the sole liveness gate (`KE16-DESIGN-W.md` §3.7;
`KE16-DESIGN-MEASUREMENT.md` §5 item 16, §7); (5) `bench_thread_install_Wminus1` is mandatory and
is the acceptance line's PRIMARY reference, the (W+1)/W-route ratio reported raw beside it
(`KE16-DESIGN-APP.md` §11; `KE16-DESIGN-MEASUREMENT.md` §3, §6, §7); (6) the LIFO pop is priced as
one local `mfence` per executed own task, so A1 vs A1-fifo reads "local barrier vs shared-line
RMW" (`KE16-DESIGN-W.md` §0; `KE16-DESIGN-A.md` §1.4); (7) the Step-A dispatcher-route wording
says what differs under a1/a1f/a3 — only the removed stage-1 empty probe on the idle path, an
improvement shared by all three, never a veto (`KE16-DESIGN-MEASUREMENT.md` §7); (8)
`install_on_foreign_worker_routes_to_global` asserts on the `(pool address, worker id)` pair
(`KE16-DESIGN-APP.md` §5).

### 9.3 Round 2 — the critic's first REVISE (revision 2, kept for the record)

| # | Blocking item | Decision | Where |
|---|---|---|---|
| 1 | W-b's push gate is check-then-push; the strong invariant is false; M4 reds or passes vacuously | Gate rule changed to the JDK/FJP shape: wake iff the pre-push length was ≤ 1; invariant restated as the weak W-b-1; the one-body stall recorded as W-b's measured cost; M4 rebuilt with a snapshot-then-push toy transport, real loom park/unpark and a shutdown-wake oracle, and calibrated to go RED on the empty gate | `KE16-DESIGN-W.md` §2.1, §2.3, §2.5 |
| 2 | Under A1, `install` on a same-pool worker yields `joiner = Some(WORKER_ID_DISPATCHER)` → OOB unpark under W-d′; three inconsistent identity checks | ONE predicate `tls::worker_lane_for(inner)` (pool-tagged deque AND `wid < worker_count`) used by the push arm, the joiner dispatch and the W-d′ target; an `install` frame on a worker is external on every arm; test row `install_on_same_pool_worker_is_external` | `KE16-DESIGN-A.md` §1.1, §1.7; `KE16-DESIGN-B.md` §2.1; `KE16-DESIGN-W.md` §3.2; `KE16-DESIGN-APP.md` §5 |
| 3 | M1c cannot drive the real count-gated `complete_task` (std `Thread` inside a loom-unbuildable `PoolInner`) | Wake target typed `*const crate::sync::WakeHandle` copied out of `ScopeShared` before the decrement; `WakeHandle` = `std::thread::Thread` under `cfg(not(loom))` and a counting newtype under `cfg(loom)`; one `cfg(loom)` pair in `PoolInner::worker_wake_handle`; `LoomScopeShared::new_worker_joined(waker, target)` | `KE16-DESIGN-W.md` §3.2, §3.3, §3.7 |
| 4 | The many-seeds zero-timeout gate was aimed at `miri_scope.rs`, whose joiners are all external | Gate re-aimed at the route-(b) test whose ONLY join is a worker's; the external-arm window stated as persisting, its fix recorded for KE17; W17 removed as a fallback | `KE16-DESIGN-W.md` §3.4–3.5, §3.7; §5 above; `KE16-DESIGN-MEASUREMENT.md` §8 |
| 5 | `$env:MIRIFLAGS = "-Zmiri-many-seeds=…"` REPLACES the `[env]` `-Zmiri-tree-borrows` | Every Miri command carries the full string; the tester echoes `$env:MIRIFLAGS` before each run; refused shape 12 | `KE16-DESIGN-MEASUREMENT.md` §5 item 12, §8 |
