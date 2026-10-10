VERDICT: APPROVED WITH CHANGES; BLOCKING=0; IMPORTANT=6

# Architecture review: `boyko_leg` rev 3 (silent-skip verdicts, device-leg runner)

## Verdict
**APPROVED WITH CHANGES.** The plan is complete: it ends at "Sources", and every section is present. No blocking remarks. The six important remarks below are gaps in the plan text; none needs a redesign.

**Rev-1 critique items (`critique.md`), each checked against rev 3 and the trees:**
- **B1, missing gates and commit order: resolved.** §9 and §10 are present, and every gate G1–G17 names a red-first run or a mutation.
- **I1(a), children flagged as foreign: resolved** by the `<-` chain plus `child_command`.
- **I1(b), drivers stripping the ledger variable: resolved.** `LegCommand` has no `env_clear`, and `env`/`env_remove` panic on reserved keys. The vkval drivers use per-key `env_remove` (`boot_validation_clean.rs:702-704`), which now panics loudly on a reserved key.
- **I1(c), worker skips unjudged: resolved** by judge rows 12–14 and orphan-worker.
- **I2, validation forced off at spawn sites: resolved** by rule (f) plus one posture definition.
- **I3, no oracle for App tests: resolved.** `done()` runs the oracle, and `HostFramesPresented` witnesses frames. One gap remains; see I1 below.
- **I4, membership leaks: resolved**, via features read from `cfg` and type-forcing through (h1)+(d2). The window before C12 stays open; see I6.
- **O1–O8 and OQ1–OQ3: resolved.**

**Rev-3 claims I verified:**
- `runner.rs` lines :237, :255, :296, :250-253, :943-947, :1013, :1087 and :2776.
- `host.rs:31-46` has 5 variants, so one variant per variant plus 3 runner stages is right.
- vkval `device.rs:820-838`. `device.rs:952`: `boot_singleton` → `Self::boot` → `create_instance`, so a single increment site holds.
- vkval `debug.rs:149-152`.
- `compute.rs:62-70`: the lock is held for the boot call only, as the plan says.
- `Cargo.toml:137-142`.
- `boyko_demo`'s dependencies, so UG-15 strict is identical by construction.
- The 17 `enable_validation:false` / default-config sites: my grep gives the same count.
- `InstanceConfig` has only two fields (`device.rs:160-182`), so `vk::boot(leg, windowed)` loses nothing.
- R-4 at `sdf_gbuffer_hybrid.rs:6967` sits in an ignored test (`:6784`).
- Rule (g)'s four waivers match the whole tree: the other 11 files that use those tokens are boyko_app files that get migrated.

**Runner cannot go green on an all-skip or zero-measured run:** confirmed. Exit 0 requires F = 0 and P ≥ 1. SKIPPED and KNOWN-RED do not count toward P. `allow` rows are limited to device-absence classes.

## Important

### I1. A terminal device error mid-run still reads PASS
- **Where:** D12 and the `judge_run` table.
- **Problem:** the frame loop has two terminal exits:
  - `runner.rs:1324-1329`: the frame fence wait fails and the loop returns;
  - `:2784-2787`: render returns `Err` and the loop returns.
  
  Both fall through to teardown (`:947`) and `AppExit(true)` (`:1005`). `diag.rs:154-167` only logs E3003. `judge_run` has no row for the loop's exit cause, and `Ran.exit` is always `AppExit(true)`.
- **Consequence:** on owner-rtx3060, any App member (the 24 scene tests, H10 ×13, the dump members) that hits `VK_ERROR_DEVICE_LOST`/TDR after ≥1 presented frame ends PASS, unless the validation layer also reported something. Only teardown_probe catches this today, through its own `budget_left == 0` assert (`teardown_probe/mod.rs:257`). This is the same "booted and exited reads as PASS" class that I3 closed.
- **Confidence:** CONFIRMED.
- **Needed:** witness the loop's exit cause once per run, with the same once-only write as `HostFramesPresented`. `judge_run` must panic on a terminal device exit. Add a fabricated-World row to G9.

### I2. `known` rows keyed on the oracle's panic text hide every validation message in that test
- **Where:** D10 ("panic with the counts"), §5's rule for a new red at a switched-on site, and the §3 grammar for `known`.
- **Problem:**
  - The ledger and the per-context state hold counts only (`debug.rs:89-107`). Message IDs exist only in stderr `[vk-validation] <id>:` lines (`:302-315`).
  - A row keyed on the oracle's text therefore matches any future message in that test.
  - The grammar also sets no floor on the substring: an empty or generic substring (e.g. the UNMARKED or "alive at done()" text) is accepted.
