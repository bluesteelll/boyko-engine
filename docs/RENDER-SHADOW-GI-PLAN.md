# Render — Shadow + Lighting + GI Architecture Plan

Branch `ecs`. The complete technique stack for the HYBRID SDF + raster-mesh renderer, chosen
per-case for MAXIMUM PERFORMANCE (HYBRID-PERF-DECIDES). Two verified deep-research passes
(shadows/lighting `wf_08f852c7`, dynamic GI `wf_fe40fa5b`) back the recommendations below.

## Guiding principle — lean on the SDF
The engine already owns a **GPU brick-atlas SDF + an analytic per-frame marcher**. The expensive
part of UE5 Lumen / Godot SDFGI / VXGI is BUILDING the scene SDF (mesh-distance-field bake,
voxelization). **We have it.** So we do as MUCH as possible — shadows, AO, and GI — through the SDF
march (cheap, unified, fully dynamic, soft for free), and reach for shadow maps / RT ONLY where the
SDF can't cheaply cover the case (exact-silhouette dynamic hero meshes).

## What exists (Phase 0)
- Deferred G-buffer + **clustered/froxel light culling** (unified deferred+forward light list).
- **Analytic SDF soft shadows + AO** (the marcher marches the field toward the light, fully dynamic).
- **SDF-proxy shadows for simple meshes** (box proxies; the mesh wins ownership, the proxy casts).
- **Brick-atlas SDF** (baked distance fields in a 3D atlas — the M-phase campaign).
- Perspective raster-mesh aligned to the marcher (linear `SV_Depth`).

## The matrix (geometry × technique)
| Geometry / case | Best-perf technique | Reuses |
|---|---|---|
| SDF objects | analytic SDF march | marcher (have) |
| Simple mesh (box, crate) | SDF box proxy | proxy mechanism (have) |
| **Dynamic character (skinned)** | **CAPSULE proxies + SSCS** | marcher + proxies (capsule = SDF primitive) |
| Complex STATIC mesh (env) | **MDF** in brick-atlas + cached CSM for the sun | brick-atlas |
| Sun / hero, exact silhouette | **CSM + static/dynamic cached split** | new |
| Many dynamic point/spot | clustered + **sparse cube/spot maps** | cluster cull (have) |
| Static light on static geo | **baked lightmap + shadowmask + probes** | new |
| **Dynamic diffuse GI** | **SDFDDGI** (probe grid updated by the marcher) | marcher + SDF shadow trace |
| Reserve (sparse low-res SM) | RT-threshold-hybrid (< ~96 texels → RT) | optional |

### Why these (verified)
- **Capsule proxies for characters:** skinned/animated meshes invalidate shadow-map caches EVERY
  frame (skeletal anim + WPO) → cached SMs are useless for them; capsules cost ~0.29 ms first
  char + ~0.05 ms each (UE), and capsules ARE analytic SDF primitives the marcher cone-traces. RT
  shadows are uneconomical on skinned meshes (per-frame BLAS rebuild). So capsules win.
- **MDF:** UE5 Mesh-Distance-Field shadows/AO; the mesh's SDF baked into the brick-atlas, the march
  casts it. We already have the atlas; UE bakes MDFs offline (the build we skip).
- **Cached static/dynamic shadow split:** UE5 Virtual Shadow Maps / HDRP Mixed Cached / Unity
  Shadowmask — render static casters once, re-render only the dynamic part.
- **Clustered shading:** same-or-better than tiled deferred, strictly better worst-case; the forward
  path lights SDF-marched pixels alongside deferred mesh pixels.
- **SDFDDGI** (Hu et al., The Visual Computer 2021, arXiv:2007.14394): an octahedral irradiance-probe
  grid whose probes are updated by SPHERE-TRACING the SDF (no RT hardware), fully dynamic in light +
  geometry, multi-bounce via previous-frame probe feedback, SDF-shadow-trace visibility. **1.67 ms**
  on RTX 2080Ti vs RTXGI 3.98 / RT-GI 4.13 / VXGI 5.24 ms. We escape SDFGI's semi-static limit (it
  re-voxelizes; our marcher reads the field per frame). The ONE new piece: the probe atlas + a
  radiance cache (an SDF hit gives position+normal, no radiance) — the probe atlas IS that cache, no
  offline cards (unlike Lumen's Surface Cache).

## Phased build order
1. **Capsules** — a `Capsule` SDF primitive (through the eDSL, byte-identical) + character capsule
   proxies → cheap dynamic character shadows. Highest ROI (reuses the march + proxy mechanism).
   Watch `MAX_SDF_EDITS = 16`: a character is ~6-10 capsules; a coarse 5-6-capsule character + scene
   stays under the cap, OR raise the cap / add a separate shadow-caster edit list.
2. **MDF** — bake a static mesh into the brick-atlas; the march casts its shadow/AO.
3. **SSCS** — a short screen-space depth ray-march per light for fine contact detail (cheap add-on).
4. **CSM + static/dynamic cached split** — directional sun + hero meshes needing exact silhouettes.
5. **Sparse cube/spot shadow maps** — many dynamic point/spot lights (clustered).
6. **Baked lightmap + shadowmask + probes** — static light on static geometry (free at runtime).
7. **SDFDDGI** — dynamic diffuse GI: an octahedral probe grid + marcher probe-update + temporal blend
   + last-frame feedback. The biggest long-term win (the SDF advantage), the most new work.

## Open questions (deferred / need their own research)
- **Radiance Cascades** (Sannikov / PoE2) — no verified evidence survived; its real 3D cost + SDF
  composability is unknown. A possible SDFDDGI alternative; re-research before committing.
- **Specular / reflection GI** — the GI evidence is diffuse-only; can the marcher serve glossy
  reflections cheaply (narrow SDF cones + radiance probes), or is a dedicated reflection trace needed?
- **Brick-atlas probe-update cost** — the SDFDDGI numbers used simple analytic primitives; our deep
  CSG edit-list fold (`MAX_SDF_EDITS` brute-force) is heavier per march step — measure on the real
  field (the empty-skip acceleration should recover it).
- **Cached-shadow VRAM budget** — the static/dynamic split doubles atlas residency; confirm the
  combined CSM + sparse-cube + brick-atlas + lightmap budget fits at the target resolution.

Every phase: build CPU-green + a `#[ignore]` offscreen screenshot dump → render on RTX → owner is the
visual oracle → commit + push after the owner's visual OK. Shaders go through `boyko_shaderdsl`
(byte-identity). 0%-gate every addition.
