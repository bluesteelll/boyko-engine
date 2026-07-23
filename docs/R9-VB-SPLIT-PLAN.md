# R9 â€” VisibilityBuffer geo/shade split (vb_geo + vb_shade_split), SSAO/DDGI/shadow-temporal under VB â€” REV 2

Architect design (critic-revised), HEAD `7421675`, branch `feat/multi-paradigm-render`. Implements rung R9 of
`docs/MULTI-PARADIGM-RENDER-PLAN.md` (Â§B VB framegraph 254-330, Â§D thin-aux 367-389, Â§G budgets 441-475, R9 row 495,
Rev-5 573-602). Scope per orchestrator leanings: MESH split only; `sdf_geo_shade_split` (R-SDFSPLIT-under-VB) stays
out; SSR = ROUGHNESS lane shape only; shipped shader files are never renamed.

## 0. Naming and composition ground rules

- The **existing** `vb_shade.comp.hlsl` / `vb_shade_tex` (VB-P2 classification lit-producer) keeps its name. The R9
  split passes are **`vb_geo`** and **`vb_shade_split`** everywhere (graph labels, plan fields, Rust idents, shader
  files `vb_geo.comp.hlsl` / `vb_shade_split.comp.hlsl`). Disambiguation notes go into
  `docs/VB-P2-CLASSIFICATION-PLAN.md` and `declare_vb_graph`'s doc: "`vb_shade` = classification lit-producer
  (fused-mode perf feature); `vb_shade_split` = the R9 pre-light split shade".
  *Rejected:* renaming the shipped classification `vb_shade` â€” OpSource/.spv churn on a frozen, pinned blob.
- **Split displaces classification** (v1): when `mesh_geo_shade_split` is armed the classify chain
  (`fill/count/scan/scatter`) is **not declared** and `vb_use_classified` is **not consulted**; textured frames keep
  textures via the per-frame `vb_tex_active()` choice between `vb_shade_split` and `vb_shade_split_tex` (mirrors the
  fused `vb_resolve`/`vb_shade_tex` selection: boot-frozen split arming, per-frame base/_tex pick).
  *Rejected:* classified-split composition â€” a later perf rung; correctness first.
- **Fused/split gating is SURGICAL inside the existing `if mesh_leg` block (graph_bridge.rs:3059)** â€” critic-fix:
  `vb_raster` stays gated on **bare `mesh_leg`** (BOTH arms consume the `vb_id`/`vb_depth` it produces; disarming it
  under split would leave `vb_geo` reading an unwritten image â€” authoring-guard panic). INSIDE the block:
  - the classify chain (`use_classified` read) AND the `vb_resolve`/`vb_shade` lit-producer selection re-gate on
    `!scene.resolved_render_path.mesh_geo_shade_split` (this is `path_vb_fused()` minus the raster);
  - `vb_geo` + `vb_shade_split` gate on `path_vb_split()`.
  Structural test (R9b): a split frame has `vb_raster` declared AND recorded, and NO classify pass declared.
  *Rejected:* re-gating the whole `mesh_leg` block â€” collapses the split path (no vb_id producer).

## 1. Resolver / config (`crates/boyko_render/src/render_path_config.rs`)

1. **`resolve_rules` line 723:** `mesh_geo_shade_split = matches!(path, VisibilityBuffer) && mesh_leg && pre_light`.
   The `&& mesh_leg` is NEW (the R-SDFFWD "mesh_leg gates the prepass" precedent at 707-716: under `GeometryLegs::Sdf`
   there is no mesh raster to split). Plan-doc errata line + fn-doc update + truth-table tests.
2. **thin-aux NORMAL arming (line 735) â€” critic-fix (Temporal-only contradiction, option (a): the mask stays the
   single truth):** the NORMAL union gains `|| (consumers.hwrt_denoise_or_vis_on && !matches!(path, RenderPath::Deferred))`
   â€” the `shadow_vis` gather (graph_bridge.rs:1468) READS a normal and is armed by `hwrt_denoise_or_vis_on`; under
   Deferred it reads the fat `gNormal` (no thin image, arming unchanged there â€” Deferred truth-table rows and pins
   untouched), under non-Deferred paths its normal source IS `thin_normal`, so the vis pass is a NORMAL consumer
   there. Consequences, propagated everywhere:
   - **Temporal-only under VB â‡’ `thin_aux == NORMAL|MOTION`** (NOT MOTION-only). The Rev-5 rung-row label
     "(MOTION-only arming)" gets a plan-doc erratum alongside the `mesh_leg` one; the Â§7 truth-table row, the Â§3
     declare list, and the plan-doc ownership table (MULTI-PARADIGM-RENDER-PLAN.md:306) are updated IN R9a, before
     the truth-table tests land.
   - Resolver `debug_assert!(consumers.shadow_temporal_on implies consumers.hwrt_denoise_or_vis_on)` documenting the
     cfg(hwrt) coupling (temporal filters the hwrt vis output; no temporal-without-vis config exists).
   - **Invariant + test: `mesh_geo_shade_split â‡’ thin_aux.contains(NORMAL)`** â€” every pre-light consumer now arms
     NORMAL under non-Deferred, so `vb_geo`'s `thin_normal` write is unconditional under split and mask/structure
     agree by construction.
   *Rejected:* option (b) "thin_normal is split-structural, written independent of thin_aux.NORMAL" â€” leaves the mask
   lying about the image set; (a) keeps one truth and costs one union term.
3. **`cap_vb_v1_consumers` (line ~938):** narrowed per stage, **NOT deleted** (revised from the original plan):
   - R9b: stops zeroing `ssao_on` **when `mesh_leg`**; keeps zeroing it under `VB && !mesh_leg` (VBÃ—Sdf â€” no split
     shade exists to consume it until R-SDFSPLIT).
   - R9c: stops zeroing `ddgi_on` when `mesh_leg && sdf_leg` (VBÃ—Both â€” the only reachable config: gpu_scene ANDs
     `ddgi_enabled` with `sdf_leg`, and consumption lives in the mesh split shade); keeps zeroing otherwise
     (VBÃ—Mesh ddgi_on would arm a split whose DDGI is structurally dead; VBÃ—Sdf as above).
   - R9d: stops zeroing `shadow_denoise_spatial_on`/`shadow_temporal_on`/`hwrt_denoise_or_vis_on` when `mesh_leg`.
   - Final form: the fn survives, zeroing ALL pre-light consumers under `VB && !mesh_leg` only; the
     `VbPreLightConsumersNotYetImplemented` degrade variant keeps its name (no enum churn), its doc re-scoped to the
     VBÃ—Sdf residual + a pointer to R-SDFSPLIT. Tests updated each stage.
   *Rejected:* full deletion at R9d â€” VBÃ—Sdf pre-light configs would arm resolver flags nothing consumes (a silent
   lie vs an explicit degrade log). *Rejected:* one-shot narrowing â€” each consumer needs its producer chain landed.
