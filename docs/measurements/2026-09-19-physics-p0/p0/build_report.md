# P0 build and prepare: MEASUREMENT-QUEUE section 10 (tester, 2026-09-19)

**Nothing was timed in this stage, and no timing is quoted here.** Every number below is a hash, a count, an
exit code, a pose hash or a load-receipt percentage.

- Tree: `D:/wt/joltab` at `dbd859771b4d7aee5f560c7d3cbceccd0f1af179`. The working tree was clean before the
  first swap and is clean now (`git status --short --untracked-files=all` prints nothing). No commit, no push.
- Toolchain: `stable-x86_64-pc-windows-msvc`, rustc 1.98.1 (48a229cea 2026-09-01), LLVM 22.1.8,
  `host: x86_64-pc-windows-msvc`. `RUSTFLAGS` was unset for every cargo call.
  `CARGO_TARGET_DIR=D:/wt/_targets/joltab-msvc`, which already existed. No new target dir was created.
- Machine: AMD Ryzen 9 5900HS, 8 cores / 16 logical processors.
- D: free space was 6.9 GB at start and **2.6 GB at the end**. Most of the drop, 6.9 to 3.0 GB, happened
  before my first compile, while other lanes were building. My builds overwrite artifacts in place and cost
  about 0.3 GB. The stop threshold (2 GB) was never reached.

## 1. boyko binaries (section 10, "Built before the window")

All binaries were copied to `p0/bin/`. `p0/bin/SHA256SUMS` passes `sha256sum -c` (5/5 OK). Every arm builds
to the same path, `D:/wt/_targets/joltab-msvc/parity/deps/jolt_parity_pyramid-804617a1205f3459.exe`, so each
exe was copied out and hashed before the next build.

| file in `p0/bin/` | source state | build command (msvc, no RUSTFLAGS) | sha256 | re-run of the same command |
|---|---|---|---|---|
| `runner_tip.exe` | tip | `cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` | `ef9325efa9a277274f602c59504b0856f75ae1ddec7d683b0ac5f8c5bf7968c3` | no-op: `Finished`, no `Compiling`; `-v` shows all 12 workspace crates `Fresh` |
| `runner_pre_instrument.exe` | tip with the physics zone sites removed (below) | same | `b7c4233425d0d4a940045784d832ad8c40ebb34aaa0f57db3a94fe097be4cab3` | no-op (3 cached warnings replayed, no `Compiling`) |
| `runner_pre_l1.exe` | tip with L1 (`00c07d0f`) reverse-applied | same | `a0da832f14157fadfbb47fd78db6c10182ca6a72362ac70eefce9c56c92486ec` | no-op |
| `runner_shipping.exe` | tip, `BOYKO_PROFILE=shipping` | `BOYKO_PROFILE=shipping cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid` | `fdcac07f8e576e63c4789e8662498641e730863b35f720a3d061bbe6d4aa02c2` | no-op |
| `broadphase_bench.exe` | tip | `cargo bench --no-run -p boyko-physics --bench broadphase` (`bench` profile) | `3c5cd8b4671165fd3995f06bc73f9c80e40e8ace453fab847d0c1febb39e3303` | no-op |

`[profile.parity]` in the root `Cargo.toml` is `inherits = "release"` and adds nothing else, so it is fat LTO
with default codegen units, as ruling W1 requires. The `.cargo/config.toml` baseline
`-C target-cpu=x86-64-v3` stays in effect because nothing overrode it.

