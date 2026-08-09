# VG — the decidability floor — MACHINE-WRITTEN by `vg_decidability_floor_measure`

**This run measured a floor of 21.2 %** — three sigma on a single-session reading, from a worst-statistic CV of 7.1 % (`froxel_shade_ns`).

⚠️ **Do not read that as "the floor".** The estimator moves — by a factor of several between IDENTICAL protocols on this box, measured on both the retired stdout channel and the artifact channel that replaced it. The cross-sitting series is kept in `docs/diagnostics/profiling/05-LADDER-GATES.md` (profiling rung 7b); this run's own repetition span is tabulated below. **The migration did not make the instrument quieter.** The defensible output of this rung is the RULE in the next section, not the number in this one.

Measured as a **NULL EXPERIMENT**: the shipped `vb_p1d_cull_shade_bench` class, same scene, same configuration, run in separate processes. Nothing differs between sessions, so every difference below is instrument plus environment. **A delta smaller than this is not resolvable by construction** — no statistical treatment recovers a signal from beneath the noise of the thing measuring it.

## The channel and the workload these numbers belong to

Read through the **profiling artifact** (`BOYKO_VB_ZONE` + `boyko_app::profiling::artifact`), NOT the retired `VB-P1d` stdout line — profiling rung 7. The statistic is each zone's **median**, where the printed channel published means.

| leg | derived `workload_tag` | declared `content_tag` |
|---|---|---|
| froxel | `visibilitybuffer_mesh#605f3da8` | `n14_kronecker` |
| flat | `visibilitybuffer_mesh#2cf8fcbd` | `n14_kronecker` |

⚠️ **The two tags differ, and that is asserted rather than assumed.** They are derived from the whole boot-resolved render path, so a `BOYKO_VB_FROXEL_FORCE_OFF` that changed nothing would give one tag on both rows and fail the run — a null experiment across two conditions that were secretly one. **Any floor published before rung 7 was taken on a different instrument and bounds nothing about this one**, by this rung's own rule.

Protocol: **3 independent repetitions × 7 sessions** per configuration (42 bench processes in total).

## ⚠️ THE FLOOR IS NOT A CONSTANT — and that, not any single number, is this rung's result

⚠️ The four runs below were taken on the **RETIRED stdout channel** (means over `VB-P1d` lines), before profiling rung 7 moved this rung to the artifact and to medians. They are kept because the FINDING they establish is about the estimator and the box, not about the channel — but their numbers are not comparable with the table further down, and nothing here claims the new channel is quieter: that would need this same repeated protocol run on both, which no sitting has done.

This protocol was run four times while it was being built. The floors it reported, in order, with what changed between them:

| run | protocol | floor | note |
|---|---|---|---|
| 1 | 7 sessions, peak-to-peak | **6.3 %** | first measurement |
| 2 | 7 sessions, peak-to-peak | **14.3 %** | *identical protocol*, 2.3× higher |
| 3 | 3 × 7, CV-derived | **4.7 %** | statistic changed after run 2 refuted peak-to-peak |
| 4 | 3 × 7, CV-derived | **13.5 %** | *identical protocol*, 2.9× higher |

Runs 1↔2 and 3↔4 are pairs of **identical** protocols on the same box and the same scene. They differ by roughly **3×**. Changing the statistic (peak-to-peak → CV) did not fix it, and neither did tripling the sessions.

**So the operational result is not a threshold, it is a rule:**

> **On this box, a claimed GPU-timing delta below ~15 % is not defensible without a NULL CONTROL measured in the same sitting.** The floor drifts on a timescale shorter than the gap between two of these runs — thermal state, driver residency, background load — so a floor measured yesterday does not bound a delta measured today.

This is a stronger and more useful finding than a constant would have been, and it fully explains the failure the research document records: a *"22× result measured inside"* a regime that *"does not reproduce"*. The remedy is not a better number here; it is that every future rung claiming a delta runs its own A/A control beside its A/B.

The single run that produced the table below repeats the whole experiment and publishes each repetition's own floor, so the drift is visible within one sitting too:

| repetition | floor (worst peak-to-peak) |
|---|---|
| 1 | 25.0 % |
| 2 | 7.0 % |
| 3 | 21.6 % |

