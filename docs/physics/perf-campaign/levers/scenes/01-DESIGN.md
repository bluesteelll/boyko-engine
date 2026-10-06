# Dynamic parity scenes (lane DYN-SCENES): the record

Owner, ruling 21 of 2026-10-01: every type of interaction must run at maximum speed, under pushes
and the like, not only in the static-box test. This lane adds a dynamic scene set that is
**identical in our runner, Jolt 5.6 and Rapier 0.36**, so window 9c can compare the three on
interactions and not only on J-T's resting pile. Ruling 22 answered the cut's questions and the
critique. Nothing here was timed: every result below is a count, a hash or a byte comparison.

Branch `u/phys-dyn-scenes` from `c335b4c6`. Cut: `D:/tmp/phys-orch/dyn-scenes/cut.md` (its
addendum resolves the critique). Implementation report: `D:/tmp/phys-orch/dyn-scenes/impl.md`.

## 1. The scenes

Every scene starts from **J-T**, unchanged: Jolt's `PyramidScene` placement loop, 1,240 boxes of
half-extent 1 with no convex radius, gap 0.5, on the floor (half-extents (50, 1, 50) at (0, −1, 0)).
Friction 0.2 and restitution 0 everywhere, dt = 1/60 (bits `3c888889`), one step per frame,
sleeping, CCD and damping off. Each engine keeps its own solver: ours flagless V2, Jolt 10 velocity
+ 2 position iterations, Rapier the row's frozen configuration.

| scene | ours | Jolt | Rapier | steps | metric window | what it adds |
|---|---|---|---|---|---|---|
| **S-LAND** | `--scene jolt` | `-s=Pyramid` | the frozen J-T row | 500 | **[0, 100)** | nothing: J-T's landing |
| **S-KICK** | `--dyn kick` | `-dyn=kick` | `--dyn kick` | 800 | **[200, 800)** | from step 200, every 25 steps, 62 boxes (5 %) get a velocity change of 1-5 m/s; 24 events, 1,488 kicks |
| **S-SHOOT** | `--dyn shoot` | `-dyn=shoot` | `--dyn shoot` | 800 | **[200, 800)** | a closed arena (4 walls, a ceiling); from step 200, every 10 steps, a J-T box launched at 15-40 m/s from a 30-34 m ring, 40-48 m up, at a point 4-28 m up the pile axis; 60 launches, 1,240 → 1,300 bodies |
| **S-SLIDE** | `--dyn slide` | `-dyn=slide` | `--dyn slide` | 500 | **[0, 500)** | the arena with the downhill wall's face at x = 30; gravity tilted to an effective 26.57° slope (tan θ = 1/2): the pile slides at ~2.6 m/s², shears, tumbles and piles against the wall |

* **Events** are applied between steps, after the state after t steps is read and before the step
  of loop index t runs: outside the timed pair in all three harnesses. A kick is a **velocity
  change** (`v ← v + Δv`, an f32 add per component), so Jolt's 1,000× density cannot change it.
* **The settle prefix** (ruling 22 Q3): S-KICK and S-SHOOT run J-T's own first 200 steps, untimed in
  the claim (outside the metric window). S-KICK's steps [0, 200) are J-T's bit for bit in our runner
  (a gate).
* **Tilted gravity** (Q4): the same physics as a ramp in the plane's frame, exact in all three
  engines (`408c63a9`, `c10c63a9`, 0: y is exactly −2·x in f32), and the J-T lattice reused bit for
  bit. A literal ramp would need irrational quaternion bits each engine normalises its own way.
* **Box projectiles** (Q8): no new shape, mass ratio 1. Sphere-box pairs are not compared (PC-DYN-4).
* **The arena's statics are large on purpose** (ruling 22 (a)): floors and walls are what games have.
  "Large statics" is a **declared part of what S-SHOOT and S-SLIDE measure**; section 5 gives the
  receipts. Their fix (static-static pairs never reaching the narrowphase, tight static bounds) is
  lever CS's scope, ruling 22 (b), not this lane's.

### Why these parameters (the engines' own suites)

* **Box2D v3** `shared/benchmarks.c`: `CreateSmash` (an 8 m box at 8× density launched at 40 m/s
  into a 120 × 80 grid, zero gravity), `StepRain` (a group spawned every 8 steps), `StepJunkyard` (a
  kinematic pusher sweeping a pile), and the agitation benchmarks `CreateSpinner`, `CreateTumbler`,
  `CreateWasher`.
* **Jolt 5.6 PerformanceTest**: `ConvexVsMesh` (21 × 4 × 21 = 1,764 convex bodies on a *sloped*
  sinusoidal mesh, friction 0.5), `HighSpeed` (5,000 bodies at 250 m/s with LinearCast CCD: outside
  the shared discrete regime), `Ragdoll` (piles dropped from 100 m).
* **Rapier `examples3d`**: the `b3d_*` ports of the Box2D benchmarks (junkyard, rain, washer, the
  pyramids) and `stress_tests/ccd3.rs` (320 bodies at 1,000 m/s with CCD: outside the regime).

