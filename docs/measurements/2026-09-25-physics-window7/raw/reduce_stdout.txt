# Window 7 reduction (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win7\raw)
542 records, 431 chosen processes, 19 dropped slots, 0 voided passes
dropped slots (no valid clean attempt): [('P1A-jolt', 0, 0, 0, 'H-trk-tree', 'trk', 2), ('P1A-jolt', 0, 0, 0, 'H-trk-ap', 'trk', 4), ('P1A-jolt', 0, 0, 1, 'H-trk-tree', 'trk', 1), ('P1A-jolt', 0, 0, 1, 'H-trk-ap', 'trk', 2), ('P1A-jolt', 0, 0, 1, 'H-trk-tree', 'trk', 16), ('P1A-jolt', 0, 0, 2, 'H-trk-ap', 'trk', 1), ('P1A-jolt', 0, 0, 2, 'H-trk-tree', 'trk', 4), ('P1A-jolt', 0, 0, 2, 'H-trk-ap', 'trk', 8), ('P1A-jolt', 0, 0, 2, 'H-trk-ap', 'trk', 16), ('P1A-jolt', 1, 0, 0, 'H-trk-tree', 'trk', 8), ('P1A-jolt', 1, 0, 0, 'H-trk-ap', 'trk', 4), ('P1A-jolt', 1, 0, 1, 'H-trk-tree', 'trk', 16), ('P1A-jolt', 1, 0, 2, 'H-trk-ap', 'trk', 4), ('P1A-jolt', 1, 0, 2, 'H-jolt56', 'j56', 4), ('P1A-jolt', 1, 0, 2, 'H-trk-tree', 'trk', 2), ('P2-L9GTW', 0, 0, 0, 'L9-R-off', 'c4', 1), ('P2-L9GTW', 0, 0, 1, 'L9-JD-on', 'c4', 4), ('P2-L9GTW', 0, 0, 1, 'L9-R-off', 'c4', 8), ('P2-L9GTW', 0, 0, 1, 'L9-JA-a-off', 'c4', 8)]

## P1 The Jolt headline, same window (trunk integ/unified 93b2615b vs Jolt v5.6.0 918fd2b7)
Rows: H-trk-tree = --cfg default --broadphase tree (reuse off on this tree); H-trk-ap = --cfg default --broadphase allpairs; H-jolt56 = -s=Pyramid -q=Discrete -f. Statistic: the process mean over [0,500) (window 3).
- Jolt manifolds per step over [100,500) (-receipt, untimed, gate): W1 8489.0, W8 8489.0 (window 3: 8489.0)

### W=1
- H-trk-tree@W1 [A] [0,500) ms: 6.2917 [6.1973-6.6359] K=5 r/i/s 6.97/4.61/1.72 %
- H-trk-tree@W1 [B] [0,500) ms: 5.9565 [5.9353-6.0435] K=6 r/i/s 1.82/1.13/0.40 %
- H-trk-tree@W1 [pooled] [0,500) ms: 6.0435 [5.9353-6.6359] K=11 r/i/s 11.59/5.26/1.55 %
- H-trk-ap@W1 [A] [0,500) ms: 7.9549 [7.8298-8.0316] K=5 r/i/s 2.54/1.77/0.62 %
- H-trk-ap@W1 [B] [0,500) ms: 7.6677 [7.6368-7.7412] K=6 r/i/s 1.36/0.70/0.27 %
- H-trk-ap@W1 [pooled] [0,500) ms: 7.7412 [7.6368-8.0316] K=11 r/i/s 5.10/3.07/0.72 %
- H-jolt56@W1 [A] [0,500) ms: 11.1250 [10.2366-13.3711] K=6 r/i/s 28.18/10.43/5.25 %
- H-jolt56@W1 [B] [0,500) ms: 9.6040 [9.4890-10.7011] K=6 r/i/s 12.62/2.73/2.45 %
- H-jolt56@W1 [pooled] [0,500) ms: 10.3816 [9.4890-13.3711] K=12 r/i/s 37.39/13.85/4.08 %
- H-trk-tree / Jolt56 W1 [A] [0,500): ratio 0.5655 (-43.45 %, -4.8333); bars r/i/s 58.05/22.81/11.06 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W1 [B] [0,500): ratio 0.6202 (-37.98 %, -3.6475); bars r/i/s 25.50/5.90/4.96 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W1 [pooled] [0,500): ratio 0.5821 (-41.79 %, -4.3381); bars r/i/s 78.30/29.63/8.74 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 6.0107 [5.9452-6.5885] K=11 r/i/s 10.70/7.59/1.67 %; jolt 10.5319 [9.9782-13.1482] K=12 r/i/s 30.10/10.43/3.97 %; ratio 0.5707 (-42.93 %, -4.5212); bars r/i/s 63.89/25.80/8.62 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled: ours 6.0517 [5.9322-6.6478] K=11 r/i/s 11.82/4.66/1.54 %; jolt 10.3311 [9.3630-13.4312] K=12 r/i/s 39.38/14.68/4.18 %; ratio 0.5858 (-41.42 %, -4.2794); bars r/i/s 82.23/30.81/8.90 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-tree / Jolt56 W1: pooled 0.5821333178341612, A 0.5655434033747498, B 0.6202134389495159 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 1.3391 us (4519.2575 manifolds/step), Jolt 1.2170 us (8489.0 manifolds/step) -> ours/Jolt 1.100x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- H-trk-ap / Jolt56 W1 [A] [0,500): ratio 0.7150 (-28.50 %, -3.1701); bars r/i/s 56.58/21.17/10.58 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W1 [B] [0,500): ratio 0.7984 (-20.16 %, -1.9363); bars r/i/s 25.39/5.63/4.93 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W1 [pooled] [0,500): ratio 0.7457 (-25.43 %, -2.6404); bars r/i/s 75.48/28.38/8.29 %; claimed r/i/s n/n/Y -> not claimed
  - [0..100) pooled: ours 7.7441 [7.6243-8.2093] K=11 r/i/s 7.55/2.75/0.83 %; jolt 10.5319 [9.9782-13.1482] K=12 r/i/s 30.10/10.43/3.97 %; ratio 0.7353 (-26.47 %, -2.7878); bars r/i/s 62.07/21.58/8.12 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled: ours 7.7405 [7.6320-8.0612] K=11 r/i/s 5.55/3.02/0.72 %; jolt 10.3311 [9.3630-13.4312] K=12 r/i/s 39.38/14.68/4.18 %; ratio 0.7492 (-25.08 %, -2.5906); bars r/i/s 79.53/29.98/8.48 %; claimed r/i/s n/n/Y -> not claimed
  **H-trk-ap / Jolt56 W1: pooled 0.7456624392443661, A 0.7150467178981494, B 0.7983889142325411 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 1.7128 us (4519.2575 manifolds/step), Jolt 1.2170 us (8489.0 manifolds/step) -> ours/Jolt 1.407x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- tree vs allpairs W1 pooled: ratio 0.7807 (-21.93 %, -1.6977); bars r/i/s 25.33/12.17/3.42 %; claimed r/i/s n/Y/Y -> not claimed
- window term: Jolt56 this window / window 3 = 1.0563 (10.3816 vs 9.8282 ms; cross-window, context only)
- block term (same window): Jolt56 B vs A: ratio 0.8633 (-13.67 %, -1.5210); bars r/i/s 61.75/21.57/11.59 %; claimed r/i/s n/n/Y -> not claimed
- block term: H-trk-tree B vs A: ratio 0.9467 (-5.33 %, -0.3352); bars r/i/s 14.41/9.48/3.53 %; claimed r/i/s n/n/Y -> not claimed
- block term: H-trk-ap B vs A: ratio 0.9639 (-3.61 %, -0.2872); bars r/i/s 5.76/3.81/1.35 %; claimed r/i/s n/n/Y -> not claimed

