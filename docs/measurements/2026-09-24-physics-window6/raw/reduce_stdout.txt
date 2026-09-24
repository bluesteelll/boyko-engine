# Window 6 reduction (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win6\raw)
553 records, 436 chosen processes, 8 dropped slots, 0 voided passes

## P1 L10 pre-C0 refutation (armed J-Son tail [264,1000), W=1; bar 0.584 ms)
- L10-Offp@W1 wall mean [264,1000) ms: 1.4670 [1.4566-1.4826] K=6 r/i/s 1.77/0.88/0.34 %
- L10-Son@W1 wall mean [264,1000) ms: 4.6973 [4.6759-4.7127] K=6 r/i/s 0.78/0.37/0.15 %
- L10-Offp@W1 sys_sum_ns mean [264,1000) ms: 1.4262 [1.4206-1.4455] K=6 r/i/s 1.75/0.82/0.35 %
- L10-Offp@W1 sys_physics_broadphase_ns mean [264,1000) ms: 0.2523 [0.2513-0.2545] K=6 r/i/s 1.29/0.43/0.23 %
- L10-Offp@W1 sys_physics_narrowphase_ns mean [264,1000) ms: 0.6442 [0.6425-0.6572] K=6 r/i/s 2.28/0.52/0.44 %
- L10-Offp@W1 sys_physics_build_graph_ns mean [264,1000) ms: 0.1160 [0.1131-0.1178] K=6 r/i/s 4.06/0.67/0.68 %
- L10-Offp@W1 sys_physics_solve_colored_ns mean [264,1000) ms: 0.3812 [0.3791-0.3885] K=6 r/i/s 2.48/0.84/0.47 %
- L10-Offp@W1 g_ns mean [264,1000) ms: 0.0367 [0.0360-0.0451] K=6 r/i/s 24.66/1.56/4.88 %
- L10-Offp@W1 wall mean over the all-frozen tail [first_frozen_step,1000) ms: 1.3043 [1.2935-1.3218] K=6 r/i/s 2.16/0.81/0.40 %
- L10-Offp@W1 first_frozen_step: 274.0000 [274.0000-274.0000] K=6 r/i/s 0.00/0.00/0.00 %
- L10-Son@W1 wall mean over the all-frozen tail ms: 4.6970 [4.6757-4.7126] K=6 r/i/s 0.78/0.38/0.15 %
- Off-prime against the lane J-Son: ratio 0.3123 (-68.77 %, -3.2302); bars r/i/s 3.87/1.91/0.75 %; claimed r/i/s Y/Y/Y -> CLAIMED
**Decision P1:** NOT refuted: Off-prime(1) median >= 0.584 ms; min 1.4566 max 1.4826 (the bar is outside [min, max]); all-frozen tail median 1.3043 ms (same verdict)

## P2a L9 C0 refutation reading (class bench --bench; STOP C4 if the predicted dt_np(1) band lies below 1.0 ms)
- classes jolt band low ms: 1.0770 [1.0730-1.1130] K=6 r/i/s 3.71/0.60/0.72 %
- classes jolt band high ms: 1.6660 [1.6610-1.7050] K=6 r/i/s 2.64/0.48/0.50 %
- classes jolt/separated ns/pair: 45.2500 [44.9000-48.4000] K=6 r/i/s 7.73/0.55/1.50 %
- classes jolt/touching ns/pair: 435.5500 [434.1000-440.9000] K=6 r/i/s 1.56/0.39/0.28 %
- classes jolt/stream ns/pair: 232.7000 [232.5000-236.8000] K=6 r/i/s 1.85/0.29/0.37 %
- classes rest/separated ns/pair: 52.4000 [52.0000-56.9000] K=6 r/i/s 9.35/0.67/1.84 %
- classes rest/touching ns/pair: 456.1000 [453.9000-457.9000] K=6 r/i/s 0.88/0.46/0.17 %
- classes rain/fast ns/pair: 208.4500 [206.9000-209.6000] K=6 r/i/s 1.30/0.36/0.22 %
- L9b-only prediction with the measured h = 0.9897 and N_touch = 4515: [1.499, 1.678] ms (t_hit in [60, 100] ns; the carry is in both arms of the same-binary A/B)
**Decision P2a:** DOES NOT FIRE: the band lies above 1.0 ms (median band [1.077, 1.666] ms)

