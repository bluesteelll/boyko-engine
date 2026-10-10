attempts 73, used 72, excluded [], reruns [(5, 7, 'boyko', 2, 'P-full', 'contaminated')]
receipts {'n': 146, 'median': 0.93, 'max': 39.11, 'min': 0.24, 'over5': 1}
warm-ups (pass, mean ms, boyko W=8 P-none): [(0, 9.0477), (1, 9.2243), (2, 9.1147), (3, 9.1388), (4, 9.0601), (5, 9.1723)]

| W | engine | policy | n | median ms | min | max | range % | IQR % | SD % | values (sorted) |
|---|---|---|---|---|---|---|---|---|---|---|
| 2 | boyko | P-none | 6 | 13.7664 | 13.6909 | 13.9558 | 1.92 | 0.63 | 0.69 | [13.6909, 13.7218, 13.7391, 13.7937, 13.8198, 13.9558] |
| 2 | boyko | P-phys | 6 | 13.7686 | 13.7325 | 13.9040 | 1.25 | 0.92 | 0.58 | [13.7325, 13.7342, 13.7663, 13.7709, 13.9023, 13.904] |
| 2 | boyko | P-full | 6 | 13.6751 | 13.4072 | 13.9175 | 3.73 | 0.35 | 1.19 | [13.4072, 13.6547, 13.6617, 13.6885, 13.7098, 13.9175] |
| 2 | jolt | P-none | 6 | 8.6571 | 8.5852 | 8.9474 | 4.18 | 2.37 | 1.73 | [8.5852, 8.6031, 8.6308, 8.6833, 8.8586, 8.9474] |
| 2 | jolt | P-phys | 6 | 8.7308 | 8.6245 | 9.0999 | 5.45 | 2.55 | 2.11 | [8.6245, 8.6559, 8.6886, 8.773, 8.9249, 9.0999] |
| 2 | jolt | P-full | 6 | 8.5792 | 8.5347 | 8.8913 | 4.16 | 1.30 | 1.61 | [8.5347, 8.5349, 8.5434, 8.615, 8.6592, 8.8913] |
| 8 | boyko | P-none | 6 | 9.1318 | 9.0850 | 9.2338 | 1.63 | 0.51 | 0.59 | [9.085, 9.092, 9.1256, 9.138, 9.1505, 9.2338] |
| 8 | boyko | P-phys | 6 | 9.1723 | 9.1224 | 9.2940 | 1.87 | 0.52 | 0.66 | [9.1224, 9.1466, 9.1583, 9.1862, 9.2007, 9.294] |
| 8 | boyko | P-full | 6 | 9.1442 | 9.0684 | 9.2050 | 1.49 | 0.20 | 0.48 | [9.0684, 9.1298, 9.1386, 9.1498, 9.1504, 9.205] |
| 8 | jolt | P-none | 6 | 3.5299 | 3.4470 | 3.5799 | 3.76 | 1.36 | 1.33 | [3.447, 3.4992, 3.5222, 3.5376, 3.5582, 3.5799] |
| 8 | jolt | P-phys | 6 | 3.5765 | 3.4444 | 3.6406 | 5.48 | 1.86 | 1.95 | [3.4444, 3.512, 3.5741, 3.5788, 3.5993, 3.6406] |
| 8 | jolt | P-full | 6 | 3.4593 | 3.4331 | 3.6621 | 6.62 | 0.74 | 2.50 | [3.4331, 3.4457, 3.4476, 3.471, 3.4719, 3.6621] |

Per-process MEDIAN step (robustness: a whole-run shift moves it, a transient does not)
| W | engine | policy | median of medians ms | range % | IQR % |
|---|---|---|---|---|---|
| 2 | boyko | P-none | 13.7520 | 0.85 | 0.63 |
| 2 | boyko | P-phys | 13.7340 | 1.06 | 0.77 |
| 2 | boyko | P-full | 13.6960 | 3.65 | 0.22 |
| 2 | jolt | P-none | 8.7491 | 3.46 | 2.08 |
| 2 | jolt | P-phys | 8.7606 | 4.64 | 2.50 |
| 2 | jolt | P-full | 8.6625 | 4.38 | 0.78 |
| 8 | boyko | P-none | 9.0698 | 1.54 | 0.11 |
| 8 | boyko | P-phys | 9.1175 | 1.22 | 0.33 |
| 8 | boyko | P-full | 9.0731 | 0.89 | 0.55 |
| 8 | jolt | P-none | 3.5842 | 2.95 | 1.47 |
| 8 | jolt | P-phys | 3.6294 | 4.19 | 1.69 |
| 8 | jolt | P-full | 3.5058 | 5.23 | 0.91 |

