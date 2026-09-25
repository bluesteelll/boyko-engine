# Volumetrics — the design for THIS engine

> Status: architect's design, 2026-09-25, **pass 2** (critique round 1 resolved). The review log is
> §12. It covers light shafts, froxel fog, local media, sky and atmosphere, clouds, and
> smoke/fire/explosions as volumes.
>
> - **The survey it rests on** is [`VOLUMETRICS-RESEARCH.md`](VOLUMETRICS-RESEARCH.md): §0 there
>   is the tree today, §2 numbers the techniques **T1–T31**, §4 normalizes every published cost to
>   the reference GPU, and §5 numbers the pitfalls **P1–P29**. This document cites all of those by
>   number.
> - **Verification.** Every `file:line` below was re-opened by content at trunk **`6394bc5e`**
>   (worktree `D:/wt/docs`). No `cargo` command was run and nothing was timed.
> - **Numbers.** Each number is either published and cited with its rig, measured in the tree and
>   cited with its file (**[M]**), or an **estimate** labelled **[E]** with its derivation. Every
>   scaled anchor uses one method (RESEARCH §4.1): the conservative end is the smallest applicable
>   hardware ratio, and gates and budgets use that end.
> - **Decisions.** §10.A holds the perf/architecture decisions taken here, each with its number;
>   §10.B holds the only questions that go to the owner, which are VALUES/SCOPE.
> - **Siblings.** The pair [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) R4a /
>   [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) R11 owns the one prerequisite
>   this campaign cannot do without: the HDR scene colour. [`POSTFX-AA-DESIGN-SPACE.md`](POSTFX-AA-DESIGN-SPACE.md)
>   §5.2 owns the pre-exposure scalar this design consumes from V2 (D23).
> - **Anchors.** This directory is not machine-anchored: `GATED_DOCS`
>   (`tests/internal_docs_anchors.rs:349`) names no `render/` document.

## 0. What the earlier documents decided — and what this one keeps and changes

