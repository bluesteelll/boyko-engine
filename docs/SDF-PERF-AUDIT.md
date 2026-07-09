# SDF Rendering — Performance Audit (vs the brick-map / clip-map SOTA)

Audit date 2026-06-23, branch `ecs`. Two-track investigation: (1) what our SDF
renderer actually does and which optimizations it uses (internal code audit), and
(2) the industry state-of-the-art for large, dynamically-modifiable SDF worlds
(Dreams / Claybook / Unreal GDF / academic), cross-checked against a reference
SDF-engine devlog. Read-only analysis; nothing here changes code.

---

## 0. Bottom line

Our SDF renderer is **squarely on the brute-force, per-pixel sphere-tracing end**:
every pixel re-evaluates the full analytic CSG edit-list distance function from
scratch, **once per march step**, with **no distance cache, no spatial
acceleration structure, no interpolated grid, no brick map, no clip-map, and no
BVH**. The per-pixel cost is `O(pixels × march_steps × edits)`.

This is a **deliberate, documented** position — the "basic analytic slice" — not an
oversight. Every cache-and-interpolate technique from the reference video is
*designed and parked behind a reserved seam* (`field_distance` in
`crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli:163`), with an explicit
deferred-subsystem table in `docs/PHASE6-BASIC-SDF-RENDER-PHYSICS-PLAN.md:305-345`
and a nine-track scaling roadmap in `docs/OPTIMIZATION-PLAN-RENDER.md`. The brute
cost is bounded today only because the edit list is hard-capped at
`MAX_SDF_EDITS = 16` (`crates/boyko_sdf_math/src/lib.rs:99`).

This is a *correct trade at this scope*, and it buys strengths the bricked engines
give up: **exact CSG** (infinitely sharp edges — Dreams / Claybook / UE all round
corners to voxel size for their cache), **one bit-identical field** shared by GPU
render + CPU render-golden + CPU physics (determinism + a test oracle + zero-readback
physics), **Tier-1 SOTA marching already shipped** (B1 over-relaxation, mesh-depth
hybrid bound, AO, soft shadows), and **physics-from-SDF done better than the
reference** (CPU analytic, zero readback, AVX2-batched — no per-edit re-meshing). The
renderer is on the analytic-base rung of a *deliberately staged ladder* whose next
rungs are pre-cut, not a dead end.

Two headline findings:

1. **A finished, golden-proven optimization is switched OFF in the live path.** The
   P4b coarse tile-cull (`sdf_tile_cull.hlsl`) is built, tested, and conservative —
   but the windowed present hard-codes `coarse_enabled = 0`
   (`crates/boyko_rhi_vulkan/src/swapchain.rs:~3552`). It is a ready lever, not a
   defect — flipping it on in the windowed present is the single highest
   value-per-effort change available.
