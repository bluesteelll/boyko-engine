# P0 timed window: MEASUREMENT-QUEUE section 10 (tester, 2026-09-19)

## Verdict: VOID

**The window was voided by §10's A/A0 rule after pass 2. Only 3 of the 5 passes ran.**

- A/A0 band at W=2: B(2) = 2.311 %, above the 2 % ceiling.
- B(W) is a maximum over passes, so later passes could not bring it back under the ceiling. The wrapper stopped the window, as the rule requires.
- **No number below is a measurement to quote.** That covers the boyko/Jolt ratio, f, the identity terms, L1's gate and every lever "build if". Each of these would need a window that passes A/A0.

**The void was not caused by load.**
- 307 of the 308 receipts read at most 4.9 % busy; the median was 1.17 %.
- The single loaded receipt was caught, and its run was re-run clean.
- The band at W=2 comes from a spread between processes at W=2 on this box. A W=2 diagnostic run after the window measured it at about ±2.5 % around the mean, on a quiet machine (last section). Passes 0 and 1 already sat at 1.83 % and 1.91 %. **A re-run under the same protocol is likely to void again at W=2.**
- Changing the protocol (for example thread affinity, more processes per row, or how W=2 is treated) is the orchestrator's call. I changed nothing.

State at the end:
- Tree `D:/wt/joltab` at `dbd859771b4d7aee5f560c7d3cbceccd0f1af179`, clean (`git status --short --untracked-files=all` prints 0 lines). No commit, no push.
- All 5 binaries match `p0/bin/SHA256SUMS` after the window. The wrapper also re-hashed every binary against the manifest before each pass.
- D: had 2.54 GB free before and after. Nothing was compiled.

## Wall clock

| stage | start | end |
|---|---|---|
| idle wait (`wait_log.txt`, 7 polls) | 05:16:03 | 05:22:13 (idle reached) |
| **window** | **05:22:15.259** | **05:46:52.939** (24 min 38 s) |
| pass 0: W ascending, Jolt then boyko | 05:22:17.682 | 05:31:59.082 (includes a 154 s quiet-rule wait and 1 re-run) |
| pass 1: W descending, boyko then Jolt; includes JOLT-P and JOLT-RCPT | 05:32:01.567 | 05:39:48.680 |
| pass 2: W ascending, Jolt then boyko | 05:39:51.445 | 05:46:51.937, then the void |
| passes 3 and 4 | not run | |
| W=2 diagnostic (not part of the window) | 05:49:22 | 05:51:30 |

All times are +03:00.

**Idle wait.**
- Polls 1–4 were busy: another lane's `cargo test --workspace --all-targets --no-fail-fast` (PID 17940, started 04:58), CPU 7.8–14.0 %.
- Polls 5–7 were quiet on all three conditions: 0 build processes, 10-second CPU of 1.59 %, 2.28 % and 0.74 %, and 241–249 processes.

**Machine.** Power plan "High performance", on AC (`BatteryStatus` 2, 100 %) before and after. Task Manager, Steam and a browser were open all through.

## How it was driven

**`tools/physics_parity/driver.py` covers most of the protocol, with two gaps.**

What it implements:
- the §10 rows (the `ROWS` table);
- the H12 order: interleaved by W, Jolt then boyko, reversed on alternate passes;
- binaries run by path, output read and written as bytes;
- a load receipt before and after every timed region;
- the quiet-rule wait;
- the contamination flag.

What it does not implement:
- re-running a contaminated run at the end of its pass;
- stopping on a void.

So I drove it through `p0/window.py`, a wrapper that imports the unmodified driver and calls its `ROWS`, `plan_pass`, `boyko_args`, `jolt_args`, `run_bytes`, `load_receipt` and `reduce`. It adds only three things:
1. **Re-run once.** A run whose receipts show load is re-run once at the end of its pass. A re-run of J-A-a also re-runs J-C, whose canary spin comes from J-A-a. A run contaminated on both attempts would be excluded from the reduction; none was.
2. **Stop rules as we go.** After every pass the driver's own `reduce` runs; B(W) above 2 % or a closure stop ends the window.
3. **Busy abort.** If the quiet rule's 420 s budget runs out, the wrapper aborts rather than time on a busy machine. This never triggered.

