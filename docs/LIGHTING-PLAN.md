# Lighting + Baked 3D Maps (Static & Dynamic GI) — Plan

> Research + phased plan. This is an ORDERING of existing render-plan Track-A/E/G items plus new ECS-storage-backed phases — NOT a parallel roadmap. INVIOLABLE: all lighting data lives in our ECS (components/resources) + our GPU storage (`GpuColumnManager` DeviceLocal SSBOs / `VmReservation`); in-house only; the SDF field (`sdf_field.hlsli`) stays FROZEN — shadow/AO/GI rays are FIELD-CONSUMERS.

## Verified engine reality (what every phase attaches to)
- Lighting = deferred Cook-Torrance/GGX resolve (`deferred_pbr.hlsl`): D_GGX + height-correlated Smith V + Schlick F + Lambert + Karis `env_brdf_approx`. SDF (mask) pixels get full PBR; mesh/empty pass through.
- **Exactly ONE light, hardcoded constant** — `LIGHT_DIR=(0,0,1)`, white, plus `SKY_DIFFUSE/SKY_SPEC` constants. No light list, no per-light loop. (`deferred_pbr.hlsl:94-100,193,206`)
- A1 soft shadow = `gMaterial.r`, A2 AO = `gMaterial.g`, mask = `gMaterial.b>0.5`. (`deferred_pbr.hlsl:162-166,206,213`)
- G-buffer = 3 RGBA8 storage images + gLit; resolve set = 6 bindings (2 free under the 8 cap).
- Camera UBO @ binding 5 ALREADY carries `lighting_flags@8` + `light_dir@16` (marcher push widened 8→32 B). `DEFAULT_LIGHT_DIR`, `LIGHTING_FLAG_AO` exist. (`swapchain.rs`)
- A per-tile grid SSBO ALREADY exists (P4 scaffold): marcher binding 6 "Tiles", `tile_grid_extent`, STORAGE, written once/extent. (`swapchain.rs:2481-2499,3219-3243`) → the natural seed for the clustered light grid.
- Material table = `MaterialGpu` (48 B std430), uploaded once/on-change via `GpuColumnManager` → the template for `GpuLight`/probe PODs.
- GPU-storage model: `GpuColumnManager` mints DeviceLocal VRAM SSBOs behind opaque `DeviceColumnHandle` keyed by `(ArchetypeId, ComponentId)`; `RhiContext` is `!Send`; setup-time staging upload; ZERO per-frame readback. (`boyko_render/src/gpu_column.rs`)

**Key finding:** the engine is *one analytic light away from many* — G-buffer, deferred resolve, a lighting-capable camera UBO, and a per-tile grid buffer all already exist. "1→many" = render-plan P7 (clustered) + a new ECS light-component layer.

