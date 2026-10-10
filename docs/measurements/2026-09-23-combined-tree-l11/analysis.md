**Tip cbd86a65, `--cfg default --broadphase tree`: T(1) = 7.106 ms, 0.723x Jolt v5.6.0's 9.828 ms (-27.7 %). That is claimed under SE but not under min-max, because Jolt's window-3 W=1 cell holds one 11.364 ms process; without that process the win holds under all three spreads at 0.726x. T(8) = 2.716 ms, 1.057x Jolt's 2.569 ms (+5.7 %), not claimed under either spread, so at eight workers the two cannot be told apart. The shipped AllPairs default on the same binary is 8.782 ms (0.894x) and 4.296 ms (1.672x). G9-on-C3: PASS. On the armed row at W=1, warm_apply went 0.5285 -> 0.2960 ms, Delta -0.2325 ms against the -0.19 ms bar: margin 0.043 ms, claimed under both spreads, and the worst pairing of processes is still -0.229 ms.**

# Window 5 reduction: the headline on tip cbd86a65 (tree vs AllPairs vs Jolt v5.6.0) and G9-on-C3 (parent 0ca312bd vs tip cbd86a65), recomputed from `win5/raw/`

results-analyst, 2026-09-23.

**Inputs:**
- Window 5: `win5/raw/runs.jsonl` (200 records), every per-process `run.csv` under `raw/{headline-p0*,headline-p1,g9-p*,armed-p*}/`, `raw/makeup/runs_makeup.jsonl`, `rows.json`, `bin/*` (hashed here), `gate/gate.json`.
- Bridges and the Jolt reference, all recomputed from their own raw files with the same selection rule:
  - window 3's `D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/` (the `JOLT56-T` per-frame CSVs, and `receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv` for the manifold count);
  - window 4's `scratchpad/win4/raw/` (`T-D-allpairs`, `T-A-tree-armed`);
  - window 4b's `scratchpad/win4b/raw/` (`J-As`, `J-As-a`).

**Method:**
- Reduction script: `win5/tools/analyze_win5.py` -> `win5/analyst/reduction.json` and `win5/analyst/tables.txt`. Nothing is imported from `reduce5.py`; I read its slot rule only to state below where it coincides with mine.
- I did not read numbers off the window report, `analysis.json` or `tables.md`. Where a number below matches the tester's, it is a reproduction, not a copy.
- **Statistic, as ruled:** a cell is the MEDIAN over K processes of the [0,500) window mean. Spreads are the min-max range, the IQR (inclusive quartiles) and the median's SE (1.2533 SD / sqrt K), each as % of the median.
- **Claim rule:** B against A is claimed iff |B/A - 1| > 2*hypot(sA, sB). This window's protocol requires the claim under BOTH min-max and SE. G9's own form is SE only. All three readings are printed as r / i / s.
- git was used read-only in `D:/wt/lighttable`. The only files written are the script and its two output files.

## 0. Selection, input checks, and the window report

**Selection:**
- The 200 records are: 3 void markers; 45 processes in the three voided `headline-p0` attempts (42 timed, 3 warm-ups); and the protocol set of 145 timed processes (120 originals + 25 re-runs) plus 6 warm-ups.
- My rule, per slot (block, pass, attempt, round, row, binary, W): the original if valid and clean, else its re-run if valid and clean, else the slot is dropped. That gives **117 of 120 slots used and 3 dropped**:
  - `HL-D-tree` tip W1, headline pass 1 round 0
  - `HL-D-allpairs` tip W2, headline pass 1 round 0
  - `J-As` tip W8, g9 pass 1 round 1
- **Agreement with the tester:** the tester's rule (re-run first) picks the same processes in all 120 slots. "Clean", recomputed from the receipts (both 5-s receipts <= 5 %, no build or lane process busy, present or appearing), disagrees with the driver's flags 0 times.

