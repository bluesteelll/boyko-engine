Q1 0.87x Jolt at W8 on the default row, reproduced in three clean blocks (B 0.872 / C 0.870 / D 0.870), NOT claimed under the window protocol (min-max fails in A, C, D and pooled; in C and D one process each sets the failing range); Q2 C2 NOT BUILT (the default row armed: t_q 0.208 ms, claimed below 0.235 in C, D and pooled; the cfg-A row's 0.247 is on the bar, context); Q3 TREE_BRUTE_MAX_ROWS = 144, AUTO_TREE_LO / HI = 144 / 152 by the recipe's rule, both families (L2's procedure beside it: 150 / 135), a code change for the tree lane, not made here; Q4 G-TW's resolution DEMONSTRATED: every rung seen, the smallest 0.5 R = 3.77 % of the step, so the wave-1 canary under SE stands

# Window 7 wave 2: the W8 headline over four blocks, C3b's t_q and C2, the tree thresholds between 64 and 256, and G-TW's canary resolution, recomputed from `raw/`

results-analyst, 2026-09-25.

**Inputs:**
- Wave 2: `raw/runs.jsonl` (259 records), every process's `stdout.txt`, `stderr.txt`, `run.csv`, Jolt
  `per_frame_discrete_thW.csv` and criterion `criterion/<id>/new/sample.json`, `rows7b.json`, `plan.md`,
  `bin/SHA256SUMS`, `bin/COMMIT.txt`, `gate/`, `variant/g4ref.diff`, `wait_log.txt`, `progress.txt`,
  `raw/window_state.json`, `raw/shell_log.txt`, `WINDOW_DONE` (`exit 0` / `complete`).
- Window 7 (wave 1), for Q1's blocks A and B: `../raw/runs.jsonl` and the process files of `P1A-jolt` and
  `P1B-jolt`, `../rows7.json`, `../bin/SHA256SUMS`, `../gate/fixtures/`, and `../gate/jolt_receipt/` (Jolt's
  manifold count).
- Context only, never pooled:
  - window 3's Jolt v5.6.0 cells (2.5693 ms at W8);
  - window 6's C3b cells (`docs/measurements/2026-09-24-physics-window6/analysis.md:214`, `:229-237`: parent t_q
    0.4044, tip 0.2102 in P3, tip 0.2354 in P5g);
  - window 4's G4 grid (the queue's section 11: `TREE_BRUTE_MAX_ROWS` 64, `AUTO_TREE_LO/HI` 64 / 256, L2 126 / 140,
    log-log crossovers 111 uniform and 138 disparity).
- The rules' sources: `plan.md` §4. The C3b design and the G4/G5 recipe are the orchestrator's scratch notes
  (`c3b/design.md`, `treebp/g4_g5_recipe.md`; see `README.md`).

**Method:**
- My script, `tools/analyze7b.py`, re-reads every process statistic from the files the process left:
  - the window means and the per-step span and count columns from `run.csv`;
  - Jolt's frame means from its per-frame CSV;
  - criterion's estimates from `sample.json`. The bench runs in Flat mode, so the estimate is the mean of
    time/iterations over the 10 samples.

  It re-derives validity from the SUMMARY line, the Jolt stat line and banner, criterion's id set and the bench's
  kernel receipts. It re-derives cleanliness from the receipts. It never imports `tools/reduce7b.py` and never uses
  the driver's per-process statistics or flags; it compares against them afterwards.
- **A cell** is the median over K processes. r = min-max, i = IQR, s = 1.2533·SD/√K, each relative to the median.
- **The claim rule:** B against A is claimed iff |B/A − 1| > 2·hypot(sA, sB) under BOTH r and s; i is printed. There
  is no claim when either side has K < 3. Against a constant bar b, a cell is claimed iff |m/b − 1| > 2r AND > 2s.
- **The slot rule (windows 6/7):** per (block, pass, round, row, binary, W), the original if valid and clean, else its
  re-run if valid and clean, else the slot is dropped. Clean = both 5-s receipts ≤ 5.0 % and no build process during
  the process.
- Flags are written r/i/s (Y = claimed under that spread).
- Per manifold uses each side's own count over [100,500): ours 4,519.2575 per step (the `manifolds` column of every
  used process's `run.csv`, the same in every one), Jolt 8,489.0 per frame (re-read from window 7's `-receipt` CSVs
  at W1 and W8; the count does not depend on the thread count, so W4 and W16 use it).
- git was read-only until the record step.

## 0. Selection, input checks, the run, and the driver's reduction

**Selection:**
- **Wave 2:** 259 records = 248 processes (10 warm-ups, 237 originals, 1 re-run) + 11 pass markers. There are 11
  passes done and 0 voided.
  - **All 237 slots are used, 1 by its re-run; 0 are dropped.** Every cell has K = 6 (Q1, Q2, Q4) or K = 3 (Q3).
  - The re-run: `Q1D-p0 #022 r2 H-trk-tree W8`. The original's after-receipt read 16.88 % (`RuntimeBroker.exe`
    4.16 CPU-s). The re-run's receipts read 1.02 / 1.07 %.
- **Window 7, blocks P1A-jolt and P1B-jolt:** 218 processes (4 warm-ups, 180 originals, 34 re-runs). 165 of 180 slots
  are used (19 by their re-run), and 15 are dropped, all in the loaded block A. So A's cells are K = 3–6, the same
  as wave 1 read them.

