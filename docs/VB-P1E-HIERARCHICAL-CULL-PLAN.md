# VB-P1e — hierarchical froxel light cull (implementation plan, Rev 3)

**Status:** DESIGN, Rev 3 — **NOT APPROVED. DO NOT IMPLEMENT.** Rev 2 was returned by the
architecture-critic with **CHANGES REQUESTED (3 × P0, 5 × P1, 7 × P2)**. Rev 3 resolves all of them
against the real code and the real toolchain.

### Rev 3 verification outcome — 1 P0 + 5 P1 REMAIN OPEN

Rev 3 was reviewed by three independent adversarial lenses (proof soundness · out-of-bounds/UB ·
arithmetic-and-gates), each able to run the pinned `dxc`/`spirv-dis`, then consolidated. **All three
returned CHANGES REQUESTED.** The design is *converging, not thrashing* — the two hardest Rev 2 holes
(P0-A's evaluation-scheme premise, P0-C's clamp) are genuinely fixed, and the base `.spv` byte
reproduction was independently confirmed. The damage is concentrated in one layer.

**P0-1 — the out-of-bounds `ClusterGrid` write, the exact property D11 and the rewritten `// SAFETY:`
exist to establish, has NO gate that can turn red.** All three named detectors are provably blind, and
this was demonstrated by simulating the mutations against §4's own thread map: dropping the `valid`
guard on phase 6 writes **2688 cells (≈21 KB) past the buffer while every assertion stays GREEN**.
Assertion 5's sentinel proves *at-least-once*, never *exactly-once*; assertions 2/3 compare only
in-range cells; and the cited "validation ON reports the overrun" is unattainable on this stack —
`device.rs:2087` enables only `SYNCHRONIZATION_VALIDATION`, there is no GPU-assisted validation
anywhere in the repo, and `robustBufferAccess` is off. This is the same class that already shipped a
GPU-UB bug this campaign (VB-P1b C1). Fix: re-specify the mutation to also re-source the dims from the
live header; add a device-visible guard tail (allocate `capacity + G`, pre-fill, assert the tail
intact); add a uniqueness check; and restate assertion 5 as totality only.

**P1-1 — §5's NaN analysis argues against instructions this shader never emits.** Measured on the
committed module: **`FMin`/`FMax` = 0, GLSL.std.450 `NMin`/`NMax` = 26** (`NMin` 8, `NMax` 18) —
independently re-confirmed by the orchestrator. `NMin`/`NMax` return the *non-NaN* operand, so
`max(max(NaN,NaN),0.0)` yields `0.0` and the fine test **accepts** every light; §5's stated
alternative (the compare is false, so the group rejects everything) is unreachable, and the claimed
order-dependence is a property of `FMin`/`FMax`, not of the emitted instruction. A single NaN lane is
therefore *already* dropped from the fold, so the "144× blast radius" that justified the mitigation is
not established — and for the reachable all-NaN group the mitigation **inverts a conservative outcome
into a maximally divergent one** (unmitigated matches the flat arm; mitigated rejects every light for
all 144 froxels). Compounding it, §4 phase 5 gates the fine walk on `valid` rather than `contrib`, so
an excluded lane still tests against a coarse box that provably does not enclose it — §5's "for every
froxel in that group" is false for those lanes.

**P1-2..P1-5** — which `precise` placement actually ships (the plan's "16 ops including the AABB
construction" does not reproduce; measured 5 decorations for D10 as written, 7 with `precise float3 d`
and no leak into the AABB build); H2(e)'s two control-flow assertions are mis-specified and cannot
fire on what they guard (a deliberately-broken probe with an early `return` + a non-uniform barrier
emits **exactly 1 `OpReturn`**, same as the correct shader — DXC canonicalises to one exit block; and
the "barrier in a merge block" test both false-REDs a correct shader and false-GREENs a broken one);
mutations (vi) and (iv) are mis-specified, (vi)'s pre-registration being falsified in both directions;
and D11's "three evaluations of one `u32`" is not literal — the host allocates from full-precision
`cluster_count()` while the shader re-derives from three 8-bit fields guarded only by a `debug_assert!`.

### Rung disposition (from the consolidated verdict)

| Rung | Start now? | Why |
|---|---|---|
| **H0** timing bracket split | **YES** | Host-only; untouched by every finding; its hang warning verified correct |
| **H1** CPU oracle + selectivity gate | **YES** | Pure host arithmetic — and its coverage-totality assertion is exactly the check that would have caught P0-1 on the CPU; strengthen it to the guard-tail/uniqueness shape while writing it |
| **H1.5** transfer probe | **YES** | Existing flat arm, no new shader |
| **H1.6** `precise` + base re-pin | **NO** | Gated on P1-2/P1-1; re-pinning the base `.spv` twice is not acceptable |
| **H2 / H3 / H4 / H5** | **NO** | Gated on P0-1 and P1-3/P1-4 — these rungs *are* the gate-and-mutation layer that is broken |

Also required before approval, independently: §12's provenance commit (the §1.3 probe must land as a
real `#[test]`; it is currently untracked).

### What Rev 3 discharges

* **P0-A — §5's proof no longer needs the premise it used to disclaim. DISCHARGED by removing the
  ambiguity, not by budgeting for it.** The hole was real and deeper than "two sites might compile
  differently": `dot(d,d)` lowers to a single `OpDot`, and Vulkan's *Precision and Operation of
  SPIR-V Instructions* specifies `OpDot` only as **"inherited from"** a formula, with the same
  appendix permitting that formula to *"be transformed using the mathematical associativity,
  commutativity, and distributivity of the operators involved"*. Two `OpDot`s in one module may
  therefore be lowered to different summation orders — and a census over 9 modules emitted by the
  pinned dxc 1.4.350.0 found **zero `Fma`** in every one, so contraction is decided by the driver
  *below* the `.spv`, where no byte gate and no `spirv-dis` gate can observe it. **Fix (D10):** the
  one shared `sq_dist_point_aabb` computes a **written-out, `precise` sum** — three `OpFMul` and two
  `OpFAdd`, each specified as **"Correctly rounded"** and each carrying `NoContraction`. Both call
  sites then evaluate *one function `F`*, and the critic's missing link `A(d_fine) ≤ B(d_fine)`
  becomes vacuous because `A ≡ B ≡ F`. Verified: an unmodified copy of `cluster_cull.hlsl` compiled
  under the frozen recipe reproduces the committed `cluster_cull.comp.spv` **byte-for-byte**
  (12 392 B), and the `precise` form's blast radius on that shader is exactly **five added
  `OpDecorate NoContraction`** plus the one `OpDot` expanded — the 8 ray-gen `OpDot`s are untouched.
  A `spirv-dis` structural gate is retained in H2 as a **tripwire, never as the proof**.
* **P0-B — the lost total bound. DISCHARGED, and the naive fix was measured to be insufficient.**
  Transplanting the base arm's `fi < cluster_count` (live header) into D3's re-derived mapping still
  writes `fi = 13 807` into a **3 456-cell** `ClusterGrid` in the critic's own worked example, and
  `6 887` for a z-only grid growth. **Fix (D11):** the BOOT grid dims travel in a `#ifdef HIER`-only
  push tail word; the dispatch size, the `ClusterGrid` allocation and the in-shader write bound
  become **three evaluations of one u32** minted in `build_froxel_light_cull`, and `valid` gains a
  three-term predicate `(s < bdx·bdy) && (slice < bdz) && (fi < capacity)`. Measured: the HIER-only
  push member **plus a full HIER arm** leaves the base compile at the identical 12 392 B / sha256
  `dbb924967b1176af…`. Two hazards D8's "no early return" newly created are also closed: an integer
  **divide-by-zero** on a degenerate `packed_dims == 0` header (the base arm's `return` masks it
  today) and a `% dim_x` by zero.
* **P0-C — the unclamped coarse mask write and the index-space disagreement. DISCHARGED.** The mask
  is **l0a-relative** (bit `j` ⇔ table index `ps_begin + j`) — the convention two of the three sites
  already used. A single group-uniform clamp `ps_n = min(ps_total, ps_room)` bounds the groupshared
  **write** and both device **reads** simultaneously, with no device value in the derivation. D6's
  defensive tail is **deleted as unfixable**: it would `load_light` rows that do not exist in the
  `MAX_LIGHTS`-row table, relocating the out-of-range read rather than preventing it. The host pin
  becomes an **equality** (`MAX_LIGHTS == HIER_MASK_WORDS * 32`), which is what makes one clamp
  sufficient for both bounds. Compile-verified (both arms build, base byte-identical), SPIR-V
  structurally verified (no clamp instruction at the write site — the bound is the loop condition),
  and simulation-verified (20 000 randomized trials, 0 mismatches, all output ascending).
* **P1-D — NaN amplification. Rev 2's claim that this "is not a new hazard" was FALSE and is
  withdrawn.** Two code-reachable NaN sources are cited, and the first — a singular camera linear
  part producing a **finite** zero basis that the shader's unguarded `normalize` turns into NaN — is
  *invisible to any host finiteness assert*, so Rev 2's proposed mitigation did not even reduce its
  probability. The flat arm's blast radius is one froxel; the hierarchical arm's is one group (144
  froxels). Mitigated **on device** by folding "non-finite" into D8's existing invalid-lane identity
  substitution (measured: 6 SPIR-V ops/lane, not `isfinite`'s 10).
* **P1-E — H0's framegraph access. DISCHARGED by DELETING it.** The `LightIndexAlloc[0]` readback
  leaves the present path entirely; §9's "no new GPU-visible resource" claim becomes *true* instead
  of *defended*. All three consumers of `alloc_total` are already served with zero new production
  machinery.
* **P1-F — the test matrix was blind to its own mapping.** Every config Rev 2 named is 16×9×24, so
  `dim_x·dim_y = 144 < 256`, `gps = 1`, and D3's `(gid % gps)·256 + lane` degenerates to `lane`. A
  **transposed mapping is indistinguishable from the correct one** on the entire stated matrix. Four
  new grid entries added (E1–E4), two of which must run on device.
* **P1-G — the barrier count. Rev 2 said "three-to-eight"; §4's phase table summed to ELEVEN.**
  Since D1's rejection of the two-dispatch shape is argued *on fixed cost*, restating it as 11 would
  weaken D1 by ~3.7×. The design is changed instead: a **radix-16 in-place reduction** (2 barriers,
  not 9) and folding the summary bit into phase 3's atomic (deletes a phase and a barrier) bring the
  true count to **3**. Barrier elision by wave size is **not** available and is not assumed: the RHI
  sets `subgroup_size_control: VK_FALSE` (`device.rs:2584`) and queries `subgroupSize` nowhere.
* **P1-H — H1's overclaim.** "The whole perf premise is falsifiable on the CPU in 0.45 s" is
  withdrawn. H1 falsifies the **pair-count** premise — a *necessary* condition and the campaign's
  cheap kill switch — and nothing more. A new rung **H1.5** bounds thread-count scaling on the
  existing flat arm with no new shader; the sufficient condition is settled only at H4, against a
  prediction §7 now pre-registers.
* **Arithmetic.** All seven slips corrected and re-derived: §D3's `TPG=128` row is **48 groups /
  24 576 coarse / ≈41 900 total / 42×** (not 36 / 18 432 / 35 700 / 50×); §D3's `TPG=1024` row
  contradicted the section's own `gps` formula (24 groups, not 6) and is now printed as two labelled
  rows; §7.1's floor is **14.89**, published as a **band 12.90–16.87**; §1.2's "anchored on N=128 and
  N=512" was wrong for `flat_shade` (it is the 8/512 secant) and imprecise for `cull` (rounded rate,
  re-fitted intercept) — both anchorings are now stated separately and are reproducible with a
  calculator; §D3's `f(nz)` premise does not hold at the default grid and is restated as a ratio
  (which makes the conclusion **stronger**: 3.85× / 29.1×, not 1.93× / 7.27×); §D9/§9's groupshared
  figure was unsupported by the emitted SPIR-V and is fixed **by construction** (scalar arrays,
  6 276 B exactly); §D3's occupancy sentence omitted that 4 of 28 SMs go idle.

### What remains open

1. **Nothing here has run on a GPU.** Every P0/P1 resolution above is a compile-time, disassembly or
   CPU-simulation result. The device claims are explicitly deferred to H1.5/H3/H4 and each carries a
   named closing test.
2. **The D10 edit perturbs the base arm by ≤1–2 ULP and requires a one-time re-pin of
   `cluster_cull.comp.spv`.** D5's "base `.spv` byte-frozen" title is no longer achievable and has
   been restated. Rung **H1.6** isolates that perturbation from the hierarchical change.
3. **The ALU cost of D10 is unmeasured** (2 extra ops per pair test on a path whose wall clock
   tracks pair count). H1.6 measures it; the fallback if it regresses is named and its constant
   derived.
4. **§1.3's provenance does not exist in the repository.** Rev 2 anchored the document's
   self-declared "single most important table" on `scratchpad/cap_probe.rs.txt`; verified absent
   (`git ls-files | grep -i cap_probe` → no match, `scratchpad/` is not a tracked path). **A commit
   landing that probe as a `#[test]` must precede Rev 3's approval** — see §12.
5. **The hot-group latency model is not written down.** §7's µs column is an aggregate-throughput
   bound; at `N=512` three of 24 groups carry 85.2 % of the fine work, so wall clock is one group's
   serialized latency. §7 now says so and pre-registers what H4 must hit under each reading.
6. **A pre-existing latent hole in the BASE arm**, found while sizing P0-B: its total bound is
   `min(64·ceil(boot_cc/64), live_cc)`, which exceeds `boot_cc` whenever `boot_cc % 64 != 0` **and**
   the live dims grow — measured at 16 cells (128 B) past the end of `ClusterGrid` for boot 16×9×23 /
   live 16×9×24. It needs two owner actions where D3-as-written needed one, and is bounded by 63
   cells where D3 was unbounded. Tracked as **VB-P1j** (§11); it is not fixed here because it
   requires its own base `.spv` re-pin decision.

Sub-plan of
[VB-PERFORMANCE-TRACK.md](VB-PERFORMANCE-TRACK.md) §4 (VB-P1). Sibling of
[VB-P2-CLASSIFICATION-PLAN.md](VB-P2-CLASSIFICATION-PLAN.md). Base commit `5e07936`
(`feat/multi-paradigm-render`).

**One-line verdict:** the cull is *pure rejection work* — at `N_ps=512` only **85 of 3456 froxels
(2.5 %)** hold any light, yet all 3456 threads scan all 512 lights and the pass costs **498 µs**. A
**single-dispatch, workgroup-local two-level cull** removes ~95 % of the (froxel, light) pair tests
with an **exactness proof that needs no floating-point epsilon**, and it introduces **no new GPU
buffer, no second dispatch, and no new framegraph resource**. Predicted cull at `N_ps=512`:
**≈ 23 µs (nominal) / ≈ 50 µs (2× pessimistic)**, break-even **≈ 17–30** (from the measured ≈ 103).
**Single-digit break-even is arithmetically impossible at the present fixed-cost floor** — §7.1
proves it across every fit and every point in the error bar, and names the follow-up that would be
needed. **The pair-count premise — a necessary condition, and the campaign's cheap kill switch — is
falsifiable on the CPU in 0.45 s at rung H1. The sufficient condition (wall clock) is not:
thread-count scaling is bounded at H1.5 with no new shader, and the rest is settled only at H4.**

---

## 1. The problem (measured, not assumed)

`crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` dispatches **one thread per froxel**
(`[numthreads(64,1,1)]`, `:107`; the host dispatches `ceil(cluster_count / 64) = 54` groups,
`present/passes/vb.rs:184`). Each thread builds its world AABB from 8 corner unprojections
(`:126-153`) and then **linearly scans every point/spot light** (`:161-175`), testing
`sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r*r` (`:102-105`). Cost is `O(froxels × lights)`.

VB-P1d measured it on RTX 3060 through GPU timestamps
(`crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`; the table is committed as the provenance
doc-comment on `CLUSTER_LO`, `crates/boyko_render/src/light_policy.rs:44-63`):

| `N_ps` | `flat_shade` ns | `froxel_cull` ns | `froxel_shade` ns | `froxel_total` ns |
|---|---|---|---|---|
| 8   | 32 799  | 19 741  | 27 075 | 46 816  |
| 32  | 60 815  | 42 253  | 29 747 | 71 999  |
| 64  | 95 877  | 72 748  | 29 973 | 102 720 |
| 128 | 167 322 | 134 920 | 28 119 | 163 039 |
| 256 | 315 044 | 252 154 | 25 508 | 277 662 |
| 512 | 592 015 | 498 067 | 25 303 | 523 370 |

Read it correctly: **`froxel_shade` is FLAT in `N` (25–30 µs) — the clustering payoff is already
fully realized in the shade.** The cull is 95 % of `froxel_total` at `N=512` and is the entire reason
the break-even sits at ≈ 103 instead of near zero. `CLUSTER_LO=64` / `CLUSTER_HI=128`
(`light_policy.rs:64,74`) is the measured hysteresis band.

### 1.1 A disproven hypothesis — do NOT repeat it

A prior attempt theorised the `uint local[256]` per-thread array (`cluster_cull.hlsl:159`) was
spilling to scratch and dominating. A two-pass count-then-write rewrite eliminating `local[]` was
implemented and measured: **cull 8 µs → 24 µs, total 498 µs → 1014 µs (2.04× REGRESSION)**, output
byte-identical. Reverted. That experiment is not just a dead end — **it is the strongest confirmation
of the cost model**: it doubled the number of (froxel, light) pair tests and the time went up 2.04×.

> **The cull's wall-clock is proportional to the number of (froxel, light) pair tests, and to
> nothing else we have been able to move.** Register/scratch pressure is refuted by measurement.
> The only lever with evidence behind it is *reducing the number of pairs tested.*

This is also the standing rule Rev 3 applies to *itself*: every number below that has not been
measured is labelled a model, and every rung that could have asserted a hypothesis instead measures
it (H0, H1.5, H1.6, H4).

### 1.2 The empirical cost model (fitted, ≤ 8.9 % error over 6 samples)

```
cull_ns(N)       ≈ 13 939 + 0.2736 · (froxels · N)   = 13 939 + 945.6·N   at froxels = 3456
flat_shade_ns(N) ≈ 23 922 + 1 109.6·N
froxel_shade_ns  ≈ 26 500 ± 2 200   (no trend in N)
```

**The two fits are anchored differently, and Rev 2 mis-stated both. Corrected, so a reader with a
calculator can reproduce them from the table above:**

* `flat_shade` is the exact **`N=8` ↔ `N=512` secant**: slope `(592 015 − 32 799)/504 = 1 109.5556`,
  intercept `32 799 − 8·1 109.5556 = 23 922.56`. It is *not* the 128/512 fit — the error column
  proves it, since only the `N=8` and `N=512` rows read 0 %.
* `cull` is **neither fit cleanly**. Its slope is the **128 ↔ 512** secant expressed per pair —
  `(498 067 − 134 920)/384 = 945.6953 ns/N`, i.e. `945.6953/3456 = 0.273638 ns/pair` — **rounded to
  `0.2736`**; the intercept is then re-fitted so `N=512` is exact *using the rounded rate*:
  `498 067 − 0.2736·3456·512 = 13 939.5`. The error table below evaluates at `945.6·N`, which is why
  `N=512` reads 498 086 (0 %) and `N=128` reads 134 976 (+0.04 %).
* For reference, the exact 128/512 secants are `cull = 13 871 + 945.70·N` and
  `flat = 25 758 + 1 106.0·N`. **§7.1 is robust to either choice** (floor 14.89 vs 13.21) — see
  there.

