# Physics perf campaign, window 8 — the W8S instrument on the W8 gap, Jolt's critical path, omega_b, sleeping at cfg a, and DM1 C6 (2026-09-27)

This directory holds the receipts for:
- `docs/MEASUREMENT-QUEUE.md` section 16, the `RESULT, 2026-09-27, window 8` block;
- the orchestrator's rulings after the window, recorded below under **Rulings**. They are recorded in this directory
  only; this record does not edit `docs/physics/perf-campaign/levers/00-RULINGS.md`.

**Read `analysis.md` first.** It is the results-analyst's reduction, recomputed from `raw/` by the analyst's own
scripts (`analysis/`), with nothing taken from the driver's statistics. It is recorded verbatim. Its first line says
the orchestrator wrote it down from the analyst's returned report. The window was launched from the tester's prep
report, recorded verbatim as `prep.md`. The code review
of the instrument binary the window ran is `review-instrument-226bd99e.md`.

**How to read the references in `analysis.md`, `prep.md` and the review.**
- **The instrument commit `226bd99e` (`u/phys-w8s`) is not on the trunk** at the time of recording: the trunk
  `integ/unified` is `d95ee579`, and `git merge-base --is-ancestor 226bd99e d95ee579` fails.
  - Every code line the review cites is a line of `226bd99e`. A note at its top maps each bare file name to its path.
  - `analysis.md` cites no code line.
- "design", "design §N" and `01-DESIGN.md` mean `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` at
  `226bd99e`. That document is not on the trunk either.
- `analysis.md` says its sources were extracted read-only under `analysis/src/`. **That directory is not committed.**
  Each of its files is byte-identical to the commit's own file, so the commit is its record:

  | file (not committed) | sha256 | byte-identical to `git show 226bd99e:<path>` |
  |---|---|---|
  | `analysis/src/design_226bd99e.md` | `b74b5ffab3f0624e…` | `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` |
  | `analysis/src/omega_226bd99e.rs` | `ec93e52c1e625b8c…` | `crates/boyko_physics/benches/omega_b_region.rs` |
  | `analysis/src/runner_226bd99e.rs` | `69ce8a51c213c551…` | `crates/boyko_physics/benches/jolt_parity_pyramid.rs` |

  The full hashes are in `skipped.sha256`.
- Three inputs are named but are not in this tree. All three are in the orchestrator's scratch directory:
  - `RULINGS-2026-09-26` (`analysis.md` § 9): the orchestrator's rulings of 2026-09-26 on the next-lever designs
    (the scaling plan S1–S7, L8, L12, and the order of the physics lanes);
  - the W8S lane's cut `w8s-s4/cut.md`;
  - the DM1 lane's cut `dm1/cut.md`.

  The driver's header and `prep.md` cite the two cuts.
- "window 7 §N" and "window 7 FOLLOW-UP N" mean `docs/measurements/2026-09-25-physics-window7/analysis.md`, and
  "window 7b" or "wave 2" means `docs/measurements/2026-09-25-physics-window7/wave2/`.

## Headline

`analysis.md`'s verdict, restated with its numbers. Block W8S-A, K = 6, [0,500) unless marked:
- **Standing against Jolt 5.6.**
  - ours/Jolt on the wall is **0.449 at W1, claimed** (ours faster). It is 0.795 at W8 and 0.866 at W16, and neither
    is claimed, because min-max fails.
  - Per manifold over [100,500), each side counted with its own manifolds (ours 4,467.7, Jolt 8,489.0 from window 7's
    `-receipt`): 0.816 / 1.456 / 1.567. Only **W16 is claimed, and there Jolt is cheaper**.
- **The W8 loss is 70 % serial.**
  - Of the excess over T(1)/8 (1.529 ms): 70 % serial, 23 % parallel loss, 6 % interference.
  - Our serial chain is 1.310 ms against Jolt's 0.630 ms (2.08x).
  - The identity closes to +0.35 % after rule N1.
- **Jolt's jobs run strictly in sequence.** Design §4.2 is confirmed, and window 7 §5's reading that Jolt overlaps
  serial with parallel work is refuted. The lighter profile costs about 0–1 %, which closes window 7 FOLLOW-UP 10.
- **S4 is larger than designed.** The P-c fill share is f = 0.82, which puts S4 at about 0.19–0.22 ms (arithmetic).
- **S6 fails its build-if:** 3.5–3.8 % of T(8), against the 5 % the build-if needs.
- **omega_b as benched is 2.8–3.0 µs per stage at 8 participants,** three times the design's upper bracket. This bench
  cannot decide whether that is the primitive or the bench (`analysis.md` § 5).
- **No W ≥ 2 resolution was demonstrated under the two-spread rule.** W8S-R's pass 0 prevented it; pass 1 alone
  resolves 0.030 ms. The zone canary on `phys_solve_build` was not seen.