2. **We "skipped" one video technique by doing it better.** Physics-from-SDF (the
   video's marching-cubes collision mesh) is replaced by *direct CPU analytic field
   sampling, zero readback, bit-identical to the GPU* — a strictly better mechanism
   for our scale. Not a gap; a superior deferral.

---

## 1. What our SDF rendering approach is, exactly

Brute-force, per-pixel, analytic sphere-tracing. No distance is cached between
pixels, steps, or frames.

Production marcher: `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`
(one compute thread per pixel, `main` ~:406). Shared field math:
`sdf_field.hlsli`.

GPU path, ray-gen → hit:
1. **Ray gen** — `generate_ray(...)` (`ray_gen.hlsli:44`), ortho/perspective, one
   ray per pixel, no beam/cone on the production path.
2. **Mesh-depth bound** — `gDepth.Load(...)` caps the march at the rasterized mesh
   depth `t_mesh` (the hybrid mesh↔SDF shared-depth occlusion).
3. **March loop** (~:497-551, `MAX_IT = 128`): each step calls `float d = sdf(p)` —
   **a full fold of the entire edit list** — then steps `t += d` (or `t += d*ω` with
   B1 over-relaxation), tests hit (`d < 0.001`), miss (`t > 10.0`), and mesh
   occlusion (`t >= t_mesh`).
4. **On hit**: normal via central differences `sdf_normal(p)` = **6 more `sdf()`
   calls**; `pick_material_id(p)` = another full per-edit primitive pass.
5. **Optional lighting**: A1 soft-shadow = **a second full 128-step march**; A2 AO =
   5 more field taps.

Edit-list traversal per step (`sdf_field.hlsli:132-146`): a flat `[loop]` over all
`n = min(count, 16)` edits — `load_edit` → `edit_distance` (sphere/box) → `combine`
(hard or polynomial-smooth boolean). **No early-out, no per-edit bound skip, no
sorting, no spatial structure** — every step always touches all `n` edits.

**Worst-case `sdf()` folds per fully-lit pixel** ≈ `128` (primary march) `+ 6`
(normal) `+ 128` (A1 shadow) `+ 5` (A2 AO) ≈ **267 full edit-list folds**, each
folding `n` edits. At `n = 16` ≈ **~4,300 primitive-distance evals per pixel**; at a
hypothetical `n = 256` ≈ **~68,000** — the wall the deferred brick cache exists to
remove.

---

## 2. Optimizations we currently use

| ID | Optimization | What it does | Where | Production-on? |
|----|--------------|--------------|-------|----------------|
| O-march | Hart sphere-tracing | Adaptive empty-space skipping (step by `d`) | `sdf_gbuffer_composite.hlsl:545` | Yes |
| O-mesh-bound | Mesh-depth march bound | Rasterized mesh depth caps the march; mesh↔SDF occlusion for free; early ray kill | `:434-436`, `:499-503` | Yes |
| O-B1 | Keinert/Bálint over-relaxation (ω-gated) | Steps `ω·d` (ω=1.2) with overshoot-retreat + provably hole-free plain re-march fallback | `:511-543`; `DEFAULT_MARCHER_OMEGA = 1.2` (`compute.rs:449`) | **Yes** |
| O-A1 | SDF cone-trace soft shadows | Quilez clamped-step penumbra; second 128-step march | `:314-333` | Optional (flag) |
| O-A2 | SDF 5-tap AO | 5 field taps along the normal | `:339-348` | Optional (flag) |
| O-P4b | 1/8-res coarse tile cone-cull pre-pass | Conservative `near_t`/`empty` per 8×8 tile; fine marcher skips empty tiles + seeds `t=near_t` | `sdf_tile_cull.hlsl`; consumed `:438-472` | **NO on-screen** — `swapchain.rs:~3552` sets `coarse_enabled=0`; only in the offscreen golden test |
| O-Lipschitz-L | √2 Lipschitz divisor for cone steps | Keeps cone/SOR steps conservative under super-Lipschitz smooth-min | `sdf_field.hlsli:196` | Yes (where cone/SOR used) |
| O-octnormal | Octahedral normal encoding | Packs world normal into RG8 of the G-buffer | `:362-369` | Yes |
| O9 (physics) | AVX2 batched-x8 edit eval | 8 field points/call for the **CPU physics narrowphase** — NOT the render path | `boyko_physics/src/sdf_simd.rs` | Yes (physics only) |

Clarifications: **O9 is a physics CPU-SIMD kernel, not a render optimization** (each
GPU lane is one pixel; edits are a scalar loop). **P4b tile-cull is built+tested but
OFF in production** — the on-screen path therefore has *no* active spatial/tile
acceleration. **There is no half-resolution rendering** — the production dispatch is
one thread per full-res pixel.

---

## 3. Cross-check against the reference-video techniques

For each: do we have it, and — if not — why. The shared reason is architectural and
explicit: we are at the analytic edit-list + sphere-trace layer; the video's
techniques are the cache-and-interpolate layers, all parked behind the
`field_distance` swap-point.

| # | Video technique | Have it? | Why / note |
|---|-----------------|----------|------------|
| a | Distance caching on a grid (eval once per grid point, reuse) | **No** | Genuine deferred win; the named first perf layer. The `field_distance` seam exists precisely for this. Highest-value structural change. |
| b | Bi/tri-linear interpolation of cached distances via one texture fetch (Valve-2007 / NVIDIA trilinear surfaces) | **No** | Deferred; depends on (a). `create_texture(D3)` RHI seam reserved but unused for SDF. |
| c | Sparse caching (only surface-straddling cells) | **No** | Deferred (the correct target at scale). Fits the engine's sparse/dense storage discipline. |
| d | Brick maps (dense pointer grid → 8³ bricks in a 3D atlas; chosen over octrees for GPU) | **No** | The single biggest deferred item ("P9"). The plan echoes the video's own reasoning (brick maps over octrees on the GPU). |
| e | 1-byte distance storage clamped to half the cell diagonal | **No** | A sub-detail of (d); `R8_UNORM`/`R16_SFLOAT` formats reserved but unused. |
| f | Clip-map / LOD (nested player-centered grids, 2× each, coarser far away) | **No** | Deferred *and currently inapplicable* — basic slice is a single near-field scene. Lowest near-term priority. |
| g | Incremental regeneration (re-eval only changed bricks) | **No** | Nothing to regenerate yet; the SSBO edit list is re-uploaded wholesale on change. Presupposes (d). |
| h | BVH/AABB tree over edits, shared CPU+GPU (raycasts, edit culling, brick-regen decisions) | **No** | Deferred. Arguably the highest-leverage *cheap* win once edits grow: per-edit AABB skip inside the fold. The B7-parked design is a relative of this (net-negative at n=16). |
| i | Mesh-for-physics-only (CPU marching-cubes collision mesh; render stays SDF) | **Partial / better mechanism** | We do NOT mesh. Physics samples the *same* analytic field on the CPU (zero readback, bit-identical to the GPU, O9-SIMD-accelerated). The video's *goal* (physics off the GPU field, no readback) is achieved by a superior mechanism for our scale. The marching-cubes hand-off is reserved for when the SDF becomes GPU-resident and large. |
| j | Fully-3D terrain via octaves of noise (Quilez), interacting with all edits | **No** | Primitive set is sphere + box only; no noise/fBm primitive. A future content feature, not a perf item. |

Net: of ten techniques, **zero implemented**, **one (i) solved by a better
mechanism**, the rest **deliberately deferred behind one frontier** (the P9 brick
atlas), none accidental.

---

## 4. Optimization opportunities, ranked (impact × feasibility)

1. **Turn on P4b coarse tile-cull in the windowed present** — *impact high, effort
   S, risk low.* Already built, tested, golden-proven conservative; the on-screen
   path hard-codes `coarse_enabled = 0`. Wire the existing `sdf_tile_cull` dispatch
   into the present loop + seed `near_t`. Cheapest real win in the repo: flip a flag,
   add one dispatch. The seam, buffer binding, and host mirror already exist.
2. **Over-relaxed sphere tracing** — *already shipped* (O-B1, ω=1.2). Keinert
   enhanced sphere tracing with the hole-free fallback. No action; noted for
   completeness (this is Tier-1 SOTA and we have it).
3. **Per-edit AABB cull inside the fold / finish the B7 Lipschitz prune** — *impact
   med→high as edits grow, effort M, risk low.* Today `sdf()` always folds all `n`
   edits. A cheap L∞ lower-bound or bounding-sphere skip drops far edits per step.
   **Conditional:** at `n=16` near contact surfaces this is plausibly net-negative
   (`docs/RENDER-B7-DESIGN-PARKED.md`); it becomes the right move the moment scenes
   exceed ~32–64 edits. Needs no new memory. The same per-edit AABBs feed incremental
   dirty-tracking later (one structure, two payoffs).
4. **GPU-resident 8³ brick atlas (P9)** — *impact very high at scale, effort L, risk
   med.* The video's (a)–(e) collapsed into one feature, swapped in behind
   `field_distance`: a dense pointer grid → 8³ bricks in a 3D-texture atlas, 1-byte
   narrow-band distances, trilinear reconstruction, empty-brick skip. Converts the
   per-step cost from `O(edits)` analytic fold to `O(1)` fetch. **On-thesis with
   Principle 0:** the brick store is exactly a "one contiguous buffer for all
   instances" case → a **dense GPU-resident ECS column**, not a side `Vec`. Hard part
   is the regen/incremental updater (item 6), not the fetch. Highest ceiling.
5. **Temporal march seeding / reprojection** — *impact high in steady state, effort
   M, risk med.* Reproject last frame's hit `t` into this frame's pixel; start the
   march there. With items 1 + B1 the steady-state march drops to a handful of
   iterations. Reuses the existing depth image + motion vectors; no new field
   machinery. Risk: disocclusion/ghosting.
6. **Incremental dirty-brick update + LBVH over edits** — *the subsystem that makes
   (4) viable for a modifiable world.* On edit change, dirty only the bricks whose
   AABB overlaps the changed edit's **new and previous** bounds (the #1 correctness
   bug is a missed dirty brick). Amortize re-eval across frames with a budget. The
   CPU edit-authority and GPU evaluator share one AABB layout — fits our existing
   CPU/GPU single-source-of-truth discipline.
