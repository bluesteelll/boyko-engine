# rapier-parity — Rapier 0.36 on boyko's J-T pyramid

The second parity comparator after Jolt 5.6. The owner asked for it on 2026-09-30: "also compare
with the Rapier engine — we must beat Rapier too".

This is an **external** cargo project. It is not a member of the boyko workspace and has its own
`Cargo.lock`, so the trunk never resolves Rapier's dependency tree. It is a benchmark comparator,
not an engine library. Its runner is shaped after boyko's fixed-window runner
`crates/boyko_physics/benches/jolt_parity_pyramid.rs` (trunk `3d9433ae`, `D:/wt/joltab`):

- one world per process;
- `--steps` steps from t = 0, with no warm-up;
- one `Instant` pair around each `PhysicsPipeline::step`, and nothing else inside it;
- every receipt, pose read and pool-size read happens after the pair closes.

**No timing verdict has been taken with it.** All gate runs are structural (counts, poses, thread
receipts, exit codes). The timing columns exist but nobody reads them: `analyze.py` and
`void_check.py` never print or aggregate one.

## Revision 2 (2026-09-30): what changed, and why

Rev 1 was built, audited (`gate/audit.md`) and reviewed (`gate/WF_review.md`). Rev 2 closes their
findings. Rev 1's sources and outputs are kept in `gate/prev_rev1/` as the baseline the rebuilt
`simd8` is compared with (poses, configs and counts must not move).

