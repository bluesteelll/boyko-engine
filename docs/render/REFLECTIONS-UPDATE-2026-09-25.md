# Reflections: update 2026-09-25 (a delta over the 2026-09-10 survey and design)

> **Status.** Architect's delta, 2026-09-25, **revision 2**. Written against trunk **`6394bc5e`**, read
> through the worktree `D:/wt/docs`. Nothing was built, timed or run, and no `cargo` command was
> issued; three build lanes were busy on the machine.
>
> **Revision 2** answers the architecture review of revision 1: CHANGES REQUESTED, 2 critical,
> 8 important, 3 optional. Every remark was re-checked against the code and accepted. The revision
> also found six defects of its own. §14 (the review log) lists each remark, the verdict and where
> it is resolved. The largest changes:
> - where probe rows live (U8, §3.8);
> - R9a now lands D21 atomically (U4, §3.4);
> - the SDF march is re-priced and SDF pixels on VB get a post-shade route (U3, R8v);
> - the pre-light depth on VB × Both is mesh-only (U19, §3.12);
> - the binding budget is re-derived (U5).
>
> **Relation to the earlier documents.** The survey
> [`REFLECTIONS-RESEARCH.md`](REFLECTIONS-RESEARCH.md) and the design
> [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) (both 2026-09-10, at `ed0bed45`) stay
> as the record and are **not edited**. This file cites them by their own numbers:
> - techniques T1–T42 and pitfalls P1–P45 (the survey);
> - rungs R0–R12, decisions D1–D21, kernel requests RK-1..RK-14 and gates G1–G11 (the design).
>
> `ed0bed45` is an ancestor of `6394bc5e`: 438 commits, 52 of them in `boyko_render`,
> `boyko_rhi_vulkan` or `boyko_rhi`. Where this file and the 09-10 design disagree, this file wins,
> and each such row says why.
>
> **Provenance tags** (the 09-10 convention, one tag added):
> - **[S]** source code read;
> - **[D]** official docs, talk or paper;
> - **[B]** blog (recorded, not relied on);
> - **[T]** this tree, re-opened at `6394bc5e` by the author of this file;
> - **[R]** carried from this session's research pass and not re-opened here;
> - **EST** an estimate derived in this file (§5.1 gives the derivation).
>
> Every `path:line` below was re-read by content at `6394bc5e`. Anything cited by symbol only was not
> pinned to a line. `docs/render/` is not in the `internal_docs_anchors` corpus (`GATED_DOCS` in
> `tests/internal_docs_anchors.rs`), so nothing machine-checks these anchors. They were checked by
> hand.
>
> **Scope.** The workflow assigned this run the topic *reflections*. The owner's relayed request also
> named post-FX, general anti-aliasing, VFX (explosions, rain), sun rays and volumetrics. Those have
> their own documents in this batch, in the same directory:
> - [`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md);
> - [`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md);
> - [`VOLUMETRICS-DESIGN-SPACE.md`](VOLUMETRICS-DESIGN-SPACE.md), whose light shafts are the sun rays.
>
> §10 records only the interfaces this campaign shares with them.

## 0. The answer in one paragraph

The 09-10 architecture still holds. It is a chain of sources, each returning a colour and a weight,
composed until the weight reaches 1 under one split-sum weight. Its prerequisites are HDR scene
colour, a nearest-surface depth pyramid separate from the occlusion HZB, and half-resolution
stochastic tracing with a reflection-only history and demodulation. Every shipped system re-read
this pass converged on the same shape (§4).

What changed is the **tree under it**, in eight ways the 09-10 design could not see:
1. **No SSR exists.** The brief's premise is false. On Forward, *every* pre-light consumer is
   degraded, so a Forward `gViewT` producer (RK-12) is not enough to reach SSR there (§3.1).
2. **Trace origins.** A hardware-shadow defect showed that any trace origin rebuilt from `gViewT` on
   the unjittered camera ray sits off the surface under TAA, and the default jitter scope *doubles*
   the error (open item D2). Of the reflection tracers only the world-space triangle trace (rayQuery,
   R10) inherits it, so the fix lands there (§3.2).
3. **VB's thin-aux lane.** It carries the geometric, not normal-mapped, normal and the untextured
   roughness, and SDF-owned pixels on VB get no thin-aux at all (§3.3).
4. **VB × Both's pre-light depth is mesh-only.** The marcher is the last `gViewT` writer and runs
   after `vb_shade_split`, so SSR sees both representations only on Deferred (§3.12).
5. **DDGI misses return zero radiance.** Fixing that (R9a) puts the sky into the probes, and D21 must
   land in the same rung or the sky is counted twice (§3.4).
6. **Binding limits.** The largest `lit`-producing resolve set holds 23 bindings (the 25-binding set
   writes shadow visibility, not `lit`). Reflections add 5, giving 28, over the cap of 25. The RHI's
   generic compute pipeline takes one descriptor set only (§3.5).
7. **Every producer walks the point/spot span flat.** So probe rows cannot live there; they get a
   span of their own (§3.8).
8. **The occlusion pyramid's base is `prev_pow2`.** At 1080p its level 0 is 1024 texels wide. That
   is sound for occlusion and too coarse for a mirror's thickness test (§3.9).

**The hybrid-specific conclusion.** Representation decides the tracer:
- rayQuery sees only BLAS triangles;
- the SDF march sees only the field;
- SSR sees whatever the pre-light depth holds: both representations on Deferred, meshes only on
  VB × Both (§3.12).

So the recommended cascade is:
1. screen-space, merged with the SDF march by nearest hit on VB × Both (U19);
2. the **nearest** of the hardware and SDF hits (U10). Which is traced first is decided by a
   fixture bench at R10a;
3. compute-captured probes;
4. the DDGI tail;
5. the environment.

**What it costs.**
- **Deferred × Sdf.** SSR plus the SDF march for SSR misses gives complete off-screen reflections on
  every GPU. The march costs EST **0.13–0.72 ms at 1080p, hit shading included**, at the 09-10 miss
  fraction (§5.2 G).
- **VB.** The marcher shades the SDF pixels after every pre-light pass. They get their traced
  reflection from a post-shade route, R8v, at EST 0.06–1.1 ms depending on how much of the screen is
  smooth SDF. Until then they get nothing traced (§3.3).

Off-screen reflections without RT hardware are what this hybrid alone can offer, and that is why the
SDF rungs move ahead of the hardware rung (§6, §7).

## 1. What the 09-10 documents decided (recap)

| Area | 09-10 decision | Where |
|---|---|---|
| Shape | One radiance query per pixel, answered by a chain of `(rgb, w)` sources composed to weight 1 in `refl_compose`, under ONE split-sum weight applied after composition | design §0, §2.3, D12 |
| Environment | Octahedral HDR image with a GGX-prefiltered bordered mip chain, SH-9, and a 128² DFG LUT; baked on the host by the leaf's `f32` instantiation; RGBE decoder in-house | D1–D3, D5, D13, D17; R0–R1 |
| Probes | `ReflectionProbe` component; light-table rows of a new kind (`LIGHT_KIND_PROBE = 4`) plus a parallel `ProbeGpu` SSBO; `D2Array` atlas; at most 4 blended; size-ascending in-place sort | §1.3, D6; R2 |
| Scene colour | `lit` → B10G11R11 behind a boot probe, else RGBA16F; tonemap moved out of eight producers into one tail pass | D4, RK-14; R4a |
| Pyramid | A separate nearest-surface view-z pyramid built from `gViewT`, reusing the occlusion HZB's build body | D7; R4b |
| SSR | Reads `lit_prev` (previous frame, HDR), copied before `taa_resolve`; subtracts the previous frame's jitter at the fetch; half-res plus ratio-estimator resolve; own three-pass denoiser; trace cut-off 0.4, DDGI tail at 0.7 | D8–D11, D19; R5–R7 |
| Traced | SDF march for SSR misses on SDF legs (R8); DDGI irradiance tail (R9); hardware rayQuery closest-hit with "hit-lighting lite" behind `hwrt`, owner-eval only (R10/R10b); radiance atlas plus cone trace as research (R11) | §3 |
| Diffuse | `diffuse_color · (gi + (1 − w_gi) · sh_diffuse) · ao_final` so the sky is not counted twice inside the DDGI grid | D21 |
| Planar | A second full view through the per-view seam, as an owner ballot | R12 |
| Gates | G1 furnace … G11 counts-not-clocks; the six-site census test for the ambient splice | §10, §2.1 |
| Owner ballots | Twelve (§13.B), led by "HDR scene colour now or never" | §13.B |

## 2. What is unchanged (re-verified at `6394bc5e`)

Every row was re-opened; the line numbers are today's.

| 09-10 fact | Today | Status |
|---|---|---|
| `lit` is RGBA8 post-tonemap | `crates/boyko_rhi_vulkan/src/present/targets.rs:2279` `GBUFFER_FORMAT = Format::R8G8B8A8Unorm` | unchanged |
| Sampler has no mip filtering | `crates/boyko_rhi/src/device.rs:248-252`: `MipMode` has only `None` | unchanged (RK-1 still owed) |
| No cube views | `crates/boyko_rhi/src/enums.rs:532-542` (D2, D3, D2Array) | unchanged; D1 octahedral still right |
| No HDR decoder | `crates/boyko_image/src/` is `error.rs inflate.rs lib.rs png.rs`, PNG only | unchanged (RK-4) |
| Raw upload is single-level | `crates/boyko_render/src/texture.rs:403` `mip_levels: 1` | unchanged (RK-11) |
| Occlusion HZB is farthest-surface | `crates/boyko_render/src/hzb.rs:55` ("the reduce is `min`, and under reverse-Z that is the FARTHEST surface") | unchanged; D7's separate pyramid still required |
| HW-RT is inline only | `crates/boyko_rhi_vulkan/src/ffi.rs:946` ("inline `rayQuery`; NO ray-tracing-pipeline") | unchanged |
| Brick-atlas sampler is NEAREST | `crates/boyko_rhi_vulkan/src/brick_atlas.rs:80-81` | unchanged; `field_skip` still source-only (`sdf_field.hlsli:254`, no `.hlsl` calls it) |
| `MAX_SDF_EDITS = 16` | `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli:48` | unchanged |
| Six ambient splice sites | `eval_pbr_ambient_hemi(` called in `deferred_pbr.hlsl:1238`, `forward_opaque.fs.hlsl:303`, `sdf_forward_march.comp.hlsl:1080`, `vb_resolve.comp.hlsl:368`, `vb_shade.comp.hlsl:522`, `vb_shade_split.comp.hlsl:540`; six `env_brdf_approx` definitions; six `R = reflect(-v, n)` hoists | census holds 6 / 6 / 6. `deferred_pbr.hlsl` moved (the `env_brdf_approx` def 582 → **623**, the call 1163 → **1238**, the hoist 861 → **902**); the other five files did not |
| Eight tonemap producers | `grep -l 'OETF_GAMMA_EXP\|tonemap_select'` finds the same eight `.hlsl` plus `pbr_lighting.hlsli` | unchanged |
| Frame-graph image count | `graph_bridge.rs:765` (22, hwrt) / `:770` (16) | unchanged |
| SSCS compares view-z | `deferred_pbr.hlsl:764-775` rebuilds `Ps` from `gViewT` via `generate_ray` and compares `dot(Ps − eye, cam_forward)` | the 09-10 §14 open item is **closed**: view-z, as D7 assumed |
| Motion is camera-only | `crates/boyko_render/src/taa_config.rs:307` `MvSource::PerObject` is "declared, and deliberately not wired" | unchanged; R7 ships camera-only as planned |

## 3. What changed, what it does to the design, and the decision taken

### 3.1 SSR does not exist, and on Forward the seam does not exist either

- `RenderPathConsumers::ssr_on` is still a reserved bit. Its doc says no `SsrConfig` exists and the
  caller threads a literal `false` (`render_path_config.rs:843-845`); the only writer is
  `crates/boyko_app/src/runner.rs:552` (`ssr_on: false`).
- `cap_vb_v1_consumers` documents the bit as capped always (`render_path_config.rs:1420`).
- **The brief's "SSR exists as a VB pre-light consumer" is false.** What exists is the arming
  plumbing, e.g. `ssr_on` inserts `ThinAuxMask::ROUGHNESS` at `:1220-1222`.

**Forward is wider than RK-12 assumed.** `cap_forward_v1_consumers` degrades **every** pre-light
consumer on Forward and ForwardPlus with `ForwardPreLightConsumersNotYetImplemented`
(`render_path_config.rs:1395`): SSAO, DDGI, shadow denoise and SSR together. TAA is degraded too,
at `:1405`. The Forward graph's `viewt` tripwire is still in place (`graph_bridge.rs:2981-2987`).
A `fwd_viewt` producer (RK-12) would therefore feed a consumer set the path refuses to arm.

**Decision U12.** Forward and ForwardPlus get the **path-independent rungs only**:
- they get the environment and LUT (R1), probes (R2, R2b) and captured probes (R3);
- they do not get the DDGI tail (DDGI never arms on Forward, §3.4) or hardware-traced reflections;
- SSR, SSPR (§7 RP) and the denoiser ship on Deferred and VB only.

A Forward pre-light seam is its own campaign: the renderer's own degrade ladder names it "not yet
implemented", and SSAO sits behind the same door. RK-12 is **withdrawn from this campaign** and
handed to that one.

### 3.2 The trace origin under TAA: a shipped defect class, and which tracer inherits it

- **The defect.** Commit `999babfb` (2026-09-21) fixed a false hardware shadow covering up to 57 %
  of the frame and jumping every frame.
  - For a raster-owned pixel, `gViewT` is the distance the **jittered** raster wrote.
  - `P = ro + rd·gViewT` on the unjittered camera ray therefore lies off the surface, below the
    receiver on +y jitter phases.
  - The fix re-places the shadow origin on the raster's own ray: the `P_shadow` block at
    `deferred_pbr.hlsl:1063-1074`, driven by `RayShadowFrame` (`crates/boyko_render/src/upload.rs:939`)
    and `raster_ray_forward` (`crates/boyko_render/src/view.rs:181`).
- **Two findings recorded in `docs/OPEN-QUESTIONS.md` widen it:**
  - **D2** (heading at `:75`, owner decision). The default `JitterScope::RasterAndBasis`
    (`taa_config.rs:118`, made default at `:467`) shears the camera basis the **opposite** way from
    the raster jitter, so the two sample positions sit `2j` apart. The sign is pinned by
    `basis_shear_mirrors_the_raster_jitter_pinned_until_owner_ruling` (`view.rs:1409`).
  - **VB D7** (heading at `:130`). `vb_shadow_vis.comp.hlsl:193` still builds `P = ro + rd * view_t`
    and casts from it at `:220`, which is the same class once the VB split is armed.
- **What 09-10 missed.** Its D19 subtracts the previous frame's jitter at the `lit_prev` **colour
  fetch**. Nothing in it places the **origin** of a reflection ray.

**Which tracer inherits it** (revision 1 assumed all of them; the review showed otherwise):
- **rayQuery (R10).** A world-space triangle trace from an origin below a mirror floor crosses the
  floor's own triangle at `t ≈ δ / cos θ`. That is a self-hit in a jitter-phase-dependent pattern,
  which a static golden freezes rather than catches. The memory record "pins froze a jitter-phase
  defect" is this class. **Exposed.**