Driver defaults were kept: `--busy-pct 5`, `--receipt-window 2 s`.

**Passes are numbered 0–4**, so the first pass has H12's base order (W ascending, Jolt then boyko); that is the driver's even-pass order. JOLT-P and JOLT-RCPT ran in pass 1, per the driver's `once` rule.

**Side effect, cleaned up.** Importing the driver wrote `tools/physics_parity/__pycache__/driver.cpython-314.pyc` into the tree at 05:19. I created that file and directory, so I deleted both. The tree is clean, and `driver.py` equals its HEAD blob `272ef289`.

## Void and stop rules

| rule | result |
|---|---|
| **A/A0** (J-A-d2 / J-A-d1, paired by step) | **VOID at W=2.** Per-pass paired medians by W:<br>• W=1: 0.99883, 1.00054, 1.00394 (B 0.39 %)<br>• **W=2: 0.98168, 0.98093, 0.97689 (B 2.31 %)**<br>• W=4: 1.00413, 1.00296, 1.00778 (B 0.78 %)<br>• W=8: 0.99332, 0.99886, 1.00786 (B 0.79 %)<br>• W=16: 0.99731, 0.99926, 1.00919 (B 0.92 %) |
| A/A1 (J-A-a / J-A-d1; bound max(B, 0.5 %)) | **FAIL at W=1:** median 1.00575 against 0.5 %; per pass 1.00276, 1.00575, 1.01251.<br>**FAIL at W=8:** 1.00797 against 0.786 %.<br>Pass at W=2 (0.99956, against a void band), W=4 (1.00379 against 0.78 %) and W=16 (1.00919 against 0.919 %, a margin of 0.0003 %). |
| A/A2 (tip disarmed / pre-instrument) | Pass at W=1: 1.00050 against 0.5 %.<br>Pass at W=8: 1.00259 against 0.786 %; per pass 1.01071, 1.00259, 0.95447. |
| Canary (J-C against J-A-a) | **Pass at W=1 and W=8.**<br>• W=1: canary span against its spin target +0.034 %; rise −0.025 % of T, within B; the A/A1 statistic 1.0489 FAILS the gate, as required.<br>• W=8: span +0.059 %, rise −0.025 %, A/A1 statistic 1.0539.<br>• Spin target: 0.98–0.99 ms at W=1, 0.46–0.47 ms at W=8.<br>So the A/A1 gate is not blind. |
| Anti-vacuity (W4) | **0 void steps** in all 154 window attempts. **0 non-zero exits**; the runner exits 3 on a void step. |
| Determinism | • **7 boyko simulation groups, 1 pose each.**<br>• `0x32d5e235342b4143` is shared by J-A (d1, a, d2), J-B (cfg-B), J-C, J-P1, J-S0, L1-A1/B/A2 (pre-L1 and tip), AA2-pre and SHIP, at every W.<br>• J-Son `0xcc2a5400c66eecce`; R `0x87e561d20589d4a5` (W=1 serial equals W=8 parallel); R-ref `0x67463b7ea88686d2`; R-S `0x2a2b7926a48aab00`; S16 `0x8877dbb1192e9b92`.<br>• H7 `--expect-pose`: match 6 of 6.<br>• Jolt: 16 binary+flags+W groups, 0 differ; JOLT-T gives `0x0ee15b89965ec747` at every W, JOLT56-T `0xb8522b4e3fc62cfe`. |
| Closure (driver, window means of J-A-a) | **No stop.**<br>• u = 1.33–1.67 µs, 0.010–0.040 % of the solve span, positive.<br>• g = 34–90 µs, positive.<br>• Σ system spans ≤ wall.<br>• drops 0.<br>• waves identical across W in every pass (109.3 per step).<br>• r = 3.9–4.6 µs. |
| Closure (per step, my census of 60 armed runs) | • g < 0: 0 steps.<br>• u < 0: 0.<br>• Σ system spans > wall: 0.<br>• void: 0.<br>• u above 3 % of the solve span on 2–9 of 300 steps in each of the 6 S16 runs. S16's solve span is tiny, and §10's closure rule is about the J-A u(W), so this is informational. |
| R-S (`--frozen-by 300`) | Not void: first frozen step 248 in all 6 runs (W=1 and W=8, three passes). |
| Thread witness | `solve_on_dispatcher_steps` = 0 in all 15 armed J-A runs. Jolt printed threads = W in every run. |
| L1 gate (O7) | The driver says pass: B/A median 1.0088 ≤ 1 + spread. **But the A/A spread is 3.2 %** (0.16 %, 3.21 %, 3.14 % per pass), so the gate cannot see anything under 3 %. Void window, so no verdict. |
| H8 | The flag is set: over [100, 500), Jolt counts 8456.0 manifolds and boyko 4519.3, a ratio of 1.87. **The final top_y agrees** (28.778 against 28.990). This looks like a counting-definition mismatch in the patch's counting `ContactListener` (possibly double counting), not a scene difference. Not verified; check it before a per-manifold figure is used. |
| O5 | All asleep: boyko from step 264, Jolt from frame 127, at both W. Tail values are in `reduction.json` (void window). |