## P2b L9 same-binary reuse off vs on (4db26681)
- L9-JA-off@W1 [0,500) ms: 18.1313 [18.0970-18.2676] K=6 r/i/s 0.94/0.29/0.18 %
- L9-JA-on@W1 [0,500) ms: 16.5068 [16.4506-16.5556] K=6 r/i/s 0.64/0.27/0.12 %
- on vs off W1 [0,500): ratio 0.9104 (-8.96 %, -1.6245); bars r/i/s 2.27/0.78/0.42 %; claimed r/i/s Y/Y/Y -> CLAIMED
- on vs off W1 [100,500): off 18.1605 [18.1324-18.3111] K=6 r/i/s 0.98/0.25/0.18 %; on 16.3152 [16.2494-16.3513] K=6 r/i/s 0.62/0.32/0.12 %; ratio 0.8984 (-10.16 %, -1.8453); bars r/i/s 2.33/0.81/0.44 %; claimed r/i/s Y/Y/Y -> CLAIMED
**Decision P2b (realized-gain rule):** realized dT(1)[100,500) = 1.845 ms against 0.6 x 1.499 = 0.900 ms: PASS
- L9-JA-off@W8 [0,500) ms: 6.0419 [6.0338-6.1725] K=6 r/i/s 2.30/0.58/0.46 %
- L9-JA-on@W8 [0,500) ms: 5.8998 [5.8593-6.0012] K=6 r/i/s 2.40/0.28/0.41 %
- on vs off W8 [0,500): ratio 0.9765 (-2.35 %, -0.1422); bars r/i/s 6.65/1.29/1.24 %; claimed r/i/s n/Y/Y -> not claimed
- on vs off W8 [100,500): off 6.0539 [5.9958-6.0794] K=6 r/i/s 1.38/0.72/0.28 %; on 5.8750 [5.8406-6.0155] K=6 r/i/s 2.98/0.81/0.57 %; ratio 0.9704 (-2.96 %, -0.1789); bars r/i/s 6.56/2.16/1.26 %; claimed r/i/s n/Y/Y -> not claimed
- armed W1 sys_physics_narrowphase_ns [100,500): off 2.4093 [2.3949-2.4531] K=6 r/i/s 2.42/0.81/0.45 %; on 0.6846 [0.6729-0.6907] K=6 r/i/s 2.59/0.45/0.44 %; ratio 0.2842 (-71.58 %, -1.7247); bars r/i/s 7.09/1.86/1.26 %; claimed r/i/s Y/Y/Y -> CLAIMED
**Measured dt_np(1) (L9b alone):** 1.725 ms against the 1.0 ms bar (L9a is in both arms)
- armed W1 wall_ns [100,500): off 18.1846 [18.1492-18.2218] K=6 r/i/s 0.40/0.16/0.08 %; on 16.3245 [16.2862-16.3770] K=6 r/i/s 0.56/0.21/0.10 %; ratio 0.8977 (-10.23 %, -1.8602); bars r/i/s 1.37/0.54/0.26 %; claimed r/i/s Y/Y/Y -> CLAIMED
- armed W1 sys_sum_ns [100,500): off 18.1359 [18.1018-18.1743] K=6 r/i/s 0.40/0.17/0.08 %; on 16.2784 [16.2424-16.3303] K=6 r/i/s 0.54/0.22/0.10 %; ratio 0.8976 (-10.24 %, -1.8575); bars r/i/s 1.34/0.55/0.25 %; claimed r/i/s Y/Y/Y -> CLAIMED

