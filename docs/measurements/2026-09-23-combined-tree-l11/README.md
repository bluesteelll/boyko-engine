# Physics perf campaign, window 5 — the combined tip (tree broadphase + L11 C3) against Jolt v5.6.0, and L11 C3 against its parent under G9 (2026-09-23)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 13, the `RESULT, 2026-09-23, window 5` block, and for the
two dated lines of 2026-09-23 in `docs/physics/perf-campaign/levers/00-RULINGS.md` (the L11 block: G9 on C3;
the broadphase block: the headline with the tree, C4 still deferred). Read `analysis.md` first: it is the
results-analyst's reduction, recomputed from `raw/` with its own script (`tools/analyze_win5.py`), not from
the tester's. The design and the gate C3 was run under: `docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`,
section "Gates", item "G9".

`analysis.md` is verbatim from the text the analyst returned. The analyst's role could not write files, so
the text was not on disk in the scratch directory `win5/`; the recording step wrote it here unchanged, apart
from dropping two things that were not part of the document: the analyst's leading note to the orchestrator
and its trailing one-line `result:` summary. Its § 5 item 4 ("`analysis.md` was not written to disk") and its
closing "Files are in …" paragraph describe the scratch directory at the time of writing. Wherever it writes
`win5/<path>`, read `<path>` under this directory. Its other inputs are committed:
- `scratchpad/win4/` is window 4, `docs/measurements/2026-09-22-broadphase-tree/` (`f25ea2fa`);
- `scratchpad/win4b/` is window 4b, `docs/measurements/2026-09-22-l11-solve-setup/` (`4595dbfa`);
- `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/` is window 3, `docs/measurements/2026-09-21-physics-window3/`
  (`2ce03b66`; it came onto this lane with the merge `0ca312bd` and is unchanged since, `git diff 2ce03b66 HEAD`
  on that directory is empty).

There is no `window_report.md`: the tester's report was not written into the scratch directory. The tester's
reduction is `raw/analysis.json`, rendered as `raw/tables.md`, with the per-process receipts in
`raw/process_receipts.md`. `analysis.md` § 0 states that every number in it reproduces from `raw/`.

## What was measured

Two questions on one window.
1. **The headline**, on the tip only: `--scene jolt --gap 0.5 --cfg default` with `--broadphase tree` and with
   `--broadphase allpairs` (the shipped default) at W = 1, 2, 4 and 8, and `--scene rest --cfg default` both
   ways at W = 1 and 8.
2. **G9 on C3**: the parent against the tip on `J-As` (`--cfg as`) at W = 1 and 8, then the armed twins `J-As-a`
   (`--arm-profiler`, `--csv`) at W = 1 and 8 for the per-stage split. The bar is warm_apply -0.19 ms at W=1 on
   the armed row. The design's listed deltas are already the 0.6x realized-gain bars (window 4b adjudicated
   this) and are not discounted a second time.

**Protocol.** The window ran under the protocol ruled after P0 and used by windows 3, 4 and 4b (queue section
10, "Protocol ruling"):
- **Cell and claim rule.** A cell is the MEDIAN over K separate processes of each process's window mean
  (ms/step). Spread is the min-max range and the IQR, with the median's SE (1.2533·SD/√K) beside them. A
  comparison is claimed iff |effect| > 2·hypot(spread_A, spread_B). It is printed under all three readings,
  and this window requires the claim under BOTH min-max and SE; G9's own form is SE only.
- **K and ordering.** **K = 6** everywhere: two passes × three rounds per block. Tree / AllPairs and parent /
  tip alternate inside each W group, row by row. Pass 1 is the whole list reversed. Each pass starts with one
  untimed warm-up: tip `HL-D-tree` at W=8 for the headline, tip `J-As` at W=8 for g9 and armed.
- **Order of the blocks.** Headline, then g9, then armed.
- **Receipts.** A 5-s load receipt is taken before and after every process. If a receipt is > 5 % busy, the
  process is re-run once at the end of its pass. A 10-s receipt opens each pass. There is no band-based void,
  and placement is P-none.
- **Void rule.** A build process (cargo / rustc / link / miri / …), or any process running from
  `D:/wt/_targets` or `D:/wt/mq-*`, seen at any point of a timed pass VOIDS the pass. The idle rule is then
  re-run and the pass re-run in a new directory.
- **Idle rule** (`tools/wait_idle5.ps1`). Before every pass the machine must show three consecutive quiet
  60-s polls, up to 120 polls. A quiet poll has 0 build processes, 0 lane-target processes and a 10-s CPU
  below 5 %. The script prints its ancestor's name, `wait_idle3.ps1`, in the log header.