| comparison | level effect % | 2x comb. range % | 2x comb. IQR % | var ratio none/pol | F(5,5) 5 % |
|---|---|---|---|---|---|
| P-phys vs P-none|boyko|W2 | +0.02 | 4.58 | 2.24 | 1.41 | ns |
| P-full vs P-none|boyko|W2 | -0.66 | 8.40 | 1.45 | 0.34 | ns |
| P-phys vs P-none|boyko|W8 | +0.44 | 4.96 | 1.46 | 0.79 | ns |
| P-full vs P-none|boyko|W8 | +0.14 | 4.42 | 1.10 | 1.49 | ns |
| P-phys vs P-none|jolt|W2 | +0.85 | 13.73 | 6.96 | 0.66 | ns |
| P-full vs P-none|jolt|W2 | -0.90 | 11.80 | 5.39 | 1.18 | ns |
| P-phys vs P-none|jolt|W8 | +1.32 | 13.30 | 4.61 | 0.45 | ns |
| P-full vs P-none|jolt|W8 | -2.00 | 15.23 | 3.10 | 0.30 | ns |

| policy | W | boyko/Jolt (medians) | resolution (RSS of ranges) % | (RSS of IQRs) % |
|---|---|---|---|---|
| P-none | W2 | 1.5902 | 4.60 | 2.45 |
| P-none | W8 | 2.5870 | 4.10 | 1.46 |
| P-phys | W2 | 1.5770 | 5.59 | 2.71 |
| P-phys | W8 | 2.5646 | 5.80 | 1.93 |
| P-full | W2 | 1.5940 | 5.59 | 1.34 |
| P-full | W8 | 2.6434 | 6.79 | 0.76 |

Witness per process (pass order): mean ms, sum of per-CPU busy (CPUs), CPUs >= 50 % busy, SMT pairs both >= 30 %, proc CPU-s per wall-s
P-none|boyko|W2:
   p0 13.9558 ms  sum 1.96  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.41
   p1 13.7937 ms  sum 1.67  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.47
   p2 13.7391 ms  sum 1.58  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.48
   p3 13.8198 ms  sum 1.71  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.48
   p4 13.7218 ms  sum 1.68  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.48
   p5 13.6909 ms  sum 1.72  cpus>=50 [10, 12]  smt-both 0  cpu/wall 1.48
P-none|boyko|W8:
   p0 9.0850 ms  sum 2.8  cpus>=50 [10]  smt-both 0  cpu/wall 2.67
   p1 9.2338 ms  sum 2.88  cpus>=50 [10]  smt-both 0  cpu/wall 2.61
   p2 9.1380 ms  sum 2.97  cpus>=50 [10]  smt-both 0  cpu/wall 2.82
   p3 9.0920 ms  sum 2.88  cpus>=50 [10]  smt-both 0  cpu/wall 2.67
   p4 9.1256 ms  sum 3.0  cpus>=50 [10]  smt-both 0  cpu/wall 2.83
   p5 9.1505 ms  sum 3.0  cpus>=50 [10]  smt-both 0  cpu/wall 2.72
P-none|jolt|W2:
   p0 8.6031 ms  sum 2.24  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.98
   p1 8.6308 ms  sum 2.14  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.98
   p2 8.5852 ms  sum 2.06  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.97
   p3 8.6833 ms  sum 2.16  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.99
   p4 8.9474 ms  sum 2.54  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.98
   p5 8.8586 ms  sum 2.25  cpus>=50 [2, 10]  smt-both 0  cpu/wall 1.98
P-none|jolt|W8:
   p0 3.5582 ms  sum 8.02  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.75
   p1 3.5222 ms  sum 8.32  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.7
   p2 3.5376 ms  sum 7.63  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.55
   p3 3.4470 ms  sum 7.91  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.79
   p4 3.4992 ms  sum 7.8  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 1  cpu/wall 7.65
   p5 3.5799 ms  sum 8.14  cpus>=50 [1, 3, 5, 7, 8, 10, 12, 14]  smt-both 1  cpu/wall 7.77
P-phys|boyko|W2:
   p0 13.7663 ms  sum 1.69  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.51
   p1 13.9023 ms  sum 1.63  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.48
   p2 13.7342 ms  sum 1.65  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.48
   p3 13.7709 ms  sum 1.66  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.49
   p4 13.7325 ms  sum 1.68  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.48
   p5 13.9040 ms  sum 1.74  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.46