4. **Boot-freeze warn-once guard (R9 is the first structural `thin_aux` consumer under VB; critic-fix: the freeze
   must cover the light-header word, not just scene() inputs):**
   - New tiny POD Resource **`RenderPathFrozenConsumers`** (boyko_render; the boot `RenderPathConsumers` snapshot +
     the resolved path discriminant), inserted ONCE by the runner at boot (runner.rs:~429, beside
     `resolved_render_path`). Principle-0 clean: plain POD Resource, no side store.
   - **One clamp, upstream of BOTH consumers:** where the runner reads `SsaoConfig â†’ ResolvedSsao` per frame
     (runner.rs:1767 â†’ gpu_scene/mod.rs:4561 threading), under `path != Deferred` clamp the effective SSAO (and the
     ddgi/shadow-denoise toggles) to the frozen snapshot BEFORE fan-out, `log::warn!` ONCE via an `AtomicBool` on the
     host ("pre-light consumer set is frozen under RenderPath::VisibilityBuffer; rebuild to change").
   - **`sync_ssao_light_gate` becomes freeze-aware:** it bridges SsaoConfig into `LightingConfig::ssao_mode`
     per-frame, path-blind, explicitly "independent of this fn" (gpu_scene/mod.rs:4198-4212) â€” without this,
     a split-armed-without-SSAO boot (Temporal-only R9d, DDGI-only R9c) + runtime SsaoConfig::High keeps the gather
     disarmed but sets `ssao_mode=1`, and `vb_shade_split`'s unconditional gSsao read COMBINES seeded-UNDEFINED
     garbage through the Filament decoupling â€” visible corruption, zero validation errors. Fix: the sync system
     `try_resource`s `RenderPathFrozenConsumers`; when present and `path != Deferred`, the effective SsaoConfig is
     the frozen one. Gather arming and header word move in lock-step again by construction (one truth, two readers).
   - Deferred: untouched (live toggle stays free, snapshot ignored).
   - Unit tests: (a) boot ssao=High, runtime â†’Off â‡’ scene still arms High + one warn; (b) **split-armed Temporal-only
     boot + runtime SsaoConfig::High â‡’ `ssao_mode` word stays 0, gather stays disarmed, one warn.**
   *Rejected:* recorder-side ssao_mode force â€” the header upload lives in the collect_lights pipeline which never
   sees `GBufferScene`; the Resource clamp keeps the layering clean.

## 2. gViewT producers + O1 predicates (each read at BOTH declare and record, parity `debug_assert!`)

All on `GBufferScene` (crates/boyko_rhi_vulkan/src/present/scene_types.rs), from the threaded
`ResolvedRenderPathGpu` (scene_types.rs:1076) + activation fields:

- `path_vb_split()` = `path_is_vb() && resolved_render_path.mesh_geo_shade_split` â€” gates the `vb_geo` +
  `vb_shade_split` pair (they arm/disarm together).
- `path_vb_fused()` = `path_is_vb() && resolved_render_path.mesh_leg && !resolved_render_path.mesh_geo_shade_split`
  â€” gates the classify chain + `vb_resolve`/`vb_shade` selection ONLY (NOT `vb_raster`; Â§0).
- `path_vb_ssao()` = `resolved_render_path.mesh_geo_shade_split && scene.ssao.is_some()` (critic-hardened: anchored
  to the boot-frozen split flag, which the resolver sets only under VB â€” the gather can only arm when its producer
  `vb_geo` is boot-armed, correct even if the Â§1 clamp ever regresses; `debug_assert!(path_is_vb())` inside).
  Gates the VB SSAO gather + the Ã -trous chain.
- `path_vb_ddgi()` = `path_is_vb() && resolved_render_path.mesh_geo_shade_split && scene.ddgi_update.is_some()` â€”
  gates `ddgi_update` in the VB graph AND the shade's conditional atlas reads (R9c; `ddgi_update` arming already
  carries the `sdf_leg` AND from gpu_scene, so this is reachable only VBÃ—Both).
- **gViewT producer widening â€” critic-fix (CRITICAL): `path_sdf_forward_writes_viewt()` is NOT touched.** It stays
  `path_has_sdf_forward() && taa.is_some()` (scene_types.rs:2550). Widening it was path-blind: `scene.ssao` is armed
  per-frame from ResolvedSsao with NO path gating, so SsaoConfig under ForwardÃ—Both would (a) fire the Forward
  declarator's tripwire `debug_assert!(!scene.path_sdf_forward_writes_viewt())` (graph_bridge.rs:2322-2326) every
  debug frame and (b) in release select the VIEWT marcher variant storage-writing a gViewT lane the Forward path
  never allocates â€” NULL-image write. Also the original justification was wrong: viewt is a per-FIF ring re-produced
  every frame; a marcher viewt write under VBÃ—Both+SSAO-no-TAA has zero consumers.
- **ONLY the `vb_viewt` arming widens**, VB-scoped and anchored to the boot-frozen split flag
  (gpu_scene/mod.rs:4474):
  `vb_viewt_armed = path_is_vb && mesh_leg && ((taa && !sdf_leg) || (mesh_geo_shade_split && ssao))`
  â€” one named fn, read by gpu_scene arming and asserted at the graph seam. Position in the declare order: PRE-TAIL
  (before `vb_geo`) when the ssao arm is the reason; the current late slot for taa-only â€” one `ssao.is_some()`
  predicate picks the slot at both declare and record.
  Config truth: VBÃ—Mesh+(taa|ssao) â‡’ vb_viewt only. VBÃ—Both+taa-no-ssao â‡’ marcher only (shipped, pins stable).
  VBÃ—Both+ssao-no-taa â‡’ vb_viewt only (pre-tail; marcher predicate stays taa-only â‡’ off). VBÃ—Both+ssao+taa â‡’ BOTH:
  vb_viewt pre-tail feeds SSAO with mesh t (SDF-covered pixels read the 1e30 sentinel â‡’ background-masked), the
  VIEWT marcher overwrites at composite as the declared LAST gViewT writer (declared order derives the WAW barrier â€”
  the Rev-5 motion_vec WAW precedent) and taa_resolve sees exactly the shipped marcher output.
