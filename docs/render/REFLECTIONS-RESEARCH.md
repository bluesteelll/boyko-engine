# Reflections — the comparative survey (research record)

> Status: research record for the reflections campaign, 2026-09-10. Companion: the design for
> THIS engine is [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md). Nothing here is
> implemented; the tree holds **no image-based lighting, no reflection probe, no SSR, no
> reflection ray** (§0) — every specular environment term on screen today is an analytic
> two-colour gradient plus a Phong-kernel sun disc.
>
> **Provenance discipline.** Every claim about another engine carries a tag: **[S]** source code
> read, **[D]** official docs / talk / paper, **[B]** blog / marketing (recorded, not relied on),
> **[T]** a file in this tree re-opened by the author of this document. External URLs were opened
> by the five research lenses of this session and are carried through verbatim in §8; every
> in-tree `file:line` was **re-opened by the architect** on branch `feat/multi-paradigm-render`
> at commit `ed0bed45` and corrected where a lens was off (**§9** lists the corrections made while
> re-opening the lenses' claims; **§10** answers the `architecture-critic`'s findings against this
> document, including three §0 shader censuses that were short). Untagged =
> UNVERIFIED. **No timing was taken on the owner's box** — every millisecond in this document is a
> published figure on someone else's GPU, quoted with its source, and §4 says so per number.

## 0. The tree today — what is true before any design (all [T])

The brief describes the engine as it wishes to be; the design must start from what is on disk.
Each fact below was re-opened at `ed0bed45`.

### 0.1 The BRDF core is standard and correct — and it is hand HLSL, not eDSL

- `crates/boyko_rhi_vulkan/shaders/pbr_lighting.hlsli:61-79` ships `D_GGX` (the stable
  `(NoH·a2 − NoH)·NoH + 1` rearrange), `V_SmithGGXCorrelated` (height-correlated, folds the
  `1/(4 NoL NoV)`), `F_Schlick`. `Surface { n, NoV, a = rough², f0, diffuse_color, energy_comp }`
  at `:109-116`; `eval_pbr_direct_bsdf` at `:133`.
- The header's own doc (`:1-12`) says it is "a VERBATIM cut" gated by **image goldens**, with
  SPIR-V byte-comparison "best-effort" because a moved span changes `__FILE__`/`__LINE__`.
- **The brief's "EVERY shader is authored in the eDSL" is false for the BRDF.** The sentinel
  census is **20 of 89** `.hlsl`/`.hlsli` files carrying a `// === GENERATED <name> BEGIN/END ===`
  span (`grep -l` over `crates/boyko_rhi_vulkan/shaders/`), and `crates/boyko_shaderdsl/src`
  contains **zero** BRDF leaves (`grep -rli 'ggx|schlick|fresnel'` is empty). The BRDF's oracle is
  `goldens.rs` (`env_brdf_approx` `:1774-1786`, `multi_scatter_energy_comp` `:1802-1808`,
  `specular_ao` `:1822-1826`), not an `_edsl_sync` test. A reflection campaign that adds leaves
  adds them beside a hand-written core, and the design must say which side each new function
  lands on.

### 0.2 The environment is analytic — "No IBL texture, no LUT (MVP-2)"

- `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:54-63` is the header paragraph: ambient
  specular = Karis-mobile `EnvBRDFApprox` sampled along `R` against a **smoothstep-steepened
  sky/ground gradient**, hemisphere diffuse along `N`, decoupled specular AO, a 1-mul
  multi-scatter term, and the sentence "No IBL texture, no LUT (MVP-2)".
- `env_brdf_approx` at `:582-589` (the `c0`/`c1` polynomial, "no DFG LUT") is duplicated verbatim
  in **SIX** files, not two (critique B1/NB10 — the first draft of this section named
  `forward_opaque.fs.hlsl` and `vb_shade.comp.hlsl` only, and that undercount is what propagated
  into the design's splice list). `grep 'float2 env_brdf_approx'` over
  `crates/boyko_rhi_vulkan/shaders/`, re-run at `ed0bed45`:

  | File | `float2 env_brdf_approx` | `eval_pbr_ambient_hemi` call | `R = reflect(-v, n)` hoist | Path |
  |---|---|---|---|---|
  | `deferred_pbr.hlsl` | `:582` | `:1163` | `:861` | Deferred resolve |
  | `forward_opaque.fs.hlsl` | `:182` | `:303` | `:250` | Forward / ForwardPlus mesh leg |
  | `sdf_forward_march.comp.hlsl` | `:855` | `:1080` | `:1035` | the SDF leg under Forward and VB |
  | `vb_resolve.comp.hlsl` | `:212` | `:368` | `:297` | VB **fused** |
  | `vb_shade.comp.hlsl` | `:247` | `:522` | `:449` | VB fused, material-classified |
  | `vb_shade_split.comp.hlsl` | `:308` | `:540` | `:451` | VB **split** |

  The two files the first draft missed are the ones whose own headers say they are
  TOKEN-FOR-TOKEN copies of a sibling's ambient+tonemap tail — `vb_resolve.comp.hlsl:23` and
  `sdf_forward_march.comp.hlsl:37` — i.e. the same duplicate class the census was counting, which
  is exactly why a prose list is the wrong instrument here and the design pins the count with a
  census test rather than a list (`REFLECTIONS-DESIGN-SPACE.md` §2.1). The "Duplicated (not
  shared) pure helpers" rule is stated at `vb_resolve.comp.hlsl:26-30`: a compute pass cannot
  share a raster FS's local helpers except textually.
- The ambient site is ONE function: `eval_pbr_ambient_hemi` (`pbr_lighting.hlsli:154-166`) —
  `spec_ambient = (f0·dfg.x + dfg.y) · refl_color · energy_comp`, `refl_color = lerp(ground, sky,
  smoothstep(refl_hemi))`. This is exactly where a prefiltered radiance replaces the gradient. It
  is ONE function with **six** consumers (the table above), one per `lit` producer, and any change
  to the gradient that reaches fewer than six leaves a path on the old appearance — the P39 class.
- The "chrome cue" is an analytic sun disc: `eval_pbr_sun_disc` (`:177-179`), the kernel at
  `deferred_pbr.hlsl:592-624` whose `sun_kernel_exponent` is documented as "the cheap analytic
  stand-in for prefiltered-cubemap mip selection" (`:596-597`), `SUN_ENV_WEIGHT = 1.0` at `:624`,
  and the site at `:1133-1150` says the double-count with the direct lobe is **intentional**
  ("real-time engines carry the sun both ways"). The background paints the same gradient + a
  fixed-exponent disc (`:626-633`, `:1409-1412`; `forward_sky.fs.hlsl:1-12`; `vb_sky` pass at
  `graph_bridge.rs:4585`).
- `R = reflect(-v, n)` is hoisted once per pixel at **all six** sites, not the three first named
  here — the third instance of the same undercount, corrected in the table above
  (`deferred_pbr.hlsl:861`, `forward_opaque.fs.hlsl:250`, `sdf_forward_march.comp.hlsl:1035`,
  `vb_resolve.comp.hlsl:297`, `vb_shade.comp.hlsl:449`, `vb_shade_split.comp.hlsl:451`). The hoist
  is free for the chain: `R` is already in scope at every site the design splices into.
- Energy compensation: `deferred_pbr.hlsl:833-843` — `Ess = max(dfg.x + dfg.y, 1e-4)`,
  `energy_comp = 1 + f0·(1/Ess − 1)`, with the comment "NOT `1/dfg.y`". The **input** is the
  analytic fit; the form is Fdez-Agüera's.
- Specular occlusion: `:958-965` is `saturate(pow(NoV + ao, exp2(−16·rough − 1)) − 1 + ao)` —
  character-equal to Filament's `SpecularAO_Lagarde` [S].
- The "sky" is a light-table row: `SkyLight { sky_color, ground_color }`
  (`crates/boyko_render/src/light.rs:355-361`), `LIGHT_KIND_SKY = 3` (`light_table.hlsli:43`),
  header lanes `sky_diffuse` @16 / `sky_spec` @32 (`light.rs:161-181`). The header's spare word
  budget is documented at `light.rs:387-412`: word 7 bits 0..4 taken, 5..6 SV0, **7 free**, 8..11
  tonemapper, 12..19 terminator, 20..31 free; growing `LIGHT_HEADER_WORDS` "would shift
  `LIGHT_HEADER_BASE` and re-encode every golden".
- The plan record: `docs/PBR-MATERIALS-PLAN.md:149-155` chose EnvBRDFApprox + analytic sky and
  rejected split-sum "for v1 … defer to a reflection-probe phase"; `:396` books "**T4 — IBL
  upgrade (deferred):** reflection-probe cube + split-sum, only if analytic sky proves
  insufficient". `docs/LIGHTING-PLAN.md:22-31` has an L2 irradiance volume and L3 DDGI but **no
  reflection-probe rung anywhere**.

### 0.3 Three blockers live in the RHI and asset layers, not in shading

1. **No cube views.** `crates/boyko_rhi/src/enums.rs:526-542`: `TextureViewDimension = { D2 = 1,
   D3 = 2, D2Array = 5 }` with the doc "1D and cube views are absent because no image in the
   engine is created with those shapes." No `CUBE` token exists anywhere in `boyko_rhi/src` or
   `boyko_rhi_vulkan/src` (`grep -rn CUBE` is empty). Image create flags are chosen at
   `boyko_rhi_vulkan/src/texture.rs:240-271` (`mutable_format` only). `MAX_TEXTURE_LAYERS = 16`
   (`texture.rs:117`).
2. **No BCn / HDR-container formats.** `Format` (`enums.rs:270-390`) holds `R8G8B8A8
   Unorm/Srgb`, `B8G8R8A8 Unorm/Srgb`, `R8Snorm/Unorm`, `R8G8Unorm`, `R16Sfloat/Unorm`,
   `R16G16Unorm/Sfloat`, `R16G16B16A16Unorm/Sfloat`, `B10G11R11UfloatPack32`, `R32Sfloat`,
   `R32G32Uint`, `R32G32B32Sfloat`, `D32Sfloat`. `grep -n 'Bc[0-9]|BC[0-9]|E5B9'` returns nothing.
   The half-float and 11-11-10 targets an HDR environment needs DO exist.
3. **No HDR image decoder.** `crates/boyko_image/src/` is `error.rs inflate.rs lib.rs png.rs`;
   `lib.rs:9-19` scopes it to PNG colour types 0/2/4/6, bit depths 8 and 16, non-interlaced. No
   `hdr`/`exr`/`ktx`/`dds`/`rgbe` match anywhere in `boyko_image` or `boyko_render/src/loaders`.
   The texture upload seam is RGBA8-shaped: `upload_texture_2d(…, rgba8: &[u8], format,
   view_format)` (`crates/boyko_render/src/texture.rs:118-124`), sRGB via a mutable view
   (`:26-37`, `:83-86`), mips by linear blit chain (`:166-168`); `upload_texture_2d_raw`
   (`:373-384`) accepts a `Format` but its byte-per-pixel match covers only `R8Unorm`, `R8G8Unorm`,
   `R8G8B8A8Unorm` today. `docs/PBR-MATERIALS-PLAN.md:13` and `:178-190` defer BC7/BC5/BC6H and an
   in-house DDS reader to `docs/PBR-TEXTURES-PLAN.md`, **which does not exist** (`ls docs | grep -i
   TEXTURE` is empty).

