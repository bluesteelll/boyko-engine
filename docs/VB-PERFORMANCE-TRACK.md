# VisibilityBuffer performance track — honest assessment + optimization roadmap

**Status:** ASSESSMENT (written at rung R10, against VB v1) + ROADMAP — **three of the four roadmap
rungs have since SHIPPED**: VB-P1 (clustered light culling), VB-P2 (material classification), and
VB-P3 (the geo/shade split, landed under its plan-of-record name **R9**). **VB-P4** (GPU-driven
cluster culling / mesh shaders) is the only rung still unbuilt.

The v1 assessment prose is kept as written — the record of what was diagnosed and planned is the
point of this document — and every claim it made that has since been falsified is annotated **in
place** with the landing evidence and with whatever residual is genuinely still open. Anchors below
were re-derived against branch `feat/multi-paradigm-render` @ `819165b`; line numbers in this repo
drift, so each anchor names its symbol as well.

Companion to [MULTI-PARADIGM-RENDER-PLAN.md](MULTI-PARADIGM-RENDER-PLAN.md) (the VB path shipped
there, rungs R7/R-VBGEO/R8/R10) and [RENDER-PARITY-PLAN.md](RENDER-PARITY-PLAN.md) (textures + SDF
shadows). This doc answers the owner's question — *"is our VB implementation the most performant
and optimal?"* — honestly, and lays out the concrete work that would make it so.

---

## 0. Verdict (the honest one-liner)

*The verdict below is the R10-era one, kept verbatim; the paragraph after "What the v1 snapshot did
NOT have" carries the update.*

**The foundation is the right, bandwidth-optimal choice — which most engines get wrong — but the
current implementation is a v1 that deliberately omits several well-known SOTA optimizations. It is
NOT "the most performant/optimal" in absolute terms; it is "architecturally sound and
bandwidth-optimal at the top level."**

The single most valuable thing we did — **pure Visibility Buffer, no G-buffer materialization** —
is exactly the decision Nanite's "VB → thin-gbuffer" and the filmicworlds VB→gbuffer approaches
compromise on, throwing away roughly half the bandwidth win. We keep it. On top of that we have
analytic barycentric derivatives and bindless geometry re-fetch. Those are genuinely SOTA choices.

What the v1 snapshot did NOT have: clustered light culling, material classification, geo/shade
split, and a GPU-driven raster. Each is a concrete, well-understood optimization with a known perf
profile.

**Three of those four have since landed.** Clustered light culling (VB-P1), material classification
(VB-P2) and the geo/shade split (VB-P3 = rung R9) all ship today; the GPU-driven raster (VB-P4) is
the one genuine hole left. The §3 gap entries are kept rather than deleted — the diagnosis is what
motivated the work — each now annotated with what closed it, and with the part that did not.

---

## 1. VB architecture at the v1 snapshot (+ what changed since)

