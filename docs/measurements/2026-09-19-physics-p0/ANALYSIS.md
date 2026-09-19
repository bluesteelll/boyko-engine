# P0b reduction: MEASUREMENT-QUEUE §10, recomputed from `p0b/raw/`

## 0. What was recomputed, and corrections to the window report

**Inputs.** `raw/window/runs.jsonl` holds 262 records. I used 258 of them (43 cells × K=6). The 2 warm-ups and the 2 contaminated originals were dropped; each of those originals has a re-run, which is the value used. All 258 exited with 0.

**Checks on the inputs:**
- Every window mean I recomputed from its CSV equals the runner's `window_mean_ns` exactly.
- Every Jolt per-frame mean equals 1/(printed steps/s) to within 1e-4.
- Receipts: 266 distinct. Median 1.26 % busy, max 14.0 %. Two were above 5 %, and no build process was seen in any.
- Structural checks all hold:
  - 0 void steps, drops 0, disarmed ring traffic 0.
  - H7: 12 of 12.
  - 7 boyko pose groups, each with one pose. Jolt's hashes are constant per (row, W).
  - Waves: one per-step sequence across J-A-a, J-B, J-C, J-P1 and J-S0 at W=1 and W=8, equal to 12 × wide colours on every step.
  - Closure: u<0 and g<0 never occur; u reaches at most 0.48 % of the solve span.
- The diagnostic's cell table reproduces from `raw/runs.jsonl`.

My numbers match the tester's tables wherever they overlap. I did not use the tester's figures directly.

**Corrections to `window_report.md`:**
1. **"profile `dev`" is the boyko_diag zone tier, not the cargo profile.**
   - Every runner row is cargo `parity` (inherits `release`: fat LTO, default codegen units, `x86-64-v3` baseline).
   - The broadphase bench is cargo `bench` (`lto = false`, `codegen-units = 1`, `Cargo.toml:114-128`).
2. **The parity headline (cfg-A) is not the tip's default configuration.**
   - At `dbd85977`, `DefaultRigidSolver = ColoredSoftStepSolver` and `simd_solve: true` (`56c1e9e7`; `resources.rs:~474`).
   - cfg-A pins `simd_solve = false` and sets `parallel_solve = (W>1)`. The default has `parallel_solve: false`.
   - R is the default configuration; J-A is not.
3. **Armed J-A ran only at W=1 and W=8** (`window_run.py:67-74`), whereas §10 specifies 1, 2, 4, 8 and 16. So I(W), E(W) and L(W) do not exist at W = 2, 4 and 16. This deviation is not in the report's list.
4. **H9 for v5.6.0 uses v5.3.0's manifold count.** v5.6.0 has no `-receipt` run, and its hash differs.
5. **Jolt `-p` shares are biased toward FindCollisions.** That stage holds 34.2k of the 48.3k profiler scopes per frame (71 %), and profiling inflates T by 24 % (W=1) and 15 % (W=8). §3 therefore brackets Jolt's stage times instead of scaling them by share.
6. **The plan's ρ = 0.376 is not out of date; it is the R scene's value.** R: 6,662 manifolds and 22,958 points give ρ = 0.3765. J gives ρ = 0.399.

## 1. T(W): J-A disarmed against Jolt, median over K=6 [min–max] in ms

| W | boyko cfg-A | Jolt v5.3.0 | Jolt v5.6.0 | b/J5.3 | b/J5.6 | per-manifold b/J5.3 [100,500) | per-manifold b/J5.6* |
|---|---|---|---|---|---|---|---|
| 1 | 19.671 [19.079–19.835] | 15.723 [14.987–16.088] | 9.650 [9.545–10.042] | 1.251 | 2.038 | 2.32 | 3.89 |
| 2 | 13.711 [13.445–13.819] | 8.794 [8.514–9.005] | 5.708 [5.666–5.785] | 1.559 | 2.402 | 2.88 | 4.52 |
| 4 | 10.701 [10.653–10.869] | 5.229 [5.139–5.374] | 3.557 [3.540–3.639] | 2.047 | 3.008 | 3.79 | 5.61 |
| 8 | 9.162 [9.103–9.250] | 3.533 [3.452–3.583] | 2.493 [2.467–2.589] | 2.593 | 3.675 | 4.73 | 6.81 |
| 16 | 9.172 [9.136–9.210] | 3.209 [3.189–3.277] | 2.447 [2.371–2.480] | 2.858 | 3.749 | 5.20 | 6.90 |

