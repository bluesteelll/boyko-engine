# Research: amortizing VB geometry fetch by folding an SDF march into the geometry pass

**Date:** 2026-08-20 · **Feeds:** [VB-SV0-DP6-DESIGN.md](VB-SV0-DP6-DESIGN.md) (external corroboration for the DP6 ladder)
**Scope:** external practice only. Attribution corrections up front: "Visibility Buffer Rendering with Material Graphs" is **John Hable** (filmicworlds.com), not Strugar (Intel — ASSAO/XeGTAO) and not Jimenez (Activision — SMAA/GTAO). No published Activision VB talk exists; the citable 2016 VB talk with numbers is **Engel's** GDCE 2016 (The Forge).

## TL;DR

1. **Nobody has published an isolated cost for the VB geometry-fetch step.** Best data: Hable's density sweep (RTX 3070, 1080p) — the WHOLE Visibility material pass is **1.06 ms at large triangles, byte-identical to Deferred's 1.06 ms**; the fetch becomes "a relevant cost" only near 1-px triangles (and even there VB 2.01 ms beats Deferred 4.64 ms). At large triangles the fetch is **hidden behind bandwidth — it is not the thing you are saving**.
2. **Industry practice for ray-marched shadows/AO is overwhelmingly a dedicated pass**, and the stated reasons are almost never occupancy: they are **reduced resolution + upsample** (UE DF shadows are half-res; The Gunk's 1/8-res cone-trace prepass took the march 6.6 → 4.0 ms — ADDING a pass saved 2.6 ms), **spatio-temporal sample distribution** (GTAO's 0.5 ms @1080p PS4 is a filtering result), and **async overlap**.
3. **The one shipped counter-example is id Tech 7**: DOOM Eternal computes SSR directly in the forward shader (id Tech 6 had a thin G-buffer for it) — "reduced render targets at the cost of higher register pressure". **No ms delta was ever published**; the precondition was a full depth prepass + hi-Z. id kept SSAO in a separate half-res pass.
4. **The register-pressure argument is real and canonical** (AMD: "a single spike in one branch can cause the entire shader to require a lot of VGPRs, even if that branch is never taken") — but AMD equally says "lower occupancy can help"; the literature does NOT support a blanket "inlining a march kills occupancy therefore lose". The actionable failure is **spilling**.
5. **Structural synthesis:** an SDF march has no completed-depth dependency (unlike SSAO/SSR), so the ordering argument for separate passes does not transfer — but an inlined march **cannot be async-overlapped by construction**, and AMD warns async performs poorly against export-bound shaders (which a VB fill is; the productive overlap partner is shadow-map/z-prepass geometry work).

## Measured anchors

| Source | Numbers |
|---|---|
| **Hable (RTX 3070, 1088×1080)** | Material pass Deferred/Visibility: **1.06/1.06 ms** (large tris), 2.95/**1.65** (medium ~8-10 px), 4.64/**2.01** (1 px). `VisUtil` (count→reorder→shade→reorder-back repack) ≈ **0.33 ms** at every density. Split-material-from-lighting rationale: merged shaders compile with worst-case register allocation of both. |
| **Engel GDCE 2016 (The Forge)** | 4K R9 380: VB **15.19** vs Deferred **20.19 ms**; 4×MSAA 37.86 vs 69.64. **99 % L2 hits** for VB textures/vertex/index. Memory 86.77 vs 160.69 MB @4K. |
| **Peters (RTX 2080 Ti, 1080p)** | VB 32 bpp; shade pass reads **384 bits/px** of vertex data; VB creation 0.36 ms; whole frame 1.6 ms; "all memory reads heavily cache-coherent". |
| **UE DF shadows (Radeon 7870, 1080p)** | half-res + depth-aware upsample: directional 3 cascades **2.3 vs 3.1 ms** (shadow maps); 6 cascades 2.8 vs 4.9; 1 point 1.3 vs 1.8; 5 points 1.8 vs 3.2. |
| **The Gunk (shipped SDF marcher)** | XSX **4 ms @4K**, Xbox One 10 ms @1080p; the 1/8-res cone-trace prepass saved **2.6 ms** (6.6→4.0). |
| **GTAO (Activision)** | **0.5 ms @1080p PS4** via spatio-temporal distribution + denoise. XeGTAO: 0.56 ms @1080p RTX 2060 (3 passes). |
| **Async (Interplay of Light, 2025)** | GTAO async over RT shadows: GTAO isolated **1.97→3.22 ms** but the serial pair **5.73→~4.6 ms combined** (>1 ms saved). DOOM 2016 (Sousa): async gained **3–5 ms**/frame. Counterweight (Pettineo): overlap can make totals WORSE via shared-resource contention; AMD: async performs poorly against export-bound shaders. |
| **Nanite UE 5.4 shading bins** | 3 075 of 3 779 bins empty (81 %); empty-bin compaction saved **≈1 ms**; the NAIVE compute port was SLOWER than the 5.0 pixel path — the repack is where the win lives. Morton-ordered 2×2 quad assignment. |
| **SER (NVIDIA hardware reorder)** | path tracing 20–50 %; UE5 HWRT reflections 20–30 %; Lumen compaction ~20 %. Vs software compaction (ray-reordering survey, arXiv 2506.11273): up to **5× fewer warps but ≤16 % faster** — execution divergence traded for memory divergence. |
| **VGPR cliffs** | AMD RDNA3/4: 1536 VGPR/SIMD; ≤96 VGPRs = 16 waves; 128 = 8 waves; allocation granularity >1. GCN (Aaltonen): 40 VGPRs → "mere 40 % occupancy" example. NVIDIA: ≥64 registers ⇒ consider 32-thread groups. |