**Input checks (all hold):**
- **0 validity problems in 238 wave-2 and 214 window-7 non-warm-up processes.**
  - Every runner process has exit 0, the exe sha256 of `bin/SHA256SUMS`, and mask `0xffff`. Its `run.csv` has the
    row's step count, and the SUMMARY line reads void 0, `expect_pose: match` and the fixture's hash:
    - J500 `0x32d5e235342b4143` on trk, par and tip;
    - JAon500c4 `0x30c5438bc6ad9ffa` on c4.

    It also reads `workers` = W, msvc, the armed flag its row expects, drops 0 on armed rows and no disarmed ring
    traffic, and the row's broadphase. TreeDiag reads `static_rebuilds 1, members 1, evictions 0` on tree rows and
    all zero on AllPairs rows. Contact reuse is on only where the row asks for it. `canary_ns` is present on the
    four canary rows and on no other. Finally, the SUMMARY's `window_mean_ns` equals the `run.csv` mean.
  - Every Jolt process: exit 0, one stat line with threads = W and hash `0xb8522b4e3fc62cfe`, the banner
    `boyko-parity-patch v1 … allow_sleep=0, receipt=0`, and 500 frames.
  - Every criterion process has exit 0 and prints exactly the 54 ids the filter expands to, each once. Each has a
    Flat 10-sample `sample.json`. **The printed estimate equals my recomputed mean to 4.4e-5 relative at worst.**
    `stderr.txt` holds the bench's kernel receipts, LeafList and RowWalk with equal pair counts, for every timed size
    of both families. That is the bench's own oracle: the tree's steady-state pair set equals AllPairs'.
- **Launch-context lengths:** every used Q1 and Q2 process hits its row's pinned lengths: 0 misses.
  - Q1's cwd: 163/161/161 at W 4/8, 164/162/162 at W16.
  - Q2's cwd: 163 / 166; its command lines: 648 / 654 characters.
- **Against the driver's per-process fields:** every valid flag, clean flag, `mean_ms`, `cols` value (mean and median
  of every span and count column over every metric window) and criterion estimate reproduces.
- **Receipts (260 distinct, warm-ups and pass openings included):** median 1.12 %, p90 3.09 %, max 16.88 %; 1 over
  5 % (the re-run above). No build or lane process at any receipt or during any process.

**The run** (`raw/shell_log.txt`, `wait_log.txt`, `progress.txt`, `raw/window_state.json`):
- **Launched at 06:07:48, once, with no flags** (`run_window.sh start:` with an empty argument list), under the
  11:45 cutoff. `WINDOW_DONE` reads `exit 0` / `complete` at 07:44:36. Nothing was cut or skipped for priority.
- **11 waits, 35 polls.** The first reached idle after 5 polls: polls 1–2 read 9.28 / 7.29 % with a `python`
  process at 10 CPU-s per 10 s. Every other wait reached idle in the minimum 3 polls.
  - No poll saw a build process or a process under `D:/wt/_targets` or `D:/wt/mq-*`. The process count was 275–286,
    and D: stayed at 122.6 GB free.
- The power scheme read High performance. `binaries_after` reads "all match", and 0 processes were voided.
- **The pass times:**

  | block | pass 0 | pass 1 |
  |---|---|---|
  | Q1C | 06:12–06:15 | 06:17–06:20 |
  | Q2C | 06:23–06:27 | 06:30–06:34 |
  | Q3 | 06:37–07:07 | — |
  | Q4 | 07:09–07:13 | 07:15–07:19 |
  | Q2D | 07:22–07:26 | 07:28–07:33 |
  | Q1D | 07:35–07:39 | 07:41–07:44 |
- **The during-process witness** (`others_busy_pct`; recorded, not gated by the protocol) over the used set. Every
  wave-2 pass reads a median of 0.59–0.69 %, and every wave-2 pass except Q1D-p1 reads a maximum ≤ 2.02 %.
  - **Q1D-p1's maximum is 14.25 %, on one process** (`#024 H-trk-tree W8`, §1.3). It is the highest witness of any
    used process in wave 2 and in window 7's blocks A and B.
  - Window 7's blocks, for comparison: P1A-p0 0.82 %, **P1A-p1 2.28 %** (the browser pass), P1B-p0 and P1B-p1 0.63 %.

**Against the driver's `raw/tables.md` (`raw/reduction.json`, `tools/reduce7b.py`):**
- **Arithmetic: 0 disagreements.** 103 cells (K, median, min, max) and 150 comparisons (the ratio and all three claim
  flags) were compared by key.
  - The 36 Q3 comparisons agree in K, in both medians and in the ratio to 6.2e-5 relative. That is the print
    precision of the criterion estimates the driver read; I recompute the mean at full precision. They also agree
    in every claim flag.
  - The five decisions agree: the Q1 W8 rule, C2 on the default row, C2 on the cfg-A row, the Q3 constants and the
    Q4 resolution.
