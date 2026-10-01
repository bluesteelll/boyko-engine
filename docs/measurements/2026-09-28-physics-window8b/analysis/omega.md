# Physics window 8b - omega group: omega_b v2, omega(W, gap) + N4, v1 continuity (results-analyst, 2026-09-29)

omega_b v2 is valid in every cell: no void rule fired. At P = 8, b = 4P the pre-registered S1 input (the slope, which in
v2 includes 4 blocks of ~0.57 us work) reads 3.35 / 3.37 us a stage on the worker route (spin / park) and 3.14 / 3.18 us
external. Net of the work (slope - b x work) the stage cost is 0.86-1.07 us at b = 4P, 0.50-0.68 at 2P and 0.35-0.42 at 1P.
v1 re-read in the same window on the same exe gives 2.99 us at P8 worker (window 8: 3.00; 8b K = 2), so the ~3x drop
is the bench change, not the machine. Spin-then-park costs what spin costs at back-to-back stages at P8: no difference
resolved at any b on either route. After a serial stretch of 20 or 80 us, parking costs +6.0 to +6.3 us a stage, CLAIMED
STRONG with pass 0 at K = 2 (U1). omega(W, gap): waking the pool costs +6.3 / +6.6 us (W8 worker / external) and +9.6 /
+15.6 us (W16) over the gap-5 awake baseline. The first helper arrives 3.4-3.7 us after the scope opens, against
0.3-0.9 us when the pool is awake. All of these are STRONG.

Scripts `analysis/omega/{lib_om,o0_select,o1_v2,o1b_guard,o1c_dropped,o2_wgap,o3_v1cont,o4_s1,o5_p8detail,o6_dirty,
o7_tables,o8_extra}.py`; outputs `analysis/omega/o0..o8.txt`, `o1_per_process.json`. Bench source read at a3adc827
(`git show`, read-only).

## Method
- Source: `raw/runs.jsonl` + each process's own `stdout.txt`. SUMMARY and CALIBRATION lines were re-parsed with
  `tools/micro8b.parse`. Validity = micro8b's rules re-applied (expect_*, void_rules, expect_present), plus exit 0 and
  the omega2 sha256 pin: **0 invalid of 95 timed processes**. The driver's valid and contaminated flags agree with
  mine 95/95 (o0).
