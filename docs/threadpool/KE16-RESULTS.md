# KE16 results — the tournament's measured record

Layout per `KE16-DESIGN-MEASUREMENT.md` §9. Decision rules quoted from that file's §7. Candidate
table in `KE16-DESIGN.md`.

**STATUS AT THE TIME OF WRITING.**

| Step | State | Where |
|---|---|---|
| 0 | MEASURED — baseline `a0+b0+w0+c0`, 24-cell grid ×3 runs, ECS at both populations, five physics rows, per-cell band | §0, §Band |
| A | **CLOSED ON `a3`** — five arms measured, then the deciding pair re-measured interleaved in ONE session | §A |
| B | **B0 BY CONSTRUCTION, NOT BY MEASUREMENT** — `b1`/`b3` do not compile over `a3` | §B |
| W1, W2 | **CLOSED ON `wc` 2026-09-08 at CS-4.** `wg` and `wgc` ELIMINATED (0 improvements, 13 and 14 regressions, and an occupancy receipt: 5–6 of 16 lanes, speedup pinned at 2.00×). `wc` is a tie on all 38 cells and rule 2's two gates — a real-park loom M1c and the route-(b) 32-seed liveness gate — are GREEN, so **W\* = `wc`**. ⚠ No physics ranking is filed: the passes carry a monotone warm-up drift and the PRIMARY row's band is 85 % | §W |
| C, F | **CLOSED ON `c0` 2026-09-08.** Both arms DROPPED — neither improves anything anywhere; `c1` runs the wave on **2 of 16 lanes**, `c1f` on 9–10 and still loses 1.86×. Measured as a PAIR, departing from rule 3 with the number that forces it. **Final: `a3+b0+wc+c0`** | §C |
| App | **NOT RUN — OWED.** No number here comes from the unconditional shipped code | §App, §Owed |

This file exists because the tournament ran for weeks and produced no written result. Everything
below is transcribed from the measuring agents' own returned objects and the preserved structured
extracts; nothing is reconstructed from a summary, and nothing is invented. Where the record does
not contain something, it says so.

---

## §0. Environment

### Tree

| | |
|---|---|
| Worktree | `D:/wt/threadpool`, branch `feat/threadpool-ke16` |
| HEAD | `4a363678e1b7fb97d0a9d6b9856678b5ba6a7870` — *"perf: the shipped profile gets fat LTO, and the bench profile is pinned against inheriting it"* |
| Working tree | `git status --porcelain` = **73** entries during the axis-A sessions, **75** during the later Stage-1 / axis-W-C sessions. The campaign's own uncommitted implementation. Identical before and after every pass; no measuring agent wrote into the repository and no git write command was run. |
| `sha256` of `crates/boyko_threadpool/src` at Step 0 | `e8638616257e77aaf9139e4952cf6c1ff660488a8584fc2e6f5782ed8e6294fb`, identical before and after the Step-0 measurement — the code did not move under the numbers |

### Toolchain

```
rustc 1.97.1 (8bab26f4f 2026-07-14)   cargo 1.97.1 (c980f4866 2026-06-30)
host stable-x86_64-pc-windows-gnu     LLVM 22.1.6
miri 0.1.0 (8925ea358a 2026-08-20) on rustc 1.100.0-nightly (8925ea358 2026-08-20),
  x86_64-pc-windows-gnu
```

`RUSTFLAGS` was **never set** on any invocation, and this is load-bearing rather than tidy:
`.cargo/config.toml` in the worktree carries `[target.x86_64-pc-windows-gnu] rustflags =
["-C","target-cpu=x86-64-v3"]` and `~/.cargo/config.toml` carries the machine-local
`dlltool`/`lld` linker flags on the **same key**, which cargo JOINS. Setting `RUSTFLAGS` would
have deleted BOTH — the ISA baseline and the linker fix. Where `--cfg loom` was needed it was
added with `cargo --config target.x86_64-pc-windows-gnu.rustflags=[…]`, which joins.

⚠ **Correction to the design's §0 recorded here so nobody re-derives it.** The brief in circulation
said `.cargo/config.toml` carries "the ISA baseline AND two mandatory linker flags". At this
checkout the worktree file carries the **ISA baseline only**; the linker flags live in
`~/.cargo/config.toml`. The no-`RUSTFLAGS` rule still stands, for the ISA reason.

⚠ **`cargo +nightly` resolves to `nightly-x86_64-pc-windows-MSVC` on this box** (msvc stable is the
rustup default host) and dies in the linker with exit 1, which is indistinguishable from "the gate
is red". Every Miri command spells `+nightly-x86_64-pc-windows-gnu`.

### Profile — every absolute in this file is a bench-profile number

```
Cargo.toml:93-94    [profile.release]  lto = "fat"
Cargo.toml:96-110   [profile.bench]    codegen-units = 1 , lto = false
```

`cargo bench` and `cargo build --release` are **different codegen configurations by design** at
this HEAD, and the manifest says so itself at :104-108. Bench optimises for variance, release for
speed; on the cross-crate-inlining axis the bench binary is the *worse* optimised of the two.
**Ratios between arms transfer** — both arms of every comparison were built under the identical
profile. **Absolutes do not describe the shipped binary and must not be quoted as if they did.**

### Topology (App-9)

AMD Ryzen 9 5900HS with Radeon Graphics — **8 physical cores / 16 logical (SMT)**, max clock
3301 MHz, L2 4096 KB total (512 KB/core), L3 16384 KB, 15.4 GB RAM, Windows 11, High-performance
power scheme, on AC. `available_parallelism` = **16** on every run of every pass, printed by every
bench binary and every gate. W was never overridden and no affinity mask was applied.

⚠ This is a **mobile (HS) part**: the 16 lanes are 16 SMT threads on 8 cores. Every "reaches W"
claim and the whole physics acceptance line must be read against that.

### `park_timeout` medians (App-7 backstop truth)

| Configuration / session | 50 µs | 1 ms | 2 ms |
|---|---|---|---|
| `a0+b0+w0+c0`, Step 0 | 15.585 ms | 15.596 ms | 15.574 ms |
| `a3+b0+w0+c0`, head-to-head | 15.505 ms | 15.520 ms | 15.532 ms |
| `a1f+b0+w0+c0`, head-to-head | 15.550 ms | 15.522 ms | 15.522 ms |
| `a3+b0+w0+c0`, axis-W/C reference (post-Stage-1) | 11.160 ms | 11.479 ms | 11.921 ms |