Two precedents soften the blockers: the SMAA LUTs are baked `.bin` files uploaded through
`upload_texture_2d_raw` at boot (`crates/boyko_render/src/smaa_luts.rs:1-22`, `:44-45`) — the
exact shape of a DFG LUT; and the DDGI atlas is an **octahedral `Texture2DArray`** with
per-tile borders (`crates/boyko_rhi_vulkan/src/ddgi.rs:1-36`, `:160-171`), with the eDSL owning
`oct_encode`/`oct_decode` (`crates/boyko_shaderdsl/src/oct.rs`; `emit/shaders.rs:1869`, `:2214`)
and the octahedral blend leaves (`probe_blend.rs:1-15`). An environment map can be an octahedral
2D image and touch none of blocker 1.

### 0.4 There is no HDR scene colour anywhere in the frame

- `lit` is `R8G8B8A8` (`crates/boyko_rhi_vulkan/src/present/targets.rs:123-127`; `GBUFFER_FORMAT
  = R8G8B8A8Unorm` at `:2267`). The resolve applies the ACES fit and a manual `pow(1/2.2)` OETF
  at the `lit` write (`pbr_lighting.hlsli:181-209`; the OETF is at `deferred_pbr.hlsl:1409`), and
  `material.rs:56-60` documents that the swapchain is UNORM end to end so the OETF must be manual.
- **The tail is applied by EIGHT shaders, not by the resolve alone** (critique NB6, whose
  undercount is seeded here — this section named only the resolve, and the design then costed the
  move at "three producers"). `grep -l 'OETF_GAMMA_EXP\|tonemap_select'` over
  `crates/boyko_rhi_vulkan/shaders/` at `ed0bed45`: `deferred_pbr.hlsl`, `forward_opaque.fs.hlsl`,
  `forward_sky.fs.hlsl`, `sdf_forward_march.comp.hlsl`, `ssaa_downsample.fs.hlsl`,
  `vb_resolve.comp.hlsl`, `vb_shade.comp.hlsl`, `vb_shade_split.comp.hlsl` — plus the definition in
  `pbr_lighting.hlsli:181-209`. `ssaa_downsample.fs.hlsl` is the one that is not merely a tail: it
  decodes, box-averages and re-encodes display-space texels (`:8-16`) and records its own ceiling
  "avg(tonemap(x)), not tonemap(avg(x))" (`:17-19`), so moving the tonemap to the frame's end
  changes that pass's SHAPE, not only its last line. Any HDR-`lit` rung therefore re-blesses eight
  producers, and a per-producer transition pin is the only way a producer that kept its tail is
  distinguishable from one that did not.
- `taa_hist` is `R16G16B16A16_SFLOAT` but "avoids per-blend re-quantization of the
  **already-8-bit-post-tonemap `lit`**" (`targets.rs:468-480`); it is `None` unless `AaMode::Taa`
  (`aa_config.rs:61-70`) and reset on first-armed / resize (`taa_state.rs:1-12`, `:24-35`). Its
  input binding is "the current frame's shaded **LDR** color `lit`" (`taa_resolve.comp.hlsl:87`).
- Every shipping SSR samples HDR pre-tonemap radiance (§2.3, §2.4). This is the single largest
  unbudgeted prerequisite of the screen-space half of the ladder.

### 0.5 Motion vectors are camera-only on every path; per-object is `hwrt`-gated

`crates/boyko_render/src/motion_cam.rs:1-46`: the `MotionCam` UBO is the camera cur/prev
view-proj pair (128 B, `MotionCamState` a `Resource`); TAA reconstructs its motion in-shader from
`gViewT` + this pair and "NO per-pixel motion-vector producer is read"
(`taa_resolve.comp.hlsl:13-28`). The per-object half (`PrevInstanceModelCol`, `motion_vec`
R16G16_SFLOAT) is `#[cfg(feature = "hwrt")]` (`targets.rs:172-181`; `motion_cam.rs:38-45`:
"TAA v1 cannot reproject a MOVING object's history — a known, surfaced v1 limitation"). VB's
`vb_geo` writes camera-only static motion under `-D MOTION` (`vb_geo.comp.hlsl:99-104`).

### 0.6 The SSR seam is pre-cut, and permanently capped

`crates/boyko_render/src/render_path_config.rs`: `pre_light = ssao ∥ ddgi ∥ shadow_denoise_spatial
∥ shadow_temporal ∥ ssr` is the sole trigger of `needs_depth_prepass` / `mesh_geo_shade_split` /
`sdf_geo_shade_split` (`:74-84`, `:1046-1049`, `:1086`); `ThinAuxMask::NORMAL` is "armed for
SSAO/DDGI/shadow-denoise-spatial/SSR" and `ROUGHNESS` is "8-bit roughness, packed in
`thin_normal.BA` — armed for SSR (future)" (`:444-447`, inserted at `:1192-1194`); `ssr_on` is "No
`SsrConfig` exists yet … the caller threads a literal `false`" (`:815-817`), "ALWAYS capped (no SSR
exists engine-wide, no rung lifts this)" (`:1392`), zeroed at `:1372` and `:1473`, and the cap is
**pinned by tests** at `:1843`, `:1944`, `:2061`, `:2643`. `docs/MULTI-PARADIGM-RENDER-PLAN.md:379`
already names SSR's inputs per path ("depth+normal+rough+prev-lit") and `:280` states that
"`roughness` in thin-aux is a **consumer output** (SSR), not a shading-input cache".

A doc discrepancy to carry: the VB `thin_normal` packs roughness in **`.B`** as a plain UNORM
(`vb_geo.comp.hlsl:44-53`; `targets.rs:339-341`) while `ThinAuxMask::ROUGHNESS`'s doc says
"`thin_normal.BA`" (`render_path_config.rs:446`). Roughness is NOT in the untextured deferred
G-buffer at all — it is fetched from the material SSBO by the 16-bit id in `gNormal.BA`
(`material.rs:4-8`; `deferred_pbr.hlsl:799-801`, clamped `[0.045, 1]`); only the TEXTURED path
writes it to the `pbr` MRT lane (`R16G16B16A16_SFLOAT`, `targets.rs:145-153`;
`gbuffer_mrt.fs.hlsl:73-81`, software resolve only).

A march precedent exists inside the resolve: `project_to_screen` (the exact inverse of
`generate_ray`, `deferred_pbr.hlsl:645-662`) and `sscs_march` (`:691`, called at `:1110`, `:1384`)
with IGN dither (`:636-644`), a thickness tolerance, edge vignette and distance fade.

### 0.7 The HZB has the wrong polarity for reflection tracing, and exists on VB only

`crates/boyko_render/src/hzb.rs:55-72` (and `hzb_build.comp.hlsl:20-33`): the pyramid is a
`min` reduce over hardware reverse-Z, i.e. **the FARTHEST surface per footprint**, chosen so the
occlusion reject `depth_near < occ` is sound; "A `max` here would hold the NEAREST surface … It
would be silently wrong in the one direction that has no visual tell in a static golden."
Hi-Z reflection tracing needs the nearest bound (§2.3). The pyramid is `R32_SFLOAT`, `prev_pow2`
base, `MAX_HZB_LEVELS = 17` (`hzb.rs:15-28`, `:188-194`), planned only when
`HzbConfig::Build` or `OcclusionMode::TwoPhase` asks (`hzb_config.rs:114-124`;
`boyko_app/src/hzb_plan.rs:18-31`), and armed only by `path_vb_occlusion_split` — VB path, mesh
leg, occlusion instances present (`scene_types.rs:4246-4253`); it is built after `vb_raster`
(`graph_bridge.rs:4933`) and before `vb_cull_late` (`:5100`), from the early-pass depth. NaN
policy: HLSL `min()` is forbidden there because `OpFMin/NMin` takes the OTHER operand on NaN
(`hzb_build.comp.hlsl:122-130`) — the same hazard every new pyramid inherits.

Depth is not one thing either: Deferred writes a custom linear `length(eye_rel)/T_MAX`, `T_MAX =
10` (`crates/boyko_render/src/gbuffer_depth.rs:5-13`, `:50`; `DepthKind::CustomLinear`,
`render_path_config.rs:419-431`), Forward/VB write hardware reverse-Z. The path-independent
screen-space input is `gViewT` (`R32_SFLOAT`, "the marcher-aligned surface ray param `t`",
`targets.rs:128-133`) + `generate_ray` (`ray_gen.hlsli:1-14`) — the seam SSAO, SSCS and TAA
share.

