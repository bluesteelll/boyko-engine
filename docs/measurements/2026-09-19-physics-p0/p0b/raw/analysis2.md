## Two-way decomposition of log(window mean): pass x policy (no replication)

| engine | W | SD pass effect % | SD policy effect % | residual SD % | F pass (5,10) | F policy (2,10) |
|---|---|---|---|---|---|---|
| boyko | 2 | 0.00 | 0.35 | 0.93 | 0.60 | 1.85 |
| boyko | 8 | 0.19 | 0.18 | 0.55 | 1.36 | 1.67 |
| jolt | 2 | 1.04 | 0.73 | 1.47 | 2.50 | 2.46 |
| jolt | 8 | 0.70 | 0.65 | 1.83 | 1.44 | 1.76 |

(* = significant at 5 %: F(5,10) > 3.33, F(2,10) > 4.10)

### Pass effects (% of the grand mean), per (engine, W)

| engine | W | p0 | p1 | p2 | p3 | p4 | p5 |
|---|---|---|---|---|---|---|---|
| boyko | 2 | +0.41 | -0.39 | -0.30 | +0.60 | -0.29 | -0.03 |
| boyko | 8 | -0.29 | +0.44 | -0.25 | -0.40 | +0.10 | +0.40 |
| jolt | 2 | -1.14 | +0.93 | -1.24 | -1.02 | +0.43 | +2.03 |
| jolt | 8 | -0.05 | +2.38 | -1.35 | -0.73 | -0.19 | -0.06 |

## Paired (same pass) vs unpaired spread of the policy effect

| comparison | unpaired: ratio of medians | paired: median of per-pass ratios | paired range % | paired IQR % |
|---|---|---|---|---|
| P-phys/P-none boyko W=2 | 1.0002 | 1.0002 | 2.91 | 0.88 |
| P-full/P-none boyko W=2 | 0.9934 | 0.9959 | 3.52 | 1.22 |
| P-phys/P-none boyko W=8 | 1.0044 | 1.0044 | 1.92 | 0.30 |
| P-full/P-none boyko W=8 | 1.0014 | 1.0014 | 1.78 | 1.29 |
| P-phys/P-none jolt W=2 | 1.0085 | 1.0050 | 7.35 | 1.26 |
| P-full/P-none jolt W=2 | 0.9910 | 0.9983 | 4.99 | 1.69 |
| P-phys/P-none jolt W=8 | 1.0132 | 1.0136 | 6.96 | 4.34 |
| P-full/P-none jolt W=8 | 0.9800 | 0.9831 | 7.62 | 2.79 |

## boyko/Jolt per policy: paired within the pass vs unpaired

| policy | W | unpaired (median/median) | paired median | paired range % | paired IQR % | per-pass ratios |
|---|---|---|---|---|---|---|
| P-none | 2 | 1.5902 | 1.5949 | 5.55 | 2.68 | [1.6222, 1.5982, 1.6003, 1.5915, 1.5336, 1.5455] |
| P-none | 8 | 2.5870 | 2.5955 | 3.25 | 2.13 | [2.5533, 2.6216, 2.5831, 2.6376, 2.6079, 2.5561] |
| P-phys | 2 | 1.5770 | 1.5730 | 4.35 | 1.82 | [1.5962, 1.5277, 1.5807, 1.5909, 1.5653, 1.5579] |
| P-phys | 8 | 2.5646 | 2.5591 | 5.46 | 3.28 | [2.5591, 2.5273, 2.667, 2.5345, 2.559, 2.6463] |
| P-full | 2 | 1.5940 | 1.5952 | 5.95 | 2.73 | [1.5914, 1.5483, 1.5991, 1.6307, 1.6039, 1.5357] |
| P-full | 8 | 2.6434 | 2.6413 | 6.32 | 0.78 | [2.6654, 2.4985, 2.6318, 2.6508, 2.6513, 2.6303] |

## Pooled over the three policies (18 processes per engine and W)

| engine | W | n | median ms | min | max | range % | IQR % | SD % |
|---|---|---|---|---|---|---|---|---|
| boyko | 2 | 18 | 13.7367 | 13.4072 | 13.9558 | 3.99 | 0.86 | 0.92 |
| boyko | 8 | 18 | 9.1482 | 9.0684 | 9.2940 | 2.47 | 0.57 | 0.60 |
| jolt | 2 | 18 | 8.6576 | 8.5347 | 9.0999 | 6.53 | 2.67 | 1.90 |
| jolt | 8 | 18 | 3.5171 | 3.4331 | 3.6621 | 6.51 | 3.53 | 2.04 |

## Whole-run shift or transient? (per-process median step against the window mean, pooled)

| engine | W | range % of means | range % of per-process medians | corr(mean, median) |
|---|---|---|---|---|
| boyko | 2 | 3.99 | 3.64 | 0.90 |
| boyko | 8 | 2.47 | 1.93 | 0.93 |
| jolt | 2 | 6.53 | 5.70 | 0.98 |
| jolt | 8 | 6.51 | 5.22 | 0.95 |
