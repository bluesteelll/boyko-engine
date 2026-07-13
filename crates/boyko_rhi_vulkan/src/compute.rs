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

/// Embeds a committed `.spv` as a 4-byte-aligned [`SpirvBlob`] whose length is
/// derived from the file itself, so there is no hand-counted size to drift out of
/// sync. Any leading doc comments and `#[cfg(...)]` attributes are forwarded to the
/// generated `static`. `$path` is written ONCE (the macro reuses it for both the
/// length and the bytes, so the two can never disagree).
macro_rules! embed_spirv {
    ($(#[$meta:meta])* $name:ident, $path:expr) => {
        $(#[$meta])*
        static $name: SpirvBlob<{ include_bytes!($path).len() }> =
            SpirvBlob(*include_bytes!($path));
    };
}

embed_spirv! {
    /// The committed SPIR-V for step 0c (`buffer[i] = i*2 + 1`).
    ///
    /// Wrapped in a `#[repr(C, align(4))]` newtype so the `include_bytes!` blob is
    /// 4-byte aligned: it is reinterpreted as a `&[u32]` word stream, and the SPIR-V
    /// spec requires that stream to be 4-byte aligned (a bare `include_bytes!` is
    /// only `align(1)`).
    WRITE_PATTERN_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/write_pattern.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for step 0d (`buffer[i] += 100`).
    TRANSFORM_ADD_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/transform_add.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for Phase-6 rung 8 (sphere-trace one analytic sphere into
    /// a packed-RGBA storage buffer — `shaders/sdf_spheretrace.hlsl`).
    SDF_SPHERETRACE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_spheretrace.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for Phase-6 rung 9 (sphere-trace an ordered SDF edit-list
    /// — multi-primitive CSG — into a packed-header storage buffer,
    /// `shaders/sdf_editlist.hlsl`).
    SDF_EDITLIST_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_editlist.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for Phase-6 rung 10 (SDF + mesh hybrid composite via a
    /// shared depth buffer — `shaders/sdf_depth_composite.hlsl`). It reuses the rung-9
    /// edit-list fold + lighting + camera verbatim and adds the per-pixel mesh-depth
    /// read that BOUNDS the march so the mesh and the SDF occlude each other.
    SDF_DEPTH_COMPOSITE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_depth_composite.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for the Render P1a GPU gate (sphere-trace the rung-9 SDF
    /// edit-list and STORE the marcher color into a STORAGE IMAGE through the
    /// multi-resource descriptor *vocabulary* set — `shaders/sdf_editlist_storage_image.hlsl`).
    /// Reuses the rung-9 field eval + ray-gen + lighting VERBATIM; the only differences
    /// are the bind points (binding 0 = a read-only `StructuredBuffer<uint>` edit-list,
    /// binding 1 = a `RWTexture2D<float4>` output) and the output sink (the marcher color
    /// is STORED to texel `(px, py)` instead of packed into the buffer's pixel region).
    SDF_EDITLIST_STORAGE_IMAGE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_editlist_storage_image.comp.spv")
}

embed_spirv! {
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
    SDF_GBUFFER_COMPOSITE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_gbuffer_composite.comp.spv")
}

embed_spirv! {
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
    /// `ssao_mode == 0` never executes the loop). [Later SUPERSEDED: the denoise moved OUT of the
    /// resolve into the `ssao_atrous.comp` edge-avoiding à-trous pass chain; the resolve now reads the
    /// already-filtered `gSsao` directly. The host mirror is now `golden_ssao_atrous`.]; ... bytes.
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
    /// byte-identical pixels (the 0%-gate); only `ddgi_indirect=true` samples. Textured-PBR T6a: the
    /// `gPbr` STORAGE image @19 (SOFTWARE-ONLY, `#if !HWRT`) + the `MATERIAL_FLAG_TEXTURED_BIT`-gated
    /// metallic/roughness/AO/emissive override — the file grows further; the flag bit is 0 on every
    /// current material (the injection never dynamically fires) → byte-identical pixels (the 0%-gate).
    /// The HWRT-family `.spv` below are BYTE-IDENTICAL to their pre-T6a state (verified by recompile
    /// diff — everything T6a adds is inside `#if !HWRT`/`#else`-mirrored blocks).
    DEFERRED_PBR_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr.comp.spv")
}

embed_spirv! {
    /// The Render terminator-softening SOFTWARE-RESOLVE-ONLY variant SPIR-V
    /// (`shaders/deferred_pbr_wrap.comp.spv`, compiled `-T cs_6_0 -D TERMINATOR_WRAP=1`).
    ///
    /// Compiled from the SAME `deferred_pbr.hlsl` with the `#if TERMINATOR_WRAP` diffuse
    /// light-wrap arm active at both direct-light accumulation sites (`diff * nol_wrapped(NoL,
    /// ts) + spec * NoL` instead of the physical `(diff + spec) * NoL` clamp) — the frozen-base
    /// discipline (mirrors `gbuffer_mrt.fs.hlsl`'s `#ifdef` variants): [`DEFERRED_PBR_SPV`]
    /// above (`TERMINATOR_WRAP` undefined) preprocesses CHARACTER-IDENTICAL to the pre-feature
    /// source, so this variant is a strictly ADDITIVE compile that never perturbs the base
    /// module's bytes (a runtime `if (ts > 0.0) {..} else {..}` guard was tried first and
    /// rejected: the mere PRESENCE of the extra branch/loads drifted DXC's FMA fusion in the
    /// base module even on the `ts == 0` path).
    ///
    /// Selected by the host ONLY when `LightingConfig::terminator_softening > 0` (the
    /// `gpu_scene::GpuSceneBundles::scene` `terminator_wrap` gate); every other frame binds
    /// [`DEFERRED_PBR_SPV`]. Reuses the SAME 20-binding software resolve layout as
    /// [`DEFERRED_PBR_SPV`] (the variant changes only diffuse-accumulation math, no
    /// descriptor), so no separate bind-group layout is built for it. An `HWRT +
    /// TERMINATOR_WRAP` combo is explicitly OUT OF SCOPE this rung — never compiled, never
    /// selected (the HWRT-family `.spv` below are unaffected by this variant).
    DEFERRED_PBR_WRAP_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_wrap.comp.spv")
}

embed_spirv! {
    /// The R2a-4b HWRT-variant deferred-resolve SPIR-V (`shaders/deferred_pbr_hwrt.comp.spv`, compiled
    /// `-T cs_6_5 -D HWRT=1`). Compiled from the SAME `deferred_pbr.hlsl` with the `#if HWRT` mesh-shadow
    /// arm active: the primary directional's mesh-shadow term routes to a SOFT inline `rayQuery` cone
    /// trace (`SHADOW_RAY_COUNT` Vogel-disk rays within the sun's angular cone against the binding-19
    /// `RaytracingAccelerationStructure`, averaged) instead of the CSM shadow-map sample, so the module
    /// carries `OpCapability RayQueryKHR` + `SPV_KHR_ray_query` + the 20th descriptor. Gated behind
    /// `feature = "hwrt"` + a runtime `ctx.ray_query_enabled()` +
    /// `RayBackendConfig.table[Shadow][Mesh] == HardwareTri`; THIS `.spv` stays byte-verbatim across
    /// changes scoped to the software-only `#if !HWRT` arm (e.g. textured-PBR T6a's `gPbr` binding +
    /// flag-gated override) — the `#else` arm this file compiles is byte-verbatim, verified by a
    /// recompile temp-diff. The SOFTWARE `.spv` above is NOT byte-frozen (it grows per rung; the
    /// PIXEL-GOLDEN, not a byte count, is its authority).
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_HWRT_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt.comp.spv")
}

embed_spirv! {
    /// The Rung-3a VIS-variant deferred-resolve SPIR-V (`shaders/deferred_pbr_hwrt_vis.comp.spv`,
    /// compiled from `deferred_pbr.hlsl` with `SHADOW_STAGE=1`). Runs the SAME primary-directional
    /// inline `rayQuery` Vogel-disk cone trace as [`DEFERRED_PBR_HWRT_SPV`] (RESOLVE_INLINE) — same
    /// `SHADOW_RAY_COUNT` spec-const, same live inputs, so `mesh_vis` is bit-identical — but instead of
    /// combining it into the lighting it **writes** `gShadowVis[px,py] = RG(mesh_vis, validity)` to the
    /// 22nd descriptor (`RWTexture2D<float2>` @21) and RETURNS (lighting stripped). Non-mesh-arm pixels
    /// write `RG(1.0, 0.0)`. This is the à-trous pre-pass. Bound to the 22-binding VIS/DENOISED layout
    /// (the 21-binding RESOLVE_INLINE-hwrt layout + `gShadowVis` @21); gated behind `feature = "hwrt"` +
    /// `ctx.ray_query_enabled()` and only ever dispatched when `scene.shadow.is_some()`.
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_VIS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt_vis.comp.spv")
}

embed_spirv! {
    /// The Rung-3a DENOISED-variant deferred-resolve SPIR-V
    /// (`shaders/deferred_pbr_hwrt_denoised.comp.spv`, compiled from `deferred_pbr.hlsl` with
    /// `SHADOW_STAGE=2`). Identical to [`DEFERRED_PBR_HWRT_SPV`] (RESOLVE_INLINE) except the inline
    /// Vogel trace is replaced by a single `mesh_vis = gShadowVis.Load(px,py).r` (reading the FINAL
    /// à-trous output at descriptor @21), then the identical `vis = min(vis, mesh_vis)` combine and
    /// full lighting. It does NOT trace, so it references no acceleration structure and declares no
    /// `SHADOW_RAY_COUNT` spec-const. Bound to the SAME 22-binding VIS/DENOISED layout; selected as the
    /// resolve pipeline only when `scene.shadow.is_some()`.
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_DENOISED_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt_denoised.comp.spv")
}

embed_spirv! {
    /// The Rung-3b step-5b MOTION_VECTORS VIS-variant deferred-resolve SPIR-V
    /// (`shaders/deferred_pbr_hwrt_vis_mv.comp.spv`, compiled from `deferred_pbr.hlsl` with
    /// `SHADOW_STAGE=1 + MOTION_VECTORS`). Identical to [`DEFERRED_PBR_VIS_SPV`] (writes `gShadowVis`
    /// @21) except it ALSO writes each SDF pixel's CAMERA-ONLY motion vector `Δuv` to a `motion_vec`
    /// STORAGE image @23 (rg16), reprojecting the reconstructed surface `P` through a `MotionCam` UBO
    /// @22 (cur+prev marcher-aligned view-proj — the SAME 128 B pair the raster MV variant reads). Mesh
    /// pixels are raster-owned (the gbuffer MV variant); the two producers write disjoint pixels of one
    /// `motion_vec`. Bound to a 24-binding VIS-MV layout (the 22 VIS bindings + `MotionCam` @22 +
    /// `motion_vec` @23); selected instead of [`DEFERRED_PBR_VIS_SPV`] only when the temporal denoiser
    /// is active. The base VIS `.spv` stays the byte-frozen 8032-byte golden.
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_VIS_MV_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt_vis_mv.comp.spv")
}

embed_spirv! {
    /// The Rung-3a à-trous spatial shadow-denoise filter SPIR-V (`shaders/shadow_atrous.comp.spv`,
    /// Dammertz 2010). A 2D 25-tap/level (5×5 B3-spline) edge-stopping wavelet: `levels` iterations,
    /// `step = 1 << level` (a 4-byte `{ uint step; }` push-const), edge-stop weight
    /// `w = h · pow(max(0,dot(n_t,n_c)),σ_n) · exp(-|z_t-z_c| / (σ_z·|o·step|+eps)) · valid_t`,
    /// normalized `Σ(w·vis)/Σw`. Bound to its OWN 6-binding layout { @0 `gVisIn` (RG read), @1
    /// `gVisOut` (RG write), @2 `gNormal`, @3 `gViewT`, @4 `ResolvedShadowDenoise` UBO (16 B,
    /// σ_z/σ_n), @5 the shared 80-byte Camera UBO }. Ping-ponged between the `shadow_vis` /
    /// `shadow_vis2` targets, one dispatch per level.
    #[cfg(feature = "hwrt")]
    SHADOW_ATROUS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/shadow_atrous.comp.spv")
}

embed_spirv! {
    /// The SSAO à-trous edge-avoiding denoise filter, INTERIOR pin (`shaders/ssao_atrous.comp.spv`,
    /// `ssao_atrous.comp.hlsl` compiled with no `-D`): `gAoIn`/`gAoOut` both `r16`. Bound to the SHARED
    /// 4-binding à-trous layout { @0 `gAoIn`, @1 `gAoOut`, @2 `gViewT`, @3 the shared 80-byte Camera
    /// UBO } + a 4-byte `{ uint step; }` push. Selected for every level EXCEPT the first (reads the R8
    /// gather) and the last (writes back to the R8 `gSsao`) — software (NOT `hwrt`-gated: the SSAO
    /// denoise gate is depth-plane-fit only, no `rayQuery`).
    SSAO_ATROUS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ssao_atrous.comp.spv")
}

embed_spirv! {
    /// The SSAO à-trous edge-avoiding denoise filter, READ-R8 pin
    /// (`shaders/ssao_atrous_read8.comp.spv`, `-D SSAO_ATROUS_READ_R8=1`): `gAoIn` pinned `r8` (reads
    /// the `sdf_ssao` gather's raw R8_UNORM output), `gAoOut` pinned `r16`. Selected for LEVEL 0 only
    /// (`N >= 2`). Same 4-binding layout as [`SSAO_ATROUS_SPV`]; only the `gAoIn` `OpTypeImage` pin
    /// differs.
    SSAO_ATROUS_READ8_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ssao_atrous_read8.comp.spv")
}

embed_spirv! {
    /// The SSAO à-trous edge-avoiding denoise filter, WRITE-R8 pin
    /// (`shaders/ssao_atrous_write8.comp.spv`, `-D SSAO_ATROUS_WRITE_R8=1`): `gAoIn` pinned `r16`,
    /// `gAoOut` pinned `r8` (writes back into the frozen `gSsao` R8_UNORM image the resolve reads at
    /// binding 11). Selected for the LAST level only. Same 4-binding layout as [`SSAO_ATROUS_SPV`];
    /// only the `gAoOut` `OpTypeImage` pin differs.
    SSAO_ATROUS_WRITE8_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ssao_atrous_write8.comp.spv")
}

embed_spirv! {
    /// The Rung-3b TEMPORAL shadow-vis reproject+accumulate SPIR-V (`shaders/shadow_temporal.comp.spv`,
    /// Option B). ONE dispatch AFTER the à-trous filter, BEFORE the RESOLVE_DENOISED resolve: reprojects
    /// the current shadow-vis (`gVisIn` — the à-trous output in `Both`, the raw VIS output in `Temporal`)
    /// through the per-pixel `motion_vec` into a per-FIF `R16G16B16A16_UNORM` history ring (R=vis,
    /// G=conf/CONF_MAX, B=depth/DEPTH_NORM, A=_), variance-clamps to the current 3×3 AABB (Salvi),
    /// velocity-adaptive `k = lerp(feedback_max, feedback_min, |Δuv|·extent/VELOCITY_REF)`, and hard-
    /// resets on disocclusion (off-screen / conf==0 / prev-vs-cur depth swap, W2). Writes the history
    /// `[fi]` + `gTemporalOut` (the DENOISED reads it at `gShadowVis` @21). Bound to its OWN 8-binding
    /// layout { @0 `gVisIn` RG read, @1 `gMotionVec` RG16F read, @2 `gViewT` r32f read, @3 `gHistIn`
    /// RGBA16 read (`hist[1-fi]`), @4 `gHistOut` RGBA16 write (`hist[fi]`), @5 `gTemporalOut` RG16 write,
    /// @6 `ResolvedTemporalShadow` UBO (16 B), @7 the shared 80-byte Camera UBO }. NEW `.spv` (no base
    /// variant to freeze); dispatched only when `mode ∈ {Temporal, Both}` ⇒ the golden path is untouched.
    #[cfg(feature = "hwrt")]
    SHADOW_TEMPORAL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/shadow_temporal.comp.spv")
}

embed_spirv! {
    /// Anti-aliasing Stage 4 (TAA) — the temporal-resolve reproject+accumulate SPIR-V
    /// (`shaders/taa_resolve.comp.hlsl`, Option B). Modeled on [`SHADOW_TEMPORAL_SPV`]'s
    /// algorithm (reproject → neighborhood clamp → confidence-adaptive feedback →
    /// disocclusion reset), generalized scalar→RGB. Bound to its OWN 8-binding layout {
    /// @0 `gLit` COMBINED_IMAGE_SAMPLER (current LDR color), @1 `gViewT` r32f read (the depth
    /// proxy the camera-only MV ray marches by), @2 `gHistIn` rgba16f read (`taa_hist[1-fi]`,
    /// the framegraph's C1-fix read-sibling), @3 `gHistOut` rgba16f write (`taa_hist[fi]`), @4
    /// `gAaOut` rgba8 write (the present-blit's input), @5 the `ResolvedTaa` tunables UBO (16
    /// B), @6 the shared 80-byte Camera UBO (UNJITTERED — C1 cut), @7 the `MotionCam` UBO
    /// (`boyko_render::motion_cam`, 128 B) } + a 4-byte `{ uint reset; }` push constant
    /// (`boyko_render::taa_state::TaaState`). NOT `hwrt`-gated (TAA works on the pure-software
    /// leg — its motion vector is reconstructed from `gViewT`, never a `rayQuery` trace).
    ///
    /// **W5**: bound at boot (`boyko_app::gpu_scene::GpuSceneBundles::boot`, mirroring
    /// [`SHADOW_TEMPORAL_SPV`]'s `shadow_temporal_pipeline` boot-build pattern, unconditionally
    /// here) and dispatched by `present::passes::taa::Renderer::record_taa` when
    /// `GBufferScene::taa.is_some()` (`AaMode::Taa` armed) — see `TaaActivation`'s doc in
    /// `present::scene_types` for the full activation shape. The const-asserted length is the
    /// anti-drift guard.
    TAA_RESOLVE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/taa_resolve.comp.spv")
}

embed_spirv! {
    /// The SDFDDGI I3 DDGI resolve-sample GPU-GOLDEN SPIR-V (`shaders/ddgi_probe_gi_resolve.comp.hlsl`).
    /// A standalone compute harness that runs the SAME `ddgi_probe_sample` the deferred resolve runs
    /// (both `#include "ddgi_resolve.hlsli"`) over host-supplied receiver samples and STOREs the
    /// resolved irradiance, so the `ddgi_probe_gi_resolve` test can diff GPU-vs-`goldens::probe_sample`
    /// to bits. Its own pipeline layout (b0 grid UBO — its pad `.x` carries the sample count, t1/s1 irr,
    /// t2/s2 depth, t3 recv-pos, t4 recv-nrm, u5 out) — NOT the resolve set, no push constant.
    DDGI_PROBE_GI_RESOLVE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ddgi_probe_gi_resolve.comp.spv")
}

embed_spirv! {
    /// The committed Lighting-L1 clustered froxel light-cull SPIR-V (`shaders/cluster_cull.hlsl`).
    /// One invocation per froxel (`CLUSTER_COUNT`): builds the froxel's world-space AABB from the
    /// shared ray-gen + the exp-Z slice view-z, culls each point/spot light's bounding sphere
    /// (`sqDistPointAABB <= r²`), and atomic-appends survivors into the flat `LightIndexList` +
    /// writes the per-froxel `{offset, count}` `ClusterGrid` cell. Bound to the cull set { camera
    /// UBO @0, light table SSBO @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4 } + a
    /// `ClusterCullPush` (near/far + caps). Directional/sky are GLOBAL (not culled). The host
    /// mirror is [`golden_cluster_cull`].
    CLUSTER_CULL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/cluster_cull.comp.spv")
}

embed_spirv! {
    /// The committed Render P4b coarse-cull / tile pre-trace SPIR-V
    /// (`shaders/sdf_tile_cull.hlsl`). A 1/8-res CONSERVATIVE cone-trace: one invocation
    /// per 8×8 fine-pixel tile emits a [`TileBound`] the fine marcher reads to early-out
    /// EMPTY tiles + seed `t = near_t`. A strict FIELD-CONSUMER (calls the frozen
    /// `field_distance`); bound to the P4b vocabulary set { SSBO edit-list @0, SAMPLED
    /// depth @1, STORAGE `TileBound` @6, UNIFORM camera @5 }.
    SDF_TILE_CULL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_tile_cull.comp.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off):
    /// the `gViewT` producer replacement (`shaders/viewt_from_depth.comp.hlsl`). A full-screen,
    /// 8×8-tiled pass that reproduces the SDF marcher's own mesh-depth → `t_mesh` conversion
    /// (`sdf_gbuffer_composite.hlsl`'s `mesh_norm`/`t_mesh`/`gViewT` sentinel logic, byte-for-
    /// byte) for every pixel, so a mesh-only frame — which never dispatches the marcher — still
    /// gives the resolve/SSAO a real `gViewT` lane. Bound to its OWN dedicated 2-binding layout
    /// { SAMPLED depth @0, STORAGE `gViewT` @1 } + the 12-byte [`ViewtFromDepthPush`] (`img_w`,
    /// `img_h`, the host-precomputed `mesh_norm`). See [`ViewtFromDepthPush`]'s doc for the
    /// `mesh_norm` single-source-of-truth (`boyko_render::gbuffer_depth::mesh_view_t_norm` — a
    /// dev-only back-edge from this crate, so not a doc-linkable path here).
    VIEWT_FROM_DEPTH_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/viewt_from_depth.comp.spv")
}

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

embed_spirv! {
    /// `SSAO_PARAMS[SSAO_QUALITY_LOW]` — `sdf_ssao_low.comp.spv` (2 slices × 3 steps × 2 = 12 taps).
    SDF_SSAO_LOW_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_ssao_low.comp.spv")
}

embed_spirv! {
    /// `SSAO_PARAMS[SSAO_QUALITY_MEDIUM]` — `sdf_ssao_medium.comp.spv` (2 slices × 4 steps × 2 = 16
    /// taps; == today's shipped consts, byte-identical to the pre-Q2 base `sdf_ssao.comp.spv`).
    SDF_SSAO_MEDIUM_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_ssao_medium.comp.spv")
}

embed_spirv! {
    /// `SSAO_PARAMS[SSAO_QUALITY_HIGH]` — `sdf_ssao_high.comp.spv` (8 slices × 6 steps × 2 = 96 taps).
    SDF_SSAO_HIGH_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_ssao_high.comp.spv")
}

// The committed SDFDDGI I2 probe-update SPIR-V — ONE PRE-COMPILED `.spv` (refactor A-1). The
// former 4 baked-const variant files collapsed to a single source whose `GI_MAX_IT` sphere-trace
// `[loop]` trip count is a Vulkan SPECIALIZATION CONSTANT (`[[vk::constant_id(0)]]`, default 64):
// the trip count resolves at pipeline-create, so a spec-const on a `[loop]` is structurally
// identical to the former baked const (the loop was never unrolled either way — ZERO per-thread
// cost) while ONE `.spv` serves every sweep value. Default callers build the pipeline with
// `spec_constants: &[]` (resolves to 64, byte-identical to the old `static const 64u`); only the
// bench sweep overrides `GI_MAX_IT` via `SpecConstant { id: 0, value }`. The interface is the
// dedicated update bind-group (set 0: t0 Buf, u1 gIrrOut, u2 gDepthOut, u3 Classification, t4
// RayTable, t5 LightBuf, b6 DdgiUpdate). `N` is the `.spv`'s own const-asserted `include_bytes!`
// length — a drifted blob fails the size guard at compile time.

embed_spirv! {
    /// The committed SDFDDGI I2 probe-update SPIR-V (`sdf_probe_update.comp.spv`) — `GI_MAX_IT` is a
    /// spec-const (id 0, default 64), so this ONE blob serves every sweep value (the default resolves
    /// byte-identical to the former baked `it64` variant).
    SDF_PROBE_UPDATE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_probe_update.comp.spv")
}

embed_spirv! {
    /// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS vertex SPIR-V
    /// (`shaders/csm_depth.vs.hlsl`). A GRAPHICS (`vs_6_0`) stage — the FIRST non-compute blob
    /// hosted here, so the resolve/depth-pass shaders live behind ONE `compute::*_spirv()`
    /// vocabulary. It reads the SAME set-0 binding-0 `InstanceModelCol` SSBO + the SAME 88-byte
    /// VERTEX push as `gbuffer_mrt.vs.hlsl`'s instanced arm, but projects by the CASCADE's
    /// world→light-clip matrix (push `@0`) instead of the camera view-proj, and outputs ONLY
    /// `SV_Position` (depth-only). Paired with [`csm_depth_fs_spirv`] in a depth-only graphics
    /// pipeline (EMPTY `color_formats`, `cull_mode: Front`, a slope+constant depth bias).
    CSM_DEPTH_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/csm_depth.vs.spv")
}

embed_spirv! {
    /// The committed CSM Increment-1b Rung-A cascade DEPTH-PASS fragment SPIR-V
    /// (`shaders/csm_depth.fs.hlsl`). An EMPTY (`ps_6_0`) stage: the cascade pass is depth-only
    /// (no color attachment), so the fragment writes nothing — the rasterizer's interpolated
    /// `SV_Position.z` is the cascade depth. Paired with [`csm_depth_vs_spirv`].
    CSM_DEPTH_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/csm_depth.fs.spv")
}

embed_spirv! {
    /// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS vertex SPIR-V
    /// (`shaders/punctual_depth.vs.hlsl`). A GRAPHICS (`vs_6_0`) stage: reads the SAME set-0
    /// `InstanceModelCol` SSBO + the 88-byte VERTEX push as [`csm_depth_vs_spirv`], projects each
    /// caster instance into one cube FACE's light-clip space (push `@0`), AND forwards the WORLD
    /// position to the fragment so the matching FS can write the linear radial distance. Paired with
    /// [`punctual_depth_fs_spirv`] in a depth-WRITE (no early-Z) graphics pipeline.
    PUNCTUAL_DEPTH_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/punctual_depth.vs.spv")
}

embed_spirv! {
    /// The committed Shadow Phase 5 Increment-2 (POINT cube) punctual DEPTH-PASS fragment SPIR-V
    /// (`shaders/punctual_depth.fs.hlsl`). A `ps_6_0` stage that writes `SV_Depth =
    /// saturate(length(world - light_pos) * inv_range)` — the LINEAR radial distance from the point
    /// light (face-independent, so all six cube faces share ONE comparison scale; the resolve compares
    /// the receiver's own `length(P - light_pos) * inv_range` against it). `light_pos`/`inv_range` ride
    /// in the DEAD `cam_eye@64` push lane (the pipeline push range covers `VERTEX | FRAGMENT`). Paired
    /// with [`punctual_depth_vs_spirv`].
    PUNCTUAL_DEPTH_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/punctual_depth.fs.spv")
}

embed_spirv! {
    /// The committed mesh-MRT G-buffer PRODUCER vertex SPIR-V (`shaders/gbuffer_mrt.vs.hlsl`).
    /// Vertex layout: position (loc 0, offset 0) + world normal (loc 2, offset 12) + color
    /// (loc 1, offset 24). The shader itself declares no stride (a `VkVertexInputBindingDescription`
    /// property the HOST pipeline sets, `boyko_render::mesh::VERTEX_STRIDE` — 64 bytes since the
    /// trailing `uv`/`tangent` fields were appended; this shader reads only the first 3 attributes,
    /// so it is unaffected). Reads the set-0 `InstanceModelCol` SSBO + the
    /// 88-byte `{ view_proj; cam_eye; base_instance; use_model_matrix }` VERTEX push
    /// ([`GBUFFER_PUSH_BYTES`](crate::swapchain::GBUFFER_PUSH_BYTES)); `use_model_matrix == 0`
    /// is the legacy merged-draw arm, `== 1` the instanced arm. Exported for the host layer
    /// (host plan R3): the SAME blob the `window_present_gbuffer` harness embeds.
    GBUFFER_MRT_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt.vs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R4b: the Forward v1 mesh raster VERTEX SPIR-V
    /// (`shaders/forward_opaque.vs.hlsl`). Emits a REAL hardware reverse-Z `SV_Position.z`
    /// (`boyko_render::view::forward_view_proj_rows`, NOT the Deferred custom-linear encode);
    /// the SAME 88-byte VERTEX push shape + set-0 `InstanceModelCol` SSBO layout as
    /// [`GBUFFER_MRT_VS_SPV`] — only the matrix CONTENT + the trailing forwarded `mat_id`
    /// (instead of `PerInstanceMaterial`'s full payload) differ. See that file's header for the
    /// full v1 scope cut.
    FORWARD_OPAQUE_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_opaque.vs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R4b: the Forward v1 mesh raster FRAGMENT SPIR-V
    /// (`shaders/forward_opaque.fs.hlsl`). Shades every covered pixel inline against the full
    /// light table (all-lights, no froxel) via the SAME shared BRDF (`pbr_lighting.hlsli`) +
    /// combined CSM/punctual shadow visibility (`shadow_apply.hlsli`) the deferred resolve uses.
    /// NO `SV_Depth`/`discard`/UAV — early-Z stays live. Set 0 (camera/light/materials + the
    /// VS instance SSBOs) + Set 1 (CSM/atlas, its OWN binding numbers — boot-panic fix:
    /// renumbered from an original Set 2 design, see `rhi_impl/device.rs::build_graphics_pipeline`'s
    /// doc) — no bindless texture table this v1 rung. See that file's header for the full v1
    /// scope cut.
    FORWARD_OPAQUE_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_opaque.fs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
    /// BACKGROUND vertex SPIR-V (`shaders/forward_sky.vs.hlsl`) — a full-screen triangle, NO
    /// vertex buffer, NO descriptor bindings (`SV_VertexID`-only, the `fullscreen_sample.vs.hlsl`
    /// idiom). Paired with [`FORWARD_SKY_FS_SPV`].
    FORWARD_SKY_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_sky.vs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
    /// BACKGROUND fragment SPIR-V (`shaders/forward_sky.fs.hlsl`) — replicates the deferred
    /// resolve's `mask == 0` background branch (analytic sky/ground gradient + visible sun disc,
    /// `deferred_pbr.hlsl:1369-1414`) so a Forward frame's uncovered pixels match a Deferred
    /// frame's instead of staying flat-clear/black. Drawn FIRST inside `forward_opaque`'s SAME
    /// dynamic-rendering scope, depth test/write OFF (`depth_format: None`), so opaque mesh
    /// geometry then draws over it. Paired with [`FORWARD_SKY_VS_SPV`].
    FORWARD_SKY_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_sky.fs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS vertex
    /// SPIR-V (`shaders/depth_prepass.vs.hlsl`) — a position-only subset of
    /// [`FORWARD_OPAQUE_VS_SPV`] (same instance SSBO + push shape, no normal/mat_id export).
    /// Paired with [`DEPTH_PREPASS_FS_SPV`] in
    /// [`VulkanContext::create_graphics_pipeline_forward_prepass`].
    DEPTH_PREPASS_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/depth_prepass.vs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS fragment
    /// SPIR-V (`shaders/depth_prepass.fs.hlsl`) — an empty entry point (zero color attachments;
    /// this RHI's pipeline builder requires a fragment module unconditionally). Paired with
    /// [`DEPTH_PREPASS_VS_SPV`].
    DEPTH_PREPASS_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/depth_prepass.fs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the `forward_opaque` FROXEL
    /// fragment SPIR-V — `shaders/forward_opaque.fs.hlsl` recompiled with `-D FROXEL=1`
    /// (see that file's header). Declares `ClusterGrid`/`LightIndexList` @5/6, a subset of the
    /// UNIFIED 7-binding `forward_layout0` every Forward-family pipeline is built against
    /// (rung R5 code-review fix — ONE Set-0 layout object, never two distinct handles); Set 1
    /// (shadow) is UNCHANGED, shared verbatim with [`FORWARD_OPAQUE_FS_SPV`]. Paired with
    /// [`FORWARD_OPAQUE_VS_SPV`] (the VS is IDENTICAL — only the fragment shader's light-loop
    /// source differs by the `#ifdef FROXEL` compile flag) in
    /// [`VulkanContext::create_graphics_pipeline_forward_plus`].
    FORWARD_OPAQUE_FROXEL_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_opaque_froxel.fs.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the SDF forward-march FUSED
    /// march-then-shade compute SPIR-V, `HAS_MESH` variant (`shaders/sdf_forward_march.comp.hlsl`
    /// compiled with `-D HAS_MESH=1`). Marches the SDF field (the M1/M2/M4 brick/clip-map
    /// acceleration + the analytic A1 soft-shadow march are VERBATIM copies of
    /// `sdf_gbuffer_composite.hlsl`'s own spans, wired to real shared resources but threaded OFF
    /// this rung), then runs the full Cook-Torrance shade (a TOKEN-FOR-TOKEN clone of
    /// `forward_opaque.fs.hlsl`'s own light loop) and stores directly into the Forward `lit`
    /// STORAGE image. Samples the Forward reverse-Z `forward_depth` image to bound the march at
    /// the mesh surface (Decision 4's ownership gate — `sdf_owns = hit && t < t_mesh`). Paired
    /// with [`SDF_FORWARD_MARCH_SDFONLY_SPV`] (the mesh-less sibling compile).
    SDF_FORWARD_MARCH_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march.comp.spv")
}

embed_spirv! {
    /// Multi-paradigm render-path plan, rung R-SDFFWD: the SDF forward-march FUSED
    /// march-then-shade compute SPIR-V, mesh-less variant (`shaders/sdf_forward_march.comp.hlsl`
    /// compiled with no `-D`). Used under `GeometryLegs::Sdf` (no raster mesh leg): never samples
    /// `forward_depth` (the ownership gate collapses to `sdf_owns = hit` — every hit is owned),
    /// so its Set-0 layout still reserves the depth-image slot (bound-but-unread, the R2
    /// contract) but its SPIR-V never references it. Paired with [`SDF_FORWARD_MARCH_SPV`].
    SDF_FORWARD_MARCH_SDFONLY_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march_sdfonly.comp.spv")
}

embed_spirv! {
    /// The committed mesh-MRT G-buffer PRODUCER fragment SPIR-V (`shaders/gbuffer_mrt.fs.hlsl`):
    /// writes albedo/normal/material as 3 MRT in the marcher's exact encoding (mask=1) + the
    /// marcher-aligned `SV_Depth` (euclidean under perspective, axial under ortho). Paired with
    /// [`gbuffer_mrt_vs_spirv`].
    GBUFFER_MRT_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt.fs.spv")
}

embed_spirv! {
    /// The Rung-3b MOTION_VECTORS-variant mesh-MRT G-buffer PRODUCER vertex SPIR-V
    /// (`shaders/gbuffer_mrt_mv.vs.spv`, compiled from `gbuffer_mrt.vs.hlsl` with
    /// `-D MOTION_VECTORS=1`). Identical to [`GBUFFER_MRT_VS_SPV`] except it additionally reads a
    /// second per-instance model ring `prev_instances` (set-0 binding 1, LAST frame's transforms)
    /// and a `MotionCam` UBO (set-0 binding 2, cur+prev marcher-aligned view-proj), and forwards
    /// the current + previous CLIP positions to the fragment. Bound into the 4-attachment MV
    /// raster pipeline (3× `R8G8B8A8_UNORM` + `motion_vec` `R16G16_SFLOAT`) with the 3-binding
    /// instance-MV layout; selected only when the shadow denoiser's temporal mode is active. The
    /// base [`GBUFFER_MRT_VS_SPV`] stays the byte-frozen 3-MRT golden (the step-5 gate).
    #[cfg(feature = "hwrt")]
    GBUFFER_MRT_MV_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_mv.vs.spv")
}

embed_spirv! {
    /// The Rung-3b MOTION_VECTORS-variant mesh-MRT G-buffer PRODUCER fragment SPIR-V
    /// (`shaders/gbuffer_mrt_mv.fs.spv`, compiled from `gbuffer_mrt.fs.hlsl` with
    /// `-D MOTION_VECTORS=1`). Writes the SAME 3 attribute MRTs + `SV_Depth` as
    /// [`GBUFFER_MRT_FS_SPV`], plus a 4th MRT `SV_Target3 motion_vec` = `clip_to_uv(prev_clip) -
    /// clip_to_uv(cur_clip)` (a static pixel writes exactly `(0,0)`). Paired with
    /// [`gbuffer_mrt_mv_vs_spirv`]; the base [`GBUFFER_MRT_FS_SPV`] stays the byte-frozen golden.
    #[cfg(feature = "hwrt")]
    GBUFFER_MRT_MV_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_mv.fs.spv")
}

embed_spirv! {
    /// Asset-streaming plan F8+ PER_INSTANCE_MATERIAL-variant mesh-MRT G-buffer PRODUCER
    /// vertex SPIR-V (`shaders/gbuffer_mrt_pm.vs.spv`, compiled from `gbuffer_mrt.vs.hlsl`
    /// with `-D PER_INSTANCE_MATERIAL=1`). Identical to [`GBUFFER_MRT_VS_SPV`] except it
    /// additionally reads a per-instance material PAYLOAD SSBO (set-0 binding 1, VERTEX —
    /// id + `base_color`) at the SAME `pc.base_instance + SV_InstanceID` index the
    /// model-matrix arm already uses, and forwards both flat (`nointerpolation`) to the
    /// fragment. Materials are device-agnostic (unlike `mv`, this is NOT
    /// `#[cfg(feature = "hwrt")]`) — built at boot on every device and bound instead of the
    /// base pipeline ONLY on a frame with a non-default material (and no temporal denoise —
    /// MV takes priority, asset-streaming plan F8 §2.3). The base [`GBUFFER_MRT_VS_SPV`]
    /// stays the byte-frozen 3-MRT golden (never recompiled by F8/F8+).
    GBUFFER_MRT_PM_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_pm.vs.spv")
}

embed_spirv! {
    /// Asset-streaming plan F8+ PER_INSTANCE_MATERIAL-variant mesh-MRT G-buffer PRODUCER
    /// fragment SPIR-V (`shaders/gbuffer_mrt_pm.fs.spv`, compiled from `gbuffer_mrt.fs.hlsl`
    /// with `-D PER_INSTANCE_MATERIAL=1`). Writes the SAME 3 attribute MRTs + `SV_Depth` as
    /// [`GBUFFER_MRT_FS_SPV`], except `gNormal.BA` packs the REAL per-instance material id
    /// (forwarded flat from the VS, unchanged from F8) AND `gAlbedo` sources the
    /// per-instance material's `base_color` (owner: material-drives-albedo-too) instead of
    /// the mesh vertex color. Paired with [`gbuffer_mrt_pm_vs_spirv`]; the base
    /// [`GBUFFER_MRT_FS_SPV`] stays the byte-frozen golden (never recompiled by F8/F8+).
    GBUFFER_MRT_PM_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_pm.fs.spv")
}

embed_spirv! {
    /// F8-mv: the combined MOTION_VECTORS + PER_INSTANCE_MATERIAL mesh-MRT G-buffer
    /// PRODUCER vertex SPIR-V (`shaders/gbuffer_mrt_mvpm.vs.spv`, compiled from
    /// `gbuffer_mrt.vs.hlsl` with BOTH `-D MOTION_VECTORS=1 -D PER_INSTANCE_MATERIAL=1`).
    /// Identical to [`GBUFFER_MRT_MV_VS_SPV`] except it ALSO reads a per-instance material
    /// PAYLOAD SSBO — moved to set-0 binding 3 (the nested `#if defined(MOTION_VECTORS)`
    /// branch resolves the binding-1 collision with `prev_instances`) — and forwards the id
    /// + `base_color` flat to the fragment, like [`GBUFFER_MRT_PM_VS_SPV`]. Bound into a
    /// 4-attachment pipeline with a 4-binding set-0 layout (instances @0, prev_instances @1,
    /// `MotionCam` @2, instance_materials @3, all VERTEX); selected only when temporal denoise
    /// AND a non-default material are both active this frame (MV+PM combined, F8-mv). The base
    /// [`GBUFFER_MRT_VS_SPV`]/[`GBUFFER_MRT_MV_VS_SPV`]/[`GBUFFER_MRT_PM_VS_SPV`] stay
    /// byte-frozen (the step-2 byte-identity gate).
    #[cfg(feature = "hwrt")]
    GBUFFER_MRT_MVPM_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_mvpm.vs.spv")
}

embed_spirv! {
    /// F8-mv: the combined MOTION_VECTORS + PER_INSTANCE_MATERIAL mesh-MRT G-buffer
    /// PRODUCER fragment SPIR-V (`shaders/gbuffer_mrt_mvpm.fs.spv`, compiled from
    /// `gbuffer_mrt.fs.hlsl` with BOTH `-D MOTION_VECTORS=1 -D PER_INSTANCE_MATERIAL=1`).
    /// Writes the SAME 3 attribute MRTs + `SV_Depth` as [`GBUFFER_MRT_FS_SPV`], PLUS the 4th
    /// MRT `motion_vec` Δuv (like [`GBUFFER_MRT_MV_FS_SPV`]) AND sources `gAlbedo`/`gNormal.BA`
    /// from the forwarded per-instance material (like [`GBUFFER_MRT_PM_FS_SPV`]). Paired with
    /// [`gbuffer_mrt_mvpm_vs_spirv`]; the `gbuffer_mrt.fs.hlsl` source is UNTOUCHED by F8-mv —
    /// only the `-D` combination is new.
    #[cfg(feature = "hwrt")]
    GBUFFER_MRT_MVPM_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_mvpm.fs.spv")
}

embed_spirv! {
    /// Textured-PBR rung T6c TEXTURED-variant mesh-MRT G-buffer PRODUCER vertex SPIR-V
    /// (`shaders/gbuffer_mrt_tex.vs.spv`, compiled from `gbuffer_mrt.vs.hlsl` with
    /// `-D TEXTURED=1`). An INDEPENDENT #ifdef axis from PER_INSTANCE_MATERIAL/
    /// MOTION_VECTORS (never compiled together with either — T6c plan Decision D4). Reads a
    /// per-instance TEXTURED material PAYLOAD SSBO (set-0 binding 1, VERTEX —
    /// `PerInstanceMaterialTex`) at the SAME `pc.base_instance + SV_InstanceID` index the
    /// model-matrix arm already uses, PLUS the vertex `uv`/`tangent` attributes (declared
    /// 4th/5th, DXC-assigned SPIR-V locations 3/4), building the tangent-space basis
    /// `world_T = normalize(mul(m3, tangent.xyz))` (the PLAIN model 3x3, glTF/Mikktspace
    /// convention). Bound into the 2-set TEXTURED raster pipeline (set 0 = the
    /// `PerInstanceMaterialTex` layout, VERTEX; set 1 = the bindless texture-array set,
    /// FRAGMENT) with the widened 64-byte `MESH_VERTEX_STRIDE` vertex layout (position@0 /
    /// normal@12 / color@24 / uv@40 / tangent@48). The base [`GBUFFER_MRT_VS_SPV`] stays the
    /// byte-frozen 3-MRT golden (never recompiled by T6c — the ENTIRE new axis is
    /// `#ifdef TEXTURED`-gated).
    GBUFFER_MRT_TEX_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_tex.vs.spv")
}

embed_spirv! {
    /// Textured-PBR rung T6c TEXTURED-variant mesh-MRT G-buffer PRODUCER fragment SPIR-V
    /// (`shaders/gbuffer_mrt_tex.fs.spv`, compiled from `gbuffer_mrt.fs.hlsl` with
    /// `-D TEXTURED=1`). Samples the bindless texture array (set 1, `NonUniformResourceIndex`-
    /// gated — see the `textured_nonuniform_spirv` hermetic proof) for gAlbedo (modulated by
    /// `base_color`), performs tangent-space normal mapping into gNormal (the TBN basis is
    /// glTF/Mikktspace convention; the sampled green channel is separately negated per this
    /// engine's own convention for OpenGL-style input maps — see `gbuffer_mrt.fs.hlsl`'s
    /// GREEN-CHANNEL CONVENTION block; geometric normal when unbound), and writes a 4th MRT
    /// `SV_Target3 pbr` (`R16G16B16A16_SFLOAT`) carrying `[metallic, roughness, AO-modulation,
    /// emissive-luminance-modulation]` (glTF metal-rough channel convention: metallic = B,
    /// roughness = G) — read by the deferred SOFTWARE resolve's flag-gated `gPbr.Load`
    /// (T6a). Paired with [`gbuffer_mrt_tex_vs_spirv`]; the base [`GBUFFER_MRT_FS_SPV`] stays
    /// byte-frozen (never recompiled by T6c).
    GBUFFER_MRT_TEX_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/gbuffer_mrt_tex.fs.spv")
}

embed_spirv! {
    /// The committed fullscreen-sample vertex SPIR-V (`shaders/fullscreen_sample.vs.hlsl`): a
    /// fullscreen triangle generating positions + UVs from `SV_VertexID` (no vertex buffer).
    /// The present-blit pass's VS.
    FULLSCREEN_SAMPLE_VS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fullscreen_sample.vs.spv")
}

embed_spirv! {
    /// The committed fullscreen-sample fragment SPIR-V (`shaders/fullscreen_sample.fs.hlsl`):
    /// samples the bound `Texture2D` + `SamplerState` at the interpolated UV and outputs it.
    /// The present-blit pass's FS; paired with [`fullscreen_sample_vs_spirv`].
    FULLSCREEN_SAMPLE_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fullscreen_sample.fs.spv")
}

embed_spirv! {
    /// FXAA 3.11 compact fragment SPIR-V (`shaders/fxaa.fs.hlsl`, three-source-validated
    /// against Lottes FXAA3_11 / Rodriguez compact form / Bevy's shipped `fxaa.wgsl`): a
    /// 12-tap edge-only luma post-process. Paired with [`fullscreen_sample_vs_spirv`] (the
    /// FXAA pipeline reuses the same fullscreen-triangle VS); reads `lit` (LINEAR sampler),
    /// writes `aa_out`. Stage-1 anti-aliasing pass — armed only when `scene.aa` is `Some`.
    FXAA_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fxaa.fs.spv")
}

embed_spirv! {
    /// AA campaign Stage 3 — SSAA 2× downsample fragment SPIR-V
    /// (`shaders/ssaa_downsample.fs.hlsl`): a linear-light 2×2 box filter that resolves the
    /// 2× LIT ring into the native-size `aa_out`. Paired with [`fullscreen_sample_vs_spirv`]
    /// (reuses the same fullscreen-triangle VS); reads `lit` via `.Load` (the bound sampler
    /// is irrelevant), writes `aa_out`. Armed only when `scene.ssaa` is `Some` — boot-fixed,
    /// host-authoritative (see `boyko_app::host::WindowHost`).
    SSAA_DOWNSAMPLE_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/ssaa_downsample.fs.spv")
}

embed_spirv! {
    /// AA campaign Stage 2 — SMAA 1x pass 1 (edge detection) fragment SPIR-V
    /// (`shaders/smaa_edge.fs.hlsl`, ported verbatim from iryoku `SMAALumaEdgeDetectionPS`).
    /// Paired with [`fullscreen_sample_vs_spirv`] (all three SMAA passes share the same
    /// fullscreen-triangle VS); reads `lit`, writes `edges` (R8G8). Armed only when
    /// `scene.smaa` is `Some`.
    SMAA_EDGE_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/smaa_edge.fs.spv")
}

embed_spirv! {
    /// AA campaign Stage 2 — SMAA 1x pass 2 (blending-weight calculation) fragment SPIR-V
    /// (`shaders/smaa_weight.fs.hlsl`, ported verbatim from iryoku
    /// `SMAABlendingWeightCalculationPS`, PRESET_HIGH diagonal + corner detection). Reads
    /// `edges` + the boot-resident `areaTex`/`searchTex` LUTs, writes `weights` (RGBA8).
    /// Armed only when `scene.smaa` is `Some`.
    SMAA_WEIGHT_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/smaa_weight.fs.spv")
}

embed_spirv! {
    /// AA campaign Stage 2 — SMAA 1x pass 3 (neighborhood blending) fragment SPIR-V
    /// (`shaders/smaa_blend.fs.hlsl`, ported verbatim from iryoku
    /// `SMAANeighborhoodBlendingPS`). Reads `lit` + pass 2's `weights`, writes `aa_out` (the
    /// same target FXAA's single pass writes). Armed only when `scene.smaa` is `Some`.
    SMAA_BLEND_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/smaa_blend.fs.spv")
}

embed_spirv! {
    /// Pillar B increment B2: the per-instance TRS interpolation compute PRE-PASS
    /// (`interp_instances.comp`, refined-B). One invocation per DYNAMIC instance reads a
    /// 96-byte `TransformPair` at binding 0 + its output slot at binding 1, interpolates at the
    /// frame-wide `alpha`, and scatters the 48-byte `InstanceModelCol`-shaped model row into the
    /// SHARED instance ring at binding 2 (`ModelOut[OutSlot[i]]`). The size pins the committed
    /// `.spv`; the `interp_edsl_sync` test proves the byte stream is the single-sourced eDSL emit.
    INTERP_INSTANCES_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/interp_instances.comp.spv")
}

embed_spirv! {
    /// HW-RT rung R2a-3: the per-frame TLAS-instance PACKER compute pre-pass
    /// (`build_tlas_instances.comp`). One invocation per drawable instance reads the shared M3
    /// instance ring (t0) + the mesh-id lane (t1) + the per-mesh BLAS-address table (t2), and
    /// stream-writes the 64-byte `VkAccelerationStructureInstanceKHR` record into the device-local
    /// output array (u3) the per-frame TLAS build reads. The size pins the committed `.spv`.
    #[cfg(feature = "hwrt")]
    BUILD_TLAS_INSTANCES_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/build_tlas_instances.comp.spv")
}

embed_spirv! {
    /// HW-RT rung R2a-4a: the AS-descriptor GPU-smoke shader (`hwrt_as_descriptor_smoke.comp`). A
    /// minimal `rayQuery` compute that binds a TLAS at t0 (the new
    /// `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` descriptor), traces one inline ray against it,
    /// and writes the hit flag to a single-`uint` output (u1). It exists ONLY as the oracle for the
    /// AS-descriptor `p_next` write — the smoke passes iff the dispatch is device-lost-free with clean
    /// validation. Compiled `dxc -T cs_6_5 -fspv-target-env=vulkan1.3` (emits `OpCapability
    /// RayQueryKHR`). The size pins the committed `.spv`.
    #[cfg(feature = "hwrt")]
    HWRT_AS_DESCRIPTOR_SMOKE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/hwrt_as_descriptor_smoke.comp.spv")
}

embed_spirv! {
    /// The committed SPIR-V for the Rung 1a specialization-constant GPU smoke
    /// (`shaders/spec_constant_smoke.comp.hlsl`): a single-thread compute that writes
    /// its `[[vk::constant_id(0)]]` value into `buffer[0]`. Gated behind
    /// `spec_constant_smoke` (OFF by default) so the shipped/golden build never
    /// references the orchestrator-compiled smoke `.spv`.
    ///
    /// The byte length is derived from the committed `spec_constant_smoke.comp.spv`
    /// automatically via `SpirvBlob<{ include_bytes!(..).len() }>`, so there is no
    /// hand-counted size to keep in sync; recompiling the shader to a different size is
    /// picked up automatically on the next build with no manual update.
    #[cfg(feature = "spec_constant_smoke")]
    SPEC_CONSTANT_SMOKE_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/spec_constant_smoke.comp.spv")
}

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

/// The committed Rung 1a spec-constant smoke SPIR-V as a `u32` word stream (writes
/// its `constant_id(0)` value into `buffer[0]`). Gated behind `spec_constant_smoke`.
#[cfg(feature = "spec_constant_smoke")]
#[inline]
pub fn spec_constant_smoke_spirv() -> &'static [u32] {
    SPEC_CONSTANT_SMOKE_SPV.as_words()
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

/// The Render terminator-softening variant SPIR-V (`-D TERMINATOR_WRAP=1`) as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Binds into the SAME 20-binding software resolve layout as [`deferred_pbr_spirv`] (the
/// variant changes only the diffuse accumulation math, no descriptor). Selected instead of
/// [`deferred_pbr_spirv`] only when `LightingConfig::terminator_softening > 0`; NOT
/// `#[cfg(feature = "hwrt")]`-gated (a software-resolve-only variant, see [`DEFERRED_PBR_WRAP_SPV`]).
#[inline]
pub fn deferred_pbr_wrap_spirv() -> &'static [u32] {
    DEFERRED_PBR_WRAP_SPV.as_words()
}

/// The R2a-4b HWRT-variant deferred-resolve SPIR-V (`shaders/deferred_pbr_hwrt.comp.spv`) as a `u32`
/// word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// This module's mesh-shadow term traces the binding-19 TLAS with an inline `rayQuery` (the `#if
/// HWRT` arm of `deferred_pbr.hlsl`) instead of sampling the CSM shadow map, so its resolve pipeline
/// layout carries a 20th [`DescriptorKind::AccelerationStructure`](boyko_rhi::DescriptorKind) binding.
/// It is bound ONLY when `feature = "hwrt"` + `ctx.ray_query_enabled()` +
/// `RayBackendConfig.table[Shadow][Mesh] == HardwareTri` all hold; every other path keeps the
/// software [`deferred_pbr_spirv`] and its 19-binding layout, byte-identical to the golden.
#[cfg(feature = "hwrt")]
#[inline]
pub fn deferred_pbr_hwrt_spirv() -> &'static [u32] {
    DEFERRED_PBR_HWRT_SPV.as_words()
}

/// The Rung-3a VIS-variant deferred-resolve SPIR-V (`SHADOW_STAGE=1`) as a `u32` word stream, ready
/// for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// The à-trous pre-pass: runs the inline Vogel `rayQuery` trace exactly as the RESOLVE_INLINE-hwrt
/// resolve does (bit-identical `mesh_vis`, same `SHADOW_RAY_COUNT` spec-const) but writes
/// `gShadowVis[px,py] = RG(mesh_vis, validity)` to descriptor @21 and returns. Bound to the
/// 22-binding VIS/DENOISED layout; dispatched only when `scene.shadow.is_some()`. See
/// [`DEFERRED_PBR_VIS_SPV`]; the const-asserted length is the anti-drift guard.
#[cfg(feature = "hwrt")]
#[inline]
pub fn deferred_pbr_vis_spirv() -> &'static [u32] {
    DEFERRED_PBR_VIS_SPV.as_words()
}

/// The Rung-3b step-5b MOTION_VECTORS VIS-variant deferred-resolve SPIR-V
/// (`SHADOW_STAGE=1 + MOTION_VECTORS`) as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Identical to [`deferred_pbr_vis_spirv`] (writes `gShadowVis` @21) plus a per-SDF-pixel
/// camera-only motion vector `Δuv` written to a `motion_vec` STORAGE image @23, reprojecting the
/// reconstructed surface `P` through a `MotionCam` UBO @22. Bound to the 24-binding VIS-MV layout;
/// selected instead of [`deferred_pbr_vis_spirv`] only when the temporal shadow denoiser is active.
/// See [`DEFERRED_PBR_VIS_MV_SPV`]; the const-asserted length is the anti-drift guard.
#[cfg(feature = "hwrt")]
#[inline]
pub fn deferred_pbr_vis_mv_spirv() -> &'static [u32] {
    DEFERRED_PBR_VIS_MV_SPV.as_words()
}

/// The Rung-3a DENOISED-variant deferred-resolve SPIR-V (`SHADOW_STAGE=2`) as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Identical to the RESOLVE_INLINE-hwrt resolve except the inline trace is replaced by one
/// `gShadowVis.Load(px,py).r` read of the FILTERED vis at descriptor @21, then the identical
/// `min`-combine + full lighting. Declares no `SHADOW_RAY_COUNT` spec-const (it never traces).
/// Bound to the SAME 22-binding VIS/DENOISED layout; selected as the resolve pipeline only when
/// `scene.shadow.is_some()`. See [`DEFERRED_PBR_DENOISED_SPV`].
#[cfg(feature = "hwrt")]
#[inline]
pub fn deferred_pbr_denoised_spirv() -> &'static [u32] {
    DEFERRED_PBR_DENOISED_SPV.as_words()
}

