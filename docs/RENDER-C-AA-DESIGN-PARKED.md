# Render C-AA — Analytic edge anti-aliasing (DESIGNED + PARKED, owner-deferred)

> Status: **DEFERRED by owner decision** (2026-06). The owner chose to keep the determinism-frozen SDF field at ZERO new reads rather than adopt the only workable single-pass design (which adds 4 read-only field taps per hit pixel). The analysis below is recorded so C-AA can be picked up cleanly when a prerequisite lands. C-AA is the remaining "smooth edges" leg of the owner-named SDF-native MVP; A1 (cone-trace soft shadows), A2 (5-tap AO), and B1 (over-relaxation) are already shipped.

## Feasibility verdict (why a clean version is not buildable now)
A **neighbor-free, zero-extra-field-eval, both-sided** analytic silhouette AA is **provably intractable on the current marcher**, for structural reasons verified in source:
1. `sdf_gbuffer_composite.hlsl` is a **1-D `[numthreads(64,1,1)]` dispatch** — no 2-D quad, so no hardware `ddx/ddy` derivatives and no free neighbor.
2. A **single center ray** yields a good *exterior* closest-approach distance (`d_min`, free — reuses the per-step `d`) but **no interior lateral margin** (the march breaks at the first `d<EPS` crossing).
3. The **resolve has no neighbor read** and mask==0 pixels carry no SDF attributes → it cannot reconstruct an SDF lit color for an exterior (missed) silhouette pixel to blend toward.
4. The normal `n = sdf_normal(p)` exists ONLY on the SDF-hit arm (inside `if (hit && t < t_mesh)`), NOT on the silhouette **miss** side; and the P4b coarse-cull EMPTY fast-path bypasses the post-loop entirely. So the v1-rejected `g=|rd·n|` term has no value on exactly the pixels AA targets — and `g` is a grazing *detector* not a sub-pixel *coverage* estimator anyway (it halos curved-face limbs and smears edge-on flats).

## The workable design (when revisited): hit-side 4-tap coverage
- **Coverage estimator (correct, SDF-text-AA form):** `cov = saturate(0.5 + d_signed / w_px)` where `d_signed` is the analytic signed field distance to the silhouette and `w_px` the screen-space pixel footprint (ortho: const `2·HALF_EXTENT/img`; perspective: `2·tan(fovY/2)·t/img_h`). This places the 50% contour AT the geometric edge and saturates to 1 within half a footprint inside — **no grazing-angle halo** (interior hits = cov 1; only the rim feathers).
- **Source of `d_signed`:** 4 reads of the FROZEN `field_distance` at the hit point offset by ±½-footprint along the screen-tangent (right/up) basis. This is the cost the owner deferred — 4 read-only taps per hit pixel.
- **Frozen-field safety (proven):** the 4 taps READ the frozen field, never modify it; live in a SEPARATE compiled AA shader variant (`*_aa.comp.spv`) so the OFF blob is **byte-identical** (committed `.spv` unchanged → the 0%-gate holds literally); the host oracle (`golden_marcher_attributes`/`golden_deferred_resolve`) mirrors the taps with the same `sdf_edit_list` so `cpu_gpu_sdf_agreement` + the distance/depth goldens are untouched; GATE-1 (`sdf_field_probe.comp.spv`) is unchanged. Single-sided (exterior fringe) AA in v1; interior sliver AA is a separate problem.
- **Plumbing (verified clean, reusable):** the resolve's 80B push range is already declared; `FineMarcherPush` 32→48B fits; coverage rides the free `gMaterial.a` lane, the mesh/bg backdrop bit the free `gAlbedo.a` lane (both confirmed read by nobody). Tolerance: edge band ±4/255 (linear lerp — no smoothstep slope amplification), interior + OFF byte-exact.

## Prerequisites that make a CLEAN (non-deferred) version feasible
Build C-AA the day ANY of these exists:
1. A **2-D / quad-tiled marcher dispatch** (gives `ddx/ddy` + free neighbor coverage) — removes the need for the 4 extra taps.
2. A **neighbor-aware resolve pass** (samples adjacent G-buffer texels; Mode B) — enables both-sided feather; must test `gMaterial.b>0.5` to skip SDF neighbors (an SDF neighbor's `gAlbedo` is raw base, not lit).
3. **C-TSR** (temporal super-resolution) — temporal AA supersedes analytic edge AA; pair C-AA's coverage as a TSR input rather than a standalone blend.

## Files this analysis is grounded in (absolute)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\sdf_gbuffer_composite.hlsl` (1-D dispatch; `n` only on the hit arm ~591; EMPTY fast-path ~429-447; `.a` stores hardcode 1.0)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\deferred_pbr.hlsl` (reads only `.rgb`/`.rg`/`.b`; `.a` lanes free)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\compute.rs` (`FineMarcherPush`, `MarcherAttributes`, the host goldens; ortho `rd=[0,0,-1]`)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\tests\field_probe_gate.rs` (GATE-1 freezes the field probe, not the marcher blob)
- `D:\claude\BoykoEngine\docs\OPTIMIZATION-PLAN-RENDER.md` (§C-AA line 357; §0a determinism; Open-Q1 tolerance)
