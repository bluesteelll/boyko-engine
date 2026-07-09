# Research: Soft-Body Simulation for boyko-engine

Status: **research report** (forward-looking; informs a future soft-body phase, after P2 rigid + SDF land). Not
yet implemented. Tailored to our system: in-house CPU TGS-Soft rigid solver (P2), analytic SDF field
(`boyko_sdf_math`, the collision source of truth), Chase-Lev work-stealing threadpool, GPU reserved for large-N.
Owner decision: **physics FULLY in-house (no Rapier/Jolt/parry FFI).**

## The recommendation (TL;DR)

**Pick: XPBD position-based soft bodies (distance + volume constraints), run as a SEPARATE position-level pass
AFTER the velocity-level TGS-Soft rigid solve — NOT a single unified solver.** TGS-Soft (velocity-level,
warm-started accumulated impulses, restitution pass) and XPBD (position-level, per-constraint Lagrange
multipliers) speak different currencies; fusing them discards exactly the machinery P2 shipped. This is what
**Jolt** does (soft bodies in a separate `SoftBodyMotionProperties` pass that collides soft↔rigid, not a unified
constraint graph) and what even SOTA XPBD soft work does (couples to a separate rigid solver).

**The SDF is our biggest differentiator + the cleanest collision path.** A soft body is N particles; per particle
call our existing `sdf_edit_list(p)` (signed distance = penetration when negative) + `sdf_edit_list_normal(p)`
(contact normal) — exactly Jolt's per-vertex `mCollisionPlane.SignedDistance(v.mPosition)` model, validated by
Macklin et al.'s SDF-contact paper for "cloth, rigid bodies, deformable solids." O(edits)/particle, branch-light,
SIMD-friendly, zero readback, reuses the CPU/GPU bit-exact source of truth.

**Substepping is the convergence lever and we already do it** (Macklin "Small Steps" 2019: n substeps × 1 iter >
1 step × n iters). Soft particles slot into the SAME `for _ in 0..substeps` loop already in `soft_step.rs`.

**CPU on the Chase-Lev pool up to ~low-thousands of particles; GPU above** (the §3.4 brick-atlas escalation via
`GpuSystem`, zero readback). Determinism on the pool requires **graph coloring** (constraints in a color share no
particle → race-free + order-independent within a color), the standard parallel-PBD technique.

## Two real conflicts to flag
1. XPBD's standard solve is **Gauss-Seidel (sequential, order-dependent)** — to parallelize deterministically on
   the pool, switch to **colored Gauss-Seidel** (fixed color partition + fixed in-color order), NOT naive Jacobi
   (poor convergence + changes results). Or accept single-threaded soft per body.
2. Self-collision needs **spatial hashing**, whose determinism depends on a fixed bucket/probe/emission order
   (the same discipline as the warm-start table: rebuilt each frame, no tombstones, order-independent occupancy).

