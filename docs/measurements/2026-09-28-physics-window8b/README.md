# Physics perf campaign, window 8b — S4-AB, S7-AB, the per-wave split, omega_b v2, F3 and the tree thresholds, J-Son-T, DM1 C6 (2026-09-29)

This directory holds the receipts for:
- `docs/MEASUREMENT-QUEUE.md` section 17, the `RESULT, 2026-09-29, window 8b` block;
- the orchestrator's rulings after the window. They are **not re-recorded here**: they are ported to
  `docs/physics/perf-campaign/levers/00-RULINGS.md`, section "After window 8b: S7's partial form rejected and frozen, S4
  kept, F3 not the default, the tree thresholds not reproduced, helpers spin (2026-09-29)", rulings 1–10 (trunk commit
  `3d9433ae`). Their source, the orchestrator's `RULINGS-2026-09-29-W8B.md`, is not in this tree.

The directory is dated 2026-09-28, the day of the preparation (`prep.md`, `rows8b.json`). **The window itself ran on
2026-09-29** (17:21–21:09 +03:00).

**Read `analysis.md` first.** It is the results-analyst's synthesis, recomputed from `raw/` by the analyst's own scripts
(`analysis/synth/`), and it is recorded verbatim. It points to five group appendices, which are also recorded:
`analysis/s7.md` (S7-AB), `analysis/s4split.md` (S4-AB and SPLIT), `analysis/omega.md` (omega_b v2, omega(W, gap), v1
continuity), `analysis/f3.md` (F3 and F3-G4) and `analysis/jsondm1.md` (J-Son-T and DM1). Each group has a directory of
scripts and tables beside it, with a `verify_numbers/` (or `verify_validity/`) sub-directory holding the independent
re-derivation that agreed with the group's figures. The window was launched from the tester's prep report, recorded
verbatim as `prep.md`. It has three parts, in the order they were written: the base prep (2026-09-28), the reserved blocks
S7-AB and omega-v2 added on 2026-09-29, and omega(W, gap) added later the same day.

**How to read the references in `analysis.md`, `prep.md` and the appendices.**
- **Trunk state.** At the time of recording the trunk `integ/unified` is `82b3867f`.
  - The tip the window measured, `16191fda`, is on the trunk.
  - **`a3adc827` is not on the trunk.** It is `u/phys-s7` commit (2), S7's partial form, rejected by the rulings and kept
    under the annotated tag `phys/s7-partial-frozen`. `git show a3adc827` shows it. The S7 lane's commits (0)
    `af8378d1` and (1) `3129a618` (the ω_b v2 bench) are on the trunk, merged by `0e4537ff`.
- "design" and `01-DESIGN.md` mean `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md`, which is on the trunk now.
  `scale_design.md` (`analysis.md` § 4.6) is not in this tree under that name.
- "window 8 §N" means `docs/measurements/2026-09-27-physics-window8/analysis.md`; "window 7" means
  `docs/measurements/2026-09-25-physics-window7/`.
- **Correction to window 8's record** (`analysis.md` PC-8b-7, `analysis/s4split.md` post hoc 1): window 8's per-wave table
  was in TSC ticks, not ns. Its "162 % of ω" and "W16 growth mostly in the tail, ~0.16 ms" need correction notes where they
  are cited. Window 8's verdicts are unchanged. Window 8's own directory is not edited by this record.
- **Named inputs that are not in this tree:**
  - `RULINGS-2026-09-27-W8.md`, whose ruling 1 is the claim rule (it is ruling 1 of the section "After window 8" of
    `00-RULINGS.md`);
  - the S7 lane's cut `s7-omega2/cut.md`, cited by `analysis.md` § 4.1 and by `prep.md`;
  - the F3 lane's `tree-f3/window_cmds.md`, which defines the F3 / F3-G4 rows and the "not R2" branch;
  - the DM1 lane's `dm1/cut.md`.
- `analysis/s4split/` holds two pickles, `q_s4.pkl` and `q_split.pkl`: caches written by the analyst's scripts, not inputs.

## Headline