### W=2
- H-trk-tree@W2 [A] [0,500) ms: 4.1263 [4.0934-4.1775] K=4 r/i/s 2.04/0.76/0.53 %
- H-trk-tree@W2 [B] [0,500) ms: 3.9187 [3.8712-4.0649] K=6 r/i/s 4.94/2.17/0.96 %
- H-trk-tree@W2 [pooled] [0,500) ms: 4.0303 [3.8712-4.1775] K=10 r/i/s 7.60/4.94/1.11 %
- H-trk-ap@W2 [A] [0,500) ms: 6.1655 [5.9096-6.5902] K=5 r/i/s 11.04/0.41/2.23 %
- H-trk-ap@W2 [B] [0,500) ms: 5.6787 [5.6466-5.8691] K=6 r/i/s 3.92/2.40/0.89 %
- H-trk-ap@W2 [pooled] [0,500) ms: 5.8691 [5.6466-6.5902] K=11 r/i/s 16.08/8.22/1.92 %
- H-jolt56@W2 [A] [0,500) ms: 6.1113 [5.9318-6.9120] K=6 r/i/s 16.04/3.93/2.98 %
- H-jolt56@W2 [B] [0,500) ms: 5.7442 [5.6812-5.8628] K=6 r/i/s 3.16/1.46/0.62 %
- H-jolt56@W2 [pooled] [0,500) ms: 5.8973 [5.6812-6.9120] K=12 r/i/s 20.87/5.99/2.13 %
- H-trk-tree / Jolt56 W2 [A] [0,500): ratio 0.6752 (-32.48 %, -1.9849); bars r/i/s 32.34/8.00/6.06 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W2 [B] [0,500): ratio 0.6822 (-31.78 %, -1.8255); bars r/i/s 11.73/5.23/2.28 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W2 [pooled] [0,500): ratio 0.6834 (-31.66 %, -1.8669); bars r/i/s 44.42/15.53/4.80 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 4.0288 [3.9096-4.2978] K=10 r/i/s 9.64/6.90/1.57 %; jolt 5.8765 [5.7316-6.4422] K=12 r/i/s 12.09/3.10/1.23 %; ratio 0.6856 (-31.44 %, -1.8477); bars r/i/s 30.93/15.13/4.00 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [100..500) pooled: ours 4.0132 [3.8616-4.1972] K=10 r/i/s 8.36/4.34/1.11 %; jolt 5.8947 [5.6686-7.0939] K=12 r/i/s 24.18/6.67/2.43 %; ratio 0.6808 (-31.92 %, -1.8815); bars r/i/s 51.17/15.92/5.33 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-tree / Jolt56 W2: pooled 0.6834238649833263, A 0.6752029984021285, B 0.682193174006949 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.8880 us (4519.2575 manifolds/step), Jolt 0.6944 us (8489.0 manifolds/step) -> ours/Jolt 1.279x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- H-trk-ap / Jolt56 W2 [A] [0,500): ratio 1.0089 (+0.89 %, +0.0543); bars r/i/s 38.94/7.90/7.45 %; claimed r/i/s n/n/n -> not claimed
- H-trk-ap / Jolt56 W2 [B] [0,500): ratio 0.9886 (-1.14 %, -0.0656); bars r/i/s 10.07/5.62/2.17 %; claimed r/i/s n/n/n -> not claimed
- H-trk-ap / Jolt56 W2 [pooled] [0,500): ratio 0.9952 (-0.48 %, -0.0282); bars r/i/s 52.69/20.35/5.74 %; claimed r/i/s n/n/n -> not claimed
  - [0..100) pooled: ours 5.7570 [5.6556-6.8383] K=11 r/i/s 20.54/7.78/2.37 %; jolt 5.8765 [5.7316-6.4422] K=12 r/i/s 12.09/3.10/1.23 %; ratio 0.9797 (-2.03 %, -0.1195); bars r/i/s 47.68/16.75/5.35 %; claimed r/i/s n/n/n -> not claimed
  - [100..500) pooled: ours 5.8828 [5.6414-6.5281] K=11 r/i/s 15.07/8.34/1.84 %; jolt 5.8947 [5.6686-7.0939] K=12 r/i/s 24.18/6.67/2.43 %; ratio 0.9980 (-0.20 %, -0.0119); bars r/i/s 56.99/21.36/6.09 %; claimed r/i/s n/n/n -> not claimed
  **H-trk-ap / Jolt56 W2: pooled 0.9952219962760427, A 1.0088814817905651, B 0.9885882544745751 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 1.3017 us (4519.2575 manifolds/step), Jolt 0.6944 us (8489.0 manifolds/step) -> ours/Jolt 1.875x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- tree vs allpairs W2 pooled: ratio 0.6867 (-31.33 %, -1.8388); bars r/i/s 35.57/19.19/4.45 %; claimed r/i/s n/Y/Y -> not claimed
- window term: Jolt56 this window / window 3 = 1.0220 (5.8973 vs 5.7705 ms; cross-window, context only)
- block term (same window): Jolt56 B vs A: ratio 0.9399 (-6.01 %, -0.3671); bars r/i/s 32.70/8.38/6.10 %; claimed r/i/s n/n/n -> not claimed
- block term: H-trk-tree B vs A: ratio 0.9497 (-5.03 %, -0.2077); bars r/i/s 10.69/4.59/2.20 %; claimed r/i/s n/Y/Y -> not claimed
- block term: H-trk-ap B vs A: ratio 0.9210 (-7.90 %, -0.4869); bars r/i/s 23.43/4.87/4.80 %; claimed r/i/s n/Y/Y -> not claimed

### W=4
- H-trk-tree@W4 [A] [0,500) ms: 3.0390 [2.9068-3.3550] K=5 r/i/s 14.75/11.81/3.81 %
- H-trk-tree@W4 [B] [0,500) ms: 2.7684 [2.7559-2.7824] K=6 r/i/s 0.95/0.61/0.21 %
- H-trk-tree@W4 [pooled] [0,500) ms: 2.7824 [2.7559-3.3550] K=11 r/i/s 21.53/7.78/2.97 %
- H-trk-ap@W4 [A] [0,500) ms: 4.8118 [4.7667-4.8252] K=3 r/i/s 1.22/0.61/0.46 %
- H-trk-ap@W4 [B] [0,500) ms: 4.5547 [4.5484-4.5691] K=6 r/i/s 0.46/0.17/0.08 %
- H-trk-ap@W4 [pooled] [0,500) ms: 4.5600 [4.5484-4.8252] K=9 r/i/s 6.07/4.68/1.13 %
- H-jolt56@W4 [A] [0,500) ms: 3.8928 [3.7579-4.0694] K=5 r/i/s 8.00/3.41/1.83 %
- H-jolt56@W4 [B] [0,500) ms: 3.6533 [3.5250-3.9810] K=6 r/i/s 12.48/3.18/2.31 %
- H-jolt56@W4 [pooled] [0,500) ms: 3.7579 [3.5250-4.0694] K=11 r/i/s 14.49/6.41/1.78 %
- H-trk-tree / Jolt56 W4 [A] [0,500): ratio 0.7807 (-21.93 %, -0.8537); bars r/i/s 33.56/24.59/8.45 %; claimed r/i/s n/n/Y -> not claimed
- H-trk-tree / Jolt56 W4 [B] [0,500): ratio 0.7578 (-24.22 %, -0.8848); bars r/i/s 25.04/6.48/4.65 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W4 [pooled] [0,500): ratio 0.7404 (-25.96 %, -0.9755); bars r/i/s 51.90/20.16/6.93 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 2.8294 [2.7987-3.3179] K=11 r/i/s 18.35/7.43/2.56 %; jolt 3.5574 [3.4071-3.9156] K=11 r/i/s 14.30/6.14/1.75 %; ratio 0.7953 (-20.47 %, -0.7281); bars r/i/s 46.53/19.27/6.21 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled: ours 2.7752 [2.7449-3.3790] K=11 r/i/s 22.85/8.13/3.08 %; jolt 3.7632 [3.5544-4.1079] K=11 r/i/s 14.71/6.73/1.91 %; ratio 0.7375 (-26.25 %, -0.9879); bars r/i/s 54.34/21.11/7.25 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-tree / Jolt56 W4: pooled 0.7404070171091224, A 0.7806838980849898, B 0.7577946340919698 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.6141 us (4519.2575 manifolds/step), Jolt 0.4433 us (8489.0 manifolds/step) -> ours/Jolt 1.385x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- H-trk-ap / Jolt56 W4 [A] [0,500): ratio 1.2361 (+23.61 %, +0.9190); bars r/i/s 16.19/6.93/3.78 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W4 [B] [0,500): ratio 1.2468 (+24.68 %, +0.9015); bars r/i/s 24.98/6.37/4.63 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W4 [pooled] [0,500): ratio 1.2135 (+21.35 %, +0.8022); bars r/i/s 31.42/15.88/4.22 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 4.5980 [4.5620-4.9234] K=9 r/i/s 7.86/4.13/1.33 %; jolt 3.5574 [3.4071-3.9156] K=11 r/i/s 14.30/6.14/1.75 %; ratio 1.2925 (+29.25 %, +1.0405); bars r/i/s 32.63/14.79/4.40 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled: ours 4.5510 [4.5360-4.8219] K=9 r/i/s 6.28/4.04/1.11 %; jolt 3.7632 [3.5544-4.1079] K=11 r/i/s 14.71/6.73/1.91 %; ratio 1.2094 (+20.94 %, +0.7879); bars r/i/s 31.99/15.69/4.42 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-ap / Jolt56 W4: pooled 1.213467024082228, A 1.2360733595305968, B 1.2467570032306516 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 1.0070 us (4519.2575 manifolds/step), Jolt 0.4433 us (8489.0 manifolds/step) -> ours/Jolt 2.272x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- tree vs allpairs W4 pooled: ratio 0.6102 (-38.98 %, -1.7777); bars r/i/s 44.74/18.15/6.36 %; claimed r/i/s n/Y/Y -> not claimed
- window term: Jolt56 this window / window 3 = 1.0493 (3.7579 vs 3.5814 ms; cross-window, context only)
- block term (same window): Jolt56 B vs A: ratio 0.9385 (-6.15 %, -0.2395); bars r/i/s 29.66/9.33/5.90 %; claimed r/i/s n/n/Y -> not claimed
- block term: H-trk-tree B vs A: ratio 0.9110 (-8.90 %, -0.2706); bars r/i/s 29.56/23.66/7.63 %; claimed r/i/s n/n/Y -> not claimed
- block term: H-trk-ap B vs A: ratio 0.9466 (-5.34 %, -0.2570); bars r/i/s 2.60/1.26/0.94 %; claimed r/i/s Y/Y/Y -> CLAIMED

