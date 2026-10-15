# sr-policy — the lane record: S1 (`SolvePolicy = V2Policy`) and the MD2 gate

sr-policy is LEVERS-R2-PLAN §2.3 reduced by window SR's verdicts (orchestrator ruling 37, R6 and R7).
What is left is one value-neutral commit, S1, plus the MD2 counter that ruling 32 carried over from
the SR-B close test. The branch is `u/phys-sr-policy` from `c2b37b98` (`integ/unified`: SR-B, S5 C1-C3,
SL, FM C1, render RD1/RD3/RD5/E0/PXe/OBS).

- The cut is `D:/tmp/phys-orch/sr-policy/cut.md` and its critique is `sr-policy/critique.md`. Ruling 52
  (`phys-next/RULINGS-2026-10-15-AM.md`) answers both.
- The implementation report is `sr-policy/impl.md`. The codegen receipt is `sr-policy/cg/receipt.md`.

Code is cited by symbol. This file is not in `tests/internal_docs_anchors.rs`'s `GATED_DOCS`, so its
anchors are not machine-checked.

---

## 1. What landed

| commit | subject | what |
|---|---|---|
| C1 `a1f440a2` | `test(physics): sr-policy MD2 - a feature-gated ARMED=true region dispatch counter …` | the `region-armed-counts` feature; the counter in `SolveStages::item_begin`; `armed_region_dispatches()`; `tests/sr_region_armed_dispatch.rs`; its CI leg |
| C2 `afa0cfae` (= S1) | `refactor(physics): sr-policy S1 - the solve region names its policy through SolvePolicy (= V2Policy) …` | the alias at every policy site; `region_helper_advances()`; `sr_region_counters`' fourth test; the runner's `helper_advances_per_step` |
| C3 | `docs(physics): sr-policy - the lane record` | this file |

**The alias.** `solver/colored.rs` declares a private `type SolvePolicy = V2Policy;` after
`RegionRoute`, and every site whose meaning depends on the policy names it:

- the claim-column sizing, `TableBuilder::entry`'s `claim_lines::<SolvePolicy>`;
- the second clause of `RegionFrame::new`'s SAFETY comment in `solve_region`;
- the `RegionColumns.claims` doc;
- both arms of `region_on_pool`;
- both arms of the `cfg(all(test, miri))` `Threads` route.

Before S1 the sizing and the dispatch were two independent spellings of `V2Policy`, so a one-line flip
of the dispatch could under-size the claim lines. Now one name sizes the frame and runs it, and
changing the alias line is the whole policy flip. A flip re-takes the codegen receipt and CR-F1's
gates.

The structural check (critique O4) holds: `V2Policy` occurs in `colored.rs` only in the import, the
alias's doc and the alias itself, and no turbofish names it.

## 2. What was not made, and why

- **No successor policy type** (R7). No arm was claimed faster in all 12 cells: Rc in 7 of 12, `all`
  in 4 of 12, and Ra was claimed slower in 10. `V2Policy` stays the shipped policy and is not edited.
  `boyko_threadpool` is untouched.
- **`RegionGrain::DEFAULT` (b6-m64) is kept** (R6). No candidate, 0 of 12, was claimed faster.
  Raising the minimum points to 256 costs 27-30 % at W16.
- **S2 and S3 are not made, and POL-AB has nothing to time** (R7). CR-F1's ω_b bar item (1) failed.
  - It was claimed in only 2 of 12 cells, P8 st36 bpp1 (0.9352) and bpp2 (0.9455).
  - P16 was claimed in 0 of 6 cells (0.935..0.962).
  - S3 "exists only if CR-F1's omega bar (1) held", so it does not exist.
  - This paragraph is PC-SR-POL3's and PC-CR-6's record of the cells (`win-sr/analysis/final.md`
    §4.3).
  - CR-F0's finisher stays an unused `boyko_threadpool` axis at zero cost (`01-CR-F-DESIGN.md` §5,
    PC-CR-6).
