# L8 design: friction per manifold patch (D2)

**Scope.** L8 replaces the per-point friction rows with one friction patch per manifold: two tangent rows plus a twist row at the patch centre, built on L11's cohort blocks.

**How this was produced.** This is a read-only design against `D:/wt/joltab`.
- I had no shell, so I could not check that the tree is at `47c5dabd`, and graphify did not run.
- Line numbers are from the working copy, which is pre-L11. Each site is named twice: by today's line, and by the L11 structure that replaces it (`levers/L11-solve-setup/02-DESIGN-REV1.md`).
- "arith." means computed from P0 spans and counts. It is not a measurement.
- **One correction to the research brief, from a fetch of Bepu's source.** `FrictionHelpers.ComputeFrictionCenter` uses *binary* weights (`depth < 0 ? 0 : 1`), not depth-proportional ones. So for touching contacts, all four twist-row engines (Jolt, Box3D, Bepu, Rapier) use a plain mean. D3 is based on that.

## Goal
- **What it does.** Each manifold's friction moves from 2 tangent rows per point to 2 tangent rows plus 1 twist row per manifold, applied at the per-body mean of the contact anchors.
  - Normal rows stay per point.
  - The limits are `|λt| ≤ μ·Σλn` (a circle) and `|λtw| ≤ μ·Σ dᵢλnᵢ` (an interval). There is no twist row when the manifold has 1 point.
  - This is Jolt v5.6's model, as in Box3D, Bepu and Rapier's Simplified mode.
- **Scope of the value change:**
  - It changes values for manifolds with 2 or more points.
  - It is **bit-identical for any scene whose manifolds all have 1 point** (spheres, sphere-SDF, edge-only), by construction (D2 and D6).
  - It applies to the colored solver's scalar oracle and to the AVX2 kernel together. The two stay bit-identical to each other. `SoftStepSolver` is untouched (D1).
