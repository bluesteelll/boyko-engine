P1 the W8 headline does NOT HOLD (ours/Jolt 0.872 claimed in block B only; A 0.836 and pooled 0.811 not claimed under min-max; per manifold ours is 1.60x Jolt in B, 1.48x pooled); P2 G-TW PASS (realized ΔT(1) = 1.937 ms, 2.15x the 0.900 ms bar, claimed; no claimed regression at any W; the canary is seen under SE only: its 5.38 % rise sits under the 10.96 % min-max bar); P3 G9 CLOSED (J-A tip vs parent +0.14 / +0.02 / −0.02 % at W 2/4/16, none claimed); P5 Jolt's step is ~65 % solve and ~27 % FindCollisions, and its narrowphase is the body-pair cache: 8,488 of 8,489 manifolds per frame go through "Add Constraint From Cached Manifold", which also builds the contact constraint. Per manifold, our only excess at W1 is the narrowphase (+236 ns), which C4 removes; at W8 it is our five serial stages (+233 ns)

# Window 7 reduction: the Jolt headline in one window, L9 C4's G-TW, G9's last three cells, and ours against Jolt stage by stage, recomputed from `win7/raw/`

results-analyst, 2026-09-25.

**Inputs:**
- Window 7: `raw/runs.jsonl` (542 records), every `run.csv`, `per_frame_discrete_thW.csv`, `stdout.txt` and Jolt `profile_chart_*.html` dump, `rows7.json`, `plan.md`, `bin/SHA256SUMS`, `bin/COMMIT.txt`, `gate/` (the fixtures, `gate_all.json`, `gate_run.log`, the Jolt receipt CSVs), `wait_log.txt`, `progress.txt`, `raw/window_log.txt`, `raw/shell_log.txt`, `WINDOW_DONE` (`exit 0 complete`) and `WINDOW_DONE.prev-1790293553` (the stop).
- Context only, never pooled: window 3's Jolt v5.6.0 cells (9.8282 / 5.7705 / 3.5814 / 2.5693 / 2.3885 ms at W 1/2/4/8/16; the constants of `tools/reduce7.py`, recomputed in window 6 `analysis.md:13`), and window 6's L9 numbers (`docs/measurements/2026-09-24-physics-window6/analysis.md` §2).
- Jolt v5.6.0 source, read only (`D:/tmp/jolt/wt-v5.6.0-parity`), to place its profile scopes:
  - `Jolt/Physics/PhysicsSystem.cpp:1049-1082` (`ProcessBodyPair`: the body-pair cache first, collision only on a miss);
  - `Jolt/Physics/Constraints/ContactConstraintManager.cpp:860` (the "Add Constraint From Cached Manifold" scope), `:921` (`CreateConstraint`) and `:957` (the non-penetration setup);
  - `Jolt/Physics/PhysicsSettings.h:84,87` (10 velocity and 2 position iterations, unchanged by `PerformanceTest.cpp:376-381`).