`analysis.md` § 1, restated with its numbers. Window [0,500) unless marked. K = 9 per cell over three passes, ruling 1.
- **S4 holds and stays.** At W8 on J-T the trunk (`16191fda`) is 0.1705 ms (8.58 %) faster than its S4-off parent,
  STRONG, and no W is slower. The claim rests on the pre-registered span route, because the block's own 0.060 ms rung was
  not seen.
- **S7's partial form fails and does not merge.** W8 is unchanged; W16 is slower, STRONG: +0.1400 ms (+6.83 %) on J-T and
  +0.6579 ms (+12.63 %) on J-A. That is the opposite of the prediction, and R_16 = 0.0525 ms was demonstrated in the same
  block.
- **F3 does not become the default.** The kd order cuts the J query by 16.88 % (R1, STRONG), but the kd build eats the
  gain (R3 not claimed), so `LeafList` stays the default.
- **The tree thresholds 144/152 are not reproduced.** The G4 block moves the tree / all-pairs crossover to 128–136.
- **C1b and C6 are kept.** Sleeping on the tree broadphase shows no resolved awake cost and cuts the settled pile by
  96.5–99.1 %. DM1's C6 is kept.
- **S1's inputs.** The ω_b v2 re-bench puts the net barrier of the S1 region at 0.35–1.07 µs per stage at 8
  participants, about today's dispatch cost per wave (0.91–0.95 µs). Parking costs about 6 µs per stage after a serial
  stretch longer than 5–20 µs. The S1 build decision turns on how an S4-pattern interim (S2i + S3i) is priced.
- **The K = 2 reading.** F3's R2, four disarmed J-Son-T product rows and the gap-20/80 park price are NOT CLAIMED by the
  pre-registered letter, only because a contaminated pass kept K = 2. If a K = 2 pass may gate, all of them are STRONG.
  Of the decisions, only F3's branch label moves with that reading (PC-8b-1).
- **No Jolt row.** The standing against Jolt 5.6 is not re-measured.

## What was measured

Nine blocks, in priority order (`rows8b.json`, `rows8b.extra.json`, `prep.md`). Every block runs K = 9 (three passes × three
rounds; pass 0 reversed, pass 1 forward, pass 2 reversed), except where noted. Counts are from `raw/runs.jsonl`.

| block | question | binaries | passes | timed (of which re-runs) | span |
|---|---|---|---|---|---|
| S4-AB | S4 on against off at W 1/2/4/8/16; the in-block rung and zone canaries at W8 | parent, tip | 3 | 155 (2) | 17:23–17:44 |
| S7-AB | S7's partial form against the trunk at W 8/16 on J-T and J-A; the ladders at W8 and W16 | s7p, s7t | 3 | 173 (2) | 17:47–18:13 |
| SPLIT | the per-wave split of the S4-off parent at W 1/8/16 | parent | 3 | 54 (0) | 18:16–18:27 |
| omega-v2 | ω_b v2, and ω(W, gap) with the participation receipt | omega2 | 3 | 75 (21) | 18:30–19:09 |
| F3 | the kd query kernel against leaflist: query, build, walls, poses | tip | 3 | 299 (56) | 19:11–19:54 |
| F3-G4 | the threshold re-read: criterion ids, sizes 128–1000 | g4ref | 1 pass, K = 3 | 3 (0) | 19:56–20:29 |
| J-Son-T | sleeping on the tree broadphase at W 1/8 | tip | 3 | 85 (13) | 20:32–20:50 |
| DM1 | C6 re-read: ABBAABBA per row at 1920×1080, FIFO present mode | dmA, dmB | 1 | 30 (6) | 20:52–20:59 |
| omega-v1-cont | window 8's omega_b rows re-run on the v2 exe | omega2 | 1 pass, K = 3 | 20 (8) | 21:02–21:09 |

The total is **894 timed processes** (786 originals and 108 re-runs) and 20 warm-ups, in 21 passes.