## P3 tree broadphase C3b: parent 6dd1f916 (RowWalk) vs tip 983480a9 (LeafList)
- C3b t_q par@W1 (median over [100,500) of phys_bp_query_ns) ms: 0.4044 [0.4038-0.4049] K=5 r/i/s 0.27/0.11/0.06 %
- C3b t_q tip@W1 (median over [100,500) of phys_bp_query_ns) ms: 0.2102 [0.2095-0.2120] K=6 r/i/s 1.18/0.60/0.24 %
- t_q tip vs par W1: ratio 0.5199 (-48.01 %, -0.1941); bars r/i/s 2.43/1.23/0.49 %; claimed r/i/s Y/Y/Y -> CLAIMED
- C3b tree span (sum of four phys_bp_* per-step medians) par@W1 ms: 0.4637 [0.4630-0.4659] K=5 r/i/s 0.62/0.11/0.13 %
- C3b tree span (sum of four phys_bp_* per-step medians) tip@W1 ms: 0.2714 [0.2707-0.2741] K=6 r/i/s 1.24/0.61/0.25 %
- C3b-TA-armed par@W1 wall [0,500) ms: 17.3984 [17.3599-17.5626] K=5 r/i/s 1.16/0.70/0.28 %
- C3b-TA-armed tip@W1 wall [0,500) ms: 17.2228 [17.1724-17.3052] K=6 r/i/s 0.77/0.52/0.17 %
- C3b-TA-armed W1 tip vs par: ratio 0.9899 (-1.01 %, -0.1756); bars r/i/s 2.79/1.75/0.66 %; claimed r/i/s n/n/Y -> not claimed
- C3b-TD-tree par@W1 wall [0,500) ms: 6.9458 [6.9364-7.0538] K=6 r/i/s 1.69/0.30/0.33 %
- C3b-TD-tree tip@W1 wall [0,500) ms: 6.8135 [6.7608-6.8767] K=5 r/i/s 1.70/1.02/0.39 %
- C3b-TD-tree W1 tip vs par: ratio 0.9810 (-1.90 %, -0.1322); bars r/i/s 4.80/2.12/1.03 %; claimed r/i/s n/n/Y -> not claimed
- C3b t_q par@W8 (median over [100,500) of phys_bp_query_ns) ms: 0.4126 [0.4120-0.4135] K=6 r/i/s 0.37/0.09/0.06 %
- C3b t_q tip@W8 (median over [100,500) of phys_bp_query_ns) ms: 0.2129 [0.2127-0.2135] K=6 r/i/s 0.37/0.20/0.08 %
- t_q tip vs par W8: ratio 0.5161 (-48.39 %, -0.1996); bars r/i/s 1.04/0.45/0.20 %; claimed r/i/s Y/Y/Y -> CLAIMED
- C3b tree span (sum of four phys_bp_* per-step medians) par@W8 ms: 0.4738 [0.4732-0.4751] K=6 r/i/s 0.41/0.14/0.07 %
- C3b tree span (sum of four phys_bp_* per-step medians) tip@W8 ms: 0.2758 [0.2757-0.2769] K=6 r/i/s 0.46/0.30/0.11 %
- C3b-TA-armed par@W8 wall [0,500) ms: 4.5290 [4.4957-4.7239] K=6 r/i/s 5.04/1.36/0.97 %
- C3b-TA-armed tip@W8 wall [0,500) ms: 4.3023 [4.2866-4.3483] K=6 r/i/s 1.43/0.66/0.28 %
- C3b-TA-armed W8 tip vs par: ratio 0.9500 (-5.00 %, -0.2266); bars r/i/s 10.48/3.03/2.03 %; claimed r/i/s n/Y/Y -> not claimed
- C3b-TD-tree par@W8 wall [0,500) ms: 2.6182 [2.6018-2.7156] K=6 r/i/s 4.35/1.07/0.83 %
- C3b-TD-tree tip@W8 wall [0,500) ms: 2.4307 [2.3764-2.4582] K=6 r/i/s 3.37/1.54/0.65 %
- C3b-TD-tree W8 tip vs par: ratio 0.9284 (-7.16 %, -0.1875); bars r/i/s 10.99/3.74/2.11 %; claimed r/i/s n/Y/Y -> not claimed
**Decision P3:** t_q(J,W=1) tip = 0.2102 ms (c_q = 170 ns/row); kernel SHIPS (ratio < 1); attribution A REFUTED (ratio > 0.55 or t_q > 0.21); C2 not built (t_q < 0.235); into the band not claimed; F3 taken up (c_q > 150 ns); tree span 0.2714 / 0.2758 ms vs limits 0.36 / 0.35: PASS

## P4 L11 G9 remainder: parent 0ca312bd vs tip cbd86a65
- G9-JA g9p@W1 ms: 18.9490 [18.8844-19.0661] K=6 r/i/s 0.96/0.36/0.18 %
- G9-JA g9t@W1 ms: 18.9285 [18.8526-19.0498] K=6 r/i/s 1.04/0.25/0.18 %
- G9-JA W1 tip vs parent: ratio 0.9989 (-0.11 %, -0.0205); bars r/i/s 2.83/0.87/0.51 %; claimed r/i/s n/n/n -> not claimed
- G9-JA g9p@W8 ms: 6.1700 [6.1285-6.2846] K=6 r/i/s 2.53/0.81/0.47 %
- G9-JA g9t@W8 ms: 6.1279 [6.1110-6.2297] K=6 r/i/s 1.94/0.44/0.37 %
- G9-JA W8 tip vs parent: ratio 0.9932 (-0.68 %, -0.0421); bars r/i/s 6.37/1.85/1.19 %; claimed r/i/s n/n/n -> not claimed
- G9-R g9p@W1 ms: 11.3311 [11.1485-11.7631] K=5 r/i/s 5.42/1.03/1.15 %
- G9-R g9t@W1 ms: 10.9177 [10.7821-11.0219] K=6 r/i/s 2.20/1.01/0.42 %
- G9-R W1 tip vs parent: ratio 0.9635 (-3.65 %, -0.4134); bars r/i/s 11.70/2.89/2.45 %; claimed r/i/s n/Y/Y -> not claimed
  - phys_solve_build_ns: parent 0.7244 [0.6940-0.7960] K=5 r/i/s 14.08/4.98/3.05 %; tip 0.7278 [0.6529-0.8397] K=6 r/i/s 25.67/13.74/5.10 %; ratio 1.0046 (+0.46 %, +0.0033); bars r/i/s 58.56/29.24/11.88 %; claimed r/i/s n/n/n -> not claimed
  - phys_warm_apply_ns: parent 0.7532 [0.7523-0.7592] K=5 r/i/s 0.91/0.62/0.23 %; tip 0.4597 [0.4514-0.4636] K=6 r/i/s 2.65/0.85/0.48 %; ratio 0.6102 (-38.98 %, -0.2936); bars r/i/s 5.61/2.10/1.07 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - phys_store_ns: parent 0.0628 [0.0608-0.0676] K=5 r/i/s 10.76/3.97/2.46 %; tip 0.0644 [0.0541-0.0710] K=6 r/i/s 26.14/16.21/5.50 %; ratio 1.0239 (+2.39 %, +0.0015); bars r/i/s 56.53/33.37/12.05 %; claimed r/i/s n/n/n -> not claimed
  - phys_color_wide_ns: parent 3.6439 [3.5712-4.1736] K=5 r/i/s 16.53/5.17/3.82 %; tip 3.5837 [3.5725-3.6656] K=6 r/i/s 2.60/1.67/0.60 %; ratio 0.9835 (-1.65 %, -0.0602); bars r/i/s 33.47/10.86/7.74 %; claimed r/i/s n/n/n -> not claimed