- **BOTH viewt asserts revised as one unit (critic-fix â€” the original plan missed the second, unconditional one):**
  - graph_bridge.rs:3362-3366 (unconditional mutual exclusion) becomes: dual arming is legal ONLY when the VB SSAO
    pre-tail lane is armed â€”
    `debug_assert!(!(vb_viewt_armed && marcher_viewt) || (scene.ssao.is_some() && resolved.mesh_geo_shade_split))`.
  - graph_bridge.rs:3397-3402 (TAA XOR): the strict XOR is KEPT verbatim for `scene.ssao.is_none()` configs
    (preserving the 7 VB TAA pins' exact barrier schedule); when ssao is armed it degrades to
    `taa â‡’ at least one producer armed` AND `sdf-carrying leg â‡’ the marcher is armed (the LAST declared writer)`.
  - Both revised forms join the W1 predicate list with a unit test per arm (4 configs: Mesh+taa, Both+taa,
    Both+ssao, Both+ssao+taa).
- SSAO under VBÃ—Both is **mesh-pixels-only this rung** (SDF pixels see the sentinel; the fused `sdf_forward_march`
  neither reads gSsao nor changes) â€” documented scope cut, closed by R-SDFSPLIT.
- `viewt[fi]` ring ALLOCATION gate (targets.rs) widens in lock-step with the arming: allocate iff
  `taa || (mesh_geo_shade_split && frozen_consumers.ssao_on)` (boot-stable: split is boot-frozen and runtime SSAO is
  freeze-clamped under VB, so the boot snapshot decides; no wasted ring under Temporal-only).

## 3. Framegraph (`declare_vb_graph`, graph_bridge.rs:2905)

**Images.** Append AFTER ResId 7 (`taa_hist_read`) with the **deferred "hwrt ids declared LAST" precedent**
(graph_bridge.rs:846-849 â€” cfg-gated ids must sit at the tail so both builds agree on every shared id and the buffer
block lands at `VB_IMAGE_COUNT` by construction):

- Software-build ids (R9b): `thin_normal=8`, `ssao=9`, `ssao_ring_a=10`, `ssao_ring_b=11`.
- R9c appends: `ddgi_irr=12`, `ddgi_depth=13`.
- R9d appends the cfg(hwrt) tail LAST: `motion_vec`, then the shadow chain (`shadow_vis` ping/pong, temporal history
  parity pair) â€” copying the deferred declarator's relative order and access shapes verbatim, incl. the seeded
  cross-frame history parity pair.
- `VB_IMAGE_COUNT`: 8â†’12 (R9b) â†’14 (R9c) â†’14+hwrt-tail (R9d; cfg-dependent constant, the deferred
  FRAMEGRAPH_IMAGE_COUNT 13/11 pattern). `debug_assert_eq!(last.index()+1, VB_IMAGE_COUNT)` updated each stage.
- **Seeding (critic-fix â€” "all plain add_image" was wrong for exactly one image):**
  - `ssao` = **`add_image_seeded("ssao", ResSync::undefined())`** â€” the deferred line 807 pattern VERBATIM, and for
    the same reason: `vb_shade_split`'s gSsao read is UNCONDITIONAL under split, so on a split-without-SSAO frame
    (DDGI-only R9c, Temporal-only R9d) no pass writes it and a plain add_image trips framegraph `compile()`'s
    unwritten-transient-read authoring guard (framegraph/graph.rs:301-317). The seed derives the discard-legal
    UNDEFINEDâ†’GENERAL first-touch on OFF frames; with SSAO armed the gather's write is the first touch and the seed
    is inert â€” armed-frame barriers unchanged.
  - `ddgi_irr`/`ddgi_depth` = `add_image_seeded` with
    `ResSync::seeded_readers_at_layout(SHADER_READ_ONLY_OPTIMAL, COMPUTE, SHADER_READ)` â€” copy deferred 830-845
    verbatim incl. the content-preserving-layout rationale (persistent round-robin accumulators; UNDEFINED would
    license discard of un-updated tiles) and `SubRange::color_layers(DDGI_ATLAS_LAYERS)` on every access.
  - `thin_normal`, `motion_vec`, `ssao_ring_a/b`, hwrt shadow ids: plain `add_image` (fresh first-touch every frame;
    the rings are never read without their producer chain) â€” except the temporal history parity pair, which copies
    the deferred seeded cross-frame form.
- Unarmed slots resolve to `VkImage::NULL` in `VbBarrierSink` (NULL-inert, taa_hist precedent) â€” with split off NO
  pass names any new id â‡’ zero barriers â‡’ existing pins byte-identical by construction (seeded ResIds named by
  nothing route no barrier â€” the deferred DDGI-off proof).

**Buffers (R9c; critic-fix â€” previously unenumerated):** after `gclassify=2` append
`ddgi_classification=3`, `ddgi_ray_table=4` as **`add_buffer_seeded`**, copying deferred 1009-1014 verbatim.
`VbBarrierSink::buffers` grows to 5.

**Pass order (split arm; fused arm untouched token-for-token):**
```
light_upload? â†’ csm? â†’ atlas? â†’ vb_sky â†’ vb_raster            (bare mesh_leg â€” BOTH arms)
  â†’ vb_viewt?                     (pre-tail slot iff ssao armed; late slot for taa-only)
  â†’ vb_geo                        (path_vb_split)
  â†’ ssao â†’ ssao_atrousÃ—N          (path_vb_ssao)
  â†’ [R9d hwrt: tlas â†’ shadow_vis â†’ shadow_atrousÃ—N â†’ shadow_temporal]
  â†’ [R9c: ddgi_update]            (path_vb_ddgi; reachable only VBÃ—Both)
  â†’ vb_shade_split                (path_vb_split; the lit producer)
  â†’ sdf_forward_march? â†’ vb_viewt?(taa-only late slot) â†’ taa_resolve? â†’ present_sample
```

**Declared accesses (new passes):**
- `vb_geo`: `vb_id` COMPUTE/SHADER_READ @SHADER_READ_ONLY_OPTIMAL (first reader derives COLORâ†’SRO);
  `vb_instance_ring` COMPUTE/READ; `thin_normal` COMPUTE/WRITE @GENERAL (first-touch, UNCONDITIONAL under split â€”
  Â§1.2 guarantees NORMAL is armed in every split config); `motion_vec` COMPUTE/WRITE @GENERAL (R9d, iff
  `thin_aux.MOTION`).
- `ssao` gather: `thin_normal` + `viewt` COMPUTE/READ @GENERAL; `ssao` COMPUTE/WRITE @GENERAL. No fat-gbuffer reads.
- `ssao_atrous` Ã—N: copy the deferred loop verbatim (graph_bridge.rs:1407-1447) â€” `viewt` READ, ring in/out via
  `ssao_atrous_step` roles; preserve the W2 "Write8 mask == gather mask" invariant comment.
- `ddgi_update` (R9c): accesses all four DDGI resources â€” `ddgi_irr`/`ddgi_depth` layered storage writes +
  `ddgi_classification`/`ddgi_ray_table` buffer RW â€” mirroring deferred 1612-1631 verbatim.
- `vb_shade_split`: `vb_id` READ @SRO; `lit` COMPUTE/WRITE @GENERAL (same transition the fused `vb_resolve`
  declares); `vb_instance_ring` READ; `light_table` READ (iff light_upload); `cascade`/`atlas` UNCONDITIONAL
  FULL-ARRAY reads @SRO (09600 â€” copy vb_resolve's arm 3243-3260); `ssao` COMPUTE/READ @GENERAL UNCONDITIONAL under
  split (backed by the seed, Â§3, and by the always-allocated image, Â§4); **`ddgi_irr`/`ddgi_depth` CONDITIONAL reads
  gated on `path_vb_ddgi()`** â€” mirroring the deferred resolve's `ddgi_update.is_some()`-gated atlas reads
  (1814-1821), so the update-write â†’ shade-read GENERAL barrier is derived and DDGI-off declares nothing (byte-id);
  R9d: `motion_vec`/shadow-chain reads per the deferred temporal consumer shapes.
- `VbPassPlan` gains `vb_geo, ssao, ssao_atrous[MAX], ddgi_update, vb_shade_split, (hwrt) tlas/shadow_vis/
  shadow_atrous[]/shadow_temporal: Option<PassId>`.
- `VbBarrierSink::images` grows per stage (Optionâ†’NULL when unarmed): `targets.thin_normal[fi]`, `targets.ssao`,
  `targets.ssao_ring_a/b`, DDGI atlases, then the hwrt tail.

## 4. Targets (`crates/boyko_rhi_vulkan/src/present/targets.rs`)

- `thin_normal`: `R8G8B8A8_UNORM` STORAGE, per-FIF ring `[VulkanTexture; FRAMES_IN_FLIGHT]` (the `viewt[fi]` policy â€”
  the cross-frame-WAR fingerprint). Allocated iff boot-frozen `mesh_geo_shade_split`. `motion_vec`: `R16G16_SFLOAT`,
  per-FIF, iff `thin_aux.MOTION` (R9d; cfg(hwrt) end-to-end like deferred).
- `ssao`/`ssao_ring_a/b`: reuse the existing deferred target fields/allocation paths; extend the boot-stable gate
  (targets.rs:4201) so a VB boot with split armed allocates `ssao` r8 **whenever split is armed** (incl. Temporal-only
  â€” backs the unconditional shade read; content never combined because `ssao_mode`=0, now freeze-guaranteed by Â§1.4).
  Ring allocation stays gated on `ssao_atrous_storage_ok()` + frozen ssao_on.
- `viewt[fi]` allocation widened per Â§2 (taa || split&&frozen-ssao).
- **"No mesh albedo/metal image exists" gate:** debug_assert + unit test on the VB targets profile: the allocated set
  contains NO albedo/material/pbr-class mesh image and the R9 delta is exactly `{thin_normal} (+{motion_vec} iff
  MOTION)`. Implementer FIRST verifies the current VB `TargetsProfile` (the "DeferredFull-shaped body" comment); any
  pre-existing surplus pins "R9 adds none" + a spun-off cleanup chip.
- Set builders (once per extent, iff armed; `forward_layout0` precedent):
  - `vb_geo_aux_set[fi]` against new `vb_geo_aux_layout` (Â§6).
  - `vb_ssao_set[fi]` against new `vb_ssao_layout` â€” thin_normal[fi]/viewt[fi]/ssao/camera.
  - `vb_split_set1` against new `vb_split_layout1` (Â§5 for the table).
  - **`build_ssao_atrous_sets` parameterized over the viewt ring (critic-fix):** the deferred builder binds the
    DEFERRED gViewT ring; under VB the chain must bind the VB `viewt[fi]` ring (possibly the only one allocated).
    Add a ring parameter (or a thin VB wrapper calling it with the VB viewt textures). `ssao_atrous_layout` itself is
    reused UNCHANGED â€” its r32f viewt binding matches `viewt_from_depth_rz`'s R32_SFLOAT output.

## 5. Shaders (`crates/boyko_rhi_vulkan/shaders/`, dxc 1.4.350.0, hermetic offline recipes)

**`vb_geo.comp.hlsl` (NEW).** One thread/pixel over `vb_id`; sentinel writes nothing. Unpack â†’
`vb_geom_fetch.hlsli` (SAME Set-2 contract, `%tri_count`, bary, perspective-correct interp â€” zero new math) â†’
interpolated GEOMETRIC vertex normal, oct-encoded with the SAME shared encoder `gNormal` uses; thin BA =
material-scalar roughness from `Materials[...].mrr`. v1: NO texture sampling in vb_geo (no SampleGrad, no Set-3) â€”
no armed consumer reads roughness today and SSAO/denoise on the geometric normal is the standard trade.
*Rejected:* SampleGrad'd normal-mapped thin normal â€” bindless set + gradients for low-frequency consumers; revisit
with SSR. Variant `-D MOTION=1` (`vb_geo_mv`, R9d): also writes `motion_vec` = unjittered curr-NDC âˆ’ prev-NDC of the
re-fetched world position (camera-reprojection, static-geometry v1 = the SDF leg's C6 semantics; per-instance
prev-transform is a later rung), reading a `MotionCam` UBO (prev/curr unjittered VPs); push stays 64 B jittered
`view_proj` (3 matrices would bust the 128 B push floor).
Bindings: Set 0 = `vb_layout0` REUSED as-is (existing 8-binding object + existing `vb_set0` group â€” gVbInstances@0,
Camera@2, Materials@4, gVbId@5 read; rest bound-but-unread, the R2 contract); Set 1 = NEW `vb_geo_aux_layout`
`{u0 thin_normal rgba8 W, u1 motion rg16f W (MOTION-only; benign thin_normal-view placeholder otherwise â€” the
mesh_sdf@15 precedent), b2 MotionCam (camera-ring placeholder when absent)}`; Set 2 = geometry (unchanged object).
*Rejected:* bespoke Set-0 â€” reuse costs zero descriptors and keeps `vb_geom_fetch.hlsli`'s contract untouched.

**`vb_shade_split.comp.hlsl` (NEW).** RE-fetch + RE-interp + RE-SampleGrad(_tex) â€” the `vb_resolve` shading tail
character-identical (sentinel skip, ALL-LIGHTS loop, duplicated pure helpers) â€” PLUS: (a) gSsao with the Filament AO
decoupling exactly as `deferred_pbr.hlsl:57-63,164-171`, gated by the light-header `ssao_mode` word (freeze-guarded,
Â§1.4 â€” no new host gate); (b) R9c DDGI probe sampling via `ddgi_resolve.hlsli` for mesh pixels, mirroring the
deferred resolve's GI-off gating mechanism verbatim (GI-off must stay byte-id â€” the 58f6c6c3 discipline); (c) R9d
`#if HWRT` denoised gShadowVis consumption mirroring the deferred `denoised` arm. Push = `vb_resolve`'s 64 B
`view_proj`. Sets: 0 = `vb_layout0` (+`vb_set0`/`vb_set0_tex` per frame, the R5 one-layout-two-sets rule);
1 = NEW **`vb_split_layout1`, 11 bindings (critic-fix â€” deferred DDGI uses SEPARATE texture+sampler pairs,
ddgi_resolve.hlsli:14-16/111-112, not combined-image-samplers):**
`@0-3` = `forward_layout1`'s 4-binding shadow table verbatim; `@4` gSsao r8 STORAGE READ; `@5/@6` ddgi_irr
Texture2DArray + SamplerState; `@7/@8` ddgi_depth Texture2DArray + SamplerState; `@9` DdgiParams UBO; `@10`
cfg(hwrt) gShadowVis rg16 STORAGE READ. The sampler objects are the existing deferred DDGI samplers, reused (no new
sampler plumbing). 11 â‰¤ 24 â€” Â§G budget arithmetic updated in the plan doc. A DISTINCT layout object so
`forward_layout1` (Forward family + `vb_resolve`) is byte-untouched. 2 = geometry; 3 = bindless textures (TEXTURED
variant only). Variant matrix (manifest rows): `vb_geo{,_mv}`, `vb_shade_split{,_tex}` Ã— software/hwrt embed legs â€”
DDGI/shadow gating runtime-worded like deferred, NOT -D axes. *Rejected:* per-consumer -D explosion (2^4 blobs) â€”
deferred_pbr proves the runtime-word + exact-fill architecture. *Rejected:* widening `forward_layout1` (touches the
Forward family) and a 5th descriptor set (breaks the maxBoundDescriptorSetsâ‰¥4 floor).

**`sdf_ssao.comp.hlsl` VB variant (`-D VB_THIN=1` â†’ `sdf_ssao_vb_{low,medium,high}.comp.spv`, 3 manifest rows).**
Under the define: bindings `{u0 thin_normal rgba8 READ, u1 gViewT r32f READ, u2 ssao r8 WRITE, b3 Camera}`
(gMaterial DROPPED); normal = thin_normal oct RG (same decoder); background mask = `view_t >= 1e30` replacing the
`gMaterial.b` test (taps on background reconstruct `Pp = P` exactly as today's `mask != 1` arm). The PCG dither /
slice-march eDSL span UNTOUCHED. *Rejected:* separate file â€” >90% shared with an oracle-pinned eDSL span; one source
of truth per the manifest discipline. Gate: `ssao_edsl_sync` re-DXC byte-identity extended to the 3 new blobs + one
host-mirror case for the `view_t>=1e30` mask arm.

**`viewt_from_depth_rz`**: reused verbatim (3-binding set, 16 B push) â€” only its ARMING widens (Â§2).

## 6. Boot / plumbing

- `crates/boyko_rhi_vulkan/src/compute.rs`: `embed_spirv!` entries + accessors `vb_geo_spirv()`, `vb_geo_mv_spirv()`
  (hwrt), `vb_shade_split_spirv()`, `vb_shade_split_tex_spirv()`, `sdf_ssao_vb_spirv_variant(q)`; `VbGeoPush` = 64 B
  view_proj; shade split reuses `VbResolvePush`.
- `crates/boyko_rhi_vulkan/src/present/scene_types.rs`: `GBufferScene` gains `vb_geo_pipeline`, `vb_geo_mv_pipeline`
  (hwrt), `vb_shade_split_pipeline`, `vb_shade_split_tex_pipeline`, `vb_geo_aux_layout`, `vb_split_layout1`,
  `ssao_vb_pipelines: Option<&[ComputePipeline; 3]>` (activation stays `SsaoActivation`; only the pipeline the VB
  recorder picks differs), all `Option` with the `forward_pipeline` None-rationale; the Â§2 predicates;
  `path_sdf_forward_writes_viewt()` UNCHANGED.
- `crates/boyko_app/src/gpu_scene/mod.rs`: build pipelines/layouts at boot (`vb_shade_split*` via the same
  deferred-build hook as `vb_shade_tex_pipeline` â€” geometry Set-2 + bindless Set-3); `to_gpu_resolved_render_path`
  round-trip test updated for the new truth table; `scene()` â€” `vb_viewt_armed` formula (Â§2), the freeze clamp
  fan-in (Â§1.4), thread `ssao_vb_pipelines`, keep `ddgi_enabled && sdf_leg`.
- `boyko_render`: `RenderPathFrozenConsumers` Resource + freeze-aware `sync_ssao_light_gate` (Â§1.4).
- `record_vb`: dispatch arms strictly in Â§3 order, each gated by the SAME predicate as its declaration (parity
  debug_asserts): vb_geo (`dispatch_group_count_x`, the vb_resolve shape) â†’ ssao gather (VB-variant pipeline +
  `vb_ssao_set[fi]`) â†’ Ã -trous loop (duplicate the deferred ~30-line role loop; shared-helper refactor is a separate
  golden-gated chip â€” record_gbuffer stays untouched) â†’ (hwrt chain) â†’ (ddgi_update) â†’ vb_shade_split (per-frame
  base/_tex by `vb_tex_active()`).
- `boyko_app/src/plugins.rs`: nothing new â€” `BOYKO_RENDER_PATH`/`BOYKO_GEOMETRY_LEGS` exist; SSAO reaches boot via
  the `SsaoConfig` Resource; the eval env seam `BOYKO_SSAO` already exists â€” no new env plumbing for the pin.

## 7. Tests / gates

- **Resolver truth tables**: VBÃ—{Mesh,Both,Sdf}Ã—{ssao,ddgi,temporal-only,combined}: `mesh_geo_shade_split` requires
  `mesh_leg`; **Temporal-only â‡’ split armed + `thin_aux == NORMAL|MOTION`** (the revised Rev-5 row, Â§1.2);
  `split â‡’ NORMAL` invariant; `shadow_temporal_on â‡’ hwrt_denoise_or_vis_on` assert; cap narrowing per stage incl.
  the VBÃ—Sdf residual rows; degrade-log expectations.
- **Freeze-guard units** (Â§1.4): both clamp tests incl. the ssao_mode-word Temporal-only case.
- **Viewt assert units** (Â§2): the 4-config table for both revised asserts.
- **Structural split test** (Â§0): split frame â‡’ vb_raster declared+recorded, classify chain absent.
- **Golden `vb_mesh_ssao`** (new `crates/boyko_app/tests/vb_mesh_ssao.rs`): clone `vb_mesh.rs` (five-sphere
  grand_showcase_2mat, production runner, `RenderPathConfig{VisibilityBuffer, Mesh}`) +
  `insert_resource(SsaoConfig{ quality: High, .. })` (deterministic in-test value). PINS.toml rows software+hwrt,
  `BOYKO_HOST_DUMP`, seeded `PENDING`, blessed only after owner visual sign-off. NOT byte-compared to any deferred
  pin (geometric-normal SSAO differs from mapped-normal SSAO by design â€” pin comment).
- **Fused unchanged when SSAO off** == ALL 15 existing pins byte-identical (standing gate); explicitly re-check
  `vb_mesh`, `vb_mesh_tex`, `vb_taa`, `vb_taa_rcas`, `vb_both`, `vb_both_taa`, `vb_sdf_only`, `vb_sdf_taa` (the
  assert revision + fused re-gating touch their code paths).
- **No-matcache assert** (Â§4). **Declare/record parity** debug_asserts on every new pass (W1 list: `path_vb_split`,
  `path_vb_fused`, `path_vb_ssao`, `path_vb_ddgi`, `vb_viewt_armed` + slot choice, the two revised viewt asserts,
  `path_needs_depth_prepass` untouched).
- **Shadow-temporal-under-VB current-frame-motion test (R9d, cfg(hwrt) only â€” under the software build the test does
  not exist, never SKIP-green):** config VBÃ—Mesh + `ShadowDenoiseMode::Temporal`, no SSAO/DDGI/SSR (the Temporal-only
  Rev-5 config, now NORMAL|MOTION): (a) unit: resolver yields split + NORMAL|MOTION + `vb_geo_mv` selected;
  (b) structural: pass order `vb_geo < shadow_temporal` assert; (c) `#[ignore]` GPU eval mirroring the R4
  shadow_lag_dump in-motion methodology (diagnose IN MOTION â€” the crossframe-race lesson).
- **Validation-ON**: `golden.ps1 -ValidationOn` over `vb_mesh_ssao` (R9b), **a VBÃ—Both + DDGI config, GI-on AND
  GI-off (R9c â€” the DDGI placeholder/off bindings in vb_split_layout1 first become reachable there; critic-fix)**,
  and one hwrt temporal config (R9d).
- **`ssao_edsl_sync`** extended to the 3 VB blobs; `vb_geo`/`vb_shade_split` recompile recipes pinned in headers;
  interpolation math needs no new oracle (vb_geom_fetch already oracle-pinned by R7's `vb_bary_edsl_sync`).
- Clippy `-D warnings` (touch edited sources first â€” false-fresh trap), full suite, Miri where new unsafe (expected:
  none beyond sink array growth SAFETY comments).

## 8. Docs

`docs/MULTI-PARADIGM-RENDER-PLAN.md`: R9 row per stage + TWO errata (`mesh_leg` on the split rule; Temporal-only =
NORMAL|MOTION, the Â§D:306 ownership table updated) + vb_shade naming note + Â§G budget update (vb_split_layout1 = 11).
`docs/SHADER-VARIANT-MANIFEST.md`: rows for `vb_geo{,_mv}`, `vb_shade_split{,_tex}`, `sdf_ssao_vb_{low,medium,high}`.
`docs/VB-P2-CLASSIFICATION-PLAN.md`: split-displaces-classification note. `graphify update .` after commit.

## 9. Rung staging (each independently green + author-only commit+push)

- **R9a â€” resolver + freeze + plumbing (no graph change).** `mesh_leg` rule; NORMAL<=hwrt arm + errata (Â§1.2);
  `RenderPathFrozenConsumers` + both clamps + warn-once; POD round-trip; truth tables; predicates dead-but-threaded.
  Gate: all Â§7 units for this layer; ALL 15 pins byte-identical; clippy.
- **R9b â€” split core + SSAO under VB (software leg).** Cap lifts ssao_on (mesh_leg-scoped); vb_geo +
  vb_shade_split{,_tex}; sdf_ssao VB_THIN variants; Ã -trous ring-parameterized reuse; vb_viewt widening + BOTH
  asserts revised; seeded ssao ResId; targets/sets; declare+record. Gate: pin `vb_mesh_ssao` blessed (owner
  sign-off); fused pins byte-identical; no-matcache; structural split test; parity asserts; validation-ON;
  ssao_edsl_sync extended.
- **R9c â€” DDGI under VBÃ—Both.** Cap lifts ddgi_on (Both-scoped); ddgi_irr/ddgi_depth seeded images +
  classification/ray_table seeded buffers appended; ddgi_update declared/recorded (`path_vb_ddgi`); conditional atlas
  reads + probe sampling in vb_shade_split (deferred-mirrored). Gate: vb_both/vb_both_taa pins unchanged (GI-off
  byte-id); **validation-ON over VBÃ—Both+DDGI on AND off**; owner GI eval on vb_lab; truth table.
- **R9d â€” hwrt shadow chain under VB (cfg hwrt).** Cap narrows to its final VBÃ—Sdf-residual form; motion_vec + shadow
  ids appended as the cfg(hwrt) TAIL; tlas + shadow_vis(thin-normal variant) + atrous + temporal; vb_geo_mv +
  MotionCam; motion pre-tail. Gate: the Temporal-only regression test (unit + order assert + in-motion eval,
  cfg(hwrt)-only); hwrt legs of all pins unchanged; validation-ON.

## 10. Risk register

1. **Viewt-assert revision regressing TAA pins** â€” strict XOR kept verbatim for all `!ssao` configs; the 7 VB pins
   are the tripwire.
2. **Placeholder descriptor validation churn** (motion/MotionCam when MOTION off, DDGI-off, gSsao under
   Temporal-only) â€” copy the deferred GI-off + mesh_sdf@15 placeholder strategies verbatim; validation-ON gates now
   at R9b AND R9c AND R9d.
3. **Targets shape unknown under VB** ("DeferredFull-shaped body" comment) â€” verified before the no-matcache assert;
   pre-existing surplus spawns a separate chip.
4. **R9d is the heavy stage** (TLAS + 3-pass chain + history parity inside the VB graph) â€” deliberately last and
   cfg-isolated; slippage does not block R9b's owner-visible SSAO deliverable.
5. **Geometric-vs-mapped thin normal** â€” VB SSAO/denoise may read flatter on normal-mapped surfaces; owner eval at
   R9b bless decides whether a SampleGrad'd vb_geo variant gets scheduled.
6. **Split displaces classification** â€” textured VB frames with any pre-light consumer lose the classify perf path
   until classified-split; documented, correctness-first.
7. **Push floor** â€” vb_geo motion via MotionCam UBO keeps every push â‰¤ 64 B; future matrices go through UBOs.
8. **DDGI-only VBÃ—Both pays the full split for nothing thin-aux-consumable (critic-added):** ddgi_update reads no
   screen-space thin-aux and vb_shade_split re-derives its own interpolated normal, so vb_geo's full-screen pass +
   thin_normal bandwidth have ZERO consumers in that config â€” a pure cost of the Rev-5 single-`pre_light`-predicate
   no-drift rule. Recorded as a measured-later cost; recovery = the classified-split rung, and a future rung may
   narrow VB's `pre_light` to consumers that actually read thin-aux once the Rev-5 coupling is re-derived.
9. **The vb_mesh_ssao pin is a fresh visual identity** â€” blessing requires the owner as visual oracle on real
   hardware (os-740: subagents can't run fresh GPU exes; loops through the orchestrator).
10. **Freeze-clamp coverage** â€” any NEW per-frame bridge of a pre-light consumer added later must route through
    `RenderPathFrozenConsumers` or the ssao_mode-class hazard reopens; the Â§1.4 unit tests + a doc note on the
    Resource are the guard.

## Key decisions (with rejected alternatives)

- Split passes named vb_geo + vb_shade_split; the shipped classification vb_shade keeps its name â€” rejected renaming the frozen pinned blob (OpSource/.spv churn).
- Split DISPLACES the classification chain in v1, but SURGICALLY: vb_raster stays gated on bare mesh_leg (both arms consume vb_id/vb_depth); only the classify chain + the vb_resolve/vb_shade selection re-gate on !mesh_geo_shade_split â€” rejected re-gating the whole mesh_leg block (kills the vb_id producer under split) and rejected classified-split composition (later perf rung).
- resolve_rules gains `&& mesh_leg` on mesh_geo_shade_split (R-SDFFWD precedent) â€” rejected the plan's literal `VB && pre_light` (splits a mesh-less path).
- thin-aux NORMAL union gains hwrt_denoise_or_vis_on under non-Deferred paths (the shadow_vis gather is a NORMAL consumer there): Temporal-only under VB = NORMAL|MOTION, with a plan-doc erratum replacing the Rev-5 'MOTION-only' label â€” rejected making thin_normal split-structural independent of the mask (the mask must stay the single truth) and rejected the original contradictory MOTION-only truth-table row.
- path_sdf_forward_writes_viewt() is NOT widened (stays taa-only): widening was path-blind and would panic the Forward declarator tripwire / NULL-write gViewT under ForwardÃ—Both+SsaoConfig; ONLY vb_viewt widens, via the VB-scoped boot-anchored predicate `vb && mesh_leg && ((taa && !sdf_leg) || (mesh_geo_shade_split && ssao))` â€” rejected the original symmetric widening (CRITICAL Forward blast radius, and the marcher's no-TAA viewt write had zero consumers).
- BOTH viewt producer asserts revised as one unit: the unconditional mutual-exclusion (graph_bridge.rs:3362) admits dual arming only when ssao && mesh_geo_shade_split; the TAA XOR (3397) stays strict for !ssao configs and degrades to at-least-one + marcher-is-last-writer when ssao armed â€” rejected revising only the XOR (the second assert would deterministically panic VBÃ—Both+SSAO+TAA).
- The VB graph's ssao ResId is add_image_seeded(ResSync::undefined()) copying deferred line 807 verbatim â€” rejected plain add_image (vb_shade_split's unconditional read on split-without-SSAO frames trips compile()'s unwritten-transient-read guard); thin_normal/rings stay plain add_image.
- Boot-freeze = a RenderPathFrozenConsumers Resource clamped ONCE upstream of BOTH the scene() arming and sync_ssao_light_gate (freeze-aware header word) â€” rejected clamping scene() only (runtime SsaoConfig would set ssao_mode=1 and combine seeded-UNDEFINED garbage under a Temporal-only boot) and rejected a recorder-side force (the header upload never sees GBufferScene).
- R9c DDGI resources fully enumerated: ddgi_irr/ddgi_depth add_image_seeded at SHADER_READ_ONLY_OPTIMAL with layered subranges + ddgi_classification/ddgi_ray_table add_buffer_seeded, ddgi_update accessing all four, vb_shade_split's atlas reads CONDITIONAL on path_vb_ddgi() â€” rejected 'copy the deferred shape, implementer verifies' (fails the no-re-deriving bar; the resources ARE framegraph-tracked).
- VB image ids follow the deferred 'cfg(hwrt) ids declared LAST' precedent: software ids thin_normal=8/ssao=9/rings=10,11, ddgi=12,13 (R9c), hwrt tail motion_vec+shadow chain last (R9d) â€” rejected the original motion_vec=9 placement (a cfg-gated id in the middle desyncs the two builds' ResId spaces).
- cap_vb_v1_consumers is narrowed per stage but NOT deleted: final form zeroes all pre-light consumers under VB && !mesh_leg (the R-SDFSPLIT boundary), keeping the existing degrade variant name â€” rejected full deletion (VBÃ—Sdf would arm flags nothing consumes) and rejected one-shot narrowing.
- path_vb_ssao() = mesh_geo_shade_split && ssao.is_some() (boot-frozen-anchored; gather can only arm when its producer vb_geo is boot-armed) â€” rejected the per-frame path_is_vb()&&ssao form (correct only while the clamp holds).
- vb_split_layout1 = forward_layout1's 4 shadow bindings + gSsao + SEPARATE DDGI texture/sampler pairs + DdgiParams + cfg(hwrt) gShadowVis = 11 bindings, reusing the deferred DDGI sampler objects â€” rejected combined-image-samplers (diverges from ddgi_resolve.hlsli's declared contract) and rejected widening forward_layout1 or a 5th set.
- SSAO gather = -D VB_THIN variant of sdf_ssao.comp.hlsl (thin_normal oct RG, gViewT>=1e30 background mask, gMaterial dropped) â€” rejected a separate file (one oracle-pinned source of truth).
- SSAO/DDGI/shadow gating in vb_shade_split is runtime-word + exact-fill (deferred_pbr architecture); only TEXTURED and cfg-hwrt are compile axes â€” rejected a 2^4 -D explosion.
- vb_geo v1 writes the GEOMETRIC interpolated normal + material-scalar roughness (no textures/SampleGrad); motion (R9d) is camera-reprojected static-geometry via a MotionCam UBO, push stays 64 B â€” rejected normal-mapped thin normal and rejected matrices-in-push.
- thin_normal/motion per-FIF rings (viewt[fi] policy) â€” rejected single-buffered (cross-frame-WAR fingerprint); ssao image allocated whenever split armed so the unconditional 09600 read is always backed.
- build_ssao_atrous_sets parameterized over the viewt ring (VB binds the VB viewt[fi] textures; ssao_atrous_layout reused unchanged, r32f match) â€” rejected 'reused verbatim' (would bind the deferred ring, possibly unallocated under VB).
- New pin vb_mesh_ssao = clone of vb_mesh.rs + insert_resource(SsaoConfig High); no new env plumbing â€” rejected riding a deferred-scene pin.
- SSAO under VBÃ—Both is mesh-pixels-only this rung (SDF pixels read the vb_viewt sentinel; fused march unchanged) â€” rejected pulling the SDF split into R9.

## Staging

- R9a â€” resolver + freeze guard + plumbing (no graph change): mesh_leg rule on mesh_geo_shade_split; NORMAL<=hwrt_denoise_or_vis_on under non-Deferred + the two plan-doc errata (mesh_leg; Temporal-only=NORMAL|MOTION incl. the Â§D ownership table); RenderPathFrozenConsumers Resource + upstream clamp + freeze-aware sync_ssao_light_gate + warn-once; POD round-trip; predicates dead-but-threaded. Gates: resolver truth tables (incl. splitâ‡’NORMAL and temporalâ‡’hwrt asserts, VBÃ—Sdf residual-cap rows), both freeze unit tests (incl. ssao_mode-word Temporal-only case), ALL 15 pins byte-identical, clippy -D warnings.
- R9b â€” vb_geo + vb_shade_split{,_tex} + SSAO under VB (software leg): cap lifts ssao_on mesh_leg-scoped; sdf_ssao VB_THIN variants; ring-parameterized Ã -trous reuse; vb_viewt arming widened + BOTH viewt asserts revised (strict XOR preserved for !ssao); ssao ResId seeded (deferred-807 pattern); images 8-11; targets/sets (vb_split_layout1 = 11 bindings, deferred DDGI samplers reused); declare+record. Gates: NEW pin vb_mesh_ssao blessed after owner visual sign-off; every existing pin byte-identical; no-matcache targets assert; structural split test (vb_raster present, classify absent); declare/record parity debug_asserts; validation-ON clean; ssao_edsl_sync re-DXC extended; viewt-assert 4-config unit table.
- R9c â€” DDGI under VBÃ—Both: cap lifts ddgi_on Both-scoped; ddgi_irr=12/ddgi_depth=13 add_image_seeded (SRO seed, layered subranges) + ddgi_classification/ddgi_ray_table add_buffer_seeded; ddgi_update declared/recorded under path_vb_ddgi(); conditional atlas reads + probe sampling in vb_shade_split mirroring deferred's GI-off-byte-id gating. Gates: vb_both/vb_both_taa pins unchanged (GI-off byte-id); golden.ps1 -ValidationOn over VBÃ—Both+DDGI GI-on AND GI-off; owner GI eval on vb_lab; truth table.
- R9d â€” hwrt shadow chain under VB (cfg hwrt): cap narrows to its final VB&&!mesh_leg residual; motion_vec + shadow-chain ids appended as the cfg(hwrt) TAIL (deferred hwrt-last precedent); tlas + shadow_vis(thin-normal variant) + atrous + temporal; vb_geo_mv + MotionCam; motion pre-tail. Gates: shadow-temporal-under-VB current-frame-motion test on the Temporal-only (now NORMAL|MOTION) config â€” resolver unit + vb_geo<shadow_temporal order assert + in-motion GPU eval, all cfg(hwrt)-only (stated in the matrix; the test does not exist on software builds, never SKIP-green); hwrt legs of all pins unchanged; validation-ON.

## Risk register (workflow-final)

- Revised viewt asserts / vb_viewt pre-tail slot could perturb the 7 shipped VB TAA pins' barrier schedule â€” the strict XOR arm is kept verbatim for all !ssao configs and the pins are the tripwire.
- Placeholder descriptor strategy (motion/MotionCam when MOTION off, DDGI-off, gSsao under Temporal-only) can regress validation-clean â€” deferred GI-off + mesh_sdf@15 patterns copied verbatim, with -ValidationOn gates at R9b, R9c (DDGI on AND off), and R9d.
- Current VB TargetsProfile shape not fully verified ('DeferredFull-shaped body'): the no-matcache assert may reveal pre-existing surplus deferred images â€” handled as a spun-off cleanup chip, not silently inside R9.
- R9d is the heavy stage (TLAS + 3-pass denoise + history parity images, all cfg(hwrt)) â€” deliberately last and cfg-isolated; slippage does not block R9b's owner-visible SSAO deliverable.
- Geometric (non-normal-mapped) thin normal may read flatter than Deferred SSAO on textured surfaces â€” owner eval at R9b bless decides whether a SampleGrad'd vb_geo variant is scheduled.
- Split displacing classification: textured VB frames with any pre-light consumer lose the classify perf path until classified-split â€” documented, measured-later.
- DDGI-only VBÃ—Both pays the full split (vb_geo full-screen pass + thin_normal bandwidth) with ZERO thin-aux consumers in that config â€” a recorded cost of the Rev-5 single-pre_light no-drift rule; recovery = classified-split, or a future rung narrowing VB's pre_light to actual thin-aux readers.
- DDGI gating in vb_shade_split must mirror deferred_pbr's shipped mechanism exactly (runtime word vs variant) or GI-off byte-identity (58f6c6c3 discipline) breaks â€” implementer verifies the shipped mechanism before writing the shader.
- The vb_mesh_ssao pin is a fresh visual identity â€” owner is the visual oracle on real hardware; cannot be auto-gated (os-740, loops through the orchestrator).
- Freeze-clamp coverage is a standing invariant: any future per-frame bridge of a pre-light consumer must route through RenderPathFrozenConsumers or the ssao_mode-class garbage-combine hazard reopens â€” guarded by the Â§1.4 unit tests + a doc note on the Resource itself.
