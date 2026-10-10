# Research: L12, caching the effective masses per inertia epoch (bit-identical)

How this was gathered: I read `D:/wt/joltab` with Read and Grep, and `D:/wt/l5np` only for its config defaults. Web sources are listed at the end. This role has no shell, so I did not verify that the tree is at `47c5dabd`, graphify was not run, and no cargo command was run. "arith." means computed from P0 spans, the in-tree opcode census or published instruction tables; it is not a measurement.

## Brief summary (TL;DR)

- **Each reference engine caches effective masses over the period its own inertia is treated as constant. Jolt is the only one where that period is also exact; boyko refreshes inertia every substep, so an exact cache must be keyed to the epoch.**
  - **Jolt** is the exact precedent for an epoch rule:
    - it computes `mEffectiveMass` at setup and reuses it for all `mNumVelocitySteps = 10` velocity iterations;
    - positions integrate only after the velocity phase, so inertia cannot change during it;
    - on every position iteration it recomputes both the inertia (from the current rotation) and the effective mass.
  - **Box2D v3** (2D, where inertia is a scalar and does not change with rotation), **Box3D, Rapier, Avian and PhysX TGS** compute the masses once per step. The 3D engines keep them across substeps even though the bodies rotate, which is an approximation.
  - **Bepu** recomputes the mass in every `Solve` call (it has had no "projection" buffer since v2.4), from a world inertia that is updated during substep integration.
  - No engine I found caches per substep epoch.
- **The tree satisfies the epoch premise by construction.**
  - The mass reads only `(dir, ra, rb, inv_mass, inv_inertia)` (`contact.rs:131-148`; `simd.rs:638-683`).
  - `inv_inertia` has exactly two writers: `build_bodies` (`colored.rs:1557-1562`) and `refresh_inertia` (`simd.rs:155`, `:366-377`, called only at `colored.rs:3597-3600`).
  - `inv_mass` is written only by `build_bodies`. Everything that runs inside the sweeps writes only velocities and impulses.
  - With S substeps and R relax iterations there are S+1 epochs, and the number of sweeps that could load is S·R − 1. At the defaults (4, 2; `resources.rs:452-453`) that is **5 sweeps that compute and 7 that load**. Epoch 0 serves one sweep, so its masses are never reused.
- **Caching adds no floating-point operation.** It replaces a recompute with a store and a load of the same f32 bits. The risks are all structural:
  - a stale epoch;
  - a lane or rank mismatch;
  - padding ranks: under L11 a present lane's padding rank has ra = rb = 0 but a real normal and real bodies, so its mass is 1/(mA+mB), not 0;
  - reuse across steps;
  - the floating-point control word (MXCSR) differing between threads.
