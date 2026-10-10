<!-- Saved from WF_audit.md by the orchestrated rework on 2026-09-30 (the author could not write files). Verbatim from the AUDIT line down, except: (1) the author's two-line note about not being able to save the file is dropped, (2) one sentence under V9 is corrected and marked [CORRECTED ...]. WF_audit.md is the untouched original. -->

AUDIT: The fairest equal-work unit is solver row-iterations per step. Rapier with default parameters does 0.60× our trunk's rows (0.56× Jolt's); Rapier "matched" does 1.80× ours, which is about 1.00–1.07× what our V2 trunk is predicted to do. So "matched" is the equal-work comparator and "defaults" is the product bar. Both Rapier configs hold the pile 9/9; our trunk holds 0/32. The harness needs a small REWORK before window 9a (a 4-lane build, pose red controls, one extra void, `--locked`).

# Results analysis: Rapier 0.36 on the J-T pyramid, equal work and fidelity against ours and Jolt 5.6

## VERDICT: REWORK (developer, harness only, small). The equal-work and fidelity findings are ACCEPTED.

**Summary.** The harness reproduces the J-T scene bit for bit. Its thread and determinism receipts hold, and I re-derived its counts and fidelity scores independently. The window cannot yet be claimed against "Rapier at its best", because only the 8-lane build exists and "maximum performance" was assumed, not measured. The `--expect-pose` path has also never been shown to fail. **No timing was read** (disclosure at the end).

## Checklist

| # | Criterion | Status |
|---|---|---|
| 1 | Release build: fat LTO, cgu 1, x86-64-v3, rustc 1.98.1 msvc | OK. Not rebuilt; sha256 pinned below |
| 2 | clippy -D warnings | OK. I re-ran `cargo clippy --locked --all-targets -- -D warnings` (clean) and `cargo fmt --check` (clean). `--locked` resolves, so the lock is consistent |
| 3 | Gates g1–g4 | OK. 42/42 exit 0. I recomputed the row formula from the g2 CSVs: 0/500 mismatches per config. The original F0 `metrics.py` (same sha `ca8673d8…`) reproduces the U poses and Jolt's 51.2 mm |
| 4 | Anti-vacuity | WARN. The lattice check goes red on the 1-ulp mutant. The `--expect-pose` exit-4 path and the pool receipt have **no red control** |
| 5/6 | Miri / Loom | N/A (no `unsafe`, no own synchronisation) |
| 7 | Rapier's fastest build identified | **FAIL**. Only `simd8` (untimed "max-perf" assumption) and `det` (`libm_force`) exist. There is no 4-lane build without the determinism feature |
| 8 | Build parity | WARN. Rapier builds with cgu 1 + fat LTO. Ours builds with `[profile.parity]` = release = fat LTO, cgu 16. Disclosed, not a blocker |
| 9 | unsafe | OK (none) |
| 10 | Brief implemented | OK. Deviations are acceptable: `simd-stable` no longer exists, density 1 (m = 8), CCD off in both configs |

## 1. Per-step work, means over [100,500)

Counts are the same at every W in all three engines: Rapier 500/500 steps equal at W 1–16, ours equal across W, Jolt hash equal.

| per step | ours trunk 16191fda | Jolt 5.6 | Rapier default | Rapier matched |
|---|---|---|---|---|
| candidate pairs | 9,545.1 (1,015 of them floor bounding-sphere pairs) | ≈8,533 (offline AABB estimate) | 8,544.8 | 8,552.0 |
| manifolds | 4,467.7 | 8,489 | 8,070.0 | 7,662.4 |
| points (pts/manifold) | 16,553.8 (3.71) | 31,112 (3.67) | 32,229.2 (3.99) | 29,753.0 (3.88) |
| floor / vertical / lateral manifolds | 225 / 4,050 / 198 (split of the final pose) | 225 / 4,060 / 4,204 | 225 / **4,060** / 3,785 | 225 / **4,060** / 3,377.4 |
| wholly speculative manifolds | 0 (overlap rule) | 3,655 (43 %) | 1,619 (20 %, approximate: `dist` goes stale under 5 cm recycling) | 2,819 (37 %) |
| substeps × sweeps | 4 × (1 biased + 2 relax) = 12 | 10 velocity + 2 position | 4 × (1 + 1) = 8 | 4 × (1 + 2) = 12 |
| friction in the biased sweep | yes | n/a | no | yes |
| friction rows | 2 per point | 2 + twist per manifold | 2 + twist per manifold | 2 per point |
| row formula | 36·Np | 10(Np+3Nm) + 2Np | 8Np + 12Nm | 36Np |
| velocity row-iterations | 595,935 | 565,790 | 354,674 | 1,071,107 |
| position row-iterations | 0 | 62,224 | 0 | 0 |
| body integrations per step | 4,960 | 1,240 | 4,960 | 4,960 |