**Checks on the inputs (all hold):**
- **Window means:** recomputed from `run.csv`, every window mean equals the driver's `mean_ms` and the runner's `window_mean_ns` exactly (max |delta| 0). Every window holds 500 steps.
- **Structure, over all 196 processes that produced a result (protocol set, voided attempts, warm-ups): 0 problems.** Each has:
  - exit 0, the row's pose hash (`0x32d5e235342b4143` J, `0xee2a67a98434919a` rest) and `expect_pose: match`;
  - void steps 0, `msvc`, `workers` = `pool_workers` = W, and its binary's sha256 and commit;
  - the row's broadphase kind;
  - TreeDiag exactly `static_rebuilds 1, members 1, all else 0` on tree rows, and all zero on AllPairs rows;
  - on armed rows, the armed flag, `drops 0` and `solve_on_dispatcher_steps 0`;
  - mask `0xffff`, ring traffic 0.
- **Binaries:** each file's sha256 equals `rows.json` and `SHA256SUMS`. Tip `9c7caff1…` = cbd86a65; parent `3993684b…` = 0ca312bd.
- **Git (read-only):**
  - `cbd86a65^` = `0ca312bd`. C3 changes only `solver/colored.rs` in `src/`.
  - `0ca312bd` merges the line `f25ea2fa` (A1b, A3 = KE13/KE14, hwrt, A4, A9/A9b, AH, tree C1+C3) into `4595dbfa` (L11 C0-C2 + window 4b docs).
- **Printed config:** parent == tip in 4/4 (row, W) pairs; the gate reported 12/12.
  - `--cfg default` at every W: Manual select, `simd_solve` on, `parallel_solve` on, `parallel_broadphase` **off**, `parallel_narrowphase` on, sleeping off, substeps 4, relax 2.
  - `--cfg as`: all three parallel flags off at W=1 and on at W=8.
- **Pose gate** (untimed, `gate.json`):
  - 24 runs exit 0 with the row's hash.
  - The pose bytes are identical across rows of a scene (sha `eff361e1…` J, `89f084c2…` rest).
  - 6 red controls (501 steps, three per binary) exit 4 with `mismatch`.
- **Jolt v5.6.0, recomputed from window 3's raw:** 9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms at W 1/2/4/8/16, K=6 each. These equal the task's values. The per-frame mean equals `mean_ms` to 1.8e-15. Manifolds per step over [100,500) = 8,489.0.
- **Receipts (145 protocol processes):**
  - Before: median 2.70 %, max 4.97 %, 0 over 5 %.
  - After: median 2.95 %, p90 6.71 %, max 13.20 %, 28 over 5 %.