At Step 0 the three rows are indistinguishable and equal the **64 Hz (15.625 ms) unguarded Windows
timer tick**: a `park_timeout` of any nominal duration below ~15.6 ms costs ~15.6 ms on this box —
the 50 µs request is overshot **312×**. Every candidate whose design leans on a sub-millisecond
park (B1's pre-park snooze, B3's timed park, W-b's cascade hop latency) is leaning on that floor.

⚠ Two instrument warnings attached to these rows, both measured:
* The bench's own **single-shot probe disagrees with its own criterion median by 2.4×**
  (6827 µs / 6270 µs against a 15.58 ms median). The probe is one sample at a random phase of the
  tick. Use the median; keep the probe beside it, never instead of it.
* The printed `KE16 park_timeout configuration=guarded|unguarded` **label is not a state
  classifier**. It flipped mid-session on `a1f`, on `a3` (twice in ten minutes), on `a5` and on the
  in-session `a0` control while the criterion medians held one sustained 15.5–15.6 ms state
  throughout. It is one 50 µs probe against a 4 ms ceiling at a uniformly random phase of a
  15.625 ms tick, so ~26 % of probes read "guarded" on a fully unguarded box. Two independent
  measurers derived this from their own data.

### `MIRIFLAGS`, echoed verbatim (§5 item 12)

```
MIRIFLAGS=-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks
MIRIFLAGS=-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..16
```

Every string carries `-Zmiri-tree-borrows`. ⚠ `MIRIFLAGS` **replaces** `.cargo/config.toml`'s
`[env]` table rather than merging with it, which is why every command spells the whole string.

### Loom calibration readings (§8 Step-0 requirement, §5 item 15)

At `a0+b0+w0+c0` — the `--cfg loom` configuration **built and ran**, which this crate has been dark
in before:

```
running 5 tests
test loom_m1_fork_join_no_lost_wakeup ... ok
test loom_m2_calibration_no_producer_fence_is_lost - should panic ... ok
test loom_m2_idle_race_c_no_lost_wakeup ... ok
test loom_m2b_idle_cas_contention_exactly_one ... ok
test loom_m3_shutdown_handshake_worker_exits ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; finished in 15.34s
```

* `loom_m2_calibration_no_producer_fence_is_lost` went RED as required, and its attribute at
  `tests/loom_pool.rs:387` is `#[should_panic(expected = "M2: lost wake")]` — not a bare
  `#[should_panic]` — so the "ok" certifies the un-fenced copy reproduced the lost wake **with its
  own oracle's message**. §5 item 15 satisfied; the model can fail, so its green is a gate.
* The M4 pair (`:1403`, `:1461`), M2c (`:677`) and M1c (`:1570`) are `cfg(feature)`-gated and
  correctly ABSENT at a0. All carry `#[should_panic(expected = …)]` with their own oracle prefixes.
* ⚠ A THIRD `should_panic` row exists that is **NOT a calibration copy**:
  `loom_m4c_stale_snapshot_strands_a_task_when_the_pusher_is_not_a_lane` at `:1520`,
  `#[should_panic(expected = "M4: lost wake")]`, documented in its own header as a row that RECORDS
  a design finding. Feature-gated, absent at a0. Flagged so no later step counts it as calibration.
* Same reading reproduced in the axis-W/C reference session (`a3+w0`, 5 models).
* Later, at the post-Stage-1 code with `ke16-w-gate,ke16-w-count`, the loom binary lists **11**
  models and all 11 pass; the M4 family takes 21.63 / 145.02 / 71.22 / 248.86 / 30.72 s, which is
  the receipt that models are actually EXPLORED rather than skipped.

---

## §CS. Which code state each number describes

**Read this before any number below.** Two code changes landed mid-tournament. A reader who mixes
them compares across a rewrite without knowing it.

| Tag | Code state | What was measured on it |
|---|---|---|
| **CS-1** | AFTER the `complete_task` protector fix (`complete_task` takes `*const Self`), **BEFORE** Stage 1 of the task representation | Step 0 baseline; axis A's five arms `a0/a1/a1f/a2/a3/a5`; the `a3`-vs-`a1f` head-to-head that closed axis A |
| **CS-2** | AFTER Stage 1 — the queue element is now `Task { payload: *const (), execute: unsafe fn(*const ()) }` | The axis-W/C reference `a3+b0+w0+c0` (49 rows); the Stage-1 structural certification |
| **CS-3** | The unconditional code after feature removal | **NOTHING. Step App has not been run.** |
| **CS-4** | AFTER stage 3b (`d51b4ced`) — the scoped task cell is emplaced into a per-scope `ScopeBlock` and released by one `free_all` in `Scope::drop`, instead of one `alloc`/`dealloc` per task | **Step W**: the re-taken reference `a3+b0+w0+c0` and the three W arms, 3 interleaved passes (§W below) |

⚠ **CS-4 is why §W's reference was RE-TAKEN rather than read off the CS-2 table.** The 49-row CS-2
reference describes a tree in which every spawned task carried its own heap allocation; stage 3b
removed that from the scoped path. Comparing a CS-4 arm against the CS-2 reference would be exactly
the mixing this section exists to forbid. The CS-2 numbers stay below, unchanged, and are not
comparable to §W's.

### Stage 1's measured effect on the task path

Instruction-level, from the disassembly, not from a timing:

* **Dispatch: instruction-count UNCHANGED.** The old `callq *24(%rdx)` loaded its target from the
  closure's vtable — a second object on a different `.rdata` line — and that load was already
  folded into the call's memory operand. It becomes a tail `jmpq *%rax` off the element's own
  second word. What is removed is **one level of the dependency chain** (a load from a second
  object), and the call **becomes tail-callable** because nothing has to happen after it. The
  shipped receipt agrees: `Scope::drop`'s inlined `run_task` site emits
  `movq 80(%rsp),%rcx ; callq *88(%rsp)` — the call's memory operand is the element's own spilled
  second word, not a vtable slot.
* **Spawn: +3 instructions and +8 payload bytes** (22 → 25 instructions; allocation 16 → 24 B).
* Structural pins, enforced as compile-time asserts in the release object:
  `size_of::<Task>() == 16`, `align_of::<Task>() == 8`, `size_of::<Option<Task>>() == 16` — so a
  release build that links is a receipt that the queue element did not grow and that
  `Injector`/`Worker`/`Stealer` still move the same 16 bytes per `read_volatile`/`write_volatile`.

**The honest Stage-1 ledger is therefore "+3 instructions and +8 B per spawn, −1 level in the
call's dependency chain"** — not an instruction win. The framing "it removes one dependent load and
changes nothing else" was corrected during review and the correction is recorded here.

### ⚠ The one comparison this section exists to forbid

`a3`'s physics `in_scheduled_system` reads **11.709 ms** in the CS-1 head-to-head and **10.019 ms**
in the CS-2 axis-W/C reference. **These two numbers are not comparable and the difference is not a
Stage-1 effect.** Different sessions, different code, and the arm-independent `single_threaded_O5`
row drifted up to 5.9 % within the CS-2 session alone. The measuring agent that took the 10.019 ms
figure said so explicitly and offered it "for context only, NOT as a comparison I performed".

---

## §Band. The per-cell band table — Step 0, `a0+b0+w0+c0` (CS-1)

`band(c) = max(0.04, |med_r1 − med_r2| / min(med_r1, med_r2))` per §4. Medians in ns from
`target/criterion/<group>/<cell>/a0+b0+w0+c0-r{1,2}/estimates.json`. **`r3` is a THIRD run taken as
a diagnostic; it is NOT part of the §4 band** and is given so a later reader can see the drift.
W = 16 on every run. Every number carries its baseline name.

| cell | med r1 (ns) | med r2 (ns) | BAND | MAD/m r1 | MAD/m r2 | bimodal |
|---|---:|---:|---:|---:|---:|:--:|
| `dispatcher/body_1us_tasks_w` | 5 794.9 | 4 725.7 | 22.63 % | 0.090 | 0.015 | — |
| `dispatcher/body_1us_tasks_4w` | 17 068.5 | 15 430.2 | 10.62 % | 0.077 | 0.017 | — |
| `dispatcher/body_1us_tasks_64w` | 241 850.6 | 216 681.6 | 11.62 % | 0.075 | 0.002 | — |
| `dispatcher/body_10us_tasks_w` | 24 048.9 | 19 038.7 | 26.32 % | 0.097 | 0.080 | — |
| `dispatcher/body_10us_tasks_4w` | 144 289.2 | 125 998.4 | 14.52 % | 0.035 | 0.078 | — |
| `dispatcher/body_10us_tasks_64w` | 798 133.0 | 733 356.2 | 8.83 % | 0.025 | 0.002 | — |
| `dispatcher/body_100us_tasks_w` | 165 420.9 | 133 204.8 | 24.19 % | 0.080 | 0.032 | — |
| `dispatcher/body_100us_tasks_4w` | 1 949 146.8 | 1 935 534.0 | 4.00 % | 0.123 | 0.103 | — |
| `dispatcher/body_100us_tasks_64w` | 7 011 768.8 | 6 699 975.0 | 4.65 % | 0.016 | 0.021 | — |
| `dispatcher/body_1ms_tasks_w` | 1 345 142.9 | 1 233 164.2 | 9.08 % | 0.098 | 0.027 | — |
| `dispatcher/body_1ms_tasks_4w` | 13 942 829.2 | 12 538 282.7 | 11.20 % | 0.331 | 0.157 | **YES** |
| `dispatcher/body_1ms_tasks_64w` | 66 725 188.8 | 66 040 162.5 | 4.00 % | 0.010 | 0.017 | — |
| `worker/body_1us_tasks_w` | 42 851.8 | 36 735.2 | 16.65 % | 0.029 | 0.004 | — |
| `worker/body_1us_tasks_4w` | 106 940.4 | 97 644.4 | 9.52 % | 0.059 | 0.008 | — |
| `worker/body_1us_tasks_64w` | 1 168 612.8 | 1 135 769.9 | 4.00 % | 0.012 | 0.008 | — |
| `worker/body_10us_tasks_w` | 186 635.5 | 181 716.7 | 4.00 % | 0.011 | 0.001 | — |
| `worker/body_10us_tasks_4w` | 688 551.9 | 676 193.1 | 4.00 % | 0.010 | 0.002 | — |
| `worker/body_10us_tasks_64w` | 10 468 016.7 | 10 357 870.0 | 4.00 % | 0.006 | 0.001 | — |
| `worker/body_100us_tasks_w` | 1 627 253.1 | 1 622 796.4 | 4.00 % | 0.003 | 0.000 | — |
| `worker/body_100us_tasks_4w` | 6 471 294.4 | 6 444 022.2 | 4.00 % | 0.004 | 0.001 | — |
| `worker/body_100us_tasks_64w` | 102 712 950.0 | 102 571 025.0 | 4.00 % | 0.001 | 0.000 | — |
| `worker/body_1ms_tasks_w` | 16 052 266.7 | 16 043 480.0 | 4.00 % | 0.001 | 0.000 | — |
| `worker/body_1ms_tasks_4w` | 64 099 125.0 | 64 073 475.0 | 4.00 % | 0.000 | 0.000 | — |
| `worker/body_1ms_tasks_64w` | 1 024 554 925.0 | 1 024 273 700.0 | 4.00 % | 0.000 | 0.000 | — |
| `par_iter_in_system/seq/4096` | 81 945 655.0 | 81 942 405.6 | 4.00 % | 0.000 | 0.000 | — |
| `par_iter_in_system/par_from_dispatcher/4096` | 31 659 982.1 | 29 168 925.6 | 8.54 % | 0.255 | 0.155 | **YES** |
| `par_iter_in_system/par_in_system/4096` | 81 960 226.2 | 81 979 233.0 | 4.00 % | 0.000 | 0.000 | — |
| `par_iter_in_system/seq/65536` | 1 310 985 650.0 | 1 311 043 750.0 | 4.00 % | 0.000 | 0.000 | — |
| `par_iter_in_system/par_from_dispatcher/65536` | 84 335 208.3 | 84 436 630.0 | 4.00 % | 0.003 | 0.004 | — |
| `par_iter_in_system/par_in_system/65536` | 1 311 142 300.0 | 1 311 058 850.0 | 4.00 % | 0.000 | 0.000 | — |
| `park_timeout_50us` | 15 584 938.0 | 15 587 186.5 | 4.00 % | 0.012 | 0.004 | — |
| `park_timeout_1ms` | 15 612 894.6 | 15 580 203.6 | 4.00 % | 0.009 | 0.008 | — |
| `park_timeout_2ms` | 15 573 789.3 | 15 574 952.5 | 4.00 % | 0.007 | 0.009 | — |
| `solve_route/bench_thread_install/29751` | 7 817 272.2 | 7 838 418.1 | 4.00 % | 0.015 | 0.015 | — |
| `solve_route/bench_thread_install_wminus1/29751` | 8 147 451.1 | 8 143 535.7 | 4.00 % | 0.018 | 0.015 | — |
| `solve_route/in_scheduled_system/29751` | 30 047 302.1 | 29 919 425.4 | 4.00 % | 0.011 | 0.008 | — |
| `solve_route/empty_schedule_control/29751` | 1 451.0 | 1 692.9 | 16.67 % | 0.012 | 0.057 | — |
| `solve_route/single_threaded_o5/29751` | 26 220 263.1 | 26 327 098.8 | 4.00 % | 0.009 | 0.005 | — |

**Bimodality (§4, MAD/median > 0.25 on any run) — exactly TWO cells flagged**, both on the
dispatcher/ECS side, both decided on p50:

* `dispatcher/body_1ms_tasks_4w` — r1 p10/p50/p90 = 11 281 475 / 12 206 458 / 19 757 450 (n=10);
  r2 = 10 359 183 / 12 111 808 / 14 340 669.
* `par_iter_in_system/par_from_dispatcher/4096` — r1 = 23 943 742 / 31 033 900 / 39 042 535;
  r2 = 26 551 938 / 28 707 407 / 35 314 183.

NO worker-route cell is bimodal (max MAD/median 0.059). NO physics row is bimodal (max 0.057) and
no physics run showed a ≥ 1 ms outlier class, so the external-arm backstop did not fire.

### ⚠ Step-0 Finding: the §4 band as taken is measuring a first-run WARM-UP, not jitter

All 24 dispatcher-route cells got faster from r1 to r2 (r2/r1 in 0.792–0.993, every one below 1). A
monotone one-directional shift across every cell is not the shape of jitter. A third pool run was
taken to test it, and it confirms:

| cell | r2/r1 | r3/r2 | band(r1,r2) | band(r2,r3) |
|---|---:|---:|---:|---:|
| `dispatcher/body_10us_tasks_w` | 0.792 | 1.035 | 26.32 % | **4.00 %** |
| `dispatcher/body_100us_tasks_w` | 0.805 | 1.009 | 24.19 % | **4.00 %** |
| `dispatcher/body_1us_tasks_w` | 0.815 | 1.001 | 22.63 % | **4.00 %** |
| `dispatcher/body_10us_tasks_4w` | 0.873 | 1.024 | 14.52 % | **4.00 %** |
| `dispatcher/body_1us_tasks_64w` | 0.896 | 1.019 | 11.62 % | **4.00 %** |
| `dispatcher/body_1us_tasks_4w` | 0.904 | 1.035 | 10.62 % | **4.00 %** |
| `dispatcher/body_10us_tasks_64w` | 0.919 | 1.003 | 8.83 % | **4.00 %** |
| `worker/body_1us_tasks_w` | 0.857 | 0.999 | 16.65 % | **4.00 %** |
| `worker/body_1us_tasks_4w` | 0.913 | 0.997 | 9.52 % | **4.00 %** |
| `dispatcher/body_1ms_tasks_4w` | 0.899 | 1.120 | 11.20 % | 11.99 % |
| `dispatcher/body_1ms_tasks_w` | 0.917 | 0.911 | 9.08 % | 9.82 % |
| `dispatcher/body_100us_tasks_64w` | 0.956 | 1.045 | 4.65 % | 4.48 % |

18 of the 24 cells read the 4.00 % floor on r2↔r3. The recorded band was NOT altered — the table
above is §4's formula applied literally, because changing the specification is not the tester's
job — but the consequence is on record: **§4's band is the denominator of every elimination rule
from Step A to Step F, and nine of the twelve dispatcher bands plus two of the twelve worker bands
are dominated by the cost of being the first criterion process of a session.** If every
configuration's r1 is likewise cold the inflation is uniform and the rule is merely conservative;
if a later step happens to run its r1 warm it gets a band 6× tighter on the same cell and the same
measured difference flips from "tie" to "regression". **Every later pass in this campaign
independently adopted a discard-r1 protocol.** The effect is confined to the routes that use all 16
lanes: the worker route at ≥ 10 µs is serial at a0 and reads 0.974–1.000 on r2/r1, with the
1 ms × 64W cell reproducing to 0.03 %.

### Step-0 Finding: defect A is exact, not approximate

* Pool worker route, 1024 tasks × 1 ms body: **1024.26 ms** against a **1024.0 ms** serial floor —
  0.03 %, reproduced to 0.03 % across all three runs. The worker route is not "slow", it **is** the
  serial floor.
* ECS: `par_in_system` equals `seq` to four significant figures at BOTH populations (81.97 vs
  81.95 ms; 1310.98 vs 1310.99 ms), with `max_in_flight = 1` both times.
* Occupancy: `max_in_flight=1, lanes_used=1, top_lane=tasks, off_pool=0` on **every** worker-route
  repetition at both W=4 and W=16.
* Defect B's signature is visible and bimodal on the **dispatcher** route at W=16: `top_lane` =
  10, 33, 33 across three reps (the ≤ 33 crossbeam batch), with `off_pool` tracking it exactly.

⚠ **Caveat for the ECS 4096-row column:** `par_from_dispatcher` reaches only `max_in_flight = 4`
there (the chunking yields 4 chunks), so **N = 4096 is a 4-LANE comparison**. Only N = 65536 is a
16-lane one. A speedup read at 4096 as if it were out of 16 is wrong by 4×.

### Step-0 Finding: the physics acceptance line FAILS at the baseline, as designed

* **Clause (1):** `in_scheduled_system` 29.98 ms is NOT below `single_threaded_O5` 26.27 ms. It is
  **1.141× worse** — enabling parallel physics on the shipping route costs **14.1 % more** than
  leaving it off.
* **Clause (2) PRIMARY**, `REF = bench_thread_install_Wminus1` because B is B0 (Step B rule 4:
  W−1 workers + a HELPING external joiner = W lanes): `(29.98 − 0.0016) / 8.145 = 3.68×` against an
  allowance of 1.04×.
* Reported raw beside it, against the OTHER row (`bench_thread_install`, W+1 lanes under B0/B1):
  `29.98 / 7.83 = 3.83×`.
* Row-ratio receipt `Wminus1 / install = 8.145 / 7.83 = 1.040` — near 1, i.e. the external arm IS
  helping, the correct B0/B1 reading. The `≈ W/(W−1) = 1.067` check of Step B rule 4 is a B3-only
  receipt and is not applied here.

---

## §A. Axis A — CLOSED ON `a3`

`a3` = `ke16-a3`, every spawn to `injector_global`; the design's own "control / reachability floor".

Axis A was measured **twice**. The first pass ranked all five arms across sessions and produced a
verdict its own judge refused to call safe. The second pass ran the deciding pair interleaved in a
single session and closed it. Both are recorded; the second supersedes the first **only for the
`a3`/`a1f` pair**.

### §A.1 — First pass: five arms, cross-session (CS-1)

Physics `in_scheduled_system/29751` (ms) and the six worker decision cells (ns, warm medians):

| | `a1` | `a1f` | `a2` | `a3` | `a5` |
|---|---:|---:|---:|---:|---:|
| physics RAW | 16.411 | 11.891 | **10.151** | 10.354 | 11.784 |
| physics ÷ own-run `O5` | 0.5635 | 0.4271 | 0.3782 | **0.3735** | 0.4231 |
| physics ÷ `REF` | 1.3309 | 1.1944 | 1.2127 | **1.0305** | 1.1710 |
| own physics band | 13.48 % | 6.82 % | 4.09 % | 4.00 % | 4.00 % |
| worker 1 µs × W | 9 686 | 8 976 | **8 727** | 9 042 | 9 920 |
| worker 1 µs × 4W | 22 240 | 20 704 | 21 762 | **20 276** | 20 832 |
| worker 1 µs × 64W | 245 977 | 244 316 | **224 903** | 239 086 | 232 745 |
| worker 10 µs × W | 28 003 | **26 800** | 27 765 | 29 145 | 28 769 |
| **worker 10 µs × 4W** | 172 646 | 176 364 | 182 994 | **111 279** | 181 170 |
| worker 10 µs × 64W | 775 650 | 751 406 | **739 907** | 748 464 | 767 048 |
| ECS ratio 4096 — wall | 0.97 / 0.96 | 1.0005 | 0.990 | 0.974 / 0.999 | (not reported) |
| ECS ratio 65536 — wall | 0.94 / 1.04 | 0.9933 | 0.996 | 0.959 / 0.980 | (not reported) |
| ECS ratio 4096 — criterion | 1.067 / 1.000 | 0.828 / 1.012 / 1.039 | 0.818 | 1.003 / 0.953 | 1.004 |
| ECS ratio 65536 — criterion | 1.034 / **1.150** | 1.143 / **1.309** / 1.139 | **1.234** | 1.012 / 1.115 | 1.000 |
| acceptance clause (1) | PASS | PASS | PASS | PASS | PASS |
| acceptance clause (2) | FAIL 1.331 | FAIL 1.181 | FAIL 1.213 | ⚠ PASS 1.030 | FAIL 1.171 |

**The deciding cell, re-derived from scratch.** `worker/body_10us_tasks_4W` (64 tasks × 10 µs over
W=16; ideal 40 µs):

```
a3 111 279 ns  |  a1 172 646  |  a1f 176 364  |  a5 181 170  |  a2 182 994
```

`a3` is 1.55–1.64× ahead of every other arm and every other arm is within 6 % of every other. That
is a **mechanistic grouping**, not a noisy cell: `a3` is the only arm that reaches all sixteen lanes
at once from a shared injector; `a1`, `a1f`, `a2` and `a5` all drain a per-worker queue in ≤ 33-element
batches and land on top of each other. Applied bands are 4.00–4.73 %, so the grouping sits 7–20×
outside the noise, and the pool harness's session stability was verified independently — `a1f`'s
in-session `a0` control reproduces this exact cell to 0.2 % (677 801 vs 676 193 ns).

**Every one of the five arms converts acceptance clause (1) from FAIL to PASS**, which is what
axis A exists for. Defect A is closed on all of them: the worker route goes from exactly the serial
floor (1024.26 ms against 1024.0, `max_in_flight=1`, `lanes_used=1`) to 65.7–66.0 ms at
`max_in_flight=16` (15.5–15.6× of a possible 16×), and ECS `par_in_system` from
bit-identical-to-`seq` at both populations to a 14.0–15.3× speedup.

**⚠ The verdict this pass returned was `a3`, and its judge refused to call it safe.** `a1f` was
eliminated by rule 2 at **14.84 % raw / 14.36 % normalised against a 13.64 % threshold — a margin
of 1.20 / 0.72 percentage points**, between two arms measured in DIFFERENT sessions on the one
harness that the same pass had PROVEN is not session-comparable in absolutes. The judge named the
single measurement that would settle it: an `a3`-vs-`a1f` head-to-head in one session. §A.2 is
that measurement.

### §A.2 — Second pass: `a3` vs `a1f`, INTERLEAVED IN ONE SESSION (CS-1) — this is what closed axis A

**Design.** Strict alternation in one continuous session: `a3` p1, `a1f` p1, `a3` p2, `a1f` p2,
`a3` p3, `a1f` p3, `a3` p4, `a1f` p4. Each pass = three bench invocations (pool → ECS → physics) in
fixed order. **Separate `CARGO_TARGET_DIR` per arm** (`D:/tmp/ke16_h2h_a3`, `D:/tmp/ke16_h2h_a1f`),
both built with `cargo bench --no-run` up front, so alternation cost no rebuild. 24 timed
invocations, 06:23:05 → 06:47:38, 24 min 33 s of continuous measurement with no gap longer than
19 s. Each arm's p1 is discarded by a **pre-fixed** rule; three kept passes per arm.

**Per-arm table.** Medians of kept passes p2/p3/p4; spread = pass-to-pass (max−min)/min;
band = max(0.04, larger arm spread); verdict at 2×band.

| Quantity | `a3` | `a1f` | a1f/a3 | 2×band | verdict |
|---|---:|---:|---:|---:|---|
| **PRIMARY physics `in_scheduled_system`** | **11.709 ms** (4.48 %) | **13.640 ms** (2.78 %) | 1.1649 | 1.0895 | **a1f REGRESSES** |
| same, normalised by own `single_threaded_O5` | 0.41731 (2.70 %) | 0.48892 (3.76 %) | 1.1716 | 1.0800 | **a1f REGRESSES** |
| **worker 10 µs × 4W (deciding cell)** | **114 960 ns** (6.09 %) | **159 470 ns** (18.23 %) | 1.3872 | 1.3646 | **a1f REGRESSES** |
| worker 1 µs × W | 9 291.6 (2.18 %) | 10 029.4 (16.45 %) | 1.0794 | 1.3290 | tie |
| worker 1 µs × 4W | 20 651.1 (3.09 %) | 22 231.1 (2.02 %) | 1.0765 | 1.0800 | tie (by 0.35 %) |
| worker 1 µs × 64W | 235 910 (1.97 %) | 249 336 (2.11 %) | 1.0569 | 1.0800 | tie |
| worker 10 µs × W | 29 386.8 (1.71 %) | 28 360.9 (4.89 %) | 0.9651 | 1.0978 | tie (a1f faster) |
| worker 10 µs × 64W | 754 928 (1.10 %) | 759 389 (1.53 %) | 1.0059 | 1.0800 | tie |
| worker 100 µs × 4W (unnominated) | 1 579 742 (1.61 %) | 1 984 008 (4.73 %) | 1.2559 | 1.0946 | a1f regresses |
| worker 1 ms × 4W (context) | 14 318 906 (17.84 %) | 16 887 768 (26.98 %) | 1.1794 | 1.5396 | tie (noisy) |
| METER 1 `single_threaded_O5` | 28.096 ms (1.86 %) | 27.905 ms (0.99 %) | 0.9932 | — | arm-independent, 0.68 % apart |
| METER 2 `Wminus1/install` ratio | 0.9965 | 1.0315 | — | — | both ≈ 1.0, per-pass interleaved |
| ECS ratio, protocol wall 4096 / 65536 | 1.0015 / 1.0371 | ≈1.02 / ≈1.02 | — | ≤ 1.15 | both PASS |
| ECS ratio, criterion 65536 | 1.006 | 1.333 | — | ≤ 1.15 | a3 PASS, a1f FAIL |
| acceptance line (2), indicative | 93.5 % of budget PASS | 105.4 % FAIL | — | — | a3 clears with defect B live |
| `park_timeout` 50 µs / 1 ms / 2 ms | 15.505 / 15.520 / 15.532 ms | 15.550 / 15.522 / 15.522 ms | ≈1.00 | — | agree to 0.3 % |
| all 12 dispatcher cells | — | — | 0.81–1.04 | — | **every one a tie** |

**Per-pass values, because the disjointness is the strongest statement available:**

```
PRIMARY  a3  11.709 / 12.019 / 11.504 ms   range [11.504, 12.019]   spread 4.477 %
         a1f 13.774 / 13.402 / 13.640 ms   range [13.402, 13.774]   spread 2.776 %
DECIDING a3  114 960 / 117 431 / 110 687 ns  range [110 687, 117 431]  spread 6.093 %
CELL     a1f 142 305 / 168 246 / 159 470 ns  range [142 305, 168 246]  spread 18.229 %
```

**The pre-registered abort rule — margin inside the pass-to-pass spread ⇒ "unresolved" — DID NOT
FIRE.** Margin / spread = **3.68×** raw and **4.56×** normalised on the primary; **2.12×** on the
deciding cell. Stronger and independent of any statistic: **the ranges are DISJOINT.** `a3`'s
slowest kept pass (12.019 ms) is 11.5 % faster than `a1f`'s fastest kept pass (13.402 ms); six kept
passes, six outcomes, no crossing, and the two discarded p1 passes (11.397 vs 12.954) fall on the
same side. On the deciding cell `a3`'s worst pass is 21.18 % faster than `a1f`'s best; all eight
passes including both p1's fall on the same side.

**Sensitivity, stated because one of the two is thin.** To turn each finding into a tie: physics
would need a band of 8.25 %, i.e. one arm's spread at 1.84× the observed worst — robust. The
deciding cell would need a band of 19.36 % against `a1f`'s observed 18.23 % — **only 1.13 pp more of
`a1f`'s own noise; THIN.** The load-bearing arithmetic is rule 2 on physics, not the deciding cell.
Had rule 3 been the only lever, an all-tie grid would run to rule 4 and back to `a1f`. Rule 2 orders
the pair before rule 3 is entered, so that path never opens — but it is one percentage point of
`a1f`'s jitter away from opening.

**Did the deciding cell's gap shrink?** Yes: 1.5889× cross-session → 1.3872× interleaved, about a
fifth of itself, and the shrinkage came almost entirely from `a1f`'s side (176 364 → 159 470,
−9.6 %) while `a3` barely moved (~111 000 → 114 960, +3.6 %). That is the shape of a session offset
being removed from `a1f`, not of a mechanism disappearing.