| row | block | args (+ `--workers W --steps 500 --window 0..500 --csv --pose-out --label --expect-pose gate/fixtures/<ref>.pose`; armed rows `--arm-profiler`) | W | pose ref |
|---|---|---|---|---|
| `S4-JT` / `S4-JT-a` | S4-AB | `--scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off` | 1/2/4/8/16; 8/16 | JT500 |
| `S4-rung` | S4-AB | as `S4-JT`, `--canary-frac 1 --canary-ref-ns 60000` | 8 | JT500 |
| `S4-zone-N30000` / `-N60000` | S4-AB | as `S4-JT`, `--canary-zone phys_solve_build --canary-ns 30000` / `60000` (armed) | 8 | JT500 |
| `S7-JT` / `S7-JA` | S7-AB | as `S4-JT`; and `--scene jolt --gap 0.5 --cfg a` | 8/16 | JT500 / JA500 |
| `S7-ladder8-F{0.5,1,1.5,2}` | S7-AB | as `S4-JT`, `--canary-frac F --canary-ref-ns 60000` | 8 | JT500 |
| `S7-ladder16-F{0.5,1,1.5,2}` | S7-AB | the same with `--canary-ref-ns 105000` | 16 | JT500 |
| `S7-JA-ladder16-F1` | S7-AB | J-A, `--canary-frac 1 --canary-ref-ns 105000` | 16 | JA500 |
| `S7-JT-W1` | S7-AB | as `S4-JT` (the optional pair) | 1 | JT500 |
| `SPLIT-J-T` / `SPLIT-J-T-a` | SPLIT | as `S4-JT` | 1/8/16 | JT500 |
| `F3-TD-armed-{leaflist,kd}` | F3 | `--scene jolt --gap 0.5 --cfg default --broadphase tree --bp-kernel leaflist\|leaflist-kd` (armed) | 1/8 | JT500 |
| `F3-TA-armed-{leaflist,kd}` | F3 | the same with `--cfg a` (armed) | 1 | JTA500 |
| `F3-TR-armed` | F3 | `--bp-kernel rowwalk` (armed, optional) | 1 | JT500 |
| `F3-JT-{leaflist,kd}` / `F3-RT-{leaflist,kd}` | F3 | as `F3-TD`, disarmed; `--scene rest --cfg default` | 1/2/4/8/16 | JT500 / RT500 |
| `G4` | F3-G4 | `--bench --noplot` with the 60-id regex of the F3 lane's `window_cmds.md`, `CRITERION_HOME` per process | — | — |
| `JSonT` / `JSonT-a`, `JSoffT` / `JSoffT-a` | J-Son-T | `--scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on\|off`; metric windows [0,100) [100,500) [274,500) | 1/8 | JSonT500 / JSoffT500 |
| `dm-{vb,deferred}-1920x1080-idle`, `dm-vb-1920x1080-edit100` | DM1 | `dm1_material_table_timing --exact --ignored --test-threads=1 --nocapture`; env `BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_ZONE=1 BOYKO_VB_BENCH_FRAMES=220 BOYKO_DM1_PATH BOYKO_DM1_RES BOYKO_DM1_EDIT_ROWS`; ABBAABBA | — | — |
| `omega2-{worker,external}` | omega-v2 | `--bench --mode omega-b2 --participants 2,4,8 --stages 36,72 --blocks-per-participant 1,2,4 --helper spin,park --route worker\|external` | — | — |
| `omega2-gap-{worker,external}` | omega-v2 | `--mode omega-b2 --participants 8 --stages 36,72 --blocks-per-participant 4 --helper spin,park --gap-us 20,80` | — | — |
| `omega-wgap-{worker,external}` | omega-v2 | `--bench --mode omega --workers 8,16 --gap-us 0,5,20,80 --route worker\|external` | — | — |
| `omega1-s{36,72}-{worker,external}` | omega-v1-cont | `--bench --mode omega-b --participants 2,4,8,16 --stages 36\|72 --route worker\|external` | — | — |

Order inside a block: the forward list runs W ascending and, inside a W group, row by row in the order above with each
row's binaries in order, so the two binaries of an A/B row (and each armed row and its disarmed twin) are adjacent in
every round. Passes 0 and 2 run that list reversed, and pass 1 runs it forward. The F3 pairs are checked for pose equality
per (pair, W, round): the `r4` records.

### Rows omitted, and what was added after the base prep (`prep.md`)

- **No non-FIFO present mode.** `boyko_app/src/host.rs:226` uses `Swapchain::new` (FIFO) with no environment knob, so
  DM1 reads FIFO only (the driver checks `present_mode = "fifo"`).