## Approaches (summary)
- **Multi-light:** deferred (have, 1 light) → **clustered/froxel** (P7, recommended): 3D grid (16×9×24), exponential Z slices, compute cull (sphere-vs-AABB) → light-index list + `{offset,count}` cluster grid; resolve loops only the pixel's cluster. (Bevy precedent: lights as ECS entities → storage-buffer light list.)
- **Shadows:** A1 SDF soft shadow (have, SDF half). Mesh side = CSM (render-plan S-CSM, lowest priority, needs D-FWD).
- **GI + baking (owner's core ask "запекание 3D карт для статичного/динамического"):**
  - Lightmaps — POOR fit (SDF has no UV chart). Deprioritize.
  - **Irradiance volume / probe grid (3D map) — RECOMMENDED**: cuboid grid, per-probe SH-L1 (~28 B) / SH-L2 (~112 B) / Valve ambient-cube (24 B). Bake once; dynamic objects trilinear-sample → static-scene bounce lights moving objects. Anti-leak via Chebyshev depth probes (DDGI) or SDF ray visibility.
  - DDGI = the SAME grid, updated at runtime (round-robin probe rays + temporal blend). Bake first, add update later.
  - **SDF-native GI (our advantage)** — trace the analytic field directly (SDFDDGI; AMD Brixelizer compute-only cascaded-SDF). Maps to render-plan A7/A8/A10/A12/A13 + G-SHARC. Plan verdict: software/SDF GI primary, HW-RT optional accelerator.
  - Bake pipeline = in-house GPU compute pass tracing the FROZEN field (+ direct lights) per probe → SH/ambient-cube → upload to a 3D image/SSBO; persist via `boyko_serialize`.

## PHASED PLAN (each phase: data in ECS + GPU via GpuColumnManager; field frozen)
- **L0 — Lights as ECS entities/components + GPU light table.** `DirectionalLight`/`PointLight`/`SpotLight` `#[derive(Component)]`; a `GpuLight` std430 POD table (mirrors `MaterialGpu`), uploaded once/on-change via `GpuColumnManager`. Replace the hardcoded `LIGHT_DIR` constant with a read from the light SSBO + count. Authoritative store = ECS columns; GPU table is a derived upload. Foundational, small. (RENDER class; 1-light golden stays in tolerance.) Owner call: light types + intensity units.
- **L1 = render-plan P7 — clustered light culling (1→many).** Compute pass builds per-cluster AABBs (exp-Z), culls the L0 table → index list + cluster grid; resolve loops the pixel's cluster. Cluster grid/index/AABB = `GpuColumnManager` SSBOs; extend the binding-6 tile SSBO 2D→3D; params = an ECS resource. (RENDER; deterministic given inputs.) Owner call: max-lights/index-list VRAM budget.
- **L2 = render-plan A10 baked — irradiance probe VOLUME (the owner's CORE ask).** 3D probe grid; in-house GPU compute bake traces the FROZEN field + L0 lights per probe → SH-L1/ambient-cube; resolve trilinear-samples → adds to the ambient term → lights DYNAMIC objects from static bounce. `IrradianceVolume` `#[derive(Component)]` (AABB+dims+`DeviceColumnHandle`); probe data = GpuColumnManager SSBO/3D-image; baked coeffs persisted via `boyko_serialize`. Anti-leak via Chebyshev/SDF visibility. (FIELD-CONSUMER; bake offline → outside the physics gate; GI-off path byte-identical = 0%-gate.) Owner calls: SH-L1 vs L2 vs ambient-cube; uniform vs adaptive placement; static-only vs +runtime; persistence; VRAM.
- **L3 = render-plan A10 full — runtime DDGI update.** Re-trace a slice of probes/frame (round-robin) + temporal blend → dynamic GI, SAME storage as L2. Needs P6-S history seam. (FIELD-CONSUMER + temporal → stochastic; gated by X-REF statistical bar.)
- **L4 = render-plan A7/A8/A12/G-SHARC — SDF-native GI capstones.** Cone-traced GI / Radiance Cascades / Brixelizer-class / spatial-hash radiance cache. Buffers = GpuColumnManager SSBOs. After L2/L3 (quality capstones).
- **L5 = render-plan S-CSM — mesh CSM.** Only if meshes become first-class shadow casters (needs D-FWD). Lowest priority for an SDF-native engine.

### Order
L0 → L1(P7, many lights) → L2(A10 baked volume = owner core) → {L3 runtime DDGI | L4 SDF GI capstones}; L5 only if meshes must cast.

## Owner VALUES/SCOPE calls to escalate
Light model + intensity units (L0); probe encoding SH-L1/L2/ambient-cube (L2); probe placement uniform-vs-adaptive (L2); static-bake vs runtime DDGI (L2↔L3); bake persistence via boyko_serialize (L2); do meshes cast shadows (L5); GI before/after the P9 brick atlas (L4); VRAM budget split across index-list + cluster grid + probe volume(s) on the 6 GB 3060.

## Open architect questions
- L2 probe buffer: 3D image (hw trilinear) vs flat SSBO (matches GpuColumnManager's SSBO-only path; a 3D-texture target is a new RHI capability — format-gate pattern). 
- L1: reuse/extend the binding-6 tile SSBO (2D→3D) vs a separate cluster grid.
- Light-table collection: setup-time vs `Changed<Light>`-driven upload.

## Primary sources
Olsson et al. *Clustered Deferred/Forward Shading* HPG2012; Majercik et al. *DDGI* JCGT2019; Hu et al. *SDF DDGI* 2020/2021; O'Donnell *Precomputed GI in Frostbite* GDC2018; Wijetunga *Brixelizer GI* GDC2024; aortiz clustered-shading primer; Granite clustered evolution; Bevy lights-as-entities (PR#3989) + irradiance_volume; RTXGI DDGI Algorithms (Chebyshev anti-leak); MS CSM. (Full annotated list in the research transcript.)
