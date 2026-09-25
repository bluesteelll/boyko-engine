# Window 7 wave 2 reduction (C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win7b\raw)
259 records, 237 chosen processes, 0 dropped slots, 0 voided passes, blocks skipped for priority: none
launch-context length misses among chosen processes: 0 []
window 7 records (Q1 blocks A, B): 542 records, 431 chosen, 19 dropped slots

## Q1 The W8 headline over four separated blocks: trunk 93b2615b default tree row vs Jolt v5.6.0 918fd2b7
Blocks: A = window 7 P1A-jolt, B = window 7 P1B-jolt (win7/raw/runs.jsonl), C = Q1C (first block of this run), D = Q1D (last). Statistic: the process mean over [0,500) (window 3). Rule: HOLDS iff claimed in A, B, C, D and pooled over A-D, same direction.

### W=4
- H-trk-tree@W4 [A] [0,500) ms: 3.0390 [2.9068-3.3550] K=5 r/i/s 14.75/11.81/3.81 %
- H-trk-tree@W4 [B] [0,500) ms: 2.7684 [2.7559-2.7824] K=6 r/i/s 0.95/0.61/0.21 %
- H-trk-tree@W4 [C] [0,500) ms: 2.7736 [2.7479-2.7927] K=6 r/i/s 1.61/0.49/0.29 %
- H-trk-tree@W4 [D] [0,500) ms: 2.7856 [2.7509-2.9320] K=6 r/i/s 6.50/1.52/1.21 %
- H-trk-tree@W4 [pooled A-D] [0,500) ms: 2.7782 [2.7479-3.3550] K=23 r/i/s 21.85/3.39/1.56 %
- H-trk-tree@W4 [pooled C+D] [0,500) ms: 2.7773 [2.7479-2.9320] K=12 r/i/s 6.63/0.86/0.63 %
- H-trk-ap@W4 [A] [0,500) ms: 4.8118 [4.7667-4.8252] K=3 r/i/s 1.22/0.61/0.46 %
- H-trk-ap@W4 [B] [0,500) ms: 4.5547 [4.5484-4.5691] K=6 r/i/s 0.46/0.17/0.08 %
- H-trk-ap@W4 [C] [0,500) ms: 4.5469 [4.5357-4.7342] K=6 r/i/s 4.37/0.34/0.86 %
- H-trk-ap@W4 [D] [0,500) ms: 4.5530 [4.5394-4.5605] K=6 r/i/s 0.46/0.16/0.08 %
- H-trk-ap@W4 [pooled A-D] [0,500) ms: 4.5545 [4.5357-4.8252] K=21 r/i/s 6.36/0.37/0.57 %
- H-trk-ap@W4 [pooled C+D] [0,500) ms: 4.5497 [4.5357-4.7342] K=12 r/i/s 4.36/0.26/0.43 %
- H-jolt56@W4 [A] [0,500) ms: 3.8928 [3.7579-4.0694] K=5 r/i/s 8.00/3.41/1.83 %
- H-jolt56@W4 [B] [0,500) ms: 3.6533 [3.5250-3.9810] K=6 r/i/s 12.48/3.18/2.31 %
- H-jolt56@W4 [C] [0,500) ms: 3.5372 [3.5342-3.5965] K=6 r/i/s 1.76/0.43/0.35 %
- H-jolt56@W4 [D] [0,500) ms: 3.6220 [3.5274-3.7492] K=6 r/i/s 6.12/2.57/1.18 %
- H-jolt56@W4 [pooled A-D] [0,500) ms: 3.6270 [3.5250-4.0694] K=23 r/i/s 15.01/5.96/1.13 %
- H-jolt56@W4 [pooled C+D] [0,500) ms: 3.5757 [3.5274-3.7492] K=12 r/i/s 6.20/2.35/0.78 %
- H-trk-tree / Jolt56 W4 [A] [0,500): ratio 0.7807 (-21.93 %, -0.8537); bars r/i/s 33.56/24.59/8.45 %; claimed r/i/s n/n/Y -> not claimed
- H-trk-tree / Jolt56 W4 [B] [0,500): ratio 0.7578 (-24.22 %, -0.8848); bars r/i/s 25.04/6.48/4.65 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W4 [C] [0,500): ratio 0.7841 (-21.59 %, -0.7636); bars r/i/s 4.77/1.31/0.91 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W4 [D] [0,500): ratio 0.7691 (-23.09 %, -0.8364); bars r/i/s 17.86/5.97/3.38 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W4 [pooled A-D] [0,500): ratio 0.7660 (-23.40 %, -0.8488); bars r/i/s 53.02/13.71/3.85 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W4 [pooled C+D] [0,500): ratio 0.7767 (-22.33 %, -0.7983); bars r/i/s 18.15/4.99/2.01 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [0..100) pooled A-D: ours 2.8166 [2.7777-3.3179] K=23 r/i/s 19.18/1.64/1.36 %; jolt 3.4873 [3.4071-3.9156] K=23 r/i/s 14.58/3.37/1.02 %; ratio 0.8077 (-19.23 %, -0.6708); bars r/i/s 48.19/7.49/3.40 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled A-D: ours 2.7712 [2.7369-3.3790] K=23 r/i/s 23.17/3.47/1.63 %; jolt 3.6587 [3.5501-4.1079] K=23 r/i/s 15.24/6.05/1.21 %; ratio 0.7574 (-24.26 %, -0.8875); bars r/i/s 55.47/13.95/4.05 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-tree / Jolt56 W4: A 0.7807, B 0.7578, C 0.7841, D 0.7691, pooled A-D 0.7660, pooled C+D 0.7767 -> does NOT hold: not claimed in every block and pooled, same direction (A n, B n, C Y, D Y, pooled A-D n); this run alone (C, D, pooled C+D): holds**
  - per manifold [100,500) pooled A-D: ours 0.6132 us (4519.26 manifolds/step), Jolt 0.4310 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 1.423x
- H-trk-ap / Jolt56 W4 [A] [0,500): ratio 1.2361 (+23.61 %, +0.9190); bars r/i/s 16.19/6.93/3.78 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W4 [B] [0,500): ratio 1.2468 (+24.68 %, +0.9015); bars r/i/s 24.98/6.37/4.63 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W4 [C] [0,500): ratio 1.2854 (+28.54 %, +1.0097); bars r/i/s 9.42/1.10/1.86 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W4 [D] [0,500): ratio 1.2571 (+25.71 %, +0.9310); bars r/i/s 12.28/5.14/2.37 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W4 [pooled A-D] [0,500): ratio 1.2557 (+25.57 %, +0.9275); bars r/i/s 32.60/11.94/2.53 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W4 [pooled C+D] [0,500): ratio 1.2724 (+27.24 %, +0.9740); bars r/i/s 15.17/4.72/1.79 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [0..100) pooled A-D: ours 4.5891 [4.5510-4.9234] K=21 r/i/s 8.12/0.55/0.63 %; jolt 3.4873 [3.4071-3.9156] K=23 r/i/s 14.58/3.37/1.02 %; ratio 1.3159 (+31.59 %, +1.1018); bars r/i/s 33.38/6.82/2.41 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled A-D: ours 4.5482 [4.5309-4.8219] K=21 r/i/s 6.40/0.36/0.57 %; jolt 3.6587 [3.5501-4.1079] K=23 r/i/s 15.24/6.05/1.21 %; ratio 1.2431 (+24.31 %, +0.8895); bars r/i/s 33.06/12.12/2.67 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-ap / Jolt56 W4: A 1.2361, B 1.2468, C 1.2854, D 1.2571, pooled A-D 1.2557, pooled C+D 1.2724 -> does NOT hold: not claimed in every block and pooled, same direction (A Y, B n, C Y, D Y, pooled A-D n); this run alone (C, D, pooled C+D): holds**
  - per manifold [100,500) pooled A-D: ours 1.0064 us (4519.26 manifolds/step), Jolt 0.4310 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 2.335x