\* v5.6.0 is normalised with v5.3.0's manifold count.

- **All ratios are claimed under range, IQR and SE.** The thinnest margin is W=1 against v5.3.0: +25.1 % against a range bar of 16.0 %.
- **H8/H9 manifold counts, each side's own:**
  - boyko has 4,519.3 manifolds per step over [100,500) and 4,524.2 over [0,500). The count is identical in every process at every W.
  - Jolt v5.3.0 has 8,456.0 and 8,171.0 (P0's `-receipt`).
  - **Independently confirmed in P0b:** Jolt's own profiler counts 8,456 "Add Constraint From Cached Manifold" calls per frame at frames 200–400, against 8,541 `ProcessBodyPair` calls and 0 new pairs.
  - The raw ratio is ×1.871 away from the per-manifold one. Jolt's extra ~4.2k manifolds are speculative lateral contacts, so the truth lies between the two. At W=1 that is 1.24–2.32 over [100,500).
- **Per manifold per step over [100,500):**
  - boyko: 4.369, 3.042, 2.378, 2.034 and 2.034 µs (W = 1, 2, 4, 8, 16).
  - Jolt v5.3.0: 1.883, 1.056, 0.628, 0.430 and 0.391 µs.
- **Sub-windows (H1), b/J5.3 over [0,100) / [100,500):** 1.295/1.240, 1.638/1.539, 2.158/2.025, 2.903/2.527, 3.190/2.780.
- **Scaling, T(1)/T(W) for W = 2, 4, 8, 16:**
  - boyko: 1.435, 1.838, 2.147, 2.145.
  - v5.3.0: 1.788, 3.007, 4.450, 4.899.
  - v5.6.0: 1.691, 2.713, 3.870, 3.944.
  - boyko W16 against W8 is +0.11 % (not claimed): boyko does not scale beyond 8 workers.

## 2. Armed profile (J-A-a), median over K=6

| span (ms) | W=1 | W=8 |
|---|---|---|
| T (armed) | 19.691 | 9.230 |
| gather / build_graph / apply | 0.027 / 0.140 / 0.018 | 0.025 / 0.117 / 0.013 |
| broadphase (AllPairs) | 2.100 | 1.945 |
| narrowphase | 3.142 | 2.946 |
| solve_colored | 14.179 | 4.145 |
| … solve_build / warm_apply / store | 0.970 / 0.613 / 0.266 | 0.920 / 0.621 / 0.208 |
| … integrate / gravity / restitution | 0.065 / 0.008 / 0.008 | 0.078 / 0.013 / 0.008 |
| … wide colours / narrow colours | 12.205 / 0.038 | 2.241 / 0.041 |
| g (executor gap) | 0.089 | 0.035 |

Terms computed as the plan specifies, from span medians:

| term | value |
|---|---|
| S(1) | 7.398 ms; per process 7.369–7.418 |
| f = S(1)/T(1) | **0.376** (per process 0.3747–0.3760) |
| f_fit | 0.393 (armed T), 0.390 (disarmed T) |
| f_fit − f | +0.014 to +0.017 |
| S(8) | 6.939 ms, **75 % of T(8)** |
| **I(8)** | **−0.459 ms** |
| P(1) | 12.205 ms; range 0.13 % |
| L(8) | 0.715 ms; every process pairing gives 0.706–0.722 |
| E(8) | 0.681 |
| waves | 109.32 per step |
| ω(8) = L/waves | 6.54 µs |
| ω₁ (J-P1) | 0.855 µs; every pairing gives 0.57–1.84 |
| g(1), g(8) | 89.0 µs, 34.6 µs |
| u(1), u(8) | 1.41 µs, 1.37 µs |
| r(1), r(8) | 4.32 µs, 4.73 µs |
| identity residual | −6.6 µs (W=1), +9.0 µs (W=8) |

- **The plan's §0 prior of f ≈ 0.48–0.53 (09-10, pre-A7, gnu) is superseded.**
- **I(8) is negative, not the non-negative "interference" the plan assumed.** Serial stages run faster at W=8, and two of the drops are claimed under all three readings:
  - broadphase −7.4 %;
  - narrowphase −6.3 %.
- **A witness (a correlation, not a cause): how long the busiest CPU was occupied.**
  - At W=1 the busiest CPU is busy 50–68 % of the process lifetime; at W=8 it is 84–92 %.
  - All 5 of the fast W=1 processes (19.08–19.43 ms, across the tip, pre-L1 and shipping binaries) had 82–92 % occupancy. The median over the 48 W=1 J-family processes is 61 %.
  - The correlation between occupancy and deviation from the cell median is r = −0.67 (n = 48). The 1-Hz clock witness is weaker.
  - This fits the thread moving between CPUs (a guess, not measured). A cheap test would be W=1 with a one-CPU process mask; the diagnostic did not test W=1.
- **ω₁ is claimed only under SE.** J-P1's wide colours minus P(1) is 93 µs per step. The range bar is 247 µs; the SE bar is 47 µs.

## 3. W=8 gap attribution (boyko 9.162 against Jolt 3.533; gap 5.63 ms)

Jolt's timed stage times are bracketed two ways: (a) wall-coverage share × timed T; (b) the profiling overhead (Update span − timed T) removed pro rata to each stage's profiler scope count. My own parser of the HTML dumps gives the same shares as the tester's: W=8 is 66.19 / 25.48 / 6.93 %; W=1 is 63.54 / 29.45 / 6.29 %.

| stage | boyko ms | Jolt ms | Δ ms (share of gap) |
|---|---|---|---|
| collision: boyko bp + np (serial) vs FindCollisions (parallel, cache replay) | 4.891 | 0.67–0.90 | **3.99–4.22 (71–75 %)** |
| solve: boyko solve span vs Jolt SolveVelocity + SolvePosition + gravity, integrate, finalize | 4.145 | 2.62–2.84 | 1.31–1.53 (23–27 %) |
| … boyko serial in-solve (build 0.920, warm 0.621, store 0.208, integrate 0.078, narrow 0.041) | 1.89 | — | — |
| … boyko parallel wide colours = P(1)/8 (1.526) + L (0.715) | 2.241 | — | — |
| other (gather, graph, apply, g) | 0.19 | ~0.07 | ~0.12 |

- **Δ_J = T_J(`-no_pair_cache`) − T_J = +1.552 ms (+43.9 %).** It is claimed (bars 12.8 / 3.2 / 2.3 %).
- **At W=1 the collision stages are close; the gap at W=8 comes from parallelism, not per-pair cost.**
  - boyko bp + np = 5.24 ms, against Jolt FindCollisions 3.0–4.6 ms.
  - boyko solve per manifold at W=1: 3.13 µs with `simd_solve` off, 1.24 µs with it on (J-B). Jolt: 1.30–1.49 µs.

## 4. J-B (Grid + simd_solve) against J-A-a, both armed

| W | T | broadphase | solve | wide colours |
|---|---|---|---|---|
| 1 | −5.48 ms (−27.9 %) | +3.07 ms (+146 %) | −8.56 ms (−60 %) | 3.41× faster |
| 8 | **+1.93 ms (+21.0 %): SLOWER** | +3.09 ms (+159 %) | −1.16 ms (−28 %) | 2.34× faster |

- All of the above are claimed under all three readings.
- `simd_solve` raises `solve_build` by +68 µs (W=1) and +101 µs (W=8). This is claimed under SE only (and IQR at W=8).
- **Derived, not a measured row: cfg-A + `simd_solve`.** This is the tip's default solver with parallel_solve = W>1.
  - Estimated T: 11.04–11.14 ms at W=1 and 7.92–8.08 ms at W=8.
  - Against v5.3.0: **0.70–0.71 at W=1** and 2.24–2.29 at W=8.
  - Against v5.6.0: 1.14–1.15 and 3.18–3.24.
- With simd: P(1) = 3.576 ms, L(8) = 0.510 ms, E(8) = 0.467.

## 5. R against R-ref (D1, shipped in `56c1e9e7`)

- **R (default configuration):** 14.226 [13.542–14.436] ms. **R-ref (SoftStepSolver):** 23.929 [23.811–24.071] ms.
- **R-ref/R = 1.682 (+68.2 %)**, claimed (bars 12.8 / 3.7 / 2.3 %). D1 saves 9.70 ms per step at W=1 (−40.5 %).
- The difference is all in the solve: 8.173 against 18.138 ms. Collision is equal (bp 2.08/2.11, np 3.63/3.55 ms).
- The contact sets differ (6,662 against 5,949 manifolds), so values changed as D1 predicts.
- R at W=8 (parallel): 9.933 ms, with S(8) = 8.49 ms, **85 % serial**.

## 6. Sleeping floors

- **R-S (F):** 6.049 ms at W=1 and 6.123 ms at W=8 (+1.2 %, not claimed). The first frozen step is 248 in 12 of 12 processes, so the row is not void.
  - F = bp 1.93 + np 3.42 + graph 0.18 + solve 0.45 (of which store is 0.32) ms. Collision plus graph is 92 % of F.
- **O5 tails [264, 1000):** every process is all asleep from boyko step 264 and Jolt frame 127 (12 of 12 on each side).
  - boyko J-Son: 5.334 ms (W=1) and 5.407 ms (W=8).
  - Jolt `-allow_sleep`: 2.17 µs and 2.29 µs.
  - The ratio is **≈2,450× and ≈2,370×**. boyko's tail is bp 1.93 + np 2.92 + graph 0.12 + solve 0.30 ms.

## 7. Gates

| comparison | effect | bars: range / IQR / SE (%) | claimed | verdict |
|---|---|---|---|---|
| A/A1 arming, W=1 | +0.11 % | 7.71 / 1.03 / 1.41 | n/n/n | pass; P0's "0.3–1.3 %" not reproduced |
| A/A1 arming, W=8 | +0.74 % | 4.14 / 1.91 / 0.79 | n/n/n | pass |
| A/A2, W=1 | +0.51 % | 7.94 / 1.39 / 1.46 | n/n/n | pass: zones stay in `dev` |
| A/A2, W=8 | +0.30 % | 6.26 / 2.15 / 1.15 | n/n/n | pass |
| SHIP against dev | −0.55 % / −0.53 % | 10.21 / 1.17 / 1.83 and 7.28 / 0.73 / 1.37 | n/n/n | — |
| canary J-C against J-A-a, W=1 | +5.06 % (injected 5.00 %) | 2.08 / 1.02 / 0.41 | Y/Y/Y | seen |
| canary J-C against J-A-a, W=8 | +4.81 % (injected 4.98 %) | 4.17 / 2.79 / 0.91 | Y/Y/Y | seen |
| canary against J-A-d1, W=1 | +5.18 % | 7.94 / 1.41 / 1.46 | n/Y/Y | **not visible under min–max** |
| L1 gate: L1-B (tip) against L1-A1 (pre-L1), J-S0 disarmed | +0.68 % | 8.91 / 2.83 / 1.67 | n/n/n | no regression; see below |
| J-S0 against J-A-a (threshold-0 bookkeeping) | +0.08 % | 1.39 / 0.72 / 0.29 | n/n/n | — |

- **Canary span:** it reads the spin to within +0.034 % (W=1) and +0.051 % (W=8).
- **L1's gate cannot fail below 8.9 % under min–max.**
  - L1's target is +2.44 % (the §8 figure). It is **not seen**: the tip reads 0.68 % slower.
  - Under SE, a gain of 1 % or more from L1 is excluded on this row.
  - On the tip, the threshold-0 bookkeeping is ≤ ~0.4 % (SE). On pre-L1, L1-A1 against J-A-d1 reads −0.58 % (SE bar 1.59 %).

**The readings disagree in only a few places:** the canary against d1, S16 W8/W1 (claimed under SE only), Jolt v5.3.0 W16/W8 (−9.2 %, claimed under IQR and SE), and some J-B stage terms.

## 8. Broadphase crossover (`bench` profile, K=1)

- **Uniform lattice, Grid/AllPairs:** 8.13 at n=100, 1.107 at 1k, 0.114 at 10k. Log-log interpolation gives **n\* ≈ 1,109**.
- **Size disparity:** 3.908 at 1k, 0.220 at 10k, giving **n\* ≈ 2,978**.
- **In the scene (1240 bodies plus a ground slab):** Grid/AllPairs is 2.46 at W=1 and 2.59 at W=8. The scene behaves like the size-disparity case.
- **`GRID_HI` = 192 and `GRID_LO` = 96 sit 6–31× below the crossovers** (`broadphase_policy.rs:79,88`). Auto would switch this scene to Grid, at +3.07 ms per step.
- **O3 Gate 7 fails** (not ruled on: one process). The parallel Grid emit at 100k, w4 against w1, is 1.92×; the gate is ≥ 2.8×.

## 9. Levers L2–L10

The build-if rule is plan §3 as amended: gain ≥ 5 % of T(W) and > 2 × the combined spread. For a before/after pair at K=6 that spread is 2√2 × the cell spread. Every lever is also gated end to end on T(W) (W3).

**Smallest claimable change at K=6 (range / IQR / SE, %):**

| row | W=1 | W=2 | W=4 | W=8 | W=16 |
|---|---|---|---|---|---|
| J | 10.9 / 1.4 / 2.0 | 7.7 / 2.5 / 1.4 | 5.7 / 3.2 / 1.2 | 4.5 / 0.5 / 0.75 | 2.3 / 1.5 / 0.5 |
| R | 17.8 / 5.2 / 3.3 | — | — | 7.0 / 2.7 / 1.3 | — |

Under SE, with the pooled boyko SD of 0.93–1.00 %, resolving 1 % takes K ≥ 12 and resolving 0.5 % takes K ≥ 47.

| rank | lever | predicted gain | build-if | verdict |
|---|---|---|---|---|
| 1 | **L4** `parallel_solve` default | W=8: P(1) − t_wide(8) = 9.96 ms (cfg-A), 2.62 ms (with simd); R: 5.27 → 1.40 ms. W=1 cost: J-P1 +0.60 % (SE bar 0.84; ω₁·waves predicts +0.47 %) | t_wide(8) < t_wide(1) on J and R ✓; S16 −2.5 % ✓; W=2/4/16 only end-to-end (T(W) < T(1), claimed) | **build**; trivial cost |
| 2 | **L5** parallel narrowphase | t_np(1)(1 − 1/(8E)) = 2.57 ms (28.0 % of T(8)); 2.37 ms against today's t_np(8) because I(8) < 0 | clears every bar at W=8; E(W) unknown at 2/4/16 | **build**; medium cost |
| — | *not a plan lever:* parallel AllPairs broadphase | ≤ 1.71 ms (18.7 %) by L5's formula; serial bp is 21 % of T(8) | — | architect's call |
| 3 | **L10** sleeping default (D3, owner-approved) | parity exactly 0; R: −8.18 ms (−57.5 %) at W=1, −3.81 ms (−38.4 %) at W=8, claimed | B1 fix `aff98fe7` is an ancestor ✓; D1 shipped ✓ | clears; F is 92 % collision+graph, so the frozen-pair skip bounds a further ≤ 5.5 ms |
| 4 | **L8** friction per patch (D2, owner-approved) | ρ = 0.399 (J), 0.376 (R); ≤ ρ·(t_colors + t_warm) = 1.16 ms (12.5 %) cfg-A, 0.64 ms with simd | t_colors(8) + t_warm(8) = 31.5 % of T(8), ≈ 44 % after L5 (≈ 29 % with simd) ✓ | clears, conditional on L5; high cost |
| 5 | **L6** colour dispatch | ≤ 0.66 ms (7.1 %) with ω_b = ω₁ | L(8) + t_narrow(8) = **8.2 % < 10 %** ✗ (6.6 % with simd) | **not built**; see note below |
| 6 | **L9** contact reuse (D4, owner-approved) | ≤ h·t_np: 2.95 ms today, 0.58 ms after L5 | Δ_J 43.9 % ✓; t_np(8) is 31.9 % today but **8.7 % after L5** ✗ | not built if L5 lands |
| 7 | **L7** warm apply per colour | 0.26 ms (2.8 %) with ω(8); 0.47 ms (5.1 %) with ω₁ | t_warm(8) = 6.7 % ✓, but gated after L6 | blocked; see note below |
| — | **L2** Grid/Auto default | **−3.07 ms (W=1), −3.09 ms (W=8)** | span rises; the size-disparity crossover (2,978) is above 1240 | **do not flip.** Recalibrate `GRID_LO`/`GRID_HI` (0 effect on parity) |
| — | **L3** `simd_solve` default | shipped: colours 3.41× (W=1) and 2.34× (W=8); estimated T −43 % (W=1), −13 % (W=8) | — | superseded; the headline must say "simd off" |

- **L6 fork (W2):** ω₁·waves/L(8) = 0.13, and at most 0.28 over every process pairing. That is < 0.5, which selects "retune the constants".
  - But ω₁ is measured without a cross-thread wake. ω(8) is 7.7× ω₁, and the split between wake and imbalance is not measured (O1).
- **L7 formula is ambiguous.** Read literally, "4·waves·ω" is a 2.86 ms loss. I read it as 4 substeps × wide colours × ω.
- **Undecidable at this resolution:**
  - L7 at W=8 under min–max (2.8 % against 4.5 %);
  - every W3 end-to-end gate at W=1 under min–max (10.9 % on J, 17.8 % on R);
  - arming and A/A2 below ~1.5 % (SE upper bounds 1.52 % and 1.53 %);
  - `parallel_solve` at W=1;
  - L1's gain;
  - boyko W16 against W8;
  - R-S at W8 against W1.

**Projection (arithmetic, not measurement), W=8, E = 0.68 carried over:**
- cfg-A 9.23 ms → with simd 7.9–8.1 ms → plus L5 5.5–5.7 ms. That is 1.57–1.61× v5.3.0 and 2.2–2.3× v5.6.0.
- Adding a parallel AllPairs broadphase gives ~4.0 ms, about 1.15× v5.3.0.

## 10. Draft RESULT block for §10

Heading: `## 10. … (perf campaign P0) — TIMED 2026-09-19 (window 1 VOID; window 2 under the ruled protocol)`. Replace "NO TIMING HAS BEEN TAKEN" with a pointer to this block.

```markdown
**RESULT, 2026-09-19.** Two windows. Window 1 VOID (below); window 2 ("P0b") timed under the orchestrator's
ruled protocol and complete. Owner's workstation: Ryzen 9 5900HS, 8C/16T (core k = CPUs {2k,2k+1}, 512 KiB L2
per core, 16 MiB L3), High performance, AC; Task Manager, Steam and a browser open. `D:/wt/joltab` at `dbd85977`,
clean; rustc 1.98.1 `host: x86_64-pc-windows-msvc`, no RUSTFLAGS, `x86-64-v3` baseline. Ancestors true: S5
`8d656ad8`, KE16 `67563d3b`, B1 `aff98fe7`, L1 `00c07d0f`; D1 + `simd_solve` default = `56c1e9e7`.
boyko runner: cargo `parity` (inherits `release`: fat LTO, default CGU), zone tier `dev` (SHIP: `shipping`);
sha256 tip `ef9325ef`, pre-instrument `b7c42334`, pre-L1 `a0da832f`, shipping `fdcac07f`. broadphase bench:
cargo `bench` (lto=false, CGU=1), `3c5cd8b4`. Jolt v5.3.0 Distribution `29b23ad1` / Release (`-p`) `de00c882`,
v5.6.0 Distribution `918fd2b7`; WinLibs GCC 16.1.0, IPO on, AVX2; source = the repository's `pyramid_scene.patch`.

**Window 1 — VOID (05:22:15–05:46:53 +03:00, passes 0–2 of 5).** A/A0 fired at W=2: B(2) = 2.311 % > 2 %
(d2/d1 = 0.98168, 0.98093, 0.97689). Not load: 307 of 308 receipts ≤ 4.9 % busy. Cause: each process of the
SAME binary draws its own speed for its whole run (W=2: 6 identical processes span 5.2 %; replaying the group
flips the sign of d2/d1); the band gated a median of paired step ratios while the quote is a window mean. No
timed number from window 1 is used. **Protocol ruling:** a cell's statistic is the median over K separate
processes of the window mean; resolution = the process spread (min–max, IQR; the median's SE
1.2533·SD/√K supplementary); a comparison is claimed only if |effect| > 2·hypot(spread_A, spread_B);
receipts gate each process (> 5 % ⇒ re-run once); no band-based void; the canary must be seen. **Placement
diagnostic** (06:13–06:36): pinning one thread per physical core did not narrow the spread in any
(engine, W∈{2,8}) cell (largest variance ratio 3.3 against F(5,5)₀.₀₅ = 7.15) and moved no median beyond
resolution ⇒ P-none (no affinity) for both engines.

**Window 2 (06:49:05–07:51:05).** K = 6 in all 43 cells (2 passes × 3 rounds, interleaved; pass 1 reversed;
one untimed warm-up per pass); 258 processes, 0 non-zero exits; 2 contaminated (after-receipts 14.0 % and
7.17 %) and re-run once. Receipts: 266, median 1.26 % busy; idle polls 9/9 quiet. Structural checks all
green: 0 void steps, drops 0, disarmed ring traffic 0; H7 12/12; 7 boyko pose groups with one pose each
(J family, sleep-off = sleep threshold 0 = pre-L1 = `0x32d5e235342b4143`); Jolt one hash per (row, W); waves
109.32 per step, identical at W=1 and 8, = 12 × wide colours on every step; closure: u ≤ 0.48 % of solve,
u, g ≥ 0; R-S first frozen step 248 (12/12); Jolt threads = W.

| W | boyko J-A (cfg-A, disarmed) ms | Jolt v5.3.0 | Jolt v5.6.0 | boyko/v5.3.0 | boyko/v5.6.0 |
|---|---|---|---|---|---|
| 1 | 19.671 [19.079–19.835] | 15.723 [14.987–16.088] | 9.650 [9.545–10.042] | 1.251 | 2.038 |
| 2 | 13.711 [13.445–13.819] | 8.794 [8.514–9.005] | 5.708 [5.666–5.785] | 1.559 | 2.402 |
| 4 | 10.701 [10.653–10.869] | 5.229 [5.139–5.374] | 3.557 [3.540–3.639] | 2.047 | 3.008 |
| 8 | 9.162 [9.103–9.250] | 3.533 [3.452–3.583] | 2.493 [2.467–2.589] | 2.593 | 3.675 |
| 16 | 9.172 [9.136–9.210] | 3.209 [3.189–3.277] | 2.447 [2.371–2.480] | 2.858 | 3.749 |

- Every ratio is claimed under range, IQR and SE. **cfg-A is not the tip's default**: it pins `simd_solve`
  off (default on since `56c1e9e7`) and sets `parallel_solve = W>1` (default off). Derived from J-B's spans,
  NOT a measured row: cfg-A + `simd_solve` ≈ 11.0–11.1 ms at W=1 (0.70× v5.3.0) and 7.9–8.1 ms at W=8
  (2.24–2.29× v5.3.0). Queue a measured row before quoting this.
- H8 fired, and it is the contact set: over [100,500) Jolt has 8,456 manifolds (receipt; confirmed by Jolt's
  own profiler, 8,456 cached-manifold adds per frame), boyko 4,519.3 (1.871×). Per manifold per step,
  boyko/v5.3.0 = 2.32, 2.88, 3.79, 4.73, 5.20 at W = 1–16; the truth lies between that and the raw ratio.
  v5.6.0's own count is not receipted.
- H1, boyko/v5.3.0 over [0,100) / [100,500): 1.295/1.240 at W=1, 2.903/2.527 at W=8.
- Scaling T(1)/T(8): boyko 2.147 (T(16) = T(8), +0.11 %, not claimed), v5.3.0 4.450, v5.6.0 3.870.

**Profile (armed J-A, W=1 / W=8).** S(1) = 7.398 ms; **f = S(1)/T(1) = 0.376**, f_fit = 0.393 (0.390 from
disarmed T), so f_fit − f = +0.017. P(1) = 12.205 ms. S(8) = 6.939 ms (75 % of T(8)); **I(8) = −0.459 ms**
(broadphase −7.4 %, narrowphase −6.3 %, both claimed). L(8) = 0.715 ms; E(8) = 0.681; ω(8) = 6.54 µs;
ω₁ = 0.855 µs (claimed only under SE). g = 89.0 / 34.6 µs; u = 1.41 / 1.37 µs; r = 4.32 / 4.73 µs;
identity residual −6.6 / +9.0 µs. The L6 fork ω₁·waves/L(8) = 0.13 (≤ 0.28 over every process pairing).
Armed J-A was run at W = 1 and 8 only, so I, E and L at W = 2, 4, 16 are not measured.

**W=8 gap (5.63 ms) beside Jolt `-p` (Release; profiled T is +24 % at W=1 and +15 % at W=8 over timed).**
- Collision: boyko bp + np is 4.891 ms and serial, against Jolt FindCollisions 0.67–0.90 ms (parallel, a
  cache replay of 8,456 of 8,541 pairs). That is 71–75 % of the gap.
- Solve: 4.145 ms against 2.62–2.84 ms, 23–27 % of the gap. boyko's serial in-solve work is 1.89 ms.
- Jolt's wall-coverage shares: SolveVelocity 66.19 %, FindCollisions 25.48 %, SolvePosition 6.93 %.
- Δ_J = +1.552 ms (+43.9 %, claimed).

**Rows.**
- J-B against J-A-a: −27.9 % at W=1, **+21.0 % (slower) at W=8**. Grid adds +3.07 and +3.09 ms;
  `simd_solve` makes the colour spans 3.41× and 2.34× faster.
- D1: R-ref/R = 1.682 (+68.2 %): the colored default saves 9.70 ms per step at W=1 on the gap-0 pile.
- Sleeping floor F (R-S): 6.049 ms (W=1) and 6.123 ms (W=8); 92 % of it is collision + graph.
  L10 on R: −57.5 % (W=1), −38.4 % (W=8).
- O5 tails [264, 1000): boyko 5.334 / 5.407 ms against Jolt 2.17 / 2.29 µs (≈2,400×).
- S16: 81.4 / 79.4 µs.

**Gates.**
- A/A1 (arming): +0.11 % at W=1, +0.74 % at W=8. Not claimed: pass.
- A/A2: +0.51 %, +0.30 %. Not claimed: pass, and the zones stay in `dev`.
- SHIP against dev: −0.55 %, −0.53 %. Not claimed.
- Canary (J-C against J-A-a): +5.06 % and +4.81 % against 5.00 % and 4.98 % injected. Seen under every
  reading; the span reads within +0.05 %. Against the disarmed J-A at W=1, the min–max bar (7.9 %) would
  hide it.
- L1's gate (J-S0 disarmed, pre-L1 against the tip): +0.68 %, not claimed ⇒ no regression. Bars: 8.9 %
  (range), 2.8 % (IQR), 1.7 % (SE). The +2.44 % L1 targets is not seen; under SE, a gain of 1 % or more is
  excluded.

**Broadphase crossover (K=1).** Uniform n\* ≈ 1,109; size-disparity n\* ≈ 2,978; the in-scene
Grid/AllPairs at n = 1240 is 2.46 (W=1) and 2.59 (W=8). `GRID_LO` = 96 and `GRID_HI` = 192 sit 6–31× below
the crossovers. O3 Gate 7 read 1.92× at 100k (w4 against w1), against ≥ 2.8×; this is not ruled on.

**Resolution at K=6.** The smallest claimable end-to-end change, as range / IQR / SE, is:
- J at W=1: 10.9 / 1.4 / 2.0 %;
- J at W=8: 4.5 / 0.5 / 0.75 %;
- R at W=1: 17.8 / 3.3 % (range / SE);
- R at W=8: 7.0 / 1.3 %.

Which spread the claim rule means (min–max against IQR against SE) is still OPEN
(`p0b/diag_report.md` §6). Resolving 1 % under SE needs K ≥ 12. A witness, not a gate: the fast W=1
processes are the ones whose thread stayed on one CPU (occupancy against deviation, r = −0.67, n = 48).

**Levers (plan §3 as amended by W2/W3).**
- L4 `parallel_solve` default: build.
- L5 parallel narrowphase: build, predicted 2.4–2.6 ms (26–28 % of T(8)).
- L2: do not flip; recalibrate `GRID_LO`/`GRID_HI`.
- L6: not built (L(8) + t_narrow(8) = 8.2 % < 10 %).
- L7: blocked (gated after L6; 2.8–5.1 % predicted).
- L9: fails its rule once L5 lands (t_np(8) → 8.7 % < 20 %; Δ_J = 43.9 %).
- L8 (D2): clears after L5; ≤ 1.16 ms (cfg-A), ≤ 0.64 ms (simd).
- L10 (D3): parity 0; −38 to −58 % on the resting world.
- L3: shipped.
- Not a plan lever: the serial AllPairs broadphase is 21 % of T(8).

Receipts: `docs/measurements/2026-09-19-physics-p0/`:
- `p0/` (window 1 and the builds): `build_report.md`, `window_report.md`, `manifest.json`, `bin/SHA256SUMS`,
  `raw/runs_all.jsonl`, `raw/runs.jsonl`, `raw/reduction.json`, `raw/pass-0{0,1,2}/`, `raw/diag_w2/`.
- `p0b/` (the diagnostic and window 2): `diag_report.md`, `window_report.md`, `raw/runs.jsonl`,
  `raw/analysis*.json`, `raw/window/runs.jsonl`, `raw/window/pass-0{0,1}/`, `raw/window/broadphase/`,
  `raw/window/window_state.json`, `raw/window/wait_log.txt`.
- `p0/dry/`, `p0/wtest/` and `p0b/test/` are rehearsals, not measurements. Leave out `__pycache__/`.
```

## Files

My reduction scripts are in `(session scratch)/analyst_p0b/`:
- `red.py`, `lib.py`, `a1.py`–`a11.py`: the timing reductions;
- `jhtml.py`, `jshares.py`, `jscopes.py`: my own parser of Jolt's profiler HTML;
- `bp.py`: the broadphase crossover;
- `data.pkl`, `jshares.json`: intermediate data.

No file in `D:/wt/joltab`, `p0/` or `p0b/` was edited or created.