## The load-bearing quotes

- AMD (Occupancy explained): *"a single spike in one branch of the shader can cause the entire shader to require a lot of VGPRs, even if that branch is never taken in practice"* — and, same page: *"just as maximum occupancy can hurt performance, lower occupancy can help it"*; check for **spilling**.
- Hable: merged Material+Lighting *"get compiled with the worst case register allocation for both."*
- AMD RDNA guide: *"Async compute performs poorly when executed in parallel with export bound shaders."*
- Laine/Karras/Aila, **"Megakernels Considered Harmful"** (HPG 2013): merging heterogeneous/divergent stages into one kernel forces worst-case register allocation and destroys latency hiding; queue-connected split kernels win despite extra memory traffic. (Thesis verified; numeric tables not extracted.)

## Pitfalls the literature names

- **Assuming the fetch is the cost** — Hable's 1.06==1.06 identity: in bandwidth-bound regimes there is ~nothing to amortize; Engel's 99 % L2 hits say VB fetch coherence is usually good.
- **Optimizing occupancy as an end** — spilling is the real cliff.
- **A rarely-taken branch still allocates** (worst-case-of-the-union).
- **Compaction that cuts warps without cutting time** (5× warps, ≤16 % time).
- **Async that slows totals** (contention; export-bound partners).
- **Compute-ness is not the win** (Nanite's naive port was slower).
- **Merged passes multiply permutations/compile cost** (Reed).

## Cases of moving an effect between passes

| Case | Direction | Delta |
|---|---|---|
| id Tech 6→7: SSR into the forward shader | INTO | none published; "bandwidth for register pressure"; full depth prepass precondition |
| Nanite 5.0→5.4: material shading to compute bins | restructure | naive port slower; compaction ≈1 ms |
| The Gunk: added 1/8-res prepass before the march | ADDED a pass | **−2.6 ms** |
| Hable: split Lighting OUT of Material | OUT | register-allocation rationale; VB −23/−32 % vs Deferred at medium/high density |
| GTAO to async queue | out of serial | pair 5.73→4.6 ms |

**No published case of folding SDF shadows/AO into a G-buffer/VB geometry pass with a reported delta — in either direction.**

## Bearing on DP6 (synthesis)

- In a VB renderer the pass that owns decoded position/normal already runs after complete depth — the ordering blocker for inlining SSAO does NOT apply to `vb_geo`. The SDF march needs no depth at all.
- What inlining FORFEITS: half-res+upsample, temporal reuse, async overlap — each individually ≥ the entire large-triangle material pass (1.06 ms) in published numbers. **DP7 (half-res term) may dominate DP6 entirely.**
- What decides DP6 locally: (1) the actual fetch cost in OUR pipeline (DP6d arm B answers it — no external number exists); (2) peak VGPR of march vs `vb_geo` separately (RGA `--livereg`; the worst-case-allocation rule is the whole Decision-1 argument); (3) whether `vb_geo` at our density is bandwidth- or ALU-bound; (4) what the dedicated pass would overlap with if kept (per AMD: shadow-map/z-prepass work, not the VB fill).
- Alternative attacking the same cost from the other side: **Schied & Dachsbacher, "Deferred Attribute Interpolation" (HPG 2015)** — store per visible triangle one sample point + screen-space partial derivatives; shading never re-fetches vertices.

## Sources

Hable filmicworlds.com/blog/visibility-buffer-rendering-with-material-graphs · Engel GDCE 2016 (media.gdcvault.com Engel_Wolfgang_Visibility_Buffer.pdf) · Peters momentsingraphics.de/ToyRenderer3RenderingBasics.html · elopezr.com/a-macro-view-of-nanite · Wihlidal GDC 2024 Nanite GPU Driven Materials + sctheblog.com notes · gpuopen.com/learn/occupancy-explained + rdna-performance-guide + optimizing-gpu-occupancy (Aaltonen) · NVIDIA advanced-api-performance-shaders + Nsight shader profiler + SER blog · simoncoenen.com DoomEternalStudy · jarllarsson.github.io gunkraymarcher · UE distance-field-soft-shadows docs · Activision GTAO paper · GameTechDev/XeGTAO · therealmjp breaking-down-barriers-part-6 · interplayoflight async-compute-all-the-things (403 to automated fetch — figures from indexed excerpts, re-verify by hand) · wccftech DOOM async (secondary) · arXiv 2506.11273 ray-reordering survey · ACM 2790060.2790066 (DAIS) · NVIDIA Megakernels paper · selfshadow overdraw-in-overdrive · reedbeta deferred-texturing · jms55.github.io Bevy virtual geometry (no resolve numbers).