**The tip exe had already been built.** An exe from 04:10 was already in the target dir, built after the
tree's sources (03:34), and cargo reported it `Fresh`. I copied that exe. To prove the restored tree produces
the same binary, I rebuilt the tip at the end, after every swap had been restored and after the shipping
build. Rebuilt exe: `b7f7d55e74aa48cca74d4b25ca5146bdea884a4c60c55b34e216d26cf4e68b94`. It differs from
`runner_tip.exe` in **9 bytes only**: the PE `TimeDateStamp` at 0xF0 (2 bytes), three debug-directory
timestamps (2 bytes each) and the CodeView PDB age (1 byte). Code and data are identical. The saved
`cmp -l` output is `logs/tip_vs_tip_rebuild.cmp`. Each variant differs from the tip by far more:
pre-instrument by 744,810 bytes, pre-L1 by 704,399, shipping by 916,090. So no swap was a silent no-op.

### Swaps: made by copy, with sha256 proof both ways

Original tip files, in `swap/ORIGINAL.sha256` (each equals its `HEAD` blob by `git hash-object`):

- `colored.rs`: `480c13f4…0ee4`
- `systems.rs`: `66067f5f…59a5`
- `resources.rs`: `9109af84…3ee3`

**Pre-instrument.**
- Source: the reverse of `13748cec`'s diff for `solver/colored.rs` and `systems.rs`, applied to copies of
  the tip files in `swap/preinst/`. All 15 hunks applied cleanly at offsets.
- Two things were kept on purpose:
  - The `pub(crate)` visibility of `MIN_PARALLEL_SLOTS_PER_COLOR`. `profiling.rs:141`
    (`WIDE_COLOR_MIN_SLOTS`) reads it, and it is not a site.
  - `profiling.rs`, `lib.rs`'s `profiling_partition!` and `Schedule::system_zones`. These are declarations
    the runner links against, not sites.
- Proof the variant is exactly "tip minus sites": the lines removed from the tip equal the diff's `+` lines,
  and the lines added equal its `-` lines (sorted-multiset md5 match both ways). No `zone!`, `zone_enabled!`
  or `counter!` remains in either file.
- Swapped in (repo files): `colored.rs` became `825eb426…9cd4` and `systems.rs` became `5d67bed8…8c9f`.
- The build compiled with 3 expected warnings: an unused `counter` macro, its unused re-export, and an unused
  `push_counter`.
- Restored: `480c13f4…` and `66067f5f…`, both equal to ORIGINAL. `git status` clean.

**Pre-L1.**
- Source: the reverse of `00c07d0f`'s diff, applied to a copy of the tip's `resources.rs`. All 3 hunks
  applied cleanly at offsets.
- The sheet's shortcut (the file as at `1c31aeac`, blob `8ed4373a`) was not valid here. `56c1e9e7` (the
  colored and simd default) changed `resources.rs` after that commit and reaches the tip through merge
  `1d6a9d6b`.
- Proof: the same multiset match against L1's diff, both ways. `island_ids()` is gone, and
  `begin_step` reads `graph.island_of(row as u32)` again.
- Swapped in: `905714f2…9655`.
- Restored: `9109af84…`, equal to ORIGINAL. `git status` clean.

Restores used plain `cp`, so each restored file got a new mtime. This matters: preserving the old mtime would
have let cargo treat the variant's artifacts as fresh for the restored source. The final tip rebuild shows
that cargo did recompile.

**The shipping tier.** The summary reads `"profile_name":"shipping","zones_compiled":false,`
`"system_zones_compiled":false,"debug_assertions":false,"target_env":"msvc"`. `--arm-profiler` is refused
with exit 2 ("folds the zones"), as the runner documents.

## 2. Jolt binaries

All three builds the sheet needs exist. **None is missing.** Nothing was downloaded or rebuilt.

