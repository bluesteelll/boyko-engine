# Window 3 build and prepare (tester, 2026-09-21 00:30-00:40 +03:00)

**Nothing was timed in this stage, and no timing is quoted here.** Every number below is a hash, a count, an
exit code, a pose hash or a byte count, and each names the file under `win3/` it comes from. The two lanes
(`D:/wt/l5np`, `D:/wt/lighttable`) were running throughout; the dry runs are structural checks, not
measurements, and no load receipt was taken for them. The `window_mean_ns` fields inside the dry-run
SUMMARY lines in `logs/` are NOT measurements and must not be quoted.

Root: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win3/`
(`win3/` below). Untouched, as required: `D:/wt/l5np`, `D:/wt/lighttable`, `D:/wt/_targets/joltab-msvc`,
`D:/wt/_targets/vkval-msvc`, and every branch/ref (no commit, no push, no checkout of an existing tree).
`git -C D:/wt/l5np log -1` was read twice (start and end); both read `aac562a7` -- **C4 has not landed**.

## 1. The tree: `D:/wt/mq-aac562a7` (new detached worktree)

- Command: `git -C D:/wt/l5np worktree add --detach D:/wt/mq-aac562a7 aac562a7` -- exit 0
  (`logs/01_worktree_add.log`). It did not exist before; `l5np` was at `aac562a7` and stays there.
- `git log -3` (`logs/02_git_log_ancestors.log`):
  - `aac562a7101451e6aee077b81fc1fb80c61b4587` perf(physics): Auto's Grid band moves from 96/192 to the
    measured 2,700/3,000, so Auto keeps AllPairs on the Jolt pyramid ... (**L2 = C2**)
  - `caac7d067d6a27c878fb807d50510ad9063a66ab` perf(physics): parallel_solve is on by default, and a
    one-worker pool solves inline instead of opening a scope for every wide color (**L4 = C1**)
  - `f236ebdd6b1528106c87fcccfa750fe40eb4b4a7` docs(physics): the tree broadphase and sleeping-by-default
    designs, revision 2, closed by ruling (**the integration line**)
- `git merge-base --is-ancestor` true for all of: `f236ebdd`, `caac7d06`, `dbd85977` (P0b's tip),
  S5 `8d656ad8`, KE16 `67563d3b`, B1 `aff98fe7`, L1 `00c07d0f`, D1+simd `56c1e9e7`.
- The two defaults, grepped in the tree (`logs/03_defaults_grep.log`):
  - `crates/boyko_physics/src/broadphase_policy.rs:89` `pub const GRID_LO: u32 = 2_700;`
    `:101` `pub const GRID_HI: u32 = 3_000;` with the const-asserts `GRID_LO < GRID_HI` (`:103`) and
    `GRID_HI >= DISPARITY_CROSSOVER_BODIES` (2,978, `:105`).
  - `crates/boyko_physics/src/resources.rs:507` `parallel_solve: true,` inside `impl Default for
    PhysicsConfig` (comment: "Default ON since L4"). Also there: `simd_solve: true` (`:484`),
    `broadphase: BroadphaseKind::AllPairs` (`:462`), `broadphase_select: BroadphaseSelectMode::Manual`
    (`:465`), `colored: false` (`:501`; `add_physics_colored_solve` -> `add_physics_pipeline` inserts
    `PhysicsConfig { colored, .. }` at `plugin.rs:496`, so the runner prints `colored: true`, below).
  - The L4 inline gate: `crates/boyko_physics/src/solver/colored.rs:3555-3557` -- `parallel =
    config.parallel_solve && widest_color_slots() >= MIN_PARALLEL_SLOTS_PER_COLOR &&
    try_with_active_pool(|pool| pool.num_threads() >= 2) == Some(true)`, decided once per step.
- `git status --short --untracked-files=all` in the worktree: 0 lines before and after the build.
  `Cargo.lock` is git-ignored (`.gitignore:4: *.lock`), so it is not a dirty path -- see section 2.

## 2. Build: `runner_l4l2.exe`

Command (`logs/build_l4l2.sh`, log `logs/04_build_runner_l4l2.log`):

```
PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 \
CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=D:/wt/_targets/mq-aac562a7   # RUSTFLAGS unset
cd D:/wt/mq-aac562a7 && cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid
```

- rustc 1.98.1 (48a229cea 2026-09-01), LLVM 22.1.8, `host: x86_64-pc-windows-msvc`; `RUSTFLAGS=[unset]`
  (echoed at the top of the log). `.cargo/config.toml:135-136` `[target.x86_64-pc-windows-msvc] rustflags
  = ["-C", "target-cpu=x86-64-v3"]` therefore stays in effect. `[profile.parity] inherits = "release"`
  (`Cargo.toml:138-139`), nothing else -- fat LTO, default CGU, as for `runner_tip`.
- Started 00:33:50, `Finished parity profile [optimized] target(s) in 41.04s`, exit 0. Output:
  `D:/wt/_targets/mq-aac562a7/parity/deps/jolt_parity_pyramid-39550833a2ead4ba.exe`.
  Target dir: **270 MB** (a bench of one crate + its closure, not a workspace build). D: free went
  29.7 -> 29.3 GB.
- Re-run of the same command with `-v` (`logs/05_build_rerun_noop.log`): 84 `Fresh`, 0 `Compiling`,
  `Finished ... in 0.29s`, exe sha256 unchanged.
- **`Cargo.lock`.** The fresh worktree had none (git-ignored), so cargo printed `Locking 444 packages` and
  wrote one. It is **byte-identical to `D:/wt/l5np/Cargo.lock`** (the lane that produced `aac562a7`). It
  differs from `D:/wt/joltab/Cargo.lock` (which built `runner_tip`) in 20 third-party versions
  (bytemuck_derive, cc, cfg-if, clap x3, crc32fast, disqualified, find-msvc-tools, generator, libredox,
  rustix, smallvec, syn 3, synstructure, toml, toml_edit, unicode-ident, yoke-derive, zerofrom-derive,
  zlib-rs). **None of them is in the runner's runtime closure**: `cargo tree -p boyko-physics -e normal`
  lists only the in-tree crates plus crossbeam-{deque,epoch,queue,utils}, fixedbitset, static_assertions,
  windows-link, windows-sys (all at the same versions in both locks) and the proc-macro trio
  (proc-macro2/quote/syn 2/unicode-ident -- compile-time only; `-i cfg-if` and `-i smallvec` under
  `-e normal` print "nothing"). The runner's `use` lines (`jolt_parity_pyramid.rs:249-285`) import std and
  in-tree crates only. So the lock drift cannot change a byte of the measured code.

| file in `win3/bin/` | source | sha256 (`bin/SHA256SUMS`, `sha256sum -c` 4/4 OK) | size |
|---|---|---|---|
| `runner_l4l2.exe` | `aac562a7`, command above | `368d4104578bd621a2eccc3be99e903a9843e070a118e94c90d4b354d1410aeb` | 1,242,112 |
| `runner_tip.exe` | copied from P0's `p0/bin/` (`dbd85977`); verified against P0's `SHA256SUMS` (5/5 OK) before the copy | `ef9325efa9a277274f602c59504b0856f75ae1ddec7d683b0ac5f8c5bf7968c3` | 1,241,600 |
| `D:/tmp/jolt/build-v5.3.0-dist/PerformanceTest.exe` (by path) | P0's v530 Distribution build; not rebuilt | `29b23ad1cfc38b7fb39b484b8cc2cfad522a0a910fd03d6e3a15f8b90e0cefac` | -- |
| `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` (by path) | P0's v560 Distribution build; not rebuilt | `918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad` | -- |

`runner_l4l2.exe` against `runner_tip.exe`: 875,244 differing bytes over the common prefix
(`logs/12_exe_diff.log`) -- the build was not a silent no-op.

## 3. What `--cfg default` means at `aac562a7`

Usage text (`logs/06_usage_text.log`; the binary has no `--help` -- without `--scene` it runs its 3-step
s16 self-check, exit 0; a bad flag with `--scene` prints the usage, exit 2):

```
usage: jolt_parity_pyramid --scene jolt|rest|s16 [--workers W] [--steps N] [--window A..B] [--gap G]
  [--solver colored|reference] [--cfg a|b|default] [--parallel-solve] [--sleeping] [--threshold T]
  [--frozen-by K] [--arm-profiler] [--canary-frac F --canary-ref-ns T] [--csv PATH] [--pose-out PATH]
  [--expect-pose PATH] [--label TEXT]