This is a **delta document**. The earlier decisions live in
[`OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md) Track E and in three siblings.

### 0.1 Decided earlier

| Where | Decision |
|---|---|
| E-FOG (`:433`) | A 160×90×64 frustum-aligned 3D texture filled by a compute scatter pass, then a front-to-back integrate. "The froxel XY×Z IS the cluster grid, so fog and opaque lighting share one cull". Needs P1 + **P6-S** + P7. Relaxed golden. Class RENDER. |
| E-SDFVOL (`:451`, cost `:461`) | The fog scatter pass calls `field_distance` toward each light. **P9-required above the ≤16-edit fixture.** The field probe stays FROZEN; accumulation may relax. Class FIELD-CONSUMER. |
| E-DENS-A / E-DENS-B (`:471`, `:489`) | A density channel either independent of the field (A, in the brick atlas or a separate 3D image), or remapped from an already-evaluated `field_distance` (B: fast-math on the remap only, never in the probe). Both threshold-gated on P9. |
| E-CLOUD (`:507`) | Perlin-Worley ray-marched layer; in-house noise; P5 half-res + P6-S reprojection; XL; threshold-gated. |
| E-GOD (`:525`) | Route (a) is "FREE as a byproduct of E-SDFVOL". Route (b), screen-space radial blur, is **demoted to optional-only**. |
| E-PART (`:610`) | "particles inject density into E-FOG". |
| VRAM row (`:853`) | "E-FOG 160×90×64 ×RGBA16 ×2 — ~30 MB". |
| [`RENDER-AA-AND-TAILS-PLAN.md`](../RENDER-AA-AND-TAILS-PLAN.md)`:109` | Volumetrics deliberately deferred to their own design pass. |
| [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) D7/F2 (`:486`), OQ1 (`:2172`) | Particles composite into LDR `lit`. HDR is an open owner question. |
| REFLECTIONS R4a = TRANSPARENCY R11 | `lit` becomes B10G11R11 (RGBA16F fallback). The tonemap leaves eight producers for one tail pass. |

### 0.2 Unchanged

- **The froxel volume is the single volumetric primitive inside its range.** Shafts are its shadowed in-scatter (T4). Every surveyed shipped engine with volumetric fog uses a froxel volume for it (T5–T8). That holds for the near and mid field only: UE also ships screen-space Light Shafts (T1) and shadowed atmosphere shafts for the far field (T24). §4.6.3 and D1 decide the far field here.
- **Screen-space radial blur stays out of the volume's range.** Inside `z_far` it duplicates what the volume already gives. Beyond `z_far` it is the only candidate that sees distant on-screen casters, and it is scoped as rung V13 behind owner question Q4 (D1).
- **The determinism firewall** (`OPTIMIZATION-PLAN-RENDER.md` §0a, `:50`):
  - the fog is RENDER;
  - the SDF-caster term is FIELD-CONSUMER through the unmodified gateway `sdf_field.hlsli`;
  - the E-DENS-B relaxation applies to the remap only.
- **In-house everything.** The noise is ours, the sky reference code is re-implemented (it is MIT, but it is not linked), and no NanoVDB.
- **Clouds and dense local volumes stay threshold/owner-gated.** They are behind the fog core.

### 0.3 Changed, and why

| # | Earlier text | Now | Why (RESEARCH §) |
|---|---|---|---|
| C1 | fog grid == cluster grid, 160×90×64 | A fog-owned grid, **160×90×64 default, 160×90×128 High**, resolution-independent. Lights come from a **second instance of the unchanged cluster-cull kernel** over a fog-only light table at 16×9×32. | The cluster grid is 16×9×24 over 0.1–50 m, VB-only (`render_path_config.rs:1252`) and 8-bit per axis (`light.rs:956-959`); 64/24 is not an integer (§0.2, P17). |
| C2 | needs P6-S | P6-S is **not** a prerequisite. | Semaphores ship (`present/frame_driver.rs:48`), and two cross-frame histories run today: the `taa_hist` ping-pong (`targets.rs:468-480`) and the persistent single DDGI probe atlas (`ddgi.rs:155-156`) (§0.9–0.10). The cited `queue.rs` does not exist. |
| C3 | "~30 MB" for two volumes | Two volumes are 14.7 MB. The chain chosen here is **three** volumes plus a fog far cascade = **≈ 27.0 MB** at Low (§3.2). | Arithmetic (P18) and O1 of the review (§12). |
| C4 | (absent) | Fog composes on the **HDR** `lit` in **one** apply pass. R4a/R11 is a hard prerequisite. | Fog added after the tonemap is not light transport (P15). |
| C5 | E-SDFVOL "P9-required above 16 edits" | The trigger has not fired: the gateway caps at 16 (`sdf_field.hlsli:48`). SDF-caster visibility runs through the frozen analytic gateway at **half resolution per axis behind a per-kind, penumbra-inflated edit-bounds prefilter** (§3.3). Valid for ≤ 16 edits only; the render plan's 256–4096-edit regime needs P9. | §0.4, P19, P29 |
| C6 | E-DENS-A "second R16 brick-atlas channel" | Deferred. The atlas is a 40³ SAMPLED near-field over [-4, 4]³ (`compute.rs:4340`, `brick_atlas.rs:79-89`), not a scene structure. | §0.4 |
| C7 | E-GOD route (a) "FREE" | Free for SDF casters only. Mesh casters need a **normal-free** CSM/atlas tap, which is new, and a **fog far cascade** where the CSM's shadow distance is below the fog range (D21). | §0.3, P26 |
| C8 | (absent) | **Sky and atmosphere added** as a rung (Hillaire 2020). They are the lighting context for fog tails, sun colour and clouds. | T23, T24 |

---

## 1. The shape in one paragraph

**The data.** A frame's participating media live in one **frustum-aligned froxel volume** of fixed size: 160×90×64 by default, independent of output resolution, like AC4 and Doom Eternal (T5, T8).

**The chain.** Compute passes fill it. Each is an ECS-declared render-graph pass that exists only when the fog is armed:

1. **`fog_sun_far_depth`** renders the shadow casters past the CSM's last split into a fog-only 1024² depth map, the **fog far cascade**. It exists only when the CSM is armed and its shadow distance is below the fog range (D21).
2. **`fog_inject`** evaluates the medium and the light arriving at each jittered froxel sample. The medium is:
   - analytic height fog × procedural noise;
   - plus up to 32 visible `FogVolume` entities.

   The light is:
   - the hemisphere or DDGI ambient;
   - the sun, through a normal-free 1-tap lookup in the surface cascades or, past their last split, in the fog far cascade; on SDF legs, also a half-resolution field-march visibility volume;
   - every `VolumetricLight`-marked punctual light, from a fog-only cluster list.

   The result is multiplied by the frame's **pre-exposure** (D23). It is blended with its reprojected history at 5 % (Frostbite), the history rescaled by the pre-exposure ratio.
3. **`fog_integrate`** walks each of the 160×90 columns front to back with Frostbite's energy-conserving integral.
4. **`fog_apply`** is one full-resolution pass on the **HDR** `lit`. It sits after the last opaque producer and before the scene-colour copy and the transparents. It writes `L·T + S` at each pixel's depth, and an analytic height-fog tail past the volume's far plane.

**Transparents and particles** fetch the same integrated volume at their own depth, through an always-bound, runtime-gated binding (D24).

**Shafts** fall out of the shadowed in-scatter, for the sun and for any marked light, mesh or SDF caster, on-screen or not, anywhere inside the volume's range (T4). The volume's resolution sets a sharpness ceiling: one froxel is 16 × 16 px at 1440p (§4.6.4). Past the range the tail is unshadowed, and far-field shafts are owner-scoped (Q4, V13).

**The ECS surface:**

- `VolumetricsConfig` + `GlobalMedium` resources, with a derived `ResolvedVolumetrics` and a `FogStats` counter resource;
- a unit `VolumetricLight` marker;
- `FogVolume` + `Enabled<FogVolumeActive>` components;
- one `VolumetricsPlugin`, default **Off**.

Off means nothing is declared, built, allocated or recorded, and every shipped shader stays byte-identical through V7. V8 re-emits the particle VS once (D24).

**Later rungs:**

- the sky and atmosphere LUTs (T23), whose f32 eDSL instantiation also computes the sun's colour on the host;
- particle density injection;
- 2.5D clouds;
- dense local volumes for hero explosions;
- far-field screen-space occlusion shafts (V13, owner Q4).

---

## 2. The design space — every option scored against the owner rules

### 2.1 Scores

The owner rules are:

- **R2 HYBRID perf:** numbers at 1440p on the RTX 3060 Laptop, RESEARCH §4.
- **R3 Principle 0:** structural capability, zero cost when unused.
- **R4 hot path:** no allocation, locks or virtual dispatch.
- **R5 in-house.**
- **R6 eDSL / variants / RHI.**

| # | Option | R2 cost @1440p [E] and what it covers | R3 | R4 | R5 | R6 | Verdict |
|---|---|---|---|---|---|---|---|
| A | Screen-space radial blur (T1), UE's occlusion mode | **0.25–0.69 ms**, scaled from UE's published 0.5 ms at 1080p on a GTX 680 (RESEARCH §4.4); sun only, on-screen only, fades toward 90° (P5); sees every on-screen caster at any depth, mesh or SDF, because its mask is depth | trivial plugin | ✓ | ✓ | 1–2 passes, no RHI gap | **Not built inside `z_far`** (D1): there it is a sun-only, screen-bound subset of D. **Built only as the far-field mask, V13, owner Q4.** |
| B | Epipolar + 1D min/max (T2) | no published cost; one directional light | ✓ | ✓ | ✓ (M–L) | new 2D passes | **Not built**: single-light ceiling. Hillaire 2020 also rules it out for a non-homogeneous atmosphere (T23). |
| C | Per-pixel shadow-map march (T3, Bevy) | full resolution × 64 steps: **1.42–2.8 ms** per directional light; half resolution × 32 steps: **0.18–0.35 ms** per light (RESEARCH §4.4); shafts at shadow-map resolution; ∝ pixels (×2.25 at 4K) | ✓ (Bevy's marker is the pattern we adopt) | ✓ | ✓ | screen targets | **Not built** (D1): per light, ∝ pixels, blind to SDF casters, one march per transparent layer, and no better than D in the far field (it needs the same shadow-map coverage). Its sharpness advantage is recorded as the escape if V2's shafts are judged too soft (§4.6.4). |
| D | **Froxel fog, fixed grid** (T5–T8) | inject + integrate **0.39–0.95 ms** (Low) + apply 0.13–0.18 ms; every light, every transparent layer | plugin, Off = zero | ✓ (§4.3) | ✓ | 3D storage images (gap G1–G2) | **Chosen** |
| D′ | Froxel fog, pixel-tiled (8 px, UE Epic / Frostbite) | 1.55–3.80 ms at 1440p; 3.48–8.54 ms at 4K | same | ✓ | ✓ | same | Rejected as default (D2): cost ∝ output pixels (P6) |
| E | Analytic height fog + local volumes (T12, T13) | ≈ 0 when fused into `fog_apply`; no shadows | ✓ | ✓ | ✓ | leaves only | **Chosen as D's far-field tail**, not standalone |
| F | Hillaire 2020 sky LUTs (T23) | 0.14–0.20 ms; < 1 MB | plugin | ✓ | ✓ (re-implemented) | 1 more 3D storage image | **Chosen as rung V9** (owner value Q1: it changes the look) |
| G | 2.5D clouds (T16, T19) | **0.60–1.86 ms**; 8.5 MB noise | plugin | ✓ | ✓ | noise gen, quarter-res + reprojection | **Rung V11**, owner scope Q2 |
| H | Voxel clouds + SDF march acceleration (T18) | no published cost | — | — | ✓ | sparse voxels | Research only |
| I | Dense heterogeneous local volumes (T26) | ∝ covered pixels × steps × shadow steps | component per volume | ✓ | ✓ if dense grids; NanoVDB would be third-party (P13) | dispatch indirect exists | **Rung V12**, owner value Q3 |
| J | Particle density injection (T25) | Frostbite: 1k particles 0.03 ms PS4 | rides on particles | ✓ (gather, §4.12) | ✓ | a cell-bin list | **Rung V10** |
| K | Flipbook sprites (T28) | particle fill | particle feature | ✓ | tool offline, runtime ours | particle texture sampling | Particles campaign's P4, not here |
| L | **SDF-caster visibility per froxel, camera space** (E-SDFVOL fog half) | 0.01 ms (object at mid distance) – 0.19 ms (camera inside the SDF region) – 0.39 ms (worst) at half resolution, 16 edits (§3.3) | only on SDF legs | ✓ | ✓ | one extra pass | **Chosen as rung V7** (D9) |
| L′ | SDF-only light-space visibility map clipped to the edit footprint | 0.05–0.18 ms per rebuild (128²–256²), 0 while edits and sun are static; hard-edged | only on SDF legs | ✓ | ✓ | one pass + 2D image | **The named escape for V7** (D9) |
| M | Hardware `rayQuery` visibility per froxel (`ShadowSources::HWRT_VIS`, `render_path_config.rs:489-491`) | ∝ rays; the far range at Low is 144,000 rays; **unpriced**: no measured ray rate for the reference GPU in the tree or the fetched sources [U] | `feature = "hwrt"` only | ✓ | ✓ | none on hwrt builds | **Recorded** (§4.6.2): a candidate replacement for the far cascade and the CSM tap on hwrt builds, measured when armed |

### 2.2 SDF casters in the fog — the honest comparison

In this engine, **SDF casters are in no shadow map**. The surface shading shadows them with a per-pixel field march (`sdf_soft_shadow_ranged`, RESEARCH §0.4), while CSM and the atlas carry mesh casters (§0.3). A volumetric technique therefore has to answer "does this sample see the sun past an SDF object?" separately.

**The comparator in pass 1 was a straw man, and it is withdrawn.** It priced drawing the field into a whole 2048² cascade (≈ 11.7 ms) and concluded "≈ 36× cheaper, the hybrid differentiator". Nobody would build that. The fair classical-style comparator is **L′**: an SDF-only light-space visibility map clipped to the edit union's footprint.

| Route | Cost at 16 edits [E] | When paid | What it gives |
|---|---|---|---|
| **L** — march per half-resolution volume cell (§3.3) | 0.01 ms (object at mid distance), 0.19 ms (camera inside the SDF region), 0.39 ms worst | every frame (camera-relative) | the surface's own soft penumbra at every cell (the same leaf, the same `T_MAX`), 2-froxel resolution |
| **L′** — 128²–256² light-space map over the edit footprint, 24 steps × 16 edits × 25 FLOP per texel | 0.16–0.63 GFLOP → **0.05–0.18 ms** | when edits or the sun direction change; 0 otherwise | hard-edged visibility at full volume resolution |

**Neither route is 36× the other.** Both are below 0.2 ms at the gateway's 16-edit cap.

**L is kept for V7**, for three reasons:

- It reproduces the surface's soft-shadow function exactly: the same generated leaf, called at the cell with the same march bound (§3.3). Fog shafts and floor shadows then agree on their penumbra.
- It needs no light-space fit, no texel snapping and no rebuild trigger.
- Its cost follows the SDF's presence in the view: 0.01 ms when the SDF is a distant object.

**L′ is the named escape.** It is taken if V7's timestamp exceeds 0.15 ms per frame in a scene whose edits and sun are static, because L′ amortizes to zero there.

**What the field actually buys.** It is not a 36× shortcut. The field gives visibility at arbitrary points with no raster pass and no coverage limit. That is also why V7 has no C1-style coverage hole (§4.6): the half-resolution volume covers the whole fog range.

**Where both stop.** Both routes are O(edits). They are valid **only up to the gateway's cap of 16 edits** (`sdf_field.hlsli:48`). At the render plan's P0 regime of 256–4096 edits (`OPTIMIZATION-PLAN-RENDER.md:463`), L's worst case scales to ≈ 6 ms (256 edits) and ≈ 100 ms (4096) [E]. That regime needs P9's O(1) brick fetch, as E-SDFVOL already states (`:461`).

---

## 3. Cost model

All figures are estimates **[E]** unless tagged [M]. Per-froxel costs use RESEARCH §4.2's anchor range of **0.42–1.03 ns per froxel** on the RTX 3060 Laptop (bandwidth-scaled from PS4 and GTX 970; Frostbite's apply is subtracted, O8 of the review). The per-pixel apply uses §4.3's range. Nothing here was measured on the reference GPU except the CSM depth anchor; V2's perf gate is the first fog measurement.

### 3.1 GPU time per rung, by resolution **[E]**

| Pass | Driver | 1080p | 1440p | 4K |
|---|---|---|---|---|
| `fog_inject` + `fog_integrate`, Low 160×90×64 | 0.92 M froxels | 0.39–0.95 ms | 0.39–0.95 ms | 0.39–0.95 ms |
| same, High 160×90×128 | 1.84 M froxels | 0.77–1.90 ms | 0.77–1.90 ms | 0.77–1.90 ms |
| `fog_apply` (+ analytic tail) | pixels | 0.07–0.10 ms | 0.13–0.18 ms | 0.30–0.40 ms |
| `fog_cull` (V4), 16×9×32 = 4608 cells | cells × lights | < 0.02 ms | < 0.02 ms | < 0.02 ms |
| `fog_sun_far_depth` (D21), 1024² D32 | caster vertices in [s, z_far] | 0.02–0.07 ms | same | same |
| `fog_sdf_sunvis` (V7), Low | 115 K cells × marched fraction | 0.01 (mid) – 0.19 (inside) – 0.39 (worst) ms | same | same |
| Sky LUTs (V9) | fixed LUTs | 0.14–0.20 ms | 0.14–0.20 ms | 0.14–0.20 ms |
| 2.5D clouds (V11) | pixels | 0.34–1.05 ms | 0.60–1.86 ms | 1.34–4.19 ms |
| Far-field occlusion shafts (V13) | pixels | 0.14–0.39 ms | 0.25–0.69 ms | 0.56–1.55 ms |

**The far-cascade row is anchored on a tree measurement.** The whole CSM depth zone, `ZONE_GBUF_CSM_DEPTH`, measured **0.067 ms** (median, RTX 3060 Laptop, device-local) for a scene of one 160 k-vertex model (`docs/diagnostics/W2208.md:29`) **[M]**. One more depth pass over the same casters costs at most that. The cost scales with caster bytes × passes, not pixels, which is the note's own finding.

**Recommended default through V8 (Low grid, sun + ambient + a few volumetric lights, SDF legs, the default 30 m CSM).** The lower bound takes the per-froxel anchor's low end, the mid-distance SDF scenario and the far cascade's low end. The upper bound takes the anchor's high end, the SDF worst case and the whole measured CSM zone.

| Resolution | Total [E] |
|---|---|
| 1080p | ≈ 0.51–1.53 ms |
| 1440p | ≈ 0.57–1.61 ms |
| 4K | ≈ 0.74–1.83 ms |

**Resolution scaling is nearly flat.** Only `fog_apply` scales with pixels, which answers Frostbite's "4K … open challenge" (P6) the way AC4 and Doom Eternal did.

**How the anchors decompose.** The anchor range's upper end is Frostbite's, which includes 1.1 ms of local lights on PS4. Without them the anchor is 0.63 ns per froxel (RESEARCH §4.2), which puts a sun + ambient scene at Low at **≈ 0.58 ms** for inject + integrate [E]. The integrate part alone, scaled from Frostbite's 0.40 ms for 1.45 M froxels on PS4, is **≈ 0.13 ms** at Low (0.40 × 0.92/1.45 ÷ 1.91) [E].

### 3.2 VRAM **[E]**

| Resource | Format | Low | High |
|---|---|---|---|
| `fog_scatter[2]` (ping-pong = history) | RGBA16F | 2 × 7.37 MB | 2 × 14.7 MB |
| `fog_integrated` (single, seeded across frames — O1) | RGBA16F | 7.37 MB | 14.7 MB |
| `fog_sdf_sunvis` (V7, half-res per axis, single, seeded) | RGBA8 | 0.46 MB | 0.92 MB |
| `fog_sun_far` (D21, only when s < `z_far`) | D32, 1024² | 4.19 MB | 4.19 MB |
| fog light table + cull grid + index list (V4) | SSBO | ≈ 0.2 MB | ≈ 0.2 MB |
| `FogVolumeBuf` (V5), 32 × 80 B | SSBO | 2.5 KB | 2.5 KB |
| **Total through V8** | | **≈ 27.0 MB** | **≈ 49.6 MB** |

That is 0.45 % / 0.83 % of the reference GPU's 6 GB. The render plan's §2a VRAM budget (`OPTIMIZATION-PLAN-RENDER.md:853`) should carry ≈ 27 MB, not "~30 MB for ×2" (C3). Later rungs add:

- sky LUTs < 1 MB, of which the 32³ aerial-perspective volume is 0.26 MB;
- cloud noise 8.5 MB, plus quarter-res cloud targets ≈ 2 × 1.8 MB at 1440p in RGBA16F.

### 3.3 SDF-caster visibility (V7) — the bounded E-SDFVOL **[E]**

**The call.** Each cell of the half-resolution volume calls the generated leaf **unmodified**, from the cell centre `P` itself: `sdf_soft_shadow_ranged(P, 0, L_sun, T_MAX)`.

- `T_MAX = 10.0` is the directional caster's march bound that the surface resolve passes (`deferred_pbr.hlsl:508`, `:1178`). So a fog cell gets the same function a surface at `P` would get, minus the surface's normal lift (`SHADOW_NORMAL_BIAS`, `:515`), which a point in a medium has no normal for.
- Pass 1's claim that the march covers "only the ray's segment inside the bound" is withdrawn. The leaf starts at `SHADOW_MINT` from its origin (`sdf_shadow_leaves.hlsli:52`), and moving the origin to the bound's entry would change the `SHADOW_K·d/t` penumbra (`:56`).
- The part of the ray before the bound is cheap. The step is `max(d/FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP)` (`:60`), so a head-on approach keeps 1 − 1/√2 ≈ 29 % of the remaining distance per step, and 45 m closes to 0.1 m in ≈ 5 steps.
- **The constants the leaf spells symbolically** (`T_MAX`, `MAX_IT`, `SHADOW_K`, `SHADOW_MINT`, `SHADOW_MINT_STEP`, `SHADOW_HIT_EPS`) live in `deferred_pbr.hlsl:508-514`. The fog skeleton mirrors them, pinned by a value-identity test, the way that file already mirrors the marcher's.

**The prefilter.** The host computes one bound per frame from the ≤ 16 `SdfEdit` rows. It must follow each kind's actual layout, because the struct's own field comment ("radius / half-extents") does not describe the capsule:

| Kind | Stored as | Per-edit AABB |
|---|---|---|
| `SPHERE` | `center.xyz`, `params = (r, 0, 0)` (`boyko_sdf_math/src/lib.rs:150-153`) | `center ± r` on every axis |
| `BOX` | `center.xyz`, `params.xyz` = half-extents | `center ± params.xyz` |
| `CAPSULE` | `center.xyz` = endpoint a, `params.xyz` = endpoint b, `params.w` = radius (`sdf_field.hlsli:55`, `:84`, `:126-133`) | the AABB of a and b, inflated by `params.w` |

The per-frame bound `B` is the union over **all** edits, inflated by two terms:

- `Σ smoothness / 4`: the generated `smin` is `lerp(b, a, h) − k·h·(1 − h)` (`sdf_field.hlsli:151-162`), so each smooth union can lower the field by at most `k/4`.
- **`T_MAX / SHADOW_K = 1.25 m`: the penumbra reach.** This term is new in pass 2 (P29). The leaf darkens any sample with `SHADOW_K·d/t < 1`, so a ray that misses the tight AABB by less than `t/SHADOW_K` is still in penumbra. A prefilter without this term would set visibility 1 where the full march returns less.

**Soundness.** For every sample `q = P + L·t` with `t ≤ T_MAX` on a segment that misses `B`, the field is at least `dist(q, B_tight) − Σk/4`:

- the primitives are exact SDFs;
- `smax` and `max` never lower the value;
- the fold takes edit 0 as-is (`:210`).

That is `≥ T_MAX/SHADOW_K ≥ t/SHADOW_K`, so `res` stays 1 and no hit occurs. **A miss therefore returns exactly 1.** Subtractive and intersecting edits after edit 0 cannot add material, so excluding them from `B` is also sound. V7 takes the simpler all-edit union and records the tightening.

- **A miss** writes visibility 1 with zero field evaluations.
- **A hit** runs the call above.

Cost per marched cell ≈ steps × edits × FLOP per edit ≈ (24 average + ≤ 5 approach) × 16 × 25 = **9.6–11.6 kFLOP**, at 50 % of the 6.9 TFLOPS base-clock peak.

**The marched fraction depends on the scenario, so pass 1's single "typical 10 %" is replaced by three scenarios:**

| Scenario (Low, 115,200 cells) | Marched fraction | Low | High (230,400 cells) |
|---|---|---|---|
| **Mid:** one 8 m SDF object 20 m ahead, sun overhead | ≈ 2.5 %: its 20 m shadow prism spans ≈ 7 slices and ≈ 23 % of each slice's cells | **0.01 ms** | 0.02 ms |
| **Inside:** the camera within the edit region (the M2 fixture spans [-4, 4]³) | ≈ 57 %: with exp-Z over 0.5–64 m, 37 of 64 slices lie within 8 m (the review's derivation) | **0.19 ms** | 0.38 ms |
| **Worst:** every cell's segment crosses `B` | 100 % | 1.34 GFLOP → **0.39 ms** | 0.77 ms |
| Full-resolution alternative (921,600 cells, worst) | 100 % | 10.7 GFLOP → 3.1 ms | 6.2 ms |

**Validity: n ≤ 16 edits** (the gateway cap). The cost is linear in edits, and §2.2 gives the 256–4096 regime.

**Half resolution per axis is enough.** Fog is low-frequency, and AC4 deliberately **down**-sampled its sun shadow for fog to kill flicker (T5, P8). The P9 brick atlas would make each step O(1) instead of O(16 edits). It stays deferred behind P9's own threshold, because it covers [-4, 4]³ at level 0 today (RESEARCH §0.4).

### 3.4 Scaling with lights and volumes **[E]**

- **Unshadowed volumetric punctual light:** ≈ 30 FLOP per froxel it reaches. At the fog cull's cap-average occupancy of 32768 / 4608 = **7.1** indices per cell, 0.92 M × 7.1 × 30 ≈ 196 MFLOP → **≈ 0.06 ms**.
- **Shadowed light:** + 1 atlas comparison tap per froxel it reaches. UE reports shadowed lights at about **3×** unshadowed (T7, P7), so the budget is ≈ 0.17 ms at that occupancy with every light shadowed.
- **Local volumes (V5):** at most 32 visible, each costing a ≈ 5-op bounding-sphere early-out per froxel. 0.92 M × 32 × 5 = 147 M ops → **≈ 0.02–0.05 ms**. Binning volumes into the cull is deferred until a scene needs more than 32 (D15).

### 3.5 CPU **[E]**

| Work | When | Cost |
|---|---|---|
| `fog_pack_volumes`: ≤ 256 `FogVolume` rows, frustum test, cap 32, 80 B each into a `ScratchColumn` lane | per frame | < 5 µs |
| `fog_pack_lights`: a filtered copy of `VolumetricLight` rows into the fog light table (≤ 1024 rows; typically tens) | per frame | < 5 µs |
| `fog_advance_frame`: jitter phase, the edit-union bound over ≤ 16 edits, reprojection data, the pre-exposure pair, the far-cascade fit | per frame | < 2 µs |
| Sun transmittance (V9): the transmittance leaf's f32 instance, 40 steps | per frame | < 1 µs |
| `resolve_volumetrics_policy` | boot only | — |

There is no per-frame allocation. All lanes are `ScratchColumn`-backed (`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:44`), the `ParticleEmitScratch` precedent.

---

## 4. Architecture

### 4.1 ECS data model (Principle 0 — every durable datum is kernel storage)

```rust
// crates/boyko_render/src/volumetrics/*.rs — a boyko_render feature module, the particle_* layout.

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct VolumetricsConfig {            // the owner knob — the SsaoConfig pattern (ssao_config.rs:1-20)
    pub mode: FogMode,                    // #[default] Off | Froxel      — capability is the enum, no bool
    pub grid: FogGrid,                    // #[default] Low (160×90×64) | High (160×90×128)
    pub z_near: f32,                      // 0.5  — exp-Z slice 0 (D7)
    pub z_far: f32,                       // 64.0 — == MESH_DEPTH_T_MAX (gbuffer_mrt.fs.hlsl:113); ≤ 256 (D7, Q4)
    pub history: FogHistory,              // #[default] Blend { current_weight: 0.05 } | Off (goldens)
    pub tail_max_distance: f32,           // 1000.0 — the analytic tail's cap for VIEWT_BG pixels
}

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct GlobalMedium {                 // the world's height fog; density 0 == no global medium
    pub density: f32, pub base_height: f32, pub height_falloff: f32,
    pub albedo: [f32; 3], pub absorption: f32, pub phase_g: f32,
    pub emission: [f32; 3],
    pub noise_amplitude: f32, pub noise_frequency: f32, pub wind: [f32; 3],   // one octave (T5)
}

#[derive(Resource)] pub struct ResolvedVolumetrics { /* armed, dims, z range, fog ClusterParams,
    ddgi_variant, sdf_sun_variant, far_cascade_armed — written ONCE at boot by
    `resolve_volumetrics_policy` */ }