- **Pose check.** Every process ran with `--expect-pose gate/<J|R>_ref.pose` (the parent's untimed gate pose:
  J = parent `HL-D-allpairs` W1, R = parent `HL-R-allpairs` W1). Its printed `pose_hash` was compared with
  `rows.json`, and its TreeDiag with the row's arm.

**When and where.** The window was timed 00:40:17–03:12:50 +03:00 on the owner's workstation: Ryzen 9
5900HS, 8C/16T, High performance, AC (`raw/window_state.json`: power scheme and battery status at both ends).
The blocks that count ran as follows:
- headline 02:16–02:48;
- g9 02:51–03:04;
- armed 03:07–03:12;
- the make-up 03:13–03:16.

**Jolt was not run.** The owner's ruling of 2026-09-21 makes Jolt v5.6.0 the only reference, and its window-3
cells are reused, never re-run (see "What was reused from window 3").

| binary (sha256 prefix, `bin/SHA256SUMS`) | commit | tree / target dir | `Compiling boyko-physics` line | log |
|---|---|---|---|---|
| `runner_parent.exe` `3993684b` | `0ca312bd`: the merge of the integration line `f25ea2fa` into the lane. It carries L11 C0–C2 and the line's A1b, A3, hwrt shadow-origin, A4, A9/A9b, AH and tree broadphase C1+C3. **No C3**; `cbd86a65^` = `0ca312bd` | `git archive 0ca312bd \| tar -x` into the scratch `win5/parent_tree/`, with the tip's `Cargo.lock` copied in. The freshly resolved lock (`logs/parent_resolved_Cargo.lock.txt`; renamed from `.lock` because the root `.gitignore` ignores `*.lock`) differed only in `libredox` 0.1.24 → 0.1.25, and the copy recompiled nothing (`logs/build_parent_lockcopy.log`, 0.25 s). Target `D:/wt/_targets/l11-parent-msvc`, cold, 43.52 s | `(C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\d9f1cf70-4eed-44f3-9e33-2efbf030fc1a\scratchpad\win5\parent_tree\crates\boyko_physics)` | `logs/build_parent.log` |
| `runner_tip.exe` `9c7caff1` | `cbd86a65`: L11 C3, `warm_apply_avx2` per cohort under `simd_solve`. It is the HEAD of `perf/physics-l11-solve-setup` and pushed; `git status --short` was empty | `D:/wt/lighttable`; target `D:/wt/_targets/vkval-msvc`, 21.43 s (a warm target dir: `boyko-physics` was the only crate compiled) | `(D:\wt\lighttable\crates\boyko_physics)` | `logs/build_tip.log` |
| Jolt v5.6.0 Distribution `918fd2b7` | — | not run; window 3's cells reused (owner ruling) | — | — |

**How both runners were built:**
- The command was `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`. The
  `parity` profile inherits `release`: fat LTO, default CGU.
- Toolchain: rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`.
- `RUSTFLAGS` was unset. `x86-64-v3` comes from `.cargo/config.toml`, and `git diff 0ca312bd cbd86a65` touches
  neither that file nor anything outside `crates/boyko_physics`: three files, `solver/colored.rs`,
  `solver/colored_tests.rs` and `tests/colored_rigid_scratch_miri.rs`.
- Environment: the window's build prefix, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=6`, with `TMP`/`TEMP`
  under `D:/wt/_targets/tmp`. `bin/COMMIT.txt` records the toolchain, the profile and that no `RUSTFLAGS`
  were set.
- The zone tier is `dev`.
- The two binaries were re-verified against `bin/SHA256SUMS` after the window (`raw/window_state.json`:
  `"binaries_after": "all match"`).
- The parent's exported tree and its target dir were deleted by absolute path after the window; neither exists
  at record time.

| row | binaries | args (+ `--workers W --steps 500 --window 0..500 --csv --pose-out --expect-pose`) | W | armed | expected pose |
|---|---|---|---|---|---|
| `HL-D-tree` | tip | `--scene jolt --gap 0.5 --cfg default --broadphase tree` | 1, 2, 4, 8 | no | `0x32d5e235342b4143` |
| `HL-D-allpairs` | tip | `--scene jolt --gap 0.5 --cfg default --broadphase allpairs` | 1, 2, 4, 8 | no | `0x32d5e235342b4143` |
| `HL-R-tree` | tip | `--scene rest --cfg default --broadphase tree` | 1, 8 | no | `0xee2a67a98434919a` |
| `HL-R-allpairs` | tip | `--scene rest --cfg default --broadphase allpairs` | 1, 8 | no | `0xee2a67a98434919a` |
| `J-As` | parent, tip | `--scene jolt --gap 0.5 --cfg as` | 1, 8 | no | `0x32d5e235342b4143` |
| `J-As-a` | parent, tip | `--scene jolt --gap 0.5 --cfg as --arm-profiler` | 1, 8 | yes | `0x32d5e235342b4143` |

