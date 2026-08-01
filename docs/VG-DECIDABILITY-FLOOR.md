# VG — the decidability floor — MACHINE-WRITTEN by `vg_decidability_floor_measure`

**This run measured a floor of 10.3 %** — three sigma on a single-session reading, from a worst-statistic CV of 3.4 % (`flat_shade_ns`).

⚠️ **Do not read that as "the floor".** Repeated runs of this SAME protocol on this same box span roughly **5 %–15 %**. The defensible output of this rung is the RULE in the next section, not the number in this one.

Measured as a **NULL EXPERIMENT**: the shipped `vb_p1d_cull_shade_bench` class, same scene, same configuration, run in separate processes. Nothing differs between sessions, so every difference below is instrument plus environment. **A delta smaller than this is not resolvable by construction** — no statistical treatment recovers a signal from beneath the noise of the thing measuring it.

Protocol: **3 independent repetitions × 7 sessions** per configuration (42 bench processes in total).

## ⚠️ THE FLOOR IS NOT A CONSTANT — and that, not any single number, is this rung's result

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
| 1 | 17.8 % |
| 2 | 5.3 % |
| 3 | 5.8 % |

**Repetition floors span 5.3 %–17.8 %, a factor of 3.35.** Read the headline as an order of magnitude, never as a constant. The table below pools every session, which is the estimate with the most evidence behind it.

| statistic | median (ns) | mean (ns) | peak-to-peak | CV | samples |
|---|---|---|---|---|---|
| `cull_reset_ns` | 577.2 | 577.0 | **4.9 %** | 1.0 % | [556, 571, 573, 573, 574, 574, 575, 575, 576, 577, 577, 577, 579, 579, 579, 579, 581, 581, 583, 584, 585] |
| `cull_dispatch_ns` | 13262.5 | 13188.7 | **11.5 %** | 2.9 % | [12210, 12434, 12504, 13032, 13095, 13153, 13174, 13195, 13244, 13254, 13262, 13272, 13292, 13348, 13391, 13392, 13420, 13437, 13509, 13594, 13742] |
| `froxel_shade_ns` | 28276.4 | 28077.1 | **13.3 %** | 3.0 % | [25646, 26498, 27015, 27634, 27708, 28029, 28039, 28099, 28160, 28257, 28276, 28299, 28336, 28341, 28374, 28471, 28588, 28644, 28695, 29090, 29412] |
| `froxel_total_ns` | 42088.4 | 41842.9 | **12.3 %** | 2.9 % | [38430, 39582, 40031, 41310, 41312, 41870, 41871, 42028, 42052, 42084, 42088, 42142, 42165, 42212, 42223, 42321, 42587, 42787, 42943, 43063, 43591] |
| `flat_shade_ns` | 41853.7 | 41615.6 | **16.3 %** | 3.4 % | [37497, 38525, 40666, 40852, 41434, 41490, 41616, 41630, 41653, 41695, 41853, 41900, 41998, 42025, 42081, 42263, 42272, 42361, 42677, 43096, 44334] |

**The floor is 3 sigma × the WORST statistic's CV — `flat_shade_ns` at 3.4 %, giving 10.3 %** — worst rather than best or average, because a campaign quoting its tightest statistic as "the floor" would be certifying deltas it cannot resolve on any other one.

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