#[derive(Resource, Default)] pub struct FogStats {   // diagnostic counters, read on a cold path (D26)
    pub volumes_dropped: u32,                        // fog_pack_volumes: visible volumes past 32
    pub lights_dropped: u32,                         // fog_pack_lights: marked lights past MAX_LIGHTS
    pub index_list_demand: u32,                      // the fog cull's alloc word, read back cold
}

#[component] pub struct VolumetricLight;             // unit marker — a punctual light without it never enters fog
#[component] pub struct FogVolume {                  // a local medium; pose from the entity's transform
    pub shape: FogShape,                             // Box | Ellipsoid
    pub half_extents: [f32; 3],
    pub density: f32, pub albedo: [f32; 3], pub absorption: f32, pub phase_g: f32,
    pub emission: [f32; 3], pub edge_fade: f32, pub height_falloff: f32,
}
#[component(storage = "bitset")] pub struct FogVolumeActive;   // + Enabled<FogVolumeActive> (the EmitterActive precedent)

#[repr(C, align(16))] pub struct FogVolumeGpu {      // 80 B = 5 × float4, POD, the upload row
    pub world_to_local: [[f32; 4]; 3],               // 48 B — affine, scale folded into half-extents
    pub density_albedo: [f32; 4],                    // xyz = σs colour × density, w = σa
    pub emission_g: [f32; 4],                        // xyz = emission, w = phase g (edge_fade/falloff packed in a follow-up
}                                                    //   rung if a 6th lane is needed — V5 decides with its oracle)

#[repr(C, align(16))] pub struct FogFrameGpu {       // the fog's per-frame UBO, one per frame in flight
    pub jitter: [f32; 4],                            // xyz = 3D Halton offset, w = history weight (0 = reset / Off)
    pub sun_dir_pre: [f32; 4],                       // xyz = toward the sun, w = this frame's pre-exposure (D23)
    pub sun_color_ratio: [f32; 4],                   // rgb = the directional row's colour, w = pre_N / pre_{N-1}
    pub sky_color: [f32; 4], pub ground_color: [f32; 4],  // the SkyLight row's two colours (§4.7)
    pub prev_eye: [f32; 4], pub prev_forward: [f32; 4],   // the previous view-z, for reprojection (§4.8)
    pub prev_view_proj: [[f32; 4]; 4],               // the previous NDC xy: MotionCamState's matrix
    pub far_view_proj: [[f32; 4]; 4],                // the fog far cascade (D21); unread when not armed
    pub ranges: [f32; 4],                            // x = z_near, y = z_far, z = s (this frame's last split), w = tail cap
    pub edit_lo: [f32; 4], pub edit_hi: [f32; 4],    // V7's inflated bound B (§3.3)
}
```

**Capability is structural.**

- **A punctual light** is volumetric iff it carries `VolumetricLight`. This is the Bevy pattern (T3) and AC4's "lights marked by artists as affecting the atmosphere" (T5).
- **The sun and the sky** are the world's lighting context and always light an armed fog, which is AC4's split: "the sun, a constant ambient, and point lights marked by artists". A flash is a punctual light without the marker (K4).
- **An entity** is a medium iff it carries `FogVolume`. Toggling it is `Enabled<FogVolumeActive>`, a flag only on objects that carry the capability.
- **The world** has fog iff `VolumetricsConfig::mode != Off` at boot.

**Runtime off versus boot off.** Turning fog off at runtime means `GlobalMedium::density = 0` with no active volumes: the passes still run, and that costs time. Boot-Off costs nothing, with the same boot-frozen rule as the particle, TAA and SSAO arms.

**Systems** (`CoreSchedule::Main`, registered by `VolumetricsPlugin`, which has the `ParticlePlugin` shape: `crates/boyko_render/src/particle_plugin.rs:54-56`, and implements `Plugin` from `crates/boyko_ecs/src/ecs/core/app/plugin.rs:27`):

- `resolve_volumetrics_policy` — cold, boot. It reads the config, `DeviceCaps`, `ResolvedRenderPath` and the resolved CSM, and writes `ResolvedVolumetrics`. It arms the far cascade iff the CSM is armed and its shadow distance is below `z_far` (D21). It pushes a degrade, never a fail-fast, on a device whose `maxImageDimension3D` is below the grid (G2), and on a Forward/ForwardPlus path before RK-12 (§4.5).
- `fog_pack_lights` — per frame. It copies the rows of punctual lights carrying `VolumetricLight` into a **separate fog light table** with the same row layout and its own header carrying the fog `ClusterParams`. **The opaque light table, its packer and every shipped lighting shader are untouched (D3).**
- `fog_pack_volumes` — per frame: query, frustum cull, sort by distance, cap 32, write the `ScratchColumn`. The overflow is counted in `FogStats`, never logged in the frame loop (D26).
- `fog_advance_frame` — per frame, ordered after the CSM resolve and the light packer. It writes `FogFrameGpu`:
  - the 3D Halton phase (16-entry table, new; the existing `HALTON_8` is 2D, `taa_jitter.rs:53`);
  - the edit-union bound;
  - the sun and SkyLight colours, copied from the rows the opaque packer wrote this frame, so fog and surfaces cannot disagree;
  - the pre-exposure pair;
  - the previous eye and forward;
  - this frame's last split `s`, and the far-cascade matrix fitted over `[s, z_far]` and texel-snapped like a cascade;
  - history reset on request.

**Arming.** `RenderPathConsumers` (`render_path_config.rs:832`) gains `fog_on`. Following the rule that a consumer arms its producer (TRANSPARENCY §2.4), it arms:

- the path's `gViewT` producer: on VB, `vb_viewt`, which today runs only when SSAO arms it (`passes/vb.rs:3911-3919`);
- the `MotionCamState` carry (`motion_cam.rs:101`), which the history needs;
- `fog_sun_far_depth`, when D21 says so.

### 4.2 Images and formats

| Image | Dims | Format | Usage | Lifetime |
|---|---|---|---|---|
| `fog_scatter[fi]` | grid | RGBA16F: `rgb` = pre-exposed σs·L_in + emission, `a` = σt | STORAGE \| SAMPLED | cross-frame ping-pong, boot-cleared to `GENERAL` (the `taa_hist` discipline, `targets.rs:468-480`) |
| `fog_integrated` | grid | RGBA16F: `rgb` = in-scatter to the slice's far edge, `a` = transmittance | STORAGE \| SAMPLED | **single**, `add_image_seeded` (`framegraph/graph.rs:360`) |
| `fog_sdf_sunvis` (V7) | grid / 2 per axis | RGBA8 (`r` used) | STORAGE \| SAMPLED | single, seeded |
| `fog_sun_far` (D21) | 1024² | D32 | DEPTH_ATTACHMENT \| SAMPLED | single, seeded (the CSM and atlas precedent) |

**Why single images for everything but the history (O1).**

- `fog_integrated` and `fog_sdf_sunvis` are written and read inside one frame. The only reason to ring them per frame in flight would be GPU overlap between frames. On the single queue (G4), a seeded start-of-frame WAR barrier serializes that point anyway.
- The tree already shares the CSM, the shadow atlas and the DDGI atlas across frames this way (`graph.rs:353-360`, `ddgi.rs:155-156`).
- Saving: 7.37 MB (Low) / 14.7 MB (High), plus 0.46 / 0.92 MB.
- `fog_scatter` must stay a ping-pong, because the inject reads the history at a reprojected, trilinearly filtered coordinate while its neighbours write.

**Why these formats:**

- **RGBA16F** storage is core-mandatory (`targets.rs:2319-2323`).
- **RGBA8** is used as storage throughout the tree (`GBUFFER_FORMAT`, `targets.rs:2279`, and the "can never fault" note at `:2321`).
- **R8/R16F** storage needs a probe. The 3 wasted bytes per cell cost 0.35 MB at Low, cheaper than a probe and a fallback arm (D20).
- **Linear filtering of RGBA16F** is taken as mandatory, recalled but not re-read here **[U]**. V0's round-trip test samples trilinearly, so it will catch this if wrong.

**Why three volumes, not five.** Frostbite's chain has two V-buffers, scatter, history and integrated.

- Here, media and lighting are **fused** in one pass (AC4: "due to a bit smaller bandwidth usage"), so there is no V-buffer (D4).
- The scatter ping-pong **is** the history, and the integrated volume is single.
- Against Frostbite's five, that saves two volumes (14.7 MB at Low) and one write-plus-read of 14.7 MB per frame (≈ 0.09 ms at 336 GB/s) [E].
- The split returns only when V10 injects particle density, which needs a media volume written before lighting.

### 4.3 The passes

All passes are compute, except `fog_sun_far_depth`. Each is an `Option<PassId>` with its `ResId`s appended **last**, and nothing is declared when disarmed: the conditional tail of [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) D13 (`:553`).

**`fog_sun_far_depth` (D21)** is one more recording of the `csm_depth` depth-only pipeline, the one spot shadow faces already reuse (`passes/gbuffer.rs:2360`). It renders into `fog_sun_far` with `FogFrameGpu.far_view_proj`, over the CSM caster set culled to the far frustum.

**`fog_cull` (V4)** is **the shipped `cluster_cull` kernel, unchanged, dispatched a second time**.

- **Input:** the fog light table, whose header carries the fog `ClusterParams`: dims 16×9×32, `z_near`/`z_far` = the fog range, fog-owned caps with an index list of 32768.
- **Why it is sound:** the kernel reads no depth and is conservative (`cluster_cull.hlsl:436-456`), so it is path-independent. It already handles the ortho camera (`:110-114`).
- **Size:** 4608 cells, at most 255 per axis, which the packing requires (`light.rs:956-959`).
- **Overflow is observable with no kernel change.** The alloc word takes `InterlockedAdd` of the full per-cell demand before the cap check (`cluster_cull.hlsl:376`), so `alloc > cap` after the dispatch means the list overflowed. A cold readback of that word feeds `FogStats::index_list_demand` (D26).
- **The slice relationship.** Each fog slice lies inside one cull slice, because 64/32 and 128/32 are integers and both use `near·(far/near)^(k/n)`. The inject pass nevertheless looks the list up **by the sample's own view-z** (`cluster_z_slice`, `light_table.hlsli:376`), never by index division, so `pow` rounding cannot misassign a froxel.

**`fog_sdf_sunvis` (V7, SDF legs only).** Per half-resolution cell: the prefilter, then the generated `sdf_soft_shadow_ranged` over `field_distance`, exactly as §3.3 specifies. It includes `sdf_field.hlsli` and `sdf_shadow_leaves.hlsli` **unmodified**. It writes visibility and keeps no history of its own: the scatter history smooths it.

**`fog_inject`.** For each froxel `(x, y, k)`:

1. **Position.** `P` is the camera ray through the XY centre at view-z `z(k + jitter.z)`, offset in XY by `jitter.xy`. That is one 3D Halton offset per frame, the same for every froxel (Frostbite's "same offset along the view ray", T6, extended to XY like UE's sub-voxel jitter, T7). Under `camera_mode` ORTHO the ray is parallel, with its origin on the image plane and view-z equal to the ray parameter, the same convention as the cull's `view_z_to_t` (`cluster_cull.hlsl:110-114`) (D25).
2. **Medium.**
   - global: `density·exp(−(P.y − base_height)·falloff)·(1 + noise_amplitude·perlin3(P·freq + wind·time))`;
   - local: for each of ≤ 32 visible `FogVolumeGpu`, a bounding-sphere early-out, then the shape and edge fade in local space;
   - combined: σs and σa added; `g` as the σs-weighted mean; emission added.
3. **Light arriving at `P`.**
   - **Ambient:** isotropic.
     - V2: the hemisphere mean `0.5·(sky + ground)` from `FogFrameGpu`, which carries the SkyLight row every path's surfaces read (§4.7). This is exact for a two-hemisphere environment under an isotropic phase.
     - V6: a normal-free DDGI sample (§4.7).
   - **Sun:** `E_sun · vis_sun(P) · vis_sdf(P) · phase(cos θ)`.
     - `vis_sun` is a **new normal-free 1-tap lookup** in a new `fog_shadow.hlsli`, with the hardware 2×2 comparison of the `BILINEAR1` arm (`shadow_apply.hlsli:151-153`), a constant depth bias and no blend band. History supplies the variance, which is what that arm's documentation says the narrow kernels are for.
       - **Below `s`:** the cascade selected as `csm_visibility` selects it (`shadow_apply.hlsli:297`).
       - **At or past `s`:** the fog far cascade. It **never** returns "lit" merely because no surface cascade covers `P`, which is what `csm_visibility` does past its last split (`:281`, `:313`) (D21).
     - `vis_sdf` is the trilinear fetch of `fog_sdf_sunvis`, or 1 off SDF legs.
   - **Punctual lights (V4):** the fog cull list at `P`: attenuation × phase × (shadowed ? 1-tap atlas lookup : 1).
4. **Output.** `S = pre · (σs·L_in + emission)`, stored with σt. `pre` is this frame's pre-exposure, the scalar every surface producer multiplies by (D23).
5. **History (V3).**
   - Reproject `P` (unjittered) into the previous grid. XY comes from `prev_view_proj` (clip.xy / clip.w). The slice comes from the previous view-z `dot(P − prev_eye, prev_forward)`, never from NDC z: on Deferred the marcher-aligned projection has `row2 == row3` (`SHADER-VARIANT-MANIFEST.md:84`, `motion_cam.rs:58`), so NDC z is pinned at 1 (D25).
   - If the result lands outside [0,1]³, there is no history (Frostbite).
   - Otherwise `h = history.rgb · (pre_N / pre_{N−1})` (σt is not radiance and is not rescaled), and `out = lerp(h, out, 0.05)`.
6. **Write** `fog_scatter[fi]`.

The noise is procedural, a `perlin3` eDSL leaf: one octave, as in AC4 (T5). There is no noise texture until clouds (D14).

**`fog_integrate`.** One thread per column (160×90 = 14,400) marches front to back:

```
T = 1; acc = 0
for k in 0..dim_z:
    S, σt = scatter[k]
    D = slice thickness along the column's centre ray
    acc += T · (S − S·exp(−σt·D)) / σt      // Frostbite's integral (T6), σt→0 limit S·D handled in the leaf
    T *= exp(−σt·D)
    integrated[k] = (acc, T)                 // the accumulation to slice k's FAR edge