## Receipts

- **308 receipts**: the before and after receipt of each of the 154 window attempts, each a 2 s window.
- Busy as % of 16 logical CPUs: min 0.24, median 1.17, p90 2.4, max 9.99. **1 receipt was above 5 %.**
- Top process by CPU in the receipt window:

  | process | receipts where it was top |
  |---|---|
  | `Taskmgr.exe` | 251 |
  | `claude.exe` | 41 (at most 1.71 % of the machine) |
  | `steamwebhelper.exe` | 10 |
  | `browser.exe` | 2 |
  | `steam.exe` | 2 |
  | `vctip.exe` | 1 |
  | `explorer.exe` | 1 |

- Build processes seen using CPU: `cargo.exe` once, at 05:27:00.
- Quiet-rule waits, 176.1 s in total:
  - J-A-a at W=4, pass 0: 154.1 s, after the contamination below;
  - J-A-d1 at W=16, pass 1: 22.0 s, the pass's first receipt.
- 0 runs aborted, 0 excluded.

## Contaminated

**Another lane ran cargo during pass 0**, despite the quiet declaration.

- **Affected run:** J-A-d1 at W=4, pass 0 (05:26:53–05:26:59). Its after-receipt read 9.99 %.
- **Other processes seen:**
  - `cargo.exe` was busy;
  - `vctip.exe`, which the MSVC linker spawns, at 1.56 %;
  - the boyko_render test binary `sdf_gbuffer_hybrid-2a81a9b8ad27e6f6.exe` at 0.54 %.
- **The load landed inside the run.** Its mean was 11.739 ms, against 10.842 ms on the clean re-run at the end of pass 0 (receipts 4.11 % / 3.54 %). The re-run replaced it.
- **The quiet rule then waited 154 s** before the next run, until the machine was quiet again.
- **No other run was contaminated:** 1 of 153 original runs, 0 re-runs contaminated again.

**Disturbances the receipt protocol cannot see.** J-A-d1 at W=8 has a plateau of 100–125 steps at +13 % to +17 %:
- pass 0: steps about 325–450, at 10.2–10.7 ms against 9.0 ms;
- pass 2: steps about 425–500;
- no single-step spikes;
- both of that run's receipts were clean (0.49 % / 1.39 % in pass 0).

These runs are in the reduction. The step-paired medians absorb them: B(8) = 0.79 %. The cause is unknown: an in-run background burst or a clock or power dip on this laptop part.

## Per-row medians over passes

**VOID window. These are diagnostic only; do not quote them.**

- Each value is the mean step time over the row's window (§10), then the median over the passes that ran.
- The boyko CSV means equal the runner's summary `window_mean_ns` exactly.
- The Jolt means come from the `-f` per-frame CSV and agree with Jolt's own printed steps/s to within 7.4e-9 relative.
- JOLT-P (`-p` shares, 5 HTML charts each at W=1 and W=8) and JOLT-RCPT (untimed) carry no time.

