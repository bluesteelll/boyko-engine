# VB-P1e — hierarchical froxel light cull (implementation plan, Rev 2)

**Status:** DESIGN, Rev 2 — **NOT APPROVED. DO NOT IMPLEMENT.** The architecture-critic reviewed this
revision and returned **CHANGES REQUESTED (3 × P0, 5 × P1, 7 × P2)**; a Rev 3 must land and pass
re-review first. Rev 2 *did* discharge four of Rev 1's five findings (verified against code, see the
table in each section) — the open blockers are:

* **P0-A — §5's proof has a hole, and it is the document's load-bearing claim.** Step 2's first four
  links are sound, but the last one is `dot(d,d)` — a single `OpDot` whose accumulation order and
  FMA contraction are fixed by neither SPIR-V nor GLSL.std.450. The frozen recipe passes **no `-O`**
  and DXC defaults to `-O3` (verified: `cluster_cull_spv_sync.rs:45-53` and this shader's own header
  recipe), so the coarse and fine call sites are inlined and optimised *independently*. For the
  boundary froxel that defines the group extremum, `d_coarse == d_fine` bit-for-bit and the whole
  margin collapses to the difference between two possibly-different instruction sequences. §5
  therefore **does** need the same-expression-tree premise it explicitly disclaims. It is cheap to
  discharge (a `spirv-dis` structural assertion at both call sites in H2, or an owned 1-ULP-class
  slack on the coarse comparison only, whose selectivity cost is nil) — but it must be discharged,
  not disclaimed.
* **P0-B — D3 drops a total bound the base arm has today.** The base shader guards
  `fi >= cluster_count` from the *live* header dims (`cluster_cull.hlsl:112-114`), while the dispatch
  size is a *boot* snapshot; `sync_cluster_light_gate` exists precisely because those two diverge.
  D3's only guard is `valid = (s < dim_x·dim_y)`, so a post-boot grid change turns a benign case into
  an **out-of-bounds device-buffer write** (`robustBufferAccess` is OFF). Compounding it,
  `GBufferScene` carries only `cluster_count`, no dims — so §4's "record sites dispatch
  `hier_group_count()`" is not implementable as written.
* **P0-C — the coarse mask write is unclamped, and D6/§4 disagree on the index space.** §4 phase 3
  indexes `gs_mask[i>>5]` by the *absolute* light index while D6 defines the defensive tail
  relative to `l0a_count`; implemented literally, a range is covered by neither and phase 3 writes
  past `gs_mask[31]` — a groupshared OOB write that lands on `gs_min`/`gs_max` and silently corrupts
  the coarse box for the whole group.

Two of Rev 2's arithmetic slips are also confirmed and must be corrected in Rev 3 (neither flips a
decision): §D3's `TPG=128` row should read **48 groups** (`ceil(144/128)·24`), not 36; and §7.1's
break-even floor is `N > 14.89`, not 14.4.

Sub-plan of
[VB-PERFORMANCE-TRACK.md](VB-PERFORMANCE-TRACK.md) §4 (VB-P1). Sibling of
[VB-P2-CLASSIFICATION-PLAN.md](VB-P2-CLASSIFICATION-PLAN.md). Base commit `5e07936`
(`feat/multi-paradigm-render`).

**One-line verdict:** the cull is *pure rejection work* — at `N_ps=512` only **85 of 3456 froxels
(2.5 %)** hold any light, yet all 3456 threads scan all 512 lights and the pass costs **498 µs**. A
**single-dispatch, workgroup-local two-level cull** removes ~95 % of the (froxel, light) pair tests
with an **exactness proof that needs no floating-point epsilon**, and it introduces **no new GPU
buffer, no second dispatch, and no second barrier**. Predicted cull at `N_ps=512`: **≈ 23 µs
(nominal) / ≈ 50 µs (2× pessimistic)**, break-even **≈ 16–30** (from the measured ≈ 103).
**Single-digit break-even is arithmetically impossible at the present fixed-cost floor** — §7.1 proves
it and names the follow-up that would be needed. The whole perf premise is **falsifiable on the CPU
in 0.45 s at rung H1, before one line of shader is written.**

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

### 1.2 The empirical cost model (fitted, ≤ 8.9 % error over 6 samples)

Anchored on `N=128` and `N=512`:

```
cull_ns(N)       ≈ 13 939 + 0.2736 · (froxels · N)   = 13 939 + 945.6·N   at froxels = 3456
flat_shade_ns(N) ≈ 23 922 + 1 109.6·N
froxel_shade_ns  ≈ 26 500 ± 2 200   (no trend in N)
```

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
> decisions and go/no-go bounds only*; the shipping decision is a measurement (H4), and the abort
> threshold in §10 is expressed in measured nanoseconds.

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

