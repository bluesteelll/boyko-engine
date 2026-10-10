# P0b timed window: MEASUREMENT-QUEUE §10 under the ruled protocol (tester, 2026-09-19)

## Verdict

**The window completed with K = 6 in every cell.**
- 258 processes were used: 43 (row, W) cells × 6 processes. None was excluded and none was invalid.
- 2 processes were contaminated. Both were re-run clean at the end of their pass.
- Placement is **P-none**, the diagnostic's recommendation, on both engines. No affinity call was made; every process read back mask `0xffff`.

**The canary is seen at W=1 and at W=8, under all three readings of "twice the combined spread".**

| W | expected rise | measured J-C/J-A-a | bar: 2 × range | bar: 2 × IQR | bar: 2 × SE | seen? |
|---|---|---|---|---|---|---|
| 1 | +5.00 % | **+5.06 %** | 2.08 % | 1.02 % | 0.41 % | yes, all three |
| 8 | +4.98 % | **+4.81 %** | 4.17 % | 2.79 % | 0.91 % | yes, all three |

At W=8 the min-max margin is thin (4.81 against 4.17).

**Headline: boyko against Jolt (J-A disarmed against the timed Jolt row).** Every ratio below clears every bar.

| W | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| boyko / Jolt v5.3.0 | 1.251 | 1.559 | 2.047 | 2.593 | 2.858 |
| boyko / Jolt v5.6.0 | 2.038 | 2.402 | 3.008 | 3.675 | 3.749 |

This is cfg-A, profile `dev`, disarmed, on the msvc host, with P-none placement.

**Under the ruling's min-max reading, this window cannot resolve any of its small A/A questions.** Each bar is larger than the effect its gate exists to detect:

| comparison | effect | bar at 2 × combined range |
|---|---|---|
| A/A1 (arming) | +0.11 % at W=1, +0.74 % at W=8 | 7.71 %, 4.14 % |
| A/A2 (permanent sites) | +0.51 %, +0.30 % | 7.94 %, 6.26 % |
| shipping against dev | −0.55 %, −0.53 % | 10.21 %, 7.28 % |
| L1's gate (tip against pre-L1) | +0.68 % | 8.91 % (the effect L1 targets is +2.44 %) |

None of these is claimed under the IQR or SE readings either (bars 0.73–2.83 %).
- **"Not claimed" is not "no difference".** Under min-max it means "cannot be seen at this K".
- The diagnostic's §6 decision (which spread the rule means) is now **load-bearing**. The canary passes under every reading only because its pair, J-A-a and J-C, happens to be tight. Against J-A disarmed at W=1 (range 3.84 %), the same 5 % canary would **not** clear the min-max bar (2 × hypot(3.84, 0.99) = 7.9 %).

**Three findings the numbers carry**
1. **The broadphase crossover is near n ≈ 1.1k (uniform lattice) and ≈ 3.0k (size-disparity scene), not in the 96–192 band.**
   - `GRID_HI` = 192 would switch Auto to Grid 6–15× too early.
   - At n = 100, Grid is 8.1× slower than AllPairs.
   - In-scene cross-check: cfg-B's Grid broadphase costs **+3.07 ms per step at W=1 and +3.09 ms at W=8** on the 1240-body pile (5.17 ms against 2.10 ms for AllPairs).
2. **boyko does not scale from 8 to 16 workers:** 9.162 ms against 9.172 ms (+0.11 %, not claimed). Over the same step, Jolt v5.3.0 drops 9 % (3.533 → 3.209 ms).
3. **The sleeping floors differ by more than three orders of magnitude.** On the all-asleep tail [264, 1000), boyko's step costs 5.33 ms (W=1) and 5.41 ms (W=8); Jolt's costs 2.2 µs.

## Protocol as run

- **Statistic:** each process's window mean of the per-step wall time; per cell, the median over K = 6 processes, with min–max, range %, IQR % (inclusive quartiles) and SD %, all as % of the median.
- **Claim rule:** |effect| > 2 × hypot(spread_A, spread_B). This is the combination the P0b diagnostic used. It is read three ways:
  - min–max range (the ruling);
  - IQR;
  - the median's standard error, 1.2533 · SD / √K per side. This reading is supplementary; it is the diagnostic's suggested gate.
- **Order:** two passes, each of 3 rounds over all 43 cells.
  - Pass 0 runs W ascending. Inside a W group Jolt goes first, and the Jolt cells are spread evenly among the boyko cells (as close to alternating as 28 boyko against 15 Jolt cells allows).
  - Pass 1 runs the whole list reversed.
  - One untimed warm-up process (J-A disarmed, W=8) opens each pass. It read 9.07 and 9.16 ms.
- **Rows and flags:** the driver's own row table and argument builders, from a byte-identical copy of `D:/wt/joltab/tools/physics_parity/driver.py` in `p0b/lib/` with bytecode writing off.
  - Pre-L1 is the driver's `L1-A1` row. L1's B side is `L1-B`.
  - Jolt timed rows: `-s=Pyramid -q=Discrete -f -t=W -i=500`; `-allow_sleep` rows use `-i=1000`.
  - JOLT-P: the Release build, `-s=Pyramid -q=Discrete -p -t=W -i=500`.
