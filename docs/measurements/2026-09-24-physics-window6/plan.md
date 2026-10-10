# Window 6: the plan for the quiet window of 2026-09-24

The quiet window runs from 08:10 MSK. Every timed pass ends by 12:45.

Script: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win6/run_window.sh`.

This file only prepares the window. No timed pass has been run. Every number marked "untimed" below comes from one gate process. The machine was otherwise idle, but this agent was running. **These numbers are not results.**

## 0. How to launch, and what comes out

- **Launch.** Run `bash <win6>/run_window.sh` once, in the background, with no agent working.
  - The orchestrator's own `claude.exe` must stay idle. Measured at 08:53, two `claude` processes used 5.5 s of CPU in 10 s, which put `cpu10` at 5.88 % and failed the idle poll.
  - Keep rust-analyzer and the LSP off. In window 5 they voided a pass.
- **Rehearsal flags.**
  - `--dry-run [--start HH:MM]` prints the schedule and runs nothing.
  - `--test` is an untimed rehearsal under `test/`.
  - `--resume` skips the passes listed in `raw/passes_done.txt`. Use it after a stop or a cut.
- **Checkpoints.**
  - Every process appends one record to `raw/runs.jsonl` as soon as it ends. Its files go to `raw/<block>-p<n>_<runtag>/<seq>_r<k>_<row>_<bin>_W<w>/`.
  - Every pass that finishes appends `'<HH:MM:SS> <item> <row> p<n> done'` to `progress.txt`, and `<block>-p<n>` to `raw/passes_done.txt`.
  - The idle waits log to `wait_log.txt`.
- **At the end.** The script runs the untimed reduction (`tools/reduce6.py` → `raw/tables.md`, `raw/reduction.json`), then writes `WINDOW_DONE`:
  - line 1 is `exit <n>`: 0 complete, 2 cut at 12:45, 3 stopped (idle rule not met in 30 min, a binary changed, or 20 voids), 5 driver crashed;
  - line 2 is the status, for example `cut at 12:45:00 after P5c-C3bG4-p0 r1 C3b-G4#bpb@W1 (next would have been …)`.

## 1. Protocol (every block)

- **Priority order.** Blocks run in the order P1 → P2a → P2b → P3 → P4 → P5a → P5b → P5g → P5d → P5e → P5f → P5c.
- **Passes and rounds.** Each block is 2 passes × 3 rounds, so K = 6 per cell.
  - Pass 0 orders the cells by W ascending. Inside a W group it goes row by row (the order of `rows6.json`), and each row's binaries in their listed order, so the arms alternate inside the W group.
  - Pass 1 is the whole list reversed.
  - Each pass starts with one untimed warm-up process.