### W=8
- H-trk-tree@W8 [A] [0,500) ms: 2.4529 [2.3290-2.9626] K=5 r/i/s 25.83/2.75/5.73 %
- H-trk-tree@W8 [B] [0,500) ms: 2.1878 [2.1850-2.2027] K=6 r/i/s 0.81/0.08/0.15 %
- H-trk-tree@W8 [pooled] [0,500) ms: 2.2027 [2.1850-2.9626] K=11 r/i/s 35.30/10.99/4.05 %
- H-trk-ap@W8 [A] [0,500) ms: 4.3273 [4.2040-4.4244] K=5 r/i/s 5.09/2.20/1.13 %
- H-trk-ap@W8 [B] [0,500) ms: 3.9764 [3.9711-3.9923] K=6 r/i/s 0.53/0.29/0.11 %
- H-trk-ap@W8 [pooled] [0,500) ms: 3.9923 [3.9711-4.4244] K=11 r/i/s 11.35/7.64/1.68 %
- H-jolt56@W8 [A] [0,500) ms: 2.9357 [2.8382-3.2679] K=6 r/i/s 14.64/3.56/2.72 %
- H-jolt56@W8 [B] [0,500) ms: 2.5085 [2.4711-2.5936] K=6 r/i/s 4.88/1.87/0.93 %
- H-jolt56@W8 [pooled] [0,500) ms: 2.7159 [2.4711-3.2679] K=12 r/i/s 29.34/14.84/3.49 %
- H-trk-tree / Jolt56 W8 [A] [0,500): ratio 0.8355 (-16.45 %, -0.4829); bars r/i/s 59.38/8.99/12.69 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W8 [B] [0,500): ratio 0.8721 (-12.79 %, -0.3207); bars r/i/s 9.90/3.74/1.88 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W8 [pooled] [0,500): ratio 0.8111 (-18.89 %, -0.5132); bars r/i/s 91.81/36.93/10.70 %; claimed r/i/s n/n/Y -> not claimed
  - [0..100) pooled: ours 2.2690 [2.2109-2.7320] K=11 r/i/s 22.96/11.15/3.09 %; jolt 2.4637 [2.2710-2.8552] K=12 r/i/s 23.71/12.36/2.82 %; ratio 0.9209 (-7.91 %, -0.1948); bars r/i/s 66.02/33.29/8.36 %; claimed r/i/s n/n/n -> not claimed
  - [100..500) pooled: ours 2.1864 [2.1698-3.0437] K=11 r/i/s 39.97/9.74/4.48 %; jolt 2.7762 [2.5211-3.3711] K=12 r/i/s 30.62/15.72/3.65 %; ratio 0.7876 (-21.24 %, -0.5898); bars r/i/s 100.70/36.99/11.56 %; claimed r/i/s n/n/Y -> not claimed
  **H-trk-tree / Jolt56 W8: pooled 0.8110560079396286, A 0.8355206646821298, B 0.8721405526437774 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.4838 us (4519.2575 manifolds/step), Jolt 0.3270 us (8489.0 manifolds/step) -> ours/Jolt 1.479x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- H-trk-ap / Jolt56 W8 [A] [0,500): ratio 1.4740 (+47.40 %, +1.3916); bars r/i/s 31.00/8.37/5.90 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [B] [0,500): ratio 1.5851 (+58.51 %, +1.4679); bars r/i/s 9.82/3.78/1.87 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [pooled] [0,500): ratio 1.4700 (+47.00 %, +1.2764); bars r/i/s 62.92/33.38/7.75 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 4.0310 [3.9779-4.6797] K=11 r/i/s 17.41/7.13/2.06 %; jolt 2.4637 [2.2710-2.8552] K=12 r/i/s 23.71/12.36/2.82 %; ratio 1.6362 (+63.62 %, +1.5673); bars r/i/s 58.83/28.54/6.98 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [100..500) pooled: ours 3.9959 [3.9612-4.4495] K=11 r/i/s 12.22/6.49/1.69 %; jolt 2.7762 [2.5211-3.3711] K=12 r/i/s 30.62/15.72/3.65 %; ratio 1.4393 (+43.93 %, +1.2197); bars r/i/s 65.93/34.01/8.04 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-ap / Jolt56 W8: pooled 1.4699589487120508, A 1.4740070028893313, B 1.5851494983554377 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.8842 us (4519.2575 manifolds/step), Jolt 0.3270 us (8489.0 manifolds/step) -> ours/Jolt 2.704x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- tree vs allpairs W8 pooled: ratio 0.5518 (-44.82 %, -1.7895); bars r/i/s 74.17/26.77/8.78 %; claimed r/i/s n/Y/Y -> not claimed
- window term: Jolt56 this window / window 3 = 1.0571 (2.7159 vs 2.5693 ms; cross-window, context only)
- block term (same window): Jolt56 B vs A: ratio 0.8545 (-14.55 %, -0.4272); bars r/i/s 30.86/8.04/5.76 %; claimed r/i/s n/Y/Y -> not claimed
- block term: H-trk-tree B vs A: ratio 0.8919 (-10.81 %, -0.2651); bars r/i/s 51.69/5.49/11.47 %; claimed r/i/s n/Y/n -> not claimed
- block term: H-trk-ap B vs A: ratio 0.9189 (-8.11 %, -0.3509); bars r/i/s 10.24/4.44/2.27 %; claimed r/i/s n/Y/Y -> not claimed

### W=16
- H-trk-tree@W16 [A] [0,500) ms: 2.7452 [2.6322-2.9531] K=4 r/i/s 11.69/6.82/3.33 %
- H-trk-tree@W16 [B] [0,500) ms: 2.3628 [2.3507-2.5798] K=6 r/i/s 9.69/6.22/2.34 %
- H-trk-tree@W16 [pooled] [0,500) ms: 2.5665 [2.3507-2.9531] K=10 r/i/s 23.47/11.73/3.26 %
- H-trk-ap@W16 [A] [0,500) ms: 4.6552 [4.5748-4.8962] K=5 r/i/s 6.91/1.44/1.47 %
- H-trk-ap@W16 [B] [0,500) ms: 4.1251 [4.1139-4.3143] K=6 r/i/s 4.86/0.45/0.97 %
- H-trk-ap@W16 [pooled] [0,500) ms: 4.3143 [4.1139-4.8962] K=11 r/i/s 18.13/12.10/2.60 %
- H-jolt56@W16 [A] [0,500) ms: 3.1752 [2.8904-5.3684] K=6 r/i/s 78.04/12.51/15.11 %
- H-jolt56@W16 [B] [0,500) ms: 2.4075 [2.3421-2.4624] K=6 r/i/s 5.00/2.57/0.99 %
- H-jolt56@W16 [pooled] [0,500) ms: 2.6764 [2.3421-5.3684] K=12 r/i/s 113.07/25.73/11.52 %
- H-trk-tree / Jolt56 W16 [A] [0,500): ratio 0.8646 (-13.54 %, -0.4300); bars r/i/s 157.82/28.49/30.95 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [B] [0,500): ratio 0.9814 (-1.86 %, -0.0448); bars r/i/s 21.81/13.46/5.08 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [pooled] [0,500): ratio 0.9589 (-4.11 %, -0.1099); bars r/i/s 230.97/56.56/23.95 %; claimed r/i/s n/n/n -> not claimed
  - [0..100) pooled: ours 2.4870 [2.4157-3.1172] K=10 r/i/s 28.21/15.16/3.92 %; jolt 2.3614 [2.1477-3.5399] K=12 r/i/s 58.96/15.19/5.95 %; ratio 1.0532 (+5.32 %, +0.1256); bars r/i/s 130.72/42.92/14.25 %; claimed r/i/s n/n/n -> not claimed
  - [100..500) pooled: ours 2.5891 [2.3345-2.9121] K=10 r/i/s 22.31/10.91/3.19 %; jolt 2.7462 [2.3907-5.8255] K=12 r/i/s 125.08/28.24/12.78 %; ratio 0.9428 (-5.72 %, -0.1571); bars r/i/s 254.10/60.54/26.35 %; claimed r/i/s n/n/n -> not claimed
  **H-trk-tree / Jolt56 W16: pooled 0.9589381055389421, A 0.8645664030616109, B 0.9814066274299598 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.5729 us (4519.2575 manifolds/step), Jolt 0.3235 us (8489.0 manifolds/step) -> ours/Jolt 1.771x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- H-trk-ap / Jolt56 W16 [A] [0,500): ratio 1.4661 (+46.61 %, +1.4800); bars r/i/s 156.69/25.18/30.37 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W16 [B] [0,500): ratio 1.7134 (+71.34 %, +1.7176); bars r/i/s 13.94/5.22/2.77 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W16 [pooled] [0,500): ratio 1.6120 (+61.20 %, +1.6379); bars r/i/s 229.04/56.87/23.62 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled: ours 4.4554 [4.1575-4.9111] K=11 r/i/s 16.92/11.85/2.45 %; jolt 2.3614 [2.1477-3.5399] K=12 r/i/s 58.96/15.19/5.95 %; ratio 1.8868 (+88.68 %, +2.0940); bars r/i/s 122.67/38.53/12.87 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled: ours 4.2790 [4.0997-4.8925] K=11 r/i/s 18.53/12.34/2.65 %; jolt 2.7462 [2.3907-5.8255] K=12 r/i/s 125.08/28.24/12.78 %; ratio 1.5582 (+55.82 %, +1.5329); bars r/i/s 252.88/61.64/26.11 %; claimed r/i/s n/n/Y -> not claimed
  **H-trk-ap / Jolt56 W16: pooled 1.6119908690318083, A 1.466119688879246, B 1.7134292506680613 -> does NOT hold (not claimed in both blocks and pooled, same direction)**
  - per manifold [100,500) pooled: ours 0.9468 us (4519.2575 manifolds/step), Jolt 0.3235 us (8489.0 manifolds/step) -> ours/Jolt 2.927x (Jolt/ours manifolds 1.878); the claim is the [100,500) step claim above
