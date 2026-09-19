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

{{CELLS}}

**Pooled per-process spread** (each process's deviation from its cell median, pooled per engine and W; SD):
- boyko: 0.96 % (W=1, 90 processes), 0.89 % (W2), 0.75 % (W4), 0.92 % (W8, 60), 0.31 % (W16).
- Jolt: 1.96 % (W1, 24), 1.28 % (W2), 1.33 % (W4), 1.68 % (W8, 30), 1.44 % (W16).
- For comparison with the diagnostic: boyko W=2's range is 2.73 % (1.92 % there, 5.2 % in P0), W=8's is 1.61 % (1.63 %).

**min–max at K = 6 is decided by one process.** J-A disarmed W=1's 3.84 % range is one process at 19.079 ms (−3.0 %); the other five sit within 0.9 %. L1-B (−3.1 %) and SHIP W=1 (−2.5 %) have the same kind of single fast process. Those three processes ran at 06:53:48, 06:59:59 and 07:38:53, so they do not cluster in time.

## Comparisons under the claim rule

{{COMPARISONS}}

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

{{H1}}

H9 over [100, 500):
- boyko has 4,519.3 manifolds per step (the median over its 6 processes; identical at every W).
- Jolt has 8,456 (P0's `-receipt` count). It was not re-run here: the count is deterministic, and Jolt's hash is identical.

{{H9}}

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

{{JOLTP}}

**Caveats**
- **Profiling perturbs the step.** The profiled Release step is 24 % slower than the timed Distribution row at W=1 (19.50 against 15.72 ms) and 15 % slower at W=8 (4.05 against 3.53 ms).
- **The sample-cap warning did not truncate any dumped frame.** All 6 W=1 processes printed `ProfileMeasurement: Too many samples, some data will be lost!`. No dumped frame reached the 65,536-sample cap (at most 48,324 per thread), so the loss happened in frames that were not dumped.

## Broadphase crossover (`broadphase_bench.exe --bench --noplot`, one process)

`CRITERION_HOME` pointed into the output directory. Without it, criterion runs `cargo metadata` (a build process) and writes to `target/` under the working directory.

{{BROADPHASE}}

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
- **Determinism:** 7 boyko simulation groups, one pose each:
  - J family (J-A, J-B, J-C, J-P1, J-S0, L1-A, L1-B, AA2-pre, SHIP) = `0x32d5e235342b4143`;
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