- **My disagreements are in reading, not arithmetic:**
  1. **Q1: the driver states the rule's result but not what fails it.** "does NOT hold (A n, B Y, C n, D n,
     pooled A-D n)" hides two things.
     - C and D fail min-max only because of one process each. D also fails SE, again because of that one process.
     - Every block claims under IQR, and every block but D claims under SE. §1.3 names each range-setting process.
  2. **Q1: the driver prints each block term with one flag**, the combined claim, e.g. "B/A 0.8545 (n)". The
     per-spread flags show block A's shift: at W8, Jolt B/A and C/A read n/Y/Y (−14.6 % / −14.0 %), and ours n/Y/n.
     Between the clean blocks nothing moves: D/C 1.0009 (ours), 1.0010 (Jolt), 1.0007 (AllPairs), all n/n/n.
  3. **Q1: the driver quotes per manifold pooled A-D only** (1.586× at W8). That cell carries block A's loaded Jolt
     (A alone reads 1.494×). The three clean blocks read **1.601× / 1.596× / 1.594×** (B / C / D). The difference is
     small this time, because B, C and D are 18 of the 24 pooled Jolt processes.
  4. **Q1: the driver's Jolt window term uses the pooled cell** (0.9858× window 3 at W8). The clean blocks read
     0.976 / 0.983 / 0.984 and A 1.143.
  5. **Q2: the driver attributes the window-6 shift to "neither layout nor block" and stops there.** Window 6's own
     parent cell answers more (§2.3): the same parent binary on the same row reads 0.4938 ms here against 0.4044 in
     window 6's P3 (+22.1 %). So the cfg-A query moved between the windows on BOTH kernels, while the kernel ratio
     held (0.500 against 0.520).
  6. **Q3: the driver sets the LeafList ratios beside window 4's pre-F1 ratios** ("[window 4, before F1: 1.137]").
     That comparison spans two binaries and two kernels. Its own bridge lines show the same kernel (RowWalk) reading
     11–18 % lower all_pairs/tree than in window 4, so most of the crossovers' rise over window 4 is not F1 (§3.3).

## 1. Q1: the W8 headline over four separated blocks (trunk `93b2615b` against Jolt v5.6.0 `918fd2b7`)

**Rule** (`plan.md` §4 Q1): ours/Jolt **HOLDS iff it is claimed in block A, in B, in C, in D AND pooled over A-D (K up
to 24), all in the same direction.** Blocks:
- A = window 7 P1A-jolt, 01:09–03:11;
- B = window 7 P1B-jolt, 04:50–05:04;
- C = Q1C, 06:12–06:20;
- D = Q1D, 07:35–07:44.

Rows (window 7's P1, verbatim):
- `H-trk-tree` = our default row (`--scene jolt --gap 0.5 --cfg default --broadphase tree`, reuse off on this
  trunk);
- `H-trk-ap` = AllPairs;
- `H-jolt56` (`-s=Pyramid -q=Discrete -f -t=W -i=500`).

Statistic: the process mean over [0,500).

> **Orchestrator's ruling Q1 (applied):** "Q1 - the W8 headline: the protocol rule (claimed in every block under
> min-max AND SE, and pooled) is NOT changed after the data; report the reading honestly: medians B 0.872 / C 0.870 /
> D 0.870 (A was taken with a browser and an agent session active, receipt median 2.3 % vs ~0.6 %: state it), the
> IQR/SE claims per block, the min-max failures and the single-process outliers that cause them (name each outlier
> process, its value and its receipts), W16 near parity, and the per-manifold ratios with each side's own counts; the
> headline wording is '0.87x Jolt at W8 on the default row, reproduced in three clean blocks, NOT claimed under the
> window protocol'."

### 1.1 The rule, per W (ours / Jolt, [0,500), flags r/i/s)

**Our default row (tree) against Jolt:**

| W | A | B | C | D | pooled A-D | pooled C+D | rule |
|---|---|---|---|---|---|---|---|
| 4 | 0.7807 n/n/Y | 0.7578 n/Y/Y | **0.7841 Y/Y/Y** | **0.7691 Y/Y/Y** | 0.7660 n/Y/Y | **0.7767 Y/Y/Y** | does NOT hold |
| **8** | **0.8355 n/Y/Y** | **0.8721 Y/Y/Y** | **0.8697 n/Y/Y** | **0.8696 n/Y/n** | **0.8674 n/n/Y** | 0.8697 n/Y/Y | **does NOT hold** |
| 16 | 0.8646 n/n/n | 0.9814 n/n/n | 0.9808 n/n/Y | 0.9773 n/n/n | 0.9771 n/n/n | 0.9808 n/n/n | does NOT hold |

**AllPairs against Jolt:**

| W | A | B | C | D | pooled A-D | pooled C+D |
|---|---|---|---|---|---|---|
| 4 | 1.2361 Y/Y/Y | 1.2468 n/Y/Y | 1.2854 Y/Y/Y | 1.2571 Y/Y/Y | 1.2557 n/Y/Y | 1.2724 Y/Y/Y |
| 8 | 1.4740 Y/Y/Y | 1.5851 Y/Y/Y | 1.5722 Y/Y/Y | 1.5716 Y/Y/Y | 1.5698 n/Y/Y | 1.5727 Y/Y/Y |
| 16 | 1.4661 n/Y/Y | 1.7134 Y/Y/Y | 1.7114 Y/Y/Y | 1.6993 Y/Y/Y | 1.7025 n/Y/Y | 1.7087 Y/Y/Y |

- **The W8 cells, [0,500), ms:**
  - ours: A 2.4529 [2.3290-2.9626] K=5 (25.83/2.75/5.73); B 2.1878 [2.1850-2.2027] (0.81/0.08/0.15); **C 2.1966
    [2.1827-2.1989] (0.74/0.45/0.17); D 2.1986 [2.1792-3.1487] (44.10/1.27/9.05)**; pooled A-D 2.1969 K=23
    (44.13/3.98/3.02);
  - Jolt: A 2.9357 [2.8382-3.2679] (14.64/3.56/2.72); B 2.5085 [2.4711-2.5936] (4.88/1.87/0.93); **C 2.5256
    [2.4736-2.7015] (9.02/1.78/1.67); D 2.5282 [2.4602-2.6055] (5.75/3.72/1.21)**; pooled A-D 2.5328 K=24
    (31.89/9.75/2.17).
