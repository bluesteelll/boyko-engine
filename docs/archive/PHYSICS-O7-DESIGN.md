> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Architecture: Physics O7 — SIMD-Batched Colored Solve (lane = manifold-group, 8-wide AVX2)

## Goal

Widen the colored solver's single hot site — `solve_color`'s per-point normal+friction kernel — to AVX2 8-wide where one lane carries one whole **manifold-group** of a color, processing **8 body-disjoint groups in parallel** per batch. The widening targets the arithmetic-dense triple `effective_mass` evaluation (3× per point: normal, t1, t2; each a full `r×dir`, `I⁻¹·rd`, `(·)×r`, two `dot`s) plus the velocity gather/apply.

**Performance target.** The scalar `solve_color` body executes (per point) ~3 full `effective_mass` (≈ 3 × {2 cross, 1 mat3·vec = 3 dot, 2 dot} ≈ 45+ scalar FLOPs) + 2 `point_velocity` diffs + 1 `sqrt` + 1 `div`. At 8-wide with the same op tree, the steady-state throughput target is **≥ 3.5× speedup on the solve_color kernel** for a color of ≥ 8 same-width groups (sub-linear vs 8× from the per-rank gather/scatter of impulse slots and the masked ragged tail). The whole-step target is the O7 bench A/B showing `{simd colored} / {scalar colored}` ≥ 1.6× wall on a 10k-box stack (the solve dominates per-substep cost; integrate/refresh are already O1-widened). **Determinism is the hard gate, not perf**: bit-exact `simd == scalar` over every input including ragged ranks, mixed cone activation, and denormals.

## Context and constraints

- **Affected**: `crates/boyko_physics/src/solver/colored.rs` (the widened dispatch + cohort enumeration), `crates/boyko_physics/src/solver/simd.rs` (the new `solve_color_avx2` kernel + the FMA guard), `crates/boyko_physics/src/resources.rs` (the reused cohort scratch, if not foldable into `ContactColumns`). No change to `contact.rs` / `math.rs` (the kernel re-derives their math inline, op-for-op).
- **Invariants preserved**:
  - **INVIOLABLE-1 bit-exactness**: SIMD output == scalar `solve_color` output, `f32::to_bits` equality, including partial batches (< 8 groups) and ragged ranks (groups of width 1..MAX_CONTACT_POINTS in one batch).
  - **INVIOLABLE-2 width-only**: the SIMD path reproduces the **O5 colored bits exactly**, not the reference manifold-order solver. O7 changes WHERE arithmetic runs (8 lanes), never the per-group op sequence.
  - **INVIOLABLE-3 0%-gate**: `config.simd == false` ⇒ byte-identical to committed O6 (`solve_color` scalar). The widened call site is `if simd { solve_color_avx2(...) } else { solve_color(...) }` — a pure dispatch fork.
  - **INVIOLABLE-4 zero per-step alloc**: the cohort + rank enumeration reuses build-time CSR (`group_start`, `color_group_start`) + capacity-retained scratch. No `Vec::new`/`vec!` in the substep loop.
  - **INVIOLABLE-5 disjoint-write soundness**: 8 groups/batch ⇒ ≤ 16 distinct dynamic body rows written; statics/sentinels never written; each `unsafe` carries `// SAFETY:` listing the disjointness invariant.
- **Target metrics**: 0 allocations/substep on the solve path; the working set of one batch (8 lanes × {2 bodies × 13 f32 velocity+mass state} + per-rank {18 geometry f32 + 3 impulse f32}) ≈ 8 × (26 + 21) × 4 B ≈ **1.5 KB**, fits L1d with room for the SoA gather staging. I-cache: one kernel function body (no inlining of the scalar oracle into it), `#[cold]` scalar tail.

## Key decisions

### Decision 1: the batch unit is the manifold-group COHORT — lane = one whole group (R1)

**What.** A *cohort* = up to 8 body-disjoint manifold-groups of one color (the color's groups taken 8 at a time in `group_start` index order). The kernel:
1. **Gather-once at batch entry**: load the 8 groups' two body pairs (A,B) into stack SoA — `inv_mass`, the 9-element `inv_inertia`, `linear_velocity` (3), `angular_velocity` (3) — for both A and B of each lane (16 bodies → SoA columns). Plus per-group `width = group_start[g+1]-group_start[g]` and the per-group base slot.
2. **Rank loop** `r = 0 .. max_width_in_cohort`: at each rank the **active mask** = `(width[lane] > r)`. Each active lane solves *its group's point r* — normal solve then friction solve, in the oracle's per-point order — reading/writing **register-carried** A/B velocity and the per-rank impulse slots gathered from `group_start[g]+r`.
3. **Register-carry** A/B linear+angular velocity (6 + 6 = 12 `__m256`) across the whole rank loop, so lane `g`'s point `p1` sees `p0`'s velocity update (the intra-group Gauss-Seidel coupling). Per-rank, the impulse slots (`normal_impulse`, `tangent1_impulse`, `tangent2_impulse` at `group_start[g]+r`) are gathered → read-modify-written → scattered (each point owns its own slots). At **batch exit**, scatter the 12 velocity registers to the 16 distinct body rows **once**.

**Why.** This is the only batching that is bit-exact to the scalar oracle without re-deriving the numerics. Each lane runs *its group's full p0→friction→p1→friction→… sequence* identically to the scalar per-group sweep (Decision-2 proof). Cross-lane independence is the O4 coloring invariant — 8 groups of one color touch pairwise-disjoint dynamic bodies — so running them simultaneously is observationally identical to running them one group at a time; cross-group visiting order is irrelevant because they are disjoint. The payoff concentrates in the 3× `effective_mass` per point (O1 note: the 3 dirs n/t1/t2 are independent, no CSE-rounding risk; each is the full angular term `dir·((I⁻¹·(r×dir))×r)`).

**Alternatives rejected.**
- *Rank-across-groups (8 different groups' point-0 in lane 0..7, then 8 point-1s)* — REJECTED (the critic's C1/C2): this is what lane=group already is at rank 0; the distinction the critic flagged is that the *velocity coupling must be register-carried per lane across ranks*, not re-gathered per rank. Re-gathering velocity per rank would read a stale body row for a lane whose previous-rank update hasn't been scattered — a value divergence. Register-carry is mandatory.
- *Pack adjacent POINTS of one group into a lane* — REJECTED: a group's points share both bodies and are order-coupled (p1 reads p0's velocity update); SIMD-parallel points of one group would race the shared body in-register — not bit-exact.
- *AoS gather via `_mm256_i32gather_ps`* — REJECTED: gather-instruction latency dominates at this group count, and it forces a hardware-gather code path that diverges from O1's proven SoA-staging template. We gather scalar-to-stack-SoA (O1's pattern), then `_mm256_loadu_ps`.

