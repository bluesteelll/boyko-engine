# Physics perf campaign, window 9a — tree C4's merge gate with Jolt 5.6 in-block, the Rapier 0.36 comparison, the G5 broadphase profile; and the 2026-10-01 resume: codegen-units, the G4 threshold re-read, the all_pairs bracket (2026-09-30)

This directory holds the receipts for:
- `docs/MEASUREMENT-QUEUE.md` section 18, the `RESULT, 2026-09-30, window 9a` block and its resume;
- the rulings the window's questions produced. They are **not re-recorded here**; they are in
  `docs/physics/perf-campaign/levers/00-RULINGS.md`:
  - section "After window 9a (2026-09-30)": the V2 rulings of 2026-09-30 (from the orchestrator's
    `RULINGS-2026-09-30-V2.md`) and the owner's answers and orchestrator's decisions of 2026-10-01, rulings 1–5
    (from `RULINGS-2026-10-01.md`), ported by trunk commit `82b3867f`;
  - section "After the window 9a resume (2026-10-01)": rulings 6–10 of `RULINGS-2026-10-01.md` (the build profile, the
    thresholds, F3, C4-BR, window 9b's list), added with this record.

  Both rulings files are in the orchestrator's scratch directory, not in this tree.

**One window, two runs.** The window was launched twice under one preparation (`prep.md`):
- **the first run**, 2026-09-30, `--cutoff 00:26`: 19:13:54–22:16:27, run tag `191354`. It closed three blocks, aborted in
  C4-BR's only pass, and ended with exit 3 (`WINDOW_DONE.run1`);
- **the resume**, 2026-10-01, `--resume --cutoff 07:03`: 03:37:40–05:48:56, run tag `033740`. It ran C4-CGU (a block added
  between the two runs), re-ran C4-BR whole, and ran C4-G4 and C4-G4-kd. `WINDOW_DONE` reads `exit 0` / `complete`.

The raw directories carry the run tag (`raw/<block>-p<n>_191354/`, `raw/<block>-p<n>_033740/`).

**Read `analysis.md` first.** It is the results-analyst's synthesis, recorded verbatim, and it holds both parts: sections 1–5
cover the first run, and the section "Resume 2026-10-01" (R1–R7) covers the resume. The group scripts, tables and
independent re-derivations are in `analysis/` (`c4ab/`, `c4ab-jolt/`, `rapier/`, `g5/` for the first run; `cgu/`, `g4/`,
`br/`, `resume/` for the resume; `synth/` for the window-wide figures; each group's `verify/` holds the check that agreed with
it). The window was launched from the tester's prep report, recorded verbatim as `prep.md`; its first lines say what the
2026-10-01 addendum changed, and its last section is the orchestrator's pre-registration addendum.

**Three notes on `analysis.md` as recorded:**
- Its first lines carry an "Operational, now" block: D: held 104 MB at 22:59 on 2026-09-30 and no build could run
  (PC-9a-0). It is a first-run note and is overtaken: the resume's idle polls read 107.79 GB free at every poll.
- The first-run section says the C4-BR abort was the idle rule never coming. **The cause was the disk guard**
  (`STOP: D: free 0.1 GB < 2 GB` at 22:16:27, a download on D:), as the resume section and the addendum at the end of
  `prep.md` correct. `WINDOW_DONE.run1` still reads the driver's own message, `machine never idle (wait_idle exit 3)`.
- Sections 1–5 say that C4-BR, C4-G4 and C4-G4-kd were never run or NOT MEASURED. They ran in the resume; R1–R4 hold their results.

**How to read the references in `analysis.md`, `prep.md` and the group reports.**
- **Trunk state.** At the time of recording the trunk `integ/unified` is `82b3867f`. The tip the window measured, `50e31f1a`
  (`u/phys-tree-c4`), is on the trunk, merged by `d6521a43`; the parent `3d9433ae` is on the trunk. The lane record the tip
  carries is `docs/physics/perf-campaign/levers/broadphase/07-C4-RECORD.md`.
- "ruling 1" is the claim rule, and "ruling 8" (the re-run waves, a K < 3 pass-cell does not gate) is ruling 8 of the section
  "After window 8b" of `00-RULINGS.md`. "Ruling (a)", "(b)" and "(c)" are the orchestrator's rulings during the Rapier
  preparation, recorded in `prep.md` ("Rapier block"). "Owner ruling 2026-10-01 #N" is ruling N of the section "After window
  9a (2026-09-30)", for N = 1–5, and of "After the window 9a resume (2026-10-01)" for N = 6–10.
- "window 8b" means `docs/measurements/2026-09-28-physics-window8b/`, and "window 7" means
  `docs/measurements/2026-09-25-physics-window7/`.
- **Named inputs that are not in this tree**, all in the orchestrator's scratch directory: the V2 lane's `v2-spec/`
  documents (`window9_v2.md`, `impl.md`) and the Jolt-gap plan (`PLAN.md` rev 3, the F0 verdict `f0/VERDICT.md`, F0e, F0g);
  the lane's cut for tree C4 (M9's finding is summarised in section 3 of `07-C4-RECORD.md`) and its `window9_c4_rows.json`; the
  G4/G5 recipe `treebp/g4_g5_recipe.md`; the
  census "DOSSIER" the analysis uses for Jolt's work counts (565,790 velocity rows + 62,224 position rows, 8,489 manifolds).
  `gate/rapier/window9a_rows.md` (also `rapier-harness/gate/window9a_rows.md`) defines the Rapier rows and the void rules
  V1–V9, and is in the tree.