```

It is a serial column march, as AC4 and Frostbite shipped it. It is associative: `(S_a, T_a) ∘ (S_b, T_b) = (S_a + T_a·S_b, T_a·T_b)`. A group-scan variant is therefore a pure perf rung, taken only if V2's timestamp shows the pass above 0.1 ms (D13). The estimate is ≈ 0.13 ms at Low, so it is marginal and gets measured.

**`fog_apply`** is a full-resolution compute pass on the HDR `lit`.

- **Depth.** `t = gViewT`. `view_z = t · dot(rd, cam_forward)` under PERSP and `view_z = t` under ORTHO, the inverse of `cluster_cull.hlsl::view_z_to_t`, because `gViewT` is the ray parameter, not view-z (RESEARCH §0.11).
- **Inside the grid.** Texel k holds the accumulation to slice k's far edge. The apply therefore samples at `w = (u − 0.5) / dim_z`, where `u` is the continuous exp-Z coordinate of `view_z`, so texel k's centre sits exactly on that far edge. For `u < 1` it lerps from `(S, T) = (0, 1)` at the near plane to texel 0. Sampling at `u / dim_z` would add up to half a slice (3.8 % of the distance at Low) of in-scatter from behind the surface: the depth half of P25.
- **Beyond `z_far`:** the analytic tail `(S_t, T_t)`, integrated from `z_far` to `min(t, tail_max_distance)`, with `t = VIEWT_BG` (1.0e30, `viewt_from_depth.comp.hlsl:58`) treated as the cap.
  - The tail uses T12's closed form for optical depth.
  - Its in-scatter is ambient plus the **unshadowed** sun × phase, as UE's height fog does beyond volumetric range, multiplied by `pre`. V13, if built, masks its sun term (§4.6.3).
- **Output:** `L_out = S_g + T_g·(S_t + T_t·L_surface)`. `L_surface` is already pre-exposed by its producer.

### 4.4 Frame order (every path; brackets = rung-gated)

```
… light_upload? → csm? → atlas? → [ddgi update, where armed]
→ [D21] fog_sun_far_depth       casters in [s, z_far] → fog_sun_far                     (raster, depth-only)
→ [V4] fog_cull                 fog light table → fog cluster grid + index list        (compute)
→ [V7] fog_sdf_sunvis           edits + prefilter → sunvis volume                       (compute, SDF legs)
→ [V2] fog_inject               medium + lights (+ history, V3) → fog_scatter[fi]       (compute)
→ [V2] fog_integrate            fog_scatter[fi] → fog_integrated                        (compute)
… geometry and opaque lit producers (resolve | forward_opaque | vb_shade / vb_resolve) → [sdf_forward_march]
→ [V2] fog_apply                lit (HDR) · T + S ; analytic tail past z_far            (compute)
→ [V13] far_shaft_mask          depth → radial occlusion mask → tail sun term           (compute, owner Q4)
→ lit_prev / scene_color_copy   (REFLECTIONS R4a / TRANSPARENCY R4 — one copy, two consumers, AFTER fog: D17)
→ translucent_draw              per-fragment fog fetch (V8; REFRACT composes per §4.9)
→ particle_draw                 per-particle fog fetch in the VS (V8)
→ taa_resolve → tonemap (R4a) → post chain → present
```

**Why the compute passes need no scene geometry.** They sit after the shadow producers and before the geometry because they read no scene depth (T5: "possibility of computing this pass … as soon as shadow-maps are ready"). On the single queue (`device.rs:3925`) that buys nothing today. It keeps the passes eligible for async compute when P13 (`OPTIMIZATION-PLAN-RENDER.md:699`) lands, which is Wronski's stated use. The HZB early-out Wronski lists alongside it (T5) is **not** taken: the current frame's HZB does not exist yet at this point, and the deck itself says culling voids the 3D-history property (P25).

**V13's order is provisional.** It runs after `fog_apply` as written above, but its mask must multiply only the tail's sun term. The design either splits that term out of `fog_apply` or runs the mask before it; V13 decides with its oracle.

**Per-path placement against the shipped recorder orders:**

- **Forward** (`passes/forward.rs:62-63`): `… light_cull? → csm? → atlas? → forward_opaque → sdf_forward_march?`
- **VB** (`passes/vb.rs:941-942`): `light_upload? → csm? → atlas? → vb_sky → vb_raster → vb_resolve`

The fog passes insert after `atlas?` and before the first lit producer. `fog_apply` inserts after the last one.

### 4.5 Participation matrix

| | Deferred | Forward | ForwardPlus | VisibilityBuffer |
|---|---|---|---|---|
| `fog_cull` / `inject` / `integrate` | ✓ | ✓ | ✓ | ✓ (path-independent, no depth read) |
| Sun via CSM (view-z < `s`) | where `csm_on` | same | same | same |
| Sun via the fog far cascade (`s` ≤ view-z < `z_far`, D21) | where `csm_on` and `s < z_far` | same | same | same |
| Punctual shadows via the atlas | where `punctual_shadows_on` | same | same | same |
| Ambient (the SkyLight row, §4.7) | hemisphere; DDGI (V6) when `ddgi_on` | hemisphere only (`cap_forward_v1_consumers`, `render_path_config.rs:1379`) | hemisphere only | hemisphere; DDGI when `ddgi_on` |
| SDF casters (V7) | SDF / Both legs | same | same | same |
| `fog_apply` depth (`gViewT`) | ✓ (marcher / `viewt_from_depth`) | **after RK-12** | **after RK-12** | ✓ (`vb_viewt`, armed by `fog_on`) |
| Camera modes | PERSP and ORTHO (D25) | same | same | same |
| Screen TAA needed? | no — fog keeps its own history | no | no | no |

**Forward and ForwardPlus take fog when RK-12 lands.** RK-12 is the reflections campaign's `fwd_viewt`, the existing `viewt_from_depth_rz` kernel dispatched on Forward ([`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) §2.2). It is one producer serving two campaigns. Until then, `resolve_volumetrics_policy` pushes a degrade and fog is not declared on those paths.

### 4.6 Shadows into the volume

#### 4.6.1 Mesh casters: the sun, and the coverage policy (D21)

**The problem pass 1 missed (C1 of the review).** `csm_visibility` returns fully lit past the last active split (`shadow_apply.hlsli:281`, `:313`), and the in-tree defaults put that split at 30 m:

- `DEFAULT_SHADOW_DISTANCE = 30.0` (`csm_config.rs:157`);
- the default `CatchAll` keeps the last split at `shadow_distance` (`:222-223`);
- `Shrink` moves the fully-lit terminator into the scene and "jumps up to 29.3% per latch transition" (`:218`);
- every in-tree example that arms CSM uses `cascade_count: 3` with that default.

With the fog range at 0.5–64 m, exp-Z slices 54–63 cover 30–64 m, which is 34 m of the 63.5 m ray. A 1-tap that inherits the surface select would give those froxels full sun:

- shadow volumes of buildings and canopies past 30 m vanish;
- an interior deeper than 30 m glows;
- under `Shrink`, the terminator pops at each latch and the 5 % history smears the pop over ≈ 90 frames.

**The options, priced:**

| Option | Cost [E] | Effect | Verdict |
|---|---|---|---|
| Clamp `z_far` to `s` | 0 | the volume covers 0.5–30 m; the analytic tail past 30 m is unshadowed, so the glow and the lost shadow volumes **move into the tail** | Rejected: it relocates the defect rather than fixing it |
| Require or raise the shadow distance to ≥ `z_far` | 0 new passes; casters at 30–64 m now drawn into the CSM | the `CatchAll` last cascade stretches from `[far_eff, 30]` to `[far_eff, 64]`, so its texels grow ≈ 2.1× [E]; every CSM golden with receivers in that cascade moves, **because fog was armed** | Rejected as automatic. It remains the owner's own `CsmConfig` choice: with `shadow_distance ≥ z_far`, D21 arms nothing |
| Fade the sun in-scatter past `s` | 0 | sunlit outdoor haze past 30 m goes dark: wrong in the common case | Rejected |
| **A fog far cascade over `[s, z_far]`** | depth pass ≤ 0.067 ms [M anchor, §3.1]; 4.19 MB | covered to `z_far` with no surface byte moved; follows `s` per frame, so `Shrink` never leaves an uncovered band | **Chosen (D21)** |
| `rayQuery` past `s` (M, O3 of the review) | ∝ 144,000 rays at Low, unpriced [U] | exact, any distance | Recorded (§4.6.2): `hwrt` builds only |

**The far cascade, concretely.**