/// The Rung-3a à-trous spatial shadow-denoise filter SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// A 25-tap/level edge-stopping wavelet (Dammertz 2010) ping-ponged over `shadow_vis`/`shadow_vis2`,
/// one dispatch per level with `step = 1 << level` pushed as a 4-byte `{ uint step; }` push-const.
/// Bound to its own 6-binding layout (see [`SHADOW_ATROUS_SPV`]). The const-asserted length is the
/// anti-drift guard.
#[cfg(feature = "hwrt")]
#[inline]
pub fn shadow_atrous_spirv() -> &'static [u32] {
    SHADOW_ATROUS_SPV.as_words()
}

/// The SSAO à-trous denoise filter, INTERIOR pin (`r16`/`r16`), as a `u32` word stream. Bound to
/// the shared 4-binding à-trous layout (see [`SSAO_ATROUS_SPV`]). Software — built unconditionally
/// (NOT `hwrt`-gated).
#[inline]
pub fn ssao_atrous_spirv() -> &'static [u32] {
    SSAO_ATROUS_SPV.as_words()
}

/// The SSAO à-trous denoise filter, READ-R8 pin (`r8`/`r16`), as a `u32` word stream — LEVEL 0
/// only (reads the raw `sdf_ssao` gather output). See [`SSAO_ATROUS_READ8_SPV`].
#[inline]
pub fn ssao_atrous_read8_spirv() -> &'static [u32] {
    SSAO_ATROUS_READ8_SPV.as_words()
}