- **The Jolt 5.6 row.** The patch Jolt's `PerformanceTest` is built with is already in the tree:
  `crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch` (its build recipe is the header of
  `crates/boyko_physics/benches/jolt_parity_pyramid.rs`; the P0 build is recorded in
  `docs/measurements/2026-09-19-physics-p0/p0/build_report.md`). Every Jolt process prints
  `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0`.
- **The Rapier harness lives outside the repository** (`D:/tmp/rapier-parity`) and is recorded in `rapier-harness/` (below).

## Headline

`analysis.md` § 1 and R1, restated with its numbers. The scene is the J-T pyramid (1,240 boxes, 500 steps), measured in
the steady state [100,500) on the shipped default after C4 (the Tree broadphase), ours = `C4-JD#tip`; K = 9 per cell.
- **Against Jolt 5.6: ours is faster at every W in medians** (NOT CLAIMED by the letter; post hoc STRONG at W1–W4 on the wall
  and W1–W8 per row), on the wall and on equal work (in-block, first run). Ours/Jolt on the wall
  is 0.435 / 0.506 / 0.586 / 0.696 / 0.818 at W 1/2/4/8/16; per velocity row-iteration, the equal-work unit (Jolt does 0.949×
  our rows), 0.413 / 0.480 / 0.556 / 0.660 / 0.776. Per manifold Jolt is cheaper from W4 up (1.11 / 1.32 / 1.55×), which
  the plan rates an unfair unit: Jolt carries 1.90× our manifolds.
- **Against Rapier's defaults: only at low W.** With its faster build (simd8), ours/Rapier is 0.884 / 0.987 / 1.073 / 1.266 /
  1.298. On equal work (Rapier matched to our configuration, 1.80× our rows) Rapier is ahead from W2: per row 1.013 at W1 and
  1.18–1.32 at W2–W16.
- **Almost none of this is claimed by the letter.** C4-AB's pass 0 was contaminated: 30 of its 34 pass-cells have K < 3 (30
  of 102 in all), and ruling 8 lets no short pass-cell gate. The only claim is Jolt's per-manifold lead at W16 (STRONG).
  In the two clean passes (post hoc), ours is faster than Jolt STRONG at W1–W4 on the wall.