- **During-process witness (`others_busy_pct`), over the 117 used processes:** median 1.48 %, p95 4.55 %, max 7.20 %. Two are over 5 %:
  - `HL-R-tree` tip W8: 7.20 %, 3.8029 ms (that cell's median process);
  - `HL-D-allpairs` tip W4: 5.73 %, 5.5610 ms (that cell's max).

  Window 4b's used median was 0.33 %, so the background here was about 4x window 4b's.
- **Used blocks:** headline 02:16-02:48, g9 02:51-03:04, armed 03:07-03:12.

**Against the window report:**
- Every cell, spread, ratio, both thresholds, per-stage span (§ 3 uses reading B, the per-process median over [100,500)), counter, receipt count and make-up variant in the report reproduces from the raw files.
- One finding is not in the report and bears on § 3 below. On the same binaries, the disarmed g9 block's W=8 cells sit 4.3-5.1 % above the armed block's: parent 4.726 vs 4.498, tip 4.510 vs 4.323. In window 4b the two were equal (tip 4.240 vs 4.242).

## 1. The headline: the tip's default row with the tree, against the same binary's AllPairs and Jolt v5.6.0

**T(W): median [min-max] ms/step, with r / i / s % beside each. Every comparison shows its effect, the bars r/i/s %, and the claim flags.**

| W | tree (`--cfg default --broadphase tree`) | AllPairs (shipped default) | Jolt v5.6.0 (window 3) | **tree / Jolt** | AllPairs / Jolt | tree / AllPairs |
|---|---|---|---|---|---|---|
| 1 | **7.106** [6.891-7.286] K=5 (5.56 / 2.26 / 1.19) | 8.782 [8.528-9.136] (6.92 / 5.34 / 1.57) | 9.828 [9.518-11.364] (18.78 / 3.31 / 3.50) | **0.723** (-27.7 %; 39.2 / 8.0 / 7.4; **n/Y/Y**) | 0.894 (-10.6 %; 40.0 / 12.6 / 7.7; n/n/Y) | 0.809 (-19.1 %; 17.7 / 11.6 / 3.9; **Y/Y/Y**) |
| 2 | 4.659 [4.469-5.559] (23.39 / 6.62 / 4.42) | 6.435 [6.244-6.557] K=5 (4.86 / 4.05 / 1.27) | 5.770 [5.703-5.818] (1.99 / 0.87 / 0.38) | 0.807 (-19.3 %; 47.0 / 13.4 / 8.9; n/Y/Y) | 1.115 (+11.5 %; 10.5 / 8.3 / 2.6; Y/Y/Y) | 0.724 (-27.6 %; 47.8 / 15.5 / 9.2; n/Y/Y) |
| 4 | 3.315 [3.297-3.398] (3.05 / 1.17 / 0.61) | 4.976 [4.867-5.561] (13.94 / 1.73 / 2.62) | 3.581 [3.519-3.663] (4.02 / 2.36 / 0.83) | 0.926 (-7.4 %; 10.1 / 5.3 / 2.1; n/Y/Y) | 1.389 (+38.9 %; 29.0 / 5.9 / 5.5; Y/Y/Y) | 0.666 (-33.4 %; 28.5 / 4.2 / 5.4; Y/Y/Y) |
| 8 | **2.716** [2.520-2.963] (16.31 / 9.96 / 3.42) | 4.296 [4.213-4.539] (7.58 / 4.09 / 1.57) | 2.569 [2.501-3.124] (24.26 / 3.09 / 4.76) | **1.057** (+5.7 %; 58.5 / 20.9 / 11.7; **n/n/n**) | 1.672 (+67.2 %; 50.8 / 10.3 / 10.0; Y/Y/Y) | 0.632 (-36.8 %; 36.0 / 21.5 / 7.5; **Y/Y/Y**) |

**Values by process:**
- tree W1: 6.891, 6.995, 7.106, 7.156, 7.286
- tree W8: 2.520, 2.605, 2.702, 2.730, 2.956, 2.963 (pass 0 median 2.730, pass 1 median 2.605)
- AllPairs W1: 8.528, 8.546, 8.689, 8.875, 9.109, 9.135
- AllPairs W8: 4.213, 4.218, 4.296, 4.296, 4.452, 4.539

**Rest scene (tip, no Jolt reference):**
- W1: tree 9.768 [9.082-10.273] against AllPairs 11.274 [11.041-11.349] = 0.866 (-13.4 %; 25.0 / 5.5 / 4.2; n/Y/Y). The min-max failure is the tree cell's 12.2 % range.
- W8: 3.803 against 5.397 = 0.705 (-29.5 %; 25.7 / 8.8 / 4.8; Y/Y/Y).

**Per manifold per step over [100,500), each side's own count** (boyko 4,519.26 in every process; Jolt 8,489.0; Jolt carries 1.878x as many manifolds):

| W | tree µs | AllPairs µs | Jolt µs | tree / Jolt | AllPairs / Jolt |
|---|---|---|---|---|---|
| 1 | 1.580 | 1.957 | 1.1445 | 1.38 | 1.71 |
| 2 | 1.033 | 1.427 | 0.6750 | 1.53 | 2.11 |
| 4 | 0.734 | 1.101 | 0.4260 | 1.72 | 2.59 |
| 8 | 0.600 | 0.950 | 0.3063 | 1.96 | 3.10 |

The step ratio of tree to Jolt over [100,500) is 0.735 / 0.815 / 0.917 / 1.042 at W 1/2/4/8.

**Scaling, T(1)/T(8):** tree 2.616, AllPairs 2.044, Jolt 3.825. By W for the tree: 1.525 / 2.144 / 2.616 at W 2/4/8; Jolt 1.703 / 2.744 / 3.825.

**Same-binary consistency:** `HL-D-allpairs` against `J-As` on the tip.
- W=1, the same code path: +1.80 %, n/n/n.
- W=8, where cfg `as` adds `parallel_broadphase`: -4.75 %, SE only. Not separable from the block drift noted in § 0.

**Sensitivity** (not the ruled statistic):
- Jolt W1 without its 11.364 ms process is 9.783 [9.518-10.073] (5.67 / 2.08 / 1.20). Tree/Jolt becomes 0.726, bars 15.9 / 6.2 / 3.4, **Y/Y/Y**.
- Jolt W8 without its 3.124 ms process: tree/Jolt 1.067, still n/n/n.
- Tree W2 without its 5.559 ms process: tree/AllPairs Y/Y/Y.
- Tree W8 by pass: 1.063x (pass 0) and 1.014x (pass 1) Jolt.

**Where the campaign stands:**
- **W=1: the default cfg with the tree is 2.72 ms (27.7 %) faster than Jolt v5.6.0 per step (0.723x).**
  - Claimed under SE and IQR, not under min-max. The min-max failure is Jolt's own window-3 cell, which the owner's ruling does not allow re-running; without its single slow process the claim holds under all three.
  - The shipped AllPairs default is 0.894x, SE only.
  - Per manifold the tree row is 1.38x Jolt. The per-step lead rests on boyko's 1.88x smaller contact set, as in windows 3 and 4b: the truth lies between the two readings.
- **W=8: 2.716 ms against 2.569 ms, +0.147 ms (1.057x), not claimed under either spread. The tree row cannot be told apart from Jolt v5.6.0 at eight workers in this window.**
  - The shipped AllPairs default is 1.672x, claimed.
  - Per manifold the tree row is 1.96x Jolt.
- **Since window 4:** the tree row went 9.026 -> 7.106 ms at W=1 (0.918x -> 0.723x) and 3.699 -> 2.716 ms at W=8 (1.440x -> 1.057x). Both are cross-window, the code difference is L11 C1-C3, and W=8 carries the bridge caveat in § 3.

**What the remaining gap is made of (W=8).** The budget takes the tip's armed `J-As-a` spans (reading B) and swaps the AllPairs broadphase span (1.954 ms) for window 4's tree broadphase span. I recomputed that span from window 4's raw with the same reading: 0.4665 ms at W=8, of which the query is 0.411 ms.
- The budget predicts 2.678 ms; the tree row measures 2.610 ms (median over steps [100,500)), -2.5 %. At W=1 it predicts 6.891 and measures 6.950 (+0.9 %). So the budget is a fair decomposition.
- **Spans that do not shrink between W=1 and W=8 (W-independent): 1.392 ms, 53 % of the W=8 row** (20 % at W=1):
  - tree broadphase 0.4665 ms (query 0.411);
  - serial-in-solve 0.771 ms: solve_build 0.336, warm_apply 0.304 after C3, integrate 0.080, store 0.031, gravity 0.013;
  - build_graph 0.116, gather 0.026, apply 0.012.
- **Spans that scale:** narrowphase 0.540, solve kernel (wide + narrow colours) 0.657, executor gap 0.034.
- **The named open item is the tree query.** Window 4 found 334 ns per queried row against the design's 100-150 ns/row; re-derived from window 4's raw here: 333.8 ns/row at W=1, 331.5 at W=8. Against the design's t_q of 0.124-0.186 ms, the excess at W=8 is **0.225-0.287 ms**, larger than the whole +0.147 ms W=8 step gap to Jolt.
  - Arithmetic only, not measured, and it holds only if nothing else moves: at the design's query cost the row would read 2.43-2.49 ms, 0.95-0.97x Jolt.
- The next W-independent block is the serial solve setup, 0.64 ms of build + warm. That 1.39 ms floor is why T(1)/T(8) is 2.62 against Jolt's 3.83.

## 2. G9-on-C3: the warm_apply lever, parent 0ca312bd against tip cbd86a65

**The gate.** G9 in `levers/L11-solve-setup/02-DESIGN-REV1.md` reads: "Realized-gain gates, >= 0.6 x the lower predicted Delta: … warm_apply -0.19 ms (C3, J-As only)".
- **The design's listed deltas ARE already the realized-gain bars:** 0.614 -> 0.30 gives a lower predicted Delta of -0.314, and 0.6 x -0.314 is about -0.19. Window 4b adjudicated this (its § 0, note 1).
- **I did not discount them a second time.** A second 0.6 would give -0.114; that would also pass, but it is not the bar.

**warm_apply, armed `J-As-a`, ms:**

| reading | W | parent [min-max] | tip [min-max] | Delta ms (effect) | bars r / s % | 2*hypot(SE) ms | worst pairing | claimed r/i/s | verdict vs -0.19 |
|---|---|---|---|---|---|---|---|---|---|
| **B** (per-process median over [100,500); design / window-4b convention) | **1** | 0.5285 [0.5265-0.5297] | **0.2960** [0.2949-0.2978] | **-0.2325** (-44.0 %) | 2.33 / 0.48 | 0.0017 | -0.2287 | **Y/Y/Y** | **PASS**, margin 0.0425 |
| B | 8 | 0.5391 [0.5342-0.5459] | 0.3043 [0.2995-0.3131] | -0.2348 (-43.6 %) | 9.93 / 1.99 | 0.0075 | -0.2212 | Y/Y/Y | PASS (reported; the gate is W=1) |
| A (per-process mean over [0,500)) | 1 | 0.5327 | 0.3063 | -0.2264 (-42.5 %) | 6.78 / 1.43 | 0.0048 | -0.2194 | Y/Y/Y | PASS |
| A | 8 | 0.5553 | 0.3243 | -0.2310 (-41.6 %) | 27.59 / 5.36 | 0.0282 | -0.2141 | Y/Y/Y | PASS |

- **Against window 4b's C2 value of 0.519 ms** (cross-window): tip - 0.519 = -0.223 ms, which also passes. The in-window parent reads 0.5285, +1.9 % over window 4b's C2. That parent carries the line merge, and G9 gates parent against tip, so the in-window -0.2325 is the binding number.
- **The realized Delta is 0.74x the lower predicted Delta** (-0.314). The tip's 0.296 ms is 0.0654 µs per manifold at 4,524.2, inside the design's target band of 0.15-0.30 ms (0.033-0.066 µs), at its slow edge.
- **Other spans** (reading B, tip against parent):
  - solve_colored: -0.329 at W1 (n/Y/Y) and -0.205 at W8 (n/Y/Y).
  - Solve kernel (wide + narrow colours): -0.077 at W1 and +0.0006 at W8, n/n/n. The "wide colours not claimed slower" gate holds.
  - Setup (build + warm + store): 0.876 -> 0.656 at W1 (SE only) and 0.890 -> 0.672 at W8 (IQR and SE). The design's setup band of 0.44-0.93 is met.
  - solve_build (+0.009 / +0.012) and store (+0.003 / +0.004) are not claimed; C3 does not touch them.
  - broadphase (-0.0001 / -0.006) and narrowphase (-0.005 / +0.003): the untouched stages reproduce between the two binaries to within 0.2 %.
- **Counters** are identical on both binaries at both W: 4,519 manifolds, 9,559 pairs, 11 colours (9 wide), 108 waves and 17,054 points per step (per-process medians); 4,524.25 manifolds over [0,500); 54,660 waves in total.

**T(1)/T(8), parent -> tip:**

| row @W | parent | tip | Delta ms (effect) | bars r / i / s % | claimed |
|---|---|---|---|---|---|
| J-As @1 | 8.859 [8.709-9.136] | 8.627 [8.389-8.981] | -0.232 (-2.62 %) | 16.77 / 7.19 / 3.19 | n/n/n |
| J-As @8 | 4.726 [4.455-4.896] | 4.510 [4.279-4.601] K=5 | -0.216 (-4.57 %) | 23.48 / 9.32 / 4.61 | n/n/n |
| J-As-a @1 (armed wall) | 8.821 [8.629-8.986] | 8.473 [8.396-8.668] | -0.348 (-3.95 %) | 10.34 / 4.47 / 1.95 | n/n/Y |
| J-As-a @8 (armed wall) | 4.498 [4.425-5.270] | 4.323 [4.179-4.425] | -0.175 (-3.89 %) | 39.28 / 10.74 / 7.80 | n/n/n |

- **C3's step gain is its warm_apply gain.** The step moves -0.232 / -0.216 ms on J-As against warm_apply's -0.2325 / -0.2348. That is 2.6-4.6 % of the step, right at K=6's SE resolution. It is claimed only on the armed W=1 wall, under SE.
- **The J-As W8 make-up variant** (outside the protocol set): tip 4.438, -6.09 %, n/n/Y.
- **The design has no separate step gate for C3.** The lever's T(1)/T(8) gates (-0.81 / -0.61 ms) were passed by C1+C2 in window 4b (-1.996 / -1.103).
- **Cumulative L11 C0 -> C3**, chained across windows 4b and 5: -2.23 ms at W=1 and -1.32 ms at W=8.

## 3. The bridges

**Bridge 1: the tip's AllPairs default row against window 4's `T-D-allpairs`** (runner_c3 `19456ab4` = a46b8287). Recomputed: 10.554 [10.464-10.738] at W1 and 5.491 [5.327-5.549] at W8, K=6 each.
- **Literal comparison: does not reproduce, and cannot.**
  - W1: 8.782 against 10.554 = 0.832 (-16.8 %, -1.771 ms; 14.8 / 10.9 / 3.3; Y/Y/Y).
  - W8: 4.296 against 5.491 = 0.782 (-21.8 %, -1.195 ms; Y/Y/Y).
  - Window 4's binary predates all of L11 (C0-C3), and also A1b (`8af0e3b9`) and A3; git shows both in `a46b8287..f25ea2fa`. The shift is the lever, not drift.
- **Lever-adjusted chain.** Predicted = window 4's cell + (window 4b tip - parent, C0 -> C2 on J-As) + (window 5 tip - parent, -> C3 on J-As).
  - W1: 10.554 - 1.996 - 0.232 = **8.326**; measured 8.782; residual **+0.457 ms (+5.5 %)**. The chain's absolute bars are 2*sqrt(sum SE^2) = 0.529 and 2*sqrt(sum range^2) = 3.58, so the residual is within both.
  - W8: 5.491 - 1.103 - 0.216 = **4.172**; measured 4.296; residual **+0.124 ms (+3.0 %)**. Bars 0.540 / 3.97: within both.
  - The W1 residual splits into three links, none claimed and all positive:
    - window 4 -> window 4b parent (C0 + A1b, two windows): +0.140 (+1.3 %; 22.7 / 6.0 / 2.8; n/n/n);
    - bridge 2 below: +0.162 (+1.9 %; n/n/n);
    - default against cfg `as` on the tip at W=1, the same code path: +0.155 (+1.8 %; n/n/n).
- **Verdict: bridge 1 holds as a chain, and nothing is invalidated.** It is a weak bridge: six cells, with an SE bar of about 6 %.

**Bridge 2: this window's parent `J-As` against window 4b's tip `J-As`** (8.697 / 4.240, K=12).
- **What differs in the code:** the line merge. That is A3's ECS query changes (`query.rs`, `query_view.rs`, `dense_store.rs`), the tree broadphase code, and a `ResMut<BroadphaseTree>` plus one match arm in `physics_broadphase`. The solver kernel is code-identical (only a one-line change in `solver/contact.rs`), and the thread pool's `src/` is unchanged.

| row @W | window 4b tip | window 5 parent | ratio (effect) | bars r / i / s % | claimed |
|---|---|---|---|---|---|
| J-As @1 | 8.697 [8.395-9.308] | 8.859 [8.709-9.136] | 1.019 (+1.9 %, +0.162) | 23.1 / 7.6 / 2.9 | **n/n/n: reproduces** |
| J-As @8 | 4.240 [4.204-5.004] | 4.726 [4.455-4.896] | **1.115 (+11.5 %, +0.486)** | 42.1 / 9.0 / 5.7 | **n/Y/Y: does NOT reproduce under SE** (G9's form) or IQR; holds only under min-max |
| J-As-a @1 | 8.554 | 8.821 | 1.031 (+3.1 %) | 15.7 / 4.3 / 2.1 | n/n/Y |
| J-As-a @8 | 4.242 | 4.498 | 1.060 (+6.0 %) | 37.8 / 7.9 / 7.4 | n/n/n (holds) |

**Stage split, armed, reading B (window 4b tip -> window 5 parent):**
- **W1:** the code-identical kernel reproduces: wide colours -0.2 %, biased -0.1 %, relax -0.2 %. What moved: broadphase +2.5 % (+0.047 ms, IQR and SE), solve_build +16.6 % (+0.046, SE), narrowphase +1.4 % (SE), warm +1.9 %, store +0.0045 ms. That is a small code cost from the line, under 2 % of the step.
- **W8:** every stage rose 1-6 %, **including the code-identical kernel**: wide colours +4.1 % (SE only), relax +4.8 %, biased +3.1 %, broadphase +2.8 %, narrowphase +2.8 %, executor gap +5.8 %.
- **Machine state:**
  - CPU clock % (the driver's `perf`): armed W8 126.7 in window 4b against 126.6 here; disarmed J-As W8 126.2 against 123.9.
  - Witness median: 0.35 % in window 4b against 1.79 % here on disarmed J-As W8 and 1.17 % on the armed twin.
  - The in-window g9-vs-armed block drift of 4-5 % (§ 0) is about the size of this bridge's own shift.

**Reading of bridge 2:**
- The W8 shift is +11.5 % in the disarmed g9 block and +6.0 % in the armed block run 3-15 minutes later on the same binaries.
- The code-identical kernel moves +4.1 % at W8 while reproducing to -0.2 % at W1.
- That pattern points to machine state at W=8 (background load landing on worker cores, with each parallel wave waiting for its slowest worker) rather than to the line's code. This window's rows cannot rule out a W8-only code cost, though.

**What that invalidates:**
- **Not invalidated: every in-window comparison.** That covers tree vs AllPairs, parent vs tip, and the warm_apply gate. They are interleaved within a block, so block drift cancels to first order: parent and tip moved together, with tip/parent 0.954 in g9 and 0.961 armed.
- **Qualified: every cross-window reading at W=8.** That covers tree/Jolt 1.057, AllPairs/Jolt 1.672, and "window 4's 3.699 -> 2.716 ms".
  - This window's W=8 cells can sit 6-11 % above what window 4b's machine gave on unchanged code.
  - The tree/Jolt W=8 verdict does not change: it is not claimed either way, and a 6-11 % correction would read 0.94-0.99, still inside the bars.
  - AllPairs/Jolt at W=8 stays claimed.
- **Cross-window readings at W=1 stand;** the bridge holds there to within 1.9 %.

## 4. What this window cannot claim

1. **The tree is not the shipped default, and this window does not make it one.**
   - `--cfg default` prints AllPairs under Manual select in every process; the tree rows force `--broadphase tree`.
   - Window 4's G4 stop rules still stand: rule 1 (J snapshot 0.443 > 0.30), G5's tree span (0.476 / 0.466 > 0.36 / 0.35), and rules 6 and 9.
   - The tree lane's C4 (the default flip) is not decided.
   - No G4 arm, tree span or query cost was re-measured here: the only armed rows run AllPairs, and the tree rows are disarmed (wall time only). The 334 ns/row is window 4's measurement; I re-derived it from window 4's raw, not from new data.
2. **No Jolt per-stage comparison.** Jolt was not re-run and has no per-stage profile. Every Jolt ratio is a whole-step, cross-window ratio against window 3's cells, and the per-manifold ratios use window 3's receipted 8,489.0.
3. **Nothing about the tree's C4/C5 or the sleeper set.** Sleeping is off in every row, `sleeper_rebuilds` is 0 in every process, and there are no churn, R-S or Auto-select rows.
4. **Tree vs Jolt is claimed under both spreads at no W.** W=1/2/4 are claimed under SE and IQR only. W=1's min-max failure is Jolt's own outlier process; the sensitivity reading without it is not the ruled statistic. W=8 is not claimed in either direction.
5. **C3's other G9 rows were not run:** J-A (cfg-A "not claimed slower"), R, R-S, S16, W 2/4/16, K=12, and the canary. C3 has passed warm_apply and "wide colours not claimed slower" on J-As-a; the rest of G9 for C3 is open.
6. **C3's step gain is not claimed on the disarmed rows.** It is claimed on the armed W=1 wall under SE only.
7. **Tree vs AllPairs is not claimed under min-max at W2** (one 5.559 ms tree process) **or on rest W1.**
8. **The make-up processes are outside the protocol set.**

## 5. Open points for the orchestrator

1. **Bridge 2 at W=8 can be settled in about a minute of timed processes.** Interleave window 4b's `runner_tip.exe` (`29dbd993…`, still in `scratchpad/win4b/bin/`) with this window's `runner_parent.exe` on J-As at W=8, K=6 each, with a W=1 control. That isolates the line merge from machine state.
2. **The tree query is the largest named item** in the W=8 row's 1.39 ms W-independent floor, and on its own it exceeds the step gap to Jolt. Next is the serial setup at 0.64 ms.
   - Arithmetic for **L11's own C4** (D10, the parallel fill), not the tree lane's C4: solve_build(8) after C3 is 0.336 ms (SE 0.010), which is 7.45 % of T(8) on J-As tip.
   - That is above D10's 5 % trigger and above its SE. I am reporting the number, not making the build decision.
3. **The three voids and the 69-poll idle wait came from a rust-analyzer launched under the tester's own host.** A timing agent should run with the lane out of `linkedProjects`, or with the LSP off, for the length of a window.
4. **`analysis.md` was not written to disk.** The harness forbids report files for this role; this text is its content.

**Suggested lane line:** `L11 C3 timed 2026-09-23, window 5: warm_apply 0.529 -> 0.296 ms at W=1 (-0.233 against the -0.19 bar, PASS, claimed under both spreads); J-As -0.23 / -0.22 ms at W=1/8 (not claimed at K=6). Headline on cbd86a65, default cfg with --broadphase tree: 7.106 ms at W=1 (0.72x Jolt v5.6.0, SE) and 2.716 ms at W=8 (1.06x, not claimed); shipped AllPairs 0.89x / 1.67x.`

**Sanity check.** I reran `analyze_win5.py` end to end (exit 0). It reproduces every number in the tester's report, and my selection rule picks the same 117 processes as the tester's. I also recomputed each reference cell the task quotes from its own raw data: Jolt 9.828 / 5.770 / 3.581 / 2.569 / 2.388, window 4's 10.554 / 5.491, window 4b's 8.697 / 4.240, and warm_apply 0.519. I re-read the ask against sections 1-4. `analysis.md` is the one deliverable not on disk.

Files are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win5/`:
- `tools/analyze_win5.py`
- `analyst/reduction.json`
- `analyst/tables.txt`