P-phys|boyko|W8:
   p0 9.1466 ms  sum 2.89  cpus>=50 [8]  smt-both 0  cpu/wall 2.7
   p1 9.2007 ms  sum 2.98  cpus>=50 [8]  smt-both 0  cpu/wall 2.82
   p2 9.1862 ms  sum 2.78  cpus>=50 [8]  smt-both 0  cpu/wall 2.58
   p3 9.1224 ms  sum 2.81  cpus>=50 [8]  smt-both 0  cpu/wall 2.67
   p4 9.1583 ms  sum 2.93  cpus>=50 [8]  smt-both 0  cpu/wall 2.58
   p5 9.2940 ms  sum 2.75  cpus>=50 [8]  smt-both 0  cpu/wall 2.53
P-phys|jolt|W2:
   p0 8.6245 ms  sum 2.1  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.98
   p1 9.0999 ms  sum 2.23  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.91
   p2 8.6886 ms  sum 2.12  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.97
   p3 8.6559 ms  sum 2.12  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.89
   p4 8.7730 ms  sum 2.48  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.81
   p5 8.9249 ms  sum 2.06  cpus>=50 [0, 2]  smt-both 0  cpu/wall 1.96
P-phys|jolt|W8:
   p0 3.5741 ms  sum 8.05  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.9
   p1 3.6406 ms  sum 8.61  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.63
   p2 3.4444 ms  sum 7.92  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.8
   p3 3.5993 ms  sum 8.19  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.89
   p4 3.5788 ms  sum 7.98  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.62
   p5 3.5120 ms  sum 7.89  cpus>=50 [0, 2, 4, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.63
P-full|boyko|W2:
   p0 13.7098 ms  sum 1.64  cpus>=50 [8, 10]  smt-both 0  cpu/wall 1.49
   p1 13.4072 ms  sum 1.62  cpus>=50 [8]  smt-both 0  cpu/wall 1.48
   p2 13.6617 ms  sum 1.7  cpus>=50 [8, 10]  smt-both 0  cpu/wall 1.45
   p3 13.9175 ms  sum 1.69  cpus>=50 [8, 10]  smt-both 0  cpu/wall 1.46
   p4 13.6885 ms  sum 1.74  cpus>=50 [8, 10]  smt-both 0  cpu/wall 1.46
   p5 13.6547 ms  sum 1.66  cpus>=50 [8, 10]  smt-both 0  cpu/wall 1.48
P-full|boyko|W8:
   p0 9.1504 ms  sum 3.09  cpus>=50 [8]  smt-both 0  cpu/wall 2.91
   p1 9.1498 ms  sum 2.78  cpus>=50 [8]  smt-both 0  cpu/wall 2.61
   p2 9.0684 ms  sum 2.79  cpus>=50 [8]  smt-both 0  cpu/wall 2.57
   p3 9.1386 ms  sum 2.76  cpus>=50 [8]  smt-both 0  cpu/wall 2.52
   p4 9.2050 ms  sum 3.06  cpus>=50 [8]  smt-both 0  cpu/wall 2.83
   p5 9.1298 ms  sum 3.03  cpus>=50 [8]  smt-both 0  cpu/wall 2.83
P-full|jolt|W2:
   p0 8.6150 ms  sum 2.25  cpus>=50 [2, 8]  smt-both 0  cpu/wall 2.0
   p1 8.6592 ms  sum 2.31  cpus>=50 [2, 8]  smt-both 0  cpu/wall 1.99
   p2 8.5434 ms  sum 2.3  cpus>=50 [2, 8]  smt-both 0  cpu/wall 1.98
   p3 8.5349 ms  sum 2.13  cpus>=50 [2, 8]  smt-both 0  cpu/wall 1.97
   p4 8.5347 ms  sum 2.18  cpus>=50 [2, 8]  smt-both 0  cpu/wall 1.98
   p5 8.8913 ms  sum 2.25  cpus>=50 [2, 8]  smt-both 0  cpu/wall 1.99
P-full|jolt|W8:
   p0 3.4331 ms  sum 7.79  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.66
   p1 3.6621 ms  sum 8.17  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 1  cpu/wall 7.72
   p2 3.4457 ms  sum 8.07  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 1  cpu/wall 7.53
   p3 3.4476 ms  sum 7.88  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.56
   p4 3.4719 ms  sum 7.9  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 0  cpu/wall 7.85
   p5 3.4710 ms  sum 7.83  cpus>=50 [1, 3, 5, 6, 8, 10, 12, 14]  smt-both 1  cpu/wall 7.67
