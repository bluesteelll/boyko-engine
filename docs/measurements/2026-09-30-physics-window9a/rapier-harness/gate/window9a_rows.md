# Window 9a rows: Rapier 0.36 (rev 2, 2026-09-30)

Supersedes section 7 of `audit.md`. The harness rework it asked for is closed: a 4-lane arm (P1), the
pose and validator red controls (P2), the loop-placement void (P3) and `--locked` (P4), plus the
review's W1 (counters) and W2 (`pool_threads` in the timed CSV). **No timing was read** to produce any
of it. Gate result at the time of writing: 242 of 242 checks passed
(`gate/gates.txt`), the validator's red controls included.

## Preconditions

- V2 is on the trunk. Ours is the V2 trunk runner's row `J-T` with its `JT500` fixture; its armed twin
  `J-T-a`, in the same window, supplies our rows census.
- The exes are copied to `$W/bin` and the fixtures to `$W/fixtures`. Rebuilds go through `build.sh` only.
- D: has at least 15 GB free.
- The idle-rule and witness exe lists include `rapier-parity-simd8.exe` and `rapier-parity-simd4.exe`.
  Any process under `D:/wt/_targets/rapier-parity` counts as a build.
- The prep gate (`bash gate/run_gates.sh; python gate/analyze.py`) is green on the exes being copied.

## Binaries

Both are timed. Which is faster is **decided in the window**, from rows at every W for both. It is not
assumed, and neither arm is labelled "maximum performance".

| key | exe | sha256 | SIMD lanes |
|---|---|---|---|
| rs8 | `bin/rapier-parity-simd8.exe` | `736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307` | 8 |
| rs4 | `bin/rapier-parity-simd4.exe` | `1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77` | 4 |

What each arm enables (resolved by `cargo tree -e features --locked`, saved in `gate/bin/features_<arm>.txt`):

- **rs8**: `cargo build --release --locked`. rapier3d 0.36.0 with `default` (`dim3`, `f32`, `std`) +
  `parallel` + `simd8`; parry3d with `parallel` + `simd8`. 8-lane f32 through `wide`, AVX2.
- **rs4**: `cargo build --release --locked --no-default-features --features simd4`. rapier3d 0.36.0 with
  `default` + `parallel` and **no** `simd8`; parry3d with `parallel`. rapier3d 0.36 has no `simd4`
  feature: its SIMD is always on through `wide` at 4 lanes, and `simd8` is the only widening. The
  harness's `simd4` feature is a marker that enables nothing in rapier3d; it names the arm so the exe
  can report it.
- Both: fat LTO, `codegen-units = 1`, `-C target-cpu=x86-64-v3`, rustc 1.98.1 stable msvc. No
  `profiler`, `enhanced-determinism`, `solver-bounds-checks` or `unsync-callbacks`.
- The two arms differ in exactly one rapier3d feature (`simd8`); the analysis asserts it.
- The `det` build (`enhanced-determinism`, 4 lanes, libm) is a receipt build and is never timed.
- Rapier's statistics counters are **off** in every timed row and on in the `--receipt` twins
  (`config.counters_enabled`; see V5 and V9).
- Stated condition on every claim: Rapier is built with `codegen-units = 1`; our trunk's `parity`
  profile is fat LTO with `codegen-units = 16`.

## Fixtures and pins

Spawn hash `0x03c5b4c9910d4d29`. The fixture is the final pose of the W1 `--receipt` run; the timed
shape reproduces it at every W (gate g2t), and the rebuilt rs8 reproduces rev 1's poses bit for bit.

| fixture | file | pose hash | sha256 |
|---|---|---|---|
| RD8 | `fixtures/RD8.pose` | `0x36e6142dc4db4701` | `06d2df2fefc09807...` |
| RD4 | `fixtures/RD4.pose` | `0x36e6142dc4db4701` | `06d2df2fefc09807...` |
| RM8 | `fixtures/RM8.pose` | `0x29f2b3226e7e46c4` | `a927ee84570fb6ef...` |
| RM4 | `fixtures/RM4.pose` | `0x29f2b3226e7e46c4` | `a927ee84570fb6ef...` |