- **Sleeping at cfg a (J-Son against J-A)** over [100,500): −49.35 % at W1 and −35.65 % at W8, both claimed. The awake
  cost is not resolved.
- **DM1 C6 passes the letter of its gate (B ≤ A),** but the gate could not have seen a regression under about 23 %.
  VB_EARLY_CULL, which is outside the gated zones, reads B slower in all four pairs, and that is unexplained.

## What was measured

Five blocks, in this order: W8S-A, W8S-R, micro, DM1, P-jolt56-prof (`rows8.json`, `prep.md`). Every block runs K = 6
(two passes × three rounds), except DM1, which runs one ABBA pass.

| block | question | binaries |
|---|---|---|
| W8S-A | Ours against Jolt 5.6 at W 1/8/16. The identity at W8/W16, from armed/disarmed twins. Reuse on against off. J-A. Sleeping (J-Son) at cfg a | instr, j56 |
| W8S-R | The resolution: the canary ladder at W8/W16, the rung at W 1/2/4, the zone canary on `phys_solve_build`, each against its own reference rows inside the block | instr |
| micro | omega_b per stage (36 and 72 stages, both routes), and omega(W, gap) | omega |
| DM1 | DM1 C6, host-visible (A) against device-local (B) material table: GPU zones, ABBA per row, then 3× grow on B | dmA, dmB |
| P-jolt56-prof | Jolt's per-job walls and critical path, from the lighter profile plus the WaitingForBatch counter | j56p |

