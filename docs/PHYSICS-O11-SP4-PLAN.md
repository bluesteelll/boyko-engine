# Physics O11 SP4 — Colored-Parallel Soft-Body Solve (PLAN, critic round 1 resolved)

> Status: architect plan, revised after architecture-critic CHANGES-REQUESTED (C1–C4 + W1–W4 resolved in-line). Pending critic re-review, then implementation. Structure (D1–D5) was APPROVED; this revision hardens the determinism spec.

## Goal

Parallelize the per-substep XPBD **projection sweeps** (SP1 distance, SP2 volume/tet, SP3 self-collision) of a single large `SoftBody` across the engine threadpool using per-constraint-type graph colorings, so that within one color every dynamic endpoint is write-disjoint. The SHIPPED serial path (SP1–SP3) stays **byte-identical** (the 0%-gate); the new colored path is **run-to-run bit-deterministic** and **{1,N}-worker bit-identical**, mirroring the rigid `ColoredSoftStepSolver` discipline. Target: for n ≳ 50k particles / m ≳ 150k distance constraints, projection scales near-linearly with lane count; small bodies fall back to serial at zero added cost.

## Determinism boundary (INVIOLABLE, identical to SP1–SP3)
Exact `mul`/`add`/`sub`/`div`/`sqrt`; NO FMA/`mul_add`/rsqrt/rcp/`Vec3::normalize`. Coloring changes only the inter-constraint *visit order*, which is value-equivalent because same-color constraints are write-disjoint on dynamic rows (proof below). The single dynamic predicate is `is_dynamic_row(inv_mass) = inv_mass != 0.0` (solver/contact.rs:38) — the SAME predicate the coloring and the write-guard share (anti-drift, colored.rs:1048-1052).

## Decisions (D1–D5, APPROVED)
- **D1** — per-type colorings: distance (2-arity), volume tets (4-arity), self-collision pairs (2-arity), each solved as its own colored sweep, preserving the SP1–SP3 phase order.
- **D2** — port `ParticleColorGraph` from the rigid `ConstraintGraph` greedy first-fit colorer (resources.rs:1910-1966), particle-indexed, arity-generalized {2,4}, dynamic-only occupancy, no union-find/islanding.
- **D3** — distance/volume colorings computed ONCE per frame (immutable topology), reused across substeps; self-collision recolored EVERY substep (its pair set is position-dependent).
- **D4** — separate `step_body_colored` sibling; `step_body` LEFT LITERALLY UNTOUCHED (see C4).
- **D5** — bare-pointer dispatch via `SoftColorPtrs`, mirroring `ColorSolvePtrs` (colored.rs:270-335) exactly.

## C1 (RESOLVED) — pinned-write guard: shared-guarded-kernel, finiteness LOAD-BEARING
The three leaf kernels (`project_distance`, `project_volume`, `project_self_pair`) gain a per-endpoint write guard `if is_dynamic_row(w_endpoint) { ...the existing add... }`, covering BOTH endpoints of every type (distance a,b; volume all 4; self i,j), routed through the SAME `is_dynamic_row` predicate the coloring uses. ONE guarded kernel is shared by both the serial `step_body` and `step_body_colored` (NOT a duplicated sibling — duplicating the hardest determinism math is the worse drift hazard).

**Mandatory for soundness:** pinned particles impose no coloring occupancy, so two constraints in one color CAN share a pinned endpoint; without the guard two workers both run `pos[pinned] += +0.0` on the same row — a value-benign but real data race, invisible to snapshot tests, visible only to Miri-TB/loom. The guard removes the pinned write entirely.

**Serial byte-identity proof (corrected after critic round 1, C1-A/C1-B):** dynamic endpoint → guard true → identical add. Pinned endpoint → today's write is `pos += nrm*(s*0.0)`. The real load-bearing invariant is: **a pinned endpoint has `inv_mass == 0.0` (EITHER sign of zero) AND `s` is finite** — NOT "inv_mass >= +0.0". (The constructor validates only `w.is_finite()` and `inv_mass` is a `pub` field, so `-0.0` or a negative finite mass are reachable; both are determinism-SAFE because the guard and the coloring both route through `is_dynamic_row(w) = (w != 0.0)`, which treats `±0.0` identically (IEEE `-0.0 == 0.0`) and treats a negative finite mass as dynamic on BOTH the serial and colored paths → no divergence. Negative/`-0.0` masses are documented as out-of-contract-but-determinism-safe; we do NOT add a constructor reject.) Given a finite `s` and `w` a signed zero, `s*w` is a signed zero, `nrm*(±0.0)` is a signed zero, and `x + (±0.0) == x` exactly. So the guarded skip is bit-equal to the unconditional `+= ±0.0` **on all finite-position states**. The byte-identity claim is scoped to finite-position states: a NaN/Inf already in `pos_*` is already out-of-contract (the kernels + the SP3 sweep `debug_assert!` finite positions), and on such a state both paths are out-of-contract. The guard's `debug_assert!` asserts `s.is_finite()` (the LOAD-BEARING invariant, mirroring colored.rs:1037-1046 apply_impulse) — it MUST NOT assert `w >= 0.0` (that invariant is false).