/// The SSAO à-trous denoise filter, WRITE-R8 pin (`r16`/`r8`), as a `u32` word stream — the LAST
/// level only (writes back into the frozen `gSsao` the resolve reads). See
/// [`SSAO_ATROUS_WRITE8_SPV`].
#[inline]
pub fn ssao_atrous_write8_spirv() -> &'static [u32] {
    SSAO_ATROUS_WRITE8_SPV.as_words()
}

/// The Rung-3b TEMPORAL shadow-vis reproject+accumulate SPIR-V as a `u32` word stream. Bound to its
/// own 8-binding layout (see [`SHADOW_TEMPORAL_SPV`]): one dispatch after the à-trous filter, before
/// the RESOLVE_DENOISED resolve; reprojects `gVisIn` through `gMotionVec` into the RGBA16 history ring
/// and writes `gTemporalOut` (the DENOISED reads it at @21). The const-asserted length is the
/// anti-drift guard.
#[cfg(feature = "hwrt")]
#[inline]
pub fn shadow_temporal_spirv() -> &'static [u32] {
    SHADOW_TEMPORAL_SPV.as_words()
}

/// Anti-aliasing Stage 4 (TAA) — the temporal-resolve SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module). Bound to its
/// own 8-binding layout (see [`TAA_RESOLVE_SPV`]'s doc for the full binding contract). NOT
/// `hwrt`-gated. Bound at boot (W5) by `boyko_app::gpu_scene::GpuSceneBundles::boot` into
/// `taa_resolve_pipeline`, dispatched by `crate::present::passes::taa::Renderer::record_taa`
/// when `GBufferScene::taa.is_some()`.
#[inline]
pub fn taa_resolve_spirv() -> &'static [u32] {
    TAA_RESOLVE_SPV.as_words()
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

/// Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` gViewT-producer SPIR-V as
/// a `u32` word stream. See [`VIEWT_FROM_DEPTH_SPV`]'s doc.
#[inline]
pub fn viewt_from_depth_spirv() -> &'static [u32] {
    VIEWT_FROM_DEPTH_SPV.as_words()
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

/// Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh raster VERTEX SPIR-V as a
/// `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module). Paired with
/// [`forward_opaque_fs_spirv`] in [`VulkanContext::create_graphics_pipeline_forward`].
#[inline]
pub fn forward_opaque_vs_spirv() -> &'static [u32] {
    FORWARD_OPAQUE_VS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh raster FRAGMENT SPIR-V as a
/// `u32` word stream. Paired with [`forward_opaque_vs_spirv`].
#[inline]
pub fn forward_opaque_fs_spirv() -> &'static [u32] {
    FORWARD_OPAQUE_FS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
/// background VERTEX SPIR-V as a `u32` word stream. Paired with [`forward_sky_fs_spirv`].
#[inline]
pub fn forward_sky_vs_spirv() -> &'static [u32] {
    FORWARD_SKY_VS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
/// background FRAGMENT SPIR-V as a `u32` word stream. Paired with [`forward_sky_vs_spirv`].
#[inline]
pub fn forward_sky_fs_spirv() -> &'static [u32] {
    FORWARD_SKY_FS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS VERTEX