- **SSR (R5–R7).** It marches the same depth buffer the origin was rebuilt from, so in screen space
  the origin lies on that buffer's own surface. It can self-hit only where the reflected ray leaves
  the surface at less than about the jitter angle. Half a pixel is 0.5 × 60° / 1080 ≈ 0.03° at 1080p
  with a 60° vertical field of view (EST). SSR's usual first-step offset absorbs this, and R5's
  mirror-floor fixture measures it. **Not exposed** in a way a gate can see (the review marks it
  PLAUSIBLE).
- **SDF march (R8, R8v).** For an SDF-owned pixel the camera-ray `P` is exact, because the marcher
  wrote `t` along that very ray. For a mesh pixel the field holds no mesh floor to self-hit.
  **Not exposed.**
- **Probe capture (R3).** Its origins are probe centres, not pixels. **Not exposed.**

**Decision U1 (revised): the on-surface origin rule belongs to R10a, the one rung whose tracer
inherits the defect, and it does not depend on D2's resolution.**

| Pixel (R10a/b rays) | Origin | Why exact |
|---|---|---|
| VB mesh pixel | `refl_trace_hw` re-fetches the triangle through `vb_geom_fetch` and uses its interpolated `world_pos` | an interpolation of the triangle's own vertices lies on the triangle, whatever ray picked the pixel |
| Deferred raster-owned pixel | the `P_shadow` rule: `ro_r + rd_r · gViewT` with `rd_r` from the raster's jittered forward (`SHADOW_RASTER_FWD`), same producer test against `gDepthHw` (`view_t == md · 64`) | the fix commit's derivation; exact under either jitter scope |
| SDF-owned pixel (any path) | the camera-ray `P` | the marcher wrote `t` along that very ray |

- **Plumbing.** The Deferred rule reads `gDepthHw` and `SHADOW_RASTER_FWD`. Both exist only on HWRT
  frames: `RayShadowFrame` is "written every HWRT frame" (`crates/boyko_render/src/upload.rs:926-948`), and `gDepthHw` sits
  in the HWRT sets only (`deferred_pbr.hlsl:1063-1074`). R10a is `hwrt`-only, so this rule adds **no**
  plumbing to any other path. Revision 1 placed it at R4c, which would have carried hwrt-class
  plumbing onto every path for no measurable benefit.
- **Shared leaf.** The rule becomes one eDSL leaf, `raster_origin` (§6.4), an oracle over
  parameters, landed at R10a. The shadow's hand-written `P_shadow` block is offered, not forced, the
  same leaf; that migration is a separate change with its own byte-identity proof.
- **Gates.**
  - *CI, device-free, red-first.* A host fixture over the leaf's `f32` instantiation. It takes a
    far-floor band of pixels, all 8 jitter phases, and both `JitterScope`s. `gViewT` is the jittered
    raster ray's analytic plane intersection. The fixture asserts two things:
    - the signed plane distance of `raster_origin`'s point is ≤ ε;
    - the camera-ray point exceeds ε on the phases `999babfb` measured.

    A red-first build that returns the camera-ray point fails the first assertion. This is the
    world-space check the review asked for, and it needs no tracer, so it can fail before any tracer
    exists.
  - *Device, owner-eval (R10a).* The 8-phase band shape of `978625a1` (phase 7 against phase 0 on a
    far-floor band) is run on a mirror floor through `refl_trace_hw`. It goes red with the camera-ray
    origin.

### 3.3 The VB thin-aux lane cannot drive a reflection

- **Normal.** `vb_geo.comp.hlsl` writes the **geometric** vertex normal: "NO tangent-space normal
  mapping — v1 scope cut" (`:25-27`).
- **Roughness.** It writes the per-material **scalar** roughness `Materials[mat].mrr.y` in B (`:47`).
- **The deferral.** Its material section records the rejected alternative, a SampleGrad'd thin
  normal, as "revisited with SSR" (`:36-42`).
- **The mismatch.** `vb_shade_split` shades with the normal-mapped normal and the textured roughness
  through `SampleGrad` (`vb_shade_split.comp.hlsl:383-403`). An SSR trace driven by thin-aux would
  therefore:
  - reflect along a direction the shading never uses;
  - classify on a roughness the shading never uses (a textured puddle mask would not show).
- **SDF pixels.** They have **no** thin-aux on VB: `gThinNormal` is written only by `vb_geo` (a
  `grep` over the shaders finds writers only there). The SDF leg under VB is shaded by
  `sdf_forward_march` after `vb_shade_split`, with no geometry pass of its own; that is the
  R-SDFSPLIT boundary in `render_path_config.rs` (`SDF_SPLIT_IMPLEMENTED = false`).
