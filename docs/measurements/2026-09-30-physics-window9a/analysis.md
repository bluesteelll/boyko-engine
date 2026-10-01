# Physics window 9a: synthesis (results-analyst, 2026-09-30)

The window ran on 2026-09-30 from 19:13:54 to 22:16:27 (182.6 min) on the owner's workstation (8 physical / 16 logical cores, windows-msvc).
- TIP = u/phys-tree-c4 `50e31f1a`: Tree is the default broadphase, and the change is value-neutral (J-D = J-T).
- PARENT = trunk `3d9433ae`: AllPairs is the default.
- The owner set the STOP flag at 22:03:29. The driver ended with exit 3: "ABORT in C4-BR-p0: machine never idle (wait_idle exit 3)".
- Three blocks closed all 3 passes: C4-AB (with Jolt 5.6 in-block), C4-RAPIER and C4-G5.
- C4-BR aborted in its only pass. C4-G4 and C4-G4-kd never ran.

> **Operational, now:** D: is full. `df` showed 104 MB free at 22:59, against 68.4 GB at 21:43.
> - The last idle attempt logged 5 GB free from 22:09 and 0.1 GB at 22:16. Its own guard ended it: "STOP: D: free 0.1 GB < 2 GB".
> - No build and no window can run until space is freed (PC-9a-0).

## 1. Headline

**Are we faster than Jolt 5.6 and Rapier 0.36?** The scene is the J-T pyramid (1,240 boxes, 500 steps), measured in the steady state [100,500), on the shipped default after C4 (Tree broadphase).

- **Against Jolt 5.6: yes, at every W**, on the wall and on equal work.
  - Wall, ours/Jolt: 0.435 at W1, 0.506 at W2, 0.586 at W4, 0.696 at W8, 0.818 at W16.
  - Per velocity row-iteration, the equal-work unit (Jolt does 0.949× our rows): 0.413 / 0.480 / 0.556 / 0.660 / 0.776.
  - Per manifold, Jolt is cheaper from W4 up (1.11 / 1.32 / 1.55×). The plan rates this unit unfair, because Jolt carries 1.90× our manifolds.
- **Against Rapier's defaults: only at low W.** With Rapier's faster build (simd8), ours/Rapier is 0.884 at W1 and 0.987 at W2, then 1.073 at W4, 1.266 at W8 and 1.298 at W16.
- **On equal work, Rapier is ahead from W2.** Per row-iteration against Rapier matched to our configuration (which does 1.80× our rows), ours/Rapier is 1.013 at W1 and 1.18–1.32 at W2–W16.
- **Almost none of this is claimed by the pre-registered letter.** The only claim is Jolt's per-manifold lead at W16 (STRONG).
  - C4-AB's pass 0 was contaminated: 30 of its 34 pass-cells have K < 3, and ruling 8 lets no short pass-cell gate.
  - In the two clean passes (post hoc), ours is faster than Jolt STRONG at W1–W4 on the wall and at W1–W8 per row, and Rapier's default is faster than ours at W8.