- **[0,100) Rapier means** (needed for the secondary window):
  - default: Nm 6,891.2, Np 27,538.5, rows 303,003
  - matched: Nm 6,683.6, Np 25,992.5, rows 935,729
- The default config's counts are constant from step 291. The matched config's counts still change at step 499 (rows range 1,069,236–1,074,312, ±0.25 %).
- Jolt and both Rapier configs keep **all 4,060 vertical supports**. Ours keeps 4,050, because its pile deforms.

## 2. The row formula, checked against the rapier3d-0.36.0 source

Checked in the cargo registry copy:
- **Friction gating:** `solve.rs:98-100`.
- **Loop structure:** substeps at `worker.rs:269`, the biased loop at `:616`, the relax loop at `:713`.
- **Twist row:** skipped for 1-point manifolds (`contact_with_twist_friction.rs:754-765`). At Np/Nm = 3.99 that overcount is negligible.
- **Rapier's solver is the algorithm our V2 lane is shipping:**
  - `contact_with_coulomb_friction.rs:413`: `dist = info.dist + (p1−p2)·n` is recomputed every substep, and again in every relax pass (`:482`). This is Box2D v3's current-separation form, i.e. our K3.
  - `:420-434`: `rhs_wo_bias = max(dist,0)/dt`, the bias is clamped to [−max, 0], and cfm is 1 (rigid) when separated. This is V2's "s > 0 → no push" branch.
- **Softness is the same formula on both sides:** ours `ω/(2ζ+ωh)` (`soft_step.rs:875-903`), Rapier `erp_inv_dt`. Both run at substep dt.

## 3. The fairest equal-work unit and its ratios

**Unit: solver row-iterations per step** (velocity rows). Jolt's 62,224 position rows are reported beside it, not added in; per DOSSIER R6 they are light.

- **Why not manifolds or points:** they are set by the contact rule (speculative or not) and the friction model (per point or per manifold). For example, Rapier default has 1.81× our manifolds but does 0.60× our rows.
- **Where the unit is incomplete:**
  - It prices the solver only. Collision work is roughly equal anyway: candidate pairs are 0.89–0.90× ours for both Jolt and Rapier, and all engines mostly replay cached contacts.
  - It is a count, not a cost. Rapier also re-derives geometry each substep and each relax pass. Ours recomputes effective masses. Jolt sets up once.

| Rapier ÷ … | manifolds | points | candidate pairs | rows |
|---|---|---|---|---|
| default ÷ ours | 1.806 | 1.947 | 0.895 | **0.595** |
| matched ÷ ours | 1.715 | 1.797 | 0.896 | **1.797** |
| default ÷ Jolt | 0.951 | 1.036 | ≈1.00 | 0.627 velocity / **0.565** all rows |
| matched ÷ Jolt | 0.903 | 0.956 | ≈1.00 | 1.893 velocity / **1.706** all rows |

For reference, Jolt ÷ ours on rows is 0.949 (velocity) and 1.054 (all).

**Projected onto our V2 trunk (arithmetic only).** F0's S20 gives 1.676× our rows (998,787) and 7,617 manifolds; K3 adds about 7 % (≈1,068,700).
- Rapier matched ÷ ours V2 = **1.07–1.00**. Manifolds are 1.006×.
- Rapier default ÷ ours V2 = **0.355–0.332**.

## 4. Which Rapier config is the fair comparator

- **`matched` is the fair (equal-work) comparator.** It runs the same algorithm as V2 (20 mm margin, positive-separation branch, per-substep current separation), the same 36·Np row formula, the same softness, substeps and 4 m/s bias cap, and 1 mm contact reuse. On the V2 trunk its rows should be within 0–7 % of ours, so its wall ratio is close to a per-work ratio: it measures implementation efficiency.
  - What remains different: parry's contact generation, recycling by relative-pose drift, Rapier's angular cap, and SIMD packing.