- Clean = window 8's rule: receipts <= 5 %, witness <= 2 %, no build process. Slot = the original if valid and clean,
  else its re-run if valid and clean, else dropped (rows8b protocol: "a slot whose re-run is also hot is dropped by
  the reduction"). **66 slots, 60 used (37 originals, 23 re-runs), 6 dropped.** omega-v2 dropped p0 r1
  omega2-gap-external and p0 r2 omega2-gap-worker, so the gap rows have n = 8 with p0 K = 2. omega-v1-cont dropped 4 of
  12, so every v1 cell has K = 2. Placement receipts are recorded (o0) and were never used.
- omega_b2 per process = (region_ns_median@72 - region_ns_median@36) / 36. Both values come from the same process
  (v2 prints 36 and 72 stages in one run). Cell = median over K. Statistics as in window 8's lib8:
  - r = (max - min) / median;
  - i = IQR (inclusive quartiles) / median;
  - s = 1.2533 x SD / sqrt(K) / median.
- A B-vs-A difference clears a spread when |B/A - 1| > 2 x hypot(spread_A, spread_B).
- **Ruling 1:** CLAIMED iff i AND s clear pooled (K <= 9) AND in each of p0/p1/p2 (K <= 3), with the same sign.
  STRONG if r also clears everywhere. The flags are printed r/i/s.
- ns throughout. All 952 region and scope medians are multiples of 100 ns, the QPC tick (o8).

## Verdict table

| claim id | pre-registered bar | cells + n | numbers (ns): median [IQR, SE, min-max] | i/s/r | verdict |
|---|---|---|---|---|---|
| V1 void (cut.md "omega_b v2 rows (W5)", amended) | work_ns_calibrated in [500, 1000]; exactly_once; lost_wakeups 0; a park row at gap > 0 with parks 0 on every rep = void | 44 configs x 2 stage counts; 34 used processes, 776 SUMMARY + 34 CALIBRATION lines | work_ns 568.6-577.6; exactly_once true and lost_wakeups 0 on every line. The gap-20/80 park rows parked exactly (P-1) x stages = 252 / 504 times a region on every rep | - | **NO VOID** (0 cells, on used reps and on every rep) |
| V2 gap-0 park rows | reported "no park taken", not void | 36 (config, stages) cells | 35 of 36 took no park on any used rep. The exception: worker P8 b=4P at 72 stages parked 2 (one rep 3) a region on every rep. Over every rep incl. dirty ones: 33 no-park, 2 mixed | - | reported |
| S1-IN omega_b2(8, 4P) | none (the S1 input) | 4 cells, n = 9 each | worker spin 3353 [111, 49, 3247-3608]; worker park 3369 [94, 63, 3192-3692]; external spin 3144 [403, 103, 2922-3647]; external park 3183 [261, 104, 3003-3694] | - | MEASURED |
| PART (receipt: barrier paid by all P?) | none | P8 gap-0 cells n = 9; gap cells n = 8 | gap 0: active_median 8 in every cell; all_active_reps >= 997 of 1000. Park at gap 20: active_median 7 (worker 7 / 7.5); all_active_reps median 294-322 (36 st) and 415-493 (72 st). Park at gap 80: 798-973 | - | gap 0: **all P**. Park at gap 20: **not all P** (the median region ran on 7 of 8) |
| PARK-0 (09-26 ruling 6 at back-to-back stages; W5: "costs what spin-only costs") | ruling 1 on the park vs spin slope | 18 pairs (P 2/4/8 x b 1/2/4P x 2 routes), n = 9 each | P8: -175..+39 (-9.6..+3.3 %). P4 b=1P: worker -94 (-11.3 %), external -97 (-11.7 %). P4 b=2P worker -81 (-5.5 %). P2: -11..+6 | P8 pooled n/n/n (ext b=2P n/n/Y); P4 b1 n/Y/Y pooled, every pass i+s | **P8 6/6 NOT CLAIMED**; P2 6/6 NOT CLAIMED. P4 b=1P x 2 routes + worker b=2P: CLAIMED, **park cheaper**. Other P4 3/3 NOT CLAIMED |
| PARK-20 (park price; W5: park slope - spin slope at the same gap) | ruling 1 | P8 b=4P; worker, external; n = 8 each (p0 K = 2) | worker 23135 [147, 46, 22950-23244] -> 29157 [91, 37, 28961-29208] = **+6022 (+26.0 %)**. External 22726 [624, 167] -> 28985 [123, 31] = **+6258 (+27.5 %)** | pooled, p1, p2 Y/Y/Y; p0 Y/Y/Y at K = 2 | **CLAIMED STRONG** (U1) |
| PARK-80 | ruling 1 | same | worker 83126 -> 89388 = **+6261 (+7.5 %)**; external 83125 -> 89299 = **+6174 (+7.4 %)** | same | **CLAIMED STRONG** (U1) |
| N4-W8w / W8e / W16w / W16e: omega(80) - omega(0) (review N4: a lower bound on park->wake) | ruling 1 | per (route, W), n = 9 | W8 worker 7100 -> 9100 = +2000 (+28.2 %); W8 external 4700 -> 10900 = +6200; W16 worker 10500 -> 20700 = +10200; W16 external 5200 -> 22200 = +17000 | W8w pooled n/Y/Y, p1 n/Y/Y, p0/p2 Y/Y/Y; the rest Y/Y/Y everywhere | W8w **CLAIMED**; W8e, W16w, W16e **CLAIMED STRONG** |
| AW-*: omega(80) - omega(5) (window 8 section 9 item 4: gap 5 = the awake baseline) | ruling 1 | per (route, W), n = 9 | W8 worker 2800 -> 9100 = +6300; W8 external 4300 -> 10900 = +6600; W16 worker 11100 -> 20700 = +9600; W16 external 6600 -> 22200 = +15600 | Y/Y/Y pooled and in every pass | **CLAIMED STRONG x 4** |
| CONT-v1 (cut section 5 item 4, optional; window 8's N5 method) | ruling 1, 8b vs window 8 | 16 region cells + 8 slopes. 8b: one pass, K = 2 per cell. Window 8: K = 6 (recomputed from win8/raw: 36 used, equal to window 8's q5) | P8 slope: worker 2994 vs 3000 (0.998x); external 3194 vs 2825 (1.131x). P2 443 / 478 vs 342 / 403. P4 1158 / 1632 vs 1186 / 988. P16 5936 / 4931 vs 5049 / 5464 | K = 2 cannot claim under lib8's guard. With the guard waived, no region cell clears i AND s | **NOT CLAIMED** (no resolved difference; underpowered) |

## omega_b2 per config (o1, o7; K = 9, gap rows 8; "omega_b" = slope - b x work_ns_calibrated - gap, per process then median; active / first helper / parks = cell medians of the SUMMARY receipts, 1000 regions a process)

| route | P | b | gap us | spin slope med [min-max] n | park slope med [min-max] n | omega_b spin / park (slope - b x work - gap) | active 36/72 (spin; park) | first helper ns 36/72 (spin; park) | parks/region 36/72 (park) |
|---|---|---|---|---|---|---|---|---|---|
| worker | 2 | 1P | 0 | 667 [650-672] 9 | 664 [656-678] 9 | 94 / 93 | 2/2; 2/2 | 600/500; 600/500 | 0 [0-0] / 0 [0-0] |
| worker | 2 | 2P | 0 | 1267 [1222-1275] 9 | 1269 [1225-1292] 9 | 120 / 126 | 2/2; 2/2 | 500/500; 500/500 | 0 [0-0] / 0 [0-0] |
| worker | 2 | 4P | 0 | 2467 [2394-2486] 9 | 2467 [2392-2478] 9 | 179 / 185 | 2/2; 2/2 | 500/500; 500/500 | 0 [0-0] / 0 [0-0] |
| worker | 4 | 1P | 0 | 836 [814-872] 9 | 742 [731-767] 9 | 265 / 171 | 4/4; 4/4 | 500/600; 500/600 | 0 [0-0] / 0 [0-0] |
| worker | 4 | 2P | 0 | 1464 [1433-1511] 9 | 1383 [1339-1397] 9 | 318 / 243 | 4/4; 4/4 | 600/600; 600/500 | 0 [0-0] / 0 [0-0] |
| worker | 4 | 4P | 0 | 2772 [2678-2806] 9 | 2769 [2669-2950] 9 | 491 / 476 | 4/4; 4/4 | 600/2000; 500/1600 | 0 [0-0] / 0 [0-0] |
| worker | 8 | 1P | 0 | 919 [861-1119] 9 | 950 [811-1089] 9 | 346 / 378 | 8/8; 8/8 | 800/800; 700/800 | 0 [0-0] / 0 [0-0] |
| worker | 8 | 2P | 0 | 1742 [1597-1894] 9 | 1703 [1650-1914] 9 | 598 / 554 | 8/8; 8/8 | 800/900; 800/800 | 0 [0-0] / 0 [0-0] |
| worker | 8 | 4P | 0 | 3353 [3247-3608] 9 | 3369 [3192-3692] 9 | 1059 / 1073 | 8/8; 8/8 | 900/7500; 800/7500 | 0 [0-0] / 2 [2-3] |
| worker | 8 | 4P | 20 | 23135 [22950-23244] 8 | 29157 [28961-29208] 8 | 844 / 6865 | 8/8; 7/7.5 | 27150/27950; 24100/30450 | 252 [252-252] / 504 [504-504] |
| worker | 8 | 4P | 80 | 83126 [83019-83172] 8 | 89388 [89281-89656] 8 | 832 / 7095 | 8/8; 8/8 | 87850/88250; 84650/90850 | 252 [252-252] / 504 [504-504] |
| external | 2 | 1P | 0 | 672 [633-894] 9 | 675 [656-836] 9 | 100 / 103 | 2/2; 2/2 | 1200/1200; 1100/1100 | 0 [0-0] / 0 [0-0] |
| external | 2 | 2P | 0 | 1267 [1231-1278] 9 | 1256 [1225-1275] 9 | 118 / 109 | 2/2; 2/2 | 1100/1200; 1100/1100 | 0 [0-0] / 0 [0-0] |
| external | 2 | 4P | 0 | 2461 [2386-2481] 9 | 2467 [2369-2469] 9 | 168 / 170 | 2/2; 2/2 | 1200/1200; 1100/1100 | 0 [0-0] / 0 [0-0] |
| external | 4 | 1P | 0 | 833 [783-856] 9 | 736 [714-742] 9 | 261 / 162 | 4/4; 4/4 | 1300/1300; 1200/1200 | 0 [0-0] / 0 [0-0] |
| external | 4 | 2P | 0 | 1453 [1272-1475] 9 | 1381 [1319-1389] 9 | 307 / 230 | 4/4; 4/4 | 1300/1300; 1200/1200 | 0 [0-0] / 0 [0-0] |
| external | 4 | 4P | 0 | 2678 [2589-2694] 9 | 2656 [2569-2681] 9 | 386 / 369 | 4/4; 4/4 | 1300/1300; 1200/1300 | 0 [0-0] / 0 [0-0] |
| external | 8 | 1P | 0 | 989 [931-1064] 9 | 972 [922-1153] 9 | 416 / 400 | 8/8; 8/8 | 1400/1500; 1400/1500 | 0 [0-0] / 0 [0-0] |
| external | 8 | 2P | 0 | 1822 [1761-1986] 9 | 1647 [1569-1742] 9 | 679 / 502 | 8/8; 8/8 | 1500/1700; 1400/1800 | 0 [0-0] / 0 [0-0] |
| external | 8 | 4P | 0 | 3144 [2922-3647] 9 | 3183 [3003-3694] 9 | 857 / 878 | 8/8; 8/8 | 1700/2100; 1700/2000 | 0 [0-0] / 0 [0-0] |
| external | 8 | 4P | 20 | 22726 [22556-23508] 8 | 28985 [28883-29056] 8 | 443 / 6705 | 8/8; 7/7 | 22050/22200; 25000/25350 | 252 [252-252] / 504 [504-504] |
| external | 8 | 4P | 80 | 83125 [82717-83508] 8 | 89299 [89206-89414] 8 | 842 / 7012 | 8/8; 8/8 | 82450/82750; 85400/85750 | 252 [252-252] / 504 [504-504] |

## omega(W, gap) with the N4 receipt (o2, o7; 32 zero-work tasks, 2000 reps a process, K = 9)

| route | W | gap us | scope ns med [IQR, SE, min-max] n | helped_reps of 2000 [min-max] | first_helper_ns_median med [min-max] |
|---|---|---|---|---|---|
| worker | 8 | 0 | 7100 [IQR 600, SE 155, 6400-7400] n=9 | 1992-2000 | 400 [300-400] |
| worker | 8 | 5 | 2800 [IQR 300, SE 300, 2600-4900] n=9 | 1998-2000 | 900 [600-900] |
| worker | 8 | 20 | 9300 [IQR 200, SE 218, 8800-10700] n=9 | 1998-2000 | 3400 [3400-3500] |
| worker | 8 | 80 | 9100 [IQR 400, SE 215, 8800-10500] n=9 | 2000-2000 | 3500 [3500-3600] |
| worker | 16 | 0 | 10500 [IQR 400, SE 115, 9900-10800] n=9 | 2000-2000 | 400 [400-400] |
| worker | 16 | 5 | 11100 [IQR 300, SE 152, 10200-11400] n=9 | 1992-2000 | 400 [400-400] |
| worker | 16 | 20 | 20600 [IQR 200, SE 310, 20200-22700] n=9 | 2000-2000 | 3600 [3600-3900] |
| worker | 16 | 80 | 20700 [IQR 300, SE 140, 20300-21300] n=9 | 2000-2000 | 3700 [3700-3700] |
| external | 8 | 0 | 4700 [IQR 0, SE 72, 4500-5100] n=9 | 1997-2000 | 300 [300-400] |
| external | 8 | 5 | 4300 [IQR 200, SE 122, 3700-4600] n=9 | 1999-2000 | 900 [700-1000] |
| external | 8 | 20 | 11000 [IQR 200, SE 47, 10800-11100] n=9 | 1999-2000 | 3500 [3400-3500] |
| external | 8 | 80 | 10900 [IQR 100, SE 42, 10800-11100] n=9 | 2000-2000 | 3600 [3500-3600] |
| external | 16 | 0 | 5200 [IQR 100, SE 47, 5000-5300] n=9 | 1997-2000 | 300 [300-300] |
| external | 16 | 5 | 6600 [IQR 200, SE 57, 6400-6800] n=9 | 2000-2000 | 400 [400-400] |
| external | 16 | 20 | 21600 [IQR 300, SE 233, 20800-22700] n=9 | 2000-2000 | 3600 [3600-3700] |
| external | 16 | 80 | 22200 [IQR 1000, SE 291, 21400-23500] n=9 | 2000-2000 | 3700 [3700-3800] |

Wake latency as a function of the gap: the first helper starts 0.3-0.4 us after the scope opens at gap 0 (both W, both
routes). At gap 5 it is 0.9 us at W8 and 0.4 us at W16. At gap 20 it is 3.4-3.6 us, and at gap 80 3.5-3.7 us.
helped_reps is 1992-2000 of 2000 in every cell, so every scope was helped. The first-helper step 5 -> 80 is CLAIMED
STRONG in all four (route, W) cells (post hoc test, o2). omega(80) - omega(20) is NOT CLAIMED in any cell, so the pool
has parked before 20 us of idleness.

## v1 continuity (o3; slope = (cell region@72 - cell region@36) / 36, window 8's N5; ns)

| route | P | w8 region36 / 72 (K 6) | 8b region36 / 72 (K 2) | slope w8 | slope 8b [worst pairing] | 8b / w8 | paired per-round slope w8 / 8b (n) |
|---|---|---|---|---|---|---|---|
| worker | 2 | 16000 / 28300 | 15250 / 31200 | 341.7 | 443.1 [394-492] | 1.297 | 353 / 443 (6 / 2) |
| worker | 4 | 39900 / 82600 | 44500 / 86200 | 1186.1 | 1158.3 [889-1428] | 0.977 | 1079 / 1158 (6 / 2) |
| worker | 8 | 118650 / 226650 | 110400 / 218200 | 3000.0 | 2994.4 [2939-3050] | 0.998 | 3453 / 2994 (6 / 2) |
| worker | 16 | 217450 / 399200 | 210800 / 424500 | 5048.6 | 5936.1 [5225-6647] | 1.176 | 5311 / 5936 (6 / 2) |
| external | 2 | 15350 / 29850 | 16500 / 33700 | 402.8 | 477.8 [447-508] | 1.186 | 407 / 486 (6 / 1) |
| external | 4 | 41600 / 77150 | 41450 / 100200 | 987.5 | 1631.9 [1400-1864] | 1.653 | 1011 / 1400 (6 / 1) |
| external | 8 | 110500 / 212200 | 108150 / 223150 | 2825.0 | 3194.4 [3092-3297] | 1.131 | 2974 / 3211 (6 / 1) |
| external | 16 | 198100 / 394800 | 222950 / 400450 | 5463.9 | 4930.6 [4622-5239] | 0.902 | 5535 / 4622 (6 / 1) |

No 8b-vs-w8 region cell or paired slope is claimed. With lib8's K >= 3 guard waived, some cells clear s alone and one
clears i and s (external P4 region72, n/Y/Y). The v1 bench still reads ~3.0 us a stage at P8 in 8b.

## Implied decisions: inputs for S1 (W8 ruling 5), S2i (W8 ruling 6) and spin vs park (09-26 ruling 6). Arith., not the decision (o4, o8)
1. **The per-stage barrier cost at P8.** omega_b is the design's quantity: stage cost beyond ideal parallel work, here
   slope - b x work, derived post hoc.
   - 0.35-0.42 us at b = 1P, 0.50-0.68 us at 2P, 0.86-1.07 us at 4P (both routes, both helpers).
   - That is inside the design's 0.3-1.0 us bracket at b <= 2P, and at its top at 4P.
   - It is 3.1-9.5x below today's per-wave omega(8) = 3.30 us (window 8).
   - The pre-registered S1 input, the raw slope at 4P (3.14-3.37 us), contains 4 x 0.572 = 2.29 us of block work.
     Net of it, the overhead is 37-47 % of the work at b = 4P.
2. **Against the stage work S1 would replace** (window 8, W8, J-T):
   - Warm apply: 0.304 ms over 36.44 stages = 8.34 us serial a stage, 1.04 us per participant at P8 (~1.8 blocks).
     omega_b is 33-40 % of that at 1P, 48-65 % at 2P, 82-103 % at 4P.
   - Integrate group: 13.2 us a stage, 1.65 us per participant (~2.9 blocks). omega_b is 21-25 % at 1P, 30-41 % at 2P,
     52-65 % at 4P.
3. **Window 8's section-5 bundle arithmetic**, re-evaluated at these omega_b:
   - wide (3.30 - 0.3 - w - 0.7) x 96 + S2 (0.304 x 0.745 - 36.44 w) + S3 ((0.1216 - 0.0027) x 0.745 - 9 w)
     = **0.38-0.49 ms = 18.9-23.9 % of T(8)** (2.0357 ms, window 8).
   - The formula reproduces window 8's 0.112 ms = 5.5 % at w = 3.0.
   - The 5 % build-if is crossed at w = 3.07 us.
   - Plugging in the raw slope instead (not like-for-like: it holds the block work) gives 2.9-4.5 %.
4. **S2i vs S2 on S1** (W8 ruling 6, window-8 numbers):
   - S2i (warm apply on today's scope path, 36.44 waves x omega(8) 3.30 us) = 0.106 ms.
   - S2 on S1 = 0.187-0.214 ms. S2 on S1 is ahead by +0.08 to +0.11 ms at every b.
5. **Spin vs spin-then-park** (09-26 ruling 6):
   - Back-to-back stages have no measurable park price at P8 (PARK-0), and no park is taken at 36 stages.
   - A park costs +6.0 to +6.3 us per stage that follows a serial stretch of >= 20 us (PARK-20/80). At gap 20 the
     median parked region also runs on 7 of 8 participants.
   - The pool's idle budget (127 PAUSE + 4 yields) expires between 5 and 20 us of idleness (omega(W, gap) receipts).
   - Per-step price = ~6 us x the number of region-internal serial stretches longer than that budget. That count is a
     design quantity and is not measured here.
6. **Recruitment** (design rev section 5 item 3: 0-0.07 ms a step):
   - After a >= 20 us gap, the first helper arrives in 3.4-3.7 us, and the W8 scope costs +6.3 / +6.6 us over the awake
     baseline. That is about 0.003-0.007 ms per region opening (arith.).
   - Inside the region at P8: first helper 0.7-1.0 us (worker, 36 stages), 1.4-2.1 us (external). It is 7.3-7.6 us on
     the worker route at 72 stages, b = 4P (A3).
7. **Route:** worker vs external slope is NOT CLAIMED in any of the 22 (P, b, helper, gap) configs. At P4 b=4P (both
   helpers) the pooled i and s clear, but not in every pass.

## Anomalies and validity notes (o0, o5, o6, o8)
- **A1: dirty processes.** 35 of 95 timed processes failed the clean rule. The top after-receipt process was
  Telegram.exe 14 times, browser.exe 14 and claude.exe 5; the witness named browser.exe 17 and Telegram.exe 12 (o6).
  Used re-runs: omega-v2 19, omega-v1-cont 4. Receipts of the used processes: before max 4.92 %, after max 4.22 %,
  witness max 1.85 % (median 0.80 %). No pass was voided.
- **A2: omega-v1-cont ran last (21:02-21:09) on a busy desktop.** Its warm-up witness read 42 %. 4 of 12 slots were
  dropped, so every v1 cell has K = 2, from one pass. The external paired slopes rest on 1 round.
- **A3: the headline cell's recruitment differs between 36 and 72 stages.**
  - Worker route, P8 b=4P: the first helper arrives 0.7-1.0 us after the region opens at 36 stages and 7.3-7.6 us at
    72 stages. This holds in every one of the 9 processes, for both helpers (o5).
  - The external route shows 1.6-1.9 vs 1.9-2.2 us; worker P4 b=4P shows 0.5-0.6 vs 0.6-2.3 us (cell medians 1.6 / 2.0).
  - The slope method assumes the same fixed cost at 36 and 72 stages. The bound on the resulting bias is (first72 -
    first36) / 36 = **178-189 ns a stage** on worker P8 b=4P, against 3-14 ns on external (o8).
  - The worker - external slope gap at P8 b=4P (+208 spin, +186 park; NOT CLAIMED) is of that size.
  - The only gap-0 park config that parked (2 a region) is this same config.
- **A4: config order inside a process is fixed.** For each P, every 36-stage row runs before every 72-stage row. Any
  within-process drift therefore lands in the slope. Reversing p0/p2 changes the order of processes, not of configs,
  so this cannot be separated.
- **A5: calibrated work is ~0.57 us, not ~0.7 us.** work_ns_calibrated is 568.6-577.6 ns against the 700 target.
  The probe measures 15.4 ns a round at 256 rounds and picks 44-45 rounds; the check run at that count reads ~572 ns.
  This is inside the void band, but the spec's "~0.7 us per block" is not met, so b x P blocks carry 0.57 x b us of
  work a participant.
- **A6: the gap rows have K = 2 in pass 0** (dropped slots): see U1. o1c puts the dropped processes back; they sit at
  or just outside the used ranges (spin 22697-23567, park 28672-29797 at gap 20), and restoring them keeps both claims.
- **A7: the non-monotone gap 0 vs gap 5 reading repeats.** omega(W8, worker) at gap 0 (7100) is above gap 5 (2800),
  as in window 8 (7600 > 2550); NOT CLAIMED here (p1 n/n/n). The first helper is earlier at gap 0 (0.4 us) than at
  gap 5 (0.9 us), so late recruitment does not explain the gap-0 excess.
- **A8: placement receipts are present on all 60 used processes** and none was used. The worker route's main thread
  holds 0.1-1.8 % of process cycles (it waits in `install`); the external route's holds 19-20 % on omega-v2 and 8.5-9.0 % on v1-cont (it orchestrates).
  main_share_top_est is null on 4 worker-route omega-wgap processes (main-thread CPU 0 on a sub-second run).

## Post hoc observations (not pre-registered)
- **H1: P8's net stage cost grows with the block count** (o4 (1), a per-process fit slope = a + b x bpp over
  b = 1/2/4P).
  - At P8 the fixed per-stage term is a = 0.12-0.28 us, and the per-block-per-participant term is b = 0.72-0.82 us.
    That is 0.15-0.24 us above the calibrated work, against 0.02-0.03 us at P2 and 0.04-0.10 us at P4.
  - Most of omega_b at 4P is therefore per-block inflation that grows with P. Its source may be line migration of
    the claim, done or payload lines, or clock/SMT effects on the work itself; this bench cannot separate them.
- **H2: at P4 b <= 2P the "spin" helper is the slower one.** In the three CLAIMED pairs it is slower by 81-97 ns a stage
  (cell medians; paired -81..-100 ns, 0 of 9 processes positive) (o1). v2's spin variant is v1's wait (5 PAUSE, then `yield_now` every iteration); the park
  variant spins 127 PAUSE first. No park was taken there, so this is the early yield, not a park.
- **H3: the gap park price is positive in every paired process**, 8/8 on both routes: +5.4 to +6.6 us (o1).
- **H4: omega(W, gap) scope medians in 8b are within -7.1..+5.0 % of window 8's** in all 16 cells (o3), and the N4
  bound reproduces it: window 8 read +2.2 / +6.1 / +9.5 / +15.95 us, 8b reads +2.0 / +6.2 / +10.2 / +17.0.
- **H5: worker vs external region slope at P8 b=4P** is +208 ns (spin) and +186 ns (park), NOT CLAIMED. A3's bias
  bound is 178-189 ns.

## Where the pre-registered rule's application was uncertain
- **U1: the K >= 3 guard.** lib8.cmp_ refuses a claim when a cell has K < 3; neither ruling 1 nor window 8's written
  method states that. The gap rows' pass 0 has K = 2 per cell.
  - With the guard, PARK-20/80 are NOT CLAIMED on the letter, only because of pass 0.
  - Without it they are CLAIMED STRONG (o1b).
  - With the dropped processes restored (original or re-run), they are CLAIMED STRONG with the guard intact (o1c).
  - I report CLAIMED STRONG. CONT-v1 is NOT CLAIMED either way.
- **U2: what counts as "omega_b".** The pre-registered S1 input is the slope, which in v2 includes b/P x work. The
  design's omega_b is the barrier cost beyond the work. Both are reported; the S1 arithmetic uses slope - b x work, a
  post-hoc derivation.
- **U3: "every rep" in the gap-park void rule.** I read it per process and per (config, stages), as micro8b does, and
  checked both the used reps and every rep, dirty ones included. There are 0 voids under either reading.
- **U4: which "ruling 6".** The brief's "rulings 5 and 6 (spin vs spin-then-park)" refers to the 27-W8 file, whose
  ruling 6 is S2i pricing. Spin vs park is the 09-26 ruling 6, named in W8 ruling 5. Both are answered (items 4 and 5
  above).
- **U5: PARK-0's pre-registered sentence is an equality**, and ruling 1 can only claim differences. "NOT CLAIMED" at
  P8 is consistent with it but does not prove it. The P4 claims point the other way (park cheaper); I did not label
  them REFUTED.
- **U6: "clean pass".** I treated all three omega-v2 passes as clean (no voided pass) and v1-cont's single pass as its
  only pass. The per-pass test is run on whatever K each pass kept.
- **U7: ruling 1 inside omega(W, gap).** The comparisons are between cells of the same process rows (different gaps
  of one process). The two-cell rule treats them as independent, the same way window 8 did.