**The primary RATIO did not move at all:** 12.14/10.35 = 1.1729 cross-session versus
13.640/11.709 = **1.1649** here — 0.68 % apart — even though BOTH arms sit 12–13 % higher in
absolute terms (`a3` +13.13 %, `a1f` +12.36 %). The session offset arrived on both arms and
cancelled in the ratio.

### §A.3 — The rules, applied in order, with the number that decided each

Rules quoted from `KE16-DESIGN-MEASUREMENT.md` §7, Step A.

**RULE 1** — *"Discard any candidate whose gates (§8) are red — after classifying the red per §5
item 14."*

Held in abeyance for the shared Miri red, **not applied as a discard**, and the reasoning is on
record: the red is on **all five arms with one signature**, `Scope::prepare`'s task-body wrapper is
byte-identical across them (its only `#[cfg]`s are `ke16-c-batch` and `ke16-w-fanout`, both off at
c0+w0), and item 14(a)'s named consequence — "triggers the A2 fallback" — is unavailable because
`a2` IS the fallback target and reds identically. A literal reading discards all five and leaves
the axis with no survivor and no fallback, an outcome the rule's own consequence clause presupposes
cannot happen. Classified as a **campaign-level defect in the shared completion path**, not an
axis-A gate. See §Miri.

⚠ **Rule 1 is NOT fully discharged for the head-to-head pass.** Only the build, witness and U1
occupancy gates ran there; the §8 loom / Miri / clippy / full-workspace legs were out of scope by
instruction (they are structural verdicts, indifferent to machine load). **Rule 1 runs BEFORE rule
2. A red there would void this ranking rather than qualify it.** Those legs were later run at CS-2
and are green for the threadpool — see §Gates.

**RULE 2** — *"Physics ranks. Order the survivors by physics `in_scheduled_system` median (lower is
better). A candidate that is worse on physics beyond 2× the band than another is behind it,
whatever the grid says — the owner's criterion is throughput on the real consumer, and a variant
that wins every 1 µs cell but loses the consumer loses."*

| Eliminated | Number that decided it | Arithmetic |
|---|---|---|
| **`a5`** | physics **11.784 ms** vs `a3` 10.354 ms = **1.1381** (13.81 %) | applied band max(4.00 %, 4.00 %) = 4.00 %; threshold 8.00 % → BEHIND. vs `a2`: 1.1610 (16.10 %) vs 8.18 % → BEHIND. Normalised: 0.4232 vs `a3` 0.3735 = 13.28 % → BEHIND. Both readings agree in both directions. |
| **`a1`** | physics **16.411 ms** vs `a3` 10.354 ms = **1.5850** (58.50 %) | applied band max(13.48 %, 4.00 %) = 13.48 %; threshold 26.96 % → BEHIND by 2.2× the threshold. Behind every other arm too (`a2` 61.68 %, `a5` 39.26 %, `a1f` 38.01 %). Even on its FASTER run alone (r7, 15.375 ms) it is 48.5 % behind `a3`. |
| **`a1f`** | head-to-head physics **13.640 ms** vs `a3` **11.709 ms** = **1.16492** | band = max(0.04, larger arm spread) = 4.477 %; threshold 11.709 × 1.08954 = **12.757 ms**; measured 13.640 clears it by 6.92 %, i.e. **7.54 pp of headroom**. Normalised: 0.48892 vs 0.41731 = 1.17160 against 1.0800 — regresses again, and by more. |

`a2` and `a3` are a **declared TIE** under rule 2 (2.01 % raw, 1.26 % normalised, both far inside
2×band = 8.18 %), so rule 2 orders them in neither direction and hands them to rule 3.

**RULE 3** — *"The grid vetoes, symmetrically, within the physics band … if V regresses any such
cell against U while U regresses none against V, V is behind U."*

| Eliminated | Number that decided it | Arithmetic |
|---|---|---|
| **`a2`** | `worker/10us_4W`: **182 993.5** vs `a3` **111 278.65 ns** = **1.6445×** | applied band max(4.00 %, 4.00 %) = 4.00 %, threshold 1.08 — **exceeded by 8×**. `a3` regresses none of `a2`'s six cells (worst 1 µs × 64W at 6.31 %; 1 µs × 4W in `a2`'s favour reaches only 7.33 %, inside 8.00 %). The verdict survives a 2.7× widening of the band. |

In the head-to-head, rule 3 was **not reached** — it applies only among candidates within 2× the
band on physics, and rule 2 established `a3`/`a1f` are not. Applied anyway in both directions over
all six worker 1 µs / 10 µs cells:

```
1us_W    band 0.1645 | a1f/a3 1.0794 | TIE
1us_4W   band 0.0400 | a1f/a3 1.0765 | TIE  (misses the 1.08 threshold by 0.35 % — 72 ns)
1us_64W  band 0.0400 | a1f/a3 1.0569 | TIE
10us_W   band 0.0489 | a1f/a3 0.9651 | TIE  (a1f faster)
10us_4W  band 0.1823 | a1f/a3 1.3872 | A1F REGRESSES (threshold 156 872 ns, measured 159 470)
10us_64W band 0.0400 | a1f/a3 1.0059 | TIE
```

`a1f` regresses one cell against `a3`; `a3` regresses **none**. Rule 3's first clause puts `a1f`
behind `a3` and the 64W tie-break column is never needed. **The dispatcher route is not a veto for
this pair** (rule 3 reserves the dispatcher 1 µs cells for `a2` and `a5` only), and in fact all
twelve dispatcher cells are ties.

⚠ **THIS IS THE STRUCTURAL DIFFERENCE FROM THE CROSS-SESSION PICTURE, and it is why the
head-to-head existed.** There, `a3` regressed `worker/10us_W` at 1.0875× and `a1f` regressed
`10us_4W` at 1.5849× — a MUTUAL regression, a silent 64W tie-break column, one 10 µs cell each ⇒
tie ⇒ rule 4 ⇒ `a1f`, which would have reversed both the winner and rule 6. Interleaved, `a3`'s
`10us_W` regression **evaporates**: 29 386.8 vs 28 360.9 = 1.0362× against a 2×band of 1.0978. The
mutuality that produced the reversal was a session artefact; only `a1f`'s side of it reproduces.

**RULE 4** (tie-break, which would have handed the axis to `a1f`) — **UNREACHABLE.** It applies
only "among candidates still tied after rules 2–3", and they are not tied under either rule. The
second reversal path — index §3's "if a deque arm is within the band of the winner it is taken,
because B1 exists only on a1/a1f" — is closed by the same arithmetic: `a1f` sits at 1.1649×,
outside band (1.0448×) and outside 2×band (1.0895×).

**RULE 5** — *"The ECS ratio at both populations must be ≤ 1.15 for the winner."* `a3` PASSES on
**both** sources at **both** populations in **every** run — protocol wall N=4096 max 1.0015,
N=65536 max 1.0371; criterion rows N=65536 per pass 1.004 / 1.000 / 1.007 / 1.036. `a1f` passes on
the wall (1.00–1.02) and FAILS on the criterion rows in all four passes (1.226 / 1.228 / 1.333 /
1.354). Because the winner passes on both, the unsettled source dispute does not touch the verdict.

**RULE 6** — *"If the winner is not `a1`/`a1f` by more than the band on physics: STOP, report."*
**FIRES.** The winner is `a3`; it beats `a1f` on physics by 16.49 % against a band of 4.48 % — more
than the band and more than 2× it.

### §A.4 — Verdict

> **The abort rule did not fire; `a3` wins by rule 2 with 3.68× its worst pass-to-pass spread and
> disjoint per-pass ranges, rule 3 agrees one-sidedly, rule 4 is unreachable, and rule 6 fires — so
> axis A closes on `a3` and axis B stops.**

Structural proof the two head-to-head builds were different artefacts, independent of any string
either printed:

```
md5 D:/tmp/ke16_h2h_a3/release/deps/ke16_nested_scope-7c16102fff7d1c76.exe  = e82ef49d30fa5ef31fb30f59db6ed718
md5 D:/tmp/ke16_h2h_a1f/release/deps/ke16_nested_scope-36826e7259efaa96.exe = 053f4db8ae5531a627d411e513b60849
```

Different cargo metadata hashes in the filenames, different content hashes, two separate target
dirs never cross-linked. The `KE16_EXPECT` witness was verified in **both** directions: crossing
the tokens produced `EXIT=101` and *"the `--features` line did not take; the run is void"* on each
binary, so the witness can fail on exactly the mislabelling it exists to forbid.

### §A.5 — Acceptance line at the winner (indicative; the formal take is Step App)

`a3+b0+w0+c0`, head-to-head numbers, **defect B live**:

* **Clause (1):** `in_scheduled_system` **11.709 ms** < `single_threaded_O5` **28.096 ms** —
  **PASS by 2.4×.** Parallel physics on the shipping route is a win, not the 14.1 % loss recorded
  at Step 0.
* **Clause (2) PRIMARY**, `REF = bench_thread_install_Wminus1` (the W-lane row, fixed by lane count
  for B0): `11.7089 − 0.0016 = 11.7073 ms` against `11.9903 × 1.04477 = 12.5271` — **PASS, at
  93.5 % of budget.** Raw beside it, per the rule: the ratio against `bench_thread_install` (W+1
  lanes under B0) is **0.9730**.
* Against the Step-0 shipping-route reference of 36.02 ms, `a3+b0` is **3.08× faster**; against the
  31.52 ms single-threaded reference, **2.69×**.
* For contrast, one more independent fact against the loser: **`a1f+b0` FAILS clause (2)** —
  13.6385 ms against 12.9413 = 105.4 % of budget.

⚠ **CLAUSE (2) RANKED NOTHING IN THE FIRST PASS AND MUST NOT BE READ AS IF IT DID.** `REF` moved
between arms on a row axis A cannot touch — a0 8.145 · `a2` 8.370 (+2.8 %) · `a1f` 9.956 (+22.2 %) ·
`a3` 10.048 (+23.4 %) · `a5` 10.064 (+23.6 %) · `a1` 12.331 (+51.4 %) — and a SLOWER `REF` makes
clause (2) EASIER. The counterfactual is decisive: give `a3` `a2`'s `REF` and `a3` reads
10.353/8.370 = 1.237 → FAIL; give `a2` `a3`'s `REF` and `a2` reads 10.150/10.048 = 1.010 → PASS.
The entire clause-(2) ordering in §A.1 is the ordering of which session each arm drew its `REF`
from. It flattered `a3`, the winner — and the winner does not rest on it: `a3` wins on the pool
grid, whose session-stability was verified to < 1 % on 22 of 24 cells.

---

## §Method. The methodological finding that matters more than the verdict

**THE DOMINANT ERROR TERM IN THIS CAMPAIGN WAS SESSION DRIFT, NOT AMBIENT LOAD — AND THE CAMPAIGN
ASSUMED THE OPPOSITE.**

The head-to-head that closed axis A was run **with a game running, at the owner's instruction**,
and it was **tighter** than the quiet cross-session runs. The arm-independent control
`single_threaded_O5` — which no axis-A candidate can touch, so any movement in it is session noise —
reads:

| | within one arm | between arms |
|---|---|---|
| Head-to-head, interleaved, **under load** | **1.86 % (`a3`) / 0.99 % (`a1f`)** | **0.68 %** |
| Cross-session, **quiet box** | **17 %** (`a1` r6→r7: 26.862 → 31.387 ms) | **3.8 %** (medians span 26.84–27.85 ms) |

The instrument was roughly an **order of magnitude tighter under load**, because the arms alternated
at ~3-minute granularity through one continuous load: a steady bias lands on both arms and cancels
in a ratio, whereas a session change does not. The preserved primary ratio proves it — 1.1729
cross-session versus 1.1649 interleaved (0.68 % apart) while BOTH absolutes moved 12–13 %. **The
quantity that ranks the arms is the quantity that survived the session change; the quantity that
did not survive (absolute ms) is the one nothing depends on.**

**Corollaries, recorded so the next measurer does not re-learn them:**

1. **Interleaving cancels session drift; two runs minutes apart do not.** The design's own idleness
   precondition already warns that "two runs minutes apart under a steady game load agree with each
   other perfectly and are both wrong" — agreement proves reproducibility, not cleanliness. That
   warning bites the ABSOLUTES here and is accepted. It does not bite the COMPARISON.
2. **The physics harness is NOT session-comparable in absolutes and the pool grid IS.** Measured,
   not assumed: `a1f`'s in-session `a0` control read `in_scheduled_system` 32.530 ms against Step
   0's 29.983 (+8.5 %), `bench_thread_install` 9.685 vs 7.830 (+23.7 %),
   `bench_thread_install_Wminus1` 9.883 vs 8.145 (+21.3 %), `O5` 27.760 vs 26.274 (+5.7 %) — **with
   no features at all**. The shift is not a single scalar (+5.7 % serial, +23.7 % parallel), so no
   one normaliser repairs it. The same control reproduces the Step-0 pool band file to < 1 % on 22
   of 24 cells and to **0.2 %** on the deciding cell. **The instrument that RANKS does not travel
   between sessions; the instrument that DECIDES does.**
3. **A single-threaded control bounds single-thread noise only.** `O5` is a single-threaded row; a
   competitor occupying ~1 lane of 16 costs a 16-thread wave far more than a 1-thread one. That is
   why a second meter was needed — the lane-scarcity ratio
   `bench_thread_install_Wminus1 / bench_thread_install`, which read `a3` 1.0261 / 1.0391 / 0.9965 /
   0.9396 and `a1f` 0.9668 / 1.0315 / 1.0437 / 0.9897: both scatter around 1.0 by ±5 % and their
   per-pass values **interleave** rather than separating into two populations. Under oversubscription
   the scarcer arm would show a systematic rise; neither does.
4. **The one load asymmetry, named rather than buried, and refuted on its own terms.** The game ran
   8–10 % hotter during `a1f`'s physics windows (41.2 / 42.1 / 34.7 game-CPU-s per minute, median
   41.2) than during `a3`'s (35.4 / 37.5 / 37.9, median 37.5); pool and ECS windows were symmetric.
   The asymmetry therefore sits on the PRIMARY number and **favours `a3`**. It does not explain the
   result: **`a1f`'s p4 physics window ran at 34.74 game-CPU-s/min — lighter than every one of
   `a3`'s three physics windows — and still measured 13.640 ms against `a3`'s worst kept pass of
   12.019 ms taken at 37.46 s/min.** The `a1f` pass with the least competition lost to the `a3`
   pass with the most, by 13.49 %, which is larger than the entire load asymmetry.
5. The load was **bench-shaped, not arm-shaped**: the game took ~25–30 s/min during pool passes
   (which saturate all 16 threads and squeeze it) and ~35–42 s/min during physics passes (which
   leave it room), identically in both arms. Session-wide it consumed 654 s over a 1473 s window =
   **0.444 of one logical core**.

---

## §Refuted. Predictions that failed, and a disagreement between instruments

A record that contains only confirmations is not a record.

### The FIFO end discipline lost the cell it was built to win

`KE16-DESIGN.md` predicts `a1f` will "win 1–10 µs × 64W by RMW count", pricing the two ends at
≈ 0.94 (LIFO) versus ≈ 0.09 (FIFO) shared RMWs per task on the consumer shape — an
**order-of-magnitude** difference, one CAS per **batch** against one SeqCst CAS per **stolen
element**.