- **The [100,500) sub-window gives the same flags at W8:** A 0.7952 n/Y/Y, B 0.8523 Y/Y/Y, C 0.8498 n/Y/Y, D 0.8488
  n/Y/n, pooled A-D 0.8445 n/n/Y. [0,100) pooled A-D is 0.9633, n/n/n.
- **This run alone** (C, D and pooled C+D):
  - at W4 it holds (0.784 / 0.769 / 0.777, Y/Y/Y);
  - at W8 it does not (pooled C+D 0.8697 n/Y/Y: its min–max range holds D's outlier, 44.14 %).
- **The rule at W8 fails even for AllPairs, which is 57 % SLOWER than Jolt.** That row is claimed in all four blocks
  (Y/Y/Y), and pooled A-D does not claim it (bar 68.1 %). The pooled min–max range spans block A's shift, as it did
  in window 7.

### 1.2 The reading, as ruled: 0.87× at W8 in three clean blocks

- **Medians, ours/Jolt at W8: B 0.8721, C 0.8697, D 0.8696**, and pooled A-D 0.8674. The three clean blocks agree
  within 0.3 %.
  - Their W8 cells agree within 0.8 % on every row: ours 2.1878 / 2.1966 / 2.1986, Jolt 2.5085 / 2.5256 / 2.5282,
    AllPairs 3.9764 / 3.9706 / 3.9733.
  - The block terms between them are n/n/n on every row (D/C 1.0009 ours, 1.0010 Jolt; D/B 1.0049 / 1.0078).
- **Block A was taken with an agent session and then the owner's browser active.** The ruling's "receipt median
  2.3 % vs ~0.6 %" is, precisely:
  - **the during-process witness median of A's second pass, P1A-jolt-p1 (02:55–03:11, the browser pass): 2.28 %**,
    against 0.59–0.69 % in every pass of B, C and D. P1A-jolt-p0 (01:09–01:29, the agent session) reads 0.82 %.
  - The 5-s receipts' medians over A's used processes read 1.46 % (p0) and 3.20 % (p1), against 0.99–1.23 % in
    B, C and D.
  - The effect: A's W8 medians sit +12.1 % (ours) and +17.0 % (Jolt) above B's. Jolt slowed more, so A's ratio
    (0.8355) is lower.
- **IQR and SE claims per block, W8 (ours faster):**

  | | A | B | C | D | pooled A-D | pooled C+D |
  |---|---|---|---|---|---|---|
  | min-max | n (bar 59.38 %) | **Y** (9.90) | n (18.11) | n (88.94) | n (108.89) | n (90.32) |
  | IQR | **Y** (8.99) | **Y** (3.74) | **Y** (3.66) | **Y** (7.87) | n (21.06) | **Y** (5.93) |
  | SE | **Y** (12.69) | **Y** (1.88) | **Y** (3.36) | n (18.26) | **Y** (7.44) | **Y** (9.29) |

  The effect is −16.45 % in A and −12.79 / −13.03 / −13.04 % in B / C / D (−13.26 % pooled).
- **Headline, as ruled: "0.87x Jolt at W8 on the default row, reproduced in three clean blocks, NOT claimed under the
  window protocol".**

### 1.3 The min-max failures at W8, and the processes that set them

Every failing bar is 2·hypot(r_ours, r_Jolt). The table names the one process at the far end of each failing range.
Its receipts are the 5-s receipts before and after it, with the top process of each; the witness is the
during-process `others_busy_pct` with its top "other" process.

| block | cell (range) | process | value | vs the block median | receipts before / after | witness | range without it |
|---|---|---|---|---|---|---|---|
| A | ours (25.83 %) | `P1A-jolt-p1 #021 r1 H-trk-tree trk W8`, 03:01:28 | **2.9626 ms** | +20.78 % | 3.60 / 3.95 % (`browser.exe` both) | 4.52 % (`browser.exe`) | 5.97 % |
| A | Jolt (14.64 %) | `P1A-jolt-p1 #005 r0 H-jolt56 j56 W8`, 02:56:07 | **3.2679 ms** | +11.32 % | 4.02 % (`browser.exe`) / 1.58 % | 2.06 % (`browser.exe`) | 4.90 % |
| C | Jolt (9.02 %) | `Q1C-p0 #014 r1 H-jolt56 j56 W8`, 06:13:41 | **2.7015 ms** | +6.97 % | 0.86 / 1.61 % (`Taskmgr.exe`) | 1.07 % (`Taskmgr.exe`) | 2.68 % |
| D | ours (44.10 %) | `Q1D-p1 #024 r2 H-trk-tree trk W8`, 07:44:09 | **3.1487 ms** | +43.22 % | 1.81 / 2.18 % (`Taskmgr.exe`) | **14.25 % (`msedge.exe`, 1.30 CPU-s)** | 1.90 % |

- **Block A** is loaded as a whole, not by one process. Its two range-setters are both in the browser pass, and its
  medians are shifted too (§1.2).
- **Block C fails on one Jolt process.** Nothing in its receipts or witness marks it: both receipts are under 1.7 %
  and the witness is 1.07 %. Its cause is not in the record.
- **Block D fails on one process of ours.**
  - Its 5-s receipts pass the protocol (1.81 / 2.18 %). During the process, `msedge.exe` took 1.30 CPU-s, a witness
    of 14.25 %. The protocol records the witness but does not gate it, which is window 7's FOLLOW-UP item 8.
  - This one value also inflates D's SD, so D fails SE as well (s 9.05 % on our cell).
  - The other five processes of the cell read 2.1792–2.2210.
- **Pooled A-D** fails min-max because its ranges hold D's 3.1487 and A's 3.2679. It fails IQR (21.06 %) because six
  of its 24 Jolt processes are block A's (Jolt i 9.75 %).
- **A diagnostic, not the protocol** (the rule is not changed; ruling Q1): the same four comparisons with each block's
  one range-setting process per side removed.
  - A reads 0.8364 (bars 15.45 / 9.28 / 4.10 %), B 0.8784, C 0.8707 and D 0.8648. Each is claimed under all three
    spreads.
  - This shows only which process sets each bar. It is not a reading of the headline.

### 1.4 W16 near parity, W4, and per manifold

- **W16: near parity, claimed nowhere under min-max.** ours/Jolt reads B 0.9814, C 0.9808, D 0.9773 (−1.9 to −2.3 %),
  pooled A-D 0.9771 (n/n/n). C alone claims under SE (n/n/Y).
  - The cells: ours 2.3628 / 2.3628 / 2.3701, Jolt 2.4075 / 2.4090 / 2.4253 ms.
  - [100,500) reads 0.951 / 0.952 / 0.948, with SE claims only in C and D.
  - Block A's W16 Jolt cell holds a 5.3684 ms process (`P1A-jolt-p1 #002`, witness 8.87 %, `browser.exe`), a range
    of 78.04 %.
  - Our tree row is slower at W16 than at W8 in every clean block (2.363–2.370 against 2.188–2.199 ms). Jolt is
    faster at W16 than at W8 (2.408–2.425 against 2.509–2.528).
- **W4:** ours/Jolt 0.758–0.784 in every block. It is claimed in C and D and in pooled C+D (Y/Y/Y). B fails min-max
  on one Jolt process (3.981 ms; its other five span 3.525–3.686, and the cell's range is 12.48 %). A fails on its
  load.
- **Per manifold, [100,500), each side's own count** (ours 4,519.2575 manifolds per step from every used process's
  `run.csv`; Jolt 8,489.0 per frame from window 7's `-receipt`). Ours ns / Jolt ns = ratio:

  | W | A | B | C | D | pooled A-D | pooled C+D |
  |---|---|---|---|---|---|---|
  | 4 | 669.8 / 461.8 = 1.450× | 610.1 / 434.4 = 1.404× | 611.9 / 419.8 = 1.457× | 614.2 / 430.4 = 1.427× | 1.423× | 1.443× |
  | **8** | 531.1 / 355.5 = 1.494× | **482.1 / 301.1 = 1.601×** | **484.0 / 303.2 = 1.596×** | **485.1 / 304.2 = 1.594×** | 1.586× | 1.596× |
  | 16 | 602.8 / 392.4 = 1.536× | 518.4 / 290.1 = 1.787× | 519.7 / 290.6 = 1.788× | 520.5 / 292.3 = 1.781× | 1.779× | 1.785× |

  - AllPairs at W8: 2.917× / 2.893× / 2.886× (B / C / D).
  - Jolt carries 1.878× our manifolds, so per step our default row is 13 % faster at W8 while Jolt is 1.6× faster
    per manifold. Wave 1's §6 places that per-manifold gap in our narrowphase at W1 and in our five serial stages
    at W8.
- **Window term (context):** Jolt against window 3 at W8 reads B 0.976, C 0.983, D 0.984 (A 1.143). The clean blocks
  reproduce window 3 within 2.4 %, at W4 and W16 as well.

## 2. Q2: C3b's t_q, the window-6 shift, and C2 (parent `6dd1f916` RowWalk against tip `983480a9` LeafList)

t_q = the per-process median over [100,500) of `phys_bp_query_ns`, then the median over K. The tree span is the sum
of the four `phys_bp_*` per-process medians. Blocks C = Q2C (06:23–06:34) and D = Q2D (07:22–07:33), K = 6 each and
12 pooled. Rows:
- `C3b-TA-armed` = window 6's `T-A-tree-armed`, in window 6 P3's launch layout (648 characters);
- `C3b-TA-pad06` = the same process in window 6 P5g's layout (654 characters);
- `C3b-TD-armed` = the default row armed, the row C2's letter names (`c3b/design.md:231`, `:372`).

> **Orchestrator's ruling Q2 (applied):** "C2 is NOT built (the design's letter row: the default row armed, t_q
> 0.208 below 0.235 claimed in C, D and pooled); the cfg-A row's 0.247 'on the bar' is recorded as context."

### 2.1 The cells (ms; r/i/s %)

| row | W | parent t_q (pooled) | tip t_q C / D / pooled | tip vs parent (pooled) | tip span (limit) | tip step |
|---|---|---|---|---|---|---|
| TD-armed (default) | 1 | 0.4030 (0.58/0.18/0.08) | **0.2079 / 0.2078 / 0.2078** (4.06/0.69/0.50) | 0.5157 Y/Y/Y | 0.2635 (0.36) | 6.6266 |
| TA-armed (cfg-A, plain) | 1 | 0.4938 (1.50/0.20/0.15) | **0.2459 / 0.2486 / 0.2471** (6.97/1.85/0.77) | 0.5005 Y/Y/Y | 0.3190 (0.36) | 17.0220 |
| TA-pad06 (cfg-A, padded) | 1 | 0.4938 (2.27/0.67/0.26) | 0.2509 / 0.2464 / 0.2480 (9.55/2.04/0.96) | 0.5023 Y/Y/Y | 0.3200 (0.36) | 16.9872 |
| TA-armed (cfg-A, plain) | 8 | 0.4113 (0.98/0.36/0.12) | 0.2124 / 0.2119 / 0.2122 (0.82/0.42/0.09) | 0.5159 Y/Y/Y | 0.2699 (0.35) | 4.0792 |

- **The kernel ships again:** tip against parent is −48 to −50 % on every row, in each block and pooled, Y/Y/Y. Every
  tip span is under its G5 limit, and every parent span is over it (0.454–0.561).

### 2.2 C2 on the letter's row: NOT BUILT (ruling)

- The default row armed, tip, W=1, against the bar 0.235 ms (`c3b/design.md:372`: "t_q(J, W=1) ≥ 0.235 ms on the
  default row | **C2 is built**"):
  - C 0.2079, −11.53 % (2r 3.18 %, 2s 0.56 %): **claimed below**;
  - D 0.2078, −11.56 % (2r 7.77 %, 2s 1.89 %): **claimed below**;
  - pooled 0.2078, −11.56 % (2r 8.12 %, 2s 1.00 %): **claimed below**.
- **So C2 is NOT BUILT**, by the plan's three-outcome rule and by the ruling.
- The same cell against the design's other bars:
  - > 0.21 ms (attribution A, `:222`): 0.2078, −1.03 %, on the bar (SE only);
  - ≤ 0.186 ms ("into the band", `:226`): +11.74 %, claimed above, so not in the band;
  - c_q = 0.2078 ms / 1,240 rows = **167.6 ns per queried row, > 150 ns** (F3, `:374`; F3 is already taken up).
- **The default row is stable across windows:** window 7's P4 read the trunk's default row (identical
  `broadphase_tree` code, `plan.md` §4 Q2) at 0.2043 [0.2034-0.2051]. Here it reads 0.2078 [0.2070-0.2155], +1.7 %
  (context; different windows are never pooled).

### 2.3 The cfg-A row, 0.247 on the bar (context), and the window-6 shift

- **C2 on window 6's reading** (the cfg-A armed row, plain layout): C 0.2459 (+4.62 %), D 0.2486 (+5.79 %), pooled
  0.2471 (+5.16 %).
  - Each is claimed above 0.235 under SE (2s 1.08 / 2.68 / 1.54 %), and none under min-max (2r 5.21 / 13.07 /
    13.93 %). So each sits **on the bar**, UNRESOLVED by the plan's rule.
  - The padded layout reads the same: 0.2509 / 0.2464 / 0.2480, on the bar.
  - Recorded as context (ruling Q2): the row C2's letter names is the default row.
- **The launch-context layout term is not claimed.** Padded against plain, tip W1:
  - +2.04 % (C), −0.88 % (D), +0.36 % pooled, n/n/n in each, with SE bars 3.38 / 3.02 / 2.46 %;
  - on the parent: −0.08 / +0.23 / +0.00 %.
- **The block term is not claimed.** D against C, tip W1, plain: +1.12 %, n/n/n (SE bar 2.89 %); every other row
  −1.78 to +0.27 %.
- **What that leaves for window 6's +11.96 % (P3 0.2102 → P5g 0.2354).** The layout test had the power to see it:
  its SE bar is 2.5 % against a 12 % shift. So the address-layout candidate of window 6 (`analysis.md:229-237`) is
  **not supported**.
  - Neither of window 6's readings reproduces here: both layouts read 0.246–0.251 in both blocks.
  - **The same parent binary on the same row reads 0.4938 ms here against 0.4044 in window 6's P3 (+22.1 %)**, while
    the tip reads +17.6 % over P3's 0.2102. The kernel ratio holds (0.500 against 0.520).
  - So the cfg-A W=1 query carries a between-window term of 12–22 % on BOTH kernels, not a launch-layout term and not
    a within-run block term. Its cause is untested.
- The default row does not show that term across windows 7 and 7b (+1.7 %, §2.2).

## 3. Q3: the tree thresholds between 64 and 256 (recipe 1.4; instrument `93b2615b` + `G4_SIZES`, K = 3)

The instrument is the trunk's `benches/broadphase.rs` with one line changed: `G4_SIZES` gains the sizes between 17
and 256 (`variant/g4ref.diff`, the recipe's own remedy, `g4_g5_recipe.md:131`). The timed filter holds 54 benchmarks
per process:
- `all_pairs` and the shipped `tree` (LeafList) at 12 sizes (64, 96, 112, 128–176 in steps of 8, 192, 256);
- `tree_rowwalk` (C1's kernel, same binary) at 64 / 128 / 256.

Both families, `uniform` and `disparity`. Criterion's own settings: 3 s warm-up, 5 s, 10 flat samples.

> **Orchestrator's ruling Q3 (applied):** "TREE_BRUTE_MAX_ROWS = 144, AUTO_TREE_LO/HI = 144/152 from the recipe's
> rule (both families; the L2 log-log crossover beside it) - a code change for the tree lane, not made here."

### 3.1 all_pairs / tree per size (ratio > 1: the tree is faster; flags r/i/s)

| n | uniform | disparity |
|---|---|---|
| 64 | 0.3610 Y/Y/Y | 0.4054 Y/Y/Y |
| 96 | 0.5951 Y/Y/Y | 0.5545 Y/Y/Y |
| 112 | 0.7062 Y/Y/Y | 0.6611 Y/Y/Y |
| 128 | 0.9109 Y/Y/Y | 0.9220 Y/Y/Y |
| 136 | 0.9649 n/Y/Y | 0.9381 Y/Y/Y |
| **144** | **1.0076 n/n/n** | **0.9929 n/n/n** |
| **152** | **1.0482 Y/Y/Y** | **1.0340 Y/Y/Y** |
| 160 | 1.1206 Y/Y/Y | 1.0886 Y/Y/Y |
| 168 | 1.1774 Y/Y/Y | 1.1436 Y/Y/Y |
| 176 | 1.2229 Y/Y/Y | 1.2072 Y/Y/Y |
| 192 | 1.3159 Y/Y/Y | 1.3119 Y/Y/Y |
| 256 | 1.8068 Y/Y/Y | 1.6950 Y/Y/Y |

- The cells' spreads are small: min-max 0.1–5.3 % per cell, and the bars on the ratio 0.5–10.7 %. At 144 the tree
  reads 12.327 / 14.863 µs against all_pairs 12.421 / 14.757 µs.
- **Both families are monotone:** all_pairs is claimed faster up to 128 (disparity: 136), neither is claimed at 144
  (136–144 uniform), and the tree is claimed faster from 152 on.

### 3.2 The constants (`g4_g5_recipe.md:131-132`)

- **Per family:**
  - LO_f = the largest n at which all_pairs is not claimed slower than tree: **uniform 144, disparity 144**;
  - HI_f = the smallest n at which tree is claimed faster: **uniform 152, disparity 152**.
- **TREE_BRUTE_MAX_ROWS = min(LO_uniform, LO_disparity) = 144.**
- **AUTO_TREE_LO / AUTO_TREE_HI = the wider band (min LO_f, max HI_f) = 144 / 152**, and LO ≥ TREE_BRUTE_MAX_ROWS
  holds.
- Both answers sit inside the grid, with measured sizes on either side: 136 below, 160 above.
- **L2's procedure beside it** (the queue's section 11): the log-log crossovers are 142.6 (uniform) and 145.4
  (disparity). HI = the larger one to two significant figures = **150**, LO = 0.9 × HI = **135**.
- The constants are a code change for the tree lane, not made here:
  - `TREE_BRUTE_MAX_ROWS` sits at `broadphase_tree/mod.rs:144` (= 64 today);
  - `AUTO_TREE_LO/HI` do not exist in code yet.

  Whether the change moves any pose or pin is the lane's to establish.
- **Against window 4** (C3 binary `a46b8287`, before F1): `TREE_BRUTE_MAX_ROWS` 64 and `AUTO_TREE_LO/HI` 64 / 256
  by the grid rule, 126 / 140 by L2's procedure, from a grid of 17/64/128/256. That grid could not place a
  crossover between 64 and 256; this one does.

### 3.3 The crossovers moved UP, against the design's prediction, and the bridge says why

`c3b/design.md:244` predicts "the tree gets cheaper at every n, so the crossovers move down". Window 4's log-log
crossovers were 111 (uniform) and 138 (disparity); these are 142.6 and 145.4. The bridge arms (same binary) split
the move:
- **LeafList against RowWalk:**
  - uniform: **1.401 at 64 (LeafList claimed SLOWER)**, 1.038 at 128 (n/Y/Y), 0.940 at 256 (claimed faster);
  - disparity: 1.057 at 64 (claimed slower), **0.890 at 128** and 0.807 at 256 (claimed faster).

  So F1 makes the tree cheaper only from about 128 up (disparity) or above 128 (uniform), not "at every n".
- **The same kernel across windows:** all_pairs / tree_rowwalk in this binary reads
  - uniform 0.506 / 0.945 / 1.698 at 64 / 128 / 256, against window 4's 0.619 / 1.137 / 1.997;
  - disparity 0.429 / 0.820 / 1.368, against 0.500 / 0.946 / 1.543.

  That is **11–18 % lower on the same kernel**. Between window 4's C3 binary and this trunk, all_pairs got relatively
  faster or the tree's fixed cost grew. Which one is untested.
- **Arithmetic** (a two-point log-log interpolation between 128 and 256; coarse): RowWalk's crossover in this binary
  sits near **137 (uniform) and 167 (disparity)**. So:
  - LeafList moves the disparity crossover DOWN (167 → 145), as the design predicted;
  - it moves the uniform crossover slightly up (137 → 143), where LeafList is not yet cheaper;
  - the rise of both over window 4's 111 / 138 is mostly the same-kernel term above, not F1.

## 4. Q4: G-TW's canary resolution (C4 binary `989ca0f0`, J-A, W=1, reuse on, unarmed)

- **R** = 7.542 % = G-TW's own J-A W1 [0,500) two-spread bar in window 7 (2·hypot(3.352, 1.728) %).
- The four rungs inject canary_frac = k·R / 1.0609 of the reference, k = 0.5 / 1 / 1.5 / 2. The reference is the
  latest valid `L9-JA-on` window mean (window 7's recipe).
- Rows: G-TW's own `on` row (`--scene jolt --gap 0.5 --cfg a --contact-reuse on`) and the same with the canary.
- **Rule** (`plan.md` §4 Q4): a rung is SEEN iff its rise over the reference is claimed upward under r and s and
  `canary_ns` is printed. G-TW's resolution is DEMONSTRATED iff the 1.5 R and 2 R rungs are SEEN. The smallest rung
  seen with every larger rung seen is the resolution demonstrated in this block.

> **Orchestrator's ruling Q4 (applied):** "G-TW's resolution demonstrated (the smallest rung seen with every larger
> one: 0.5 R = 3.77 % of the step); the wave-1 canary under SE stands."

- **The reference:** `L9-JA-on` 16.6480 ms [16.6066-16.8070] K=6 (1.20 / 0.22 / 0.22 %). Window 7 read it at
  16.6838 ms.

| rung | frac | expected rise | step ms (r/i/s %) | injected ms | rise | rise / injected | bars r/i/s | |
|---|---|---|---|---|---|---|---|---|
| 0.5 R | 0.036 | 3.77 % | 17.2533 (0.60/0.18/0.11) | 0.5987 | **+0.6053 ms = +3.64 %** | 1.011 | 2.69 / 0.57 / 0.50 % | **SEEN** Y/Y/Y |
| 1 R | 0.071 | 7.54 % | 17.8861 (0.53/0.39/0.12) | 1.1807 | +1.2381 = +7.44 % | 1.049 | 2.63 / 0.90 / 0.51 % | **SEEN** Y/Y/Y |
| 1.5 R | 0.107 | 11.31 % | 18.5120 (0.54/0.27/0.11) | 1.7794 | +1.8640 = +11.20 % | 1.048 | 2.64 / 0.70 / 0.50 % | **SEEN** Y/Y/Y |
| 2 R | 0.142 | 15.08 % | 19.1239 (0.65/0.49/0.15) | 2.3614 | +2.4759 = +14.87 % | 1.048 | 2.74 / 1.07 / 0.53 % | **SEEN** Y/Y/Y |

- **DEMONSTRATED.** Every rung is seen under both spreads, so the smallest rung seen with every larger one seen is
  **0.5 R = 3.77 % of the step** (measured rise +3.64 %).
  - Each rise tracks its injection (1.01–1.05×), and each bar is about 2.7 % under min-max, against window 7's
    10.96 %.
- **The wave-1 canary under SE stands** (ruling Q4). Window 7 saw its 5.38 % canary under SE only, on the armed twin
  of this row with a 10.96 % min-max bar, and its analysis put that miss on pass 0's load (window 7 `analysis.md`
  §2.3). In a quiet block, G-TW's own J-A W=1 row on the same binary resolves 3.6 % under both spreads.
- **Scope.** This demonstrates the W=1 J-A gate only. G-TW's two-spread resolution at W ≥ 2 and on J-D and R read
  24–73 % in window 7 and is not re-measured here.

## 5. What this wave cannot claim

1. **The W8 headline under the window protocol** (ruling Q1). The rule fails in A (load), C (one Jolt process), D (one
   process of ours) and pooled. The 0.87× is a reading of three clean blocks, not a claim.
2. **The cause of block C's 2.7015 ms Jolt process.** Its receipts and witness are quiet.
3. **The cause of the cfg-A query's between-window term** (+22 % on the parent, +18 % on the tip, against window 6's
   P3).
4. **Why the same-kernel all_pairs / tree_rowwalk ratio fell 11–18 % since window 4.** The RowWalk crossovers (137 /
   167) are two-point arithmetic.
5. **G-TW's resolution at W ≥ 2** or on J-D and R.
6. **Any W1 or W2 Q1 cell** (not in this wave's rows).

## FOLLOW-UP WORK ITEMS

**Q1: the headline**
1. **The rule cannot be met while one process per block can set its bar.** Blocks C and D each fail on one process
   whose 5-s receipts pass. D's is visible only in the during-process witness (14.25 %, `msedge.exe`). Two
   protocol items for the orchestrator, for windows AFTER this one (the rule is not changed for this data, ruling
   Q1):
   - gate the witness, as window 7's FOLLOW-UP 8 proposed (re-run a process whose `others_busy_pct` > 2 %, or its
     re-run slot as for a hot receipt). D's outlier would then have been re-run;
   - decide whether a pooled cell may span blocks that were run under different machine states (block A).
2. **Re-take the headline after C4 merges**, on the new default. In B, C and D the per-manifold ratio is 1.59–1.60× at
   W8, and window 7's arithmetic puts C4 at about 1.44×.

**Q2: C3b and C2**
3. **C2 is not built** (ruling). The default row's t_q is 0.208 ms, 11.6 % below the bar in both blocks.
4. **The cfg-A query's between-window term** (both kernels, +18–22 %) belongs to F3's same-binary A/B.
   `--bp-kernel` (the 2026-09-24 ruling) lets F3 time both kernels in one binary and one block, so this term cancels
   there. It does not cancel in any absolute-bar decision read on cfg-A.

**Q3: the tree lane**
5. **Commit the constants in the tree lane** (ruling): `TREE_BRUTE_MAX_ROWS` 64 → 144 (`broadphase_tree/mod.rs:144`)
   and `AUTO_TREE_LO/HI` = 144 / 152 when Auto's high side is added. Check pins and poses under the lane's own rules.
   The queue's section 11 ("commit only after the refinement run") is answered by this run.
6. **Optional:** a RowWalk arm at 136–176 in the same binary would replace the two-point RowWalk crossover with a
   measured one. That separates F1's effect from the between-window term (§3.3).

**Q4: G-TW**
7. None for the W=1 J-A gate. If G-TW's "no regression" at W ≥ 2 is ever load-bearing, it needs its own quiet
   re-read (window 7's FOLLOW-UP 1).

## 6. Files

In `docs/measurements/2026-09-25-physics-window7/wave2/`:
- `tools/analyze7b.py`: this analysis's script. It resolves the wave-2 record from its own location, and window 7's
  record from the directory above.
- `analyst/tables.txt` (byte-identical to `analyst_run.log`) and `analyst/reduction.json`.
- The driver's reduction: `raw/reduction.json`, rendered as `raw/tables.md` (`tools/reduce7b.py`).
- This file: the harness refused the analyst's write, so its text was returned to the orchestrator to add verbatim.