## C2 (RESOLVED) — coloring occupies ALL dynamic endpoints; disjointness Lemma
`ParticleColorGraph` sets occupancy on AND the free-test checks ALL dynamic endpoints: 2 (distance/self) and **all 4 vertices** (tets). 4-arity free-test = the conjunction over all four; a pinned vertex short-circuits its conjunct and is never occ_set. Distinct tet vertices are guaranteed by the constructor (`SoftBodyError::DegenerateTet`).

**Lemma (color disjointness):** within any color, for distinct constraints C1≠C2 and any row p: if p is dynamic it is touched by at most one of them (read- AND write-disjoint); if p is pinned it may be read-shared but is written by neither (C1's guard). *Proof:* greedy first-fit gives C2 a color only if every dynamic endpoint is un-set; a constraint reads/writes only its own endpoints → same-color dynamic read/write sets are pairwise disjoint; pinned rows read-only-shared, write-never. ∎ This closes the loop with C1 (the guard makes "written by neither" true → concurrent &mut into pos columns is sound).

## C3 (RESOLVED) — self-collision recolor: emission order + alloc contract
(a) The per-substep pair emission runs the BYTE-FOR-BYTE SP3 sweep (particle asc; 27-cell dz→dy→dx; ascending CSR within bucket; accept iff j>i AND cell_coord(j)==queried), emitting the ordered pair list; the coloring consumes pairs in that emission order (greedy first-fit is order-dependent). Structure: emit → color → solve color-by-color (vs the serial solve-inside-sweep); the CSR build + traversal are reused unchanged, only the leaf action differs.

(b) Alloc contract: ADOPT the rigid documented "steady-state-zero / growth-frame realloc" contract (a hard worst-case pre-size is O(n²) absurd and never occurs). Recolor buffers (pair list, chosen scratch, color_occ, CSR) are reserve-sized at body construction (from n and self_table_size) so the common case never reallocs, but may resize-grow on a denser-than-reserved substep. The zero-alloc TEST asserts STEADY STATE (after a warm-up window), NOT first-N-frames. Documented in the recolor doc comment citing resources.rs:1946-1949.

## C4 (RESOLVED) — `step_body` LITERALLY untouched (option a)
`step_body` is not edited at all (body, op order, predict/collide/velocity passes). `step_body_colored` is a SEPARATE function that DUPLICATES the driver passes (predict, sdf-collide, velocity — simple `for i in 0..n` SoA loops) and calls only the shared LEAF kernels (which C1 proves byte-preserving serially). Sharing happens where it is provably safe (the projection math); duplication where extraction would risk the serial determinism surface. A `{1,N}` runtime equivalence gate (forced 1-color/1-lane == serial) replaces a fragile before/after bit-baseline.

**MANDATORY interleaving (critic C4 caveat):** `step_body_colored` MUST reproduce `step_body`'s exact per-substep structure (solver.rs:261-394): one `for _ in 0..substeps` loop, inside it predict → distance-projection → volume-projection → self-collision → coupling → SDF-collide → velocity-update, in that order; AND the self-collision pass builds its spatial hash ONCE per substep and runs `self_collision_iters` Gauss-Seidel sweeps over that one hash (self_collision.rs:136-141, 90-95) — the colored self-collision must likewise emit/color the pair set against the single per-substep hash and sweep `iters` times over it. If the colored driver reorders phases or rebuilds the hash per sweep, the {1,N} gate compares two DIFFERENT integrators and passes vacuously. Coupling stays serial (IM-1 boundary).

## W1–W4 (RESOLVED)
- **W1** {1,N} oracle anti-vacuity (BOTH mandatory): (i) a debug counter asserts ≥1 color crossed `MIN_PARALLEL_SLOTS_PER_COLOR` and emitted >1 parallel chunk; (ii) the body actually MOVED (non-trivial snapshot). Cite the rigid anti-vacuity asserts (colored.rs:3267, 3593).
- **W2** Amdahl: serial fraction f ≈ 3n/(3n+m+k+s_p) ≈ 0.27 for a tet mesh; ~2.77× at L=8. Break-even ~1k particles (a color needs ≥256 slots to pay a scope; widest color ≈ m/12). SP4 ships PROJECTION-ONLY; predict/velocity/collide stay serial (low arithmetic intensity — a scope per O(n) streaming pass doesn't pay); a flat chunked par-for for those is deferred to SP4.1 if the bench shows the 27% floor dominates.
- **W3** PORT (not generalize) `color_manifolds` → `ParticleColorGraph`, carrying a `debug_assert_coloring` invariant re-scan. Diff table — every determinism-load-bearing divergence from the rigid source (resources.rs:1910-1989), each argued determinism-preserving:

  | # | Rigid source | Soft port | Determinism preserved because |
  |---|---|---|---|
  | 1 | 2-body manifold endpoints | arity {2 distance/self, 4 tet} | 4-arity free-test = conjunction over all 4 vertices in fixed order; pinned short-circuits; distinct vertices guaranteed (DegenerateTet) |
  | 2 | `is_dynamic` on BodyEffective.inv_mass | `is_dynamic_row(SoftBody.inv_mass[i])` | same predicate, different column; routed identically by guard + coloring (C1) |
  | 3 | occupancy words over body count | words over PARTICLE count (`n.div_ceil(64)`) | pure sizing; no value impact |
  | 4 | recolor every step | distance/volume colored ONCE/frame, reused across substeps; self-collision recolored every substep | reuse valid ∵ topology immutable (component.rs); self pairs are position-dependent → recolor (C3) |
  | 5 | reuses uf_parent/uf_size as scratch | dedicated `chosen`/cursor in `SoftColorScratch` (no union-find) | pure scratch |
  | 6 | counting-sort CSR (stable, ascending within color) | same | identical stable-CSR determinism property |
  | 7 | fixed visit order = manifold order | fixed order = distance `0..m` / volume `0..k` / self = SP3 emission order (C3a) | greedy first-fit is a pure function of the (fixed) visit order |
  | 8 | n/a — rigid colorer + solver in one module | **leaf kernels + SP3 traversal SHARED across modules** (W3-A) | byte-preserving visibility widening, single definition, no duplication |

- **W3-A** (shipped-file edits beyond the guard): the leaf kernels `project_distance`/`project_volume` (soft::solver) and `project_self_pair` + the `sweep`/`resolve_pair_in_cell` traversal (soft::self_collision) are module-private and must be reachable from the new `soft::colored`. DECISION: widen them to `pub(in crate::soft)` and SHARE the single definition (NOT duplicate — duplicating the hardest determinism math is the drift hazard C1 avoids). The colored self-collision emit path calls the SAME `sweep`/`resolve_pair_in_cell` in emit mode. This is a byte-preserving visibility change; it is the ONLY shipped-file edit beyond the C1 guard.
- **W4** `SoftColorPtrs` TB discipline: dispatcher reads color/group CSR via RAW pointers (no &[]/&mut into the body live across scope.spawn); &mut into pos columns formed ONLY inside the spawned task body; inv_mass as `*const`; one scope-Drop barrier; private fields via &self accessors (disjoint-capture idiom). Mirrors ColorSolvePtrs; cites the Phase-9.3c foreign-write failure mode.

## Determinism proof (summary)
Serial 0%-gate: step_body unedited (C4a) + leaf-kernel guard byte-identical serially (C1) → non-colored world bit-identical to SP1–SP3. Colored bit-identity: colorings are pure functions of immutable topology / the deterministic SP3 emission order; within a color each dynamic row is written by exactly one constraint in its own fixed-order kernel → scheduling-independent ({1,N}); colors solved 0..n_colors sequentially with a scope barrier → deterministic inter-color order; per-write op order unchanged.

## API (additive)
`PhysicsConfig { soft_body_colored: bool /*default false*/, soft_self_collision_colored: bool /*default false*/ }`; `physics_soft_step_colored` system (sibling of `physics_soft_step`, same `.after(solve).before(apply)` slot, never both run); `ParticleColorGraph::{color_constraints_2, color_constraints_4, color_span}`; new `SoftColorScratch` resource (coloring buffers). Leaf kernels keep their private signatures (guard internal).

## Integration
New module `soft/colored.rs` (physics_soft_step_colored, step_body_colored, ParticleColorGraph, SoftColorPtrs, SoftColorScratch). `resources.rs` two flags. `plugin.rs` `add_physics_soft_colored` stands in for `physics_soft_step` (mirroring `physics_solve_colored`). Shipped-file edits are exactly TWO, both byte-preserving: (1) the C1 leaf-kernel guard; (2) the W3-A visibility widening of the leaf kernels + the SP3 `sweep`/`resolve_pair_in_cell` traversal to `pub(in crate::soft)` so the single definition is shared (no duplication).

## Gates
Serial bit-baseline (kernel guard byte-identity); {1,N} oracle with BOTH W1 anti-vacuity gates; run-to-run colored bit-identity; debug_assert_coloring; 4-arity disjointness; pinned-shared-across-color scene (proves the guard); **Miri-TB MANDATORY** on the {1,N} oracle + a multi-worker stress scene (the only oracle for the pinned-write race + SoftColorPtrs aliasing); property test (random meshes: colored == serial bitwise); criterion A/B at n∈{1k,10k,50k,200k} measuring the empirical f.

## Owner questions (defaults applied)
Q1 soft_self_collision_colored default false (highest-risk parallel surface, opt-in). Q2 SIMD 8-wide cohorts deferred to SP4.1. Q3 predict/velocity stay serial in SP4 (W2 arithmetic-intensity), revisit in SP4.1.