## The unification that DOES fit: one constraint PROJECTOR, not one solver
XPBD compliant constraint: `Δλ = -C(x) / (Σwᵢ|∇Cᵢ|² + α̃)`, `Δx = wᵢ∇CᵢΔλ`, with `α̃ = α/Δt²` (α = 1/stiffness).
**α=0 ⇒ pure PBD (rigid constraint)** — so the SAME projector spans cloth (high α) → soft solids (low α) →
near-rigid attachment (α≈0). Distance + volume constraints are the minimum soft-solid kit (volume `C = 6(V−V_rest)`
via the scalar triple product = Jolt's `mSixRestVolume`; bend is cloth-only polish).

## What to reject
- **The unified particle solver (NVIDIA FleX pole)** — represents rigid bodies as low-fidelity shape-matched
  particle clusters → would DOWNGRADE the 6-DOF inertia-tensor rigid solver P2 shipped. Reject for the rigid side
  (the cluster representation is still a useful shape-matching reference).
- **Pure Jacobi parallelism** — poor convergence; use colored Gauss-Seidel.
- **External FFI** — barred by the fully-in-house decision; none needed (XPBD + SDF is small self-contained `core` math).

## Phased roadmap (each: dev→review→tester(Miri+proptest+determinism)→commit; SP1–SP3 = ZERO new unsafe)
- **SP1 — single soft body, single-threaded, SDF-only collision** (minimal slice). New `boyko_physics::soft`:
  `SoftBody { positions, prev_positions, inv_masses, distance_constraints, volume_constraints }` SoA, preallocated.
  Substep loop reusing `h`: predict → project distance → project volume → collide each particle vs `SdfField` via
  `sdf_edit_list`/`_normal` → velocity update. Gate: dropped soft cube rests on an SDF box without exploding;
  `soft_is_deterministic`; Miri-green; rest volume preserved.
- **SP2 — soft↔rigid coupling.** Soft runs after rigid in the substep; rigid bodies are kinematic colliders for
  particles; reaction impulse fed back via `BodyEffective::apply_impulse`. Gate: soft body tips a dynamic plank;
  rigid `solver_is_deterministic` still passes; static body bit-identical.
- **SP3 — self-collision via spatial hash** (deterministic dense hash, fixed order, no tombstones → neighbor pairs
  → short-range distance constraints). Gate: folded sheet doesn't interpenetrate; determinism; no per-step alloc.
- **SP4 — parallel soft on the Chase-Lev pool (colored Gauss-Seidel).** Spawn-time graph coloring (topology is
  static for a fixed mesh); parallel-dispatch per color, fixed order. Gate: bit-identical to single-threaded
  (the determinism oracle); criterion scaling at ~thousands of particles; no atomics in the inner solve.
- **SP5 — GPU escalation (§3.4 brick-atlas scale, deferred).** >~10k-particle bodies → compute soft solve via
  `GpuSystem`, zero readback, SDF collision reusing the GPU `sdf_editlist.hlsl` (bit-exact to `boyko_sdf_math`).
  Break-even MEASURED (criterion/golden oracle), not assumed.

**CPU/GPU seam:** a `SoftSolver` trait with CPU (SP1–SP4) + future GPU (SP5) impls, selected per-body by a
residency rule (analogous to MEM-D1: opt-in per body, never global) — below the measured break-even → CPU on the
pool; above → GPU-resident zero-readback. Exactly the PERF-DIRECTIONS principle-3 split.

## Open questions for the architect
- Coupling granularity: soft particles couple to dynamic rigid via (a) the SDF field only (if rigid bodies are
  also edit-list primitives) or (b) a separate particle-vs-rigid-shape narrowphase? (a) cheaper/one-path; (b) general.
- Topology source: tetrahedralized mesh (true volume) vs shape-matching clusters (Müller 2005, no topology, lower
  fidelity but trivially robust) for the BASE tier?
- Where the soft pass sits vs `physics_apply` + the `IntegrationMode` gate (soft owns its particle integration like
  TGS owns rigid integration → needs an analogous ownership contract).
- Spatial-hash determinism: per-frame-rebuilt (warm-start style) acceptable, or incremental later?

## Sources
- Jolt Soft Body (DeepWiki 3.2/3.3 + `SoftBodyMotionProperties.cpp`) — the closest architectural precedent (separate XPBD pass; `mCollisionPlane.SignedDistance` per vertex; edge/volume/bend constraints; parallel grouping). https://deepwiki.com/jrouwe/JoltPhysics/3.2-soft-body-physics
- Macklin/Müller/Chentanez, "XPBD" (MIG 2016) — https://dl.acm.org/doi/10.1145/2994258.2994272 — compliance; α=0 ⇒ PBD.
- Macklin et al., "Small Steps in Physics Simulation" (SCA 2019) — https://mmacklin.com/smallsteps.pdf — substeps > iterations.
- Macklin et al., "Local Optimization for Robust SDF Collision" (I3D 2020) — https://mmacklin.com/sdfcontact.pdf — the SDF-collision authority (cloth/rigid/deformable).
- Macklin et al., "Unified Particle Physics" (SIGGRAPH 2014) — https://mmacklin.com/uppfrta_preprint.pdf — the unified pole (rigid-as-clusters); reference, not adopted.
- Müller, "Meshless Deformations Based on Shape Matching" (2005) — the cheaper low-fidelity tier.
- Teschner et al., "Optimized Spatial Hashing for Deformables" (2003) — https://matthias-research.github.io/pages/publications/tetraederCollision.pdf — self-collision hashing.
- "Parallel Block Neo-Hookean XPBD via Graph Clustering" (MIG 2022) — http://profs.etsmtl.ca/sandrews/pdf/xpbdBlockNeo_MIG22_preprint.pdf — coloring for deterministic parallel batches.
- bevy_xpbd/avian docs — Rust XPBD reference; soft bodies still future.
- Müller, "Ten Minute Physics" XPBD soft-body tutorials — reference loop (`alpha = compliance/dt²`).
