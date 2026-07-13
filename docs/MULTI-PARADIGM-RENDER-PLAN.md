# Multi-Paradigm Render-Path Plan (Forward / Forward+ / Deferred / Visibility Buffer)

## Goal

Turn the single hardcoded deferred pipeline into a **boot-selected** matrix of `RenderPath × GeometryLegs`:

- **RenderPath** ∈ `Deferred` (current, DEFAULT) | `Forward` | `ForwardPlus` | `VisibilityBuffer` — chooses *how geometry becomes lit pixels*.
- **GeometryLegs** ∈ `Both` (DEFAULT) | `Mesh` | `Sdf` — chooses *which geometry producers exist*. A disabled leg allocates **no images, builds no pipelines, records no passes, binds no descriptors, sets no extra buffer-usage bits**.

Performance intent, grounded in the research:

| Path | Wins when | Mechanism | Target |
|---|---|---|---|
| Deferred | many lights, heavy screen-space stack, medium tris | 1 shade/px, fat-aux reuse | byte-identical to today |
| Forward | few lights, small scenes, low overdraw | no gbuffer write/read bandwidth; early-Z-clean | ≤ Deferred at ≤4 lights/px |
| ForwardPlus | many lights + forward variety | froxel reuse + **EQUAL-depth early-Z zero-overdraw** | ≥ Deferred at high light count |
| VisibilityBuffer | sub-pixel triangles (dense meshes) | material eval once/px, no 2×2 quad waste, 8 B/px VB | ~23-32% faster material pass @≤1px tris (Hable/DAIS), loses at coarse geometry (documented) |

Zero-cost leg toggle: a mesh-only app commits **0 bytes** of SDF VRAM and **0** marcher dispatches; an SDF-only app commits **0** vertex pipelines / instance rings / VB images / geometry-table slots, and MeshGpu buffers are created with **exactly today's usage bits** (no STORAGE bit).

## Context and constraints

**Affected subsystems:** `graph_bridge.rs` (declarators), `targets.rs` (`GBufferTargets` → path-conditional), `gbuffer.rs` + new pass bodies, `scene_types.rs` (variant predicates), `deferred_pbr.hlsl` + new shaders, `boyko_shaderdsl` (VB barycentric math), `boyko_render` config crate + `mesh_assets.rs`/`instance_model.rs`/`mesh_draw.rs` (bindless geometry table + VB-path instance lane), `boyko_app::runner.rs` + `gpu_scene/mod.rs` (boot-lock threading), `light.rs` (froxel SSBOs reused verbatim), `bindless.rs` (geometry-table sibling of the texture table), `gbuffer_depth.rs`/`compute.rs` (depth constants — read, not edited).

**Invariants preserved (INVIOLABLE):**
- **Byte-identity, every rung:** `RenderPath=Deferred, GeometryLegs=Both` (default) reproduces `58f6c6c3` (base, both cfg legs, hwrt on/off), `a5ad662d` (grand_showcase), `f6147f90` (2mat). New images **append last** to the fixed ResId order (`FRAMEGRAPH_IMAGE_COUNT` discipline); new passes are new `Option<PassId>`; the deferred declarator, its images, its resolve variant chain, `deferred_pbr.hlsl`'s **binding-count exact-fill sets (20/22/24)**, and the **Deferred instance SSBO row (48-byte `InstanceModelCol`)** are **not edited**.
- **Principle 0:** config = ECS Resources; all GPU data stays in existing VM-native mirrors. The C1 geometry store is **not a side table** — it is the device face of the existing `Assets<MeshGpu>` table (§Decision 0); the VB `mesh_id` is a lane **appended to the existing instance mirror** under the VB path only.
- **Principle 1:** path/legs **and the set of pre-light screen-space consumers** are resolved **once at `WindowHost::boot`** (`host.ssaa_armed` precedent) → `ResolvedRenderPath` carrier → per-path declarator + per-path pipeline handles selected once → **no per-frame branch on a config enum in any hot loop, and no per-frame change to framegraph shape**.
- **Single BRDF source:** Cook-Torrance/GGX exists **once** (`pbr_lighting.hlsli`); Forward FS, VB-shade compute, and SDF-forward shade all `#include` it.
- **Vulkan portability:** hardware-raster VB only. **No** `VK_KHR_fragment_shader_barycentric` (Intel Arc gap), **no** `shaderImageInt64Atomics`, **no** software rasterizer — analytic (Burns/DAIS) barycentrics work on every Vulkan 1.2 device. The VB geometry table needs only `shaderStorageBufferArrayNonUniformIndexing` (a widely-supported descriptor-indexing bit), and **degrades VB→Deferred at boot** if absent (§A). VB uses **4 descriptor sets**; Vulkan guarantees `maxBoundDescriptorSets ≥ 4`.

**Target metrics:** default path 0 added cost (byte-identical); disabled leg 0 VRAM/0 dispatch/0 extra usage bits; VB attribute reconstruction **bit-exact/pinned-ULP** against a host oracle (like `sdf_field_edsl_sync`); every descriptor set ≤ `MAX_BIND_GROUP_BINDINGS`(24) **per set per path** (worst set = 13, proven §G), never growing the deferred set.

**Correction to orchestrator input.** The constraints doc line 61 ("Uber vertex/index/transform/material SSBO mirrors ALREADY exist → VB attribute re-fetch is already served") is **false for vertex/index**: `MeshGpu` owns its own per-mesh `vertex_buffer`/`index_buffer` (mesh.rs:137-147, `VERTEX`/`INDEX` usage, no STORAGE bit), a draw binds them per-batch (gbuffer.rs:800/805), and there is **no** `vertex_base`/`index_base`/`mesh_id` addressing lane. Transform (`InstanceModelCol`) and material (`MaterialTable` SSBO + `PerInstanceMaterial(Tex)` ring) **are** id-addressable; textures are bindless (`BindlessTextureTable`, set 1). Therefore VB material/texture re-fetch is served, but **geometry re-fetch is not** — Decision 0 builds it as a scheduled foundation rung.

---

## Key decisions

### Decision 0 (resolves C1): Bindless per-mesh geometry table for VB attribute re-fetch

**What.** A `MeshGeometryTable`: a **bindless storage-buffer array** (`ByteAddressBuffer gMeshVerts[]`, `ByteAddressBuffer gMeshIndices[]`, plus a small `gMeshMeta[]` SSBO of `{index_width, vertex_count, index_count}`), one slot per registered mesh, slot index = `mesh_id`. Built as a direct sibling of `BindlessTextureTable` (free-list `BindlessSlotAllocator`, fence-gated recycle, reserved slot 0). Each `MeshGpu`'s existing `vertex_buffer`/`index_buffer` gains the `STORAGE_BUFFER` usage bit **at registration, but only when the boot-committed path is VisibilityBuffer with a mesh leg and the device supports the table** (P2-b: `ResolvedRenderPath.vb_geometry_table`); otherwise the buffers are created with exactly today's `VERTEX|INDEX` usage. A per-instance **`mesh_id: u32` lane is appended to the instance SSBO row under the VB path only**; `vb_shade`/`vb_geo`/`vb_resolve` map pixel `instance_id → mesh_id` (instance SSBO) → `gMeshIndices[mesh_id]` → 3 indices → `gMeshVerts[mesh_id]` → 3 vertices → analytic barycentric.

**Descriptor placement (P2-c).** The geometry arrays live in the **VB path's own descriptor Set 3**, a VB-only set. They are **NOT appended to `BindlessTextureTable`'s Set 1**, which VB binds **identically to the Deferred/Forward TEXTURED pipelines** (same set, same layout, unchanged) — so Deferred/Forward TEXTURED goldens cannot churn from this addition. The VB path therefore uses 4 sets (Set 0 core / Set 1 texture-table UNCHANGED / Set 2 shadow-GI-screen-space / Set 3 VB geometry); `maxBoundDescriptorSets ≥ 4` is the Vulkan guaranteed floor (boot `debug_assert`).

**Why.** A VB shading compute pass holds only `(instance_id, triangle_id)` per pixel and has **no per-draw buffer binding**; with each mesh's geometry in a distinct `VkBuffer` (mesh.rs:137-147) it physically cannot fetch the triangle — the single hard prerequisite every surveyed VB shares. The bindless-array mechanism (a) needs only `runtimeDescriptorArray` (already a **hard boot invariant**, bindless.rs:199-200) + `shaderStorageBufferArrayNonUniformIndexing` (near-universal), **not** `buffer_device_address` (which is **hwrt-gated** on this engine, device.rs:2640-2662 — so a BDA-based store would wrongly make VB require the hwrt feature); (b) **preserves the deep `MeshGpu`-owns-its-buffers invariant** (mesh.rs:4-19) — no asset-system re-architecture of F1/F6/F7; (c) reuses the proven F6/F7 fence-gated bindless recycle machinery verbatim; (d) is Principle-0 clean — the array **is** the device face of `Assets<MeshGpu>`, and `mesh_id` extends the existing instance mirror rather than adding a side store.

**Alternatives rejected.** (1) *Unified suballocated global vertex+index buffer with `vertex_base`/`index_base` lanes*: rejected — requires `MeshGpu` to stop owning standalone buffers, a deep, high-risk rewrite of the asset system's fence-gated free/grow (F6/F7) that contradicts a load-bearing invariant. (2) *BLAS `buffer_device_address` reuse* (critic O3): rejected as the **default** — BDA is hwrt-gated, so it cannot serve the non-hwrt VB path; noted only as a hwrt-only micro-optimization that composes cleanly.

**Trade-off.** VB requires `shaderStorageBufferArrayNonUniformIndexing` (per-pixel `mesh_id` is wave-non-uniform → `NonUniformResourceIndex`); absent → resolver degrades VB→Deferred at boot with a warn. One extra bindless set of two arrays + a meta SSBO **and** the STORAGE usage bit are paid **only** when the boot path is VB; zero cost otherwise (P2-b).

### Decision 1: Boot-committed `ResolvedRenderPath` (P-A VALIDATE)

Path + legs + the pre-light-consumer set resolved once at boot into an immutable `Copy` carrier; a live per-frame path/leg toggle is **forbidden** (re-allocates fixed-size images/pipelines — the `ssaa_armed` reason). Per-view seam kept open as parametric declarators; global-per-app ships first.

### Decision 2: One linear declarator per path + shared post-geometry tail (P-B VALIDATE)

4 declarators (`declare_deferred_graph` unchanged; `declare_forward_graph`; `declare_vb_graph`); the shared screen-space + present tail is factored into helper fns driven by **one predicate fn per pass used at BOTH declare and record sites** (O1 hard rule — the W1 lesson).