**The rows and their configs:**
- `HL-D-*` is window 4's `T-D-*`, `HL-R-*` is window 4's `R-*` (500 steps here), and `J-As` / `J-As-a` are
  window 4b's rows of the same names.
- `--cfg default` prints `PhysicsConfig::default()`: colored, `simd_solve` on, `parallel_solve` on,
  `parallel_broadphase` **off**, `parallel_narrowphase` on, AllPairs under `broadphase_select: Manual`,
  sleeping off, substeps 4, relax 2, `tree_brute_max_rows` 64. The tree rows override only the kind.
- `--cfg as` is the same with the three parallel flags = (W > 1).
- The printed config is field-equal between the two binaries in all 12 (row, W) pairs the gate compared
  (`logs/04_pose_gate.log`).

**Counts.** 16 cells (12 headline, 4 g9, 4 armed) × K = 6 make 120 slots. There were 25 re-runs, so the
protocol set is 145 timed processes plus 6 warm-ups. The three voided headline pass-0 attempts add 42 timed
processes, 3 warm-ups and 3 void markers, for 200 records in `raw/runs.jsonl`. 117 slots are used. 3 are
dropped, because the original and its one re-run were both contaminated, and those cells are K = 5:
- `HL-D-tree` W1;
- `HL-D-allpairs` W2;
- tip `J-As` W8.

The make-up (`raw/makeup/`, outside the protocol set) ran each dropped cell once more after the window. There
were 0 non-zero exits.

**Pose and structure.** Every one of the 196 processes that produced a result has:
- exit 0;
- the row's pose hash and `expect_pose: "match"`;
- void 0, `msvc`, `workers` = `pool_workers` = W, and mask `0xffff`;
- TreeDiag `static_rebuilds 1, members 1`, all else 0, on tree rows, and every field 0 on AllPairs rows;
- on armed rows, `drops 0`.

The counters are identical on both binaries (armed J rows: 4,524.25 manifolds per step over [0,500),
54,660 waves in total).

**Pose gate** (untimed; `logs/04_pose_gate.log`, `gate/gate.json`, `tools/pose_gate5.py`):
- 24 green runs: every row at W 1 and 8 on both binaries, with the tip checked against the parent's reference
  file.
- 6 red controls, 501 steps against the 500-step reference: `HL-D-tree`, `J-As` and `HL-R-tree` on each binary.
  All exit 4 with `mismatch`, at pose `0x9336b30a06a7d8af` (J) and `0xcd9134a5004429cc` (rest). The gate can
  fail.

**Contamination.**

*Receipts* (the 145 protocol processes):

| receipt | median | p90 | max | over 5 % |
|---|---|---|---|---|
| before | 2.70 % | — | 4.97 % | 0 |
| after | 2.95 % | 6.71 % | 13.20 % | 28 |

The 28 are the 25 hot originals plus the 3 re-runs that were hot again.

*During-process witness* (`others_busy_pct`) over the 117 used processes: median 1.48 %, p95 4.55 %, max
7.20 %. That is about 4× window 4b's 0.33 %.

*Voided attempts.* The headline block's pass 0 was voided three times before its fourth attempt
(`headline-p0-v3/`) counted:
1. 00:52:53, two `rustc.exe` present; 32 timed processes voided.
2. 00:57:39, eight `cargo.exe` appearing during a process; 6 voided.
3. 01:07:00, two `cargo.exe` and `D:\wt\_targets\joltab-msvc\debug\deps\ignore_reasons_census-….exe` present
   before a process; 4 voided.

*Idle waits.* There were ten (`raw/wait_log.txt`):
- one of 69 polls, 01:07–02:15, after the third void;
- three of 3–8 polls: the opening wait and the waits after the first two voids;
- six of 3–4 polls, before the five later passes and the make-up.

During the 69-poll wait the log names `cargo` / `rustc` / `link`, and test executables under `D:/wt/_targets`:
`m3_dirty_atlas`, `m5_scroll_atlas`, `taa_jitter_eval`, `production_reachability_census`, `c6_compile_fail`.
`analysis.md` § 5 item 3 attributes the voids and that wait to a rust-analyzer under the tester's host. The
names recorded here include test executables, which rust-analyzer does not launch, so at least part of the
load was another lane's test run.

