# Architecture: Lighting L0 + L1 (ECS Light Entities → GPU Light Table → Clustered Multi-Light Resolve)

> FINALIZED implementation reference. This document is self-contained and verbatim-complete:
> the `developer` and `GPU-tester` follow it directly. It supersedes the L0/L1 rows of
> [docs/LIGHTING-PLAN.md](LIGHTING-PLAN.md) with the concrete data structures, std430
> offsets, shader edits, descriptor wiring, rung split, per-rung GPU goldens, and 0%-gates.
>
> **INVIOLABLE constraints (carried from the engine principles + [CLAUDE.md](../CLAUDE.md) principle 0):**
> 1. All lighting data lives in the ECS (components/resources) + our GPU storage
>    (`GpuColumnManager` DeviceLocal SSBOs on `VmReservation`). **No parallel `std::Vec`/`HashMap`
>    light store.** The authoritative store is ECS columns; the GPU table is a *derived upload*.
> 2. The SDF field (`sdf_field.hlsli`) stays **FROZEN** — shadow/AO/GI/light-visibility rays are
>    strict FIELD-CONSUMERS that CALL `field_distance`; they never edit the field math.
> 3. **In-house only** (no external render/light libraries). Hand-FFI Vulkan via the existing RHI.
> 4. **0%-gate**: the lighting-OFF / single-default-light degenerate path is **byte-identical** to
>    today's golden output. Every rung carries its own 0%-gate proof.
> 5. **Deterministic**: given identical inputs (light table + camera + field), the resolve output is
>    bit-reproducible; the host oracle (`compute.rs`) models the GPU math identically (consumer-side
>    terms within the existing ±2/255 .. ±3/255 tolerance, OFF path bit-exact).

---

## Goal

Turn the engine from **one hardcoded analytic directional light** (`LIGHT_DIR=(0,0,1)`, white,
compiled into `deferred_pbr.hlsl:94`) into **many ECS-authored lights resolved through a clustered
froxel cull**, with zero new heap allocations on the frame path and zero regression on the existing
single-light golden.

- **L0** — Lights become ECS entities/components; a GPU `GpuLight[]` table (std430 POD, mirrors
  `MaterialGpu`) is uploaded once / on-change via `GpuColumnManager`. The resolve reads the table +
  a small header (count, ambient, exposure) instead of the compiled-in constant.
- **L1** (= render-plan **P7**) — A compute pass builds per-cluster froxel AABBs (16×9×24,
  exponential-Z), culls the L0 table (sphere-vs-AABB) into a per-cluster `{offset,count}` grid + a
  flat light-index list; the resolve loops **only the pixel's cluster's** lights.

### Target performance metrics