```

Module doc (`crates/boyko_physics/benches/jolt_parity_pyramid.rs:47-76`, at this commit): `--cfg default`
= "`PhysicsConfig::default()` as this tree ships it, except that `--parallel-solve` and `--sleeping` force
their knobs on ... Since L4 that default has `parallel_solve` on, so `--parallel-solve` no longer changes a
`--cfg default` row". `configure()` (`:743-769`) confirms: for `CfgKind::Default` it touches nothing
unless those two flags are given. `--cfg a` still pins `simd_solve = false`, `parallel_solve = (W > 1)`,
AllPairs/Manual (`:747-756`), so `runner_tip --cfg a` and `runner_l4l2 --cfg a` set the same knobs.

**J-P1 is retired at this commit**: `--parallel-solve --workers 1` is refused with exit 2
("... a one-worker pool solves inline ...", `logs/06_usage_text.log`). Any row list carrying J-P1 will fail.

Printed config summary, one 12-step dry run each (`logs/07_cfg_summary_default_W1_W8.log`), identical at
W=1 and W=8:

```
scene jolt gap 0.5 friction 0.2 bodies 1240 solver Colored cfg Default profile dev armed false
"config":{"substeps":4,"relax_iterations":2,"broadphase":"AllPairs","broadphase_select":"Manual",
          "simd":true,"simd_solve":true,"parallel_solve":true,"parallel_broadphase":false,
          "sleeping":false,"sleep_threshold":0.0001,"sleep_frames":60,"colored":true,
          "contact_hertz":30,"contact_damping":10}