| role | exe | sha256 | build type | source |
|---|---|---|---|---|
| `v530` (headline) | `D:/tmp/jolt/build-v5.3.0-dist/PerformanceTest.exe` | `29b23ad1cfc38b7fb39b484b8cc2cfad522a0a910fd03d6e3a15f8b90e0cefac` | Distribution | `wt-v5.3.0-parity`, `v5.3.0-dirty` (0373ec0d) |
| `v530_release` (`-p` shares) | `D:/tmp/jolt/build-v5.3.0-release/PerformanceTest.exe` | `de00c882c5b667ab9b2029573548cc013450a38238268dc23353df3026b782f1` | Release | same tree |
| `v560` (second column) | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` | `918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad` | Distribution | `wt-v5.6.0-parity`, `v5.6.0-dirty` (e77f1755) |

- **Compiler** (all three): `c++.exe (MinGW-W64 x86_64-ucrt-posix-seh, built by Brecht Sanders, r4) 16.1.0`,
  WinLibs POSIX UCRT, MinGW Makefiles.
- **CMake options** (all three): `INTERPROCEDURAL_OPTIMIZATION=ON`, AVX2/FMADD/F16C/LZCNT/TZCNT ON, AVX512 OFF,
  `CROSS_PLATFORM_DETERMINISTIC=OFF`, `USE_ASSERTS=OFF`, `DOUBLE_PRECISION=OFF`,
  `PROFILER_IN_DISTRIBUTION=OFF`, `PROFILER_IN_DEBUG_AND_RELEASE=ON`.
  - So only the Release build has the profiler, which is what `-p` needs. Distribution compiles it out (O6).
  - `FLOATING_POINT_EXCEPTIONS_ENABLED=ON` sits in the cache but is a no-op on this compiler:
    `Jolt.cmake:527` applies it only for MSVC.
  - v5.6.0 additionally records `JPH_USE_DX12/VK/MTL/CPU_COMPUTE=ON`. These are its defaults, recorded rather
    than patched away (plan open question 2).
- **Patch:** in both source trees the diff signature (`27df7f3b…e210`) equals the repository's
  `pyramid_scene.patch`, so `source_is_repo_patch = true` for all three. Every run prints
  `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=…, allow_sleep=…, receipt=…`.
- **The builds are not stale:**
  - patched sources were last modified at 01:41–01:47;
  - the `PerformanceTest.cpp.obj` files are from 01:47–01:49;
  - the exes are from 01:48–01:50.
- **Runtime DLLs beside each exe** are identical across the three builds: `libstdc++-6.dll`
  `a36c2e68…ae03`, `libgcc_s_seh-1.dll` `4dfa2c0d…6b00`, `libwinpthread-1.dll` `c29cbbe3…0494`.
- The full record is in `p0/manifest.json`, written by the driver's own `manifest` subcommand.

## 3. Receipts

- HEAD `dbd859771b4d7aee5f560c7d3cbceccd0f1af179`, `dirty_paths: []`.
- `rustc -vV` host line: `x86_64-pc-windows-msvc`. The runner's summary also reports `target_env: "msvc"`.
- `git merge-base --is-ancestor <c> HEAD` gives exit 0 (true) for all four:
  - S5 `8d656ad8`
  - KE16 `67563d3b`
  - B1 `aff98fe7`
  - L1 `00c07d0f`

**Thread counts.** The OS counts were sampled with Toolhelp while each process ran (jolt scene, 120 steps,
untimed). The steady count per process:

| W | boyko pool workers | boyko OS threads | Jolt job threads | Jolt OS threads | Jolt stat line prints |
|---|---|---|---|---|---|
| 1 | 1 | 5 | 0 | 4 | 1 |
| 2 | 2 | 6 | 1 | 5 | 2 |
| 4 | 4 | 8 | 3 | 7 | 4 |
| 8 | 8 | 12 | 7 | 11 | 8 |
| 16 | 16 | 20 | 15 | 19 | 16 |

- **boyko:** OS threads = W + 4. That is W pool workers (`ThreadPoolBuilder::num_threads(W)` spawns exactly W,
  `thread_pool.rs:668-760`), plus the main thread, which is the dispatcher, plus 3 idle system threads.
- **Jolt:** OS threads = W + 3. That is W − 1 `JobSystemThreadPool` threads (`PerformanceTest.cpp:357-364`,
  `-t=W` becomes `W-1`), plus the calling thread, plus the same 3.
- The 3 are most likely the ntdll loader's thread-pool workers. That is inferred from the arithmetic only.
- The extra boyko thread is the dispatcher.

**Does boyko's dispatcher run scope tasks at W=16? No.**

By code:
- `physics_solve_colored` is an ordinary system with `Res`/`ResMut` parameters, not exclusive, so it runs on a
  pool worker.
- The solve's `pool.scope` joiner is therefore that worker (`join_on_worker`). Its chunk tasks go to that
  worker's deque, and siblings steal them.
- Meanwhile the dispatcher sits in `Schedule::run` step 5, `park_timeout` (`schedule.rs:848-869`). It is not
  in the pool's idle bitset, so no `claim_one_idle` wake can reach it.
- The only path on which the dispatcher runs a pool task is `join_external_helping` at the end of the
  executor's own `install` scope, after every system has completed.

By witness: the runner's `threads.solve_on_dispatcher_steps` is 0 at every W in both dry passes, and 0 of 500
in the full-length armed W=16 run.

Caveat: that witness sees where the solve's zones land, not where chunk tasks run, because no zone may sit in
a chunk task (O1). The "no" for chunk tasks rests on the code path above.

So at W=16 both engines run 16 threads through the parallel work. That is 16 logical CPUs on an 8-core SMT
part; W=8 equals the physical core count.

## 4. Dry pass of the driver (untimed rehearsal; no timing quoted)

Commands:
- `driver.py manifest` (section 2)
- `driver.py pass --dry --pass 1`: 53 runs, W descending, boyko first, including the pass-1-only rows
  `JOLT-P` and `JOLT-RCPT`
- `driver.py pass --dry --pass 2`: 50 runs, W ascending, Jolt first
- `driver.py reduce --dry`

Output is in `p0/dry/`. The checks were run with `p0/check_dry.py` and `p0/show_cfg.py`.

**Every row id ran:**
- boyko: J-A-d1, J-A-a, J-A-d2, J-B, J-C, J-P1, J-Son, J-S0, L1-A1, L1-B, L1-A2, R, R-ref, R-S, S16,
  AA2-pre, SHIP.
- Jolt: JOLT-T, JOLT56-T, JOLT-NPC, JOLT-SLP, JOLT-P, JOLT-RCPT.

Across the two passes: 103 runs, 0 skipped, 0 non-zero exits, and every boyko run printed a `SUMMARY`.

**Anti-vacuity holds (ruling W4):**
- 0 void boyko runs. The runner's per-step structural check is green on every step of every armed run.
- Armed runs, per step:
  - each system span once;
  - `phys_solve_build`, `phys_restitution`, `phys_store` and `phys_write_back` once each;
  - `phys_gravity`, `phys_warm_apply`, `phys_integrate` and `phys_pass_biased` 4 each;
  - `phys_pass_relax` 8;
  - the sleep zones 1/2/1 exactly on the sleeping rows (J-Son, J-S0, R-S) and 0 elsewhere;
  - `waves == 12 × wide_colors` and narrow spans `== 12 × (colors − wide)` on every step;
  - 0 dropped samples.
- The jolt scene holds 1240 dynamic bodies and s16 holds 16.
- Every disarmed run reports `disarmed_ring_traffic = 0`. That includes SHIP (zones folded) and AA2-pre.
- R-ref reads zero on every in-solve zone, as designed.
- At 12 dry steps, the jolt scene has only wide colors (8–9). A full-length run exercises the narrow class:
  in 500 armed steps at W=16, 480 steps carry at least one narrow color, with colors 8–15 and wide 8–12. The
  rest rows (R, R-S) exercise both classes in the dry pass.

**Pose hashes agree where section 10 requires it** (determinism: armed = disarmed = cfg-A = cfg-B):
- At 12 steps, one hash, `0x6976399d3f4d3199`, is shared by J-A-d1, J-A-a, J-A-d2, J-B, J-C, J-P1, AA2-pre
  (pre-instrument binary) and SHIP (shipping binary), at every W from 1 to 16.
- The same hash appears on J-S0, L1-A1, L1-B and L1-A2: pre-L1 equals the tip.
- J-B's `--expect-pose` against this pass's J-A-d1 reads `match` at W=1 and W=8 in both passes (H7).
- The reducer reports 7 simulation groups, 0 with more than one pose.
- On the Jolt side, 16 binary+flags+W groups, 0 differing across passes. JOLT-T gives one hash at every W,
  and Release gives the same hash as Distribution. JOLT-NPC differs, as the runner header documents (the pair
  cache is not simulation-identical).

**The canary row shows its span:**
- J-C at W=1 and W=8 has a `sys_parity_canary_ns` column with 1 sample per step.
- The span is at least the spin target on every step.
- In the schedule the canary sits between `physics_narrowphase` and `physics_build_graph`, as specified.
- The canary's `canary_ns` is derived by the driver from this pass's J-A-a at the same W.

**Jolt outputs:**
- Every run printed the patch line and one `Discrete, W, …` stat line.
- The `-f` per-frame CSVs exist.
- JOLT-SLP adds an `Active Bodies` column.
- JOLT-RCPT wrote `receipt_discrete_th1.csv` with `Frame, Manifolds, Points, Active Bodies, Top Y`.
- JOLT-P (Release, `-p`) wrote an HTML profile. It dumps every 100 iterations, so a 500-step run gives 5.

**The gates can fail (each shown red, then left untouched):**
- M1, anti-vacuity: the pre-instrument binary run armed voids at step 0 (``phys_solve_build` recorded 0
  spans, the structure has 1``) and exits 3.