- block terms H-jolt56 W4: B/A 0.9385 (n); C/A 0.9087 (n); D/C 1.0240 (n); D/B 0.9914 (n)
- block terms H-trk-tree W4: B/A 0.9110 (n); C/A 0.9127 (n); D/C 1.0043 (n); D/B 1.0062 (n)
- block terms H-trk-ap W4: B/A 0.9466 (C); C/A 0.9450 (n); D/C 1.0013 (n); D/B 0.9996 (n)
- window term: Jolt56 pooled A-D / window 3 = 1.0127 (context only)

### W=8
- H-trk-tree@W8 [A] [0,500) ms: 2.4529 [2.3290-2.9626] K=5 r/i/s 25.83/2.75/5.73 %
- H-trk-tree@W8 [B] [0,500) ms: 2.1878 [2.1850-2.2027] K=6 r/i/s 0.81/0.08/0.15 %
- H-trk-tree@W8 [C] [0,500) ms: 2.1966 [2.1827-2.1989] K=6 r/i/s 0.74/0.45/0.17 %
- H-trk-tree@W8 [D] [0,500) ms: 2.1986 [2.1792-3.1487] K=6 r/i/s 44.10/1.27/9.05 %
- H-trk-tree@W8 [pooled A-D] [0,500) ms: 2.1969 [2.1792-3.1487] K=23 r/i/s 44.13/3.98/3.02 %
- H-trk-tree@W8 [pooled C+D] [0,500) ms: 2.1966 [2.1792-3.1487] K=12 r/i/s 44.14/0.63/4.54 %
- H-trk-ap@W8 [A] [0,500) ms: 4.3273 [4.2040-4.4244] K=5 r/i/s 5.09/2.20/1.13 %
- H-trk-ap@W8 [B] [0,500) ms: 3.9764 [3.9711-3.9923] K=6 r/i/s 0.53/0.29/0.11 %
- H-trk-ap@W8 [C] [0,500) ms: 3.9706 [3.9495-3.9736] K=6 r/i/s 0.61/0.13/0.12 %
- H-trk-ap@W8 [D] [0,500) ms: 3.9733 [3.9634-4.0712] K=6 r/i/s 2.71/0.27/0.53 %
- H-trk-ap@W8 [pooled A-D] [0,500) ms: 3.9759 [3.9495-4.4244] K=23 r/i/s 11.94/1.52/0.95 %
- H-trk-ap@W8 [pooled C+D] [0,500) ms: 3.9719 [3.9495-4.0712] K=12 r/i/s 3.06/0.17/0.28 %
- H-jolt56@W8 [A] [0,500) ms: 2.9357 [2.8382-3.2679] K=6 r/i/s 14.64/3.56/2.72 %
- H-jolt56@W8 [B] [0,500) ms: 2.5085 [2.4711-2.5936] K=6 r/i/s 4.88/1.87/0.93 %
- H-jolt56@W8 [C] [0,500) ms: 2.5256 [2.4736-2.7015] K=6 r/i/s 9.02/1.78/1.67 %
- H-jolt56@W8 [D] [0,500) ms: 2.5282 [2.4602-2.6055] K=6 r/i/s 5.75/3.72/1.21 %
- H-jolt56@W8 [pooled A-D] [0,500) ms: 2.5328 [2.4602-3.2679] K=24 r/i/s 31.89/9.75/2.17 %
- H-jolt56@W8 [pooled C+D] [0,500) ms: 2.5256 [2.4602-2.7015] K=12 r/i/s 9.55/2.90/0.99 %
- H-trk-tree / Jolt56 W8 [A] [0,500): ratio 0.8355 (-16.45 %, -0.4829); bars r/i/s 59.38/8.99/12.69 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W8 [B] [0,500): ratio 0.8721 (-12.79 %, -0.3207); bars r/i/s 9.90/3.74/1.88 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-tree / Jolt56 W8 [C] [0,500): ratio 0.8697 (-13.03 %, -0.3290); bars r/i/s 18.11/3.66/3.36 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-tree / Jolt56 W8 [D] [0,500): ratio 0.8696 (-13.04 %, -0.3296); bars r/i/s 88.94/7.87/18.26 %; claimed r/i/s n/Y/n -> not claimed
- H-trk-tree / Jolt56 W8 [pooled A-D] [0,500): ratio 0.8674 (-13.26 %, -0.3360); bars r/i/s 108.89/21.06/7.44 %; claimed r/i/s n/n/Y -> not claimed
- H-trk-tree / Jolt56 W8 [pooled C+D] [0,500): ratio 0.8697 (-13.03 %, -0.3290); bars r/i/s 90.32/5.93/9.29 %; claimed r/i/s n/Y/Y -> not claimed
  - [0..100) pooled A-D: ours 2.2381 [2.1995-2.7320] K=23 r/i/s 23.79/3.45/1.71 %; jolt 2.3233 [2.2710-2.8552] K=24 r/i/s 25.14/5.08/1.74 %; ratio 0.9633 (-3.67 %, -0.0852); bars r/i/s 69.23/12.28/4.87 %; claimed r/i/s n/n/n -> not claimed
  - [100..500) pooled A-D: ours 2.1864 [2.1638-3.3585] K=23 r/i/s 54.64/3.70/3.56 %; jolt 2.5891 [2.5006-3.3711] K=24 r/i/s 33.62/11.28/2.27 %; ratio 0.8445 (-15.55 %, -0.4027); bars r/i/s 128.32/23.75/8.44 %; claimed r/i/s n/n/Y -> not claimed
  **H-trk-tree / Jolt56 W8: A 0.8355, B 0.8721, C 0.8697, D 0.8696, pooled A-D 0.8674, pooled C+D 0.8697 -> does NOT hold: not claimed in every block and pooled, same direction (A n, B Y, C n, D n, pooled A-D n); this run alone (C, D, pooled C+D): does not hold**
  - per manifold [100,500) pooled A-D: ours 0.4838 us (4519.26 manifolds/step), Jolt 0.3050 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 1.586x