- **Instruction mix (arith., today's kernel, per rank of 8 points).**
  - The three `effective_mass_x8` calls are 144 `vmulps`, 57 `vaddps`, 36 `vsubps`, 3 `vdivps` and 3 blends. That is about 50 % of the rank's 483 floating-point operations.
  - On Zen 3 they are 147 of the 306 operations issued on the two multiply pipes: 153 cycles per rank today, 79.5 in a sweep that loads the masses.
  - They are not on the velocity dependency chain, which I estimate at about 155 cycles per rank. L12 does not shorten that chain.
  - Loading the masses instead costs 3 ymm loads (96 B) per rank, which is 20–40× cheaper than the recompute in throughput.
  - **Bounds at J, W=1:** at most about 0.26–0.36 ms per step (0.057–0.080 µs per manifold) before L8; at least about −0.02 ms (a small loss).
- **Two in-tree facts need care, and the parity binary has never been censused.**
  - The FMA study's "no prize" (0.98–1.01× after fusing 32 % of floating-point ops) does not predict L12. Fusing removes add-pipe work and lengthens the chain (FMA latency 4 against 3), while multiply-pipe pressure stays the same. L12 removes multiply-pipe work.
  - In the censused build (2026-09-02: codegen-units 16, no LTO) `effective_mass_x8` was a call, not inlined into `solve_color_avx2`. The Rust ABI passes `__m256` by pointer. If the parity build (fat LTO) also keeps the call:
    - L12 removes the call overhead in 7 of 12 sweeps;
    - an `#[inline]` hint alone would remove it in all 12, which confounds L12's A/B;
    - `#[inline(always)]` cannot be combined with `#[target_feature]` on stable Rust.

## Approaches in state-of-the-art engines

### Jolt (master)
- **Approach:** cache the effective mass for exactly the period in which inertia is constant (the velocity phase); recompute it where inertia changes (the position phase).
- **What it stores:** `AxisConstraintPart` holds `mR1PlusUxAxis`, `mR2xAxis`, `mInvI1_R1PlusUxAxis`, `mInvI2_R2xAxis`, `mEffectiveMass` and `mTotalLambda`. `SolveVelocityConstraint` reads `mEffectiveMass` and the stored vectors [4].
- **Setup:** inertia comes from `mp1->GetInverseInertiaForRotation(transform_body1.GetRotation())` [5].
- **Job order per collision step:** SolveVelocityConstraints → PreIntegrateVelocity → IntegrateVelocity → PostIntegrateVelocity → ResolveCCDContacts → SolvePositionConstraints. The velocity loop does not recompute constraint properties [7].
- **Position phase:** "`// Update constraint properties (bodies may have moved)`" followed by `CalculateConstraintProperties(constraint.mInvMass1, inv_i1, r1, …)`, with `inv_i1 = constraint.mInvInertiaScale1 * ioBody1.GetInverseInertia()` [5]. `Body::GetInverseInertia()` returns `GetInverseInertiaForRotation(Mat44::sRotation(mRotation))` [6], so inertia is recomputed from the current rotation on every position iteration.
- **Reuse:** `mNumVelocitySteps = 10`, `mNumPositionSteps = 2` [8].
- **Trade-offs:**
  - Exact for Jolt's phase structure.
  - It also caches I⁻¹(r×axis) for the impulse application. For boyko that is not bit-identical (see Pitfalls).

### Box2D v3
- **Masses:** computed in `b2PrepareContacts_Overflow` / `_Wide`:
  - `kNormal = mA + mB + iA*rnA*rnA + iB*rnB*rnB; normalMass = kNormal > 0 ? 1/kNormal : 0`, and tangentMass the same way;
  - wide fields `normalMass1/2` and `tangentMass1/2`;
  - the solve functions read these fields and never rewrite them [1].
- **Stages:** contacts are prepared once per step. Each substep then runs: integrate velocities, warm start (per colour), solve, integrate positions, relax (`RELAX_ITERATIONS`) [2].
- **Separation per substep:** `ds = dp + (rotate(dqB, rB) − rotate(dqA, rA))`, then `s = baseSeparation + dot(ds, normal)` [1].
- **Trade-off:** in 2D the inertia is a scalar, so it does not change with rotation. Once-per-step masses therefore lose only the anchor rotation, never an inertia change.

### Box3D
- **What prepare stores:** `iA = simA->invInertiaWorld` is copied into the constraint (`contactConstraint->invIA`).
- **Masses:**
  - per point: `kNormal = mA + mB + dot(rnA, iA·rnA) + dot(rnB, iB·rnB)`;
  - per manifold: `tangentMass = b3Invert2(k)`, a 2×2 matrix built at the friction centre (`centerA/B`);
  - per manifold: `twistMass = 1/dot(n, (iA+iB)·n)`.
- **Wide layout:** `b3SymMatrix2W tangentMass`, `b3FloatW twistMass`, and per point `normalMasses`.
- **No recomputation:** no solve, relax or restitution function recomputes a mass or the inertia. Separation is updated per substep from the delta rotations [3].
- **Trade-off:** the 3D world inertia is frozen at step start for all substeps. That is a value approximation.

### Rapier (master)
- **`generate` (once per step):** `projected_mass = simd_inv(force_dir1·(imsum∘force_dir1) + ii_torque_dir1·torque_dir1 + ii_torque_dir2·torque_dir2)`, where `ii_torque_dir1 = poses1.ii.transform_vector(torque_dir1)`. The tangent `r` is computed the same way [9].
- **`update` (per substep):** recomputes only `dist`, `rhs` and `cfm_factor`. `normal_part.r` and `tangent_part.r[j]` are not recomputed [9].

### Avian (Rust, Bevy ecosystem)
- `ContactNormalPart { impulse, total_impulse, effective_mass, softness }`.
- `generate()` takes `inverse_angular_inertia1/2: &SymmetricTensor` and stores `effective_mass: k.recip_or_zero()`. `solve_impulse()` reads `self.effective_mass` [10].
- I did not verify where `generate` runs relative to substeps.

### Bepu v2 (2.4 and later)
- **Recompute, never cache:**
  - changelog: "Constraint type batches no longer have a 'projection' buffer; anything loaded from it is now recalculated on the fly" [11];
  - `PenetrationLimit.Solve` computes `var effectiveMass = effectiveMassCFMScale / (linear + angularA0 + angularB0);` on every call [12];
  - `Contact4Functions.Solve(... in BodyInertiaWide inertiaA ...)` loads no prestep projection [13].
- **World inertia:** it is stored in body memory and gathered (`var offsetInFloats = worldInertia ? 24 : 16;`) [14].
  - `PoseIntegrator` rotates it (`RotateInverseInertia`).
  - `IntegrateBundlesAfterSubstepping` says "There is no need to write world inertia, since the solver is done" [15].
  - My reading is that world inertia is written during the substeps; I did not verify this further.
- **Author's opinion (issue #104):**
  - "The primary determinant of performance in the solver is *convergence per byte memory bandwidth*."
  - Presteps held heavy math "with the expectation that the inverse will be used several times per constraint in the velocity iterations". With one iteration per substep, that reuse disappears [16].

### PhysX 5 TGS (the copy vendored by Qt)
- `constructContactConstraintStep` computes the contact response from `sqrtInvInertia0/1`.
- Each contact stores `raXnI`, `rbXnI`, `velMultiplier`, `separation`, `biasCoefficient` and `targetVelocity` [17].
- I did not verify that the solve loop reuses `velMultiplier` unchanged.

## Comparative table

| aspect | Jolt | Box2D v3 | Box3D | Rapier | Bepu 2.4+ | boyko today |
|---|---|---|---|---|---|---|
| mass computed | at setup (velocity phase); every position iteration | prepare, once per step | prepare, once per step | `generate`, once per step | every `Solve` call | every sweep (12 per step), 3 per point |
| inertia used | rotation at setup; current rotation in position iterations | scalar (2D) | world inertia at step start | `poses.ii` at step start | refreshed per substep | refreshed per substep (5 epochs) |
| inertia changes while the mass is reused? | no | not applicable | yes (substeps rotate) | yes | not applicable (no reuse) | would be no if keyed to the epoch |
| uses per computation | 10 velocity steps | every sweep of the step | every sweep of the step | every substep | 1 | 1 today; 1–3 with the epoch cache (12/5 = 2.4 on average) |
| also cached | r×axis, I⁻¹(r×axis) | anchors, base separation | invI, 2×2 tangent mass, twist mass | parts of rhs | nothing | nothing |
| exact with respect to the engine's own inertia | yes | yes (2D) | no | no | yes | yes |

## boyko tree facts: the code paths and data L12 touches

**Epoch schedule** (substep loop `colored.rs:3547-3617`: gravity → warm apply → biased sweep → position integrate + `refresh_inertia` → relax × R):

| epoch | inertia comes from | sweeps served at the defaults |
|---|---|---|
| 0 | `build_bodies` (`:1542-1564`, copies `BodyState.inv_inertia`) | biased sweep of substep 0 (`:3568-3580`) only |
| 1–3 | refresh after substep s−1 (`:3597-3600`) | relax ×2 of substep s−1 (`:3604-3616`) + biased sweep of substep s |
| 4 | refresh after substep 3 | relax ×2 of substep 3 + restitution (`:3619-3623`; normal mass only, `:3162-3166`) |

The general formula: loads / sweeps = (S·R − 1)/(S(1+R)). That is 7/12 at (4, 2), 3/8 at (4, 1), and 0 at R = 0.

**Consumers of `effective_mass`:**
- the scalar oracle `solve_color`:
  - the normal mass at `:2084-2088`, before the normal impulse is applied;
  - the tangent masses at `:2132-2141`, after it. The value is the same, because the apply changes only velocities;
- the AVX2 kernel at `:2485`, `:2540-2541`. `simd.rs:632-634` documents the three direction calls as independent ("called three times per rank");
- restitution at `:3162-3166`;
- the test oracle `cone_probe` in `colored_tests.rs:1778`, `:1807-1808`;
- `SoftStepSolver` (`soft_step.rs:640/687/696/790`, refresh at `:1027`). This is a separate solver and not the default.

**Writers of the inputs** (a grep of `solver/`, tests excluded):
- `inv_inertia`: `simd.rs:155` (scalar), `simd.rs:366-377` (AVX2; it blends only `inv_mass != 0` lanes), and `colored.rs:1557-1562` (build).
- `inv_mass`: `colored.rs:1557-1562` only.
- Everything else writes only velocities:
  - `apply_impulse` (`contact.rs:108-111`);
  - `apply_impulse_blend_x8`, which returns only lin/ang (`simd.rs:697-740`);
  - the kernel's body scatter (`colored.rs:2636-2649`);
  - gravity (`simd.rs:198-202`);
  - warm apply (`colored.rs:1881`);
  - the frozen-row restore, after the loop (`:3645-3651`).
- The sentinel `IMMOVABLE_AT_REST` (`soft_step.rs:145`) has `inv_mass` 0 and zero inertia.

**Ownership and synchronisation precedent:**
- The impulses ni/ti1/ti2 are already written in sweep k and read in sweep k+1, per active lane, by whoever owns the cohort (`colored.rs:2611-2621`).
- SIMD chunks are cut on cohort boundaries (`:2998-3017`, `COHORT = 8` at `:332`). Scalar chunks are cut on group boundaries.
- The only ordering between sweeps is the scope join. A per-rank mass row has the same writers and the same ordering, so it needs no new atomics.

**L11 base layout** (L11 design `:116-131`, `:177-191`, `:216`):
- A 320 B `CohortHead` (per-lane n and t1, friction, body ids, width) and a 320 B `RankBlock` (ra, rb, sep, ni, ti1, ti2 × 8 lanes), both 64-aligned.
- At J: about 2,320 rank blocks and about 571 cohorts; about 0.92 MB of kernel bytes.
- Padding lanes and ranks are zero, and the impulse store is `blendv(old, new, active)`.
- The kernel still gathers both bodies' inertia per cohort, because the four impulse applies need it (`colored.rs:2352-2363`, `:2661-2684`). L12 does not remove that gather.

**Scratch ids:** a new per-rank column must join the solver cohort's contiguous id run. The run's width must stay ≤ `POOL_STAGGER_LINES` (`scratch_ids.rs:197-202`). The comment at `:124-185` explains why: the last time columns shared a cache-set slot, the solver regressed about 40 %. L11 reduces `CONTACT_COLUMN_COUNT` from 31 to 12 (`:69`).

**Configuration:**
- `simd_solve: true` (`resources.rs:480`).
- `parallel_solve: false` here (`:501`), `true` in `D:/wt/l5np` (`resources.rs:507`).
- `effective_mass_x8`, `apply_impulse_blend_x8` and `pointvel_x8` carry no `#[inline]` (`simd.rs:635-646`, `:694-705`, `:607-613`).

**Profiling zones:** `PHYS_PASS_BIASED`, `PHYS_PASS_RELAX` and `PHYS_COLOR_WIDE` (`profiling.rs:26-30`, `:79-83`). Under a cache the biased zone would hold 1 compute sweep and 3 load sweeps, and the relax zone 4 of each. So the existing zones cannot separate compute sweeps from load sweeps.

**P0 spans (J-B, simd on; `p0b/raw/window/driver_reduce/reduction.json:905-983`):**

| W | pass_biased (4 sweeps) | pass_relax (8 sweeps) | wide colours | per sweep | wide colours per manifold |
|---|---|---|---|---|---|
| 1 | 1.212 ms | 2.384 ms | 3.576 ms | 0.303 / 0.298 ms | 0.790 µs |
| 8 | 0.342 ms | 0.638 ms | 0.957 ms | 0.086 / 0.080 ms | 0.211 µs |

- Per step: 4,524.2 manifolds, 16,887.8 points, 9.11 wide colours, 109.32 waves.
- Per rank block per sweep at W=1: 0.298 ms / 2,320 = 128 ns, which is about 420–590 cycles at 3.3–4.6 GHz. The P0 receipt names the CPU (Ryzen 9 5900HS) but not the clock.

**In-tree opcode census** (`docs/physics/FMA-DETERMINISM-MEASUREMENTS.md:121-129`; 2026-09-02, gnu toolchain, codegen-units 16, no LTO — before `[profile.release] lto = "fat"`, `Cargo.toml:111-112`):
- `solve_color_avx2`: 1,710 instructions: 127 `vmulps`, 66 `vaddps`, 42 `vsubps`, 1 `vsqrtps`, **1 `vdivps`**, 2 `vmaxps`.
- `effective_mass_x8` is a separate symbol: 48 / 19 / 12 and 1 `vdivps`. `apply_impulse_blend_x8` is 18 / 12 / 3; `pointvel_x8` is 6 / 3 / 3.
- The kernel's one `vdivps` is the friction scale (`colored.rs:2568`). So in that build the three mass computations were **out-of-line calls**, while the scalar `solve_color` inlined them (it holds 4 `vdivss`).
- The transcription study measured 453 vector floating-point ops, 1,309 loop instructions and 51 spill stores / 58 reloads per rank (`:322-324`). It also concluded "latency-bound on the serial Gauss–Seidel chain" (`:335-340`).

## Arithmetic for the build-if (arith.; Zen 3 is the P0 machine)

**Instruction facts:**
- `vmulps`/`vaddps` ymm: latency 3, throughput 0.5 cycles; FMA latency 4 (uops.info, cited at `FMA-DETERMINISM-MEASUREMENTS.md:24-27`).
- `vdivps` ymm: latency ≤11, throughput 5.0 [18]. `vsqrtps` ymm: latency ≤14, throughput 5.0 [19].
- `vblendvps` ymm: latency 1, throughput 0.5, runs on the multiply pipes (FP0/FP1) [20].
- Zen 3 has 2 multiply and 2 add floating-point pipes. It does 3 loads or 2 stores per cycle, but only 2 loads and 1 store for 256-bit values [22].
- ROB 256 entries; L2 512 KB per core; L3 transfers 32 B per cycle [21].

**Per-rank operation mix** (from the source at `colored.rs:2427-2621` plus the helpers; the per-helper counts match the census):

| | 3 masses | rest of the rank | today | load sweep |
|---|---|---|---|---|
| mul (FP0/FP1) | 144 | 133 | 277 | 133 |
| blend (FP0/FP1) | 3 | 26 | 29 | 26 |
| add + sub (FP2/FP3) | 93 | 108 | 201 | 108 |
| div + sqrt | 3 | 2 | 5 | 2 |
| cycles on the multiply pipes | 73.5 | 79.5 | **153** | **79.5** |
| cycles on the add pipes | 46.5 | 54 | 100.5 | 54 |
| divider cycles | 15 | 10 | 25 | 10 |

The table leaves out about 9 compare / and / max operations per rank, which I did not assign to pipes.

**Velocity chain per rank (estimate):**
- The normal part is about 62 cycles: point velocity → dv → vn → d_lambda → clamp → impulse → apply.
- The friction part is about 93 cycles: … → sqrt (≤14) → div (≤11) → … → apply.
- **Total about 155 cycles.** The masses are not on this chain.
- The normal mass takes about 48 cycles from its inputs, but is needed about 24 cycles into the rank. So it must start during the previous rank's tail. More than 300 instructions separate the two, so the 256-entry ROB decides whether it is hidden.
- Both floors (about 155 cycles) are far below the measured 420–590 cycles. Today's time is therefore dominated by what L11 removes (the per-rank scalar gather and spills). What limits the kernel after L11 is unmeasured; the L11 review's OQ2 asks for exactly that.

**Bounds at J, W=1** (2,320 blocks, 7 load sweeps):

| quantity | value |
|---|---|
| multiply-pipe relief | 73.5 × 2,320 × 7 = **1.19 M cycles = 0.26–0.36 ms per step = 0.057–0.080 µs per manifold** |
| … as a share | 7–10 % of today's wide colours; 8–13 % of L11's predicted 2.68–3.22 ms |
| out-of-line call overhead (only if the parity build does not inline) | 3 calls per rank, each passing up to ~29 ymm values by pointer [25][26]; not measured; an inlining hint alone recovers part of it |
| cache traffic | 223 KB per sweep; 4 store sweeps + 7 load sweeps = 2.45 MB per step = 76.6k cycles at 32 B/cycle, so **≤ 17–23 µs per step** even if fully exposed |
| lower bound | about −0.02 ms (recompute fully hidden, cache traffic exposed) |
| per point | recompute ≈ 9.2 cycles on the multiply pipes; load: 12 B ≈ 0.19 load-port cycles + 0.375 cycles of L3 bandwidth |
| after L8, if Box3D-shaped | mass rows per sweep 50,664 → 30,460–34,984 (×0.60–0.69); vector calls 6,960 → 4,033–4,604; cached bytes 223 → about 147 KB; bound becomes **0.15–0.25 ms** |
| W=8 (parallel solve on) | bound / (8 · E(8) = 0.467) = ≤ 0.07–0.10 ms, which is ≤ 2.5 % of the projected T(8) of 3.9–4.7 ms: below the 5 % rule |

**Gate resolution:**
- Stage spans are tight: cfg-A's P(1) range is 0.13 % over K=6 (ANALYSIS §2). With σ ≈ range / 2.5, a per-stage, per-contact gate on the wide-colour span resolves about 0.1 %.
- End to end at W=1 the SE bar is 2.0 % at K=6 (ANALYSIS §9), about 0.1–0.2 ms on a T(1) of 5–10 ms.
- The 0.13 % is cfg-A's. The J-B zone spread is not reduced in ANALYSIS; the raw data is in `p0b/raw/window/runs.jsonl`.

## Bit identity: the ingredients, with sources

1. **Same inputs.** Within an epoch the inputs are bitwise constant (the writers are listed above).
2. **Same operations.**
   - Rust's basic float ops "exactly match IEEE 754-2008 (with roundTiesToEven … default exception handling … without abruptUnderflow/flush-to-zero)" [23].
   - rustc does not contract `a*b + c` (measured in-tree, `FMA-DETERMINISM.md:89-95`). The census tests enforce it: `colored_has_no_fma_or_approx_callsites` (`colored_tests.rs:3221`) and `solver_simd_has_no_fma_or_approx_callsites`.
   - The msvc target also gets `-C target-cpu=x86-64-v3` (`.cargo/config.toml:135-136`).
   - The binary-level census was taken on the gnu toolchain; I found no msvc disassembly census in `docs/physics`. MSVC's `/fp:contract` does not apply, because rustc code is generated by LLVM.
3. **Exact storage.** An f32 moved through memory keeps its bits; SSE/AVX has no extended precision.
4. **Control word.**
   - Windows x64 ABI: MXCSR bits 6–15 are nonvolatile and hold standard values at program start [27].
   - The L5 ruling O2 asserts that pool workers share one MXCSR (`levers/00-RULINGS.md:20-21`).
   - What the cache adds is a new cross-thread path: computed on worker X in sweep k, used on worker Y in sweep k+1.
   - This matters only if MXCSR differs between threads, and that would already break the {1,N} identity (`FMA-DETERMINISM.md:131-144`).
5. **Scalar against SIMD.**
   - `effective_mass` equals `effective_mass_x8` per lane, under the gates `simd_solve_bits_match_scalar` (`colored_tests.rs:1454`), `:1626`, `degenerate_lane_differential_test_1d` (`:2009`) and `:2101`.
   - `simd_solve` is fixed for the whole step (`colored.rs:3514`), so a mass written by one path is never read by the other. Either path can cache or not cache without changing any bit.
6. **Folding.**
   - The biased sweep computes `(−1·mass_coeff)·m` (`:2503`); relax computes `(−1·m)·vn` (`:2510`).
   - So the quantity both sweep kinds can share is m itself.
   - Caching a product with a per-step constant stays bit-identical only if the operands and their order are unchanged.

## Pitfalls and mistakes

- **Padding ranks.** A present lane's padding rank has a real normal and real bodies with ra = rb = 0, so its mass is 1/(mA+mB). An unblended store therefore breaks L11 G3 (padding bytes are zero) unless it follows the impulse precedent `blendv(old, new, active)`.
- **Absent lanes** give k = 0 → 0 and are harmless. They must still read no body row (L11 review O2).
- **Cross-step reuse.** Epoch 0's masses are never reused. A mutation "epoch 0 loads" would read the previous step's epoch-4 row, which is laid out for the previous step's cohorts.
- **The stale-epoch mutation can pass without testing anything.** A body whose rotation bits do not change between refreshes (identity rotation, zero ω, frozen) has bitwise-equal inertia and masses in consecutive epochs, so a stale load changes nothing. The gate scene needs rotating bodies, plus a non-vacuity count of lanes whose epoch-e mass bits differ from epoch e−1's.
- **Once-per-step masses** (Box2D, Box3D, Rapier, Avian) change values in boyko, because boyko refreshes inertia per substep (L11 D8, `:146`).
- **Jolt's cached I⁻¹(r×axis) does not transfer.** boyko applies I⁻¹·(r×(dir·λ)) (`contact.rs:110`; `simd.rs:717-720`). Jolt's λ·(I⁻¹(r×dir)) rounds differently, so it is a value change.
- **A cheaper formula is also a value change.** boyko's angular term, dir·((I⁻¹(r×dir))×r) (`contact.rs:139-142`), costs 38 operations. The rn·(I⁻¹·rn) form used by Box3D and Avian costs 29 and is equal only mathematically.
- **Inlining confound.** If the baseline makes out-of-line calls, an L12 A/B also measures the removal of call overhead. An `#[inline]` hint captures that in all 12 sweeps. On stable Rust "`#[inline(always)]` may not be used with a `target_feature` attribute" [24]; there is a nightly tracking issue [25].
- **Profile.** The `bench` profile (lto = false, codegen-units 1) generates different code from `parity` (fat LTO).
- **L8 first.** L8 changes which masses exist and may change the angular formula. Arithmetic done on today's kernel would have to be redone.
- **Register pressure.** `a_ii` and `b_ii` alone need 18 ymm registers, more than the 16 architectural ones. The transcription already shows 51 spill stores per rank. A cache changes the spill pattern, so "not slower" has to be measured.
- **No cross-step state.** The cache holds nothing from one step to the next, so it creates no replay, save or row-identity hazard, unlike the L9/L10/L11 records. It needs a solver-owned `ScratchColumn`, not a `Vec`.
- **Correctness bounds.** Because the lever is bit-identical, A7-R1, freeze steps, the sleep suites and incline stick/slip cannot move. Any movement is a defect.
- **Stability of the rejected once-per-step alternative:** I found no reliable quantitative information. The only statement is qualitative: the XPBD rigid-body abstract says traditional solvers freeze "constraint directions for multiple iterations" [28].

## Relevant academic works
- Macklin et al., "Small Steps in Physics Simulation", SCA 2019 [29]. Many substeps with one iteration each beat one step with many iterations. That lowers how many times any per-step setup data is reused, which is the premise behind Bepu's recompute choice.
- Müller et al., "Detailed Rigid Body Simulation with Extended Position Based Dynamics", CGF 2020 [28]. Their method "always works with the most recent constraint directions". This is context for refreshing inertia per substep, not about caching.
- Catto, "Iterative Dynamics with Temporal Coherence", GDC 2005. I could not extract the passage on precomputing the effective mass (the PDF would not render), so there is no reliable information on that point.

## Applicability to boyko-engine
- **Usable directly:** Jolt's rule — cache for exactly the interval in which inertia is constant, and recompute when it changes.
- **Usable with adaptation:**
  - The Box2D/Box3D "prepare" stage. Epoch-0 masses depend only on build-time data (ra, rb, n, t1, and the body state at build), so they could be computed in L11's fill (P-c). That moves one compute pass out of the kernel without removing one.
  - Box3D's per-manifold 2×2 tangent mass and twist mass are the likely shape of L8.
- **Does not fit (all change values):** once-per-step masses, Jolt's cached I⁻¹(r×axis), and the cheaper algebraic form.

## Open questions for the architect
1. Is `effective_mass_x8` inlined in the **parity** binary's `solve_color_avx2`? The number of `vdivps` in the kernel symbol answers it (4 means inlined, 1 means calls), and it determines what an L12 A/B actually measures.
2. Which masses does L8 define (normal per point; per manifold a 2×2 tangent plus twist?), and does it change the angular formula? L12's arithmetic should be redone on L8's kernel.
3. Is the post-L11 kernel limited by the multiply pipes or by the chain? The L11 review's OQ2 already asks for that measurement. Does L12 wait for it?
4. Where do the mass rows live? Inside `RankBlock` (320 → 416 B, or 448 B = 7 lines when padded or holding a twist row), or in a separate 96/128 B-per-rank column with its own stagger id?
5. Should the scalar path cache or keep recomputing? The bits are identical either way.
6. Should restitution load epoch 4's normal mass or recompute it? Its span is 0.008 ms.
7. Which gate claims the gain: the per-stage wide-colour span per manifold at W=1 (resolves about 0.1 %) or end-to-end T(1) (2.0 % at K=6)? W=8 cannot clear the 5 % rule by this arithmetic. L9's ruling set the precedent of gating on W=1 and per contact (`levers/00-RULINGS.md:115-116`).

## Sources
[1] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c — prepare-time `normalMass`/`tangentMass`, no recompute, per-substep separation
[2] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver.c — stage list and substep loop
[3] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact_solver.c — `invIA = invInertiaWorld`, normal/tangent/twist masses once per step
[4] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ConstraintPart/AxisConstraintPart.h — cached `mEffectiveMass` and `mInvI1_R1PlusUxAxis`
[5] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Constraints/ContactConstraintManager.cpp — setup inertia; recompute per position iteration
[6] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Body/Body.inl — `GetInverseInertia()` from the current rotation
[7] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSystem.cpp — job order: velocity phase, then integration, then position phase
[8] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/PhysicsSettings.h — 10 velocity steps, 2 position steps
[9] https://raw.githubusercontent.com/dimforge/rapier/master/src/dynamics/solver/contact_constraint/contact_with_coulomb_friction.rs — `generate` versus `update`
[10] https://raw.githubusercontent.com/avianphysics/avian/main/src/dynamics/solver/contact/normal_part.rs — stored `effective_mass`
[11] https://raw.githubusercontent.com/bepu/bepuphysics2/master/Documentation/changelog.md — projection buffer removed in 2.4
[12] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/PenetrationLimit.cs — mass recomputed in `Solve`
[13] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs — `Solve`/`WarmStart` signatures
[14] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Bodies_GatherScatter.cs — world inertia gathered from body memory
[15] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/PoseIntegrator.cs — `RotateInverseInertia`, substep integration
[16] https://github.com/bepu/bepuphysics2/issues/104 — the author's bandwidth argument (opinion)
[17] https://codebrowser.dev/qt6/qtquick3dphysics/src/3rdparty/PhysX/source/lowleveldynamics/src/DyTGSContactPrep.cpp.html — TGS prep fields
[18] https://www.uops.info/html-instr/VDIVPS_YMM_YMM_YMM.html
[19] https://www.uops.info/html-instr/VSQRTPS_YMM_YMM.html
[20] https://www.uops.info/html-instr/VBLENDVPS_YMM_YMM_YMM_YMM.html
[21] https://en.wikipedia.org/wiki/Zen_3 — ROB 256, L2 512 KB, L3 at 32 B per cycle
[22] https://semiaccurate.com/2020/11/07/a-long-look-at-amds-zen-3-core-and-chips/ — 2 multiply + 2 add pipes; load/store rates
[23] https://rust-lang.github.io/rfcs/3514-float-semantics.html — IEEE semantics, no contraction, no flush-to-zero
[24] https://doc.rust-lang.org/stable/reference/attributes/codegen.html — no `#[inline(always)]` with `target_feature`
[25] https://github.com/rust-lang/rust/issues/145574 — `target_feature_inline_always` (nightly)
[26] https://github.com/rust-lang/rust/issues/116558 — "The Rust ABI passes `__m256` by-pointer"
[27] https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention?view=msvc-170 — MXCSR volatility
[28] https://dl.acm.org/doi/10.1111/cgf.14105 — Müller et al. 2020
[29] https://dl.acm.org/doi/abs/10.1145/3309486.3340247 — Macklin et al. 2019
[30] https://box2d.org/posts/2024/02/solver2d/ — Soft Step: substepping plus relaxation

Files read:
- D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/soft_step.rs
- D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs
- D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs
- D:/wt/joltab/crates/boyko_physics/src/profiling.rs
- D:/wt/joltab/crates/boyko_physics/src/resources.rs
- D:/wt/l5np/crates/boyko_physics/src/resources.rs
- D:/wt/joltab/Cargo.toml
- D:/wt/joltab/.cargo/config.toml
- D:/wt/joltab/docs/physics/FMA-DETERMINISM.md
- D:/wt/joltab/docs/physics/FMA-DETERMINISM-MEASUREMENTS.md
- D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md
- D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/p0b/raw/window/driver_reduce/reduction.json
- D:/wt/joltab/docs/physics/perf-campaign/00-RULINGS.md
- D:/wt/joltab/docs/physics/perf-campaign/01-PLAN-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/01-RESEARCH.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/03-REVIEW-OF-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md
- D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/03-REVIEW-OF-REV1.md