| `N` | model cull | measured | err | model flat | measured | err |
|---|---|---|---|---|---|---|
| 8   | 21 504  | 19 741  | +8.9 % | 32 799  | 32 799  | 0 % |
| 32  | 44 198  | 42 253  | +4.6 % | 59 429  | 60 815  | −2.3 % |
| 64  | 74 457  | 72 748  | +2.3 % | 94 936  | 95 877  | −1.0 % |
| 128 | 134 976 | 134 920 | +0.04 % | 165 951 | 167 322 | −0.8 % |
| 256 | 256 013 | 252 154 | +1.5 % | 307 980 | 315 044 | −2.2 % |
| 512 | 498 086 | 498 067 | 0 % | 592 037 | 592 015 | 0 % |

Two constants matter for everything below:

* **`0.2736 ns` per (froxel, light) pair test** — the marginal cost.
* **`13.9 µs` fixed cost per cull invocation** — independent of `N`. It is *not* the 3456 AABB
  builds (8 unprojections × 3456 threads ≈ 0.8 Mflop ≈ 0.1 µs at peak). The `LightCull` timestamp
  bracket (`present/passes/vb.rs:140-215`) spans `cmd_fill_buffer(alloc)` → graph-derived
  `TRANSFER→COMPUTE` barrier → dispatch, so the fixed cost is almost certainly **fill + pipeline
  barrier + dispatch ramp**. **This is a hypothesis, and rung H0 measures it instead of assuming it**
  (§1.1's lesson applied to our own reasoning).

> **Model validity caveat.** `0.2736 ns/pair` is calibrated on *this* dispatch shape (3456 threads,
> warp-uniform light index, balanced across all 54 groups). The hierarchical arm changes the shape
> (6144 threads, lane-varying light index in the coarse phase, group-uniform candidate list in the
> fine phase, deliberately *imbalanced* across groups). The model is used below for *sizing
> decisions and go/no-go bounds only*. **Rung H1.5 tests the one part of that transfer which can be
> tested without writing a shader** — whether the rate is froxel-count-invariant, i.e. whether the
> pass is throughput-bound or has a latency floor. The shipping decision is a measurement (H4), and
> the abort threshold in §10 is expressed in measured nanoseconds.

### 1.3 The occupancy profile (CPU probe)

A CPU probe over the repo's own host oracle `golden_cluster_cull`
(`crates/boyko_rhi_vulkan/src/goldens.rs:3510`), replicating the VB-P1d camera (eye `(0,1.1,7.8)` →
`(0,0.55,0)`, `fov_y 52°`, aspect 1.0, 512×512) and its procedural rig
(`vb_p1d_cull_shade_bench.rs:124,142`) against the default `ClusterConfig` (16×9×24 = 3456 froxels,
`z_near 0.1`, `z_far 50.0`, `MAX_LIGHTS_PER_CLUSTER 256`, `INDEX_LIST_CAP 16384`):

| `N_ps` | total indices | % of 16384 cap | non-empty froxels | max per froxel |
|---|---|---|---|---|
| 8    | 789  | 4.8 %  | 514 | 3 |
| 14   | 1239 | 7.6 %  | 543 | 5 |
| 32   | 1916 | 11.7 % | 557 | 10 |
| 64   | 2063 | 12.6 % | 364 | 15 |
| 128  | 1654 | 10.1 % | 143 | 24 |
| 256  | 2072 | 12.6 % | 115 | 40 |
| 512  | 2597 | 15.9 % | 85  | 64 |
| 1024 | 2709 | 16.5 % | 55  | 109 |

**This is the single most important table in the document.** At `N=512` the cull performs
`3456 × 512 = 1 769 472` pair tests and **2 597 of them succeed — 0.147 %**. The pass is 99.85 %
rejection work, and 97.5 % of froxels are empty. A level that rejects *whole blocks of froxels
against whole ranges of lights* attacks exactly that.

**Provenance — and a defect Rev 2 shipped.** Rev 2 anchored this table on
`scratchpad/cap_probe.rs.txt`, a session-ephemeral file that **is not in the repository** (verified:
`git ls-files | grep -i cap_probe` returns nothing; no tracked path contains `scratchpad`). Since
§7's entire fine-pair column, §6's saturation discharge and §10's abort criterion rest on this
table, prose cannot re-derive it. **§12 specifies the commit that lands it as a `#[test]`, and that
commit must precede Rev 3's approval.** After it lands, this table is a pin, not a one-off print, and
this paragraph is re-anchored on the test's path.

### 1.4 A defect in the bench rig that bounds what we may claim

`light_position` (`vb_p1d_cull_shade_bench.rs:124-137`) claims its three Kronecker multipliers
(`g = 0.618033988750`, `g² = 0.381966011250`, `g³ = 0.236067977`) are "mutually irrational, so the
sequence never repeats/aliases across the three axes". **That claim is false, and provably so:**

* `g + g² = 1` exactly ⇒ `frac(i·g²) = 1 − frac(i·g)` for every non-integral `i·g` ⇒ **`fy = 1 − fx`.**
* `g³ = g − g²` ⇒ `frac(i·g³) = frac(2·i·g)` ⇒ **`fz = frac(2·fx)`.**

So the "3-D low-discrepancy volume fill" is a **one-dimensional locus**: with `fx` sweeping `[0,1)`,
`x`, `y` and `z` are each affine in `fx` on each of the two halves `fx < ½` and `fx ≥ ½`. **All
`N_ps` lights lie on exactly two straight 3-D segments.** This explains §1.3 exactly: the segments
run diagonally out of the frustum, so as `N` grows (and the placement volume grows as `cbrt(N/14)`)
the lights are pushed laterally outside the view cone — `non-empty froxels` collapses 514 → 55 while
`max per froxel` climbs 3 → 109. The doc-comment's "keeps the AVERAGE per-froxel light density
roughly constant" is refuted by its own data.

**Numerically verified** (over `i ∈ [1, 1024]` at the literals the source uses):
`g + g² = 1.0` exactly; `g − g² = 0.23606797750000003` vs the source's `g³ = 0.2360679775`;
`fy = 1 − fx` and `fz = frac(2·fx)` hold with **0 violations / 1024** at a max deviation of
`1.99e-13` (pure float round-off). The dependency is real, not an approximation.

**Consequences (all binding on this plan):**

1. The high-`N` rows of the VB-P1d table measure a scene whose lights are *mostly out of frustum*.
   The hierarchy's win on this rig is a **best case** (rejection-dominated).
2. VB-P1e must therefore report **two rigs**: the existing one (unchanged — it is the provenance of
   `CLUSTER_LO`/`CLUSTER_HI`) *and* a new in-frustum rig where lights stay inside the view volume as
   `N` grows (§8, H4).
3. `CLUSTER_LO=64`/`CLUSTER_HI=128` were calibrated on this rig and may shift for dense in-frustum
   scenes. **VB-P1e does not re-tune them** — it publishes new numbers and flags the re-tune as
   VB-P1f (§11). Consequence to state openly: in `Auto` mode a 64 < `N` < 128 scene keeps the flat
   path and sees no VB-P1e benefit until VB-P1f lands.

---

## 2. Goal

Make the cull's pair count **sublinear in the froxel count** (not in `N` — see §7.1), so the
break-even collapses and the froxel path wins across a far wider range.

**Success is defined numerically (§10 restates these as the abort criterion):**

| Metric | Today | Required to ship |
|---|---|---|
| `froxel_cull_ns` @ `N_ps=512`, existing rig | 498 067 | **≤ 250 000** (≥ 2×; predicted ≈ 23 000) |
| `froxel_cull_ns` @ `N_ps=64`, existing rig | 72 748 | **≤ 72 748** (no regression) |
| `froxel_cull_ns` @ `N_ps=8`, existing rig | 19 741 | **≤ 21 700** (≤ +10 %, the fixed-cost floor) |
| break-even (`froxel_total < flat_shade`) | ≈ 103 | **≤ 40** measured (predicted 17–30) |
| per-froxel index SET vs the flat arm | — | **exactly equal**, order included (§9) |
| `vb_mesh_froxel` / `vb_mesh_tex_froxel` pins | green | **byte-identical, no re-pin** |

Note the one target Rev 3 *removes*: the base `cluster_cull.comp.spv` is **not** byte-frozen across
the whole rung. D10 changes the shared distance function, so that blob is re-pinned exactly once, at
H1.6, and is byte-frozen from that commit onward (D5).

---

## 3. Key decisions

### D1 — Two levels, gather-side, **inside one dispatch, in groupshared memory**

**What.** Keep one thread per froxel. Make the **workgroup** the coarse cell: the group's threads
first co-operatively reduce their own froxel AABBs into a **group AABB**, test the light table
against *that* once (striped across all lanes), record the survivors as a **groupshared bitmask**,
and only then run today's exact per-froxel test over the mask's set bits.

**Why.**
* The coarse level costs `groups × N` pair tests instead of `froxels × N` — a `froxels/groups`
  reduction on the rejection work (144× at the chosen size, §D3).
* **No second dispatch.** §1.2's fixed cost is ≈ 13.9 µs *per cull invocation*; a separate coarse
  dispatch would plausibly add another one, which at low `N` is the entire budget. A groupshared
  hierarchy adds **zero** dispatch-level overhead.
* **No new GPU buffer** ⇒ no new framegraph resource, no seeding decision, no cross-frame WAR
  surface, no stale-data hole `[P0-1]`, `[P0-4]`. Rev 3 makes this claim *true* rather than
  *defended* by removing H0's `LightIndexAlloc` readback from the present path (§P1-E, H0, §9).
* Groupshared is per-dispatch by construction: the "stale mask" failure mode cannot exist.

**Alternatives rejected.**
* *A coarse pass writing a global bitmask buffer, consumed by a second dispatch* (Rev 1's shape).
  Rejected on the fixed cost above, plus it would need a new `add_buffer_seeded` resource, an
  unconditional every-word-write totality argument, and a cross-TU FP-margin proof. Every one of
  those problems is *deleted*, not solved, by moving the level into the group.
* *Screen-space XY column hierarchy (one coarse cell per (x,y) over all z).* Rejected on numbers:
  the z-slab is the dominant discriminator in this scene (§1.4 — the in-frustum lights occupy 3 of
  24 slices); collapsing z makes the coarse AABB span `view_z ∈ [0.1, 50]` and reject nothing.
* *Light-centric scatter (rasterize each light's sphere into the froxel grid).* Genuinely
  output-sensitive (≈ 2 600 writes instead of 1.77 M tests at `N=512`) and it is what Doom-2016-class
  clustered pipelines do — **but** it produces per-froxel lists in atomic order, and per-froxel
  **table order is load-bearing**: the shipped flat-vs-froxel equality golden
  (`vb_mesh_froxel.rs`, `BOYKO_VB_FROXEL_FORCE_OFF`) holds only because the froxel list is the flat
  loop's order with exact-zero contributions elided (`x + 0.0 == x`), and FP addition is not
  associative. Restoring table order requires a per-froxel bitmask plus a compaction pass, whose
  cost (`froxels × ceil(N/32)` word scans) lands within a few percent of D1's total anyway. Rejected
  as strictly more machinery for no modelled gain and a live regression risk to a shipped gate.
* *`WaveActiveMin`/`WaveActiveBallot` (SM 6.0 wave intrinsics) instead of groupshared.* Would remove
  every barrier, but requires `VK_KHR_shader_subgroup_ballot`/`_arithmetic` support in COMPUTE, which
  this raw-FFI engine does not query today. Rejected for portability; re-openable as a measured
  follow-up (§11) because it is output-neutral by D4.

**Trade-off (corrected — Rev 2 said "three-to-eight" while its own phase table summed to eleven).**
**Three** `GroupMemoryBarrierWithGroupSync()` per group:

* **B1** after the per-lane AABB store + mask init;
* **B2** after the radix-16 in-place fold (D9);
* **B3** after the coarse mask/summary publish (§4 phase 4).

…plus a strict uniform-control-flow obligation (§D8) — the single most likely implementation bug in
the rung. The count is stated **once**, here, and §4 carries a `Barriers: 3 total` footer so the two
cannot drift apart again.

**The barrier count is not reducible by wave-synchronous elision, and this plan does not assume it
is.** The RHI enables no subgroup feature (`crates/boyko_rhi_vulkan/src/device.rs:2584` —
`subgroup_size_control: VK_FALSE`) and queries `subgroupSize` nowhere (a grep over
`crates/boyko_rhi_vulkan/src` + `crates/boyko_rhi/src` returns only raw FFI field declarations at
`ffi.rs:2623,2624,2691,2703` and that one `VK_FALSE`). Without an enabled subgroup guarantee,
dropping the tail steps of a reduction on an assumed wave width is UB under the Vulkan memory model,
and NVIDIA's post-Volta independent thread scheduling has made the idiom unsound even where it once
worked. **The portable lever is fewer reduction *steps*, not skipped barriers** — which is exactly
what D9's radix-16 fold buys (9 barriers → 2).

### D2 — The coarse AABB is the **componentwise min/max of the children's own AABBs**

**What.** The group AABB is *not* recomputed from block-corner geometry. It is a reduction over the
values each lane already computed for its own froxel.

**Why.** It makes the conservative-enclosure property a **tautology, exact in IEEE-754, with no
epsilon and no dilation** (§5 proves it). This is the direct discharge of the critic's `[P1]`
("D4 reduces to an unmeasured inequality"): there is no second computation of the same geometric
quantity, hence no discrepancy to bound. It is also *cheaper* than an independent coarse AABB build
(no extra unprojections).

**Corollary (very strong, state it in the shader header):** **any** assignment of froxels to groups
is correct. The grouping affects performance only — never the output set. This removes the entire
class of "is the block decomposition conservative?" review questions, including for partial blocks,
ragged grid dimensions, and hardware-dependent wave sizes. It also covers D9's change of reduction
*shape* from a halving tree to a radix-16 fold: `min`/`max` remain exactly associative and
commutative, so §5 Step 1 is untouched.

**Trade-off.** The coarse AABB is the union bound of the children's *AABBs*, which is slightly
looser than the true block hull; irrelevant, since the fine test is unchanged and exact.

### D3 — Group = 256 threads = one z-slice of the froxel grid (at the default 16×9×24)

**What.** `TPG = 256`. `gps = ceil(dim_x·dim_y / 256)` groups per z-slice; total groups
`= gps · dim_z`. For the default grid: `gps = ceil(144/256) = 1`, **24 groups**, 6144 threads
(3456 valid + 2688 idle-for-fine-work but fully used by the coarse phase).

**Thread→froxel map (rewritten for P0-B — every dim comes from the BOOT push, never the live
header; see D11):**

```hlsl
uint bdx = pc.cluster_dims_packed & 0xFFu;
uint bdy = (pc.cluster_dims_packed >>  8) & 0xFFu;
uint bdz = (pc.cluster_dims_packed >> 16) & 0xFFu;
uint capacity = bdx * bdy * bdz;                    // == the ClusterGrid element count
uint gps   = max(1u, (bdx * bdy + 255u) / 256u);    // max(1) => OpUDiv can never divide by 0
uint slice = gid.x / gps;
uint s     = (gid.x % gps) * 256u + lane;
uint x = (bdx != 0u) ? (s % bdx) : 0u;              // % 0 is UB on a degenerate header
uint y = (bdx != 0u) ? (s / bdx) : 0u;
uint z = slice;
uint fi = cluster_linear_index(x, y, z, bdx, bdz);  // light_table.hlsli:329 — UNCHANGED
bool valid = (s < bdx * bdy) && (slice < bdz) && (fi < capacity);
```

`fi` is computed **unconditionally** — it is pure uint arithmetic and touches no memory, so no lane
skips a barrier (D8). `valid` is a data predicate guarding phases 1, 5 and 6. The HIER arm **does
not call `load_cluster_params` at all**; it consults the live header only for
`l0a_count`/`light_count` via `load_light_header`, exactly as the base arm does.

*Why all three terms are load-bearing.* `s < bdx·bdy` bounds `y` (`x = s % bdx` is bounded by
construction). `slice < bdz` bounds `z`; without it two distinct `(gid, lane)` pairs alias onto one
in-range cell — a silent double-write of `ClusterGrid[fi]`, measured at **3 432 aliased cells** in
one skew case. `fi < capacity` is the hard device-write bound and the only term naming the buffer's
real size. With push-sourced dims the first two already imply `fi < bdx·bdy·bdz` algebraically
(`fi ≤ (bdx·bdy − 1)·bdz + bdz − 1 = capacity − 1`), so the third is a cheap, locally auditable
restatement — kept in D7's spirit, and *load-bearing* the moment anyone re-sources the dims from the
header. No product can wrap: every dim is 8-bit packed (`light_table.hlsli:317-319`,
`ClusterConfig::packed_dims` `light.rs:763-769`), so `bdx·bdy ≤ 65 025` and `capacity ≤ 16.6 M`, all
< 2²⁴.

**Why this shape (arithmetic, corrected).** For a block of `nz` z-slices, with exp-Z ratio
`q = (far/near)^(1/dim_z) = 500^(1/24) = 1.2955587`, the block's depth extent scales as
`q^{3nz}·(1 − q^{−nz})`:

| `nz` | 1 | 2 | 4 |
|---|---|---|---|
| depth term | **0.4961** | 1.9115 | 14.424 |
| ratio vs `nz=1` | 1× | **3.85×** | **29.1×** |

Rev 2 multiplied this by a screen-footprint term `64/nz`. That constant is **unsourced** — it
matches neither `TPG` (256) nor `dim_x·dim_y` (144) — and the premise behind it does not hold here:
the `TPG/nz` footprint-shrink term applies only where `dim_x·dim_y > TPG` (i.e. `gps > 1`), and at
the default 16×9×24 with `TPG=256` a group already covers a whole slice, so the footprint is
constant in `nz` and drops out of the ratio. **Removing it makes the conclusion stronger, not
weaker**: `nz=2` is 3.85× worse (not 1.93×) and `nz=4` is 29.1× worse (not 7.3×). Exp-Z means depth
extent grows *geometrically* with slice count, so **single-slice blocks are decisively best**, and a
4-slice block stops rejecting anything at the far slices. Note also that `nz ≥ 2` **is not
expressible under this section's own thread→froxel map** at `TPG=256`, so this table is a sanity
check on the map rather than a live design fork.

Given `nz=1`, the remaining choice is how many froxels of the slice per group. Modelled totals at
`N=512` on the measured occupancy profile (§1.3), with `gps` printed so every row is checkable
against the formula above (`gps = ceil(144/TPG)`, `groups = gps·24`, flat = 1 769 472):

| `TPG` | `gps` | groups | coarse pairs (`groups·N`) | fine pairs (est.) | total | vs flat |
|---|---|---|---|---|---|---|
| 64  | 3 | 72 | 36 864 | ≈ 12 000 | ≈ 48 900 | 36× |
| 128 | 2 | **48** | **24 576** | ≈ 17 300 | **≈ 41 900** | **42×** |
| **256** | **1** | **24** | **12 288** | ≈ 20 000 | **≈ 32 300** | **55×** |
| 1024 (`nz=1`, *this section's map*) | 1 | **24** | **12 288** | ≈ 20 000 | ≈ 32 300 | 55× |
| 1024 (`nz=4` block map — **not expressible** by the `gps` formula; retained only to show the `nz=4` penalty) | — | 6 | 3 072 | ≈ 1 700 000 | ≈ 1 700 000 | 1.04× |

Rev 2's `TPG=128` row said 36 groups / 18 432 / 35 700 / 50×; the correct row is **48 / 24 576 /
41 900 / 42×**, which **widens** `TPG=256`'s margin (42× → 55×, not 50× → 55×). Rev 2's `TPG=1024`
row ("6 groups, spans 4 slices") is arithmetically self-consistent only under an `nz=4`
decomposition the section had already excluded two paragraphs earlier — it contradicts
`ceil(144/1024)·24 = 24`. Both readings are now printed, labelled.

`TPG=256` also wins under the *opposite* (uniform in-frustum) assumption — a Steiner/Minkowski
model over a frustum-filling light field gives `TPG=64: ≈ 65 900` vs `TPG=256: ≈ 52 600` pairs —
because the coarse term shrinks 3× while the fine term grows only 1.39×.

**Occupancy (corrected — Rev 2 omitted the machine it gives up).** 24 groups × 8 warps lands 8 warps
on **24 of the RTX 3060's 28 SMs and leaves 4 SMs idle**; today's `ceil(3456/64) = 54` groups × 2
warps feed **all 28** (26 SMs × 4 warps, 2 SMs × 2 warps). The hierarchical shape therefore trades
**14 % of the machine** for 2× the warps per active SM — a net win only if the pass is
latency-bound, which **H1.5 measures**. (Confirm the 28-SM figure from `VkPhysicalDeviceProperties`
at H0 rather than carrying it as an assumed hardware constant.)

**Alternatives rejected.** `TPG=64` (would keep `[numthreads(64,1,1)]` and the existing
`LIGHT_CULL_LOCAL_SIZE_X` constant untouched) — rejected: 3× the coarse cost for a modelled 1.35×
worse total. `TPG=1024` under this section's own map — rejected **not** on pair count (it ties
`TPG=256` exactly, same one-slice coarse box) but on three other grounds: 880 of 1024 lanes idle
(86 %), **32.9 KB** of groupshared (`2 × 1024 × 16 + 132`), which exceeds the Vulkan-required
minimum `maxComputeSharedMemorySize` of 16 384 B — and the engine queries that limit **nowhere** (a
grep for `max_compute_shared_memory` over `crates/` returns no hits), so nothing would catch it at
runtime — and 1 group/SM. Blocks spanning ≥ 2 z-slices — rejected by the depth-term table. A 3-D
block (e.g. 8×4×2) — modelled within 5 % of 16×4×1 and strictly worse than one-slice-per-group, and
it needs a 3-D delinearization for no gain.

**Trade-offs (three, all stated).**
1. 2688 of 6144 lanes hold no froxel at the default grid (**43.75 %**). They cost nothing in phases 1
   and 5 (guarded by `valid`), and in phase 4 they are **productive** — the coarse light scan is
   striped across all 256 lanes regardless of `valid`.
2. With 24 groups on 28 SMs the dispatch is deliberately *imbalanced* (3 hot groups, 21 near-empty
   at `N=512`); wall clock is then set by the hot group, which is exactly the work that cannot be
   avoided. §7 pre-registers what that means for H4.
3. **Write locality changes, and neither H1 nor H1.5 can see it.** Today `fi = tid.x` and
   `cluster_linear_index = (y·dim_x + x)·dim_z + z` (`cluster_cull.hlsl:116-121`,
   `light_table.hlsli:329`), so consecutive lanes write consecutive `fi` — a 64-lane group writes 512
   contiguous bytes (8 cache lines). Under this map consecutive lanes share `z` and stride `fi` by
   `dim_z = 24`, i.e. **192 B apart**, so a 256-lane group touches 256 distinct lines. Worst case the
   full-grid `ClusterGrid` write traffic goes 27 KiB → 216 KiB. At Ampere bandwidth that is well
   under 1 µs against a 23 µs target and L2 will coalesce most of it, so it does **not** threaten the
   design — but it is dispositioned here rather than discovered at H4, and it is why §4's
   "token-identical" claim for phase 6 is scoped to the *source text*, not the access pattern.

### D4 — Output is bit-identical to the flat arm (under the stated scope), by construction

Four properties, each independently required:

1. **Same set** — the coarse level never rejects a light the fine test would accept (§5, exact,
   and — new in Rev 3 — with its evaluation-function premise *discharged* by D10 rather than
   disclaimed).
2. **Same order** — the fine loop walks `gs_summary` with `firstbitlow` ascending and, within a
   word, `gs_mask[w]` with `firstbitlow` ascending, so `j = (w<<5)|b` is strictly ascending and
   `i = ps_begin + j` is strictly monotone in `j` ⇒ **table order**, identical to today's
   `for i in [l0a_count, light_count)`. The coarse phase's striped `InterlockedOr` order does not
   matter: bitwise-OR is commutative and associative, so the final mask is deterministic. This is
   what preserves the shipped flat-vs-froxel equality golden.
3. **Same clamp** — `max_lights_per_cluster` truncates the same ascending prefix (`:170`). Preserved
   because the fine walk visits a *subset* of the flat loop's indices that still contains every
   ACCEPTED index (§5), applies the token-identical predicate and the token-identical
   `nlocal < pc.max_lights_per_cluster && nlocal < 256u` guard, and never `break`s early — so both
   arms keep the same ascending prefix of the same accepted set.
4. **Different slice offsets only** — the global `InterlockedAdd` claim order changes, so
   `ClusterGrid[fi].offset` differs. Offsets do not affect any shaded pixel; the resolve reads
   `[offset, offset+count)`.

**Verified by simulation** (20 000 randomized trials sweeping `l0a_count ∈ {0,1,2,3,7,31,32,33,100,
512,1000,1023}`, point/spot spans up to the 1024 budget, caps `∈ {1,2,5,256,∞}`, with a conservative
coarse superset and injected non-punctual rows): **0 mismatches**, every emitted sequence equal to
its sorted form, mask word 31 exercised in 196 runs.

**Scope of the claim (five clauses — Rev 2 had four).**

* **(a)** Non-saturating configurations only — under global-cap saturation the arms may legitimately
  differ in *which* froxel loses its tail (§6).
* **(b)** `boot_dims == live_dims` — the no-skew precondition (D11). Under skew the HIER arm's `fi`
  is still in-bounds and total, but its geometry no longer matches what the resolve (which reads the
  live header) expects; that is the pre-existing skew class, and H3 asserts the precondition loudly
  like §6's `alloc_total`.
* **(c)** **Finite AABBs only.** Under a non-finite AABB the arms may legitimately differ for the
  affected froxel and neither arm's result is meaningful (§P1-D, §5). The D8 identity substitution
  bounds the *damage* to one lane; it does not restore equality.
* **(d)** Both arms compiled from the same `sq_dist_point_aabb` (D10). This is why D10 must change
  the **shared** function and cannot be an `#ifdef HIER`-only refinement.
* **(e)** Byte-identity is a claim about the *output buffers*, not about the memory access pattern
  (D3 trade-off 3).

### D5 — `#ifdef HIER` two-compile; the base `.spv` is re-pinned **exactly once**, then frozen

`cluster_cull.hlsl` is hand-authored (no `// === GENERATED ... ===` sentinels), so it is edited
directly. The hierarchical body is compiled in only under `-D HIER=1`, producing a **new**
`cluster_cull_hier.comp.spv`. Rationale: the cull pipeline is shared by Deferred
(`passes/gbuffer.rs`), ForwardPlus (`passes/forward.rs`) and VB (`passes/vb.rs`) — a frozen base
removes all risk to those paths while the variant is proven, and it gives the equality oracle a
*free A/B*: both arms are runnable in the same process against the same inputs. Precedent: the
`-D FROXEL=1` family (`tests/vb_froxel_spv_sync.rs:98-105`,
`docs/SHADER-VARIANT-MANIFEST.md:91-97`).

**Amended for P0-A.** Rev 2's title — "base `.spv` byte-frozen" — is no longer achievable: D10
changes the *shared* `sq_dist_point_aabb`, so `cluster_cull.comp.spv` changes. The honest statement:

> `cluster_cull.comp.spv` is **re-pinned exactly once, at rung H1.6**, in a commit that contains no
> HIER code at all, and is byte-frozen from that commit onward.
> `cluster_cull_spv_sync.rs` continues to gate it under the unchanged frozen recipe
> (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-D`, no `-O`,
> `cluster_cull_spv_sync.rs:45-53`).

Two measured facts make the seam itself safe:

* An **unmodified** copy of `cluster_cull.hlsl` compiled with the frozen recipe reproduces the
  committed `cluster_cull.comp.spv` byte-for-byte (12 392 B, sha256 `dbb924967b1176af…`). The gate is
  reproducible outside the repo, so every experiment below was run in a scratch directory and **no
  committed `.spv` was written**.
* Adding the `#ifdef HIER` push member **and** a full ~130-line HIER arm leaves the no-`-D` compile
  at the *identical* 12 392 B / `dbb924967b1176af…`. The seam is physically inert; H2 gate (b)
  survives a widened push.
* Incidental, and it simplifies §4: the **shared** 3-parameter entry point
  `void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex)`
  also compiles byte-identically with no `-D` — DXC dead-strips the unused SV parameters — so the
  signature does **not** need to be `#ifdef`-split.

### D6 — Mask capacity: **equality**, and there is no defensive tail

`HIER_MASK_WORDS = 32`, plus one `summary` word whose bit *j* marks "mask word *j* is non-zero" (so
the fine walk visits only non-empty words). The host pin is an **equality**, not Rev 2's `<=`:

```rust
// crates/boyko_render/src/light.rs, beside MAX_LIGHTS:51
pub const HIER_MASK_WORDS: u32 = 32;
const _: () = assert!(MAX_LIGHTS == HIER_MASK_WORDS * 32,
    "invariant: the hier mask covers the table EXACTLY — one clamp bounds both the \
     groupshared write and the device read");
```

plus a text pin test that reads `cluster_cull.hlsl` (the `shaders_dir()` + `read_to_string` idiom
already used at `cluster_cull_spv_sync.rs:20-22` and `field_probe_gate.rs:102`) and asserts the file
contains `#define HIER_MASK_WORDS 32u`.

**Equality is load-bearing.** With `<=` (say `MAX_LIGHTS = 512` against 32 words) `ps_room` would
exceed the table's row count and the single clamp of D7 would no longer bound the *device read*.
Stating the cost openly so it is not discovered during VB-P1f: any future `MAX_LIGHTS` change is now
a compile error that forces a shader edit and a `.spv` re-bake. That is the intended price of one
clamp covering two bounds.

**There is no defensive tail.** Rev 2 promised "any index `i ≥ l0a_count + 1024` is tested
exhaustively rather than trusted to the mask". That cannot be made safe: the tail's whole point is
to `load_light(i)` for `i` past the mask capacity, and with `HIER_MASK_BITS == MAX_LIGHTS == 1024`
those rows **do not exist in the device buffer** (`LIGHT_TABLE_CAPACITY = LIGHT_HEADER_BYTES +
MAX_LIGHTS * GPU_LIGHT_BYTES`, `crates/boyko_app/src/gpu_scene/mod.rs:205-207`), and
`robustBufferAccess` is OFF. The tail would **relocate** the out-of-range read from the coarse phase
to the fine phase, not prevent it — while adding a review obligation and a second copy of the fine
test that must be kept token-identical for D4. It is replaced by an explicit, documented truncation
at `ps_n` (D7), made unreachable through the host fold: `fold_light_table_slotted`
(`crates/boyko_render/src/light_system.rs:212-300`) gates **every** write with a saturating
`if written == MAX_LIGHTS { return finish_folded_overflow(..) }` (`:263, :272, :282, :291`) and its
doc-comment `:199-210` states that this clamp is present "in ALL build profiles … the sole bounds
enforcement … so it must NOT be a debug-only guard". (The complementary checks
`l0a_count + point_spot_count <= MAX_LIGHTS` at `:300` and `light_count <= MAX_LIGHTS` at
`light.rs:1082` are `debug_assert!` only — which is precisely why the shader must not trust the
header, and does not.)

### D7 — One clamp `ps_n` bounds the groupshared write and both device reads `[P0-4b]`

**Index convention (this was Rev 2's P0-C disagreement, now settled): the mask is l0a-RELATIVE.**
Mask bit `j` ⇔ table index `ps_begin + j`, where `ps_begin = hd.l0a_count`. Three reasons, in order
of weight:

1. It is already the convention of two of the three sites Rev 2 wrote — D6's tail and D7's
   reconstruction (`i = l0a_count + (w<<5) + firstbitlow(bits)`) were both relative; only §4 phase 3
   was absolute. Relative fixes the inconsistency at its single source.
2. Under relative, **the bit index IS the coarse loop counter**, so the write bound is a one-line
   syntactic consequence of the loop condition *in the same basic block*, with no device value in the
   derivation: `j < ps_n <= HIER_MASK_BITS = HIER_MASK_WORDS*32 ⇒ (j>>5) < HIER_MASK_WORDS`. Under
   absolute the reviewer must additionally reason that `light_count <= 1024`, which is device data at
   the trust boundary — exactly the argument this decision exists to avoid.
3. Capacity headroom is expressed in the quantity the mask actually indexes: relative needs
   `point_spot_count <= 1024`, absolute needs `l0a_count + point_spot_count <= 1024`. Absolute
   overflows first.

**The clamp** (evaluated once, before any barrier; group-uniform because every lane reads the same
header words):

```hlsl
#define HIER_TPG        256u
#define HIER_MASK_WORDS 32u
#define HIER_MASK_BITS  (HIER_MASK_WORDS * 32u)   // 1024 == MAX_LIGHTS (pinned EQUAL, D6)
#if (HIER_MASK_WORDS) > 32
#error "HIER_MASK_WORDS > 32: gs_summary is a SINGLE uint, one bit per mask word"
#endif
#if (HIER_MASK_WORDS) > (HIER_TPG)
#error "HIER_MASK_WORDS > HIER_TPG: phase 1 inits exactly one mask word per lane"
#endif

LightHeader hd = load_light_header(LightBuf);
uint ps_begin = hd.l0a_count;
uint ps_room  = (ps_begin < HIER_MASK_BITS) ? (HIER_MASK_BITS - ps_begin) : 0u;
uint ps_total = (hd.light_count > ps_begin) ? (hd.light_count - ps_begin) : 0u;
uint ps_n     = min(ps_total, ps_room);
```

Both branches are uint-underflow-proof by construction. For **any** 32-bit header bytes whatsoever:

* **WRITE:** `j < ps_n <= ps_room <= HIER_MASK_BITS` ⇒ `(j>>5) <= 31` — `gs_mask[]` is never left.
* **READ (capacity):** `ps_begin + j < ps_begin + ps_room <= HIER_MASK_BITS == MAX_LIGHTS` — never
  past the table's row capacity.
* **READ (live span):** `ps_begin + j < ps_begin + ps_total == hd.light_count` — never past the live
  table, which is the bound the smaller test-harness light buffers need.

Both read bounds hold simultaneously because `ps_n` is the `min`. **This is strictly stronger than
the base arm**, whose flat loop (`cluster_cull.hlsl:161-162`) has no clamp at all.

**The fine arm re-checks the identical bound**, `if (j >= ps_n) { continue; }`, applied *before* the
reconstruction `i = ps_begin + j`. It implies `i < light_count` (`ps_n <= ps_total`) and
`i < MAX_LIGHTS` (`ps_n <= ps_room`) in one line each, so no second runtime compare is needed, and a
reviewer checks **one** bound for **both** phases. `robustBufferAccess` is OFF in this engine; an
out-of-range `StructuredBuffer<uint>` read is real UB, and this exact class already shipped one
GPU-UB bug this campaign (VB-P1b C1). The clamp makes the impossibility **local and auditable**
instead of a cross-phase argument.

**Deliberately NOT `hd.point_spot_count`** (`light_table.hlsli:248`): the base arm's range is
`[l0a_count, light_count)` (`cluster_cull.hlsl:161`), so the span must be derived from the **same two
header words** or D4's byte-identity breaks. A header whose word 3 disagreed with
`light_count - l0a_count` would otherwise make the two arms scan different ranges — and word 3 can
only *shrink* the range, i.e. silently drop lights the flat arm keeps.

**Rejected:** clamping at the write (`gs_mask[min(j>>5, 31u)]` or `& 31u`) — actively **wrong**, not
merely inelegant: saturating/masking the word index aliases out-of-capacity bits onto word 31, so the
fine walk reconstructs a *different* `j` than the one tested and can emit a light the coarse phase
never accepted. Guarding at the write (`if (j < HIER_MASK_BITS) InterlockedOr(...)`) — correct but
strictly worse: it adds a compare to the hot coarse loop, it leaves the **device read**
`load_light(ps_begin + j)` unbounded (the read precedes the guard), and it turns a loop-level
truncation into a silent per-bit drop.

### D8 — Uniform control flow around every barrier (the mandatory review checkpoint)

Today's shader early-returns on `fi >= cluster_count` (`:112-114`). The hierarchical body **must
not**: every lane, valid or not, must reach every `GroupMemoryBarrierWithGroupSync()`. The
out-of-range condition becomes a `bool valid` that guards work, never control flow across a barrier.
An early `return` here is undefined behaviour and typically a device hang. This is called out as an
explicit code-review gate item, not a comment. (Structurally confirmed on the probe: the `-D HIER=1`
module contains **exactly one `OpReturn`** — the function end — and **three `OpControlBarrier`**,
each in a *merge* block.)

**Corollary (extended in Rev 3).** An **invalid OR NON-FINITE** lane's AABB is forced to
`(+1e30, −1e30)` — the exact identity element of `min`/`max` — so it may participate in the reduction
unconditionally with no special case. The non-finite half is the P1-D mitigation (§5); folding it
into the existing substitution means it adds no new concept to review.

**Two obligations the "no early return" rule newly creates** (the base arm's `return` masks both
today, because it fires *before* `z = fi % cp.dim_z` at `:118`), both promoted to code-review gate
items alongside the barrier rule:

* `gps` **must** be `max(1u, …)`. A degenerate live header with `packed_dims == 0` — the value
  `sync_cluster_light_gate` writes on an unarmed path (`light.rs:836-840`) — gives
  `ceil(0/256) = 0` and an integer **divide-by-zero** in `slice = gid.x / gps`.
* `x`/`y` **must** be `bdx != 0u`-guarded, for the same reason on `s % bdx`.

### D9 — Reduction by a groupshared **radix-16 in-place fold** over **scalar** arrays

**Storage.** Six `groupshared float` arrays, not two `float3` arrays:

```hlsl
groupshared float gs_min_x[HIER_TPG], gs_min_y[HIER_TPG], gs_min_z[HIER_TPG];
groupshared float gs_max_x[HIER_TPG], gs_max_y[HIER_TPG], gs_max_z[HIER_TPG];
groupshared uint  gs_mask[HIER_MASK_WORDS];
groupshared uint  gs_summary;                     // bit j <=> gs_mask[j] != 0
```

Footprint: `6 × 256 × 4 = 6 144 B` + `32 × 4 = 128 B` + `4 B` = **6 276 B, exact by construction**.

*Why not `float3`.* Rev 2 asserted "6 KB" and §9 said "6.3 KB/group"; **both are unsupported by the
artifact**. Compiled under the frozen recipe and disassembled, DXC 1.4.350.0 emits
`%_arr_v3float_uint_256 = OpTypeArray %v3float %uint_256` with a `Workgroup` pointer and **no
`ArrayStride` decoration** (the module's only `ArrayStride` is the `4` on the `StructuredBuffer`'s
runtime array), and declares no `VK_KHR_workgroup_memory_explicit_layout` / no
`WorkgroupMemoryExplicitLayoutKHR` capability. Workgroup storage therefore carries **no explicit
layout**: the `float3` stride (12 B or 16 B ⇒ 6 276 B or 8 324 B) is chosen by the driver and is not
derivable from the `.spv`. A `float` has no padding ambiguity in any layout, so scalarizing removes
a driver-dependent variable from a plan whose selling point is that its correctness argument needs no
unverifiable premise. It also makes lane-indexed access **4-byte-strided (bank-conflict-free)**
instead of 12- or 16-byte-strided.

*Why not float-as-int atomics.* `InterlockedMin/Max` are integer-only; the order-preserving
float↔uint key trick would work but adds a lemma to review, and 256 lanes contending on 6 addresses
serialize anyway.

*Why radix-16 in place, and not Rev 2's 8-step halving tree.* 256 lanes → strides 128, 64, 32, 16, 8,
4, 2, 1 = **8 steps, 8 barriers** (Rev 2 counted these correctly; it was D1's summary line that was
wrong). The radix-16 fold needs **two**:

1. every lane stores its 6 scalars; **B1**;
2. lanes `l ∈ [0,16)` each serially fold the 16 entries `gs[l + 16k], k = 0..15` and write `gs[l]`.
   **Race-free in place**: every active writer has `l < 16`, while every read address `l + 16k` for
   `k ≥ 1` is `≥ 16`, and `k = 0` is the writer's own slot. **B2**;
3. **every** lane then folds `gs[0..16)` itself — 16 group-uniform broadcast reads, no write — and
   lands `coarse_min`/`coarse_max` in registers. **No third barrier**: B2 already published and
   nobody writes afterwards.

Cost: 32 serial `min`/`max` per active lane in step 2 plus 16 broadcast reads × 6 components in step
3 — roughly 96 extra scalar ops per lane, against **seven barriers deleted**. `min`/`max` are
**exact** in IEEE-754 and associative/commutative, so the fold order is irrelevant and the result is
exactly the componentwise extremum — which is what §5's proof requires, and D2's corollary already
covers the change of shape.

*Rejected:* a separate 16-entry destination array (also 2 barriers) — in-place is provably race-free
at radix 16, so the extra 384 B and the extra symbol buy nothing. A serial fold by every lane over
all 256 entries (1 barrier) — ≈ 1 536 ops/lane against ≈ 96, to save one barrier.

*Note on `[unroll]`.* The frozen recipe passes no `-O`, and a probe confirms the reduction loop is
left **rolled** in the emitted module (5 static `OpControlBarrier` for 12 dynamic ones in the probe's
8-step form, with `OpLoopMerge … Unroll` as a hint only). The barrier counts in this document are
**dynamic** counts; a `spirv-dis` gate must count executions, not instructions.

### D10 — The cull distance is a **written-out `precise` sum**, never `dot()` `[P0-A]`

**The one shared `sq_dist_point_aabb` is replaced verbatim** (`cluster_cull.hlsl:102-105`):

```hlsl
// Squared distance from a point to an AABB (0 inside). The canonical clustered-cull test:
// a sphere (center, r) intersects the AABB iff this <= r².
//
// The sum is WRITTEN OUT and `precise`, not `dot()`, on purpose. Vulkan specifies OpFAdd /
// OpFSub / OpFMul as "Correctly rounded" (one legal fp32 result), but specifies OpDot only as
// "inherited from a formula", and the same appendix permits that formula to "be transformed
// using the mathematical associativity, commutativity, and distributivity of the operators
// involved". Two OpDot instructions in one module may therefore be lowered to different
// summation orders (or to different FMA-contracted forms) by the driver — and DXC emits no
// Fma at all, so contraction is decided BELOW the .spv, where no byte- or disassembly-gate can
// see it. VB-P1e's coarse->fine enclosure proof needs the two call sites to evaluate the SAME
// function of their operands; correctly-rounded ops plus NoContraction (what `precise` emits)
// deliver exactly that, unconditionally. It also makes the GPU match the host oracle
// `golden_sq_dist_point_aabb` (goldens.rs:3491), which accumulates `s += d*d` in the identical
// ((dx²+dy²)+dz²) order and never fuses.
float sq_dist_point_aabb(float3 c, float3 aabb_min, float3 aabb_max) {
    float3 d = max(max(aabb_min - c, c - aabb_max), 0.0.xxx);
    precise float sd = d.x * d.x + d.y * d.y + d.z * d.z;
    return sd;
}
```

**Put `precise` on the scalar result, not on the return type or on `d`.** Measured: this form's
blast radius on the real shader is exactly **five** added `OpDecorate NoContraction` (3 `OpFMul` +
2 `OpFAdd`) plus the `OpDot` expanded into 3 `OpCompositeExtract` + 3 `OpFMul` + 2 `OpFAdd`, and
`Bound: 524 → 531`. Every other line of the module is unchanged; the 8 ray-gen `OpDot`s in
`view_z_to_t` are untouched. A variant with `precise` on the return type *and* on `d` **leaks**,
decorating 16 ops including the AABB construction — do not use it.

**`r * r` at `:168` stays undecorated deliberately:** it is a lone `OpFMul` whose result feeds only
`OpFOrdLessThanEqual`, so it has no contraction partner, and being correctly rounded it is
bit-identical at both sites for the same `L.range`.

**Measured opcode census** (frozen recipe, dxc 1.4.350.0, all modules `spirv-val`-clean):

| module | `OpDot` | `NoContraction` |
|---|---|---|
| base, today | 9 | 0 |
| base + D10 | **8** | **5** |
| HIER with `dot()` | 10 | 0 |
| HIER + D10 | **8** | **10** |

The 8 residual `OpDot`s are the `dot(rd, cam_forward.xyz)` in `view_z_to_t` (`:87`), 4 corners ×
near/far — i.e. **zero `OpDot` in the cull comparison**. Under D10 the id-normalised 14-instruction
window ending at each `OpFOrdLessThanEqual` is byte-equal to the other (script-verified).

**This also repairs an existing, currently-unbacked claim.** `golden_sq_dist_point_aabb`
(`crates/boyko_rhi_vulkan/src/goldens.rs:3491-3498`) accumulates `s += d * d` in Rust `f32` — never
fused, association `((dx²+dy²)+dz²)`, i.e. exactly the tree D10 emits. Two shipped tests call
`golden_cluster_cull` "the bit-exact source of truth" / "bit-exact to what the GPU cull writes"
(`tests/sdf_gbuffer_hybrid.rs:5187-5188`, `:6291-6293`) and assert GPU `ClusterGrid` occupancy
froxel-for-froxel against it (`:5199-5202`). **Today that bit-exactness is an accident of NVIDIA's
lowering; after D10 it is structural.** The repo already wrote this argument down once, for DDGI
(`shaders/ddgi_resolve.hlsli:136-143`: "DXC by default CONTRACTS the blend MACs … `precise` forbids
contraction/reassociation … matching the host to bits"), and the established minimal-use pattern
there is `precise` on the accumulator local — which is what D10 prescribes.

**Rejected alternatives** (each was tested, not argued):

* ***`precise` on `dot()`.*** DXC does propagate it and decorates the `OpDot` itself
  (`OpDecorate %19 NoContraction` → `%19 = OpDot`), and `spirv-val` accepts it. But SPIR-V defines
  `NoContraction` as constraining *combination across instructions*; it says nothing about the
  internal accumulation of a single `OpDot`, which the Vulkan precision rule continues to leave
  reassociable. **Validator acceptance is not a specified guarantee.** This is the trap that looks
  like a fix.
* ***The `spirv-dis` structural assertion as the discharge.*** Empirically satisfiable — the two
  sites are instruction-for-instruction identical even with plain `dot()` — but it proves the wrong
  thing. DXC emitted **zero `Fma`** in all 9 modules, so fusion happens in the driver's SPIR-V→ISA
  pass, strictly below what `spirv-dis` can see. A gate asserting "both sites are `OpDot`" is
  compatible with a driver lowering one as `FFMA(dy,dy,FFMA(dx,dx,0))` and the other as
  `dx*dx + (dy*dy + dz*dz)`. It would make the proof true *on today's compiler*, not unconditional.
  **Retained in H2 as a tripwire only.**
* ***Written-out sum without `precise`.*** Both sites emit `FMul, FMul, FAdd, FMul, FAdd`
  identically, but with **no** `NoContraction` — a driver may contract one site and not the other.
  Not sufficient alone.
* ***An owned bounded slack on the coarse comparison only.*** Viable, and retained as the **named
  fallback** if H1.6 measures a regression. The bound is derivable: `OpDot`'s legal return set is
  `{v : |v−e| ≤ |E−e|}` for exact `e` and any correctly-rounded association `E`; for 3 products +
  2 sums, `|E−e|/e ≤ (1+u)^5 − 1 ≈ 5u` with `u = 2⁻²⁴`, so the coarse comparison needs relative slack
  `≥ 10u = 5.96e-7 = 2⁻²⁰·⁷`, and `r*r*(1.0 + 0x1p-20)` (`9.5e-7`) covers it with 1.6× margin. Its
  selectivity cost is nil (a 10 m light grows by 9.5 µm against a coarse box spanning a whole z-slice;
  the expected change in `E_coarse` is zero and it is not measurable in H1). It loses on rigour: it
  reintroduces the epsilon §5 boasts of not needing; the `5u` figure is inferred from spec *prose*
  rather than a spec-stated ULP number for `OpDot`; and it leaves the GPU↔host bit-exactness claim
  still unbacked.
* ***A per-axis coarse test (`d.x*d.x <= rr && …`) leaving the fine test untouched.*** Attractive
  because it would leave the base `.spv` and every existing golden alone. Its soundness argument
  ("a sum of non-negatives is ≥ each of its terms") survives any association order *and* FMA
  contraction — but **not** `OpDot`'s licensed accuracy interval: a conforming `OpDot` may return up
  to `~5u` *below* the exact `Σd_j²`, hence below `fl(d_x²)`, and then fine accepts while coarse
  rejects. It still requires the fine site to stop being an `OpDot`, at which point D10 has already
  been paid for.
* ***An `#ifdef HIER`-only `precise` distance function, leaving the base arm bit-frozen.*** Breaks
  D4: D4.1 needs the HIER fine test to compute the same value as the base arm's test. The shared
  function must change for both arms; the base re-pin is the honest cost.

### D11 — The total bound is **boot-sourced** `[P0-B]`

**Invariant.** *The dispatch size, the `ClusterGrid` allocation and the shader's write bound are
three evaluations of one `u32`, minted once in `build_froxel_light_cull`.*

**Why it is needed.** The two sources genuinely diverge:

* **BOOT.** `runner.rs:636-643` reads `ClusterConfig` from the World once
  (`try_resource::<ClusterConfig>().copied().unwrap_or_default()`) and passes it to
  `build_froxel_light_cull` (`gpu_scene/mod.rs:4241`), which sizes `ClusterGrid` at
  `cluster_count * 8` bytes (`:4317-4324`) and freezes `self.cluster_count` (`:4346`).
* **LIVE.** `sync_cluster_light_gate` (`light.rs:830-851`) runs **every frame**, reads
  `Res<ClusterConfig>`, and writes `cfg.cluster_packed_dims` into the light header → the shader's
  `load_cluster_params` (`light_table.hlsli:313-323`). Its own doc-comment says it is "stale the
  moment the owner changes the grid/near/far without also touching a light" (`light.rs:783-786`).
  The arm bit `ResolvedRenderPath::froxel_light_cull` is boot-frozen (`light.rs:792-793`) while the
  **dims stay live** — the skew vector is real and documented.

`ClusterConfig` has **no production writer today** (a repo-wide grep for
`ResMut<ClusterConfig>` / `resource_mut::<ClusterConfig>` / `insert_resource(ClusterConfig` returns
4 hits, all `insert_resource` at App-build time: `plugins.rs:195` plus three test setups), but the
Resource is `pub` and world-mutable, so one owner system reaches it. The campaign has already ruled
on this hazard class **for this exact buffer**: `plugins.rs:352-363` justifies the gate's
`.before_set(LightCollectSet)` edge because a stale dims lane "would then underflow to an
out-of-bounds `ClusterGrid` index — real GPU UB with `robust_buffer_access` disabled" (same wording
at `light_system.rs:410`).

**Measured bounds** (host simulation over the boot/live matrix; buffer = boot `cluster_count`,
dispatch = host-derived from boot, mapping = as noted; `max fi` written vs capacity):

| case (boot → live) | buffer | base arm | D3 as Rev 2 wrote it | D3 + `fi < cluster_count` (live) | **D3 + D11** |
|---|---|---|---|---|---|
| 16×9×24 → 16×9×24 | 3456 | 3455 ok | 3455 ok | 3455 ok | 3455 ok, dup 0, unwritten 0 |
| 16×9×24 → **32×18×24** | 3456 | 3455 ok | **13 807 OOB** | **13 807 OOB** | 3455 ok, dup 0, unwritten 0 |
| 16×9×24 → 16×9×48 | 3456 | 3455 ok | **6 887 OOB** | **6 887 OOB** | 3455 ok, dup 0, unwritten 0 |
| 32×18×24 → 16×9×24 | 13824 | 3455 ok | 3503 ok | 3455 ok, **3 432 aliased cells** | 13823 ok, dup 0, unwritten 0 |
| **16×9×23 → 16×9×24** | 3312 | **3327 OOB** | **3454 OOB** | **3454 OOB** | 3311 ok, dup 0, unwritten 0 |
| 16×9×24 → 0×0×0 | 3456 | −1 (early return) | **div-by-0** (`gps==0`) | **div-by-0** | 3455 ok, dup 0, unwritten 0 |

Three results the design must account for, none of which Rev 2 did: `13 807` reproduces the critic's
worked example exactly; **the naive transplant `fi < cluster_count` does not fix it** (13 807 < live
13 824), proving the base arm's guard is the wrong *shape* for a re-derived `fi` — the base arm is
safe only because `fi = tid.x` is bounded by the *dispatch*, and D3 deletes that bound; and the
degenerate-header divide-by-zero is created purely by D8's "no early return" (D8 now carries both
guards). The last row also exposes the pre-existing base-arm hole tracked as VB-P1j.

**Transport — a `#ifdef HIER`-only push tail word.**

```hlsl
struct ClusterCullPush {
    float z_near; float z_far; uint max_lights_per_cluster; uint index_list_cap;
#ifdef HIER
    uint cluster_dims_packed;   // BOOT snapshot: dim_x | dim_y<<8 | dim_z<<16
#endif
};
```

Measured on the probe: the `-D HIER=1` push block has **5 members with `Offset 16` on
`cluster_dims_packed`** (base: 4 members, last `Offset 12`) ⇒ 20 B vs 16 B, and the no-`-D` compile
is byte-identical to the committed blob. Widening the *shared* struct instead — no `#ifdef` — was
also measured: it changes the base module's push-constant block type and would fail H2 gate (b).

**Precedent (this is why a push is the right transport):** the push *already* carries a boot-snapshot
buffer capacity used as a device-write bound — `pc.index_list_cap` clamps the `LightIndexList`
scatter at `cluster_cull.hlsl:184-190`, and that same `cluster_config.index_list_cap` sized the
buffer at `gpu_scene/mod.rs:4325-4331`. `cluster_dims_packed`/`capacity` is that identical pattern
applied to `ClusterGrid`, which today has no such bound. *Rejected:* a specialization constant — the
RHI exposes it (`ComputePipelineDesc { spec_constants: &[] }`, `gpu_scene/mod.rs:4304`) and its
lifetime is arguably better (baked at pipeline create), but it introduces a second transport
mechanism to review for 4 bytes and cannot be const-asserted the way push offsets are
(`compute.rs:3467-3471`). Re-openable as an output-neutral follow-up. *Rejected:*
`vkCmdDispatchIndirect` with a GPU-computed group count — it deletes the skew at the root but pays a
new device buffer, a new framegraph resource with a seeding decision and a new barrier against
§1.2's **13.9 µs fixed cost**, which is 70 % of the measured 19.7 µs cull at `N_ps=8`. *Rejected:*
doing nothing on the grounds that no production writer exists — the base arm carries its bound
unconditionally, and the fix is three comparisons on a path whose cost model is "(froxel, light) pair
tests and nothing else" (§1.1).

**Host plumbing (Principle 0: one derived accessor + one activation struct, no side store).**

* `crates/boyko_rhi_vulkan/src/compute.rs`: a **second** `#[repr(C)]` mirror
  `ClusterCullHierPush { z_near, z_far, max_lights_per_cluster, index_list_cap, cluster_dims_packed }`
  + `CLUSTER_CULL_HIER_PUSH_BYTES = 20`, with the same `offset_of!` const-asserts as
  `ClusterCullPush` (`compute.rs:3467-3471`). **`ClusterCullPush` (16 B) is not widened** — the base
  pipeline's push range and the base `.spv` stay as they are.
* `crates/boyko_render/src/light.rs`, beside `cluster_count()` (`:728`):
  `pub const fn hier_group_threads() -> u32 { 256 }` and
  `pub const fn hier_group_count(&self) -> u32 { self.dim_x.mul(self.dim_y).div_ceil(256) * self.dim_z }`.
* `crates/boyko_rhi_vulkan/src/present/scene_types.rs`: **one** new `GBufferScene` field in the
  existing activation-struct idiom (`BrickActivation` `:438`, `ViewtFromDepthActivation`,
  `ViewtFromVbDepthActivation`):
  ```rust
  pub struct ClusterCullHierDispatch { pub groups: u32, pub push: [u8; CLUSTER_CULL_HIER_PUSH_BYTES as usize] }
  /// `Some` IFF the HIER variant is the pipeline in `cluster_cull`.
  pub cluster_cull_hier: Option<ClusterCullHierDispatch>,
  ```
  `cluster_count` and `cluster_cull_push` are untouched, so the base arm is byte-identical. The four
  test literals (`window_present_gbuffer.rs:2387, 3434, 8420, 9905`) each gain
  `cluster_cull_hier: None`.
* `crates/boyko_app/src/gpu_scene/mod.rs`: `build_froxel_light_cull` (`:4241`) is the **single
  writer** — it already receives `cluster_config: ClusterConfig` and already sizes `cluster_grid`
  from `cluster_config.cluster_count()` (`:4317-4324`). It stores the activation beside the existing
  `self.cluster_count = cluster_count` (`:4346`) and `scene()` threads it beside `cluster_count`
  (`:5237`, `:5307`).
* Record sites (`vb.rs:184`, `gbuffer.rs:1583`, `forward.rs:359`) — **one `match`**, so the group
  count, push pointer and push length can never be mixed across arms:
  ```rust
  let (cull_groups, push_ptr, push_len) = match scene.cluster_cull_hier.as_ref() {
      Some(h) => (h.groups, h.push.as_ptr(), CLUSTER_CULL_HIER_PUSH_BYTES),
      None    => (scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X),
                  scene.cluster_cull_push.as_ptr(), CLUSTER_CULL_PUSH_BYTES),
  };
  ```

**Why a boot/live disagreement is now harmless, and how it is made loud.** `groups`, `gps` and
`capacity` are three evaluations of the same u32; the HIER arm never reads the header's dims lane, so
a live `ClusterConfig` edit cannot move `fi` at all — worst case it produces a grid whose geometry no
longer matches what the resolve expects (the pre-existing skew class, D4 scope clause (b)), never a
memory fault. In debug it is caught at the per-frame `scene()` call site (`runner.rs:1951`, which
already holds `world`) with the same host-authoritative-lock pattern the SSAA arm uses twelve lines
above (`runner.rs:1919-1940` — "resolution is a boot commitment … so the per-frame mode MUST agree
with it, never the reverse"):
`debug_assert_eq!(world.try_resource::<ClusterConfig>().map(|c| c.packed_dims()), Some(boot_packed_dims), "invariant: ClusterConfig dims are a boot commitment (cull buffers are boot-sized)")`.
**Stated limit:** in release this stays silent — an owner edit still yields a wrong-but-in-bounds
grid. Making it loud in release means disarming the cull for that frame (`cluster_cull = None`),
which is a behaviour/scope call for the owner, tracked as **VB-P1k**. The cheapest closure is H3's
assertion 7, so no test can silently run skewed.

**The `// SAFETY:` comment at the VB record site must be rewritten.** Today it reads (verbatim,
`vb.rs:185-190`, and byte-for-byte identically at `gbuffer.rs:1587-1588` and `forward.rs:363-364`):

```
// SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
// at set 0 + the 16-byte COMPUTE push range) are live on this device (caller
// contract); the cull set binds the camera UBO + light table + the cluster buffers;
// `cull_groups` covers `cluster_count` froxels at the 64-wide group; the push bytes
// are exactly `CLUSTER_CULL_PUSH_BYTES` (16) at offset 0; ...
```

"`cull_groups` covers `cluster_count` froxels" is a **coverage** property — the wrong obligation for
an `unsafe` FFI block, and false for the HIER map. Replacement clause:

```
// the dispatch size and the push image are the SAME `Option` arm (base: `cluster_count`
// froxels at the 64-wide group + the 16-byte `ClusterCullPush`; hier: `h.groups` groups of
// 256 + the 20-byte `ClusterCullHierPush`), so the group count can never be paired with the
// other arm's push range; NO invocation of either arm can write outside `ClusterGrid`,
// because every `ClusterGrid[fi]` write is guarded by `fi < capacity`, and `capacity` is the
// BOOT `ClusterConfig` froxel count the buffer itself was allocated from
// (`gpu_scene/mod.rs:4317-4324`) — never the live header's dims lane, which
// `sync_cluster_light_gate` (`light.rs:830`) may move behind this dispatch's back;
```

The base arm's clause must **also** be tightened, because today's wording over-claims for a
non-64-aligned grid (VB-P1j). H4 fixes `vb.rs`; H5 fixes the other two.

---

## 4. Shader structure (both arms, one file)

```hlsl
// cluster_cull.hlsl — base arm unchanged EXCEPT the shared D10 distance function;
// the HIER arm is compiled in only under -D HIER=1.
#ifdef HIER
#define HIER_TPG        256u        // host mirror: ClusterConfig::hier_group_threads()
#define HIER_MASK_WORDS 32u         // MAX_LIGHTS / 32  (D6, pinned EQUAL)
#define HIER_MASK_BITS  (HIER_MASK_WORDS * 32u)
#if (HIER_MASK_WORDS) > 32
#error "HIER_MASK_WORDS > 32: gs_summary is a SINGLE uint, one bit per mask word"
#endif
#if (HIER_MASK_WORDS) > (HIER_TPG)
#error "HIER_MASK_WORDS > HIER_TPG: phase 1 inits exactly one mask word per lane"
#endif
groupshared float gs_min_x[HIER_TPG], gs_min_y[HIER_TPG], gs_min_z[HIER_TPG];
groupshared float gs_max_x[HIER_TPG], gs_max_y[HIER_TPG], gs_max_z[HIER_TPG];
groupshared uint  gs_mask[HIER_MASK_WORDS];
groupshared uint  gs_summary;       // bit j <=> gs_mask[j] != 0
[numthreads(256, 1, 1)]
#else
[numthreads(64, 1, 1)]
#endif
void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex)
```

`HIER_TPG`/`HIER_MASK_WORDS` are `#define`, not `static const uint`, because the `#if`/`#error`
guards need preprocessor constants. (Both guards were executed once against dxc 1.4.350.0 with
`HIER_MASK_WORDS 64u`; the preprocessor accepts the `u` suffix and the `#error` fires — the guard is
mechanical, not a comment.) The **shared** 3-parameter signature is verified byte-neutral for the
base compile (D5), so it is not `#ifdef`-split.

**Phase −1 — the group-uniform prologue** (before phase 0, before any barrier). Evaluate D7's clamp
(`ps_begin`, `ps_room`, `ps_total`, `ps_n`) from header words 0 and 2 — **not** from
`point_spot_count` (word 3), because the base arm's range is `[l0a_count, light_count)`
(`cluster_cull.hlsl:161`). Evaluate D3/D11's mapping (`bdx/bdy/bdz`, `capacity`, `gps`, `slice`, `s`,
`x/y/z`, `fi`, `valid`) from the push. Both are group-uniform: every lane reads the same header words
and the same push.

Phases of the `HIER` arm (each lane; `valid` per D8):

| # | Work | Barrier after | Cost |
|---|---|---|---|
| 0 | **unchanged** froxel AABB build — 4 `generate_ray` × {near, far} + `view_z_to_t` + `expand_aabb` (`:126-153`), only when `valid` | — | 8 unprojections |
| 1 | `contrib = valid && all(abs(aabb_min) <= 1e30) && all(abs(aabb_max) <= 1e30)`; store 6 scalars = own AABB when `contrib`, else the `(+1e30, −1e30)` identity (D8); lanes 0..31 set `gs_mask[lane] = 0`, lane 0 sets `gs_summary = 0` | **B1** | 6 stores + 6 SPIR-V ops/lane (measured) |
| 2 | radix-16 in-place fold: lanes 0..15 each serially fold `gs[l + 16k], k = 0..15` → `gs[l]` (D9) | **B2** | 32 min/max × 6 on 16 lanes |
| 3 | **every** lane folds `gs[0..16)` into registers `coarse_min`/`coarse_max` — broadcast reads, no write | — | 16 reads × 6 |
| 4 | coarse scan: `for (uint j = lane; j < ps_n; j += HIER_TPG)` → `load_light(LightBuf, ps_begin + j)` → `light_kind` filter → `sq_dist_point_aabb(CL.pos, coarse_min, coarse_max) <= cr*cr` → `InterlockedOr(gs_mask[j >> 5], 1u << (j & 31u))` **and** `InterlockedOr(gs_summary, 1u << (j >> 5))`. **All 256 lanes, `valid` or not.** | **B3** | `ceil(ps_n/256)` per lane |
| 5 | fine walk (`valid` only): for each set bit of `gs_summary` ascending, for each set bit of `gs_mask[w]` ascending, `j = (w<<5)\|b`; **`if (j >= ps_n) continue;` (D7)**; `i = ps_begin + j`; then the **token-identical** fine test + `local[]` append (`:161-175`) | — | `E_coarse` tests |
| 6 | **unchanged** `InterlockedAdd` claim + scatter + `ClusterGrid[fi] = uint2(offset, write_count)` (`:180-194`), `valid` only — and it is `fi < capacity` (D11) that makes the write in-bounds | — | 1 atomic |

**Barriers: 3 total (B1, B2, B3).** This footer exists so the count cannot drift from D1 again.
Rev 2's table summed to 11 (1 + 8 + 1 + 1); the radix-16 fold removes 7 and folding the summary bit
into phase 4's atomic removes the 8th along with a whole phase. The extra groupshared atomic in
phase 4 fires only on an *accepted* coarse light — rare by construction, which is the whole premise.

**Two ordering defects in Rev 2's table are fixed here:** it stored `gs_min/gs_max[lane]` in phase 0
and *built* the AABB in phase 1, i.e. it stored a value one row before the row that computes it; and
phase 1's barrier column read "(in 0)". This table is what D8's uniform-control-flow review gate is
read against, so it must be executable as written.

**Exact HLSL for the two hot loops** (so the review gate reads code, not prose):

```hlsl
// --- phase 4, coarse ---------------------------------------------------------------
for (uint j = lane; j < ps_n; j += HIER_TPG) {
    LightElem CL = load_light(LightBuf, ps_begin + j);
    uint ck = light_kind(CL);
    if (ck != LIGHT_KIND_POINT && ck != LIGHT_KIND_SPOT) { continue; }
    float cr = CL.range;
    if (sq_dist_point_aabb(CL.pos, coarse_min, coarse_max) <= cr * cr) {
        // j < ps_n <= ps_room <= HIER_MASK_BITS == HIER_MASK_WORDS*32  =>  (j>>5) < HIER_MASK_WORDS
        InterlockedOr(gs_mask[j >> 5], 1u << (j & 31u));
        InterlockedOr(gs_summary, 1u << (j >> 5));
    }
}
GroupMemoryBarrierWithGroupSync();                              // B3

// --- phase 5, fine -----------------------------------------------------------------
uint summary = gs_summary;
while (summary != 0u) {
    uint mw = firstbitlow(summary);
    summary &= ~(1u << mw);
    uint bits = gs_mask[mw];
    while (bits != 0u) {
        uint mb = firstbitlow(bits);
        bits &= ~(1u << mb);
        uint j = (mw << 5) | mb;
        if (j >= ps_n) { continue; }        // D7: the SAME bound phase 4 wrote under
        uint i = ps_begin + j;
        LightElem L = load_light(LightBuf, i);
        uint k = light_kind(L);
        if (k != LIGHT_KIND_POINT && k != LIGHT_KIND_SPOT) { continue; }
        float r = L.range;
        if (sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r * r) {
            if (nlocal < pc.max_lights_per_cluster && nlocal < 256u) {
                local[nlocal] = i; nlocal += 1u;
            }
        }
    }
}
```

Structurally verified on the probe: the coarse write lowers to
`OpShiftRightLogical %uint %j %uint_5` → `OpAccessChain %gs_mask` → `OpAtomicOr`, with **no clamp
instruction at the write site** — the bound is entirely the loop condition, which is the
"locally evident" form D7 asks for. `%gs_mask` is declared as
`OpVariable %_ptr_Workgroup__arr_uint_uint_32 Workgroup`, adjacent to `gs_min`/`gs_max`/`gs_summary`
in the same storage class, which is exactly why an unclamped word write would silently corrupt the
coarse box rather than fault.

**What is token-identical, and what that buys.** Phases 0, 5-tail and 6 are *token*-identical to the
base arm — including the D10 `sq_dist_point_aabb`, which is now the **same shared function** for both
arms and both levels. That is what makes D4's byte-identity a construction rather than a hope, and
after P0-A it is **load-bearing for the §5 proof**, not merely convenient. Two honest qualifications:
phase −1 substitutes `bdx/bdy/bdz` for `cp.dim_x/…`, which is *value*-identical under D4 scope clause
(b) (both are the same 8-bit fields of the same encoding); and phase 6's memory **access pattern** is
not identical (D3 trade-off 3).

**Host side (Rev 2's paragraph here was not implementable and is replaced).** `GBufferScene` carries
`cluster_count: u32` and **no dims** (`present/scene_types.rs:1409-1411`), and `ClusterConfig` is a
`boyko_render` Resource not reachable from the RHI crate — so "the three record sites dispatch
`hier_group_count()`" could not be written. The replacement is D11's plumbing in full:
`ClusterConfig::hier_group_threads()/hier_group_count()` beside `cluster_count()` (`light.rs:728`);
one `GBufferScene` field `cluster_cull_hier: Option<ClusterCullHierDispatch>`; written **only** in
`build_froxel_light_cull` (`gpu_scene/mod.rs:4241`, beside `:4346`); consumed by one `match` at each
of `vb.rs:184`, `gbuffer.rs:1583`, `forward.rs:359`; four test literals updated. The host↔shader pin
test is a **pure-arithmetic CPU test** (H1 assertion 7) that replicates the shader walk over the six
grid configs of §8's matrix and asserts the written set is exactly `[0, cluster_count)`.

---

## 5. The exactness proof (re-derived against the real code; `[P1]` and `[P0-A]` discharged)

**Claim.** If the coarse test rejects light `L` for a group, then the fine test rejects `L` for every
froxel in that group — in IEEE-754 arithmetic, with no epsilon.

**Setup.** Lane `i` computes `(min_i, max_i)` by expanding from `(+1e30, −1e30)` over 8 world points
`ro + rd·t`, where `(ro, rd) = generate_ray(...)` (`ray_gen.hlsli:44`) and
`t = view_z_to_t(slice_view_z(z|z+1), rd)` (`cluster_cull.hlsl:77-91`). Lanes that are `!valid` or
whose AABB is non-finite substitute the identity `(+1e30, −1e30)` (D8, §4 phase 1). The group values
are `MIN = min_i min_i`, `MAX = max_i max_i`, componentwise (D2/D9).

**Step 0 — the evaluation function is spec-determined (this is what Rev 2 disclaimed and Rev 3
discharges).** Under D10, `sq_dist_point_aabb` computes the explicit tree

```
F(d) = ((fl(d.x·d.x) + fl(d.y·d.y)) + fl(d.z·d.z))
```

as `OpFMul`/`OpFMul`/`OpFAdd`/`OpFMul`/`OpFAdd`, every node of which Vulkan's *Precision and
Operation of SPIR-V Instructions* specifies as **"Correctly rounded"** — one legal fp32 result — and
every node of which carries `NoContraction` (emitted by `precise`; measured: exactly 5 decorations
per call site, 10 in the HIER module, 5 in the base module). **Therefore both call sites evaluate one
function `F`, not two schemes**, and the link the critic identified as missing —
`A(d_fine) ≤ B(d_fine)` for a coarse scheme `A` and a fine scheme `B` — is **vacuous, because
`A ≡ B ≡ F`.**

*The counter-fact that motivates it.* `dot(d,d)` lowers to a single `OpDot`, whose Vulkan precision
is only **"inherited from"** a formula, and the same appendix permits that formula to *"be
transformed using the mathematical associativity, commutativity, and distributivity of the operators
involved to yield an equivalent formula"*. Two `OpDot` instructions in one module may therefore be
lowered to different summation orders. And **DXC emits zero `Fma`** in every variant measured (9
modules), so contraction is a *driver-side* decision at SPIR-V→ISA lowering — invisible to any
`.spv` byte gate and to any `spirv-dis` gate. That is why a structural gate alone **cannot** discharge
this, and why H2's gate is labelled a tripwire.

*`r*r` needs no decoration:* it is a lone `OpFMul` feeding only `OpFOrdLessThanEqual`, so it has no
contraction partner, and being correctly rounded it is bit-identical at both sites for the same
`L.range`.

**Step 1 — the reduction is exact.** `min`/`max` on floats introduce no rounding: the result is one
of the inputs (`max(...)` lowers to `GLSL.std.450 NMax`, which returns an operand). Hence,
componentwise and exactly, `MIN_j ≤ min_{i,j}` and `MAX_j ≥ max_{i,j}` for every contributing lane
`i` and axis `j`. Fold order is irrelevant: `min`/`max` are exactly associative and commutative, so
D9's radix-16 shape is as sound as any tree, and D2's corollary already covers it.

**Step 2 — `F` is monotone in the box.** With `d_j = max(lo_j − c_j, c_j − hi_j, 0)` and result
`F(d)`:

* `lo_j ≤ lo'_j ⇒ fl(lo_j − c_j) ≤ fl(lo'_j − c_j)` — `OpFSub` is correctly rounded, and IEEE-754
  round-to-nearest is **monotone** (a ≤ b ⇒ `fl(a) ≤ fl(b)` for the same operation and rounding).
* `hi_j ≥ hi'_j ⇒ fl(c_j − hi_j) ≤ fl(c_j − hi'_j)`, same reason.
* `NMax(·, ·)` is monotone and returns an operand exactly; the result is non-negative.
* `d_j ≥ 0`, and `x ↦ fl(x·x)` is monotone on non-negatives; `fl(a + b)` is monotone in each
  argument.

Therefore, componentwise `d_coarse ≤ d_fine` and hence `F(d_coarse) ≤ F(d_fine)` **as computed**, not
merely in exact arithmetic.

**Step 3 — conclusion.** Fine accepts ⇔ `F(d_fine) ≤ r·r`. By Step 2 that implies
`F(d_coarse) ≤ r·r`, i.e. coarse accepts. Contrapositive: coarse rejects ⇒ fine rejects. ∎

**Lemma (degenerate group).** An all-identity group yields the inverted box `(+1e30, −1e30)`;
`sq_dist_point_aabb` then computes `d ≈ 1e30` per axis and `F(d)` overflows to `+inf` (finite ×
finite → inf, never NaN), and `inf <= r*r` is false ⇒ the group rejects everything. Well-defined, no
NaN, no UB. A fully-invalid group cannot occur by construction (the `ceil` in `gps`).

**What the proof needs, and what it does not.**

* **It does not need** a dilation constant, an epsilon, or an assumption that two *sites* compile
  identically. (They need not — they need only be `F`.)
* **It does need exactly one named premise, and that premise is discharged, not disclaimed:**

  > **Premise P.** *Every arithmetic node in `sq_dist_point_aabb` is a correctly-rounded,
  > `NoContraction`-decorated SPIR-V op.*
  >
  > **Discharged in the shader source** (D10's body is the artifact), **and gated** by H2's
  > structural assertion (`OpDot == 8` — i.e. zero in the cull comparison — `NoContraction == 10`,
  > and the two 14-instruction windows id-normalised equal). The gate is defence in depth; the proof
  > is this step.

**Residual hypotheses (each gets a test, §9):**

* **Finiteness — and Rev 2's handling of it was wrong.** Rev 2 wrote "today's flat arm has the
  identical exposure, so this is not a new hazard". **That is false.** The flat arm's blast radius is
  **one froxel**; the hierarchical arm's is **one group (144 froxels at the default grid)**. Worse,
  the outcome is *undefined and non-deterministic in both directions*: `GLSL.std.450 FMin`/`FMax`
  leave the NaN result undefined, so if `max` swallows the NaN then `d = 0`, `F(d) = 0` and the
  coarse box **accepts everything**; if the NaN reaches the compare then `NaN <= r*r` is false and
  the group **rejects everything**; and because `min(NaN,x)` vs `min(x,NaN)` differ under the common
  `b<a ? b : a` lowering, whether the NaN even reaches the root depends on the poisoned lane's
  position in the fold and on operand order the compiler chose — yielding, in between, a silently
  non-enclosing coarse box.

  **Two reachable sources**, both cited, and the first is invisible to any host finiteness assert:

  1. `crates/boyko_scene/src/camera.rs:325-327` normalizes the three camera basis vectors, and
     `crates/boyko_math/src/vec.rs:226-233` — `Vec3::normalize` returns `Self::ZERO` when
     `len_sq <= f32::MIN_POSITIVE`. A singular/zero-scale camera `GlobalTransform` therefore yields
     `cam_forward = (0,0,0)`, which is **finite**. `camera.rs:331-336` gives `view` an identity
     fallback ("the degenerate camera renders the identity view rather than NaN") but gives the
     **basis** none. The finite zeros are uploaded verbatim (`compute.rs:3005-3015`), and on device
     `ray_gen.hlsli:63-67` computes `dir = cam_fwd + right·(..) + up·(..)` = exactly `(0,0,0)` and
     then `normalize(dir)`, which is **undefined** per GLSL.std.450 (in practice `0 × rsqrt(0)` =
     NaN). `cluster_cull.hlsl:151-152` feeds that `rd` to `expand_aabb`. Every uploaded float is
     finite, so a host assert sees nothing.
  2. `cluster_cull.hlsl:77-79`'s `slice_view_z` is `z_near * pow(z_far/z_near, k/dim_z)`, and
     `ClusterCullPush::new` (`compute.rs:3473-3483`) validates neither. The only `z_far > z_near > 0`
     check is a `debug_assert!` in a **different** function (`ClusterConfig::z_scale`,
     `light.rs:738-743`) that is not on the push path. With `z_near == 0.0`: `+inf`, then
     `0.0 * inf = NaN` ⇒ a NaN AABB even under ORTHO, with no ray-gen involved.

  **Mitigation — on device, two lines, folded into D8's existing substitution** (§4 phase 1):

  ```hlsl
  bool finite  = all(abs(aabb_min) <= 1.0e30) && all(abs(aabb_max) <= 1.0e30);
  bool contrib = valid && finite;
  ```

  `all(abs(v) <= 1e30)` and not `isfinite(v)`: an ordered compare is false for NaN **and** ±inf, so
  the predicate is exactly "finite and inside the sentinel envelope", the sentinel satisfies it
  (`abs(1e30) <= 1e30`, so the substitution is idempotent), and it is **measured cheaper** — 6
  SPIR-V instructions (`FAbs`, `FOrdLessThanEqual`, `All`, ×2) against `isfinite`'s 10 (`IsNan`,
  `IsInf`, `LogicalOr`, `LogicalNot`, `All`, ×2). Applied **only to the groupshared write**, never to
  the fine test, so §4's token-identity — and hence D4 — is untouched.

  **After the mitigation §5's proof is unconditional**: MIN/MAX are componentwise extrema over a set
  of finite values ∪ {identity}, so Steps 1 and 2 never see a NaN input, and the blast radius is
  restored to per-froxel — which is what Rev 2 *claimed* and did not deliver.

  **Cost is a MODEL number, not a fact** (§1.1's rule applied to this plan): ≈ 6–12 scalar ops per
  lane, once per lane per dispatch, i.e. 6144 times at the default grid — `O(froxels)`, independent
  of `N`. Against §1.2's 0.2736 ns/pair and a ~25-op pair test that is ≈ 0.4–0.9 µs per invocation,
  ≤ ~4 % of the predicted 22.7 µs at `N=512` and ≈ 5 % of the predicted 15.9 µs at `N=8` (where §2's
  gate allows 21.7 µs, i.e. 5.8 µs of headroom). **H4 measures it**; the plan does not assert it.
* `NMax` returns an operand — exact, no rounding — so the `d` chain contributes no scheme dependence
  of its own.
* `±0.0`: `min(+0.0, −0.0)` may return either; both compare equal in every downstream operation, so
  Step 2 is unaffected.

---

## 6. `index_list_cap` saturation `[P1-3]`

The critic is right that reordering is **not** output-neutral under saturation: slices are claimed
by a single global `InterlockedAdd(LightIndexAlloc[0], nlocal)` (`:183`), and when the claim runs past
`index_list_cap` the tail is dropped (`:184-191`) — *which* froxel loses its tail depends on claim
order, and the hierarchical arm changes claim order.

**Discharge, with numbers (§1.3):** on the VB-P1d rig, peak total claim is **2 709 words = 16.5 %**
of the 16 384-word cap (at `N_ps=1024`), and peak per-froxel count is **109 vs the 256 cap**. Neither
cap is reached anywhere in the swept range, so the drop path is never taken and claim order cannot
affect any surviving index. Byte-identity between the arms **is** achievable on this rig.

**But the plan does not rest on that estimate remaining true.** Three mechanisms:

1. **An exact runtime detector.** After the cull, `LightIndexAlloc[0]` holds the *total claimed*
   (pre-clamp) count, because `InterlockedAdd` bumps even when the write is dropped. So
   `alloc_total ≤ index_list_cap` ⇔ **no index was dropped anywhere**. One `u32` settles it exactly,
   per run, with no modelling.
   **Where that `u32` is read (this changed in Rev 3, and it is what makes §9's `[P0-1]` row true):**
   **not** from the production present path. It is read in **H3's cull-only driver**, which creates
   `LightIndexAlloc` as `MemoryLocation::HostVisibleCoherent` and reads it through
   `buffer_mapped_ptr` after the fence — the existing idiom at
   `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:5415-5425`. No `vkCmdCopyBuffer`, no staging
   buffer, no framegraph resource, no seed decision. §1.3's CPU oracle computes the same
   `total_indices` exactly, on the CPU, in 0.45 s; and when an H4 pin moves, the diagnosis is to
   reproduce that scene's light table under the same cull-only driver — diagnosis on demand, not
   per-frame work.
2. **The equality oracle asserts it as a precondition** (§9, H3): if `alloc_total ≥ index_list_cap`
   the test **fails loudly** rather than silently comparing two differently-clamped results.
3. **The honest caveat, stated in the shader header and the test:** under saturation the arms may
   legitimately differ in *which* froxel loses its tail; byte-identity is claimed only for
   non-saturating configurations (D4 scope clause (a)), and the saturating case is pinned only
   against itself.

The per-froxel cap (`max_lights_per_cluster`, `:170`) is *not* order-sensitive — it truncates the
ascending prefix identically in both arms (D4.3) — so only the global cap needs this treatment.

---

## 7. Predicted win (the pair-count half is falsifiable at H1 before any shader work)

Using §1.2's model and §1.3's occupancy profile, `TPG=256` ⇒ 24 groups:

```
pairs_hier(N) = 24·N                       (coarse, phase 4)
              + Σ_froxels E_coarse(parent) (fine,  phase 5)
```

| `N` | flat pairs | coarse | fine (est.) | hier pairs | ratio | model cull hier | measured cull flat |
|---|---|---|---|---|---|---|---|
| 8   | 27 648    | 192    | ≈ 6 900  | ≈ 7 100  | 3.9× | ≈ 15.9 µs | 19.7 µs |
| 64  | 221 184   | 1 536  | ≈ 5 000  | ≈ 6 500  | 34×  | ≈ 15.7 µs | 72.7 µs |
| 128 | 442 368   | 3 072  | ≈ 5 200  | ≈ 8 300  | 53×  | ≈ 16.2 µs | 134.9 µs |
| 512 | 1 769 472 | 12 288 | ≈ 20 000 | ≈ 32 300 | 55×  | ≈ 22.7 µs | 498.1 µs |

`froxel_total_hier(N) ≈ 26 500 + 13 939 + 0.2736·pairs_hier(N)` ⇒ break-even against
`flat_shade(N) = 23 922 + 1 109.6·N` at **`N ≈ 17`**, not Rev 2's 16: from this table's own fine
column, at `N=16` flat still wins (41 676 vs 42 358) and at `N=17` hier wins (42 355 vs 42 785). It
is necessarily **above** §7.1's floor. At a 2×-pessimistic marginal rate, **`N ≈ 25–30`**. The
conclusion is robust to a 2× model error: at `0.547 ns/pair` the `N=512` cull is still ≈ 51 µs, a
10× win.

**Fine-column derivation (the number H1 replaces with a measurement).** From §1.4's collinearity
result, the in-frustum lights at `N=512` are the ≈ 14 % of the rig lying at view-depth 8.7–14.4, i.e.
z-slices 17–19 of 24. Those three groups therefore carry ≈ 40 candidates each over their 144
froxels (3 × 144 × 40 ≈ 17 300); the remaining 21 groups carry ≈ 0–2 (≈ 3 000). H1 computes this
exactly, per config, on the CPU.

### 7.0 What this table is a bound on — pre-registered, so H4 cannot be retro-fitted

**The µs column above is an AGGREGATE-THROUGHPUT bound.** It multiplies total pair count by
§1.2's marginal rate, which was calibrated on a *balanced* 54-group dispatch. The hierarchical
dispatch is deliberately imbalanced: at `N=512`, **3 of 24 groups carry 17 300 of ≈ 20 300 fine pairs
= 85.2 %**, while 21 groups idle. If the pass is latency-bound rather than throughput-bound, wall
clock is set by **one group's serialized latency**, and the aggregate bound is optimistic by roughly
the imbalance factor.

The plan therefore commits, **in advance**, to two readings and one discriminating experiment:

* **Reading A (aggregate-throughput).** `cull_ns(512) ≈ 13 939 + 0.2736 × 32 300 ≈ 22.7 µs`. This is
  the number §2's ship gate is written against (`≤ 250 000`, a 10× margin over it).
* **Reading B (hot-group latency).** Wall clock tracks the *hottest group's* pair count
  (≈ 5 800 fine + 512 coarse ≈ 6 300) plus the fixed cost, but serialized against a machine that is
  mostly idle. Reading B predicts a *higher* number than A whenever the hot group cannot hide its
  own latency, and — critically — it predicts that `cull_ns` is **insensitive to `dim_z`** at fixed
  `N`, whereas A predicts it scales with total groups.
* **The discriminating measurement is H4's `dim_z` sweep at fixed `N`.** If `cull_ns` tracks the
  hottest group's pair count rather than the total, Reading B is confirmed and §7's µs column must be
  re-derived from it before any ship decision. **Neither H1 nor H1.5 can see this** — H1 counts
  pairs, H1.5 varies thread count on a *balanced* dispatch.

**And the necessary/sufficient split, stated plainly.** H1's 55× selectivity is a *pair-count* result.
**55× selectivity is fully compatible with a sub-2× wall-clock win**, because the hierarchical arm
also introduces: 6144 threads instead of 3456; **43.75 %** of lanes idle in the fine phase; 3 barriers
per group; 24 groups on 28 SMs, leaving 4 SMs empty (D3); a 192-B-strided `ClusterGrid` write pattern
(D3 trade-off 3); and the 85.2 % hot-group concentration above. None of those is visible to a
pair-count oracle.

### 7.1 The negative result — single-digit break-even is impossible here, and why

`froxel_shade` alone is 25–30 µs and the cull's fixed cost is ≈ 13.9 µs, so **the froxel arm's floor
is ≈ 40 µs**, while `flat_shade`'s intercept is **23.9 µs**. Break-even requires
`flat_shade(N) > floor`, **and that is with a cull of cost zero**:

```
N > (26 500 + 13 939 − 23 922) / 1 109.6 = 16 517 / 1 109.6 = 14.89
```

Rev 2 printed **14.4**, which came from rounding the numerator 16 517 → 16 000. **The conclusion is
unaffected and is now proved rather than estimated**, because the floor is published as a **band**
over `froxel_shade`'s own stated error bar and over the choice of fit:

| variant | floor |
|---|---|
| `froxel_shade = 24 300` (−1σ) | `N > 12.90` |
| `froxel_shade = 26 500` (nominal) | **`N > 14.89`** |
| `froxel_shade = 28 700` (+1σ) | `N > 16.87` |
| consistent 128/512 re-anchoring of §1.2 (`13 871 + 945.70·N`, `25 758 + 1 106.0·N`) | `N > 13.21` |

**Every variant exceeds 12.9**, so the negative result does not depend on which fit is chosen or
where in the error bar `froxel_shade` lands. No amount of cull optimisation can push the break-even
below ≈ 13 on this hardware and this grid. Reaching single digits would require attacking the *fixed*
costs (merge the cull into the shade dispatch to delete a barrier; eliminate the `cmd_fill_buffer`
reset via a per-FIF alloc ring; or shrink the froxel grid at low `N`). Those are named as VB-P1g in
§11 and are explicitly **out of scope** here. The stated goal "break-even collapses toward single
digits" is therefore **partially unreachable**, and the plan does not pretend otherwise: it targets
**≈ 17–30**, a 3–6× improvement on the measured ≈ 103.

---

## 8. Rungs

Each rung is independently committable, has one gate, and states what turns that gate RED — and,
where Rev 3 changed what a gate can prove, what it **can no longer prove**.

### H0 — Instrument the fixed cost (no behaviour change, and **no new framegraph access**)
* **Files:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (+1 `VbTimedPass` slot;
  `VB_PASS_COUNT` 2 → 3), `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-219` (split the
  `LightCull` bracket into `CullReset` = fill+barrier and `CullDispatch`),
  `crates/boyko_app/src/runner.rs` (print both).
* **Removed from Rev 2's H0:** "plus `alloc_total` read back from `LightIndexAlloc[0]`". That
  readback **leaves the present path entirely** (§P1-E). Rev 2's shape would have appended a
  `TRANSFER_READ` to `light_index_alloc`, whose declared seed is
  `ResSync::seeded_writer(COMPUTE_SHADER, SHADER_WRITE)` (`graph_bridge.rs:3187-3190`); the frame-end
  state would then be `visible = TRANSFER/TRANSFER_READ, flush = 0`, which that seed no longer
  describes — the same shape as the WAR race fixed at `5e07936`. It is non-racy today only because
  `runner.rs:2069` calls `ctx.wait_idle()` on every armed frame, and an undocumented dependency on an
  incidental `wait_idle` in a different crate is a landmine, not an invariant. `alloc_total` now
  comes from H3's host-visible cull-only driver and H1's CPU oracle (§6). **What H0 can no longer
  prove:** nothing about `alloc_total`; its scope is strictly the fixed-cost attribution.
* **Why first:** §1.2's "13.9 µs is fill+barrier" is a *hypothesis*. §1.1 is this campaign's standing
  reminder that unmeasured hypotheses about this shader have already cost one 2× regression. If the
  fixed cost turns out to be dispatch-intrinsic rather than barrier-intrinsic, §7.1's follow-up list
  changes and the low-`N` predictions move. H0 also prints the device's SM count from
  `VkPhysicalDeviceProperties` rather than letting §D3 carry 28 as an assumption.
* **HANG WARNING (a real defect in Rev 2's wording).** `read_vb_bench_ns` uses
  `VK_QUERY_RESULT_WAIT_BIT` and will **hang forever** on any timestamp pair a code path fails to
  write. Both new pairs must therefore be written on **every armed frame**, including a flat-leg boot
  where `scene.cluster_cull` is `None` — i.e. the new `write_begin`/`write_end` calls go **outside**
  the `if let (Some(cull_pipeline), …)` gate at `passes/vb.rs:157`, exactly as the existing
  `LightCull` bracket does (`:146-150`, `:216-219`). Rev 2 placed the split inside that gate.
* **Gate:** the bench prints `cull_reset_ns + cull_dispatch_ns`, and their sum reproduces the existing
  `froxel_cull_ns` **within 5 % at `N ∈ {8, 512}`**; every golden pin byte-identical.
* **RED if:** any pin moves (timestamp writes must not perturb rendering results); the sub-brackets do
  not sum within 5 %; the run hangs (⇒ an unwritten pair).

### H1 — CPU oracle: the host hierarchical mirror + the permanent set/occupancy/selectivity gate
* **Files:** `crates/boyko_rhi_vulkan/src/goldens.rs` (+ `golden_cluster_cull_hier`, a
  block-decomposed mirror of `golden_cluster_cull`:3510 using D2's min/max merge),
  `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (+ the matrix test, beside the §12 pin
  test that this rung *hardens*, not replaces).
* **What it asserts, per config in the matrix:**
  1. `golden_cluster_cull_hier == golden_cluster_cull` **exactly**, per froxel, **including order**.
  2. Coverage totality: every froxel index is produced by exactly one (group, lane).
  3. `total_indices < INDEX_LIST_CAP` and `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (§6, and it pins
     §1.3's table as a regression).
  4. All AABB bounds finite (§5's residual hypothesis — note this is now *defence in depth*: the
     device enforces it structurally via D8's identity substitution).
  5. **Selectivity (the perf premise):** `pairs_hier / pairs_flat ≤ 1/8` on the bench rig at
     `N ≥ 128`. This is a *pair-count gate that runs on the CPU in 0.45 s with no GPU*.
  6. **Mask-capacity boundary.** A config with `l0a_count == 0` and `point_spot_count == MAX_LIGHTS`
     (1024) must be present, so **mask word 31 / bit 1023** is exercised, and the produced set must
     still equal the flat oracle. *Reason:* every other config leaves word 31 dark — `light_count` is
     clamped to 1024 by the host fold, so any directional/sky light pushes the point/spot span below
     1024. A 20 000-trial randomized simulation hit word 31 in only 196 runs.
  7. **The host↔shader mapping pin (D11).** Replicate the shader walk (`gps`/`slice`/`s`/`x`/`y`/`z`/
     `fi` + the three-term `valid`) on the host over a dims matrix that **includes non-64-aligned and
     degenerate grids** — 16×9×23, 1×1×1, 0×0×0, 255×255×255 — and assert the written set is exactly
     `[0, cluster_count)`: no duplicate, no gap, no index ≥ `capacity`. Pure arithmetic, no GPU.
* **Matrix (six grid configs — Rev 2's two were both `gps = 1` and could not test D3 at all):**

  | entry | dims | `dim_x·dim_y` | `gps` | what it alone catches |
  |---|---|---|---|---|
  | M1 | 16×9×24, ORTHO 64×64 (the `l1_cluster_config` fixture, `sdf_gbuffer_hybrid.rs:5215`) | 144 | 1 | the shipped fixture |
  | M2 | 16×9×24, PERSPECTIVE 512×512 (the VB-P1d camera) | 144 | 1 | the bench camera |
  | E1 | 16×16×24 | 256 | 1 | the `gps=1` boundary **from above**; a `<` vs `<=` slip in `ceil(dim_x·dim_y/256)` |
  | E2 | 32×16×24 | 512 | 2 exact | a **transposed** mapping (`slice = gid % gps; s = (gid/gps)·256+lane`) — provably indistinguishable from the correct one at `gps=1`, and here it drives `fi` far out of range |
  | E3 | 16×17×24 | 272 | 2 ragged (16 of 256 lanes valid in the tail group) | D8's identity-element corollary under load (240 identity lanes must not perturb MIN/MAX) and `valid` gating **both** phase 6 and the fine walk — E2 cannot catch either, every lane there is valid |
  | E4 | 32×24×24 | 768 | 3 exact | an off-by-one or a hardcoded `gid >> 1` in `gid / gps`, which a `gps=2` case masks exactly |

  **Why this matters:** at 16×9 the map's `(gid % gps)·256 + lane` degenerates to `lane` and
  `gid / gps` to `gid`, so a transposed mapping is *indistinguishable from the correct one* and D3 /
  D11 are untested by Rev 2's entire matrix.

  Crossed with {bench Kronecker rig, corrected R3 rig, dense in-frustum rig, adversarial boundary
  rig} × `N ∈ {0, 1, 8, 64, 128, 512, 1024}`. The **adversarial** rig places lights so that
  `sq_dist_point_aabb == r*r` exactly for a chosen froxel, and at `r ± 1 ulp`, on faces, edges and
  corners of the AABB — the boundary of the `<=` test, which is where a non-conservative coarse level
  would first fail.

  **Runnability, verified structurally:** nothing on the path is hardcoded to 16×9×24 —
  `runner.rs:637-642` reads the live `ClusterConfig` Resource, `gpu_scene/mod.rs:4317-4338` sizes
  every buffer from it, `passes/vb.rs:184` derives the dispatch from `scene.cluster_count`, and the
  base shader reads dims from the header. The host oracle is dim-generic (`GoldenClusterConfig`
  `goldens.rs:3398-3411`, `golden_cluster_index(x,y,z,dim_x,dim_z)` `:3437-3441`). The only hard
  limit is `packed_dims`' 8 bits per dim (`light.rs:764-772`), which all six configs respect. E2/E3/E4
  set `index_list_cap = cluster_count * 8` (the `sdf_gbuffer_hybrid.rs:5230` idiom) so the cap does
  not bind — E4's 18 432 froxels are 5.3× the default.
* **Gate:** all seven assertions green over the whole matrix.
* **RED if:** any froxel's index vector differs in content or order; a light is dropped at the range
  boundary; selectivity misses 8×; assertion 7 reports a duplicate, gap or out-of-capacity index.
  **Concrete mutation that must turn it red:** scale the coarse extents by `0.999`
  (`MIN *= 1.001; MAX *= 0.999`) — a non-conservative coarse box — and the adversarial rig must fail.
* **What H1 can prove, and what it explicitly cannot.** It falsifies the **pair-count premise** — a
  *necessary* condition, and the campaign's cheap kill switch. It is **not sufficient**: it cannot
  see thread count, barrier cost, the 43.75 %-idle fine phase, hot-group serialization, or the
  `ClusterGrid` write pattern. Rev 2's one-line verdict claimed otherwise and is withdrawn (§7.0).
* **Abort point:** if selectivity on *both* the bench rig and the in-frustum rig is < 4×, the rung
  stops here at zero GPU cost and the plan is rewritten (§10).

### H1.5 — Dispatch-shape transfer probe (no new shader, no new `.spv`, no framegraph change)
* **Files:** `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs:278` — replace
  `app.insert_resource(ClusterConfig::default())` with a swept
  `ClusterConfig { dim_x, dim_y, dim_z, ..Default::default() }`, **one app boot per config** so
  D11's boot-snapshot hazard is not exercised (`dim_x/dim_y/dim_z` are `pub` fields).
* **What:** at fixed `N_ps = 512`, measure `froxel_cull_ns` on the **existing flat arm** at grids
  8×9×24 (1 728 froxels), 16×9×24 (3 456, the anchor), 16×9×48 (6 912) and 32×18×24 (13 824), and fit
  against `froxels × N`.
* **Why:** §1.2's own model-validity caveat says `0.2736 ns/pair` is calibrated on **one** dispatch
  shape. This is the only test of whether it transfers that costs no shader bytes. Run E2's dims
  (32×16×24) through the **base** pipeline here as well, before the hier arm exists — if the base arm
  is green at `gps ≥ 2` dims then the config plumbing is proven independently, and any later
  `gps ≥ 2` failure is attributable to the hier mapping alone.
* **Gate:** the fitted rate is within **±25 %** of 0.2736 ns/pair across the 8× froxel range.
* **RED if:** the low-froxel points sit well above the line — a **latency floor** exists, and §7's
  µs predictions must be re-derived from it *before* H2's `.spv` + manifest + gate work is committed.
* **What it can no longer be claimed to prove:** it bounds *thread-count* scaling on a **balanced**
  dispatch. It says nothing about barrier cost, idle lanes or hot-group serialization; those are
  H4's (§7.0).
* **Note:** raise `index_list_cap` (or assert §6's `alloc_total < index_list_cap`) at the
  13 824-froxel point.

### H1.6 — The D10 `precise` edit and the **one-time** base `.spv` re-pin (no HIER code)
* **Files:** `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` (`sq_dist_point_aabb` only, D10's
  body verbatim), `crates/boyko_rhi_vulkan/shaders/cluster_cull.comp.spv` (re-pinned once),
  `crates/boyko_rhi_vulkan/src/goldens.rs:3488-3490` and
  `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:5187-5188, :6291-6293` (doc-comments: the
  GPU↔host bit-exactness of the cull distance is now **structural** — matching `((dx²+dy²)+dz²)`
  association, no fusion on either side — rather than incidental; the same note
  `shaders/ddgi_resolve.hlsli:136-141` already carries for DDGI).
* **Why a rung of its own:** it isolates the base-arm ULP perturbation from the hierarchical change,
  so H3's arm-vs-arm equality oracle compares two already-`precise` arms and a moved pin here has
  exactly one possible cause.
* **Gate:** `cluster_cull_spv_sync` green under the unchanged frozen recipe; `lighting_l1_host_oracle`
  green; `sdf_gbuffer_hybrid::l1_known_light_lands_in_the_expected_clusters` and
  `l1_clustered_resolve_matches_the_brute_force_image` green; the full image-golden suite green **or
  moved-and-explained**; and `froxel_cull_ns` from `vb_p1d_cull_shade_bench.rs` at
  `N_ps ∈ {128, 512}` recorded **before and after, in this document**, with no regression beyond
  run-to-run noise.
* **RED if:** a golden pin moves and the move cannot be attributed to a ≤ 1–2 ULP shift in
  `sq_dist` (any re-pin must record the ULP explanation, never happen silently); **or** the base cull
  regresses beyond noise. **On a measured regression, fall back to D10's named alternative** — slack
  `r*r*(1.0 + 0x1p-20)` on the **coarse comparison only**, base arm untouched — and re-run this gate.
* **What it changes about the plan's other claims:** D5's "base `.spv` byte-frozen" no longer applies
  before this commit; from this commit onward it does.

### H2 — The `-D HIER=1` shader variant (dark infra)
* **Files:** `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` (the `#ifdef HIER` arm, §4),
  `crates/boyko_rhi_vulkan/shaders/cluster_cull_hier.comp.spv` (new, offline dxc 1.4.350.0),
  `crates/boyko_rhi_vulkan/src/compute.rs` (+ `cluster_cull_hier_spirv()` beside
  `cluster_cull_spirv()` at `:1610`, + the `ClusterCullHierPush` mirror per D11),
  `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs` (adopt the multi-variant idiom of
  `vb_froxel_spv_sync.rs:88-130`),
  `crates/boyko_rhi_vulkan/tests/cluster_cull_hier_dis_gate.rs` (**new**, below),
  `docs/SHADER-VARIANT-MANIFEST.md:91-97` (+ one row). Pipeline **built but never selected**.
* **Gate:**
  * **(a)** `cluster_cull_hier.comp.spv` byte-equals its re-DXC under the frozen recipe.
  * **(b)** `cluster_cull.comp.spv` byte-equals its re-DXC **with no `-D`** — i.e. the base arm is
    physically unperturbed by the seam. (Measured on a probe: adding the `#ifdef HIER` push member
    *and* a full HIER arm leaves the no-`-D` compile at the identical 12 392 B /
    sha256 `dbb924967b1176af…`.) Note this is against the **H1.6-re-pinned** blob, not Rev 2's.
  * **(c)** every golden pin unchanged (nothing selects the variant).
  * **(d)** `cargo clippy --workspace --all-targets -- -D warnings`.
  * **(e) NEW — the structural tripwire** (`cluster_cull_hier_dis_gate.rs`, cloning the `spirv-dis`
    locator and skip semantics of `crates/boyko_rhi_vulkan/tests/field_probe_gate.rs:43-105`,
    precedent documented at `shaders/sdf_field.hlsli:146-148`). It re-DXCs both variants **into the
    temp dir** — never overwriting a committed artifact — disassembles, and asserts, all measured as
    exact integers on the real shader:
    * on the `-D HIER=1` module: `OpDot` count **== 8** (the `dot(rd, cam_forward.xyz)` in
      `view_z_to_t` `:87`, 4 corners × near/far — i.e. **zero** in the cull comparison);
      `OpDecorate … NoContraction` count **== 10** (5 per call site); the id-normalised
      14-instruction window ending at each of the two `OpFOrdLessThanEqual` is byte-equal to the
      other; the push block has **5 members with `Offset 16`** on `cluster_dims_packed`; **exactly
      one `OpReturn`**; every `OpControlBarrier` sits in a **merge** block.
    * on the base module: `OpDot == 8`, `NoContraction == 5`, one `OpFOrdLessThanEqual` window, push
      block 4 members with last `Offset 12`.
    * The test's doc-comment **must state that it is a tripwire, not the proof** — the proof is §5
      Step 0 — because contraction is decided below the `.spv` (DXC emits zero `Fma`).
  * **(f)** the two `#error` guards fire when `HIER_MASK_WORDS` is set to `64` (executed once during
    review; verified against dxc 1.4.350.0).
* **RED if:** the base `.spv` moves by one byte (the seam leaked into the `#else` arm); any (e)
  count differs; the manifest row is missing (the `-D` matrix must stay enumerable by one grep).

### H3 — The GPU set-level equality oracle `[P0-2]`, `[P0-3]`
* **Files:** `crates/boyko_rhi_vulkan/tests/cluster_cull_hier_equiv.rs` (new) + a **cull-only**
  driver: camera UBO + light table + the three buffers + one dispatch + three readbacks. It does
  **not** go through `run_gbuffer_hybrid_lit_clustered` (`sdf_gbuffer_hybrid.rs:5276`) — no SDF, no
  resolve, ~10× faster, and it can drive a PERSPECTIVE camera trivially. The driver creates
  `LightIndexAlloc` as `MemoryLocation::HostVisibleCoherent` and reads `alloc_total` through
  `buffer_mapped_ptr` after the fence, following `tests/sdf_gbuffer_hybrid.rs:5415-5425` — **no
  `vkCmdCopyBuffer`, no staging buffer, no framegraph resource** (§6, §P1-E). This is where §6's
  exact detector lives.
* **Why a new test rather than extending `l1_known_light_lands_in_the_expected_clusters`
  (`sdf_gbuffer_hybrid.rs:6432`):** that test is **ORTHO-only** (`CompositeCamera::Ortho`, `:6455`)
  and drives the *base* pipeline. Extended naively it would exercise the flat arm and pass green
  while testing nothing about the hierarchy — the exact failure mode `[P0-2]` names. It stays as-is
  (the flat arm's host cross-check); the hierarchy gets its own oracle that *cannot* be satisfied by
  the flat arm.
* **Asserts, per config (same matrix as H1, plus both arms on-device):**
  1. `alloc_total < index_list_cap` — the saturation precondition (§6). **Fails loudly**, does not
     silently compare clamped results.
  2. Per-froxel `count` equal between arms.
  3. Per-froxel `LightIndexList[offset .. offset+count)` **equal as a sequence** (order included),
     between arms.
  4. Both arms equal to the host `golden_cluster_cull` set (per-froxel, as a set). After H1.6 the
     host↔GPU distance comparison is **structural** (D10), not incidental; the arm-vs-arm comparison
     is exact because both run the same fine test on the same device.
  5. **Totality of `ClusterGrid`:** pre-fill the grid with `0xFFFFFFFF`; after the cull no cell
     retains the sentinel ⇒ every froxel was written exactly once by the block decomposition
     `[P0-4a]`.
  6. Non-vacuity: at least one froxel non-empty, and the hier pipeline handle is asserted distinct
     from the base one.
  7. **NEW — the no-skew precondition (D11, D4 scope clause (b)):** the boot snapshot equals the
     live header dims, asserted as loudly as assertion 1, so no test can silently run skewed.
* **GPU matrix.** E2 (32×16×24, `gps=2` exact) and E3 (16×17×24, `gps=2` ragged) **must run on
  device** — the failure D11 names is an out-of-bounds *device* write with `robustBufferAccess` OFF,
  which a CPU mirror cannot exhibit. E4 (32×24×24, `gps=3`) **may stay CPU-only**, because it tests
  index arithmetic rather than device behaviour; this is stated explicitly rather than left
  ambiguous. Add the degenerate-header config (`packed_dims == 0`) as a hang / divide-by-zero probe.
  Keep the `N = 0` row: with no point/spot lights, `ps_n == 0`, the coarse loop body never runs,
  `gs_summary == 0`, the fine walk is empty and every froxel writes `uint2(0,0)` — covered by
  simulation as arithmetic, but the barriers on that path are only executed here.
* **RED if:** any of 1–7 fails. **Concrete mutations that must turn it red, each to be executed once
  during review:**
  * **(i)** drop the `valid` guard on phase 6 → totality / duplicate writes;
  * **(ii)** replace the reduction's `min` with the lane-0 value → non-conservative coarse box → set
    mismatch on the adversarial rig;
  * **(iii)** walk mask words descending → order mismatch on a multi-light froxel;
  * **(iv)** force `ps_n = 0` in the fine arm while leaving the coarse phase's mask writes intact,
    **and** delete the `j >= ps_n` clamp → out-of-range candidate indices in the readback.
    *(Rev 2's wording — "inject a synthetic out-of-range bit" — no longer names a reachable state,
    because D7's clamp is at the loop.)*
  * **(v)** delete the `fi < capacity` term, boot at **16×9×23** and raise the live `ClusterConfig`
    to 16×9×24 before the frame → with validation ON the buffer-overrun must be reported and the
    `0xFFFFFFFF` sentinel probe must show writes past the last cell;
  * **(vi)** transpose the group mapping to `slice = gid % gps; s = (gid/gps)·256 + lane` → must turn
    **RED on E2/E3/E4** via the sentinel, and must (correctly) stay **GREEN on every 16×9×24 entry** —
    which is the demonstration that Rev 2's matrix was blind;
  * **(vii)** delete the finiteness fold and run the deliberately-degenerate-camera config. This one
    asserts **only** that the hier arm does not hang, writes every `ClusterGrid` cell and emits no
    out-of-range index — **not** that it matches the flat arm (D4 scope clause (c)).

### H4 — Arm the variant for VB + the two-rig bench
* **Files:** `crates/boyko_app/src/gpu_scene/mod.rs` (pipeline choice + the
  `ClusterCullHierDispatch` write in `build_froxel_light_cull`, `:4241`, beside `:4346`),
  `crates/boyko_rhi_vulkan/src/present/scene_types.rs` (the new `Option` field),
  `crates/boyko_rhi_vulkan/src/compute.rs` (the second push mirror, if not already landed at H2),
  `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:184` (the one `match`; the `// SAFETY:` clause
  rewritten per D11), `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`
  (4 literals at `:2387, 3434, 8420, 9905` gain `cluster_cull_hier: None`),
  `crates/boyko_render/src/light.rs` (`hier_group_threads`/`hier_group_count`),
  `crates/boyko_app/src/runner.rs:1951` (the debug-only boot/live dims assert),
  `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs` (+ `BOYKO_VB_BENCH_RIG=kronecker|r3|infrustum`,
  default `kronecker` so the existing provenance is reproducible verbatim).
  **The framegraph is untouched** — same buffers, same passes, same accesses, same seeds `[P0-1]`.
* **The new rigs** (§1.4): `r3` = the plastic-constant 3-D Kronecker sequence
  `α = (1/φ₃, 1/φ₃², 1/φ₃³)`, `φ₃ = 1.220744084605760` (the root of `x⁴ = x + 1`) — genuinely
  3-D equidistributed, unlike `g/g²/g³`; `infrustum` = stratified inside the view frustum
  (screen `(u,v)` × depth `d ∈ [3,12]` mapped through the camera basis), so density *rises* with `N`
  instead of leaking out of frustum.
* **Gate:** (a) `vb_mesh_froxel` and `vb_mesh_tex_froxel` pins **byte-identical, no re-pin**;
  (b) the §2 numeric table met on the `kronecker` rig; (c) the full sweep published for all three
  rigs into `light_policy.rs`'s provenance comment **as additional data — `CLUSTER_LO`/`CLUSTER_HI`
  are NOT changed** (§1.4.3); (d) **the §7.0 discrimination run**: a `dim_z` sweep at fixed `N`,
  recorded in this document, deciding between Reading A and Reading B; (e) the §5 mitigation's cost
  measured — `froxel_cull_ns` at `N ∈ {8, 512}` with and without the two-line finiteness predicate.
* **RED if:** a froxel pin moves (⇒ the set or the order differs, or a cap saturated — H3's
  precondition should have caught it, so a moved pin here means the pin's scene saturates and must be
  diagnosed by reproducing its light table under H3's cull-only driver and reading `alloc_total`
  there); any §2 threshold missed; or §7's µs column proves to have been the wrong reading and the
  re-derived prediction misses §2.

### H5 — (conditional on H4) migrate Deferred + ForwardPlus to the hierarchical arm
* **Files:** `present/passes/gbuffer.rs:1583`, `present/passes/forward.rs:359` (the `match` + the
  `// SAFETY:` clause, which is byte-for-byte the same text as `vb.rs`'s and must get the same
  rewrite), `gpu_scene/mod.rs`. Retire the flat arm to test-only status (it remains the equality
  oracle's reference forever).
* **Gate:** every Deferred/ForwardPlus golden byte-identical; `l1_known_light_lands...` and
  `l1_clustered_resolve_matches_the_brute_force_image` green; `forwardplus_mesh` green.
* **RED if:** any of the above moves. **Precondition:** H4 shows a win on *both* the `kronecker` and
  `infrustum` rigs; a win only on the out-of-frustum rig does not justify migrating shipped paths.

---

## 9. Validation plan (consolidated)

| Requirement | Mechanism | Where | Can it actually fail? |
|---|---|---|---|
| `[P0-1]` framegraph seeding | **No new buffer and no new framegraph access is introduced** — the coarse mask is groupshared, and H0's `LightIndexAlloc` readback was **removed** from the present path (§P1-E), so this row is now *true* rather than *defended*. `alloc_total` comes from H3's host-visible cull-only driver and H1's CPU oracle (§6). The existing trio keeps the `add_buffer_seeded` seeds landed at `5e07936` (`graph_bridge.rs:3179-3190`; the `light_cull` pass `:3212-3242`). Any future rung that wants a *live* counter must follow §11's recipe — and must **not** construct a hybrid `ResSync` seed, because `framegraph/sync.rs:288-296` takes the flush branch first and silently discards the visible-stage WAR half. | design | n/a (nothing new declared); `framegraph_gbuffer_equiv` still covers the trio |
| `[P0-2]` tests that can fail | H3's oracle drives the **hier** pipeline explicitly on a **PERSPECTIVE** camera and asserts the hier handle ≠ base handle; seven named mutations are executed during review | `tests/cluster_cull_hier_equiv.rs` | yes — mutations (i)–(vii) in H3 |
| `[P0-3]` set-level oracle | Per-froxel index **sequence** equality between arms + against the host oracle, not an image hash. Image pins are a secondary no-regression gate only. | H1 (CPU, exhaustive) + H3 (GPU) | yes — a single dropped marginal light fails, where an 8-bit image hash would not |
| `[P0-4a]` totality | Groupshared mask is re-initialised **every dispatch** by lanes 0..31 unconditionally — there is no cross-frame state to go stale. `ClusterGrid` totality proven by the `0xFFFFFFFF` pre-fill probe. | D1, H3.5 | yes — drop the init or the `valid` guard |
| `[P0-4b]` range clamp | **One clamp `ps_n` bounds the coarse groupshared WRITE and both device READS**; the fine arm re-checks the identical `j < ps_n` (D7). Locally evident at both sites, no cross-phase argument, no device value in the derivation. | shader (D7) | yes — H3 mutation (iv) |
| **`[P0-A]` same-expression premise** | **Discharged, not disclaimed:** D10 makes `sq_dist_point_aabb` a written-out `precise` sum of correctly-rounded, `NoContraction`-decorated ops, so both call sites evaluate one function `F` (§5 Step 0). H2 gate (e) is a **tripwire** (`OpDot == 8`, `NoContraction == 10`, identical id-normalised windows) — explicitly *not* the proof, because DXC emits zero `Fma` and contraction is decided below the `.spv`. | §5 + D10 + H1.6 + H2(e) | yes — H2(e) fires if `dot()` returns or `precise` is dropped; H1.6 catches the ULP fallout |
| **`[P0-B]` total bound** | `valid = (s < bdx·bdy) && (slice < bdz) && (fi < capacity)`, with `capacity` from the **boot** push (D11). Dispatch size, allocation and write bound are three evaluations of one u32. A live-header disagreement **cannot move `fi` at all**. | D3/D11 + H1.7 + H3.7 | yes — H3 mutations (v), (vi) |
| `[P1]` FP margin | **Deleted, not bounded**: D2 makes enclosure a monotonicity theorem (§5) with no epsilon. Finiteness is now **ENFORCED on device** by D8's identity substitution; H1.4 additionally asserts it on the host. | §5 + D8 + H1 | yes — H1's finiteness assert, H3 mutations (ii) and (vii) |
| `[P1-3]` cap saturation | §1.3's measured table + the exact `alloc_total ≤ cap` detector asserted as a precondition of every equality run, read from H3's **host-visible** counter | §6, H1.3, H3.1 | yes — the equality test aborts loudly instead of comparing clamped results |
| wave/subgroup coherence | No wave intrinsics used, and **no barrier elided on an assumed wave width** — the RHI sets `subgroup_size_control: VK_FALSE` (`device.rs:2584`) and queries `subgroupSize` nowhere. Phase 5's trip count is **group-uniform** (all lanes walk the same mask) ⇒ zero loop divergence; only the append predicate diverges, exactly as today. | design (D1, D9) | n/a |
| dispatch shape | One boot u32 drives the dispatch size, the allocation and the in-shader write bound; a live-header disagreement cannot move `fi`. **H1.7** pins the derivation on the CPU over six grid configs incl. degenerate ones, **H3.7** asserts no-skew on device, and a `debug_assert` at `runner.rs:1951` catches an owner edit. | H1.7, H3.7, H4 | yes — H3 mutations (v), (vi) |
| barriers | **3 total** (B1, B2, B3), stated once in D1 and footed in §4. H2 gate (e) asserts each `OpControlBarrier` sits in a merge block and that there is exactly one `OpReturn`. | D1, D9, §4, H2(e) | yes — a non-uniform barrier hangs H3 on device |
| occupancy / groupshared | **6 276 B/group, exact by construction** (six scalar `float[256]` + 32-word mask + summary; D9). `float3` is avoided because the emitted module carries **no `ArrayStride`** for `Workgroup` storage, so its stride is driver-chosen (6 276 B or 8 324 B) and not derivable from the artifact. 24 groups against Ampere's 100 KB/SM — not a limiter. `local[256]` is **unchanged** (§1.1). | design | measured at H4 |
| `unsafe` discipline | The rung adds no new Rust `unsafe`; the record-site changes are inside existing `unsafe` blocks whose `// SAFETY:` comments are **rewritten** (D11) — the old "`cull_groups` covers `cluster_count` froxels" is a coverage claim, the wrong obligation, and false for the HIER map. | H4 (`vb.rs`), H5 (`gbuffer.rs`, `forward.rs`) | clippy `-D warnings` + review |

---

## 10. Risks and the ABORT criterion

| Risk | Mitigation |
|---|---|
| The `0.2736 ns/pair` rate does not transfer to the new dispatch shape | **H1.5** bounds froxel-count scaling on the existing flat arm with no shader work; H4 measures the real thing; the abort threshold is in measured ns |
| **H1's selectivity gate is mistaken for a wall-clock predictor** | H1 is explicitly a *necessary* condition on pair count (§7.0). H1.5 bounds thread-count scaling. Barrier cost, the 43.75 % idle fine phase, hot-group serialization and the `ClusterGrid` write pattern are settled only at H4, against §7.0's **pre-registered** prediction |
| The 13.9 µs fixed cost is dispatch-intrinsic, not barrier-intrinsic ⇒ low-`N` gains vanish | H0 measures it *first*; it changes only the low-`N` prediction, not the `N ≥ 128` win |
| Load imbalance (3 hot groups of 24, 85.2 % of the fine work) leaves the GPU idle | The hot group's work is irreducible; imbalance is a *symptom of having removed the other 97.5 %*. §7.0 pre-registers the `dim_z`-sweep discrimination; if Reading B holds, the follow-up is a second in-group level (§11), not a redesign |
| A barrier reached under non-uniform control flow ⇒ device hang | D8 is a named code-review gate item (now with two extra obligations: `max(1u, …)` on `gps` and the `bdx != 0u` guard); H2 gate (e) asserts one `OpReturn` and merge-block barriers; H3 runs on-device and a hang is unmissable |
| The equality oracle is run in a saturating configuration and silently compares clamped results | H3.1's `alloc_total` precondition fails the test loudly (§6) |
| Byte-identity claim over-reached | Explicitly scoped by D4's **five** clauses: non-saturating, no boot/live skew, finite AABBs, one shared distance function, output buffers not access patterns |
| **The D10 edit perturbs the base arm's `sq_dist` by ≤1–2 ULP and flips an exactly-tangent light in an existing golden** | Detected by H1.6's re-run of `cluster_cull_spv_sync`, `lighting_l1_host_oracle`, the two L1 GPU oracles and the image goldens; mitigated by re-pinning **with a recorded ULP explanation**, never silently. The flip requires a light within ~1e-7 relative of exact tangency — measure-zero in principle, not provably zero on a procedural rig |
| **The D10 edit's ALU cost is unmeasured** (2 extra ops per pair test on a path whose wall clock tracks pair count) | H1.6 records `froxel_cull_ns` at `N_ps ∈ {128, 512}` before and after **in this document**; on a regression beyond noise, fall back to the named alternative — slack `r*r*(1.0 + 0x1p-20)` on the coarse comparison only, base arm untouched |
| **An owner edits `ClusterConfig` post-boot in a release build** | The HIER arm cannot fault (D11: `fi` never moves), only mis-shape the grid. `debug_assert` at `runner.rs:1951` catches it in debug; H3.7 catches it in tests. Making it loud in release means disarming the cull for that frame — a VALUES call, tracked as **VB-P1k** |

**ABORT (revert exactly as the two-pass attempt was reverted) if any of:**

1. **H1**: pair-count selectivity < 4× on both the `kronecker` and `infrustum` rigs at `N ≥ 128`.
   *(Costs zero GPU time and zero shader code — this is the cheap kill switch, and it is a
   necessary-condition test only.)*
2. **H1.5**: a latency floor is found and the re-derived `N=512` prediction exceeds §2's 250 000 ns
   threshold. *(Still no shader written.)*
3. **H3**: any per-froxel index sequence differs between arms in a non-saturating, non-skewed,
   finite-AABB configuration.
4. **H4**: `froxel_cull_ns` at `N_ps=512` > 250 000 (< 2× win), **or** any of `N_ps ∈ {8, 32, 64}`
   regresses `froxel_total_ns` by > 10 %, **or** any froxel golden pin moves.

A partial result — e.g. a large win at `N ≥ 128` and a 5 % loss at `N = 8` — is **not** an abort: the
`Auto` policy band already disarms clustering below `CLUSTER_LO`, so the low-`N` arm is not the
shipping configuration. It must, however, be reported in the provenance table rather than smoothed
over.

---

## 11. Tracked follow-ups (explicitly out of scope for VB-P1e)

* **VB-P1f — re-tune `CLUSTER_LO`/`CLUSTER_HI`.** Owner-gated. Requires H4's two-rig sweep. Until it
  lands, `Auto`-mode scenes with 64 < `N` < 128 keep the flat path and see no VB-P1e benefit
  (§1.4.3). Must also fix the false "mutually irrational" doc-comment
  (`vb_p1d_cull_shade_bench.rs:114-123`) — §1.4's proof belongs next to the code it refutes.
* **VB-P1g — attack the fixed cost** (the only route to a single-digit break-even, §7.1): delete the
  `cmd_fill_buffer` + `TRANSFER→COMPUTE` barrier by resetting `LightIndexAlloc` from within the
  previous frame's cull (or a per-FIF alloc ring), and/or fold the cull into the shade dispatch's
  prologue. Gated on H0's attribution.
* **VB-P1h — a second in-group level** (per-16-lane sub-block masks), only if H4 shows the fine phase
  or Reading B dominating. Output-neutral by D2's corollary, so it is a pure perf experiment.
* **VB-P1i — wave-intrinsic reduction** (`WaveActiveMin`/`WaveActiveBallot`). Output-neutral by D2's
  corollary. **Concrete precondition, verified:** the RHI sets `subgroup_size_control: VK_FALSE`
  (`device.rs:2584`) and queries `subgroupSize` nowhere, so VB-P1i must first add the device-feature
  query.
* **VB-P1j — give the BASE arm the same capacity bound.** Its total bound is
  `min(64·ceil(boot_cc/64), live_cc)`, which exceeds `boot_cc` when `boot_cc % 64 != 0` **and** the
  live dims grow: measured **16 cells (128 B) past the end of `ClusterGrid`** at boot 16×9×23 / live
  16×9×24. Two owner actions required (vs one for D3-as-written), bounded by 63 cells (vs unbounded).
  It needs its own base `.spv` re-pin, which is why it is not folded into H1.6.
* **VB-P1k — decide whether a detected boot/live `ClusterConfig` skew should disarm the cull in
  release** (`cluster_cull = None` for that frame). Owner/VALUES call, not a safety requirement.
* **A `safe_normalize` in `ray_gen.hlsli`** — the true root fix for the device NaN of §5's first
  source. Deliberately **not** smuggled into VB-P1e: `ray_gen.hlsli` is included by the marcher and
  the deferred PBR resolve, so the change re-DXCs and moves every dependent committed `.spv` and its
  byte pin.
* **A non-degenerate basis fallback in `ViewUniform::from_camera`** (mirroring the identity fallback
  it already gives `view` at `camera.rs:331-336`: fall back to the canonical right/up/forward when
  any of the three normalizes to ZERO), plus **release-visible `z_near > 0 && z_far > z_near`
  validation in `ClusterCullPush::new`** (`compute.rs:3473-3483`) — today the only check is a
  `debug_assert!` in a different function (`ClusterConfig::z_scale`, `light.rs:738-743`). Defence in
  depth for §5's two NaN sources; neither replaces the on-device mitigation, which closes the *class*
  regardless of source.
* **A live `alloc_total` HUD counter** so saturation is visible in ordinary runs. Rev 2 listed this
  as a one-liner; it is not, and the recipe is written out here so it is not re-attempted cheaply:
  * a **new** graph pass `cull_alloc_readback`, declared immediately after `light_cull`, gated on the
    same 4-buffers-`Some` predicate **and** a new `Option<&BoundBuffer>` bench-staging field on
    `GBufferScene` (the `vb_gpu_timing` gating precedent verbatim, `gpu_timing.rs:232-240`: `None` ⇒
    zero declared accesses ⇒ byte-identical command stream);
  * its single access `g.buffer_access(light_index_alloc, VK_PIPELINE_STAGE_TRANSFER_BIT,
    VK_ACCESS_TRANSFER_READ_BIT)`, letting the graph derive the
    `src=(COMPUTE, SHADER_WRITE) → dst=(TRANSFER, TRANSFER_READ)` availability barrier. **Do not
    hand-roll it** — a hand-rolled barrier leaves `ResSync` describing a state the command stream no
    longer has, and the *next* derived access is then wrong. (`present_blit.rs:335-400` is the right
    precedent for the `Option`-gating half only; the swapchain image is not a graph resource,
    `light_index_alloc` is.);
  * usage is already legal: `create_buffer` unconditionally ORs `TRANSFER_SRC | TRANSFER_DST` onto
    every `DeviceLocal` buffer (`rhi_impl/device.rs:52-58`), which is the *only* reason the copy is
    legal, since `light_index_alloc` is created with `BufferUsage::STORAGE` alone
    (`gpu_scene/mod.rs:4332-4338`);
  * the staging destination is a 4-byte `HostVisibleCoherent` buffer created **once at boot**,
    per-frame-in-flight (`[BoundBuffer; FRAMES_IN_FLIGHT]`) — never per frame (Principle 5);
  * **seed hazard, stated:** leave the declared seed as
    `ResSync::seeded_writer(COMPUTE_SHADER, SHADER_WRITE)` and pin, in a comment at the declaration
    site, why the stale seed is not a WAR race here — the bench loop calls `ctx.wait_idle()` on every
    armed frame before reading (`runner.rs:2069`), so no two frames overlap while the readback pass
    exists — with an assert next to the arm site that the `wait_idle` is present. Switching the seed
    to `seeded_readers(TRANSFER, TRANSFER_READ)` is *sound without* the `wait_idle` but changes the
    fill's in-barrier from a COMPUTE→TRANSFER memory dependency to a TRANSFER→TRANSFER execution-only
    one, i.e. it removes a compute drain from inside the `CullReset` bracket and **biases H0's
    fixed-cost number low**; if used, the alloc-readback mode and the fixed-cost attribution run must
    be declared **mutually exclusive**. A **hybrid** seed is forbidden outright: `transition()`
    (`framegraph/sync.rs:288-296`) takes the flush branch first and drops the visible-stage WAR half.
  * record the copy **after** the `CullDispatch` timestamp closes so neither sub-bracket is
    perturbed.
* **A specialization constant instead of D11's push word** — output-neutral, re-openable; rejected
  here on economy and precedent (D11).

---

## 12. Required before Rev 3 can be approved (one commit, ahead of re-review)

**Land §1.3's occupancy probe as a real test.** Rev 2 anchored the document's self-declared "single
most important table" — and, through it, §7's entire fine-pair column, §6's saturation discharge and
§10's abort criterion — on `scratchpad/cap_probe.rs.txt`, which **is not in the repository**
(verified: `git ls-files | grep -i cap_probe` → no match; no tracked path contains `scratchpad`).
Prose cannot re-derive a measured table, and Rev 3 must be reviewable *before* H1 is written.

* **Where:** one added `#[test]` in `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (it
  already exists and is already H1's named target file — no new file, no new fixture, no GPU).
  *Rejected:* committing the probe as a `scratchpad/` text file (an untested, unrun artifact that
  rots exactly like the doc-comment §1.4 refutes); a new `cluster_cull_occupancy_probe.rs` (the host
  oracle domain is already owned by `lighting_l1_host_oracle.rs`).
* **What it drives:** `golden_cluster_cull` (`crates/boyko_rhi_vulkan/src/goldens.rs:3510`) with the
  VB-P1d camera (eye `(0, 1.1, 7.8)` → `(0, 0.55, 0)`, `fov_y 52°`, aspect 1.0, 512×512) and the
  bench rig reproduced from `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs:124` (`light_position`)
  and `:142` (`light_range`), against `ClusterConfig::default()`.
* **What it asserts:** for `N_ps ∈ {8, 14, 32, 64, 128, 256, 512, 1024}`, exactly §1.3's three
  columns — `total_indices`, `non_empty_froxels`, `max_per_froxel` — as **literal expected values**,
  plus `total_indices < INDEX_LIST_CAP` and `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (which is
  simultaneously §6's saturation discharge).
* **Then:** re-anchor §1.3 and the appendix on this test's `file::test_name`. It is provenance for
  the *plan*, not for the implementation, which is why it lands as its own commit rather than inside
  H1.

---

## Appendix — source anchors

| What | Where |
|---|---|
| The cull shader (hand-authored, no eDSL sentinels) | `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` — dispatch shape `:107`, early return `:112-114`, AABB build `:126-153`, `local[256]` `:159`, flat light loop `:161-175`, per-froxel cap `:170`, claim+write `:180-194`, `index_list_cap` clamp `:184-190`, `sq_dist_point_aabb` `:102-105` (D10 rewrites this), `view_z_to_t` `:85-91` (the 8 residual `OpDot`s), `slice_view_z` `:77-79` |
| Frozen dxc recipe (no `-D`, no `-O`) | `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:45-53`; the shader's own header `cluster_cull.hlsl:25-27`; text-pin idiom `:20-22` |
| Shared ray-gen (and §5's NaN source 1) | `crates/boyko_rhi_vulkan/shaders/ray_gen.hlsli:44-75`, `normalize(dir)` `:63-67`; host side `crates/boyko_scene/src/camera.rs:325-327`, `:331-336`; `crates/boyko_math/src/vec.rs:226-233` |
| Cluster linearization / params (one source of truth) | `crates/boyko_rhi_vulkan/shaders/light_table.hlsli:313-323, 329-331`; `LightHeader` `:223`, `load_light_header` `:243-248`, `load_light` `:255-265` (no bound check), `light_kind` `:271-273` |
| Host constants + config | `crates/boyko_render/src/light.rs:41-61` (`CLUSTER_DIM_*`, `MAX_LIGHTS:51`, `MAX_LIGHTS_PER_CLUSTER`, `INDEX_LIST_CAP:57`), `:691-770` (`ClusterConfig`; `cluster_count()` `:728`, `z_scale` debug-assert `:738-743`, `packed_dims` `:763-772`) |
| The live-header gate (D11's divergence source) | `crates/boyko_render/src/light.rs:783-786, 792-793, 830-851`; the prior UB-class ruling `crates/boyko_app/src/plugins.rs:352-363` and `light_system.rs:410`; the Resource seed `plugins.rs:195` |
| Release-clamped host fold (D6's unreachability argument) | `crates/boyko_render/src/light_system.rs:199-210, 212-300` (gates at `:263, 272, 282, 291`; the `debug_assert!` at `:300`); `LightHeaderGpu::new` debug-assert `light.rs:1082` |
| Boot snapshot (D11) | `crates/boyko_app/src/runner.rs:636-643`; `crates/boyko_app/src/gpu_scene/mod.rs:4241` (`build_froxel_light_cull`), `:4304` (spec constants), `:4317-4324` (`ClusterGrid` sizing), `:4325-4331` (`LightIndexList` sizing), `:4332-4338` (`LightIndexAlloc`), `:4346` (`cluster_count` freeze), `:5237`, `:5307` (scene threading); light-table capacity `:205-207` |
| SSAA host-authoritative-lock precedent (D11's debug assert) | `crates/boyko_app/src/runner.rs:1919-1940`; the `scene()` call site `:1951`; the bench `wait_idle` `:2069` |
| Measured band + provenance table | `crates/boyko_render/src/light_policy.rs:40-77` (reproduces §1's six rows verbatim) |
| Host cull oracle | `crates/boyko_rhi_vulkan/src/goldens.rs:3510` (+ `GoldenClusterConfig` `:3398-3411`, `golden_cluster_index` `:3437-3441`, `golden_sq_dist_point_aabb` `:3488-3498`) |
| "Bit-exact" claims that D10 makes structural | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:5187-5188`, `:5199-5202`, `:6291-6293`; the DDGI precedent `crates/boyko_rhi_vulkan/shaders/ddgi_resolve.hlsli:136-143` |
| VB framegraph declaration (seeded trio, `5e07936`) | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:3179-3190`; the `light_cull` pass `:3212-3242`; barrier derivation `crates/boyko_rhi_vulkan/src/framegraph/sync.rs:157-169` (`seeded_readers`), `:198-208` (`seeded_writer`), `:266-330` (`transition`, flush branch `:288-296`) |
| VB record site (fill → barrier → dispatch, timestamps) | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-219` — timestamp open `:146-150`, the `if let` gate `:157`, fill `:170-174`, `record_vb_pass` `:180`, group count `:184`, `// SAFETY:` `:185-190`, dispatch `:211`, timestamp close `:216-219`. Siblings: `gbuffer.rs:1583` (`// SAFETY:` `:1587-1588`), `forward.rs:359` (`:363-364`) |
| Scene plumbing (D11) | `crates/boyko_rhi_vulkan/src/present/scene_types.rs:415` (`LIGHT_CULL_LOCAL_SIZE_X`), `:438` (`BrickActivation`, the activation idiom), `:1409-1411` (`cluster_count`, and no dims); push mirrors `crates/boyko_rhi_vulkan/src/compute.rs:3447-3496` (const-asserts `:3467-3471`, `ClusterCullPush::new` `:3473-3483`), `cluster_cull_spirv()` `:1610`, camera push `:3005-3015`; test literals `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs:2387, 3434, 8420, 9905` |
| Existing ORTHO-only cull oracle + host-visible readback idiom | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:6432` (fixture `:5215`, cap `:5230`, driver `:5276`, ORTHO `:6455`); mapped `LightIndexAlloc` read `:5415-5425` |
| Bench-armed capability whose absence is byte-identical | `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:232-240`; boot gate `crates/boyko_app/src/gpu_scene/mod.rs:3554-3571`, consumed `:5938-5949`; `Option`-gated readback precedent `crates/boyko_rhi_vulkan/src/present/passes/present_blit.rs:335-400`; `TRANSFER_SRC\|DST` OR `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:52-58` |
| Subgroup features (VB-P1i's precondition) | `crates/boyko_rhi_vulkan/src/device.rs:2584` (`subgroup_size_control: VK_FALSE`); FFI fields `ffi.rs:2623, 2624, 2691, 2703`; `robustBufferAccess` bit `ffi.rs:2718` (never enabled) |
| `.spv` byte gates + `spirv-dis` gate idioms (to clone) | `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:63-88`; multi-variant `crates/boyko_rhi_vulkan/tests/vb_froxel_spv_sync.rs:88-130`; disassembly gate `crates/boyko_rhi_vulkan/tests/field_probe_gate.rs:43-105` (precedent documented at `shaders/sdf_field.hlsli:146-148`) |
| Variant manifest | `docs/SHADER-VARIANT-MANIFEST.md:91-97` |
| The bench | `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs` — rig `:114-144` (the false "mutually irrational" claim `:122-123`, literals `:133-135`, `light_range` `:142`), camera `:235-254`, `ClusterConfig::default()` insertion `:278` (H1.5's hook) |
| §1.3 occupancy pin (to be landed by §12) | `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (the fixture `cfg()` `:28-37`) |