- **Launch:** the diagnostic's P-none path. `CreateProcess` suspended, mask read back, resumed; `SetProcessAffinityMask` is never called.
- **Receipts:**
  - the idle rule (`p0/wait_idle.ps1`) before each pass and before the broadphase bench;
  - a 10-s receipt opening each pass;
  - a 5-s receipt between consecutive processes, serving as the after of one and the before of the next.
  - A busy before-receipt waits for quiet: 20-s steps, up to 420 s, then the idle rule.
  - A busy receipt (> 5 %, or a build process using CPU) marks the process contaminated; it is re-run once at the end of its pass.
- **Witnesses (not gates), recorded per process:**
  - its own CPU seconds;
  - the machine-wide busy fraction of each logical CPU over its lifetime;
  - the CPU seconds of every other process during it (a Toolhelp walk before launch and after exit);
  - each CPU's `% Processor Performance`, sampled at 1 Hz by a thread in the parent.
- **Nothing was compiled.** Every binary matched the manifest before each pass and matched `bin/SHA256SUMS` and the manifest after the window.

## Wall clock (+03:00)

| stage | start | end |
|---|---|---|
| launch; idle rule (3 of 3 polls quiet) | 06:49:05 | 06:51:16 |
| pass 0 (warm-up + 129 processes, no re-runs) | 06:51:26 | 07:15:59 |
| idle rule (3 of 3 quiet) | 07:16:04 | 07:18:15 |
| pass 1 (warm-up + 129 processes + 2 re-runs at the end) | 07:18:25 | 07:44:11 |
| idle rule (3 of 3 quiet) | 07:44:16 | 07:46:26 |
| broadphase bench (criterion, 260.5 s) | 07:46:39 | 07:51:00 |
| **window** | **06:49:05** | **07:51:05** (62 min) |

- **Machine:** High performance plan, on AC (battery status 2, 100 %) before and after. There were 240–241 processes.
- **Load:** Task Manager, Steam and a browser were open, as in P0.

## Receipts and contamination

- **Distinct receipts:** 266 (2 × 10 s opening a pass, 264 × 5 s).
  - Busy: min 0.27 %, **median 1.26 %**, p90 2.06 %, max 14.0 %.
  - 2 were above 5 %. None saw a build process.
  - The top process was `Taskmgr.exe` in 241 of 266.
- **Idle polls:** 9 of 9 quiet.
- **Broadphase bench's own receipts:** 1.02 % before (10 s) and 1.17 % after (5 s). Other processes used 0.77 % of the machine during the run.
- **Contaminated: 2 of 260 attempts.** Both were caught by the after-receipt and re-run once at the end of pass 1; the re-run is the value used.