- tree vs allpairs W16 pooled: ratio 0.5949 (-40.51 %, -1.7478); bars r/i/s 59.32/33.70/8.34 %; claimed r/i/s n/Y/Y -> not claimed
- window term: Jolt56 this window / window 3 = 1.1205 (2.6764 vs 2.3885 ms; cross-window, context only)
- block term (same window): Jolt56 B vs A: ratio 0.7582 (-24.18 %, -0.7677); bars r/i/s 156.40/25.54/30.29 %; claimed r/i/s n/n/n -> not claimed
- block term: H-trk-tree B vs A: ratio 0.8607 (-13.93 %, -0.3824); bars r/i/s 30.37/18.45/8.14 %; claimed r/i/s n/n/Y -> not claimed
- block term: H-trk-ap B vs A: ratio 0.8861 (-11.39 %, -0.5301); bars r/i/s 16.89/3.02/3.52 %; claimed r/i/s n/Y/Y -> not claimed

### Scaling T(1)/T(W), pooled medians
- H-trk-tree: W1 1.000, W2 1.500, W4 2.172, W8 2.744, W16 2.355
- H-trk-ap: W1 1.000, W2 1.319, W4 1.698, W8 1.939, W16 1.794
- H-jolt56: W1 1.000, W2 1.760, W4 2.763, W8 3.823, W16 3.879

**Decision P1 (W8, the default row against Jolt v5.6.0, pooled K=12, must hold in each block):** {'ratio_pooled': 0.8110560079396286, 'ratio_A': 0.8355206646821298, 'ratio_B': 0.8721405526437774, 'verdict': 'does NOT hold (not claimed in both blocks and pooled, same direction)'}