| row | block | args (+ `--workers W --steps 500 --window 0..500 --csv --pose-out --label --expect-pose gate/fixtures/<ref>.pose`; armed rows `--arm-profiler`; Jolt `-t=W -i=500`) | W | armed | pose ref / hash |
|---|---|---|---|---|---|
| `J-T` / `J-T-a` | A | `--scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off` (reuse on, the instrument's default) | 1, 8, 16 | no / yes | JT500 |
| `H-jolt56` | A | `-s=Pyramid -q=Discrete -f` | 1, 8, 16 | — | `0xb8522b4e3fc62cfe` |
| `J-T-off` / `J-T-off-a` | A | as `J-T`, `--contact-reuse off` | 1, 8, 16 | no / yes | JToff500 |
| `J-A` / `J-A-a` | A | `--scene jolt --gap 0.5 --cfg a` | 1, 8, 16 | no / yes | JA500 |
| `J-Son` / `J-Son-a` | A | `--scene jolt --gap 0.5 --cfg a --sleeping on` | 1, 8 | no / yes | JSon500 |
| `R-J-T` | R | as `J-T` (the ladder and rung reference) | 1, 2, 4, 8, 16 | no | JT500 |
| `ladder8-F{0.5,1,1.5,2}` | R | as `J-T`, `--canary-frac F --canary-ref-ns 60000` | 8 | no | JT500 |
| `ladder16-F{0.5,1,1.5,2}` | R | as `J-T`, `--canary-frac F --canary-ref-ns 105000` | 16 | no | JT500 |
| `rung` | R | as `J-T`, `--canary-frac 1 --canary-ref-ns 60000` | 1, 2, 4 | no | JT500 |
| `R-J-T-a` | R | as `J-T` (the zone canary's reference) | 8 | yes | JT500 |
| `zone-N30000` / `zone-N60000` | R | as `J-T`, `--canary-zone phys_solve_build --canary-ns 30000` / `60000` | 8 | yes | JT500 |
| `omega-b-s{36,72}-{worker,external}` | micro | `--bench --mode omega-b --participants 2,4,8,16 --stages 36`/`72` `--route worker`/`external` | — | — | — |
| `omega-{worker,external}` | micro | `--bench --mode omega --workers 8,16 --gap-us 0,5,20,80 --route worker`/`external` | — | — | — |
| `dm-{vb,deferred}-1920x1080-{idle,edit100}` | DM1 | `dm1_material_table_timing --exact --ignored --test-threads=1 --nocapture`; env `BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_ZONE=1 BOYKO_VB_BENCH_FRAMES=220 BOYKO_DM1_PATH BOYKO_DM1_RES BOYKO_DM1_EDIT_ROWS`, `BOYKO_PROFILE_ARTIFACT` per process | — | — | — |
| `dm-vb-1920x1080-grow150` | DM1 | the same, `BOYKO_DM1_GROW_AT=150`, B only | — | — | — |
| `P-jolt56-prof` | P | `-s=Pyramid -q=Discrete -f -p -wfb` | 1, 8, 16 | — | `0xb8522b4e3fc62cfe` |

Order inside a block: pass 0 runs W ascending, row by row inside a W group in the order above, so each armed row sits
beside its disarmed twin in the same round and W. Pass 1 is the whole list reversed. DM1 runs A B B A for each of its
four A/B rows (idle and the 100-row edit, on both paths), then 3× grow on B: 19 processes.

### Rows omitted, and why (`rows8.json` → `excluded`, `dm1_dropped`; `prep.md`)

- **S4-AB.** S4 (commit (4) of `u/phys-w8s`) was not built. It gets its own block in window 8b.
- **DM1 at 2560x1440 and 3840x2160, both paths (4 rows).** The display is 2560x1440 physical at 125 % scaling. Both
  requests came up 2052x1133, and the harness asserted NOT MEASURED (`gate/dm1_res.json`, exit 101).
- **The per-wave split rows.** The instrument review's B1 and B2 (below) make the split NO-GO on `226bd99e`, so it is
  read in window 8b on the fixed instrument. Amendment 3 added none.

### The orchestrator's amendments, and the deviations from the spec (`prep.md`)

- Amendments:
  1. omega_b runs at `--stages 36` and `72` on both routes, 4 rows. The slope is the per-stage cost (review rule N5).
  2. omega(W, gap) is unchanged.
  3. No per-wave-split rows.
  4. Armed/disarmed twins in the same block and round. J-Son was added disarmed, and both J-Son rows got W1, because
     D(W) needs the W1 term. That is 3 cells beyond the spec.
- Deviations:
  - **W8S-R carries its own references** (R-J-T at W 1/2/4/8/16 and R-J-T-a at W8), as window 7b's Q4 did, so each
    rung and each zone canary is compared inside one block.
  - **The J-A and J-Son fixtures come from the gate** (`--pose-out` there, `--expect-pose` on every timed process).
    The spec said pass 1.
  - Blocks run in priority order, and a block that does not fit before the cutoff is skipped whole (7b's rule). None
    was skipped.
- **DM1's binaries are dev-profile builds.** The cut's `cargo test --no-run` gives an unoptimized build with debuginfo.
  The A/B reads GPU zones; absolute values are not product numbers (`analysis.md` § 8).

## Protocol

Window 7's (`docs/measurements/2026-09-25-physics-window7/README.md`), with **the during-process witness now gated**
(window 7 FOLLOW-UP 8) and two new process kinds (`rows8.json` → `protocol`; `tools/window8_run.py` header).
- **Cell and claim rule.** A cell is the MEDIAN over K separate processes of each process's statistic.
  - Ours: the mean of `wall_ns` over the window.
  - Jolt: the mean of `Time (ms)` of `per_frame_discrete_thW.csv`.
  - Armed rows also keep every span column.
  - Spread: r = min–max, i = IQR, and s = 1.2533·SD/√K, each relative to the median. B against A is claimed iff
    |B/A − 1| > 2·√(sA² + sB²) under BOTH r and s; i is printed. There is no claim when either side has K < 3.

  **Ruling 1 below replaces this rule from window 8b on.** Window 8's own verdicts stay under it.
- **Idle rule before EVERY pass** (`tools/wait_idle8.ps1`: window 7's `wait_idle7.ps1` plus the window-8 target-dir
  exe names). It needs three consecutive 60-s polls with all of:
  - no build process;
  - no process whose image is under `D:/wt/_targets` or `D:/wt/mq-*`;
  - a 10-s CPU below 5 %.

  It allows up to 30 polls. A timeout STOPS the window (exit 3).
- **Receipts.** A 10-s receipt opens each pass, and a 5-s receipt sits between processes.
- **Re-run once at the end of the pass** for any of:
  - a receipt before or after above 5 %;
  - **the during-process witness `others_busy_pct` above 2 % (new in window 8)**;
  - an invalid process.

  The reduction drops a slot whose re-run is also hot. The slot rule is original if clean, else its re-run, else
  dropped. Clean means both receipts ≤ 5 % and the witness ≤ 2 %.
- **Void rule.** A build or lane process seen at any receipt, or started during a process, voids the whole pass.
- **Launch** P-none: suspended, the affinity mask read back, resumed; no affinity call.
- **Validity on every runner process:**
  - exit 0 and `void_steps` 0;
  - `expect_pose: match` against the row's fixture;
  - `workers` = W, `target_env` msvc, and the armed flag the row expects;
  - `canary_ns` on the ladder and rung rows, and `phys_solve_build` with the right N on the zone rows.
- **Micro:** the SUMMARY lines counted and route-checked.
- **DM1:** one windowed process at a time, validation off, each run bounded by its frame count. A process alive past
  300 s is a hang, terminated by its own handle. The harness asserts the client area after the run.
- **Jolt:** exit 0, one stat line with threads = W and hash `0xb8522b4e3fc62cfe`, and 500 frames. The profiled build
  also needs dumps at frames 100–400 and a `wfb_*.csv` with one job per thread per frame.
- **Hard stop.** No process starts whose estimated end passes the cutoff. The launcher ran with `--cutoff 21:00`
  (`raw/shell_log.txt`), and the cutoff was not reached. A `STOP` flag file was available and was not used.

## When and where

The owner's workstation (Ryzen 9 5900HS, 8C/16T; 16 logical CPUs and mask `0xffff` in every process). The power
scheme read High performance (`raw/window_state.json`). The build of every binary and the whole untimed gate came
before the launch.
- Builds: 18:08–18:17, 2026-09-27 +03:00 (`logs/`).
- The untimed gate, 18:20:32–18:36:52. The dry run was at 18:41, and the idle script's smoke test at 18:42.
- **Launch 18:45:27** (`bash run_window8.sh --cutoff 21:00`). Idle was reached at 18:47:37.
- **The timed window ran 18:47:47–19:55:18.** `WINDOW_DONE` was written at 19:55:23 and reads `exit 0` / `complete` /
  `counts exit 0`.
- `raw/window_state.json` reads `"status": "complete"`, `"exit": 0`, `"voided_processes": 0` and
  `"binaries_after": "all match"`.

The passes, from the first process start to the last process end (`raw/runs.jsonl`, `progress.txt`):

| block | pass 0 | pass 1 | processes (incl. one warm-up per pass) |
|---|---|---|---|
| W8S-A | 18:47:47–18:57:38 | 19:00:04–19:10:04 | 76 + 77 (one re-run) |
| W8S-R | 19:15:30–19:21:33 | 19:23:59–19:30:01 | 58 + 58 |
| micro | 19:32:27–19:34:22 | 19:36:47–19:38:36 | 20 (one re-run) + 19 |
| DM1 | 19:44:01–19:48:05 | — | 20 |
| P-jolt56-prof | 19:50:31–19:51:41 | 19:54:07–19:55:18 | 10 + 10 |

The total is **348 processes in 9 passes, 0 voided**.

## Binaries

| key | exe (sha256 prefix; `bin/SHA256SUMS`) | commit | what | built from | log |
|---|---|---|---|---|---|
| instr | `runner_226bd99e.exe` `c3cef91c` | `226bd99e` | `u/phys-w8s` commit (1): the W8S instrument (armed-only) and the omega_b bench; the `jolt_parity_pyramid` runner, reuse ON by default | `git archive` tree `D:/wt/_targets/w8s-trees/226bd99e`, never built in a worktree; `cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid` | `logs/build_instr.log`, `logs/build_instr_jolt_parity_pyramid.json` |
| omega | `omega_b_region_226bd99e.exe` `ffcac12f` | `226bd99e` | the `omega_b_region` bench | the same tree and target dir, `--bench omega_b_region` | `logs/build_instr_omega_b_region.json` |
| dmA | `dm1_A.exe` `47cb2c9b` | `cad5439b` | DM1 A: the trunk-sync merge into `u/dm1` (host-visible material table; `dm1/cut.md` § 11.5) | detached worktree `D:/wt/_dm1_ab`; `cargo test -p boyko-app --test dm1_material_table_timing --no-run --locked` (dev profile) | `logs/build_dm1_A.{log,json,stderr}` |
| dmB | `dm1_B.exe` `79bba864` | `97ee830f` | DM1 B: C6, the material table device-local | the same worktree checked out at `97ee830f` (one file, +9/−7), the same target dir | `logs/build_dm1_B.{log,json,stderr}` |
| j56 | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` `918fd2b7` | Jolt v5.6.0 (`wt-v5.6.0-parity`, the repository patch) | P0's Distribution build, windows 3 and 7's, run in place, not rebuilt | — | — |
| j56p | `jolt56prof8/PerformanceTest.exe` `b35986ef` | the same Jolt source, copied, plus `tools/patch_jolt_w8.py` | the lighter profile (three of window 7's profile scopes removed; `bin/COMMIT.txt`) and the WaitingForBatch per-thread counter (`-wfb` writes `wfb_<tag>.csv`) | window 7's recipe (`tools/build_jolt_prof8.sh`: WinLibs MinGW gcc, Distribution, `PROFILER_IN_DISTRIBUTION=ON`), out of source `D:/wt/_targets/w8-jolt56prof`; nothing written into the Jolt worktree | `logs/build_jolt56prof8.log` |

- **Toolchain:** rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`. `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=8`, and `RUSTFLAGS` unset (`x86-64-v3` comes from each tree's `.cargo/config.toml`).
- **The lock:** `--locked`, with each tree's own `Cargo.lock`. All three commits track the same lock, LF sha256
  `fab5f63fd1c85f8bd3766a736a60efd38b5a34c2171490fa7f70ae6696966ecc`, and it was unchanged after the builds
  (`bin/COMMIT.txt`).
- **The `Compiling` path.** `logs/build_instr.log` reads
  `Compiling boyko-physics v0.1.0 (D:\wt\_targets\w8s-trees\226bd99e\crates\boyko_physics)`, so the instrument compiled
  the exported tree of the commit it is named for.
- **dm1_A and dm1_B have different hashes.** The one file between them is `crates/boyko_render/src/material_table.rs`.
  The runtime-read files are identical at both commits.
- **The profiled Jolt build's MinGW DLLs** are byte-identical to window 7's and to `build-v5.6.0-dist`'s. The build was
  copied to `bin/jolt56prof8/`, so no timed image ran from under `D:/wt/_targets` (the void rule).
- **Every binary was re-checked against `bin/SHA256SUMS` after the window** (`"binaries_after": "all match"`). Each
  process's `exe_sha256` in `runs.jsonl` equals its key's hash.

## The idle rule, the receipts and the counts

**The idle-rule log** (`wait_log.txt`): 9 waits, one before each pass, and 33 polls.
- Seven waits reached idle in the minimum three polls.
- Two took six: before W8S-R-p0 (one poll at 7.88 %) and before DM1-p0 (one poll at 7.55 %).
- No wait timed out.
- **No poll saw a build process or a process under `D:/wt/_targets` or `D:/wt/mq-*`.** The process count was 225–238,
  and D: stayed at 216.59 GB free.

**Counts** (`raw/counts.txt`, `analysis.md` § Method):
- `raw/runs.jsonl` holds 357 records: 348 processes (9 warm-ups, 337 originals, 2 re-runs) and 9 pass markers.
  0 passes were voided, and no block was skipped.
- **All 337 slots are used**, two of them by their re-run. 0 are dropped, and there are 0 validity problems, 0 hangs
  and 0 DM1 rows NOT MEASURED among the rows that ran.
- **The two re-runs were both witness re-runs, the gate new in this window:**
  - `H-jolt56` W8, pass 1, round 2: witness 3.29 %, with `claude.exe` and `git.exe` in it. Its re-run read 0.23 %.
  - `omega-worker`, micro pass 0, round 2: witness 3.62 %. Its re-run read 0.0 %.

**Receipts:** 357 distinct, median 1.04 %, p90 1.99 %, max 4.35 %, and **0 over 5 %**.
- No build or lane process was present at any receipt or during any process.
- The during-process witness over all 348 processes: median 0.25 %, max 3.62 %.
- `claude.exe` tops the witness in 253 of the 348; the analyst counts 248 of its 339 timed processes. This is the
  desktop app's own activity. Ruling 11 addresses it.
- `analysis.md` § 10 lists the processes it reads as agent activity under the 2 % gate, and two slow Jolt W1 processes
  it reads as placement under P-none.

**The untimed gate** (18:20:32–18:36:52; `gate/gate_run.log`, `gate/gate_*.json`, `tools/gate8.py`,
`tools/gate_dm1.py`): every part passed.
- **Fixtures** (`gate/fixtures/`, `fixtures.json`):
  - JT500 `0x30c5438bc6ad9ffa` and JToff500 `0x32d5e235342b4143`, both equal to the spec's hashes;
  - JA500 `0x30c5438bc6ad9ffa` and JSon500 `0x3db47fae414b655c` (at W8), recorded.

  **All four `.pose` files are byte-identical to files already committed:**

  | fixture | sha256 prefix | already committed as |
  |---|---|---|
  | JT500, JA500 | `e268d7a5` | window 6's `gate/fixtures/JAon500.pose`, window 7's `JAon500c4.pose` / `JDon500c4.pose` |
  | JToff500 | `eff361e1` | window 7's J500 (window 3's P0 J pose) |
  | JSon500 | `205bc544` | window 6's `gate/fixtures/Offp-J1000.pose`, `2026-09-23-l10-sleeping/fixtures/J-Son_W{1,8}.pose` |
- **Runner: 105 of 105 pass** (21 rows × W 1/2/4/8/16, `--steps 500`, `--expect-pose`), 41 of them timed cells.
  - This is the re-run after the amendments, 18:33–18:36. The first pass, 100 of 100, is in the same log.
  - Every process: exit 0, the expected pose, `void_steps` 0.
  - Armed runs carry `w8s`; ladder and rung runs report `canary_ns`; zone runs report `phys_solve_build`.
  - `phys_setup_chunks` does not exist at `226bd99e`, so the spec's "reads 0" check does not apply.
- **Red controls** (`gate/red/`):
  - a 501-step run against JT500 exits 4 (`expect_pose: mismatch`, pose `0xcf93d4ea9c3e0922`);
  - J-T-off against JT500 exits 4.

  The pose gate can fail.
- **Micro:** the self-check is ok, the smallest configuration exits 0 on both routes, and 6/6 full-size dry samples
  exit 0 with 4/4/8 SUMMARY lines.
- **Jolt:**
  - j56 at W 1/8/16 reads hash `0xb8522b4e3fc62cfe`;
  - j56p at W 1/8/16 reads the same hash, with dumps 0–400 and 1/8/16 wfb jobs per frame.
- **DM1:**
  - the crash check at 1280x720 passes on A and B;
  - the resolution probe hosts only 1920x1080 (`gate/dm1_res.json`);
  - the 220-frame dry samples (vb idle, deferred idle, vb edit100, B grow150) pass.
- The per-process schedule estimates are `gate/estimates.json`. The dry run's schedule is `dryrun.txt`: about 66 min.

## Rulings

**The orchestrator's rulings after window 8, 2026-09-27.** They are recorded from the orchestrator's rulings file of
that date, which is not in this tree. Its inputs were this directory's `analysis.md`, `review-instrument-226bd99e.md`,
and the rulings of 2026-09-26.

**Standing** (window 8, block A, [0,500)): ours/Jolt 5.6 is 0.449x at W1 (claimed), 0.795x at W8 and 0.866x at W16
(not claimed under min-max). Per manifold it is 0.816x / 1.456x / 1.567x (W16 claimed, Jolt cheaper). The W8 excess
over T(1)/8 is 70 % serial: our serial chain is 1.31 ms against Jolt's 0.63 ms.

1. **The claim rule from window 8b on, pre-registered before any 8b data.**
   - The reason: two windows in a row lost a claim to single processes that ran uniformly 4–13 % slow with clean
     receipts, while the other pass resolved 0.030 ms. They are window 7 wave 2's Q1 and window 8's W8S-R pass 0.
   - From 8b: K = 9 per cell, over three passes (p0 reversed, p1, p2 reversed).
   - A claim needs i (IQR) AND s (SE) in every clean block and pooled. r (min–max) is reported beside it, and a claim
     that also passes r is marked STRONG. The resolution ladders are judged by the same rule.
   - The driver records a per-process placement receipt: the main thread's share on its top logical CPU. It is
     recorded, and never used to drop a process.
   - Window 8's own verdicts stay as analysed. Nothing changes retroactively.
2. **S4 stays first.**
   - Its bar stays the design's 0.060 ms (0.6 × 0.10).
   - Window 8 moved its expected gain to about 0.19–0.22 ms (f = 0.82).
   - The claim is taken in 8b's S4-AB block, with its own ladder, rung and zone canary.
3. **S6: NOT BUILT.** Its build-if (≥ 5 % of T(8)) fails: 3.5–3.8 % on the lower hit rate (J-T's). The design is frozen
   with this number. It is reopened only if a later window's hit rate moves it over 5 %.
4. **S7's partial form is PROMOTED:** the lanes capped at the physical cores on today's scope path (scale design §6.7).
   - It becomes a small lane right after S4. It touches the same solver files, so it comes after S4 merges.
   - Its window claim: W16 not slower than W8 on J-T and J-A, and W8 unchanged.
   - The engine reads the physical-core count per S7's ruling Q6.
5. **S1 + S2 + S3: the decision is DEFERRED to window 8b.** Two inputs come first:
   - an omega_b v2 re-bench (`analysis.md` § 9 item 3): about 0.7 µs of work per block, no shared `runs` RMW, padded
     claim and done words, blocks at 1× / 2× / 4× P, P capped at 8, and the spin-then-park variant of the
     2026-09-26 ruling 6;
   - B1 and B2 from the fixed instrument.

   omega_b v2 is a bench-only follow-up commit on `u/phys-w8s` after its verify.
6. **Warm apply is the largest serial stage after setup** (0.304 ms at W8).
   - Today it reaches the parallel path only through S2, on S1.
   - The S1 lane's design review must price an interim "S2i" against S2-on-S1, with window-8 numbers. S2i is warm apply
     on today's scope path, the S4 pattern.
   - The same holds for the integrate group (0.121 ms) and the graph (0.121 ms).
7. **F3 → tree C4 → S5 is unchanged.** S5 is supported: the query is 10.5 % of T(8), 0.210 ms.
8. **L10 C1b: no measurable awake cost.**
   - The settled pile reads −36 % at W8 and −49 % at W1, at cfg a.
   - The product row (sleeping on the TREE broadphase) goes into 8b: J-Son-T at W 1/8, over [0,100) / [100,500) /
     [274,500).
   - C1b's lane keeps its place, after the lanes that move no pins, because it moves every pose pin.
9. **DM1 C6: KEEP**, by the gate's letter.
   - 8b re-reads it with power:
     - ABBA × 2 per row (8 processes);
     - VB_EARLY_CULL and VB_RUN added to the gated zones;
     - the harness's non-FIFO present mode, if it has one.
   - If VB_EARLY_CULL's B−A stays above its band, C6 is dropped by sha. That is a follow-up revert commit on the trunk,
     not a history edit.
   - The grow frame stays NOT MEASURED. It is a reported number by design, and no harness work is planned for it.
10. **The Jolt re-read is closed.** Design §4.2 is CONFIRMED, and window 7 §5's "Jolt overlaps serial with parallel" is
    REFUTED. The lighter profile costs 0–1 %, which closes window 7 FOLLOW-UP 10.
11. **Window hygiene.** The desktop app's own `git.exe` and `claude.exe` activity shows in the witness: one re-run in
    window 8. The orchestrator issues no tool calls during a timed block beyond a progress read.

## Follow-ups: what window 8b must carry

From the rulings and `analysis.md` § 9. It is assembled after the lanes merge, and all of it runs under ruling 1.
- **S4-AB.** The parent is the w8s commit (3) and the tip is commit (4). It carries an in-block ladder at W8/W16, the
  rung at W 1/2/4, and the zone canary at 30/60 µs. It also needs a pre-registered handling of pass-0-style slow
  processes.
- **The per-wave split rows on the parent,** with the fixed instrument: J-T / J-T-a at W 1/8/16, for B1, B2, N3, N6,
  and N2's padded `TaskStamp`.
- **omega_b v2:** both routes, participants 2/4/8, stages 36/72, spin-only and spin-then-park.
- **omega(W, gap) with the N4 participation receipt.** Gap 5 is the awake baseline.
- **The F3 rows:** R1–R4 and a G4 block, from the tree lane's window commands (scratch `tree-f3/window_cmds.md`, not in
  this tree).
- **J-Son-T:** sleeping on the tree broadphase, the product row.
- **The DM1 re-read:** ABBA × 2, VB_EARLY_CULL and VB_RUN gated, non-FIFO present if available, and a display that
  hosts 1440p/2160p. The grow frame stays NOT MEASURED (ruling 9); `analysis.md` § 9 had asked for a per-frame grow
  record.

`analysis.md` § 9 lists the same items as the analyst wrote them, and the review defines B1, B2 and N1–N8.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The analyst's reduction, verbatim (see the notes at the top). § 1 standing against Jolt; § 2 the identity; § 3 per wave, route, histogram, S6; § 4 resolution; § 5 omega_b and omega(W, gap); § 6 Jolt's critical path; § 7 sleeping; § 8 DM1 C6; § 9 consequences for the lane order and window 8b; § 10 anomalies |
| `prep.md` | The tester's prep report the window was launched from, verbatim: binaries, omitted rows, amendments, deviations, estimated minutes, the untimed gate, a disclosure |
| `review-instrument-226bd99e.md` | The code review of the instrument commit `226bd99e`: verdict, the orchestrator's disposition, B1/B2 (blocking for the per-wave split), N1–N8 (reduction rules). A record note at its top pins its citations to `226bd99e` |
| `rows8.json` | The run list: `window`, `protocol`, `binaries`, `blocks` (with warm-up cells and DM1's explicit order), the 34 rows, `dm1_dropped`, `excluded` |
| `run_window8.sh` | The launcher, which resolves everything relative to its own directory: the driver, then `tools/counts8.py`, then `WINDOW_DONE` |
| `dryrun.txt` | The dry-run schedule (`run_window8.sh --dry-run`) |
| `wait_log.txt`, `progress.txt`, `WINDOW_DONE` | The idle-rule poll log, the per-row pass completion lines, the completion marker |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The binaries' sha256, including the Jolt exes and the three MinGW DLLs beside the profiled one, and each binary's commit, tree, command, target dir, exe path and lock. No executable is in the tree |
| `logs/` | The build logs: the instrument (text, and cargo's JSON messages for both benches), dm1 A/B (log, JSON, stderr), the profiled Jolt build |
| `gate/` | The untimed gate. `gate_run.log`; `gate_{runner,micro,jolt,dm1}.json`; `*_stdout.txt`; `estimates.json`; `dm1_res.json`; `wait_idle8_smoke.txt`. `fixtures/` holds the four `.pose` files every timed runner process asserted against, with `fixtures.json`, and `fixture_runs/` the runs that recorded them. `cells/` holds one directory per runner gate process (105). Also `red/`, `micro/`, `jolt/`, `jolt_cells/`, `dm1/`, and `j56p_w7_scopecount/`, the prep's scope-count run named for window 7's profiled build, which is not part of any gate count |
| `raw/runs.jsonl` | One record per process (357 with the pass markers): args, env, exit, both receipts, the during-process witness with its top-5, per-CPU busy, validity and contamination flags, the binary's hash, the driver's summary |
| `raw/<block>-p<n>_184527/<seq>_r<k>_<row>_<bin>_W<w>[_warmup\|_rerun]/` | One directory per process (348): `stdout.txt` and `stderr.txt`, plus the per-step `run.csv` (armed rows carry the span columns); for Jolt, `per_frame_discrete_thW.csv`; for the profiled Jolt, five `profile_chart_*_it{0,100,200,300,400}.html` dumps and `wfb_discrete_thW.csv`; for DM1, the zone `artifact.toml` |
| `raw/counts.txt`, `raw/manifest_1790523927.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The untimed validity counts, the launch manifest, the run's start/end state, the window log, the driver's stdout, the launcher's log, the passes done, and the driver's exit status |
| `analysis/` | The analyst's scripts (`lib8`, `sel8`, `common8`, `q0_misc`, `q1_headline`, `q2_identity`, `q2_run`, `q3_waves`, `q4_resolution`, `q5_micro`, `q6_jolt`, `jprof`, `q7_sleep`, `q8_dm1`), their tables (`q0.txt`–`q8.txt`, all of them in `tables8.txt`, and `sel8_out.txt`), and their JSON outputs (`proc8.json`, `q2_0..100.json`, `q2_100..500.json`, `q6.json`). They resolve this directory relative to their own location |
| `tools/` | `window8_run.py` (the driver: window 7 wave 2's `window7b_run.py` plus the witness gate and the micro, DM1 and profiled-Jolt kinds), `wait_idle8.ps1` (the idle rule), `mkrows8.py` (writes `rows8.json`), `gate8.py` and `gate_dm1.py` (the untimed gate), `counts8.py`, `joltprof.py`, `patch_jolt_w8.py` (the Jolt patch, applied to a copy), `build_instr.sh`, `build_dm1.sh`, `build_jolt_prof8.sh`, `lib/driver.py`, `window_lib/pdhperf.py`. `lib/driver.py`, `window_lib/pdhperf.py` and `joltprof.py` are byte-identical (line endings aside) to window 7's |
| `skipped.sha256` | Every file of the scratch record that is not committed here: sha256, size, reason, path |

**Not committed** (each file's sha256 is in `skipped.sha256`):
- **The executables and DLLs**, 8 files: the four runner/bench/DM1 exes, and the profiled Jolt exe with its three
  DLLs. Their hashes are also in `bin/SHA256SUMS`.
- **Every pose dump, 359 `pose.bin` files** (250 in `raw/`, 109 in `gate/`). Each is byte-identical to one of the
  committed fixtures, so `gate/fixtures/` keeps every distinct pose:

  | sha256 prefix | `pose_hash` | = fixture | files |
  |---|---|---|---|
  | `e268d7a5` | `0x30c5438bc6ad9ffa` | JT500 = JA500 | 277 |
  | `eff361e1` | `0x32d5e235342b4143` | JToff500 | 47 |
  | `205bc544` | `0x3db47fae414b655c` | JSon500 | 35 |

  Each process's `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`.
- **`analysis/src/`**, 3 files: the extracted sources, byte-identical to `226bd99e` (the table at the top).
- **Two more files:** `tools/__pycache__/joltprof.cpython-314.pyc`, and the empty `driver.log`.
- No committed file exceeds 5 MB. The largest is `raw/runs.jsonl` (4.4 MB).

Also outside the tree: the exported tree `D:/wt/_targets/w8s-trees/226bd99e`, the DM1 worktree `D:/wt/_dm1_ab`, the
Jolt source copy `D:/wt/_targets/w8-jolt56prof-src`, and every target dir.

## How to re-run

1. **Build** into `bin/` and hash.
   - `tools/build_instr.sh`, `tools/build_dm1.sh` and `tools/build_jolt_prof8.sh` hard-code the scratch directory as
     `W`. Point it at this directory.
   - The instrument is built from a `git archive` export of `226bd99e`, never in a worktree.
   - DM1 is built at `cad5439b` and `97ee830f`.
   - The profiled Jolt build needs `tools/patch_jolt_w8.py <copy>` on a copy of the Jolt v5.6.0 parity tree, never on
     the worktree.
   - Quote each build's `Compiling` path.
2. **Gate:** `python -B tools/gate8.py fixtures|runner|micro|jolt` and `python -B tools/gate_dm1.py` write `gate/`,
   including the fixtures and the red controls.
3. **Run the window:** `bash run_window8.sh --cutoff HH:MM`, once, with no agent working.
   - `--dry-run [--start HH:MM]` prints the schedule, and `--test` is an untimed rehearsal under `test/`.
   - `--resume` skips the passes in `raw/passes_done.txt`, and `--blocks a,b` limits the blocks.
   - Creating `STOP` stops the driver before its next process.
4. **Reduce:** `raw/counts.txt` comes from `tools/counts8.py`, which `run_window8.sh` runs after the last pass. The
   analyst's statistics come from `analysis/*.py`, which read only `raw/`, `gate/fixtures/`, `rows8.json` and
   `bin/SHA256SUMS`.
5. **Quote nothing that is not in `raw/`.** From window 8b on, a claim follows ruling 1, not the rule above.