**Method:**
- My script, `tools/analyze7.py`, recomputes every process statistic from the raw files: the window means and the per-step span and count columns from `run.csv`, and Jolt's frame means from its per-frame CSV. It parses every SUMMARY, every Jolt stat line and every patch banner, and it re-parses every profile dump with its own scope-tree walk. It re-derives validity and cleanliness from the files and the receipts, and imports nothing from `reduce7.py`. It then compares its results against the driver's per-process fields and against `raw/reduction.json`.
- **A cell** is the median over K. r = min–max, i = IQR, s = 1.2533·SD/√K, each relative to the median.
- **Claim rule:** B against A is claimed iff |B/A − 1| > 2·hypot(sA, sB) under both r and s (i printed); no claim when either side has K < 3.
- **Slot rule** (window 6's): per (block, pass, round, row, binary, W), the original if valid and clean, else its re-run if valid and clean, else the slot is dropped. Clean means both 5-s receipts are ≤ 5.0 %.
- Flags are written r/i/s (Y = claimed under that spread).
- Per manifold uses each side's own count over [100,500): ours 4,519.2575 (every used process, from `run.csv`), Jolt 8,489.0 (recomputed from `gate/jolt_receipt/j56_W{1,8}/receipt_discrete_thW.csv`).
- git was read-only.

## 0. Selection, input checks, the interruption, and the driver's reduction

**Selection:**
- 542 records = 530 processes (12 warm-ups, 450 originals, 68 re-runs) + 12 pass markers. There are 0 voided passes, and `binaries_after` reads "all match" at both window ends.
- Each pass has one run tag: `010216` for P1A-jolt-p0 (the first launch), `024553` for the other eleven (the resumed launch).
- **431 of 450 slots are used, 49 of them re-runs. 19 slots are dropped** because both attempts had a receipt over 5 %: 15 in P1A-jolt (9 in p0, 6 in p1) and 4 in P2-L9GTW-p0.
- The cells below K = 6:
  - P1 block A: tree W1/W4/W8 K=5, W2/W16 K=4; AllPairs W1/W2/W8/W16 K=5, W4 K=3; Jolt W4 K=5 (so the pooled cells are K = 9–12);
  - P2: `L9-R-off` W1 and W8, `L9-JD-on` W4 and `L9-JA-a-off` W8, each K=5.

**Input checks (all hold):**
- **Structure: 0 problems in 518 non-warm-up processes.**
  - Every runner process has exit 0 and 0 void steps, the csv step count equal to the row's steps, and `expect_pose: match` with the hash equal to its fixture: J500 `0x32d5e235342b4143`, JAon500c4 = JDon500c4 `0x30c5438bc6ad9ffa`, R1100 `0x87e561d20589d4a5`, Ron1100c4 `0xc8bbe34cf6a8afc6`.
  - It also has `workers` = `pool_workers` = W and msvc, the armed flag its row expects, drops 0 and no disarmed ring traffic, and the row's broadphase.
  - TreeDiag reads `static_rebuilds 1, members 1, evictions 0` on tree rows and all zero on AllPairs rows.
  - `contact_reuse` equals the row's flag (off on every trunk row, which passes no flag), `canary_ns` is present on `L9-JC`, the mask reads `0xffff`, and the exe sha256 equals `bin/SHA256SUMS`.
  - Every Jolt process has exit 0, one stat line with threads = W and hash `0xb8522b4e3fc62cfe`, the banner `boyko-parity-patch v1 … allow_sleep=0, receipt=0`, and 500 frames. The profiled build also has its dumps at iterations 100/200/300/400.
- **Against the driver's per-process fields:** every valid flag, clean flag, `mean_ms` and `cols` value (mean and median of every span and count column over every metric window) reproduces.
- **Poses per row across W:**
  - `L9-JA-on`, `L9-JD-on`, `L9-JA-a-on` and `L9-JC` read one hash, `0x30c5438bc6ad9ffa`, at every W they ran.
  - `L9-R-on` reads `0xc8bbe34cf6a8afc6` at W 1–16.
  - Every reuse-off row reads the C0 hashes: J `0x32d5…`, R `0x87e5…` (the gate's 10 C0-fixture matches, `plan.md` §3). This is the design's pose line, `L9-contact-reuse/02-DESIGN-REV1.md:414`.
- **Counts:** ours is constant, 4,519.2575 manifolds and 17,020.4975 points per step over [100,500), in every used `H-trk-*` and `S-trk-tree-a` process. Jolt's receipt reads 8,489.0 manifolds and 31,111.9575 points (W1 = W8).
  - Jolt's count equals ours at frame 0 (4,495 manifolds). It diverges while the pile collapses (frames 10–70) and holds 8,489 from frame 80 on.
  - So Jolt carries 1.878× our manifolds and 1.828× our points (3.665 against 3.766 points per manifold). The per-manifold and per-point ratios below therefore differ by at most 2.7 %.
  - Why Jolt's count diverges is untested.
- **Receipts (628 distinct, warm-ups and pass openings included):** median 1.92 %, p90 5.68 %, max 21.98 %, 88 over 5 %. `browser.exe` topped 55 of the 88 hot receipts and `claude.exe` 29. No build or lane process was present at any receipt or during any process.
  - The only tool process that appears is one `git.exe` (0.48 CPU-s) inside a P2-p0 re-run that the after-receipt (21.98 %) had already dropped.

**The interruption, verified from `wait_log.txt`, `raw/shell_log.txt`, `raw/window_log.txt` and the receipts:**
- The first launch was at 01:02:16 with `--cutoff 10:00`. `plan.md`'s hard stop of 04:15 was not the one in force; this is recorded, not corrected.
  - Idle was reached at 01:09:27 after 8 polls, and **P1A-jolt-p0 ran 01:09:37–01:29:50** (45 originals, 19 re-runs).
  - The hot receipts in that pass were `claude.exe` (an agent session, 29 of them). The used set's during-process witness has median 0.82 %, max 2.98 %.
- The wait before P1A-jolt-p1 opened at 01:29:50 at 16.49 % (browser 7.50 CPU-s, steamwebhelper 1.88 s per 10 s).
  - From 01:30:50 `Inscryption` topped every poll (15.97–27.13 %) through poll 30 at 01:58:50. The wait timed out at 01:59:50, and the driver exited 3, "STOP before P1A-jolt-p1" (`WINDOW_DONE.prev-1790293553`).
  - D: free space went from 46.72 to 123.06 GB between the 01:29:50 and 01:30:50 polls. Something deleted about 76 GB after P1A-jolt-p0's last process and before any other one.
  - **No process ran from 01:29:50 to 02:54:06**, and `Inscryption` appears in no poll after 01:58:50 and in no receipt or during-process top-5 of any process.
- The resume was at 02:45:53 with `--cutoff 10:30 --resume`.
  - P1A-jolt-p0 was skipped and the canary references restored. Idle was reached at 02:54:06 after 9 polls: poll 1 read 47.53 % with `rust-analyzer` on top, and polls 2, 3, 5 and 6 read 5.16–10.60 %.
  - Every later pass waited for three quiet polls, and the window completed at 05:03:57 (`WINDOW_DONE` `exit 0` / `complete`).
- **So the game ran under no pass. But two passes ran under the owner's browser, which the idle rule and the 5-s receipts let through.**
  - `browser.exe` topped the during-process witness of 138 processes, 02:55:13–03:59:56: all of **P1A-jolt-p1** (02:55–03:12) and **P2-L9GTW-p0** (03:14–04:00).
  - The used set's witness (median / p90 / max) reads 2.28 / 6.05 / 8.87 % in P1A-p1 and 1.90 / 4.10 / 5.61 % in P2-p0, against 0.54–0.66 % medians in every pass from 04:02 on.
  - The protocol does not gate this witness (window 6 README, "How to re-run" item 5), so these processes are used. They are the cause of block A's width in P1 and of P2's wide W ≥ 2 cells (§1, §2).

**Against the driver's `raw/tables.md`:** every cell and comparison reproduces: 151 cells (K, median, min, max) and 124 comparisons (ratio and all three claim flags), compared by key, 0 differences. My disagreements are in reading, not arithmetic:
1. **Per manifold, the driver quotes only the pooled ratio** (1.100× at W1, 1.479× at W8). The pooled Jolt [100,500) cell is inflated by block A's loaded pass: at W1, 10.331 ms pooled against 9.498 in block B (+8.8 %). So the pooled ratio understates our per-manifold deficit. **Block B**, the quiet block and P5's neighbour in time, reads **1.178× / 1.601×**.
2. **The driver's Jolt "window term"** (1.056 / 1.022 / 1.049 / 1.057 / 1.121 at W 1/2/4/8/16) uses the pooled cell. Block B alone reads **0.977 / 0.995 / 1.020 / 0.976 / 1.008** of window 3. The quiet block reproduces window 3 within 2.4 % at every W; the pooled excess is block A's load, not a window term.
3. **The driver's P5 stages.**
   - Its "setup (SetupVelocityConstraints) 0.0005 ms" is Jolt's non-contact setup only. Jolt builds every contact constraint inside "Add Constraint From Cached Manifold" (`ContactConstraintManager.cpp:860`, `:921`, `:957`).
   - Its "solve velocity" includes the warm start (0.235 ms at W1) and the island preparation (SortContacts + SplitIsland, 0.554 ms).
   - Its W8 per-scope walls are unions that overlap across threads: the stages' union walls sum to 4.49 ms against a 3.04 ms frame. My partition (§5) sums to the frame.
4. **The canary:** agreed, not seen under the two-spread rule and seen under SE alone. That miss is set by pass 0's load, not by K or the canary's size (§2.3).
5. **"No claimed regression at any W" is true, but at W ≥ 2 the min–max bars were 26–73 %** (§2.2). The protocol's claim-free reading at W ≥ 2 is weak this window; the quiet pass alone agrees with it.

## 1. P1: the Jolt headline in one window (trunk 93b2615b against Jolt v5.6.0 918fd2b7)

**Rule** (`plan.md` §4 P1; window 6 `analysis.md:412`, item 15): ours/Jolt per block (A = P1A-jolt, B = P1B-jolt) and pooled (K ≤ 12). **The claim HOLDS iff it is claimed pooled AND in block A AND in block B, in the same direction.** Rows: `H-trk-tree` (the default row: `--cfg default --broadphase tree`, reuse off on this trunk), `H-trk-ap` (AllPairs), `H-jolt56` (`-s=Pyramid -q=Discrete -f -t=W -i=500`). Statistic: the process mean over [0,500).

**Our default row (tree) against Jolt, [0,500):**

| W | A | B | pooled | two-block rule | per manifold [100,500): B / pooled |
|---|---|---|---|---|---|
| 1 | 0.5655 n/Y/Y | **0.6202 Y/Y/Y** | 0.5821 n/Y/Y | does NOT hold | 1.3185 vs 1.1188 µs = **1.178×** / 1.100× |
| 2 | 0.6752 Y/Y/Y | 0.6822 Y/Y/Y | 0.6834 n/Y/Y | does NOT hold | 1.282× / 1.279× |
| 4 | 0.7807 n/n/Y | 0.7578 n/Y/Y | 0.7404 n/Y/Y | does NOT hold | 1.404× / 1.385× |
| **8** | **0.8355 n/Y/Y** | **0.8721 Y/Y/Y** | **0.8111 n/n/Y** | **does NOT hold** | 0.4821 vs 0.3011 µs = **1.601×** / 1.479× |
| 16 | 0.8646 n/n/n | 0.9814 n/n/n | 0.9589 n/n/n | does NOT hold | 1.787× / 1.771× |

**AllPairs against Jolt, [0,500):**

| W | A | B | pooled | per manifold: B / pooled |
|---|---|---|---|---|
| 1 | 0.7150 | 0.7984 | 0.7457 | 1.515× / 1.407× |
| 2 | 1.0089 | 0.9886 | 0.9952 | 1.862× / 1.875× |
| 4 | 1.2361 Y/Y/Y | 1.2468 | 1.2135 | 2.318× / 2.272× |
| 8 | 1.4740 Y/Y/Y | 1.5851 Y/Y/Y | 1.4700 n/Y/Y | 2.917× / 2.704× |
| 16 | 1.4661 | 1.7134 Y/Y/Y | 1.6120 | 3.139× / 2.927× |

- The [100,500) sub-window gives the same flags at W8: A 0.7952 n/Y/Y, B 0.8523 Y/Y/Y, pooled 0.7876 n/n/Y.
- **Cells at W8, [0,500):**
  - tree: A 2.4529 [2.3290-2.9626] K=5 (25.83/2.75/5.73); B **2.1878** [2.1850-2.2027] (0.81/0.08/0.15); pooled 2.2027 K=11 (35.30/…);
  - Jolt: A 2.9357 [2.8382-3.2679] (14.64/…); B **2.5085** [2.4711-2.5936] (4.88/1.87/0.93); pooled 2.7159 K=12 (29.34/14.84/3.49).
- **Verdict P1: the W8 headline does NOT HOLD.**
  - Our default row is faster than Jolt per step in every block at every W (point estimates 0.57–0.98).
  - The rule is met by B alone at W8 (−12.8 %, Y/Y/Y): A fails min–max (bar 59.4 %), and the pooled cell fails it too (bar 91.8 %).
  - Per manifold, Jolt is faster at every W: 1.10–1.18× at W1 and 1.48–1.60× at W8.
- **Why A and the pooled cell cannot claim.**
  - Block A slows every row. The pass medians at W8 are: tree 1Ap0 2.407, 1Ap1 2.708 against B 2.187 / 2.190; Jolt 2.905 / 2.981 against 2.478 / 2.528. At W1 Jolt reads 10.527 / **11.968** (the browser pass) against 9.611 / 9.552.
  - The block term B vs A is −5 to −24 % on every row and every W, claimed only for AllPairs at W4 (−5.34 %, Y/Y/Y).
  - A pooled min–max range spans that block shift. At W2 **both blocks claim ours faster (Y/Y/Y), yet the pooled cell does not** (n/Y/Y, bar 44.4 %). AllPairs at W8 is +47 % slower than Jolt, claimed in both blocks, and still unclaimed pooled (bar 62.9 %).
  - As ruled, the rule is decidable only when both blocks are quiet (FOLLOW-UP item 7).
- **Tree against AllPairs, same binary, block B:** −22.3 / −31.0 / −39.2 / −45.0 / −42.7 % at W 1/2/4/8/16, all Y/Y/Y. Pooled: n/Y/Y, for the same block-shift reason.
- **Scaling T(1)/T(W), block B, [0,500) (efficiency T1/(W·TW)):**

  | row | W2 | W4 | W8 | W16 |
  |---|---|---|---|---|
  | tree | 1.520 (0.76) | 2.152 (0.54) | 2.723 (0.34) | 2.521 (0.16) |
  | AllPairs | 1.350 | 1.683 | 1.928 | 1.859 |
  | Jolt | 1.672 (0.84) | 2.629 (0.66) | 3.829 (0.48) | 3.989 (0.25) |

  - Pooled: tree 1.500 / 2.172 / 2.744 / 2.355; Jolt 1.760 / 2.763 / 3.823 / 3.879.
  - Our tree row gets slower from W8 to W16 (2.188 → 2.363 ms in B), while Jolt still gains (2.509 → 2.408).
  - The ceiling is Amdahl's: 1.10 ms of our W1 stage time (18.4 %) and 1.17 ms of our W8 stage time (53.4 %) run in serial stages (§4).
- **Window term (context):** Jolt in block B / window 3 = 0.977 / 0.995 / 1.020 / 0.976 / 1.008 at W 1/2/4/8/16. The quiet block reproduces window 3 at every W, so window 6's ±4–6 % W8 window term did not appear in the quiet block here.

## 2. P2: L9 C4's timed gate G-TW (one binary, 989ca0f0, `--contact-reuse off` against `on`)

**Rules** (`levers/L9-contact-reuse/02-DESIGN-REV1.md`):
- `:405`: "a claimed regression at any W blocks";
- `:409`: the L9b rows (J-A, J-D, R at every W);
- `:410`: the realized-gain rule;
- `:411`: the J [0,100) witness, "not claimed slower";
- `:412`: "the canary must be seen";
- `:413`: the per-contact table;
- `:414`: the poses.

The prediction and the bar are window 6 `analysis.md:129-130`: 0.9897 × 4,515 × (435.56 − [100, 60]) ns = [1.4994, 1.6782] ms, and the bar is 0.6 × 1.4994 = **0.8997 ms**.

**Binary:** `989ca0f0` = the trunk `6c40b3e6` + the C4 flip (11 files, +78/−26). `93b2615b` is the same `6c40b3e6` + D-M0 (ECS kernel only; no `crates/boyko_physics` change), so c4's reuse-off arm runs trk's physics code.

### 2.1 The realized-gain rule and the rows

| row, window | W1 | W2 | W4 | W8 | W16 |
|---|---|---|---|---|---|
| J-A [0,500) | **−9.25 % Y/Y/Y** | −6.57 % n/n/Y | −6.62 % n/n/n | −0.08 % n/n/n | −1.45 % n/n/n |
| J-A [100,500) | **−10.52 % Y/Y/Y** | −7.57 % n/n/Y | −7.85 % n/n/Y | −0.12 % | −1.59 % |
| J-A [0,100) witness | −4.43 % n/Y/Y | −2.68 % | −2.05 % | −0.10 % | −0.87 % |
| J-D [0,500) | −21.22 % n/Y/Y | −15.15 % n/Y/Y | −12.77 % n/n/Y | −4.52 % | −5.17 % |
| J-D [0,100) witness | −9.88 % n/Y/Y | −6.17 % n/Y/n | −7.54 % | −1.70 % | **+2.12 % n/n/n** |
| R [600,1100) | **−27.06 % Y/Y/Y** | −12.57 % n/n/Y | −15.55 % n/n/Y | **+0.62 % n/n/n** | −2.44 % |

- **Realized gain: PASS.**
  - Over J-A, W=1, [100,500): off 18.4127 [18.3856-18.9831] (3.24/2.37/0.82) → on 16.4753 [16.4125-16.7739] (2.19/0.41/0.41).
  - **ΔT(1) = 1.9374 ms (−10.52 %), Y/Y/Y, 2.15× the 0.8997 bar.** The worst pairing, min(off) − max(on), is +1.6117 ms, also above the bar.
  - Against the prediction: 1.292× its low end and 1.154× its high end (window 6: 1.845 ms on C3's switch, `4db26681`).
- **G-TW's blocking rule does not fire:** no on/off ratio is claimed above 1 at any W or window. The only point estimates above 1 are J-D W16 [0,100) +2.12 % (bars 64.8 / 12.9 %) and R W8 +0.62 % (48.1 / 11.4 %). Neither is claimed under r, s or i.
- **The moving-scene witness holds:** J [0,100) is not claimed slower at any W.
- **Poses hold** (§0): the on rows read one hash per row across W 1–16, and every off row reads C0's hash.

### 2.2 G-TW's resolution this window

The bar each on/off cell had to clear, min–max / SE, [0,500) or [600,1100):

| row | W1 | W2 | W4 | W8 | W16 |
|---|---|---|---|---|---|
| J-A | 7.5 / 1.6 % | 26.2 / 5.4 % | 31.5 / 7.3 % | 48.2 / 10.6 % | 58.4 / 13.1 % |
| J-D | 28.1 / 5.3 % | 36.0 / 7.2 % | 50.4 / 12.3 % | 44.9 / 10.2 % | 63.4 / 13.4 % |
| R | 17.9 / 3.6 % | 36.1 / 7.5 % | 54.7 / 12.4 % | 48.1 / 11.4 % | 72.8 / 14.7 % |

- **Under the two-spread rule, G-TW could have caught a regression only above 7.5–73 %.** Under SE alone the threshold is 1.6–14.7 %, and still no ratio above 1 is claimed.
- **The width comes from pass 0 (03:14–04:00, browser active).** The P2 pass medians: J-A off W8 6.486 (p0) against 5.769 (p1), on 6.199 against 5.630; J-D off W8 4.531 against 4.005.
- **A diagnostic, not the protocol cell: pass 1 alone** (04:02–04:20, witness median 0.65 %), K=3 per arm, on against off:
  - J-A: −9.29 / −8.22 / −5.54 / −2.42 / −1.93 % (Y/Y/Y except W8, n/n/n);
  - J-D: −20.54 / −15.67 / −10.12 / −5.95 / −6.26 % (Y/Y/Y except W16, n/Y/Y);
  - R: −26.81 / −16.95 / −13.17 / −7.65 / −3.01 % (W 1/2 Y/Y/Y, W 4/8 n/Y/Y, W16 n/n/n).
  - All faster, with bars mostly under 10 %. J-A at W8, −0.139 ms, is window 6's −0.142 ms again.
- **What that says:**
  - G-TW passes on the letter, and the quiet pass agrees in sign and size at every W.
  - But the window's own claim-free "no regression" at W ≥ 2 rests on bars of 26–73 %. A real 10–20 % regression at W ≥ 2 would also have passed this window's letter.

### 2.3 The canary (`L9-JC`: armed `--cfg a --contact-reuse on --canary-frac 0.05`, against its reference `L9-JA-a-on`, W=1)

- **The reference:** 16.6508 [16.4962-17.1805] (4.11/2.30/0.89).
- **The canary row:** 17.5469 [17.4890-18.1249] (3.62/1.94/0.80).
- **The injection:** 0.8447 ms [0.8248-0.8590], 5.07 % of the reference. Its span (`sys_parity_canary_ns`) reads 0.8450 ms, and every process's span is within 1.0003–1.0007× its own injection, well inside the 5 % requirement.
- **The step rise is +0.8961 ms (106.1 % of the injection) = +5.38 %,** with bars 10.96 / 6.01 / 2.40 %.
- **Claimed n/n/Y: NOT SEEN under the two-spread rule; seen under SE.**
  - The design writes G-TW's rule as "the P0 protocol: median over K processes, receipts, canary, **SE claim rule**" (`:407`). Under that letter the canary is seen.
  - Under this window's stricter protocol it is not. This is the same split window 6 recorded for G9's parent canary, which was accepted "under G9's own SE form" (window 6 `analysis.md` §4).
- **What the miss says about G-TW's resolution:**
  - The two cells' processes: the reference reads p0 17.181 / 16.496 / 17.134 and p1 16.625 / 16.652 / 16.650; the canary row reads p0 17.972 / 17.489 / 18.125 and p1 17.522 / 17.555 / 17.539.
  - The two high pass-0 values in each cell ran at a witness of 2.5–4.1 %, against 0.56–0.75 % in pass 1. **Pass 0's load alone set the 4 % ranges.**
  - Pass 1 alone (diagnostic, K=3): 16.6498 (0.16 %) against 17.5392 (0.18 %), rise +0.8894 ms = +5.34 %, bars 0.48 / 0.24 / 0.18 %, Y/Y/Y. There the 5 % canary is 11× the bar.
- **What would make it visible, at this window's spreads:**
  - SD 1.75 % and 1.57 %, hypot 2.35 %.
  - The expected min–max bar is 2·d2(K)·2.35 %: **7.9 % at K=3, 11.9 % at K=6, 15.3 % at K=12, 18.3 % at K=24.** The range grows with K, so **no K makes a 5 % canary visible under min–max.**
  - The SE bar is 3.40 / 2.40 / 1.70 / 1.20 %, so K=6 already sees it under SE.
  - Under min–max at K=6, a canary must exceed the observed 10.96 % bar: **canary_frac ≥ ~0.12, about 1.8–2.0 ms at W=1.**
  - Or the machine must be as quiet as pass 1, where 5 % is ample.

### 2.4 The armed per-contact table (`:413`; J-A, [100,500), reading A, own counts)

Counts: off 4,519.26 manifolds / 9,559.22 pairs; on 4,467.66 / 9,545.12, with reused 4,457.51, full 46.67 and separated hits 5,040.94 per step.

| stage | W1 off → on ms | on vs off | W1 µs/manifold off → on | W1 ns/pair on | W8 off → on ms | on vs off | W8 µs/manifold off → on |
|---|---|---|---|---|---|---|---|
| wall | 18.4703 → 16.4320 | −11.04 % n/Y/Y | 4.0870 → 3.6780 | 1,721.5 | 5.8149 → 5.6127 | −3.48 % n/n/n | 1.2867 → 1.2563 |
| bp | 2.0838 → 2.0781 | −0.27 % | 0.4611 → 0.4651 | 217.7 | 1.9614 → 1.9644 | +0.15 % | 0.4340 → 0.4397 |
| **np** | **2.6112 → 0.7511** | **−71.23 % Y/Y/Y** | **0.5778 → 0.1681** | **78.7** | **0.4526 → 0.1591** | −64.84 % n/Y/Y | 0.1002 → 0.0356 |
| setup (solve_build) | 0.3686 → 0.3211 | −12.90 % | 0.0816 → 0.0719 | 33.6 | 0.3469 → 0.3049 | −12.10 % | 0.0768 → 0.0682 |
| warm_apply | 0.5364 → 0.5263 | −1.89 % | 0.1187 → 0.1178 | 55.1 | 0.5430 → 0.5286 | −2.65 % | 0.1201 → 0.1183 |
| colours wide | 12.4500 → 12.2095 | −1.93 % n/n/Y | 2.7549 → 2.7329 | 1,279.1 | 2.1630 → 2.1471 | −0.73 % | 0.4786 → 0.4806 |
| colours narrow | 0.0277 → 0.1788 | **+546 % Y/Y/Y** | 0.0061 → 0.0400 | 18.7 | 0.0299 → 0.1925 | **+543 % Y/Y/Y** | 0.0066 → 0.0431 |
| graph | 0.1426 → 0.1195 | −16.20 % | 0.0316 → 0.0268 | 12.5 | 0.1200 → 0.1158 | −3.54 % | 0.0266 → 0.0259 |

- **Δt_np(1) = −1.860 ms, claimed.** np-on costs 0.168 µs per manifold and 78.7 ns per pair; window 6 read 0.153 µs and 71.7 ns, and the budget is 0.14–0.39 µs and 41–139 ns (`:328`).
- **Narrow colours rise ×6.5 with reuse on** (+0.151 / +0.163 ms at W 1/8, claimed). This is a real side effect, far smaller than the np gain, and it moves no verdict. Recorded for the L9 lane.
- **J-A's colours are the scalar path** (`simd_solve: false` in its SUMMARY), so its solve row is not comparable with Jolt's. §6 uses the default row (P4) for that comparison.
- **Jolt's bracket** (P5, per manifold, block B basis, model U / M from §5):
  - W1: bp 57.3 / 72.2 ns; np with the contact constraint built inside it 301.0 / 263.4; warm 20.4 / 9.8; solve (velocity + position + prep) 733.3 / 765.8;
  - W8: bp 14.3, np 72.7, warm 6.4, solve 199.9.

### 2.5 G-TW's verdict

**PASS.** Each clause:
- the realized gain: 2.15× the bar, claimed;
- no claimed regression at any W, under either rule;
- the witness is not claimed slower;
- the poses hold;
- the per-contact table is supplied;
- the canary is seen under the design's own SE rule, not under the window's two-spread rule.

The last clause, and the 26–73 % min–max bars at W ≥ 2, are the orchestrator's to weigh (FOLLOW-UP item 1).

## 3. P3: G9's last three cells (J-A, W 2/4/16, tip `cbd86a65` against parent `0ca312bd`)

**Rule** (`levers/L11-solve-setup/02-DESIGN-REV1.md:359-371`; the ruling "G9 on C3 CLOSED by ruling", `levers/00-RULINGS.md:217-229` on the tree that records this window): these three cells go to the next quiet window "as a record against the same 'not claimed slower' gate".

| W | parent | tip | tip vs parent |
|---|---|---|---|
| 2 | 11.5423 [11.5165-11.6217] (0.91/0.49/0.19) | 11.5580 [11.5423-11.6426] (0.87/0.57/0.20) | +0.14 %, bars 2.52/1.50/0.56 %, n/n/n |
| 4 | 7.7408 [7.7238-7.8875] (2.12/0.16/0.41) | 7.7424 [7.7278-7.7728] (0.58/0.33/0.12) | +0.02 %, bars 4.39/0.73/0.86 %, n/n/n |
| 16 | 5.6993 [5.6905-5.8470] (2.75/0.28/0.54) | 5.6984 [5.6670-5.9873] (5.62/0.59/1.09) | −0.02 %, bars 12.51/1.30/2.44 %, n/n/n |

- **Not claimed slower at any W.** The block ran 04:22–04:35 with a witness median of 0.61–0.65 %, and the min–max bars are 2.5–12.5 %.
- **Verdict P3: G9 on C3 is CLOSED.** Every row the design lists now has a cell: J-A and J-As at W 1/2/4/8/16, R and R-S at W 1/8, S16 (windows 5, 6 and 7). None is claimed slower.
- The ruling of 2026-09-24 (K = 6 under the window protocol supersedes the letter "K=12") stands. This record adds nothing to reopen.
- J-A is flat here (±0.14 %), where J-As moved −3.7 to −5.3 % at W 2/4/16 in window 6. That is consistent with C3's change being the SIMD warm apply, which `cfg a` (scalar colours) does not take. Untested.

## 4. P4: our per-stage spans (trunk `93b2615b`, default row: `--cfg default --broadphase tree`, reuse off; armed; [100,500))

Reading A = the per-process mean over steps; reading B = the per-process median; per cell the median over K=6. B's per-cell s is at most 2.5 % on every span above 0.01 ms.

| stage (spans) | W1 A ms | W1 ns/manifold | W8 A ms | W8 ns/manifold | W1/W8 |
|---|---|---|---|---|---|
| wall | 6.0239 (B 5.9920) | 1,332.9 | 2.2187 (B 2.1942) | 490.9 | 2.72× |
| bp (`sys_physics_broadphase`) | 0.2656 | 58.8 | 0.2684 | 59.4 | 0.99× |
| – query / assemble / build / verify | 0.2084 / 0.0289 / 0.0219 / 0.0064 | 46.1 / 6.4 / 4.8 / 1.4 | 0.2088 / 0.0309 / 0.0221 / 0.0064 | 46.2 / 6.8 / 4.9 / 1.4 | |
| np (`sys_physics_narrowphase`) | 2.4274 | 537.1 | 0.3823 | 84.6 | 6.35× |
| graph (`sys_physics_build_graph`) | 0.1153 | 25.5 | 0.1166 | 25.8 | 0.99× |
| setup (`phys_solve_build`) | 0.2841 | 62.9 | 0.3122 | 69.1 | 0.91× |
| warm_apply | 0.2990 | 66.2 | 0.3080 | 68.2 | 0.97× |
| colours wide (biased + relax) | 2.4340 (0.8217 + 1.6246) | 538.6 | 0.6193 (0.2217 + 0.4125) | 137.0 | 3.93× |
| colours narrow + restitution | 0.0126 | 2.8 | 0.0144 | 3.2 | |
| integrate + gravity + store + write_back | 0.0978 | 21.6 | 0.1216 | 26.9 | 0.80× |
| gather + apply + select | 0.0389 | 8.6 | 0.0398 | 8.8 | |
| sum of the stage spans | 5.9747 | 1,322.1 | 2.1827 | 483.0 | |

- **The serial stages** (bp, graph, setup, warm_apply, the integrate group, gather/apply) sum to **1.101 ms at W1 (18.4 %) and 1.167 ms at W8 (53.4 %)**. None of them shrinks from W1 to W8 (0.80–0.99×).
- The two parallel stages, np and the wide colours, speed up 6.35× and 3.93×.
- Our W8 step is therefore half serial, which is where the scaling in §1 stops (Amdahl bound T(1)/T(∞) ≤ 5.97 / 1.10 = 5.4 at W1's composition).

## 5. P5: Jolt v5.6.0's per-stage profile (the profiled Distribution build `aa23db26`, frames 100/200/300/400, K=6)

**How it is read:**
- My parser walks each thread's samples into a scope tree. Every sample's exclusive time goes to the stage of its nearest job ancestor, with these re-attributions:
  - `FindCollidingPairs` and `NotifyBodiesAABBChanged` go to bp;
  - "Add Constraint From Cached Manifold" goes to np_cached;
  - the collide scopes go to np_collide;
  - `WarmStart*` goes to warm;
  - `SortContacts` and `SplitIsland` go to solve_prep;
  - `Check Sleeping` goes to integrate;
  - the rest of `FindCollisions` is np_rest.
- cpu = the exclusive time summed over threads.
- The **wall partition** splits each instant of the frame among the stages open on any thread, in proportion to their thread counts, so it sums to the frame. Instants with no thread in any stage are "sched".
- Per process, the mean over the four frames; per cell, the median over K.

**Profiler overhead:**
- The Update scope reads 12.9235 ms at W1 and 3.0352 at W8. It equals the harness's own "Time" at those frames (12.9229 / 3.0355).
- Against block B's unprofiled Jolt [100,500) (9.4978 / 2.5562) that is **+36.1 % at W1 and +18.7 % at W8**. So only the shares below are data, never the times.

| stage | W1 ms (share) | W8 partition ms (share) | W8 cpu ms, all threads | calls per frame |
|---|---|---|---|---|
| bp (prepare + FindCollidingPairs + finalize + AABB notify) | 0.660 (5.1 %) | 0.142 (4.7 %) | 0.829 | 237 samples |
| np_rest (the pair loop: `ProcessBodyPair`, cache lookups) | 2.288 (17.7 %) | 0.370 (12.2 %) | 2.953 | — |
| np_cached (copy the manifold + build its contact constraint) | 1.181 (9.1 %) | 0.352 (11.6 %) | 2.813 | **8,488** |
| np_collide (`sCollideConvexVsConvex`) | 0.001 | 0.000 | 0.002 | **1** |
| islands | 0.017 (0.1 %) | 0.018 (0.6 %) | 0.023 | |
| setup (`SetupVelocityConstraints`, non-contact only) | 0.0005 | 0.0000 | 0.0002 | |
| warm start | 0.235 (1.8 %) | 0.063 (2.1 %) | 0.506 | 1,078 samples |
| solve_prep (SortContacts + SplitIsland) | 0.554 (4.3 %) | 0.073 (2.4 %) | 0.580 | 2 |
| solve velocity | 7.000 (54.2 %) | 1.749 (57.6 %) | 13.990 | 10,784 samples |
| solve position | 0.899 (7.0 %) | 0.164 (5.4 %) | 1.280 | 2,159 samples |
| integrate (gravity, integrate, bounds, sleep check) | 0.046 (0.4 %) | 0.043 (1.4 %) | 0.090 | |
| other + sched | 0.017 (0.1 %) | 0.017 (0.6 %) | 0.162 | |

**The finding:**
- **Jolt's step is about 65 % solve** (velocity 54–58 %, position 5–7 %, preparation 2–4 %, warm start 2 %).
- **About 27 % is its one FindCollisions job:** the broadphase pairs are 4–5 %, and the narrowphase 24–27 %.
- **Its narrowphase runs on the body-pair cache** (`PhysicsSystem.cpp:1080-1082`, `mUseBodyPairContactCache`, on in this build: `no_pair_cache=0`). 8,488 of its 8,489 manifolds per frame come through "Add Constraint From Cached Manifold", and the SAT (`sCollideConvexVsConvex`) runs once per frame.
- **That cached path also builds the contact constraint** (`ContactConstraintManager.cpp:921` `CreateConstraint`, `:957` the non-penetration setup). So Jolt has no separate contact-setup stage: `SetupVelocityConstraints` is non-contact only and costs 0.5 µs.
- **Jolt's serial work overlaps its parallel work.** At W8 its only single-threaded stage, the island preparation (0.580 ms of one thread's time), gets 0.073 ms of the wall partition because other threads are solving at the same moment. Its "sched" is 0.47 % of the frame.
- From W1 to W8 every Jolt stage scales 3.7–4.8× except islands and integrate, which are tiny.
- The driver's "np = FindCollisions − FindCollidingPairs" (3.4374 ms at W1) agrees with my np groups (3.4692) to the non-additivity of per-cell medians.

## 6. Stage by stage, per manifold: where the remaining gap lives, and what C4 removes

**How the table is built:**
- Ours is P4's reading A over 4,519.2575 manifolds. Jolt is P5's wall partition over 8,489.0 manifolds, scaled to block B's unprofiled [100,500) step.
- **Model U** scales every stage by the same factor (0.7365 at W1, 0.8546 at W8).
- **Model M** (W1) instead charges a constant per recorded sample, set at the largest value that leaves every stage non-negative: c = 139.1 ns. That covers 3.18 of the 3.40 ms overhead; the rest is uniform. A pure per-sample model (c = 149.9 ns, all of the overhead) drives the cached-manifold stage negative, so the overhead is not per-sample alone. U and M bracket the plausible attributions.
- **Ours with reuse on is arithmetic:** P4's np × the same-W armed on/off ratio of §2.4 (0.2877 at W1, 0.3516 at W8), over 4,467.66 manifolds.
- **Stage boundaries differ.** Jolt builds its contact constraints inside np, so compare "np + setup + warm" as one line.

**W = 1** (the step per manifold, block B: ours 1,318.5 ns, Jolt 1,118.8 ns = 1.178×; pooled 1.100×):

| stage | ours ns/manifold (off) | ours, reuse on (arith.) | Jolt U | Jolt M | ours − Jolt U, off | on |
|---|---|---|---|---|---|---|
| bp | 58.8 | 59.4 | 57.3 | 72.2 | +1.5 | +2.2 |
| np | 537.1 | 156.3 | 301.0 | 263.4 | **+236.2** | **−144.7** |
| setup | 62.9 | 63.6 | 0.0 (inside np) | 0.0 | +62.8 | +63.6 |
| warm | 66.2 | 66.9 | 20.4 | 9.8 | +45.8 | +46.5 |
| **np + setup + warm** | **666.2** | **286.8** | **321.4** | **273.2** | **+344.8** | **−34.6** |
| solve (velocity + position + prep) | 541.4 | 547.6 | 733.3 | 765.8 | −191.9 | −185.7 |
| islands / graph | 25.5 | 25.8 | 1.5 | 1.6 | +24.0 | +24.3 |
| integrate group | 21.6 | 21.9 | 4.0 | 5.1 | +17.7 | +17.9 |
| other | 8.6 | 8.7 | 1.4 | 0.9 | +7.2 | +7.3 |
| **sum** | **1,322.1** | **950.3** | **1,118.8** | **1,118.8** | **+203.2** | **−168.5** |

**W = 8** (block B: ours 482.1 ns, Jolt 301.1 ns = 1.601×; pooled 1.479×):

| stage | ours ns/manifold (off) | ours, reuse on (arith.) | Jolt U | ours − Jolt, off | on |
|---|---|---|---|---|---|
| bp | 59.4 | 60.1 | 14.3 | **+45.1** | **+45.8** |
| np | 84.6 | 30.1 | 72.7 | +11.9 | −42.6 |
| setup | 69.1 | 69.9 | 0.0 (inside np) | **+69.1** | **+69.9** |
| warm | 68.2 | 68.9 | 6.4 | **+61.8** | **+62.6** |
| np + setup + warm | 221.8 | 168.9 | 79.1 | +142.7 | +89.8 |
| solve | 140.2 | 141.9 | 199.9 | −59.7 | −58.0 |
| islands / graph | 25.8 | 26.1 | 1.8 | **+24.0** | **+24.3** |
| integrate group | 26.9 | 27.2 | 4.3 | **+22.6** | **+22.9** |
| other | 8.8 | 8.9 | 1.7 | +7.1 | +7.2 |
| **sum** | **483.0** | **433.1** | **301.1** | **+181.9** | **+132.0** |

**Where the gap lives at W1.**
- **It is all narrowphase.** np is +236 ns per manifold, or +345 ns counting np + setup + warm against Jolt's cached path that includes its setup.
- bp is at parity: 58.8 against 57.3–72.2 ns.
- **Our solve is 186–224 ns per manifold cheaper than Jolt's.** Our 4 substeps × (biased + 2 relax) cost less per manifold per step than Jolt's 10 velocity + 2 position iterations.
- The small serial stages add +49 ns: graph +24, integrate +18, other +7.

**What L9 C4 (reuse on) removes at W1:**
- It removes np's −1.860 ms (armed J-A, claimed): **−380 ns per manifold, 537 → 156.** np + setup + warm drops to 287 ns, inside Jolt's 273–321 bracket.
- The W1 step per manifold goes from **1.18× Jolt to about 0.85×** (950 against 1,119 ns; arithmetic).
- Measured on the default AllPairs row of the same binary: J-D at W1 [100,500) reads −1.878 ms (−24.1 %, n/Y/Y); pass 1 alone reads −1.592 ms over [0,500), Y/Y/Y.

**Where the gap lives at W8:**
- It is **our five stages that do not parallelize**: bp +45, setup +69, warm +62, graph +24, integrate group +23. That is **+223 ns per manifold, or +233 with "other".**
- These are 1.167 ms of our 2.183 ms step, and each is flat from W1 to W8 (§4).
- Jolt's equivalents either run inside its parallel jobs (its broadphase pair-finding and its contact setup are in FindCollisions, and its warm start is in the parallel solve) or overlap the parallel work (island preparation).
- Our parallel stages are already at or below Jolt: np +12 (−43 with reuse), solve −60.

**What C4 removes at W8:**
- It removes np's −0.29 ms (armed, n/Y/Y), −54 ns per manifold. The W8 ratio goes **1.60× → about 1.44×** (433 against 301 ns; arithmetic).
- Measured: J-D at W8 reads −0.227 ms (n/n/n); pass 1 alone −0.238 ms (Y/Y/Y).
- **After C4 the whole remaining W8 gap is the serial stages.** If they scaled like Jolt's, about 225 ns per manifold would come off: 433 → about 208 ns, 0.69× Jolt (arithmetic ceiling, not a prediction).

## 7. What this window cannot claim

1. **The P1 headline under its own rule.** Block A ran loaded (P1A-p0 with an agent session, P1A-p1 with the browser), so the two-block rule is undecidable at every W. Only block B's claims stand as readings.
2. **G-TW's "no regression" at W ≥ 2 as strong evidence.** The min–max bars are 26–73 %. The quiet pass 1 agrees in sign and size (diagnostic, K=3), but it is not the protocol's cell.
3. **The canary under the two-spread rule** (§2.3).
4. **Any per-stage Jolt time.** Only shares are data; the U/M bracket is a model. Every "ours, reuse on" number is arithmetic across two configurations (the J-A armed ratio applied to the default tree row).
5. **The Tree default with C4 on the trunk.** No binary had both, so the post-C4 W1 ≈ 0.85× and W8 ≈ 1.44× per-manifold figures are arithmetic.
6. **Why Jolt's manifold count doubles** during the collapse (4,495 → 8,489 while ours stays at 4,519).

## FOLLOW-UP WORK ITEMS

**P2: G-TW PASS, so L9 C4 can merge**
1. **u/phys-l9-c4 (`989ca0f0`) → integ/unified.**
   - The pins it moves are window 6 `analysis.md` §2.3's list: J500 → `0x30c5438bc6ad9ffa`, R1100 → `0xc8bbe34cf6a8afc6`, `PINNED_FINAL_HASH`, `A7_R1_D_MAX_BITS`, A7-R1/R2, G2/G7/G8, H8's count (4,519.26 → 4,467.66 on J), the box-pile goldens, the tree lane's J-pose pins and u/phys-l10's lane-base fixtures.
   - **Orchestrator's call on the canary:** accept "seen under SE" as the design's own rule (`:407`) and window 6's G9 precedent, or re-read the canary alone in a quiet window: `L9-JA-a-on` + `L9-JC` at W1, K=6, about 3 minutes. Pass 1's 0.16 % ranges say it would be seen.
   - Optional: re-read J-A, J-D and R off/on at W 2/4/8/16 in a quiet window (about 25 minutes) if the orchestrator wants the W ≥ 2 "no regression" at bars under 10 %.
   - **Narrow colours ×6.5 with reuse on** (+0.15 ms): recorded for the L9 lane. The rise is 12× smaller than np's gain, and it is unexplained.

**The W8 gap: the serial stages (the whole post-C4 deficit, +223 ns per manifold)**
2. **bp query, 0.209 ms serial at W8** (46 ns per manifold; Jolt's whole bp is 14). It belongs to the tree lane's **C2** (the parallel query; D6 fired in both of window 6's blocks, `broadphase/04-DESIGN-REV2.md:204-214`), after **F3** as ruled (window 6 FOLLOW-UP 7 and 9).
3. **setup (`solve_build`), 0.312 ms serial at W8 = 14.1 % of T(8)** (the armed default tree row's 2.219 ms wall). **L11 C4** (D10, the parallel fill) was left conditional on its 5 % trigger (`00-RULINGS.md`, G9 on C3, window 5: 7.45 % on J-As, the build decision not taken). It reads 2.8× the trigger here, so I recommend building it.
4. **warm_apply, 0.308 ms serial at W8 = 13.9 % of T(8).** L7 was retired on a dispatch price (4 × 9.11 waves × 6.54 µs ≈ 0.24 ms). A warm apply folded into the first colour pass pays no extra dispatch; that needs a new design (L11's follow-up or L8's block layout) and its own measurement.
5. **graph, 0.117 ms serial** (25.8 ns per manifold against Jolt's 1.8). No lane owns an awake-scene parallel or incremental graph build (L10 removes it only on frozen islands): a new lever candidate.
6. **The integrate group, 0.122 ms at W8, grows from W1** (0.80×). A parallel per-row integrate, or folding it into the last colour dispatch: a small new lever.

**P1: the headline, re-taken**
7. **After C4 merges (and the tree default flip is decided): P1 again, both blocks in a quiet window.**
   - **Protocol question (orchestrator):** a pooled min–max range spans the between-block shift. At W2 both blocks claim Y/Y/Y and the pooled cell does not; AllPairs +47 % at W8 is unclaimed pooled.
   - Options: keep the rule and require two quiet blocks, or read the pooled cell under SE only, with both blocks claiming under both spreads.
8. **Gate the during-process witness** (orchestrator): P1A-p1 and P2-p0 passed the idle rule and most receipts with a browser active (witness medians 2.28 % / 1.90 % against about 0.63 % when quiet). For example, re-run a process whose `others_busy_pct` exceeds 2 %, or void a pass whose witness median exceeds 1 %.

**Jolt comparison hygiene**
9. **Jolt's manifold count** (4,495 at frame 0 = ours, 8,489 from frame 80, ours 4,519): the per-manifold and per-point ratios agree within 2.7 %, so the normalization holds, but the divergence is unexplained. Put it to the P0 harness owner.
10. **A lighter Jolt profile.** Model M's c = 139 ns per sample, and 8,488 + 10,784 samples per frame sit in two scopes. A build with `JPH_PROFILE` removed from those two scopes (patch in the build directory only) would cut the 36 % W1 overhead and turn the shares into times.

**P3: G9 CLOSED.** No code follow-up (C3 moves no pin).

## 8. Open points and files

1. The canary and G-TW's W ≥ 2 resolution: the design's SE letter says seen and passed; the window's two-spread rule says not seen, and it cannot resolve under 26 %. Orchestrator's call (item 1).
2. The P1 rule as ruled cannot decide with one loaded block (item 7).
3. `plan.md`'s hard stop (04:15) was replaced at launch by `--cutoff 10:00` / `10:30` (`raw/shell_log.txt`). The window ended at 05:03:57 with nothing cut.
4. The 76 GB freed on D: between 01:29:50 and 01:30:50 fell between passes; no timed process was running.

Files are in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7/`:
- `analysis.md`: not written there, because the harness blocks report files for this role; this text is its content;
- `tools/analyze7.py`;
- `analyst/tables.txt` (identical to `analyst_run.log`);
- `analyst/reduction.json`;
- `analyst_run.log`.
