# Physics window 8 — analysis (results-analyst, 2026-09-27; recorded by the orchestrator from the returned report)

WINDOW 8: ours/Jolt 5.6 on the wall is 0.45x at W1 (claimed), 0.80x at W8 and 0.87x at W16 (neither claimed). Per
manifold it is 0.82x / 1.46x / 1.57x (only W16 claimed, and there Jolt is cheaper). 70 % of the W8 loss is serial: our
serial chain is 1.31 ms against Jolt's 0.63 ms, and the Jolt profile confirms its jobs run in sequence. The identity
closes (+0.35 % after N1). S4's fill share is 0.82, so S4 is larger than designed. S6 fails its build-if (3.7 %).
omega_b measures 2.8-3.0 us per stage at 8 participants, three times the design's bracket. No wall or span resolution
was demonstrated under the two-spread rule. DM1 C6 passes the letter of its gate, but the gate could not have seen a
regression under ~23 %.

All tables: `analysis/tables8.txt`; scripts `analysis/{lib8,sel8,common8,q0_misc,q1_headline,q2_identity,q2_run,
q3_waves,q4_resolution,q5_micro,q6_jolt,jprof,q7_sleep,q8_dm1}.py`; outputs `analysis/{proc8.json,q2_*.json,q6.json}`;
sources at 226bd99e extracted read-only under `analysis/src/`.

## Method
Recomputed from `raw/*/run.csv`, Jolt `per_frame_*.csv`, profile dumps, `wfb_*.csv`, micro SUMMARY lines, DM1
`artifact.toml` — nothing taken from the driver's statistics. Cell = median over K of per-process means. r = min-max,
i = IQR, s = 1.2533*SD/sqrt(K), relative to the median. Claimed iff |B/A-1| > 2*hypot under both r and s; flags r/i/s.
Slot rule (window 7): original if valid and clean, else its re-run, else dropped; clean = both receipts <= 5 % and the
witness <= 2 %. Jolt only in block W8S-A; our J-T in W8S-A (J-T) and W8S-R (R-J-T).

Selection: 339 timed processes (337 originals, 2 re-runs), 337 used, 0 dropped, 0 validity problems. Per-process means
equal the driver's `mean_ms` 283/283; recomputed r/g/u equal the runner's columns; per-step identity
T = S + np_disp + wide + r + u + g exact to 1e-15 ms. Receipts max 4.35 %; witness medians 0.12-0.37 % per pass, max
1.97 %. `claude.exe` is the witness's top process in 248 of 339 processes.