/// SPIR-V as a `u32` word stream. Paired with [`depth_prepass_fs_spirv`] in
/// [`VulkanContext::create_graphics_pipeline_forward_prepass`].
#[inline]
pub fn depth_prepass_vs_spirv() -> &'static [u32] {
    DEPTH_PREPASS_VS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS FRAGMENT
/// SPIR-V (an empty entry point) as a `u32` word stream. Paired with [`depth_prepass_vs_spirv`].
#[inline]
pub fn depth_prepass_fs_spirv() -> &'static [u32] {
    DEPTH_PREPASS_FS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the `forward_opaque` FROXEL
/// FRAGMENT SPIR-V as a `u32` word stream. Paired with [`forward_opaque_vs_spirv`] (the SAME
/// vertex shader — only the fragment shines through a different `#ifdef FROXEL` compile) in
/// [`VulkanContext::create_graphics_pipeline_forward_plus`].
#[inline]
pub fn forward_opaque_froxel_fs_spirv() -> &'static [u32] {
    FORWARD_OPAQUE_FROXEL_FS_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R-SDFFWD: the SDF forward-march `HAS_MESH` compute
/// SPIR-V as a `u32` word stream. Paired with [`sdf_forward_march_sdfonly_spirv`].
#[inline]
pub fn sdf_forward_march_spirv() -> &'static [u32] {
    SDF_FORWARD_MARCH_SPV.as_words()
}