| row | W | median over passes (ms) | min (ms) | max (ms) | n | spread max/min | per pass (ms) |
|---|---|---|---|---|---|---|---|
| J-A-d1 | 1 | 19.566 | 19.489 | 19.608 | 3 | 0.61 % | p0 19.566, p1 19.608, p2 19.489 |
| J-A-d1 | 2 | 13.751 | 13.743 | 13.785 | 3 | 0.30 % | p0 13.751, p1 13.743, p2 13.785 |
| J-A-d1 | 4 | 10.638 | 10.569 | 10.842 | 3 | 2.58 % | p0 10.842 (re-run), p1 10.638, p2 10.569 |
| J-A-d1 | 8 | 9.359 | 9.116 | 9.497 | 3 | 4.18 % | p0 9.497, p1 9.116, p2 9.359 |
| J-A-d1 | 16 | 9.161 | 9.148 | 9.212 | 3 | 0.71 % | p0 9.212, p1 9.161, p2 9.148 |
| J-A-a | 1 | 19.693 | 19.612 | 19.796 | 3 | 0.94 % | p0 19.612, p1 19.693, p2 19.796 |
| J-A-a | 2 | 13.783 | 13.780 | 14.082 | 3 | 2.19 % | p0 14.082, p1 13.780, p2 13.783 |
| J-A-a | 4 | 10.754 | 10.670 | 10.884 | 3 | 2.00 % | p0 10.884, p1 10.670, p2 10.754 |
| J-A-a | 8 | 9.233 | 9.194 | 9.354 | 3 | 1.73 % | p0 9.194, p1 9.354, p2 9.233 |
| J-A-a | 16 | 9.296 | 9.236 | 9.630 | 3 | 4.26 % | p0 9.630, p1 9.296, p2 9.236 |
| J-A-d2 | 1 | 19.540 | 19.513 | 19.616 | 3 | 0.53 % | p0 19.513, p1 19.616, p2 19.540 |
| J-A-d2 | 2 | 13.495 | 13.482 | 13.556 | 3 | 0.55 % | p0 13.556, p1 13.482, p2 13.495 |
| J-A-d2 | 4 | 10.684 | 10.683 | 11.078 | 3 | 3.69 % | p0 11.078, p1 10.683, p2 10.684 |
| J-A-d2 | 8 | 9.153 | 9.124 | 9.227 | 3 | 1.13 % | p0 9.153, p1 9.124, p2 9.227 |
| J-A-d2 | 16 | 9.209 | 9.144 | 9.363 | 3 | 2.39 % | p0 9.209, p1 9.144, p2 9.363 |
| J-B | 1 | 14.158 | 14.034 | 14.427 | 3 | 2.80 % | p0 14.034, p1 14.427, p2 14.158 |
| J-B | 8 | 11.170 | 11.162 | 11.179 | 3 | 0.15 % | p0 11.162, p1 11.179, p2 11.170 |
| J-C | 1 | 20.744 | 20.587 | 20.775 | 3 | 0.91 % | p0 20.587, p1 20.744, p2 20.775 |
| J-C | 8 | 9.714 | 9.692 | 9.733 | 3 | 0.43 % | p0 9.714, p1 9.733, p2 9.692 |
| J-P1 | 1 | 19.874 | 19.717 | 20.020 | 3 | 1.54 % | p0 19.717, p1 19.874, p2 20.020 |
| J-Son | 1 | 9.143 | 9.137 | 9.170 | 3 | 0.36 % | p0 9.137, p1 9.170, p2 9.143 |
| J-Son | 8 | 6.415 | 6.411 | 6.470 | 3 | 0.92 % | p0 6.411, p1 6.415, p2 6.470 |
| J-S0 | 1 | 19.671 | 19.584 | 19.842 | 3 | 1.32 % | p0 19.671, p1 19.584, p2 19.842 |
| L1-A1 (pre-L1) | 1 | 19.187 | 19.021 | 19.504 | 3 | 2.54 % | p0 19.504, p1 19.187, p2 19.021 |
| L1-B (tip) | 1 | 19.678 | 19.229 | 19.713 | 3 | 2.51 % | p0 19.229, p1 19.678, p2 19.713 |
| L1-A2 (pre-L1) | 1 | 19.659 | 19.476 | 19.758 | 3 | 1.45 % | p0 19.476, p1 19.758, p2 19.659 |
| R | 1 | 14.138 | 14.067 | 14.384 | 3 | 2.26 % | p0 14.067, p1 14.138, p2 14.384 |
| R | 8 | 9.751 | 9.688 | 9.832 | 3 | 1.49 % | p0 9.751, p1 9.688, p2 9.832 |
| R-ref | 1 | 24.065 | 23.794 | 24.198 | 3 | 1.70 % | p0 23.794, p1 24.065, p2 24.198 |
| R-S | 1 | 6.068 | 6.025 | 6.068 | 3 | 0.72 % | p0 6.068, p1 6.068, p2 6.025 |
| R-S | 8 | 6.083 | 6.068 | 6.115 | 3 | 0.76 % | p0 6.083, p1 6.068, p2 6.115 |
| S16 | 1 | 0.081 | 0.080 | 0.082 | 3 | 2.26 % | p0 0.080, p1 0.082, p2 0.081 |
| S16 | 8 | 0.080 | 0.079 | 0.080 | 3 | 0.30 % | p0 0.080, p1 0.080, p2 0.079 |
| AA2-pre (pre-instrument) | 1 | 19.619 | 19.549 | 19.668 | 3 | 0.61 % | p0 19.549, p1 19.619, p2 19.668 |
| AA2-pre (pre-instrument) | 8 | 9.179 | 9.135 | 9.626 | 3 | 5.37 % | p0 9.179, p1 9.135, p2 9.626 |
| SHIP (shipping tier) | 1 | 19.492 | 19.405 | 19.544 | 3 | 0.72 % | p0 19.492, p1 19.544, p2 19.405 |
| SHIP (shipping tier) | 8 | 9.198 | 9.016 | 9.370 | 3 | 3.93 % | p0 9.370, p1 9.198, p2 9.016 |
| JOLT-T (v5.3.0 Dist) | 1 | 15.151 | 15.108 | 15.842 | 3 | 4.86 % | p0 15.108, p1 15.151, p2 15.842 |
| JOLT-T | 2 | 8.666 | 8.628 | 8.758 | 3 | 1.51 % | p0 8.666, p1 8.758, p2 8.628 |
| JOLT-T | 4 | 5.221 | 5.161 | 5.241 | 3 | 1.53 % | p0 5.241, p1 5.161, p2 5.221 |
| JOLT-T | 8 | 3.529 | 3.491 | 3.556 | 3 | 1.85 % | p0 3.491, p1 3.529, p2 3.556 |
| JOLT-T | 16 | 3.206 | 3.203 | 3.327 | 3 | 3.89 % | p0 3.203, p1 3.206, p2 3.327 |
| JOLT56-T (v5.6.0 Dist) | 1 | 9.643 | 9.569 | 9.665 | 3 | 1.00 % | p0 9.665, p1 9.643, p2 9.569 |
| JOLT56-T | 2 | 5.677 | 5.664 | 5.681 | 3 | 0.31 % | p0 5.677, p1 5.681, p2 5.664 |
| JOLT56-T | 4 | 3.586 | 3.542 | 3.588 | 3 | 1.30 % | p0 3.588, p1 3.586, p2 3.542 |
| JOLT56-T | 8 | 2.490 | 2.489 | 2.532 | 3 | 1.73 % | p0 2.490, p1 2.489, p2 2.532 |
| JOLT56-T | 16 | 2.429 | 2.380 | 2.484 | 3 | 4.40 % | p0 2.380, p1 2.429, p2 2.484 |
| JOLT-NPC | 8 | 5.071 | 4.994 | 5.122 | 3 | 2.55 % | p0 5.122, p1 4.994, p2 5.071 |
| JOLT-SLP | 1 | 1.901 | 1.895 | 1.944 | 3 | 2.61 % | p0 1.901, p1 1.895, p2 1.944 |
| JOLT-SLP | 8 | 0.424 | 0.407 | 0.430 | 3 | 5.73 % | p0 0.430, p1 0.407, p2 0.424 |