| item | finding | change |
|---|---|---|
| review W1 | `PhysicsPipeline::new()` turns Rapier's statistics counters on, and then `init_and_solve` walks every solver manifold on every step, serially, inside the timed pair. Our J-T runner counts after the pair closes. | Counters are **off** unless `--receipt` is given (`--counters on\|off` overrides). The state in force is read back from the pipeline and recorded as `config.counters_enabled`. The `--receipt` twins keep them on: they are the recount gate. |
| review W2 | V3 asked for a CSV `pool_threads` column that a timed command (no `--receipt`) never wrote. | The per-step CSV **always** carries `pool_threads`. `threads.pool_threads_min/max` were already in every SUMMARY. |
| review W3, audit P1 | Only an 8-lane build existed, and "maximum performance" was assumed. | A second timed arm, `simd4` (Rapier's 4-lane default, no `simd8`). Both are sha256-pinned and both get rows; the window decides which is faster. |
| audit P2 | The `--expect-pose` exit-4 path and the window validator had never been shown to fail. | Gate g5 (mutated / wrong / truncated / missing fixtures, `--steps 501`, the other arm's fixture, counters off under `--receipt`) and gate gv (`void_check.py` against 33 mutations, on two baselines). |
| audit P3 | `loop_on_pool_worker == false` was recorded but not voided. | `placement_void`: exit 3 in `--install loop`. The review notes that it cannot fire from the command line (inside `pool.install` the loop is always on a worker), so its red control is the unit test `placement_void_fires_only_for_a_loop_outside_the_pool`. |
| audit P4, review O1 | `build.sh` had no `--locked`. | Every cargo call passes `--locked`. |
| review O2 | Nothing tied an exe to a source. | The SUMMARY carries `source_fnv1a64` (`main.rs`, `Cargo.toml`, `Cargo.lock`); `build.sh` writes `gate/bin/SOURCES.sha256` next to `SHA256SUMS`. |
| review note | `matched` keeps `normalized_allowed_linear_error = 0.005`. | Not a difference: see "Differences `matched` does not remove". |
| review note | The audit's V9 reason (a deferred BVH task crossing a step) is wrong. | Corrected in `gate/audit.md` and `gate/window9a_rows.md`. |

Not done, as the audit marks them optional and not blockers: P5 (a `profiler` armed arm for a Rapier
stage map) and P6 (a `--gap` flag so Rapier can join F0f at gap 1.0).

## Layout

| path | what |
|---|---|
| `Cargo.toml`, `Cargo.lock` | `rapier3d = "=0.36.0"` with `parallel`. Arm features `simd8` (default), `simd4` (marker), `det` (`enhanced-determinism`); exactly one must be on. Release profile: `lto = "fat"`, `codegen-units = 1`. |
| `.cargo/config.toml` | `-C target-cpu=x86-64-v3`, target dir `D:/wt/_targets/rapier-parity` |
| `build.sh` | builds the three arms with `--locked`, copies them to `gate/bin/`, writes `SHA256SUMS`, `SOURCES.sha256` and `features_<arm>.txt` |
| `src/main.rs` | the runner (module docs: scene, builds, counters, configs, flags, output) |
| `gate/run_gates.sh` | the untimed gates g1, g2, g2t, g2cpu, g3, g5 on both timed arms, and g2det on the `det` arm |
| `gate/analyze.py` | reads every run and writes `gate/gates.txt`, `gate/gates.json` and `gate/pins.json`. Exit 0 iff every check passed. |
| `gate/void_check.py` | the reference implementation of the window's void rules V1-V9 for one timed process |
| `gate/mutate_pose.py` | makes the mutated fixtures of g5 |
| `gate/make_rows.py`, `gate/window9a_rows.md` | the window-9a Rapier rows, generated from `pins.json` |
| `gate/fixtures/` | `RD8`, `RM8`, `RD4`, `RM4`: the final poses the timed rows are compared with |
| `gate/f0_metrics.py` | a verbatim copy of the F0 ensemble's `metrics.py` (sha256 `ca8673d8…`), so Rapier's piles are scored by the code that scored ours and Jolt's |
| `gate/simd8/`, `gate/simd4/` | per arm: `g1 g2 g2t g2cpu g3 g5`, every run's log (stdout + stderr + `rc=`), CSV and pose |
| `gate/det/g2det/` | the determinism receipt |
| `gate/audit.md`, `gate/WF_*.md` | the equal-work audit, and the research, build report, audit and review as received (`WF_audit.md` is the untouched original of `audit.md`) |
| `gate/prev_rev1/` | rev 1's sources and gate outputs |

## Build

```bash
bash D:/tmp/rapier-parity/build.sh      # JOBS=8 by default; about 10 minutes for the three arms
```

The script runs with `PATH="$HOME/.cargo/bin:$PATH"`, `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc`
(rustc 1.98.1), `CARGO_TARGET_DIR=D:/wt/_targets/rapier-parity` and
`RUSTFLAGS="-C target-cpu=x86-64-v3"`. That is the same ISA as boyko's runner and Jolt's parity
build. Every cargo call passes `--locked`.

| arm | command | rapier3d features | parry3d features | lanes | timed |
|---|---|---|---|---|---|
| `simd8` | `cargo build --release --locked` | `default` (`dim3`, `f32`, `std`) + `parallel` + `simd8` | `parallel` + `simd8` | 8 | yes |
| `simd4` | `cargo build --release --locked --no-default-features --features simd4` | `default` + `parallel` | `parallel` | 4 | yes |
| `det` | `cargo build --release --locked --no-default-features --features det` | `default` + `parallel` + `enhanced-determinism` | `parallel` + `enhanced-determinism` | 4 | no |

Pinned by `gate/bin/SHA256SUMS` (written by `build.sh`, 2026-09-30; `analyze.py` re-hashes the exes):

| exe | sha256 |
|---|---|
| `gate/bin/rapier-parity-simd8.exe` | `736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307` |
| `gate/bin/rapier-parity-simd4.exe` | `1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77` |
| `gate/bin/rapier-parity-det.exe` | `f34291c5ffb8ee130e2c5eab92fbc4d8ad42f542db24ebfbfd6c19f921f3219d` |

Rev 1's `simd8` (`24808408…`) and `det` (`f9f66f9f…`) are superseded: the source changed, so the pins
changed. The sources they were built from are in `gate/bin/SOURCES.sha256`.

**What the arms are.** rapier3d 0.36 has no `simd-stable` (removed in 0.35) and no `simd4` feature.
Its SIMD is always on through `wide`, at 4 lanes, and `simd8` (`simd8 = ["parry3d/simd8"]`, AVX2)
widens it to 8. So the 4-lane arm is "no `simd8`", and the harness's `simd4` feature is a marker that
enables nothing in rapier3d: it only names the arm so the exe can say so (`arm`, `simd_lanes`,
`features.simd4` in the SUMMARY). `main.rs` refuses to compile unless exactly one of
`simd8 | simd4 | det` is on. `simd8` and `enhanced-determinism` cannot be built together (rapier3d
`lib.rs:19-21`), and Cargo unifies features per build, so each arm is its own build.

**Why two timed arms.** rapier3d caps its solver workers at
`num_workers.clamp(1, (num_two_body / SIMD_WIDTH / 16).max(1))` (`staged_island_solver/init.rs:258-273`).
With 8 lanes all 16 workers are allowed only from about 2,048 manifolds, against about 1,024 with 4
lanes. So the 8-lane build may lose at high W, during the landing steps of [0,100) in particular.
That is a hypothesis, not a finding: the window decides, by giving both arms rows.

`gate/bin/features_<arm>.txt` holds `cargo tree --locked -e features` for each arm, and `analyze.py`
asserts that the two timed arms differ in exactly the rapier3d `simd8` feature. No arm has
`profiler`, `solver-bounds-checks` or `unsync-callbacks`. Each run's SUMMARY records its features,
its arm, its lane count and its `target_feature` receipt (`avx2`, `fma`, `bmi2` are all true).

## Run

Run the exe by path, never through cargo:

```bash
B=D:/tmp/rapier-parity/gate/bin/rapier-parity-simd8.exe      # or rapier-parity-simd4.exe
$B --cfg rapier-default --workers 8 --steps 500 --window 100..500 --csv w8.csv --pose-out w8.pose
$B --cfg matched --workers 1 --steps 500 --receipt --csv m1.csv
$B --cfg matched --workers 1 --steps 500 --perturb 112:z:+1 --pose-out p2.pose
```

| flag | meaning |
|---|---|
| `--cfg rapier-default\|matched` | solver configuration (below); default `rapier-default` |
| `--workers W` | a rayon pool of exactly W threads (`rapier3d::rayon`, the crate Rapier steps on); default 1 |
| `--steps N` | steps from spawn; default 500 |
| `--window A..B` | the summary's timing window, `A < B <= N`; default `0..N`. This is how warm-up steps are excluded. |
| `--install loop\|step` | `loop` (default): one `pool.install` around the whole loop. The timed pair holds only `step`, and the loop thread is Rapier's inline worker 0, so W threads do the parallel work while the caller blocks outside. `step`: `pool.install` per step from the main thread, with the pair around it. This is Rapier's own `configure_thread_pool` shape and it prices the injection. Both give the same pose bytes (g2). |
| `--receipt` | per-step counts (untimed) into the CSV and the summary (below). Turns Rapier's counters on. |
| `--counters on\|off` | Rapier's statistics counters. Default: on iff `--receipt`. |
| `--csv PATH` | per-step CSV: `step, wall_ns, pool_threads`, plus the receipt columns under `--receipt` |
| `--pose-out PATH` | final pose bytes in boyko's format (below) |
| `--spawn-pose-out PATH` | the same format, written after spawn and before step 0 |
| `--expect-pose PATH` | compare the final pose bytes with a file; exit 4 on a mismatch, 2 if the file is unreadable |
| `--perturb B:AXIS[:DIR]` | move box B's spawn coordinate AXIS (`x`/`y`/`z`) by one f32 ulp. DIR is `+1` (default) or `-1`. This is the F0 ensemble's `BOYKO_F0_PERTURB` as a flag. It prints the same `F0_PERTURB …` line to stderr and refuses a zero coordinate. |
| `--label TEXT` | echoed into the summary |

**Counters.** `PhysicsPipeline::new()` starts with `Counters::new(true)`. With them on,
`init_and_solve` walks every solver manifold on every step
(`staged_island_solver/init.rs:601-610`: `graph.buckets()…store.get(*r).data.solver_contacts.len()`),
serially, before the parallel phase, and inside the timed pair. That walk is the only work the flag
gates: without the `profiler` feature the timers are no-ops, and nothing in the dynamics reads
`counters.solver.*`. So a timed run turns the counters off, and the final pose does not depend on
them (g2t checks the timed shape's pose against the counters-on fixture at every W and arm). The
counters' `ncontact_pairs` is assigned unconditionally (`solve.rs:158`), so it stays correct with the
counters off; `nconstraints` and `ncontacts` are not written, and stay 0. Gate g5 uses that: with
`--receipt --counters off` the recount must go void on exactly those two counters.

**Output.** stdout has a few readable lines, then one `SUMMARY {json}` line. The summary carries:

- the build: arm, lane count, features, target features, `source_fnv1a64`;
- the full `IntegrationParameters` in force, and `counters_enabled`, as `config`;
- `scene_identity`;
- `perturb`;
- the pose hash (FNV-1a 64);
- the final and `[window]`-mean counts (under `--receipt`);
- the receipt gates;
- `threads`: pool size requested, `pool_threads_min/max` (`rayon::current_num_threads()` read inside
  the install after every step in loop mode), the install receipt, whether the loop ran on a pool
  worker, logical cores.

**Pose bytes.** 13 little-endian `f32` per dynamic body, in spawn order: position, linear velocity,
rotation (x, y, z, w), angular velocity. This is byte for byte boyko's `pose_bytes`, so
`metrics.py` and the equal-work scripts read it unchanged.

**CSV.** `step, wall_ns, pool_threads` always (`pool_threads` is 0 under `--install step`, where it is
not read). Under `--receipt` the receipt columns follow, all read after the step:

| column | meaning |
|---|---|
| `bp_pairs` | contact-graph edges, i.e. the broad-phase pairs |
| `pairs_with_manifold`, `pairs_active` | pairs with at least one raw manifold, and pairs with at least one solver contact |
| `raw_manifolds`, `raw_points` | manifolds and points before the solver's filter |
| `manifolds` (Nm), `points` (Np) | solver manifolds with at least one solver contact, and their solver contacts |
| `spec_points`, `spec_manifolds` | solver contacts with `dist > 0`, and manifolds whose every point is speculative |
| `floor_manifolds`, `floor_points` | solver manifolds against the floor, and their points |
| `lateral_manifolds` | solver manifolds with normal \|n.y\| < 0.5 |
| `counter_pairs`, `counter_constraints`, `counter_contacts` | Rapier's own `counters.cd.ncontact_pairs`, `counters.solver.nconstraints` and `counters.solver.ncontacts` |
| `rows` | solver row-iterations per step |
| `awake` | `IslandManager::num_active_bodies` |
| `top_y` | the top box's height |

`SolverContact.dist` can be stale on recycled pairs, so the speculative split is approximate.

**Exit codes.**

| code | meaning |
|---|---|
| 0 | ok |
| 2 | bad flag, an unwritable file, or an unreadable `--expect-pose` file |
| 3 | void: a receipt gate failed. The gates are: not 1240 dynamic + 1 fixed body; first/last/AABB/mass not equal to boyko's; a pool-size read ≠ W; under `--install loop`, the loop not on a pool worker; under `--receipt`, a body asleep, or Rapier's counters disagreeing with the recount at any step. |
| 4 | `--expect-pose` mismatch |

## The scene, and the transcription evidence

J-T is Jolt 5.6's `PerformanceTest/PyramidScene.h` (in `D:/tmp/jolt/wt-v5.6.0-parity`, parity patch
applied), which boyko's `spawn_scene` (`--scene jolt`) transcribes index for index.

| item | Jolt 5.6 `PyramidScene.h` | boyko `jolt_parity_pyramid.rs` | this harness `src/main.rs` |
|---|---|---|---|
| floor | `BoxShape(Vec3(50,1,50), 0)` at `(0,−1,0)`, static (`:35`) | `FLOOR_HALF_EXTENTS` (`:469`), `spawn_box((0,−1,0), …, false)` (`:1051`) | `FLOOR_HALF_EXTENTS`/`FLOOR_CENTER`; `RigidBodyBuilder::fixed()` + `ColliderBuilder::cuboid(50,1,50)` (`:540-542`) |
| constants | `cBoxSize 2`, `cBoxSeparation 0.5`, `cHalfBoxSize 1`, `cPyramidHeight 15` (`:37-40`) | `BOX_SIZE`, `JOLT_SEPARATION`, `HALF_BOX`, `PYRAMID_HEIGHT` (`:457-463`) | same names/values (`BOX_SIZE`, `BOX_SEPARATION`, `HALF_BOX`, `PYRAMID_HEIGHT`) |
| box shape | `BoxShape(sReplicate(1), 0)`: no convex radius (`:42`) | `ColliderShape::Box { half_extents: (1,1,1) }` | `ColliderBuilder::cuboid(1,1,1)`: parry cuboid, no margin, `contact_skin` 0 |
| loop | `i∈[0,15)`, `j,k∈[i/2, 15−(i+1)/2)` (`:45-47`) | same (`:1056-1060`) | same (`:547-551`) |
| position | `(−15 + 2j + (i&1?1:0), 1 + 2.5i, −15 + 2k + (i&1?1:0))` (`:49`) | `-(15 as f32) + BOX_SIZE*j + odd`, `1.0 + (BOX_SIZE+gap)*i`, … (`:1062-1066`) | boyko's expression, operand for operand (`:553-558`), so the f32 bits are boyko's |
| rotation, velocity | `Quat::sIdentity()`, 0 | `Quat::IDENTITY`, 0 | builder defaults: identity, 0 |
| damping | 0 (parity patch, `:52-53`) | none exists | builder default 0 |
| mass | Jolt density 1000 → m = 8000 | `BOX_INV_MASS 0.125`, `BOX_INV_INERTIA 0.1875` (m = 8, `:477-479`) | density 1 (Rapier's default) → m = 8. Uniform density does not change the dynamics against a static floor. |
| friction / restitution | 0.2 (`mFriction` default), √(0.2·0.2) combine / 0 | `JOLT_FRICTION 0.2`, max-combine / 0 | 0.2 on every collider, `CoefficientCombineRule::Max` (boyko's rule) / 0. Every rule gives 0.2 here: max, √, and Rapier's default Average. |
| sleeping | `mAllowSleeping = false` | `--sleeping off` | `.can_sleep(false)` on every box (`:566`) |
| CCD | `-q=Discrete` | none | `max_ccd_substeps = 0` (`:416`), which also disables Rapier's automatic fixed-collider CCD |
| gravity, dt | (0,−9.81,0), 1/60, one collision step | same | same, one `step` per frame |

**Runtime evidence** (`gate/gates.txt`, g1, both arms). Both configs match on every item:

- 1240 dynamic + 1 fixed bodies, 1241 colliders.
- First box (−15, 1, −15) and last (top) box (−1, 36, −1), both equal to boyko's.
- Spawn AABB (−16, 0, −16)..(14, 37, 14), as expected.
- Inverse mass 0.125 and inverse inertia 0.1875 on all three axes: **0 ulp** from boyko's constants.
- All boxes uniform.
- All 1240 × 3 spawn coordinates **bitwise equal** to the F0 lattice (`f0_metrics.LAT`, the
  independent Python transcription that scored our and Jolt's piles).
- Velocities 0, rotations identity.
- Spawn hash `0x03c5b4c9910d4d29`, equal across both configs and both arms.
- Friction combined to 0.2, and every solver manifold of step 1 carries friction 0.2.

**Anti-vacuity.** A 1-ulp move of box 0 (`--perturb 0:x:+1`) fails the bitwise lattice check on
exactly 1 body.

**Jolt's body order.** Jolt's final pose (`equal-work/jolt_final_pose.bin`) has body i in lattice
site i's column and on its layer's rest height for 1240 of 1240 bodies. So Jolt's creation order is
this lattice order too.

## Configurations (`--cfg`)

Both configs carry the scene invariants: dt = 1/60 and CCD off.

| parameter | `rapier-default` | `matched` | boyko (trunk) | Jolt 5.6 |
|---|---|---|---|---|
| substeps × (biased + relax) | 4 × (1 + 1) | 4 × (1 + 2) | 4 × (1 + 2) | 10 velocity + 2 position |
| friction in the biased pass | no | yes | yes | — |
| friction rows | Simplified: 2 + twist per manifold | Coulomb: 2 per point | 2 per point | 2 + twist per manifold |
| softness | 30 Hz / ζ 10; floor 60 Hz | 30 Hz / ζ 10 everywhere | 30 Hz / 10 | Baumgarte 0.2, 0.02 slop |
| bias cap | 3 m/s | 4 m/s | 4 m/s | — |
| speculative distance | 0.02 | 0.02 | 0 today (V2 in flight: 20 mm) | 0.02 |
| contact reuse | recycling at 5 cm | recycling at 1 mm | 1 mm | 1 mm cache |
| gyroscopic | on | off | none | off |
| rows per step | 8·Np + 12·Nm | 36·Np | 36·Np | 12·Np + 30·Nm |

Rows are solver row-iterations per step, the Jolt dossier's equal-work unit. They come from
Rapier's own loop (`staged_island_solver/worker.rs:269,616,713`, `solve.rs:98`):
`S·[P·(Np + fib·F) + Q·(Np + F)]` with `F = 3·Nm` (Simplified) or `2·Np` (Coulomb). Warm start is
not counted. The runner computes the rows from the measured Np and Nm every step.

**Differences `matched` does not remove:**

- Rapier's angular speed cap (π/4 per step), which Jolt also has and boyko does not.
- Rapier's 400 m/s linear cap.
- parry's contact generation and its reduction to at most 4 points per manifold.
- Rapier's recycling test (relative-pose drift), against boyko's per-record reuse rule.

**Checked and found not to be a difference:** `normalized_allowed_linear_error = 0.005` stays at
Rapier's default in both configs, and boyko has no equivalent slop in `boyko_physics/src`. In
rapier3d 0.36 that parameter is read only by the CCD solver (`ccd_solver.rs:130,182`), which is off
here. The contact solver applies no slop deadzone: `contact_with_coulomb_friction.rs:422` says
"`allowed_linear_error` is geometric slop, not a solver deadzone". So the row counts and the
trajectory do not depend on it.

**Conditions that belong on every claim made with this harness:**

- Rapier is built with `codegen-units = 1` (per the brief); boyko's `parity` profile
  (`D:/wt/joltab/Cargo.toml`) inherits `release`: fat LTO with the default 16 codegen units. boyko's
  root `Cargo.toml` records, from a 2026-09-04 matrix on its ECS benchmarks, that fat LTO with
  `codegen-units = 1` won 5 of 7 benchmarks and regressed one by 17.4 %, so boyko does not use it.
  What `codegen-units = 16` would do to Rapier has not been measured here.
- Rapier's statistics counters are off in the timed rows. Its own default is on, and the field is
  documented "benchmarking only" (`physics_pipeline/mod.rs:47`).
- `rapier-default` is Rapier's defaults plus the scene invariants (dt, CCD off); `matched` is the
  research's closest match, and the list above is what it does not remove.

## Gates (2026-09-30, `gate/`)

Command: `bash gate/run_gates.sh`, then `python gate/analyze.py`, which writes `gates.txt`,
`gates.json` and `pins.json` and exits 0 iff every check passed. **242 of 242 checks passed.** The 144
logged runs (70 per timed arm, 4 `det`) all ran with the exit code their gate expects; the only
non-zero exits are the ones a red control provokes on purpose.

- **g1 — scene identity (both arms):** as above, every item, for both configs; the 1-ulp mutant turns
  the lattice check red on exactly 1 body; spawn hash `0x03c5b4c9910d4d29` everywhere.
- **g2 — threads and determinism (both arms, `--receipt`, counters on):**
  - W = 1/2/4/8/16, 500 steps. Every step's pool read equals W, the install receipt is `[W, W]`, and
    the loop ran on a pool worker.
  - Every body was awake at every step, and Rapier's own pair, constraint and contact counters equal
    the recount at all 500 steps.
  - The **final pose bytes and all 500 steps' counts are identical at every W**, in both configs and
    on both arms (`rapier-default` `0x36e6142dc4db4701`, `matched` `0x29f2b3226e7e46c4`). Neither
    timed build has `enhanced-determinism`, so this is Rapier's colouring-only determinism, observed
    and not only claimed.
  - **The two arms reach the same poses and the same counts.** The 8-lane and 4-lane builds are
    different exes (different sha256, different resolved features) on the same trajectory, so
    `RD4 = RD8` and `RM4 = RM8` as files.
  - The rebuilt `simd8` reproduces rev 1's poses and rev 1's row counts (303,003 / 354,674 and
    935,729 / 1,071,107 rows per step over [0,100) / [100,500)) exactly, and its `config` object
    equals rev 1's apart from the new `counters_enabled`. So neither the counters change nor the CSV
    change moved a trajectory.
  - Repeat runs at W1 and W8 are identical, and so is `--install step` at W8.
  - g2cpu (process CPU time over wall, 200 steps): about 1 at W1 and well above 1 at W8 and W16 on
    both arms, so the step ran on several threads at once. It is a thread receipt, not a timing
    verdict, and load can only lower it.
- **g2t — the timed shape (both arms, no `--receipt`, counters off, csv + pose-out + `--expect-pose`),
  W = 1/2/4/8/16, both configs = 20 runs.** Each one exits 0, matches its fixture, writes the CSV
  header `step,wall_ns,pool_threads` with `pool_threads == W` on all 500 rows, reports
  `config.counters_enabled: false`, and passes `void_check.py` V1-V9.
- **g2det — the `det` build:** W1 = W8 bitwise in both configs (`0x976ee1e69ff0602b`,
  `0xb9b341b123a3ffcc`), equal to rev 1's `det` build. The trajectory differs from the `simd` builds;
  that arm also uses libm, so the difference is not attributable to the lane count alone.
- **g3 — fidelity** (F0 metrics; W1; U plus the F0 ensemble's eight 1-ulp seeds; both arms give the
  same numbers because they have the same trajectories). "Holds" means 0 boxes > 0.5 m and max drift
  < 0.5 m.

  | | holds | boxes > 0.1 m | max drift (mm) | max rotation (°) |
  |---|---|---|---|---|
  | Rapier `rapier-default`, simd8 and simd4 | **9/9** | 76–89 | 210–285 | 0.40–1.20 |
  | Rapier `matched`, simd8 and simd4 | **9/9** | 5–13 | 149–293 | 0.88–1.80 |
  | Jolt 5.6 (one run) | holds | 0 | 51.2 | 0.54 |
  | boyko trunk 16191fda U | **fails** (26 boxes > 0.5 m) | 190 | 2183 | 28.76 |

  Rapier's pile holds on the symmetric-brick J-T scene in every trajectory. dimforge's own
  `pyramid3` deliberately avoids that scene as "marginally stable". The pile is looser than Jolt's
  (3–6× the max drift) and tighter than boyko's trunk.

  4 of 8 (`rapier-default`) and 3 of 8 (`matched`) perturbed runs end more than 0.1 m from U
  somewhere. That is below F0's "chaos" bar of 6 of 8.
- **g4 — both configs, both arms**, per-step means at W1, `--receipt` twin. Nm and Np are given as
  [0,100) / [100,500):

  | | manifolds (Nm) | points (Np) | rows, [0,100) | rows, [100,500) |
  |---|---|---|---|---|
  | Rapier `rapier-default` (either arm) | 6,891.2 / 8,070.0 | 27,538.5 / 32,229.2 | 303,003 | 354,674 |
  | Rapier `matched` (either arm) | 6,683.6 / 7,662.4 | 25,992.5 / 29,753.0 | 935,729 | 1,071,107 |
  | boyko trunk (dossier F3/F5, [100,500)) | 4,467.7 | 16,554 | — | 595,935 |
  | Jolt 5.6 (window 7 receipt, [100,500)) | 8,489 | 31,112 | — | 565,790 velocity + 62,224 position |

  Rapier keeps Jolt's 0.02 m speculative contacts, so its contact set is Jolt-sized. Its defaults do
  **0.60× our row-iterations** and **0.63× Jolt's velocity rows**; `matched` runs our 36·Np formula
  on a Jolt-sized contact set and does **1.80× our rows**, close to the 1.68× the F0 verdict
  predicts for our own V2 (20 mm speculative) trunk. A wall-clock comparison must state which
  configuration it uses and divide by rows as well as report the ratio.
- **g5 — red controls of the pose gate (each arm × each config, W4).** A timed-shape run against a
  mutated fixture must exit 4:
  - the fixture with one bit of body 0, 617 or 1239 flipped (a 1-ulp move of x): exit 4, and the exe
    names that body;
  - the fixture truncated or padded by one body: exit 4;
  - the other config's fixture: exit 4;
  - `--steps 501` against the 500-step fixture: exit 4;
  - a missing fixture: exit 2;
  - the other arm's fixture: exit 0, because the arms' fixtures are the same bytes (recorded, not a
    failure);
  - **counters off under `--receipt`: exit 3.** Rapier's `nconstraints` and `ncontacts` stay 0
    (the statistics block at `init.rs:601` is skipped), so the recount gate is void on 500 of 500
    steps for both, while `ncontact_pairs` still agrees (`solve.rs:158` is unconditional). This shows
    two things: the recount gate can fail, and turning the counters off really removes the walk;
  - `--counters on` in the timed shape: exit 0, `config.counters_enabled: true`, and `void_check.py`
    flags V5.
- **gv — the validator's red controls.** `void_check.py` on two baselines (simd8 `rapier-default` W8,
  simd4 `matched` W16) is valid unmutated, and each of 33 mutations per baseline (66 in all) turns
  exactly the rules it targets red: exit code, hang, SUMMARY count and syntax, pool receipts (min,
  max, install, placement, one CSV row, the CSV column absent), sha256 swapped between arms, arm
  label, features (`simd8`, `simd4`, determinism), version, `avx2`, source hash, config fields
  (`num_solver_iterations`, `counters_enabled`, the other config's object), spawn hash, `perturb`,
  scene identity, a 1-ulp pose change, the exe's own `expect_pose`, a 499-row CSV, `wall_ns = 0`,
  receipt columns in a timed CSV, `--receipt` in the args, and a failed prep twin.
- **Unit test:** `cargo test --locked` runs `placement_void_fires_only_for_a_loop_outside_the_pool`
  (the loop-placement void cannot fire from the command line, so its red control is a test).
- **Every gate run's SUMMARY names the current sources** (`source_fnv1a64` equals the FNV-1a 64 of
  `main.rs`, `Cargo.toml` and `Cargo.lock` on disk), and `analyze.py` re-hashes the exes and the
  sources against `SHA256SUMS` and `SOURCES.sha256`.

Nothing above reads a timing column. The window's rows are in `gate/window9a_rows.md`.