**Trade-off.** Ragged widths waste lanes at high ranks (a cohort of 8 groups with widths {4,1,1,1,1,1,1,1} runs 4 ranks but only lane 0 is active at ranks 1..3). Accepted: contact manifolds are overwhelmingly 1- or 4-point; cohorts are formed in `group_start` order (no width-bucketing — see Decision 4) to keep the canonical warm-store order dispatch-independent. The per-rank impulse gather/scatter (3 columns) is the dominant non-8× cost.

### Decision 2: the lane=group bit-exactness proof (PINNED kernel-structure invariant) (R1)

**Premise (critic-confirmed, the anchor)**: within one color, each *dynamic* body belongs to **exactly one** manifold-group (the O4 coloring invariant: no two manifolds in a color share a dynamic body). A shared body across two groups in a color is necessarily *static* (`inv_mass == 0`), and statics are never written.

**Claim**: for any cohort of ≤ 8 groups `{G₀…G₇}` of one color, the kernel's final body-velocity state and impulse-column state are `f32`-bit-identical to solving `G₀`, then `G₁`, …, then `G₇` sequentially with the scalar `solve_color` over each group's slot run.

**Proof.**
1. *Per-lane op identity.* Lane `k` at rank `r` (when active) executes exactly the scalar `solve_color` body for slot `group_start[Gₖ]+r`: same normal solve (`m_eff`, `vn`, `bias`, `d_lambda`, `max(0)` clamp, applied_n), same friction solve (`m_eff_t1`, `m_eff_t2`, `vt1/vt2`, the cone clamp, applied_t1/t2), in the same order, reading the lane's register-carried A/B velocity (which holds exactly the value the scalar oracle would have after processing this group's points `0..r`). The AVX2 ops are the same IEEE round-to-nearest as scalar (Decision 5: no FMA, no rsqrt/rcp), so each lane's per-rank result is bit-identical to the scalar per-point result. ⇒ each lane's full rank sequence == the scalar group sweep for that group, bit-for-bit.
2. *Cross-lane non-interference.* The 8 lanes write disjoint register sets (lane `k` owns its A/B velocity registers and its own impulse slots `group_start[Gₖ]+r`). No lane reads another lane's register. The only shared memory a lane *reads* is a shared static body row — never written by any lane (Decision 6 guard). ⇒ no lane's computation depends on any other lane's state.
3. *Order irrelevance.* Because the lanes are non-interfering (step 2), the parallel evaluation of `{G₀…G₇}` produces, for each lane, the identical result it would produce run alone. Sequential scalar evaluation `G₀;G₁;…;G₇` likewise produces each group's result independent of the others (disjoint dynamic bodies). ⇒ the two agree per-group, hence the union (all 16 body rows + all impulse slots) agrees bit-for-bit.
4. *Masked exhausted lanes (Decision 3) contribute nothing* (proof in Decision 3). ∎

**PIN**: any kernel change that breaks (a) per-lane op-for-op identity, (b) register-carry of velocity across ranks, or (c) the masked no-write of inactive/static lanes is a bit-exactness regression and is a bug. The `simd == scalar` differential test (Test 1) is the oracle.

**Per-lane CARRIED state (explicit, resolves C2):**

| State | Width | Lifetime | Notes |
|---|---|---|---|
| A linear velocity (x,y,z) | 3 × `__m256` | **register-carried across all ranks** | gathered once at batch entry, scattered once at exit |
| A angular velocity (x,y,z) | 3 × `__m256` | register-carried | " |
| B linear velocity (x,y,z) | 3 × `__m256` | register-carried | sentinel B lanes hold IMMOVABLE_AT_REST (zeros), never scattered |
| B angular velocity (x,y,z) | 3 × `__m256` | register-carried | " |
| A inv_mass, B inv_mass | 2 × `__m256` | gathered once, constant | |
| A inv_inertia (9), B inv_inertia (9) | 18 × `__m256` | gathered once, constant | the world tensor refreshed pre-solve; constant within the solve_color call |
| ra (x,y,z), rb (x,y,z) | 6 × `__m256` | **per-rank** (gather slot `base+r`) | |
| normal/t1/t2 (9) | 9 × `__m256` | per-rank | |
| separation, friction | 2 × `__m256` | per-rank | |
| normal_impulse | 1 × `__m256` | **per-rank read-modify-write** at `base+r` | gather → new_lambda → scatter |
| tangent1_impulse, tangent2_impulse | 2 × `__m256` | per-rank read-modify-write at `base+r` | |
| max_friction | 1 × `__m256` | per-rank, = `friction * new_lambda` (the freshly-written λn this rank) | |
| active mask | 1 × `__m256` | per-rank = `(width[lane] > r)` | |
| ia_movable / ib_movable masks | 2 × `__m256` | per-rank (from gathered inv_mass + b_is_sentinel) | guards the velocity blend (Decision 6) |

Register pressure: AVX2 has 16 `__m256` (ymm0–15). The 12 carried velocity + 20 constant body-state registers exceed 16 ⇒ the carried velocity (12) stays in registers, the 20 constant inv_inertia/inv_mass columns spill to the stack SoA staging buffers and are reloaded per-rank via `_mm256_loadu_ps` (cheap L1 hits — they are the gather-once stack arrays). This is the same staging O1 uses; the compiler manages the spill. The *velocity* registers must not spill across the rank loop (LLVM keeps them live; verified via asm inspection in the bench step — if it spills, the value is still correct, only slower).

### Decision 3: the friction cone — two-predicate mask + unconditional divide, blend-discarded (R3)

**What.** Reproduce the scalar
```rust
let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
if len_sq > max_friction * max_friction && len_sq > 0.0 {
    let scale = max_friction / len_sq.sqrt();
    new_t1 *= scale;  new_t2 *= scale;
}
```
8-wide as:
```
len_sq   = add(mul(t1,t1), mul(t2,t2))                       // left-to-right, matches scalar
mf2      = mul(max_friction, max_friction)
cone_mask= and( cmp_GT_OQ(len_sq, mf2), cmp_GT_OQ(len_sq, zero) )   // BOTH predicates
scale    = div(max_friction, sqrt(len_sq))                  // UNCONDITIONAL on all 8 lanes
t1_clamp = mul(new_t1, scale);   t2_clamp = mul(new_t2, scale)
new_t1   = blendv(new_t1, t1_clamp, cone_mask)              // discard scale on unclamped lanes
new_t2   = blendv(new_t2, t2_clamp, cone_mask)
```

**Why.** Op-for-op match to the scalar: same `len_sq` add order, same `max_friction²` (a separate `mul`, matching `max_friction * max_friction`), same single `div(mf, sqrt(len_sq))` operand order (NOT `mf * (1/sqrt)` — the scalar is one divide), same `mul` by scale. A lane with `len_sq == 0` yields `scale = mf/0 = +Inf` (or `0/0 = NaN`); the `cone_mask` for that lane is false (its second predicate `len_sq > 0` fails), so `blendv` selects the *unscaled* `new_t1/new_t2` — the Inf/NaN is bit-discarded. A lane with `len_sq ≤ mf²` likewise selects unscaled. **No `_mm256_max_ps`** is used for the cone (correcting the earlier draft — the cone is compare+div+blend, not a max).

