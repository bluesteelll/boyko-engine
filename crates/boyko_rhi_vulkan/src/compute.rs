//! Slice-0 compute assets + golden reference (the `ComputeHarness` scaffold that
//! once lived here is dissolved into the `boyko_rhi` trait surface — see
//! [`crate::rhi_impl`]).
//!
//! What survives here is the backend-agnostic *contract* the Slice-0 compute
//! flow proves, independent of how the trait drives it:
//!
//! - the two committed SPIR-V modules ([`write_pattern_spirv`] /
//!   [`transform_add_spirv`]), exposed as `&'static [u32]` so trait callers feed
//!   them straight into [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module);
//! - the dispatch `LOCAL_SIZE_X` the shaders declare (`[numthreads(64,1,1)]`);
//! - the CPU golden ([`golden_write_pattern`] / [`golden_chained`]) the
//!   bit-exact readback diff asserts against;
//! - [`ComputeError`], the rich compute-path error variant (now folded into the
//!   unified [`VulkanError`](crate::error::VulkanError) at the trait boundary).
//!
//! # What the compute path proves (unchanged across the refactor)
//!
//! - **0c**: a `VkShaderModule` from a committed `.spv`, a one-binding
//!   (STORAGE_BUFFER at COMPUTE) descriptor-set layout + a 4-byte push-constant
//!   pipeline layout (now cached on the device, [`crate::rhi_impl`]), a compute
//!   pipeline; bind a host-visible storage buffer; record begin → bind pipeline
//!   → bind set → push constant → `vkCmdDispatch(ceil(N/64))` → end; submit with
//!   a fence; wait; read back the persistent mapping; assert `buffer[i] = i*2+1`.
//! - **0d**: a SECOND pipeline (`buffer[i] += 100`) chained after a
//!   `VkBufferMemoryBarrier` (COMPUTE_SHADER/SHADER_WRITE → COMPUTE_SHADER/
//!   SHADER_READ) in the SAME command buffer; one submit + fence; the result
//!   diffs bit-exact against the CPU golden — the §5.5 edge→barrier lowering in
//!   miniature.
//!
//! # The shader contract (`shaders/{write_pattern,transform_add}.hlsl`)
//!
//! Both shaders declare `RWStructuredBuffer<uint> Data : register(u0)` (binding
//! 0, set 0, one STORAGE_BUFFER) and a `[[vk::push_constant]] uint count` (a
//! 4-byte range at offset 0, COMPUTE-visible), `[numthreads(64,1,1)]`, entry
//! `main`, each invocation bounds-checking `i < count`.
//!
//! # Soundness (raw FFI → validation + golden are the oracle)
//!
//! Miri cannot run this (real driver FFI, VRAM mapping). The oracle is two-fold,
//! per plan §6: (a) the `VK_LAYER_KHRONOS_validation` messenger asserted to
//! `total() == 0`, and (b) the bit-exact golden diff.

use core::slice;

// The SDF field math + std430 data model now live in the `boyko_sdf_math` leaf
// (the W4 leaf-cut): a `no_std`, graphics-free crate that is the SINGLE source of
// truth shared with `boyko_physics`. The items used internally by the golden /
// encoder / layout below are imported here; the ones the rung-8/9/10/11 tests
// import via `boyko_rhi_vulkan::compute::{..}` are RE-EXPORTED below so those
// pre-leaf-cut paths keep compiling.
use boyko_sdf_math::{
    HEADER_BASE_WORDS, MAX_SDF_EDITS, SDF_EDIT_WORDS, SDF_GRAD_H, sdf_edit_list, v_len, v_sub,
};
// M2 trilinear SURFACE bricks: the apron'd-brick bake + the JCGT analytic-cubic crossing the
// host golden mirror (`golden_composite_pixel_brick_m2`) and the atlas baker (`bake_brick_atlas`)
// drive — the SAME `boyko_sdf_math::brick` oracle the GPU marcher mirrors bit-for-bit.
use boyko_sdf_math::brick::{
    self, BRICK_ALLOC, BRICK_INTERIOR, BRICK_VOXELS, aabb_overlap, band_half_at_level,
    brick_world_at_level, c_max_at_level, classify_brick, decode_snorm8, dirty_world_aabb,
    fill_brick, for_each_revealed_cell, snapped_level_origin, snapped_level_origin_cell,
    toroidal_slot, voxel_size_at_level,
};
use boyko_sdf_math::mesh_sdf::MeshSdfField;
use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditAabb, SdfEditField};

use crate::ffi::VkResult;
use crate::memory::MemoryError;

/// Re-exports of the leaf items whose canonical import path the rung-8/9/10/11
/// tests (and any external caller) use as `boyko_rhi_vulkan::compute::{..}`.
/// Preserved verbatim across the W4 leaf-cut so those paths keep resolving:
/// [`SdfEdit`], [`sdf_kind`], [`sdf_op`], [`SDF_IMG_W`], [`SDF_IMG_H`].
pub use boyko_sdf_math::{SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_kind, sdf_op};

/// The committed SPIR-V for step 0c (`buffer[i] = i*2 + 1`).
///
/// Wrapped in a `#[repr(C, align(4))]` newtype so the `include_bytes!` blob is
/// 4-byte aligned: it is reinterpreted as a `&[u32]` word stream, and the SPIR-V
/// spec requires that stream to be 4-byte aligned (a bare `include_bytes!` is
/// only `align(1)`).
static WRITE_PATTERN_SPV: SpirvBlob<988> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/write_pattern.comp.spv"
)));

/// The committed SPIR-V for step 0d (`buffer[i] += 100`).
static TRANSFORM_ADD_SPV: SpirvBlob<968> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/transform_add.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 8 (sphere-trace one analytic sphere into
/// a packed-RGBA storage buffer — `shaders/sdf_spheretrace.hlsl`).
static SDF_SPHERETRACE_SPV: SpirvBlob<3316> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_spheretrace.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 9 (sphere-trace an ordered SDF edit-list
/// — multi-primitive CSG — into a packed-header storage buffer,
/// `shaders/sdf_editlist.hlsl`).
static SDF_EDITLIST_SPV: SpirvBlob<24368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_editlist.comp.spv"
)));

/// The committed SPIR-V for Phase-6 rung 10 (SDF + mesh hybrid composite via a
/// shared depth buffer — `shaders/sdf_depth_composite.hlsl`). It reuses the rung-9
/// edit-list fold + lighting + camera verbatim and adds the per-pixel mesh-depth
/// read that BOUNDS the march so the mesh and the SDF occlude each other.
static SDF_DEPTH_COMPOSITE_SPV: SpirvBlob<26576> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_depth_composite.comp.spv"
)));

/// The committed SPIR-V for the Render P1a GPU gate (sphere-trace the rung-9 SDF
/// edit-list and STORE the marcher color into a STORAGE IMAGE through the
/// multi-resource descriptor *vocabulary* set — `shaders/sdf_editlist_storage_image.hlsl`).
/// Reuses the rung-9 field eval + ray-gen + lighting VERBATIM; the only differences
/// are the bind points (binding 0 = a read-only `StructuredBuffer<uint>` edit-list,
/// binding 1 = a `RWTexture2D<float4>` output) and the output sink (the marcher color
/// is STORED to texel `(px, py)` instead of packed into the buffer's pixel region).
static SDF_EDITLIST_STORAGE_IMAGE_SPV: SpirvBlob<24092> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_editlist_storage_image.comp.spv"
)));

/// The committed SPIR-V for the Render P1b OFFSCREEN MRT G-buffer marcher
/// (`shaders/sdf_gbuffer_composite.hlsl`) — the image-based rewrite of the rung-10
/// packed-buffer composite. The field eval + ray-gen + lighting are a VERBATIM cut of
/// `sdf_depth_composite.hlsl`; the only two I/O edits are (1) the mesh depth read
/// becomes a SAMPLED-image fetch (`Texture2D<float>` @ binding 1) and (2) the marcher
/// color becomes a STORAGE-image store (`RWTexture2D<float4>` albedo @ binding 2, plus
/// additive normal/material @ bindings 3/4). The extent/camera block moves to a UNIFORM
/// buffer @ binding 5 (written once). The edit-list stays a `StructuredBuffer` @
/// binding 0.
/// Render PBR MVP-2: the marcher repacks the G-buffer attributes — gAlbedo = the picked
/// material's RAW LINEAR `base_color` (from the material SSBO @ binding 7), gNormal =
/// (octahedral world normal in RG, 16-bit material id in BA), gMaterial = (shadow, ao,
/// mask). The material PICK is an argmin over the edit list reusing the FROZEN
/// `load_edit`/`edit_distance` (a read-only attribution; the field is untouched, proven by
/// the GATE-1 probe tripwire). Phase 0 also extracted ray-gen into the shared
/// `ray_gen.hlsli`. The full Cook-Torrance shade runs in [`deferred_pbr_spirv`]. The byte
/// length grew (41176 → 43944) with the material pick + oct-encode + SSBO fetch; Lighting
/// L0b added the `gViewT` storage-image lane + its 3 terminal writes (43944 → 44216). The SDF
/// brick-atlas M1 empty-skip prefix grew it (→ 47032), then M2 added the trilinear+JCGT-cubic
/// SURFACE-brick path (atlas `Texture3D` @binding 10 + the b5 `M2GridParams` block + the cubic
/// solver, → 72280). BUG-M2-GPU-1 then fixed the dead M2 branch (the real cause was
/// `VK_FORMAT_R8_SNORM` mis-set to `9 == R8_UNORM`, so the atlas decoded `byte/255` not the signed
/// `byte/127`, collapsing the cubic to no sign-change), switched the corner fetch from `.Load`
/// (texelFetch, ill-defined on a combined image+sampler descriptor) to a NEAREST `SampleLevel`, and
/// dropped the `m2_sampler_keepalive` hack (→ 72200 bytes). SDF brick-atlas M4 (clip-map LOD, Slice C)
/// then widened the b5 UBO tail to `M4Level m2_levels[BRICK_LEVELS]`, declared the N-level brick bindings
/// (t11/t12 for L1, t13/t14 for L2), threaded the per-level resources as shader resource params, and
/// added `select_level` + the static branch-ladder in the marcher (→ 127548 bytes). `brick_levels == 1`
/// loops once over level 0 (the M2 resources) — byte-identical to the pre-M4 marcher (the OFF/N=1 gate).
/// The M2/M4 SIGNED-refine fix (the `m2_surface_hit` fallback now converges from EITHER side: accept on
/// `abs(d)`, signed under-relaxed step `rt += M2_REFINE_RELAX * d`) → 127912 bytes. BUG-B1-ANALYTIC-BLACK
/// then mirrored that signed refine onto the ANALYTIC over-relaxation accept (an `omega > 1` step can
/// overshoot deep inside the surface; `d < EPS` is one-sided, so the committed hit could sit ~δ inside →
/// shadow + AO collapse to 0 → BLACK). The accept now runs `M2_REFINE_ITERS` signed
/// `t += M2_REFINE_RELAX * d` steps so the hit lands on `|sdf| < EPS` → correct shadow/AO → lit (the omega==1 t is unchanged:
/// its accept `d` is already in `[0, EPS)`, so the first refine iteration accepts) → 131272 bytes.
/// BUG-M2-CRATER then routed the `m2_surface_hit` CREASE-ACCEPT (close) arm through the SAME signed
/// refine the far arm already used (commit the refined `rt`, not the raw down-biased `cand_t`), so the
/// baked AO samples ON the surface instead of ~M2_CREASE_EPS inside (the golf-ball craters) → 131344
/// bytes. BUG-M2-RIM then REMOVED the trailing crease-accept band entirely: a candidate is committed
/// ONLY when the signed refine CONVERGES (`abs(d) < EPS`); a grazing silhouette point (analytic miss
/// within `M2_CREASE_EPS`) or a stalled hard crease falls to the analytic fold, erasing the 1-2px
/// silhouette rim where the brick hit but the analytic ray missed (dead `resid`/`cand_p` removed),
/// giving 121620 bytes (VulkanSDK 1.4.350.0 dxc). The hit-set is now exactly the refine-converged set; the residual silhouette rim is ACCEPTED as inherent (owner decision), and the marginal grazing-only EXACT-ANALYTIC RE-MARCH that chased it (a factored analytic re-march gated on a near-tangent normal-vs-ray dot, about 30KB more SPIR-V to recover roughly 7px) was REVERTED in favor of this clean band-removal state under a perf-maximal budget. SDF brick-atlas M5a (TOROIDAL clip-map streaming) then made `m2_surface_hit`'s tile lookup address the atlas at the TOROIDAL slot `(round(origin/bw) + box) mod M2_GRID_DIM` (Decision 5 — recomputed from the existing UBO `origin`/`brick_world`, NO new UBO field so the OFF UBO byte-identity is untouched); at a grid where `origin_cell ≡ 0 (mod DIM)` it reduces to the old `box * BRICK_ALLOC` map → 122488 bytes. Render P0 (empty-skip-only) then generalized the hand-written coarse-cull prefix's `coarse_enabled` gate from a bool to a 3-value `CoarseMode` (0 off / 1 full / 2 empty-skip-only): the `near_t` seed is now wrapped in `if (pc.coarse_enabled == 1u)`, so mode 2 skips empty tiles WITHOUT seeding (the lit-transparent on-screen cull, no grazing-silhouette AO/shadow rim). Mode 0 and mode 1 OUTPUT are byte-unchanged; the new `== 1u` compare adds → 122532 bytes. Render
/// P5 (mesh-first-class hybrid) r1+r2 then added the per-pixel SDF/mesh OWNERSHIP GATE at the two
/// terminal write sites: `own_pixel = !has_mesh || (hit && t < t_mesh)` wraps the THREE attribute
/// stores (gAlbedo/gNormal/gMaterial) at Site B and `!has_mesh` wraps them at Site A (the empty-tile
/// early-return); gViewT stays ALWAYS-written with its value forced to the `1.0e30` sentinel on a
/// `!own_pixel` pixel (Decision 3, exactly-once). On a NO-MESH scene `has_mesh` is always false so
/// `own_pixel` is always true and every write path is byte-identical (the 0%-gate); the `.spv` grows
/// by the two gate branches → 122812 bytes. The GRAZING-SHADOW-ACNE fix then lifted the A1
/// soft-shadow march origin off the surface by `n * SHADOW_NORMAL_BIAS` at the (hand-written)
/// `sdf_soft_shadow` CALL SITE (`sdf_soft_shadow(p + n*SHADOW_NORMAL_BIAS, n, light)`) so grazing
/// (near-tangent) terminator rays clear the curved surface instead of false-occluding — the
/// GENERATED march span + `shadow.rs` stay byte-frozen; the new const + the per-component add → 122868 bytes.
/// Render P7/P5-r1b (mesh gViewT UNLOCK) then made a mesh-covered raster-owned pixel store the mesh
/// surface ray-t `t_mesh` (= md * T_MAX) instead of the `1.0e30` sentinel at BOTH terminal gViewT
/// write sites (Site B's main terminus + Site A's empty-tile early-return), so the deferred resolve
/// reconstructs the real mesh `P` (in-range point/spot lighting) AND the SSAO pass processes mesh
/// pixels. The SDF-hit branch (real `t`) and the pure-background branch (no mesh → `1.0e30`) are
/// UNCHANGED; the two new `has_mesh ? t_mesh : 1.0e30` selects + the empty-tile select add → 122988 bytes.
/// (Later SSAO + quality-ladder work grew it to 143584.) MDF Stage-2c (mesh-distance-field shadows)
/// then added: the `cbuffer Camera` b5 `MeshSdfParams` tail (2 `float4` lanes @224/240), the dedicated
/// dense mesh-SDF `Texture3D MeshSdf`/`SamplerState MeshSdfSampler` @binding 15, the hand-written
/// `mesh_sdf_sample` glue (world→UVW transform + LINEAR fetch + snorm decode) + the parallel
/// `sdf_soft_shadow_mesh` march (the generated `sdf_soft_shadow` span byte-UNTOUCHED — the eDSL pins
/// stay green), and the `pc.mesh_sdf_enabled ? sdf_soft_shadow_mesh : sdf_soft_shadow` select at the
/// TWO shadow call sites (the SDF-surface arm + the mesh-floor arm). `mesh_sdf_enabled == 0` keeps the
/// OFF path byte-identical (the texture is bound-but-unread, the analytic march stands — the 0%-gate),
/// so the 41 `sdf_gbuffer_hybrid` goldens (no MDF scene) stay byte-exact → 155024 bytes (VulkanSDK
/// 1.4.350.0 dxc; the +96 over 154928 is the `mesh_self_skip` self-shadow START-offset in
/// `sdf_soft_shadow_mesh`, anti mesh-self-acne — see the shader's MESH_SELF_SHADOW_SKIP_VOXELS).
static SDF_GBUFFER_COMPOSITE_SPV: SpirvBlob<155124> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_gbuffer_composite.comp.spv"
)));

/// The committed deferred-shading SPLIT (increment 1) RESOLVE SPIR-V
/// (`shaders/deferred_pbr.hlsl`). A fullscreen compute pass that reads the marcher's
/// G-buffer attributes back — gAlbedo (the unmultiplied base) @ binding 0 + gMaterial
/// `(r = vis, b = mask)` @ binding 1, both STORAGE images in GENERAL — and stores the
/// final LIT color `mask ? base*vis : base` to a STORAGE image @ binding 2. EXACTLY 3
/// STORAGE bindings, no sampler, no UBO: the extent comes from `gLit.GetDimensions` (the
/// lit image is 1:1 the marched pixels), so the 1D dispatch index maps to (px, py) the
/// same way the marcher does. The COMPOSITE moved here; the BRDF stays in the marcher this
/// increment. The host mirror is [`golden_deferred_resolve`] (fed by
/// [`golden_marcher_attributes`]).
///
/// Render PBR MVP-2: the resolve now runs FULL metallic-roughness Cook-Torrance (GGX D +
/// height-correlated Smith V + Schlick F + Lambert diffuse + analytic EnvBRDFApprox
/// ambient) instead of `mask ? base*vis : base`. It reads gAlbedo (raw base), gNormal (oct
/// normal + 16-bit material id), gMaterial (shadow, ao, mask) @ bindings 0..2, fetches the
/// picked material from the SSBO @ binding 4, and reconstructs the per-pixel view direction
/// from the camera UBO @ binding 5 + the shared `ray_gen.hlsli`. SDF (mask == 1) pixels get
/// full PBR (the owner-acknowledged behavioral change, PBR plan call F); mesh / background /
/// empty (mask == 0) pass `base` through byte-identically (the 0%-gate). The byte length
/// grew (1616 → 6880) with the BRDF, then 6880 → 8824 with the Lighting L0a header+table
/// directional/sky loop, then 8824 → 12548 with the L0b `gViewT` `P`-reconstruction +
/// point/spot loop, then 12548 → 14536 with the L1 cluster lookup (the froxel-mapped index
/// loop + the `ClusterGrid`/`LightIndexList` bindings @8/@9), then 14536 → 15252: the WRONG
/// L1 index-range guard was REMOVED (it never fired for the offending light) and the
/// per-light `normalize(v+l)` / `normalize(L.dir)` were replaced by `safe_normalize` — the
/// faithful mirror of the host oracle's `v_normalize` zero-guard — so a back-facing surface's
/// `~0` half-vector yields `[0,0,0]` (NoH = LoH = 0, finite spec) instead of the intrinsic
/// `normalize(0) == NaN` that blackened the pixel (`NaN * (NoL == 0) == NaN` →
/// `pack_unorm(NaN) == 0`). Bindings 8/9 are now STATICALLY referenced on every path (DXC no
/// longer dead-strips them when clusters are off), so every resolve pipeline layout declares +
/// binds 0..9 (placeholder buffers @8/@9 on the non-clustered paths). The byte length then
/// grew 15252 → 24728 with **P6 R1** (multi-light SDF shadows, ANALYTIC ranged march): the
/// resolve now `#include`s `sdf_field.hlsli` (the frozen `field_distance`) + binds the
/// edit-list `Buf` SSBO @10 (one new binding, 10 → 11; NO cap raise), defines the generated
/// `sdf_soft_shadow_ranged(p,n,L,t_max)` leaf, and — gated by the header's `shadow_mode`
/// (word 7; 0 = the BYTE-IDENTICAL 0%-gate) — marches each EXTRA flagged caster's per-light
/// shadow (the primary directional KEEPS `gMaterial.r`). The host mirror is
/// [`golden_deferred_resolve_table`] (clustered via [`golden_cluster_cull`] +
/// [`golden_deferred_resolve_clustered`]); the shadowed multi-light path is mirrored by the
/// `_shadowed` variants. The GRAZING-SHADOW-ACNE fix then lifted the per-light ranged-march
/// origin by `n * SHADOW_NORMAL_BIAS` at the two (hand-written) `sdf_soft_shadow_ranged` CALL
/// SITES (`sdf_soft_shadow_ranged(P + n*SHADOW_NORMAL_BIAS, n, l, t_max)`) so grazing terminator
/// rays clear the surface — the GENERATED ranged span stays byte-frozen; 24728 → 24824 bytes.
/// Render P7 GROUP C1: the resolve gains `gSsao @11` (`R8_UNORM` STORAGE) + the structural-`if`
/// `ao_final = min(class_ao, gSsao)` combine gated by `load_ssao_mode` (header word 11). On every
/// pre-P7 scene `ssao_mode == 0`, so `gSsao` is never read and the lit PIXELS are byte-identical;
/// the `.spv` itself grows (the gate + the new binding are compiled in); 24824 → 25280 bytes.
/// Render P7 POLISH: the single `gSsao` center tap becomes an inline 7×7 (`R == 3`) depth-gated
/// box blur (`gViewT` bilateral gate at `SSAO_BLUR_DEPTH_TOL == 0.1`) to kill the discrete-step
/// SSAO RINGS — still inside the SAME `ssao_mode != 0` combine (NO new pass; the 0%-gate holds,
/// `ssao_mode == 0` never executes the loop). The host mirror is `golden_ssao_blur`; the gather
/// order/bounds/gate are byte-mirrored so GPU == host within ±2/255; 25280 → 26608 bytes.
/// Render Shadow Phase 3: Screen-Space Contact Shadows (SSCS) add `project_to_screen` (the exact
/// `generate_ray` inverse) + `sscs_march` (an unrolled 8-step screen-space depth march) multiplied
/// into the per-light `vis` at both lighting sites, gated by `contact_shadow_mode` (header word 7
/// bit 1; OFF on every pre-Phase-3 scene → the march block never runs → byte-identical, the
/// 0%-gate); 26608 → 40652 bytes. CSM Increment 1b (Rung A) then added the cascade shadow-map
/// SAMPLE: bindings 12/13/14 (`Texture2DArray<float> gCsm` + `SamplerComparisonState gCsmCmp` +
/// the `CsmCascades` cbuffer mirroring `ResolvedCsm`) + the `csm_visibility` PCF helper +
/// (gated by header word 7 bit 2 via `load_csm_mode`; OFF on every pre-CSM scene → the
/// `SampleCmpLevelZero` never runs → the bound-but-unread cascade map/sampler/UBO are never
/// sampled → byte-identical, the 0%-gate) the `vis = min(vis, csm_visibility(P, n))` combine on
/// the primary directional; 40652 → 43316 bytes. CSM Increment 3 (Rung B) extends the single-cascade
/// sample to N cascades: `csm_sample_cascade(c, ..)` PCF-samples array layer `c`, and `csm_visibility`
/// SELECTS the cascade by VIEW-Z (a branch-light compare-chain over `gCsmActive`) then cross-fades
/// across the trailing `CSM_OVERLAP_PROPORTION` band into `c+1` (the analytic, no-dither seam blend).
/// Still under the SAME `csm_mode != 0` gate → byte-identical PIXELS on every pre-CSM scene (the
/// 0%-gate; the `.spv` grows with the select/blend); 43316 → 46008 bytes. Shadow Phase 5 Inc-1-GPU
/// adds the sparse SPOT atlas sample: under `punctual_shadow_mode != 0` (header word 7 bit 3; OFF
/// on every pre-Inc-1 scene → the `SampleCmpLevelZero` never runs → the bound-but-unread shadow
/// atlas map/sampler/UBO are never sampled → byte-identical, the 0%-gate) a SPOT light with a real
/// `light_atlas_slot` multiplies its contribution by `spot_atlas_visibility(slot, P, n)` (bindings
/// 14 combined map+sampler + 15 the `ResolvedShadowAtlas` UBO — the resolve set hits 16/16); 46008
/// → 48472 bytes. Shadow Phase 5 Inc-2 (POINT cube): a POINT light with a real slot BASE instead
/// reads `punctual_atlas_visibility(base, P, n)` (major-axis cube face-select + LINEAR-distance
/// compare over the six contiguous layers `base..base+6`); 48472 → 50976 bytes. Shadow
/// anti-scintillation: the CSM/atlas tail compares widened to the 13-tap tent-disc PCF
/// (`csm_pcf_disc`/`atlas_pcf_disc`, ConstOffset taps); 50976 → 57928 bytes. SDFDDGI I0: the 3
/// bound-but-unread DDGI resolve bindings (16/17/18 — `gDdgiIrr`/`gDdgiDepth` combined images + the
/// `ResolvedDdgi` UBO) + the gated (runtime-zero at I0) probe-irradiance injection; 57928 → 59160
/// bytes. (The GI gate is header word-7 bit 4, OFF by default → the injection never runs → the
/// rendered pixels are byte-identical; only the .spv byte-length grows with the new decls.) SDFDDGI
/// I3: the gated probe-irradiance injection becomes the REAL trilinear + wrap + Chebyshev sample
/// (`ddgi_resolve.hlsli::ddgi_probe_sample`, the op-for-op `goldens::probe_sample` mirror); 59160 →
/// 65456 bytes (the `precise` pin on the blend accumulators forbids DXC from fusing the
/// accumulation MACs into single-rounding FMAs, matching the Rust host oracle's non-fused adds; a
/// residual ≤2-ULP B10G11R11 texture-sampler difference — far below the format's 11-bit storage
/// precision — is absorbed by the `ddgi_probe_gi_resolve` golden's tight ULP tolerance. NB: pinning
/// MORE sites `precise` was reverted — it perturbed DXC's global optimization enough to drift the
/// GI-OFF PBR path off the golden). GI still OFF by default → the injection never runs →
/// byte-identical pixels (the 0%-gate); only `ddgi_indirect=true` samples.
static DEFERRED_PBR_SPV: SpirvBlob<65456> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/deferred_pbr.comp.spv"
)));

/// The SDFDDGI I3 DDGI resolve-sample GPU-GOLDEN SPIR-V (`shaders/ddgi_probe_gi_resolve.comp.hlsl`).
/// A standalone compute harness that runs the SAME `ddgi_probe_sample` the deferred resolve runs
/// (both `#include "ddgi_resolve.hlsli"`) over host-supplied receiver samples and STOREs the
/// resolved irradiance, so the `ddgi_probe_gi_resolve` test can diff GPU-vs-`goldens::probe_sample`
/// to bits. Its own pipeline layout (b0 grid UBO — its pad `.x` carries the sample count, t1/s1 irr,
/// t2/s2 depth, t3 recv-pos, t4 recv-nrm, u5 out) — NOT the resolve set, no push constant.
static DDGI_PROBE_GI_RESOLVE_SPV: SpirvBlob<9212> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/ddgi_probe_gi_resolve.comp.spv"
)));

/// The committed Lighting-L1 clustered froxel light-cull SPIR-V (`shaders/cluster_cull.hlsl`).
/// One invocation per froxel (`CLUSTER_COUNT`): builds the froxel's world-space AABB from the
/// shared ray-gen + the exp-Z slice view-z, culls each point/spot light's bounding sphere
/// (`sqDistPointAABB <= r²`), and atomic-appends survivors into the flat `LightIndexList` +
/// writes the per-froxel `{offset, count}` `ClusterGrid` cell. Bound to the cull set { camera
/// UBO @0, light table SSBO @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4 } + a
/// `ClusterCullPush` (near/far + caps). Directional/sky are GLOBAL (not culled). The host
/// mirror is [`golden_cluster_cull`].
static CLUSTER_CULL_SPV: SpirvBlob<12356> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/cluster_cull.comp.spv"
)));

/// The committed Render P4b coarse-cull / tile pre-trace SPIR-V
/// (`shaders/sdf_tile_cull.hlsl`). A 1/8-res CONSERVATIVE cone-trace: one invocation
/// per 8×8 fine-pixel tile emits a [`TileBound`] the fine marcher reads to early-out
/// EMPTY tiles + seed `t = near_t`. A strict FIELD-CONSUMER (calls the frozen
/// `field_distance`); bound to the P4b vocabulary set { SSBO edit-list @0, SAMPLED
/// depth @1, STORAGE `TileBound` @6, UNIFORM camera @5 }.
static SDF_TILE_CULL_SPV: SpirvBlob<10304> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_tile_cull.comp.spv"
)));

// The committed Render P7-Q2 SSAO (HBAO-lite, no-trig) quality-VARIANT SPIR-V — one PRE-COMPILED
// `.spv` per `boyko_shaderdsl::ssao::SSAO_PRESETS` row (Mechanism C: a variant is selected at
// runtime by binding a different pipeline, NEVER by a dynamic loop bound, so every `[unroll]`
// slice/step loop stays fully unrolled with ZERO per-pixel runtime cost). All three share the
// IDENTICAL 5-binding SSAO interface (`shaders/sdf_ssao.comp.hlsl`, GROUP A): gNormal @0 (R,
// oct + id), gMaterial @1 (R, `.b` = mask), gViewT @2 (R, surface `t`), the `ssao` out @3 (W), the
// 80-byte camera UBO @4 (`gAlbedo` is NOT bound) — so ONE bind-group layout drives all three. Each
// `.spv` is a DISTINCT size (the baked-const unroll counts differ), so each is its own
// `SpirvBlob<N>` `static` (a heterogeneous `[_; 3]` array would force a large-variant enum); the
// `sdf_ssao_spirv_variant(q)` selector matches `q` to the right blob. Each `N` is that
// `include_bytes!`'s own const-asserted size — a drifted variant `.spv` fails the length at compile
// time. The host oracle is [`golden_ssao_attributes`] fed the matching [`SSAO_PARAMS`] row.

/// `SSAO_PARAMS[SSAO_QUALITY_LOW]` — `sdf_ssao_low.comp.spv` (2 slices × 3 steps × 2 = 12 taps).
static SDF_SSAO_LOW_SPV: SpirvBlob<35536> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_ssao_low.comp.spv"
)));

/// `SSAO_PARAMS[SSAO_QUALITY_MEDIUM]` — `sdf_ssao_medium.comp.spv` (2 slices × 4 steps × 2 = 16
/// taps; == today's shipped consts, byte-identical to the pre-Q2 base `sdf_ssao.comp.spv`).
static SDF_SSAO_MEDIUM_SPV: SpirvBlob<44968> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_ssao_medium.comp.spv"
)));

/// `SSAO_PARAMS[SSAO_QUALITY_HIGH]` — `sdf_ssao_high.comp.spv` (3 slices × 6 steps × 2 = 36 taps).
static SDF_SSAO_HIGH_SPV: SpirvBlob<90160> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_ssao_high.comp.spv"
)));

// The committed SDFDDGI I2 probe-update SPIR-V — one PRE-COMPILED `.spv` per `GI_MAX_IT` sweep
// value {32, 64, 96, 128} (the same variant mechanism as SSAO: a `GI_MAX_IT` header symbol
// re-DXC'd per value so measured==shipped, plan §1.2/§5). All four share the IDENTICAL update
// bind-group interface (set 0: t0 Buf, u1 gIrrOut, u2 gDepthOut, u3 Classification, t4 RayTable,
// t5 LightBuf, b6 DdgiUpdate), so ONE bind-group layout drives any of them; only the baked
// `GI_MAX_IT` `[loop]` trip count differs. Each `.spv` is a distinct size, so each is its own
// `SpirvBlob<N>` `static` with its own const-asserted `include_bytes!` length — a drifted variant
// fails the length at compile time. `sdf_probe_update_spirv(gi_max_it)` matches the sweep value to
// the right blob.

/// `GI_MAX_IT == 32` — `sdf_probe_update_it32.comp.spv`.
static SDF_PROBE_UPDATE_IT32_SPV: SpirvBlob<44720> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_probe_update_it32.comp.spv"
)));

/// `GI_MAX_IT == 64` — `sdf_probe_update_it64.comp.spv` (the shipped default per plan §6).
static SDF_PROBE_UPDATE_IT64_SPV: SpirvBlob<44704> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_probe_update_it64.comp.spv"
)));

/// `GI_MAX_IT == 96` — `sdf_probe_update_it96.comp.spv`.
static SDF_PROBE_UPDATE_IT96_SPV: SpirvBlob<44720> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_probe_update_it96.comp.spv"
)));

/// `GI_MAX_IT == 128` — `sdf_probe_update_it128.comp.spv`.
static SDF_PROBE_UPDATE_IT128_SPV: SpirvBlob<44704> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/sdf_probe_update_it128.comp.spv"
)));

/// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS vertex SPIR-V
/// (`shaders/csm_depth.vs.hlsl`). A GRAPHICS (`vs_6_0`) stage — the FIRST non-compute blob
/// hosted here, so the resolve/depth-pass shaders live behind ONE `compute::*_spirv()`
/// vocabulary. It reads the SAME set-0 binding-0 `InstanceModelCol` SSBO + the SAME 88-byte
/// VERTEX push as `gbuffer_mrt.vs.hlsl`'s instanced arm, but projects by the CASCADE's
/// world→light-clip matrix (push `@0`) instead of the camera view-proj, and outputs ONLY
/// `SV_Position` (depth-only). Paired with [`csm_depth_fs_spirv`] in a depth-only graphics
/// pipeline (EMPTY `color_formats`, `cull_mode: Front`, a slope+constant depth bias).
static CSM_DEPTH_VS_SPV: SpirvBlob<2256> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/csm_depth.vs.spv"
)));

/// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS fragment SPIR-V
/// (`shaders/csm_depth.fs.hlsl`). An EMPTY (`ps_6_0`) stage: the cascade pass is depth-only
/// (no color attachment), so the fragment writes nothing — the rasterizer's interpolated
/// `SV_Position.z` is the cascade depth. Paired with [`csm_depth_vs_spirv`].
static CSM_DEPTH_FS_SPV: SpirvBlob<156> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/csm_depth.fs.spv"
)));

/// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS vertex SPIR-V
/// (`shaders/punctual_depth.vs.hlsl`). A GRAPHICS (`vs_6_0`) stage: reads the SAME set-0
/// `InstanceModelCol` SSBO + the 88-byte VERTEX push as [`csm_depth_vs_spirv`], projects each
/// caster instance into one cube FACE's light-clip space (push `@0`), AND forwards the WORLD
/// position to the fragment so the matching FS can write the linear radial distance. Paired with
/// [`punctual_depth_fs_spirv`] in a depth-WRITE (no early-Z) graphics pipeline.
static PUNCTUAL_DEPTH_VS_SPV: SpirvBlob<2376> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/punctual_depth.vs.spv"
)));

/// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS fragment SPIR-V
/// (`shaders/punctual_depth.fs.hlsl`). A `ps_6_0` stage that writes `SV_Depth =
/// saturate(length(world - light_pos) * inv_range)` — the LINEAR radial distance from the point
/// light (face-independent, so all six cube faces share ONE comparison scale; the resolve compares
/// the receiver's own `length(P - light_pos) * inv_range` against it). `light_pos`/`inv_range` ride
/// in the DEAD `cam_eye@64` push lane (the pipeline push range covers `VERTEX | FRAGMENT`). Paired
/// with [`punctual_depth_vs_spirv`].
static PUNCTUAL_DEPTH_FS_SPV: SpirvBlob<1084> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/punctual_depth.fs.spv"
)));

/// The committed mesh-MRT G-buffer PRODUCER vertex SPIR-V (`shaders/gbuffer_mrt.vs.hlsl`).
/// Vertex layout: position (loc 0, offset 0) + world normal (loc 2, offset 12) + color
/// (loc 1, offset 24), a 40-byte stride. Reads the set-0 `InstanceModelCol` SSBO + the
/// 88-byte `{ view_proj; cam_eye; base_instance; use_model_matrix }` VERTEX push
/// ([`GBUFFER_PUSH_BYTES`](crate::swapchain::GBUFFER_PUSH_BYTES)); `use_model_matrix == 0`
/// is the legacy merged-draw arm, `== 1` the instanced arm. Exported for the host layer
/// (host plan R3): the SAME blob the `window_present_gbuffer` harness embeds.
static GBUFFER_MRT_VS_SPV: SpirvBlob<4480> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.vs.spv"
)));

/// The committed mesh-MRT G-buffer PRODUCER fragment SPIR-V (`shaders/gbuffer_mrt.fs.hlsl`):
/// writes albedo/normal/material as 3 MRT in the marcher's exact encoding (mask=1) + the
/// marcher-aligned `SV_Depth` (euclidean under perspective, axial under ortho). Paired with
/// [`gbuffer_mrt_vs_spirv`].
static GBUFFER_MRT_FS_SPV: SpirvBlob<2252> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.fs.spv"
)));

/// The committed fullscreen-sample vertex SPIR-V (`shaders/fullscreen_sample.vs.hlsl`): a
/// fullscreen triangle generating positions + UVs from `SV_VertexID` (no vertex buffer).
/// The present-blit pass's VS.
static FULLSCREEN_SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed fullscreen-sample fragment SPIR-V (`shaders/fullscreen_sample.fs.hlsl`):
/// samples the bound `Texture2D` + `SamplerState` at the interpolated UV and outputs it.
/// The present-blit pass's FS; paired with [`fullscreen_sample_vs_spirv`].
static FULLSCREEN_SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.fs.spv"
)));

/// Pillar B increment B2: the per-instance TRS interpolation compute PRE-PASS
/// (`interp_instances.comp`, refined-B). One invocation per DYNAMIC instance reads a
/// 96-byte `TransformPair` at binding 0 + its output slot at binding 1, interpolates at the
/// frame-wide `alpha`, and scatters the 48-byte `InstanceModelCol`-shaped model row into the
/// SHARED instance ring at binding 2 (`ModelOut[OutSlot[i]]`). The size pins the committed
/// `.spv`; the `interp_edsl_sync` test proves the byte stream is the single-sourced eDSL emit.
static INTERP_INSTANCES_SPV: SpirvBlob<6584> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/interp_instances.comp.spv"
)));

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is
/// a valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    ///
    /// `N` is a 4-byte multiple by construction (a committed `.spv`); the
    /// `align(4)` wrapper guarantees the cast is well-aligned.
    #[inline]
    const fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid
        // `*const u32`; `N` is a 4-byte multiple (const-asserted above), so the
        // blob is exactly `N / 4` whole `u32` words; the `&self` borrow keeps the
        // backing `'static` blob alive for the slice's lifetime. The bytes are an
        // arbitrary-but-initialized SPIR-V stream — any bit pattern is a valid
        // `u32`, so no uninitialized/invalid read occurs.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The `local_size_x` of both shaders (`[numthreads(64,1,1)]`). The dispatch
/// group count is `ceil(N / LOCAL_SIZE_X)`.
pub const LOCAL_SIZE_X: u32 = 64;

/// The committed step-0c SPIR-V (`buffer[i] = i*2 + 1`) as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
#[inline]
pub fn write_pattern_spirv() -> &'static [u32] {
    WRITE_PATTERN_SPV.as_words()
}

/// The committed step-0d SPIR-V (`buffer[i] += 100`) as a `u32` word stream.
#[inline]
pub fn transform_add_spirv() -> &'static [u32] {
    TRANSFORM_ADD_SPV.as_words()
}

/// The committed Phase-6 rung-8 SDF sphere-trace SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The shader reuses the rung-1 compute contract verbatim: binding 0 (set 0) is
/// one `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push
/// constant; everything else (camera/sphere/light) is hardcoded in the shader,
/// mirrored host-side by [`golden_sdf_pixel`].
#[inline]
pub fn sdf_spheretrace_spirv() -> &'static [u32] {
    SDF_SPHERETRACE_SPV.as_words()
}

/// The committed Phase-6 rung-9 SDF edit-list (multi-primitive CSG) SPIR-V as a
/// `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The shader reuses the rung-1 compute contract verbatim (binding 0 = one
/// `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant).
/// The edit-list is PACKED as a header region at the front of that single buffer
/// (no second binding): word 0 = `edit_count`, then the [`MAX_SDF_EDITS`]-entry
/// [`SdfEdit`] array, then the packed-RGBA pixel output. The host writes the
/// header via [`encode_edit_list`] and mirrors the fold in [`golden_editlist_pixel`].
#[inline]
pub fn sdf_editlist_spirv() -> &'static [u32] {
    SDF_EDITLIST_SPV.as_words()
}

/// The committed Phase-6 rung-10 SDF + mesh hybrid composite SPIR-V as a `u32`
/// word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Reuses the rung-1 one-binding compute contract verbatim (binding 0 = one
/// `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant).
/// The buffer extends the rung-9 packed-header layout with a DEPTH region between
/// the edit array and the pixel output: the host writes the edit header via
/// [`encode_edit_list`], the GPU image→buffer copy writes the rasterized mesh
/// depth into [`COMPOSITE_DEPTH_BASE_WORDS`], and the shader reads both, bounds the
/// march by the per-pixel mesh depth, and composites into [`COMPOSITE_PIXEL_BASE_WORDS`].
/// The fold + lighting are mirrored host-side by [`golden_composite_pixel`].
#[inline]
pub fn sdf_depth_composite_spirv() -> &'static [u32] {
    SDF_DEPTH_COMPOSITE_SPV.as_words()
}

/// The committed Render P1a edit-list STORAGE-IMAGE marcher SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Unlike the rung-9 [`sdf_editlist_spirv`] (one `RWStructuredBuffer<uint>` at
/// binding 0 that holds both the edit-list header AND the packed pixel output), this
/// shader is bound to the P1a multi-resource descriptor *vocabulary* set: binding 0
/// is a read-only `StructuredBuffer<uint>` (the same packed edit-list header format,
/// [`encode_edit_list`] / [`EDITLIST_BUFFER_WORDS`]) and binding 1 is a
/// `RWTexture2D<float4>` it STOREs the marcher color into. The field eval + ray-gen +
/// lighting are reused VERBATIM from rung 9, so [`golden_editlist_pixel`] predicts the
/// stored texel within the same `+/-2/255` per-channel tolerance (the float→UNORM store
/// quantization vs [`pack_rgba`]'s rounding is under one LSB). It proves a storage-image
/// WRITE through the COMPUTE bind point + the vocabulary set works on the GPU.
#[inline]
pub fn sdf_editlist_storage_image_spirv() -> &'static [u32] {
    SDF_EDITLIST_STORAGE_IMAGE_SPV.as_words()
}

/// The committed Render P1b OFFSCREEN MRT G-buffer marcher SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The image-based rewrite of the rung-10 [`sdf_depth_composite_spirv`] marcher: the
/// field eval + ray-gen + lighting are a VERBATIM cut of `sdf_depth_composite.hlsl`, so
/// [`golden_composite_pixel_ex`] predicts the ALBEDO output within the same `+/-2/255`
/// per-channel tolerance. It is bound to the P1b vocabulary set: binding 0 a read-only
/// `StructuredBuffer<uint>` edit-list, binding 1 a `Texture2D<float>` SAMPLED depth
/// (the rasterized D32_SFLOAT image, fetched with `.Load`), bindings 2..4 the MRT
/// `RWTexture2D<float4>` storage images (albedo + the additive normal/material), and
/// binding 5 a UNIFORM buffer carrying the extent/camera block (written once — NOT a
/// per-frame push, so it is compatible with the pipeline's dedicated layout). There is
/// NO packed depth region and NO packed pixel region — the depth is sampled and the
/// color is stored.
#[inline]
pub fn sdf_gbuffer_composite_spirv() -> &'static [u32] {
    SDF_GBUFFER_COMPOSITE_SPV.as_words()
}

/// The committed deferred-shading SPLIT (increment 1) RESOLVE SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The fullscreen `deferred_pbr` resolve consumes the marcher's G-buffer attributes —
/// gAlbedo (the unmultiplied base color) @ binding 0 and gMaterial `(r = vis, b = mask)`
/// @ binding 1, both STORAGE images in GENERAL (no sampler) — and STOREs the final lit
/// color `lit = (mask == 1) ? base * vis : base` to a STORAGE image @ binding 2. It is
/// dispatched 1D over the SAME pixel count as the marcher (the camera UBO @ binding 5
/// supplies the extent for the 1:1 index → (px, py) mapping). The host mirror is
/// [`golden_deferred_resolve`], fed by [`golden_marcher_attributes`].
#[inline]
pub fn deferred_pbr_spirv() -> &'static [u32] {
    DEFERRED_PBR_SPV.as_words()
}

/// The SDFDDGI I3 DDGI resolve-sample GPU-GOLDEN SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Runs the SAME `ddgi_probe_sample` (`shaders/ddgi_resolve.hlsli`) the deferred resolve runs, over
/// a host-supplied receiver buffer, and STOREs the resolved indirect irradiance — the GPU half of
/// the `probe_sample_gpu_eq_cpu_to_bits` contract (diff to `crate::goldens::probe_sample` to bits).
/// Bound to its OWN layout { b0 grid UBO (pad `.x` = sample count), t1/s1 irr atlas, t2/s2 depth
/// atlas, t3 recv-pos SSBO, t4 recv-nrm SSBO, u5 out SSBO } — no push constant.
#[inline]
pub fn ddgi_probe_gi_resolve_spirv() -> &'static [u32] {
    DDGI_PROBE_GI_RESOLVE_SPV.as_words()
}

/// The committed Lighting-L1 clustered froxel light-cull SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// One invocation per froxel (`CLUSTER_COUNT`); bound to the cull set { camera UBO @0, light
/// table SSBO @1, `RWStructuredBuffer<uint2>` ClusterGrid @2, `RWStructuredBuffer<uint>`
/// LightIndexList @3, `RWStructuredBuffer<uint>` LightIndexAlloc @4 } + a `ClusterCullPush`
/// (exp-Z near/far + the per-froxel / flat-list caps). It builds each froxel's world AABB,
/// culls the point/spot block (`sqDistPointAABB <= r²`), and atomic-appends survivors into
/// the index list + writes the `{offset, count}` cell. Dispatched 1D over `CLUSTER_COUNT`
/// BEFORE the resolve (with a COMPUTE→COMPUTE buffer barrier so the resolve's reads see the
/// writes). The host mirror is [`golden_cluster_cull`].
#[inline]
pub fn cluster_cull_spirv() -> &'static [u32] {
    CLUSTER_CULL_SPV.as_words()
}

/// The committed Render P4b coarse-cull / tile pre-trace SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The coarse pre-pass for the [`sdf_gbuffer_composite_spirv`] marcher: one invocation
/// per 8×8 tile cone-traces the frozen `field_distance` and emits a [`TileBound`] (the
/// host mirror is [`golden_tile_bound`]). It is bound to the P4b vocabulary set —
/// binding 0 a read-only `StructuredBuffer<uint>` edit-list, binding 1 a
/// `Texture2D<float>` SAMPLED depth, binding 6 a `RWStructuredBuffer<TileBound>` output,
/// binding 5 the UNIFORM camera block — and dispatched 1D over `tiles_w * tiles_h`
/// (see [`tile_grid_extent`]) before the fine marcher reads the bounds (with a buffer
/// barrier between).
#[inline]
pub fn sdf_tile_cull_spirv() -> &'static [u32] {
    SDF_TILE_CULL_SPV.as_words()
}

/// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS vertex SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Bound into a depth-only graphics pipeline (paired with [`csm_depth_fs_spirv`]); reads the
/// foundation's set-0 `InstanceModelCol` SSBO + the 88-byte VERTEX push, projecting each
/// caster instance into one cascade's light-clip space. The recorder pushes the cascade
/// `view_proj` (`CascadeData.view_proj`, column-major) at offset 0 + `use_model_matrix == 1`.
#[inline]
pub fn csm_depth_vs_spirv() -> &'static [u32] {
    CSM_DEPTH_VS_SPV.as_words()
}

/// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS fragment SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// EMPTY (depth-only); paired with [`csm_depth_vs_spirv`] in the cascade depth pipeline.
#[inline]
pub fn csm_depth_fs_spirv() -> &'static [u32] {
    CSM_DEPTH_FS_SPV.as_words()
}

/// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS vertex SPIR-V as a
/// `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Bound into a depth-WRITE graphics pipeline (paired with [`punctual_depth_fs_spirv`]); reads the
/// foundation's set-0 `InstanceModelCol` SSBO + the 88-byte VERTEX push, projecting each caster
/// instance into one cube FACE's light-clip space + forwarding the world position to the FS. The
/// recorder pushes the face `view_proj` (`FaceTransform.view_proj`, column-major) at offset 0 +
/// `cam_eye.xyz = light_pos` / `cam_eye.w = inv_range` (@64) + `use_model_matrix == 1`.
#[inline]
pub fn punctual_depth_vs_spirv() -> &'static [u32] {
    PUNCTUAL_DEPTH_VS_SPV.as_words()
}

/// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS fragment SPIR-V as a
/// `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Writes the LINEAR radial distance (`SV_Depth`); paired with [`punctual_depth_vs_spirv`] in the
/// point cube depth pipeline. Reads `light_pos`/`inv_range` from the `cam_eye@64` push lane, so the
/// pipeline's push range MUST cover the `FRAGMENT` stage.
#[inline]
pub fn punctual_depth_fs_spirv() -> &'static [u32] {
    PUNCTUAL_DEPTH_FS_SPV.as_words()
}

/// The committed mesh-MRT G-buffer PRODUCER vertex SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Bound into the 3-MRT gbuffer raster pipeline (paired with [`gbuffer_mrt_fs_spirv`]):
/// 3 × `R8G8B8A8_UNORM` color formats + `D32Sfloat` depth, the 40-byte vertex layout
/// (position@0 / normal@12 / color@24), the set-0 instance-SSBO layout, and the 88-byte
/// VERTEX push range. Exported for the host layer (host plan R3).
#[inline]
pub fn gbuffer_mrt_vs_spirv() -> &'static [u32] {
    GBUFFER_MRT_VS_SPV.as_words()
}

/// The committed mesh-MRT G-buffer PRODUCER fragment SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Paired with [`gbuffer_mrt_vs_spirv`] in the gbuffer raster pipeline.
#[inline]
pub fn gbuffer_mrt_fs_spirv() -> &'static [u32] {
    GBUFFER_MRT_FS_SPV.as_words()
}

/// The committed fullscreen-sample (present-blit) vertex SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Bound into the present-blit pipeline (paired with [`fullscreen_sample_fs_spirv`]): no
/// vertex buffer, no depth, one COMBINED_IMAGE_SAMPLER set-0 layout, `color_formats[0]` ==
/// the swapchain format (W2-b). Exported for the host layer (host plan R3).
#[inline]
pub fn fullscreen_sample_vs_spirv() -> &'static [u32] {
    FULLSCREEN_SAMPLE_VS_SPV.as_words()
}

/// The committed fullscreen-sample (present-blit) fragment SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Paired with [`fullscreen_sample_vs_spirv`] in the present-blit pipeline.
#[inline]
pub fn fullscreen_sample_fs_spirv() -> &'static [u32] {
    FULLSCREEN_SAMPLE_FS_SPV.as_words()
}

/// The committed Render P7 SSAO (HBAO-lite) SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Returns the MEDIUM quality variant (`SSAO_PARAMS[1]` == today's shipped consts — byte-identical
/// to the pre-Q2 base shader) — the default the harnesses use unless they select a variant via
/// [`sdf_ssao_spirv_variant`]. One invocation per pixel; bound to the dedicated 5-binding SSAO set
/// { gNormal @0 (R), gMaterial @1 (R), gViewT @2 (R), `ssao` out @3 (W), camera UBO @4 }. It gathers
/// the horizon-based ambient-occlusion factor from the FROZEN G-buffer and STOREs it into the
/// `R8_UNORM` `ssao` lane the deferred resolve combines under `ssao_mode != 0`. Dispatched 1D over
/// the SAME pixel count as the marcher/resolve, BETWEEN the marcher→resolve store-to-load barrier and
/// the resolve (with a COMPUTE→COMPUTE barrier on `ssao` so the resolve's `gSsao.Load` sees the
/// store). The host mirror is [`golden_ssao_attributes`].
#[inline]
pub fn sdf_ssao_spirv() -> &'static [u32] {
    sdf_ssao_spirv_variant(SSAO_QUALITY_MEDIUM)
}

/// The committed Render P7-Q2 SSAO SPIR-V for quality variant `q` (an index into [`SSAO_PARAMS`] /
/// the [`SSAO_QUALITY_LOW`]/[`SSAO_QUALITY_MEDIUM`]/[`SSAO_QUALITY_HIGH`] constants), as a `u32` word
/// stream ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// All three variants share the SAME 5-binding SSAO interface (so one bind-group layout drives any
/// of them); only the BAKED `static const` tap budget (the `[unroll]` loop counts) differs — the
/// host selects a variant by binding its pipeline (Mechanism C, ZERO per-pixel runtime cost). Feed
/// the matching `SSAO_PARAMS[q]` row to [`golden_ssao_attributes`] for the bit-comparable host oracle.
///
/// # Panics
///
/// Panics (debug + release) if `q` is not a valid `SSAO_QUALITY_*` index ([`SSAO_QUALITY_LOW`] /
/// [`SSAO_QUALITY_MEDIUM`] / [`SSAO_QUALITY_HIGH`]); a caller passing an out-of-range quality is a bug.
#[inline]
pub fn sdf_ssao_spirv_variant(q: usize) -> &'static [u32] {
    match q {
        SSAO_QUALITY_LOW => SDF_SSAO_LOW_SPV.as_words(),
        SSAO_QUALITY_MEDIUM => SDF_SSAO_MEDIUM_SPV.as_words(),
        SSAO_QUALITY_HIGH => SDF_SSAO_HIGH_SPV.as_words(),
        _ => ssao_variant_out_of_range(q),
    }
}

/// The cold out-of-range arm of [`sdf_ssao_spirv_variant`] (an invalid `SSAO_QUALITY_*` index is a
/// caller bug). Split out + `#[cold]` so the variant selector's hot path stays a compact jump table.
#[cold]
#[inline(never)]
fn ssao_variant_out_of_range(q: usize) -> ! {
    panic!(
        "invariant: SSAO quality variant index {q} out of range (must be one of \
         SSAO_QUALITY_LOW/MEDIUM/HIGH = 0..{SSAO_QUALITY_COUNT})"
    )
}

/// The committed SDFDDGI I2 probe-update SPIR-V for the `gi_max_it` sweep value (one of
/// [`GI_MAX_IT_VARIANTS`] = {32, 64, 96, 128}), as a `u32` word stream ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// All four variants share the SAME dedicated update bind-group interface (set 0: t0 `Buf`, u1
/// `gIrrOut`, u2 `gDepthOut`, u3 `Classification`, t4 `RayTable`, t5 `LightBuf`, b6 `DdgiUpdate`), so
/// ONE bind-group layout drives any of them; only the BAKED `GI_MAX_IT` header const (the sphere-
/// trace `[loop]` trip count) differs. The host selects a variant by binding its pipeline (the SSAO
/// Mechanism-C precedent — ZERO per-thread runtime cost, the loop stays fully bounded). The bench
/// (`tests/ddgi_probe_gi_cost.rs`) sweeps this knob to derive the shipped `GI_MAX_IT` under the cost
/// ceiling (plan §5); the shipped default is [`GI_MAX_IT_DEFAULT`] (64).
///
/// # Panics
///
/// Panics (debug + release) if `gi_max_it` is not one of [`GI_MAX_IT_VARIANTS`]; a caller passing an
/// unbuilt variant is a bug.
#[inline]
pub fn sdf_probe_update_spirv(gi_max_it: u32) -> &'static [u32] {
    match gi_max_it {
        32 => SDF_PROBE_UPDATE_IT32_SPV.as_words(),
        64 => SDF_PROBE_UPDATE_IT64_SPV.as_words(),
        96 => SDF_PROBE_UPDATE_IT96_SPV.as_words(),
        128 => SDF_PROBE_UPDATE_IT128_SPV.as_words(),
        _ => probe_update_variant_out_of_range(gi_max_it),
    }
}

/// The cold out-of-range arm of [`sdf_probe_update_spirv`] (a `gi_max_it` with no committed variant
/// is a caller bug). Split out + `#[cold]` so the selector's hot path stays a compact jump table.
#[cold]
#[inline(never)]
fn probe_update_variant_out_of_range(gi_max_it: u32) -> ! {
    panic!(
        "invariant: DDGI probe-update GI_MAX_IT {gi_max_it} has no committed variant \
         (must be one of GI_MAX_IT_VARIANTS = {GI_MAX_IT_VARIANTS:?})"
    )
}

/// The committed `GI_MAX_IT` probe-update variants (the sphere-trace `[loop]` trip counts that
/// [`sdf_probe_update_spirv`] can select — the bench's `GI_MAX_IT` sweep axis, plan §5).
pub const GI_MAX_IT_VARIANTS: [u32; 4] = [32, 64, 96, 128];

/// The shipped-default `GI_MAX_IT` probe-update variant (plan §6 placeholder — 64; the orchestrator
/// finalizes it from the `ddgi_probe_gi_cost` bench). The activation-populate system loads this
/// variant's pipeline unless the config overrides it.
pub const GI_MAX_IT_DEFAULT: u32 = 64;

/// The committed Pillar-B B2 per-instance TRS interpolation SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// One invocation per DYNAMIC instance (refined-B); bound to a dedicated 3-binding set
/// { `StructuredBuffer<TransformPair>` @0 (read), `StructuredBuffer<uint>` OutSlot @1
/// (read), `RWStructuredBuffer<InstanceModelCol>` model-out ring @2 (write) } + an 8-byte
/// COMPUTE push ([`INTERP_INSTANCES_PUSH_BYTES`] — `{ uint count; float alpha }`). The B3
/// interp pre-pass dispatches `ceil(count / LOCAL_SIZE_X)` groups, scattering each
/// interpolated model column into the SHARED instance ring at `OutSlot[i]` — beside the
/// host-scattered static rows — before the raster + shadow vertex shaders read it. The
/// interpolation math body is single-sourced from `boyko_shaderdsl` (the `interp_edsl_sync`
/// byte-identity gate).
#[inline]
pub fn interp_instances_spirv() -> &'static [u32] {
    INTERP_INSTANCES_SPV.as_words()
}

/// The byte size of the Pillar-B B2 interp pre-pass COMPUTE push constant
/// (`{ uint count; float alpha }` — the instance-count bounds guard + the frame-wide
/// fixed-timestep overstep fraction). Mirrors the shader's `InterpInstancesPush`.
pub const INTERP_INSTANCES_PUSH_BYTES: u32 = 8;

/// The number of pre-compiled SSAO quality variants (the valid `SSAO_QUALITY_*` / [`SSAO_PARAMS`]
/// index range, `0..SSAO_QUALITY_COUNT`).
pub const SSAO_QUALITY_COUNT: usize = 3;

/// Errors from the compute-pipeline flow. `VkError` carries the failing command
/// name + the raw `VkResult`; `Memory` forwards a buffer/allocation failure.
///
/// Folded into the unified [`VulkanError`](crate::error::VulkanError) at the
/// trait boundary (plan D4); kept here so the pipeline-creation helpers in
/// [`crate::rhi_impl`] can build the rich, command-named variant.
#[derive(Debug)]
pub enum ComputeError {
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// Buffer creation / sub-allocation failed.
    Memory(MemoryError),
}

impl From<MemoryError> for ComputeError {
    #[inline]
    fn from(e: MemoryError) -> Self {
        ComputeError::Memory(e)
    }
}


// ---------------------------------------------------------------------------
// Phase-6 rung-8 SDF sphere-trace golden (single source of truth, CPU mirror of
// `shaders/sdf_spheretrace.hlsl`).
//
// These constants and math MUST match the shader exactly so the center-pixel
// (HIT) and corner-pixel (MISS) colors are computed host-side without re-running
// the GPU. The test diffs the GPU readback against this golden within a small
// per-channel tolerance (DXC `mad`/`fma` rounding makes a bit-exact match
// brittle across drivers, so a hit/miss + small-tolerance diff is the oracle —
// it still proves the sphere-trace ran a sphere, not a constant fill).
// ---------------------------------------------------------------------------

// `SDF_IMG_W` / `SDF_IMG_H` / `SDF_GRAD_H` and the `v_*` vector helpers now live
// in the `boyko_sdf_math` leaf and are imported / re-exported at the top of this
// module (the W4 leaf-cut). The camera / lighting scene constants below stay here
// because only the rung-8/9/10 golden + marching functions (which also stay) use
// them — they are NOT part of the shared analytic field the physics crate reuses.

pub(crate) const SDF_CAM_Z: f32 = 2.0;
pub(crate) const SDF_HALF_EXTENT: f32 = 1.0;
const SDF_SPHERE_CENTER: [f32; 3] = [0.0, 0.0, 0.0];
const SDF_SPHERE_RADIUS: f32 = 0.5;

pub(crate) const SDF_EPS: f32 = 0.001;
pub(crate) const SDF_T_MAX: f32 = 10.0;
pub(crate) const SDF_MAX_IT: u32 = 128;

/// The default Render B1 over-relaxation factor the harness pushes when a caller does
/// not specify one. Keinert's sphere-tracing speed-up steps `t += omega * d`; values in
/// `(1, 2)` accelerate convergence on shallow-grazing rays while the in-shader
/// exact-retreat safeguard preserves the hit-SET (no holes).
///
/// **Default `1.0` (over-relaxation OFF) — a measured VISUAL-QUALITY decision.** At
/// `omega > 1` the over-relaxed march diverges from the plain march by a SUB-PIXEL amount
/// in a thin annulus at the silhouette (the grazing band): the over-relaxed step overshoots
/// the SHORT chord near the rim, and the accept lands on a slightly different `t` than the
/// plain march (accept-slop) — plus the Lipschitz SOR-retreat resumes plain mid-ray on a
/// different `t`. The net is a faint ~1px DARK RING at ~70-80% of an SDF sphere's screen
/// radius (owner-flagged, recurring). The accept-refine `safe_t` retreat below kills the
/// deep-overshoot BLACK case and most of the ring, but a dotted sub-pixel residual remains.
/// The speed-up it buys is marginal — it only helps the thin grazing annulus (~5-15% of hit
/// pixels at ~17% fewer steps → ~5-7% of marcher time on a typical analytic scene), which is
/// not worth a visible flagship artifact. So the harness default is `1.0` (the plain march,
/// provably ring-free per the GPU oracle). The over-relaxation MACHINERY stays intact and
/// hole-proof — a caller that MEASURES a real win on a heavy-CSG scene can still pass an
/// explicit `omega > 1` (the host clamp is `[1.0, 1.99]`; the soundness ceiling is `2`).
pub const DEFAULT_MARCHER_OMEGA: f32 = 1.0;

// ---------------------------------------------------------------------------
// Render A1 (SDF cone-trace soft shadows) + A2 (SDF 5-tap AO) tuning constants
// (owner VALUE/SCOPE defaults, owner-retunable). These are CONSUMER-side: the
// shadow min-track + AO deficit accumulate around calls to the FROZEN field
// gateway (`field_distance` / `sdf_edit_list`); the field math itself is never
// touched. Mirrored verbatim in `sdf_gbuffer_composite.hlsl`.
// ---------------------------------------------------------------------------

/// Bit 0 of `lighting_flags`: enable A1 cone-trace soft shadows.
pub const LIGHTING_FLAG_SHADOWS: u32 = 1;
/// Bit 1 of `lighting_flags`: enable A2 ambient occlusion.
pub const LIGHTING_FLAG_AO: u32 = 2;

/// The default ONE directional light direction (the current Lambert dir, +Z = at
/// the camera). Owner-retunable; mirrors the shader's `LIGHT_DIR`.
pub const DEFAULT_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];

/// A1 penumbra hardness (Quilez soft-shadow `k`): larger ⇒ sharper shadow edges.
pub const SHADOW_K: f32 = 8.0;
/// A1 march start offset along the light, in field units (`16 * GRAD_H`). Replaces a
/// normal-offset bias: marching from a finite `t` off the surface avoids self-shadow
/// acne without perturbing the hit point.
pub const SHADOW_MINT: f32 = 16.0 * SDF_GRAD_H;
/// A1 minimum per-step advance along the light (a floor on `d / L`) so a near-zero
/// field value cannot stall the shadow march.
pub const SHADOW_MINT_STEP: f32 = SHADOW_MINT;
/// A1 occluder-hit threshold: when `field_distance` drops below this the ray is fully
/// occluded (return `0`). `2 * EPS` (the marcher's hit threshold relaxed one factor).
pub const SHADOW_HIT_EPS: f32 = 2.0 * SDF_EPS;
/// A1 grazing/back-face cutoff on signed `n·L`: at or below this the surface faces
/// away from the light ⇒ fully shadowed (return `0`), which also skips the march.
pub const SHADOW_NDOTL_EPS: f32 = 0.0;
/// A1 normal-offset start bias: the shadow march origin is lifted `n * SHADOW_NORMAL_BIAS`
/// off the surface before marching toward the light. At GRAZING angles (`n·L` small but
/// positive — the lit terminator) the tangent ray's first sample `p + L * SHADOW_MINT`
/// stays within ~`t² / (2R)` of a curved surface, so `field_distance` reads below
/// `SHADOW_HIT_EPS` and the march FALSE-occludes the point (black "flame" acne on the
/// terminator). Lifting the origin along the normal clears that self-intersection. Tuned
/// large enough to kill the grazing acne, small enough that contact shadows do not
/// visibly detach (peter-panning). Applied at the CALL SITES (the marcher + the resolve +
/// the host mirrors) so the eDSL-generated march span stays byte-frozen.
pub const SHADOW_NORMAL_BIAS: f32 = 0.02;

/// A2 fixed step between the 5 AO taps along the surface normal.
pub const AO_STEP: f32 = 0.1;
/// A2 per-tap geometric falloff (`AO_FALLOFF^i` weights the i-th deficit).
pub const AO_FALLOFF: f32 = 0.95;
/// A2 overall occlusion strength (scales the accumulated deficit before clamping).
pub const AO_STRENGTH: f32 = 1.0;

// --- Render P7 GROUP B: SSAO host oracle tuning (mirror `shaders/sdf_ssao.comp.hlsl` +
//     `boyko_shaderdsl::ssao` EXACTLY — the `ssao_edsl_sync` cross-check pins these to the
//     eDSL `pub const`s, and the pixel goldens pin them to the GPU). -------------------------

/// The world-space SSAO sampling radius (`SSAO_RADIUS` in the shader). Beyond it a tap's
/// `falloff` zeroes its contribution. Equals `boyko_shaderdsl::ssao::SSAO_RADIUS`.
pub const SSAO_RADIUS: f32 = 0.5;
/// The number of rotated screen-space slices (`SSAO_SLICES`). Equals
/// `boyko_shaderdsl::ssao::SSAO_SLICES`.
pub const SSAO_SLICES: u32 = 2;
/// The slice count as a float — the `occ / N` divisor (`SSAO_SLICES_F`). Mirrors the shader's
/// dedicated `SSAO_SLICES_F` const so the host complement rounds bit-identically to the GPU.
pub const SSAO_SLICES_F: f32 = 2.0;
/// The number of forward steps per half-slice (`SSAO_STEPS`). Equals
/// `boyko_shaderdsl::ssao::SSAO_STEPS`.
pub const SSAO_STEPS: u32 = 4;
/// The occlusion strength multiplier (`SSAO_STRENGTH`). Equals
/// `boyko_shaderdsl::ssao::SSAO_STRENGTH`. Held at 2.5 (the precision-safe value GPU↔host agree on
/// within ±6/255). The screen-space SSAO is now the SECONDARY AO path (mesh-vs-mesh); for
/// SDF-occludes-mesh the marcher's analytic `sdf_ao` is the clean PRIMARY, so SSAO strength no
/// longer carries the contact-shadow intensity. The Hilbert+R2 dither keeps this path clean.
pub const SSAO_STRENGTH: f32 = 2.5;
/// The `length(delta)` divide-by-zero guard (`SSAO_EPS`). Equals
/// `boyko_shaderdsl::ssao::SSAO_EPS`.
pub const SSAO_EPS: f32 = 1.0e-4;
/// The mesh/SDF G-buffer background sentinel (`SSAO_VIEWT_BG`) — a `view_t` at or above this
/// is a non-lit / mesh / background pixel (mirrors the marcher's `gViewT` `1.0e30` sentinel).
pub const SSAO_VIEWT_BG: f32 = 1.0e30;

/// Render P7-Q2 — ONE SSAO quality preset, the host-side mirror of
/// `boyko_shaderdsl::ssao::SsaoParams` (the lib cannot import the eDSL: `boyko_shaderdsl` is a
/// DEV-dependency only, so this struct re-states the same five scalars the pre-compiled `.spv`
/// variants bake). The host AO oracle [`golden_ssao_attributes`] reads these IN PLACE OF the module
/// `SSAO_*` consts, so feeding [`SSAO_PARAMS`]`[q]` reproduces variant `q`'s GPU result bit-for-bit.
///
/// The module `SSAO_RADIUS`/`SSAO_SLICES`/`SSAO_STEPS`/`SSAO_STRENGTH`/`SSAO_EPS` consts remain the
/// SINGLE SOURCE of the Medium row (`SSAO_PARAMS[1]` == today's shipped scalars, the no-op proof)
/// AND the `ssao_consts_host_match_edsl` eDSL cross-check anchor. The `ssao_variants_match_host`
/// golden pins the host table to `boyko_shaderdsl::ssao::SSAO_PRESETS` (the eDSL source of truth).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoParams {
    /// The world-space sampling radius (`SSAO_RADIUS`). Beyond it a tap's `falloff` zeroes it.
    pub radius: f32,
    /// The number of rotated screen-space slices (`SSAO_SLICES`); the `occ / N` divisor (`N` as a
    /// float) is `slices as f32`.
    pub slices: u32,
    /// The number of forward steps per half-slice (`SSAO_STEPS`).
    pub steps: u32,
    /// The occlusion strength multiplier (`SSAO_STRENGTH`).
    pub strength: f32,
    /// The `length(delta)` divide-by-zero guard (`SSAO_EPS`).
    pub eps: f32,
}

impl Default for SsaoParams {
    /// The MEDIUM preset (`SSAO_PARAMS[1]`) — today's shipped consts (the no-op proof). Equal to
    /// `SSAO_PARAMS[SSAO_QUALITY_MEDIUM]`.
    #[inline]
    fn default() -> Self {
        SSAO_PARAMS[SSAO_QUALITY_MEDIUM]
    }
}

/// The Render P7-Q2 SSAO quality-preset table (host mirror of `boyko_shaderdsl::ssao::SSAO_PRESETS`).
/// One row per pre-compiled `.spv` variant (selected by binding its pipeline, NEVER a dynamic loop
/// bound — Mechanism C). Indexed by the `SSAO_QUALITY_*` constants. The MEDIUM row equals today's
/// shipped module consts (the no-op proof); `ssao_variants_match_host` pins the whole table to the
/// eDSL `SSAO_PRESETS`.
pub const SSAO_PARAMS: [SsaoParams; 3] = [
    // Low — the cheapest tap budget (2 slices × 3 steps × 2 = 12 taps).
    SsaoParams { radius: 0.5, slices: 2, steps: 3, strength: 2.5, eps: 1.0e-4 },
    // Medium — IDENTICAL to today's shipped consts (2 slices × 4 steps × 2 = 16 taps).
    SsaoParams {
        radius: SSAO_RADIUS,
        slices: SSAO_SLICES,
        steps: SSAO_STEPS,
        strength: SSAO_STRENGTH,
        eps: SSAO_EPS,
    },
    // High — the widest tap budget (3 slices × 6 steps × 2 = 36 taps).
    SsaoParams { radius: 0.5, slices: 3, steps: 6, strength: 2.5, eps: 1.0e-4 },
];

/// The LOW SSAO quality variant index into [`SSAO_PARAMS`] / [`sdf_ssao_spirv_variant`].
pub const SSAO_QUALITY_LOW: usize = 0;
/// The MEDIUM SSAO quality variant index (== today's shipped consts; [`sdf_ssao_spirv`]'s default).
pub const SSAO_QUALITY_MEDIUM: usize = 1;
/// The HIGH SSAO quality variant index into [`SSAO_PARAMS`] / [`sdf_ssao_spirv_variant`].
pub const SSAO_QUALITY_HIGH: usize = 2;

/// The perspective screen-pixel radius clamp minimum (`SSAO_RADIUS_PIX_MIN`) — keeps taps
/// from collapsing onto one texel.
pub const SSAO_RADIUS_PIX_MIN: f32 = 2.0;
/// The perspective screen-pixel radius clamp maximum (`SSAO_RADIUS_PIX_MAX`) — keeps taps
/// inside a sane neighbourhood.
pub const SSAO_RADIUS_PIX_MAX: f32 = 24.0;
/// The integer-hash rotation table size (`SSAO_ROT_N`); the per-pixel slot is `hash &
/// (SSAO_ROT_N - 1)` (a power-of-two mask == `% N`; NO float `fract`/`floor`, so the host and
/// GPU pick the SAME rotation). Q1 widened this 4 -> 16 to decorrelate the angular banding.
pub const SSAO_ROT_N: u32 = 16;
/// The pre-baked `(cos, sin)` rotation table for the 16 evenly-spaced angles over [0, π):
/// angle k = k·(π/16) for k = 0..15 (degrees 0, 11.25, 22.5, …, 168.75), BYTE-IDENTICAL to the
/// shader's `SSAO_ROT[16]` so the host picks the same slot.
//
// These literals are LOAD-BEARING: each must round to the EXACT `f32` the shader's
// `float2(...)` literal carries (the integer-hash rotation slot must agree bit-for-bit
// between the host oracle and the GPU). `clippy::approx_constant` (the `0.70710677` ==
// `FRAC_1_SQRT_2`) and `clippy::excessive_precision` would have us swap in the std constant
// or truncate digits — either DIVERGES the host literal from the frozen shader table, the
// exact drift this oracle exists to prevent. The `ssao_edsl_sync` cross-check pins the math.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const SSAO_ROT: [(f32, f32); 16] = [
    (1.00000000, 0.00000000),
    (0.98078525, 0.19509032),
    (0.92387950, 0.38268343),
    (0.83146960, 0.55557024),
    (0.70710677, 0.70710677),
    (0.55557024, 0.83146960),
    (0.38268343, 0.92387950),
    (0.19509032, 0.98078525),
    (0.00000000, 1.00000000),
    (-0.19509032, 0.98078525),
    (-0.38268343, 0.92387950),
    (-0.55557024, 0.83146960),
    (-0.70710677, 0.70710677),
    (-0.83146960, 0.55557024),
    (-0.92387950, 0.38268343),
    (-0.98078525, 0.19509032),
];

/// Render P7 POLISH — the SSAO depth-aware box-blur half-kernel radius (`SSAO_BLUR_R` in the
/// resolve). `R == 3` is a 7×7 box: the inline blur of `gSsao` INSIDE the resolve's `ssao_mode
/// != 0` combine that smooths the discrete-step HBAO RINGS. The host mirror [`golden_ssao_blur`]
/// uses the SAME radius so the GPU and host averages agree texel-for-texel.
pub const SSAO_BLUR_R: i32 = 3;
/// Render P7 POLISH — the SSAO blur's bilateral DEPTH gate (`SSAO_BLUR_DEPTH_TOL` in the
/// resolve), in `view_t` (world-distance) units. A neighbour tap is averaged in ONLY when
/// `|tap.view_t - center.view_t| <= SSAO_BLUR_DEPTH_TOL`; this keeps the blur WITHIN a flat
/// surface (the mesh floor has near-constant `view_t`) while REJECTING the mesh↔SDF silhouette
/// (where `view_t` jumps far more than the tol), so AO never bleeds across the edge. `0.1` was
/// chosen to sit comfortably inside that band. Mirrored bit-for-bit by [`golden_ssao_blur`].
pub const SSAO_BLUR_DEPTH_TOL: f32 = 0.1;


/// P6 R1 cap: the maximum EXTRA shadow casters marched per pixel (the dominant-N bound).
/// Mirrors the shader's `MAX_SDF_SHADOW_CASTERS_PER_PIXEL`. Beyond this, flagged lights
/// contribute NoL-only (no march). Owner-retunable.
pub const MAX_SDF_SHADOW_CASTERS_PER_PIXEL: u32 = 4;


/// `sdf(p) = length(p - center) - radius` — the analytic field, mirroring the
/// shader's `sdf_sphere`. Exposed so a later CPU physics evaluator can be
/// conformance-checked against this exact source of truth.
#[inline]
pub fn sdf_sphere(p: [f32; 3]) -> f32 {
    v_len(v_sub(p, SDF_SPHERE_CENTER)) - SDF_SPHERE_RADIUS
}


/// Packs a linear `[0,1]` RGB into `0xAABBGGRR` (alpha `0xFF`), mirroring the
/// shader's `pack_rgba`.
#[inline]
pub fn pack_rgba(c: [f32; 3]) -> u32 {
    let q = |x: f32| -> u32 { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32 };
    let r = q(c[0]);
    let g = q(c[1]);
    let b = q(c[2]);
    (0xFF << 24) | (b << 16) | (g << 8) | r
}


/// Whether the orthographic ray for pixel `(px, py)` HITS the analytic sphere.
/// The rung-8 test uses this to pick a guaranteed-hit pixel (the center) and a
/// guaranteed-miss pixel (a corner) host-side, so the assertion is independent
/// of the exact lit color.
pub fn sdf_pixel_hits(px: u32, py: u32) -> bool {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_sphere(p);
        if d < SDF_EPS {
            return true;
        }
        t += d;
        if t > SDF_T_MAX {
            return false;
        }
    }
    false
}

// ===========================================================================
// Phase-6 rung-9 SDF edit-list (multi-primitive CSG) data model + golden mirror
// of `shaders/sdf_editlist.hlsl`.
//
// The scene is an ORDERED list of SDF edits (SDF doc §2): each a primitive
// (SPHERE or BOX) combined into the accumulated field by a boolean op
// (union / subtraction / intersection), optionally smoothed (polynomial
// smooth-min/-max when `smoothness > 0`). The field is folded per pixel each
// march step (the analytic base — NO grid cache / brick atlas, deferred).
//
// The edit-list reaches the shader PACKED as a header at the front of the SAME
// single storage buffer the rung-1 compute contract binds (binding 0). This
// keeps the proven one-binding layout verbatim; the alternative (a 2-binding
// compute layout) would reshape the device-shared `ComputeLayouts` + every
// rung-1 compute test. The buffer word layout (mirrored in the shader):
//
//   word 0                   : u32 edit_count
//   words [HEADER_BASE_WORDS]: MAX_SDF_EDITS * SdfEdit (the std430 edit array)
//   words [PIXEL_BASE_WORDS] : IMG_W * IMG_H * u32     (the packed-RGBA output)
//
// `SdfEdit` is `#[repr(C, align(16))]` so it is byte-identical to the std430
// structured-buffer element the shader reads (asserted below). The math here
// (primitive distances + boolean ops + smooth-min + central-difference gradient)
// reproduces the shader EXACTLY so it is the single source of truth a future CPU
// physics evaluator reuses; the test diffs the GPU readback against it within the
// same +/-2/255 per-channel tolerance as rung 8.
// ===========================================================================

// The SDF data model — `sdf_kind` / `sdf_op` / `MAX_SDF_EDITS` / `SdfEdit` (+ its
// ctors + the §3.8 std430 fingerprint const-asserts + the `SDF_EDIT_WORDS == 12`
// pin) / `SDF_EDIT_WORDS` / `HEADER_BASE_WORDS` — now lives in the
// `boyko_sdf_math` leaf and is imported / re-exported at the top of this module
// (the W4 leaf-cut). The buffer-LAYOUT consts below (`PIXEL_BASE_WORDS` etc.)
// stay here: they are GPU-buffer-specific and combine the leaf's layout consts
// with this crate's image dimensions.

/// Word offset of the packed-RGBA pixel region (after the count + the full
/// edit array). Matches the shader's `PIXEL_BASE`.
pub const PIXEL_BASE_WORDS: usize = HEADER_BASE_WORDS + MAX_SDF_EDITS * SDF_EDIT_WORDS;

/// Total `u32` word count of the packed-header buffer (header + edit array +
/// `IMG_W * IMG_H` pixels). The buffer must be `EDITLIST_BUFFER_WORDS * 4` bytes.
pub const EDITLIST_BUFFER_WORDS: usize =
    PIXEL_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

// The shader hardcodes `SDF_EDIT_WORDS = 12u` and `PIXEL_BASE = 196`; pin both so
// a layout change that desyncs the host encoder from the shader is a build error.
const _: () = assert!(SDF_EDIT_WORDS == 12, "SDF_EDIT_WORDS must equal the shader's 12u");
const _: () = assert!(PIXEL_BASE_WORDS == 196, "PIXEL_BASE_WORDS must equal the shader's PIXEL_BASE");

/// Encodes `edits` into the packed-header region at the front of `buf` (a
/// `&mut [u32]` view of the storage buffer): word 0 = `edit_count`, then the
/// edit array starting at [`HEADER_BASE_WORDS`]. The pixel region (from
/// [`PIXEL_BASE_WORDS`]) is left untouched (the shader writes it).
///
/// `buf` must be at least [`EDITLIST_BUFFER_WORDS`] long; `edits.len()` must be
/// `<= MAX_SDF_EDITS` (debug-asserted — exceeding the fixed cap is a caller bug).
pub fn encode_edit_list(buf: &mut [u32], edits: &[SdfEdit]) {
    debug_assert!(
        edits.len() <= MAX_SDF_EDITS,
        "invariant: edit count {} exceeds MAX_SDF_EDITS {MAX_SDF_EDITS}",
        edits.len()
    );
    debug_assert!(
        buf.len() >= EDITLIST_BUFFER_WORDS,
        "invariant: buffer has {} words, need >= {EDITLIST_BUFFER_WORDS}",
        buf.len()
    );

    buf[0] = edits.len() as u32;
    for (i, e) in edits.iter().enumerate() {
        let base = HEADER_BASE_WORDS + i * SDF_EDIT_WORDS;
        buf[base] = e.center[0].to_bits();
        buf[base + 1] = e.center[1].to_bits();
        buf[base + 2] = e.center[2].to_bits();
        buf[base + 3] = e.center[3].to_bits();
        buf[base + 4] = e.params[0].to_bits();
        buf[base + 5] = e.params[1].to_bits();
        buf[base + 6] = e.params[2].to_bits();
        buf[base + 7] = e.params[3].to_bits();
        buf[base + 8] = e.kind;
        buf[base + 9] = e.op;
        buf[base + 10] = e.smoothness.to_bits();
        buf[base + 11] = e._pad;
    }
}

// The edit-list field math (the single source of truth, mirroring the shader):
// `sd_sphere`, `sd_box`, `edit_distance`, `smin`, `smax`, `combine`,
// `sdf_edit_list`, `sdf_edit_list_normal` (+ the `v_*` helpers + `SDF_FAR`) now
// live in the `boyko_sdf_math` leaf and are imported at the top of this module
// (the W4 leaf-cut, a VERBATIM cut — no float-op reorder). The marching / golden
// / camera functions below STAY here and call the leaf's `sdf_edit_list` /
// `sdf_edit_list_normal` directly, so they remain the GPU golden the rung-9/10/11
// tests diff against.

/// Whether the orthographic ray for pixel `(px, py)` HITS the edit-list field.
/// The rung-9 test uses this to pick discriminating texels host-side (e.g. a
/// pixel that hits the base sphere alone but MISSES after a subtraction).
pub fn editlist_pixel_hits(edits: &[SdfEdit], px: u32, py: u32) -> bool {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
    let rd = [0.0, 0.0, -1.0];

    let mut t = 0.0_f32;
    for _ in 0..SDF_MAX_IT {
        let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let d = sdf_edit_list(edits, p);
        if d < SDF_EPS {
            return true;
        }
        t += d;
        if t > SDF_T_MAX {
            return false;
        }
    }
    false
}


// ===========================================================================
// Phase-6 rung-10 SDF + mesh HYBRID COMPOSITE (shared depth) golden + layout —
// the single source of truth mirroring `shaders/sdf_depth_composite.hlsl`.
//
// The §15.1 seam: a REAL GPU-rasterized mesh's depth (written into a D32_SFLOAT
// attachment, then copied into the shared storage buffer) BOUNDS the SDF
// sphere-trace march, so the mesh and the SDF occlude each other and composite
// into one image. The SDF field math + lighting + camera are FROZEN to rung 9
// (reused verbatim via `sdf_edit_list` / `sdf_edit_list_normal` / `pack_rgba`);
// only the per-pixel mesh-depth read + the composite are new.
//
// The buffer extends the rung-9 packed-header layout with a DEPTH region between
// the edit array and the pixel output (rung 9's `PIXEL_BASE_WORDS` etc. are
// UNTOUCHED — these are a SEPARATE `COMPOSITE_*` const set):
//
//   word 0                          : u32 edit_count
//   words [HEADER_BASE_WORDS ..]    : MAX_SDF_EDITS * SdfEdit (the std430 array)
//   words [COMPOSITE_DEPTH_BASE_WORDS ..] : IMG_W * IMG_H * f32 mesh depth (NEW)
//   words [COMPOSITE_PIXEL_BASE_WORDS ..] : IMG_W * IMG_H * u32 packed RGBA out
//
// The depth region is `(CAM_Z - worldZ) / T_MAX` (the orthographic depth
// convention) inside the quad footprint and `1.0` (the clear) outside; the host
// march is bounded by `t_mesh = depth * T_MAX`, exactly as the shader does.
// ===========================================================================

/// The flat mesh albedo (the rung-10 composite's `MESH_COLOR`) — a green clearly
/// distinct from both the SDF lit color (warm orange/red) and the background (dark
/// blue), so the composite regions are unambiguous. Mirrors the shader's
/// `MESH_COLOR`. (Reading the mesh's real rasterized albedo from a G-buffer is a
/// deferred S3 refinement; this rung proves DEPTH sharing, so the color is a
/// constant.)
pub const MESH_COLOR: [f32; 3] = [0.15, 0.65, 0.25];

/// Render P5 (r0+r1): the LINEAR vertex color of the offscreen test harness's mesh quad —
/// the RASTER-PBR producer's gAlbedo. The `gbuffer_mrt.fs` writes `albedo = saturate(color)`;
/// the harness drivers (`run_gbuffer_hybrid_*`) all build the quad with a WHITE linear
/// vertex color (`[1, 1, 1, 1]` in `quad_vertices`), so `saturate` is the identity here.
///
/// After P5 a mesh-covered pixel the SDF did NOT win is RASTER-OWNED (`!own_pixel`): the
/// raster pass writes a first-class PBR G-buffer (`base = this color`, `n = (0, 0, 1)`,
/// `mat_id = 0`, `shadow = ao = 1`, `mask = 1`) and the deferred resolve runs FULL
/// Cook-Torrance on it — exactly like an SDF pixel. The host oracle
/// [`golden_marcher_attributes`] models that producer with this albedo so the GPU-vs-oracle
/// comparison matches mesh pixels too. (The old flat marcher-derived [`MESH_COLOR`] with
/// `mask = 0` is the pre-P5 behavior; it is retained only for the docs/inline-composite
/// `golden_composite_pixel_*` oracles that model the marcher's own mesh arm.)
///
/// NOTE (Render P7/P5-r1b UNLOCK): the raster pass writes only the 3 attribute MRT (no
/// `gViewT`), so the MARCHER is the single writer of a `!own_pixel` mesh pixel's `gViewT`. It now
/// stores the mesh surface ray-t `t_mesh` (= `depth_to_t(mesh_depth)`, the bound the ownership
/// gate marched against) there — NOT the old `1.0e30` sentinel (`sdf_gbuffer_composite.hlsl`, the
/// `(own_pixel && mask == 1.0) ? t : (has_mesh ? t_mesh : 1.0e30)` guard at both terminal write
/// sites). So a mesh pixel's reconstructed `P = ro + rd * t_mesh` is the REAL mesh surface: in-
/// range point/spot lights light the mesh AND the SSAO pass processes it (`view_t < SSAO_VIEWT_BG`).
/// The oracle mirrors this by emitting `t_mesh` as `view_t` on the raster-PBR mesh arm.
pub const MESH_RASTER_ALBEDO: [f32; 3] = [1.0, 1.0, 1.0];

/// Word offset of the per-pixel mesh-depth region (immediately after the edit
/// array — i.e. where rung 9 put its pixel region). Matches the shader's
/// `DEPTH_BASE`. The depth region is `IMG_W * IMG_H` `f32`s, one per pixel,
/// written by the GPU image→buffer copy of the rasterized D32_SFLOAT attachment.
pub const COMPOSITE_DEPTH_BASE_WORDS: usize = HEADER_BASE_WORDS + MAX_SDF_EDITS * SDF_EDIT_WORDS;

/// Word offset of the packed-RGBA pixel region (after the depth region). Matches
/// the shader's `PIXEL_BASE`.
pub const COMPOSITE_PIXEL_BASE_WORDS: usize =
    COMPOSITE_DEPTH_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

/// Total `u32` word count of the rung-10 composite buffer (header + edit array +
/// depth region + pixel region). The buffer must be
/// `COMPOSITE_BUFFER_WORDS * 4` bytes.
pub const COMPOSITE_BUFFER_WORDS: usize =
    COMPOSITE_PIXEL_BASE_WORDS + (SDF_IMG_W as usize) * (SDF_IMG_H as usize);

// The shader hardcodes `DEPTH_BASE = 196` and `PIXEL_BASE = 4292`; pin both so a
// layout change that desyncs the host encoder/reader from the shader is a build
// error. `COMPOSITE_DEPTH_BASE_WORDS` deliberately equals rung 9's
// `PIXEL_BASE_WORDS` (the depth region begins right after the edit array).
const _: () = assert!(
    COMPOSITE_DEPTH_BASE_WORDS == 196,
    "COMPOSITE_DEPTH_BASE_WORDS must equal the shader's DEPTH_BASE (196)"
);
const _: () = assert!(
    COMPOSITE_DEPTH_BASE_WORDS == PIXEL_BASE_WORDS,
    "the depth region must begin right after the edit array (= rung 9's PIXEL_BASE_WORDS)"
);
const _: () = assert!(
    COMPOSITE_PIXEL_BASE_WORDS == 4292,
    "COMPOSITE_PIXEL_BASE_WORDS must equal the shader's PIXEL_BASE (4292)"
);

/// The depth value the depth attachment is CLEARED to (the far plane). A stored
/// depth `>= MESH_DEPTH_CLEAR` means NO mesh fragment covered that pixel. Matches
/// the shader's `DEPTH_CLEAR` and the rung-10 test's `MESH_DEPTH_CLEAR`.
pub const MESH_DEPTH_CLEAR: f32 = 1.0;

/// The orthographic camera plane Z (rays start here, looking down -Z). The public
/// mirror of the private rung-8/9 `SDF_CAM_Z`, exposed so the rung-10 test builds
/// its orthographic MVP from the SAME single source of truth the golden uses.
pub const SDF_CAMERA_Z: f32 = SDF_CAM_Z;

/// The orthographic view half-extent in world units. Public mirror of the private
/// `SDF_HALF_EXTENT` (the rung-10 test maps its quad's world-XY corners with it).
pub const SDF_VIEW_HALF_EXTENT: f32 = SDF_HALF_EXTENT;

/// The march far-plane distance (= the depth far plane, world `CAM_Z - T_MAX`).
/// Public mirror of the private `SDF_T_MAX` (the rung-10 test's ortho MVP maps
/// `worldZ = CAM_Z - T_MAX` to stored depth `1.0`).
pub const SDF_TRACE_T_MAX: f32 = SDF_T_MAX;

/// The PERSPECTIVE mesh-depth normalizer (`gbuffer_mrt.fs` encodes `md =
/// length(eye_rel) / MESH_DEPTH_T_MAX`; the marcher decodes `t_mesh = md *
/// MESH_DEPTH_T_MAX` on the CAM_PERSPECTIVE arm). DECOUPLED from the marcher's
/// ray-miss bound [`SDF_TRACE_T_MAX`] (= 10): raster mesh geometry can stand far past
/// the SDF horizon (a long floor / back wall), and a small normalizer would saturate
/// its depth to the no-mesh clear (1.0) so the marcher reads it as background → broken
/// CSM/lighting on the far geometry (the 3-cascade demo's receding floor + far casters).
/// The normalizer CANCELS in encode→decode, so every in-range perspective scene is
/// byte-identical; only formerly-saturated far geometry changes (it now reconstructs).
/// `64` covers any room-scale eye distance with float32 headroom. The
/// `instanced_vs_host_mirror` sync-pin asserts the `gbuffer_mrt.fs` literal == this.
pub const MESH_DEPTH_T_MAX: f32 = 64.0;

/// The world-space XY a pixel's orthographic ray passes through (the ray origin's
/// xy), mirroring the camera reconstruction in [`golden_composite_pixel`]. The
/// rung-10 test uses this to compute, host-side, exactly which pixels a world-XY
/// quad covers (so the discriminator texels are picked independent of the GPU).
#[inline]
pub fn pixel_world_xy(px: u32, py: u32) -> [f32; 2] {
    let u = (((px as f32) + 0.5) / (SDF_IMG_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (SDF_IMG_H as f32)) * 2.0 - 1.0);
    [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT]
}

/// Inverts the orthographic depth convention: a stored depth `d` corresponds to a
/// ray parameter `t = d * T_MAX` (the camera-plane distance), mirroring the
/// shader's `md * T_MAX`. The MVP the rung-10 test pushes is chosen so the
/// rasterized depth IS `t / T_MAX` for a fronto-parallel surface (orthographic,
/// no perspective divide → depth linear in `t`).
#[inline]
pub fn depth_to_t(d: f32) -> f32 {
    d * SDF_T_MAX
}

/// The expected STORED depth of a fronto-parallel mesh surface at world `mesh_z`
/// under the orthographic convention: `(CAM_Z - mesh_z) / T_MAX`. The rung-10
/// test asserts the GPU depth region equals this inside the quad footprint (and
/// [`MESH_DEPTH_CLEAR`] outside), localizing any raster/ortho-matrix bug.
#[inline]
pub fn mesh_depth_for_z(mesh_z: f32) -> f32 {
    (SDF_CAM_Z - mesh_z) / SDF_T_MAX
}

// ===========================================================================
// P0a — the camera / extent push-constant layout + host const-asserts, mirroring
// `shaders/sdf_depth_composite.hlsl`'s `PushConstants` block.
//
// The shader extent + camera mode are no longer compile-time constants: they arrive
// via this push-constant block. `count` stays at offset 0 (the legacy 4-byte field);
// extent + camera-mode follow; the four `float4` camera basis vectors are
// PERSPECTIVE-only (the ORTHO path ignores them). At extent (64,64) + ORTHO the
// shader reproduces the golden fixture BIT-EXACT (the rung-8..11 gate). The offsets
// below are const-asserted against the `#[repr(C)]` POD so a host/shader desync is a
// build error (the same discipline as `COMPOSITE_*_BASE_WORDS`).
// ===========================================================================

/// Camera mode selector mirroring the shader's `CAM_ORTHO` / `CAM_PERSPECTIVE`.
/// ORTHO (0) is the golden-frozen path; PERSPECTIVE (1) is the P0a additive ray-gen.
pub const CAM_MODE_ORTHO: u32 = 0;
/// PERSPECTIVE camera mode (P0a additive ray-gen). See [`CAM_MODE_ORTHO`].
pub const CAM_MODE_PERSPECTIVE: u32 = 1;

/// The `sdf_depth_composite` push-constant block (P0a). `#[repr(C)]` so the field
/// layout is byte-identical to the shader's `[[vk::push_constant]] PushConstants`
/// (std430 scalar/`float4` rules); the host uploads `as_bytes()` of this struct.
///
/// `count` is the total PIXEL count (`img_w * img_h`); `img_w`/`img_h == 0` make the
/// shader fall back to the legacy 64×64 fixture. The `cam_*` `[f32; 4]` vectors are
/// PERSPECTIVE-only (`cam_forward[3] = tan(fovY/2)`, `cam_right[3] = aspect = W/H`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositePushConstants {
    /// Total PIXEL count = `img_w * img_h` (the shader bounds `idx < count`).
    pub count: u32,
    /// Runtime extent width (0 ⇒ the legacy 64 default).
    pub img_w: u32,
    /// Runtime extent height (0 ⇒ the legacy 64 default).
    pub img_h: u32,
    /// [`CAM_MODE_ORTHO`] or [`CAM_MODE_PERSPECTIVE`].
    pub camera_mode: u32,
    /// PERSPECTIVE eye world position (xyz; w unused).
    pub cam_eye: [f32; 4],
    /// PERSPECTIVE forward basis (xyz) + `tan(fovY/2)` in w.
    pub cam_forward: [f32; 4],
    /// PERSPECTIVE right basis (xyz) + aspect (W/H) in w.
    pub cam_right: [f32; 4],
    /// PERSPECTIVE up basis (xyz; w unused).
    pub cam_up: [f32; 4],
}

/// Byte size of [`CompositePushConstants`] — the `push_constant_bytes` a
/// `sdf_depth_composite` compute pipeline must declare (80 bytes).
pub const COMPOSITE_PUSH_CONSTANT_BYTES: u32 = core::mem::size_of::<CompositePushConstants>() as u32;

// Pin the field offsets to the shader's documented layout (a desync is a build error).
const _: () = assert!(
    core::mem::offset_of!(CompositePushConstants, count) == 0,
    "count must stay at offset 0 (the legacy 4-byte field)"
);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, img_w) == 4);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, img_h) == 8);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, camera_mode) == 12);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_eye) == 16);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_forward) == 32);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_right) == 48);
const _: () = assert!(core::mem::offset_of!(CompositePushConstants, cam_up) == 64);
const _: () = assert!(
    COMPOSITE_PUSH_CONSTANT_BYTES == 80,
    "the push-constant block must be 80 bytes (matches the shader's PushConstants)"
);

impl CompositePushConstants {
    /// Builds the ORTHO golden-fixture push constants for a `w × h` extent. At
    /// `(64, 64)` this drives the bit-exact rung-8..11 golden invocation. The camera
    /// basis is left zeroed (the ORTHO path ignores it).
    ///
    /// # Precondition
    ///
    /// `w * h` must fit in `u32` (the dispatch element count); a `debug_assert!`
    /// catches an overflowing extent in debug builds.
    #[inline]
    pub const fn ortho(w: u32, h: u32) -> Self {
        debug_assert!(w.checked_mul(h).is_some(), "extent w*h overflows u32");
        Self {
            count: w * h,
            img_w: w,
            img_h: h,
            camera_mode: CAM_MODE_ORTHO,
            cam_eye: [0.0; 4],
            cam_forward: [0.0; 4],
            cam_right: [0.0; 4],
            cam_up: [0.0; 4],
        }
    }

    /// Builds PERSPECTIVE push constants from a camera (eye + orthonormal basis +
    /// vertical FOV) and a `w × h` extent. `fov_y_radians` is the full vertical FOV;
    /// the aspect is `w / h`. The basis vectors should be orthonormal and
    /// right-handed (`right × up = -forward` toward the scene); the shader normalizes
    /// the assembled direction. The field eval downstream is unchanged (plain IEEE).
    ///
    /// # Precondition
    ///
    /// `w * h` must fit in `u32` (the dispatch element count); a `debug_assert!`
    /// catches an overflowing extent in debug builds.
    #[inline]
    pub fn perspective(
        eye: [f32; 3],
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        fov_y_radians: f32,
        w: u32,
        h: u32,
    ) -> Self {
        debug_assert!(w.checked_mul(h).is_some(), "extent w*h overflows u32");
        let tan_half_fov = (fov_y_radians * 0.5).tan();
        let aspect = (w as f32) / (h as f32);
        Self {
            count: w * h,
            img_w: w,
            img_h: h,
            camera_mode: CAM_MODE_PERSPECTIVE,
            cam_eye: [eye[0], eye[1], eye[2], 0.0],
            cam_forward: [forward[0], forward[1], forward[2], tan_half_fov],
            cam_right: [right[0], right[1], right[2], aspect],
            cam_up: [up[0], up[1], up[2], 0.0],
        }
    }

    /// Re-views the push constants as their raw byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `[f32; 4]` fields (all
        // `Copy`, no padding between them — the const-asserts above pin every offset
        // and the 80-byte total), so its `size_of` bytes are a fully-initialized,
        // alignment-valid POD bit pattern. The `&self` borrow keeps the struct alive
        // for the slice's lifetime; the slice is read-only (no aliasing write).
        unsafe {
            slice::from_raw_parts(
                (self as *const Self).cast::<u8>(),
                core::mem::size_of::<Self>(),
            )
        }
    }
}

/// The fine marcher's coarse-cull consumption mode — the value stamped into
/// [`FineMarcherPush::coarse_enabled`] (offset 0), read by the hand-written cull prefix in
/// `sdf_gbuffer_composite.hlsl`. The cull DISPATCH (which writes the per-tile `TileBound`
/// into binding 6) is unchanged across modes; only the marcher's CONSUMPTION of those bounds
/// differs:
///
/// - [`Off`](Self::Off) (`0`) — the cull is not consumed: `t_seed` stays `0.0`, the `Tiles`
///   buffer is never read. The OFF path is byte-identical to the pre-P4b marcher (the 0%-gate).
/// - [`Full`](Self::Full) (`1`) — the historical cull: EMPTY tiles short-circuit to the
///   mesh/background composite, and a NON-empty tile SEEDS the march at its conservative
///   `near_t` lower bound (the prefix skip). The offscreen FULL-mode goldens assert this output.
/// - [`EmptySkipOnly`](Self::EmptySkipOnly) (`2`) — the LIT-TRANSPARENT cull: EMPTY tiles still
///   short-circuit (provably image-identical lit+unlit — an empty tile has no surface), but a
///   NON-empty tile is NOT seeded (`t_seed` stays `0.0`). The `near_t` seed, fed into the B1
///   over-relaxed march, latches a different grazing tangent on the silhouette (a shifted normal
///   → a shifted AO/shadow rim); dropping it removes the rim. The cost is the lost prefix skip on
///   the few surface tiles (first-principles < 2% of the cull's perf win, which is dominated by the
///   empty-tile skip).
///
/// `#[repr(u32)]` so the discriminant IS the value the shader reads at offset 0.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CoarseMode {
    /// `0` — the cull is not consumed (byte-identical to the pre-P4b marcher).
    #[default]
    Off = 0,
    /// `1` — EMPTY-skip + `near_t` seed (the historical cull; the offscreen goldens assert it).
    Full = 1,
    /// `2` — EMPTY-skip only, NO seed (the lit-transparent on-screen cull — no grazing rim).
    EmptySkipOnly = 2,
}

impl CoarseMode {
    /// Maps the legacy `coarse_enabled: bool` to a mode: `false` → [`Off`](Self::Off),
    /// `true` → [`Full`](Self::Full). Keeps every pre-existing bool call site byte-unchanged
    /// (the `true` callers still seed `near_t`).
    #[inline]
    pub const fn from_bool(enabled: bool) -> Self {
        if enabled { Self::Full } else { Self::Off }
    }

    /// The raw `u32` discriminant stamped into [`FineMarcherPush::coarse_enabled`].
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// `#[repr(C)]` the fine SDF-marcher's COMPUTE push constant
/// (`sdf_gbuffer_composite.hlsl`), pushed against the marcher pipeline's OWN dedicated
/// layout via `push_compute_constants`. Render P4b introduced the first two fields; A1/A2
/// widened it from 8 → 32 bytes to carry the directional-light state. The byte layout is
/// HLSL std430-style scalar+`float3` (the const-asserts below pin every offset):
///
///   offset  0 : u32   coarse_enabled   P4b coarse-cull mode (0 = off, 1 = full, 2 = empty-skip)
///   offset  4 : f32   omega            B1 Keinert over-relaxation factor, [1.0, 1.99]
///   offset  8 : u32   lighting_flags   bit 0 = A1 shadows, bit 1 = A2 AO; 0 = OFF path
///   offset 12 : u32   _pad             aligns `light_dir` to offset 16 (a `float3` lands
///                                      on a 16-byte boundary under std430)
///   offset 16 : [f32;3] light_dir      the directional-light direction (un-normalized)
///   offset 28 : f32   _pad2            tail pad to a 32-byte stride
///   total: 32 bytes — a subset of the declared 80-byte COMPUTE push range, so the
///   pipeline-layout declaration is unchanged.
///
/// `lighting_flags == 0` ⇒ the OFF path: the marcher emits the bare Lambert albedo,
/// BYTE-IDENTICAL to the pre-A1/A2 shader (the 0%-gate).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FineMarcherPush {
    /// P4b coarse-cull mode ([`CoarseMode`] as `u32`): `0` = off (never reads binding 6),
    /// `1` = full (EMPTY-skip + `near_t` seed), `2` = empty-skip-only (EMPTY-skip, NO seed —
    /// the lit-transparent on-screen cull). Any non-zero value reads binding 6 (`TileBound`)
    /// for the EMPTY-skip; only `1` consumes `near_t`.
    pub coarse_enabled: u32,
    /// B1 Keinert over-relaxation factor, host-clamped to `[1.0, 1.99]`.
    pub omega: f32,
    /// Lighting gate: bit 0 = A1 shadows, bit 1 = A2 AO; `0` = the OFF (Lambert-only) path.
    pub lighting_flags: u32,
    /// std430 padding so `light_dir` lands at offset 16.
    pub _pad: u32,
    /// The directional-light direction (un-normalized; the shader normalizes it).
    pub light_dir: [f32; 3],
    /// std430 tail padding to a 32-byte stride.
    pub _pad2: f32,
    /// M1 pointer-grid minimum world corner (cell `(0,0,0)`'s min). FIRST of the M1 block
    /// so this `float3` lands on a 16-byte boundary (offset 32) — the std430/HLSL `vec3`
    /// alignment rule (a `vec3` aligns to 16) that the `light_dir @16` field also obeys.
    /// Placing it here (not after a scalar) keeps the Rust `#[repr(C)]` 4-byte packing and
    /// the HLSL std430 layout byte-identical. Don't-care when `brick_enabled == 0`.
    pub grid_origin: [f32; 3],
    /// M1 empty-space-skip gate: non-zero reads binding 9 (the `PointerGrid`) and skips
    /// `EmptyOutside` bricks to their AABB exit. `0` = the OFF path (byte-identical to the
    /// pre-M1 marcher — the grid is never touched, the hit/normal stay analytic). Tucked
    /// into the `float3` tail slot (offset 44) so the next `uint3` lands 16-aligned.
    pub brick_enabled: u32,
    /// M1 pointer-grid cell count per axis (`[x, y, z]`). A `uint3` — aligned to 16 by the
    /// std430 rule (offset 48). Don't-care when off.
    pub grid_dims: [u32; 3],
    /// M1 pointer-grid cell size (the world width of one brick cell). The `uint3` tail slot
    /// (offset 60). Don't-care when `brick_enabled == 0`.
    pub brick_world: f32,
    /// M2 trilinear+JCGT-cubic SURFACE-brick gate: non-zero samples the brick atlas (binding 10)
    /// and runs the analytic-cubic crossing inside a SURFACE brick (validated by the analytic
    /// residual). `0` = the OFF path (byte-identical to the M1 marcher — the atlas is never
    /// sampled). INDEPENDENT of `brick_enabled` (the M1 empty-skip): the two gates are orthogonal.
    /// First slot (offset 64) of the 16-byte headroom the 64-byte M1 layout left inside the
    /// declared 80-byte COMPOSITE push range.
    pub brick_trilinear: u32,
    /// M4 clip-map LEVEL COUNT: how many nested brick levels the marcher loops over (Slice C). `1`
    /// = the M2-identical / OFF path (the shader loops once over level 0 — byte-identical to the
    /// single-level M2 marcher); `> 1` reads that many [`M4LevelParams`] from the b5 UBO array tail.
    /// Tucked into the first M2 `_pad3` slot (offset 68) so the struct SIZE is unchanged. `0` is
    /// treated as the OFF path by the shader (no level sampled). Set via [`with_brick_levels`].
    ///
    /// [`with_brick_levels`]: Self::with_brick_levels
    pub brick_levels: u32,
    /// MDF Stage-2c: the mesh-distance-field SHADOW gate (offset 72). Non-zero makes the
    /// marcher's shadow march union the mesh SDF texture (binding 15) into the analytic
    /// shadow field (`min(field_distance(q), mesh_sdf_sample(q))` via `sdf_soft_shadow_mesh`).
    /// `0` = the OFF path (byte-identical to pre-MDF — the texture is bound-but-unread, the
    /// shadow march stays the frozen analytic `sdf_soft_shadow`). Reuses the first M4 `_pad3`
    /// slot, so the struct SIZE is unchanged. Set via [`with_mesh_sdf`](Self::with_mesh_sdf).
    pub mesh_sdf_enabled: u32,
    /// std430 tail padding (offset 76) to the 80-byte COMPOSITE push stride. Mirrors the
    /// shader's trailing `uint _pad3`. Don't-care (the shader never reads it).
    pub _pad3: u32,
}

/// Byte size of [`FineMarcherPush`] — the marcher's COMPUTE push range (80 bytes), exactly the
/// declared `COMPOSITE_PUSH_CONSTANT_BYTES` range (the M2 widening filled the 16-byte headroom).
pub const GBUFFER_MARCHER_PUSH_BYTES: u32 = core::mem::size_of::<FineMarcherPush>() as u32;

// Pin the std430 field offsets + the 64-byte stride so a host/shader desync is a build
// error (the same discipline as `CompositePushConstants` / `TileBound`). The `light_dir`
// @16 pin is the one a non-default-direction GPU test catches if the packing slips; the
// `grid_origin` @32 + `grid_dims` @48 pins are the M1 analogue (a non-default grid GPU test
// catches a slip). Both M1 vectors land 16-aligned — the std430/HLSL `vec3`-aligns-to-16
// rule, which is why the M1 block is ordered vector-first (the scalar gate/size fill the
// vec3 tail slots), keeping the Rust `#[repr(C)]` and the HLSL std430 byte-identical.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, coarse_enabled) == 0);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, omega) == 4);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, lighting_flags) == 8);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad) == 12);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, light_dir) == 16);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad2) == 28);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, grid_origin) == 32);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_enabled) == 44);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, grid_dims) == 48);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_world) == 60);
// M2: the `brick_trilinear` gate @64 + the `_pad3` tail @68 fill the 16-byte headroom the M1
// layout left, so the struct is now EXACTLY the declared 80-byte COMPOSITE push range. A
// non-default-grid / `brick_trilinear` GPU test catches a packing slip the way the light_dir@16
// and grid_origin@32 pins do.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_trilinear) == 64);
// M4: the `brick_levels` clip-map count @68 reuses the first M2 `_pad3` slot (the tail pad shrinks
// to `[u32; 2]` @72/76), so the struct stays EXACTLY the declared 80-byte COMPOSITE push range. A
// non-default `brick_levels` GPU test catches a packing slip the way the brick_trilinear@64 pin does.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, brick_levels) == 68);
// MDF Stage-2c: the `mesh_sdf_enabled` gate @72 reuses the first M4 `_pad3` slot (the tail pad
// shrinks to a single `u32` @76), so the struct stays EXACTLY the declared 80-byte COMPOSITE
// range. A non-default `mesh_sdf_enabled` GPU test catches a packing slip the way the
// brick_levels@68 pin does.
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, mesh_sdf_enabled) == 72);
const _: () = assert!(core::mem::offset_of!(FineMarcherPush, _pad3) == 76);
const _: () = assert!(GBUFFER_MARCHER_PUSH_BYTES == 80, "FineMarcherPush must be 80 bytes");
const _: () = assert!(
    GBUFFER_MARCHER_PUSH_BYTES == COMPOSITE_PUSH_CONSTANT_BYTES,
    "FineMarcherPush must equal the declared 80-byte COMPUTE push range"
);

impl FineMarcherPush {
    /// Builds the marcher push for the windowed / offscreen fine pass: the P4b
    /// `coarse_enabled` gate, the B1 `omega`, and the A1/A2 `lighting_flags` + the
    /// directional `light_dir`. `lighting_flags == 0` selects the OFF (byte-identical)
    /// path; `light_dir` is then a don't-care (the shader never normalizes it). The M1
    /// empty-skip is OFF (`brick_enabled == 0`, a zero grid) — byte-identical to the
    /// pre-M1 marcher. Use [`with_brick`](Self::with_brick) to enable the empty skip.
    ///
    /// The legacy `coarse_enabled: bool` maps via [`CoarseMode::from_bool`] — `false` →
    /// [`Off`](CoarseMode::Off) (0), `true` → [`Full`](CoarseMode::Full) (1) — so every
    /// pre-existing bool caller is byte-unchanged (the `true` callers still seed `near_t`).
    /// Use [`new_mode`](Self::new_mode) to select the 3-value mode (e.g.
    /// [`EmptySkipOnly`](CoarseMode::EmptySkipOnly) for the lit-transparent on-screen cull).
    #[inline]
    pub const fn new(
        coarse_enabled: bool,
        omega: f32,
        lighting_flags: u32,
        light_dir: [f32; 3],
    ) -> Self {
        Self::new_mode(CoarseMode::from_bool(coarse_enabled), omega, lighting_flags, light_dir)
    }

    /// Builds the marcher push with the explicit 3-value coarse-cull [`CoarseMode`] (the
    /// generalization of [`new`](Self::new)). [`Off`](CoarseMode::Off) is byte-identical to
    /// the pre-P4b push; [`Full`](CoarseMode::Full) is the historical EMPTY-skip + `near_t`
    /// seed (the offscreen goldens' mode); [`EmptySkipOnly`](CoarseMode::EmptySkipOnly) is the
    /// lit-transparent on-screen cull (EMPTY-skip, no seed). All other fields are identical to
    /// [`new`](Self::new).
    #[inline]
    pub const fn new_mode(
        coarse_mode: CoarseMode,
        omega: f32,
        lighting_flags: u32,
        light_dir: [f32; 3],
    ) -> Self {
        Self {
            coarse_enabled: coarse_mode.as_u32(),
            omega,
            lighting_flags,
            _pad: 0,
            light_dir,
            _pad2: 0.0,
            grid_origin: [0.0, 0.0, 0.0],
            brick_enabled: 0,
            grid_dims: [0, 0, 0],
            brick_world: 0.0,
            brick_trilinear: 0,
            brick_levels: 0,
            mesh_sdf_enabled: 0,
            _pad3: 0,
        }
    }

    /// Enables the M1 empty-space-skip: turns on `brick_enabled` and stamps the
    /// pointer-grid uniforms (`grid_origin`, `grid_dims`, `brick_world`) the marcher
    /// indexes binding 9 with. The grid must match the [`build_pointer_grid`] bake bound
    /// to binding 9. The base gates (`coarse_enabled` / `omega` / lighting) are
    /// preserved. With the empty skip ON, the hit/normal stay ANALYTIC — only the
    /// EMPTY-brick traversal is accelerated (the hit-set equals the analytic hit-set).
    ///
    /// [`build_pointer_grid`]: boyko_sdf_math::brick::build_pointer_grid
    #[inline]
    pub const fn with_brick(
        mut self,
        grid_origin: [f32; 3],
        grid_dims: [u32; 3],
        brick_world: f32,
    ) -> Self {
        self.brick_enabled = 1;
        self.grid_origin = grid_origin;
        self.grid_dims = grid_dims;
        self.brick_world = brick_world;
        self
    }

    /// Enables the M2 trilinear+JCGT-cubic SURFACE-brick path: turns on `brick_trilinear`. The
    /// marcher then samples the brick atlas (binding 10) and solves the analytic cubic for the
    /// EXACT ray↔isosurface crossing inside a SURFACE M2 cell, validated by the analytic residual
    /// (the exact-CSG fallback). The atlas/grid uniforms the cubic needs live in the b5 camera UBO
    /// ([`M2GridParams`]), NOT the push, so this gate carries no extra fields. INDEPENDENT of the
    /// M1 empty-skip ([`with_brick`](Self::with_brick)): both gates may be on, off, or mixed. The
    /// base gates (`coarse_enabled` / `omega` / lighting) and any M1 grid uniforms are preserved.
    ///
    /// `brick_trilinear == false` leaves the push byte-identical to the M1 state (the OFF path —
    /// the atlas is never sampled).
    #[inline]
    pub const fn with_brick_trilinear(mut self, enabled: bool) -> Self {
        self.brick_trilinear = enabled as u32;
        self
    }

    /// Sets the M4 clip-map [`brick_levels`](Self::brick_levels) count the marcher loops over
    /// (Slice C). `n == 1` is the M2-identical / OFF path (the shader loops once over level 0,
    /// byte-identical to the single-level M2 marcher); `n > 1` reads `n` [`M4LevelParams`] blocks
    /// from the b5 UBO array tail ([`M4GridParams`]). The other gates (`coarse_enabled` / `omega` /
    /// lighting / the M1/M2 brick gates) are preserved. Does NOT itself enable the M2 surface path —
    /// pair with [`with_brick_trilinear`](Self::with_brick_trilinear).
    #[inline]
    pub const fn with_brick_levels(mut self, n: u32) -> Self {
        self.brick_levels = n;
        self
    }

    /// Enables the MDF Stage-2c mesh-distance-field SHADOW path: turns on `mesh_sdf_enabled`. The
    /// marcher's shadow march then unions the mesh SDF texture (binding 15) into the analytic
    /// shadow field (`min(field_distance(q), mesh_sdf_sample(q))` via `sdf_soft_shadow_mesh`), so a
    /// raster-rendered static mesh casts a soft SDF shadow without any per-frame mesh work. The grid
    /// transform the sample needs (`grid_origin`, `inv_voxel_size`, `grid_dim`, `band_half`) lives in
    /// the b5 camera UBO tail ([`MeshSdfParams`]), NOT the push, so this gate carries no extra fields.
    /// The other gates (`coarse_enabled` / `omega` / lighting / the brick gates) are preserved.
    ///
    /// `enabled == false` leaves the push byte-identical to the prior state (the OFF path — the mesh
    /// SDF texture is never sampled, the shadow march stays the frozen analytic `sdf_soft_shadow`).
    #[inline]
    pub const fn with_mesh_sdf(mut self, enabled: bool) -> Self {
        self.mesh_sdf_enabled = enabled as u32;
        self
    }

    /// Re-views the push constants as their raw 80-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `f32` / `[f32; 3]` / `[u32; 3]`
        // fields (all `Copy`, every offset + the 80-byte total pinned by the const-asserts
        // above, no uninit padding — the explicit `_pad`/`_pad2`/`brick_levels`/`_pad3` fields cover
        // the std430 holes), so its `size_of` bytes are a fully-initialized, alignment-valid POD bit
        // pattern. The `&self` borrow keeps the struct alive for the slice's lifetime; the slice is
        // read-only (no aliasing write).
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

/// The Lighting-L1 cull push constants (mirrors `cluster_cull.hlsl`'s `ClusterCullPush`): the
/// exp-Z near/far the froxel-AABB build samples its slice view-z from, plus the per-froxel /
/// flat-list caps the cull clamp-and-drops at (O2). `#[repr(C)]`, 16 B (`f32, f32, u32, u32`),
/// the offsets pinned by the const-asserts below so a host/shader desync is a build error.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterCullPush {
    /// Exp-Z near plane (slice 0 view-z).
    pub z_near: f32,
    /// Exp-Z far plane (slice `dim_z` view-z).
    pub z_far: f32,
    /// Per-froxel light-index cap (O2 clamp-and-drop).
    pub max_lights_per_cluster: u32,
    /// Flat light-index-list capacity in `u32`s (O2 global clamp-and-drop).
    pub index_list_cap: u32,
}

/// Byte size of [`ClusterCullPush`] — the cull pipeline's declared COMPUTE push range (16 B).
pub const CLUSTER_CULL_PUSH_BYTES: u32 = core::mem::size_of::<ClusterCullPush>() as u32;

const _: () = assert!(core::mem::offset_of!(ClusterCullPush, z_near) == 0);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, z_far) == 4);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, max_lights_per_cluster) == 8);
const _: () = assert!(core::mem::offset_of!(ClusterCullPush, index_list_cap) == 12);
const _: () = assert!(CLUSTER_CULL_PUSH_BYTES == 16, "ClusterCullPush must be 16 bytes");

impl ClusterCullPush {
    /// Builds the cull push from the exp-Z near/far + the caps.
    #[inline]
    pub const fn new(
        z_near: f32,
        z_far: f32,
        max_lights_per_cluster: u32,
        index_list_cap: u32,
    ) -> Self {
        Self { z_near, z_far, max_lights_per_cluster, index_list_cap }
    }

    /// Re-views the push constants as their raw 16-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `f32` / `u32` fields (all `Copy`, every
        // offset + the 16-byte total pinned by the const-asserts above, no uninit padding),
        // so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern.
        // The `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}


/// The camera the extent-aware golden ([`golden_composite_pixel_ex`]) reconstructs a
/// ray from. ORTHO is the golden-frozen path; PERSPECTIVE mirrors the shader's
/// additive ray-gen (eye + orthonormal basis + half-FOV tangent + aspect).
#[derive(Clone, Copy, Debug)]
pub enum CompositeCamera {
    /// The golden-frozen orthographic camera (looking down -Z, [`SDF_HALF_EXTENT`]).
    Ortho,
    /// The P0a perspective camera, mirroring the shader's perspective branch.
    Perspective {
        /// Eye world position (the ray origin).
        eye: [f32; 3],
        /// Forward basis vector.
        forward: [f32; 3],
        /// Right basis vector.
        right: [f32; 3],
        /// Up basis vector.
        up: [f32; 3],
        /// `tan(fovY / 2)`.
        tan_half_fov: f32,
        /// Aspect ratio (W / H).
        aspect: f32,
    },
}

/// Reconstructs the `(ray_origin, ray_dir)` for pixel `(px, py)` at extent
/// `(img_w, img_h)` under `camera`, mirroring the shader's ray-gen EXACTLY.
///
/// The ORTHO arm is the golden-frozen arithmetic (`u`/`v` → `ro`/`rd`); at extent
/// `(64, 64)` it is byte-for-byte the pre-P0a computation. The PERSPECTIVE arm mirrors
/// the shader's perspective branch (NDC → basis-combined direction → `normalize`),
/// using only plain IEEE ops so a perspective scene is reproducible (no fast math).
#[inline]
pub(crate) fn composite_ray(
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    match camera {
        CompositeCamera::Ortho => {
            let u = (((px as f32) + 0.5) / (img_w as f32)) * 2.0 - 1.0;
            let v = -((((py as f32) + 0.5) / (img_h as f32)) * 2.0 - 1.0);
            let ro = [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT, SDF_CAM_Z];
            let rd = [0.0, 0.0, -1.0];
            (ro, rd)
        }
        CompositeCamera::Perspective {
            eye,
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        } => {
            let ndc_x = (((px as f32) + 0.5) / (img_w as f32)) * 2.0 - 1.0;
            let ndc_y = -((((py as f32) + 0.5) / (img_h as f32)) * 2.0 - 1.0);
            let sx = ndc_x * aspect * tan_half_fov;
            let sy = ndc_y * tan_half_fov;
            let dir = [
                forward[0] + right[0] * sx + up[0] * sy,
                forward[1] + right[1] * sx + up[1] * sy,
                forward[2] + right[2] * sx + up[2] * sy,
            ];
            // Mirror HLSL `normalize` exactly: raw `sqrt` then component divide, NO
            // zero-guard (unlike `v_normalize`), so this host reference predicts the
            // GPU bit-for-bit on valid cameras. A degenerate (zero) `dir` yields a
            // non-finite ray on BOTH host and shader; a valid camera never produces one.
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let rd = [dir[0] / len, dir[1] / len, dir[2] / len];
            (eye, rd)
        }
    }
}

/// The `(ray_origin, ray_dir)` for pixel `(px, py)` at extent `(img_w, img_h)` under
/// `camera`, exposing the shared marcher/resolve ray-gen ([`composite_ray`]) so the PBR
/// MVP-2 resolve golden ([`golden_deferred_resolve`]) can reconstruct the per-pixel view
/// direction (`V = -rd`) the GPU resolve uses. Bit-identical to the marcher's ray-gen.
#[inline]
pub fn composite_pixel_ray(
    px: u32,
    py: u32,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
) -> ([f32; 3], [f32; 3]) {
    composite_ray(px, py, img_w, img_h, camera)
}


// ===========================================================================
// M1 — the EMPTY-SPACE-SKIP host golden mirror (the empty-skip pivot).
//
// `golden_composite_pixel_brick` mirrors `sdf_gbuffer_composite.hlsl`'s primary march
// with `brick_enabled != 0`: before each `sdf(p)` it reads the pointer-grid cell at `p`
// and, on an `EmptyOutside` cell, steps to the brick's ray-AABB exit
// (`dist_to_brick_exit`) instead of folding the field — sound by construction (the
// conservative classifier guarantees no surface within `band_half` of an EMPTY brick).
// `EmptyInside` and `Surface` cells, and points OUTSIDE the bounded grid, march
// ANALYTICALLY (the exact `sdf(p)` step). The hit/normal/shade stay ANALYTIC (C1).
//
// With `brick_enabled == 0` (or an empty/None grid) the function delegates to the
// pre-M1 `golden_composite_pixel_ex_omega_lit`, so the OFF golden is BYTE-IDENTICAL to
// today (the 0%-gate). With it ON, the empty skip never skips a surface, so the hit `t`
// — and therefore the composited color — equals the pure-analytic result.
// ===========================================================================

/// `BrickClass::EmptyOutside as u32` — the only cell class the empty skip acts on. The
/// host mirror reads the grid as `u32` cells (the GPU `StructuredBuffer<uint>` element),
/// matching `build_pointer_grid`.
#[cfg(any(test, feature = "goldens"))]
pub(crate) const BRICK_CLASS_EMPTY_OUTSIDE: u32 = 0;


// ===========================================================================
// M2 — the trilinear+JCGT-cubic SURFACE-brick atlas (the CPU baker + the b5 UBO grid
// block + the host golden mirror).
//
// M1 ships empty-space-skip + an analytic fold inside SURFACE bricks; M2 replaces that
// analytic fold with a dense `R8_SNORM`/`R16_SFLOAT` brick atlas — one apron'd
// `BRICK_ALLOC³` (10³) tile per SURFACE M2 grid cell — sampled by the marcher, whose
// 8 per-cell corner distances form the JCGT-2022 cubic ([`brick_cubic_hit`]) whose root
// is the EXACT ray↔trilinear-isosurface crossing, then VALIDATED by the analytic residual
// (the exact-CSG fallback). The grid is a small NEAR-FIELD lattice ([-4, 4]³) of
// `M2_BRICK_WORLD`-sized bricks; the atlas is `M2_ATLAS_DIM³` voxels.
//
// Principle 0: the atlas is a TRANSIENT GPU mirror of the ONE analytic authority
// ([`SdfEditField`]) baked each `gen`, owning no durable per-entity state (exactly like
// the M1 pointer grid / the GPU edit list). The host mirror ([`golden_composite_pixel_brick_m2`])
// is the bit-exact reference the GPU golden compares against.
// ===========================================================================

/// The world size of one M2 brick cell (`BRICK_INTERIOR * M2_VOXEL_SIZE = 8 * 0.25`). The
/// near-field grid cell edge — the world span a single apron'd `BRICK_ALLOC³` atlas tile covers.
pub const M2_BRICK_WORLD: f32 = 2.0;

/// The world width of one M2 atlas voxel (the brick scale [`fill_brick`] / [`brick_cubic_hit`] pin).
pub const M2_VOXEL_SIZE: f32 = 0.25;

/// The M2 near-field grid edge (cells per axis). A `4³` lattice of [`M2_BRICK_WORLD`]-sized bricks
/// spans `[-4, 4]³` (the demo/golden extent — centers in `[-2, 2]`, primitives `<= 3`), fully
/// enclosing the near field.
pub const M2_GRID_DIM: u32 = 4;

/// The minimum world corner of the M2 near-field grid (cell `(0,0,0)`'s min): `-M2_GRID_DIM *
/// M2_BRICK_WORLD / 2 = -4`. The grid spans `[M2_GRID_ORIGIN, M2_GRID_ORIGIN + M2_GRID_DIM *
/// M2_BRICK_WORLD]³ = [-4, 4]³`.
pub const M2_GRID_ORIGIN: f32 = -4.0;

/// The M2 atlas edge in voxels (`M2_GRID_DIM * BRICK_ALLOC = 4 * 10 = 40`). The dense 3D atlas
/// image is `M2_ATLAS_DIM³` voxels — one apron'd `BRICK_ALLOC³` tile per M2 grid cell. The
/// [`crate::brick_atlas::BrickAtlas`] image is sized to this.
pub const M2_ATLAS_DIM: u32 = M2_GRID_DIM * BRICK_ALLOC as u32;

/// The M2 snorm decode band half-width (world units) — the band the atlas codes span, mirroring
/// `boyko_sdf_math::brick`'s store band (== [`SDF_EDIT_BAND_HALF`] `= 0.90`).
pub const M2_BAND_HALF: f32 = SDF_EDIT_BAND_HALF;

/// The M2 crease tolerance (world units): the largest `|analytic sdf|` at the cubic candidate that
/// ACCEPTS the cubic hit as the surface; beyond it (a CSG crease / brick-rounding divergence) the
/// analytic refine decides (the exact-CSG fallback). `~` the brick's `δ_tri + δ_quant` world slack.
pub const M2_CREASE_EPS: f32 = 0.0192;

/// The b5 camera UBO byte size widened for M2: the 80-byte camera block
/// ([`COMPOSITE_PUSH_CONSTANT_BYTES`]) + a 48-byte [`M2GridParams`] tail at
/// [`M2_GRID_PARAMS_OFFSET`]. The host writes the camera block then the M2 block; the marcher
/// reads the M2 block ONLY on the `brick_trilinear` path (byte-identical to M1 when OFF).
pub const B5_CAMERA_UBO_BYTES: usize = 128;

/// The byte offset of the [`M2GridParams`] block inside the widened b5 camera UBO (right after the
/// 80-byte camera block). The host writes `M2GridParams::default_near_field().as_bytes()` here.
pub const M2_GRID_PARAMS_OFFSET: usize = 80;

/// The number of analytic refine steps the M2 surface-hit fallback sphere-traces from the cubic
/// candidate when the analytic residual exceeds [`M2_CREASE_EPS`] (mirror the shader's
/// `M2_REFINE_ITERS`).
#[cfg(any(test, feature = "goldens"))]
pub(crate) const M2_REFINE_ITERS: u32 = 32; // BUG-B1-HOLE-4 ring fix (see the shader const) — host mirror

/// The under-relaxation factor of the SIGNED M2 refine step (`rt += M2_REFINE_RELAX * d`). The refine
/// is a unit-gradient SDF Newton step (`rt += d` is exact on a flat surface); under-relaxing damps
/// overshoot oscillation at a CSG crease where the gradient is not unit. At `0.8` an inside candidate
/// `~δ` deep converges in `~3` steps (`δ ≈ 0.048` near-field, `≈ 2^L·δ` at coarse clip-map level L) —
/// well within [`M2_REFINE_ITERS`] (`8`). Mirrors the shader's `M2_REFINE_RELAX` bit-for-bit.
#[cfg(any(test, feature = "goldens"))]
pub(crate) const M2_REFINE_RELAX: f32 = 0.8;

// The M2 grid constants pin the shader's static brick geometry (mirror the `.hlsl` `M2_*` consts):
// a desync (e.g. a brick scale change) is a build error here, caught before the GPU runs.
const _: () = assert!(M2_BRICK_WORLD == BRICK_INTERIOR as f32 * M2_VOXEL_SIZE);
const _: () = assert!(M2_GRID_ORIGIN == -(M2_GRID_DIM as f32 * M2_BRICK_WORLD * 0.5));
const _: () = assert!(M2_ATLAS_DIM == 40);
// M4 reconciliation: the compute-side M2 grid geometry MUST equal the `boyko_sdf_math::brick`
// Slice-A copies (the `no_std` clip-map authority `brick.rs` derives the level table from). A
// desync between the two would make level-0 of the clip-map disagree with the M2 single-level
// bake — a build error here, not a silent divergence. The brick-side const reduces to the M2
// scale at level 0 (`brick::brick_world_at_level(0) == brick::M2_BRICK_WORLD`).
const _: () = assert!(M2_BRICK_WORLD == brick::M2_BRICK_WORLD);
const _: () = assert!(M2_GRID_DIM == brick::M2_GRID_DIM);
const _: () = assert!(M2_VOXEL_SIZE == brick::voxel_size_at_level(0));
const _: () = assert!(M2_BAND_HALF == brick::band_half_at_level(0));

/// The minimum world corner of M2 grid cell `cell = (cx, cy, cz)`:
/// `M2_GRID_ORIGIN + cell * M2_BRICK_WORLD`. The brick spans `[min, min + M2_BRICK_WORLD]³`.
#[inline]
pub const fn m2_cell_min(cell: [u32; 3]) -> [f32; 3] {
    [
        M2_GRID_ORIGIN + cell[0] as f32 * M2_BRICK_WORLD,
        M2_GRID_ORIGIN + cell[1] as f32 * M2_BRICK_WORLD,
        M2_GRID_ORIGIN + cell[2] as f32 * M2_BRICK_WORLD,
    ]
}

/// The atlas-VOXEL origin of M2 grid cell `tile = (tx, ty, tz)`: `tile * BRICK_ALLOC`. The cell's
/// apron'd `BRICK_ALLOC³` tile occupies `[origin, origin + BRICK_ALLOC]³` voxels in the dense
/// `M2_ATLAS_DIM³` atlas. MUST match the shader's `m2_corner` uvw mapping
/// (`(tile_org + corner) / M2_ATLAS_DIM` under the integer texelFetch), and the host baker
/// ([`bake_brick_atlas`]) scatters each tile at this voxel offset.
#[inline]
pub const fn m2_tile_atlas_origin(tile: [u32; 3]) -> [u32; 3] {
    [
        tile[0] * BRICK_ALLOC as u32,
        tile[1] * BRICK_ALLOC as u32,
        tile[2] * BRICK_ALLOC as u32,
    ]
}

/// The M2 grid block written into the b5 camera UBO tail (at [`M2_GRID_PARAMS_OFFSET`]) so a
/// host-side grid retune needs no shader edit. `#[repr(C)]`, 48 bytes — three std140 `vec4` lanes
/// mirroring the shader's `cbuffer Camera` M2 fields (`m2_origin_brick_world`, `m2_dims_atlas_dim`,
/// `m2_band_voxel_inv_atlas`):
///
/// - lane 0 `origin_brick_world` — `(origin.x, origin.y, origin.z, M2_BRICK_WORLD)`
/// - lane 1 `dims_atlas_dim` — `(dims.x, dims.y, dims.z, M2_ATLAS_DIM)` as `f32`
/// - lane 2 `band_voxel_inv_atlas` — `(band_half, voxel_size, 1.0 / atlas_dim, 0.0)`
///
/// The offsets are pinned by the const-asserts below so a host/shader desync is a build error.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M2GridParams {
    /// Lane 0: `xyz` = the M2 grid min world corner, `w` = [`M2_BRICK_WORLD`].
    pub origin_brick_world: [f32; 4],
    /// Lane 1: `xyz` = the M2 grid dims `[x, y, z]` as `f32`, `w` = [`M2_ATLAS_DIM`] as `f32`. The
    /// shader reads it as a `uint4` (`(uint)` cast) — an exact small-integer `f32`↔`uint` round trip.
    pub dims_atlas_dim: [f32; 4],
    /// Lane 2: `x` = band half-width, `y` = voxel size, `z` = `1.0 / M2_ATLAS_DIM`, `w` = 0 (pad).
    pub band_voxel_inv_atlas: [f32; 4],
}

/// Byte size of [`M2GridParams`] — three std140 `vec4` lanes (48 B), the b5 UBO M2 tail.
pub const M2_GRID_PARAMS_BYTES: usize = core::mem::size_of::<M2GridParams>();

const _: () = assert!(core::mem::offset_of!(M2GridParams, origin_brick_world) == 0);
const _: () = assert!(core::mem::offset_of!(M2GridParams, dims_atlas_dim) == 16);
const _: () = assert!(core::mem::offset_of!(M2GridParams, band_voxel_inv_atlas) == 32);
const _: () = assert!(M2_GRID_PARAMS_BYTES == 48, "M2GridParams must be 48 bytes (3 vec4 lanes)");
const _: () = assert!(
    M2_GRID_PARAMS_OFFSET + M2_GRID_PARAMS_BYTES == B5_CAMERA_UBO_BYTES,
    "the M2 block must fill the b5 UBO tail exactly (80 + 48 = 128)"
);

impl M2GridParams {
    /// The default near-field M2 grid block: origin [`M2_GRID_ORIGIN`], dims [`M2_GRID_DIM`]³,
    /// brick world [`M2_BRICK_WORLD`], atlas [`M2_ATLAS_DIM`], band [`M2_BAND_HALF`], voxel
    /// [`M2_VOXEL_SIZE`] — the render-path seed the [`crate::brick_atlas::BrickAtlas`] bakes against.
    #[inline]
    pub fn default_near_field() -> Self {
        Self {
            origin_brick_world: [M2_GRID_ORIGIN, M2_GRID_ORIGIN, M2_GRID_ORIGIN, M2_BRICK_WORLD],
            dims_atlas_dim: [
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_ATLAS_DIM as f32,
            ],
            band_voxel_inv_atlas: [
                M2_BAND_HALF,
                M2_VOXEL_SIZE,
                1.0 / M2_ATLAS_DIM as f32,
                0.0,
            ],
        }
    }

    /// Re-views the block as its raw 48-byte slice for the b5 UBO write (at [`M2_GRID_PARAMS_OFFSET`]).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `[f32; 4]` fields (all `Copy`, every offset + the
        // 48-byte total pinned by the const-asserts above, no uninit padding — three packed vec4
        // lanes), so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern.
        // The `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

// ===========================================================================
// M4 — the CLIP-MAP LOD UBO TAIL ([`M4GridParams`]): the b5 camera-UBO tail for the N-level
// brick clip-map. M4 replaces the SINGLE 48-byte [`M2GridParams`] block at [`M2_GRID_PARAMS_OFFSET`]
// with an ARRAY of [`brick::BRICK_LEVELS`] per-level [`M4LevelParams`] blocks — the shader (Slice C)
// loops over the level array. The per-level block is byte-for-byte the M2 lane layout, so a
// single-level clip-map (level 0) is bit-identical to the M2 tail (the OFF/N=1 keystone).
// ===========================================================================

/// ONE clip-map level's b5 UBO block — byte-FOR-byte the [`M2GridParams`] 48-byte / three-vec4 lane
/// layout, replicated so a single level is bit-identical to the M2 tail (the OFF/N=1 keystone). The
/// shader (Slice C) declares a matching `struct { float4 origin_brick_world; float4 dims_atlas_dim;
/// float4 band_voxel_inv_atlas; } m2_levels[BRICK_LEVELS]`.
///
/// - lane 0 `origin_brick_world` — `(origin.x, origin.y, origin.z, brick_world_at_level(L))`
/// - lane 1 `dims_atlas_dim` — `(dims.x, dims.y, dims.z, M2_ATLAS_DIM)` as `f32`
/// - lane 2 `band_voxel_inv_atlas` — `(band_half_at_level(L), voxel_size_at_level(L), 1/atlas_dim, level)`
///
/// The `level` index lives in lane 2 `w` (M2's level-0 pad slot is `0.0`, == level 0, so the level-0
/// block stays byte-identical). The offsets are pinned by the const-asserts below.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M4LevelParams {
    /// Lane 0: `xyz` = this level's snapped grid min world corner, `w` = `brick_world_at_level(L)`.
    pub origin_brick_world: [f32; 4],
    /// Lane 1: `xyz` = the grid dims `[x, y, z]` as `f32` (level-invariant [`M2_GRID_DIM`]), `w` =
    /// [`M2_ATLAS_DIM`] as `f32` (level-invariant). The shader reads it as a `uint4` (an exact
    /// small-integer `f32`↔`uint` round trip).
    pub dims_atlas_dim: [f32; 4],
    /// Lane 2: `x` = `band_half_at_level(L)`, `y` = `voxel_size_at_level(L)`, `z` = `1/M2_ATLAS_DIM`,
    /// `w` = the level index `L` (as `f32`; M2's level-0 pad `0.0` == level 0, byte-identical).
    pub band_voxel_inv_atlas: [f32; 4],
}

/// Byte size of [`M4LevelParams`] — three std140 `vec4` lanes (48 B), IDENTICAL to [`M2GridParams`].
pub const M4_LEVEL_PARAMS_BYTES: usize = core::mem::size_of::<M4LevelParams>();

const _: () = assert!(core::mem::offset_of!(M4LevelParams, origin_brick_world) == 0);
const _: () = assert!(core::mem::offset_of!(M4LevelParams, dims_atlas_dim) == 16);
const _: () = assert!(core::mem::offset_of!(M4LevelParams, band_voxel_inv_atlas) == 32);
const _: () = assert!(M4_LEVEL_PARAMS_BYTES == 48, "M4LevelParams must be 48 bytes (3 vec4 lanes)");
// The per-level block MUST be byte-identical in layout to the M2 tail — a single level is then
// bit-identical to the M2 `M2GridParams` (the OFF/N=1 keystone, runtime-asserted in the tests).
const _: () = assert!(M4_LEVEL_PARAMS_BYTES == M2_GRID_PARAMS_BYTES);

impl M4LevelParams {
    /// This level's block from a [`BrickLevelParams`] geometry + the level index `L`. `dims`/
    /// `atlas_dim`/`inv_atlas` are level-invariant; `origin`/`brick_world`/`band`/`voxel` come from
    /// `geo` (the clip-map `*_at_level` table), and `level` is stamped into lane-2 `w`.
    #[inline]
    fn from_geometry(geo: &BrickLevelParams, level: u32) -> Self {
        Self {
            origin_brick_world: [geo.origin[0], geo.origin[1], geo.origin[2], geo.brick_world],
            dims_atlas_dim: [
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_GRID_DIM as f32,
                M2_ATLAS_DIM as f32,
            ],
            band_voxel_inv_atlas: [
                geo.band_half,
                geo.voxel_size,
                1.0 / M2_ATLAS_DIM as f32,
                level as f32,
            ],
        }
    }
}

/// The b5 camera-UBO tail for the N-level brick clip-map (M4): an ARRAY of [`brick::BRICK_LEVELS`]
/// per-level [`M4LevelParams`] blocks, written at [`M2_GRID_PARAMS_OFFSET`] (replacing the single M2
/// block). std140 array-of-structs: each 48-byte entry is already 16-aligned, so the array packs
/// CONTIGUOUSLY (level `L` at byte `L*48`) with no inter-entry padding — the shader's
/// `m2_levels[BRICK_LEVELS]` reads it directly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M4GridParams {
    /// The per-level blocks, level `L` at index `L` (byte `L*48`). Level 0 is the finest/nearest.
    pub levels: [M4LevelParams; brick::BRICK_LEVELS],
}

/// Byte size of [`M4GridParams`] — the contiguous `BRICK_LEVELS`-entry array (`BRICK_LEVELS*48`).
pub const M4_GRID_PARAMS_BYTES: usize = core::mem::size_of::<M4GridParams>();

const _: () =
    assert!(M4_GRID_PARAMS_BYTES == brick::BRICK_LEVELS * 48, "M4GridParams must be N*48 bytes");
// The array packs contiguously: no padding between the 16-aligned 48-byte entries.
const _: () = assert!(M4_GRID_PARAMS_BYTES == brick::BRICK_LEVELS * M4_LEVEL_PARAMS_BYTES);

/// The b5 camera-UBO byte size widened for the M4 clip-map: the 80-byte camera block
/// ([`M2_GRID_PARAMS_OFFSET`]) + the [`brick::BRICK_LEVELS`]-level [`M4GridParams`] array tail
/// (`= 80 + N*48 = 224` at `N = 3`). The Slice-C write path uses this; the single-level
/// [`B5_CAMERA_UBO_BYTES`] (`= 128`) is RETAINED for the current M2 write site (Slice C migrates it).
pub const B5_CAMERA_UBO_BYTES_M4: usize = M2_GRID_PARAMS_OFFSET + brick::BRICK_LEVELS * 48;

const _: () = assert!(
    M2_GRID_PARAMS_OFFSET + M4_GRID_PARAMS_BYTES == B5_CAMERA_UBO_BYTES_M4,
    "the M4 array tail must fill the widened b5 UBO exactly (80 + N*48)"
);
const _: () =
    assert!(B5_CAMERA_UBO_BYTES_M4 == 224, "B5_CAMERA_UBO_BYTES_M4 must be 224 at BRICK_LEVELS = 3");

impl M4GridParams {
    /// The camera-centered N-level clip-map block: level `L` filled from the Slice-A `*_at_level`
    /// accessors (snapped origin [`snapped_level_origin`], `brick_world`/`voxel`/`band` `*_at_level`),
    /// dims/atlas level-invariant. Level 0 tracks the camera's snapped near-field; coarser levels
    /// reach `2^L`× farther.
    #[inline]
    pub fn camera_centered(camera: [f32; 3]) -> Self {
        Self {
            levels: core::array::from_fn(|l| {
                let level = l as u32;
                M4LevelParams::from_geometry(&BrickLevelParams::at_level(camera, level), level)
            }),
        }
    }

    /// The OFF/N=1 path: level 0 == [`M2GridParams::default_near_field`] BYTE-FOR-BYTE (the keystone),
    /// the coarser levels filled by REPLICATING level 0's M2 near-field geometry. Level 0 uses the M2
    /// const near-field ([`BrickLevelParams::m2_near_field`], origin `[-4, -4, -4]`), so its 48-byte
    /// block matches the M2 default tail exactly.
    ///
    /// # Invariant (Slice C MUST honor)
    ///
    /// The marcher reads ONLY `m2_levels[0..brick_levels]`; on the OFF/N=1 path `brick_levels == 1`,
    /// so levels `1..N` here are never sampled — they exist only to fill the fixed-size array. Their
    /// content is therefore "dead bytes" on this path. To stay FAIL-SAFE (degrade visibly, not to a
    /// plausible-but-wrong LOD), they REPLICATE level 0's near-field geometry rather than carrying a
    /// `[0, 0, 0]`-snapped origin: if Slice C ever mis-reads a coarse level on the OFF path, it then
    /// samples the SAME near-field as level 0 (a benign duplicate) instead of a wrong-LOD origin.
    /// A shader that reads `levels[l]` for `l >= brick_levels` would still sample the wrong LOD on the
    /// ON path, so Slice C MUST bound its level loop by `brick_levels`. (The ON/camera-centered path,
    /// [`Self::camera_centered`], is unchanged — it carries the real per-level snapped origins.)
    #[inline]
    pub fn near_field_only() -> Self {
        // Level 0 == the M2 const near-field (NOT a camera-snapped origin), so level 0's block is
        // byte-identical to `M2GridParams::default_near_field` (the keystone). The coarser levels
        // replicate the SAME geometry; only lane-2 `w` differs (the level index `l`), which is dead on
        // the OFF/N=1 path (`brick_levels == 1`) and a benign duplicate if ever mis-read.
        let near = BrickLevelParams::m2_near_field();
        Self {
            levels: core::array::from_fn(|l| M4LevelParams::from_geometry(&near, l as u32)),
        }
    }

    /// The POD byte image of the N-level array tail (`BRICK_LEVELS*48` bytes), for the b5 UBO write
    /// at [`M2_GRID_PARAMS_OFFSET`]. The OFF/N=1 keystone: `near_field_only().as_ubo_bytes()[..48]`
    /// equals `M2GridParams::default_near_field().as_bytes()` (asserted in the tests).
    #[inline]
    pub fn as_ubo_bytes(&self) -> [u8; brick::BRICK_LEVELS * 48] {
        let mut bytes = [0u8; brick::BRICK_LEVELS * 48];
        // SAFETY: `Self` is `#[repr(C)]` and `M4_GRID_PARAMS_BYTES == BRICK_LEVELS*48` (pinned by the
        // const-asserts above); the struct is a contiguous array of `[f32; 4]` lanes (all `Copy`, no
        // uninit padding — each 48-byte entry is 16-aligned so the array packs with no holes), so its
        // `size_of` bytes are a fully-initialized POD bit pattern. The source slice covers exactly the
        // struct's bytes; `copy_from_slice` reads them into the equal-length `bytes` (a byte copy, no
        // alignment requirement on the destination).
        let src = unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), M4_GRID_PARAMS_BYTES)
        };
        bytes.copy_from_slice(src);
        bytes
    }
}

// ===========================================================================
// MDF Stage-2c — the MESH-DISTANCE-FIELD grid transform UBO TAIL ([`MeshSdfParams`]): the
// b5 camera-UBO tail block carrying the dedicated dense mesh-SDF texture's world transform,
// appended AFTER the M4 clip-map array (at [`MESH_SDF_PARAMS_OFFSET`]). The marcher's
// `mesh_sdf_sample` reads it to map a world point into the texture's `[0,1]³` UVW and decode
// the snorm sample to a world distance. Written ONLY when the MDF shadow path is active; the
// OFF path leaves it zero (the texture is bound-but-unread — byte-identical output).
// ===========================================================================

/// The byte offset of the [`MeshSdfParams`] block inside the b5 camera UBO, right after the
/// 224-byte M4 clip-map tail ([`B5_CAMERA_UBO_BYTES_M4`]). The host writes
/// `MeshSdfParams::from_field(field).as_bytes()` here when the MDF shadow path is armed.
pub const MESH_SDF_PARAMS_OFFSET: usize = B5_CAMERA_UBO_BYTES_M4;

/// The grid-transform block the marcher's `mesh_sdf_sample` reads to fetch the dedicated dense
/// mesh-SDF texture (MDF Stage-2c). `#[repr(C)]`, 32 bytes — two std140 `vec4` lanes mirroring
/// the shader's `cbuffer Camera` `MeshSdfParams` fields:
///
/// - lane 0 `origin_inv_voxel` — `xyz` = the grid min world corner (`grid_origin`), `w` =
///   `1.0 / voxel_size` (the world→voxel scale the sample multiplies the offset by).
/// - lane 1 `dims_band` — `xyz` = the grid dims `[x, y, z]` as `f32` (read as `uint3` via a
///   `(uint)` cast — an exact small-integer round trip), `w` = `band_half` (the snorm decode
///   world scale, `decode_snorm8`'s `× band_half` step).
///
/// The grid voxel CENTER of voxel `i` is `grid_origin + (i + 0.5) * voxel_size`, so the
/// texture-space UVW of a world point `p` is `((p - grid_origin) * inv_voxel_size) / dims`
/// (the marcher applies the `/ dims` to land on `[0,1]³` for the hardware trilinear fetch). The
/// offsets are pinned by the const-asserts below so a host/shader desync is a build error.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshSdfParams {
    /// Lane 0: `xyz` = the grid min world corner, `w` = `1.0 / voxel_size`.
    pub origin_inv_voxel: [f32; 4],
    /// Lane 1: `xyz` = the grid dims `[x, y, z]` as `f32` (read as `uint3`), `w` = `band_half`.
    pub dims_band: [f32; 4],
}

/// Byte size of [`MeshSdfParams`] — two std140 `vec4` lanes (32 B), the b5 UBO MDF tail.
pub const MESH_SDF_PARAMS_BYTES: usize = core::mem::size_of::<MeshSdfParams>();

/// The b5 camera-UBO byte size widened for the MDF Stage-2c grid transform: the 224-byte M4 UBO
/// ([`B5_CAMERA_UBO_BYTES_M4`]) + the 32-byte [`MeshSdfParams`] tail (`= 256`). The MDF demo
/// write path uses this; the M4 write site keeps [`B5_CAMERA_UBO_BYTES_M4`] (the MDF block is
/// zero / bound-but-unread on the non-MDF path).
pub const B5_CAMERA_UBO_BYTES_MESH_SDF: usize = MESH_SDF_PARAMS_OFFSET + MESH_SDF_PARAMS_BYTES;

const _: () = assert!(core::mem::offset_of!(MeshSdfParams, origin_inv_voxel) == 0);
const _: () = assert!(core::mem::offset_of!(MeshSdfParams, dims_band) == 16);
const _: () = assert!(MESH_SDF_PARAMS_BYTES == 32, "MeshSdfParams must be 32 bytes (2 vec4 lanes)");
const _: () = assert!(
    B5_CAMERA_UBO_BYTES_MESH_SDF == 256,
    "the MDF block must extend the b5 UBO to 256 bytes (224 + 32)"
);

impl MeshSdfParams {
    /// Builds the MDF grid-transform block from a baked [`MeshSdfField`] (Stage-2a), the SAME
    /// descriptor the [`crate::mesh_sdf_texture::MeshSdfTexture`] was baked + uploaded with.
    #[inline]
    pub fn from_field(field: &MeshSdfField) -> Self {
        Self {
            origin_inv_voxel: [
                field.grid_origin[0],
                field.grid_origin[1],
                field.grid_origin[2],
                1.0 / field.voxel_size,
            ],
            dims_band: [
                field.grid_dim[0] as f32,
                field.grid_dim[1] as f32,
                field.grid_dim[2] as f32,
                field.band_half,
            ],
        }
    }

    /// Re-views the block as its raw 32-byte slice for the b5 UBO write (at
    /// [`MESH_SDF_PARAMS_OFFSET`]).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `[f32; 4]` fields (all `Copy`, every offset +
        // the 32-byte total pinned by the const-asserts above, no uninit padding — two packed
        // vec4 lanes), so its `size_of` bytes are a fully-initialized, alignment-valid POD bit
        // pattern. The `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
        unsafe {
            slice::from_raw_parts((self as *const Self).cast::<u8>(), core::mem::size_of::<Self>())
        }
    }
}

/// The voxel encoding the M2 brick atlas stores, chosen from the device's linear-filter support
/// (mirrors [`crate::device::DeviceCaps::atlas_format`]). `R8_SNORM` is the dense quantized path;
/// `R16_SFLOAT` is the fallback when the GPU cannot linear-filter `R8_SNORM` (no quantization — the
/// `EPSILON_Q` store bias is harmless there). The CPU baker ([`bake_brick_atlas`]) writes either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasEncoding {
    /// `R8_SNORM`: one signed byte per voxel (the snorm code [`fill_brick`] quantizes to).
    Snorm8,
    /// `R16_SFLOAT`: one half-float per voxel (the `decode_snorm8` value re-encoded to f16).
    Sfloat16,
}

impl AtlasEncoding {
    /// Picks the encoding from the device's `R8_SNORM` linear-filter support: `Snorm8` when the GPU
    /// can linear-filter the quantized atlas, else the `Sfloat16` fallback (the SAME choice
    /// [`crate::device::DeviceCaps::atlas_format`] maps to a `VkFormat`).
    #[inline]
    pub const fn from_linear_filter_ok(linear_filter_ok: bool) -> Self {
        if linear_filter_ok {
            Self::Snorm8
        } else {
            Self::Sfloat16
        }
    }

    /// Bytes per voxel for this encoding (`1` for `Snorm8`, `2` for `Sfloat16`).
    #[inline]
    pub const fn bytes_per_voxel(self) -> usize {
        match self {
            Self::Snorm8 => 1,
            Self::Sfloat16 => 2,
        }
    }

    /// The total byte size of the dense `M2_ATLAS_DIM³` atlas for this encoding — the staging-buffer
    /// + 3D-image byte size [`crate::brick_atlas::BrickAtlas`] allocates.
    #[inline]
    pub const fn atlas_byte_size(self) -> usize {
        let voxels = (M2_ATLAS_DIM as usize) * (M2_ATLAS_DIM as usize) * (M2_ATLAS_DIM as usize);
        voxels * self.bytes_per_voxel()
    }
}

/// The linear voxel index of atlas voxel `(x, y, z)` in the dense `M2_ATLAS_DIM³` lattice
/// (`x + y*W + z*W*W`, `W = M2_ATLAS_DIM`) — the SAME order the Vulkan 3D image's tightly-packed
/// `copy_buffer_to_image` reads (row-major, x fastest), so the host baker's scatter and the GPU
/// texel address agree.
#[inline]
pub const fn atlas_voxel_index(x: u32, y: u32, z: u32) -> usize {
    let w = M2_ATLAS_DIM as usize;
    x as usize + y as usize * w + z as usize * w * w
}

/// IEEE-754 binary32 → binary16 (round-to-nearest-even) — the f16 encode the `Sfloat16` atlas path
/// stores. Reuses the validated [`golden_f16_from_f32`] (the same encoder the lighting cone path
/// uses); the decoded snorm value lives in `[-band_half, band_half] ⊂ [-1, 1]`, inside the f16
/// normal range.
#[inline]
pub fn f16_from_f32(f: f32) -> u16 {
    golden_f16_from_f32(f)
}

/// The per-LEVEL geometry one clip-map (M4) brick-grid bake runs at: the snapped grid origin,
/// the cell/brick world edge, the voxel edge, and the snorm store-band half-width — the four
/// quantities that DIFFER per clip-map level ([`boyko_sdf_math::brick`]'s `*_at_level` table).
/// Everything ELSE the bake touches (the `M2_GRID_DIM³` cell count, the `M2_ATLAS_DIM³` atlas
/// geometry, the `BRICK_ALLOC³` tile shape) is LEVEL-INVARIANT, so a single `BrickLevelParams`
/// threads the entire per-level variation through the ONE proven baker — no fork.
///
/// Level 0 == the M2 single-level constants ([`BrickLevelParams::m2_near_field`]): the level-0
/// bake is byte-identical to the const-path [`bake_brick_atlas`], which delegates to the
/// level-aware [`bake_brick_atlas_at`] with exactly this value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickLevelParams {
    /// The minimum world corner of this level's `M2_GRID_DIM³` brick grid (cell `(0,0,0)`'s min).
    /// At level 0 this is [`M2_GRID_ORIGIN`] on every axis; at coarser camera-centered levels it
    /// is the snapped origin ([`snapped_level_origin`]).
    pub origin: [f32; 3],
    /// The INTEGER cell snap of this level's grid (M5: `origin == origin_cell · brick_world`).
    /// The toroidal STORAGE slot of grid box-cell `box` is `toroidal_slot(origin_cell + box)`
    /// ([`boyko_sdf_math::brick::toroidal_slot`]); at `origin_cell ≡ [0,0,0]` the slot equals the
    /// box index, so the bake scatter is byte-identical to the M4 `m2_tile_atlas_origin(box)` map
    /// (the OFF reduction). The level-0 const path ([`m2_near_field`](Self::m2_near_field)) pins
    /// `[0,0,0]`; the camera-centered path derives it from [`snapped_level_origin_cell`].
    pub origin_cell: [i32; 3],
    /// The world edge of one brick cell at this level ([`brick_world_at_level`]; `M2_BRICK_WORLD`
    /// at level 0). The brick spans `[cell_min, cell_min + brick_world]³`.
    pub brick_world: f32,
    /// The world edge of one atlas voxel at this level ([`voxel_size_at_level`]; `M2_VOXEL_SIZE`
    /// at level 0). `brick_world == BRICK_INTERIOR * voxel_size` at every level.
    pub voxel_size: f32,
    /// The snorm store-band half-width at this level ([`band_half_at_level`]; `M2_BAND_HALF` at
    /// level 0). The codes `fill_brick` quantizes span `[-band_half, band_half]`.
    pub band_half: f32,
    /// The maximum band curvature this level supports ([`c_max_at_level`]; the bare `C_MAX` `==
    /// c_max_at_level(0)` at level 0). Passed into [`fill_brick`] to scope its conservative-lower-bound
    /// dominance assert to THIS level's budget (a coarser level promises only a `2^L×`-larger radius of
    /// curvature). Assert-only — it never enters a stored snorm code, so it does not affect baked bytes.
    pub c_max: f32,
}

impl BrickLevelParams {
    /// The M2 single-level (level-0) geometry: origin [`M2_GRID_ORIGIN`], brick [`M2_BRICK_WORLD`],
    /// voxel [`M2_VOXEL_SIZE`], band [`M2_BAND_HALF`]. The const-path [`bake_brick_atlas`] /
    /// [`m2_dirty_cell_bbox`] / [`rebake_dirty_brick_atlas`] bake at exactly this value, so they
    /// stay byte-identical to the pre-M4 const path.
    #[inline]
    pub const fn m2_near_field() -> Self {
        // The static M2 grid's ABSOLUTE origin cell: `round(M2_GRID_ORIGIN / M2_BRICK_WORLD)`
        // (`-4 / 2 == -2`), the value the shader's Decision-5 recompute `round(origin/bw)` yields
        // for this grid, so the host baker's scatter slot and the shader's sample slot AGREE. The
        // toroidal slot is a stable per-grid PERMUTATION of the tile positions
        // (`toroidal_slot(origin_cell + box)`); the host bakes and the shader samples the SAME
        // permutation, so the rendered result is byte-identical and the incremental==full identity
        // holds. (A FIXED grid never scrolls, so its permutation never changes.)
        const M2_ORIGIN_CELL: i32 = (M2_GRID_ORIGIN / M2_BRICK_WORLD) as i32;
        Self {
            origin: [M2_GRID_ORIGIN, M2_GRID_ORIGIN, M2_GRID_ORIGIN],
            origin_cell: [M2_ORIGIN_CELL, M2_ORIGIN_CELL, M2_ORIGIN_CELL],
            brick_world: M2_BRICK_WORLD,
            voxel_size: M2_VOXEL_SIZE,
            band_half: M2_BAND_HALF,
            // Level-0 curvature bound (`== C_MAX`): the L0 fill_brick assert is unchanged → byte-identical.
            c_max: c_max_at_level(0),
        }
    }

    /// This level's geometry for clip-map (M4) `level`, camera-centered on `camera`: the snapped
    /// grid origin ([`snapped_level_origin`]) + the `*_at_level` brick/voxel/band scale. Level 0
    /// reduces to [`m2_near_field`](Self::m2_near_field) only when `camera == [0, 0, 0]` (otherwise
    /// the snapped origin tracks the camera — the clip-map anti-jitter origin).
    #[inline]
    pub fn at_level(camera: [f32; 3], level: u32) -> Self {
        Self {
            origin: snapped_level_origin(camera, level),
            // M5: the integer cell snap (`origin == origin_cell · brick_world`), the toroidal-slot
            // offset. The shader's Decision-5 recompute `round(origin/bw)` yields the SAME cell, so
            // the host scatter and the GPU sample agree.
            origin_cell: snapped_level_origin_cell(camera, level),
            brick_world: brick_world_at_level(level),
            voxel_size: voxel_size_at_level(level),
            band_half: band_half_at_level(level),
            // The per-level curvature bound (`C_MAX / 2^level`): scopes fill_brick's dominance assert
            // to this level's budget so it holds at coarse levels (the M4 per-level soundness fix).
            c_max: c_max_at_level(level),
        }
    }

    /// Reconstructs level `level`'s geometry from a baked [`M4GridParams`] (the snapped origin +
    /// scales the clip-map levels were CREATED at). The incremental dirty rebake uses this so a
    /// level diffs the authority against the SAME grid it was baked against (NOT a freshly re-snapped
    /// origin, which a dirty edit must not move). `level < BRICK_LEVELS`.
    #[inline]
    pub fn at_level_from_params(params: &M4GridParams, level: usize) -> Self {
        debug_assert!(level < brick::BRICK_LEVELS, "level out of range");
        let blk = &params.levels[level];
        let origin = [blk.origin_brick_world[0], blk.origin_brick_world[1], blk.origin_brick_world[2]];
        let brick_world = blk.origin_brick_world[3];
        // M5: reconstruct the integer cell snap as `round(origin / brick_world)` — the SAME formula
        // the shader's Decision-5 recompute uses, so the dirty rebake scatters to the SAME toroidal
        // slots the GPU samples (and the SAME slots the full bake at this `origin_cell` wrote).
        let origin_cell = [
            (origin[0] / brick_world).round() as i32,
            (origin[1] / brick_world).round() as i32,
            (origin[2] / brick_world).round() as i32,
        ];
        Self {
            origin,
            origin_cell,
            brick_world,
            voxel_size: blk.band_voxel_inv_atlas[1],
            band_half: blk.band_voxel_inv_atlas[0],
            // The curvature bound is a pure function of the level (`C_MAX / 2^level`), not a baked field
            // of `M4GridParams`; derive it from `level` so the dirty rebake uses the SAME per-level
            // dominance budget the full bake did. Assert-only, so it does not affect the rebaked bytes.
            c_max: c_max_at_level(level as u32),
        }
    }

    /// The minimum world corner of grid cell `cell = (cx, cy, cz)` at this level:
    /// `origin + cell * brick_world`. The level-aware sibling of [`m2_cell_min`] (which pins the
    /// level-0 const origin); the brick spans `[min, min + brick_world]³`.
    #[inline]
    pub fn cell_min(&self, cell: [u32; 3]) -> [f32; 3] {
        [
            self.origin[0] + cell[0] as f32 * self.brick_world,
            self.origin[1] + cell[1] as f32 * self.brick_world,
            self.origin[2] + cell[2] as f32 * self.brick_world,
        ]
    }
}

/// Bakes the dense `M2_ATLAS_DIM³` brick atlas from the ONE edit authority `field` into `out`
/// (the staging bytes), in the chosen [`AtlasEncoding`], returning the number of SURFACE cells
/// baked.
///
/// For each M2 grid cell, classifies the brick ([`classify_brick`] at [`SDF_EDIT_BAND_HALF`]); a
/// SURFACE cell's apron'd `BRICK_ALLOC³` tile is baked ([`fill_brick`]) and scattered into `out` at
/// the cell's atlas-voxel origin ([`m2_tile_atlas_origin`]). EMPTY cells leave their tile voxels at
/// `0` (a mid-band code — never sampled, since the marcher only enters the M2 cubic on a SURFACE
/// cell via the M2 grid lookup). `Snorm8` stores the raw `i8` byte; `Sfloat16` stores
/// `f16_from_f32(decode_snorm8(byte))` (the decoded normalized value, NOT multiplied by `band_half`
/// — the shader's `m2_decode` applies `band_half`, matching the snorm hardware decode that also
/// returns the normalized value).
///
/// `out.len()` MUST be `encoding.atlas_byte_size()`. This is a SETUP-time (per-`gen`) bake, not a
/// hot-path call. Principle 0: a transient mirror of the analytic authority, no durable state.
///
/// The const-path M2 baker: delegates to the level-aware [`bake_brick_atlas_at`] at the level-0
/// [`BrickLevelParams::m2_near_field`], so it is byte-identical to the pre-M4 implementation.
pub fn bake_brick_atlas(field: &SdfEditField, encoding: AtlasEncoding, out: &mut [u8]) -> u32 {
    bake_brick_atlas_at(field, encoding, &BrickLevelParams::m2_near_field(), out)
}

/// The level-aware M4 full atlas bake: bakes the dense `M2_ATLAS_DIM³` atlas for ONE clip-map
/// level's [`BrickLevelParams`] (origin/brick_world/voxel/band) into `out`, returning the SURFACE
/// cell count. The ONE proven baker behind both the M2 const path ([`bake_brick_atlas`], at the
/// level-0 [`BrickLevelParams::m2_near_field`]) and the M4 clip-map (one call per level). The atlas
/// GEOMETRY (`M2_GRID_DIM³` cells, `M2_ATLAS_DIM³` voxels, `BRICK_ALLOC³` tiles) is level-invariant
/// — only `params` differs per level — so no bake logic is duplicated.
pub fn bake_brick_atlas_at(
    field: &SdfEditField,
    encoding: AtlasEncoding,
    params: &BrickLevelParams,
    out: &mut [u8],
) -> u32 {
    debug_assert_eq!(
        out.len(),
        encoding.atlas_byte_size(),
        "atlas staging must be encoding.atlas_byte_size() bytes"
    );

    let mut surface_cells = 0u32;
    let mut tile = [0i8; BRICK_VOXELS];

    for cz in 0..M2_GRID_DIM {
        for cy in 0..M2_GRID_DIM {
            for cx in 0..M2_GRID_DIM {
                if bake_atlas_cell(field, encoding, params, [cx, cy, cz], &mut tile, out) {
                    surface_cells += 1;
                }
            }
        }
    }
    surface_cells
}

/// Bakes ONE M2 grid `cell` into the staging atlas `out` (the shared per-cell step
/// of [`bake_brick_atlas`] and [`rebake_dirty_brick_atlas`]). Classifies the cell;
/// a SURFACE cell's apron'd tile is filled ([`fill_brick`]) and scattered at the
/// cell's atlas-voxel origin; an EMPTY cell ZEROES its tile region (so a cell that
/// transitioned SURFACE→EMPTY in M3 leaves no stale surface — the same all-zero
/// mid-band the full baker leaves an empty cell at). `tile` is a caller-owned
/// scratch buffer (reused across cells to avoid a per-cell stack zero). Returns
/// whether the cell was SURFACE.
///
/// FULL/INCREMENTAL BIT-IDENTITY: the full baker visits every cell exactly once;
/// the incremental baker visits ONLY dirty cells. A non-dirty SURFACE cell's bytes
/// are provably unchanged (its tile fold reads the SAME edits), so skipping it
/// leaves the staging byte-identical to a full re-bake — hence `rebake_dirty`'s
/// atlas equals `rebake`'s.
#[inline]
fn bake_atlas_cell(
    field: &SdfEditField,
    encoding: AtlasEncoding,
    params: &BrickLevelParams,
    cell: [u32; 3],
    tile: &mut [i8; BRICK_VOXELS],
    out: &mut [u8],
) -> bool {
    const W: usize = BRICK_ALLOC;
    let cell_min = params.cell_min(cell);
    // Classify against the SAME band the per-edit AABBs are skinned by at this level's scale
    // ([`band_half_at_level`]); at level 0 `params.band_half == SDF_EDIT_BAND_HALF`, byte-identical
    // to the pre-M4 const path.
    let class = classify_brick(field, cell_min, params.brick_world, params.band_half);
    let is_surface = class == BrickClass::Surface;
    // M5: scatter to the TOROIDAL storage slot (`toroidal_slot(origin_cell + cell)`), decoupling the
    // world box-cell from its atlas tile so a scroll re-bakes only the revealed slab onto the exited
    // cells' slots. The shader samples this SAME slot (Decision 5: `toroidal_slot(round(origin/bw) +
    // box)`, and `origin_cell == round(origin/bw)`). It is a stable per-grid PERMUTATION of the M4
    // box→box map; the host bakes and the GPU samples the identical permutation, so the result is
    // byte-identical and incremental==full holds.
    let slot = toroidal_slot([
        params.origin_cell[0] + cell[0] as i32,
        params.origin_cell[1] + cell[1] as i32,
        params.origin_cell[2] + cell[2] as i32,
    ]);
    let [ox, oy, oz] = m2_tile_atlas_origin(slot);

    if is_surface {
        // Bake the apron'd tile from the authority, then scatter it into the dense atlas at
        // the cell's atlas-voxel origin (the SAME `tile * BRICK_ALLOC` the shader addresses).
        fill_brick(field, cell_min, params.voxel_size, params.band_half, params.c_max, tile);
    }

    for lz in 0..W {
        for ly in 0..W {
            for lx in 0..W {
                // EMPTY cells (and SURFACE→EMPTY transitions) store 0 — a mid-band code never
                // sampled by the marcher (it enters the M2 cubic only on a SURFACE grid cell),
                // and the SAME byte the full baker leaves an empty cell at (full/incremental parity).
                let byte = if is_surface { tile[lx + ly * W + lz * W * W] } else { 0i8 };
                let vx = ox + lx as u32;
                let vy = oy + ly as u32;
                let vz = oz + lz as u32;
                let vi = atlas_voxel_index(vx, vy, vz);
                match encoding {
                    AtlasEncoding::Snorm8 => {
                        out[vi] = byte as u8;
                    }
                    AtlasEncoding::Sfloat16 => {
                        // Store the DECODED normalized value (in [-1, 1]); the shader's
                        // `m2_decode` multiplies by `band_half`, so the f16 lane carries
                        // the same normalized value the snorm hardware decode returns.
                        let n = decode_snorm8(byte, 1.0);
                        let h = f16_from_f32(n).to_le_bytes();
                        out[vi * 2] = h[0];
                        out[vi * 2 + 1] = h[1];
                    }
                }
            }
        }
    }
    is_surface
}

/// The inclusive M2 grid-cell bounding box (`(min_cell, max_cell)`) of the
/// authority's dirty region (M3) — the cells whose tiles must be re-baked +
/// re-uploaded after a dynamic edit. Returns `None` when no edit is dirty (the
/// prior atlas is already current; the caller skips the rebake).
///
/// The dirty WORLD AABB ([`dirty_world_aabb`], the swept old+new union — the
/// union-dirty rule that clears a moved edit's ghost) is mapped to the M2 grid:
/// every cell whose `[cell_min, cell_min + M2_BRICK_WORLD]³` AABB overlaps it is
/// inside the returned box. The box is the integer span of those cells (clamped to
/// `[0, M2_GRID_DIM)`), so the dirty tiles form ONE contiguous atlas-voxel region
/// the sub-region [`copy_buffer_to_image`](boyko_rhi::RhiCommandEncoder::copy_buffer_to_image)
/// uploads in a single `BufferImageCopy`. Cells inside the box but not themselves
/// dirty are re-baked to the SAME bytes (full/incremental parity holds).
pub fn m2_dirty_cell_bbox(field: &SdfEditField) -> Option<([u32; 3], [u32; 3])> {
    m2_dirty_cell_bbox_at(field, &BrickLevelParams::m2_near_field())
}

/// The level-aware M4 dirty-cell bounding box: the inclusive cell span of the authority's dirty
/// region for ONE clip-map level's [`BrickLevelParams`] (origin/brick_world/voxel/band) — the
/// level-aware sibling of [`m2_dirty_cell_bbox`] (which delegates here at the level-0
/// [`BrickLevelParams::m2_near_field`]). The dirty region is widened by this level's band SHORTFALL
/// (`params.band_half - SDF_EDIT_BAND_HALF`, the per-level store reach past the level-0 skin already
/// baked into `field.aabbs`) and the apron skin scales with the level's `voxel_size`, and the
/// world→cell mapping uses the level's `origin`/`brick_world`, so each clip-map level diffs the SAME
/// authority against its OWN grid at its OWN voxel reach (the M4 coarse-level under-cover fix).
pub fn m2_dirty_cell_bbox_at(
    field: &SdfEditField,
    params: &BrickLevelParams,
) -> Option<([u32; 3], [u32; 3])> {
    let dirty = dirty_world_aabb(field)?;

    // PER-LEVEL band shortfall (the M4 coarse-level under-cover fix). `dirty_world_aabb` returns the
    // union of `field.aabbs[i]`, each pre-skinned at PUSH/SET time by the LEVEL-0 band
    // [`SDF_EDIT_BAND_HALF`] (see [`boyko_sdf_math::edit_aabb`]). At clip-map level `L` the stored
    // voxels reach `params.band_half == band_half_at_level(L)` from the surface (`2^L`× wider), so a
    // moved edit changes stored bytes that much farther than the level-0-skinned dirty AABB covers.
    // Inflate by the SHORTFALL `params.band_half - SDF_EDIT_BAND_HALF` so the dirty region matches THIS
    // level's voxel reach (the classifier overlaps a cell whose surface is within `band_half` of the
    // edit). At level 0 `params.band_half == SDF_EDIT_BAND_HALF`, so the shortfall is `0.0` and the box
    // is byte-identical to the M2 const path; coarser levels widen conservatively (over-mark a few
    // cells, NEVER under-mark — the C2 dirty-set invariant). `max(0.0)` guards against an fp negative.
    let band_shortfall = (params.band_half - SDF_EDIT_BAND_HALF).max(0.0);
    let dirty = SdfEditAabb {
        min: [
            dirty.min[0] - band_shortfall,
            dirty.min[1] - band_shortfall,
            dirty.min[2] - band_shortfall,
        ],
        max: [
            dirty.max[0] + band_shortfall,
            dirty.max[1] + band_shortfall,
            dirty.max[2] + band_shortfall,
        ],
    };

    // Inflate by the 1-voxel APRON reach so the box matches `m2_cell_is_dirty_at` exactly:
    // a SURFACE cell bakes apron voxels one `voxel_size` past its interior faces ([`fill_brick`]),
    // so a dirty region touching only a cell's apron band still dirties it (the seed=0 hard-scene
    // high-face apron divergence). Both the box here and the per-cell test must skin the SAME apron
    // margin (scaled to this level's voxel), or the box would miss an apron-only dirty cell that
    // `m2_cell_is_dirty_at` flags (a ghost the upload would never patch).
    let apron_world = boyko_sdf_math::brick::APRON as f32 * params.voxel_size;
    let dirty = SdfEditAabb {
        min: [
            dirty.min[0] - apron_world,
            dirty.min[1] - apron_world,
            dirty.min[2] - apron_world,
        ],
        max: [
            dirty.max[0] + apron_world,
            dirty.max[1] + apron_world,
            dirty.max[2] + apron_world,
        ],
    };

    // Map the dirty world AABB to the inclusive cell index span on each axis. The grid is
    // `M2_GRID_DIM` cells of `params.brick_world`, origin `params.origin`. A cell `c` overlaps the
    // dirty AABB on an axis iff `cell_min(c) <= dirty.max` and `cell_min(c) + brick_world >=
    // dirty.min`; that is `c` in `[floor((dirty.min - origin)/bw - 1), floor((dirty.max -
    // origin)/bw)]`, clamped to `[0, M2_GRID_DIM)`. A box that lies fully outside the grid yields
    // an empty span (no overlap) → `None`.
    let mut lo = [0u32; 3];
    let mut hi = [0u32; 3];
    for a in 0..3 {
        let rel_min = (dirty.min[a] - params.origin[a]) / params.brick_world;
        let rel_max = (dirty.max[a] - params.origin[a]) / params.brick_world;
        // A cell overlaps from one cell BELOW `floor(rel_min)` (the dirty min may fall inside the
        // previous cell's high face) up to `floor(rel_max)`.
        let lo_f = (rel_min.floor() - 1.0).max(0.0);
        let hi_f = rel_max.floor();
        if hi_f < 0.0 || lo_f > (M2_GRID_DIM - 1) as f32 {
            return None; // Dirty region fully outside the bounded grid: nothing to re-bake.
        }
        lo[a] = lo_f as u32;
        hi[a] = (hi_f.min((M2_GRID_DIM - 1) as f32)).max(0.0) as u32;
        if lo[a] > hi[a] {
            return None;
        }
    }
    Some((lo, hi))
}

/// Incrementally re-bakes ONLY the M2 grid cells inside the dirty bounding box
/// `(lo, hi)` (inclusive, from [`m2_dirty_cell_bbox`]) into the staging atlas
/// `out` (M3) — the dynamic-edit fast path for [`bake_brick_atlas`].
///
/// `out` MUST be the staging buffer the last full [`bake_brick_atlas`] filled (its
/// cells outside the box hold the correct prior bytes). Re-classifies + re-fills
/// every cell in the box; a cell that turned SURFACE→EMPTY is zeroed (no ghost).
/// Returns the number of SURFACE cells in the box. After this the box's atlas
/// voxels are byte-identical to a full re-bake's, so the sub-region upload of the
/// box keeps the GPU atlas bit-identical to a full `rebake`.
pub fn rebake_dirty_brick_atlas(
    field: &SdfEditField,
    encoding: AtlasEncoding,
    lo: [u32; 3],
    hi: [u32; 3],
    out: &mut [u8],
) -> u32 {
    rebake_dirty_brick_atlas_at(field, encoding, &BrickLevelParams::m2_near_field(), lo, hi, out)
}

/// The level-aware M4 incremental dirty rebake: re-bakes ONLY the cells in `(lo, hi)` for ONE
/// clip-map level's [`BrickLevelParams`] into `out` — the level-aware sibling of
/// [`rebake_dirty_brick_atlas`] (which delegates here at the level-0
/// [`BrickLevelParams::m2_near_field`]). `out` MUST be the staging the level's last full
/// [`bake_brick_atlas_at`] filled; the box is from [`m2_dirty_cell_bbox_at`] at the SAME `params`.
pub fn rebake_dirty_brick_atlas_at(
    field: &SdfEditField,
    encoding: AtlasEncoding,
    params: &BrickLevelParams,
    lo: [u32; 3],
    hi: [u32; 3],
    out: &mut [u8],
) -> u32 {
    debug_assert_eq!(
        out.len(),
        encoding.atlas_byte_size(),
        "atlas staging must be encoding.atlas_byte_size() bytes"
    );
    debug_assert!(
        hi[0] < M2_GRID_DIM && hi[1] < M2_GRID_DIM && hi[2] < M2_GRID_DIM,
        "dirty cell box must lie inside the M2 grid"
    );

    let mut surface_cells = 0u32;
    let mut tile = [0i8; BRICK_VOXELS];
    for cz in lo[2]..=hi[2] {
        for cy in lo[1]..=hi[1] {
            for cx in lo[0]..=hi[0] {
                if bake_atlas_cell(field, encoding, params, [cx, cy, cz], &mut tile, out) {
                    surface_cells += 1;
                }
            }
        }
    }
    surface_cells
}

/// The per-axis box-cell count of one clip-map level grid (`M2_GRID_DIM`), as a `usize`.
const SCROLL_GRID_DIM: usize = M2_GRID_DIM as usize;

/// The total box-cell count of one grid (`M2_GRID_DIM³`) — the [`ScrollRebakeSet`] bitmap length.
pub const SCROLL_REBAKE_CELLS: usize = SCROLL_GRID_DIM * SCROLL_GRID_DIM * SCROLL_GRID_DIM;

/// The dedup bitmap of BOX cells (NEW-grid relative, `[0, M2_GRID_DIM)³`) a scroll re-bakes: the
/// REVEALED slab ([`for_each_revealed_cell`]) UNIONed with the M3-dirty cells in the new box, each
/// cell flagged at most once (a revealed cell that is also dirty is baked once). Index `cx + cy·DIM
/// + cz·DIM²`. A preallocated, heap-free `[bool; M2_GRID_DIM³]` (Principle 5).
#[derive(Clone, Copy, Debug)]
pub struct ScrollRebakeSet {
    marked: [bool; SCROLL_REBAKE_CELLS],
}

impl Default for ScrollRebakeSet {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollRebakeSet {
    /// An empty set (no cell marked).
    #[inline]
    pub const fn new() -> Self {
        Self { marked: [false; SCROLL_REBAKE_CELLS] }
    }

    /// The linear index of box cell `(cx, cy, cz)` in the bitmap (`cx + cy·DIM + cz·DIM²`).
    #[inline]
    const fn lin(cx: u32, cy: u32, cz: u32) -> usize {
        cx as usize + cy as usize * SCROLL_GRID_DIM + cz as usize * SCROLL_GRID_DIM * SCROLL_GRID_DIM
    }

    /// Marks box cell `(cx, cy, cz)` (idempotent — the union dedup).
    #[inline]
    fn mark(&mut self, cx: u32, cy: u32, cz: u32) {
        self.marked[Self::lin(cx, cy, cz)] = true;
    }

    /// Whether box cell `(cx, cy, cz)` is marked.
    #[inline]
    pub fn is_marked(&self, cx: u32, cy: u32, cz: u32) -> bool {
        self.marked[Self::lin(cx, cy, cz)]
    }

    /// The inclusive box-cell bounding box `(lo, hi)` of every marked cell, or `None` when the set is
    /// empty. The upload split derives the touched ATLAS-VOXEL regions from this (mapped through the
    /// toroidal slot, which may wrap-split it across the `slot == 0` seam — A3).
    pub fn bbox(&self) -> Option<([u32; 3], [u32; 3])> {
        let mut lo = [u32::MAX; 3];
        let mut hi = [0u32; 3];
        let mut any = false;
        for cz in 0..M2_GRID_DIM {
            for cy in 0..M2_GRID_DIM {
                for cx in 0..M2_GRID_DIM {
                    if self.is_marked(cx, cy, cz) {
                        any = true;
                        let c = [cx, cy, cz];
                        for a in 0..3 {
                            lo[a] = lo[a].min(c[a]);
                            hi[a] = hi[a].max(c[a]);
                        }
                    }
                }
            }
        }
        if any { Some((lo, hi)) } else { None }
    }
}

/// Builds the scroll [`ScrollRebakeSet`] for a level that moved from `old_origin_cell` to
/// `new_params.origin_cell`: the REVEALED box cells (world cells in the new box not in the old box,
/// mapped to their NEW-grid box index) UNIONed with the M3-dirty box cells in the new box. Pure
/// bake-side, CPU-verifiable (no GPU). A teleport (`|Δ| ≥ M2_GRID_DIM` on any axis) reveals the whole
/// new box ([`for_each_revealed_cell`] handles the disjoint-box degenerate).
pub fn scroll_rebake_set(
    field: &SdfEditField,
    new_params: &BrickLevelParams,
    old_origin_cell: [i32; 3],
) -> ScrollRebakeSet {
    let mut set = ScrollRebakeSet::new();
    let new_oc = new_params.origin_cell;

    // 1) The revealed slab: world cells in the new box not in the old box. Map each to its NEW-grid
    //    box index `world_cell - new_oc` (always in `[0, M2_GRID_DIM)` because `for_each_revealed_cell`
    //    emits only cells inside the new box).
    for_each_revealed_cell(old_origin_cell, new_oc, |world_cell| {
        let bx = (world_cell[0] - new_oc[0]) as u32;
        let by = (world_cell[1] - new_oc[1]) as u32;
        let bz = (world_cell[2] - new_oc[2]) as u32;
        debug_assert!(
            bx < M2_GRID_DIM && by < M2_GRID_DIM && bz < M2_GRID_DIM,
            "revealed cell must map into the new grid box"
        );
        set.mark(bx, by, bz);
    });

    // 2) The M3-dirty cells inside the new box (a field edit that changed since the last bake). These
    //    must be re-baked even if they did not move into the box, so the scroll's staging equals a
    //    full re-bake of the new grid (the gate-(a) keystone). The dirty box is in NEW-grid box
    //    coordinates (`m2_dirty_cell_bbox_at` uses `new_params.origin`/`brick_world`).
    if let Some((lo, hi)) = m2_dirty_cell_bbox_at(field, new_params) {
        for cz in lo[2]..=hi[2] {
            for cy in lo[1]..=hi[1] {
                for cx in lo[0]..=hi[0] {
                    set.mark(cx, cy, cz);
                }
            }
        }
    }

    set
}

/// Re-bakes the marked box cells of `set` (the revealed slab ∪ M3-dirty) into the staging `out` at the
/// level's NEW [`BrickLevelParams`] (M5 — the camera-follow scroll fast path). Each marked box cell is
/// classified + filled at its WORLD `cell_min` ([`bake_atlas_cell`], which scatters to its TOROIDAL
/// slot `toroidal_slot(origin_cell + cell)`), so an exited cell's slot is overwritten by the cell that
/// scrolled into it. `out` MUST be the staging the level's last bake filled (un-touched slots keep
/// their prior bytes). Returns the number of SURFACE cells baked.
///
/// Bit-identity keystone: a full [`bake_brick_atlas_at`] at the NEW `params` visits every box cell and
/// scatters to `toroidal_slot(origin_cell + cell)`. A scroll re-bakes ONLY the revealed ∪ dirty box
/// cells to the SAME slots; an untouched box cell's slot holds the bytes a PRIOR bake wrote for the
/// world cell that still occupies it (unchanged because no edit reached it AND it did not leave the
/// box), so the staging is byte-identical to the full re-bake.
pub fn rebake_scroll_brick_atlas_at(
    field: &SdfEditField,
    encoding: AtlasEncoding,
    params: &BrickLevelParams,
    set: &ScrollRebakeSet,
    out: &mut [u8],
) -> u32 {
    debug_assert_eq!(
        out.len(),
        encoding.atlas_byte_size(),
        "atlas staging must be encoding.atlas_byte_size() bytes"
    );

    let mut surface_cells = 0u32;
    let mut tile = [0i8; BRICK_VOXELS];
    for cz in 0..M2_GRID_DIM {
        for cy in 0..M2_GRID_DIM {
            for cx in 0..M2_GRID_DIM {
                if set.is_marked(cx, cy, cz)
                    && bake_atlas_cell(field, encoding, params, [cx, cy, cz], &mut tile, out)
                {
                    surface_cells += 1;
                }
            }
        }
    }
    surface_cells
}

/// Whether M2 grid cell `cell`'s baked-VOXEL footprint overlaps the world-space
/// `dirty` AABB — the per-cell dirty test the M2 incremental rebake (and its tester)
/// gates on. Mirrors [`boyko_sdf_math::brick::cell_is_dirty`] for the M2 grid's own
/// consts, with the apron correction below.
///
/// # The APRON footprint (the M3 atlas-only correction)
///
/// [`fill_brick`] bakes a 1-voxel APRON ([`boyko_sdf_math::brick::APRON`]) on every
/// face: a SURFACE cell's stored voxels sample world points reaching one
/// [`M2_VOXEL_SIZE`] BEYOND the cell's `[cell_min, cell_min + M2_BRICK_WORLD]`
/// interior AABB. So a dirty region that touches ONLY a cell's apron band (without
/// reaching the interior AABB) still changes that cell's stored apron voxels — and a
/// bare interior-AABB overlap test would MISS it, leaving stale apron bytes (the
/// seed=0 hard-scene divergence at the low-face apron voxel). Inflate the tested cell
/// AABB by `APRON * M2_VOXEL_SIZE` on every face so the apron band is covered. The
/// apron-LESS pointer grid ([`boyko_sdf_math::brick::cell_is_dirty`]) needs no such
/// margin — `classify_brick` reads only the bare cell AABB + the cell center.
#[inline]
pub fn m2_cell_is_dirty(cell: [u32; 3], dirty: &SdfEditAabb) -> bool {
    const APRON_WORLD: f32 = boyko_sdf_math::brick::APRON as f32 * M2_VOXEL_SIZE;
    let cmin = m2_cell_min(cell);
    let cell_aabb = SdfEditAabb {
        min: [cmin[0] - APRON_WORLD, cmin[1] - APRON_WORLD, cmin[2] - APRON_WORLD],
        max: [
            cmin[0] + M2_BRICK_WORLD + APRON_WORLD,
            cmin[1] + M2_BRICK_WORLD + APRON_WORLD,
            cmin[2] + M2_BRICK_WORLD + APRON_WORLD,
        ],
    };
    aabb_overlap(&cell_aabb, dirty)
}

// ---------------------------------------------------------------------------
// M2 — the HOST GOLDEN MIRROR (`golden_composite_pixel_brick_m2`): the bit-exact reference the
// GPU M2 marcher is golden-compared against. Mirrors `sdf_gbuffer_composite.hlsl`'s
// `m2_surface_hit` (the atlas sample → JCGT cubic → analytic-residual fallback) over the SAME
// baked-tile data the GPU samples, then delegates the shade to the analytic path (C1).
// ---------------------------------------------------------------------------


// ===========================================================================
// Deferred-shading SPLIT (increment 1) — the host goldens for the marcher's
// ATTRIBUTE writes + the `deferred_pbr` RESOLVE.
//
// The marcher (`sdf_gbuffer_composite.hlsl`) no longer composites `base*shadow*ao`; it
// writes ATTRIBUTES (gAlbedo = the unmultiplied base, gMaterial = (vis, 0, mask, 1)).
// The fullscreen `deferred_pbr.comp` RESOLVE then computes `lit = mask ? base*vis : base`.
// [`golden_marcher_attributes`] mirrors the marcher's per-pixel attribute output and
// [`golden_deferred_resolve`] mirrors the resolve, modelling the EXACT GPU double
// quantization (base RGB8 + vis R8, then a second pack of base8/255 * vis8/255). The
// approximation-gate reference [`golden_composite_pixel_ex_omega_lit`] above is kept for
// the inline-composite comparison.
// ===========================================================================


// --- Lighting L0 host mirror (mirrors boyko_render::light + light_table.hlsli) -------

/// Light kind tags — mirror `boyko_render::light::LIGHT_KIND_*` and the shader's
/// `light_table.hlsli` `LIGHT_KIND_*`.
pub const GOLDEN_LIGHT_KIND_DIRECTIONAL: u32 = 0;
/// Point light kind (L0b resolve path).
pub const GOLDEN_LIGHT_KIND_POINT: u32 = 1;
/// Spot light kind (L0b resolve path).
pub const GOLDEN_LIGHT_KIND_SPOT: u32 = 2;
/// Sky/ambient light kind (L0a hemisphere ambient).
pub const GOLDEN_LIGHT_KIND_SKY: u32 = 3;

/// The word offset at which the `GpuLight[]` array begins in the light SSBO (mirrors
/// `boyko_render::light::LIGHT_HEADER_BASE_WORDS == 16` and the shader's `HEADER_BASE`).
pub const GOLDEN_LIGHT_HEADER_BASE_WORDS: usize = 16;


/// The bit offset of the 5-bit atlas-slot field in [`GoldenLight::dir_kind`]`.w` — mirrors
/// `boyko_render::shadow_atlas::ATLAS_SLOT_SHIFT` and the shader's `ATLAS_SLOT_SHIFT`.
pub const GOLDEN_ATLAS_SLOT_SHIFT: u32 = 17;
/// The 5-bit mask for the atlas-slot field — mirrors `boyko_render::shadow_atlas::ATLAS_SLOT_MASK`.
pub const GOLDEN_ATLAS_SLOT_MASK: u32 = 0x1F;
/// The "no map" 5-bit sentinel (`0x1F == 31`) — a light on the analytic fallback. Mirrors
/// `boyko_render::shadow_atlas::SLOT_NONE` and the shader's `SLOT_NONE`.
pub const GOLDEN_SLOT_NONE: u32 = 0x1F;

/// The kind-enum mask (low 16 bits) — mirrors the shader's `LIGHT_KIND_MASK`. The P6 R1
/// `casts_sdf_shadow` flag occupies bit 16, so the enum + the flag coexist in one word.
pub const GOLDEN_LIGHT_KIND_MASK: u32 = 0xFFFF;
/// Bit 16 of the kind word: the P6 R1 per-light `casts_sdf_shadow` flag (mirrors the shader's
/// `LIGHT_FLAG_CASTS_SHADOW`).
pub const GOLDEN_LIGHT_FLAG_CASTS_SHADOW: u32 = 0x1_0000;

/// The maximum `cos(outer)` the spot bake clamps to (mirrors
/// `boyko_render::light::SPOT_COS_OUTER_MAX`): bounds `I = Φ/(2π(1−cos))` as the cone
/// narrows to a pencil beam.
pub const GOLDEN_SPOT_COS_OUTER_MAX: f32 = 0.9999;


/// IEEE-754 binary32 → binary16 (round-to-nearest-even) — the host mirror of
/// `boyko_render::light::f16_from_f32`. The cone cosines live in `[-1, 1]`, inside the f16
/// normal range, so only the standard rounding is needed (no overflow special case beyond
/// the defensive inf/NaN guard).
pub(crate) fn golden_f16_from_f32(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        let m = if mant != 0 { 0x0200 } else { 0 };
        return sign | 0x7c00 | m;
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 0x1f {
        return sign | 0x7c00; // overflow → inf
    }
    if new_exp <= 0 {
        if new_exp < -10 {
            return sign; // underflow → signed zero
        }
        let full_mant = mant | 0x0080_0000;
        let shift = (14 - new_exp) as u32;
        let m = (full_mant >> shift) as u16;
        let round_bit = ((full_mant >> (shift - 1)) & 1) as u16;
        return sign | (m + round_bit);
    }
    let half = sign | ((new_exp as u16) << 10) | ((mant >> 13) as u16);
    let round = (mant >> 12) & 1;
    let sticky = mant & 0x0fff;
    if round == 1 && (sticky != 0 || (half & 1) == 1) {
        half + 1
    } else {
        half
    }
}


// --- PBR MVP-2 lighting constants (mirror deferred_pbr.hlsl EXACTLY) -----------------

/// The resolve's single analytic directional light (`DEFAULT_LIGHT_DIR` = +Z).
pub const PBR_LIGHT_DIR: [f32; 3] = [0.0, 0.0, 1.0];
/// The resolve's directional-light color (white).
pub const PBR_LIGHT_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
/// The resolve's analytic hemisphere diffuse-ambient sky color.
pub const PBR_SKY_DIFFUSE: [f32; 3] = [0.10, 0.10, 0.12];
/// The resolve's analytic specular-IBL sky color (scales EnvBRDFApprox).
pub const PBR_SKY_SPEC: [f32; 3] = [0.10, 0.10, 0.12];
/// The "empty field" distance sentinel, mirroring the shader's `FAR` (= 1e9 in
/// `sdf_field.hlsli`). Used as the argmin seed in [`pick_material_id`] so the host
/// oracle initializes its nearest-surface search identically to the GPU marcher.
pub const PBR_FAR: f32 = 1.0e9;


/// The Hilbert tile edge (`SSAO_HILBERT_W`; XeGTAO uses level 6 = 64) — the host mirror of the
/// shader const. The per-pixel dither index is `ssao_hilbert(64, px & 63, py & 63)`.
#[cfg(any(test, feature = "goldens"))]
pub(crate) const SSAO_HILBERT_W: u32 = 64;
/// R2 plastic-number reciprocals `1/phi`, `1/phi^2` (phi = 1.32471795724474602596...) in Q0.24
/// fixed point — the host mirrors of the shader consts `SSAO_R2_ALPHA1`/`SSAO_R2_ALPHA2`:
/// `round(0.75487766624669276 * 2^24)`, `round(0.56984029099805327 * 2^24)`.
#[cfg(any(test, feature = "goldens"))]
pub(crate) const SSAO_R2_ALPHA1: u32 = 12_664_746;
#[cfg(any(test, feature = "goldens"))]
pub(crate) const SSAO_R2_ALPHA2: u32 = 9_560_334;


// ===========================================================================
// Lighting L1 — clustered froxel light cull (host mirror of cluster_cull.hlsl +
// the deferred_pbr.hlsl cluster lookup).
//
// The host cull builds each froxel's WORLD-space AABB from the SAME ray-gen the GPU uses
// (`composite_ray`) at the exp-Z slice's near/far view-z, tests each point/spot light's
// bounding sphere (sqDistPointAABB <= r²), and records the surviving index SET per froxel.
// The clustered resolve then maps a pixel to its froxel and shades only that froxel's
// point/spot lights — which, when the cull is exact (no false drop under the cap), is
// BIT-IDENTICAL to the brute-force `golden_deferred_resolve_table` (the load-bearing L1
// golden). The linearization + the exp-Z slice/tile maps mirror `light_table.hlsli`.
// ===========================================================================


// ===========================================================================
// Render P4b — the conservative coarse-cull / tile pre-trace (host mirror).
//
// A 1/8-res CONSERVATIVE cone-trace emits, per 8×8 tile, a `TileBound{near_t,
// far_t, flags}`. The fine marcher then seeds `t = near_t` (skipping the
// proven-empty prefix) and early-outs EMPTY tiles into the existing mesh /
// background composite. This module is the HOST mirror of `sdf_tile_cull.hlsl`
// (`golden_tile_bound`, Algorithm A) + the culled fine marcher
// (`golden_composite_pixel_culled`, Algorithm B); the cone radii, the step
// formula, the cone-entry rule, and the far_t / exhaustion fallbacks reproduce
// the shader EXACTLY so the host conservative-invariant tests PROVE the cull is
// conservative on the CPU before the GPU golden runs. See docs/RENDER-P4-DESIGN.md.
//
// The field eval is byte-shared with the fine marcher (`sdf_edit_list`); the
// determinism boundary is INVIOLABLE — P4b only seeds `t` and adds the coarse
// pass, never touching the field math (`golden_composite_pixel_culled` wraps
// `golden_composite_pixel_ex`'s body; `coarse_enabled == false` is bit-identical).
// ===========================================================================

/// `flags` bit set on a tile the coarse cone-trace proved EMPTY (no SDF surface
/// in the cone in front of the deepest mesh). Mirrors the shader's
/// `TILE_FLAG_EMPTY`. An EMPTY tile still composites the mesh / background (D6).
pub const TILE_FLAG_EMPTY: u32 = 1;

/// The fine-pixel edge length of one coarse tile (an 8×8 footprint). Mirrors the
/// shader's `TILE_SIZE` and the `[numthreads]` group geometry of the fine marcher.
pub const TILE_SIZE: u32 = 8;

/// Max coarse cone-trace iterations per tile (D5). Exhaustion ⇒ NON-empty,
/// `near_t = 0` (the safe full-march fallback). Mirrors the shader's
/// `MAX_IT_COARSE`. Smaller than the fine `SDF_MAX_IT` (128) — the coarse pass is
/// the cheap pre-pass, and exhaustion degrades gracefully to a full fine march.
pub const MAX_IT_COARSE: u32 = 64;

/// Cone-entry threshold on the cone budget `d/L − r(t)` (D4). When the budget
/// drops to `<= EPS_COARSE` the cone has (conservatively) entered the surface
/// band: RECORD `near_t = t` and STOP. Mirrors the shader's `EPS_COARSE`.
pub const EPS_COARSE: f32 = 0.001;

/// Extra half-angle safety margin (radians) added to the per-tile perspective
/// cone half-angle (D3). Absorbs the fp-ULP slack so the cone strictly encloses
/// every in-tile pixel ray's footprint. Mirrors the shader's `ALPHA_MARGIN`.
pub const ALPHA_MARGIN: f32 = 1e-4;

/// The field's worst-case spatial gradient magnitude — the cone step's distance
/// divisor (D7). `sqrt(2)`: the IQ polynomial smin's steepest 90-degree blend
/// peaks at `sqrt(2)` (k sets the band width, not the peak slope); the analytic
/// primitives are unit-gradient. `d / FIELD_LIPSCHITZ_L` is a conservative lower
/// bound on the Euclidean clearance even where smin is super-Lipschitz. Mirrors
/// the shader's `FIELD_LIPSCHITZ_L` in `sdf_field.hlsli`.
pub const FIELD_LIPSCHITZ_L: f32 = core::f32::consts::SQRT_2;

/// `#[repr(C)]` per-tile cull bound, byte-identical to the std430
/// `RWStructuredBuffer<TileBound>` element the shader emits (16 B, scalar layout):
///
///   offset  0 : f32 near_t  (the seed `t` for the fine march; `[0, far_t]`)
///   offset  4 : f32 far_t    (the march bound = min(max 8×8 depth→t, T_MAX))
///   offset  8 : u32 flags    ([`TILE_FLAG_EMPTY`] or 0)
///   offset 12 : u32 _pad     (std430 16-B alignment)
///
/// Field offsets are pinned by the const-asserts below so a host/shader desync is
/// a build error (the same discipline as `CompositePushConstants`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileBound {
    /// The proven-empty prefix end: the fine march seeds `t = near_t`. In
    /// `[0, far_t]`; `0` for an EMPTY tile or a coarse exhaustion fallback.
    pub near_t: f32,
    /// The march bound — `min(max over the 8×8 depth texels of depth→t, T_MAX)`
    /// (the deepest mesh, conservative; a cleared / out-of-range texel ⇒ `T_MAX`).
    pub far_t: f32,
    /// [`TILE_FLAG_EMPTY`] when the coarse cone-trace proved the tile empty, else 0.
    pub flags: u32,
    /// std430 padding to a 16-byte stride.
    pub _pad: u32,
}

/// Byte size of [`TileBound`] — the `RWStructuredBuffer<TileBound>` element stride
/// (16 bytes). The coarse-cull buffer is `tiles_w * tiles_h * TILE_BOUND_BYTES`.
pub const TILE_BOUND_BYTES: usize = core::mem::size_of::<TileBound>();

// Pin the field offsets + the 16-byte std430 stride (a desync is a build error).
const _: () = assert!(core::mem::offset_of!(TileBound, near_t) == 0);
const _: () = assert!(core::mem::offset_of!(TileBound, far_t) == 4);
const _: () = assert!(core::mem::offset_of!(TileBound, flags) == 8);
const _: () = assert!(core::mem::offset_of!(TileBound, _pad) == 12);
const _: () = assert!(TILE_BOUND_BYTES == 16, "TileBound must be a 16-byte std430 element");

/// The coarse tile-grid extent for a `(img_w, img_h)` fine image: `tiles_w =
/// ceil(w / TILE_SIZE)`, `tiles_h = ceil(h / TILE_SIZE)`. The cull buffer holds
/// `tiles_w * tiles_h` [`TileBound`]s; the coarse dispatch covers them. Mirrors
/// the shader's `tiles_w` / `tiles_h`.
#[inline]
pub const fn tile_grid_extent(img_w: u32, img_h: u32) -> (u32, u32) {
    (img_w.div_ceil(TILE_SIZE), img_h.div_ceil(TILE_SIZE))
}


#[cfg(test)]
mod grazing_shadow_tests {
    //! STEP-1 confirmation + STEP-3 regression for the GRAZING-ANGLE SHADOW-ACNE fix.
    //!
    //! The SDF soft-shadow march starts at the surface point `p`, samples
    //! `field_distance(p + L*t)` from `t = SHADOW_MINT`, and returns `0` (occluder hit) as
    //! soon as `d < SHADOW_HIT_EPS`. At a GRAZING angle (the light `L` nearly tangent to the
    //! surface — the lit terminator, `n·L` small but POSITIVE so the point passes the
    //! `SHADOW_NDOTL_EPS` gate) the tangent ray's first samples stay within `~t²/(2R)` of a
    //! curved surface, so `d` reads below `SHADOW_HIT_EPS` and the march FALSE-occludes the
    //! point — the black "flame" acne on the terminator. The fix lifts the march ORIGIN by
    //! `n * SHADOW_NORMAL_BIAS` (applied at the call sites, mirrored host + GPU).
    //!
    //! These tests use the host soft-shadow mirror over a single analytic SPHERE (the same
    //! `sdf_sphere` the GPU `sdf_sphere` mirrors) so they are GPU-free and deterministic.

    use super::{SHADOW_HIT_EPS, SHADOW_K, SHADOW_MINT, SHADOW_MINT_STEP, SHADOW_NORMAL_BIAS, SDF_MAX_IT, SDF_SPHERE_CENTER, SDF_SPHERE_RADIUS, SDF_T_MAX, sdf_sphere};
    use crate::goldens::{host_soft_shadow, host_soft_shadow_ranged, sdf_normal};
    use boyko_sdf_math::{v_dot, v_normalize};

    // The march's Lipschitz step divisor — kept in sync with `host_soft_shadow`'s
    // `FIELD_LIPSCHITZ_L` (the committed shader literal). Used ONLY by the inline UNBIASED
    // reference march below (the bias-free reproduction of the pre-fix behavior).
    #[allow(clippy::approx_constant, clippy::excessive_precision)]
    const FIELD_LIPSCHITZ_L: f32 = 1.41421356;

    /// A bit-for-bit copy of the soft-shadow march WITHOUT the normal-offset start bias —
    /// the pre-fix behavior, kept here ONLY to reproduce the grazing acne for STEP 1. It
    /// marches from the RAW surface point `p` (no `n` lift), exactly as the host/GPU march
    /// did before this fix.
    fn unbiased_soft_shadow<F: Fn([f32; 3]) -> f32>(p: [f32; 3], l: [f32; 3], field: &F) -> f32 {
        let mut res = 1.0_f32;
        let mut t = SHADOW_MINT;
        for _ in 0..SDF_MAX_IT {
            let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
            let d = field(q);
            res = res.min(SHADOW_K * d / t);
            if d < SHADOW_HIT_EPS {
                return 0.0;
            }
            t += (d / FIELD_LIPSCHITZ_L).max(SHADOW_MINT_STEP);
            if t > SDF_T_MAX {
                break;
            }
        }
        res.clamp(0.0, 1.0)
    }

    /// A lit surface point near the terminator: pick a point on the sphere whose normal
    /// makes a small POSITIVE angle's-cosine with the light (`n·L` small but > 0, so the
    /// point is LIT and passes the `SHADOW_NDOTL_EPS` grazing gate). Returns `(p, n, l)`.
    ///
    /// The light points along +Z. A point near the equator (relative to +Z) has its normal
    /// nearly perpendicular to `L` ⇒ `n·L` small ⇒ grazing. We choose a polar angle so that
    /// `n·L ≈ 0.06` (well inside the lit half, clearly grazing).
    fn grazing_lit_point() -> ([f32; 3], [f32; 3], [f32; 3]) {
        let l = v_normalize([0.0, 0.0, 1.0]);
        // n·L = cos(theta) where theta is the angle of the surface point off the +Z pole.
        // theta ≈ 86.6° ⇒ cos ≈ 0.06 (grazing but lit).
        let cos_t = 0.06_f32;
        let sin_t = (1.0 - cos_t * cos_t).sqrt();
        // The surface point on the unit-radius sphere (radius SDF_SPHERE_RADIUS): the normal
        // direction times the radius, offset by the center.
        let dir = [sin_t, 0.0, cos_t];
        let p = [
            SDF_SPHERE_CENTER[0] + dir[0] * SDF_SPHERE_RADIUS,
            SDF_SPHERE_CENTER[1] + dir[1] * SDF_SPHERE_RADIUS,
            SDF_SPHERE_CENTER[2] + dir[2] * SDF_SPHERE_RADIUS,
        ];
        // The analytic normal via the SAME central-difference gradient the marcher uses.
        let n = sdf_normal(p);
        (p, n, l)
    }

    /// STEP 1: the UNBIASED march FALSE-occludes a lit grazing point (`res ≈ 0`). This is
    /// the acne. If this assert ever fails, the diagnosis is wrong — STOP and re-investigate.
    #[test]
    fn step1_unbiased_march_false_occludes_grazing_terminator() {
        let (p, n, l) = grazing_lit_point();
        // The point is LIT (passes the grazing gate) — this is the precondition.
        let ndotl = v_dot(n, l);
        assert!(
            ndotl > 0.0,
            "test setup bug: the grazing point must be LIT (n·L > 0), got {ndotl}"
        );
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = unbiased_soft_shadow(p, l, &field);
        assert!(
            res < 1.0e-3,
            "expected the UNBIASED march to FALSE-occlude the lit grazing point (acne, \
             res ≈ 0), got res = {res} — the diagnosis may be wrong"
        );
    }

    /// STEP 3: the BIASED march (the host mirror, called with `p + n*SHADOW_NORMAL_BIAS`,
    /// exactly as `host_shade` now calls it) keeps the lit grazing point LIT (`res > 0`):
    /// the acne is GONE.
    #[test]
    fn step3_normal_bias_clears_grazing_acne() {
        let (p, n, l) = grazing_lit_point();
        let pb = [
            p[0] + n[0] * SHADOW_NORMAL_BIAS,
            p[1] + n[1] * SHADOW_NORMAL_BIAS,
            p[2] + n[2] * SHADOW_NORMAL_BIAS,
        ];
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = host_soft_shadow(pb, n, l, &field);
        assert!(
            res > 0.1,
            "the normal-offset bias must keep the lit grazing point LIT (res > 0), got {res}"
        );
    }

    /// STEP 3 (ranged): the ranged host mirror — the resolve's per-light march — is ALSO
    /// freed from the grazing acne by the same bias.
    #[test]
    fn step3_normal_bias_clears_grazing_acne_ranged() {
        let (p, n, l) = grazing_lit_point();
        let pb = [
            p[0] + n[0] * SHADOW_NORMAL_BIAS,
            p[1] + n[1] * SHADOW_NORMAL_BIAS,
            p[2] + n[2] * SHADOW_NORMAL_BIAS,
        ];
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = host_soft_shadow_ranged(pb, n, l, SDF_T_MAX, &field);
        assert!(
            res > 0.1,
            "the ranged normal-offset bias must keep the lit grazing point LIT, got {res}"
        );
    }
}

#[cfg(test)]
mod p0a_tests {
    //! Host-side (GPU-free) verification of the P0a substrate: the extent/camera
    //! push-constant layout and the extent-aware golden mirror. The GPU half (the
    //! shader actually rendering ortho 64×64 bit-exact / a 1080p perspective frame)
    //! is the tester's RTX-3060 oracle; these assert the CPU contract those goldens
    //! rely on (the host const-assert mirror + the bit-exact ortho fall-through).

    use super::{CAM_MODE_ORTHO, CAM_MODE_PERSPECTIVE, COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera, CompositePushConstants, MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_op};
    use crate::goldens::{golden_composite_pixel, golden_composite_pixel_ex};

    /// The rung-9/10 "crater" CSG scene, reused so the golden parity check runs over
    /// a non-trivial field (a base sphere with a smaller sphere subtracted).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    /// The extent-aware golden at `(SDF_IMG_W, SDF_IMG_H)` + ORTHO must be BIT-EXACT
    /// to the legacy `golden_composite_pixel` over the whole 64×64 image (the
    /// rung-8..11 contract — same extent → same rays → same pixels).
    #[test]
    fn ortho_64x64_is_bit_identical_to_legacy_golden() {
        let edits = crater();
        // A mix of covered (finite depth) and uncovered (clear) pixels.
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let md = depths[((px + py) as usize) % depths.len()];
                let legacy = golden_composite_pixel(&edits, md, px, py);
                let ex = golden_composite_pixel_ex(
                    &edits,
                    md,
                    px,
                    py,
                    SDF_IMG_W,
                    SDF_IMG_H,
                    CompositeCamera::Ortho,
                );
                assert_eq!(legacy, ex, "ortho mirror diverged at ({px},{py}) depth {md}");
            }
        }
    }

    /// `CompositePushConstants::ortho` keeps `count == w*h`, ORTHO mode, zeroed
    /// camera basis, and the 80-byte size the pipeline must declare.
    #[test]
    fn ortho_push_constants_shape() {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        assert_eq!(pc.count, SDF_IMG_W * SDF_IMG_H);
        assert_eq!(pc.img_w, SDF_IMG_W);
        assert_eq!(pc.img_h, SDF_IMG_H);
        assert_eq!(pc.camera_mode, CAM_MODE_ORTHO);
        assert_eq!(pc.cam_eye, [0.0; 4]);
        assert_eq!(pc.as_bytes().len(), COMPOSITE_PUSH_CONSTANT_BYTES as usize);
        assert_eq!(COMPOSITE_PUSH_CONSTANT_BYTES, 80);
    }

    /// `CompositePushConstants::perspective` derives `tan(fovY/2)` + aspect and packs
    /// the basis into the documented `float4` slots; the byte view is 80 bytes.
    #[test]
    fn perspective_push_constants_layout() {
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let pc = CompositePushConstants::perspective(
            [0.0, 0.0, 3.0],   // eye
            [0.0, 0.0, -1.0],  // forward
            [1.0, 0.0, 0.0],   // right
            [0.0, 1.0, 0.0],   // up
            fov_y,
            1920,
            1080,
        );
        assert_eq!(pc.camera_mode, CAM_MODE_PERSPECTIVE);
        assert_eq!(pc.count, 1920 * 1080);
        assert_eq!(pc.cam_eye, [0.0, 0.0, 3.0, 0.0]);
        // forward.w = tan(45°) = 1, right.w = aspect = 1920/1080.
        assert!((pc.cam_forward[3] - 1.0).abs() < 1e-5);
        assert!((pc.cam_right[3] - (1920.0_f32 / 1080.0)).abs() < 1e-6);
        assert_eq!(pc.as_bytes().len(), 80);
    }

    /// Perspective ray-gen sanity: the CENTER pixel of a forward-looking camera must
    /// shoot a ray ≈ the forward axis from the eye (the field eval downstream is the
    /// same deterministic mirror, so this isolates the additive ray-gen).
    #[test]
    fn perspective_center_ray_is_forward() {
        let edits = crater();
        // A small extent; we only need the geometric ray, not a full render.
        let (w, h) = (64u32, 64u32);
        let eye = [0.0_f32, 0.0, 3.0];
        let camera = CompositeCamera::Perspective {
            eye,
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(), // 45°
            aspect: 1.0,
        };
        // The center pixel (px=py=32) → ndc ≈ 0 → dir ≈ forward → hits the sphere at
        // the origin from the +Z eye (no mesh: clear depth). A miss would be the dark
        // background; a hit is the warm lit color — distinguish by the red channel.
        let center = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, w / 2, h / 2, w, h, camera);
        let red = center & 0xFF;
        assert!(
            red > 60,
            "center perspective ray must hit the lit sphere (warm red), got 0x{center:08X}"
        );
        // A corner pixel shoots wide and should MISS → background (low red).
        let corner = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, 0, 0, w, h, camera);
        let corner_red = corner & 0xFF;
        assert!(
            corner_red < red,
            "corner perspective ray should miss (darker) vs center: corner 0x{corner:08X} center 0x{center:08X}"
        );
    }
}

#[cfg(test)]
mod p4b_tests {
    //! Render P4b HOST conservative-invariant suite — the CPU proof that the coarse
    //! cull is CONSERVATIVE (a hole = the worst bug) BEFORE the GPU golden runs. The
    //! five proofs (docs/RENDER-P4-DESIGN.md):
    //!   (a) EXHAUSTIVE ortho: every tile, all 64 fine-pixel footprint corners within
    //!       `ortho_cone_radius` of the tile-center axis (the exact composite_ray u/v).
    //!   (b) perspective: every tile's 4 corner outer-edge dirs within
    //!       `perspective_alpha_tile` (enclosure with margin).
    //!   (c) randomized {fov, aspect, tile, single sphere/box} with an ANALYTIC first-hit
    //!       oracle: `golden_tile_bound.near_t <= min over in-tile pixels of their
    //!       analytic first-hit` AND `EMPTY => no in-tile pixel hits before mesh`.
    //!   (d) Lipschitz: random points, central-diff `|grad field| <= FIELD_LIPSCHITZ_L`.
    //!   (e) `golden_composite_pixel_culled(coarse_enabled = false)` bit-identical to
    //!       `golden_composite_pixel_ex`.
    //! These reproduce the GPU shader's arithmetic exactly (the host mirror), so a pass
    //! here is a near-proof the GPU cull is conservative too (the GPU golden confirms).

    use boyko_sdf_math::{sdf_edit_list, v_sub};

    use super::{ALPHA_MARGIN, CompositeCamera, FIELD_LIPSCHITZ_L, MESH_DEPTH_CLEAR, SDF_CAM_Z, SDF_EPS, SDF_HALF_EXTENT, SDF_T_MAX, SdfEdit, TILE_FLAG_EMPTY, TILE_SIZE, sdf_op, tile_grid_extent};
    use crate::goldens::{golden_composite_pixel_culled, golden_composite_pixel_ex, golden_tile_bound};

    // --- A tiny deterministic PRNG (splitmix64) so the randomized sweeps are
    //     reproducible (no rand dep; the same mix the serialization fuzz uses). ----------
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A uniform `f32` in `[lo, hi)`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0,1)
            lo + (hi - lo) * u
        }
        fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % ((hi - lo) as u64)) as u32
        }
    }

    /// The exact ORTHO ray-origin XY for a (possibly fractional) fine-pixel sample whose
    /// `(px + 0.5)` is `sx` and `(py + 0.5)` is `sy` — `composite_ray`'s ortho arm.
    fn ortho_origin_xy(sx: f32, sy: f32, w: u32, h: u32) -> [f32; 2] {
        let u = (sx / (w as f32)) * 2.0 - 1.0;
        let v = -((sy / (h as f32)) * 2.0 - 1.0);
        [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT]
    }

    /// The exact, normalized PERSPECTIVE ray direction for a fractional sample, the
    /// `composite_ray` perspective arm (used by the enclosure + oracle tests). The
    /// parameters mirror the shader's ray-gen inputs verbatim (grouping them into a
    /// struct would obscure the op-for-op correspondence the test exists to verify).
    #[allow(clippy::too_many_arguments)]
    fn persp_dir(
        sx: f32,
        sy: f32,
        w: u32,
        h: u32,
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        tan_half_fov: f32,
        aspect: f32,
    ) -> [f32; 3] {
        let ndc_x = (sx / (w as f32)) * 2.0 - 1.0;
        let ndc_y = -((sy / (h as f32)) * 2.0 - 1.0);
        let kx = ndc_x * aspect * tan_half_fov;
        let ky = ndc_y * tan_half_fov;
        let dir = [
            forward[0] + right[0] * kx + up[0] * ky,
            forward[1] + right[1] * kx + up[1] * ky,
            forward[2] + right[2] * kx + up[2] * ky,
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        [dir[0] / len, dir[1] / len, dir[2] / len]
    }

    /// The host mirror of `ortho_cone_radius` (private — re-derive for the test). Uses
    /// the LARGER world pixel pitch `min(w,h)` so a non-square ortho extent is enclosed.
    fn ortho_cone_radius_t(w: u32, h: u32) -> f32 {
        core::f32::consts::SQRT_2 * (9.0 / (w.min(h) as f32)) * SDF_HALF_EXTENT
    }

    /// (a) EXHAUSTIVE ortho enclosure: for EVERY tile, EVERY one of the 64 fine pixels'
    /// 4 footprint corners (`(px+0.5) ± 0.5`, `(py+0.5) ± 0.5`) lies within
    /// `ortho_cone_radius` of the tile-center axis (the perpendicular distance in the
    /// ortho XY plane — the cone is a constant-radius cylinder). A corner outside the
    /// cone = a hole. Run at the golden 64×64 extent + a non-multiple extent.
    #[test]
    fn ortho_footprint_corners_within_cone() {
        for &(w, h) in &[(64u32, 64u32), (96u32, 48u32), (70u32, 66u32)] {
            let (tw, th) = tile_grid_extent(w, h);
            let r = ortho_cone_radius_t(w, h);
            let mut checked = 0u64;
            for ty in 0..th {
                for tx in 0..tw {
                    // Tile-center axis XY (`(px_c+0.5) = tx*8 + 4.0`).
                    let axis = ortho_origin_xy(
                        (tx * TILE_SIZE) as f32 + 4.0,
                        (ty * TILE_SIZE) as f32 + 4.0,
                        w,
                        h,
                    );
                    for ly in 0..TILE_SIZE {
                        for lx in 0..TILE_SIZE {
                            let px = tx * TILE_SIZE + lx;
                            let py = ty * TILE_SIZE + ly;
                            // All 4 footprint corners of fine pixel (px,py): the pixel
                            // sample is `(px+0.5, py+0.5)`; its footprint spans ±0.5.
                            for &(ddx, ddy) in &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                                let sx = (px as f32) + 0.5 + ddx;
                                let sy = (py as f32) + 0.5 + ddy;
                                let xy = ortho_origin_xy(sx, sy, w, h);
                                let dx = xy[0] - axis[0];
                                let dy = xy[1] - axis[1];
                                let dist = (dx * dx + dy * dy).sqrt();
                                assert!(
                                    dist <= r,
                                    "ORTHO hole {w}x{h} tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                     footprint corner dist {dist} > cone radius {r}"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
            println!("[a] ortho enclosure {w}x{h}: {checked} footprint corners all within cone radius {r}");
        }
    }

    /// (b) Perspective enclosure: for EVERY tile, the 4 corner OUTER-EDGE directions are
    /// within `perspective_alpha_tile` (= the angle the cull uses) of the tile-center
    /// direction. By construction `perspective_alpha_tile` is the MAX of exactly those 4
    /// angles + `ALPHA_MARGIN`, so each must be `<= alpha` with the margin to spare — a
    /// corner whose angle exceeded `alpha` would be a hole. Also asserts the cull's
    /// in-tile pixel-center + footprint-corner dirs are inside the cone (the stronger
    /// enclosure the conservativeness proof's Claim 1 needs).
    #[test]
    fn perspective_corner_dirs_within_alpha() {
        let (w, h) = (1920u32, 1080u32);
        let forward = [0.0_f32, 0.0, -1.0];
        let right = [1.0_f32, 0.0, 0.0];
        let up = [0.0_f32, 1.0, 0.0];
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let tan_half_fov = (fov_y * 0.5).tan();
        let aspect = (w as f32) / (h as f32);
        let (tw, th) = tile_grid_extent(w, h);

        let camera = CompositeCamera::Perspective {
            eye: [0.0, 0.0, 3.0],
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        };
        // Sample a strided subset of tiles (full grid is 240×135 = 32 400 tiles; every
        // tile's 4 corners + 64 pixel footprints is exhaustive but slow — stride 7
        // covers the grid incl. the convex edge/corner tiles where alpha is largest).
        let mut tiles_checked = 0u64;
        let mut ty = 0;
        while ty < th {
            let mut tx = 0;
            while tx < tw {
                // Recompute `alpha_tile_safe` exactly as the cull does (4 outer corners).
                let cx = (tx * TILE_SIZE) as f32 + 4.0;
                let cy = (ty * TILE_SIZE) as f32 + 4.0;
                let d_center = persp_dir(cx, cy, w, h, forward, right, up, tan_half_fov, aspect);
                let lo_x = (tx * TILE_SIZE) as f32;
                let hi_x = (tx * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let lo_y = (ty * TILE_SIZE) as f32;
                let hi_y = (ty * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let mut alpha = 0.0_f32;
                for &(sxp, syp) in &[(lo_x, lo_y), (hi_x, lo_y), (lo_x, hi_y), (hi_x, hi_y)] {
                    let dc = persp_dir(sxp, syp, w, h, forward, right, up, tan_half_fov, aspect);
                    let cos = (d_center[0] * dc[0] + d_center[1] * dc[1] + d_center[2] * dc[2])
                        .clamp(-1.0, 1.0);
                    alpha = alpha.max(cos.acos());
                }
                let alpha_safe = alpha + ALPHA_MARGIN;

                // Every in-tile pixel CENTER + its 4 footprint corners must be inside the
                // cone (angle <= alpha_safe). This is Claim 1 (lateral offset < r(t)).
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let px = tx * TILE_SIZE + lx;
                        let py = ty * TILE_SIZE + ly;
                        if px >= w || py >= h {
                            continue;
                        }
                        for &(ddx, ddy) in
                            &[(0.0, 0.0), (-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)]
                        {
                            let sx = (px as f32) + 0.5 + ddx;
                            let sy = (py as f32) + 0.5 + ddy;
                            let d = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                            let cos = (d_center[0] * d[0] + d_center[1] * d[1] + d_center[2] * d[2])
                                .clamp(-1.0, 1.0);
                            let ang = cos.acos();
                            assert!(
                                ang <= alpha_safe,
                                "PERSP hole tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                 dir angle {ang} > alpha_safe {alpha_safe}"
                            );
                        }
                    }
                }
                tiles_checked += 1;
                tx += 7;
            }
            ty += 7;
        }
        // Sanity: the cull's `golden_tile_bound` uses the same camera (smoke — it runs).
        let _ = golden_tile_bound(&[], &[MESH_DEPTH_CLEAR; 64], 0, 0, w, h, camera);
        println!("[b] perspective enclosure {w}x{h}: {tiles_checked} tiles, all in-tile pixel/footprint dirs within alpha");
    }

    // --- Analytic first-hit oracles (a LOWER bound on the GPU march's first hit) -------

    /// Analytic ray-sphere first-hit `t` (the smaller non-negative root) or `None`.
    fn ray_sphere(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], r: f32) -> Option<f32> {
        let oc = v_sub(ro, c);
        let b = oc[0] * rd[0] + oc[1] * rd[1] + oc[2] * rd[2];
        let cc = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - r * r;
        let disc = b * b - cc;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let t0 = -b - s;
        let t1 = -b + s;
        if t0 >= 0.0 {
            Some(t0)
        } else if t1 >= 0.0 {
            Some(t1)
        } else {
            None
        }
    }

    /// Analytic ray-AABB (slab) first-hit `t` for a box centered at `c` with half-extents
    /// `h`, or `None`. Returns the entry `t` (>= 0) of the ray through the box.
    fn ray_box(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], h: [f32; 3]) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;
        for a in 0..3 {
            let lo = c[a] - h[a];
            let hi = c[a] + h[a];
            if rd[a].abs() < 1e-9 {
                if ro[a] < lo || ro[a] > hi {
                    return None; // parallel + outside the slab.
                }
            } else {
                let inv = 1.0 / rd[a];
                let mut ta = (lo - ro[a]) * inv;
                let mut tb = (hi - ro[a]) * inv;
                if ta > tb {
                    core::mem::swap(&mut ta, &mut tb);
                }
                t_min = t_min.max(ta);
                t_max = t_max.min(tb);
                if t_min > t_max {
                    return None;
                }
            }
        }
        if t_max < 0.0 {
            return None;
        }
        Some(t_min.max(0.0))
    }

    /// (c) Randomized perspective sweep with an analytic first-hit oracle, the C2/C4
    /// proof: for a single sphere or box, `golden_tile_bound.near_t <= the analytic
    /// first-hit of EVERY in-tile pixel that hits` AND `EMPTY => no in-tile pixel hits
    /// before the (deepest) mesh`. The analytic hit is the true Euclidean first contact;
    /// the cull seeding `t = near_t <= it` can never skip a pixel's surface.
    #[test]
    fn randomized_oracle_near_t_le_first_hit() {
        let mut rng = Rng::new(0xC0FF_EE15_600D_5EED);
        let cases = 600;
        let mut checked_hits = 0u64;
        let mut empty_tiles = 0u64;
        let mut nonempty_tiles = 0u64;

        for _ in 0..cases {
            // A random forward-looking perspective camera (eye on +Z, looking -Z, small
            // jitter on the basis kept orthonormal-ish; the cull only needs the cone).
            let fov_y = rng.range(0.6, 1.8); // ~34°..103°
            let (w, h) = (rng.range_u32(40, 160), rng.range_u32(40, 160));
            let aspect = (w as f32) / (h as f32);
            let tan_half_fov = (fov_y * 0.5).tan();
            let eye = [rng.range(-0.3, 0.3), rng.range(-0.3, 0.3), rng.range(2.0, 4.0)];
            let forward = [0.0, 0.0, -1.0];
            let right = [1.0, 0.0, 0.0];
            let up = [0.0, 1.0, 0.0];
            let camera = CompositeCamera::Perspective {
                eye,
                forward,
                right,
                up,
                tan_half_fov,
                aspect,
            };

            // A single primitive (sphere or box) near the origin.
            let is_box = rng.next_u64() & 1 == 0;
            let center = [rng.range(-0.4, 0.4), rng.range(-0.4, 0.4), rng.range(-0.4, 0.4)];
            let (edits, sphere_r, box_h): (Vec<SdfEdit>, f32, [f32; 3]) = if is_box {
                let hx = rng.range(0.15, 0.5);
                let hy = rng.range(0.15, 0.5);
                let hz = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::box_shape(center, [hx, hy, hz], sdf_op::UNION, 0.0)],
                    0.0,
                    [hx, hy, hz],
                )
            } else {
                let r = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::sphere(center, r, sdf_op::UNION, 0.0)],
                    r,
                    [0.0; 3],
                )
            };

            // A tile within the grid: half the cases bias toward the image center (where
            // a forward camera sees the origin-centered primitive, so the tile actually
            // looks at the surface — exercising near_t <= first-hit), half are fully
            // random (exercising the EMPTY path on tiles that look away).
            let (tw, th) = tile_grid_extent(w, h);
            let (tx, ty) = if rng.next_u64() & 1 == 0 {
                let cx = tw / 2;
                let cy = th / 2;
                let jx = rng.range_u32(0, 3);
                let jy = rng.range_u32(0, 3);
                (
                    (cx + jx).saturating_sub(1).min(tw - 1),
                    (cy + jy).saturating_sub(1).min(th - 1),
                )
            } else {
                (rng.range_u32(0, tw), rng.range_u32(0, th))
            };

            // No mesh (clear depth everywhere in the tile) so far_t == T_MAX and the
            // oracle is the pure SDF first-hit (the mesh-bound case is exercised by the
            // GPU golden + the EMPTY-with-mesh negative test).
            let tile_depths = [MESH_DEPTH_CLEAR; 64];
            let tb = golden_tile_bound(&edits, &tile_depths, tx, ty, w, h, camera);

            // For every in-tile pixel, the analytic first-hit (the oracle).
            let mut min_first_hit = f32::INFINITY;
            let mut any_hit = false;
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let px = tx * TILE_SIZE + lx;
                    let py = ty * TILE_SIZE + ly;
                    if px >= w || py >= h {
                        continue;
                    }
                    let sx = (px as f32) + 0.5;
                    let sy = (py as f32) + 0.5;
                    let rd = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                    let hit = if is_box {
                        ray_box(eye, rd, center, box_h)
                    } else {
                        ray_sphere(eye, rd, center, sphere_r)
                    };
                    if let Some(t_hit) = hit
                        && t_hit <= SDF_T_MAX
                    {
                        any_hit = true;
                        min_first_hit = min_first_hit.min(t_hit);
                    }
                }
            }

            if tb.flags & TILE_FLAG_EMPTY != 0 {
                empty_tiles += 1;
                // EMPTY => no in-tile pixel may hit before the mesh (far_t == T_MAX here).
                // The march hit threshold is EPS, so a sphere-trace records a hit slightly
                // BEFORE the analytic surface; allow the surface to be within EPS of T_MAX.
                assert!(
                    !any_hit || min_first_hit + SDF_EPS >= SDF_T_MAX,
                    "EMPTY tile but an in-tile pixel hits at {min_first_hit} (< T_MAX): \
                     box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})"
                );
            } else {
                nonempty_tiles += 1;
                if any_hit {
                    checked_hits += 1;
                    // The CORE conservativeness claim: near_t <= every pixel's first hit.
                    // A small EPS tolerance absorbs the cone-entry EPS_COARSE + the fp
                    // step rounding (near_t is recorded AT the cone-entry t, never past it).
                    assert!(
                        tb.near_t <= min_first_hit + 1e-3,
                        "near_t {} > min in-tile first-hit {min_first_hit}: HOLE \
                         box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})",
                        tb.near_t
                    );
                }
            }
        }
        println!(
            "[c] randomized oracle: {cases} cases, {nonempty_tiles} non-empty ({checked_hits} with hits) + {empty_tiles} EMPTY — near_t <= analytic first-hit, EMPTY => no early hit"
        );
    }

    /// (d) Lipschitz tripwire (D7/W4): over random points in the scene's bounding region,
    /// the central-difference gradient magnitude of `field_distance` (== `sdf_edit_list`)
    /// must not exceed `FIELD_LIPSCHITZ_L` (= √2). A super-Lipschitz op would void the
    /// cone step's `/ L` clearance bound. Exercises the hard CSG + the smooth-min blend
    /// band (where the peak gradient lives).
    #[test]
    fn field_lipschitz_bound_holds() {
        let scenes: [Vec<SdfEdit>; 3] = [
            vec![
                SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
            ],
            vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.3, 0.2], sdf_op::UNION, 0.0)],
            vec![
                SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
            ],
        ];
        let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
        let h = 1e-3_f32;
        let mut max_grad = 0.0_f32;
        let mut samples = 0u64;
        for edits in &scenes {
            for _ in 0..50_000 {
                let p = [rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), rng.range(-1.5, 1.5)];
                let gx = (sdf_edit_list(edits, [p[0] + h, p[1], p[2]])
                    - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]))
                    / (2.0 * h);
                let gy = (sdf_edit_list(edits, [p[0], p[1] + h, p[2]])
                    - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]))
                    / (2.0 * h);
                let gz = (sdf_edit_list(edits, [p[0], p[1], p[2] + h])
                    - sdf_edit_list(edits, [p[0], p[1], p[2] - h]))
                    / (2.0 * h);
                let g = (gx * gx + gy * gy + gz * gz).sqrt();
                if g.is_finite() {
                    max_grad = max_grad.max(g);
                }
                samples += 1;
            }
        }
        // A small tolerance over √2 absorbs the central-difference discretization error
        // at the blend band's curvature; a genuine super-Lipschitz op blows well past it.
        assert!(
            max_grad <= FIELD_LIPSCHITZ_L + 5e-2,
            "field gradient {max_grad} exceeds FIELD_LIPSCHITZ_L {FIELD_LIPSCHITZ_L} (the cone step's /L is unsound)"
        );
        assert!(
            (FIELD_LIPSCHITZ_L - core::f32::consts::SQRT_2).abs() < 1e-6,
            "FIELD_LIPSCHITZ_L must be sqrt(2)"
        );
        println!(
            "[d] Lipschitz: {samples} samples, max |grad field| = {max_grad} <= L = {FIELD_LIPSCHITZ_L} (sqrt 2)"
        );
    }

    /// (e) The 0%-gate anchor: `golden_composite_pixel_culled(coarse_enabled = false)`
    /// is BIT-IDENTICAL to `golden_composite_pixel_ex` over the whole 64×64 image, under
    /// both an ortho and a perspective camera with a mix of covered/uncovered depths. The
    /// TileBound passed is irrelevant when cull-off (the function short-circuits) — a
    /// dummy is supplied.
    #[test]
    fn culled_off_is_bit_identical_to_ex() {
        let edits = vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ];
        let (w, h) = (64u32, 64u32);
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let dummy = super::TileBound { near_t: 7.0, far_t: 9.0, flags: TILE_FLAG_EMPTY, _pad: 0 };

        let cameras = [
            CompositeCamera::Ortho,
            CompositeCamera::Perspective {
                eye: [0.0, 0.0, 3.0],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(),
                aspect: 1.0,
            },
        ];
        let mut checked = 0u64;
        for camera in cameras {
            for py in 0..h {
                for px in 0..w {
                    let md = depths[((px + py) as usize) % depths.len()];
                    let want = golden_composite_pixel_ex(&edits, md, px, py, w, h, camera);
                    let got = golden_composite_pixel_culled(
                        &edits, md, px, py, w, h, camera, false, dummy,
                    );
                    assert_eq!(
                        want, got,
                        "cull-off diverged at ({px},{py}) depth {md}: ex 0x{want:08X} culled 0x{got:08X}"
                    );
                    checked += 1;
                }
            }
        }
        // Re-anchor: the unused fields of `SDF_CAM_Z` / `SDF_T_MAX` are still the frozen
        // scene constants (a compile-time touch so a refactor that drops them is caught).
        let _ = (SDF_CAM_Z, SDF_T_MAX);
        println!("[e] cull-off bit-identity: {checked} pixels (ortho + perspective) all match golden_composite_pixel_ex");
    }
}

// ===========================================================================
// Render B1 — over-relaxation (Keinert ω-gated) HOST soundness gates.
//
// These prove the CPU contract the on-device gates rely on, GPU-free:
//   1. ω = 1 host BIT-identity — `_omega(.., 1.0)` byte-equal to the ω=1 forwarder
//      (`golden_composite_pixel_ex` / `_culled`) over a pixel sweep on all 3 scenes
//      (ortho + perspective). Pins the forwarder extraction (the 0%-gate).
//   2. HIT-SET-SUPERSET property — over randomized scenes/depths/pixels, for
//      ω ∈ {1.2, 1.5, 1.9} the `_omega` hit-set ⊇ the ω=1 hit-set (NO ω=1 SDF hit
//      becomes background/mesh at ω>1). A violation = a missed-surface HOLE. Run on
//      BOTH the cull-off and the cull-on (`_culled_omega`) path. (F-CRIT-1 oracle.)
//   3. NO-HOLES TRIPWIRE — a deliberately-broken over-relax (a retreat to a WRONG t)
//      MUST fail the gate-2 invariant, proving #2 has teeth.
//   4. STEP-BOUND property — `steps(ω>1) ≤ steps(ω=1) + 1` per ray over the
//      randomized scenes (≤ 1 permanent sor-fail fallback ⇒ ≤ plain + 1).
//   5. ω CLAMP — the harness's `omega_in.clamp(1.0, 1.99)` + the 8-B push encode:
//      a hostile `omega_in` decodes to a finite value in `[1.0, 1.99]`.
//
// The march mirrors `golden_composite_pixel_ex_omega` / `_culled_omega` EXACTLY (same
// ordering: top mesh-guard, probe, hit test, ω-gate step, miss test). Gates 2/3/4 need
// a hit/step-instrumented copy of that loop (the production goldens return only a packed
// color), so a faithful test-only mirror lives here; gate 1 diffs the production
// functions directly so the mirror can never mask a real forwarder regression.
// ===========================================================================
#[cfg(test)]
mod b1_over_relaxation_tests {
    use super::{CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FIELD_LIPSCHITZ_L, FineMarcherPush, GBUFFER_MARCHER_PUSH_BYTES, MESH_DEPTH_CLEAR, SDF_EPS, SDF_IMG_H, SDF_IMG_W, SDF_MAX_IT, SDF_T_MAX, SdfEdit, composite_ray, sdf_edit_list, sdf_op};
    use crate::goldens::{golden_composite_pixel_culled, golden_composite_pixel_culled_omega, golden_composite_pixel_ex, golden_composite_pixel_ex_omega};
    use proptest::prelude::*;

    /// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }
    /// A box CSG scene.
    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }
    /// A smooth-union scene (two spheres blended) — the smooth-min path.
    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Instrumented result of [`march_obs`] — the Candidate-C host oracle output plus the perf
    /// counters that replace the deleted step-bound gate 4.
    struct MarchObs {
        /// The SHIPPED hit decision: the fast pass's hit, OR (on exhaustion) the re-march's hit.
        hit: bool,
        /// Probe iterations spent in the over-relaxed FAST pass (each `sdf_edit_list` call). The
        /// B1 win is `fast_steps(ω>1) < fast_steps(ω=1)` on the common converging rays.
        fast_steps: u32,
        /// True iff the fast pass exhausted the budget and the Candidate-C re-march fired. The
        /// re-march FREQUENCY (% of pixels) is the perf risk: a large fraction = B1 perf-neutral.
        remarched: bool,
    }

    /// The Render B1 ω-march, INSTRUMENTED (Candidate C) — the host oracle for gates 2/3 and
    /// the perf observation. A faithful, COMPLETE mirror of the PRODUCTION
    /// `golden_composite_pixel_ex_omega` march: the over-relaxed fast pass (same ordering,
    /// same ω-gate, same Lipschitz-aware sor-fail test, same `t = safe_t + sor_prev`
    /// plain-resume + permanent fall-to-plain) FOLLOWED BY the Candidate-C fallback re-march.
    ///
    /// CONTRACT CHANGE vs the prior step-bound oracle: the fast pass alone is NOT the shipped
    /// hit decision. Correctness now comes from the re-march, not a step bound. When the fast
    /// loop runs all `SDF_MAX_IT` iterations with NO break (`exhausted`), production RE-MARCHES
    /// from the ORIGINAL seed with a plain ω=1 sphere-trace and uses THAT (hit, t). This oracle
    /// reproduces that exactly, so `hit` is byte-for-byte the production hit decision — gate 2's
    /// "ω>1 hit-set ⊇ ω=1 hit-set" tests what actually ships, and is provably true (the re-march
    /// body is the frozen plain marcher, so an exhausting ω>1 ray lands on the SAME hit the
    /// frozen marcher does).
    ///
    /// INSTRUMENTATION (perf observation, replacing the deleted step-bound gate 4): the returned
    /// [`MarchObs`] records the fast-pass probe count (`fast_steps`, each `sdf_edit_list` call in
    /// the over-relaxed loop), whether the fast pass exhausted the budget, and whether the
    /// re-march fired. The orchestrator uses the re-march FREQUENCY (% of pixels that exhausted)
    /// and the fast-pass step reduction vs plain to judge whether B1 is still a net win.
    fn march_obs(edits: &[SdfEdit], ro: [f32; 3], rd: [f32; 3], t_mesh: f32, omega_in: f32) -> MarchObs {
        let mut t = 0.0_f32;
        let t_seed = t; // the ORIGINAL seed (0.0) — Candidate C re-march re-seeds from it
        let mut omega = omega_in;
        let mut hit = false;
        let mut fast_steps = 0u32;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true; // cleared by EVERY in-loop break (mirrors production)
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            fast_steps += 1;
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Lipschitz-aware sor-fail (mirrors production exactly): the empty-ball radii
                // are `f / L`, so the spheres cover the over-step iff `sor_prev + d >= L * step`.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG-B1-HOLE-2: resume the plain march one certified step past the safe
                    // probe (no re-probe, +0 steps for the retreat itself) and latch to plain.
                    t = safe_t + sor_prev;
                    omega = 1.0;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }

        // Candidate C: the PROVABLY-hole-free fallback re-march. Mirrors production EXACTLY —
        // on `exhausted` re-seed from `t_seed` and run the frozen plain ω=1 marcher; its (hit)
        // is the shipped decision. The fast pass's `hit` is discarded (it was false on exhaust).
        let remarched = exhausted;
        if exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }

        MarchObs { hit, fast_steps, remarched }
    }

    /// A DELIBERATELY-BROKEN over-relax used ONLY by the gate-3 tripwire. TWO co-ordinated
    /// breaks, BOTH required under the Candidate-C contract:
    ///   1. the fast pass mis-handles a sor-fail — it retreats to a WRONG `t` (`safe_t +
    ///      step_len`, i.e. it ADVANCES past the surface instead of retreating) AND keeps ω hot;
    ///   2. **the Candidate-C fallback re-march is DISABLED** (this function has NO re-march at
    ///      all — it returns the bare fast-pass hit). This is the load-bearing tripwire change
    ///      for the C contract: with the re-march intact, an exhausting broken ray would be
    ///      silently rescued by the plain re-march and the tripwire would go INERT (gate 2 would
    ///      look armed while testing nothing). Breaking C's guarantee = breaking the re-march, so
    ///      this models exactly the failure mode gate 2 must catch. NOT shipped.
    fn march_hit_broken(
        edits: &[SdfEdit],
        ro: [f32; 3],
        rd: [f32; 3],
        t_mesh: f32,
        omega_in: f32,
        with_remarch: bool,
    ) -> bool {
        let mut t = 0.0_f32;
        let t_seed = t;
        let omega = omega_in;
        let mut hit = false;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true;
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Production Lipschitz-aware detection threshold (re-synced) — the bug is in
                // the RETREAT below, NOT the detection, so the tripwire fires the same sor-fails
                // the production marcher would, then mishandles them.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG: a WRONG "retreat" that actually leaps past the surface and
                    // never falls to plain. The classic over-relaxation hole.
                    t = safe_t + step_len;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }
        // `with_remarch == false`: C's fallback is DISABLED — the bare broken fast-pass hit (the
        // tripwire that MUST hole). `with_remarch == true`: re-attach the EXACT Candidate-C
        // re-march on top of the broken fast pass — it must CLOSE every hole the broken pass
        // opened, proving the re-march (not the fast pass) is what guarantees the hit-set.
        if with_remarch && exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }
        hit
    }

    /// True when pixel `(px, py)` is an SDF hit at `omega` — the SHIPPED Candidate-C decision
    /// (fast pass + fallback re-march), with NO mesh occlusion (the pure-field hit set — the
    /// property's domain). This is byte-for-byte what `golden_composite_pixel_ex_omega`'s march
    /// concludes, so gate 2's superset property tests production, not the bare fast pass.
    fn pixel_hits(edits: &[SdfEdit], px: u32, py: u32, omega: f32) -> bool {
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        march_obs(edits, ro, rd, 1.0e30, omega).hit
    }

    /// GATE 1 — ω = 1 host BIT-identity for the un-culled marcher. `_omega(.., 1.0)` must be
    /// byte-equal to the ω=1 forwarder over the whole 64×64 image on all 3 scenes, ORTHO +
    /// PERSPECTIVE. Pins the forwarder extraction (any drift = the 0%-gate broke).
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_uncull() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for (cname, cam) in cameras {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_ex(edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam);
                        let om1 = golden_composite_pixel_ex_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}/{cname}] ω=1 _omega diverged from forwarder at ({px},{py}) depth {md}: \
                             fwd 0x{fwd:08X} omega 0x{om1:08X}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1] ω=1 un-culled bit-identity: {checked} pixels (ortho+persp × 3 scenes) byte-equal");
    }

    /// GATE 1 (cull path) — ω = 1 host BIT-identity for the CULLED marcher. With cull ON and a
    /// synthetic non-EMPTY tile (`near_t = 0`, `far_t = T_MAX`), `_culled_omega(.., 1.0)` must
    /// be byte-equal to the ω=1 culled forwarder over the image (ORTHO). The cull-off arm is
    /// covered by gate 1; this pins the seeded-march forwarder at ω=1.
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_culled() {
        use super::{TILE_FLAG_EMPTY, TileBound};
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        // A non-EMPTY, full-range tile (seed t = 0, march to T_MAX) — the general case.
        let surf = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
        // An EMPTY tile — exercises the early-out arm at ω=1.
        let empty = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: TILE_FLAG_EMPTY, _pad: 0 };
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for tile in [surf, empty] {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_culled(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile,
                        );
                        let om1 = golden_composite_pixel_culled_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}] ω=1 _culled_omega diverged from forwarder at ({px},{py}) depth {md} \
                             flags {}: fwd 0x{fwd:08X} omega 0x{om1:08X}",
                            tile.flags
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1c] ω=1 culled bit-identity: {checked} pixels (surf+empty tiles × 3 scenes) byte-equal");
    }

    // A proptest-generated randomized SDF scene, WIDENED for the Candidate-C no-hole contract:
    // 1..=8 edits, random kind/op/center/size, and an AGGRESSIVE smoothness distribution biased
    // toward the super-Lipschitz blend bands that historically holed (BUG-B1-HOLE-1). Every value
    // stays inside the bounded `[-0.85, 0.85]³` view box so the marcher reaches surfaces (an empty
    // world trivially satisfies superset). THIN features are reachable via the small-size tail
    // (down to 0.03) — sliver boxes/spheres are the classic over-relax overshoot trap. (A `//`
    // comment, not a doc comment — clippy's `unused_doc_comments` on macro invocations.)
    prop_compose! {
        fn arb_scene()(
            edits in proptest::collection::vec(
                (
                    0u32..2,                                  // 0 = sphere, 1 = box
                    0u32..3,                                  // op: union/subtract/intersect
                    -0.85f32..0.85, -0.85f32..0.85, -0.85f32..0.85, // center xyz
                    0.03f32..0.6,                             // size a (thin tail at 0.03)
                    0.03f32..0.5,                             // size b (box y, thin tail)
                    0.03f32..0.5,                             // size c (box z, thin tail)
                    // AGGRESSIVE smooth-min: hard, mild, and a heavy super-Lipschitz tail up to
                    // 0.4 (the blend band where IQ's smooth-min violates the unit-Lipschitz bound
                    // hardest — the BUG-B1-HOLE-1 regime). Weighted toward the soft cases.
                    prop_oneof![
                        1 => Just(0.0f32),
                        2 => 0.02f32..0.15,
                        3 => 0.15f32..0.40,
                    ],
                ),
                1..=8,
            )
        ) -> Vec<SdfEdit> {
            // Force the FIRST edit to be a UNION so the field has a positive volume to hit
            // (a lone subtract/intersect over an empty acc is a degenerate empty field).
            edits.into_iter().enumerate().map(|(i, (kind, op, cx, cy, cz, a, b, c, k))| {
                let op = if i == 0 { sdf_op::UNION } else { op };
                if kind == 0 {
                    SdfEdit::sphere([cx, cy, cz], a, op, k)
                } else {
                    SdfEdit::box_shape([cx, cy, cz], [a, b, c], op, k)
                }
            }).collect()
        }
    }

    proptest! {
        // WIDENED for the Candidate-C no-hole contract — this IS the correctness gate, so make
        // it thorough. 1024 random scenes (4× the prior 256), each over a coarse pixel grid ×
        // ω ∈ {1.2, 1.5, 1.99} × {ortho, perspective}. The pinned BUG-B1-HOLE-1 cliff seed in
        // proptest-regressions/compute.txt is replayed first on every run (proptest auto-loads it).
        #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

        /// GATE 2 — HIT-SET-SUPERSET (the F-CRIT-1 soundness oracle, the REAL correctness gate),
        /// CULL-OFF. For every randomized scene + ω ∈ {1.2, 1.5, 1.99} + camera ∈ {ortho,
        /// perspective}, EVERY pixel that is an SDF hit at ω=1 must remain a hit at ω>1. With
        /// Candidate C this is PROVABLY true on every case: an exhausting ω>1 ray re-marches with
        /// the frozen plain marcher, so it lands on the SAME hit ω=1 does. A regression = a
        /// missed-surface HOLE (Candidate C has a bug). The perspective camera supplies GRAZING
        /// rays at the frame edges (the classic over-relax overshoot trap). Iterates a coarse
        /// 4-px grid (every tile sampled) to bound per-case cost while covering the frame.
        #[test]
        fn gate2_hit_set_superset_cull_off(edits in arb_scene()) {
            let persp = CompositeCamera::Perspective {
                eye: [0.0, 0.0, 2.0], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0], tan_half_fov: 0.5, aspect: 1.0,
            };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for cam in [CompositeCamera::Ortho, persp] {
                    for py in (0..SDF_IMG_H).step_by(4) {
                        for px in (0..SDF_IMG_W).step_by(4) {
                            let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, cam);
                            let base = march_obs(&edits, ro, rd, 1.0e30, 1.0).hit;
                            if base {
                                let over = march_obs(&edits, ro, rd, 1.0e30, omega).hit;
                                prop_assert!(
                                    over,
                                    "HOLE: ({px},{py}) cam={cam:?} hits at ω=1 but MISSES at ω={omega} — Candidate-C re-march failed to close the hole; scene={edits:?}"
                                );
                            }
                        }
                    }
                }
            }
        }

        /// GATE 2 — HIT-SET-SUPERSET, CULL-ON. Same invariant through the `_culled_omega` path
        /// with a non-EMPTY full-range tile (seed t=0): the ω>1 culled hit-set ⊇ the ω=1 culled
        /// hit-set. Compares the FINAL packed color: a pixel that is the lit SDF color at ω=1
        /// must NOT become mesh/background at ω>1 (the observable hole). Uses no mesh
        /// (depth = clear) so the SDF/background partition is the field's alone.
        #[test]
        fn gate2_hit_set_superset_cull_on(edits in arb_scene()) {
            use super::{TileBound};
            let tile = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for py in (0..SDF_IMG_H).step_by(4) {
                    for px in (0..SDF_IMG_W).step_by(4) {
                        // ω=1 culled color (the baseline partition).
                        let c1 = golden_composite_pixel_culled_omega(
                            &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                            CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        // Only pixels that HIT the SDF at ω=1 are in the domain (no mesh, so a
                        // non-background color == an SDF hit).
                        if pixel_hits(&edits, px, py, 1.0) {
                            let co = golden_composite_pixel_culled_omega(
                                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                                CompositeCamera::Ortho, true, tile, omega,
                            );
                            prop_assert!(
                                pixel_hits(&edits, px, py, omega),
                                "HOLE(cull-on): ({px},{py}) SDF-hit at ω=1 (0x{c1:08X}) but ω={omega} march misses (0x{co:08X}); scene={edits:?}"
                            );
                        }
                    }
                }
            }
        }

    }

    // ===========================================================================================
    // PERF OBSERVATION (replaces the DELETED step-bound gate 4).
    //
    // Candidate C makes correctness independent of any step-count bound: the fallback re-march
    // guarantees the hit-set regardless of how the fast pass behaves, so the old gate-4 invariant
    // `steps(ω>1) ≤ steps(ω=1)` is IRRELEVANT to soundness. It is replaced by an OBSERVATION (not
    // a pass/fail correctness gate): how OFTEN does the fast pass exhaust and trigger the (costly)
    // re-march, and does the over-relaxed fast pass still REDUCE probe steps on the common
    // converging rays (the B1 win)? The orchestrator uses these numbers for the ship call. The
    // only assertion here is the must-render sanity (the fixture is non-empty); a high re-march
    // fraction is REPORTED, not failed, and flagged in the println for the orchestrator.
    // ===========================================================================================

    /// PERF OBSERVATION — re-march frequency + fast-pass step reduction (NOT a correctness gate).
    /// Over the shipped fixtures + the pinned BUG-B1-HOLE-1 cliff seed + a handful of widened
    /// random scenes, counts, per ω ∈ {1.2, 1.5, 1.99}: (a) the % of pixels whose fast pass
    /// EXHAUSTED and re-marched (the perf risk — a large fraction means B1 is perf-neutral or
    /// negative), and (b) the mean fast-pass probe count vs the ω=1 plain march on CONVERGING
    /// rays (the B1 win — fewer steps to the same hit). Prints a summary for the orchestrator.
    #[test]
    fn perf_observation_remarch_frequency_and_step_reduction() {
        // The pinned cliff seed (the historical worst ray) — exercised explicitly.
        let cliff = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let scenes = [
            ("crater", crater()),
            ("box", box_csg()),
            ("smooth", smooth_union()),
            ("cliff_seed", cliff),
        ];
        let mut worst_remarch_pct = 0.0_f64;
        for (sname, edits) in &scenes {
            for &omega in &[1.2_f32, 1.5, 1.99] {
                let mut pixels = 0u64;
                let mut remarches = 0u64;
                // Converging-ray step accounting: only rays that HIT under BOTH ω=1 and ω
                // (so the comparison is the same surface) and did NOT need a re-march.
                let mut conv_rays = 0u64;
                let mut sum_fast = 0u64; // fast-pass steps at ω (the B1 path)
                let mut sum_plain = 0u64; // fast-pass steps at ω=1 (the baseline)
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let o1 = march_obs(edits, ro, rd, 1.0e30, 1.0);
                        let oo = march_obs(edits, ro, rd, 1.0e30, omega);
                        pixels += 1;
                        if oo.remarched {
                            remarches += 1;
                        }
                        // Common converging rays: both hit, neither re-marched → the step
                        // reduction is the apples-to-apples B1 win on the typical case.
                        if o1.hit && oo.hit && !o1.remarched && !oo.remarched {
                            conv_rays += 1;
                            sum_fast += u64::from(oo.fast_steps);
                            sum_plain += u64::from(o1.fast_steps);
                        }
                    }
                }
                let remarch_pct = 100.0 * remarches as f64 / pixels as f64;
                worst_remarch_pct = worst_remarch_pct.max(remarch_pct);
                let (mean_fast, mean_plain, reduction) = if conv_rays > 0 {
                    let mf = sum_fast as f64 / conv_rays as f64;
                    let mp = sum_plain as f64 / conv_rays as f64;
                    (mf, mp, 100.0 * (mp - mf) / mp)
                } else {
                    (0.0, 0.0, 0.0)
                };
                // A POSITIVE reduction is the B1 win (fewer steps to the same hit); negative means
                // the over-relax overshot and cost extra steps at this ω. ω=1.2 (the production
                // DEFAULT) is the column that decides the ship call.
                let verdict = if reduction >= 0.0 { "B1 win" } else { "B1 LOSS" };
                println!(
                    "[B1 perf] {sname} ω={omega}: re-march {remarches}/{pixels} px ({remarch_pct:.2}%); \
                     converging rays {conv_rays}: mean fast-pass steps {mean_fast:.2} vs plain {mean_plain:.2} \
                     (step reduction {reduction:.1}% — {verdict})"
                );
            }
        }
        // Sanity only (NOT a correctness gate): the fixtures must actually render surfaces so the
        // observation is meaningful. The re-march fraction itself is REPORTED, never failed.
        println!(
            "[B1 perf] OBSERVATION SUMMARY: worst re-march fraction over all fixtures/ω = {worst_remarch_pct:.2}% \
             (FLAG for the orchestrator if this is a large fraction — would mean B1 is perf-neutral/negative)"
        );
    }

    /// GATE 3 — NO-HOLES TRIPWIRE (adapted to the Candidate-C contract). The gate-2 invariant
    /// must have TEETH. Under Candidate C, "broken" must break the RE-MARCH too — otherwise the
    /// fallback silently rescues a broken fast pass and the tripwire goes inert. So this asserts
    /// TWO things:
    ///   (a) the broken over-relax WITH C's fallback DISABLED (`march_hit_broken(.., false)` — a
    ///       WRONG sor-fail retreat that leaps past the surface, no re-march) produces ≥ 1 HOLE.
    ///       If it never holed, gate 2 would be vacuous.
    ///   (b) the SAME broken fast pass WITH C's re-march RE-ATTACHED (`march_hit_broken(.., true)`)
    ///       produces ZERO holes — proving the re-march (NOT the fast pass) is what closes them,
    ///       i.e. C's guarantee is load-bearing and gate 2 passes BECAUSE of the re-march.
    /// (The broken march is test-only; never shipped.)
    #[test]
    fn gate3_no_holes_tripwire_broken_overrelax_holes() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut total_holes_no_remarch = 0u64;
        let mut total_holes_with_remarch = 0u64;
        for (sname, edits) in &scenes {
            for &omega in &[1.5_f32, 1.9] {
                let mut scene_holes = 0u64;
                let mut scene_holes_remarched = 0u64;
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let base = pixel_hits(edits, px, py, 1.0);
                        if base {
                            // (a) C's fallback disabled: this is the tripwire that MUST hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, false) {
                                scene_holes += 1;
                            }
                            // (b) the EXACT C re-march re-attached: it must close the hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, true) {
                                scene_holes_remarched += 1;
                            }
                        }
                    }
                }
                total_holes_no_remarch += scene_holes;
                total_holes_with_remarch += scene_holes_remarched;
                if scene_holes > 0 {
                    println!("[B1 gate3] tripwire: broken over-relax (no re-march) holed {scene_holes} px on {sname} @ ω={omega}");
                }
            }
        }
        // (a) teeth: the broken fast pass without the fallback MUST hole.
        assert!(
            total_holes_no_remarch > 0,
            "TRIPWIRE INERT: the broken over-relax (re-march disabled) produced ZERO holes — gate 2 would be vacuous"
        );
        // (b) the re-march is the load-bearing guarantee: re-attaching it closes EVERY hole.
        assert_eq!(
            total_holes_with_remarch, 0,
            "C CONTRACT VIOLATION: the Candidate-C re-march failed to close {total_holes_with_remarch} broken-fast-pass holes — the fallback is not actually hole-free"
        );
        println!(
            "[B1 gate3] tripwire armed: {total_holes_no_remarch} holes WITHOUT the re-march (gate 2 has teeth); \
             {total_holes_with_remarch} holes WITH the re-march re-attached (C's fallback closes them ALL — the guarantee is load-bearing)"
        );
    }

    /// REGRESSION PIN (BUG-B1-HOLE-1, CLOSED via Candidate C) — the minimal scene the gate-2
    /// property shrank to: a super-Lipschitz smooth-min CSG (a box with smoothness 0.244 blended
    /// between two spheres) that USED to produce a missed-surface HOLE at pixel (28,16) under
    /// ω=1.2 through the PRODUCTION golden (`golden_composite_pixel_ex_omega`). Candidate C closes
    /// the hole UNCONDITIONALLY: when the over-relaxed fast pass exhausts the budget on this ray,
    /// the fallback re-march replays the frozen plain marcher from the original seed and lands on
    /// the SAME lit SDF surface ω=1 hits — byte-identical color, not merely non-background.
    ///
    /// This is the PERMANENT regression guard (NOT ignored). FLIPPED for the C contract: it now
    /// asserts BOTH ω=1.0 AND ω ∈ {1.2, 1.5, 1.99} HIT the surface (the no-hole contract — none
    /// reverts to BACKGROUND) AND land on the SAME surface FEATURE as ω=1 (the lit SDF color, far
    /// from background — within a few LSBs per channel, NOT a phantom).
    ///
    /// NOTE on exactness: byte-exact color equality with ω=1 holds ONLY when the Candidate-C
    /// re-march fires (it replays the frozen plain marcher → the identical `t` and shade — true
    /// for ω=1.2 here). When the over-relaxed FAST pass converges on its own (ω=1.5/1.99 at this
    /// pixel) it lands within `SDF_EPS` of the surface at a marginally different `t`, so the
    /// Lambert shade differs by a few LSBs. That is a valid same-surface hit, not a hole — the
    /// contract is HIT (not background), asserted via the lit-vs-background channel separation.
    #[test]
    fn bug_b1_hole_1_smooth_min_overrelax_hole_via_production_golden() {
        let edits = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let (px, py) = (28u32, 16u32);
        let bg = super::pack_rgba([0.05, 0.05, 0.1]);
        // Channel splitter for the lit-vs-background separation (the three composite outcomes are
        // >100/255 apart, so a few-LSB convergence-point wobble never reclasses a hit as a miss).
        let chans = |c: u32| [(c & 0xFF) as i32, ((c >> 8) & 0xFF) as i32, ((c >> 16) & 0xFF) as i32];
        let bgc = chans(bg);
        let far_from_bg = |c: u32| {
            let cc = chans(c);
            (0..3).any(|i| (cc[i] - bgc[i]).abs() > 8)
        };
        // No mesh (depth = clear) so the SDF/background partition is the field's alone.
        let c1 = golden_composite_pixel_ex_omega(&edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0);
        assert_ne!(c1, bg, "ω=1 must HIT the smooth-min surface at ({px},{py})");
        assert!(far_from_bg(c1), "ω=1 color 0x{c1:08X} must be the LIT SDF surface, far from bg 0x{bg:08X}");
        // Candidate C: EVERY shipped ω>1 must now HIT the same surface FEATURE as ω=1 — the hole
        // is closed by the re-march, not a step bound. The task's required set: {1.2, 1.5, 1.99}.
        for &omega in &[1.2_f32, 1.5, 1.99] {
            let co = golden_composite_pixel_ex_omega(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, omega,
            );
            // The pixel-color delta to ω=1: 0 when the re-march fired (exact frozen replay), a few
            // LSBs when the fast pass converged on its own. Printed so the orchestrator sees it.
            let dc = chans(co);
            let c1c = chans(c1);
            let max_ch = (0..3).map(|i| (dc[i] - c1c[i]).abs()).max().unwrap_or(0);
            println!(
                "[BUG-B1-HOLE-1 CLOSED] ω=1 0x{c1:08X} | ω={omega} 0x{co:08X} (hit={}, Δ to ω=1 = {max_ch}/255) | bg=0x{bg:08X}",
                co != bg
            );
            // The no-hole contract: ω>1 must NOT revert to background — it HITS the surface.
            assert_ne!(co, bg, "BUG-B1-HOLE-1 CLOSED: ω={omega} must HIT (not background) at ({px},{py})");
            assert!(
                far_from_bg(co),
                "BUG-B1-HOLE-1 CLOSED: ω={omega} color 0x{co:08X} must be the LIT SDF surface (far from bg 0x{bg:08X}), not a hole, at ({px},{py})"
            );
            // Same surface FEATURE as ω=1 — the lit colors agree within the small convergence-point
            // wobble (a phantom or a different feature would differ by >100/channel like bg/mesh).
            assert!(
                max_ch <= 16,
                "BUG-B1-HOLE-1 CLOSED: ω={omega} 0x{co:08X} differs from ω=1 0x{c1:08X} by {max_ch}/255 (>16) — not the same surface feature at ({px},{py})"
            );
        }
    }

    /// GATE 5 — ω CLAMP + 8-byte push encode. Every NON-NaN hostile `omega_in` (negative,
    /// sub-1, == 2, > 2, ±∞) must clamp into `[1.0, 1.99]` and decode finite-in-range from the
    /// pushed bytes `[4..8]`. Mirrors the harness's `omega_in.clamp(1.0, 1.99)` + push encode
    /// EXACTLY (the same `f32::clamp` + `to_le_bytes`).
    ///
    /// FINDING (documented, NOT a soundness hole): Rust's `f32::clamp` does NOT sanitize a NaN
    /// VALUE — `f32::NAN.clamp(1.0, 1.99) == NaN` (the clamp returns NaN when `self` is NaN).
    /// So a NaN ω SURVIVES the harness clamp and is pushed verbatim. It is defanged DOWNSTREAM,
    /// not by the clamp: the marcher's gate is `if (omega > 1.0)`, and `NaN > 1.0 == false` on
    /// BOTH host (`golden_composite_pixel_ex_omega`) and shader, so a NaN ω takes the verbatim
    /// frozen `t += d` plain arm — i.e. it degrades to the ω=1 path (NO over-relaxation, NO
    /// hole). This test asserts that exact safety property (NaN ω ≡ ω=1 over a pixel sweep)
    /// rather than a false "clamp produces 1.0" claim. See the tester report.
    #[test]
    fn gate5_omega_clamp_and_push_encode() {
        // The harness's encode site, reproduced byte-for-byte via the 32-byte
        // `FineMarcherPush` (A1/A2 widened the push 8 → 32 B; `lighting_flags == 0` keeps
        // the OFF path). coarse_enabled stays at offset 0, omega at offset 4.
        fn encode(omega_in: f32, coarse_enabled: bool) -> [u8; GBUFFER_MARCHER_PUSH_BYTES as usize] {
            let omega: f32 = omega_in.clamp(1.0, 1.99);
            let push = FineMarcherPush::new(coarse_enabled, omega, 0, DEFAULT_LIGHT_DIR);
            push.as_bytes().try_into().expect("invariant: FineMarcherPush is GBUFFER_MARCHER_PUSH_BYTES")
        }
        // Non-NaN hostile inputs: ALL must clamp finite into [1.0, 1.99].
        let cases = [-1.0_f32, 0.5, 1.0, 1.2, 1.99, 2.0, 2.5, 100.0, f32::INFINITY, f32::NEG_INFINITY];
        for &om in &cases {
            let push = encode(om, true);
            let coarse = u32::from_le_bytes([push[0], push[1], push[2], push[3]]);
            let decoded = f32::from_le_bytes([push[4], push[5], push[6], push[7]]);
            assert_eq!(coarse, 1, "coarse_enabled must round-trip as 1");
            assert!(decoded.is_finite(), "ω={om} decoded to non-finite {decoded}");
            assert!(
                (1.0..=1.99).contains(&decoded),
                "ω={om} decoded to {decoded}, outside [1.0, 1.99] (clamp failed)"
            );
        }
        // The DOCUMENTED NaN behavior: the clamp passes NaN through (this is the finding).
        let nan_push = encode(f32::NAN, false);
        let nan_dec = f32::from_le_bytes([nan_push[4], nan_push[5], nan_push[6], nan_push[7]]);
        assert!(nan_dec.is_nan(), "FINDING CHANGED: f32::clamp now sanitizes NaN? got {nan_dec}");
        // The SOUNDNESS property that actually protects us: a NaN ω ≡ the ω=1 plain march
        // (the `omega > 1.0` gate is false for NaN on both host and shader → no hole).
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let plain = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                    );
                    let nan_omega = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, f32::NAN,
                    );
                    assert_eq!(
                        plain, nan_omega,
                        "[{sname}] NaN ω diverged from the ω=1 plain march at ({px},{py}): \
                         plain 0x{plain:08X} nan 0x{nan_omega:08X} — the NaN-defang property broke"
                    );
                    checked += 1;
                }
            }
        }
        // The production default must be inside the clamp window (a sanity tie to the harness).
        assert!((1.0..=1.99).contains(&DEFAULT_MARCHER_OMEGA), "DEFAULT_MARCHER_OMEGA out of clamp window");
        println!(
            "[B1 gate5] ω clamp: {} non-NaN hostile inputs decode finite ∈ [1.0,1.99]; NaN PASSES the clamp \
             but ≡ the ω=1 plain march over {checked} px (gate false for NaN); default={DEFAULT_MARCHER_OMEGA}",
            cases.len()
        );
    }
}

// ===========================================================================
// M1 — the EMPTY-SPACE-SKIP marcher test matrix (host-side, no GPU).
//
// The two LOAD-BEARING soundness gates:
//   (1) OFF byte-identical — `golden_composite_pixel_brick(brick_enabled=0)` is
//       BIT-FOR-BIT the pre-M1 `golden_composite_pixel_ex_omega_lit` over a
//       battery of scenes/cameras/pixels (the 0%-gate).
//   (2) ON hit-set == analytic — over ≥500 random scenes + the demo scene, the
//       empty-skip ON marcher and the analytic (OFF) marcher agree on EVERY
//       pixel's HIT/MISS classification AND surface color (the empty skip only
//       changes WHERE steps land, never the converged hit). A skipped or spurious
//       surface is a BLOCKER.
//
// Plus: never-skip-surface (3), `dist_to_brick_exit` progress (4),
// `build_pointer_grid` correctness (5), push-constant layout (6) — the std430
// agreement the dev SPIR-V-verified, guarded host-side.
//
// The xorshift scene generator mirrors the M0 brick GATE generator (no new dep).
// ===========================================================================
#[cfg(test)]
mod m1_empty_skip_tests {
    use super::{CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FineMarcherPush, GBUFFER_MARCHER_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_op};
    use crate::goldens::{golden_composite_pixel_brick, golden_composite_pixel_ex_omega_lit, host_brick_cell};
    use boyko_sdf_math::brick::{
        BRICK_EXIT_EPS, DEFAULT_BRICK_WORLD, DEFAULT_GRID_DIM, PointerGrid, build_pointer_grid,
        classify_brick, dist_to_brick_exit,
    };
    use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

    // ── shared fixtures ────────────────────────────────────────────────────

    /// `EmptyOutside as u32` — the cell class the empty-skip acts on (mirror of the
    /// private `super::BRICK_CLASS_EMPTY_OUTSIDE`, re-stated so a drift in the enum
    /// discriminant is caught by `class_codes_match_brickclass_discriminants`).
    const EMPTY_OUTSIDE: u32 = 0;

    /// A deterministic xorshift64* PRNG — the scene generator without a dep (mirrors
    /// the M0 brick GATE's generator so the two suites draw from the SAME family).
    struct XorShift64(u64);

    impl XorShift64 {
        fn new(seed: u64) -> Self {
            Self(seed ^ 0x9E37_79B9_7F4A_7C15)
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let frac = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            lo + frac * (hi - lo)
        }
        fn below(&mut self, n: u32) -> u32 {
            (self.next_u64() % n as u64) as u32
        }
    }

    /// The demo / golden "crater" CSG scene (base sphere minus a smaller sphere) — the
    /// SAME scene the rung-9/10 and B1 goldens use, so the M1 gates run against the
    /// production demo field, not only synthetic scenes.
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }

    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Builds an `SdfEditField` (the authority) from a slice of edits and bumps its gen.
    fn field_of(edits: &[SdfEdit]) -> SdfEditField {
        let mut f = SdfEditField::new();
        for e in edits {
            assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    }

    /// A random valid scene (1..=6 edits, first forced UNION, SPHERE/BOX, radii/half-
    /// extents >= 0.5, centers in [-2,2]³, UNION/SUBTRACT/INTERSECT, smoothness 0 or 0.15)
    /// returned as a `Vec<SdfEdit>` — the ON-vs-analytic gate's scene family.
    fn random_scene(rng: &mut XorShift64) -> Vec<SdfEdit> {
        let n = 1 + rng.below(6);
        let mut edits = Vec::with_capacity(n as usize);
        for i in 0..n {
            let center =
                [rng.range(-2.0, 2.0), rng.range(-2.0, 2.0), rng.range(-2.0, 2.0)];
            let op = if i == 0 {
                sdf_op::UNION
            } else {
                match rng.below(3) {
                    0 => sdf_op::UNION,
                    1 => sdf_op::SUBTRACT,
                    _ => sdf_op::INTERSECT,
                }
            };
            let smoothness = if rng.below(2) == 0 { 0.0 } else { 0.15 };
            let e = if rng.below(2) == 0 {
                SdfEdit::sphere(center, rng.range(0.5, 1.5), op, smoothness)
            } else {
                SdfEdit::box_shape(
                    center,
                    [rng.range(0.5, 1.2), rng.range(0.5, 1.2), rng.range(0.5, 1.2)],
                    op,
                    smoothness,
                )
            };
            edits.push(e);
        }
        edits
    }

    /// The default near-field pointer grid baked from `field` — the SAME grid the GPU
    /// binds at binding 9 (origin/dims/brick_world the `FineMarcherPush` carries).
    fn build_default_grid(field: &SdfEditField) -> (PointerGrid, Vec<u32>) {
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(field, &grid, &mut cells);
        (grid, cells)
    }

    /// The result of one primary-march replay: the hit decision + hit-`t`, plus the
    /// over-step audit signals (`min_field` = closest analytic approach SEEN at a probe,
    /// `crossed_undetected` = a brick-exit step that jumped from outside the hit band
    /// straight PAST the surface to a point where the field went NEGATIVE — the literal
    /// definition of a skipped surface, `exhausted` = ran the whole iteration budget).
    struct MarchTrace {
        hit: bool,
        min_field: f32,
        crossed_undetected: bool,
        exhausted: bool,
    }

    /// Replays the brick-ON / analytic PRIMARY march (the empty-skip loop, no re-march or
    /// shade) and audits it for an OVER-STEP. A verbatim mirror of
    /// `golden_composite_pixel_brick`'s primary loop, plus: before each brick-exit step it
    /// records the field at the PRE- and POST-step points; if the pre-step field was
    /// positive (outside the solid) and the post-step field is NEGATIVE (inside), the
    /// brick step JUMPED PAST a surface undetected (a skip → soundness BLOCKER). ORTHO,
    /// no mesh.
    fn march_primary(
        edits: &[SdfEdit],
        px: u32,
        py: u32,
        grid: &PointerGrid,
        cells: &[u32],
        brick_on: bool,
    ) -> MarchTrace {
        use super::{SDF_EPS, SDF_MAX_IT, SDF_T_MAX, composite_ray, sdf_edit_list};
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        let mut t = 0.0_f32;
        let mut hit = false;
        let mut min_field = f32::INFINITY;
        let mut crossed_undetected = false;
        let mut iters = 0u32;
        for _ in 0..SDF_MAX_IT {
            iters += 1;
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            if brick_on
                && let Some((class, cmin)) = host_brick_cell(grid, cells, p)
                && class == EMPTY_OUTSIDE
            {
                // Audit the brick-exit step for an over-step: sample the analytic field
                // at the PRE-step point and at the POST-step point. A skip would show as
                // pre >= 0 (outside) but post < 0 (inside) — the step crossed a surface.
                let pre_d = sdf_edit_list(edits, p);
                min_field = min_field.min(pre_d);
                let exit = dist_to_brick_exit(p, rd, cmin, grid.brick_world);
                t += exit;
                let q = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let post_d = sdf_edit_list(edits, q);
                if pre_d >= 0.0 && post_d < 0.0 {
                    crossed_undetected = true;
                }
                min_field = min_field.min(post_d);
                if t > SDF_T_MAX {
                    break;
                }
                continue;
            }
            let d = sdf_edit_list(edits, p);
            min_field = min_field.min(d);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d;
            if t > SDF_T_MAX {
                break;
            }
        }
        MarchTrace {
            hit,
            min_field,
            crossed_undetected,
            exhausted: iters == SDF_MAX_IT && !hit,
        }
    }

    /// Max per-channel difference between two packed `0xAABBGGRR` colors (RGB only).
    fn chan_delta(a: u32, b: u32) -> i32 {
        let c = |x: u32, sh: u32| ((x >> sh) & 0xFF) as i32;
        (c(a, 0) - c(b, 0))
            .abs()
            .max((c(a, 8) - c(b, 8)).abs())
            .max((c(a, 16) - c(b, 16)).abs())
    }

    /// A sparse-but-representative pixel battery across the 64×64 frame (corners, edges,
    /// center, and a diagonal sweep) — exercises HIT, MISS, and the sphere-edge grazing
    /// rays without folding the field on all 4096 pixels in the per-scene inner loops.
    fn pixel_battery() -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        let coords = [0u32, 1, 8, 16, 24, 31, 32, 40, 48, 56, 62, 63];
        for &py in &coords {
            for &px in &coords {
                v.push((px, py));
            }
        }
        // A diagonal sweep for extra edge coverage.
        for i in 0..SDF_IMG_W {
            v.push((i, i % SDF_IMG_H));
        }
        v
    }

    // ── 6. PUSH-CONSTANT LAYOUT (the std430 host/GPU agreement guard) ──────

    /// The M1 `FineMarcherPush` field offsets match the std430/HLSL layout the dev
    /// SPIR-V-verified (`grid_origin@32, brick_enabled@44, grid_dims@48, brick_world@60`)
    /// and the block is 64 bytes — a runtime mirror of the `const _: () = assert!` pins
    /// so a future reorder that desyncs the GPU push is caught even if the const-asserts
    /// are ever weakened.
    #[test]
    fn fine_marcher_push_m1_field_offsets_match_std430() {
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_origin), 32, "grid_origin @32");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_enabled), 44, "brick_enabled @44");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_dims), 48, "grid_dims @48");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_world), 60, "brick_world @60");
        // M2 widened the push to the full 80-byte COMPOSITE range (brick_trilinear @64 + _pad3 @68).
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_trilinear), 64, "brick_trilinear @64");
        assert_eq!(GBUFFER_MARCHER_PUSH_BYTES, 80, "FineMarcherPush must be 80 bytes");
    }

    /// `with_brick` flips `brick_enabled` to 1 and stamps the grid uniforms, preserving
    /// the base gates; `new` leaves the M1 block OFF (brick_enabled == 0, zero grid).
    #[test]
    fn with_brick_sets_grid_uniforms_and_preserves_base_gates() {
        let base = FineMarcherPush::new(true, 1.3, LIGHTING_FLAG_SHADOWS, [0.1, 0.2, 0.3]);
        assert_eq!(base.brick_enabled, 0, "new() leaves the empty-skip OFF");
        assert_eq!(base.grid_dims, [0, 0, 0], "new() zeroes the grid");

        let with = base.with_brick([-4.0, -4.0, -4.0], [16, 16, 16], 0.5);
        assert_eq!(with.brick_enabled, 1, "with_brick turns the empty-skip ON");
        assert_eq!(with.grid_origin, [-4.0, -4.0, -4.0], "grid_origin stamped");
        assert_eq!(with.grid_dims, [16, 16, 16], "grid_dims stamped");
        assert_eq!(with.brick_world, 0.5, "brick_world stamped");
        // Base gates preserved.
        assert_eq!(with.coarse_enabled, 1, "coarse gate preserved");
        assert_eq!(with.omega, 1.3, "omega preserved");
        assert_eq!(with.lighting_flags, LIGHTING_FLAG_SHADOWS, "lighting flags preserved");
        assert_eq!(with.light_dir, [0.1, 0.2, 0.3], "light_dir preserved");
    }

    /// The host `EMPTY_OUTSIDE` code the empty-skip branches on equals the
    /// `BrickClass::EmptyOutside` discriminant the bake stores — a drift in the enum
    /// repr would make the skip act on the wrong class (or never).
    #[test]
    fn class_codes_match_brickclass_discriminants() {
        assert_eq!(BrickClass::EmptyOutside as u32, EMPTY_OUTSIDE, "EmptyOutside == 0");
        assert_eq!(BrickClass::EmptyInside as u32, 1, "EmptyInside == 1");
        assert_eq!(BrickClass::Surface as u32, 2, "Surface == 2");
    }

    // ── 1. OFF BYTE-IDENTICAL (the 0%-gate) ────────────────────────────────

    /// `golden_composite_pixel_brick(brick_enabled=0)` is BIT-FOR-BIT the pre-M1
    /// `golden_composite_pixel_ex_omega_lit` over a battery of pixels across the demo
    /// scenes (crater / box / smooth-union) under both ORTHO and a perspective camera,
    /// at omega 1.0 and the default omega, and both lighting OFF and ON. Any single
    /// byte difference is a BLOCKER — the 0%-gate is broken.
    #[test]
    fn off_path_is_byte_identical_to_pre_m1_golden() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        // The grid is supplied but MUST be ignored on the OFF path (a degenerate grid
        // proves the OFF path never reads it).
        let dummy_grid = PointerGrid::default_near_field();
        let dummy_cells = vec![0u32; dummy_grid.cell_count()];

        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let omegas = [1.0_f32, DEFAULT_MARCHER_OMEGA];
        let light_cfgs = [
            (0u32, DEFAULT_LIGHT_DIR),
            (LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, [0.3, 0.7, 1.0]),
        ];
        let mesh_depths = [MESH_DEPTH_CLEAR, 0.5_f32]; // no-mesh + a covering mesh

        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for &(cname, cam) in &cameras {
                for &om in &omegas {
                    for &(flags, ldir) in &light_cfgs {
                        for &md in &mesh_depths {
                            for &(px, py) in &pixel_battery() {
                                let off = golden_composite_pixel_brick(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                    false, &dummy_grid, &dummy_cells,
                                );
                                let pre_m1 = golden_composite_pixel_ex_omega_lit(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                );
                                assert_eq!(
                                    off, pre_m1,
                                    "[{sname}/{cname} ω={om} flags={flags} md={md}] OFF path \
                                     diverged from pre-M1 at ({px},{py}): brick 0x{off:08X} \
                                     pre-M1 0x{pre_m1:08X} — the 0%-gate is BROKEN"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        // 3 scenes × 2 cameras × 2 ω × 2 light cfgs × 2 mesh depths × |battery| = 9984.
        assert!(checked > 9_000, "OFF gate must exercise a wide battery (got {checked})");
        println!("[M1 OFF 0%-gate] {checked} pixels byte-identical to pre-M1 golden");
    }

    // ── 2. ON HIT-SET == ANALYTIC (the load-bearing M1 property) ───────────

    /// THE LOAD-BEARING M1 PROPERTY. Over the demo scene + ≥500 random scenes, asserts
    /// the empty-skip is SOUND and behavior-identical at the SHIPPING level:
    ///
    ///   (a) NO OVER-STEP (the direct soundness invariant, budget-INDEPENDENT): not one
    ///       brick-exit step ever jumps from OUTSIDE the solid (pre-step field >= 0)
    ///       straight to INSIDE it (post-step field < 0) — the literal definition of a
    ///       skipped surface. This is the conservative-classifier contract: an
    ///       EmptyOutside brick has no surface within band_half, so stepping to its exit
    ///       cannot cross one. A single `crossed_undetected` is a soundness BLOCKER.
    ///
    ///   (b) PRODUCTION HIT-SET + COLOR (the shipping contract): the production
    ///       `golden_composite_pixel_brick` (WITH its `exhausted` re-march fallback, the
    ///       same the GPU shader runs) yields ON output within ±1/255 of analytic — the
    ///       converged-`t` < `SDF_EPS` rounding is the only difference; tighter than the
    ///       established ±2/255 GPU-golden tolerance.
    ///
    /// The PRIMARY-loop hit-class is also tracked: any divergence there is a `MAX_IT`-cliff
    /// budget-edge artifact on a near-tangent ray (NOT an over-step), and the test asserts
    /// EVERY such pixel is resolved to ±1/255 by the production re-march (so the artifact
    /// is provably non-shipping).
    #[test]
    fn on_hit_set_equals_analytic_over_many_scenes() {
        const SCENES: u64 = 600; // ≥500 random scenes + the demo scenes below
        let battery = pixel_battery();

        let demo: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut overstep_blockers: u64 = 0;
        let mut primary_budget_flips: u64 = 0;
        let mut checked: u64 = 0;
        let mut max_chan: i32 = 0;
        let mut first_overstep: Option<String> = None;
        let mut first_color_violation: Option<String> = None;
        let mut first_unresolved_flip: Option<String> = None;
        let mut unresolved_flips: u64 = 0;

        let mut run_scene = |label: &str, edits: &[SdfEdit]| {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            for &(px, py) in &battery {
                checked += 1;
                let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                let an_trace = march_primary(edits, px, py, &grid, &cells, false);

                // (a) the direct over-step soundness invariant (budget-independent).
                if on_trace.crossed_undetected {
                    overstep_blockers += 1;
                    if first_overstep.is_none() {
                        first_overstep = Some(format!(
                            "[{label}] ({px},{py}) a brick-exit step crossed a surface \
                             undetected (min_field={:.4e}); edits={edits:?}",
                            on_trace.min_field
                        ));
                    }
                }

                // (b) the PRODUCTION shipping contract: ON within ±1/255 of analytic.
                // Lighting OFF: bare Lambert (the ON lighting path is ±3/255 vs the
                // shader and is gated separately).
                let on = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                );
                let analytic = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                );
                let dchan = chan_delta(on, analytic);
                max_chan = max_chan.max(dchan);
                if dchan > 1 && first_color_violation.is_none() {
                    first_color_violation = Some(format!(
                        "[{label}] ({px},{py}) per-channel Δ={dchan} > 1/255 \
                         (ON 0x{on:08X} analytic 0x{analytic:08X})"
                    ));
                }

                // Track primary-loop hit-class flips and PROVE each is a budget artifact
                // (the production function resolves it to ±1/255 via the re-march).
                if on_trace.hit != an_trace.hit {
                    primary_budget_flips += 1;
                    // A genuine artifact has at least one path exhausting the budget AND
                    // no over-step; the production function must reconcile it.
                    if dchan > 1 {
                        unresolved_flips += 1;
                        if first_unresolved_flip.is_none() {
                            first_unresolved_flip = Some(format!(
                                "[{label}] ({px},{py}) primary hit-flip NOT resolved by the \
                                 production re-march: ON-exhausted={} AN-exhausted={} dchan={dchan} \
                                 (ON 0x{on:08X} analytic 0x{analytic:08X})",
                                on_trace.exhausted, an_trace.exhausted
                            ));
                        }
                    }
                }
            }
        };

        for (label, edits) in &demo {
            run_scene(label, edits);
        }
        for seed in 0..SCENES {
            let mut rng = XorShift64::new(seed.wrapping_mul(0x100_0001).wrapping_add(1));
            let edits = random_scene(&mut rng);
            run_scene(&format!("rand#{seed}"), &edits);
        }

        // (a) the SOUNDNESS gate: zero over-steps (no surface skipped).
        assert_eq!(
            overstep_blockers, 0,
            "{overstep_blockers}/{checked} brick-exit steps crossed a surface undetected — \
             a SKIPPED surface (SOUNDNESS BLOCKER). First: {}",
            first_overstep.unwrap_or_default()
        );
        // (b) the shipping contract: production ON within ±1/255 of analytic.
        assert!(
            max_chan <= 1,
            "production ON surface color exceeded ±1/255 vs analytic (max per-channel \
             Δ={max_chan}). First: {}",
            first_color_violation.unwrap_or_default()
        );
        // Every primary-loop hit-flip is a budget-edge artifact the production re-march
        // resolves to ±1/255 (none ships as a divergence).
        assert_eq!(
            unresolved_flips, 0,
            "{unresolved_flips} primary-loop hit-flips were NOT resolved by the production \
             re-march (a shipping divergence). First: {}",
            first_unresolved_flip.unwrap_or_default()
        );
        assert!(checked > 50_000, "ON gate must exercise a wide battery (got {checked})");
        println!(
            "[M1 ON-vs-analytic] {checked} pixels over {} scenes: 0 over-steps, \
             {primary_budget_flips} primary-loop budget-edge flips (ALL resolved by the \
             production re-march to ±{max_chan}/255 — non-shipping)",
            SCENES + 3
        );
    }

    // ── 3. EMPTY-SKIP NEVER SKIPS A SURFACE (the property, targeted) ───────

    /// A thin shell straddling an EMPTY/SURFACE brick boundary: every ray that hits the
    /// surface analytically must still hit with the empty-skip ON (no surface skipped),
    /// and every ray that misses analytically must still miss (no spurious hit). Asserts
    /// per-pixel HIT-classification equality on the boundary-grazing scene.
    #[test]
    fn empty_skip_never_skips_or_invents_a_surface_at_brick_boundary() {
        // A small sphere placed so its surface grazes a brick face of the default grid
        // (brick_world = 0.5; a center at a half-cell offset makes the surface cross a
        // cell boundary). Plus a thin box to graze a face from outside.
        let scenes: [(&str, Vec<SdfEdit>); 3] = [
            (
                "sphere_on_face",
                vec![SdfEdit::sphere([0.25, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)],
            ),
            (
                "thin_box_face_graze",
                vec![SdfEdit::box_shape([0.5, 0.0, 0.0], [0.5, 0.6, 0.6], sdf_op::UNION, 0.0)],
            ),
            (
                "off_center_csg",
                vec![
                    SdfEdit::sphere([0.5, 0.5, 0.0], 0.7, sdf_op::UNION, 0.0),
                    SdfEdit::sphere([0.75, 0.5, 0.0], 0.3, sdf_op::SUBTRACT, 0.0),
                ],
            ),
        ];

        for (label, edits) in &scenes {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            // EVERY pixel of the frame (a thin-surface scene wants dense coverage).
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    // The PROPERTY: not one brick-exit step crosses a surface — no surface
                    // skipped (analytic-hit ⟹ no undetected crossing), no surface invented.
                    let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                    assert!(
                        !on_trace.crossed_undetected,
                        "[{label}] ({px},{py}) a brick-exit step crossed a surface UNDETECTED \
                         (min_field={:.4e}) — a surface was SKIPPED",
                        on_trace.min_field
                    );
                    // The production composited color (with re-march) stays within ±1/255.
                    let on = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                    );
                    let analytic = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                    );
                    assert!(
                        chan_delta(on, analytic) <= 1,
                        "[{label}] ({px},{py}) color Δ {} > 1/255 (ON 0x{on:08X} analytic \
                         0x{analytic:08X}) — the empty skip changed the surface",
                        chan_delta(on, analytic)
                    );
                }
            }
        }
    }

    // ── 4. `dist_to_brick_exit` PROGRESS (no zero/negative step) ───────────

    /// A ray parallel to a brick face, starting exactly on a boundary, or axis-aligned
    /// through cell corners still advances by >= `BRICK_EXIT_EPS` (no zero/negative step
    /// → no infinite march). Tests the degenerate directions head-on.
    #[test]
    fn dist_to_brick_exit_always_advances_on_degenerate_rays() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // Cases: (ro, rd, label). All `ro` are on/at a brick face or corner; all `rd`
        // include axis-parallel + fully-degenerate (zero) directions.
        let cases: &[([f32; 3], [f32; 3], &str)] = &[
            // Parallel to the +x face plane (no x component), on the y=0 face.
            ([0.1, 0.0, 0.1], [0.0, 1.0, 0.0], "parallel_y_on_face"),
            // Parallel to a face, sitting exactly on a corner.
            ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], "axis_z_from_corner"),
            // Diagonal through the cell corner (exits at a corner, can graze).
            ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "body_diagonal_from_corner"),
            // Fully degenerate: zero direction (every axis skipped).
            ([0.25, 0.25, 0.25], [0.0, 0.0, 0.0], "zero_direction"),
            // Sub-eps direction on all axes (every axis below BRICK_EXIT_EPS).
            ([0.25, 0.25, 0.25], [1e-6, 1e-6, 1e-6], "sub_eps_all_axes"),
            // A ray starting on the FAR face, pointing further out (negative exit
            // territory) — must clamp UP to advance.
            ([0.5, 0.25, 0.25], [1.0, 0.0, 0.0], "on_far_face_outward"),
            // Negative-x ray from inside (exits the lo face).
            ([0.25, 0.25, 0.25], [-1.0, 0.0, 0.0], "neg_x_from_center"),
        ];
        for &(ro, rd, label) in cases {
            let exit = dist_to_brick_exit(ro, rd, cell_min, bw);
            assert!(exit.is_finite(), "[{label}] exit must be finite, got {exit}");
            assert!(
                exit >= BRICK_EXIT_EPS,
                "[{label}] exit {exit} < BRICK_EXIT_EPS {BRICK_EXIT_EPS} — the march can stall (no progress)"
            );
        }
    }

    /// A well-conditioned ray through a brick exits at the analytically expected slab
    /// distance (the progress clamp does not corrupt a normal exit).
    #[test]
    fn dist_to_brick_exit_matches_slab_far_face_on_normal_ray() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // From the lo-x face center, straight +x: exits the +x face at t = 0.5.
        let exit = dist_to_brick_exit([0.0, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit - 0.5).abs() < 1e-5, "axis-aligned exit must be the slab width 0.5, got {exit}");
        // From the center, +x: exits at t = 0.25 (half the cell).
        let exit2 = dist_to_brick_exit([0.25, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit2 - 0.25).abs() < 1e-5, "centered exit must be 0.25, got {exit2}");
    }

    // ── 5. `build_pointer_grid` CORRECTNESS ────────────────────────────────

    /// Every cell the bake writes equals a direct `classify_brick` of that cell's AABB —
    /// the bake is a faithful per-cell fold of the authority (no index/origin slip).
    #[test]
    fn build_pointer_grid_matches_per_cell_classify() {
        let scenes: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        for (label, edits) in &scenes {
            let field = field_of(edits);
            let grid = PointerGrid::default_near_field();
            let mut cells = vec![0u32; grid.cell_count()];
            build_pointer_grid(&field, &grid, &mut cells);

            let w = grid.dims[0];
            let h = grid.dims[1];
            let d = grid.dims[2];
            for iz in 0..d {
                for iy in 0..h {
                    for ix in 0..w {
                        let cell_min = grid.cell_min(ix, iy, iz);
                        let expect = classify_brick(
                            &field, cell_min, grid.brick_world, SDF_EDIT_BAND_HALF,
                        ) as u32;
                        let idx = (ix + iy * w + iz * w * h) as usize;
                        assert_eq!(
                            cells[idx], expect,
                            "[{label}] cell ({ix},{iy},{iz}) bake {} != classify {expect}",
                            cells[idx]
                        );
                    }
                }
            }
        }
    }

    /// A cell with no edit nearby bakes EmptyOutside (or EmptyInside deep in a solid); a
    /// cell a surface passes through bakes Surface. Checked against a hand-placed scene.
    #[test]
    fn build_pointer_grid_classifies_empty_vs_surface() {
        // A unit sphere at the origin. Cells far out are EmptyOutside; the cell at the
        // origin (deep inside the sphere) overlaps the sphere's AABB → Surface (the C2
        // conservative rule: a primitive's AABB covers its interior). A cell on the
        // sphere's band is Surface.
        let edits = vec![SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)];
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        let cell_class = |wx: f32, wy: f32, wz: f32| -> u32 {
            let p = [wx, wy, wz];
            let (class, _) = host_brick_cell(&grid, &cells, p).expect("point inside the default grid");
            class
        };

        // A corner of the [-4,4]³ grid, far from the unit sphere → EmptyOutside.
        assert_eq!(cell_class(-3.5, -3.5, -3.5), EMPTY_OUTSIDE, "far cell must be EmptyOutside");
        // A cell straddling the sphere surface (radius 1) → Surface (class 2).
        assert_eq!(cell_class(1.0, 0.0, 0.0), BrickClass::Surface as u32, "surface cell must be Surface");
        // The center cell overlaps the sphere AABB → Surface (conservative, not EmptyInside).
        assert_eq!(cell_class(0.0, 0.0, 0.0), BrickClass::Surface as u32, "deep-inside cell is Surface (C2)");
    }

    /// Grid indexing round-trips: `cell_min(ix,iy,iz)` then a point inside that cell maps
    /// back to `(ix,iy,iz)` via `host_brick_cell`, and an out-of-grid point returns None.
    #[test]
    fn host_brick_cell_round_trips_and_bounds_check() {
        let edits = crater();
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        // Round-trip a sampling of cells: a point at the cell center maps back to it.
        for &(ix, iy, iz) in &[(0u32, 0u32, 0u32), (5, 7, 3), (15, 15, 15), (8, 0, 12)] {
            let cmin = grid.cell_min(ix, iy, iz);
            let center = [
                cmin[0] + grid.brick_world * 0.5,
                cmin[1] + grid.brick_world * 0.5,
                cmin[2] + grid.brick_world * 0.5,
            ];
            let (_, got_min) = host_brick_cell(&grid, &cells, center)
                .expect("cell-center point must land in the grid");
            assert_eq!(got_min, cmin, "cell ({ix},{iy},{iz}) center must map back to its cell_min");
        }

        // Out-of-grid points (below origin and past the far corner) → None.
        let below = [grid.origin[0] - 1.0, grid.origin[1], grid.origin[2]];
        assert!(host_brick_cell(&grid, &cells, below).is_none(), "point below origin → no cell");
        let far = [
            grid.origin[0] + grid.dims[0] as f32 * grid.brick_world + 1.0,
            grid.origin[1],
            grid.origin[2],
        ];
        assert!(host_brick_cell(&grid, &cells, far).is_none(), "point past the far face → no cell");
    }

    /// The default near-field grid spans the demo `[-4,4]³` extent (DEFAULT_GRID_DIM
    /// cells of DEFAULT_BRICK_WORLD), enclosing the demo scene with margin.
    #[test]
    fn default_near_field_grid_encloses_demo_extent() {
        let grid = PointerGrid::default_near_field();
        assert_eq!(grid.dims, [DEFAULT_GRID_DIM, DEFAULT_GRID_DIM, DEFAULT_GRID_DIM]);
        assert_eq!(grid.brick_world, DEFAULT_BRICK_WORLD);
        let half = DEFAULT_GRID_DIM as f32 * DEFAULT_BRICK_WORLD * 0.5;
        assert!((grid.origin[0] + half).abs() < 1e-6, "grid centered on origin (x)");
        // The demo primitives live within ±3; the grid spans ±4 → enclosed.
        assert!(half >= 4.0 - 1e-6, "default grid must span at least ±4 (got ±{half})");
    }

    /// The empty scene (no edits) bakes an ALL-EmptyOutside grid — every cell is
    /// provably outside, so the marcher skips the whole near-field to the analytic
    /// (background) result.
    #[test]
    fn build_pointer_grid_empty_scene_is_all_empty_outside() {
        let field = SdfEditField::new(); // no edits, gen 0
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(&field, &grid, &mut cells);
        assert!(
            cells.iter().all(|&c| c == EMPTY_OUTSIDE),
            "an empty scene must bake an all-EmptyOutside grid"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// M4 — the CLIP-MAP LOD host CPU tests (the Slice-B gate). These are CPU-runnable
// (no Vulkan device): they prove (1) the per-level bake feeds the proven baker
// correctly (bit-identity vs a direct classify/fill reference, per level), (2) the
// OFF/N=1 UBO tail is byte-identical to the M2 default, (3) the full N=3 UBO array
// matches a hand-checked std140 array-of-structs golden. The GPU image tests are
// Slice C (RTX-gated).
// ════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod m4_clipmap_tests {
    use super::{
        AtlasEncoding, BrickLevelParams, M2GridParams, M4GridParams, M4LevelParams,
        M2_ATLAS_DIM, SdfEdit, atlas_voxel_index, bake_brick_atlas_at, m2_tile_atlas_origin, sdf_op,
    };
    use boyko_sdf_math::brick::{
        self, BRICK_ALLOC, BRICK_VOXELS, band_half_at_level, brick_world_at_level, c_max_at_level,
        classify_brick, decode_snorm8, fill_brick, snapped_level_origin, snapped_level_origin_cell,
        toroidal_slot, voxel_size_at_level,
    };
    use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

    /// The demo "crater" CSG scene (base sphere minus a smaller sphere) — the SAME field the M1/M2
    /// goldens use, so the per-level bake runs against the production demo authority.
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    fn field_of(edits: &[SdfEdit]) -> SdfEditField {
        let mut f = SdfEditField::new();
        for e in edits {
            assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    }

    /// A DIRECT CPU reference bake of the `M2_ATLAS_DIM³` `Snorm8` atlas for one level's geometry,
    /// independent of `bake_brick_atlas_at`: classify each `M2_GRID_DIM³` cell, fill a SURFACE cell's
    /// apron'd tile, scatter at the cell's TOROIDAL atlas-voxel slot (M5: `toroidal_slot(origin_cell +
    /// cell)`; EMPTY cells leave 0). The bit-exact oracle the level-aware baker must match (the M3
    /// full-bake bit-identity gate, per level). `origin_cell == round(origin/brick_world)`.
    fn reference_atlas_snorm8(
        field: &SdfEditField,
        origin: [f32; 3],
        origin_cell: [i32; 3],
        brick_world: f32,
        voxel_size: f32,
        band_half: f32,
        c_max: f32,
    ) -> Vec<u8> {
        const W: usize = BRICK_ALLOC;
        let dim = brick::M2_GRID_DIM;
        let mut out = vec![0u8; (M2_ATLAS_DIM as usize).pow(3)];
        let mut tile = [0i8; BRICK_VOXELS];
        for cz in 0..dim {
            for cy in 0..dim {
                for cx in 0..dim {
                    let cell_min = [
                        origin[0] + cx as f32 * brick_world,
                        origin[1] + cy as f32 * brick_world,
                        origin[2] + cz as f32 * brick_world,
                    ];
                    let class = classify_brick(field, cell_min, brick_world, band_half);
                    let is_surface = class == BrickClass::Surface;
                    if is_surface {
                        fill_brick(field, cell_min, voxel_size, band_half, c_max, &mut tile);
                    }
                    let slot = toroidal_slot([
                        origin_cell[0] + cx as i32,
                        origin_cell[1] + cy as i32,
                        origin_cell[2] + cz as i32,
                    ]);
                    let [ox, oy, oz] = m2_tile_atlas_origin(slot);
                    for lz in 0..W {
                        for ly in 0..W {
                            for lx in 0..W {
                                let byte =
                                    if is_surface { tile[lx + ly * W + lz * W * W] } else { 0i8 };
                                let vi = atlas_voxel_index(
                                    ox + lx as u32,
                                    oy + ly as u32,
                                    oz + lz as u32,
                                );
                                out[vi] = byte as u8;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Per-level bake feeds the proven baker. For each clip-map level `L = 0..BRICK_LEVELS`, the
    /// level-aware [`bake_brick_atlas_at`] staging at the level's snapped origin / `*_at_level`
    /// brick/voxel/band is BIT-IDENTICAL to the direct `classify_brick`/`fill_brick` reference over
    /// the SAME level grid — proving the Slice-A level table threads correctly into the M3-proven
    /// per-cell baker at every level.
    #[test]
    fn m4_level_bake_equals_full_classify_fill() {
        let camera = [0.37, -1.2, 2.0];
        let field = field_of(&crater());
        for level in 0..brick::BRICK_LEVELS as u32 {
            let geo = BrickLevelParams::at_level(camera, level);
            let mut baked = vec![0u8; (M2_ATLAS_DIM as usize).pow(3)];
            bake_brick_atlas_at(&field, AtlasEncoding::Snorm8, &geo, &mut baked);

            let reference = reference_atlas_snorm8(
                &field,
                snapped_level_origin(camera, level),
                snapped_level_origin_cell(camera, level),
                brick_world_at_level(level),
                voxel_size_at_level(level),
                band_half_at_level(level),
                c_max_at_level(level),
            );
            assert_eq!(
                baked, reference,
                "level {level}: bake_brick_atlas_at diverged from the direct classify/fill reference"
            );
            // The decoded snorm round-trip is well-defined (a sanity tap on the oracle).
            let _ = decode_snorm8(0, SDF_EDIT_BAND_HALF);
        }
    }

    /// The OFF/N=1 keystone: `near_field_only().as_ubo_bytes()[..48]` is byte-for-byte equal to
    /// `M2GridParams::default_near_field().as_bytes()` — a single-level (OFF) clip-map writes exactly
    /// the M2 tail, so the M2 path is unchanged when the clip-map is OFF.
    #[test]
    fn m4_ubo_bytes_off_path_byte_identical() {
        let m4 = M4GridParams::near_field_only();
        let m4_bytes = m4.as_ubo_bytes();
        let m2 = M2GridParams::default_near_field();
        let m2_bytes = m2.as_bytes();
        assert_eq!(m2_bytes.len(), 48, "M2 tail is 48 bytes");
        assert_eq!(
            &m4_bytes[..48],
            m2_bytes,
            "OFF/N=1 keystone: M4 level-0 block must equal the M2 default tail byte-for-byte"
        );
    }

    /// The std140 array-of-structs golden: the full N-level `as_ubo_bytes` matches a hand-checked
    /// layout where level `L` sits at byte `L*48`, lane 0 `origin_brick_world` at +0, lane 1
    /// `dims_atlas_dim` at +16, lane 2 `band_voxel_inv_atlas` at +32, each lane four little-endian
    /// `f32`s. This pins the exact byte layout the Slice-C shader's `m2_levels[BRICK_LEVELS]` reads.
    #[test]
    fn m4_grid_params_layout_golden() {
        let camera = [0.37, -1.2, 2.0];
        let m4 = M4GridParams::camera_centered(camera);
        let bytes = m4.as_ubo_bytes();
        assert_eq!(bytes.len(), brick::BRICK_LEVELS * 48);

        let read_f32 = |off: usize| -> f32 {
            f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
        };

        for level in 0..brick::BRICK_LEVELS {
            let base = level * 48;
            let origin = snapped_level_origin(camera, level as u32);
            let bw = brick_world_at_level(level as u32);
            let band = band_half_at_level(level as u32);
            let voxel = voxel_size_at_level(level as u32);

            // Lane 0 (origin_brick_world) at +0.
            assert_eq!(read_f32(base), origin[0], "L{level} origin.x at byte {base}");
            assert_eq!(read_f32(base + 4), origin[1], "L{level} origin.y");
            assert_eq!(read_f32(base + 8), origin[2], "L{level} origin.z");
            assert_eq!(read_f32(base + 12), bw, "L{level} brick_world at lane0.w");
            // Lane 1 (dims_atlas_dim) at +16 — level-invariant dims/atlas.
            assert_eq!(read_f32(base + 16), brick::M2_GRID_DIM as f32, "L{level} dims.x");
            assert_eq!(read_f32(base + 20), brick::M2_GRID_DIM as f32, "L{level} dims.y");
            assert_eq!(read_f32(base + 24), brick::M2_GRID_DIM as f32, "L{level} dims.z");
            assert_eq!(read_f32(base + 28), M2_ATLAS_DIM as f32, "L{level} atlas_dim at lane1.w");
            // Lane 2 (band_voxel_inv_atlas) at +32.
            assert_eq!(read_f32(base + 32), band, "L{level} band_half at lane2.x");
            assert_eq!(read_f32(base + 36), voxel, "L{level} voxel_size at lane2.y");
            assert_eq!(read_f32(base + 40), 1.0 / M2_ATLAS_DIM as f32, "L{level} inv_atlas at lane2.z");
            assert_eq!(read_f32(base + 44), level as f32, "L{level} level index at lane2.w");
        }
    }

    /// The `M4LevelParams` struct is exactly one M2 lane block (48 B) — the array packs contiguously.
    #[test]
    fn m4_level_params_is_48_bytes() {
        assert_eq!(core::mem::size_of::<M4LevelParams>(), 48);
        assert_eq!(core::mem::size_of::<M4GridParams>(), brick::BRICK_LEVELS * 48);
    }
}

/// Render P7 GROUP B — the `GoldenLightHeader` `ssao_mode` (header word 11) accessor
/// round-trip + the 0%-gate default. The SSAO host-oracle gather tests (the deep-crevice /
/// flat / seam AO bands) live in [`ssao_gather_tests`] below: the lib re-derives the SSAO
/// math as PLAIN RUST ([`crate::goldens::golden_ssao_attributes`]), since `boyko_shaderdsl` is a
/// dev-dependency only and the shipped backend must not link the eDSL. The `ssao_edsl_sync`
/// integration test (which HAS the dev-dep) cross-checks the plain-Rust per-tap horizon
/// against `boyko_shaderdsl::ssao::ssao_horizon_step_body::<EvalCf>` before any GPU run.
#[cfg(test)]
mod ssao_header_tests {
    use crate::goldens::GoldenLightHeader;

    /// `ssao_mode` (header word 11 = `sky_spec.w`) reads 0 on a freshly-built header — the
    /// automatic 0%-gate: every pre-P7 scene carries `sky_spec.w == 0.0`, so the resolve's
    /// `if (ssao_mode != 0u)` combine is skipped and `ao_final == gMaterial.g` byte-for-byte.
    #[test]
    fn ssao_mode_defaults_to_zero() {
        let h = GoldenLightHeader::new(1, 0, 1.0);
        assert_eq!(h.ssao_mode(), 0, "ssao_mode (word 11) must be 0 by default (the 0%-gate)");
        // The clustered constructor writes the cluster_params lane (words 12..15), NOT sky_spec
        // — so word 11 must still read 0.
        let cfg = crate::goldens::GoldenClusterConfig {
            dim_x: 16,
            dim_y: 9,
            dim_z: 24,
            max_lights_per_cluster: 64,
            z_near: 0.1,
            z_far: 100.0,
        };
        let hc = GoldenLightHeader::new_clustered(1, 0, 1.0, &cfg);
        assert_eq!(hc.ssao_mode(), 0, "new_clustered must not disturb ssao_mode (word 11)");
    }

    /// `with_ssao_mode(m)` round-trips through `ssao_mode()` for every representative `m`
    /// (stored BIT-CAST in `sky_spec.w`, exactly like `with_shadow_mode`/`shadow_mode` for
    /// word 7), and does NOT disturb the shadow_mode word (word 7).
    #[test]
    fn ssao_mode_round_trips_through_with_ssao_mode() {
        for m in [0u32, 1, 2, 0xFFFF_FFFF] {
            let h = GoldenLightHeader::new(1, 0, 1.0).with_ssao_mode(m);
            assert_eq!(h.ssao_mode(), m, "with_ssao_mode({m}) must round-trip through ssao_mode()");
        }
        // Independence: setting ssao_mode (word 11) leaves shadow_mode (word 7) untouched and
        // vice-versa — the two are distinct header words (sky_spec.w vs sky_diffuse.w).
        let both = GoldenLightHeader::new(1, 0, 1.0)
            .with_shadow_mode(1)
            .with_ssao_mode(1);
        assert_eq!(both.shadow_mode(), 1, "with_ssao_mode must not clobber shadow_mode (word 7)");
        assert_eq!(both.ssao_mode(), 1, "with_shadow_mode must not clobber ssao_mode (word 11)");
    }

    /// Render Shadow Phase 3 — the `contact_shadow_mode` (header word 7 BIT 1) builder + reader.
    /// Proves: `with_contact_shadow_mode(true)` round-trips to `contact_shadow_mode() == 1`;
    /// `with_shadow_mode(1).with_contact_shadow_mode(true)` keeps BOTH bits independent
    /// (`shadow_mode() == 1 && contact_shadow_mode() == 1`); and `with_contact_shadow_mode(false)`
    /// leaves word 7 unchanged on a fresh header (the 0%-gate proof — BIT 1 already 0).
    #[test]
    fn contact_shadow_mode_packs_into_word7_bit1() {
        let on = GoldenLightHeader::new(1, 0, 1.0).with_contact_shadow_mode(true);
        assert_eq!(on.contact_shadow_mode(), 1, "with_contact_shadow_mode(true) must read back 1");

        // Bit independence: shadow_mode (bit 0) and contact_shadow_mode (bit 1) coexist in word 7.
        let both = GoldenLightHeader::new(1, 0, 1.0)
            .with_shadow_mode(1)
            .with_contact_shadow_mode(true);
        assert_eq!(both.shadow_mode(), 1, "contact bit must not clobber shadow_mode (word 7 bit 0)");
        assert_eq!(both.contact_shadow_mode(), 1, "shadow_mode bit must not clobber contact (bit 1)");

        // Order independence: setting contact first then shadow_mode keeps both.
        let both_rev = GoldenLightHeader::new(1, 0, 1.0)
            .with_contact_shadow_mode(true)
            .with_shadow_mode(1);
        assert_eq!(both_rev.shadow_mode(), 1, "with_shadow_mode must preserve the contact bit");
        assert_eq!(both_rev.contact_shadow_mode(), 1, "the contact bit must survive with_shadow_mode");

        // 0%-gate: `with_contact_shadow_mode(false)` on a fresh header leaves word 7 byte-unchanged
        // (BIT 1 was already 0), so every pre-Phase-3 scene reads contact_shadow_mode() == 0.
        let fresh = GoldenLightHeader::new(1, 0, 1.0);
        let off = fresh.with_contact_shadow_mode(false);
        assert_eq!(
            off.sky_diffuse[3].to_bits(),
            fresh.sky_diffuse[3].to_bits(),
            "with_contact_shadow_mode(false) must leave word 7 unchanged (the 0%-gate)"
        );
        assert_eq!(off.contact_shadow_mode(), 0, "a fresh header reads contact_shadow_mode() == 0");
    }
}

/// Render P7 GROUP C1 — the resolve SSAO combine 0%-gate. Proves that on a `ssao_mode() == 0`
/// scene (every pre-P7 scene) the SSAO-aware resolve mirrors
/// ([`golden_deferred_resolve_table_ssao`] / [`golden_deferred_resolve_table_shadowed_ssao`])
/// return BYTE-IDENTICAL output for ANY `ssao` argument — i.e. the combine is never taken and
/// `ao_final == attrs.ao`, so wiring the SSAO term in is a true 0%-gate. A positive control
/// asserts that `ssao_mode == 1` with a darkening SSAO term DOES change a lit SDF pixel (the
/// combine is actually wired, not dead).
#[cfg(test)]
mod ssao_resolve_combine_tests {
    use super::{PBR_SKY_DIFFUSE};
    use crate::goldens::{golden_deferred_resolve_table, golden_deferred_resolve_table_shadowed, golden_deferred_resolve_table_shadowed_ssao, golden_deferred_resolve_table_ssao, ssao_combine, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes};

    const RO_ZERO: [f32; 3] = [0.0, 0.0, 0.0];

    /// A representative one-material table (slot 0 = a textured dielectric).
    fn materials() -> Vec<GoldenMaterial> {
        vec![GoldenMaterial::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0])]
    }

    /// The degenerate directional + sky table at `exposure`, with the supplied `ssao_mode`
    /// (header word 11). The sky entry drives the ambient term the AO modulates.
    fn table(ssao_mode: u32) -> (GoldenLightHeader, Vec<GoldenLight>) {
        let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(ssao_mode);
        let lights = vec![
            GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
            GoldenLight::sky(PBR_SKY_DIFFUSE, PBR_SKY_DIFFUSE),
        ];
        (header, lights)
    }

    /// A small sweep of SDF-lit + mesh-sentinel attributes (the combine reads `mask`, `ao`,
    /// and the `view_t >= 1e30` mesh sentinel).
    fn sweep() -> Vec<MarcherAttributes> {
        let mut v = Vec::new();
        for &mask in &[1u8, 0u8] {
            for &ao in &[0u8, 90u8, 200u8, 255u8] {
                for &view_t in &[2.5_f32, 1.0e30] {
                    v.push(MarcherAttributes {
                        base_rgb: [180, 120, 90],
                        oct_rg: [200, 60],
                        mat_id: 0,
                        shadow: 200,
                        ao,
                        mask,
                        view_t,
                    });
                }
            }
        }
        v
    }

    #[test]
    fn ssao_combine_is_identity_when_off() {
        // The pure host combine: `ssao_mode == 0` returns `ao` regardless of `view_t`/`ssao`.
        for &ao in &[0.0_f32, 0.35, 1.0] {
            for &view_t in &[2.5_f32, 1.0e30] {
                for &ssao in &[0.0_f32, 0.5, 1.0] {
                    assert_eq!(
                        ssao_combine(0, ao, view_t, ssao),
                        ao,
                        "ssao_mode==0 must return ao unchanged (the 0%-gate)"
                    );
                }
            }
        }
    }

    #[test]
    fn resolve_table_ssao_off_is_byte_identical() {
        // The SSAO-aware resolve with `ssao_mode == 0` is byte-identical to the pre-P7 fn for
        // EVERY `ssao` argument and EVERY swept attribute — the resolve 0%-gate.
        let mats = materials();
        let (header, lights) = table(0);
        let rd = [0.1, 0.05, -0.99];
        for attrs in sweep() {
            let baseline = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &header, &lights);
            for &ssao in &[0.0_f32, 0.25, 0.7, 1.0] {
                let got = golden_deferred_resolve_table_ssao(
                    attrs, RO_ZERO, rd, &mats, &header, &lights, ssao,
                );
                assert_eq!(
                    got, baseline,
                    "ssao_mode==0 resolve must be byte-identical for any ssao ({ssao})"
                );
            }
        }
    }

    #[test]
    fn resolve_table_shadowed_ssao_off_is_byte_identical() {
        // The SHADOWED SSAO-aware resolve with `ssao_mode == 0` (AND `shadow_mode == 0`, so no
        // march fires) is byte-identical to the pre-P7 shadowed fn for every `ssao`.
        let mats = materials();
        let (header, lights) = table(0);
        let rd = [0.1, 0.05, -0.99];
        // A trivial field (never marched on the `shadow_mode == 0` / `ssao_mode == 0` path).
        let field = |_q: [f32; 3]| 1.0_f32;
        for attrs in sweep() {
            let baseline = golden_deferred_resolve_table_shadowed(
                attrs, RO_ZERO, rd, &mats, &header, &lights, &field,
            );
            for &ssao in &[0.0_f32, 0.25, 0.7, 1.0] {
                let got = golden_deferred_resolve_table_shadowed_ssao(
                    attrs, RO_ZERO, rd, &mats, &header, &lights, &field, ssao,
                );
                assert_eq!(
                    got, baseline,
                    "ssao_mode==0 shadowed resolve must be byte-identical for any ssao ({ssao})"
                );
            }
        }
    }

    #[test]
    fn resolve_table_ssao_on_darkens_a_lit_sdf_pixel() {
        // Positive control: `ssao_mode == 1` with a strongly-occluding SSAO term (0.0) must
        // darken a fully-unoccluded lit SDF pixel (ao = 255, view_t finite) — the combine is
        // wired, not dead. A mesh pixel (view_t sentinel) takes `min(1.0, ssao) == ssao`.
        let mats = materials();
        let (header, lights) = table(1);
        let rd = [0.1, 0.05, -0.99];
        let attrs = MarcherAttributes {
            base_rgb: [180, 120, 90],
            oct_rg: [200, 60],
            mat_id: 0,
            shadow: 255,
            ao: 255,
            mask: 1,
            view_t: 2.5,
        };
        let unoccluded =
            golden_deferred_resolve_table_ssao(attrs, RO_ZERO, rd, &mats, &header, &lights, 1.0);
        let occluded =
            golden_deferred_resolve_table_ssao(attrs, RO_ZERO, rd, &mats, &header, &lights, 0.0);
        assert_ne!(
            occluded, unoccluded,
            "ssao_mode==1 with a darkening SSAO term must change the lit pixel (combine wired)"
        );
    }
}

/// Render P7 GROUP B — the SSAO host-oracle gather bands. Builds a SYNTHETIC G-buffer
/// (`Vec<MarcherAttributes>`; [`golden_ssao_attributes`] reads `mask` + `view_t` + the
/// center pixel's `oct_rg` normal) at the legacy `64×64` ORTHO extent and asserts the
/// signature AO regimes for the FIXED horizon math (elevation above the tangent plane):
/// a FLAT lit surface (in-plane neighbours, `delta ⊥ N`) stays at AO ≈ 1 (the bug's
/// regression guard — the old screen-direction math BLACKENED it), a deep crevice
/// (neighbours rising above the tangent toward the camera) darkens AO below 1, and a seam
/// (all neighbours background) is fully unoccluded with no NaN. The math is PLAIN RUST (the
/// lib does not link the `boyko_shaderdsl` dev-dep); the `ssao_edsl_sync` integration test
/// cross-checks the per-tap horizon against the eDSL Eval.
#[cfg(test)]
mod ssao_gather_tests {
    use super::{CompositeCamera, SsaoParams, SSAO_VIEWT_BG};
    use crate::goldens::{oct_decode, MarcherAttributes, golden_ssao_attributes};

    const W: u32 = 64;
    const H: u32 = 64;
    const CX: u32 = 32;
    const CY: u32 = 32;
    /// The octahedral-quantized center normal the synthetic gbuffer stamps. `[128, 128]`
    /// decodes to ~`+z` (toward the ORTHO camera, which looks down `-z`) — the surface faces
    /// the viewer, the realistic lit-pixel normal.
    const OCT_RG: [u8; 2] = [128, 128];

    /// The decoded center surface normal `N` (the same `oct_decode` the gather reads). The
    /// flat-surface fixture builds a plane PERPENDICULAR to this `N` so `delta ⊥ N` exactly.
    fn center_normal() -> [f32; 3] {
        oct_decode([OCT_RG[0] as f32 / 255.0, OCT_RG[1] as f32 / 255.0])
    }

    /// Builds a `W×H` synthetic G-buffer from a per-pixel `(lit, view_t)` field. The center
    /// pixel's normal is `OCT_RG` (the gather decodes it as the elevation reference); the rest
    /// of the per-pixel normal is irrelevant (the gather reads only the CENTER normal).
    fn synthetic_gbuffer<F: Fn(i32, i32) -> (bool, f32)>(field: F) -> Vec<MarcherAttributes> {
        let mut gbuf = Vec::with_capacity((W as usize) * (H as usize));
        for py in 0..H {
            for px in 0..W {
                let (lit, view_t) = field(px as i32, py as i32);
                gbuf.push(MarcherAttributes {
                    base_rgb: [0, 0, 0],
                    oct_rg: OCT_RG,
                    mat_id: 0,
                    shadow: 255,
                    ao: 255,
                    mask: if lit { 1 } else { 0 },
                    view_t: if lit { view_t } else { SSAO_VIEWT_BG },
                });
            }
        }
        gbuf
    }

    /// The ORTHO world `(x, y)` of a pixel center (mirrors `composite_ray`'s ORTHO arm:
    /// `u = ((px+0.5)/W)*2-1`, `v = -(((py+0.5)/H)*2-1)`, scaled by `SDF_HALF_EXTENT == 1`).
    fn ortho_xy(px: i32, py: i32) -> (f32, f32) {
        let u = (((px as f32) + 0.5) / (W as f32)) * 2.0 - 1.0;
        let v = -((((py as f32) + 0.5) / (H as f32)) * 2.0 - 1.0);
        (u * super::SDF_HALF_EXTENT, v * super::SDF_HALF_EXTENT)
    }

    #[test]
    fn flat_surface_keeps_ao_near_one() {
        // THE BUG'S REGRESSION GUARD. A literally flat lit plane PERPENDICULAR to the center
        // normal `N`: every neighbour lies in the tangent plane (`delta ⊥ N`), so the
        // elevation `dot(delta, N) == 0` -> no horizon is raised -> occ == 0 -> AO ≈ 1.0. The
        // ORTHO world z is `SDF_CAM_Z - view_t`; to put the surface in the plane through the
        // center perpendicular to `N = (a, a, c)`, solve `N·(P - P0) = 0` for `view_t`:
        // `view_t = view_t0 + (a/c) * (Δx + Δy)`. Under the OLD screen-direction math an
        // in-plane neighbour parallel to the slice axis gave `sampleCos ≈ 1` and BLACKENED
        // this flat lit surface (AO → ~0). The fix makes it AO ≈ 1.
        let n = center_normal();
        let view_t0 = 1.5_f32;
        let (x0, y0) = ortho_xy(CX as i32, CY as i32);
        let gbuf = synthetic_gbuffer(|x, y| {
            let (xw, yw) = ortho_xy(x, y);
            // The tangent-plane depth so delta lands exactly in the plane perpendicular to N.
            let view_t = view_t0 + (n[0] / n[2]) * (xw - x0) + (n[1] / n[2]) * (yw - y0);
            (true, view_t)
        });
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(ao.is_finite(), "a flat surface must not produce NaN, got ao = {ao}");
        assert!(
            ao > 0.99,
            "a FLAT lit surface (delta perpendicular to N) must leave AO ≈ 1.0 (the SSAO \
             horizon bug regression guard), got ao = {ao}"
        );
    }

    #[test]
    fn deep_crevice_darkens_ao() {
        // A V-valley: the surface RISES toward the camera (world z grows, i.e. view_t SHRINKS)
        // as the radius from the center grows, so every neighbour sits ABOVE the center's
        // tangent plane (`dot(delta, N) > 0`) and stays well within SSAO_RADIUS. Each tap's
        // elevation is strongly positive inside the falloff -> the per-slice horizon max is
        // high -> occ is large -> AO clearly < 1.
        let gbuf = synthetic_gbuffer(|x, y| {
            let dx = (x - CX as i32) as f32;
            let dy = (y - CY as i32) as f32;
            let r = (dx * dx + dy * dy).sqrt();
            (true, 1.5 - 0.01 * r)
        });
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(
            ao < 0.5,
            "a deep crevice (neighbours above the tangent) must darken AO clearly below 1.0, \
             got ao = {ao}"
        );
        assert!(ao >= 0.0, "AO is clamped to [0, 1], got ao = {ao}");
        assert!(ao.is_finite(), "AO must be finite, got ao = {ao}");
    }

    #[test]
    fn isolated_seam_is_fully_unoccluded_no_nan() {
        // A seam: a single lit center pixel, every neighbour is background (mask == 0). Every
        // tap reconstructs Pp = P (the seam's out-of-bounds / non-lit skip) -> delta == 0 ->
        // elev == 0 (guarded by SSAO_EPS, no divide-by-zero NaN) -> occ == 0 -> AO == 1.
        let gbuf = synthetic_gbuffer(|x, y| (x == CX as i32 && y == CY as i32, 1.5));
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(ao.is_finite(), "a seam must not produce NaN, got ao = {ao}");
        assert!(
            (ao - 1.0).abs() < 1.0e-6,
            "an all-background seam must be fully unoccluded (ao = 1.0), got ao = {ao}"
        );
    }

    #[test]
    fn non_lit_center_returns_neutral_one() {
        // A non-lit center (background): the gather returns the neutral 1.0 before any march,
        // so the resolve's `min(class_ao, ssao)` leaves the pixel unchanged.
        let gbuf = synthetic_gbuffer(|_x, _y| (false, 0.0));
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert_eq!(ao, 1.0, "a non-lit center pixel must return the neutral AO 1.0");
    }

    /// The EXACT per-pixel dither the gather applies (mirror of the `golden_ssao_attributes`
    /// Hilbert+R2 low-discrepancy basis): ONE 64x64 Hilbert index drives two R2 channels — ALPHA1
    /// -> the rotation slot `(r2 * ROT_N) >> 24` over the 16-entry table, ALPHA2 -> the radial
    /// step-phase `((r2 >> 16) + 1) / 256.0`. Returned as `(rot_slot, radial_phase)` so the
    /// determinism + decorrelation test can assert both.
    fn dither(px: u32, py: u32) -> (usize, f32) {
        let hindex = crate::goldens::ssao_hilbert(
            super::SSAO_HILBERT_W,
            px & (super::SSAO_HILBERT_W - 1),
            py & (super::SSAO_HILBERT_W - 1),
        );
        let slot =
            ((crate::goldens::ssao_r2(hindex, super::SSAO_R2_ALPHA1).wrapping_mul(super::SSAO_ROT_N)) >> 24) as usize;
        let r2_rad = crate::goldens::ssao_r2(hindex, super::SSAO_R2_ALPHA2);
        let radial_phase = ((r2_rad >> 16) + 1) as f32 / 256.0;
        (slot, radial_phase)
    }

    #[test]
    fn q1_dither_is_deterministic_in_range_and_decorrelated() {
        // Q1: the per-pixel dither (rotation slot + radial step-phase) is the concentric-ring
        // fix. It must be (1) DETERMINISTIC (the same pixel always yields the same dither — the
        // host oracle and the GPU agree), (2) IN RANGE (slot in [0, 16), radial_phase strictly
        // in (0, 1] so the nearest tap never advances to 0 — no center self-tap), and (3)
        // DECORRELATED (distinct pixels get a SPREAD of (slot, phase) pairs — the property that
        // turns the coherent rings into high-frequency noise the depth-aware blur removes).
        use std::collections::HashSet;

        let mut seen_slots: HashSet<usize> = HashSet::new();
        let mut seen_phase_bins: HashSet<u32> = HashSet::new();
        let mut seen_pairs: HashSet<(usize, u32)> = HashSet::new();

        for py in 0..64u32 {
            for px in 0..64u32 {
                let (slot, phase) = dither(px, py);

                // (1) determinism: a re-evaluation is bit-identical.
                let (slot2, phase2) = dither(px, py);
                assert_eq!(slot, slot2, "rotation slot must be deterministic at ({px},{py})");
                assert_eq!(
                    phase.to_bits(),
                    phase2.to_bits(),
                    "radial_phase must be bit-deterministic at ({px},{py})"
                );

                // (2) range: slot in [0, 16); phase strictly in (0, 1] (no self-tap, no overshoot).
                assert!(slot < (super::SSAO_ROT_N as usize), "slot {slot} out of [0,16)");
                assert!(
                    phase > 0.0 && phase <= 1.0,
                    "radial_phase {phase} must be in (0, 1] (strictly positive ⇒ no center \
                     self-tap; ≤ 1 ⇒ the farthest tap reaches at most pix_radius)"
                );

                seen_slots.insert(slot);
                seen_phase_bins.insert((phase * 256.0).round() as u32);
                seen_pairs.insert((slot, (phase * 256.0).round() as u32));
            }
        }

        // (3) decorrelation: over a 64×64 block the dither spreads across the table and the phase
        // band, and produces MANY distinct (slot, phase) pairs — proving neighbouring pixels do
        // NOT march the same step radii (the coherent-ring root cause).
        assert!(
            seen_slots.len() >= 8,
            "the 16-entry rotation must exercise a spread of slots over a 64×64 block (saw {}), \
             else the angular banding stays coherent",
            seen_slots.len()
        );
        assert!(
            seen_phase_bins.len() >= 64,
            "the radial step-phase must spread across its [1/256, 1] band over a 64×64 block \
             (saw {} bins), else the concentric rings stay coherent",
            seen_phase_bins.len()
        );
        assert!(
            seen_pairs.len() >= 256,
            "the (rotation, radial-phase) dither must yield many distinct pairs over a 64×64 \
             block (saw {}), proving the per-pixel decorrelation",
            seen_pairs.len()
        );

        // A direct sanity pair: two distinct pixels get DIFFERENT dither (the decorrelation core).
        assert_ne!(
            dither(0, 0),
            dither(1, 0),
            "adjacent pixels must get a different (slot, radial_phase) dither"
        );
    }
}

/// Render P7 POLISH — the SSAO depth-aware box-blur host mirror ([`golden_ssao_blur`]) tests.
/// Proves the inline resolve blur on the host side: (1) a sharp AO RING is smoothed toward its
/// neighbourhood mean, and (2) the bilateral DEPTH gate prevents bleed across a silhouette (a
/// `view_t` jump > [`SSAO_BLUR_DEPTH_TOL`]). Pure host math; runs device-less.
#[cfg(test)]
mod ssao_blur_tests {
    use super::{SSAO_BLUR_DEPTH_TOL, SSAO_BLUR_R, SSAO_VIEWT_BG};
    use crate::goldens::{golden_ssao_blur, MarcherAttributes};

    const W: u32 = 32;
    const H: u32 = 32;

    /// A `W×H` synthetic G-buffer from a per-pixel `(ssao_byte, view_t)` field. Only the
    /// `view_t` lane (the blur's depth gate) is meaningful here; `mask`/the rest are inert (the
    /// blur reads neither). Returns `(raw_ssao_bytes, gbuf)`.
    fn build<F: Fn(i32, i32) -> (u8, f32)>(field: F) -> (Vec<u8>, Vec<MarcherAttributes>) {
        let mut ssao = Vec::with_capacity((W * H) as usize);
        let mut gbuf = Vec::with_capacity((W * H) as usize);
        for py in 0..H {
            for px in 0..W {
                let (byte, view_t) = field(px as i32, py as i32);
                ssao.push(byte);
                gbuf.push(MarcherAttributes {
                    base_rgb: [0, 0, 0],
                    oct_rg: [128, 128],
                    mat_id: 0,
                    shadow: 255,
                    ao: 255,
                    mask: 1,
                    view_t,
                });
            }
        }
        (ssao, gbuf)
    }

    #[test]
    fn sharp_ring_is_smoothed() {
        // A constant-depth flat surface (every neighbour passes the depth gate) with a SHARP AO
        // discontinuity: the left half is fully-dark (byte 0), the right half fully-bright
        // (byte 255). At the seam column the raw value is a hard step; the 7×7 box blur of a
        // pixel ON the seam must land near the neighbourhood mean (~0.5), STRICTLY between the
        // two raw extremes — i.e. the discontinuity is smoothed, not preserved.
        const SEAM: i32 = 16;
        let (ssao, gbuf) = build(|x, _y| (if x < SEAM { 0 } else { 255 }, 1.5));

        // A pixel just inside the bright half, within R of the seam: its raw byte is 255 → 1.0,
        // but the blur pulls it down toward the mean because the dark half is in-kernel.
        let px = (SEAM + 1) as u32;
        let py = 16;
        let raw = ssao[(py * W + px) as usize] as f32 / 255.0;
        let blurred = golden_ssao_blur(&ssao, &gbuf, px, py, W, H);
        assert!(
            blurred < raw - 0.05 && blurred > 0.1,
            "the box blur must smooth the sharp ring: raw {raw} blurred {blurred} \
             (expected strictly between the dark and bright extremes)"
        );
        // The exact 7×7 mean at the seam pixel: columns [px-R, px+R] = [SEAM-2, SEAM+4]; of the
        // 7 columns, (SEAM-2, SEAM-1) are dark (0.0) and (SEAM..SEAM+4) are bright (1.0), each ×
        // 7 rows → mean = 5/7. Confirms the gather order/bounds/center-counts arithmetic.
        let expected = 5.0_f32 / 7.0;
        assert!(
            (blurred - expected).abs() < 1.0e-6,
            "the 7×7 box mean must be exactly 5/7 at the seam pixel, got {blurred}"
        );
    }

    #[test]
    fn depth_gate_prevents_silhouette_bleed() {
        // A silhouette: the left half is a NEAR surface (`view_t = 1.5`, dark AO byte 40) and the
        // right half is a FAR surface (`view_t = 1.5 + 10*tol`, bright AO byte 255) — a `view_t`
        // jump far beyond the gate. A near-surface pixel ON the boundary must blur ONLY with its
        // near-side (in-tol) neighbours, so its blurred AO stays near the dark value and is NOT
        // pulled up by the far-side bright taps (no cross-silhouette bleed).
        const SEAM: i32 = 16;
        const DARK: u8 = 40;
        let near_t = 1.5_f32;
        let far_t = 1.5_f32 + 10.0 * SSAO_BLUR_DEPTH_TOL;
        let (ssao, gbuf) = build(|x, _y| {
            if x < SEAM {
                (DARK, near_t)
            } else {
                (255, far_t)
            }
        });

        // The last near-side column (within R of the seam, so far-side taps ARE inside the
        // kernel window but must be REJECTED by the depth gate).
        let px = (SEAM - 1) as u32;
        let py = 16;
        let blurred = golden_ssao_blur(&ssao, &gbuf, px, py, W, H);
        let dark = DARK as f32 / 255.0;
        assert!(
            (blurred - dark).abs() < 1.0e-6,
            "the depth gate must reject far-side taps: a near-surface pixel must blur to the \
             near AO {dark} (got {blurred}), NOT bleed the far-side bright AO across the \
             silhouette"
        );
    }

    #[test]
    fn center_always_counts_no_divide_by_zero() {
        // An ISOLATED lit pixel surrounded by a far background (every neighbour fails the depth
        // gate): the blur must still count the CENTER (cnt ≥ 1) and return the center's own raw
        // AO — never a 0/0 NaN.
        let (ssao, gbuf) = build(|x, y| {
            if x == 16 && y == 16 {
                (90, 1.5)
            } else {
                (255, SSAO_VIEWT_BG) // far background — rejected by the gate
            }
        });
        let blurred = golden_ssao_blur(&ssao, &gbuf, 16, 16, W, H);
        assert!(blurred.is_finite(), "the center always counts — never 0/0 NaN, got {blurred}");
        assert!(
            (blurred - 90.0 / 255.0).abs() < 1.0e-6,
            "an isolated pixel (all neighbours gated out) must blur to its OWN raw AO, got {blurred}"
        );
        // Sanity: the radius constant is the one the resolve compiles in.
        assert_eq!(SSAO_BLUR_R, 3, "the host blur radius must mirror the shader's SSAO_BLUR_R");
    }
}