- **`rapier-default` is the product bar.** It is what "beat Rapier" means, because it is what users run. It holds the pile with about one third of our V2 rows. A wall win against it is the owner's goal; the per-row reading explains any loss.
- **Against today's pre-V2 trunk, neither comparison is equal quality.** Our pile fails, so no "beat" claim is valid until V2 lands.

## 5. Fidelity (F0 metrics; W1; "holds" = 0 boxes > 0.5 m and max drift < 0.5 m)

| | holds | > 0.1 m | max drift (mm) | max rotation (°) | vertical overlap | residual max speed |
|---|---|---|---|---|---|---|
| Jolt 5.6 (1 run) | yes | 0 | 51.2 | 0.54 | 15.6 mm | 0 |
| Rapier default (U + 8 seeds; det build U also holds) | **9/9** | 76–89 | 210–285 | 0.40–1.20 | 0.39–0.40 mm | ≤ 0.00037 m/s |
| Rapier matched (U + 8 seeds; det build U also holds) | **9/9** | 5–13 | 149–293 | 0.88–1.80 | 0.39–0.41 mm | **0.018–0.062 m/s (not at rest)** |
| ours trunk 16191fda | 0/32 (F0) | 190 (U) | 2,183 (U) | 28.8 | 0.43 mm | — |
| ours V2 prediction (F0e K3+S20, spec-base harness, 8/8) | 8/8 | 2–6 | 106–153 | — | — | — |

- If the F0e prediction holds on the landed trunk, V2 would be tighter than both Rapier configs and looser than Jolt.
- Rapier holds the symmetric-brick pile that dimforge calls "marginally stable". The mechanism is the margin plus the current-separation update, which is exactly our V2 remedy.
- Rapier's trajectories are less chaotic: 4/8 (default) and 3/8 (matched) perturbed runs diverge by more than 0.1 m, against F0's chaos bar of 6/8.

## 6. Problems and where they go

- **P1 — Rapier's fastest build is unknown** (developer):
  - Add a third arm: `cargo build --release --locked --no-default-features` → `gate/bin/rapier-parity-simd4.exe` (`parallel`, 4-lane).
  - Re-run g1/g2 for it: pose equal at W 1–16, counts pinned.
  - Acceptance: sha pinned; fixtures RD4 and RM4 recorded.
- **P2 — no red control on the pose gate or the window validator** (developer or window prep):
  - `--steps 501` against a 500-step fixture must exit 4.
  - `--cfg matched` against the RD8 fixture must exit 4.
  - Validator mutations must go red: rs8/rs4 sha swap, pool receipt edited, features edited, config edited, `loop_on_pool_worker` false.
- **P3 — in `--install loop` mode, `loop_on_pool_worker == false` is recorded but not voided** (`main.rs:974-980`, `:993`). Make it exit 3.
- **P4 — `build.sh` has no `--locked`.** Add it, for provenance parity with our builds.
- **P5 (optional, later) — a `profiler` armed arm** for a Rapier stage map (structural, untimed). Not a 9a blocker.
- **P6 (optional) — a `--gap` flag**, so Rapier can join F0f at gap 1.0.

## 7. WINDOW 9a ROWS — Rapier

**Preconditions**
- V2 is on the trunk. Ours = the V2 trunk runner row `J-T` with its JT500 fixture, and its armed twin `J-T-a` in the same window supplies our rows census.
- P1–P4 are closed.
- The exes are copied to `$W/bin`.
- D: has at least 15 GB free.
- The idle-rule and witness exe lists include `rapier-parity-simd8.exe` and `rapier-parity-simd4.exe`. Any process under `D:/wt/_targets/rapier-parity` counts as a build.

**Binaries**

| key | exe | sha256 | features |
|---|---|---|---|
| rs8 | `bin/rapier-parity-simd8.exe` | `24808408be3a49617f538d87ac42a4c53fdc22dd0764b9757af84fb6f0e4a94b` | parallel + simd8 |
| rs4 | `bin/rapier-parity-simd4.exe` | pinned at prep | parallel only |

The `det` build (`f9f66f9f06109415a11d0b9bcfba83904c37ce1eea0f238ab33f7dd71ffb1e3f`) is not timed.