- **rs8 and rs4 reach bit-identical final poses in both configs** (the SIMD width does not move the trajectory, and neither do their counts), so `RD4` and `RD8` are the same bytes, and so are `RM4` and `RM8`. Each arm still names its own fixture, so a future build where they diverge needs no change to the rows. Consequence: the pose check cannot tell which arm produced a run. V4 does: the exe sha256, `arm` and `features` (gv mutations "exe sha256 swapped" and "SUMMARY arm label swapped" are red).
- Count pins are every non-timing, non-`pool_threads` column of `{arm}/g2/{cfg}_W1.csv`; their sha256
  is in `pins.json` (`count_pin_sha256`).
- Config pins are `pins.json` `config_timed` per (arm, cfg): the SUMMARY `config` object with
  `counters_enabled: false`. The `--receipt` twins' object is the same with `counters_enabled: true`.
  Apart from that field it equals rev 1's object, and the simd4 object equals the simd8 one.
- `rows_pin` (per-step means of the row-iteration count, from the W1 twin):

| cfg | arm | rows over [0,100) | rows over [100,500) | non-timing columns, 500 steps |
|---|---|---|---|---|
| RP-D | rs8 | 303,003 | 354,674 | `5bbda70d94a8b3ab...` |
| RP-D | rs4 | 303,003 | 354,674 | `5bbda70d94a8b3ab...` |
| RP-M | rs8 | 935,729 | 1,071,107 | `c837a7dd725fec3a...` |
| RP-M | rs4 | 935,729 | 1,071,107 | `c837a7dd725fec3a...` |

## Rows

Kind `rapier`, block `PAR` (shared with ours `J-T`; `H-jolt56` may share it). Every cell has both arms.

```json
{"id":"RP-D","blocks":["PAR"],"kind":"rapier","binaries":["rs8","rs4"],"args":["--cfg","rapier-default","--install","loop"],"workers":[1,2,4,8,16],"steps":500,"window":[0,500],"metric_windows":[[0,100],[100,500]],"armed":false,"config_pin":"RD","pose_ref":{"rs8":"RD8","rs4":"RD4"},"rows_pin":{"rs8":{"0..100":303003,"100..500":354674},"rs4":{"0..100":303003,"100..500":354674}}}
{"id":"RP-M","blocks":["PAR"],"kind":"rapier","binaries":["rs8","rs4"],"args":["--cfg","matched","--install","loop"],"workers":[1,2,4,8,16],"steps":500,"window":[0,500],"metric_windows":[[0,100],[100,500]],"armed":false,"config_pin":"RM","pose_ref":{"rs8":"RM8","rs4":"RM4"},"rows_pin":{"rs8":{"0..100":935729,"100..500":1071107},"rs4":{"0..100":935729,"100..500":1071107}}}
```

## Exact timed commands

Run in the process directory (`bin/` and `fixtures/` beside it). The driver appends
`--label <row>_<bin>_W<W>_p<n>r<k>`. There is no `--receipt` and no `--counters`, so the counters are
off. 20 cells:

```
bin/rapier-parity-simd8.exe --cfg rapier-default --install loop --workers 1  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD8.pose
bin/rapier-parity-simd8.exe --cfg rapier-default --install loop --workers 2  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD8.pose
bin/rapier-parity-simd8.exe --cfg rapier-default --install loop --workers 4  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD8.pose
bin/rapier-parity-simd8.exe --cfg rapier-default --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD8.pose
bin/rapier-parity-simd8.exe --cfg rapier-default --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD8.pose
bin/rapier-parity-simd8.exe --cfg matched        --install loop --workers 1  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM8.pose
bin/rapier-parity-simd8.exe --cfg matched        --install loop --workers 2  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM8.pose
bin/rapier-parity-simd8.exe --cfg matched        --install loop --workers 4  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM8.pose
bin/rapier-parity-simd8.exe --cfg matched        --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM8.pose
bin/rapier-parity-simd8.exe --cfg matched        --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM8.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 1  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 2  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 4  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 1  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 2  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 4  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
```