- **Idle rule before EVERY pass** (`tools/wait_idle6.ps1`, a copy of window 5's with `MaxPolls` set to 30). All of the following must hold on 3 consecutive polls taken 60 s apart:
  - no process named cargo, rustc, link, lld-link, dxc, clippy-driver, cl, msbuild, miri or cargo-miri, and none of the three cargo bench exe names;
  - no process whose image is under `D:/wt/_targets` or `D:/wt/mq-*`. Every timed exe runs from `win6/bin` on C:, so every such process is foreign;
  - `cpu10` below 5 %.

  The script waits up to 30 minutes. A timeout STOPS the window with exit 3, and `--resume` continues from there.
- **Receipts.**
  - A 10-s receipt opens each pass.
  - A 5-s receipt is taken between processes. The receipt after process i is also the receipt before process i+1, as in windows 3–5.
  - Each receipt records machine CPU, the top-5 processes and a presence snapshot.
  - A process is contaminated if its receipt before or after reads above 5 % busy. A contaminated or invalid process is re-run ONCE at the end of its pass.
- **Void rule.** A build or lane process seen at any receipt, or started during a process, VOIDS the whole pass. The pass is abandoned, the idle rule is re-run, and the pass is re-run from its start. The records stay, marked, and are excluded.
- **Launch.** Every process is launched P-none, as in windows 3–5: suspended, the mask read back, then resumed. No affinity call is made.
- **Validity checks on every process:**
  - exit 0 and `void_steps` 0 (the runner's W4 per-step structure);
  - `--expect-pose` against `gate/fixtures/<ref>.pose` reads `match`, and the pose hash equals the fixture;
  - `workers` equals W, `target_env` is msvc, and the armed flag matches the row;
  - `drops_total` is 0 on armed rows, and there is no disarmed ring traffic;
  - `config.broadphase` equals the row's;
  - TreeDiag on tree rows reads `static_rebuilds` 1, `members` 1, `evictions` 0, and on AllPairs rows every field is 0;
  - the class bench prints its jolt band and SUMMARY;
  - criterion prints an estimate for both J kernels.
- **Statistic.**
  - Per process: the mean of `wall_ns` over the row's window. Armed rows also keep, per zone column, the mean and the median over the window, over `metric_windows` ([100,500)) and over the all-frozen tail [`first_frozen_step`, steps).
  - Per cell: the **median over K**, [min–max], and three spreads relative to the median: r = max − min, i = IQR, and s = SE of the median = 1.2533 · sd / √K.
  - **Claim rule:** B against A is claimed iff |B/A − 1| > 2·√(sA² + sB²) under BOTH r and s. i is printed as well.
- **Hard stop.** No process starts whose estimated end (from the gate's untimed wall) falls after 12:45. The current process always finishes, so a cut loses only the tail.

## 2. Binaries (`bin/`, `bin/SHA256SUMS`, `bin/COMMIT.txt`)

| key | exe | commit | what |
|---|---|---|---|
| b4db | runner_4db26681.exe | 4db26681 | trunk with L9 C0–C3 merged; reuse dormant and switchable (`--contact-reuse`). The L10 lane base |
| c4db | classes_4db26681.exe | 4db26681 | the L9 class bench `narrowphase_classes` (`--bench`) |
| par | runner_6dd1f916.exe | 6dd1f916 | C3b's parent (tree query RowWalk) |
| tip | runner_983480a9.exe | 983480a9 | C3b's tip (tree query LeafList) |
| bpb | bpbench_983480a9.exe | 983480a9 | `benches/broadphase.rs`, criterion, `bench` profile |
| g9p | runner_0ca312bd.exe | 0ca312bd | window 5's parent (L11 C2 + the line merge), copied; sha256 3993684b… |
| g9t | runner_cbd86a65.exe | cbd86a65 | window 5's tip (tree + L11 C0–C3), copied; sha256 9c7caff1… |
| w4bt | runner_w4btip.exe | f8873aae | window 4b's tip (L11 C2), copied; sha256 29dbd993… |

**Builds.**
- Each tree was exported with `git -C D:/wt/lighttable archive <sha> | tar -x` to `win6/trees/<sha>/`, and lighttable's Cargo.lock (LF sha256 5f8de754…) was copied in.
- The builds used `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` (and `--bench narrowphase_classes`), with `CARGO_TARGET_DIR=D:/wt/_targets/win6-<sha>`, cold, 3 in parallel with 5 jobs each. They took 1m22s–1m23s, all exit 0. The criterion bench took 57 s.
- Each log shows `Compiling boyko-physics v0.1.0 (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-…\scratchpad\win6\trees\<sha>\crates\boyko_physics)`, so each build compiled its exported tree.
- **Pruned lock.** Cargo pruned the lock: it removed the `boyko-symcensus` package, which is absent from these trees. The pruned lock has LF sha256 **530cc386380c3b1315bd0158713ed33eec639a5d188a28493b976e3b6c6d9af0**, identical in all three trees.
- Disk: D: had 156 GB free before the builds and 155 GB after.

## 3. Pose gate (untimed, run now): 126 processes, 0 failing

The gate files are `gate/gate_run.log`, `gate/gate_all.json` and `gate/gate_newcells.json`.

- **Base, per runner** (b4db, par, tip, g9p, g9t, w4bt):
  - J `--cfg default`, `--broadphase tree` and `allpairs`, W 1/8, 500 steps → `0x32d5e235342b4143`. w4bt has no `--broadphase` flag, so it ran the flagless default, which also gives 0x32d5….
  - rest `--cfg default`, W 1/8 → `0xee2a67a98434919a`.
  - Result: 36 of 36.
- **L9's C0 fixtures** (`docs/measurements/2026-09-23-l9-contact-reuse/fixtures`, 600 steps) on b4db: J-A, J-D, R, R-S and J-Son at W 1/8, all `expect match`, 10 of 10.
  - J-Son is `0xcc2a5400c66eecce` and R-S is `0x2a2b7926a48aab00`. These are byte-equal to the L10 lane-base fixtures (`b984a2af:docs/measurements/2026-09-23-l10-sleeping/`, whose README says so), so both sleeping-on arms are tied to the lane base.
- **Every timed cell, once, with `--expect-pose`:** 72 of 72 (50 in the first gate run, 22 for the rows added later). These gate runs are also the source of the per-process time estimates. Fixtures (`gate/fixtures/fixtures.json`):

  | ref | hash | rows | note |
  |---|---|---|---|
  | J500 | 0x32d5e235342b4143 | every J 500-step reuse-off row, all binaries, cfg a/as/default, tree/allpairs, W 1–16, canary included | = P0 |
  | JAon500 | 0x30c5438bc6ad9ffa | J-A and J-D with `--contact-reuse on`, W 1/2/4/8/16 | new; one hash across W and across cfg a/default (L9's "one hash per row across W") |
  | Offp-J1000 | 0x3db47fae414b655c | L10 Off′, J, 1000 steps, W 1/8 | new |
  | Son-J1000 | 0xcc2a5400c66eecce | J-Son, 1000 steps, W 1/8 | = the lane-base J-Son fixture (frozen before 600) |
  | R1100 | 0x87e561d20589d4a5 | R `--solver colored`, 1100 steps, g9p/g9t/b4db-off | = window 4b |
  | Ron1100 | 0xc8bbe34cf6a8afc6 | R with reuse on, W 1/8 | new |
  | RS800 | 0x2a2b7926a48aab00 | R-S, 800 steps, g9p/g9t/b4db | = window 4b = the lane base |
  | RSOffp800 | 0xb7f1e9e8f91f75ab | R-S Off′ (tree, reuse on), W 1/8 | new |
  | S16-300 | 0x8877dbb1192e9b92 | S16 cfg a, 300 steps | = window 4b |

- **Red controls.** A 501-step J-A twin run with `--expect-pose J500` exits 4 (`expect mismatch`) on all 6 runners, so the gate can fail.
- **Benches.** The class bench `--bench` and the filtered criterion run both exited 0. The criterion receipt reads LeafList `leaf_list_leaves` 465, `fallback_leaves` 2 on J; RowWalk reads 467.
- **Re-runs.** No mismatch occurred, so no memory-fault re-run was needed.

## 4. Items

### P1 — L10 pre-C0 refutation (block P1-L10, ~8 min)
- **Source.**
  - `docs/physics/perf-campaign/levers/L10-sleeping/08-DESIGN-REV2.3.md:221`: "Take the armed Off′ J-Son span at W=1, K=6 on the lane base. If Off′(1) < 0.584 ms (= 0.6 × 0.59 + 0.23), stop and escalate before C0".
  - `07-REVIEW-OF-REV2.2.md:162` and `09-REVIEW-OF-REV2.3.md:22`.
  - The Off′ definition and table are in `06-DESIGN-REV2.2.md:251-269`.
  - The rulings block is `00-RULINGS.md:252-300`.
  - Lane plan `…/d9f1cf70…/scratchpad/l10/plan.md:23-43`: deviation (1) moves this to the quiet window. `:454` is E8, the recipe.
- **Tree.** The lane base is **4db26681** (trunk). Its runner already has `--sleeping` (bare), `--broadphase`, `--contact-reuse` and `--arm-profiler`. dee30959 (C1a, `--sleeping on|off`) is therefore not needed, and the base is the tree the design names.
- **Rows** (W=1, 1000 steps, window [264,1000), armed; the arms alternate):
  - `L10-Offp` = E8's Off′: `--scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on`. This is the design's Off′ (Tree + L9 with reuse on + L11 + L5).
  - `L10-Son` = the lane base's own J-Son row: `--cfg a --sleeping`, AllPairs, reuse off. It is context, and it is the alternating arm.
- **Stop rule.** The median over K of `L10-Offp`'s per-process wall mean over [264,1000). **< 0.584 ms → STOP L10 and escalate.**
- **Also reported:**
  - the same row over its all-frozen tail [first_frozen_step, 1000). Off′ freezes at step 274, not 264, so the literal window holds 10 awake steps; both readings are printed, and a disagreement is flagged;
  - Σ system spans, and the per-stage bp, np, graph and solve spans and g.
- **Untimed indication.** Off′ tail 1.53–1.66 ms and J-Son 4.87 ms, far above the 0.584 bar.

### P2 — L9 C0 refutation → C4 decision (blocks P2a-L9classes, ~7 min, and P2b-L9reuse, ~13 min)
- **Source.**
  - `levers/L9-contact-reuse/02-DESIGN-REV1.md:380`: "if the measured costs predict Δt_np(1) < 1.0 ms, stop and escalate".
  - `:403` G-L9b-6 (h_J(1 mm) = 0.9897 ≥ 0.7, already passed; `…/scratchpad/l9/h_tau.md:3`).
  - `:407-410` G-TW: "L9b: same binary …, `--contact-reuse off` against `on`, on J-A, J-D and R, at every W", and "Realized-gain rule: ΔT(1) on J-A ≥ 0.6 × the prediction from the bench and the measured h".
  - `…/scratchpad/l9/c3.md:171-173` lists what is owed before C4.
  - The bench formula is `benches/narrowphase_classes.rs:746-755` (tree 4db26681). Low = N_sep·(t_sep − 60) + 0.80·N_touch·(t_touch − 100) − 0.06 ms; high = N_sep·(t_sep − 35) + 0.97·N_touch·(t_touch − 60) − 0.03 ms.
- **Binaries.** `classes_4db26681.exe --bench`, and the trunk runner 4db26681 (reuse switchable in the same binary).
- **Rows.**
  - `L9-classes`: K = 6 processes, each with its own 51-rep medians.
  - `L9-JA-off` / `L9-JA-on` (`--cfg a`): W 1/8, 500 steps. The reducer reads both [0,500) and [100,500).
  - `L9-JA-a-off` / `L9-JA-a-on`: armed twins at W=1. They measure Δt_np(1) directly as the narrowphase system span, off − on, over [100,500).
- **Stop rules.**
  - (a) Class-bench median band:
    - FIRES (STOP C4) iff the band's high end < 1.0 ms;
    - AMBIGUOUS iff low < 1.0 ≤ high (escalate with the numbers);
    - CLEAR iff low ≥ 1.0.
  - (b) Realized-gain rule: ΔT(1)[100,500) on J-A (off − on, claimed) ≥ 0.6 × the lower end of the L9b-only prediction. That prediction is h_meas (0.9897) · N_touch (4515) · (t_touch − t_hit), with t_hit ∈ [60,100] ns and t_touch measured by the class bench.
- **Caveat.** On 4db26681 the bench's `separated` class is already post-L9a (C1/C2 are merged), so the bench's N_sep term prices mostly nothing new. The same-binary A/B isolates L9b (reuse) alone: L9a and the carry run in both arms.
- **Untimed indication.**
  - The bench band was [1.094, 1.686] in one process and [1.079, 1.669] in another. **Its low end sits within 0.1 ms of the 1.0 bar**, so this reading could come out AMBIGUOUS.
  - The J-A untimed means were off 18.9 vs on 17.0 ms at W1.

### P3 — tree broadphase C3b query cost (block P3-C3b, ~13 min)
- **Source.**
  - `…/scratchpad/c3b/design.md:215-226` ("The quiet window (K = 6 …)"), `:365-376` (actions per outcome) and `:253` (no `--bp-kernel` in the runner, so two binaries are used).
  - `…/scratchpad/treebp/g4_g5_recipe.md:157` (the `T-A-tree-armed` spelling) and `:208` (tree span limits 0.36 / 0.35 ms).
  - The C3b commit note: c_q 334 → 133–135 ns.
- **Binaries.** par 6dd1f916 against tip 983480a9, interleaved inside each W group.
- **Rows.**
  - `C3b-TA-armed` = `T-A-tree-armed` (`--cfg a --broadphase tree --arm-profiler`), W 1/8.
  - `C3b-TD-tree` = J default tree (`--cfg default --broadphase tree`), W 1/8.
- **Statistics.**
  - t_q = the per-process median over [100,500) of `phys_bp_query_ns`, then the median over K.
  - The tree span = Σ of the four `phys_bp_*` per-step medians.
  - T is the wall.
- **Decision rules (design `:367-376`):**
  - tip/par t_q < 1 → the kernel ships; ≥ 1 → it is rejected;
  - ratio > 0.55 or t_q(J,W=1) > 0.21 ms → attribution A is refuted;
  - t_q ≥ 0.235 → C2 is built;
  - t_q ≤ 0.186, claimed → "into the band";
  - c_q > 150 ns → F3;
  - the tree span must be ≤ 0.36 / 0.35 ms at W 1/8, else stop.

### P4 — L11 G9 remainder (block P4-G9, ~30 min)
- **Source.**
  - `docs/measurements/2026-09-23-combined-tree-l11/analysis.md:227` (window 5 §4.5): "C3's other G9 rows were not run: J-A (cfg-A 'not claimed slower'), R, R-S, S16, W 2/4/16, K=12, and the canary".
  - `levers/L11-solve-setup/02-DESIGN-REV1.md:359-371` (G9).
  - The row spellings are window 4b's `rows.json`. The `win5/analysis.md` path given in the task does not exist; the analysis is in the docs file named above.
- **Binaries.** Window 5's own: g9p (0ca312bd) against g9t (cbd86a65), both copied with their sha256 re-checked.
- **Rows.**
  - `G9-JA` (`--cfg a`), W 1/8.
  - `G9-R` (`--scene rest --solver colored`, 1100 steps [600,1100), armed), W 1/8.
  - `G9-RS` (`--scene rest --sleeping`, 800 steps [300,800), armed), W 1/8.
  - `G9-S16` (`--scene s16 --cfg a`, 300 steps, armed), W1.
  - `G9-JAs-mid` (`--cfg as`), W 2/4/16.
  - `G9-JAs-a` (armed, W1): the canary's reference.
  - `G9-JC`: the canary, `--canary-frac 0.05 --canary-ref-ns` set to the latest valid `G9-JAs-a` window mean of the same binary and W. This is P0's recipe (`driver.py:427`).
- **Rules.**
  - "Not claimed slower" on every row, tip against parent.
  - Per-stage solve_build, warm_apply, store and wide colours on the armed rows.
  - The canary must be SEEN: its span within 5 % of the injected value, and the step rise claimed.
- **Untimed note.** In one gate process the canary span read 0.450 ms of 0.450 injected, but J-C's wall (9.427) did not visibly exceed J-As-a's (9.420) at W1. The window's K=6 comparison decides whether the canary is seen.

### P5 — extras, all in the script (budget allows)
- **P5a bridge 2** (~9 min).
  - Source: `analysis.md:234`, "Interleave window 4b's runner_tip.exe (29dbd993…) with this window's runner_parent.exe on J-As at W=8, K=6 each, with a W=1 control".
  - Rows: `B2-JAs` w4bt against g9p, W 1/8.
- **P5b L10 armed Off′ spans** (~11 min).
  - Source: `06-DESIGN-REV2.2.md:309`, "Record the armed Off′ spans for J-Son and R-S at W ∈ {1, 8}, K=6, to feed the R gate"; also deviation (1) item 2.
  - Rows: `L10-Offp8` and `L10-Son8` (W8); `L10-RS-Offp` (rest, sleeping, tree, reuse on) and `L10-RS` (R-S), W 1/8, 800 steps [300,800).
- **P5g C3b G5 on the tip, same binary** (~13 min).
  - Source: `c3b/design.md:223-224` (the G5 T-A-allpairs-armed Δbp and the T-D bridge rows).
  - Rows: tree against allpairs, armed `--cfg a` and disarmed `--cfg default`, W 1/8. This gives Δbp(W) = the allpairs − tree broadphase span; window 4's bar at W8 is ≥ 1.03 ms.
- **P5d L9 J-D reuse off/on, W 1/8** (~9 min); source `02-DESIGN-REV1.md:409`.
- **P5e L9 R reuse off/on, W 1/8, 1100 steps [600,1100)** (~11 min); same source. R's pose moves under reuse, fixture Ron1100.
- **P5f L9 J-A reuse off/on, W 2/4/16** (~11 min); same source, "at every W".
- **P5c C3b G4 criterion, same binary** (~42 min, last because it is the longest).
  - Source: `c3b/design.md:219-220`.
  - Filter: `bp_g4_scene/{tree,tree_rowwalk}/{j100,1240}` and `bp_g4_{uniform,disparity}/{tree,tree_rowwalk}/1000`.
  - One process takes ~271 s: the bench builds every G4 family, the 100k scenes included, before it filters.
  - Rules:
    - LeafList/RowWalk on J above 0.55 → attribution A is refuted;
    - a uniform or disparity ratio outside [0.30, 0.60] → the model is structurally wrong;
    - a ratio ≥ 1 → the kernel is rejected.
  - Untimed indication: j100 0.147/0.444 = 0.33, uniform 0.53, disparity 0.45.

### Left out, and why
- **The unified plan's MQ list** (`UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md:561-562`: MQ-13, MQ-03, MQ-18, MQ-21, MQ-22, MQ-23). Each hangs off a rung that is not built (D-M6, D-S2, D-E0, D-E20, RP-0, RP-2), so none can run on these binaries.
- **UG-21** (`UNIFIED-SYSTEM-PLAN-03-GATES.md:30`: "receipts carry the idle-machine line and ancestry"). It is satisfied by construction: every record carries its receipts, the idle log, the binary's commit and its sha256.
- **K=12 for G9** (L11 design `:360`). The window protocol is K = 6; every G9 cell here is K = 6.
- **G9 at W 2/4/16 for J-A** (the design's full W list covers J-A too). Only J-As is extended to 2/4/16.
- **The tree's `AUTO_TREE_LO/HI` / `TREE_BRUTE_MAX_ROWS` refinement run between 64 and 256** (`MEASUREMENT-QUEUE.md:1147`, "owed … after F1"). The rows and the rule are not pinned in a source read here, and it would add ~2 min per criterion process.
- **G4 at 10k/100k (K = 3).** Only the untimed fallback counts were asked first, and they are in the gate receipt.
- **The W=1 determinism event (window 4 §9.1), and anything needing another build:**
  - L10 C1b and L9 C4 are not built;
  - `MEASUREMENT-QUEUE.md` §7 R4 is an allocation census, not a timed row;
  - Jolt is never re-run (owner ruling).

## 5. Estimated duration (dry run; the table assumes a 09:05 start, and `dryrun.txt` was re-run with a 09:12 start, which ends 12:06)

Per process: the gate's untimed wall, plus 0.4 s launch, plus a 5-s receipt. Per pass: at least 135 s of idle rule, a 10-s opening receipt and one warm-up. Re-runs and voids are not included.

| block | item | cells | timed processes | minutes | ends |
|---|---|---|---|---|---|
| P1-L10 | P1 | 2 | 12 | 7.8 | 09:12 |
| P2a-L9classes | P2 | 1 | 6 | 6.5 | 09:19 |
| P2b-L9reuse | P2 | 6 | 36 | 12.6 | 09:31 |
| P3-C3b | P3 | 8 | 48 | 12.7 | 09:44 |
| P4-G9 | P4 | 24 | 144 | 30.4 | 10:14 |
| P5a-bridge2 | P5 | 4 | 24 | 8.7 | 10:23 |
| P5b-L10spans | P5 | 6 | 36 | 10.7 | 10:34 |
| P5g-C3bG5 | P5 | 8 | 48 | 12.9 | 10:47 |
| P5d-L9reuseJD | P5 | 4 | 24 | 8.5 | 10:55 |
| P5e-L9reuseR | P5 | 4 | 24 | 10.8 | 11:06 |
| P5f-L9reuseJAmid | P5 | 6 | 36 | 10.9 | 11:17 |
| P5c-C3bG4 | P5 | 1 | 6 | 41.7 | 11:59 |
| **total** | | 74 | 444 | **174.2** | **11:59** |

- **Slack.** 46 min before the 12:45 cutoff for a 09:05 start, or 39 min for a 09:12 start, for re-runs, voids and idle waits longer than 3 polls.
- **P1–P4 alone** take 70 min, ending 10:14.
- The full dry run is in `dryrun.txt`.

## 6. Control-flow tests (untimed, `test/window/`)

- **Full rehearsal** of every block except P5c: exit 0 `complete`, and 31 progress lines.
- **Cut path.** A run with `--test-cutoff-s 25` exited 2 with `cut at … after P2a-L9classes-p0 r0 L9-classes#c4db@W1 (next would have been P2b-L9reuse-p0 r0 …)`. The `--resume` run then skipped `P1-L10-p0`, which was already done, re-ran the cut pass into a new run-tag directory without overwriting the old one, and exited 0.
- **Reducer.** It ran on each of these rehearsals with exit 0.
- **Criterion block (P5c).** The driver-level rehearsal (08:54–09:08) ended exit 0 `complete`.
  - The warm-up, the original and the re-run each parsed all 8 estimates, with a j100 LeafList/RowWalk ratio of 0.331–0.333.
  - The original and the re-run were both marked contaminated (receipts at 5.98 %, from this agent working), so the reducer dropped the slot, as the selection rule requires.