- G9-R g9p@W8 ms: 5.7155 [5.5954-5.8973] K=6 r/i/s 5.28/3.27/1.11 %
- G9-R g9t@W8 ms: 5.4469 [5.3200-5.7255] K=6 r/i/s 7.45/3.17/1.41 %
- G9-R W8 tip vs parent: ratio 0.9530 (-4.70 %, -0.2686); bars r/i/s 18.26/9.11/3.59 %; claimed r/i/s n/n/Y -> not claimed
  - phys_solve_build_ns: parent 0.6618 [0.5958-0.8405] K=6 r/i/s 36.99/25.38/8.32 %; tip 0.7222 [0.5975-0.8178] K=6 r/i/s 30.50/14.12/5.83 %; ratio 1.0911 (+9.11 %, +0.0603); bars r/i/s 95.88/58.09/20.33 %; claimed r/i/s n/n/n -> not claimed
  - phys_warm_apply_ns: parent 0.7733 [0.7709-0.7832] K=6 r/i/s 1.59/0.56/0.31 %; tip 0.4729 [0.4682-0.5072] K=6 r/i/s 8.25/1.71/1.58 %; ratio 0.6115 (-38.85 %, -0.3004); bars r/i/s 16.80/3.60/3.21 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - phys_store_ns: parent 0.0618 [0.0503-0.0810] K=6 r/i/s 49.67/27.32/9.95 %; tip 0.0690 [0.0590-0.0818] K=6 r/i/s 33.04/13.62/6.17 %; ratio 1.1176 (+11.76 %, +0.0073); bars r/i/s 119.31/61.06/23.42 %; claimed r/i/s n/n/n -> not claimed
  - phys_color_wide_ns: parent 1.0260 [1.0142-1.0358] K=6 r/i/s 2.11/1.41/0.45 %; tip 1.0297 [1.0205-1.0435] K=6 r/i/s 2.23/0.73/0.40 %; ratio 1.0036 (+0.36 %, +0.0037); bars r/i/s 6.15/3.18/1.20 %; claimed r/i/s n/n/n -> not claimed
- G9-RS g9p@W1 ms: 6.0832 [6.0453-6.1027] K=6 r/i/s 0.94/0.62/0.21 %
- G9-RS g9t@W1 ms: 6.0552 [6.0114-6.1052] K=6 r/i/s 1.55/0.63/0.28 %
- G9-RS W1 tip vs parent: ratio 0.9954 (-0.46 %, -0.0279); bars r/i/s 3.63/1.77/0.70 %; claimed r/i/s n/n/n -> not claimed
  - phys_solve_build_ns: parent 0.0607 [0.0599-0.0625] K=6 r/i/s 4.23/1.91/0.82 %; tip 0.0605 [0.0585-0.0657] K=6 r/i/s 11.89/2.25/2.10 %; ratio 0.9964 (-0.36 %, -0.0002); bars r/i/s 25.24/5.91/4.51 %; claimed r/i/s n/n/n -> not claimed
  - phys_warm_apply_ns: parent 0.0000 [0.0000-0.0001] K=6 r/i/s 46.51/10.20/8.25 %; tip 0.0001 [0.0000-0.0001] K=6 r/i/s 28.09/17.09/5.84 %; ratio 1.3073 (+30.73 %, +0.0000); bars r/i/s 108.67/39.80/20.22 %; claimed r/i/s n/n/Y -> not claimed
  - phys_store_ns: parent 0.1315 [0.1309-0.1337] K=6 r/i/s 2.16/0.46/0.40 %; tip 0.1183 [0.1165-0.1203] K=6 r/i/s 3.25/1.75/0.65 %; ratio 0.8999 (-10.01 %, -0.0132); bars r/i/s 7.80/3.63/1.52 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - phys_color_wide_ns: parent 0.0000 [0.0000-0.0000] K=6 r/i/s 0.00/0.00/0.00 %; tip 0.0000 [0.0000-0.0000] K=6 r/i/s 0.00/0.00/0.00 %; n/a
