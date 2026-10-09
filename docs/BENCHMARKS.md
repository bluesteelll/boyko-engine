# Benchmarks

> Measured results for boyko-engine at commit [`68ec9312`](https://github.com/bluesteelll/boyko-engine/commit/68ec9312968e737730690ebad9077cbb5bd9ffb5), taken on one laptop on 2026-10-09.
> Every comparison is printed in one fixed form: which engine was faster, by what factor, and how
> firmly the measurement protocol supports it. Losses are printed beside the wins. A machine-readable
> copy of every printed result is in [`benchmarks.json`](benchmarks.json).

## At a glance

- **Physics vs Jolt 5.6** on three dynamic scenes (kick, shoot, slide) at 1-16 worker threads: boyko
  physics is 1.08 x to 1.18 x faster in 13 of 15 comparisons (all claimed,
  8 of them claimed STRONG), with sleeping off in both engines. The other two, slide at 8
  and 16 threads, show no measurable difference at this resolution (5.6 % and 5.2 %).
- **Physics vs Rapier 0.36** on the same scenes: boyko physics is 1.39 x to 2.31 x
  **slower** in all 30 comparisons (all claimed, 27 of them claimed STRONG).
- **ECS** vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3),
  Windows system heap: boyko_ecs is faster in six comparisons over four shapes, by 1.43 x to
  48.97 x, and slower on three shapes, by 1.17 x, 1.67 x and 5.89 x (all claimed). The
  48.97 x is g8's worst-sub-batch spike with Bevy not pre-reserved; the 1.43 x is g6b, built with
  the nightly toolchain for both engines; the 5.89 x loss is on a shape whose harness rebuilds the
  system in every timed iteration. Four comparisons show no measurable difference at this resolution
  (1.0 % to 8.1 %).
- **Sleeping** (boyko only, claimed STRONG): with sleeping on, a step of the settled pile is
  5.74 x (W8) to 7.68 x (W1) faster than with sleeping off over steps [100, 500), and
  17.24 x (W8) to 65.96 x (W1) faster once every body is asleep (steps [147, 500)). This block ran
  while the machine was in a degraded state (section 1.5).
- **Pending:** the resting-pile comparisons with Jolt and Rapier, the GPU pass timings and ten
  micro-benchmarks were not measured cleanly. One short re-measure window is owed (section 4).
- **Not benchmarked:** render frame time (no harness exists), and flecs, EnTT, PhysX, Box2D or Avian
  (section 5).

Every number here comes from one machine. Another machine will give other factors.

## 1. How the numbers were taken

### 1.1 The machine

| Part | Value |
|---|---|
| Form factor | Laptop. Its CPU and GPU share the machine's power and cooling; section 1.5 explains why that matters here. |
| CPU | AMD Ryzen 9 5900HS: 8 cores, 16 hardware threads |
| Memory | 16 GiB, 3200 MT/s |
| GPU | NVIDIA GeForce RTX 3060 Laptop GPU, plus the CPU's integrated AMD Radeon Graphics. No printed result uses either. |
| OS | Windows 11 Home Single Language, build 26200 |
| Power | The Windows "High performance" scheme, logged before every pass. The AC state was not logged during the window. |

### 1.2 What was built, and how

| Subject | Version | Compiler | Build |
|---|---|---|---|
| boyko physics (the physics runner) | commit `68ec9312`. The runner binary was built from `d8320451`; its code is identical to `68ec9312` (the diff outside `docs/` is empty). | rustc 1.98.1, `stable-x86_64-pc-windows-msvc` | `cargo bench --profile parity`. `parity` inherits `[profile.release]`: fat LTO, opt-level 3. |
| boyko_ecs and the micro-benchmarks | commit `68ec9312` | rustc 1.98.1 msvc | `cargo bench --profile bench-shipped`. `bench-shipped` inherits `[profile.release]` and sets nothing else. |
| bevy_ecs | 0.18.1, from `Cargo.lock` | the same compiler as boyko_ecs | the same profile and flags as boyko_ecs, in the same binary |
| g6 / g6b (both engines) | as above | rustc 1.100.0-nightly (2026-09-09), `nightly-x86_64-pc-windows-msvc` | `bench-shipped`, `--features nightly` |
| Jolt Physics | 5.6.0 (`e77f175`) with the parity patch v2 from [`pyramid_scene.patch`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch) | MinGW-w64 g++ 16.1.0 | CMake `Distribution`, LTO, AVX2 / FMA / F16C / LZCNT / TZCNT |
| Rapier | rapier3d 0.36.0 (`--locked`), two builds: `simd8` (8 SIMD lanes) and `block` (the block solver) | rustc 1.98.1 msvc | x86-64-v3. The harness lives outside this repository. |

Every boyko and Bevy build targets `x86-64-v3` (AVX2) through `.cargo/config.toml`. The prep checked
the shipped configuration from cargo's `-v` rustc lines, for every crate linked into each benchmark
binary: `opt-level=3`, `target-cpu=x86-64-v3`, the default codegen units, fat LTO. bevy_ecs and
boyko_ecs carry the identical flag set.