/// Multi-paradigm render-path plan, rung R-SDFFWD: the SDF forward-march mesh-less compute
/// SPIR-V as a `u32` word stream. Paired with [`sdf_forward_march_spirv`].
#[inline]
pub fn sdf_forward_march_sdfonly_spirv() -> &'static [u32] {
    SDF_FORWARD_MARCH_SDFONLY_SPV.as_words()
}

/// The Rung-3b MOTION_VECTORS-variant mesh-MRT gbuffer VERTEX SPIR-V as a `u32` word stream.
/// Bound into the 4-attachment MV raster pipeline (3× `R8G8B8A8_UNORM` + `motion_vec`
/// `R16G16_SFLOAT`, `D32Sfloat` depth) with the 3-binding instance-MV set (instances @0,
/// prev_instances @1, `MotionCam` UBO @2) + the 88-byte VERTEX push. Paired with
/// [`gbuffer_mrt_mv_fs_spirv`]; selected only when the temporal shadow denoiser is active.
#[cfg(feature = "hwrt")]
#[inline]
pub fn gbuffer_mrt_mv_vs_spirv() -> &'static [u32] {
    GBUFFER_MRT_MV_VS_SPV.as_words()
}

/// The Rung-3b MOTION_VECTORS-variant mesh-MRT gbuffer FRAGMENT SPIR-V as a `u32` word stream.
/// Paired with [`gbuffer_mrt_mv_vs_spirv`]; writes the 4th MRT `motion_vec` (`Δuv`).
#[cfg(feature = "hwrt")]
#[inline]
pub fn gbuffer_mrt_mv_fs_spirv() -> &'static [u32] {
    GBUFFER_MRT_MV_FS_SPV.as_words()
}

/// Asset-streaming plan F8 PER_INSTANCE_MATERIAL-variant mesh-MRT gbuffer VERTEX SPIR-V as a
/// `u32` word stream. Bound into the PM raster pipeline (the base 3-attachment layout) with the
/// 2-binding instance-material set (instances @0, `instance_materials` @1) + the 88-byte VERTEX
/// push. Paired with [`gbuffer_mrt_pm_fs_spirv`]; selected only on a frame with a non-default
/// material. NOT `#[cfg(feature = "hwrt")]` — materials are device-agnostic.
#[inline]
pub fn gbuffer_mrt_pm_vs_spirv() -> &'static [u32] {
    GBUFFER_MRT_PM_VS_SPV.as_words()
}

/// Asset-streaming plan F8 PER_INSTANCE_MATERIAL-variant mesh-MRT gbuffer FRAGMENT SPIR-V as a
/// `u32` word stream. Paired with [`gbuffer_mrt_pm_vs_spirv`]; packs the real per-instance
/// material id into `gNormal.BA`.
#[inline]
pub fn gbuffer_mrt_pm_fs_spirv() -> &'static [u32] {
    GBUFFER_MRT_PM_FS_SPV.as_words()
}

/// F8-mv combined MOTION_VECTORS + PER_INSTANCE_MATERIAL mesh-MRT gbuffer VERTEX SPIR-V as a
/// `u32` word stream. Bound into the 4-attachment mvpm raster pipeline with the 4-binding
/// set-0 layout (instances @0, prev_instances @1, `MotionCam` @2, instance_materials @3, all
/// VERTEX) + the 88-byte VERTEX push. Paired with [`gbuffer_mrt_mvpm_fs_spirv`]; selected only
/// when temporal denoise AND a non-default material are both active this frame.
#[cfg(feature = "hwrt")]
#[inline]
pub fn gbuffer_mrt_mvpm_vs_spirv() -> &'static [u32] {
    GBUFFER_MRT_MVPM_VS_SPV.as_words()
}

/// F8-mv combined MOTION_VECTORS + PER_INSTANCE_MATERIAL mesh-MRT gbuffer FRAGMENT SPIR-V as a
/// `u32` word stream. Paired with [`gbuffer_mrt_mvpm_vs_spirv`]; writes the 4th MRT
/// `motion_vec` (`Δuv`) AND sources `gAlbedo`/`gNormal.BA` from the per-instance material.
#[cfg(feature = "hwrt")]
#[inline]
pub fn gbuffer_mrt_mvpm_fs_spirv() -> &'static [u32] {
    GBUFFER_MRT_MVPM_FS_SPV.as_words()
}

/// Textured-PBR rung T6c TEXTURED-variant mesh-MRT gbuffer VERTEX SPIR-V as a `u32` word
/// stream. Bound into the 2-set TEXTURED raster pipeline (set 0 = the `PerInstanceMaterialTex`
/// layout, VERTEX; set 1 = the bindless texture-array set, FRAGMENT) with the widened 64-byte
/// `MESH_VERTEX_STRIDE` vertex layout. Paired with [`gbuffer_mrt_tex_fs_spirv`]; selected only
/// on a frame with at least one textured material AND no active temporal denoise (TEXTURED is
/// never compiled with MOTION_VECTORS — T6c plan Decision D4). NOT `#[cfg(feature = "hwrt")]`
/// — materials/textures are device-agnostic.
#[inline]
pub fn gbuffer_mrt_tex_vs_spirv() -> &'static [u32] {
    GBUFFER_MRT_TEX_VS_SPV.as_words()
}

/// Textured-PBR rung T6c TEXTURED-variant mesh-MRT gbuffer FRAGMENT SPIR-V as a `u32` word
/// stream. Paired with [`gbuffer_mrt_tex_vs_spirv`]; samples the bindless texture array for
/// gAlbedo/gNormal (tangent-space normal mapping) and writes the 4th MRT `pbr` (metallic/
/// roughness/AO/emissive) the deferred software resolve reads under the TEXTURED flag.
#[inline]
pub fn gbuffer_mrt_tex_fs_spirv() -> &'static [u32] {
    GBUFFER_MRT_TEX_FS_SPV.as_words()
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

/// The committed FXAA 3.11 compact fragment SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Paired with [`fullscreen_sample_vs_spirv`] in the FXAA post-process pipeline
/// (`color_formats[0]` == `aa_out`'s format, NOT the swapchain format — see
/// [`AaActivation`](crate::present::AaActivation)).
#[inline]
pub fn fxaa_fs_spirv() -> &'static [u32] {
    FXAA_FS_SPV.as_words()
}

/// The committed SSAA 2× downsample fragment SPIR-V as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// Paired with [`fullscreen_sample_vs_spirv`] in the SSAA downsample pipeline
/// (`color_formats[0]` == `aa_out`'s format; the SAME `present_layout` FXAA/present reuse —
/// see [`SsaaActivation`](crate::present::SsaaActivation)).
#[inline]
pub fn ssaa_downsample_fs_spirv() -> &'static [u32] {
    SSAA_DOWNSAMPLE_FS_SPV.as_words()
}

/// The committed SMAA 1x pass-1 (edge detection) fragment SPIR-V as a `u32` word stream,
/// ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
/// Paired with [`fullscreen_sample_vs_spirv`] in the SMAA edge pipeline.
#[inline]
pub fn smaa_edge_fs_spirv() -> &'static [u32] {
    SMAA_EDGE_FS_SPV.as_words()
}

/// The committed SMAA 1x pass-2 (blending-weight calculation) fragment SPIR-V as a `u32`
/// word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module). Paired
/// with [`fullscreen_sample_vs_spirv`] in the SMAA weight pipeline.
#[inline]
pub fn smaa_weight_fs_spirv() -> &'static [u32] {
    SMAA_WEIGHT_FS_SPV.as_words()
}