## P2 L9 C4 G-TW on 989ca0f0: same binary, --contact-reuse off vs on
- JA W1 [0..500): off 18.3854 [18.3518-18.9680] K=6 r/i/s 3.35/1.72/0.74 %; on 16.6838 [16.6192-16.9076] K=6 r/i/s 1.73/0.28/0.31 %; on vs off ratio 0.9075 (-9.25 %, -1.7016); bars r/i/s 7.54/3.49/1.61 %; claimed r/i/s Y/Y/Y -> CLAIMED
- JA W1 [0..100): off 18.2703 [18.0867-18.9076] K=6 r/i/s 4.49/0.65/0.81 %; on 17.4609 [17.3689-17.6349] K=6 r/i/s 1.52/0.38/0.26 %; on vs off ratio 0.9557 (-4.43 %, -0.8093); bars r/i/s 9.49/1.50/1.71 %; claimed r/i/s n/Y/Y -> not claimed
- JA W1 [100..500): off 18.4127 [18.3856-18.9831] K=6 r/i/s 3.24/2.37/0.82 %; on 16.4753 [16.4125-16.7739] K=6 r/i/s 2.19/0.41/0.41 %; on vs off ratio 0.8948 (-10.52 %, -1.9374); bars r/i/s 7.83/4.81/1.84 %; claimed r/i/s Y/Y/Y -> CLAIMED
- JA W2 [0..500): off 11.2753 [11.1869-12.2819] K=6 r/i/s 9.71/4.35/2.04 %; on 10.5342 [10.2629-11.1881] K=6 r/i/s 8.78/4.81/1.78 %; on vs off ratio 0.9343 (-6.57 %, -0.7412); bars r/i/s 26.19/12.97/5.41 %; claimed r/i/s n/n/Y -> not claimed
- JA W2 [0..100): off 11.2230 [11.1660-12.2503] K=6 r/i/s 9.66/2.43/1.93 %; on 10.9218 [10.7868-11.6185] K=6 r/i/s 7.62/4.45/1.67 %; on vs off ratio 0.9732 (-2.68 %, -0.3013); bars r/i/s 24.60/10.15/5.11 %; claimed r/i/s n/n/n -> not claimed
- JA W2 [100..500): off 11.2915 [11.1791-12.2898] K=6 r/i/s 9.84/4.80/2.08 %; on 10.4373 [10.1319-11.0804] K=6 r/i/s 9.09/4.90/1.82 %; on vs off ratio 0.9243 (-7.57 %, -0.8543); bars r/i/s 26.78/13.73/5.53 %; claimed r/i/s n/n/Y -> not claimed
- JA W4 [0..500): off 7.6776 [7.5668-8.3523] K=6 r/i/s 10.23/5.15/2.15 %; on 7.1691 [7.1298-7.9901] K=6 r/i/s 12.00/8.20/2.96 %; on vs off ratio 0.9338 (-6.62 %, -0.5086); bars r/i/s 31.54/19.37/7.32 %; claimed r/i/s n/n/n -> not claimed
- JA W4 [0..100): off 7.5859 [7.5433-8.4399] K=6 r/i/s 11.82/6.91/2.74 %; on 7.4306 [7.4138-8.3126] K=6 r/i/s 12.10/7.91/2.97 %; on vs off ratio 0.9795 (-2.05 %, -0.1553); bars r/i/s 33.82/21.02/8.09 %; claimed r/i/s n/n/n -> not claimed
- JA W4 [100..500): off 7.7088 [7.5639-8.3304] K=6 r/i/s 9.94/4.82/2.02 %; on 7.1037 [7.0589-7.9380] K=6 r/i/s 12.38/7.98/2.97 %; on vs off ratio 0.9215 (-7.85 %, -0.6051); bars r/i/s 31.75/18.63/7.18 %; claimed r/i/s n/n/Y -> not claimed
- JA W8 [0..500): off 5.7742 [5.7649-6.8497] K=6 r/i/s 18.79/9.34/4.23 %; on 5.7694 [5.6087-6.4809] K=6 r/i/s 15.12/8.49/3.20 %; on vs off ratio 0.9992 (-0.08 %, -0.0048); bars r/i/s 48.23/25.24/10.61 %; claimed r/i/s n/n/n -> not claimed
- JA W8 [0..100): off 5.8041 [5.7549-6.9815] K=6 r/i/s 21.13/6.92/4.35 %; on 5.7982 [5.7027-7.1615] K=6 r/i/s 25.16/14.99/5.83 %; on vs off ratio 0.9990 (-0.10 %, -0.0059); bars r/i/s 65.72/33.03/14.54 %; claimed r/i/s n/n/n -> not claimed
- JA W8 [100..500): off 5.7693 [5.7622-6.8168] K=6 r/i/s 18.28/9.93/4.23 %; on 5.7622 [5.5852-6.3108] K=6 r/i/s 12.59/6.96/2.65 %; on vs off ratio 0.9988 (-0.12 %, -0.0071); bars r/i/s 44.39/24.26/9.98 %; claimed r/i/s n/n/n -> not claimed
- JA W16 [0..500): off 5.6599 [5.6409-6.9361] K=6 r/i/s 22.88/9.00/4.88 %; on 5.5780 [5.5243-6.5378] K=6 r/i/s 18.17/11.76/4.35 %; on vs off ratio 0.9855 (-1.45 %, -0.0819); bars r/i/s 58.44/29.62/13.08 %; claimed r/i/s n/n/n -> not claimed
- JA W16 [0..100): off 5.7207 [5.6813-6.7839] K=6 r/i/s 19.27/8.92/4.21 %; on 5.6708 [5.6288-6.7443] K=6 r/i/s 19.67/14.64/5.09 %; on vs off ratio 0.9913 (-0.87 %, -0.0498); bars r/i/s 55.08/34.30/13.21 %; claimed r/i/s n/n/n -> not claimed
- JA W16 [100..500): off 5.6501 [5.6211-6.9742] K=6 r/i/s 23.95/9.07/5.06 %; on 5.5601 [5.4973-6.4874] K=6 r/i/s 17.81/11.18/4.17 %; on vs off ratio 0.9841 (-1.59 %, -0.0900); bars r/i/s 59.69/28.79/13.11 %; claimed r/i/s n/n/n -> not claimed
- JD W1 [0..500): off 7.7922 [7.6252-8.1873] K=6 r/i/s 7.21/1.95/1.31 %; on 6.1390 [6.0491-6.7890] K=6 r/i/s 12.05/1.98/2.32 %; on vs off ratio 0.7878 (-21.22 %, -1.6532); bars r/i/s 28.09/5.56/5.32 %; claimed r/i/s n/Y/Y -> not claimed
- JD W1 [0..100): off 7.7323 [7.6473-8.1195] K=6 r/i/s 6.11/1.05/1.16 %; on 6.9682 [6.9334-7.3781] K=6 r/i/s 6.38/0.51/1.25 %; on vs off ratio 0.9012 (-9.88 %, -0.7641); bars r/i/s 17.67/2.34/3.41 %; claimed r/i/s n/Y/Y -> not claimed
- JD W1 [100..500): off 7.8058 [7.6197-8.2042] K=6 r/i/s 7.49/2.19/1.35 %; on 5.9283 [5.8281-6.6417] K=6 r/i/s 13.73/2.54/2.64 %; on vs off ratio 0.7595 (-24.05 %, -1.8775); bars r/i/s 31.27/6.70/5.93 %; claimed r/i/s n/Y/Y -> not claimed
- JD W2 [0..500): off 5.6928 [5.6613-6.1519] K=6 r/i/s 8.62/2.27/1.72 %; on 4.8305 [4.8010-5.5631] K=6 r/i/s 15.78/1.79/3.18 %; on vs off ratio 0.8485 (-15.15 %, -0.8623); bars r/i/s 35.96/5.78/7.23 %; claimed r/i/s n/Y/Y -> not claimed
- JD W2 [0..100): off 5.6869 [5.6698-6.1701] K=6 r/i/s 8.80/0.32/1.79 %; on 5.3358 [5.2922-6.2622] K=6 r/i/s 18.18/1.33/3.66 %; on vs off ratio 0.9383 (-6.17 %, -0.3511); bars r/i/s 40.39/2.73/8.15 %; claimed r/i/s n/Y/n -> not claimed
- JD W2 [100..500): off 5.6922 [5.6574-6.1473] K=6 r/i/s 8.61/2.87/1.74 %; on 4.7047 [4.6762-5.3884] K=6 r/i/s 15.14/1.91/3.05 %; on vs off ratio 0.8265 (-17.35 %, -0.9875); bars r/i/s 34.82/6.88/7.02 %; claimed r/i/s n/Y/Y -> not claimed
- JD W4 [0..500): off 4.7130 [4.5380-5.1835] K=6 r/i/s 13.70/9.94/3.12 %; on 4.1112 [4.1058-4.9760] K=5 r/i/s 21.17/10.98/5.32 %; on vs off ratio 0.8723 (-12.77 %, -0.6019); bars r/i/s 50.43/29.62/12.33 %; claimed r/i/s n/n/Y -> not claimed
- JD W4 [0..100): off 4.7792 [4.5693-5.5752] K=6 r/i/s 21.05/10.61/4.30 %; on 4.4187 [4.4006-4.9052] K=5 r/i/s 11.42/7.44/2.96 %; on vs off ratio 0.9246 (-7.54 %, -0.3605); bars r/i/s 47.89/25.92/10.43 %; claimed r/i/s n/n/n -> not claimed
- JD W4 [100..500): off 4.6965 [4.5301-5.0889] K=6 r/i/s 11.90/9.72/2.86 %; on 4.0362 [4.0276-5.0351] K=5 r/i/s 24.96/10.85/6.14 %; on vs off ratio 0.8594 (-14.06 %, -0.6602); bars r/i/s 55.31/29.13/13.55 %; claimed r/i/s n/n/Y -> not claimed
- JD W8 [0..500): off 4.1624 [3.9901-4.7813] K=6 r/i/s 19.01/11.33/4.07 %; on 3.9741 [3.7638-4.2370] K=6 r/i/s 11.91/10.92/3.05 %; on vs off ratio 0.9548 (-4.52 %, -0.1883); bars r/i/s 44.86/31.47/10.18 %; claimed r/i/s n/n/n -> not claimed
- JD W8 [0..100): off 4.1744 [4.0205-4.5556] K=6 r/i/s 12.82/9.30/2.92 %; on 4.1034 [3.9016-4.3295] K=6 r/i/s 10.43/9.30/2.66 %; on vs off ratio 0.9830 (-1.70 %, -0.0711); bars r/i/s 33.05/26.30/7.89 %; claimed r/i/s n/n/n -> not claimed
- JD W8 [100..500): off 4.1616 [3.9769-4.8377] K=6 r/i/s 20.68/11.79/4.37 %; on 3.9345 [3.7264-4.2175] K=6 r/i/s 12.48/11.42/3.18 %; on vs off ratio 0.9454 (-5.46 %, -0.2271); bars r/i/s 48.32/32.83/10.81 %; claimed r/i/s n/n/n -> not claimed
- JD W16 [0..500): off 4.2442 [4.1501-4.9031] K=6 r/i/s 17.74/9.86/3.89 %; on 4.0247 [3.9643-5.0217] K=6 r/i/s 26.27/10.07/5.43 %; on vs off ratio 0.9483 (-5.17 %, -0.2195); bars r/i/s 63.40/28.18/13.36 %; claimed r/i/s n/n/n -> not claimed
- JD W16 [0..100): off 4.2445 [4.1955-4.9445] K=6 r/i/s 17.65/6.52/3.62 %; on 4.3345 [4.1054-5.2841] K=6 r/i/s 27.19/9.10/5.33 %; on vs off ratio 1.0212 (+2.12 %, +0.0899); bars r/i/s 64.83/22.39/12.88 %; claimed r/i/s n/n/n -> not claimed
- JD W16 [100..500): off 4.2444 [4.1339-4.8927] K=6 r/i/s 17.88/10.79/3.99 %; on 3.9488 [3.9290-4.9561] K=6 r/i/s 26.01/10.37/5.54 %; on vs off ratio 0.9304 (-6.96 %, -0.2955); bars r/i/s 63.12/29.93/13.65 %; claimed r/i/s n/n/n -> not claimed
- R W1 [600..1100): off 10.7594 [10.7508-10.8667] K=5 r/i/s 1.08/0.80/0.29 %; on 7.8482 [7.8048-8.5022] K=6 r/i/s 8.89/1.24/1.76 %; on vs off ratio 0.7294 (-27.06 %, -2.9113); bars r/i/s 17.90/2.96/3.57 %; claimed r/i/s Y/Y/Y -> CLAIMED
- R W2 [600..1100): off 7.4940 [7.3729-8.3190] K=6 r/i/s 12.62/2.07/2.44 %; on 6.5523 [6.1217-6.9655] K=6 r/i/s 12.88/9.05/2.85 %; on vs off ratio 0.8743 (-12.57 %, -0.9417); bars r/i/s 36.07/18.57/7.51 %; claimed r/i/s n/n/Y -> not claimed
- R W4 [600..1100): off 6.2568 [5.8187-6.9797] K=6 r/i/s 18.56/13.50/4.16 %; on 5.2840 [5.0916-6.1540] K=6 r/i/s 20.11/13.71/4.57 %; on vs off ratio 0.8445 (-15.55 %, -0.9728); bars r/i/s 54.72/38.48/12.36 %; claimed r/i/s n/n/Y -> not claimed
- R W8 [600..1100): off 4.9790 [4.8844-5.5367] K=5 r/i/s 13.10/3.11/3.05 %; on 5.0100 [4.5892-5.5989] K=6 r/i/s 20.15/17.02/4.81 %; on vs off ratio 1.0062 (+0.62 %, +0.0311); bars r/i/s 48.08/34.61/11.39 %; claimed r/i/s n/n/n -> not claimed
- R W16 [600..1100): off 5.0840 [5.0653-6.2804] K=6 r/i/s 23.90/0.68/4.93 %; on 4.9601 [4.8769-6.2402] K=6 r/i/s 27.49/4.84/5.43 %; on vs off ratio 0.9756 (-2.44 %, -0.1239); bars r/i/s 72.85/9.77/14.66 %; claimed r/i/s n/n/n -> not claimed
**Decision P2 (realized-gain rule):** realized dT(1)[100,500) on J-A = 1.9374 ms against the bar 0.6 x 1.4994 = 0.8997 ms (window 6 prediction [1.4994, 1.6782] ms): PASS
**Decision P2 (G-TW: a claimed regression at any W blocks):** no claimed regression at any W