"profile_name":"dev","zones_compiled":true,"debug_assertions":false,"target_env":"msvc"
```

So the task's reading holds on three of four points -- **colored, `simd_solve` on, `parallel_solve` on** --
and needs one correction: the broadphase is **AllPairs under `broadphase_select: Manual`, not `Auto`**.
`PhysicsConfig::default()` ships `Manual` (`resources.rs:465`, "the user owns `broadphase`"), and the runner
does not override it. C2's band (2,700/3,000) is what `Auto` *would* apply; at 1240 bodies it would also
select AllPairs, so the pair path is the same, but **the `select_broadphase` policy's band is not exercised
by this row** (the `sys_select_broadphase` system still runs -- it is a column in `dry/armed_W8.csv` -- but
under `Manual` it only counts). If the window is meant to measure Auto's cost, that needs a runner flag
that does not exist at this commit; flag for the orchestrator, not a defect.

## 4. Dry runs (untimed)

### 4.1 W-invariance and the bridge to cfg-A (`logs/08_pose_500_W_invariance.log`, `logs/09_pose_checks_and_red_control.log`)

Reference: `runner_tip --scene jolt --gap 0.5 --cfg a --workers 1 --steps 500 --pose-out
dry/tip_cfga_W1.pose.bin` -> `pose_hash 0x32d5e235342b4143`, exit 0. This equals P0b's J-family 500-step
pose (`0x32d5e235342b4143`, MEASUREMENT-QUEUE section 10 RESULT block).

Then `runner_l4l2 --scene jolt --gap 0.5 --cfg default --workers W --steps 500 --pose-out ... --expect-pose
dry/tip_cfga_W1.pose.bin`:

| W | pose_hash | `expect_pose` | exit | void | ring traffic | manifolds / pairs (final) |
|---|---|---|---|---|---|---|
| 1 | `0x32d5e235342b4143` | match | 0 | 0 | 0 | 4515 / 9559 |
| 8 | `0x32d5e235342b4143` | match | 0 | 0 | 0 | 4515 / 9559 |
| 16 | `0x32d5e235342b4143` | match | 0 | 0 | 0 | 4515 / 9559 |
| 2 (extra) | `0x32d5e235342b4143` | match | 0 | 0 | 0 | 4515 / 9559 |
| 4 (extra) | `0x32d5e235342b4143` | match | 0 | 0 | 0 | 4515 / 9559 |

All six `*.pose.bin` files (64,480 bytes each) share sha256
`eff361e1bdf7261ddee5237cfbc4dbc3ad79d3a743a94fb2ebdee39c832e390f`. **The three required hashes are
EQUAL, and equal to runner_tip's cfg-A pose. No defect.** The 12-step default pose is
`0x6976399d3f4d3199` at W=1 and W=8 (`logs/07`), which is P0's 12-step J-family hash.

The gate can go red (`logs/09`): `runner_l4l2 --cfg default --workers 8 --steps 501 --expect-pose
dry/tip_cfga_W1.pose.bin` -> `pose_hash 0x9336b30a06a7d8af`, `"expect_pose":"mismatch: 64480 vs 64480
bytes, first differing body Some(0)"`, **exit 4**.

