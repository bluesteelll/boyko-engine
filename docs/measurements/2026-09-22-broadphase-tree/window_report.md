DONE — G4 complete (156 timed processes + 10 protocol re-runs, all exit 0, 0 excluded slots) AND G5 complete (156 timed processes + 2 warm-ups + 1 re-run, all exit 0, 0 contaminated, 0 excluded slots, 1 invalid process re-run: a W=1 AllPairs pose divergence, section 8.3), both on the quiet machine, 2026-09-22. The G4 section 1.2 stop rules that FIRED (stated caveat, orchestrator ruling 08:05 to run G5 regardless): `J snapshot` (`bp_g4_scene/tree/j100` 0.4429 ms > 0.30 ms), `compaction` at all three m (`compaction/m − eviction_filter/m` = 0.1607 / 1.966 / 24.27 ms vs 0.05 / 0.40 / 6.0 ms), `high_jumper` at m = 100000 (`high_jumper/100000 − stable/100000` = 5.239 ms vs 5.0 ms); every other 1.2 rule held. G5's tree-span gate input is also above its limit (0.4756 / 0.4662 ms vs 0.36 / 0.35 ms at W=1 / 8, section 8.6). C4's constants and the default flip are NOT decided from this window unless the analyst shows the fired rules do not touch them. G5 ran 10:16–10:44 after waiting for a concurrent timed window (`scratchpad/win4b`, section 8.1).

# Window 4 — tree broadphase C3, G4 + G5 — tester's report, 2026-09-22

Recipe: `scratchpad/treebp/g4_g5_recipe.md` (binding). Protocol block: window 3 (`D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/README.md`), its tools copied into `tools/`. Every path below is relative to `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4/` unless absolute. Nothing here is a claim; the analyst reduces. The first run of this stage (STOPPED 03:34, idle rule never met) did all the untimed preparation; its report's sections 2–5 are reproduced unchanged in sections 2–5 below, and its stop artefacts are under `raw/g4/stop1_0334/`.

## 1. Resume, receipts and the one deviation

- 03:39 — D: 83 GB free (`df`: 238G / 156G used / 83G avail); at the end (08:05) unchanged at 83 GB. `bin/SHA256SUMS` re-verified (`sha256sum -c`: `runner_c3.exe` OK, `broadphase-595045019352a67f.exe` OK, `row_identity_churn-ec1df0bd330016d8.exe` OK; mtimes 01:42–01:43, untouched). No rebuild. Commit a46b8287 (`bin/COMMIT.txt`).
- 03:40:39–03:42:49 — the standalone idle receipt (`tools/wait_idle3.ps1 -MaxPolls 33`, appended to `wait_log.txt` after the `# ---- RESUME 03:40` line): polls 1–3 cpu10 = **2.18 / 4.64 / 1.26 %**, build procs 0, lane-target procs 0 on all three, `# IDLE reached … after 3 polls`. No build process was present at any point of the run; nothing was killed.
- One edit to my own drivers before launch (`tools/g4_run.py:78`, `tools/window4_run.py:397`): `wait_idle` now takes a fresh 90-minute budget per call (`max(3, IDLE_BUDGET_S // 60)`) instead of the one-shot deadline set at process start, which would have left only the 3-poll minimum for every wait after the first ~90 min of a 4-hour run. Nothing about receipts, thresholds, order or statistics changed.
- 03:43:01 — `tools/g4_run.py` launched detached (pid 26184, `logs/05_g4_run.pid.txt`; stdout/stderr `logs/05_g4_run.{stdout,stderr}.txt`, the stderr file is empty). Its own idle rule (`raw/g4/wait_log.txt`) reached idle after 6 polls (poll 3 was 6.27 % — the Claude desktop app, see below); block A opened 03:48:22 on a 10-s receipt of 1.17 %.
- **Deviation (operational, reported not asked): the session's own UI host was the contaminant, and I minimized its window at 05:57:55.** Every contaminated receipt up to then named `claude.exe` pids 28784 / 28700 (the Claude desktop app from `WindowsApps\Claude_2.2553…`, NOT claude-code 30548) at 1.4–2.8 % of the machine each. Measured with `tools/claude_ui_cpu.ps1` (`logs/08_claude_ui_cpu.txt`): over 10 s with the window visible `28784 = 3.88 s, 28700 = 1.89 s` (3.6 % of 16 logical CPUs); after `ShowWindowAsync(…, SW_MINIMIZE)` on pid 34656 ('Claude') `28700 = 0.08 s`, 28784 absent. The consequence in the receipts: `others_busy_pct` during a timed process was **1.83–3.12 %** for the 31 processes that ended before 05:57:55 (all 13 of block A and block B seq 13–30 = k1 of the churn block) and **0.12–1.18 %** for the 135 after. Under the protocol both are "clean" (< 5 %); 9 of the 10 contaminated after-receipts (5.19–9.54 %) fall in the visible-window period. The window is still minimized at the time of writing (restore: `Get-Process claude | ? MainWindowHandle -ne 0 | % { [U.W]::ShowWindowAsync($_.MainWindowHandle, 9) }` after the `Add-Type` in `tools/claude_ui_cpu.ps1`); a future timed stage should minimize it first (`powershell -File tools/claude_ui_cpu.ps1 -Minimize`).
- My polling during the run: one `until [ -f raw/g4/G4_DONE ]; do sleep 60` loop per ~9 min, plus three short python/grep commands at 05:56–05:58 (the diagnosis above).

## 2. Section 0 — builds (done in the first run, green; not repeated)