/// The committed SMAA 1x pass-3 (neighborhood blending) fragment SPIR-V as a `u32` word
/// stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module). Paired
/// with [`fullscreen_sample_vs_spirv`] in the SMAA blend pipeline.
#[inline]
pub fn smaa_blend_fs_spirv() -> &'static [u32] {
    SMAA_BLEND_FS_SPV.as_words()
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

/// The committed SDFDDGI I2 probe-update SPIR-V (`sdf_probe_update.comp.spv`), as a `u32` word
/// stream ready for [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// ONE blob (refactor A-1): `GI_MAX_IT` (the sphere-trace `[loop]` trip count) is a Vulkan
/// SPECIALIZATION CONSTANT (`[[vk::constant_id(0)]]`, default 64), resolved at pipeline-create. A
/// spec-const on a `[loop]` bound is structurally identical to the former baked const (the loop was
/// never unrolled either way — ZERO per-thread cost). Build the pipeline with `spec_constants: &[]`
/// for the shipped default (resolves to 64, byte-identical to the old baked `it64` variant); the
/// bench (`tests/ddgi_probe_gi_cost.rs`) overrides `GI_MAX_IT` per sweep value (plan §5) via
/// `SpecConstant { id: 0, value }` on the SAME module. The interface is the dedicated update
/// bind-group (set 0: t0 `Buf`, u1 `gIrrOut`, u2 `gDepthOut`, u3 `Classification`, t4 `RayTable`,
/// t5 `LightBuf`, b6 `DdgiUpdate`); the shipped default is [`GI_MAX_IT_DEFAULT`] (64).
#[inline]
pub fn sdf_probe_update_spirv() -> &'static [u32] {
    SDF_PROBE_UPDATE_SPV.as_words()
}

/// The `GI_MAX_IT` sweep values the bench overrides via a spec-const (id 0) — the sphere-trace
/// `[loop]` trip counts `tests/ddgi_probe_gi_cost.rs` measures on the ONE committed `.spv` (plan
/// §5). No longer selects a per-variant blob (the trip count is a spec-const, default 64).
pub const GI_MAX_IT_VARIANTS: [u32; 4] = [32, 64, 96, 128];

/// The shipped-default `GI_MAX_IT` sphere-trace trip count (plan §6 placeholder — 64; the
/// orchestrator finalizes it from the `ddgi_probe_gi_cost` bench). Equals the
/// `[[vk::constant_id(0)]]` default baked into `sdf_probe_update.comp.spv`, so a pipeline built with
/// `spec_constants: &[]` runs this exact trip count.
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

/// The committed HW-RT rung R2a-3 TLAS-instance packer SPIR-V (`build_tlas_instances.comp`)
/// as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// One invocation per drawable instance; bound to a dedicated 4-binding set
/// { `StructuredBuffer<InstanceModelCol>` @0 (the shared M3 ring, read),
/// `StructuredBuffer<uint>` mesh-ids @1 (read), `StructuredBuffer<uint2>` BLAS-address table
/// @2 (read), `RWByteAddressBuffer` output @3 (the 64-byte instance records, write) } + a
/// 4-byte COMPUTE push ([`BUILD_TLAS_INSTANCES_PUSH_BYTES`] — `{ uint count }`). The pack
/// pre-pass dispatches `ceil(count / LOCAL_SIZE_X)` groups, writing one
/// `VkAccelerationStructureInstanceKHR` per instance for the per-frame TLAS build.
#[cfg(feature = "hwrt")]
#[inline]
pub fn build_tlas_instances_spirv() -> &'static [u32] {
    BUILD_TLAS_INSTANCES_SPV.as_words()
}

/// The byte size of the HW-RT rung R2a-3 TLAS-instance packer COMPUTE push constant
/// (`{ uint count }` — the drawable-instance bounds guard). Mirrors the shader's `Push`.
#[cfg(feature = "hwrt")]
pub const BUILD_TLAS_INSTANCES_PUSH_BYTES: u32 = 4;

/// The committed HW-RT rung R2a-4a AS-descriptor GPU-smoke SPIR-V
/// (`hwrt_as_descriptor_smoke.comp`) as a `u32` word stream, ready for
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module).
///
/// A minimal `rayQuery` compute bound to a 2-binding set { `RaytracingAccelerationStructure` @0
/// (a `DescriptorKind::AccelerationStructure` — the R2a-4a descriptor under test),
/// `RWStructuredBuffer<uint>` @1 (the hit-flag output) }, dispatched `1×1×1` by the R2a-4a GPU
/// smoke to exercise the AS-descriptor `p_next` write on real hardware. No push constants.
#[cfg(feature = "hwrt")]
#[inline]
pub fn hwrt_as_descriptor_smoke_spirv() -> &'static [u32] {
    HWRT_AS_DESCRIPTOR_SMOKE_SPV.as_words()
}

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
    // High — the widest tap budget (8 REAL evenly-spaced slices × 6 steps × 2 = 96 taps; Change B
    // owner-escalated — 8 divides SSAO_ROT_N(16) for even stride-2 spacing; variance of the slice
    // mean falls ~1/N, attacking the contact-shadow noise at the source).
    SsaoParams { radius: 0.5, slices: 8, steps: 6, strength: 2.5, eps: 1.0e-4 },
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
/// The rotation table size (`SSAO_ROT_N`); the per-pixel slot is `(r2 * SSAO_ROT_N) >> 24`
/// (an INTEGER scale of the Q0.24 R2 fraction; NO float `fract`/`floor`/div, so the host and
/// GPU pick the SAME rotation bit-exactly). Widened 16 -> 64: an even-slice axis set has only
/// `SSAO_ROT_N / SSAO_SLICES` EFFECTIVE dither classes (rotating the set by its own slice
/// spacing maps it onto itself) — 16 entries left just 2 classes at 8 slices, whose coherent
/// Hilbert+R2 layout read as un-blurrable streaks; 64 keeps >= 8 classes at a 2.8125° step.
pub const SSAO_ROT_N: u32 = 64;
/// The pre-baked `(cos, sin)` rotation table for the 64 evenly-spaced angles over [0, π):
/// angle k = k·(π/64) for k = 0..63 (a 2.8125° step), BYTE-IDENTICAL to the shader's
/// `SSAO_ROT[64]` so the host picks the same slot. Also the per-slice BASE axes (Change A —
/// `SSAO_ROT[sl * (SSAO_ROT_N / slices)]`; the strided entries are bit-identical to the
/// retired 16-entry table's).
//
// These literals are LOAD-BEARING: each must round to the EXACT `f32` the shader's
// `float2(...)` literal carries (the integer-hash rotation slot must agree bit-for-bit
// between the host oracle and the GPU). `clippy::approx_constant` (the `0.70710677` ==
// `FRAC_1_SQRT_2`) and `clippy::excessive_precision` would have us swap in the std constant
// or truncate digits — either DIVERGES the host literal from the frozen shader table, the
// exact drift this oracle exists to prevent. The `ssao_edsl_sync` cross-check pins the math.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub const SSAO_ROT: [(f32, f32); 64] = [
    (1.00000000, 0.00000000),
    (0.99879545, 0.04906768),
    (0.99518472, 0.09801714),
    (0.98917651, 0.14673047),
    (0.98078525, 0.19509032),
    (0.97003126, 0.24298018),
    (0.95694035, 0.29028466),
    (0.94154406, 0.33688986),
    (0.92387950, 0.38268343),
    (0.90398932, 0.42755508),
    (0.88192129, 0.47139674),
    (0.85772860, 0.51410276),
    (0.83146960, 0.55557024),
    (0.80320752, 0.59569931),
    (0.77301043, 0.63439327),
    (0.74095112, 0.67155898),
    (0.70710677, 0.70710677),
    (0.67155898, 0.74095112),
    (0.63439327, 0.77301043),
    (0.59569931, 0.80320752),
    (0.55557024, 0.83146960),
    (0.51410276, 0.85772860),
    (0.47139674, 0.88192129),
    (0.42755508, 0.90398932),
    (0.38268343, 0.92387950),
    (0.33688986, 0.94154406),
    (0.29028466, 0.95694035),
    (0.24298018, 0.97003126),
    (0.19509032, 0.98078525),
    (0.14673047, 0.98917651),
    (0.09801714, 0.99518472),
    (0.04906768, 0.99879545),
    (0.00000000, 1.00000000),
    (-0.04906768, 0.99879545),
    (-0.09801714, 0.99518472),
    (-0.14673047, 0.98917651),
    (-0.19509032, 0.98078525),
    (-0.24298018, 0.97003126),
    (-0.29028466, 0.95694035),
    (-0.33688986, 0.94154406),
    (-0.38268343, 0.92387950),
    (-0.42755508, 0.90398932),
    (-0.47139674, 0.88192129),
    (-0.51410276, 0.85772860),
    (-0.55557024, 0.83146960),
    (-0.59569931, 0.80320752),
    (-0.63439327, 0.77301043),
    (-0.67155898, 0.74095112),
    (-0.70710677, 0.70710677),
    (-0.74095112, 0.67155898),
    (-0.77301043, 0.63439327),
    (-0.80320752, 0.59569931),
    (-0.83146960, 0.55557024),
    (-0.85772860, 0.51410276),
    (-0.88192129, 0.47139674),
    (-0.90398932, 0.42755508),
    (-0.92387950, 0.38268343),
    (-0.94154406, 0.33688986),
    (-0.95694035, 0.29028466),
    (-0.97003126, 0.24298018),
    (-0.98078525, 0.19509032),
    (-0.98917651, 0.14673047),
    (-0.99518472, 0.09801714),
    (-0.99879545, 0.04906768),
];

/// The Dammertz 5-tap B3-spline weights for the SSAO à-trous kernel (`SSAO_ATROUS_H` in
/// `ssao_atrous.comp.hlsl`), for offsets `-2..=2`. EXACT `f32` literals. Equals
/// `boyko_shaderdsl::ssao::SSAO_ATROUS_H` and `shadow_atrous.comp.hlsl`'s `ATROUS_H`.
pub const SSAO_ATROUS_H: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];
/// The SSAO à-trous per-pass normalization guard (`SSAO_ATROUS_W_EPS` in
/// `ssao_atrous.comp.hlsl`). Equals `boyko_shaderdsl::ssao::SSAO_ATROUS_W_EPS`.
pub const SSAO_ATROUS_W_EPS: f32 = 1.0e-4;
/// The SSAO à-trous plane-fit RESIDUAL depth gate (`SSAO_BLUR_DEPTH_TOL` in
/// `ssao_atrous.comp.hlsl`), in linear view-Z (world-distance) units. A neighbour tap is
/// averaged in ONLY when `|residual| <= SSAO_BLUR_DEPTH_TOL` (the plane-fit residual, not the
/// raw difference); this keeps the filter WITHIN a flat/sloped surface while REJECTING the
/// mesh↔SDF silhouette. Equals `boyko_shaderdsl::ssao::SSAO_BLUR_DEPTH_TOL`; mirrored bit-for-bit
/// by [`golden_ssao_atrous`].
pub const SSAO_BLUR_DEPTH_TOL: f32 = 1.0;
/// The SSAO à-trous per-pass DEPTH falloff scale (`SSAO_BLUR_DEPTH_SIGMA` in
/// `ssao_atrous.comp.hlsl`), in linear view-Z units: the per-tap depth weight is
/// `clamp01(1 - (dz*dz) / (SSAO_BLUR_DEPTH_SIGMA * SSAO_BLUR_DEPTH_SIGMA))`, softening the
/// depth agreement WITHIN the hard [`SSAO_BLUR_DEPTH_TOL`] gate. Equals
/// `boyko_shaderdsl::ssao::SSAO_BLUR_DEPTH_SIGMA`; mirrored bit-for-bit by
/// [`golden_ssao_atrous`].
pub const SSAO_BLUR_DEPTH_SIGMA: f32 = 1.0;
/// The SSAO à-trous slope-aware depth-gate gradient clamp (`SSAO_BLUR_GRAD_CLAMP` in
/// `ssao_atrous.comp.hlsl`): each pass predicts a tap's linear-Z from the center's clamped local
/// gradient (min-magnitude one-sided differences at the fixed ±1 offset) and gates the
/// SVGF step-scaled RESIDUAL — the band follows a sloped/curved surface instead of truncating
/// the kernel, while a silhouette/background step (clamped) still rejects. Equals
/// `boyko_shaderdsl::ssao::SSAO_BLUR_GRAD_CLAMP`; mirrored bit-for-bit by [`golden_ssao_atrous`].
pub const SSAO_BLUR_GRAD_CLAMP: f32 = 0.1;

