# Physics timings, MEASUREMENT-QUEUE sections 6-9 (2026-09-18)

Receipts for the struck entries 6, 8 and 9 and the timed half of entry 7 in
[`docs/MEASUREMENT-QUEUE.md`](../../MEASUREMENT-QUEUE.md). Owner's workstation (Ryzen 9 5900HS, 16 logical
CPUs), `stable-x86_64-pc-windows-msvc` rustc 1.98.1, bench profile, `target-cpu=x86-64-v3` from
`.cargo/config.toml`, no `RUSTFLAGS`. The owner declared the machine quiet; no other job ran.

| File | What it is |
|---|---|
| `ANALYSIS.md` | Every entry read against its own rules, and the struck blocks copied into the queue |
| `timing_raw.md`, `runs.jsonl` | Every timed run: arm, repetition, bench ids, median and CI, CPU load receipts before and after, `estimates.json` write time |
| `raw/<entry>/<arm><n>/` | criterion's `estimates.json` (and `sample.json`) per run, copied before the next run overwrote it |
| `runlogs/` | stdout/stderr of every timed run |
| `build_report.md`, `exes.json`, `logs/` | The arms' detached worktrees, the prebuilt bench executables with sha256, the no-compile proof, the ISA parity proof, the structural receipts |
| `s8_armA_port.diff` | Section 8 arm A: `a56007ab`'s bench ported onto `d552be05` with `contact_wakes` stubbed to 0 |
| `s9_probe*.diff`, `probe/` | Section 9 R2: the contact-point probe (never timed) and its per-step series |
| `*.py`, `*.sh`, `*.ps1` | The drivers, the load receipt and the recompute scripts |

In `build_report.md`, `MQ/` means this directory.