So: S-KICK is the agitation family adapted to a shared, seeded, solver-neutral input (25 steps is
about the decay time of a pile disturbance under 30 Hz / ζ 10 contacts, so activity is sustained;
24 × 62 ≈ 1.2 kicks per box over the window). S-SHOOT is smash plus rain: a fixed cadence, speeds on
both sides of V2's 30 m/s velocity-margin saturation, arrivals up to ~50 m/s, far below tunnelling
through a 2 m box (≥ 240 m/s), so the scene is CCD-free stable on all three; body growth 4.8 %.
S-SLIDE is ConvexVsMesh's bodies-on-a-slope, made exact.

## 2. The program generator

Specified once, derived three times: `crates/boyko_physics/benches/jolt_parity_pyramid/dyn_spec.rs`
(ours, and byte-copied into the Rapier `dyn/` arms), the C++ `namespace BoykoDyn` of the Jolt patch
v2, and `gate/dyn_spec_ref.py`. Integer arithmetic only; every float is an exact
`int × 2^-k` (positions in 1/16 m, velocities in 1/256 m/s, velocity changes in 1/64 m/s), so
Jolt's `-ffp-contract=on -mfma` cannot move a bit.

* **PRNG**: SplitMix64 (seed-0 outputs `e220a8397b1dcdaf`, `6e789e6aa1b965f4`, `06c45d188009454f`).
  Seeds: kick `0x6b69636b00000001`, shoot `0x73686f6f00000001`. `below(n)` rejects draws under
  `2^64 mod n`; `range(lo, hi)` is inclusive.
* **Kick event e** (step 200 + 25e): 62 distinct indices `below(1240)` in draw order; then for each
  index in draw order `k = range(−320, 320)³` redrawn until `64² ≤ |k|² ≤ 320²`; Δv = k/64 m/s;
  applied and dumped in ascending index order.
* **Launch k** (step 200 + 10k), draw order: the ring offset `(a, b) = range(−544, 544)²` redrawn
  until `480² ≤ a² + b² ≤ 544²` and, after the first, until ≥ 30° from the previous
  (`dot ≤ 0` or `4·dot² ≤ 3·r²·r²_prev`); `hL = range(640, 768)`; `hT = range(64, 448)`;
  `s = range(15, 40)` m/s. Spawn `(a − 16, hL, b − 16)/16`; `d = (−a, hT − hL, −b)`;
  `n = ⌊√(d·d)⌋` exactly; velocity `trunc(s·256·dᵢ/n)/256` (toward zero). Body index 1240 + k.
* **Statics**, after the J-T boxes (floor first): S-SHOOT walls (1, 31, 52) at (±51, 30, 0) and
  (52, 31, 1) at (0, 30, ±51), ceiling (52, 1, 52) at (0, 62, 0) - inner faces at x, z = ±50 and
  y = 61; S-SLIDE the same with the +x wall at (31, 30, 0); S-KICK and S-LAND the floor only.

## 3. The canonical scene dump (`boyko-dyn-scene v1`)

Written by ours `--scene-dump`, Rapier `--scene-dump`, Jolt `-scene_dump=`, from values **read back
from the engine** (critique W3), every float as 8 hex digits of its f32 bits, LF, one space:

```
boyko-dyn-scene v1
program <none|kick|shoot|slide>
steps <the program's length>
dt <h>                          (the engine's step, read back at the end of the run)
gravity <h> <h> <h>             (read back at the end of the run)
static <half x3> <p x3> <q x4> <friction> <restitution> <convex radius>          (creation order)
body <i> <p x3> <q x4> <v x3> <w x3> <half x3> <friction> <restitution> <convex radius>
     <inv mass / box 0's> <inv inertia diag / box 0's x3> <linear damping> <angular damping> <can sleep> <ccd>
kick <step> <i> <dv x3>         (the velocity read back equals v + dv bit for bit; else `READBACK-MISMATCH`)
launch <step> <the body fields of the projectile, read back after its creation>
end <events applied> <dynamic bodies at the end>
```

The cross-engine identity gate is `cmp` (`gate/compare_dumps.py`, which names the first differing
line and field). Pins (FNV-1a 64 of the canonical text; `tests/dyn_scenes_spec.rs`):

| program | FNV-1a 64 | bytes | lines |
|---|---|---|---|
| none | `0x90a27955d1811c55` | 295,474 | 1,247 |
| kick | `0x964622ebd4902cd7` | 355,172 | 2,735 |
| shoot | `0x7d804c68b157a6ac` | 310,796 | 1,312 |
| slide | `0x3a62433918d71628` | 296,095 | 1,252 |

Each harness also enforces what the dump cannot carry: ours voids a run whose statics are not
static; Rapier voids a body whose gyroscopic setting is not the row's or that may sleep (the row's
`--gyro`, `can_sleep(false)`); every harness voids (or, Jolt, exits 1 on) a kick read-back mismatch.

## 4. The sanity bar: one bar, one scorer (`gate/dyn_sanity.py`)