/// The SSAO à-trous push-constant size (`{ uint step; }`, `ssao_atrous.comp.hlsl`'s
/// `SsaoAtrousPush`) — 4 bytes, the single-`u32` hole-`step` compute push range (mirrors
/// `shadow_atrous.comp.hlsl`'s own 4-byte `step` push).
pub const SSAO_ATROUS_PUSH_BYTES: u32 = 4;


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

/// Multi-paradigm render-path plan, rung R-SDFFWD: `#[repr(C)]` the SDF forward-march compute
/// pass's OWN dedicated push constant (`shaders/sdf_forward_march.comp.hlsl`) — a NEW pass, not
/// sharing [`FineMarcherPush`]/[`GBUFFER_MARCHER_PUSH_BYTES`] (this pass has no coarse-cull, no
/// A2 AO, no MDF; it needs the reverse-Z view-Z decode constants instead). 40 bytes, HLSL
/// scalar-packed (the const-asserts below pin every offset):
///
///   offset  0 : u32     extent_w        render extent width (dispatch bound `idx < w*h`)
///   offset  4 : u32     extent_h        render extent height
///   offset  8 : f32     view_z_a        HAS_MESH reverse-Z decode `A` (don't-care w/o HAS_MESH)
///   offset 12 : f32     view_z_b        HAS_MESH reverse-Z decode `B`
///   offset 16 : [f32;3] light_dir       primary directional light direction (un-normalized)
///   offset 28 : u32     brick_enabled   M1 empty-skip gate; 0 = OFF (this rung's host default)
///   offset 32 : u32     brick_trilinear M2 trilinear+cubic gate; 0 = OFF
///   offset 36 : u32     brick_levels    M4 clip-map level count; 0 = OFF
///
/// `view_z_a`/`view_z_b` mirror [`boyko_render::view::forward_view_z_from_depth`]'s own `A`/`B`
/// derivation (`A = -near/(far-near)`, `B = near*far/(far-near)`) exactly — the shader's
/// `view_z = view_z_b / (depth - view_z_a)` is that function's algebraic inverse, ported to HLSL
/// so the compute pass does not need `near`/`far` themselves.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfForwardMarchPush {
    /// Render extent width — the dispatch bounds `idx < extent_w * extent_h`.
    pub extent_w: u32,
    /// Render extent height.
    pub extent_h: u32,
    /// HAS_MESH reverse-Z decode `A` (don't-care on the mesh-less SDFONLY variant).
    pub view_z_a: f32,
    /// HAS_MESH reverse-Z decode `B`.
    pub view_z_b: f32,
    /// The primary directional light direction (un-normalized; the shader normalizes it).
    pub light_dir: [f32; 3],
    /// M1 empty-space-skip gate: non-zero reads the pointer-grid bindings. `0` = OFF (the
    /// analytic-only march — this rung's host default; see this struct's doc).
    pub brick_enabled: u32,
    /// M2 trilinear+JCGT-cubic SURFACE-brick gate. `0` = OFF (this rung's host default).
    pub brick_trilinear: u32,
    /// M4 clip-map LEVEL COUNT. `0` = OFF (no level is ever selected — this rung's host
    /// default).
    pub brick_levels: u32,
}

/// Byte size of [`SdfForwardMarchPush`] — the SDF forward-march pass's COMPUTE push range.
pub const SDF_FORWARD_MARCH_PUSH_BYTES: u32 = core::mem::size_of::<SdfForwardMarchPush>() as u32;

const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, extent_w) == 0);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, extent_h) == 4);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, view_z_a) == 8);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, view_z_b) == 12);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, light_dir) == 16);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, brick_enabled) == 28);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, brick_trilinear) == 32);
const _: () = assert!(core::mem::offset_of!(SdfForwardMarchPush, brick_levels) == 36);
const _: () = assert!(SDF_FORWARD_MARCH_PUSH_BYTES == 40, "SdfForwardMarchPush must be 40 bytes");

impl SdfForwardMarchPush {
    /// Builds the push for a mesh-less (`GeometryLegs::Sdf`) dispatch: `view_z_a`/`view_z_b`
    /// are don't-care (the SDFONLY variant never reads them). The brick/clip-map acceleration
    /// stays OFF (`brick_enabled = brick_trilinear = 0`, `brick_levels = 0`) — see this struct's
    /// doc for why that is a deliberate, precedented 0%-gate rather than a missing feature.
    #[inline]
    pub const fn sdf_only(extent_w: u32, extent_h: u32, light_dir: [f32; 3]) -> Self {
        Self {
            extent_w,
            extent_h,
            view_z_a: 0.0,
            view_z_b: 0.0,
            light_dir,
            brick_enabled: 0,
            brick_trilinear: 0,
            brick_levels: 0,
        }
    }

    /// Builds the push for a `HAS_MESH` dispatch: `view_z_a`/`view_z_b` are the reverse-Z decode
    /// constants [`boyko_render::view::forward_view_z_from_depth`] itself derives from
    /// `near`/`far` (`A = -near/(far-near)`, `B = near*far/(far-near)`) — the caller passes them
    /// precomputed so this pass needs no `near`/`far` fields of its own.
    #[inline]
    pub const fn has_mesh(extent_w: u32, extent_h: u32, view_z_a: f32, view_z_b: f32, light_dir: [f32; 3]) -> Self {
        Self {
            extent_w,
            extent_h,
            view_z_a,
            view_z_b,
            light_dir,
            brick_enabled: 0,
            brick_trilinear: 0,
            brick_levels: 0,
        }
    }

    /// Re-views the push constants as their raw 40-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `f32` / `[f32; 3]` fields (all `Copy`,
        // every offset + the 40-byte total pinned by the const-asserts above, no uninit padding),
        // so its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern. The
        // `&self` borrow keeps the struct alive for the slice's lifetime; the slice is read-only
        // (no aliasing write).
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

/// Multi-paradigm render-path plan, rung R3b: the `viewt_from_depth` compute push constants
/// (mirrors `viewt_from_depth.comp.hlsl`'s `ViewtFromDepthPush`). `#[repr(C)]`, 12 B (`u32, u32,
/// f32`), the offsets pinned by the const-asserts below so a host/shader desync is a build
/// error (the same discipline as [`ClusterCullPush`]).
///
/// `mesh_norm` is the ONLY field this shader's `mesh_norm` selection reads — it is NOT
/// recomputed in HLSL from `camera_mode` (that would be a THIRD hand-written copy of the
/// marcher's `mesh_norm` ternary, alongside `sdf_gbuffer_composite.hlsl` and
/// `sdf_tile_cull.hlsl`). The host caller MUST derive it via
/// `boyko_render::gbuffer_depth::mesh_view_t_norm` (the single-sourced Rust mirror of that same
/// ternary, over [`CAM_MODE_PERSPECTIVE`]/[`MESH_DEPTH_T_MAX`]/[`SDF_TRACE_T_MAX`]) — never by
/// re-deriving the branch ad hoc at the call site.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewtFromDepthPush {
    /// The runtime extent width — the dispatch bounds guard (`ceil(img_w/8)` groups may run
    /// threads past the real extent; the shader discards `tid.x >= img_w`).
    pub img_w: u32,
    /// The runtime extent height — same bounds-guard role as [`Self::img_w`].
    pub img_h: u32,
    /// The host-precomputed mesh-depth ray-t normalizer (`boyko_render::gbuffer_depth::
    /// mesh_view_t_norm`): [`MESH_DEPTH_T_MAX`] under `CAM_MODE_PERSPECTIVE`, [`SDF_TRACE_T_MAX`]
    /// under ortho — EXACTLY the marcher's own `mesh_norm` value for this frame's camera.
    pub mesh_norm: f32,
}

/// Byte size of [`ViewtFromDepthPush`] — the `viewt_from_depth` pipeline's declared COMPUTE push
/// range (12 B).
pub const VIEWT_FROM_DEPTH_PUSH_BYTES: u32 = core::mem::size_of::<ViewtFromDepthPush>() as u32;

const _: () = assert!(core::mem::offset_of!(ViewtFromDepthPush, img_w) == 0);
const _: () = assert!(core::mem::offset_of!(ViewtFromDepthPush, img_h) == 4);
const _: () = assert!(core::mem::offset_of!(ViewtFromDepthPush, mesh_norm) == 8);
const _: () = assert!(VIEWT_FROM_DEPTH_PUSH_BYTES == 12, "ViewtFromDepthPush must be 12 bytes");

impl ViewtFromDepthPush {
    /// Builds the push from the runtime extent + the host-precomputed mesh-depth normalizer.
    #[inline]
    pub const fn new(img_w: u32, img_h: u32, mesh_norm: f32) -> Self {
        Self { img_w, img_h, mesh_norm }
    }

    /// Re-views the push constants as their raw 12-byte slice for `push_constants`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Self` is `#[repr(C)]` with only `u32` / `f32` fields (all `Copy`, every
        // offset + the 12-byte total pinned by the const-asserts above, no uninit padding), so
        // its `size_of` bytes are a fully-initialized, alignment-valid POD bit pattern. The
        // `&self` borrow keeps the struct alive for the slice's lifetime; read-only.
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

// --- PBR P1 — the HDR sun disc in the reflected environment (mirrors deferred_pbr.hlsl) ----

/// The sun-kernel exponent clamp floor (mirrors the shader's `SUN_KERNEL_EXPONENT_MIN`): a
/// fully rough surface (GGX alpha -> 1) maps its Blinn-Phong-equivalent exponent to `n -> 0`;
/// floored at 1 so the kernel stays a valid (if very broad) cosine lobe.
pub const SUN_KERNEL_EXPONENT_MIN: f32 = 1.0;
/// The sun-kernel exponent clamp ceiling (mirrors the shader's `SUN_KERNEL_EXPONENT_MAX`):
/// guards the `pow` blowup as alpha -> 0 (a mirror-smooth surface) while keeping the disc
/// visibly wider than one screen pixel.
pub const SUN_KERNEL_EXPONENT_MAX: f32 = 2048.0;
/// The default gate on the env sun-disc contribution (mirrors the shader's `SUN_ENV_WEIGHT`).
/// Owner-retunable at the visual gate; `1.0` keeps the disc's peak commensurate with the
/// material's own DFG-weighted specular tint (the kernel already peaks at exactly 1.0 only
/// where `R` points at the light and falls off sharply elsewhere).
pub const SUN_ENV_WEIGHT: f32 = 1.0;

// --- Render sky background — the visible sun disc baked into the BACKGROUND (mask == 0) ----

/// The FIXED cosine-power exponent of the sky background's sun disc (mirrors the shader's
/// `SKY_SUN_EXPONENT`). Unlike `sun_kernel_exponent` (roughness-driven, for the metal's own
/// reflected sun-disc term), this is a single moderate exponent (~512, a tight but clearly
/// visible disc) used for every directional light — the background is a flat environment
/// element, not a BRDF lobe. Owner-retunable.
pub const SKY_SUN_EXPONENT: f32 = 512.0;


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
mod tests;