- **Targets.** J, `simd_solve` on, per-contact metric = µs per manifold per stage (arith., on L11's projected end state; derivation in §Gain):

| quantity | base (L11 end state) | L8 Δ | after L8 |
|---|---|---|---|
| wide colours K(1) | 2.68–3.22 ms (0.59–0.71 µs/m) | −0.62 … −0.99 ms | 1.90–2.39 ms (**0.42–0.53 µs/m**) |
| solve span per manifold, W=1 | 0.72–0.94 µs | −0.14 … −0.23 µs | **0.53–0.76 µs** (Jolt v5.3.0: 1.30–1.49) |
| T(1) | 8.67–9.79 ms | **−0.64 … −1.06 ms** (−6.5 … −12.2 %) | — |
| T(8) (after L5 + L11) | 3.93–4.69 ms | **−0.10 … −0.19 ms** (−2.1 … −4.8 %) | — |
| heap allocations / scopes per step | 0 / unchanged | 0 / 0 | — |

## Context and constraints
- **Base.** The L10 tip (rev 2.2), which carries L11's records and blocks and L9's tags. L8 edits only L11's structures.
- **Tree facts it rests on:**
  - The per-point normal-then-friction order: `colored.rs:2083-2172` (scalar) and `:2484-2603` (kernel).
  - The friction limit reads the λn just written: `:2131`, `:2539`.
  - Friction runs in all 12 sweeps: `:3567-3616`, 4 substeps × (1 biased + 2 relax), `resources.rs:452-453`.
  - Lane = manifold group, and the groups of one colour are body-disjoint (`colored.rs:44-67`).
  - The normal clamp never yields −0 (`:2106-2114`, `:3176-3181`).
  - Inertia is refreshed every substep (`:3597-3600`).
  - `tangent_basis` switches its seed at `|n.z| ≥ 0.999` (`contact.rs:57-69`).
- **Row counts.** J: 16,888 points and 4,524 manifolds, 3.73 points per manifold; L11 packs them into about 571 cohorts at 4.06 ranks each (2,320 blocks). Rows go from 3P = 50,663 to P + 3M = 30,460 (ρ = 0.399).
- **Binding rules:**
  - bit identity across W ∈ {1, 2, 4, 8, 16} and across scalar and SIMD;
  - replay on any machine without id identity;
  - principle 0 (all new state is `ScratchColumn`s owned by the solver);
  - no hot-path allocation; lock-free;
  - no FMA (the census tests cover new code automatically);
  - every gate can fail;
  - the P0 protocol;
  - A7-R1/R2, the freeze steps, the sleep suites and incline stick/slip are never loosened.

## Key decisions

### D1 — The colored solver adopts the patch model with no runtime switch. `SoftStepSolver` stays per point, byte-untouched, as the discrete Coulomb reference.
- **What:**
  - The colored scalar oracle and the AVX2 kernel both switch to the patch model.
  - `soft_step.rs` gets no diff.
  - The two solvers are compared on the new physical gates (G-P1…G-P4), which run on both with the same analytic bounds, and on the paired acceptance suites (`colored_acceptance_o5.rs` against `softstep.rs`, `sdf_collision.rs`).
  - R-ref's pose hash must stay equal to its pre-L8 value.
- **Why:**
  - Per-point friction at point supports is the discrete Coulomb model: each point's friction opposes its own slip, up to μλnᵢ. That makes it the right reference for bounding the patch approximation (G-P4).
  - D1 (`56c1e9e7`) already made the default differ from the reference in value.
  - The red set stays out of `softstep.rs` and R-ref.
  - One friction kernel means one hot loop in the I-cache and 2 solve paths (scalar, SIMD) instead of 4.
- **Rejected:**
  - **A `FrictionModel {PerPoint, Patch}` switch (Rapier-style).** Both models would share one block, which needs `ti1`/`ti2` per rank plus `d`: 352 B, which rounds up to a 384 B stride, so +20 % block traffic in the default path to keep a non-default model alive. It would also double the O7 differential suite. The same-binary A/B it would enable is not needed: the predicted effect (≥ 6.5 % at W=1) is far above binary-layout noise, so a C0-against-C2 binary A/B resolves it.
  - **Porting the patch model to the reference.** That loses the Coulomb reference and moves `softstep.rs`, the reference arm of `sdf_collision` and R-ref.
- **Trade-off:** per-point friction is no longer available on the colored path. The parent binary and `SoftStepSolver` remain the per-point comparison.

### D2 — Order: all of a manifold's normal rows, then twist, then the tangent pair. Normal-first (Box3D, Bepu, Rapier), not Jolt's friction-first.
- **What:** per lane:
  - normal rows in point order, each accumulating `s_n += λn'` and `s_d += dᵣ·λn'`;
  - then, if count ≥ 2, the twist row with limit `μ·s_d`;
  - then the tangent pair at the centre with limit `μ·s_n`.
- **Why:**
  - **(i) Exact reduction.** With count = 1, `s_n = +0 + λn' = λn'` exactly (λn' ≥ +0 and never −0), and the centre is exactly the anchor (D3). So the tangent step is op for op today's per-point friction. Friction-first would read the previous sweep's λn.
  - **(ii) No new per-lane state.** `s_n` and `s_d` stay in registers across the rank loop. Friction-first would need a stored Σλn per lane.
  - **(iii) No first-sweep gap.** A new contact gets friction on its first sweep. Friction-first gives it a limit of 0, because Σλn starts at 0.
  - **Twist before tangent (Box3D, Rapier):** the stick/slip-deciding row gets the last word, and it reads the angular velocity the twist has already changed.
- **Rejected:**
  - Jolt's friction-first. Its rationale, "non-penetration has the last word", matters less with 12 sweeps, and it breaks (i).
  - Bepu's tangent-then-twist order.

### D3 — Centre = the per-body plain mean of anchors, `rc_a = Σraᵢ / c` and `rc_b = Σrbᵢ / c`. dᵢ = in-plane distance on side A.
- **What:**
  - The sum starts from `ra₀` (not from +0), then adds ranks 1..c in order, then divides by `c` as f32 (IEEE division, not ×1/c).
  - `dᵢ = |δ − (δ·n)n|` with `δ = raᵢ − rc_a`, stored per rank. Padding ranks hold 0.
- **Why:**
  - It is the rule of all four engines for touching contacts. Box3D's weight is 1 up to its speculative distance; Bepu's is binary. boyko keeps no speculative points, so their continuity mechanisms have nothing to act on.
  - Separate centres per body avoid Jolt #2121's lever-arm error (a shared midpoint adds sep/2 to the arm).
  - For count 1, `ra₀ / 1.0 = ra₀` bitwise, −0 included, so no special case is needed.
- **Rejected:**
  - **Depth-proportional weights.** No engine uses them. Noise of 1e-6 m in depth moves the centre when resting depths are close.
  - **Weights from λn.** They tie the geometry to warm-start state and configuration.
  - **Box3D's C0 decay moved onto a penetration band.** It needs a new tuned constant with no measurement behind it. It is held as the fallback in OQ5, triggered only by A7-R1 evidence.
- **Trade-off:** the centre jumps when the point count changes (1,836 support point-count changes over A7-R1's window). The torque kick is `Δrc × F_t`, and F_t is small at rest. The A7-R1 bands are the refuting gate.

### D4 — Tangent pair: two 1D rows plus a circle clamp, today's exact ops (Jolt). Not the 2×2 block (Box3D, Rapier).
- **Why:**
  - It keeps D2's exact count = 1 reduction.
  - It reuses `effective_mass(_x8)`.
  - The block's off-diagonal term, `(rc×t1)·I⁻¹·(rc×t2)`, is 0 for `rc ∥ n` with a diagonal inertia. That is the centred resting patch, so the block buys nothing there. Elsewhere it costs a 2×2 inverse, about 12 more ops and 1 more division per lane.

### D5 — Warm storage: raw `(λt1, λt2, λtw)` per manifold in L11's `WarmRecord`, seeded only if at least one fid of the new manifold hits. No new key class.
- **What:**
  - The record keeps `n[4]` per point (matched by fid, as in L11) plus `t1`, `t2`, `tw` per manifold.
  - The seed goes through L11's single lookup routine (the W1 ruling). The friction seed is taken from the first record the routine visits (descending `mi` over the equal-key run) that holds at least one of the new manifold's fids; otherwise it is 0. `tw` is seeded only if the new count is ≥ 2.
  - The B1/L10 carry uses the same routine, so a frozen manifold carries its friction under the same rule.
- **Why:**
  - Friction persists as long as any point persists. Today a point whose fid changes loses its friction seed, a slip kick on flickering supports.
  - "No hit ⇒ 0" keeps today's behaviour when every point is new.
  - Raw λ in the manifold's basis is what Jolt does, and it is bit-exact for count 1.
- **Rejected:**
  - **A world-space vector with re-projection (Box3D, Rapier).** It breaks count = 1 exactness, and it fixes a basis-seed switch exposure that per-point storage already has today (`colored.rs:3243`). It is a separate value change; OQ4 holds it with a wall-contact gate.
  - **Unconditional carry (Jolt).** It seeds a geometrically new patch with the old friction.

### D6 — Warm apply: one merged impulse per (manifold, body). Count = 1 lanes use today's exact op sequence.
- **What, per lane:**
  - `P = (n·s_n + t1·λt1) + t2·λt2`.
  - Torque on A:
    - count = 1: `ra₀ × (P·−1)`, today's ops;
    - count ≥ 2: `(((Q_a×n) + (rc_a×F)) + n·λtw)·−1`, with `Q_a = Σλnᵢ·raᵢ` and `F = t1λt1 + t2λt2`.
  - B is the same without the sign.
  - Then `v += P·m⁻¹` and `ω += I⁻¹·T`, blended by `present ∧ movable`.
  - Rank accumulators use `blendv(acc, acc + x, active)`, never a masked +0 add, so they are exact by construction.
- **Why (arith., AVX2 op counts per cohort):**
  - today's (L11 C3) warm apply: 399 ops;
  - applying each part separately (per-rank normal, then friction, then twist): 488 (+22 %);
  - merged: about 252 (**−37 %**).
- **Rejected:** applying each part separately. It costs more, and it changes count-1 bits (`(v+a)+b` against `v+(a+b)`).

### D7 — Layout: L11's head unchanged; a new 320 B `CohortPatch`; `RankBlock` swaps `ti1, ti2` for `d, vn0`; L11's `rank_cold` is deleted.
- **Why:**
  - The rank block stays 320 B, so the kernel streams the same bytes per rank.
  - Line 5 of each block (`d`, `vn0`) becomes read-only in the kernel, so there is one dirty line per block per sweep instead of two.
  - The read-only centre lines are kept apart from the λ lines that are rewritten each sweep.
  - `vn0` sits in the otherwise unused slot of line 5, which the kernel loads anyway for `d`. Restitution reads `ra`, `rb` and `ni` from the same block.
- **Trade-off:** +320 B per cohort (head 320 + patch 320 + blocks 4.06×320 = 1,939 B against L11's 1,619 B, +20 %). The per-step set is 1.11 MB at J: in L3 at W=1, and about 139 KB per worker at W=8, which fits L2. The kernel is latency- and ALU-bound: 13.3 MB per step over about 2 ms is about 6.6 GB/s.

### D8 — Patch geometry is computed in the fill: a scalar oracle plus an 8-lane pass per cohort under `simd_solve`, bit-identical to each other.
- **Why:**
  - Scalar geometry costs about 26 flops plus 1 sqrt per point, about 0.08–0.11 ms of serial fill (arith.). That can erase L8's W=8 gain (§Gain).
  - The 8-lane pass costs about 144 vector ops per cohort, about 0.01–0.03 ms.
  - This is L11's warm-apply pattern: SIMD under the flag, scalar as the oracle.
- **Rejected:** computing the geometry in the kernel. It is constant for the step, and the kernel runs 12 times.

### D9 — Clamp selects with SIMD semantics
- **What:** the twist clamp uses `max_sel(a,b) = if a > b {a} else {b}` and `min_sel(a,b) = if a < b {a} else {b}` in the scalar oracle. These are exactly VMAXPS/VMINPS for ±0 ties and NaN.
- **Why:** `μ·s_d` can be +0, so its negation is −0, and ±0 ties do occur. The normal clamp's "a tie cannot occur" proof does not apply here.

### D10 — Friction stays in all 12 sweeps
- Box2D main and Box3D skip friction in bias sweeps. That is a separate value lever with its own stability question (fewer friction iterations near the threshold). It is not bundled into D2; see OQ3.

## Data structures
```rust
// solver/colored.rs (L11's CohortColumns gains `patch`; `rank_cold` deleted)
#[repr(C, align(64))] #[derive(Clone, Copy)]
struct CohortPatch {                      // 320 B = 5 lines, one per cohort; index = cohort k
    rc_a: [[f32; 8]; 3],                  // lines 0-1.5: centre on A (read-only in sweeps; fill writes)
    rc_b: [[f32; 8]; 3],                  // lines 1.5-3: centre on B (0 on a sentinel lane)
    lt1: [f32; 8], lt2: [f32; 8],         // line 3: accumulated tangent impulses (RMW per sweep)
    ltw: [f32; 8],                        // line 4a: accumulated twist impulse (0 on count-1 / absent lanes)
    _spare: [f32; 8],                     // line 4b: zero; reserved for L12's per-epoch twist mass
}
#[repr(C, align(64))] #[derive(Clone, Copy)]
struct RankBlock {                        // 320 B, unchanged size (L11: ra, rb, sep, ni, ti1, ti2)
    ra: [[f32; 8]; 3], rb: [[f32; 8]; 3], // lines 0-2
    sep: [f32; 8], ni: [f32; 8],          // line 3: ni written by the kernel (the only dirty line)
    d: [f32; 8],                          // line 4a: in-plane distance to rc_a; 0 on padding ranks
    vn0: [f32; 8],                        // line 4b: restitution-only (lazy, L11 D9 / O3 predicate)
}
// solver/warm_records.rs (L11): value fields change, 64 B kept
#[repr(C, align(64))] #[derive(Clone, Copy)]
pub(crate) struct WarmRecord {
    n: [f32; 4],                          // per-point λn, point order (fid-matched on lookup)
    t1: f32, t2: f32, tw: f32, _r: f32,   // per-manifold friction; tw = 0 when count < 2
    fid: [u16; 4], count: u8, _pad: [u8; 23],
}
// row_identity.rs: WarmSeedStats (pub) + 2 fields, 32 -> 40 B, no padding
pub manifold_hits: u32,       // solved manifolds whose friction seed was taken (D5)
pub manifold_carry_hits: u32, // frozen/kept manifolds whose friction was carried
```
- **Sizes:** const-asserted 320, 320 and 64 B, with 64-aligned bases (debug-asserted).
- **Scratch ids:** `patch` takes `rank_cold`'s id, so the id count is unchanged. Co-swept pairs stay asserted distinct mod `POOL_STAGGER_LINES`.
- **Drop and Send/Sync:** every type is POD with no `Drop`. `CohortSolveView` keeps L11's `unsafe impl Send + Sync` and gains the `patch` base under the same disjointness argument.
- **Committed memory at J:** +183 KB of patch, −74 KB of `rank_cold`; records unchanged.

## Public API
- **Changes:** `WarmSeedStats` gains 2 `pub` fields (additive; the const assert goes from 32 to 40). Nothing else public changes: `PhysicsConfig`, the solver constructors and `tangent_basis`/`effective_mass` are untouched.
- **New `pub(crate)` items:**
  - `BodyEffective::apply_angular_impulse(t)`, `twist_mass(n, a, b)`, `max_sel`, `min_sel` (`contact.rs`);
  - `twist_mass_x8`, `apply_angular_blend_x8`, `patch_geometry_x8` (`simd.rs`).

## Algorithms for the critical paths
```text
fill P-c, per cohort (after L11's per-lane/per-rank writes):
  seed λn[r] by fid (L11 routine); friction seed per D5 → patch.lt1/lt2/ltw (0 if no hit; ltw 0 if c<2)
  geometry (scalar oracle | x8 under simd_solve):
    S_a = ra_0; S_b = rb_0; for r in 1..depth: S = blendv(S, S + ra_r, active_r)   // same for rb
    rc = S / c_f32                                        // IEEE div; c=1 ⇒ rc = ra_0 bitwise
    for r: δ = ra_r − rc_a; p = δ − (δ·n)·n; d_r = blendv(0, sqrt(p·p), active_r)

kernel, per cohort (masks: active_r = width>r, present = width>0, multi = width>1):
  L11 gather + head; s_n = 0; s_d = 0
  for r in 0..depth:                     // normal rows only: 9 aligned loads (ra, rb, sep, ni, d)
    normal row (today's ops) → ni'; blk.ni = blendv(old, ni', active_r)
    s_n = blendv(s_n, s_n + ni', active_r); s_d = blendv(s_d, s_d + d_r·ni', active_r)
  twist:  m = twist_mass_x8(n, Ia, Ib)   // 1/(n·Ia⁻¹n + n·Ib⁻¹n), 0 if k ≤ 0
          w = (ω_b − ω_a)·n; lim = μ·s_d; new = min(max(ltw − m·w, lim·−1), lim)
          τ = n·(new − ltw); ω_a/ω_b += I⁻¹(∓τ) under multi∧movable; ltw = blendv(ltw, new, multi)
  tangent: today's friction ops with (ra, rb) → (rc_a, rc_b), max = μ·s_n, λ = lt1/lt2,
           masks present∧movable; lt = blendv(lt, new, present)
  L11 scatter
scalar oracle: the same per group (lane), group-major; count-1 groups take no twist branch
warm apply: D6, per cohort, serial (L11 C3's structure)
store: rec.n[r] = ni (r < c); rec.t1/t2/tw = lt1/lt2/ltw; carry per D5
```

| pass | ops per cohort (AVX2, D = 4.06) | today (L11) | cache / branching |
|---|---|---|---|
| kernel | D·213 + 311 (tangent) + 110 (twist) ≈ **1,286** | D·519 ≈ 2,107 (**−39 %**) | rank loop: normal only (9 loads, 1 dirty line); patch step once per cohort; no new branches (masks) |
| masses inside the kernel | D·82 + 164 + 44 ≈ 541 | D·246 ≈ 999 | L12 then caches 1 per rank + 3 per lane |
| warm apply | ≈ 252 | ≈ 399 (−37 %) | per-rank accumulators; one apply per body per lane |
| fill geometry | ≈ 144 (x8) | 0 | L1-resident cohort lines; 1 sqrt per rank vector |
| store | per point 1 float + per lane 3 | per point 3 | 11 ascending write streams (L11) |

- **I-cache:** the rank loop body roughly halves; the patch step (about 420 ops) sits outside it. No `#[inline(always)]`; the new x8 helpers are `#[target_feature]` functions like today's.

## Determinism argument
- **Scalar = SIMD.**
  - Every lane runs the scalar oracle's op sequence: IEEE `add/sub/mul/div/sqrt`, no FMA; the selects follow D9.
  - Inactive ranks and lanes change nothing: accumulators and λ stores use `blendv(old, new, mask)`, velocities use the existing movable blends, and padding lanes and ranks stay 0.
  - Absent lanes read no body row (L11 O2).
- **Count = 1 identity.** Each piece reduces to today's ops:
  - the centre, `ra₀/1` (D3);
  - the limit, `s_n = λn'` (D2);
  - the twist, masked (`multi`);
  - the warm apply, D6's single branch;
  - the seed, "a hit ⇔ at least one hit" when count is 1;
  - restitution, which is untouched.

  So a scene whose manifolds all have count 1 is bit-identical to the pre-L8 tree (gate G-P5).
- **W invariance.**
  - Cohort membership, cuts, waves and chunk quotas are L11's and independent of W.
  - The patch step is a pure function of its lane, and lanes within a colour are body-disjoint.
  - Colour order is fixed. The fill is serial or per cohort, and records are indexed by `mi`.
- **Replay on any machine.** Integer merges plus op-for-op f32, the same binary, and no entity-id input (keys are L11's row ordinals through `RowIdentity`). Nothing depends on thread count or steal order.

## Multithreading model
- **Unchanged from L11.** P-a, P-b, the fill, the warm apply and the store run on the calling thread; the kernel keeps L11's dispatch.
- **SIMD chunks** own whole cohorts, so heads, patches and blocks are exclusive.
- **Scalar chunks** own groups (lanes). They write distinct lane elements of `patch.lt*` and `blk.ni` through `addr_of_mut!` projections, never through a `&mut` to a whole struct. This adds 2 `unsafe` sites, each with a `// SAFETY:` citing cohort or lane ownership, `k < n_cohorts`, no concurrent reader until the join, and provenance from `solve_base`.
- **False sharing** is possible only where a scalar chunk boundary splits a cohort: at most 2 patch lines per boundary per sweep, the class L11 already accepted.
- **Synchronisation:** no new atomics. The existing scope join gives happens-before to the serial store, so loom is not needed.

## Gain (arith., J, `simd_solve` on)
- **Kernel.** L11's G(1) is the measured body gather/scatter share (L11's OQ2 ruling); until then it is the review's 100–160 ns per cohort-sweep × 6,852 cohort-sweeps = 0.69–1.10 ms.
  - ΔK = −0.390·(K_L11 − G) = −0.390 × (1.58–2.53) = **−0.62 … −0.99 ms**.
  - K = 2.68 pairs with −0.62…−0.78, and K = 3.22 with −0.83…−0.99, so K_L8 = 1.90–2.39 ms.
  - The ceiling with G = 0 is −1.05 … −1.26.
- **Warm apply.** The math part is 2,284 cohort-passes × 399 ops ≈ 0.11–0.15 ms, and −37 % of it is **−0.04 … −0.06**. Gather/scatter is unchanged.
- **Fill.** The x8 geometry adds **+0.01 … +0.03**; the scalar geometry would add +0.08 … +0.11. Per-point friction seed writes disappear.
- **Store.** Writes fall from 50.7k to 30.5k floats: **−0.01 … −0.02**.
- **Totals:**
  - W=1: **−0.64 … −1.06 ms**; per manifold (÷4,519.3) **−0.14 … −0.23 µs**.
  - W=8: ΔK/8 = −0.08…−0.12 plus serial −0.02…−0.07 = **−0.10 … −0.19 ms**. L(8) = 0.510 ms (dispatch plus imbalance) is held constant. That is why this is far below the plan's ceiling ρ·(t_colours + t_warm) = 0.64 ms, which scaled L(8) and gather/scatter too.
- **Claimability at K=6** (SE bars: J W=1 2.0 %, W=8 0.75 %):
  - W=1, −6.5 … −12.2 %: yes;
  - W=8, −2.1 … −4.8 %: yes if realized.
- **cfg-A (scalar colours).** Per point the scalar path goes from 3 masses and 12 body copies to 1 mass, plus 3 per group. Expected −30 … −45 % of P(1) (−3.7 … −5.5 ms), claimed only if measured.
- **Refutation.** At C0, recompute ΔK with L11's *measured* G and K. If the lower end is below 0.3 ms at W=1, stop and escalate.
- **Against Jolt.**
  - Solve per manifold 0.53–0.76 µs against v5.3.0's 1.30–1.49 (each side's own count).
  - v5.6.0 has no per-stage profile. "If its −15 % were all in its solve: 1.02–1.21 µs" is an assumption, not a comparison.

## Integration: files and lines (today's line, then L11's name)

| file | lines | change |
|---|---|---|
| `solver/contact.rs` | `:101-148` | + `apply_angular_impulse`, `twist_mass`, `max_sel`, `min_sel` (C2) |
| `solver/simd.rs` | `:540-740` | + `twist_mass_x8`, `apply_angular_blend_x8` (C2); `patch_geometry_x8` (C1) |
| `solver/colored.rs` | `:1-80` | module doc: "friction per patch" section; IM-2b text already rewritten by L11 |
| | `:1737-1817` (L11 P-c fill) | friction seeds per D5; `vn0` into the block; geometry (C1) |
| | `:1881-1918` (L11 scalar + `warm_apply_avx2`) | D6 merged form, both paths |
| | `:2028-2174` (L11 group-major oracle) | D2 patch step; per-point friction `:2127-2172` removed |
| | `:2228-2653` (L11 cohort kernel) | rank loop normal-only plus accumulators (`:2537-2603` removed); patch step; patch load/store |
| | `:3141-3197` | restitution reads `vn0` from the block |
| | `:3229-3319` (L11 D3 store/carry) | record fields; carry per D5; `manifold_carry_hits` |
| `solver/warm_records.rs` (L11) | — | `WarmRecord` value fields; the lookup returns the friction source |
| `row_identity.rs` | `:259-292` | + 2 fields, assert 40 B, doc |
| `scratch_ids.rs` (L11 contact band) | — | `patch` layout on `rank_cold`'s id; census |
| `solver/colored_tests.rs` | `:736-772` (digest), `:1420-1454`, `:1748-1900` (`cone_probe` → `patch_probe`), `:1915`, `:2009`, `:2101` | ports |
| `tests/friction_patch.rs` (new) | — | G-P1…G-P5 |
| `tests/bodytype_determinism_golden.rs` | `:1-55`, `:81`, `:348-380` | re-pin plus contract amendment; + `GOLDEN_COUNT1` |
| `tests/sleep_settles_box_piles.rs` | `:64-127`, `:313` context | L8 bands (C0), readings (C2) |
| `tests/frozen_island_warm_start.rs` | `:642-787`, `:1256` | + friction non-vacuity asserts |
| `benches/jolt_parity_pyramid.rs` | `:140`, `:1326-1334` | pose fixtures |
| `benches/sleeping_pipeline.rs` | `:23-25` | freeze-step doc |
| docs | `SYSTEMS.md`, `FEATURE_MAP.md`, `OPEN-QUESTIONS.md` (A7 witness), MEASUREMENT-QUEUE | — |
| `solver/soft_step.rs` | — | **no diff** (a gate) |

**Interactions with other lanes:**
- **L10.** Its kept records copy whole 64 B records, so friction rides along. Restore goes through the single lookup routine, so D5 applies. The new stats follow L10's logical-view ruling (B1): they count kept manifolds exactly as with sleeping off.
- **L9.** Refreshed manifolds keep their fids, so friction seeds hit.
- **L12.** It inherits a smaller mass set (1 per rank plus 3 per lane) and the `_spare` slot.
- **Dispatch.** Chunk quotas stay in points, so waves, scopes and chunk pins are unchanged. Re-tuning the floors is OQ6.

## Commit sequence (each green: `cargo test --workspace --all-targets --no-fail-fast`, clippy `-D warnings`, the release physics leg, the Miri leg)

| commit | content | values |
|---|---|---|
| **C0** | Gates and fixtures on the base, with no solver change: `friction_patch.rs` G-P1, G-P2, G-P4 on both solvers (bands registered from the base's own run, below); a G-P3 base run (receipt); `GOLDEN_COUNT1` plus a count-1 scene hash (spheres, sphere-SDF, sphere-sphere) at W {1, 8} × simd {off, on}; runner pose fixtures (J, J-A, R, R-ref, R-S, S16 at W {1, 8}); the L8 A7-R1 bands written into the file; L11 G and K re-read; the ΔK refutation computed | none |
| **C1** | Bit-identical preparation: `CohortPatch` (rc from the geometry; λ = 0, not read); `rank_cold` → `{vn0, d}` (d computed, not read); scalar and x8 geometry under `simd_solve`; setup digest extended | none, zero pin moves |
| **C2** | **The value change:** D2/D4 in the oracle and the kernel, D6 in both warm applies, D5 seed/store/carry, the `RankBlock` swap (`rank_cold` deleted), the math helpers, `WarmSeedStats` fields, the O7 port, G-P3 (red-first on C1 if the base failed it), every re-pin with its rule | multi-point only |
| **C3** | P0-protocol receipts (tester/analyst), the per-contact table, docs | — |

## Gates (each shown able to fail; mutations recorded red before their commit)

**Physical gates, both solvers, same bounds** (`friction_patch.rs`):

| gate | scene and bound | fails via |
|---|---|---|
| **G-P1 incline** | Flat box, half (0.5, 0.2, 0.5) (tips only above tan θ = 2.5), yaw 0° and 30°, μ ∈ {0.2, 0.5, 0.8}. **Stick** at tan θ = 0.95μ: creep < 5 mm over 240 frames after a 240-frame settle (dt 1/120). **Slide** at 1.05μ: displacement in [0.5, 1.5] × ½g(sin θ − μ cos θ)t². At 2μ: acceleration within ±5 % of analytic. The ±5 % band holds if the C0 base passes it; otherwise the registered band is the tightest the base passes in 1 % steps (≤ ±10 %, else escalate). L8 must pass the registered band. | M1 limit `μ·maxᵢλnᵢ` (stick red); M2 limit `c·μΣλn` (slide red) |
| **G-P2 twist stop** | Box half (1, 0.25, 1), 8 kg (I_n = 5.333), μ = 0.2, ω₀ = 2 rad/s, v = 0. Analytic stop t = ω₀I/(μmg√2) = 0.481 s = 28.9 steps at 60 Hz. Stop step ∈ [25, 33]; COM drift ≤ 1 mm; after the stop \|ω\| < 1e-3 for 60 steps | M3 twist dropped (never stops); M4 limit `μΣλn` (≈ 41 steps) |
| **G-P3 yaw while sliding** (Jolt #983) | Box half (0.5, 0.25, 0.5), μ = 0.3, yaw 30°, v₀ = 3 m/s, ω₀ = 0. Heading change ≤ 0.5°, lateral ≤ 1 cm, stop distance within ±5 % of v²/2μg = 1.529 m | red-first if C0's per-point run fails; else M5 tangent rows at `ra₀` |
| **G-P4 slide + spin** | Box half (1, 0.25, 1), μ = 0.3, v₀ = 2 m/s, ω₀ = 2 rad/s. Stop-time ratio patch/reference ∈ [0.5, 1.05]. Derivation: by the triangle inequality P_coulomb ≤ P_patch; for a symmetric patch P_patch ≤ 2·P_coulomb | M3 (ratio > 1.05); M6 both limits ×3 (ratio < 0.5) |
| **G-P5 count-1 identity** | The C0 count-1 hashes and `GOLDEN_COUNT1` are unchanged at W {1, 8} × simd {off, on}. A unit test on a scene with ω = −0.0 checks the ω bits are unchanged | M7 warm apply split into normal + friction parts for count-1 lanes; M-C3 twist solved at count 1 (flips −0 in the −0.0 scene) |

**Rest and sleep (never loosened):**
- **A7-R1** (`CREEP_BOUND_M = 0.01`, today 0.7180 mm). Pre-registered at C0:

  | D_max | verdict |
  |---|---|
  | < 2 mm | confirms |
  | [2, 10) mm | acceptable, reported beside 0.7180 |
  | ≥ 10 mm | STOP: no merge; escalate with OQ5's decay variant |

  Witnesses (reported, not gated): support point-count changes, and the per-layer drift vector. The (−x, −z) bias is a hypothesis only.
- **A7-R2 and G8** must freeze within their tests' existing limits; the steps are read again.
- **G2/G7 budgets** stay unchanged under the file's triage protocol. The `flicker_redraw_distribution` generator is re-run. A result that would need a higher budget is a STOP, never a raise.
- **Existing suites:** `support_loss_wakes_sleepers`, all of `frozen_island_warm_start` plus the new asserts (`manifold_hits > 0` on the twin; `manifold_carry_hits` equals the frozen manifolds with at least one fid hit, on every frozen step), `colored_acceptance_o5` (`:512-524`, `:565-580`) and `sdf_collision` all pass unchanged.
- **Carry mutations:** M8, "carry drops friction", turns `a_woken_pile_steps_exactly_as_its_twin…` and the carry count red. M9, "seed without a fid hit", and M10, "tw seeded for count 1", are caught by lookup unit tests.

**Structural gates:**
- **G-S1, scalar = SIMD:** `simd_solve_on_off_bit_identical`, the GOLDEN pair, and the ported O7 suite. `patch_probe` replaces `cone_probe`, and the proptest must show at least one circle clamp, one twist clamp, one unclamped twist, count-1 and count-4 lanes in one cohort, and a sentinel lane. There is also a new {scalar, simd} warm-apply differential. Mutation M11: plain `f32::max/min` in the scalar twist clamp, caught by a zero-limit lane with a −0 candidate.
- **G-S2, W invariance:** pose hashes equal across W ∈ {1, 2, 4, 8, 16}; L11's G3 layout-bytes test covers `CohortPatch` (M12: an unblended patch store leaves padding lanes non-zero).
- **G-S3, census:** 0 heap per step; scope and chunk pins unchanged.
- **G-S4, Miri (Tree Borrows):** two threads writing distinct lanes of one cohort's patch (scalar parallel). M13: a base taken via `as_read_slice().as_ptr().cast_mut()`.
- **G-S5, setup digest:** simd on and off are equal at C1 and C2. M15: `S·(1/c)` in x8 against `S/c` in scalar.
- **G-S6, the reference is untouched:** an empty `soft_step.rs` diff, and R-ref's pose hash equal to its C0 value.

**G-T timing (P0 protocol).**
- **Setup:** A = C0 binary, B = C2 binary, cargo `parity`, K = 6 interleaved, receipts per process, and the canary must be seen. Claim iff |effect| > 2·√(SE_A² + SE_B²).
- **Rows:** J (default configuration) and J-A (cfg-A) at W ∈ {1, 2, 4, 8, 16}; R, R-S and R-ref at W {1, 8}; S16. Armed J-a at W {1, 8} for per-stage figures.
- **Realized-gain gates:**
  - Δ wide colours(1) ≤ −0.37 ms, claimed;
  - ΔT(1) on J ≤ −0.38 ms, claimed (0.6 × the lower predictions);
  - W = 2/4/8/16: not claimed slower (claimed where above SE);
  - cfg-A: not claimed slower;
  - R-ref: not claimed different.
- **Per contact:** each run uses its own manifold count, for bp, np, setup (build/warm/store), colours and solve per manifold at W {1, 8}, beside Jolt's bracket. If B's H8 count moves by more than 2 %, T(W) is flagged as confounded by the contact set, and the per-manifold stage figures become the primary claim.

**Debug assertions:**
- `s_n` and `s_d` ≥ +0 and never −0;
- `ltw == 0` on count-1 and absent lanes;
- after each patch step, `lt1² + lt2² ≤ (μ·s_n)²·(1 + 1e-6)` and `|ltw| ≤ μ·s_d`;
- count-1 `rc_a` is bitwise `ra₀`;
- `d ≥ 0` and finite, and 0 on padding;
- a record with `count == 0` has zero friction fields;
- padding lanes and ranks are zero (L11).

**Edge cases covered:** μ = 0, μ NaN (D9 semantics), warm start disabled, a Reset remap, duplicate-pair streams (the first record with a hit), a sentinel B, all anchors coincident (d = 0, twist limit 0), and singular inertia (k ≤ 0 gives mass 0).

## What moves at C2, and how each is re-measured
- **`bodytype_determinism_golden.rs:81` GOLDEN.** Its contract names only the EnableTag refactor, so it is re-pinned by amending the contract with an owner-approved D2 entry. Both `golden…` and `golden_scalar_colored_equals_golden` must produce the **same** new value. `GOLDEN_COUNT1` must not move.
- **`sleep_settles_box_piles.rs`:**
  - A7-R1 D_max, under the C0 bands;
  - the A7-R2 freeze step (248) and G8 (185), read again (including `sleeping_pipeline.rs:23-25`);
  - the `:125` line (6671 manifolds / 22,975 points), read again;
  - the G2/G7 observed counts, under triage with budgets untouched.
- **Runner:** the `--expect-pose` fixtures and P0's J pose `0x32d5e235342b4143` are re-recorded (one hash per row across W and across scalar/SIMD). R-ref's must equal C0's. The H8 count 4,519.3 is read again.
- **O7 suite:** ported as relative (scalar against SIMD), never re-pinned.
- **Enumeration:** the red set is enumerated by running C2. Anything red outside this list (render goldens fed by box physics, `soft_body_sp1`/`sp2` coupling) is re-measured under its own file's rule, listed in the commit, or treated as a defect until triaged.

## What it cannot claim
- **v5.6.0 parity.** No per-contact per-stage parity with Jolt v5.6.0: it has no stage profile, and its manifold count is not receipted.
- **Coulomb exactness.** Combined sliding and spinning dissipates between 1× and 2× the per-point rate (G-P4 bounds it).
- **A7.** Any reduction of A7's drift rate; the witness is reported, not claimed.
- **W=8.** A gain beyond "not slower" unless it is measured above SE. The kernel figure is an op-count model bounded by L11's measured G.
- **Other gaps.** Torsional friction on single-point contacts, where there is none, as before. Continuity across the tangent-basis seed switch, whose exposure is unchanged.
- **Trajectories.** Bit continuity with pre-L8 trajectories for manifolds with 2 or more points.
- **cfg-A.** Gains unless measured.

## Open questions (orchestrator)
1. **W=8 gate.** The predicted −2.1 … −4.8 % is below the 5 % build-if at the low end. Should L8 be gated on W=1 plus per contact, as the L9 ruling did?
2. **GOLDEN re-pin.** A ruling on it: the contract amendment text, with the same value required for SIMD and scalar.
3. **L8b, friction only in relax sweeps** (Box2D, Box3D). A separate value lever: arith. −0.10 … −0.17 ms at W=1 on L8's base (M_L8 0.96–1.54 ms × patch share 0.327 × 4/12). Commission it?
4. **World-space friction warm storage** (the basis-seed switch at |n.z| = 0.999). A follow-on with a gate for a box resting against a wall.
5. **Centre fallback.** If A7-R1 reads ≥ 2 mm and the witness links drift to point-count-change steps: Box3D's C0 decay moved onto a penetration band. It needs a constant, so it needs a ruling.
6. **Chunk floors.** Point-based chunk floors re-derived after L8 (per-slot work −39 %). Bit-identical, but chunk-count pins move.
7. **`WarmSeedStats`.** The additive public fields (32 → 40 B).

## Files
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/contact.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/simd.rs`
- `D:/wt/joltab/crates/boyko_physics/src/solver/soft_step.rs` (read only; must stay unchanged)
- `D:/wt/joltab/crates/boyko_physics/src/solver/colored_tests.rs`
- `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`
- `D:/wt/joltab/crates/boyko_physics/src/scratch_ids.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/bodytype_determinism_golden.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/sleep_settles_box_piles.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/colored_acceptance_o5.rs`
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`

Sources:
- [Bepu ContactConvexTypes.cs](https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/Constraints/Contact/ContactConvexTypes.cs): binary friction-centre weights (fetched 2026-09-19).
- [Bepu repository](https://github.com/bepu/bepuphysics2) (search result).
- All other web facts come from the research brief's sources [1]–[24] (Jolt PR #2039, issues #983 and #2121; Box3D and Box2D `contact_solver.c`; Rapier `contact_with_twist_friction.rs`; the PhysX friction docs).