## Protocol

- Ruling 1: K = 9 = 3 passes x 3 rounds, with p0 reversed, p1 forward, p2 reversed.
- Forward order within a W group: ours `J-T`, RP-D rs8, RP-D rs4, RP-M rs8, RP-M rs4. Ours and Rapier
  stay adjacent in every round.
- One untimed warm-up **per exe** per pass (RP-D rs8 W8, RP-D rs4 W8), the idle rule per pass,
  receipts at 5 % and witness at 2 %, a re-run once at the end of the pass, P-none.
- A pass-cell with K < 3 does not gate (8b ruling 8).
- The per-process value is the mean of `wall_ns` over the metric window from `run.csv`. **The primary
  window is [100,500); [0,100) is measured, claimed under the same rule and reported separately.** The
  step-0-excluded [1,100) mean is a sensitivity check and is not claimed, because step 0 carries each
  engine's one-time costs.
- The placement receipt is recorded only. In loop mode the step runs on pool worker 0 while the main
  thread blocks, so main-thread share means nothing for Rapier.
- Size: 20 cells x 9 = 180 timed Rapier processes, plus ours `J-T`, warm-ups and re-runs. Minutes come
  from the prep dry run.

## Void rules

Any one voids the process, and the protocol's re-run applies. `gate/void_check.py` is their executable
form: it takes only what a timed command line produces (the log with its `SUMMARY` line, `run.csv`,
`final.pose`, the exe) and it is what the g2t gate ran on all 20 (arm, cfg, W) timed-shape twins.
66 mutations of it (two baselines x 33) are its red controls; each turns exactly the rules
it targets red.

- **V1**: exit != 0 (2 usage, 3 receipt gate, 4 pose mismatch), a crash, or a hang (> 300 s, terminated
  by the driver's own handle).
- **V2**: not exactly one parsable `SUMMARY` line.
- **V3**: pool-size receipt != W. All of these must hold: `threads.pool_threads_min ==
  threads.pool_threads_max == W`; `threads.install_receipt == [W, W]`; `threads.loop_on_pool_worker ==
  true`; **and the CSV `pool_threads` column is W on all 500 rows**. *Amended:* the column is now
  written with or without `--receipt`, so the clause is checkable on a timed row. The exe also voids
  itself (exit 3) if the loop ran outside the pool.
- **V4**: build receipt differs. rapier3d must be 0.36.0; `features` must equal the arm's
  (rs8 `{simd8: true, simd4: false}`, rs4 `{simd8: false, simd4: true}`, `parallel` true and
  `enhanced_determinism` false for both); `arm` and `simd_lanes` must match; avx2, fma and bmi2 must
  be true; debug assertions off; the exe sha256 must equal its pin; `source_fnv1a64` must equal the
  pinned sources. *Amended:* arm, lanes and source hashes are new.
- **V5**: the `config` object differs from the (arm, cfg) pin in any field. *Amended:* it now includes
  `counters_enabled`, which must be `false` in every timed row.
- **V6**: scene identity is not all-true (1240 + 1 bodies, first/last/AABB equal to ours, uniform
  boxes, 0-ulp mass and inertia), the spawn hash is not `0x03c5b4c9910d4d29`, `perturb` is not
  null, or the run reports itself void.
- **V7**: the final pose bytes differ from the arm's fixture (or the exe's own `expect_pose` is not
  `match`).
- **V8**: the CSV is not exactly the header `step,wall_ns,pool_threads` plus 500 rows (steps 0..499),
  each with `wall_ns > 0`. *Amended:* the header is exact.