- **The marcher has no DDGI consumer** (`sdf_forward_march.comp.hlsl:1038`, "v1 scope cut … no
  SSAO/DDGI consumer"). Revision 1 said SDF pixels on VB receive "the DDGI tail" at the marcher's
  ambient site. That was false.

**Decision U2: the textured reflection lane.**
- **The variant.** SSR arms `vb_geo -D REFL`. It re-uses the `SampleGrad` splice to fetch the
  normal map and the metal-rough map. `vb_geom_fetch` produces the tangent, `tex_w` and `uv_grad`
  (the `vb_uv_grad` eDSL span) only under its `TEXTURED` define (`VbGeomFetchResult`), so the
  `REFL` compile also defines `TEXTURED`.
- **Roughness** is written into thin-normal **B**. That changes no current output, because nothing
  reads B or A today (`vb_geo.comp.hlsl:51-54`).
- **The normal-mapped normal** goes to a new `thin_refl_n` image, **R32_UINT** holding a 16:16
  octahedral normal. Why that format:
  - R32_UINT storage is core-mandatory [R], while R16G16 storage sits behind
    `shaderStorageImageExtendedFormats`, the gap the tree already probes for other formats
    (`device.rs:276-281`).
  - 8-bit octahedral precision (~1.4° steps) displaces a mirror hit by ~24 cm at 10 m (EST,
    `10 m · tan 1.4°`).
  - `boyko_rhi::Format` has **no** `R32Uint` today (§6.6): new request **RK-17**.
- **Rejected: overwriting RG with the textured normal.** It would change SSAO's input whenever SSR
  is armed, making one consumer silently depend on another consumer's arming.
- **Cost.** 8.3 MB at 1080p, plus two bindless fetches and one tangent load per covered pixel in
  `vb_geo`. EST +0.1–0.2 ms at 1080p, from the same fetch count `vb_shade_split` already pays.

**Decision U3 (revised): SDF pixels on VB get their traced reflection from a post-shade route
(R8v), priced and opt-in. Before R8v they get the environment and probes only.**
- **Why not before shading.** On VB × {Both, Sdf} the marcher is the composite and the last
  `gViewT` writer (§3.12). No SDF-pixel depth, normal or roughness exists before it runs.
- **Why not inline in the marcher (revision 1's form).** Revision 1 did not price it. Priced now
  (§5.1 rates): it runs at full resolution on every smooth SDF pixel, `2.07 M × s × (768 + 608 h)`
  evaluations. Here `s` is the fraction of the screen that is smooth SDF and `h` the hit fraction.
  At `s = 0.5` that is 0.80–1.42 G evaluations, **1.3–7.1 ms** at 1080p (EST). It would also carry
  a heavy span inside the marcher, the case §3.6 forbids.
- **The R8v route.**
  1. **Marker.** The marcher's `-D REFL_SDF_POST` variant stores, for its SDF-owned pixels only:
     - the oct-16 field normal into `thin_refl_n`;
     - the clamped roughness into thin-normal B;
     - a marker into A.

     B and A are unread today, and RG, SSAO's input, is left untouched.
  2. **`refl_trace_sdf`** runs at half resolution over marked pixels with roughness below
     `max_trace_roughness`, after the marcher, when `gViewT` is complete. It is a deterministic
     march along the dominant direction: no noise, no denoiser.
  3. **`refl_delta_compose`** (full resolution, marked pixels) adds
     `F_ss · w · (L_trace − L_fb)` to `lit`. It recomputes the split-sum weight `F_ss` (DFG LUT;
     `pick_material_id` for the specular colour) and the probe/environment fallback `L_fb` with the
     same leaves the marcher used. The result is algebraically `F_ss · (w · L_trace + (1 − w) · L_fb)`,
     D12's single split-sum weight after composition.
- **Cost (EST, §5.1).** The trace costs `0.259 M × s × (768 + 608 h)` evaluations. The compose costs
  `2.07 M × s × 16` evaluations plus about six fetches per pixel.

  | Smooth SDF share `s` | R8v | Inline march (rejected) |
  |---|---|---|
  | 0.1 | 0.06–0.21 ms | 0.26–1.4 ms |
  | 0.5 | 0.34–1.1 ms | 1.3–7.1 ms |

  R8v is 4–6× cheaper at equal coverage.
- **Gates.**
  - An identity gate: with `L_trace := L_fb` the compose leaves `lit` byte-identical. That proves
    the recomputed `F_ss` and `L_fb` match the marcher's.
  - The SDF-mirror analytic-intersection fixture on VB × Sdf.
- **What SDF pixels on VB still lack:** stochastic glossy SSR. Their traced term is a deterministic
  mirror blended into the prefiltered chain by roughness (risk K6). R-SDFSPLIT closes it.

### 3.4 DDGI: what the tail can actually give

Four facts, all post-dating or missed by the 09-10 design:

1. **Probe misses return zero radiance.** `sdf_probe_update.comp.hlsl:452` says a sky miss
   contributes zero radiance. The probe world is the SDF edit list only: meshes neither bounce nor
   occlude, and the sky is black (commit `230585d0`, which calls whether meshes and the sky should
   bounce a scope question). **D21's premise, "the probes integrate the sky at their misses", is
   false today.** Today's additive form ("DDGI is an ADDITIVE indirect term",
   `deferred_pbr.hlsl` beside the `:1265` sample) is the correct one for a sky-less probe. Inside the
   grid it leaks the sky through SDF walls, because the hemisphere term is not occluded.
2. **DDGI arms only on paths with the SDF leg.** It is capped on VB except on `Both`
   (`render_path_config.rs:1466`), filtered by `sdf_leg` in `crates/boyko_app/src/gpu_scene/mod.rs:6480`,
   and degraded on Forward (§3.1). So DDGI exists on Deferred × {Sdf, Both} and VB × Both.
   **Its two diffuse sites** are `deferred_pbr.hlsl:1265-1271` and
   `vb_shade_split.comp.hlsl:553-559`. `ddgi_probe_sample(` has no other shading caller, and the
   marcher has none (§3.3).
3. **The octahedral border precedent was defective when D1/D2 leaned on it.** `230585d0` found 24
   of 28 border texels copied from the opposite edge in `sdf_probe_update.comp.hlsl`'s hand-written
   glue. It was fixed the same day and is now gated by `ddgi_oct_border_host_oracle.rs`, together
   with a `.spv` byte gate that had not existed (`ddgi_probe_update_spv_sync.rs`). The environment
   chain's G4 must be **a host oracle over every border texel of every mip**, in that shape, not a
   single ½-texel fetch.
4. **The bounce is 3.93× darker than before.** It now carries ρ/π. That does not change the design,
   only any tail weight tuned by eye before 09-10.

Also: `ddgi_probe_sample` (`ddgi_resolve.hlsli:107`) **clamps** the receiver into the grid before
the trilinear walk, so it has no notion of "outside the grid". The `w_gi` that D21 needs does not
exist yet.

**Decision U4 (revised): R9a fixes the DDGI transport and lands D21 in the same rung.** Revision 1
landed R9a (sky at misses) without D21. On DDGI-armed pixels the diffuse ambient would then have
gained one extra sky irradiance, and the re-bless would have frozen that into the pins. The review
caught it.
- **The miss arm returns the sky the pixel's own ambient integrates,** so the two halves of D21
  split one sky:
  - without an `EnvironmentLight`, and on every boot without `ReflectionPlugin`, it returns the
    analytic hemisphere of the SKY row: sky colour above the horizon, ground colour below. That is
    the radiance whose cosine integral is `eval_pbr_ambient_hemi`'s diffuse lerp;
  - with an `EnvironmentLight`, it returns the `env_map` top level along the ray. This needs the
    environment bound in the probe update: `sdf_probe_update -D REFL`, one variant row.
- **D21 at both DDGI sites, in the same commit.** The diffuse ambient becomes
  `diffuse_color · (gi + (1 − w_gi) · sky_diffuse) · ao_final`. `sky_diffuse` is the hemisphere
  term, or SH-9 under an `EnvironmentLight`. The SKY block runs *before* the DDGI block
  (`deferred_pbr.hlsl:1238` before `:1265`; `vb_shade_split.comp.hlsl:540` before `:553`), so
  `w_gi` is hoisted above the SKY block. Only the SKY block's **diffuse** term is scaled; its
  specular term belongs to the reflection chain. Every edit sits inside the existing
  `ddgi_mode != 0` branch.
- **`w_gi` is a new leaf, `ddgi_coverage(p, origin, inv_spacing, dims)`.** It is the product of three
  per-axis ramps: 1 inside the grid box shrunk by one probe spacing, 0 at the box faces. A pure
  function, so an eDSL oracle. `ddgi_probe_sample` itself is unchanged, so its bit-exact gate
  (`probe_sample_gpu_eq_cpu_to_bits`, via `ddgi_probe_gi_resolve.comp.hlsl`) stands. Stated limit:
  the tree keeps no per-probe convergence state, so an unconverged grid in the first frames after
  boot under-lights until hysteresis settles, as it does today.
- **Gates (red-first; the pixel gate moves here from 09-10 §2.3):**
  - *probe level:* a constant-sky furnace gives probe irradiance = π·L within `CHANNEL_TOL`. It is
    red today, with a result of 0;
  - *pixel level:* a receiver inside a fully covering grid under a constant sky has diffuse ambient
    equal to the probe irradiance alone, within `CHANNEL_TOL`. The additive form gives about 2×, so
    it is red, and so is a build that fixes the miss without D21. This is the gate that fails on
    revision 1's defect;
  - *outside the grid:* a receiver outside the grid box (`w_gi = 0`) shades byte-identically to the
    DDGI-off image.
- **G5 at R9a is an image gate, not a `.spv` gate.** The transport fix is not reflection-only. It
  changes today's `.spv`: the four `lit`-writing `deferred_pbr` variants, the four `vb_shade_split`
  variants and `sdf_probe_update`. Every DDGI-off pin stays byte-identical, because the edits sit
  inside `ddgi_mode != 0`.
- **What the owner re-blesses (Q7, restated per the review).** Only DDGI-armed pins change. Expected
  change (derived):
  - open-sky receivers stay roughly unchanged: the probe now carries the sky the hemisphere used to
    add, slightly darker where the hemisphere's ground-colour guess is replaced by the SDF ground's
    real bounce;
  - SDF-occluded interiors get **darker**, because the sky no longer leaks through SDF walls.

  A pin that gets *brighter* in the open is the double-count signature and fails review.
- **Cost of R9a.** 2048 probes × the per-frame ray budget, one fetch per miss, plus one
  `ddgi_coverage` per DDGI-armed pixel. EST < 0.02 ms, independent of resolution.
- **The mesh gap.** Mesh walls do not occlude the probes, so a mesh-walled interior still leaks sky.
  It leaks no more than today's unoccluded hemisphere does, and SSAO or specular occlusion remain its
  only damping (a stated limit, §8 risk K5).

### 3.5 Binding limits: neither "grow the set" nor "add a set" is free

Revision 1 derived the budget from the wrong set; the review corrected it. Recounted:

| Resolve-family set | Bindings | Writes `lit`? | Source |
|---|---|---|---|
| software (base, wrap) | **20** (19 shared + `gPbr` @19, a StorageImage) | yes | `gpu_scene/mod.rs` `resolve_software_layout_entries` ("EXACT-FILL at 20") |
| RESOLVE_INLINE-hwrt | 22 | yes | `targets.rs` `RESOLVE_HWRT_BINDINGS` |
| DENOISED | **23** | yes | `RESOLVE_HWRT_DENOISE_BINDINGS`; `crates/boyko_rhi/src/device.rs:70-75` |
| VIS | 23 | **no**, "writes vis, not lit" | [`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md) deferred table |
| VIS + MV | 25 = today's cap | **no** | `RESOLVE_HWRT_VIS_MV_BINDINGS`; `targets.rs:10340` |

- **What reflections add: 5 bindings, not 6.** `boyko_rhi::DescriptorKind` has no standalone
  sampler kind (`enums.rs:723-738`: combined image sampler, sampled image, storage image, uniform
  buffer, storage buffer, acceleration structure). So reflections bind the DDGI shape:
  - `env_map`, `dfg_lut` and `probe_atlas` as three **combined image samplers**, each carrying
    RK-1's trilinear sampler;
  - `EnvUbo`;
  - `refl_color`.

  Revision 1 counted "a trilinear sampler" as a sixth binding.
- **The largest REFL set is DENOISED + 5 = 28** > 25. VIS and VIS + MV never compose reflections.
- **VB sets are far from the cap.** `vb_shade_split`'s lighting set is Set 1, `vb_split_layout1`
  (9 entries, `gpu_scene/mod.rs` `vb_split_layout1_entries`), so it becomes 14.
- **VB is at the Vulkan floor of 4 descriptor sets.** VB binds **four** sets (0 core, 1 shadow,
  2 geometry, 3 bindless textures), which equals the Vulkan-guaranteed floor of
  `maxBoundDescriptorSets` (`device.rs:399-405`).

**Decision U5 (revised).**
- **Raise the cap 25 → 28 (RK-15).** That is exact fill on DENOISED + REFL, the tree's own
  exact-fill tripwire precedent (the 16 → 19 raise "restoring exact-fill"). SSPR writes into
  `refl_color` (§7 RP), so no sixth compose binding is needed. RK-15 is four edits:
  1. **Both copies of the cap:** `crates/boyko_rhi/src/device.rs:76` and the backend's own copy,
     `crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs:97`.
  2. **The pinned test.** `hwrt_resolve_binding_counts_unchanged_by_the_c1_fix` (`targets.rs:10335`)
     ties VIS + MV to the cap (`:10340-10341`). VIS + MV stays 25; the cap-equality assertion moves
     to the DENOISED + REFL set.
  3. **The per-type need constants** of `check_resolve_descriptor_limits`
     (`crates/boyko_rhi_vulkan/src/device.rs:2711-2717`). This is the per-stage guard the cap's own
     doc names (`crates/boyko_rhi/src/device.rs:40-41`). REFL adds 3 combined image samplers,
     1 uniform buffer and 1 storage image (`refl_color`, read by `Load`).
     - **The constants are stale today, before REFL.** They are documented for the 19-binding
       software set. The software set is 20, and its `gPbr` is a StorageImage, so the true
       storage-image need is 7 against a constant of 6. None of the HWRT additions are counted
       (TLAS, `RayShadowUbo`, `gDepthHw`, `gShadowVis`, `MotionCam`, `gMotionVec`).
     - Real devices clear these limits by orders of magnitude (the constant's own doc), so this is
       a guard defect, not a boot failure. But it is the guard RK-15 leans on.
  4. **A host census test.** It counts each resolve-family layout's descriptor kinds and asserts
     each constant equals the maximum over the sets. It is red-first today, on `gPbr`.

  REFL's per-type additions keep combined images at or below the Vulkan-guaranteed 16 samplers and
  sampled images, and uniform buffers at or below 12 [R]. Storage images already exceed the
  guaranteed 4 today, so no new device class is rejected.
- **Append; no new set.**
  - **VB** is at the 4-set floor.
  - **Deferred.** The spec-floor argument does not apply to Deferred, which the review pointed out.
    The tree already knowingly rejects spec-minimum devices (`device.rs:2733`). The deciding fact is
    the RHI surface: the generic `ComputePipelineDesc::bind_group_layout`
    (`crates/boyko_rhi/src/descriptor.rs:261`) declares exactly one set, set 0. Multi-set compute
    pipelines exist only as the Vulkan-concrete `create_compute_pipeline_vb{,_textured}`
    (`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:2470`, `:2591`). A reflection set on Deferred
    would need a generic multi-set surface: new RHI API, for an organisational gain only. The cap
    raise is four edits and changes no emitted bytes.
- **Where the bindings go.** Onto each producer's existing lighting set: the deferred resolve set,
  `vb_split_layout1` for `vb_shade_split`, `vb_layout0` for `vb_resolve`/`vb_shade`, the marcher's
  own Set 0, and `forward_layout1` on Forward.
- **Stage flags.** A shared layout must declare **FRAGMENT | COMPUTE** stage flags. `0973eec2`
  found `forward_layout1` declared FRAGMENT-only while compute pipelines used it
  (VUID-07988 on every validated boot). The reflection bindings on `forward_layout1` inherit that
  lesson from the start.

### 3.6 Zero cost when off: an interface variant, not a runtime bit

- **What 09-10 proposed.** A runtime gate (light-header word 7 bit 7) with the reflection bindings
  always declared.
- **The rule.** Owner rule 3 requires a subsystem that is not used to cost **zero**, with nothing
  built, declared or recorded.
- **The precedent.** `230585d0` proved GI-OFF "by construction": of 117 committed `.spv` exactly one
  changed, and every resolve blob stayed byte-identical.

**Decision U6 (revised).**
- **The axis.** Reflections are a `-D REFL=1` **interface** axis on every `lit` producer. They
  qualify under the manifest's own admission rule ("adds/removes a binding"). A boot without
  `ReflectionPlugin` selects today's pipelines, whose `.spv` stay **byte-identical**, so G5 becomes
  structural rather than a golden observation. (R9a is the one exception, a DDGI transport fix, §3.4.)
- **Price, recounted.** The six producer families own 22 committed `.spv`, but only **20 write
  `lit`**:

  | Family | `.spv` | Of which write `lit` |
  |---|---|---|
  | `deferred_pbr` | 6 | 4 (base, wrap, hwrt, hwrt_denoised; `_hwrt_vis` and `_hwrt_vis_mv` write vis) |
  | `forward_opaque.fs` | 2 | 2 (base, froxel) |
  | `sdf_forward_march` | 4 | 4 (`{HAS_MESH} × {VIEWT}`) |
  | `vb_resolve` | 2 | 2 |
  | `vb_shade` | 4 | 4 (base, froxel, tex, tex_froxel) |
  | `vb_shade_split` | 4 | 4 |
  | **Total** | **22** | **20** |

  REFL adds 20. The whole campaign adds about 45 `.spv` (§6.5). That is P40, and it is made
  mechanical: the §2.1 census test (call sites == sentinel spans) is extended to
  **sites × the 20 REFL variants**.
- **Inside a REFL pipeline: classified, not blanket-branched.** Revision 1 made every sub-feature a
  runtime branch on `EnvUbo.flags`. That goes against the manifest (`SHADER-VARIANT-MANIFEST.md`
  checklist step 5, "never a runtime uniform branch") and a measured precedent:
  `vb_geo.comp.hlsl:132-137` measured a dark march carried inside a kernel at **+64 %** instruction
  footprint (+75 % on `vb_resolve`). The rule now:
  - a span that changes the interface is a `-D` row;
  - a heavy span is a separate pass, never inside a producer;
  - a runtime selection is admitted only for a span measured cheap at the rung that adds it. The
    measure is the SPIR-V size delta (the `vb_geo` method) and the producer's `gpu_zone` time armed
    at zero. Above 3 % (the sibling VFX design's FX10 dark-tax bound) the span becomes a `-D` row.

  | Sub-feature | Span inside the producer | Selection |
  |---|---|---|
  | Environment: map vs analytic sky | one octahedral fetch vs the existing hemisphere lerp, each ~10 instructions | runtime, wave-uniform on `EnvUbo.flags`. An `EnvironmentLight` can be added at runtime, so a specialization constant would force a pipeline rebuild on a scene change |
  | Probes | a loop over `EnvUbo.probe_count` head rows (≤ 64), ≤ 4 accepted (U8) | none: the loop bound; zero trips with no probe |
  | Screen-space / traced answer | one `refl_color` load and a lerp | none: with no tracer armed, `refl_color` is a 1×1 image with `a = 0` |
  | DDGI tail (R9) | reuses the DDGI sample the site already pays | the existing `ddgi_mode` branch |
  | SDF-pixel traced reflection on VB (R8v) | three stores in the marcher | `-D REFL_SDF_POST` (interface: two more storage images); trace and compose are separate passes |
  | Tracers, denoiser, capture, SSPR | none | separate passes, declared in the frame graph or not |

  The heavy span revision 1 would have carried, the inline march in `sdf_forward_march`, no longer
  exists (U3). The light-header word-7 bit of 09-10 is **not needed**; the header stays untouched.

### 3.7 Indirect dispatch: the precedent is particles, and the trait path is a silent no-op

- **The trait method does nothing.** The `RhiCommandEncoder::dispatch_indirect` default is a
  `#[cold] #[inline(never)]` **no-op** (`crates/boyko_rhi/src/encoder.rs:406-410`), and no backend
  overrides it.
- **The raw function exists.** It has been in the Vulkan function table since VG-R1 (`5e46ab44`,
  2026-08-01): `device.rs:599` field, `:2131` load.
- **Its one user** is the particle passes, e.g. `present/passes/particles.rs:514`.
- **The VB cull is host-sized.** Its comment still says `vkCmdDispatchIndirect` is not in "this
  device's fn table" (`present/passes/vb.rs:2790-2791`), which has been stale since VG-R1 (doc rot,
  recorded here, not repaired).
- **09-10 R6 was wrong.** Its "the VB cull chain has one" is false.
- **IndirectCount is absent by design.** `vkCmdDrawIndexedIndirectCount` is "deliberately NOT
  loaded", because it needs `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features` this device
  never chains (`device.rs:671`). Nothing here needs it: the classifier writes the exact tile count
  into the dispatch arguments.

**Decision U7 (revised).** **RK-16 implements the Vulkan override** of
`RhiCommandEncoder::dispatch_indirect` over the already-loaded `fns.cmd_dispatch_indirect`. The
review asked for one choice instead of revision 1's "debug_assert or implement", and implementing
wins: the function has been loaded since VG-R1, so it is a few lines, and it makes the trait path
real instead of loud-but-still-empty. The reflection classifier then records through the trait, not
through raw functions. A pass recorded through it can no longer dispatch nothing and still pass,
which is the "green from emptiness" class.

### 3.8 Probes and Principle 0: no second mirror, and no row a light walk can read

The unification review flags two things 09-10 introduced (`docs/unification/ENGINE-RUNTIME-ECS-RESEARCH.md:529-533`):
- the parallel `ProbeGpu` SSBO, "another mirror";
- the `Captured { every }` source, which needs per-probe cross-frame state and a per-view seam while
  the renderer is single-view.

The unification design keeps `LightTableStaging` as a ScratchColumn Resource and **deletes**
`LightTableDirty` in favour of change detection (`docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md:583`,
`:586`, rung RE5). 09-10 §1.3 gated the probe fold on `LightTableDirty`. The device-backed-column
path (`crates/boyko_render/src/gpu_column.rs`, `GpuColumnManager`) is test-only in production
(`ENGINE-RUNTIME-ECS-DESIGN.md:910`), so it is not a path a new feature should be the first to
depend on.

**Every producer walks the point/spot span flat.** Revision 1 did not say where probe rows sit, and
both spans it could have meant fail:
- **The point/spot span `[l0a_count, light_count)`** is walked with no kind filter, and each loop
  body treats every non-SPOT row as a POINT light:
  - always by `vb_shade_split.comp.hlsl:564` and `sdf_forward_march.comp.hlsl:1086`;
  - by the non-froxel arms of `forward_opaque.fs.hlsl:389`, `vb_resolve.comp.hlsl:441`,
    `vb_shade.comp.hlsl:596` and `deferred_pbr.hlsl:1355`.

  A continuation row's quaternion, half-extents and blend would be read as `pos`, `range` and
  `color`. Probe rows there would also compete with lights for `MAX_LIGHTS_PER_CLUSTER` and
  `INDEX_LIST_CAP`.
- **09-10's "last in the L0a span"** (`REFLECTIONS-DESIGN-SPACE.md` §1.3) is never binned, because
  the cull scans only `[l0a_count, light_count)` (`cluster_cull.hlsl:201-205`, `:473`).

**Decision U8 (revised): a probe is two rows of the existing light table, in a span of its own
after `light_count`.**
- **Where.** The probe span is `[light_count, light_count + 2P)`. Every existing loop is bounded
  inside `[0, light_count)`:
  - the L0a loop by `l0a_count`, and it also filters kinds (`deferred_pbr.hlsl:1011-1013`);
  - the six flat walks and the froxel cull by `[l0a_count, light_count)`.

  So no light walk reads a probe row. The three cull predicates (`cluster_cull.hlsl:324`, `:357`,
  `:476`) stay **unedited**, so the cull `.spv` are unchanged too. Probes never compete for
  `MAX_LIGHTS_PER_CLUSTER` or `INDEX_LIST_CAP`. The header is untouched: word 0 still counts lights
  only.
- **The count** is `EnvUbo.probe_count`, bound only in REFL pipelines and written every frame by
  `upload_env` from `ProbeFoldState`. For a frame-in-flight slot, the table upload and `EnvUbo` are
  both written when that slot is recorded. So a slot never pairs a table with another frame's count.
- **Per-pixel selection: a bounded walk over the head rows, no froxel binning.**
  - Rows are size-ascending. The walk tests each head's sphere and loads the continuation only for
    a candidate. It accepts at most 4 probes whose box contains `P`, and stops when the weight
    reaches 1.
  - Addresses are wave-uniform, so the loads are scalar.
  - Cost (EST): 64 heads × ~6 ALU ≈ 400 ALU per pixel. At 2.07 M pixels on 12.8 TFLOPS that is
    ≈ 0.065 ms for a full 64-probe table, and 0.016 ms at 16.
  - P33 (unbounded per-pixel lists) does not apply: the list is capped at 64 by RK-3 and the loop
    is uniform.
  - Revision 1's cull binning would also have left the non-froxel arms (Forward, VB without
    clusters) with no probe selection at all. A froxel-binned probe list is a recorded follow-up,
    taken only if R2's perf gate at 64 probes exceeds its ceiling.
- **Row bytes.** Each row is one 48-B `GpuLight` (`crates/boyko_render/src/light.rs:124`), and on
  every row the kind word stays `dir_kind.w` at bytes 12..16 (`:128`):

  | Row | bytes 0..12 | 12..16 | 16..32 | 32..48 |
  |---|---|---|---|---|
  | head (`LIGHT_KIND_PROBE = 4`) | reserved 0 | kind word 4 | centre xyz, influence radius | reserved 0 |
  | continuation (kind 5) | half-extents xyz | kind word 5 | rotation quaternion | blend f32; atlas layer u16 + flags u16; intensity f32; 4 B reserved |

  - The payload is 40 of the 44 free bytes. The review noted that the quaternion cannot take bytes
    0..16.
  - The kind words are defensive only, since no walk reads the span; the host oracle checks them.
  - A single-row packing (quaternion as 32-bit smallest-three) fits 44 B exactly. It saves 64 rows
    of 1024 at the cost of a quantised rotation, so it was not taken.