**Fixtures and pins**
- RD8 = `gate/g2/rapier-default_W1.pose`, hash `0x36e6142dc4db4701`.
- RM8 = `gate/g2/matched_W1.pose`, hash `0x29f2b3226e7e46c4`.
- RD4 and RM4 are recorded at prep.
- Count pins are every non-timing column of `gate/g2/{cfg}_W1.csv`.
- rows_pin for rs8:
  - RD: 303,003 over [0,100) and 354,674 over [100,500).
  - RM: 935,729 over [0,100) and 1,071,107 over [100,500).
- Config pins are the two SUMMARY `config` objects in `g2/*_W8.log`.
- Spawn hash `0x03c5b4c9910d4d29`.

**Rows** (kind `rapier`, block `PAR` shared with ours J-T; H-jolt56 may share it)

```json
{"id":"RP-D","blocks":["PAR"],"kind":"rapier","binaries":["rs8","rs4"],"args":["--cfg","rapier-default","--install","loop"],
 "workers":[1,2,4,8,16],"workers_by_binary":{"rs4":[1,8,16]},"steps":500,"window":[0,500],
 "metric_windows":[[0,100],[100,500]],"armed":false,"pose_ref":{"rs8":"RD8","rs4":"RD4"},"config_pin":"RD",
 "rows_pin":{"rs8":{"0..100":303003,"100..500":354674},"rs4":"<prep>"}}
{"id":"RP-M","blocks":["PAR"],"kind":"rapier","binaries":["rs8","rs4"],"args":["--cfg","matched","--install","loop"],
 "workers":[1,2,4,8,16],"workers_by_binary":{"rs4":[1,8,16]},"steps":500,"window":[0,500],
 "metric_windows":[[0,100],[100,500]],"armed":false,"pose_ref":{"rs8":"RM8","rs4":"RM4"},"config_pin":"RM",
 "rows_pin":{"rs8":{"0..100":935729,"100..500":1071107},"rs4":"<prep>"}}
```