**Numerically verified** (orchestrator, over `i ∈ [1, 1024]` at the literals the source uses):
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
| break-even (`froxel_total < flat_shade`) | ≈ 103 | **≤ 40** measured (predicted 16–30) |
| per-froxel index SET vs the flat arm | — | **exactly equal**, order included (§9) |
| `vb_mesh_froxel` / `vb_mesh_tex_froxel` pins | green | **byte-identical, no re-pin** |

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
* **No second dispatch and no second barrier.** §1.2's fixed cost is ≈ 13.9 µs *per cull
  invocation*; a separate coarse dispatch would plausibly add another one, which at low `N` is the
  entire budget. A groupshared hierarchy adds **zero** dispatch-level overhead.
* **No new GPU buffer** ⇒ no new framegraph resource, no seeding decision, no cross-frame WAR
  surface, no stale-data hole `[P0-1]`, `[P0-4]`.
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

**Trade-off.** Three-to-eight `GroupMemoryBarrierWithGroupSync()` per group and a strict
uniform-control-flow obligation (§D8) — the single most likely implementation bug in the rung.

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
ragged grid dimensions, and hardware-dependent wave sizes.

**Trade-off.** The coarse AABB is the union bound of the children's *AABBs*, which is slightly
looser than the true block hull; irrelevant, since the fine test is unchanged and exact.

### D3 — Group = 256 threads = one z-slice of the froxel grid (at the default 16×9×24)

**What.** `TPG = 256`. `gps = ceil(dim_x·dim_y / 256)` groups per z-slice; total groups
`= gps · dim_z`. For the default grid: `gps = ceil(144/256) = 1`, **24 groups**, 6144 threads
(3456 valid + 2688 idle-for-fine-work but fully used by the coarse phase). Thread→froxel map:

```
slice = gid / gps ;  s = (gid % gps)·256 + lane
valid = (s < dim_x·dim_y)
x = s % dim_x ;  y = s / dim_x ;  z = slice
fi = cluster_linear_index(x, y, z, dim_x, dim_z)        // light_table.hlsli:329 — UNCHANGED
```

**Why this shape (arithmetic).** For a block of `nz` z-slices starting at view-z `z₀`, with exp-Z
ratio `q = (far/near)^(1/dim_z) = 500^(1/24) = 1.2955`, the block's AABB volume scales as
`(64/nz)·q^(3nz)·(1 − q^(−nz))·z₀³`. Evaluating `f(nz) = (1/nz)·q^(3nz)·(1−q^(−nz))`:

| `nz` | 1 | 2 | 4 |
|---|---|---|---|
| `f(nz)` | **0.496** | 0.955 | 3.604 |

Exp-Z means depth extent grows *geometrically* with slice count while the block's screen footprint
grows with its far plane — so **single-slice blocks are decisively best**, and a 4-slice block
(`nz=4`) inflates the coarse volume 7.3× and stops rejecting anything at the far slices. Given
`nz=1`, the remaining choice is how many froxels of the slice per group. Modelled totals at
`N=512` on the measured occupancy profile (§1.3):

| `TPG` | groups | coarse pairs (`groups·N`) | fine pairs (est.) | total | vs flat 1 769 472 |
|---|---|---|---|---|---|
| 64  | 72 | 36 864 | ≈ 12 000 | ≈ 48 900 | 36× |
| 128 | 36 | 18 432 | ≈ 17 300 | ≈ 35 700 | 50× |
| **256** | **24** | **12 288** | ≈ 20 000 | **≈ 32 300** | **55×** |
| 1024 | 6 (spans 4 slices) | 3 072 | ≈ 1 700 000 (no rejection) | ≈ 1 700 000 | 1.04× |

`TPG=256` also wins under the *opposite* (uniform in-frustum) assumption — a Steiner/Minkowski
model over a frustum-filling light field gives `TPG=64: ≈ 65 900` vs `TPG=256: ≈ 52 600` pairs —
because the coarse term shrinks 3× while the fine term grows only 1.39×. And it improves occupancy:
24 groups × 256 threads = 8 warps/SM versus today's ≈ 4, i.e. 2× more latency hiding for the same
work.

**Alternatives rejected.** `TPG=64` (would keep `[numthreads(64,1,1)]` and the existing
`LIGHT_CULL_LOCAL_SIZE_X` constant untouched) — rejected: 3× the coarse cost for a modelled 1.5×
worse total. Blocks spanning ≥ 2 z-slices — rejected by the `f(nz)` table. A 3-D block (e.g. 8×4×2)
— modelled within 5 % of 16×4×1 and strictly worse than one-slice-per-group, and it needs a 3-D
delinearization for no gain.

**Trade-off.** 2688 of 6144 lanes hold no froxel at the default grid (44 %). They cost nothing in
phases 1 and 5 (guarded by `valid`), and in phase 3 they are **productive** — the coarse light scan
is striped across all 256 lanes regardless of `valid`. Second trade-off: with 24 groups on 28 SMs
the dispatch is deliberately *imbalanced* (3 hot groups, 21 near-empty at `N=512`); wall clock is
then set by the hot group, which is exactly the work that cannot be avoided.

### D4 — Output is bit-identical to the flat arm (under non-saturation), by construction