## What was reused from window 3, and why

- **The Jolt v5.6.0 reference cells.**
  - The values: 9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms at W = 1 / 2 / 4 / 8 / 16, K = 6, from window 3's
    `JOLT56-T` rows, and 8,489.0 manifolds per step over [100,500) from its receipt
    `raw/receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv`.
  - Why: the owner's ruling of 2026-09-21. Jolt v5.6.0 is the only comparison, and its window-3 cells are the
    reference, never re-run.
  - Verified: the analyst recomputed every cell from window 3's per-frame CSVs, and they equal the ruled
    values (`analysis.md` § 0).
  - The cost: every Jolt ratio here is cross-window, and at W=8 it carries the bridge-2 qualification
    (`analysis.md` § 3).
- **The protocol block.**
  - What: the 2026-09-19 ruling as window 3 first ran it, with window 5's own additions (the pass-level void
    rule and the 120-poll idle wait).
  - Why: so that windows 3, 4, 4b and 5 are read with one statistic and one claim rule.
- **The tooling.**
  - What: the window-3 driver lineage, `window3_run.py` → `window4b_run.py` → `window5_run.py` (and
    `wait_idle3.ps1` → `wait_idle4b.ps1` → `wait_idle5.ps1`).
  - Kept unchanged in `tools/window3/`: window 3's `tools/`, all 17 files. They are identical in content to
    `docs/measurements/2026-09-21-physics-window3/tools/` in this tree, and byte-identical to its committed
    blobs; a working-tree checkout differs only in `core.autocrlf` line endings.
  - Kept unchanged at the top of `tools/`: window 4b's `window4b_run.py`, `wait_idle4b.ps1`, `pose_gate.py`
    and `reduce.py`, identical to `docs/measurements/2026-09-22-l11-solve-setup/tools/`.
  - `tools/lib/driver.py` is identical to this tree's `tools/physics_parity/driver.py` at `cbd86a65`.
  - Why: each copy sits in the record rather than being imported in place.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The reduction the queue block is built from: selection and input checks, the headline table under three readings, rest, per manifold, scaling, sensitivity, the W=8 budget, G9 on C3 (warm_apply under readings A and B, other spans, T), the two bridges, what cannot be claimed, open points |
| `rows.json` | The run list the window executed, with the protocol block, both binaries (exe, sha256, commit) and the expected pose per row |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The two runners' sha256 and the commit, tree and target dir each was built from. The executables are not in the tree |
| `raw/runs.jsonl` | One record per process (200): args, exit, receipts before/after, the during-process witness, the window mean, pose / `expect_pose`, structural counters, the binary's hash; the three `voided_pass` markers |
| `raw/headline-p0-v3/`, `raw/headline-p1/`, `raw/g9-p{0,1}/`, `raw/armed-p{0,1}/` | The protocol set, one directory per process: `stdout.txt`, `stderr.txt`, the per-step `run.csv` (armed rows carry the zone columns) |
| `raw/headline-p0/`, `raw/headline-p0-v1/`, `raw/headline-p0-v2/` | The three voided attempts of the headline's pass 0 (excluded from every cell; kept as the receipt of the voids) |
| `raw/makeup/` | The make-up for the three dropped slots (`runs_makeup.jsonl`, `makeup-a0/`, `MAKEUP_DONE`), outside the protocol set |
| `raw/manifest.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/WINDOW_DONE` | The window's manifest, its start/end state (power scheme, process count, voids, the binary re-check), its log, the idle-rule poll log, the completion marker |
| `raw/analysis.json`, `raw/tables.md`, `raw/process_receipts.md`, `raw/analysis_with_makeup.json`, `raw/tables_with_makeup.md` | The tester's reduction (`tools/reduce5.py`), its rendered tables and per-process receipts, and the same with the make-up variant (`logs/07_reduce_with_makeup.txt`) |
| `analyst/reduction.json`, `analyst/tables.txt` | The analyst's reduction (`tools/analyze_win5.py`) |
| `gate/` | The untimed pose gate: `J_ref.pose` / `R_ref.pose` (the files every timed process asserted against), one directory per gate run (24 green + 6 `_red501`), `gate.json` |
| `logs/` | The builds (`build_parent.log`, `build_parent_lockcopy.log`, `parent_resolved_Cargo.lock.txt`, `build_tip.log`), the pose gate (`04_pose_gate.log`), the window's stdout (`05_window_stdout.log`), the make-up's (`06_makeup_stdout.log`), the tester's reduction with the make-up (`07_reduce_with_makeup.txt`) |
| `tools/` | This window: `window5_run.py` + `wait_idle5.ps1` (the driver and idle rule), `pose_gate5.py`, `reduce5.py` (the tester's reduction), `makeup5.py`, `analyze_win5.py` (the analyst's). Window 4b's, unchanged: `window4b_run.py`, `wait_idle4b.ps1`, `pose_gate.py`, `reduce.py`. Window 4c's pose gate: `win4c/pose_gate.sh`, `win4c/tally.py`. Shared: `lib/driver.py`, `window_lib/pdhperf.py`. Window 3's, unchanged: `window3/` |