- H-trk-ap / Jolt56 W8 [A] [0,500): ratio 1.4740 (+47.40 %, +1.3916); bars r/i/s 31.00/8.37/5.90 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [B] [0,500): ratio 1.5851 (+58.51 %, +1.4679); bars r/i/s 9.82/3.78/1.87 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [C] [0,500): ratio 1.5722 (+57.22 %, +1.4451); bars r/i/s 18.09/3.56/3.35 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [D] [0,500): ratio 1.5716 (+57.16 %, +1.4451); bars r/i/s 12.71/7.46/2.64 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W8 [pooled A-D] [0,500): ratio 1.5698 (+56.98 %, +1.4431); bars r/i/s 68.10/19.73/4.73 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W8 [pooled C+D] [0,500): ratio 1.5727 (+57.27 %, +1.4463); bars r/i/s 20.06/5.80/2.05 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [0..100) pooled A-D: ours 4.0024 [3.9739-4.6797] K=23 r/i/s 17.63/1.01/1.13 %; jolt 2.3233 [2.2710-2.8552] K=24 r/i/s 25.14/5.08/1.74 %; ratio 1.7227 (+72.27 %, +1.6792); bars r/i/s 61.42/10.36/4.14 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [100..500) pooled A-D: ours 3.9698 [3.9396-4.4495] K=23 r/i/s 12.84/2.05/0.95 %; jolt 2.5891 [2.5006-3.3711] K=24 r/i/s 33.62/11.28/2.27 %; ratio 1.5332 (+53.32 %, +1.3806); bars r/i/s 71.98/22.93/4.92 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-ap / Jolt56 W8: A 1.4740, B 1.5851, C 1.5722, D 1.5716, pooled A-D 1.5698, pooled C+D 1.5727 -> does NOT hold: not claimed in every block and pooled, same direction (A Y, B Y, C Y, D Y, pooled A-D n); this run alone (C, D, pooled C+D): holds**
  - per manifold [100,500) pooled A-D: ours 0.8784 us (4519.26 manifolds/step), Jolt 0.3050 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 2.880x
- block terms H-jolt56 W8: B/A 0.8545 (n); C/A 0.8603 (n); D/C 1.0010 (n); D/B 1.0078 (n)
- block terms H-trk-tree W8: B/A 0.8919 (n); C/A 0.8955 (n); D/C 1.0009 (n); D/B 1.0049 (n)
- block terms H-trk-ap W8: B/A 0.9189 (n); C/A 0.9176 (n); D/C 1.0007 (n); D/B 0.9992 (n)
- window term: Jolt56 pooled A-D / window 3 = 0.9858 (context only)

### W=16
- H-trk-tree@W16 [A] [0,500) ms: 2.7452 [2.6322-2.9531] K=4 r/i/s 11.69/6.82/3.33 %
- H-trk-tree@W16 [B] [0,500) ms: 2.3628 [2.3507-2.5798] K=6 r/i/s 9.69/6.22/2.34 %
- H-trk-tree@W16 [C] [0,500) ms: 2.3628 [2.3497-2.4315] K=6 r/i/s 3.46/0.66/0.66 %
- H-trk-tree@W16 [D] [0,500) ms: 2.3701 [2.3532-2.6038] K=6 r/i/s 10.57/1.84/2.08 %
- H-trk-tree@W16 [pooled A-D] [0,500) ms: 2.3696 [2.3497-2.9531] K=22 r/i/s 25.47/8.94/1.94 %
- H-trk-tree@W16 [pooled C+D] [0,500) ms: 2.3658 [2.3497-2.6038] K=12 r/i/s 10.74/1.04/1.09 %
- H-trk-ap@W16 [A] [0,500) ms: 4.6552 [4.5748-4.8962] K=5 r/i/s 6.91/1.44/1.47 %
- H-trk-ap@W16 [B] [0,500) ms: 4.1251 [4.1139-4.3143] K=6 r/i/s 4.86/0.45/0.97 %
- H-trk-ap@W16 [C] [0,500) ms: 4.1229 [4.1124-4.1606] K=6 r/i/s 1.17/0.43/0.23 %
- H-trk-ap@W16 [D] [0,500) ms: 4.1213 [4.1058-4.1292] K=6 r/i/s 0.57/0.11/0.10 %
- H-trk-ap@W16 [pooled A-D] [0,500) ms: 4.1292 [4.1058-4.8962] K=23 r/i/s 19.14/2.88/1.55 %
- H-trk-ap@W16 [pooled C+D] [0,500) ms: 4.1213 [4.1058-4.1606] K=12 r/i/s 1.33/0.33/0.12 %
- H-jolt56@W16 [A] [0,500) ms: 3.1752 [2.8904-5.3684] K=6 r/i/s 78.04/12.51/15.11 %
- H-jolt56@W16 [B] [0,500) ms: 2.4075 [2.3421-2.4624] K=6 r/i/s 5.00/2.57/0.99 %
- H-jolt56@W16 [C] [0,500) ms: 2.4090 [2.3479-2.4195] K=6 r/i/s 2.97/1.65/0.65 %
- H-jolt56@W16 [D] [0,500) ms: 2.4253 [2.3547-2.4507] K=6 r/i/s 3.96/2.16/0.85 %
- H-jolt56@W16 [pooled A-D] [0,500) ms: 2.4253 [2.3421-5.3684] K=24 r/i/s 124.78/7.90/6.90 %
- H-jolt56@W16 [pooled C+D] [0,500) ms: 2.4120 [2.3479-2.4507] K=12 r/i/s 4.26/2.67/0.52 %
- H-trk-tree / Jolt56 W16 [A] [0,500): ratio 0.8646 (-13.54 %, -0.4300); bars r/i/s 157.82/28.49/30.95 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [B] [0,500): ratio 0.9814 (-1.86 %, -0.0448); bars r/i/s 21.81/13.46/5.08 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [C] [0,500): ratio 0.9808 (-1.92 %, -0.0463); bars r/i/s 9.13/3.56/1.85 %; claimed r/i/s n/n/Y -> not claimed
- H-trk-tree / Jolt56 W16 [D] [0,500): ratio 0.9773 (-2.27 %, -0.0552); bars r/i/s 22.58/5.68/4.49 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [pooled A-D] [0,500): ratio 0.9771 (-2.29 %, -0.0557); bars r/i/s 254.71/23.86/14.33 %; claimed r/i/s n/n/n -> not claimed
- H-trk-tree / Jolt56 W16 [pooled C+D] [0,500): ratio 0.9808 (-1.92 %, -0.0462); bars r/i/s 23.11/5.73/2.42 %; claimed r/i/s n/n/n -> not claimed
  - [0..100) pooled A-D: ours 2.4377 [2.3987-3.1172] K=22 r/i/s 29.47/9.18/2.16 %; jolt 2.1953 [2.1453-3.5399] K=24 r/i/s 63.53/6.89/3.50 %; ratio 1.1104 (+11.04 %, +0.2424); bars r/i/s 140.06/22.96/8.23 %; claimed r/i/s n/n/Y -> not claimed
  - [100..500) pooled A-D: ours 2.3522 [2.3345-2.9121] K=22 r/i/s 24.56/10.20/1.94 %; jolt 2.4838 [2.3776-5.8255] K=24 r/i/s 138.81/8.11/7.66 %; ratio 0.9470 (-5.30 %, -0.1317); bars r/i/s 281.94/26.06/15.80 %; claimed r/i/s n/n/n -> not claimed
  **H-trk-tree / Jolt56 W16: A 0.8646, B 0.9814, C 0.9808, D 0.9773, pooled A-D 0.9771, pooled C+D 0.9808 -> does NOT hold: not claimed in every block and pooled, same direction (A n, B n, C n, D n, pooled A-D n); this run alone (C, D, pooled C+D): does not hold**
  - per manifold [100,500) pooled A-D: ours 0.5205 us (4519.26 manifolds/step), Jolt 0.2926 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 1.779x
