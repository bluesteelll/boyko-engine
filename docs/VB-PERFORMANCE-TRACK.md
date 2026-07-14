# VisibilityBuffer performance track — honest assessment + optimization roadmap

**Status:** ASSESSMENT + ROADMAP (not yet implemented). Companion to
[MULTI-PARADIGM-RENDER-PLAN.md](MULTI-PARADIGM-RENDER-PLAN.md) (the VB path shipped there, rungs
R7/R-VBGEO/R8/R10) and [RENDER-PARITY-PLAN.md](RENDER-PARITY-PLAN.md) (textures + SDF shadows,
in flight). This doc answers the owner's question — *"is our VB implementation the most performant
and optimal?"* — honestly, and lays out the concrete work that would make it so.

---

## 0. Verdict (the honest one-liner)

**The foundation is the right, bandwidth-optimal choice — which most engines get wrong — but the
current implementation is a v1 that deliberately omits several well-known SOTA optimizations. It is
NOT "the most performant/optimal" in absolute terms; it is "architecturally sound and
bandwidth-optimal at the top level."**

The single most valuable thing we did — **pure Visibility Buffer, no G-buffer materialization** —
is exactly the decision Nanite's "VB → thin-gbuffer" and the filmicworlds VB→gbuffer approaches
compromise on, throwing away roughly half the bandwidth win. We keep it. On top of that we have
analytic barycentric derivatives and bindless geometry re-fetch. Those are genuinely SOTA choices.

What we do NOT yet have: clustered light culling, material classification, geo/shade split, and a
GPU-driven raster. Each is a concrete, well-understood optimization with a known perf profile.

---

## 1. Current VB architecture (what shipped)

The path (R7 → R-VBGEO → R8 → R10), grounded in the source:

- **`vb_raster`** — id-raster writing **`R32G32_UINT`** (`{global_instance_id, raw SV_PrimitiveID}`,
  8 bytes/pixel) + a HW reverse-Z depth (`GREATER`, early-Z-clean). Decision 9: `local_tri =
  raw_prim_id % tri_count`. A standard `vkCmdDrawIndexed` per `DrawBatch` (`passes/vb.rs`
  `record_vb`).
- **`vb_resolve`** — a **FUSED** compute pass (`shaders/vb_resolve.comp.hlsl`): reads `vb_id`,
  re-fetches vertices/indices through the bindless **`MeshGeometryTable`** (Set 2,
  `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta`), computes **analytic DAIS barycentrics**
  (`vb_geom_fetch.hlsli` + the R7 eDSL `vb_barycentric`/`vb_uv_grad`), interpolates
  position/normal/color/uv, and shades with the shared PBR BRDF (`pbr_lighting.hlsli`) +
  `shadow_apply.hlsli`, then writes `lit`. Sky survives sentinel misses (`VB_ID_SENTINEL`).
- **Material** — flat per-instance lookup `Materials[pm.id]` (via the `PerInstanceMaterial` ring).
  No classification, no per-material shader.