- G9-RS g9p@W8 ms: 3.3061 [3.2584-3.3345] K=5 r/i/s 2.30/1.25/0.52 %
- G9-RS g9t@W8 ms: 3.2761 [3.2369-3.3212] K=6 r/i/s 2.57/1.18/0.48 %
- G9-RS W8 tip vs parent: ratio 0.9909 (-0.91 %, -0.0300); bars r/i/s 6.90/3.43/1.42 %; claimed r/i/s n/n/n -> not claimed
  - phys_solve_build_ns: parent 0.0608 [0.0591-0.0617] K=5 r/i/s 4.22/2.97/1.04 %; tip 0.0572 [0.0567-0.0601] K=6 r/i/s 5.86/3.00/1.23 %; ratio 0.9414 (-5.86 %, -0.0036); bars r/i/s 14.44/8.44/3.22 %; claimed r/i/s n/n/Y -> not claimed
  - phys_warm_apply_ns: parent 0.0000 [0.0000-0.0001] K=5 r/i/s 34.31/29.39/9.63 %; tip 0.0001 [0.0000-0.0001] K=6 r/i/s 29.82/7.99/5.18 %; ratio 1.3426 (+34.26 %, +0.0000); bars r/i/s 90.92/60.91/21.87 %; claimed r/i/s n/n/Y -> not claimed
  - phys_store_ns: parent 0.1350 [0.1322-0.1365] K=5 r/i/s 3.13/1.05/0.69 %; tip 0.1238 [0.1204-0.1284] K=6 r/i/s 6.44/1.97/1.12 %; ratio 0.9169 (-8.31 %, -0.0112); bars r/i/s 14.33/4.47/2.64 %; claimed r/i/s n/Y/Y -> not claimed
  - phys_color_wide_ns: parent 0.0000 [0.0000-0.0000] K=5 r/i/s 0.00/0.00/0.00 %; tip 0.0000 [0.0000-0.0000] K=6 r/i/s 0.00/0.00/0.00 %; n/a
- G9-S16 g9p@W1 ms: 0.0825 [0.0814-0.0842] K=5 r/i/s 3.38/1.29/0.75 %
- G9-S16 g9t@W1 ms: 0.0822 [0.0812-0.0835] K=6 r/i/s 2.77/0.78/0.48 %
- G9-S16 W1 tip vs parent: ratio 0.9968 (-0.32 %, -0.0003); bars r/i/s 8.75/3.02/1.78 %; claimed r/i/s n/n/n -> not claimed
  - phys_solve_build_ns: parent 0.0014 [0.0013-0.0014] K=5 r/i/s 9.79/2.98/2.04 %; tip 0.0014 [0.0013-0.0014] K=6 r/i/s 6.93/4.63/1.48 %; ratio 0.9929 (-0.71 %, -0.0000); bars r/i/s 23.99/11.01/5.05 %; claimed r/i/s n/n/n -> not claimed
  - phys_warm_apply_ns: parent 0.0021 [0.0020-0.0022] K=5 r/i/s 10.62/3.62/2.35 %; tip 0.0020 [0.0020-0.0022] K=6 r/i/s 8.29/3.25/1.62 %; ratio 0.9930 (-0.70 %, -0.0000); bars r/i/s 26.95/9.73/5.71 %; claimed r/i/s n/n/n -> not claimed
  - phys_store_ns: parent 0.0001 [0.0001-0.0001] K=5 r/i/s 12.13/4.53/2.67 %; tip 0.0001 [0.0001-0.0001] K=6 r/i/s 9.37/1.87/1.59 %; ratio 1.2073 (+20.73 %, +0.0000); bars r/i/s 30.66/9.79/6.22 %; claimed r/i/s n/Y/Y -> not claimed
  - phys_color_wide_ns: parent 0.0000 [0.0000-0.0000] K=5 r/i/s 0.00/0.00/0.00 %; tip 0.0000 [0.0000-0.0000] K=6 r/i/s 0.00/0.00/0.00 %; n/a