- H-trk-ap / Jolt56 W16 [A] [0,500): ratio 1.4661 (+46.61 %, +1.4800); bars r/i/s 156.69/25.18/30.37 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W16 [B] [0,500): ratio 1.7134 (+71.34 %, +1.7176); bars r/i/s 13.94/5.22/2.77 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W16 [C] [0,500): ratio 1.7114 (+71.14 %, +1.7138); bars r/i/s 6.39/3.41/1.37 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W16 [D] [0,500): ratio 1.6993 (+69.93 %, +1.6960); bars r/i/s 8.00/4.33/1.71 %; claimed r/i/s Y/Y/Y -> CLAIMED
- H-trk-ap / Jolt56 W16 [pooled A-D] [0,500): ratio 1.7025 (+70.25 %, +1.7039); bars r/i/s 252.48/16.82/14.14 %; claimed r/i/s n/Y/Y -> not claimed
- H-trk-ap / Jolt56 W16 [pooled C+D] [0,500): ratio 1.7087 (+70.87 %, +1.7093); bars r/i/s 8.93/5.38/1.07 %; claimed r/i/s Y/Y/Y -> CLAIMED
  - [0..100) pooled A-D: ours 4.1736 [4.1369-4.9111] K=23 r/i/s 18.55/4.23/1.51 %; jolt 2.1953 [2.1453-3.5399] K=24 r/i/s 63.53/6.89/3.50 %; ratio 1.9011 (+90.11 %, +1.9783); bars r/i/s 132.36/16.17/7.63 %; claimed r/i/s n/Y/Y -> not claimed
  - [100..500) pooled A-D: ours 4.1138 [4.0919-4.8925] K=23 r/i/s 19.46/2.82/1.57 %; jolt 2.4838 [2.3776-5.8255] K=24 r/i/s 138.81/8.11/7.66 %; ratio 1.6562 (+65.62 %, +1.6300); bars r/i/s 280.34/17.17/15.64 %; claimed r/i/s n/Y/Y -> not claimed
  **H-trk-ap / Jolt56 W16: A 1.4661, B 1.7134, C 1.7114, D 1.6993, pooled A-D 1.7025, pooled C+D 1.7087 -> does NOT hold: not claimed in every block and pooled, same direction (A n, B Y, C Y, D Y, pooled A-D n); this run alone (C, D, pooled C+D): holds**
  - per manifold [100,500) pooled A-D: ours 0.9103 us (4519.26 manifolds/step), Jolt 0.2926 us (8489.0 manifolds/step, window 7 -receipt) -> ours/Jolt 3.111x
- block terms H-jolt56 W16: B/A 0.7582 (n); C/A 0.7587 (n); D/C 1.0068 (n); D/B 1.0074 (n)
- block terms H-trk-tree W16: B/A 0.8607 (n); C/A 0.8607 (n); D/C 1.0031 (n); D/B 1.0031 (n)
- block terms H-trk-ap W16: B/A 0.8861 (n); C/A 0.8856 (n); D/C 0.9996 (n); D/B 0.9991 (n)
- window term: Jolt56 pooled A-D / window 3 = 1.0154 (context only)

**Decision Q1 (W8, the default row against Jolt v5.6.0, blocks A-D and pooled):** does NOT hold: not claimed in every block and pooled, same direction (A n, B Y, C n, D n, pooled A-D n)