| Metric | Target | Rationale |
|---|---|---|
| Frame-path allocations (L0 + L1) | **0** | Light table + header + cluster grid + index list are device SSBOs minted once at setup, grown only on capacity cross (setup-class). |
| Per-on-change CPU cost (L0) | O(live lights) memcpy into the mapped staging buffer + set a dirty flag; the per-frame recorder emits a `cmd_copy_buffer` + barrier (C3 — NO fence-wait, NO stall) | `Changed<Light>`-gated; idle frames record nothing. |
| Resolve cost per pixel (L1) | O(lights in this pixel's cluster), typ. 1–8 | vs O(all lights) brute force; the froxel cull is the whole point of P7. |
| Cluster-cull dispatch (L1) | 1 compute dispatch, `16*9*24 = 3456` froxels, ≤ `MAX_LIGHTS` sphere tests each | runs before the marcher's resolve; reads the L0 table + camera UBO. |
| L0 directional/sky 0%-gate | **byte-identical** to today | 1-light default table reproduces the compiled-in constant exactly. |
| L1 OFF 0%-gate | **byte-identical** to L0 | `clusters_enabled == 0` → resolve loops the flat table `[0..count)` (same as L0). |
| Header size | **64 B** (one cache line, std430 4×vec4) | see Decision 3; fits with `exposure` added (O3). |
| `GpuLight` size | **48 B** (3×vec4 std430) | exactly mirrors `MaterialGpu`; one L2-resident table. |

---

## Context and constraints

### Verified engine reality (every decision attaches to a read-confirmed fact)

- **Deferred resolve** = `deferred_pbr.hlsl` (Cook-Torrance/GGX: `D_GGX` + height-correlated Smith
  `V` + Schlick `F` + Lambert + Karis `env_brdf_approx`). SDF (mask==1) pixels get full PBR; mesh /
  background / empty (mask==0) pass `base` through byte-identically (the existing 0%-gate). The
  single light is the compiled-in `LIGHT_DIR`/`LIGHT_COLOR` constant (`deferred_pbr.hlsl:94-95,193,206`),
  plus `SKY_DIFFUSE`/`SKY_SPEC` ambient constants (`:99-100,211-212`).
- **Resolve descriptor set (set 0)** = **6 bindings; the binding cap is raised 8 → 12 in this plan
  (see Decision 1 / C1), so the resolve starts at 6 with ample headroom**
  (`swapchain.rs:3529-3540`): `@0` gAlbedo (storage), `@1` gNormal (storage), `@2` gMaterial
  (storage), `@3` gLit (storage), `@4` material SSBO, `@5` camera UBO. The cap is a self-imposed
  engine inline-array size (`MAX_BIND_GROUP_BINDINGS`), NOT a hardware limit — see C1.
- **Ray reconstruction already exists in the resolve**: `deferred_pbr.hlsl:190` calls the SHARED
  `generate_ray` (`ray_gen.hlsli`) → `ro`,`rd`. View dir `v = -rd` (`:191`). ORTHO: `rd=(0,0,-1)`,
  `ro=(u*1.0, v*1.0, 2.0)`. PERSP: `ro=cam_eye`, `rd=normalize(dir)`.
- **The surface world position `P` is NOT currently available to the resolve.** The marcher
  (`sdf_gbuffer_composite.hlsl`) computes the exact hit param `t` and `p = ro + rd*t` (`:588-590`)
  but writes only gAlbedo/gNormal/gMaterial — **`t` is never exported**. `gMaterial.a` is hardcoded
  `1.0` (`:625`) — an 8-bit lane, useless for world-space `t`. The mesh `gDepth` (D32_SFLOAT) is
  bound only to the **marcher's** vocab set (`@1`, `swapchain.rs:3495`), not the resolve set, and it
  carries *mesh* depth, not the *SDF surface* `t`. **→ This is the entire substance of O1; see Decision 1.**
- **Camera UBO @5** (`swapchain.rs`, `deferred_pbr.hlsl:70-79`) carries `count/img_w/img_h/camera_mode`
  + 4 `float4` basis lanes (eye, forward[.w=tan(fovY/2)], right[.w=aspect], up). 80 B, written once.
  This is the source of `ro`/`rd` and (L1) of the froxel-AABB view-space reconstruction.
- **A per-tile grid SSBO already exists** (P4 scaffold): marcher binding `@6` "Tiles"
  (`StructuredBuffer<TileBound>`, 16 B each), `tile_grid_extent(w,h)` → `tiles_w*tiles_h`,
  `TILE_SIZE=8` (`compute.rs:1906`), written once/extent. This is the **2D** seed; L1 extends the
  concept to a **3D** froxel grid (Decision 6) — a NEW cluster grid SSBO, not a mutation of `Tiles`.
- **`MaterialGpu`** (`boyko_render/src/material.rs`) = `#[repr(C, align(16))]`, 48 B / 3 clean vec4
  lanes, const-assert layout fingerprint (size/align/every offset), `MATERIAL_GPU_WORDS==12` pinned
  against the shader. The `material_table` is **host-seeded ONCE before the present loop**
  (`swapchain.rs:3258`) and never re-uploaded on change — so it is the POD-layout template for
  `GpuLight` + the header, but **NOT** a precedent for the dynamic on-change re-upload (see C3 /
  Decision 4). **This is the exact layout template for `GpuLight` + the header POD.**
- **`GpuColumnManager`** (`boyko_render/src/gpu_column.rs`) mints DeviceLocal VRAM SSBOs behind
  opaque `DeviceColumnHandle` keyed by `(ArchetypeId, ComponentId)`; staging upload; **ZERO per-frame
  readback**. Note both its upload entry points fence-wait: `upload_initial` (`:833`) is documented
  SETUP-only and fence-waits before return (`:863-866`); `dispatch_compute` (`:576`) is a recorded +
  fence-waited dispatch (`:571`, fence at `:610`). So neither is a fence-free async path — the C3
  on-change re-upload is a NEW recorded-copy capability (rung L0-r0). `RhiContext` is `!Send + !Sync`
  (owns `VulkanContext`), `impl NonSendResource`.
- **The light SSBO is a SINGLETON, not a per-archetype column.** It mirrors how `material_table` is a
  scene-global `BoundBuffer` (`swapchain.rs:3262`, bound to both vocab `@7` and resolve `@4`), not a
  per-archetype `GpuColumnManager` column. So L0's light table is a **scene-global SSBO** created at
  setup exactly like `material_table` (Decision 7). The FIRST seed reuses `upload_initial`; **the
  on-change re-upload is the recorded async copy + barrier (C3 / L0-r0), NOT the fence-waited setup
  path.** (`GpuColumnManager` is the *uploader*; the *handle* is a scene resource, not a
  `(ArchetypeId,ComponentId)` pair.)
- **Host oracle** = `compute.rs` (`golden_deferred_resolve`, `host_shade`, `golden_composite_pixel_ex`,
  `composite_ray`) models the GPU math; const-assert fingerprints pin every POD offset; `FineMarcherPush`
  already carries `lighting_flags`/`light_dir` (`compute.rs:1067+`). The light model elevation moves
  the light *data* out of the push/constant into the SSBO; the push keeps only the A1/A2 marcher flags.

### Subsystems affected

`boyko_render` (new `light.rs`, `LightSet` collection system, header resource); `boyko_rhi_vulkan`
(`deferred_pbr.hlsl` resolve rewrite, new `cluster_cull.hlsl`, `swapchain.rs` resolve-set wiring,
`compute.rs` host oracle + POD fingerprints); `boyko_ecs` (no core changes — lights are ordinary
`#[derive(Component)]` types + a resource).

### Invariants to preserve

- Frozen `sdf_field.hlsli`. Frozen marcher field-eval ordering. The existing single-light golden
  output (byte-identical at the L0-default / L1-OFF degenerate). The 12-binding descriptor cap
  (raised from 8 in this plan; see Decision 1 / C1). The std430/`repr(C)` fingerprint discipline.
  `RhiContext` `!Send`. No frame-path allocation.

---

## Key decisions

### Decision 1 (O1 RESOLVED): how the resolve reconstructs the surface world position `P`

**The problem (precisely).** Point/spot attenuation needs `posToLight = L.pos - P`, i.e. the surface
**world position** `P`. The resolve already has `ro`,`rd` (shared `generate_ray`), so
`P = ro + rd * t` — but it has **no per-pixel `t`**. The marcher knows the exact `t` (it marched to
it) and never exports it. There is **no spare high-precision lane** in the current G-buffer:
gMaterial.a is an 8-bit `1.0`, and the existing `gDepth` is mesh-only and on the wrong descriptor set.

**Decision.** Add a **new dedicated R32_SFLOAT G-buffer lane `gViewT`** (full fp32 ray parameter `t`)
that the marcher *already-computed* value is stored into, and the resolve binds + reads to reconstruct
`P = ro + rd * t` with the SHARED `generate_ray`. This is a **G-buffer ADDITION (a 4th attachment),
not a LAYOUT CHANGE** to the three existing RGBA8 targets — so any pass that does not read `gViewT`
is byte-unchanged.

Why R32_SFLOAT and not a packed lane:
- `t` ranges `[0, T_MAX=10]` in ORTHO and is an unbounded perspective ray length; world `P` precision
  must be ≤ sub-millimetre at scene scale for attenuation/spot-cone math not to band. An 8-bit lane
  (gMaterial.a) gives ~0.04 absolute error in `t` → visible attenuation stairstepping. fp32 is exact
  to the marcher's own `t`.
- Storing `t` (one scalar) and reconstructing `P` in-shader is **cheaper in bandwidth** than storing
  `P` (a float3 → RGBA32F, 16 B/px): `t` is 4 B/px and `P` reconstruction is one `mad` the resolve
  already does for `v=-rd`. (Bevy/DOOM-2016-style deferred reconstruct-from-depth precedent; we reuse
  our analytic ray instead of an inverse-projection because we *have* the exact ray-gen.)

Exact bind + reconstruction:
- **Marcher** (`sdf_gbuffer_composite.hlsl`): new storage image `gViewT : register(uN)` declared
  `[[vk::image_format("r32f")]] RWTexture2D<float>`. **`gViewT` MUST be written on EVERY terminal
  exit, exactly once per pixel per frame (C2).** The marcher has THREE terminal write sites across
  TWO return points — the prior plan, which wrote `gViewT` only at the final block, would MISS the
  EMPTY-tile early return:
  1. **P4b EMPTY-tile fast path** (`:434-444`): writes gAlbedo/gNormal/gMaterial then `return;` at
     `:444` — a SEPARATE early return BEFORE the final block. Add `gViewT[uint2(px,py)] = 1.0e30;`
     (sentinel) at `:441-444`, BEFORE the `return;` at `:444`.
  2. **SDF-hit arm** (`if (hit && t < t_mesh)`, `:588-590`): computes `float3 p = ro + rd * t;` — the
     real exact `t`. This `t` is the value to store on the lit arm of the final block.
  3. **Final write block** (`:623-625`): the mesh arm (`:615`) and background arm (`:617`) fall
     through here and write gAlbedo/gNormal/gMaterial unconditionally. Add
     `gViewT[uint2(px,py)] = (mask == 1.0) ? t : 1.0e30;` — the real marched `t` on the SDF-lit arm,
     the **sentinel `1.0e30`** on mesh/background. (`t` is in scope at the final block; if HLSL
     scoping requires it, hoist a `float view_t = 1.0e30;` and set `view_t = t;` inside the SDF arm,
     then write `view_t`.) The sentinel is `1.0e30` (not `t_mesh`/`T_MAX`) so a stray read is a far
     miss, not a black hole — but it is **never read on a non-lit pixel** (see the read-under-mask
     gate below). This is `gMaterial.a`'s would-be free lane replaced by a *real* fp32 image — no
     change to gAlbedo/gNormal/gMaterial bit layout.
- **Resolve** (`deferred_pbr.hlsl`): bind `gViewT` at the **first free resolve slot `@6`** (≤ the
  12-binding cap with headroom — see C1). **Read-under-mask gate (C2): `gViewT` is read STRICTLY
  inside the `is_sdf_lit` / `mask == 1` branch** (the only branch that does point/spot, and the L1
  cluster froxel-z computation) — so a stale/sentinel value on a non-lit pixel is NEVER consumed:
  ```
  float t = gViewT.Load(coord);          // r32f, exact marcher param — inside mask==1 only
  float3 P = ro + rd * t;                 // ro,rd already from generate_ray (line 190)
  // per point/spot light: posToLight = L.pos - P; ...
  ```
- **R32_SFLOAT storage support (W2):** before creating the `gViewT` image, fail-fast at device-caps
  validation. Extend `DeviceCaps` (`boyko_rhi_vulkan/src/device.rs:1838-1841`, today
  `{ bindless_capable, gbuffer_storage_format_ok }`) with `viewt_storage_format_ok: bool`, and extend
  `query_device_caps` (`:1819-1842`) to query `VK_FORMAT_R32_SFLOAT` for
  `VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT` via the same `get_physical_device_format_properties` call on
  OPTIMAL tiling, mirroring the existing `gbuffer_storage_format_ok` pattern EXACTLY (which today
  checks only `R8G8B8A8_UNORM`). Fail-fast at the same site/discipline as `gbuffer_storage_format_ok`
  so the new R32_SFLOAT lane keeps the fail-fast discipline. This lands in rung L0b (when `gViewT` is
  introduced).
- **Descriptor wiring** (`swapchain.rs`): `gViewT` is a 4th `create_gbuffer_image` (format R32_SFLOAT,
  STORAGE usage), transitioned UNDEFINED→GENERAL alongside the other three (`:2443` loop gains it),
  added to the marcher vocab set (new `@8` — the vocab ALREADY has 8 entries `@0..@7`, so this 9th
  entry would have OVERFLOWED the prior 8-cap; the cap is raised to 12 in this plan, see C1 below, so
  `@8` is now ≤ 12 with headroom), and added to the resolve set as `@6`. The store→load barrier loop
  (`:2539`) gains `gViewT`.

**C1 resolution (the binding cap is raised 8 → 12).** The prior plan claimed the marcher vocab's
`gViewT` would be `@8` and "still ≤ cap" — this was FALSE. **The marcher vocab set
(`swapchain.rs:3493-3506`) ALREADY has 8 entries:** `@0` edit_list SSBO, `@1` depth sampled, `@2`
albedo storage-img, `@3` normal storage-img, `@4` material storage-img, `@5` camera UBO, `@6` Tiles
SSBO (P4b coarse-cull), `@7` material_table SSBO. So adding `gViewT` as a 9th binding OVERFLOWS the
current cap of 8. The cap `MAX_BIND_GROUP_BINDINGS = 8` is declared TWICE (agnostic
`boyko_rhi/src/device.rs:22` and the backend mirror `boyko_rhi_vulkan/src/rhi_impl.rs:74`,
const-asserted equal at `rhi_impl.rs:80`), and enforced by debug_assert `(1..=MAX_BIND_GROUP_BINDINGS)`
(`rhi_impl.rs:782, 902`), release clamp `count.clamp(1, MAX_BIND_GROUP_BINDINGS)` (`:788, :913`), and a
binding-index assert (`:797`). It is a **self-imposed engine inline-array size, NOT a hardware limit**:
the device limit re-verified on NVIDIA Ampere / RTX 3060 is `maxPerStageDescriptorStorageImages =
1048576` and `maxPerStageResources = 1048576` (Vulkan Hardware Database / NVIDIA driver), so raising
the cap is safe.

**Decision (option a — raise the cap, clean and contained): set `MAX_BIND_GROUP_BINDINGS = 12` in BOTH
`boyko_rhi/src/device.rs:22` and `boyko_rhi_vulkan/src/rhi_impl.rs:74`** (the const-assert at `:80`
keeps them in lockstep). **Why 12, not 9:** 12 hits every need on a SINGLE set with headroom —
resolve L0a = 7 (6 existing + `light_table`), L0b = 8 (+`gViewT`), L1 = 10 (+`cluster_grid`
+`light_index`); marcher vocab L0b = 9 (8 existing + `gViewT`). 12 leaves 2 free for a future
shadow/probe lane and keeps L1's resolve inputs on ONE set (no second-set multi-bind juggling). All
≤ 12 ≪ 1048576. This RESOLVES the old "@8 still ≤ cap" falsehood and OQ-1 (no second descriptor set
needed for L1 — see OQ-1, now RESOLVED).

**Exhaustive list of what the cap change touches** (the critic demanded the full enumeration):
  1. The two literal consts (`boyko_rhi/src/device.rs:22`, `rhi_impl.rs:74`) + the equality
     const-assert (`rhi_impl.rs:80`).
  2. The layout struct field `entries: [BindGroupLayoutBinding; MAX_BIND_GROUP_BINDINGS]`
     (`rhi_impl.rs:477`) — already parameterized, scales automatically.
  3. The inline arrays in `create_bind_group_layout` (`rhi_impl.rs:803`, `:817`) and in
     `create_bind_group` (`:999`, `:1004`, `:1005`) — all `[_; MAX_BIND_GROUP_BINDINGS]`, scale
     automatically.
  4. The debug_asserts (`:782, :797, :902`) and release clamps (`:788, :913`) — all expressed via the
     const, scale automatically.
  5. The agnostic const's docstring at `boyko_rhi/src/device.rs:17-22` (update the "Sized for ... with
     headroom" wording to reflect 12 + the `gViewT`/light-table additions).
  **NOT TOUCHED (proves containment):** the descriptor-pool histogram (`rhi_impl.rs:918-943`) is keyed
  on `KIND_COUNT = 5` (descriptor KINDS: uniform/storage-buffer/sampled/storage-image/sampler), NOT on
  the binding cap, so it is unaffected; `pool_sizes` is `[_; KIND_COUNT]`, `max_sets: 1`. There are NO
  other `[_; 8]` literal arrays tied to the cap (verified by grep — every site uses the named const).
  So the change is exactly: **edit 2 const literals + 1 docstring; everything else scales.**

**Consequence — the rung split (this is the load-bearing outcome of O1).** Directional + Sky/ambient
have **NO `P` dependency** (directional `L` is a constant direction; sky is the analytic ambient).
They ship FIRST on the **existing 6-binding resolve set**, hitting the 0%-gate with no G-buffer touch.
Point + Spot need `gViewT` wired and therefore land in a SEPARATE later rung. The rungs are:

- **Rung L0a (Directional + Sky, multi-light, NO P)** — resolve reads the `GpuLight[]` table + header,
  loops the directional/sky lights, **no `gViewT`, existing 6-binding set unchanged**. 0%-gate: a
  1-entry default table == the compiled-in constant.
- **Rung L0b (Point + Spot, adds `gViewT`)** — add the R32_SFLOAT lane to the marcher + resolve;
  resolve reconstructs `P` and adds point/spot loops. 0%-gate: a table with zero point/spot lights
  produces byte-identical output to L0a (the point/spot loop bodies are skipped).
- **Rung L1 (clustered cull)** — froxel cull pass + per-cluster loop; OFF path == L0b.

**Final O1 decision (definitive):** Point/spot **DO** require a G-buffer touch (a new R32_SFLOAT
`gViewT` lane), because no existing resolve-visible high-precision source for the SDF surface `t`
exists. Therefore **Directional + Sky ship first (L0a, 0%-gate, no G-buffer change)** and **Point +
Spot follow (L0b) once `gViewT` is wired**, reconstructing `P = ray_origin + ray_dir * t` from the
already-shared `generate_ray` plus the new fp32 `t` lane.

**Alternatives rejected.**
- *Reuse `gMaterial.a` (8-bit) for `t`*: precision-fatal (~0.04 in `t` → mm→cm world error,
  attenuation banding). Rejected.
- *Store full `P` in an RGBA32F lane*: 16 B/px vs 4 B/px, no quality gain over reconstruct-from-`t`
  (the ray is exact). Rejected on bandwidth.
- *Inverse-project mesh `gDepth` (already exists)*: it is on the wrong descriptor set, is mesh-only
  (SDF pixels have no rasterized depth), and would need a projection matrix the camera UBO does not
  carry. Rejected — our analytic ray is strictly better and already present.
- *Recompute `t` in the resolve by re-marching the field*: re-marches the FROZEN field per pixel in
  the resolve = doubles the most expensive work + risks a field-eval drift vs the marcher. Rejected.

**Trade-off.** +4 B/px VRAM (one R32_SFLOAT target) and +1 descriptor binding on two sets (within the
raised 12-binding cap — see C1). Paid only from L0b onward; L0a is free.

### Decision 2 (O2 ADOPTED): reflector spot intensity model

**What.** Spot luminous intensity `I = Φ / (2π (1 − cos(θ_outer)))`, where `Φ` is the spot's luminous
power (lumens, authored) and `θ_outer` is the outer cone half-angle. The angular falloff between inner
and outer cone is the standard smooth `t = saturate((cosθ − cos_outer)/(cos_inner − cos_outer))`,
`atten_angle = t*t` (squared for a soft edge). Radiant contribution
`= I * angular * (1/d²) * NoL`, with `d = length(L.pos − P)`.

**Why.** `Φ/(2π(1−cos))` is the *reflector* normalization: it conserves authored power across cone
width (a narrower cone is brighter for the same lumens), which is the physically meaningful authoring
control and what the exposure scalar (O3) then maps to display range. It needs only `Φ` + the two cone
cosines in the POD — no extra LUT, no per-light precompute on the GPU.

**Alternatives rejected.** The *absorber* model (`I = Φ/(4π)`, a point-source normalization ignoring
the cone) was the alternative — dropped per O2: it makes cone width not affect brightness, which is
unintuitive for authoring and wastes the cone parameters. Rejected.

**Trade-off.** `(1−cos(θ_outer))` → 0 as the cone narrows, so `I` → ∞ for a pencil beam; clamp
`cos_outer ≤ 0.9999` host-side (a `debug_assert!` + a runtime clamp in the `SpotLight` constructor) so
the division is bounded.

### Decision 3 (O3 ADOPTED): a single global EXPOSURE scalar in the light header

**What.** Add `exposure: f32` to the `LightHeaderGpu` std430 header. Applied as the **final multiply**
on the accumulated **linear** radiance in the resolve: `lit = (direct + ambient + emissive) * exposure;`
**DEFAULT = 1.0 (identity)** so the 0%-gate degenerate stays byte-identical. Sourced from a
`LightingConfig` ECS resource (`exposure: f32`, default 1.0) that feeds the header upload.

**Why.** It makes physical units usable (sun ≈ 100 000 lux, a lamp ≈ a few hundred lumens) without the
full auto-exposure + tonemapping pipeline (those stay a deferred phase). One scalar, one multiply, one
header field; the oracle multiplies identically. Identity default means the existing single-light
golden is untouched.

**Alternatives rejected.** Per-light exposure (redundant — fold into per-light power); a separate
exposure UBO (the header already exists and has room — Decision 7). Auto-exposure/histogram
(deferred — out of L0/L1 scope).

**Trade-off.** One non-physical global knob until auto-exposure lands; documented as a manual stop.

### Decision 4: lights are ECS entities/components; the GPU table is a derived upload

**What.** `DirectionalLight`, `PointLight`, `SpotLight` are `#[derive(Component)]` PODs in
`boyko_render/src/light.rs`. A `collect_lights` system folds the live light components into one
contiguous `GpuLight[]` staging slice + a `LightHeaderGpu`. Two distinct upload paths (C3):
- **FIRST seed (setup):** `GpuColumnManager::upload_initial` (`gpu_column.rs:833`) — the documented
  SETUP-only path that does `run_copy` then FENCE-WAITS so the upload completes before return
  (`:863-866`). Used ONCE to seed the table; a synchronous setup stall is fine.
- **On-change (per-frame, ASYNC barrier-ordered — NOT fence-waited):** on a
  `Changed<DirectionalLight> | Changed<PointLight> | Changed<SpotLight> | Changed<LightingConfig>`
  frame (or add/despawn), `collect_lights` writes the new table into the persistently-mapped
  host-coherent STAGING buffer and sets a "dirty" flag; the per-frame recorder records a
  staging→device copy + a TRANSFER_WRITE→SHADER_READ buffer barrier into the frame's command stream
  (see Decision 7 / Algorithm A / rung **L0-r0**). **Routing on-change through `upload_initial` would
  STALL the frame on every light change** — that path is setup-only. `upload_initial` is NOT used
  on-change; idle frames record nothing → zero cost.

**Why.** Principle 0 (ECS is THE SDK — no side store). The authoritative data is ECS columns; the GPU
table is a *projection*. `Changed`-gating means no per-frame collection or upload when lights are
static — zero frame-path cost. This is the Bevy lights-as-entities precedent adapted to our
`GpuColumnManager`.

**Alternatives rejected.** A bespoke `Vec<GpuLight>` light manager (a parallel data system — the exact
principle-0 violation that caused the SP4 race). Per-frame unconditional upload (wastes bandwidth when
static). Rejected.

**Trade-off.** A light's change re-uploads the whole table (one memcpy + one transfer). At
`MAX_LIGHTS=1024` that is ~48 KiB — negligible, and `Changed`-gated. A future delta-upload is possible
but unjustified at this scale.

### Decision 5: a tagged-union GpuLight POD (one table, branchless type dispatch)

**What.** ONE `GpuLight` std430 element (48 B, mirrors `MaterialGpu`) holds all three light types via a
`kind` tag in a free lane; unused fields are inert per kind. The resolve switches on `kind`
(branch-coherent within a cluster after the cull sorts by kind — see Decision 6) — directional has no
position, point has position+radius, spot adds direction+cone.

**Why.** One contiguous SoA-friendly table = one SSBO, one upload, one cull pass, sequential resolve
read (D-cache optimal). A tagged union avoids three separate tables + three binds + three cull passes.
The `kind` branch is the only divergence, and clustering keeps it coherent.

**Alternatives rejected.** Three typed tables (3× binds, 3× cull, 3× upload — more I-cache, more
descriptors). A `dyn`/virtual light (forbidden, and meaningless on the GPU). Rejected.

**Trade-off.** ~16 B/light wasted on a directional light (no position/cone). At 48 B × 1024 = 48 KiB
total, irrelevant; the uniformity win dominates.

### Decision 6: L1 froxel cluster cull — a NEW 3D cluster grid SSBO + flat index list

**What.** A `cluster_cull.hlsl` compute pass builds `16×9×24 = 3456` froxel AABBs in view space
(exponential-Z slices: `z_slice = near * (far/near)^(k/24)`), tests each `GpuLight`'s bounding sphere
(point/spot: center+radius; directional: in every cluster) vs each froxel AABB, and writes:
- a `ClusterGrid[]` SSBO: per froxel `{ u32 offset, u32 count }` into the index list (3456 × 8 B = ~27 KiB),
- a `LightIndex[]` flat SSBO: concatenated per-cluster light indices (`MAX_LIGHTS_PER_CLUSTER=256`
  cap × 3456 froxels worst case, but typically tiny; sized at setup, grown on cross).
The resolve maps a pixel to its froxel (`px,py` → tile x,y; `t`/view-z → exp-Z slice k), reads
`{offset,count}`, loops only those indices. **This is a NEW cluster grid SSBO — the existing 2D
`Tiles@6` buffer is the *conceptual* seed but a 3D froxel grid is a distinct buffer** (the 2D tile
buffer stays the SDF coarse-cull's; we do not overload it).

**Per-froxel overflow (O2, RESOLVED — clamp-and-drop).** When a froxel's accepted-light count reaches
`MAX_LIGHTS_PER_CLUSTER` (256), additional lights for that froxel are DROPPED (not appended); the
atomic bump is clamped, never overflowing the slice. Debug builds
`debug_assert!(per_cluster_count <= MAX_LIGHTS_PER_CLUSTER)`; release silently clamps-and-drops (no UB,
no overflow). 256 is a documented known cap. (Trade-off: a true counter would need a second pass;
clamp-and-drop is one pass, bounded, documented.)

**Camera forward-basis contract (O1, RESOLVED).** L1 computes PERSP froxel view-z from the surface `t`
as `dot(rd_world, fwd) * t` — so the camera UBO's `cam_forward.xyz` lane (`deferred_pbr.hlsl:70-79`,
`forward.w = tan(fovY/2)`) is contractually **NORMALIZED host-side**, with a host-side
`debug_assert!(|cam_forward.xyz| ≈ 1.0)` (within an epsilon) where the camera UBO is written. ORTHO
view-z is linear in `t`.

**Why.** O(cluster lights) per pixel instead of O(all lights) — the entire P7 payoff. Exponential-Z
matches perspective depth distribution (Olsson HPG2012 / aortiz primer). View-space AABBs are built
once per froxel per frame, not per pixel. Froxel→pixel mapping is integer arithmetic (branchless).

**Alternatives rejected.** Tiled (2D-only) forward+ (no Z culling → distant clusters keep near lights).
Overloading the 2D `Tiles@6` buffer (couples two unrelated culls; a layout change risks the SDF
coarse-cull golden). Per-pixel light loop over the full table (brute force — the thing P7 exists to
avoid). Rejected.

**Trade-off.** +1 compute dispatch/frame + 2 new SSBOs (~27 KiB grid + index list). The cull reads the
camera UBO for the view-space froxel build; deterministic given camera+table. The froxel→view-z map in
the resolve needs the surface view-z, derived from `t` (ORTHO: linear; PERSP: `dot(rd_world, fwd)*t`),
which `gViewT` (Decision 1) already provides — so L1 is gated behind L0b's `gViewT`.

### Decision 7: the light table + header are SCENE-GLOBAL SSBOs (like `material_table`)

**What.** `light_table: BoundBuffer` (the `GpuLight[]`) and the `LightHeaderGpu` (a small UBO or the
first element of the table SSBO) are created at setup exactly like `material_table` (`swapchain.rs:3262`),
bound to the resolve set at new slots. The FIRST seed uses `GpuColumnManager::upload_initial` (setup,
fence-waited); **the on-change re-upload uses the recorded ASYNC copy path (C3 / rung L0-r0), NOT
`upload_initial`** — see Decision 4. They are **scene resources**, NOT `(ArchetypeId,ComponentId)`
columns (a singleton table has no archetype key).

**Why.** The light table is a single global array indexed by light id, not a per-archetype column.
`material_table` is a close precedent — a scene-global SSBO bound to two sets — BUT note `material_table`
is host-seeded ONCE before the present loop (`swapchain.rs:3258`) and never re-uploaded on change, so it
is **not** a precedent for the dynamic on-change path; that path is the new L0-r0 async recorder
(Decision 4 / C3). The scene-global SSBO shape itself reuses the `material_table` pattern (no new RHI
capability for the buffer), while the dynamic re-upload IS a new recorded-copy capability.

**Alternatives rejected.** Forcing the singleton through the `(ArchetypeId,ComponentId)` keyed
`GpuColumnManager.meta` map (a fake archetype key — abuse of the column model). Rejected.

**Trade-off.** Two scene `BoundBuffer`s to grow on a `MAX_LIGHTS` cross (setup-class realloc, like the
arena/pool grows). Header in the table's word-0 region (HEADER_BASE pattern, see Data structures) keeps
it to one SSBO bind.

### Decision 8: header carried in the light SSBO's leading region (HEADER_BASE pattern)

**What.** Mirror the proven `HEADER_BASE_WORDS` edit-list pattern (`compute.rs:53`,
`sdf_gbuffer_composite.hlsl:40`): the light SSBO begins with the `LightHeaderGpu` (count, ambient,
exposure, cluster params), then the `GpuLight[]` array at a fixed word offset. One SSBO, one bind, one
upload covers both header and table.

**Why.** Fewer descriptors (the resolve is binding-tight), one staging copy, the existing word-0-header
idiom the codebase already validates with const-asserts. The header is read once per dispatch
(broadcast-uniform across the wave) — no per-pixel cost.

**Alternatives rejected.** A separate header UBO (extra bind on a tight set; the camera UBO@5 is
already the only UBO and is full). Rejected.

**Trade-off.** The array starts at `HEADER_BASE_WORDS` (16 words = 64 B), a fixed offset every shader
read accounts for — a one-time const, const-asserted host/shader.

---

## Data structures

### `GpuLight` (std430 POD, 48 B — mirrors `MaterialGpu`)

```rust
// boyko_render/src/light.rs
//
// One GPU light-table element. #[repr(C, align(16))], 3 clean vec4 lanes (NO greedy
// mixed-scalar packing) so the std430 mapping is unambiguous — exactly MaterialGpu's
// discipline. A const-assert fingerprint (size/align/every offset) pins the layout;
// GPU_LIGHT_WORDS == 12 mirrors the shader's pin. All radiometric values LINEAR.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuLight {
    // lane 0 (off 0): direction.xyz (DIRECTIONAL/SPOT) | unused (POINT), w = kind tag.
    //   kind: 0 = DIRECTIONAL, 1 = POINT, 2 = SPOT (bit-cast u32 in .w).
    pub dir_kind:   [f32; 4],
    // lane 1 (off 16): position.xyz (POINT/SPOT world pos) | unused (DIRECTIONAL),
    //   w = range/radius (POINT/SPOT cull sphere radius; INF for directional).
    pub pos_range:  [f32; 4],
    // lane 2 (off 32): color.rgb * intensity (LINEAR radiant/luminous, premultiplied
    //   for directional = irradiance, for point/spot = the I from Decision 2 baked OR
    //   color.rgb + w = packed cone cosines). To keep one lane: rgb = LINEAR color *
    //   power-scale; w = bit-cast packed cone = (cos_inner:16 | cos_outer:16) half-floats
    //   (SPOT only; unused otherwise).
    pub color_cone: [f32; 4],
}
pub const GPU_LIGHT_WORDS: usize = core::mem::size_of::<GpuLight>() / 4; // 12

// Layout fingerprint (drift = build error, exactly like MaterialGpu §):
const _: () = assert!(core::mem::size_of::<GpuLight>() == 48);
const _: () = assert!(core::mem::align_of::<GpuLight>() == 16);
const _: () = assert!(core::mem::offset_of!(GpuLight, dir_kind)   == 0);
const _: () = assert!(core::mem::offset_of!(GpuLight, pos_range)  == 16);
const _: () = assert!(core::mem::offset_of!(GpuLight, color_cone) == 32);
const _: () = assert!(GPU_LIGHT_WORDS == 12);

// Light kinds (mirror the shader's LIGHT_KIND_* and the host oracle).
pub const LIGHT_KIND_DIRECTIONAL: u32 = 0;
pub const LIGHT_KIND_POINT:       u32 = 1;
pub const LIGHT_KIND_SPOT:        u32 = 2;
```

> Spot cones are packed as two `f16` cosines in `color_cone.w` to keep `GpuLight` at 48 B (one lane
> per role). If `f16` packing risks precision at grazing cones, OQ-2 escalates a 64 B `GpuLight`
> (4 lanes) — measured before committing; the const-assert pins whichever is chosen.

### `LightHeaderGpu` (std430 header — 64 B, one cache line, 4×vec4) — O3 folded in

```rust
// boyko_render/src/light.rs
//
// The leading region of the light SSBO (HEADER_BASE pattern). #[repr(C, align(16))],
// 4 vec4 lanes = 64 B (one cache line). Read once per dispatch (wave-uniform broadcast).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightHeaderGpu {
    // lane 0 (off 0): x = bitcast<f32>(light_count u32), y = exposure (O3, default 1.0),
    //   z = bitcast<f32>(directional_count u32), w = bitcast<f32>(point_spot_count u32).
    //   (split counts let L0a loop directionals without touching point/spot — the rung split.)
    pub counts_exposure: [f32; 4],
    // lane 1 (off 16): ambient sky diffuse rgb (replaces SKY_DIFFUSE const), w = unused.
    pub sky_diffuse: [f32; 4],
    // lane 2 (off 32): ambient sky specular rgb (replaces SKY_SPEC const), w = unused.
    pub sky_spec: [f32; 4],
    // lane 3 (off 48): L1 cluster params — x = bitcast(cluster_dim_x), y = bitcast(dim_y),
    //   z = bitcast(dim_z), w = bitcast<f32>(clusters_enabled u32). 0 => L1 OFF (loop flat
    //   table [0..count) — the L1 0%-gate == L0b).
    pub cluster_params: [f32; 4],
}
pub const LIGHT_HEADER_WORDS: usize = core::mem::size_of::<LightHeaderGpu>() / 4; // 16
pub const LIGHT_HEADER_BASE_WORDS: usize = LIGHT_HEADER_WORDS; // GpuLight[] starts here

const _: () = assert!(core::mem::size_of::<LightHeaderGpu>() == 64);  // one cache line — fits with exposure
const _: () = assert!(core::mem::align_of::<LightHeaderGpu>() == 16);
const _: () = assert!(core::mem::offset_of!(LightHeaderGpu, counts_exposure) == 0);
const _: () = assert!(core::mem::offset_of!(LightHeaderGpu, sky_diffuse)     == 16);
const _: () = assert!(core::mem::offset_of!(LightHeaderGpu, sky_spec)        == 32);
const _: () = assert!(core::mem::offset_of!(LightHeaderGpu, cluster_params)  == 48);
const _: () = assert!(LIGHT_HEADER_WORDS == 16);
```

> **64 B header re-confirmed with `exposure` added (O3):** exposure occupies `counts_exposure.y`,
> a previously-unused word — the header was 64 B before O3 (counts in lane 0, sky in lanes 1–2,
> cluster in lane 3) and **stays 64 B**; exposure consumes the spare `.y` of lane 0. No size change.

### `ClusterGrid` + `LightIndex` (L1 only — std430)

```rust
// boyko_render/src/light.rs (L1)
#[repr(C)] #[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterCell { pub offset: u32, pub count: u32 }   // 8 B, per froxel
// LightIndex SSBO = flat [u32] index list, per-cluster slices concatenated.
pub const CLUSTER_DIM_X: u32 = 16;
pub const CLUSTER_DIM_Y: u32 = 9;
pub const CLUSTER_DIM_Z: u32 = 24;
pub const CLUSTER_COUNT: u32 = CLUSTER_DIM_X * CLUSTER_DIM_Y * CLUSTER_DIM_Z; // 3456
pub const MAX_LIGHTS: u32 = 1024;
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 256;
const _: () = assert!(core::mem::size_of::<ClusterCell>() == 8);
```

### ECS components + the config resource

```rust
// boyko_render/src/light.rs — authoritative ECS store (Decision 4). #[repr(C)] for a
// predictable layout; #[derive(Component)] via boyko_macros.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct DirectionalLight {
    pub direction: [f32; 3],   // world direction TO the light (normalized host-side)
    pub color:     [f32; 3],   // LINEAR rgb
    pub illuminance: f32,      // lux (physical); exposure (O3) maps to display
}
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct PointLight {
    pub position: [f32; 3],    // world
    pub color:    [f32; 3],    // LINEAR rgb
    pub power:    f32,         // luminous power Φ (lumens); I = Φ/(4π) for point
    pub range:    f32,         // cull-sphere radius (cutoff where atten ~ 0)
}
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct SpotLight {
    pub position:  [f32; 3],   // world
    pub direction: [f32; 3],   // world spot axis (normalized host-side)
    pub color:     [f32; 3],   // LINEAR rgb
    pub power:     f32,        // luminous power Φ; I = Φ/(2π(1−cos(outer)))  (O2)
    pub range:     f32,        // cull-sphere radius
    pub inner_deg: f32,        // inner cone half-angle (full intensity within)
    pub outer_deg: f32,        // outer cone half-angle (zero beyond)
}
// O3: the global exposure (+ future tonemap knobs). Resource, default identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingConfig {
    pub exposure: f32,         // O3 — DEFAULT 1.0 (identity → 0%-gate byte-identical)
    pub sky_diffuse: [f32; 3], // ambient hemisphere diffuse (default = today's SKY_DIFFUSE)
    pub sky_spec:    [f32; 3], // ambient specular        (default = today's SKY_SPEC)
    pub clusters_enabled: bool,// L1 gate (default false → L0b flat-loop path)
}
impl Default for LightingConfig {
    fn default() -> Self { Self {
        exposure: 1.0,                              // identity
        sky_diffuse: [0.10, 0.10, 0.12],           // == deferred_pbr.hlsl:99
        sky_spec:    [0.10, 0.10, 0.12],           // == deferred_pbr.hlsl:100
        clusters_enabled: false,
    }}
}
```

### `gViewT` G-buffer lane (O1 / Decision 1)

```text
New attachment: gViewT — VkFormat R32_SFLOAT, ImageUsage::STORAGE, extent = present_extent.
                (W2: R32_SFLOAT/STORAGE_IMAGE support fail-fast-checked in DeviceCaps before creation.)
  Marcher store : [[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(uN);
                  WRITE AT ALL THREE TERMINAL EXITS, exactly once/pixel/frame (C2):
                  1. P4b EMPTY early-return (:441-444): gViewT[uint2(px,py)] = 1.0e30;  (BEFORE return; @:444)
                  2. SDF-hit arm (:588-590): t = ro+rd*t  -> this t is stored on the lit final arm
                  3. final block (:623-625): gViewT[uint2(px,py)] = (mask==1.0) ? t : 1.0e30;
                     (real t on the SDF-lit arm; 1.0e30 sentinel on mesh/background)
                  (hoist `float view_t = 1.0e30;` if HLSL scoping needs it)
  Resolve read  : float t = gViewT.Load(coord);  float3 P = ro + rd * t;
                  READ-UNDER-MASK GATE (C2): inside the is_sdf_lit / mask==1 branch ONLY
                  (resolve point/spot AND L1 froxel-z) — a sentinel on a non-lit pixel is never consumed.
  Layout        : transition UNDEFINED→GENERAL with the other 3 (swapchain.rs:2443 loop);
                  store→load barrier (swapchain.rs:2539 loop) gains gViewT;
                  marcher vocab set += @8 (9th entry, ≤ raised 12-cap, C1);
                  resolve set += @6 (storage image, ≤ 12).
```

---

## Public API (signatures only)

```rust
// boyko_render/src/light.rs
impl DirectionalLight { pub fn new(direction: [f32;3], color: [f32;3], illuminance: f32) -> Self; }
impl PointLight       { pub fn new(position: [f32;3], color: [f32;3], power: f32, range: f32) -> Self; }
impl SpotLight {
    /// Clamps cos(outer) ≤ 0.9999 (Decision 2 trade-off: bounds I = Φ/(2π(1−cos))).
    pub fn new(position: [f32;3], direction: [f32;3], color: [f32;3],
               power: f32, range: f32, inner_deg: f32, outer_deg: f32) -> Self;
}
impl GpuLight {
    pub fn from_directional(l: &DirectionalLight) -> Self;
    pub fn from_point(l: &PointLight) -> Self;      // bakes I = Φ/(4π)
    pub fn from_spot(l: &SpotLight) -> Self;        // bakes I = Φ/(2π(1−cos(outer))) (O2)
}
impl LightHeaderGpu {
    pub fn new(dir_count: u32, point_spot_count: u32, cfg: &LightingConfig) -> Self; // O3 exposure folded
}

// The collection system (Decision 4) — a normal ECS system; Changed-gated.
pub fn collect_lights_system(/* Query<DirectionalLight>, Query<PointLight>, Query<SpotLight>,
                               Res<LightingConfig>, NonSend<RhiContext> */);

// swapchain.rs GBufferScene gains (Decision 7): scene-global light SSBO handle + (L1) cluster SSBOs.
//   pub light_table: &'a BoundBuffer,       // header (word 0..16) + GpuLight[] (Decision 8)
//   pub cluster_grid: &'a BoundBuffer,      // L1 ClusterCell[CLUSTER_COUNT]
//   pub light_index:  &'a BoundBuffer,      // L1 flat [u32]
```

---

## Algorithms for critical paths

### A. `collect_lights` (CPU, setup + on-change; Decision 4)

1. `Changed`-filter the three light queries + `Changed<LightingConfig>`; if none changed and no
   add/despawn, **return early — zero work** (the static-scene fast path).
2. Walk directional lights → `GpuLight::from_directional` into staging `[HEADER..]`, counting
   `dir_count`. Then point + spot → `from_point`/`from_spot` (bakes O2's `I`), counting
   `point_spot_count`. (Directionals first so L0a can loop `[0..dir_count)`.)
3. Write `LightHeaderGpu::new(dir_count, point_spot_count, &cfg)` into staging `[0..16]` (O3 exposure
   folded).
4. **Upload (C3 — two paths):**
   - *Setup (first seed):* one `GpuColumnManager::upload_initial` of `[header || array]` into
     `light_table` (fence-waited; setup-class; grow on `MAX_LIGHTS` cross).
   - *On-change (per frame):* write `[header || array]` into the persistently-mapped host-coherent
     STAGING buffer and **set a "dirty" flag** — this CPU step is a memcpy only (no GPU sync, no
     fence). The per-frame recorder (`render_gbuffer_frame`, rung L0-r0) does the actual
     staging→device `cmd_copy_buffer` + a TRANSFER_WRITE→SHADER_READ barrier on the GPU timeline,
     BEFORE the marcher dispatch. **No synchronous `upload_initial` call on-change.** Idle frames
     leave the dirty flag clear → the recorder records nothing → zero cost.
- **Complexity:** O(live lights). **Cache:** sequential staging write (streaming). **Branching:** one
  `kind` switch per light (cold, setup-path). **Allocations:** none on the frame path (staging is
  preallocated, grown only on cross). **SIMD:** the per-light POD build is auto-vectorizable; not hot.

### B. Resolve loop — L0a (directional + sky, NO P)

Inside `is_sdf_lit` (the existing branch), replace the compiled-in constant with:
```
LightHeaderGpu H = load_header();              // word 0..16, wave-uniform
float3 lit_direct = 0;
for (uint i = 0; i < H.dir_count; ++i) {        // [0..dir_count)
    GpuLight L = lights[HEADER_BASE + i];
    float3 l = normalize(L.dir_kind.xyz);
    // ... existing D*V*F + Lambert, * NoL * shadow, * L.color_cone.rgb
    lit_direct += brdf(...) ;
}
float3 ambient = (spec_ambient*H.sky_spec + diff_ambient*H.sky_diffuse) * ao;
lit = (lit_direct + ambient + m.emissive.rgb) * H.exposure;   // O3 final multiply
```
- **Complexity:** O(dir_count). **Cache:** sequential table read (D-cache optimal); header broadcast.
- **Branching:** the loop; no `kind` divergence (directionals only). **0%-gate:** `dir_count==1` +
  default color/exposure==1.0 reproduces `LIGHT_DIR`/`LIGHT_COLOR` exactly.
- **Byte-identity op-order (W1) — HARD requirement.** The host oracle `golden_deferred_resolve`
  (`compute.rs:1863-1876`) is straight-line: `let mut lit = [0.0_f32; 3];`, per channel
  `let direct = (diff + spec) * (nol * shadow) * PBR_LIGHT_COLOR[c];` (`:1867`),
  `lit[c] = direct + ambient + mat.emissive[c];` (`:1875`), then `pack_rgba(lit)` (`:1877`); no
  exposure today. The L0a single-light expansion MUST keep the EXACT per-light expression:
  accumulator initialized to `0.0`; each light contributes `(diff + spec) * (nol * shadow) * color`;
  the FINAL `* exposure` (O3) is **literally last**. Because exposure default = 1.0, `x * 1.0 == x`
  exact and `0.0 + x == x` exact, so the OFF/default path is **bit-identical**. The math identity
  holds; this is a discipline pin — **FORBID any abstraction that reassociates the accumulation**
  (no Horner-style fold, no fused-multiply reordering that changes rounding). The existing host
  OFF-path golden is added as a bit-exact regression.

### C. Resolve loop — L0b (adds point + spot; needs `P` from `gViewT`)

After B, reconstruct `P` once, then loop point/spot. **The `gViewT.Load` here executes STRICTLY inside
the `is_sdf_lit` / `mask == 1` branch (C2 read-under-mask gate)** — a non-lit pixel carries the
`1.0e30` sentinel that is never consumed:
```
float t = gViewT.Load(coord);  float3 P = ro + rd * t;   // O1 / Decision 1; inside mask==1 only
for (uint i = H.dir_count; i < H.light_count; ++i) {
    GpuLight L = lights[HEADER_BASE + i];
    uint kind = asuint(L.dir_kind.w);
    float3 toL = L.pos_range.xyz - P;  float d2 = dot(toL,toL);
    if (d2 > L.pos_range.w * L.pos_range.w) continue;     // outside cull sphere (range)
    float3 l = toL * rsqrt(d2);  float atten = 1.0 / max(d2, 1e-4);
    if (kind == LIGHT_KIND_SPOT) {
        float2 cones = unpack_cones(L.color_cone.w);      // (cos_inner, cos_outer)
        float cosA = dot(-l, normalize(spot_dir(L)));
        float tt = saturate((cosA - cones.y) / (cones.x - cones.y));
        atten *= tt * tt;                                  // O2 angular falloff
    }
    lit_direct += brdf(...) * atten * L.color_cone.rgb;
}
lit = (lit_direct + ambient + m.emissive.rgb) * H.exposure;
```
- **Complexity:** O(point_spot_count) brute (L0b) → O(cluster lights) (L1). **Cache:** sequential table
  read. **Branching:** `kind` switch + range cull (coherent post-cluster). **0%-gate:** zero
  point/spot lights → loop body never runs → byte-identical to L0a.

### D. `cluster_cull` (L1 compute, per frame; Decision 6)

1. Per froxel `(x,y,z)` (1 thread): compute the view-space AABB (tile x,y → view-frustum slab; exp-Z
   `z_near*(z_far/z_near)^(z/24)`), from the camera UBO basis + extent.
2. For each `GpuLight i`: directional → always in; point/spot → sphere(center=`pos`,r=`range`) vs AABB.
   Append `i` to this froxel's index slice (atomic bump of a per-froxel counter into `LightIndex`).
   **Overflow policy (O2 — clamp-and-drop):** when a froxel's accepted-light count reaches
   `MAX_LIGHTS_PER_CLUSTER` (256), additional lights for that froxel are **DROPPED** (not appended) —
   the atomic bump is clamped, never overflowing the slice. Debug builds
   `debug_assert!(per_cluster_count <= MAX_LIGHTS_PER_CLUSTER)`; release silently clamps-and-drops (no
   UB, no overflow). The 256 cap is a documented known limit. (Trade-off: a counter that recorded the
   true count would need a second pass; clamp-and-drop is one pass, bounded, documented.)
3. Write `ClusterCell{offset,count}` for the froxel.
- **Complexity:** O(CLUSTER_COUNT × lights) = 3456 × ≤1024 worst case, typ. ≪. **Cache:** the table is
  re-read per froxel (L2-resident, 48 KiB). **Branching:** the sphere-AABB test (branchless via
  `max(0, dist)`). **SIMD:** the AABB test is vectorizable. **Sync:** per-froxel atomic counter into
  the index list (lock-free; each froxel owns a disjoint slice region or a global atomic bump — see
  Multithreading). **Determinism:** index *order within a froxel* may vary with atomic race; the
  resolve sums commutatively (float add reorder within ±tolerance — consumer-side, like A1/A2). OQ-3
  escalates if bit-exact ordering is required (then a 2-pass count+scatter, deterministic).

---

## Multithreading model

- **CPU side.** `collect_lights` is one ECS system reading three light queries + `LightingConfig`,
  writing one staging slice on the dispatcher thread that owns `RhiContext` (`!Send`). It conflicts
  (write) with nothing else touching the light table; the scheduler serializes it via the normal
  conflict graph (it `NonSend`-borrows `RhiContext`, which is single-thread by construction). **No
  Mutex/RwLock** — the `!Send` `RhiContext` is the synchronization (compiler-enforced single owner).
- **GPU side.** All passes recorded into the SINGLE per-frame command stream (`render_gbuffer_frame`,
  `swapchain.rs:2008`), submitted ONCE at frame end with NO intra-frame fence — the async precedent:
  it already records marcher dispatch (`:2531`) → a store-to-load IMAGE barrier
  (`:2539-2570`, COMPUTE_SHADER→COMPUTE_SHADER, SHADER_WRITE→SHADER_READ) → resolve dispatch
  (`:2581-2597`) → lit barrier, all into one `cmd` encoder. Ordering (no CPU sync):
  - **L0-r0 (C3, on a dirty frame only):** BEFORE the marcher dispatch — a `cmd_copy_buffer`
    staging→`light_table` device SSBO, then a **`BUFFER_MEMORY_BARRIER` TRANSFER_WRITE → SHADER_READ,
    `VK_PIPELINE_STAGE_TRANSFER_BIT → VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`** on the `light_table`
    buffer — so the copy is visible to the marcher/resolve reads on the GPU timeline (mirroring the
    existing store-to-load image barrier at `:2539`). No fence-wait, no stall, ZERO readback. (NOTE:
    BOTH `GpuColumnManager::upload_initial` (`gpu_column.rs:863-866`) AND
    `GpuColumnManager::dispatch_compute` (`gpu_column.rs:571,610`) fence-wait, and `GpuSystem` Wave C
    "uses a straightforward submit+wait" (`gpu_system.rs:351-352`) — so NEITHER is a fence-free path;
    the recorded-async-copy here is a genuinely NEW capability. `GpuSystem` only supplies the
    STRUCTURAL precedent of a recorded barrier-replay + work in an encoder, GPU-timeline ordered, zero
    readback.)
  - marcher (stores G-buffer incl. `gViewT`) → [L1: `cluster_cull` reads table+camera, writes
    grid+index] → resolve (reads G-buffer + `gViewT` + table + grid+index). Barriers: the existing
    UNDEFINED→GENERAL + store→load loops gain `gViewT`; L1 adds a COMPUTE→COMPUTE buffer barrier
    (cull WRITE → resolve READ) on `cluster_grid`/`light_index`.
- **`cluster_cull` intra-pass.** Each froxel thread writes its own `ClusterCell` (disjoint — no sync).
  The shared `LightIndex` append uses ONE global `InterlockedAdd` per (froxel, accepted light) to claim
  a slice base, then disjoint writes — lock-free, no data race (each claimed range is exclusive). The
  only nondeterminism is index *order within a froxel*; the resolve's per-light add is commutative
  within ±tolerance (consumer-side, matching the A1/A2 precedent). **Data-race freedom:** disjoint
  cell writes + atomic-claimed disjoint index ranges ⇒ no two threads write the same address.
- **Send/Sync.** `GpuLight`/`LightHeaderGpu`/`ClusterCell` are POD `Copy` (`Send+Sync`). The light
  components are POD `Component`s (`Send+Sync`, stored in ECS columns). `RhiContext` stays `!Send`
  (unchanged).

---

## Integration

### Files touched / created

| File | Change |
|---|---|
| `crates/boyko_render/src/light.rs` | **NEW** — `GpuLight`, `LightHeaderGpu`, `ClusterCell`, the three light components, `LightingConfig`, constructors, const-assert fingerprints, `collect_lights_system`. |
| `crates/boyko_render/src/lib.rs` | register `light` module + prelude exports. |
| `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl` | replace the compiled-in `LIGHT_DIR`/`SKY_*` constants with header+table reads; add the directional loop (L0a), `gViewT`+`P`+point/spot loops (L0b), cluster lookup (L1), final `* exposure` (O3). |
| `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` | add the `gViewT` store (L0b): `[[vk::image_format("r32f")]] RWTexture2D<float> gViewT @uN`. **Write at ALL THREE terminal exits (C2):** sentinel `1.0e30` at the P4b EMPTY early-return (`:441-444`, before `return;` at `:444`); `gViewT = (mask==1.0) ? t : 1.0e30;` at the final block (`:623-625`) — real `t` (from the SDF arm `:588-590`) on the lit arm, sentinel on mesh/background. |
| `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` | **NEW** (L1) — froxel AABB build + sphere-AABB cull → grid + index list. |
| `crates/boyko_rhi_vulkan/shaders/light_table.hlsli` | **NEW** — shared `GpuLight`/`LightHeaderGpu` std430 struct decls + `HEADER_BASE` + `unpack_cones` + kind consts (included by resolve + cull; ONE source of truth, like `ray_gen.hlsli`). |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` | create `gViewT` image (L0b) + transition/barrier loops; create `light_table` SSBO (L0a, like `material_table`); add resolve-set bindings (`light_table`, then `gViewT @6`); (L0-r0, C3) wire the per-frame staging→`light_table` `cmd_copy_buffer` + TRANSFER_WRITE→SHADER_READ barrier into `render_gbuffer_frame` (`:2008`), recorded only on a dirty frame, before the marcher dispatch (`:2531`); (L1) create `cluster_grid`/`light_index` + the cull dispatch + its barrier. `GBufferScene`/`GBufferTargets` gain the new handles. |
| `crates/boyko_render/src/gpu_column.rs` | **(NEW, C3)** add a `record_upload(encoder, handle, bytes)`-style recorder that records a staging→device `cmd_copy_buffer` + a TRANSFER_WRITE→SHADER_READ buffer barrier into a caller-supplied encoder (NO fence) — distinct from the setup-only fence-waited `upload_initial` (`:833`) and the fence-waited `dispatch_compute` (`:576`). (Alternative: do the copy+barrier inline in `render_gbuffer_frame` against the scene's `light_table` `BoundBuffer` + a manager-owned staging buffer.) Mint the staging buffer at setup; set a "dirty" flag from `collect_lights` so the copy records ONLY on a changed frame. |
| `crates/boyko_rhi_vulkan/src/device.rs` | **(W2)** extend `DeviceCaps` (`:1838-1841`) with `viewt_storage_format_ok: bool` and `query_device_caps` (`:1819-1842`) to query `VK_FORMAT_R32_SFLOAT` for `VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT` (OPTIMAL tiling, same `get_physical_device_format_properties` pattern as `gbuffer_storage_format_ok`); fail-fast at the same caps-validation site before the `gViewT` image is created (L0b). |
| `crates/boyko_rhi/src/device.rs`, `crates/boyko_rhi_vulkan/src/rhi_impl.rs` | **(C1)** raise `MAX_BIND_GROUP_BINDINGS` 8 → 12 (the two literal consts `boyko_rhi/src/device.rs:22` + `rhi_impl.rs:74`, const-asserted equal at `rhi_impl.rs:80`) + update the agnostic docstring (`boyko_rhi/src/device.rs:17-22`). Everything else (`entries[..]` field, inline arrays, debug_asserts, clamps) scales via the named const; the pool histogram (`KIND_COUNT=5`) is untouched. See the C1 exhaustive touch-list in Decision 1. |
| `crates/boyko_rhi_vulkan/src/compute.rs` | host oracle: `golden_deferred_resolve` loops the table + applies exposure (op-order pinned, W1); `golden_composite_pixel_ex` writes `gViewT`; POD fingerprints for `GpuLight`/`LightHeaderGpu`/`ClusterCell` mirrored host-side; L1 host cull oracle. |

### Required changes to existing APIs

- `GBufferScene<'a>` (+`light_table`, L1 `+cluster_grid`/`+light_index`) and `GBufferTargets`
  (+`gViewT` from L0b, + `resolve_set` gains bindings). The resolve bind-group LAYOUT widens 6→7
  (L0a: +`light_table`) → 8 (L0b: +`gViewT @6`) → 10 (L1: +`cluster_grid` +`light_index`) bindings —
  **all ≤ 12 with headroom** (the cap is raised 8 → 12 in this plan; see C1). **L1's resolve inputs
  (`cluster_grid` + `light_index`) stay on the SAME single resolve set** (10 ≤ 12) — no second
  descriptor set, no multi-bind juggling (OQ-1 RESOLVED).
- No `boyko_ecs` core change (lights are ordinary components + a resource).

### Compatibility with engine storage

- Light components live in `ComponentPool` columns (ECS-native). The GPU table is a scene-global
  `BoundBuffer` (the `material_table` layout precedent): seeded once via `upload_initial`, then
  re-uploaded on-change via the recorded async copy + barrier (C3 / L0-r0). `RhiContext` `!Send`.
  No `std::Vec`/`HashMap` side store (principle 0). The frozen field is consumed (L1 cull is geometric,
  not field-touching; point/spot shadows are a later GI rung).

---

## Implementation plan (rung by rung)

### Rung L0-r0 — Async barrier-ordered light-table re-upload (C3; lands BEFORE L0a's light rungs depend on it)

A genuinely NEW manager/RHI capability: both `upload_initial` (`gpu_column.rs:833`) and
`dispatch_compute` (`gpu_column.rs:576`) fence-wait, so no recorded-async-buffer-copy path exists today.
This rung builds the on-change upload mechanism the L0a/L0b light rungs consume; lights MUST support
runtime change without a frame stall (do NOT ship static-only). Deliverables:
1. **Staging-buffer mint (setup):** a manager-owned persistently-mapped host-coherent staging buffer
   for the light table.
2. **The recorder:** a `GpuColumnManager::record_upload(encoder, handle, bytes)`-style method that
   records a staging→device `cmd_copy_buffer` + a `BUFFER_MEMORY_BARRIER` TRANSFER_WRITE → SHADER_READ
   (`VK_PIPELINE_STAGE_TRANSFER_BIT → VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`) into a caller-supplied
   encoder — **NO fence**. (Alternative: the copy+barrier inline in `render_gbuffer_frame` against the
   scene's `light_table` `BoundBuffer` + the manager-owned staging buffer.)
3. **Per-frame wiring:** record the copy+barrier in `render_gbuffer_frame` (`swapchain.rs:2008`) BEFORE
   the marcher dispatch (`:2531`), so the upload is visible to the marcher/resolve reads on the GPU
   timeline (mirroring the store-to-load image barrier at `:2539`).
4. **Dirty flag:** `collect_lights` sets a "dirty" flag when the table changes; the copy is recorded
   ONLY on a changed frame — idle frames record nothing → zero cost.
5. **Validation note:** the barrier orders TRANSFER_WRITE → SHADER_READ (a Miri/Vulkan-validation
   note; no fence-wait, no readback).
- **0%-gate:** an idle (non-dirty) frame records nothing — byte-identical command stream to today.
- The FIRST seed still uses `upload_initial` (setup, fence-waited); only the on-change path is async.

### Rung L0a — Directional + Sky, multi-light, NO `P` (hits the 0%-gate first)

1. `light.rs`: `GpuLight`, `LightHeaderGpu` (with O3 `exposure`), `DirectionalLight`, `LightingConfig`,
   `from_directional`, `LightHeaderGpu::new`, all const-assert fingerprints.
2. `light_table.hlsli`: shared std430 decls + `HEADER_BASE` + kind consts.
3. `swapchain.rs`: create the scene-global `light_table` SSBO (like `material_table`); bind it to the
   resolve set (6→7 bindings); `GBufferScene` gains `light_table`.
4. `deferred_pbr.hlsl`: read header+table; loop `[0..dir_count)` directional; ambient from header
   `sky_*`; final `* exposure`. Replace `LIGHT_DIR`/`SKY_*` constants.
5. `compute.rs`: `golden_deferred_resolve` reads a host `GpuLight`/header, loops directionals, applies
   exposure; POD fingerprints.
6. `collect_lights_system` (directional-only path), `Changed`-gated.
- **GPU golden:** a 1-entry default table (`direction=(0,0,1)`, white, illuminance mapping to 1.0,
  exposure=1.0) → **byte-identical** to today's resolve output. Then a 2-directional golden (host
  oracle predicts within ±2/255).
- **0%-gate:** the default-table degenerate is bit-identical to the compiled-in constant.

### Rung L0b — Point + Spot (wires `gViewT`, reconstructs `P`)

1. `sdf_gbuffer_composite.hlsl`: add the `gViewT` storage image; write at **ALL THREE terminal exits
   (C2)** — sentinel `1.0e30` at the P4b EMPTY early-return (`:441-444`, before `return;` at `:444`);
   at the final block (`:623-625`) `gViewT = (mask==1.0) ? t : 1.0e30;` (real `t` from the SDF-hit arm
   `:588-590` on the lit arm, sentinel on mesh/background). Hoist a `float view_t = 1.0e30;` if HLSL
   scoping requires it.
2. `device.rs` (W2): extend `DeviceCaps` + `query_device_caps` with the `VK_FORMAT_R32_SFLOAT` /
   `STORAGE_IMAGE` check; fail-fast before the `gViewT` image is created.
3. `swapchain.rs`: create the R32_SFLOAT `gViewT` image; add to the UNDEFINED→GENERAL loop, the
   store→load barrier loop, the marcher vocab set (`@8` — the 9th vocab entry, now ≤ the raised
   12-binding cap, C1), the resolve set (`@6` — now 8 resolve bindings, ≤ 12).
4. `light.rs`: `PointLight`, `SpotLight` (+ `cos_outer` clamp, O2), `from_point`/`from_spot` (bake
   `I = Φ/(4π)` / `I = Φ/(2π(1−cos(outer)))`).
5. `deferred_pbr.hlsl`: reconstruct `P = ro + rd * gViewT.Load(coord)` (inside `mask==1` only, C2);
   loop `[dir_count..light_count)` point/spot with range cull + O2 spot falloff + inverse-square.
6. `compute.rs`: `golden_composite_pixel_ex` writes `gViewT`; the resolve oracle reconstructs `P` and
   loops point/spot (host `from_point`/`from_spot` mirror the bake).
7. `collect_lights_system`: full directional+point+spot path.
- **GPU golden:** (a) zero point/spot lights → byte-identical to L0a; (b) one point light at a known
  `P` → host oracle predicts the inverse-square + NoL term within ±2/255; (c) one spot → O2 cone
  falloff predicted within ±2/255; (d) a `gViewT` round-trip golden (marcher writes `t`, a readback
  asserts `ro+rd*t == p` to fp32); (e) **a full-frame `gViewT`-coverage golden (C2): every pixel's
  `gViewT` is written EXACTLY ONCE per frame — no NaN/uninitialized lane; SDF (`mask==1`) pixels carry
  a finite marched `t`, non-SDF (mesh/background/EMPTY) pixels carry the `1.0e30` sentinel** (proves no
  exit path leaves `gViewT` unwritten, including the P4b EMPTY early-return at `:444`).
- **0%-gate:** an all-directional table on L0b == L0a output (point/spot loop skipped); the marcher's
  gAlbedo/gNormal/gMaterial bytes are unchanged (only the new `gViewT` lane is added).

### Rung L1 — Clustered froxel cull (1→many at scale)

1. `light.rs`: `ClusterCell`, cluster consts, `cluster_params` header fields, `clusters_enabled`.
2. `cluster_cull.hlsl`: froxel AABB build (exp-Z) + sphere-AABB cull → `cluster_grid` + `light_index`.
3. `swapchain.rs`: create `cluster_grid`/`light_index` SSBOs (grow on cross); the cull dispatch
   (`CLUSTER_COUNT` threads) before the resolve; a COMPUTE→COMPUTE buffer barrier (cull WRITE →
   resolve READ); bind grid+index to the cull set AND to the SAME single resolve set (resolve now 10
   bindings ≤ the 12-binding cap — OQ-1 RESOLVED, no second set, C1).
4. `deferred_pbr.hlsl`: map pixel → froxel (`px,py`→tile; `gViewT` view-z → exp-Z slice); read
   `ClusterCell`; loop only `light_index[offset..offset+count)`.
5. `compute.rs`: host cull oracle (build froxel AABBs, cull, predict the per-pixel index slice) +
   resolve uses the cluster path; POD fingerprint for `ClusterCell`.
- **GPU golden:** (a) `clusters_enabled=0` → byte-identical to L0b; (b) a scene with lights spread
  across froxels → each pixel's lit value matches the brute-force (L0b) value within ±2/255 (the cull
  is exact for the test scene — no light wrongly dropped); (c) a froxel-assignment golden (host cull
  vs GPU cull index sets match as SETS, order-independent); (d) **"no false drop under cap" (O2): a
  scene with ≤ `MAX_LIGHTS_PER_CLUSTER` lights in every froxel drops NOTHING — the cull is exact below
  the cap.**
- **0%-gate:** `clusters_enabled==0` loops the flat table (== L0b), byte-identical.

---

## Metrics and validation

### Benchmarks (criterion / GPU timestamp)

- `collect_lights` cost vs light count (assert O(n), assert **0 allocations** on the static-scene
  early-out path).
- Resolve cost: L0a (directional only) vs L0b (brute point/spot, N lights) vs L1 (clustered, N lights)
  — assert L1 ≪ L0b at N ≥ 64 (the P7 payoff).
- `cluster_cull` dispatch time vs light count (assert sub-frame at MAX_LIGHTS).
- Frame-path allocation counter == 0 across L0a/L0b/L1 (mimalloc bench-alloc counter, like Phase X.E).

### Mandatory unit tests

- POD fingerprints: `GpuLight`==48 B, `LightHeaderGpu`==64 B (with exposure), `ClusterCell`==8 B, every
  offset (const-assert — compile-time).
- `from_spot` bakes `I = Φ/(2π(1−cos(outer)))` (O2) for known cones; `cos_outer` clamp at 0.9999.
- `from_point` bakes `I = Φ/(4π)`.
- `LightHeaderGpu::new(.., cfg)` carries `exposure` (O3) and split counts.
- `LightingConfig::default().exposure == 1.0` and `sky_*` == the old constants (0%-gate anchor).
- Host oracle exposure multiply identity at exposure==1.0.
- **W1 bit-exact OFF-path regression:** the existing host OFF-path golden (`golden_deferred_resolve`,
  `compute.rs:1863-1876`) is pinned as a **bit-exact** regression — the L0a single-light expansion with
  exposure==1.0 must reproduce it byte-for-byte (`x*1.0==x`, `0.0+x==x` exact). The test FORBIDS any
  abstraction that reassociates the accumulation (no Horner-style fold, no fused-multiply reordering
  that changes rounding); the per-light expression stays `(diff + spec) * (nol * shadow) * color` with
  the `* exposure` literally last.

### Property-based tests

- Random light tables: host oracle resolve == host brute-force, and L1 host cull index-set ⊇ every
  light whose sphere intersects the froxel (no false drop).
- `gViewT` round-trip: for random rays, `ro + rd * t` reconstructs `p` to fp32 (O1 correctness).

### Mandatory `debug_assert!`

- `light_count ≤ MAX_LIGHTS`; `per_cluster_count ≤ MAX_LIGHTS_PER_CLUSTER` (O2: debug catches the
  cap; release silently clamps-and-drops — no UB, no overflow).
- `cos_outer ≤ 0.9999` (O2 division bound).
- `exposure > 0.0` and finite (O3).
- `HEADER_BASE_WORDS == LIGHT_HEADER_WORDS` host==shader; `GPU_LIGHT_WORDS == 12` host==shader.
- `gViewT` is written EXACTLY ONCE per pixel per frame across ALL THREE exits (C2): the SDF-hit arm
  writes the marched `t`; the P4b EMPTY early-return (`:444`) and the mesh/background final arms write
  the `1.0e30` sentinel; no exit leaves it unwritten (paired with the full-frame coverage golden).
- **O1: `|cam_forward.xyz| ≈ 1.0`** (within an epsilon) host-side where the camera UBO is written —
  the forward basis lane (`deferred_pbr.hlsl:70-79`, `forward.w = tan(fovY/2)`) is contractually
  NORMALIZED, since L1 computes PERSP view-z as `dot(rd_world, fwd) * t`.
- **W2:** `viewt_storage_format_ok` (R32_SFLOAT supports `STORAGE_IMAGE`) is validated fail-fast at
  device-caps before the `gViewT` image is created.

---

## Open questions (final state)

- **O1 — RESOLVED (Decision 1).** Point/spot **require a G-buffer touch**: a new R32_SFLOAT `gViewT`
  lane the marcher stores its already-computed surface `t` into and the resolve reads to reconstruct
  `P = ray_origin + ray_dir * t` via the shared `generate_ray`. No existing resolve-visible
  high-precision `t` source exists (gMaterial.a is 8-bit; mesh `gDepth` is on the marcher's set and is
  mesh-only). **Therefore Directional + Sky ship FIRST (Rung L0a — 0%-gate, no G-buffer change), and
  Point + Spot follow (Rung L0b) once `gViewT` is wired.** Definitive.
- **O2 — RESOLVED (Decision 2).** Spot intensity `I = Φ/(2π(1−cos(outer)))` (reflector model); the
  absorber alternative is dropped.
- **O3 — RESOLVED (Decision 3).** A single global `exposure: f32` in `LightHeaderGpu`
  (`counts_exposure.y`), default 1.0 (identity → 0%-gate byte-identical), applied as the final multiply
  on accumulated linear radiance; sourced from `LightingConfig`. Header stays **64 B** (exposure fills
  a previously-unused word — no size change).
- **OQ-1 (binding budget, L1) — RESOLVED (C1).** With the binding cap raised 8 → 12 (Decision 1 / C1),
  L1's `cluster_grid` + `light_index` go on the **SAME** single resolve set (resolve total = 10 ≤ 12) —
  no second descriptor set, no multi-bind juggling, no ambient-folding workaround. The prior "L0b hits
  exactly 8" pressure is gone (the cap is 12 now).
- **OQ-2 (spot cone precision) — OPEN.** Two `f16` cosines packed in `color_cone.w` keep `GpuLight` at
  48 B; if grazing-cone banding appears in the L0b spot golden, widen `GpuLight` to 64 B (4 lanes,
  full-fp32 cones). Measured at L0b before committing; the const-assert pins whichever.
- **OQ-3 (cull determinism) — OPEN, but the overflow policy is now decided (O2).** Atomic index-append
  makes per-froxel index *order* nondeterministic; the resolve sums commutatively (±tolerance, A1/A2
  precedent). If a bit-exact golden is required, switch to a 2-pass count+prefix-scan+scatter cull
  (deterministic order). Default: the single-pass atomic cull with order-independent set goldens. The
  *capacity-overflow* policy (separate from order) is RESOLVED as O2's clamp-and-drop (`MAX_LIGHTS_PER_
  CLUSTER` = 256, drop-on-cap, documented).
- **O1 (camera `cam_forward` normalized contract) — RESOLVED (Decision 6).** The camera UBO's
  `cam_forward.xyz` lane is contractually NORMALIZED host-side (used as `dot(rd_world, fwd) * t` for
  PERSP froxel view-z), with a host-side `debug_assert!(|cam_forward.xyz| ≈ 1.0)`.
- **O2 (per-froxel overflow policy) — RESOLVED (Decision 6 / Algorithm D).** Clamp-and-drop at
  `MAX_LIGHTS_PER_CLUSTER` (256): debug asserts the cap, release silently clamps-and-drops (no UB);
  documented limit; golden "no false drop under cap".
- **Owner VALUES/SCOPE calls (escalate, not architect-decided):** intensity unit conventions
  (lux/lumens vs a unitless scale) for authoring; whether L1 ships before or after the L2 probe volume;
  the VRAM budget split (table + grid + index + future probe volume) on the 6 GB target.

---

## Changelog (vs the pre-finalization design)

- **O1 folded:** added Decision 1 (the `gViewT` R32_SFLOAT lane + `P = ro+rd*t` reconstruction) and
  split the rungs into **L0a (directional+sky, no P, 0%-gate first)** and **L0b (point+spot, wires
  `gViewT`)**; L1 now explicitly gates behind L0b's `gViewT` for froxel view-z.
- **O2 folded:** Decision 2 fixes spot `I = Φ/(2π(1−cos(outer)))`; `from_spot` + the `cos_outer` clamp.
- **O3 folded:** Decision 3 + `LightHeaderGpu.counts_exposure.y` + `LightingConfig.exposure` (default
  1.0) + the resolve's final `* exposure`; header re-confirmed at **64 B** (no size change — exposure
  fills a spare word).

## Changelog (vs the critic-review-1 revision)

- **C1 (CRITICAL) — binding cap 8 → 12.** The prior "@8 still ≤ cap" claim was FALSE: the marcher
  vocab set (`swapchain.rs:3493-3506`) ALREADY has 8 entries `@0..@7`, so `gViewT` as a 9th binding
  OVERFLOWS the cap of 8. `MAX_BIND_GROUP_BINDINGS` is raised 8 → 12 in BOTH `boyko_rhi/src/device.rs:22`
  and `boyko_rhi_vulkan/src/rhi_impl.rs:74` (const-asserted equal at `:80`) + the docstring. Re-verified
  the 8 is a self-imposed inline-array size, NOT a hardware limit (NVIDIA Ampere / RTX 3060
  `maxPerStageDescriptorStorageImages = 1048576`, `maxPerStageResources = 1048576`). Exhaustive
  touch-list enumerated in Decision 1 (2 const literals + 1 docstring; everything else scales via the
  named const; the pool histogram `KIND_COUNT=5` is untouched). Resolves W3 and OQ-1 (single resolve
  set, 10 ≤ 12).
- **C2 (CRITICAL) — `gViewT` at all THREE marcher exits + read-under-mask gate.** The marcher
  (`sdf_gbuffer_composite.hlsl`) has three terminal write sites across two return points: the P4b
  EMPTY early-return (`:444`, MISSED by the prior single-site plan) gets the `1.0e30` sentinel; the
  final block (`:623-625`) writes `(mask==1.0) ? t : 1.0e30` (real `t` from the SDF-hit arm `:588-590`,
  sentinel on mesh/background). The resolve + L1 froxel-z read `gViewT` STRICTLY inside `mask==1`, so a
  sentinel is never consumed. Added the EXACTLY-ONCE debug_assert + a full-frame coverage golden.
- **C3 (CRITICAL) — new rung L0-r0: async barrier-ordered re-upload.** `upload_initial`
  (`gpu_column.rs:833`) is setup-only/fence-waited; `dispatch_compute` (`:576`) ALSO fence-waits; even
  `GpuSystem` Wave C "submit+wait" (`gpu_system.rs:351-352`) — none is fence-free. The real async
  precedent is the per-frame `render_gbuffer_frame` (`swapchain.rs:2008`) command stream (marcher →
  store-to-load barrier `:2539` → resolve, one submit, no intra-frame fence). On-change re-upload is now
  a recorded staging→`light_table` `cmd_copy_buffer` + a TRANSFER_WRITE→SHADER_READ buffer barrier into
  that stream, recorded only on a dirty frame (idle frames cost zero). New rung L0-r0 + a
  `GpuColumnManager::record_upload` recorder; `upload_initial` is the FIRST-seed-only path.
- **W1 (op-order pin).** The L0a single-light expansion keeps the exact `golden_deferred_resolve`
  (`compute.rs:1863-1876`) op-order — accumulator from `0.0`, `(diff+spec)*(nol*shadow)*color`, `*
  exposure` literally last; identity at exposure==1.0 → bit-identical OFF path. No reassociation allowed;
  bit-exact OFF-path regression added.
- **W2 (R32_SFLOAT caps check).** Extend `DeviceCaps` (`boyko_rhi_vulkan/src/device.rs:1838-1841`) with
  `viewt_storage_format_ok` + `query_device_caps` (`:1819-1842`) to check `VK_FORMAT_R32_SFLOAT` /
  `STORAGE_IMAGE` (OPTIMAL tiling), mirroring `gbuffer_storage_format_ok`; fail-fast before the `gViewT`
  image at L0b.
- **W3 (folded into C1).** The old "@8 still ≤ cap" falsehood is corrected by the cap raise to 12.
- **O1 (cam_forward normalized contract).** `cam_forward.xyz` is contractually normalized host-side
  (`debug_assert!(|cam_forward.xyz| ≈ 1.0)`) for L1 PERSP view-z `dot(rd_world, fwd) * t`.
- **O2 (clamp-and-drop).** Per-froxel overflow at `MAX_LIGHTS_PER_CLUSTER` (256) drops extras (atomic
  bump clamped, no UB); debug asserts the cap; golden "no false drop under cap"; documented limit.