## 1. Standing against Jolt 5.6 (q1.txt)
Median over K = 6 of the process mean over [0,500). Per manifold on [100,500), each side's own count: ours 4,467.665
(every process), Jolt 8,489.0 (window 7's `-receipt`, not re-measured).

| W | ours J-T (A) | ours R-J-T (R) | Jolt (A) | ours/Jolt A | ours A+R / Jolt | ns/manifold ours A / R / Jolt | per-manifold ratio A (R) |
|---|---|---|---|---|---|---|---|
| 1 | 4.4701 [4.417-4.610] | 4.3893 | 9.9612 [9.632-12.153] | **0.449 Y/Y/Y** (bars 51.4 / 11.1 %) | 0.445 Y/Y/Y | 951.6 / 933.1 / 1166.1 | 0.816 n/n/Y (0.800 n/n/Y) |
| 8 | 2.0357 [1.987-2.136] | 1.9944 | 2.5598 [2.454-3.661] | 0.795 n/Y/Y (95.4 / 18.8) | 0.779 n/Y/Y | 447.9 / 436.4 / 307.6 | 1.456 n/Y/Y (1.419 n/Y/Y) |
| 16 | 2.2233 [2.202-2.336] | 2.2120 | 2.5674 [2.349-2.902] | 0.866 n/n/Y (44.7 / 9.5) | 0.863 n/n/Y | 488.6 / 486.0 / 311.7 | **1.567 Y/Y/Y** (1.559 Y/Y/Y) |

- Claimed: W1 wall (ours faster), W16 per manifold (Jolt faster). W8 not claimed: Jolt's range 47 %, one bursty
  process (`W8S-A p1 r0`, 3.661 ms; 17.5 % of frames > 1.2x median).
- [100,500): 0.430 Y/Y/Y, 0.766 n/Y/Y, 0.825 n/n/Y. [0,100): 0.526 Y/Y/Y, 0.924 n/n/n, 1.060 n/n/n.
- T1/T8 2.196 ours vs 3.891 Jolt; T1/T16 2.011 vs 3.880. W16/W8 ours +9.2 % n/n/Y (A), +10.9 % n/Y/Y (R); Jolt +0.3 %.
- Contact reuse on vs off (J-T / J-T-off): -26.7 % Y/Y/Y W1, -8.5 % n/n/Y W8, -6.7 % n/Y/Y W16.

Against window 7 (block B, trunk 93b2615b, reuse off; engine changed since — no claim):

| | window 7 (B) | window 8 |
|---|---|---|
| ours/Jolt W 1/8/16 | 0.620 / 0.872 / 0.981 | 0.449 / 0.795 / 0.866 |
| per manifold W 1/8/16 | 1.178 / 1.601 / 1.787 | 0.816 / 1.456 / 1.567 |

Our walls 5.957 / 2.188 / 2.363 -> 4.470 / 2.036 / 2.223 ms; Jolt ~9.60 / 2.509 / 2.408 -> 9.961 / 2.560 / 2.567 (W1,
W16 inflated by outliers, §10). W8 per manifold matches window 7 §6's post-C4 arithmetic (~1.44x).

## 2. The identity at W8 and W16 (q2.txt; J-T-a / R-J-T-a vs J-T / R-J-T; [100,500); ms)
S(1) = 1.2197 (serial spans 1.1713 + g(1) 0.0380 + u(1) 0.0104; np_ser(1) := np_ser(8) = 0.0373, estimate; u(1) holds
the S6 probe, N7). P(1) = 3.0157 (np 0.6725 + wide 2.3432). s = S(1)/T(1) = 0.285.

| | T_a | I | P(1)/W | L (np + wide) | dg | du | identity | residual % solve span | after N1 (dr) | u/solve | D(W) (N2) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| W8 (A) | 2.0648 | +0.0965 | 0.3770 | 0.3469 (0.0298 + 0.3171) | -0.0065 | +0.0005 | 2.0341 | +0.0307 = 2.13 % | +0.0050 = 0.35 % (0.0257) | 0.76 % | +0.0037 |
| W8 (R) | 2.0316 | +0.0686 | 0.3770 | 0.3347 | -0.0070 | +0.0004 | 1.9934 | 2.67 % | 0.89 % | 0.76 % | +0.0222 |
| W16 (A) | 2.2899 | +0.1587 | 0.1885 | 0.6867 (0.0571 + 0.6296) | -0.0040 | +0.0007 | 2.2503 | +0.0396 = 2.37 % | +0.0049 = 0.29 % (0.0347) | 0.67 % | +0.0379 |

- Closure within ±3 % before N1; within 0.9 % after. [0,100) residual after N1: -0.01 % W8, +0.07 % W16.
- N2: D(W) bounds stamp inflation of L_wide at 0.04-0.23 us/wave (W8) and 0.39 us (W16) = 1-7 % and 6 % of L_wide
  (below the review's 8-17 % estimate).
- W8 excess over T(1)/8 = 1.529 ms: serial S(1)(1-1/8) = 1.067 (**70 %**), parallel loss L 0.347 (23 %), interference
  I 0.097 (6 %).
- Serial stages at W8: setup 0.319 (fill P-c **0.261**, P-b 0.031, P-a 0.024, bodies 0.004); **warm apply 0.304**;
  broadphase 0.266 (query 0.210); graph 0.121; integrate group 0.121; narrow colours 0.047; gather/select/apply 0.039;
  np serial pieces 0.037.
- W16 excess 2.022 ms: 57 % serial, **34 % parallel loss**, 8 % interference. W16 - W8 = +0.225 ms armed: wide colours
  +0.166 (74 %); serial +0.062 = SMT interference (integrate +0.016, narrow +0.014, warm +0.011, bp +0.007, graph
  +0.006, np serial +0.005, setup +0.005); r +0.009; np dispatch -0.015.
- Armed/disarmed perturbation: +1.0 % n/n/n W1, +3.7 % n/n/n W8, +4.9 % n/n/Y W16.

## 3. Per wave, route, histogram, stage ratios, S6 (q3.txt, q2.txt; [100,500))

| row, W | waves/step | tasks/wave | omega = L_wide/waves | omega after D | ramp/wave | tail/wave | in-flight / lanes |
|---|---|---|---|---|---|---|---|
| J-T-a, 8 | 96 | 23.4 | **3.30 us** | 3.27 | 2.03 | 3.32 | 8.00 / 8.00 |
| R-J-T-a, 8 | 96 | 23.4 | 3.23 | 3.00 | 2.02 | 3.26 | 8 / 8 |
| J-T-a, 16 | 96 | 23.4 | **6.56** | 6.16 | 2.16 | 4.98 | 14.49 / 15.39 |
| J-T-off-a, 8 / 16 | 108 | 21.4 | 3.02 / 6.05 | - | 1.83 / 2.01 | 2.90 / 4.76 | - |
| J-A-a, 8 / 16 | 96 | 31.0 | 6.75 / 12.80 | - | 2.59 / 2.74 | 8.81 / 13.59 | - |

- omega(8) rose from the design's 2.88 us (window 7, reuse off, 109.32 waves) to 3.30 us. E(8) 0.521 (np 0.738, wide
  0.480); E(16) 0.215 (np 0.424, wide 0.189).
- Ramp + tail per wave (5.35 us at W8) = 162 % of omega: the ramp overlaps the caller's own work. First-wave ramp not
  recoverable (B1); tail = imbalance + join unsplit (B2). W16 growth mostly in the tail: +1.66 us/wave x 96 ~ 0.16 ms.
- np wave (J-T W8): dispatch 0.114 ms, ramp 2.76 us, **tail 30.5 us**, 8 lanes; route not recorded (N6).
- Route: every armed step on every row ran on the worker route; `solve_on_dispatcher_steps` 0 everywhere.

Colour histogram per step, W8, [100,500):

| | [1,32) | [32,64) | [128,256) | >= 256 | narrow slots | narrow span |
|---|---|---|---|---|---|---|
| J-T (reuse on) | 1.88 colours / 23.3 slots | 0 | **1.00 / 211.2** | 8.00 / 16,319 | 234.5 | 0.047 ms |
| J-T-off | 0.81 / 5.9 | 0.88 / 30.8 | 0 | 9.00 / 16,984 | 36.7 | 0.011 ms |

Reuse on drops one ~211-slot colour under the 256-slot wide threshold (x6.5 narrow rise); design's r = +0.036 ms at W8
(low end of [0, 0.16]). J-A's scalar narrow colours 0.19 ms.

Stage ratios W1/W8 and W1/W16 (J-T-a): wall 2.08/1.87; bp 0.96/0.94; np 4.71/5.03; graph 0.96/0.92; setup 0.92/0.91;
warm 0.96/0.93; integrate group 0.80/0.71; wide 3.84/3.02; narrow 0.83/0.64.

S6 hit rates (graph / P-b), [100,500): J-T **0.535 / 0.3275** (identical at W 1/8/16, every process); J-Son 0.655 /
0.5825; J-T-off 0.3325 / 0.005; every row 0/0 over [0,100).

## 4. Resolution (q4.txt; block W8S-R, reference R-J-T at the same W)

| rung | injected | rise [0,500) | flags | bars r / s |
|---|---|---|---|---|
| W8 F0.5 | 0.030 | +0.0422 | n/n/n | 13.4 / 2.6 % |
| W8 F1 | 0.060 | +0.0613 | n/n/n | 16.8 / 3.2 |
| W8 F1.5 | 0.090 | +0.0938 | n/Y/Y | 15.5 / 3.0 |
| W8 F2 | 0.120 | +0.1363 | n/Y/Y | 15.5 / 3.0 |
| W16 F0.5/F1/F1.5/F2 | 0.0525/0.105/0.1575/0.210 | +0.049/+0.098/+0.156/+0.213 | n/Y/Y each | 9.0-9.8 / 1.6-1.8 |
| rung W1 / W2 / W4 | 0.060 | +0.064 / +0.092 / +0.086 | n/n/Y, n/n/Y, n/Y/Y | 5.0 / 11.2 / 11.0 (r) |

- R_8, R_16 NOT demonstrated under the two-spread rule. W 1/2/4 rungs "unresolved above 5.04 %, 11.20 %, 10.98 %".
  SE alone: R_8 = 0.090 ms, R_16 = 0.0525 ms.
- Zone canary on `phys_solve_build` (ref R-J-T-a W8, span 0.3108 ms, range 17.3 %) not seen: 30 us span +0.042 n/n/n
  (62.7 / 13.9 %); 60 us span +0.072 n/Y/Y (101.5 / 20.2 %); wall +0.040 n/n/n, +0.074 n/n/n.
- S4's 0.060 ms bar resolvable neither on wall nor on span here. SE alone: span resolves 60 us, not 30 us.
- Cause: pass 0 of W8S-R, not K. Pass 1 alone (diagnostic, K = 3) sees every W8 rung from 0.030 ms (Y/Y/Y, bars
  ~1.4-2.2 %; F2 n/Y/Y) and every W16 rung. In pass 0 single processes run uniformly 4-13 % slow (median step shifts,
  no bursts; e.g. R-J-T r1 +5.7 %; zone-N60000 r1 +10.3 %, span 0.552 ms), receipts <= 1.9 %, witness 0.0-0.9 %. No
  receipt, witness or per-CPU record identifies them.

## 5. omega_b and omega(W, gap) (q5.txt)
Per-stage cost (N5) = (region at 72 stages - region at 36) / 36, cell medians, ns:

| participants | worker | external | intercept worker / external |
|---|---|---|---|
| 2 | 342 | 403 | 3.7 / 0.9 us |
| 4 | 1,186 | 988 | -2.8 / 6.1 |
| 8 | **3,000** (paired 3,453 [1,883-4,031]) | **2,825** | 10.7 / 8.8 |
| 16 | 5,049 | 5,464 | 35.7 / 1.4 |

- ~Linear in participants (contention signature); at 8 = 2.8-3.0x the design's upper bracket 1.0 us and ~ today's
  omega(8) 3.3 us. Zero-work `pool.scope` of 32 tasks with helpers awake (gap 5 us): 2.55 us worker / 4.15 external.
- S1 arithmetic at this omega_b (estimate): wide term (3.30 - 0.3 - 3.0 - 0.7) us x 96 ~ -0.07 ms; S2 ~ 0.12; S3 ~
  0.06; narrow ~ 0; bundle ~ 0.11-0.14 ms = 5.5-6.8 % of T(8) (vs 20-26 % at omega_b 0.3-1.0).
- Not decidable from this bench: zero-work blocks (16 relaxed stores) make all participants contend at once; the
  bench-only shared `runs.fetch_add` per block sits in `Region` beside `publish` and Vec headers (false sharing — a
  reading of `omega_b_region.rs`, not measured).

omega(W, gap) scope medians (us) at gap 0 / 5 / 20 / 80: W8 worker 7.6 / 2.55 / 9.7 / 9.8, external 4.9 / 4.15 /
10.9 / 11.0; W16 worker 10.6 / 11.25 / 20.1 / 20.1, external 5.2 / 6.9 / 21.2 / 21.15. N4 lower bound omega(80) -
omega(0): +2.2 / +6.1 us W8, +9.5 / +15.95 us W16, all Y/Y/Y. W8 worker non-monotone (gap 0 > gap 5); from the gap-5
minimum the bound is 6.9-7.3 us at W8 (diagnostic). Helpers park before 20 us. Region recruitment ~9-11 us at 8
participants (intercept, estimate).

## 6. Jolt's critical path (q6.txt; j56p frames 100/200/300/400, K = 6; WaitingForBatch over [100,500))
Lighter profile costs ~0-1 % (j56p [100,500): 9.916 / 2.640 / 2.480 vs j56 9.899 / 2.611 / 2.646) — closes window 7
FOLLOW-UP 10.

| ms | W1 | W8 | W16 |
|---|---|---|---|
| FindCollisions (parallel) | 2.514 | 0.724 | 0.569 |
| FinalizeIslands (serial) | 0.011 | 0.016 | 0.023 |
| Island sort + split (one thread) | 0.537 (0.396 + 0.140) | **0.578** (0.422 + 0.152) | **0.705** (0.515 + 0.186) |
| Parallel velocity solve | 5.665 | 1.054 | 0.914 |
| SolvePosition | 1.047 | 0.228 | 0.220 |
| Other serial jobs and gaps | 0.114 | 0.051 | 0.080 |
| **Serial chain** | 0.648 | **0.630** | **0.786** |
| Update | 9.854 | 2.638 | 2.468 |
| WaitingForBatch sum over threads / frame | 0.0002 | **5.235** | 14.46 |
| ... per non-prep thread / prep thread | - | **0.727** / 0.149 | 0.948 / 0.243 |

- Jobs strictly in sequence (FindCollisions -> FinalizeIslands -> SolveVelocity [serial Island, then parallel
  batches] -> Integrate -> SolvePosition); only UpdateBroadPhasePrepare (0.064) and ApplyGravity overlap the start.
- W8: each of 7 other threads waits 0.727 ms = 0.578 prep + 0.149 barrier waits.
- Ruling 5 (window 7 §5 re-read): design §4.2 CONFIRMED; window 7 §5's "Jolt's serial work overlaps its parallel
  work" REFUTED.
- Jolt's losses: W8 22 % of frame is serial sort + split while 7 threads yield (4.05 thread-ms/frame) + 0.15 ms/thread
  barrier waits; W16 serial prep +22 % (SMT), waits 37 % of thread time. Parallel efficiency Jolt 0.575 (W8) / 0.34
  (W16), ours 0.52 / 0.22 — at W16 our parallel machinery is clearly worse.

Serial chains: ours 1.310 ms (293 ns/manifold) W8, 1.375 W16; Jolt 0.630 (74 ns/manifold) W8, 0.786 W16; ratio
**2.08x** W8, 1.75x W16. Per manifold at W8: bp + np + setup + warm ours 232.5 ns vs Jolt 85.2 (FindCollisions);
solve ours 147.1 vs Jolt 150.9.

## 7. Sleeping: J-Son vs J-A, cfg a (q7.txt)

| window | W1 disarmed | W8 disarmed | W8 armed |
|---|---|---|---|
| [0,100) awake | +0.55 % n/n/n (3.6 / 0.7) | +0.46 % n/n/n (5.3 / 0.9) | +0.55 % n/n/n |
| [100,500) | **-49.35 % Y/Y/Y** | **-35.65 % Y/Y/Y** (5.479 -> 3.526 ms) | -36.6 % Y/Y/Y |

- Awake cost unresolved under both spreads; L10 serial pieces ~17.7 us/step armed at W8 (begin 3.8, freeze 1.7,
  end 4.1, classify 8.0).
- Binary on this scene: the single island freezes at step 274 in every process; over [100,274) J-Son -0.03 %; over
  [274,500) 1.96 ms W1 / 2.01 ms W8, a floor dominated by the AllPairs broadphase (1.87 ms), which sleeping does not
  reduce.
- Pose caveat: J-Son 0x3db47fae414b655c by design (equal at every W and process), not J-A's pose.
- For L10 C1b: no measurable awake cost; a large settled-pile gain whose size on the product row depends on the tree
  broadphase — that row was not measured.

## 8. DM1 C6 (q8.txt; 1920x1080 only, FIFO; zone median over 220 frames; A B B A)

| row, zone | A1 / B1 / B2 / A2 (us) | median(B-A) | band | gate |
|---|---|---|---|---|
| vb idle, VB_SHADE | 300.5 / 249.9 / 318.5 / 293.9 | -13.1 (-4.4 %) | 68.6 | B <= A |
| vb idle, VB_PRODUCE_NET | 300.5 / 249.9 / 319.0 / 294.4 | -13.1 | 69.1 | B <= A |
| deferred idle, GBUF_DEFERRED_RESOLVE | 605.2 / 571.4 / 446.5 / 607.2 | -97.3 (-16 %) | 124.9 | B <= A |
| vb edit100, SHADE / PRODUCE_NET | - | +18.7 / +18.4 | 30.2 / 29.7 | inside band |
| deferred edit100 | - | -27.6 | 149.7 | B <= A |

KEEP C6 by the letter, with qualifications: (1) no power — repeats of one binary differ up to 25 %, bands 21-25 % of
the zone, a B regression < ~23 % invisible; (2) only 1920x1080 measurable; (3) VB_EARLY_CULL (outside the gate) B-A
+22.3 us (+2.8 %) idle above its 18.9 us band and +24.1 us with the edit above its 3.1 us band, B slower in all four
pairs, VB_RUN moves +26 us — unexplained; re-read before C6 is relied on. 100-row edit: gated zones -78..+35 us, all
noise; copy and CPU stager unzoned (as the cut foresaw). Grow frame NOT MEASURED (no per-frame record; the implied
220 x dmean is noise). Dev-profile host: pairing like-for-like; FIFO idles the GPU between frames either way; the ±20 %
spread looks like GPU power state; absolute values are not product numbers.

## 9. Consequences for the lane order (RULINGS-2026-09-26)
Confirmed: T(8) ~ 1.97 (+0..0.16) -> 2.001 / 1.950 over [100,500); T(1) ~ 4.30 -> 4.25 / 4.17; 441 ns/manifold at
W8 (1.47x Jolt) -> 448 / 436 (1.46x / 1.42x); serial ~63 % of T(8) -> 61 %; W8 excess split 71/25/4 -> 70/23/6;
Jolt serial ~0.58 not overlapped -> 0.578 + 0.051; S5 query >= 5 % of T(8) -> 10.5 % (0.210 ms; S5 ~ 0.15-0.16 ms,
arith.); narrow rise = a colour under 256 slots, r = 0.036 ms.
Refuted / moved: P-c fill share f = 0.82 (not 0.5-0.75) -> S4 ~ 0.19-0.22 ms (arith.), not 0.10-0.15; omega_b as
benched 2.8-3.0 us/stage at 8 participants (not 0.3-1.0); S6 delta ~ 0.070-0.075 ms = 3.5-3.8 % of T(8) < the 5 %
build-if (lower of J-T's and J-Son's rates = J-T's); W>=2 resolution not demonstrated.
Per lane: S4 first and larger — its claim needs R_8 or the span canary, demonstrated in the S4-AB block itself.
F3 -> tree C4 -> S5 unchanged; S5 supported. S1+S2+S3 on the 5 % line at the benched omega_b; B1/B2 unknown — decide
after window 8b's corrected omega_b. S7 supported (W16 0.19-0.22 ms slower than W8 on J-T, n/Y/Y in R; 74 % wide
waves, ~28 % serial SMT interference); its partial form (lanes cap at physical cores on today's scope path, design
§6.7) need not wait for S1. S6 not built. C1b: product row (sleeping on the tree broadphase) unmeasured. L8, L12: no
reading.
Window 8b must add: (1) fixed instrument B1, B2, N3, N6, N2's padded TaskStamp; (2) S4-AB with its own ladder, rung
and zone canary, and a pre-registered handling of pass-0-style slow processes; (3) omega_b re-bench (~0.7 us work per
block, no shared `runs` RMW, padded claim/done words, blocks at 1x/2x/4x P, P capped at 8, spin-then-park variant);
(4) N4 participation receipt, gap 5 as the awake baseline; (5) J-Son on the tree broadphase; (6) DM1 per-frame grow
record, more ABBA repeats or controlled GPU clocks, VB_EARLY_CULL re-read, a display for 1440p/2160p.

## 10. Anomalies (q0.txt, q1.txt, q4.txt)
- H-jolt56 W8 re-run: original `W8S-A p1 r2` (19:08:22) 3.169 ms, witness 3.29 % (`claude.exe` 0.359 s, `git.exe`
  0.297 s); re-run 19:10:03 2.501 ms, witness 0.23 %, used. Micro `omega-worker p0 r2` (witness 3.62 %) re-run too.
- Under the 2 % gate: Jolt W8 p1 r0 (3.661, bursty, witness 1.01 %) and J-T W8 p1 r0 (bursts 7.2 % of steps) ran
  seconds apart 19:01:46-53; J-T W8 p1 r2 (19:08:29) with the git.exe event — agent activity under the gate.
- Jolt W1 p0 r1 and p1 r0 (11.47, 12.15 vs 9.6-9.99): uniform slowdowns, main thread spread over three logical CPUs
  (<= 44 % on one) where fast runs sit 64-88 % on one — placement under P-none.
- W8S-R pass-0 slowdowns: no identifiable cause in the record.
- Receipts: every used process passes (max 4.35 %), 0 validity failures. omega_b P4 worker intercept negative
  (2-point fit, r 36-39 %); omega W8 worker gap 0 > gap 5; DM1 ±20 % between repeats.