- M2, the H7 pose gate: cfg-b at 13 steps against the 12-step cfg-a pose gives `mismatch …, first differing
  body Some(0)` and exits 4. The matching control (cfg-b at W=16 against cfg-a at W=1, 12 steps) gives
  `match` and exits 0.
- M3, the R-S void rule: `--frozen-by 30` on a 30-step rest pile voids ("1240 dynamic rows still awake") and
  exits 3.

**Full-length structural checks (untimed) to de-risk the window:**
- **J-A, 500 steps:** one pose, `0x32d5e235342b4143`, shared by:
  - cfg-a W=1 disarmed;
  - cfg-b W=8;
  - cfg-a W=16 armed (0 void steps, 0/500 solve steps on the dispatcher);
  - pre-instrument at W=8;
  - shipping at W=8.
- **J-S0, 500 steps:** pre-L1 = tip, and both equal the J-A pose. Sleeping on with threshold 0 is
  bit-identical to sleeping off.
- **J-Son, 1000 steps:** tip at W=8 = pre-L1 at W=8 = tip at W=1 armed (0 void). Every body is asleep from
  step 265 on both binaries. So L1's identity holds across a real freeze.
- **R-S:** `--frozen-by 300` at W=1 armed is not void. First frozen step 248, which is A7-R2's figure. So the
  row should not void in the window.