- **Fidelity caveat.** Every "ours faster" is at unequal fidelity: the pre-V2 pile deforms (26 boxes past 0.5 m, 2.18 m
  maximum drift) where Jolt and Rapier hold theirs. V2 is ruled in and nearly doubles our contact work (points ×1.86;
  manifolds 8,496–8,501 against Jolt's 8,489), so the lead over Jolt at W ≥ 8 is not safe at equal fidelity.
- **C4 merges by its own rule (M7).** All 10 no-slower comparisons HOLD. In medians J-D is 1.58–1.68 ms below C4-JDap (the
  tip's own AllPairs) and 1.61–1.69 ms below the parent's default (C4-JDpar) at every W, 93–100 % of the plan's −1.70 ms
  ("J-D claimed faster" is NOT CLAIMED by the letter; CLAIMED STRONG ×9 post hoc). G5 is recorded: B1–B3 STRONG, B4 and B5 hold
  (B1 0.2502 ms at W1 against 0.36, 0.2608 ms at W8 against 0.35; B2 +1.5779 ms against 1.03).
- **The resume.** C4-CGU: the profile stays at codegen-units 16 (cgu1/cgu16 within ±1.05 % at every W, nothing claimed).
  C4-G4: the thresholds are **MEASURED** at 128/136 in both families, and window 7's 144/152 is REFUTED. C4-G4-kd: F3
  (`LeafListKd`) is FREEZE-AND-REMOVE. C4-BR: the all_pairs slowdown since window 7 is a binary (placement) term, about
  1.12, so no fix lane opens.
- **A post hoc caution.** The same tip exe with the same args ran about 5 % slower at W ≥ 8 in the resume than in the first run
  (W8 +5.62 %, W16 +5.32 %), and no receipt catches it. Every A/B must stay in-block, and 9a's T(8) = 1.7878 ms is not a
  cross-run baseline.

## What was measured

Seven blocks in the window's priority order (`rows9a.json`, `rows9a.extra.json`, `prep.md`). Every block runs K = 9 (three
passes × three rounds; passes 0 and 2 reversed, pass 1 forward) except where noted. Counts are from `raw/runs.jsonl`.

| block | question | binaries | passes | timed (of which re-runs) | span |
|---|---|---|---|---|---|
| C4-AB | M7: the Tree default against AllPairs (same binary and the parent's default), the controls, the rung; **Jolt 5.6 in-block** | tip, parent, j56 | 3 (first run) | 379 (73) | 19:16–20:43 |
| C4-RAPIER | Rapier 0.36's two builds and two configurations at W 1/2/4/8/16 | rs8, rs4 | 3 (first run) | 214 (34) | 20:45–21:22 |
| C4-G5 | the broadphase profile: B1–B5 and the S5 input | tip | 3 (first run) | 95 (5) | 21:24–21:40 |
| C4-BR | the all_pairs bracket across three binaries, **K = 3, diagnostic only** | g4r7, g4r8b, g4rT | 1 (aborted in the first run) | 8 in the first run (7 completed, no `pass_done`); 9 in the resume | first run 21:43–22:02; resume 04:00–04:26 |
| C4-CGU | codegen-units 1 against 16 on the tip's default | tip, tipcgu1 | 3 (resume) | 113 (23) | 03:40–03:58 |
| C4-G4 | the threshold re-read: criterion, sizes 96–160 | g4rT | 3 (resume) | 9 (0) | 04:28–05:38 |
| C4-G4-kd | F3's keep or freeze: `tree_kd` at 96/112/128, **its own K = 3** | g4rT | 1 (resume) | 3 (0) | 05:41–05:48 |

The first run is **696 timed processes** (379 + 214 + 95 + 8) and 12 warm-ups; the resume is **134** (113 + 9 + 9 + 3) and 6
warm-ups; the window is 830 timed processes. The 8 first-run C4-BR records have no `pass_done` and are excluded from the
analysis; `--resume` re-ran that pass whole.

| row | block | args (+ `--workers W --steps 500 --window 0..500 --csv --pose-out --label --expect-pose gate/fixtures/<ref>.pose`; armed rows `--arm-profiler`; Jolt `-t=W -i=500`) | W | pose ref |
|---|---|---|---|---|
| `C4-JD` | C4-AB | tip: `--scene jolt --gap 0.5 --cfg default --sleeping off` (the Tree default) | 1/2/4/8/16 | JT500 |
| `C4-JDap` | C4-AB | tip: the same + `--broadphase allpairs` | 1/2/4/8/16 | JT500 |
| `C4-JDpar` | C4-AB | parent: the same as `C4-JD` (the literal parent default, AllPairs) | 1/2/4/8/16 | JT500 |
| `C4-JA` | C4-AB | parent, tip: `--scene jolt --gap 0.5 --cfg a` (control) | 1/8/16 | JA500 |
| `C4-JT` | C4-AB | parent, tip: `--cfg default --broadphase tree --sleeping off` (control; an A/A twin of J-D on the tip) | 1/8/16 | JT500 |
| `C4-jolt56` | C4-AB | j56: `-s=Pyramid -q=Discrete -f` | 1/2/4/8/16 | hash `0xb8522b4e3fc62cfe` |
| `C4-rung` | C4-AB | tip: as `C4-JD`, `--canary-frac 1 --canary-ref-ns 60000` | 8/16 | JT500 |
| `RP-D` / `RP-M` | C4-RAPIER | rs8, rs4: `--cfg rapier-default` / `--cfg matched`, `--install loop` | 1/2/4/8/16 | RD8, RD4 / RM8, RM4 |
| `C4-JD-armed` / `C4-JDap-armed` | C4-G5 | tip: as `C4-JD` / `C4-JDap`, armed | 1/8 | JT500 |
| `C4-R` / `C4-Rap` | C4-G5 | tip: `--scene rest --cfg default --sleeping off`, and + `--broadphase allpairs` | 1/8 | RT500 |
| `C4-S16` / `C4-S16ap` | C4-G5 | tip: `--scene s16 --cfg default --sleeping off`, and + `--broadphase allpairs` | 1 | S16500 |
| `C4-JDcgu` | C4-CGU | tip (cgu 16) and tipcgu1 (cgu 1): as `C4-JD` | 1/2/4/8/16 | JT500 |
| `C4-BR` | C4-BR | `--bench --noplot ^bp_g4_(uniform\|disparity)/(all_pairs\|tree)/(144\|256)$` (8 ids), `CRITERION_HOME` per process | — | — |
| `C4-G4` | C4-G4 | `^bp_g4_(uniform\|disparity)/(all_pairs\|tree)/(96\|104\|…\|160)$` (36 ids) | — | — |
| `C4-G4-kd` | C4-G4-kd | `^bp_g4_(uniform\|disparity)/tree_kd/(96\|112\|128)$` (6 ids) | — | — |

The forward order inside a block is W ascending and, inside a W group, row by row with each row's binaries in order, so the
binaries of an A/B row are adjacent in every round. In C4-AB the Jolt cell sits beside the tip's J-D and J-T cells in every
round; C4-RAPIER's four configurations are adjacent within each W. **Rapier's rows are not adjacent to ours**, because the
blocks run one after another: the adjacency the Rapier rows file asked for was waived by ruling (a), so every Rapier
comparison is cross-block.

### The block added between the runs, and the ruled-in deviations (`prep.md`)

- **C4-CGU** (priority 0.5, so it runs first on `--resume`) was added on 2026-10-01 after owner ruling 2026-10-01 #3
  ("codegen-units = 1 is acceptable for the shipped profile if it measures faster"). Its **pre-registered claim**, written in
  `prep.md` and in the block and row notes of `rows9a.extra.json` (copied into `raw/manifest_1790815060.json` at launch,
  before any C4-CGU process was timed): "cgu1 claimed faster than cgu16 at W8 AND not claimed slower at any W -> the
  shipped/parity profile moves to cgu 1". The claim sentence names no window; the **primary window [100,500)** was fixed in
  `prep.md`'s orchestrator addendum, whose mtime (03:20:47) is 19 minutes before the first C4-CGU process (03:40:01). Both
  windows give the same verdicts.
- **C4-BR at K = 3 and diagnostic only** (ruling (b)); the letter of ruling 1 (K = 9) is `tools/mkrows9a.py --br-k9`
  (the unused `alt/` schedule is not committed).
- **C4-RAPIER** was added on 2026-09-30 after the first cut of the preparation (rulings (a), (b), (c)).
- **Jolt was folded into C4-AB** (W 1/2/4/8/16), not a block of its own. `J-D` stands in for `J-T` at W2/W4 (an A/A twin,
  at most 1.52 % apart). Rapier's rows census comes from the armed J-D rather than J-T-a (the same contact set).
- `C4-S16` / `C4-S16ap` carry the pose ref S16500 and expect `static_rebuilds 0` / `members 0` (16 rows are at or below the
  brute-force threshold). The lane's file had `null` for both; each was found during the preparation and fixed.

## Protocol

Window 8b's (`docs/measurements/2026-09-28-physics-window8b/README.md`), **with ruling 8 of window 8b's rulings implemented in
the driver** (`tools/window9a_run.py`).
- **Cell and claim rule.** The median over K = 9 separate processes of each process's statistic (runner and Rapier: the mean
  of `wall_ns` over the window; Jolt: `Time (ms)`). Ruling 1: CLAIMED iff i (IQR) AND s (SE) flag, pooled and in every pass,
  with one sign throughout; STRONG iff r (min–max) also flags. A pass-cell with K < 3 sets no flag.
- **Re-run waves.** A slot is unclean when its latest attempt has a receipt before or after above 5 %, an others-busy witness
  above 2 %, a build process, or is invalid. At the end of a pass the driver re-runs the unclean slots in waves, until every
  pass-cell has K = 3 clean slots or the pass ends (4 waves, or the per-pass wall allowance, or the cutoff or STOP flag).
  The slot rule: the original if clean and valid, else the **first** clean valid re-run, else the slot is dropped.
  - The allowance is `max(0.5 × the pass's own timed wall, 4 waves × 1.3 × the block's longest process)`. The first cut had
    the fraction alone, which on a one-cell criterion block allowed one re-run; the pre-launch audit found it (W1) and fixed it.
  - A pass cut inside its re-run stage writes no `pass_done` and no pass-cell records, and `--resume` re-runs it whole.
    The analysis uses passes with a `pass_done` record only.
- **Idle rule before every pass** (`tools/wait_idle9a.ps1`: three consecutive 60-s polls with no build process, no process under
  `D:/wt/_targets` or `D:/wt/mq-*`, a 10-s CPU below 5 %; up to 30 polls; a timeout stops the window, exit 3), plus a free-space
  guard on D: (below 2 GB stops the wait). Receipts: 10 s before a pass, 5 s between processes.
- **Placement** P-none; the placement receipt is recorded, never used. **Own K:** C4-G4-kd 1 pass × 3 rounds (not gating);
  C4-BR 1 pass × 3 rounds (diagnostic, ruling (b)).
- **Rapier's validator** (`validate_rapier`, rules V1–V9 of `gate/rapier/window9a_rows.md`): exit, one parsable SUMMARY, pool
  receipts equal W, the build receipt (including the sha256 of the file that ran), the config pin, the scene identity, the
  final pose bytes against the arm's fixture, the CSV shape, and no `--receipt`. A Rapier process alive past 300 s is a hang.
- **Criterion blocks** run one criterion process per (round, exe) with its own `CRITERION_HOME`; every expected id must
  appear on stdout and on disk, none extra.

## When and where

The owner's workstation (8 physical / 16 logical cores, windows-msvc). The power scheme read High performance
(`raw/window_state.json`). Every build and the whole untimed gate came before the first launch; the C4-CGU build and its gate
came before the resume.

| run | launch | timed | ended | `WINDOW_DONE` |
|---|---|---|---|---|
| first | 2026-09-30 19:13:54 (`bash run_window9a.sh --cutoff 00:26`) | 19:16:15–22:02:05 | 22:16:27 | `WINDOW_DONE.run1`: `exit 3`, `ABORT in C4-BR-p0` |
| resume | 2026-10-01 03:37:40 (`bash run_window9a.sh --resume --cutoff 07:03`) | 03:40:01–05:48:51 | 05:48:56 | `WINDOW_DONE`: `exit 0` / `complete` / `counts exit 0` |

- **First run:** 182.6 min. The owner set the STOP flag at 22:03:29; it took effect only at 22:16:27, because the driver
  checks it between processes, not inside the quiet wait or the idle rule (PC-9a-5).
- **Resume:** 131.3 min, against 171.4 estimated. The cutoff was not reached. `raw/window_state.json` is the resume's:
  `"timed": 134`, `"rerun": 23`, `"warm": 6`, `"voided_processes": 0`, `"binaries_after": "all match"`.

Passes completed, from `progress.txt` and `raw/passes_done.txt` (17): C4-AB p0 19:59, p1 20:14, p2 20:43; C4-RAPIER p0 21:01,
p1 21:12, p2 21:22; C4-G5 p0 21:28, p1 21:34, p2 21:40; then, in the resume, C4-CGU p0 03:45, p1 03:51, p2 03:58; C4-BR p0 04:26;
C4-G4 p0 04:50, p1 05:14, p2 05:38; C4-G4-kd p0 05:48.

## Binaries

Sha256 prefixes, from `bin/SHA256SUMS`; `bin/COMMIT.txt` has the full provenance, commands and logs. Every runner key also
carries a `sha256_pin` in `rows9a*.json`, and the driver stops at start if one differs.

| key | exe | commit | what | built from |
|---|---|---|---|---|
| tip | `runner_tip_50e31f1a.exe` `8d6e7d41` | `50e31f1a` (`u/phys-tree-c4`) | the Tree is the default broadphase; the `parity` profile (fat LTO, 16 codegen units) | `git archive` export, `tools/build_runner.sh`, `cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid` |
| parent | `runner_parent_3d9433ae.exe` `a09fca08` | `3d9433ae` | the literal trunk: AllPairs is the default, S4 on; not window 8b's S4-off parent | the same command on an export of `3d9433ae` |
| g4rT | `broadphase_g4rT_50e31f1a.exe` `19c9eb1f` | `50e31f1a` + `bin/g4ref.patch` | the G4 criterion bench with the 28-size grid; bench profile (cgu 1, no LTO) | `tools/build_g4rT.sh`, `cargo bench --no-run --locked -p boyko-physics --bench broadphase` |
| g4r7 | `bpbench_g4ref_93b2615b.exe` `f96a9c11` | `93b2615b` + window 7's `g4ref.diff` | window 7 wave 2's G4 instrument | a byte copy (sha256 checked) |
| g4r8b | `broadphase_g4ref_16191fda.exe` `b887850f` | `16191fda` + window 8b's `g4ref.patch` | window 8b's G4 instrument | a byte copy (sha256 checked) |
| j56 | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` `918fd2b7` | Jolt v5.6.0 + the repository patch | P0's Distribution build (windows 3, 7 and 8's `j56`) | not rebuilt; run in place |
| rs8 | `rapier_parity_simd8_736c2a06.exe` `736c2a06` | the harness, revision 2 | Rapier 0.36.0, `parallel` + `simd8` (8 lanes) | the harness's `build.sh`; a byte copy |
| rs4 | `rapier_parity_simd4_1e5136ca.exe` `1e5136ca` | the harness, revision 2 | Rapier 0.36.0, `parallel` only (4 lanes) | the same |
| tipcgu1 | `runner_tipcgu1_50e31f1a.exe` `3e1f72ba` | `50e31f1a` | the tip with `CARGO_PROFILE_PARITY_CODEGEN_UNITS=1` (the one flag) | a second export of `50e31f1a`, `tools/build_tipcgu1.sh` |

- **Toolchain:** rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`, `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=8`, `RUSTFLAGS` unset (`x86-64-v3` from each tree's `.cargo/config.toml`).
- **The lock:** `--locked`, each tree's own `Cargo.lock`, sha256
  `c00bf4c41c52c8ae4bd12ec437acde99ae5dfc6881166551c8ddd93f4afceeab` (CRLF as exported), equal to window 8b's and identical
  before and after every build (`logs/build_*.log.lock_{before,after}`).
- **The g4rT tree** differs from the tip export in exactly one file (`benches/broadphase.rs`, +90 bytes; 17,586 files in both).
  `bin/g4ref.patch` is applied by content because the lane shifted the `G4_SIZES` line by one.
- **The cgu 1 build** reports 77 of 77 `rustc --crate-name` lines naming `-C codegen-units=1`. A control build with the
  override unset (not an arm, not in `bin/`) is byte-identical to the tip exe in `.text`, `.data`, `.pdata` and `.reloc`, and
  differs in 28 bytes of link identity (`tools/pe_diff9a.py`, `logs/pe_diff_cgu.txt`); so the only difference between `tip` and
  `tipcgu1` is the flag. The two exes report the same config receipt and the same pose, so **the sha256 pin is the only
  guard** against one running under the other's label. Build time: 66 s for cgu 1 against 59 s for the control (n = 1, informational).
- **Swap guards** for the runner keys: the pin, the config receipt (`tree_brute_max_rows` tip 128, parent 144, `broadphase_select`
  `Manual`) on every process, and the row's own broadphase and TreeDiag expectation.
- **No executable is committed.** `bin/SHA256SUMS` lists `j56` by its path under `D:/tmp/jolt`.

## The Rapier harness (`rapier-harness/`)

The Rapier comparator is an external cargo project, not a member of the workspace (`D:/tmp/rapier-parity` in the scratch
tree). `rapier-harness/` records it **at revision 2, the revision that built the two arms the window ran**, taken from the
harness's `gate/prev_rev2/` (its README, `Cargo.toml`, `build.sh` and `src/main.rs` as `*.rev2`):
- `README.md`, `Cargo.toml`, `Cargo.lock.txt`, `build.sh`, `.cargo/config.toml`, `src/main.rs.txt`, and `gate/*.md` (the harness's
  research, build, audit and review reports of its first revision, and `window9a_rows.md`, the rows and void rules the window used);
- `gate/bin/SHA256SUMS` and `gate/bin/SOURCES.sha256`, the revision 2 receipts. `sha256sum -c gate/bin/SOURCES.sha256` run
  from `rapier-harness/` passes for the five source files once the two renamed files are copied back:
  `cp Cargo.lock.txt Cargo.lock` and `cp src/main.rs.txt src/main.rs`. Both carry the `.txt` suffix in this record, and the bytes
  are the ones the exes were built with. `Cargo.lock.txt` because the repository ignores `*.lock`. `src/main.rs.txt` because
  `internal_docs_anchors` bounds-checks every `file.rs:N` citation written inside a `.rs` source under the tree, and the harness
  source holds four such citations of rapier3d's own files (lines 37, 51, 56 and 446: `solve.rs:98` twice, `lib.rs:19`,
  `staged_island_solver/init.rs:601`), files that are not in this repository, so they would count against that gate's caps (the
  precedent is window 8b's `analysis/f3/broadphase_16191fda.rs.txt`). The harness's own `README.md` and `gate/*.md` keep saying
  `src/main.rs`, as they were written. They are the same hashes `bin/COMMIT.txt` records for the window
  (`src/main.rs` `a27f8b55…`, `Cargo.toml` `c0368ed7…`, `Cargo.lock` `36dc81e0…`, `build.sh` `ff3cfeaf…`, `.cargo/config.toml` `2fab97fc…`).
- **The three arm exes** (not committed; the first two were the window's, copied into `bin/`):

  | arm | exe | sha256 | in the window |
  |---|---|---|---|
  | simd8 | `rapier-parity-simd8.exe` | `736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307` | key rs8, timed |
  | simd4 | `rapier-parity-simd4.exe` | `1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77` | key rs4, timed |
  | det | `rapier-parity-det.exe` | `f34291c5ffb8ee130e2c5eab92fbc4d8ad42f542db24ebfbfd6c19f921f3219d` | a receipt build (`enhanced-determinism`), never timed, not copied |

  `bin/SHA256SUMS` lists the first two; the `det` hash is in `rapier-harness/gate/bin/SHA256SUMS`.
- **Build:** rapier3d 0.36.0, rustc 1.98.1 msvc, `cargo build --release --locked`, fat LTO, `codegen-units = 1`,
  `-C target-cpu=x86-64-v3`. The resolved feature sets are `gate/rapier/features_simd8.txt` and `features_simd4.txt`; the two
  arms differ in one rapier3d feature, `simd8`.
- **The harness has moved on since the window.** Its scratch directory now holds a later revision: `src/main.rs` is no longer
  `a27f8b55…` (it is `a9269981…`), the three exes were rebuilt on 2026-10-01 between 02:20 and 02:26, and a `sweep/` directory
  exists. None of that ran in this window, and none of it is in this record. The window's pins are the hashes above, and
  `gate/rapier/pins.json` pins the arms, the fixtures and the source hash the exes report (`source_fnv1a64`).
- **The Rapier gate** (`gate/rapier/`, `gate/gate_rapier.json`, `gate/xcheck_rapier9a.txt`): the 20 timed-shape twins `g2t`
  and the 44 `g5` red-control logs the harness produced; `tools/xcheck_rapier9a.py` cross-checks the driver's validator against
  the harness's own `void_check.py` on 1,279 cases with 0 disagreeing, and `logs/mutate_rapier.out` catches 42 of 42 mutated
  copies of the driver.

## Validity, receipts and counts

**The first run** (`analysis.md` § 2, `analysis/synth/s0_validity.txt`):
- 696 timed process records and 12 warm-ups; 0 voided passes, 0 voided processes, 0 invalid; R4 pose compares 13 of 13 equal.
  The analyst's independent re-checks agree with the driver on 379/379, 214/214 and 95/95, and its K equals the driver's
  `k_clean` on 102/102, 60/60 and 30/30 pass-cells.
- **Poses and hashes hold:** all 222 used C4-AB runner processes carry pose `0x30c5438bc6ad9ffa` with bytes equal to the fixture
  and `void_steps` 0; all 38 used Jolt processes carry `0xb8522b4e3fc62cfe`; Rapier fails no V1–V9 rule on any of its 214
  processes; G5's 90 carry the three expected poses.
- **C4-AB pass 0 is a dead pass by the letter.** The desktop was busy (the top other process on unclean runs: Telegram 47,
  the browser 41, `claude.exe` 24; the witness median 2.15 % in pass 0, against 0.43 % and 1.06 %). 84 of 140 pass-0 processes
  were unclean, 46 slots were dropped (all in pass 0), and 30 of C4-AB's 102 pass-cells have K < 3, all in pass 0. Passes 1 and
  2 are clean, with K = 3 everywhere.
- **Re-runs read slow** (post hoc): the 50 used C4-AB re-runs have a median of 1.0378× their cell's originals.
- **Jolt W16 is bimodal** (five processes at 2.344–2.454 ms and four at 2.836–2.936 ms), so no W16 wall reading against Jolt can resolve.
- **Idle rule:** reached at the 130.5 s floor in all 10 first-run passes (21.7 min in total).
- The precondition "V2 is on the trunk" is NOT met, so every "ours faster" is recorded as UNEQUAL QUALITY.

**The resume** (`analysis.md` R2, `analysis/resume/z0_resume.txt`):
- The run wrote 185 records (140 process, 37 pass-cell, 8 `pass_done`); 134 timed, 23 re-runs, 6 warm-ups, 0 invalid, 0 voided,
  0 slots dropped. All 37 pass-cells have K = 3, and the analyst's K equals the driver's on 37 of 37.
- All 140 process records carry their key's pin (tip 61 × `8d6e7d41`, tipcgu1 58 × `3e1f72ba`, g4rT 15 × `19c9eb1f`,
  g4r8b 3 × `b887850f`, g4r7 3 × `f96a9c11`). All 119 C4-CGU processes, warm-ups included, carry pose `0x30c5438bc6ad9ffa` equal
  to JT500 (64,480 B), with `void_steps` 0.
- All 8 idle waits reached quiet in exactly 3 polls (130–131 s). The 23 unclean processes were all C4-CGU originals, and the top
  other process was `claude.exe` in every one. The used re-runs read 0.9990× the originals of their cell (no re-run bias).
- **What weakens the resume** (`analysis.md` R2): C4-CGU's resolution at W8/W16 (the unpaired i bar is 11.22 % at W8, against a
  paired-ratio IQR of 0.71 %); the cross-run drift above; kd is cross-block with n = 3; G4 runs under the `bench` profile (cgu 1,
  no LTO) while the shipped build is fat LTO.