Lane `D:/wt/mq-de06b6c9`, HEAD `a46b828797b5e4ff9881598fcc8c4a7e82dcd6ae` (`logs/01_commit.txt`), `git status --short` empty (`logs/01_status.txt`), `merge-base --is-ancestor ecbfe416 HEAD` = yes. Toolchain `stable-x86_64-pc-windows-msvc`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=6`, `RUSTFLAGS` unset, `-C target-cpu=x86-64-v3` on every rustc line (the `[target]` baseline), `TMP/TEMP=D:/wt/_targets/tmp`.

| build | log | receipt |
|---|---|---|
| `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid -v` | `logs/02_build_parity.log` | `Compiling boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics)`; rustc line: `-C opt-level=3 -C lto=fat ... -C target-cpu=x86-64-v3`; `Finished parity profile [optimized] target(s) in 17.02s`; `Executable D:/wt/_targets/mq-de06b6c9\parity\deps\jolt_parity_pyramid-39550833a2ead4ba.exe` |
| `cargo bench --no-run -p boyko-physics --bench broadphase --bench row_identity_churn -v` | `logs/03_build_bench.log` | `Compiling boyko-physics v0.1.0 (D:\wt\mq-de06b6c9\crates\boyko_physics)`; `Finished bench profile [optimized] target(s) in 19.08s`; `Executable D:/wt/_targets/mq-de06b6c9\release\deps\broadphase-595045019352a67f.exe`, `...\row_identity_churn-ec1df0bd330016d8.exe` |

`bin/SHA256SUMS`:

```
19456ab440e21d79132c8a98901698edeae7e80f6c816ce65aa2b374614b080b *runner_c3.exe
efb84636b2d52a904e052307d637e9ba5c905ef3a1e51c352260d70cbd56a6a0 *D:/wt/_targets/mq-de06b6c9/release/deps/broadphase-595045019352a67f.exe
d8ba73fc98c3e0355a09be29df57b149f8651170444fe461ee3d2800bb5c4c91 *D:/wt/_targets/mq-de06b6c9/release/deps/row_identity_churn-ec1df0bd330016d8.exe
```

Lane vs integration line (`logs/01_joltab_physics_diff.txt`): `git diff --stat a46b8287 c52ad183 -- crates/boyko_physics/src` = 10 files (+381/−158, the A1b RowIdentity merge); restricted to `src/broadphase_tree/` it is `tests.rs` only (+21/−5). The timed binaries are the lane's. The G4 driver verified both bench exes against `bin/SHA256SUMS` at 03:43:01 before timing (`raw/g4/g4_log.txt` line 1 carries the digests).

## 3. Section 2.2 — the pose gate and the receipts (done in the first run, untimed, all green; not repeated)

`tools/pose_gate.sh`; stdout/stderr/exit per process under `gate/`; the summary `logs/04_pose_gate.txt`; pose files `logs/04_pose_files_sha256.txt`. 28 processes:

| process | W | exit | pose_hash | expect_pose | TreeDiag (`broadphase_tree`) |
|---|---|---|---|---|---|
| `j_def_allpairs_w1` (`--pose-out pose_j_default.bin`) | 1 | 0 | `0x32d5e235342b4143` | none | all 0 |
| `j_def_tree_w{1,8,16}` | 1/8/16 | 0/0/0 | `0x32d5e235342b4143` ×3 | match ×3 | static_rebuilds 1, members 1, sleeper_rebuilds 0, evictions 0, translations 0, patches 0, hint_candidates 0, wide_rows 0, excluded_rows 0, locator_resets 0 |
| `j_def_grid_w{1,8,16}` | 1/8/16 | 0/0/0 | `0x32d5e235342b4143` ×3 | match ×3 | all 0 |
| `j_def_allpairs_w{8,16}` | 8/16 | 0/0 | `0x32d5e235342b4143` ×2 | match ×2 | all 0 |
| `j_def_tree_red501` (501 steps) | 1 | **4** | `0x9336b30a06a7d8af` | `mismatch: 64480 vs 64480 bytes, first differing body Some(0)` | static_rebuilds 1, members 1 |
| `j_a_allpairs_w1` (`--pose-out pose_j_a.bin`) | 1 | 0 | `0x32d5e235342b4143` | none | all 0 |
| `j_a_tree_w{1,8,16}` | 1/8/16 | 0/0/0 | `0x32d5e235342b4143` ×3 | match ×3 | static_rebuilds 1, members 1, rest 0 |
| `j_a_allpairs_w{8,16}` | 8/16 | 0/0 | `0x32d5e235342b4143` ×2 | match ×2 | all 0 |
| `j_a_tree_red501` | 1 | **4** | `0x9336b30a06a7d8af` | mismatch, body Some(0) | static_rebuilds 1, members 1 |
| `rest_allpairs_w1` (`--pose-out pose_rest.bin`) | 1 | 0 | `0xee2a67a98434919a` | none | all 0 |
| `rest_tree_w{1,8,16}` | 1/8/16 | 0/0/0 | `0xee2a67a98434919a` ×3 | match ×3 | static_rebuilds 1, members 1, rest 0 |
| `rest_allpairs_w8` | 8 | 0 | `0xee2a67a98434919a` | match | all 0 |
| `rest_tree_red501` | 1 | **4** | `0xcd9134a5004429cc` | mismatch, body Some(0) | static_rebuilds 1, members 1 |
| `s16_allpairs_w1` (`--pose-out pose_s16.bin`) | 1 | 0 | `0x71313833f6a8e645` | none | all 0 |
| `s16_tree_w1` | 1 | 0 | `0x71313833f6a8e645` | match | all 0 (the brute path: 17 rows ≤ `tree_brute_max_rows` 64) |
| `s16_tree_red501` | 1 | **4** | `0xa2dcb5083d89d453` | `mismatch: 832 vs 832 bytes, first differing body Some(0)` | all 0 |
| `usage_bad` (`--scene jolt --broadphase foo`) | – | **2** | – | – | stderr `jolt_parity_pyramid: --broadphase: expected allpairs|tree|grid, got Some("foo")` + the usage line |
| `self_check` (no `--scene`) | – | 0 | `0x2d9782c2598c835a` (3 steps, s16) | none | all 0 |

Facts the gate pins: J at `--cfg default`: tree = allpairs = grid = `0x32d5e235342b4143` at W 1/8/16 — window 3's `D-L5` hash; `pose_j_default.bin` sha256 `eff361e1bdf7261ddee5237cfbc4dbc3ad79d3a743a94fb2ebdee39c832e390f` = window 3's one identical pose file. J at `--cfg a` at 500 steps has the SAME hash and byte-identical pose file as `--cfg default` (the recipe's "cfg a rows have their own hash" holds only for its 20-step dry run, `0x58426687e8647451`). R: `0xee2a67a98434919a`; S16: `0x71313833f6a8e645`. Every SUMMARY carries `"tree_brute_max_rows":64`, `"target_env":"msvc"`, `void_steps 0`. TreeDiag gate (recipe 2.2 item 2): every J and R `tree` row `static_rebuilds == 1, members == 1`, all other counters 0; every `allpairs`, `grid` and `s16` row all 0 — 0 violations. The four 501-step red controls exit 4 (`expect mismatch`).

## 4. The tools

| file | what |
|---|---|
| `rows.json` | 11 rows / 26 cells / 156 timed processes + 2 warm-ups (K=6) for G5 (run 10:16–10:44, section 8): `T-A-tree`, `T-A-allpairs` (W 1,2,4,8,16); `T-D-tree`, `T-D-allpairs`, `R-tree`, `R-allpairs` (W 1,8); `S16-tree`, `S16-allpairs` (W 1); `T-A-tree-armed`, `T-A-allpairs-armed` (W 1,8, `--arm-profiler`); `T-C-tree` (W 1,8, armed, `canary_of: T-A-tree`). `sha256` = `runner_c3.exe`'s, `commit` = a46b8287, expected poses from the gate, `expect_pose_of` = the A twin. No Jolt row (v5.6.0's window-3 cells are read by the reducer from `.../2026-09-21-physics-window3/raw/analysis.json`; no v5.3.0 anywhere). |
| `tools/window4_run.py` (+ `window4_run.diff`) | window 3's `window3_run.py` with the four edits of the first run (`TREE_DRIVER`, `canary_of` threaded into `D.B(...)`, warm-up row `T-D-tree` W=8, `IDLE_BUDGET_S`) plus today's per-call idle budget. Everything else verbatim (P-none, 5-s/10-s receipts, > 5 % ⇒ re-run once at the end of the pass, two passes × three rounds, pass 1 reversed, binaries verified before each pass, `wait_idle3.ps1` before each pass). |
| `tools/g4_run.py` | the G4 driver that ran (section 6): block A = 12 whole-group processes (`broadphase.exe --bench '^<group>/' --save-baseline k<k> --noplot`, u d s m interleaved, K=3); block B = 144 churn processes (`row_identity_churn.exe --bench '^<cell>$' --save-baseline k<k> --noplot`, 24 cells, allpairs/tree alternating, order reversed on odd k, K=6); `CRITERION_HOME = raw/g4`; receipts and the during-process witness from `window4_run.py`; contaminated / voided ⇒ re-run once at the end of the block as `k<k>r` (the original kept); the idle rule before each block and before each block's re-runs. |
| `tools/reduce_g4.py` | section 7's tables: per cell the median over K of criterion `median.point_estimate`, min–max, range %, SE % (1.2533·SD/√K), per-k; the 1.2 stop rules with raw inputs; the churn claim inputs. Selection = window 3's rule (original if clean, else its re-run if clean, else excluded). Output `raw/g4/g4_reduction.json` (sha256 `eb5fd68d79141549c…`), the printed tables `logs/09_reduce_g4.txt`. |
| `tools/reduce_g5.py` | section 8's tables (window 3's statistic via `analyze_win3.py`); output `raw/g5_reduction.json`, printed `logs/13_reduce_g5.txt`. |
| `tools/pose_gate.sh`, `tools/claude_ui_cpu.ps1` | section 3; the UI-host CPU probe / minimizer of section 1 |
| `tools/wait_idle3.ps1`, `wait_flag.py`, `analyze_win3.py`, `render_tables.py`, `window3_run.py`, `lib/driver.py`, `window_lib/pdhperf.py` | window 3's, copied unmodified |

## 5. Untimed rehearsals (first run; structure only — NOT measurements)

`test/window/` (27 records, `WIN4_TEST=1`), `test/g4/` (`G4_TEST=1`), `rehearsal/` (one churn cell and the `bp_g4_scene` group at real criterion settings, loaded machine). The loaded-machine rehearsal medians that sat above a 1.2 limit were `tree/j100` 444 µs, `compaction − eviction_filter` 161 µs / 1.98 ms / 24.5 ms, `high_jumper − stable` at 100k 5.25 ms; today's quiet medians are 442.9 µs, 160.7 µs / 1.966 ms / 24.27 ms, 5.239 ms — the same numbers, so the loaded-rehearsal values were not load artefacts.

## 6. G4 run record (timed, quiet machine)

`raw/g4/g4_log.txt` (176 lines), `raw/g4/runs.jsonl` (166 records, one per process, sha256 `ce53ea4d94b0e09c0…`), `raw/g4/wait_log.txt` (the driver's four idle waits), `raw/g4/logs/` (332 files = stdout + stderr per process), `raw/g4/<group>/<arm>/<param>/k<k>[r]/estimates.json` (criterion), `raw/g4/G4_DONE` = `complete`.

| item | value |
|---|---|
| `G4 START` / `G4 END status complete` | 03:43:01 / 07:54:37 (+03:00) |
| idle waits (`raw/g4/wait_log.txt`) | before block A: 6 polls (poll 3 = 6.27 %, `claude(28784)=3.88s claude(28700)=2.5s`; polls 4–6 quiet) → idle 03:48:11; before A's re-run: 3 polls → 05:27:11; before block B: 3 polls → 05:38:11; before B's re-runs: 3 polls → 07:47:03. No build process, no lane-target process on any poll. |
| block A | 12 originals (seq 1–12, 03:48:22–05:25:01) + 1 re-run; wall per process: uniform 512–589 s, disparity 536–588 s, scene 485–565 s, maintenance 286–287 s; estimates per process 28 / 28 / 16 / 21; every exit 0 |
| block B | 144 originals (seq 13–156, 05:38:47–07:44:53) + 9 re-runs (07:47–07:54); wall 43.1–49.8 s each; 1 estimate each; every exit 0 |
| processes total / used / excluded slots | 166 / 156 / **0** (every contaminated original has a clean re-run) |
| `others_busy_pct` during timed processes (used) | 0.12–3.12 % (1.83–3.12 % before 05:57:55, 0.12–1.18 % after) |
| receipts of used processes | max `receipt_before` 4.67 %, max `receipt_after` 4.67 %; `build_procs_during` empty on all 166; `waited_s` = 25 s (one 20-s re-take of a busy before-receipt) on seq 6, 13, 14, 18, 21, 22, 24, 25, 30, 31, 156 |
| power / battery | `High performance`, `BatteryStatus 2, 100 %` at both block openings |

Contaminated originals (all `CONTAMINATED-after`, none `VOIDED-during`, none `before`) and what the 5-s after-receipt saw:

| block | k | seq | cell | end | after % | top of the receipt | re-run (`k<k>r`) |
|---|---|---|---|---|---|---|---|
| A | 2 | 5 | `bp_g4_uniform` | 04:31:03 | 6.50 | claude.exe 28700 2.52 %, 28784 2.48 % | 05:36:01, rb 1.71 % ra 2.36 % others 2.66 %, 28 estimates |
| B | 1 | 13 | `burst_migrate/sleeping_on_tree` | 05:39:36 | 5.33 | claude.exe 28784 2.01 %, 28700 1.41 % | clean |
| B | 1 | 17 | `first_archetype_spawn/sleeping_on_tree` | 05:43:30 | 9.54 | claude.exe 0.49 + 0.43 % (the rest not in top-5) | clean |
| B | 1 | 20 | `archetype_shift/sleeping_on_allpairs` | 05:46:32 | 5.30 | claude.exe 2.58 + 1.66 % | clean |
| B | 1 | 21 | `swap_churn/sleeping_on_tree` | 05:47:51 | 5.78 | claude.exe 2.32 + 1.82 % | clean |
| B | 1 | 23 | `stable/sleeping_on_tree` | 05:50:00 | 5.19 | claude.exe 2.50 + 1.45 % | clean |
| B | 1 | 24 | `stable/sleeping_on_allpairs` | 05:51:16 | 7.16 | claude.exe 1.80 + 1.72 % | clean |
| B | 1 | 29 | `first_archetype_spawn/sleeping_off_tree` | 05:56:10 | 6.68 | claude.exe 2.83 + 1.37 % | clean |
| B | 1 | 30 | `first_archetype_spawn/sleeping_off_allpairs` | 05:57:27 | 7.10 | claude.exe 2.25 + 1.41 %, powershell 20400 0.45 % (my probe) | clean |
| B | 6 | 155 | `burst_migrate/sleeping_on_allpairs` | 07:43:33 | 5.85 | nvcontainer.exe 8344 3.18 % (NVIDIA, transient) | 07:54:36, rb 0.37 % ra 1.07 % others 0.17 % |

All 9 block-B re-runs: exit 0, rb 0.29–1.5 %, ra 0.32–1.59 %, others 0.14–0.27 %. Per-k values of the used processes are in section 7's table; the k1 churn values that ran with the UI window visible are marked there by their seq (13–30) in section 1.

Structural receipts (every process's stderr, `receipts` in `runs.jsonl`; `logs/10_g4_facts.txt`): every cell carries ONE receipt line (two for `high_jumper` and the churn `_tree` cells, which print two) identical across all its processes (13 of 13 in block A, 153 of 153 in block B), and every count equals the recipe's dry-run table: `bp_g4_scene/1240` rows 1241 pairs 9570 members 1 static_rebuilds 1; `j100` pairs 9564; `10000` / `100000` pairs 83559 / 840494 members 1; `uniform/100000` 293501, `disparity/100000` 574763, members 0; `stable/{1240,10000,100000}` pairs 9464 / 83149 / 867834; `high_jumper` "the mover has 9 higher-row partners", timed step translations +1 patches +18 evictions +0 static_rebuilds +0; `compaction` evictions +1 static_rebuilds +1 members 620 / 5000 / 50000; `admission_of_64` static_rebuilds +1 members 1304 / 10064 / 100064; `admission_from_empty` static_rebuilds +1 members m; `shift_translation` translations +1 patches +0; `eviction_filter` evictions +1 members m−1; every `row_identity_churn/<non-stable>/*_tree` "translations +4 evictions +0 patches +0; static_rebuilds 1 members 1", `stable/*_tree` "translations +0 …"; `(oracle ok)` on every maintenance line.

## 7. G4 results (section 1.1 statistic: median over K of criterion `median.point_estimate`; range = (max−min)/median; SE = 1.2533·SD/√K / median; claim iff |ratio − 1| > 2·hypot(spread_A, spread_B) under both spreads). Full per-k table: `logs/09_reduce_g4.txt`; machine-readable `raw/g4/g4_reduction.json`.

### 7.1 The section 1.2 stop rules — raw inputs

| rule | cell | value | limit | fired |
|---|---|---|---|---|
| **J snapshot** | `bp_g4_scene/tree/j100` | **0.4429 ms** (k 0.4429 / 0.4440 / 0.4424; range 0.36 %, SE 0.13 %) | 0.30 ms | **YES** (+48 %) |
| Tree vs Grid | `uniform/{128,256,1000,10000,100000}` | tree/grid_w1 ratio 0.210 / 0.187 / 0.230 / 0.374 / 0.444 | tree claimed slower | no (tree claimed faster at every n) |
| Tree vs Grid | `disparity/{128,256,1000,10000,100000}` | 0.084 / 0.089 / 0.095 / 0.223 / 0.328 | ″ | no |
| Tree vs Grid | `scene/{1240,10000,100000,j100}` | 0.082 / 0.131 / 0.176 / 0.089 | ″ | no |
| stable | `stable/{1240,10000,100000}` | 32.69 µs / 0.2784 ms / 4.368 ms | 58 µs / 0.46 / 4.6 ms | no / no / no (100k at 95 % of the limit) |
| translation | `shift_translation − stable` | 22.73 µs / 0.1897 ms / 4.828 ms | 62 µs / 0.5 / 5.0 ms | no / no / no (100k at 97 %; `shift_translation/100000` k = 9.196 / 9.856 / 9.157 ms, range 7.6 %) |
| eviction | `eviction_filter − stable` | 13.48 µs / 0.1108 ms / 1.712 ms | 48 µs / 0.38 / 3.8 ms | no / no / no |
| **compaction** | `compaction − eviction_filter` | **160.7 µs / 1.966 ms / 24.27 ms** (`compaction/m` = 0.2069 / 2.355 / 30.35 ms, ranges 0.45 / 0.05 / 0.54 %; `eviction_filter/m` = 46.17 µs / 0.3893 / 6.08 ms) | 50 µs / 0.40 / 6.0 ms | **YES / YES / YES** (3.2× / 4.9× / 4.0×) |
| admission of 64 | `admission_of_64 − stable` | 36.48 µs / 0.2518 ms / 5.357 ms | 120 µs / 0.8 / 9.8 ms | no / no / no |
| from empty | `admission_from_empty − stable` | 0.4402 ms / 4.560 ms / 58.17 ms | 0.46 / 4.8 / 60 ms | no / no / no (96 % / 95 % / 97 % of the limits) |
| **high jumper (W1)** | `high_jumper − stable` | 27.22 µs / 0.2269 ms / **5.239 ms** (`high_jumper/100000` = 9.607 ms, k 9.549 / 9.607 / 9.620, range 0.73 %; `stable/100000` = 4.368 ms, k 4.457 / 4.279 / 4.368, range 4.07 % — the difference is 5.15–5.33 ms under stable's min–max) | 62 µs / 0.5 / 5.0 ms | no / no / **YES** (+4.8 %) |

Note for the analyst, not a claim: block A ran entirely with the UI host at 1.8–3.1 % `others` (section 1); the three fired rules are 48 % / 3.2–4.9× / 4.8 % over their limits, and the first-run loaded-machine rehearsal gave the same three values.

### 7.2 Pair finding (K=3 whole-group processes; medians; `all_pairs` vs `tree` with the claim under both spreads)

| family | n | all_pairs | grid_w1 | grid_w8 | tree | tree/all_pairs | claim bar range / SE % | tree claimed faster | all_pairs claimed faster |
|---|---|---|---|---|---|---|---|---|---|
| uniform | 17 | 0.182 µs | 3.105 µs | 3.103 µs | 1.771 µs | 9.729 | 5.5 / 2.1 | no | **yes / yes** |
| uniform | 64 | 2.767 µs | 47.69 µs | 47.56 µs | 4.471 µs | 1.616 | 0.9 / 0.4 | no | **yes / yes** |
| uniform | 128 | 11.74 µs | 49.08 µs | 48.96 µs | 10.32 µs | 0.879 | 0.9 / 0.3 | **yes / yes** | no |
| uniform | 256 | 43.93 µs | 0.1178 ms | 0.1177 ms | 22.00 µs | 0.501 | 1.7 / 0.6 | yes / yes | no |
| uniform | 1000 | 0.6343 ms | 0.6651 ms | 0.6661 ms | 0.1531 ms | 0.241 | 6.2 / 2.2 | yes / yes | no |
| uniform | 10000 | 75.67 ms | 8.069 ms | 3.440 ms | 3.022 ms | 0.040 | 5.1 / 1.8 | yes / yes | no |
| uniform | 100000 | 12020 ms (k 16880 / 11750 / 12020, range 42.7 %) | 89.69 ms | 36.41 ms | 39.85 ms | 0.003 | 85.3 / 34.8 | yes / yes | no |
| disparity | 17 | 0.290 µs | 7.290 µs | 7.312 µs | 2.323 µs | 8.011 | 1.2 / 0.5 | no | **yes / yes** |
| disparity | 64 | 3.499 µs | 52.60 µs | 52.55 µs | 6.996 µs | 1.999 | 2.4 / 1.0 | no | **yes / yes** |
| disparity | 128 | 13.89 µs | 0.1756 ms | 0.1762 ms | 14.68 µs | 1.057 | 3.3 / 1.2 | no | **yes / yes** (+5.7 % vs bar 3.3 %) |
| disparity | 256 | 48.44 µs | 0.3513 ms | 0.3517 ms | 31.40 µs | 0.648 | 2.5 / 0.9 | **yes / yes** | no |
| disparity | 1000 | 0.6547 ms | 2.439 ms | 2.445 ms | 0.2306 ms | 0.352 | 2.7 / 1.1 | yes / yes | no |
| disparity | 10000 | 75.94 ms | 15.93 ms | 8.649 ms | 3.557 ms | 0.047 | 4.1 / 1.6 | yes / yes | no |
| disparity | 100000 | 14850 ms (range 20.5 %) | 139.3 ms | 83.61 ms | 45.73 ms | 0.003 | 41.3 / 16.2 | yes / yes | no |
| scene | 1240 | 1.849 ms | 5.411 ms | 5.411 ms | 0.4415 ms | 0.239 | 0.6 / 0.2 | yes / yes | no |
| scene | j100 | 1.854 ms | 4.981 ms | 4.982 ms | 0.4429 ms | 0.239 | 0.9 / 0.3 | yes / yes | no |
| scene | 10000 | 123.4 ms | 36.79 ms | 14.52 ms | 4.807 ms | 0.039 | 0.6 / 0.2 | yes / yes | no |
| scene | 100000 | 17630 ms (range 26.3 %) | 336.3 ms | 128.1 ms | 59.34 ms | 0.003 | 52.7 / 21.8 | yes / yes | no |

Spreads (range %): every `tree` cell ≤ 3.0 % (`uniform/tree/1000` 3.01, `/17` 2.76, `disparity/tree/100000` 2.68, the rest ≤ 1.2); every `grid_w1` ≤ 1.0 %; `grid_w8` ≤ 4.4 %; `all_pairs` ≤ 2.5 % except the three 100k cells (20–43 %, 10 samples of 12–22 s each). Crossover inputs for 1.4 (`TREE_BRUTE_MAX_ROWS`, `AUTO_TREE_LO/HI`): `all_pairs` claimed faster at n = 17, 64 (uniform) and 17, 64, 128 (disparity); `tree` claimed faster from n = 128 (uniform) and 256 (disparity); the grid has no point between 64 and 128 or 128 and 256.

### 7.3 Maintenance (K=3; medians; per-k in `logs/09_reduce_g4.txt`)

| arm \ m | 1240 | 10000 | 100000 |
|---|---|---|---|
| stable | 32.69 µs (range 3.44 %) | 0.2784 ms (2.26 %) | 4.368 ms (4.07 %) |
| admission_from_empty | 0.4729 ms (3.32 %) | 4.838 ms (2.94 %) | 62.54 ms (3.00 %) |
| admission_of_64 | 69.17 µs (2.83 %) | 0.5302 ms (0.72 %) | 9.725 ms (3.94 %) |
| eviction_filter | 46.17 µs (0.86 %) | 0.3893 ms (0.36 %) | 6.080 ms (2.34 %) |
| shift_translation | 55.42 µs (2.36 %) | 0.4681 ms (0.56 %) | 9.196 ms (7.61 %) |
| compaction | 0.2069 ms (0.45 %) | 2.355 ms (0.05 %) | 30.35 ms (0.54 %) |
| high_jumper | 59.91 µs (0.67 %) | 0.5053 ms (0.33 %) | 9.607 ms (0.73 %) |

### 7.4 Churn (K=6 per cell, one cell per process; medians; the G4 churn claim inputs)

| arm | sleeping | allpairs | tree | tree/allpairs | tree claimed faster (range / SE) | tree(arm) − tree(stable) |
|---|---|---|---|---|---|---|
| stable | off | 11.59 ms (1.52 %) | 9.606 ms (1.93 %) | 0.829 | yes / yes | 0 |
| swap_churn | off | 11.88 ms (1.30 %) | 9.769 ms (2.12 %) | 0.822 | yes / yes | +0.163 ms |
| archetype_shift | off | 11.58 ms (1.56 %) | 9.694 ms (3.04 %) | 0.837 | yes / yes | +0.088 ms |
| first_archetype_spawn | off | 11.86 ms (1.74 %) | 9.874 ms (2.37 %) | 0.833 | yes / yes | +0.268 ms |
| burst_despawn | off | 12.00 ms (5.31 %) | 9.653 ms (6.26 %) | 0.804 | yes / yes | +0.047 ms |
| burst_migrate | off | 11.95 ms (6.80 %) | 9.755 ms (8.54 %) | 0.817 | **no (range) / yes (SE)** | +0.149 ms |
| stable | on | 5.739 ms (1.18 %) | 4.043 ms (0.46 %) | 0.704 | yes / yes | 0 |
| swap_churn | on | 5.803 ms (8.41 %) | 4.120 ms (1.54 %) | 0.710 | yes / yes | +0.077 ms |
| archetype_shift | on | 5.994 ms (3.66 %) | 4.183 ms (10.72 %) | 0.698 | yes / yes | +0.140 ms |
| first_archetype_spawn | on | 5.830 ms (9.20 %) | 4.150 ms (1.51 %) | 0.712 | yes / yes | +0.107 ms |
| burst_despawn | on | 5.814 ms (9.26 %) | 4.130 ms (8.22 %) | 0.710 | yes / yes | +0.086 ms |
| burst_migrate | on | 5.832 ms (11.05 %) | 4.190 ms (0.93 %) | 0.718 | yes / yes | +0.147 ms |

Per-k (ms) for the cells whose range exceeds 5 %, with the k1 process (seq 13–36, block B's first round; seq 13–30 ran with the UI window visible) first: `burst_despawn/off_allpairs` 12.40 12.10 11.98 11.76 12.00 12.00; `burst_despawn/off_tree` 10.22 9.612 9.797 9.638 9.634 9.668; `burst_migrate/off_allpairs` 12.64 11.83 11.95 11.86 12.07 11.94; `burst_migrate/off_tree` 10.55 9.714 9.757 9.754 9.92 9.746; `swap_churn/on_allpairs` 6.27 5.783 5.782 5.827 5.793 5.812; `archetype_shift/on_tree` 4.589 4.161 4.152 4.21 4.205 4.141; `first_archetype_spawn/on_allpairs` 6.348 5.877 5.816 5.812 5.842 5.817; `burst_despawn/on_allpairs` 6.33 5.826 5.80 5.802 5.854 5.791; `burst_despawn/on_tree` 4.452 4.135 4.113 4.126 4.12 4.133; `burst_migrate/on_allpairs` 6.464 5.843 5.836 5.827 5.82 5.824(k6r). In each the k1 value is the maximum; the k1 slots of these cells were "clean" by the receipts (their `others` 1.8–2.5 %) and are used as-is by the window-3 selection rule. The analyst decides what to do with k1 of block B.

### 7.5 t_q inputs (1.3) and C4 inputs (1.4) available without G5

- t_q at J needs `T-A-tree-armed` (G5) — now measured, section 8.6: 0.414 / 0.411 ms at W=1 / 8 (median over [100,500), then over K). The bench upper bound on the tree step at J scale is `bp_g4_scene/tree/j100` = 0.4429 ms and `/1240` = 0.4415 ms (verify + build + query + assembly); at 10k `bp_g4_scene/tree/10000` = 4.807 ms, `bp_g4_uniform/tree/10000` = 3.022 ms.
- `ADMIT_BUILD_RATIO` inputs at m = 10k: c_build = (4.838 − 0.2784) ms / 10000 = 0.456 µs per row; c_list = (0.3893 − 0.2784) ms / 83149 = 1.33 ns per list entry; k = 83149 / 10000 = 8.31; c_q from t_q(J) = 0.414 ms / 1240 rows = 0.334 µs per row (section 8.6) — the bench check value `bp_g4_uniform/tree/10000` / 10000 = 0.302 µs per row.

## 8. G5 — run record and results (timed, quiet machine; orchestrator ruling 08:05: run G5 despite the 1.2 stop rules)

### 8.1 Timeline, the idle receipts, and two deviations (reported, not asked)

All times +03:00. `wait_log.txt` (standalone receipts and the pointers), `raw/wait_log.txt` (the driver's two idle waits), `raw/window_log.txt` (173 lines), `logs/06_window4_run.{pid,stdout,stderr}.txt` (stderr empty), `logs/11_claude_ui_cpu_g5.txt`, `logs/12_idle_receipt_g5.pid.txt`.

- 08:03 — D: 83 GB free; `sha256sum -c bin/SHA256SUMS`: `runner_c3.exe` OK, both bench exes OK (re-checked 10:13, OK; the driver verified them before each pass and after the window: `binaries_after: all match`). No rebuild.
- 08:04:15 — the UI host window had been restored since 05:57 (`IsIconic` false; pids 28784 / 28700 = 6.8 CPU-s over 10 s, about 4.2 % of 16 CPUs); minimized per section 8's launch recipe (28784 gone, 28700 2.02 → 0.09 s).
- 08:04:35–08:17:46 — the standalone idle receipt (`wait_idle3.ps1 -MaxPolls 90`, appended to `wait_log.txt` after `# ---- RESUME 08:04:35`): **IDLE after 14 polls**; polls 12–14 cpu10 = **3.33 / 2.19 / 3.11 %**, build procs 0, lane-target procs 0. What the earlier polls saw: poll 3 (08:06:36) **8 build processes** (`cargo` ×2, `rustc` ×6; cpu10 54.76 %); polls 9–11 `runner_parent` / `runner_tip` (another window's parity runners, see below); poll 7 a `python(38344)` at 10 s; the UI host at 2–4.5 s per 10 s on polls 2–7 (its window was restored again within a minute of 08:04:28 — not by me).
- **Deviation 1 — a second timed window was running, and I waited for it.** 08:19:13 the G5 driver was launched (pid 28248) after re-minimizing the UI host (08:18:49–08:19:13: 28784 3.91 s + 28700 3.48 s → none). Its idle wait saw, from poll 3, `runner_tip` / `runner_parent` processes: `scratchpad/win4b/tools/window4b_run.py` (pid 18716, another agent's window, launched 08:18:13: 30 cells × 2 passes × 6 rounds = 360 main + 48 armed + 2 canary timed processes, each block behind its own idle rule). Those runners live under the scratchpad, so the lane-target rule (`D:\wt\_targets`, `D:\wt\mq-*`) cannot see them, and their W=1/W=2 cells (about 5 s wall, 1–2 cores, 5-s receipts between) read 3–4 % on a 10-s poll — the rule could have passed by chance and both windows would have been timed over each other's processes. **08:29:54 I stopped my driver (pid 28248 and its `wait_idle3` child; nothing had been timed — it was still in its first idle wait)** and parked that launch's `manifest.json` / `window_log.txt` / `wait_log.txt` under `raw/launch1_aborted_0819/`. `win4b` wrote `WINDOW_DONE` (status complete) at 10:12:24. My 90-minute idle budget, counted from 08:30, would have expired at 10:00 with `win4b`'s armed block still running; I extended the wait to its completion (a known, finite, orchestrator-launched noise source, and the owner's window is "at least" to 10:50), hard cap 10:30. During the wait my only activity was a 30-s `sleep` poll loop on its log.
- 10:13:16 — no `runner_*` / `python` / `cargo` / `rustc` process; sha256 OK; 10:13:30 the UI host was already quiet (30548 0.14 s; 34656 minimized again); **10:13:54 the G5 driver relaunched (pid 6796)**.
- **Pass 0**: idle after 3 polls (cpu10 **2.06 / 1.20 / 0.95 %**, 0 build, 0 lane-target) at 10:16:05; binaries verified; opening 10-s receipt **1.18 %** (build procs none); warm-up `T-D-tree` W=8 (untimed) 10:16:16–10:16:23, 3.678 ms; 78 originals 10:16:23–10:28:57; **0 contaminated, 0 invalid, no re-runs**.
- **Pass 1**: idle after 3 polls (**1.83 / 1.65 / 1.38 %**) at 10:31:07; opening receipt **0.69 %**; warm-up 3.730 ms; 78 originals (reversed order) 10:31:24–10:44:00; **1 invalid (seq 75, section 8.3), re-run 10:44:00–10:44:05 valid**; `WINDOW END` 10:44:10, status `complete`.
- Machine: `High performance`, `BatteryStatus 2, 100 %` at both pass openings and at the end; process count 241–247.
- **Deviation 2 (the UI host)**: minimized three times (08:04:28, 08:19:13, 10:13:54; `logs/11_claude_ui_cpu_g5.txt`), restored twice by something other than me; during the timed passes `claude.exe` appears in `others_top5` of 196 records at ≤ 0.1 s each (window minimized) — the run itself was not affected. The window is minimized at the time of writing.

### 8.2 Load receipts and the process census

`raw/runs.jsonl` — 159 records (sha256 `d754008c3f6688a45dd1…`): 2 warm-ups + 156 originals + 1 re-run; **used 156, excluded slots 0** (window-3 selection: original if clean and valid, else its re-run). `raw/pass-00/`, `raw/pass-01/` (one directory per process: `stdout.txt`, `stderr.txt`, `run.csv`, `pose.bin`). `raw/receipts/` is empty (no Jolt row — v5.6.0 is read from window 3, never re-run; no v5.3.0 anywhere).

| item | value |
|---|---|
| exits | every process 0 |
| `receipt_before` / `receipt_after` (5 s) over all 159 | **0.44–3.97 %** / **0.44–3.97 %**; `build_procs_busy` empty on every receipt; contaminated 0 |
| `waited_s` | 0 on every process (no busy before-receipt) |
| build process during a timed process (`others_top5` names) | **none** on all 159 (the task's void rule: no pass voided) |
| `others_busy_pct` during timed processes | 0.0–**6.49 %**; one process > 3 %: pass 1 round 0 seq 6 `R-allpairs` W=8 (10:32:04–08), `python.exe` pid 16036 = 3.75 CPU-s during its 3.75-s wall (not mine — my only activity was `sleep`; unidentified, gone by 10:44); its receipts 1.92 / 1.10 % ⇒ used by the protocol; its mean 7.440 ms is the cell's max (the other five 6.69–7.00) |
| `others_top5` names over the window | `claude.exe` (196 records, ≤ 0.1 s), `steamwebhelper.exe` 91, `steam.exe` 87, `Telegram.exe` 61, `nvsphelper64.exe` 41, `browser.exe` 37, `explorer.exe` 31, `nvcontainer.exe` 16, `NVIDIA Overlay.exe` 16, `AmneziaVPN.exe` 16, `backgroundTaskHost.exe` 7, `sleep.exe` 3 (my poll loop) |
| wall per process | 0.029 s (S16) – 9.914 s (T-A W=1) |
| `mask_readback` | `0xffff` on every process (P-none, 16 logical CPUs) |

### 8.3 Pose receipts (recipe 2.2 items 1–2 on the timed rows; reducer `gate_inputs`)

- Every used process: `expect_pose ∈ {match, none}` (`expect_pose_not_match_or_none: []`); `void_steps` 0, `first_void` null everywhere; `config.tree_brute_max_rows` 64 and `target_env msvc` on all 156; `tree_diag_violations: []` (every J and R `tree` process `static_rebuilds 1, members 1`, all other counters 0; every `allpairs` and `s16` process all-zero).
- One hash per (scene, cfg) across kinds and W over the used set: J (cfg default and cfg a; tree, allpairs, armed, canary; W 1/2/4/8/16) `0x32d5e235342b4143`; R `0xee2a67a98434919a`; S16 `0x71313833f6a8e645`.
- **The one INVALID process (a determinism fact for the analyst, not a claim): pass 1 round 2 seq 75 `T-D-allpairs` W=1** (`--scene jolt --gap 0.5 --cfg default --broadphase allpairs --workers 1`, 10:43:16–10:43:21, exit 0, receipts 0.85 / 0.68 %, others 0.22 %, `void_steps 0`) reported `pose_hash 0xf6e397d168e0e8b8`. Its `final_manifolds 4515` and `final_pairs 9559` equal every other J process's; `final_top_y` = 28.990150 vs 28.990126; its `pose.bin` differs from the re-run's in **42,845 of 64,480 bytes, first differing body 0, all 1240 bodies touched** — a numerical, not structural, divergence on a one-worker AllPairs process (SUMMARY: `workers 1`, `pool_workers 1, dispatcher 1`, `parallel_solve true, parallel_narrowphase true, colored true, simd_solve true`). The re-run (10:44:00–05) hashed `0x32d5e235342b4143`. 1 of 13 `T-D-allpairs` W=1 processes (12 originals + the re-run); all other 120 used J processes of this window, both warm-ups, and the 15 J processes of the 500-step pose gate hashed `0x32d5e235342b4143`. The invalid record was not used as an `--expect-pose` reference (the driver stores only valid records; seq 76 `T-D-tree` W=1 matched against pass 1 round 1's seq 49). Files: `raw/pass-01/075_r2_T-D-allpairs_W1/` and `…_rerun/`.

### 8.4 Cells (median over K=6 of the process's [0,500) window mean, ms/step; range = (max−min)/median; IQR; SE = 1.2533·SD/√K; `[0,100)` / `[100,500)` sub-window medians). `raw/g5_reduction.json` (sha256 `ba76f61be9a1d76a34f9…`), printed tables `logs/13_reduce_g5.txt`.

| cell | K | median ms | min | max | range % | IQR % | SE % | [0,100) | [100,500) | pose |
|---|---|---|---|---|---|---|---|---|---|---|
| T-A-tree@W1 | 6 | **17.8887** | 17.7909 | 18.1075 | 1.77 | 0.48 | 0.32 | 17.568 | 17.956 | 0x32d5…4143 |
| T-A-allpairs@W1 | 6 | **19.5711** | 19.4435 | 19.6971 | 1.30 | 0.34 | 0.22 | 19.308 | 19.629 | same |
| T-A-tree@W2 | 6 | **10.5878** | 10.5113 | 10.6917 | 1.70 | 0.76 | 0.32 | 10.474 | 10.622 | same |
| T-A-allpairs@W2 | 6 | **12.2700** | 12.1556 | 12.2920 | 1.11 | 0.80 | 0.26 | 12.097 | 12.308 | same |
| T-A-tree@W4 | 6 | **6.8738** | 6.8258 | 7.0866 | 3.79 | 1.37 | 0.73 | 6.824 | 6.886 | same |
| T-A-allpairs@W4 | 6 | **8.5305** | 8.5081 | 8.7359 | 2.67 | 1.46 | 0.59 | 8.463 | 8.547 | same |
| T-A-tree@W8 | 6 | **5.0234** | 5.0006 | 5.0812 | 1.60 | 0.51 | 0.29 | 4.991 | 5.032 | same |
| T-A-allpairs@W8 | 6 | **6.6421** | 6.5817 | 6.7404 | 2.39 | 1.26 | 0.47 | 6.587 | 6.656 | same |
| T-A-tree@W16 | 6 | **4.9258** | 4.8693 | 5.1730 | 6.17 | 0.91 | 1.15 | 4.916 | 4.925 | same |
| T-A-allpairs@W16 | 6 | **6.4863** | 6.4383 | 6.5832 | 2.23 | 0.90 | 0.42 | 6.469 | 6.490 | same |
| T-D-tree@W1 | 6 | **9.0262** | 8.9176 | 9.0917 | 1.93 | 0.97 | 0.38 | 8.966 | 9.034 | same |
| T-D-allpairs@W1 | 6 | **10.5537** | 10.4639 | 10.7378 | 2.60 | 1.05 | 0.49 | 10.445 | 10.581 | same (the re-run replaces seq 75) |
| T-D-tree@W8 | 6 | **3.6987** | 3.6880 | 3.7706 | 2.23 | 0.79 | 0.44 | 3.697 | 3.699 | same |
| T-D-allpairs@W8 | 6 | **5.4905** | 5.3267 | 5.5493 | 4.05 | 2.15 | 0.84 | 5.312 | 5.535 | same |
| R-tree@W1 | 6 | **12.3082** | 12.2217 | 12.7038 | 3.92 | 1.17 | 0.75 | 12.440 | 12.292 | 0xee2a…919a |
| R-allpairs@W1 | 6 | **13.9059** | 13.6707 | 14.0529 | 2.75 | 0.32 | 0.46 | 13.989 | 13.885 | same |
| R-tree@W8 | 6 | **5.3097** | 5.0855 | 5.4814 | 7.46 | 5.38 | 1.65 | 5.204 | 5.320 | same |
| R-allpairs@W8 | 6 | **6.8474** | 6.6937 | 7.4401 | 10.90 | 2.65 | 2.00 | 6.927 | 6.781 | same (max = the others-6.49 % process) |
| S16-tree@W1 | 6 | **0.0448** | 0.0445 | 0.0458 | 3.08 | 1.83 | 0.65 | 0.0488 | 0.0439 | 0x7131…e645 |
| S16-allpairs@W1 | 6 | **0.0450** | 0.0445 | 0.0459 | 3.01 | 1.59 | 0.60 | 0.0484 | 0.0441 | same |
| T-A-tree-armed@W1 | 6 | **17.8700** | 17.7608 | 18.1505 | 2.18 | 1.08 | 0.44 | 17.692 | 17.946 | 0x32d5…4143 |
| T-A-allpairs-armed@W1 | 6 | **19.5546** | 19.2706 | 19.7255 | 2.33 | 0.71 | 0.41 | 19.195 | 19.630 | same |
| T-A-tree-armed@W8 | 6 | **5.0186** | 5.0118 | 5.2985 | 5.71 | 2.85 | 1.28 | 5.020 | 5.021 | same |
| T-A-allpairs-armed@W8 | 6 | **6.6947** | 6.6057 | 7.0288 | 6.32 | 2.19 | 1.21 | 6.635 | 6.710 | same |
| T-C-tree@W1 | 6 | **18.7639** | 18.6803 | 18.9201 | 1.28 | 0.45 | 0.24 | 18.539 | 18.824 | same |
| T-C-tree@W8 | 6 | **5.3397** | 5.2935 | 6.0387 | 13.96 | 1.74 | 2.78 | 5.336 | 5.308 | same |

Per-process means (pass, round, seq, ms) of the cells whose range exceeds 5 %: `R-allpairs@W8` (0,0,21) 6.694 (0,1,47) 6.881 (0,2,73) 6.998 **(1,0,6) 7.440** (1,1,32) 6.814 (1,2,58) 6.779; `R-tree@W8` 5.221 5.086 5.451 5.129 5.398 5.481; `T-A-allpairs-armed@W8` 6.709 6.799 6.681 6.613 **(1,1,30) 7.029** 6.606; `T-A-tree-armed@W8` 5.203 5.018 **(0,2,74) 5.299** 5.012 5.019 5.013; `T-C-tree@W8` 5.301 5.402 5.306 5.294 5.373 **(1,2,55) 6.039**; `T-A-tree@W16` **(0,0,25) 5.173** 4.927 4.869 4.884 4.943 4.924. The `[0,100)` vs `[100,500)` sub-windows agree within 2 % on every J and R cell (S16: 10 % — a 45-µs step).

### 8.5 Comparisons (the section 1.1 statistic: claimed iff |ratio − 1| > 2·hypot(spread_A, spread_B), printed under range / IQR / SE). Window-3 reference cells (`D-L5`, `JOLT56-T`, K=6 each) read from `…/2026-09-21-physics-window3/raw/analysis.json`; Jolt v5.6.0 only.

| comparison (B vs A) | A median | B median | ratio | effect % | bar range / IQR / SE % | claimed (range / IQR / SE) |
|---|---|---|---|---|---|---|
| T-A-tree vs T-A-allpairs @W1 | 19.5711 | 17.8887 | 0.9140 | −8.60 | 4.39 / 1.18 / 0.78 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W2 | 12.2700 | 10.5878 | 0.8629 | −13.71 | 4.07 / 2.19 / 0.83 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W4 | 8.5305 | 6.8738 | 0.8058 | −19.42 | 9.28 / 4.00 / 1.88 | yes / yes / yes |
| **T-A-tree vs T-A-allpairs @W8 (headline)** | 6.6421 | 5.0234 | 0.7563 | **−24.37** | 5.75 / 2.72 / 1.11 | yes / yes / yes |
| T-A-tree vs T-A-allpairs @W16 | 6.4863 | 4.9258 | 0.7594 | −24.06 | 13.12 / 2.56 / 2.45 | yes / yes / yes |
| T-D-tree vs T-D-allpairs @W1 | 10.5537 | 9.0262 | 0.8553 | −14.47 | 6.47 / 2.86 / 1.23 | yes / yes / yes |
| T-D-tree vs T-D-allpairs @W8 | 5.4905 | 3.6987 | 0.6737 | −32.63 | 9.26 / 4.59 / 1.91 | yes / yes / yes |
| R-tree vs R-allpairs @W1 | 13.9059 | 12.3082 | 0.8851 | −11.49 | 9.57 / 2.43 / 1.75 | yes / yes / yes |
| R-tree vs R-allpairs @W8 | 6.8474 | 5.3097 | 0.7754 | −22.46 | 26.41 / 12.00 / 5.19 | **no** / yes / yes |
| S16-tree vs S16-allpairs @W1 | 0.0450 | 0.0448 | 0.9962 | −0.38 | 8.61 / 4.84 / 1.77 | no / no / no |
| T-A-tree-armed vs T-A-allpairs-armed @W1 | 19.5546 | 17.8700 | 0.9139 | −8.61 | 6.38 / 2.59 / 1.20 | yes / yes / yes |
| T-A-tree-armed vs T-A-allpairs-armed @W8 | 6.6947 | 5.0186 | 0.7496 | −25.04 | 17.04 / 7.19 / 3.52 | yes / yes / yes |
| T-A-tree-armed vs T-A-tree (arming cost) @W1 | 17.8887 | 17.8700 | 0.9990 | −0.10 | 5.62 / 2.36 / 1.08 | no / no / no |
| T-A-tree-armed vs T-A-tree (arming cost) @W8 | 5.0234 | 5.0186 | 0.9990 | −0.10 | 11.87 / 5.79 / 2.62 | no / no / no |
| T-C-tree vs T-A-tree @W1 (the canary's step rise) | 17.8887 | 18.7639 | 1.0489 | +4.89 | 4.37 / 1.31 / 0.79 | yes / yes / yes |
| T-C-tree vs T-A-tree @W8 | 5.0234 | 5.3397 | 1.0630 | +6.30 | 28.10 / 3.62 / 5.60 | no / yes / yes |
| **T-D-allpairs (this binary) vs window-3 D-L5 @W1 (the bridge)** | 10.7379 | 10.5537 | 0.9828 | −1.72 | 5.57 / 2.57 / 1.08 | no / no / **yes** |
| **T-D-allpairs (this binary) vs window-3 D-L5 @W8 (the bridge)** | 5.3474 | 5.4905 | 1.0268 | +2.68 | 15.20 / 5.49 / 3.00 | no / no / no |
| T-D-tree vs window-3 Jolt v5.6.0 @W1 | 9.8282 | 9.0262 | 0.9184 | −8.16 | 37.76 / 6.89 / 7.04 | no / yes / yes |
| **T-D-tree vs window-3 Jolt v5.6.0 @W8** | 2.5693 | 3.6987 | 1.4396 | **+43.96** | 48.73 / 6.37 / 9.56 | no / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W1 | 9.8282 | 17.8887 | 1.8201 | +82.01 | 37.73 / 6.69 / 7.03 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W2 | 5.7705 | 10.5878 | 1.8348 | +83.48 | 5.24 / 2.31 / 1.00 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W4 | 3.5814 | 6.8738 | 1.9193 | +91.93 | 11.06 / 5.46 / 2.21 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W8 | 2.5693 | 5.0234 | 1.9552 | +95.52 | 48.64 / 6.26 / 9.54 | yes / yes / yes |
| T-A-tree vs window-3 Jolt v5.6.0 @W16 | 2.3885 | 4.9258 | 2.0623 | +106.23 | 16.29 / 7.07 / 3.21 | yes / yes / yes |

Window-3 reference cells as read: `D-L5@W1` 10.7379 (range 1.01 %, SE 0.23 %), `D-L5@W8` 5.3474 (6.43 / 1.24 %); `JOLT56-T` @W1 9.8282 (18.78 / 3.50 %), @W2 5.7705 (1.99 / 0.38 %), @W4 3.5814 (4.02 / 0.83 %), @W8 2.5693 (24.26 / 4.76 %), @W16 2.3885 (5.32 / 1.12 %).

### 8.6 The armed rows — `phys_bp_*` spans, Δbp, t_q, the structure receipts (recipe 2.3 gate inputs; per process the median over [100,500) of the per-step value, then the median over K; ms)

| cell | verify | build | query (t_q) | assemble | **Σ four spans** | `sys_physics_broadphase` | per-process Σ |
|---|---|---|---|---|---|---|---|
| T-A-tree-armed @W1 | 0.0078 | 0.0267 | **0.4140** | 0.0264 | **0.4756** | 0.4762 | 0.4756 0.4727 0.4734 0.4781 0.4755 0.4795 |
| T-A-tree-armed @W8 | 0.0066 | 0.0228 | **0.4110** | 0.0261 | **0.4662** | 0.4665 | 0.4687 0.4672 0.4659 0.4643 0.4664 0.4661 |
| T-C-tree @W1 | 0.0074 | 0.0259 | 0.4141 | 0.0260 | 0.4735 | 0.4741 | 0.4708 0.4762 0.4661 0.4756 0.4776 0.4713 |
| T-C-tree @W8 | 0.0066 | 0.0223 | 0.4097 | 0.0259 | 0.4646 | 0.4648 | 0.4637 0.4696 0.4644 0.4629 0.4647 0.4664 |
| T-A-allpairs-armed @W1 | – | – | – | – | – | **2.0291** (per process 2.0493 2.0137 2.0446 1.9550 2.0659 1.9831) | |
| T-A-allpairs-armed @W8 | – | – | – | – | – | **1.9543** (1.9544 1.9588 1.9541 1.9475 1.9533 1.9548) | |

- **Δbp** (`sys_physics_broadphase` allpairs − tree): @W1 mean over [0,500) 2.0341 − 0.5167 = **1.5174 ms**, median over [100,500) 2.0291 − 0.4762 = **1.5529 ms**; @W8 1.9541 − 0.4700 = **1.4841 ms** (mean), 1.9543 − 0.4665 = **1.4878 ms** (median). The recipe's realized-Δbp(8) requirement is ≥ 1.03 ms.
- **The tree span** (Σ of the four `phys_bp_*` spans, median over [100,500), then over K): @W1 **0.4756 ms** vs the recipe's limit **0.36 ms**; @W8 **0.4662 ms** vs **0.35 ms** — both above the limit (the recipe's gate reads "else stop"; the window had completed; recorded here for the analyst, next to G4's `bp_g4_scene/tree/j100` 0.4429 ms bench figure of section 7.1). t_q (the query span) is 0.414 / 0.411 ms of it (mean over [100,500) 0.452 / 0.413).
- Arming cost: `T-A-tree-armed` vs `T-A-tree` −0.10 % at both W (not claimed under any spread).
- Structure, every armed process (tree-armed, allpairs-armed, canary; W 1 and 8): `void_steps 0`, `first_void null`; tree rows `span_n_sets = [(1,1,1,1)]` (each of the four spans once per step), `queried + members = 1241` on every step, `members ∈ {0, 1}` (0 on step 0 before the first static build, 1 after); allpairs-armed rows all four `_n` = 0, `queried + members = 0`.

### 8.7 The canary (`T-C-tree`, `--canary-frac 0.05 --canary-ref-ns <T-A-tree window mean at the same W, most recent valid process>`)

| W | `--canary-ref-ns` used (pass 0 r0/r1/r2, pass 1 r0/r1/r2) | `canary_ns` = F·T per process | `sys_parity_canary_ns` median per process (ms) | T-C-tree median | T-A-tree median | step rise |
|---|---|---|---|---|---|---|
| 1 | 18107483 / 17914429 / 17790907 / 17790907 / 17863065 / 17825052 | 905374 895721 889545 889545 893153 891253 (median 0.8922 ms) | 0.9057 0.8961 0.8898 0.8899 0.8935 0.8916 | 18.7639 | 17.8887 | **+0.8752 ms** (claimed under all three spreads, section 8.5) |
| 8 | 5000595 / 5038960 / 5081182 / 5081182 / 5014144 / 5010739 | 250030 251948 254059 254059 250707 250537 (median 0.2513 ms) | 0.2503 0.2522 0.2543 0.2543 0.2510 0.2508 | 5.3397 | 5.0234 | **+0.3163 ms** (claimed under IQR and SE, not under range — `T-C-tree@W8` range 13.96 % from its seq 55 process at 6.039 ms) |

The canary's own span reads F·T on every process (span median vs `canary_ns` within 0.1 %); the step rise at W=1 is 0.875 ms against F·T = 0.892 ms, at W=8 0.316 ms against 0.251 ms.

### 8.8 G5 facts next to the fired G4 rules (no claims)

- The G4 1.2 stop rules fired (`J snapshot` 0.4429 ms > 0.30; `compaction` 3.2–4.9× its limits; `high_jumper/100000` +4.8 %) and the orchestrator ruled to run G5 regardless; C4's constants and the default flip are not decided from this window unless the analyst shows the fired rules do not touch them. G5's own tree-span gate input is above its limit at both W (8.6) — the same quantity the `J snapshot` rule measured.
- The bridge: `T-D-allpairs` on this binary vs window 3's `D-L5`: −1.72 % at W=1 (claimed only under SE), +2.68 % at W=8 (not claimed).
- `T-D-tree@W8` 3.6987 ms against Jolt v5.6.0's 2.5693 ms: +43.96 %.

## 9. Files

`bin/` (runner_c3.exe, SHA256SUMS, COMMIT.txt) · `logs/00_disk.txt`, `01_*.txt`, `02_build_parity.log`, `03_build_bench.log`, `04_pose_gate.txt`, `04_pose_files_sha256.txt`, `05_g4_run.{pid,stdout,stderr}.txt`, `07_idle_receipt.pid.txt`, `08_claude_ui_cpu.txt`, `09_reduce_g4.txt`, `10_g4_facts.txt` · `gate/` (28 × .out/.err/.rc + 4 pose .bin) · `rows.json` · `tools/` · `wait_log.txt` (the first run's 89 polls, the 03:40 standalone receipt, the pointer to the driver's log) · `raw/g4/{g4_log.txt, wait_log.txt, runs.jsonl, g4_reduction.json, G4_DONE, logs/ (332), <group>/… estimates, stop1_0334/}` · `test/window/`, `test/g4/`, `rehearsal/` (untimed). · **G5**: `logs/06_window4_run.{pid,stdout,stderr}.txt`, `logs/11_claude_ui_cpu_g5.txt`, `logs/12_idle_receipt_g5.pid.txt`, `logs/13_reduce_g5.txt` (+ `.stderr.txt`, empty), `logs/14_g5_section.md` (the staged text of section 8), `logs/window_report_before_g5.md.bak` · `raw/manifest.json`, `raw/runs.jsonl` (159), `raw/window_log.txt`, `raw/wait_log.txt`, `raw/window_state.json`, `raw/WINDOW_DONE` (`complete`), `raw/pass-00/` (79 process dirs), `raw/pass-01/` (80 incl. `075_r2_T-D-allpairs_W1_rerun`), `raw/receipts/` (empty: no Jolt row), `raw/g5_reduction.json`, `raw/launch1_aborted_0819/` (the 08:19 launch that timed nothing) · `wait_log.txt` (+ the 08:04 standalone receipt and the G5 pointers).