- **The DM1 grow frame is NOT MEASURED** (ruling 9 of window 8's rulings): a reported number by design.
- **DM1 runs at 1920×1080 only.** Window 8's probe found that the display hosts nothing larger (window 8's
  `gate/dm1_res.json`), and `analysis.md` § 6 Q3 asks whether a 1440p or 2160p display is available.
- **Added on 2026-09-29** through `rows8b.extra.json`, which the driver merges at start: the S7-AB block (cut § 5 item 3,
  19 cells), omega-v2 (`omega2-*`, then `omega-wgap-*` with the N4 participation receipt) and omega-v1-cont (the optional
  continuity rows, K = 3). The additions also changed tool files (the new `micro8b.py` rules, the `sha256_pin` check, a
  per-cell park rule in `counts8b.py`); the earlier versions are in `logs/tools_pre_s7_20260929/` and
  `logs/tools_pre_wgap_20260929/`.
- **No Jolt row** was in the window.

## Protocol

Window 8's (`docs/measurements/2026-09-27-physics-window8/README.md`), with **ruling 1 of window 8's rulings** replacing
the claim rule (`rows8b.json` → `protocol`; `tools/window8b_run.py` header).
- **Cell.** The MEDIAN over K = 9 separate processes of each process's statistic: the mean of `wall_ns` over the window;
  F3's armed spans, the median over steps [100,500); ω_b v2, (region@72 − region@36)/36 within one process.
- **Spreads.** r = min–max, i = IQR (inclusive quartiles), s = 1.2533·SD/√K, each relative to the median. B against A is
  flagged iff |B/A − 1| > 2·hypot(A_x, B_x).
- **Claim rule (ruling 1).** CLAIMED iff i AND s flag, pooled (K = 9) AND in each of the three passes, with one sign
  throughout. STRONG iff r also flags, pooled and in every pass. Flags are printed i/s/r in `analysis.md`.
