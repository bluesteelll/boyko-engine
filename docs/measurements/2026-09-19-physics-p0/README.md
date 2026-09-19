# Physics perf campaign P0 — per-stage profile and like-for-like Jolt parity (2026-09-19)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 10. Read `ANALYSIS.md` first: it reduces both windows by
the plan (`docs/physics/perf-campaign/01-PLAN-REV1.md` with `00-RULINGS.md`) and carries the RESULT block
copied into the queue.

| Path | What it is |
|---|---|
| `p0/` | Window 1 (VOID by its own A/A0 rule at W=2: per-process speed shifts, not load) and the build receipts: the five boyko binaries' sha256, the source swaps, the Jolt builds' hashes and compiler |
| `p0b/diag_report.md` | The placement diagnostic: CPU affinity does not narrow the per-process spread |
| `p0b/window_report.md`, `p0b/raw/` | Window 2 under the ruled protocol: K = 6 processes per (row, W), medians, receipts, the broadphase crossover |
| `series.tar.xz` | Every per-step CSV and pose dump of both windows (842 files), packed; `tar -xJf series.tar.xz` restores them in place |

Not kept in the tree: the executables (their sha256 are in `p0/bin/SHA256SUMS`) and Jolt's `-p` HTML charts
(their reduced stage shares are in `ANALYSIS.md` §3).