- **Lights** — the resolve does an **ALL-LIGHTS flat scan**: an L0a directional/sky loop
  (`for i < l0a_count`) + an L0b point/spot loop (`for j in l0a..light_count`), **no cluster/froxel
  lookup** (`vb_resolve.comp.hlsl:280` "ALL-LIGHTS — no cluster/froxel lookup, VB v1 is fused-only,
  mirrors plain Forward's own base compile").
- **Resolver flags** — `mesh_geo_shade_split == false` (fused, `render_path_config.rs:397`);
  `cap_vb_v1_consumers` forces SSAO/DDGI/denoise/TAA **OFF** under VB
  (`VbPreLightConsumersNotYetImplemented` / `VbTaaNotYetImplemented`, `render_path_config.rs:569,573`).

---

## 2. What is genuinely SOTA-correct (the wins — keep these)

| Choice | Why it is right | Evidence |
|---|---|---|
| **Pure VB, no G-buffer materialization** | The 8 B/px id-buffer is the whole bandwidth premise of VB. Materializing a fat G-buffer (Nanite's VB→gbuffer, filmicworlds) throws away ~half the win. | `vb_raster` writes `R32G32_UINT` only, never a G-buffer |
| **Analytic barycentric derivatives (DAIS)** | Compute has no hardware `ddx/ddy`; analytic gradients (Wihlidal 2016 / Nanite) give correct texture LOD without a G-buffer or a derivatives pass. | `vb_geom_fetch.hlsli` `vb_uv_grad`, R7 eDSL |
| **Bindless per-mesh geometry re-fetch** | Re-fetch from a bindless table (Nanite / The Forge TVB) instead of storing interpolants — the classic VB space/bandwidth trade. | `MeshGeometryTable`, Set 2 |
| **Compute resolve + single shared BRDF** | Deferred-style compute shading; one BRDF source across all four paths (no divergence). | `vb_resolve.comp.hlsl` + `pbr_lighting.hlsli` |

At the top architectural level, this is a correct, modern VB — and *more* bandwidth-optimal than a
typical deferred or a VB→gbuffer hybrid.

---

## 3. Gaps vs SOTA (what keeps it from "the most optimal")

### G1 — No clustered/tiled light culling (BIGGEST perf gap)
The resolve shades **every pixel against every light** (`O(pixels × lights)`). SOTA VB (The Forge
Triangle Visibility Buffer, any clustered pipeline) bins lights into froxels/tiles and each pixel
touches only its cluster's lights. For many-light scenes this is a multiplicative loss.
- **Evidence:** `vb_resolve.comp.hlsl:280,310` — two flat all-lights loops, no cluster lookup.
- **Perf impact:** linear in total light count instead of ~constant per-pixel; dominates on dense
  lighting.

### G2 — No material classification / material-tiled shading
The classic VB optimization (Burns & Hunt, Intel 2013; The Forge) **classifies pixels by material**
and shades each material in its own pass, so a pixel runs only its material's shader (no über-shader
carrying every material's texture-sampling + branch paths). We use **one über-resolve** with runtime
`Materials[pm.id]` params.
- **Evidence:** flat `Materials[pm.id]` lookup; one resolve `.spv`.
- **Perf impact:** über-shader occupancy/register pressure + branch divergence across materials in a
  warp; worsens as material variety + texture-sampling paths grow (the TV0 textured resolve adds all
  texture-sampling paths to the one über-shader — see §6).

### G3 — Fused, not geo/shade split
`mesh_geo_shade_split == false` — geometry fetch + shade are one dispatch. The split form (geo
pre-tail → a thin `sdf_surface_cache`-style surface cache → shade post-tail) is what unlocks
thin-aux consumers (SSAO/TAA/DDGI) *and* changes the perf profile (shade once from a cached surface
instead of re-fetching under overdraw-free VB).
- **Evidence:** `render_path_config.rs:397` (`mesh_geo_shade_split`), `cap_vb_v1_consumers`
  (`:569,573`).
- **Impact:** SSAO/TAA/DDGI are structurally impossible under VB v1 (feature gap); no surface-cache
  reuse.

### G4 — Standard raster, not GPU-driven / mesh-shader cluster culling
The id-raster is a CPU-recorded `vkCmdDrawIndexed` per batch — not the GPU-driven
cluster-cull + mesh-shader pipeline that makes Nanite fast on high-poly scenes. Per-pixel vertex
re-fetch can also be cache-unfriendly on high-poly meshes.
- **Evidence:** `record_vb` draw loop; single-vendor (RTX), `feature = hwrt` gated off by default.
- **Impact:** no fine-grained culling; re-fetch cache misses scale with triangle density.

---

## 4. Optimization roadmap (prioritized, with reuse targets)

Each rung: golden byte-identity gate (VB base `vb_mesh f4719cbf` unchanged when the optimization is
off), criterion + GPU-capture benchmark proving the win, dev → code-reviewer → orchestrator verify,
author-only commit+push. Ordered by **perf-win-per-effort**.

| Rung | What | Why (perf) | How / reuse | Size |
|---|---|---|---|---|
| **VB-P1** Clustered light culling in `vb_resolve` | Replace the all-lights flat scan with a froxel/cluster lookup | Kills G1 — the biggest win; `O(pixels × lights)` → ~`O(pixels × lights_per_cluster)` | **Reuses `cluster_cull.hlsl` + the ForwardPlus `#ifdef FROXEL` light-cull infra verbatim** (already in tree, built for ForwardPlus). Add a `#ifdef FROXEL` seam to `vb_resolve` mirroring `forward_opaque.fs`. App-side must arm the cluster buffers (the known unwired L1-cluster gap — `scene()` hardcodes `cluster_* None`). | **M** |
| **VB-P2** Material-tiled classification | Classify pixels by material id → per-material shade passes (or an indirect-dispatch material tile pass) | Kills G2 — removes über-shader occupancy/divergence; each pixel runs only its material shader; pairs with textures (each material samples only its own maps) | New material-classify compute (histogram/prefix-sum over `vb_id → mat_id`) + indirect per-material dispatch. Burns & Hunt (Intel 2013, `info/Burns2013Visibility.pdf`) is the reference. Larger; changes the resolve from one über-dispatch to a classified pipeline. | **L** |
| **VB-P3** Geo/shade split | Split fused resolve → geo pre-tail + surface cache + shade post-tail | Kills G3 — unlocks SSAO/TAA/DDGI under VB (lifts `cap_vb_v1_consumers`) + surface-cache reuse | The resolver already models it (`mesh_geo_shade_split`, `sdf_surface_cache`). This is the plan's deferred **R9** rung. | **L** |
| **VB-P4** GPU-driven cluster culling / mesh shaders | Replace CPU `vkCmdDrawIndexed` with GPU cluster-cull + mesh-shader raster | Kills G4 — fine-grained culling + Nanite-class high-poly scaling | Largest; needs a cluster builder + `VK_EXT_mesh_shader` + GPU-driven indirect. New subsystem, not a shader tweak. | **XL** |

**Recommended order:** VB-P1 first (best win/effort, near-total infra reuse), then VB-P2 (the
material-classification optimum, especially valuable now that textures are landing), then VB-P3
(features + surface cache), then VB-P4 (the big one) only if high-poly Nanite-class scaling is a
real target for this engine.

---

## 5. Positioning vs the references

- **vs Nanite:** we share pure-VB + analytic barycentrics + bindless re-fetch, but Nanite adds
  GPU-driven cluster culling + mesh shaders (VB-P4) + material tiles (VB-P2). We match its *bandwidth*
  discipline, not its *culling/classification* pipeline.
- **vs The Forge Triangle Visibility Buffer:** they add clustered lights (VB-P1) + material binning
  (VB-P2) on the same pure-VB base. VB-P1/P2 close most of that gap.
- **vs Burns & Hunt (Intel 2013, the original VB paper):** their headline is material
  classification (VB-P2). We implement their VB *transport* but not their *classification* shading.
- **vs a VB→gbuffer hybrid (Nanite-lite / filmicworlds):** we are *ahead* — we do not materialize a
  G-buffer, so we keep the full bandwidth win they trade away.

---

## 6. Immediate decision — TV0 textures: über-resolve vs material-tiled

TV0 (textured materials under VB, in flight per [RENDER-PARITY-PLAN.md](RENDER-PARITY-PLAN.md)) adds
texture sampling to the **existing fused über-resolve**. This is pragmatic and correct, but it
*deepens* G2: the one über-shader now carries every material's texture-sampling paths, growing
register pressure / branch divergence.

**The "clean architecture the first time" fork (owner's principle):**
- **(a) Ship TV0 on the über-resolve now, do VB-P2 (material tiles) later.** Faster; textures work
  under VB immediately; VB-P2 later re-homes the sampling into per-material passes. Risk: TV0's
  über-sampling is thrown away by VB-P2.
- **(b) Do VB-P2 (material classification) FIRST, then land textures onto the classified pipeline.**
  The theoretically-clean path — textures land in per-material passes from day one, no über-sampling
  detour. Cost: VB-P2 is an **L** rung, so textures under VB slip behind it.

This is a VALUES/SCOPE call for the owner. Recommendation: if VB is the flagship perf path (owner's
stated priority) and material variety will grow, **(b)** honors "clean the first time"; if the goal
is "textures under VB working soon," **(a)** ships faster and VB-P2 subsumes it later without a
correctness regression (only wasted TV0 über-sampling work).

---

## References
- Burns & Hunt, *The Visibility Buffer: A Cache-Friendly Approach to Deferred Shading* (Intel 2013)
  — `info/Burns2013Visibility.pdf` (+ code zip). The material-classification foundation (VB-P2).
- Wihlidal, *Optimizing the Graphics Pipeline with Compute* (GDC 2016) / Nanite deep-dives —
  analytic barycentric derivatives (already ours) + GPU-driven raster (VB-P4).
- The Forge, *Triangle Visibility Buffer* — clustered lights (VB-P1) + material binning (VB-P2) on
  pure VB.
- filmicworlds VB→gbuffer notes — the hybrid we deliberately do NOT do (kept the bandwidth win).
