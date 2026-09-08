# KE16 design — the tournament: order, commands, metrics, decision rules, noise, receipts

Part of the KE16 design; index in `KE16-DESIGN.md`. The tester follows this file literally; the
developer builds what `KE16-DESIGN-A/B/W/APP.md` specify before Step 0 starts. Results go to
`docs/threadpool/KE16-RESULTS.md` (new; layout in §9). Revision 4: Step A's grid rule is
SYMMETRIC — physics ranks, the grid vetoes, and the `a1`/`a1f` pair is decided on the 1–10 µs ×
64W worker cells in both directions, never by an RMW count (§7; the critic's round-3 blocking item
1); the physics acceptance line's reference row is chosen by LANE COUNT, `bench_thread_install`
under B3 (§3, §7, §9; blocking item 2); the loom calibration copies are `#[should_panic(expected =
…)]` (§3, §5 item 15, §8; non-blocking item 5); the second A1 Miri shape has a THIRD red kind —
"receipt not observed" (§5 item 14, §8; item 2); the B1-P receipt test is a gate row (§8).
Revision 3: the two loom calibration copies are Step-0 receipts (§8); the A1 Miri gate is two
shapes, and a red on either is CLASSIFIED before any fallback (§5 item 14, §8); the physics
`bench_thread_install_Wminus1` row is mandatory (§3, §6, §7); the Step-A dispatcher-route veto
wording says what actually differs (§7).

## 0. Preconditions for every number

1. **Toolchain path.** rustup's cargo first: PowerShell `$env:PATH = "$HOME\.cargo\bin;$env:PATH"`
   (bash: `export PATH="$HOME/.cargo/bin:$PATH"`). A chocolatey `rustc` shadows rustup and
   `RUSTUP_TOOLCHAIN` alone does not fix it. Record `cargo --version` and `rustc -vV` in
   `KE16-RESULTS.md` §0.
2. **Release profile, one box, nothing else building.** criterion runs in release by default; the
   ECS/physics protocol passes print `release=true` — a `release=false` line voids the run. No
   concurrent `cargo` on the machine (the memory note: a shared build skews every cell).
3. **W recorded.** Every bench prints `available_parallelism`; a baseline taken at a different W is
   not comparable. The grid's W is `available_parallelism` (16 on the bench box).
4. **The witness.** Every run sets `KE16_EXPECT` to the exact variant string
   (`"{A}+{B}+{W}+{C}"`, e.g. `a1+b1+wg+c0`); the bench panics on mismatch. An unset `KE16_EXPECT`
   is allowed only for a developer's smoke run and its numbers are not recorded.
5. **Two runs per configuration** (`-r1`, `-r2`), back to back, same session. The band (§4) comes
   from them.
6. **No `| tail`, no `| grep`, no `| Select-Object -Last`** on any cargo command: the exit code is
   the datum for the gates (§8), and a pipe on Windows PowerShell swallows it.
7. **Fingerprints.** After editing sources, `cargo clippy` may report "Finished" in 0.1 s with lints
   not re-run; touch the edited files (or `cargo clean -p boyko-threadpool`) before the lint gate.
8. **`MIRIFLAGS` is replaced, not merged.** `.cargo/config.toml:14-15` sets `MIRIFLAGS =
   "-Zmiri-tree-borrows"` in its `[env]` table; a `$env:MIRIFLAGS` set in the shell REPLACES it.
   Every Miri command in §8 therefore spells the whole string, and the tester prints
   `$env:MIRIFLAGS` immediately before each Miri run and pastes the line into the results file.
9. **Miri version recorded.** `cargo +nightly miri --version` and the toolchain's `rustc -vV` go
   into §0 of the results file: the Tree-Borrows and weak-memory behaviour the A1 gates rely on is
   the installed Miri's (`nightly-2026-08-20` at this checkout).

## 1. Steps and configurations

Notation: `A ∈ {a0, a1, a1f, a2, a3, a5}`, `B ∈ {b0, b1, b3}`, `W ∈ {w0, wg, wc, wgc}`,
`C ∈ {c0, c1, c1f}`. `a0+b0+w0+c0` is today's code plus the base items (W-a with `publish_fence`,
App-1/6/7/8/9, receipts).