- **V9**: count receipts fail.
  - Timed rows run **without** `--receipt` (checked: no `--receipt` in the recorded args). *Corrected
    reason:* the counting walk between steps changes the cache state the next step starts from. It is
    **not** that a deferred BVH task could finish off the clock: that task is spawned in
    `detect_collisions` and joined in `update_moved_collider_aabbs` in the same step.
  - The counts of a timed row are identified by V7: same binary and same final pose bytes means the
    pinned trajectory. That holds because Rapier's counters are write-only (nothing in the dynamics
    reads them); g2t compares the counters-off pose with the counters-on fixture at every W.
  - The prep gate re-runs `--receipt` twins for all **20** (cfg, arm, W) cells. Each must exit 0, match
    every pinned non-timing column at 500/500 steps, show 0 counter mismatches, keep 1240 bodies
    awake, and show `pool_threads == W` on every row. If a twin fails, that (cfg, arm) is void for the
    per-row claims only.

## Pre-registered claims

For each W and each metric window; [100,500) is primary.

- **The rule** (the same i/s/r rule as the Jolt rows): the cell is the median over K. A flag is set iff
  |B/A - 1| > 2 * hypot(A_x, B_x) for x in {i = IQR, s = 1.2533 * SD / sqrt(K), r = min-max}. CLAIMED iff
  i AND s flag pooled and in each pass, with one sign throughout. STRONG iff r also flags pooled and in
  every pass.
- **R-ARM(cfg, W):** rs8 vs rs4 by the rule. This **decides** which arm is Rapier's faster build at
  that cell. The faster arm is named only if the rule CLAIMS it; otherwise the arms are tied there and
  both are used below.
- **R-WALL-D(W):** ours `J-T` vs RP-D, compared per build. "Ours faster" is CLAIMED only if it is claimed
  against **every** build (rs8 and rs4). "Rapier faster" is CLAIMED if it is claimed against **any**
  build. Both builds are measured at every W, so no result carries a "rs8 only" label.
- **R-WALL-M(W):** the same against RP-M. It counts as an equal-work wall claim only if rows_M / rows_ours
  falls in [0.90, 1.10] (rows_M taken from the build compared). Otherwise it is reported with its row
  ratio.
- **R-ROW-D(W) and R-ROW-M(W):** the same test on wall / rows(window). rows_ours comes from the V2
  trunk's `J-T-a` census over the same window; Rapier's rows come from `rows_pin` of the build compared.
  Rows are fixed per (engine, cfg, build), so the relative spreads are unchanged and only the ratio is
  scaled.
- **Fidelity qualifier:** an "ours faster" claim is recorded as a WIN only if V2's fidelity gate is green
  (J-T gap 0.5, reuse on and off, 8/8). Otherwise it is recorded as UNEQUAL QUALITY, since Rapier holds
  9/9 in both configs, on both arms.
- **Pre-registered reading of the outcomes:**
  - Rapier faster on WALL-D while ours is cheaper on ROW-M: the loss is our iteration budget (12
    sweeps, friction in the biased sweep, against Rapier's 8 sweeps with normal-only biased passes).
    This goes to the architect as a budget experiment gated by the fidelity gate.
  - Rapier cheaper on ROW-M: a per-row implementation loss. This needs a stage map (a `profiler`
    armed Rapier arm, not built yet, plus our spans).
- **Conditions stated with every claim:** Rapier `codegen-units = 1` against ours fat LTO with
  `codegen-units = 16`; Rapier's counters are off in the timed rows (its `PhysicsPipeline::new()`
  default is on, and its field is documented "benchmarking only"); RP-D is defaults plus dt and CCD
  off, and RP-M is the research's closest match, with the differences listed in `README.md`.
- **Reported, not claimed:** T(1)/T(W) for each engine, Rapier vs Jolt.
- **Not in this window, and not registered:** a timed `--counters on` row, which would price the
  statistics walk directly. The exe supports it and the validator would flag it under V5.