The driver's full reduction (profile spans, identity terms, sub-windows, O5 tails, Δ_J, the shipping ratio) is in `raw/reduction.json` and `raw/reduction.txt`. It carries the same VOID status.

## Diagnostic: why W=2 voids

This was run after the window, on the still-idle machine. It is not part of the window, and its numbers are not measurements.

**The signature in the window.**
- At W=2, J-A-d2 ran faster than J-A-d1 by 1.6–2.5 % in every 100-step segment of all three passes, in both run orders. So it is a shift at the level of the whole process, from step 0, not a transient.
- J-A-a sat with J-A-d1 in passes 1 and 2.
- No other W shows it. That includes W=4 and W=16 in pass 0, where d1, a and d2 also ran back to back.

**The diagnostic** (`raw/diag_w2/`) used the same binaries, the same arguments, the driver's receipts and the same quiet rule. Receipts were at most 2.75 %, with no build processes. It had two parts.

1. The W=2 group replayed exactly, twice:

   | replay | JOLT-T | JOLT56-T | d1 | a | d2 | d2/d1 |
   |---|---|---|---|---|---|---|
   | 1 | 8.524 | 5.679 | 13.520 | 13.867 | 13.832 | about +2.3 % |
   | 2 | 8.568 | 6.082 | 13.756 | 13.511 | 13.544 | about −1.6 % |

   Times are ms. **The sign flips.**