Each harness writes one CSV row per state under `--sanity` / `-sanity` (row s = the state after s
steps; row 0 the spawn): `step,e_lin,e_rot,e_pot,e_total,e_inj,max_speed,min_x,max_x,min_y,max_y,
min_z,max_z,cx,cy,cz,nonfinite,bodies,clearance,engine_err`. Energies are f64 sums per unit J-T box
mass (`½|v|²`, `⅓|ω|²`, `−g·p`; Jolt's 8,000 kg boxes need no rescale, the formula has no mass).
`e_inj(s)` is the energy injected by the events applied **before** step s (critique O7: row s is
written before the events of step s); an event's injection is the energy of the bodies it changed
just after it minus just before it (a projectile's own energy). `clearance` is set on the row after
which a launch was applied.

A run fails if any of these holds (constants fixed before any engine ran against them, PC-DYN-8):

| bar | condition |
|---|---|
| B1 | a body state value is non-finite |
| B2 | a body centre outside the scene: y ≥ 0; \|x\|, \|z\| ≤ 50; arena y ≤ 61; S-SLIDE x ≤ 30 |
| B3 | a body faster than 70 m/s (the highest legitimate: a 40 m/s shot from 48 m, ~50 m/s) |
| B4 | **energy created from nothing**: with D(s) = e_total(s) − e_inj(s), D(s) − min_{s'<s} D(s') > 620 (every J-T box gaining 1 m/s) |
| B5 | a launch's spawn point closer than 2√3 m to a dynamic body's centre |
| B6 | the engine reported an error for a step (Jolt's `EPhysicsUpdateError`, critique O5) |

Premises (the scene did what it says): every S-KICK event injects energy (24 jumps); S-SHOOT's body
count grows by one per launch; S-SLIDE's centroid moves ≥ 5 m downhill; S-LAND keeps 1,240 bodies.

**Why B4 is a running minimum (critique W1).** Physically D never rises: contacts and friction only
dissipate. The cut's cumulative form `e_total(s) ≤ e_total(0) + e_inj(s) + 1030` could not fail
after the landing, which dissipates 20,601 per unit mass (`0.5 · 9.81 · Σ (15−i)²·i`): an engine
creating up to ~2× S-KICK's whole injection passed it. Measured on a real S-KICK CSV with 700
created at step 400: the running-minimum B4 fires, the cumulative form does not.

**What a failed bar means** (ruling 22 Q7): the engine is excluded from that scene's claim; for ours
it is a STOP and a finding.

## 5. Structural results (2026-10-06; nothing timed)

Every gate below ran from scratch after the crash resume of 2026-10-06 (the pre-crash outputs were
voided). Outputs: `D:/tmp/phys-orch/dyn-scenes/r2/` (`runs/`, `mut/`, `logs/`); the implementation
report `D:/tmp/phys-orch/dyn-scenes/impl.md` names each file.

Our runner was gated twice: on C2's own tree (`c335b4c6` + C1 + C2), and again on the lane head after
the trunk sync (`38c0d04c`, SR phase A) and C3. Both runs give every hash, count and receipt below
identically; on the head, the existing rows are unchanged against window 9b's `c335b4c6` exe (11/11),
the runner pins hold (12/12) and both L10 pose-gate sets pass (264/264, both negatives exit 4, 26/26,
17/17).

### 5.1 One scene in three engines

The scene dump of every harness equals the canonical text byte for byte, for every program: ours at
W1 and W8, Jolt at `-t=1` and `-t=8`, Rapier on every ruling-22 row at W1. The dump is read back from
each engine after creation (shape, convex radius, damping, can-sleep, CCD / motion quality, mass and
inertia ratios, gravity, dt; each kick's velocity read back as `v + Δv` bit for bit), so a projectile
or wall created with an engine default, or a missed gravity override, cannot pass it.

### 5.2 Final poses: determinism and worker counts

| program | ours (`--workers` 1/2/4/8/16) | Jolt 5.6 (`-t=` 1/2/4/8/16) |
|---|---|---|
| none (S-LAND) | `0x441a568e91a4f9c9` (= the trunk's J500 pin) | `0xb8522b4e3fc62cfe` (= window 9b's) |
| kick | `0x5c27d4ede389f12a` | `0x63faa28ed77e1a89` |
| shoot | `0x49e2bf025baa5294` | `0xc989d89c28ecdd03` |
| slide | `0xe627f35c282605ba` | `0x781c433c5d8278f1` |

* **Ours**: one pose at every W (the engine's {1, N} promise), twice at W1 and W8, unchanged by
  `--sanity` and by `--arm-profiler` (armed W1 / W8: `void_steps 0` on every program: the
  structural recount needed no change for the arena's statics or the mid-run spawns, so the STOP the
  cut extended to statics did not fire). On the head run two armed W8 processes voided while the
  machine ran five gate suites at once (S-KICK: one span longer than the profiler's u32 tick range;
  S-SHOOT: 311 dropped samples, so a system "recorded 0 spans"); both re-ran clean twice
  (`void_steps 0`, `drops_total 0`, the same poses): a load artifact of the profiler's anti-vacuity,
  not of the scenes. S-KICK's steps [0, 200) are J-T's bit for bit (`0x46f89695ae477577` both ways).
* **Jolt**: `-t=1` and `-t=8` twice each, unchanged by `-sanity` and `-receipt`; the pose was also
  equal across `-t=1/2/4/8/16` (a receipt: Jolt promises no W-independence).
* **Rapier**: the ruling-22 rows (9b's R-CLAIM union and `simd8/D0`), each at W 1/2/4/8/16 equal to
  W1, twice at W1 and W8, unchanged by `--sanity` and `--receipt`; S-LAND (`--sanity` on the J-T row)
  reproduces each row's frozen fixture. The dynamic programs' hashes:

| row | kick | shoot | slide |
|---|---|---|---|
| `simd8/s1p7q2pd0+roff` | `0xec01f9a343e63d78` | `0xcb5ba15b0fef0bae` | `0x1465647bc2b165f5` |
| `simd8/s1p8q2pd0+roff` | `0x142d964ddcb4455b` | `0xb21d9621143d426b` | `0x890e3eff90128583` |
| `simd8/s1p9q2` | `0xba6c8e3ed1302089` | `0x2fc9a586b9606847` | `0x3b1c5fc0369d3b1a` |
| `simd4/s1p8q3pd0+roff` | `0xba1575baa5653c8e` | `0xd61fe31752686278` | `0xc7f74de28af03456` |
| `block/s1p4q3pd0+roff` | `0x324d42f6d51411c7` | `0x5be135c6b923ddf3` | `0x7e79e2ac5c0e370d` |
| `block/s1p5q2pd0+roff` | `0x69079128ab40cd5a` | `0x63b84973d7033932` | `0x0e628ea206c9cf51` |
| `block/s1p5q3pd0+roff` | `0xcb83b30269076544` | `0xf2365df9295098ba` | `0xf80328e08691aedb` |
| `simd8/D0` | `0x9e933ff0fb6f557e` | `0xeaadaed4c8830fa9` | `0x3afb44f4eaebe777` |

Rapier promises thread-count independence only on its `enhanced-determinism` arm; on these rows it
held on every dynamic scene as it did on J-T (FROZEN-9b section 8), so one fixture per row and program
serves every W (the critique's open question 1). All 70 frozen J-T rows reproduce their fixtures on the
dyn exes at W1 and W8 (140 runs), and one row per arm gives the frozen exe's SUMMARY.

### 5.3 The bar (B1-B6 and the premises), per step at W1

Every engine PASSES on every program, Rapier on all 8 ruling-22 rows (simd8/D0, the non-holding default, included). Receipts from the scorer's JSON:

| engine | program | max speed (m/s) | lowest centre (m) | D's largest rise (bar 620) | launch clearance min (m, bar 3.46) | centroid x moved (m) | injected (per unit mass) |
|---|---|---|---|---|---|---|---|
| ours | none | 11.61 | 0.992 | 34.7 | - | -0.01 | 0 |
| ours | kick | 20.01 | 0.992 | 34.7 | - | 0.04 | 11,318.6 |
| ours | shoot | 45.24 | 0.991 | 34.7 | 5.22 | 0.00 | 51,465.3 |
| ours | slide | 12.90 | 0.992 | 1.4 | - | **19.40** | 0 |
| Jolt | none | 11.94 | 0.994 | 74.6 | - | 0.00 | 0 |
| Jolt | kick | 19.21 | 0.960 | 74.6 | - | 0.06 | 11,207.2 |
| Jolt | shoot | 45.36 | 0.896 | 74.6 | 5.23 | -0.06 | 51,465.3 |
| Jolt | slide | 13.87 | 0.988 | 1.3 | - | **19.42** | 0 |
| Rapier `simd8/s1p7q2pd0+roff` | kick | 19.37 | 0.965 | 159.3 | - | 0.03 | 11,239.4 |
| Rapier `simd8/s1p7q2pd0+roff` | shoot | 45.36 | 0.957 | 159.3 | 5.23 | -0.06 | 51,465.3 |
| Rapier `simd8/s1p7q2pd0+roff` | slide | 13.62 | 0.982 | 9.2 | - | **19.24** | 0.0 |
| Rapier `simd8/s1p8q2pd0+roff` | kick | 20.32 | 0.928 | 168.0 | - | 0.08 | 11,281.3 |
| Rapier `simd8/s1p8q2pd0+roff` | shoot | 45.36 | 0.986 | 168.0 | 5.23 | -0.08 | 51,465.3 |
| Rapier `simd8/s1p8q2pd0+roff` | slide | 13.80 | 0.973 | 10.3 | - | **19.17** | 0.0 |
| Rapier `simd8/s1p9q2` | kick | 20.11 | 0.935 | 125.0 | - | 0.09 | 11,309.3 |
| Rapier `simd8/s1p9q2` | shoot | 45.36 | 0.962 | 125.0 | 5.23 | 0.09 | 51,465.3 |
| Rapier `simd8/s1p9q2` | slide | 13.72 | 0.997 | 1.2 | - | **19.40** | 0.0 |
| Rapier `simd4/s1p8q3pd0+roff` | kick | 19.29 | 0.903 | 144.3 | - | 0.07 | 11,240.4 |
| Rapier `simd4/s1p8q3pd0+roff` | shoot | 45.36 | 0.906 | 144.3 | 5.23 | -0.00 | 51,465.3 |
| Rapier `simd4/s1p8q3pd0+roff` | slide | 13.38 | 0.983 | 7.0 | - | **19.40** | 0.0 |
| Rapier `block/s1p4q3pd0+roff` | kick | 18.51 | 0.957 | 180.4 | - | 0.06 | 11,288.1 |
| Rapier `block/s1p4q3pd0+roff` | shoot | 45.36 | 0.828 | 180.4 | 5.23 | -0.02 | 51,465.3 |
| Rapier `block/s1p4q3pd0+roff` | slide | 13.11 | 0.974 | 195.7 | - | **19.39** | 0.0 |
| Rapier `block/s1p5q2pd0+roff` | kick | 20.13 | 0.974 | 172.9 | - | 0.05 | 11,256.4 |
| Rapier `block/s1p5q2pd0+roff` | shoot | 45.36 | 0.865 | 172.9 | 5.23 | -0.01 | 51,465.3 |
| Rapier `block/s1p5q2pd0+roff` | slide | 14.08 | 0.987 | 157.2 | - | **19.39** | 0.0 |
| Rapier `block/s1p5q3pd0+roff` | kick | 20.05 | 0.922 | 172.8 | - | 0.07 | 11,290.0 |
| Rapier `block/s1p5q3pd0+roff` | shoot | 45.36 | 0.956 | 172.8 | 5.23 | 0.03 | 51,465.3 |
| Rapier `block/s1p5q3pd0+roff` | slide | 13.30 | 0.990 | 16.8 | - | **19.39** | 0.0 |
| Rapier `simd8/D0` | kick | 19.39 | 0.975 | 151.8 | - | 0.07 | 11,274.5 |
| Rapier `simd8/D0` | shoot | 45.24 | 0.862 | 151.8 | 5.22 | 0.01 | 51,465.3 |
| Rapier `simd8/D0` | slide | 14.39 | 0.997 | 33.2 | - | **19.40** | 0.0 |

The pile is settled when the events start: kinetic energy at step 200 is 4.4e-7 (ours) and 3.1e-8
(Jolt) per unit mass. S-KICK's injection differs by engine (ours 11,318.6, Jolt 11,207.2) because a
velocity change injects `v·Δv + ½|Δv|²`, which depends on each engine's own pile; S-SHOOT's is the
projectiles' own energy and is identical. D's largest rise is the landing's (row < 100): a
position-correction transient, an order of magnitude under the bar.

### 5.4 Work receipts and the static pairs (ruling 21 (b), ruling 22)

Means per step over each scene's metric window. Counts, never times; the engines count different
things under one name (ours: the narrowphase's manifolds and points; Jolt: the contact listener's
reduced manifolds; Rapier: solver rows, points and manifolds from `--receipt`).

| engine | program | candidate pairs | manifolds | points | pairs with a static endpoint | static-static pairs | static-static manifolds |
|---|---|---|---|---|---|---|---|
| ours | none [0,100) | 9,570 | 7,085 | 26,508 | 1,240 | 0 | 0 |
| ours | kick [200,800) | 9,405 | 7,104 | 26,671 | 1,240 | 0 | 0 |
| ours | shoot [200,800) | 14,924 | 8,282 | 30,741 | **6,522** | **15** | **12** |
| ours | slide [0,500) | 15,945 | 7,002 | 26,531 | **5,839** | **15** | **12** |
| Jolt | none | - | 7,042 | 24,232 | 225 | 0 | 0 |
| Jolt | kick | - | 7,198 | 27,158 | 232 | 0 | 0 |
| Jolt | shoot | - | 8,251 | 30,449 | 236 | 0 | 0 |
| Jolt | slide | - | 6,201 | 23,091 | 292 | 0 | 0 |

Rapier (`--receipt` over the metric window; `rows` is FROZEN-9b's solver-row count; static pairs from
`contact_pairs()`):

| row | program | solver rows | manifolds | points | pairs with a static endpoint | static-static pairs | static-static manifolds |
|---|---|---|---|---|---|---|---|
| `simd8/s1p7q2pd0+roff` | kick | 200,570 | 5,137 | 18,861 | 232 | 0 | 0 |
| `simd8/s1p7q2pd0+roff` | shoot | 208,160 | 5,462 | 19,488 | 235 | 0 | 0 |
| `simd8/s1p7q2pd0+roff` | slide | 171,529 | 4,539 | 16,033 | 295 | 0 | 0 |
| `simd8/s1p8q2pd0+roff` | kick | 220,792 | 5,162 | 18,982 | 231 | 0 | 0 |
| `simd8/s1p8q2pd0+roff` | shoot | 230,056 | 5,537 | 19,683 | 235 | 0 | 0 |
| `simd8/s1p8q2pd0+roff` | slide | 192,067 | 4,605 | 16,443 | 292 | 0 | 0 |
| `simd8/s1p9q2` | kick | 373,809 | 7,514 | 29,884 | 231 | 0 | 0 |
| `simd8/s1p9q2` | shoot | 413,076 | 8,304 | 33,023 | 237 | 0 | 0 |
| `simd8/s1p9q2` | slide | 304,625 | 6,239 | 24,290 | 292 | 0 | 0 |
| `simd4/s1p8q3pd0+roff` | kick | 256,427 | 5,204 | 19,054 | 232 | 0 | 0 |
| `simd4/s1p8q3pd0+roff` | shoot | 266,158 | 5,535 | 19,667 | 235 | 0 | 0 |
| `simd4/s1p8q3pd0+roff` | slide | 225,961 | 4,659 | 16,730 | 292 | 0 | 0 |
| `block/s1p4q3pd0+roff` | kick | 177,314 | 5,166 | 18,689 | 232 | 0 | 0 |
| `block/s1p4q3pd0+roff` | shoot | 183,625 | 5,428 | 19,254 | 234 | 0 | 0 |
| `block/s1p4q3pd0+roff` | slide | 159,914 | 4,649 | 16,867 | 292 | 0 | 0 |
| `block/s1p5q2pd0+roff` | kick | 162,095 | 5,144 | 18,747 | 232 | 0 | 0 |
| `block/s1p5q2pd0+roff` | shoot | 167,978 | 5,426 | 19,346 | 235 | 0 | 0 |
| `block/s1p5q2pd0+roff` | slide | 133,409 | 4,362 | 15,319 | 292 | 0 | 0 |
| `block/s1p5q3pd0+roff` | kick | 197,847 | 5,192 | 18,890 | 232 | 0 | 0 |
| `block/s1p5q3pd0+roff` | shoot | 204,910 | 5,485 | 19,444 | 234 | 0 | 0 |
| `block/s1p5q3pd0+roff` | slide | 177,386 | 4,648 | 16,944 | 292 | 0 | 0 |
| `simd8/D0` | kick | 311,835 | 7,130 | 28,285 | 231 | 0 | 0 |
| `simd8/D0` | shoot | 348,771 | 7,967 | 31,646 | 238 | 0 | 0 |
| `simd8/D0` | slide | 254,863 | 5,930 | 22,962 | 293 | 0 | 0 |

Ours (armed W1): colour passes 14.6 / 16.0 / 16.0 / 17.5 and colour waves 174.0 / 188.1 / 192.0 /
178.8 per step for none / kick / shoot / slide.

**Large statics, declared (critique C1, ruling 22 (a)).** Our broadphase pairs a body with a static
by bounding sphere: the floor's (corner distance 70.7 m) covers every J-T box, so J-T already pays
1,240 static candidate pairs per step where Jolt pays ~225-292 (its AABB test with the speculative
distance); the arena's walls and ceiling add ~5.3k more (the critique estimated 4-5k), and the six
statics pair with each other (15 pairs, 12 manifolds per step reach the narrowphase output). Jolt
has no static-static pair by construction (NON_MOVING pairs only with MOVING); Rapier excludes
FIXED_FIXED. This extra work is part of what S-SHOOT and S-SLIDE measure for ours, by declaration;
the fix (static-static pairs never reaching the narrowphase or the solver, a tight static bound in
every broadphase) belongs to lever CS (ruling 22 (b)), which re-reads these receipts.

## 6. Pins and fixtures

| what | value |
|---|---|
| the canonical dumps (FNV-1a 64) | section 3 (`tests/dyn_scenes_spec.rs`; `gate/dyn_spec_ref.py` derives the same) |
| ours, W1 = every W (`fixtures/ours_<p>.pose`) | kick `0x5c27d4ede389f12a` (sha256 `91884a6c…`), shoot `0x49e2bf025baa5294` (`dea05a4e…`), slide `0xe627f35c282605ba` (`9107ceb3…`); S-LAND = the trunk's `docs/measurements/2026-09-30-v2-speculative/w8/J500.pose` (byte-equal) |
| Jolt 5.6 dyn exe (`fixtures/jolt56_<p>.pose`, `-t=1`) | none `0xb8522b4e3fc62cfe` (`33a3d320…`), kick `0x63faa28ed77e1a89` (`b409997f…`), shoot `0xc989d89c28ecdd03` (`a7b98658…`), slide `0x781c433c5d8278f1` (`8d41c963…`) |
| Rapier dyn fixtures | `pins.json` `rapier`: per ruling-22 row and program, the pose hash, the fixture path under `D:/tmp/rapier-parity/dyn/fixtures/<arm>/` and its sha256 |
| exes | `pins.json` `exes`: Jolt dyn `9df09901…`; Rapier dyn simd8 `1d1f499f…`, simd4 `6e372558…`, block `41e29ab5…` |

The Rapier `dyn/` sources are copied here as `rapier/*.txt` (`.txt` keeps them out of every Rust
census; `-text`, so their bytes are the ones `dyn/bin/SOURCES.sha256` pins).

## 7. Window 9c (written, not run)

The protocol is 9b's (lib9a verbatim; ruling 1 LETTER; in-block; per W, ours / Jolt / Rapier rows
adjacent with the order alternating per pass; K = 3 per pass; 3 passes **in one launch** (PC-9b-1);
no agent session alive (PC-9b-2)). Every timed process is also checked, untimed, by
`dyn_sanity.py --final <program> OUT.pose` (B1-B3; a failing process is VOID) and by its pose check.

```bash
G=D:/wt/merge/docs/physics/perf-campaign/levers/scenes
FX=$G/fixtures
# ours: the 9c tip runner (cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid)
R=<9c runner exe>
$R --scene jolt --dyn kick  --workers W --steps 800 --window 200..800 --csv OUT.csv --pose-out OUT.pose --expect-pose $FX/ours_kick.pose
$R --scene jolt --dyn shoot --workers W --steps 800 --window 200..800 --csv OUT.csv --pose-out OUT.pose --expect-pose $FX/ours_shoot.pose
$R --scene jolt --dyn slide --workers W --steps 500 --window 0..500 --csv OUT.csv --pose-out OUT.pose --expect-pose $FX/ours_slide.pose
# S-LAND = the J-T row, read over [0,100):
$R --scene jolt --workers W --steps 500 --csv OUT.csv --pose-out OUT.pose --expect-pose D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/w8/J500.pose

# Jolt 5.6: the dyn exe for EVERY Jolt row (ruling 22 Q5); a fresh working directory per process
J=D:/tmp/jolt/build-v5.6.0-dyn-dist/PerformanceTest.exe   # sha256 9df09901... (SHA256SUMS beside it)
$J -s=Pyramid -dyn=kick  -q=Discrete -f -t=W -i=800 -pose_out=OUT.pose   # window [200,800)
$J -s=Pyramid -dyn=shoot -q=Discrete -f -t=W -i=800 -pose_out=OUT.pose   # window [200,800)
$J -s=Pyramid -dyn=slide -q=Discrete -f -t=W -i=500 -pose_out=OUT.pose   # window [0,500)
$J -s=Pyramid            -q=Discrete -f -t=W -i=500 -pose_out=OUT.pose   # S-LAND, window [0,100)
# each Jolt process: exit 0; stdout carries "boyko-parity-dyn v1: program=<p>" (Jolt ignores an
# unknown option, so an old exe would run J-T: critique O2) and the receipts line with
# "readback_mismatches=0, update_errors=0"; the stat line's hash = section 6; cmp OUT.pose $FX/jolt56_<p>.pose

# Rapier 0.36: the dyn arms, the ruling-22 rows (9b's 7-row R-CLAIM union + simd8/D0);
# <row flags> = frozen9b.json `flags` of the row
RD=D:/tmp/rapier-parity/dyn/bin/rapier-parity-dyn-<arm>.exe;  FD=D:/tmp/rapier-parity/dyn/fixtures/<arm>
$RD --dyn kick  --cfg rapier-default <row flags> --workers W --steps 800 --window 200..800 --csv OUT.csv --pose-out OUT.pose --expect-pose $FD/<row>_kick.pose
$RD --dyn shoot --cfg rapier-default <row flags> --workers W --steps 800 --window 200..800 --csv OUT.csv --pose-out OUT.pose --expect-pose $FD/<row>_shoot.pose
$RD --dyn slide --cfg rapier-default <row flags> --workers W --steps 500 --window 0..500   --csv OUT.csv --pose-out OUT.pose --expect-pose $FD/<row>_slide.pose
# S-LAND = the frozen J-T rows on the FROZEN exes (FROZEN-9b section 7 commands), window [0,100)

# The layout bridge (critique W6): in every block that times a Rapier row on a dynamic scene, the same
# row's J-T on the DYN exe and on the FROZEN exe, adjacent, same W:
$RD                                    --cfg rapier-default <row flags> --workers W --steps 500 --window 100..500 --csv OUT.csv --pose-out OUT.pose --expect-pose <frozen fixture>
D:/tmp/rapier-parity/gate/bin/rapier-parity-<arm>.exe --cfg rapier-default <row flags> --workers W --steps 500 --window 100..500 --csv OUT.csv --pose-out OUT.pose --expect-pose <frozen fixture>
# The same bridge for Jolt across windows (only if a 9c Jolt number is compared with 9b's): J-T on
# D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe (9b, 918fd2b7...) adjacent to J-T on $J.

python $G/gate/dyn_sanity.py --final <program> OUT.pose     # every process; B1-B3, a failure is VOID
```

The dyn exes' J-T over the frozen exes' J-T is the binary-layout term (ruling 9 measured 1.119 from
layout alone); a dynamic-scene ratio against Rapier is reported beside it, never corrected by it
silently.

Row templates, in 9b's `rows9b.json` schema (ruling 21 (c): SR, S5, IB and CS add their `arms` to the
same ids):

```json
{"id":"DYN-KICK-ours","blocks":["DYN"],"kind":"runner","binaries":["tip"],"args":["--scene","jolt","--dyn","kick"],"workers":[1,2,4,8,16],"steps":800,"window":[200,800],"metric_windows":[[200,800]],"armed":false,"pose_ref":"scenes/ours_kick","sanity":"final"}
{"id":"DYN-KICK-jolt56","blocks":["DYN"],"kind":"jolt","binaries":["j56dyn"],"args":["-s=Pyramid","-dyn=kick","-q=Discrete","-f"],"workers":[1,2,4,8,16],"steps":800,"metric_windows":[[200,800]],"jolt_hash":"0x63faa28ed77e1a89","jolt_banner":"boyko-parity-dyn v1: program=kick","pose_ref":"scenes/jolt56_kick","sanity":"final"}
{"id":"DYN-KICK-RX.<arm>.<row>","blocks":["DYN"],"kind":"rapier","binaries":["rd-<arm>"],"args":["--dyn","kick","--cfg","rapier-default","<row flags>"],"workers":[1,2,4,8,16],"steps":800,"metric_windows":[[200,800]],"pose_ref":"rapier-dyn/<arm>/<row>_kick","sanity":"final"}
{"id":"DYN-BRIDGE-RX.<arm>.<row>","blocks":["DYN"],"kind":"rapier","binaries":["rd-<arm>","r-<arm>"],"args":["--cfg","rapier-default","<row flags>"],"workers":[1,2,4,8,16],"steps":500,"metric_windows":[[100,500]],"pose_ref":"<the frozen row's fixture>"}
```

The same rows exist for `shoot` (800, [200,800)) and `slide` (500, [0,500)). S-LAND adds no row:
9c's J-T rows read `metric_windows [[0,100],[100,500]]`. Before the window: the per-scene work
receipts (section 5.4) go into 9c's per-row-iteration beside readings, and the Rapier rows are
screened per scene with 9c's R-SCREEN statistic.

**What a claim on these scenes may say.** Against Rapier, the set is ruling 22's (9b's R-CLAIM union
of 7 rows plus D0 as a non-holding baseline), not "Rapier's fastest holding configuration" in ruling
16's all-rows sense (critique W7): every dynamic-scene claim is labelled "against the ruling-22 set".
A row that fails a scene's bar is excluded from that scene's claim; for ours a failed bar is a STOP
(ruling 22 Q7).

## 8. PC lines (record only, ruling 18)

* **PC-DYN-1 (IB):** the runner declares its two child files with `#[path = "jolt_parity_pyramid/…"]`
  (critique W2: a crate root resolves `mod x;` next to itself, and `benches/jolt_parity_pyramid/`
  has no `main.rs`, so Cargo takes no target from it). IB's move of the runner to
  `boyko-physics-parity/src/bin/` must carry the directory `jolt_parity_pyramid/` beside it
  (`src/bin/jolt_parity_pyramid/` without a `main.rs` is not a target either) and re-point
  `tests/dyn_scenes_spec.rs`'s `#[path]`. Either miss is a compile error, not a silent one.
* **PC-DYN-2 (S5, SR):** expect a manual merge in the runner's `Args`, `self_check` literal, parse
  arms, usage literal and SUMMARY format; keep both sides. SR's `--scene pairs` lives in
  `SceneKind`, which this lane does not touch.
* **PC-DYN-3:** Jolt's `CreateAndAddBody` inserts a projectile into its broadphase inside the
  untimed hook; ours and Rapier insert it during the next timed step. In Jolt's favour, bounded by
  one body per 10 steps; stated beside every S-SHOOT claim.
* **PC-DYN-4:** the set covers box-box and box-static pairs only (box projectiles, ruling 22 Q8).
* **PC-DYN-5:** the lane template's trunk-pin list is pre-V2 (its "flagless" J500 and A7-R1 values
  are the `_d0` arms; the alloc census has 19 scenes).
* **PC-DYN-6:** the Rapier dyn exes are outside FROZEN-9b. 9c needs a dyn addendum to the freeze
  (`pins.json`'s `rapier` part per row and program; the validator's V1-V8 per dyn arm).
* **PC-DYN-7:** Jolt runs its default on the dynamic scenes: it is below our fidelity gate (37/44,
  window 9b) and has no holding-configuration search.
* **PC-DYN-8:** B4's 620, B3's 70 m/s and B5's 2√3 m are design constants fixed before any engine
  ran; an engine whose legitimate dynamics trip them is reported, the bar is not re-tuned.
* **PC-DYN-9:** the Jolt patch v2 keeps `Update`'s return value inside Jolt's timed pair (one store,
  critique O5); J-T's hash is unchanged and 9c times every Jolt row on the v2 exe, so it only
  matters when a 9c Jolt number is compared with 9b's (the bridge in section 7).
* **PC-DYN-10:** the static-pair receipts are each engine's own predicate (ours: our sphere-bound
  candidate stream's logical view; Jolt: its AABB test recomputed with the speculative distance;
  Rapier: the narrow phase's contact pairs, i.e. its broad-phase AABB pairs): the same name, not the
  same quantity.

## 9. Corrections to the cut, found while implementing

* The cut's mutant (b) for C1 (the shell bound 4096 → 4095) cannot go red: 4095 = 8·511 + 7 is no sum
  of three squares (critique W4); replaced by `range(−320, 319)` (C1's commit message).
* The cut's C2 mutant (b) predicted a W-identity red at W8 only; a W-dependent shift reds every
  W ≠ 1 (critique O1; measured).
* FROZEN-9b §1 pins **23** files (`frozen9b.json` `files`), not the cut's 24.
* The dump's dynamic-body lines are `body` (and `launch` carries the projectile's full body fields),
  wider than the cut's `box` schema: critique W3's read-back fields.
* The gate scripts are `rapier_gates.py` (not `.sh`) and `scene_receipts.py` (the three engines'
  work receipts side by side).
