# PBR Material + Texture Pipeline

> Status: PLAN (pre-implementation). Frozen-field contract is INVIOLABLE. This document is the full intended content of `docs/PBR-MATERIALS-PLAN.md`.
> Revision: r2 (post-critique). Changelog at the end.

## Goal

Add a maximally-performant, physically-based shading + material + texture pipeline to the in-house Vulkan 1.3 renderer, spanning both the SDF-native marcher half and the flat-color mesh half, **without perturbing the frozen `sdf_field.hlsli` distance field or the host/GPU determinism mirror**.

This plan ships in two scope tiers (the critique's C2/OQ-3 split):

- **THIS PLAN'S DELIVERABLE — MVP-A + MVP-B + MVP-C (flat deferred PBR).** No textures, no bindless, no DDS. The deferred-shading split, Cook-Torrance core, material-id G-buffer + material SSBO, oct-normal, and unified sRGB/tonemap. This is the low-risk, high-confidence core and needs **zero** new bindless RHI.
- **FOLLOW-UP PLAN — `docs/PBR-TEXTURES-PLAN.md` (textured PBR).** Decisions 7/8/9b (bindless arrays, in-house DDS reader, triplanar/UV sampling, compute LOD). Designed here at the decision level so the MVP's data structures are forward-compatible, but the bindless RHI work is **explicitly deferred** to its own ranked sub-plan because the descriptor-pool/feature/binding-flag changes are the bulk of that effort (C2).

Performance targets (RTX 3060, 1080p, ~2.07M px):
- **Deferred PBR resolve**: ≤ 0.6 ms fullscreen. Cook-Torrance core ~40-60 ALU/px + 1 material-SSBO fetch; bandwidth-bound on G-buffer reads (~14 B/px after the layout below → ~29 MB/frame, trivially under ~360 GB/s).
- **Zero per-frame heap allocation** on the render path (material SSBO at setup, written via persistently-mapped staging on change).
- **Material fetch**: 1 dependent SSBO load per shaded pixel.
- **0% regression** on the frozen marcher's distance/depth golden (BRDF is a strictly consumer-side swap; see C3 validation sequence).
- **Determinism preserved**: the *distance/depth* golden stays byte-exact; the *shading-color* golden moves to the resolve and keeps the existing ±2..3/255 tolerance.

## Context and constraints

Affected subsystems (all integration points verified against the code):

- `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` — **FROZEN, INVIOLABLE.** Material eval MUST NOT touch `field_distance`/`sdf`/`smin`/`combine`.
- `crates/boyko_sdf_math/src/lib.rs:118-209` — `SdfEdit` is 48 B / 12 words, pinned by 6 const-asserts + the shader `SDF_EDIT_WORDS == 12` pin. Growing it is a coordinated host+shader ABI change that trips a build error on desync. **We do not grow it.**
- `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` — the marcher; `load_edit` provably skips word 3 (`center.w (unused)`, line 75); writes `gAlbedo`/`gNormal`/`gMaterial` (`[[vk::image_format("rgba8")]] RWTexture2D<float4>`, line 102, written `float4`); composites the FINAL lit color into ALBEDO inline (A1+A2 folded).
- `crates/boyko_rhi_vulkan/src/compute.rs:442-499` — `host_ao` + `host_shade`; `host_shade` is the single factored Lambert+ambient golden helper, gated `lighting_flags`, ±2..3/255 tolerance. The shading golden checks the ALBEDO color the marcher composites.
- `crates/boyko_rhi_vulkan/src/swapchain.rs:3200-3438` — `GBufferTargets`: depth, raster_color (throwaway), albedo (STORAGE|SAMPLED), normal (STORAGE), material (STORAGE), all `R8G8B8A8_UNORM` (`GBUFFER_FORMAT`); `vocab_set` 7 entries (SSBO@0, sampled depth@1, storage albedo@2/normal@3/material@4, camera UBO@5, tiles SSBO@6); `present_set` 1 entry (CombinedImage albedo). Sets written ONCE per extent, no per-frame update.
- `crates/boyko_rhi/src/device.rs:22` — `MAX_BIND_GROUP_BINDINGS = 8`.
- `crates/boyko_rhi_vulkan/src/device.rs:1797-1842` — `query_device_caps`: `bindless_capable` (1.2 `descriptor_indexing && runtime_descriptor_array`) detected only; `gbuffer_storage_format_ok` checks `R8G8B8A8_UNORM` storage support and the caller fail-fasts on it; `p_enabled_features: null`, P1b enables NEITHER feature, the marcher uses explicit storage-image formats so `shaderStorageImageWriteWithoutFormat` is unneeded.
- `crates/boyko_rhi_vulkan/src/rhi_impl.rs:824,917-943` — `descriptor_count: e.count` already threaded from `BindGroupLayoutEntry.count`; but the pool is per-bind-group `max_sets: 1` with a stack-local kind-histogram `pool_sizes` array — **no** UAB pool, **no** `DescriptorBindingFlags`, **no** variable-count allocate-info.
- `crates/boyko_rhi_vulkan/src/abi_guard.rs:1-415` — `Format` discriminant == `VkFormat` constant, static-asserted; trivial to extend per new member.
- `crates/boyko_rhi/src/enums.rs:227-552` — `Format`; no BCn, no R16_UINT, no oct-normal format members; `TextureDesc`/`SamplerDesc` lack mips/array/LOD.
- `crates/boyko_render/src/lib.rs` — the only crate naming both ECS and RHI (orphan-rule home for `Material`/`MaterialId`/`TextureHandle`).

Invariants to preserve:
1. `field_distance(p).x` bit-identical → depth/distance golden + physics agreement.
2. `SdfEdit` const-assert fingerprint (build error on drift).
3. No `dyn`/`HashMap`/`Vec::new()`/`Box` on the render hot path; array-indexed-by-id only.
4. No external image/codec deps on the engine path.
5. `MAX_BIND_GROUP_BINDINGS = 8` — the resolve set MUST fit (proven below, Decision 1a) or the constant change is justified.

## Key decisions

### Decision 1: Deferred PBR resolve, not inline-in-marcher

**What**: Move shading OUT of the marcher into a dedicated fullscreen deferred-lighting compute pass (the planned P7 split). The marcher's job becomes: write depth + **oct-normal** + **material-id** to the MRT and stop compositing the lit color into ALBEDO. A new `deferred_pbr.comp` reads the G-buffer, fetches material params from a material SSBO by id, runs Cook-Torrance, applies A1 shadow + A2 AO as BRDF inputs, tonemaps, and writes the lit color to a dedicated output image. The mesh half writes the **same** G-buffer layout → both halves share one BRDF.

**Why**:
- **I-cache + per-ray cost**: the marcher sphere-traces per ray; folding a 40-60 ALU BRDF (and later texture fetches) into the engine's hottest shader pays BRDF cost in its shading branch and bloats the determinism-frozen TU's neighbor. Deferred pays BRDF cost exactly once per visible pixel; zero for missed/occluded rays.
- **Unification**: SDF + mesh getting identical Cook-Torrance is the deferred norm and eliminates a second shading path.
- **Determinism isolation**: the BRDF lives in a new `pbr.hlsli` consumed only by the resolve pass; it may use fast-math/rsqrt freely because it never feeds back into the distance field (material eval is a strict consumer of the surface hit).

**Alternatives**:
- *Inline forward in the marcher* (extend the A1/A2 site): rejected — pays BRDF per-march, no mesh unification, drags PBR ALU into the frozen TU.
- *Visibility-buffer / deferred-texturing*: rejected for v1 — compute has no `ddx/ddy`, needs explicit UV-gradient storage; over-engineered before textures exist.

**Trade-off**: deferred costs G-buffer bandwidth and forbids hardware MSAA (irrelevant; the engine is compute-marched). No transparency in the deferred pass (acceptable; a later forward-transparent pass is out of scope).

### Decision 1a: Resolve-pass descriptor-set binding tables — explicit, proven ≤ 8 (resolves C1)

The resolve pass is a **new shader with its own descriptor set** (set 0 of the resolve pipeline) — independent of the marcher's `vocab_set`. The marcher's 7/8 budget is irrelevant to it. The binding budget that matters is the resolve set's own. Enumerated for every phase:

**MVP resolve set (this plan) — 6 bindings, fits 8 with 2 free:**

| Binding | Kind | Resource |
|---|---|---|
| 0 | SAMPLED_IMAGE (+sampler) | depth (reconstruct world pos) |
| 1 | STORAGE_IMAGE (load) | oct-normal (RG16) |
| 2 | STORAGE_IMAGE (load, uint) | material-id (R16_UINT) |
| 3 | STORAGE_BUFFER | material SSBO (`MaterialGpu[]`) |
| 4 | UNIFORM_BUFFER | camera + light + sky params (one UBO; reuses the marcher's camera UBO struct, extended) |
| 5 | STORAGE_IMAGE (store) | lit output (R8G8B8A8 or R16G16B16A16 → present) |

This is allocatable by the existing `create_bind_group` / `max_sets: 1` per-bind-group pool path unchanged. No new RHI.

**Phase-4 textured resolve set (FOLLOW-UP plan) — restructured into TWO sets, NOT one 8+ set:**

The textured phase adds a bindless `SAMPLED_IMAGE[]` array and a `SAMPLER[]` array. Per the critique (C1/C2), **bindless arrays live in their own dedicated UAB set** (the current single-set pool does not model this). The textured resolve therefore uses:
- **Set 0** = the 6 MVP bindings above (unchanged, normal pool).
- **Set 1** = the bindless table: binding 0 `SAMPLED_IMAGE[16384]` (variable-count, UAB, partially-bound) + binding 1 `SAMPLER[N]`. Allocated from a **new** UAB descriptor pool (the follow-up plan's primary RHI work item).

This keeps every set ≤ 8 and **does not** change `MAX_BIND_GROUP_BINDINGS`. The cap invariant (item 5) is preserved.

**Decision: `MAX_BIND_GROUP_BINDINGS` stays 8.** No phase needs more than 6 in a single set.

### Decision 2: Material-ID in the G-buffer + a material SSBO indexed by id

**What**: `gMaterial` carries a **material id**; the resolve does `MaterialGpu m = materials[id]`. `MaterialGpu` is a POD 64-B struct (Data structures below). The SSBO is uploaded once / on-change (the Phase-5 `GpuColumnManager` pattern, zero per-frame readback).

**Why**:
- **Precision/decoupling**: packing metallic+roughness+id+emissive into 4×8 bits caps id ≤256 and roughness to 8-bit. An id → table indirection decouples material count and param precision from G-buffer width.
- **Data-oriented fit**: the table is the engine's "array indexed by id, one binding" pattern (mirrors `DeviceColumnHandle`). No `HashMap`, no per-material descriptor set.
- **Free path to textures**: the textured phase only adds `u32` index fields to `MaterialGpu`; the G-buffer write path is unchanged.

**Alternatives**: pack full PBR params into a widened G-buffer (rejected — costs bandwidth + a new image, caps params at texel width); RGBA8 id only / 256 materials (rejected — too few; see Decision 3).

**Trade-off**: 1 dependent SSBO fetch per shaded pixel. The cost claim is honest (see Decision 2a). Material id width drives Decision 3.

### Decision 2a: Material-SSBO fetch cost — L2-residency, NOT tile-coherence (resolves W4)

The earlier "coherent across a tile" justification is **withdrawn** for the SDF half. At smin blend seams, adjacent pixels resolve to different nearest-surface materials (Decision 4 picks nearest-surface) → the per-lane `materials[id]` fetch **diverges** at exactly the visually-busy regions, giving uncoalesced loads per lane. The honest justification: the entire table is **L2-resident** (64 B × 4096 materials = 256 KB; the 3060 has 3 MB L2), so even a fully-divergent wave hits L2, not VRAM. Worst case per shaded pixel is a divergent L2 load — still cheap. Mesh regions remain coherent (one material per draw) and coalesce naturally.

### Decision 3: Material-id G-buffer target = R16_UINT (16-bit id, 65 536 materials), gated by a format-capability check (resolves W1)

**What**: Change `gMaterial` from `R8G8B8A8Unorm` to `R16Uint`. The marcher/mesh declares `[[vk::image_format("r16ui")]] RWTexture2D<uint> gMaterial` and writes `uint material_id`; the resolve `.Load`s it.

**Capability gate (W1)**: add an R16_UINT `STORAGE_IMAGE` format-property check to `query_device_caps`, mirroring the existing `gbuffer_storage_format_ok` pattern (`optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT`), and **fail-fast** in the same caller. The codebase's own pattern demands this — we do not assume R16_UINT storage support, we query it. (Widely supported on desktop NVIDIA, but the check is mandatory by house style.)

**Format-feature commitment (W1)**: **keep the explicit `[[vk::image_format("r16ui")]]` declaration.** Therefore `shaderStorageImageWriteWithoutFormat` is **NOT** enabled — the integration bullet offering "drop the format / enable the feature" is **removed**. `p_enabled_features` stays null for the MVP. (The earlier plan offered both and didn't commit; this commits.)

**Why**: 256 materials (RGBA8) is too few; R16_UINT gives 65 536 at the same 2 B/px and removes float/sRGB ambiguity from an integer id.

**Alternatives**: R32_UINT (overkill, 4 B/px); RGBA8 packing (256-cap). Rejected.

**Trade-off**: a new `Format::R16Uint` member + abi_guard assert + the format-capability gate + the `GBufferTargets` material-format change. All localized.

### Decision 4: Per-edit material id lands in `SdfEdit`'s free `center.w` lane — NO stride change

**What**: Use word 3 (`center.w`, currently "unused", which `load_edit` provably skips at `sdf_field.hlsli:75`) to carry the per-edit `material_id` (bit-cast `u32`). The marcher reads `asuint(Buf[base+3])` for the id in a **separate** code path that field eval never touches.

**Why**:
- **Zero stride change** → the 48-B fingerprint, 6 const-asserts, and `SDF_EDIT_WORDS==12` pin all stay satisfied. No coordinated ABI churn.
- **Determinism preserved**: the id is read OUTSIDE `field_distance`/`sdf`. The frozen field math source is character-identical → DXC emits byte-identical distance SPIR-V for the field region (verified via the C3 tripwire below, not merely asserted).

**Alternatives**: grow `SdfEdit` (rejected — trips the fingerprint, forces a new golden, churns the std430 mirror for nothing when a free lane exists); `float2 sdf()` returning (dist, material) (rejected for v1 — risks perturbing the `.x` distance bits; the per-edit-id path needs no smin material blend yet).

**Trade-off**: per-edit material **blending across smin** is deferred. The polynomial smin is order-dependent; associative exp-smin would change the frozen field → an owner fork. v1 picks the **nearest-surface material** — the standard hard-union rule, already determined by `combine()`'s `argmin distance`. Smooth material lerp at seams is a Phase-2+ owner VALUE call (OQ-5).

### Decision 5: Octahedral normal encoding (RG16) for the G-buffer normal

**What**: Replace `n*0.5+0.5` RGB8 normal storage with octahedral-encoded RG16. Both halves encode to oct-RG; the resolve decodes. Add `Format::R16G16Unorm` (or `R16G16Snorm`) + abi_guard assert + a storage-format capability gate (same pattern as Decision 3).

**Why**: RGB8 `n*0.5+0.5` is ~8-bit/axis → visible specular banding once a view-dependent GGX highlight exists. Octahedral-RG16 gives ~16-bit angular precision at 4 B/px. A unified G-buffer wants one encoding. The SDF analytic normal is high-quality; this mainly protects the mesh half and the specular term.

**Alternatives**: keep RGB8 (rejected — specular banding); RGB10A2 (decent, but oct-RG16 is the precision/byte sweet spot and standard).

**Trade-off**: ~6 ops encode + ~6 decode (negligible) + a normal-target format change. This is part of the MVP because the GGX specular term (Decision 6) needs it — folding it in now avoids a re-bake of the normal golden later.

### Decision 6: Cook-Torrance subset — GGX + fast height-correlated Smith + Schlick + Lambert + analytic EnvBRDFApprox

**What** (the leanest high-quality subset, the Filament/Karis convergence):
- **Specular**: `D_GGX · V_SmithGGXCorrelatedFast · F_Schlick`, in visibility-folded form (`V` absorbs `1/(4·NoL·NoV)`).
- **Diffuse**: Lambert `albedo/π`.
- **Params (metallic-roughness)**: `α = perceptualRoughness²`, clamped `[0.045, 1.0]` (fp32 shading; no fp16 floor needed); `diffuseColor = (1−metallic)·baseColor`; `f0 = 0.16·reflectance²·(1−metallic) + baseColor·metallic`.
- **Direct light**: the existing one directional light; **A1 soft shadow → the direct visibility multiplier** (diffuse + specular), **A2 AO → the ambient term only**. A1/A2 become BRDF *inputs*, not a flat post-multiply.
- **Ambient/IBL (v1)**: analytic **EnvBRDFApprox** (Karis mobile / Narkowicz, ~10 ALU, no LUT, no prefilter, no env asset) for specular IBL against an **analytic sky** (uniform or hemisphere gradient) + a **uniform/hemisphere diffuse ambient × A2 AO**. Strongest fit for "fully in-house, no external deps."
- **Multiscatter**: deferred (the 1-mul Filament term needs a DFG `.y`; single-scatter first).

**Why**: the industry-converged real-time core (Filament/UE/Bevy) reduced to its leanest desktop form. The no-sqrt fast `V` + analytic DFG match "maximally performant" and "no env/LUT assets" exactly. fp32 in the deferred pass (off the frozen field) means no determinism concern.

**Alternatives**: full split-sum IBL (prefiltered cube + DFG LUT) — rejected for v1 (needs a cubemap asset + a precompute pass; conflicts with no-image-crates/minimal-alloc; defer to a reflection-probe phase). Disney diffuse / SG Fresnel — rejected (micro-opt, no visible desktop gain).

**Trade-off**: analytic sky reflection is "plausible," not ground-truth; rough-metal energy loss until multiscatter lands. Both acceptable, deferrable.

### Decision 7 (FOLLOW-UP PLAN): Textures via BINDLESS descriptor-indexing arrays — NOT auto-atlas

> Deferred to `docs/PBR-TEXTURES-PLAN.md`. Stated here so the MVP's `MaterialGpu` is forward-compatible. The bindless RHI work is the bulk of that plan (C2) and is NOT in this plan's deliverable.

**Recommendation (the owner asked for honesty)**: a single global bindless `SAMPLED_IMAGE` array (partially-bound, update-after-bind, variable-count, ~16K slots) + a small `SAMPLER` array, indexed by the `u32` slot fields in `MaterialGpu`. A CPU free-list hands out slots; writes batched at frame end behind the frame-fence. **Reject auto-atlas as the general texturing default.**

**Why bindless over atlas** (unanimous in research):
- Atlas's hard problems are mip-bleeding (gutters that scale with mip count), no per-tile `REPEAT` wrap (structurally impossible — one sampler for the whole image), repack-on-add (re-upload the whole atlas), and 4-px BC-block alignment. Bindless has none: per-texture mips, per-texture wrap, per-texture block alignment, O(1) add-one-descriptor.
- Bindless matches the engine's bare-handle data-oriented model exactly (the `DeviceColumnHandle` precedent).
- Descriptor indexing is Vulkan 1.2 core → guaranteed on the 3060 (already detected as `bindless_capable`, just not enabled).
- Atlas earns its place in exactly one future subsystem: UI / sprites / glyph cache (many small, no-wrap, few-mip) — there an in-house **skyline** packer is right. Not for 3D PBR materials.

**Sizing**: a fixed 16K-slot table at device init; under NVIDIA's ≤1M-descriptor / ≤2K-sampler budget.

**Scope honesty (C2)**: the bindless seam does **NOT** mostly exist. Only `BindGroupLayoutEntry.count` is threaded. The follow-up plan must scope as distinct work items: (1) a UAB descriptor pool (`VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT`, 16K `descriptor_count`, a new pool path — the current `max_sets:1` histogram pool does not model it); (2) `DescriptorBindingFlags` + `VkDescriptorSetLayoutBindingFlagsCreateInfo` + variable-count allocate-info (absent today); (3) **enable** the 1.2 descriptor-indexing feature struct at device creation (`p_enabled_features` currently null — UAB descriptors fault at create until chained); (4) `write_bindless_slot`; (5) `nonuniformEXT` discipline in-shader; (6) the free-list lifecycle tied to the frame-fence.

**Alternatives**: auto-atlas (rejected as default; owner VALUE call OQ-1); texture arrays (rejected as general path — uniform size+format; kept for terrain-splat/uniform sets); per-material descriptor sets (rejected — the CPU-bind-cost baseline bindless beats); virtual texturing (parked — only pays when texel data ≫ VRAM; the SDF-native model keeps detail analytic, so that workload doesn't exist).

**Trade-off**: more upfront RHI than one atlas image, but the correct, future-proof default.

### Decision 8 (FOLLOW-UP PLAN): Texture transport — in-house DDS reader feeding precompressed BC7/BC5; NO image/ktx2/basis crate

> Deferred to `docs/PBR-TEXTURES-PLAN.md`.

**What**: assets ship as **precompiled DDS** (built offline with texconv/Compressonator — a build-time tool, not a runtime engine dep). The engine has an in-house DDS reader: parse the 4-byte magic + 124-byte header + 20-byte DX10 extension, map `dxgiFormat → Format`, walk mips by block math, blit each level via `copy_buffer_to_image`. Formats: BC7_SRGB (albedo), BC5_UNORM (normals, reconstruct Z), BC4_UNORM (single masks), BC6H (HDR, later).

**Why**: the crux insight — decoding PNG/JPEG needs a real codec (DEFLATE/DCT) = forbidden-scale dep; reading a precompiled container is a byte-slice parse, then the already-BC bytes blit straight to the GPU (hardware-decoded, free at sample time). The in-house path is a **container reader, not an image decoder.** DDS over KTX2 for v1: DDS is the minimal parse (fixed 124-B header); require the DX10 extension for unambiguous format+colorspace. BCn-only matches desktop-x86_64 (ASTC mobile-only; Basis = forbidden-scale dep, no desktop benefit).

**LOC honesty (O2)**: the "~150-LOC" estimate covers the parse only. With fuzz-hardened bounds-checking against malformed headers, full DX10 handling, the dxgiFormat→Format match for every BC variant, the mip-walk, and barrier choreography, it is realistically more. Minor; doesn't change the decision.

**Alternatives**: KTX2 reader (good, heavier — deferred); `image`/`ktx2`/`basis` crates (violate no-deps); runtime PNG decode (forbidden codec); runtime BC encode / GPU mip-gen for compressed (illegal — BCn mips must be offline). All rejected.

**Trade-off**: requires an offline asset-build step (texconv/toktx) — a build-time tool, not a runtime dep (owner VALUE/SCOPE call OQ-2).

### Decision 9: sRGB/linear pipeline + tonemap unified across both halves

**What**: shade in **linear**, output through one OETF. Albedo/base-color & sky use sRGB encoding (flat base-color constants stored linear; BC7_SRGB textures linearize free on fetch in the textured phase). Data maps (normal/metallic/roughness/AO) are never gamma'd. The deferred resolve applies tonemap + the final OETF in **one place** (the resolve's output), feeding present. The swapchain stays `B8G8R8A8Unorm` with an explicit OETF in the resolve.

**Why**: a flat-color mesh path that skips linear-space mismatches the SDF path; double-sRGB is the classic bug. One tonemap/OETF site for both halves is the only correct unified-deferred convention. This is **in the MVP** (the flat path must already be linear-correct).

**Trade-off**: touch the present format/OETF once. Localized.

## Data structures

```rust
// crates/boyko_render/src/material.rs — the GPU material table element.
// SoA-friendly POD; uploaded once / on-change via GpuColumnManager (Phase-5).
// std430-compatible. A const-assert fingerprint pins the layout like SdfEdit,
// AND a shader-side word-count pin + documented std430 offsets mirror it (W2).
//
// LAYOUT DISCIPLINE (W2): all lanes are 16-B-aligned vec4/uvec4 groups so the
// std430 mapping in pbr.hlsli is unambiguous (no mixed-scalar greedy packing).
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct MaterialGpu {
    base_color: [f32; 4],   // off 0  : rgb linear + a (alpha/cutoff)        | vec4 lane 0
    emissive:   [f32; 4],   // off 16 : rgb linear + w unused                 | vec4 lane 1
    // metallic / roughness / reflectance / flags packed as ONE vec4 lane:
    mrr_flags:  [f32; 4],   // off 32 : [metallic, roughness, reflectance,    | vec4 lane 2
                            //           bitcast<f32>(flags)]
    // textured phase only (bindless slot indices; 0xFFFFFFFF = none):
    tex_idx:    [u32; 4],   // off 48 : [albedo_idx, normal_idx, mrao_idx,    | uvec4 lane 3
                            //           emissive_idx]
}                           // 64 B = one cache line; 4096 materials = 256 KB SSBO
const _: () = assert!(core::mem::size_of::<MaterialGpu>() == 64);
const _: () = assert!(core::mem::align_of::<MaterialGpu>() == 16);
const _: () = assert!(core::mem::offset_of!(MaterialGpu, base_color) == 0);
const _: () = assert!(core::mem::offset_of!(MaterialGpu, emissive)  == 16);
const _: () = assert!(core::mem::offset_of!(MaterialGpu, mrr_flags) == 32);
const _: () = assert!(core::mem::offset_of!(MaterialGpu, tex_idx)   == 48);
// pbr.hlsli MUST declare:  static const uint MATERIAL_GPU_WORDS = 16;  (4 vec4 lanes)
// and document each lane offset, mirroring SDF_EDIT_WORDS==12 in sdf_field.hlsli.
```

```rust
// crates/boyko_render/src/material.rs — the ECS-side authoring component (cold, setup-time).
// MVP-A ships the flat subset; the four TextureHandle fields are FOLLOW-UP-PLAN-reserved
// (O1): present in the type but documented as "ignored until the textured phase".
#[repr(C)]
pub struct Material {
    pub base_color: [f32; 4],
    pub emissive:   [f32; 3],
    pub metallic:    f32,
    pub roughness:   f32,
    pub reflectance: f32,
    // --- FOLLOW-UP PLAN (textured); ignored by the MVP resolve ---
    pub albedo:       TextureHandle,
    pub normal:       TextureHandle,
    pub mrao:         TextureHandle,
    pub emissive_tex: TextureHandle,
}

/// A material-table index handed to the G-buffer (16-bit id range, R16_UINT target).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MaterialId(u16);

/// A bindless slot handle — a bare u32 (mirrors DeviceColumnHandle's bare-u64 style).
/// FOLLOW-UP PLAN only. 0xFFFFFFFF = none/unbound.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TextureHandle(u32);
```

```rust
// crates/boyko_render/src/bindless.rs — FOLLOW-UP PLAN ONLY (docs/PBR-TEXTURES-PLAN.md).
// Listed here for forward-compat visibility; NOT built in this plan's deliverable.
pub struct BindlessTable {
    layout: VulkanBindGroupLayout,      // set 1: variable-count SAMPLED_IMAGE[] + SAMPLER[]
    set:    VulkanBindGroup,            // update-after-bind, partially-bound (new UAB pool)
    free_list: Box<[u32]>,              // preallocated free-slot stack (no Vec growth)
    free_top: u32,
    capacity: u32,                      // 16384
    pending_free: Box<[(u32 /*slot*/, u64 /*frame*/)]>,  // deferred-release ring (frame-fence)
    pending_top: u32,
}
```

```rust
// crates/boyko_rhi/src/enums.rs — new Format members (discriminant = VkFormat const,
// each guarded in abi_guard.rs by a static_assert == the VkFormat constant).
// MVP (this plan):
//   R16_UINT       = 74   (material-id target)
//   R16G16_UNORM   = 35   (oct-normal target)
// FOLLOW-UP PLAN:
//   BC7_SRGB = 146, BC7_UNORM = 145, BC5_UNORM = 141, BC4_UNORM = 139, BC6H_UFLOAT = 144
```

```rust
// crates/boyko_rhi/src/device.rs — TextureDesc/SamplerDesc extensions.
// MVP needs NEITHER (the new G-buffer targets are mip_levels=1, array_layers=1).
// These are FOLLOW-UP PLAN additions, listed for forward-compat:
pub struct TextureDesc { /* ...existing...,*/ mip_levels: u32, array_layers: u32 } // FOLLOW-UP
pub struct SamplerDesc {
    /* ...existing...,*/ mipmap_mode: Filter,
    min_lod: f32, max_lod: f32, mip_lod_bias: f32, max_anisotropy: f32,            // FOLLOW-UP
}
// DescriptorBindingFlags (PARTIALLY_BOUND | UPDATE_AFTER_BIND | VARIABLE_COUNT) — FOLLOW-UP.
```

## Public API

```rust
// boyko_render — ECS-facing (setup-time; no hot-path alloc). MVP subset:
pub fn register_material(world: &mut World, m: Material) -> MaterialId;   // u16 id into the SSBO
pub fn upload_material_table(ctx: &RhiContext, table: &[MaterialGpu]);    // once / on-change

// SDF authoring — per-edit material via the free lane (NO stride change). MVP:
impl SdfEdit { pub fn with_material(self, id: MaterialId) -> Self; }      // packs id into center.w

// FOLLOW-UP PLAN (textured) — listed for shape, NOT in this deliverable:
pub fn load_texture_dds(ctx: &RhiContext, bytes: &[u8]) -> TextureHandle;
fn create_bindless_layout(&self, capacity: u32) -> Result<BindGroupLayout>;
fn write_bindless_slot(&self, set: &BindGroup, slot: u32, view: &Texture, sampler: &Sampler);
```

## Algorithms for critical paths

**Deferred PBR resolve (fullscreen compute, once per visible px) — MVP**
1. `.Load` depth → reconstruct world pos (the camera UBO @ binding 4).
2. `.Load` oct-normal → decode `n`; `.Load` material-id (R16_UINT, `uint`).
3. `MaterialGpu m = materials[id]` — 1 dependent SSBO fetch (L2-resident; Decision 2a).
4. Unpack `m.mrr_flags`: `metallic, roughness, reflectance, flags=asuint(.w)`. Compute `diffuseColor`, `f0`, `α=clamp(roughness²,0.045,1)`.
5. Direct: `D_GGX·V_fast·F_Schlick` + `Lambert`, × NoL, × **A1 shadow** (the shadow consumer reads the FROZEN field gateway, unchanged).
6. Ambient: `EnvBRDFApprox(f0,roughness,NoV)·sky_spec + diffuseColor·sky_diffuse` × **A2 AO**.
7. + emissive; tonemap; OETF; store to the lit output (binding 5).
- **Complexity**: O(visible px). **Cache**: sequential G-buffer reads (streaming); 1 small SSBO fetch/px, L2-resident even when divergent at seams. **Branching**: minimal — `flags` bit-tests select an index or a constant branchlessly (`lerp`/`select`); metallic kills diffuse via `(1−metallic)` multiply. **SIMD**: fullscreen compute is wave-parallel; the BRDF is per-lane scalar, vectorizes across the wave.

**In-house DDS load (setup-time, cold) — FOLLOW-UP PLAN**
1. `from_le_bytes` parse magic+header (+DX10). `// SAFETY` on every slice→scalar read; NO transmute over unaligned file bytes; every field bounds-checked.
2. Map `dxgiFormat → Format` (small match).
3. Per mip: `bytes = max(1,⌈w/4⌉)·max(1,⌈h/4⌉)·blockBytes`; blit via `copy_buffer_to_image` (`TRANSFER_DST_OPTIMAL` → barrier → `SHADER_READ_ONLY_OPTIMAL`).
4. Acquire a bindless slot from the free-list; `write_bindless_slot`.
- **Complexity**: O(total texels). **Cache**: streaming copy. **Branching**: none in the hot blit loop.

**Compute-pass texture LOD selection — FOLLOW-UP PLAN, FLAGGED UNSOLVED (W3)**
Compute has no `ddx/ddy`. For an SDF surface reached by sphere-marching with triplanar projection, deriving a correct per-pixel mip LOD without screen-space derivatives is **genuinely hard** — you need the world-space texel footprint projected through the camera, per triplanar axis. `SampleLevel` does NOT make this trivial; it only moves the burden to computing the level. This is explicitly an **open sub-problem of the follow-up plan**, not a solved detail. Fallback options for the developer (to be designed in that plan): a manual gradient via neighboring-tile world-position differences within the compute group, or a depth-derived footprint approximation, or (worst-case stopgap) a fixed LOD bias. The mesh half can use interpolated UVs + analytic gradients and is not affected.

## Multithreading model

- **Material table writes**: single-threaded, at setup or in the apply-window (the RHI records single-threaded). No locks.
- **Material SSBO upload**: written before the frame that reads it (host-side ordering + a barrier).
- **Bindless slot lifecycle (FOLLOW-UP)**: acquire/release O(1) on a preallocated stack; release goes through a deferred ring keyed by the **frame-fence** so a slot referenced by an in-flight command buffer is never overwritten (never mutate a live descriptor slot in-flight). This is the existing frame-fence/deferred-destroy discipline.
- **Resolve pass**: GPU-parallel by construction; the host only records the dispatch.
- **Data-race freedom**: no shared mutable host state on the render hot path → no atomics beyond the existing frame-fence. The MVP adds no new shared state at all (the material SSBO is write-before-read).
- `Send`/`Sync`: `MaterialGpu` upload and (later) `BindlessTable` stay on the render thread, consistent with the engine's `!Send` GPU-access discipline (Phase-5 `DispatcherToken`).

## Integration

Interacts with: `sdf_gbuffer_composite.hlsl` (write `uint material_id` from `asuint(Buf[base+3])`; write oct-normal; **stop compositing the lit color into ALBEDO**), a new `deferred_pbr.comp` + `pbr.hlsli`, `swapchain.rs::GBufferTargets` (material target → R16_UINT, normal target → R16G16; add the resolve set + dispatch + the lit-output image), `compute.rs` (the host BRDF mirror for the shading golden; the field golden untouched), `boyko_render` (new `material.rs`; `bindless.rs`/`dds.rs` are FOLLOW-UP), `GpuColumnManager` (material SSBO upload), `boyko_serialize` (persist `Material`).

Existing-code changes (MVP):
- `enums.rs`: + `R16Uint` (74), `R16G16Unorm` (35) + abi_guard asserts.
- `device.rs` (vulkan) `query_device_caps`: + R16_UINT and R16G16 `STORAGE_IMAGE` format-capability checks (the W1 gate), fail-fast in the caller. **No** feature enable; `p_enabled_features` stays null.
- `swapchain.rs`: material-target format → R16_UINT; normal-target format → R16G16; add a lit-output STORAGE image; build the resolve descriptor set (6 bindings, Decision 1a) + record the resolve dispatch; the present-blit now samples the **lit output** instead of ALBEDO.
- `sdf_gbuffer_composite.hlsl`: declare `[[vk::image_format("r16ui")]] RWTexture2D<uint> gMaterial`; write the id; oct-encode the normal; remove the inline A1/A2 composite into ALBEDO (ALBEDO becomes a pure base-color attribute the resolve consumes).
- `compute.rs`: keep `host_shade`/`host_ao` as the field-consumer mirror; add a host PBR mirror used by the *resolve* shading golden (the field/depth golden stays on the existing path).

Existing-code changes (FOLLOW-UP, NOT this deliverable): `enums.rs` BC formats; `texture.rs` mips/array/staging; `rhi_impl.rs` sampler LOD/aniso + UAB pool + variable-count layout; `device.rs` enable 1.2 descriptor-indexing features.

New modules: `boyko_render::material`, shaders `deferred_pbr.comp` + `pbr.hlsli` (MVP); `boyko_render::{bindless,dds}` (FOLLOW-UP).

## Determinism + perf

- **Field/distance**: untouched. `field_distance(p).x` is bit-identical because the id is read outside it and the field source text is character-identical. Proven by the C3 tripwire, not asserted.
- **Shading**: moves to the resolve in fp32 (off the frozen field); the shading golden keeps ±2..3/255.
- **Perf**: resolve ≤ 0.6 ms @1080p; zero per-frame heap alloc; 1 L2-resident material fetch/px; 0%-regression bench on the marcher (the BRDF left it).

## Validation sequence — C3 golden split (the field tripwire)

The single highest-consequence claim. The current golden composites the lit color into ALBEDO, so removing inline shading perturbs that color golden. Sequence:

1. **FIRST, split the golden (must precede MVP-A).** Determine whether `golden_composite_pixel` couples distance/depth with shading-color. If coupled, split into:
   - **`golden_depth` / `cpu_gpu_sdf_agreement`** — asserts the *distance/depth* output. This is the REAL field tripwire and MUST stay **byte-exact** across all MVP work.
   - **`golden_shading_color`** — asserts the *lit color*. This moves to the resolve output and shifts to the ±2..3/255 tolerance.
   Confirm the depth/distance golden is independent of the color golden so the field guard survives the shading move.
2. **Tripwire for the marcher TU**: because `sdf_gbuffer_composite.hlsl` both `#include`s the frozen field AND gains the id-read + stop-shading edits, DXC could in principle re-schedule instructions in the shared field region. The guard is the depth/distance golden staying green after the edit — that is the empirical proof the field SPIR-V behavior is unchanged. (If paranoia is warranted, a SPIR-V diff of the field functions can be added as a CI check; the golden is the authoritative gate.)
3. Only after the split is green do the id-write + oct-normal + stop-shading edits land.

## MVP-vs-deferred + phased roadmap with per-phase test gates

**THIS PLAN (flat deferred PBR):**

- **Phase 0 — golden split (C3).** Split distance/depth golden from shading-color golden; prove the field guard is independent.
  - *Gate*: depth/distance golden byte-exact; color golden isolated and green.
- **MVP-A — flat PBR, deferred split.** `pbr.hlsli` (D/V/F/Lambert + EnvBRDFApprox); `deferred_pbr.comp` consuming the existing G-buffer; material id in `center.w` (read outside the field); `gMaterial`→R16_UINT (with the W1 capability gate); `MaterialGpu` SSBO + upload; host PBR mirror; A1→visibility, A2→ambient; stop the marcher's inline composite. **No textures, no bindless, no DDS.**
  - *Gate*: depth/distance golden byte-exact; shading-color golden within ±2..3/255; 0%-regression marcher bench; resolve ≤ 0.6 ms; validation-layer clean; `MaterialGpu` fingerprint const-asserts + the `MATERIAL_GPU_WORDS==16` shader pin compile.
- **MVP-B — mesh-half parity.** The mesh G-buffer writes the same layout (id + oct-normal); both halves share `deferred_pbr.comp`.
  - *Gate*: a mesh + SDF mixed-scene golden; both halves identical BRDF; no double-shading.
- **MVP-C — oct-normal + unified sRGB/tonemap (Decisions 5 + 9).** Oct-RG16 normal target both halves; one tonemap/OETF site; present samples the lit output. (Folded with MVP-A's normal-format change to avoid a normal-golden re-bake.)
  - *Gate*: normal golden re-baked once at oct-RG16; specular-highlight banding visually gone; linear-space round-trip correct on both halves; (optional) Filament 1-mul multiscatter if measured worthwhile.

**FOLLOW-UP PLAN (`docs/PBR-TEXTURES-PLAN.md`) — textured PBR:**

- **T1 — bindless RHI** (the C2 work, ranked): UAB descriptor pool; `DescriptorBindingFlags` + variable-count alloc; enable the 1.2 descriptor-indexing feature struct; `write_bindless_slot`; BC formats + abi_guard; mips/array/sampler-LOD/aniso on the descs.
  - *Gate*: a 16K-slot UAB set allocates + binds; validation/sync2 clean with descriptor-indexing enabled; `nonuniformEXT` on every divergent index.
- **T2 — in-house DDS reader** (BC7/BC5/BC4): header/DX10/mip-byte math; malformed → Err never UB; fuzz (random bytes → Err-or-valid, never UB/over-alloc); Miri subset.
  - *Gate*: parser unit + property tests; round-trip a texconv-built DDS; bounded-alloc proof.
- **T3 — triplanar + UV sampling + compute LOD** (the W3 open sub-problem): whiteout-blend triplanar for SDF; UV path for mesh; the compute-LOD design (the W3 fallback chosen and justified).
  - *Gate*: triplanar seam quality; no mip aliasing at the chosen LOD; `MaterialGpu` index fields wired; bindless sample vs bound-set baseline bench.
- **T4 — IBL upgrade (deferred):** reflection-probe cube + split-sum, only if analytic sky proves insufficient.
- **T5 — streaming / VT (parked):** only if a texel-≫-VRAM workload appears.

## Metrics and validation

- **0%-gate / golden**: depth/distance golden byte-exact (the field tripwire, C3); shading-color golden within ±2..3/255 (host PBR mirror).
- **Unit tests (MVP)**: `MaterialGpu` fingerprint const-asserts + offset asserts; `MATERIAL_GPU_WORDS==16` shader pin; `with_material` packs/round-trips `center.w` AND the field eval is bit-identical with/without an id set (the determinism proof for Decision 4); R16_UINT/R16G16 capability-gate fail-fast path.
- **Unit tests (FOLLOW-UP)**: DDS parser (header/DX10/mip-byte math, malformed → Err never UB, Miri subset); free-list acquire/release + deferred-fence reuse.
- **Property tests (FOLLOW-UP)**: DDS fuzz (random bytes → Err-or-valid, never UB/over-alloc — bounded-alloc); free-list never double-issues a slot.
- **Benchmarks**: resolve ms @1080p; material-fetch overhead vs flat-Lambert baseline; 0%-regression on the marcher; (FOLLOW-UP) bindless sample vs bound-set.
- **Validation/sync clean**: validation layers + sync2 clean (MVP needs no descriptor-indexing; FOLLOW-UP must be clean with it enabled).
- **debug_assert!**: material id < table_len; (FOLLOW-UP) bindless slot < capacity; slot not already free; mip count ≤ image mips; BC dims multiple-of-4 (or padded-edge accepted).

## Owner VALUE/SCOPE decisions

Engineering recommendations are made; these are the calls that involve values/scope/dependency-philosophy and are escalated rather than decided unilaterally:

1. **Scope split of THIS plan (C2/OQ-3)**: deliverable = MVP-A+B+C (flat deferred PBR, no bindless/DDS); Decisions 7/8 spun into `docs/PBR-TEXTURES-PLAN.md`. **Recommend yes** — ships the high-confidence core without blocking on the unscoped bindless RHI. *Confirm.*
2. **Atlas vs bindless (OQ-1)**: engineering recommendation is **bindless** (Decision 7); atlas only for a future UI/glyph subsystem. *Confirm the recommendation overrides the atlas hypothesis.*
3. **Texture-loading dependency (OQ-2)**: in-house DDS reader + **offline texconv build-step** (Decision 8). This accepts a *build-time* tool, not a runtime dep. DDS-vs-KTX2 is a minor sub-call (recommend DDS). *Confirm accepting a build-time asset-build step.*
4. **IBL scope v1 (OQ-4)**: analytic EnvBRDFApprox + analytic sky; no env asset, no prefilter pass. Reflection-probe split-sum deferred to T4. *Confirm.*
5. **smin material blending (OQ-5)**: v1 picks **nearest-surface material** (no smooth lerp at seams; keeps the frozen field). Smooth blend needs associative exp-smin → **changes the frozen field + a new golden** — a real owner fork. *Recommend defer.*
6. **Multiscatter (OQ-6)**: ship single-scatter v1; add the 1-mul Filament term later. *Recommend.*

## Changelog (r1 → r2, post-critique)

- **C1 resolved**: added Decision 1a with explicit resolve-set binding tables for MVP (6 bindings) and the Phase-4 textured case (restructured into two sets — set 0 = 6 MVP bindings, set 1 = the bindless UAB table). Committed `MAX_BIND_GROUP_BINDINGS` stays 8; the cap invariant is preserved.
- **C2 resolved**: split the deliverable. This plan = MVP-A+B+C (flat, zero bindless RHI). Decisions 7/8 explicitly deferred to `docs/PBR-TEXTURES-PLAN.md`; the bindless seam is honestly scoped (UAB pool, binding flags, variable-count alloc, feature enable — all absent today, the bulk of the follow-up).
- **C3 resolved**: added the "Validation sequence — C3 golden split" section. Phase 0 splits the distance/depth golden (byte-exact field tripwire) from the shading-color golden (moves to the resolve, ±2..3/255) before any marcher edit; the depth golden is the empirical proof the field SPIR-V is unchanged.
- **W1 resolved**: Decision 3 now mandates an R16_UINT `STORAGE_IMAGE` capability check mirroring `gbuffer_storage_format_ok` (fail-fast), and **commits** to explicit `[[vk::image_format("r16ui")]]` so `shaderStorageImageWriteWithoutFormat` is NOT enabled (`p_enabled_features` stays null). The "drop format / enable feature" bullet is removed.
- **W2 resolved**: `MaterialGpu` repacked into 4 clean 16-B vec4/uvec4 lanes (no mixed-scalar greedy packing); added offset_of const-asserts AND a shader-side `MATERIAL_GPU_WORDS==16` pin + documented std430 offsets, mirroring the `SdfEdit`/`SDF_EDIT_WORDS` discipline.
- **W3 resolved**: compute-pass texture LOD is explicitly flagged as an UNSOLVED follow-up sub-problem (T3), with fallback options noted; `SampleLevel` no longer implied trivial.
- **W4 resolved**: the SSBO-fetch cost justification changed from "tile-coherent" to **L2-residency** (256 KB table in 3 MB L2), honestly acknowledging divergent per-lane fetches at smin seams (Decision 2a).
- **O1 addressed**: the four `TextureHandle` fields on `Material` are documented as FOLLOW-UP-reserved/ignored-by-MVP.
- **O2 addressed**: the DDS-reader "~150 LOC" estimate is acknowledged as parse-only; realistically more with fuzz-hardening + mip-walk + barriers.
- **MVP scoping**: oct-normal (Decision 5) and unified sRGB/tonemap (Decision 9) pulled into MVP-C (folded with MVP-A's format change) so the normal/color goldens bake once.

Key files: `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` (FROZEN), `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` (`gMaterial` rgba8 float4 @ line 102, `load_edit` skips word 3 @ line 75), `crates/boyko_sdf_math/src/lib.rs:118-209` (`SdfEdit` fingerprint + free `center.w`), `crates/boyko_rhi_vulkan/src/compute.rs:442-499` (`host_ao`/`host_shade` mirror), `crates/boyko_rhi_vulkan/src/swapchain.rs:3200-3438` (`GBufferTargets`, `GBUFFER_FORMAT`, `vocab_set` 7 entries), `crates/boyko_rhi/src/device.rs:22` (`MAX_BIND_GROUP_BINDINGS=8`), `crates/boyko_rhi_vulkan/src/device.rs:1797-1842` (`query_device_caps`, `gbuffer_storage_format_ok` pattern, `p_enabled_features` null), `crates/boyko_rhi_vulkan/src/rhi_impl.rs:824,917-943` (`descriptor_count: e.count`, `max_sets:1` histogram pool), `crates/boyko_rhi_vulkan/src/abi_guard.rs:1-415` (`Format`==`VkFormat` static asserts), `crates/boyko_rhi/src/enums.rs:227-552` (`Format`/`TextureDesc`/`SamplerDesc`), `crates/boyko_render/src/lib.rs` (orphan-rule home).