- **Budget.** Lights + 2P ≤ `MAX_LIGHTS` = 1024 (`light.rs:57`). The staging capacity is
  `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES` (`LightTableStaging`'s `Default`). Lights keep
  precedence: the fold clamps P to the rows left and counts dropped probes in a diagnostic, never
  silently. 64 probes take 128 rows.
- **The fold: `fold_probe_rows`.** It is registered only by `ReflectionPlugin` and ordered after
  `collect_lights` in `LightCollectSet`.
  - **Why the trigger matters.** `collect_lights` rebuilds the **whole** staging table on any light
    change: `fold_light_table_slotted` writes into `scratch`, `used_bytes` is reset at
    `light_system.rs:740`, and the generation is bumped at `:748`. A fold triggered only by probe
    changes would lose every probe the moment a light changed. Revision 1 had this defect.
  - **Triggers.** The fold runs when any of these holds:
    - `LightTableGeneration` differs from `ProbeFoldState::seen_gen`, i.e. a light rebuild erased
      the probe tail;
    - `Changed<ReflectionProbe>` or `Changed<Transform>` on a probe entity;
    - a probe was removed. That signal uses the channel `collect_lights` uses today
      (`LightTableDirty`, marked by an `on_remove` hook), and its RE5 replacement afterwards.
  - **Steps.**
    1. Read `light_count` from header word 0.
    2. Truncate to the end of the light span.
    3. Write the 2P rows.
    4. Insertion-sort the 96-B pairs in place (09-10 §1.3's sort, keyed by priority, then
       half-extent volume).
    5. Set `used_bytes` and `dirty`.
    6. Bump `LightTableGeneration` once, and record the new value as `seen_gen`.

    The header is not patched, because the count lives in `EnvUbo`.
  - **One new method on `LightTableStaging`**: truncate to a byte offset and return the tail for
    `n` rows. Its `used_bytes` doc invariant (`light_system.rs:91-92`,
    `LIGHT_HEADER_BYTES + light_count * GPU_LIGHT_BYTES`) gains "plus the probe tail".
- **What it saves.** No `ProbeGpu` buffer, no second upload and no second mirror: the probe bytes
  ride the one `LightTableStaging` (`crates/boyko_render/src/light_system.rs:86`) and its FIF upload.
- **Gates (R2).**
  - *Host, CI, red-first.* The fold's bytes:
    - header word 0 is unchanged by the fold;
    - probe rows sit at `[light_count, light_count + 2P)`, with kind words 4 and 5;
    - a light-only change followed by the fold leaves the probe tail present. This is red on a fold
      triggered by probe changes only;
    - the size-ascending swap fixture.
  - *Device (gpu leg).* A REFL scene holds one probe whose influence covers no visible pixel. Its
    continuation row, **read as a light**, is a point light at the world origin with range 1 m and a
    non-zero colour: the identity quaternion gives `pos_range = (0, 0, 0, 1)`, and blend, layer and
    intensity give `color_cone`. `lit` must be byte-identical to the same REFL scene without the
    probe, on every `lit` producer and both cull arms. Red-first: a build that writes the probe span
    inside `[l0a_count, light_count)` lights a disc at the origin.
  - `probe_row_unpack` host ↔ shader equality (§6.4).
- **Captured probes** carry a `ProbeCapture { every: u16 }` component plus a
  `ProbeCaptureState { next_frame: u32, stage: u8 }` component. This is the structural form
  (Principle 0 rule 3): a baked probe lacks both and no capture system iterates it. It replaces 09-10's
  `ProbeSource::Captured` enum arm.

### 3.9 The occlusion pyramid's base is `prev_pow2`

- `HzbLayout` sizes level 0 as `prev_pow2(width) × prev_pow2(height)` (`hzb.rs:395-396`). At
  1920×1080 that is **1024×1024**: level 0 is ~1.9× coarser than the screen horizontally.
- That is sound for a conservative occlusion bound. It is wrong for SSR, whose thickness test at
  "mip 0" must be per pixel.
- **The 09-10 memory figure was wrong.** Its "`refl_hiz` 11 MB at 1080p" is the 1440p figure
  (2048×1024 × 4 B × 4/3 = 11.2 MB). At 1080p it is 5.6 MB.

**Decision U11.**
- **Level 0 is `gViewT` itself.** The trace forms view-z as `t · dot(rd, fwd)` inline, so nothing is
  copied.
- **Stored levels start at half resolution** with a non-power-of-two reduce: a 3-tap footprint on
  odd edges, the SPD form SSSR uses.
- **What is reused from the occlusion builder** is the `isnan` spelling (`hzb_build.comp.hlsl`, P30)
  and the host-oracle shape. The `prev_pow2` body is **not** reused.
- **Memory.** 2.8 / 4.9 / 11.1 MB at 1080p / 1440p / 4K (arithmetic).

### 3.10 Hit fetch, BLAS identity, and what `vb_geom_fetch` really is

- **The research pass was wrong about the hit fetch.** It said `vb_geom_fetch.hlsli` turns
  `(instance, prim)` plus barycentrics into attributes. It does not.
  - `vb_geom_fetch(instance_id, raw_prim_id, pixel_xy, view_proj, extent)` projects the triangle and
    derives **screen-space** barycentrics at `pixel_xy`.
  - Its reusable halves are `vb_load_index` and `vb_load_vertex`, over the bindless geometry table,
    plus the `vb_interp` eDSL span.
  - A closest-hit needs a sibling, `vb_geom_fetch_bary(row, prim, bary)`: the same loads, weighted by
    the hit's barycentrics.
- **Index spaces must be reconciled.** The TLAS keys `customIndex` to the M3 instance-ring row
  (`build_tlas_instances.comp.hlsl:85-87`), while `vb_geom_fetch` reads `gVbInstances[instance_id]`.
  Whether the two are the same row space was **not verified**. R10a's first gate is a fixture
  asserting that the hit's resolved mesh and material equal the rasterised pixel's for a
  camera-facing mirror.
- **BLAS triangles equal VB's triangles today.** This resolves the research pass's N9.
  `mesh_assets.rs` builds each BLAS from the vertex and index buffers it has just created, the same
  buffers the geometry table references (`crates/boyko_render/src/mesh_assets.rs:525-530`). No
  meshlet LOD exists yet (the VG campaign is at its measurement rung). A future VG LOD rung
  **breaks** that identity and must re-open R10's self-hit budget.
- **Mesh geometry is in device-local memory** since `5e86fe2d`. Host-visible buffers had made shadow
  passes ~40× slower on the RTX 3060. Hit-shading fetches therefore read VRAM.

### 3.11 Other deltas

- **Pins.** There are **34 pins and 61 blessed legs** (34 software + 27 hwrt, 7 hwrt legs
  `PENDING`), recorded at `goldens/PINS.toml:39`, not "30". R4a re-blesses all 61.
- **Particles are a ninth `lit` writer.** `declare_particle_draw` blends into `lit` after the path's
  last `lit` producer and before `taa_resolve`. Its doc also records that `lit` carries
  `COLOR_ATTACHMENT` usage on **all four** paths, which strengthens RK-14's attachment bit. Under
  R4a, particle colours blend in HDR, which is a declared look change shared with post-FX (§10).
  By the 09-10 placement (copy after the last opaque producer), particles are **not** in `lit_prev`.
- **Subgroup ballot is a boot requirement** (`REQUIRED_SUBGROUP_OPERATIONS`, `device.rs:3043`), so
  the classifier may use wave ballots freely.
- **GPU timestamp zones exist** (`present/gpu_zone.rs`). Every rung's perf gate reads one.
- **One queue.** `device.rs:1258` fetches queue 0 of one family, and `synchronization2` is off
  (`:2873`). Nothing here needs either.
- **No transient aliasing.** Frame-graph Phase 2 aliasing is "not started" per the
  [frame-graph plan](../ARCHITECTURE-FRAME-GRAPH-PLAN.md) header, so every reflection transient is
  persistent memory (§5).

### 3.12 VB × Both: the pre-light depth holds meshes only (found during revision 2)

- **The facts.**
  - `vb_viewt` arms on VB only with the mesh leg, and only for (a) TAA without the SDF leg or
    (b) the split's SSAO (`gpu_scene/mod.rs:6659-6670`). It converts `vb_depth`, the mesh raster,
    only.
  - On an SDF-carrying VB leg the marcher is the gViewT producer: "SOLE gViewT producer (`vb_viewt`
    below is mesh-only …)" (`graph_bridge.rs:6095`). It is declared after `vb_shade_split`, and under
    `Both` + SSAO + TAA "the marcher stays the LAST declared writer".
  - On VB × Sdf every pre-light consumer is capped (`render_path_config.rs:1416-1417`).
- **Consequences for revision 1.**
  - Every pre-light pass on VB × Both, including SSR and the reflection pyramid, sees **mesh depth
    only**. SSR there can return a mesh hit *behind* an SDF object, for example a mirror floor
    showing the wall behind an SDF sphere.
  - Revision 1's "SSR is the one tracer that sees both, for free" holds on Deferred only, where the
    composite writes SDF pixels into the G-buffer and `gViewT` before the resolve.
- **Decision U19.**
  - On VB × Both **every** SSR-traced ray, hit or miss, also runs the SDF march. When SSR hit, the
    march is bounded by the SSR hit's world distance. The nearer hit wins through
    `refl_hit_merge`: U10's rule applied to SSR.
  - Cost: R8 runs for the traced fraction instead of the miss fraction, EST × 2.5 (09-10's 20 %
    miss fraction against an EST 50 % traced fraction). The bound shortens marches toward SSR hits;
    the upper bound is 0.33–1.8 ms at 1080p (§5.2 G, row "VB × Both").
  - The `vb_viewt_pre` predicate (`graph_bridge.rs:5650`) becomes `mesh_geo_shade_split && (ssao || ssr)`.
  - Deferred is unaffected.

## 4. Research delta: what shipped systems do now

Rows marked **[D]/[S]/[B]** were re-opened by the author of this file on 2026-09-25. Rows marked
**[R]** are the research pass's and were not re-opened. The web-search budget was exhausted, so every
source here was fetched directly by URL.