Four properties, each independently required:

1. **Same set** — the coarse level never rejects a light the fine test would accept (§5, exact).
2. **Same order** — the fine loop walks mask words ascending and, within a word, `firstbitlow`
   ascending ⇒ candidate indices ascend ⇒ **table order**, identical to today's `for i in
   [l0a_count, light_count)`. This is what preserves the shipped flat-vs-froxel equality golden.
3. **Same clamp** — `max_lights_per_cluster` truncates the same ascending prefix (`:170`).
4. **Different slice offsets only** — the global `InterlockedAdd` claim order changes, so
   `ClusterGrid[fi].offset` differs. Offsets do not affect any shaded pixel; the resolve reads
   `[offset, offset+count)`. **Except under saturation** — see §6.

### D5 — `#ifdef HIER` two-compile, base `.spv` byte-frozen

`cluster_cull.hlsl` is hand-authored (no `// === GENERATED ... ===` sentinels), so it is edited
directly. The hierarchical body is compiled in only under `-D HIER=1`, producing a **new**
`cluster_cull_hier.comp.spv`; the committed `cluster_cull.comp.spv` must remain **byte-identical**
(gated, §9). Rationale: the cull pipeline is shared by Deferred (`passes/gbuffer.rs`), ForwardPlus
(`passes/forward.rs`) and VB (`passes/vb.rs`) — the frozen base removes all risk to those paths
while the variant is proven, and it gives the equality oracle a *free A/B*: both arms are runnable
in the same process against the same inputs. Precedent: the `-D FROXEL=1` family
(`tests/vb_froxel_spv_sync.rs:98-105`, `docs/SHADER-VARIANT-MANIFEST.md:91-97`).

### D6 — Mask capacity and the defensive tail

`MAX_LIGHTS = 1024` (`light.rs:51`) bounds the point/spot block, so `HIER_MASK_WORDS = 32`
(`1024/32`) covers every representable light table, plus one `summary` word whose bit *j* marks
"mask word *j* is non-zero" (so the fine loop visits only non-empty words). A `const _: () =
assert!(MAX_LIGHTS <= 32 * 32)` host pin makes a future capacity bump a compile error rather than a
silent drop. The shader additionally carries a **provably-empty defensive tail**: any index
`i ≥ l0a_count + 1024` is tested exhaustively rather than trusted to the mask. Cost today: zero
iterations.

### D7 — Explicit range clamp on the fine arm `[P0-4b]`

The fine arm reconstructs `i = l0a_count + (w<<5) + firstbitlow(bits)` and **must** guard
`i < light_count` before `load_light` — even though the coarse phase only ever sets in-range bits.
`robustBufferAccess` is OFF in this engine; an out-of-range `StructuredBuffer<uint>` read is real UB,
and this exact class already shipped one GPU-UB bug this campaign (VB-P1b C1). The clamp makes the
impossibility **local and auditable** instead of a cross-phase argument.

### D8 — Uniform control flow around every barrier (the mandatory review checkpoint)

Today's shader early-returns on `fi >= cluster_count` (`:112-114`). The hierarchical body **must
not**: every lane, valid or not, must reach every `GroupMemoryBarrierWithGroupSync()`. The
out-of-range condition becomes a `bool valid` that guards work, never control flow across a barrier.
An early `return` here is undefined behaviour and typically a device hang. This is called out as an
explicit code-review gate item, not a comment.

Corollary that makes it safe: an invalid lane's AABB is still `(+1e30, −1e30)` — the exact identity
element of `min`/`max` — so it may participate in the reduction unconditionally with no special case.

### D9 — Reduction by groupshared tree, not by float-as-int atomics