### The armed table (J-A, [100,500), per-process mean per step, median over K)
- L9-JA-a-off W1: manifolds/step 4519.26 [4519.26-4519.26] K=6 r/i/s 0.00/0.00/0.00 %; pairs/step 9559.22 [9559.22-9559.22] K=6 r/i/s 0.00/0.00/0.00 %; reused/step 0.00 [0.00-0.00] K=6 r/i/s 0.00/0.00/0.00 %
  - wall                 18.4703 ms/step [18.4101-19.0140]; 4.0870 us/manifold; 1.9322 us/pair
  - bp                   2.0838 ms/step [2.0056-2.1028]; 0.4611 us/manifold; 0.2180 us/pair
  - np                   2.6112 ms/step [2.5788-2.7104]; 0.5778 us/manifold; 0.2732 us/pair
  - graph                0.1426 ms/step [0.1410-0.1654]; 0.0316 us/manifold; 0.0149 us/pair
  - solve                13.4960 ms/step [13.4431-13.9833]; 2.9863 us/manifold; 1.4118 us/pair
  - setup(solve_build)   0.3686 ms/step [0.3308-0.6472]; 0.0816 us/manifold; 0.0386 us/pair
  - colours wide         12.4500 ms/step [12.4220-12.5593]; 2.7549 us/manifold; 1.3024 us/pair
  - colours narrow       0.0277 ms/step [0.0275-0.0280]; 0.0061 us/manifold; 0.0029 us/pair
  - warm_apply           0.5364 ms/step [0.5332-0.5708]; 0.1187 us/manifold; 0.0561 us/pair
  - store                0.0327 ms/step [0.0265-0.0785]; 0.0072 us/manifold; 0.0034 us/pair
  - sys_sum              18.3795 ms/step [18.3111-18.9499]; 4.0669 us/manifold; 1.9227 us/pair
- L9-JA-a-on W1: manifolds/step 4467.66 [4467.66-4467.66] K=6 r/i/s 0.00/0.00/0.00 %; pairs/step 9545.12 [9545.12-9545.12] K=6 r/i/s 0.00/0.00/0.00 %; reused/step 4457.51 [4457.51-4457.51] K=6 r/i/s 0.00/0.00/0.00 %
  - wall                 16.4320 ms/step [16.2298-17.0714]; 3.6780 us/manifold; 1.7215 us/pair
  - bp                   2.0781 ms/step [2.0023-2.0961]; 0.4651 us/manifold; 0.2177 us/pair
  - np                   0.7511 ms/step [0.6892-0.8776]; 0.1681 us/manifold; 0.0787 us/pair
  - graph                0.1195 ms/step [0.1140-0.1732]; 0.0268 us/manifold; 0.0125 us/pair
  - solve                13.3327 ms/step [13.3172-13.8869]; 2.9843 us/manifold; 1.3968 us/pair
  - setup(solve_build)   0.3211 ms/step [0.3064-0.6591]; 0.0719 us/manifold; 0.0336 us/pair
  - colours wide         12.2095 ms/step [12.1856-12.3269]; 2.7329 us/manifold; 1.2791 us/pair
  - colours narrow       0.1788 ms/step [0.1778-0.1808]; 0.0400 us/manifold; 0.0187 us/pair
  - warm_apply           0.5263 ms/step [0.5190-0.5548]; 0.1178 us/manifold; 0.0551 us/pair
  - store                0.0261 ms/step [0.0244-0.0721]; 0.0058 us/manifold; 0.0027 us/pair
  - sys_sum              16.3363 ms/step [16.1786-17.0171]; 3.6566 us/manifold; 1.7115 us/pair
  - wall on vs off W1: ratio 0.8896 (-11.04 %, -2.0383); bars r/i/s 12.15/5.70/2.48 %; claimed r/i/s n/Y/Y -> not claimed
  - bp on vs off W1: ratio 0.9973 (-0.27 %, -0.0057); bars r/i/s 12.98/5.80/2.58 %; claimed r/i/s n/n/n -> not claimed
  - np on vs off W1: ratio 0.2877 (-71.23 %, -1.8601); bars r/i/s 51.15/9.21/8.74 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - graph on vs off W1: ratio 0.8380 (-16.20 %, -0.0231); bars r/i/s 104.92/31.08/20.73 %; claimed r/i/s n/n/n -> not claimed
  - solve on vs off W1: ratio 0.9879 (-1.21 %, -0.1634); bars r/i/s 11.71/7.12/2.73 %; claimed r/i/s n/n/n -> not claimed
  - setup(solve_build) on vs off W1: ratio 0.8710 (-12.90 %, -0.0476); bars r/i/s 278.79/147.95/61.92 %; claimed r/i/s n/n/n -> not claimed
  - colours wide on vs off W1: ratio 0.9807 (-1.93 %, -0.2405); bars r/i/s 3.20/2.19/0.74 %; claimed r/i/s n/n/Y -> not claimed
  - colours narrow on vs off W1: ratio 6.4637 (+546.37 %, +0.1511); bars r/i/s 4.70/3.18/1.03 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - warm_apply on vs off W1: ratio 0.9811 (-1.89 %, -0.0102); bars r/i/s 19.53/9.80/4.15 %; claimed r/i/s n/n/n -> not claimed
  - store on vs off W1: ratio 0.7977 (-20.23 %, -0.0066); bars r/i/s 485.27/277.38/110.70 %; claimed r/i/s n/n/n -> not claimed
  - sys_sum on vs off W1: ratio 0.8888 (-11.12 %, -2.0432); bars r/i/s 12.40/6.17/2.60 %; claimed r/i/s n/Y/Y -> not claimed
- L9-JA-a-off W8: manifolds/step 4519.26 [4519.26-4519.26] K=5 r/i/s 0.00/0.00/0.00 %; pairs/step 9559.22 [9559.22-9559.22] K=5 r/i/s 0.00/0.00/0.00 %; reused/step 0.00 [0.00-0.00] K=5 r/i/s 0.00/0.00/0.00 %
  - wall                 5.8149 ms/step [5.7820-6.2780]; 1.2867 us/manifold; 0.6083 us/pair
  - bp                   1.9614 ms/step [1.9610-2.0327]; 0.4340 us/manifold; 0.2052 us/pair
  - np                   0.4526 ms/step [0.4515-0.5166]; 0.1002 us/manifold; 0.0473 us/pair
  - graph                0.1200 ms/step [0.1196-0.1436]; 0.0266 us/manifold; 0.0126 us/pair
  - solve                3.2090 ms/step [3.1756-3.5030]; 0.7101 us/manifold; 0.3357 us/pair
  - setup(solve_build)   0.3469 ms/step [0.3135-0.4312]; 0.0768 us/manifold; 0.0363 us/pair
  - colours wide         2.1630 ms/step [2.1494-2.3041]; 0.4786 us/manifold; 0.2263 us/pair
  - colours narrow       0.0299 ms/step [0.0299-0.0311]; 0.0066 us/manifold; 0.0031 us/pair
  - warm_apply           0.5430 ms/step [0.5421-0.5798]; 0.1201 us/manifold; 0.0568 us/pair
  - store                0.0350 ms/step [0.0255-0.0448]; 0.0077 us/manifold; 0.0037 us/pair
  - sys_sum              5.7819 ms/step [5.7488-6.2397]; 1.2794 us/manifold; 0.6048 us/pair
- L9-JA-a-on W8: manifolds/step 4467.66 [4467.66-4467.66] K=6 r/i/s 0.00/0.00/0.00 %; pairs/step 9545.12 [9545.12-9545.12] K=6 r/i/s 0.00/0.00/0.00 %; reused/step 4457.51 [4457.51-4457.51] K=6 r/i/s 0.00/0.00/0.00 %
  - wall                 5.6127 ms/step [5.6049-6.5155]; 1.2563 us/manifold; 0.5880 us/pair
  - bp                   1.9644 ms/step [1.9595-2.0887]; 0.4397 us/manifold; 0.2058 us/pair
  - np                   0.1591 ms/step [0.1571-0.2984]; 0.0356 us/manifold; 0.0167 us/pair
  - graph                0.1158 ms/step [0.1153-0.1525]; 0.0259 us/manifold; 0.0121 us/pair
  - solve                3.3015 ms/step [3.2977-3.8872]; 0.7390 us/manifold; 0.3459 us/pair
  - setup(solve_build)   0.3049 ms/step [0.3029-0.5096]; 0.0682 us/manifold; 0.0319 us/pair
  - colours wide         2.1471 ms/step [2.1438-2.4229]; 0.4806 us/manifold; 0.2249 us/pair
  - colours narrow       0.1925 ms/step [0.1921-0.2010]; 0.0431 us/manifold; 0.0202 us/pair
  - warm_apply           0.5286 ms/step [0.5266-0.5852]; 0.1183 us/manifold; 0.0554 us/pair
  - store                0.0266 ms/step [0.0261-0.0563]; 0.0059 us/manifold; 0.0028 us/pair
  - sys_sum              5.5801 ms/step [5.5717-6.4751]; 1.2490 us/manifold; 0.5846 us/pair
  - wall on vs off W8: ratio 0.9652 (-3.48 %, -0.2021); bars r/i/s 36.66/11.50/7.88 %; claimed r/i/s n/n/n -> not claimed
  - bp on vs off W8: ratio 1.0015 (+0.15 %, +0.0030); bars r/i/s 15.05/1.51/3.20 %; claimed r/i/s n/n/n -> not claimed
  - np on vs off W8: ratio 0.3516 (-64.84 %, -0.2935); bars r/i/s 179.88/36.28/36.85 %; claimed r/i/s n/Y/Y -> not claimed
  - graph on vs off W8: ratio 0.9646 (-3.54 %, -0.0042); bars r/i/s 75.72/16.88/16.25 %; claimed r/i/s n/n/n -> not claimed
  - solve on vs off W8: ratio 1.0288 (+2.88 %, +0.0924); bars r/i/s 41.13/15.92/9.01 %; claimed r/i/s n/n/n -> not claimed
  - setup(solve_build) on vs off W8: ratio 0.8790 (-12.10 %, -0.0420); bars r/i/s 151.61/57.07/32.48 %; claimed r/i/s n/n/n -> not claimed
  - colours wide on vs off W8: ratio 0.9927 (-0.73 %, -0.0159); bars r/i/s 29.67/12.59/6.57 %; claimed r/i/s n/n/n -> not claimed
  - colours narrow on vs off W8: ratio 6.4332 (+543.32 %, +0.1626); bars r/i/s 12.20/7.61/2.96 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - warm_apply on vs off W8: ratio 0.9735 (-2.65 %, -0.0144); bars r/i/s 26.19/3.08/5.61 %; claimed r/i/s n/n/n -> not claimed
  - store on vs off W8: ratio 0.7586 (-24.14 %, -0.0085); bars r/i/s 252.48/84.50/53.28 %; claimed r/i/s n/n/n -> not claimed
  - sys_sum on vs off W8: ratio 0.9651 (-3.49 %, -0.2018); bars r/i/s 36.56/11.45/7.86 %; claimed r/i/s n/n/n -> not claimed