**One build setting is not the shipped one.** No binary was built with `BOYKO_PROFILE` set, so the
engine's diagnostics profile was its default, `dev`, which compiles the profiling zones in
([`build.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_diag/build.rs#L195)). The physics runner reports `profile_name dev`
and `zones_compiled true` in every process. "Shipped configuration" below therefore means the Cargo
codegen (fat LTO, x86-64-v3), not the `shipping` diagnostics profile. The setting adds work on boyko's
side only.

### 1.3 The protocol

The measurements ran in a quiet window: a driver started every benchmark process, and no build or
other work was meant to run on the machine. Section 1.5 says where that broke.

- **One process is one sample.** A physics process runs a scene and reports the mean wall time per
  step over the scene's timed window. boyko and Rapier time each step with a pair of `Instant`
  reads; Jolt uses its own per-frame clock. A criterion process reports criterion's typical
  estimate (the slope) for each benchmark id. Both engines' ids of a comparison run in the same
  process, boyko first.
- **K = 9 samples per cell: 3 passes x 3 rounds.** Pass 0 runs the list in reverse, pass 1 forward,
  pass 2 in reverse. Inside a pass, the arms of one comparison run back to back, so both see the same
  machine state. Each executable gets one untimed warm-up per pass. The micro-benchmark absolutes
  use one pass of 3 rounds (K = 3).
- **The load guard.** Before each pass the driver waits for an idle machine: no build process on
  three one-minute polls in a row, and under 10 % CPU over 10 s. A sample counts only if the machine
  was at most 5 % busy in the 2 s before and after it, other processes used at most 2 % of it during
  the run, and no build process ran. Unclean samples are re-run at the end of the pass, in up to four
  waves, and the first clean re-run takes the slot. If an arm has fewer than 3 clean samples in a
  pass, that pass gates nothing, so the comparison cannot be claimed.
- **Validity.** Every process checks its executable's sha256 against a pin. Every physics process
  compares its final pose byte for byte with a pinned fixture (Jolt: its printed state hash). In the
  printed blocks there were 0 mismatches.
- **One launch.** A claim reads all three passes from one launch of the driver. A block split across
  launches is re-run whole.

**The claim rule.** The cell of each arm is the median of its samples. Its spread is measured three
ways, each relative to the median: the interquartile range (*i*), the standard error of the median,
1.2533 x SD / sqrt(K) (*s*), and the min-max range (*r*). A test passes when the relative difference
between the two medians exceeds twice the combined spread, 2 x sqrt(a² + b²).

- **CLAIMED:** the *i* and *s* tests both pass in each of the three passes and in the pooled nine
  samples, with the same sign every time.
- **CLAIMED STRONG:** the *r* test passes everywhere as well.
- **NOT RESOLVED:** anything else. The result states its resolution: the smallest difference the
  spreads could have claimed, rounded up to one decimal.

### 1.4 How to read a printed statement

- Each comparison takes r = boyko's cell / the other engine's cell.
- A CLAIMED comparison prints "boyko ... is X.XX x faster than ..." when r < 1 and "X.XX x slower"
  when r > 1, with X.XX = max(r, 1/r) rounded to two decimals. The subject is always boyko, and a
  factor below 1 is never printed.
- A NOT RESOLVED comparison prints only "no measurable difference at this resolution (X %)". This is
  not parity: a difference smaller than X % may exist. The tables leave its cells out, so no ratio
  can be read from it.
- Tables show each cell as the median over K, with the min-max in parentheses.
- Descriptive absolutes (section 2.4) compare nothing and claim nothing.

### 1.5 The two launches, and a machine-state caveat

The window ran in two launches on 2026-10-09 (times are local, UTC+03:00):

| Launch | Time | Printed |
|---|---|---|
| 1 | 14:16-16:12 | Nothing. The Claude desktop app was active, most samples were unclean, and Windows hung at about 16:11. |
| 2 | 19:53-23:26 | Every result in this document. |

**A slowdown the load guard cannot see.** The analysis found a machine-wide slowdown that coincides
with the Claude desktop app being active, and that passes the CPU-load guard. In launch 1, clean
samples ran 8.9 % (boyko), 6.5 % (Jolt) and 32-39 % (Rapier) slower than in launch 2. In launch 1's
quiet minutes (16:05-16:12), boyko and Jolt matched launch 2 within 1 %. The most likely cause is the laptop's shared
CPU/GPU power budget: the app's renderer works on the GPU, which no CPU receipt sees.

In launch 2 the app was idle until about 22:50:

| Block | Time | State |
|---|---|---|
| Physics vs Jolt and Rapier | 19:58-20:58 | quiet |
| ECS vs Bevy, first set | 21:01-21:33 | quiet |
| Micro-benchmarks, first set | 21:35-21:51 | quiet |
| ECS vs Bevy, second set | 22:03-22:33 | quiet |
| Micro-benchmarks, second set | 22:52-23:05 | the third round ran inside the slowdown |
| GPU pass timings | 23:07-23:16 | inside the slowdown; withheld |
| Sleeping on vs off | 23:19-23:26 | inside the slowdown; printed with a note |

**The factors are a dated snapshot.** The same boyko and Jolt executables also ran in an earlier
window on this machine, early on 2026-10-09. On the dynamic scenes they took 2.5-12.0 % (boyko)
and 8.5-21.3 % (Jolt) less time per step in this window. The two engines did not move by the same
amount, so a factor measured on another day can differ.

## 2. Results

### 2.1 Physics: boyko vs Jolt 5.6 and Rapier 0.36 on three dynamic scenes

Every scene starts from Jolt's `PyramidScene`: 1,240 boxes of half-extent 1 m, placed with a 0.5 m
gap on a floor. Friction is 0.2, restitution 0, dt = 1/60 s, one step per frame. Sleeping, CCD and
damping are off in all three engines. Each engine runs its own solver: boyko its defaults, Jolt 10
velocity + 2 position iterations, Rapier the row's configuration.

| Scene | Steps | Timed window | What happens |
|---|---|---|---|
| kick | 800 | steps [200, 800) | The settled pile. From step 200, every 25 steps, 62 boxes (5 %) get a velocity change of 1-5 m/s: 24 events, 1,488 kicks. |
| shoot | 800 | steps [200, 800) | The pile inside a closed arena (4 walls and a ceiling). From step 200, every 10 steps, a box is launched into the pile at 15-40 m/s: 60 launches, 1,240 to 1,300 bodies. |
| slide | 500 | steps [0, 500) | Gravity tilted to an effective 26.57° slope. The pile slides, shears, tumbles and piles up against a wall. |

- **The same work in every engine.** Events are applied between steps, outside the timed region, in
  all three harnesses. Every process reported the same work: kick 1,488 kicks on 1,240 bodies with 1
  static (the floor), shoot 60 launches ending at 1,300 bodies with 6 statics, slide 1,240 bodies
  with 6 statics.
- **Large statics are part of what shoot and slide measure.** boyko's broadphase pairs a body with a
  static by bounding sphere, so it handles more body-static candidate pairs than Jolt does, plus a few
  static-static pairs that Jolt and Rapier exclude by construction. The
  [scenes' design record](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/docs/physics/perf-campaign/levers/scenes/01-DESIGN.md) lists the counts.
- **W is the worker-thread count:** boyko `--workers W`, Jolt `-t=W`, Rapier `--workers W`, each
  with its own threading model.
- **How to read the Rapier labels.** An earlier window (9b, 2026-10-05) screened a frozen set of 70
  Rapier configurations on the resting pile. At each W it ranked the fastest configuration that keeps
  the pile standing; that configuration is "9b's J-T rank-1 row". "Ruling 22's set" is the set of
  Rapier rows the project pinned for these scenes. The rank-1 rows were ranked on the resting pile,
  not on these scenes, so they are never called "Rapier's fastest on this scene". The s1p9q2 row is
  the only Rapier configuration that passes the 88-run fidelity ensemble (section 2.5). A row name
  spells out its
  harness options: `s1p7q2pd0+roff` stands for `--substeps 1 --pgs 7 --stab 2 --prediction 0
  --recycling off`.

**Against Jolt 5.6** (Jolt runs with `allow_sleep=0`):

| Scene | W | boyko (ms/step) | Jolt 5.6 (ms/step) | Printed statement | K | Date | Commit |
|---|---|---|---|---|---|---|---|
| kick | 1 | 7.8528 (7.8088-7.9720) | 9.2719 (9.1975-9.3433) | boyko physics (trunk 68ec9312) is 1.18 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W1; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 2 | 4.6937 (4.6446-4.8383) | 5.5069 (5.4677-5.5597) | boyko physics (trunk 68ec9312) is 1.17 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W2; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 4 | 2.9119 (2.9013-2.9239) | 3.4159 (3.4056-3.4845) | boyko physics (trunk 68ec9312) is 1.17 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W4; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 8 | 2.0695 (2.0551-2.0855) | 2.3866 (2.3760-2.4956) | boyko physics (trunk 68ec9312) is 1.15 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W8; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 16 | 2.1406 (2.1057-2.1667) | 2.3335 (2.2909-2.4123) | boyko physics (trunk 68ec9312) is 1.09 x faster than Jolt 5.6 (allow_sleep=0) (claimed; W16; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 1 | 9.1867 (9.1603-9.3392) | 10.2065 (10.0440-10.3011) | boyko physics (trunk 68ec9312) is 1.11 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W1; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 2 | 5.3559 (5.3085-5.6009) | 6.0549 (5.9677-6.3947) | boyko physics (trunk 68ec9312) is 1.13 x faster than Jolt 5.6 (allow_sleep=0) (claimed; W2; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 4 | 3.3537 (3.2953-3.4985) | 3.7546 (3.7276-4.0178) | boyko physics (trunk 68ec9312) is 1.12 x faster than Jolt 5.6 (allow_sleep=0) (claimed; W4; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 8 | 2.3215 (2.3050-2.4361) | 2.6349 (2.6204-2.7645) | boyko physics (trunk 68ec9312) is 1.14 x faster than Jolt 5.6 (allow_sleep=0) (claimed; W8; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 16 | 2.3542 (2.2999-2.3796) | 2.5504 (2.5143-2.5886) | boyko physics (trunk 68ec9312) is 1.08 x faster than Jolt 5.6 (allow_sleep=0) (claimed; W16; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 1 | 8.7228 (8.6112-8.8026) | 10.0546 (10.0089-10.2254) | boyko physics (trunk 68ec9312) is 1.15 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W1; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 2 | 5.1644 (5.1043-5.3012) | 5.8229 (5.7818-5.8650) | boyko physics (trunk 68ec9312) is 1.13 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W2; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 4 | 3.1959 (3.1582-3.2547) | 3.5514 (3.5074-3.6101) | boyko physics (trunk 68ec9312) is 1.11 x faster than Jolt 5.6 (allow_sleep=0) (claimed STRONG; W4; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 8 | - | - | boyko physics (trunk 68ec9312) vs Jolt 5.6 (allow_sleep=0) (W8; slide [0,500)): no measurable difference at this resolution (5.6 %) | 9 | 2026-10-09 | `68ec9312` |
| slide | 16 | - | - | boyko physics (trunk 68ec9312) vs Jolt 5.6 (allow_sleep=0) (W16; slide [0,500)): no measurable difference at this resolution (5.2 %) | 9 | 2026-10-09 | `68ec9312` |

Slide at W8 and W16 is not parity. The protocol could not resolve a difference smaller than
5.6 % and 5.2 %.

**Against Rapier 0.36, the rank-1 row of each W:**

| Scene | W | boyko (ms/step) | Rapier 0.36, rank-1 row (ms/step) | Printed statement | K | Date | Commit |
|---|---|---|---|---|---|---|---|
| kick | 1 | 7.8528 (7.8088-7.9720) | 4.5727 (4.4878-4.9305) | boyko physics (trunk 68ec9312) is 1.72 x slower than Rapier 0.36 against 9b's J-T rank-1 row (simd8/s1p7q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W1; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 2 | 4.6937 (4.6446-4.8383) | 2.8282 (2.7252-3.0051) | boyko physics (trunk 68ec9312) is 1.66 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed STRONG; W2; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 4 | 2.9119 (2.9013-2.9239) | 1.8475 (1.7296-2.2617) | boyko physics (trunk 68ec9312) is 1.58 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed; W4; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 8 | 2.0695 (2.0551-2.0855) | 1.1715 (1.1230-1.3566) | boyko physics (trunk 68ec9312) is 1.77 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W8; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 16 | 2.1406 (2.1057-2.1667) | 1.1866 (1.1545-1.2682) | boyko physics (trunk 68ec9312) is 1.80 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W16; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 1 | 9.1867 (9.1603-9.3392) | 4.9330 (4.7560-5.2152) | boyko physics (trunk 68ec9312) is 1.86 x slower than Rapier 0.36 against 9b's J-T rank-1 row (simd8/s1p7q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W1; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 2 | 5.3559 (5.3085-5.6009) | 3.0568 (2.8991-3.6936) | boyko physics (trunk 68ec9312) is 1.75 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed STRONG; W2; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 4 | 3.3537 (3.2953-3.4985) | 2.0357 (1.8232-2.3711) | boyko physics (trunk 68ec9312) is 1.65 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed STRONG; W4; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 8 | 2.3215 (2.3050-2.4361) | 1.2589 (1.2205-1.3833) | boyko physics (trunk 68ec9312) is 1.84 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W8; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 16 | 2.3542 (2.2999-2.3796) | 1.2574 (1.2177-1.4389) | boyko physics (trunk 68ec9312) is 1.87 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W16; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 1 | 8.7228 (8.6112-8.8026) | 5.5806 (5.3948-5.9129) | boyko physics (trunk 68ec9312) is 1.56 x slower than Rapier 0.36 against 9b's J-T rank-1 row (simd8/s1p7q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W1; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 2 | 5.1644 (5.1043-5.3012) | 3.3806 (3.2759-3.8440) | boyko physics (trunk 68ec9312) is 1.53 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed STRONG; W2; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 4 | 3.1959 (3.1582-3.2547) | 2.1975 (2.0572-2.4231) | boyko physics (trunk 68ec9312) is 1.45 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p4q3pd0+roff; a member of ruling 22's set) (claimed STRONG; W4; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 8 | 2.2476 (2.2227-2.3464) | 1.3461 (1.3221-1.5196) | boyko physics (trunk 68ec9312) is 1.67 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W8; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 16 | 2.2678 (2.2394-2.3217) | 1.3425 (1.3039-1.4403) | boyko physics (trunk 68ec9312) is 1.69 x slower than Rapier 0.36 against 9b's J-T rank-1 row (block/s1p5q2pd0+roff; a member of ruling 22's set) (claimed STRONG; W16; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |

**Against Rapier 0.36, the s1p9q2 row:**

| Scene | W | boyko (ms/step) | Rapier 0.36, s1p9q2 row (ms/step) | Printed statement | K | Date | Commit |
|---|---|---|---|---|---|---|---|
| kick | 1 | 7.8528 (7.8088-7.9720) | 3.8742 (3.7063-4.2144) | boyko physics (trunk 68ec9312) is 2.03 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W1; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 2 | 4.6937 (4.6446-4.8383) | 2.4369 (2.3768-2.4733) | boyko physics (trunk 68ec9312) is 1.93 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W2; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 4 | 2.9119 (2.9013-2.9239) | 1.6739 (1.5847-2.1781) | boyko physics (trunk 68ec9312) is 1.74 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W4; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 8 | 2.0695 (2.0551-2.0855) | 1.2149 (1.1484-1.3914) | boyko physics (trunk 68ec9312) is 1.70 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W8; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| kick | 16 | 2.1406 (2.1057-2.1667) | 1.3318 (1.2240-1.7718) | boyko physics (trunk 68ec9312) is 1.61 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed; W16; kick [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 1 | 9.1867 (9.1603-9.3392) | 4.4859 (4.3560-4.9528) | boyko physics (trunk 68ec9312) is 2.05 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W1; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 2 | 5.3559 (5.3085-5.6009) | 2.8844 (2.7499-3.6913) | boyko physics (trunk 68ec9312) is 1.86 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W2; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 4 | 3.3537 (3.2953-3.4985) | 2.0785 (1.9877-2.2331) | boyko physics (trunk 68ec9312) is 1.61 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W4; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 8 | 2.3215 (2.3050-2.4361) | 1.5899 (1.4621-1.7687) | boyko physics (trunk 68ec9312) is 1.46 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W8; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| shoot | 16 | 2.3542 (2.2999-2.3796) | 1.6930 (1.5787-2.0160) | boyko physics (trunk 68ec9312) is 1.39 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed; W16; shoot [200,800)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 1 | 8.7228 (8.6112-8.8026) | 3.7762 (3.6246-3.9319) | boyko physics (trunk 68ec9312) is 2.31 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W1; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 2 | 5.1644 (5.1043-5.3012) | 2.4152 (2.3501-3.3779) | boyko physics (trunk 68ec9312) is 2.14 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W2; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 4 | 3.1959 (3.1582-3.2547) | 1.7345 (1.6572-1.8768) | boyko physics (trunk 68ec9312) is 1.84 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W4; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 8 | 2.2476 (2.2227-2.3464) | 1.2720 (1.2030-1.4051) | boyko physics (trunk 68ec9312) is 1.77 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W8; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |
| slide | 16 | 2.2678 (2.2394-2.3217) | 1.3879 (1.3319-1.5956) | boyko physics (trunk 68ec9312) is 1.63 x slower than Rapier 0.36 against Rapier's s1p9q2 row (claimed STRONG; W16; slide [0,500)) | 9 | 2026-10-09 | `68ec9312` |

### 2.2 ECS: boyko_ecs vs bevy_ecs 0.18.1

Both engines are built in the shipped configuration (fat LTO, x86-64-v3) and run on the Windows
system heap. One process holds both engines' ids of a comparison and always runs the boyko id first;
an in-process order effect is not measured. The cell is criterion's typical estimate per process,
with the median over 9 processes. The spike rows use the harness's own median-of-iteration-spikes
line.

| Shape | What it does | boyko_ecs | bevy_ecs 0.18.1 | Printed statement | K | Date | Commit |
|---|---|---|---|---|---|---|---|
| g1 | `Schedule::run` of 50 empty systems, 8-thread pool, multi-threaded executor on both sides | 8.238 µs (8.024 µs - 8.641 µs) | 23.55 µs (21.73 µs - 23.96 µs) | boyko_ecs is 2.86 x faster than bevy_ecs 0.18.1 (claimed STRONG; g1 50 empty systems; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g2 | a 10k-row read-only query run through `world.run_system` inside the timed loop, which rebuilds the boyko system every iteration; bevy iterates a pre-built `QueryState` (see the note below the table) | 39.45 µs (39.35 µs - 39.47 µs) | 6.697 µs (6.587 µs - 6.714 µs) | boyko_ecs is 5.89 x slower than bevy_ecs 0.18.1 (claimed STRONG; g2 query over 10k rows through world.run_system, which rebuilds the boyko system in every timed iteration, against a pre-built bevy QueryState; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g2b | a 10k-row read-only query through the direct API (boyko `EcsMaster::query`, bevy `QueryState::iter`) | 11.24 µs (11.07 µs - 12.58 µs) | 6.713 µs (6.695 µs - 6.729 µs) | boyko_ecs is 1.67 x slower than bevy_ecs 0.18.1 (claimed STRONG; g2b query_iter 10k direct; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g3 | `par_iter` over 10k rows, 8 threads | - | - | boyko_ecs vs bevy_ecs 0.18.1 (g3 par_iter 10k; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap): no measurable difference at this resolution (1.0 %) | 9 | 2026-10-09 | `68ec9312` |
| g4 | `Commands::spawn` x 10k, apply included | - | - | boyko_ecs vs bevy_ecs 0.18.1 (g4 commands spawn 10k; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap): no measurable difference at this resolution (8.1 %) | 9 | 2026-10-09 | `68ec9312` |
| g5 | `Commands::spawn_batch` of 10k | 206.7 µs (194.6 µs - 211 µs) | 177.1 µs (170.1 µs - 181.8 µs) | boyko_ecs is 1.17 x slower than bevy_ecs 0.18.1 (claimed; g5 commands spawn_batch 10k; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g8 total | 1,000,000 entities into one archetype (3 x 192 B components) in 100 sub-batches of 10k, cold world, total time; Bevy not pre-reserved | 116.3 ms (115.4 ms - 118.4 ms) | 314 ms (306.4 ms - 320.2 ms) | boyko_ecs is 2.70 x faster than bevy_ecs 0.18.1 (claimed STRONG; g8 growth total (1 M entities, one archetype); Bevy not pre-reserved; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g8 spike | the same run: the worst sub-batch minus the median sub-batch, median over iterations | 1.552 ms (1.501 ms - 1.62 ms) | 76.02 ms (75.77 ms - 78 ms) | boyko_ecs is 48.97 x faster than bevy_ecs 0.18.1 (claimed STRONG; g8 worst-sub-batch spike (median-of-iteration-spikes); Bevy not pre-reserved; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g7 total | 960,000 entities into 16 archetypes (3 x 192 B components), total time | 148.7 ms (146.9 ms - 151.4 ms) | 289.3 ms (286.4 ms - 302.4 ms) | boyko_ecs is 1.95 x faster than bevy_ecs 0.18.1 (claimed STRONG; g7 growth total (960k entities, 16 archetypes); vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g7 spike | the same run: the worst sub-batch spike, median over iterations | 449.3 µs (419.7 µs - 538.3 µs) | 8.428 ms (8.358 ms - 8.648 ms) | boyko_ecs is 18.76 x faster than bevy_ecs 0.18.1 (claimed STRONG; g7 worst-sub-batch spike (median-of-iteration-spikes); vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g6 | sum of one f32 field over 10k rows: boyko `for_each_chunk` vs bevy `iter().fold`, algebraic float add (nightly toolchain, both engines) | - | - | boyko_ecs vs bevy_ecs 0.18.1 (g6 algebraic sum 10k (boyko for_each_chunk vs bevy iter fold); g6: nightly toolchain, both engines; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap): no measurable difference at this resolution (3.4 %) | 9 | 2026-10-09 | `68ec9312` |
| g6b idx | a three-component reduction over 10k rows: boyko `for_each_chunk` (indexed) vs bevy `iter().fold` (nightly toolchain, both engines) | 871.7 ns (862.4 ns - 875.5 ns) | 1.244 µs (1.236 µs - 1.247 µs) | boyko_ecs is 1.43 x faster than bevy_ecs 0.18.1 (claimed STRONG; g6b triple 10k (boyko for_each_chunk idx vs bevy iter fold); g6: nightly toolchain, both engines; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap) | 9 | 2026-10-09 | `68ec9312` |
| g6b zip | the same reduction: boyko `for_each_chunk` (zipped) vs bevy `iter().fold` (nightly toolchain, both engines) | - | - | boyko_ecs vs bevy_ecs 0.18.1 (g6b triple 10k (boyko for_each_chunk zip vs bevy iter fold); g6: nightly toolchain, both engines; vs bevy_ecs 0.18.1, both engines built in the shipped configuration (fat LTO, x86-64-v3), Windows system heap): no measurable difference at this resolution (1.4 %) | 9 | 2026-10-09 | `68ec9312` |

**About g2.** The boyko side calls `world.run_system` inside the timed loop, which builds the system
again in every iteration. Bevy iterates a `QueryState` it built once. The bench's own source calls
the rebuild "a harness artifact, not a steady-state cost"
([`comparison_v2.rs:164-172`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/bench_bevy_vs_boyko/benches/comparison_v2.rs#L164-L172)):
a real schedule builds a system once. A variant that caches the system (`g2c`) exists in the same
binary but was not part of this window. The direct-iteration comparison is g2b, a 1.67 x loss.

### 2.3 Physics: sleeping on vs off (boyko only)

The scene is the resting pile (`--scene jolt --gap 0.5`), 500 steps, at W1 and W8. When sleeping is
on, every body is asleep from step 147 onward, in every process. The windows are the landing
[0, 100), the settled pile [100, 500), and the frozen tail [147, 500).

| W | Steps | Sleeping ON (ms/step) | Sleeping OFF (ms/step) | Printed statement | K | Date | Commit |
|---|---|---|---|---|---|---|---|
| 1 | [0,100) | - | - | boyko physics with sleeping ON vs the same engine with sleeping OFF (W1; [0,100)): no measurable difference at this resolution (5.9 %) | 9 | 2026-10-09 | `68ec9312` |
| 1 | [100,500) | 1.1093 (1.0836-1.1449) | 8.5165 (8.4681-8.6245) | boyko physics with sleeping ON is 7.68 x faster than the same engine with sleeping OFF (claimed STRONG; W1; [100,500)) | 9 | 2026-10-09 | `68ec9312` |
| 1 | [147,500) | 0.1292 (0.1199-0.1315) | 8.5222 (8.4637-8.6464) | boyko physics with sleeping ON is 65.96 x faster than the same engine with sleeping OFF (claimed STRONG; W1; [147,500) frozen tail) | 9 | 2026-10-09 | `68ec9312` |
| 8 | [0,100) | - | - | boyko physics with sleeping ON vs the same engine with sleeping OFF (W8; [0,100)): no measurable difference at this resolution (16.3 %) | 9 | 2026-10-09 | `68ec9312` |
| 8 | [100,500) | 0.4014 (0.3658-0.4056) | 2.3052 (2.2443-2.4792) | boyko physics with sleeping ON is 5.74 x faster than the same engine with sleeping OFF (claimed STRONG; W8; [100,500)) | 9 | 2026-10-09 | `68ec9312` |
| 8 | [147,500) | 0.1335 (0.1232-0.1363) | 2.3013 (2.2484-2.4884) | boyko physics with sleeping ON is 17.24 x faster than the same engine with sleeping OFF (claimed STRONG; W8; [147,500) frozen tail) | 9 | 2026-10-09 | `68ec9312` |

- The comparisons with Jolt and Rapier in section 2.1 run with sleeping **off** in every engine.
  boyko with sleeping on is not compared with any other engine.
- This block ran from 23:19 to 23:26, inside the slowdown described in section 1.5. Both arms ran
  back to back in the same block, so the direction and the size of the effect hold. The two-decimal
  factors carry a bias that could not be quantified, and the absolute ms/step cells were measured
  inside the slowdown too. A re-confirm is owed (section 4).

### 2.4 Micro-benchmark absolutes (descriptive)

These are absolutes for boyko alone. Each compares nothing and claims nothing. Every value is the
median over K = 3 processes (one pass of 3 rounds), with the min-max over those 3. The profile is
`bench-shipped`, the shipped `[profile.release]` codegen (fat LTO, x86-64-v3), on one machine
(Ryzen 9 5900HS, Windows 11, MSVC 1.98.1).

**First set** (21:35-21:51, quiet):

**`boyko_ecs` bench `phase9_scheduler`: schedule runs - an empty schedule, 50 exclusive systems, `par_iter` over 4,096 entities, two disjoint systems, one exclusive system**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `phase9_schedule_run_empty` | 6.579 ns | 6.578 ns - 6.599 ns | 3 |
| `phase9_schedule_run_50_exclusive_systems` | 1.975 µs | 1.975 µs - 1.978 µs | 3 |
| `phase9_par_iter_4096_entities` | 37.3 µs | 37.1 µs - 37.79 µs | 3 |
| `phase9_schedule_run_two_disjoint` | 876.6 ns | 853.9 ns - 952.2 ns | 3 |
| `phase9_schedule_run_one_exclusive` | 82.87 ns | 82.52 ns - 83.06 ns | 3 |

**`boyko_ecs` bench `app_overhead`: the App driver, per frame and per fixed substep**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `app_overhead/empty_main` | 22.12 ns | 22.08 ns - 22.16 ns | 3 |
| `app_overhead/fixed_loop_1_substep` | 32.93 ns | 32.88 ns - 33.46 ns | 3 |
| `app_overhead/bare_empty_schedule_run` | 6.774 ns | 6.769 ns - 6.801 ns | 3 |

**`boyko_ecs` bench `ecs_master_new`: constructing a world**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `ecs_master_new/EcsMaster::new` | 4.411 µs | 4.335 µs - 4.488 µs | 3 |

**`boyko_ecs` bench `query_iter`, time per call: `query_state_iter` walks a cached `QueryState`'s matching archetypes and sums their entity counts; `query_find_into` runs one archetype-registry scan. The id's number is n: the world holds n entities in each of two archetypes. Neither call reads a component row**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `query_state_iter/1000` | 1.518 ns | 1.516 ns - 1.523 ns | 3 |
| `query_state_iter/10000` | 1.513 ns | 1.511 ns - 1.52 ns | 3 |
| `query_state_iter/100000` | 1.517 ns | 1.513 ns - 1.518 ns | 3 |
| `query_find_into/1000` | 13.82 ns | 13.75 ns - 14.17 ns | 3 |
| `query_find_into/10000` | 13.78 ns | 13.73 ns - 13.88 ns | 3 |
| `query_find_into/100000` | 13.81 ns | 13.7 ns - 13.9 ns | 3 |

**`boyko_ecs` bench `random_access`: get / has / create / delete / dense iteration**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `get_component_raw_hot` | 5.237 ns | 5.222 ns - 5.238 ns | 3 |
| `get_component_raw_cold` | 1.018 ms | 1.016 ms - 1.027 ms | 3 |
| `get_component_typed` | 5.265 ns | 5.243 ns - 5.269 ns | 3 |
| `has_entity` | 7.779 ns | 7.763 ns - 7.827 ns | 3 |
| `iter_entities_dense_10k` | 4.967 µs | 4.967 µs - 5.01 µs | 3 |
| `delete_entity_10k` | 377 µs | 335.5 µs - 383 µs | 3 |
| `create_entity_10k` | 668.5 µs | 665.4 µs - 670.3 µs | 3 |

**`boyko_ecs` bench `event_dispatch`: send / update / read**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `event_send_warm_cache` | 6.892 µs | 6.594 µs - 6.925 µs | 3 |
| `event_update_events_1_type` | 62.19 ns | 56.24 ns - 75.81 ns | 3 |
| `event_read_iteration_1k` | 327.9 ns | 323 ns - 328.1 ns | 3 |

**`boyko_ecs` bench `phase12_5_spawn_batch`: `spawn_batch` of 10k entities with 1 and 3 components (this bench's own ids)**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `spawn_batch_10k_1comp` | 203.3 µs | 197.3 µs - 209 µs | 3 |
| `spawn_batch_10k_3comp` | 373.9 µs | 370.2 µs - 381.5 µs | 3 |
| `spawn_batch_direct_10k_1comp` | 204.2 µs | 195.4 µs - 218.2 µs | 3 |
| `component_ids_static_pin` | 0.6456 ns | 0.6455 ns - 0.6487 ns | 3 |

**`boyko_ecs` bench `ke16_par_iter_in_system` (SECONDARY): 16 threads, 20 µs synthetic bodies - a regression receipt, never ECS iteration speed (the `par_iter` comparison with Bevy is g3)**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `ke16_par_iter_in_system/seq/4096` | 81.92 ms | 81.92 ms - 81.92 ms | 3 |
| `ke16_par_iter_in_system/par_from_dispatcher/4096` | 20.6 ms | 20.54 ms - 20.6 ms | 3 |
| `ke16_par_iter_in_system/par_in_system/4096` | 20.54 ms | 20.54 ms - 20.55 ms | 3 |
| `ke16_par_iter_in_system/seq/65536` | 1.311 s | 1.311 s - 1.311 s | 3 |
| `ke16_par_iter_in_system/par_from_dispatcher/65536` | 83.84 ms | 83.83 ms - 84.26 ms | 3 |
| `ke16_par_iter_in_system/par_in_system/65536` | 83.86 ms | 83.71 ms - 84.03 ms | 3 |

**Second set** (22:52-23:05). Its third round ran inside the slowdown described in section 1.5. Ten
ids of this set ran more than 5 % slower in that round than the mean of the first two, so they are
withheld (section 4). None of the twelve ids below slowed by more than 5 % in it; two of them,
`query_init_state_50_archetypes` and `ke16_solve_route/empty_schedule_control/29751`, ran 6.6-6.8 %
faster.

**`boyko_ecs` bench `enable_tags`**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `enable_toggle` | 18.98 ns | 18.96 ns - 18.98 ns | 3 |
| `query_iter_enabled` | 45.18 µs | 45 µs - 45.57 µs | 3 |
| `enable_toggle_large_archetype` | 18.99 ns | 18.94 ns - 19.06 ns | 3 |

**`boyko_ecs` bench `query_dsl`: the query DSL**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `query_ref_iter_10k` | 10.14 µs | 10.05 µs - 10.17 µs | 3 |
| `query_tuple_2_ref_iter_10k` | 8.343 µs | 8.34 µs - 8.491 µs | 3 |
| `query_mut_iter_10k` | 8.048 µs | 7.898 µs - 8.09 µs | 3 |
| `query_with_archetypal_filter_10k` | 7.787 µs | 7.761 µs - 7.891 µs | 3 |
| `query_archetype_transition_2x5k` | 10.18 µs | 10.1 µs - 10.35 µs | 3 |
| `query_cold_construction` | 98.35 µs | 97.85 µs - 105.2 µs | 3 |
| `query_init_state_50_archetypes` | 451.5 ns | 440.1 ns - 528.4 ns | 3 |

**`boyko_physics` bench `ke16_solve_in_system`: the contact solve over 29,751 contacts, by route**

| Benchmark id | Median | Min - max | K |
|---|---|---|---|
| `ke16_solve_route/empty_schedule_control/29751` | 1.171 µs | 1.157 µs - 1.306 µs | 3 |
| `ke16_solve_route/single_threaded_O5/29751` | 14.36 ms | 14.35 ms - 14.84 ms | 3 |

**Logging (`boyko_log`).** These benches print their own verdict against a bound written in the
bench source. This window prints the verdict words only, not the timings.

| Row | What it times | Verdict (words only) | K |
|---|---|---|---|
| `log_disabled_warn` | a `warn!` call on a target switched off at run time | NO SUBJECT in all 3 processes: the disabled call costs what an empty loop costs; the gate folded and there is nothing to bound | 3 |
| `log_enabled_0args` | an enabled log call with no arguments | PASS in all 3 processes (the bench's bound: 15 ns per call) | 3 |
| `log_enabled_2u32` | an enabled log call with two u32 arguments | PASS in all 3 processes (the bench's bound: 20 ns per call) | 3 |
| `log_enabled_str32` | an enabled log call with a 32-byte string argument | PASS in all 3 processes (the bench's bound: 30 ns per call) | 3 |
| `log_enabled_rate_once_fired` | a `once`-rate-limited call whose latch has already fired | verdict not stable over the 3 processes (OVER BOUND, NO SUBJECT, NO SUBJECT) | 3 |
| `log_enabled_sampled_out` | a call that sampling suppresses | PASS in all 3 processes (the bench's regression guard) | 3 |
| `downstream_code_warn` | a `warn!` carrying a log code defined outside the engine (a downstream crate) | absolute PASS in all 3 processes (the bench's absolute bound: 18 ns per call) | 3 |
| `sink_sustained_rate` | the text sink format | PASS in all 3 processes (one verdict for both sink legs: the bench's regression guard on the binary format's speed-up over the text format) | 3 |
| `sink_sustained_rate_binary` | the binary sink format | PASS in all 3 processes (the same verdict as the text leg) | 3 |

### 2.5 Structural facts (gated by tests, not timed)

These hold on `68ec9312` and are checked by tests in the repository. They are counts and byte
comparisons, not timings.

- **Determinism.** The default physics world produces a bit-identical trajectory of the 1,240-box
  pyramid run to run, at 1, 2 and 8 workers, and against the scalar solver with SIMD off. The final
  hash is pinned. Test:
  [`default_world_pyramid_determinism.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/tests/default_world_pyramid_determinism.rs).
  In this window, boyko produced the same final pose at every W from 1 to 16 on each dynamic scene.
- **No data-path heap allocation in a physics step.** A counting allocator records every heap
  acquisition of a steady physics step on three pile scenes, in release and debug. In release, the
  only acquisitions are the thread pool's: 3 scope frames and 3 chunks per step, whatever the colour
  count, pinned exactly, plus the pool injector's dispatcher-side block on about one step in eight.
  Reallocations are 0 in both builds. Every other class is 0 in release; a debug build adds one
  debug-assertion scratch allocation per step. Test:
  [`alloc_frame_census.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/tests/alloc_frame_census.rs#L553-L584).
- **No data-path heap allocation in an engine frame.** The same census over a steady frame of the
  headless `EnginePlugins` + UI app: the pool takes 2 scope frames and 2 chunks per frame, plus at
  most one injector block; everything else is 0, in debug and release. Test:
  [`e1_engine_ui_alloc_census.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_app/tests/e1_engine_ui_alloc_census.rs#L686-L700).
- **Fidelity.** The J-T pile must still stand at step 500, after its landing, at every drop gap from
  0.5 to 2.0 m, with contact reuse on and off. boyko holds in 88 of 88 runs; Jolt 5.6's default
  holds in 37 of 44
  ([`v2_speculative_fidelity.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/tests/v2_speculative_fidelity.rs#L14-L18)
  records both counts; its in-tree gate, an ignored `slow` test run in release, repeats 26 of the
  runs). Rapier 0.36's default configuration does **not** hold the pile: it holds in 6 of 22 cells of
  the gap sweep. Its s1p9q2 configuration is the only one that holds all 88 runs. The Rapier counts
  come from the project's Rapier harness, whose records are outside this repository (2026-10-01).

## 3. Losses, stated plainly

- **boyko physics is slower than Rapier 0.36 in every dynamic-scene comparison:** 1.39 x to
  2.31 x slower, all 30 claimed. Against the rank-1 rows the range is 1.45 x to
  1.87 x; against s1p9q2 it is 1.39 x to 2.31 x. The rank-1 rows were chosen on the
  resting pile, and no per-scene screen of Rapier's configurations was run; such a screen could only
  find an equal or faster Rapier row for these scenes.
- **boyko_ecs is slower on three shapes**, vs bevy_ecs 0.18.1, both engines built in the shipped
  configuration (fat LTO, x86-64-v3), Windows system heap:
  - g2b, direct query iteration over 10k rows: boyko_ecs is 1.67 x slower (claimed STRONG).
  - g5, `spawn_batch` of 10k: boyko_ecs is 1.17 x slower (claimed).
  - g2, a 10k-row query through a system that the harness rebuilds every iteration: boyko_ecs is
    5.89 x slower (claimed STRONG). Section 2.2 explains why this shape is a harness artifact.
- **boyko physics against Jolt 5.6 at slide W8 and W16:** no measurable difference at this
  resolution (5.6 % and 5.2 %). That is not a win, and it is not parity.

## 4. Pending: owed to a re-measure

One short re-measure window is owed: the Claude desktop app **closed** (not minimised), one fresh
launch, covering everything below.

| What | Why it is not printed |
|---|---|
| boyko physics vs Jolt 5.6 and Rapier 0.36 on the resting pile (steps [100, 500)) and on the landing (steps [0, 100)), W 1-16 | It ran in launch 1. Every one of the 30 comparisons has a pass with fewer than 3 clean samples, and no Rapier pass reached 3 clean samples, so every Rapier cell is empty and neither a claim nor a resolution can be stated. A boyko-only series from the same block is withheld too: its cells come from launch 1, mostly from one pass, inside the slowdown. |
| Per-pass GPU timings (visibility-buffer and deferred passes) on one synthetic scene at 1920x1080 | One pass's timing is bimodal across and within processes. Every GPU process ran while the Claude desktop app was active, and nothing measured GPU contention. The display state was not recorded. |
| Ten micro-benchmark ids: `spawn_with_enable_tag/spawn_then_enable`, `spawn_with_enable_tag/plain_spawn`, `pob/load`, `decode_png_fixed_huffman/256x256`, `decode_png_fixed_huffman/1024x1024`, `ke16_solve_route/bench_thread_install/29751`, `ke16_solve_route/bench_thread_install_Wminus1/29751`, `decode_png_fixed_huffman/64x64`, `ke16_solve_route/in_scheduled_system/29751`, `pob/save` | Their third process ran inside the slowdown and was more than 5 % slower than the mean of the first two. |
| Sleeping on vs off | Printed in section 2.3 with a note. The re-measure confirms the factors outside the slowdown. |
| An internal A/B of a broadphase change (S5) | A development decision, not a published comparison. No number from it appears here. |

The resting-pile and landing comparisons against Jolt 5.6 and Rapier 0.36 were not measured cleanly
in this window and are not shown.

## 5. Not benchmarked

- **Render frame time.** No frame-time harness exists. The only `boyko_render` bench times the
  CPU-side packing of UI instances and renders nothing.
- **Other ECS engines:** flecs, EnTT, hecs, legion and Unity DOTS have no harness.
- **Other physics engines:** PhysX, Box2D / Box3D and Avian have no harness.
- **Bevy 0.19.** The comparison uses bevy_ecs 0.18.1. Bevy 0.19 is newer and the harness was not
  ported.
- **ECS shapes not compared with Bevy:** adding and removing components, despawn, fragmented
  iteration, change detection.
- **Physics body-count scaling** against Jolt.
- **The `shipping` diagnostics profile** (section 1.2).

## 6. Reproducing

The window's driver and its raw records are not part of the repository. The benchmark sources and
the build recipes are (except the Rapier harness), so you can rebuild the binaries and repeat each
run by hand.

**Builds.** Run from a checkout of `68ec9312` with `RUSTFLAGS` unset: it would replace the
`x86-64-v3` flag that `.cargo/config.toml` sets. `cargo bench --no-run` prints each executable's path.

```powershell
# boyko physics runner (profile parity = [profile.release])
cargo +stable-x86_64-pc-windows-msvc bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid

# ECS vs Bevy (stable; g6 needs nightly and the feature)
cargo +stable-x86_64-pc-windows-msvc bench --no-run --locked --profile bench-shipped -p bench-bevy-vs-boyko --bench comparison --bench comparison_v2 --bench growth_crossing_pool --bench growth_crossing
cargo +nightly-x86_64-pc-windows-msvc bench --no-run --locked --profile bench-shipped -p bench-bevy-vs-boyko --bench g6_for_each_chunk --features nightly

# Micro-benchmarks
cargo +stable-x86_64-pc-windows-msvc bench --no-run --locked --profile bench-shipped -p boyko-ecs --bench phase9_scheduler --bench app_overhead --bench ecs_master_new --bench query_iter --bench random_access --bench event_dispatch --bench phase12_5_spawn_batch --bench ke16_par_iter_in_system --bench enable_tags --bench query_dsl
cargo +stable-x86_64-pc-windows-msvc bench --no-run --locked --profile bench-shipped -p boyko-physics --bench ke16_solve_in_system
cargo +stable-x86_64-pc-windows-msvc bench --no-run --locked --profile bench-shipped -p boyko-log --bench log_disabled_cost --bench log_enabled_cost --bench sink_sustained_rate
```

Jolt 5.6 is built from its `v5.6.0` tag with
[`pyramid_scene.patch`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch) applied.
The recipe, the compiler and the CMake options are in the header of
[`jolt_parity_pyramid.rs`](https://github.com/bluesteelll/boyko-engine/blob/68ec9312968e737730690ebad9077cbb5bd9ffb5/crates/boyko_physics/benches/jolt_parity_pyramid.rs#L420-L485).

**Runs.** One process per sample; the window ran K = 9 of them per cell, in the order of section 1.3.

```powershell
# boyko: one dynamic scene at W workers (kick and shoot: 800 steps, timed [200, 800); slide: 500 steps, timed [0, 500))
<runner.exe> --scene jolt --dyn kick --workers 8 --steps 800 --window 0..800 --csv run.csv --pose-out pose.bin

# Jolt 5.6: the same scene, in a fresh working directory (it writes per_frame_discrete_th8.csv)
PerformanceTest.exe -s=Pyramid -dyn=kick -q=Discrete -f -t=8 -i=800

# Rapier 0.36: the W8 rank-1 row on the same scene (harness outside this repository)
<rapier-dyn-block.exe> --cfg rapier-default --substeps 1 --pgs 5 --stab 2 --prediction 0 --recycling off --dyn kick --workers 8 --steps 800 --window 200..800

# boyko sleeping on (off: --sleeping off)
<runner.exe> --scene jolt --gap 0.5 --cfg default --sleeping on --workers 8 --steps 500 --window 0..500 --csv run.csv

# A criterion benchmark: the window ran each binary with these arguments and a fresh CRITERION_HOME
<comparison.exe> --bench --noplot "^g[1-4]_(boyko|bevy)_"
<comparison_v2.exe> --bench --noplot "^(g2b_(boyko|bevy)_query_iter_10k_direct|g5_(boyko|bevy)_commands_spawn_batch_10k)$"
<random_access.exe> --bench --noplot "^(get_component_raw_hot|get_component_raw_cold|get_component_typed|has_entity|create_entity_10k|delete_entity_10k|iter_entities_dense_10k)$"

# The logging benches have their own main and print their verdicts
<log_enabled_cost.exe> --bench
```

[`docs/BENCHMARKING.md`](BENCHMARKING.md) describes the methodology for local criterion A/B runs on
a noisy Windows machine, and the [book's benchmark page](https://bluesteelll.github.io/boyko-engine/reference/benchmarks.html)
lists every harness and build profile.

**Binaries.** Every executable behind a printed result, with the sha256 its processes were checked
against:

| Binary | sha256 | Used in |
|---|---|---|
| boyko physics runner (`jolt_parity_pyramid`, profile `parity`), built from `d8320451`, whose code equals `68ec9312` | `8693116feb427880f1d2c152217854b35006530d26eec5b39367cfcc154645af` | section 2.1, 2.3 |
| Jolt 5.6.0 `PerformanceTest` with the parity patch v2 (MinGW g++ 16.1, Distribution, LTO) | `9df0990168bef31f6ba722ff6950b9b6bbfb5e9077a174d17c27c95eb0913a98` | section 2.1 |
| Rapier 0.36.0 dynamic-scene harness, arm simd8 | `c71f747042688c9558aaee2bdb8a0e7bfaf79642cd99c25d8621acd8274ebfeb` | section 2.1 |
| Rapier 0.36.0 dynamic-scene harness, arm block | `5b0a1b60c23e3b34f6a16c3bf15ab4f591d3469d4805ad667dfa26cb06e5fb6b` | section 2.1 |
| `bench-bevy-vs-boyko` bench `comparison` (g1-g4) | `decef42905a3203f9e839f936c7ff4b8d3df0a157ca2c8e8f0120a764bcc20b9` | section 2.2 |
| `bench-bevy-vs-boyko` bench `comparison_v2` (g2b, g5) | `0d1aa171981f3174a39cd962f79e48eabb82476abcdec2ba540de14e978dc09a` | section 2.2 |
| `bench-bevy-vs-boyko` bench `growth_crossing_pool` (g8) | `76f357c7993dd44838a3cf8a2ad2ae0e69c3e8d6da97e1491fa9d054c5c74591` | section 2.2 |
| `bench-bevy-vs-boyko` bench `growth_crossing` (g7) | `8d89991696dd57258433e163a62abcb219693823c152dd94a4abc1315c1120c2` | section 2.2 |
| `bench-bevy-vs-boyko` bench `g6_for_each_chunk` (g6, g6b; nightly) | `71e420f3f2885b09b60f8d01b0434c6c27344f4b9e397253da6f0e79a931a4ae` | section 2.2 |
| `boyko-ecs` bench `phase9_scheduler` | `ae5177baff3ad555d75bd6c8391f80c12e240d85a486f09b039fc2702699d3d9` | section 2.4 |
| `boyko-ecs` bench `app_overhead` | `b112c360e2f593e78bddfeac1d944628fa2215d4bee91ca8e725e8ef233cf461` | section 2.4 |
| `boyko-ecs` bench `ecs_master_new` | `aebe9d5ff884f5a20895c27ca5c66a921ec56da7bbdeb8fbc170ab3f205c8184` | section 2.4 |
| `boyko-ecs` bench `query_iter` | `d33635bef92d87504ca078ebfe6fe1009c67316e2066fd09d85cdbaee3430371` | section 2.4 |
| `boyko-ecs` bench `random_access` | `64294b60b5347c659b8139248fc90eb4321aa2a0e8d9cf763e97780f1a1f22a1` | section 2.4 |
| `boyko-ecs` bench `event_dispatch` | `f9d1af5f9cc26a9a1271adb694258ce63a23f41558ee290f11b0f6d2938501d7` | section 2.4 |
| `boyko-ecs` bench `phase12_5_spawn_batch` | `cd020b28eda6751fd35a80fdfa409ac2818fba99b8b8a6be8759c40cf9cdc93f` | section 2.4 |
| `boyko-ecs` bench `ke16_par_iter_in_system` | `8ab45b3c93e4955995d36f983656e62469a5f1ff487575a0ce559e42002d89e9` | section 2.4 |
| `boyko-ecs` bench `enable_tags` | `21df1c094c4f9759153920f2d70e4a39e1a029953cc4145922e2b14bd942f74b` | section 2.4 |
| `boyko-ecs` bench `query_dsl` | `bffb9297ac443d81241839e0eba01898c7de0ae25592710b83d05ba4b618f0ff` | section 2.4 |
| `boyko-physics` bench `ke16_solve_in_system` | `b55d9205373fe25ce939e738bc743bb96a65a6ab2bbd243607fc10670548bd68` | section 2.4 |
| `boyko-log` bench `log_disabled_cost` | `36b1089bfb76d69b2310c1d4389d3c2ed8659b44540d79fd5a55df01024666eb` | section 2.4 |
| `boyko-log` bench `log_enabled_cost` | `598541a5cb8a754cf4c96f2a37feb74e5723cba051d14e43e07d1712397a6577` | section 2.4 |
| `boyko-log` bench `sink_sustained_rate` | `9832c6d63377d61edbeeda4dc8a75a6afafea6f7b8aea7fc9970da033c8ff787` | section 2.4 |

## 7. The machine-readable file

[`benchmarks.json`](benchmarks.json) holds the same 120 printed results, one entry each, plus
the machine, the toolchains, the protocol, the pending items and what is not measured.

- `verdict` is `CLAIMED_FASTER`, `CLAIMED_SLOWER`, `NOT_RESOLVED` or `DESCRIPTIVE`.
- `ratio.factor` is the printed factor, never below 1, and `ratio.word` is its direction. `ratio` is
  `null` for NOT_RESOLVED and DESCRIPTIVE entries.
- NOT_RESOLVED entries carry no values, so no ratio can be computed from them.
- `printed` is the exact statement this document prints.