- **Consequence:** once a W6 row exists for a test T, a new VUID introduced in T by any later rung is KNOWN-RED, not FAIL, and UG-12 stays green. G4's "other message → FAIL" arm cannot see it, because the panic text is identical.
- **Confidence:** CONFIRMED.
- **Needed:**
  - Key validation `known` rows on message identity (the set of `pMessageIdName`s, which the runner already captures, or which the per-context state could record), so that a new ID means FAIL.
  - Reject, with exit 2, substrings that are empty or that match the state-machine, UNMARKED or oracle templates.

### I3. Default-leg sites switched on have no known-row channel, and §5 contradicts §8
- **Where:** §5's "on" rows, §5's final paragraph, and §8's rule for who may add a `known` row.
- **Problem:** the sites switched to validation-on include tests that are not ignored:
  - `roundtrip.rs:32/:139/:378` (5 tests)
  - `vg_block_pool_growth.rs:51` (5)
  - `gpu_zone_deadlines.rs:36` (3)
  - `gpu_zone_label_control.rs:68`, `gpu_command_census.rs:68`, `gpu_query_availability_truth.rs:59`
  - `present_mode_probe.rs:40`
  
  None of these files carries `#[ignore]` (grep). They run in UG-01 on every rung (03 §2).
  - `known` rows are read only by `leg run`'s judge. Under plain `cargo test`, `done()` panics.
  - §8 allows a `known` row only for "reds that the owner's legacy-recipe run reproduces on their parent commit". A red that exists only because B5-4 switched validation on is, by construction, not red on the parent. So §8 forbids the very row §5 prescribes.
  - UG-01's posture is not stated anywhere: `tester.md` has no `BOYKO_*` posture.
- **Consequence:** there are two outcomes, depending on the tester's shell.
  - If validation is on, one engine-caused warning in `roundtrip` after C7 makes UG-01 red on the owner box for B5-4 and every later rung.
  - If the shell carries `BOYKO_DISABLE_VALIDATION=1`, the default-leg oracle silently degrades in UG-01.
- **Confidence:** CONFIRMED.
- **Needed:**
  - State UG-01's validation posture.
  - Make §5 and §8 agree.
  - Decide what a new red at a default-leg site does. Options: a site must be clean at the cut or stay off; `done()` consults known rows; or the site becomes a `gpu:` member.

### I4. C0 and C4 are not green on their own, and the lock sets omit shared registry files
- **Where:** §9 C0/C4 and the §8 lock sets.
- **Problem:** three existing checks trip, and none of their files is in B5-0's or B5-2's lock set:
  - `tests/engine_packages_census.rs:133-149` requires every `crates/*` member to be in `ENGINE_PACKAGES` or `USER_PACKAGES`.
  - `tests/production_reachability_census.rs:44-51`, `:260-264` compares unreachable members against a recorded list for exact equality. `boyko_leg` and `boyko_leg_core` are referenced only by dev-dependencies.
  - `scripts/check_hotpath_exceptions.py:46-49`, `:103-108` treats `crates/*/src` as production. Moving `BOOT_LOCK`'s `Mutex` from `tests/` into `boyko_leg/src` needs a `docs/HOT-PATH-EXCEPTIONS.md` row. CI runs this check at `ci.yml:60`, and so does UG-18.
  
  The first two files are also edited by every rung that adds a crate: B3 (`boyko_symcensus`), C1 (`boyko_memory`), RP-2 (`boyko_replay`, `boyko_build_id`). So §8's "Only B5-0 touches shared files" is false.
- **Consequence:** C0 is red in UG-01 on its first run, and C4 is red in the CI hot-path job.
- **Confidence:** CONFIRMED.
- **Needed:** add these files to C0 and C4, to the lock sets, and as §4.3 rows.

### I5. A red member can still leave the leg without a finding ID
- **Where:** D7, rules (a), (d1) and (f), and §8's "only two routes may add a known row".
- **Problem:** two routes bypass the `known`-row discipline.
  - **Reclassing.** (d1) requires every `gpu*` body to open a Leg, but not every Leg body to be `gpu*`. Rule (a) accepts `deferred:`/`slow:`/`solo:`/`flaky:`.
  - **A `timing:` declaration.** Once the file has a member, rule (f) never marks the declaration stale. §3 step 6 then gives the whole file posture off.
- **Consequence:**
  - A rung with a red member writes `#[ignore = "deferred: …"]`. The member drops out of `leg list` and UG-12 is green, with no finding ID and no report line. This is exactly the route W1 rejected.
  - Or the rung adds a `timing:` declaration, and every test in that file becomes a DEGRADED PASS.
- **Confidence:** CONFIRMED from the plan text.
- **Needed:**
  - A Leg body may carry only `gpu*` or `generator` classes, unless it is listed in a shrinking table with a finding ID.
  - `leg-validation: off` declarations go in a pinned table that changes only in owner or document steps.
  - `leg list` reports excluded Leg bodies.