- **A pass-cell with K < 3 sets no flag** (window 7's clause, imported by every later window). The synthesis applies this
  reading ("LETTER") window-wide; "K2" means a K = 2 pass-cell gates on its own i and s, which needs a ruling (PC-8b-1).
- **Slot rule.** The original if valid and clean, else its re-run if valid and clean, else the slot is dropped. Clean means
  both receipts ≤ 5 %, the during-process witness ≤ 2 % and no build process. Placement receipts are recorded and never
  used to drop a process.
- **Idle rule before EVERY pass** (`tools/wait_idle8b.ps1`: three consecutive 60-s polls with no build process, no process
  under `D:/wt/_targets` or `D:/wt/mq-*`, a 10-s CPU below 5 %; up to 30 polls; a timeout stops the window, exit 3).
  Receipts: 10 s before a pass, 5 s between processes. One re-run per unclean slot at the end of the pass.
- **Own K.** F3-G4 keeps its recipe's K = 3 (one pass, no separate warm-up: criterion warms every benchmark); DM1 keeps
  ruling 9's ABBA × 2 per row, one pass.
- **Placement receipt** (new): per process, the main thread's cycle share (`QueryThreadCycleTime`), its CPU seconds, the
  top-3 logical CPUs by busy share, and `main_share_top_est`. Recorded, never used.
- **Launch** P-none: no affinity call. **Hard stop:** no process starts whose estimated end passes the cutoff, 21:36 here,
  which was not reached. A `STOP` flag file was available and was not used.

## When and where

The owner's workstation (8 physical / 16 logical cores, windows-msvc). The power scheme read High performance
(`raw/window_state.json`). Every build and the whole untimed gate came before the launch.
- **Launch 17:21:02** (`bash run_window8b.sh --cutoff 21:36`); idle was reached at 17:23:13.
- **The timed window ran 17:23:23–21:09:15.** `WINDOW_DONE` was written at 21:09:20 and reads `exit 0` / `complete` /
  `counts exit 0`. `raw/window_state.json` reads `"status": "complete"`, `"exit": 0`, `"timed": 894`, `"rerun": 108`,
  `"warm": 20`, `"voided_processes": 0` and `"binaries_after": "all match"`.
- The window lasted 228.3 min. The schedule was 212.9 min of processes plus idle waits (`dryrun_wgap.txt`).

The per-pass completion lines are in `progress.txt`, and the per-block spans are in the table above.

## Binaries

Sha256 prefixes, from `bin/SHA256SUMS`; `bin/COMMIT.txt` has the full provenance, build commands and logs.

| key | exe | commit | what | built from |
|---|---|---|---|---|
| tip (and s7p) | `runner_tip_16191fda.exe` `aa34fadf` | `16191fda` | the trunk with S4 on, F3 behind `--bp-kernel`; S7-AB's P is the same file | `git archive` export, `tools/build_runner.sh`, `cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid` |
| parent | `runner_parent_16191fda_s4off.exe` `9c6690ac` | `16191fda` + `bin/parent.patch` | S4 off: `solver/colored.rs` `SETUP_MAX_TASKS` 32 → 1, so the step takes the inline setup path the tip takes at W1 | the same export, patched byte-exactly by `tools/patch_trees.py` |
| g4ref | `broadphase_g4ref_16191fda.exe` `b887850f` | `16191fda` + `bin/g4ref.patch` | the G4 criterion bench with the 28-size grid (the `G4_SIZES` line) | `tools/build_g4ref.sh`, `cargo bench --no-run --locked -p boyko-physics --bench broadphase` (bench profile) |
| s7t | `runner_s7_tip_a3adc827.exe` `c4232eae` | `a3adc827` (`u/phys-s7`, not on the trunk) | S4 on, S7's partial form | `git archive` export of `a3adc827`, `tools/build_runner.sh … --bench omega_b_region` |
| omega2 | `omega_b_region_v2_a3adc827.exe` `48e9486a` | `a3adc827` | the ω_b v2 bench, with window 8's v1 modes | the same build |
| dmA / dmB | window 8's `dm1_A.exe` `47cb2c9b`, `dm1_B.exe` `79bba864` | `cad5439b` / `97ee830f` | DM1 A (host-visible material table) and B (C6, device-local), dev-profile builds | not rebuilt; run in place from window 8's scratch directory |

- **Toolchain:** rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`, `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=8`, `RUSTFLAGS` unset (`x86-64-v3` comes from each tree's `.cargo/config.toml`).
- **The lock:** `--locked`, with each tree's own `Cargo.lock`, sha256 `c00bf4c41c52c8ae4bd12ec437acde99ae5dfc6881166551c8ddd93f4afceeab`
  (CRLF as exported). It was identical before and after every build (`logs/build_*.log.lock_{before,after}`).
- **The `Compiling` path.** Each build log reads `Compiling boyko-physics v0.1.0 (D:\wt\_targets\w8b-trees\<name>\crates\boyko_physics)`,
  so each binary compiled the exported tree of the commit it is named for. The two patched trees differ from the tip
  export in one line of one file each (`diff -rq` over the whole trees).
- **S7-AB's provenance** (`prep.md`): `git diff --stat 54a7714f 16191fda` is `CLAUDE.md` only (no `crates/` file), and
  `git diff --stat 16191fda a3adc827` is the S7 lane's eleven files; both commits are ancestors of `a3adc827`.
- **`bin/SHA256SUMS` lists the two DM1 exes by their scratch path in window 8's directory.** Their hashes are window 8's.
- **Every binary was re-checked after the window** (`"binaries_after": "all match"`), and each runner process's
  `exe_sha256` equals its key's pin (`sha256_pin`; the driver stops at start if a pin differs).

## Validity, the idle rule, receipts and counts (`analysis.md` § 2)

- **Validity.** 0 of 894 processes are invalid, and the analyst's own check agrees with the driver on all 894 (exit,
  sha256, 500 CSV steps, `void_steps` 0, the expected pose, workers, the SUMMARY mean, the `micro8b` rules, DM1's FIFO and
  zone n = 220, and all 60 criterion ids). Passes voided: 0. R4 pose compares: 171 of 171 equal. Park-gap void cells:
  0 of 8.
- **Contamination.** 134 of 894 timed processes were unclean (the witness's top "other" process: `claude.exe` 99,
  `browser.exe` 17, `Telegram.exe` 12). 82 of 108 re-runs were used, and **26 of 786 slots were dropped**: F3 16, omega-v1-cont
  4, omega-v2 2, J-Son-T 2, DM1 2. Pass-cells with K = 2: F3 16 of 81, omega-v1-cont all 4, omega-v2 2, J-Son-T 2.
- **Idle waits** (`wait_log.txt`): 21, one before each pass. 18 reached idle at the 130.4–130.5 s floor; S7-AB pass 1 took
  250.5 s, omega-v2 pass 0 190.5 s and omega-v2 pass 2 490.5 s. In total the idle waits took 54.7 min.
- **The F3 block ran slow, post hoc.** F3's leaflist J-T row and S4-AB's tip row are the same binary with an identical
  effective configuration, yet F3 reads 4–11 % slower at every W (1.0417 / 1.0521 / 1.0779 / 1.1066 / 1.0824 at W
  1/2/4/8/16), and no per-process receipt sees it. Every F3 verdict is within-block, so none is affected; F3's absolute
  values are not comparable across blocks (`analysis.md` § 2 item 2).
- **Recorded weaknesses** (`analysis.md` § 2): S4-AB's parent at W16 had uniformly slow processes with clean receipts;
  S4-AB has no in-block ladder at W8/W16 and no rungs at W 1/2/4; the ω_b v2 calibrated block work was 568.6–577.6 ns,
  not the specified 0.7 µs, the 36-stage configs always run before the 72-stage ones, and the worker route at P8,
  b = 4P has a recruitment asymmetry; omega-v1-cont ran last on a busy desktop at K = 2; the DM1 gate is weak on three
  zones (bands of 21.9–25.4 %).

## The untimed gate (`gate/`, `prep.md`)

Every part passed before the launch.
- **Fixtures** (`gate/fixtures/`, `fixtures.json`): JT500 `0x30c5438bc6ad9ffa`, JToff500 `0x32d5e235342b4143`, JTA500
  `0x30c5438bc6ad9ffa` and RT500 `0x6cbe24bf8fafda26` (all known); JSonT500 `0x3db47fae414b655c` and JSoffT500
  `0x30c5438bc6ad9ffa` (recorded). JA500, recorded by `tools/s7pre8b.py fixture`, is byte-equal to window 8's. Six of the
  seven `.pose` files are byte-identical to files already committed in window 8's `gate/fixtures/`; RT500 is new:

  | fixture | sha256 prefix | already committed as |
  |---|---|---|
  | JT500, JTA500, JSoffT500, JA500 | `e268d7a5` | window 8's `JT500.pose` and `JA500.pose` |
  | JToff500 | `eff361e1` | window 8's `JToff500.pose` |
  | JSonT500 | `205bc544` | window 8's `JSon500.pose` |
  | RT500 | `11b83951` | (new in this record) |

- **S4 (30/30):** both exes × {J-T, J-T-a, J-T-off} × W 1/2/4/8/16 exit 0 with the expected pose and `void_steps` 0. The
  tip's setup steps/tasks read 0/0, 500/6000, 500/12000, 500/15980, 500/15980 at W 1/2/4/8/16, and the parent's 0/0 at every W.
- **Red controls (4/4, exit 4):** a 501-step run against JT500 and J-T-off against JT500, on both exes. The pose gate can fail.
- **Rows (58/58)** at their W, 500 steps, `--expect-pose`; then the S7 rows (19/19) and the micro rows (8/8, then 2/2 for
  the omega(W, gap) rows). F3: the pre-flight `cmp` is silent, kd builds 0/20, R4 13/13 equal. The J-Son-T freeze step is
  274 at W1 and W8, armed and disarmed. G4: `--list` equals the 60 expected ids and a full sample passes 60/60.
- **S7 pre-flight (`gate/s7/`):** P and T × W 1/2/4/8/16 × {J-T, J-A} exit 0 with the expected pose, `cmp` P against T
  silent 10/10; the engagement receipt: at W16 the trunk's lanes sum is above the physical-core bound (607,272 on J-T, 597,418
  on J-A, against 403,732) and the capped build's is within (391,361 and 399,017); at W8 both are within.
- **DM1:** A and B at 1280×720 and 220-frame samples of the three rows exit 0 with zones n = 220 and `fifo`.
- **The validators go red under mutation** (`tools/selftest8b.py`, 63 of 63 at the last run, `gate/selftest8b.txt`).
- **Driver `--test` rehearsals** (`test/`, not committed): every block exits 0 with 0 invalid.

## Rulings

Recorded in `docs/physics/perf-campaign/levers/00-RULINGS.md`, "After window 8b" (see the top of this file). In one
sentence: helpers spin (owner); S7's partial form is rejected and frozen; S4 is kept; F3 is not the default; the tree
thresholds are re-read at 96–128 before tree C4 wires them; C1b is supported and DM1 C6 is kept; a K < 3 pass-cell does not gate and
the driver re-runs a dropped slot until K = 3; S1 stays undecided until S2i/S3i are priced; and the owner asked for a
diagnosis of why we are slower than Jolt. `analysis.md` § 6 lists the open points PC-8b-1…PC-8b-11 and the owner
questions Q1–Q3 as the analyst wrote them.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The synthesis, verbatim. § 1 headline; Method; § 2 validity; § 3 pre-registered verdicts for every block; § 4 decisions implied (S7, S4, F3 and the thresholds, C1b, DM1, the S1 inputs, SPLIT); § 5 standing; § 6 open questions |
| `analysis/{s7,s4split,omega,f3,jsondm1}.md` | The group appendices, verbatim |
| `analysis/<group>/`, `analysis/synth/` | The analyst's scripts and their `.txt` / `.json` outputs, and each group's `verify_numbers/`. The scripts resolve their own location, and read `raw/`, `gate/fixtures/`, `rows8b.json`, `bin/SHA256SUMS` |
| `prep.md` | The tester's prep report the window was launched from, verbatim |
| `rows8b.json`, `rows8b.extra.json`, `rows8b.extra.example.json` | The run list (24 rows, protocol, binaries, blocks); the reserved blocks added on 2026-09-29 (22 rows); the template |
| `run_window8b.sh` | The launcher, which resolves everything relative to its own directory: the driver, then `tools/counts8b.py`, then `WINDOW_DONE` |
| `dryrun.txt`, `dryrun_s7.txt`, `dryrun_wgap.txt` | The dry-run schedules: the base prep, with S7-AB and omega-v2 added, and with omega(W, gap) added |
| `wait_log.txt`, `progress.txt`, `WINDOW_DONE` | The idle-rule poll log, the per-row pass completion lines, the completion marker |
| `bin/SHA256SUMS`, `bin/COMMIT.txt`, `bin/parent.patch`, `bin/g4ref.patch` | The binaries' sha256 and provenance, and the two one-line patches. No executable is in the tree |
| `logs/` | The build logs and their lock before/after hashes, the gate and rehearsal outputs, and `tools_pre_s7_20260929/`, `tools_pre_wgap_20260929/` (the tools as they were before the 2026-09-29 additions) |
| `gate/` | The untimed gate: `gate_*.json`, `gate_run.log`, `estimates.json`, `selftest8b.txt`; `fixtures/` (the seven `.pose` files and `fixtures.json`) and `fixture_runs/`, `cells/` (one directory per runner gate process), `s4/`, `s7/`, `f3pre/`, `red/`, `micro/`, `dm1/`, `criterion_list/`, `criterion_sample/`, `selftest_tmp/` |
| `raw/runs.jsonl.gz` | 1,106 records (914 processes, 171 R4 pose compares, 21 pass markers): args, env, exit, both receipts, the witness with its top-5, per-CPU busy, validity and contamination flags, the binary's hash, the placement receipt, the driver's summary. **Gzipped** (see below) |
| `raw/<block>-p<n>_172102/<seq>_r<k>_<row>_<bin>_W<w>[_warmup\|_R]/` | One directory per process (914; a re-run has the suffix `_R`): `stdout.txt`, `stderr.txt`, `run.csv` (armed rows carry the span columns); for DM1 the zone `artifact.toml`; for the criterion block the `criterion/` tree |
| `raw/counts.txt`, `raw/manifest_1790691662.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The untimed validity counts, the launch manifest, the run's start/end state, the window log, the driver's stdout, the launcher's log, the passes done, and the driver's exit status |
| `tools/` | `window8b_run.py` (the driver), `wait_idle8b.ps1`, `mkrows8b.py`, `gate8b.py`, `gate_dm1_8b.py`, `counts8b.py`, `micro8b.py`, `s7pre8b.py`, `selftest8b.py`, `patch_trees.py`, `build_runner.sh`, `build_g4ref.sh`, `joltprof.py`, `lib/driver.py`, `window_lib/pdhperf.py` |
| `skipped.sha256` | Every file of the scratch record that is not committed here: sha256, size, reason, path |

**`raw/runs.jsonl` is stored gzipped, as `raw/runs.jsonl.gz`.** The file is 9,846,958 bytes, over the 5 MB rule; the gzip
is 816,532 bytes (deterministic: `mtime` 0, level 9; sha256 `4905b3e35762c5b24a0bda044e9a4c079c10c90c2544c18909e1e02231337ee1`).
**The sha256 of the uncompressed file is `f9baa52ec4ada956e1f311630e71aab2af2af5dd4e9095a2fdc1d283e438b1db`.** The analyst's
scripts read `raw/runs.jsonl`; `gunzip` it in place before re-running them.

**Not committed** (each file's sha256 is in `skipped.sha256`):
- **The executables**, 5 files: the five exes of the binaries table that were built for this window. The two DM1 exes were
  never in this window's directory.
- **Pose dumps, 929 `pose.bin` files in `raw/` and `gate/`.** 925 are byte-identical to one of the committed fixtures, so
  `gate/fixtures/` keeps every distinct pose. The other four are gate controls that are not fixtures: two 501-step red
  controls (`gate/red/`, sha256 `6cdcbc31…`) and the two F3 pre-flight poses (`gate/f3pre/`, `a3811dbf…`, equal to each
  other).

  | sha256 prefix | `pose_hash` | = fixture | files in `raw/` | files in `gate/` |
  |---|---|---|---|---|
  | `e268d7a5` | `0x30c5438bc6ad9ffa` | JT500 = JTA500 = JSoffT500 = JA500 | 620 | 115 |
  | `11b83951` | `0x6cbe24bf8fafda26` | RT500 | 112 | 11 |
  | `205bc544` | `0x3db47fae414b655c` | JSonT500 | 49 | 5 |
  | `eff361e1` | `0x32d5e235342b4143` | JToff500 | — | 13 |

  Each process's `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`. The analysis scripts that
  compare pose bytes (`analysis/f3/lib_f3.py`, `r4.py`, `sel_f3.py`) read `pose.bin`; a process's `pose.bin` is the fixture whose
  hash its `stdout.txt` reports.
- **The rehearsal directory `test/`**, 1,319 files, which the prep cites (`test/rehearsal_*`, `test/window`). It is the driver's
  untimed `--test` output, not part of the record.
- **Bytecode caches**, 4 `.pyc` files under `analysis/`.
- No committed file exceeds 5 MB. The largest is `analysis/s4split/proc_s4split.json` (3.4 MB).

Also outside the tree: the exported trees `D:/wt/_targets/w8b-trees/*`, the DM1 worktree `D:/wt/_dm1_ab`, and every target
directory.

## How to re-run

1. **Build** into `bin/` and hash, with the commands in `bin/COMMIT.txt`: `tools/build_runner.sh <tree> <target> <log>`
   for the three runner builds (the tip, the parent, the S7 tip with `--bench omega_b_region`), and `tools/build_g4ref.sh` for
   the G4 bench. Every tree is a `git archive` export, never a worktree; the two patches are applied by `tools/patch_trees.py`.
   Quote each build's `Compiling` path.
2. **Gate:** `python -B tools/gate8b.py fixtures|rows|micro …`, `python -B tools/gate_dm1_8b.py` and
   `python -B tools/selftest8b.py` write `gate/`.
3. **Run the window:** `bash run_window8b.sh --cutoff HH:MM`, once, with no agent working. `--dry-run [--start HH:MM]` prints
   the schedule, `--test` is an untimed rehearsal under `test/`, `--resume` skips the passes in `raw/passes_done.txt`, and
   `--blocks a,b` limits the blocks. Creating `STOP` stops the driver before its next process.
4. **Reduce:** `raw/counts.txt` comes from `tools/counts8b.py`. The analyst's statistics come from `analysis/**/*.py`.
5. **Quote nothing that is not in `raw/`.**