**Measured, `a1f` ties `a1` on ALL SIX worker decision cells** — 1.0792, 1.0742, 1.0068, 1.0449,
1.0215, 1.0323, every one inside its 2×band. An order-of-magnitude difference in shared RMWs
produced a **2.15 % wall-clock difference** on the deciding cell. And on the cell it was built to
attack, `a1f` **LOSES to `a3` by 1.5849×** (176 364 vs 111 279 ns) cross-session, and loses it again
in the head-to-head at **1.3872×** (159 470 vs 114 960 ns, ranges disjoint, both discarded p1 passes
on the same side).

**The binding constraint on this shape is BATCH GRANULARITY, not CAS count.** `a3` reaches all
sixteen lanes at once from a shared injector; every deque arm is drained in ≤ 33-element batches,
and all four of them land within 6 % of each other. That is a finding for the architect and it
survives the verdict: **the lever is how wide a wave becomes visible, not how cheap each steal is.**

§7 rule 3 says the grid "is the ONLY way `a1` and `a1f` are separated — never by an RMW count". It
could not separate them. The pair was separated only by physics, and `a1`'s physics is the least
trustworthy number in the pass (16.84 % drift on its arm-independent control). **The LIFO-vs-FIFO
question the split exists to answer is still open**, and will stay open while `a3` holds, because
then neither end ever ships.

### `a5`'s predicted veto did not fire either

The three dispatcher 1 µs cells — the veto §7 rule 3 reserves for `a2`/`a5` — read against `a3` as
1.0393 (band 12.55 %, threshold 25.1 %), 1.0361 (threshold 15.6 %) and 0.971: **three ties.** And
one of the three, `dispatcher/1us_64W`, is one of five cells whose reference band is too wide for
§4's rule to ever return a verdict, so the veto is evaluable on only 2 of its 3 cells. `a5`'s
predicted spawner-side cost (a foreign-line push + an unpark syscall per spawn) **did not resolve
above the noise in the median.** It shows in the TAIL instead: `a5`'s `in_scheduled_system` max
samples are 15.296 ms and 17.190 ms against p50s of 11.62 / 11.95 ms — and **15.296 ms is the
15.6 ms unguarded park tick to three figures.** `a5` was eliminated on the consumer, not on the
mechanism the design predicted would kill it.

### Occupancy and the wall clock DISAGREED, and the wall clock ranks

In the head-to-head's occupancy printout, **`a1f`'s worker route shows LOWER `top_lane` (15–22 of
64) than `a3`'s (17–33)** on the 200 µs shape — i.e. `a1f` spreads the wave better. **It loses
every timed comparison.** The owner's criterion is throughput, not core occupancy; the wall clock
ranks. Recorded because an occupancy-first reading of the same two builds returns the opposite
winner.

Related, and not decision-relevant: `a3`'s measured `max_in_flight` is 4/15 at W=4/W=16 where
`a1`/`a1f`/`a2`/`a5` reach 4/16. The floor is W/2 = 8, so `a3` clears it; the one-lane shortfall is
a characteristic of `a3`, not a failure.

### Five cells that cannot fire — labelled, never counted as agreement

Reference bands too wide for §4's rule to ever return a verdict: `dispatcher/100us×W` (41.06 %),
`dispatcher/1ms×4W` (35.81 %, also the known-bimodal cell), `dispatcher/1ms×W` (28.61 %),
`dispatcher/100us×64W` (10.10 %), `dispatcher/1us×64W` (6.76 %). A candidate would have to be
1.14–1.82× slower on them to be called a regression. Every tie read off these is a **tie by
construction**. None is a worker decision cell, so none touches the ranking — but one of them is
load-bearing for the `a2`/`a5` veto, as above.

---

## §B. Axis B — B0 IN THIS PASS; the "BY CONSTRUCTION" reading was based on a FALSE premise (see the 2026-09-07 correction below)

`b1` and `b3` carry a compile-time refusal. Verified in the tree, twice, on independent passes, and
receipted as an exit code rather than read off the source:

```rust
// crates/boyko_threadpool/src/lib.rs:110-119
#[cfg(all(any(feature = "ke16-b1", feature = "ke16-b3"),
          not(any(feature = "ke16-a1", feature = "ke16-a1-fifo"))))]
compile_error!("KE16 axis B: `ke16-b1` / `ke16-b3` require `ke16-a1` or `ke16-a1-fifo` \
  — the worker joiner needs a REGISTERED destination deque, which only the A1 arms give it; \
  A2/A3/A5 leave the joiner at B0")
```

`cargo check -p boyko-threadpool --features ke16-a3,ke16-b1` exits **101** with that message.
**Neither `b1` nor `b3` BUILDS over `a3`. There is no Step B tournament to run.**

### ⚠ CORRECTION 2026-09-07 — the refusal is right, its stated reason was FALSE

The `compile_error!` quoted above said the A1 arms are what *give* the worker joiner a registered
destination deque. **That is false as written, and it is what made axis B read as closed by
construction.** Verified in the tree three ways:

* `thread_pool.rs:718-737` builds `worker_count` deques and registers their `Stealer`s
  **unconditionally**; the only `cfg` there chooses `new_lifo()` vs `new_fifo()`.
* `worker.rs:33` hands each worker its `Worker<Task>` by move **under every arm**.
* What the A1 arms uniquely do is publish that deque's address into the thread's TLS —
  `WorkerDequeDeposit` at `worker.rs:60-61`. And `tls::worker_lane_for` **already has a working
  non-A1 branch** (`tls.rs:238-248`) returning `Some(WorkerLane { wid })`; it carries
  `#[cfg_attr(feature = "ke16-a3", allow(dead_code))]` (`tls.rs:221`) only because under `a3`
  nothing calls it.

So the axis-B closure is a **REACHABILITY gap, not a structural impossibility**. The refusal itself
stays — `b1`/`b3` call `lane.deque()`, which the non-A1 `WorkerLane` does not have — but its message
has been rewritten to say the missing thing is the deposit, not the deque.

**Defect B is live under `a3` by DIRECT measurement of the joiner's own call, not only by occupancy
inference.** `the_worker_joiner_does_not_run_a_residue_before_re_checking_its_scope`
(`tests/ke16_b_join_arms.rs:343`) under `--features ke16-a3`, five runs, `running 1 test` each:

| | µs |
|---|---|
| measured join, five runs | 192 142 / 192 205 / 192 111 / 188 129 / 192 133 |
| **median** | **192 133 (192.1 ms)** |
| B0 residue floor | 132 000 |
| join budget | 60 000 |
| `ke16-a1,ke16-b1` / `,ke16-b3` (recorded) | 4 / 5 |

`a3+b0` reads the same 192 ms `ke16-a1` does, at **3.2× the budget**, against arms that read
microseconds — a separation of roughly four orders of magnitude that no ambient load can flip.

⚠ **Number correction.** The worker-route occupancy for `a3` at W=16 is `top_lane = 33` on **all
three** reps with `lanes_used = 15/16/15` (§ appendix). A session note circulated `33/31/24`; that
number appears nowhere in this corpus — `13/24/21` is `a1`'s row, not `a3`'s.