The path (R7 → R-VBGEO → R8 → R10), grounded in the source. Each bullet states the v1 shape it was
written against; the **Since:** lines carry today's truth.

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
  **Since:** the fused resolve is now one of **three** mutually-exclusive `lit` producers — the
  fused `vb_resolve`, the material-classified `vb_shade` (VB-P2), and the split `vb_geo` +
  `vb_shade_split` pair (R9 = VB-P3). `declare_vb_graph`
  (`crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:2986`) and `record_vb`
  (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:75`) select exactly one per frame; a frame
  that selects neither the classified nor the split arm declares neither, so it pays no tax for
  their existence.
- **Material (v1)** — flat per-instance lookup `Materials[pm.id]` (via the `PerInstanceMaterial`
  ring). No classification, no per-material shader.
  **Since VB-P2:** a `fill → count → scan → scatter` classify chain bins pixels by material id and
  `vb_shade` shades material-coherent groups against a **wave-uniform `mat_id`**
  (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:702` gates the chain,
  `:913` selects `vb_shade` over the fused resolve). The shading tail is still ONE shader body, by
  design (the plan's D3 byte-identity constraint) — "one `.spv` per material" was never this
  rung's deliverable; see §3 G2.
- **Lights (v1)** — the resolve did an **ALL-LIGHTS flat scan**: an L0a directional/sky loop
  (`for i < l0a_count`) + an L0b point/spot loop (`for j in l0a..light_count`), **no cluster/froxel
  lookup**.
  **Since VB-P1:** the L0b point/spot loop carries a `#ifdef FROXEL` arm
  (`crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl:340`, and the classified sibling
  `vb_shade.comp.hlsl:507`) that maps the pixel to its froxel and walks only the survivors
  `cluster_cull.hlsl` wrote into `ClusterGrid`/`LightIndexList`; the base compile and any unarmed
  frame keep the identical flat walk (`vb_resolve.comp.hlsl:388`). The L0a directional/sky loop is
  still ALL-LIGHTS (`vb_resolve.comp.hlsl:299-301`) and correctly so — directionals are not
  froxel-cullable. The **split** producer `vb_shade_split.comp.hlsl` has **no FROXEL arm at all**
  (`:473`, `:531`) — the open residual recorded in §3 G1.
- **Resolver flags (v1)** — `mesh_geo_shade_split == false` (fused only); `cap_vb_v1_consumers`
  forced SSAO/DDGI/denoise **OFF** under VB (`VbPreLightConsumersNotYetImplemented`).
  **Since R9 (= VB-P3) — and the change is NOT the one it looks like.** The flag was never
  hardcoded: at the very commit that wrote this document its derivation already read
  `matches!(path, RenderPath::VisibilityBuffer) && pre_light`
  (`git show 1025c7d:crates/boyko_render/src/render_path_config.rs`, line 698). It resolved `false`
  because `cap_vb_v1_consumers` runs BEFORE `resolve_rules` and zeroed all five members of the
  `pre_light` union (`ssao_on`, `ddgi_on`, `shadow_denoise_spatial_on`, `shadow_temporal_on`,
  `ssr_on` — plus `hwrt_denoise_or_vis_on` besides), so `pre_light` could not be `true` under VB.
  The v1 cap's own doc said as much: "`mesh_geo_shade_split` stays structurally `false` under VB
  today". Two distinct R9 changes, only one of which arms anything: **R9a** (`c9a3d0c`) added the
  `&& mesh_leg` term to the derivation
  (`crates/boyko_render/src/render_path_config.rs:883`; the field itself at `:510`) — a
  *narrowing* of the rule, `GeometryLegs::Sdf` has no `vb_raster` to split; **R9b/R9c/R9d**
  (`e5b951d` / `e9f4171` / `5cd285d`) rewrote `cap_vb_v1_consumers` (`:1135`, called by
  `resolve_render_path` on the line immediately above its `resolve_rules` call, `:1243`) from an
  unconditional zero into per-consumer rules — `ssr_on` is the one member still zeroed
  unconditionally. **The cap's narrowing is what arms the split** — a
  consumer that now survives the cap makes `pre_light` true, and the derivation (unchanged in
  shape since v1) follows. `cap_vb_v1_consumers` still exists; §3 G3 states exactly what it still
  caps. TAA is NOT capped (the TAA-under-VB rungs:
  VB×Mesh via `viewt_from_depth_rz`, the SDF-carrying legs via the `VIEWT`-variant
  `sdf_forward_march` gViewT composite; the former `VbTaaNotYetImplemented` variant is deleted).

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

## 3. Gaps vs SOTA (what kept it from "the most optimal")

Kept as the original diagnosis, each annotated with what closed it. Three of the four are closed.

### G1 — No clustered/tiled light culling (BIGGEST perf gap) — **CLOSED by VB-P1** (residual below)
The resolve shades **every pixel against every light** (`O(pixels × lights)`). SOTA VB (The Forge
Triangle Visibility Buffer, any clustered pipeline) bins lights into froxels/tiles and each pixel
touches only its cluster's lights. For many-light scenes this is a multiplicative loss.
- **Evidence (v1):** two flat all-lights loops in `vb_resolve.comp.hlsl`, no cluster lookup.
- **Perf impact:** linear in total light count instead of ~constant per-pixel; dominates on dense
  lighting.
- **CLOSED (VB-P1a…P1e, plus the P1j/P1k bound fixes):** the `#ifdef FROXEL` arm in
  `vb_resolve.comp.hlsl:340` / `vb_shade.comp.hlsl:507`, fed by `cluster_cull.hlsl` — **taken from
  the ForwardPlus L1 infra, but "verbatim" was the plan's word and it did not survive contact.**
  It needed a correctness fix *before* the seam could land, and has been rewritten substantially
  *since*:
  - **Before:** rung **VB-P1-0** (`ee94cbc`, an ancestor of P1a `78d0534`) — the cull filtered
    point/spot on the RAW kind word while the flat resolve applies no kind predicate at all over
    its `[l0a_count, light_count)` punctual block, and `light_table.hlsli` packs
    `LIGHT_FLAG_CASTS_SHADOW` at bit 16 and a 5-bit atlas slot at bits 17..21 ABOVE the 16-bit
    `LIGHT_KIND_MASK` tag — so a shadow-flagged or atlas-slotted punctual was silently dropped by
    the cull and kept by the flat path. Masking via `light_kind()` restored the froxel↔flat
    equality VB-P1 rests on. (P1a itself then touched the file not at all — that one commit *was*
    a verbatim reuse, of an already-repaired shader.)
  - **Since:** `git diff --stat 78d0534~1 HEAD -- crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl`
    is **+318/-5** over three commits: `7f18a63` (P1e H1.6) made `sq_dist_point_aabb` a written-out
    `precise` sum instead of `dot()`, pinning it against FMA contraction — that one is in the BASE
    arm's own light test; `2903469` (P1e H2) added the `#ifdef HIER` hierarchical arm (+263);
    `c61e87b` (VB-P1j) replaced the base arm's write bound `dim_x*dim_y*dim_z` with
    `min(…, GetDimensions)` → SPIR-V `OpArrayLength` on the bound descriptor.
  - **Result:** one source, **two** committed modules — `cluster_cull.comp.spv` and
    `cluster_cull_hier.comp.spv`, exposed as `cluster_cull_spirv()`
    (`crates/boyko_rhi_vulkan/src/compute.rs:1627`) and `cluster_cull_hier_spirv()` (`:1635`); both
    rows in `docs/SHADER-VARIANT-MANIFEST.md`'s `## cluster_cull.hlsl` section.

  Armed by the single boot-frozen bit `ResolvedRenderPath::froxel_light_cull`
  (`crates/boyko_render/src/render_path_config.rs:942`, field at `:537`), which the app boot threads
  from the real `LightingConfig::clusters_enabled` toggle
  (`crates/boyko_app/src/runner.rs:531` → the arm test at `:697` → the build at `:755` →
  `GpuSceneBundles::build_froxel_light_cull`, `crates/boyko_app/src/gpu_scene/mod.rs:4299`).
  MEASURED, not asserted: the VB-P1d GPU-timestamp bench
  (`crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`) put the flat-vs-froxel break-even at ≈103
  point/spot lights, which is what the banded `CLUSTER_LO = 64` / `CLUSTER_HI = 128` auto-selector
  is built from (`crates/boyko_render/src/light_policy.rs:77,87` — read the reproducibility caveat
  in `CLUSTER_LO`'s own doc before re-tuning; that re-tune is the open VB-P1f). VB-P1e then made the
  hierarchical cull the default arm at 22.5x the base cull at N=512 and 1.4x FASTER at N=8
  (`crates/boyko_app/src/runner.rs:718-736`).
- **Residual, genuinely open:** the froxel arm exists for the FUSED producers only
  (`vb_resolve_froxel`, `vb_shade_froxel`, `vb_shade_tex_froxel`). The R9 **split** producer
  `vb_shade_split.comp.hlsl` still runs the flat all-lights scan (`:473`, `:531`) and has no FROXEL
  variant in `docs/SHADER-VARIANT-MANIFEST.md:154-159`. So a VB frame with a pre-light consumer
  armed (SSAO/DDGI/hwrt-shadow → the split arm) gets **no** light culling. Also: `froxel_light_cull`
  is boot-frozen, so `ClusterSelectMode::Auto` alone does not arm the machinery — the per-frame
  policy (`select_lighting_cull`, registered per-frame at
  `crates/boyko_render/src/light_plugin.rs:121`) can only flip the shader's runtime `use_clusters`
  bit inside a boot that already read `clusters_enabled == true`.

### G2 — No material classification / material-tiled shading — **CLOSED by VB-P2** (scope note below)
The classic VB optimization (Burns & Hunt, Intel 2013; The Forge) **classifies pixels by material**
and shades each material in its own pass, so a pixel runs only its material's shader (no über-shader
carrying every material's texture-sampling + branch paths). We use **one über-resolve** with runtime
`Materials[pm.id]` params.
- **Evidence (v1):** flat `Materials[pm.id]` lookup; one resolve `.spv`.
- **Perf impact:** über-shader occupancy/register pressure + branch divergence across materials in a
  warp; worsens as material variety + texture-sampling paths grow.
- **CLOSED (VB-P2 rungs P2a/P2b/P2c, plus VB-P1c for the froxel×classified cross-product):** the
  full-screen per-material bin chain ships — `vb_classify_count` / `_scan` / `_scatter`
  (`crates/boyko_rhi_vulkan/shaders/vb_classify_{count,scan,scatter}.comp.hlsl` + their committed
  `.spv`), built by `GpuSceneBundles::build_vb_classify_pipelines`
  (`crates/boyko_app/src/gpu_scene/mod.rs:3950`), recorded as the `fill → count → scan → scatter`
  chain in `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:702`ff., feeding the
  `vb_shade`/`vb_shade_tex` producer at `:913` via the regular over-dispatch
  `dispatch_group_count_x + vb_classify_material_count` (`:1072`, the plan's D2 — the FFI has no
  `vkCmdDispatchIndirect`). Selector: `vb_use_classified`
  (`crates/boyko_app/src/gpu_scene/mod.rs:5230`) = `BOYKO_VB_FORCE_CLASSIFIED` OR a frame that
  actually bound textured materials — the owner's P1-4 decision to keep the fused resolve for flat
  frames, where classification is a small net loss.
- **Scope note (not a residual — a deliberate design choice):** this is material-**grouped**
  shading, not N per-material `.spv`. The win taken is wave-uniform `mat_id` (each material samples
  only its own bindless maps through a UNIFORM index — no `NonUniformResourceIndex`, no intra-wave
  sampler divergence). Splitting the shading tail into per-material permutations remains
  unexplored, and nothing measured says it would pay.
- **Not done:** the plan's P2c deliverable "measure the classify tax on RTX (fused vs classified @
  M ∈ {1,5})" left no measurement artifact in the tree. The selector threshold therefore rests on
  the P1-4 *estimate* (~0.3 ms), not on a number — the exact failure mode the VB-SV0 revert was
  about.

### G3 — Fused, not geo/shade split — **CLOSED by rung R9 (= VB-P3)** (residual below)
`mesh_geo_shade_split == false` — geometry fetch + shade are one dispatch. The split form (geo
pre-tail → a thin `sdf_surface_cache`-style surface cache → shade post-tail) is what unlocks
thin-aux consumers (SSAO/TAA/DDGI) *and* changes the perf profile (shade once from a cached surface
instead of re-fetching under overdraw-free VB).
- **Evidence (v1):** `mesh_geo_shade_split` was already DERIVED at v1 —
  `matches!(path, RenderPath::VisibilityBuffer) && pre_light` (`1025c7d:render_path_config.rs:698`
  — the same shape today, modulo R9a's added `&& mesh_leg`) — but structurally always `false`,
  because `cap_vb_v1_consumers` ran
  first and zeroed every pre-light consumer, so `pre_light` could not be `true` under VB. The
  unconditional part lived in the CAP, never in the flag's derivation; §1's resolver-flags bullet
  has the full account, including which R9 rung changed which of the two.
- **Impact:** SSAO/TAA/DDGI were structurally impossible under VB v1 (feature gap); no
  surface-cache reuse.
- **CLOSED (R9a…R9d):** `vb_geo.comp.hlsl` (+ its `-D MOTION` sibling) and
  `vb_shade_split.comp.hlsl` (+ `_tex` / `_hwrt` / `_tex_hwrt`) ship — manifest rows at
  `docs/SHADER-VARIANT-MANIFEST.md:154-159` and `:166-175`. The resolver derives the arm
  (`crates/boyko_render/src/render_path_config.rs:883`) and `cap_vb_v1_consumers` was narrowed from
  an unconditional zero to per-consumer rules (`:1150` SSAO passes through on any mesh-carrying leg
  set, `:1155` DDGI on VB×Both only, `:1160-1163` the two denoise stages only with the hwrt carrier
  present). Golden: `[vb_mesh_ssao]` (`goldens/PINS.toml:705`, `85625a11…`).
- **Residual, genuinely open:** (a) SSR is still capped unconditionally — no `SsrConfig` exists
  engine-wide (`render_path_config.rs:1171`, and the boot call site threads a literal `ssr_on:
  false` at `crates/boyko_app/src/runner.rs:514`); (b) every pre-light consumer is still zeroed
  under `VB × Sdf` (no mesh raster to split — the R-SDFSPLIT boundary, `render_path_config.rs:1150`);
  (c) the split displaces classification rather than composing with it (R9 §0), so a frame cannot
  have both VB-P2's material coherence and VB-P3's thin aux; (d) split ⇒ no light culling, per G1's
  residual. The "surface cache" half of the original G3 wording did not land either — `vb_geo`
  writes thin aux for the consumers, it does not cache a shading surface for reuse.

### G4 — Standard raster, not GPU-driven / mesh-shader cluster culling — **STILL OPEN**
The id-raster is a CPU-recorded `vkCmdDrawIndexed` per batch — not the GPU-driven
cluster-cull + mesh-shader pipeline that makes Nanite fast on high-poly scenes. Per-pixel vertex
re-fetch can also be cache-unfriendly on high-poly meshes.
- **Evidence:** the `record_vb` draw loop (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:75`);
  single-vendor (RTX), `feature = hwrt` gated off by default. Still true today: no `VK_EXT_mesh_shader`
  and no meshlet data structure exists anywhere in `crates/`.
- **Impact:** no fine-grained culling; re-fetch cache misses scale with triangle density.
- **In research, not built:** `docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md` +
  `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md`.

---

## 4. Optimization roadmap (prioritized, with reuse targets)

Each rung: golden byte-identity gate (VB base `vb_mesh f4719cbf` unchanged when the optimization is
off), criterion + GPU-capture benchmark proving the win, dev → code-reviewer → orchestrator verify,
author-only commit+push. Ordered by **perf-win-per-effort**.

*Anchor check (re-verified this revision, not assumed): `[vb_mesh]`'s pin is still
`f4719cbf13da5badb7a659d572d1817bbc45db683e5f0311f9bed8c933913ea1` —
`goldens/PINS.toml:263` (section) / `:292` (`sha256_software`). The three rungs below all landed
with that anchor unmoved.*

| Rung | What | Why (perf) | How / reuse — and what actually landed | Size |
|---|---|---|---|---|
| **VB-P1 — SHIPPED** (P1a…P1e, + the P1j/P1k bound fixes) Clustered light culling in `vb_resolve` | Replace the all-lights flat scan with a froxel/cluster lookup | Killed G1 for the fused producers; `O(pixels × lights)` → ~`O(pixels × lights_per_cluster)` above the measured break-even | **Plan:** reuse `cluster_cull.hlsl` + the ForwardPlus `#ifdef FROXEL` infra verbatim; add the same seam to `vb_resolve`; the app side must arm the cluster buffers (at the time of writing, `scene()` hardcoded `cluster_* None`). **Landed:** that seam (`shaders/vb_resolve.comp.hlsl:340`, `shaders/vb_shade.comp.hlsl:507`) plus `vb_layout0_froxel` and the three `_froxel` pipelines — but **the "verbatim" half of the plan did not hold**: rung VB-P1-0 (`ee94cbc`) had to fix the cull's unmasked kind-word compare before the seam could land, and `cluster_cull.hlsl` is +318/-5 today across the `precise` pin, the `#ifdef HIER` arm and the `OpArrayLength` write bound (§3 G1 has the per-commit breakdown). The app-side gap is CLOSED — `froxel_light_cull` resolves from a real toggle (`crates/boyko_render/src/render_path_config.rs:942`, field `:537`; consumer field `clusters_wanted` at `:662`) which `crates/boyko_app/src/runner.rs:531` reads from `LightingConfig::clusters_enabled` and `:697`/`:755` uses to drive `GpuSceneBundles::build_froxel_light_cull` (`crates/boyko_app/src/gpu_scene/mod.rs:4299`). It went well past the planned P1a/P1b pair: **P1c** the textured×froxel cross-product (`vb_shade_tex_froxel` + `vb_set0_tex_froxel`), **P1d** the GPU-timestamp bench + the `CLUSTER_LO=64`/`CLUSTER_HI=128` band (`crates/boyko_render/src/light_policy.rs:77,87`), **P1e** the hierarchical cull, now the DEFAULT arm (22.5x the base cull at N=512, 1.4x faster at N=8; `crates/boyko_app/src/runner.rs:718-736`), **P1j/P1k** the `ClusterGrid` write/read bound fixes. Goldens: `[vb_mesh_froxel]` `fb220ff3…` (`goldens/PINS.toml:728`/`:758`), `[vb_mesh_tex_froxel]` `6d7ea00d…` (`:767`/`:800`). **Still open:** no FROXEL variant of the R9 split producer (§3 G1 residual); `CLUSTER_LO`/`CLUSTER_HI` re-tune under a repeated-run protocol = **VB-P1f** (owner-gated); **VB-P1g/h/i** (fixed-cost attack, second in-group level, wave intrinsics) named but unbuilt — all four tracked in `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §11. | **L** |
| **VB-P2 — SHIPPED** (P2a/P2b/P2c) Material classification | Classify pixels by material id → material-coherent shade groups | Killed G2 — the shade dispatch now runs against a wave-uniform `mat_id`; this is what lets each textured material sample its own bindless maps through a UNIFORM index | **Plan:** a new material-classify compute (histogram/prefix-sum over `vb_id → mat_id`) + indirect per-material dispatch; Burns & Hunt (Intel 2013, `info/Burns2013Visibility.pdf`) the reference. **Landed:** exactly that chain, minus the indirection — `vb_classify_count`/`_scan`/`_scatter` (`crates/boyko_rhi_vulkan/shaders/vb_classify_{count,scan,scatter}.comp.hlsl`, committed `.spv`), built by `GpuSceneBundles::build_vb_classify_pipelines` (`crates/boyko_app/src/gpu_scene/mod.rs:3950`), recorded as `fill → count → scan → scatter` at `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:702`ff. and consumed by the `vb_shade` producer at `:913`. `vkCmdDispatchIndirect` is NOT in the FFI, so it is a regular over-dispatch of `G + present_material_count` groups with a SENTINEL early-out (`:1072`, plan D2). Producer selection: `vb_use_classified` (`crates/boyko_app/src/gpu_scene/mod.rs:5230`) — textured frames, or `BOYKO_VB_FORCE_CLASSIFIED`; flat frames keep the fused resolve by the owner's P1-4 call. Plan: `docs/VB-P2-CLASSIFICATION-PLAN.md`. **Not delivered:** the P2c on-RTX classify-tax measurement (fused vs classified @ M ∈ {1,5}) left no artifact — the selector threshold still rests on an estimate. Material-**grouped**, not per-material `.spv` permutations (deliberate; see §3 G2). | **L** |
| **VB-P3 — SHIPPED as rung R9** (R9a…R9d) Geo/shade split | Split fused resolve → geo pre-pass + shade | Killed G3 — SSAO/DDGI/hwrt-shadow now resolve under VB (`cap_vb_v1_consumers` narrowed, not lifted) | **Plan:** the resolver already modelled it (`mesh_geo_shade_split`, `sdf_surface_cache`); it was the multi-paradigm plan's deferred R9 rung. **Landed:** `vb_geo.comp.hlsl` (+ `-D MOTION`) and `vb_shade_split.comp.hlsl` (+ `_tex`/`_hwrt`/`_tex_hwrt`) — `docs/SHADER-VARIANT-MANIFEST.md:154-159`, `:166-175`; resolver derivation at `crates/boyko_render/src/render_path_config.rs:883`; the narrowed cap at `:1135`, `:1150-1166`. Plan: `docs/R9-VB-SPLIT-PLAN.md`. Golden `[vb_mesh_ssao]` `85625a11…` (`goldens/PINS.toml:705`/`:719`). **Still open:** SSR stays capped engine-wide (no `SsrConfig`); pre-light consumers stay zeroed under `VB × Sdf` (R-SDFSPLIT's boundary); split and classification **displace** each other rather than composing; the split arm has no froxel cull; and the "surface cache" the row promised was not built — `vb_geo` writes thin aux, it does not cache a shading surface. | **L** |
| **VB-P4 — OPEN** GPU-driven cluster culling / mesh shaders | Replace CPU `vkCmdDrawIndexed` with GPU cluster-cull + mesh-shader raster | Kills G4 — fine-grained culling + Nanite-class high-poly scaling | Largest; needs a cluster builder + `VK_EXT_mesh_shader` + GPU-driven indirect. New subsystem, not a shader tweak. Nothing built: no mesh-shader extension and no meshlet structure exists in `crates/`. Research/design in flight — `docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`, `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md`. | **XL** |

**Recommended order (as planned):** VB-P1 first (best win/effort, near-total infra reuse), then
VB-P2 (the material-classification optimum, especially valuable now that textures are landing), then
VB-P3 (features + surface cache), then VB-P4 (the big one) only if high-poly Nanite-class scaling is
a real target for this engine.

**What actually happened:** the order was inverted at the front — **VB-P2 landed first** (P2a
`8e3d268` → P2b `1bfadc8` → P2c `fc9f14a`, deliberately *before* textures, so TV0 could land onto
the classified pipeline rather than onto an über-resolve — see §6), then **VB-P3 as R9** (`c9a3d0c`
→ `5cd285d`), then **VB-P1** (`78d0534` → `d60d95b` → `b2b1240` → `e7a4767`, and the P1e
hierarchical-cull sub-campaign). VB-P4 remains the only unstarted rung, and the two residual
threads worth naming are **the split arm's missing froxel cull** and **split-vs-classified
composition** — neither is a new rung so much as the seam where the three shipped rungs meet.

---

## 5. Positioning vs the references

Updated to what ships today; the v1 reading of each line is preserved in the parenthetical.

- **vs Nanite:** we share pure-VB + analytic barycentrics + bindless re-fetch, and we now also have
  material grouping (VB-P2). What Nanite still has and we do not is the **GPU-driven cluster cull +
  mesh-shader raster** (VB-P4). We match its *bandwidth* discipline and part of its
  *classification* discipline; we do not match its *culling* pipeline. *(v1 read: neither
  classification nor culling.)*
- **vs The Forge Triangle Visibility Buffer:** they add clustered lights + material binning on the
  same pure-VB base. **Both have landed here** (VB-P1, VB-P2) — with one asymmetry they do not
  have: our light cull is wired to the fused/classified producers but not to the R9 split producer.
- **vs Burns & Hunt (Intel 2013, the original VB paper):** their headline is material
  classification. **We implement both their VB transport and their classification** — the
  `count → scan → scatter` bin chain feeding a material-coherent shade dispatch (VB-P2). The one
  divergence from the paper is that we group into a **wave-uniform `mat_id`** shading one shader
  body, rather than dispatching a distinct per-material shader; and the dispatch is a bounded
  over-dispatch rather than indirect, because `vkCmdDispatchIndirect` is not in our FFI. *(This
  line previously read "we implement their VB transport but not their classification shading" —
  false since VB-P2 rung P2c, `fc9f14a`.)*
- **vs a VB→gbuffer hybrid (Nanite-lite / filmicworlds):** we are *ahead* — we do not materialize a
  G-buffer, so we keep the full bandwidth win they trade away.

---

## 6. Immediate decision — TV0 textures: über-resolve vs material-tiled — **RESOLVED: (b)**

**Outcome (kept here because the fork itself is the record):** the owner took **(b)**. VB-P2 landed
first (P2c `fc9f14a`) and TV0 landed onto the classified pipeline afterwards (`b7efbeb`) — which is
why `vb_use_classified` is defined as "forced, OR this frame actually bound a textured material"
(`crates/boyko_app/src/gpu_scene/mod.rs:5230`) and why the textured VB producers are
`vb_shade_tex` / `vb_shade_tex_froxel`, not a textured fused resolve (`vb_resolve.comp.hlsl` has no
TEXTURED variant at all — `docs/SHADER-VARIANT-MANIFEST.md:124-129`). No über-sampling detour was
built and none was thrown away. The original fork text follows.

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