**Trap-free proof.** AVX2 `vdivps`/`vsqrtps` on a stale/zero lane produce Inf/NaN *as data*, not as a trap: the crate does not enable FP exceptions (no `_MM_SET_EXCEPTION_MASK` unmasking, default MXCSR masks all FP exceptions), so `vdivps` by zero raises only the (masked) divide-by-zero flag and yields ±Inf; `vsqrtps` of a negative would yield NaN (cannot occur here — `len_sq ≥ 0`). The result is consumed only by `mul` then `blendv`, and `blendv` selects the *other* operand for that lane (mask false). `blendv` is bit-exact selection (no arithmetic), so the discarded Inf/NaN never reaches the scatter. ∎

**Mandated test** (resolves C3, Test 1c): a differential batch with mixed cone activation in ONE cohort — ≥ 1 clamped lane (`len_sq > mf² > 0`), ≥ 1 unclamped lane (`0 < len_sq ≤ mf²`), ≥ 1 `len_sq == 0` lane (zero tangent velocity), ≥ 1 denormal-`len_sq` lane (`len_sq` a subnormal `f32`) — non-vacuous mask coverage proving the blend-discard is bit-exact.

### Decision 4: masked exhausted lanes — trap-free, no width-bucketing (R2)

**What.** Within a cohort, groups have widths 1..MAX_CONTACT_POINTS. At rank `r`, `active_mask[lane] = (width[lane] > r)`. An exhausted lane (`width ≤ r`) has no rank-`r` point; it still executes all the rank-`r` arithmetic on **stale gathered inputs** (the gather for an exhausted lane reads slot `base+r` which is *out of the group's run* — see the gather-clamp below), and every write (velocity blend, impulse scatter) is gated by `active_mask` via `blendv`, so its results are discarded.

**Gather safety for exhausted lanes.** The per-rank gather for lane `g` reads slot `s = base[g] + r`. For an exhausted lane `s ≥ group_start[g+1]`, which may point into the *next group's* slots (still in-bounds of the columns) or, for the last group of the last cohort, one-past-end. To keep the gather in-bounds without a per-lane branch, clamp the gathered slot to `min(s, len-1)` (a branchless `min`); the read is then always a valid slot whose value is *garbage for this lane* but bit-irrelevant (the lane is masked inactive, every write discarded). This is the standard masked-SIMD discipline; the clamp is computed in the scalar gather loop (the O1 staging pattern), not via a vector gather.

**Trap-free proof.** Identical to Decision 3: an exhausted lane's normal solve may compute `m_eff` with a `k ≤ 0` (garbage geometry) → `effective_mass` returns 0.0 (the `if k > 0` is widened as `blendv(0, 1/k, k>0)`, same discipline) or a finite value; its friction `sqrt`/`div` may produce Inf/NaN; ALL of it is `blendv`-discarded under `active_mask`. No FP trap (exceptions masked). The impulse scatter is gated: an inactive lane writes back the *original* gathered impulse value (`blendv(gathered, new, active_mask)`), so the slot it read (even a clamped-into-another-group slot) is rewritten with its own original value — a no-op write. **Critical**: the clamped slot must be rewritten with the value just gathered from it, so a collision with another lane's *active* slot would be a hazard — but two lanes never gather the same slot when both could write: active lanes gather distinct in-range slots; an exhausted lane's clamped slot, if it collides with an active lane's slot, writes back that slot's *current* value which the active lane in the SAME cohort owns. **To eliminate this hazard entirely**, exhausted lanes scatter is *fully masked off* (not "write original") via a masked store: use `active_mask` to select between a scatter and a skip per lane. AVX2 has no masked scalar scatter; we scatter via the stack-SoA pattern (vector store to stack, then a scalar per-lane loop `if active[lane] { columns[slot] = staged[lane] }`). The scalar scatter loop's `if active` makes the exhausted-lane write a true no-op — **no collision possible**. (This is the resolution: scatter through stack staging + a guarded scalar write-back, exactly as O1 scatters inertia.)

**No width-bucketing.** We do NOT reorder groups by width to pack equal-width cohorts, because the canonical warm-store walk (`canonical[]`, IM-2b) and the `group_start` order are the dispatch-independent determinism anchor. Bucketing would either (a) change `group_start` order (breaking the canonical store determinism) or (b) require a separate solve-order permutation (extra per-step scratch + indirection). The masked-ragged cohort keeps the solve order == `group_start` order == canonical-store-independent. The wasted lanes are the accepted cost (Decision 1 trade-off).

### Decision 5: FMA-free build guard — defense-in-depth (R5)

**What (accurate framing).** Rust does **not** auto-contract `a*b + c` into a single FMA: contraction requires either an explicit `f32::mul_add` (the kernel never calls it) or a global fast-math flag (Rust stable exposes none). So a `RUSTFLAGS="-Ctarget-feature=+fma"` build does **not** silently fuse the kernel's explicit `_mm256_mul_ps` + `_mm256_add_ps` — the differential test is therefore *not blind* (no contraction occurs; the test would still catch any accidental `mul_add`). 

**Still, defense-in-depth**: add at module scope in `simd.rs` (guarding O1's kernels too):
```rust
#[cfg(target_feature = "fma")]
compile_error!(
    "boyko-physics determinism requires no FMA contraction; \
     this SIMD module is written mul_add-free and must be built without +fma. \
     (No mul_add is emitted, so +fma would not actually contract our explicit \
      mul+add, but the build is rejected to make the no-FMA invariant load-bearing.)"
);
```
plus a module-doc invariant paragraph. This makes the no-FMA assumption a compile-time contract rather than a runtime hope. **Optional future hardening** (noted, not implemented): a stored cross-machine golden snapshot (Intel vs AMD bits) as a regression artifact. **This resolves W3 with no runtime assert** (the guard is compile-time, zero hot-path cost).

### Decision 6: O7 widens ONLY solve_color's normal+friction kernel (R6)

`warm_start_apply` and `apply_restitution` **stay scalar**, byte-identical to O6 — they are cheap once-per-substep / once-per-step sequential passes with *sentinel-only* guards (`if !b_is_sentinel`), a different and simpler guard than `solve_color`'s movable+sentinel guard. Widening them would add two more masked sites for marginal gain and is out of scope (removes W4's multi-site complexity).

**The single widened-site guard table** (the `solve_color` kernel, reproduced per-lane via `blendv` on BOTH the normal-velocity scatter and the friction-velocity scatter):

| Quantity | Scalar source | SIMD reproduction |
|---|---|---|
| `b_is_sentinel[lane]` | `cols.b_is_sentinel[i]` | gathered `bool→f32` mask (1.0/0.0) at batch entry |
| `bb_view` (body B) | `if b_is_sentinel { IMMOVABLE_AT_REST } else { bodies_eff[ib] }` | sentinel lanes gather B = zeros (IMMOVABLE_AT_REST is inv_mass 0, inv_inertia ZERO, vel ZERO) |
| `ia_movable` | `is_dynamic_row(bodies_eff[ia].inv_mass)` = `inv_mass_a != 0` | `mask_a = cmp_NEQ_OQ(inv_mass_a, 0)` |
| `ib_movable` | `!b_is_sentinel && is_dynamic_row(inv_mass_b)` | `mask_b = and(not_sentinel, cmp_NEQ_OQ(inv_mass_b, 0))` |
| normal velocity write A | `if ia_movable { apply_impulse(ra, -impulse) }` | A-velocity registers updated unconditionally, then `blendv(old_A_vel, new_A_vel, and(active_mask, mask_a))` |
| normal velocity write B | `if ib_movable { apply_impulse(rb, impulse) }` | `blendv(old_B_vel, new_B_vel, and(active_mask, mask_b))` |
| friction velocity write A/B | same guards | same blend, gated `and(active_mask, mask_{a,b})` |
| impulse slot write | unconditional `cols.normal_impulse[i] = new_lambda` (etc.) | scatter gated by `active_mask` ONLY (impulse slots are written for every live point regardless of body movability — matches scalar) |

**Note on the impulse write vs velocity write asymmetry** (load-bearing, matches scalar): the scalar writes `cols.normal_impulse[i] = new_lambda` *unconditionally* (even for a contact against a static B — the accumulated impulse is stored), but guards the *velocity* `apply_impulse` by `*_movable`. The SIMD must mirror this: impulse scatter gated by `active_mask` only; velocity blend gated by `active_mask AND *_movable`. A static *A* body cannot occur (A is always the dynamic side by manifold convention — but the guard `ia_movable` is kept for exactness with the scalar, which guards both sides). The `apply_impulse` itself is a value no-op on a static row (`inv_mass 0`, `inv_inertia ZERO`), so the `*_movable` guard is bit-identical to an unconditional blend — BUT it is **load-bearing for the disjoint-write soundness** (Decision 7): skipping the no-op write to a shared static row means no two cohorts/workers write the same `BodyEffective`.

### Decision 7: parallel cohort-partitioning — whole 8-group cohorts per worker (R4)

**What.** Under `parallel_solve && simd`, the unit dispatched to a worker is a whole **8-group cohort** (or a contiguous run of cohorts), NOT an arbitrary slot-balanced chunk. Decouple the SIMD batch size (8 groups, fixed) from the work-stealing granularity (a run of cohorts per task):

- Enumerate the color's cohorts: cohort `j` = groups `[g_lo + 8j, min(g_lo + 8(j+1), g_hi))`. The last cohort may be a partial (< 8) cohort — solved by the same masked kernel (Decision 4 handles `n_groups_in_cohort < 8`: the absent lanes are permanently inactive, `width = 0`).
- Distribute cohorts across workers: target `(num_threads + 1) * CHUNKS_PER_WORKER` tasks, each task a **contiguous run of whole cohorts** (the chunk boundary always falls on a cohort boundary = a `group_start` index that is a multiple-of-8 offset from `g_lo`). Balance by total slot count across the cohort run (groups vary in width), accumulating cohorts until the run's slot count reaches the per-task quota.
- **Composition with O6's `solve_color_parallel`**: the existing chunker cuts on *group* boundaries balanced by slot count. The change: when `simd`, **snap each chunk boundary to a cohort (8-group) boundary** so every task solves only full-width-8 cohorts (plus one possibly-partial trailing cohort per color, never per task). Concretely, the chunk-growth loop advances `chunk_g_hi` in steps of 8 (cohort granularity) instead of 1 (group granularity) until the slot quota is met. A task's slot span is still `[group_start[chunk_g_lo], group_start[chunk_g_hi])` — contiguous, the same shape `solve_color_avx2` consumes.
- The W1 min-work threshold (`MIN_PARALLEL_SLOTS_PER_COLOR`) still routes tiny colors inline (solved by `solve_color_avx2` on the calling thread when `simd`, else scalar `solve_color`).

**{SIMD}×{1,N workers} bit-identity.** Cohorts within a color are body-disjoint (each cohort is 8 disjoint groups; distinct cohorts are also pairwise disjoint — every group in the color touches a unique set of dynamic bodies). So a worker's cohort-run writes dynamic body rows disjoint from every other worker's. Statics never written (Decision 6). ⇒ partitioning cohorts across workers is bit-identical to solving all cohorts of the color single-threaded, for any worker count — and (by Decision 2) bit-identical to scalar. **No cross-worker write.** The proof composes: `{simd, 1 worker} == {scalar, 1 worker}` (Decision 2) and `{simd, N workers} == {simd, 1 worker}` (cohort disjointness, same as O6's group-chunk disjointness argument) ⇒ all four of `{scalar,simd}×{1,N}` agree bit-for-bit.

**Bench A/B (resolves W2/silent-width-degradation):** `{parallel + simd}` vs `{parallel scalar}` on a large color, to catch a regression where cohort-snapping starves a worker of width (e.g. a color with 9 groups → cohort 0 (8 groups) + cohort 1 (1 group): two tasks, the second is a degenerate 1-lane batch). The bench asserts `{parallel+simd}` ≥ `{parallel scalar}` wall (else the snapping granularity is mis-tuned).

## Data structures

The kernel introduces **no new persistent struct** — it reuses `ContactColumns` (the existing SoA + `group_start`/`color_group_start` CSR) and stack-local SoA staging (O1's pattern). The only addition is **per-cohort stack scratch**, declared inside `solve_color_avx2`, zero heap:

```rust
// Inside solve_color_avx2, per-cohort stack staging (no heap, ≤ ~2 KB):
const W: usize = 8;                 // cohort width = AVX2 lanes
// Per-lane (group) metadata, gathered once at cohort entry:
let mut g_base  = [0u32; W];        // slot base of each lane's group (group_start[g])
let mut g_width = [0u32; W];        // each lane's group width (group_start[g+1]-group_start[g]); 0 = absent lane
// Per-lane A/B body row indices (for the final scatter), gathered once:
let mut ia = [0u32; W];   let mut ib = [0u32; W];
let mut a_is_static = [0.0f32; W];  // mask source: 1.0 if !movable
// (sentinel B handled by gathering B = IMMOVABLE_AT_REST zeros)

// Per-lane A/B carried velocity staging (the register-carried state spills here only at exit):
let mut alvx = [0.0f32; W]; ... let mut aavz = [0.0f32; W];   // A lin(3) + ang(3)
let mut blvx = [0.0f32; W]; ... let mut bavz = [0.0f32; W];   // B lin(3) + ang(3)
// Per-lane A/B inv_mass + inv_inertia(9) staging (gather-once constants):
let mut a_invm = [0.0f32; W]; let mut b_invm = [0.0f32; W];
let mut a_ii = [[0.0f32; W]; 9]; let mut b_ii = [[0.0f32; W]; 9];
// Per-RANK geometry + impulse staging (re-gathered each rank from base+r, clamped):
let mut ra_s = [[0.0f32; W]; 3]; let mut rb_s = [[0.0f32; W]; 3];
let mut n_s  = [[0.0f32; W]; 3]; let mut t1_s = [[0.0f32; W]; 3]; let mut t2_s = [[0.0f32; W]; 3];
let mut sep_s = [0.0f32; W]; let mut fric_s = [0.0f32; W];
let mut ni_s = [0.0f32; W]; let mut ti1_s = [0.0f32; W]; let mut ti2_s = [0.0f32; W];
let mut active = [0.0f32; W];       // (g_width[lane] > r) as 1.0/0.0
```

Total stack ≈ (8 + 2 + 6 + 6 + 2 + 18 + 6 + 9 + 2 + 3 + 1) × 8 × 4 B ≈ **2.0 KB** per cohort — well within the stack and L1d. Reused across ranks (the geometry/impulse arrays) and across cohorts (re-overwritten). **Zero heap, zero per-step alloc** (INVIOLABLE-4).

The body-velocity registers (12 `__m256`) live in ymm across the rank loop; the staging arrays above are written only at gather (entry) and the velocity ones only at scatter (exit). The compiler keeps the live velocity vectors in registers; the constant inv_inertia/inv_mass reload from stack per rank (cheap L1).

## Public API

**No public API change.** `solve_color_avx2` is `pub(super)` (called from `colored.rs`), `cfg`+`target_feature`-gated like O1's kernels. The dispatch is internal to `solve_color_parallel` / `solve_all_colors`.

```rust
// In simd.rs (new): the widened kernel + its scalar fallback dispatcher.
// Mirrors O1's refresh_inertia(...) dispatch shape exactly.
#[inline]
pub(super) fn solve_color_dispatch(
    cols: &mut ContactColumns,
    bodies_eff: &mut [BodyEffective],
    span: (usize, usize),          // the color (or cohort-run) slot span — for the scalar fallback
    g_lo: usize, g_hi: usize,      // the group range [g_lo, g_hi) to solve as cohorts
    group_start: &[u32],           // the CSR (read-only)
    bias_rate: f32, mass_coeff: f32, impulse_coeff: f32, bias_active: bool,
    simd: bool,
);
// simd == false  -> calls solve_color(cols, bodies_eff, span, ...)   [O6 byte-identical, the 0%-gate]
// simd == true   -> calls solve_color_avx2(cols, bodies_eff, g_lo, g_hi, group_start, ...) on AVX2 builds,
//                   else solve_color (non-AVX2 / Miri).

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn solve_color_avx2(
    cols: &mut ContactColumns,
    bodies_eff: &mut [BodyEffective],
    g_lo: usize, g_hi: usize,
    group_start: &[u32],
    bias_rate: f32, mass_coeff: f32, impulse_coeff: f32, bias_active: bool,
);
```

`solve_color` stays exactly as-is (the bit-oracle). `solve_color_dispatch` REPLACES the direct `Self::solve_color(...)` calls in `solve_all_colors` (the non-parallel path) and inside each worker's `scope.spawn` body (the parallel path) — both now route through the dispatcher and pass `simd`.

## Algorithms for critical paths

### `solve_color_avx2` — op-by-op (the one hot path)

```
for cohort_lo in (g_lo .. g_hi).step_by(8):
    cohort_hi = min(cohort_lo + 8, g_hi)
    nlanes    = cohort_hi - cohort_lo        // 1..8 (last cohort may be partial)

    # ── Gather-ONCE (cohort entry) ──────────────────────────────────────────
    for lane in 0..8:
        if lane < nlanes:
            g = cohort_lo + lane
            base   = group_start[g];  width = group_start[g+1] - base
            s      = base                       # first slot of the group
            ia[lane]=cols.body_a[s]; ib[lane]=cols.body_b[s]; sent=cols.b_is_sentinel[s]
            gather A body (inv_mass, inv_inertia[9], lin[3], ang[3]) from bodies_eff[ia]
            gather B body: if sent { IMMOVABLE_AT_REST zeros } else bodies_eff[ib]
            g_base[lane]=base; g_width[lane]=width
            a_static[lane] = (A.inv_mass == 0) ? 1 : 0
            sent_mask[lane]= sent ? 1 : 0
        else:
            g_width[lane]=0   # permanently inactive lane (partial cohort)
            (everything else zeroed → harmless, masked off every rank)
    load A/B velocity → 12 __m256 (REGISTER-CARRIED from here)
    load A/B inv_mass(2), inv_inertia(18), a_static_mask, sent_mask  → stack-resident, reloaded per rank
    max_width = max(g_width[0..nlanes])

    # ── Rank loop (register-carry velocity) ─────────────────────────────────
    for r in 0..max_width:
        active = ( g_width[lane] > r )  as 8-wide mask
        # per-rank gather: slot s = clamp(g_base[lane] + r, 0, len-1)
        for lane in 0..8: s = min(g_base[lane]+r, len-1); stage ra/rb/n/t1/t2/sep/fric/ni/ti1/ti2 from cols[s]
        load ra(3),rb(3),n(3),t1(3),t2(3),sep,fric, ni,ti1,ti2  → __m256

        ## NORMAL solve (mirrors scalar, op-for-op, NO FMA) ##
        m_eff   = effective_mass_x8(n, ra, rb, A, B)         # the 3× angular term, widened (below)
        vn      = dot8( pointvel_x8(B, rb) - pointvel_x8(A, ra), n )
        bias    = bias_active ? max(mul(bias_rate, sep), -MAX_BIAS_VELOCITY) : 0
        lambda_n= ni
        d_lambda= bias_active ? sub(mul(neg_mass_coeff*m_eff, vn+bias), mul(impulse_coeff, lambda_n))
                              : mul(neg_m_eff, vn)            # EXACT scalar two-branch (see note)
        new_lam = max( add(lambda_n, d_lambda), 0 )
        applied_n = sub(new_lam, lambda_n)
        ni := new_lam                                         # impulse register updated
        impulse_n = mul(n, applied_n)                         # vec3 ×8
        # velocity blend (gated active AND movable):
        maskA = and(active, not(a_static)); maskB = and(active, and(not(sent), b_movable))
        A_vel := apply_impulse_blend(A_vel, ra, neg(impulse_n), maskA)
        B_vel := apply_impulse_blend(B_vel, rb,     impulse_n,  maskB)

        ## FRICTION solve (2-DOF cone, Decision 3) ##
        max_fric = mul(fric, ni)                              # ni = the JUST-written new_lambda
        m_eff_t1 = effective_mass_x8(t1, ra, rb, A, B)        # independent dir → no CSE risk (O1 note)
        m_eff_t2 = effective_mass_x8(t2, ra, rb, A, B)
        dv = pointvel_x8(B, rb) - pointvel_x8(A, ra)          # RE-READ post-normal velocity (scalar does too)
        vt1 = dot8(dv, t1);  vt2 = dot8(dv, t2)
        nt1 = sub(ti1, mul(m_eff_t1, vt1));  nt2 = sub(ti2, mul(m_eff_t2, vt2))
        len_sq = add(mul(nt1,nt1), mul(nt2,nt2))
        cone   = and( cmp_GT(len_sq, mul(max_fric,max_fric)), cmp_GT(len_sq, 0) )
        scale  = div(max_fric, sqrt(len_sq))                 # unconditional
        nt1 = blendv(nt1, mul(nt1,scale), cone); nt2 = blendv(nt2, mul(nt2,scale), cone)
        applied_t1 = sub(nt1, ti1); applied_t2 = sub(nt2, ti2)
        ti1 := nt1; ti2 := nt2
        impulse_t = add(mul(t1, applied_t1), mul(t2, applied_t2))
        A_vel := apply_impulse_blend(A_vel, ra, neg(impulse_t), maskA)
        B_vel := apply_impulse_blend(B_vel, rb,     impulse_t,  maskB)

        # per-rank impulse SCATTER (gated by active, via stack staging + guarded scalar write):
        store ni,ti1,ti2 → stack; for lane in 0..8: if active[lane] { cols.normal_impulse[g_base[lane]+r]=...; ... }

    # ── Scatter-ONCE (cohort exit): A/B velocity registers → 16 body rows ────
    store A_vel(6),B_vel(6) → stack; for lane in 0..nlanes: bodies_eff[ia[lane]].{lin,ang}=...; if !sent[lane] { bodies_eff[ib[lane]].{lin,ang}=... }
```

**Note on `d_lambda` two branches.** `bias_active` is a *scalar* (whole-call) flag, not per-lane — so it is a Rust `if`/`else` over the whole kernel (two monomorphized code paths or a runtime branch hoisted outside the rank loop), NOT a per-lane blend. This matches O5 exactly (the scalar `if bias_active` is loop-invariant). The relax passes (`bias_active == false`) take the `mul(neg_m_eff, vn)` path; the main sweep takes the soft path. Hoist the branch outside the cohort loop for I-cache compactness.

**`effective_mass_x8(dir, ra, rb, A, B)`** widens `contact::effective_mass` op-for-op:
```
angular(ii, r):  rd = cross8(r, dir); dir · ( cross8( mat3mulvec8(ii, rd), r ) )   # mat3mulvec8 = 3 per-row dot8 (O1 has this)
k = A.invm + B.invm + angular(A.ii, ra) + angular(B.ii, rb)
m_eff = blendv( 0, div(1, k), cmp_GT(k, 0) )                                        # the `if k>0 {1/k} else {0}` widened
```
`cross8`, `dot8`, `mat3mulvec8` reuse O1's `mat3_mul_x8` building blocks (`_mm256_mul_ps`/`_mm256_add_ps`/`_mm256_sub_ps`, all left-to-right matching `Vec3::cross`/`dot` and `Mat3::mul_vec`'s per-row dot). The `1/k` is `div(one, k)` matching scalar `1.0 / k` (one divide).

**Complexity / cache / branching / SIMD.**
- **Complexity**: per color, `Σ_cohorts (max_width_in_cohort × per-rank-cost)`. For uniform-width-1 (sphere/floor, the common case), 1 rank/cohort, 8 groups/rank ⇒ `ceil(n_groups/8)` rank iterations vs `n_groups` scalar slot iterations → ~8× iteration reduction. For width-4 box manifolds, 4 ranks/cohort but each rank does 8 points → same 8× over scalar's 32 slot iterations, modulo masked-lane waste in mixed-width cohorts.
- **Cache**: gather is scalar-strided over the SoA columns (sequential within a group's slot run; strided across the 8 groups of a cohort — but the groups of a color are contiguous in slot order, so the 8 group-bases are near each other → mostly L1-resident). Body gather is random (16 body rows) but only ONCE per cohort. Steady state: **streaming-sequential over the color's columns**, the SoA layout's design intent.
- **Branching**: the only hot branches are the `for cohort` / `for rank` loop conditions and the guarded scalar scatter `if active[lane]`. All numeric conditionals (`max(0)`, the cone, `k>0`, `*_movable`) are branchless `blendv`/`max`/`cmp`. `bias_active` hoisted outside the loop.
- **SIMD potential**: fully realized — the entire per-rank body is `__m256` ops; the only scalar code is the gather/scatter staging loops (unavoidable SoA<->cohort-strided marshaling, the O1 pattern).

### Cohort-snapping in `solve_color_parallel` (the parallel partition)

The existing chunk-growth loop (`colored.rs:1119–1202`) advances `chunk_g_hi` group-by-group until the slot quota is hit. **Change when `simd`**: round `chunk_g_lo`/`chunk_g_hi` to cohort boundaries — `chunk_g_lo` is always `g_lo + 8k`, and `chunk_g_hi` advances in steps of 8 (a whole cohort) until the slot quota is met or `g_hi` is reached (clamped to `g_hi`). The chunk's slot span `[group_start[chunk_g_lo], group_start[chunk_g_hi])` is passed to `solve_color_dispatch(simd=true)` with `(chunk_g_lo, chunk_g_hi)` as the cohort range. Every task thus solves whole cohorts (the last cohort of the *color* may be partial, handled by the masked kernel; the last cohort of a *task* is always whole because boundaries snap to multiples of 8). O(1) extra logic, zero alloc.

## Multithreading model

- **Shared**: `ContactColumns` (read geometry, read-modify-write impulse slots), `[BodyEffective]` (read-modify-write velocity rows). Dispatched via the existing `ColorSolvePtrs` `Send+Sync` raw-pointer wrapper (unchanged).
- **Thread-local / per-task**: the entire `solve_color_avx2` stack staging (≈ 2 KB) is task-local (declared inside the kernel, on each worker's stack). No shared scratch.
- **Synchronization points**: the per-color `pool.scope` Drop barrier (unchanged from O6) orders the Gauss-Seidel sweep across colors. **No new sync**; no atomics; no locks. Within a color, tasks are fully independent (cohort disjointness).
- **Partitioning**: cohorts (8 groups) → runs of cohorts → tasks (Decision 7). Work-stealing balances cohort-runs by slot count.
- **Atomics**: NONE. The disjoint-write argument eliminates contention entirely.
- **Data-race freedom proof**: (1) distinct cohorts of a color touch pairwise-disjoint dynamic body rows (O4 invariant, extended from group to cohort = union of 8 disjoint groups) and pairwise-disjoint impulse slots (distinct group slot runs); (2) shared static rows are never written (`*_movable` guard, Decision 6); (3) sentinel B never written; (4) the scope-Drop join completes color `c` before color `c+1` reads. ⇒ no two tasks write the same `BodyEffective` or column element; all cross-task sharing is read-only (geometry + shared statics). The kernel's `&mut` reborrows alias only provably-disjoint written elements — the same `// SAFETY:` as O6's `scope.spawn`, extended: "a cohort-run writes only its cohorts' disjoint dynamic rows and disjoint impulse slots; ≤ 8 groups/cohort ⇒ ≤ 16 distinct dynamic rows/cohort; statics/sentinels never written."
- **Send/Sync**: unchanged — `ColorSolvePtrs` carries the same pointers; the kernel adds only stack-local state (trivially `Send`).

## Integration

- **`simd.rs`**: add `solve_color_dispatch`, `solve_color_avx2`, the `effective_mass_x8` / `apply_impulse_blend_x8` / `cross8` / `dot8` / `mat3mulvec8` / `pointvel_x8` helpers (reusing O1's `mat3_mul_x8` style), and the `#[cfg(target_feature="fma")] compile_error!` guard + module-doc invariant. Import `ContactColumns` — currently `ContactColumns` is private to `colored.rs`; make it `pub(super)` (visibility-widen only, no layout change) OR keep the kernel in `colored.rs`. **Decision: keep `solve_color_avx2` in `colored.rs`** (next to its oracle `solve_color`, the same module as `ContactColumns`), and put only the reusable `effective_mass_x8` / vector-math `x8` helpers in `simd.rs` (they take `__m256`, not `ContactColumns`). This avoids widening `ContactColumns` visibility and keeps oracle+kernel co-located (the O1 pattern is kernel-next-to-oracle).
- **`colored.rs`**: 
  - `solve_all_colors` non-parallel branch: replace `Self::solve_color(...)` with the dispatch `if use_simd { Self::solve_color_avx2(...) } else { Self::solve_color(...) }` — but `solve_all_colors` currently doesn't carry `simd`. **Change**: thread `simd: bool` through `solve_all_colors` and `solve_color_parallel` (both gain a `simd` param; `solve_colored` passes `use_simd`).
  - `solve_color_parallel`: add cohort-snapping to the chunk-growth loop when `simd`; the inline (W1) path and the no-pool fallback call `solve_color_avx2` when `simd` (over the whole color's `(g_lo, g_hi)`), else `solve_color`.
  - `solve_colored`: pass `use_simd` into both `solve_all_colors` calls (it already computes `use_simd = config.simd`).
- **No change** to `contact.rs`, `math.rs`, `resources.rs` (scratch is stack-local), `systems.rs`, the plugin, or `PhysicsConfig` (the `simd` flag already exists, used by O1).

### Implementation plan (for the developer)

1. **`simd.rs`** — add the `#[cfg(target_feature="fma")] compile_error!` guard + module-doc no-FMA invariant paragraph (Decision 5). Add the `x8` math helpers: `cross8`, `dot8`, `mat3mulvec8` (3 `dot8`), `pointvel_x8` (lin + cross8(ang, r)), `effective_mass_x8` (the `angular` closure widened + `blendv(0, div(1,k), k>0)`), `apply_impulse_blend_x8(vel_regs, r, p, mask) -> vel_regs` (lin += blendv(0, p*invm, mask); ang += blendv(0, ii·(r×p), mask) — *but* invm/ii folded so a masked lane adds 0). All op-for-op vs `contact.rs`, NO FMA, left-to-right.
2. **`colored.rs`** — add `solve_color_avx2(cols, bodies_eff, g_lo, g_hi, group_start, bias_rate, mass_coeff, impulse_coeff, bias_active)` with the cohort/rank structure (Algorithm above): gather-once, rank-loop with register-carried velocity, per-rank impulse scatter via stack-staging + guarded scalar write, scatter-once at exit. Mirror O1's `cfg`+`target_feature` gating + per-`unsafe` SAFETY blocks. Add a scalar-fallback wrapper for non-AVX2/Miri that loops cohorts calling `solve_color` per group span (so the entry point is uniform) — OR simpler: the non-AVX2 dispatch just calls `solve_color` over the whole `(span)`. **Choose the latter** (one fallback = the oracle over the color span; bit-identical, simpler).
3. **`colored.rs`** — thread `simd: bool` through `solve_all_colors` + `solve_color_parallel`; wire the dispatch fork at all three call sites (non-parallel inline, parallel-inline W1, parallel-worker spawn, no-pool fallback). Add cohort-snapping to the chunk-growth loop under `simd`.
4. **`colored.rs`** — `solve_colored`: pass `use_simd` to both `solve_all_colors` calls.
5. **Tests** (see below) — co-located in `colored.rs` `#[cfg(test)]` (Miri-safe, no pool) + the parallel/bench ones in `tests/` / `benches/`.

## Metrics and validation

**Unit tests (mandatory, in `colored.rs` test module — native + Miri):**
1. **Test 1 — differential `solve_color_avx2 == solve_color`, bit-exact** (the INVIOLABLE-1 gate). Build a cohort of N groups (N = 1..16, crossing the 8-lane boundary and a partial trailing cohort) with **ragged widths** (a mix of 1,2,3,4-point groups in ONE cohort so high ranks have exhausted lanes). Run both the SIMD kernel and the scalar `solve_color` over the same columns/bodies; assert every `BodyEffective` (lin+ang) and every impulse column slot bit-identical (`to_bits`). Cover `bias_active ∈ {true,false}`. Randomized via the existing splitmix64 RNG (the `simd.rs` pattern).
   - **1c — mixed cone activation in one cohort** (resolves C3): construct a cohort with ≥1 clamped lane, ≥1 unclamped, ≥1 `len_sq==0` lane (zero tangent velocity), ≥1 denormal-`len_sq` lane; assert bit-exact. Non-vacuous mask coverage.
   - **1d — adversarial**: denormal velocities, `-0.0` components, a static-A guard lane, a sentinel-B lane, a `k≤0` (degenerate geometry) lane — all in one cohort, bit-exact.
2. **Test 2 — 0%-gate**: `solve_colored(simd=false)` produces byte-identical body+impulse state to the committed O6 (run twice, once on this branch with `simd=false`, compare to a captured O6 snapshot, OR assert `solve_color_dispatch(simd=false)` is literally `solve_color`). The 0%-gate is structural (dispatch fork) + verified by Test 1's `simd=false` arm == oracle.
3. **Test 3 — {1,N}×{simd} bit-identity**: solve a multi-cohort color with `{parallel=false, simd=true}`, `{parallel=true, 1 worker, simd=true}`, `{parallel=true, N workers, simd=true}`; assert all three bit-identical (and == `{parallel=false, simd=false}` scalar). In `tests/` (spawns a pool, native-only).
4. **Test 4 — warm-store cross-gate (O2)**: run `solve_colored` under each of `{O5 single-thread scalar, O6 parallel scalar, O7 simd}` over the same scene; assert the post-store `warm_read` table **bytes are identical** across all three (the canonical store is dispatch-independent). This catches a cohort/dispatch change leaking into the warm seed.

**Property tests:** extend Test 1 to a proptest over random cohort shapes (group count 1..32, per-group width 1..MAX_CONTACT_POINTS, random body masses incl. statics/sentinels, random velocities incl. denormals) asserting `simd == scalar` bit-exact. Failing case fully described by the seed.

**Benchmarks (criterion, in `benches/`):**
- **Bench A — kernel A/B**: `{simd colored}` vs `{scalar colored}` `solve_color` over a 10k-group color (width-1 and width-4 variants), target ≥ 3.5× on the kernel.
- **Bench B — whole-step A/B**: `{simd colored}` vs `{scalar colored}` on a 10k-box stack, target ≥ 1.6× wall.
- **Bench C — {parallel+simd} vs {parallel scalar}** (resolves W2/silent-width-degradation): on a large color, assert `{parallel+simd}` ≥ `{parallel scalar}` (else cohort-snapping mis-tuned).

**`debug_assert!` invariants:**
- `nlanes <= 8`, `cohort_lo + lane`'s group index `< g_hi` for active lanes.
- per-rank clamped slot `s < cols.len()` (gather in-bounds).
- at scatter, `ia[lane]/ib[lane] < bodies_eff.len()`.
- `g_width[lane] >= 1` for `lane < nlanes` (every group has ≥1 point — the build invariant), `g_width[lane] == 0` for `lane >= nlanes`.
- the cohort-snapped chunk boundary is a multiple-of-8 group offset from `g_lo` (parallel path).

## Restated lane=group bit-exactness proof (crisp)

> Within a color, every *dynamic* body lies in exactly one manifold-group (O4). A cohort packs 8 such groups, one per lane. Each lane runs *its group's* exact scalar `solve_color` op sequence (same IEEE round-to-nearest ops, no FMA, no rsqrt/rcp ⇒ bit-identical per op), carrying its A/B velocity in registers across ranks so point `p` sees point `p−1`'s update (intra-group Gauss-Seidel). Lanes share no written state — distinct lanes write disjoint dynamic body rows and disjoint impulse slots; shared statics/sentinels are never written (the `*_movable` guard). Therefore the 8 lanes' parallel evaluation equals solving the 8 groups sequentially in any order, which equals the scalar single-threaded colored solve, bit-for-bit. Masked exhausted/partial-cohort lanes compute garbage that every `blendv`/guarded-scatter discards (FP exceptions masked ⇒ Inf/NaN are inert data, never traps). Cohorts within a color are mutually disjoint ⇒ distributing cohort-runs across N workers is bit-identical to one worker. ∎ `{scalar,simd} × {1,N workers}` all agree.

## Residual risks (flagged)

1. **Register spill of the carried velocity** (12 ymm + 20 constant + per-rank temporaries > 16 ymm). If LLVM spills the *velocity* registers across the rank loop, correctness holds (the value is right) but the speedup degrades toward the per-rank-reload cost. **Mitigation**: the bench step inspects the kernel asm; if the velocity spills, split the kernel so the inv_inertia constants reload from stack per rank (they already do via the staging arrays) keeping velocity in registers. This is a perf-tuning risk, not a correctness risk — flagged for the developer/tester, not a design blocker.
2. **Ragged-cohort lane waste** in pathological width distributions (one width-4 group cohorted with seven width-1 groups → 4 ranks, 7 idle lanes at ranks 1–3). Real contact scenes are width-homogeneous-ish (all-floor = width-1, all-box-stack = width-4), so cohorts are mostly uniform-width; the masked waste is bounded and acceptable (Decision 4 trade-off — no bucketing to preserve canonical-store determinism). Bench B on a *mixed* scene quantifies it.
3. **Partial trailing cohort** (color with `n_groups % 8 != 0`) runs a < 8-lane batch — lower utilization for that one cohort per color. Negligible (one cohort/color); the masked kernel handles it correctly (permanently-inactive lanes).
4. **`a_static` (static-A) guard lane** is included for scalar-exactness (the scalar guards both sides) though A is conventionally dynamic. If a future manifold convention allows a static A, the guard already covers it (no change needed); flagged so the developer keeps the `ia_movable` blend even though it appears always-true today.

None of these are correctness blockers; (1) is the only perf risk and is bench-gated.

---

## Plan readiness checklist

**Structure**: goal in perf+functional terms ✓; metrics concrete (≥3.5× kernel, ≥1.6× step, 0 alloc, ~2 KB working set) ✓; every decision justified ✓; alternatives rejected with reasons ✓; trade-offs listed ✓.
**Data structures**: stack staging fields typed + sized ✓; no new persistent struct (reuses `ContactColumns`/CSR) ✓; working set sized (~2 KB, L1-fit) ✓; `repr` N/A (stack `[f32;8]` arrays, O1 pattern) ✓.
**API**: minimal (`pub(super)` kernel + dispatch, no public change) ✓; no internal type leak ✓; no `dyn` ✓; `cfg`+`target_feature` gated like O1 ✓.
**Multithreading**: model explicit (cohort-disjoint, no atomics, scope-barrier) ✓; data-race-freedom proof ✓; partitioning (cohort-runs) ✓; Send/Sync unchanged ✓.
**Correctness**: edge cases (ragged ranks, partial cohort, `len_sq==0`, denormal, sentinel, static, `k≤0`) enumerated ✓; bit-exactness proof restated ✓; trap-free proof ✓; unsafe invariants stated (gather/scatter bounds + disjoint-write) ✓.
**Integration**: affected modules listed ✓; API change (`simd` param threaded through `solve_all_colors`/`solve_color_parallel`) noted ✓; compat with CSR/`ContactColumns`/`ColorSolvePtrs` verified ✓; step plan ✓.
**Validation**: unit tests (differential + mixed-cone + adversarial) ✓; proptest ✓; benches (kernel A/B, step A/B, parallel+simd vs parallel-scalar) ✓; warm-store cross-gate ✓; debug_asserts listed ✓.

All resolutions R1–R6 + O1/O2 folded; INVIOLABLE-1..5 preserved. This is implementation-ready.

**Relevant files**: `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\colored.rs` (oracle `solve_color`, the cohort enumeration via `group_start`/`color_group_start`, `solve_color_parallel` cohort-snapping, the new `solve_color_avx2`), `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\simd.rs` (the `x8` math helpers + FMA guard, O1 kernel template), `D:\claude\BoykoEngine\crates\boyko_physics\src\solver\contact.rs` (`effective_mass`/`apply_impulse`/`is_dynamic_row` — the op-for-op widening source), `D:\claude\BoykoEngine\crates\boyko_physics\src\math.rs` (`Mat3::mul_vec`/`Vec3::cross`/`dot` op order).
agentId: a5a03a3d7446f2d25 (use SendMessage with to: 'a5a03a3d7446f2d25' to continue this agent)
<usage>subagent_tokens: 127750
tool_uses: 7
duration_ms: 315799</usage>