**But "path-independent" is true of the IMAGE and false of the PRODUCER** (critique B4, whose
error is seeded by the first draft's word "universal"). `viewt` is allocated on every path
(`targets.rs:128-133`), and exactly TWO passes write it, each path-specific: `viewt_from_depth`,
"armed iff `mesh_leg && !sdf_leg`" (`graph_bridge.rs:1319-1330`, Deferred), and `vb_viewt` =
`viewt_from_depth_rz.comp.hlsl` (`:3608-3609`, VB). `declare_forward_graph` (`:2585`) declares
**no** `viewt` write at all and asserts that fact in a tripwire — "the Forward family has NO AA
seam, so the VIEWT-variant marcher (whose `viewt` write only `declare_vb_graph` declares) must
never arm here … the Forward graph declares no viewt write for the marcher"
(`:2936-2941`, a `debug_assert!`); an `awk` over the Forward declarator's pass block (`:2585-3300`)
finds `viewt` only in that tripwire and in the ResId image list at `:3110`, and Forward's depth
prepass writes hardware reverse-Z, not `t` (`forward_opaque.vs.hlsl:16`). So any rung that reads
`gViewT` on Forward/ForwardPlus carries an unnamed prerequisite — a Forward `viewt` producer —
which the design books as RK-12. The decode is not new work: `viewt_from_depth_rz.comp.hlsl:5-12`
already inverts exactly Forward's encode (`forward_view_proj_rows`, `view_z = B / (d − A)`).

### 0.8 SDF and DDGI — what a shader can trace or sample today

- Any TU that declares `StructuredBuffer<uint> Buf : register(t0)` can `#include sdf_field.hlsli`
  and call `field_distance(p)` (`:13-18`, `:246`); the determinism contract is at `:21-31` (plain
  IEEE, no fast-math); `MAX_SDF_EDITS = 16` (`:48`). The marcher shape a reflection ray reuses is
  the eDSL-generated `sdf_soft_shadow_ranged` (`sdf_shadow_leaves.hlsli:49-51`) and `sdf_ao`
  (`:78`).
- The field is a **wholesale analytic CSG fold** per sample: `docs/SDF-PERF-AUDIT.md:82-85` puts
  a frame at ≈267 folds/pixel, "~4,300 primitive-distance evals per pixel" at `n = 16`, "~68,000"
  at a hypothetical `n = 256` — "the wall the deferred brick cache exists to remove". The brick
  atlas exists as M2/M3/M4 GPU code (`brick_atlas.rs:1-36`, `R8_SNORM`/`R16_SFLOAT` at
  `:129-133`, **NEAREST/clamp sampler** per BUG-M2-GPU-1 at `:80-84` — not the trilinear sampler
  the digest assumed) with a CPU oracle (`boyko_sdf_math/src/brick.rs:1-12`); the marcher's
  `field_skip` gateway is "Source-only for W0 — no shader yet calls it" (`sdf_field.hlsli:248-254`;
  `grep field_skip shaders/*.hlsl` is empty). The audit's load-bearing trade: bricking "rounds
  sharp CSG corners" (`SDF-PERF-AUDIT.md:247-252`).
- DDGI stores **diffuse irradiance only**: `B10G11R11_UFLOAT` octahedral 8×8 tiles (6×6 valid + a
  1-texel border), `Texture2DArray` with layer = probe Y, 16×8×16 = 2048 probes, `RG16F` two-moment
  depth, one linear non-comparison sampler (`ddgi.rs:1-36`); sampled at resolve bindings @16/17/18
  (`targets.rs:2527-2536`; `ddgi_probe_sample` at `ddgi_resolve.hlsli:107`); default disabled
  (`ddgi_config.rs:27-34`). `docs/RENDER-SDFDDGI-PLAN.md:23-24` is owner-locked "Diffuse-only now …
  Specular = a later cone-trace, accepting an atlas-format revisit then."

### 0.9 HW-RT: visibility-only inline ray queries behind `feature = "hwrt"`

The only `RayQuery` in the tree is `RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | FORCE_OPAQUE`
shadow visibility (`deferred_pbr.hlsl:1038-1042`, `:1073-1077`); no closest-hit, no hit distance,
no hit shading. `RayWorkload::Reflection = 3` is declared (`ray_backend.rs:67-76`) and the
routing comment says reflections "keep their software paths" (`:249-253`); `RtTier` at
`device.rs:178-191`; TLAS per FIF from a compute-packed instance array (`boyko_app/src/gpu_scene/
tlas.rs:8-22`; `accel_build.rs:202`, `:365`, `:453`). `docs/RENDER-HWRT-OPTIONAL-ANALYSIS.md:94-96`
rejects an RT pipeline/SBT by design; `:122` lists "Reflections (future) — HW-eligible — not a
current workload"; `:155-162` makes the HW path "effectively untestable in CI and permanently
owner-eval-only" (per-vendor tolerance, single box). `docs/RENDER-HYBRID-RAY-SYSTEM-DESIGN.md:205`
routes **Reflection/Mesh → HardwareTriBvh (hit-lighting) with SSR as the fallback**, at rung R4
(`:231`).

### 0.10 Frame graph, variants, storage rules

- The RDG is `crates/boyko_rhi_vulkan/src/framegraph/` (`mod.rs:1-20`); API `add_image` `:334`,
  `add_buffer` `:349`, `add_image_seeded` `:360`, `add_image_mipped` `:392`, `add_pass` `:459`,
  `image_access` `:496`, `compile` `:576`. Per-path declarators: `declare_frame_graph`
  (`graph_bridge.rs:1243-1260`) → `declare_deferred_graph` `:1288` / `declare_forward_graph`
  `:2585` / `declare_vb_graph`. ResIds are pinned in declaration order and a new image is
  appended LAST (`:1277-1285`); `FRAMEGRAPH_IMAGE_COUNT` is **22 under `hwrt`, 16 otherwise**
  (`:765`, `:770`, asserted at `:888-894`). Deferred pass line: … `ssao` `:1930`, `resolve` `:2305`,
  `taa_resolve` `:2481`, `present_sample` `:2520`; VB: `vb_sky` `:4585`, `vb_raster` `:4933`,
  `vb_cull_late` `:5100`, `vb_shade` `:5412`, `vb_geo` `:5632`, `vb_shade_split` `:5939`,
  `sdf_forward_march` `:6052`, `taa_resolve` `:6188`, `present_sample` `:6219`. No public node API
  exists: `docs/RENDER-GRAPH-API-PLAN.md` is CONVERGED (`:1-3`, `ExtensionPoint { AfterResolve,
  BeforePresent }` at `:87-89`, all transients FIF-slotted at `:161-166`) but no `rendergraph`
  module is on disk; remaining RDG work is Phase 2 aliasing and Phase 3 async compute
  (`docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md:1-8`).
- Variant discipline: `docs/SHADER-VARIANT-MANIFEST.md` rows per `-D` interface variant
  (`deferred_pbr.hlsl` already has six rows, `:26-39`); spec constants exist (`GI_MAX_IT`,
  `compute.rs:691-707`). Seventeen `*_sync.rs` tests gate committed `.spv`
  (`crates/boyko_rhi_vulkan/tests/`, listed in §8). The eDSL can EMIT a texture sample only
  through an emit-only `ResRef` node whose f32 oracle cannot run (`emit/mod.rs:529-535`).
- Config pattern: `SsaoConfig` (owner-set `Resource`) + `ResolvedSsao` (derived carrier) +
  `resolve_ssao_policy` (`ssao_config.rs:1-14`, `:113`, `:177`, `:210`, `:237`); `CsmConfig` copies
  it (`csm_config.rs:1-8`). Mode gates ride a light-header spare word or a `ResolvedX` UBO.
- Render path and legs are boot-frozen; "a live per-frame path/leg toggle is forbidden by design"
  (`docs/FEATURE_MAP.md:122`).
- Gaia: `data` profile `table` bakes rows into ONE archetype column (`docs/gaia/LANGUAGE.md:130-135`);
  references are name hashes (`DECISIONS.md:96-104`); bake-time-only reflection (`:11-15`);
  `BOYKOSAV` v2 has a dense region but **no asset-blob region** (`crates/boyko_serialize/src/
  format.rs:22-34`); non-Gaia binary assets carry a sidecar id with a hard fail on absence
  (`DECISIONS.md:123-127`). `docs/animation/ANIMATION-DESIGN-SPACE.md` §11 AK-6 already requests
  the blob region — a baked LUT / prefiltered environment is the same class.
- The image golden harness: `docs/FEATURE_MAP.md:1047-1062` (`golden.ps1`, `goldens/PINS.toml`,
  the `goldens.rs` host oracles, `CHANNEL_TOL`).

## 1. Systems surveyed

| System | What was read | Tag |
|---|---|---|
| Filament | `surface_light_indirect.fs`, `surface_light_reflections.fs`, `surface_ambient_occlusion.fs`, `libs/ibl/src/CubemapIBL.cpp`, issue #875; the PBR document | [S] / [D] |
| Bevy (`main`, 0.17-era) | `light_probe/{mod.rs, environment_map.rs, environment_map.wesl, environment_filter.wesl, generate.rs}`, `ssr/mod.rs`, PR #19076, PR #13418, PR #7051, issue #14639, docs.rs | [S] / [D] |
| Godot 4.6 | `ss_effects.cpp`, `screen_space_reflection.glsl`, PR #111210 (SSR rewrite), PR #100241 (probe priority), PR #69514 (roughness remap), docs (Sky, ReflectionProbe, SDFGI) | [S] / [D] |
| Unity | SRP core `ImageBasedLighting.hlsl`; HDRP docs (Reflection hierarchy, SSR override, RT reflections, HDRP asset, probe performance); HDRP `reference-screen-space-reflection.md` | [S] / [D] |
| Unreal | Reflection captures, environment, planar, SSR, Lumen (overview, technical details, performance guide — the last 403'd and is second-hand); UE4 `ScreenSpaceReflections.usf` via a mirror | [D] / [S] |
| Frostbite | Lagarde 2012 parallax-corrected cubemaps; Lagarde & de Rousiers 2014 (course notes not fully opened); Stachowiak 2015 stochastic SSR (abstract + author page) | [B] / [D] |
| AMD | FidelityFX SSSR 1.5 manual; Hybrid Stochastic Reflections; interplayoflight integration notes (the only per-pass timings on a comparable GPU) | [D] / [B] |
| NVIDIA | NRD README (REBLUR / RELAX / SIGMA, inputs, perf table) | [D] |
| Papers | Karis 2013; Fdez-Agüera 2019; Kulla-Conty 2017; Turquin 2019; McGuire & Mara 2014; Uludag GPU Pro 5; Hofmann et al. HPG 2017; Heitz 2018 VNDF; Schied et al. SVGF 2017; Ouyang et al. ReSTIR GI 2021; Tokuyoshi & Kaplanyan 2019; Engelhardt & Dachsbacher 2008; Cocco et al. 2024; Hirvonen et al. RTG ch.32; Lengyel oblique clipping; Radiance RGBE spec; KTX2 spec | [D] |

## 2. Technique catalogue

Each entry: what it needs, what it costs, how it fails, who ships it, source.

### 2.1 The IBL foundation — "what makes metal look like metal"

**T1 Split-sum specular IBL** (Karis 2013 [D]). Factor ∫L·f·cos into a prefiltered radiance map
(mip k = GGX convolution at roughness r(k)) × a pre-integrated 2-term BRDF `(scale, bias)` on
`f0`. Needs a mip-chained environment image and a DFG term. ~2 fetches/px. Dominant error is the
`N = V = R` assumption (stretched grazing reflections are lost). Ships in UE, Filament, Unity,
Godot, Bevy. This tree ships the DFG half only, as a polynomial (§0.2).

**T2 DFG / environment-BRDF LUT.** 2D table indexed `(NoV, perceptualRoughness)` → `(scale,
bias)`. Filament: `textureLod(iblDFG, vec2(NoV, pR))` [S]; default size **128**, RG16F after
RGB16F proved "widely unsupported" (issue #875 [S/D]); cmgen bakes with `coord =
sqrt(linear_roughness)` [S CubemapIBL.cpp]; `DFV_Multiscatter` and `DFV_Charlie_Uniform` (4096
samples) variants. Unity ships `PreIntegratedFGD` as a texture [S]. Bevy and this tree ship an
analytic fit instead. Cost 1 bilinear fetch, ~64 KB. Failure: a bake/read parameterisation
mismatch (roughness vs α vs √α) silently biases every metal.

**T3 Analytic env-BRDF** (Karis mobile / Narkowicz). ~10 ALU, zero memory; error grows at low
`NoV` and high roughness. Bevy `environment_map.wesl` [S]; this tree `deferred_pbr.hlsl:582-589`
[T]. The important consequence: **any energy-compensation term built on it divides by the fit**,
so the fit's error is amplified, not averaged (§5 P8).

**T4 Roughness → LOD mapping.** Filament `lod = iblRoughnessOneLevel · pR · (2 − pR)` (quadratic,
"works very well for a 256 cubemap with 5 levels") [S]; Unity `pR·(1.7 − 0.7·pR)·maxMip`,
`UNITY_SPECCUBE_LOD_STEPS = 6` [S]; Bevy `pR·(numLevels − 1)` (linear) [S]; Godot bakes the
non-linear mapping into coefficients and had to `sqrt` on read (PR #69514 [D]). **The read
mapping must be the exact inverse of what the prefilter baked** — three engines use three
different maps.

**T5 Prefiltered radiance chain + filtered importance sampling.** Per mip, GGX-importance-sample
the source with a per-sample source LOD from the solid-angle ratio: Filament `lod =
log4(K·Ωs/Ωp)`, `K = 4`, trilinear blend `l0/l0+1` [S CubemapIBL.cpp]; Unity `mip =
0.5·log2(ΩS·invΩP) + roughness`, 34 (mobile) / 55, 89, 89, 89 samples per level [S]; Bevy `lod =
0.5·log2(ω_s/ω_p)`, `32 · 2^(roughness·4)` samples, mirror shortcut below roughness 0.01 [S
environment_filter.wesl]. Without it a 32-sample convolution over an HDR sun produces fireflies
that persist in every rough mip — which is why **the source needs its own full mip chain before
convolution** (Bevy's SPD pass). Sampling: Hammersley + Cranley-Patterson rotation, VNDF /
spherical-cap GGX [S Bevy; D Heitz 2018].

**T6 Runtime GPU environment filtering** (Bevy PR #19076 [S/D]): copy → single-pass downsample
(mips 0-5, then 6-12) → GGX radiance convolution per mip → Lambertian irradiance into a fixed
**32×32** cube (1024 samples), all `Rgba16Float`, **every frame** while the component exists.
Needs storage-image views per mip and an SPD-style pass. Enables dynamic skies; the cost is a real
per-frame budget that scales with cube resolution × sample count.

**T7 Offline prefilter bake** (glTF IBL Sampler [D]: Lambertian / GGX / Charlie, default 1024
samples, KTX2 out, LUT emitted; Filament cmgen [S]). Zero runtime cost; cannot follow a dynamic
sky. Bevy's documented default workflow [D docs.rs]; UE reflection captures are static at
runtime [D].

**T8 Irradiance: SH-9 vs a small cubemap.** Filament `CONFIG_SH_BANDS_COUNT ∈ {1,2,3}`, `max(sh,
0)` [S]; UE skylight, Unity. 27 floats, ~20-30 ALU, no texture; ringing on high-contrast sources
needs windowing. Bevy's runtime path instead produces a 32×32 irradiance cube [S generate.rs] —
one fetch, no ringing, needs the texture shape.

**T9 Multiple-scattering energy compensation.** Single-scatter GGX loses up to ~40 % at high
roughness; the failure is real and repeatedly rediscovered (glTF-Sample-Viewer #43 "Rough metal
looks too dark" [S/D]). Filament `energyCompensation = 1 + f0·(1/dfg.y − 1)` on a real LUT [D
Filament.md]; Bevy `compute_multiscatter()` (FssEss / FmsEms / Edss) per Fdez-Agüera JCGT 8(1)
[S]; Kulla-Conty 2017 [D], Turquin 2019 [D]. This tree: `1 + f0·(1/(dfg.x + dfg.y) − 1)` on the
analytic fit [T]. The mechanical check is a **white-furnace test**, cheap on a host oracle.

**T10 Specular occlusion.** Filament `SpecularAO_Lagarde` (this tree, verbatim [T]) and the
cone/GTAO variant `SpecularAO_Cones` which needs a **bent normal** the current SSAO pass does not
produce [S]; Bevy's artistic `saturate(dot(F0, 50·0.33))` [S]. Multi-bounce AO (Jimenez 2016
`gtaoMultiBounce` cubic) [S Filament]. Horizon occlusion for normal-mapped surfaces (Lagarde &
de Rousiers 2014 [D]).

**T11 Specular dominant direction.** Unity `GetSpecularDominantDir` lerps `N → R` by roughness
before the env fetch [S]; a few ALU; without it rough reflections sit off-peak. This tree does
not apply it.

**T12 Clear coat / anisotropy / sheen IBL.** Filament: coat = second lobe, own normal, `F_Schlick
(0.04, 1, ccNoV)`, base attenuated by `(1 − Fc)`; anisotropy is "faux" (bent reflection vector —
"doesn't work with prefiltering"); sheen = `sheenDFG · sheenColor · specularAO · prefiltered
Charlie map` [S]. The accurate anisotropic route is BRDF major-axis sampling (Cocco et al. CGF
2024 [D]). Each needs a material lane this tree's 48-B `MaterialGpu` (three `vec4`s,
`material.rs:61-70`) does not have; `mrr.w = flags` is the only reserved bit-field.

**T13 Geometric specular anti-aliasing** (Kaplanyan 2016; Tokuyoshi & Kaplanyan 2019 [D]): widen
α from screen-space normal derivatives; ~10 ALU; needs `ddx/ddy`, absent in the compute resolve
— the same unsolved sub-problem `PBR-MATERIALS-PLAN.md:394` (T3 "compute LOD", W3) already flags.

**T14 Roughness clamping.** Filament clamps perceptual roughness at 0.089 (fp16 → α 6.274e-5)
[D]; this tree clamps α's source roughness at **0.045** in fp32 (`deferred_pbr.hlsl:800`;
`PBR-MATERIALS-PLAN.md:149`).

**T15 HDR transport.** Radiance RGBE `.hdr`: ASCII header, `-Y M +X N` resolution line,
adaptive RLE with the `(2, 2, hi, lo)` new-format marker plus the old flat/old-RLE scanline
forms [D LBL picture_format]. ~200 lines in-house. EXR is a real codec (zip/piz) — not the cheap
door. KTX2 + BC6H (or UASTC-HDR → BC6H via Basis [D]) needs a BCn `Format` family the tree lacks.

**T16 Octahedral environment maps** (Engelhardt & Dachsbacher 2008 [D]; HDRP moved its probe
cache to a 2D octahedral atlas in HDRP 14 [B changelog]). One square 2D image per environment
instead of six faces; reuses this tree's `oct_encode`/`oct_decode`; costs explicit border /
gutter texels per mip and seam-aware filtering that cube hardware gives for free.

**T17 Storage as bindless slots / arrays.** Bevy packs probe cubemaps into **texture binding
arrays** and disables probes entirely where `TEXTURE_BINDING_ARRAY` + non-uniform indexing are
unavailable (WebGL2/WebGPU, Adreno ≤ 610) [S light_probe/mod.rs]; HDRP uses one 2D atlas for cube
+ planar probes with "last valid mip" padding and "Decrease Reflection Probe Resolution To Fit"
[D]; UE a cubemap array, 128 px faces by default, `r.ReflectionCaptureResolution` 16…1024, **≤ 341
enabled** [D]. This tree's bindless table is `Texture2D gTextures[]`, 4096 slots,
`PARTIALLY_BOUND | UPDATE_AFTER_BIND`, bound only on the textured raster pipeline
(`boyko_rhi_vulkan/src/bindless.rs:1-24`, `:72`) — not on the resolve, whose bindings are
explicit (`targets.rs:2525-2536`).

### 2.2 Local probes, parallax, blending, capture

**T18 Parallax-corrected cubemaps** (Lagarde & Zanuttini, SIGGRAPH 2012 Talks [D]; formulas [B/S
seblagarde]): AABB — `FirstPlaneIntersect = (BoxMax − P)/R`, `SecondPlaneIntersect = (BoxMin −
P)/R`, `Furthest = max(…)`, `Distance = min(F.x, F.y, F.z)`, `R' = (P + R·Distance) −
CubemapPosition`; OBB by transforming into unit-box space; sphere analogously. Valid only for
normals near the reflection plane (distortion at oblique normals); UE prefers sphere captures,
box for hallways/rooms [D]. Stated PS3/X360 costs: ~0.08 ms per extra cubemap in the mixing-step
form, 0.25 / 0.75 ms per cubemap at 25 % / 75 % coverage for per-pixel correction [B].

**T19 Probe blending.** Lagarde: normalised-distance-field weights with the constraints "100 %
at a primitive's centre regardless of overlap, 0 % at its boundary", then normalised [B/S].
HDRP: accumulate weights until 1, smaller influence volume = higher priority, per-probe manual
Weight, fall back to a lower-priority probe [D]. UE: smaller captures override larger [D].
Godot: ≤ 4 blended per pixel in Forward+ (8 mobile, 2 compatibility); PR #100241 added size
priority sorting with early-out when `reflection_accum.a ≥ 1`, up to 32 overlapping probes [S/D].
Bevy: frustum cull, sort by camera distance, cap `MAX_VIEW_LIGHT_PROBES = 8` "because the
fragment shader does a linear search through the list for each fragment", accumulate to
`total_weight`, fall back to the view environment map [S]. **Order and termination are
correctness** (pre-#100241 Godot showed "duplicate reflections").

**T20 Capture / update budget.** Unity: a realtime probe update is **9 frames** ("All Faces at
Once") or **14 frames** ("Individual Faces") including per-mip convolution [D]; UE captures are
baked, "static at runtime" [D]; Godot `UPDATE_ONCE` / `UPDATE_ALWAYS` into a shared atlas [D];
Bevy per-frame GPU filtering (T6). Time-slicing = the probe lags lighting by that many frames.

**T21 Planar reflections.** A second full scene view from the mirrored camera with the mirror
plane as an oblique near plane (Lengyel [D]). UE: "budget half your frame time"; Kite demo
**+23.07 ms** on a 31 ms scene, Infinity Blade Dungeons **+1.67 ms** on 11 ms; screen-percentage
and prefilter-roughness mitigate; multiple planars discouraged [D]. HDRP stores planar probes in
the same 2D atlas as cube probes [D].

### 2.3 Screen-space reflections

**T22 Linear / DDA march** (McGuire & Mara JCGT 3(4) 2014 [D]): perspective-correct 2D DDA over
depth with an explicit thickness model. Bevy: `linear_steps 10`, `bisection_steps 5`,
`use_secant`, `thickness 0.25`, roughness fade `0.08..0.12` in / `0.55..0.6` out, edge fadeout;
deferred-only (`DepthPrepass` + `DeferredPrepass`), runs after deferred lighting; PR #13418: no
hi-Z, no temporal/spatial filtering, full-res [S/D]; stochastic SSR is an open request (#14639
[S]). UE4 `ScreenSpaceReflections.usf`: linear `RayCast`, quality tiers `NumSteps/NumRays` =
8/1, 16/1, 8/4, 12/12, `ImportanceSampleBlinn`, `RoughnessFade = min(Roughness·SSRParams.y + 2,
1)`, samples scene colour with a tonemap-weighted average [S]. Failure: stair-stepping, missed
thin geometry, streaks at grazing angles.

**T23 Hi-Z / hierarchical tracing** (Uludag, GPU Pro 5 [D]). Traverse a depth pyramid: cross a
cell boundary with an epsilon, descend on hit, ascend on miss, thickness test at mip 0 only
[B sugulee]; needs the **nearest-surface** pyramid. Godot 4.6 [S `screen_space_reflection.glsl`,
PR #111210]: `source_hiz` `R32_SFLOAT` linear depth, `mip_offset = hit ? −1 : +1`, `(z0 − z1) >
depth_tolerance` at mip 0, **roughness ≥ 0.7 skips tracing**, cone-angle → blur radius →
`mip_bias = log2(blur·max(screen)/16 + 1)`, 5 % screen-margin fade, backface rejection,
confidence in alpha, `source_last_frame` colour, half-res with a surface-aware upsample; RTX 3070
Ti / Sponza / 64 steps: upstream 3.72 ms, PR half-res **3.10 ms**, full-res 4.28 ms [S PR].
FidelityFX SSSR: 7-mip hierarchy via SPD, `FFX_SSSR_ENABLE_DEPTH_INVERTED` for reverse-Z [D].
The GPU Pro 5 chapter itself notes the Hi-Z/cone tension: Hi-Z wants the largest steps, a cone
tracer needs linear front-to-back integration [D].

**T24 Stochastic SSR** (Stachowiak, SIGGRAPH 2015 Advances [D]; author page [B]): GGX/VNDF
importance-sampled rays at **half resolution**, hit reuse across neighbouring pixels in a
full-res BRDF-weighted resolve ("a ratio estimator all along"), spatial + temporal filtering;
shipped in Mirror's Edge Catalyst and Need for Speed. PICA PICA traces indirect specular at half
res and reconstructs at full [D EA SEED, search-summarised]. AMD SSSR 1.5 [D] is the documented
production form: SPD depth hierarchy → **tile classification** (8×8; pixels above
`roughnessThreshold` skip rays and sample the environment map; `samplesPerQuad`;
temporal-variance-guided escalation to 4 rays/quad) → 128×128 blue noise from Sobol + scrambling
tiles → indirect args → intersection → denoise (reproject → prefilter → resolve-temporal).
Required inputs: depth, normals, roughness, motion vectors, **HDR colour with direct lighting**,
environment cubemap, BRDF LUT, variance history; ~13 internal fp16 targets [B].

**T25 Contact hardening / cone blur.** Godot outputs an `output_mip_level` from the roughness
cone angle for a following filter pass [S]; HDRP "reads a color buffer with a blurred mipmap
generated during the previous frame" [D]. Needs a mip-chained colour buffer; bleeds across depth
discontinuities.

**T26 Fades and thickness.** Godot 5 %-margin `smoothstep`, distance fade, `depth_tolerance` at
mip 0 [S]; Bevy `edge_fadeout`, `thickness 0.25` [S]; SSSR exposes a depth-thickness "to prevent
thin geometry over-occluding" and an "exit due to low occupancy" wave cutoff [B]. Too thin
tunnels, too thick smears.

**T27 Multi-layer screen-space tracing** (Hofmann, Bogendörfer, Stamminger, Selgrad, HPG 2017
[D]): several depth layers traversed multi-resolution; the academic answer to single-layer
occlusion misses; needs depth peeling (memory + a second geometry pass).

**T28 Specular history reprojection.** Surface motion vectors are wrong for reflections
(view-dependent): the correct route reprojects the virtual reflected point through the hit
distance; production picks the smallest-magnitude candidate among {motion at incidence, motion at
the hit, parallax-corrected variants} [B jpgrenier, crediting Stachowiak]. NRD does surface
motion AND virtual motion, blended by roughness toward surface motion at roughness 1 [D/B].
DNSR reconstructs the 3D hit position "to find where the reflected object was last frame"
[B interplayoflight].

**T29 Colour source and composition.** Filament traces against the structure (depth) buffer,
reprojects the hit and samples `sampler0_ssr` = the previous frame; alpha = confidence (edge,
distance, backface, thickness); the IBL site blends `Fr = Fr·(1 − ssrFr.a) + E·ssrFr.rgb` and
skips the IBL entirely when `a == 1`; SSR is gated to `pR < sqrt(0.5)` [S]. HDRP: SSR → probes →
sky, "until it reaches an overall weight of 1", sky's weight fixed at 1 [D]. Godot 4.6 replaced
its hard mask with a smooth distance-based blend into probe/SDFGI [S PR, B]. UE composites SSR
over captures over skylight [D].

**T30 SSR under a visibility buffer.** No engine runs SSR directly off a VB; VB pipelines
materialise a G-buffer in the material pass so screen-space effects keep working [B
filmicworlds]. This tree's VB split materialises thin-aux only, never albedo
(`MULTI-PARADIGM-RENDER-PLAN.md:280`, `:389`) — enough for the RAY stage; colour must come from a
lit buffer, so SSR's placement relative to `vb_shade` is a correctness constraint.

### 2.4 Traced reflections — SDF, brick, cone, hardware

**T31 SDF sphere-traced reflections on the analytic fold.** Reuse the existing marcher shape for
SSR-miss rays; no new acceleration structure; cost = steps × `O(edits)` (§0.8). SDF legs only —
mesh geometry is invisible to the field.

**T32 Brick-atlas / global-distance-field tracing** (Lumen software RT: per-mesh SDF for the
first ~2 m, merged Global Distance Field for the rest [D]). Converts `O(edits)` to `O(1)`
trilinear fetches; rounds sharp CSG (§0.8). AMD Brixelizer is the compute-only cascaded-SDF
analogue [D via LIGHTING-PLAN].

**T33 SDF / voxel cone tracing for glossy.** One cone per pixel, aperture from roughness,
coarser mips as the footprint grows; **noiseless — no denoiser**; collapses to ray marching at
roughness → 0 [D OGRE VCT; D Hermanns/Franke SSCT]. Godot's SDFGI supplies "sharp and rough"
specular alongside GI [D]. This tree's SDFDDGI plan owner-locks specular as "a later cone-trace"
[T]. Needs a directional radiance volume or a radiance-carrying brick atlas.

**T34 HW-RT reflections, cache lighting vs hit lighting.** Lumen: screen traces first, then SW
or HW rays; HW "has the option to evaluate lighting at the ray hit instead of the lower quality
Surface Cache" (`Ray Lighting Mode = Hit Lighting for Reflections`) [D]; `MaxRoughnessToTrace`
default **0.4**, `r.Lumen.Reflections.DownsampleFactor ∈ {1, 2}` + checkerboard,
`RadianceCache = 1` reuses diffuse-GI rays for roughness 0.2-0.4, `Allow = 0` reverts to SSR and
"saves 1 ms on Xbox Series S" [D performance guide — page 403'd, second-hand]. HDRP: Tracing =
Ray Marching | Ray Tracing | Mixed (`Max Mixed Ray Steps`); Performance mode `Full Resolution` off
= one ray per four pixels; RT reflections "replace the Screen Space Reflection override";
clear-coat traces the coat only [S/D]. AMD Hybrid Stochastic Reflections: SSSR replaces RT
intersection where it can, per-pixel feedback, FSR1 upscale of a lower-res reflection buffer [D].
Battlefield V applied RT reflections only to reflective surfaces [D GDC, not opened].

**T35 Radiance-cache tail** (Hirvonen et al., RTG 2019 ch.32 [D, sample HLSL public]; Lumen
radiance cache [D]): decouple radiance computation from shading; the roughest surfaces are
approximated from the cache with no rays. Requires **directional radiance**, which DDGI
irradiance octahedra are not (DDGI "terminates paths into the irradiance probes at their first
diffuse interaction" — [D] abstract of ACM 10.1145/3675249.3675304; page 403'd).

**T36 Far-field retrace** (`r.Lumen.Reflections.HardwareRayTracing.Retrace.FarField` [B]): a
second trace after a near-field pass; needs a second TLAS or proxy scene.

### 2.5 Denoisers and sampling

**T37 SVGF** (Schied et al., HPG 2017 [D]): temporal accumulation + spatiotemporal luminance
variance driving an à-trous wavelet; 1920×1080 in ~10 ms from 1 spp on 2017 hardware, ~10× more
temporally stable than prior filters. Needs variance, normals, depth + gradient, motion vectors,
material demodulation. This tree's shadow denoiser (`RENDER-RUNG3A/3B`) is structurally SVGF
minus the variance channel, for a **scalar** signal.

**T38 NRD** (README [D]): REBLUR (recurrent blur, hit-distance-driven radius, ≤ 63 frames),
RELAX (à-trous, separate diffuse/specular, ≤ 255 frames), SIGMA (shadows). **RTX 4080 @ 1440p:**
`REBLUR_DIFFUSE_SPECULAR` 2.55 ms (3.40 SH), `RELAX_DIFFUSE_SPECULAR` 3.25 ms (4.80 SH),
`SIGMA_SHADOW` 0.40 ms; REBLUR ds working set **262.56 MB**. Hard requirements: motion vectors,
world normal + **linear** roughness, viewZ, a **hit distance "within the specific BRDF lobe being
denoised"**, normalised, excluding the primary hit; **non-jittered matrices**; **materials
demodulated before denoising**; specular sampled with **VNDF v3**; blue noise ≥ 64×64
(Owen-scrambled Sobol). Anti-firefly costs 0-2 % (REBLUR), 7-10 % (RELAX).

**T39 FidelityFX DNSR** (SSSR's denoiser [D]; timings [B]): reproject (0.66 ms) → prefilter
(0.34 ms, 16 Halton taps weighted by normal/depth/radiance/variance) → resolve-temporal (0.53 ms)
on an RTX 3080 Mobile @ 1080p; variance stabilisation interpolates toward new values after
disocclusion.

**T40 Material demodulation, hit-distance encoding, blue noise, firefly clamp.** Divide out
albedo/F0 before filtering and re-multiply after, or texture detail smears into the reflection
[D NRD]. Hit distance drives blur radius, virtual motion and disocclusion — a hole in it corrupts
specular tracking [D NRD]. SSSR generates 128×128 blue noise per frame [D]; white noise converges
visibly slower. HDRP exposes `Clamp Value` and `Anti-flickering Strength` [S].

**T41 ReSTIR GI** (Ouyang et al., CGF 2021 [D]): reservoir resampling across space/time;
"improves the quality of glossy reflections and refractions"; reservoir buffers per pixel, MIS
weights; research rung.

**T42 Radiance Cascades** (Sannikov; Path of Exile 2 [B radiance.wiki]): penumbra-hypothesis
discretisation, geometry-agnostic constant cost; 3D specular use is research; named in this
tree's SDFDDGI plan as the fallback re-research [T].

### 2.6 Transparency

HDRP: only transparents in the `BeforeRefraction` pass appear in the colour pyramid; `Receive SSR
Transparent` forces the *Approximation* algorithm; SSR runs before transparent depth is written,
so transparents reflect using the depth **behind** them; `Low Resolution` transparents cannot
receive SSR [D]. UE: Lumen high-quality translucency reflections; single-layer water "forced to
mirror" [D]. Godot SSR is opaque-only (an open proposal asks for transparent support) [D]. Bevy:
SSR is deferred-only; forward materials get probes/env only [S]. Filament: opaque/masked only [S].
Practically: transparents get probes/IBL in the forward pass, and at best a degraded SSR.

## 3. Comparative table

| Aspect | Filament | Bevy (main) | Godot 4.6 | Unity HDRP | Unreal | AMD SSSR |
|---|---|---|---|---|---|---|
| DFG term | 128² RG16F LUT [S] | analytic + multiscatter [S] | — (Godot has its own) | `PreIntegratedFGD` LUT [S] | LUT (Karis) [D] | app-provided BRDF LUT [D] |
| Roughness→LOD | `pR·(2−pR)·oneLevel` [S] | linear [S] | Activision coeffs, `sqrt` on read [D] | `pR·(1.7−0.7pR)·6` [S] | mip per roughness [D] | — |
| Prefilter | cmgen offline, FIS `log4(4Ωs/Ωp)` [S] | offline (IBL Sampler) OR per-frame GPU SPD+VNDF [S] | runtime, `ggx_samples` [D] | baked, compressible [D] | baked captures [D] | — |
| Diffuse | SH 1-3 bands [S] | 32² cube (runtime) [S] | radiance map [D] | — | skylight SH [D] | — |
| Fallback chain | SSR(`a`) → IBL [S] | probes (weight→1) → view env; SSR separate [S] | SSR → probe/SDFGI (smooth blend) → sky [S/B] | SSR → probes → sky, weight→1 [D] | Lumen/SSR → captures → skylight [D] | SSR → env map (confidence) [D] |
| SSR trace | linear, prev-frame colour [S] | linear + bisection + secant, full-res [S] | Hi-Z, prev-frame colour, half-res [S] | prev-frame blurred pyramid, 2 algorithms [D] | linear 8-16 steps, current SceneColor [S UE4] | Hi-Z 7 mips, HDR radiance [D] |
| Roughness cutoff | `pR < √0.5` [S] | 0.55..0.6 fade [S] | ≥ 0.7 skip [S] | Minimum Smoothness [D] | Max Roughness ~0.8 (SSR), 0.4 (Lumen) [D] | `roughnessThreshold` [D] |
| Stochastic + denoise | no [S] | no (issue #14639) [S] | cone-blur mip, no stochastic [S] | PBR Accumulation mode [D] | Lumen denoiser [D] | VNDF + blue noise + 3-pass DNSR [D] |
| Probe storage | single IBL [S] | cubemap binding array, ≤ 8/view [S] | shared atlas, ≤ 4 blended [D] | 2D (octahedral) atlas, cube + planar [D/B] | cubemap array, ≤ 341, 128 px [D] | app cubemap [D] |
| Parallax | — | `ParallaxCorrection` [S] | box projection [D] | proxy volumes [D] | sphere/box [D] | — |
| Transparents | opaque only [S] | forward gets env; SSR deferred-only [S] | opaque only [D] | BeforeRefraction only, Approximation forced [D] | Lumen HQ translucency; water = mirror [D] | — |

## 4. Cost model — every number is someone else's GPU

| Item | Figure | Source | Hardware |
|---|---|---|---|
| SSSR pass list @1080p | SPD 0.12, classify 0.17, blue noise < 0.01, **intersect 0.88**, reproject 0.66, prefilter 0.34, temporal 0.53 → **≈ 2.70 ms, denoiser 1.53 ms (57 %)** | [B] interplayoflight | RTX 3080 Mobile |
| NRD diffuse+specular @1440p | REBLUR 2.55 ms (SH 3.40), RELAX 3.25 ms (SH 4.80); SIGMA shadow 0.40 ms; REBLUR ds 262.56 MB | [D] NRD README | RTX 4080 |
| Godot 4.6 SSR, Sponza, 64 steps | upstream 3.72 ms; PR half-res 3.10 ms; PR full-res 4.28 ms | [S] PR #111210 | RTX 3070 Ti |
| SVGF | ~10 ms at 1080p from 1 spp | [D] Schied 2017 | 2017 hardware |
| UE planar reflections | +23.07 ms on a 31 ms frame (Kite); +1.67 ms on 11 ms (IBD) | [D] UE docs | unspecified |
| Lumen `Allow = 0` → SSR | "saves 1 ms" | [D] second-hand | Xbox Series S |
| Lagarde parallax cubemaps | 0.08 ms/cubemap (mixing step); 0.25 / 0.75 ms per cubemap at 25 / 75 % coverage | [B] | PS3 / X360 |
| Unity realtime probe update | 9 frames (all faces) / 14 frames (individual faces) | [D] | — |
| DFG LUT | 128×128 RG16F = 64 KB, one bilinear fetch | [S] Filament | — |
| Filament prefilter | tuned for 256 px cube, 5 usable levels | [S] | — |

**No published per-pass reflection figure for an RTX 3060 was found**; the 3080-Mobile and
3070 Ti numbers are the nearest anchors and are NOT a prediction for the owner's box.

## 5. Pitfalls register

Numbered so the design can cite them.

- **P1 Bake/read roughness mapping mismatch.** Godot shipped a square-on-bake / linear-on-read
  pair and surfaces read "a much more reflective mipmap than intended" (PR #69514 [D]). Filament
  (quadratic), Unity (1.7 − 0.7 fit) and Bevy (linear) differ; a prefilter borrowed from one and a
  lookup from another is silently wrong. Fix class: ONE function, used by both bake and read, with
  a host oracle.
- **P2 Perceptual roughness vs α in the LUT index / LOD map** (Filament #5421 exists because of
  it): the LUT is indexed by `pR`, the BRDF uses `α = pR²`.
- **P3 Prefiltering without filtered importance sampling** — fireflies from an HDR sun survive
  into every rough mip; the fix needs the SOURCE mip chain first.
- **P4 Too few levels / too small a base** — roughness quantises into bands; Filament's quadratic
  map is tuned for 256 px / 5 levels.
- **P5 Seams.** Cube-face trilinear filtering is undefined without seamless-cubemap support;
  octahedral maps trade that for explicit gutters per mip, and auto-generated mips pull gutter
  texels into the interior [B marbarod]. The DDGI atlas's 1-texel border rule must be checked for
  generalisation to a many-mip chain.
- **P6 AO applied to ambient specular** reads as "AO-darkened matte paint"; the engine already
  decoupled `spec_ao` (§0.2) and any new IBL path must keep the split.
- **P7 Ignoring multiple scattering** — rough metal dark and desaturated (glTF-Sample-Viewer
  #43). Furnace test.
- **P8 Energy compensation on an analytic DFG** — division amplifies the fit's error at low
  `NoV` / high roughness. The engine does this today.
- **P9 "Raise the ambient until it looks right" is not a fix** — memory
  `project-pbr-quality-diagnosis` records the 2026-07 brightening of a flat hemisphere; it restores
  midtones but cannot produce the spatially varying reflection that reads as metal, and inflates
  diffuse for every dielectric.
- **P10 Trusting a filename / vendor label for a convention** — memory
  `reference-normal-map-convention` (a map named `-ogl` measured as DirectX). HDR environments
  have the same trap class: equirect orientation, Y-up vs Z-up, exposure pre-scaling.
- **P11 Sun triple-count** — the tree carries the sun as direct + env sun-disc **by design**
  (`deferred_pbr.hlsl:1138-1141`); a prefiltered environment that CONTAINS the sun makes it a
  triple count unless the disc is retired or the sun is removed at bake.
- **P12 Format-support assumptions** — Filament's RGB16F LUT was "widely unsupported" (#875);
  this tree's `DeviceCaps::*_storage_ok` probes (`device.rs:255-264`) are the house rule.
- **P13 fp16 range** — `Rgba16Float` radiance inherits half-float limits on very bright suns.
- **P14 Runtime filtering assumed free** — Bevy re-filters EVERY frame; amortising across frames
  lags reflections, which TAA then smears.
- **P15 Sampling a tonemapped, gamma-encoded buffer as scene colour.** `lit` is exactly that
  (§0.4). Highlights are compressed twice; reflections of bright windows lose their energy and
  cannot be re-linearised exactly.
- **P16 Reusing the occlusion HZB for reflections.** Farthest-surface polarity (§0.7); rays step
  over near occluders and hit behind walls — "wrong only in motion", the class this repository
  catalogues.
- **P17 Assuming `taa_hist` is a general previous-frame colour** — `None` when TAA is off,
  post-tonemap content, reset on resize.
- **P18 Surface motion vectors for reflection history** — smears under camera motion (T28).
- **P19 Reusing the scalar shadow denoiser for radiance** — no demodulation, no hit distance, no
  virtual motion; 3a/3b are a good STRUCTURAL template (variant-split shaders, ringed targets,
  ResId append, byte-identity gates) and a bad ALGORITHMIC one.
- **P20 Denoising without demodulation / without VNDF** — silent: the denoiser does not report a
  violated variance model.
- **P21 Jittered matrices reaching the denoiser** — NRD forbids; the TAA path keeps an
  UNJITTERED camera ring at `taa_resolve_set` @6 (`targets.rs:494-498`) — extend, do not re-derive.
- **P22 Under-costing the denoiser** — 57 % of SSSR's total on the one comparable measurement.
- **P23 Under-costing memory** — 13 internal SSSR targets; 262 MB REBLUR; interacts with FIF
  ringing.
- **P24 DDGI probes as a glossy source** — cosine-convolved irradiance; roughness ≈ 1 tail only.
- **P25 No roughness gate** — Lumen's single biggest saving is not tracing above 0.4.
- **P26 Full-res tracing because half-res "looks noisy"** — every shipping system traces at
  half/quarter and spends the saving on reconstruction.
- **P27 Two accumulators over one signal** — reflection temporal + TAA compound ghosting; the
  reflection history must be a separate ring with its own reprojection.
- **P28 Thickness as one global constant**; **P29 Hi-Z boundary epsilons and origin
  parameterisation** [B sugulee]; **P30 HLSL `min()`/`max()` NaN inversion** (`hzb_build.comp.
  hlsl:122-130`; memory `reference-nan-inverts-under-nmin-nmax`); **P31 Hi-Z/cone conflict** [D].
- **P32 No energy match at the SSR/probe seam** — the cutoff roughness becomes a ring unless the
  miss blends with the probe under the same BRDF weighting (HDRP's weight-to-1 rule).
- **P33 Unbounded per-pixel probe lists** — linear search per fragment (Bevy's own reason for
  8); an I-cache / divergence problem before a bandwidth one.
- **P34 Probe popping at the K-th contributor** (Lagarde's stated open limit); **P35 box
  proxies distort at oblique normals**; **P36 realtime capture latency** (9/14 frames);
  **P37 atlas exhaustion has a UX** (HDRP's resolution-reduction; UE's 341 cap).
- **P38 SSR on VB without a colour source** — placement after `vb_shade` is correctness.
- **P39 Believing SSR exists because a flag does** — `ssr_on` is capped and the cap is pinned by
  four test sites (§0.6); a rung must lift the cap AND re-bless them.
- **P40 Shader-variant explosion** — `deferred_pbr.hlsl` already has six manifest rows; a
  reflection rung crossing {backend} × {resolution} × {denoiser} multiplies; the
  `RENDER-HYBRID-RAY-SYSTEM-DESIGN.md` §6 rule (one parameterised body, owner-eval per authored
  body) applies from the first commit.
- **P41 Byte-identity accounting** — moving the tonemap out of the resolve changes `lit` for
  every pixel and breaks every pin by construction; a planned re-bless, declared up front.
- **P42 The RHI is not ready** — cube views absent BECAUSE nothing creates them; zero BCn
  formats; PNG only. Any plan that starts at the shader has three unbudgeted work items under it.
- **P43 The brief's "every shader is eDSL"** — false for the BRDF; a BRDF change has no
  `_edsl_sync` tripwire, only the image golden.
- **P44 Gaia has no blob asset kind** — a baked LUT / prefiltered environment cannot be a `table`
  row; the blob region is a kernel request (AK-6), not a Gaia feature.
- **P45 Storing probe/LUT data outside ECS storage** — Principle 0; the DDGI atlas is the
  precedent, and physics paid for a side-`Vec` mirror once
  (`docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md`).

## 6. The feature ladder — prerequisites, as the evidence orders them

The order below is forced by §0's gaps, not by preference; the design document turns it into
rungs with gates.

1. **HDR source decode** — RGBE in-house (~200 lines); EXR is not cheap; KTX2/BC6H needs a BCn
   `Format` family + `abi_guard` asserts + a container reader + an offline encoder.
2. **An environment image shape** — cube views (RHI: `TextureViewDimension::Cube/CubeArray`,
   `VK_IMAGE_CREATE_CUBE_COMPATIBLE` at `texture.rs:240-271`, a `TextureCube` sampling path, the
   bindless table is `Texture2D`-only) OR an **octahedral 2D / 2D-array** with explicit borders
   (zero RHI changes; `oct.rs` leaves; DDGI precedent). The single largest architectural fork.
3. **The DFG LUT** — no new shape; `upload_texture_2d_raw` + one `bpp` arm + a `.bin` (the SMAA
   precedent); replaces the analytic fit under `energy_comp` immediately.
4. **Irradiance SH-9** — no texture shape; lives in a dedicated env UBO, not the light header
   (growing `LIGHT_HEADER_WORDS` re-encodes every golden).
5. **Prefiltered specular chain** — needs 1 + 2, a source mip chain (compute downsample; today
   mips are transfer blits), a GGX convolution pass with filtered importance sampling; the HZB
   pyramid is the in-tree mip-walking precedent (`add_image_mipped`, per-level views).
6. **Local probes** — a probe component + proxy row (the light-table/GPU-column shape), a
   `D2Array` atlas (`MAX_TEXTURE_LAYERS = 16` binds the count), per-pixel selection, capture
   through a per-view seam (plan R11 "reflection-probe-view forward vs main deferred smoke",
   `MULTI-PARADIGM-RENDER-PLAN.md:497`; CSM / shadow atlas are the existing non-main views).
7. **HDR scene colour + tonemap at the tail** — prerequisite of every screen-space technique.
8. **SSR** — lift the cap, `SsrConfig`/`ResolvedSsr`, roughness thin-aux on every path, a
   **nearest-surface** pyramid on every path, a previous-frame HDR ring, confidence-weighted
   composition at the ambient site.
9. **Stochastic SSR + denoiser** — VNDF, blue noise, half-res, ratio-estimator resolve, virtual
   motion, a separate history, demodulation, hit distance.
10. **Traced fallbacks** — SDF march for SSR misses (SDF legs); brick atlas when P9 lands; HW-RT
    closest-hit + hitT + hit lighting behind `hwrt`, owner-eval only; DDGI radiance-atlas revisit
    for the rough tail.
11. **Transparents, planar, material extensions, geometric specular AA.**

## 7. What the research could not resolve

- No UE shader source beyond a GitHub mirror of `ScreenSpaceReflections.usf` was read; Lumen's
  performance-guide numbers (`MaxRoughnessToTrace = 0.4`, `DownsampleFactor`, "1 ms on Series S")
  are second-hand from search summaries of a 403'd page.
- Stachowiak's 2015 slide deck (58 MB) was not opened; stochastic-SSR detail comes from the
  abstract, SSSR's manual and interplayoflight.
- The SIGGRAPH 2022 Lumen course PDF (Wright et al.) exceeded the fetch cap — the single
  highest-value unread primary source for Lumen's reflection denoiser and per-pass costs.
- Filament's `Filament.md.html` IBL numeric section did not serve; the LUT size 128 comes from
  cmgen / issue #875.
- Ray Tracing Gems ch.32 and RTG II ch.49 (ReBLUR) sit behind Springer auth; ch.32's sample HLSL
  is public.
- The "ratio estimator" refinement of stochastic SSR is recorded as existing; its formula is
  unverified here.
- No RTX 3060 per-pass reflection timing exists in public; §4 says which GPU each number is from.
- **Two ladder candidates raised in critique that this survey could not source — recorded as
  UNVERIFIED, ruled neither in nor out** (critique NB11). They are named here so a later pass with
  the right primary source can decide them, rather than leaving the technique catalogue silently
  short:
  1. **Reflection-capture normalisation against baked / probe-volume lighting** — UE's "reflection
     capture normalization", HDRP's "reflection probe normalization with APV". The mechanism, if
     real, divides a probe's baked ambient out and multiplies the live irradiance in, so a probe
     captured under one lighting scenario does not carry stale sky into a relit scene. If it
     exists it belongs between T19 (probe blending) and T35 (the DDGI tail), and in the design's
     ladder between R2 and R9 as an R2b.
  2. **HDRP "sky occlusion" for probe lighting** — a per-probe visibility-to-sky term that lets
     probe data react to a changing sky at runtime.

  Three pages were opened for them and none carried the text [D]:
  `dev.epicgames.com/documentation/en-us/unreal-engine/reflections-environment-in-unreal-engine`
  (no normalisation text);
  `docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Reflection-Probe.html`
  (no normalisation / APV text);
  `…/manual/probevolumes.html` (only a table-of-contents-level mention, "Use Lighting Scenarios or
  sky occlusion to change how objects use the data in Adaptive Probe Volumes at runtime"). Under
  this document's own rule an unsourced claim is UNVERIFIED, so neither is a T-number and neither
  is a rung. `REFLECTIONS-DESIGN-SPACE.md` §14 carries the same entry.

## 8. Sources

### By system

- **Filament** [S]: `github.com/google/filament/blob/main/shaders/src/surface_light_indirect.fs`,
  `…/surface_light_reflections.fs`, `…/surface_ambient_occlusion.fs`,
  `…/libs/ibl/src/CubemapIBL.cpp`, `…/tools/cmgen/src/cmgen.cpp`; issue #875; [D]
  `google.github.io/filament/Filament.md.html`.
- **Bevy** [S]: `raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_pbr/src/light_probe/
  {mod.rs, environment_map.rs, environment_map.wesl, environment_filter.wesl, generate.rs}`,
  `…/ssr/mod.rs`; PR #19076, PR #13418, PR #7051, issue #14639 [S/D];
  `docs.rs/bevy/latest/bevy/pbr/environment_map/index.html`,
  `docs.rs/bevy/latest/bevy/pbr/struct.ScreenSpaceReflectionsPlugin.html` [D].
- **Godot** [S]: `raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/
  effects/ss_effects.cpp`, `…/shaders/effects/screen_space_reflection.glsl`; PR #111210, PR
  #100241, PR #69514; [D] `godot-docs … reflection_probes.rst`, `class_sky.html`,
  `godotengine.org/article/godot-40-gets-sdf-based-real-time-global-illumination/`.
- **Unity** [S]: `Unity-Technologies/Graphics … ShaderLibrary/ImageBasedLighting.hlsl`, HDRP
  `Documentation~/reference-screen-space-reflection.md`; [D] HDRP manuals `reflection-understand`,
  `Reflection-in-HDRP` (12.0), `Override-Screen-Space-Reflection` (17.0), `Ray-Traced-Reflections`
  (17.0), `HDRP-Asset` (14.0), `Reflection-Probe` (17.0), `RefProbePerformance`; [B] HDRP 15
  changelog (octahedral atlas).
- **Unreal** [D]: `dev.epicgames.com/documentation/en-us/unreal-engine/{reflections-environment,
  reflections-captures, planar-reflections, screen-space-reflections,
  lumen-global-illumination-and-reflections, lumen-technical-details, lumen-performance-guide
  (403)}`; `indxzero.github.io/ue544cvarwiki/articles/r.reflectioncaptureresolution/`; [S]
  `github.com/chendi-YU/UnrealEngine/blob/master/Engine/Shaders/ScreenSpaceReflections.usf`
  (mirror); [B] `unrealdirective.com … r-lumen-reflections-hardwareraytracing-retrace-farfield`.
- **Frostbite / Lagarde** [B/D]: `seblagarde.wordpress.com/2012/09/29/image-based-lighting-
  approaches-and-parallax-corrected-cubemap/`; `dl.acm.org/doi/10.1145/2343045.2343094`;
  `seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/`;
  `blog.selfshadow.com/publications/s2014-shading-course/`; `advances.realtimerendering.com/s2015/`;
  `h3.gd/stochastic-ssr/`; `ea.com/frostbite/news/stochastic-screen-space-reflections`.
- **AMD** [D]: `gpuopen.com/manuals/fidelityfx_sdk/fidelityfx_sdk-page_techniques_stochastic-
  screen-space-reflections/`, `gpuopen.com/fidelityfx-sssr/`, `gpuopen.com/fidelityfx-hybrid-
  reflections/`; [B] `interplayoflight.wordpress.com/2022/09/28/notes-on-screenspace-reflections-
  with-fidelityfx-sssr/`.
- **NVIDIA** [D]: `github.com/NVIDIA-RTX/NRD/blob/master/README.md`; [B]
  `deepwiki.com/NVIDIA-RTX/NRD/3-denoisers`; [D]
  `research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing`.
- **Papers** [D]: Karis 2013 `blog.selfshadow.com/publications/s2013-shading-course/karis/
  s2013_pbs_epic_notes_v2.pdf`; Fdez-Agüera `jcgt.org/published/0008/01/03/`; Kulla-Conty
  `…/s2017-shading-course/imageworks/s2017_pbs_imageworks_slides.pdf`; Turquin
  `blog.selfshadow.com/publications/turquin/ms_comp_final.pdf`; McGuire & Mara
  `jcgt.org/published/0003/04/04/`; Uludag GPU Pro 5 ch.45 (Taylor & Francis / O'Reilly); Hofmann
  et al. `selgrad.org/publications/2017_hpg_HBSS.pdf`; Heitz 2018
  `jcgt.org/published/0007/04/01/paper.pdf`; Schied et al. SVGF (HPG 2017 PDF); Tokuyoshi &
  Kaplanyan `yusuketokuyoshi.com/papers/2019/ImprovedGeometricSpecularAA.pdf`; Engelhardt &
  Dachsbacher 2008 (Semantic Scholar); Cocco et al. `onlinelibrary.wiley.com/doi/abs/10.1111/
  cgf.15233`; Hirvonen et al. RTG ch.32 (`github.com/Apress/ray-tracing-gems/…/RayGen.hlsl`);
  Lengyel `terathon.com/lengyel/Lengyel-Oblique.pdf`; glTF-Sample-Viewer issue #43;
  glTF-IBL-Sampler; Radiance `radsite.lbl.gov/radiance/refer/Notes/picture_format.html`; KTX2
  `github.khronos.org/KTX-Specification/ktxspec.v2.html`; Basis Universal KTX2 wiki.
- **Blogs recorded, not relied on** [B]: sugulee (Hi-Z SSR part 2), jpgrenier (reflections
  reprojection), bitsquid (reprojecting reflections — fetch failed), filmicworlds (VB + material
  graphs), marbarod (octahedral mip atlases), OGRE VCT notes, radiance.wiki, tobias-franke SSCT.

### In this tree (all re-opened at `ed0bed45`)

`crates/boyko_rhi_vulkan/shaders/{pbr_lighting.hlsli, deferred_pbr.hlsl, forward_opaque.fs.hlsl,
forward_sky.fs.hlsl, vb_shade.comp.hlsl, vb_geo.comp.hlsl, vb_pack.hlsli, gbuffer_mrt.fs.hlsl,
light_table.hlsli, ray_gen.hlsli, sdf_field.hlsli, sdf_shadow_leaves.hlsli,
sdf_spheretrace.hlsl, sdf_gbuffer_composite.hlsl, ddgi_resolve.hlsli, hzb_build.comp.hlsl,
taa_resolve.comp.hlsl}`; `crates/boyko_rhi_vulkan/src/{goldens.rs, ddgi.rs, brick_atlas.rs,
bindless.rs, texture.rs, device.rs, compute.rs, accel_build.rs, present/targets.rs,
present/graph_bridge.rs, present/scene_types.rs, framegraph/mod.rs, framegraph/graph.rs}`;
`crates/boyko_rhi_vulkan/tests/*_sync.rs` (17: `cluster_cull_spv`, `ddgi_probe_gi`,
`gbuffer_mrt_edsl`, `hzb_build_spv`, `interp_edsl`, `marcher_spv`, `particle_edsl`,
`sdf_field_edsl`, `sdf_mesh_shadow_spv`, `ssao_atrous_edsl`, `ssao_edsl`, `vb_bary_edsl`,
`vb_batch_cull_spv`, `vb_froxel_spv`, `vb_geo_preprocess`, `vb_lit_producer_spv`,
`vb_raster_geo_classify_spv`); `crates/boyko_rhi/src/{enums.rs, device.rs}`;
`crates/boyko_render/src/{light.rs, material.rs, material_table.rs, texture.rs, smaa_luts.rs,
bindless.rs, hzb.rs, hzb_config.rs, render_path_config.rs, ray_backend.rs, motion_cam.rs,
taa_state.rs, aa_config.rs, ssao_config.rs, csm_config.rs, ddgi_config.rs, gbuffer_depth.rs}`;
`crates/boyko_app/src/{hzb_plan.rs, gpu_scene/tlas.rs}`; `crates/boyko_shaderdsl/src/{oct.rs,
probe_blend.rs, ssao.rs, emit/mod.rs, emit/shaders.rs}`; `crates/boyko_image/src/lib.rs`;
`crates/boyko_sdf_math/src/brick.rs`; `crates/boyko_serialize/src/format.rs`;
`docs/{PBR-MATERIALS-PLAN, LIGHTING-PLAN, MULTI-PARADIGM-RENDER-PLAN, SDF-PERF-AUDIT,
RENDER-SDFDDGI-PLAN, RENDER-HYBRID-RAY-SYSTEM-DESIGN, RENDER-HWRT-OPTIONAL-ANALYSIS,
RENDER-GRAPH-API-PLAN, ARCHITECTURE-FRAME-GRAPH-PLAN, SHADER-VARIANT-MANIFEST, FEATURE_MAP}.md`;
`docs/gaia/{LANGUAGE, DECISIONS, CAMPAIGN}.md`; `docs/animation/ANIMATION-DESIGN-SPACE.md`;
`tests/internal_docs_anchors.rs`.

## 9. Critique log — corrections made while re-opening the lenses' claims

| # | Lens claim | What the tree says | Action |
|---|---|---|---|
| C1 | "21 of ~75 HLSL files carry a sentinel" | `grep -l` over `*.hlsl` + `*.hlsli`: **20 of 89** | corrected in §0.1 |
| C2 | Brick atlas has a "trilinear clamp sampler" | `brick_atlas.rs:80-84`: **NEAREST / clamp-to-edge / no-mip** (BUG-M2-GPU-1) | corrected in §0.8; a reflection cone-trace over the atlas needs its own filtering |
| C3 | `field_skip` is "consumed through the gateway swap" | `sdf_field.hlsli:248-254`: "Source-only for W0 — no shader yet calls it"; no `.hlsl` references it | corrected in §0.8 |
| C4 | `FRAMEGRAPH_IMAGE_COUNT` is 11 / 16 (as-built at 3b) | `graph_bridge.rs:765`, `:770`: **22 (hwrt) / 16 (not hwrt)** | corrected in §0.10 |
| C5 | Commit `128233be` | `git rev-parse` → `ed0bed45` on `feat/multi-paradigm-render` | all citations re-opened at `ed0bed45` |
| C6 | `ThinAuxMask::ROUGHNESS` "packed in `thin_normal.BA`" | VB packs roughness in `.B` alone (`vb_geo.comp.hlsl:47-50`); `render_path_config.rs:446` is the stale doc | recorded as a doc discrepancy, §0.6 |
| C7 | `gbuffer_depth.rs` cited under `boyko_rhi_vulkan` | it is `crates/boyko_render/src/gbuffer_depth.rs` | corrected |
| C8 | "the bindless free-list" as the env-map home | the table is bound only on the textured raster set 1, not on the resolve (`bindless.rs:1-3`) | §0.3: the resolve needs explicit env bindings, like DDGI @16/17/18 |
| C9 | `ssao_config.rs` / `csm_config.rs` as the config precedent | confirmed (`ssao_config.rs:1-14`; `csm_config.rs:1-8`) | cited |
| C10 | Deferred pass names `vb_sky :4585 … present_sample :6219` | confirmed by `grep add_pass` | cited |
| C11 | Several lens `file:line`s off by 1-3 lines (`enums.rs:525-529` vs `:526-542`; `deferred_pbr.hlsl:580-586` vs `:582-589`; `hzb.rs:55-67` vs `:55-72`; `light.rs:356-360` vs `:355-361`) | re-opened | corrected throughout |

## 10. Critique log — the `architecture-critic`'s pass (2026-09-10)

§9 above is this survey's own log of the corrections its author made while re-opening the five
research lenses' claims. This section is a different thing: the findings the `architecture-critic`
raised against **this** document and its companion, and what the revision did with each. Every
`file:line` was re-opened at `ed0bed45`; every census was **re-run, not copied from the finding**.

⚠️ **Three of these rows were marked "Fixed" in the design's §15 table before the edits to THIS
file existed.** `REFLECTIONS-DESIGN-SPACE.md` §15 (pass 1) recorded "RESEARCH §0.2 corrected (its
C12)" for B1 and "Fixed in the RESEARCH (§0.2 + its C12)" for NB10, and "RESEARCH §7 carries both"
for NB11 — and at that moment §0.2 still named two duplicate files, §7 carried neither candidate,
and there was no C12 in §9. The design-side repairs were real; the RESEARCH-side ones were
asserted. A reader of §15 alone would have believed the census was fixed in both documents, which
is the repository's catalogued "a summary outlives its retraction" failure applied to a critique
log — the instrument whose entire purpose is to be believed without re-checking. The rows below
are the pass-2 edits that make those three claims true, and §15 now carries the same admission.

| # | Finding against this document | Action |
|---|---|---|
| C12 | §0.2 stated `env_brdf_approx` is "duplicated verbatim in `forward_opaque.fs.hlsl:182-184` and `vb_shade.comp.hlsl:247-249`" — two files (critique B1's second half, and NB10) | **Fixed.** `grep 'float2 env_brdf_approx'` over `crates/boyko_rhi_vulkan/shaders/` returns **SIX**: `deferred_pbr.hlsl:582`, `forward_opaque.fs.hlsl:182`, `sdf_forward_march.comp.hlsl:855`, `vb_resolve.comp.hlsl:212`, `vb_shade.comp.hlsl:247`, `vb_shade_split.comp.hlsl:308`. §0.2 now carries a six-row table pairing each definition with its `eval_pbr_ambient_hemi` call and its path. The two missed files are the two whose headers say their ambient+tonemap tail is a TOKEN-FOR-TOKEN copy (`vb_resolve.comp.hlsl:23`, `sdf_forward_march.comp.hlsl:37`) — the very duplicate class the census was counting |
| C13 | §0.2's companion census, `R = reflect(-v, n)` "hoisted once per pixel at `deferred_pbr.hlsl:861`, `vb_shade.comp.hlsl:449`, `forward_opaque.fs.hlsl:250`" — three sites | **Fixed.** The same six files: `+ sdf_forward_march.comp.hlsl:1035`, `vb_resolve.comp.hlsl:297`, `vb_shade_split.comp.hlsl:451`. Third instance of one undercount, found only because C12 forced the grep to be re-run rather than the list re-read. Folded into the same table |
| C14 | §0.4 named the tonemap + OETF at the resolve alone, which is how the design costed the HDR-`lit` move at "three producers" (critique NB6) | **Fixed.** `grep -l 'OETF_GAMMA_EXP\|tonemap_select'` returns **eight** shaders + the `pbr_lighting.hlsli:181-209` definition. §0.4 lists them and singles out `ssaa_downsample.fs.hlsl`, which does not merely append the tail — it decodes, box-averages and re-encodes (`:8-16`) and books its own ceiling at `:17-19`, so the move changes that pass's shape |
| C15 | §0.7 called `gViewT` "the universal screen-space input" — the word the design turned into "`gViewT` exists on every path" (critique B4) | **Fixed.** The IMAGE is allocated on every path (`targets.rs:128-133`); the PRODUCERS are two and both path-specific — `viewt_from_depth` "armed iff `mesh_leg && !sdf_leg`" (`graph_bridge.rs:1319-1330`) and `vb_viewt` (`:3608-3609`). `declare_forward_graph` (`:2585`) declares no `viewt` write and asserts it (`:2936-2941`); Forward's prepass writes hardware reverse-Z (`forward_opaque.vs.hlsl:16`). §0.7 now separates image from producer and names the missing Forward producer as a prerequisite (the design's RK-12) |
| C16 | Two ladder candidates raised in critique — reflection-capture normalisation, HDRP sky occlusion — could not be sourced from the three pages opened (critique NB11) | **Fixed as an open item, not as a technique.** §7 now carries both as UNVERIFIED with the three URLs that did not answer and where each would sit if a primary source later confirms it. Neither gets a T-number; under this document's own provenance rule, unsourced is UNVERIFIED, and inventing a T-row to look complete would be the failure the rule exists to prevent |

**What this pass did NOT change.** The technique catalogue (T1–T42), the pitfall register
(P1–P45), the cost model (§4) and the source list (§8) are untouched: none of the sixteen findings
disputed an external claim. Every correction above is a claim about **this tree** — which is where
the survey's own §9 corrections landed too, and that is the standing pattern worth naming: of the
sixteen corrections this document has taken across two passes (C1–C16), **every one is a claim
about this repository and none is a claim about a surveyed engine**. The one external finding
(NB11) was an *omission*, not an error, and it is answered by an UNVERIFIED entry rather than a
technique row. Externally sourced claims arrive already gated — a URL either serves the sentence
or it does not — while an in-tree claim is a prose census that nothing re-runs. The in-tree half
of a survey is the half that needs a gate — three separate shader censuses in §0 (§0.2's two,
§0.4's one) were each short, and each was found only by re-running the grep, never by re-reading
the sentence.