### I6. Between B5-4's merge and C12, new device tests are invisible, and C12 depends on files outside its lock set
- **Where:** §6 (rules (b)–(j) switch on at C12), §8's UG-12 transition, and D7.
- **Problem:** from B5-4's merge, UG-12 is `leg run --only`, and membership means the test body opens a Leg. Rungs cut in that window may add a render or app device test in today's form: `VulkanContext::boot` followed by an early return. Candidates are D-S2, D-S6, D-E18, D-E19 and D-E23, all of which run UG-12 per 03 §2 and touch render or app code. Such a test is not a member, so `leg run --only` does not see it.
- **Consequence:** UG-12 is green while a silent-skip test lands. Later, C12's (h1) and (d) go red on a file outside B5-6's lock set.
- **Confidence:** CONFIRMED from the plan text and the 03 §2 rung list.
- **Needed:** either switch (h1), (d) and (i) on per package at the B5-4 and B5-5 merges, with per-package floors, or add a PENDING table for these stragglers, like `PREFIX_PENDING`.

## Optional
- **O1. Shepherd exit code and leaked processes.** The shepherd has assigned itself to the job, so `TerminateJobObject` kills it too. "Exits 124" must therefore be the `uExitCode` argument; G16 catches a wrong code. Also consider reporting leaked descendants: `ActiveProcesses > 1` when the member exits is a defect, and today it is silently killed.
- **O2. No way to express intermittent conditions.** For `surface-lost` on a workstation, an `expect` row makes every clean run FAIL stale-expectation, and no row makes an occurrence FAIL forbidden-skip. deqp-runner and DRM CI keep a flakes list for this. PLAUSIBLE.
- **O3. Rule (c)'s macro set is undefined.** G5's green control "W3' SKIP semantic" sits inside `assert_eq!` messages (`phase19_drain_panic.rs:284`, `command_queue_panic_recovery.rs:276`). It stays green only if `assert*`/`panic!` are outside the set.
- **O4. Rule (b) cannot link a driver to its worker lexically in every case.** Drivers pass a `const WORKER` or Gate A's `Worker::test_name()` table. The rule needs const resolution or a literal requirement.
- **O5. Probe (iv) runs `windowed_smoke` before step 4 builds it.** Reorder the steps.
- **O6. `g7_validation_reporting` needs rule (f) waivers too:** 3 `InstanceConfig` tokens and the `"BOYKO_DISABLE_VALIDATION"` literal. Only an (h1) waiver is listed.
- **O7. F1 = no needs more `expect` rows.** Besides the corpus drivers, the default-leg payload members need rows: `vg_corpus_ingest :308/:329`, `vg_cull_granularity_census :399`, `vg_glb_decode :300`, `vg_r0d_census :232`, `vb_inst_cull_corpus :462`.
- **O8. Three unprefixed generator sites, not two:** `vb_barrier_stream_baseline.rs:1766`, `:1794`, and `framegraph_gbuffer_equiv.rs:1884` ("generator, not a gate").
- **O9. Pool cost is unstated.** Six L sub-rungs occupy one of the three code-pool worktrees (two of them during B5-4 ∥ B5-5) through phases C and D (02 §4.1). State B5's priority against the MEM, STORE and ENG lanes.
- **O10. G-LOOP scope.** B5-0 adds runner World writes: the boot-failure inserts and the post-loop `HostFramesPresented`. If B2 has merged first, G-LOOP (UG-14) could flag them. `record_teardown_stats` is a precedent for a post-loop write, but G-LOOP's scope should be pinned to frame-loop steps.

## Positive (keep)
- `judge_run` fails closed: with neither or both witnesses it panics.
- Type-forcing through `&Leg` with (h1) and (d2) makes every device boot a member by construction.
- The mode is a pure function of argv/env, and G10 has a mutation for it. `app::run` refuses parallel mode deterministically.
- The contexts-dead rule at `done()` pulls the device-destroy window into the verdict.
- The shepherd assigns itself to the job before spawning, so no descendant escapes.
- Set-fold skip semantics, and UnexpectedImprovement handling for `known` rows.
- The feature-legs derivation (`ci.yml:309-341`) will check `boyko_leg/vk` on Linux automatically.

## Open questions for the architect
1. Is UG-01 run with validation on or off on the gate host (see I3)?
2. Does the oracle, or the runner, record `pMessageIdName` so that known rows can key on it (see I2)?

Relevant files:
- `D:/wt/joltab/crates/boyko_app/src/runner.rs`, `D:/wt/joltab/crates/boyko_app/src/diag.rs`
- `D:/wt/joltab/tests/engine_packages_census.rs`, `D:/wt/joltab/tests/production_reachability_census.rs`
- `D:/wt/joltab/scripts/check_hotpath_exceptions.py`
- `D:/wt/vkval/crates/boyko_rhi_vulkan/src/debug.rs`, `D:/wt/vkval/crates/boyko_rhi_vulkan/src/device.rs`
- `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`, `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-03-GATES.md`