| System | What it does (reflections) | Numbers and provenance |
|---|---|---|
| Godot 4.6, PR #111210 [S/D] | Hi-Z tracing; samples the **previous frame's** colour; half-res with upsample, full-res optional; merged 2025-10-21 | RTX 3070 Ti **Laptop**, Sponza, **whole-frame** ms: 64 steps upstream 3.72 / half 3.10 / full 4.28; 512 steps 5.33 / 3.52 / 5.48. SSR-attributable: only the half-vs-full delta, **1.18 ms** (64) and **1.96 ms** (512) |
| Bevy, PR #22379 [D] | "Physically Based SSR", merged 2026-01-16 for 0.19: spatio-temporal blue noise, rough reflections with fade ranges, environment-map blending, temporal accumulation | no numbers. Supersedes the 09-10 table's "no stochastic (#14639)" |
| AMD FidelityFX SSSR 1.5 [D] | Six passes (SPD depth hierarchy of 7 mips, tile classification, 128² blue noise per frame, indirect args, intersection, denoise); above `roughnessThreshold` the fallback environment map is evaluated; `samplesPerQuad` plus variance-guided escalation; inputs include HDR colour, cube environment, BRDF LUT | manual gives no timings; the 2.70 ms / 1080p / RTX 3080 Mobile breakdown is [B] (interplayoflight, 09-10 §4) |
| UE Lumen, performance guide [D] | roughness below **0.4** traces, above it reuses GI; `DownsampleFactor` 1 or 2 (2 = one ray per quad); checkerboard; `RadianceCache=1` reuses diffuse rays for roughness 0.2–0.4; Hit Lighting is "expensive for games" | disabling Lumen reflections for SSR "can save 1 ms on Xbox Series S". **Now first-hand**; 09-10 had it second-hand |
| UE Lumen, technical details [D] | Screen traces first; software RT marches each **mesh distance field for the first two meters**, then the merged global distance field; HW RT may light at the hit instead of the surface cache | Epic scalability ≈ **8 ms GI + reflections at 1080p internal** on current consoles |
| UE mesh distance fields [D] | Generated **offline** ("cannot be done at runtime"); per mesh at most 128³ and 8 MB; no non-uniform scale; WPO artefacts | the cost of making meshes SDF-traceable the UE way |
| kajiya (Embark) [D] | Half-res reflection rays, VNDF, blue noise; hit lit from screen-space irradiance when visible, else from traced diffuse rays; above a roughness threshold no reflection rays are traced; temporal ReSTIR with M clamped to 8, no spatial exchange; dual-source reprojection plus box clamp, then TAA | **~2.2 ms reflections** of an 8.4 ms frame, 1920×1080, **RX 6800 XT** |
| Bevy Solari 0.18 [B, author's blog] | Specular with **bounded VNDF** (Eto & Tokuyoshi); roughness > 0.4 reuses the ReSTIR GI reservoir; up to 3 bounces; DLSS-RR denoise; "specular motion vectors are not implemented" → ghosting | specular pass **0.09–0.61 ms**, frames 7.27–14.06 ms, 1600×900 upscaled to 3200×1800 (the GPU, an RTX 3080 per the research pass, was not re-confirmed) |
| Eto & Tokuyoshi, bounded VNDF, SIGGRAPH Asia 2023 Technical Communications [D, supplement only] | a tighter spherical cap in GGX's stretched space; for `i_z < 0` (back-facing shading normal) it uses the previous PDF (supplement §3); the supplement's Fig. 1: "Our spherical cap bounds the orange line more tightly than the previous spherical cap" | the bound **reduces**, it does not eliminate, reflection directions below the surface; the main paper's PDF did not serve (over 10 MB) |
| Dupuy & Benyoub 2023 [D, abstract] | VNDF sampling with spherical caps, "systematic speed-ups" over Heitz's method | the sampler U13 bounds |
| AMD Brixelizer GI 1.0.1 [D] | Diffuse **and specular** from a runtime sparse SDF built from triangles; specular = quarter-res pre-trace for brick ids, full-res brick march plus radiance cache; `roughnessThreshold` | **no** ms or MB published |
| NVIDIA NRD v4.18.0 [D] | REBLUR / RELAX / SIGMA; needs in-lobe hit distance not divided by pdf, demodulation ("BRDF should be applied after denoising"), non-jittered matrices, VNDF v3, blue noise | RTX 4080 at 1440p: REBLUR diff+spec **2.55 ms**, RELAX 3.25 ms, SIGMA 0.40 ms. Working set REBLUR diff+spec: **148.12 MB at 1080p** (88.88 + 59.25), **262.56 MB at 1440p** (157.50 + 105.06). The two figures are both right; they are different resolutions |
| SSPR, Ghost Recon Wildlands [B, Rémi Génin's blog; affiliation not stated on the page] | Projection pass scatters `PixelY << 16 | PixelX` into R32_UINT with `InterlockedMax` (the pixel nearest the plane wins); resolve decodes; holes filled by temporal reprojection and bleeding the previous frame; N most visible planes into a texture array | "0.3–0.4 ms on consoles at 1/4 resolution" |
| PPR, Cichocki, SIGGRAPH 2017 Advances [R] | Same scatter idea for analytic rectangular reflectors | listing only; slides not parsed |
| AMD Hybrid Stochastic Reflections; RDNA and NVIDIA RT guides; Khronos RT blog [R] | SSSR where it can, RT elsewhere, FSR1 upscale; inline RT is best in compute and wants one RayQuery in scope; divergent shading favours the pipeline; BLAS compaction ~50 % | no reflection ms |
| Unity HDRP SSR [R] | Previous-frame blurred colour pyramid; transparent SSR forces the Approximation algorithm | — |
| Godot SDFGI [R] | Sharp reflections on opaque only; transparents get rough reflections | — |
| Specular occlusion [R] | GTSO (Jimenez et al. 2016) uses bent normals and cones; XeGTAO outputs bent normal plus AO, +~25 % cost | the tree's SSAO has no bent normal (`grep bent` over the shaders finds only `gbuffer_mrt.fs.hlsl`, not SSAO) |

**Corrections to the 09-10 numbers.**
- **C17.** The Godot "3.10 / 3.72 / 4.28 ms" are whole-frame times on a laptop GPU, not SSR pass
  times. Design R5 and survey §4 read them as pass costs.
- **C18.** The Lumen performance-guide figures are confirmed first-hand.
- **C19.** NRD's 262.56 MB is the 1440p figure; 1080p is 148.12 MB.
- **C20.** Bevy SSR is stochastic and temporally accumulated from 0.19.
- **C21.** The spherical-cap VNDF sampler is Dupuy & Benyoub 2023. Heitz 2018 (JCGT) is the
  original VNDF sampler [R]. Revision 1 attributed the cap to Heitz.

## 5. Options against the owner rules, with numbers

### 5.1 Reference GPU and how the estimates are derived

- **Reference GPU: RTX 3060 12 GB,** the owner's box (the golden ledger in `goldens/PINS.toml`
  records its runs).
- **Vendor figures** (NVIDIA product page, re-opened): 3584 CUDA cores, 1.78 GHz boost, 12 GB GDDR6,
  192-bit bus.
- **Derived:** FP32 = 3584 × 2 × 1.78 GHz = **12.8 TFLOPS**. Bandwidth = 192 bit × 15 Gbps / 8 =
  **360 GB/s**; the 15 Gbps data rate is not on the page and is an assumption.
- **Scaling factors** from a published anchor to the RTX 3060. All are EST, derived from relative
  core counts, clocks and bus widths, not measured ratios:

  | Anchor GPU | Multiply the anchor ms by (EST) |
  |---|---|
  | RTX 3080 Mobile / RTX 3070 Ti Laptop (power-limited, 256-bit) | 1.0–1.4 |
  | RTX 4080 | 2–3 |
  | RX 6800 XT (raster/compute; RT throughput differs by vendor) | 1.5–2 |
  | RTX 3080 | 2–2.3 |

- **SDF march rates (EST).**
  - **Throughput: 200–600 M field evaluations per ms,** derived from 09-10 R8, which puts
    0.4 M rays × 48 steps × 16 edits ≈ 300 M evaluations at 0.5–1.5 ms.
  - **Per ray:** the primary march is 48 steps × 16 edits = **768** evaluations.
  - **Per hit, 608 evaluations.** 09-10 R8 shades a hit with the sun and the analytic soft shadow.
    That costs the normal (6 taps × 16 = 96) plus a soft-shadow march of 32 average steps × 16 = 512.
    The 32 average steps are EST; the leaf's cap is `MAX_IT = 128`
    (`sdf_forward_march.comp.hlsl:280`, the loop in `sdf_shadow_leaves.hlsli`).
  - **Formula:** `rays × (768 + 608 h)`, with `h` the hit fraction.
  - **Fractions:** 09-10's 20 % traced-miss fraction is the default. It is scene-dependent: a mirror
    floor facing open space can reach 50–100 % (×2.5–5).
- **Resolution:** screen passes scale with pixels (1440p = ×1.78, 4K = ×4). Probe and environment
  passes do not.
- **CPU:** every rung is GPU-driven. CPU cost is command recording, EST 1–3 µs per pass, plus the
  change-gated probe fold (≤ 64 probes, 128 rows, insertion sort: EST < 10 µs, on change only).
- **Nothing in this section was timed.**

### 5.2 Candidates

| # | Approach | Sees | 1080p / 1440p / 4K ms on RTX 3060 (EST) | VRAM at 1080p (arithmetic) | Owner-rule notes |
|---|---|---|---|---|---|
| A | Analytic gradient plus sun disc (today) | nothing spatial | 0 | 0 | shipping state |
| B | Split-sum IBL, octahedral, DFG LUT (R1) | distant environment | 0.05–0.15 / 0.09–0.27 / 0.2–0.6 | 1.4 MB at the 512² default (U21) | in-house (RGBE decoder, host bake by the `f32` leaf); needs RK-1, RK-11 |
| C | Baked local probes, parallax, ≤ 4 blended (R2, R2b) | local static | +0.05–0.2 / 0.09–0.36 / 0.2–0.8 (the head walk ≤ 0.065 ms included) | 5.6 MB (64 × 128² × 4 B × 4/3) | P0 by U8 (no second mirror; no row a light walk reads) |
| D | Captured probes by compute (R3, U9) | local dynamic; SDF (R3), both representations after R10 | per full probe refresh 0.1–0.26 ms, amortised by the U23 budget (resolution-independent); sky refilter 0.3–0.8, or ~0.1 amortised | transient capture layer 0.09 MB | P0: `ProbeCapture` is structural; no per-view seam needed |
| E | Mirror Hi-Z SSR, half-res (R4b + R5) | on-screen; **both** representations on Deferred, **meshes only** on VB × Both (§3.12) | 0.4–1.0 / 0.7–1.8 / 1.6–4.0 (from SSSR's 0.88 ms full-rate intersect × 1.0–1.4, quartered for half-res, plus upsample and pyramid) | pyramid 2.8 + hit 4.1 + colour 4.1 + `thin_refl_n` 8.3 (VB) ≈ 11–19 MB, plus `lit_prev` (R4a) | needs HDR `lit`, U2 |
| F | Stochastic SSR plus own 3-pass denoiser (R6 + R7), including E | glossy on-screen | 2.7–3.8 / 4.8–6.8 / 10.8–15.1 (SSSR 2.70 ms total × 1.0–1.4) | ≈ 80 MB (88 on VB), 142–157 at 1440p, 318–352 at 4K (§5.3) | in-house, NRD requirements met by construction |
| G | SDF march for SSR misses, half-res, **hit shading included** (R8) | off-screen, **SDF geometry** | Deferred: 0.13–0.72 / 0.24–1.3 / 0.53–2.9 (0.104 M rays × (768 + 608 h) = 80–143 M evaluations at 200–600 M/ms) | ~0 | works on every GPU; the hybrid's unique asset |
| G (VB × Both, U19) | the same march for every SSR-traced ray, bounded by the SSR hit | nearest of mesh-SSR and SDF | upper bound × 2.5: 0.33–1.8 / 0.6–3.2 / 1.3–7.2 | ~0 | the price of the mesh-only pre-light depth |
| G2 | Post-shade SDF reflections for SDF pixels on VB (R8v, U3) | off-screen for VB's SDF pixels | 1080p: 0.06–0.21 at `s = 0.1`, 0.34–1.1 at `s = 0.5` (§3.3); × 1.78 / × 4 | 0 (reuses `thin_refl_n`, thin-normal B/A) | opt-in; the inline march it replaces was 1.3–7.1 at `s = 0.5` |
| H | DDGI tail (R9; exact after R9a) | roughness ≥ 0.7, SDF world | ≈ 0 (a sample the pixel already pays) | 0 | path-limited (§3.4) |
| I | HW inline closest-hit plus hit-lighting lite (R10a/b) | off-screen, **mesh triangles** | 0.4–2.0 / 0.7–3.6 / 1.6–8.0, plus F's denoiser. Band spans kajiya (2.2 ms incl. denoise on RX 6800 XT, ~half of it tracing, × 1.5–2) and Solari (0.09–0.61 ms at 900p on RTX 3080, × 1.44 px × 2–2.3), for SSR-miss pixels only | TLAS/BLAS shared with shadows; hit-distance lane 2 MB | inline only (the RHI has no RT pipeline, and one material model makes a pipeline's per-shader dispatch pointless); owner-eval only |
| J | Radiance atlas plus cone trace, or Brixelizer-class (R11) | glossy off-screen, noiseless | unknown (Brixelizer publishes none) | atlas-dependent | research rung |
| K | Planar via a second full view | exact mirror | scene-dependent re-render; UE: +23.07 ms on a 31 ms frame to +1.67 ms on 11 ms [D, 09-10] | a second target set | needs the per-view seam |
| L | **SSPR** (new rung RP) | one plane, on-screen content | ≤ 0.05–0.3 at ¼ res (upper bound = the published console figure; the console-to-3060 factor is unknown) / ×1.78 / ×4 | hash R32_UINT 2 MB at ½ res (960×540), colour 4 MB, per plane | structural `PlanarReflector` component; needs RK-17 (`R32Uint`); 32-bit image atomics on R32_UINT are required by the spec [R] |
| M | Mesh distance fields for off-screen mesh reflections without `hwrt` | off-screen meshes on every GPU | march cost as G | UE caps 8 MB per mesh at 128³; 50 meshes ≈ up to 400 MB (EST) | a new offline asset class; research door R13 only |
| N | NRD or FFX denoiser as a library | reference quality | NRD 2.55 ms at 1440p on RTX 4080 → EST 2.8–4.3 ms at 1080p on RTX 3060 | 148 MB at 1080p | **rejected**: foreign shader toolchain outside the eDSL (rules 5, 6). Used as the reference design only |

**R8 still moves ahead of R10 with hit shading counted on both sides.** G is 0.13–0.72 ms against
I's 0.4–2.0 ms, and G runs on every GPU. Revision 1 compared G's march alone (0.4) with I including
hit shading (2.0).

### 5.3 Memory at R7, recomputed (arithmetic, no aliasing)

| Image | Shape | 1080p |
|---|---|---|
| `refl_hiz` levels ≥ 1 (U11) | half-res R32F, 4/3 chain | 2.8 MB |
| `refl_hit` | half-res RGBA16F | 4.1 MB |
| `refl_color` (trace) + resolved full-res | half + full RGBA16F | 4.1 + 16.6 MB |
| `lit_prev` ×2 | full B10G11R11 (×2 on the RGBA16F arm) | 16.6 MB |
| `refl_hist` ×2 | full RGBA16F | 33.2 MB |
| `refl_var` ×2 | half-res R16F | 2.1 MB |
| `refl_tiles` | 240 × 135 × 4 B | 0.13 MB |
| `thin_refl_n` (VB only, U2; also R8v's SDF normals) | full R32_UINT | 8.3 MB |
| **Total** | | **≈ 80 MB (88 on VB)**; ×1.78 at 1440p, ×4 at 4K |

This is under 0.8 % of the reference box's 12 GB at 1080p and under 3 % at 4K. Frame-graph aliasing
would recover EST ~25 MB at 1080p (hit, half-res colour, variance) but is **not** made a prerequisite
(decision U17).

## 6. The recommended architecture (updated)

### 6.1 The cascade, per representation

| Order | Source | Mesh pixels | SDF pixels | Coverage |
|---|---|---|---|---|
| 1 | SSR over `lit_prev` (R5–R7) | Deferred ✔; VB ✔ (U2), seeing meshes only on VB × Both | Deferred ✔; VB: none (no thin-aux before the marcher) | on-screen |
| 1b | SDF march merged with the SSR hit (U19, R8) | VB × Both: every traced ray | — | fixes SSR's mesh-only depth on VB × Both |
| 2 | Nearest traced hit `t = min(t_hw, t_sdf)` (U10) | rayQuery (`hwrt`) and SDF march | Deferred: same; VB: R8v (SDF march only) | off-screen; meshes only with `hwrt` |
| 3 | Local probes (R2/R2b baked; R3 compute-captured) | ✔ | ✔ | local |
| 4 | DDGI tail (R9, exact after R9a) | where DDGI arms | where DDGI arms (not VB's marcher, §3.3) | roughness ≥ 0.7 |
| 5 | Environment (R1) | ✔ | ✔ | always; weight `1 − Σw` |

**Decision U10: the Both leg takes the nearest of two representations.** Cascading from a miss in
one representation to the other returns a farther hit whenever the other occludes nearer.
- **With `hwrt`:** both are traced, and `refl_hit_merge` keeps the nearer hit. The second trace is
  bounded by the first hit's distance. This is the `HAS_MESH` precedent: `sdf_forward_march.comp.hlsl`
  already bounds its march by the rasterised mesh surface. The resolve's
  `deferred_pbr.hlsl:1121` `vis = min(vis, mesh_vis)` is the same rule for shadows.
- **Which goes first is measured at R10a** (the review's O2; revision 1 argued "hardware first"
  with no number). Both orders return the same nearest hit by construction:
  - SDF first with a rayQuery `TMax = t_sdf` lets the BVH cull every node beyond the SDF hit;
  - hardware first with `t_max = t_hw` shortens the march.

  A fixture bench at R10a runs both orders on an SDF-dominant and a mesh-dominant showcase scene.
  The cheaper becomes the default, possibly per leg set.
- **Without `hwrt`:** SDF only, and the stated error is that meshes are invisible off-screen.
  Probes (baked with meshes, R2b) and the environment are its ceiling. Option M is the only door to
  closing that gap without `hwrt`, and it is a research rung (R13).

**Decision U9 (revised): runtime capture is a compute pass, and the offline bake waits for HDR.**
- **R2** lands the probe machinery (rows, atlas, fold, compose) and gates it with host-filled
  content: tiles filtered from the environment chain and analytic fixtures. None of its gates needs a
  scene bake.
- **R2b (new, after R4a): the offline bake.**
  - **Frames.** The tool renders six sequential single-view frames from the probe centre with a 90°
    square camera. Sequential single-view frames need no per-view seam.
  - **TAA off, jitter zero,** for the bake frames; otherwise the 90° jumps between faces blend
    unrelated histories.
  - **HDR readback.** It reads back `lit` in HDR (B10G11R11 or RGBA16F after R4a; not `display`).
    Then the host `f32` leaves convert the faces to the octahedral tile and prefilter it.
  - **Why after R4a.** Before R4a, `lit` is `R8G8B8A8Unorm` post-tonemap (`targets.rs:2279`). A bake
    then would store clipped, tonemapped radiance beside an HDR environment chain and break G7's seam
    energy.
- **R3: `probe_capture {SDF}`.** It writes the octahedral layer of a captured probe by marching the
  field from the probe centre, the DDGI probe-update shape. Hits are shaded with the sun and the
  environment. The GGX chain is then filtered by R3's `BufferLoad` convolution (09-10 D13 unchanged).
- **`probe_capture {SDF+HW}` moves to R10b.** It needs R10a (`vb_geom_fetch_bary`, the index-space
  fixture) and R10b (mesh hit shading). Revision 1 listed it at R3, before either existed.
- **Why compute capture.** It removes R3's dependency on the per-view seam, which the renderer does
  not have (single `ViewUniform`).
- **What still needs the seam:** only the planar second view (K).

### 6.2 ECS landing (Principle 0)

| Kind | Name | Holds / does | Zero when off because |
|---|---|---|---|
| Plugin | `ReflectionPlugin` | registers everything below; selects the `-D REFL` pipelines (U6) | absent ⇒ today's `.spv`, no rows, no passes |
| Resource | `ReflectionConfig` (owner) → `ResolvedReflections` (boot-frozen, `resolve_reflection_policy`, the `SsaoConfig` pattern) | env mode, probes, `SsrMode`, `TracedMode` (incl. R8v), `max_trace_roughness` = 0.4, tail = 0.7 (U24), `ProbeCaptureBudget` = 0.3 ms (U23), SSPR on/off | not inserted |
| Component | `EnvironmentLight { asset: EnvHandle, intensity, yaw }` | at most one active; writes the single SKY row (09-10 D20 unchanged) | no entity ⇒ analytic sky |
| Component | `ReflectionProbe { shape, half_ext, influence, blend, priority, intensity }` + `Transform` | two light-table rows past `light_count` (U8) | no entity ⇒ no rows |
| Component | `ProbeCapture { every: u16 }`, `ProbeCaptureState { next_frame: u32, stage: u8 }` | captured probes only | a baked probe lacks them; the capture system never iterates it |
| Component | `ProbeLayer(u16)` | atlas layer, assigned on `Added<ReflectionProbe>` from a free list in a Resource-owned `ScratchColumn` | — |
| Component | `PlanarReflector { normal: [f32; 3], d: f32, half_ext: [f32; 2] }` | an SSPR plane (rung RP) | no entity ⇒ no SSPR passes |
| Resource | `ProbeFoldState { seen_gen: u64, probe_rows: u16, dropped: u16 }` | the fold's generation watermark (U8, W3) and the count `upload_env` writes into `EnvUbo` | not inserted |
| Resource | `EnvAssets` | append-only resource-owned column of `EnvAssetHeader` (per-asset resolution, U21) plus cold `EnvSh9` (09-10 §1.1) | empty |
| Resource | `CaptureQueue` | ≤ K probe ids for this frame, a `ScratchColumn`; written by `schedule_probe_captures` | empty ⇒ no dispatch |
| NonSend | `ReflectionTargets` | atlases, pyramid, histories: the FFI/GPU-contiguity class (the DDGI atlas precedent) | not created |
| Systems | `fold_probe_rows` (after `collect_lights`; triggers per U8); `schedule_probe_captures` (budgeted, U23); `upload_env` (the RF1 FIF-mirror protocol, `ENGINE-RUNTIME-ECS-DESIGN.md:911`; writes `EnvUbo.probe_count`) | all on the engine scheduler; no heap, no locks, no virtual dispatch in the per-frame path | not registered |

### 6.3 Frame order (Deferred and VB; brackets = rung)

```
... last pre-light gViewT producer (Deferred: the composite; VB: vb_viewt, armed for SSR too - U19)
[R4c] vb_geo -D REFL           VB: thin_refl_n + textured roughness in B (U2)
[R4b] refl_hiz_build           gViewT → view-z, levels >= 1, non-pow2 reduce (U11)
[R6 ] refl_classify            tiles + indirect args (trait dispatch_indirect, RK-16/U7)
[R5 ] refl_trace               Hi-Z on refl_hiz, level 0 = gViewT; reads lit_prev[1-fi] (D8/D19)
[R10] refl_trace_hw            rayQuery closest-hit from the U1 origin, hitT, ray-cone LOD (hwrt)
[R8 ] refl_trace_sdf           misses (Deferred) / every traced ray, bounded by t_ssr (VB × Both, U19)
[R6 ] refl_resolve             half → full ratio estimator
[R7 ] refl_reproject → refl_prefilter → refl_temporal   (own history, demodulated)
[RP ] sspr_project → sspr_resolve  (per visible PlanarReflector; writes refl_color)
[R3 ] probe_capture → env_filter   (budgeted; independent of screen passes)
      resolve / vb_shade_split (-D REFL)   ← refl_compose
      sdf_forward_march (-D REFL [-D REFL_SDF_POST])   VB: marks SDF pixels (U3)
[R8v] refl_trace_sdf (post) → refl_delta_compose       VB SDF pixels only
[R4a] lit_prev copy            after the last opaque producer, before particle_draw (U16; fog: §10)
      particle_draw, translucents (transparency campaign)
      taa_resolve (pre-tonemap under R4a)
[R4a] tonemap                  lit (HDR) → display   (post-FX's post_final, PX1)
      fxaa / smaa / present_sample
```

### 6.4 eDSL surface (additions to 09-10 §4)

| Leaf | Rung | Kind | Gate |
|---|---|---|---|
| `raster_origin(px, py, view_t, raster_fwd, cam)` | R10a | oracle over parameters | the CI host far-floor fixture (U1), red on the camera-ray point; host equality with `raster_ray_forward`; the device 8-phase band |
| `oct16_pack` / `oct16_unpack` | R4c | oracle | round-trip error < 0.01° over a direction grid |
| `refl_hiz_reduce` (non-pow2, `isnan` spelling) | R4b | oracle | G6 polarity plus an odd-extent fixture (1921×1081) |
| `hiz_reflect_trace` / `hiz_cell_step` | R5 | emit-only / oracle | as 09-10 |
| `vndf_bounded_sample` + pdf (Eto & Tokuyoshi 2023, bounding Dupuy & Benyoub 2023's spherical-cap sampler) | R6 | oracle | the pdf integrates to 1 over a sample grid; the below-horizon count is **≤ the unbounded cap sampler's** on the same (α, i) grid and strictly lower on a grazing subset; a rejected sample carries weight 0 (unbiased); `i_z < 0` takes the previous PDF |
| `refl_hit_merge(t_a, t_b)` | R8/R10 | oracle | tie and ordering fixture; used by U10 and U19 |
| `refl_delta(F_ss, w, L_trace, L_fb)` | R8v | oracle | `L_trace == L_fb` ⇒ zero delta, bit-exact |
| `ddgi_coverage(p, origin, inv_spacing, dims)` | R9a | oracle | 1 at an interior point, 0 on a face, monotone ramp over one spacing |
| `env_radiance_dir` (the R9a miss arm: hemisphere, or `env_map` under REFL) | R9a | oracle over parameters | a constant-sky furnace gives probe irradiance = π·L within `CHANNEL_TOL` |
| `ray_cone_spread` / `ray_cone_lod` (Akenine-Möller et al., JCGT 2021) | R10b | oracle | LOD equals the analytic footprint on a flat mirror |
| `bary_interp` (for `vb_geom_fetch_bary`) | R10a | oracle | equals `vb_interp` at a pixel centre for a camera-facing triangle |
| `probe_row_unpack` (head plus continuation) | R2 | oracle | host `fold_probe_rows` bytes ↔ shader unpack equality |
| `sspr_hash_encode` / `sspr_hash_decode`; `sspr_project` | RP | oracle / emit-only (atomics) | a known-plane fixture maps each source pixel to its mirrored target |

### 6.5 Variant rows (to be added to [`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md) per rung)

| Family | Rows |
|---|---|
| `-D REFL` on the 20 `lit`-writing producer variants (U6) | +20 |
| `vb_geo -D REFL` (× {base, MV, SV0}) | +3 |
| `sdf_forward_march -D REFL -D REFL_SDF_POST` (× `{HAS_MESH} × {VIEWT}`, VB legs; R8v) | +4 |
| `sdf_probe_update -D REFL` (R9a's environment miss arm) | +1 |
| `refl_hiz_build` | 1 |
| `refl_trace` {MIRROR, STOCHASTIC} | 2 |
| `refl_trace_hw` {HIT_SHADOW 0/1/2} (cs_6_5) | 3 |
| `refl_trace_sdf` (one interface; pre- and post-shade selection by push constant) | 1 |
| `refl_delta_compose` | 1 |
| `refl_resolve`, `refl_reproject`, `refl_prefilter`, `refl_temporal` | 4 |
| `probe_capture` {SDF (R3), SDF+HW (R10b)} | 2 |
| `env_filter` | 1 |
| `sspr_project`, `sspr_resolve` | 2 |
| **Total** | **≈ 45** |

R9a also changes the bytes of nine existing `.spv` (§3.4); those are re-blessed rows, not new ones.
The census test counts the REFL rows, and its product with the sibling VFX design's `-D WEATHER`
axis (§8 K2).

### 6.6 RHI and kernel gaps (checked in code at `6394bc5e`)

| Gap | Where | Needed by | Request |
|---|---|---|---|
| Trilinear sampler | `crates/boyko_rhi/src/device.rs:248-252` (`MipMode::None` only) | R1–R3 | RK-1 (09-10) |
| Mipped raw upload | `crates/boyko_render/src/texture.rs:403` | R1 | RK-11 (09-10) |
| B10G11R11 storage and attachment probe | `crates/boyko_rhi_vulkan/src/device.rs:276-281` (storage optional); attachment bit still unread in the spec (§12) | R4a, captured atlases (U22) | RK-14 (09-10): probe both bits, fallback RGBA16F |
| Binding cap, in **two** copies | `crates/boyko_rhi/src/device.rs:76` and `crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs:97`, both 25 | R1+ | **RK-15** 25 → 28, the pinned test re-aimed, the per-type need constants re-derived and census-gated (U5) |
| No `Format::R32Uint` | `crates/boyko_rhi/src/enums.rs:270` `Format`: `R32Sfloat = 100` (`:380`), `R32G32Uint = 101` (`:386`), no 98 | U2 `thin_refl_n`, RP hash | **RK-17** add `R32Uint = 98` (storage and 32-bit image atomics are required for it by the spec [R]; RP still probes `STORAGE_IMAGE_ATOMIC` at boot) |
| No standalone sampler descriptor | `enums.rs:723-738` `DescriptorKind` | U5 | none: combined image samplers (the DDGI shape), so REFL adds 5 bindings |
| Generic compute pipeline declares one set | `crates/boyko_rhi/src/descriptor.rs:261` `ComputePipelineDesc::bind_group_layout` | U5 (why append) | none |
| Indirect dispatch through the trait | `crates/boyko_rhi/src/encoder.rs:406-410` silent no-op | R6 | **RK-16** implement the Vulkan override (U7) |
| IndirectCount | `device.rs:671` "deliberately NOT loaded" | none (exact counts) | — |
| Per-layer view cap | `MAX_TEXTURE_LAYERS = 16` (09-10 RK-3) | > 16 probes | RK-3 |
| RT pipeline / SBT | `ffi.rs:946` (absent by design) | none (inline chosen) | — |
| Async compute, sync2 | one queue (`device.rs:1258`); sync2 off (`:2873`) | none | — |
| Transient aliasing | frame-graph Phase 2 not started | memory only (§5.3) | — (U17) |
| Per-view seam | single `ViewUniform` | K (planar second view) only, after U9 | MPR R11 (09-10 RK-8), scope narrowed |

## 7. The revised ladder

Every rung has three gates:
- a **red-first** gate: the test is proven red on a build that lacks the rung;
- a **golden** gate: a new pin or a declared re-bless;
- a **perf** gate: a `gpu_zone` for the rung, run only in an owner-granted timing window. An overrun
  of the EST upper bound is a red that sends the rung back to design, never to re-estimation. Counts
  are asserted in CI (G11).

**Phase A: foundations (every path).**

| Rung | Adds | Red-first | Golden | Perf (1080p, RTX 3060, EST ceiling) |
|---|---|---|---|---|
| R0 | RGBE decoder, DFG LUT `.bin`, host bake, leaves (09-10) | G1 furnace red on the analytic fit at `pR > 0.7, NoV < 0.2` | — | bake only |
| R1 | env chain at the six sites, REFL variants (U6), RK-1, RK-11, RK-15 | G4 **all-border-texel host oracle** (the `ddgi_oct_border_host_oracle` shape, §3.4); G3 Godot #69514 fixture; the RK-15 need-constant census (red today on `gPbr`) | G5 structural (the 22 existing producer-family `.spv` unchanged) plus `grand_showcase_ibl` | 0.15 ms |
| R2 | probes as two rows past `light_count` (U8), `fold_probe_rows`, `EnvUbo.probe_count` | the U8 host byte gates (incl. the light-change-then-fold case); the device "probe rows are invisible to light walks" fixture | +1 armed pin | +0.2 ms (head walk included); fold < 10 µs CPU |
| R4a | HDR `lit` + tonemap tail + `lit_prev` copy (shared with transparency R11 and post-FX PX1, §10) | one transition pin per producer (8) **plus the particle blend** (§3.11) | **all 61 legs re-bless** (owner ballot Q1) | +0.1 ms, +25 MB |
| R2b (new) | offline probe bake: six single-view frames, TAA off, HDR readback (U9) | a baked probe of a known scene equals the host-rendered reference within `CHANNEL_TOL`; red on an LDR (`display`) readback | +1 baked-probe pin | offline |

**Phase B: screen space (Deferred, VB).**

| Rung | Adds | Red-first | Golden | Perf |
|---|---|---|---|---|
| R4c (new) | `vb_geo -D REFL` (U2), RK-17 | `thin_refl_n` equals `vb_shade_split`'s normal on a normal-mapped fixture (red on the geometric normal); thin-normal B equals its textured roughness (red on the scalar) | 0 pins move (unarmed) | +0.2 ms on VB |
| R4b | pyramid, level 0 = `gViewT` (U11) | G6 polarity; an odd-extent fixture red on a `prev_pow2` base | — | 0.1 ms, 2.8 MB |
| R5 | mirror SSR half-res; `vb_viewt` arms for SSR (U19) | G7 seam energy; the mirror-floor column fixture (also measures the grazing residual of §3.2) | +1; the four `ssr_on` assertion sites re-blessed | 1.0 ms |
| R6 | bounded VNDF (U13), blue noise, classify via the trait's `dispatch_indirect` (RK-16), resolve | the variance-reduction count gate; tile count == roughness census; the VNDF rejection-count comparison (§6.4) | — | cumulative 2.0 ms |
| R7 | own denoiser, demodulation, `refl_hist` | G8 TAA-off/on identity; G9 demodulation; disocclusion `a == 1` | — | cumulative 3.8 ms; 80 MB |

**Phase C: off-screen without special hardware.**

| Rung | Adds | Red-first | Golden | Perf |
|---|---|---|---|---|
| R8 | SDF march with hit shading (half-res): misses on Deferred, every traced ray on VB × Both (U19); `refl_hit_merge` | SDF mirror at the analytic intersection; a VB × Both fixture (mirror floor, mesh wall behind an SDF sphere) that shows the sphere, red on SSR alone; the miss and traced fractions asserted as counts | +1 SDF-mirror pin | 0.72 ms (Deferred, 20 % fraction); 1.8 ms (VB × Both); the cost-vs-edit-count and cost-vs-fraction curves in the gate |
| R8v (new) | post-shade SDF reflections for VB's SDF pixels (U3): marcher marks, `refl_trace_sdf` (post), `refl_delta_compose` | the `L_trace := L_fb` byte identity; SDF mirror at the analytic intersection on VB × Sdf | +1 VB × Sdf mirror pin | 0.21 ms at an `s = 0.1` view, 1.1 ms at `s = 0.5`; the cost-vs-`s` curve in the gate |
| R9a (new) | DDGI miss returns the sky (hemisphere, or `env_map` under REFL) **and D21 at both DDGI sites, with `ddgi_coverage`**, in one commit (U4) | probe furnace (red today: 0); **pixel furnace: diffuse ambient == probe irradiance alone** (red on the additive form and on a miss-only fix); outside-grid identity | DDGI-armed pins re-bless with the §3.4 expected change (owner ballot Q7); every DDGI-off pin byte-identical | < 0.02 ms |
| R9 | DDGI tail (the replacing form; R9a precedes it) | a furnace over a covering grid | — | ≈ 0 |
| R3 | compute capture `{SDF}` (U9) + GPU filter (09-10 D13, bit-equal to the host bake) | GPU chain == host bake; a capture of a known SDF box equals the host march | — | 0.26 ms per probe refresh; K from the budget (U23) |

**Phase D: hardware (`hwrt`, owner-eval only).**

| Rung | Adds | Red-first | Golden | Perf |
|---|---|---|---|---|
| R10a | closest-hit rayQuery from the U1 origin (`raster_origin`), hitT, `vb_geom_fetch_bary`, index-space fixture (§3.10), nearest-hit merge (U10) with the order bench | the CI host far-floor fixture (U1); hit mesh/material == raster pixel's; a Both-leg fixture with a mesh occluding an SDF behind it, red on a miss-cascade; the device 8-phase mirror band, red on the camera-ray origin | owner-eval | 2.0 ms incl. hit shading |
| R10b | hit-lighting lite (09-10 R10b), ray-cone LOD, `HIT_SHADOW` variants, `probe_capture {SDF+HW}` | the ray-cone LOD fixture; a capture of a known mesh box equals the host reference | owner-eval | inside R10a's ceiling |

**Phase E: planar.**

| Rung | Adds | Red-first | Golden | Perf |
|---|---|---|---|---|
| RP (new) | SSPR for `PlanarReflector` (hash projection, resolve into `refl_color`, temporal hole fill); the classifier skips reflector pixels for SSR | the known-plane mapping fixture; **coverage:** after `refl_compose` no reflector pixel has total cascade weight < 1 (count == 0; holes fall through to probes and the environment). The SSPR hole count before the fallback is reported, not gated: holes are expected where the reflected content is off-screen | +1 water pin | 0.3 ms per plane |
| K | second-view planar | — | — | behind the per-view seam; owner ballot Q5 |

**Research doors (not rungs):**
- R11: radiance atlas plus cone trace;
- R13: per-mesh distance fields (option M);
- bent-normal specular occlusion (09-10 T10);
- SSPR on Forward through the R8v post-shade delta-compose shape (§14, open question 2).

**Landing order**, forced by prerequisites:

R0 → R1 → R2 → R4a → R2b → R4c → R4b → R5 → R6 → R7 → R8 → R8v → R9a → R9 → R3 → R10a → R10b → RP.

- **R8 moves ahead of R10.** It is the only off-screen tracer on every GPU, and with hit shading
  counted on both sides it costs less (0.72 ms against 2.0 ms ceilings).
- **R2b follows R4a** because a bake needs HDR `lit` (U9).
- **RP is independent after R4a.** It can move earlier if the water of the transparency or VFX
  campaigns needs it.

## 8. Risks

- **K1: D2 stays unresolved.** Every other consumer that rebuilds `P` from the camera ray (SSAO,
  SSCS, the CSM lookup) keeps its `2j` error under the default scope. Reflections avoid it where it
  bites (R10, U1); their *comparison* against those consumers in a golden does not.
- **K2: variant growth.** About 45 new `.spv` and 20 new producer variants. The sibling VFX design's
  `-D WEATHER` axis (FX10) sits on an armable subset of the same producers, so the axes multiply: up
  to 20 × 2 × 2 = 80 producer `.spv` if both land. The census must count the product. The memory
  record of a hung `dxc.exe` stalling a workspace run applies, so re-DXC runs per rung, not in one
  batch.
- **K3: the binding-cap raise.** RK-15 touches two inline-array caps, one pinned test and a per-type
  guard that is already stale (§3.5). The byte-neutral argument must be re-proven by the updated
  tests, not asserted.
- **K4: VRAM without aliasing.** 80 MB at 1080p and up to 352 MB at 4K (§5.3).
- **K5: DDGI's SDF-only world.** On mesh-heavy Both boots the tail and captured probes see no meshes
  without `hwrt`; mesh walls leak sky (§3.4).
- **K6: asymmetry on VB.** SDF pixels on VB get no SSR and no stochastic glossy chain until
  R-SDFSPLIT. Their traced term is R8v's deterministic mirror (U3). SSR on VB × Both sees meshes
  only, and U19's merge is what restores occlusion by SDF objects.
- **K7: Forward receives no SSR or SSPR** (U12).
- **K8: inline divergence** at glossy hits (NVIDIA's warning, [R]). Bounded by one material model,
  but bindless fetches at incoherent hits stay divergent. R10's perf gate decides whether hit texture
  fetches drop to a fixed ray-cone mip.
- **K9: seven `hwrt` golden legs are `PENDING`.** The hardware arm's owner-eval base is itself
  incomplete.
- **K10: reusing a precedent that had a defect.** §3.4's border finding is the example. Every
  borrowed rule (octahedral borders, origin rule, `min` pyramid) gets its own oracle in this
  campaign rather than inheriting "already oracle'd".
- **K11: fog in `lit_prev`.** The volumetrics design places the `lit_prev` / `scene_color_copy` copy
  **after** `fog_apply` (its D17). An SSR sample then carries the fog of the camera-to-hit path
  instead of the reflector-to-hit path, and the reflector pixel is fogged again by `fog_apply`. That
  is negligible in light fog and wrong in dense fog. It is decided jointly at R4a/PX1 (§10).
- **K12: the traced fractions are scene-dependent.** R8's and U19's ceilings assume 09-10's 20 % miss
  fraction and an EST 50 % traced fraction. A mirror floor facing open space can multiply R8 by up to
  5. The gate records the fractions as counts, so an overrun is attributable.

## 9. Decisions taken here (technical forks; not owner questions)

| # | Decision | The number or fact that decides it |
|---|---|---|
| U1 | On-surface origins for R10a/b only; `raster_origin` leaf; a CI host far-floor fixture plus the device band | only a world-space triangle trace self-hits (§3.2); `999babfb`: up to 57 % false shadow; the Deferred rule's inputs exist only on HWRT frames |
| U2 | `vb_geo -D REFL`: textured roughness in B, a normal-mapped `thin_refl_n` R32_UINT oct-16 lane (RK-17) | B is unread today; R32_UINT storage is core; 8-bit oct ≈ 24 cm error at 10 m (EST) |
| U3 | SDF pixels on VB: env and probes; traced term by the post-shade R8v (opt-in); no inline march | inline 1.3–7.1 ms vs R8v 0.34–1.1 ms at `s = 0.5`; no SDF data before the marcher on VB |
| U4 | R9a = miss radiance **and** D21 **and** `ddgi_coverage` in one commit, with the pixel furnace | `sdf_probe_update.comp.hlsl:452`: misses return 0; a miss-only fix counts the sky twice |
| U5 | Cap 25 → 28 in both copies; the pinned test re-aimed; the need constants re-derived and census-gated; append, no new set | DENOISED 23 + 5 = 28; VIS/VIS + MV do not write `lit`; `ComputePipelineDesc` takes one set |
| U6 | `-D REFL` interface axis on 20 producers; sub-features classified (runtime only when measured cheap) | owner rule 3; the `230585d0` precedent; `vb_geo`'s +64 % dark-span measurement |
| U7 | RK-16 implements the Vulkan `dispatch_indirect`; the classifier records through the trait | the trait default is a silent no-op; the raw function has been loaded since VG-R1 |
| U8 | Probe span `[light_count, light_count + 2P)`; count in `EnvUbo`; flat head walk; fold re-triggered by the light-table generation | six producers walk `[l0a_count, light_count)` flat; `collect_lights` rebuilds the whole table; 64-probe walk ≈ 0.065 ms (EST) |
| U9 | Compute capture `{SDF}` at R3, `{SDF+HW}` at R10b; offline bake R2b after R4a, TAA off, HDR readback | no per-view seam; `lit` is RGBA8 post-tonemap before R4a |
| U10 | Both leg: `min(t_hw, t_sdf)`; which is traced first is measured at R10a | a miss-cascade returns farther hits; both orders give the same hit by construction |
| U11 | Reflection pyramid level 0 = `gViewT`; half-res stored levels; non-pow2 reduce | `prev_pow2` makes level 0 1024 wide at 1920 |
| U12 | Forward/ForwardPlus get R1–R3 only; RK-12 moves to a Forward pre-light campaign | Forward degrades every pre-light consumer (`:1395`) |
| U13 | Bounded VNDF sampling (Eto & Tokuyoshi) over the spherical-cap sampler; gated by a rejection-count comparison, not by zero | the supplement: the cap "bounds … more tightly", so it reduces rejections without removing them; shipped in Solari 0.18 |
| U14 | Ray-cone texture LOD at hits; `vb_geom_fetch_bary` sibling; an index-space fixture first | no screen derivatives at a hit; `vb_geom_fetch` is screen-space |
| U15 | SSPR (RP) is the first planar rung; the second view stays behind the seam | 0.3–0.4 ms on consoles at ¼ res vs UE's +1.67 to +23 ms |
| U16 | `lit_prev` stays before `particle_draw` (one copy shared with transparency's `scene_color_copy`) | refraction must not see particles; including particles is owner ballot Q3 |
| U17 | Aliasing is not a prerequisite | ≤ 3 % of 12 GB at 4K |
| U18 | RK-14 probes both B10G11R11 bits | the spec table did not serve again (§12); a boot probe makes the answer non-load-bearing |
| U19 | VB × Both: every SSR-traced ray also runs the SDF march bounded by `t_ssr`; nearest wins; `vb_viewt` arms for SSR | the pre-light depth on VB × Both is mesh-only (§3.12); +×2.5 on R8 there |
| U20 | RK-17: `Format::R32Uint = 98` | no R32 integer format exists; `thin_refl_n` and the SSPR hash need it |
| U21 | Environment bake default 512² octahedral (per-asset, `EnvAssetHeader`); probe tile 128² | 4π/N² per texel: 0.40° at 512 vs 0.79° at 256; 1.4 vs 0.35 MB; identical runtime cost (one fetch); only the offline bake scales ×4. Tile 128: 5.6 MB vs 22.4 MB for 64 probes, and R3's capture cost scales with texels (EST 0.26 → ~1 ms per refresh at 256); probes show only where the tracers above them miss |
| U22 | Probe and environment atlases B10G11R11, RGBA16F where RK-14's probe finds no storage support (captured atlases are written by compute) | half the memory and fetch bandwidth; 6-bit (R, G) / 5-bit (B) mantissas are 1.6 % / 3.1 % relative steps, against ≈ 1.7 % for one 8-bit sRGB step at mid-grey (`2.2 × (1/255) / 0.5`); the specular term that consumes them is scaled by `F ≤ 1` |
| U23 | Capture cadence K = ⌊budget / measured per-probe cost⌋, at least 1; `ProbeCaptureBudget` = 0.3 ms | K = 1 at the EST upper bound of 0.26 ms; 64 captured probes refresh in 64 frames (1.07 s at 60 Hz); the budget follows the Q2 configuration |
| U24 | `max_trace_roughness` 0.4; DDGI tail full weight at 0.7 | 0.4: Lumen's default and Solari's split, both first-hand [D]. 0.7: the reflected GGX lobe's half-maximum half-angle ≈ 2·atan(0.644·α), α = r² (small-angle, normal incidence, EST): 11.7° at r = 0.4 and 35° at r = 0.7, against 60° for the cosine lobe DDGI stores. Between them the probe's prefiltered chain is the correct lobe by construction. Lumen already reuses its diffuse gather above 0.4 [D], so 0.7 is the conservative side |
| U25 | `.hdr` authoring conventions (up-axis, exposure pre-scale) are mandatory bake settings with no default | a wrong up-axis silently rotates the sky (the P10 class); a mandatory field turns it into a bake error. A correctness call, not a look |

Unchanged 09-10 decisions: D1–D3, D5, D6 (the storage form changed by U8), D7 (built per U11),
D8–D14, D17, D18 (capture now by U9), D19 (extended by U1), D20, D21 (now landed by R9a, U4).
Superseded: the word-7 bit-7 gate (by U6), R3's per-view-seam prerequisite (by U9), and 09-10's
"probe rows last in the L0a span" (by U8).

## 10. Interfaces with the sibling topics of this research batch

These topics are designed in their own documents; the interfaces are recorded so the batch lands
them once.

- **Post-FX and general AA** ([`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md)).
  - R4a is that design's rung **PX1** and transparency's R11. It covers HDR `lit`, one tonemap tail
    (grown there into the `post_final` uber pass), TAA pre-tonemap, and FXAA/SMAA/present reading
    `display`.
  - It must land **once**, by whichever campaign goes first, with the 8 + 1 per-producer transition
    pins (§7 R4a). Both documents count the re-bless as 61 legs.
- **VFX: explosions and rain** ([`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md)).
  - Particles are a `lit` writer that becomes an HDR blend under R4a (§3.11). Whether emissive VFX
    appear in SSR is owner ballot Q3.
  - The VFX design's weather shading (`wet_surface`, rung FX10) lowers roughness on wet surfaces
    and puddles through a `-D WEATHER` axis on the producers. The reflection chain reads that
    roughness through thin-aux on VB (U2 carries the textured roughness), so puddles classify as
    smooth and are traced.
  - Rain puddles are also the natural first client of SSPR (RP): one `PlanarReflector` per puddle
    plane set.
  - The two producer axes multiply (§8 K2).
- **Sun rays and volumetrics** ([`VOLUMETRICS-DESIGN-SPACE.md`](VOLUMETRICS-DESIGN-SPACE.md); its
  light shafts are the sun rays).
  - **The sun in the environment** (09-10 ballot 11, the P11 triple count): a baked sun in the map
    and an analytic sun disc for shafts must not both light the reflection. It stays an owner
    ballot (Q8), shared with that design.
  - **`lit_prev` after `fog_apply`.** That design's frame order places the one `lit_prev` /
    `scene_color_copy` copy after `fog_apply` (its D17). Reflections would prefer pre-fog radiance
    (risk K11). A second, pre-fog copy costs +16.6 MB and ≈ 0.05 ms at 1080p (EST, the Q3 copy
    figure). The choice is made jointly when R4a/PX1 lands.
  - **Separate froxel grids.** The volumetrics froxel volume (160×90×64) is not the light cluster
    grid (16×9×24). Probes are not binned into either (U8), so revision 1's "the same froxel grid the
    probes bin into" no longer applies.
  - Both chains use the camera-only motion reprojection that R7 uses.

## 11. Owner value questions (values and scope only)

Revision 1's Q4 (resolutions, atlas format), the cadence half of Q5, and the thresholds and `.hdr`
conventions of Q9 were technical forks. They are now decided in §9 (U21–U25). What remains:

- **Q1. HDR scene colour (R4a = PX1).** Re-blesses all **61** legs and is shared by three campaigns
  (transparency, reflections, post-FX). Recommended **before R5**; nothing screen-space is right
  without it.
- **Q2. Default configuration and its frame budget.** Ceiling sums at 1080p on the RTX 3060, all EST
  (§7 ceilings; R3, R9 and RP excluded):

  | Config | Adds | Deferred × {Sdf, Both} | VB × Mesh | VB × Both, SDF-heavy (`s = 0.5`) |
  |---|---|---|---|---|
  | A | env + probes (R1, R2) | 0.35 ms | 0.35 ms | 0.35 ms |
  | B | + HDR, pyramid, mirror SSR (R4a, R4b, R5; R4c on VB) | 1.55 ms | 1.75 ms | 1.75 ms |
  | C | + stochastic + denoise (R6, R7) | 4.35 ms | 4.55 ms | 4.55 ms |
  | D | + SDF (R8; U19 on VB × Both; R8v on VB) | 5.07 ms | — (no SDF leg) | 4.55 + 1.8 + 1.1 = **7.45 ms** |
  | E | + hardware (R10) | 7.07 ms | 6.55 ms | 9.45 ms |

  Revision 1's D (4.6 ms) omitted hit shading, U19 and the SDF pixels on VB. Which configuration is
  the showcase default? The capture budget (U23) and per-plane SSPR (0.3 ms) come on top.
- **Q3. Should VFX appear in reflections?** Copying `lit_prev` after `particle_draw` makes explosions
  and fire reflect in SSR. The price: +8 MB and a second copy (EST 0.05 ms), and the glow sits at the
  depth of the surface behind the particle. The default (U16) excludes them, like HDRP and Godot.
- **Q4. Probe scope.** Is RK-3 (more than 16 probes, up to 64) in this campaign, and is runtime
  capture (R3) in this campaign or its own?
- **Q5. Planar reflections:** SSPR (RP) for water and puddles only, or also the exact second view
  (K) for mirrors, at "half your frame"?
- **Q6.** Hardware-ray reflections (R10) in scope, or parked with the rest of the `hwrt` track
  (owner-eval only; seven `hwrt` legs already `PENDING`)?
- **Q7. Grant the DDGI-armed re-bless of R9a** (the transport fix plus D21, §3.4). The expected
  change: open-sky receivers roughly unchanged, SDF-occluded interiors darker. A pin that brightens
  in the open is rejected as a double count.
- **Q8. Carried from 09-10 §13.B (look or scope):**
  - the sun baked into the environment or kept analytic (shared with the volumetrics shafts, §10);
  - material extensions (coat, anisotropy, sheen);
  - R11 as a planned rung or a research door.

## 12. Unverified or open

- **The mandatory-format table for B10G11R11 did not serve** from three URLs this pass
  (`docs.vulkan.org` formats chapter; the `Vulkan-Docs` `formats.adoc` source). Truncated before the
  tables both times. U18 makes the answer non-load-bearing.
- **R32_UINT's mandatory storage and atomic support** and the per-type descriptor minimums (16
  samplers, 16 sampled images, 12 uniform buffers, 4 storage images) are carried [R] from the
  research pass, for the same reason. RP still probes the atomic bit at boot.
- **Bounded VNDF.** Only the supplement served (the main PDF exceeds the fetch limit). The gate is
  now a comparison against the unbounded sampler, so no absolute rejection figure is relied on.
- **The index-space identity** between the TLAS `customIndex` (M3 ring row) and `gVbInstances` rows
  (§3.10) is unverified. It is R10a's first fixture.
- **Every millisecond in §5 and §7 is EST.** The scaling factors are derived from relative
  core/clock/bus figures, not measured. No per-pass reflection timing on an RTX 3060 exists in
  public. Of the SDF figures, the soft-shadow step average (32) and the traced fraction (50 %) are
  the least grounded.
- **The Solari GPU** (RTX 3080) comes from the research pass; the blog page re-opened here did not
  state it in the fetched excerpt.
- **The SSPR author's employment** is not stated on the page. The technique description is first-hand
  from the post; the "shipped in Ghost Recon Wildlands" attribution rests on the post's title.
- **The Lumen SIGGRAPH 2022 deck, the Battlefield V deck, the PPR slides and Ray Tracing Gems
  chapters** remain unread (PDF or behind auth).
- **09-10 §14 items still open:** Filament's LUT channel convention (not depended on); reflection-
  capture normalisation and HDRP sky occlusion (unsourced); K-th-probe popping (unsolved everywhere).

## 13. Sources

**Re-opened by the author of this file (2026-09-25):**
- https://github.com/godotengine/godot/pull/111210
- https://github.com/bevyengine/bevy/pull/22379
- https://gpuopen.com/manuals/fidelityfx_sdk/fidelityfx_sdk-page_techniques_stochastic-screen-space-reflections/
- https://gpuopen.com/manuals/fidelityfx_sdk/techniques/brixelizer-gi/
- https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/mesh-distance-fields-in-unreal-engine
- https://github.com/EmbarkStudios/kajiya/blob/main/docs/gi-overview.md
- https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18
- https://github.com/NVIDIA-RTX/NRD/blob/master/README.md
- https://remi-genin.github.io/posts/screen-space-planar-reflections-in-ghost-recon-wildlands/
- https://www.nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti/
- Revision 2: https://yusuketokuyoshi.com/ (the bounded-VNDF entry: SIGGRAPH Asia 2023 Technical
  Communications, DOI 10.1145/3610543.3626163) and its supplementary document,
  <https://yusuketokuyoshi.com/papers/2023/Bounded_VNDF_Sampling_for_Smith-GGX_Reflections_(Supplementary_Document).pdf>
- Revision 2: https://arxiv.org/abs/2306.05044 (Dupuy & Benyoub, spherical-cap VNDF; abstract)
- Did not serve the needed content: https://docs.vulkan.org/spec/latest/chapters/formats.html,
  https://raw.githubusercontent.com/KhronosGroup/Vulkan-Docs/main/chapters/formats.adoc; the
  bounded-VNDF main PDF (over the fetch size limit); https://dl.acm.org/doi/10.1145/3610543.3626163
  (HTTP 403)

**From this session's research pass [R], not re-opened here:**
- https://bevy.org/news/bevy-0-17/ and https://bevy.org/news/bevy-0-18/
- https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17/
- https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-global-illumination-and-reflections-in-unreal-engine
- https://gpuopen.com/manuals/fidelityfx_sdk/techniques/denoiser/
- https://gpuopen.com/fidelityfx-hybrid-reflections/
- https://gpuopen.com/learn/getting-the-most-out-of-fidelityfx-brixelizer/
- https://gpuopen.com/learn/rdna-performance-guide/
- https://developer.nvidia.com/blog/best-practices-for-using-nvidia-rtx-ray-tracing-updated/
- https://www.khronos.org/blog/ray-tracing-in-vulkan
- https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceFeatures.html
- https://advances.realtimerendering.com/s2017/ (PPR listing)
- https://bitsquid.blogspot.com/2017/06/reprojecting-reflections_22.html
- https://jcgt.org/published/0007/04/01/ (Heitz 2018, the original VNDF sampler)
- https://www.jcgt.org/published/0010/01/01/ (ray cones)
- https://research.activision.com/publications/archives/practical-real-time-strategies-for-accurate-indirect-occlusion
- https://github.com/GameTechDev/XeGTAO
- https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md
- https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html
- https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Override-Screen-Space-Reflection.html
- https://arxiv.org/abs/2607.20384 (Split Radiance Cascades; diffuse only)

**Inherited:** every URL in [`REFLECTIONS-RESEARCH.md`](REFLECTIONS-RESEARCH.md) §8.

**In this tree (`6394bc5e`), by content:**
- Docs:
  - [`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) (D2 at `:75`, VB D7 at `:130`);
  - [`ENGINE-RUNTIME-ECS-RESEARCH.md`](../unification/ENGINE-RUNTIME-ECS-RESEARCH.md) `:529-533`;
  - [`ENGINE-RUNTIME-ECS-DESIGN.md`](../unification/ENGINE-RUNTIME-ECS-DESIGN.md) `:583`, `:586`,
    `:910`, `:911`;
  - [`ARCHITECTURE-FRAME-GRAPH-PLAN.md`](../ARCHITECTURE-FRAME-GRAPH-PLAN.md) (header);
  - [`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md) (the deferred table; the checklist);
  - [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) (R11);
  - [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md);
  - `goldens/PINS.toml:39`.
- Sibling documents of this batch (same worktree): [`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md),
  [`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md), [`VOLUMETRICS-DESIGN-SPACE.md`](VOLUMETRICS-DESIGN-SPACE.md).
- Code:
  - `crates/boyko_render/src/{render_path_config.rs, upload.rs, view.rs, taa_config.rs, hzb.rs,
    texture.rs, light.rs, light_system.rs, mesh_assets.rs, gpu_column.rs}`;
  - `crates/boyko_app/src/{runner.rs, gpu_scene/mod.rs}`;
  - `crates/boyko_rhi/src/{device.rs, enums.rs, encoder.rs, descriptor.rs}`;
  - `crates/boyko_rhi_vulkan/src/{device.rs, ffi.rs, brick_atlas.rs, rhi_impl/mod.rs,
    rhi_impl/device.rs, present/targets.rs, present/graph_bridge.rs, present/passes/vb.rs,
    present/passes/particles.rs, accel_build.rs}`.
- Shaders (`crates/boyko_rhi_vulkan/shaders/`): `deferred_pbr.hlsl`, `forward_opaque.fs.hlsl`,
  `vb_resolve.comp.hlsl`, `vb_shade.comp.hlsl`, `vb_geo.comp.hlsl`, `vb_shade_split.comp.hlsl`,
  `sdf_forward_march.comp.hlsl`, `cluster_cull.hlsl`, `light_table.hlsli`, `vb_geom_fetch.hlsli`,
  `vb_shadow_vis.comp.hlsl`, `viewt_from_depth_rz.comp.hlsl`, `sdf_probe_update.comp.hlsl`,
  `ddgi_resolve.hlsli`, `ddgi_probe_gi_resolve.comp.hlsl`, `sdf_field.hlsli`,
  `sdf_shadow_leaves.hlsli`, `build_tlas_instances.comp.hlsl`.
- Commits: `999babfb`, `978625a1`, `197e2155`, `230585d0`, `5e86fe2d`, `0973eec2`, `5e46ab44`.

## 14. Review log (revision 2)

The review of revision 1 is kept verbatim outside the repository, in the session scratchpad
(`research/reflections.critique.md`). Each remark was re-checked against the code at `6394bc5e`
before it was acted on.

| # | Remark (short) | Verdict | Resolution |
|---|---|---|---|
| C1 | Probe rows are read as point lights by the flat walks; the R2 gate cannot see it; the span is unstated; the quaternion cannot take bytes 0..16 | **Accepted** (confirmed: all six walks, `light.rs:124-133`) | U8: the span moves past `light_count`, so no walk reads it and no cull predicate is edited; the count goes in `EnvUbo`; the row bytes keep `dir_kind.w`; a device fixture fails when a walk lights a continuation row (§3.8, §7 R2) |
| C2 | R9a lands before D21, so the sky is counted twice; Q8 blesses it | **Accepted** (confirmed: additive sites at `deferred_pbr.hlsl:1271`, `vb_shade_split.comp.hlsl:559`) | U4: R9a = miss radiance + D21 + `ddgi_coverage` in one commit; the pixel furnace moves into R9a; Q7 (was Q8) restated with the expected direction (§3.4) |
| W1 | Both SDF-march costs understated (hit shading; U3 unpriced) | **Accepted** | Hit shading priced (768 + 608 h per ray, §5.1); G re-derived (0.13–0.72 ms); U3's inline form priced (1.3–7.1 ms at `s = 0.5`) and replaced by R8v with its own rung and ceiling; Q2 re-derived as a table |
| W2 | Sub-features as runtime branches, against the manifest and the +64 % precedent | **Accepted** | U6: each sub-feature classified; heavy spans are passes or `-D` rows; runtime only when measured under 3 % (§3.6) |
| W3 | The fold loses probe rows whenever a light changes | **Accepted** (confirmed: `light_system.rs:740`, `:748`) | U8: the fold also triggers on `LightTableGeneration`, truncates to the light span, re-appends, bumps the generation once; the header is not patched (count in `EnvUbo`); a host gate covers the light-change case |
| W4 | Wrong variants counted (vis variants do not write `lit`); budget from the wrong set; per-type limits skipped; the spec-floor argument is VB-only | **Accepted** | 20 producer variants (§3.6); budget from DENOISED: 23 + 5 = 28; RK-15 covers both cap copies, the pinned test and the need constants (found stale today); "no new set" now rests on `ComputePipelineDesc` taking one set (§3.5) |
| W5 | U1's red-first gate cannot fail before R10a; hwrt-only plumbing | **Accepted** (the SSR part as PLAUSIBLE, as the review rated it) | U1 moves to R10a; a CI host world-space fixture that the camera-ray build fails; no non-hwrt plumbing (§3.2) |
| W6 | Bake before R4a stores LDR; the SDF+HW capture precedes its prerequisites | **Accepted** | New R2b after R4a (TAA off, HDR readback); `probe_capture {SDF+HW}` moves to R10b (U9, §7) |
| W7 | Owner questions that are technical forks | **Accepted** | Decided with numbers: U21 (resolutions), U22 (atlas format), U23 (cadence), U24 (thresholds), U25 (`.hdr` conventions); §11 keeps only look and scope |
| W8 | RHI gaps: no `R32Uint`, no standalone sampler, IndirectCount without evidence | **Accepted** (confirmed: `enums.rs:270-390`, `:723-738`, `device.rs:671`) | RK-17; 5 bindings, not 6; IndirectCount cited (§3.7, §6.6) |
| O1 | Spherical cap is Dupuy & Benyoub, not Heitz | **Adopted** | §6.4, C21 |
| O2 | "Hardware first" argued without numbers | **Adopted** | U10: the order is measured by a fixture bench at R10a |
| O3 | RK-16 offers a choice, not a decision | **Adopted** | RK-16 implements the Vulkan override (U7) |

**Answers to the review's open questions.**
1. **Bounded VNDF gate.** The review's recollection holds. The supplement's own words are that the
   new cap "bounds … more tightly than the previous spherical cap", and for `i_z < 0` the method
   falls back to the previous PDF. The bound reduces below-horizon samples; it does not remove them.
   The gate is now a count comparison against the unbounded sampler (§6.4).
2. **SSPR.**
   - **Value over R5 (EST, not measured).** SSPR replaces a depth march with one scatter for the
     plane's pixels: ≤ 0.05–0.3 ms per plane at ¼ resolution, against R5's 0.4–1.0 ms for the
     half-resolution screen. It has no thickness ambiguity, being a scatter and not a march, and
     needs no stochastic chain for a mirror. Its value is where SSR is off (configuration A) or where
     planes dominate (water).
   - **Forward.** SSPR must finish before the producer composes, so it is a pre-light consumer and
     waits for the Forward seam, like SSR (U12). The R8v post-shade delta-compose shape could lift
     that; it is recorded as a research door.
   - **"Holes == 0".** It is redefined: coverage after the fallback is gated (no reflector pixel
     below total weight 1); the SSPR hole count before the fallback is only reported (§7 RP).
3. **Sibling documents.** The post-FX/AA, VFX and volumetrics (sun rays included) documents exist in
   this batch, in the same directory. §10 now links them and records three interfaces revision 1 did
   not have:
   - PX1 = R4a;
   - the multiplied `WEATHER` × `REFL` axes;
   - `lit_prev` after `fog_apply`.

**Found during revision 2, beyond the review.**
- **The marcher has no DDGI consumer** (`sdf_forward_march.comp.hlsl:1038`). Revision 1's claim
  that SDF pixels on VB get the DDGI tail was false (§3.3).
- **VB × Both's pre-light `gViewT` is mesh-only** (§3.12). SSR there cannot see SDF objects, so U19
  merges it with the SDF march; `vb_viewt` must arm for SSR.
- **`ddgi_probe_sample` clamps into the grid** (`ddgi_resolve.hlsli:107`). D21's `w_gi` has no
  existing source, so it becomes a new leaf (§3.4).
- **Two copies of the binding cap exist**, and the pinned test ties VIS + MV to the cap (§3.5).
- **`check_resolve_descriptor_limits`' need constants are stale today**, since `gPbr` is uncounted.
  This is a guard defect independent of reflections; RK-15's census makes it red-first (§3.5).
- **R9a changes existing `.spv`**, because it is a DDGI transport fix, not a reflection feature. G5
  for R9a is therefore stated at image level (§3.4).
