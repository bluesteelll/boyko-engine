# P0b: diagnosis of the per-process spread under three CPU-placement policies (tester, 2026-09-19)

This is a diagnostic, not a measurement window. No number here is a P0 result.

## Recommendation (one line)

**Use P-none (no affinity) for both engines. Pinning is not the fix. The spread comes from each process, not from where its threads run, so the ruling's K-process median is the right tool. The orchestrator still has to settle which "spread" the claim rule means (section 6).**

## Verdict

1. **Thread placement does not cause the spread.**
   - No policy changed the process-to-process spread significantly, in any of the 4 (engine, W) cells. The largest variance ratio was 3.3, against an F(5,5) 5 % bound of 7.15.
   - P-phys's spread was not smaller than P-none's. Its point estimate is larger for Jolt at both W and for boyko at W=8, and smaller only for boyko at W=2 (range 1.25 % against 1.92 %; SD 0.58 % against 0.69 %).
2. **Under P-none, Windows already runs one thread per physical core, and in the same place every time.**
   - Each engine's busy threads sit on distinct physical cores.
   - Every placement that the ≥ 50 %-busy CPUs pin down is the same in all 6 of its processes: boyko W=2 on CPUs {10, 12}, Jolt W=2 on {2, 10}, Jolt W=8 on {1, 3, 5, 7, 8, 10, 12, 14} (cores 0–7, one CPU each).
   - Processes with identical placement still spread by 1.9–4.2 %.
   - boyko W=8 is the exception: its threads move around (3 CPUs' worth of work, with only CPU 10 at ≥ 50 % busy). Pinning it to 8 CPUs did not narrow its spread either (1.87 % against 1.63 %).
3. **No policy changed the level beyond resolution, so pinning would not change what is measured.**
   - P-phys against P-none: boyko +0.02 % (W=2) and +0.44 % (W=8); Jolt +0.85 % and +1.32 %. The 2 × combined-range bars are 4.6 %, 5.0 %, 13.7 % and 13.3 %.
   - P-phys moves boyko W=2 from the cores Windows prefers ({10, 12}) to cores 0 and 1 ({0, 2}), which include the interrupt-heavy CPU 0. That costs nothing measurable at the 1 % level.
4. **The control passes: P-full equals P-none within resolution.** Levels differ by −0.66 %, +0.14 %, −0.90 % and −2.00 %; all four are under 2 × the combined range. One side effect: an explicit 0xFFFF mask moved boyko W=2 from {10, 12} to {8, 10} in 5 of 6 processes, with no measurable effect.
5. **Each process draws its own speed, for its whole run. Time drift does not explain it.** See section 5.

## 1. Setup

**Machine and state**
- Ryzen 9 5900HS, 8 cores / 16 logical CPUs.
- High performance power plan, on AC, battery at 100 %, before and after.
- Process count: 236 before, 245 after.

**SMT map**, from `GetLogicalProcessorInformationEx(RelationProcessorCore)` (`topology.py` → `topology.json`):
- Core k = logical CPUs {2k, 2k+1}, for k = 0..7.
- Every core has SMT and EfficiencyClass 0.
- L2 is 512 KiB per core, shared only by the two siblings.
- One 16 MiB L3 serves all 16 CPUs (a single CCX), so P-phys can only change SMT sharing and core identity.

**Policies**

| policy | W=2 mask | W=8 mask | meaning |
|---|---|---|---|
| P-none | none (no call) | none (no call) | today's launch |
| P-phys | 0x5 = CPUs {0, 2} | 0x5555 = CPUs {0, 2, …, 14} | one CPU per physical core, first W cores |
| P-full | 0xFFFF | 0xFFFF | control; must equal P-none |

**Launch path** (`diag.py`), identical for all three policies:
1. `CreateProcess` with `CREATE_SUSPENDED`.
2. `SetProcessAffinityMask`, for P-phys and P-full only.
3. `GetProcessAffinityMask` read-back, required to equal the request. It matched in all 73 attempts.
4. `NtResumeProcess`.

The child runs no instruction before its mask is set, so its pool and job threads inherit the mask. Neither binary sets thread affinity itself: `boyko_threadpool`'s `affinity` flag is a no-op stub (`thread_pool.rs:635`), and Jolt's `JobSystemThreadPool` makes no affinity call.

**Rows.** The arguments come from the driver's own builders, in `lib/driver.py`. That file is a byte-identical copy of `D:/wt/joltab/tools/physics_parity/driver.py`: same sha256 `84bc5813…`, and the git blob equals HEAD. Running a copy meant nothing was written into the tree.
- boyko = J-A-d1, disarmed: `runner_tip.exe --scene jolt --gap 0.5 --cfg a --workers W --steps 500 --window 0..500 …`.
- Jolt = JOLT-T, v5.3.0 Distribution: `-s=Pyramid -q=Discrete -f -t=W -i=500`.
- The statistic is the window mean of per-step wall time: boyko `run.csv` `wall_ns` over [0, 500), Jolt per-frame `Time (ms)` over its 500 frames.

**Order**
- 6 passes. Each pass is one warm-up (untimed boyko W=8 under P-none; excluded) followed by 12 processes, one per cell.
- W order alternates by pass.
- Policy order rotates by pass, so each policy takes each slot twice within a W.
- Which engine goes first alternates.

**Gates**
- **Binaries:** all 5 in `p0/bin` matched `SHA256SUMS` before and after the diagnostic. The two binaries used (`runner_tip.exe` `ef9325ef…` and v5.3.0 `PerformanceTest.exe` `29b23ad1…`) were re-hashed before every pass and matched `p0/manifest.json`.
- **Anti-vacuity:** every one of the 72 used processes exited 0.
  - boyko: 500 steps, pose `0x32d5e235342b4143`, 0 void steps, workers = W.
  - Jolt: hash `0xee15b89965ec747`, threads = W, 500 frames.
- **Idle rule:** `p0/wait_idle.ps1` before each of the 6 passes. All 18 polls were quiet (log in `wait_log.txt`). Each pass started after 3 consecutive quiet polls, about 2 min 10 s.
- **Receipts:** 146, median 0.93 % busy, one above 5 %.
  - The outlier: boyko W=2 P-full in pass 5 had an after-receipt of **39.11 %**. `OneDrive.Sync.Service.exe` used 30.6 % of the machine.
  - That process read 14.079 ms, against a median step of 13.629 ms, which is a transient landing in the run. It was re-run immediately and clean at 13.655 ms, and the re-run is the value used. The original is kept in `runs.jsonl`.
  - No process was excluded.

**Timing**
- Diagnostic: 06:13:28 to 06:35:37 (+03:00).
- Timed runs: about 9 minutes. Idle polls: about 13 minutes.

## 2. Results per policy (window mean, 6 processes per cell)

| W | engine | policy | median ms | min | max | range % | IQR % | SD % |
|---|---|---|---|---|---|---|---|---|
| 2 | boyko | P-none | 13.7664 | 13.6909 | 13.9558 | 1.92 | 0.63 | 0.69 |
| 2 | boyko | P-phys | 13.7686 | 13.7325 | 13.9040 | 1.25 | 0.92 | 0.58 |
| 2 | boyko | P-full | 13.6751 | 13.4072 | 13.9175 | 3.73 | 0.35 | 1.19 |
| 2 | jolt | P-none | 8.6571 | 8.5852 | 8.9474 | 4.18 | 2.37 | 1.73 |
| 2 | jolt | P-phys | 8.7308 | 8.6245 | 9.0999 | 5.45 | 2.55 | 2.11 |
| 2 | jolt | P-full | 8.5792 | 8.5347 | 8.8913 | 4.16 | 1.30 | 1.61 |
| 8 | boyko | P-none | 9.1318 | 9.0850 | 9.2338 | 1.63 | 0.51 | 0.59 |
| 8 | boyko | P-phys | 9.1723 | 9.1224 | 9.2940 | 1.87 | 0.52 | 0.66 |
| 8 | boyko | P-full | 9.1442 | 9.0684 | 9.2050 | 1.49 | 0.20 | 0.48 |
| 8 | jolt | P-none | 3.5299 | 3.4470 | 3.5799 | 3.76 | 1.36 | 1.33 |
| 8 | jolt | P-phys | 3.5765 | 3.4444 | 3.6406 | 5.48 | 1.86 | 1.95 |
| 8 | jolt | P-full | 3.4593 | 3.4331 | 3.6621 | 6.62 | 0.74 | 2.50 |

IQR uses inclusive quartiles. The per-process values in pass order are in `raw/analysis.md`.

**Pooled over the three policies** (18 processes per cell; placement does not matter, see §3):

| engine | W | median ms | range % | IQR % | SD % |
|---|---|---|---|---|---|
| boyko | 2 | 13.7367 | 3.99 | 0.86 | 0.92 |
| boyko | 8 | 9.1482 | 2.47 | 0.57 | 0.60 |
| jolt | 2 | 8.6576 | 6.53 | 2.67 | 1.90 |
| jolt | 8 | 3.5171 | 6.51 | 3.53 | 2.04 |

**Jolt's per-process spread is 2–3× boyko's.** Jolt's worker threads spin: CPU-seconds per wall-second is 1.81–2.00 at W=2 and 7.53–7.90 at W=8. boyko's park: 1.41–1.51 at W=2 and 2.52–2.91 at W=8.

## 3. Policy comparisons

| comparison | level effect | 2 × comb. range | 2 × comb. IQR | var ratio none/pol | F(5,5), 5 % |
|---|---|---|---|---|---|
| P-phys vs P-none, boyko W=2 | +0.02 % | 4.58 % | 2.24 % | 1.41 | ns |
| P-full vs P-none, boyko W=2 | −0.66 % | 8.40 % | 1.45 % | 0.34 | ns |
| P-phys vs P-none, boyko W=8 | +0.44 % | 4.96 % | 1.46 % | 0.79 | ns |
| P-full vs P-none, boyko W=8 | +0.14 % | 4.42 % | 1.10 % | 1.49 | ns |
| P-phys vs P-none, Jolt W=2 | +0.85 % | 13.73 % | 6.96 % | 0.66 | ns |
| P-full vs P-none, Jolt W=2 | −0.90 % | 11.80 % | 5.39 % | 1.18 | ns |
| P-phys vs P-none, Jolt W=8 | +1.32 % | 13.30 % | 4.61 % | 0.45 | ns |
| P-full vs P-none, Jolt W=8 | −2.00 % | 15.23 % | 3.10 % | 0.30 | ns |

"Combined" means the root sum of squares of the two cells' spreads, each as a % of its median.

**boyko/Jolt ratio of medians, per policy.** None of the differences exceeds the resolution:

| policy | W=2 | W=8 |
|---|---|---|
| P-none | 1.5902 | 2.5870 |
| P-phys | 1.5770 | 2.5646 |
| P-full | 1.5940 | 2.6434 |

The ratio's own resolution (RSS of the two ranges) is 4.1–6.8 %.

## 4. Placement witness (per process, per-CPU busy over the process lifetime)

The witness is `NtQuerySystemInformation(SystemProcessorPerformanceInformation)`, sampled before resume and after exit. It is machine-wide, but the machine was idle. Full lists are in `raw/analysis.md`.

| cell | CPUs ≥ 50 % busy, all 6 processes | SMT pairs with both siblings ≥ 30 % |
|---|---|---|
| P-none, boyko W=2 | {10, 12} in all 6 | 0 |
| P-none, Jolt W=2 | {2, 10} in all 6 | 0 |
| P-none, Jolt W=8 | {1, 3, 5, 7, 8, 10, 12, 14} in all 6 (cores 0–7) | 0 in 4 processes, 1 in 2 |
| P-none, boyko W=8 | {10} only; the work (3 CPUs' worth) floats | 0 |
| P-phys, W=2 (both engines) | {0, 2} in all 6 | 0 |
| P-phys, Jolt W=8 | {0, 2, 4, …, 14} in all 6 | 0 |
| P-full, boyko W=2 | {8, 10} in 5 processes, {8} in 1 | 0 |
| P-full, Jolt W=2 | {2, 8} in all 6 | 0 |
| P-full, Jolt W=8 | {1, 3, 5, 6, 8, 10, 12, 14} in all 6 | 0 in 3 processes, 1 in 3 |

This witness also proves the P-phys pin was in force: its processes ran on the pinned CPUs and nowhere else. A pin that failed silently would not have produced this.

## 5. Where the spread lives

**Two-way decomposition of log(window mean), pass × policy** (`analyze2.py`, `raw/analysis2.md`):

| engine | W | pass SD | policy SD | residual (per-process) SD | F pass (5,10) | F policy (2,10) |
|---|---|---|---|---|---|---|
| boyko | 2 | 0.00 % | 0.35 % | 0.93 % | 0.60 ns | 1.85 ns |
| boyko | 8 | 0.19 % | 0.18 % | 0.55 % | 1.36 ns | 1.67 ns |
| jolt | 2 | 1.04 % | 0.73 % | 1.47 % | 2.50 ns | 2.46 ns |
| jolt | 8 | 0.70 % | 0.65 % | 1.83 % | 1.44 ns | 1.76 ns |

No pass effect and no policy effect is significant; the residual dominates every cell. Three more checks agree:

- **Adjacent processes do not share their deviation.**
  - boyko and Jolt ran back to back in the same pass and policy. Their values correlate at r = 0.135 (W=2) and 0.113 (W=8), with n = 18; the 5 % threshold is 0.47.
  - Across all 72 processes in run order, the lag-1 autocorrelation of the residuals is 0.034.
  - A drifting machine state (boost, thermal) would show up as correlation between neighbours. There is none.
- **Pairing within a pass does not narrow anything.**
  - The per-pass paired ratios have ranges of 1.8–7.6 % for policy against policy and 3.3–6.3 % for boyko/Jolt.
  - That is no tighter than the unpaired spread, as expected when the deviations are independent per process.
- **The deviation lasts the whole run.** Across processes, the window mean and the per-process median step correlate at 0.90–0.98 (pooled per engine and W). The range of per-process medians is 3.6 % against 4.0 % of means for boyko W=2, and 5.7 % against 6.5 % for Jolt W=2. So a process that runs slow does so for most of its steps. This agrees with the P0 window's finding.

## 6. Consequences for the ruling (the orchestrator decides)

**The claim rule's "combined spread" is ambiguous, and the reading decides whether the 5 % canary can be claimed at all.**

Below is the minimum claimable effect when two rows of the same engine are compared at K = 6, using the pooled SD from §2 (normal approximations). The "obs" values use the measured P-none cell.

| cell | min-max range (K=6), pred / obs | IQR, pred / obs | SE of the median, K=6 / K=10 |
|---|---|---|---|
| boyko W=2 | **6.6 % / 5.4 %** | 3.5 % / 1.8 % | 1.33 % / 1.03 % |
| boyko W=8 | 4.3 % / 4.6 % | 2.3 % / 1.4 % | 0.87 % / 0.67 % |
| Jolt W=2 | 13.6 % / 11.8 % | 7.2 % / 6.7 % | 2.75 % / 2.13 % |
| Jolt W=8 | 14.6 % / 10.6 % | 7.8 % / 3.8 % | 2.95 % / 2.29 % |

What follows from the table:
- **Under the min-max reading, a 5 % change on boyko at W=2 cannot be claimed.** At W=8 it is marginal. Jolt-side effects under about 11–15 % cannot be claimed.
- **The min-max range also grows with K.** The expected range is 2.53σ at K=6 and 3.08σ at K=10. Adding processes makes that reading stricter, while the median it gates gets more precise, roughly as 1.25σ/√K.
- **The IQR reading does not depend on K.** Its bar is about 3.8σ.
- **Only a reading based on the standard error of the median (or a bootstrap) improves with K.**
- **Recommendation to the orchestrator:** keep reporting the min-max range and IQR, as the ruling asks, but gate the claim on the median's own uncertainty, a bootstrap or an order-statistic CI over the K processes. Then show the canary against that same gate.
- **The magnitude of the spread itself varies from session to session.** The P0 diagnostic measured a 5.2 % range for boyko W=2 over 6 processes. Today it was 1.92 % (P-none) and 3.99 % pooled over 18 processes. Size K from the pooled worst case, not from one session.
- **This diagnostic ran no canary.** J-C is an armed window row, and its visibility is a window-protocol item.

## 7. Limits (what could make this wrong)

- **Power: n = 6 per cell only detects a large spread change.** An F(5,5) difference needs about a 2.7× change in SD. So this rules out placement as the *dominant* cause of the spread; a small contribution cannot be excluded. The point estimates do not lean towards P-phys: the geometric-mean variance ratio none/phys across the 4 cells is 0.76, so P-phys has the larger variance.
- **The witness is coarse.** It is machine-wide per-CPU busy over the process lifetime, including start-up, with a 50 % threshold. It cannot show per-thread migrations inside boyko W=8's floating work.
- **Untested candidates for the per-process draw.** Nothing was compiled or modified, so neither was tested:
  - per-process address layout (ASLR image and heap bases, the kernel's separately reserved `VmReservation` ranges);
  - physical page placement (the L2 set index uses physical address bits 12–15, so page colouring differs per process);
  - a per-process boost/frequency state.

  The next cheap witness is a low-rate `% Processor Performance` sample per CPU during each process (PDH via ctypes, about 1 Hz), correlated against that process's mean. Testing ASLR would need a relinked binary, which is outside this workflow's no-compile rule.
- **W=1, W=4 and W=16 were not tested,** as the task specified.

## State

- `D:/wt/joltab` is at `dbd859771b4d7aee5f560c7d3cbceccd0f1af179` with a clean status. No git writes. No `__pycache__` was created in the tree: the driver was imported from a copy, with bytecode writing off.
- Nothing was compiled. All binaries were re-verified after the runs.
- Nothing outside `p0b/` was created or deleted.

## Files (all under `docs/measurements/2026-09-19-physics-p0/p0b/`)

- `diag_report.md`: this report.
- `diag.py`: the runner. `analyze.py`: per-cell statistics and witness. `analyze2.py`: the decomposition, pairing and pooling.
- `topology.py`, `topology.json`: the SMT and cache map.
- `lib/driver.py`: the byte-identical driver copy.
- `wait_log.txt`: all 18 idle polls.
- `raw/diag_log.txt`, `raw/diag_stdout.log`, `raw/diag_state.json`, `raw/DIAG_DONE` (`complete`).
- `raw/runs.jsonl`: 79 records (6 warm-ups, 72 used, 1 contaminated original). Each record carries its receipts, masks, per-CPU busy, CPU seconds, mean and median.
- `raw/pass-00` … `raw/pass-05`: per-process `run.csv` / `per_frame_*.csv`, `pose.bin`, stdout and stderr.
- `raw/analysis.md` / `.json`, `raw/analysis2.md` / `.json`: the reductions.
- `test/`: an untimed 12-step rehearsal of `diag.py`. Its numbers are not measurements.