**External corroboration of the framing.** A survey of Rayon, Go, Tokio, Java ForkJoinPool, TBB,
Cilk-5, .NET and `async-executor` finds **no production scheduler that batch-steals into a private
buffer only the stealer can drain**: they either steal one task (Rayon — current `registry.rs` has
zero `steal_batch` calls — FJP, TBB, Cilk, .NET) or batch into a destination that stays stealable by
everyone (Go's `runqsteal` into the P's own runq, whose `runqgrab` is documented "Can be executed by
any P"; Tokio; `async-executor`). Our own `tests/shutdown.rs:24-37` already records the consequence
of the private buffer — a task blocking on a peer that sits unrun in the same `scratch` deadlocks —
and states that **both B arms remove it**.

⚠ Two constraints any remedy inherits. `Injector` is a linked list of 63-slot blocks, so the
documented "steals about half" rule applies **only** when head and tail share a block; a real
multi-block wave takes the flat `(BLOCK_CAP − offset).min(limit)` branch, i.e. 33 every call — "it
only takes half, so it self-limits" is wrong here. And `tests/loom_pool.rs:208-236, 330` models the
joiner's re-poll **as `steal_batch_and_pop` specifically**, citing the `SeqCst` fence it carries, so
changing that call changes what those models cover.

The design pre-committed the response and it was followed: its B1(i) row names the trigger
*"A2/A3/A5 wins Step A beyond the band (then the worker joiner also has no registered deque)"* and
classifies it as *"a re-plan point, not a fallback taken silently"*; the index says it in imperative
form — ***"stop and report, do not improvise B1(i)."***

### The consequence, stated rather than hidden

`KE16-DESIGN.md` §1: *"every A-fix promotes B onto the production path"* — B never fires on the ECS
frame path today only because `Scope::drop` returns on its first `is_drained()`.

**Shipping `a3` therefore puts defect B live on the frame path: the joiner batch-steals into an
unregistered `scratch` and runs ~33 of 64 bodies inline. This pass supplies NO remedy, and `b1`/`b3`
cannot supply one over the `a3` substrate.**

It is live under `a3` **by measurement, not by inference**: `a3`'s own worker-route occupancy prints
`top_lane = 33` of 64 — defect B's documented ≤ 33 `scratch` batch, now appearing on the **worker**
route because `a3`'s joiner drains `injector_global` into `scratch` ≤ 33 at a time. The axis-W/C
reference reproduces it on both routes at W=16.

⚠ **That receipt is BISTABLE across processes** — 33/33/33, then 16, then 5/7/5, with the dev wall
moving 6.66 → 1.03 ms — while `max_in_flight` and `lanes_used` stay 15–16. The occupancy gate is
green in every case, so **anyone quoting one `top_lane` reading for `a3` is quoting whichever mode
that process landed in.**

The registered-scratch alternative is explicitly not to be reached for: it would require publishing
a stack-lived deque's `Stealer` into `inner.stealers` for the join's duration and withdrawing it
while a sibling may be mid-`steal_batch_and_pop` — a hazard-pointer or epoch protocol on the
registry.

**Axis B under `a3` is B0 by default, plus an open re-plan item for anyone who wants B fixed on this
substrate.** The good news, and it is already inside the numbers: every `a3` figure in §A.2 and
§A.5 is `a3+b0+w0+c0`, i.e. **taken with defect B live**, and it still clears both acceptance
clauses.

**Step B rule 4 consequence, fixed for every later step:** because B is B0, the physics reference
row is `bench_thread_install_Wminus1` (W−1 workers + a HELPING external joiner = W lanes). The
row-ratio receipt `Wminus1 / install` reads near 1 on every arm (a0 1.040, `a2` 1.040/1.043, `a3`
0.995/0.998, `a5` 1.027, `a1f` 0.987–1.008, `a1` 0.984) — the external arm IS helping, which is the
correct B0/B1 reading. The `≈ W/(W−1) = 1.067` check is a **B3-only** receipt and correctly holds
nowhere.

---

## §W. MEASURED 2026-09-08 at CS-4 — `wg` and `wgc` ELIMINATED on an OCCUPANCY receipt; `wc` a tie everywhere

**Three interleaved passes over `{w0, wg, wc, wgc}` at CS-4**, one pass = all four arms in table
order, driver `scripts/ke16_measure.sh`, witness `KE16_EXPECT` matched in all three harness logs for
all twelve runs. 38 cells carry a complete 4 × 3 set. `c1`/`c1f` are NOT measured and cannot be until
W\* is fixed: step C is defined as `A*+B*+W*+c1`.

### The verdict, and it does not rest on the wall clock

| arm | cells improved | cells regressed | `max_in_flight` at N = 65536 | ECS speedup there |
|---|---|---|---|---|
| `wg` (W-b) | **0** | **13 of 38** | **5–6** of 16 | **2.00×** |
| `wgc` | **0** | **14 of 38** | 5–8 of 16 | 2.00–4.00× |
| `wc` (W-d′) | 0 | **0** | **16** of 16 | 10.6–15.8× |
| `w0` (reference) | — | — | **16** of 16 | 13.9–15.7× |

**`wg` and `wgc` fail step W rule 1 on BOTH clauses** — each regresses a 1 µs cell beyond 2 × band
(`worker/body_1us_tasks_64w`, × 1.42 and × 1.26; `wgc` also `dispatcher/body_1us_tasks_64w`, × 1.51)
and neither improves a single cell or consumer. **Eliminated.**

**The deciding evidence is the occupancy receipt, not a time.** W-b does not make the pool slower by
adding work; **it leaves ten of sixteen lanes parked.** The ECS protocol pass reports
`max_in_flight` directly, and under `wg` it reads 5–6 with the speedup pinned at **2.00×** in every
pass — which is the gate's own arithmetic rather than a measurement artefact: on a wave pushed to
one destination only the first two pushes see `pre_len` of 0 and 1, so only two workers are woken,
and the thief-residue cascade restores width one unpark-round at a time, too late for the wave. At
N = 4096 the same reading is `max_in_flight` 3 against the reference's 4, speedup 2.00 against 4.00.

⚠ **This is a receipt the wall clock alone could not have produced**, and it is why
`KE16-DESIGN-MEASUREMENT.md` §3 lists occupancy as "the diagnostic behind a wall-clock change".

### The design named this outcome as its own risk, on exactly these cells

`KE16-DESIGN-W.md` §2.4 predicted the gain at **1 µs × {4W, 64W} on the worker route**, and wrote the
risk as: *"the 100 µs–1 ms × W cells, where the chain's 4 hops of Windows wake latency plus the
one-body stall may exceed the W serial unparks the spawner paid before."*

| cell | design said | measured `wg` / `w0` |
|---|---|---|
| `worker/body_1us_tasks_4w` | gain | 1.10 — tie |
| `worker/body_1us_tasks_64w` | gain | **1.42 — REGRESSES** |
| `worker/body_100us_tasks_w` | risk | **3.66** |
| `worker/body_1ms_tasks_w` | risk | **3.31** |
| `dispatcher/body_100us_tasks_w` | risk | **4.69** |
| `dispatcher/body_1ms_tasks_w` | risk | **5.51** |

**Both halves of the prediction failed in the same direction**: the two cells named as the gain went
tie and regression, and the risk fired at 3.3 × to 5.5 × rather than the "may exceed" it was written
as. §2.4 also said the gain would show on the physics consumer, "whose 6 colors leave workers parked
between steps" — the physics rows separate no arm at all (below).

### ⚠ The data carries a MONOTONE DRIFT, and the verdict is stated only where the drift cannot reach

The reference's own three passes are not stationary. They get FASTER, monotonically:

| reference row | r1 | r2 | r3 | spread |
|---|---:|---:|---:|---:|
| `bench_thread_install` | 36.24 ms | 13.36 ms | 8.12 ms | **346 %** |
| `bench_thread_install_Wminus1` | 19.79 ms | 12.74 ms | 7.84 ms | 152 % |
| `in_scheduled_system` **PRIMARY** | 15.65 ms | 14.88 ms | 8.45 ms | **85 %** |
| `single_threaded_O5` (session meter) | 33.46 ms | 32.92 ms | 26.37 ms | 27 % |
| `par_in_system/65536` | 174.4 ms | 109.2 ms | 85.7 ms | 104 % |
| `seq/65536` (ECS single-threaded) | 1314 ms | 1365 ms | 1311 ms | **4.1 %** |

This is the §Band Step-0 finding again — *"the band as taken is measuring a first-run WARM-UP, not
jitter"* — in a session rather than in a run. The load receipts agree: pass 1 opened at 14.8 % CPU
and pass 3's first receipt reads 1.4 / 0 / 0.1 %.

### ⚠⚠ THE DRIFT'S CAUSE IS THE DRIVER ITSELF, and it is named here rather than guessed at

The machine was verified idle before the series (CPU 9.5 → 5.9 → **0.7 %**) and no game is present in
any of the twenty-four load receipts. **The contamination was self-inflicted, and the build logs say
so:**

| pass | crates compiled immediately before each timed region |
|---|---|
| 0 (rehearsal) | the initial full build — a 119-line log |
| **1** | `wg`, `wc`, `wgc`: **7 crates each**; `w0` none (already built by pass 0) |
| 2, 3 | **zero** — every arm's log is three `Finished` lines |

Each arm is a different `--features` line, so cargo rebuilds the pool crate and its dependents on
first use of that arm. The driver therefore ALTERNATES a full-core compile with a timed region, and
the first pass measures in the wake of one. Moving the build ahead of the load receipt (`e7910d5f`)
made the receipt HONEST — it is what reports the 15–44 % — but it did not make the machine quiet.
**The monotone speed-up across passes is the compiles stopping, not the box freeing itself.**

⇒ **A future pass should PRE-BUILD EVERY ARM BEFORE PASS 1**, then measure. `scripts/ke16_measure.sh`
does not do this yet; the fix is one `--no-run` loop over the variant table ahead of the pass loop.

### The within-pass control: `seq` says the arms ARE comparable inside a pass

The drift is BETWEEN passes, which is exactly what interleaving is built to absorb — and the ECS
sequential row is the receipt that it does. Within each pass, across all four arms:

| control | pass 1 | pass 2 | pass 3 |
|---|---:|---:|---:|
| `seq/65536` spread across the four arms | 7.2 % | 4.1 % | 5.0 % |

At pass 3 it is 1.00, 1.00, 1.05, 1.00 relative to the reference. **A single-threaded row that is
identical across arms inside a pass says the machine state is identical across those arms**, so a
difference in the parallel rows of that pass is the arm and not the box.

### `wg` is eliminated by PASS 3 ALONE — the pass that compiled nothing

Discard passes 1 and 2 entirely and the verdict does not move:

| cell (pass 3 only) | `w0` | `wg` | ratio | `wc` | ratio |
|---|---:|---:|---:|---:|---:|
| `worker/body_1ms_tasks_w` | 2.554 ms | 7.878 ms | **3.08** | 1.330 ms | 0.52 |
| `worker/body_10us_tasks_w` | 31 437 ns | 55 269 ns | **1.76** | 31 333 ns | 1.00 |
| `dispatcher/body_1ms_tasks_w` | 1.712 ms | 8.003 ms | **4.68** | 1.199 ms | 0.70 |
| `dispatcher/body_100us_tasks_w` | 245 976 ns | 804 307 ns | **3.27** | 137 975 ns | 0.56 |
| `ecs par_in_system/65536` | 85.69 ms | 635.37 ms | **7.41** | 89.38 ms | 1.04 |
| `ecs par_from_dispatcher/65536` | 85.16 ms | 655.56 ms | **7.70** | 126.49 ms | 1.49 |
| `ecs seq/65536` **CONTROL** | 1310.98 ms | 1311.11 ms | **1.00** | 1375.83 ms | 1.05 |
| `ecs seq/4096` **CONTROL** | 81.96 ms | 81.94 ms | **1.00** | 81.95 ms | 1.00 |

**17 of 38 cells put `wg` above 1.30 × the reference in that one clean pass; `wc` reaches 1.30 × on
three, none above 1.63 ×.** The two controls sit at 1.00. The three-pass verdict above is therefore
carried by the clean pass on its own, and passes 1 and 2 add agreement rather than evidence.

**Consequences, stated rather than smoothed over:**

1. **NO PHYSICS VERDICT IS FILED.** With an 85 % band on the PRIMARY row every arm is a tie by
   construction, and a tie against a band that wide says nothing. The physics rows below are
   recorded as data, not as a ranking.
2. **The elimination of `wg`/`wgc` survives the drift, and here is why it is not an artefact of it.**
   The effect is 1.8 ×–5.5 ×, larger than the drift; it is present in EVERY pass, not in one; it
   appears on cells whose reference spread is tight (`worker/body_10us_tasks_w`: reference
   31 584 / 32 090 / 31 437 ns, spread 2.1 %, against `wg` 59 354 / 73 507 / 55 269); and it is
   corroborated by an occupancy integer that drift cannot move — 5 lanes is not a slow 16.
3. **`wc` is a tie on the TIGHT cells too**, which is what makes its tie meaningful rather than a
   consequence of wide bands: 33 796 / 30 387 / 31 333 ns on that same cell, against the reference's
   31 584 / 32 090 / 31 437.
4. **A fourth pass would not settle the physics rows** — the drift is monotone, so more passes taken
   the same way extend the trend rather than average it out. A physics verdict needs a session that
   opens already settled, or a discard rule taken per harness as §W's CS-2 pass did.

### ✅ `wc` IS KEPT — W\* = `wc`. Rule 2's two gates ran at CS-4 and are green

Step W rule 2 keeps `wc` when (a) it regresses no 1 µs cell and it is a tie everywhere — both hold
above — **"PROVIDED both its gates are green: loom M1c (a real-park M1c) and the route-(b)
many-seeds gate."** Both ran on 2026-09-08 at CS-4:

| gate | recipe | verdict |
|---|---|---|
| **loom M1c**, real park | `cargo --config target.x86_64-pc-windows-gnu.rustflags=["-C","target-cpu=x86-64-v3","--cfg","loom"] test --release -p boyko-threadpool --test loom_pool --features ke16-a3,ke16-w-count` | **6 passed, 0 failed**, `loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner ... ok`. Repeated with `LOOM_MAX_PREEMPTIONS=3`: same |
| **route-(b) liveness**, 32 seeds | `MIRIFLAGS="… -Zmiri-many-seeds=0..32" cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool --test miri_scope nested_scope_from_worker_is_stolen_by_sibling --features ke16-a3,ke16-w-count` | **32 × `1 passed`**, zero UB, zero liveness timeouts |

**It is an M1c and not an "M1c-count"** (the distinction §5 item 16 makes, and the one rule 2
depends on): the model's joiner is `while !shared.is_drained() { thread::park(); }` — loom's real
park — not M1's yield re-poll, verified in the source before the run. The `loom_pool` run also
carries its own anti-vacuity receipt: `loom_m2_calibration_no_producer_fence_is_lost - should panic
... ok`, i.e. the un-fenced copy did reproduce the lost wake.

⚠ **The ISA flag is spelled in full in that `--config` array on purpose.** `.cargo/config.toml` sets
`rustflags = ["-C", "target-cpu=x86-64-v3"]` for this target, and `--config` REPLACES the key rather
than appending to it, so a bare `--config …rustflags=["--cfg","loom"]` would have silently dropped
the x86-64-v3 baseline the whole campaign is measured under. The same asymmetry as `RUSTFLAGS`.

### ⚠⚠ The route-(b) gate's FIRST run was a vacuous pass, and the gate is what caught it

Run with `--features ke16-w-count` alone — the arm's own feature, which is what the rule names — it
printed `running 1 test` **thirty-two times** and exited **0**. It ran nothing. The result line reads
`0 passed; 0 failed; 1 ignored; 0 measured; 6 filtered out`, and thirty-two `running 1 test` lines
look like far better evidence of thirty-two runs than one does.

The test refused on its own terms, and its `ignore` reason is the whole diagnosis: *"Under
ke16-w-count WITH NO A ARM the shape needs a SIBLING to take one of the two handshaked nested bodies
and has none, so it would hang rather than measure: **NO W-d-prime liveness row may be filed from
that build**."* It also forecloses the obvious workaround — forcing it with `--ignored` hits
`refuse_to_certify_without_a_reachability_arm()` as the body's first statement.

⇒ **§8's `<F>` is the CONFIGURATION's feature list, not the arm's**, and for this step that is
`ke16-a3,ke16-w-count`. The corrected run is the one in the table above. Recorded because the failure
mode is this corpus's own: `running N` is not a receipt, and a count of `running N` lines is not a
better one.

### ⚠ Raw data lost, and it was this pass that lost it

`--save-baseline` is keyed on the variant string alone, which does not name the code state, and
criterion overwrites in place. Re-taking `a3+b0+w0+c0` at CS-4 therefore **destroyed the CS-2
reference's raw samples for r1, r2 and r3**; `-r4` and `-r5` survive only because this pass stopped
at three. Every median, band and per-run value of that reference is preserved in prose below, so the
findings are intact — the per-sample data behind three of five runs is not, and criterion baselines
are not rebuildable from anything in the tree. Fixed in `e7910d5f`: the baseline name now carries the
short HEAD hash, so two code states can no longer address one directory.


### Appendix: every cell, median of three passes at CS-4

Medians are the median of the three per-pass medians. `band` is the reference's widest
pairwise gap over its three passes, floored at 4 % (§4). A verdict is `REG` when
`med_V > med_R x (1 + 2 x band)` with the band taken as the larger of the arm's and the
reference's, `IMP` at the mirror inequality, `tie` otherwise.

| cell | `w0` | band | `wg` | vs | `wc` | vs | `wgc` | vs |
|---|---:|---:|---:|:--|---:|:--|---:|:--|
| `worker/body_100us_tasks_4w` | 1.69 ms | 10.1 % | 2.13 ms | tie | 1.73 ms | tie | 2.19 ms | tie |
| `worker/body_100us_tasks_64w` | 7.23 ms | 12.0 % | 7.56 ms | tie | 7.44 ms | tie | 7.67 ms | tie |
| `worker/body_100us_tasks_w` | 189.8 us | 32.8 % | 695.0 us | **REG** | 183.3 us | tie | 711.6 us | **REG** |
| `worker/body_10us_tasks_4w` | 130.3 us | 10.3 % | 215.1 us | **REG** | 126.0 us | tie | 219.9 us | **REG** |
| `worker/body_10us_tasks_64w` | 813.3 us | 12.5 % | 807.0 us | tie | 762.7 us | tie | 777.3 us | tie |
| `worker/body_10us_tasks_w` | 31.6 us | 4.0 % | 59.4 us | **REG** | 31.3 us | tie | 56.4 us | **REG** |
| `worker/body_1ms_tasks_4w` | 17.23 ms | 12.5 % | 19.35 ms | tie | 16.27 ms | tie | 18.68 ms | tie |
| `worker/body_1ms_tasks_64w` | 67.98 ms | 8.0 % | 72.53 ms | tie | 68.36 ms | tie | 70.45 ms | tie |
| `worker/body_1ms_tasks_w` | 1.58 ms | 78.5 % | 5.22 ms | **REG** | 1.37 ms | tie | 7.08 ms | **REG** |
| `worker/body_1us_tasks_4w` | 15.9 us | 8.2 % | 17.5 us | tie | 15.9 us | tie | 15.0 us | tie |
| `worker/body_1us_tasks_64w` | 109.4 us | 12.6 % | 155.3 us | **REG** | 98.7 us | tie | 137.8 us | **REG** |
| `worker/body_1us_tasks_w` | 9.7 us | 12.3 % | 9.3 us | tie | 9.5 us | tie | 9.4 us | tie |
| `dispatcher/body_100us_tasks_4w` | 1.69 ms | 12.9 % | 2.48 ms | **REG** | 1.60 ms | tie | 2.48 ms | **REG** |
| `dispatcher/body_100us_tasks_64w` | 7.63 ms | 13.7 % | 7.51 ms | tie | 7.54 ms | tie | 7.82 ms | tie |
| `dispatcher/body_100us_tasks_w` | 173.7 us | 61.1 % | 815.0 us | **REG** | 154.2 us | tie | 808.1 us | **REG** |
| `dispatcher/body_10us_tasks_4w` | 139.1 us | 4.0 % | 273.4 us | **REG** | 144.1 us | tie | 270.9 us | **REG** |
| `dispatcher/body_10us_tasks_64w` | 791.3 us | 15.2 % | 835.9 us | tie | 768.6 us | tie | 827.8 us | tie |
| `dispatcher/body_10us_tasks_w` | 21.9 us | 4.0 % | 65.5 us | **REG** | 22.1 us | tie | 69.2 us | **REG** |
| `dispatcher/body_1ms_tasks_4w` | 14.99 ms | 10.7 % | 20.67 ms | tie | 14.64 ms | tie | 23.78 ms | tie |
| `dispatcher/body_1ms_tasks_64w` | 67.94 ms | 7.3 % | 70.65 ms | tie | 69.33 ms | tie | 70.67 ms | tie |
| `dispatcher/body_1ms_tasks_w` | 1.45 ms | 32.8 % | 7.96 ms | **REG** | 1.36 ms | tie | 7.69 ms | **REG** |
| `dispatcher/body_1us_tasks_4w` | 11.8 us | 20.5 % | 15.8 us | tie | 13.2 us | tie | 14.8 us | tie |
| `dispatcher/body_1us_tasks_64w` | 106.1 us | 4.0 % | 171.5 us | tie | 108.3 us | tie | 160.0 us | **REG** |
| `dispatcher/body_1us_tasks_w` | 4.8 us | 8.3 % | 5.4 us | tie | 4.8 us | tie | 5.3 us | tie |
| `park_timeout_1ms` | 15.57 ms | 4.0 % | 15.57 ms | tie | 15.56 ms | tie | 15.59 ms | tie |
| `park_timeout_2ms` | 15.55 ms | 4.0 % | 15.57 ms | tie | 15.56 ms | tie | 15.55 ms | tie |
| `park_timeout_50us` | 15.57 ms | 4.0 % | 15.58 ms | tie | 15.58 ms | tie | 15.58 ms | tie |
| `physics bench_thread_install` | 13.36 ms | 346.3 % | 17.67 ms | tie | 12.74 ms | tie | 13.33 ms | tie |
| `physics bench_thread_install_wminus1` | 12.74 ms | 152.3 % | 9.94 ms | tie | 12.17 ms | tie | 13.12 ms | tie |
| `physics empty_schedule_control` | 1.7 us | 9.1 % | 1.7 us | tie | 1.7 us | tie | 1.8 us | tie |
| `physics in_scheduled_system` | 14.88 ms | 85.2 % | 10.42 ms | tie | 12.43 ms | tie | 12.71 ms | tie |
| `physics single_threaded_o5` | 32.92 ms | 26.9 % | 30.98 ms | tie | 30.83 ms | tie | 32.85 ms | tie |
| `ecs par_from_dispatcher/4096` | 22.60 ms | 65.2 % | 43.00 ms | tie | 23.47 ms | tie | 40.99 ms | tie |
| `ecs par_from_dispatcher/65536` | 100.00 ms | 68.7 % | 687.46 ms | **REG** | 119.07 ms | tie | 675.75 ms | **REG** |
| `ecs par_in_system/4096` | 21.63 ms | 7.3 % | 33.79 ms | **REG** | 22.20 ms | tie | 35.49 ms | **REG** |
| `ecs par_in_system/65536` | 109.20 ms | 103.6 % | 533.09 ms | **REG** | 124.24 ms | tie | 493.82 ms | **REG** |
| `ecs seq/4096` | 82.23 ms | 4.3 % | 84.84 ms | tie | 81.95 ms | tie | 82.00 ms | tie |
| `ecs seq/65536` | 1,314.06 ms | 4.1 % | 1,358.65 ms | tie | 1,375.83 ms | tie | 1,311.34 ms | tie |

---

## §C, §F. MEASURED 2026-09-08 — CLOSED ON `c0`; both arms dropped

### ✅ MEASURED 2026-09-08 — BOTH ARMS DROPPED. Axis C closes on `c0`

Three interleaved passes over `{c0, c1, c1f}` at CS-4 (`7ccab360`), taken after
`scripts/ke16_measure.sh --prebuild`, so **no timed region followed a compile** — the driver writes
`DIRTY-PASS.txt` when one does and wrote none. Settling receipt before pass 1: 8.3 / 7.8 / 5.2 / 5.6
/ 4.1 % CPU.

| arm | improved | regressed | `max_in_flight` at N = 65536 | ECS speedup there |
|---|---|---|---|---|
| `c1` (App-4 batch spawn) | **0** | 6 of 38 | **2** of 16 | **2.00×** |
| `c1f` (+ W-f fan-out) | **0** | 5 of 38 | 9–10 of 16 | **7.95–8.00×** |
| `c0` (reference) | — | — | **16** of 16 | 15.05–15.28× |

**Both fail step W/C rule 1 on clause (b): neither improves a single cell or consumer anywhere.**
Clause (a) holds for both — no 1 µs cell regresses — so the drop is purely the absence of a gain.
**W\* + C\* = `wc + c0`.**

### The occupancy integers again, and `c1`'s is the code's own sentence

`src/scope.rs::Scope::wake_for_wave` says a build with `ke16-c-batch` alone takes a wave *"down to
the spawner plus ONE, with the rest asleep for the whole wave"*. Measured: **`max_in_flight` = 2**,
in all three passes, with the speedup pinned at 2.00×. Not "about two" — two.

`c1f` is the arm built to repair exactly that, and it does repair most of it: 2 → 9–10 lanes,
speedup 7.95–8.00×. **It still loses 1.86× to the reference**, because a single-snapshot fan-out at
the wave's first push reaches only the siblings already parked at that instant, and that is about
half of them. So App-4's `pending` saving — one `fetch_add(n)` per wave instead of `n` — is real,
invisible, and bought with a lane-count collapse no fan-out available here fully undoes.

### ⚠ The pair measurement is what separates two very different findings, and rule 3 forbade it

Applying §7 rule 3 literally — measure `c1`, see it regress, do not measure `c1f` — would have filed
**"App-4 rejected"**. What the pair actually shows is **"App-4's accounting is invisible under every
wake width this tree can build"**: `c1f` recovers 2 → 10 lanes and STILL improves nothing. The second
statement is the true one, and it is the one that tells a future reader not to retry App-4 behind a
better fan-out without first finding a wake mechanism that reaches 16.

### The design's prediction about WHERE, and only about where

`KE16-DESIGN-W.md` §2.4 wrote that criterion's back-to-back iterations keep workers spinning, *"so
the pool grid understates this; the physics consumer, whose 6 colors leave workers parked between
steps, is where it shows"*. Exactly so: **all 24 pool-grid cells and all three `park_timeout` rows
are ties**, and every one of the eleven regressions across the two arms is on physics or ECS. The
design was right about the site and wrong about the sign, on both W and C.

### ✅ A PHYSICS RANKING IS FILED HERE, and the reason it could not be filed for §W was the driver

| physics row | `c0` (r1 / r2 / r3) | spread | `c1` | `c1f` |
|---|---|---:|---:|---:|
| `single_threaded_O5` (session meter) | 29.69 / 29.61 / 29.71 ms | **0.3 %** | 29.60 ms | 29.64 ms |
| `in_scheduled_system` **PRIMARY** | 10.87 / 11.15 / 11.34 ms | **4.3 %** | 16.99 ms (**1.52×**) | 13.77 ms (**1.24×**) |
| `bench_thread_install_Wminus1` **REF** | 10.80 / 11.00 / 10.91 ms | 1.9 % | 17.04 ms | 13.06 ms |

Compare §W, where the same reference row spread **85 %** and the session meter **27 %**. The
difference is not the machine and not the arms: it is `--prebuild`. §W's passes alternated a
seven-crate compile with a timed region; these compiled nothing. **A physics ranking was not
impossible — it was being destroyed by the driver**, and the acceptance line's clause (1) reads
cleanly here: `in_scheduled_system` 11.15 ms < `single_threaded_O5` 29.61 ms.


### Appendix: every cell, median of three passes, step C at CS-4 / `7ccab360`

Same rule as §W's appendix. The reference is `c0` = `a3+b0+wc+c0`, re-taken in these same
passes rather than carried over from the W step.

| cell | `c0` | band | `c1` | vs | `c1f` | vs |
|---|---:|---:|---:|:--|---:|:--|
| `worker/body_100us_tasks_4w` | 1.66 ms | 4.8 % | 1.69 ms | tie | 1.66 ms | tie |
| `worker/body_100us_tasks_64w` | 6.68 ms | 4.0 % | 6.65 ms | tie | 6.69 ms | tie |
| `worker/body_100us_tasks_w` | 156.4 us | 4.0 % | 158.6 us | tie | 158.4 us | tie |
| `worker/body_10us_tasks_4w` | 110.8 us | 4.0 % | 108.9 us | tie | 107.4 us | tie |
| `worker/body_10us_tasks_64w` | 694.6 us | 4.0 % | 696.7 us | tie | 696.9 us | tie |
| `worker/body_10us_tasks_w` | 27.0 us | 4.0 % | 27.4 us | tie | 27.6 us | tie |
| `worker/body_1ms_tasks_4w` | 13.32 ms | 9.2 % | 14.39 ms | tie | 13.47 ms | tie |
| `worker/body_1ms_tasks_64w` | 65.61 ms | 4.0 % | 65.62 ms | tie | 65.39 ms | tie |
| `worker/body_1ms_tasks_w` | 1.31 ms | 4.0 % | 1.30 ms | tie | 1.30 ms | tie |
| `worker/body_1us_tasks_4w` | 12.5 us | 4.0 % | 12.6 us | tie | 12.6 us | tie |
| `worker/body_1us_tasks_64w` | 81.6 us | 4.0 % | 81.6 us | tie | 81.4 us | tie |
| `worker/body_1us_tasks_w` | 7.5 us | 4.0 % | 7.6 us | tie | 7.7 us | tie |
| `dispatcher/body_100us_tasks_4w` | 1.62 ms | 6.3 % | 1.63 ms | tie | 1.62 ms | tie |
| `dispatcher/body_100us_tasks_64w` | 6.70 ms | 4.0 % | 6.70 ms | tie | 6.68 ms | tie |
| `dispatcher/body_100us_tasks_w` | 128.9 us | 4.0 % | 126.3 us | tie | 127.4 us | tie |
| `dispatcher/body_10us_tasks_4w` | 129.2 us | 6.1 % | 128.9 us | tie | 124.1 us | tie |
| `dispatcher/body_10us_tasks_64w` | 690.8 us | 4.0 % | 693.9 us | tie | 691.7 us | tie |
| `dispatcher/body_10us_tasks_w` | 15.9 us | 4.0 % | 16.0 us | tie | 16.0 us | tie |
| `dispatcher/body_1ms_tasks_4w` | 13.89 ms | 4.7 % | 13.31 ms | tie | 13.17 ms | tie |
| `dispatcher/body_1ms_tasks_64w` | 66.06 ms | 4.0 % | 65.72 ms | tie | 66.20 ms | tie |
| `dispatcher/body_1ms_tasks_w` | 1.19 ms | 4.0 % | 1.17 ms | tie | 1.19 ms | tie |
| `dispatcher/body_1us_tasks_4w` | 9.9 us | 4.3 % | 9.6 us | tie | 9.7 us | tie |
| `dispatcher/body_1us_tasks_64w` | 86.4 us | 5.8 % | 84.7 us | tie | 83.5 us | tie |
| `dispatcher/body_1us_tasks_w` | 4.1 us | 5.0 % | 4.1 us | tie | 4.1 us | tie |
| `park_timeout_1ms` | 6.96 ms | 12.1 % | 6.94 ms | tie | 13.26 ms | tie |
| `park_timeout_2ms` | 6.95 ms | 10.3 % | 7.04 ms | tie | 13.47 ms | tie |
| `park_timeout_50us` | 6.93 ms | 14.9 % | 6.95 ms | tie | 12.52 ms | tie |
| `physics bench_thread_install` | 12.20 ms | 17.9 % | 17.32 ms | tie | 11.97 ms | tie |
| `physics bench_thread_install_wminus1` | 10.91 ms | 4.0 % | 17.04 ms | **REG** | 13.06 ms | tie |
| `physics empty_schedule_control` | 1.5 us | 4.0 % | 1.5 us | tie | 1.5 us | tie |
| `physics in_scheduled_system` | 11.15 ms | 4.3 % | 16.99 ms | **REG** | 13.77 ms | **REG** |
| `physics single_threaded_o5` | 29.69 ms | 4.0 % | 29.70 ms | tie | 29.64 ms | tie |
| `ecs par_from_dispatcher/4096` | 22.08 ms | 8.1 % | 40.57 ms | **REG** | 38.99 ms | **REG** |
| `ecs par_from_dispatcher/65536` | 88.33 ms | 4.0 % | 655.61 ms | **REG** | 164.95 ms | **REG** |
| `ecs par_in_system/4096` | 23.38 ms | 5.0 % | 42.04 ms | **REG** | 29.94 ms | **REG** |
| `ecs par_in_system/65536` | 88.86 ms | 4.0 % | 655.60 ms | **REG** | 165.38 ms | **REG** |
| `ecs seq/4096` | 81.93 ms | 4.0 % | 81.94 ms | tie | 81.93 ms | tie |
| `ecs seq/65536` | 1,310.86 ms | 4.0 % | 1,310.81 ms | tie | 1,310.80 ms | tie |


### ⚠⚠ They were measured as a PAIR, which DEPARTS from rule 3 — and the departure is forced by §W's own result

§7 Steps W/C rule 3 says *"`c1` (batch spawn) not kept ⇒ `c1f` is not measured"*. Its premise is
that `c1` is App-4's mechanism and `c1f` a width refinement on top of it.
`src/scope.rs::Scope::wake_for_wave` states the opposite about `ke16-c-batch` ALONE: the wave's ONE
wake decision activates the spawner plus ONE sibling, and what turns that into a wave's worth of
lanes is `worker::wake_after_residue` — which is `ke16-w-gate`'s and a no-op without it. So `c1`
alone is not App-4's accounting; it is **App-4's accounting minus up to W/2 lanes**, which is exactly
the mechanism §W measured at 2–6× against `wg`.

The design escalated the resulting fork as a DESIGN call, offering two ways to make a `c1` row
interpretable: over `ke16-w-gate`'s cascade, or over `ke16-w-fanout`. **§W deleted the first** — `wg`
is eliminated, so a `c1` row taken over it would describe a configuration that will never ship. Only
`c1f` remains.

Applying rule 3 literally would therefore file a rejection of App-4 that is really a rejection of the
wake collapse, and forbid measuring the one arm that could tell them apart. **Both were taken; the C
verdict is read off `c1f`, and `c1` is recorded as the isolated cost of the wake collapse rather than
as a candidate.** The driver carries the same statement at its variant table. The measurement
vindicated the departure: `c1f` recovered 2 → 10 lanes and still improved nothing, which is a
different and more useful finding than "App-4 rejected".

⚠ The `c1` caveat below still travels with any future C row, and CS-4 does not change it.

### The CS-2 reference: `a3+b0+w0+c0` on the POST-STAGE-1 code, 49 rows, preserved — NOT comparable to §W above

⚠ **These numbers describe CS-2 and MUST NOT be compared against §A's, which are CS-1.** See §CS.

Five runs were taken rather than three, because contamination on this box turned out to be
**per-bench-invocation, not per-run**: the burst that ruined r3's pool grid had subsided by the time
r3's physics ran, and the burst that ruined r2's physics arrived after r2's pool grid finished. A
per-run keep/discard decision would have been wrong in both directions. **Kept sets are per-harness:
pool grid {r2, r4, r5}, ECS {r2, r3, r4, r5}, physics {r3, r4, r5}.** With three or four kept runs
the band generalises conservatively to `band(c) = max(0.04, (max_med − min_med)/min_med)` over the
kept set — the widest pairwise gap, which can only make a later candidate harder to eliminate.

**Physics (the primary row and its four companions):**

| row | median | band | per-run |
|---|---:|---:|---|
| `in_scheduled_system/29751` **PRIMARY** | **10.019 ms** | 6.82 % | r3 10.548 / r4 9.875 / r5 10.019 |
| `bench_thread_install_Wminus1/29751` **REF** | 9.796 ms | 4.00 % | 9.796 / 9.833 / 9.785 — the tightest of the five |
| `bench_thread_install/29751` (W+1 lanes, raw only) | 9.851 ms | 5.93 % | 10.083 / 9.851 / 9.519 |
| `empty_schedule_control/29751` | 1 532 ns | 4.00 % | 1533 / 1531 / 1532 |
| `single_threaded_O5/29751` (session meter) | 27.422 ms | 4.00 % kept (actual 1.43 %) | r1 27.612 / r2 28.783 / r3 27.571 / r4 27.422 / r5 27.183 — **all five given; spread 5.88 % and the whole excess sits in r2, which is the datum that identified r2 as contaminated** |

Acceptance line, derived: clause (1) TRUE in all three kept runs. Clause (2) PRIMARY, per-run
pairing: r3 1.0766 (fails by 0.8 pp against 1.0682), r4 1.0041 (holds), r5 1.0237 (holds); on the
kept-run medians `net/REF = 10.0173/9.7962 = 1.0226 ≤ 1.0682` (holds). Raw beside it, against
`bench_thread_install`: 1.0459 / 1.0023 / 1.0524.

**The six 1 µs veto cells** (rule 1(a) surface for every later step): WORKER 1 µs/W = 9 894 ns
(band 6.18 %), 1 µs/4W = 19 864 ns (7.77 %), 1 µs/64W = 216 716 ns (4.00 %); DISPATCHER 1 µs/W =
5 109 ns (4.00 %), 1 µs/4W = 15 320 ns (7.45 %), 1 µs/64W = 203 087 ns (14.10 %). Every one has max
MAD/median ≤ 0.067 — internally tight; **where their band is wide it is BETWEEN-run drift, not
within-run jitter.** `dispatcher/1us_64W` is the clearest case: r4's median is 230 072 ns with a MAD
of 864 ns (0.4 %), sitting 14 % above r2 and r5, which agree with each other to 0.7 %. That is the
session-drift signature of §Method, appearing again in a different session.

**Worker route, kept medians (ns):** 1 µs W/4W/64W = 9 894 / 19 864 / 216 716 · 10 µs = 30 621 /
117 741 / 744 546 · 100 µs = 172 302 / 1 620 215 / 6 769 632 · 1 ms = 1 329 860 / 15 354 145
(band 37.51 %, **the widest on the grid — treat with care**) / 65 923 926.

**Dispatcher route, kept medians (ns):** 1 µs = 5 109 / 15 320 / 203 087 · 10 µs = 22 349
(band 27.64 %) / 137 655 / 745 461 · 100 µs = 150 053 / 1 863 202 / 6 898 710 · 1 ms = 1 288 581 /
16 283 238 (bimodal, the documented joiner-inline-share cell) / 66 496 689.

**ECS anti-vacuity receipt (rule 5):** protocol wall 1.00× at N=4096 in all four kept runs;
1.01 / 0.95 / 0.99 / 1.00× at N=65536. Criterion rows 0.918–0.991× at 4096 and 0.997–1.013× at
65536. **The two sources disagree by up to 0.082× at N=4096 and agree to 0.06× at N=65536**; both
are reported and neither is reconciled against the other. `max_in_flight` is 16 on both routes at
N=65536 and 4 on both at N=4096, so the worker route is not serialising under `a3` at either
population.

### ⚠ The `c1` caveat, which must travel with any future C row

The reference arm is `w0`: **there is no thief-residue cascade**, because the cascade is
`ke16-w-gate`'s and is a no-op without it. Consequently a `c1`-vs-`c0` row taken over `w0` is **NOT
the isolation of App-4's accounting the design asks for** — App-4 takes a wave from the
`min(n, popcount(idle))` lanes the per-task baseline wakes down to TWO, and only a cascade re-fans
it out. Any `c1` row compared against these numbers must carry that qualification, whatever it
shows.

Per §7 Steps W/C rule 3: if `c1` (batch spawn) is not kept, **`c1f` is not measured** — it needs the
batch push.

---

## §App. NOT RUN — OWED

**No number in this file comes from the code that ships.** §1 of the design requires a Step-App
retake: *"the final configuration with every feature removed (unconditional code): the full grid +
consumers ONCE MORE — the number that goes on record must come from the code that ships, not from a
feature build"*, and *"the unconditional code's numbers must match the winning feature build's
within the band on every cell and consumer; a difference beyond the band is a defect in the removal
step, not a new datum."*

That has not been run. Two independent reasons the retake is not optional:

1. Every absolute here is a **bench-profile** number (`codegen-units = 1`, `lto = false`) and the
   shipped release profile is `lto = "fat"` — a different codegen configuration by the manifest's
   own statement.
2. The winning configuration is still a **feature build**; the features have not been removed, the
   freeze has not been taken, and the removal commit's `grep -rn 'feature = "ke16' crates` has not
   been made to return nothing.

**The freeze that must precede removal has also not been done** (owner ruling, 2026-09-02: *"Do not
delete the unsuitable one, leave them as a spare — so the code is recorded but not present in the
project"*): no annotated tag `ke16/tournament`, no `docs/threadpool/KE16-REJECTED.md`. The order is
not negotiable — freeze, then remove, because after the removal there is nothing left to point a
tag at. The return conditions the register needs are already measured and are recorded in
§Rejected below.

---

## §Rejected. Return conditions for the frozen candidates

Written here so `KE16-REJECTED.md` can be produced from measured facts rather than wishes. A
condition is a fact about the world that could change.

| Candidate | Flag | Eliminated by | Return condition |
|---|---|---|---|
| `a1f` | `ke16-a1-fifo` | rule 2, head-to-head: 13.640 vs 11.709 ms, disjoint ranges | **Its loss is carried by the worker route at 10 µs × 4W and by `par_in_system` falling out of its own fast mode.** If either the consumer's chunk shape or the joiner's substrate changes so those stop applying, `a1f` is the arm that reopens axis B. Independently: the separator is **batch granularity**, so any change to steal granularity (a smaller batch cap, a steal-half policy, C-axis batch spawn) makes it a different candidate over a different substrate and voids this measurement. |
| `a1` | `ke16-a1` | rule 2: 16.411 vs 10.354 ms (58.50 % vs a 26.96 % threshold) | Re-measure in a session whose `O5` spread is ≤ 1 % with a same-session `a0` control — `a1` is the only arm whose own machine witness failed (16.84 % drift on a row it cannot touch, against 0.19 % / 0.48 % / 0.96 % / 1.21 % on the others). Second in priority behind `a1f`, because the two tie on all six grid cells. |
| `a2` | `ke16-a2` | rule 3: `worker/10us_4W` 182 993.5 vs 111 278.65 ns = 1.6445×, 8× the threshold | It is the design's NAMED FALLBACK — *"it survives only as the fallback if A1's Miri gate reports UB that D5 cannot remove"*. That condition was **undecidable** while the shared `Scope::prepare` UB stood (a2 red identically at the same site); at CS-2 the UB is fixed. Returns if a residual UB ever proves A1-specific **and** `a3` fails some later gate; or if the ECS criterion-vs-wall dispute is settled in the wall's favour and a re-measure in `a3`'s own session closes the 1.6445×. |
| `a5` | `ke16-a5` | rule 2: 11.784 vs 10.354 ms (13.81 % vs an 8.00 % threshold) | **Returns if the timer guard ships.** `a5`'s whole cost model is one unpark syscall per spawn while any worker is parked, and on this box an unguarded `park_timeout` of any nominal duration below ~15.6 ms costs ~15.6 ms — measured three ways. Owner ruling 4 (*"Timer resolution — RAISE IT"*) puts App-12's raise into the host layer, to be measured both ways. That is a scheduled change to the world and it is exactly what `a5`'s syscall price depends on. Nothing else about `a5` should be reconsidered without it. |
| `b1`, `b3` | `ke16-b1`, `ke16-b3` | Never measured — they do not compile over `a3` | Return if defect B is ever fixed by a route that gives the worker joiner a registered destination deque, or if a future axis-A winner is an A1 arm. |

---

## §Miri. The shared Tree-Borrows red — its two readings, and its fix

### At CS-1: red on all five arms, one signature, zero discrimination

Identical kind-(a) report on **every** arm — `a1` (seed 1), `a1f` (seed 12), `a2` (seed 3), `a3`
(seed 4/9), `a5` (seed 11/14):

```
error: Undefined Behavior: deallocation through <TAG> (root of the allocation)
       at alloc<N>[0x8] is forbidden
  help: the accessed tag <TAG> is foreign to the protected tag <TAG2> (i.e., it is not a child)
  help: this deallocation (acting as a foreign write access) would cause the protected tag
        (currently Frozen) to become Disabled
  accessed tag created at  crates/boyko_threadpool/tests/miri_scope.rs:492:20
  protected tag created at crates/boyko_threadpool/src/scope.rs:1075:22  (Frozen)
  in test nested_scope_from_worker_is_stolen_by_sibling, on thread `boyko-worker-0`
```

Classified per §5 item 14 as **KIND (a)** — a UB report naming a tag and an access. Not kind (b)
(no `spin_until timed out`), not kind (c) (no `receipt not observed`).

Three measured facts that decided how it was read:

1. **Schedule-dependent, not deterministic.** The same test, same features, same flags, run
   **alone**, is GREEN (1.19 s, 1 passed). The Step-0 tester produced that false green
   **deliberately** to show that isolation cannot clear it.
2. **Not A1-specific**, so it cannot be attributed to the FIFO end discipline. `a1` hides it at the
   default seed and shows it at seed 1; `a1f` shows it at the default seed and at seed 12.
3. **`a0` is clean over the same 16 seeds — but that is NOT a clearance**, because the two A-arm
   shapes are `#[cfg_attr]`-ignored at a0 and were never executed. **The baseline is silent, not
   clean.**

`src/scope.rs:1075:22` is the `let wrapped = move ||` of `Scope::prepare`, and the only `#[cfg]`s in
that function are `ke16-c-batch` and `ke16-w-fanout` — axes C and W, both off at c0+w0 — so **the
bytes under Miri are identical across all five arms and the gate cannot discriminate on axis A.**

The shape is this campaign's own recorded finding on a new site: the `// SAFETY:` at
`scope.rs:1102-1111` argues the LAST ACCESS (*"nothing after this line touches it"*,
*"`complete_task` takes the address by value"*) — both true, neither what Tree Borrows checks. **Tree
Borrows judges by the PROTECTOR'S LIFETIME**, which runs to the end of the callee's frame.

A1's own specific obligation was separately GREEN and re-confirmed:
`nested_scope_inline_body_spawns_through_tls_deque_under_live_join` passed 16/16 seeds with the
inline receipt observed 16/16 — no kind-(c) miss, no genuine A1 stopping condition.

### At CS-2: fixed, and armed

On the shipped post-Stage-1 worktree, `miri_scope` is **GREEN on default and on all five A arms**,
0 `Undefined Behavior` occurrences; the dedicated protector gate
`miri_scope_completion_protector` is **GREEN and ARMED on all five** (`firings 4/4` everywhere;
`overlaps` 2/4..4/4 per arm), and under `ke16-w-count,ke16-a2` the W-d′ arm is
`firings=2/2 overlaps=2/2`, 3/3 runs. Anti-vacuity all three arms:

* Defect restored on a scratch mutant differing in exactly one block → **RED**, protected tag now
  at `src/task.rs:402:18` (`let wrapped = move || {`), under default, `ke16-a3` and `ke16-a1-fifo`.
* Gate's own probe weakened (`MIRI_RELEASE_PROBE_YIELDS 16 → 0`, nothing else) → **fails loudly**,
  exit 101, both arms `overlaps=0/4`, with the harness's own DISARMED message. It does not go
  quietly green.
* 24 runs on the shipped tree, 12 at `--test-threads=1` and 12 at default parallelism → **zero
  variation**, every run `firings=4/4 overlaps=3/4` and `4/4 4/4`, exit 0. The gate is not flaky.

### ⚠ A CARDINALITY DISCREPANCY, recorded rather than adjudicated

The campaign brief said the defect *"today reds the Miri gate on ALL FIVE axis-A arms"*, and the
five CS-1 arm testers each measured a red on their own arm. The CS-2 certifier, running the
**reintroduced-defect mutant** of the post-Stage-1 tree, measured
`miri_scope::nested_scope_from_worker_is_stolen_by_sibling` **RED on `ke16-a1` and `ke16-a3` only
(3/3 runs each) and GREEN on `ke16-a1-fifo`, `ke16-a2`, `ke16-a5` and default (3/3 each)** — 2 of 5,
not 5 of 5 — and reported the brief's cardinality as wrong.

**These are two different experiments on two different code states** (the CS-1 shipped defect versus
a CS-2 mutant that reintroduces it into a rewritten task representation), and the test is
schedule-dependent by fact 1 above. Both readings are recorded with their provenance; **this file
does not adjudicate between them.** What both agree on: the red is real, it does not discriminate on
axis A, and it is fixed at CS-2.

Two further CS-2 findings about the instrument itself, both escalated and neither closed:

* **Only ONE of the protector file's two default tests is a gate for this defect.** On the mutant,
  `completer_holds_no_protector_when_the_joiner_frees` is GREEN under all three configurations while
  `body_environment_protector_expires_before_the_borrowed_frame_pops` is RED under all three. The
  first test's bodies borrow the test function's frame, which never pops, so no reclamation races a
  completer. **`test result: ok. 2 passed` reads as two independent certifications. It is one.**
* **The gate file's armedness is coupled to an unrelated function's DEBUG instruction count.**
  Deleting one unfireable `debug_assert!(layout.size() > 0)` from `alloc_cell` — a line that
  compiles out of every release object — moves **all three** arms: body-environment
  `overlaps 3/4 → 2/4`, external-joiner `4/4 → 3/4`, W-d′ `2/2 → 0/2`, deterministic over three
  runs. Only W-d′ crosses the `≥ 1` threshold, so only it fails; **the other two are now one and two
  steps from silence.** The shipped instruction stream is untouched — what is coupled is the
  instrument. The durable fix the harness's own failure message prescribes (re-tune
  `MIRI_RELEASE_PROBE_YIELDS` against an observation) has not been taken.

---

## §Gates. Build / lint / test ladder

At CS-1, `a0+b0+w0+c0`: `cargo check --workspace --all-targets` exit 0;
`cargo clippy -p boyko-threadpool --all-targets -- -D warnings` exit 0 (sources touched first to
defeat the stale-fingerprint false-fresh, a 5.03 s run that recompiled `boyko-log` +
`boyko-threadpool`, not a 0.1 s false-fresh); `cargo test -p boyko-threadpool --tests
--no-fail-fast -- --test-threads=1` exit 0 over 17 binaries; `--all-targets` bare exit 0 and
additionally runs `benches\ke16_nested_scope.rs` in test mode.

At CS-2 the full ladder was run and is green for the threadpool: `cargo check --workspace
--all-targets` (18.01 s after `cargo clean -p boyko-threadpool` removed 2124 files / 154.0 MiB, so
not a false-fresh green); `cargo clippy -p boyko-threadpool --all-targets -- -D warnings` green,
with false-fresh ruled out **by counting** (19 clippy-driver invocations over 18 distinct
crate-names, 0 diagnostics) rather than by the wall clock; `cargo build --release -p
boyko-threadpool` green, which is also what enforces the three `const _: () = assert!(…)` size/align
pins; the crate's own suite green on default (116 tests over 17 targets) and on `ke16-a3` /
`ke16-a1-fifo` / `ke16-w-count` / `ke16-c-batch` (114 / 117 / 115 / 114 passed, 0 failed); the
device-free ignored leg prints `running 1 test` and passes; loom builds, lists 11 models, all 11
pass.

⚠ **THREE WORKSPACE REDS EXIST AND NONE OF THEM IS KE16.** They are named here so a later reader
does not attribute them:

1. `cargo clippy --workspace --all-targets -- -D warnings` → **RED**:
   `crates/boyko_physics/benches/simd_end_to_end.rs:121`, `clippy::assertions_on_constants` on
   `assert!(HAS_AVX2_ARM, "VACUOUS: …")`. The file is untracked and contains zero occurrences of
   `threadpool`. Two second-order points: the failure is at `boyko-physics`, so **every target
   ordered behind it was shadowed** until a re-run with `--exclude boyko-physics` (green); and the
   lint fires only BECAUSE the ISA baseline works — `HAS_AVX2_ARM` is
   `cfg!(target_feature="avx2")`, which `-C target-cpu=x86-64-v3` makes a compile-time `true`. **The
   anti-vacuity guard is itself the thing the linter refuses.**
2. `-p boyko-engine --test internal_docs_anchors` → RED, 19 stale anchors, 17 of them in
   `boyko_ecs`/`boyko_physics`. The red CLAUDE.md itself calls known.
3. `-p boyko-ui --test ui_macro_compile_fail` → RED, 2 of 14 trybuild fixtures mismatching on rustc
   **diagnostic text** only. **It sits BEHIND `internal_docs_anchors` in target order, so a
   fail-fast run reports only the known red and this one never appears.** `--no-fail-fast` earned
   its keep again.

Aggregate over the workspace run: 5544 passed, 0 failed, 157 ignored across 671 result lines.

`cargo fmt -p boyko-threadpool -- --check` is RED at 11 sites, **zero of them in `task.rs`** —
pre-existing, reproduced site-for-site across two independent rounds.

---

## §Instruments. Defects this campaign found in its own gates

Quoted because the next reader will hit them.

### 1. ⚠ §8's occupancy / red-first recipe is STALE for every A arm, and the document's own refusal rule condemns the document's own command

`KE16-DESIGN-MEASUREMENT.md:381` spells the U1 occupancy gate with
`-- --ignored --test-threads=1 --nocapture`. But
`crates/boyko_threadpool/tests/ke16_nested_scope_occupancy.rs:474-482` carries

```rust
#[cfg_attr(not(any(feature = "ke16-a1", feature = "ke16-a1-fifo",
                   feature = "ke16-a2", feature = "ke16-a3")), ignore = …)]
```

and the other two tests (lines 385, 429) have **no `#[ignore]` at all**. So **in exactly the
configuration the gate exists to check — built WITH an A-arm feature — nothing is ignored,
`--ignored` matches nothing, and the command exits 0 having run zero tests.**

Measured, verbatim: `running 0 tests … 3 filtered out … EXIT=0`.

§5 item 1 of the same document forbids that shape by name and requires `running 1 test` from this
very invocation. The test file's own doc comment at line 460 already states the correct behaviour —
*"the moment any A arm is enabled the gate runs in the ORDINARY cargo test leg, with no `--ignored`
needed"* — so **the source and the command contradict each other and the command is the wrong one.**
The fix is a one-word doc edit. The valid receipt was obtained without the flag: `running 3 tests /
3 passed` on both head-to-head arms.

### 2. ⚠ The ECS bench's printed RATIO label is INVERTED

The bench prints `RATIO dispatcher/system wall = …` while **computing
`par_in_system / par_from_dispatcher`** — the metric §3 actually names. Verified arithmetically:
`87.76 / 93.05 = 0.943`, which is the printed `0.94`. The number is right; the label's word order is
backwards. Anyone reading the label and inverting the value gets the reciprocal of the receipt.

### 3. ⚠ `tasklist` is DARK in this environment — a load receipt built on it is green from an emptiness

`tasklist` has returned **zero processes** on this box while PowerShell simultaneously reported
267. An empty process list reads as "clean". **Every load receipt in this campaign was therefore
taken with PowerShell** (`Get-Process` / `Get-Counter`) and **every one proves the tool was alive
before it proves the box was quiet.** In the head-to-head: `(Get-Process).Count` min 268 / max 279
across 162 samples every 8 s, and the observed Game-process count set was **exactly `{5}`** at every
sample — never 0. The primary receipt is a CPU **percentage**, not a list, precisely so it cannot be
green from a zero.

### 4. ⚠ The `park_timeout configuration=` label is not a state classifier

See §0. ~26 % of probes read "guarded" on a fully unguarded box. **Decide state on the criterion
medians; record the label beside them, never instead.** Verified not to split any decision cell.

### 5. ⚠ Two ECS sources disagree, and only for one arm

Protocol wall (best-of-3, the source §3 names) versus criterion medians, at N=65536:

```
a3   wall 0.98 / 1.00 / 1.04    criterion 1.004 / 1.000 / 1.007 / 1.036   both PASS
a1f  wall 1.02 / 1.01 / 1.00    criterion 1.226 / 1.228 / 1.333 / 1.354   wall PASS, criterion FAIL
```

The mechanism is visible in the sample distributions and is not noise: `a1f` **has** `a3`'s fast
mode — its minimum sample, 90.4–91.9 ms, is indistinguishable from its own `par_from_dispatcher` —
but its **median** sits at 117–125 ms with MAD/median 19–20 % on three of four passes. `a3` shows no
such split (p10 91.7 → p50 93.0 → p90 101–116). **Best-of-3 reports the mode it reaches; criterion
reports the mode it lives in.** Which source rule 5 is entitled to use is a design call and remains
**unmade** — it is moot for this verdict because `a3` passes on both, but it is live for any future
candidate, and best-of-3 over a bimodal distribution is a reporting shape this campaign has been
burned by before.

### 6. §5 item 1's "`running 3 tests`" is the a0 count

Under `ke16-b1` or `ke16-b3` the B1-P receipt `parked_joiner_is_claimed_by_a_foreign_wave` is
compiled in and the correct unfiltered count is **4** (measured: `running 4 tests`, all named and
green). A later step holding out for 3 under B1 would be reading a stale constant.

### 7. `loom_pool.rs` and `miri_scope.rs` print `running 0 tests` in the ordinary run

That is **correct** — they are `#![cfg(loom)]` / `#![cfg(miri)]` files — and it is the one place in
this campaign where `running 0 tests` is not a vacuous pass. The loom models really ran under the
`--cfg loom` configuration and the Miri tests really ran under Miri. Neither is the "the tool itself
was dark" shape, and both were separately receipted.

### 8. The disk went to zero and produced TWO different fake compiler reds

First `error: could not compile 'read-fonts' (lib)` with `rustc.exe … exit code 0xc0000005,
STATUS_ACCESS_VIOLATION` on a registry crate with no incremental cache; the retry died with
`error: linking with 'x86_64-w64-mingw32-gcc' failed`, whose **fourth** note reads
`LLVM ERROR: IO failure on output stream: no space on device`. `df -h /d` then read
`238G 238G 0 100%`. **Neither headline names the disk.** Removing `target/debug/incremental`
restored it. Add the ACCESS_VIOLATION face to the recorded "full disk masquerades as mingw" family.

---

## §Receipts

### Occupancy, per configuration (structural; not load-sensitive)

**`a0`, worker route, W=4 and W=16, every repetition:** `max_in_flight=1, lanes_used=1,
top_lane=tasks, off_pool=0`. Defect A, exactly.

**`a1`, worker route:** W=4 three reps `max_in_flight=4/4/4, lanes_used=4/4/4, top_lane=4/4/4,
off_pool=0/0/0, outer_worker_id=0/0/0, outer_same_pool=true`; W=16 three reps `max_in_flight=15/16/16,
lanes_used=15/16/16, top_lane=13/24/21, off_pool=0/0/0`. **Defect A fixed** — 15–16 of 16 lanes.
Note `top_lane` 24 and 21 at W=16: that is **defect B's self-steal signature promoted onto the
worker route** under A1+B0, exactly as `KE16-DESIGN-A` §0 predicts; `off_pool` stays 0 because
`scratch` runs on a registered worker.

**`a1`, dispatcher route (for contrast):** W=4 `max_in_flight=5/5/5, top_lane=8/4/4, off_pool=8/3/3,
outer_worker_id=4294967295 (UNATTACHED), outer_same_pool=false`; W=16 `max_in_flight=12/16/12,
top_lane=16/33/33, off_pool=16/33/33`. The ≤ 33 crossbeam batch is visible and bimodal, as at a0.
`a1` does not touch it — correct, that is B's axis.

**`a3`, axis-W/C reference:** worker route W=4 tasks=16 → `max_in_flight=4 lanes_used=4 top_lane=4
off_pool=0 outer_worker_id=0 outer_same_pool=true` (walls 0.819 / 0.822 / 0.816 ms, speedup vs
serial floor 3.91× / 3.89× / 3.92×); W=16 tasks=64 → `max_in_flight=15/16/15 lanes_used=15/16/15
top_lane=33 off_pool=0` (walls 6.682 / 6.656 / 6.661 ms). Dispatcher route W=16 →
`max_in_flight=15/16/16 top_lane=33 off_pool=33`. **B0's ≤ 33 inline batch is present on BOTH
routes** — under `a3` the worker route inherits it because `a3` leaves the joiner at B0.

**Head-to-head, both arms:** `running 3 tests / 3 passed`. `a3` worker-route lines read
`outer_worker_id ∈ {0,4,5,7}` and `outer_same_pool=true`; `a1f` reads `{0,1,6}` and `true`;
dispatcher lines correctly read `4294967295 / false` on both. **No "healthy route twice" shape
anywhere**: no worker-route number in that session was taken with `outer_worker_id =
4294967294/4294967295` or `outer_same_pool=false`, and a grep across all 24 pass logs for
`panicked|unable to complete|4294967294|4294967295` returned nothing.

### Witness coverage

`KE16_EXPECT` was exported on every recorded invocation of every pass and never mismatched. Every
bench binary printed its own `KE16 variant: …` token on entry, every consumer protocol header
printed `release=true`, and `available_parallelism=16` appears in every one. The witness was
verified **negatively** as well as positively (§A.4): crossing the tokens exits 101.

### Load receipt, head-to-head (the pass whose numbers close axis A)

Sampled by a background PowerShell loop writing a timestamped TSV every 8 s for the whole session;
162 samples across 06:23:05–06:47:38. `(Get-Process).Count` 268–279 throughout; Game process count
set exactly `{5}`; total CPU % min 2.6 / max 100.0 / mean 53.5 (the benches themselves are what
saturates); Game cumulative CPU 1142.03 → 1796.16 s = 654.13 s over 1473 s = **0.444 of one logical
core**. Baseline before any work at 06:15:36: 269 procs, 6.29 % CPU, 5 Game processes.

Every one of the 24 timed windows carries ≥ 5 in-window CPU % samples (pool windows 8–9). A
pre-declared game-CPU-delta outlier rule — discard outside 0.66×–1.5× of the 30.56 s/min session
median — was applied and **no window fell outside** (range 25.11–42.06). **No pass was discarded on
load grounds.**

Sampler overhead disclosed: one PowerShell process waking every 8 s, identical for both arms and
continuous across the alternation, so it cannot bias a comparison. Its first launch failed (exit
127, a backslash path mangled by the shell) and produced no data; it was relaunched at 06:22:10 and
ran from 55 s before the first timed pass through the last.

### Exclusive access, axis-W/C session

`find target/criterion -name estimates.json -newermt "2026-09-06 22:30"` returns exactly five
baseline directories — `a3+b0+w0+c0-r1..r5` — plus criterion's own `new`, at 38 cells each (24 grid
+ 3 park + 6 ECS + 5 physics). No other agent wrote a criterion baseline into the worktree during
that window. A scare was checked rather than assumed: the scratchpad held `pool_r2b`/`ecs_r2b`/
`phys_r2b` files with mtimes 2026-09-04 18:55–18:57 whose witness line reads
`KE16 variant: a1+b0+w0+c0` — **stale artefacts of an earlier session, not a concurrent writer.**