## Q2 C3b t_q block-shift test: parent 6dd1f916 (RowWalk) vs tip 983480a9 (LeafList), blocks C and D
t_q = per-process median over [100,500) of phys_bp_query_ns, median over K. Layouts: armed = window 6 P3 (cwd 163, 648 chars), pad06 = window 6 P5g (cwd 166, 654 chars); length misses excluded. Window 6 context (tip, W1): {'P3 (plain layout, 648 chars)': 0.2102, 'P5g (padded layout, 654 chars)': 0.2354}
- t_q C3b-TA-armed par@W1 [C] ms: 0.4940 [0.4934-0.5000] K=6 r/i/s 1.33/0.60/0.28 %
- t_q C3b-TA-armed par@W1 [D] ms: 0.4938 [0.4925-0.4950] K=6 r/i/s 0.51/0.19/0.09 %
- t_q C3b-TA-armed par@W1 [pooled] ms: 0.4938 [0.4925-0.5000] K=12 r/i/s 1.50/0.20/0.15 %
- tree span C3b-TA-armed par@W1 [pooled] ms (limit 0.36): 0.5613 [0.5590-0.5658] K=12 r/i/s 1.23/0.40/0.14 %
- step C3b-TA-armed par@W1 [pooled] [0,500) ms: 17.2760 [17.1922-17.3333] K=12 r/i/s 0.82/0.60/0.12 %
- t_q C3b-TA-armed tip@W1 [C] ms: 0.2459 [0.2431-0.2495] K=6 r/i/s 2.61/1.51/0.54 %
- t_q C3b-TA-armed tip@W1 [D] ms: 0.2486 [0.2440-0.2603] K=6 r/i/s 6.54/3.58/1.34 %
- t_q C3b-TA-armed tip@W1 [pooled] ms: 0.2471 [0.2431-0.2603] K=12 r/i/s 6.97/1.85/0.77 %
- tree span C3b-TA-armed tip@W1 [pooled] ms (limit 0.36): 0.3190 [0.3154-0.3313] K=12 r/i/s 4.98/1.49/0.57 %
- step C3b-TA-armed tip@W1 [pooled] [0,500) ms: 17.0220 [16.9454-17.1267] K=12 r/i/s 1.07/0.33/0.10 %
- t_q tip vs par C3b-TA-armed W1 [C]: ratio 0.4977 (-50.23 %, -0.2481); bars r/i/s 5.85/3.26/1.21 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-armed W1 [D]: ratio 0.5035 (-49.65 %, -0.2452); bars r/i/s 13.11/7.17/2.69 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-armed W1 [pooled]: ratio 0.5005 (-49.95 %, -0.2467); bars r/i/s 14.25/3.72/1.57 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q C3b-TA-armed par@W8 [C] ms: 0.4119 [0.4101-0.4141] K=6 r/i/s 0.98/0.48/0.19 %
- t_q C3b-TA-armed par@W8 [D] ms: 0.4111 [0.4104-0.4135] K=6 r/i/s 0.75/0.12/0.14 %
- t_q C3b-TA-armed par@W8 [pooled] ms: 0.4113 [0.4101-0.4141] K=12 r/i/s 0.98/0.36/0.12 %
- tree span C3b-TA-armed par@W8 [pooled] ms (limit 0.35): 0.4648 [0.4633-0.4675] K=12 r/i/s 0.91/0.21/0.10 %
- step C3b-TA-armed par@W8 [pooled] [0,500) ms: 4.2692 [4.2617-4.4095] K=12 r/i/s 3.46/0.13/0.35 %
- t_q C3b-TA-armed tip@W8 [C] ms: 0.2124 [0.2117-0.2127] K=6 r/i/s 0.47/0.21/0.09 %
- t_q C3b-TA-armed tip@W8 [D] ms: 0.2119 [0.2115-0.2133] K=6 r/i/s 0.82/0.36/0.17 %
- t_q C3b-TA-armed tip@W8 [pooled] ms: 0.2122 [0.2115-0.2133] K=12 r/i/s 0.82/0.42/0.09 %
- tree span C3b-TA-armed tip@W8 [pooled] ms (limit 0.35): 0.2699 [0.2688-0.2708] K=12 r/i/s 0.76/0.44/0.10 %
- step C3b-TA-armed tip@W8 [pooled] [0,500) ms: 4.0792 [4.0580-4.1220] K=12 r/i/s 1.57/0.38/0.16 %
- t_q tip vs par C3b-TA-armed W8 [C]: ratio 0.5156 (-48.44 %, -0.1995); bars r/i/s 2.17/1.05/0.42 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-armed W8 [D]: ratio 0.5153 (-48.47 %, -0.1993); bars r/i/s 2.22/0.76/0.43 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-armed W8 [pooled]: ratio 0.5159 (-48.41 %, -0.1991); bars r/i/s 2.55/1.10/0.30 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q C3b-TA-pad06 par@W1 [C] ms: 0.4936 [0.4927-0.5001] K=6 r/i/s 1.48/0.30/0.29 %
- t_q C3b-TA-pad06 par@W1 [D] ms: 0.4949 [0.4929-0.5040] K=6 r/i/s 2.24/0.74/0.43 %
- t_q C3b-TA-pad06 par@W1 [pooled] ms: 0.4938 [0.4927-0.5040] K=12 r/i/s 2.27/0.67/0.26 %
- tree span C3b-TA-pad06 par@W1 [pooled] ms (limit 0.36): 0.5608 [0.5577-0.5706] K=12 r/i/s 2.31/0.71/0.23 %
- step C3b-TA-pad06 par@W1 [pooled] [0,500) ms: 17.3059 [17.1637-17.3562] K=12 r/i/s 1.11/0.39/0.13 %
- t_q C3b-TA-pad06 tip@W1 [C] ms: 0.2509 [0.2442-0.2649] K=6 r/i/s 8.25/3.96/1.60 %
- t_q C3b-TA-pad06 tip@W1 [D] ms: 0.2464 [0.2412-0.2509] K=6 r/i/s 3.92/1.09/0.69 %
- t_q C3b-TA-pad06 tip@W1 [pooled] ms: 0.2480 [0.2412-0.2649] K=12 r/i/s 9.55/2.04/0.96 %
- tree span C3b-TA-pad06 tip@W1 [pooled] ms (limit 0.36): 0.3200 [0.3142-0.3362] K=12 r/i/s 6.90/1.67/0.69 %
- step C3b-TA-pad06 tip@W1 [pooled] [0,500) ms: 16.9872 [16.9552-17.1623] K=12 r/i/s 1.22/0.38/0.13 %
- t_q tip vs par C3b-TA-pad06 W1 [C]: ratio 0.5082 (-49.18 %, -0.2427); bars r/i/s 16.77/7.95/3.25 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-pad06 W1 [D]: ratio 0.4979 (-50.21 %, -0.2485); bars r/i/s 9.03/2.64/1.64 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TA-pad06 W1 [pooled]: ratio 0.5023 (-49.77 %, -0.2458); bars r/i/s 19.63/4.30/1.98 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q C3b-TD-armed par@W1 [C] ms: 0.4031 [0.4025-0.4048] K=6 r/i/s 0.57/0.36/0.13 %
- t_q C3b-TD-armed par@W1 [D] ms: 0.4029 [0.4025-0.4044] K=6 r/i/s 0.48/0.12/0.09 %
- t_q C3b-TD-armed par@W1 [pooled] ms: 0.4030 [0.4025-0.4048] K=12 r/i/s 0.58/0.18/0.08 %
- tree span C3b-TD-armed par@W1 [pooled] ms (limit 0.36): 0.4543 [0.4537-0.4587] K=12 r/i/s 1.10/0.43/0.15 %
- step C3b-TD-armed par@W1 [pooled] [0,500) ms: 6.8334 [6.7407-7.0402] K=12 r/i/s 4.38/0.74/0.43 %
- t_q C3b-TD-armed tip@W1 [C] ms: 0.2079 [0.2070-0.2103] K=6 r/i/s 1.59/0.38/0.28 %
- t_q C3b-TD-armed tip@W1 [D] ms: 0.2078 [0.2074-0.2155] K=6 r/i/s 3.89/2.58/0.94 %
- t_q C3b-TD-armed tip@W1 [pooled] ms: 0.2078 [0.2070-0.2155] K=12 r/i/s 4.06/0.69/0.50 %
- tree span C3b-TD-armed tip@W1 [pooled] ms (limit 0.36): 0.2635 [0.2621-0.2712] K=12 r/i/s 3.44/0.63/0.44 %
- step C3b-TD-armed tip@W1 [pooled] [0,500) ms: 6.6266 [6.5893-6.8460] K=12 r/i/s 3.87/1.30/0.51 %
- t_q tip vs par C3b-TD-armed W1 [C]: ratio 0.5158 (-48.42 %, -0.1952); bars r/i/s 3.37/1.05/0.62 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TD-armed W1 [D]: ratio 0.5159 (-48.41 %, -0.1951); bars r/i/s 7.83/5.16/1.90 %; claimed r/i/s Y/Y/Y -> CLAIMED
- t_q tip vs par C3b-TD-armed W1 [pooled]: ratio 0.5157 (-48.43 %, -0.1952); bars r/i/s 8.21/1.42/1.01 %; claimed r/i/s Y/Y/Y -> CLAIMED
- BLOCK TERM t_q D vs C C3b-TA-armed par@W1: ratio 0.9996 (-0.04 %, -0.0002); bars r/i/s 2.84/1.25/0.58 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TA-armed par@W8: ratio 0.9981 (-0.19 %, -0.0008); bars r/i/s 2.46/0.99/0.47 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TA-pad06 par@W1: ratio 1.0027 (+0.27 %, +0.0013); bars r/i/s 5.37/1.61/1.04 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TD-armed par@W1: ratio 0.9995 (-0.05 %, -0.0002); bars r/i/s 1.49/0.76/0.31 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed par@W1 [C]: ratio 0.9992 (-0.08 %, -0.0004); bars r/i/s 3.98/1.34/0.80 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed par@W1 [D]: ratio 1.0023 (+0.23 %, +0.0012); bars r/i/s 4.59/1.53/0.89 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed par@W1 [pooled]: ratio 1.0000 (+0.00 %, +0.0000); bars r/i/s 5.45/1.41/0.59 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TA-armed tip@W1: ratio 1.0112 (+1.12 %, +0.0027); bars r/i/s 14.08/7.78/2.89 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TA-armed tip@W8: ratio 0.9976 (-0.24 %, -0.0005); bars r/i/s 1.89/0.83/0.38 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TA-pad06 tip@W1: ratio 0.9822 (-1.78 %, -0.0045); bars r/i/s 18.27/8.22/3.49 %; claimed r/i/s n/n/n -> not claimed
- BLOCK TERM t_q D vs C C3b-TD-armed tip@W1: ratio 0.9996 (-0.04 %, -0.0001); bars r/i/s 8.40/5.21/1.97 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed tip@W1 [C]: ratio 1.0204 (+2.04 %, +0.0050); bars r/i/s 17.31/8.48/3.38 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed tip@W1 [D]: ratio 0.9912 (-0.88 %, -0.0022); bars r/i/s 15.24/7.49/3.02 %; claimed r/i/s n/n/n -> not claimed
- LAYOUT TERM t_q pad06 vs armed tip@W1 [pooled]: ratio 1.0036 (+0.36 %, +0.0009); bars r/i/s 23.64/5.51/2.46 %; claimed r/i/s n/n/n -> not claimed
**Q2 attribution:** layout term (tip W1) claimed in no block; block term D vs C (tip W1, plain) not claimed
- C2 bar, C3b-TD-armed tip W1 [C]: 0.2079 vs 0.235 (-11.53 %; bars 2r/2s 3.18/0.56 %; claimed r/s Y/Y) -> below CLAIMED
- C2 bar, C3b-TD-armed tip W1 [D]: 0.2078 vs 0.235 (-11.56 %; bars 2r/2s 7.77/1.89 %; claimed r/s Y/Y) -> below CLAIMED
- C2 bar, C3b-TD-armed tip W1 [pooled]: 0.2078 vs 0.235 (-11.56 %; bars 2r/2s 8.12/1.00 %; claimed r/s Y/Y) -> below CLAIMED
**Decision Q2 C2 on the letter's row: the DEFAULT row armed (c3b/design.md:231, :372): C2 NOT BUILT (< 0.235 claimed below in C, D and pooled)**; > 0.21 (attribution A): 0.2078 vs 0.21 (-1.03 %; bars 2r/2s 8.12/1.00 %; claimed r/s n/Y) -> on the bar (not claimed); <= 0.186 (into the band): 0.2078 vs 0.186 (+11.74 %; bars 2r/2s 8.12/1.00 %; claimed r/s Y/Y) -> above CLAIMED; c_q = 167.6 ns/row (> 150: F3)
- C2 bar, C3b-TA-armed tip W1 [C]: 0.2459 vs 0.235 (+4.62 %; bars 2r/2s 5.21/1.08 %; claimed r/s n/Y) -> on the bar (not claimed)
- C2 bar, C3b-TA-armed tip W1 [D]: 0.2486 vs 0.235 (+5.79 %; bars 2r/2s 13.07/2.68 %; claimed r/s n/Y) -> on the bar (not claimed)
- C2 bar, C3b-TA-armed tip W1 [pooled]: 0.2471 vs 0.235 (+5.16 %; bars 2r/2s 13.93/1.54 %; claimed r/s n/Y) -> on the bar (not claimed)
**Decision Q2 C2 on window 6's reading: the cfg-A armed row, plain layout (window 6 P3): UNRESOLVED (on the bar in at least one of C, D, pooled) - the orchestrator decides**; > 0.21 (attribution A): 0.2471 vs 0.21 (+17.68 %; bars 2r/2s 13.93/1.54 %; claimed r/s Y/Y) -> above CLAIMED; <= 0.186 (into the band): 0.2471 vs 0.186 (+32.87 %; bars 2r/2s 13.93/1.54 %; claimed r/s Y/Y) -> above CLAIMED; c_q = 199.3 ns/row (> 150: F3)
- C2 bar, C3b-TA-pad06 tip W1 [C]: 0.2509 vs 0.235 (+6.75 %; bars 2r/2s 16.51/3.20 %; claimed r/s n/Y) -> on the bar (not claimed)
- C2 bar, C3b-TA-pad06 tip W1 [D]: 0.2464 vs 0.235 (+4.85 %; bars 2r/2s 7.83/1.39 %; claimed r/s n/Y) -> on the bar (not claimed)
- C2 bar, C3b-TA-pad06 tip W1 [pooled]: 0.2480 vs 0.235 (+5.54 %; bars 2r/2s 19.10/1.91 %; claimed r/s n/Y) -> on the bar (not claimed)
**Decision Q2 C2 on the cfg-A armed row, padded layout (window 6 P5g), for robustness: UNRESOLVED (on the bar in at least one of C, D, pooled) - the orchestrator decides**; > 0.21 (attribution A): 0.2480 vs 0.21 (+18.11 %; bars 2r/2s 19.10/1.91 %; claimed r/s n/Y) -> on the bar (not claimed); <= 0.186 (into the band): 0.2480 vs 0.186 (+33.35 %; bars 2r/2s 19.10/1.91 %; claimed r/s Y/Y) -> above CLAIMED; c_q = 200.0 ns/row (> 150: F3)