- G9-JAs-mid g9p@W2 ms: 6.5329 [6.4738-6.6183] K=5 r/i/s 2.21/0.99/0.49 %
- G9-JAs-mid g9t@W2 ms: 6.2901 [6.2553-6.4068] K=6 r/i/s 2.41/1.40/0.53 %
- G9-JAs-mid W2 tip vs parent: ratio 0.9628 (-3.72 %, -0.2429); bars r/i/s 6.54/3.43/1.44 %; claimed r/i/s n/Y/Y -> not claimed
- G9-JAs-mid g9p@W4 ms: 5.2088 [5.1734-5.2699] K=6 r/i/s 1.85/0.68/0.35 %
- G9-JAs-mid g9t@W4 ms: 4.9884 [4.9615-5.0734] K=6 r/i/s 2.24/0.38/0.41 %
- G9-JAs-mid W4 tip vs parent: ratio 0.9577 (-4.23 %, -0.2205); bars r/i/s 5.82/1.56/1.07 %; claimed r/i/s n/Y/Y -> not claimed
- G9-JAs-mid g9p@W16 ms: 4.8798 [4.8512-4.9637] K=6 r/i/s 2.31/1.12/0.47 %
- G9-JAs-mid g9t@W16 ms: 4.6192 [4.5856-4.6314] K=5 r/i/s 0.99/0.68/0.25 %
- G9-JAs-mid W16 tip vs parent: ratio 0.9466 (-5.34 %, -0.2606); bars r/i/s 5.02/2.63/1.06 %; claimed r/i/s Y/Y/Y -> CLAIMED
- G9-JAs-a g9p@W1 ms: 8.9108 [8.6716-9.0698] K=6 r/i/s 4.47/2.40/0.89 %
- G9-JC g9p@W1 ms: 9.3803 [9.1553-9.6585] K=6 r/i/s 5.36/3.94/1.20 %
- G9-JC g9p@W1 injected canary ms: 0.4416 [0.4336-0.4535] K=6 r/i/s 4.51/3.43/1.02 %
- G9-JC g9p@W1 canary span ms: 0.4419 [0.4338-0.4537] K=6 r/i/s 4.51/3.44/1.02 %
- canary g9p: span 0.4419 vs injected 0.4416 ms (ok); step rise +0.4694 ms (NOT SEEN under the claim rule); ratio 1.0527 (+5.27 %, +0.4694); bars r/i/s 13.96/9.22/2.99 %; claimed r/i/s n/n/Y -> not claimed
- G9-JAs-a g9t@W1 ms: 8.5486 [8.4454-8.6318] K=6 r/i/s 2.18/0.82/0.39 %
- G9-JC g9t@W1 ms: 8.9966 [8.9692-9.0971] K=6 r/i/s 1.42/0.50/0.27 %
- G9-JC g9t@W1 injected canary ms: 0.4278 [0.4223-0.4316] K=6 r/i/s 2.18/0.86/0.40 %
- G9-JC g9t@W1 canary span ms: 0.4281 [0.4225-0.4319] K=6 r/i/s 2.18/1.11/0.42 %
- canary g9t: span 0.4281 vs injected 0.4278 ms (ok); step rise +0.4481 ms (SEEN (claimed)); ratio 1.0524 (+5.24 %, +0.4481); bars r/i/s 5.21/1.91/0.95 %; claimed r/i/s Y/Y/Y -> CLAIMED
**Decision P4:** G9 remainder: no row claimed slower