---

## §Refused. Runs and readings that were DISCARDED, with why

A contaminated run named is worth more than one silently dropped.

| What | Why | Values, so nothing is hidden |
|---|---|---|
| **The first `a3`-vs-`a1f` cross-session comparison, AS THE BASIS FOR CLOSING AXIS A** | Margin 1.20 pp raw / 0.72 pp normalised, between arms measured in different sessions on the harness that pass itself proved is not session-comparable. Its judge reported `a3` as the winner while explicitly refusing to call it safe, and named the measurement that would settle it. | Superseded by §A.2, **not deleted**: its per-arm numbers remain the record for `a1`, `a2` and `a5`, which the head-to-head did not re-measure. |
| Head-to-head `a3` p1 (pool 06:23:05–06:24:22, ECS 06:24:29–06:25:24, physics 06:25:24–06:26:13) | **Pre-fixed rule**: each arm's first pass after the build sequence is discarded; the first bench run after a build is systematically slow and two back-to-back runs agree while both drawn from that state. | deciding cell **112 263 ns**, primary **11.397 ms** — pointing the SAME way as the kept passes, so the discard costs `a3` nothing and was applied because it was fixed in advance. |
| Head-to-head `a1f` p1 (06:26:32–06:29:31) | Same pre-fixed rule. | deciding cell **163 464 ns**, primary **12.954 ms** — **p1 was `a1f`'s BEST physics pass**, so the discard removed the loser's most favourable datum. |
| Axis-W/C **r1, all three harnesses** | Protocol discard, taken before any number was read. | pool `worker/1us_w`=9 078 ns, ECS `par_in_system/65536`=95 506 256 ns (MAD 10 364 139, the noisiest ECS cell of the session), physics `in_scheduled_system`=10 048 043 ns, `O5`=27 611 670 ns. ⚠ Noted honestly: r1's physics agrees with the kept set to 0.3 % and its `O5` sits inside the kept spread — **discarded on protocol, not on evidence**. |
| Axis-W/C **r3, POOL GRID ONLY** | Contaminated, on **two independent witnesses that agree**. (i) Load: began at a 16.27 % 5-sample CPU mean and ended at 3.70 % — a before/after disagreement that §8 rule 2 alone makes a re-take. (ii) Signal: **all 24 grid cells inflated simultaneously** and every MAD exploded. | `worker/1us_w` 17 053 ns vs a kept 9 330–9 907 (+72 %, MAD/med 0.30); `worker/1us_4w` 28 792 vs 18 797–20 257; `worker/1us_64w` 281 917 vs ~216 000; `dispatcher/10us_w` 35 808 vs 20 815–26 570. A simultaneous uniform inflation with 5–50× normal MADs is a machine-load signature. **r3's ECS and physics rows, which ran after the burst subsided, are KEPT.** |
| Axis-W/C **r2, PHYSICS ROWS ONLY** | Contaminated, caught by the **arm-independent meter**: `O5` = 28 782 604 ns, 4.4 % above r1/r3/r4/r5 (27 183 283–27 611 670, themselves within 1.6 % of each other). | `in_scheduled_system` read **13 479 998 ns** against a kept 9 874 564–10 547 738 — **32 % high**, MAD/med 0.1457, more than double any kept run — and `Wminus1` 13 911 305 against a kept 9 785 297–9 832 817 (+42 %). **Had it been kept, the PRIMARY number would have been overstated by roughly a third.** r2's pool grid ran before the contamination arrived and is kept. |
| Axis-W/C r2's **ECS rows** | **NOT discarded, but flagged.** They sit in the run whose physics was contaminated, and `par_from_dispatcher/4096`'s MAD (2 758 366 ns) is the largest in the kept ECS set. **Retained because dropping it would NARROW the band on that cell — the direction that invents an elimination for a later candidate** — and because its median (22.44 ms) is not an outlier. | A judge preferring the tighter set can recompute `par_from_dispatcher/4096` over r3/r4/r5 alone: 20 899 704–21 402 415, band 4.00 % (floor) instead of 7.35 %. |
| **A vacuous gate invocation** | `cargo test … --features ke16-a3 -- --ignored --test-threads=1 --nocapture` exited 0 printing `running 0 tests … 3 filtered out`. **That is a vacuous pass, refused shape 1 of §5, and it is NOT a pass.** Load-independent and structural, so it stands regardless of the machine's state. | Re-run without the flag: `running 3 tests / 3 passed` on both arms. See §Instruments 1. |
| **`a1`'s physics numbers** | **NOT discarded** — flagged as the least trustworthy in the pass and eliminated anyway. Its arm-independent `O5` moved **16.84 %** between the two runs it was banded on (26.862 → 31.387 ms); `bench_thread_install` moved 45.41 %, `Wminus1` 21.88 % — on rows no axis-A candidate can touch. | Even taking `a1`'s FASTER run alone (r7, 15.375 ms) it is 48.5 % behind `a3`, still beyond the threshold. |
| **The §4 band as taken at Step 0** | **NOT altered** — reported as §4's formula applied literally, with the warm-up finding recorded beside it, because changing the specification is not the tester's job. | See §Band. |
| Nothing else | No timed head-to-head pass met the outlier condition; no pass produced a criterion "unable to complete N samples" warning, a panic, a witness mismatch, an `outer_worker_id` sentinel on a worker-route number, a `release=false` header, or a differing `available_parallelism`. | — |