- **R, 1100 steps:** W=1 serial = W=8 `--parallel-solve` armed (0 void).
- **S16, 300 steps:** W=1 = W=8 armed.

## 5. Sheet flags checked against the runner and the driver

The sheet was written before the runner existed. Checking it against the runner's usage text and
`driver.py` found no flag the runner rejects, with one exception, which the driver already handles:
- J-C's `--canary-frac 0.05` alone is refused (`--canary-frac and --canary-ref-ns go together`, exit 2). The
  driver passes `--canary-ref-ns` = this pass's J-A-a window mean at the same W.

Flags the sheet does not spell, which the driver adds:
- `--arm-profiler` on armed rows;
- `--frozen-by 300` on R-S (dropped under `--dry`);
- `--window`;
- `--workers`;
- `--steps`;
- `--csv`, `--pose-out` and `--expect-pose` on J-B;
- `-t=W` and `-i=<steps>` on Jolt (PerformanceTest accepts `-i=`: `PerformanceTest.cpp:159`).

Both build commands in the sheet are valid as written.

Discrepancies to decide, not defects:
1. **R-S at W=8 runs with `parallel_solve = false`.** The sheet gives `--parallel-solve` at W=8 to R but not
   to R-S, and the driver follows the sheet literally (`show_cfg.py`: `R-S@W8 … parallel_solve=False`). Once
   the pile is frozen the solve dispatches nothing, so the effect is likely small. It is still not the same
   configuration as R@W8, against which F is subtracted.