`InterlockedMin/Max` are integer-only; the order-preserving float↔uint key trick would work but adds
a lemma to review, and 256 lanes contending on 6 addresses serialize anyway. Instead: each lane
writes its 6 floats to `groupshared float3 gs_min[256], gs_max[256]` (6 KB, irrelevant against
Ampere's 100 KB/SM when only 24 groups exist), then an 8-step halving tree with a barrier per step.
`min`/`max` are **exact** in IEEE-754 and associative/commutative, so the tree order is irrelevant
and the result is exactly the componentwise extremum — which is what §5's proof requires.

---

## 4. Shader structure (both arms, one file)

```hlsl
// cluster_cull.hlsl — base arm unchanged; HIER arm compiled in only under -D HIER=1
#ifdef HIER
static const uint HIER_TPG        = 256u;   // host mirror: ClusterConfig::hier_group_threads()
static const uint HIER_MASK_WORDS = 32u;    // MAX_LIGHTS / 32  (D6)
groupshared float3 gs_min[HIER_TPG];
groupshared float3 gs_max[HIER_TPG];
groupshared uint   gs_mask[HIER_MASK_WORDS];
groupshared uint   gs_summary;              // bit j <=> gs_mask[j] != 0
[numthreads(256, 1, 1)]
#else
[numthreads(64, 1, 1)]
#endif
void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex)
```

Phases of the `HIER` arm (each lane; `valid` per D8):

| # | Work | Barrier after | Cost |
|---|---|---|---|
| 0 | init `gs_mask[0..32)` + `gs_summary` (lanes 0..32), write `gs_min/gs_max[lane]` = own froxel AABB (identity when `!valid`) | yes | 33 stores |
| 1 | **unchanged** froxel AABB build — 8 `generate_ray` + `view_z_to_t` + `expand_aabb` (`:126-153`), only when `valid` | (in 0) | 8 unprojections |
| 2 | halving tree over `gs_min`/`gs_max` (8 steps) | 8 | 8 × (read+min/max+write) |
| 3 | coarse scan: `for (i = l0a + lane; i < light_count; i += 256)` → `light_kind` filter + `sq_dist_point_aabb(L.pos, coarse_min, coarse_max) <= r*r` → `InterlockedOr(gs_mask[i>>5], 1<<(i&31))`. **All 256 lanes, `valid` or not.** | yes | `ceil(N/256)` per lane |
| 4 | lanes 0..31 fold `gs_summary` from `gs_mask[lane] != 0` | yes | 32 atomics |
| 5 | fine walk (`valid` only): for each set bit of `gs_summary`, for each set bit of `gs_mask[w]` ascending: clamp `i < light_count` (D7), then the **token-identical** fine test + `local[]` append (`:161-175`) | — | `E_coarse` tests |
| 6 | **unchanged** `InterlockedAdd` claim + scatter + `ClusterGrid[fi] = uint2(offset, write_count)` (`:180-194`), `valid` only | — | 1 atomic |

Everything in phases 1, 5-tail and 6 is *character-identical* to the base arm — that is what makes
D4's byte-identity a construction rather than a hope.

**Host side.** `ClusterConfig` gains `hier_group_threads() -> u32` (256) and
`hier_group_count() -> u32 = ceil(dim_x·dim_y / 256) · dim_z` (`crates/boyko_render/src/light.rs`,
next to `cluster_count()`:728 — Principle 0: a derived accessor on the existing Resource, no side
store). The three record sites dispatch `hier_group_count()` instead of
`cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X)` **only when the hier pipeline is selected**
(`present/passes/vb.rs:184` and its gbuffer/forward siblings). A host↔shader pin test asserts the two
derivations agree for a matrix of grid dims.

---

## 5. The exactness proof (re-derived against the real code; `[P1]` discharged)

**Claim.** If the coarse test rejects light `L` for a group, then the fine test rejects `L` for every
froxel in that group — in IEEE-754 arithmetic, with no epsilon.

**Setup.** Lane `i` computes `(min_i, max_i)` by expanding from `(+1e30, −1e30)` over 8 world points
`ro + rd·t`, where `(ro, rd) = generate_ray(...)` (`ray_gen.hlsli:44`) and
`t = view_z_to_t(slice_view_z(z|z+1), rd)` (`cluster_cull.hlsl:77-91`). The group values are
`MIN = min_i min_i`, `MAX = max_i max_i`, componentwise (D2/D9).

**Step 1 — the reduction is exact.** `min`/`max` on floats introduce no rounding: the result is one
of the inputs. Hence, componentwise and exactly, `MIN_j ≤ min_{i,j}` and `MAX_j ≥ max_{i,j}` for
every lane `i` and axis `j`. (Tree order is irrelevant: `min`/`max` are exactly associative and
commutative.)

**Step 2 — `sq_dist_point_aabb` is monotone in the box.** With `d_j = max(lo_j − c_j, c_j − hi_j, 0)`
and result `Σ d_j²` (`:102-105`):

* `lo_j ≤ lo'_j ⇒ fl(lo_j − c_j) ≤ fl(lo'_j − c_j)` — IEEE-754 round-to-nearest is **monotone**
  (a ≤ b ⇒ `fl(a) ≤ fl(b)` for the same operation and rounding mode).
* `hi_j ≥ hi'_j ⇒ fl(c_j − hi_j) ≤ fl(c_j − hi'_j)`, same reason.
* `max(·, ·, 0)` is monotone; the result is non-negative.
* `d_j ≥ 0`, and `fl(d_j · d_j)` is monotone on non-negatives; `fl(a + b)` is monotone in each
  argument.

Therefore `sqdist(c, MIN, MAX) ≤ sqdist(c, min_i, max_i)` **as computed**, not merely in exact
arithmetic.

**Step 3 — conclusion.** Fine accepts ⇔ `sqdist(c, min_i, max_i) ≤ r·r`. By Step 2 that implies
`sqdist(c, MIN, MAX) ≤ r·r`, i.e. coarse accepts. Contrapositive: coarse rejects ⇒ fine rejects. ∎

**What the proof does *not* need**, and why the Rev 1 weakness is gone: no dilation constant, no
epsilon, no claim about FMA contraction, no assumption that two translation units or two `.spv`
compute the same expression bit-identically. The coarse and fine tests consume *the same values*
produced by *the same instructions in the same shader invocation family*; the only relation used is
monotonicity, which IEEE-754 guarantees unconditionally.

**Residual hypotheses (each gets a test, §9):**
* `min_i`/`max_i` are finite. `generate_ray` on a degenerate camera could emit NaN; a NaN in the
  reduction would poison `MIN/MAX` and could drop lights. Today's flat arm has the identical
  exposure, so this is not a new hazard — but H1's CPU oracle asserts finiteness over the whole grid
  for every camera in its matrix, and a `debug_assert!`-class host check accompanies the camera
  upload.
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
   `alloc_total ≤ index_list_cap` ⇔ **no index was dropped anywhere**. One `u32` readback settles it
   exactly, per run, with no modelling.
2. **The equality oracle asserts it as a precondition** (§9, H3): if `alloc_total ≥ index_list_cap`
   the test **fails loudly** rather than silently comparing two differently-clamped results.
3. **The honest caveat, stated in the shader header and the test:** under saturation the arms may
   legitimately differ in *which* froxel loses its tail; byte-identity is claimed only for
   non-saturating configurations, and the saturating case is pinned only against itself.

The per-froxel cap (`max_lights_per_cluster`, `:170`) is *not* order-sensitive — it truncates the
ascending prefix identically in both arms (D4.3) — so only the global cap needs this treatment.

---

## 7. Predicted win (falsifiable at H1 before any shader work)

Using §1.2's model and §1.3's occupancy profile, `TPG=256` ⇒ 24 groups:

```
pairs_hier(N) = 24·N                       (coarse, phase 3)
              + Σ_froxels E_coarse(parent) (fine,  phase 5)
```

| `N` | flat pairs | coarse | fine (est.) | hier pairs | ratio | model cull hier | measured cull flat |
|---|---|---|---|---|---|---|---|
| 8   | 27 648    | 192    | ≈ 6 900  | ≈ 7 100  | 3.9× | ≈ 15.9 µs | 19.7 µs |
| 64  | 221 184   | 1 536  | ≈ 5 000  | ≈ 6 500  | 34×  | ≈ 15.7 µs | 72.7 µs |
| 128 | 442 368   | 3 072  | ≈ 5 200  | ≈ 8 300  | 53×  | ≈ 16.2 µs | 134.9 µs |
| 512 | 1 769 472 | 12 288 | ≈ 20 000 | ≈ 32 300 | 55×  | ≈ 22.7 µs | 498.1 µs |

`froxel_total_hier(N) ≈ 26 500 + 13 939 + 0.2736·pairs_hier(N)` ⇒ break-even against
`flat_shade(N) = 23 922 + 1 109.6·N` at **`N ≈ 16`**; at a 2×-pessimistic marginal rate,
**`N ≈ 25–30`**. The conclusion is robust to a 2× model error: at `0.547 ns/pair` the `N=512` cull is
still ≈ 51 µs, a 10× win.

**Fine-column derivation (the number H1 replaces with a measurement).** From §1.4's collinearity
result, the in-frustum lights at `N=512` are the ≈ 14 % of the rig lying at view-depth 8.7–14.4, i.e.
z-slices 17–19 of 24. Those three groups therefore carry ≈ 40 candidates each over their 144
froxels (3 × 144 × 40 ≈ 17 300); the remaining 21 groups carry ≈ 0–2 (≈ 3 000). H1 computes this
exactly, per config, on the CPU.

### 7.1 The negative result — single-digit break-even is impossible here, and why

`froxel_shade` alone is 25–30 µs and the cull's fixed cost is ≈ 13.9 µs, so **the froxel arm's floor
is ≈ 40 µs**, while `flat_shade`'s intercept is **23.9 µs**. Break-even requires
`flat_shade(N) > floor`, i.e. `1 109.6·N > 16 000` ⇒ `N > 14.4` — **and that is with a cull of cost
zero.** No amount of cull optimisation can push the break-even below ≈ 15 on this hardware and this
grid. Reaching single digits would require attacking the *fixed* costs (merge the cull into the shade
dispatch to delete a barrier; eliminate the `cmd_fill_buffer` reset via a per-FIF alloc ring; or
shrink the froxel grid at low `N`). Those are named as VB-P1g in §11 and are explicitly **out of
scope** here. The stated goal "break-even collapses toward single digits" is therefore **partially
unreachable**, and the plan does not pretend otherwise: it targets **≈ 16–30**, a 3–6× improvement
on the measured ≈ 103.

---

## 8. Rungs

Each rung is independently committable, has one gate, and states what turns that gate RED.

### H0 — Instrument the fixed cost (no behaviour change)
* **Files:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (+2 `VbTimedPass` slots),
  `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-215` (split the `LightCull` bracket into
  `CullReset` = fill+barrier and `CullDispatch`), `crates/boyko_app/src/runner.rs` (print both, plus
  `alloc_total` read back from `LightIndexAlloc[0]`).
* **Why first:** §1.2's "13.9 µs is fill+barrier" is a *hypothesis*. §1.1 is this campaign's standing
  reminder that unmeasured hypotheses about this shader have already cost one 2× regression. If the
  fixed cost turns out to be dispatch-intrinsic rather than barrier-intrinsic, §7.1's follow-up list
  changes and the low-`N` predictions move.
* **Gate:** the bench prints `cull_reset_ns + cull_dispatch_ns`, and their sum reproduces the existing
  `froxel_cull_ns` within 5 % at `N ∈ {8, 512}`; every golden pin byte-identical.
* **RED if:** any pin moves (timestamp writes must not perturb rendering results); the sub-brackets do
  not sum; `alloc_total` disagrees with §1.3's CPU probe by > 1 %.

### H1 — CPU oracle: the host hierarchical mirror + the permanent set/occupancy/selectivity gate
* **Files:** `crates/boyko_rhi_vulkan/src/goldens.rs` (+ `golden_cluster_cull_hier`, a
  block-decomposed mirror of `golden_cluster_cull`:3510 using D2's min/max merge),
  `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (+ the matrix test), hardened from the
  probe preserved at `scratchpad/cap_probe.rs.txt`.
* **What it asserts, per config in the matrix:**
  1. `golden_cluster_cull_hier == golden_cluster_cull` **exactly**, per froxel, **including order**.
  2. Coverage totality: every froxel index is produced by exactly one (group, lane).
  3. `total_indices < INDEX_LIST_CAP` and `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (§6, and it pins
     §1.3's table as a regression).
  4. All AABB bounds finite (§5's residual hypothesis).
  5. **Selectivity (the perf premise):** `pairs_hier / pairs_flat ≤ 1/8` on the bench rig at
     `N ≥ 128`. This is a *performance gate that runs on the CPU in 0.45 s with no GPU*.
* **Matrix:** {ORTHO 64×64 (the `l1_cluster_config` fixture, `sdf_gbuffer_hybrid.rs:5215`),
  PERSPECTIVE 512×512 (VB-P1d camera)} × {bench Kronecker rig, corrected R3 rig, dense in-frustum
  rig, adversarial boundary rig} × `N ∈ {0, 1, 8, 64, 128, 512, 1024}`.
  The **adversarial** rig places lights so that `sq_dist_point_aabb == r*r` exactly for a chosen
  froxel, and at `r ± 1 ulp`, on faces, edges and corners of the AABB — the boundary of the `<=`
  test, which is where a non-conservative coarse level would first fail.
* **Gate:** all five assertions green over the whole matrix.
* **RED if:** any froxel's index vector differs in content or order; a light is dropped at the range
  boundary; selectivity misses 8×. **Concrete mutation that must turn it red:** scale the coarse
  extents by `0.999` (`MIN *= 1.001; MAX *= 0.999`) — a non-conservative coarse box — and the
  adversarial rig must fail.
* **Abort point:** if selectivity on *both* the bench rig and the in-frustum rig is < 4×, the rung
  stops here at zero GPU cost and the plan is rewritten (§10).

### H2 — The `-D HIER=1` shader variant (dark infra)
* **Files:** `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` (the `#ifdef HIER` arm, §4),
  `crates/boyko_rhi_vulkan/shaders/cluster_cull_hier.comp.spv` (new, offline dxc 1.4.350.0),
  `crates/boyko_rhi_vulkan/src/compute.rs` (+ `cluster_cull_hier_spirv()` beside `:1609`),
  `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs` (adopt the multi-variant idiom of
  `vb_froxel_spv_sync.rs:88-130`), `docs/SHADER-VARIANT-MANIFEST.md:91-97` (+ one row).
  Pipeline **built but never selected**.
* **Gate:** (a) `cluster_cull_hier.comp.spv` byte-equals its re-DXC under the frozen recipe;
  (b) `cluster_cull.comp.spv` byte-equals its re-DXC **with no `-D`** — i.e. the base arm is
  physically unperturbed by the seam; (c) every golden pin unchanged (nothing selects the variant);
  (d) `cargo clippy --workspace --all-targets -- -D warnings`.
* **RED if:** the base `.spv` moves by one byte (the seam leaked into the `#else` arm); the manifest
  row is missing (the `-D` matrix must stay enumerable by one grep).

### H3 — The GPU set-level equality oracle `[P0-2]`, `[P0-3]`
* **Files:** `crates/boyko_rhi_vulkan/tests/cluster_cull_hier_equiv.rs` (new) + a **cull-only**
  driver: camera UBO + light table + the three buffers + one dispatch + three readbacks. It does
  **not** go through `run_gbuffer_hybrid_lit_clustered` (`sdf_gbuffer_hybrid.rs:5276`) — no SDF, no
  resolve, ~10× faster, and it can drive a PERSPECTIVE camera trivially.
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
  4. Both arms equal to the host `golden_cluster_cull` set (per-froxel, as a set — the host/GPU
     ULP caveat applies to the host comparison only; the arm-vs-arm comparison is exact because both
     run the same fine test on the same device).
  5. **Totality of `ClusterGrid`:** pre-fill the grid with `0xFFFFFFFF`; after the cull no cell
     retains the sentinel ⇒ every froxel was written exactly once by the block decomposition
     `[P0-4a]`.
  6. Non-vacuity: at least one froxel non-empty, and the hier pipeline handle is asserted distinct
     from the base one.
* **RED if:** any of 1–6 fails. **Concrete mutations that must turn it red, each to be executed once
  during review:** (i) drop the `valid` guard on phase 6 → totality/duplicate writes; (ii) replace
  the tree reduction's `min` with the lane-0 value → non-conservative coarse box → set mismatch on
  the adversarial rig; (iii) walk mask words descending → order mismatch on a multi-light froxel;
  (iv) delete the D7 range clamp and inject a synthetic out-of-range bit → out-of-range index in the
  readback.

### H4 — Arm the variant for VB + the two-rig bench
* **Files:** `crates/boyko_app/src/gpu_scene/mod.rs` (pipeline choice at the cull build site
  ~`:4300`), `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:184` (group count from
  `hier_group_count()` when the hier pipeline is bound), `crates/boyko_render/src/light.rs`
  (`hier_group_threads`/`hier_group_count` + the host↔shader pin),
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
  are NOT changed** (§1.4.3).
* **RED if:** a froxel pin moves (⇒ the set or the order differs, or a cap saturated — H3's
  precondition should have caught it, so a moved pin here means the pin's scene saturates and must be
  measured with `alloc_total`); any §2 threshold missed.

### H5 — (conditional on H4) migrate Deferred + ForwardPlus to the hierarchical arm
* **Files:** `present/passes/gbuffer.rs`, `present/passes/forward.rs` (group count + pipeline),
  `gpu_scene/mod.rs`. Retire the flat arm to test-only status (it remains the equality oracle's
  reference forever).
* **Gate:** every Deferred/ForwardPlus golden byte-identical; `l1_known_light_lands...` and
  `l1_clustered_resolve_matches_the_brute_force_image` green; `forwardplus_mesh` green.
* **RED if:** any of the above moves. **Precondition:** H4 shows a win on *both* the `kronecker` and
  `infrustum` rigs; a win only on the out-of-frustum rig does not justify migrating shipped paths.

---

## 9. Validation plan (consolidated)

| Requirement | Mechanism | Where | Can it actually fail? |
|---|---|---|---|
| `[P0-1]` framegraph seeding | **No new buffer is introduced** — the coarse mask is groupshared. The existing trio keeps the `add_buffer_seeded` seeds landed at `5e07936` (`graph_bridge.rs:3179-3190`). If any future rung promotes the mask to a global buffer it **must** use `add_buffer_seeded(ResSync::seeded_readers(COMPUTE, SHADER_READ))` — single-instance, cross-frame reused, read by a later COMPUTE pass — matching `declare_deferred_graph`/`declare_forward_graph`. | design | n/a (nothing new declared); `framegraph_gbuffer_equiv` still covers the trio |
| `[P0-2]` tests that can fail | H3's oracle drives the **hier** pipeline explicitly on a **PERSPECTIVE** camera and asserts the hier handle ≠ base handle; four named mutations are executed during review | `tests/cluster_cull_hier_equiv.rs` | yes — mutations (i)–(iv) in H3 |
| `[P0-3]` set-level oracle | Per-froxel index **sequence** equality between arms + against the host oracle, not an image hash. Image pins are a secondary no-regression gate only. | H1 (CPU, exhaustive) + H3 (GPU) | yes — a single dropped marginal light fails, where an 8-bit image hash would not |
| `[P0-4a]` totality | Groupshared mask is re-initialised **every dispatch** by lanes 0..32 unconditionally — there is no cross-frame state to go stale. `ClusterGrid` totality proven by the `0xFFFFFFFF` pre-fill probe. | D1, H3.5 | yes — drop the init or the `valid` guard |
| `[P0-4b]` range clamp | Explicit `i < light_count` guard before `load_light` in the fine arm (D7) | shader | yes — H3 mutation (iv) |
| `[P1]` FP margin | **Deleted, not bounded**: D2 makes enclosure a monotonicity theorem (§5) with no epsilon. Residual finiteness hypothesis asserted by H1.4. | §5 + H1 | yes — H1's finiteness assert, H3 mutation (ii) |
| `[P1-3]` cap saturation | §1.3's measured table + the exact `alloc_total ≤ cap` detector asserted as a precondition of every equality run | §6, H0, H1.3, H3.1 | yes — the equality test aborts loudly instead of comparing clamped results |
| wave/subgroup coherence | No wave intrinsics used (D1 alternatives). Phase 5's trip count is **group-uniform** (all lanes walk the same mask) ⇒ zero loop divergence; only the append predicate diverges, exactly as today. | design | n/a |
| dispatch shape | `hier_group_count()` host↔shader pin over a matrix of grid dims | H4 | yes — mismatched derivations leave froxels unwritten ⇒ H3.5 sentinel |
| occupancy / registers | Groupshared 6.3 KB/group with 24 groups total (Ampere 100 KB/SM) — not a limiter. Fine loop adds ≈ 6 VGPRs to a shader already at ≈ 8 % occupancy and latency-bound. `local[256]` is **unchanged** (§1.1). | design | measured at H4 |
| `unsafe` discipline | The rung adds no new Rust `unsafe`; the record-site changes are inside existing `unsafe` blocks whose `// SAFETY:` comments gain the new group-count invariant. | H4 | clippy `-D warnings` |

---

## 10. Risks and the ABORT criterion

| Risk | Mitigation |
|---|---|
| The `0.2736 ns/pair` rate does not transfer to the new dispatch shape | H1 gates the *pair-count* premise on the CPU for free; H4 measures the real thing; the abort threshold is in measured ns |
| The 13.9 µs fixed cost is dispatch-intrinsic, not barrier-intrinsic ⇒ low-`N` gains vanish | H0 measures it *first*; it changes only the low-`N` prediction, not the `N ≥ 128` win |
| Load imbalance (3 hot groups of 24) leaves the GPU idle | The hot group's work is irreducible; imbalance is a *symptom of having removed the other 97.5 %*. If H4 shows the hot group dominating, the follow-up is a second in-group level (§11), not a redesign |
| A barrier reached under non-uniform control flow ⇒ device hang | D8 is a named code-review gate item; H3 runs on-device and a hang is unmissable |
| The equality oracle is run in a saturating configuration and silently compares clamped results | H3.1's `alloc_total` precondition fails the test loudly (§6) |
| Byte-identity claim over-reached | Explicitly scoped to non-saturating configurations (§6); the saturating case is pinned only against itself |

**ABORT (revert exactly as the two-pass attempt was reverted) if any of:**

1. **H1**: pair-count selectivity < 4× on both the `kronecker` and `infrustum` rigs at `N ≥ 128`.
   *(Costs zero GPU time and zero shader code — this is the cheap kill switch.)*
2. **H3**: any per-froxel index sequence differs between arms in a non-saturating configuration.
3. **H4**: `froxel_cull_ns` at `N_ps=512` > 250 000 (< 2× win), **or** any of `N_ps ∈ {8, 32, 64}`
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
* **VB-P1h — a second in-group level** (per-64-lane sub-block masks), only if H4 shows the fine phase
  dominating. Output-neutral by D2's corollary, so it is a pure perf experiment.
* **VB-P1i — wave-intrinsic reduction** (`WaveActiveMin`/`WaveActiveBallot`) once the device-feature
  query for subgroup ballot/arithmetic exists in the RHI. Output-neutral by D2's corollary.
* **A generalized `alloc_total` HUD counter** so saturation is visible in ordinary runs rather than
  only in tests.

---

## Appendix — source anchors

| What | Where |
|---|---|
| The cull shader (hand-authored, no eDSL sentinels) | `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` — dispatch shape `:107`, AABB build `:126-153`, flat light loop `:161-175`, `local[256]` `:159`, claim+write `:180-194`, `sq_dist_point_aabb` `:102-105`, `view_z_to_t` `:85-91`, `slice_view_z` `:77-79` |
| Shared ray-gen | `crates/boyko_rhi_vulkan/shaders/ray_gen.hlsli:44-75` |
| Cluster linearization / params (one source of truth) | `crates/boyko_rhi_vulkan/shaders/light_table.hlsli:313-323, 329-331`; `light_kind` `:271-273`; `load_light` `:255-265` |
| Host constants + config | `crates/boyko_render/src/light.rs:43-61` (`CLUSTER_DIM_*`, `MAX_LIGHTS`, `MAX_LIGHTS_PER_CLUSTER`, `INDEX_LIST_CAP`), `:691-770` (`ClusterConfig`) |
| Measured band + provenance table | `crates/boyko_render/src/light_policy.rs:40-77` |
| Host cull oracle | `crates/boyko_rhi_vulkan/src/goldens.rs:3510` (+`:3437`, `:3472-3498`) |
| VB framegraph declaration (seeded trio, `5e07936`) | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:3167-3190`; the `light_cull` pass `:3202-3242` |
| VB record site (fill → barrier → dispatch, timestamps) | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-215` (group count `:184`) |
| Existing ORTHO-only cull oracle | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:6432` (fixture `:5215`, cap `:5230`, driver `:5276`) |
| `.spv` byte gates (idioms to clone) | `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:63-88`; multi-variant: `crates/boyko_rhi_vulkan/tests/vb_froxel_spv_sync.rs:88-130` |
| Variant manifest | `docs/SHADER-VARIANT-MANIFEST.md:91-97` |
| The bench | `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs` (rig `:124-144`, camera `:235-254`) |
| Occupancy probe source (to be hardened into H1) | `scratchpad/cap_probe.rs.txt` |