---

## §Owed. What this document does NOT contain

Named here rather than in a covering note, because a reader who mistakes this file for a completed
tournament will ship on numbers that were never taken.

1. **AXIS W IS CLOSED ON `wc`; AXIS C IS NOT MEASURED; ONE THING INSIDE W IS STILL OWED.** `wg` and
   `wgc` are eliminated at CS-4 and rule 2's two gates are green, so W\* = `wc` (§W). Owed:
   **(a)** a physics ranking, which this pass cannot supply because its three passes drift
   monotonically and the PRIMARY row's band is 85 % — that needs a session that opens already
   settled, not more passes taken the same way, and it is NOT on the critical path since W\* was
   fixed by gates rather than by numbers and Step App re-takes every consumer row anyway;
   **(b)** `c1`/`c1f`, now buildable and measured as a pair (§C).
   ⚠ The CS-2 reference (49 rows) is preserved as PROSE only — this pass overwrote its criterion
   baselines for r1–r3 under the same directory names (§W, last subsection).
2. **STEP APP HAS NOT BEEN RUN.** No number in this file comes from the unconditional shipped code.
   The design requires the retake on the code that ships, and separately every absolute here is a
   bench-profile number that does not describe the shipped codegen configuration.
3. **THE FREEZE HAS NOT BEEN TAKEN.** No `ke16/tournament` tag; no `docs/threadpool/KE16-REJECTED.md`.
   The order is freeze, then remove. §Rejected supplies the measured return conditions the register
   needs.