## P5 extras
- B2-JAs w4bt(L11 C2)@W1 ms: 8.7433 [8.6514-9.1162] K=6 r/i/s 5.32/3.03/1.13 %
- B2-JAs g9p(C2+line merge)@W1 ms: 8.7571 [8.7111-8.7759] K=6 r/i/s 0.74/0.49/0.17 %
- bridge 2 W1 (C2+merge vs C2): ratio 1.0016 (+0.16 %, +0.0138); bars r/i/s 10.73/6.13/2.29 %; claimed r/i/s n/n/n -> not claimed
- B2-JAs w4bt(L11 C2)@W8 ms: 4.4508 [4.4284-4.4831] K=5 r/i/s 1.23/0.54/0.27 %
- B2-JAs g9p(C2+line merge)@W8 ms: 4.5288 [4.5053-4.5672] K=6 r/i/s 1.37/0.86/0.29 %
- bridge 2 W8 (C2+merge vs C2): ratio 1.0175 (+1.75 %, +0.0780); bars r/i/s 3.68/2.03/0.79 %; claimed r/i/s n/n/Y -> not claimed
- L10-Offp8@W8 wall ms: 0.8170 [0.8089-0.8810] K=6 r/i/s 8.83/1.55/1.71 %
- L10-Offp8@W8 sys_sum ms: 0.7864 [0.7787-0.8490] K=6 r/i/s 8.95/1.61/1.73 %
- L10-Son8@W8 wall ms: 2.7989 [2.7971-2.8067] K=6 r/i/s 0.34/0.10/0.06 %
- L10-Son8@W8 sys_sum ms: 2.7665 [2.7645-2.7741] K=6 r/i/s 0.35/0.10/0.06 %
- L10-RS-Offp@W1 wall ms: 1.5936 [1.5820-1.6984] K=6 r/i/s 7.30/0.68/1.43 %
- L10-RS-Offp@W1 sys_sum ms: 1.5611 [1.5494-1.6620] K=6 r/i/s 7.22/0.70/1.41 %
- L10-RS-Offp@W8 wall ms: 0.9656 [0.9609-1.0410] K=6 r/i/s 8.29/0.75/1.67 %
- L10-RS-Offp@W8 sys_sum ms: 0.9349 [0.9306-1.0080] K=6 r/i/s 8.28/0.79/1.67 %
- L10-RS@W1 wall ms: 5.7784 [5.7575-5.9205] K=6 r/i/s 2.82/0.47/0.54 %
- L10-RS@W1 sys_sum ms: 5.7440 [5.7244-5.8782] K=6 r/i/s 2.68/0.46/0.51 %
- L10-RS@W8 wall ms: 3.1212 [3.1107-3.2863] K=6 r/i/s 5.63/1.05/1.12 %
- L10-RS@W8 sys_sum ms: 3.0877 [3.0774-3.2514] K=6 r/i/s 5.63/1.02/1.12 %
- C3b-G4 bp_g4_scene/tree/j100 ms: 0.1446 [0.1443-0.1457] K=6 r/i/s 0.95/0.33/0.18 %
- C3b-G4 bp_g4_scene/tree_rowwalk/j100 ms: 0.4388 [0.4383-0.4399] K=6 r/i/s 0.37/0.04/0.06 %
- C3b-G4 bp_g4_scene/tree/1240 ms: 0.1444 [0.1440-0.1451] K=6 r/i/s 0.75/0.53/0.17 %
- C3b-G4 bp_g4_scene/tree_rowwalk/1240 ms: 0.4374 [0.4370-0.4433] K=6 r/i/s 1.43/0.10/0.28 %
- C3b-G4 bp_g4_uniform/tree/1000 ms: 0.0841 [0.0836-0.0853] K=6 r/i/s 1.99/1.01/0.40 %
- C3b-G4 bp_g4_uniform/tree_rowwalk/1000 ms: 0.1607 [0.1583-0.1622] K=6 r/i/s 2.43/0.76/0.43 %
- C3b-G4 bp_g4_disparity/tree/1000 ms: 0.1045 [0.1041-0.1053] K=6 r/i/s 1.17/0.18/0.20 %
- C3b-G4 bp_g4_disparity/tree_rowwalk/1000 ms: 0.2293 [0.2281-0.2307] K=6 r/i/s 1.12/0.91/0.26 %
- C3b-G4 LeafList/RowWalk scene/j100 (per-process ratio): 0.3297 [0.3289-0.3311] K=6 r/i/s 0.66/0.32/0.13 %
- C3b-G4 LeafList/RowWalk scene/1240 (per-process ratio): 0.3303 [0.3252-0.3319] K=6 r/i/s 2.02/0.68/0.38 %
- C3b-G4 LeafList/RowWalk uniform/1000 (per-process ratio): 0.5247 [0.5157-0.5378] K=6 r/i/s 4.19/1.53/0.76 %
- C3b-G4 LeafList/RowWalk disparity/1000 (per-process ratio): 0.4570 [0.4526-0.4589] K=6 r/i/s 1.37/0.77/0.29 %
- L9 J-D on vs off W1: off 7.4858 [7.4635-7.5300] K=6 r/i/s 0.89/0.26/0.16 %; on 6.0205 [5.9952-6.0671] K=6 r/i/s 1.19/0.50/0.23 %; ratio 0.8043 (-19.57 %, -1.4652); bars r/i/s 2.98/1.13/0.56 %; claimed r/i/s Y/Y/Y -> CLAIMED
- L9 J-D on vs off W8: off 3.9391 [3.9279-3.9978] K=6 r/i/s 1.77/0.23/0.33 %; on 3.7475 [3.7332-3.7666] K=6 r/i/s 0.89/0.38/0.17 %; ratio 0.9514 (-4.86 %, -0.1916); bars r/i/s 3.97/0.89/0.74 %; claimed r/i/s Y/Y/Y -> CLAIMED