### 4.2 Armed de-risk (`logs/11_armed_derisk.log`, `dry/armed_W{1,8}.csv`)

`--cfg default --arm-profiler`, 12 steps: W=1 and W=8 both `void steps 0`, `drops 0`, `waves 1164`,
`solve on dispatcher 0 of 12`, pose `0x6976399d3f4d3199`, exit 0. The per-step anti-vacuity check is
green on the L4 tree at both W, so armed rows can be added to the window if wanted.

### 4.3 Jolt (`logs/10_jolt_dry_v5.3.0.log`, `logs/10_jolt_dry_v5.6.0.log`; cwd `dry/jolt_v5.x.y/`)

`PerformanceTest.exe -s=Pyramid -q=Discrete -t=1 -i=12 -f` and the same with `-receipt`, each exe:

| exe | banner | stat line (12 steps, W=1) | receipt file | receipt frame 0 |
|---|---|---|---|---|
| v5.3.0 | `boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0` (`=1` on the receipt run) | `Discrete, 1, ..., 0x69afedf436821a38` | `receipt_discrete_th1.csv` (`Frame, Manifolds, Points, Active Bodies, Top Y`) | `0, 4495, 11890, 1240, 35.9972763` |
| v5.6.0 | same | `Discrete, 1, ..., 0x781114657d662672` | same | `0, 4495, 11890, 1240, 35.9972763` |

Both 12-step hashes equal P0's dry-pass hashes for JOLT-T / JOLT56-T at every W
(`0x69afedf436821a38`, `0x781114657d662672`; P0 `dry/runs.jsonl` in the a5f69b98 scratchpad). Both exes
print `SSE2 SSE4.1 SSE4.2 AVX AVX2 F16C LZCNT TZCNT FMADD`. All four Jolt runs exit 0.

## 5. Tools (`win3/tools/`)

Copied, not imported in place (no `__pycache__` was written into `D:/wt/mq-aac562a7` or
`D:/wt/joltab/tools` -- checked with `find`): `window.py`, `wait_idle.ps1`, `launch_after_idle.py`,
`receipts_summary.py` (P0), `window_run.py`, `analyze_window.py`, `window_lib/pdhperf.py` (P0b), and
`driver.py` both at `tools/driver.py` and `tools/lib/driver.py` (the layout `window_run.py` imports from).
`D:/wt/joltab/tools/physics_parity/driver.py` is byte-identical to the copy in
`D:/wt/mq-aac562a7/tools/physics_parity/` and to P0b's `lib/driver.py`.