## Q3 The tree thresholds between 64 and 256 (recipe 1.4), instrument 93b2615b + G4_SIZES, K = 3

### uniform
- n=64: all_pairs 0.002333 [0.002322-0.002333] K=3 r/i/s 0.49/0.24/0.20 % ms; tree 0.006462 [0.006248-0.006487] K=3 r/i/s 3.70/1.85/1.47 % ms; all_pairs/tree ratio 0.3610 (-63.90 %, -0.0041); bars r/i/s 7.47/3.74/2.97 %; claimed r/i/s Y/Y/Y -> CLAIMED  [window 4, before F1: 0.619]
- n=96: all_pairs 0.005170 [0.005157-0.005211] K=3 r/i/s 1.06/0.53/0.40 % ms; tree 0.008687 [0.008673-0.008743] K=3 r/i/s 0.81/0.40/0.31 % ms; all_pairs/tree ratio 0.5951 (-40.49 %, -0.0035); bars r/i/s 2.67/1.33/1.01 %; claimed r/i/s Y/Y/Y -> CLAIMED
- n=112: all_pairs 0.006871 [0.006863-0.006890] K=3 r/i/s 0.39/0.20/0.15 % ms; tree 0.009729 [0.009689-0.009738] K=3 r/i/s 0.51/0.25/0.20 % ms; all_pairs/tree ratio 0.7062 (-29.38 %, -0.0029); bars r/i/s 1.29/0.64/0.49 %; claimed r/i/s Y/Y/Y -> CLAIMED
- n=128: all_pairs 0.010033 [0.010032-0.010055] K=3 r/i/s 0.23/0.11/0.09 % ms; tree 0.011014 [0.010902-0.011169] K=3 r/i/s 2.42/1.21/0.88 % ms; all_pairs/tree ratio 0.9109 (-8.91 %, -0.0010); bars r/i/s 4.87/2.44/1.77 %; claimed r/i/s Y/Y/Y -> CLAIMED  [window 4, before F1: 1.137]
- n=136: all_pairs 0.011206 [0.011144-0.011230] K=3 r/i/s 0.77/0.38/0.29 % ms; tree 0.011614 [0.011502-0.011859] K=3 r/i/s 3.07/1.54/1.14 % ms; all_pairs/tree ratio 0.9649 (-3.51 %, -0.0004); bars r/i/s 6.34/3.17/2.35 %; claimed r/i/s n/Y/Y -> not claimed
- n=144: all_pairs 0.012421 [0.012375-0.012450] K=3 r/i/s 0.60/0.30/0.22 % ms; tree 0.012327 [0.012301-0.012452] K=3 r/i/s 1.22/0.61/0.47 % ms; all_pairs/tree ratio 1.0076 (+0.76 %, +0.0001); bars r/i/s 2.73/1.37/1.05 %; claimed r/i/s n/n/n -> not claimed
- n=152: all_pairs 0.013581 [0.013557-0.013718] K=3 r/i/s 1.19/0.59/0.46 % ms; tree 0.012956 [0.012889-0.012972] K=3 r/i/s 0.64/0.32/0.25 % ms; all_pairs/tree ratio 1.0482 (+4.82 %, +0.0006); bars r/i/s 2.70/1.35/1.05 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=160: all_pairs 0.015082 [0.015074-0.015140] K=3 r/i/s 0.44/0.22/0.17 % ms; tree 0.013459 [0.013456-0.013487] K=3 r/i/s 0.23/0.12/0.09 % ms; all_pairs/tree ratio 1.1206 (+12.06 %, +0.0016); bars r/i/s 0.99/0.49/0.39 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=168: all_pairs 0.016446 [0.016384-0.016497] K=3 r/i/s 0.69/0.34/0.25 % ms; tree 0.013968 [0.013960-0.014033] K=3 r/i/s 0.52/0.26/0.21 % ms; all_pairs/tree ratio 1.1774 (+17.74 %, +0.0025); bars r/i/s 1.73/0.86/0.65 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=176: all_pairs 0.017876 [0.017875-0.017900] K=3 r/i/s 0.14/0.07/0.06 % ms; tree 0.014617 [0.014559-0.014648] K=3 r/i/s 0.61/0.30/0.22 % ms; all_pairs/tree ratio 1.2230 (+22.30 %, +0.0033); bars r/i/s 1.25/0.62/0.46 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=192: all_pairs 0.021179 [0.021155-0.021209] K=3 r/i/s 0.25/0.13/0.09 % ms; tree 0.016095 [0.015907-0.016102] K=3 r/i/s 1.21/0.61/0.50 % ms; all_pairs/tree ratio 1.3159 (+31.59 %, +0.0051); bars r/i/s 2.48/1.24/1.01 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=256: all_pairs 0.037992 [0.037828-0.038165] K=3 r/i/s 0.89/0.44/0.32 % ms; tree 0.021027 [0.020997-0.021111] K=3 r/i/s 0.54/0.27/0.20 % ms; all_pairs/tree ratio 1.8068 (+80.68 %, +0.0170); bars r/i/s 2.08/1.04/0.76 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)  [window 4, before F1: 1.997]
**uniform: largest n with all_pairs not claimed slower = 144; smallest n with tree claimed faster = 152; monotone True; log-log crossover 142.6**
  - bridge n=64: tree_rowwalk 0.004612 [0.004604-0.004634] K=3 r/i/s 0.65/0.33/0.25 % ms; LeafList/RowWalk ratio 1.4011 (+40.11 %, +0.0018); bars r/i/s 7.52/3.76/2.99 %; claimed r/i/s Y/Y/Y -> CLAIMED; all_pairs/tree_rowwalk 0.506 (window 4, the same kernel: 0.619)
  - bridge n=128: tree_rowwalk 0.010613 [0.010598-0.010677] K=3 r/i/s 0.74/0.37/0.29 % ms; LeafList/RowWalk ratio 1.0378 (+3.78 %, +0.0004); bars r/i/s 5.07/2.54/1.85 %; claimed r/i/s n/Y/Y -> not claimed; all_pairs/tree_rowwalk 0.945 (window 4, the same kernel: 1.137)
  - bridge n=256: tree_rowwalk 0.022374 [0.022359-0.022398] K=3 r/i/s 0.17/0.09/0.06 % ms; LeafList/RowWalk ratio 0.9398 (-6.02 %, -0.0013); bars r/i/s 1.14/0.57/0.43 %; claimed r/i/s Y/Y/Y -> CLAIMED; all_pairs/tree_rowwalk 1.698 (window 4, the same kernel: 1.997)