7. **Cone-trace shadows/AO instead of the A1 128-step march** — *impact med, effort
   M, risk low.* A1 currently runs a full second 128-step march per lit pixel. A
   proper cone trace (or, post-P9, voxel-cone-trace over bricks) slashes shadow cost.
   The Lipschitz infra + `field_distance` seam already exist; consumer-side, low risk.
8. **Half-resolution SDF pass + upscale** — *impact med, effort M, risk med.* The
   plan's own cost ceiling assumes half-res but the shipped marcher is full-res.
   1/4-pixel marching + reconstruction is a classic 2–4× win. Risk: seam quality.
9. **Clip-map LOD (f)** and **noise terrain (j)** — *lowest near-term value.* Both
   inapplicable to the current single-near-field-scene, 16-edit scope. Correctly
   deferred until world extent / edit count grows.

Specialist / high-effort (payoff only at heavy CSG, dozens→thousands of edits):
**segment tracing** (local Lipschitz bounds, Galin 2020 — up to ×629 fewer field
queries, no accel structure, but invasive to the shared `boyko_sdf_math` field) and
**interval/affine step-skipping + Lipschitz pruning** (fractal/extreme-CSG fields).

Over-engineering for our goal (a dynamically-modifiable SDF world): **sparse voxel
octrees** (pointer-chasing divergence + costly dynamic restructuring — brick maps
dominate), **mesh-based collision (MC/DC)** for the in-house physics (re-meshing on
every edit defeats "modifiable"; SDF-direct is strictly better), and a **point-splat
renderer** (Dreams) (a whole alternative pipeline, orthogonal to our sphere-trace +
hybrid-mesh path).