- L9-JA-a-on c4@W1 (canary reference) ms: 16.6508 [16.4962-17.1805] K=6 r/i/s 4.11/2.30/0.89 %
- L9-JC c4@W1 ms: 17.5469 [17.4890-18.1249] K=6 r/i/s 3.62/1.94/0.80 %
- L9-JC c4@W1 injected canary ms: 0.8447 [0.8248-0.8590] K=6 r/i/s 4.05/2.97/0.94 %
- L9-JC c4@W1 canary span ms: 0.8450 [0.8251-0.8596] K=6 r/i/s 4.08/3.00/0.95 %
**Decision P2 canary:** span 0.8450123549999999 vs injected 0.8447 ms (ok); step rise +0.8961 ms (NOT SEEN under the two-spread rule; SE alone: seen); ratio 1.0538 (+5.38 %, +0.8961); bars r/i/s 10.96/6.01/2.40 %; claimed r/i/s n/n/Y -> not claimed

## P3 L11 G9 remainder: J-A at W 2/4/16, parent 0ca312bd vs tip cbd86a65
- G9-JA g9p@W2 ms: 11.5423 [11.5165-11.6217] K=6 r/i/s 0.91/0.49/0.19 %
- G9-JA g9t@W2 ms: 11.5580 [11.5423-11.6426] K=6 r/i/s 0.87/0.57/0.20 %
- G9-JA W2 tip vs parent: ratio 1.0014 (+0.14 %, +0.0157); bars r/i/s 2.52/1.50/0.56 %; claimed r/i/s n/n/n -> not claimed
- G9-JA g9p@W4 ms: 7.7408 [7.7238-7.8875] K=6 r/i/s 2.12/0.16/0.41 %
- G9-JA g9t@W4 ms: 7.7424 [7.7278-7.7728] K=6 r/i/s 0.58/0.33/0.12 %
- G9-JA W4 tip vs parent: ratio 1.0002 (+0.02 %, +0.0017); bars r/i/s 4.39/0.73/0.86 %; claimed r/i/s n/n/n -> not claimed
- G9-JA g9p@W16 ms: 5.6993 [5.6905-5.8470] K=6 r/i/s 2.75/0.28/0.54 %
- G9-JA g9t@W16 ms: 5.6984 [5.6670-5.9873] K=6 r/i/s 5.62/0.59/1.09 %
- G9-JA W16 tip vs parent: ratio 0.9998 (-0.02 %, -0.0009); bars r/i/s 12.51/1.30/2.44 %; claimed r/i/s n/n/n -> not claimed
**Decision P3:** not claimed slower at W 2/4/16

## P4 Our per-stage armed spans, trunk 93b2615b default row (--cfg default --broadphase tree), [100,500)
Per process: the MEDIAN over steps [100,500) of each span column (and the mean beside it); per cell: the median over K.

### W=1
- wall_ns                            median 5.9920 ms [5.9582-6.0625] (s 0.33 %); mean 6.0239 ms
- g_ns                               median 0.0334 ms [0.0324-0.0340] (s 0.89 %); mean 0.0345 ms
- phys_bp_assemble_ns                median 0.0284 ms [0.0282-0.0285] (s 0.22 %); mean 0.0289 ms
- phys_bp_build_ns                   median 0.0214 ms [0.0213-0.0217] (s 0.35 %); mean 0.0219 ms
- phys_bp_query_ns                   median 0.2043 ms [0.2034-0.2051] (s 0.16 %); mean 0.2084 ms
- phys_bp_verify_ns                  median 0.0064 ms [0.0064-0.0065] (s 0.26 %); mean 0.0064 ms
- phys_color_narrow_ns               median 0.0091 ms [0.0090-0.0094] (s 0.80 %); mean 0.0096 ms
- phys_color_wide_ns                 median 2.4291 ms [2.4117-2.5002] (s 0.68 %); mean 2.4340 ms
- phys_gravity_ns                    median 0.0073 ms [0.0072-0.0075] (s 0.84 %); mean 0.0075 ms
- phys_integrate_ns                  median 0.0656 ms [0.0652-0.0659] (s 0.20 %); mean 0.0660 ms
- phys_np_axis_commit_ns             median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_np_compact_ns                 median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_np_dispatch_ns                median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_pass_biased_ns                median 0.8197 ms [0.8140-0.8427] (s 0.66 %); mean 0.8217 ms
- phys_pass_relax_ns                 median 1.6214 ms [1.6099-1.6701] (s 0.69 %); mean 1.6246 ms
- phys_restitution_ns                median 0.0029 ms [0.0029-0.0029] (s 0.09 %); mean 0.0030 ms
- phys_sleep_begin_ns                median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_sleep_end_ns                  median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_sleep_freeze_ns               median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_solve_build_ns                median 0.2764 ms [0.2720-0.2822] (s 0.69 %); mean 0.2841 ms
- phys_store_ns                      median 0.0206 ms [0.0204-0.0213] (s 0.86 %); mean 0.0218 ms
- phys_warm_apply_ns                 median 0.2975 ms [0.2948-0.3004] (s 0.38 %); mean 0.2990 ms
- phys_write_back_ns                 median 0.0025 ms [0.0025-0.0025] (s 0.21 %); mean 0.0025 ms
- r_ns                               median 0.0026 ms [0.0026-0.0027] (s 1.16 %); mean 0.0027 ms
- sys_physics_apply_ns               median 0.0108 ms [0.0100-0.0116] (s 2.32 %); mean 0.0109 ms
- sys_physics_broadphase_ns          median 0.2606 ms [0.2598-0.2616] (s 0.14 %); mean 0.2656 ms
- sys_physics_build_graph_ns         median 0.1135 ms [0.1127-0.1151] (s 0.41 %); mean 0.1153 ms
- sys_physics_gather_ns              median 0.0274 ms [0.0269-0.0278] (s 0.64 %); mean 0.0279 ms
- sys_physics_integrate_ns           median 0.0000 ms [0.0000-0.0000] (s 8.81 %); mean 0.0000 ms
- sys_physics_narrowphase_ns         median 2.4158 ms [2.4132-2.4208] (s 0.06 %); mean 2.4274 ms
- sys_physics_solve_colored_ns       median 3.1224 ms [3.0966-3.1946] (s 0.59 %); mean 3.1376 ms
- sys_select_broadphase_ns           median 0.0001 ms [0.0001-0.0001] (s 8.81 %); mean 0.0001 ms
- sys_sum_ns                         median 5.9578 ms [5.9250-6.0299] (s 0.33 %); mean 5.9892 ms
- u_ns                               median 0.0011 ms [0.0011-0.0011] (s 0.99 %); mean 0.0011 ms
- manifolds per step (mean over [100,500)): 4519.26
- pairs per step (mean over [100,500)): 9559.22
- phys_np_pairs per step (mean over [100,500)): 9559.22
- phys_bp_pairs per step (mean over [100,500)): 9559.22
- phys_np_manifolds per step (mean over [100,500)): 4519.26