2. Six identical, consecutive, disarmed J-A runs at W=2: 14.059, 13.691, 14.175, 13.843, 13.752 and 13.480 ms. That is a **5.2 % range on a quiet machine**.

**Conclusion.** At W=2 the step time of a whole run moves by about ±2.5 % from one process to the next, which is above the 2 % A/A0 ceiling. It is not a fixed order effect: the window's same sign in 3 of 3 passes was chance. So the A/A0 gate at W=2 measures per-process variance, not whether the machine is quiet.

The mechanism is not measured. One candidate is per-process thread placement: whether the two workers land on SMT siblings or on separate cores, and relative to the parked dispatcher. That could be tested with pinned affinity, but that is a protocol decision.

## Findings for the orchestrator

1. **The window is VOID** (A/A0 at W=2). Under the unchanged protocol a re-run will most likely void again at W=2, because the per-process spread exceeds the ceiling. Decide first whether §10's A/A0 design for W=2 changes; for example affinity, several processes per row, or the median of more runs.
2. **A/A1 fails at W=1 and W=8.** Arming costs 0.3–1.3 % at W=1 in every pass, which is above the 0.5 % floor. This bears on whether the armed profile is a faithful stand-in at W=1; the window is void, so no verdict.
3. **L1's A/A spread is 3.2 %** on the J-S0 rows at W=1, against 0.39 % for J-A at W=1. The L1 gate as designed cannot resolve anything under about 3 %.
4. **H8's manifold counts differ by 1.87x** while the final top_y agrees. Check the Jolt patch's counting `ContactListener` before any per-manifold figure is quoted.
5. **The quiet declaration did not hold.** Another lane ran cargo and a test binary at about 05:27, during the window. It was caught, re-run and waited out.

## Files

Everything is under `docs/measurements/2026-09-19-physics-p0/p0/`.

- `window_report.md`: this report.
- `wait_log.txt`: every idle poll.
- `wait_idle.ps1`, `launch_after_idle.py`, `window.py`, `analyze.py`, `diag_w2.py`: the scripts used.
- `raw/window_log.txt`: every run line, receipts, re-runs and the per-pass stop checks.
- `raw/window_state.json`: start, end, machine state and status.
- `raw/WINDOW_DONE`: the status.
- `raw/runs_all.jsonl`: every attempt, with both receipts.
- `raw/runs.jsonl`: the reduction input.
- `raw/pass-00/`, `raw/pass-01/`, `raw/pass-02/`: one directory per run, holding `stdout.txt`, `stderr.txt` and output. boyko runs write `run.csv` (per step) and `pose.bin`. Jolt runs write `per_frame_*.csv`, `profile_chart_*.html` (JOLT-P) and `receipt_discrete_th1.csv` (JOLT-RCPT).
- `raw/reduction.json` and `raw/reduction.txt`: the driver's reduce after pass 2.
- `raw/reduction_after_pass{0,1,2}.json` and `raw/reduce_after_pass{0,1,2}.txt`: the stop check after each pass.
- `raw/analysis.json`, `raw/analysis.txt`, `raw/rows_table.md`: the supplementary analysis.
- `raw/diag_w2/`, `raw/diag_w2_stdout.log`: the W=2 diagnostic.
- `wtest/`, `wtest_stdout.log`: an untimed rehearsal of the wrapper, run under load before the window. Its numbers are not measurements.

§10 says receipts go in `docs/measurements/<date>-physics-p0/`. Nothing was written into the tree, per the task.