---

## 5. Verdict

Architecturally we sit on the **analytic-base rung of a deliberately staged ladder**,
not at a dead end. Every march step re-folds the entire edit list
(`sdf_gbuffer_composite.hlsl:505`) over a flat scan with no acceleration
(`sdf_field.hlsli:132-146`), so it scales as `O(pixels × steps × edits)` and is
bounded today by `MAX_SDF_EDITS = 16`. But that is the **correct trade at this
scope**, and it buys real strengths the bricked engines give up:

- **Exact CSG** — infinitely sharp edges. Dreams / Claybook / UE all *round* corners
  to voxel size to get their cache; we keep them sharp.
- **One bit-identical field** shared by GPU render, the CPU render-golden, and CPU
  physics — determinism + a test oracle + zero-readback physics. A genuinely rare
  property.
- **Tier-1 SOTA marching already shipped** — B1 over-relaxation (Keinert), the
  mesh-depth hybrid bound, AO, soft shadows. The marcher is not naive.
- **Physics-from-SDF done better than the reference** — CPU analytic, zero readback,
  AVX2-batched, no per-edit re-meshing.

The cache-and-interpolate hierarchy is **parked behind the `field_distance`
swap-point — pre-cut, not missing**. It pays off only above the ~thousands-of-edits
regime; adopting it now would trade the sharp-CSG advantage for a problem we do not
yet have.

- **Where it will bind at scale**: the absence of an acceleration structure (no
  cache / brick / BVH). The one finished optimization currently left off on-screen is
  the P4b tile-cull (`coarse_enabled = 0` in the windowed present) — a ready lever,
  not debt.
- **Highest-value upgrades, by horizon**:
  - *Immediate (S, low-risk):* flip on the existing, golden-proven P4b tile-cull in
    the windowed present.
  - *Strategic (L):* the **P9 GPU-resident 8³ brick atlas behind `field_distance`,
    as a dense GPU column** — the video's distance-cache / trilinear / sparse / brick /
    1-byte techniques collapsed into one on-thesis feature — when the world outgrows
    ~thousands of edits. Pairs with the incremental dirty-brick updater + per-edit LBVH.

---

## 6. Industry reference points (SOTA)

| Aspect | Dreams (MM) | Claybook | Unreal GDF/Lumen | Pure analytic (Quilez) | boyko (current) |
|---|---|---|---|---|---|
| Field representation | Cached 8³ bricks, mega-texture volume | Cached 8³ SDF cubes, deformable | Atlas bricks + clip-maps | Analytic edit-list | **Analytic edit-list** |
| Per-pixel cost | 1 trilinear fetch (then splat) | trilinear fetch + adaptive trace | trilinear fetch + cone trace | `O(edits)` fold/step | **`O(edits)` fold/step** |
| Dynamic edits | Async-compute re-eval of affected regions | GPU re-eval, live deformable | Dirty object bounds / revealed slices | Re-fold (free) | Re-fold (free) |
| LOD | Multi-res point clouds | grid-res adaptive | 4 camera-centered clip-maps | none/repetition | none |
| Physics | n/a (editor) | SDF-direct, deformable | DF AO/shadows; mesh physics separate | n/a | **SDF-direct, CPU, zero-readback** |
| Sharp CSG edges | Rounded (painterly) | Rounded | Rounded | Exact | **Exact** |

