# MEASUREMENT-QUEUE §6–§9: timing record (raw), 2026-09-18

Tester, timing stage. 62 timed runs in total, from 2026-09-18T21:05:01.786+03:00 to 2026-09-18T22:01:12.012+03:00: the 46 the protocol asks for, plus 16 supplementary runs (n = 3, 4; see §0.4). §1 summarises, §2 indexes every run in one row per benchmark id, §3 holds the full per-run records. No run compiled anything. Each run is a PREBUILT bench-profile exe from `exes.json`, invoked directly with no cargo involved. Machine: 16 logical CPUs; the owner declared it quiet at about 19:00, and no other workflow ran.

## 0. Conditions and protocol

### 0.1 Invocation
- `cd <worktree>/crates/boyko_physics; CARGO_TARGET_DIR=<arm target dir> CRITERION_DEBUG=1 <exe> --bench --noplot <filter>`. The driver (`run_one.py`) removes `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CRITERION_HOME`, `CARGO` and `CARGO_CRITERION_PORT` from the environment, and before every run it checks the exe sha256 against `exes.json` (it asserts). Every exe matched.
- **Filters are exact.** `full_step/1` as a criterion regex also matches `full_step/16`, so every single-row run uses `--exact <full id>`. Multi-row runs use anchored regexes (`^row_identity_churn/`, `^row_identity_churn/stable/`, `^soft_step_sp2/coupled/`, `^sleeping_pipeline/`). Every run's record lists the ids that actually ran. All 62 match their entry exactly, and none ran 0 rows.
- Additions to the spec's `cargo bench -- <filter>`:
  - `--noplot` skips criterion's plot/HTML rendering between rows. gnuplot is absent, so the fallback would be in-process plotters. The flag has no effect on sampling.
  - `CRITERION_DEBUG=1` makes criterion 0.5.1 print `Completed <W> iterations` after warm-up. That is the W §9 R2 needs. It adds prints only, outside the timed region.
- The value recorded is `estimates.json` `median.point_estimate` with its 95 % CI (the median of per-sample time/iteration). Criterion's console `time:` line is the linear-regression **slope**, which is listed beside it for reference.
- `estimates.json` is copied, together with sample.json, benchmark.json and tukey.json, from `<target>/criterion/<id>/new/` to `raw/<entry>/<arm><n>/<dir>/` right after each run. The driver checks that every copied estimates.json has a LastWriteTime later than its run start. All 62 runs passed that check, so no stale file was recorded.

### 0.2 Load receipts
- BEFORE and AFTER every run, in PowerShell (`receipt.ps1`):
  - `(Get-Process).Count`.
  - The 10 × 1 s average of `\Processor(_Total)\% Processor Time`.
  - The top 5 processes by CPU-time delta over those same 10 s. This covers the processes whose `TotalProcessorTime` is readable without elevation, about 108 of about 235.
  - Whether any cargo / rustc / link / lld / dxc / clippy / miri / rust-analyzer process exists.
- **Toolchain processes:** no cargo, rustc, link, lld, dxc or clippy process existed at any receipt. `rust-analyzer` was present throughout and never appeared in any top 5, so it was idle.
- **Background at every receipt:** the owner's browser (`browser`, several processes, intermittent bursts), Task Manager, steamwebhelper, audiodg, and claude.
- **DURING-run accounting** was added to the driver after the s9 full_step/1 required set, once the after-receipt of A2 had shown a burst (see 0.3). It covers s9 full_step/4, s9 awake and all supplementary runs:
  - `GetSystemTimes` busy time over the run window, minus the bench process's own CPU (`GetProcessTimes`), as a % of the whole machine.
  - The top 5 other processes by CPU delta over the run.
  - Runs before that have only the before/after receipts.

### 0.3 Wait rule, contamination, suspect runs
- **Wait rule:** 2 runs had a before-receipt above 5 % and waited. Each wait was one poll of about 59 s, after which the receipt settled below 5 %: s9 full_step_1 B1 (first poll 5.2 %, browser top); s9 full_step_1 B2 (first poll 9.69 %, browser top). **No run was CONTAMINATED under the before-rule.** The maximum before-receipt was 4.42 %.
- **After-receipts above 5 %:** s7 full_step_1 B1 = 5.61 %; s7 churn_stable B2 = 5.4 %; s9 full_step_1 A2 = 16.66 %; s9 full_step_4 B2 = 5.12 %.
  - **s9 full_step/1 A2 is SUSPECT.** Its after-receipt was 16.66 %: four browser processes used 15 cpu-s in 10 s, and the per-second samples ran 13–28 %. Its median, 20.61 ms, sits 6–10 % above the other three A runs (18.66 / 19.49 / 18.97).
  - The other three sit at 5.1–5.6 %, all browser bursts.
    - s7 full_step/1 B1 (5.61 %): its median is within 0.3 % of B2.
    - s9 full_step/4 B2 (5.12 %): its median is 2.8 % above B1.
    - s7 churn_stable B2 (5.4 %): its stable/sleeping_off median is the outlier described in 0.4.
- **During-run non-bench load** over the 24 runs that have it: 1.72–4.1 % of the machine, about 0.3–0.7 of one core. The browser and Task Manager were the top consumers throughout.

### 0.4 Supplementary runs (n = 3, 4), beyond the A1 B1 A2 B2 protocol
Each set was run interleaved as A3 B3 A4 B4, immediately after the required set:
- **s9 full_step/1:** replaces the suspect A2.
- **s9 pyramid_sleeping_off:** the PRIMARY row. B1 and B2 differed by 3.6 %.
- **s9 pyramid_awake_sleeping_on:** the A/A spread was 4.5 %.
- **s7 row_identity_churn/stable:** B2 sleeping_off read 17.55 ms against 16.97–17.12 ms for the other three runs of that row, with receipts of 4.1 % before and 5.4 % after, and it alone would have moved s7 R1.
The summaries in §1 list the required set (n = 1, 2) first, then the all-runs set. **Rule application is left to the results-analyst.**

### 0.5 Procedural notes
- The first run, s6 full_step/1 A1, was captured by the first driver version, which wrote stdout and stderr to separate files. That broke the id and W parse. Its `new/` dir was untouched when the record was re-derived from it (the next write to that target dir came from A2). The logs are kept as `.out` / `.err` / `.log`, and the superseded parses as `runs_first_parse.jsonl` / `runs_second_parse.jsonl`.
- Criterion also writes a `base/` baseline in each target dir, so A2 printed a change against A1 in the same dir. Those change lines are console output only.
- No file in any worktree or repository was modified. Nothing was committed or deleted. The only new files are under the scratch dir `mq/`, plus criterion's own `criterion/` dirs inside `D:/wt/_targets/mq-*`.

## 1. Summaries (computed from the medians below)

Definitions:
- spread = (max − min) / mean, over the medians of one arm.
- B/A = mean(B medians) / mean(A medians).
- range = the most pessimistic and most optimistic B/A pairing.

### §6 — A = d5782d43, B = b74f7ee8
| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.3185 / 18.8567 | 18.7951 / 18.7534 | 2.42 % | 0.22 % | 0.9836 | 0.9708 .. 0.9967 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.0775 / 11.2249 | 11.1455 / 10.7047 | 1.32 % | 4.03 % | 0.9797 | 0.9537 .. 1.0061 |

`row_identity_churn` (B only, all 12 rows, B1 and B2). Arm / `stable` ratio uses the same run's `stable` row with the same sleeping setting:

| sleeping | arm | B1 median (ms) | B2 median (ms) | B/B spread | arm/stable B1 | arm/stable B2 | arm/stable (means) |
|---|---|---|---|---|---|---|---|
| sleeping_off | stable | 16.8461 | 17.0567 | 1.24 % | 1.0000 | 1.0000 | 1.0000 |
| sleeping_off | swap_churn | 17.1893 | 16.7919 | 2.34 % | 1.0204 | 0.9845 | 1.0023 |
| sleeping_off | archetype_shift | 16.7894 | 16.7695 | 0.12 % | 0.9966 | 0.9832 | 0.9899 |
| sleeping_off | first_archetype_spawn | 16.6071 | 17.0903 | 2.87 % | 0.9858 | 1.0020 | 0.9939 |
| sleeping_off | burst_despawn | 16.5297 | 17.1821 | 3.87 % | 0.9812 | 1.0074 | 0.9944 |
| sleeping_off | burst_migrate | 16.8776 | 16.9283 | 0.30 % | 1.0019 | 0.9925 | 0.9971 |
| sleeping_on | stable | 16.6673 | 16.6225 | 0.27 % | 1.0000 | 1.0000 | 1.0000 |
| sleeping_on | swap_churn | 16.8403 | 16.7267 | 0.68 % | 1.0104 | 1.0063 | 1.0083 |
| sleeping_on | archetype_shift | 16.8685 | 16.7642 | 0.62 % | 1.0121 | 1.0085 | 1.0103 |
| sleeping_on | first_archetype_spawn | 16.6618 | 16.7118 | 0.30 % | 0.9997 | 1.0054 | 1.0025 |
| sleeping_on | burst_despawn | 16.5212 | 16.7115 | 1.15 % | 0.9912 | 1.0054 | 0.9983 |
| sleeping_on | burst_migrate | 16.6224 | 16.7752 | 0.92 % | 0.9973 | 1.0092 | 1.0032 |

Structural receipt printed by every row_identity_churn run, for 4 churn steps. It was identical in all 10 logs, both s6 B runs and all 8 s7 runs on both arms:

| row | maps built | rows resolved by stage 2 |
|---|---|---|
| stable/sleeping_off | 0 | 0 |
| swap_churn/sleeping_off | 4 | 4 |
| archetype_shift/sleeping_off | 4 | 5 |
| first_archetype_spawn/sleeping_off | 4 | 2 |
| burst_despawn/sleeping_off | 4 | 124 |
| burst_migrate/sleeping_off | 4 | 2546 |
| stable/sleeping_on | 0 | 0 |
| swap_churn/sleeping_on | 4 | 4 |
| archetype_shift/sleeping_on | 4 | 5 |
| first_archetype_spawn/sleeping_on | 4 | 2 |
| burst_despawn/sleeping_on | 4 | 124 |
| burst_migrate/sleeping_on | 4 | 2546 |

### §7 — A = d552be05, B = a56007ab
| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.0483 / 19.3088 | 19.2060 / 19.2486 | 1.36 % | 0.22 % | 1.0025 | 0.9947 .. 1.0105 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.7231 / 10.7650 | 11.0553 / 10.6700 | 0.39 % | 3.55 % | 1.0110 | 0.9912 .. 1.0310 |

`row_identity_churn/stable`, the required set (n = 1, 2):

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.0243 / 16.9737 | 17.1154 / 17.5472 | 0.30 % | 2.49 % | 1.0195 | 1.0054 .. 1.0338 |
| `row_identity_churn/stable/sleeping_on` | 16.7693 / 16.7577 | 17.0705 / 16.7199 | 0.07 % | 2.08 % | 1.0079 | 0.9971 .. 1.0187 |

`row_identity_churn/stable`, all runs (n = 1..4, supplementary included):

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.0243 / 16.9737 / 17.1108 / 16.9773 | 17.1154 / 17.5472 / 17.1838 / 16.7985 | 0.81 % | 4.36 % | 1.0082 | 0.9817 .. 1.0338 |
| `row_identity_churn/stable/sleeping_on` | 16.7693 / 16.7577 / 16.6816 / 16.8479 | 17.0705 / 16.7199 / 16.6283 / 17.1120 | 0.99 % | 2.87 % | 1.0071 | 0.9870 .. 1.0258 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `soft_step_sp2/coupled/64` | 0.0873 / 0.0874 | 0.0869 / 0.0869 | 0.17 % | 0.08 % | 0.9950 | 0.9938 .. 0.9963 |
| `soft_step_sp2/coupled/256` | 0.3835 / 0.3815 | 0.3814 / 0.3823 | 0.52 % | 0.24 % | 0.9984 | 0.9946 .. 1.0022 |

§7 R4 (the alloc census / attribution) is structural. It was not run here and is still outstanding (see the build report).