## The untimed gate (`gate/`, `prep.md`)

Every part passed before each launch.
- **Fixtures** (`gate/fixtures/`, `fixtures.json`): JT500 `0x30c5438bc6ad9ffa`, JA500 `0x30c5438bc6ad9ffa`, RT500
  `0x6cbe24bf8fafda26` (each byte-equal to window 8b's, since C4 moves no pose), S16500 `0x4470add61a854f4c` (new), and
  the Rapier fixtures RD8 = RD4 (`0x36e6142dc4db4701`) and RM8 = RM4 (`0x29f2b3226e7e46c4`).

  | fixture | sha256 prefix | already committed as |
  |---|---|---|
  | JT500, JA500 | `e268d7a5` | window 8's `JT500.pose` and `JA500.pose`, and window 8b's `JT500.pose` |
  | RT500 | `11b83951` | window 8b's `RT500.pose` |
  | S16500 | `4bf5e161` | (new in this record) |
  | RD8, RD4 | `06d2df2f` | (new in this record; the two files are the same bytes) |
  | RM8, RM4 | `a927ee84` | (new in this record; the two files are the same bytes) |

- **Rows 44/44** (the driver's own validators): exit 0, the expected pose, `void_steps` 0, the config receipt, TreeDiag, `canary_ns`
  60000 on the rung rows; armed rows read setup steps/tasks 0/0 at W1 and 500/15980 at W8; the five Jolt cells exit 0 with
  hash `0xb8522b4e3fc62cfe` at every W and 500 frames.
- **Pre-flight at W 1/2/4/8/16:** the tip flagless is the Tree with `tree_brute_max_rows` 128, the parent flagless is AllPairs
  with 144, both with the JT500 hash; the tip with `--broadphase allpairs` equals the parent flagless byte for byte, and the
  tip flagless equals the parent flagless byte for byte. **Red controls 4/4, exit 4** (501 steps against JT500, and
  `--contact-reuse off` against JT500, on both exes).
- **Criterion:** `--list` of every (row, exe) equals the expected ids (C4-BR 8/8 on g4r7, g4r8b and g4rT, C4-G4 36/36, C4-G4-kd
  6/6), and full-size samples pass.
- **Rapier:** 20/20 timed-shape cells through the driver's validator; the cross-check above.
- **C4-CGU:** 10/10 gate cells; every gate cell's `pose.bin` equals JT500 byte for byte for both exes.
- **The selftest** (`tools/selftest9a.py`, `gate/selftest9a.txt`): 320 of 320 at the last run (148 at the first cut, 256 after the
  Rapier block); mutation proofs in `logs/mutate_*.out` (25 of 25 for the C4-CGU additions, 42 of 42 for the Rapier validator,
  and the audit's twelve); two defects of the gate's own were found and fixed (`prep.md`).
- **Driver `--test` rehearsals** (`test/`, not committed): every block exits 0 with 0 invalid.

## Rulings

Recorded in `docs/physics/perf-campaign/levers/00-RULINGS.md` (see the top of this file). The window's own pre-registered
rulings, which `analysis.md` cites, are in `prep.md`: ruling (a) the Rapier block and its adjacency waiver, (b) C4-BR at K = 3,
(c) the validator, and the audit's findings.

**What moved to window 9b** (ruling 10 of 2026-10-01; `analysis.md` R5): C4-BR, C4-G4, C4-G4-kd and the cgu 1 twin are dropped
(measured). Window 9b is V2-AB/SPAN/RES with Jolt 5.6 in-block, Rapier's fastest holding configuration in-block and adjacent to
our rows, the SR and S5 A/B when they exist, and an armed C4-JD W16 row for S5. Every bar is formed in-block.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The synthesis, verbatim; both parts (see the top of this file). § 1 headline; Method; § 2 validity; § 3 pre-registered verdicts (C4-AB, C4-RAPIER, C4-G5); § 4 decisions; § 5 owner questions; "Resume 2026-10-01" R1–R7 |
| `analysis/{c4ab,c4ab-jolt,rapier,g5}/` | The first run's group scripts and outputs, and their `verify/`. **`c4ab/` also holds the `.txt` outputs of the G4 group** (`g0_validity.txt` … `g5_robust.txt`): `lib9a.Out.save` wrote them beside `lib9a.py`, not into `g4/` (PC-9a-11); the `.py` and `.json` files are in `g4/` |
| `analysis/{cgu,g4,br,resume}/`, `analysis/synth/` | The resume's group scripts and outputs (with `verify/`), the resume census (`resume/z0_resume.py`), and the window-wide figures (`synth/s0`–`s3`) |
| `prep.md` | The tester's prep report, verbatim: binaries, the Rapier block, the re-run allowance, the C4-CGU block and the resume launch line, the untimed gate, the disclosure, and the orchestrator's pre-registration addendum |
| `rows9a.json`, `rows9a.extra.json`, `rows9a.extra.example.json` | The run list (16 rows, protocol, lane rules, binaries); the overlay with C4-RAPIER and C4-CGU (3 rows); the placement template |
| `run_window9a.sh` | The launcher, which resolves everything relative to its own directory: the driver, then `tools/counts9a.py`, then `WINDOW_DONE` |
| `dryrun.txt`, `dryrun_resume_cgu.txt` | The full schedule (279.4 min), and the resume's schedule (171.4 min) |
| `wait_log.txt`, `progress.txt`, `WINDOW_DONE`, `WINDOW_DONE.run1` | The idle-rule poll log (both runs), the per-row pass completion lines, the resume's marker, and the first run's |
| `bin/SHA256SUMS`, `bin/COMMIT.txt`, `bin/g4ref.patch` | The binaries' sha256 and provenance, and the one-line `G4_SIZES` patch. No executable is in the tree |
| `rapier-harness/` | The Rapier 0.36 harness at revision 2 (see above) |
| `logs/` | The build logs and lock hashes; the gate and rehearsal outputs; the mutation runs; and `*.before_audit1.*`, `*.before_rapier.*`, `*.before_cgu.*`, the files as they were before each preparation step |
| `gate/` | The untimed gate: `gate_*.json`, `gate_run.log`, `estimates.json`, `selftest9a.txt`, `xcheck_rapier9a.txt`; `fixtures/` (the eight `.pose` files and `fixtures.json`), `fixture_runs/`, `cells/`, `preflight/`, `red/`, `rapier/`, `criterion_sample/`, `selftest_tmp/` |
| `raw/runs.jsonl.gz` | 1,107 records (848 processes, 229 pass-cells, 17 pass markers, 13 R4 pose compares): args, env, exit, both receipts, the witness with its top-5, per-CPU busy, validity and contamination flags, the binary's hash, the placement receipt, the driver's summary. **Gzipped** (see below) |
| `raw/<block>-p<n>_{191354,033740}/<seq>_r<k>_<row>_<bin>_W<w>[_warmup\|_R\|_R<n>]/` | One directory per process (a re-run has the suffix `_R`, or `_R2`, … for a later one): `stdout.txt`, `stderr.txt`, `run.csv`; for Jolt the per-frame CSV; for the criterion blocks the `criterion/` tree |
| `raw/counts.txt`, `raw/manifest_1790784834.json`, `raw/manifest_1790815060.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The untimed validity counts (over both runs), the two launch manifests (first run; resume), the resume's start/end state, the window log, the driver's stdout, the launcher's log of both runs, the passes done, and the driver's exit status |
| `tools/` | `window9a_run.py` (the driver), `wait_idle9a.ps1`, `mkrows9a.py`, `mkextra9a.py`, `gate9a.py`, `counts9a.py`, `micro9a.py`, `selftest9a.py`, `xcheck_rapier9a.py`, `patch_trees9a.py`, `pe_diff9a.py`, `rehearsal_summary9a.py`, `rehearsal_summary_cgu9a.py`, `build_all9a.sh`, `build_runner.sh`, `build_g4rT.sh`, `build_tipcgu1.sh`, `joltprof.py`, `lib/driver.py`, `window_lib/pdhperf.py` |
| `skipped.sha256` | Every file of the scratch record that is not committed here: sha256, size, reason, path |

**`raw/runs.jsonl` is stored gzipped, as `raw/runs.jsonl.gz`.** The file is 7,184,033 bytes, over the 5 MB rule; the gzip is
497,949 bytes (deterministic: `mtime` 0, level 9; sha256 `5977c0285b46747e144ea777e9583c9a9b6ecced7e3d6890edc57140a45e6f53`).
**The sha256 of the uncompressed file is `00186d58fd1af0318a6bc3cb099db4664f80c2b5763a70161ff8290f34b88673`.** The analyst's
scripts read `raw/runs.jsonl`; `gunzip` it in place before re-running them.

**Not committed** (each file's sha256 is in `skipped.sha256`):
- **The executables**, 8 files (the eight exes of the binaries table that were copied into the scratch `bin/`).
- **Pose dumps, 1,007 `pose.bin` files in `raw/` and `gate/`.** 890 are byte-identical to one of the committed fixtures, so
  `gate/fixtures/` keeps every distinct pose of a timed or gate process. The other 117 are not fixtures: 113 are 12-step outputs of
  the selftest's rehearsals (`gate/selftest_tmp/`), and 4 are red controls (`gate/red/`: two 501-step runs, `6cdcbc31…`, and
  two `--contact-reuse off` runs, `eff361e1…`, which is window 8's JToff500).

  | sha256 prefix | `pose_hash` | = fixture | files in `raw/` | files in `gate/` |
  |---|---|---|---|---|
  | `e268d7a5` | `0x30c5438bc6ad9ffa` | JT500 = JA500 | 486 | 70 |
  | `06d2df2f` | `0x36e6142dc4db4701` | RD8 = RD4 | 115 | 37 |
  | `a927ee84` | `0x29f2b3226e7e46c4` | RM8 = RM4 | 105 | 10 |
  | `11b83951` | `0x6cbe24bf8fafda26` | RT500 | 37 | 5 |
  | `4bf5e161` | `0x4470add61a854f4c` | S16500 | 22 | 3 |

  Each process's `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`. The analysis scripts that
  compare pose bytes (`analysis/c4ab/lib9a.py`, `analysis/cgu/c0_validity.py`, `analysis/rapier/lib_rp.py`) read `pose.bin`; a
  process's `pose.bin` is the fixture whose hash its `stdout.txt` reports.
- **`analysis.before_resume.md`**: the first run's synthesis before the resume section was appended. It is byte-for-byte the
  first 27,984 bytes of `analysis.md`.
- **The rehearsal directory `test/`** (1,646 files) and **`alt/`** (the K = 9 C4-BR schedule that was not run:
  `rows9a.br_k9.json`, `dryrun_br_k9.txt`); `prep.md` cites both. They are the driver's untimed `--test` output and an
  alternative schedule, not part of the record.
- No committed file exceeds 5 MB.

Also outside the tree: the exported trees under `D:/wt/_targets/w9a-trees*`, the build trees and every target directory, and the
Rapier harness's own scratch directory (the revision-2 sources are in `rapier-harness/`).

## How to re-run

1. **Build** into `bin/` and hash, with the commands in `bin/COMMIT.txt`: `tools/build_runner.sh <tree> <target> <log>` for the
   tip and the parent, `tools/build_g4rT.sh` for the G4 bench, `tools/build_tipcgu1.sh` for the cgu 1 twin. Every tree is a `git
   archive` export, never a worktree; `bin/g4ref.patch` is applied by `tools/patch_trees9a.py`. The Rapier arms are built by the
   harness's `build.sh` (`rapier-harness/`), and an exe is accepted by its sha256 pin.
2. **Gate:** `python -B tools/gate9a.py fixtures|rows|preflight|criterion|rapier …` and `python -B tools/selftest9a.py` write `gate/`.
3. **Run the window:** `bash run_window9a.sh --cutoff HH:MM`, once, with no agent working; `--resume` after a stop, `--dry-run
   [--start HH:MM]` for the schedule, `--test` for an untimed rehearsal, `--blocks a,b` to limit blocks. Creating `STOP` stops
   the driver before its next process.
4. **Reduce:** `raw/counts.txt` comes from `tools/counts9a.py`. The analyst's statistics come from `analysis/**/*.py`, which read
   `raw/` (after `gunzip`), `gate/fixtures/`, `rows9a*.json` and `bin/SHA256SUMS`.
5. **Quote nothing that is not in `raw/`.**