## P5g C3b G5 on the tip, same binary: tree vs allpairs
- G5 broadphase span tree tip@W1 [100,500) ms: 0.3121 [0.3104-0.3238] K=6 r/i/s 4.29/0.58/0.83 %
- G5 broadphase span allpairs tip@W1 [100,500) ms: 2.1196 [2.1043-2.1345] K=6 r/i/s 1.42/0.82/0.29 %
- dbp(W1) = 1.8076 ms
- G5 t_q tip@W1 (same-block repeat of P3) ms: 0.2354 [0.2325-0.2399] K=6 r/i/s 3.13/0.58/0.53 %
- G5-TA-ap-a tip@W1 ms: 18.8918 [18.8416-18.9112] K=6 r/i/s 0.37/0.19/0.07 %
- G5-TA-tree-a tip@W1 ms: 16.9488 [16.8609-16.9610] K=6 r/i/s 0.59/0.39/0.14 %
- G5-TA-tree-a vs G5-TA-ap-a W1: ratio 0.8972 (-10.28 %, -1.9430); bars r/i/s 1.39/0.88/0.31 %; claimed r/i/s Y/Y/Y -> CLAIMED
- G5-TD-ap tip@W1 ms: 8.4040 [8.3568-8.4754] K=6 r/i/s 1.41/0.31/0.24 %
- G5-TD-tree tip@W1 ms: 6.5050 [6.4649-6.5871] K=6 r/i/s 1.88/1.06/0.39 %
- G5-TD-tree vs G5-TD-ap W1: ratio 0.7740 (-22.60 %, -1.8990); bars r/i/s 4.70/2.22/0.92 %; claimed r/i/s Y/Y/Y -> CLAIMED
- G5 broadphase span tree tip@W8 [100,500) ms: 0.2699 [0.2690-0.2775] K=6 r/i/s 3.13/0.84/0.61 %
- G5 broadphase span allpairs tip@W8 [100,500) ms: 1.9681 [1.9655-1.9718] K=6 r/i/s 0.32/0.12/0.06 %
- dbp(W8) = 1.6982 ms
- G5 t_q tip@W8 (same-block repeat of P3) ms: 0.2094 [0.2087-0.2156] K=6 r/i/s 3.31/0.64/0.65 %
- G5-TA-ap-a tip@W8 ms: 5.8276 [5.8196-5.8331] K=6 r/i/s 0.23/0.15/0.05 %
- G5-TA-tree-a tip@W8 ms: 4.0433 [4.0315-4.1513] K=6 r/i/s 2.96/0.67/0.57 %
- G5-TA-tree-a vs G5-TA-ap-a W8: ratio 0.6938 (-30.62 %, -1.7843); bars r/i/s 5.94/1.36/1.15 %; claimed r/i/s Y/Y/Y -> CLAIMED
- G5-TD-ap tip@W8 ms: 4.0237 [4.0128-4.0350] K=6 r/i/s 0.55/0.14/0.10 %
- G5-TD-tree tip@W8 ms: 2.2466 [2.2399-2.2524] K=6 r/i/s 0.55/0.09/0.09 %
- G5-TD-tree vs G5-TD-ap W8: ratio 0.5583 (-44.17 %, -1.7771); bars r/i/s 1.56/0.34/0.26 %; claimed r/i/s Y/Y/Y -> CLAIMED

## P5e / P5f L9 reuse off vs on: R W1/8, J-A W2/4/16
- L9-R-on vs L9-R-off W1: off 10.7346 [10.6351-10.8007] K=6 r/i/s 1.54/0.37/0.27 %; on 7.8190 [7.7762-7.9300] K=6 r/i/s 1.97/0.37/0.35 %; ratio 0.7284 (-27.16 %, -2.9156); bars r/i/s 5.00/1.04/0.88 %; claimed r/i/s Y/Y/Y -> CLAIMED
- L9-R-on vs L9-R-off W8: off 4.8212 [4.8148-4.8364] K=6 r/i/s 0.45/0.22/0.09 %; on 4.5275 [4.5164-4.5812] K=6 r/i/s 1.43/0.35/0.27 %; ratio 0.9391 (-6.09 %, -0.2937); bars r/i/s 3.00/0.82/0.57 %; claimed r/i/s Y/Y/Y -> CLAIMED
- L9-JA-on-mid vs L9-JA-off-mid W2: off 11.1203 [11.0732-11.1257] K=6 r/i/s 0.47/0.05/0.09 %; on 10.3278 [10.2909-10.3415] K=6 r/i/s 0.49/0.10/0.09 %; ratio 0.9287 (-7.13 %, -0.7925); bars r/i/s 1.36/0.22/0.25 %; claimed r/i/s Y/Y/Y -> CLAIMED
- L9-JA-on-mid vs L9-JA-off-mid W4: off 7.5114 [7.4866-7.5354] K=6 r/i/s 0.65/0.13/0.11 %; on 7.1147 [7.1011-7.1355] K=6 r/i/s 0.48/0.26/0.10 %; ratio 0.9472 (-5.28 %, -0.3967); bars r/i/s 1.62/0.58/0.29 %; claimed r/i/s Y/Y/Y -> CLAIMED
- L9-JA-on-mid vs L9-JA-off-mid W16: off 5.5976 [5.5865-5.6103] K=6 r/i/s 0.42/0.14/0.07 %; on 5.4821 [5.4766-5.5199] K=6 r/i/s 0.79/0.40/0.17 %; ratio 0.9794 (-2.06 %, -0.1155); bars r/i/s 1.79/0.84/0.36 %; claimed r/i/s Y/Y/Y -> CLAIMED
