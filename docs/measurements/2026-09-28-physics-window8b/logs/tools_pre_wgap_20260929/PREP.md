# Physics window 8b: prep (2026-09-28, trunk 16191fda). Ready to launch. Nothing timed; no timing number read.
W = C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b
**Launch:** `bash $W/run_window8b.sh --cutoff HH:MM [--resume]`; preview `--dry-run --start HH:MM`; stop: create `$W/STOP`.
Outputs as window 8 (raw/runs.jsonl, progress.txt, raw/counts.txt, WINDOW_DONE 0/2/3/5). **The idle rule cannot pass
while u/phys-s7 builds or runs anything under D:/wt/_targets** (it did throughout this prep; smoke poll: gate/wait_idle_smoke.txt).

## Binaries (bin/SHA256SUMS, bin/COMMIT.txt; rustc 1.98.1 msvc, --locked, lock unchanged, RUSTFLAGS unset)
| key | file | sha256 | source |
|---|---|---|---|
| tip | bin/runner_tip_16191fda.exe | aa34fadff73923bcc69daa3e517bbbf1de693f01659ab86900460f84e0a371a1 | export 16191fda, `--profile parity` |
| parent | bin/runner_parent_16191fda_s4off.exe | 9c6690ac116cba029f8ec810e9d947bf2b1a366031f02a38bcb94d87671fc8a4 | + bin/parent.patch |
| g4ref | bin/broadphase_g4ref_16191fda.exe | b887850f38633d62419344c41080d1bada92bc931c3307989b6f6fb314a33b6d | + bin/g4ref.patch, bench profile |
| dmA/dmB | win8/bin/dm1_A.exe, dm1_B.exe (in place) | 47cb2c9b…, 79bba864… = win8 SHA256SUMS (OK) | cad5439b / 97ee830f; keep D:/wt/_dm1_ab |
- **parent.patch:** `solver/colored.rs` `pub(crate) const SETUP_MAX_TASKS: usize = 32;` -> `= 1;`. **g4ref.patch:** the
  G4_SIZES line -> window_cmds.md's 28 sizes (found by content). Trees are CRLF (autocrlf); a first `sed -i` stripped the
  endings, so both edits were redone byte-exact (tools/patch_trees.py); `diff -rq` vs tip: exactly one file each.

## Protocol (ruling 1; rows8b.json `protocol`)
K 9 = 3 passes x 3 rounds, **p0 reversed, p1 forward, p2 reversed**, warm-up per pass, idle rule per pass, receipts,
busy 5 %, others-busy 2 % re-run once, P-none, --cutoff/--resume, a block that does not fit is skipped whole.
**Placement receipt** (`placement` per process; recorded, never used to drop): top-3 logical CPUs by busy % (window 8's
reading), main-thread CPU s (GetThreadTimes, 15.6-ms ticks: 0 on sub-second runs), main-thread cycle share of the
process (QueryThreadCycleTime, exact), main_share_top_est = top-CPU busy s / main CPU s (exact at W1 idle, upper bound W>=2).
**Own K:** F3-G4 keeps the Q3 recipe (K 3, 1 pass x 3 rounds, no warm-up); DM1 ruling 9's ABBA x 2, 1 pass, as written.

## Blocks (priority order), rows, timed processes, minutes (dryrun.txt)
1. **S4-AB** 153, 25.5 min: S4-JT parent+tip W1/2/4/8/16 (adjacent each round); S4-JT-a parent+tip W8/16; S4-rung tip W8
   (`--canary-frac 1 --canary-ref-ns 60000`); S4-zone-N30000/N60000 tip W8 (armed, phys_solve_build).
2. **SPLIT** 54, 14.2 min: SPLIT-J-T, SPLIT-J-T-a on PARENT, W1/8/16.
3. **F3** 243, 46.8 min (TIP): TD-armed {leaflist, kd} W1/8; TA-armed {both} W1; TR-armed rowwalk W1; F3-JT and F3-RT
   {both} W1/2/4/8/16. R4 pose cmp per (pair, W, round) -> runs.jsonl records `r4` (keys r4_*, never `row`).
4. **F3-G4** 3, 39.6 min: the window_cmds.md regex (60 ids), CRITERION_HOME per process.
5. **J-Son-T** 72, 23.5 min (TIP): JSonT / JSonT-a (cfg a, tree, sleeping on) + JSoffT / JSoffT-a, W1/8; metric
   windows [0,100), [100,500), **[274,500)**.
6. **DM1** 24 + warm-up, 8.1 min: vb-idle, deferred-idle, vb-edit100 @1920x1080, ABBAABBA per row; zones VB_SHADE 2,
   VB_EARLY_CULL 4, VB_RUN 9, VB_PRODUCE_NET 14 / GBUF_DEFERRED_RESOLVE 17.