**Repetition floors span 7.0 %–25.0 %, a factor of 3.56.** Read the headline as an order of magnitude, never as a constant. The table below pools every session, which is the estimate with the most evidence behind it.

| statistic | median (ns) | mean (ns) | peak-to-peak | CV | samples |
|---|---|---|---|---|---|
| `cull_reset_ns` | 512.0 | 509.0 | **12.5 %** | 4.4 % | [480, 480, 480, 480, 480, 480, 512, 512, 512, 512, 512, 512, 512, 512, 512, 512, 512, 544, 544, 544, 544] |
| `cull_dispatch_ns` | 13872.0 | 13861.3 | **10.7 %** | 3.0 % | [13024, 13280, 13376, 13408, 13520, 13552, 13600, 13632, 13776, 13792, 13872, 13952, 13952, 13984, 14112, 14272, 14304, 14368, 14400, 14400, 14512] |
| `froxel_shade_ns` | 25600.0 | 25843.8 | **24.0 %** | 7.1 % | [23552, 23552, 23552, 24576, 24576, 24576, 24576, 25600, 25600, 25600, 25600, 25600, 25600, 25600, 25600, 25600, 27648, 27648, 28672, 29696, 29696] |
| `flat_shade_ns` | 40960.0 | 41239.6 | **12.5 %** | 2.8 % | [38912, 39936, 39936, 39936, 39936, 40960, 40960, 40960, 40960, 40960, 40960, 41200, 41472, 41984, 41984, 41984, 41984, 41984, 41984, 43008, 44032] |

**The floor is 3 sigma × the WORST statistic's CV — `froxel_shade_ns` at 7.1 %, giving 21.2 %** — worst rather than best or average, because a campaign quoting its tightest statistic as "the floor" would be certifying deltas it cannot resolve on any other one.

## What this decides

**K3 — the undecidable harness** — is the kill this measures. Any rung claiming a delta **below** the figure above is not defensible on this box, whatever the arithmetic around it. The research ladder's R2 is the immediate case: its own expected magnitude on this content is stated as *"near zero"*, so its gate — *"measured Δ, decidable by R0's floor"* — is unsatisfiable in **both** directions at once. R2 still has value, but that value is de-risking the cull-pass declaration, compaction, indirect barriers and count buffers; it is not the delta, and its gate should say so.

## Two statistics, and why both are here

**Peak-to-peak** `(max − min) / median` is the definition `sv0_deferred_term_bench` already uses for its own cross-session gate, so these numbers are comparable to the one existing gate in the tree. ⚠️ It **grows with session count**, so it is only meaningful beside its `n` — which is why `n` is printed above. That growth is in the safe direction for a floor.

**CV** `σ / mean` is stable in `n`, and it is what the floor is built from.

⚠️ **That choice OVERTURNED this rung's own first design, by measurement.** The draft adopted the worst peak-to-peak, on the argument that a floor which under-states noise is the one direction that silently blesses wrong constants — the failure this project has already recorded once. The repetitions refuted it: peak-to-peak floors swung ~4× between identical runs while the CV barely moved. **A bound that cannot reproduce itself is not a bound, however conservative it looks on any single run.** Peak-to-peak stays in the table because the one existing gate in the tree is written in it; it is not what a new gate should use.

## What this does NOT decide

* **It is one box.** The floor is a property of this GPU, this driver and this machine's background load, not of the engine.
* **It is one bench class.** GPU timestamp brackets around compute dispatches. A CPU-side or end-to-end frame-time measurement has its own floor and does not inherit this one.
* ⚠️ **It is one CONFIGURATION, and this bounds what it contradicts.** These sessions ran the bench's default light rig. The research document's *"does not reproduce above N=128 with ~21% spread"* is a reading at a much heavier configuration, and nothing here refutes it: a floor is a property of the workload as much as of the box, and a rung that measures at a different scale must re-measure its own floor rather than cite this one. What this figure DOES establish is that the class is not hopeless — the noise is single- digit percent where the workload is light, so a rung with a large enough effect can be decidable here.
* **It is not a confidence interval.** It bounds what is resolvable; it does not say how many sessions a future rung needs to resolve a given delta. That is the CV's job and it is recorded above rather than applied here.
* **No clock pinning was applied.** The floor therefore includes driver/OS clock behaviour, which is what a real measurement on this box would also include. A pinned-clock floor would be tighter and would describe a machine nobody measures on.