**Exact timed commands** (run in the process directory; the driver appends `--label <row>_<bin>_W<W>_p<n>r<k>`):

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
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg rapier-default --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RD4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 1  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 8  --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
bin/rapier-parity-simd4.exe --cfg matched        --install loop --workers 16 --steps 500 --window 0..500 --csv run.csv --pose-out final.pose --expect-pose fixtures/RM4.pose
```

**Protocol**
- Ruling 1: K = 9 = 3 passes × 3 rounds, with p0 reversed, p1 forward, p2 reversed.
- The forward order within a W group is: ours J-T, RP-D rs8, RP-D rs4, RP-M rs8, RP-M rs4. This keeps ours and Rapier adjacent in every round.
- One untimed warm-up per pass (RP-D rs8 W8), the idle rule per pass, receipts at 5 % and witness at 2 %, a re-run once at the end of the pass, P-none.
- A pass-cell with K < 3 does not gate (8b ruling 8).
- The per-process value is the mean of `wall_ns` over the metric window from `run.csv`. The step-0-excluded [1,100) mean is reported as a sensitivity check and not claimed, because step 0 carries each engine's one-time costs.
- The placement receipt is recorded only. In loop mode the step runs on pool worker 0 while the main thread blocks, so main-thread share means nothing for Rapier.
- Size: 16 cells × 9 = 144 timed processes, plus warm-ups and re-runs. Minutes come from the prep dry run. If the window has spare time, rs4 extends to W2 and W4.

**Void rules** (any one voids the process; the protocol's re-run applies)
- V1: exit ≠ 0 (2 usage, 3 receipt gate, 4 pose mismatch), a crash, or a hang (> 300 s, terminated by its own handle).
- V2: not exactly one parsable `SUMMARY` line.
- V3: pool-size receipt ≠ W. Specifically, `pool_threads_min == pool_threads_max == W`, `install_receipt == [W,W]`, `loop_on_pool_worker == true`, and CSV `pool_threads == W` on all 500 rows must all hold.
- V4: build receipt differs. rapier3d must be 0.36.0, features must equal the key's features (`enhanced_determinism` false), avx2, fma and bmi2 must be true, and the exe sha256 must equal its pin.
- V5: the `config` object differs from the cfg's pin in any field.
- V6: scene identity is not all-true, the spawn hash is not `0x03c5b4c9910d4d29`, or `perturb` is not null.
- V7: pose ≠ fixture.
- V8: the CSV does not have exactly 500 rows (steps 0..499) each with `wall_ns > 0`.
- V9: count receipts fail.
  - Timed rows run **without** `--receipt`. The counting walk would open an untimed gap between steps and change the cache state the next step starts from. [CORRECTED 2026-09-30 from WF_review.md, "Open questions": the original sentence said the counting walk lets Rapier's deferred BVH task, "spawned during the solve, joined in the next step", finish off the clock. That is wrong: the task is spawned in `detect_collisions` (`solve.rs:91-98`) and joined in `update_moved_collider_aabbs` (`substep.rs:246`, called at `:707`) in the SAME step, so no Rapier work crosses a step boundary. The rule stands for the cache-state reason. The verbatim original is `WF_audit.md`.]
  - Their counts are identified by V7: same binary and same final pose bytes means the pinned trajectory.
  - The prep gate re-runs `--receipt` twins for all 16 (cfg, build, W) cells. Each must exit 0, match every pinned non-timing CSV column at 500/500 steps, show 0 counter mismatches, and keep 1240 bodies awake.
  - If a twin fails, that (cfg, build) is void for the per-row claims only.

**Pre-registered claims** (for each W and each metric window; [100,500) is primary, [0,100) is claimed under the same rule and reported separately)
- **The rule** (the same i/s/r rule as the Jolt rows): the cell is the median over K. A flag is set iff |B/A − 1| > 2·hypot(A_x, B_x) for x ∈ {i = IQR, s = 1.2533·SD/√K, r = min–max}. CLAIMED iff i AND s flag pooled and in each pass, with one sign throughout. STRONG iff r also flags pooled and in every pass.
- **R-WALL-D(W):** ours J-T vs RP-D, compared per build.
  - "Ours faster" is CLAIMED only if it is claimed against **every** build measured at that W. At W2 and W4 it is labelled "rs8 only".
  - "Rapier faster" is CLAIMED if it is claimed against **any** build.
- **R-WALL-M(W):** the same against RP-M. It counts as an equal-work wall claim only if rows_M ÷ rows_ours falls in [0.90, 1.10]. Otherwise it is reported with its row ratio.
- **R-ROW-D(W) and R-ROW-M(W):** the same test on wall ÷ rows(window).
  - rows_ours comes from the V2 trunk's J-T-a census over the same window.
  - Rapier's rows come from rows_pin.
  - Rows are fixed per (engine, cfg, build), so the relative spreads are unchanged and only the ratio is scaled.
- **Fidelity qualifier:** an "ours faster" claim is recorded as a WIN only if V2's fidelity gate is green (J-T gap 0.5, reuse on and off, 8/8). Otherwise it is recorded as UNEQUAL QUALITY, since Rapier holds 9/9 in both configs.
- **Pre-registered reading of the outcomes:**
  - Rapier faster on WALL-D while ours is cheaper on ROW-M: the loss is our iteration budget (12 sweeps, friction in the biased sweep, against Rapier's 8 sweeps with normal-only biased passes). This goes to the architect as a budget experiment gated by the fidelity gate.
  - Rapier cheaper on ROW-M: a per-row implementation loss. This needs a stage map (P5 plus our spans).
- **Reported, not claimed:** rs8 vs rs4 per W, T(1)/T(W) for each engine, Rapier vs Jolt.

## Disclosure

- **Timing seen, not used:**
  - The first line of `g2/rapier-default_W1.csv`, printed during a structural check, shows one step-0 `wall_ns` value.
  - `g2cpu/cpu_receipt.txt` shows process walls.
  - I used neither and quote neither.
- **Processes and disk:** no process is left running (none was started except clippy and python). D: has 91 GB free.

**Files**
- `D:/tmp/rapier-parity/src/main.rs`
- `D:/tmp/rapier-parity/build.sh`
- `D:/tmp/rapier-parity/gate/gates.txt`
- `D:/tmp/rapier-parity/gate/g2/`
- `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/jolt-gap/DOSSIER.md`
- `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/jolt-gap/f0/VERDICT.md`
- `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/phys-next/RULINGS-2026-09-30-V2.md`
- `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b/analysis.md` (lines 39–47: the i/s/r rule)