- **Fidelity caveat.** Every "ours faster" is at unequal fidelity. Our pre-V2 pile deforms (26 boxes past 0.5 m, 2.18 m maximum drift); Jolt and Rapier hold theirs.
  - V2 is already ruled in. It fixes this and nearly doubles our contact work (points ×1.86; manifolds 8,496–8,501, level with Jolt's 8,489).
  - By arithmetic, ours after V2 would be 0.93–1.30× Jolt at W8 and 1.09–1.53× at W16, and 1.68–2.36× Rapier's default at W8.
  - So the lead over Jolt at W ≥ 8 is not safe at equal fidelity, and the loss to Rapier widens. Window 9b measures it.

**Headline table** ([100,500); ours = C4-JD#tip; Rapier = its faster build per cell, rs8 in all 10; `s1_standing.txt` §4):

| W | ours ms | ours/Jolt wall | ours/Jolt per velocity row | ours/Jolt per manifold | ours/Rapier-default wall | ours/Rapier-matched wall | ours/Rapier-matched per row |
|---|---|---|---|---|---|---|---|
| 1 | 4.3504 | 0.435 | 0.413 | 0.826 | 0.884 | 0.564 | 1.013 |
| 2 | 2.9392 | 0.506 | 0.480 | 0.961 | 0.987 | 0.658 | 1.182 |
| 4 | 2.1605 | 0.586 | 0.556 | 1.113 | 1.073 | 0.677 | 1.216 |
| 8 | 1.7878 | 0.696 | 0.660 | 1.322 | 1.266 | 0.701 | 1.260 |
| 16 | 2.0486 | 0.818 | 0.776 | 1.554 | 1.298 | 0.733 | 1.317 |

**Other cells:**
- Jolt: 10.0044 / 5.8123 / 3.6881 / 2.5702 / 2.5056 ms.
- Rapier default (RP-D rs8): 4.9213 / 2.9767 / 2.0129 / 1.4127 / 1.5780 ms.
- Rapier matched (RP-M rs8): 7.7172 / 4.4683 / 3.1927 / 2.5494 / 2.7958 ms.
- On [0,500) the ours/Jolt wall ratio is 0.4505 / 0.5270 / 0.6117 / 0.7221 / 0.8513.
- The Rapier group's pre-registered comparator is J-T#tip at W1/8/16 (J-D#tip at W2/4). Its ratios differ from these by at most 1.52 %.

**Scaling, T1/TW at W2/4/8/16** ([0,500)):

| engine | W2 | W4 | W8 | W16 | T16/T8 |
|---|---|---|---|---|---|
| ours | 1.488 | 2.044 | 2.500 | 2.188 | 1.143 |
| Jolt | 1.741 | 2.776 | 4.008 | 4.134 | 0.969 |
| RP-D rs8 | 1.655 | 2.430 | 3.423 | 3.034 | 1.128 |
| RP-M rs8 | 1.708 | 2.406 | 3.015 | 2.780 | 1.085 |

## Method

- **Statistics.** Window 8b's method, unchanged.
  - A cell is the median over K of per-process values (runner and Rapier: the mean of `wall_ns` over the window; Jolt: `Time (ms)`).
  - i = IQR / median, s = 1.2533·SD/√K / median, r = (max − min) / median.
  - A flag is set iff |B/A − 1| > 2·hypot(A_x, B_x).
- **Claim rule.** Ruling 1: CLAIMED iff i AND s flag pooled and in every pass, with one sign throughout. STRONG iff r also flags.
- **Slot rule.** Ruling 8: use the original if it is clean and valid, else the first clean, valid re-run, else drop the slot.
- **Readings.** LETTER is the verdict: a pass-cell with K < 3 sets no flag. GATING-ONLY, which is the Rapier group's EXCL, gates only on passes where both pass-cells have K ≥ 3. It is **post hoc**, for sizing only.
- **Passes used.** Only passes with a `pass_done` record.
- **Per-unit ratios.** They scale the wall ratio by a constant work factor:
  - ours: C4-JD-armed#tip census, identical in all 18 armed processes: 16,553.755 points, 4,467.665 manifolds and 595,935.2 velocity row-iterations per step;
  - Jolt: the DOSSIER census, 565,790 velocity rows + 62,224 position rows, 8,489 manifolds;
  - Rapier: `rows_pin` values, RP-D 354,674 and RP-M 1,071,107.
- **Sources.** Each group's numbers come from its verified scripts (`analysis/c4ab/`, `analysis/rapier/`, `analysis/g5/`; all three verifications: AGREE). The window-wide census, the consolidated standing, the V2 arithmetic and a second derivation of every decisive figure come from `analysis/synth/` (listed at the end).

## 2. Window validity (`synth/s0_validity.txt`)

**What ran:**

| block | passes | time | timed processes (originals + re-runs) | invalid | unclean | slots used / dropped | pass-cells K < 3 |
|---|---|---|---|---|---|---|---|
| C4-AB (+ Jolt) | 3 of 3 | 19:13–20:43 | 379 (306 + 73) | 0 | 119 (p0 84/140, p1 3/105, p2 32/134) | 260 (50 re-runs) / **46, all p0** | **30 of 102, all p0** |
| C4-RAPIER | 3 of 3 | 20:43–21:22 | 214 (180 + 34) | 0 | 34 (p0 31/91) | 180 (27) / 0 | 0 of 60 |
| C4-G5 | 3 of 3 | 21:22–21:40 | 95 (90 + 5) | 0 | 5 | 90 (5) / 0 | 0 of 30 |
| C4-BR | p0 aborted | 21:40–22:16 | 8 records, 7 completed, no `pass_done` | — | — | — | **NOT MEASURED, not analysed** |
| C4-G4, C4-G4-kd | never ran | — | 0 | — | — | — | **NOT MEASURED** |

**Validity checks.**
- 696 timed process records and 12 warm-ups; 0 voided passes, 0 voided processes; R4 pose compares 13 of 13 equal.
- Invalid: 0 by the driver. The group libraries' independent re-checks agree on 379/379, 214/214 and 95/95.
- My K equals the driver's `k_clean` on every pass-cell: 102/102, 60/60 and 30/30.
- **Poses and hashes hold:**
  - all 222 used C4-AB runner processes carry pose `0x30c5438bc6ad9ffa`, with bytes equal to the fixture and void_steps 0;
  - all 38 used Jolt processes carry `0xb8522b4e3fc62cfe`;
  - Rapier fails no V1–V9 rule on any of its 214 processes;
  - G5's 90 processes carry the three expected poses, with void 0.

**What weakens the window:**

1. **C4-AB pass 0 is a dead pass by the letter.**
   - The desktop was busy: the top other process on unclean runs was Telegram 47, browser 41, claude.exe 24.
   - The witness median over all p0 processes was 2.15 % (p1 0.43 %, p2 1.06 %).
   - The re-run stage ran one wave of 74 slots, then its 425 s allowance was spent.
   - Only four p0 pass-cells are full: JA#tip@W1, JD#tip@W16, jolt56@W16 and rung@W8. jolt56@W8 has K = 0.
   - As a result, every C4-AB and Rapier claim that uses a short p0 cell is NOT CLAIMED. Passes 1 and 2 are clean, with K = 3 everywhere.
2. **Re-runs read slow (post hoc, C4-AB group).**
   - The 50 used re-runs have a median of 1.0378× their cell's originals, and 45 of 50 are above it.
   - Control: originals, leave-one-out, median 0.9996.
   - Ruling 8's re-runs therefore widen the pooled bars.
3. **Jolt W16 is bimodal.** Five processes run at 2.344–2.454 ms and four at 2.836–2.936 ms; IQR 22.6 %. No W16 wall reading against Jolt can resolve.
4. **Rapier is cross-block.** No row is shared with C4-AB, so block drift cannot be measured. The adjacency with our rows was waived by ruling (a).
   - Ours ran on the busier machine. Used-process receipt medians: AB p0 2.47/2.65/0.95 %, against RAPIER p1–p2 ≤ 0.62/0.62/0.50 %. Any drift counts against ours.
   - Uniformly slow Rapier processes with clean receipts recurred at W1 in p0.
5. **The precondition "V2 is on the trunk" is NOT met** (window9a_rows.md). Every "ours faster" is recorded as UNEQUAL QUALITY.
6. **Idle rule.** It was reached at the 130.5 s floor in all 10 passes, 21.7 min in total.
7. **The end of the window.**
   - The STOP flag (22:03:29) took effect only at 22:16:27. The driver had spent 22:02–22:09 in the pre-process quiet wait, then ran the full idle rule.
   - During that idle rule the machine sat at 44–89 % CPU for six polls (top process `steam`), and D: fell to 0.1 GB.
8. **Sentinel (cross-window, no claim).** 9a's tip J-D reads 0.7–2.7 % slower than 8b's trunk J-T; it is the same engine code in a different binary.

**Deviations (PREP):**
- C4-BR ran at K 3, diagnostic only (ruling (b)).
- Jolt was folded into C4-AB.
- Rapier's rows census comes from J-D-armed rather than J-T-a (the same contact set).
- J-D stands in for J-T at W2/W4 (A/A twin: at most 1.52 % apart, nnn in every cell).
- Re-runs followed ruling 8's waves.

No git worktree was edited: `git status` is clean in D:/wt/lighttable and D:/wt/joltab.

## 3. Pre-registered verdicts

### 3.1 C4-AB + Jolt 5.6

Window [0,500) unless named. A = the AllPairs row or parent; B = C4-JD#tip. n is pooled (p0/p1/p2). Flags are i/s/r, pooled; then per pass.

| claim | bar | cells, ms (n) | B/A, B − A | flags | LETTER | post hoc (GATING-ONLY) |
|---|---|---|---|---|---|---|
| MG-ap W1 | J-D not claimed slower than J-Dap (same binary) | 6.1501 (7: 1/3/3) → 4.5692 (7: 1/3/3) | 0.7429, −1.5809 | YYn; nnn(sep) YYY YYn | **HOLDS** | faster CLAIMED |
| MG-ap W2 | same | 4.7129 (8: 2/3/3) → 3.0706 (8: 2/3/3) | 0.6515, −1.6423 | YYY | **HOLDS** | faster STRONG |
| MG-ap W4 | same | 3.9106 (7) → 2.2356 (7) | 0.5717, −1.6750 | YYY | **HOLDS** | STRONG |
| MG-ap W8 | same | 3.4851 (7) → 1.8277 (7) | 0.5244, −1.6574 | YYY | **HOLDS** | STRONG |
| MG-ap W16 | same | 3.7003 (8: 2/3/3) → 2.0887 (9: 3/3/3) | 0.5645, −1.6116 | YYY | **HOLDS** | STRONG |
| MG-par W1 | J-D not claimed slower than J-Dpar (parent default) | 6.2008 (8) → 4.5692 (7) | 0.7369, −1.6316 | YYY; nnn(sep) YYY YYY | **HOLDS** | STRONG |
| MG-par W2 | same | 4.6971 (8) → 3.0706 (8) | 0.6537, −1.6265 | YYY | **HOLDS** | STRONG |
| MG-par W4 | same | 3.8845 (7) → 2.2356 (7) | 0.5755, −1.6489 | YYY | **HOLDS** | STRONG |
| MG-par W8 | same | 3.5206 (7) → 1.8277 (7) | 0.5191, −1.6929 | YYY | **HOLDS** | STRONG |
| MG-par W16 | same | 3.7023 (8) → 2.0887 (9) | 0.5642, −1.6136 | YYY | **HOLDS** | STRONG |
| J-D claimed faster | plan B1 arithmetic −1.70 ms, no bar | as above; [100,500) −1.565 to −1.668 | 93–100 % of −1.70 | as above | **NOT CLAIMED** | CLAIMED STRONG ×9; W1-ap CLAIMED |
| CTL J-A W1 / W8 / W16 | parent vs tip not claimed different | 16.6280 → 16.6133 (8; 9); 5.3197 → 5.3231 (7; 7); 5.2500 → 5.2484 (8; 8) | 0.9991 / 1.0006 / 0.9997 | nnn | **NOT CLAIMED** (control holds) | same |
| CTL J-T W1 / W8 / W16 | same | 4.5465 → 4.5930 (8; 7); 1.8733 → 1.8502 (8; 7); 2.0756 → 2.0597 (8; 7) | 1.0102 / 0.9876 / 0.9923 | nnn | **NOT CLAIMED** (control holds) | same |
| RUNG W8 | a 0.060 ms rise claimed | J-D 1.8277 (7) → rung 1.9191 (9) | +0.0914 (+5.00 %); bars i 16.56 / s 5.22 % | nnn; nnn YYn nnn | **NOT SEEN** | NOT SEEN |
| RUNG W16 | same | 2.0887 (9) → 2.1294 (8: 2/3/3) | +0.0407 (+1.95 %) | nnn | **NOT SEEN** | NOT SEEN |
| poses / hashes / void steps | every hash equals its row's; void 0 | 222 runner + 38 Jolt used processes | — | — | **HOLD** | — |
| G5 recorded | M7's second half | 30 of 30 G5 pass-cells at K 3 | — | — | **RECORDED** | — |
| **M7: C4 merges** | no J-D W slower, and G5 recorded | 10 of 10 comparisons HOLD | — | — | **MERGE** | — |

**Jolt rows (PLAN M8, a measurement with no bar).** Ours/Jolt uses A = Jolt, B = ours. Jolt n: 8 (2/3/3), 8 (2/3/3), 7 (1/3/3), **6 (0/3/3)**, 9 (3/3/3). Ours n: 7 / 8 / 7 / 7 / 9.

| unit | W1 | W2 | W4 | W8 | W16 | LETTER | post hoc |
|---|---|---|---|---|---|---|---|
| Jolt wall [0,500), ms | 10.1435 | 5.8263 | 3.6547 | 2.5311 | 2.4535 (i 22.6 %) | — | — |
| ours/Jolt wall [0,500) | 0.4505 | 0.5270 | 0.6117 | 0.7221 | 0.8513 | **NOT CLAIMED** ×5 | ours faster STRONG W1/2/4, CLAIMED W8; W16 not claimed |
| per velocity row [100,500) | 0.4128 | 0.4801 | 0.5562 | 0.6604 | 0.7763 | **NOT CLAIMED** ×5 | ours cheaper STRONG W1–W8 |
| per all rows [100,500) | 0.4583 | 0.5329 | 0.6174 | 0.7330 | 0.8616 | **NOT CLAIMED** ×5 | STRONG W1–4, CLAIMED W8 |
| per manifold [100,500) | 0.8263 | 0.9609 | 1.1131 | 1.3217 | **1.5536** | W1–W8 NOT CLAIMED; **W16 CLAIMED STRONG, Jolt cheaper** | Jolt cheaper CLAIMED W4, STRONG W8 |

**Post hoc context:**
- The shipped default before C4 (J-Dpar/Jolt, in-block) was 0.611 / 0.806 / 1.063 / **1.391** / **1.509**. So C4 turned the default from "Jolt faster at W4–W16" into "ours faster in medians at every W".
- Window 8 (cross-window): ours/Jolt 0.449 / 0.795 / 0.866 at W1/8/16, against 9a's 0.4505 / 0.7221 / 0.8513.
- In paired rounds, ours was faster in every round at every W (5/5, 7/7, 6/6, 5/5, 6/6).

### 3.2 C4-RAPIER (Rapier 0.36, rs8 = simd8, rs4 = 4 lanes)

**R-ARM** (rs8 against rs4; K 9 (3/3/3) everywhere; [100,500)). The ratio is rs4/rs8, so above 1 means rs8 is faster.

| cfg | W1 | W2 | W4 | W8 | W16 |
|---|---|---|---|---|---|
| RP-D | 1.168, NOT CLAIMED | 1.144, **CLAIMED rs8** | 1.136, **CLAIMED rs8** | 1.111, NOT CLAIMED | 1.077, NOT CLAIMED |
| RP-M | 1.303, **CLAIMED STRONG rs8** | 1.256, **CLAIMED rs8** | 1.169, **CLAIMED rs8** | 1.073, NOT CLAIMED | 1.057, NOT CLAIMED |

- rs4 is claimed faster nowhere, and rs8's median is lower in 10 of 10 cells.
- On [0,100), rs8 is claimed at RP-D W2 and RP-M W1 (STRONG) and W2.

**R-WALL / R-ROW, [100,500) (primary).**
- A = ours = J-T#tip at W1/8/16, J-D#tip at W2/4. Ours n is 7 (1/3/3), or 8 (2/3/3) at W2. Rapier n is 9 (3/3/3).
- The table gives **ours/Rapier** (< 1 = ours faster or cheaper) as rs8 ; rs4.
- Rule: "ours faster" must be claimed against both builds; "Rapier faster" against either one.

| claim | W1 | W2 | W4 | W8 | W16 | LETTER | EXCL (post hoc sizing, ours-first) |
|---|---|---|---|---|---|---|---|
| R-WALL-D (Rapier defaults) | 0.886 ; 0.759 | 0.987 ; 0.863 | 1.073 ; 0.945 | 1.281 ; 1.153 | 1.279 ; 1.187 | **NOT CLAIMED** ×5 | W1 ours faster; W8 **Rapier faster** (both builds); W2/W4/W16 not |
| R-WALL-M (matched; rows 1.797× ours, outside [0.90, 1.10], so not equal work) | 0.565 ; 0.434 | 0.658 ; 0.524 | 0.677 ; 0.579 | 0.710 ; 0.662 | 0.722 ; 0.683 | **NOT CLAIMED** ×5 | ours faster at every W (STRONG W1/2/4/16) |
| R-ROW-D (per row, vs defaults) | 0.528 ; 0.452 | 0.588 ; 0.514 | 0.639 ; 0.562 | 0.763 ; 0.686 | 0.761 ; 0.707 | **NOT CLAIMED** ×5 | ours cheaper at every W (STRONG W1–W8) |
| R-ROW-M (per row, vs matched) | 1.016 ; 0.780 | 1.182 ; 0.942 | 1.216 ; 1.041 | 1.276 ; 1.189 | 1.297 ; 1.227 | **NOT CLAIMED** ×5 | **Rapier cheaper** at W4 (rs8), W8 (rs4), W16 (both); W1/W2 not |
| all 20 [0,100) claims | — | — | — | — | — | **NOT CLAIMED** ×20 | Rapier default is faster at every W during the landing (ours/Rapier-rs8 1.27–1.39) |
| VOID | V1–V9 | 0 of 214 processes | | | | **none** | — |
| REFUTED | — | — | | | | **none** | — |

**What blocks the letter.** Every R-WALL/R-ROW claim fails only because our C4-AB p0 pass-cell has K 1–2.

**Post hoc:**
- J-D#tip at W16 is our only cell with K 3/3/3. With it as comparator, the letter gives:
  - R-WALL-M ours faster STRONG against both builds;
  - R-ROW-D ours cheaper CLAIMED;
  - **R-ROW-M Rapier cheaper CLAIMED against both builds** (ours/Rapier 1.317 / 1.246).
- The pre-registered outcome reading "Rapier cheaper on ROW-M = a per-row implementation loss" fires only under EXCL (W4–W16).
- Per-row parity at W1 (1.016 against rs8) and a loss that grows with W make it a **scaling** loss, not a kernel loss.
- Correction to the Rapier group's EXCL column (sizing only): R-ROW-D W8 is STRONG in the ours-first orientation (`r1_claims.txt:147`). The table printed the Rapier-first strength.

### 3.3 C4-G5 (broadphase profiles, TIP, single binary; K 3 in all 30 pass-cells)

| claim | bar | cells (n) | value | flags | verdict |
|---|---|---|---|---|---|
| B1 W1 | Σ verify+build+query+assemble on C4-JD-armed, median over [100,500) steps ≤ 0.36 ms | n 9 (3/3/3) | 0.2502 [0.2486–0.2522]; −30.50 % against the bar | YYY ×4 | **PASS, CLAIMED below, STRONG** (0.110 ms headroom) |
| B1 W8 | same, ≤ 0.35 ms | n 9 | 0.2608 [0.2602–0.2617]; −25.48 % | YYY ×4 | **PASS, STRONG** (0.089 ms headroom) |
| B2 | realised Δbp(8) = SYS(JDap-armed) − SYS(JD-armed) ≥ 1.03 ms | n 9 / 9 | 1.8390 − 0.2611 = **+1.5779** (per pass 1.5780 / 1.5780 / 1.5773) | YYY; above 1.03 YYY | **PASS, STRONG** |
| B3 W1 | C4-R (Tree) claimed faster than C4-Rap | n 9 / 9 | 7.8501 → 6.2638 (−20.21 %) | YYY ×4 | **CLAIMED STRONG** |
| B3 W8 | same | n 9 / 9 | 4.1626 → 2.5136 (−39.61 %) | YYY ×4 | **CLAIMED STRONG** |
| B4 | C4-S16 not claimed slower than C4-S16ap (W1) | n 9 / 9 | 30.142 → 30.621 µs (+1.59 %); resolves only above ~5.8 % | nnn | **HOLDS** |
| B5 | void 0 on armed rows; poses = fixture; R4 S16 = S16ap | 36 armed / 90 used / 13 R4 | all hold | — | **HOLDS** |
| S5 input | t_q(W8) against 8b's 0.2069 ms | n 9 | 0.2076 (mean over steps), +0.33 % | nYn | **NOT CLAIMED different** (cross-window) |
| S5 letter re-read | t_q ≥ 5 % of T(8) | n 9 | 11.1 % of the armed step | YYY | **CLAIMED STRONG** |
| t_q(W16) on the tip; spans at W2/W4/W16 | — | no row | — | — | **NOT MEASURED** |

**Stage profile (C4-JD-armed, W8).**
- Query 0.2046 ms (median over steps) is 78.5 % of Σ4. The serial broadphase system is 0.2611 ms, 14.2 % of the armed step.
- Post hoc: every serial stage costs about 4 % more at W8 than at W1 (query +4.41 % STRONG).

### 3.4 Not measured

**C4-BR** (aborted), **C4-G4** and **C4-G4-kd** are NOT MEASURED. C4-BR's 7 completed processes belong to a pass with no `pass_done` record and are not analysed.

## 4. Decisions

### 4.1 C4 merges by its own rule (M7)

- **The rule is met.** All 10 no-slower comparisons HOLD, and G5 is recorded (30/30 pass-cells at K 3, every bar passing STRONG).
  - Every hash and pose equals its row's, and void steps are 0.
  - The controls show no binary drift: J-A −0.09 to +0.06 %, J-T −1.24 to +1.02 %, never claimed.
- **The pass-0 question does not touch this.** The rule is "not claimed slower", and J-D is 1.58–1.69 ms faster at every W in both readings.
  - That is 93–100 % of the plan's −1.70 ms.
  - Even in p0's short cells, every J-D process is below every AllPairs process (`s3_decisive.txt` §1).
- **What the merge does not settle.** C4-G4 and C4-BR are NOT MEASURED, so `TREE_BRUTE_MAX_ROWS` 128 and `AUTO_TREE_LO/HI` 128/136 stay PROVISIONAL / NOT RE-READ.
  - Ruling W8B-5's letter ("re-read G4 at 96–128 before tree C4 wires AUTO_TREE_LO/HI") is not met.
  - The flip reads only `TREE_BRUTE_MAX_ROWS` (C4-1). Auto is opt-in (C4-2, `b858c238`, separable).
  - Merging the whole lane now, or holding C4-2, is the orchestrator's call.

### 4.2 What the standing implies for the plan (SR, S5, V2)

1. **We win at low W and lose ground as W grows, against both engines.**
   - T1/T8 is 2.50 for us, against 3.02–3.42 for Rapier and 4.01 for Jolt.
   - At W1 we are 2.3× faster than Jolt, 1.13× faster than Rapier's default, and level per row with Rapier matched.
   - The losses appear from W4: per manifold against Jolt, the wall against Rapier's default, and per row against Rapier matched from W2, growing with W.
   - The gap is parallel scaling, not the kernel. That is SR's target (window 8: 70 % of the W8 excess is serial), plus S5's serial broadphase.
2. **S5's target stands on the C4 tip.**
   - t_q(W8) is 0.2076 ms, unchanged from 8b (+0.33 %, not claimed).
   - The re-priced gain is 0.142–0.145 ms at W8, which is 8 % of T(8) and above PLAN rev 3's 0.066 ms floor (G5 group).
   - W16 needs an armed C4-JD row, which 9a did not have.
3. **SR.** Its predicted Δ is 0.29 ms at W8 (PLAN M7, priced on today's contact set). Its merge bar is a formula fixed at window prep on the landing trunk's census (ruling 2026-09-30 item 8, Q4).
   - 9a gives the pre-V2 baseline: J-D T(8) = 1.8277 ms [0,500) and 1.7878 ms [100,500).
   - It also shows the trunk is slower at W16 than at W8 in medians (+14.3 %, not claimed). SR's "no W slower, W16 included" gate starts from that.
4. **V2 changes the question: arithmetic, not a measurement** (`s2_v2_arith.txt`).
   - **Work.** F0g's work census for V2 (K3 + S20 + VMARGIN, gap 0.5) gives points ×1.857–1.868 and manifolds 8,496–8,501. The contact set becomes like-for-like with Jolt (8,489).
     - But our velocity row-iterations become 1.96× Jolt's (Jolt/ours 0.949 → 0.51) and 3.12–3.14× Rapier-default's rows.
     - RP-M/ours rows becomes 0.962–0.968, inside the equal-work band, so R-WALL-M becomes an equal-work claim after V2.
   - **Cost.** The F0 VERDICT's model, reproduced on S20 (+0.476–0.479 ms at W8 against its +0.476), gives **at least +0.589–0.598 ms at W8** and about +0.68 ms at W16 (rows only).
     - This is a lower bound, because setup and narrowphase growth are left out.
     - The upper bound, if the whole step scales with rows, is ×1.86.
   - **Standing after V2:**
     - W8: 0.93–1.30× Jolt, 1.68–2.36× Rapier default, 0.93–1.31× Rapier matched.
     - W16: 1.09–1.53× Jolt.
     - W1: at most 0.81× Jolt.
   - **V2 costs more than SR + S5 win at W8** (0.43 ms on pre-V2 pricing).
     - Chained through, the W8 trunk after V2 + SR + S5 lands at 1.94–2.91 ms: 0.76–1.13× Jolt and 1.38–2.06× Rapier's default.
     - So SR + S5 are what keeps "not slower than Jolt" at W8 after V2. Beating Rapier's default at W ≥ 4, at equal fidelity, is outside what the current plan prices.
     - The plan's end state (T(8) 1.28–1.42 ms, per-row 0.46–0.57 of Jolt) was priced pre-V2 and must be re-priced (PC-V2-6 already requires it).
   - **The iteration budget.** At near-equal manifold counts we would do about 2× Jolt's velocity work, and about 3× the rows on which Rapier's default holds the same pile.
     - The pre-registered Rapier reading "Rapier faster on WALL-D, ours cheaper on ROW-M = our iteration budget" did not fire.
     - V2 still puts the budget on the table: owner Q3.

### 4.3 What remains to be measured (window 9b and after)

- **C4-BR** (bracket receipt, K 3 diagnostic), **C4-G4** (sizes 96–160, the one-constant commit for `TREE_BRUTE_MAX_ROWS` / `AUTO_TREE_LO/HI`) and **C4-G4-kd** (F3 keep or freeze): all NOT MEASURED. Run them before the one-constant commit.
- **V2's cost:** the V2-AB / V2-SPAN / V2-RES blocks of `v2-spec/window9_v2.md` with Jolt in-block, and the work receipts at d = 0 and d = 0.02.
- **SR and S5 A/B (M7)** on the V2 trunk, J-T and J-A, W 1/2/4/8/16, with Jolt 5.6 in-block. Add an armed C4-JD (Tree) W16 row to price S5 at W16.
- **A Rapier standing that can be claimed:** RP-D / RP-M rows inside the same block as our J-T / J-D, adjacent (placement recipe (B)), on the V2 trunk. That removes the cross-block drift, and R-WALL-M then counts as equal work.

### 4.4 Points for the orchestrator

- **PC-9a-0 (now).** D: is full: 104 MB free at 22:59, down from 68.4 GB at 21:43. The idle rule refuses below 2 GB, and builds will fail. Free space before any lane resumes.
- **PC-9a-1 (the K guard).** Keep ruling 8's letter; taking EXCL after seeing the data would be a post hoc rule change.
  - Do not spend a quiet window re-running pre-V2 C4-AB for a standing that V2 supersedes.
  - Restate Jolt and Rapier in-block on the V2 trunk in 9b instead.
- **PC-9a-2 (re-run bias, +3.8 %).** Re-run both members of an adjacent pair, or keep the original order inside a wave. Keep the desktop quiet (owner Q1).
- **PC-9a-3 (C4-2).** Merge C4-1 only, or the whole lane with Auto opt-in: the orchestrator's call. The constants stay marked PROVISIONAL until C4-G4 runs.
- **PC-9a-4 (correction notes, no verdict changes):**
  - The Rapier group's V2 arithmetic (1.676–1.793×) omits VMARGIN (ruling 9); full V2 per F0g is 1.857–1.868× rows.
  - C4-AB's "≈7.6k manifolds" is S20 only; full V2 is 8,496–8,501.
  - R-ROW-D W8 EXCL is STRONG ours-first.
- **PC-9a-5 (STOP latency).** The STOP flag at 22:03 took effect at 22:16, because the driver checks it only between processes, not inside the quiet wait or the idle rule. Consider polling STOP inside both waits.
- **PC-9a-6 (build condition).** Rapier is built with codegen-units = 1 and x86-64-v3; ours uses the parity profile with codegen-units = 16. Neither side's sensitivity is measured. Either measure a codegen-units = 1 twin of our tip, or rule the condition standing (see owner Q4).

## 5. Open questions for the owner (values only)

1. **The desktop during quiet windows.** Telegram, the browser, claude.exe and CurseForge made 84 of 140 C4-AB pass-0 processes unclean. That cost 46 slots and every letter claim against Jolt and Rapier; Steam and Discord were busy when the window ended. Can future windows run with these closed and agent sessions idle? (8b's Q2, again.)
2. **What "beat Rapier" means.** Is it Rapier as it ships (defaults, which today does 0.60× our rows and holds the pile), or Rapier configured to our work (matched)?
   - Today, ours is faster than the first only at W1–W2, and more expensive per row than the second at W ≥ 2.
3. **The solver's iteration budget.** After V2, we would spend about 2× Jolt's velocity row-iterations and about 3× Rapier-default's rows on a pile all three hold. Today's budget is 4 substeps × (1 biased + 2 relax) = 12 sweeps.
   - May that budget be reduced, trading solver margin for speed, if the V2 fidelity gate (the gap sweep, reuse on and off) still holds every run?
4. **Build profile.** Rapier's comparison build uses codegen-units = 1. Is a slower release build of ours (codegen-units = 1) acceptable as the shipped profile, if it measures faster?

## Scripts and outputs

Everything is in `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a/analysis/synth/`. Every script reads `raw/` through the verified group libraries.

| script | output | what it covers |
|---|---|---|
| `synth9a.py` | library | reuses `analysis/c4ab/lib9a.py` and `analysis/rapier/lib_rp.py` |
| `s0_validity.py` | `s0_validity.txt` | records, validity, clean, slots, K against the driver, timeline and idle waits, receipts, C4-BR, the last idle attempt |
| `s1_standing.py` | `s1_standing.txt` / `.json` | cells, ours against Jolt and Rapier on every unit, both readings, both comparators, scaling |
| `s2_v2_arith.py` | `s2_v2_arith.txt` | V2 work and cost arithmetic; the lever budget |
| `s3_decisive.py` | `s3_decisive.txt` | M7, the controls, G5 B1/B2 and t_q, the one letter claim, Jolt W16 bimodality, figures reported on different bases |

The group appendices, all verified AGREE: `analysis/c4ab/` (a0–a7), `analysis/rapier/` (r0–r5), `analysis/g5/` (g5_validity, g5_profile, g5_bars, g5_s5).

## Resume 2026-10-01: C4-CGU, C4-BR, C4-G4, C4-G4-kd

The resumed run (`--resume`, run tag `033740`) ran from 03:37:40 to 05:48:56 (131.3 min; the schedule estimated 171.4) on the owner's workstation (8 physical / 16 logical cores, windows-msvc). The other apps were closed. WINDOW_DONE reads `exit 0 / complete` and counts exit 0. The cutoff (07:03) was not reached. Sections 1–5 above cover the first run (C4-AB + Jolt, C4-RAPIER, C4-G5) and are unchanged.

### R1. Headline

- **All four blocks are complete and clean by the letter.** 8 of 8 passes closed and 134 timed processes ran, with 0 invalid and 0 slots dropped. All 37 pass-cells have K = 3. Unlike C4-AB's pass 0, every pre-registered comparison here gates.
- **codegen-units 1 vs 16 (C4-CGU): the profile stays at cgu 16.**
  - In both metric windows, cgu1/cgu16 is within ±1.05 % at every W, and no flag is set in any pass or pooled.
  - CGU-PROFILE is NOT CLAIMED, so "PROFILE MOVES TO cgu 1" is **NO**. It is not REFUTED: nothing is claimed in either direction.
- **Tree thresholds (C4-G4): the re-read lands exactly on the provisional constants.** In both families it gives LO 128 / HI 136.
  - all_pairs is STRONG faster than tree at 96–120.
  - At 128 nothing is claimed.
  - The tree is STRONG faster at 136–160.
  - So `TREE_BRUTE_MAX_ROWS` = 128 and `AUTO_TREE_LO/HI` = 128/136 are now **measured**. Window 7's 144/152 is REFUTED.
- **F3 / tree_kd (C4-G4-kd): FREEZE-AND-REMOVE.** C4 never selects kd. Below T, all_pairs beats kd by 25–47 %, and at 128 kd is not faster than either tree or all_pairs.
- **The all_pairs bracket (C4-BR, diagnostic, K 3): the slowdown since window 7 is a binary term, not a window term.**
  - In one window, g4r8b and g4rT are 11.5–13.0 % slower than g4r7 on all_pairs at 4 of 4 ids, and min-max separated at each one.
  - g4rT and g4r8b read the same (≤ 0.35 % apart).
  - This is consistent with M9's verdict (b), placement. No fix lane opens (cut Q4).
- **Post hoc caution.** The same tip exe with the same args ran **about 5 % slower at W ≥ 8** in the resume than in the first run (W8 +5.62 %, W16 +5.32 %). No receipt catches this.
  - In-block A/Bs are unaffected.
  - Any reading across the two runs at W ≥ 8 is unsafe by about 5 %. That includes 9a's T(8) = 1.7878 ms baseline.

### R2. Validity of the resumed run (`analysis/resume/z0_resume.txt`)

| block | time | passes | timed (originals + re-runs) | warm-ups | invalid (driver / independent) | unclean | slots used / dropped | pass-cells, K < 3 |
|---|---|---|---|---|---|---|---|---|
| C4-CGU | 03:40–03:58 | 3 of 3 | 113 (90 + 23) | 6 | 0 / 0 | 23 | 90 (20 re-runs) / 0 | 30, **0** |
| C4-BR | 04:00–04:26 | 1 of 1 (K 3 by ruling (b)) | 9 (9 + 0) | 0 | 0 / 0 | 0 | 9 / 0 | 3, 0 |
| C4-G4 | 04:28–05:38 | 3 of 3 | 9 (9 + 0) | 0 | 0 / 0 | 0 | 9 / 0 | 3, 0 |
| C4-G4-kd | 05:41–05:49 | 1 of 1 (its own K 3) | 3 (3 + 0) | 0 | 0 / 0 | 0 | 3 / 0 | 1, 0 |

**Checks:**
- **Run records.** The run wrote 185 records: 140 process, 37 passcell and 8 pass_done. They match the driver's counters (timed 134, re-runs 23, warm-ups 6, 0 voided processes) and its line "binaries after all match".
  - My K equals the driver's `k_clean` on 37 of 37 pass-cells.
  - The deepest re-run is 2, all in CGU.
- **Binaries.** All 140 process records carry their key's pin, which also equals SHA256SUMS:
  - tip 61 × `8d6e7d41`, tipcgu1 58 × `3e1f72ba`;
  - g4rT 15 × `19c9eb1f`, g4r8b 3 × `b887850f`, g4r7 3 × `f96a9c11`.
  - Re-hashing the five bin/ files now gives the pins.
- **Payload.**
  - CGU: 119 of 119 processes, warm-ups included, have pose `0x30c5438bc6ad9ffa`, bytes equal to JT500 (64,480 B), void_steps 0 and exit 0.
  - Criterion: ids 8/8, 36/36 and 6/6 appear on stdout and on disk, with none extra. All are Flat × 10 samples, and the printed values and the driver's `est_ms` agree with `sample.json` to ≤ 4.7e-5.
  - The G4 kernel receipts (equal pair counts across kernels, kd builds only on kd) hold in every process.
- **Idle rule and disk.** All 8 of 8 waits reached quiet in exactly 3 polls (130–131 s each). D: showed 107.79 GB free at every poll.
- **Receipts of used processes** (median, %):

  | block | before | after | witness |
  |---|---|---|---|
  | CGU | 1.15 | 1.14 | 0.48 |
  | BR | 2.12 | 2.12 | 1.22 |
  | G4 | 2.12 | 1.73 | 1.04 |
  | kd | 2.47 | 2.32 | 1.23 |

  - The PDH clock witness in BR read 128.8–129.2 % in every process.
- **Uncleanness.** All 23 unclean processes are CGU originals, and the top other process was `claude.exe` in every one. The owner's other apps did not appear.
  - Used re-runs read 0.9990× the originals of their cell (10 of 20 above 1), so there is no re-run bias. C4-AB showed +3.8 %.
- **The aborted first attempt.** C4-BR's 8 records of run 191354 have no `pass_done` and are excluded. `--resume` re-ran that pass whole, as the protocol says.
- **Pre-registration.**
  - The resume manifest (`raw/manifest_1790815060.json`, 03:37:40) carries the CGU claim sentence.
  - The primary window [100,500) is fixed only in PREP.md's orchestrator addendum. Its mtime (03:20:47) is 19 min before the first CGU process (03:40:01), so it predates every datum. The manifest's block note still reads "names no primary one".
  - This is moot: both windows give the same verdicts.
- **The estimates were upper bounds, as the audit read them.** BR took 25.5 min against 50.9 estimated, and kd about 8 against 17.9.
- **Deviations.**
  - C4-BR ran at K 3 and is DIAGNOSTIC ONLY (ruling (b)).
  - C4-G4-kd ran at its own K 3 and does not gate.
  - The CGU addendum fixes the primary window.
  - The PREP addendum corrects the cause of the 22:16 abort: it was the disk guard (D: 0.1 GB), not CPU load.

**What weakens these blocks:**
1. **CGU's resolution at W8/W16.** The spread has two levels, and both members of an adjacent pair fall in the same level. So the unpaired i bar is 11.22 % at W8 (10.18 % at W16), against a paired-ratio IQR of 0.71 %.
   - Ruling 1 could only have claimed a W8 gain above about 11 %.
   - The paired reading below (post hoc) bounds any real effect to about 1 %.
2. **Cross-run drift (post hoc).** This is the W ≥ 8 shift of about 5 % between the two runs, described in R5.
3. **kd is cross-block with n = 3.** kd ran in its own block after G4. Pass drift in G4 is at most about 1.5 % per arm.
4. **G4 runs under the `bench` profile** (cgu 1, no LTO), while the shipped build is fat LTO. BR shows that this binary property moves the 144 crossover (R4.4).

No git worktree was edited (`D:/wt/lighttable` is clean at `50e31f1a`). Nothing was built or timed.

### R3. Pre-registered verdicts

**C4-CGU.**
- A = `tip` (cgu 16), B = `tipcgu1` (cgu 1). B/A < 1 means cgu1 is faster.
- n = 9 per cell (3/3/3). Flags are i/s/r; "nnn ×4" means pooled and p0/p1/p2.
- The primary window is [100,500).

| claim | bar | A → B, ms (median) | B/A | pooled bars 2i / 2s / 2r % | flags | verdict |
|---|---|---|---|---|---|---|
| W1 | ruling 1, one sign | 4.3096 → 4.2956 | 0.9967 (−0.33 %) | 3.66 / 1.08 / 7.51 | nnn ×4 | NOT CLAIMED |
| W2 | " | 2.9569 → 2.9725 | 1.0053 (+0.53 %) | 4.05 / 1.30 / 9.68 | nnn ×4 | NOT CLAIMED |
| W4 | " | 2.2015 → 2.1993 | 0.9990 (−0.10 %) | 3.53 / 0.99 / 6.68 | nnn ×4 | NOT CLAIMED |
| **W8** | " | 1.8884 → 1.8699 | 0.9903 (−0.97 %) | **11.22** / 2.78 / 16.77 | nnn ×4 | **NOT CLAIMED** |
| W16 | " | 2.1576 → 2.1705 | 1.0060 (+0.60 %) | 10.18 / 2.51 / 15.84 | nnn ×4 | NOT CLAIMED |
| CGU-W8 | cgu1 claimed faster at W8 | — | — | — | — | **NOT CLAIMED** |
| CGU-NOSLOWER | cgu1 not claimed slower at any W | — | — | — | 0 of 5 claimed slower | **HOLDS** |
| **CGU-PROFILE** | CGU-W8 AND CGU-NOSLOWER | — | — | — | — | **NOT CLAIMED, so PROFILE MOVES TO cgu 1: NO** |
| [0,500), shown beside, decides nothing | same rule | — | 0.9919 / 1.0010 / 1.0026 / 1.0032 / 1.0105 | i 2.99 / 3.41 / 2.70 / 9.86 / 8.42 | nnn everywhere | NOT CLAIMED ×5 |
| VOID / REFUTED | sha pin, pose = JT500, exit, void | 119 processes | — | — | 0 failures | none / none |

**C4-G4.**
- A = tree, B = all_pairs; ratio = all_pairs/tree.
- n = 9 per arm per cell (3/3/3). Both arms come from the same processes.
- The rule is the pre-registered recipe (`lane_rules.C4-G4`) under ruling 1.

| claim | uniform: ratio (pooled 2i / 2s / 2r %) | disparity: ratio (pooled 2i / 2s / 2r %) | flags | verdict |
|---|---|---|---|---|
| all_pairs faster, 96 / 104 / 112 / 120 | 0.6553 / 0.7455 / 0.8031 / 0.8800 (at 120: 1.13 / 0.35 / 2.22) | 0.6555 / 0.6935 / 0.7402 / 0.8140 (at 120: 2.50 / 0.77 / 5.23) | YYY ×4 at every size, min-max separated | **STRONG CLAIMED** ×8 |
| **128** | 1.0067 (+0.075 µs; 1.42 / 1.07 / 6.50) | 0.9758 (−0.335 µs; 3.00 / 1.15 / 8.54) | uniform nnn ×4; disparity pooled nYn, p0 YYY, p1 nnn, p2 YYY | **NOT CLAIMED** (both), so LO = 128 |
| **tree faster, 136** | 1.0500 (+0.599 µs; 1.91 / 0.58 / 3.73) | 1.0537 (+0.763 µs; 1.09 / 0.49 / 4.23) | YYY ×4, separated | **STRONG CLAIMED**, so HI = 136 |
| tree faster, 144 / 152 / 160 | 1.1164 / 1.1773 / 1.2517 | 1.0793 / 1.1265 / 1.1901 | YYY ×4, separated | **STRONG CLAIMED** ×6 |
| **Recipe** | LO 128 / HI 136, monotone | LO 128 / HI 136, monotone | no edge fired; LO ≥ T and LO < HI hold | **T = 128, AUTO = 128/136: RE-READ, equal to the provisional values, which STAND** |
| Window 7's 144/152 | all_pairs STRONG claimed slower at 136 and 144 | same | — | **REFUTED** (and under window 7's own r-AND-s rule) |
| VOID | 9 processes, 3 closed passes | — | — | none |

**C4-G4-kd.**
- It is RECORDED, not gating: its own K 3, one pass, and a separate block.
- Each row compares kd (n = 3) against G4 pooled (n = 9). Flags are i/s/r.

| cell | kd µs | kd/tree | kd/all_pairs | reading |
|---|---|---|---|---|
| uniform 96 | 8.788 | 0.9653 YYn | 1.4730 YYY | kd beats tree net; all_pairs beats kd by 47 % |
| uniform 112 | 10.099 | 1.0081 nnn | 1.2553 YYY | no difference from tree |
| uniform 128 | 11.336 | 1.0123 YYn | 1.0055 nnn | kd not faster than either |
| disparity 96 | 10.293 | 0.9469 YYn | 1.4445 YYY | kd beats tree net; all_pairs beats kd by 44 % |
| disparity 112 | 11.823 | 0.9416 YYn | 1.2721 YYY | kd beats tree net; all_pairs beats kd by 27 % |
| disparity 128 | 13.915 | 1.0083 nnn | 1.0333 YYn | kd not faster than either |

**C4-BR.**
- It is DIAGNOSTIC ONLY (ruling (b)), K 3, one pass, n = 3 per cell. Nothing is claimed.
- The four ratios are uniform 144 / uniform 256 / disparity 144 / disparity 256.

| comparison | ratios (4 ids) | flags | diagnostic reading | verdict |
|---|---|---|---|---|
| BR-1 all_pairs g4r8b/g4r7 | 1.1302 / 1.1184 / 1.1200 / 1.1145 (median 1.1192) | YYY 4/4, separated 4/4 | binary term | NOT CLAIMED |
| BR-2 all_pairs g4rT/g4r7 | 1.1289 / 1.1183 / 1.1239 / 1.1177 (1.1211) | YYY 4/4, separated 4/4 | binary term, carried into the tip | NOT CLAIMED |
| BR-3 all_pairs g4rT/g4r8b (no pre-registered rule) | 0.9988 / 1.0000 / 1.0035 / 1.0029 | nnn 4/4 | same | NOT CLAIMED |
| BR-C tree g4r8b/g4r7 (control) | 1.0093 / 0.9934 / 1.0218 / 1.0270 | 2 of 4 resolved | small, ≤ +2.7 % | NOT CLAIMED |
| BR-C tree g4rT/g4r7 (control) | 1.0092 / 1.0038 / 1.0265 / 1.0239 | 1 of 4 resolved | small, ≤ +2.7 % | NOT CLAIMED |
| Cut §5.1's "window term" branch | — | — | does not apply | — |
| M9 verdict (b) | — | — | **CONSISTENT, not proven** | — |
| VOID | 9 of 9 valid and clean | — | — | no |

**Supporting BR readings:**
- g4r7 reproduces its own window-7 cells within 0.2 % (all_pairs 1.0002).
- Of 8b's cross-window shift (1.1348 on these ids), 85–94 % is the binary term. The rest is a window term of about 1.2 % on 8b's side.
- The static placement receipt, run by the BR analyst and untimed:
  - g4rT's `all_pairs_into` sits at g4r8b's RVA exactly (loop head 32 mod 64, 4 cache lines);
  - g4r7's loop head is at 0 mod 64 (3 lines).

### R4. Decisions

1. **Build profile: it stays at codegen-units 16,** by the claim's own rule. Owner ruling 2026-10-01 #3 moves the profile only on a claimed gain, and none was claimed.
   - Post hoc, the paired readings agree and are not limited by the rule's resolution. Over n = 45 same-round pairs the geometric mean of cgu1/cgu16 is 0.9996, and cgu1 is faster in 21 of 45.
   - At W8 the paired median is 1.0022, with cgu1 faster in 3 of 9.
   - cgu 1 costs about 12 % more build time (n = 1, measured under another lane's load) and buys nothing measurable. Owner Q4 of section 5 is closed by this measurement.
2. **Tree thresholds: keep 128/136, which are now MEASURED.**
   - The source is window 9a G4: run 033740, g4rT = `50e31f1a` + g4ref, `bench` profile, K 9, ruling 1. The C4 lane at `50e31f1a` is what merged as `d6521a43`.
   - **The one-constant commit on the trunk becomes doc-only.** No value changes and no pose or census pin moves. G-TH1 needs no re-scene, because T is unchanged.
     - Drop the PROVISIONAL marks at `broadphase_policy.rs:230-238` and `broadphase_tree/mod.rs:192` (integ/unified), citing 9a G4.
     - Rename `tree_band_and_brute_threshold_are_window_eight_b_provisional` (`broadphase_policy.rs:274`).
     - Add the 9a line to `07-C4-RECORD.md`.
   - **Rulings W8B-5 and 2026-10-01 #4.** Their re-read leg ("re-read G4 at 96–128") is met. #4 said "PROVISIONAL until window 9b's C4-G4 re-read", and the re-read has now run in 9a's resume. Lifting the mark now, rather than in 9b, is the orchestrator's call; I recommend it.
3. **F3 (tree_kd): FREEZE-AND-REMOVE,** by ruling 4's own clause ("if C4 does not use it, freeze and remove"; cut Q8).
   - C4 never selects kd: only `--bp-kernel leaflist-kd` reaches it.
   - Below T the shipped default runs all_pairs, which beats kd by 25–47 %. At 128 kd is not faster than either arm.
   - Above T, window 8b's same-process rows show kd claimed slower at 144–1000.
   - Do it as a separate commit: an annotated tag on the kd commit plus a registry line, as for S7. Suggested return condition: a kd build form that claims t_qb lower at J W1 and is not slower at W8.
4. **BR diagnosis: a binary term, so no fix lane opens.**
   - M9 found no code cause, so cut Q4's recommendation applies: rule the constants on the tip as measured, which decision 2 does.
   - **Cut Q2's pre-registered revisit trigger fires.** Q2 says "Revisit only if M9's verdict is (b) and C4-BR shows a large binary effect", and both now hold.
     - The 128/136 crossover is a property of g4rT's placement class of `all_pairs_into`. In g4r7's class the 144 crossover sits at about 1.00.
     - The shipped fat-LTO runner's placement is unmeasured.
   - By arithmetic, the other placement would move the constants by about 2 grid steps, worth under 1 µs per broadphase step. A `bench-shipped` second reading is the orchestrator's call.
   - My recommendation: commit the constants now, since they are value-neutral and can be re-derived. Run a shipped-profile reading only if 9b carries the aligned-loop-head twin below anyway.

### R5. What this changes for window 9b and for the Jolt/Rapier standing

**Window 9b contents** (ruling 2026-10-01 #5):
- **Drop C4-BR, C4-G4 and C4-G4-kd.** They are measured. This frees about 152 scheduled minutes (104 actual).
- **Drop the cgu 1 twin.** It already gave NO on the pre-V2 tip. Re-measure only if the orchestrator wants it on the V2 trunk, whose solver hot loop changes.
  - Under ruling 1's unpaired bars, a re-run cannot claim a W8 effect below about 11 %, and the paired data bound the effect to about 1 %.
  - So a re-run is worth its minutes only with a paired statistic pre-registered beside ruling 1. That is a rule change, so it is the orchestrator's call.
- **Keep the rest:** V2-AB/SPAN/RES with Jolt 5.6 in-block, Rapier's fastest holding configuration adjacent to our rows, SR/S5 A/B when they exist, and an armed C4-JD W16 row for S5.
- **Optional diagnostic (BR post hoc 8; orchestrator's call; low priority).** A g4rT twin whose `all_pairs_into` loop head is forced to 0 mod 64 would turn M9 (b) from "consistent" into "shown".
  - On the shipped default `all_pairs_into` runs only for worlds of 128 rows or fewer (about 11 µs per step at 128). The stake is J-A/cfg-A's AllPairs, and only if the shipped runner has the slow placement.
  - If it runs, fold Q2's bench-shipped G4 reading into the same block.
- **Every A/B in 9b must stay in-block.** The resume read the same tip exe +5.62 % (W8) and +5.32 % (W16) slower than the first run ([100,500), post hoc; neither run claims a difference).
  - The shift is spread across the step distribution: at W8, p10 +1.9 %, p50 +4.5 %, p90 +10.3 %.
  - Process CPU/wall time fell from 4.59 to 3.64, with the same power plan and a lower witness. So the cause is a term no receipt catches.
  - 9a's T(8) = 1.7878 ms ([100,500)) is therefore not a baseline for SR, S5 or V2 bars across runs. Those bars must be formed in-block (ruling 2026-09-30 #8 already fixes SR's bar at window prep).

**The standing:**
- **Jolt 5.6.** The ratios in section 1 are in-block (C4-AB, first run), so the resume changes nothing there.
- **Rapier 0.36.** The numbers do not change. PC-9a-6 (the build condition) is resolved on our side:
  - our runner is insensitive to codegen-units within about 1 %, so building ours at cgu 1 like Rapier would move no ours/Rapier ratio by more than about 1 %;
  - the cgu question concerned our rows only, and both sides are x86-64-v3;
  - Rapier's own sensitivity to cgu is unmeasured. Under owner ruling 2026-10-01 #1 ("fastest vs fastest") Rapier keeps its harness's build (cgu 1) unless its configuration search shows a faster one.
- **The drift sets a scale for cross-block readings (post hoc).** If a term of about 5 % also exists between blocks of one run, it does not decide Rapier's lead at W8/W16 (ours/Rapier-default 1.27–1.30). It could decide W2/W4 (0.987 / 1.073). That supports ruling #5's requirement of in-block, adjacent Rapier rows in 9b.

### R6. Points for the orchestrator

- **PC-9a-7.** The doc-only constant commit (R4.2), and F3's freeze-and-remove as a separate commit (R4.3).
- **PC-9a-8.** Q2 reopened: whether to take a bench-shipped G4 reading (R4.4).
- **PC-9a-9.** Remove C4-BR, C4-G4, C4-G4-kd and the cgu twin from 9b's list. Pre-register in-block baselines (R5).
- **PC-9a-10 (desktop).** `claude.exe` was the only source of uncleanness: 23 of 113 CGU processes, while the criterion blocks had 0 of 21. Agent sessions should be idle during a window, including the orchestrator's polling.
- **PC-9a-11 (path correction, no number changes).** The G4 group's `.txt` outputs (`g0_validity.txt` … `g5_robust.txt`) were written to `analysis/c4ab/`, not `analysis/g4/`, because `lib9a.Out.save` writes beside `lib9a.py`. The `.py` and `.json` files are in `analysis/g4/`.
- **PC-9a-12 (cleanup).** `D:/wt/_targets/w9a-tipcgu1`, `w9a-tipctl` and `w9a-trees2` (about 1.0 GB) are still present. The window is complete, so PREP's cleanup applies.

### R7. Scripts and outputs (resume)

| location | content |
|---|---|
| `analysis/resume/z0_resume.py` → `z0_resume.txt` | Census of run 033740: validity, slots, K against the driver, receipts, idle waits, D:, sha. A re-derivation of every decisive figure above (CGU per W in both windows plus the paired readings; G4's 18 cells and the recipe; the kd ratios; the BR ratios) and the cross-run comparison. It reuses the verified group libraries unchanged. **It agrees with all three group reports on every figure quoted.** |
| `analysis/cgu/` (c0–c4, v0; verify/ x1–x6) | Group C4-CGU, verified AGREE. |
| `analysis/g4/` (`.py`, `.json`; the `.txt` files are in `analysis/c4ab/`; verify/ v1–v2) | Group C4-G4 + kd, verified AGREE. |
| `analysis/br/` (br0–br3; verify/) | Group C4-BR, verified AGREE. |

All paths are relative to `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a/`. Nothing was built or timed, and no git worktree was edited.