### disparity
- n=64: all_pairs 0.003068 [0.003048-0.003074] K=3 r/i/s 0.83/0.42/0.32 % ms; tree 0.007568 [0.007551-0.007612] K=3 r/i/s 0.81/0.40/0.30 % ms; all_pairs/tree ratio 0.4054 (-59.46 %, -0.0045); bars r/i/s 2.33/1.16/0.88 %; claimed r/i/s Y/Y/Y -> CLAIMED  [window 4, before F1: 0.5]
- n=96: all_pairs 0.006248 [0.006247-0.006257] K=3 r/i/s 0.17/0.09/0.07 % ms; tree 0.011267 [0.010781-0.011381] K=3 r/i/s 5.33/2.66/2.05 % ms; all_pairs/tree ratio 0.5545 (-44.55 %, -0.0050); bars r/i/s 10.66/5.33/4.10 %; claimed r/i/s Y/Y/Y -> CLAIMED
- n=112: all_pairs 0.008116 [0.008110-0.008117] K=3 r/i/s 0.08/0.04/0.03 % ms; tree 0.012276 [0.012214-0.012301] K=3 r/i/s 0.71/0.35/0.26 % ms; all_pairs/tree ratio 0.6611 (-33.89 %, -0.0042); bars r/i/s 1.43/0.71/0.53 %; claimed r/i/s Y/Y/Y -> CLAIMED
- n=128: all_pairs 0.012159 [0.012153-0.012191] K=3 r/i/s 0.31/0.16/0.12 % ms; tree 0.013187 [0.013070-0.013300] K=3 r/i/s 1.74/0.87/0.63 % ms; all_pairs/tree ratio 0.9220 (-7.80 %, -0.0010); bars r/i/s 3.54/1.77/1.29 %; claimed r/i/s Y/Y/Y -> CLAIMED  [window 4, before F1: 0.946]
- n=136: all_pairs 0.013310 [0.013299-0.013315] K=3 r/i/s 0.12/0.06/0.04 % ms; tree 0.014189 [0.014184-0.014313] K=3 r/i/s 0.91/0.45/0.37 % ms; all_pairs/tree ratio 0.9381 (-6.19 %, -0.0009); bars r/i/s 1.83/0.92/0.75 %; claimed r/i/s Y/Y/Y -> CLAIMED
- n=144: all_pairs 0.014757 [0.014746-0.014774] K=3 r/i/s 0.19/0.09/0.07 % ms; tree 0.014863 [0.014858-0.015032] K=3 r/i/s 1.17/0.59/0.48 % ms; all_pairs/tree ratio 0.9929 (-0.71 %, -0.0001); bars r/i/s 2.37/1.19/0.97 %; claimed r/i/s n/n/n -> not claimed
- n=152: all_pairs 0.016325 [0.016224-0.016335] K=3 r/i/s 0.68/0.34/0.27 % ms; tree 0.015787 [0.015784-0.015853] K=3 r/i/s 0.44/0.22/0.18 % ms; all_pairs/tree ratio 1.0341 (+3.41 %, +0.0005); bars r/i/s 1.62/0.81/0.65 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=160: all_pairs 0.017906 [0.017901-0.017937] K=3 r/i/s 0.20/0.10/0.08 % ms; tree 0.016449 [0.016439-0.016460] K=3 r/i/s 0.13/0.06/0.05 % ms; all_pairs/tree ratio 1.0886 (+8.86 %, +0.0015); bars r/i/s 0.48/0.24/0.18 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=168: all_pairs 0.019383 [0.019357-0.019405] K=3 r/i/s 0.25/0.12/0.09 % ms; tree 0.016949 [0.016855-0.017059] K=3 r/i/s 1.20/0.60/0.44 % ms; all_pairs/tree ratio 1.1436 (+14.36 %, +0.0024); bars r/i/s 2.46/1.23/0.89 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=176: all_pairs 0.020930 [0.020804-0.020943] K=3 r/i/s 0.66/0.33/0.27 % ms; tree 0.017337 [0.017286-0.017425] K=3 r/i/s 0.80/0.40/0.29 % ms; all_pairs/tree ratio 1.2072 (+20.72 %, +0.0036); bars r/i/s 2.08/1.04/0.79 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=192: all_pairs 0.024679 [0.024619-0.024742] K=3 r/i/s 0.50/0.25/0.18 % ms; tree 0.018812 [0.018715-0.018904] K=3 r/i/s 1.00/0.50/0.36 % ms; all_pairs/tree ratio 1.3119 (+31.19 %, +0.0059); bars r/i/s 2.24/1.12/0.81 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)
- n=256: all_pairs 0.042475 [0.042391-0.042612] K=3 r/i/s 0.52/0.26/0.19 % ms; tree 0.025059 [0.025036-0.025212] K=3 r/i/s 0.70/0.35/0.28 % ms; all_pairs/tree ratio 1.6950 (+69.50 %, +0.0174); bars r/i/s 1.75/0.87/0.67 %; claimed r/i/s Y/Y/Y -> CLAIMED  <- all_pairs SLOWER (tree claimed faster)  [window 4, before F1: 1.543]
**disparity: largest n with all_pairs not claimed slower = 144; smallest n with tree claimed faster = 152; monotone True; log-log crossover 145.4**
  - bridge n=64: tree_rowwalk 0.007160 [0.007089-0.007170] K=3 r/i/s 1.13/0.56/0.44 % ms; LeafList/RowWalk ratio 1.0570 (+5.70 %, +0.0004); bars r/i/s 2.77/1.39/1.07 %; claimed r/i/s Y/Y/Y -> CLAIMED; all_pairs/tree_rowwalk 0.429 (window 4, the same kernel: 0.5)
  - bridge n=128: tree_rowwalk 0.014822 [0.014749-0.014969] K=3 r/i/s 1.48/0.74/0.55 % ms; LeafList/RowWalk ratio 0.8897 (-11.03 %, -0.0016); bars r/i/s 4.58/2.29/1.67 %; claimed r/i/s Y/Y/Y -> CLAIMED; all_pairs/tree_rowwalk 0.820 (window 4, the same kernel: 0.946)
  - bridge n=256: tree_rowwalk 0.031039 [0.031026-0.031183] K=3 r/i/s 0.51/0.25/0.20 % ms; LeafList/RowWalk ratio 0.8073 (-19.27 %, -0.0060); bars r/i/s 1.73/0.87/0.69 %; claimed r/i/s Y/Y/Y -> CLAIMED; all_pairs/tree_rowwalk 1.368 (window 4, the same kernel: 1.543)