**Pose files: one per distinct pose, not one per process.**
- **`raw/` keeps one `pose.bin` for each of the two poses the window produced:**
  - J (sha256 `eff361e1…`, `pose_hash` `0x32d5e235342b4143`) in `raw/headline-p0-v3/001_r0_HL-D-tree_tip_W1/`;
  - rest (`89f084c2…`, `0xee2a67a98434919a`) in `raw/headline-p0-v3/003_r0_HL-R-tree_tip_W1/`.
- The other 198 process directories wrote byte-identical files. At record time the sha256 of all 200 was
  recomputed: every one is one of the two, J in 157 and rest in 43. Those 198 files are not committed; each
  process's `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`.
- **`gate/` likewise keeps four files:**
  - `J_ref.pose` and `R_ref.pose`;
  - one red-control pose per hash: J at 501 steps (`41297842…`) in `HL-D-tree_W1_parent_red501/`, and rest at
    501 steps (`2596fd90…`) in `HL-R-tree_W1_parent_red501/`.

  The other 28 gate `pose.bin` files duplicate one of those four and are not committed.

**Not kept in the tree:**
- the executables (their hashes are in `bin/SHA256SUMS`);
- the untimed rehearsals (`test/`: `probe/`, `wait_probe.txt`, `window_void_rehearsal/`);
- the parent's exported tree and its target dir (deleted; the parent binary survives only as its hash);
- `__pycache__/`;
- the duplicate pose files above.

## How to re-run

1. **Build the runners** into `bin/` and hash them.
   - The tip is this lane at `cbd86a65`. The parent is `git archive 0ca312bd | tar -x` into a scratch
     directory, with the tip's `Cargo.lock` copied in.
   - Use the build prefix `PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
     CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=6`, never `RUSTFLAGS`, and `cd` into the tree in the same command.
   - Build with `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`, one
     `CARGO_TARGET_DIR` per tree.
   - Quote each build's `Compiling boyko-physics (path)` line: the parent binary must come from the exported
     tree, the tip from the lane.
   - Copy the exes to `bin/runner_{parent,tip}.exe` and regenerate `bin/SHA256SUMS` and `bin/COMMIT.txt`.
   - Never re-run Jolt for a comparison against these rows. v5.6.0's window-3 cells are the reference.
2. **Gate the poses:** `python -B tools/pose_gate5.py` writes `gate/`, including `J_ref.pose` / `R_ref.pose`
   and the red controls. It resolves the directory from an absolute `SP` path; point that at this directory.
3. **Run the window:** `python -B tools/window5_run.py`. It resolves `rows.json`, `bin/`, `gate/` and `raw/`
   relative to its own directory.
   - It waits for the idle rule (`tools/wait_idle5.ps1`) before each pass, voids and re-runs a pass on any
     build or lane process, and writes `raw/`.
   - `WIN5_TEST=1` runs an untimed rehearsal under `test/`, and `WIN5_BLOCKS` selects blocks.
   - `python -B tools/makeup5.py` re-runs the dropped slots once, into `raw/makeup/`.
4. **Reduce:**
   - `python -B tools/reduce5.py [raw_dir]` writes `<raw_dir>/analysis.json` and prints the tables.
   - `python -B tools/analyze_win5.py` writes `analyst/`. Its `S` / `W3` constants are the absolute scratch
     and window-3 paths of the machine it ran on: point `W5` at this directory, `W4` / `W4B` at the window-4 /
     4b records and `W3` at `docs/measurements/2026-09-21-physics-window3/`.
5. **Quote nothing that is not in `raw/`.** The machine must be quiet.
   - The 5-s receipts gate each process, and the pass-level void rule gates build and lane processes.
   - A burst inside a process is visible only through the during-process witness (`others_busy_pct` in
     `runs.jsonl`). This window reports it but does not gate on it.