| Step | Configurations (each on all three harnesses, twice) | Holds fixed |
|---|---|---|
| 0 | `a0+b0+w0+c0` | — (reference: today's dispatcher route is the ceiling, today's worker route the serial floor; the consumers' reference numbers; the `park_timeout` rows; the topology; the two loom calibration readings) |
| A | `a1+b0+w0+c0`, `a1f+b0+w0+c0`, `a2+b0+w0+c0`, `a3+b0+w0+c0`, `a5+b0+w0+c0` | B0, W0, C0 |
| B | `A*+b0`, `A*+b1`, `A*+b3` (A* = Step A's winner; `b1`/`b3` exist only for `a1`/`a1f`) | A*, W0, C0 |
| W1 | `A*+B*+wg+c0` | A*, B* |
| W2 | `A*+B*+wc+c0`, and if W1 kept: `A*+B*+wgc+c0` | A*, B*, the W1 verdict |
| C | `A*+B*+W*+c1` | A*, B*, W* |
| F | `A*+B*+W*+c1f` (only if C kept c1) | A*, B*, W*, c1 |
| App | the final configuration with every feature removed (unconditional code): the full grid + consumers ONCE MORE — the number that goes on record must come from the code that ships, not from a feature build | — |

## 2. Commands

PowerShell, from `D:\wt\threadpool`. `<F>` is the comma-separated feature list for the
configuration (`a1+b1+wg+c0` → `ke16-a1,ke16-b1,ke16-w-gate`); for the consumer crates each
feature is prefixed `boyko-threadpool/`. `<V>` is the variant string; `<N>` the run number.

```powershell
# Pool grid (24 cells x 2 routes, ~2 min) + the park_timeout rows
$env:KE16_EXPECT = "<V>"
cargo bench -p boyko-threadpool --bench ke16_nested_scope --features <F> -- --save-baseline <V>-r<N> --noplot

# ECS harness (protocol pass printout + criterion rows, ~3 min)
cargo bench -p boyko-ecs --bench ke16_par_iter_in_system --features <F-prefixed> -- --save-baseline <V>-r<N> --noplot

# Physics consumer (~2.5 min with the Wminus1 row)
cargo bench -p boyko-physics --bench ke16_solve_in_system --features <F-prefixed> -- --save-baseline <V>-r<N> --noplot
```

For `a0+b0+w0+c0` omit `--features`. The protocol-pass printouts (ECS: the `-- N=... rows` block;
physics: the `=== KE16 physics consumer` header) are copied verbatim into the results file.

Gates per configuration (§8) run BEFORE its benches; a configuration whose gates are red is not
measured.

## 3. Metrics, and where each comes from

| Metric | Source | Used for |
|---|---|---|
| grid cell median, MAD | `target/criterion/ke16_nested_scope/{dispatcher,worker}/body_<b>_tasks_<t>/<V>-r<N>/estimates.json` → `median.point_estimate`, `median_abs_dev.point_estimate` (ns) | the A/B/W/C decision rules |
| grid cell per-sample times | `.../<V>-r<N>/sample.json` → `times[i] / iters[i]` | bimodality (§4) |
| `park_timeout_50us` (and 1 ms, 2 ms) median | same layout, group `ke16_park_timeout` | App-7; the backstop truth for every latency statement |
| ECS wall + occupancy | the protocol pass lines `seq (baseline)`, `par_from_dispatcher`, `par_in_system` with `max_in_flight` and `speedup`, at N ∈ {4096, 65536}; plus criterion rows `ke16_par_iter_in_system/{seq,par_from_dispatcher,par_in_system}/<N>` | the ECS ratio receipt |
| ECS ratio | `par_in_system / par_from_dispatcher` wall (protocol pass, best-of-3) | anti-vacuity + secondary ranking |
| physics medians | `ke16_solve_route/{bench_thread_install,bench_thread_install_Wminus1,in_scheduled_system,empty_schedule_control,single_threaded_O5}/29751` | PRIMARY ranking; the acceptance line's reference `REF` is chosen by LANE COUNT — `bench_thread_install_Wminus1` when the shipped B keeps the external helper (B0/B1), `bench_thread_install` when it parks (B3) (§7; `KE16-DESIGN-APP.md` §11) |
| occupancy receipts | `tests/ke16_nested_scope_occupancy.rs` printout: `max_in_flight`, `lanes_used`, `top_lane`, `off_pool`, `outer_worker_id`, `outer_same_pool` per (route, W, rep) | the diagnostic behind a wall-clock change (B's signature is `top_lane`, never `max_in_flight`) |
| route receipts | bench printouts: `outer_worker_id` (pool bench), system-closure worker id (ECS, physics), `KE16 variant=<V>` | vacuous-pass refusal (§5) |
| Miri flag receipt | the echoed `$env:MIRIFLAGS` line before each Miri run | §5 item 12 |
| loom calibration receipts | the three calibration tests (§8), each `#[should_panic(expected = "<its oracle's literal message prefix>")]` — `loom_m2_calibration_no_producer_fence_is_lost` (`"M2: lost wake"`), `loom_m4_calibration_empty_gate_is_lost` (`"M4: lost wake"`), `loom_m4_calibration_no_self_exclusion_claims_self` (`"cascade claimed self"`); never a bare `#[should_panic]`, which passes on a loom deadlock report or any unrelated assertion | a model that cannot fail is not a gate; a copy that fails for the wrong reason is not calibration |

## 4. Noise, bimodality, and the band

- **Band per cell** `band(c) = max(0.04, |med_r1(c) − med_r2(c)| / min(med_r1, med_r2))`, computed
  for the reference configuration of the step AND for the candidate; the band applied is the larger
  of the two. A 4 % floor because criterion's 10 samples over a spinning body have that much
  jitter on this box even at 1 ms.
- **Regression / improvement** of candidate V vs reference R on cell c: V regresses if
  `med_V(c) > med_R(c) × (1 + 2·band(c))`; V improves if `med_V(c) < med_R(c) × (1 − 2·band(c))`;
  otherwise the cell is a tie. The factor 2 is the margin that keeps a single noisy run from
  eliminating a variant.
- **Bimodality flag**: `MAD / median > 0.25` on any run of a cell. The dispatcher route is known to
  be bimodal (the joiner's inline share: 33 / 21 / 14 / 8 of 64 — the ≤33 batch into `scratch`,
  `KE16-DESIGN-B.md` §1). For a flagged cell the tester reports `p10 / p50 / p90` from
  `sample.json` and the decision uses p50; a candidate that removes the bimodality (MAD/median <
  0.1 where the reference was > 0.25) is recorded as such — that is B1's expected signature on the
  dispatcher route.
- **The consumers' medians** are compared with the same rule; the physics rows have 20 samples and
  their own `estimates.json`. A physics `in_scheduled_system` run with MAD/median > 0.25 is flagged
  and its p90 reported: a ≥1 ms outlier class there is the external-arm backstop firing on the
  frame path (`KE16-DESIGN-B.md` §4), which no candidate is expected to produce.
- **Run-to-run receipts**: the occupancy test is run once per configuration with
  `--test-threads=1 --nocapture` and its three repeats per (route, W) pasted; `outer_worker_id`
  must be a worker id on every worker-route line.

## 5. Vacuous-pass shapes the tester refuses (each has produced a false green before)

1. `running 0 tests` on any gate invocation — a filter that matched nothing (`#![cfg(miri)]` files
   natively, a renamed test). The FILTERED invocations of the two red-first gates (§8) must read
   `running 1 test`; the UNFILTERED invocations of their files must read `running 3 tests`
   (`ke16_nested_scope_occupancy`) and `running 2 tests` (`ke16_occupancy_gate`) with every name
   listed (`KE16-DESIGN-APP.md` §10).
2. **Healthy route twice**: a worker-route number whose receipt says `outer_worker_id =
   WORKER_ID_DISPATCHER` (4294967294) or `WORKER_ID_UNATTACHED` (4294967295), or
   `outer_same_pool=false`, or a system-closure receipt ≥ `MAX_WORKERS`. The bench panics on these
   after the receipt edits; a bench built from before those edits is not a valid instrument.
3. **Features not enabled**: `KE16_EXPECT` mismatch (the bench panics), or a criterion baseline name
   that does not match the printed `KE16 variant=` line.
4. **Equal counts over different sets**: a `cargo test` summary whose pass count equals the previous
   run's while the set of test names differs (the census on `#[ignore]` reasons guards the third
   mechanism); compare names, not counts, when un-ignoring.
5. **A different W**: `available_parallelism` differing between the two runs being compared.
6. **`release=false`** on a protocol-pass header.
7. **Widest color ≤ threshold** on the physics bench (it asserts; a run that skips the assertion by
   editing the scene is not the consumer).
8. **The `rows == n` assertion** in the ECS bench edited out or failing.
9. **Illegal feature pair building**: `cargo check -p boyko-threadpool --features ke16-a1,ke16-a2`
   must FAIL with the `compile_error!` message; a success is a red result for the feature scheme.
10. **Clippy "Finished" in under a second after an edit** — stale fingerprints; touch and re-run.
11. **A number from a feature build in the final table**: Step App's numbers come from the
    unconditional code only.
12. **A Miri run whose echoed `MIRIFLAGS` lacks `-Zmiri-tree-borrows`**: the run executed under
    Stacked Borrows, not the model the A1 obligation names; its result — green or red — is
    discarded and the run repeated with the full string of §8. Likewise a many-seeds run whose
    echoed string lacks `-Zmiri-many-seeds=` measured one seed.
13. **A W-d′ liveness reading taken from the external-arm tests**: a many-seeds timeout in
    `miri_scope_forced_cross_thread_transmute_is_clean`, `miri_scope_read_modify_write_borrowed_
    stack` or `miri_scope_multiple_distinct_borrows` is the documented external-arm window
    (`miri_scope.rs:56-59`, unchanged by W-d′) and is neither a W-d′ failure nor a W-d′ pass; the
    W-d′ gate is `nested_scope_from_worker_is_stolen_by_sibling` alone (§8).
14. **An A1 Miri red taken as "Tree Borrows unsound" without reading its KIND.** Three kinds. (a)
    A Tree-Borrows violation is an `error: Undefined Behavior:` report naming a tag and an access;
    it is routed to the code-reviewer's re-derivation of D5 first, and only a UB report that
    survives it triggers the A2 fallback. (b) A `spin_until timed out` panic is a LIVENESS defect
    in the wake protocol (`KE16-DESIGN-A.md` §1.3): routed to the developer (check `publish_fence`
    at `unpark_one_idle`'s prologue and the cascade's reachability, re-run loom M2 + its
    calibration) and NEVER grounds for the fallback. (c) A `receipt not observed` panic from
    `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` is NOT a defect: the
    protector-bearing inline case is deterministic only under B1 + LIFO, and under B0 or FIFO the
    sibling can take both nested-spawning bodies (`KE16-DESIGN-A.md` §1.3). The run is repeated
    with `-Zmiri-many-seeds=0..64`; the gate for that shape is "zero UB across every seed AND the
    receipt observed in at least one seed", and the tester records the number of seeds that
    observed it. A kind-(c) red counted as a UB failure, or a kind-(c) miss on every seed recorded
    as a pass, are both refused readings. The tester records the kind verbatim in every case.
15. **A loom calibration copy that stays green — or goes red for the wrong reason.**
    `loom_m2_calibration_no_producer_fence_is_lost`, `loom_m4_calibration_empty_gate_is_lost` and
    `loom_m4_calibration_no_self_exclusion_claims_self` must FAIL WITH THEIR OWN ORACLE'S MESSAGE:
    each carries `#[should_panic(expected = "<oracle prefix>")]` (§3), so a loom deadlock report or
    an unrelated assertion does not satisfy it. A green there means the model cannot observe the
    race it claims to check; a red on a different message means the copy is broken, not
    calibrated. Either way the corresponding M2/M4 green is discarded until the model is fixed. A
    calibration copy found with a bare `#[should_panic]` is itself a refused shape.
16. **An M1c reading labelled "green" when the joiner was modelled with the yield re-poll**: that
    model proves the unpark COUNT only (`KE16-DESIGN-W.md` §3.7); it is recorded as
    "M1c-count: green (count only)" and the route-(b) many-seeds Miri run is the sole liveness gate.

## 6. Bench and test edits the developer makes BEFORE Step 0 (part of the base commit)

- `crates/boyko_threadpool/benches/ke16_nested_scope.rs`: print `boyko_threadpool::ke16_variant()`
  and `available_parallelism` on entry; `KE16_EXPECT` check; `worker_wave` records
  `current_worker_id()` inside the outer task into an `AtomicU32` and asserts `< MAX_WORKERS` after
  the wave; a `ke16_park_timeout` group with rows `park_timeout_50us`, `park_timeout_1ms`,
  `park_timeout_2ms` (each `b.iter(|| std::thread::park_timeout(d))`, no unpark pending —
  `sample_size(20)`, `measurement_time(2 s)`); under `ke16-c-batch` a third route `worker_batch/...`
  and `dispatcher_batch/...` using `spawn_batch`.
- `crates/boyko_ecs/benches/ke16_par_iter_in_system.rs` and
  `crates/boyko_physics/benches/ke16_solve_in_system.rs`: the same witness + `KE16_EXPECT`; the
  system closure records `current_worker_id()` and the bench asserts `< MAX_WORKERS` once per
  `b.iter` batch (an `AtomicU32` `fetch_max`, read after the run). The physics bench gains the
  mandatory `bench_thread_install_Wminus1` row: the `bench_thread_install` shape over a second pool
  built once with `num_threads(W − 1)` (`KE16-DESIGN-APP.md` §11).
- `tests/ke16_nested_scope_occupancy.rs` and `boyko_ecs/tests/ke16_occupancy_gate.rs`: print the
  witness; `KE16_EXPECT` check when set. The former gains the B1-P receipt
  `parked_joiner_is_claimed_by_a_foreign_wave` under `#[cfg(any(feature = "ke16-b1", feature =
  "ke16-b3"))]` (`KE16-DESIGN-B.md` §2.7), which needs the new public accessor
  `ThreadPool::parked_mask() -> u64` (one `Acquire` load of `inner.idle`; App-11).
- `tests/cross_pool_routing.rs`: the three new tests (index §8; `KE16-DESIGN-APP.md` §5), with the
  `(pool address, worker id)` receipt in `install_on_foreign_worker_routes_to_global`.
- `tests/loom_pool.rs`: M2's producer fence replaced by the exported `publish_fence()` and its
  calibration copy; M1c, M2c, M4 (+ M4's two calibration copies), gated by the matching features via
  `cfg(feature)` inside the `#![cfg(loom)]` file; the fidelity note, the stale line references and
  the "#246" sentence corrected.
- `tests/miri_scope.rs`: `nested_scope_from_worker_is_stolen_by_sibling` and
  `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` (no external join in either:
  `pool.spawn` + `AtomicBool` spin-wait; `KE16-DESIGN-A.md` §1.3); line references corrected; the
  `:56-59` note scoped to the external arm.
- `boyko_ecs/tests/miri_phase9.rs`: depth-2 guard test; the ECS nested-system test (new file
  `boyko_ecs/tests/ke16_nested_system_inline.rs`).

## 7. Decision rules, applied in order at each step

**Step A** (reference for "regresses": the best candidate's cell, not today's — today's worker route
is serial and every candidate beats it):

1. Discard any candidate whose gates (§8) are red — after classifying the red per §5 item 14.
2. **Physics ranks.** Order the survivors by physics `in_scheduled_system` median (lower is
   better). A candidate that is worse on physics beyond 2× the band than another is behind it,
   whatever the grid says — the owner's criterion is throughput on the real consumer, and a variant
   that wins every 1 µs cell but loses the consumer loses.
⚠⚠ **RULES 2 AND 3 ARE AMENDED — 2026-09-08, AFTER RULE 3 DECIDED AGAINST A CANDIDATE THAT WAS
THEN MEASURED BETTER ON TEN CELLS.** The amendment is stated before the original text rather than
replacing it, because a decision rule rewritten after it has ruled is exactly the shape that should
be auditable. Both the old rule and the reason it was changed are preserved, and the order of
discovery is named: the defect was found BECAUSE the rule gave an answer the measurement
contradicted. Motivated scrutiny is still scrutiny — but only if the defect stands on its own, and
this one does, because it is a fact about the engine rather than a preference about candidates.

**THE DEFECT.** Rule 3's tiebreak gives the decisive vote to the **64W column**, and the engine
cannot produce that column:

* `BatchingStrategy::default()` sets `batches_per_thread = 1`
  (`boyko_ecs/src/ecs/core/iters/query/par_iter.rs`), so `chunk_size = entity_count / worker_count`
  and an archetype splits into **exactly W chunks — one per worker**. Reaching 64W requires
  `batches_per_thread = 64`; **no caller in the tree sets it**.
* `MIN_ARCHETYPE_FOR_PARALLEL = 1024` floors a chunk at 1024 rows, so the 1 µs body the tiebreak
  fires on needs a row costing under a nanosecond. The ECS harness's real chunk is **~82 ms**.
* The physics solver does not use that path at all — it cuts by colour group
  (`solver/colored.rs`), and its cut count is of the order of the colour count, not 64 × W.

So on 2026-09-08 the tiebreak discarded a 2–4 × win across six mid-body cells and a tie on BOTH
consumers, on the strength of one cell at a width and a body size nothing in the engine generates
(§A-RE in `KE16-RESULTS.md`).

**AMENDED RULE 2 — CONSUMER AGREEMENT, not physics primacy.** A candidate is ahead of another only
if it is ahead or tied on **every** consumer harness, each judged against its own band. Physics no
longer ranks alone. If the consumers disagree — one ahead on physics, the other on the ECS driver —
the pair is a **TIE at rule 2 and the disagreement is REPORTED**, never averaged and never broken by
picking the consumer that gives an answer. The owner's criterion is throughput on the real
consumers, plural: the pool serves the scheduler, ECS iteration, the physics solver and the MSDF
bake, and a rule that ranks by one of them ranks by one subsystem's shape.

**AMENDED RULE 3 — the veto surface is the widths the engine PRODUCES.** Within the rule-2 tie,
compare the worker-route 1 µs and 10 µs cells pairwise and in both directions **at `tasks = W`
only**, that being the width the default chunking emits and the one every shipped caller reaches.
Cells at 4W and 64W are **recorded and reported, and cannot decide** — 4W is reachable only by a
caller that sets `batches_per_thread = 4` (the API allows it; nothing uses it) and 64W by none.
If the pair still ties on that surface, the tiebreak is the **whole producible column** — 1 µs
through 1 ms at `tasks = W`, on both routes — read as a count of cells improved minus regressed,
and only then rule 4. **No single cell may override the rest of the grid and both consumers again.**

⚠ **WHAT THIS AMENDMENT DOES NOT DO.** It does not retroactively decide axis A. §A-RE's reversal
rests on ten improved cells, two regressed, a tie on both consumers and a green gate ladder — a
reader who rejects this amendment entirely still has to explain those numbers. The amendment
changes what the RULE would say next time; the measurement is what changed the verdict.

⚠ **THIS IS A SCOPE CALL AND IT IS FLAGGED TO THE OWNER** in `docs/OPEN-QUESTIONS.md`. Changing a
measurement rule mid-campaign is not a perf fork of the kind the orchestrator decides alone. The
amendment is applied so the campaign can continue on a coherent rule, and it is reversible: the
original text is directly below, unedited.

**THE ORIGINAL RULES 2 AND 3, PRESERVED VERBATIM:**

3. **The grid vetoes, symmetrically, within the physics band.** Among candidates within 2× the
   band of each other on physics, compare their 1 µs and 10 µs WORKER-route cells (all three task
   counts) PAIRWISE and in BOTH directions: if V regresses (§4 rule) any such cell against U while
   U regresses none against V, V is behind U; if each regresses the other on some cell, the 64W
   column decides (it is where the steal path dominates and where the per-element vs per-batch
   CAS difference of `KE16-DESIGN-A.md` §1.4 shows), then the 10 µs cells, then the pair is a tie.
   Revision 3's rule compared cells against the top-ranked candidate ONLY, so a candidate better
   on every 1 µs cell could never displace one that ranked first within the band (the critic's
   round-3 blocking item 1 and non-blocking item 10); this rule is the symmetric replacement for
   every pair, and it is the ONLY way `a1` and `a1f` are separated — never by an RMW count, whose
   two sides (`KE16-DESIGN-W.md` §0: LIFO ≈ `s` shared RMWs per task, FIFO ≈ `(1 − s) + s/33`,
   with `s ≈ 0.94` on the consumer shape) point in opposite directions from revision 3's ordering.
   The DISPATCHER-route 1 µs cells are a veto ONLY for `a2` and `a5` — the two candidates whose
   dispatcher-side code differs from today's in a way that can COST (A2's longer idle scan, W−1
   extra fenced probes per acquisition; A5's idle-keyed placement of the dispatcher's pushes).
   Under `a1`/`a1f`/`a3` the dispatcher's SPAWN path is byte-identical to today (pushes go to
   `injector_global`) and its IDLE path differs from today only by the removed stage-1
   empty-`Injector` probe (one fence + two loads fewer per acquisition, shared by all three) — so a
   dispatcher-route IMPROVEMENT there is real and expected, a difference AMONG the three is noise
   on the bimodal route, and neither is a veto.
4. **Tie-break, last.** Among candidates still tied after rules 2–3, prefer the smaller diff and
   the fewer shared RMWs on the SPAWNER's path, in this order: `a1f` and `a1` before `a3` before
   `a2` before `a5` (the spawner's path: 0 queue RMWs for the deque arms; 2 multi-writer for `a3`;
   2 single-writer + a longer idle scan for `a2`; 3 + a foreign line + a syscall for `a5`); an
   `a1` / `a1f` tie goes to `a1f` (today's constructor; no reversal path is reachable; the
   thief's path is one CAS per batch). This rule can only order candidates the measurement could
   not separate; it never overrides a measured difference.
5. The ECS ratio at both populations must be ≤ 1.15 for the winner (receipt; a larger ratio means
   the ECS driver is still serialising and the winner is not a fix).
6. If the winner is not `a1`/`a1f` by more than the band on physics: STOP, report (index §3 — B1
   needs a redesign for that substrate).

**Step B** (reference: `A*+b0`):

1. Rank by physics `in_scheduled_system`; B0 stays unless B1 improves it beyond 2× the band
   (the owner's "keep as is" is the default, not the exception).
⚠⚠ **STEP B RULE 2 IS AMENDED — 2026-09-08, and it is the SAME defect as Step A rule 3 in a
different grain.** There, the deciding cell was one the engine cannot produce. Here, it is one
produced by something that does not matter: the rule below anchors a POOL-WIDE axis-B verdict on
"the fontbake shape", and fontbake is the MSDF atlas bake — a load-time asset step, not a frame path
and by no reading a bottleneck. Both are the same underlying error: **a deciding cell chosen without
asking whether it represents work that matters.**

The citation itself is factually correct and was checked before it was doubted:
`boyko_fontbake/src/msdf/distance.rs::pick_band_rows` sets `target_bands = workers * 4`, so the bake
really does dispatch 4W tasks. What is wrong is not the arithmetic but the anchoring.

**THE DISPATCHER THAT MATTERS IS THE SCHEDULER.** `boyko_ecs/.../schedule/schedule.rs:455` calls
`pool.install(|scope| self.executor_main_loop(world, scope))` — ONE install per frame, wrapping the
entire executor main loop. The calling thread is the frame thread and it is the external joiner for
the whole frame. That is the arm `b1` and `b3` actually differ on.

**AND THE TWO DISPATCHERS POINT IN OPPOSITE DIRECTIONS**, which is what makes this a real fork
rather than a formality:

| dispatcher | what its calling thread is doing | what it argues |
|---|---|---|
| fontbake (`install` from an app thread) | nothing — it has no other work while the bake runs | helping (`b1`) is a free extra lane ⇒ **against `b3`** |
| the scheduler (`install` per frame) | it IS the dispatcher: it scans for ready systems and **parks between rounds** (`schedule.rs`, "dispatcher parks below and wakes on completion") | if it takes a task under `b1`, the next system's dispatch waits for that task to finish — latency injected into the schedule's critical path ⇒ **for `b3`** |

⚠⚠ **AND THE DECIDING PROPERTY IS NOT MEASURED BY ANY HARNESS THIS CAMPAIGN HAS.** The scheduler's
question is *"does the frame thread's dispatch of the NEXT system get delayed"*, and that is neither
a width nor a body size. The pool grid measures a wave's makespan, not the latency of the thread
that hands waves out. So:

* `b3` vs `b1` can be settled on THROUGHPUT alone only if one dominates the other across the grid
  and both consumers. If it does, take that verdict.
* **If they are close, the honest verdict is UNDECIDED pending a scheduler harness** — and it must
  be filed that way rather than broken on the fontbake cells, which is the anchoring this amendment
  rejects.

**FONTBAKE IS NOT CHANGED**, and the owner offered to change it. It is not a constraint here: the
amendment moves the RULE, not the bake. Two reasons for leaving `pick_band_rows` alone — it is not a
bottleneck, so a change buys nothing measurable; and 4× over-decomposition is defensible on the
bake's own shape, since MSDF band cost is uneven (a band crossing glyph edges evaluates distances
against every edge, an empty band nearly nothing) and over-decomposition is what buys load balance.
Revisit it when the bake has a harness, and measure `top_lane` rather than the wall clock, because
`4W` vs `W` is an argument about balance and balance shows up in occupancy.

2. `b3` vs `b1`: decided on the dispatcher-route 100 µs and 1 ms × 4W cells (the fontbake shape) and
   on `top_lane` from the occupancy test; `b3` is taken only if those cells are ties or better
   (owner question 2 may override on code size).
3. Receipt: on the worker route `top_lane ≤ 2 × tasks / W` for the winner at 200 µs × 4W (the
   occupancy test); B0's ~33-of-64 signature (the ≤33 batch) must be gone unless B0 won.
4. **The Step-B verdict fixes the physics reference row** for every later step and for the
   acceptance line: `REF = bench_thread_install_Wminus1` if B0 or B1 won (the bench thread's
   external joiner helps: W−1 workers + 1 = W lanes), `REF = bench_thread_install` if B3 won (the
   external joiner parks: W workers = W lanes). Under B3 the row ratio `bench_thread_install_Wminus1
   / bench_thread_install` must read ≈ W/(W−1) — a receipt that the joiner parked; a ratio near 1
   there means the external arm is still helping and the B3 build is not B3.

**Steps W1, W2, C, F** (reference: the running best):

1. Keep the candidate only if (a) it regresses no 1 µs cell beyond 2× the band AND (b) it improves
   at least one consumer or at least one grid cell beyond 2× the band.
2. Exception, W-d′ (`wc`): kept if (a) holds and it is a tie everywhere — it removes one
   multi-writer RMW per task and closes the route-(b) lost-wakeup window of `KE16-DESIGN-W.md`
   §3.4 — PROVIDED both its gates are green: loom M1c (a real-park M1c; a count-only M1c does not
   satisfy this clause on its own, §5 item 16 — then the many-seeds gate carries the liveness
   obligation alone) and the route-(b) many-seeds gate (§8). A red on either is a defect in the
   gated arm: the developer fixes it or the feature is dropped; there is no W17 fallback (it would
   fail the same gate).
3. `c1` (batch spawn) not kept ⇒ `c1f` is not measured (it needs the batch push).
4. The final configuration is `A*+B*+W*+C*`.

## Precondition on every timed step: the machine must be IDLE, and the run must prove it

Owner, 2026-09-02: *"if tests were run in the last hour they will need redoing, because I was
playing Dota 2."* This box is the owner's workstation, not a bench rig. A game holds gigabytes
resident and saturates every core, and nothing in this plan would have noticed: a contaminated
number arrives looking exactly like a clean one.

**This applies to the TIMED steps only, and the distinction is worth stating because it decides how
much has to be re-run.** A structural verdict — does it compile, is the feature actually enabled,
does the witness match, is a test ignored or vacuous, does a binary segfault before `main`, do two
test-name sets differ — is indifferent to machine load and does not need re-taking. Everything that
is a ratio of times is not, and neither is any test whose assertion is a scheduling observation
(`cross_pool_routing`'s "a B worker ran at least one" is the measured example; its flake rate was
taken under exactly this load and is discarded, though the reason the assertion is unsound is
structural and stands).

Therefore, for every step that reports a time:

1. **Ask before running.** The tournament is not started until the owner says the machine is free.
   This is a dependency, not a courtesy: a step re-run later costs the same as a step run right.
2. **Record a load receipt beside every number**, in the same table: what else was running. The
   cheapest sufficient form on Windows is the process list filtered to anything holding more than a
   few hundred megabytes, captured immediately before and immediately after the timed region, plus
   `available_parallelism`. A number whose before and after receipts disagree is re-taken.
3. **The spread rule already in this document is necessary and not sufficient.** Two runs minutes
   apart under a steady game load agree with each other perfectly and are both wrong. Agreement
   between runs proves reproducibility, not cleanliness — the same distinction this campaign already
   recorded about gates that re-run a published command.
4. **State the machine's other occupants in the results file.** `KE16-RESULTS.md` carries the
   topology (App-9) already; it carries this too, because a reader a year from now comparing against
   a re-take needs to know what the box was doing.

**Step App**: the unconditional code's numbers must match the winning feature build's within the
band on every cell and consumer; a difference beyond the band is a defect in the removal step, not a
new datum.

**Step App, first half — FREEZE BEFORE REMOVING (owner ruling, 2026-09-02).** The losing candidates
are not destroyed. *"Do not delete the unsuitable one, leave them as a spare — so the code is
recorded but not present in the project."* The removal commit is therefore preceded by a freeze,
and the order is not negotiable: freeze, then remove, because after the removal there is nothing
left to point a tag at.

1. **One annotated tag, `ke16/tournament`**, on the last commit at which every candidate still
   builds — that is, the commit whose gates Step 7 ran. ONE tag rather than one per candidate: the
   candidates coexist in a single tree behind mutually exclusive features, so per-candidate tags
   would all address the same commit and would falsely suggest independent snapshots. The tag
   message carries the verdict line for each candidate and the exact feature flag that builds it,
   so `git show ke16/tournament` answers "what was tried" without a checkout.
2. **A register, `docs/threadpool/KE16-REJECTED.md`**, written in the removal commit. One row per
   candidate that did not ship: what it was, in one sentence; the measurement that eliminated it,
   with the number and the cell it was taken in; the feature flag and the tag that build it; and —
   the column that decides whether this file is worth keeping — **the condition under which it
   should be reconsidered**. A condition is a fact about the world that could change, not a wish:
   "if the machine exceeds 16 hardware threads", "if the physics waves stop being latency-bound at
   the chunk sizes the solver produces", "if defect B is ever fixed by a route other than B1". A
   row with no such condition says the candidate is closed, and says so explicitly rather than
   leaving the reader to guess.
3. ⚠ **The register states its own decay, in its header, because a "spare" invites a false
   expectation.** A frozen candidate is a snapshot, not a part on a shelf. The tree moves; within
   months it will not apply to the current code and will not build against it. Its value is that
   it shows HOW the thing was done and WHY it lost, at a commit where that was measured — not that
   it can be switched back on. Any claim to the contrary is refuted by the tag's own age.
4. The removal commit's `grep -rn 'feature = "ke16' crates` still must return nothing, and the
   shipped crate still carries no `cfg` residue: the freeze changes what is RECOVERABLE, not what
   is BUILT.

**The physics acceptance line** for the whole pass (`KE16-DESIGN-APP.md` §11): (1)
`in_scheduled_system < single_threaded_O5` (31.52 ms today); (2) PRIMARY:
`in_scheduled_system − empty_schedule_control ≤ REF × (1 + band)`, where `REF` is the W-LANE
row chosen by lane count in Step B rule 4 — `bench_thread_install_Wminus1` under B0/B1,
`bench_thread_install` under B3 — a W-lane reference against the W-lane scheduled route, no
structural allowance; reported beside it, raw, the ratio against the OTHER row (W+1 lanes under
B0/B1; W−1 lanes under B3), with the row name and the lane-count reasoning written next to the
number. If no configuration reaches (1), parallel physics on the shipping route is still a loss
and that is the finding — reported, not hidden behind the healthy-route number.

## 8. Gates per configuration (before its benches)

```powershell
# Build and lint. The default (no-feature) build is checked workspace-wide (a root `cargo check`
# without --workspace is vacuously green); the feature build is checked per package, because the
# `pkg/feature` syntax is guaranteed only for a direct dependency of a selected package.
cargo check --workspace --all-targets
cargo check -p boyko-threadpool --all-targets --features <F>
cargo check -p boyko-ecs -p boyko-physics --all-targets --features boyko-threadpool/<F-list>
cargo clippy -p boyko-threadpool --all-targets --features <F> -- -D warnings

# Tests, no fail-fast (a red target hides every target behind it otherwise)
cargo test -p boyko-threadpool --all-targets --no-fail-fast --features <F>
cargo test -p boyko-threadpool --test cross_pool_routing --features <F>
cargo test -p boyko-threadpool --test ke16_nested_scope_occupancy worker_spawned_wave_reaches_at_least_half_the_workers --features <F> -- --ignored --test-threads=1 --nocapture
cargo test -p boyko-ecs --test ke16_occupancy_gate par_iter_in_system_reaches_more_than_one_thread --features boyko-threadpool/<F> -- --ignored --test-threads=1 --nocapture
cargo test -p boyko-physics --lib --features boyko-threadpool/<F>      # the {1,N} bit-identity oracles

# loom (M1..M4 as applicable to the features). The calibration copies are `#[should_panic]`-shaped
# tests inside the same file: a GREEN loom_pool run means every calibration copy went red as required.
$env:RUSTFLAGS = "--cfg loom"; cargo test --release -p boyko-threadpool --test loom_pool --features <F>
$env:LOOM_MAX_PREEMPTIONS = "3"; cargo test --release -p boyko-threadpool --test loom_pool --features <F>
Remove-Item Env:RUSTFLAGS; Remove-Item Env:LOOM_MAX_PREEMPTIONS

# Miri. The FULL flag string every time: `$env:MIRIFLAGS` REPLACES the [env] default
# (`-Zmiri-tree-borrows`, .cargo/config.toml:14-15); the header of tests/miri_scope.rs (:61-69)
# lists the other four flags the primary surface needs. Echo the variable first; the echoed line
# goes into the results file (refused shape 12 if it lacks -Zmiri-tree-borrows).
cargo +nightly miri --version
$env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks"
Write-Output "MIRIFLAGS=$env:MIRIFLAGS"
cargo +nightly miri test -p boyko-threadpool --test miri_scope --features <F>
$env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..16"
Write-Output "MIRIFLAGS=$env:MIRIFLAGS"
cargo +nightly miri test -p boyko-threadpool --test miri_scope --features <F>
Remove-Item Env:MIRIFLAGS

# Feature-scheme negative gate (once, at Step 0): each must FAIL
cargo check -p boyko-threadpool --features ke16-a1,ke16-a2
cargo check -p boyko-threadpool --features ke16-b1,ke16-b3
cargo check -p boyko-threadpool --features ke16-a3,ke16-b1
```

The two `--ignored` gate invocations are RED until an A candidate is in place (they are the
red-first gates); for `a0` their failure is the expected reading and is recorded as such. For every
`a*` configuration they must be green with `running 1 test` (the name filter is part of the
command; without it the files print `running 3 tests` / `running 2 tests`, `KE16-DESIGN-APP.md`
§10).

**At Step 0, additionally** (the base carries W-a's fence): the `loom_pool` run above with NO
`--features` must show `loom_m2_idle_race_c_no_lost_wakeup` green with the model's producer fence
being the exported `publish_fence()`, AND `loom_m2_calibration_no_producer_fence_is_lost` behaving
as a `#[should_panic]` (i.e. the un-fenced copy reproduces the lost wake). Paste both names with
their verdicts into the results file §receipts; refused shape 15 applies.

**Additional gates by feature.**

- `ke16-a1` / `ke16-a1-fifo`: the Miri run above must list BOTH new shapes —
  `nested_scope_from_worker_is_stolen_by_sibling` and
  `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` — and the second must print
  its inline receipt (a nested-spawning body ran on the outer worker's id) in at least one seed of
  the 16-seed run; a `receipt not observed` panic is kind (c) of §5 item 14 (not a defect —
  re-run that test alone with `-Zmiri-many-seeds=0..64`; the gate is zero UB on every seed AND the
  receipt on at least one; record the count). Under `ke16-a1,ke16-b1` the receipt is deterministic
  (`KE16-DESIGN-A.md` §1.3) and a miss there IS a defect (the joiner is not popping its own back).
  Under `ke16-b1` the second shape is run again (it is the B1-specific protector shape,
  `KE16-DESIGN-B.md` §2.7). Any red is classified per §5 item 14 before it is acted on.
- `ke16-b1` / `ke16-b3`: `cargo test -p boyko-ecs --test ke16_nested_system_inline --features
  boyko-threadpool/<F-list>`; `cargo test -p boyko-ecs --test miri_phase9` (native) for the depth-2
  guard case; the B1-P receipt `cargo test -p boyko-threadpool --test ke16_nested_scope_occupancy
  parked_joiner_is_claimed_by_a_foreign_wave --features <F> -- --test-threads=1 --nocapture`
  (`running 1 test`; the foreign task's receipt must be the parked joiner's id,
  `KE16-DESIGN-B.md` §2.7).
- `ke16-a5`: loom M2c is in the `loom_pool` run above (feature-gated inside the file).
- `ke16-w-gate`: loom M4 is in the `loom_pool` run above; the tester confirms in the output that the
  two calibration copies (`loom_m4_calibration_empty_gate_is_lost`,
  `loom_m4_calibration_no_self_exclusion_claims_self`) went red as `#[should_panic]` requires — an
  M4 that cannot fail is not a gate.
- `ke16-w-count`: loom M1c is in the `loom_pool` run above (recorded as "M1c" only if it parks for
  real; as "M1c-count" otherwise, §5 item 16). The liveness gate is the ROUTE-(b) test alone, at
  32 seeds:

  ```powershell
  $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..32"
  Write-Output "MIRIFLAGS=$env:MIRIFLAGS"
  cargo +nightly miri test -p boyko-threadpool --test miri_scope nested_scope_from_worker_is_stolen_by_sibling --features <F>
  Remove-Item Env:MIRIFLAGS
  ```

  Required: `running 1 test`, zero liveness timeouts across the 32 seeds. The other tests of the
  file are run at 16 seeds in the block above and their documented ~1/16 external-arm timeout,
  if it appears, is recorded as "expected, external arm" — refused shape 13 forbids counting it
  either way.
- `ke16-c-batch`: the `spawn_batch` k<n / k==n / k>n unit tests are in the `--all-targets` run; the
  physics `{1, N}` oracles are the `--lib` run above; `cargo test -p boyko-ecs --lib` for the
  `par_chunk` driver's own tests (`par_chunk.rs:645-1016` install-driven tests).

At Step App, additionally: `cargo test --workspace --all-targets --no-fail-fast`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p boyko-log --test
l14_sink_policy -- --ignored --test-threads=1` (the device-free leg, `running 1 test`), the ignore
census (`cargo test --test ignore_reasons_census`), the loom and Miri runs above with NO
`--features`, and `grep -rn 'feature = "ke16' crates` returning nothing.

## 9. Results file layout (`docs/threadpool/KE16-RESULTS.md`)

§0 environment (toolchain, Miri version, W, topology, `park_timeout` medians, the echoed `MIRIFLAGS`
lines, the two loom calibration readings);
§A one table: rows = the five configurations, columns = the 24 grid cells (median, band, flag) + ECS
ratios at both populations + the five physics medians, then the applied rules and the verdict line;
§B, §W1, §W2, §C, §F the same shape against the running best; §App the unconditional numbers, the
acceptance line with both raw ratios (against `REF`, the W-lane row named by Step B rule 4 —
primary — and against the other row), the row name and the lane-count reasoning written beside the
number, and the O-series retake line; §receipts the pasted occupancy-test lines
per configuration, the Miri red classifications (kind verbatim, if any), and the M2/M4 calibration
readings; §refused a list of any run that hit a §5 shape and was discarded, with the reason. Every
number carries its baseline name (`<V>-r<N>`).