**Total 157.7 min** + idle waits beyond 135 s/pass. Dry samples were taken under the other lane's load.

## Omitted / unavailable
- **No non-FIFO present mode:** boyko_app/src/host.rs:226 uses `Swapchain::new` (FIFO), no env knob; artifacts read
  `present_mode = "fifo"`, which the driver checks. Cut §5's optional ω(W, gap) row: left to the reserved omega_b v2
  block. DM1 grow frame NOT MEASURED (ruling 9). No listed row dropped; F3-TR-armed (optional) included.

## Untimed gate (gate/, gate_run.log): all passed
- **Fixtures (TIP W1):** JT500 0x30c5438bc6ad9ffa, JToff500 0x32d5e235342b4143, JTA500 0x30c5438bc6ad9ffa, RT500
  0x6cbe24bf8fafda26 (= known); JSonT500 0x3db47fae414b655c, JSoffT500 0x30c5438bc6ad9ffa (recorded).
- **S4 (30/30):** both exes x {J-T, J-T-a, J-T-off} x W1/2/4/8/16: exit 0, pose, expect match, void 0. TIP setup
  steps/tasks 0/0, 500/6000, 500/12000, 500/15980, 500/15980 (= cut.md exactly); PARENT 0/0 at every W. All 143
  non-timing CSV columns equal at W1; at W>=2, net of 14 run-to-run scheduling columns (same-exe control), only
  phys_setup_chunks/_stamped/_stamped_n, phys_route_worker, phys_wave_helped differ (S4's own scope and wave).
- **Red controls 4/4 exit 4** (501 steps vs JT500; J-T-off vs JT500; both exes).
- **Rows 58/58** at their W, 500 steps, --expect-pose: exit, pose, void 0, TreeDiag, canaries, kd_order_builds > 0
  exactly on leaflist-kd, S4 counters by binary. JSonT's sleeper-tree TreeDiag has its own expectation (W1 = W8).
- **F3:** pre-flight cmp silent, kd builds 0 / 20; R4 13/13 equal. **Freeze:** step 274 at W1/W8, armed and disarmed.
  **G4:** `--list` = the 60 expected ids; full dry sample exit 0, 60/60, none extra.
- **DM1:** A and B at 1280x720 x 10 frames exit 0, zones n=10, fifo; 220-frame samples of the 3 rows exit 0, zones n=220.
- **Validators go red under mutation** (tools/selftest8b.py, 20/20): swapped exes, kd on the wrong kernel, default
  TreeDiag on J-Son-T, wrong freeze step, R4 mismatch, missing/extra criterion id, mailbox / missing DM1 zone.
- **Driver --test:** all 6 blocks exit 0, 166 timed + 5 warm-ups, 0 invalid, R4 39/39 (test/rehearsal_full_1); after two
  fixes it found (R4 records counted as processes; DM1 reversed) F3+DM1 again: exit 0, 102 + 2, 0 invalid (test/window);
  every rehearsal process was re-run once (the other lane contaminated them), as the protocol requires.

## Adding a block (reserved S7-AB, omega_b v2): no code change
1. Build from an export, copy to $W/bin, add `sha256 *bin/<file>` to bin/SHA256SUMS and a COMMIT.txt entry.
2. Write `$W/rows8b.extra.json` from rows8b.extra.example.json (binaries; runner `"s4": "on"` if S4 is in the exe;
   rows; a block with a fractional `priority`, e.g. 1.5 = after S4-AB). The driver merges it at start.
3. `python -B tools/gate8b.py rows <ids>` (runner) or `... micro <ids>` (omega: exit, SUMMARY count/fields).
4. `run_window8b.sh --dry-run`, then `--test --blocks <name>` (proved: test/overlay_rehearsal_XTEST, exit 0).
**Disclosure:** grepping win8/analysis/*.txt for window 8's placement method showed a few of window 8's published timing
lines (unused). No commits, no worktree edited. Remove D:/wt/_targets/w8b-* after the window.

## S7-AB and omega-v2 added (2026-09-29)
Untimed; no timing number read (the dry-sample walls feed the schedule only). No commit, no worktree edited or checked
out (D:/wt/merge read only: `git diff --stat`, `git archive`). Every build COLD (D:/wt/_targets had been deleted).
**Provenance (both required, both hold):** `git -C D:/wt/merge diff --stat 54a7714f 16191fda` = CLAUDE.md only (no
crates/ file); `diff --stat 16191fda a3adc827` = the lane's 11 files (omega_b_region.rs, colored.rs, colored_tests.rs,
cpu_topology.rs, solver/mod.rs, alloc_frame_attribution.rs, s7_lane_cap.rs, s7_lane_cap_armed.rs, support/s7_pyramid.rs,
00-RULINGS.md, 01-DESIGN.md), Cargo.lock unchanged; 54a7714f -> 16191fda -> a3adc827 by `merge-base --is-ancestor`.

| key | file | sha256 | source |
|---|---|---|---|
| s7p (P) | bin/runner_tip_16191fda.exe (= key tip, same file, reused) | aa34fadff73923bcc69daa3e517bbbf1de693f01659ab86900460f84e0a371a1 | trunk 16191fda, S4 on, no S7 |
| s7t (T) | bin/runner_s7_tip_a3adc827.exe | c4232eaed89e8afc7edd2bf51fa7504acde772d1ff43a1c0ad9c91f42323ace8 | export a3adc827 -> D:/wt/_targets/w8b-trees/s7tip, `--profile parity` |
| omega2 | bin/omega_b_region_v2_a3adc827.exe | 48e9486ac04a36c0f16426c0b6f3eaa50e782f4ea6951d099df392fb2ba954f4 | same build (`--bench omega_b_region`) |

Build: `tools/build_runner.sh D:/wt/_targets/w8b-trees/s7tip D:/wt/_targets/w8b-s7 $W/logs/build_s7tip.log --bench
omega_b_region` (rustc 1.98.1 msvc, --locked, RUSTFLAGS unset, exit 0; Cargo.lock c00bf4c4... before = after, = 16191fda's).
SHA256SUMS + COMMIT.txt appended; each extra key carries `sha256_pin` (see Tools). `sha256sum -c bin/SHA256SUMS`: 7/7 OK.

**Rows / blocks** (rows8b.extra.json; the driver merges it; `--dry-run` reads "rows8b.json + rows8b.extra.json, 9 blocks"):
- **S7-AB** (prio 1.5, warm-up S7-JT#s7t@W8), cut.md §5 item 3 exactly, disarmed, 500 steps, window 0..500, metric windows
  [0,100) [100,500): S7-JT and S7-JA (s7p, s7t; W 8, 16), S7-ladder8-F{0.5,1,1.5,2} (s7p, W8, `--canary-ref-ns 60000`),
  S7-ladder16-F{0.5,1,1.5,2} (s7p, W16, 105000), S7-JA-ladder16-F1 (s7p, W16), S7-JT-W1 (s7p, s7t; W1, the optional Q10
  pair). JT = mkrows8b's J-T spelling (Tree, TreeDiag {1,1,0}); JA = `--scene jolt --gap 0.5 --cfg a` (AllPairs, as
  window 8's J-A). 19 cells; P and T adjacent at every W in every round (cell_order; checked in dryrun_s7.txt).
- **omega-v2** (prio 2.5, warm-up omega2-worker): the amended section's four command lines (omega2-worker / -external:
  36 SUMMARY each; omega2-gap-worker / -external: P 8, b 4, gap 20,80, 8 SUMMARY each), K 9.
- **omega-v1-cont** (prio 7.5, `passes 1, rounds 3` = K 3 - the driver already supports a per-block K; warm-up
  omega1-s36-worker): cut §5 item 4's optional continuity = window 8's four omega-b rows (participants 2,4,8,16;
  stages 36 / 72; both routes), T's exe.
- **Fixture JA500** added to gate/fixtures (`tools/s7pre8b.py fixture`): recorded by s7p (= tip) at W1, 500 steps, pose
  0x30c5438bc6ad9ffa, byte-equal to window 8's committed JA500.pose (export of a3adc827), sha256 e268d7a5... (= JT500's).

**Tools** (backups of the four edited files: logs/tools_pre_s7_20260929/):
- NEW `tools/micro8b.py`: the micro rules in one place (driver per process, gate per sample, counts8b per cell,
  selftest). Window 8's expect_* checks verbatim + the row's `void_rules`: exactly one CALIBRATION line
  {bench omega_b2, version 2}; `work_ns_calibrated` in [500, 1000] on CALIBRATION and every SUMMARY; expect_fields
  version 2 / bench / route / `exactly_once` true / `lost_wakeups` 0 on every SUMMARY; the park receipt kept per process
  (`micro_notes`). **The gap rule is per CELL:** a park row at gap_us > 0 with parks_per_region_median 0 on EVERY rep
  (process) voids that (row, config); 0 on some reps does not; a gap-0 park row at 0 is reported "no park taken".
  (Reading: parks_per_region_median is already a per-process median over regions, so "every rep" = every process.)
- NEW `tools/s7pre8b.py`: the S7 pre-flight (`fixture`, `run`) and `engagement_why()`.
- `window8b_run.py`: micro validation via micro8b (`validate_micro`); `verify_binaries` also checks `sha256_pin`
  (SHA256SUMS is keyed by path, so a key/exe swap was invisible to it); `--test` now gives omega-b2 `--regions 10` (it
  gave `--reps 10`, which omega-b2 rejects - found by reading, before the rehearsal). `gate8b.py micro`: micro8b rules +
  the one-sample park rule. `counts8b.py`: the per-cell park rule (VOID / no park taken lines). `selftest8b.py`: cases 9-13.

**Pre-flight (gate/s7/: preflight.json, engagement.txt, preflight.log): PASS.** P and T x W 1/2/4/8/16 x {JT, JA}, 500
steps, `--expect-pose` F/JT500 | F/JA500: 20/20 exit 0, pose 0x30c5438bc6ad9ffa, expect match, void 0; `cmp` P vs T 10/10
silent. Armed (`--arm-profiler`, 500 steps, W 8/16): 8/8 exit 0, pose, void 0, S4 setup_tasks > 0. Engagement (w8s:
lanes sum = lanes_mean x waves; bound = 8 x color_scopes + (W+1) x setup_steps; structural counts):

| cfg W | P lanes sum | T lanes sum | bound | scopes / tasks / setup steps / setup tasks |
|---|---|---|---|---|
| JT 16 | 607,272 (ABOVE) | 391,361 (within) | 403,732 | 49,404 / 1,126,176 / 500 / 15,980, P = T |
| JA 16 | 597,418 (ABOVE) | 399,017 (within) | 403,732 | 49,404 / 1,485,480 / 500 / 15,980, P = T |
| JT 8 | 392,023 (within) | 381,099 (within) | 399,732 | P = T (W <= physical: recorded, not gated) |
| JA 8 | 394,544 (within) | 380,996 (within) | 399,732 | P = T |

Physical cores: the engine's own read, s7-omega2/tr1/t_s7_debug.txt:14 (T-S7-4 on a3adc827): "physical 8, logical 16,
source Os" - used for the bound; the runner SUMMARY has only `host.logical_cores` 16 (Q5 not built); independent OS
cross-check (CIM Win32_Processor): 8 cores / 16 logical (gate/s7/host_cores.txt). The lane's tests were not built.

**Gate:** `gate8b.py rows <12 S7 ids>` 19/19 PASS (logs/gate_rows_s7.out); the driver's own `validate_runner` on the same
19 cells: 0 invalid; `verify_binaries` (pins included): all match. `gate8b.py micro <8 ids>` 8/8 PASS (logs/gate_micro.out;
the gap rows parked on every park row; the gap-0 park rows mostly "no park taken", reported). **selftest8b 50/50**
(gate/selftest8b.txt: the prior 20 + 30 new). New red controls: P/T relabelled on the real armed W16 records (JT, JA) ->
engagement red both ways; s7p/s7t exes swapped as the overlay loader would -> verify_binaries red, and GREEN with the
pins removed (the pin is load-bearing); SUMMARY version 1, exactly_once false, lost_wakeups 1, work_ns_calibrated 450
(SUMMARY) / 1200 (CALIBRATION), CALIBRATION missing / version 1 -> red; v1 output under the v2 row -> red; gap > 0 park
rows at 0 on every rep -> VOID (also end to end through counts8b.py), at 0 on 2 of 3 reps -> not void, gap-0 park rows
at 0 on every rep -> "no park taken", not void.

**Schedule (dryrun_s7.txt, all 9 blocks, dry samples taken under the other lanes' load):** S4-AB 25.5, **S7-AB 29.7**
(171 + 3 warm-ups), SPLIT 14.2, **omega-v2 19.7** (36 + 3), F3 46.8, F3-G4 39.6, J-Son-T 23.5, DM1 8.1,
**omega-v1-cont 3.8** (12 + 1) = **TOTAL 211.0 min** (was 157.7) + idle waits beyond 135 s per pass (21 passes).
**Rehearsal:** `--test --blocks S7-AB,omega-v2`: exit 0 complete, 48 processes + 2 warm-ups, 0 invalid, 0 non-zero exits
(each re-run once: other lanes' rustc/link/cargo were running) -> test/rehearsal_s7_omega_20260929/; `--test --blocks
omega-v1-cont`: exit 0, 8 + 1, 0 invalid -> test/rehearsal_omega_v1_cont_20260929/. test/window (the F3+DM1 rehearsal
cited above) was parked during both and restored unchanged. raw/, WINDOW_DONE, progress.txt, wait_log.txt, STOP and
test/window/STOP: absent before and after.

**Launch:** `bash $W/run_window8b.sh --cutoff HH:MM` with **cutoff = launch time + 254 min** (211.0 x 1.2 = 253.2: the
schedule + 20 % for idle waits), e.g. launch 20:00 -> `--cutoff 00:14` (the driver rolls a cutoff earlier than now - 1 h
to the next day). If the cutoff bites, blocks skip whole in priority order (omega-v1-cont is last). Remove after the
window, in addition to the above: D:/wt/_targets/w8b-s7, D:/wt/_targets/w8b-trees/s7tip, D:/wt/_targets/tmp.