- LEVERS-R2-PLAN's `sr-policy-ab` and `sr-policy-flip` legs (§2.3) and POL-AB (§4.2) are void (PC-SRP-2).

## 3. The MD2 gate

**Why it exists.** `solve_region` picks the region's monomorph once per region from
`zone_enabled!(PHYS_REGION_OPENS)`. The SR-B close test found that a selector which always answers
`true` (MD2, `core::hint::black_box(true)`) survived every gate, the disarmed codegen receipt included.

- An ARMED=true region on a disarmed step pushes no sample and changes no bit.
- The receipt proves what the ARMED=false monomorph contains. Under MD2 both monomorphs stay built
  and referenced, so the receipt cannot see the swap.

**The witness.** `region-armed-counts` is a feature that no crate of the workspace enables (the
precedent is `bp-query-counts` / `narrowphase-counts`).

- Under it, `<SolveStages as RegionStages>::item_begin` adds one to a process-wide `AtomicU64`
  (Relaxed) on item 0, and `solver::colored::armed_region_dispatches()` reads it.
- The region protocol calls `item_begin` only from an `ARMED = true` monomorph, on participant 0
  (`RegionStages::item_begin`'s contract; `run_orchestrator`'s `if ARMED`). So the count follows the
  monomorph that ran, not the boolean that chose it (ruling 52, W1 (b)).
- An earlier draft keyed the count to the `if armed` block. That version would have missed MA, the
  swapped arms (critique W1).

**The gate.** `tests/sr_region_armed_dispatch.rs` is `harness = false` (the arm mask is
process-wide) with `required-features = ["region-armed-counts"]`. Without the feature, cargo refuses
the named target and exits 101 ("requires the features: `region-armed-counts`"), so the gate can
never read as a vacuous `running 0 tests`.

It runs one world on `support/profiling_harness.rs`'s scene (W4, `parallel_solve`, a wide floor
colour). Per step, `a` is the rise of the ARMED=true count and `r` the rise of `region_dispatches`.

- **Disarmed:** 12 steps, with `a == 0` on every step and `r == 1` from step 4.
- **Armed:** `arm_profiler`, then 8 steps, each folded, with `r == 1` and `a == r`.

Arming mid-run in the one world works, so the pre-registered two-scene fallback (critique Q1) was not
needed. The green run reads "12 disarmed steps ran 12 regions, 0 of them ARMED = true; 8 armed steps
ran 8 regions, 8 of them ARMED = true", in debug and in release.

The gate runs in CI as the third leg of the `feature-count-tests` step, with its `rc=1` arm and
`grep -qx 'running 1 test'` (ruling 52 Q2, critique O1).

**Shown red.** Each mutation was applied alone in the lane's tree, then reverted by content with the
md5 re-checked:

| mutation | profile | red |
|---|---|---|
| MD2: the selector reads `core::hint::black_box(true)` | debug, release; re-run on S1's spelling | `disarmed step 0: a = 1, r = 1: a disarmed step dispatched the ARMED = true region monomorph` |
| MD: the selector is the literal `true` | debug | the same |
| MA: `region_on_pool`'s two const arguments swapped | debug; re-run on S1's spelling | the same |
| half swap: both arms `ARMED = true` | debug | the same |
| NOINC: the increment deleted (the counter's own anti-vacuity) | debug | `armed step 0: a = 0, r = 1: the armed step's regions did not run the ARMED = true monomorph` |

MA was recorded before only as caught by `profiling_zone_counts`, and in debug only
(`sr-b/close_test.md`). It is now also caught by a gate that runs in both profiles.

## 4. The getter and the key

**`ColoredSoftStepSolver::region_helper_advances()`** (doc-hidden, `#[inline]`) is the last region's
`RegionReport::helper_advances`, the publishes the participants 1.. made (cut D2, ruling 52 Q3).

- The datum already exists once per region: `solve_region` stores the report as `region_last`.
- A cumulative field would have added 8 B to the solver Resource, a load-add-store to
  `solve_region`, and new size/initialiser immediates in five setup functions.
- Like `last_region_report()`, the value is stale on a step that opened no region, so a reader pairs
  it with `region_dispatches()`.

**`helper_advances_per_step`** is a parity-runner SUMMARY key, between `layout_census` and `w8s`.

- Its value is the getter's sum over the window's steps whose `region_dispatches` rose, divided by
  the window's steps. It is read untimed after every step, on colored rows armed or not.
- It is `null` on reference rows and on an empty window.

**The value on S1 is 0 by construction.** `V2Policy` is `Advance::Orchestrator`, and an armed region
or a W1 step also gives 0. `sr_region_counters::the_shipped_policy_advances_every_item_on_participant_0`
pins that behaviourally (`pile(1200, 40)`, W8 and W16, at least 10 steps each). On every step it
asserts:

- `region_helper_advances() == 0`;
- the identity `advances == published + 1`, in release too;
- non-vacuity: the helpers ran blocks.

That test makes two new catches (critique O3):

- a shipped policy that takes the finisher axis (SR-B's close-test mutation MB). MB was caught before
  only by the threadpool lib test and the disarmed receipt;
- a broken identity on the physics region in release, which closes the SR-B close test's finding 2
  for physics.

**R-ADV (shown red).** The alias was set to `boyko_threadpool::WithAdvance<V2Policy, true>`, the line
an S3 would have shipped.

- The test fails at `W8 step 0` with `helper_advances: 9` (`published: 26`, `advances: 27`; the
  identity still holds).
- The runner from that tree (release; jolt, 100 steps, window `0..100`) reads
  `helper_advances_per_step` 215.79 at W8, 0 at W1 and 0 at W8 `--arm-profiler`.
- The exe is kept as `sr-policy/bin/adv_runner.exe`. It is G-SRP-SUM's negative control (ruling 52
  W3).

If a finisher policy ever ships, the `== 0` pin becomes CR's G-CR-F1-VAC (`> 0`) in that lane's
commit, and the runner key gains CR's void rule (PC-SRP-7).

## 5. The codegen receipt (cut D5)

The lib asm was built release, `lto = false`, one codegen unit, `--emit asm`, default features, each
side from a `git archive` export with its own target dir. PARENT is `c2b37b98`. TIP is S1's tree,
`9edc8db6` (= `afa0cfae^{tree}`). **VERDICT: PASS.**

| clause | reading |
|---|---|
| (i) `same_fns.py parent.s tip.s` | `SAME`: 1556 = 1556 functions, 0 only-in, 0 differing. The getter is not emitted, so the pre-registered one-function alternative was not needed |
| (ii) `disarmed.py tip.s` | `VERDICT: PASS`: 7 + 7 region functions; ARMED=false reaches 32 functions and 0 clock reads |
| (iii) `ARMED_REGION_DISPATCHES` | 0 occurrences in `tip.s`; 8 in `tipf.s` (the feature build) |
| R-CG (the alias set to `WithAdvance<V2Policy, true>`) | DIFFERENT. Both `V2Policy` region families are only in A (14). The `WithAdvance` ones and `p0_boundary` are only in B (14). `solve_region` and its 23 dtor funclets differ |
| R-FEAT (`--features region-armed-counts`) | DIFFERENT, 43 functions. 1 is structural: `item_begin` gains the item-0 test and `lock incq ARMED_REGION_DISPATCHES`. 42 differ only in `TypeId` literal-pool constants, because the feature enters the crate's metadata hash. No ARMED=true region function differs, because `item_begin` is out of line |

The receipt proves the lib is codegen-neutral (critique O5). It does not prove the parity exe's
physics code byte-unchanged after the runner's edit enters fat LTO. Values are proved on the TIP exe
by the pin gates below.

This confirms ruling 44's premise: S1 is value-neutral and the region monomorph is unchanged, so
nothing of SCREEN-L leg A's export needs a re-run.

## 6. Gates and tallies

**Developer, on each commit** (stable msvc, `D:/wt/_targets/fm-msvc`):

- check and clippy:
  - `cargo check --workspace --all-targets`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo clippy -p boyko-physics --all-targets --features region-armed-counts -- -D warnings`.

  Each exits 0, on C1 and on C2.
- `sr_region_armed_dispatch`: 1/1 in debug and release, on C1 and C2. Without the feature it is
  refused (exit 101).
- `sr_region_counters`: running 4 tests, 4 passed in both profiles, by name:
  - `region_counts_equal_the_builder_replica`;
  - `helpers_run_blocks_on_every_step_at_w8_and_w16`;
  - `a_warmed_region_step_allocates_two_per_scope_it_opened`;
  - `the_shipped_policy_advances_every_item_on_participant_0`.
- `sr_region_worker_invariance`: 1/1 in both profiles. The file is byte-unchanged against `c2b37b98`.
- SR-B's PHYS set, both profiles:
  - `sr_region_store` 2;
  - `profiling_zone_counts` 1;
  - `profiling_bit_identity` 1;
  - `default_world_worker_invariance` 3;
  - `frozen_island_warm_start` 9.

  All passed.
- lib: debug 366 (365 passed, 1 ignored), release 364 (363 passed, 1 ignored). The source diff adds
  and removes no `#[test]`.
- C3: `internal_docs_anchors` and `ignore_reasons_census` (impl report).

**Tester, on the final HEAD** (cut §5): the trunk pins P1-P9 and P11, the TIP runner's `runner_pins`
and both pose-gate sets, G-SRP-SUM with its two negative controls, the Miri SB leg, and one workspace
run. The Miri leg pre-registers both of its tests by name (ruling 52 W2): running 2 tests,
`sr_region_sb_route_runs_every_kind_on_both_threads` and its `_fm` twin. P10 is not run: no J-T value
can move, because the lib asm is SAME. The tester's report holds the tallies.

**No pin moves.** Three counts change, each declared:

- `sr_region_counters`: 3 → 4;
- a new feature-only binary, `sr_region_armed_dispatch`: 1;
- the ignore census's `.rs` count: +1.

No gate script's row set changes.

## 7. PC lines (recorded, not done here)

- **PC-SRP-1** (orchestrator): sr-policy's entry in `levers/00-RULINGS.md` (merge sha, receipt) after
  the merge.
- **PC-SRP-2** (orchestrator): LEVERS-R2-PLAN §2.3's `sr-policy-ab` / `sr-policy-flip` and §4.2's
  POL-AB are void.
- **PC-SRP-3:** closed. The MD2 gate is in CI (ruling 52 Q2). The orchestrator adds it to the lane
  template's standing pins as P12, beside `sr_region_counters` running 4 (Q5).
- **PC-SRP-4** (kit-owned): the kit README's P10 row still lists V2's JSonT500 / JSonToff500. The tree
  carries SL's `0xc637_a051_7ada_bab7` / `0x4aec_2a5f_78f4_88d0`.
- **PC-SRP-5:** `tests/support/profiling_harness.rs`'s header still says "the two profiling validation
  binaries". There are three now, so that line is stale. The file was outside this lane's lock.
- **PC-SRP-6:** `01-CR-F-DESIGN.md` §8 PC-CRF-4 (SR-B's replica check under `WithAdvance`) is moot
  while S3 is not made.
- **PC-SRP-7:** if a finisher policy ever ships, the new `sr_region_counters` test's `== 0` turns into
  G-CR-F1-VAC (`> 0`), and `helper_advances_per_step` gains CR's void rule, in that lane's commit.