| process | after-receipt | during (other processes) | original | re-run (used) |
|---|---|---|---|---|
| L1-A (pre-L1) W=1, pass 1 round 1, 07:33:03 | **14.0 %** (top: `RuntimeBroker.exe` 1.83 CPU-s) | 0.82 % | 19.3768 ms | 19.6318 ms |
| Jolt `-allow_sleep` W=1, pass 1 round 2, 07:41:23 | **7.17 %** (the readable top 5 total ~0.7 %: the load came from a process Toolhelp cannot open) | 1.02 % | 1.9082 ms | 1.9734 ms (the cell's max, +4.2 %) |

  The process after each busy receipt waited 25 s for quiet.
- **During-process witness:** median 0.74 % of the machine; 5 of 260 attempts above 2 %, the highest being 2.03 %, 2.39 %, 3.12 %, 4.55 % and 7.5 %. The three above 3 % are all S16 processes of about 0.065 s, where one Task Manager slice dominates the denominator.

## Per (row, W): median over K = 6 processes of the window mean

Windows: J rows [0, 500) (J-Son [0, 1000)); R and R-ref [600, 1100); R-S [300, 800); S16 [0, 300); Jolt 500 frames (`-allow_sleep` 1000). JOLT-P's value is 1000 / its printed steps/s on a **profiled** Release build, not a timing row.

| row | W | K | median ms | min–max ms | range % | IQR % | SD % |
|---|---|---|---|---|---|---|---|
| J-A disarmed | 1 | 6 | 19.6707 | 19.0793–19.8351 | 3.84 | 0.50 | 1.37 |
| J-A disarmed | 2 | 6 | 13.7107 | 13.4448–13.8193 | 2.73 | 0.87 | 0.97 |
| J-A disarmed | 4 | 6 | 10.7013 | 10.6533–10.8692 | 2.02 | 1.14 | 0.82 |
| J-A disarmed | 8 | 6 | 9.1620 | 9.1025–9.2497 | 1.61 | 0.17 | 0.52 |
| J-A disarmed | 16 | 6 | 9.1721 | 9.1359–9.2100 | 0.81 | 0.52 | 0.33 |
| J-A armed | 1 | 6 | 19.6914 | 19.6686–19.7284 | 0.30 | 0.11 | 0.11 |
| J-A armed | 8 | 6 | 9.2295 | 9.1555–9.2762 | 1.31 | 0.94 | 0.57 |
| J-C canary | 1 | 6 | 20.6888 | 20.6093–20.8148 | 0.99 | 0.50 | 0.38 |
| J-C canary | 8 | 6 | 9.6737 | 9.6221–9.7790 | 1.62 | 1.03 | 0.68 |
| J-B | 1 | 6 | 14.2065 | 14.0497–14.5792 | 3.73 | 1.03 | 1.33 |
| J-B | 8 | 6 | 11.1640 | 11.0913–11.2992 | 1.86 | 0.87 | 0.70 |
| J-P1 | 1 | 6 | 19.8089 | 19.7604–20.1045 | 1.74 | 1.21 | 0.81 |
| J-S0 | 1 | 6 | 19.7068 | 19.5950–19.7182 | 0.62 | 0.34 | 0.27 |
| L1-B (tip, J-S0 disarmed) | 1 | 6 | 19.6904 | 19.0772–19.8463 | 3.91 | 1.34 | 1.45 |
| L1-A (pre-L1, J-S0 disarmed) | 1 | 6 | 19.5575 | 19.2652–19.6834 | 2.14 | 0.46 | 0.74 |
| AA2-pre | 1 | 6 | 19.5718 | 19.5160–19.7133 | 1.01 | 0.48 | 0.39 |
| AA2-pre | 8 | 6 | 9.1342 | 9.0691–9.3143 | 2.68 | 1.06 | 0.99 |
| SHIP | 1 | 6 | 19.5633 | 19.0850–19.7432 | 3.36 | 0.30 | 1.15 |
| SHIP | 8 | 6 | 9.1137 | 9.0843–9.3819 | 3.26 | 0.32 | 1.24 |
| J-Son | 1 | 6 | 9.1742 | 9.1530–9.2320 | 0.86 | 0.41 | 0.34 |
| J-Son | 8 | 6 | 6.4220 | 6.3795–6.4898 | 1.72 | 0.92 | 0.67 |
| R | 1 | 6 | 14.2263 | 13.5424–14.4364 | 6.28 | 1.83 | 2.26 |
| R | 8 | 6 | 9.9329 | 9.7829–10.0270 | 2.46 | 0.95 | 0.88 |
| R-ref | 1 | 6 | 23.9293 | 23.8110–24.0708 | 1.09 | 0.15 | 0.35 |
| R-S | 1 | 6 | 6.0485 | 6.0110–6.0849 | 1.22 | 0.80 | 0.50 |
| R-S | 8 | 6 | 6.1226 | 6.0224–6.3496 | 5.34 | 1.32 | 1.86 |
| S16 | 1 | 6 | 0.08142 | 0.08003–0.08186 | 2.24 | 1.38 | 0.95 |
| S16 | 8 | 6 | 0.07936 | 0.07843–0.08061 | 2.75 | 1.07 | 0.98 |
| Jolt v5.3.0 | 1 | 6 | 15.7225 | 14.9870–16.0878 | 7.00 | 1.69 | 2.38 |
| Jolt v5.3.0 | 2 | 6 | 8.7938 | 8.5143–9.0047 | 5.58 | 0.84 | 1.81 |
| Jolt v5.3.0 | 4 | 6 | 5.2286 | 5.1390–5.3735 | 4.48 | 2.00 | 1.66 |
| Jolt v5.3.0 | 8 | 6 | 3.5328 | 3.4523–3.5830 | 3.70 | 1.46 | 1.35 |
| Jolt v5.3.0 | 16 | 6 | 3.2094 | 3.1891–3.2769 | 2.74 | 0.81 | 0.99 |
| Jolt v5.6.0 | 1 | 6 | 9.6497 | 9.5454–10.0418 | 5.14 | 1.03 | 1.84 |
| Jolt v5.6.0 | 2 | 6 | 5.7077 | 5.6660–5.7853 | 2.09 | 0.73 | 0.74 |
| Jolt v5.6.0 | 4 | 6 | 3.5574 | 3.5399–3.6391 | 2.79 | 1.73 | 1.20 |
| Jolt v5.6.0 | 8 | 6 | 2.4932 | 2.4672–2.5886 | 4.87 | 2.59 | 1.97 |
| Jolt v5.6.0 | 16 | 6 | 2.4467 | 2.3714–2.4799 | 4.43 | 2.95 | 1.89 |
| Jolt -no_pair_cache | 8 | 6 | 5.0850 | 5.0014–5.2683 | 5.25 | 0.62 | 1.76 |
| Jolt -allow_sleep | 1 | 6 | 1.8941 | 1.8657–1.9734 | 5.69 | 2.36 | 2.12 |
| Jolt -allow_sleep | 8 | 6 | 0.41348 | 0.40867–0.43118 | 5.44 | 2.63 | 2.18 |
| Jolt Release -p (profiled) | 1 | 6 | 19.4990 | 19.3487–20.3324 | 5.04 | 1.50 | 1.93 |
| Jolt Release -p (profiled) | 8 | 6 | 4.0516 | 3.9948–4.1605 | 4.09 | 1.45 | 1.45 |

**Pooled per-process spread** (each process's deviation from its cell median, pooled per engine and W; SD):
- boyko: 0.96 % (W=1, 90 processes), 0.89 % (W2), 0.75 % (W4), 0.92 % (W8, 60), 0.31 % (W16).
- Jolt: 1.96 % (W1, 24), 1.28 % (W2), 1.33 % (W4), 1.68 % (W8, 30), 1.44 % (W16).
- For comparison with the diagnostic: boyko W=2's range is 2.73 % (1.92 % there, 5.2 % in P0), W=8's is 1.61 % (1.63 %).

**min–max at K = 6 is decided by one process.** J-A disarmed W=1's 3.84 % range is one process at 19.079 ms (−3.0 %); the other five sit within 0.9 %. L1-B (−3.1 %) and SHIP W=1 (−2.5 %) have the same kind of single fast process. Those three processes ran at 06:53:48, 06:59:59 and 07:38:53, so they do not cluster in time.

## Comparisons under the claim rule

| comparison (B against A) | B/A | effect % | 2× range bar | 2× IQR bar | 2× SE bar | claimed (range / IQR / SE) |
|---|---|---|---|---|---|---|
| boyko/Jolt v5.3.0 @W1 (J-A-d1 vs JOLT-T) | 1.2511 | +25.11 | 15.97 | 3.52 | 2.81 | yes / yes / yes |
| boyko/Jolt v5.6.0 @W1 (J-A-d1 vs JOLT56-T) | 2.0385 | +103.85 | 12.84 | 2.29 | 2.34 | yes / yes / yes |
| Jolt v5.6.0 vs v5.3.0 @W1 | 0.6137 | -38.63 | 17.38 | 3.95 | 3.08 | yes / yes / yes |
| boyko/Jolt v5.3.0 @W2 (J-A-d1 vs JOLT-T) | 1.5591 | +55.91 | 12.42 | 2.41 | 2.10 | yes / yes / yes |
| boyko/Jolt v5.6.0 @W2 (J-A-d1 vs JOLT56-T) | 2.4021 | +140.21 | 6.88 | 2.27 | 1.25 | yes / yes / yes |
| Jolt v5.6.0 vs v5.3.0 @W2 | 0.6491 | -35.09 | 11.91 | 2.23 | 2.00 | yes / yes / yes |
| boyko/Jolt v5.3.0 @W4 (J-A-d1 vs JOLT-T) | 2.0467 | +104.67 | 9.83 | 4.61 | 1.89 | yes / yes / yes |
| boyko/Jolt v5.6.0 @W4 (J-A-d1 vs JOLT56-T) | 3.0082 | +200.82 | 6.88 | 4.15 | 1.49 | yes / yes / yes |
| Jolt v5.6.0 vs v5.3.0 @W4 | 0.6804 | -31.96 | 10.56 | 5.29 | 2.09 | yes / yes / yes |
| boyko/Jolt v5.3.0 @W8 (J-A-d1 vs JOLT-T) | 2.5934 | +159.34 | 8.07 | 2.93 | 1.48 | yes / yes / yes |
| boyko/Jolt v5.6.0 @W8 (J-A-d1 vs JOLT56-T) | 3.6748 | +267.48 | 10.25 | 5.18 | 2.08 | yes / yes / yes |
| Jolt v5.6.0 vs v5.3.0 @W8 | 0.7057 | -29.43 | 12.23 | 5.94 | 2.44 | yes / yes / yes |
| boyko/Jolt v5.3.0 @W16 (J-A-d1 vs JOLT-T) | 2.8579 | +185.79 | 5.71 | 1.93 | 1.07 | yes / yes / yes |
| boyko/Jolt v5.6.0 @W16 (J-A-d1 vs JOLT56-T) | 3.7488 | +274.88 | 9.01 | 5.99 | 1.96 | yes / yes / yes |
| Jolt v5.6.0 vs v5.3.0 @W16 | 0.7623 | -23.77 | 10.42 | 6.12 | 2.18 | yes / yes / yes |
| A/A1 armed vs disarmed @W1 (J-A-a vs J-A-d1) | 1.0011 | +0.11 | 7.71 | 1.03 | 1.41 | no / no / no |
| A/A2 tip vs pre-instrument @W1 (J-A-d1 vs AA2-pre) | 1.0051 | +0.51 | 7.94 | 1.39 | 1.46 | no / no / no |
| shipping vs dev (disarmed) @W1 (SHIP vs J-A-d1) | 0.9945 | -0.55 | 10.21 | 1.17 | 1.83 | no / no / no |
| canary @W1 (J-C vs J-A-a) | 1.0506 | +5.06 | 2.08 | 1.02 | 0.41 | yes / yes / yes |
| cfg-B vs cfg-A, armed @W1 (J-B vs J-A-a) | 0.7215 | -27.85 | 7.48 | 2.06 | 1.36 | yes / yes / yes |
| Jolt -allow_sleep vs timed @W1 | 0.1205 | -87.95 | 18.04 | 5.80 | 3.26 | yes / yes / yes |
| A/A1 armed vs disarmed @W8 (J-A-a vs J-A-d1) | 1.0074 | +0.74 | 4.14 | 1.91 | 0.79 | no / no / no |
| A/A2 tip vs pre-instrument @W8 (J-A-d1 vs AA2-pre) | 1.0030 | +0.30 | 6.26 | 2.15 | 1.15 | no / no / no |
| shipping vs dev (disarmed) @W8 (SHIP vs J-A-d1) | 0.9947 | -0.53 | 7.28 | 0.73 | 1.37 | no / no / no |
| canary @W8 (J-C vs J-A-a) | 1.0481 | +4.81 | 4.17 | 2.79 | 0.91 | yes / yes / yes |
| cfg-B vs cfg-A, armed @W8 (J-B vs J-A-a) | 1.2096 | +20.96 | 4.55 | 2.56 | 0.93 | yes / yes / yes |
| Jolt -allow_sleep vs timed @W8 | 0.1170 | -88.30 | 13.16 | 6.01 | 2.63 | yes / yes / yes |
| L1 gate: tip vs pre-L1, J-S0 disarmed @W1 (L1-B vs L1-A1) | 1.0068 | +0.68 | 8.91 | 2.83 | 1.67 | no / no / no |
| arming on J-S0 @W1 (J-S0 armed vs L1-B disarmed tip) | 1.0008 | +0.08 | 7.91 | 2.76 | 1.51 | no / no / no |
| sleep bookkeeping, threshold 0, armed @W1 (J-S0 vs J-A-a) | 1.0008 | +0.08 | 1.39 | 0.72 | 0.29 | no / no / no |
| J-P1 vs J-A-a @W1 (parallel_solve at one worker) | 1.0060 | +0.60 | 3.53 | 2.42 | 0.84 | no / no / no |
| D1 price @W1 (R-ref vs R) | 1.6821 | +68.21 | 12.76 | 3.66 | 2.34 | yes / yes / yes |
| Delta_J @W8 (JOLT-NPC vs JOLT-T) | 1.4394 | +43.94 | 12.84 | 3.17 | 2.27 | yes / yes / yes |

**Scaling, T(1)/T(W):**
- boyko J-A disarmed: 1.435, 1.838, 2.147, 2.145 (W = 2, 4, 8, 16).
- Jolt v5.3.0: 1.788, 3.007, 4.450, 4.899.
- Jolt v5.6.0: 1.691, 2.713, 3.870, 3.944.
- boyko R (serial W=1, `--parallel-solve` W=8): 1.432. S16: 1.026. J-B: 1.273.
- Δ_J = T(`-no_pair_cache`) − T at W=8: **+1.552 ms** (+43.9 %, claimed).

**What the claimed rows say**
- **cfg-B against cfg-A** (armed; Grid + `simd_solve`): −27.9 % at W=1, +21.0 % at W=8.
  - From the armed span medians, the Grid broadphase adds +3.07 ms (W=1) and +3.09 ms (W=8) against AllPairs.
  - The solve span shrinks by 8.56 ms at W=1 but only 1.16 ms at W=8. `simd_solve` is cfg-B's other difference.
- **D1's price** (R-ref against R, W=1): +68.2 % (23.93 against 14.23 ms).

**Not claimed under any reading:**
- A/A1, A/A2 and shipping against dev at W=1 and W=8;
- L1's gate;
- arming on J-S0;
- the threshold-0 sleep bookkeeping (J-S0 against J-A-a, +0.08 %, bar 1.39 %);
- `parallel_solve` at one worker (J-P1, +0.60 %).

## The canary (J-C against J-A-a, both armed)

- **Spin amount:** each J-C derived it from the most recent valid J-A-a at the same W: `--canary-frac 0.05 --canary-ref-ns <that mean>`. The six spins were 983,432–984,980 ns at W=1 and 457,775–463,809 ns at W=8.
- **Expected rise:** 5.000 % (W=1) and 4.983 % (W=8) of median(J-A-a).
- **Measured rise:** +5.065 % and +4.813 %, which is +0.065 and −0.170 points off the injected amount. The rise matches the injection within every bar.
- **Span check:** the canary system's own span reads the injected ns to +0.034 % (W=1) and +0.051 % (W=8) median, well inside ±5 %.
- **Seen:** yes at both W, under range, IQR and SE (table in the Verdict).

## Headline sub-windows (H1) and time per manifold (H9)

Medians over K of the sub-window means, in ms:

| W | boyko [0,100) | boyko [100,500) | v5.3.0 [0,100) | v5.3.0 [100,500) | v5.6.0 [0,100) | v5.6.0 [100,500) | ratio v5.3.0 early / late | ratio v5.6.0 early / late |
|---|---|---|---|---|---|---|---|---|
| 1 | 19.337 | 19.743 | 14.936 | 15.923 | 10.170 | 9.502 | 1.295 / 1.240 | 1.901 / 2.078 |
| 2 | 13.552 | 13.746 | 8.272 | 8.934 | 5.766 | 5.694 | 1.638 / 1.539 | 2.350 / 2.414 |
| 4 | 10.517 | 10.747 | 4.873 | 5.308 | 3.452 | 3.584 | 2.158 / 2.025 | 3.046 / 2.999 |
| 8 | 9.034 | 9.194 | 3.112 | 3.638 | 2.348 | 2.526 | 2.903 / 2.527 | 3.848 / 3.640 |
| 16 | 9.088 | 9.191 | 2.849 | 3.307 | 2.188 | 2.494 | 3.190 / 2.780 | 4.153 / 3.685 |

H9 over [100, 500):
- boyko has 4,519.3 manifolds per step (the median over its 6 processes; identical at every W).
- Jolt has 8,456 (P0's `-receipt` count). It was not re-run here: the count is deterministic, and Jolt's hash is identical.

| W | boyko µs per manifold per step | Jolt v5.3.0 µs per manifold per step | ratio |
|---|---|---|---|
| 1 | 4.369 | 1.883 | 2.32 |
| 2 | 3.042 | 1.056 | 2.88 |
| 4 | 2.378 | 0.628 | 3.79 |
| 8 | 2.034 | 0.430 | 4.73 |
| 16 | 2.034 | 0.391 | 5.20 |

## Other rows

- **Sleeping (O5, the tails after everything is asleep).** boyko is all asleep from step 264 and Jolt from frame 127, in every process.

  | tail [264, 1000) | boyko (J-Son) | Jolt (`-allow_sleep`) | ratio |
  |---|---|---|---|
  | W=1 | 5.334 ms (range 1.59 %) | 0.00217 ms | ≈ 2,450× |
  | W=8 | 5.407 ms (range 1.12 %) | 0.00228 ms | ≈ 2,370× |

  Whole-window means: J-Son 9.174 / 6.422 ms; `-allow_sleep` 1.894 / 0.413 ms.
- **R-S (the sleeping floor F), [300, 800):** 6.049 ms (W=1) and 6.123 ms (W=8). The first frozen step is 248 in all 12 processes, so the row is not void. At W=8 it still runs without `--parallel-solve`, as in the driver's table; the decision the P0 reduction raised is still open.
- **S16:** 81.4 µs (W=1) and 79.4 µs (W=8).
- **The profile identity** (the driver copy's own reduction over the 258 used processes, one synthetic pass per round; medians over K, not individually gated):

  | term | value |
  |---|---|
  | f | 0.376 |
  | f_fit | 0.393 |
  | S(1) | 7.398 ms |
  | P(1) | 12.205 ms |
  | ω₁ | 855 ns |
  | waves per step | 109.32 |
  | E(8) | 0.681 |
  | L(8) | 0.715 ms |
  | g(1), g(8) | 89 µs, 35 µs |
  | u | 1.4 µs |
  | r(1), r(8) | 4.3 µs, 4.7 µs |
  | identity residual | −6.6 µs (W=1), +9.0 µs (W=8) |
  | L6 fork, ω₁ · waves / L(8) | 0.131 |

  Closure: no stop.
- **The driver's band-based verdicts are not used.** Its A/A1, A/A2 and canary lines rest on the old band rules (B(W) with a 0.5 % floor), which the ruling replaced. They are in `raw/window/driver_reduce/reduction.txt`.

## Jolt `-p` stage shares (Release build, profiled)

**How these were reduced**
- **Source:** Jolt's own chart dump. Each HTML is **one step**: frames 0, 100, 200, 300 and 400; frame 0 is the first step and is dropped.
- **Stages:** the stage is the job-level scope, meaning a child of `Executing Jobs` on a worker or of `Execute Jobs` on the main thread, which helps inside `WaitForJobs`.
- **Wall coverage** is the union of a stage's intervals over all threads, divided by the `PhysicsSystem::Update` span. **Use this column.**
- **Thread-time share** includes time a worker spins inside a job scope waiting for work. That is why SolveVelocityConstraints takes 21.5 thread-ms of a 4.05-ms step at W=8.
- **Statistic:** per process, the median over frames 100–400; then the median over K = 6. The min–max in brackets is over the 6 processes.

Jolt -p, W=1: K=6, frames [100, 200, 300, 400] per process; profiled T 19.499 ms; Update span 19.656 ms; summed thread time in scopes 19.652 ms

| stage | wall coverage % of the step | thread-time share % (min–max over K) | thread-time ms |
|---|---|---|---|
| SolveVelocityConstraints | 63.54 | 63.56 (62.38–64.25) | 12.537 |
| FindCollisions | 29.45 | 29.46 (28.67–29.88) | 5.802 |
| SolvePositionConstraints | 6.29 | 6.29 (5.78–7.96) | 1.222 |
| UpdateBroadPhasePrepare | 0.34 | 0.34 (0.27–0.39) | 0.067 |
| ApplyGravity | 0.12 | 0.12 (0.11–0.13) | 0.023 |
| FinalizeIslands | 0.06 | 0.06 (0.06–0.07) | 0.012 |
| IntegrateVelocity | 0.05 | 0.05 (0.05–0.06) | 0.010 |
| main serial (outside WaitForJobs) | 0.00 | 0.04 (0.04–0.06) | 0.009 |

Jolt -p, W=8: K=6, frames [100, 200, 300, 400] per process; profiled T 4.052 ms; Update span 4.047 ms; summed thread time in scopes 31.578 ms

| stage | wall coverage % of the step | thread-time share % (min–max over K) | thread-time ms |
|---|---|---|---|
| SolveVelocityConstraints | 66.19 | 67.94 (67.52–68.29) | 21.474 |
| FindCollisions | 25.48 | 25.93 (25.08–26.90) | 8.235 |
| SolvePositionConstraints | 6.93 | 5.32 (5.08–6.17) | 1.664 |
| UpdateBroadPhasePrepare | 1.58 | 0.20 (0.19–0.21) | 0.066 |
| ApplyGravity | 0.23 | 0.10 (0.10–0.11) | 0.033 |
| IntegrateVelocity | 0.12 | 0.07 (0.07–0.09) | 0.023 |
| main serial (outside WaitForJobs) | 0.00 | 0.06 (0.06–0.07) | 0.021 |
| FinalizeIslands | 0.41 | 0.05 (0.05–0.07) | 0.017 |
| ContactRemovedCallbacks | 0.16 | 0.02 (0.02–0.03) | 0.007 |
| job-system overhead | 0.11 | 0.02 (0.02–0.02) | 0.006 |
| BodySetIslandIndex | 0.13 | 0.02 (0.02–0.02) | 0.005 |

**Caveats**
- **Profiling perturbs the step.** The profiled Release step is 24 % slower than the timed Distribution row at W=1 (19.50 against 15.72 ms) and 15 % slower at W=8 (4.05 against 3.53 ms).
- **The sample-cap warning did not truncate any dumped frame.** All 6 W=1 processes printed `ProfileMeasurement: Too many samples, some data will be lost!`. No dumped frame reached the 65,536-sample cap (at most 48,324 per thread), so the loss happened in frames that were not dumped.

## Broadphase crossover (`broadphase_bench.exe --bench --noplot`, one process)

`CRITERION_HOME` pointed into the output directory. Without it, criterion runs `cargo metadata` (a build process) and writes to `target/` under the working directory.

| benchmark | median | 95 % CI of the median |
|---|---|---|
| broadphase/all_pairs/100 | 5.50 µs | 5.49 µs – 5.50 µs |
| broadphase/all_pairs/1000 | 597.75 µs | 597.49 µs – 598.01 µs |
| broadphase/all_pairs/10000 | 70.220 ms | 70.169 ms – 70.293 ms |
| broadphase/grid/100 | 44.68 µs | 44.65 µs – 44.71 µs |
| broadphase/grid/1000 | 661.82 µs | 661.43 µs – 662.12 µs |
| broadphase/grid/10000 | 8.015 ms | 8.013 ms – 8.017 ms |
| broadphase_disparity/all_pairs/1000 | 618.31 µs | 617.85 µs – 618.57 µs |
| broadphase_disparity/all_pairs/10000 | 71.747 ms | 71.570 ms – 71.867 ms |
| broadphase_disparity/grid/1000 | 2.416 ms | 2.416 ms – 2.417 ms |
| broadphase_disparity/grid/10000 | 15.802 ms | 15.798 ms – 15.807 ms |
| broadphase_parallel/serial_o2/1000 | 662.34 µs | 661.75 µs – 663.06 µs |
| broadphase_parallel/serial_o2/10000 | 8.026 ms | 8.021 ms – 8.033 ms |
| broadphase_parallel/serial_o2/100000 | 88.364 ms | 88.295 ms – 88.489 ms |
| broadphase_parallel/w1/1000 | 661.28 µs | 660.36 µs – 662.59 µs |
| broadphase_parallel/w1/10000 | 7.990 ms | 7.981 ms – 8.006 ms |
| broadphase_parallel/w1/100000 | 88.222 ms | 88.137 ms – 88.529 ms |
| broadphase_parallel/w2/1000 | 661.86 µs | 660.79 µs – 663.62 µs |
| broadphase_parallel/w2/10000 | 6.107 ms | 6.099 ms – 6.126 ms |
| broadphase_parallel/w2/100000 | 67.677 ms | 67.559 ms – 67.752 ms |
| broadphase_parallel/w4/1000 | 660.29 µs | 659.76 µs – 662.32 µs |
| broadphase_parallel/w4/10000 | 4.330 ms | 4.323 ms – 4.343 ms |
| broadphase_parallel/w4/100000 | 45.901 ms | 45.592 ms – 46.026 ms |

**Crossover.** Uniform lattice, t_grid / t_all_pairs:

| n | t_grid / t_all_pairs |
|---|---|
| 100 | 8.13 |
| 1,000 | 1.107 |
| 10,000 | 0.114 |

- The ratio changes sign between 1,000 and 10,000. Log-log interpolation gives **n\* ≈ 1,109**.
- The size-disparity scene (ratio 3.91 at 1,000, 0.220 at 10,000) gives **n\* ≈ 2,978**.
- Against `GRID_LO` = 96 and `GRID_HI` = 192 (`broadphase_policy.rs:79,88`, marked `[ESTIMATE:needs-calibration]`): **both edges sit about 6–15× below the measured crossover.**

**Limits**
- The bench's sizes are fixed at 100/1k/10k, and nothing could be rebuilt, so n\* is interpolated, not bracketed tighter.
- It is one process (K = 1), so the 1–3 % per-process spread applies. That is small against the 10.7 % gap at n = 1,000.

**Observation outside the brief.** The parallel group's 4-worker speedup at 100k bodies is **1.92×**:
- 88.22 → 45.90 ms against w1, and 88.36 → 45.90 ms against `serial_o2`;
- w2 gives 1.30×.

The bench's own doc names ≥ 2.8× as "O3 Gate 7". One process; flagged, not ruled on.

## Structural checks (all green)

- **Exits:** 0 non-zero in 262 processes. There were 0 void steps, `drops_total` was 0 everywhere, and disarmed ring traffic was 0.
- **Determinism:** 7 boyko simulation groups, one pose each (6 distinct poses):
  - J family = `0x32d5e235342b4143`. This covers two groups: J-A, J-B, J-C, J-P1, AA2-pre and SHIP with sleeping off; and J-S0, L1-A and L1-B with sleeping at threshold 0, which gives the same pose;
  - J-Son `0xcc2a5400c66eecce`;
  - R `0x87e561d20589d4a5` (serial W=1 equals parallel W=8);
  - R-ref `0x67463b7ea88686d2`;
  - R-S `0x2a2b7926a48aab00`;
  - S16 `0x8877dbb1192e9b92`.
- **H7:** 12 of 12 `match`.
- **Jolt:** 15 groups, 0 differ. v5.3.0 Distribution and Release `0xee15b89965ec747`; v5.6.0 `0xb8522b4e3fc62cfe`; `-no_pair_cache` `0xf7fd640e276d8281`; `-allow_sleep` `0x6b2368f4c588aeae`.
- **Closure:** no stop. Waves are identical across W. The solve ran on the dispatcher on 0 steps in every armed row. Jolt printed threads = W in every run.
- **Build receipts (H6):** head `dbd85977` (0 dirty paths), host msvc; S5, KE16, B1 and L1 are ancestors. Every boyko row reports `target_env` msvc and profile `dev`, except SHIP (`shipping`).

## Witnesses (exploratory, not gates)

- **The 1-Hz clock witness does not explain the per-process spread consistently.** It correlates each process's deviation from its cell median with the `% Processor Performance` of the CPUs it kept busy.
  - Overall, r = +0.02 (n = 246); against the machine total, r = −0.17.
  - boyko W=1: r = −0.44 (n = 90). The sign is the expected one, and it explains about 19 % of that cell group's variance.
  - Jolt W=1: r = +0.63 (n = 18), the wrong sign.
  - The other groups have |r| ≤ 0.2, except boyko W=2 (−0.83, n = 6).
- **Neighbouring processes share little of their deviation:** the lag-1 autocorrelation in run order is 0.15. The median machine performance level was the same in both passes (131.2 and 131.2).
- So the placement result of the diagnostic (placement is not the cause) is joined by "the sampled clock is not the main cause either". The per-process speed draw is still unexplained. The candidates the diagnostic left untested (address layout, physical page placement) remain.

## Deviations from the brief (all deliberate)

1. **J-A disarmed ran as one row** (the driver's `J-A-d1`, K = 6). `J-A-d2` existed only for the A/A0 pair, which the ruling replaced with the per-process spread.
2. **L1:** besides J-S0 (armed) and pre-L1, I ran `L1-B` (the tip, J-S0 disarmed). §10 defines L1's gate as disarmed pre-L1 against the disarmed tip. Comparing pre-L1 with the armed J-S0 would have mixed in the arming cost. For the record, the arming cost on J-S0 read +0.08 %, not claimed.
3. **One untimed warm-up per pass,** as in the diagnostic.
4. **Canary reference:** the most recent valid J-A-a at the same W, because a pass now holds 3 J-A-a. In pass 1's reversed order, J-C precedes J-A-a in a round, so its reference comes from the previous round.
5. **Witnesses added:** the 1-Hz PDH sampler thread in the parent, and the Toolhelp "others during" walk (before launch, after exit). Neither runs code in the measured process.
6. **Broadphase run** with `--noplot` and `CRITERION_HOME` (see that section).
7. **Not run:** JOLT-RCPT (not in the brief; H9 reuses P0's deterministic count).

## State

- `D:/wt/joltab` is at `dbd859771b4d7aee5f560c7d3cbceccd0f1af179`, clean (checked with `git --no-optional-locks`). There were no git writes and no `__pycache__`.
- Nothing was compiled or deleted.
- D: holds 2.6 GB free, unchanged. Outputs (112 MB) are on C:.

## Files

All under `docs/measurements/2026-09-19-physics-p0/p0b/`:

**Scripts**
- `window_run.py` — the runner.
- `window_lib/pdhperf.py` — the clock witness.
- `analyze_window.py`, `analyze_window_extra.py`, `render_tables.py` — the reductions (pure reading).
- `window_report_template.md` — this report before its tables were filled in.

**Raw output, in `raw/window/`**
- `runs.jsonl` — 262 records: 258 used, 2 warm-ups, 2 contaminated originals.
- `window_log.txt`, `wait_log.txt`, `window_state.json`, `WINDOW_DONE` (`complete`), `stdout.log`.
- `pass-00/`, `pass-01/` — per-process `run.csv`, `pose.bin`, stdout and stderr; Jolt `per_frame_*.csv`; JOLT-P `profile_chart_*.html`.
- `broadphase/` — criterion `estimates.json` per benchmark, `stdout.txt`, `stderr.txt`, `broadphase_run.json`.
- `analysis.json`, `analysis_extra.json`, `tables.md`.
- `driver_reduce/` — the driver copy's structural reduction.

**Rehearsal**
- `test/window/` — an untimed dry-step rehearsal of the runner. Not a measurement.