4. **RULE 1 IS NOT DISCHARGED FOR THE HEAD-TO-HEAD PASS ITSELF.** Its §8 loom / Miri / clippy /
   full-workspace legs were out of scope by instruction. They sit UPSTREAM of rule 2 — a red there
   deletes the verdict rather than qualifying it. They were later run at CS-2 and are green for the
   threadpool (§Gates), which is reassurance about the code, not a discharge of that pass.
5. **DEFECT B HAS NO REMEDY IN THIS PASS.** Shipping `a3` puts it live on the frame path. §B.
6. **THE ECS SOURCE QUESTION IS UNMADE** — protocol wall versus criterion medians. Moot for this
   verdict, live for any future candidate. §Instruments 5.
7. **THE LIFO-VS-FIFO QUESTION IS OPEN.** The mechanism the design built to answer it (the six
   worker decision cells) could not separate `a1` from `a1f`, and while `a3` holds, neither end
   ships. §Refuted.
8. **W WAS NOT VARIED.** W=16 throughout, from `available_parallelism()`. Forcing W=8 would need a
   bench edit or an affinity mask, and both move the experiment off the phenomenon.
9. **THE PROTECTOR GATE'S ARMEDNESS COUPLING IS NOT FIXED**, only documented at the coupling site.
   §Miri.
10. **THE INSTRUMENT DEFECTS OF §Instruments 1 AND 2 ARE NOT FIXED** — the stale `--ignored` recipe
    and the inverted ECS ratio label. Both are one-line edits that no measuring pass was permitted
    to make.

---

## Appendix. Adjacent measurements taken on the same box in the same window

**NOT PART OF THE TOURNAMENT.** Recorded here because they were taken on the same box, in the same
weeks, under the same measurement queue and the same load discipline, and because a reader
comparing numbers across this campaign will otherwise mix them with KE16's.

### Physics switch pricing (bench profile; `simd_solve` is the one worth acting on)

| Switch | Shipped default | Worth | Confidence |
|---|---|---|---|
| `simd` (O1) | **TRUE** (`resources.rs:444`) | `refresh_inertia` **1.41×** (band 1.399–1.428); `gravity` **~1.0×** (three runs 1.03/0.89/0.95 straddling unity over an 11.5 % denominator spread — no measurable effect, and the noise does not support calling it a regression); `position_integrate` is **not gated by the flag at all** and its 1.44×-slower widened arm is context that corroborates why production hard-codes scalar there | HIGH on the ratio, MEDIUM on transfer |
| `simd_solve` (O7) | **FALSE** (`resources.rs:449`) | **1.96×** on the production-shaped whole colored step (bounds 1.947–1.974, sub-1 % bands), clearing the bench's own documented ≥ 1.8× bar; **2.09×** at the Amdahl-free shape (substeps=8, relax=4) — **the largest priced win in that pass, and it ships DISABLED** | HIGH |
| colored vs reference | reference | **1.059×** @1k, **1.131×** @10k on the honest ratio (`colored_solve_plus_graph`, i.e. graph build included). ⚠ **Not a drop-in speed swap** — the colored sweep order differs, so converged float values DIFFER (equally valid, gated on tolerance acceptance, not a bit-baseline) | HIGH on the numbers; speed does not decide it |
| `parallel_4w` | — | 1.43× — **history, not a verdict.** It dispatches through `pool.scope`, the route KE16 is replacing, and enters via `pool.install` from the bench thread (the healthy dispatcher-labelled route), so it does not even exercise the defect KE16 targets | do not act on it |
| broadphase `GRID_LO`/`GRID_HI` | 96 / 192, both labelled `[ESTIMATE:needs-calibration]` | Measured crossover **~1.1k** (uniform) and **~3.0k** (disparity), bracketed directly to 1000 < n ≤ 10000 on both fixtures. Both constants are too low by ~6–15× | ⚠ **The defect is LATENT, not shipping:** `broadphase_select` defaults to `Manual` and no production site sets `Auto` — only tests. The action is "do not enable Auto until recalibrated", not "we are shipping a regression" |

⚠ **The circulating "3.8×" for `simd` is a KERNEL ratio from a transcription built outside the
repository at a non-shipped profile.** Nothing in-repo lands near it: the in-repo kernel ratios are
1.41× and ~1.0×. And the reconciliation to state plainly — **3.8× and 1.41× are BOTH kernel figures,
so even if they agreed exactly the agreement would prove nothing end-to-end.**

### The end-to-end `simd` A/B — the number that resolved it

A new runtime-switched A/B bench (`crates/boyko_physics/benches/simd_end_to_end.rs`) over the same
~10k pyramid fixture, both arms in one binary from two `PhysicsConfig`s differing in exactly one
field:

* **Whole-step ratio: 1.00× (median 0.9982, range 0.9874–1.0102).** Five runs, both orderings.
* **And it is a structural result, not a failed measurement.** `simd` gates exactly two passes, each
  run **once per substep**: widened scalar = 4 × (105.081 + 18.534) µs = 494.5 µs = **1.686 % of the
  step**; widened simd = 350.5 µs = 1.195 %. Max saving 143.9 µs = 0.491 % ⇒ **Amdahl ceiling
  1.0049×, which sits BELOW the whole-step noise floor (±1.2 %). No whole-step measurement on this
  box can resolve the effect regardless of how quiet the machine is.**
* Arms proven distinct three independent ways, because ~1.00× is exactly what a silently-inert
  switch produces: a compile-time `HAS_AVX2_ARM` assert that panics rather than printing 1.00×; the
  campaign's own `isa_baseline_census` gate (`running 2 tests`, both passed — not a vacuous pass);
  and empirically, `refresh_inertia` scalar 104.917–105.362 µs vs simd 71.209–73.861 µs across three
  interleaved pairs, **ranges non-overlapping**.
* ⚠ **EXECUTION ORDER INVERTED THE SIGN.** With both arms in one criterion group in fixed order
  (scalar first), simd measured 0.36–1.58 % **slower** in 4 of 4 runs — a consistent, plausible
  result that would have shipped as *"the simd switch costs ~1 %"*. Re-running each arm in its own
  process with simd first flipped it (simd faster in 2 of 3 pairs). **The ~1 % was execution order,
  not SIMD.** §Method's alternation finding applies *within* a run too. Note for other instruments:
  `colored_solve.rs`'s `o7_simd_ab` runs eight variants in one group in fixed order with `scalar_*`
  always preceding `simd_*`; its bar is 1.8×, far above a ~1 % bias, so it is unlikely to be
  *decided* by this — but its `_prod` pair should be read with the bias in mind.
* ⚠ **A doc error that would have tripled the expected answer:** `soft_step.rs:906` states
  `refresh_inertia` runs `substeps × (1+relax)` times. It does not — the call sits inside the
  substep loop but **before** the relax loop, so it runs `substeps` = 4 times, not 12. Anyone
  predicting the widened fraction from that comment would expect ~5 % and measure 1.7 %.

### Doc rot found in passing

`MEASUREMENT-QUEUE.md:17-18` cites `docs/threadpool/KE16-MEASUREMENT.md` as *"added in b51d03a7"*.
**That file does not exist.** `git show --stat b51d03a7` shows the commit added
`docs/threadpool/KE16-DESIGN-MEASUREMENT.md`. The idle-machine precondition the whole queue rests on
points at a filename that was never created.

`MEASUREMENT-QUEUE.md:44-48` states the root `Cargo.toml` has no `[profile.release]` and concludes
bench numbers come from "a BETTER-OPTIMISED binary than the one that ships". **False at this HEAD,
and inverted in direction** — see §0. The queue text was written 2026-09-03; the profile changed
2026-09-04 in HEAD `4a363678` itself.

Four bench headers still instruct a `RUSTFLAGS` two-build recipe (`simd_o1.rs:11`,
`colored_solve.rs:241`, `boyko_physics/Cargo.toml:65` and `:97`). On this box that would **delete**
both the ISA baseline and the linker fix, and the failure mode **looks like "the switch does
nothing"**. One site is already correct and is the model (`sdf_narrowphase_o9.rs:11-15`) — but the
fix landed in that `.rs` header and **not** in the same bench's manifest comment at
`Cargo.toml:97`, which still carries the retracted recipe. A header and its manifest disagreeing
about the shipped recipe, where the manifest is the one a reader skims.

---

*Every number in this file is transcribed from the measuring agents' returned objects and the
preserved structured extracts. Where a number could not be sourced, this file says so rather than
supplying one.*