**Decision Q3 (recipe 1.4 grid rule; L2 procedure beside it):** {'TREE_BRUTE_MAX_ROWS': 144, 'AUTO_TREE_LO': 144, 'AUTO_TREE_HI': 152, 'L2_HI': 150, 'L2_LO': 135, 'families': {'uniform': {'LO': 144, 'HI': 152, 'monotone': True, 'loglog_crossover': 142.56512134181537}, 'disparity': {'LO': 144, 'HI': 152, 'monotone': True, 'loglog_crossover': 145.3767718946689}}, 'grid_edge': 'inside the grid'}

## Q4 G-TW's canary resolution on the C4 binary 989ca0f0 (J-A W1, reuse on, unarmed)
R = 7.542 % = window 7's G-TW J-A W1 [0,500) two-spread bar (2 hypot(3.352, 1.728) %); rise/injected assumed 1.0609 (window 7's armed canary). Reference = G-TW's own 'on' row; window 7 read it 16.6838 ms.
- L9-JA-on c4@W1 (reference) [0,500) ms: 16.6480 [16.6066-16.8070] K=6 r/i/s 1.20/0.22/0.22 %
- L9-JC-r050 c4@W1 (frac 0.036 = 0.5 R) [0,500) ms: 17.2533 [17.2378-17.3411] K=6 r/i/s 0.60/0.18/0.11 %
- L9-JC-r050 injected canary ms: 0.5987 [0.5978-0.6051] K=6 r/i/s 1.21/0.17/0.23 %
- rung 0.5 R: rise +0.6053 ms = +3.64 % (injected 0.5987 ms, rise/injected 1.011); ratio 1.0364 (+3.64 %, +0.6053); bars r/i/s 2.69/0.57/0.50 %; claimed r/i/s Y/Y/Y -> CLAIMED -> SEEN (SE alone: seen)
- L9-JC-r100 c4@W1 (frac 0.071 = 1.0 R) [0,500) ms: 17.8861 [17.8221-17.9171] K=6 r/i/s 0.53/0.39/0.12 %
- L9-JC-r100 injected canary ms: 1.1807 [1.1791-1.1933] K=6 r/i/s 1.21/0.17/0.23 %
- rung 1.0 R: rise +1.2381 ms = +7.44 % (injected 1.1807 ms, rise/injected 1.049); ratio 1.0744 (+7.44 %, +1.2381); bars r/i/s 2.63/0.90/0.51 %; claimed r/i/s Y/Y/Y -> CLAIMED -> SEEN (SE alone: seen)
- L9-JC-r150 c4@W1 (frac 0.107 = 1.5 R) [0,500) ms: 18.5120 [18.4440-18.5432] K=6 r/i/s 0.54/0.27/0.11 %
- L9-JC-r150 injected canary ms: 1.7794 [1.7769-1.7983] K=6 r/i/s 1.21/0.17/0.23 %
- rung 1.5 R: rise +1.8640 ms = +11.20 % (injected 1.7794 ms, rise/injected 1.048); ratio 1.1120 (+11.20 %, +1.8640); bars r/i/s 2.64/0.70/0.50 %; claimed r/i/s Y/Y/Y -> CLAIMED -> SEEN (SE alone: seen)
- L9-JC-r200 c4@W1 (frac 0.142 = 2.0 R) [0,500) ms: 19.1239 [19.0715-19.1964] K=6 r/i/s 0.65/0.49/0.15 %
- L9-JC-r200 injected canary ms: 2.3614 [2.3581-2.3866] K=6 r/i/s 1.21/0.17/0.23 %
- rung 2.0 R: rise +2.4759 ms = +14.87 % (injected 2.3614 ms, rise/injected 1.048); ratio 1.1487 (+14.87 %, +2.4759); bars r/i/s 2.74/1.07/0.53 %; claimed r/i/s Y/Y/Y -> CLAIMED -> SEEN (SE alone: seen)
**Decision Q4:** DEMONSTRATED: the 1.5 R and 2 R rungs are SEEN; smallest rung seen with every larger rung seen: 0.5 R = 3.77 % of the step
