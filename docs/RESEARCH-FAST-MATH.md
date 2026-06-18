# Research: Fast Math for boyko-engine (physics + SDF + render)

Status: **research report** (forward-looking; informs a future math-perf phase). Not yet implemented.
Tailored to our system: Rust 2024, AVX2 baseline (AVX-512 via cfg), DoD/SoA columns, **bit-deterministic
physics path** (`solver_is_deterministic`/IM-2). Companion to `docs/PERF-DIRECTIONS.md` (CPU-D2/D3/D6).

## The determinism boundary (the load-bearing constraint)

Two SEPARATE hazards forbid fast math in the physics path; both must be respected:
1. **Float reassociation** (`algebraic_*` intrinsics, autovec of float reductions, FMA contraction) reorders
   adds → non-deterministic *even within one run*.
2. **`rsqrtps`/`rcpps` approximations are implementation-defined and differ between Intel and AMD silicon** —
   not just non-portable, *wrong-by-vendor* (O'Callahan/rr 2021). **`sqrtss`/`sqrtps` IS IEEE-754-specified and
   bit-identical everywhere**; `divps` (exact reciprocal) likewise.

→ The physics solver + `boyko_sdf_math` keep **exact `sqrt` + exact `1/x`, no approximations, no `algebraic_*`,
no FMA contraction**. The speedup there comes ONLY from *width* (SIMD lanes), never from *approximation*.
Render / broadphase / visual-interpolation paths (outside the determinism gate + not GPU-golden oracles) MAY
use fast approximations.

## Key findings (state of the art)

- **The big win is NOT AoS-SIMD of a single `Vec3` — it is SoA-batched ("8 bodies/contacts per AVX2 lane").**
  mathbench: single-op AoS-SIMD is near-worthless for heavy ops (Mat4×Mat4 batch = **1.04×**), while SoA-wide is
  **3–9×** (normalize-batch 3.24×, ray-sphere 9.23×, Euler integrate 1.45×). A 12-B `Vec3` in a 16-B register
  wastes a lane.
- **Box2D v3 (Erin Catto, "SIMD Matters" 2024) is the DIRECT precedent for our `SoftStepSolver`** (TGS
  sequential-impulse): SSE2 ~2× scalar, AVX2 ~2.3× scalar on the contact solve — but **the prerequisite is graph
  coloring** (a color holds each body at most once → race-free to SIMD-batch 4/8 contacts/lane). Our
  `solve_velocities` is sequential Gauss-Seidel; a SIMD block sharing a body would corrupt it. Coloring is a
  **solver-architecture change** (CPU-D6 islands), not a math-lib swap.
- **glam guarantees "bit-for-bit identical results on all platforms" by default** — its SSE2 path uses only
  IEEE-exact ops (SSE2 == scalar). Proof that *width-only* SIMD is determinism-safe. Its non-deterministic
  speedups are quarantined behind an opt-in `fast-math` feature ("intermediate libraries should not use this").
- **Our scalar `Vec3`/`Quat`/`Mat3` should largely STAY scalar** (AoS-SIMD of a single vector is the 1.04× case;
  the autovectorizer already handles per-row scalar ops, and scalar is trivially deterministic). The lever is
  rewriting the *hot batched loops* (integrate, inertia refresh, constraint prep) as **SoA chunk kernels**.

## Recommendations for boyko-engine

**Keep scalar (do NOT SIMD-ize):** `Vec3`/`Quat`/`Mat3` single-vector ops in `math.rs`;
`boyko_sdf_math` `v_*`/`sd_*`/`smin`/`combine`/`sdf_edit_list` (**determinism-CRITICAL: the CPU/GPU golden
source of truth — no fast math EVER; a reordered FMA breaks the ±2/255 goldens**).

**Where width-only SIMD IS worth it (determinism-SAFE):**
- **Batched quaternion integrate + normalize** (`physics_integrate`): the textbook batched case (normalize-batch
  3.24×); SoA over the `RigidBody` column, exact `sqrtps`/`divps`, fixed lane-reduction order. Also unblocks the
  CPU-D2 "autovec-through-closure (likely scalar)" problem. **Highest determinism-safe ratio.**
- **Batched `R·I⁻¹·Rᵀ` inertia refresh** (`refresh_inertia`, runs `substeps×`/step): SoA-batch 8 bodies.
- **Batched SoA SDF evaluator `sdf_edit_list_x8`** (for W5 CPU SDF collision narrowphase): a SEPARATE fn —
  the scalar `sdf_edit_list` STAYS the GPU golden oracle.
- **The constraint solve (`solve_velocities`)** — the biggest prize (Box2D's 2–2.3×) but **GATED on graph
  coloring** (CPU-D6). Determinism needs pinned color order + intra-color order + lane-reduction order (we are
  STRICTER than Box2D, which doesn't discuss determinism).

**Render / non-deterministic path (fast approximations OK):** render-side normalize/lighting normals via
`rsqrtps`+Newton (~0.175% error); broadphase distance compares via `algebraic_*` (output is a candidate set
re-checked by exact narrowphase); the existing GPU-mirror `mix(prev,pos,alpha)` interpolation. **Never share
that code with `boyko_sdf_math`'s `v_normalize` (golden oracle).**

**Rust SIMD approach:** primary = `core::arch` AVX2 behind `cfg(target_feature)` + a scalar fallback oracle +
a differential proptest asserting `simd_bits == scalar_bits` (bit-for-bit, not epsilon) for deterministic
kernels — our proven house template (CPU-D3 `bitset_intersects_avx2`). Secondary = nightly `portable_simd` for
benches/non-deterministic only (its `reduce_*` lane order is UNSPECIFIED → never in a deterministic reduction;
hand-write horizontal reductions). `algebraic_*` = render/broadphase only, feature-gated.

## Prioritized, sequenced changes (effort/impact/determinism)

1. **Render fast-normalize + `algebraic_*` broadphase kernels** — S effort, M impact, SAFE (outside gate). Lowest risk; establishes the "fast-math quarantined to render" boundary.
2. **Batched quaternion integrate + normalize chunk kernel** — M, M-H, SAFE. Highest determinism-safe ratio.
3. **Batched `R·I⁻¹·Rᵀ` inertia refresh** — M, M, SAFE. Pairs with #2.
4. **Batched SoA `sdf_edit_list_x8`** — M-L, M (once W5 SDF physics exists), SAFE (scalar stays oracle).
5. **Graph-coloring / constraint islands** (CPU-D6 prerequisite) — H, enabler. Define fixed color order.
6. **SIMD-batched contact solve** — H (needs #5), H impact (Box2D 2–2.3×), SAFE only with pinned orders + an Intel-vs-AMD bit-diff added to `solver_is_deterministic`. The headline win, but last.

All of 2/3/6 touch hot kernels behind the 0%-gate → they must be opt-in `_simd` chunk kernels over slices, never a rewrite of the generic path; scalar path stays byte-identical. No blind `#[inline(always)]` (principle 7).

## Open questions for the architect
- Determinism verification scope: add an Intel-vs-AMD bit-diff to `solver_is_deterministic`, or assert at the kernel level (differential proptest `simd==scalar`)?
- FMA contraction: forbid `mul_add` in the deterministic path entirely (use separate mul+add), accepting the perf cost, to stay bit-reproducible? (Recommended yes.)
- Coloring vs the current `(min,max)` manifold order (D4): coloring reorders the sweep → changes the converged float result (different but valid). Accept a one-time determinism-baseline reset when the colored solver lands?
- AVX-512 path for the solver (16 contacts/lane) vs AVX2 (8) given downclocking?

## Sources
- Catto, "SIMD Matters" (Box2D 2024) — https://box2d.org/posts/2024/08/simd-matters/ — the direct solver precedent (graph-coloring + 4/8-wide SoA; SSE2 2×, AVX2 2.3×).
- mathbench-rs — https://github.com/bitshifter/mathbench-rs — wide-vs-scalar numbers.
- glam-rs — https://github.com/bitshifter/glam-rs — "bit-for-bit identical on all platforms" default; `fast-math` opt-in.
- ultraviolet — https://github.com/fu5ha/ultraviolet — AoSoA `Vec3x8`.
- f32 `algebraic_*` docs — https://doc.rust-lang.org/std/primitive.f32.html — "same inputs may produce different results even within a single run."
- O'Callahan, RSQRTSS Intel-vs-AMD divergence (2021) — https://robert.ocallahan.org/2021/09/rr-trace-portability-diverging-behavior.html.
- arXiv:2408.05148 (float non-associativity / reproducibility); arXiv:1802.06302 (rsqrt+Newton error bounds).
- Davidoff, "State of SIMD in Rust 2025" — https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d.
- Unity.Mathematics + Burst best-practices — SoA "process 4 vectors at once"; FloatMode Fast/Deterministic.