**The load-bearing trade for us:** bricking caps detail at voxel size and **rounds
sharp CSG corners** — Dreams accepted this and leaned into a soft look. Our current
selling point is *exact* analytic CSG. A brick cache must consciously either accept
that rounding or keep an analytic fallback at edges. This single decision separates
"Tier-1 marcher tweaks only" from "go to a cached brick world."

### Key references
- Hart, *Sphere Tracing* (1996) — the baseline.
- Keinert et al., *Enhanced Sphere Tracing* (SCCG 2014) — over-relaxation (we have it).
  https://erleuchtet.org/~cupe/permanent/enhanced_sphere_tracing.pdf
- Galin et al., *Segment Tracing Using Local Lipschitz Bounds* (EG 2020) — up to ×629
  fewer queries, no accel structure.
  https://aparis69.github.io/public_html/projects/galin2020_Segment.html
- Söderlund, Evans, Akenine-Möller, *Ray Tracing of SDF Grids* (JCGT 2022) — analytic
  trilinear-interpolant intersection (don't naively sphere-trace a sampled grid).
  https://jcgt.org/published/0011/03/06/
- Evans (Media Molecule), *Learning from Failure* (SIGGRAPH 2015) — 8³ bricks + point
  splats. https://advances.realtimerendering.com/s2015/AlexEvans_SIGGRAPH-2015-sml.pdf
- Aaltonen (Second Order), *GPU-Based Clay Simulation… Claybook* (GDC 2018) —
  deformable SDF, SDF-direct collision.
  https://media.gdcvault.com/gdc2018/presentations/Aaltonen_Sebastian_GPU_Based_Clay.pdf
- Losasso & Hoppe, *Geometry Clipmaps* (SIGGRAPH 2004) — the LOD foundation.
  https://hhoppe.com/geomclipmap.pdf
- Crassin et al., *GigaVoxels* — SVO + brick pool.
  http://maverick.inria.fr/Membres/Cyril.Crassin/thesis/CCrassinThesis_EN_Web.pdf
- Barbier et al., *Lipschitz Pruning* (CGF 2025); Keeter, *Gradients Are the New
  Intervals* (2025); Duff, *Interval Arithmetic for Implicit Functions and CSG* (1992).
- Quilez, *Raymarching Distance Fields* / *Distance to Surfaces*.
  https://iquilezles.org/articles/raymarchingdf/
- Index: https://github.com/CedricGuillemet/SDF

### Recurring patterns the SOTA agrees on
- **8³ brick + dense/clip-map index + 1-byte narrow band** is the universal storage
  pattern for dynamic SDF (Dreams, Claybook, UE).
- **1-voxel apron** per brick — every brick stores its neighbors' boundary voxels so
  trilinear filtering never samples a missing neighbor across a seam (the single most
  common bricked-SDF bug).
- **Over-relaxation with overlap-check fallback** — cheapest sphere-trace speedup (we
  have it).
- **Cone/beam pre-pass at reduced resolution** — find a safe per-ray start.
- **AABB-per-edit + (L)BVH** — serves both per-step/per-tile edit culling *and*
  incremental dirty-brick detection (one structure, two payoffs).
- **Sphere-tracing a *sampled* grid over-shoots** — the sampled field is not
  1-Lipschitz between samples; intersect the trilinear interpolant analytically.

---

## 7. Source map (where to look)

- `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` — frozen field eval + the
  `field_distance` swap-point (the brick-backend seam).
- `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` — production per-pixel
  marcher.
- `crates/boyko_rhi_vulkan/shaders/sdf_tile_cull.hlsl` — built-but-on-screen-disabled
  P4b coarse cull.
- `crates/boyko_rhi_vulkan/src/swapchain.rs:~3540-3650` — windowed present wiring
  (where `coarse_enabled = 0` is set).
- `crates/boyko_rhi_vulkan/src/compute.rs:449` — `DEFAULT_MARCHER_OMEGA = 1.2` + host
  goldens.
- `crates/boyko_sdf_math/src/lib.rs` — the analytic field source of truth,
  `MAX_SDF_EDITS = 16`.
- `crates/boyko_physics/src/sdf_query.rs` + `src/sdf_simd.rs` — CPU analytic
  narrowphase (our answer to video technique (i)) + the O9 AVX2 x8 kernel.
- `docs/PHASE6-BASIC-SDF-RENDER-PHYSICS-PLAN.md:305-345` — deferred-subsystem table +
  the "analytic edit-list" decision.
- `docs/OPTIMIZATION-PLAN-RENDER.md` — the nine-track scaling roadmap.
- `docs/RENDER-B7-DESIGN-PARKED.md` — parked Lipschitz fold-prune + its
  net-negative-at-n=16 rationale.
