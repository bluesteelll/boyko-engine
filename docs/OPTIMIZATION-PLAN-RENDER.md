# Render Optimization Plan

> Status: implementation-ready, sequenced (v2 — full track expansion). From the shipped deferred + SDF-hybrid base into a production-scale renderer that exploits the SDF field we already evaluate. Companion to `docs/RESEARCH-GRAPHICS-OPT.md` (priority/sequence), `docs/PERF-DIRECTIONS.md` (RT-*/GPU-D* IDs), `docs/RESEARCH-FAST-MATH.md` (the determinism boundary the physics path reuses).
>
> **v2 mandate.** v1 (P0→P15) graduated the golden fixture into a real deferred + SDF-hybrid renderer and stopped at HW-RT lighting. The owner verdict: that selection is too meagre — it under-weights the **SDF-native** wins (cone-trace the field we already evaluate for shadows/AO/GI; the brick atlas as a universal acceleration structure for surfaces, fog, particles, and secondary rays) and the **marcher-acceleration** wins (over-relaxation, Lipschitz pruning, mesh-depth seeding) that directly cheapen the field. v2 adds nine tracks (A SDF-native lighting + A-GI, B marcher acceleration, C reconstruction/AA/VRS, D GPU-driven geometry, E volumetrics/post/OIT/particles, F render-graph/RHI substrate, S classic shadow/AO complement, G frontier GI/neural/PT) plus three cross-cutting infra phases (F-GRAPH render-graph, X-SPD shared mip primitive, X-REF stochastic acceptance oracle) that the critique proved are load-bearing.
>
> **Changelog vs the reviewed v2 draft** (every critique point resolved):
> - **C1** — **F-GRAPH promoted to a near-spine phase** (lands after P1/P3a, before any Track E/S/G pass opens) and **defined in full here** (§F-GRAPH). Its barrier representation reserves the (src-queue, dst-queue) dimension from day one (resolves M7). Every Track E/S/G phase now declares "lowered by F-GRAPH" instead of hand-rolling barriers. Critical-path diagram and priority table updated.
> - **C2** — Added **D-FWD** (transparent/blended material substrate) and made the mesh-material-G-buffer question an explicit owner decision (Open-Q11). Every false "needs P1" restated as "needs P1 + <the specific blend/raster/light-list capability>". E-WBOIT/MBOIT/LLOIT/E-PART/S-CSM now depend on D-FWD and/or P7 explicitly.
> - **C3** — **E-DENS split** into E-DENS-A (independent render-authored density channel — clean RENDER) and E-DENS-B (density derived from the already-evaluated `field_distance` scalar — the field probe stays frozen, only the extinction remap relaxes). The blanket "fast-math OK" is removed.
> - **C4** — Added **X-REF** (the statistical acceptance oracle): an in-house offline path-traced reference generator + a fixed convergence budget + a named metric/threshold per stochastic phase + an owner statistical-bar sign-off. Every Track-G stochastic phase now lists X-REF as a hard prerequisite (resolves Open-Q9's hand-wave).
> - **M1** — **Priority inverted.** Track B (marcher acceleration) + Track A1/A2/A-SDFSHADOW (SDF-native surface shadows/AO) are sequenced **immediately after the spine, before all of Track E post and Track G neural.** The generic post chain (E-BLOOM..E-LUT) is explicitly demoted to **polish-tier**. E-SDFVOL no longer hard-requires the full froxel-fog pipeline — A1/A2 give a direct surface-shading SDF soft-shadow/AO path.
> - **M2/M5** — E-SDFVOL and E-PART SDF-collision cost is **bounded** and declared **P9-required above the ≤16-edit fixture**; "free"/"a few cone steps" replaced with an O(consumers × edits/brick-fetch) cost statement.
> - **M3** — The **P6-S semaphore-present seam** is named as an explicit prerequisite every cross-frame phase lists directly (not transitively through "needs P6").
> - **M4** — **X-SPD** promoted to a single parametric mip-reduce primitive (reduction-op + format interface); P11/S-HIZ/S-GTAO/E-BLOOM/G-REBLUR **consume** it. The odd-dimension boundary fix ships once.
> - **M6** — Added the **render↔physics geometric-divergence contract** (Open-Q12): the R16 brick field's world-space error bound vs the analytic physics field, owner-signed.
> - **m1** — E-GOD route (b) screen-space radial blur demoted to optional-only.
> - **m2** — E-MBOIT moment-inversion mitigation made concrete (stable Cholesky + documented epsilon bias regardless of fast-math).
> - **m3** — `VK_NV_cooperative_vector` explicitly classified as an acceptable raw extension (not a forbidden vendor SDK), with the KHR/subgroup fallback mandatory.
> - **m4** — The "~80% free via P6" framing is corrected: each denoiser's core (à-trous / reservoir / edge-stopping) is counted as real per-phase effort; shared infra is the motion-vector/history/reprojection seam only.
> - **Completeness gaps** — Added **§VRAM Budget** (sums brick atlas + clipmap + VSM pages + reservoir SSBOs + froxel/fog 3D textures + G-buffer against 6 GB), **§PSO/Permutation discipline**, and **§G-buffer Bandwidth** accounting (the D-VIS visibility-buffer tension quantified).

---

## 0. Shipped reality this plan attaches to (verified in-repo)

| Subsystem | State | Cite |
|---|---|---|
| On-screen frame | Compute composite → packed `RWStructuredBuffer<uint>` → `present_sampled` fullscreen blit | `swapchain.rs:1661`; `sdf_depth_composite.hlsl:78` |
| SDF marcher | **Golden fixture**: `IMG_W=IMG_H=64u`, `MAX_SDF_EDITS=16u`, `MAX_IT=128u`, ORTHOGRAPHIC, one binding, CPU-written edits; folds the full edit-list from `t=0`, bounded only by shared mesh depth | `sdf_depth_composite.hlsl:86-87,104,117,204,247-252` |
| Field math = golden source of truth | `sdf`/`smin`/`combine`/normal mirrored EXACTLY host-side (`golden_composite_pixel`); **a future CPU physics evaluator reuses the SAME field math** | `sdf_depth_composite.hlsl:8-12,47-51` |
| MRT G-buffer | Offscreen golden-tested rungs only (`gbuffer.fs`/`deferred_light.fs`, `color_formats: &[Format]` MRT). NOT the on-screen path. **Mesh albedo is flat-color** — "reading the mesh's real rasterized albedo from a G-buffer is a deferred refinement" (`sdf_depth_composite.hlsl:92-95`) | `rhi_impl.rs`; `gbuffer.fs.hlsl`; `deferred_light.fs.hlsl` |
| Shared depth | Real D32 prepass → `copy_image_to_buffer` into a DEPTH region (64×64 = 16 KB/frame) | `swapchain.rs:2071` `sync_depth` |
| Queue | ONE graphics+compute family, `queue_count:1`, **fence-only `submit`, NO semaphores** (`submit_windowed` is an unbuilt seam) | `device.rs:1634,1750`; `queue.rs:3-5,22,31` |
| On-screen barriers | **Hand-FFI** `(self.fns.cmd_pipeline_barrier)`, one image barrier per call, **11 sites** | `swapchain.rs:844,914,1169,1201,1337,1370,1432,1708,1831,1864,1926` |
| ECS-edge barriers (distinct path) | `lower_barriers` lowers conflict-graph edges into `PlannedBarrier` POD keyed by `(ArchetypeId, ComponentId)`, replayed by `GpuSystem` — the **compute-column** path, never touches the on-screen frame | `boyko_render/barrier.rs:1-15,58-76,196-233` |
| Barrier constants | `enums.rs` lacks `DRAW_INDIRECT`/`INDIRECT_COMMAND_READ`/`ALL_COMMANDS`/`MEMORY_*`; widen is `COMPUTE_SHADER\|TRANSFER` only | `enums.rs:101`; `barrier.rs:36,121-124` |
| Bind groups | `create_bind_group`/`_layout` **COMBINED_IMAGE_SAMPLER-only**; compute path binds exactly one storage buffer; no blend-state raster pipeline beyond the prototype | `rhi_impl.rs:717`; `encoder.rs:38` |
| Indirect | `dispatch_indirect` `#[cold]` no-op stub; `BufferUsage::INDIRECT` exists | `encoder.rs:269`; `enums.rs:37` |
| HW-RT | No `AccelerationStructure`/`RayTracingPipeline` associated type | `api.rs:46-67` |
| Sub-allocator | Free-list sub-allocator (F-MEM/F-ALIAS extend it) | `suballocator.rs` |
| Threadpool | Chase-Lev work-stealing pool (F-PSO uses it for parallel PSO warm) | `boyko_threadpool/` |

**GPU oracle = RTX 3060 Laptop (6 GB).** Every phase gated by golden-image-equality + Vulkan validation + sync-validation + GPU-timestamps on this device.

**Inviolable gates carried into every phase:** 0%-gate (a world not using `boyko_render` pays nothing; named CPU hot loops `row_ptr`/`for_each_chunk`/query-iter/`find_ready` byte-identical); in-house only (raw-FFI Vulkan 1.3, no ash/wgpu/any external GPU lib — every open-source reference FSR2/XeGTAO/NRD/VkNRC/FidelityFX-SPD/meshoptimizer is **reimplemented**, never linked); native (not web); golden-image-equal + GPU-timestamp + validation/sync-validation clean; every `unsafe` carries `// SAFETY:`; **the deterministic scalar field eval (`sdf`/`smin`/`smax`/`combine`/normal) + `golden_composite_pixel` stay byte-identical — no fast-math, no reordered FMA, no rsqrt/rcp, no FP16 — because physics collision reuses them through the single gateway `sdf_field.hlsli` (the P4 invariant).**

---

## 0a. The determinism contract (THE load-bearing constraint)

Three sources fix this (`RESEARCH-FAST-MATH.md:7-19`, `sdf_depth_composite.hlsl:8-12,47-51`):

1. **Float reassociation** (algebraic intrinsics, FMA contraction, autovec of float reductions) reorders adds → non-deterministic even within one run.
2. **`rsqrtps`/`rcpps` are implementation-defined and differ Intel-vs-AMD** — wrong-by-vendor, not merely non-portable. `sqrtss`/`sqrtps`/`divps` ARE IEEE-754-exact and bit-identical everywhere.
3. The CPU physics SDF-collision narrowphase evaluates the **same** `sdf`/`smin`/`combine` math the GPU marcher does; a render-side reordering would silently desync physics from visuals.

**Per-phase determinism class (the firewall):**

- **FROZEN** — IS the field eval. `sdf`/`smin`/`smax`/`combine`/normal in `sdf_field.hlsli` + the host mirror `golden_composite_pixel`. Byte-identical across the ENTIRE plan. No exceptions.
- **FIELD-CONSUMER** — CALLS the frozen field at offset points (shadow/AO/visibility/secondary rays, particle collision, in-brick solve). The field *probe* goes through `sdf_field.hlsli` with NO fast-math; the *accumulation around it* (min-tracking penumbra, cone weights, reservoir math) is consumer-side and MAY relax.
- **RENDER** — never touches the field. Post, OIT, reconstruction, classic shadows/AO, neural. Fully relaxable. Temporal/stochastic non-determinism here is OUTSIDE the physics gate by design.
- **FORKED-BACKEND** — P9/P10/P12: the field *backend* (brick fetch) is forked behind `field_distance` for render; the analytic path remains the FROZEN reference + the physics source of truth (physics never reads bricks). Carries a documented render↔physics geometric-divergence bound (Open-Q12).

**The single field gateway is `sdf_field.hlsli`** (the P4 invariant). Every field touch in the whole engine — render and physics — goes through it. Two traps the critique surfaced:
- **B9's cheaper 4-tap tetrahedron normal is FROZEN-adjacent** — keep the 6-tap central-difference normal as the frozen physics-shared normal; fork a render-only 4-tap behind a separate function (Open-Q2).
- **E-DENS-B's "fast-math OK"** applies ONLY to the extinction remap of an *already-evaluated* scalar `d = field_distance(p)`; the `field_distance` call itself is frozen (C3).

---

## 1. Critical path (dependency-ordered)

```
─── SPINE (graduate the fixture into a real renderer) ────────────────────────────
P0   Production scene substrate (res-as-dispatch, perspective cam, ECS edit feed)  [L]   first
P1   MRT G-buffer (+P2 descriptor vocab) + shared-depth image + kill depth-copy    [XL]  ← P0
P3a  On-screen barrier batching (hand-FFI swapchain.rs)                            [M]   ← co-req P1
F-GRAPH  Render-graph: auto-derive barriers + resource lifetimes for BOTH          [L]   ← P1+P3a  ★ gates the pass explosion
         codepaths from pass declarations; reserves the queue-ownership dimension
P4   Hierarchical tile-cull / coarse pre-trace (behind sdf_field.hlsli)            [M]   ← P0+P1
P5   Half-res trace + depth-aware upscale  (FORKED; golden marcher frozen)         [M]   ← P0+P4
P6   Motion vectors + history + TAA  (FORKED; lands the P6-S semaphore seam)       [L]   ← P0+P1+P5
─── SDF-NATIVE FAST-TRACK (the owner-named wins — land right after the spine) ─────
B1   Over-relaxation sphere-tracing (ω-gated)                                      [S]   ← P4    FIELD-CONSUMER
B5   Mesh-depth + previous-frame march seeding                                     [S]   ← P1+P6 FIELD-CONSUMER
B7   Lipschitz / bound pruning of the edit-list fold (analytic, distance-exact)    [M]   ← P4    FROZEN-preserving
A1   SDF cone-trace soft shadows (Quilez penumbra)                                 [M]   ← P4    FIELD-CONSUMER
A2   SDF 5-tap ambient occlusion                                                   [S]   ← P4    FIELD-CONSUMER
C-AA Analytic edge AA from the distance field                                      [S]   ← P4    FIELD-CONSUMER
─── PARALLEL-AFTER-P1 (independent of the SDF spine) ──────────────────────────────
P3b  enums constants + boyko_render::barrier.rs narrowing                          [S]   ← P1   serves P8/P11
P7   Clustered/froxel deferred light culling (bitfield + subgroup)                 [L]   ← P1
P8   GPU-driven indirect dispatch (render-local count)                            [M]   ← P3b  prereq P11
D-FWD Transparent/blended material substrate (blend-state raster + sorted pass)   [M]   ← P1+P7  prereq all OIT + S-CSM + E-PART render
X-SPD Parametric SPD mip-reduce primitive (one build, many consumers)             [S]   ← P1   prereq P11/S-HIZ/S-GTAO/E-BLOOM/G-REBLUR
─── RECONSTRUCTION / VRS / CLASSIC SHADOW-AO (after P6) ───────────────────────────
C-TSR  Full FSR2/TSR temporal super-resolution (supersedes simple P5/P6)          [L]   ← P5+P6
C-CTSS/C-VRS/C-CBR  Checkerboard / VRS / coarse shading                           [M/M/M] capability
S-HIZ  Hi-Z shared accelerator (canonical X-SPD consumer)                          [M]   ← P1+X-SPD  feeds S-GTAO/S-CONTACT/A1/P11
S-GTAO / S-CONTACT / S-CSM / S-VSM  Classic shadow/AO complement (mesh side)      [L/M/L/XL]
─── VOLUMETRICS / POST / OIT / PARTICLES (after P6+P7; post = polish-tier) ────────
E-FOG  Froxel volumetric fog/lighting                                             [L]   ← P1+P6-S+P7
E-SDFVOL  SDF-native analytic volumetric self-shadow & AO (P9-required at scale)  [M]   ← P4(+P9)  FIELD-CONSUMER
E-DENS-A/B  Brick-atlas participating-medium density                              [L]   ← P9+E-FOG threshold
E-CLOUD  Volumetric clouds                                                        [XL]  ← E-FOG+P5+P6-S threshold
E-GOD  God rays (route a = free from E-SDFVOL; route b optional)                  [S]   ← E-FOG
E-WBOIT / E-MBOIT / E-LLOIT  OIT ladder                                           [S/L/L] ← D-FWD
E-PART  GPU particle system (+ free SDF collision, P9-required at scale)          [L]   ← P1+P8+D-FWD  RENDER+FIELD-CONSUMER
── POLISH-TIER POST (low priority; any-engine commodity) ──────────────────────────
E-BLOOM / E-DOF / E-MBLUR / E-EXP / E-TONE / E-LUT                                [M..S] ← P1(+P6-S)
─── A-GI: SDF-NATIVE GLOBAL ILLUMINATION (software-first, no HW-RT) ────────────────
A8   Radiance Cascades 2D (noiseless, no bricks)                                  [L]   ← P1+P6    RENDER
A13  SSGI bitmask                                                                 [M]   ← P1+P6+S-HIZ RENDER
A7/A10/A12  VCT / DDGI / Brixelizer-class GI (over bricks)                        [L/L/XL] ← P9  FIELD-CONSUMER
─── THRESHOLD/CAPABILITY/PROFILE-GATED SDF-ACCEL + GPU-DRIVEN ──────────────────────
P9   SDF brick atlas (field backend swap behind P4 invariant)            [XL] threshold ← P4 invariant; re-eval vs B7 first
P10  Clipmap LOD over bricks                                             [L]  threshold ← P9
G-BRICKRT  Software brick-RT (hierarchical-DDA) — RT-CORE-FREE KEYSTONE  [L]  ← P9      FIELD-CONSUMER
P11  Hi-Z two-pass mesh occlusion culling                               [L]  threshold ← P8+S-HIZ
P12  BVH dirty-region regen + JFA (+ SDF motion vectors for mutating)   [XL] threshold ← P9
D-VIS / D-SWRAST / D-MESH / D-CULL / D-COMPACT  GPU-driven geometry      [L..XL] threshold/capability
P13  Async compute (multi-queue + cross-queue semaphores)               [XL] profile   ← P3a/P3b + P6-S seam
─── HW-RT (optional accelerator over the software path — NOT the GI prerequisite) ──
P14  AS/RT seam + HW-RT SDF-as-AABB-BLAS backend                        [XL] capability← P9; fallback G-BRICKRT
P15  RT lighting (shadows/AO/GI/reflections) + SVGF                     [XL] confirmed ← P6+P14+G-SVGF
─── TRACK G: FRONTIER GI / NEURAL / PATH-TRACING ──────────────────────────────────
X-REF  Stochastic acceptance oracle (offline PT reference + metric/bar) [M]  ★ prereq for ALL stochastic Track-G
G-SVGF / G-REBLUR / G-FIRE  Denoisers (consume P6 + X-SPD)              [L/L/S] ← P6-S
G-RDI / G-RGI / G-REGIR / G-PT  ReSTIR DI/GI/world-grid/PT             [L/L/M/XL] ← P6-S+P7+X-REF+(G-BRICKRT|P14)
G-SHARC  Spatial-hash radiance cache (non-neural NRC twin)             [M]   ← G-RGI/PT  RENDER+FIELD-CONSUMER
G-SOIT  Stochastic OIT (absorbed by TAA)                               [M]   ← P6-S+denoiser
G-NRC / G-NDENOISE / G-NUP / G-NMAT  Neural (coopmat-gated)            [XL..L] capability; non-neural twin = mandatory fallback
─── RENDER-GRAPH / RHI SUBSTRATE EXTENSIONS (cross-cut, land as needed) ────────────
F-SYNC2 / F-BIND / F-FP16 / F-PUSH / F-MEM / F-ALIAS / F-PSO          [S..M] capability/threshold
```

**The spine is P0→P1→P3a→F-GRAPH→P4→P5→P6.** F-GRAPH is inside the spine boundary (the critique's C1): it lands once the 4-pass frame (P1) and batched barriers (P3a) exist, and it MUST precede the Track-E/S/G pass explosion or every later phase hand-rolls barriers across two codepaths — an O(passes²) hazard-tracking liability.

**Why this order, concretely:**
- **P0 first** — the shipped marcher is a 64×64/≤16-edit fixture, not march-step-bound; every ray-budget claim is unmeasurable until P0 instantiates a resolution, perspective camera, and ECS edit feed.
- **P1 second** — the packed `u32` buffer conflates depth+attributes+output and forces a per-frame depth-image→buffer copy; every downstream opt needs real attribute images.
- **F-GRAPH inside the spine** — 60+ later passes against two hand-maintained barrier paths do not scale; auto-derived barriers from pass declarations are the single highest-leverage infra item (C1).
- **SDF-native fast-track right after the spine** — Track B (cheapens the field we already evaluate) + A1/A2/C-AA (the owner-named SDF-native lighting) are the highest-ROI work and land before any commodity post or neural filler (M1).
- **Software/SDF-native GI is primary, HW-RT is an accelerator** — G-BRICKRT makes the brick atlas itself the acceleration structure, so all of ReSTIR/PT/cache run with NO RT cores; P14/P15 become an optional accelerator for incoherent rays, not the GI prerequisite (Open-Q6).
- **Post chain is polish-tier** — E-BLOOM..E-LUT are any-engine commodity; they never compete for sequencing attention with the differentiators (M1).

---

## 1b. Windowed-present synchronization model (the P6-S seam)

`submit` takes no semaphores today (`queue.rs:22,31`). Two scopes:
- **Intra-frame (one command buffer):** the multi-pass frame synchronizes via pipeline barriers in a single recorded command buffer (the existing `record_scene` model). Sufficient for P0–P5.
- **Cross-frame (acquire→render→present + history):** P6's history and P11's "last frame visible set" depend on the previous frame's output. This needs the acquire-image / render-finished **semaphore chain** `submit` cannot express. **The `submit_windowed` semaphore-present seam — named P6-S — lands as P6's first sub-step.** P13 extends it to cross-queue; it does not invent it.

**P6-S is a named prerequisite (resolves M3).** Every cross-frame phase — E-EXP, E-CLOUD, E-MBLUR, all of Track G (reservoir ping-pong, G-SHARC temporal accumulation), C-TSR, S-VSM (page caching) — lists **needs P6-S** directly, not transitively through "needs P6". P6 ships the FULL cross-frame semaphore chain, not merely intra-frame TAA, before Track G opens.

---

## 1c. Per-phase block legend

Every phase carries: **What / Why our path needs it / How (raw-Vulkan-optimal) / Expected win (GPU-timestamp; never a quoted speedup) / Dependency-order / Gate / Effort / Conflicts / Determinism class.** All HW-dependent phases carry a `DeviceCaps` query at device-create + a named in-house fallback. All barriers + resource lifetimes are **lowered by F-GRAPH** once it lands (phases before F-GRAPH use the hand-FFI/`lower_barriers` paths it later subsumes).

---

# SPINE (P0–P6) — preserved verbatim from v1

> P0–P6 are unchanged in intent from v1; the full bodies live in v1 §P0–§P6 and are summarized here for the unified track scheme. The one structural change: **F-GRAPH is inserted after P3a** (see its phase below). The spine exit criteria (§3) are unchanged.

- **P0 — Production scene substrate.** Resolution-as-dispatch-dim, additive perspective ray-gen (ortho golden-frozen), ECS `SdfEdit` feed. Target regimes: 1080p/720p; 256–4096-edit "scene" band; sparse (~70% empty) + dense canonical scenes. Gate: rung-8..11 ortho goldens bit-exact; sparse/dense GPU-timestamp baselines recorded. **L. RENDER (ray-gen only; field FROZEN).**
- **P1 — MRT G-buffer + descriptor vocabulary + shared-depth image + kill depth-copy + marcher single→multi-binding rewrite.** Descriptor vocab `{StorageImage, SampledImage, CombinedImageSampler, StorageBuffer, UniformBuffer}`; G-buffer `depth(D32) / normal(R8G8B8A8, octahedral later per Open-Q3) / albedo(R8G8B8A8) / material(R8G8B8A8)`; marcher rewritten to write STORAGE images + sample the depth image. Field eval copied **verbatim** (FROZEN). Gate: golden-equal ±2/255 vs packed-buffer; depth-copy GONE. **XL.**
- **P3a — On-screen barrier batching (hand-FFI swapchain.rs, 11 sites).** Batch N transitions into one `cmd_pipeline_barrier`. Masks unchanged (batch, don't narrow). Gate: barrier-call count down; sync-validation clean. **M. RENDER.**
- **P4 — Hierarchical tile-cull / coarse pre-trace.** 1/8-res coarse cone-trace → per-tile `near_t`/`empty`; fine marcher starts from `near_t`. **Establishes the `sdf_field.hlsli` invariant** (`field_distance(p)` + `tile_bound(tile)`) — the verbatim cut of the FROZEN field math shared with `golden_composite_pixel` and the physics evaluator. Gate: conservative golden (a wrongly-empty tile is a hole). **M. FIELD-CONSUMER.**
- **P5 — Half-res trace + depth-aware upscale (FORKED).** New half-res compute pass *calls* `sdf_field.hlsli` (unchanged) at jittered origins; joint-bilateral upscale via full-res depth/normal. Field eval byte-identical (diff-verified). Gate: relaxed tolerance (owner-approval); ±4/255 smooth + edge no-bleed. **M. RENDER (consumer; field FROZEN).**
- **P6 — Motion vectors + history + TAA (FORKED). Lands the P6-S semaphore seam (sub-step 0).** `R16G16_SFLOAT` motion vectors; double-buffered history; YCoCg neighborhood-clamp resolve. Static analytic field + jittered camera → deterministic camera reprojection (per-hit velocity for a mutating field is deferred to P12, Open-Q7). Gate: static converges to P5 reference; pan no-ghost; disocclusion no-smear; field eval byte-identical; P6-S sync-validation clean. **L. RENDER (consumer; field FROZEN).**

---

# F-GRAPH — Render-graph (resolve C1) — NEAR-SPINE INFRA ★

**What.** A declarative render-graph that auto-derives **barriers + resource state-transitions + transient-resource lifetimes/aliasing** from per-pass resource declarations, for **BOTH** existing barrier codepaths (the hand-FFI `swapchain.rs` on-screen frame AND the `boyko_render::barrier.rs` ECS-edge compute-column path). A pass declares its reads/writes (image/buffer + the access + the stage); the graph topologically orders passes, inserts the minimal barrier set, transitions layouts, and computes transient-resource aliasing windows. Until F-GRAPH lands, P0–P5 use the hand-FFI path (batched by P3a) and the compute-column uses `lower_barriers` (narrowed by P3b); F-GRAPH **subsumes both** behind one pass-declaration API.

**Why our path needs it (C1).** v2 adds ~60 passes (Track E ~14, Track S 6, Track G ~15, Track A/C/D the rest). Each needs barrier lowering, resource lifetime, and aliasing. Today there are **two distinct hand-maintained barrier codepaths** (verified: `swapchain.rs` 11 hand-FFI sites + `boyko_render/barrier.rs::lower_barriers`). Adding 60 passes against two hand-maintained paths is an **O(passes²) hazard-tracking liability** and a sync-validation nightmare; the spine's 0%-gate and the sync-validation oracle do not scale to 60 hand-rolled passes. F-GRAPH is the single highest-leverage infra item and MUST land before the pass explosion. It generalizes `barrier.rs`'s existing edge→barrier algorithm (which already lowers conflict-graph edges into `PlannedBarrier` POD, `barrier.rs:196-233`) from `(ArchetypeId, ComponentId)` keys to image/buffer resources.

**How (raw-Vulkan-optimal, in-house).**
- A pass-declaration POD: `PassDesc { reads: &[ResourceUse], writes: &[ResourceUse], stage, queue }` where `ResourceUse { handle, access, layout }`. No `dyn` — passes are monomorphized records, the graph walks POD slices (the `lower_barriers` POD discipline extended).
- A build step (setup-time, NOT per-frame hot): topological sort over the read/write dependency DAG; for each edge emit the minimal `(srcStage, dstStage, srcAccess, dstAccess, oldLayout, newLayout)` — reusing P3b's narrowed `enums.rs` constants and P3a's batching (N barriers per sync point in one call). The build is cached and replayed; only resource handles rebind per frame (no per-frame graph rebuild — the rung-10 const-assert discipline applies to the cached barrier table).
- **Reserves the (src-queue, dst-queue) dimension from day one (resolves M7).** Every `ResourceUse` carries a queue field; until P13 it is always the single queue, but the barrier representation can express a queue-family-ownership transfer without a rewrite. P13 (async) populates the second queue; F-GRAPH's data model already holds it.
- Transient-resource aliasing: passes whose lifetimes do not overlap share backing memory via the `suballocator.rs` free-list (feeds F-ALIAS).
- The seam-with-default-`#[cold]`-body pattern keeps Mock + ABI stable; static dispatch, no `dyn` in the record path.

**Expected win.** Eliminates hand-written barriers for all 60+ later passes (correctness + maintainability — the real win); fewer redundant barriers than hand-rolling (the graph computes the minimal set); transient aliasing cuts peak VRAM (feeds §VRAM Budget). Measured by sync-validation cleanliness across the full pass set + barrier-count + peak-VRAM vs a hand-rolled baseline.

**Dependency/order.** Needs P1 (the multi-pass frame to model) + P3a (the batched-barrier primitive it emits into) + P3b's constants (the narrowed masks it uses). **Inside the spine boundary; precedes every Track E/S/G phase.** Every later phase declares "lowered by F-GRAPH".

**Gate.** Every spine pass (P1's 4-pass frame) reproduces its golden when barriers are F-GRAPH-derived instead of hand-written (byte-identical image); a deliberately-omitted declaration trips sync-validation (the negative test — the graph's correctness IS the sync-validation oracle); barrier count ≤ the hand-rolled baseline; the queue dimension round-trips (a single-queue build is identical to today; a synthetic two-queue build emits a valid ownership transfer); validation + sync-validation clean on RTX 3060.

**Effort.** L. **Conflicts.** It unifies the two barrier codepaths C1 keeps distinct — the unification is the goal, but P3a/P3b ship first (the graph emits into their primitives), so the distinction holds until F-GRAPH lands and then collapses. 0%-gate neutral (setup-time build, cached replay). **Class: RENDER / CPU-sync (no field).**

---

# X-SPD — Parametric SPD mip-reduce primitive (resolve M4) — SHARED INFRA

**What.** ONE single-dispatch FidelityFX-SPD-style mip-pyramid builder, **parametric over the reduction op + source/dest format**: `min`, `max`, `min/max` (paired), `average` (bloom), `weighted-bilateral` (ReBLUR). One implementation, one **odd-dimension boundary fix**, consumed by every phase that needs a pyramid.

**Why our path needs it (M4).** Five phases independently claim to "share an SPD mip pattern" (P11 Hi-Z, S-HIZ, S-GTAO prefilter, E-BLOOM, G-REBLUR) but need *different* reductions and formats. "Share the pattern" without a defined interface means each reimplements SPD with its own bugs — and the draft itself flags the odd-dimension boundary as a known pitfall that would otherwise ship **5 times**. A single parametric primitive with an explicit `reduction-op + format` interface makes the others *consume* it, not re-derive it.

**How.** A single-dispatch mip-build (atomic counter for the last-tile reduction; optional subgroup for the in-tile reduction with a **scalar fallback**, capability-gated, differential-golden'd per the CPU-D3 house template). The reduction op is a monomorphized generic parameter (no `dyn`, no branch in the inner loop — specialized per consumer at compile/SPIR-V-permutation time). The odd-dimension boundary fix is written and tested **once**.

**Expected win.** One amortized pyramid build per consumer; the boundary bug fixed once. Measured by GPU-timestamp per consumer + the count of distinct SPD implementations (target: 1).

**Dependency/order.** Needs P1 (storage images + compute). Prereq for S-HIZ, P11, S-GTAO, E-BLOOM, G-REBLUR. Parallel-after-P1.

**Gate.** Each reduction op matches a CPU reference pyramid bit-for-bit (min/max are order-independent → exact; average/bilateral within the consumer's documented tolerance); the odd-dimension case is golden-tested at non-power-of-two extents; the subgroup in-tile path matches the scalar fallback bit-for-bit (differential); validation clean.

**Effort.** S. **Conflicts.** None — it removes duplication. **Class: RENDER (no field).**

---

# X-REF — Stochastic acceptance oracle (resolve C4) — TRACK-G PREREQUISITE ★

**What.** The concrete statistical acceptance protocol every stochastic phase is gated by — without it none of Track G can be golden-gated (which would violate the inviolable GPU-oracle rule). Four artifacts:
1. **An in-house offline path-traced reference generator** — a separate host/compute tool (NOT in the frame hot path) that renders a scene to N-thousand spp converged ground truth, casting rays against the SAME `sdf_field.hlsli` field (so the reference is field-consistent). It may run G-PT to convergence or a dedicated CPU/compute reference path.
2. **A fixed convergence frame budget** — each stochastic phase declares "converges within K frames at a fixed seed under fixed camera"; the test accumulates K frames then compares.
3. **A named metric + threshold per phase** — relative-MSE, FLIP, and SSIM are the three metrics; each phase declares its bar (e.g. ReSTIR DI: relative-MSE ≤ X vs the reference at K frames; a denoiser: SSIM ≥ Y).
4. **An owner statistical-bar sign-off** — because thread scheduling on atomics + cross-workgroup fp accumulation order make even fixed-seed output **not bit-reproducible run-to-run**, each stochastic phase is accepted under a documented "inherently non-deterministic, accepted under statistical bar X" owner approval (Open-Q9 — the neural tier needs its own bar even with seed control).

**Why our path needs it (C4).** "Relaxed-converged-reference at a relaxed tolerance" is a wish, not a test. The project's measurement oracle is golden-buffer diff + validation; ~15 stochastic phases (G-RDI/RGI/PT, G-REGIR, G-SOIT, G-NRC/NDENOISE, A14) have no defined "how many frames, what metric, what threshold, against what reference". X-REF defines them ONCE as a shared deliverable Track G depends on, instead of per-phase hand-waving.

**How.** The reference generator is host-side Rust + a compute path tracer (G-PT run headless to convergence is the natural implementation, so X-REF and G-PT share the ray core — X-REF can ship a CPU reference first, then upgrade to GPU-PT once G-PT exists). The metric library (rMSE/FLIP/SSIM) is in-house Rust over the readback buffer. The fixed-seed harness reuses the existing golden-test infra with a K-frame accumulation loop.

**Expected win.** Makes every Track-G phase falsifiable. No GPU-timestamp win — it is a correctness-oracle prerequisite. Without it, no Track-G phase can be declared done.

**Dependency/order.** Needs the field gateway `sdf_field.hlsli` + a ray primitive (G-BRICKRT or P14) for the GPU reference (or a CPU reference to bootstrap). **Hard prerequisite for ALL stochastic Track-G phases.**

**Gate.** The reference generator reproduces a known analytic case (e.g. a Cornell-box-equivalent with a single SDF emitter) within the literature's converged values; the metric library matches a reference rMSE/SSIM implementation on a known image pair; the K-frame harness is deterministic in its *comparison* (same seed + same K → same metric value within the documented fp-accumulation tolerance).

**Effort.** M. **Conflicts.** The reference itself is in-house code that must exist before Track G — a real sequencing cost, surfaced honestly. **Class: RENDER / tooling (no field in the hot path; the reference probes the frozen field through `sdf_field.hlsli`).**

---

# TRACK B — MARCHER ACCELERATION (cheapen the field we already evaluate) — SDF-NATIVE FAST-TRACK

> The highest-ROI track (M1): every win reduces the cost of the FROZEN field eval directly, with zero new Vulkan features and no HW-RT. B1/B5 are FIELD-CONSUMER (they change *where/how often* the frozen field is sampled, never the field). B7 is FROZEN-preserving (it prunes the analytic fold without altering its result). Land immediately after the spine.

### PHASE B1 — Over-relaxation sphere-tracing (ω-gated) — needs P4

**What.** Accelerate the sphere-trace by stepping `ω·d` (ω ∈ (1,2)) instead of `d`, with a conservative fall-back step when an over-relaxed step overshoots (the returned distance at the new point is smaller than the previous step minus the radius). Keiser/Bálint over-relaxation: fewer iterations to converge on smooth fields.

**Why our path needs it.** The marcher does `MAX_IT=128` conservative `d`-steps; over-relaxation cuts iteration count on the empty-space prefix and along grazing rays — directly fewer FROZEN field evals per pixel, the cheapest possible marcher win. Pure step-logic around an unchanged `field_distance`.

**How.** Step-loop logic in the marcher; the field probe is `field_distance(p)` through `sdf_field.hlsli`, byte-identical. ω is a uniform (Open-Q for the default). The overshoot-detect is a comparison, not a field change.

**Expected win.** Fewer iterations → fewer field evals; GPU-timestamp on the P0 sparse scene. ω=1 reproduces the frozen path exactly (the regression anchor).

**Dependency/order.** Needs P4 (the `sdf_field.hlsli` invariant). First on the fast-track.

**Gate.** Golden-image-equal (±2/255) at ω=1 (must be the frozen path); at ω>1 within the relaxed marcher tolerance (the *hit point* converges to the same surface — the field is unchanged, only the step schedule); the overshoot fallback is conservative (no missed-surface holes — the golden catches them); GPU-timestamp shows iteration reduction; validation clean.

**Effort.** S. **Conflicts.** Overshoot mis-detection is the soundness surface (golden is the oracle). **Class: FIELD-CONSUMER (steps around the frozen `field_distance`; never alters it).**

---

### PHASE B5 — Mesh-depth + previous-frame march seeding — needs P1+P6

**What.** Seed the fine march's start `t` from (a) the G-buffer mesh depth (the SDF need not march past an opaque mesh — P4 already clamps the far bound; this tightens the *near* start using last frame's converged hit) and (b) the previous frame's per-pixel hit distance reprojected via P6 motion vectors. A reprojected hit gives a near-exact start `t` for a coherent next frame.

**Why our path needs it.** Temporal coherence is free convergence: a pixel that hit at `t=5.2` last frame starts there this frame instead of `t=near`. Combined with P4's empty-skip and B1's over-relaxation, the steady-state march is a handful of iterations. Reuses P1's depth image + P6's motion vectors — no new resources.

**How.** Read the reprojected previous hit-`t` (a history channel, bounded, ping-pong like P6 color); clamp to P4's `[near_t, far_t]`; fall back to `near_t` on disocclusion/rejection (P6's existing rejection mask). Field probe unchanged.

**Expected win.** Near-converged start `t` on coherent pixels → minimal iterations in steady state; GPU-timestamp on a panning P0 sparse scene (the coherent case) vs a teleport (the disocclusion worst case).

**Dependency/order.** Needs P1 (depth) + P6 (motion vectors + history + P6-S) + P4 (the `[near_t, far_t]` clamp). On the fast-track after B1.

**Gate.** Golden-image-equal within the marcher tolerance (a correct seed converges to the same surface — the field is unchanged); a disocclusion test shows correct fallback (no stale-seed holes); GPU-timestamp shows steady-state iteration reduction; the seed-history channel is bounded; validation clean.

**Effort.** S. **Conflicts.** A stale seed past a disocclusion is the surface (P6's rejection mask + clamp guard it). **Class: FIELD-CONSUMER (seeds the march around the frozen field).**

---

### PHASE B7 — Lipschitz / bound pruning of the edit-list fold (analytic, distance-exact) — needs P4

**What.** Prune the `O(edits)` fold: maintain a coarse spatial bound (a low-res grid or a per-edit AABB+Lipschitz bound) so a given `field_distance(p)` evaluates only edits whose bound can possibly be the minimum at `p`, skipping edits provably farther than the current running minimum. Because SDFs are 1-Lipschitz, a conservative bound is exact — the pruned fold returns the **identical** distance, not an approximation.

**Why our path needs it.** The analytic fold is `O(edits)` per eval at the P0 256–4096-edit regime — the dominant cost B1/B5 cannot touch (they cut *eval count*, B7 cuts *cost per eval*). Critically, **B7 attacks the same `O(edits)` wall as the P9 brick atlas but stays distance-EXACT** (no R16 quantization, no render↔physics divergence) and keeps the analytic path as the physics source. B7 may push the P9 threshold much higher (Open-Q3 / the v1 P9 note) — evaluate B7 before committing to P9.

**How.** A conservative spatial acceleration over the edit list (a coarse grid the CPU builds at edit-feed time, or per-edit bounds) consulted inside `field_distance` to skip edits whose lower-bound distance exceeds the running min. The skip is provably exact (1-Lipschitz) → the returned distance is byte-identical to the full fold. **This is FROZEN-preserving: the field eval's RESULT is unchanged, only the unevaluated-edit early-out changes the instruction count — but the contract is about the *value*, and the value is bit-identical.** (Verify: the running-min comparison order must be pinned so the fold's float accumulation is unchanged — a pruned edit contributes nothing to the min, so the surviving fold order is a subsequence of the original; pin it.)

**Expected win.** Sub-`O(edits)` fold on spatially-distributed edits; GPU-timestamp on the P0 256–4096-edit scene. Distance-exact (the golden is bit-exact, not relaxed — the standout property vs P9).

**Dependency/order.** Needs P4 (the `sdf_field.hlsli` fold to prune). On the fast-track; **re-evaluate the P9 threshold after B7 ships** (Open-Q3).

**Gate.** Golden-image-equal **bit-exact** vs the un-pruned analytic fold (the pruning is provably exact — any diff is a bound bug; this is the gate that distinguishes B7 from lossy P9); the running-min fold order is pinned (diff-verified the float accumulation is unchanged → the physics-reused field stays byte-identical); GPU-timestamp shows fold-cost reduction scaling with edit spatial distribution; validation clean.

**Effort.** M. **Conflicts.** The fold-order pinning is the determinism surface — a re-ordered min breaks the physics contract; pin it and diff-verify. **Class: FROZEN-preserving (the field VALUE is byte-identical; only unevaluated-edit early-out changes — physics-safe).**

---

# TRACK A — SDF-NATIVE LIGHTING (cone-trace the field we already evaluate)

> The owner-named differentiator (M1). A1/A2/C-AA land on the fast-track (the "SDF-native lighting MVP", §3). A-GI capstones (A7/A8/A10/A12/A13) land later as the software-first GI path (Open-Q6). All are FIELD-CONSUMER except the screen-space A8/A13 (RENDER): they call the FROZEN field at offset points; the accumulation is consumer-side.

### PHASE A1 — SDF cone-trace soft shadows (Quilez penumbra) — needs P4

**What.** For each light, march a shadow ray from the surface toward the light through `field_distance`; the closest-pass ratio `k·d/t` gives a free analytic penumbra (Quilez soft shadows) — no shadow map, no extra geometry. Layered into the per-light shadow factor.

**Why our path needs it.** The canonical SDF-native win: triangle engines build separate shadow maps / DF generators; **we already evaluate the field, so a shadow ray is just more `field_distance` calls along a direction** — contact-hardening soft shadows for free on the SDF half. The shared shadow-march include other phases (S-CONTACT combine, E-SDFVOL) reuse.

**How.** A shadow-march loop in the deferred-lighting pass calling `field_distance` through `sdf_field.hlsli` (unchanged); the penumbra accumulation (min-tracking `k·d/t`) is consumer-side math. Far-bound by S-HIZ when it lands. P9 makes each step O(1).

**Expected win.** Contact-hardening soft shadows on the SDF; GPU-timestamp (cost = shadow-march steps × lights). On the analytic path the cost is O(lights × steps × edits) — **bounded by P4's empty-skip + B7's pruning; P9 makes it O(1)/step** (M2 cost discipline applies, but A1 is per-surface-pixel not per-froxel, so it is feasible on the analytic field at the P0 regime with B7, unlike E-SDFVOL's per-froxel cost).

**Dependency/order.** Needs P4 (the `sdf_field.hlsli` shadow march); pairs with A2/C-AA on the fast-track; benefits from B7/P9 (cheaper steps) + S-HIZ (far-bound). Part of the SDF-native MVP.

**Gate. CRITICAL boundary:** the shadow march CALLS the FROZEN field through `sdf_field.hlsli` (no fast-math inside `field_distance` — physics reuses it); the penumbra accumulation (min-tracking, `k`, `d/t`) is consumer-side and MAY relax; golden within owner tolerance on a known light/scene; the un-shadowed→shadowed flag is additive (the no-shadow path stays the rung golden); validation clean; GPU-timestamp.

**Effort.** M. **Conflicts.** None. **Class: FIELD-CONSUMER (frozen field probe; consumer-side penumbra).**

---

### PHASE A2 — SDF 5-tap ambient occlusion — needs P4

**What.** Sample `field_distance` at a few steps along the surface normal; the deficit between expected and actual distance gives cheap analytic AO (the classic 5-tap SDF AO), darkening crevices.

**Why our path needs it.** Same SDF-native logic as A1 for ambient: free crevice AO from the field we already evaluate, no GTAO depth-sweep needed for the SDF half (S-GTAO complements for the *mesh* half / SDF↔mesh screen-space, A2 is the SDF-intrinsic AO).

**How.** A 5-tap loop along the normal calling `field_distance` (unchanged); the AO accumulation is consumer-side. Cheapest A-track phase.

**Expected win.** Crevice AO on the SDF; GPU-timestamp (5 field evals/pixel, bounded by B7/P9 on the analytic path).

**Dependency/order.** Needs P4; pairs with A1/C-AA. Part of the SDF-native MVP.

**Gate.** Frozen field probe via `sdf_field.hlsli`; consumer-side AO accumulation relaxable; golden within owner tolerance; additive (no-AO path = rung golden); validation clean; GPU-timestamp.

**Effort.** S. **Conflicts.** None. **Class: FIELD-CONSUMER.**

---

### PHASE A-GI capstones — Software-first SDF global illumination (Open-Q6)

> The core v2 verdict: software/SDF-native GI is the PRIMARY path; HW-RT (P14/P15) is an optional accelerator, not the GI prerequisite. Ships A8/A13 on P1+P6 (no bricks) and A7/A10/A12 over bricks (no HW-RT).

- **A8 — Radiance Cascades 2D (noiseless, no bricks).** Hierarchical radiance probes with angular/spatial cascade trade-off; noiseless penumbra-correct GI in 2D/2.5D over the P1 G-buffer + P6 reprojection. **L. RENDER** (screen-space; no field probe). Needs P1+P6.
- **A13 — SSGI bitmask.** Bitmask horizon-based screen-space GI over the depth/normal G-buffer, reusing S-HIZ for long-range sampling. **M. RENDER.** Needs P1+P6+S-HIZ.
- **A7 — Voxel cone tracing over bricks.** Cone-trace the P9 brick atlas for one-bounce diffuse GI. **L. FIELD-CONSUMER** (brick fetch via the forked backend). Needs P9.
- **A10 — DDGI (irradiance probe volume) over the field.** A probe grid sampling `field_distance`/`field_radiance` via G-BRICKRT/cone-trace; temporal probe update. **L. FIELD-CONSUMER.** Needs P9 (+G-BRICKRT for probe rays).
- **A12 — Brixelizer-class SDF GI cascades.** AMD-Brixelizer-style cascaded SDF GI over the P9/P10 brick clipmap. **XL. FIELD-CONSUMER.** Needs P9+P10.

**Determinism (all A-GI).** Screen-space (A8/A13) = RENDER. Brick/field-probing (A7/A10/A12) = FIELD-CONSUMER: the field/brick probe goes through `sdf_field.hlsli`/the forked backend; the irradiance/cone accumulation is consumer-side and relaxable. None enter `boyko_sdf_math`. Stochastic variants (if any) gated by X-REF.

---

# TRACK C — RECONSTRUCTION / AA / VRS

> C-AA lands on the SDF-native fast-track (analytic edge AA from the distance field — FIELD-CONSUMER). C-TSR supersedes the simple P5/P6 upscale/TAA (Open-Q4). C-CTSS/C-VRS/C-CBR are capability-gated shading-rate reductions (RENDER).

### PHASE C-AA — Analytic edge AA from the distance field — needs P4

**What.** Use the distance field's gradient at the silhouette to compute analytic coverage (the pixel's signed distance to the edge → a smooth coverage term), antialiasing SDF edges with no MSAA and no temporal cost — a free byproduct of the field we evaluate.

**Why our path needs it.** SDF edges are analytically defined; the field already gives the distance-to-surface, so edge coverage is a near-free consumer of `field_distance` — crisper edges than MSAA on the SDF half, no extra samples. Part of the SDF-native MVP.

**How.** Compute coverage from `field_distance` and the screen-space gradient (`fwidth`-style) in the marcher/composite; the coverage blend is consumer-side. Field probe unchanged.

**Expected win.** Smooth SDF edges at ~zero cost; GPU-timestamp negligible.

**Dependency/order.** Needs P4. Part of the SDF-native MVP (A1/A2/C-AA + B1/B5).

**Gate.** Frozen field probe; consumer-side coverage relaxable; golden within owner tolerance (edge pixels change by design — an owner-approved AA bar); validation clean.

**Effort.** S. **Conflicts.** None. **Class: FIELD-CONSUMER.**

---

### PHASE C-TSR — Full FSR2/TSR temporal super-resolution (supersedes simple P5/P6) — needs P5+P6

**What.** The full FSR2/TSR recipe (reactive mask, depth/normal/luma history clamping, lock/disocclusion handling, robust contrast-adaptive sharpening) replacing P5's simple bilateral upscale + P6's basic TAA resolve. The quality upgrade once the simple forked proof (P5/P6) ships.

**Why our path needs it.** P5/P6 ship first as the minimal forked proof (§3); C-TSR is the production-quality reconstruction (Open-Q4 confirms the two-stage approach). Reuses the P5 half-res input + P1 G-buffer + P6 motion vectors/history/P6-S — the simple path is the fallback and the differential reference.

**How.** Raw-Vulkan compute, reimplemented (FSR2 algorithm ported, never linked). Consumes the P6-S seam. The simple P5/P6 path remains as the relaxed-tolerance reference.

**Expected win.** Higher reconstruction quality at the half-res cost; GPU-timestamp + SSIM vs the P5/P6 simple path and a native-res reference.

**Dependency/order.** Needs P5 + P6 + P6-S. Supersedes (does not delete) the simple path.

**Gate.** Render-side; relaxable but reproducible (deterministic given inputs + jitter sequence); temporal by design (outside the physics gate); owner-approved relaxed SSIM/PSNR bar (the P5/P6 precedent extends); a DLSS-class quality reference (the native-res ground truth at matched frames) is the upper bar; validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** None (the simple path is the fallback). **Class: RENDER (no field).**

### PHASE C-CTSS / C-VRS / C-CBR — Checkerboard / variable-rate / coarse shading — CAPABILITY-GATED

Checkerboard rendering (C-CBR), hardware VRS (C-VRS, `VK_KHR_fragment_shading_rate` + a compute-raster fallback), and coarse-tile shading (C-CTSS) reduce shaded samples. **M each. RENDER.** Capability-gated (VRS) with a full-rate fallback. Needs P1+P6.

---

# TRACK D — GPU-DRIVEN GEOMETRY + TRANSPARENT SUBSTRATE

> D-FWD is the transparent/blended-material substrate the critique (C2) proved all OIT + S-CSM + E-PART rendering need but the spine does not build. D-VIS is the 64-bit visibility-buffer alternative to the P1 depth-copy seam (Open-Q5). D-MESH/D-CULL/D-SWRAST/D-COMPACT are the GPU-driven mesh pipeline.

### PHASE D-FWD — Transparent / blended material substrate (resolve C2) — needs P1+P7

**What.** The missing forward/transparent material path: a blend-state raster pipeline (the RHI has only the COMBINED_IMAGE_SAMPLER prototype, no blend-state pipeline beyond it), a sorted transparent pass, and float-RT additive-blend capability — the substrate E-WBOIT/MBOIT/LLOIT, S-CSM's transparent casters, and E-PART's blended particles all require. **Resolves the C2 false-prerequisite:** P1 graduates the SDF *compute marcher* into MRT storage images; it does NOT build a blend-capable transparent raster pass.

**Why our path needs it (C2).** Three OIT phases + E-PART + S-CSM say "needs P1" but P1 delivers no blend-state pipeline, no transparent material path, no float-RT additive blend. Without D-FWD a reader schedules E-WBOIT "after P1" and discovers a hidden XL dependency. D-FWD makes the substrate explicit and shared.

**How.** Extend the RHI pipeline-create with blend-state descriptors (additive/over, src/dst factors); a transparent-pass slot in the frame (lowered by F-GRAPH); float-format color attachments. The forward path reads P7's light list (so transparent surfaces are lit). Mesh-material-G-buffer scope is an owner decision (Open-Q11 — whether the mesh gets a real material G-buffer or stays flat-color gates S-CSM/S-GTAO mesh occlusion/E-PART albedo).

**Expected win.** Enables transparency at all (a capability, not a speedup). GPU-timestamp of the transparent pass.

**Dependency/order.** Needs P1 (G-buffer + depth) + P7 (light list for lit transparency) + F-GRAPH (barrier lowering). **Prereq for E-WBOIT/MBOIT/LLOIT, E-PART rendering, S-CSM transparent casters.**

**Gate.** A blended transparent quad over the deferred opaque produces the host-reference composite; additive blend is commutative (order-independent for the OIT consumers); validation/sync-validation clean (the new pipeline state + transparent-pass barriers are F-GRAPH-derived); golden within ±2/255 on an opaque+transparent scene.

**Effort.** M. **Conflicts.** None; additive pass under `boyko_render`. **Class: RENDER (no field).**

### PHASE D-VIS / D-SWRAST / D-MESH / D-CULL / D-COMPACT — GPU-driven mesh pipeline

- **D-VIS — 64-bit visibility buffer.** Mesh + SDF write `(depth<<32)|id` via core `shaderBufferInt64Atomics` `atomicMin`; correct mesh↔SDF occlusion with no MRT bandwidth — the alternative to P1's depth-copy seam (Open-Q5: P1 MRT first; D-VIS when Track-D lands, resolving the depth-seam overlap then). **L. RENDER.** Capability: `shaderBufferInt64Atomics` (core 1.2, guaranteed on RTX 3060) + fallback.
- **D-SWRAST — compute software raster** for tiny triangles (Nanite-style). **L. RENDER.** Capability-gated (int64-atomic) + HW-raster fallback.
- **D-MESH — mesh/task shaders.** **M. RENDER.** Capability `VK_EXT_mesh_shader` + compute-cull fallback.
- **D-CULL — GPU frustum/cluster cull** (indirect, P8). **M. RENDER.**
- **D-COMPACT — persistent-thread stream compaction** (shared with G-BRICKRT traversal). **M. RENDER.**

All D-* lowered by F-GRAPH; capability-gated with named in-house fallbacks; RENDER class (no field).

---

# TRACK E — VOLUMETRICS / POST / OIT / PARTICLES

> Largely a v1 gap. E-FOG is the unifying volumetric primitive; E-SDFVOL/E-DENS are the SDF-native participating-medium wins. **The post stack (E-BLOOM/DOF/MBLUR/TONE/EXP/LUT) is explicitly POLISH-TIER (M1)** — commodity any engine has, sequenced last, never competing with the differentiators. OIT spans a cost/quality ladder. All RENDER except E-SDFVOL/E-DENS-B/E-PART-collision (FIELD-CONSUMER). All passes lowered by F-GRAPH.

### PHASE E-FOG — Froxel volumetric fog/lighting — needs P1+P6-S+P7

**What.** A frustum-aligned 3D texture (160×90×64) filled by a compute scatter pass (per-froxel in-scattered RGB + extinction, reusing P7's clustered light list — **the froxel XY×Z IS the cluster grid, so fog and opaque lighting share one cull; this requires the fog grid dimensions to match P7's froxel grid, an explicit constraint**) then a front-to-back integrate pass; the final pass reconstructs froxel-Z from G-buffer depth. Exponential depth slicing + P6 temporal reprojection.

**Why our path needs it.** Pure compute over storage-3D images (P1's vocabulary); reads P6 depth/history (needs **P6-S** for the cross-frame reprojection, M3); consumes P7's per-cluster bitfield directly. The unifying primitive E-SDFVOL/E-DENS/E-CLOUD/E-GOD build on.

**How.** 3D STORAGE/SAMPLED images + two compute dispatches + trilinear sampler + HG phase. Lowered by F-GRAPH. Subgroup optional for the scatter reduction (X-SPD-style, scalar fallback).

**Expected win.** Volumetric atmosphere/local fog; GPU-timestamp.

**Dependency/order.** Needs P1 + **P6-S** (cross-frame reproject) + P7 (light list; froxel grid must match). Prereq for E-SDFVOL/E-DENS/E-CLOUD/E-GOD.

**Gate.** Consumer-side; never touches the field; HG/exp-transmittance/jitter relaxable; temporal reproject non-deterministic by design (outside the physics gate); golden under an owner-approved relaxed PSNR/SSIM bar; the fog-grid==froxel-grid constraint verified; validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** None. **Class: RENDER (no field).**

---

### PHASE E-SDFVOL — SDF-native analytic volumetric self-shadow & AO — needs P4 (P9-REQUIRED at scale, resolve M2)

**What.** Reuse `field_distance` for volumetric soft shadows + AO: a shadow ray's closest-pass gives free penumbra (Quilez), cone steps along the normal give hemispheric AO; applied to surface shading AND the E-FOG scatter pass (self-shadowed fog). **M1 fix: the surface-shading half is A1/A2 (no fog pipeline needed); E-SDFVOL adds the froxel/fog application on top of A1's shared shadow-march include — the differentiator is NOT gated behind the commodity E-FOG for surface shading.**

**Why our path needs it.** The SDF-native win triangle engines cannot match: **the froxel scatter pass calls `field_distance` toward each light for volumetric shadows on fog** — impossible cheaply in a triangle pipeline. Shares A1's shadow-march include.

**How.** Added shader math in the marcher / the E-FOG scatter pass; zero new resources. Field probe via `sdf_field.hlsli`.

**Expected win.** Self-shadowed fog + god rays (with E-GOD route a); GPU-timestamp.

**Cost discipline (M2 — "free"/"a few cone steps" replaced).** The froxel grid is 160×90×64 ≈ 920K froxels; `field_distance` toward each light × cone-steps × O(edits) on the analytic path is potentially **billions of FROZEN evals/frame** (no rsqrt/FMA → full IEEE cost). **E-SDFVOL's fog application is therefore P9-REQUIRED above the ≤16-edit fixture** (O(1) brick fetch with empty-brick skip). On the analytic path it is restricted to the low-edit regime; above it, P9 is a hard prerequisite, not "ideally P9". The surface-shading half (A1/A2, per-pixel not per-froxel) is feasible analytically with B7.

**Dependency/order.** Surface half: A1/A2 (P4). Fog half: E-FOG + **P9 (required at the P0 256–4096-edit regime)**.

**Gate. CRITICAL boundary:** the shadow/AO cone-trace CALLS the FROZEN field through `sdf_field.hlsli` (no fast-math inside `field_distance` — physics reuses it); the penumbra/AO accumulation is consumer-side and may relax; golden within owner tolerance; the per-froxel cost is bounded by P9's empty-brick skip (timestamp proves it); validation clean; GPU-timestamp.

**Effort.** M. **Conflicts.** Analytic-path cost (P9-gated at scale). **Class: FIELD-CONSUMER (frozen field probe; consumer-side accumulation).**

---

### PHASE E-DENS-A — Independent render-authored density channel (resolve C3) — THRESHOLD-GATED

**What.** A density channel for fog/clouds authored as **independent render data** (a separate 3D image or a second R16 brick-atlas channel) with **no coupling to the field eval** — local fog volumes as `SdfEdit`-like ECS components, sampled by E-FOG. Brick-accelerated empty-space skip.

**Why our path needs it.** The brick-map's empty-space skipping accelerates fog marching too — one acceleration structure, two consumers. As an *independent* channel it is cleanly RENDER (the C3 split: this half never touches `field_distance`).

**How.** A density channel in the 3D brick texture (or a separate D3 image); raw-Vulkan 3D sampling; no field coupling. Lowered by F-GRAPH.

**Expected win.** Brick-accelerated participating media; GPU-timestamp.

**Dependency/order.** Needs P9 (the brick structure) + E-FOG (the consumer) + P4 (empty-brick skip).

**Gate.** Render-side only; density is independent render data (NO field-eval contact — the clean half of the C3 split); brick R16 quantization is an owner-approved P9-class tolerance; validation clean; GPU-timestamp.

**Effort.** L (shared with E-DENS-B). **Conflicts.** None. **Class: RENDER (no field).**

---

### PHASE E-DENS-B — Field-derived density (the field probe stays FROZEN) (resolve C3) — THRESHOLD-GATED

**What.** Density derived from the SDF: near-surface distance maps to extinction, so volumetric density is authored by the field itself. **The C3 split: the `field_distance(p)` call is FROZEN (through `sdf_field.hlsli`, no fast-math); the distance→extinction remap is a pure consumer transform on the already-evaluated scalar `d` (fast-math OK on the remap, NEVER inside the probe).**

**Why our path needs it.** Field-derived fog density is a true SDF-native differentiator (no triangle engine has it), but it must not leak fast-math into the field. The remap `extinction = remap(d)` operates on the scalar output, never re-entering or re-implementing the field eval.

**How.** `let d = field_distance(p);` (FROZEN) then `extinction = remap(d)` (consumer transform). If a density channel is *baked* from the field at bake time, the bake uses the FROZEN field (no fast-math) and the bricks are golden-compared on the render side only — the analytic field stays the physics source.

**Expected win.** Field-native participating media; GPU-timestamp.

**Dependency/order.** Needs P9/E-FOG/P4 (as E-DENS-A) + the FROZEN field probe.

**Gate.** The `field_distance` probe is byte-identical FROZEN (diff-verified, physics-safe — the explicit C3 fix replacing the blanket "fast-math OK"); the extinction remap of the already-evaluated scalar may relax; if baked, the bake path uses the FROZEN field; render-side golden at owner tolerance; validation clean; GPU-timestamp.

**Effort.** L (shared with E-DENS-A). **Conflicts.** The remap-vs-probe boundary is the determinism surface (the C3 trap) — enforced by keeping the probe a frozen `sdf_field.hlsli` call. **Class: FIELD-CONSUMER (frozen probe; consumer-side remap).**

---

### PHASE E-CLOUD — Volumetric clouds (Perlin-Worley raymarch) — THRESHOLD-GATED

**What.** Raymarch a cloud layer (low-freq Perlin-Worley base eroded by high-freq detail, weather map + height gradients; Beer-Lambert + HG + powder; adaptive step + cone light steps). Temporal upscaling/reprojection makes it affordable.

**Why our path needs it.** A forked compute marcher analogous to our SDF marcher; reuses P5 half-res + P6 jitter/TAA (clouds are THE canonical temporal-upscaling use). 3D noise textures generated once at startup by an in-house compute pass (own noise gen satisfies the in-house constraint).

**How.** In-house compute-generated 3D noise + one raymarch compute pass + blue-noise jitter. No RT/mesh-shader. The most ALU-heavy E technique. Lowered by F-GRAPH.

**Expected win.** Volumetric clouds; GPU-timestamp.

**Dependency/order.** Needs E-FOG (shared scatter/HG) + P5 (half-res) + **P6-S** (TAA reprojection — required to be affordable) + P7 (in-scatter).

**Gate.** Fully consumer-side; no field coupling; HG/Beer/noise/reproject relaxable; non-deterministic via temporal upscaling (outside the physics gate); golden under a relaxed temporal bar; validation clean; GPU-timestamp.

**Effort.** XL. **Conflicts.** ALU-heavy. **Class: RENDER (no field).**

---

### PHASE E-GOD — God rays / light shafts — needs E-FOG (route a)

**What.** **Route (a) — the SDF-native win:** FREE as a byproduct of E-SDFVOL once the sun is shadow-sampled per froxel (physically-correct god rays from fog self-shadowing). **Route (b) — DEMOTED to optional-only (m1):** a screen-space bright-pass + radial blur, a low-end fallback exploiting nothing SDF-native — built only if a no-volumetrics path is needed, not a default phase slot.

**Why our path needs it.** Route (a) needs zero new code beyond E-SDFVOL's per-froxel sun-shadow sampling — the differentiator. Route (b) is commodity filler (m1).

**How.** Route (a): part of the fog dispatch. Route (b): two fullscreen compute passes (optional).

**Expected win.** Light shafts; GPU-timestamp negligible (route a).

**Dependency/order.** Route (a): E-FOG + E-SDFVOL. Route (b, optional): P1 composite.

**Gate.** Pure render-side; route (a) inherits the fog tolerance; validation clean.

**Effort.** S. **Conflicts.** None. **Class: RENDER (no field).**

---

### POLISH-TIER POST — E-BLOOM / E-DOF / E-MBLUR / E-EXP / E-TONE / E-LUT (M1: low-priority commodity)

> Any-engine post commodity, sequenced last, never competing with the differentiators. All RENDER (no field), all lowered by F-GRAPH, all deterministic frame-to-frame except the intentionally-varying grain (E-LUT) and frame-dependent adaptation (E-EXP).

- **E-BLOOM — progressive dual-filter mip chain** (13-tap Karis down + 9-tap tent up, ~6 mips). **Consumes X-SPD** (the average reduction — M4). **M.** Needs P1+X-SPD. Pairs with E-TONE.
- **E-DOF — separable disk / scatter-as-gather** (CoC from depth, complex-phasor separable). **M.** Needs P1.
- **E-MBLUR — tile-based motion blur** (TileMax→NeighborMax→reconstruct, **consuming P6's R16G16 motion vectors** — nearly free once P6 exists; TileMax/NeighborMax via X-SPD). **M.** Needs P6.
- **E-EXP — auto-exposure histogram** (256-bin log-luma shared-memory atomics → reduce → eye-adapt). Cross-frame exposure read needs **P6-S** (M3). **M.** Needs P1+P6-S. Feeds E-TONE.
- **E-TONE — ACES/AgX tonemap.** **S.** Needs E-EXP+E-BLOOM+P1 composite.
- **E-LUT — 3D-LUT grade + grain/CA/vignette.** **S.** Needs E-TONE. Grain intentionally per-frame (non-deterministic by design); LUT/vignette/CA deterministic.

---

### PHASE E-WBOIT — Weighted-blended OIT (single-pass, deterministic) — needs D-FWD

**What.** OIT in ONE additive pass: `sum(color·alpha·w)` + `sum(alpha·w)` into one RT + `product(1-alpha)` revealage into another; resolve by dividing. No sorting, no per-pixel lists.

**Why our path needs it.** Cheapest OIT, the first transparency path. Two small float RTs + additive blend — **within D-FWD's blend capability (C2: not "needs P1" — needs D-FWD's blend-state pipeline + float-RT additive blend)**. No atomics, no unbounded memory, deterministic.

**How.** Dual-RT additive blending in one pass over D-FWD's substrate. Lowered by F-GRAPH.

**Expected win.** Approximate transparency; GPU-timestamp.

**Dependency/order.** Needs **D-FWD** (MRT float blend + transparent-material path) — the C2 restated prerequisite.

**Gate.** Consumer-side + DETERMINISTIC (additive blend is order-independent); weight math relaxable; no field; tight golden tolerance; validation clean.

**Effort.** S. **Conflicts.** None. **Class: RENDER (deterministic; no field).**

---

### PHASE E-MBOIT — Moment-Based OIT (filterable, bounded memory) — needs D-FWD

**What.** Approximate per-pixel transmittance with 4–8 power moments (or up to 4 trigonometric) of log-transmittance vs depth; reconstruct + composite. Bounded memory (~10–18 B/px), no sorting, filterable.

**Why our path needs it.** Best quality/memory balance for heavy transparency without unbounded buffers — fits our bounded-memory principle far better than linked lists. Filterability pairs with P5 half-res.

**How.** Additive float-RT blending (D-FWD) + a reconstruction pass. Trigonometric (3 moments) beat 8 power moments. Lowered by F-GRAPH.

**Expected win.** High-quality bounded OIT; GPU-timestamp.

**Dependency/order.** Needs **D-FWD** (multi-RT float + additive blend). Benefits from P5.

**Gate.** Consumer-side; additive moment accumulation is order-independent (deterministic); **moment reconstruction uses a numerically-stable Cholesky with a documented epsilon bias regardless of fast-math — moment OIT is famously ill-conditioned at low moment counts, so the epsilon bias + stable formulation is mandatory, not "watch fast-math" (m2)**; no field; validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** Moment-inversion conditioning (mitigated by the mandated stable formulation + epsilon). **Class: RENDER (deterministic; no field).**

---

### PHASE E-LLOIT — Per-pixel linked-list / A-buffer OIT (exact) — needs D-FWD (atomics)

**What.** Exact OIT: a fragment pass atomically allocates a node in a big **preallocated** storage buffer (`{color, depth, next}`), atomic-exchanges the per-pixel head; a resolve walks each list, sorts the front N (≤16), blends in order, tail-blends the rest.

**Why our path needs it.** The high-quality option; exact for moderate overlap. Needs `fragmentStoresAndAtomics` (our compute path uses atomics) + a **preallocated fixed-size node pool** (fits "preallocate at setup", no hot-path alloc).

**How.** Fragment atomics + storage buffer (D-FWD substrate). **Preallocate + clamp to avoid buffer-overflow UB.** Lowered by F-GRAPH.

**Expected win.** Exact transparency; GPU-timestamp.

**Dependency/order.** Needs **D-FWD** (storage head + node pool + fragment atomics).

**Gate.** Consumer-side; allocation ORDER nondeterministic (atomic races) BUT the resolve SORTS by depth before blending → the final image is deterministic given a stable fragment set; the overflow tail-blend is the only approximation; **buffer-overflow UB guarded by preallocate + clamp** (validation/the clamp is the oracle); no field; golden-stable after sort; validation clean.

**Effort.** L. **Conflicts.** Unbounded memory (preallocated pool + clamp). **Class: RENDER (deterministic after sort; no field).**

---

### PHASE E-PART — GPU particle system (+ free SDF collision) — needs P1+P8+D-FWD (P9-REQUIRED collision at scale, resolve M5)

**What.** Fully GPU-resident particles: a compute pass spawns/updates/integrates in a persistent **preallocated** buffer, maintains an alive-list (dead-list freelist) via atomics, writes a `DispatchIndirect`/`DrawIndirect` arg from the alive count, renders only live particles (transparent particles use an OIT path). **SDF-field collision is a `field_distance` test.**

**Why our path needs it.** Reuses the GPU-column compute-dispatch path (GpuSystem) + the P8 indirect seam + D-FWD for blended-particle rendering; **the SDF field gives analytic particle collision (`field_distance < 0` → collide) — an SDF-native win**; particles inject density into E-FOG. Buffers preallocated at setup.

**How.** Compute sim + storage buffers + atomics + `vkCmdDrawIndirect` (P8 seam) + D-FWD blended render. Lowered by F-GRAPH.

**Cost discipline (M5 — "free" dropped).** N particles × O(edits) FROZEN evals/sim-step: at 1M particles × 256 edits ≈ 256M frozen evals/frame for collision alone (no rsqrt/FMA → full IEEE cost). **"Free" is wrong on the analytic path — particle-SDF collision at scale is P9-REQUIRED (O(1) brick fetch); on the analytic path, cap particle count or collision frequency.**

**Expected win.** GPU-resident particles + SDF collision + fog injection; GPU-timestamp.

**Dependency/order.** Needs P1 (storage + compute) + P8 (indirect alive-count) + **D-FWD** (blended render). Collision uses P4's `field_distance` (**P9 at scale**). Density injection pairs E-FOG.

**Gate.** Sim consumer-side; **if collision calls `field_distance` it uses the FROZEN path (no fast-math inside the field eval)**; integration/forces relaxable; atomic alive-list ordering nondeterministic (acceptable — particles visual-only, outside the physics gate); transparent particles need an OIT path; the collision cost is bounded by P9 at scale (timestamp); validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** Analytic collision cost (P9-gated at scale). **Class: RENDER (sim) + FIELD-CONSUMER (collision via the frozen `field_distance`).**

---

# TRACK S — CLASSIC SHADOW / AO (mesh-side complement + shared accelerator)

> Not redundant with Track A: contact shadows recover fine mesh↔SDF detail the far cone-trace misses; GTAO sees screen-space neighbors/meshes the analytic SDF-AO (A2) cannot; CSM/VSM are the correct directional path for MESHES (which are NOT in the SDF). S-HIZ is the shared accelerator (canonical X-SPD consumer). All RENDER (no field), all lowered by F-GRAPH.

### PHASE S-HIZ — Hi-Z / depth-MIP shared accelerator (canonical X-SPD consumer) — needs P1+X-SPD

**What.** A min/max depth pyramid (built via **X-SPD** — the parametric primitive, NOT a re-derivation; M4) used to (a) accelerate S-GTAO/S-SSILVB long-range horizon sampling, (b) early-skip empty space in S-CONTACT marches, (c) bound the SDF cone-shadow (A1) far-march, (d) the P11 occlusion test.

**Why our path needs it (M4).** v1's P11 builds a Hi-Z for culling, but its role as a SHARED accelerator for AO/contact-shadow/cone-shadow range queries is under-weighted. **GTAO's prefilter MIP and P11's Hi-Z are the same structure — built ONCE via X-SPD, consumed in S-GTAO, S-CONTACT, A1's far-bound, P11.** The cross-cutting infra that makes the classic complement cheap.

**How.** Consume X-SPD with the `min/max` reduction on the depth image. No new SPD code (M4). Lowered by F-GRAPH.

**Expected win.** One pyramid amortized across ≥4 consumers; GPU-timestamp.

**Dependency/order.** Needs P1 + **X-SPD**. Consumed by S-GTAO, S-CONTACT, A1, P11. (Factors P11's Hi-Z build out as the shared primitive.)

**Gate.** Pure depth derivative; no field; min/max reduction order-independent (stable, X-SPD-gated); validation clean; GPU-timestamp.

**Effort.** M. **Conflicts.** Overlaps P11's Hi-Z (factored here as the shared primitive via X-SPD). **Class: RENDER (no field).**

### PHASE S-GTAO / S-CONTACT / S-CSM / S-VSM

- **S-GTAO — ground-truth AO** (XeGTAO-style, horizon-based, multi-slice, bent-normal feeds A6). The **prefilter MIP is X-SPD/S-HIZ** (M4). NO subgroup required (XeGTAO-confirmed). **L. RENDER.** Needs P1+S-HIZ; benefits from P6.
- **S-CONTACT — screen-space contact shadows** (depth-buffer ray-march, 8–16 steps; layers ON TOP of every shadow source by min — recovers mesh-on-mesh / mesh-on-SDF / SDF-on-mesh fine occlusion A1/S-CSM miss). Uses P6 IGN jitter + S-HIZ empty-skip. **M. RENDER.** Needs P1(+P6+S-HIZ).
- **S-CSM — cascaded shadow maps** (the correct directional path for the MESH half — meshes are NOT in the SDF, so cone-tracing them is impossible; CSM casts mesh shadows onto meshes AND the SDF surface; SDF casts onto meshes via A1; combine by min). PCF + receiver-plane bias + texel-snap; front-face-cull. **Needs D-FWD for transparent casters / Open-Q11 for mesh-material scope. L. RENDER.** Needs P1 mesh raster.
- **S-VSM — virtual shadow maps** (16k² sparse paged, clipmap, static/dynamic caching). Page table via core `shaderBufferInt64Atomics` (RTX 3060 guaranteed) + indirect (P8) + P11 page marking; **shares the clipmap concept with P10/A12**; page caching needs **P6-S** (M3). **XL. RENDER.** Capability `shaderBufferInt64Atomics` + CSM fallback. THRESHOLD/CAPABILITY-gated.

---

# THRESHOLD/CAPABILITY/PROFILE-GATED SDF-ACCEL + GPU-DRIVEN + HW-RT

> P9–P15 preserved from v1 (content verbatim where unchanged); v2 adds G-BRICKRT (the no-RT keystone), the FORKED-BACKEND determinism class + the render↔physics divergence contract (M6), and re-sequences HW-RT as an accelerator (Open-Q6).

### PHASE P9 — SDF brick atlas (field backend swap behind the P4 invariant) — THRESHOLD-GATED (re-eval vs B7 first)

**What / Why / How / Gate** — as v1 §P9 (sparse brick-map + `maxImageDimension3D`-sized atlas; `field_distance`→trilinear fetch, `tile_bound`→per-brick AABB; **R16 default**, O2). The marcher/tile-cull/half-res/TAA/clustered-lighting/shared-depth/all-Track-A-lighting/G-BRICKRT are byte-untouched (the P4 invariant). **Determinism (C3/M6):** the brick fetch replaces the analytic eval behind `field_distance` as a **FORKED-BACKEND** selected by a scene flag; the analytic `sdf`/`smin`/`combine` path **remains byte-identical, the FROZEN reference + the physics source** (physics evaluates the analytic field on the CPU, not bricks). The non-empty-brick AABB list is the BLAS source for P14 + the HDDA target for G-BRICKRT (do NOT pick a layout that can't emit AABBs).

**M6 — render↔physics geometric-divergence contract (Open-Q12).** Render visibility/lighting/particle-collision use the BRICK field; physics collision uses the ANALYTIC field. R16 quantization makes them disagree by a bounded world-space error. **A character's feet may visibly float/sink relative to where physics says the ground is.** The contract: document the max world-space geometric divergence (R16 distance quantization × brick extent → an error bound) and get owner sign-off that this visual/physics mismatch is acceptable. **B7 (distance-exact, analytic) has ZERO such divergence — evaluate B7 first (Open-Q3); treat P9 as the world-scale/discrete-field tier B7 cannot reach.**

**Effort.** XL. **Conflicts.** Brick quantization leaves bit-exact on the render side (documented, owner-approved, M6 divergence bound); the analytic reference + physics source preserved. **Class: FORKED-BACKEND (render fetch forked; analytic FROZEN eval preserved as the physics source).**

### PHASE P10 — Geometry clipmap LOD over bricks — THRESHOLD-GATED

As v1 §P10. Cascade selection in `field_distance`/`tile_bound` (behind the invariant). The cascade structure aligns with A12 (Brixelizer cascades), A10 (DDGI volumes), S-VSM (shadow clipmap) — a shared cascade concept. **L. Class: FORKED-BACKEND.** Needs P9.

### PHASE G-BRICKRT — Software brick-RT (hierarchical-DDA over the brick atlas) — RT-CORE-FREE KEYSTONE — needs P9

**What.** Traverse the P9 sparse brick atlas with a hierarchical DDA (leapfrog empty bricks via bitmask/coarse levels; fine-march or analytic-solve only inside occupied bricks) to cast arbitrary secondary/incoherent rays in compute — a software substitute for HW-RT BLAS traversal that runs on ANY GPU.

**Why our path needs it (the keystone — Open-Q6).** **It makes ALL the resampling/PT/cache techniques run WITHOUT RT cores:** ReSTIR shadow/GI rays (G-RDI/RGI), NRC/SHaRC bounces (G-NRC/SHARC), RT-AO all need a secondary-ray primitive, and v1 only provided it at P14 (HW-RT). Software brick-RT makes the brick atlas itself the acceleration structure — the brick AABB list P14 would feed a BLAS is traversed directly in compute. The universal incoherent-ray fallback v1 names but never builds. **This is the architectural inversion: Track G no longer depends on P14.**

**How.** Pure compute; HDDA is well-documented (Laine-Karras ESVO, brickmap, VoxelRT) — reimplemented, no extensions, NO RT cores. Synergistic with D-COMPACT's persistent-thread traversal. Lowered by F-GRAPH.

**Expected win.** Arbitrary secondary rays on any GPU; GPU-timestamp vs HW-RT (P14) on a capable device.

**Dependency/order.** Needs P9 (brick atlas + brick-map) + P4 + `sdf_field.hlsli`. Prereq for the no-RT path of Track G + A-GI brick probing.

**Gate.** The DDA traversal is integer/address logic (deterministic); the in-brick solve calls the FROZEN `sdf`/`smin`/`combine` via `sdf_field.hlsli` unchanged; brick R16 quantization is the only relaxation (owner-approved P9 tolerance + the M6 divergence bound); the analytic CPU field stays the physics source; validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** None (its purpose is RT-core independence). **Class: FIELD-CONSUMER (in-brick solve via the frozen field; DDA is integer logic; physics field untouched).**

### PHASE P11 — Hi-Z two-pass mesh occlusion culling — THRESHOLD-GATED, needs P8+S-HIZ

As v1 §P11, with the Hi-Z build factored to **S-HIZ/X-SPD** (M4 — P11 consumes the shared pyramid, does not rebuild it). Two-pass (no false culls — the negative test). **L. RENDER.** Needs P8 + **S-HIZ** + P1.

### PHASE P12 — BVH dirty-region regen + JFA (+ SDF motion vectors for the mutating field) — THRESHOLD-GATED

As v1 §P12. Per-cascade AABB BVH over non-empty bricks, dirty-region JFA; BVH = software-traversal accelerator (G-BRICKRT) + HW-RT BLAS (P14). **A GPU-mutating field is where SDF motion vectors need per-hit velocity (Open-Q7) — that work lands here, not P6.** **Determinism: a GPU-mutating brick field is render-side (FORKED-BACKEND); the CPU physics evaluator stays on the analytic field (C3/M6 boundary preserved).** **XL. Class: FORKED-BACKEND.** Needs P9.

### PHASE P13 — Async compute (multi-queue + cross-queue semaphores) — PROFILE-GATED

As v1 §P13. Extends the **P6-S** seam to cross-queue semaphores. **F-GRAPH already reserves the (src-queue, dst-queue) dimension (M7) — P13 populates the second queue without rewriting the graph.** STRICTLY profile-gated (build only when a GPU profile shows under-occupancy). **XL. Class: CPU/sync (no field).** Needs P3a/P3b + P6-S + F-GRAPH's queue dimension.

### PHASE P14 — AS/RT seam + HW-RT SDF-as-AABB-BLAS backend — CAPABILITY-GATED (accelerator, not GI prereq)

As v1 §P14, re-scoped (Open-Q6): **with G-BRICKRT, HW-RT is now strictly an accelerator over an existing software secondary-ray path — Track G no longer depends on P14.** Pack non-empty bricks as `VK_GEOMETRY_TYPE_AABBS_KHR` BLAS; the intersection shader solves the FROZEN field inside the hit brick. **Capability: `VK_KHR_acceleration_structure` + `VK_KHR_ray_query`/`ray_tracing_pipeline` — in-house fallback: G-BRICKRT (software HDDA) / the P4/P5 sphere-tracer.** Golden-equal vs G-BRICKRT/the software tracer (the fallback is the oracle). **XL. Class: FIELD-CONSUMER (intersection-shader solve via the frozen field; physics untouched).** Needs P9.

### PHASE P15 — RT lighting (shadows/AO/GI/reflections) + SVGF — CONFIRMED FUTURE (accelerated variant)

As v1 §P15, re-scoped (Open-Q6): **v2 ships software equivalents of ALL these earlier (A1 shadows, A2/A13 AO, A7/A8/A10/A12 GI, A4 reflections) on the marcher/bricks; P15 is the HW-RT-accelerated variant for incoherent rays, NOT the first appearance of these effects.** Pair with **G-SVGF** (the built-out SVGF). **XL. Class: FIELD-CONSUMER (RT rays solve the frozen field; denoiser is render-side).** Needs P6-S + P14 + G-SVGF.

---

# TRACK G — FRONTIER GI / NEURAL / PATH-TRACING

> The capstone layer over G-BRICKRT (or P14). Pillars: RESAMPLING (G-RDI/RGI/PT/REGIR — pure compute reservoir streaming, RT-agnostic, served by G-BRICKRT/P4/P14), DENOISERS (G-SVGF/REBLUR/FIRE — built on P6-S + X-SPD), NEURAL (G-NRC/NDENOISE/NUP/NMAT — coopmat-gated, subgroup/non-neural fallback, float-nondeterministic, firewalled from physics), and the non-neural twin G-SHARC. **Every stochastic phase lists X-REF (the acceptance oracle, C4) as a hard prerequisite.** Visibility rays are FIELD-CONSUMER via `sdf_field.hlsli`; none enter `boyko_sdf_math`.

### PHASE G-SVGF — SVGF / A-SVGF / ReLAX denoiser — needs P1+P6-S+X-SPD

**What.** Temporal accumulation (EMA + luma moments via motion-vector reprojection) → per-pixel variance → 5 à-trous wavelet iterations with depth/normal/luma edge-stopping. A-SVGF adds temporal-gradient adaptive alpha; ReLAX adds fast-history clamping. Albedo demodulated before / remodulated after.

**Why our path needs it.** **This IS the P15 'SVGF' line, built out** — a real subsystem, not one bullet. **The shared infra is the P6-S motion-vector/history/reprojection seam; the à-trous + edge-stopping + variance core is the real per-phase work (m4 — NOT "80% free").** The natural denoiser for G-RDI/RGI/PT AND the cheaper A1 cone-shadow + A13 SSGI noise.

**How.** In-house raw-Vulkan compute (Q2VKPT/vkdt reference, reimplemented). No extensions/RT/neural HW. Edge-stopping is scalar ALU.

**Expected win.** Converged image from few-sample signals; GPU-timestamp + SSIM vs the **X-REF** reference.

**Dependency/order.** Needs P1 + **P6-S** + a noisy signal (A1/A13/G-RDI/P15) + **X-REF** (acceptance).

**Gate.** Render-consumer filter; never touches the field; may relax math; temporally non-deterministic by design (outside the physics gate); golden at the **X-REF** relaxed SSIM/PSNR bar on a converged reference (NOT bit-equality); validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** None. **Class: RENDER (no field).**

### PHASE G-REBLUR — ReBLUR-style recurrent-blur denoiser — needs P1+P6-S+X-SPD

Cross-bilateral recurrent blur over a mip-hierarchy (**consumes X-SPD's bilateral reduction**, M4) + temporal accumulation; specular virtual-position tracking. The specular/AO companion to G-SVGF (A4 reflections, A3 AO, P15 specular). **L. RENDER.** Needs P1+P6-S+X-SPD+X-REF+a specular/AO signal.

### PHASE G-FIRE — Firefly suppression + antilag — needs P6-S+a denoiser

Spike rejection + neighborhood YCoCg variance clamp (the TAA clamp generalized) + antilag, hardening few-sample signals. Reuses the P6-S clamp. **S. RENDER.** Needs P6-S+a denoiser+X-REF.

### PHASE G-RDI — ReSTIR DI — needs P1+P6-S+P7+X-REF+a visibility ray

**What.** Per-pixel streaming RIS (1-sample reservoir, temporal reuse from P6-reprojected previous frame + spatial neighbor reuse, MIS-weighted). 10–100× effective light samples at ~1 spp, thousands of lights.

**Why our path needs it.** Compute over the P1 G-buffer; temporal reuse reuses the **P6-S** motion-vector/history seam; **visibility/shadow rays via the software sphere-tracer (P4) / G-BRICKRT (bricks) / HW-RT (P14) — RT-core-agnostic**; reservoir SSBOs in P1's vocabulary.

**How.** Raw-Vulkan compute — no extension beyond P1. Subgroup ops (P7's caps) optionally accelerate neighbor reuse (scalar fallback). Lowered by F-GRAPH.

**Expected win.** Near-converged many-light direct lighting at ~1 spp; GPU-timestamp + variance vs **X-REF**.

**Dependency/order.** Needs P1 + **P6-S** + P7 (candidates) + a visibility ray (P4/G-BRICKRT/P14) + **X-REF**.

**Gate.** Render-consumer reservoir math; does NOT touch the frozen eval; stochastic by design (per-frame RNG-seeded) → render output non-deterministic like TAA (acceptable, outside the physics gate); **shadow/visibility rays MUST call the field through `sdf_field.hlsli`** (no fast-math in the probe); MIS/reservoir math relaxable; golden uses the **X-REF fixed-seed + relaxed-converged-reference protocol** (relative-MSE ≤ phase bar at K frames, NOT bit-equality); validation clean; GPU-timestamp.

**Effort.** L. **Conflicts.** Stochastic determinism surface (X-REF). **Class: RENDER (reservoir) + FIELD-CONSUMER (visibility rays via the frozen field).**

### PHASE G-RGI — ReSTIR GI — needs G-RDI + 1-bounce ray

One-bounce indirect reservoir reuse (sample = secondary hit + incoming radiance) from one bounce of the software trace / G-BRICKRT / HW-RT; radiance into reservoir SSBOs reprojected by P6-S. **L.** Needs G-RDI + a 1-bounce ray + P6-S + **X-REF**. **Class: RENDER + FIELD-CONSUMER (secondary rays via `sdf_field.hlsli`).**

### PHASE G-REGIR — ReGIR world-space grid light reservoirs — needs P7+G-RDI

A world-space grid pre-sampling reservoirs over local lights (decouples cost from light count), feeding G-RDI's initial sampling. Complements P7's screen-space froxel cull (P7 per-froxel for deferred; ReGIR per-world-cell for ReSTIR candidates). Grid SSBO addressed by hashed world position. **M. RENDER** (no field probe). Needs P7+G-RDI+X-REF.

### PHASE G-PT — GRIS / ReSTIR PT (full path reuse, shift maps) — needs G-RDI+(G-BRICKRT|P14)+P6-S

Generalized RIS reusing multi-bounce paths via shift mappings (reconnection/random-replay/hybrid). Reconnection vertices in an SSBO column; ray casts via **G-BRICKRT software HDDA (RT cores sufficient-not-necessary)** or P14. Heaviest single technique. **XL.** Needs G-RDI + a multi-bounce ray + P6-S + **X-REF** (+ doubles as the X-REF reference generator at high spp). **Class: RENDER + FIELD-CONSUMER (path-vertex probes via `sdf_field.hlsli`).**

### PHASE G-SHARC — Spatial-Hash Radiance Cache (non-neural NRC twin) — needs G-RGI/G-PT

**What.** A world-space radiance cache WITHOUT neural networks: hash the world position (LOD-scaled voxel) into a hash-grid SSBO, accumulate incoming radiance per cell with temporal feedback, query to terminate/short-circuit paths.

**Why our path needs it.** **The recommended FIRST radiance-cache rung before G-NRC** (Open-Q9): same path-shortening benefit, pure compute + hashing — no tensor cores, no training. World-position hashing matches our world-space SSBO patterns + the brick atlas's world parameterization. **Ship the cache architecture + prove the path-termination win, then optionally swap the backend to NRC behind the same query interface (the P9 forked-backend discipline).**

**How.** In-house raw-Vulkan compute — a spatial hash + atomic accumulation into an SSBO + a ray primitive (G-BRICKRT). No RT cores / neural HW / extensions. Broadest HW support of any cache option. Lowered by F-GRAPH.

**Expected win.** Path-termination cache without neural HW; GPU-timestamp.

**Dependency/order.** Needs G-RGI/G-PT (or P15) + a world-space hash-grid SSBO + a visibility ray + **P6-S** + **X-REF**.

**Gate.** Render-side; temporal accumulation → non-deterministic frame-to-frame (acceptable, outside the physics gate); far MORE determinism-amenable than NRC (no fp16 tensor math, no online weights); cache-population field probes via `sdf_field.hlsli`; not a physics source; golden at the X-REF bar; validation clean; GPU-timestamp.

**Effort.** M. **Conflicts.** None. **Class: RENDER (cache) + FIELD-CONSUMER (visibility rays).**

### PHASE G-SOIT — Stochastic / hashed-alpha OIT — needs P6-S + a denoiser

Single-pass sort-free transparency (stochastic sub-pixel coverage ∝ alpha, or hashed threshold). **Our pipeline is already stochastic (ReSTIR + TAA + denoiser), so the noise is absorbed by the existing temporal accumulation** — exactly its best regime, and the OIT-sort cost vanishes. Fixed memory, no lists, no sort. **M. RENDER** (coverage decision, no field). Needs P6-S + a denoiser + **X-REF** (the hashed-alpha hash is a pure reproducible function; the stochastic dither is RNG-seeded, resolved by TAA).

### PHASE G-NRC — Neural Radiance Cache (online-trained MLP) — CAPABILITY-GATED

**What.** A small MLP (6×64 ReLU, frequency/one-blob encoding) caching outgoing radiance over (position, direction, normal, roughness, albedo); trained ONLINE each frame from self-generated path traces; most paths terminate early into the cache. Position queries are SDF world points — **the field gives a cheap exact spatial parameterization (distance + normal) ideal as MLP input features.**

**Why our path needs it.** Path-termination for G-PT/P15 GI. The single most frontier "no one ships it in-house" capstone — **and the only family that cannot satisfy a bit/tolerance golden even with seed control (Open-Q9: it needs its own statistical bar via X-REF).**

**How.** Raw Vulkan WITHOUT CUDA/vendor SDK: a fully-fused trainable MLP via `VK_KHR_cooperative_matrix` (RTX 3060 Ampere supports it from compute; VkNRC proves it, reimplemented). **MANDATORY fallback: a subgroup-shared-memory GEMM (slower but correct), or G-SHARC (the non-neural twin) behind the same query interface.** HW dependency = coopmat, NOT RT cores.

**Expected win.** Shorter paths via a learned cache; GPU-timestamp + variance vs X-REF.

**Dependency/order.** Needs G-PT/P15 + `VK_KHR_cooperative_matrix` (a new DeviceCaps query + raw-FFI enable) + a compute train+inference pipeline + **X-REF** (statistical bar). **G-SHARC ships first as the proof rung behind the same interface.**

**Gate. STRICTLY render-consumer-side, FIREWALLED from `boyko_sdf_math`.** MLP train/inference is fp16/fp32 tensor math with FMA-contraction + nondeterministic online weights — categorically incompatible with the FROZEN field eval; it may NEVER be the field's source of truth (physics keeps the analytic CPU field). Its INPUTS (position/normal) come via `sdf_field.hlsli` unchanged; its OUTPUT (cached radiance) is render-only. Golden = the **X-REF** statistical bar (bit-equality impossible, not required); capability-gate verified (a non-coopmat device takes the subgroup-GEMM fallback or G-SHARC); validation clean; GPU-timestamp.

**Effort.** XL. **Capability:** `VK_KHR_cooperative_matrix` — **fallback: subgroup-GEMM / G-SHARC.** **Conflicts.** Float-nondeterministic (firewalled). **Class: RENDER (firewalled) + FIELD-CONSUMER (inputs only).**

### PHASE G-NDENOISE — Neural denoiser (kernel-predicting CNN) — CAPABILITY-GATED

A small CNN over noisy radiance + G-buffer outputting the denoised image (or per-pixel kernels), unifying denoise + temporal stabilize. **A higher-quality replacement for G-SVGF/REBLUR behind the same denoiser interface, with G-SVGF as the mandatory fallback (Open-Q9).** Trained offline on our own path-traced references (X-REF — we own the renderer, we generate ground truth). **XL. Capability: `VK_KHR_cooperative_matrix` — fallback: G-SVGF.** fp16 conv, nondeterministic, firewalled from the field; X-REF statistical bar. **Class: RENDER (firewalled; no field).** Needs G-SVGF baseline + coopmat infra + X-REF.

### PHASE G-NUP — Neural super-resolution — CAPABILITY-GATED

A temporal CNN upscaler (radiance demodulation, FuseSR) generalizing C-TSR/P5. **The shader-only FSR-class variant IS C-TSR (already shipping, deterministic-friendly, the low-risk first step + mandatory fallback); the CNN is the optional coopmat upgrade.** **L. Capability: `VK_KHR_cooperative_matrix` (CNN) — fallback: C-TSR.** **Class: RENDER (no field).** Needs P5+P1+P6-S(+X-REF for the CNN variant).

### PHASE G-NMAT — Neural material / BTF compression — CAPABILITY-GATED

A per-material MLP decoder over BC6 latent feature textures (BC6 = fixed-function HW decode; the MLP via coopmat). Feeds the P1 material G-buffer; SDF surfaces gain rich appearance without per-edit texture blowup. **L. Capability: `VK_KHR_cooperative_matrix` / `VK_NV_cooperative_vector` — m3: `VK_NV_cooperative_vector` is acceptable as a RAW Vulkan extension (raw-FFI, NOT a forbidden vendor SDK like DLSS/OptiX), with the KHR/subgroup/scalar fallback mandatory.** fp16 inference, firewalled (materials are appearance, never geometry — never enter `sdf`/`smin`/`combine`); X-REF bar. **Class: RENDER (firewalled; no field).** Needs P1 + coopmat/coop-vector infra + BC texture support.

---

# TRACK F — RENDER-GRAPH / RHI SUBSTRATE EXTENSIONS

> F-GRAPH (the render-graph) and X-SPD/X-REF are defined above as near-spine/shared infra. The remaining F-* are RHI substrate extensions landing as needed; all capability/threshold-gated, all with in-house fallbacks.

- **F-SYNC2 — `VK_KHR_synchronization2`** upgrade of the barrier emission (F-GRAPH emits sync2 when available). **S. Capability** (core 1.3, RTX 3060 guaranteed) + sync1 fallback.
- **F-BIND — descriptor-indexing / bindless** (F-GRAPH-aware). **M. Capability** (core 1.2) + bound-sets fallback.
- **F-FP16 — `shaderFloat16` G-buffer/intermediate** packing (octahedral normal pairs, Open-Q3). **S. Capability** + FP32 fallback. **RENDER (never the field — FP16 is forbidden in the FROZEN eval).**
- **F-PUSH — push-descriptor / push-constant** fast bind. **S. Capability** + descriptor-set fallback.
- **F-MEM / F-ALIAS — sub-allocator + transient aliasing** (extends `suballocator.rs`; F-GRAPH computes the aliasing windows). **M/M. Threshold.** Feeds §VRAM Budget.
- **F-PSO — parallel PSO warm** via the Chase-Lev threadpool (`boyko_threadpool/`); bounds the §PSO permutation compile cost. **S.**

All F-* are RENDER/CPU-substrate (no field). F-GRAPH is their organizing center.

---

## 2. Cross-cutting principles applied (extended for v2)

- **0%-gate.** Every render pass runs only under the `boyko_render` schedule; a world not using it pays nothing; the named CPU hot loops (`row_ptr`, `for_each_chunk`, query iter, `find_ready`) stay byte-identical. GPU type-keyed routing stays behind const/flag gates (CG-D5).
- **Determinism boundary (§0a, the load-bearing constraint).** `sdf`/`smin`/`smax`/`combine`/normal + `golden_composite_pixel` are **byte-identical across the entire plan** — the CPU/GPU golden source of truth physics reuses. No fast-math, no reordered FMA, no rsqrt/rcp, no FP16. The single gateway is `sdf_field.hlsli` (the P4 invariant). Classes: B is mostly FIELD-CONSUMER except B7 (FROZEN-preserving — distance-exact); A is FIELD-CONSUMER (offset probes) except screen-space A8/A13 (RENDER); C/D/E/S/G are RENDER (or FIELD-CONSUMER for visibility/in-brick solves); P9/P10/P12 are FORKED-BACKEND (render fetch forked; analytic FROZEN preserved as physics). Render-history non-determinism (TAA/temporal) is OUTSIDE the physics gate. **Traps: B9's 4-tap normal is FROZEN-adjacent (keep 6-tap frozen, fork a render 4-tap — Open-Q2); E-DENS-B's fast-math applies ONLY to the extinction remap of the already-evaluated scalar, never the probe (C3).**
- **Measurement oracle.** The GPU half cannot be Miri'd: golden-buffer diffs (bit-exact where possible; **owner-approved documented tolerance where not — every relaxing phase carries a per-phase tolerance gate, NOT critic-discretion**) + Vulkan validation + sync-validation wired to FAIL tests, on RTX 3060. **Stochastic phases (G-RDI/RGI/PT, G-REGIR, G-SOIT, G-NRC/NDENOISE, A14) use the X-REF protocol: a fixed-seed + relaxed-converged-reference + a named metric/threshold + an owner statistical-bar sign-off — never bit-equality (C4).** CPU-side lowering/build/octree (F-GRAPH build, B7 bound build) stays criterion + Miri where unsafe.
- **In-house / native / raw-Vulkan.** No ash/wgpu; all new verbs raw-FFI behind the RHI trait (static dispatch, monomorphized, no `dyn` in the hot record path). The seam-with-default-`#[cold]`-body pattern is the template. Every open-source reference (FSR2, XeGTAO, NRD, VkNRC, FidelityFX-SPD, meshoptimizer) is **REIMPLEMENTED**, never linked. **`VK_NV_cooperative_vector` is an acceptable RAW extension (raw-FFI), not a forbidden vendor SDK (m3).**
- **Render-graph as the barrier authority (C1).** Once F-GRAPH lands, every Track-E/S/G/A-GI/D pass declares its resource uses and is lowered by F-GRAPH (barriers + layouts + transient aliasing + the reserved queue dimension). The two pre-F-GRAPH codepaths (hand-FFI `swapchain.rs` batched by P3a; ECS-edge `lower_barriers` narrowed by P3b) are subsumed.
- **Shared SPD primitive (M4).** X-SPD is the single parametric mip-reduce; P11/S-HIZ/S-GTAO/E-BLOOM/E-MBLUR/G-REBLUR consume it. The odd-dimension boundary fix ships once.
- **Capability gating discipline.** Every HW-dependent phase (C-VRS, D-MESH/SWRAST/VIS, S-VSM, F-BIND/FP16/PUSH/SYNC2, P14, G-NRC/NDENOISE/NUP/NMAT) carries a `DeviceCaps` query at device-create + a named in-house fallback (full-rate / compute-raster / int64-atomic / CSM / FP32 / G-BRICKRT / G-SHARC / subgroup-GEMM / C-TSR / scalar). `shaderBufferInt64Atomics` (D-VIS/SWRAST/S-VSM) + descriptor-indexing/sync2 (F-BIND/SYNC2) are core 1.2/1.3 → guaranteed on the RTX 3060 oracle, still gated for portability.
- **Differential SIMD/subgroup discipline (P7/X-SPD/D-CULL).** Subgroup paths ship with a scalar reference + a differential golden (the CPU-D3 house template applied to GPU), capability-gated, never hard-coding wave width.

## 2a. VRAM budget accounting (RTX 3060 Laptop, 6 GB) — completeness gap (d)

Large allocations must sum against 6 GB. The plan tracks a running budget; F-MEM/F-ALIAS transient aliasing (computed by F-GRAPH) reclaims non-overlapping lifetimes.

| Consumer | Approx footprint @1080p | Notes |
|---|---|---|
| G-buffer (depth+normal+albedo+material+motion) | ~40 MB | R8 normal now; F-FP16 octahedral halves normal/material |
| History (P6 color + B5 hit-`t` + reservoir ping-pong) | ~50–120 MB | double-buffered; bounded, NOT blanket-doubled |
| Froxel/fog 3D textures (E-FOG 160×90×64 ×RGBA16 ×2) | ~30 MB | + E-CLOUD noise (one-time startup) |
| Brick atlas (P9, R16, `maxImageDimension3D`-sized) | **budgeted, NOT hardcoded** | the largest single consumer; capped by the atlas pool size; R16 (O2) |
| Clipmap cascades (P10) | shares the atlas pool | cascades share one atlas pool, not N copies |
| VSM pages (S-VSM 16k² sparse) | sparse — only touched pages | the whole point of S-VSM is sparsity |
| Reservoir SSBOs (G-RDI/RGI/PT) | ~30–80 MB | bounded, configurable cells×reservoirs |
| OIT node pool (E-LLOIT) | preallocated fixed pool | clamped (no overflow UB) |
| BVH (P12) | per-cascade, dirty-region | refit not rebuild |

**Discipline:** the brick atlas + reservoirs + history are the three pressure points; each is **configurable + budgeted at setup** (no hot-path growth — the "preallocate at setup" principle). F-ALIAS aliases transient passes (e.g. bloom mips, TileMax) into shared backing. **A budget-overflow at setup is a hard error (a documented cap), never a silent OOM.** The owner sets the per-consumer caps (Open-Q13).

## 2b. PSO / shader-permutation discipline — completeness gap (c)

60+ passes × feature/material variants is a PSO-compile-time + I-cache liability. Discipline:
- **Permutation budget per pass** — each pass declares its variant axes (e.g. subgroup-vs-scalar, camera-mode, light-count tier); the product is bounded and documented. Specialization constants (not `#ifdef` explosion) where the driver supports them.
- **F-PSO parallel warm** — compile permutations on the Chase-Lev threadpool at load (preallocate at setup, never in the frame loop).
- **I-cache (principle 3)** — hot marcher/lighting loops stay compact; `#[cold]`/`#[inline(never)]`-equivalent (rare-branch hoisting) for error/fallback paths; no blind always-inline. The branchy fallbacks (capability-off paths) are out-of-line.
- **Variant collapse** — capability fallbacks (scalar/FP32/full-rate) are separate pipelines, not branches in the hot variant, so the common path stays branch-lean.

## 2c. G-buffer bandwidth accounting — completeness gap (d) / Open-Q5 tension

~30 consumers read the P1 G-buffer (depth/normal/albedo/material/motion) at 1080p×60. Read-bandwidth across the deferred-lighting + Track-S AO/shadow + Track-E fog/post + Track-G denoiser/reservoir consumers is the deferred renderer's classic wall.
- **Mitigations:** F-FP16 octahedral normal + packed material (halves two channels, Open-Q3); the **D-VIS 64-bit visibility buffer** (Open-Q5) eliminates the MRT attribute fetch entirely for visibility (mesh+SDF `atomicMin` → no albedo/material/normal MRT bandwidth for the occlusion pass) — quantify D-VIS vs MRT once the cluster pipeline lands; tile-local G-buffer caching (consumers in the same tile share one fetch via shared memory).
- **The tension (Open-Q5):** P1 MRT first (it unblocks every track immediately); D-VIS when Track-D lands (it overlaps P1's depth seam — resolve the scope overlap then). The bandwidth sum is the deciding metric: if the ~30-consumer read-bandwidth measured post-P7 exceeds budget, D-VIS is promoted from optional to required.

---

## 3. "Production-ready when" — spine exit criteria

The renderer is production-scale (the spine complete) when ALL hold: **P0** (1080p perspective, ECS-fed, ortho goldens bit-exact, sparse/dense baselines), **P1** (real MRT-image deferred renderer, depth-copy gone, golden-equal), **P3a** (per-frame barrier call count reduced, sync-validation clean), **F-GRAPH** (the 4-pass frame's barriers are graph-derived, sync-validation clean, the queue dimension reserved), **P4** (measured fine-march step reduction on the sparse scene, conservative), **P5** (half-res+upscale at an owner-approved relaxed tolerance, field eval byte-identical, ~4× ray reduction), **P6** (TAA-stable on static+pan+disocclusion, the P6-S semaphore-present seam built + sync-validation clean, field eval byte-identical), and throughout (validation+sync-validation clean, 0%-gate, every `unsafe` carries `// SAFETY:`).

**v2 addition — "SDF-native lighting MVP when":** **B1** (over-relaxation, ω-gated) + **B5** (mesh-depth + previous-frame seeding) + **B7** (Lipschitz pruning, distance-exact) + **A1** (cone-trace soft shadows) + **A2** (5-tap SDF AO) + **C-AA** (analytic edge AA) all ship on the software marcher behind `sdf_field.hlsli`, golden-tested (additive flags; ortho goldens bit-exact; field eval byte-identical) — delivering contact-hardening soft shadows + crevice AO + smooth edges + materially faster marching on the **analytic** field, **with zero new Vulkan features and no HW-RT**. This is the owner-demanded SDF-native bar, reachable immediately after the spine and BEFORE any commodity post or neural work.

P3b/P7/P8/D-FWD/X-SPD may land in parallel after P1. The SDF-native fast-track (B + A1/A2/C-AA) lands right after the spine. Track-C reconstruction/Track-E post/Track-S classic shadows land after P6. Everything else (P9–P15, A-GI capstones, Track D, Track G, the F-* substrate extensions) is threshold/capability/profile-gated and NOT part of the production-ready spine bar.

---

## 4. Priority table

| Tier | Phases | Gate | Class |
|---|---|---|---|
| **Spine** | P0, P1, P3a, **F-GRAPH**, P4, P5, P6 | none (must ship) | RENDER + FIELD-CONSUMER (P4) |
| **SDF-native fast-track** | B1, B5, B7, A1, A2, C-AA | none (the owner-named MVP) | FIELD-CONSUMER / FROZEN-preserving (B7) |
| **Parallel-after-P1** | P3b, P7, P8, D-FWD, X-SPD | none | RENDER |
| **Reconstruction / classic shadow-AO** | C-TSR, C-CTSS/VRS/CBR, S-HIZ, S-GTAO, S-CONTACT, S-CSM | none / capability (VRS) | RENDER |
| **Volumetrics / OIT / particles** | E-FOG, E-SDFVOL, E-DENS-A/B, E-GOD(a), E-WBOIT/MBOIT/LLOIT, E-PART | threshold (E-DENS/SDFVOL/PART at scale) | RENDER + FIELD-CONSUMER |
| **Polish-tier post** | E-BLOOM, E-DOF, E-MBLUR, E-EXP, E-TONE, E-LUT, E-GOD(b) | none (low priority) | RENDER |
| **SDF-native GI** | A8, A13, A7, A10, A12 | threshold (brick ones) | RENDER / FIELD-CONSUMER |
| **SDF-accel capstones** | P9 (re-eval vs B7), P10, G-BRICKRT, P11, P12 | threshold | FORKED-BACKEND / FIELD-CONSUMER |
| **GPU-driven geometry** | D-VIS, D-SWRAST, D-MESH, D-CULL, D-COMPACT, S-VSM | threshold / capability | RENDER |
| **Frontier GI / neural** | X-REF, G-SVGF/REBLUR/FIRE, G-RDI/RGI/REGIR/PT, G-SHARC, G-SOIT, G-NRC/NDENOISE/NUP/NMAT | X-REF statistical bar / capability | RENDER (firewalled) + FIELD-CONSUMER |
| **HW-RT (accelerator)** | P14, P15 | capability / confirmed | FIELD-CONSUMER |
| **Profile-gated** | P13 | profile (under-occupancy) | CPU/sync |
| **RHI substrate** | F-SYNC2/BIND/FP16/PUSH/MEM/ALIAS/PSO | capability / threshold | RENDER / CPU |

---

## 5. Open questions for the owner (decisions, not critic-discretion)

1. **Per-phase relaxed-tolerance / baseline-reset approval (C3, extended).** P5/P6/P9/P12 (v1) + C-TSR/C-AA/C-CTSS/C-VRS/C-CBR + all of Track-E post/fog/OIT + S-GTAO/contact + E-SDFVOL/E-DENS-B (consumer accumulation only) + the stochastic Track-G phases each leave bit-exact on the *render* side. **Owner sign-off on each per-phase tolerance bar.** Default: ±4/255 SSIM/PSNR for reconstruction/post; documented per-format quantization for P9/P12/F-FP16; the **X-REF** fixed-seed + relaxed-converged-reference (NOT bit-equality) for all stochastic phases.
2. **B9 render-only normal fork.** The 4-tap tetrahedron normal halves normal cost but changes the bit pattern → FROZEN-adjacent. **Proposal:** keep the 6-tap central-difference normal as the frozen physics-shared normal (byte-identical), fork a render-only 4-tap behind a separate function, under a one-time owner-approved render baseline reset. **Confirm.**
3. **B7 vs P9 ordering.** B7 (distance-EXACT, analytic, ZERO render↔physics divergence) attacks the same `O(edits)` wall as P9 (lossy R16). **Proposal:** build B7 before committing to P9; treat P9 as the world-scale/discrete-field tier B7 cannot reach. **Confirm the priority.**
4. **C-TSR supersedes simple P5/P6.** **Proposal:** ship P5/P6 simple as the forked proof; upgrade to C-TSR (full FSR2/TSR). **Confirm the two-stage approach** vs jumping straight to C-TSR. (What proves C-TSR "good enough"? — the native-res ground truth at matched frames is the upper bar.)
5. **D-VIS supersedes the P1 depth-copy + the §15.1 seam.** A 64-bit visibility buffer gives correct mesh↔SDF occlusion + no MRT bandwidth. **Proposal:** P1 MRT first (it unblocks every track immediately); D-VIS when Track-D lands (resolve the depth-seam overlap then — promoted to required if the §2c bandwidth sum exceeds budget). **Decision needed.**
6. **Software-first GI vs HW-RT (the core v2 verdict).** **Proposal:** the software/SDF-native GI path (Track A-GI + Track G over G-BRICKRT) is the PRIMARY GI; HW-RT (P14/P15) is the optional accelerator for incoherent rays on capable devices, NOT the GI prerequisite. **Confirm this inversion of the v1 priority.**
7. **SDF motion vectors for a GPU-mutating field (scoped to P12).** Static analytic field + jittered camera (P6) → deterministic camera reprojection, sufficient. A GPU-mutating field (P12) needs per-hit velocity, folded into P12. **Confirm the split.**
8. **G-buffer normal format (O3).** R8G8B8A8 now, R16G16 octahedral later (a golden-baseline reset, pairs with F-FP16). **Proposal:** start R8; switch to octahedral once P6/C-TSR exposes normal-precision banding, under a one-time owner-approved baseline reset.
9. **Coopmat neural tier.** G-NRC/NDENOISE/NMAT need `VK_KHR_cooperative_matrix` (RTX 3060 supports) + are float-nondeterministic (firewalled). **Proposal:** ship the non-neural twins first (G-SHARC for NRC, G-SVGF for NDENOISE, C-TSR for NUP) behind a shared query interface; the neural backends are an opt-in capability-gated upgrade with the non-neural path mandatory. **Confirm the neural tier is in-scope at all** (it is the only family that cannot satisfy a bit/tolerance golden even with seed control — it needs its own statistical bar via X-REF).
10. **Mesh material G-buffer scope (C2 / Open-Q11).** The mesh side is currently flat-color ("reading the mesh's real rasterized albedo is a deferred refinement", `sdf_depth_composite.hlsl:92-95`). **Does the mesh ever get a real material G-buffer, or stay flat-color?** This gates S-CSM, S-GTAO mesh occlusion, E-PART rendering, and D-FWD's transparent-material substrate. **Decision needed.**
11. **VRAM caps (§2a / Open-Q13).** The brick atlas + reservoir SSBOs + history are the three 6 GB pressure points. **Owner sets the per-consumer caps** (a setup-time hard error on overflow, never silent OOM).
12. **Render↔physics geometric-divergence bound (M6).** The P9 brick render-field (R16) and the analytic physics-field disagree by a bounded world-space error; a character's feet may visibly float/sink vs where physics says the ground is. **Document the max world-space divergence (R16 quantization × brick extent) and sign off that the visual/physics mismatch is acceptable** (B7 has zero such divergence — another reason to evaluate B7 first, Open-Q3).

---

## Files this plan is grounded in (absolute paths)

- `D:\claude\BoykoEngine\docs\OPTIMIZATION-PLAN-RENDER.md` — v1 spine (P0–P15) + gates + the two-codepath barrier distinction (this file, extended)
- `D:\claude\BoykoEngine\docs\RESEARCH-GRAPHICS-OPT.md` — v1 priority/sequence source
- `D:\claude\BoykoEngine\docs\PERF-DIRECTIONS.md` — RT-*/GPU-D*/MEM-D* catalog + 0%-gate + honesty discipline
- `D:\claude\BoykoEngine\docs\RESEARCH-FAST-MATH.md` — the SDF-golden determinism boundary (the physics-reuse contract; `:7-19,41-44`)
- `D:\claude\BoykoEngine\crates\boyko_rhi\src\{api,encoder,enums,queue}.rs` — RHI seam: AS absent (`api.rs:46-67`), `dispatch_indirect` stub (`encoder.rs:269`), `bind_storage_buffer` (`encoder.rs:38`), no-semaphore `submit` (`queue.rs:22,31`), `BufferUsage::INDIRECT` (`enums.rs:37`)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\{swapchain,device,rhi_impl}.rs` — on-screen hand-FFI barriers (`swapchain.rs:844,914,1169,1201,1337,1370,1432,1708,1831,1864,1926`), `record_scene:1123`/`sync_depth:2071`/`record_present_sampled:1661`, single queue (`device.rs:1634,1750`), COMBINED_IMAGE_SAMPLER-only `create_bind_group` (`rhi_impl.rs:717`)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\suballocator.rs` — the free-list sub-allocator F-MEM/F-ALIAS extend
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\{sdf_depth_composite,gbuffer.fs,deferred_light.fs}.hlsl` — the 64×64/16-edit/`MAX_IT=128` fixture marcher (`sdf_depth_composite.hlsl:86-87,104,117`) + the determinism-frozen field math (`:8-12,47-51`) — the future `sdf_field.hlsli` source + the MRT/deferred prototypes; **mesh flat-color deferral (`:92-95`, Open-Q10)**
- `D:\claude\BoykoEngine\crates\boyko_render\src\barrier.rs` — the **distinct** ECS-edge `lower_barriers` pass (`:1-15,58-76,196-233`); P3b adds constants + narrows here; **F-GRAPH generalizes its edge→barrier algorithm to images + reserves the queue dimension**
- `D:\claude\BoykoEngine\crates\boyko_threadpool\` — the Chase-Lev work-stealing pool F-PSO uses for parallel PSO warm