### W=8
- wall_ns                            median 2.1942 ms [2.1816-2.2583] (s 0.65 %); mean 2.2187 ms
- g_ns                               median 0.0307 ms [0.0304-0.0316] (s 0.72 %); mean 0.0311 ms
- phys_bp_assemble_ns                median 0.0295 ms [0.0294-0.0302] (s 0.49 %); mean 0.0309 ms
- phys_bp_build_ns                   median 0.0218 ms [0.0217-0.0229] (s 1.08 %); mean 0.0221 ms
- phys_bp_query_ns                   median 0.2057 ms [0.2051-0.2132] (s 0.77 %); mean 0.2088 ms
- phys_bp_verify_ns                  median 0.0065 ms [0.0065-0.0067] (s 0.60 %); mean 0.0064 ms
- phys_color_narrow_ns               median 0.0104 ms [0.0102-0.0116] (s 2.47 %); mean 0.0111 ms
- phys_color_wide_ns                 median 0.6097 ms [0.6036-0.6155] (s 0.38 %); mean 0.6193 ms
- phys_gravity_ns                    median 0.0124 ms [0.0123-0.0127] (s 0.63 %); mean 0.0125 ms
- phys_integrate_ns                  median 0.0801 ms [0.0798-0.0807] (s 0.27 %); mean 0.0808 ms
- phys_np_axis_commit_ns             median 0.0046 ms [0.0046-0.0047] (s 0.62 %); mean 0.0047 ms
- phys_np_compact_ns                 median 0.0188 ms [0.0185-0.0192] (s 0.64 %); mean 0.0190 ms
- phys_np_dispatch_ns                median 0.3436 ms [0.3429-0.3540] (s 0.64 %); mean 0.3475 ms
- phys_pass_biased_ns                median 0.2185 ms [0.2168-0.2210] (s 0.38 %); mean 0.2217 ms
- phys_pass_relax_ns                 median 0.4065 ms [0.4014-0.4102] (s 0.38 %); mean 0.4125 ms
- phys_restitution_ns                median 0.0030 ms [0.0030-0.0031] (s 0.67 %); mean 0.0033 ms
- phys_sleep_begin_ns                median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_sleep_end_ns                  median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_sleep_freeze_ns               median 0.0000 ms [0.0000-0.0000] (s 0.00 %); mean 0.0000 ms
- phys_solve_build_ns                median 0.3067 ms [0.3057-0.3247] (s 1.24 %); mean 0.3122 ms
- phys_store_ns                      median 0.0249 ms [0.0248-0.0272] (s 1.88 %); mean 0.0256 ms
- phys_warm_apply_ns                 median 0.3043 ms [0.3037-0.3081] (s 0.35 %); mean 0.3080 ms
- phys_write_back_ns                 median 0.0027 ms [0.0026-0.0028] (s 1.11 %); mean 0.0027 ms
- r_ns                               median 0.0037 ms [0.0034-0.0038] (s 1.85 %); mean 0.0038 ms
- sys_physics_apply_ns               median 0.0108 ms [0.0102-0.0111] (s 1.80 %); mean 0.0109 ms
- sys_physics_broadphase_ns          median 0.2635 ms [0.2626-0.2739] (s 0.84 %); mean 0.2684 ms
- sys_physics_build_graph_ns         median 0.1149 ms [0.1138-0.1165] (s 0.45 %); mean 0.1166 ms
- sys_physics_gather_ns              median 0.0281 ms [0.0280-0.0283] (s 0.24 %); mean 0.0288 ms
- sys_physics_integrate_ns           median 0.0000 ms [0.0000-0.0001] (s 17.61 %); mean 0.0001 ms
- sys_physics_narrowphase_ns         median 0.3783 ms [0.3780-0.3887] (s 0.57 %); mean 0.3823 ms
- sys_physics_solve_colored_ns       median 1.3668 ms [1.3549-1.3946] (s 0.51 %); mean 1.3829 ms
- sys_select_broadphase_ns           median 0.0001 ms [0.0001-0.0001] (s 5.28 %); mean 0.0001 ms
- sys_sum_ns                         median 2.1636 ms [2.1508-2.2256] (s 0.64 %); mean 2.1879 ms
- u_ns                               median 0.0011 ms [0.0011-0.0013] (s 3.18 %); mean 0.0012 ms
- manifolds per step (mean over [100,500)): 4519.26
- pairs per step (mean over [100,500)): 9559.22
- phys_np_pairs per step (mean over [100,500)): 9559.22
- phys_bp_pairs per step (mean over [100,500)): 9559.22
- phys_np_manifolds per step (mean over [100,500)): 4519.26

## P5 Jolt v5.6.0 per-stage profile (the profiled Distribution build, frames 100/200/300/400 per process)
Per process: the mean over its four dumped frames of each scope's wall (union of its intervals across threads) and cpu (sum over threads); per cell: the median over K. Profiler on: the frame time carries its overhead.

### W=1
- P-jolt56-prof@W1 frame ms at the dumped frames: 12.9229 [12.6112-13.2371] K=6 r/i/s 4.84/2.10/0.93 %
- P-jolt56-prof@W1 [0,500) mean ms (profiled build): 13.0157 [12.6448-13.1381] K=6 r/i/s 3.79/1.60/0.74 %
- update                                                     wall 12.9235 ms [12.6118-13.2375]; cpu 12.9235 ms
- bp prepare (UpdateBroadPhasePrepare)                       wall 0.0552 ms [0.0487-0.0633]; cpu 0.0552 ms
- bp finalize (UpdateBroadPhaseFinalize)                     wall 0.0002 ms [0.0001-0.0003]; cpu 0.0002 ms
- FindCollisions job (bp pairs + narrowphase + contact add)  wall 3.9982 ms [3.9019-4.2357]; cpu 3.9982 ms
-   bp FindCollidingPairs                                    wall 0.5609 ms [0.5559-0.5652]; cpu 0.5609 ms
-   np Add Constraint From Cached Manifold                   wall 1.1806 ms [1.1282-1.1918]; cpu 1.1806 ms
-   np sCollideConvexVsConvex                                wall 0.0011 ms [0.0010-0.0020]; cpu 0.0011 ms
- islands (BuildIslandsFromConstraints)                      wall 0.0005 ms [0.0005-0.0005]; cpu 0.0005 ms
- islands (FinalizeIslands)                                  wall 0.0113 ms [0.0106-0.0125]; cpu 0.0113 ms
- setup (SetupVelocityConstraints)                           wall 0.0005 ms [0.0005-0.0006]; cpu 0.0005 ms
- solve velocity (SolveVelocityConstraints job)              wall 7.7896 ms [7.7159-7.9078]; cpu 7.7896 ms
- solve position (SolvePositionConstraints job)              wall 0.9557 ms [0.8715-1.2990]; cpu 0.9557 ms
- integrate (IntegrateVelocity)                              wall 0.0113 ms [0.0110-0.0121]; cpu 0.0113 ms
- contact removed callbacks                                  wall 0.0036 ms [0.0026-0.0063]; cpu 0.0036 ms
- derived: narrowphase + contact add (cpu) = FindCollisions - FindCollidingPairs = 3.4374 ms

### W=8
- P-jolt56-prof@W8 frame ms at the dumped frames: 3.0355 [2.9046-3.1185] K=6 r/i/s 7.05/3.32/1.38 %
- P-jolt56-prof@W8 [0,500) mean ms (profiled build): 2.9471 [2.8726-3.0293] K=6 r/i/s 5.32/4.74/1.34 %
- update                                                     wall 3.0352 ms [2.9044-3.1180]; cpu 3.0352 ms
- bp prepare (UpdateBroadPhasePrepare)                       wall 0.0719 ms [0.0625-0.0765]; cpu 0.0719 ms
- bp finalize (UpdateBroadPhaseFinalize)                     wall 0.0001 ms [0.0001-0.0001]; cpu 0.0001 ms
- FindCollisions job (bp pairs + narrowphase + contact add)  wall 0.8207 ms [0.7406-0.8933]; cpu 6.4884 ms
-   bp FindCollidingPairs                                    wall 0.1027 ms [0.0986-0.1046]; cpu 0.7152 ms
-   np Add Constraint From Cached Manifold                   wall 0.7194 ms [0.6428-0.7909]; cpu 2.8131 ms
-   np sCollideConvexVsConvex                                wall 0.0018 ms [0.0016-0.0030]; cpu 0.0018 ms
- islands (BuildIslandsFromConstraints)                      wall 0.0004 ms [0.0003-0.0005]; cpu 0.0004 ms
- islands (FinalizeIslands)                                  wall 0.0158 ms [0.0152-0.0174]; cpu 0.0158 ms
- setup (SetupVelocityConstraints)                           wall 0.0002 ms [0.0002-0.0003]; cpu 0.0002 ms
- solve velocity (SolveVelocityConstraints job)              wall 1.8850 ms [1.8769-1.9698]; cpu 15.0697 ms
- solve position (SolvePositionConstraints job)              wall 0.2330 ms [0.2274-0.2461]; cpu 1.3490 ms
- integrate (IntegrateVelocity)                              wall 0.0057 ms [0.0051-0.0066]; cpu 0.0282 ms
- contact removed callbacks                                  wall 0.0044 ms [0.0036-0.0068]; cpu 0.0044 ms
- derived: narrowphase + contact add (cpu) = FindCollisions - FindCollidingPairs = 5.7732 ms