Things the window stage must change in `window_run.py` before running (not done here, it is the window's
file): the `P0` placeholder path and `TREE_DRIVER`/`WAIT_PS1`; `BOYKO_CELLS`/`JOLT_CELLS` (they list
P0b's rows, including the now-refused `J-P1`); the manifest path; `RAW` -> `win3/raw/`.

## 6. Run list: `win3/rows.json`

4 unconditional rows, 17 cells, 102 processes at K=6:

| row | exe | args | W |
|---|---|---|---|
| `D-L4L2` | `bin/runner_l4l2.exe` (`368d4104...`) | `--scene jolt --gap 0.5 --cfg default` | 1, 2, 4, 8, 16 |
| `J-A` | `bin/runner_tip.exe` (`ef9325ef...`) | `--scene jolt --gap 0.5 --cfg a` | 1, 8 |
| `JOLT-T` | `D:/tmp/jolt/build-v5.3.0-dist/PerformanceTest.exe` (`29b23ad1...`) | `-s=Pyramid -q=Discrete -f` | 1, 2, 4, 8, 16 |
| `JOLT56-T` | `D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe` (`918fd2b7...`) | `-s=Pyramid -q=Discrete -f` | 1, 2, 4, 8, 16 |

plus the driver's per-process suffix (`--workers W --steps 500 --window 0..500 --csv --pose-out --label`;
Jolt `-t=W -i=500`), the protocol block (K=6, 2 passes x 3 rounds, interleaved/reversed, alternating,
5-s/10-s receipts, 5 % gate with one re-run, no band void, P-none), the expected 500-step pose/hash per
row, and P0b's reference medians for the bridge check. Conditional block `D-L5` is present and **empty**:
`l5np` HEAD is still `aac562a7`, so no C4 runner exists; the block states what the window stage must do
to fill it (own worktree/target, `bin/runner_l5.exe` + hash, re-read the usage text for the
parallel-narrowphase flag -- `--parallel-np off` is the task's spelling, not a flag at any commit seen
here -- and the 500-step pose gate at W=1/8/16 before any timed process).

## 7. For the window stage

- Idle baseline first (`wait_idle.ps1`); the lanes were live during this stage.
- The `J-A` bridge cells are the window's link to P0b's table: if `J-A@W1` / `J-A@W8` move beyond the
  process spread from P0b's 19.671 / 9.162 ms, the window's boyko/Jolt ratios are not comparable to P0b's.
- No further builds are needed on D: unless C4 lands (then one more parity build, ~0.3 GB at this
  target's footprint).
- `D:/wt/mq-aac562a7` and `D:/wt/_targets/mq-aac562a7` were created by this stage and may be removed by
  the owner after the window; nothing else on disk was created outside `win3/`.

## Files

- `win3/bin/`: `runner_l4l2.exe`, `runner_tip.exe`, `SHA256SUMS` (4 entries)
- `win3/rows.json`
- `win3/logs/`: `01_worktree_add.log`, `02_git_log_ancestors.log`, `03_defaults_grep.log`,
  `04_build_runner_l4l2.log`, `05_build_rerun_noop.log`, `06_usage_text.log`,
  `07_cfg_summary_default_W1_W8.log`, `08_pose_500_W_invariance.log`,
  `09_pose_checks_and_red_control.log`, `10_jolt_dry_v5.3.0.log`, `10_jolt_dry_v5.6.0.log`,
  `11_armed_derisk.log`, `12_exe_diff.log`, `build_l4l2.sh`
- `win3/dry/`: the pose files, the per-step CSVs of the untimed runs, `jolt_v5.3.0/`, `jolt_v5.6.0/`
  (**not measurements**)
- `win3/tools/`: the copied P0/P0b tools and `driver.py`
- `win3/raw/`: empty -- the window stage's directory