### §8 — A = d552be05 + port (contact_wakes helper = 0), B = a56007ab
| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 17.7852 / 17.8312 | 18.1349 / 17.5806 | 0.26 % | 3.10 % | 1.0028 | 0.9859 .. 1.0197 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 16.7251 / 17.0295 | 17.3349 / 17.2416 | 1.80 % | 0.54 % | 1.0244 | 1.0125 .. 1.0365 |
| `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.6345 / 5.7020 | 5.4530 / 5.6311 | 1.19 % | 3.21 % | 0.9777 | 0.9563 .. 0.9994 |

- **Frozen-arm receipts.** All 4 s8 logs print `froze at step 61, manifolds 8555, islands 1` and `contact_wakes 0`. Arm A's 0 is the stub; arm B's is the real counter.
  - R3: freeze step, manifolds and islands are the same on both trees.
  - R4: the manifold count is not 0.
  - The same receipt appears in all 16 s9 sleeping_pipeline logs, because setup always builds the frozen pile.
- **`jolt_parity_pyramid/full_step/1` cross-check: REUSED from §7, not re-run.** It is the same pair of arms (d552be05 / a56007ab) and the same binaries: exe sha256 `aa70f3e2…` on A and `662a9685…` on B.
  - The d552be05 bench exe was built before the s8 port and was byte-identical after it (build report §1).
  - So §7's full_step/1 table above is §8's cross-check row.

### §9 — A = 08fe7b9f, B = 8d656ad8
`pyramid_sleeping_off` (PRIMARY), `full_step/1`, `full_step/4` and `pyramid_awake_sleeping_on`, required set (n = 1, 2):

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.8551 / 18.4564 | 26.1900 / 25.2639 | 2.14 % | 3.60 % | 1.3790 | 1.3399 .. 1.4190 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.6569 / 20.6072 | 20.7566 / 20.9655 | 9.93 % | 1.00 % | 1.0626 | 1.0072 .. 1.1237 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.8959 / 11.0667 | 11.6027 / 11.9273 | 1.56 % | 2.76 % | 1.0714 | 1.0484 .. 1.0947 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.8255 / 17.0423 | 21.5734 / 21.0114 | 4.49 % | 2.64 % | 1.2213 | 1.1787 .. 1.2659 |

All runs (n = 1..4). The full_step/1 row is shown with and without the suspect A2:

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.8551 / 18.4564 / 18.9987 / 18.5245 | 26.1900 / 25.2639 / 27.2549 / 26.9695 | 2.90 % | 7.54 % | 1.4122 | 1.3298 .. 1.4767 |

| row (n=1..4, incl. A2) | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.6569 / 20.6072 / 19.4885 / 18.9712 | 20.7566 / 20.9655 / 21.1604 / 21.1147 | 10.04 % | 1.92 % | 1.0807 | 1.0072 .. 1.1342 |

| row (n=1..4, A2 excluded) | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.6569 / 19.4885 / 18.9712 | 20.7566 / 20.9655 / 21.1604 / 21.1147 | 4.37 % | 1.92 % | 1.1030 | 1.0651 .. 1.1342 |

| row | A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.8255 / 17.0423 / 18.1057 / 17.0879 | 21.5734 / 21.0114 / 21.8375 / 20.2263 | 6.07 % | 7.61 % | 1.2082 | 1.1171 .. 1.2814 |

**The sleeping-on row (`pyramid_awake_sleeping_on`) is NOT like for like** (§9: arm A's gravity-on pile never freezes and arm B's does).
- This bench arm uses `sleep_threshold = 0`, so neither arm latches.
- Beside the primary number only. It is never S5's price.

#### §9 R2: live contact points over each run's OWN timed steps

Source: the probe series `probe/sp_<sha8>.csv` and `probe/jolt_<sha8>.csv`, from the build report §5b, evaluated with `probe/r2_window.py`'s functions over each run's own window:
- **sleeping_pipeline:** steps [31+W, 30+W+N].
- **full_step:** every sample restarts at step 21. Linear d = i1 (the first sample's iteration count from sample.json).

| row | run | W | N | sampling | timed steps | mean live points / timed step |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | A1 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 14734.0 |
| `sleeping_pipeline/pyramid_sleeping_off` | B1 (8d656ad8) | 127 | 210 | linear d=1 | 158–367 | 22957.6 |
| `sleeping_pipeline/pyramid_sleeping_off` | A2 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 14734.0 |
| `sleeping_pipeline/pyramid_sleeping_off` | B2 (8d656ad8) | 127 | 210 | linear d=1 | 158–367 | 22957.6 |
| `jolt_parity_pyramid/full_step/1` | A1 (08fe7b9f) | 255 | 420 | linear d=2 | 21–60 | 15532.7 |
| `jolt_parity_pyramid/full_step/1` | B1 (8d656ad8) | 255 | 420 | linear d=2 | 21–60 | 17499.4 |
| `jolt_parity_pyramid/full_step/1` | A2 (08fe7b9f) | 255 | 420 | linear d=2 | 21–60 | 15532.7 |
| `jolt_parity_pyramid/full_step/1` | B2 (8d656ad8) | 255 | 420 | linear d=2 | 21–60 | 17499.4 |
| `jolt_parity_pyramid/full_step/4` | A1 (08fe7b9f) | 511 | 630 | linear d=3 | 21–80 | 14853.5 |
| `jolt_parity_pyramid/full_step/4` | B1 (8d656ad8) | 511 | 420 | linear d=2 | 21–60 | 17499.4 |
| `jolt_parity_pyramid/full_step/4` | A2 (08fe7b9f) | 511 | 630 | linear d=3 | 21–80 | 14853.5 |
| `jolt_parity_pyramid/full_step/4` | B2 (8d656ad8) | 255 | 630 | linear d=3 | 21–80 | 17489.8 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | A1 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 13510.2 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | B1 (8d656ad8) | 255 | 420 | linear d=2 | 286–705 | 17167.6 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | A2 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 13510.2 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | B2 (8d656ad8) | 255 | 420 | linear d=2 | 286–705 | 17167.6 |
| `jolt_parity_pyramid/full_step/1` | A3 (08fe7b9f) | 255 | 420 | linear d=2 | 21–60 | 15532.7 |
| `jolt_parity_pyramid/full_step/1` | B3 (8d656ad8) | 255 | 420 | linear d=2 | 21–60 | 17499.4 |
| `jolt_parity_pyramid/full_step/1` | A4 (08fe7b9f) | 255 | 420 | linear d=2 | 21–60 | 15532.7 |
| `jolt_parity_pyramid/full_step/1` | B4 (8d656ad8) | 255 | 420 | linear d=2 | 21–60 | 17499.4 |
| `sleeping_pipeline/pyramid_sleeping_off` | A3 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 14734.0 |
| `sleeping_pipeline/pyramid_sleeping_off` | B3 (8d656ad8) | 127 | 210 | linear d=1 | 158–367 | 22957.6 |
| `sleeping_pipeline/pyramid_sleeping_off` | A4 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 14734.0 |
| `sleeping_pipeline/pyramid_sleeping_off` | B4 (8d656ad8) | 127 | 210 | linear d=1 | 158–367 | 22957.6 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | A3 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 13510.2 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | B3 (8d656ad8) | 255 | 420 | linear d=2 | 286–705 | 17167.6 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | A4 (08fe7b9f) | 255 | 420 | linear d=2 | 286–705 | 13510.2 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | B4 (8d656ad8) | 255 | 420 | linear d=2 | 286–705 | 17167.6 |

| row | set | A pts/step (mean over runs) | B pts/step (mean over runs) | row ratio B/A (points) | time ratio B/A (medians) | time/row |
|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | n=1,2 | 14734.0 | 22957.6 | 1.5581 | 1.3790 | 0.8851 |
| `sleeping_pipeline/pyramid_sleeping_off` | n=1..4 | 14734.0 | 22957.6 | 1.5581 | 1.4122 | 0.9063 |
| `jolt_parity_pyramid/full_step/1` | n=1,2 | 15532.7 | 17499.4 | 1.1266 | 1.0626 | 0.9432 |
| `jolt_parity_pyramid/full_step/1` | n=1..4 | 15532.7 | 17499.4 | 1.1266 | 1.0807 | 0.9593 |
| `jolt_parity_pyramid/full_step/1` | n=1..4 excl. A2 | 15532.7 | 17499.4 | 1.1266 | 1.1030 | 0.9790 |
| `jolt_parity_pyramid/full_step/4` | n=1,2 | 14853.5 | 17494.6 | 1.1778 | 1.0714 | 0.9096 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | n=1,2 | 13510.2 | 17167.6 | 1.2707 | 1.2213 | 0.9611 |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | n=1..4 | 13510.2 | 17167.6 | 1.2707 | 1.2082 | 0.9508 |

- **The two arms timed different windows.**
  - `pyramid_sleeping_off`: every A run chose W=255, N=420 (steps 286–705), and every B run chose W=127, N=210 (steps 158–367). B also drew criterion's "Unable to complete 20 samples in 5.0s" warning in all 4 runs.
  - `full_step/4`: B1 chose linear d=2 where every other run chose d=3.
  - The row ratios above are therefore per arm and per run, as the build report's FLAG R2-1 / R2-2 prescribe.

## 2. Every run, compact index (execution order)

Column key:
- **before / after:** `process count, 10 s CPU avg, top consumer (cpu-s over the 10 s)`.
- **during:** whole-machine busy minus the bench process, where recorded.
- **mtime:** the LastWriteTime of the copied `estimates.json`.

The full per-run blocks (exe path, sha256, args, all five top processes, raw per-second samples, copy path) are in §3.

| # | entry | run | sha8 | id | median ms [95 % CI] | W / N | before | after | during | mtime |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | s6 | A1 | d5782d43 | `jolt_parity_pyramid/full_step/1` | 19.3185 [18.8033, 19.6802] | 255 / 420 | 234, 4.42 %, browser 2.047 | 234, 2.96 %, browser 0.766 | — | 21:05:35.798 |
| 2 | s6 | B1 | b74f7ee8 | `jolt_parity_pyramid/full_step/1` | 18.7951 [18.6170, 19.0201] | 255 / 420 | 235, 2.13 %, browser 0.875 | 235, 4.22 %, browser 1.953 | — | 21:07:10.293 |
| 3 | s6 | A2 | d5782d43 | `jolt_parity_pyramid/full_step/1` | 18.8567 [18.7348, 19.7413] | 255 / 420 | 235, 2.09 %, Taskmgr 0.891 | 235, 2.6 %, browser 1.125 | — | 21:08:00.507 |
| 4 | s6 | B2 | b74f7ee8 | `jolt_parity_pyramid/full_step/1` | 18.7534 [18.6676, 19.1779] | 255 / 420 | 235, 2.19 %, Taskmgr 0.859 | 234, 2.81 %, browser 1.328 | — | 21:08:45.500 |
| 5 | s6 | A1 | d5782d43 | `jolt_parity_pyramid/full_step/4` | 11.0775 [10.7964, 11.5013] | 511 / 630 | 234, 2.34 %, Taskmgr 0.562 | 233, 2.12 %, browser 0.812 | — | 21:09:31.098 |
| 6 | s6 | B1 | b74f7ee8 | `jolt_parity_pyramid/full_step/4` | 11.1455 [10.9003, 11.3677] | 511 / 630 | 233, 2.66 %, Taskmgr 0.766 | 233, 3.53 %, browser 0.922 | — | 21:10:12.178 |
| 7 | s6 | A2 | d5782d43 | `jolt_parity_pyramid/full_step/4` | 11.2249 [11.0172, 11.3982] | 511 / 630 | 233, 2.21 %, Taskmgr 0.547 | 233, 3.38 %, browser 1.672 | — | 21:10:53.368 |
| 8 | s6 | B2 | b74f7ee8 | `jolt_parity_pyramid/full_step/4` | 10.7047 [10.4838, 10.7967] | 511 / 630 | 233, 2.36 %, Taskmgr 0.672 | 233, 4.08 %, browser 2.344 | — | 21:11:33.826 |
| 9 | s6 | B1 | b74f7ee8 | `row_identity_churn/stable/sleeping_off` | 16.8461 [16.7924, 17.2986] | 255 / 420 | 233, 2.52 %, Taskmgr 0.859 | 233, 3.68 %, browser 2.062 | — | 21:12:13.221 |
| | | | | `row_identity_churn/swap_churn/sleeping_off` | 17.1893 [16.9129, 17.2769] | 255 / 420 | 〃 | 〃 | 〃 | 21:12:25.455 |
| | | | | `row_identity_churn/archetype_shift/sleeping_off` | 16.7894 [16.6635, 16.8962] | 255 / 420 | 〃 | 〃 | 〃 | 21:12:37.454 |
| | | | | `row_identity_churn/first_archetype_spawn/sleeping_off` | 16.6071 [16.4698, 16.8542] | 255 / 420 | 〃 | 〃 | 〃 | 21:12:49.429 |
| | | | | `row_identity_churn/burst_despawn/sleeping_off` | 16.5297 [16.4598, 16.6155] | 255 / 420 | 〃 | 〃 | 〃 | 21:13:01.364 |
| | | | | `row_identity_churn/burst_migrate/sleeping_off` | 16.8776 [16.6423, 16.9553] | 255 / 420 | 〃 | 〃 | 〃 | 21:13:13.559 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.6673 [16.3907, 16.7923] | 255 / 420 | 〃 | 〃 | 〃 | 21:13:25.532 |
| | | | | `row_identity_churn/swap_churn/sleeping_on` | 16.8403 [16.6251, 16.9525] | 255 / 420 | 〃 | 〃 | 〃 | 21:13:37.583 |
| | | | | `row_identity_churn/archetype_shift/sleeping_on` | 16.8685 [16.6187, 16.9509] | 255 / 420 | 〃 | 〃 | 〃 | 21:13:49.593 |
| | | | | `row_identity_churn/first_archetype_spawn/sleeping_on` | 16.6618 [16.5911, 16.7333] | 255 / 420 | 〃 | 〃 | 〃 | 21:14:01.602 |
| | | | | `row_identity_churn/burst_despawn/sleeping_on` | 16.5212 [16.5056, 16.5630] | 255 / 420 | 〃 | 〃 | 〃 | 21:14:13.626 |
| | | | | `row_identity_churn/burst_migrate/sleeping_on` | 16.6224 [16.4528, 16.7367] | 255 / 420 | 〃 | 〃 | 〃 | 21:14:25.667 |
| 10 | s6 | B2 | b74f7ee8 | `row_identity_churn/stable/sleeping_off` | 17.0567 [16.9193, 17.1965] | 255 / 420 | 233, 2.09 %, Taskmgr 0.688 | 232, 3.3 %, browser 0.75 | — | 21:15:04.000 |
| | | | | `row_identity_churn/swap_churn/sleeping_off` | 16.7919 [16.6352, 16.9195] | 255 / 420 | 〃 | 〃 | 〃 | 21:15:16.161 |
| | | | | `row_identity_churn/archetype_shift/sleeping_off` | 16.7695 [16.7126, 16.8007] | 255 / 420 | 〃 | 〃 | 〃 | 21:15:28.268 |
| | | | | `row_identity_churn/first_archetype_spawn/sleeping_off` | 17.0903 [17.0035, 17.2836] | 255 / 420 | 〃 | 〃 | 〃 | 21:15:40.549 |
| | | | | `row_identity_churn/burst_despawn/sleeping_off` | 17.1821 [17.0098, 17.2464] | 255 / 420 | 〃 | 〃 | 〃 | 21:15:52.846 |
| | | | | `row_identity_churn/burst_migrate/sleeping_off` | 16.9283 [16.6589, 17.0707] | 255 / 420 | 〃 | 〃 | 〃 | 21:16:05.141 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.6225 [16.5134, 16.7711] | 255 / 420 | 〃 | 〃 | 〃 | 21:16:17.208 |
| | | | | `row_identity_churn/swap_churn/sleeping_on` | 16.7267 [16.5829, 16.9986] | 255 / 420 | 〃 | 〃 | 〃 | 21:16:29.361 |
| | | | | `row_identity_churn/archetype_shift/sleeping_on` | 16.7642 [16.6909, 17.0826] | 255 / 420 | 〃 | 〃 | 〃 | 21:16:41.518 |
| | | | | `row_identity_churn/first_archetype_spawn/sleeping_on` | 16.7118 [16.6415, 16.9165] | 255 / 420 | 〃 | 〃 | 〃 | 21:16:53.670 |
| | | | | `row_identity_churn/burst_despawn/sleeping_on` | 16.7115 [16.5958, 16.8274] | 255 / 420 | 〃 | 〃 | 〃 | 21:17:05.693 |
| | | | | `row_identity_churn/burst_migrate/sleeping_on` | 16.7752 [16.7019, 16.8411] | 255 / 420 | 〃 | 〃 | 〃 | 21:17:17.921 |
| 11 | s7 | A1 | d552be05 | `jolt_parity_pyramid/full_step/1` | 19.0483 [18.7098, 19.4401] | 255 / 420 | 232, 3.29 %, browser 1.078 | 232, 3.76 %, browser 1.219 | — | 21:18:08.207 |
| 12 | s7 | B1 | a56007ab | `jolt_parity_pyramid/full_step/1` | 19.2060 [18.8113, 19.5227] | 255 / 420 | 232, 2.66 %, browser 0.719 | 232, 5.61 %, browser 2.141 | — | 21:18:53.481 |
| 13 | s7 | A2 | d552be05 | `jolt_parity_pyramid/full_step/1` | 19.3088 [19.1607, 19.5211] | 255 / 420 | 241, 2.19 %, browser 0.734 | 241, 3.65 %, browser 1.719 | — | 21:19:39.036 |
| 14 | s7 | B2 | a56007ab | `jolt_parity_pyramid/full_step/1` | 19.2486 [18.8739, 19.5230] | 255 / 420 | 241, 2.27 %, Taskmgr 0.688 | 236, 2.49 %, browser 1.266 | — | 21:20:24.586 |
| 15 | s7 | A1 | d552be05 | `jolt_parity_pyramid/full_step/4` | 10.7231 [10.5640, 11.1322] | 511 / 630 | 235, 1.98 %, Taskmgr 0.672 | 235, 2.9 %, browser 1.547 | — | 21:21:11.425 |
| 16 | s7 | B1 | a56007ab | `jolt_parity_pyramid/full_step/4` | 11.0553 [10.6272, 11.3009] | 511 / 630 | 235, 3.17 %, steamwebhelper 0.734 | 235, 4.34 %, browser 1.547 | — | 21:21:52.116 |
| 17 | s7 | A2 | d552be05 | `jolt_parity_pyramid/full_step/4` | 10.7650 [10.5423, 10.8925] | 511 / 630 | 235, 2.37 %, browser 0.641 | 235, 3.47 %, browser 1.703 | — | 21:22:32.529 |
| 18 | s7 | B2 | a56007ab | `jolt_parity_pyramid/full_step/4` | 10.6700 [10.5497, 11.4083] | 511 / 630 | 235, 2.57 %, browser 0.719 | 235, 3.04 %, browser 1.297 | — | 21:23:13.075 |
| 19 | s7 | A1 | d552be05 | `row_identity_churn/stable/sleeping_off` | 17.0243 [16.9429, 17.1030] | 255 / 420 | 235, 2.47 %, Taskmgr 0.734 | 235, 3.27 %, browser 0.609 | — | 21:23:52.883 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.7693 [16.5971, 17.0516] | 255 / 420 | 〃 | 〃 | 〃 | 21:24:08.352 |
| 20 | s7 | B1 | a56007ab | `row_identity_churn/stable/sleeping_off` | 17.1154 [16.9037, 17.3099] | 255 / 420 | 235, 2.45 %, browser 0.844 | 233, 2.39 %, steamwebhelper 0.922 | — | 21:24:46.290 |
| | | | | `row_identity_churn/stable/sleeping_on` | 17.0705 [16.8612, 17.2654] | 255 / 420 | 〃 | 〃 | 〃 | 21:25:01.844 |
| 21 | s7 | A2 | d552be05 | `row_identity_churn/stable/sleeping_off` | 16.9737 [16.8424, 17.0579] | 255 / 420 | 233, 1.93 %, Taskmgr 0.859 | 233, 2.28 %, Taskmgr 0.766 | — | 21:25:39.778 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.7577 [16.5950, 16.8782] | 255 / 420 | 〃 | 〃 | 〃 | 21:25:54.972 |
| 22 | s7 | B2 | a56007ab | `row_identity_churn/stable/sleeping_off` | 17.5472 [17.2591, 17.7277] | 255 / 420 | 233, 4.1 %, browser 1.859 | 233, 5.4 %, browser 1.922 | — | 21:26:33.231 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.7199 [16.6932, 16.8314] | 255 / 420 | 〃 | 〃 | 〃 | 21:26:48.648 |
| 23 | s7 | A1 | d552be05 | `soft_step_sp2/coupled/64` | 0.0873 [0.0872, 0.0873] | 65535 / 60600 | 233, 1.95 %, Taskmgr 0.891 | 233, 2.77 %, browser 1.25 | — | 21:27:34.185 |
| | | | | `soft_step_sp2/coupled/256` | 0.3835 [0.3833, 0.3841] | 16383 / 15150 | 〃 | 〃 | 〃 | 21:27:46.851 |
| 24 | s7 | B1 | a56007ab | `soft_step_sp2/coupled/64` | 0.0869 [0.0869, 0.0870] | 65535 / 60600 | 233, 1.96 %, Taskmgr 0.781 | 233, 2.27 %, browser 0.828 | — | 21:28:20.551 |
| | | | | `soft_step_sp2/coupled/256` | 0.3814 [0.3794, 0.3831] | 16383 / 15150 | 〃 | 〃 | 〃 | 21:28:33.170 |
| 25 | s7 | A2 | d552be05 | `soft_step_sp2/coupled/64` | 0.0874 [0.0873, 0.0876] | 65535 / 60600 | 233, 2.1 %, Taskmgr 0.688 | 233, 2.59 %, browser 0.734 | — | 21:29:06.945 |
| | | | | `soft_step_sp2/coupled/256` | 0.3815 [0.3806, 0.3817] | 16383 / 15150 | 〃 | 〃 | 〃 | 21:29:19.584 |
| 26 | s7 | B2 | a56007ab | `soft_step_sp2/coupled/64` | 0.0869 [0.0868, 0.0870] | 65535 / 60600 | 233, 2.27 %, Taskmgr 0.688 | 233, 2.68 %, Taskmgr 0.734 | — | 21:29:53.402 |
| | | | | `soft_step_sp2/coupled/256` | 0.3823 [0.3818, 0.3832] | 16383 / 15150 | 〃 | 〃 | 〃 | 21:30:06.083 |
| 27 | s8 | A1 | d552be05 | `sleeping_pipeline/pyramid_sleeping_off` | 17.7852 [17.6194, 17.9045] | 255 / 420 | 233, 2.02 %, Taskmgr 0.766 | 232, 3.49 %, browser 0.969 | — | 21:30:49.336 |
| | | | | `sleeping_pipeline/pyramid_awake_sleeping_on` | 16.7251 [16.6495, 16.8758] | 255 / 420 | 〃 | 〃 | 〃 | 21:31:00.733 |
| | | | | `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.6345 [5.5885, 5.6872] | 1023 / 1050 | 〃 | 〃 | 〃 | 21:31:12.511 |
| 28 | s8 | B1 | a56007ab | `sleeping_pipeline/pyramid_sleeping_off` | 18.1349 [17.8068, 18.2410] | 255 / 420 | 232, 2.77 %, browser 1.406 | 232, 2.09 %, browser 0.812 | — | 21:31:50.790 |
| | | | | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.3349 [17.2684, 17.5140] | 255 / 420 | 〃 | 〃 | 〃 | 21:32:02.583 |
| | | | | `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.4530 [5.3826, 5.5031] | 1023 / 1050 | 〃 | 〃 | 〃 | 21:32:14.080 |
| 29 | s8 | A2 | d552be05 | `sleeping_pipeline/pyramid_sleeping_off` | 17.8312 [17.7384, 18.1376] | 255 / 420 | 232, 3.05 %, browser 1.016 | 238, 1.88 %, Taskmgr 0.672 | — | 21:32:52.224 |
| | | | | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0295 [16.8810, 17.3224] | 255 / 420 | 〃 | 〃 | 〃 | 21:33:04.232 |
| | | | | `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.7020 [5.6252, 5.8399] | 511 / 840 | 〃 | 〃 | 〃 | 21:33:12.333 |
| 30 | s8 | B2 | a56007ab | `sleeping_pipeline/pyramid_sleeping_off` | 17.5806 [17.1683, 17.6954] | 255 / 420 | 238, 3.43 %, Taskmgr 0.766 | 236, 3.85 %, browser 2.016 | — | 21:33:50.259 |
| | | | | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.2416 [17.0465, 17.3979] | 255 / 420 | 〃 | 〃 | 〃 | 21:34:01.920 |
| | | | | `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.6311 [5.5640, 5.7218] | 1023 / 1050 | 〃 | 〃 | 〃 | 21:34:13.558 |
| 31 | s9 | A1 | 08fe7b9f | `sleeping_pipeline/pyramid_sleeping_off` | 18.8551 [18.5487, 18.9620] | 255 / 420 | 236, 2.4 %, Taskmgr 0.766 | 234, 2.71 %, browser 1.297 | — | 21:35:04.961 |
| 32 | s9 | B1 | 8d656ad8 | `sleeping_pipeline/pyramid_sleeping_off` | 26.1900 [25.9682, 26.4565] | 127 / 210 | 234, 3.42 %, Taskmgr 0.812 | 240, 1.86 %, Taskmgr 0.734 | — | 21:35:40.038 |
| 33 | s9 | A2 | 08fe7b9f | `sleeping_pipeline/pyramid_sleeping_off` | 18.4564 [18.3097, 18.6788] | 255 / 420 | 240, 0.6 %, Taskmgr 0.484 | 235, 0.94 %, Taskmgr 0.516 | — | 21:36:18.351 |
| 34 | s9 | B2 | 8d656ad8 | `sleeping_pipeline/pyramid_sleeping_off` | 25.2639 [24.8788, 25.4272] | 127 / 210 | 235, 0.6 %, Taskmgr 0.438 | 234, 0.97 %, Taskmgr 0.531 | — | 21:36:52.803 |
| 35 | s9 | A1 | 08fe7b9f | `jolt_parity_pyramid/full_step/1` | 18.6569 [18.3577, 18.9535] | 255 / 420 | 234, 1.17 %, Taskmgr 0.688 | 234, 1.52 %, Taskmgr 0.547 | — | 21:37:43.734 |
| 36 | s9 | B1 | 8d656ad8 | `jolt_parity_pyramid/full_step/1` | 20.7566 [20.5218, 21.2375] | 255 / 420 | 236, 2.49 %, browser 0.969 | 237, 2.36 %, browser 0.734 | — | 21:39:29.890 |
| 37 | s9 | A2 **SUSPECT** | 08fe7b9f | `jolt_parity_pyramid/full_step/1` | 20.6072 [19.9676, 21.2714] | 255 / 420 | 236, 1.91 %, Taskmgr 0.5 | 235, 16.66 %, browser 6.078 | — | 21:40:16.700 |
| 38 | s9 | B2 | 8d656ad8 | `jolt_parity_pyramid/full_step/1` | 20.9655 [20.2475, 21.5043] | 255 / 420 | 238, 3.05 %, browser 0.922 | 238, 2.44 %, browser 0.781 | — | 21:42:03.490 |
| 39 | s9 | A1 | 08fe7b9f | `jolt_parity_pyramid/full_step/4` | 10.8959 [10.7989, 11.2127] | 511 / 630 | 237, 2.87 %, Taskmgr 0.828 | 236, 3.07 %, browser 0.906 | 2.35 % | 21:43:53.353 |
| 40 | s9 | B1 | 8d656ad8 | `jolt_parity_pyramid/full_step/4` | 11.6027 [11.3750, 11.8843] | 511 / 420 | 236, 2.49 %, Taskmgr 0.812 | 236, 3.03 %, browser 1.375 | 2.99 % | 21:44:33.608 |
| 41 | s9 | A2 | 08fe7b9f | `jolt_parity_pyramid/full_step/4` | 11.0667 [10.7709, 11.4294] | 511 / 630 | 236, 2.35 %, Taskmgr 0.625 | 237, 3.71 %, browser 1.781 | 3.55 % | 21:45:15.359 |
| 42 | s9 | B2 | 8d656ad8 | `jolt_parity_pyramid/full_step/4` | 11.9273 [11.6345, 12.2241] | 255 / 630 | 237, 2.55 %, Taskmgr 0.75 | 235, 5.12 %, browser 1.547 | 1.72 % | 21:45:54.940 |
| 43 | s9 | A1 | 08fe7b9f | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.8255 [17.2878, 18.0856] | 255 / 420 | 235, 1.71 %, Taskmgr 0.516 | 235, 4.78 %, browser 2.219 | 2.03 % | 21:46:39.618 |
| 44 | s9 | B1 | 8d656ad8 | `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.5734 [21.4513, 21.7184] | 255 / 420 | 235, 2.65 %, Taskmgr 0.703 | 234, 4.02 %, browser 1.797 | 2.97 % | 21:47:21.229 |
| 45 | s9 | A2 | 08fe7b9f | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0423 [16.8530, 17.1835] | 255 / 420 | 234, 2.93 %, browser 1.016 | 234, 2.79 %, Taskmgr 0.719 | 2.82 % | 21:47:59.304 |
| 46 | s9 | B2 | 8d656ad8 | `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.0114 [20.8089, 21.5294] | 255 / 420 | 235, 3.05 %, browser 1.844 | 235, 2.16 %, Taskmgr 0.703 | 2.71 % | 21:48:40.565 |
| 47 | s9 | A3 (supp.) | 08fe7b9f | `jolt_parity_pyramid/full_step/1` | 19.4885 [19.2609, 19.7566] | 255 / 420 | 241, 2.09 %, Taskmgr 0.547 | 241, 2.39 %, browser 0.766 | 2.6 % | 21:49:45.509 |
| 48 | s9 | B3 (supp.) | 8d656ad8 | `jolt_parity_pyramid/full_step/1` | 21.1604 [20.9456, 21.7777] | 255 / 420 | 236, 2.81 %, browser 0.672 | 235, 2.58 %, browser 0.625 | 2.89 % | 21:50:33.732 |
| 49 | s9 | A4 (supp.) | 08fe7b9f | `jolt_parity_pyramid/full_step/1` | 18.9712 [18.7815, 19.1912] | 255 / 420 | 236, 1.56 %, Taskmgr 0.688 | 236, 2.36 %, Taskmgr 0.797 | 3.12 % | 21:51:19.598 |
| 50 | s9 | B4 (supp.) | 8d656ad8 | `jolt_parity_pyramid/full_step/1` | 21.1147 [20.7152, 21.6788] | 255 / 420 | 236, 2.59 %, browser 1.016 | 236, 2.4 %, Taskmgr 0.766 | 3.3 % | 21:52:07.686 |
| 51 | s9 | A3 (supp.) | 08fe7b9f | `sleeping_pipeline/pyramid_sleeping_off` | 18.9987 [18.6644, 19.1784] | 255 / 420 | 236, 3.15 %, browser 0.953 | 236, 1.78 %, Taskmgr 0.672 | 2.98 % | 21:52:52.500 |
| 52 | s9 | B3 (supp.) | 8d656ad8 | `sleeping_pipeline/pyramid_sleeping_off` | 27.2549 [26.9366, 27.4068] | 127 / 210 | 236, 1.98 %, browser 0.875 | 236, 2.41 %, browser 0.625 | 4.1 % | 21:53:28.425 |
| 53 | s9 | A4 (supp.) | 08fe7b9f | `sleeping_pipeline/pyramid_sleeping_off` | 18.5245 [18.4086, 18.6602] | 255 / 420 | 236, 1.7 %, Taskmgr 0.641 | 236, 2.9 %, browser 0.844 | 3.28 % | 21:54:07.709 |
| 54 | s9 | B4 (supp.) | 8d656ad8 | `sleeping_pipeline/pyramid_sleeping_off` | 26.9695 [26.5798, 27.1435] | 127 / 210 | 236, 2.06 %, browser 0.969 | 236, 3.69 %, browser 1.703 | 2.79 % | 21:54:43.533 |
| 55 | s9 | A3 (supp.) | 08fe7b9f | `sleeping_pipeline/pyramid_awake_sleeping_on` | 18.1057 [17.7532, 18.4449] | 255 / 420 | 235, 2.12 %, Taskmgr 0.578 | 235, 3.02 %, browser 1.547 | 2.8 % | 21:55:30.586 |
| 56 | s9 | B3 (supp.) | 8d656ad8 | `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.8375 [21.5864, 22.2428] | 255 / 420 | 235, 2.25 %, browser 0.609 | 234, 3.8 %, browser 2.312 | 2.71 % | 21:56:12.249 |
| 57 | s9 | A4 (supp.) | 08fe7b9f | `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0879 [16.8841, 17.6053] | 255 / 420 | 234, 2.25 %, browser 0.688 | 234, 3.96 %, browser 1.844 | 2.69 % | 21:56:50.471 |
| 58 | s9 | B4 (supp.) | 8d656ad8 | `sleeping_pipeline/pyramid_awake_sleeping_on` | 20.2263 [20.0906, 20.5511] | 255 / 420 | 234, 2.42 %, browser 0.891 | 234, 3.6 %, Taskmgr 0.891 | 3.08 % | 21:57:31.188 |
| 59 | s7 | A3 (supp.) | d552be05 | `row_identity_churn/stable/sleeping_off` | 17.1108 [17.0202, 17.3561] | 255 / 420 | 234, 3.07 %, browser 1.281 | 234, 3.62 %, browser 1.281 | 2.7 % | 21:58:11.266 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.6816 [16.5984, 16.8261] | 255 / 420 | 〃 | 〃 | 〃 | 21:58:26.464 |
| 60 | s7 | B3 (supp.) | a56007ab | `row_identity_churn/stable/sleeping_off` | 17.1838 [16.7336, 17.3525] | 255 / 420 | 234, 1.78 %, browser 0.703 | 234, 1.75 %, Taskmgr 0.609 | 3.32 % | 21:59:05.153 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.6283 [16.5130, 16.7549] | 255 / 420 | 〃 | 〃 | 〃 | 21:59:20.290 |
| 61 | s7 | A4 (supp.) | d552be05 | `row_identity_churn/stable/sleeping_off` | 16.9773 [16.8751, 17.1294] | 255 / 420 | 234, 1.87 %, Taskmgr 0.656 | 234, 2.62 %, Taskmgr 0.609 | 3.79 % | 21:59:58.750 |
| | | | | `row_identity_churn/stable/sleeping_on` | 16.8479 [16.7040, 17.0540] | 255 / 420 | 〃 | 〃 | 〃 | 22:00:14.295 |
| 62 | s7 | B4 (supp.) | a56007ab | `row_identity_churn/stable/sleeping_off` | 16.7985 [16.6330, 16.9542] | 255 / 420 | 234, 3.72 %, browser 0.938 | 233, 3.25 %, browser 0.844 | 2.33 % | 22:00:52.668 |
| | | | | `row_identity_churn/stable/sleeping_on` | 17.1120 [16.8215, 17.2640] | 255 / 420 | 〃 | 〃 | 〃 | 22:01:08.183 |

## 3. Every run in full, in execution order

#### s6 · full_step_1 · A1 · d5782d43
- exe `D:/wt/_targets/mq-d5782d43/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `9dff38a51a5e5dc5…` (verified before the run); cwd `D:/wt/mq-d5782d43/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d5782d43`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:05:12.736+03:00 → 2026-09-18T21:05:35.806+03:00 (23.07 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:05:01.786+03:00: procs 234, CPU 10 s avg **4.42 %** (samples [5.84, 9.47, 9.17, 3.79, 2.67, 2.46, 1.95, 2.2, 2.34, 4.3]), top5 [browser 2.047, browser 1.141, Taskmgr 0.719, claude 0.484, powershell 0.469], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:05:36.047+03:00: procs 234, CPU 10 s avg **2.96 %** (samples [4.03, 3.89, 2.24, 1.29, 0.99, 1.76, 2.63, 2.74, 4.75, 5.31]), top5 [browser 0.766, Taskmgr 0.641, browser 0.578, audiodg 0.453, powershell 0.391], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.3185 | [18.8033, 19.6802] | 18.9427 | 19.4596 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:05:35.798+03:00 | `raw/s6/A1/jolt_parity_pyramid__full_step__1` |

#### s6 · full_step_1 · B1 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `3bbbd97325e1aa21…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:06:47.545+03:00 → 2026-09-18T21:07:10.300+03:00 (22.75 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:06:36.597+03:00: procs 235, CPU 10 s avg **2.13 %** (samples [4.46, 2.06, 1.28, 1.72, 1.61, 1.7, 0.89, 1.64, 3.72, 2.25]), top5 [browser 0.875, Taskmgr 0.766, browser 0.547, powershell 0.484, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:07:10.526+03:00: procs 235, CPU 10 s avg **4.22 %** (samples [4.93, 3.76, 7.37, 6.71, 7.18, 4.08, 4.08, 2.72, 0.52, 0.9]), top5 [browser 1.953, browser 1.297, Taskmgr 0.625, claude 0.484, powershell 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.7951 | [18.6170, 19.0201] | 18.6285 | 19.1198 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:07:10.293+03:00 | `raw/s6/B1/jolt_parity_pyramid__full_step__1` |

#### s6 · full_step_1 · A2 · d5782d43
- exe `D:/wt/_targets/mq-d5782d43/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `9dff38a51a5e5dc5…` (verified before the run); cwd `D:/wt/mq-d5782d43/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d5782d43`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:07:37.590+03:00 → 2026-09-18T21:08:00.536+03:00 (22.95 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:07:26.672+03:00: procs 235, CPU 10 s avg **2.09 %** (samples [5.08, 3.12, 1.82, 1.68, 2.26, 1.77, 1.38, 1.56, 1.29, 0.91]), top5 [Taskmgr 0.891, browser 0.453, browser 0.422, powershell 0.406, steamwebhelper 0.25], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:08:00.766+03:00: procs 235, CPU 10 s avg **2.6 %** (samples [2.28, 2.3, 2.05, 3.62, 3.01, 3.45, 3.19, 3.62, 1.86, 0.61]), top5 [browser 1.125, Taskmgr 0.75, browser 0.719, powershell 0.359, audiodg 0.328], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.8567 | [18.7348, 19.7413] | 18.7698 | 19.3720 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:08:00.507+03:00 | `raw/s6/A2/jolt_parity_pyramid__full_step__1` |

#### s6 · full_step_1 · B2 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `3bbbd97325e1aa21…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:08:22.870+03:00 → 2026-09-18T21:08:45.532+03:00 (22.66 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:08:11.976+03:00: procs 235, CPU 10 s avg **2.19 %** (samples [0, 5.52, 1.5, 1.75, 2.22, 1.39, 4.47, 1.09, 1.81, 2.14]), top5 [Taskmgr 0.859, browser 0.422, powershell 0.406, browser 0.391, steamwebhelper 0.281], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:08:45.771+03:00: procs 234, CPU 10 s avg **2.81 %** (samples [3.12, 4.18, 3.59, 2.2, 4.36, 1.71, 3.12, 1.18, 1.42, 3.17]), top5 [browser 1.328, Taskmgr 0.734, browser 0.688, powershell 0.359, steamwebhelper 0.281], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.7534 | [18.6676, 19.1779] | 18.6369 | 18.9258 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:08:45.500+03:00 | `raw/s6/B2/jolt_parity_pyramid__full_step__1` |

#### s6 · full_step_4 · A1 · d5782d43
- exe `D:/wt/_targets/mq-d5782d43/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `9dff38a51a5e5dc5…` (verified before the run); cwd `D:/wt/mq-d5782d43/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d5782d43`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:09:12.551+03:00 → 2026-09-18T21:09:31.107+03:00 (18.56 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:09:01.663+03:00: procs 234, CPU 10 s avg **2.34 %** (samples [2.07, 2.54, 1.61, 2.21, 1.51, 4.14, 2.45, 2.25, 2.67, 1.94]), top5 [Taskmgr 0.562, powershell 0.422, browser 0.344, steamwebhelper 0.281, browser 0.141], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:09:31.340+03:00: procs 233, CPU 10 s avg **2.12 %** (samples [2.14, 1.08, 1.09, 3.88, 3.12, 2.45, 1.9, 2.07, 1.99, 1.44]), top5 [browser 0.812, browser 0.656, Taskmgr 0.578, powershell 0.391, steamwebhelper 0.203], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.0775 | [10.7964, 11.5013] | 11.3026 | 11.1414 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:09:31.098+03:00 | `raw/s6/A1/jolt_parity_pyramid__full_step__4` |

#### s6 · full_step_4 · B1 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `3bbbd97325e1aa21…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:09:53.449+03:00 → 2026-09-18T21:10:12.186+03:00 (18.74 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:09:42.560+03:00: procs 233, CPU 10 s avg **2.66 %** (samples [3.55, 1.76, 4.27, 2.44, 2.15, 1.9, 2.01, 4.19, 3.71, 0.64]), top5 [Taskmgr 0.766, browser 0.406, steamwebhelper 0.391, powershell 0.391, browser 0.188], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:10:12.412+03:00: procs 233, CPU 10 s avg **3.53 %** (samples [5.32, 5.18, 3.21, 1.38, 3.89, 2.88, 1.81, 4.47, 3.51, 3.7]), top5 [browser 0.922, browser 0.844, Taskmgr 0.734, powershell 0.422, audiodg 0.359], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.1455 | [10.9003, 11.3677] | 11.1987 | 11.1723 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:10:12.178+03:00 | `raw/s6/B1/jolt_parity_pyramid__full_step__4` |

#### s6 · full_step_4 · A2 · d5782d43
- exe `D:/wt/_targets/mq-d5782d43/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `9dff38a51a5e5dc5…` (verified before the run); cwd `D:/wt/mq-d5782d43/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d5782d43`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:10:34.512+03:00 → 2026-09-18T21:10:53.397+03:00 (18.88 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:10:23.623+03:00: procs 233, CPU 10 s avg **2.21 %** (samples [2.5, 2.44, 1.96, 3.41, 3.21, 2.92, 2.54, 1.76, 0.49, 0.86]), top5 [Taskmgr 0.547, browser 0.531, powershell 0.422, browser 0.375, claude 0.234], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:10:53.625+03:00: procs 233, CPU 10 s avg **3.38 %** (samples [5.41, 2.86, 3.67, 2.34, 3.03, 2.83, 2.63, 4.66, 2.64, 3.73]), top5 [browser 1.672, browser 0.984, Taskmgr 0.688, powershell 0.469, steamwebhelper 0.312], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.2249 | [11.0172, 11.3982] | 11.2948 | 11.2731 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:10:53.368+03:00 | `raw/s6/A2/jolt_parity_pyramid__full_step__4` |

#### s6 · full_step_4 · B2 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `3bbbd97325e1aa21…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:11:15.734+03:00 → 2026-09-18T21:11:33.855+03:00 (18.12 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:11:04.852+03:00: procs 233, CPU 10 s avg **2.36 %** (samples [1.28, 2.71, 2.78, 2.16, 6.25, 2.44, 3.29, 1.84, 0.29, 0.51]), top5 [Taskmgr 0.672, browser 0.516, browser 0.484, powershell 0.375, steamwebhelper 0.203], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:11:34.087+03:00: procs 233, CPU 10 s avg **4.08 %** (samples [3.01, 5.62, 2.08, 4.93, 5.18, 4.69, 4.7, 3.75, 3.74, 3.08]), top5 [browser 2.344, browser 0.828, Taskmgr 0.625, powershell 0.422, steamwebhelper 0.406], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.7047 | [10.4838, 10.7967] | 10.7869 | 10.6852 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:11:33.826+03:00 | `raw/s6/B2/jolt_parity_pyramid__full_step__4` |

#### s6 · row_identity_churn_all · B1 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `bf089a1f9cc50392…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:12:00.981+03:00 → 2026-09-18T21:14:25.680+03:00 (144.7 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/swap_churn/sleeping_off`, `row_identity_churn/archetype_shift/sleeping_off`, `row_identity_churn/first_archetype_spawn/sleeping_off`, `row_identity_churn/burst_despawn/sleeping_off`, `row_identity_churn/burst_migrate/sleeping_off`, `row_identity_churn/stable/sleeping_on`, `row_identity_churn/swap_churn/sleeping_on`, `row_identity_churn/archetype_shift/sleeping_on`, `row_identity_churn/first_archetype_spawn/sleeping_on`, `row_identity_churn/burst_despawn/sleeping_on`, `row_identity_churn/burst_migrate/sleeping_on`
- receipt BEFORE 2026-09-18T21:11:50.054+03:00: procs 233, CPU 10 s avg **2.52 %** (samples [2.41, 2.01, 4.66, 3.15, 1.77, 2.98, 2.68, 1.67, 1.48, 2.36]), top5 [Taskmgr 0.859, browser 0.547, browser 0.469, steamwebhelper 0.422, powershell 0.406], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:14:25.918+03:00: procs 233, CPU 10 s avg **3.68 %** (samples [3.87, 2.47, 1.57, 1.09, 2.7, 4.56, 7.17, 5.85, 4.53, 2.98]), top5 [browser 2.062, browser 1.016, Taskmgr 0.656, powershell 0.438, steamwebhelper 0.297], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 16.8461 | [16.7924, 17.2986] | 17.2468 | 17.0183 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:12:13.221+03:00 | `raw/s6/B1/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/swap_churn/sleeping_off` | 17.1893 | [16.9129, 17.2769] | 17.1199 | 17.0577 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:12:25.455+03:00 | `raw/s6/B1/row_identity_churn__swap_churn__sleeping_off` |
| `row_identity_churn/archetype_shift/sleeping_off` | 16.7894 | [16.6635, 16.8962] | 16.6912 | 16.8120 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:12:37.454+03:00 | `raw/s6/B1/row_identity_churn__archetype_shift__sleeping_off` |
| `row_identity_churn/first_archetype_spawn/sleeping_off` | 16.6071 | [16.4698, 16.8542] | 16.5445 | 16.6495 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:12:49.429+03:00 | `raw/s6/B1/row_identity_churn__first_archetype_spawn__sleeping_off` |
| `row_identity_churn/burst_despawn/sleeping_off` | 16.5297 | [16.4598, 16.6155] | 16.5008 | 16.5459 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:13:01.364+03:00 | `raw/s6/B1/row_identity_churn__burst_despawn__sleeping_off` |
| `row_identity_churn/burst_migrate/sleeping_off` | 16.8776 | [16.6423, 16.9553] | 16.8956 | 16.7941 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:13:13.559+03:00 | `raw/s6/B1/row_identity_churn__burst_migrate__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.6673 | [16.3907, 16.7923] | 16.7224 | 16.6289 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:13:25.532+03:00 | `raw/s6/B1/row_identity_churn__stable__sleeping_on` |
| `row_identity_churn/swap_churn/sleeping_on` | 16.8403 | [16.6251, 16.9525] | 16.8773 | 16.8149 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:13:37.583+03:00 | `raw/s6/B1/row_identity_churn__swap_churn__sleeping_on` |
| `row_identity_churn/archetype_shift/sleeping_on` | 16.8685 | [16.6187, 16.9509] | 16.6459 | 16.8026 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:13:49.593+03:00 | `raw/s6/B1/row_identity_churn__archetype_shift__sleeping_on` |
| `row_identity_churn/first_archetype_spawn/sleeping_on` | 16.6618 | [16.5911, 16.7333] | 16.5596 | 16.7166 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:14:01.602+03:00 | `raw/s6/B1/row_identity_churn__first_archetype_spawn__sleeping_on` |
| `row_identity_churn/burst_despawn/sleeping_on` | 16.5212 | [16.5056, 16.5630] | 16.5461 | 16.5392 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:14:13.626+03:00 | `raw/s6/B1/row_identity_churn__burst_despawn__sleeping_on` |
| `row_identity_churn/burst_migrate/sleeping_on` | 16.6224 | [16.4528, 16.7367] | 16.7438 | 16.6025 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:14:25.667+03:00 | `raw/s6/B1/row_identity_churn__burst_migrate__sleeping_on` |

#### s6 · row_identity_churn_all · B2 · b74f7ee8
- exe `D:/wt/_targets/mq-b74f7ee8/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `bf089a1f9cc50392…` (verified before the run); cwd `D:/wt/mq-b74f7ee8/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-b74f7ee8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:14:51.754+03:00 → 2026-09-18T21:17:17.957+03:00 (146.2 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/swap_churn/sleeping_off`, `row_identity_churn/archetype_shift/sleeping_off`, `row_identity_churn/first_archetype_spawn/sleeping_off`, `row_identity_churn/burst_despawn/sleeping_off`, `row_identity_churn/burst_migrate/sleeping_off`, `row_identity_churn/stable/sleeping_on`, `row_identity_churn/swap_churn/sleeping_on`, `row_identity_churn/archetype_shift/sleeping_on`, `row_identity_churn/first_archetype_spawn/sleeping_on`, `row_identity_churn/burst_despawn/sleeping_on`, `row_identity_churn/burst_migrate/sleeping_on`
- receipt BEFORE 2026-09-18T21:14:40.878+03:00: procs 233, CPU 10 s avg **2.09 %** (samples [2.35, 2.15, 1.92, 1.13, 3.98, 4.52, 1.86, 1.86, 0.33, 0.77]), top5 [Taskmgr 0.688, browser 0.578, browser 0.562, powershell 0.469, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:17:18.191+03:00: procs 232, CPU 10 s avg **3.3 %** (samples [2.3, 6.72, 2.44, 1.44, 0.78, 0.54, 1.64, 3.85, 5.82, 7.52]), top5 [browser 0.75, browser 0.688, Taskmgr 0.578, powershell 0.391, audiodg 0.359], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.0567 | [16.9193, 17.1965] | 17.0671 | 17.0517 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:15:04.000+03:00 | `raw/s6/B2/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/swap_churn/sleeping_off` | 16.7919 | [16.6352, 16.9195] | 16.8921 | 16.7850 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:15:16.161+03:00 | `raw/s6/B2/row_identity_churn__swap_churn__sleeping_off` |
| `row_identity_churn/archetype_shift/sleeping_off` | 16.7695 | [16.7126, 16.8007] | 16.8202 | 16.7654 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:15:28.268+03:00 | `raw/s6/B2/row_identity_churn__archetype_shift__sleeping_off` |
| `row_identity_churn/first_archetype_spawn/sleeping_off` | 17.0903 | [17.0035, 17.2836] | 17.1626 | 17.1490 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:15:40.549+03:00 | `raw/s6/B2/row_identity_churn__first_archetype_spawn__sleeping_off` |
| `row_identity_churn/burst_despawn/sleeping_off` | 17.1821 | [17.0098, 17.2464] | 17.0532 | 17.1536 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:15:52.846+03:00 | `raw/s6/B2/row_identity_churn__burst_despawn__sleeping_off` |
| `row_identity_churn/burst_migrate/sleeping_off` | 16.9283 | [16.6589, 17.0707] | 16.8909 | 16.9045 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:16:05.141+03:00 | `raw/s6/B2/row_identity_churn__burst_migrate__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.6225 | [16.5134, 16.7711] | 16.7419 | 16.6644 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:16:17.208+03:00 | `raw/s6/B2/row_identity_churn__stable__sleeping_on` |
| `row_identity_churn/swap_churn/sleeping_on` | 16.7267 | [16.5829, 16.9986] | 17.0281 | 16.8300 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:16:29.361+03:00 | `raw/s6/B2/row_identity_churn__swap_churn__sleeping_on` |
| `row_identity_churn/archetype_shift/sleeping_on` | 16.7642 | [16.6909, 17.0826] | 16.8680 | 16.8599 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:16:41.518+03:00 | `raw/s6/B2/row_identity_churn__archetype_shift__sleeping_on` |
| `row_identity_churn/first_archetype_spawn/sleeping_on` | 16.7118 | [16.6415, 16.9165] | 16.6613 | 16.8034 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:16:53.670+03:00 | `raw/s6/B2/row_identity_churn__first_archetype_spawn__sleeping_on` |
| `row_identity_churn/burst_despawn/sleeping_on` | 16.7115 | [16.5958, 16.8274] | 16.6164 | 16.7354 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:17:05.693+03:00 | `raw/s6/B2/row_identity_churn__burst_despawn__sleeping_on` |
| `row_identity_churn/burst_migrate/sleeping_on` | 16.7752 | [16.7019, 16.8411] | 16.7759 | 16.7693 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:17:17.921+03:00 | `raw/s6/B2/row_identity_churn__burst_migrate__sleeping_on` |

#### s7 · full_step_1 · A1 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `aa70f3e266f0055e…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:17:45.413+03:00 → 2026-09-18T21:18:08.214+03:00 (22.8 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:17:34.513+03:00: procs 232, CPU 10 s avg **3.29 %** (samples [5.06, 3.71, 4.27, 3.5, 2.46, 3.13, 2.41, 2.06, 1.99, 4.27]), top5 [browser 1.078, Taskmgr 0.719, browser 0.484, powershell 0.438, steamwebhelper 0.297], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:18:08.450+03:00: procs 232, CPU 10 s avg **3.76 %** (samples [6.07, 4.28, 4.9, 2.53, 2.81, 2.54, 5.11, 0.63, 2.86, 5.86]), top5 [browser 1.219, browser 1.094, Taskmgr 0.609, powershell 0.422, steamwebhelper 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.0483 | [18.7098, 19.4401] | 18.8874 | 19.2226 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:18:08.207+03:00 | `raw/s7/A1/jolt_parity_pyramid__full_step__1` |

#### s7 · full_step_1 · B1 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `662a96852d1d44a2…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:18:30.629+03:00 → 2026-09-18T21:18:53.489+03:00 (22.86 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:18:19.712+03:00: procs 232, CPU 10 s avg **2.66 %** (samples [4.4, 3.55, 3.12, 2.64, 1.88, 2.58, 0.8, 2.17, 1.69, 3.76]), top5 [browser 0.719, browser 0.719, Taskmgr 0.531, powershell 0.344, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:18:53.715+03:00: procs 232, CPU 10 s avg **5.61 %** (samples [4.15, 13.85, 3.88, 3.77, 7.62, 6.78, 4.83, 3.85, 4.53, 2.89]), top5 [browser 2.141, browser 1.031, Taskmgr 0.734, powershell 0.344, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.2060 | [18.8113, 19.5227] | 18.9563 | 19.2916 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:18:53.481+03:00 | `raw/s7/B1/jolt_parity_pyramid__full_step__1` |

#### s7 · full_step_1 · A2 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `aa70f3e266f0055e…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:19:15.830+03:00 → 2026-09-18T21:19:39.065+03:00 (23.23 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:19:04.937+03:00: procs 241, CPU 10 s avg **2.19 %** (samples [2.23, 3.21, 2.35, 3.13, 3.42, 1.74, 2.14, 2.25, 0.6, 0.78]), top5 [browser 0.734, browser 0.625, Taskmgr 0.594, powershell 0.391, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:19:39.307+03:00: procs 241, CPU 10 s avg **3.65 %** (samples [6.53, 4.87, 5.61, 6.49, 3.03, 3.02, 0.8, 1.67, 1.37, 3.15]), top5 [browser 1.719, browser 1.078, Taskmgr 0.609, powershell 0.375, claude 0.281], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.3088 | [19.1607, 19.5211] | 19.2160 | 19.5257 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:19:39.036+03:00 | `raw/s7/A2/jolt_parity_pyramid__full_step__1` |

#### s7 · full_step_1 · B2 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `662a96852d1d44a2…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:20:01.516+03:00 → 2026-09-18T21:20:24.619+03:00 (23.1 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:19:50.561+03:00: procs 241, CPU 10 s avg **2.27 %** (samples [3.28, 2.15, 3.51, 2.73, 1.67, 0.6, 1.5, 0.77, 1.38, 5.13]), top5 [Taskmgr 0.688, browser 0.547, browser 0.531, powershell 0.391, steamwebhelper 0.203], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:20:24.847+03:00: procs 236, CPU 10 s avg **2.49 %** (samples [3.28, 1.85, 1.24, 0.97, 4.67, 2.98, 2.18, 2.14, 1.75, 3.83]), top5 [browser 1.266, browser 0.906, Taskmgr 0.625, steamwebhelper 0.609, powershell 0.438], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.2486 | [18.8739, 19.5230] | 19.0964 | 19.4827 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:20:24.586+03:00 | `raw/s7/B2/jolt_parity_pyramid__full_step__1` |

#### s7 · full_step_4 · A1 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `aa70f3e266f0055e…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:20:53.601+03:00 → 2026-09-18T21:21:11.436+03:00 (17.84 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:20:42.701+03:00: procs 235, CPU 10 s avg **1.98 %** (samples [2.15, 3, 1.87, 0.89, 1.48, 3.12, 2.44, 2, 2.11, 0.77]), top5 [Taskmgr 0.672, audiodg 0.453, powershell 0.391, browser 0.391, browser 0.203], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:21:11.679+03:00: procs 235, CPU 10 s avg **2.9 %** (samples [3.98, 2.56, 2.99, 2.44, 3.63, 3.47, 1.22, 1.63, 4.75, 2.35]), top5 [browser 1.547, browser 0.75, Taskmgr 0.578, steamwebhelper 0.391, powershell 0.359], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.7231 | [10.5640, 11.1322] | 10.5694 | 10.8508 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:21:11.425+03:00 | `raw/s7/A1/jolt_parity_pyramid__full_step__4` |

#### s7 · full_step_4 · B1 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `662a96852d1d44a2…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:21:33.882+03:00 → 2026-09-18T21:21:52.125+03:00 (18.24 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:21:22.946+03:00: procs 235, CPU 10 s avg **3.17 %** (samples [2.55, 4.06, 1.98, 3.12, 3.5, 3.03, 3.3, 6.67, 1.47, 1.97]), top5 [steamwebhelper 0.734, Taskmgr 0.719, powershell 0.406, browser 0.406, claude 0.281], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:21:52.365+03:00: procs 235, CPU 10 s avg **4.34 %** (samples [10.67, 8.3, 4.17, 2.63, 1.08, 2.61, 1.22, 2.34, 6.2, 4.18]), top5 [browser 1.547, browser 0.922, steamwebhelper 0.766, Taskmgr 0.656, powershell 0.438], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.0553 | [10.6272, 11.3009] | 10.7571 | 10.9624 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:21:52.116+03:00 | `raw/s7/B1/jolt_parity_pyramid__full_step__4` |

#### s7 · full_step_4 · A2 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `aa70f3e266f0055e…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:22:14.584+03:00 → 2026-09-18T21:22:32.560+03:00 (17.98 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:22:03.672+03:00: procs 235, CPU 10 s avg **2.37 %** (samples [2.41, 2.15, 3.02, 2.93, 0, 4.36, 3.13, 1.5, 2.41, 1.77]), top5 [browser 0.641, Taskmgr 0.609, browser 0.516, powershell 0.438, steamwebhelper 0.234], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:22:32.797+03:00: procs 235, CPU 10 s avg **3.47 %** (samples [2.5, 7.14, 5.73, 6.49, 2.49, 1.8, 1.73, 0.68, 2.25, 3.93]), top5 [browser 1.703, browser 1.328, Taskmgr 0.484, powershell 0.438, claude 0.359], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.7650 | [10.5423, 10.8925] | 10.5872 | 10.7725 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:22:32.529+03:00 | `raw/s7/A2/jolt_parity_pyramid__full_step__4` |

#### s7 · full_step_4 · B2 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `662a96852d1d44a2…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:22:55.017+03:00 → 2026-09-18T21:23:13.106+03:00 (18.09 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:22:44.079+03:00: procs 235, CPU 10 s avg **2.57 %** (samples [3.25, 1.95, 3.31, 3.59, 2.34, 1.86, 1.63, 2.38, 1.99, 3.36]), top5 [browser 0.719, browser 0.703, Taskmgr 0.688, powershell 0.688, claude 0.203], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:23:13.347+03:00: procs 235, CPU 10 s avg **3.04 %** (samples [2.74, 1.86, 1.28, 4.18, 5.64, 4.35, 3.87, 2.86, 1.7, 1.93]), top5 [browser 1.297, browser 0.922, Taskmgr 0.625, powershell 0.391, steamwebhelper 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.6700 | [10.5497, 11.4083] | 10.7129 | 10.9599 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:23:13.075+03:00 | `raw/s7/B2/jolt_parity_pyramid__full_step__4` |

#### s7 · churn_stable · A1 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `6e37d7db2cb3110b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:23:40.631+03:00 → 2026-09-18T21:24:11.629+03:00 (31.0 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:23:29.732+03:00: procs 235, CPU 10 s avg **2.47 %** (samples [1.28, 3.21, 2.23, 2.17, 1.96, 3.02, 1.95, 5.28, 1.83, 1.8]), top5 [Taskmgr 0.734, browser 0.625, browser 0.531, powershell 0.391, steamwebhelper 0.281], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:24:11.863+03:00: procs 235, CPU 10 s avg **3.27 %** (samples [1.57, 2.01, 3.14, 4.94, 5.97, 5.62, 2.3, 1.19, 1.57, 4.37]), top5 [browser 0.609, steamwebhelper 0.609, Taskmgr 0.562, powershell 0.484, browser 0.469], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.0243 | [16.9429, 17.1030] | 16.9445 | 17.0584 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:23:52.883+03:00 | `raw/s7/A1/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.7693 | [16.5971, 17.0516] | 17.0505 | 16.8334 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:24:08.352+03:00 | `raw/s7/A1/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · B1 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `48e71cca5e94674b…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:24:33.949+03:00 → 2026-09-18T21:25:05.119+03:00 (31.17 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:24:23.111+03:00: procs 235, CPU 10 s avg **2.45 %** (samples [2.22, 2.8, 1.66, 1.57, 1.27, 5.91, 2.38, 1.82, 1.17, 3.69]), top5 [browser 0.844, Taskmgr 0.656, powershell 0.359, browser 0.359, steamwebhelper 0.219], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:25:05.345+03:00: procs 233, CPU 10 s avg **2.39 %** (samples [1.53, 1.59, 1.87, 3.13, 3.42, 2.06, 2.39, 1.67, 5.88, 0.35]), top5 [steamwebhelper 0.922, Taskmgr 0.688, browser 0.516, powershell 0.438, browser 0.297], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.1154 | [16.9037, 17.3099] | 17.1651 | 17.1079 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:24:46.290+03:00 | `raw/s7/B1/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 17.0705 | [16.8612, 17.2654] | 17.1271 | 17.0868 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:25:01.844+03:00 | `raw/s7/B1/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · A2 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `6e37d7db2cb3110b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:25:27.577+03:00 → 2026-09-18T21:25:58.218+03:00 (30.64 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:25:16.644+03:00: procs 233, CPU 10 s avg **1.93 %** (samples [0.72, 2.77, 4.24, 1.04, 0.75, 1.48, 1.76, 2.73, 1.48, 2.38]), top5 [Taskmgr 0.859, powershell 0.328, browser 0.281, audiodg 0.281, browser 0.141], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:25:58.459+03:00: procs 233, CPU 10 s avg **2.28 %** (samples [3.87, 4.81, 2.42, 1.62, 1.88, 1.47, 0, 1.74, 1.14, 3.8]), top5 [Taskmgr 0.766, browser 0.641, powershell 0.391, steamwebhelper 0.297, browser 0.25], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 16.9737 | [16.8424, 17.0579] | 17.0918 | 16.9726 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:25:39.778+03:00 | `raw/s7/A2/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.7577 | [16.5950, 16.8782] | 16.6005 | 16.7473 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:25:54.972+03:00 | `raw/s7/A2/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · B2 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `48e71cca5e94674b…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:26:20.680+03:00 → 2026-09-18T21:26:51.982+03:00 (31.3 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:26:09.733+03:00: procs 233, CPU 10 s avg **4.1 %** (samples [0.01, 0.92, 2.79, 7.16, 5.27, 6.27, 5.34, 4.37, 4.89, 3.94]), top5 [browser 1.859, browser 1.094, Taskmgr 0.656, powershell 0.375, steamwebhelper 0.344], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:26:52.224+03:00: procs 233, CPU 10 s avg **5.4 %** (samples [5.9, 2.83, 1.86, 3.51, 4.56, 11.81, 5.83, 8.96, 4.84, 3.85]), top5 [browser 1.922, steamwebhelper 1.062, browser 0.844, Taskmgr 0.75, powershell 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.5472 | [17.2591, 17.7277] | 17.5425 | 17.4958 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:26:33.231+03:00 | `raw/s7/B2/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.7199 | [16.6932, 16.8314] | 16.7302 | 16.7775 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:26:48.648+03:00 | `raw/s7/B2/row_identity_churn__stable__sleeping_on` |

#### s7 · soft_coupled · A1 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/soft_step_sp2-5eb4eb9741068bd6.exe` sha256 `086758fe0bef689f…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^soft_step_sp2/coupled/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:27:22.857+03:00 → 2026-09-18T21:27:46.858+03:00 (24.0 s)
- ids that ran: `soft_step_sp2/coupled/64`, `soft_step_sp2/coupled/256`
- receipt BEFORE 2026-09-18T21:27:11.950+03:00: procs 233, CPU 10 s avg **1.95 %** (samples [2.83, 2.19, 1.52, 1.66, 1.67, 1.22, 1.82, 3.99, 0.52, 2.07]), top5 [Taskmgr 0.891, browser 0.453, browser 0.422, powershell 0.375, steamwebhelper 0.281], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:27:47.086+03:00: procs 233, CPU 10 s avg **2.77 %** (samples [1.27, 5.7, 2.92, 3.28, 2.09, 2.93, 2.05, 1.09, 2.95, 3.38]), top5 [browser 1.25, browser 0.547, Taskmgr 0.547, audiodg 0.391, powershell 0.391], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `soft_step_sp2/coupled/64` | 0.0873 | [0.0872, 0.0873] | 0.0873 | 0.0872 | 65535 | 60600 (i1=12) | Linear | 2026-09-18T21:27:34.185+03:00 | `raw/s7/A1/soft_step_sp2_coupled__64` |
| `soft_step_sp2/coupled/256` | 0.3835 | [0.3833, 0.3841] | 0.3839 | 0.3859 | 16383 | 15150 (i1=3) | Linear | 2026-09-18T21:27:46.851+03:00 | `raw/s7/A1/soft_step_sp2_coupled__256` |

#### s7 · soft_coupled · B1 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/soft_step_sp2-5eb4eb9741068bd6.exe` sha256 `cc01bda69891e974…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^soft_step_sp2/coupled/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:28:09.219+03:00 → 2026-09-18T21:28:33.179+03:00 (23.96 s)
- ids that ran: `soft_step_sp2/coupled/64`, `soft_step_sp2/coupled/256`
- receipt BEFORE 2026-09-18T21:27:58.320+03:00: procs 233, CPU 10 s avg **1.96 %** (samples [2.93, 2.25, 1.76, 1.38, 3.11, 1, 3.7, 0.13, 1.95, 1.38]), top5 [Taskmgr 0.781, powershell 0.375, browser 0.359, browser 0.312, claude 0.266], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:28:33.415+03:00: procs 233, CPU 10 s avg **2.27 %** (samples [2.09, 0.99, 2.64, 1.7, 0.67, 4.27, 2.28, 2.03, 3.4, 2.64]), top5 [browser 0.828, browser 0.688, Taskmgr 0.672, powershell 0.391, browser 0.141], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `soft_step_sp2/coupled/64` | 0.0869 | [0.0869, 0.0870] | 0.0871 | 0.0869 | 65535 | 60600 (i1=12) | Linear | 2026-09-18T21:28:20.551+03:00 | `raw/s7/B1/soft_step_sp2_coupled__64` |
| `soft_step_sp2/coupled/256` | 0.3814 | [0.3794, 0.3831] | 0.3837 | 0.3812 | 16383 | 15150 (i1=3) | Linear | 2026-09-18T21:28:33.170+03:00 | `raw/s7/B1/soft_step_sp2_coupled__256` |

#### s7 · soft_coupled · A2 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/soft_step_sp2-5eb4eb9741068bd6.exe` sha256 `086758fe0bef689f…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^soft_step_sp2/coupled/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:28:55.559+03:00 → 2026-09-18T21:29:19.638+03:00 (24.08 s)
- ids that ran: `soft_step_sp2/coupled/64`, `soft_step_sp2/coupled/256`
- receipt BEFORE 2026-09-18T21:28:44.679+03:00: procs 233, CPU 10 s avg **2.1 %** (samples [2.26, 0.56, 1.42, 1.47, 2.26, 1.65, 1.28, 4.09, 3.37, 2.69]), top5 [Taskmgr 0.688, powershell 0.391, steamwebhelper 0.312, browser 0.234, browser 0.203], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:29:19.865+03:00: procs 233, CPU 10 s avg **2.59 %** (samples [4.17, 2.86, 2.6, 2.63, 1.67, 1.19, 1.47, 0.49, 4.41, 4.45]), top5 [browser 0.734, Taskmgr 0.609, browser 0.562, steamwebhelper 0.391, powershell 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `soft_step_sp2/coupled/64` | 0.0874 | [0.0873, 0.0876] | 0.0878 | 0.0878 | 65535 | 60600 (i1=12) | Linear | 2026-09-18T21:29:06.945+03:00 | `raw/s7/A2/soft_step_sp2_coupled__64` |
| `soft_step_sp2/coupled/256` | 0.3815 | [0.3806, 0.3817] | 0.3822 | 0.3809 | 16383 | 15150 (i1=3) | Linear | 2026-09-18T21:29:19.584+03:00 | `raw/s7/A2/soft_step_sp2_coupled__256` |

#### s7 · soft_coupled · B2 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/soft_step_sp2-5eb4eb9741068bd6.exe` sha256 `cc01bda69891e974…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^soft_step_sp2/coupled/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:29:42.068+03:00 → 2026-09-18T21:30:06.141+03:00 (24.07 s)
- ids that ran: `soft_step_sp2/coupled/64`, `soft_step_sp2/coupled/256`
- receipt BEFORE 2026-09-18T21:29:31.135+03:00: procs 233, CPU 10 s avg **2.27 %** (samples [4.42, 2.53, 1.95, 1.41, 1.63, 2.06, 1.09, 2.05, 3.98, 1.57]), top5 [Taskmgr 0.688, powershell 0.391, steamwebhelper 0.344, browser 0.25, browser 0.234], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:30:06.367+03:00: procs 233, CPU 10 s avg **2.68 %** (samples [2.48, 1.28, 1.83, 1.8, 4.95, 3.87, 3.4, 3.12, 2.44, 1.67]), top5 [Taskmgr 0.734, browser 0.578, powershell 0.406, steamwebhelper 0.359, browser 0.312], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `soft_step_sp2/coupled/64` | 0.0869 | [0.0868, 0.0870] | 0.0872 | 0.0869 | 65535 | 60600 (i1=12) | Linear | 2026-09-18T21:29:53.402+03:00 | `raw/s7/B2/soft_step_sp2_coupled__64` |
| `soft_step_sp2/coupled/256` | 0.3823 | [0.3818, 0.3832] | 0.3837 | 0.3818 | 16383 | 15150 (i1=3) | Linear | 2026-09-18T21:30:06.083+03:00 | `raw/s7/B2/soft_step_sp2_coupled__256` |

#### s8 · sp_all · A1 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `e9ec0a2eb67ae10b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^sleeping_pipeline/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:30:33.727+03:00 → 2026-09-18T21:31:12.526+03:00 (38.8 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`, `sleeping_pipeline/pyramid_awake_sleeping_on`, `sleeping_pipeline/pyramid_frozen_sleeping_on`
- receipt BEFORE 2026-09-18T21:30:22.823+03:00: procs 233, CPU 10 s avg **2.02 %** (samples [3.04, 1.96, 0.75, 1.92, 2.83, 2.92, 1.07, 2.25, 1.65, 1.77]), top5 [Taskmgr 0.766, powershell 0.609, browser 0.297, steamwebhelper 0.281, browser 0.156], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:31:12.796+03:00: procs 232, CPU 10 s avg **3.49 %** (samples [1.18, 4.34, 2.53, 2.54, 0.7, 1.08, 1.67, 4.89, 7.54, 8.39]), top5 [browser 0.969, browser 0.906, Taskmgr 0.688, audiodg 0.422, powershell 0.422], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 17.7852 | [17.6194, 17.9045] | 17.5327 | 17.8077 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:30:49.336+03:00 | `raw/s8/A1/sleeping_pipeline__pyramid_sleeping_off` |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 16.7251 | [16.6495, 16.8758] | 16.6971 | 16.7691 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:31:00.733+03:00 | `raw/s8/A1/sleeping_pipeline__pyramid_awake_sleeping_on` |
| `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.6345 | [5.5885, 5.6872] | 5.6272 | 5.6462 | 1023 | 1050 (i1=5) | Linear | 2026-09-18T21:31:12.511+03:00 | `raw/s8/A1/sleeping_pipeline__pyramid_frozen_sleeping_on` |

#### s8 · sp_all · B1 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `ec3ff6ee8ee16db1…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^sleeping_pipeline/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:31:35.050+03:00 → 2026-09-18T21:32:14.092+03:00 (39.04 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`, `sleeping_pipeline/pyramid_awake_sleeping_on`, `sleeping_pipeline/pyramid_frozen_sleeping_on`
- receipt BEFORE 2026-09-18T21:31:24.114+03:00: procs 232, CPU 10 s avg **2.77 %** (samples [5.62, 3.11, 3.12, 2.25, 1.67, 2.55, 2.33, 2.69, 2.39, 1.96]), top5 [browser 1.406, browser 0.875, Taskmgr 0.594, powershell 0.344, browser 0.125], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:32:14.333+03:00: procs 232, CPU 10 s avg **2.09 %** (samples [3.87, 4.57, 3.52, 1.21, 0.98, 1.38, 0.88, 1.31, 1.95, 1.21]), top5 [browser 0.812, Taskmgr 0.578, powershell 0.375, browser 0.375, browser 0.156], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.1349 | [17.8068, 18.2410] | 17.9309 | 18.0610 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:31:50.790+03:00 | `raw/s8/B1/sleeping_pipeline__pyramid_sleeping_off` |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.3349 | [17.2684, 17.5140] | 17.2840 | 17.4501 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:32:02.583+03:00 | `raw/s8/B1/sleeping_pipeline__pyramid_awake_sleeping_on` |
| `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.4530 | [5.3826, 5.5031] | 5.4845 | 5.4536 | 1023 | 1050 (i1=5) | Linear | 2026-09-18T21:32:14.080+03:00 | `raw/s8/B1/sleeping_pipeline__pyramid_frozen_sleeping_on` |

#### s8 · sp_all · A2 · d552be05
- exe `D:/wt/_targets/mq-d552be05/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `e9ec0a2eb67ae10b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^sleeping_pipeline/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:32:36.519+03:00 → 2026-09-18T21:33:12.366+03:00 (35.85 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`, `sleeping_pipeline/pyramid_awake_sleeping_on`, `sleeping_pipeline/pyramid_frozen_sleeping_on`
- receipt BEFORE 2026-09-18T21:32:25.576+03:00: procs 232, CPU 10 s avg **3.05 %** (samples [6.84, 1.98, 5.4, 6.49, 0.79, 1.18, 2.78, 3.96, 1.09, 0]), top5 [browser 1.016, Taskmgr 0.609, audiodg 0.469, claude 0.453, powershell 0.391], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:33:12.608+03:00: procs 238, CPU 10 s avg **1.88 %** (samples [2.17, 2.14, 0.87, 2.14, 2.77, 1.05, 3.71, 0, 2.23, 1.67]), top5 [Taskmgr 0.672, powershell 0.391, browser 0.359, steamwebhelper 0.312, browser 0.219], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 17.8312 | [17.7384, 18.1376] | 17.6029 | 17.9087 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:32:52.224+03:00 | `raw/s8/A2/sleeping_pipeline__pyramid_sleeping_off` |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0295 | [16.8810, 17.3224] | 18.4633 | 17.6472 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:33:04.232+03:00 | `raw/s8/A2/sleeping_pipeline__pyramid_awake_sleeping_on` |
| `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.7020 | [5.6252, 5.8399] | 5.6440 | 5.8290 | 511 | 840 (i1=4) | Linear | 2026-09-18T21:33:12.333+03:00 | `raw/s8/A2/sleeping_pipeline__pyramid_frozen_sleeping_on` |

#### s8 · sp_all · B2 · a56007ab
- exe `D:/wt/_targets/mq-a56007ab/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `ec3ff6ee8ee16db1…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^sleeping_pipeline/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:33:34.726+03:00 → 2026-09-18T21:34:13.591+03:00 (38.87 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`, `sleeping_pipeline/pyramid_awake_sleeping_on`, `sleeping_pipeline/pyramid_frozen_sleeping_on`
- receipt BEFORE 2026-09-18T21:33:23.831+03:00: procs 238, CPU 10 s avg **3.43 %** (samples [2.65, 2.28, 2.33, 1.08, 1.44, 1.66, 3.12, 6.4, 5.85, 7.45]), top5 [Taskmgr 0.766, browser 0.625, steamwebhelper 0.516, browser 0.5, powershell 0.469], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:34:13.817+03:00: procs 236, CPU 10 s avg **3.85 %** (samples [3.29, 3.98, 5.34, 5.34, 7.93, 4.4, 3.32, 1.69, 1.56, 1.63]), top5 [browser 2.016, browser 1.078, Taskmgr 0.75, powershell 0.406, steamwebhelper 0.219], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 17.5806 | [17.1683, 17.6954] | 17.6146 | 17.4513 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:33:50.259+03:00 | `raw/s8/B2/sleeping_pipeline__pyramid_sleeping_off` |
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.2416 | [17.0465, 17.3979] | 17.3227 | 17.1850 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:34:01.920+03:00 | `raw/s8/B2/sleeping_pipeline__pyramid_awake_sleeping_on` |
| `sleeping_pipeline/pyramid_frozen_sleeping_on` | 5.6311 | [5.5640, 5.7218] | 5.5671 | 5.6476 | 1023 | 1050 (i1=5) | Linear | 2026-09-18T21:34:13.558+03:00 | `raw/s8/B2/sleeping_pipeline__pyramid_frozen_sleeping_on` |

#### s9 · sp_off · A1 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:34:48.846+03:00 → 2026-09-18T21:35:04.974+03:00 (16.13 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:34:37.908+03:00: procs 236, CPU 10 s avg **2.4 %** (samples [1.79, 4.19, 2.28, 1.75, 2.42, 3.49, 2.54, 2.03, 2.2, 1.34]), top5 [Taskmgr 0.766, claude 0.5, steamwebhelper 0.391, audiodg 0.375, powershell 0.344], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:35:05.202+03:00: procs 234, CPU 10 s avg **2.71 %** (samples [1.61, 3.89, 2.54, 2.63, 4.38, 4.22, 2.59, 0.8, 3.21, 1.28]), top5 [browser 1.297, browser 0.766, Taskmgr 0.656, powershell 0.406, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.8551 | [18.5487, 18.9620] | 18.4397 | 18.7599 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:35:04.961+03:00 | `raw/s9/A1/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · B1 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:35:27.335+03:00 → 2026-09-18T21:35:40.052+03:00 (12.72 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:35:16.450+03:00: procs 234, CPU 10 s avg **3.42 %** (samples [3.08, 4.06, 2.38, 2.6, 5.19, 5.14, 3.68, 3.3, 2.93, 1.86]), top5 [Taskmgr 0.812, browser 0.516, browser 0.422, powershell 0.391, steamwebhelper 0.312], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:35:40.285+03:00: procs 240, CPU 10 s avg **1.86 %** (samples [1.28, 2.97, 3.2, 1.98, 2.54, 2.25, 1.37, 1.86, 0, 1.18]), top5 [Taskmgr 0.734, claude 0.484, steamwebhelper 0.453, powershell 0.344, browser 0.094], toolchain procs present ['rust-analyzer']
- criterion warnings: Warning: Unable to complete 20 samples in 5.0s. You may wish to increase target time to 5.5s, enable flat sampling, or reduce sample count to 10.

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 26.1900 | [25.9682, 26.4565] | 27.3973 | 26.6205 | 127 | 210 (i1=1) | Linear | 2026-09-18T21:35:40.038+03:00 | `raw/s9/B1/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · A2 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:36:02.377+03:00 → 2026-09-18T21:36:18.389+03:00 (16.01 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:35:51.524+03:00: procs 240, CPU 10 s avg **0.6 %** (samples [0.07, 2.06, 0.34, 1.52, 0, 0.98, 0, 0, 0.8, 0.21]), top5 [Taskmgr 0.484, powershell 0.359, steamwebhelper 0.172, claude 0.078, browser 0.031], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:36:18.609+03:00: procs 235, CPU 10 s avg **0.94 %** (samples [0.61, 0.8, 0.51, 1.18, 1.48, 1.19, 1.83, 0.23, 1.56, 0]), top5 [Taskmgr 0.516, powershell 0.391, steamwebhelper 0.203, claude 0.094, AmneziaVPN 0.047], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.4564 | [18.3097, 18.6788] | 18.3311 | 18.4932 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:36:18.351+03:00 | `raw/s9/A2/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · B2 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:36:40.639+03:00 → 2026-09-18T21:36:52.834+03:00 (12.19 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:36:29.804+03:00: procs 235, CPU 10 s avg **0.6 %** (samples [0.19, 0.39, 1.38, 0.1, 0.01, 0.31, 1.59, 0, 0.52, 1.55]), top5 [Taskmgr 0.438, powershell 0.359, claude 0.094, browser 0.031, steamwebhelper 0.031], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:36:53.055+03:00: procs 234, CPU 10 s avg **0.97 %** (samples [2.79, 0.32, 2.13, 1.97, 0, 0.29, 0, 1.47, 0.24, 0.48]), top5 [Taskmgr 0.531, powershell 0.344, claude 0.219, steamwebhelper 0.062, browser 0.016], toolchain procs present ['rust-analyzer']
- criterion warnings: Warning: Unable to complete 20 samples in 5.0s. You may wish to increase target time to 5.4s, enable flat sampling, or reduce sample count to 10.

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 25.2639 | [24.8788, 25.4272] | 25.4094 | 25.2110 | 127 | 210 (i1=1) | Linear | 2026-09-18T21:36:52.803+03:00 | `raw/s9/B2/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · full_step_1 · A1 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:37:21.470+03:00 → 2026-09-18T21:37:43.742+03:00 (22.27 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:37:10.618+03:00: procs 234, CPU 10 s avg **1.17 %** (samples [1.07, 2.25, 1.28, 1.47, 3.5, 0, 1.5, 0, 0.61, 0]), top5 [Taskmgr 0.688, powershell 0.328, claude 0.297, steamwebhelper 0.219, steamwebhelper 0.047], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:37:43.964+03:00: procs 234, CPU 10 s avg **1.52 %** (samples [0, 0.9, 1.33, 0.76, 1.67, 0.13, 2.28, 1.95, 2.02, 4.21]), top5 [Taskmgr 0.547, powershell 0.391, steamwebhelper 0.266, browser 0.109, browser 0.078], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.6569 | [18.3577, 18.9535] | 18.3620 | 18.7578 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:37:43.734+03:00 | `raw/s9/A1/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · B1 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:39:05.165+03:00 → 2026-09-18T21:39:29.899+03:00 (24.73 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:38:54.274+03:00: procs 236, CPU 10 s avg **2.49 %** (samples [2.91, 3.26, 1.33, 1.76, 5.22, 2.06, 1.92, 0.99, 1.77, 3.64]), top5 [browser 0.969, Taskmgr 0.672, browser 0.5, powershell 0.391, steamwebhelper 0.297], toolchain procs present ['rust-analyzer']; waited 59 s (polls: 5.2 %)
- receipt AFTER 2026-09-18T21:39:30.133+03:00: procs 237, CPU 10 s avg **2.36 %** (samples [3.19, 1.96, 1.72, 2.49, 2.68, 2.39, 2.87, 1.54, 3.03, 1.77]), top5 [browser 0.734, Taskmgr 0.703, browser 0.656, powershell 0.453, claude 0.219], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 20.7566 | [20.5218, 21.2375] | 21.3020 | 20.9256 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:39:29.890+03:00 | `raw/s9/B1/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · A2 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:39:52.311+03:00 → 2026-09-18T21:40:16.745+03:00 (24.43 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:39:41.418+03:00: procs 236, CPU 10 s avg **1.91 %** (samples [0.44, 1.09, 1.57, 1.18, 1.41, 3.39, 1.27, 5.15, 2.63, 0.98]), top5 [Taskmgr 0.5, powershell 0.359, browser 0.344, browser 0.344, audiodg 0.281], toolchain procs present ['rust-analyzer']
- receipt AFTER 2026-09-18T21:40:17.015+03:00: procs 235, CPU 10 s avg **16.66 %** (samples [13.17, 19.88, 23.42, 7.54, 15.59, 27.71, 22.07, 13.56, 10.77, 12.84]), top5 [browser 6.078, browser 3.484, browser 3, browser 2.578, Taskmgr 0.906], toolchain procs present ['rust-analyzer']
- **SUSPECT: after-receipt 16.66 % (browser burst right at run end); A2 = 20.61 ms against A1/A3/A4 = 18.66/19.49/18.97**

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 20.6072 | [19.9676, 21.2714] | 20.7986 | 21.2586 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:40:16.700+03:00 | `raw/s9/A2/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · B2 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:41:38.634+03:00 → 2026-09-18T21:42:03.531+03:00 (24.9 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:41:27.758+03:00: procs 238, CPU 10 s avg **3.05 %** (samples [3.99, 4.78, 3.09, 2.24, 0.7, 1.18, 3.22, 4.47, 3.51, 3.26]), top5 [browser 0.922, browser 0.812, Taskmgr 0.672, powershell 0.359, steamwebhelper 0.203], toolchain procs present ['rust-analyzer']; waited 59 s (polls: 9.69 %)
- receipt AFTER 2026-09-18T21:42:03.771+03:00: procs 238, CPU 10 s avg **2.44 %** (samples [3, 2.46, 2.42, 1.86, 1.29, 2.57, 4.97, 1.62, 1.61, 2.6]), top5 [browser 0.781, Taskmgr 0.656, powershell 0.453, browser 0.438, browser 0.219], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 20.9655 | [20.2475, 21.5043] | 21.5528 | 21.0363 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:42:03.490+03:00 | `raw/s9/B2/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_4 · A1 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:43:35.117+03:00 → 2026-09-18T21:43:53.361+03:00 (18.24 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:43:23.851+03:00: procs 237, CPU 10 s avg **2.87 %** (samples [5.31, 2.92, 2.46, 2.42, 3.61, 3.81, 2.9, 1.19, 1.24, 2.87]), top5 [Taskmgr 0.828, steamwebhelper 0.438, powershell 0.422, browser 0.344, claude 0.297], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.35 %** (bench itself 12.53 %); top5 other [browser 1.609, Taskmgr 1.312, browser 1.266, claude 0.422, steamwebhelper 0.281]
- receipt AFTER 2026-09-18T21:43:53.932+03:00: procs 236, CPU 10 s avg **3.07 %** (samples [4.05, 4.4, 1.09, 4.29, 1.36, 3.25, 1.15, 3.6, 3.92, 3.59]), top5 [browser 0.906, browser 0.812, Taskmgr 0.672, powershell 0.406, claude 0.328], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 10.8959 | [10.7989, 11.2127] | 10.8069 | 10.9786 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:43:53.353+03:00 | `raw/s9/A1/jolt_parity_pyramid__full_step__4` |

#### s9 · full_step_4 · B1 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:44:16.491+03:00 → 2026-09-18T21:44:33.617+03:00 (17.13 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:44:05.237+03:00: procs 236, CPU 10 s avg **2.49 %** (samples [2.86, 3.02, 2.81, 2.43, 2.62, 2.41, 2.32, 1.71, 1.37, 3.4]), top5 [Taskmgr 0.812, browser 0.75, powershell 0.375, browser 0.344, steamwebhelper 0.312], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.99 %** (bench itself 12.86 %); top5 other [browser 1.531, browser 1.156, Taskmgr 1.125, steamwebhelper 0.531, claude 0.344]
- receipt AFTER 2026-09-18T21:44:34.191+03:00: procs 236, CPU 10 s avg **3.03 %** (samples [6.64, 2.63, 2.69, 1.41, 2.83, 4.56, 2.44, 1.66, 1.76, 3.69]), top5 [browser 1.375, browser 0.828, Taskmgr 0.547, powershell 0.438, steamwebhelper 0.297], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.6027 | [11.3750, 11.8843] | 11.7490 | 11.6280 | 511 | 420 (i1=2) | Linear | 2026-09-18T21:44:33.608+03:00 | `raw/s9/B1/jolt_parity_pyramid__full_step__4` |

#### s9 · full_step_4 · A2 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:44:56.800+03:00 → 2026-09-18T21:45:15.390+03:00 (18.59 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:44:45.510+03:00: procs 236, CPU 10 s avg **2.35 %** (samples [1.78, 2.73, 3.9, 2.43, 0.78, 1.26, 1.9, 2.93, 3.61, 2.22]), top5 [Taskmgr 0.625, browser 0.5, browser 0.391, powershell 0.375, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.55 %** (bench itself 12.59 %); top5 other [Taskmgr 1.312, browser 1.0, browser 0.781, steamwebhelper 0.391, browser 0.281]
- receipt AFTER 2026-09-18T21:45:15.961+03:00: procs 237, CPU 10 s avg **3.71 %** (samples [5.9, 5.29, 2.12, 2.35, 3.61, 1.4, 2.52, 4.95, 4.66, 4.31]), top5 [browser 1.781, browser 1.156, Taskmgr 0.719, powershell 0.391, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.0667 | [10.7709, 11.4294] | 10.9529 | 11.1339 | 511 | 630 (i1=3) | Linear | 2026-09-18T21:45:15.359+03:00 | `raw/s9/A2/jolt_parity_pyramid__full_step__4` |

#### s9 · full_step_4 · B2 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/4`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:45:38.528+03:00 → 2026-09-18T21:45:54.973+03:00 (16.44 s)
- ids that ran: `jolt_parity_pyramid/full_step/4`
- receipt BEFORE 2026-09-18T21:45:27.247+03:00: procs 237, CPU 10 s avg **2.55 %** (samples [1.86, 1.47, 2.67, 2.31, 2.46, 3.54, 2.23, 1.09, 5.59, 2.3]), top5 [Taskmgr 0.75, steamwebhelper 0.578, powershell 0.578, browser 0.484, browser 0.406], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **1.72 %** (bench itself 13.09 %); top5 other [Taskmgr 1.047, browser 0.969, browser 0.391, claude 0.297, browser 0.25]
- receipt AFTER 2026-09-18T21:45:55.525+03:00: procs 235, CPU 10 s avg **5.12 %** (samples [2.83, 5.99, 6.77, 6.49, 3.98, 3.21, 3.14, 4.4, 7.68, 6.75]), top5 [browser 1.547, browser 1.141, Taskmgr 0.984, powershell 0.562, steamwebhelper 0.438], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/4` | 11.9273 | [11.6345, 12.2241] | 12.0496 | 11.9646 | 255 | 630 (i1=3) | Linear | 2026-09-18T21:45:54.940+03:00 | `raw/s9/B2/jolt_parity_pyramid__full_step__4` |

#### s9 · sp_awake · A1 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:46:24.050+03:00 → 2026-09-18T21:46:39.633+03:00 (15.58 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:46:12.869+03:00: procs 235, CPU 10 s avg **1.71 %** (samples [1.27, 2.45, 1.85, 2.44, 1.47, 1.82, 1.42, 2.05, 1.09, 1.27]), top5 [Taskmgr 0.516, browser 0.516, browser 0.344, powershell 0.328, steamwebhelper 0.172], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.03 %** (bench itself 6.33 %); top5 other [Taskmgr 1.297, browser 0.516, audiodg 0.484, claude 0.391, steamwebhelper 0.359]
- receipt AFTER 2026-09-18T21:46:40.169+03:00: procs 235, CPU 10 s avg **4.78 %** (samples [3.5, 6.83, 4.31, 4.81, 4.37, 4.08, 6.01, 6.49, 4.21, 3.17]), top5 [browser 2.219, browser 0.828, Taskmgr 0.703, powershell 0.391, steamwebhelper 0.391], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.8255 | [17.2878, 18.0856] | 17.9735 | 17.7444 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:46:39.618+03:00 | `raw/s9/A1/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · B1 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:47:02.663+03:00 → 2026-09-18T21:47:21.243+03:00 (18.58 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:46:51.440+03:00: procs 235, CPU 10 s avg **2.65 %** (samples [1.76, 2.77, 0.19, 4.08, 1.62, 2.42, 1.03, 4.93, 2.9, 4.85]), top5 [Taskmgr 0.703, browser 0.703, browser 0.547, steamwebhelper 0.422, powershell 0.406], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.97 %** (bench itself 6.36 %); top5 other [Taskmgr 1.469, browser 1.438, browser 0.734, steamwebhelper 0.641, claude 0.297]
- receipt AFTER 2026-09-18T21:47:21.779+03:00: procs 234, CPU 10 s avg **4.02 %** (samples [1.67, 1.81, 3.32, 5.49, 6.59, 5.22, 5.86, 4.95, 2.38, 2.89]), top5 [browser 1.797, browser 1, Taskmgr 0.656, powershell 0.375, steamwebhelper 0.375], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.5734 | [21.4513, 21.7184] | 21.6120 | 21.6178 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:47:21.229+03:00 | `raw/s9/B1/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · A2 · 08fe7b9f
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:47:44.239+03:00 → 2026-09-18T21:47:59.339+03:00 (15.1 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:47:33.004+03:00: procs 234, CPU 10 s avg **2.93 %** (samples [3.58, 0.12, 2.93, 2.83, 3.51, 2.72, 2.53, 1.96, 1.96, 7.17]), top5 [browser 1.016, browser 0.703, Taskmgr 0.625, powershell 0.406, steamwebhelper 0.281], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.82 %** (bench itself 6.4 %); top5 other [Taskmgr 1.031, steamwebhelper 0.766, browser 0.625, browser 0.484, audiodg 0.266]
- receipt AFTER 2026-09-18T21:47:59.899+03:00: procs 234, CPU 10 s avg **2.79 %** (samples [4.35, 4.14, 1.86, 3.4, 1.22, 2.31, 2.25, 1.72, 1.76, 4.87]), top5 [Taskmgr 0.719, browser 0.5, powershell 0.391, browser 0.344, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0423 | [16.8530, 17.1835] | 17.2135 | 17.0326 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:47:59.304+03:00 | `raw/s9/A2/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · B2 · 8d656ad8
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:48:22.391+03:00 → 2026-09-18T21:48:40.600+03:00 (18.21 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:48:11.166+03:00: procs 235, CPU 10 s avg **3.05 %** (samples [3.32, 3.31, 1.18, 1.37, 1.94, 6.33, 2.95, 2.51, 2.63, 4.98]), top5 [browser 1.844, browser 0.734, Taskmgr 0.469, powershell 0.422, browser 0.156], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.71 %** (bench itself 6.28 %); top5 other [browser 1.375, Taskmgr 1.375, browser 1.109, claude 0.562, steamwebhelper 0.359]
- receipt AFTER 2026-09-18T21:48:41.163+03:00: procs 235, CPU 10 s avg **2.16 %** (samples [2.46, 1.71, 4.06, 2.71, 1.78, 1.2, 0.96, 0.8, 2.54, 3.33]), top5 [Taskmgr 0.703, powershell 0.406, browser 0.281, browser 0.156, browser 0.156], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.0114 | [20.8089, 21.5294] | 21.3683 | 21.1873 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:48:40.565+03:00 | `raw/s9/B2/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · full_step_1 · A3 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:49:22.223+03:00 → 2026-09-18T21:49:45.546+03:00 (23.32 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:49:10.984+03:00: procs 241, CPU 10 s avg **2.09 %** (samples [2.89, 1.97, 0.78, 1.69, 1.01, 0.67, 2.13, 2.25, 2.35, 5.2]), top5 [Taskmgr 0.547, powershell 0.5, browser 0.453, browser 0.422, claude 0.203], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.6 %** (bench itself 6.35 %); top5 other [browser 2.469, browser 1.438, Taskmgr 1.375, steamwebhelper 0.703, browser 0.375]
- receipt AFTER 2026-09-18T21:49:46.094+03:00: procs 241, CPU 10 s avg **2.39 %** (samples [1.49, 1.54, 3.98, 3.22, 2.93, 3.14, 2.72, 1.56, 1.67, 1.68]), top5 [browser 0.766, browser 0.672, Taskmgr 0.562, powershell 0.359, audiodg 0.281], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 19.4885 | [19.2609, 19.7566] | 19.2432 | 19.7119 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:49:45.509+03:00 | `raw/s9/A3/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · B3 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:50:08.550+03:00 → 2026-09-18T21:50:33.766+03:00 (25.22 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:49:57.333+03:00: procs 236, CPU 10 s avg **2.81 %** (samples [1.47, 2.83, 2.73, 1.96, 5.62, 2.64, 5.16, 2.37, 0.85, 2.47]), top5 [browser 0.672, Taskmgr 0.625, browser 0.453, powershell 0.391, steamwebhelper 0.328], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.89 %** (bench itself 6.23 %); top5 other [browser 2.578, browser 2.109, Taskmgr 1.453, steamwebhelper 1.078, claude 0.469]
- receipt AFTER 2026-09-18T21:50:34.336+03:00: procs 235, CPU 10 s avg **2.58 %** (samples [1.99, 0.95, 1.1, 1.99, 3.61, 2.31, 3.6, 4.39, 3.16, 2.68]), top5 [browser 0.625, Taskmgr 0.594, powershell 0.391, browser 0.359, claude 0.219], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 21.1604 | [20.9456, 21.7777] | 21.6465 | 21.2444 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:50:33.732+03:00 | `raw/s9/B3/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · A4 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `c78a9ffe77a9bf0c…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:50:56.806+03:00 → 2026-09-18T21:51:19.627+03:00 (22.82 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:50:45.578+03:00: procs 236, CPU 10 s avg **1.56 %** (samples [0.69, 0.85, 1.7, 1.4, 1.52, 0.43, 3.19, 1.99, 1.82, 1.96]), top5 [Taskmgr 0.688, powershell 0.438, steamwebhelper 0.359, browser 0.25, browser 0.188], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.12 %** (bench itself 6.3 %); top5 other [browser 2.203, browser 1.656, Taskmgr 1.531, steamwebhelper 0.578, claude 0.531]
- receipt AFTER 2026-09-18T21:51:20.182+03:00: procs 236, CPU 10 s avg **2.36 %** (samples [4, 2.95, 2.11, 2.82, 1.48, 3, 1.4, 2.52, 2.56, 0.8]), top5 [Taskmgr 0.797, browser 0.469, browser 0.438, powershell 0.391, steamwebhelper 0.25], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 18.9712 | [18.7815, 19.1912] | 18.8963 | 19.1122 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:51:19.598+03:00 | `raw/s9/A4/jolt_parity_pyramid__full_step__1` |

#### s9 · full_step_1 · B4 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/jolt_parity_pyramid-43b10ef74d0e3f27.exe` sha256 `80142a730beaa1c3…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact jolt_parity_pyramid/full_step/1`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:51:42.716+03:00 → 2026-09-18T21:52:07.714+03:00 (25.0 s)
- ids that ran: `jolt_parity_pyramid/full_step/1`
- receipt BEFORE 2026-09-18T21:51:31.453+03:00: procs 236, CPU 10 s avg **2.59 %** (samples [5.33, 4.01, 6.02, 3.11, 0.51, 0.99, 0.2, 3.01, 1.68, 1]), top5 [browser 1.016, Taskmgr 0.688, powershell 0.391, browser 0.312, browser 0.266], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.3 %** (bench itself 6.25 %); top5 other [browser 3.094, browser 2.203, Taskmgr 1.75, steamwebhelper 0.562, browser 0.375]
- receipt AFTER 2026-09-18T21:52:08.247+03:00: procs 236, CPU 10 s avg **2.4 %** (samples [2.08, 1.4, 2.02, 1.77, 1.67, 6.58, 4.85, 1.5, 0.88, 1.28]), top5 [Taskmgr 0.766, browser 0.469, powershell 0.438, browser 0.359, steamwebhelper 0.344], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `jolt_parity_pyramid/full_step/1` | 21.1147 | [20.7152, 21.6788] | 21.4754 | 21.2309 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:52:07.686+03:00 | `raw/s9/B4/jolt_parity_pyramid__full_step__1` |

#### s9 · sp_off · A3 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:52:36.225+03:00 → 2026-09-18T21:52:52.535+03:00 (16.31 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:52:24.980+03:00: procs 236, CPU 10 s avg **3.15 %** (samples [5.63, 1.6, 1.06, 0.02, 1.86, 1.17, 1.64, 4.12, 8.47, 5.97]), top5 [browser 0.953, browser 0.812, Taskmgr 0.703, steamwebhelper 0.516, powershell 0.406], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.98 %** (bench itself 6.21 %); top5 other [browser 1.734, browser 1.156, Taskmgr 0.875, steamwebhelper 0.469, browser 0.359]
- receipt AFTER 2026-09-18T21:52:53.076+03:00: procs 236, CPU 10 s avg **1.78 %** (samples [2.98, 2.75, 2.13, 1.34, 0.87, 1.65, 0.42, 3.67, 0.12, 1.87]), top5 [Taskmgr 0.672, browser 0.625, powershell 0.391, browser 0.297, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.9987 | [18.6644, 19.1784] | 18.7109 | 18.8712 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:52:52.500+03:00 | `raw/s9/A3/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · B3 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:53:15.626+03:00 → 2026-09-18T21:53:28.459+03:00 (12.83 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:53:04.376+03:00: procs 236, CPU 10 s avg **1.98 %** (samples [0.22, 3.6, 1.85, 0.59, 2.38, 4.3, 0.57, 1.95, 1.67, 2.63]), top5 [browser 0.875, Taskmgr 0.688, powershell 0.656, steamwebhelper 0.297, browser 0.281], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **4.1 %** (bench itself 6.29 %); top5 other [browser 1.969, browser 1.281, Taskmgr 0.781, steamwebhelper 0.469, browser 0.188]
- receipt AFTER 2026-09-18T21:53:29.001+03:00: procs 236, CPU 10 s avg **2.41 %** (samples [2.33, 1.67, 1.66, 2.44, 3.34, 1.44, 4.57, 1.95, 1.86, 2.83]), top5 [browser 0.625, browser 0.594, Taskmgr 0.516, claude 0.516, powershell 0.391], toolchain procs present ['rust-analyzer']
- criterion warnings: Warning: Unable to complete 20 samples in 5.0s. You may wish to increase target time to 5.7s, enable flat sampling, or reduce sample count to 10.

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 27.2549 | [26.9366, 27.4068] | 27.1610 | 27.1806 | 127 | 210 (i1=1) | Linear | 2026-09-18T21:53:28.425+03:00 | `raw/s9/B3/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · A4 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:53:51.447+03:00 → 2026-09-18T21:54:07.745+03:00 (16.3 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:53:40.248+03:00: procs 236, CPU 10 s avg **1.7 %** (samples [0.9, 3.59, 1.66, 1.95, 0.69, 0.71, 2.17, 1.87, 2.24, 1.21]), top5 [Taskmgr 0.641, browser 0.406, powershell 0.375, browser 0.344, steamwebhelper 0.188], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.28 %** (bench itself 6.41 %); top5 other [browser 2.375, browser 1.188, Taskmgr 0.891, steamwebhelper 0.391, browser 0.234]
- receipt AFTER 2026-09-18T21:54:08.307+03:00: procs 236, CPU 10 s avg **2.9 %** (samples [2.61, 2.17, 5.37, 1.99, 1.62, 1.63, 2.44, 4.4, 4.46, 2.31]), top5 [browser 0.844, Taskmgr 0.672, browser 0.469, powershell 0.438, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 18.5245 | [18.4086, 18.6602] | 18.5544 | 18.5678 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:54:07.709+03:00 | `raw/s9/A4/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_off · B4 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_sleeping_off`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:54:30.764+03:00 → 2026-09-18T21:54:43.567+03:00 (12.8 s)
- ids that ran: `sleeping_pipeline/pyramid_sleeping_off`
- receipt BEFORE 2026-09-18T21:54:19.545+03:00: procs 236, CPU 10 s avg **2.06 %** (samples [1.91, 1.64, 1.5, 1.86, 1.29, 1.75, 1.09, 1.57, 0.3, 7.71]), top5 [browser 0.969, Taskmgr 0.875, powershell 0.406, browser 0.344, browser 0.156], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.79 %** (bench itself 6.31 %); top5 other [Taskmgr 0.734, steamwebhelper 0.641, browser 0.297, browser 0.297, claude 0.281]
- receipt AFTER 2026-09-18T21:54:44.100+03:00: procs 236, CPU 10 s avg **3.69 %** (samples [5.1, 6.28, 6.28, 4.84, 2.73, 3.33, 3.11, 1.27, 1.19, 2.73]), top5 [browser 1.703, browser 1.297, Taskmgr 0.75, powershell 0.438, audiodg 0.375], toolchain procs present ['rust-analyzer']
- criterion warnings: Warning: Unable to complete 20 samples in 5.0s. You may wish to increase target time to 5.7s, enable flat sampling, or reduce sample count to 10.

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_sleeping_off` | 26.9695 | [26.5798, 27.1435] | 27.1118 | 26.8944 | 127 | 210 (i1=1) | Linear | 2026-09-18T21:54:43.533+03:00 | `raw/s9/B4/sleeping_pipeline__pyramid_sleeping_off` |

#### s9 · sp_awake · A3 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:55:14.780+03:00 → 2026-09-18T21:55:30.620+03:00 (15.84 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:55:03.538+03:00: procs 235, CPU 10 s avg **2.12 %** (samples [1.58, 1.47, 4.18, 1.57, 2.07, 1.27, 0.9, 1.57, 3.61, 3.01]), top5 [Taskmgr 0.578, browser 0.469, browser 0.422, powershell 0.406, steamwebhelper 0.312], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.8 %** (bench itself 6.4 %); top5 other [browser 1.141, browser 0.969, Taskmgr 0.844, steamwebhelper 0.312, browser 0.219]
- receipt AFTER 2026-09-18T21:55:31.168+03:00: procs 235, CPU 10 s avg **3.02 %** (samples [3.75, 1, 0.91, 1.94, 2.44, 6.69, 6.15, 3.69, 2.06, 1.61]), top5 [browser 1.547, browser 0.703, Taskmgr 0.703, powershell 0.375, steamwebhelper 0.328], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 18.1057 | [17.7532, 18.4449] | 18.3403 | 18.0961 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:55:30.586+03:00 | `raw/s9/A3/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · B3 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:55:53.607+03:00 → 2026-09-18T21:56:12.284+03:00 (18.68 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:55:42.371+03:00: procs 235, CPU 10 s avg **2.25 %** (samples [1.29, 1, 3.4, 5.34, 1.67, 2.05, 4.09, 1.56, 1.19, 0.95]), top5 [browser 0.609, Taskmgr 0.531, audiodg 0.484, browser 0.469, powershell 0.375], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.71 %** (bench itself 6.26 %); top5 other [browser 1.25, Taskmgr 1.203, browser 0.516, steamwebhelper 0.438, claude 0.359]
- receipt AFTER 2026-09-18T21:56:12.817+03:00: procs 234, CPU 10 s avg **3.8 %** (samples [7.32, 3.89, 3.51, 3.61, 3.61, 3.5, 4.59, 2.6, 1.88, 3.49]), top5 [browser 2.312, browser 1.062, Taskmgr 0.703, claude 0.375, powershell 0.359], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 21.8375 | [21.5864, 22.2428] | 21.9716 | 21.8753 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:56:12.249+03:00 | `raw/s9/B3/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · A4 · 08fe7b9f  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-08fe7b9f/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `fe421ad35d6c893b…` (verified before the run); cwd `D:/wt/mq-08fe7b9f/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-08fe7b9f`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:56:35.231+03:00 → 2026-09-18T21:56:50.506+03:00 (15.27 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:56:24.032+03:00: procs 234, CPU 10 s avg **2.25 %** (samples [0.58, 1.77, 2.38, 5.78, 2.03, 1.68, 1.95, 1.29, 2.12, 2.87]), top5 [browser 0.688, Taskmgr 0.656, browser 0.562, powershell 0.359, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.69 %** (bench itself 6.33 %); top5 other [Taskmgr 1.062, browser 0.578, steamwebhelper 0.531, browser 0.484, claude 0.312]
- receipt AFTER 2026-09-18T21:56:51.068+03:00: procs 234, CPU 10 s avg **3.96 %** (samples [0.98, 0.48, 0.17, 3.66, 2.05, 7.37, 5.23, 6.99, 8.03, 4.59]), top5 [browser 1.844, browser 0.891, Taskmgr 0.719, steamwebhelper 0.438, powershell 0.422], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 17.0879 | [16.8841, 17.6053] | 17.5410 | 17.2511 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:56:50.471+03:00 | `raw/s9/A4/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s9 · sp_awake · B4 · 8d656ad8  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-8d656ad8/release/deps/sleeping_pipeline-b79088505f0ef46b.exe` sha256 `225b355d7bb1ba37…` (verified before the run); cwd `D:/wt/mq-8d656ad8/crates/boyko_physics`
- args `--bench --noplot --exact sleeping_pipeline/pyramid_awake_sleeping_on`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-8d656ad8`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:57:13.514+03:00 → 2026-09-18T21:57:31.221+03:00 (17.71 s)
- ids that ran: `sleeping_pipeline/pyramid_awake_sleeping_on`
- receipt BEFORE 2026-09-18T21:57:02.334+03:00: procs 234, CPU 10 s avg **2.42 %** (samples [2.83, 2.76, 1.17, 1.09, 2.83, 3.53, 5.06, 0.99, 0.83, 3.08]), top5 [browser 0.891, Taskmgr 0.578, browser 0.469, powershell 0.406, audiodg 0.156], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.08 %** (bench itself 6.22 %); top5 other [Taskmgr 1.375, browser 1.344, browser 0.719, steamwebhelper 0.531, claude 0.516]
- receipt AFTER 2026-09-18T21:57:31.779+03:00: procs 234, CPU 10 s avg **3.6 %** (samples [2.32, 2.05, 3.15, 5.63, 2.24, 1.67, 3.98, 3.21, 5.76, 5.94]), top5 [Taskmgr 0.891, browser 0.797, browser 0.641, powershell 0.469, steamwebhelper 0.422], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `sleeping_pipeline/pyramid_awake_sleeping_on` | 20.2263 | [20.0906, 20.5511] | 20.4905 | 20.3213 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:57:31.188+03:00 | `raw/s9/B4/sleeping_pipeline__pyramid_awake_sleeping_on` |

#### s7 · churn_stable · A3 · d552be05  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-d552be05/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `6e37d7db2cb3110b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:57:58.773+03:00 → 2026-09-18T21:58:29.715+03:00 (30.94 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:57:47.504+03:00: procs 234, CPU 10 s avg **3.07 %** (samples [3.95, 3.28, 4.74, 6.16, 2.86, 1.93, 2.25, 1.87, 1.56, 2.09]), top5 [browser 1.281, browser 0.641, Taskmgr 0.609, powershell 0.375, claude 0.203], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.7 %** (bench itself 6.34 %); top5 other [Taskmgr 2.219, browser 1.844, browser 1.484, steamwebhelper 1.0, claude 0.625]
- receipt AFTER 2026-09-18T21:58:30.267+03:00: procs 234, CPU 10 s avg **3.62 %** (samples [5.74, 3, 3.23, 2.73, 2.8, 4.69, 2.67, 2.12, 3.11, 6.1]), top5 [browser 1.281, Taskmgr 0.625, browser 0.609, powershell 0.453, steamwebhelper 0.312], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.1108 | [17.0202, 17.3561] | 17.3623 | 17.1900 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:58:11.266+03:00 | `raw/s7/A3/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.6816 | [16.5984, 16.8261] | 16.6165 | 16.7064 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:58:26.464+03:00 | `raw/s7/A3/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · B3 · a56007ab  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-a56007ab/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `48e71cca5e94674b…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:58:52.841+03:00 → 2026-09-18T21:59:23.552+03:00 (30.71 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:58:41.586+03:00: procs 234, CPU 10 s avg **1.78 %** (samples [3.14, 2.93, 2.44, 0.8, 1.19, 0.03, 1.94, 1.88, 1.64, 1.78]), top5 [browser 0.703, Taskmgr 0.594, powershell 0.438, browser 0.359, steamwebhelper 0.25], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.32 %** (bench itself 6.43 %); top5 other [browser 3.484, Taskmgr 2.297, browser 1.922, steamwebhelper 1.422, claude 0.812]
- receipt AFTER 2026-09-18T21:59:24.113+03:00: procs 234, CPU 10 s avg **1.75 %** (samples [0.63, 1.04, 1.19, 0.76, 2.19, 1.86, 3.15, 2.21, 2.99, 1.44]), top5 [Taskmgr 0.609, powershell 0.422, browser 0.406, browser 0.359, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 17.1838 | [16.7336, 17.3525] | 17.2391 | 17.0975 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:59:05.153+03:00 | `raw/s7/B3/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.6283 | [16.5130, 16.7549] | 16.6035 | 16.6669 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:59:20.290+03:00 | `raw/s7/B3/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · A4 · d552be05  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-d552be05/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `6e37d7db2cb3110b…` (verified before the run); cwd `D:/wt/mq-d552be05/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-d552be05`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T21:59:46.594+03:00 → 2026-09-18T22:00:17.566+03:00 (30.97 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T21:59:35.379+03:00: procs 234, CPU 10 s avg **1.87 %** (samples [1.25, 0.69, 1.27, 0.62, 0.61, 1.89, 4.54, 2.44, 3.41, 1.95]), top5 [Taskmgr 0.656, powershell 0.406, browser 0.328, steamwebhelper 0.281, browser 0.266], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **3.79 %** (bench itself 6.25 %); top5 other [browser 3.297, browser 2.438, Taskmgr 1.953, claude 0.828, steamwebhelper 0.656]
- receipt AFTER 2026-09-18T22:00:18.106+03:00: procs 234, CPU 10 s avg **2.62 %** (samples [1.54, 3.5, 4.1, 4.02, 4.18, 0.71, 1.82, 4.12, 1.08, 1.08]), top5 [Taskmgr 0.609, browser 0.562, powershell 0.375, audiodg 0.375, browser 0.281], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 16.9773 | [16.8751, 17.1294] | 16.9046 | 17.0008 | 255 | 420 (i1=2) | Linear | 2026-09-18T21:59:58.750+03:00 | `raw/s7/A4/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 16.8479 | [16.7040, 17.0540] | 16.9351 | 16.8439 | 255 | 420 (i1=2) | Linear | 2026-09-18T22:00:14.295+03:00 | `raw/s7/A4/row_identity_churn__stable__sleeping_on` |

#### s7 · churn_stable · B4 · a56007ab  (SUPPLEMENTARY)
- exe `D:/wt/_targets/mq-a56007ab/release/deps/row_identity_churn-a5d5ed5921338d69.exe` sha256 `48e71cca5e94674b…` (verified before the run); cwd `D:/wt/mq-a56007ab/crates/boyko_physics`
- args `--bench --noplot ^row_identity_churn/stable/`; env CARGO_TARGET_DIR=`D:/wt/_targets/mq-a56007ab`, CRITERION_DEBUG=1; rc=0; run 2026-09-18T22:00:40.565+03:00 → 2026-09-18T22:01:11.464+03:00 (30.9 s)
- ids that ran: `row_identity_churn/stable/sleeping_off`, `row_identity_churn/stable/sleeping_on`
- receipt BEFORE 2026-09-18T22:00:29.316+03:00: procs 234, CPU 10 s avg **3.72 %** (samples [1.85, 2, 3.57, 5.71, 2.75, 2.42, 5.58, 3.08, 6.3, 3.98]), top5 [browser 0.938, browser 0.828, Taskmgr 0.781, powershell 0.391, steamwebhelper 0.266], toolchain procs present ['rust-analyzer']
- DURING run: whole-machine busy minus the bench process = **2.33 %** (bench itself 6.29 %); top5 other [browser 2.359, Taskmgr 1.891, browser 1.516, steamwebhelper 0.719, browser 0.438]
- receipt AFTER 2026-09-18T22:01:12.012+03:00: procs 233, CPU 10 s avg **3.25 %** (samples [3, 1.93, 2.47, 3.69, 5.16, 1.48, 3.2, 2.92, 5.24, 3.4]), top5 [browser 0.844, browser 0.828, Taskmgr 0.531, powershell 0.422, claude 0.172], toolchain procs present ['rust-analyzer']

| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |
|---|---|---|---|---|---|---|---|---|---|
| `row_identity_churn/stable/sleeping_off` | 16.7985 | [16.6330, 16.9542] | 16.7358 | 16.7996 | 255 | 420 (i1=2) | Linear | 2026-09-18T22:00:52.668+03:00 | `raw/s7/B4/row_identity_churn__stable__sleeping_off` |
| `row_identity_churn/stable/sleeping_on` | 17.1120 | [16.8215, 17.2640] | 17.1229 | 17.0665 | 255 | 420 (i1=2) | Linear | 2026-09-18T22:01:08.183+03:00 | `raw/s7/B4/row_identity_churn__stable__sleeping_on` |

