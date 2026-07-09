# Shader `-D` Variant Manifest

**Single source of truth for the monolithic `-D`-preprocessor shader variants** that *cannot*
collapse to one `.spv` (they change the descriptor set / capability / emitted code), so they stay
**N `.spv` compiled from ONE `.hlsl`**. This is the A-3 half of the shader-growth remediation (see
[REFACTORING-PLAN.md](REFACTORING-PLAN.md) §A3): the registry de-dup (A-2 `embed_spirv!` macro) removed
the hand-counted embed sizes, and this table removes the *scavenger hunt* — a new variant is a row here,
not archaeology through `compute.rs` doc-paragraphs.

Contrast with the **spec-constant-collapsible** families (`GI_MAX_IT` — SHIPPED @3c10826) and the
**perf-justified** ones (SSAO quality — `[unroll]`, kept as 3 files; see the plan). A `-D` variant belongs
HERE only if it changes the *interface* (adds/removes a binding, a capability, or half the shader) — a
spec-constant cannot do that.

## The three axes

| Axis (`-D`) | Values | What it changes |
|---|---|---|
| `SHADOW_STAGE` | `RESOLVE_INLINE` (unset/0), `VIS` (1), `DENOISED` (2) | How the mesh shadow term is produced/consumed. INLINE combines the trace into lighting directly; VIS **writes** `gShadowVis` (the à-trous/temporal pre-pass) and strips lighting; DENOISED **reads** the final denoised `gShadowVis` and combines it. |
| `HWRT` | unset (0), `1` | 0 = software SDF-marched mesh shadows (no ray HW). 1 = hardware inline `rayQuery` against a TLAS — adds `OpCapability RayQueryKHR`, the acceleration-structure descriptor, and **requires `-T cs_6_5`** (vs `cs_6_0`). Gated by `feature = "hwrt"` + a runtime `ctx.ray_query_enabled()`. |
| `MOTION_VECTORS` | unset (0), `1` | 0 = no motion output. 1 = also emit per-pixel motion (Δuv) for the temporal shadow denoiser — a new storage binding (deferred) or a 4th MRT (gbuffer). Static camera ⇒ (0,0). |

## `deferred_pbr.hlsl` — the fullscreen deferred resolve (compute)

One source `shaders/deferred_pbr.hlsl`; the host selects a variant by **binding a different pipeline**,
never a dynamic branch. All share the base 0..11 binding block (G-buffer STORAGE images, material SSBO,
camera UBO, light table, cluster grid, SDF edit-list, SSAO). The deltas:

| Variant | `SHADOW_STAGE` | `HWRT` | `MV` | `.spv` | dxc `-T` | Interface delta vs base (0..11) |
|---|---|---|---|---|---|---|
| RESOLVE_INLINE (software) | — | — | — | `deferred_pbr.comp.spv` | `cs_6_0` | none — software SDF `sdf_soft_shadow_ranged`; +CSM `gCsm`/`gCsmCmp` @12/13/14 + punctual atlas + (bound-unread) DDGI @16/17/18. |
| RESOLVE_INLINE (hardware) | — | `1` | — | `deferred_pbr_hwrt.comp.spv` | `cs_6_5` | `+RaytracingAccelerationStructure gTlas` @19 + `OpCapability RayQueryKHR`; the `#if HWRT` Vogel-disk cone trace (`SHADOW_RAY_COUNT` spec-const) replaces the software march. |
| VIS | `1` | `1` | — | `deferred_pbr_hwrt_vis.comp.spv` | `cs_6_5` | hwrt layout **+ `RWTexture2D<float2> gShadowVis`** @21 (**write** `RG(mesh_vis, validity)`); lighting stripped (writes vis, not lit). The à-trous/temporal pre-pass. |
| DENOISED | `2` | `1` | — | `deferred_pbr_hwrt_denoised.comp.spv` | `cs_6_5` | same 22-binding VIS/DENOISED layout, but `gShadowVis` @21 is **read** (`mesh_vis = gShadowVis.Load().r`, the final denoised output) and combined `vis = min(vis, mesh_vis)`. Declares NO acceleration structure. |
| VIS + motion | `1` | `1` | `1` | `deferred_pbr_hwrt_vis_mv.comp.spv` | `cs_6_5` | VIS layout **+ `MotionCam` UBO** @22 **+ `RWTexture2D<float2> gMotionVec`** (`rg16f`, SIGNED) @23 — writes clip-space Δuv for the temporal reproject. |

Reachability note: `SHADOW_STAGE ∈ {VIS, DENOISED}` and `MOTION_VECTORS=1` are only reachable **with**
`HWRT=1` (the spatial/temporal shadow-vis denoise pipeline is built on the hardware mesh-shadow trace);
there is no software VIS/DENOISED/MV `.spv`.

## `gbuffer_mrt.{vs,fs}.hlsl` — the mesh G-buffer raster

| Variant | `MV` | `.spv` | Interface delta |
|---|---|---|---|
| base | — | `gbuffer_mrt.vs.spv` / `gbuffer_mrt.fs.spv` | 3 MRT attachments (albedo / normal+id / material). |
| motion | `1` | `gbuffer_mrt_mv.vs.spv` / `gbuffer_mrt_mv.fs.spv` | **+ a 4th MRT** carrying per-pixel Δuv (prev-instance ring @1 + `MotionCam` UBO @2); static instance+camera ⇒ (0,0). |

## Shadow-denoise compute (separate shaders, not `-D` variants of the resolve)

These are distinct `.hlsl`, listed here for the temporal/spatial pipeline picture, not because they are
`-D` variants: `shadow_atrous.comp` (spatial edge-stopping à-trous over `gShadowVis`) and
`shadow_temporal.comp` (reproject + variance-clamp + accumulate against the cross-frame history pool).
The mode matrix (None / Spatial / Temporal / Both) is a **host** selection of which of these passes run
between the VIS producer and the DENOISED consumer — see `BOYKO_SHADOW_DENOISE`.

## Why these stay N `.spv` (do NOT try to spec-const-collapse)

A specialization constant is resolved at *pipeline-create* and can only change a **value** (a loop bound,
a count). It cannot: add or remove a descriptor-set binding (VIS's `gShadowVis`, HWRT's TLAS, MV's
`gMotionVec`), add a SPIR-V capability (`RayQueryKHR`), or delete half a shader (VIS strips lighting).
Every variant here does exactly one of those, so it is a genuine separate module. The good pattern is
already in place: **one `.hlsl`, N `.spv` via `-D`, ONE embed macro per `.spv`** — the source is
single, only the compiled artifact multiplies, and each artifact is a row above.

## Adding a new variant — checklist

1. **Shader**: guard the new code with `#if <FLAG>` in the existing `.hlsl` (never fork the file).
2. **Recipe**: compile offline with the frozen recipe + your `-D <FLAG>=<v>` (and `-T cs_6_5` if it needs
   `rayQuery`); commit the new `.spv` next to its siblings.
3. **Embed**: add ONE `embed_spirv! { /// doc … [#[cfg(feature = "hwrt")]] NAME_SPV, concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/<file>.spv") }` in `compute.rs` + a `pub fn <name>_spirv()` accessor. (No hand-counted size — the macro derives it.)
4. **Layout**: if the variant adds/removes a binding, add its pipeline-layout arm; keep the binding
   numbers consistent with the table above.
5. **Host**: select it by binding the right pipeline for the mode — never a runtime uniform branch.
6. **Row**: add it to this table. Gate byte-identity via the golden (`58f6c6c3`, GI-OFF) + the relevant
   `*_edsl_sync` re-emit test if the body is eDSL-generated.