### Decision 3: Shared BRDF via `.hlsli`, image-golden-gated (P-C VALIDATE, O1)

Textual extraction of Cook-Torrance/GGX into `pbr_lighting.hlsli`; the authoritative R0 gate is the **image goldens** (a moved textual span changes `__FILE__`/`__LINE__` → dxc may emit non-identical SPIR-V for identical output); SPIR-V byte-cmp is a best-effort secondary check under `-Qstrip_debug`.

### Decision 4: DepthKind split restores early-Z (P-C1 fix, VALIDATE)

Deferred keeps its custom-linear depth **(both camera-mode-selected literals, §C)**; Forward/FwdPlus/VB use **standard hardware reverse-Z depth** (no `SV_Depth`) → early-Z live → ForwardPlus `DEPTH_EQUAL` zero-overdraw; consumers reconstruct view position via inv-proj.

### Decision 5: Pure-VB, no material G-buffer (P-C2 fix, VALIDATE)

VB never materializes albedo/metal/emissive; `vb_shade` re-fetches from the id via the Decision-0 geometry table + material ring. Only the thin-aux contract (normal/motion/roughness) persists cross-pass.

### Decision 6 (resolves W1): SDF leg participates in pre-light consumers under non-deferred paths via a geo/shade split

Under Forward/FwdPlus/VB, when a **pre-light consumer is armed** (SSAO/DDGI/shadow-denoise), the SDF leg splits into `sdf_geo` (march once, write thin-aux + a **thin SDF-surface cache** = the marcher's *existing today* albedo/normal/material channels, scoped to SDF pixels) **before** the tail, and `sdf_shade` (read the cache — **no second march** — inline BRDF) **after**. This gives true parity with Deferred+Both (SDF feeds SSAO/DDGI, receives denoised shadows). See §E for the staged rollout.

### Decision 7 (resolves W2): `ShadowSources` bitflags, not a single mode

Shadow at a shade site is multi-source (CSM PCF + punctual atlas + **SDF soft-march** + optional HW-RT vis), exactly as `deferred_pbr.hlsl` combines them today. Model it as flags armed structurally; the shade-site variant binds the armed sources and feeds the combined visibility into `eval_pbr_direct` (which takes shadow as an input, keeping the BRDF single-source). Restores Deferred's **default non-hwrt SDF soft shadow** to Forward/VB.

### Decision 8 (NEW — resolves W4): motion producer split by consumer phase

`motion_vec` has two consumer classes: a **pre-light** consumer (`shadow_temporal`, in the screen-space tail) and a **post-light** consumer (`taa_resolve`, in the present tail). The mesh Forward leg's inline shade pass (`mesh_forward`) runs **after** the screen-space tail, so if it were the sole motion producer, `shadow_temporal` would read a frame-stale motion. Resolution, following id Tech 6 (Doom 2016 writes velocity in the depth prepass): when a **pre-light motion consumer is armed**, the **`depth_prepass` writes a `motion_vec` MRT** (a depth+motion prepass) so motion precedes the tail; when only **post-light** motion consumers are armed, `mesh_forward` writes motion (cheaper, no prepass forced by motion alone). Under VB and the SDF split this is already correct (motion produced pre-tail by `vb_geo`/`sdf_geo`); the fix is scoped to the mesh Forward leg. Encoded as `ResolvedRenderPath.prepass_writes_motion`.

### Decision 9 (NEW — resolves VB1): triangle-id is normalized `% tri_count`, semantics-agnostic

`vb_raster.fs` stores the **raw** rasterizer-provided `SV_PrimitiveID` into `vb_id.G` (a system value, no VS export). Because every instance of a `DrawBatch` draws the **same mesh** (one `vkCmdDrawIndexed`, one shared index buffer, `tri_count = index_count/3`, a plain non-restart triangle list — engine-map line 16, mesh.rs), the in-mesh triangle index is recovered in the compute fetch (`vb_geom_fetch.hlsli`) as `local_tri = raw_prim_id % gMeshMeta[mesh_id].tri_count`. This is **provably correct under both possible SV_PrimitiveID semantics**: if it resets per instance, `raw < tri_count` and the modulo is identity; if it accumulates instance-major (`raw = inst*tri_count + local`, the only contiguous accumulation possible from repeating one index buffer), the modulo recovers `local`. No per-instance base-primitive lane and no VS export are needed. Pinned by a host↔shader convention constant + a mandatory **2-instance VB golden fixture** — **the fixture is the only real semantics gate (Rev 5)**: a bounds assert on `local_tri` is a tautology given the modulo. The fetch instead carries a `tri_count > 0` `debug_assert` (guards GPU-undefined `raw % 0`; safe by construction since a 0-index mesh draws no primitives, made explicit). This answers critic open-Q2: the design does **not** depend on per-instance-relative `SV_PrimitiveID`.

---

## Orchestrator position verdicts (P-A … P-I) + critic open questions

| # | Verdict | Note |
|---|---|---|
| **P-A** boot commitment | **VALIDATE** | Decision 1; extended to freeze the pre-light-consumer set (P2-d). |
| **P-B** one linear declarator/path | **VALIDATE** | Decision 2; O1 single-predicate rule. |
| **P-C** shared BRDF via `.hlsli` | **VALIDATE** | Decision 3. |
| **P-D** SDF forward-marched | **VALIDATE + EXTEND** | Decision 6 adds the geo/shade split (W1). |
| **P-E** thin-aux + structural arming | **VALIDATE** | §D; motion producer refined by Decision 8 (W4). |
| **P-F** SSAO ordering | **VALIDATE** | Forward+SSAO ⇒ depth(+normal[+motion]) prepass; VB ⇒ split; SDF ⇒ Decision 6. |
| **P-G** VB specifics | **OVERTURN format** | VB = **`R32G32_UINT`**; HW reverse-Z depth; raw `SV_PrimitiveID` + `% tri_count` normalize (Decision 9). §F. |
| **P-H** deferred leg-disable | **VALIDATE (simplify)** | Mesh-only Deferred = skip marcher only (O2 verified). |
| **P-I** per-path descriptor sets | **VALIDATE (3-set; VB 4-set)** | §G, all sets ≤13; VB adds a 4th VB-only geometry set (P2-c). |
| **Critic Q1** (W4 motion) | **ANSWERED** | Combined depth+**motion** prepass (id Tech 6) when a pre-light motion consumer is armed; else `mesh_forward` writes motion. Decision 8. |
| **Critic Q2** (VB1 SV_PrimitiveID) | **ANSWERED** | Design is semantics-agnostic via `% tri_count`; no base-primitive lane needed; 2-instance fixture verifies in CI. Decision 9. |
| **Critic Q3** (freeze vs over-arm) | **ANSWERED** | **Frozen at boot** under non-Deferred paths (structural); **live-toggle preserved under Deferred** (free there). P2-d. |

---

## A. Config surface

New module `boyko_render/src/render_path_config.rs` (mirrors `aa_config.rs`).

```rust
/// Owner-set Resource. Structural: enablement is the enum, no `bool` flags.
/// DEFAULT = Deferred + Both = today (byte-identity anchor).
#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderPathConfig { pub path: RenderPath, pub legs: GeometryLegs }
impl Default for RenderPathConfig {
    fn default() -> Self { Self { path: RenderPath::Deferred, legs: GeometryLegs::Both } }
}

#[repr(u32)] #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RenderPath {
    #[default] Deferred = 0,   // fat-MRT + compute resolve; custom-linear depth
    Forward = 1,               // raster FS shades inline, all-lights; hardware reverse-Z
    ForwardPlus = 2,           // depth prepass (EQUAL early-Z) + froxel inline FS
    VisibilityBuffer = 3,      // R32G32_UINT id raster + compute shade (needs geometry table)
}

#[repr(u32)] #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GeometryLegs { #[default] Both = 0, Mesh = 1, Sdf = 2 }
impl GeometryLegs {
    #[inline] pub const fn has_mesh(self) -> bool { !matches!(self, GeometryLegs::Sdf) }
    #[inline] pub const fn has_sdf(self)  -> bool { !matches!(self, GeometryLegs::Mesh) }
}
```

**Resolved carrier (boot-committed, immutable per boot):**

```rust
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)] #[repr(C)]
pub struct ResolvedRenderPath {
    pub path: RenderPath,
    pub legs: GeometryLegs,
    pub mesh_leg: bool,             // legs.has_mesh()
    pub sdf_leg: bool,              // legs.has_sdf()
    pub sdf_forward_marched: bool,  // sdf_leg && path != Deferred
    pub needs_depth_prepass: bool,  // ForwardPlus | (Forward & pre_light_consumers) — FULL union incl. MOTION-only shadow_temporal (Rev 5)
    pub prepass_writes_motion: bool,// Decision 8: pre-light motion consumer armed (shadow_temporal)
    pub mesh_geo_shade_split: bool, // VB & pre_light_consumers — same single predicate (Rev 5)
    pub sdf_geo_shade_split: bool,  // sdf_forward_marched & pre_light_consumer (Decision 6)
    pub sdf_surface_cache: bool,    // == sdf_geo_shade_split (thin SDF albedo/normal/material)
    pub vb_geometry_table: bool,    // path == VisibilityBuffer && mesh_leg && device supports it
    pub depth_kind: DepthKind,      // CustomLinear (Deferred) | HardwareReverseZ (others)
    pub thin_aux: ThinAuxMask,      // FROZEN at boot under non-Deferred paths (P2-d)
    pub shadow: ShadowSources,      // Decision 7; FROZEN at boot under non-Deferred paths
}

#[repr(u32)] #[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthKind { CustomLinear = 0, HardwareReverseZ = 1 }

bitflags! {
    #[repr(transparent)] pub struct ThinAuxMask: u32 {
        const NORMAL    = 1;   // octahedral, for SSAO/DDGI/shadow-denoise/SSR
        const ROUGHNESS = 2;   // 8-bit, for SSR (future); packed in thin_normal.BA
        const MOTION    = 4;   // RG16F, for TAA / shadow-temporal
        // depth ALWAYS present (D32 mesh + gViewT sdf), not a flag.
    }
    #[repr(transparent)] pub struct ShadowSources: u32 {   // Decision 7 / W2
        const CSM            = 1;  // directional cascades (bindings 12/13)
        const PUNCTUAL_ATLAS = 2;  // spot/point atlas (14/15)
        const SDF_SOFT_MARCH = 4;  // sdf_soft_shadow_ranged inline (edit-list); needs sdf_leg
        const HWRT_VIS       = 8;  // gShadowVis (denoised) or inline rayQuery; feature=hwrt
    }
}
```

**Resolver (`resolve_render_path`, pure fn, once at boot):** consumes `RenderPathConfig` + resolved consumer set (`ResolvedSsao`, `ResolvedAa` TAA bit, `CsmConfig`, `DdgiConfig`, shadow-temporal config, `feature="hwrt"`) + **device caps** (`shaderStorageBufferArrayNonUniformIndexing`) → `ResolvedRenderPath`. Single writer. **Rev 5:** the union `pre_light_consumers = ssao ∥ ddgi ∥ shadow_denoise_spatial ∥ shadow_temporal ∥ ssr` is computed ONCE and is the sole trigger for `needs_depth_prepass` / `mesh_geo_shade_split` / `sdf_geo_shade_split` (shadow_temporal is MOTION-only — see §D). `prepass_writes_motion = needs_depth_prepass && shadow_temporal_armed` (Decision 8).

**Validation rules (boot; degrade-not-panic):**

| Combo | Legal? | Resolution |
|---|---|---|
| `Deferred` × any legs | yes | as declared; pre-light consumers stay **live-toggleable** (fat gbuffer materializes normal/motion regardless — free) |
| `Forward/FwdPlus/VB` × `Mesh` | yes | mesh-only, no SDF cost |
| `Forward/FwdPlus/VB` × `Sdf` | yes | **collapses to `sdf_forward`-only** (O3): identical render → one shared golden `sdf_forward_only` |
| `Forward/FwdPlus/VB` × `Both` | yes (from R-SDFFWD) | mesh path + SDF forward leg composite |
| `VB` on device **without** `shaderStorageBufferArrayNonUniformIndexing` | **degraded** | `path→Deferred`, `warn` (Decision 0) |
| `Forward/FwdPlus/VB × {Both,Sdf}` **before R-SDFFWD lands** | **degraded** | `legs→Mesh`, `warn` |
| `VB × {Both,Sdf}` **before R-VBSDF lands** | **degraded** | `legs→Mesh`, `warn` |
| pre-light consumer armed (non-Deferred) | yes | sets `needs_depth_prepass`/`prepass_writes_motion`/`mesh_geo_shade_split`/`sdf_geo_shade_split` per path; **consumer set FROZEN at boot** (P2-d) |
| runtime toggle of a frozen pre-light consumer (non-Deferred) | **no-op** | `warn-once` ("frozen under RenderPath=X; rebuild to change"); value read at boot only |
| pre-light consumer + prepass suppressed by app flag | degraded | consumer → `Off`, `warn` |
| any path requesting native MSAA | rejected | out of scope; AA stays FXAA/SMAA/TAA/SSAA at resolve→present |
| resolution failure | fallback | `Deferred + Both` (byte-identical anchor) |

`GeometryLegs` has **no `None`** — an app with no 3D geometry does not compose the render plugin (`world.try_resource` graceful-degrade precedent).

**Boot-freeze semantics (P2-d, critic Q3).** Under **non-Deferred** paths the pre-light-consumer set (SSAO/DDGI/shadow-temporal) is **committed at boot** exactly like `ssaa_armed`, because it determines framegraph *structure* (prepass presence, `prepass_writes_motion`, geo/shade split, which thin-aux images exist). Runtime toggling one afterward is a warn-once no-op until the next boot. Rationale (hybrid-perf-decides): (1) Principle 1 — structural shape must not branch per-frame on config; (2) over-arming (always producing thin-aux for a maybe-off consumer) would pay motion/normal MRT bandwidth every frame, violating "disabled consumer costs zero"; (3) under **Deferred** the fat gbuffer materializes normal/motion unconditionally, so live toggling is *free there and is preserved*. Live where free, frozen where structural.

**Threading:** `runner.rs` reads `ResolvedRenderPath` via `world.try_resource` (degrade → default), commits it into `WindowHost` at boot beside `ssaa_armed`, threads the plain value into `GpuSceneBundles::scene()` → a `ResolvedRenderPath` field on `GBufferScene`. It selects the declarator + per-path pipeline handles once; never enters a per-draw hot loop. **Rev 5 (streaming-precise invariant):** the immutable `vb_geometry_table` flag is committed once at `WindowHost::boot` and is available at **EVERY** `MeshGpu` registration site — including meshes streamed in at any later runtime point (FULL STREAMING) — so the STORAGE-usage decision (P2-b) is uniform across the app's lifetime; the R-VBGEO gate asserts the flag is threaded to the registration site before the first mesh upload.

---

## B. Per-path framegraph

Dispatched once by `ResolvedRenderPath.path`:

```rust
fn declare_frame_graph(fg, scene) -> FramePlan {
    match scene.resolved_path.path {
        Deferred              => declare_deferred_graph(fg, scene),  // UNCHANGED — byte-identical
        Forward | ForwardPlus => declare_forward_graph(fg, scene),
        VisibilityBuffer      => declare_vb_graph(fg, scene),
    }
}
```

**Shared-tail helpers (one source of the W1-lesson predicate; O1):**
- `declare_screen_space_tail(fg, scene)` = `{ssao, ssao_atrous[], shadow_vis, shadow_atrous[], shadow_temporal, ddgi_update, light_cull, csm, atlas}`.
- `declare_present_tail(fg, scene)` = `{taa_resolve?, present_sample}`.
- **C5 discipline:** tail helpers declare **reads only** on `lit`, producer-layout-agnostic. Each path declares its own `lit` producer *access* (Deferred/VB/SDF-shade = `StorageWrite`/GENERAL; Forward = `ColorAttachmentWrite`/COLOR_ATTACHMENT_OPTIMAL). The framegraph derives the transition from the declared producer→consumer access pair. No tail helper hardcodes a source layout.
- **WAW ordering on `motion_vec` (Rev 5):** when `prepass_writes_motion` AND `sdf_geo_shade_split` are both armed, the prepass (mesh pixels, `ColorAttachmentWrite`) and `sdf_geo` (SDF pixels, `StorageWrite`) are two pre-tail WRITERS of one `motion_vec` image — a write-after-write pair. Both accesses MUST be declared so the framegraph auto-derives the `COLOR_ATTACHMENT_WRITE → SHADER_WRITE` barrier, exactly like the existing raster-MV → VIS-MV precedent (graph_bridge.rs:790-801, 1049-1061). Do not declare only the producer→consumer edges.
- **O1 hard rule:** each new pass's presence is decided by ONE shared `path_has_<pass>(scene) -> bool` fn called at **both** declare and record sites; a declare/record parity `debug_assert!` guards it.

### Deferred (unchanged; reference)
`interp? → tlas?(hwrt) → raster → light_upload? → coarse? → marcher → [screen-space tail] → resolve → [present tail]`. `raster`/`marcher` become `Option<PassId>` (Both keeps both `Some` → identical). Depth = custom-linear. **Not edited.**

### Forward / ForwardPlus
```
interp?
depth_prepass            (ForwardPlus always; Forward iff needs_depth_prepass)
                          -> D32 HW reverse-Z
                          (+ thin_normal MRT iff pre-light NORMAL consumer)
                          (+ motion_vec MRT iff prepass_writes_motion  [Decision 8, id Tech 6])
                          [earlydepthstencil-clean: no SV_Depth, no discard, no UAV;
                           color MRTs (normal/motion) do NOT defeat early-Z]
sdf_geo?                 (Decision 6, iff sdf_geo_shade_split: march once ->
                          gViewT + thin_normal + motion + SDF-surface cache; NO lit)
[screen-space tail]      (ssao reads D32(reconstruct)+thin_normal+SDF cache; light_cull; csm;
                          atlas; ddgi_update; shadow chain [shadow_temporal reads prepass motion]
                          -> gShadowVis  — SDF now included)
mesh_forward             (raster FS: shared BRDF inline; DEPTH_EQUAL if prepass else LESS;
                          samples gSsao + shadow sources; NO SV_Depth/discard/UAV -> early-Z live)
                          -> lit(COLOR) + thin_aux MRT(motion iff post-light-only; late normal?)
sdf_shade? / sdf_forward_march?
                          (split: read SDF cache + gSsao + gShadowVis -> inline BRDF -> lit;
                           fused: march + shade + composite in one (no pre-light consumer))
[present tail]           (taa_resolve reads motion — produced pre-tail by prepass OR by mesh_forward,
                          both precede present tail -> always current-frame)
```
Early-Z rationale: prepass writes standard hardware depth (no `SV_Depth`); `mesh_forward` runs `DEPTH_EQUAL` with a fragment shader that is a pure function of interpolated inputs (no `SV_Depth`/`discard`/UAV), so hardware early-Z rejects occluded fragments **before** inline lighting. Motion in the prepass is a **color MRT** and thus early-Z-safe; the VS exports curr/prev clip interpolants only when `prepass_writes_motion` (same cost the deferred `gbuffer_mrt_mv` variant already pays, relocated to the prepass). **Rev 5 — known, measured cost:** adding any color MRT to the prepass forfeits the hardware depth-only double-rate rasterization path a pure depth prepass would get; this is NOT free, and the scheduled `depth+motion prepass vs depth-only` benchmark quantifies it.

### VisibilityBuffer
```
interp?
vb_raster                (mesh HW raster) -> vb_id(R32G32_UINT) + D32 HW reverse-Z
                          (jittered projection for TAA; FS writes only SV_Target0 = id:
                          R=base_instance+SV_InstanceID, G=raw SV_PrimitiveID [Decision 9];
                          hardware depth test resolves mesh-vs-mesh; early-Z-clean; NO VS export)
sdf_geo?                 (Decision 6, iff sdf_geo_shade_split)
--- fused (mesh_geo_shade_split == false): ---
vb_resolve               (compute: geometry-table fetch + %tri_count + bary + interp + SampleGrad + shade)
                          -> lit + thin_aux(normal,motion?)     [NO material cache written]
--- split (mesh_geo_shade_split == true): ---
vb_geo                   (compute: fetch + %tri_count + bary + interp; SampleGrad only for thin-aux)
                          -> thin_aux(normal, motion?, roughness?)   [thin-aux ONLY; motion pre-tail]
[screen-space tail]      (ssao reads D32(reconstruct)+thin_aux.normal(+SDF cache); light_cull;
                          csm; atlas; ddgi_update; shadow chain [shadow_temporal reads vb_geo motion]
                          -> gShadowVis)
vb_shade                 (compute: RE-fetch + RE-%tri_count + RE-bary + RE-interp + RE-SampleGrad
                          albedo/metal/emissive; read gSsao + gShadowVis + froxel -> shared BRDF)
                          -> lit                                 [recompute, NOT read-from-cache]
---
sdf_shade? / sdf_forward_march?   (as Forward)
[present tail]
```
VB has no W4 hole: the fused `vb_resolve` runs only when no pre-light consumer is armed (a pre-light consumer forces the split), so its motion feeds only TAA (post-light); whenever `shadow_temporal` is armed VB is in split mode and `vb_geo` writes motion pre-tail.

**Pure-VB (Decision 5).** The split materializes **only the thin-aux contract**; `vb_shade` **re-fetches** material params via `(instance_id→mesh_id, triangle_id)` (Decision 0 table + material ring) — the Burns/Hable recompute-for-bandwidth trade. `roughness` in thin-aux is a **consumer output** (SSR), not a shading-input cache.

### Image ownership tables

Legend: **R** raster-written, **C** compute-written, **s** sampled-read, **u** storage-read.

**Forward / ForwardPlus:**

| Image | Format | Present when | Written | Read |
|---|---|---|---|---|
| `depth`(D32) | D32_SFLOAT, HW reverse-Z | always | prepass R / mesh_forward R (test) | ssao·s, sdf_geo·s (view-Z gate), consumers·s |
| `thin_normal` | R8G8B8A8_UNORM (oct RG + rough BA) | `thin_aux.NORMAL` | prepass R / sdf_geo C | ssao, ddgi, shadow-denoise |
| `motion_vec` | R16G16_SFLOAT | `thin_aux.MOTION` | **prepass R iff `prepass_writes_motion`, else mesh_forward R** / sdf_geo·C | taa (always), shadow_temporal (needs prepass producer) |
| `lit` | R8G8B8A8_UNORM | always | mesh_forward **R(COLOR)** / sdf_shade C | taa, present, AA |
| `gViewT` | R32_SFLOAT | `sdf_leg` | sdf_geo/sdf_forward C | consumers·s (SDF pixels) |
| `sdf_surf_albedo`,`sdf_surf_material` | R8G8B8A8_UNORM ×2 | `sdf_surface_cache` | sdf_geo C | ssao/shadow (tail), sdf_shade·u |
| `gShadowVis` (hwrt/denoise) | R16G16_UNORM | `shadow & HWRT_VIS` | shadow chain | mesh_forward·s, sdf_shade·u |
| `ssao`,`ssao_ring_*` | R8/R16_UNORM | ssao armed | ssao/atrous | mesh_forward, sdf_shade |
| (no `albedo`/`material`/`pbr` for **mesh**) | — | never | — | — |

**VisibilityBuffer:**

| Image | Format | Present when | Written | Read |
|---|---|---|---|---|
| `vb_id` | **R32G32_UINT** | `mesh_leg` | vb_raster R (COLOR) | vb_resolve/geo/shade·u |
| `depth`(D32) | D32_SFLOAT, HW reverse-Z | `mesh_leg` | vb_raster R | vb_*·s, sdf_geo, consumers |
| `thin_normal` | R8G8B8A8_UNORM | `thin_aux.NORMAL` | vb_resolve/geo C / sdf_geo C | ssao, ddgi, shadow |
| `motion_vec` | R16G16_SFLOAT | `thin_aux.MOTION` | vb_resolve/geo/sdf_geo C (always pre-tail in split) | taa, shadow_temporal |
| `lit` | R8G8B8A8_UNORM | always | vb_resolve/shade C / sdf_shade C | taa, present, AA |
| `gViewT` | R32_SFLOAT | `sdf_leg` | sdf_geo/sdf_forward C | consumers |
| `sdf_surf_albedo`,`sdf_surf_material` | R8G8B8A8_UNORM ×2 | `sdf_surface_cache` | sdf_geo C | ssao/shadow tail, sdf_shade·u |
| `gShadowVis` | R16G16_UNORM | `shadow & HWRT_VIS` | shadow chain | vb_resolve/shade·u |
| (no `vb_matcache`, no mesh `albedo`/`material`/`pbr`) | — | never | — | — |

**How `declare_gbuffer_graph` splits:** renamed `declare_deferred_graph`, left byte-for-byte. New declarators are new functions; new images append after the fixed ResId order (`FRAMEGRAPH_IMAGE_COUNT` grows monotonically); existing ResIds never shift. `GBufferTargets::sync_*` becomes path-conditional (`Option<[VulkanTexture;N]>` degrade): allocates only the active path's table (incl. the `sdf_surf_*` pair only when `sdf_surface_cache`).

---

## C. Shader inventory

**Rung-0 extraction (image-golden gated — O1/Decision 3):** cut the hand-written Cook-Torrance/GGX span from `deferred_pbr.hlsl` into `shaders/pbr_lighting.hlsli` (verbatim tokens); `deferred_pbr.hlsl` `#include`s it. Authoritative gate = image goldens; SPIR-V byte-cmp secondary/best-effort under `-Qstrip_debug`. Permanent BRDF seam.

`pbr_lighting.hlsli` public surface (all paths):
```hlsl
struct Surface { float3 albedo; float3 N; float roughness, metallic; float3 emissive; float ao; };
struct ShadowInputs { float vis; };  // combined CSM*atlas*sdf_soft*hwrt (Decision 7)
float3 eval_pbr_direct(Surface s, float3 V, LightSample l, ShadowInputs sh);
float3 eval_pbr_ambient(Surface s, float3 V, float ao, ...); // IBL/DDGI approx
// froxel walk + CSM/atlas PCF + sdf_soft_shadow_ranged reuse light_table.hlsli (unchanged)
```

**New shader files:**

| File | Stage | Includes | Notes |
|---|---|---|---|
| `pbr_lighting.hlsli` | — | `light_table.hlsli` | shared BRDF (extracted) |
| `thin_aux.hlsli` | — | `oct` (eDSL) | pack/unpack normal+rough+motion |
| `shadow_apply.hlsli` | — | `light_table`, `sdf_soft_shadow_ranged`(eDSL) | combines armed `ShadowSources` → one `vis` (Decision 7) |
| `vb_geom_fetch.hlsli` | — | `vb_bary`(eDSL) | bindless `gMeshIndices[]`/`gMeshVerts[]`/`gMeshMeta[]` fetch (Decision 0); **`local_tri = raw_prim_id % tri_count`** (Decision 9); u16/u32 index width |
| `depth_prepass.{vs,fs}.hlsl` | raster | — | `[earlydepthstencil]`-clean; `+normal` MRT variant; **`+motion` MRT variant (Decision 8), VS exports curr/prev clip** |
| `forward_opaque.{vs,fs}.hlsl` | raster | `pbr_lighting`,`light_table`,`thin_aux`,`shadow_apply` | inline shade; froxel (`#ifdef FROXEL`); early-Z live; motion MRT only when post-light-only |
| `vb_raster.{vs,fs}.hlsl` | raster | `vb_pack.hlsli` | FS writes only `SV_Target0=R32G32_UINT` = (base_instance+SV_InstanceID, raw SV_PrimitiveID); HW reverse-Z; jittered; **no VS export of ids** |
| `vb_resolve.hlsl` | compute | `pbr_lighting`,`thin_aux`,`vb_geom_fetch`,`shadow_apply` | fused fetch+bary+SampleGrad+shade |
| `vb_geo.hlsl` / `vb_shade.hlsl` | compute | as above | split; `vb_geo` thin-aux only, `vb_shade` re-fetches+shades |
| `sdf_forward_march.hlsl` | compute | `pbr_lighting`,`thin_aux`,`shadow_apply`, marcher eDSL | fused SDF forward leg |
| `sdf_geo.hlsl` / `sdf_shade.hlsl` | compute | as above | split (Decision 6); `sdf_geo` writes thin-aux + SDF-surface cache, `sdf_shade` reads cache + inline BRDF |
| `vb_pack.hlsli` | — | `vb_bary`(eDSL) | id pack/unpack + SDF sentinel |

**eDSL-authored math (`boyko_shaderdsl`, host oracle mandatory) — new module `emit/vb.rs`:**
- `emit_hlsl_vb_barycentric` — DAIS Appendix A: `D = det(p3−p2, p1−p2)`; `dλ_i/dx = (y_j−y_k)/D`, `dλ_i/dy = (x_k−x_j)/D` (constant per triangle). Determinants + one divide, transcendental-free.
- `emit_hlsl_vb_interp` — perspective-correct `a(x,y) = Σλ_i·a_i/w_i · [Σλ_i/w_i]^{-1}` + derivative chain-rule.
- `emit_hlsl_vb_uv_grad` — texcoord ∂/∂x,∂/∂y for `SampleGrad` (free from `vb_interp`).
- `emit_hlsl_near_clip` — simplified Blinn-Newell near-plane **shrink only** (DAIS: hardware ddx/ddy unstable as w→0; computed analytically).

**Oracle & tolerance (C4):** host f32 CPU mirror + `vb_bary_edsl_sync.rs` (pattern of `sdf_field_edsl_sync.rs`). Gate is **bit-exact / pinned-ULP** (B10G11R11 ≤2 ULP precedent; exact-equal where FMA order is pinned via `precise`) — same formula on two executors, a loose gate would mask real divergence. DAIS ~1e-5 analytic-vs-ground-truth is a **documented visual caveat**, never the reproducibility gate. Two hazards as explicit fixtures: (1) **McLaren/Hill `CalcFullBary` gradient bug** (Hable 2022 correction) — oracle encodes the *corrected* gradient + a regression case; (2) **near-clip Blinn-Newell shrink** — a near-plane-straddling triangle fixture proves stability where hardware ddx/ddy would not.

**Sync-pins (host↔shader, const-assert + test):**
- `VB_ID_SENTINEL = 0xFFFF_FFFF` (SDF-owned pixel marker in `vb_id.R`) — host mirror in `render_path_config.rs`.
- **Depth contract (C1 / P2-a — precise reconciliation):** the Deferred custom-linear depth machinery is governed by **two camera-mode-selected literals**, and Deferred references **both**, edits **neither**:
  - `MESH_DEPTH_T_MAX = 64.0` — the **perspective** normalizer (`gbuffer_mrt.fs.hlsl:113` ↔ `compute.rs:2210`; the marcher's `sdf_gbuffer_composite.hlsl:449/1439` uses it as `mesh_norm` when `camera_mode == CAM_PERSPECTIVE`; host-mirrored + tested in `instanced_vs_host_mirror.rs`).
  - `GBUFFER_T_MAX = SDF_TRACE_T_MAX = 10.0` — the **ortho / `T_MAX`** branch of the same mesh↔marcher agreement (`gbuffer_depth.rs:36/58` const-assert `GBUFFER_T_MAX == SDF_TRACE_T_MAX`; `compute.rs:2197`).
  Forward/FwdPlus/VB use **standard HW reverse-Z** on a **separate depth image allocation** and reference **neither** literal — their depth contract is the camera **inverse-projection** in the camera UBO. No new linear-depth sync-pin is introduced. Implementation note (R4/R8): any depth-touching code must confirm which literal governs before editing; the new paths never read or write either.
- **Geometry-table pins (Decision 0):** `Vertex` stride (64 B) and field offsets (position@0/normal@12/color@24/uv@40/tangent@48, mesh.rs:83) mirrored host↔`vb_geom_fetch.hlsli`; `mesh_id` lane offset in the VB-path instance row; `index_width` encoding + `tri_count = index_count/3` in `gMeshMeta`.
- **VB packing convention (Decision 9):** `instance_id` in R (`= base_instance + SV_InstanceID`), **raw** `triangle_id` in G (`= SV_PrimitiveID`); the in-mesh triangle is `raw % tri_count`. Convention + `tri_count` normalization pinned in `vb_pack.hlsli` + host const, verified by the **2-instance VB fixture**.

---

## D. Thin-aux contract (channel matrix per path × consumer)

Depth = (`depth` D32 for mesh pixels; `gViewT` for SDF pixels) — always present; Deferred mesh depth is custom-linear, Forward/VB is HW reverse-Z reconstructed via inv-proj. Consumers **arm structurally**: a consumer whose required channels are absent from `thin_aux` is forced `Off` at boot (and, under non-Deferred paths, the armed set is frozen — P2-d).

| Consumer | Needs | Deferred | Forward/FwdPlus | VisibilityBuffer | SDF participation (non-deferred) |
|---|---|---|---|---|---|
| TAA (post-light) | motion + depth | fat gbuffer motion | `motion_vec` MRT (prepass or mesh_forward) | resolve/geo writes `motion_vec` | sdf_geo/fused writes camera-reprojected motion |
| **shadow-temporal (pre-light, W4)** | motion + depth | raster MRT motion (pre-tail) | **prepass `motion_vec` MRT (`prepass_writes_motion`)** | `vb_geo` `motion_vec` (pre-tail, split) | sdf_geo motion (pre-tail, split) |
| SSAO (+à-trous) | depth + normal | gNormal | prepass `thin_normal` (⇒prepass) | `vb_geo` `thin_normal` (⇒split) | **via Decision 6 split** (sdf_geo before tail) |
| Shadow denoise (spatial) | depth+normal | gNormal | `thin_normal` | thin_aux normal | via Decision 6 split |
| SDFDDGI relight | depth + normal | gNormal | `thin_normal` | thin_aux normal | via Decision 6 split |
| **Shadow apply (W2)** | combined visibility | resolve combines CSM·atlas·SDF-soft·hwrt | `forward_opaque.fs` via `shadow_apply.hlsli` | `vb_resolve`/`vb_shade` via `shadow_apply.hlsli` | `sdf_shade`/fused via `shadow_apply.hlsli` |
| SSR (future) | depth+normal+rough+prev-lit | gNormal+pbr+lit-hist | `thin_normal`(rough BA)+lit-hist | thin_aux+lit-hist | — |

**Arming rules (boot):**
- `thin_aux.NORMAL` ⇐ (SSAO ∥ DDGI ∥ shadow-denoise ∥ SSR); `thin_aux.MOTION` ⇐ (TAA ∥ shadow-temporal); `thin_aux.ROUGHNESS` ⇐ SSR (packed in `thin_normal.BA`).
- **Single pre-light predicate (Rev 5 / final-critic P1):** ONE resolver fn `pre_light_consumers(...) -> bool` = (SSAO ∥ DDGI ∥ shadow-denoise-spatial ∥ **shadow-temporal** ∥ SSR) is the SOLE trigger for `needs_depth_prepass` (Forward), `mesh_geo_shade_split` (VB), and `sdf_geo_shade_split` (SDF) alike — three flags, one predicate, no drift. Rationale: `shadow_temporal` is a MOTION-only pre-light consumer (reads motion+viewt, NOT normal — graph_bridge.rs:1129), so gating the prepass/split on a NORMAL consumer alone would leave `Forward + hwrt shadows + ShadowDenoiseMode::Temporal + no SSAO/DDGI/SSR` reading frame-stale motion (the original W4 failure, re-opened). If a future rung makes shadow_temporal consume normal, that is a stated coupling to re-derive, not an accident.
- Forward: `pre_light_consumers` ⇒ `needs_depth_prepass`; the prepass writes depth (+`thin_normal` MRT iff a NORMAL consumer is armed, +`motion_vec` MRT iff `prepass_writes_motion`) so every pre-light consumer precedes lighting. A MOTION-only arming yields a depth+motion prepass with no normal MRT.
- **Motion producer (Decision 8 / W4):** `prepass_writes_motion` ⇐ (`needs_depth_prepass && shadow_temporal_armed`); when set, the **prepass** writes `motion_vec` (pre-tail) and `mesh_forward` does not. Else (only TAA needs motion) `mesh_forward` writes `motion_vec` (post-tail, still before the present tail → TAA current-frame). A `NORMAL`-only armed Forward with TAA and no shadow-temporal keeps the cheaper mesh_forward-motion form.
- VB: `pre_light_consumers` ⇒ `mesh_geo_shade_split`; in split mode `vb_geo` always writes motion pre-tail → shadow-temporal is automatically satisfied (including the MOTION-only arming).
- **SDF (Decision 6):** any pre-light consumer armed ⇒ `sdf_geo_shade_split` + `sdf_surface_cache` (sdf_geo before tail, sdf_shade after). Else the SDF leg stays fused-and-last.
- **`ShadowSources` (Decision 7):** `CSM` ⇐ CsmConfig; `PUNCTUAL_ATLAS` ⇐ punctual shadows; `SDF_SOFT_MARCH` ⇐ `sdf_leg && shadows && !hwrt_inline`; `HWRT_VIS` ⇐ `feature="hwrt" && (denoise|vis)`. The shade-site variant binds exactly the armed sources and combines them in `shadow_apply.hlsli` → one `vis` into `eval_pbr_direct`, reproducing Deferred's combination incl. the **default non-hwrt SDF soft shadow**.
- **Mesh albedo / full material params are NEVER materialized outside Deferred.** The `sdf_surf_*` cache is the **SDF leg's own** materialization (SDF pixels only), armed only under `sdf_surface_cache`, zero otherwise — not a mesh gbuffer, consistent with Decision 5.

---

## E. SDF leg treatment per path + mask-routing

**Deferred (unchanged):** marcher RW-composites into the fat gbuffer via ALU gate `own_pixel = !has_mesh || (hit && t < t_mesh)`; resolve routes on `gMaterial.b` mask bit.

**Forward / ForwardPlus / VB (P-D + Decision 6):**

- **Fused (no pre-light consumer):** `sdf_forward_march.hlsl`, after the mesh path:
  1. March the field (existing eDSL brick traversal/normal/soft-shadow).
  2. Read mesh `depth` (HW reverse-Z), **reconstruct view-Z via inv-proj**. Gate `sdf_owns = !mesh_leg || (hit && z_sdf < z_mesh_view)` in **view-Z** (invariant to sub-pixel TAA jitter → no silhouette shimmer, C6).
  3. Where `sdf_owns`: assemble `Surface`, evaluate **shared `pbr_lighting.hlsli` inline** (froxel + `shadow_apply.hlsli` + DDGI), write `lit`, `thin_aux`, `gViewT`. Under VB stamp `vb_id = VB_ID_SENTINEL`.
  4. Else predicated skip.

- **Split (pre-light consumer armed — Decision 6, W1 fix):**
  - `sdf_geo` (before tail): march once; write `gViewT` + `thin_normal` + `motion` (camera-reprojected, pre-tail — also feeds shadow-temporal) + the **thin SDF-surface cache** (`sdf_surf_albedo`, `sdf_surf_material` = the marcher's *existing* channels). No `lit` → no contention with `mesh_forward`'s COLOR `lit` write (C5).
  - Tail: SSAO/DDGI/shadow now see BOTH mesh thin-aux AND the SDF cache → **SDF participates fully**.
  - `sdf_shade` (after tail): read the cache back (**no re-march**), assemble `Surface`, inline BRDF reading `gSsao` + `gShadowVis` + froxel, composite into `lit` via the same view-Z ownership gate (StorageWrite; framegraph transitions `lit` COLOR→GENERAL between `mesh_forward` and `sdf_shade`).

**Why the SDF cache is not a Decision-5 violation.** VB can recompute mesh material cheaply from an id; SDF **cannot** (re-marching the field is the heaviest pass). Caching a thin SDF surface (the *same channels Deferred already writes for SDF pixels*) is the only way to both feed pre-light consumers AND avoid a double-march. Scoped to SDF pixels, armed only when needed, reusing Deferred's existing marcher output contract — a precedented, bounded cost.

**SDF motion (C6):** `sdf_geo`/fused writes `motion_vec` as **camera-reprojected** motion (SDF hit world-pos = origin + t·dir → previous-frame view-proj → NDC delta), the Doom-2016 static-geometry-from-depth pattern. Dynamic SDF-edit motion remains the same v1 limitation as today — **equal to, not worse than** current behavior; gate is jitter-invariant view-Z.

**Leg-disable semantics (P-H):**
- **Mesh-only:** skip `vb_raster`/`raster`/prepass, vertex pipelines, `PerInstanceMaterial*` rings, geometry-table slots, **and the STORAGE usage bit (P2-b)**. Under VB, `vb_id`+`depth` exist but no marcher; `sdf_forward_march` compiled `#ifdef HAS_MESH` off ⇒ owns every hit unconditionally.
- **SDF-only:** skip raster leg entirely (0 vertex pipelines/rings/VB image/geometry table/STORAGE bit). `sdf_forward_march`/`sdf_geo`+`sdf_shade` (non-deferred) or the marcher (Deferred) owns every hit. Forward/FwdPlus/VB × Sdf all resolve to one identical `sdf_forward_only` plan (O3) → one golden.
- **Fused `sdf_gbuffer_composite` (Deferred)** gains `#ifdef HAS_MESH` variants (compiled-variant idiom, like `TEXTURED`).

**Mask-routing for mesh-only Deferred (O2 — verified):** mesh raster FS **already writes `mask = 1`** (`gbuffer_mrt.fs.hlsl:5,295`), gbuffer **clears `mask = 0`** (`gbuffer.rs:386`, `material clear=(1,1,0,1)`, bit-identical to the marcher's neutral background). Therefore mesh-only Deferred = **skip the marcher pass only**; existing resolve routes on `mask` exactly as today. **No `RESOLVE_MESH_ONLY` variant, no FS change.**

---

## F. VB specifics

**ID packing — `R32G32_UINT` (overturns P-G's `R32_UINT`).**
- `R = instance_id` (u32) = global per-frame instance index (`base_instance + SV_InstanceID`) — the key already addressing the instance SSBO, `InstanceModelCol`, `PerInstanceMaterial(Tex)`, **and** (via the appended `mesh_id` lane) the Decision-0 geometry table. Width matches `base_instance:u32`. Computed in the FS from a push constant + `SV_InstanceID` (no VS export).
- `G = triangle_id` (u32) = **raw** `SV_PrimitiveID` (rasterizer-provided PS system value, **no VS export** → avoids the interplayoflight primitive-ID export cliff). The in-mesh triangle is recovered downstream as `raw % gMeshMeta[mesh_id].tri_count` (Decision 9 / VB1), correct under any `SV_PrimitiveID`-per-instance semantics because all instances of a `DrawBatch` share one index buffer.

**Bit-budget justification.** VB targets sub-pixel-triangle density, so meshes routinely exceed 2¹² triangles — any triangle-field truncation (Forge 12/20 or 8/23 split) breaks exactly VB's target workload; Forge's 8-bit drawID (256-cap) is the documented failure. Nanite avoids this only via fixed 128-tri clusters (7-bit id); we have **no meshlet/cluster system (out of scope)**, so `triangle_id` must carry a full mesh-triangle index. Full-width `R32G32_UINT` is the cap-free choice. Cost: 8 B/px → ~16 MB@1080p, ~64 MB@4K (id) + D32 — versus fat ~20 B/sample (~5× footprint reduction holds). **SDF sentinel:** `instance_id == VB_ID_SENTINEL`; real instance indices never reach `u32::MAX`.

**VB image format & depth:** `vb_id` = `R32G32_UINT` color attachment + separate `depth` = `D32_SFLOAT` **HW reverse-Z** (hardware depth test resolves mesh-vs-mesh — no 64-bit atomics, no software raster). FS writes only `SV_Target0` → early-Z-clean. Consumers reconstruct view position via inv-proj.

**Geometry fetch (Decision 0/9):** per pixel unpack `(instance_id, raw_prim_id)` → `mesh_id = instances[instance_id].mesh_id` → `tri_count = gMeshMeta[mesh_id].tri_count`; `local_tri = raw_prim_id % tri_count` → `gMeshIndices[NonUniformResourceIndex(mesh_id)]` (width from `gMeshMeta[mesh_id]`) → 3 indices → `gMeshVerts[NonUniformResourceIndex(mesh_id)]` ×3 → 3 clip-space vertices → `emit_hlsl_vb_barycentric` → `emit_hlsl_vb_interp` → `emit_hlsl_vb_uv_grad` → `SampleGrad` bindless. Hardware ddx/ddy rejected (unavailable in compute + DAIS near-clip instability).

**Resolve dispatch shape:** one full-screen compute dispatch, `8×8` tiles, one thread/pixel. **No per-material worklist** — one uber PBR material model (owner-validated); the classification seam is left open (`vb_classify` before `vb_shade` when material graphs arrive) but not built.

**TAA jitter interaction (+ O2):** `vb_raster.vs`/`depth_prepass.vs` **jitter the projection** (same push as `gbuffer_mrt`), so the mesh VB/forward leg is TAA-supersampled. Barycentric derivatives use jittered clip-space vertices + jittered pixel NDC consistently. **O2:** all within-frame inv-proj view-position reconstruction (SSAO/DDGI/shadow/SDF view-Z gate) uses the **same jittered projection** that wrote that frame's depth, so reconstructed positions are self-consistent within the frame; **motion** is computed jitter-removed (`unjittered_curr_ndc − unjittered_prev_ndc`) for TAA. The SDF leg stays unjittered with camera-reprojected motion (§E).

---

## G. Descriptor / pipeline budget per path (C3 / W3 / P2-c — exact worst-case)

**Descriptor-set layout per path** (Vulkan `maxBoundDescriptorSets ≥ 4` guaranteed; boot `debug_assert`):

- **Forward / ForwardPlus / SDF paths — 3 sets:**
  - **Set 0 — per-frame core** (camera/instance/material/light/froxel + pass I/O images)
  - **Set 1 — bindless texture table** (existing `BindlessTextureTable`, **bound identically to Deferred**, unchanged layout)
  - **Set 2 — shadow + GI + screen-space** (CSM/atlas/SDF-edit-list/DDGI/gSsao/gShadowVis)
- **VisibilityBuffer path — 4 sets (P2-c):** Set 0/1/2 as above **plus**
  - **Set 3 — VB-only geometry** (`gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta[]`). This is a **distinct VB-only set**; it is **not** appended to Set 1, so the `BindlessTextureTable` layout Deferred/Forward TEXTURED pipelines bind is byte-unchanged → no TEXTURED golden churn.

Deferred's 20/22/24 exact-fill sets are **never touched**.

**Exact worst-case counts (maximally armed: split + CSM + atlas + SDF-soft + DDGI + TAA + SSAO + HW-RT):**

| Path / pass | Set 0 | Set 1 | Set 2 | Set 3 | Max set |
|---|---|---|---|---|---|
| **Forward `mesh_forward.fs`** | camera, light table, ClusterGrid, LightIndexList, instance, material, per-inst-mat = **7** | tex = **1** | CSM(2)+atlas(2)+SDF-edit(1)+DDGI(3)+gSsao(1)+gShadowVis(1) = **10** | — | **10** |
| **`depth_prepass.fs`** (+normal +motion) | camera, instance (+material,per-inst-mat if normal MRT) = **2-4** | 0-1 | — | — | **≤4** |
| **VB `vb_geo`** | camera, instance, material, per-inst-mat, vb_id·u, depth·s, MotionCam, thin_normal·u, motion·u = **9** | tex = **1** | — | verts+indices+meta = **3** | **9** |
| **VB `vb_shade`** | camera, instance, material, per-inst-mat, light table, ClusterGrid, LightIndexList, vb_id·u, depth·s, lit·u, MotionCam = **11** | **1** | CSM(2)+atlas(2)+SDF-edit(1)+DDGI(3)+gSsao(1)+gShadowVis(1) = **10** | **3** | **11** |
| **VB `vb_resolve`** (fused, SSAO off) | +thin_normal·u+motion·u+MotionCam = **13** | **1** | CSM(2)+atlas(2)+SDF-edit(1)+DDGI(3)+gShadowVis(1) = **9** | **3** | **13** |
| **`sdf_geo`** (split) | camera, field, brick, edit-list, mesh-depth·s, gViewT·u, thin_normal·u, motion·u, sdf_surf_albedo·u, sdf_surf_material·u = **10** | 0 | — | — | **10** |
| **`sdf_shade`** (split) | camera, sdf_surf_albedo·s, sdf_surf_material·s, gViewT·s, light table, ClusterGrid, LightIndexList, lit·u, MotionCam = **9** | 0-1 | CSM(2)+atlas(2)+SDF-edit(1)+DDGI(3)+gSsao(1)+gShadowVis(1) = **10** | — | **10** |
| **`sdf_forward_march`** (fused — W3) | camera, field, brick, edit-list, mesh-depth·s, light table, ClusterGrid, LightIndexList, lit·u, gViewT·u, thin_normal·u, motion·u, MotionCam = **13** | tex = **1** | CSM(2)+atlas(2)+DDGI(3)+gSsao(1)+gShadowVis(1) = **9** (SDF-edit reused from Set0) | — | **13** |

Every set ≤ **13** < 24; VB set **count** = 4 = Vulkan floor (≥ headroom on every real GPU). Deferred's 20/22/24 sets remain byte-identical.

**Variant selection chains** (precompiled-`.spv` + fixed priority, per path, resolved to a pipeline handle **at boot**):
- Forward: `forward_opaque_[froxel]_[tex]_[mv]_[shadowmask]` by `ForwardPlus?`, `mesh_tex_active()`, `MOTION` arm, `ShadowSources` bits.
- Prepass: `depth_prepass_[normal]_[motion]` by `thin_aux.NORMAL`, `prepass_writes_motion`.
- VB: `mesh_geo_shade_split ? (vb_geo,vb_shade) : vb_resolve`; `_tex` by texture arm; `_shadow` by `ShadowSources`.
- SDF: `sdf_geo_shade_split ? (sdf_geo,sdf_shade) : sdf_forward_march`; `_hasmesh` by `mesh_leg`; `_shadow` by `ShadowSources`.
- Deferred: **untouched** chain (`denoised > hwrt_inline > software`).

---

## H. Rung-staged implementation plan

Each rung independently shippable, ordered by risk. **Every rung:** Deferred goldens byte-identical (`58f6c6c3` both cfg legs ±hwrt, `a5ad662d`, `f6147f90`); `clippy -D warnings`; full test suite; Miri where new unsafe; author-only commit+push.

| Rung | Deliverable | Risk | Gate (beyond standing gates) |
|---|---|---|---|
| **R0** BRDF extraction | `pbr_lighting.hlsli` textual cut + `shadow_apply.hlsli` scaffold; recompile 6 resolve `.spv` | low | **image goldens unchanged (authoritative)**; spv-cmp best-effort. No behavior |
| **R1** Config surface | `render_path_config.rs` (types+resolver incl. `DepthKind`/`ShadowSources`/split flags/`prepass_writes_motion`/`vb_geometry_table` device-cap degrade + **boot-freeze of pre-light consumers**), boot-lock in `runner.rs`, thread to `GBufferScene`; default Deferred+Both | low | resolver truth table (all combos + Sdf-collapse O3 + device-cap degrade + pre-rung degrades + arm masks + **freeze no-op warn** + **motion-producer rule**); goldens unchanged |
| **R2** Declarator split + leg Options | rename→`declare_deferred_graph`; `raster`/`marcher`→`Option<PassId>`; path-conditional `GBufferTargets` scaffold; **O1 single-predicate `path_has_*` fns + declare/record parity assert** | med | golden byte-identical (Both = both `Some`) |
| **R3** Deferred leg-disable | mesh-only (**skip marcher only**, O2) + sdf-only (`sdf_gbuffer_composite` `#ifdef HAS_MESH`); path-conditional VRAM | med | goldens `deferred_mesh_only`,`deferred_sdf_only`; **Both still `58f6c6c3`**; VRAM assert (mesh-only = 0 SDF bytes) |
| **R4** Forward (mesh-only) | `depth_prepass` (HW reverse-Z, early-Z-clean, **+normal/+motion MRT variants, W4**), `forward_opaque` FS (all-lights, `#include pbr_lighting`+`shadow_apply`), `declare_forward_graph`, thin-aux MRT, 3-set layout, inv-proj reconstruct; resolver forces `Forward×{Both,Sdf}→Mesh` until R-SDFFWD | high | golden `forward_mesh`; **early-Z invocation-count check** (with motion MRT present); **shadow-temporal-under-forward reads current-frame motion test (W4) — MUST include the `Temporal-only denoise, no SSAO/DDGI/SSR` config (MOTION-only arming, Rev 5)**; confirm no shared depth-state a 2nd reverse-Z pipeline violates (open-Q3); Deferred untouched |
| **R5** ForwardPlus | `#ifdef FROXEL` (reuse `cluster_cull` SSBOs verbatim), EQUAL-depth prepass, SSAO-via-prepass ordering | med | golden `forwardplus_mesh`; froxel SSBO byte-reuse assert; DEPTH_EQUAL zero-overdraw invocation check |
| **R-SDFFWD** SDF forward-march (fused) | `sdf_forward_march.hlsl`, view-Z reconstruct gate, camera-reprojected motion, shared BRDF+`shadow_apply` inline; unlocks `Forward/FwdPlus × {Both,Sdf}` (fused, SDF-last) | high | **GPU-vs-host bit-exact** view-Z reconstruct + gate; goldens `forward_both`,`sdf_forward_only`(O3-shared); TAA-on-SDF motion visual check |
| **R-SDFSPLIT** SDF geo/shade split (Decision 6/W1) | `sdf_geo`+`sdf_shade`, `sdf_surface_cache` images, `sdf_geo_shade_split` arming → SDF feeds SSAO/DDGI/shadow under Forward/FwdPlus | med | golden `forward_both_ssao` (SDF now AO'd/shadowed); assert fused path unchanged when no pre-light consumer; §D SDF-participation column flips no→yes |
| **R7** VB eDSL math | `emit/vb.rs` (bary/interp/uv-grad/near-clip) + host oracle + `vb_bary_edsl_sync.rs` (bit-exact/ULP; McLaren/Hill + near-clip fixtures) | high | **GPU-vs-host bit-exact/ULP-pinned**, no rendering yet |
| **R-VBGEO** Bindless geometry table (Decision 0/C1) | `MeshGeometryTable` (bindless `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta[]`, fence-gated recycle), **VB-boot-conditional `STORAGE_BUFFER` usage on `MeshGpu` buffers (P2-b)**, VB-path `mesh_id` instance lane, `vb_geom_fetch.hlsli` (incl. `%tri_count`) | high | **golden byte-identical (usage-bit gated on boot flag + VB-only instance lane do not touch Deferred/Forward)**; unit: geometry-table slot alloc/recycle under churn (F6/F7 harness); host↔shader `Vertex`/index-width/`tri_count` pins; device-cap degrade test; **assert non-VB boot creates MeshGpu with VERTEX\|INDEX usage only** |
| **R8** VB fused | `vb_raster` (R32G32_UINT+HW-Z, jittered, raw `SV_PrimitiveID`), `vb_resolve` (fused, uses R-VBGEO fetch), `declare_vb_graph`, 4-set (incl. Set 3); SSAO structurally off under VB this rung | high | golden `vb_mesh`; **2-instance VB fixture (VB1/Decision 9): instance>0 triangles resolve correctly**; TAA-on-VB visual check; Deferred untouched |
| **R9** VB geo/shade split | `vb_geo`+`vb_shade` (thin-aux only, **no matcache**), `mesh_geo_shade_split` arming; SSAO/DDGI under VB; `vb_geo` motion pre-tail | med | golden `vb_mesh_ssao`; assert fused path unchanged when SSAO off; **assert no mesh albedo/metal image exists**; shadow-temporal-under-VB current-frame-motion test — **incl. the Temporal-only (MOTION-only arming) config (Rev 5)** |
| **R10** VB + SDF | `sdf_forward_march`/`sdf_geo`+`sdf_shade` under VB (sentinel, view-Z composite); VB×Sdf collapse | med | goldens `vb_both`, `sdf_forward_only`(O3-shared); leg-cost asserts |
| **R11** Per-view seam (optional) | parametric declarators keyed by per-view `ResolvedRenderPath` | low | reflection-probe-view forward vs main deferred smoke; ship only if per-view adds no allocation |

Transparency/OIT, native MSAA, material-graph classification, VB skinning/pre-skin, GPU-driven culling, meshlet clusters: **seam left open, not implemented**. Extension points: `vb_classify` before `vb_shade`; forward-transparent pass reusing froxel SSBOs + shared depth; hwrt-only BDA geometry fetch as a micro-optimization on the R-VBGEO table.

---

## Data structures

```rust
// Decision 0 — device face of Assets<MeshGpu>; sibling of BindlessTextureTable.
// Principle-0 legitimate FFI/GPU store, NOT a parallel data system.
// Lives in the VB path's OWN descriptor Set 3 (P2-c) — NOT appended to the
// shared BindlessTextureTable (Set 1), which VB binds identically to Deferred.
pub struct MeshGeometryTable {
    verts:   VulkanBindlessSet,   // ByteAddressBuffer gMeshVerts[]   (VB Set 3) — one slot / mesh
    indices: VulkanBindlessSet,   // ByteAddressBuffer gMeshIndices[] (VB Set 3)
    meta:    BoundBuffer,         // gMeshMeta[]: {index_width:u32, vertex_count:u32, index_count:u32}
    alloc:   BindlessSlotAllocator, // free-list + fence-gated recycle (F6/F7 pattern)
    // slot 0 reserved (degenerate), mirroring BindlessTextureTable.
    // MeshGpu buffers get STORAGE_BUFFER usage ONLY when boot-committed vb_geometry_table (P2-b).
    // tri_count(mesh_id) = gMeshMeta[mesh_id].index_count / 3   (Decision 9 normalizer).
}

// VB-path instance row (path-conditional; Deferred/Forward keep the 48-byte InstanceModelCol).
#[repr(C)]
pub struct VbInstanceRow {
    affine: [f32; 12],   // row-major 3x4, byte-identical leading bytes (offset 0..48)
    mesh_id: u32,        // Decision-0 geometry-table slot; appended lane (offset 48)
    _pad: [u32; 3],      // pad to 64B for std430 stability
}
```

`ResolvedRenderPath` is `#[repr(C)]`, `Copy`, `Send+Sync`, ~44 B — read-only after boot, fits any cache line.

---

## Multithreading model

Unchanged from the existing renderer. Path/legs/consumer-set resolution is a **cold, single-threaded, once-at-boot** computation → immutable `ResolvedRenderPath`. No shared mutable state, no atomics added, no new lock. Framegraph records single-threaded; GPU parallelism is intra-dispatch. Data-race freedom: the resolved carrier is read-only after boot; only one path's declarator runs, so per-path images are disjoint; VB/forward/SDF compute read immutable SSBO mirrors + the bindless geometry table (immutable per frame — mesh registration/recycle is fence-gated on the setup path, F6/F7) and write disjoint output pixels (one thread/pixel); framegraph auto-barriers serialize `geo → tail → shade → present` from declared reads/writes (incl. the C5 per-path `lit`-producer access, the `sdf_surf_*` cache producer/consumer pair, and the Decision-8 motion producer/consumer ordering). `MeshGeometryTable` recycle uses the existing `RETIRE_DELAY` fence-gated slot free — no in-flight slot is overwritten.

## Integration

- **New:** `boyko_render/src/render_path_config.rs`; `MeshGeometryTable` (in `bindless.rs` + `mesh_assets.rs` wiring); `boyko_shaderdsl/src/emit/vb.rs`; shaders in §C; `declare_forward_graph`/`declare_vb_graph` + tail helpers + `path_has_*` predicate fns in `graph_bridge.rs`; new pass bodies in `present/passes/`; `tests/vb_bary_edsl_sync.rs`; per-path 3-set/4-set layout builders.
- **Changed:** `GbufferPassPlan.raster/marcher` → `Option<PassId>`; `GBufferTargets` → path-conditional (+`sdf_surf_*`); `GBufferScene` gains `resolved_path`; `runner.rs`/`gpu_scene/mod.rs` boot-lock threading; `FRAMEGRAPH_IMAGE_COUNT` grows (append-last); `sdf_gbuffer_composite` gains `#ifdef HAS_MESH`; **`MeshGpu` vertex/index buffers gain `STORAGE_BUFFER` usage only under the boot-committed VB path (P2-b)** (non-VB registration = today's `VERTEX|INDEX`, byte-identical); **VB-path-only** instance packing appends `mesh_id` (`mesh_draw.rs`/`instance_model.rs`).
- **Untouched (byte-identity):** `declare_deferred_graph` body; deferred resolve variant chain + its 20/22/24 exact-fill sets; `deferred_pbr.hlsl` bindings; `gbuffer_mrt.fs.hlsl` (mesh FS mask + custom-linear depth); the **48-byte `InstanceModelCol`** for Deferred/Forward; **`BindlessTextureTable` Set-1 layout** (P2-c); `cluster_cull.hlsl`; `light.rs` SSBO layout; `present_blit`/UI seam; `gbuffer_depth.rs` (`GBUFFER_T_MAX`/`SDF_TRACE_T_MAX`) and `compute.rs` `MESH_DEPTH_T_MAX` (referenced, not edited — P2-a).
- **Compatible with** `Arena`/`ComponentPool`/`UnitId`: config is a plain Resource; VB geometry table is the device face of `Assets<MeshGpu>` (Principle 0); VB uses existing VM-native mirrors + the appended `mesh_id` lane.

## Metrics and validation

**Benchmarks (criterion + GPU frame capture):** per-path frame time at low/medium/high triangle density (reproduce Hable's crossover — VB wins ~1px tris, may lose at coarse geometry, **documented not regression**); ForwardPlus early-Z invocation-count vs no-prepass; depth+motion prepass cost vs depth-only (W4 relocation, isolates the motion-in-prepass VS-export delta); leg-disable VRAM assert (mesh-only = 0 SDF bytes; sdf-only = 0 VB/geometry-table/STORAGE-bit bytes); descriptor-count assert (each path's each set ≤ 24, worst = 13; VB set count = 4); geometry-table fetch cost (non-uniform `mesh_id` indexing) vs a synthetic single-buffer baseline.

**Mandatory unit tests:** resolver truth table (all combos + Sdf-collapse + device-cap degrade + pre-rung degrades + arm masks + depth_kind + `ShadowSources` + **`prepass_writes_motion` rule (W4)** + **freeze-no-op warn (P2-d)**); `ThinAuxMask`/`ShadowSources` structural arming; `VB_ID_SENTINEL` host↔shader pin; `Vertex` stride/offset + index-width + **`tri_count = index_count/3`** geometry-table pins; `MeshGeometryTable` slot alloc/recycle under churn (F6/F7 fence-gated harness); **non-VB boot creates MeshGpu with VERTEX|INDEX usage only (P2-b)**; `vb_bary_edsl_sync` GPU-vs-host bit-exact/ULP + McLaren/Hill + near-clip fixtures; **2-instance VB golden fixture (VB1/Decision 9)**.

**`debug_assert!` invariants:** `instance_id != VB_ID_SENTINEL` for real instances; `mesh_id < geometry_table.len()` at fetch; `tri_count > 0` at fetch (Rev 5: `raw % 0` is GPU-undefined; safe by construction — a 0-index mesh draws no primitives so no pixel carries its mesh_id — but the assert makes that reasoning explicit; NOTE the former `(raw % tri_count)*3 < index_count` assert is a tautology given the modulo and is NOT a semantics gate — the **2-instance golden fixture is the only real Decision-9 gate**); ownership gate operates in reconstructed **view-Z**; resolved path's declarator matches its `GBufferTargets` variant + its `lit`-producer access + the shared `path_has_*` predicate at both declare/record sites (W1); `depth_kind == HardwareReverseZ` ⇒ no shader on that path writes `SV_Depth`; `prepass_writes_motion` ⇒ shadow-temporal's motion producer precedes the screen-space tail (W4); `sdf_surface_cache ⇔ sdf_geo_shade_split`; `vb_geometry_table` ⇒ device advertises `shaderStorageBufferArrayNonUniformIndexing` **and** `limits.maxBoundDescriptorSets >= 4`.

## Open questions

1. **Per-view (R11):** ship global-only if per-view costs anything beyond a per-view `ResolvedRenderPath` param on declarators. VALUES call to owner only if it forces extra allocation.
2. **`shaderStorageBufferArrayNonUniformIndexing` reach on the owner's actual GPUs:** near-universal on desktop AVX2 targets; the resolver degrades VB→Deferred if absent. If the owner's target set is known to lack it, VB is effectively unavailable there — a VALUES datapoint, not a design fork.
3. **Reverse-Z clear/compare on new pipelines (open-Q3 → verification item, not a fork):** Forward/VB pipelines use `GREATER` compare + clear `0.0` on **new** pipeline objects against the **path-conditional** depth image (a distinct allocation from Deferred's). R4 gate confirms no shared depth-state in `targets.rs` a second compare-op violates.

---

## Changelog (Rev 3 → Rev 4)

- **W4 (🟡 P1) FIXED — mesh-Forward motion-vector producer ordering.** Added **Decision 8** + `ResolvedRenderPath.prepass_writes_motion`. When a **pre-light** motion consumer (`shadow_temporal`) is armed under Forward/FwdPlus, `motion_vec` is written by the **depth prepass** (id Tech 6 depth+motion prepass — forward-plus.md line 12, hybrid-thin-gbuffer.md line 5), placing it before the screen-space tail; when only the **post-light** consumer (TAA) is armed, `mesh_forward` writes it (cheaper). VB (split `vb_geo`) and the SDF split already produce motion pre-tail, so the fix is scoped to the mesh Forward leg. Motion in the prepass is a color MRT → early-Z-safe (only SV_Depth/discard/UAV defeat early-Z). Updated §B (Forward framegraph + image ownership), §D (motion producer table + arming rule), §G (prepass variant), §H (R4 gate + W4 current-frame-motion test), debug_asserts.
- **VB1 (🟡 P1) FIXED — `SV_PrimitiveID` per-instance semantics pinned + made semantics-agnostic.** Added **Decision 9**: `vb_raster.fs` stores **raw** `SV_PrimitiveID` (system value, no VS export); the in-mesh triangle is recovered in `vb_geom_fetch.hlsli` as `raw % gMeshMeta[mesh_id].tri_count`, which is **provably correct whether `SV_PrimitiveID` resets per instance or accumulates instance-major** (all instances of a `DrawBatch` share one index buffer / `tri_count`). No per-draw base-primitive lane and no VS export needed (answers critic Q2). Pinned by a host↔shader convention + `tri_count` const, a GPU bounds `debug_assert`, and a mandatory **2-instance VB golden fixture** (R8 gate). Updated §F, §C sync-pins + `vb_geom_fetch.hlsli`, §H, tests, debug_asserts.
- **P2-a (🟢) FIXED — depth-contract citation reconciled.** Verified via source: the Deferred custom-linear depth machinery has **two camera-mode-selected literals** — `MESH_DEPTH_T_MAX = 64.0` (perspective; gbuffer_mrt.fs.hlsl:113 ↔ compute.rs:2210, host-mirrored in instanced_vs_host_mirror.rs) and `GBUFFER_T_MAX = SDF_TRACE_T_MAX = 10.0` (ortho/`T_MAX` branch; gbuffer_depth.rs:36/58 const-assert). Deferred references **both**, edits **neither**; Forward/VB use HW reverse-Z on a separate allocation and reference **neither**. Removes the "future editor touches the wrong literal" hazard. Updated §C sync-pins + Integration + implementation note.
- **P2-b (🟢) FIXED — STORAGE usage bit is VB-boot-conditional.** The `STORAGE_BUFFER` usage bit on `MeshGpu` vertex/index buffers is set **only** when the boot-committed `vb_geometry_table` is true (path=VB, mesh leg, device-supported); otherwise buffers are created with today's `VERTEX|INDEX` usage → byte-identical registration, zero cost when VB is not the boot path. Boot commit precedes any `MeshGpu` creation, so the flag is available. Updated Decision 0, Integration, R-VBGEO gate + test.
- **P2-c (🟢) FIXED — geometry arrays are a VB-only descriptor Set 3.** The `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta[]` arrays live in the VB path's **own Set 3**, NOT appended to the shared `BindlessTextureTable` (Set 1) — VB binds Set 1 identically to the Deferred/Forward TEXTURED pipelines, so TEXTURED goldens cannot churn. VB path = 4 sets (Vulkan floor `maxBoundDescriptorSets ≥ 4`, boot assert); Forward/SDF = 3. Updated §G (per-path set layout + re-costed table with Set 3), Decision 0, data structures.
- **P2-d (🟢) FIXED / critic Q3 ANSWERED — boot-freeze of pre-light consumers.** Under **non-Deferred** paths the pre-light-consumer set (SSAO/DDGI/shadow-temporal) is **frozen at boot** (ssaa_armed precedent) because it determines framegraph structure; runtime toggling is a warn-once no-op. Under **Deferred** it stays **live-toggleable** (fat gbuffer materializes normal/motion regardless — free). Chose freeze over over-arm on Principle-1 + zero-cost grounds. Updated §A validation + boot-freeze semantics, resolved carrier notes.
- **Critic open questions answered:** Q1 → combined depth+motion prepass (Decision 8); Q2 → `% tri_count` semantics-agnostic normalization, no base-primitive lane, 2-instance fixture (Decision 9); Q3 → frozen under non-Deferred / live under Deferred (P2-d).
- **Unchanged and re-affirmed:** all Rev 3 resolutions of C1 (Decision 0/R-VBGEO), W1 (Decision 6), W2 (Decision 7), W3 (§G enumeration), O1 (single predicate), O2 (jitter-consistent reconstruction), O3 (BDA rejected as default) stand. Golden hashes corrected to `a5ad662d` (was mistyped `a5ad642d` in two Rev 3 cells).

---

## Changelog (Rev 4 -> Rev 5, FINAL)

Final-verdict critique pass on Rev 4 found one P1 + four P2, all with prescribed
resolutions; applied by the orchestrator (internal-consistency corrections, no new
design decisions — the carrier fields already encoded the correct general intent):

- **P1 FIXED — pre-light trigger widened to the full consumer union.** The SS-D
  operational rules gated `needs_depth_prepass` (Forward) and `mesh_geo_shade_split`
  (VB) on a NORMAL consumer, but `shadow_temporal` is a MOTION-only pre-light
  consumer (motion+viewt, no normal — graph_bridge.rs:1129); the config
  `Forward + hwrt shadows + ShadowDenoiseMode::Temporal + no SSAO/DDGI/SSR` would
  have read frame-stale motion (W4 re-opened). Now ONE resolver predicate
  `pre_light_consumers = ssao | ddgi | shadow_denoise_spatial | shadow_temporal | ssr`
  drives all three flags (`needs_depth_prepass`, `mesh_geo_shade_split`,
  `sdf_geo_shade_split`) so they cannot drift; the R4/R9 W4 regression tests MUST
  include the Temporal-only (MOTION-only arming) config.
- **P2 FIXED — Decision-9 assert honesty.** `(raw % tri_count)*3 < index_count` is a
  tautology and gates nothing; the 2-instance golden fixture is the only real
  SV_PrimitiveID-semantics gate. Replaced with `tri_count > 0` at fetch
  (`raw % 0` is GPU-undefined; safe by construction, now explicit).
- **P2 FIXED — P2-b invariant restated for FULL STREAMING.** The `vb_geometry_table`
  flag is boot-committed and available at EVERY MeshGpu registration (including
  runtime-streamed meshes); R-VBGEO asserts the flag reaches the registration site
  before the first mesh upload.
- **P2 FIXED — motion_vec WAW ordering stated.** Prepass (mesh, color-attachment
  write) and sdf_geo (SDF, storage write) are two pre-tail writers of one image;
  both accesses must be declared so the framegraph derives the WAW barrier
  (raster-MV -> VIS-MV precedent).
- **P2 FIXED — Decision-8 cost labeled.** Motion MRT in the prepass forfeits
  depth-only double-rate rasterization; a known, benchmarked cost — not free.

## Design provenance

Produced 2026-07-13 by the architect/architecture-critic loop (4 architect
revisions x 4 critic passes, all remarks source-verified against the repo), seeded
by a 6-agent research sweep: Burns & Hunt 2013 (JCGT) + reference HLSL, Schied &
Dachsbacher DAIS 2015, Hable "Adventures in Visibility Rendering" (filmicworlds,
4 parts), The Forge triangle visibility buffer, Doom 2016 / id Tech 6 clustered
forward + thin g-buffer, Nanite deep dive (SIGGRAPH 2021), Bevy/Unity/Unreal
paradigm-selection surfaces, and a file:line map of this engine 
(current pass inventory, g-buffer contract, deferred-coupling audit).
Resolved critique tags referenced throughout: C1 (geometry table), W1 (SDF split),
W2 (ShadowSources), W3 (descriptor budget), W4 (motion ordering), O1 (single
predicate), O2 (mask routing / jitter-consistent reconstruction), O3 (BDA
rejected), VB1 (SV_PrimitiveID), P2-a..P2-d.