- **Fit.** `fog_advance_frame` fits the far cascade over the view-z slice `[s, z_far]` with the cascade fit, texel-snapped. `s` is this frame's last active split, so under `Shrink` it is that frame's `far_eff`.
- **Resolution.** For a 60° vertical field of view [E: the tree's camera fov was not read], the slice's bounding sphere is dominated by the far plane's corners. Its radius is ≈ 75 m, so a 1024² map has ≈ 0.15 m texels.
- **Against the froxels.** A froxel is `z · 0.0128` m tall at that fov, so the far cascade's texel is no coarser than a froxel from view-z ≈ 12 m on. Under `Shrink` with `far_eff < 12 m`, fog shadows between `far_eff` and 12 m are softer than one froxel. That is stated, not hidden.
- **Arming.** It is armed iff the CSM is armed and `s < z_far`; with CSM off, the sun in fog is unshadowed everywhere, matching the surfaces. It is not a layer of the CSM array: the array is created with `MAX_CASCADES = 4` layers (`texture.rs:120-122`), and a scene with four cascades has no free layer, so one code path uses a fog-owned image.

**Alternative for a free layer (not taken):** ≤ 3-cascade scenes could reuse the CSM array's spare layer 3 at no VRAM. That would give two code paths for 4.19 MB, and it is not taken.

#### 4.6.2 HWRT visibility (O3 of the review, scored)

`ShadowSources::HWRT_VIS` ships behind `feature = "hwrt"` (`render_path_config.rs:489-491`). A per-froxel `rayQuery` toward the sun is exact at any distance, sees every BLAS instance, and needs no coverage policy at all.

- **Cost:** the far range at Low is 10 slices × 14,400 = 144,000 rays per frame, or 18,000 at half resolution per axis.
- **Not taken for V2**, for two reasons:
  - The default build has no `hwrt`, so the far cascade must exist anyway.
  - Neither the tree nor the fetched sources give a ray rate for the reference GPU, so the option cannot be priced [U].
- **Recorded as a candidate** replacement for both the far cascade and the CSM tap on `hwrt` builds. A V2 perf gate on an `hwrt` build measures it when the owner arms that feature.

#### 4.6.3 The far field past `z_far` (W4 of the review)

Past `z_far` the tail's sun in-scatter is unshadowed, so a sun behind a skyline, a ridge or trees beyond 64 m casts no shafts. The candidates:

| Mechanism | Cost @1440p [E] | Covers | Verdict |
|---|---|---|---|
| **None** | 0 | nothing past `z_far` | **Default** |
| **T1 in UE's occlusion mode (V13):** a depth-derived occlusion mask blurred radially from the sun, multiplying the tail's sun term (and V9's aerial-perspective sun term) | 0.25–0.69 ms, scaled from UE's published 0.5 ms at 1080p on a GTX 680 (RESEARCH §4.4) | every on-screen caster at any depth, mesh or SDF; sun only; fades toward 90° | **The mechanism if the owner wants it: Q4** |
| Raise `z_far` to 128 or 256 m (the far cascade follows) | 0 volume cost (a fixed grid); far-cascade raster ∝ casters out to `z_far` | off-screen casters too, inside the volume and shadowed | **Available as config.** It costs near-field depth resolution: slices within 8 m at Low go 36.6 → 32.0 → 28.4 for `z_far` = 64 → 128 → 256 m. On Deferred it helps sky pixels only past 64 m (P22). High (128 slices) buys the resolution back at 2× the per-froxel cost |
| Shadowed aerial perspective (V9's 32³ AP volume) | — | its slices are 1 km thick (32 over 32 km, T23): no shafts at 100 m scale | Rejected |
| Per-pixel marched atmosphere with jitter + reprojection (Hillaire's recommendation for mountain shadows, T23) | T3-class: 0.18–0.35 ms at half resolution per light | off-screen casters within the shadow map's coverage | Recorded; bounded by the same coverage as the far cascade, so it adds nothing over raising `z_far` here |

#### 4.6.4 The shaft-sharpness ceiling (stated, W4)

| Limit | Value | Consequence |
|---|---|---|
| Lateral (XY) | one froxel = 12 × 12 px at 1080p, **16 × 16 px at 1440p**, 24 × 24 px at 4K | a gap narrower than a froxel keeps roughly its width fraction of the shaft contrast: a 4 px gap at 1440p keeps ≈ 25 % [E] |
| SDF shafts (V7) | 2 froxels = 32 px at 1440p | softer than mesh shafts; matches the soft SDF surface penumbra |
| Depth (Z) | 7.6 % of the distance per slice at Low, 3.8 % at High | a 0.76 m slice at 10 m |
| Far-cascade texels | ≈ 0.15 m at the default range | below the froxel size from 12 m on |

**Escapes if V2's shafts are judged too soft:** a 320×180 grid (4× the per-froxel cost: 1.55–3.80 ms [E]), or T3 at half resolution for the sun only (+0.18–0.35 ms). Neither is taken by default.

#### 4.6.5 Punctual lights and SDF casters

**Mesh casters, punctual lights.** A 1-tap atlas lookup (a sibling of `atlas_pcf_disc`'s centre tap, `shadow_apply.hlsli:226`), only for marked lights that carry an atlas slot.

**SDF casters, the sun (V7, §3.3).**

- **Punctual SDF casters are not marched.** The surface resolve's per-light SDF shadow is hand-placed in its light loop. Per froxel, that would be O(lights × steps × edits), E-SDFVOL's original problem.
- **Recorded as out of scope.** When the P9 atlas scales, a second visibility volume per shadowed SDF light is the route.

#### 4.6.6 Where the functions live, and their oracle (W3 of the review)

**The functions stay out of `shadow_apply.hlsli`.** They live in a new `fog_shadow.hlsli`, included only by fog shaders, which declares no binding of its own for the shipped maps and reuses the shipped include's descriptor names. No shipped `.spv` moves (D3).

**They are oracled against an independent value before any shaft golden is blessed.** The tree's own documentation of the CSM Y convention disagrees with its code:

- the header comment says the lookup applies `uv.y = 1 - (clip.y/clip.w * 0.5 + 0.5)` (`shadow_apply.hlsli:114-115`);
- the body deliberately does not (`:260`), and warns that a second flip mirrors every shadow and is invisible only for casters on the cascade's light-up = 0 line.

A fog lookup written from the header comment would be mirrored and would bless itself into a golden. The probe fixture that prevents this is V2 gate (g) (§7).

### 4.7 Ambient in the medium

- **Hemisphere (V2).** Every path's surfaces read the same SkyLight row: `sky_color = L.color; ground_color = L.pos`.
  - Deferred: `deferred_pbr.hlsl:1236-1237`.
  - Forward: `forward_opaque.fs.hlsl:301-302`.
  - VB: `vb_shade.comp.hlsl:520-521`.

  `fog_advance_frame` copies that row into `FogFrameGpu`, so the fog on every path uses the surfaces' ambient. Under an isotropic phase the in-scattered ambient is the mean of the two colours. Without an ambient term, shadowed fog goes black (P16, AC4 p. 52).
- **DDGI (V6).**
  - **The new function.** `ddgi_probe_sample_iso`, a hand-written sibling of `ddgi_probe_sample` (`ddgi_resolve.hlsli:107`), with the same trilinear and Chebyshev weights but **no wrap term** (P24).
  - **The two directions.** It fetches irradiance for ±Y and averages. For uniform radiance, `E(d) + E(−d) = 2πL` holds for any `d`, so the estimator is exact there, and exact for any environment that is constant per hemisphere about Y.
  - **Its bias** is horizon-concentrated light. Recorded; six directions is the escape at 3× fetches.
  - **Normalization** (whether probes store E or E/π) is fixed by the V6 oracle, not assumed.
- **The variant.** `FOG_DDGI` is a `-D` on the fog's own inject shader, because it adds the DDGI atlas bindings (§5).

### 4.8 Temporal

- **Blend.** Current weight 0.05 (Frostbite, T6). Jitter comes from a 16-phase 3D Halton sequence.
- **What the history converges to.** It converges to an exponentially weighted mean, not a uniform one:
  - At steady state over the 16 phases, phase weights are ∝ 0.95ᵏ, so the newest phase weighs 1/0.95¹⁵ = 2.16× the oldest.
  - From a reset, frame 0 still carries 0.95¹⁵ ≈ 0.46 of the weight at frame 15.
  - V3's oracle is therefore the EMA recursion itself (§7).
- **The fp16 floor (P27).** A lerp step `0.05·|c − h|` below half an fp16 ULP rounds back to `h`. The history therefore stops up to 0.5 ULP / 0.05 = **10 ULP** from its target, ≈ 0.5–1 % relative. That is the stated bias bound. A decay toward 0 never stalls, because the ULP shrinks with the value, so the P1 trail bound is unaffected.
- **Rejection.** History is rejected outside the previous grid only. Behind moving occluders history stays valid in 3D (T5 p. 58, T15).
- **Pre-exposure.** The history is rescaled by `pre_N / pre_{N−1}` before the blend (D23). Without it, a 4× exposure step leaves the fog at the old scale: 0.95¹⁴ ≈ 49 % stale after 14 frames and ≈ 10 % after 45.
- **Trail bound (P1).** A light switched off decays below 1 % of its in-scatter within ⌈ln 0.01 / ln 0.95⌉ = **90 frames**. That is pinned by a V3 gate, not hidden.
- **History Off.** It exists so a deterministic byte golden is possible (P14). With history Off the jitter phase is pinned to 0.
- **Resets.** On boot and on an explicit request bit in `FogFrameGpu.jitter.w`, which a camera cut sets.

### 4.9 Transparents and particles

**The composite rule.** A premultiplied fragment `(c, α)` at depth `d` becomes `c·T(d) + S(d)·α`. The background behind it is already fogged by `fog_apply`, so for α → 0 the result is the fogged background, and nothing is double-counted. This is AC4's "any number of transparent layers" (T5) and Doom Eternal's transparents reading the scatter data (T8, `TRANSPARENCY-RESEARCH.md:91`).

**One gating rule for every consumer draw shader (D24).** Fog reaches the translucent FS and the particle VS through an **always-bound binding with a boot-resolved uniform gate**, never a `-D` axis.

- When fog is unarmed the binding holds a 1×1×1 placeholder volume (T = 1, S = 0). That is one boot allocation of 8 B of texels, and only when such a consumer is armed. The gate word skips the fetch, so the output is bit-identical to a shader without the term.
- This is the tree's rule already:
  - TRANSPARENCY D18: `FROXEL`/`TEXTURED` are not axes because their bindings are always bound;
  - an absent input binds a valid placeholder (`targets.rs:204-208`);
  - `csm_pcf_disc` selects by a wave-uniform word, not a variant.
- **Axes vs gating, priced.** The axis alternative is +4 particle VS and +8 translucent FS `.spv`. The runtime gate costs one wave-uniform branch per vertex or fragment when fog is off.
- **Fog's own passes** use axes only where their interface differs (`FOG_DDGI`, `FOG_SDF_SUN`, `LIT_FORMAT`, §5).

**Translucent meshes.** The transparency design's `forward_translucent.fs` fetches per fragment. **No new axis**, so TRANSPARENCY D18's variant bound stands.

**Refractive draws compose differently (O2 of the review, confirmed).** TRANSPARENCY R5 makes a transmissive fragment **replace** the background it sampled, with `a = base_color.a`, which is 1.0 for pure transmission. The sampled background `B` comes from the scene-colour copy, which runs after `fog_apply` (D17), so it already carries the fog of `[0, d]`. The generic rule would apply that segment twice.

The physically correct radiance at the camera, for glass of own radiance `L_s` and transmittance `τ`:

```
S(0,d) + T(0,d)·[L_s + τ·L_behind]   with   B = S(0,d) + T(0,d)·L_behind
                                      ⇒   rgb = T(d)·L_s + τ·B + (1 − τ)·S(d)
```

The `-D REFRACT` variant keeps its transmitted and own terms separate before the fog composite and uses that formula. It is exact when the refraction offset is zero, and it uses pixel `x`'s fog for the offset sample `x′` otherwise, which is acceptable because fog is low-frequency. REFRACT is already an axis, so there is no new variant. V8 pins it.

**Particles.** One fetch per vertex at the particle centre in the VS, through the generator `emit_particles.rs` (PARTICLES D12). The result folds into the `nointerpolation` vertex colour, an fp32 interpolant, so the FS is untouched. The FS computes `color × tex` (`particle_draw.fs.hlsl`).

- **Additive particles** take `color.rgb·T`, because in-scatter belongs to what is behind them. This is exact.
- **Alpha particles** take `color.rgb·T + S`. The FS then yields `(c·T + S)·tex.rgb·α`, against the exact `c·tex.rgb·α·T + S·α`.
  - **The error is `S·α·(tex.rgb − 1)`.** It is zero for white or greyscale-mask smoke sprites and darkens in-scatter in proportion on dark-textured sprites.
  - **This is VFX-DESIGN-SPACE's recommended VS-only fold** (`VFX-DESIGN-SPACE.md:653-662`), taken here with the approximation stated.
  - **The exact escape** is an FS interpolant (FS 24 → 48), taken only if a V8 golden shows the error.
- **Rejected: evaluating (T, S) per particle in `particle_sim`** (the PARTICLES D11 site). The render record's colour lane is RGBA8 (`ParticleRender::color_rgba8`, `particle.rs`), so S would clip at 1.0 and T and S would quantize to 1/255, the F2 trade-off class. It would also force `particle_sim` after `fog_integrate`.
- **Cost:** 100 k particles × 4 vertices = 400 k trilinear fetches, **< 0.02 ms** [E].

**The scene-colour copy runs after `fog_apply` (D17).** Refraction sees a fogged background and composes per the formula above. SSR next frame reflects fogged radiance, an approximation: the fog along the camera path, not along the reflection path. REFLECTIONS' "one copy, two consumers" is kept rather than paying a second full-screen copy of 8–16 MB at 1080p. This is a known deviation, recorded in §9.

### 4.10 Sky and atmosphere (V9 — owner value Q1)

The design is Hillaire 2020's four LUTs (T23), re-implemented in-house from the paper. The MIT reference is read, not linked.

| LUT | Size | Format | Size in memory |
|---|---|---|---|
| Transmittance | 256×64 | RGBA16F | — |
| Multi-scatter | 32² | RGBA16F | — |
| Sky-view | 200×100 | RGBA16F | — |
| Aerial perspective | 32³ over 32 km | RGBA16F | 0.26 MB |

The aerial-perspective volume is a second 3D storage image; it is kept separate from the fog volume because the ranges differ by three orders of magnitude.

**Where each LUT is used:**

- **Sky pixels.** The three analytic sky sites are `forward_sky.fs.hlsl`, the `H_bg` branch of `deferred_pbr.hlsl` (`:1497`) and `vb_sky` (`passes/vb.rs:1555`). They become three splice points of **one** include function that samples the sky-view LUT plus a transmittance-attenuated sun disc. This is the "one site, N splice points" rule REFLECTIONS §2.1 set.
- **Sun colour.** The transmittance leaf's **f32 instance runs on the host** each frame and writes the directional row's colour: a low sun reddens every surface and the fog with no GPU readback. This is the eDSL dual instantiation used as a feature, not only as an oracle.
- **Aerial perspective.** It replaces the analytic tail's in-scatter colour past `z_far`. **Sky pixels are excluded** (P11, T24).
- **Fog ambient.** The sky-view LUT, averaged by one tiny reduction, replaces the hemisphere colours as the isotropic sky term.
- **Pre-exposure.** Physical sun radiance would overflow fp16 in the fog volumes (P23). Pre-exposure is applied at injection **from V2 on** (D23), so V9 introduces no new scalar.
- **Amortization.** Doom Eternal amortizes the atmosphere LUT over 32 frames (T8). Taken only if V9's timestamp exceeds 0.2 ms.

### 4.11 Clouds (V11 — owner scope Q2)

The design is a 2.5D layer, HZD/UE class (T16, T19):

- **Noise.** Generated in-house once at boot by a compute pass: 128³ RGBA8 Perlin-Worley, 32³ Worley detail and 128² curl, 8.5 MB. This is the render plan's "own noise gen" (`OPTIMIZATION-PLAN-RENDER.md:507`).
- **Tracing.** 64 steps, 6 cone light samples, quarter-resolution trace with 1-of-16 update per frame and reprojection (HZD's 10× lever).
- **Shadows.** A cascaded "Beer shadow map" (UE), sampled next to CSM by `fog_inject` and by surface lighting.
- **Composite.** Before `fog_apply`, behind all geometry.
- **Cost.** **0.60–1.86 ms at 1440p** [E]. The march is dominated by 3D-noise fetches, so the conservative bandwidth ratio sets the upper end, by the one scaling method (RESEARCH §4.1, §4.4). Pass 1 scaled by FLOPS only, which is the optimistic end.
- **Voxel clouds with SDF-accelerated marching (Nubis³, T18)** stay research. They are the one place the SDF machinery could accelerate clouds, but there is no published cost to argue from.

### 4.12 Local volumes — smoke, fire, explosions

The sibling effects survey written the same day, [`VFX-RESEARCH.md`](VFX-RESEARCH.md), owns flipbooks, rain and explosion authoring. Its §6 table places "particle density injected into froxel fog" as the tier for soft volumes that receive light and shadow. That tier is V10 below; this document owns only the volumetric half.

**V10: particle density injection** (dust, smoke trails, fire glow).

- **Binning.** Particles are binned into the fog cull's 4608 cells by a gather pass after `particle_sim`. The bins are count-capped per cell (64 [E]) and ordered by a prefix sum, the particle radix-sort precedent. A per-frame drop counter in the bin header, accumulated like `ParticleCounters::clamped_spawns` (`particle.rs:444`), is read cold into `FogStats` (D26).
- **Injection.** `fog_inject` loops each cell's bin, adding σs, σa and emission (fire).
- **Why gather, not scatter.** A scatter with float atomics needs `VK_EXT_shader_atomic_float`, which is not loaded, and would make the medium's summation order vary per frame. Gather is deterministic and extension-free (D16).
- **The split.** This rung **splits** the fused inject into media and lighting (D4's stated exit), because injected density must exist before lighting reads it.
- **Resolution.** Bounded by the froxel size (T25). Fine for dust and smoke, not for a crisp fireball.

**V12: dense local volumes** (hero explosions, thick smoke columns, owner value Q3).

- **The component.** `DenseVolume` carries a handle to a baked density + temperature 3D texture: R8G8 64³ = 512 KB each, baked in-house from a simulation or procedurally. It is a dense grid because NanoVDB's static-topology format is third-party (P13) and its sparsity pays only at film resolution.
- **The march.** Half resolution inside the volume's screen bounds, over tiles by `cmd_dispatch_indirect` (loaded and used today: `passes/particles.rs:514`), with in-volume shadow rays to the sun only.
- **Composite.** Before transparents, depth-tested against `gViewT`.
- **Fog continuity.** The density is also injected at froxel resolution into the fog, so shafts and fog see it.
- **Empty-space skipping.** A coarse min/max mip. **A distance field of the density isosurface** (Nubis³'s trick, T18) is the SDF-native variant.

**Flipbook explosions** (EmberGen-style, T28) are a particle texture feature, the particles campaign's P4. They are not part of this campaign.

**Rain** belongs to the effects/particles research. Its volumetric interface here is small:

- rain particles read the fog like any particle (V8);
- rain haze is a `GlobalMedium` density authored by gameplay.

### 4.13 What the SDF field makes cheap — and what it does not

**Cheap:**

- **Shafts from SDF casters without a shadow map** (§2.2, §3.3). They carry the surface's own soft penumbra, cost 0.01–0.19 ms in the scenarios of §3.3, and have no coverage limit inside the fog range. Pass 1's "≈ 36× cheaper than drawing the field into one cascade" is withdrawn: the fair light-space comparator L′ costs the same order.
- **Density hugging surfaces (E-DENS-B).** `extinction = remap(field_distance(p))` over the frozen probe. It is a V12-class research leaf, and no shipped game does it (RESEARCH §2.6).
- **Empty-space skipping** for local volume marches.
- **Particle collision.** Already shipped (`particle_sim_sdf`), so injected smoke respects SDF geometry for free.

**Not cheap, or not available:**

- **Mesh geometry has no field here,** so mesh casters still need CSM, the far cascade and the atlas.
- **The analytic gateway** is ≤ 16 edits (`sdf_field.hlsli:48`), and the march is bounded by `T_MAX = 10.0`, `MAX_IT = 128u` (`deferred_pbr.hlsl:508-509`). An SDF shaft is therefore at most 10 m long, exactly like the SDF shadow on a surface.
- **The brick atlas** is a 40³ near-field SAMPLED image. It is not a scene-scale density store, which is why E-DENS-A is deferred (C6).
- **Per-froxel SDF shadows for punctual lights** remain O(lights × steps × edits) (§4.6.5).

---

## 5. The shader-eDSL surface and the variant manifest

**Leaves** (new `crates/boyko_shaderdsl/src/volume.rs`). Each is generic over `C: Cf`. **"Oracle"** means the `f32` instance is a host oracle, because the body has no texture fetch (`ResRef` is emit-only, `emit/mod.rs:546-552`).

| Leaf | Body | Oracle | Rung |
|---|---|---|---|
| `phase_hg(cos, g)` | Henyey-Greenstein | ✓ (normalization test) | V1 |
| `phase_hg_draine(cos, g, α)` | Jendersie–d'Eon blend (T14) | ✓ | later (P9 fix) |
| `fog_slice_view_z(k, n, near, far)` / `fog_view_z_to_u(z, …)` | exp-Z and its continuous inverse | ✓ (round-trip) | V1 |
| `fog_apply_w(u, n)` | the half-texel far-edge mapping and the slice-0 ramp weight (§4.3) | ✓ (vs the per-slice accumulation) | V1 |
| `froxel_integrate_step(acc, T, S, σt, D)` | Frostbite's integral (T6), including the σt→0 limit | ✓ (vs closed form) | V1 |
| `height_fog_optical_depth(ro_y, rd_y, t0, t1, a, b)` | T12's closed form, including the `rd_y → 0` branch | ✓ (vs quadrature) | V1 |
| `perlin3(p)` | one-octave gradient noise (T5) | ✓ | V1 |
| `fog_volume_density(p_local, shape, fade)` | box / ellipsoid + edge fade | ✓ | V5 |
| `reproject_to_prev_grid(P, prev_view_proj, prev_eye, prev_forward, …)` | history coordinate: NDC xy + previous view-z (never NDC z) | ✓ | V3 |
| `sdf_edit_aabb(kind, center, params)` | the per-kind bound of §3.3 | ✓ (vs dense sampling of the primitive) | V7 |
| `ray_aabb_segment(p, L, t0, t1, lo, hi)` | the SDF prefilter over `[SHADOW_MINT, T_MAX]` | ✓ | V7 |
| `atmo_*` (transmittance / sky-view / aerial-perspective parameterizations and density profiles) | Hillaire's mappings (T23) | ✓, and the f32 instance **is** the host sun-colour evaluator (§4.10) | V9 |

**Skeletons** hold the loops, `RWTexture3D` stores, light-list walks and history fetch. They are owned by a new generator, `crates/boyko_shaderdsl/src/bin/emit_volumetrics.rs`, as templates with eDSL holes. This is PARTICLES D12's rule, because the eDSL has no stores, atomics or `groupshared` (F13, `PARTICLES-PLAN.md:547`). The texture-shaped helpers are hand-written: `fog_shadow.hlsli`'s 1-tap lookups and `ddgi_probe_sample_iso`. They are oracled by the device probe fixtures named in §7: V2 (g) for the sun, V4 for the atlas, V6 for DDGI. **No shipped include is edited.**

**Variant rows for [`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md).** Only axes that change the interface are listed, per the manifest's own rule. Placeholders and runtime uniforms cover everything else: CSM via `gCsmActive == 0`, the atlas, local volumes with count 0, and fog in consumer draw shaders (D24).

| Source | Axis | Values | `.spv` | Interface delta |
|---|---|---|---|---|
| `fog_inject.comp.hlsl` | `FOG_DDGI` | 0, 1 | 2 → | +DDGI irradiance/depth atlases + samplers |
| | `FOG_SDF_SUN` | 0, 1 | ×2 = **4** | +`fog_sdf_sunvis` `Texture3D` + sampler |
| `fog_sdf_sunvis.comp.hlsl` | — | — | 1 | edit-list `Buf` (the gateway's) + `RWTexture3D` |
| `fog_integrate.comp.hlsl` | — | — | 1 | two 3D images |
| `fog_apply.comp.hlsl` | `LIT_FORMAT` (REFLECTIONS R4a's axis) | b10g11r11, rgba16f | 2 | the `lit` image format |
| `fog_sun_far_depth` | — | — | **0 new** (the shipped `csm_depth` pipeline, recorded once more) | none |
| `cluster_cull.hlsl` | — | — | **0 new** (same `.spv`, second dispatch) | none (D3) |
| `particle_draw.vs.hlsl` (V8) | — (**no axis**, D24) | — | count unchanged (VFX's 4); **re-emitted once** at V8 | +`fog_integrated` + sampler + a gate word, always bound |
| `forward_translucent.fs.hlsl` (V8) | — (no axis, D24) | — | count unchanged (TRANSPARENCY D18's 8) | same, always bound; `REFRACT` composes per §4.9 |
| V9 `atmo_lut_*.comp.hlsl` | — | — | 4 | LUT images |
| V11 `cloud_*.comp.hlsl` | — | — | 4 | noise, trace, reconstruct, shadow |
| V13 `far_shaft_mask.comp.hlsl` | — | — | 1–2 | depth in, mask out |

That is **8 new `.spv` through V8**, plus one re-emit of `particle_draw.vs`. Pass 1 said 10: it counted a `PARTICLE_FOG` axis "+2 on top of `DEPTH_LINEAR`" that would in fact have been 4 → 8 on top of VFX's `PARTICLE_FX` (`VFX-DESIGN-SPACE.md:640`). The test-only kernels (`volume_roundtrip.comp.hlsl`, `fog_shadow_probe.comp.hlsl`) are not manifest rows.

**A cross-document note.** VFX-DESIGN-SPACE §6.3 carries this rung's `PARTICLE_FOG` as "VS 4 → 8". Under D24 its number is **4**, and its FS bound stays 24.

---

## 6. RHI gaps (checked in the code at `6394bc5e`)

| # | Gap | State today | Closed by |
|---|---|---|---|
| G1 | **First 3D storage image and first `RWTexture3D`** | `ImageUsage::STORAGE` exists (`crates/boyko_rhi/src/enums.rs:457`); 3D images get a 3D view (`texture.rs:263`); `StorageImage` binds by view (`crates/boyko_rhi/src/device.rs:361`). **No image is ever 3D+STORAGE and no shader writes one.** | V0: the create path accepts D3 + STORAGE \| SAMPLED; a round-trip device test |
| G2 | `maxImageDimension3D` unrecorded | only `max_image_dimension_2d` (`device.rs:392`), read from the limits blob. `vkGetPhysicalDeviceImageFormatProperties` is not loaded. | V0: `DeviceCaps::max_image_dimension_3d` from the same blob; a boot **degrade**, never a fail-fast. The grid is ≤ 255 per axis anyway (the Vulkan minimum is recalled as 256, **[U]**) |
| G3 | Cross-frame 3D history | 2D precedent `taa_hist` (`targets.rs:468-480`), boot-cleared `UNDEFINED → GENERAL` | V2/V3: the same discipline for `fog_scatter[2]`; the single images use `add_image_seeded` |
| G4 | No async compute | one family, `queue_count: 1` (`device.rs:2814`, `:3925`) | Not closed: fog is serial. P13 later (§4.4) |
| G5 | Dispatch indirect | loaded (`device.rs:599`), used (`passes/particles.rs:514`) | nothing to close (V12 uses it) |
| G6 | `vkCmdDrawIndexedIndirectCount` | deliberately not loaded (`device.rs:669-672`) | not needed by any rung |
| G7 | Subgroup ops | BASIC/BALLOT checked at boot (`device.rs:3034` region) | not needed (the integrate scan, if ever, uses `groupshared`) |
| G8 | GPU timing | `present/gpu_zone.rs` | every V-rung's perf gate |
| G9 | Frame-graph declaration | `add_image` / `add_image_seeded` (`framegraph/graph.rs:334`, `:360`); conditional-tail precedent (PARTICLES D13) | V2 |
| G10 | A depth-only pass into a fog-owned D32 target | the `csm_depth` pipeline already renders into non-CSM targets (spot faces, `passes/gbuffer.rs:2360`) | nothing new; D21 records it once more |

---

## 7. The ladder

Every gate is **red-first**: the test lands before the code it pins and is shown red, by absence or by a named mutation, before it is accepted. Goldens are byte goldens only where history is Off (P14). Perf gates are GPU timestamps on the RTX 3060 Laptop at 1080p / 1440p / 4K, run **with the owner's consent in a quiet window**: three build lanes share the machine. **Every perf gate is set at the conservative end of its estimate** (RESEARCH §4.1), so a correct implementation cannot fail it on the estimate's spread.

**Tolerances are derived, not chosen.** Pixel-level oracles run with the `LIT_FORMAT = rgba16f` arm forced, because B10G11R11's 6-bit mantissa (5 bits on blue) cannot resolve 1e-3. Probe quads sit at exact slice far edges and probe pixels at column centres, so every filter weight is exactly 0 or 1 whatever the device's `subTexelPrecisionBits`. What remains is at most four fp16 roundings on the path (scatter, integrated, `lit`, each ≤ 2⁻¹¹ relative) plus the σt rounding's optical-depth term τ·2⁻¹¹ with τ ≤ 2 in the fixtures, which is **≤ 2e-3 relative**.

### V0 — 3D storage images (RHI)

- **Adds:**
  - D3 + STORAGE \| SAMPLED creation;
  - `DeviceCaps::max_image_dimension_3d`;
  - a test-only `volume_roundtrip.comp.hlsl` that writes `hash(x, y, z)` into a 160×90×64 RGBA16F volume and samples it back trilinearly at texel centres.
- **Gate:**
  - All 921,600 texels equal the host hash; shown red under the mutation "write at `z + 1`".
  - Validation-clean.
  - A forced `max_image_dimension_3d = 64` boot disarms fog with a degrade entry and boots.
- **Cost:** zero at runtime.

### V1 — eDSL leaves and oracles

- **Adds:** the V1 rows of §5, plus the generator scaffold.
- **Gate:**
  - HG integrates to 1 ± 1e-4 at g ∈ {−0.9, 0, 0.5, 0.9}.
  - The integral step equals the closed form for a constant medium, including σt = 0.
  - Height-fog optical depth matches a 10⁴-step quadrature, including |rd_y| < 1e-5.
  - Exp-Z round-trips every integer slice; `fog_apply_w` returns the exact far-edge accumulation at every integer `u` and the zero-extent ramp at `u = 0`.
  - Emit byte-identity (the `*_edsl_sync` shape).
- **Cost:** none.

### V2 — The fog core: global medium, sun, hemisphere ambient, no history

- **Prerequisites:**
  - V0, V1;
  - **R4a / R11 HDR `lit`**;
  - the path's `gViewT` (Deferred; VB via `fog_on` arming `vb_viewt`; Forward/ForwardPlus after RK-12).
- **Adds:**
  - `VolumetricsPlugin`, config, medium, resolved and stats carriers, `FogFrameGpu`;
  - `fog_inject` (sun + ambient + noise, pre-exposed), `fog_integrate`, `fog_apply` with the tail;
  - the three volumes;
  - `fog_shadow.hlsli`;
  - the fog far cascade (D21).
- **Gates:**
  - **(a) 0 %-gate.** Plugin absent, and plugin present with `Off`: every existing golden is byte-identical, every shipped `.spv` is byte-identical, and the frame-graph census (ResIds, passes, pipelines) is unchanged. Shown red by a mutation that declares one fog image when Off.
  - **(b) Oracle fixture.** Homogeneous medium, noise 0, sun off, history Off. HDR `lit` is read back at 16 probe pixels over quads at known distances (on slice far edges) and must equal the f32 oracle's `L·T + S` within 2e-3 relative (derivation above). It runs under PERSP and ORTHO cameras (D25), and is red under the mutation "sample at `u / dim_z`" (the half-slice overshoot, P25).
  - **(c) Shaft golden.** A mesh occluder under the sun with CSM, history Off, jitter phase 0: a byte golden per Deferred and VB. **It is blessed only after (g) is green**, and it carries a semantic assertion. The host computes, for 8 probe pixels, the length of the view ray's segment inside the occluder's shadow volume within the fog range. Pixels with a zero-length segment must match the unshadowed fixture within (b)'s tolerance, and pixels with longer segments must be darker, monotonically.
  - **(d) Perf.** Low: inject + integrate ≤ 0.95 ms and apply ≤ 0.18 ms at 1440p, the §3.1 conservative bounds; far-cascade depth ≤ 0.07 ms in the W2208 scene. The result is recorded against the estimate.
  - **(e) Exposure consistency (W1).** With history Off, rendering at `exposure = 0.5` and `1.0` gives HDR `lit` probe values in the ratio 0.5 within 2 fp16 ULP, surfaces and fog alike. Red under the mutation "fog not exposed", which makes the fog term 2× too bright relative to the surfaces.
  - **(f) Far coverage (C1).** Default `CsmConfig` (3 cascades, 30 m, `CatchAll`), `z_far = 64`, a box caster at 45 m under the sun, history Off:
    - froxels in its shadow volume at view-z 40–60 m read sun visibility 0 in the probe fixture;
    - the HDR `lit` at probe pixels whose view ray lies in that shadow volume equals the ambient-only oracle within 2e-3;
    - red under the mutation "past the last split return 1", which is `csm_visibility`'s behaviour at `shadow_apply.hlsli:313`.

    A host test also forces a `Shrink` latch sequence and asserts that the far cascade's near edge equals that frame's last split in every frame, so no band is ever uncovered.
  - **(g) Visibility probe fixture (W3).** A test-only `fog_shadow_probe.comp.hlsl` includes `fog_shadow.hlsli` and `shadow_apply.hlsli`. It evaluates at 64 host-chosen world points: the fog's `vis_sun(P)`, and the shipped `csm_visibility(P, L, view_z, 1)` where a surface cascade covers `P`.
    - **The scene:** one world-fixed box caster off the cascade's light-up = 0 and light-right = 0 lines, where a mirrored lookup would be invisible (`shadow_apply.hlsli:255-259`), and the sun at 60° elevation.
    - **The points:** 16 inside the caster's shadow volume in each covering cascade and in the far cascade, each ≥ 0.5 m from any surface and ≥ 3 texels from the shadow edge; their mirror images across the cascade's light-space axes; and points beside the caster.
    - **The oracle:** a host ray-box test along L, independent of any shadow map, gives the expected 0 or 1. The GPU must equal it exactly, and must equal `csm_visibility` where both apply.
    - **Red-first mutations:** the `1 − (…)` Y-flip the header comment still describes (`:114-115`), an X-mirror, cascade index + 1, and a bias sign flip.
- **Cost:** §3.1.

### V3 — Temporal: jitter and reprojection

- **Gates:**
  - **EMA-recursion oracle (W2).** Static camera, noise amplitude 0.5, sun and CSM on.
    - **Reference:** 16 history-Off renders at phases 0..15 in the order the history-On run uses them; each frame's `fog_scatter` (7.37 MB) is read back as `c_0..c_15`. On the host, `h_0 = c_0` (a reset frame has no history) and `h_n = fp16(h_{n−1} + 0.05·(c_n − h_{n−1}))` in f32 with an fp16 store per step, the shader's arithmetic.
    - **Run:** history On for 16 frames from reset; read back the scatter at frame 15.
    - **Assert:** per texel, `|GPU − host| ≤ 20` fp16 ULP at the host value's magnitude. Derivation: the GPU and host lerps may round to adjacent fp16 values when the f32 value lies within an f32 ULP of a rounding boundary, a per-step difference of ≤ 1 ULP, contracted by 0.95 per later step, and Σ 0.95ᵏ ≤ 20.
    - **Non-vacuity:** the test itself asserts `max |c_15 − h_15| ≥ 200` ULP over the texels, so the gate is red with history disabled, where the GPU returns `c_15`.
    - **Not the uniform mean.** The oracle is the recursion because the EMA's steady state weights phases ∝ 0.95ᵏ, and from a reset the first frame still weighs ≈ 0.46 at frame 15 (§4.8).
  - **Pre-exposure invariance (W1).** The same run with a ×4 pre-exposure step at frame 8. The host recursion applies `h ← 4h` at the step and uses history-Off references rendered at the new pre-exposure; the GPU must match within the same bound. Red under "no history rescale", which leaves the result ≈ 71 % low at the step.
  - **Reprojection.** The leaf's oracle maps a world point through prev/cur to itself, under PERSP (the marcher-aligned matrix with `row2 == row3`) and ORTHO. Red under "slice from NDC z".
  - **Trail bound.** A light switched off decays below 1 % within 90 frames (§4.8), pinned.
  - **Robustness.** A moving camera produces no NaN, and out-of-grid history is rejected.

### V4 — Volumetric punctual lights

- **Adds:** `VolumetricLight`, the fog light table, `fog_cull` (second dispatch of the unchanged kernel), and 1-tap atlas shadows.
- **Gates:**
  - **Structural.** A light without the marker leaves fog output byte-identical to the scene without it. Red under the mutation "pack all lights".
  - **Cull.** The fog cull lists contain the exact per-froxel light set of a host oracle (the shape of `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs`).
  - **Opaque untouched.** The opaque cull's `.spv` is unchanged, byte-checked.
  - **Atlas probe.** The V2 (g) fixture extended to a spot light over an off-centre caster: the atlas 1-tap equals the host ray-box oracle, with the same mutation set.
  - **Overflow counter.** A scene exceeding the fog index cap reads `FogStats::index_list_demand > 32768` on the cold readback. Red under "counter not read".
  - **Perf.** Scaling with 0 / 16 / 64 / 256 marked lights, half of them shadowed.

### V5 — `FogVolume` entities

- **Gates:**
  - Density is exactly 0 outside the shape (leaf oracle).
  - Edge fade is monotone.
  - Packing 256 rows takes ≤ 5 µs (criterion; not run with `--test-threads`, which breaks criterion targets).
  - The 33rd visible volume is dropped, deterministically by distance, and counted in `FogStats::volumes_dropped` (D26).

### V6 — DDGI isotropic ambient (`FOG_DDGI`)

- **Gates:**
  - A uniform-radiance probe fixture gives the analytic value (the host DDGI mirror is the oracle).
  - The per-hemisphere fixture is exact.
  - Forward/ForwardPlus select the hemisphere term (participation test).

### V7 — SDF casters in fog (E-SDFVOL, fog half)

- **Gates:**
  - **Frozen field.** `sdf_field.hlsli` and the generated `sdf_soft_shadow_ranged` are unchanged: their existing sync tests stay green and the new pass includes them verbatim. The mirrored march constants equal `deferred_pbr.hlsl:508-514` by value.
  - **Prefilter soundness.** Over sampled cells, a prefilter miss implies the full march returns exactly 1 (host oracle, the leaf's f32 instance). The fixture must contain a sphere, a box and a capsule, at least one smooth union, and rays that pass within `T_MAX/SHADOW_K` of the tight bound. Red under each of three mutations:
    - "bound from `center ± params.xyz` for every kind", which gives a sphere a zero-thickness box;
    - "no penumbra inflation";
    - "no smoothness inflation".
  - **Shaft golden.** An SDF sphere between sun and camera casts a shaft, on Deferred and VB SDF legs, with history Off, blessed after the soundness gate is green and with (c)'s semantic assertion.
  - **Perf.** The three §3.3 scenarios at edits {1, 4, 16}, each within its conservative bound. If the static-SDF scenario exceeds 0.15 ms, L′ is taken (D9).

### V8 — Transparents and particles read the volume

- **Gates:**
  - An α = 1 non-refractive translucent quad at `d` receives the same fog as `fog_apply` at `d` (±2e-3).
  - **Refraction (O2).** A clear glass quad (τ = 1, no own radiance, roughness 0) at `d` in front of a fogged background gives the no-glass pixel ±2e-3. Red under the generic rule `c·T + S·α`, which fogs `[0, d]` twice.
  - Additive particles receive `c·T` exactly, and alpha particles receive `(c·T + S)·tex` against the host oracle of the stated fold.
  - With fog Off, particle and translucent pixel goldens are byte-identical (the gate word skips the fetch). `particle_draw.vs` is re-emitted once at V8, pinned by its generator sync test, and its `.spv` count is unchanged.

### V9 — Sky and atmosphere (owner Q1)

- **Gates:**
  - Each LUT texel equals its leaf's f32 instance within 1e-3 relative.
  - The host sun colour equals a GPU transmittance-LUT readback at the sun direction.
  - The sky goldens are re-blessed **with owner approval**, through transition pins: the atmosphere Off keeps the old sky byte-identical.
  - Perf ≤ 0.20 ms, the conservative end of 0.14–0.20.

### V10 — Particle density injection (split media/lighting)

- **Gates:**
  - The binned gather equals a host oracle sum per cell.
  - With the bin cap exceeded, the drop is deterministic and counted in the bin header's drop word, read cold into `FogStats` (D26).
  - Perf at 10 k and 100 k particles.

### V11 — 2.5D clouds (owner Q2)

- **Gates:**
  - Noise textures equal their f32 leaf instances.
  - Quarter-res reprojection holds a static-sky convergence bar.
  - Beer shadow map vs. ray-marched shadow within tolerance.
  - Perf ≤ 1.9 ms at 1440p, the conservative end of 0.60–1.86 ms; the measured value is recorded against both ends.

### V12 — Dense local volumes and field-derived density (owner Q3; research)

- **Gates:**
  - Per-volume march vs. a host oracle on a 16³ fixture.
  - Density injection into fog equals the dense march integrated over each froxel (± the froxel-resolution bound).
  - E-DENS-B's probe stays frozen, with the remap only on the scalar.

### V13 — Far-field occlusion shafts (owner Q4)

- **Adds:** `far_shaft_mask.comp.hlsl`: a half-resolution depth-derived mask (sky and view-t past `z_far` = unoccluded) blurred radially toward the sun's screen position, fading toward 90° (UE's behaviour, T1). It multiplies only the tail's sun term, and V9's aerial-perspective sun term when present.
- **Gates:**
  - With the sun behind a skyline caster past `z_far`, tail in-scatter is reduced along the caster's screen-space shadow wedge, against a host radial-blur oracle.
  - The volume's own in-scatter (≤ `z_far`) is byte-identical with and without V13 (the mask never double-shadows).
  - Perf ≤ 0.69 ms at 1440p, the conservative end.

---

## 8. Gates in one table

| Gate | Rung | What it proves | Red-first by |
|---|---|---|---|
| 3D round-trip | V0 | the first 3D storage write and read | `z+1` mutation |
| Dimension degrade | V0 | a small device boots without fog | forced cap |
| Leaf oracles | V1, V5, V7, V9 | math = closed form / quadrature | test before body |
| 0 %-gate census | V2 | Off costs zero; no shipped `.spv` moves | declare-one mutation |
| Homogeneous oracle (PERSP + ORTHO) | V2 | apply composes `L·T + S` exactly at the pixel's depth | test before pass; `u/dim_z` mutation |
| Exposure consistency | V2 | fog and surfaces share one exposure scale | "fog not exposed" |
| Far coverage | V2 | no froxel is lit merely for lack of a cascade | "past the last split return 1" |
| Visibility probe | V2, V4 | the fog's CSM, far-cascade and atlas taps agree with an independent host oracle | Y-flip, X-mirror, cascade + 1, bias sign |
| Shaft goldens | V2, V7 | mesh and SDF shafts, blessed after the probe, with a semantic assertion | probe first; new golden |
| EMA-recursion oracle | V3 | the history is the recursion it claims to be | history disabled (non-vacuity asserted) |
| Pre-exposure invariance | V3 | an exposure change does not leave stale fog | "no history rescale" |
| Trail bound | V3 | P1 is bounded, not hidden | α mutation |
| Marker structural | V4 | capability is structural | pack-all mutation |
| Overflow counters | V4, V5, V10 | overflow is observable without a frame-loop log | "counter not read" |
| Prefilter soundness | V7 | a miss really means visibility 1, for every primitive kind and the penumbra | three bound mutations |
| Frozen field | V7 | FIELD-CONSUMER discipline | existing sync tests |
| Translucent + refraction consistency | V8 | one fog for all layers; no double fog through glass | generic-rule mutation |
| Sky transition pins | V9 | the re-bless is checked, not trusted | atmosphere-off pin |
| Far-field mask | V13 | shafts past `z_far` without touching the volume's own in-scatter | mask on the volume term |

---

## 9. Risks

| # | Risk | Mitigation |
|---|---|---|
| K1 | R4a (HDR) does not land, which blocks V2 | Fog cannot compose physically on LDR (P15). The coupling is stated, and the only alternative, pre-tonemap splices in 8 producers + 3 sky sites, is rejected in D6. |
| K2 | Forward/ForwardPlus wait on RK-12 | Degrade entry. RK-12 is small: an existing kernel dispatched. |
| K3 | 1-tap CSM shimmer survives the history (P8) | The ESM downsample fallback (§4.6), measured in V3. AC4 downsampled its cascades into an R32F 1024×256 ESM with a box blur because full-resolution cascades flickered in fog (T5); that is a pass plus a 1 MB target, estimated at < 0.05 ms. |
| K4 | Trails from flashing lights (P1) | Bounded (V3). A flash (muzzle, explosion light) is authored as a light **without** `VolumetricLight`, so it never enters the fog. UE's documented workaround is a volumetric scattering intensity of 0 on that light ([`VFX-RESEARCH.md`](VFX-RESEARCH.md) §6); here it is structural and free. A per-light "no history" bit is the later escape for flashes that must scatter. |
| K5 | SDF worst case (every cell under the edit bound) | 0.39 ms at Low, bounded, for ≤ 16 edits. High doubles it. Above 16 edits the route needs P9 (§2.2). |
| K6 | fp16 overflow once the sky is physical (P23) | Pre-exposure at injection from V2 (D23). |
| K7 | SSR reflects fogged radiance (D17) | Accepted approximation. A second copy (8–16 MB, ≈ 0.05 ms) is the escape if it is visible. |
| K8 | The fog range tops at 64 m while the scene grows | `z_far` is config up to 256 m, at the stated near-field slice cost (§4.6.3). Past it the analytic tail, aerial perspective and, if the owner wants it, V13 carry the far field (P4, P22). |
| K9 | The fog index list overflows with many marked lights | Fog-owned caps (32768). Overflow is visible in `FogStats` through the unchanged kernel's alloc word (D26). |
| K10 | 3D storage validation findings on first use | V0 exists to find them before any pass depends on them. |
| K11 | The estimates are wrong by the anchors' spread (0.42–1.03 ns per froxel, 2.5×) | V2's perf gate records the first real number. The default preset (Low) is chosen so even the upper end fits, and every gate sits at the conservative end. |
| K12 | The far cascade's near edge coarsens under `Shrink` (texel > froxel below ≈ 12 m) | Stated (§4.6.1). The history smooths it; HWRT (§4.6.2) or a 2048² far map is the escape if a golden shows it. |
| K13 | The fp16 EMA floor leaves a ≤ 10 ULP bias (≈ 0.5–1 %) (P27) | Stated. Dithered rounding before the store is the escape if it is ever visible. |
| K14 | Froxel-scale leaking across thin screen-space edges (P25) | The depth half is removed by the far-edge mapping (§4.3). The XY half is accepted at V2. The escape is a depth-aware XY weight from the HZB tile range, ≈ +0.01–0.03 ms at 1440p [E]. |
| K15 | The alpha-particle fold darkens in-scatter on dark sprites | Stated error `S·α·(tex.rgb − 1)`. The FS interpolant (FS 24 → 48) is the escape. |

---

## 10. Decisions

### 10.A PERF / ARCHITECTURE — taken here, with the numbers

| # | Decision | Reason |
|---|---|---|
| D1 | Inside `z_far` the froxel volume is the only shaft mechanism; T2 and T3 are not built. Past `z_far`, the mechanism is T1 in occlusion mode (V13), built only on the owner's Q4. | Inside the range T1 (0.25–0.69 ms) is a sun-only, screen-bound subset of what V2 gives at 0.39–0.95 ms. T3 is 1.42–2.8 ms per light at full resolution, or 0.18–0.35 ms at half resolution × 32 steps, ∝ pixels, blind to SDF casters, and bounded by the same shadow-map coverage. Past the range T1 is the only candidate that sees distant on-screen casters at a cost independent of their count (§4.6.3). |
| D2 | A fixed grid: **Low 160×90×64 default**, High ×128. | The cost is flat in output resolution: 0.39–0.95 ms at 1080p, 1440p and 4K, against 3.48–8.54 ms for 8-px tiles at 4K. Low is AC4's and Doom Eternal's shipped choice, and the owner prioritizes throughput. |
| D3 | The fog light list comes from **a second dispatch of the unchanged cluster-cull kernel** over a **separate fog light table**. | < 0.02 ms, ≈ 0.2 MB. It touches no shipped shader, no opaque light-table byte and no VB pin. Sharing the opaque grid is impossible (C1). Rows of a new kind would touch every lighting loop; REFLECTIONS' probe-row choice fits there only because probes are opaque consumers. |
| D4 | Media and lighting are fused in `fog_inject` until V10. | −2 volumes (14.7 MB) and −0.09 ms of traffic [E]; AC4 shipped it fused. V10 splits it. |
| D5 | Three RGBA16F volumes: the scatter ping-pong (= history) and one **single, seeded** integrated volume. | 22.1 MB at Low. Ringing the integrated volume buys no overlap on the single queue (G4); the CSM, atlas and DDGI atlas already cross frames this way. It saves 7.37 MB (Low) / 14.7 MB (High). |
| D6 | Fog composes in **one** `fog_apply` pass on the HDR `lit`, and requires R4a / R11. | ≈ 0.13–0.18 ms at 1440p [E], against 8 + 3 splices, 8 transition pins and variants doubling, work that R4a moves out of the producers anyway. |
| D7 | Range 0.5–64 m by default, analytic tail beyond; `z_far` configurable to 256 m. | 64 m = `MESH_DEPTH_T_MAX` (P22). With 64 slices, the last slice is 4.7 m thick. A 0.1 m near plane would spend slices on centimetres. Raising `z_far` costs near-field slices (36.6 → 32.0 → 28.4 within 8 m, §4.6.3), not GPU time. |
| D8 | The sun reaches fog through a normal-free 1-tap lookup + history: in the selected surface cascade below `s`, in the fog far cascade from `s` to `z_far`. The ESM downsample is the fallback. | 1 tap per froxel, against a pass + 1 MB. The tree's `BILINEAR1` arm is documented for temporal pairing. |
| D9 | SDF casters: a camera-space half-resolution visibility volume (L), per-kind + penumbra-inflated prefilter, the frozen analytic gateway called from the cell centre with `T_MAX`. Valid for ≤ 16 edits. L′ (the light-space map) is the escape. | 0.01 / 0.19 / 0.39 ms for the mid, inside and worst scenarios [E]. L′ costs 0.05–0.18 ms per rebuild, which is the same order, so L's surface-consistent penumbra and zero trigger logic decide. L′ is taken if the static-SDF scenario measures > 0.15 ms. 256–4096 edits need P9 (≈ 6–100 ms analytic worst). |
| D10 | Ambient: the SkyLight row's hemisphere mean always, copied from the surfaces' row; DDGI isotropic where armed. | P16. Exact for per-hemisphere environments, and identical to what every path's surfaces read. |
| D11 | HG first; HG+Draine as a later leaf. | P9. Equal cost per T14, deferred only to keep V1 small. |
| D12 | History: α = 0.05, 16-phase 3D Halton, same offset for every froxel; history Off for goldens. | Frostbite's shipped setting; P1 and P14. Its convergence target is the EMA, gated as such (D22). |
| D13 | Serial column integrate; a scan only if > 0.1 ms measured. | ≈ 0.13 ms estimated; marginal, so measure first. |
| D14 | Procedural one-octave noise; no noise texture before clouds. | AC4's measured-sufficient choice; saves a generation pass and a texture. |
| D15 | Local volumes: CPU cull, cap 32 visible, loop in inject. | ≈ 0.02–0.05 ms. UE's `r.LocalFogVolume.TileMaxInstanceCount` defaults to 32 "per view (and per tile for consistency)" (T13): a global cap, as here. Binning waits for a scene that needs more. |
| D16 | Particle density: gather via cell bins. | Deterministic, no `VK_EXT_shader_atomic_float`. |
| D17 | The scene-colour / `lit_prev` copy stays single and runs **after** `fog_apply`. `REFRACT` composes as `T(d)·L_s + τ·B + (1 − τ)·S(d)`. | Refraction is correct with that formula; the generic rule would double-fog `[0, d]` (§4.9). The SSR approximation is recorded (K7). A second copy costs 8–16 MB. |
| D18 | Translucents fetch per fragment; particles fetch per vertex at the centre and fold into the vertex colour (alpha: the stated `tex.rgb` approximation). | < 0.02 ms for 100 k particles; FS bounds unchanged (24 particle, 8 translucent). |
| D19 | The sun colour is computed on the host from the transmittance leaf's f32 instance. | < 1 µs, no readback; every consumer of the light table sees it. |
| D20 | RGBA16F volumes, RGBA8 SDF-visibility volume. | Both core-storage formats in the tree's own usage; R8/R16F would need a probe and a fallback arm to save 0.35 MB. |
| D21 | **Sun coverage:** no froxel is lit for lack of a cascade. When the CSM is armed and `s < z_far`, a fog-owned 1024² far cascade covers `[s, z_far]`, fitted per frame. | ≤ 0.067 ms [M anchor] + 4.19 MB, no surface byte moved. Clamping `z_far` relocates the defect, raising the shadow distance moves every surface golden in the last cascade, and fading is wrong for lit haze (§4.6.1). |
| D22 | History-On correctness is gated at the scatter level by the EMA-recursion oracle with a derived 20-ULP bound; image goldens are history-Off byte goldens. | The fp16/EMA arithmetic fixes the tolerance (§4.8), so no statistical image bar and no owner tolerance call is needed (the former Q4, withdrawn: O4 of the review). |
| D23 | **Pre-exposure from V2:** `fog_inject` and the tail multiply by the frame's pre-exposure, and the history is rescaled by `pre_N / pre_{N−1}`. | Surfaces already multiply by `H.exposure` (`deferred_pbr.hlsl:1479`, `:1481`; `forward_opaque.fs.hlsl:436`), a live knob (`light.rs:562-564`) that POSTFX §5.2 makes the per-frame pre-exposure. Without it, fog is 2× too bright at exposure 0.5 and stale for 45+ frames after a change. It also settles P23 from day one. |
| D24 | **One gating rule:** consumer draw shaders take fog through an always-bound binding with a boot-resolved gate, never a `-D` axis. | +0 `.spv` against +12 for axes; the cost when off is one wave-uniform branch. It is the tree's rule (TRANSPARENCY D18, the placeholder pattern, `csm_pcf_disc`'s selector). |
| D25 | ORTHO cameras are supported, with the cull's own convention; reprojection takes the previous slice from view-z, never NDC z. | The cull already handles ORTHO (`cluster_cull.hlsl:110-114`). On Deferred the marcher projection pins NDC z (`SHADER-VARIANT-MANIFEST.md:84`). A degrade path would need per-frame detection and costs more than the branch. |
| D26 | Overflows are counters, never frame-loop log lines: `FogStats` on the host, the fog cull's alloc word and V10's bin drop word on the GPU, read on a cold path. | The `ParticleCounters::clamped_spawns` precedent (`particle.rs:444`). The unchanged cull kernel already accumulates the full demand (`cluster_cull.hlsl:376`). |

### 10.B VALUES / SCOPE — to the owner

1. **Q1 — Physical sky.** Should the Hillaire atmosphere (V9) replace the two-colour gradient and sun disc? It changes the look of every outdoor frame: sky colour, a low sun reddening all surfaces, distant haze. It re-blesses every golden that shows sky, through transition pins. Costs 0.14–0.20 ms and < 1 MB [E].
2. **Q2 — Clouds scope.**
   - none;
   - a 2.5D sky layer (V11: HZD/UE class, **0.60–1.86 ms at 1440p**, 8.5 MB [E]; pass 1's 0.6–0.95 ms was the optimistic end only);
   - fly-through volumetric clouds (Nubis³ class, research, no published cost).
3. **Q3 — The look of explosions and thick smoke.**
   - froxel-resolution smoke and fire glow only (V10: soft, cheap);
   - ray-marched dense volumes for hero effects (V12: crisp, costs ∝ screen coverage);
   - flipbook sprites (a particles-campaign feature).
4. **Q4 — Sun rays from distant geometry (skyline, ridge, trees past 64 m).** Inside 64 m, shafts come from the volume by default, including off-screen casters.
   - **none** (the default: the tail past `z_far` is unshadowed);
   - **screen-space occlusion shafts** (V13: 0.25–0.69 ms at 1440p [E]; sun only, on-screen or near it, fading toward 90°);
   - **a longer fog range** (`z_far` 128–256 m: no volume cost; the far cascade draws more distant casters; fewer depth slices near the camera, 36.6 → 28.4 within 8 m at 256 m).

   Former Q4 (the temporal golden tolerance) is withdrawn: it is decided by arithmetic in D22.

---

## 11. Unverified / open

- Every cost in §3 is an estimate from RESEARCH §4's anchors, except the CSM depth anchor [M]. The first fog measurement is V2's perf gate.
- The Vulkan-required minimum of `maxImageDimension3D` (recalled 256) and the mandatory linear filtering of RGBA16F were not re-read in the specification. V0's degrade and round-trip tests cover both at runtime.
- The DDGI probe normalization (E or E/π) is fixed by V6's oracle, not by reading `ddgi.rs` further.
- The exact placement of the DDGI probe update in each recorder's order was not traced. §4.4 states the constraint ("after it, where armed"), not a line.
- Whether an existing camera-cut signal exists to reuse for the fog history reset was not traced. §4.8 names a new request bit.
- `FogVolumeGpu`'s last two parameters (edge fade, height falloff) may need a sixth `float4`. V5's oracle decides the packing; 96 B × 32 is still 3 KB.
- The AC4 "double resolution" reading (160×90×128) underlies one anchor row (RESEARCH §7).
- The tree's camera field of view was not read; the far cascade's texel size (§4.6.1) assumes 60° vertical **[E]**.
- Whether the CSM depth recorder culls casters per cascade was not traced. D21 assumes the far cascade gets the CSM caster set culled to its own frustum; if the recorder has no per-cascade cull, the far pass draws every caster, bounded by the same [M] anchor.
- No ray rate for the reference GPU was found, so HWRT (§4.6.2) is unpriced.
- UE's default Dynamic Shadow Distance and Volumetric Fog View Distance were not fetched (the session's search budget ran out). The UE Sky Atmosphere page recommends a very large shadow distance for atmosphere shafts (T24), which supports D21's premise but gives no default.
- The review counted 41 ortho goldens in the tree; that count was not re-made here. D25 does not depend on it.

---

## 12. Review log — critique round 1 (`architecture-critic`, CHANGES_REQUESTED, 1 critical, 7 important, 8 optional)

Every remark was re-checked against the tree at `6394bc5e` or the cited source before it was acted on. **All 16 are accepted, and none is refuted.** O7's attribution to Wronski's HZB early-out is corrected: the deck presents that early-out as a performance optimization, not a leak fix.

| Remark | Verdict | What changed |
|---|---|---|
| **C1** fog past the CSM range is fully lit | **Accepted, confirmed.** `csm_visibility` returns 1 past the last split (`shadow_apply.hlsli:281`, `:313`); `DEFAULT_SHADOW_DISTANCE = 30.0` (`csm_config.rs:157`); every CSM example uses 3 cascades with that default. | D21, §4.6.1 (four options priced), a fog far cascade, the V2 (f) far-coverage gate with its red mutation and the `Shrink` handover test, the participation matrix, §3.1/§3.2 costs. |
| **W1** exposure missing until V9, no history rescale | **Accepted, confirmed.** Surfaces multiply by `H.exposure` (`deferred_pbr.hlsl:1479`, `:1481`); POSTFX §5.2 makes it the pre-exposure. | D23: pre-exposure at injection and in the tail from V2; the history rescale by `pre_N/pre_{N−1}`; V2 (e) exposure consistency; the V3 pre-exposure-invariance gate; K6 closed at V2. **Cross-document:** POSTFX §5.2 still names V9 as this campaign's consumer; it is now V2, which its owner should update. |
| **W2** V3's convergence gate cannot pass | **Accepted, confirmed by arithmetic.** The reset weight is 0.95¹⁵ ≈ 0.46, the steady-state weights are ∝ 0.95ᵏ, and the fp16 floor is 10 ULP. | V3 now uses the EMA recursion over history-Off readbacks at the scatter level, with a derived 20-ULP bound and an asserted non-vacuity precondition; §4.8 states the floor (P27) and the weighting. |
| **W3** no oracle for the fog's shadow taps | **Accepted, and stronger than stated.** The tree's own header comment describes the Y-flip the body removes (`shadow_apply.hlsli:114-115` vs `:260`). | V2 (g) visibility probe fixture: a host ray-box oracle, a cross-check against `csm_visibility`, an off-axis caster, and four red mutations; V4's atlas probe; shaft goldens blessed only after the probe, with a semantic assertion. |
| **W4** sun rays stop at min(30 m, 64 m); "converged" overstated; T3 mispriced | **Accepted.** The UE Sky Atmosphere page's shaft sentence was re-fetched and confirmed. UE's Light Shafts page publishes 0.5 ms (occlusion) at 1080p on a GTX 680, which RESEARCH §7 had called absent. | §0.2 wording; §4.6.3 far-field options priced; V13 and owner Q4; §4.6.4 the sharpness ceiling (16 px at 1440p, 24 px at 4K); T3 re-priced from the listed texture rate (166.4 GT/s): 1.42–2.8 ms full resolution, 0.18–0.35 ms at half resolution × 32 steps; D1 rewritten. |
| **W5** V7 bounds, march segment, typical fraction, edit regime, comparator | **Accepted, all five**, plus a sixth defect found while fixing (a): the leaf's penumbra reaches `T_MAX/SHADOW_K = 1.25 m` past the tight bound, so even per-kind bounds were unsound (P29). | §3.3 rewritten: a per-kind bound table (`boyko_sdf_math/src/lib.rs:150-153`, `sdf_field.hlsli:55`, `:84`), Σk/4 smoothness from the generated `smin` (`:151-162`), penumbra inflation, a soundness proof, the march origin stated (the cell centre, `T_MAX`, approach cost ≈ 5 steps), scenario fractions (2.5 % / 57 % / 100 %), validity ≤ 16 edits and the 256–4096 cost; §2.2 replaces the 36× claim with L′ at 0.05–0.18 ms; D9; V7 gates with three bound mutations. |
| **W6** particle variants undercounted; the FS decision not taken | **Accepted.** The decision is taken. The review's untried option (per-particle in `particle_sim`) was evaluated and **rejected on a measured layout fact**: the render record's colour lane is RGBA8 (`ParticleRender::color_rgba8`), so S would clip at 1.0. | D24 (one rule: always-bound + gate for every consumer draw shader, so no `PARTICLE_FOG` axis); D18 (the VS fold with the `S·α·(tex.rgb − 1)` error stated); the manifest recounted at 8 new `.spv` + one re-emit; a VFX cross-note (its VS stays 4, FS 24). |
| **W7** clouds scaled by the optimistic ratio | **Accepted, confirmed by arithmetic.** | One scaling method (RESEARCH §4.1): conservative = smallest applicable ratio. Clouds 0.60–1.86 ms at 1440p; Q2 and the V11 gate (≤ 1.9 ms) follow; the sky LUTs and T1 are re-scaled by the same method (GTX 1080 and GTX 680 specs fetched). |
| **O1** single `fog_integrated` | **Adopted** (`graph.rs:353-360`, `ddgi.rs:155-156`). | D5, §4.2, §3.2: 27.0 / 49.6 MB. |
| **O2** REFRACT double-fogs `[0, d]` | **Adopted, confirmed**: TRANSPARENCY R5 is α = 1 replace. | §4.9 formula `T(d)·L_s + τ·B + (1 − τ)·S(d)`, D17, the V8 clear-glass gate. |
| **O3** HWRT not scored | **Adopted as a scored option** (`render_path_config.rs:489-491`). | §2.1 row M, §4.6.2: not the default (feature build, unpriced ray rate), a candidate on `hwrt` builds. |
| **O4** Q4 is technical | **Adopted.** | D22; Q4 withdrawn and the number reused for the far-field scope question. |
| **O5** ortho; NDC z under the marcher matrix | **Adopted, confirmed** (`cluster_cull.hlsl:110-114`, `motion_cam.rs:58`, `SHADER-VARIANT-MANIFEST.md:84`). | D25; `FogFrameGpu.prev_eye/prev_forward`; the reprojection leaf; V2 (b) and V3 run under both modes. |
| **O6** logged overflows | **Adopted.** | D26, `FogStats`; the fog cull's alloc word already sums the demand (`cluster_cull.hlsl:376`). |
| **O7** thin-wall leaking | **Pitfall adopted as P25; attribution corrected.** Wronski's deck presents HZB early-out as a performance optimization (p. 49) and notes that such culling voids the 3D-history property (p. 58); it is not a leak fix. | §4.3 far-edge mapping (removes the depth half); K14 (the XY half accepted, escape priced); §4.4 says why the HZB early-out is not taken. |
| **O8** numeric nits | **Adopted, all four.** | Frostbite's apply subtracted: anchors 0.42–1.03 ns (every derived number updated); occupancy 7.1 in §3.4; D15 cites UE's "per view (and per tile for consistency)"; C2 names both histories. |

**The review's open questions, answered.**

1. **Deferred's fog ambient.** It is the same SkyLight row every path reads: Deferred `deferred_pbr.hlsl:1236-1237`, Forward `forward_opaque.fs.hlsl:301-302`, VB `vb_shade.comp.hlsl:520-521`. `fog_advance_frame` copies it (§4.7).
2. **Ortho.** Supported (D25).
3. **REFRACT composition.** α = 1 replace (TRANSPARENCY R5), handled by the §4.9 formula.

**Found while revising (not in the review).**

- The prefilter's penumbra hole (W5 row).
- V2 (b)'s 1e-3 tolerance was unreachable under B10G11R11 and under 4-bit sub-texel filter weights. The oracles now force the RGBA16F arm, place probes on exact texel centres, and derive 2e-3 (§7 preamble).
- The apply's half-slice depth overshoot (§4.3, P25).
- The stale Y-flip header comment in `shadow_apply.hlsli:110-116` is a tree defect outside this campaign. It is recorded here, not edited.
