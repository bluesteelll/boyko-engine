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

**Step App**: the unconditional code's numbers must match the winning feature build's within the
band on every cell and consumer; a difference beyond the band is a defect in the removal step, not a
new datum.

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