2. **JOLT-P runs without `-f`.** The sheet says "the timed flags + `-p`". It yields shares only, never times,
   so no quoted number depends on it.
3. **The broadphase bench samples n ∈ {100, 1000, 10000}** (22 criterion benchmarks; `--list` works by path).
   A crossover near `GRID_LO = 96` / `GRID_HI = 192` (`broadphase_policy.rs:79,88`) can only be bracketed by
   these sizes, not located.

## 6. For the window stage

- **The machine was not quiet during this stage.** All 206 dry-pass load receipts exceeded the driver's
  default `--busy-pct 5` (range 6.25–81.64 % busy). The top processes were another lane's test binaries:
  - `properties-*.exe`
  - `gate_goldens-*.exe`
  - another build of `jolt_parity_pyramid-2d333acd….exe`, not mine
  - `broadphase-*`, `colored_solve-*`, `parallel_solve-*`, `row_identity_churn-*`, `ke16_solve_in_system-*`
  - `cargo.exe`
  - `Taskmgr.exe` and `RuntimeBroker.exe` were also in the top 3 of some receipts.

  One of my cargo calls also blocked on the package-cache lock of a concurrent cargo.

  The window must wait these lanes out. Before starting, check the machine's idle baseline against
  `--busy-pct`: with Task Manager or a browser open, a baseline above 5 % would stall every run up to
  `--wait-budget`.
- **The runner does not hold `timeBeginPeriod(1)` the way the shipped host does.** The executor's
  `PARK_TIMEOUT` backstop (`schedule.rs:64-92`) then expires after one system timer quantum. That is ~15.6 ms
  by default, or finer if another process has requested it. A missed wake would show as a single-step
  outlier in the per-step CSV. This is the runner's documented, measured configuration, stated here only so
  that an outlier is not misread.
- D: holds 2.6 GB. The window compiles nothing, so this is enough, but no further builds should land on D:
  first.

## Files

- `docs/measurements/2026-09-19-physics-p0/p0/bin/`:
  `runner_tip.exe`, `runner_pre_instrument.exe`, `runner_pre_l1.exe`, `runner_shipping.exe`,
  `broadphase_bench.exe`, `SHA256SUMS`
- `…/p0/manifest.json`: the driver manifest (H6 receipts, every sha256, Jolt compilers and CMake options).
  The window's `driver.py pass --manifest` takes this file.
- `…/p0/swap/`: `ORIGINAL.sha256`, `*_swapin.sha256`, `*_restore.sha256`, the diffs used
  (`instrument_sites_minus_const.diff`, `l1.diff`), the backups (`orig/`) and the variants (`preinst/`,
  `prel1/`)
- `…/p0/logs/`: every build log and its no-op re-run log, `tip_vs_tip_rebuild.cmp`, the dry-pass and reduce
  logs, `check_dry_pass1.log`
- `…/p0/dry/`: the rehearsal (`runs.jsonl`, per-run dirs, `reduction.json`/`.txt`). **Its numbers are not
  measurements.**
- `…/p0/full/` and `…/p0/mut/`: the full-length structural runs and the mutation proofs
