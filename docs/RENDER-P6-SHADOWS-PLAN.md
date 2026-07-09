# Architecture: P6 — Hybrid Shadows (multi-light SDF + rasterized mesh shadow map, MIN-combined)

> `docs/RENDER-P6-SHADOWS-PLAN.md` material. Branch `ecs`, crate `boyko_rhi_vulkan`. Builds on P5 (94a89a6, mesh = first-class MRT G-buffer producer). Closes the two hard commitments named in `ARCHITECTURE-HYBRID-PERF.md` §3.2: (1) exactly ONE shadow caster, (2) SDF occluders only.

## Critique resolutions

Both critics returned **REVISE**. Every BLOCKER is folded below; resolution summary:

- **B1 (0%-gate is NOT byte-identical at the 1→2-light transition).** ACCEPTED. The 0%-gate is now scoped *strictly* to the single-directional `shadow_mode==0` scene and proven byte-identical there only. The multi-light path is declared a **NEW numerical result with NO byte-identity claim** (it didn't exist before). The design fork is resolved: in multi-light mode the **primary directional KEEPS reading `gMaterial.r`** (the marcher still marches it, brick-accelerated, into the channel); only *extra* casters get the resolve-march. This preserves the 1st light's exact path across the 1→N transition and avoids re-marching the one light that already works. See Decision 1 (rewritten) + Decision 6 (NEW).
- **B2 / cheap-half perf (unoccluded ray marches to T_MAX; brute-force analytic vs brick).** ACCEPTED — this was the design's worst hole. Two structural fixes: (a) the resolve shadow march **binds the brick atlas** (PointerGrid/BrickAtlas) and uses the SAME empty-space-skip the marcher's primary visibility uses — it is NOT brute-force `field_distance`; (b) a per-light **`casts_sdf_shadow` flag** + a **`MAX_SDF_SHADOW_CASTERS_PER_PIXEL` dominant-N cap** bound the march count below the cluster slice. The perf model is rederived from the field Lipschitz step for the **unoccluded (common) case** marching to `T_MAX`, with a concrete worst-case eval budget and a **rung-level perf-gate test**. See Decision 2 (rewritten) + the perf model.
- **B3 (eDSL `t_max` parameterization may perturb the frozen marcher `.spv`).** ACCEPTED. Resolved by **option (a)**: the marcher's `sdf_soft_shadow` body is left **literally untouched** (T_MAX-hardcoded, frozen). The eDSL emits a **separately-named `sdf_soft_shadow_ranged(p,n,L,t_max)`** consumed ONLY by the resolve. The frozen `.comp.spv` cannot move because its spliced span is unchanged. See Decision 3 (rewritten).
- **B4 (SDF-visibility cache space ambiguity + stale invalidation).** ACCEPTED. The cache (R2b) is declared **light-space** (marched from the LIGHT frustum, camera-independent), NOT screen-space-from-P. The full invalidation set is enumerated (light transform, SDF-edit dirty, field-param; NOT camera). It is reconciled: a cached SDF visibility **becomes a shadow-map depth-compare test** (same shape as the mesh map), so the resolve "samples" via a light-space projection + compare, not a re-march. Directional/large-spot frusta only; punctual needs cube/DPSM (deferred). See Decision 4 (rewritten).
- **B5 (R2 point/spot mesh shadows are a hidden correctness gap).** ACCEPTED. R2a scope is **restated as directional mesh shadows ONLY** in the Goal + metrics. A host-side reject + `debug_assert` forbids flagging a punctual light as a mesh-caster (so the host cannot silently believe a point light mesh-shadows). Punctual lights get SDF shadows (R1) but `mesh_vis=1` until the deferred cube/DPSM rung — surfaced, not buried. See Decision 5 (R2) + Goal.

CONCERNS folded: directional cost is **unbounded by clusters** (M-directional term separated from K-punctual term in the perf model); resolve binds **one** new `Buf` descriptor (camera already bound — wording corrected); `gMaterial.r` is 8-bit (multi-light resolve-march is *higher* fidelity, stated); NoL≤0 front-of-loop skip elides the march; resolve binding-cap raise to 16 is the **plan**, not an afterthought; R2 light-space fit = single tight ortho over mesh AABB, cascades deferred; R2b dirty-channel readability under the INVIOLABLE boundary is a **gating feasibility check** (R2a is clean without it); GPU machine-model framing corrected (occupancy/VGPR/divergence, not x86 L1d); the auto-select policy keys on **"the single caster IS the directional the marcher shadowed into `gMaterial.r`"**, not `count==1`.

---

## Goal

Make shadows **hybrid-correct on both axes** the directive contradicts today, without regressing the shipped single-directional SDF scene by a single byte:

1. **N casters.** Every directional/point/spot light reaching a pixel casts its own shadow, range/cluster-gated — not one hardcoded `light_dir`.
2. **Both occluder representations.** SDF geometry shadows via the brick-accelerated field march (today's mechanism, generalized to per-light); **mesh geometry shadows via a rasterized light-space depth map**; combined per light by `min(sdf_vis, mesh_vis)`.

**Scope honesty (folded from B5):**
- **R1** (ships first) = multi-light **SDF** shadows for all light kinds (directional/point/spot), brick-accelerated, dominant-N-bounded.
- **R2a** (later) = **directional-only** mesh shadow maps MIN'd with the SDF march. **Point/spot lights get NO mesh shadow in R2a** (`mesh_vis=1`); a mesh wall does NOT darken an SDF floor under a *point* light until the deferred cube/DPSM rung. This is an explicit scope cut, surfaced here.
- **R2b** (later) = light-space SDF-visibility cache for the static-directional case.

**Target:** the directive's biggest shipped-vs-stated gap closed; the strictly-single-directional scene is a literal `shadow_mode==0`-class 0%-gate (byte-identical `.spv` on that arm); per-light cost bounded by `min(cluster_slice, MAX_SDF_SHADOW_CASTERS_PER_PIXEL)` AND brick-skipped per step.

## Context and constraints

- **Subsystems touched (INVIOLABLE boundary):** only `boyko_rhi_vulkan` (`sdf_gbuffer_composite.hlsl`, `deferred_pbr.hlsl`, a new `shadow_map.{vs,fs}.hlsl`, host `swapchain.rs`/`compute.rs` recorder + goldens) and `boyko_shaderdsl` (the new `sdf_soft_shadow_ranged` body). **Do NOT touch** `boyko_render`/`boyko_ecs`. Light table, cluster grid/list, mesh raster, D32 depth, **and the brick atlas (PointerGrid/BrickAtlas)** are consumed as already-shipped.
- **Frozen contracts:** `sdf_field.hlsli` + the marcher field math (`field_distance`/`sdf`/gradient) AND the marcher's existing `sdf_soft_shadow` (T_MAX-hardcoded) stay BYTE-FROZEN. The resolve shadow march is a strict **field-consumer** (it CALLS `field_distance` + reads the brick atlas read-only, never edits the field).
- **0%-gate (scoped per B1):** a **strictly-single-directional, SDF-only** scene must render BYTE-IDENTICAL to today, via `shadow_mode==0` (resolve reads `gMaterial.r`, no resolve-march, no new `.spv` arm executed). The multi-light path is additive and carries **no byte-identity claim against today** — it is new functionality with a fp32 (higher-precision) shadow term.
- **Reuse L1:** range/cluster-gate the per-light marches to the froxel's `LightIndexList` slice (`ps_offset`/`ps_count`), already in the resolve. No light culled by the cluster is shadow-marched for that pixel.
- **GPU verification:** `BOYKO_DISABLE_VALIDATION=1` for all GPU goldens (validation broken on windows-gnu, d99dfda). Pixel goldens are the hard gate; screenshots dumped + visual gate RELAXED.

### Invariants preserved
- The marcher's three terminal `gViewT` writes (real `t` r32f on SDF-lit, `1.0e30` on mesh/bg/empty) and the resolve's read-under-`mask==1` gate stay intact; P6 reconstructs `P = ro + rd*view_t` identically. **Note (B1):** `view_t` is r32f (exact fp32), so the resolve's `P` is exact; this is the origin of the resolve-march and need NOT bit-match the marcher's last sphere-trace position because the directional's shadow on the gate scene comes from `gMaterial.r`, not a resolve re-march.
- The frozen `dist_to_brick_exit`/`brick_cell_class`/`m2_*` eDSL splice sentinels and the `sdf_field_edsl_sync` text gate are untouched.
- `safe_normalize` host-parity (the L1 black-pixel fix) reused on every new half-vector.
- The marcher's 16-binding cap (9..=14 brick + 0..8) is unchanged. **The resolve's binding cap is raised from 12 to 16** (the planned escape for R1's `Buf` + R2's maps + light-VP UBO; see Data structures — this is the plan, not an afterthought).

## Owner-call #2 RESOLUTION — rung order + scope

**Decision: SPLIT into rungs; ship R1 (multi-light SDF) FIRST and standalone; R2 (mesh map) is a SEPARATE later rung. Do NOT bundle them.** Justification unchanged from the draft and reinforced by the critiques: R1 lives entirely inside the resolve's existing per-light loop (one new descriptor, no new pass/barrier), is the unconditional always-correct half, and establishes the `vis_l = (NoL * vis_l)` seam that R2 augments with a single `min`. R2 carries every high-risk item (new pass, new descriptor, new barrier, light-space matrix, bias tuning, cache invalidation) and its win is conditional + cache-gated.

**Critique-driven refinement to the rung boundary:** R1 is NOT shippable as "march every cluster-slice caster brute-force." Per B2 it ships with (a) brick-accelerated marches and (b) a dominant-N caster cap + per-light `casts_sdf_shadow` flag. The perf-gate test (below) is R1's exit criterion — R1 does not ship if the K-caster full-screen frame exceeds the stated multiple of the 1-caster frame.

## Key decisions

### Decision 1: Per-light shadow term in the RESOLVE; primary directional KEEPS `gMaterial.r` (B1 fork resolved)

**What:** Multi-light shadowing is a per-light scalar `vis_l` in the resolve's per-light loop, replacing the single `shadow` factor. **The primary directional light** (the one the marcher already marches into `gMaterial.r`, brick-accelerated) **continues to read `gMaterial.r` in all modes** — it is never re-marched in the resolve. **Extra casters** (a 2nd+ directional, every point/spot) get an on-demand `sdf_soft_shadow_ranged` march in the loop. The `gMaterial.r` channel is thus NOT retired; it remains the exact, byte-stable term for the one light it always encoded.

**Why (B1):** This makes the 1→N transition byte-preserving for the 1st light (its shadow path never changes), eliminates the "directional flips from 8-bit gMaterial.r to fp32 resolve-march" discontinuity the critic flagged, AND avoids paying a resolve-march for the one light that already has a brick-accelerated answer. The resolve already has `P`, `n`, `v`, `L`, the cluster slice — extra casters march there, naturally cluster-gated and shadowed-pixel-only.

**Alternatives rejected:** multi-channel/array shadow G-buffer — unbounded marcher cost (march every light per pixel), can't cluster-gate (marcher has no per-pixel cluster slice), burns G-buffer channels against the cap, bakes a max-light-count into a frozen layout. A separate full-screen shadow-resolve compute pass — an extra pass + a G-buffer-sized per-light buffer, no win over folding into the loop that already reconstructs `P`.

**Trade-off:** The resolve gains the field march (must `#include "sdf_field.hlsli"`, bind the edit-list `Buf` + the brick atlas read-only). Grows resolve VGPR pressure + I-cache; gated by `shadow_mode` (OFF arm DCE-strips the march to the frozen-today `.spv`); bounded by `MAX_SDF_EDITS=16` per step AND brick empty-space skip AND the dominant-N cap.

### Decision 2: The resolve shadow march is BRICK-ACCELERATED + dominant-N-bounded (B2)

**What:** The resolve's `sdf_soft_shadow_ranged` reads the SAME `PointerGrid`/`BrickAtlas` the marcher's primary visibility uses (bound read-only to the resolve set), so each march step empty-space-skips through bricks instead of re-folding all 16 edits every step. Additionally:
- A per-light **`casts_sdf_shadow` host flag** (packed into the light table's existing flag word — see Data structures): lights not flagged pay ZERO march (the resolve falls back to `vis_l=1`).
- A **`MAX_SDF_SHADOW_CASTERS_PER_PIXEL`** cap (const, default 4): the resolve marches at most the dominant-N nearest/brightest flagged casters in the cluster slice; beyond N, lights contribute NoL-only (`vis_l=1`). This bounds the per-pixel march count *below* the cluster slice size.
- A front-of-loop `NoL <= SHADOW_NDOTL_EPS → continue` (skips both the lighting AND the march for back-faced lights — matches the marcher's `sdf_soft_shadow` early-out semantics, saves the cost).

**Why (B2):** The unoccluded ray (the COMMON case) marches to `T_MAX` because soft-shadow only early-outs on an occluder HIT — so the worst case (full march) dominates, and brute-force analytic at K casters/pixel is an order-of-magnitude blowup. Brick skipping is exactly how the marcher's own visibility avoids this; the resolve march MUST share it. The dominant-N cap + per-light flag bound the multiplier the critic exposed.

**Alternatives rejected:** brute-force analytic `field_distance` only (the critic's blowup — rejected); marching ALL cluster-slice casters unconditionally (unbounded multiplier — rejected for the dominant-N cap).

**Trade-off:** Binding the brick atlas to the resolve adds descriptors (covered by the cap raise to 16) and VGPR pressure. The dominant-N cap is an approximation (the (N+1)th-nearest occluded light shows no shadow) — acceptable: distant/dim casters' shadows are visually negligible, and N=4 matches the cluster-local light expectation. Tunable const.

### Decision 3: The marcher body stays FROZEN; the eDSL emits a SEPARATE `sdf_soft_shadow_ranged` for the resolve (B3 — option a)

**What:** The marcher's `sdf_soft_shadow(p,n,L)` (T_MAX-hardcoded) is **left literally untouched** — same generator entrypoint, same emitted span, same spliced bytes, same frozen `.comp.spv`. The eDSL `boyko_shaderdsl::shadow` gains a **second, separately-named entrypoint** emitting `sdf_soft_shadow_ranged(p,n,L,t_max)` (the `t_max` parameter only on the NEW function). The resolve splices `sdf_soft_shadow_ranged`; the marcher splices the unchanged `sdf_soft_shadow`.

**Why (B3):** A new function PARAMETER changes SPIR-V signature / OpFunctionParameter / SSA numbering even when the value is `T_MAX` — NOT obviously byte-free. By never touching the marcher's emitted body, the frozen `.comp.spv` *cannot* move. The proof is structural (identical input → identical output bytes) rather than a fragile "DXC folds the param" assertion.

**Alternatives rejected:** parameterize the single `shadow` generator and pass `T_MAX` at the marcher call site (B3's option b) — rejected because it requires *proving* DXC re-emits the marcher span byte-identically, a risk on the inviolable 0%-gate foundation. Hand-write the resolve march (violates the eDSL single-source rule + drift risk).

**Trade-off:** Two emitted functions share the inner march logic in the eDSL generator (one parameterized helper, two thin wrappers — the duplication is in the generator's *output*, not its source). The marcher's frozen cmp-`.spv` re-pin remains a gating step-1 deliverable with **byte-identity as the explicit exit criterion**, with the separate-entrypoint structure guaranteeing it.

### Decision 4 (R2b): SDF-visibility cache is LIGHT-SPACE, becomes a depth-compare, directional-only (B4)

**What:** When a **directional** light AND the SDF edit-list are both static, bake a **light-space** SDF-visibility/depth map **once** by marching from the LIGHT's ortho frustum texels into the field (NOT from the visible surface P — camera-independent). The resolve then computes `sdf_vis_l = depth_compare(sdf_shadow_map_l, light_vp_l * P, bias)` — i.e. the cached SDF path **becomes a shadow-map test of identical shape to the mesh path**, sampled not re-marched. Camera motion does NOT invalidate it.

**Invalidation set (complete, B4):** light transform change, SDF edit-authority dirty, field-param change. **NOT** camera motion, **NOT** receiver motion (receivers are the field itself — covered by the SDF-edit dirty). Cube/DPSM bake for punctual lights is OUT of scope (deferred with the punctual mesh-map work).

**Why (B4):** §3.2's `[DERIVED]`: a shadow-map-of-SDF only wins **cached**. A screen-space-from-P bake invalidates every camera frame → zero amortization → the whole "map-of-SDF" justification collapses. Light-space is the only space that amortizes. Reconciling with the screen-space A1 march: they are DIFFERENT marches; the cached path is a projection+compare, made explicit here, so the resolve "samples instead of marches" correctly.

**Alternatives rejected:** screen-space-from-P cache (invalidates per camera frame — pointless); always-cache (stale shadows on a moving light/edited SDF — correctness bug); never-cache (static sun pays full march every frame).

**Trade-off + boundary check (folded concern):** R2b's invalidation reads `LightTableDirty` + the SDF-edit dirty flag. These live in the edit authority / light table — **GATING FEASIBILITY CHECK:** R2b ships ONLY IF these dirty signals are already readable from the host `swapchain.rs` recorder WITHOUT a `boyko_render`/`boyko_ecs` change (the INVIOLABLE boundary). **R2a (no cache) is clean and ships regardless;** R2b is contingent on this check passing. If it fails, R2b is re-scoped or dropped — R2a's correctness does not depend on it.

### Decision 5 (R2a): Rasterized MESH shadow map, MIN'd, DIRECTIONAL-ONLY (B5)

**What:** For each **directional** light flagged a mesh-shadow caster (host-set, ≤`MAX_SHADOW_CASTERS`), a light-space depth-only raster pass renders the mesh (P5's geometry, re-drawn with a light-space ortho view-proj) into a per-light `D32` map. In the resolve, `mesh_vis_l = PCF_compare(map_l, light_vp_l*P, bias)`; `vis_l = min(sdf_vis_l, mesh_vis_l)`. **Point/spot lights are FORBIDDEN as mesh-casters in R2a** (`mesh_vis=1` for them).

**Why (B5):** A shadow-map of the SDF half loses uncached (every texel marches the field); a map of the mesh half is ~free (triangles rasterize). The `min` is the correct hybrid union. Directional-only because a point light needs a cube (6 faces) / dual-paraboloid — costlier, deferred. Surfacing this in the Goal (not Open Questions) per B5.

**Host guard (B5):** the host rejects + `debug_assert`s any punctual light flagged as a mesh-caster; `mesh_caster_count` counts directionals only. The host cannot silently believe a point light mesh-shadows.

**Alternatives rejected:** mesh-as-SDF march (no field — impossible without a bake); unified map of both (SDF-map texel-march loss); screen-space contact shadows as THE mesh shadow (misses off-screen + large-penumbra occlusion — a future augment, not P6).

**Trade-off:** The high-risk half — separate rung. Directional-only leaves the punctual-mesh-shadow gap (stated in Goal). Light-space fit (folded concern): **single tight ortho over the mesh AABB; cascades deferred.** Bias guards acne (peter-panning trade-off; owner-retunable const like the A1 `SHADOW_K`).

### Decision 6: Auto-select keys on "the single caster IS the gMaterial.r directional" (B1 + folded concern)

**What:** Host policy:
```
shadow_mode = (the ONLY shadow caster is the directional the marcher marched into gMaterial.r) ? 0 : 1
```
NOT `count==1`. A scene with exactly one POINT light (count==1, but punctual — the marcher set `gMaterial.r=1` unshadowed) takes `shadow_mode==1` so it gets its SDF shadow via the resolve-march. Only the strictly-single-*directional* scene (whose shadow IS `gMaterial.r`) takes `shadow_mode==0`.

**Why:** The folded concern: keying on `count==1` would route a single-point-light scene to `shadow_mode==0`, reading a `gMaterial.r` the marcher set to 1.0 → the point light silently loses its SDF shadow. Keying on "is the single caster the gMaterial.r directional" is the correct gate and keeps the 0%-gate automatic for exactly the scene it covers.

**Trade-off:** None — a precise host-side classification, no shader cost.

### Decision 7: Cluster-slice bounds PUNCTUAL marches; DIRECTIONALS are unbounded-per-pixel (folded concern)

**What:** Point/spot marches iterate the existing cluster slice (`use_clusters ? LightIndexList[ps_offset+jj] : ps_offset+jj`) — a light the cluster dropped is never marched. **Directionals are in the L0a front block (`i=0..l0a_count`), looped UNCONDITIONALLY** — they reach everywhere, so they are NOT cluster-bounded. With M directionals, every `is_sdf_lit` pixel pays up to M marches (minus the primary `gMaterial.r` one, minus dominant-N cap, minus back-face skip).

**Why:** Correct (directionals reach all pixels) but the perf model MUST separate the **unbounded M-directional term** from the **cluster-bounded K-punctual term**. Today M=1 (and that one reads `gMaterial.r`, costing zero resolve-march), so the realistic resolve-march count is `min(extra_directionals + cluster_punctual_casters, MAX_SDF_SHADOW_CASTERS_PER_PIXEL)`.

**Trade-off:** A 2nd+ directional is a per-pixel unconditional march (capped by dominant-N). Acceptable: multi-directional scenes are rare; the cap bounds it.

## Data structures

### Resolve push constant (NEW — resolve was push-less)
```hlsl
// deferred_pbr.hlsl — NEW push. shadow_mode==0 reproduces today (read gMaterial.r, no march).
[[vk::push_constant]] struct ResolvePush {
    uint  shadow_mode;        // 0 = legacy single-directional gMaterial.r (BYTE-IDENTICAL today)
                              // 1 = R1: gMaterial.r for primary dir + per-light ranged march for extras
                              // 2 = R2: + directional mesh-map MIN
    uint  mesh_caster_count;  // R2: # directional mesh maps bound (0 on R1; NEVER counts punctual)
    float shadow_bias;        // R2 depth-compare bias; unused on R0/R1
    uint  _pad;               // std430 16-B tail
} rpc;
```

### Light table flag word (B2 — `casts_sdf_shadow`, no layout change)
`GpuLight` keeps its frozen 12-word layout. The per-light `casts_sdf_shadow` bit + the mesh-caster bit are packed into an **existing** flag/type word in `GpuLight` (the kind/flags field already present) — NO new word, frozen `light_table.hlsli` element size preserved. The host sets these bits; the resolve tests them.

### R2 shadow-map resources (preallocated at scene-build — Principle 5)
```rust
// swapchain.rs GBufferTargets — NEW, Some only when R2 wired.
struct ShadowCasters {
    maps:       Texture2DArray<D32_SFLOAT>,   // MAX_SHADOW_CASTERS slices, one resolution (concern)
    light_vp:   [Mat4; MAX_SHADOW_CASTERS],   // light-space ortho view-proj per caster (UBO-bound)
    static_valid: u32,                        // R2b: bit i => map i is a baked static map, skip rebuild
}
```
Preallocated once at `GBufferTargets` build; a caster flip is a re-raster into the existing slice (a fill), never a mid-frame `vkCreateImage`. `Texture2DArray` = 1 descriptor (binding-cap-friendly; couples all maps to one resolution — accepted concern).

### Binding budget (concern — the plan, not an afterthought)
**The resolve descriptor-set cap is raised 12→16** (mirrors the marcher's prior raise to 16). R1 adds: `Buf` (edit-list SSBO) + `PointerGrid` + `BrickAtlas` (+ its sampler) read-only = current 10 → ~14. R2 adds: the `Texture2DArray` of maps + the light-VP UBO = → ~16. Within the raised cap. The `Texture2DArray` keeps R2's maps to a single descriptor.

## Public API (host-side; no boyko_render/ecs change)
```rust
// GBufferScene — NEW optional fields (None => byte-identical pre-P6 stream).
pub shadow_mode: u32,                                  // 0/1/2 (R1 ships 0|1)
pub resolve_push: [u8; RESOLVE_PUSH_BYTES as usize],   // the 16-B ResolvePush
pub shadow_casters: Option<&'a ShadowCasters>,         // R2: bound maps + light-VP UBO
pub shadow_caster_lights: &'a [u32],                   // R2: DIRECTIONAL table indices only (≤ MAX)

// compute.rs goldens (the HARD gate):
fn golden_deferred_resolve_multilight_shadow(...) -> [u8; 4];  // R1 oracle (ranged brick march)
fn golden_shadow_map_compare(...) -> f32;                       // R2 mesh-map depth test oracle
```
Host classifies `shadow_mode` per Decision 6; rejects punctual mesh-casters per Decision 5.

## Algorithms for critical paths

### R1: per-light SDF shadow in the resolve (brick-accelerated, dominant-N-bounded)
```
# primary directional: ALWAYS gMaterial.r (Decision 1) — no resolve march, byte-stable across 1->N
marched := 0
for each directional L in [0..l0a_count):
    NoL = dot(n, L.dir); if (NoL <= SHADOW_NDOTL_EPS) continue   # skip lighting + march
    if (L is the gMaterial.r primary):  vis_l = gMaterial.r
    elif (shadow_mode!=0 && L.casts_sdf_shadow && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL):
        vis_l = sdf_soft_shadow_ranged(P, n, L.dir, T_MAX); marched++   # brick-skipped march
    else: vis_l = 1.0
    lit_direct += (diff+spec) * (NoL * vis_l) * L.color
for each point/spot L in cluster-slice [ps_offset..ps_offset+ps_count):   # L1-bounded
    NoL = dot(n, Ldir); if (NoL <= SHADOW_NDOTL_EPS) continue
    Ldir = normalize(L.pos - P); dist = length(L.pos - P)
    if (shadow_mode!=0 && L.casts_sdf_shadow && marched < MAX_SDF_SHADOW_CASTERS_PER_PIXEL):
        vis_l = sdf_soft_shadow_ranged(P, n, Ldir, dist); marched++      # range-bounded brick march
    else: vis_l = 1.0
    lit_direct += (diff+spec) * (NoL * vis_l) * atten * L.color
```
- **Steps/complexity:** per shadowed SDF pixel, `O(min(extra_dir + cluster_punctual_casters, N_CAP) × march_steps_bricked × ≤16 edits)`, shadowed-pixel-only (under `mask==1`), NoL-skip + dominant-N cap + brick empty-space skip.
- **GPU behavior (concern — corrected machine model):** the cost is GPU occupancy / VGPR pressure from inlining the field march into the already-large resolve, plus per-step latency of the field fold across the 64-wide wave and divergence on the occluder early-out. The edit list (≤16 edits, ≤768 B) is tiny + scalar-cacheable; brick fetches are the dominant memory term, shared with the marcher's primary path.
- **SIMD:** independent per-caster marches, FMA-chained field body; `shadow_mode` wave-uniform (OFF arm DCE-strips). Divergence concentrated at the occluder early-out — bounded by brick skipping.

### R2a: mesh shadow map build + MIN (directional-only)
- **Build (per directional caster, once if static via R2b):** depth-only mesh raster with the light-space ortho view-proj → `D32` slice. `O(triangles)`. Barrier DEPTH_WRITE→SHADER_READ (the P5 dual-use depth-barrier pattern, per caster). Light-space fit = single tight ortho over the mesh AABB (cascades deferred).
- **Resolve combine:** `mesh_vis_l = PCF_compare(map_l, light_vp_l*P, bias)`; `vis_l = min(sdf_vis_l, mesh_vis_l)`. O(1) (or O(PCF-K)) in edits — the structural mesh-shadow win.
- **R2b cache:** if `static_valid` bit set, skip the build, sample the baked LIGHT-SPACE map (Decision 4). Invalidate per Decision 4's set.

## Perf model (B2 + folded concerns)

Let `S` = SDF-lit screen coverage, `K` = effective resolve-march casters/pixel = `min(extra_directionals + cluster_punctual_casters, MAX_SDF_SHADOW_CASTERS_PER_PIXEL=4)`.

- **`shadow_mode==0` (strictly single directional):** **0 added cost** — identical `.comp.spv` arm, identical dispatch/barrier stream, `gMaterial.r` read as today. The 0%-gate.
- **Unoccluded ray (the COMMON case, B2-derived):** soft-shadow only early-outs on an occluder HIT, so an unoccluded ray marches to `T_MAX=10.0`. With brick empty-space skipping the step count is `~T_MAX / mean_brick_skip` (the marcher's primary visibility step distribution), **NOT** `T_MAX / SHADOW_MINT_STEP` (the brute-force Lipschitz floor). This is the load-bearing fix: brick skipping turns the worst-case full march from hundreds of MINT-floor steps into the marcher's already-shipped brick-step count.
- **R1 per-frame field cost:** `~ S × K × (bricked_march_steps × ≤16 edits)`. Worst case (full-screen SDF, K=4, all unoccluded full marches): `4 ×` the marcher's single bricked shadow march, on SDF pixels only. The dominant-N cap + per-light `casts_sdf_shadow` flag + NoL-skip keep K well below the raw cluster slice.
- **R2 mesh map:** per shadowed pixel, `mesh_caster_count ×` one (or PCF-K) depth-compare fetch — O(1) in edits. Build `O(triangles)/caster`, cached → ~0/frame amortized for static directionals (R2b).

**Rung-level perf-gate test (B2 exit criterion):** a full-screen SDF scene with K=4 flagged casters must not exceed **`PERF_GATE_MULT × `** the 1-caster (`shadow_mode==0`) frame time (proposed `PERF_GATE_MULT=5.0`, owner-retunable). R1 does NOT ship if this fails — the brick-acceleration + dominant-N cap are validated by this gate, not asserted.

## Multithreading model
- GPU-only; CPU side is the existing single-threaded recorder under `DispatcherToken` on the `!Send` GPU thread (unchanged).
- **R1:** no new GPU sync. The per-light march is in-shader in the existing resolve dispatch (already barriers the G-buffer stores). The resolve gains read-only access to `Buf` + `PointerGrid` + `BrickAtlas` — **the SAME buffers the marcher already uploaded + barriered for its COMPUTE read** (concern-confirmed: the resolve dispatch is ordered after the marcher in the same submit, so the prior upload+barrier covers the resolve's second compute-read — no new barrier).
- **R2:** per-caster light-space depth raster, each with a DEPTH_WRITE→SHADER_READ barrier before the resolve reads it (the P5 dual-use pattern). Maps: write-once-per-frame (or once-if-static) by a single raster, read-only in the resolve, ordered by one barrier each — no concurrent writer.
- **Data-race freedom:** R1 adds only reads of already-ordered buffers. R2's maps are single-writer-raster → barrier → read-only-resolve. **Send/Sync unchanged** (all GPU access inside `DispatcherToken`).

## Integration
- **Modified shaders:** `deferred_pbr.hlsl` (R1: `#include "sdf_field.hlsli"`, bind `Buf`+brick atlas read-only, add `ResolvePush`, the per-light `sdf_soft_shadow_ranged` term + `shadow_mode` gate + dominant-N + NoL-skip, primary directional KEEPS `gMaterial.r`; R2: map sample + `min`). `sdf_gbuffer_composite.hlsl` — **byte-FROZEN** (the marcher's `sdf_soft_shadow` untouched; on multi-light the host clears the marcher's shadow `lighting_flags` bit for *extra* lights only — but the primary directional's `gMaterial.r` write is UNCHANGED, value-identical).
- **New shaders (R2 only):** `shadow_map.vs.hlsl` / `shadow_map.fs.hlsl` (light-space depth-only mesh raster).
- **eDSL (B3):** `boyko_shaderdsl::shadow` adds a SEPARATE `sdf_soft_shadow_ranged` entrypoint (param `t_max`); the marcher's `sdf_soft_shadow` emit is UNCHANGED. Re-splice only the resolve; the marcher's cmp-`.spv` is re-pinned with **byte-identity as the exit criterion** (guaranteed by the untouched body).
- **Host (`swapchain.rs`):** add `ResolvePush`; bind `Buf`+brick atlas to the resolve set (R1); record per-caster light-space depth rasters + barriers + bind the `Texture2DArray` + light-VP UBO (R2); classify `shadow_mode` (Decision 6); reject punctual mesh-casters (Decision 5).
- **Host goldens (`compute.rs`):** `golden_deferred_resolve_multilight_shadow` (mirror the ranged brick march + the L0a/L1 loop + dominant-N + the gMaterial.r-primary rule); `golden_shadow_map_compare` (R2). `host_soft_shadow` reused as the per-light march oracle.
- **Compatibility:** `Arena`/`ComponentPool`/`UnitId` untouched. Light table, cluster grid/list, mesh raster, D32 depth, brick atlas consumed as shipped.

## Implementation plan (for the developer)

**Rung R1 — multi-light SDF shadows (ship first, standalone):**
1. `boyko_shaderdsl`: add a SEPARATE `sdf_soft_shadow_ranged(p,n,L,t_max)` entrypoint; **do NOT touch** the marcher's `sdf_soft_shadow` emit. **Exit criterion:** the marcher's `sdf_gbuffer_composite.comp.spv` re-DXCs **byte-identical** (the body is unchanged — structural guarantee).
2. `deferred_pbr.hlsl`: `#include "sdf_field.hlsli"`; bind `Buf` + `PointerGrid` + `BrickAtlas` read-only; add `ResolvePush`; implement the R1 loop (Decision 1/2/7: primary-directional `gMaterial.r`, ranged brick march for extras, `casts_sdf_shadow` flag, `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` cap, NoL-skip).
3. `swapchain.rs`: bind the brick atlas + `Buf` to the resolve set (camera already bound); add `ResolvePush`; classify `shadow_mode` per Decision 6; clear the marcher shadow `lighting_flags` bit for EXTRA lights only (primary directional's `gMaterial.r` unchanged).
4. `compute.rs`: `golden_deferred_resolve_multilight_shadow`; a multi-light GPU golden (1 primary directional via gMaterial.r + 1 extra point with an SDF occluder → the point shadowed, the directional byte-stable).
5. **Perf-gate test** (K=4 full-screen SDF ≤ `PERF_GATE_MULT ×` 1-caster frame). Pixel golden = hard gate (`BOYKO_DISABLE_VALIDATION=1`); the strictly-single-directional 0%-gate golden = byte-identical; dump screenshot; commit + push.

**Rung R2a — directional mesh shadow map MIN (separate later commit):**
6. `shadow_map.{vs,fs}.hlsl`: light-space depth-only mesh raster (single tight ortho over mesh AABB).
7. `swapchain.rs`: `ShadowCasters` preallocated `Texture2DArray` + light-VP UBO; per-caster raster pass + DEPTH_WRITE→SHADER_READ barrier; bind to the resolve; **reject + `debug_assert` punctual mesh-casters**.
8. `deferred_pbr.hlsl`: `shadow_mode==2` → `mesh_vis_l = PCF(map,...)`; `vis_l = min(sdf_vis_l, mesh_vis_l)`.
9. `compute.rs`: `golden_shadow_map_compare`; a hybrid GPU golden (a mesh wall shadowing an SDF floor under ONE directional). Pixel golden + screenshot; commit + push.

**Rung R2b — light-space SDF-visibility cache (contingent):**
10. **GATING feasibility check (Decision 4):** confirm `LightTableDirty` + SDF-edit dirty are readable from `swapchain.rs` WITHOUT touching `boyko_render`/`boyko_ecs`. If NO → re-scope/drop R2b (R2a stands).
11. If YES: bake light-space SDF-visibility map (march from light frustum); `static_valid` gate; resolve samples via `depth_compare(sdf_shadow_map, light_vp*P, bias)`. Invalidate per Decision 4's full set. Golden + screenshot; commit + push.

## Metrics and validation

**Benchmarks:**
- Strictly-single-directional (0%-gate): assert resolve dispatch + command stream + `.comp.spv` arm byte-identical pre/post — no perf delta.
- N-light SDF (R1): GPU time vs caster count; confirm the dominant-N cap + cluster bound (cost flat past `MAX_SDF_SHADOW_CASTERS_PER_PIXEL`).
- **Perf-gate (R1 exit):** K=4 full-screen SDF ≤ `PERF_GATE_MULT × ` 1-caster frame.
- R2: map build cost vs triangle count; cached (R2b static) vs live per-frame delta.

**Mandatory pixel/unit tests (HARD gate):**
- `resolve_single_directional_byte_identical` — `shadow_mode==0` bit-matches the pre-P6 golden (the SCOPED 0%-gate).
- `marcher_spv_byte_frozen` — the marcher `.comp.spv` re-DXCs byte-identical (body untouched — B3).
- `resolve_multilight_sdf_shadow` — primary directional via gMaterial.r (byte-stable) + an extra occluded point darkened, others unchanged (vs the host oracle).
- `single_point_light_gets_sdf_shadow` — Decision 6: a count==1 POINT scene routes to `shadow_mode==1` and shadows (NOT `shadow_mode==0`).
- `dominant_n_cap` — beyond `MAX_SDF_SHADOW_CASTERS_PER_PIXEL` casters, extra lights contribute NoL-only (no march).
- `sdf_field_edsl_sync` — the resolve's `sdf_soft_shadow_ranged` splice matches the generator; the marcher splice unchanged.
- R2: `shadow_map_depth_compare`; `hybrid_mesh_shadows_sdf` (a mesh occluder darkens an SDF pixel for the directional caster only); `punctual_mesh_caster_rejected` (host rejects a point light flagged as mesh-caster).

**Property tests:**
- `vis_l ∈ [0,1]` for all light kinds / all `P`.
- `min(sdf_vis, mesh_vis) ≤ each` (R2 combine soundness).
- NoL≤0 ⇒ march NOT entered (semantics + cost).

**debug_assert! invariants:**
- `mesh_caster_count <= MAX_SHADOW_CASTERS` AND every mesh-caster is DIRECTIONAL (B5).
- `marched <= MAX_SDF_SHADOW_CASTERS_PER_PIXEL`.
- punctual `t_max = dist(P, light) > 0` before the ranged march.
- `shadow_mode ∈ {0,1,2}`; `shadow_mode==2 ⇒ shadow_casters.is_some()`.
- `shadow_mode==0 ⇒ the single caster is the gMaterial.r directional` (Decision 6).
- resolve binding count ≤ 16 (raised cap); marcher ≤ 16.

## Open questions (residual — non-blocking)
1. **`MAX_SDF_SHADOW_CASTERS_PER_PIXEL` + `PERF_GATE_MULT` values:** proposed 4 and 5.0 — owner-retunable consts; confirm or set via the perf-gate measurement on the target scene.
2. **R2 shadow-map resolution + `MAX_SHADOW_CASTERS`:** the `Texture2DArray` couples all maps to one resolution; proposed a single fixed resolution + `MAX_SHADOW_CASTERS=4`. Confirm.
3. **Punctual mesh shadows (cube/DPSM):** explicitly DEFERRED past R2a (stated in Goal). The hybrid-mesh-shadow gap for point/spot persists until that follow-up rung; point/spot still get SDF shadows (R1). Confirm the scoping (charted follow-up, not P6).

---

**Grounded files (all absolute):**
- Marcher shadow march + `gMaterial` write (FROZEN): `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\sdf_gbuffer_composite.hlsl` (`sdf_soft_shadow` ~450–478; T_MAX hardcode ~472; gMaterial layout 28–49; eDSL splice 454–477).
- Resolve per-light loop + single-channel shadow + camera b5: `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\deferred_pbr.hlsl` (shadow read ~220; per-light `NoL*shadow` ~279, ~382; cluster slice 310–331; `P` reconstruct 299–300; `safe_normalize` 183–191).
- Brick atlas / field (FROZEN): `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\sdf_field.hlsli` (`field_distance` ~218; brick bindings t9–t14).
- Cluster cull + index list (reuse): `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\cluster_cull.hlsl`; layout `light_table.hlsli` (`ClusterParams`/`cluster_*` 100–164; `GpuLight` 56–88).
- Mesh raster (P5, casts no shadow today): `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\gbuffer_mrt.fs.hlsl` (gMaterial=(1,1,1,1) ~93; eDSL splice 59–80).
- Barrier schedule + dispatch order: `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\swapchain.rs` (G-buffer store→load barrier 2971–3013; cluster_cull pass 3015–3132 = R2 raster insertion point; resolve dispatch 3134+; D32 depth + dual-use barrier 2627–2645; marcher push 2886–2917).
- Roadmap: `D:\claude\BoykoEngine\docs\ARCHITECTURE-HYBRID-PERF.md` (P6 line 271; §3.2 shadows 197–207; Owner-call #2 line 297).

---

## DECISION (orchestrator, 2026-06-26): R1 ships the ANALYTIC ranged march (A); brick acceleration → R1b (measure-first)

The R1 developer correctly found Decision 2 (brick-accelerated march) and Decision 3 (a `t_max`-parameterized *variant of the existing analytic `sdf_soft_shadow` leaf*) are **mutually exclusive**, and that **Decision 2's premise is factually wrong**: the marcher's own `sdf_soft_shadow` (the body that writes `gMaterial.r`) is **analytic** (`field_distance` = `sdf`), NOT brick-accelerated — the brick empty-space-skip exists ONLY in the marcher's *primary surface march*, a different control-flow leaf. So there is **no existing brick-accelerated shadow leaf to "reuse"**; option (B) would be a substantial NEW control-flow leaf + wiring grid geometry (`grid_origin`/`grid_dims`/`brick_world`) into the resolve (a new push/UBO) + binding `PointerGrid`/`BrickAtlas` into the resolve set (cap→16).

**RESOLUTION = (A): R1 ships the analytic `t_max`-clone ranged march** (`sdf_soft_shadow_ranged` = `sdf_soft_shadow` with the hardcoded `T_MAX` replaced by a `t_max` parameter). Rationale, perf-first: (1) the marcher's OWN shadow is analytic, so **analytic parity is the correct R1 baseline** — R1 is not a new brute-force hole, it is the SAME perf class as the shipped single-light shadow; (2) the march is **bounded** — `t_max` = the light DISTANCE for punctual lights (the common multi-light case = nearby point/spot → short, cheap), `T_MAX` only for the rare extra-directional caster; (3) the dominant-N cap (`MAX_SDF_SHADOW_CASTERS_PER_PIXEL`) + the `NoL<=0` skip + the cluster-slice gating bound the per-pixel march count; (4) **measure-first** — do not add the large brick shadow leaf speculatively. **Brick acceleration is RE-SCOPED to R1b**, a measure-first follow-up added IF a profile shows the analytic resolve-march dominant on a far/large-range-light scene. This DROPS Decision 2's brick requirement AND the cap-raise-to-16 from R1: R1 adds only the edit-list `Buf` (the analytic `field_distance` needs it) at one free resolve binding (12→13), and bumps the `DEFERRED_PBR_SPV` `SpirvBlob<15252>` length to the new size. The marcher `.comp.spv` (122812) stays frozen.
