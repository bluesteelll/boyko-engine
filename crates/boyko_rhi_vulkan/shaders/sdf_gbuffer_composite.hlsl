// Render P1b compute shader: SDF + MESH HYBRID COMPOSITE writing an OFFSCREEN MRT
// G-buffer, the image-based rewrite of the rung-10 packed-buffer marcher
// (`sdf_depth_composite.hlsl`). It is a VERBATIM derivative of that shader's field
// eval + ray-gen + lighting — the SDF field math (primitive distances, boolean ops,
// smooth-min, central-difference gradient), the deterministic ortho/perspective
// camera, the directional Lambert+ambient light, and every scene constant are reused
// LINE-FOR-LINE so the host golden (`golden_composite_pixel_ex` in compute.rs)
// predicts the albedo output UNCHANGED. There are exactly TWO I/O edits vs the packed
// marcher; everything else is byte-identical (the determinism boundary, INVIOLABLE):
//
//   (1) the per-pixel mesh DEPTH read becomes a SAMPLED-IMAGE fetch
//       `gDepth.Load(int3(px, py, 0)).r` (a `Texture2D<float>` at binding 1, the
//       rasterized D32_SFLOAT image transitioned to SHADER_READ_ONLY) INSTEAD of the
//       packed `asfloat(Buf[DEPTH_BASE + idx])` buffer-region read. The depth VALUE
//       and its `< DEPTH_CLEAR` / `* T_MAX` interpretation are identical; only the
//       SOURCE (a sampled image vs a buffer region) changes.
//   (2) the marcher color output becomes a STORAGE-IMAGE store
//       `gAlbedo[uint2(px, py)] = float4(color, 1.0)` (an `RWTexture2D<float4>` at
//       binding 2) INSTEAD of the packed `Buf[pixel_base() + idx] = pack_rgba(color)`.
//       The float->UNORM store quantizes `clamp(color,0,1)` to bytes; the host
//       golden's `pack_rgba` uses `(x*255+0.5)` rounding — the <=1-LSB difference is
//       absorbed by the +/-2/255 per-channel tolerance (same as rungs 8..11).
//
// # Deferred-shading SPLIT (increment 1)
//
// The A1 shadow + A2 AO COMPOSITE moves OUT of this marcher into a fullscreen
// `deferred_pbr.comp` resolve. The marcher computes base/shadow/ao exactly as before
// (in-register, exact p/n, frozen field) but WRITES ATTRIBUTES rather than compositing:
//   * binding 2 (gAlbedo) — the UNMULTIPLIED base color (Lambert+ambient on an SDF hit,
//     else MESH_COLOR / BACKGROUND). NO `base * shadow * ao`.
//   * binding 4 (gMaterial) — `(r = vis, g = 0, b = mask, a = 1)` where
//     `vis = clamp(shadow*ao, 0, 1)` (ONE combined visibility factor, quantized once to
//     R8) and `mask = 1` on the two SDF-LIT arms, `0` on mesh / background / empty. The
//     resolve computes `lit = (mask == 1) ? base * vis : base` (a strict if on mask).
//   * binding 3 (gNormal) — `sdf_normal(p)*0.5+0.5` on an SDF hit, else neutral 0.5 (the
//     world normal encoded to UNORM). UNCHANGED + UNREAD this increment (forward-compat).
//
// # The vocabulary set (set 0, written ONCE — no per-frame vkUpdateDescriptorSets)
//
//   binding 0 : StructuredBuffer<uint> (READ-ONLY) — the packed edit-list header
//               (word 0 = edit_count, then MAX_SDF_EDITS * SdfEdit; the rung-9
//               `encode_edit_list` format). There is NO depth region and NO pixel
//               region in the buffer anymore — depth is the sampled image, the color
//               is the storage image.
//   binding 1 : Texture2D<float> (SAMPLED) — the mesh depth (DEPTH-aspect view of the
//               D32_SFLOAT image), fetched with `.Load` (OpImageFetch, no sampler).
//   binding 2 : RWTexture2D<float4> (STORAGE, rgba8) — albedo (the UNMULTIPLIED base).
//   binding 3 : RWTexture2D<float4> (STORAGE, rgba8) — world normal (unread this incr.).
//   binding 4 : RWTexture2D<float4> (STORAGE, rgba8) — material (r=vis, b=mask, a=1).
//   binding 5 : cbuffer Camera (UNIFORM) — the 80-byte extent/camera block, written
//               ONCE at setup (NOT per-frame). This replaces the rung-10 push
//               constant: the vocabulary pipeline uses a DEDICATED layout, so the
//               encoder's `push_constants` (which records against the device-shared
//               compute layout) is incompatible; a UBO sidesteps both the per-frame
//               push and the layout-incompat (review fix P1a-O1, option b). The camera
//               params feed ray-gen with plain IEEE ops (no fast math) — determinism
//               preserved.
//   binding 6 : StructuredBuffer<TileBound> (READ-ONLY) — the Render P4b per-tile
//               coarse-cull bound, written by `sdf_tile_cull.hlsl`. Read ONLY when the
//               `coarse_enabled` push constant is non-zero; the OFF path never touches
//               it (byte-identical to the pre-P4b marcher — the 0%-gate).
//   binding 7 : StructuredBuffer<MaterialGpu> (READ-ONLY) — the PBR MVP-2 material table
//               (the marcher fetches the picked edit's RAW LINEAR `base_color`).
//   binding 8 : RWTexture2D<float> (STORAGE, r32f) — the Lighting L0b `gViewT` lane (the
//               surface ray param `t` for the resolve's `P = ro + rd * t`). The 9th vocab
//               entry, within the raised 12-binding cap (C1).
//   binding 9 : StructuredBuffer<uint> (READ-ONLY) — the SDF brick-atlas M1 POINTER GRID
//               (a dense lattice of BrickClass codes, built CPU-side from the edit
//               authority). Read ONLY when the `brick_enabled` push constant is non-zero;
//               the OFF path never touches it (byte-identical to the pre-M1 marcher — the
//               0%-gate). The 10th vocab entry, within the 12-binding cap.
//
// # The push constant (Render P4b + B1 + A1/A2 — pushed against THIS pipeline's OWN
//   dedicated layout via `push_compute_constants`, NOT the shared `push_constants`)
//
// A 32-byte `[[vk::push_constant]]` block (`FineMarcherPush`): `uint coarse_enabled`
// (offset 0) gates the coarse cull, `float omega` (offset 4) carries the Render B1
// over-relaxation factor, `uint lighting_flags` (offset 8) gates A1/A2, and `float3
// light_dir` (offset 16) is the directional light. `coarse_enabled == 0` (cull-off) keeps
// the marcher byte-identical to today; `!= 0` reads binding 6 and either early-outs an
// EMPTY tile (mesh/background composite) or seeds `t = near_t`. `omega == 1.0` keeps the
// sphere-trace TEXTUALLY the frozen plain loop (the 0%-gate); `> 1.0` enables Keinert
// over-relaxation with an exact-retreat safeguard. `lighting_flags == 0` keeps the lit
// color the bare Lambert term (the 0%-gate); bit 0 folds in A1 soft shadows, bit 1 the A2
// AO. 32 bytes fits the declared 80-byte COMPOSITE push range.
//
// # The `[[vk::image_format("rgba8")]]` qualifier (REQUIRED)
//
// `shaderStorageImageWriteWithoutFormat` is NOT enabled at device creation, so each
// `RWTexture2D<float4>` must declare an explicit SPIR-V `OpTypeImage` format matching
// its R8G8B8A8_UNORM view — without it DXC defaults to `Rgba32f`, which the validation
// layer flags as a storage-image format mismatch (the store would be undefined). This
// mirrors the P1a `sdf_editlist_storage_image.hlsl` pattern.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_gbuffer_composite.hlsl \
//       -Fo sdf_gbuffer_composite.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_gbuffer_composite.comp.spv

StructuredBuffer<uint> Buf : register(t0); // binding 0: edit-list header (READ-ONLY)

// The shared determinism-frozen field gateway. INCLUDE CONTRACT: `Buf` (above) must
// be in scope first — the field eval reads the packed edit-list out of it. This is a
// VERBATIM cut of the rung-10/P1b field math; `field_distance(p)`/`sdf(p)` and the
// host golden `golden_composite_pixel_ex` stay byte-identical. Resolved relative to
// this .hlsl at DXC time.
#include "sdf_field.hlsli"

// Shared camera ray-generation (PBR MVP-2 Phase 0). The ORTHO + PERSPECTIVE ray-gen
// was extracted VERBATIM into this header so the marcher and the deferred PBR resolve
// share ONE ray-gen (the resolve reconstructs the per-pixel view direction from it).
// Resolved relative to this .hlsl at DXC time; included AFTER `cbuffer Camera` is in
// scope below would be ideal, but the header takes the camera fields as PARAMETERS
// (not a global cbuffer read), so include order vs the cbuffer is irrelevant.
#include "ray_gen.hlsli"

// binding 1: the mesh depth as a SAMPLED IMAGE (DEPTH-aspect view of the D32_SFLOAT
// rasterized image). `.Load(int3(px,py,0)).r` is an unfiltered fetch (OpImageFetch),
// so no sampler is consumed — the descriptor is a plain SAMPLED_IMAGE.
Texture2D<float> gDepth : register(t1);

// bindings 2..4: the MRT G-buffer STORAGE IMAGES. `[[vk::image_format("rgba8")]]`
// pins each `OpTypeImage` to `Rgba8` so it matches the R8G8B8A8_UNORM views (see the
// header note — shaderStorageImageWriteWithoutFormat is OFF).
[[vk::image_format("rgba8")]] RWTexture2D<float4> gAlbedo   : register(u2);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gNormal   : register(u3);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gMaterial : register(u4);

// Camera modes selected by `cam.camera_mode`. ORTHO is the golden-frozen path; the
// PERSPECTIVE branch is strictly additive. Mirrored host-side in compute.rs as
// `CAM_MODE_ORTHO` / `CAM_MODE_PERSPECTIVE`. The ray-gen enum itself lives in
// `ray_gen.hlsli` (`RAYGEN_CAM_ORTHO` / `RAYGEN_CAM_PERSPECTIVE`); these aliases keep
// the marcher's existing `camera_mode == CAM_PERSPECTIVE` site readable.
static const uint CAM_ORTHO       = RAYGEN_CAM_ORTHO;
static const uint CAM_PERSPECTIVE = RAYGEN_CAM_PERSPECTIVE;

// The legacy fixture extent reproduced when `img_w`/`img_h` are zero (an all-zero UBO
// tail), and the host const-assert anchor (`SDF_IMG_W`/`SDF_IMG_H` both equal this).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;

// binding 5: the extent/camera UNIFORM block (written ONCE at setup, NOT per-frame).
// Byte-identical field layout to the rung-10 `PushConstants` block (and to the host
// `CompositePushConstants` `#[repr(C)]` POD): std140/std430 scalar + `float4` rules
// agree for this all-scalar/`float4` block at these offsets. Field offsets are pinned
// host-side by the existing `COMPOSITE_PC_*` const-asserts.
//
//   offset  0 : uint   count        total PIXEL count = img_w * img_h
//   offset  4 : uint   img_w        runtime extent width  (0 => IMG_W_DEFAULT)
//   offset  8 : uint   img_h        runtime extent height (0 => IMG_H_DEFAULT)
//   offset 12 : uint   camera_mode  CAM_ORTHO | CAM_PERSPECTIVE
//   offset 16 : float4 cam_eye      xyz = eye world pos          (PERSPECTIVE)
//   offset 32 : float4 cam_forward  xyz = forward basis, w = tan(fovY/2) (PERSPECTIVE)
//   offset 48 : float4 cam_right    xyz = right basis,  w = aspect (W/H)  (PERSPECTIVE)
//   offset 64 : float4 cam_up       xyz = up basis                (PERSPECTIVE)
//   total: 80 bytes, 16-byte aligned.
//
// SDF brick-atlas M4 (clip-map LOD): the b5 UBO is WIDENED to 224 bytes (host `B5_CAMERA_UBO_BYTES_M4`)
// — the 80-byte camera block above + an ARRAY of `BRICK_LEVELS` (3) per-level `M4Level` blocks at
// offset 80 (`M2_GRID_PARAMS_OFFSET`), each three std140 `float4` lanes (48 B). M4 REPLACES the single
// M2 grid block at offset 80 with `m2_levels[BRICK_LEVELS]`. Level 0's 48-byte block is byte-FOR-byte
// the old M2 tail (the OFF/N=1 keystone), so `brick_levels == 1` reads `m2_levels[0]` and is identical
// to the pre-M4 M2 marcher. The marcher reads the M4 blocks ONLY on the `brick_trilinear` path;
// `brick_trilinear == 0` never touches these fields (byte-identical to M1). The host writes them via
// `M4GridParams::{camera_centered,near_field_only}().as_ubo_bytes()`; each 48-byte entry mirrors that
// `#[repr(C)]` exactly (level `L` at byte `L*48`):
//   +0  : float4 origin_brick_world   xyz = level L grid min world corner, w = brick_world_at_level(L)
//   +16 : float4 dims_atlas_dim       xyz = grid dims [x,y,z] (as f32), w = M2_ATLAS_DIM (as f32)
//   +32 : float4 band_voxel_inv_atlas x = band_half_at_level(L), y = voxel_size_at_level(L),
//                                      z = 1/atlas_dim, w = level index L
// (`dims_atlas_dim` is read as a `uint4` via `(uint)` casts — an exact small-integer f32↔uint round trip.)
//
// The number of nested clip-map levels (mirror host `boyko_sdf_math::brick::BRICK_LEVELS`). The b5 UBO
// tail carries this many `M4Level` blocks; the marcher loops over `pc.brick_levels <= BRICK_LEVELS`.
static const uint BRICK_LEVELS = 3u;

// One clip-map level's 48-byte b5 UBO block (mirror host `M4LevelParams` / the old M2 lane layout).
struct M4Level {
    float4 origin_brick_world;   // xyz = level grid origin, w = brick_world_at_level(L)
    float4 dims_atlas_dim;       // xyz = grid dims (f32), w = atlas_dim (f32); read via (uint) cast
    float4 band_voxel_inv_atlas; // x = band_half(L), y = voxel_size(L), z = 1/atlas_dim, w = level L
};

cbuffer Camera : register(b5) {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
    // The N-level clip-map array tail at offset 80 (`M2_GRID_PARAMS_OFFSET`). Each 48-byte entry is
    // 16-aligned so the array packs contiguously (level L at byte 80 + L*48). Level 0 == the old M2
    // tail byte-for-byte (the OFF/N=1 keystone). 80 + 3*48 == 224 (`B5_CAMERA_UBO_BYTES_M4`).
    M4Level m2_levels[BRICK_LEVELS]; // offsets 80 / 128 / 176
};

// Render P4b: the per-tile coarse-cull bound, READ-ONLY here (the coarse pass writes
// it). Byte-identical to the host `#[repr(C)] TileBound` (16 B std430). binding 6.
struct TileBound {
    float near_t;
    float far_t;
    uint  flags;
    uint  _pad;
};
StructuredBuffer<TileBound> Tiles : register(t6);

// PBR MVP-2: the material table (`MaterialGpu[]`), binding 7. The marcher reads
// `materials[id].base_color` (the picked edit's RAW
// LINEAR albedo) to write into gAlbedo; the resolve reads metallic/roughness/etc. The
// std430 layout MIRRORS `boyko_render::material::MaterialGpu` (48 B / 12 words):
//
//   off 0  : float4 base_color   rgb = linear base color, w = alpha/cutoff
//   off 16 : float4 mrr          [metallic, roughness, reflectance, bitcast(flags)]
//   off 32 : float4 emissive     rgb = linear emissive, w unused
//
// The MATERIAL_GPU_WORDS == 12 pin (below) mirrors SDF_EDIT_WORDS == 12 in
// sdf_field.hlsli so a host/shader layout desync is a build error host-side.
struct MaterialGpu {
    float4 base_color;
    float4 mrr;
    float4 emissive;
};
StructuredBuffer<MaterialGpu> Materials : register(t7);

// Lighting L0b: the `gViewT` G-buffer lane (binding 8, the 9th vocab entry — within the
// raised 12-binding cap, C1). An R32_SFLOAT STORAGE image carrying the marcher's surface
// ray parameter `t` (the SDF-hit `p = ro + rd * t`), which the deferred resolve reads to
// reconstruct the world position `P = ro + rd * t` for point/spot attenuation. The full
// fp32 lane avoids the precision banding an 8-bit gMaterial.a would cause.
// `[[vk::image_format("r32f")]]` pins the `OpTypeImage` to `R32f` (matching the view;
// `shaderStorageImageWriteWithoutFormat` is OFF). WRITTEN AT ALL THREE TERMINAL EXITS,
// exactly once per REAL pixel per frame (C2): the real marched `t` on the SDF-lit arm, a
// `1.0e30` sentinel on the EMPTY/mesh/background arms (never read on a non-lit pixel — the
// resolve gates the read inside `mask == 1`). The kernel-entry over-hang guard
// (`if (idx >= count) return;`) is a legitimate NON-writing exit: those threads own no
// pixel, so gViewT is deliberately NOT written there.
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u8);

// SDF brick-atlas M1 (empty-space-skip): the pointer grid, binding 9 (the 10th vocab
// entry — within the raised 12-binding cap, C1). A dense `grid_dims.x*y*z` lattice of
// `uint` BrickClass codes (0 = EmptyOutside, 1 = EmptyInside, 2 = Surface), built CPU-side
// by `boyko_sdf_math::brick::build_pointer_grid` from the ONE edit authority and uploaded
// as a plain `StructuredBuffer<uint>` (NO 3D image, NO trilinear — that is M2). Read ONLY
// when `pc.brick_enabled != 0`; the OFF path never touches it (the 0%-gate). Linear index
// `ix + iy*W + iz*W*H` (`W = grid_dims.x`, `H = grid_dims.y`) — the SAME order the host
// builder writes.
StructuredBuffer<uint> PointerGrid : register(t9);

// SDF brick-atlas M4 (clip-map LOD): the LEVEL-1 + LEVEL-2 pointer grids, bindings 11 + 13 (the same
// dense `M2_GRID_DIM³` lattice of BrickClass codes as level 0's @t9, but built from the authority at
// the coarser level's snapped grid). N SEPARATE count=1 bindings (NOT a dynamic resource array): the
// RHI bind-group WRITE path is one-descriptor-per-binding, so a `BrickAtlas[lvl]` dynamic array
// (descriptorCount=N) is not writable. HLSL cannot dynamically index separate resources, so the
// marcher dispatches via a STATIC branch-ladder (`if (lvl==0) {*0} else if (lvl==1) {*1} ...`). On the
// OFF/N=1 path (`pc.brick_levels == 1`) the branch-ladder takes ONLY the lvl==0 arm, so t11/t13 are
// bound-but-unread — but they MUST be bound (DXC keeps the static refs past the runtime level branch).
StructuredBuffer<uint> PointerGrid1 : register(t11); // M4 level 1 pointer grid
StructuredBuffer<uint> PointerGrid2 : register(t13); // M4 level 2 pointer grid

// SDF brick-atlas M2 (trilinear SURFACE bricks): the dense `M2_ATLAS_DIM³` 3D `R8_SNORM`
// (or `R16_SFLOAT` fallback) atlas, binding 10 (the 11th vocab entry — within the 12-binding
// cap, C1). One apron'd `BRICK_ALLOC³` (10³) tile per M2 grid cell, baked CPU-side from the ONE
// edit authority by `boyko_rhi_vulkan::compute::bake_brick_atlas` (principle 0 — no parallel
// field store). The `Texture3D` + `SamplerState` at the SAME `[[vk::binding(10, 0)]]` collapse
// to ONE `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` under DXC (the same pattern
// `fullscreen_sample.fs.hlsl` uses), so the marcher's `BrickAtlas.SampleLevel(BrickSampler, …)`
// point fetch reads the SAME corners the baker wrote. The sampler is NEAREST / clamp-to-edge /
// no-mip (BUG-M2-GPU-1): the M2 DDA cubic needs the EXACT texel corner values, NOT trilinear
// interpolation — a texel-center uvw `(texel + 0.5)/atlas_dim` through a NEAREST sampler returns
// the exact decoded snorm, bit-matching the host `decode_snorm8(brick[i])` integer-index fetch,
// AND exercises the combined descriptor correctly (a `.Load` on the texture half of a combined
// image+sampler read 0 on-device → the cubic was degenerate → the M2 branch was dead). Clamp keeps
// an apron-edge fetch reading the edge texel, not a neighbour wrap.
//
// Read ONLY when `pc.brick_trilinear != 0`; the OFF path never touches it (byte-identical to the
// M1 marcher — the M2 0%-gate). The R2 contract: the marcher SPIR-V STATICALLY references t10/s10
// inside the runtime-gated M2 branch, so the layout MUST declare binding 10 = combined-image-
// sampler and bind a VALID atlas/sampler even when the trilinear path is gated OFF (the windowed
// present path runs `brick_trilinear == 0` → bound-but-unread, byte-identical output), or
// `vkCreateComputePipelines` / `vkCmdDispatch` trip the layout VUIDs (the M1 R2 lesson at t9).
[[vk::binding(10, 0)]] Texture3D<float>  BrickAtlas   : register(t10);
[[vk::binding(10, 0)]] SamplerState      BrickSampler : register(s10);

// SDF brick-atlas M4 (clip-map LOD): the LEVEL-1 + LEVEL-2 atlases + their NEAREST samplers, combined
// image+sampler at bindings 12 + 14 (the same `M2_ATLAS_DIM³` `R8_SNORM` tile-grid as level 0's @t10,
// baked from the authority at the coarser level's geometry). 6 brick bindings total (9..=14, under the
// 16-binding cap). The marcher's branch-ladder calls `m2_surface_hit(... BrickAtlas1, BrickSampler1 ...)`
// in the lvl==1 arm, `... BrickAtlas2, BrickSampler2 ...` in the lvl==2 arm. On the OFF/N=1 path these
// are bound-but-unread (the ladder takes only the lvl==0 arm), yet MUST be bound — DXC keeps the static
// `register(t12)`/`register(t14)` refs past the runtime level branch (the same R2 contract as t10).
[[vk::binding(12, 0)]] Texture3D<float>  BrickAtlas1   : register(t12);
[[vk::binding(12, 0)]] SamplerState      BrickSampler1 : register(s12);
[[vk::binding(14, 0)]] Texture3D<float>  BrickAtlas2   : register(t14);
[[vk::binding(14, 0)]] SamplerState      BrickSampler2 : register(s14);

// Mirrors `boyko_render::material::MATERIAL_GPU_WORDS` (48 B / 4 = 12). A documentation
// + intent pin; the StructuredBuffer<MaterialGpu> element stride is the std430 layout.
static const uint MATERIAL_GPU_WORDS = 12u;

// The fallback material id used when an SDF hit's argmin attribution is ambiguous or the
// table is unbound (id 0 is the engine's default material). NOT used by the field eval.
static const uint DEFAULT_MATERIAL_ID = 0u;

// Render P4b: a push constant on the fine pipeline (the ONLY push — the camera is the
// UBO @ b5). `coarse_enabled == 0` keeps the OFF path BYTE-IDENTICAL to today's marcher
// (the 0%-gate anchor); `!= 0` reads the tile's `TileBound` and culls.
//
// Render B1: a second 4-byte field `omega` carries the Keinert over-relaxation factor,
// host-clamped to [1.0, 1.99]. At `omega == 1.0` the marcher's live path is TEXTUALLY
// the frozen plain sphere-trace (see the gated loop below — the 0%-gate).
//
// Render A1/A2: widened 8 -> 32 bytes to carry the directional-light state.
//   offset  0 : uint   coarse_enabled  (unchanged)
//   offset  4 : float  omega           (unchanged)
//   offset  8 : uint   lighting_flags  bit 0 = A1 shadows, bit 1 = A2 AO; 0 = OFF path
//   offset 12 : uint   _pad            aligns light_dir to offset 16 (std430 float3)
//   offset 16 : float3 light_dir       the directional-light direction (un-normalized)
//   offset 28 : float  _pad2           tail pad to a 32-byte stride
// 32 bytes fits the declared 80-byte COMPOSITE push range, so pipeline creation is
// unchanged. Byte-identical to the host `#[repr(C)] FineMarcherPush` (the host
// const-asserts pin every offset; a non-default light_dir GPU test catches mis-packing).
// `lighting_flags == 0` selects the OFF path: the marcher emits the bare Lambert albedo,
// byte-identical to the pre-A1/A2 shader (the 0%-gate).
//
// SDF brick-atlas M1 (empty-space-skip): widened 32 -> 64 bytes to carry the pointer-grid
// uniforms. `brick_enabled == 0` keeps the marcher byte-identical to the pre-M1 path (the
// grid @binding 9 is never read, the march is the exact analytic sphere-trace — the
// 0%-gate); `!= 0` reads `PointerGrid[brick_index(p)]` before each `sdf(p)` and skips an
// EmptyOutside brick to its AABB exit (sound by construction — the conservative classifier
// guarantees no surface within band_half of an EMPTY brick). The hit/normal stay ANALYTIC.
//
// The M1 block is ordered VECTOR-FIRST so both `float3 grid_origin` (@32) and `uint3
// grid_dims` (@48) land on 16-byte boundaries — the std430/HLSL `vec3`-aligns-to-16 rule
// (the same rule `light_dir @16` obeys). The scalar gate/size fill the vec3 TAIL slots
// (@44, @60), so the HLSL std430 layout and the host `#[repr(C)] FineMarcherPush`
// (4-byte-packed) are byte-identical with NO explicit pad fields.
//   offset 32 : float3 grid_origin    pointer-grid min world corner (cell 0,0,0)
//   offset 44 : uint   brick_enabled  M1 empty-skip gate; 0 = OFF (byte-identical)
//   offset 48 : uint3  grid_dims      cells per axis [x, y, z]
//   offset 60 : float  brick_world    pointer-grid cell world size (one brick)
// 64 bytes fits the declared 80-byte COMPOSITE push range. The host const-asserts pin every
// offset; a non-default-grid GPU test catches a packing slip the way the light_dir@16 pin
// does for A1/A2.
//
// SDF brick-atlas M2: widened 64 -> 80 bytes to carry the `brick_trilinear` gate at offset 64
// (the first slot of the 16-byte headroom the 64-byte M1 layout left inside the declared 80-byte
// COMPOSITE range). `brick_trilinear == 0` keeps the marcher byte-identical to M1 (the M2 atlas
// @binding 10 is never sampled, SURFACE bricks fold the analytic field — the M2 0%-gate); `!= 0`
// samples the atlas + runs the JCGT cubic inside SURFACE bricks. INDEPENDENT of `brick_enabled`
// (the M1 empty-skip): the gates are orthogonal. The `_pad3[3]` fills the 16-byte tail (offsets
// 68/72/76), matching the host `#[repr(C)] FineMarcherPush` byte-for-byte (offsets const-asserted).
//   offset 64 : uint   brick_trilinear  M2 trilinear+cubic gate; 0 = OFF (byte-identical to M1)
//
// SDF brick-atlas M4 (clip-map LOD): the `brick_levels` count @68 reuses the first M2 `_pad3` slot (the
// tail pad shrinks to `uint2 _pad3` @72), so the struct SIZE is unchanged (the declared 80-byte range).
// `brick_levels == 1` (or 0) is the OFF/M2-identical path (`select_level` loops once over level 0); `> 1`
// makes the marcher's branch-ladder dispatch the finest enclosing level (read from the b5 UBO array tail).
//   offset 68 : uint   brick_levels     M4 clip-map level count; 1 (or 0) = OFF (byte-identical to M2)
//   offset 72 : uint2  _pad3            tail pad to the 80-byte COMPOSITE stride
[[vk::push_constant]] struct PushConstants {
    uint   coarse_enabled;  // offset 0 (unchanged)
    float  omega;           // offset 4 — Keinert over-relaxation factor, host-clamped [1.0, 1.99]
    uint   lighting_flags;  // offset 8 — bit 0 = A1 shadows, bit 1 = A2 AO; 0 = OFF
    uint   _pad;            // offset 12 — std430 pad so light_dir lands at offset 16
    float3 light_dir;       // offset 16 — directional-light direction (un-normalized)
    float  _pad2;           // offset 28 — pad to the 32-byte A1/A2 stride
    float3 grid_origin;     // offset 32 — pointer-grid min world corner (16-aligned vec3)
    uint   brick_enabled;   // offset 44 — M1 empty-skip gate; 0 = OFF (byte-identical)
    uint3  grid_dims;       // offset 48 — pointer-grid cells per axis (16-aligned vec3)
    float  brick_world;     // offset 60 — pointer-grid cell world size
    uint   brick_trilinear; // offset 64 — M2 trilinear+cubic gate; 0 = OFF (byte-identical to M1)
    uint   brick_levels;    // offset 68 — M4 clip-map level count; 1/0 = OFF (byte-identical to M2)
    uint2  _pad3;           // offset 72 — tail pad to the 80-byte COMPOSITE stride
} pc;

// P4b: the EMPTY flag bit (mirrors the host `TILE_FLAG_EMPTY` + sdf_tile_cull.hlsl).
static const uint TILE_FLAG_EMPTY = 1u;
// The coarse tile edge in fine pixels (mirrors the host `TILE_SIZE`).
static const uint TILE_SIZE_FINE = 8u;

// Resolves the runtime extent, falling back to the legacy 64x64 fixture when a field
// is zero (so an all-zero UBO tail reproduces the golden — the 0%-gate).
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// --- Deterministic scene constants (mirrored host-side in compute.rs) ---------

// CAM_Z + HALF_EXTENT moved to `ray_gen.hlsli` (RAYGEN_CAM_Z / RAYGEN_HALF_EXTENT)
// alongside the extracted ortho ray-gen that consumes them.

// PBR MVP-2: the SDF surface no longer has a hardcoded BASE_COLOR / AMBIENT in the
// marcher — gAlbedo carries the picked material's RAW LINEAR `base_color` (from the
// material SSBO) and the resolve runs the full Cook-Torrance BRDF (ambient via
// EnvBRDFApprox). The static `LIGHT_DIR` was already removed (BUG-A-NDOTL); the A1/A2
// marches consume the runtime `pc.light_dir`.

static const float3 BACKGROUND = float3(0.05, 0.05, 0.1);  // miss color
// The flat mesh albedo — a green clearly distinct from both the SDF lit color
// (warm orange/red) and the background (dark blue). Mirrored host-side.
static const float3 MESH_COLOR = float3(0.15, 0.65, 0.25);

// Sphere-trace tuning (the §S2 march budget; identical to rung 9/10).
static const float EPS    = 0.001;  // hit threshold on |sdf|
static const float T_MAX  = 10.0;   // miss distance bound (= depth-1.0 far plane)
static const uint  MAX_IT = 128u;   // max march steps per ray (the §S2 ceiling)
// NOTE: the field-eval tuning consts (GRAD_H, FAR) + the field-layout contract +
// the field functions (Edit/load_edit/sd_*/edit_distance/smin/smax/combine/sdf/
// sdf_normal) live in `sdf_field.hlsli` (included below) — the determinism-frozen
// shared field gateway. See the `#include` at the `Buf` declaration.

// The depth value the depth attachment was CLEARED to (the far plane, 1.0). A pixel
// whose stored depth is >= this sentinel had NO mesh fragment rasterized.
static const float DEPTH_CLEAR = 1.0;

// --- Render A1 (SDF cone-trace soft shadows) + A2 (SDF 5-tap AO) ---------------
//
// Both are strict FIELD-CONSUMERS: they CALL the FROZEN `field_distance` (the shared
// gateway in sdf_field.hlsli — no fast-math, no reorder) and accumulate consumer-side
// (the shadow min-track, the AO deficit). The field math is NEVER touched. Mirrored
// host-side in compute.rs (`host_soft_shadow` / `host_ao` / `host_shade`) within
// +/-3/255 (consumer-side relaxable, NOT bit-exact for the ON path; the OFF path stays
// byte-exact). Tuning consts mirror the host owner-default constants.

// lighting_flags bits (mirror host LIGHTING_FLAG_SHADOWS / LIGHTING_FLAG_AO).
static const uint LIGHTING_FLAG_SHADOWS = 1u;
static const uint LIGHTING_FLAG_AO      = 2u;

// A1 tuning (owner defaults, owner-retunable). Mirror the host consts.
static const float SHADOW_K         = 8.0;          // penumbra hardness
static const float SHADOW_MINT      = 16.0 * GRAD_H; // march start offset (replaces a normal-offset)
static const float SHADOW_MINT_STEP = 16.0 * GRAD_H; // minimum per-step advance (floor on d/L)
static const float SHADOW_HIT_EPS   = 2.0 * EPS;    // occluder-hit threshold
static const float SHADOW_NDOTL_EPS = 0.0;          // signed n.L grazing/back-face cutoff

// A2 tuning (owner defaults). Mirror the host consts.
static const float AO_STEP     = 0.1;   // step between the 5 taps along the normal
static const float AO_FALLOFF  = 0.95;  // per-tap geometric falloff (AO_FALLOFF^i)
static const float AO_STRENGTH = 1.0;   // overall occlusion strength

// A1: clamped-step Quilez BASIC soft shadow (NO sqrt — minimal FP-parity surface). March
// `t` from SHADOW_MINT toward the NORMALIZED light `L`, tracking the smallest
// `SHADOW_K * d / t` (the penumbra estimate). Signed-ndotl early-out skips grazing /
// back-faces (replaces a normal-offset, prevents acne). The `/L` Lipschitz correction on
// the STEP (floored at SHADOW_MINT_STEP) keeps the super-Lipschitz smin from leaking light
// through thin occluders. Returns visibility in [0,1] (1 = fully lit, 0 = occluded).
float sdf_soft_shadow(float3 p, float3 n, float3 L) {
    if (dot(n, L) <= SHADOW_NDOTL_EPS) {
        return 0.0; // surface faces away from the light — fully shadowed (and skips the march)
    }
    // === GENERATED sdf_soft_shadow BEGIN (boyko_shaderdsl::emit) ===
    // The penumbra loop+tail below is MACHINE-GENERATED by boyko_shaderdsl::shadow
    // (Track-B Inc 4b: the runtime [loop] + brk + the field_distance call1), the SAME single
    // source whose f32 Eval backend mirrors this control flow. The dot/early-return preamble
    // ABOVE stays hand-written (framing b). Regenerate with:
    //     cargo run -p boyko_shaderdsl --features emit --bin emit_field
    // Byte-identical SPIR-V vs the prior hand-written body (R1: `t = t + max` == `t += max`);
    // the sdf_soft_shadow_matches_edsl_emit sync guard pins this span to the generator.
    float res = 1.0;
    float t = SHADOW_MINT;
    [loop]
    for (uint i = 0u; i < MAX_IT; ++i) {
        float d = field_distance(p + L * t);
        res = min(res, SHADOW_K * d / t);
        if (d < SHADOW_HIT_EPS) {
            return 0.0;
        }
        t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
        if (t > T_MAX) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
    // === GENERATED sdf_soft_shadow END ===
}

// A2: 5-tap ambient occlusion. March the surface normal `n` from `p`, summing the
// `(h - d)` field-deficit (how much closer the surface is than the unoccluded clearance
// would be) weighted by AO_FALLOFF^i. `field_distance` is the FROZEN field. Returns an
// occlusion factor in [0,1] (1 = unoccluded).
float sdf_ao(float3 p, float3 n) {
    float occ = 0.0;
    [unroll]
    for (uint i = 1u; i <= 5u; ++i) {
        float h = (float)i * AO_STEP;
        float d = field_distance(p + n * h);
        occ += (h - d) * pow(AO_FALLOFF, (float)i);
    }
    return clamp(1.0 - AO_STRENGTH * occ, 0.0, 1.0);
}

// --- PBR MVP-2: G-buffer attribute packing + material attribution (CONSUMER-side) ---
//
// All three helpers below are strict CONSUMERS of the surface hit; NONE touches the
// FROZEN field functions (`sdf`/`smin`/`combine`/...). `pick_material_id` re-evaluates
// the per-edit primitive distance via the FROZEN `load_edit`/`edit_distance` purely to
// ATTRIBUTE the hit to the nearest edit — a read-only re-evaluation, exactly the
// hard-union nearest-surface rule (PBR plan Decision 4 / OQ-5). The field is untouched.

// Octahedral-encode a unit normal `n` into [0,1]^2 (Cigolle et al. / Meyer survey). The
// resolve decodes it via `oct_decode`. ~16-bit angular precision when stored in RG16, but
// stored here in the RG channels of the RGBA8 gNormal target (MVP-2 keeps the existing
// RGBA8 G-buffer; the BA channels carry the material id). Plain ops; off the frozen field.
float2 oct_encode(float3 n) {
    n /= (abs(n.x) + abs(n.y) + abs(n.z));
    float2 e = n.xy;
    if (n.z < 0.0) {
        e = (1.0 - abs(e.yx)) * float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 : -1.0);
    }
    return e * 0.5 + 0.5; // [-1,1] -> [0,1] for the UNORM store
}

// Pack a 16-bit material id into the B + A channels of an RGBA8 texel: low byte -> B,
// high byte -> A, each as a normalized [0,1] UNORM value (byte/255). The resolve
// reconstructs `id = round(b*255) | (round(a*255) << 8)`. 16 bits = 65 536 materials.
float2 pack_material_id_ba(uint id) {
    uint lo = id & 0xFFu;
    uint hi = (id >> 8) & 0xFFu;
    return float2((float)lo / 255.0, (float)hi / 255.0);
}

// ATTRIBUTE an SDF hit point `p` to the nearest edit's material id via an argmin over the
// edit list, reusing the FROZEN `load_edit` + `edit_distance`. This is the hard-union
// nearest-surface rule (the material the surface is closest to). The id is carried in the
// per-edit `center.w` FREE LANE (PBR plan Decision 4): `asuint(Buf[base+3])`, read OUTSIDE
// the field eval. Gated to SDF (mask==1) hits by the caller. The field is NOT touched.
uint pick_material_id(float3 p) {
    uint n = min(Buf[0], MAX_SDF_EDITS);
    if (n == 0u) {
        return DEFAULT_MATERIAL_ID;
    }
    float best_d = FAR;
    uint best_id = DEFAULT_MATERIAL_ID;
    [loop]
    for (uint i = 0u; i < n; ++i) {
        Edit e = load_edit(i);                 // FROZEN decode (skips word 3 = center.w)
        float d = abs(edit_distance(e, p));    // FROZEN per-primitive distance
        if (d < best_d) {
            best_d = d;
            // The material id lives in the edit's center.w free lane (bit-cast u32).
            uint base = HEADER_BASE + i * SDF_EDIT_WORDS;
            best_id = asuint(Buf[base + 3u]);
        }
    }
    return best_id;
}

// --- SDF brick-atlas M1: the EMPTY-SPACE-SKIP helpers (mirror boyko_sdf_math::brick) ---

// The BrickClass discriminants (mirror `boyko_sdf_math::BrickClass`). Only EMPTY_OUTSIDE
// is acted on by the skip; EMPTY_INSIDE + SURFACE fold the analytic field.
static const uint BRICK_EMPTY_OUTSIDE = 0u;
static const uint BRICK_EMPTY_INSIDE  = 1u;
static const uint BRICK_SURFACE       = 2u;

// The minimum per-step progress a brick-exit makes (world units) — the progress guarantee
// (mirror `boyko_sdf_math::brick::BRICK_EXIT_EPS`). A face-parallel / boundary-grazing ray
// would compute a zero exit and stall; clamping to this forces the march forward.
static const float BRICK_EXIT_EPS = 1.0e-4;

// A sentinel returned by `brick_cell_class` when `p` is OUTSIDE the bounded grid: the
// marcher then folds the analytic field (the grid is a near-field accelerator, never a
// correctness boundary). Distinct from the three real classes.
static const uint BRICK_OUTSIDE_GRID = 0xFFFFFFFFu;

// The ray-AABB SLAB exit distance for the brick at `cell_min` of size `pc.brick_world`,
// from `p` along `rd` (the empty-skip step length, mirror `dist_to_brick_exit`). Returns
// the additive `t` step to leave the brick AABB; clamped UP to BRICK_EXIT_EPS so a
// degenerate ray still advances (the progress guarantee — INVIOLABLE). Only called for an
// EMPTY_OUTSIDE brick, which has provably no surface within band_half, so the step to the
// brick boundary never over-steps a surface (the empty-skip soundness contract).
// M4: the brick world edge `bw` is a PARAMETER so the marcher can pass this level's
// `m2_levels[L].origin_brick_world.w` (the per-level cell size). At N=1 the caller passes
// `pc.brick_world` — byte-identical to the pre-M4 M1 empty-skip.
// === GENERATED dist_to_brick_exit BEGIN ===
// MACHINE-GENERATED by `boyko_shaderdsl::emit::emit_hlsl_dist_to_brick_exit` (Increment 1
// of the in-house Rust shader eDSL — the first CONTROL-FLOW leaf). Authored ONCE generic
// over the value axis `S: FieldScalar` + the control-flow axis `C: Cf`
// (`boyko_shaderdsl::brick::dist_to_brick_exit_body`), whose `<f32, EvalCf>`
// instantiation is the CPU oracle. The host `boyko_sdf_math::brick::dist_to_brick_exit`
// stays HAND-WRITTEN (firewall option B) and is NOT byte-identical on an all-axes-
// degenerate ray (1e30 vs EPS) — that input is marcher-UNREACHABLE (a normalized rd
// cannot have all three |components| <= 1e-4). DO NOT HAND-EDIT: re-run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice between these sentinels (the `sdf_field_edsl_sync` test guards drift; the
// cmp-`.spv` gate proves it re-DXCs byte-identical to the committed `.comp.spv`).
float dist_to_brick_exit(float3 p, float3 rd, float3 cell_min, float bw) {
    float exit = 1.0e30;
    [unroll]
    for (uint a = 0u; a < 3u; ++a) {
        float t0 = rd[a];
        float t1 = cell_min[a];
        float t2 = t1 + bw;
        if (abs(t0) <= BRICK_EXIT_EPS) {
            continue;
        }
        float t3 = 1.0 / t0;
        float t4 = (t1 - p[a]) * t3;
        float t5 = (t2 - p[a]) * t3;
        float t6 = max(t4, t5);
        exit = min(exit, t6);
    }
    return (exit < BRICK_EXIT_EPS) ? BRICK_EXIT_EPS : exit;
}
// === GENERATED dist_to_brick_exit END ===

// Reads the pointer-grid cell class containing world point `p`, returning its BrickClass
// (and `cell_min` via the out param) or BRICK_OUTSIDE_GRID when `p` is outside the bounded
// grid. Mirrors `boyko_sdf_math::brick`'s host_brick_cell index + bounds check exactly.
// M4: the grid geometry (`origin`/`brick_world`/`dims`) + the level's `PointerGrid` are PARAMETERS so
// the marcher's branch-ladder can pass this level's `m2_levels[L]` geometry + `PointerGrid{L}`. At N=1
// the caller passes the push grid geometry (`pc.grid_*`) + level-0's `PointerGrid` — byte-identical to
// the pre-M4 M1 empty-skip (which read those same push fields + binding 9 directly).
// === GENERATED brick_cell_class BEGIN ===
// Machine-generated by `boyko_shaderdsl::emit::emit_hlsl_brick_cell_class` (Increment 3 —
// the second CONTROL-FLOW leaf: early returns + a `StructuredBuffer<uint>` load + an `out
// float3` param + `uint` index math). Pinned by `sdf_field_edsl_sync`'s
// `brick_cell_class_matches_edsl_emit` (the durable text gate) + the one-shot cmp-`.spv`
// (re-DXC byte-identical to the committed `.comp.spv`). DO NOT HAND-EDIT — re-run
//     cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. The three call sites (PointerGrid/1/2) are UNTOUCHED.
uint brick_cell_class(StructuredBuffer<uint> grid, float3 origin, float bw, uint3 dims,
                      float3 p, out float3 cell_min) {
    float3 rel = (p - origin) / bw;
    cell_min = origin;
    if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) {
        return BRICK_OUTSIDE_GRID;
    }
    uint ix = (uint)rel.x;
    uint iy = (uint)rel.y;
    uint iz = (uint)rel.z;
    if (ix >= dims.x || iy >= dims.y || iz >= dims.z) {
        return BRICK_OUTSIDE_GRID;
    }
    uint idx = ix + iy * dims.x + iz * dims.x * dims.y;
    cell_min = origin + float3(ix, iy, iz) * bw;
    return grid[idx];
}
// === GENERATED brick_cell_class END ===

// --- SDF brick-atlas M2: the trilinear+JCGT-cubic SURFACE-brick path (mirror boyko_sdf_math::brick) ---
//
// Inside a SURFACE brick the marcher replaces the analytic fold with: a 3D-DDA march through the
// brick's interior voxel cells, each cell's 8 corners forming the JCGT-2022 trilinear cubic
// (`m2_jcgt_cubic_coeffs`), the near root found by the Marmitt iterative root-finder
// (`m2_marmitt_root`, FMA-only, no transcendentals, no 1/c3) — the EXACT ray↔trilinear-isosurface
// crossing. An ANALYTIC-RESIDUAL FALLBACK then validates the candidate against the exact field
// (`field_distance`): a |sdf| within CREASE_EPS accepts the cubic hit, else a few analytic refine
// steps decide it (the exact-CSG guarantee). The HLSL below mirrors `brick_cubic_hit` /
// `jcgt_cubic_coeffs` / `marmitt_root` / `atlas_uvw` / `decode_snorm8` bit-for-bit (the offscreen
// golden compares the GPU hit against `brick_cubic_hit` and the analytic field within tolerance).

// M2 brick geometry (mirror `boyko_sdf_math::brick` + `boyko_rhi_vulkan::compute`). Read from the
// b5 UBO M2 block so a host-side grid retune needs no shader edit; the brick-edge consts are
// compile-time pins (the apron'd 10³ tile shape is fixed by the oracle).
static const uint  M2_BRICK_INTERIOR = 8u;     // BRICK_INTERIOR
static const uint  M2_BRICK_ALLOC    = 10u;    // BRICK_ALLOC (interior + 1-voxel apron each face)
static const uint  M2_GRID_DIM       = 4u;     // M2_GRID_DIM — per-axis cell count (M5 toroidal mask DIM)
static const float M2_APRON          = 1.0;    // APRON (one voxel)
static const float M2_ATLAS_BIAS     = 0.0;    // ATLAS_SAMPLE_BIAS (golden-locked to 0)
static const float M2_CUBIC_ROOT_EPS = 1.0e-6; // CUBIC_ROOT_EPS (root residual / bracket tol)
static const uint  M2_MARMITT_ITERS  = 8u;     // MARMITT_ITERS (regula-falsi cap)
static const uint  M2_MAX_CELLS      = 30u;    // 3 * BRICK_ALLOC — the longest 3D-DDA path
// The analytic-residual crease band (world units): a cubic candidate whose |sdf| is within this is
// accepted as the surface; beyond it (a CSG crease / brick-rounding divergence) the analytic refine
// decides. `0.0192` ~ the brick's δ_tri + δ_quant world slack (the M0 EPSILON_Q dominance budget).
static const float M2_CREASE_EPS     = 0.0192;
// The analytic refine: a handful of SIGNED, under-relaxed sphere-trace steps from the cubic
// candidate, used to settle a crease/divergence onto the EXACT field from EITHER side (the
// exact-CSG fallback). The signed step pulls an inside candidate (`d < 0`, the EPSILON_Q down-bias)
// BACK toward the surface; a forward-only step could not.
static const uint  M2_REFINE_ITERS   = 8u;
// The under-relaxation factor of the signed refine step (`rt += M2_REFINE_RELAX * d`). `rt += d` is
// the exact unit-gradient SDF Newton step; under-relaxing damps overshoot at a CSG crease. Mirrors
// the host `M2_REFINE_RELAX` bit-for-bit.
static const float M2_REFINE_RELAX   = 0.8;

// Decodes one R8_SNORM code (a normalized float in [-1,1] from the hardware texel fetch) to a world
// distance. The NEAREST `SampleLevel` returns the hardware-decoded SNORM value at the EXACT texel
// (point sampling, no interpolation), so `n` is the decode of the stored i8 byte (the -128→-1
// asymmetry is applied by the Vulkan R8_SNORM rule the host `decode_snorm8` mirrors). Multiply by
// band_half (mirror `decode_snorm8` post-normalization).
//
// SINGLE-SOURCED (A2): this body is MACHINE-GENERATED by `boyko_shaderdsl::brick::snorm_scale` (the
// world-scale step of `decode_snorm8`), whose `f32` Eval backend IS the host
// `boyko_sdf_math::brick::decode_snorm8`. Only the SCALE is shader code — the byte→normalized-float
// step (`q == i8::MIN ? -1 : q/127`) is performed by the fixed-function R8_SNORM sampler in HARDWARE,
// so it never appears here. Do NOT hand-edit between the sentinels: re-run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. The `sdf_field_edsl_sync` test pins this body to the generator; a hand-edit fails CI.
// === GENERATED decode_snorm8 BEGIN ===
float m2_decode(float n, float band_half) {
    float t0 = n * band_half;
    return t0;
}
// === GENERATED decode_snorm8 END ===

// Fetches the atlas at apron'd-grid corner `(cx, cy, cz)` of tile origin `tile_org` (atlas-voxel
// units), returning the decoded world distance.
//
// GPU-BUG FIX (BUG-M2-GPU-1, M2 corner-fetch): the per-corner cubic MUST read the SAME decoded i8 the
// host's `brick_cubic_hit` reads via `decode_snorm8(brick[exact_index])`. Two issues collapsed the
// M2 branch on-device (RTX 3060); both are fixed here.
//
//   (1) ROOT CAUSE — the atlas format constant `VK_FORMAT_R8_SNORM` was mis-set to 9, which is
//       actually `VK_FORMAT_R8_UNORM` (the real `R8_SNORM` is 10). So the atlas image+view were
//       created UNORM and the sampler decoded byte 127 as `127/255 = 0.498` instead of the signed
//       `127/127 = 1.0`. Every corner was ~2× too small with the wrong sign, the cubic never changed
//       sign, `m2_surface_hit` returned false for every pixel, and the branch was DEAD (gViewT
//       bit-identical to the analytic marcher). Fixed in `boyko_rhi::Format::R8Snorm` (= 10) and
//       `VK_FORMAT_R8_SNORM` (= 10). On-device: corner s111 then decoded 1.0 (correct SNORM).
//   (2) the corner fetch is a POINT sample through the COMBINED image+sampler descriptor (the RHI has
//       no standalone Sampler kind; binding 10 = combined). A NEAREST sampler (set in
//       `brick_atlas.rs`) + `SampleLevel` at the texel CENTER uvw `(texel + 0.5) / atlas_dim` returns
//       the EXACT texel's decoded snorm with ZERO interpolation — bit-matching the host's
//       integer-index `decode_snorm8(brick[i])` while exercising the descriptor as a genuine combined
//       image+sampler (a `.Load` texelFetch on the texture half of a combined descriptor is the wrong
//       access path). The corner texel is the integer atlas-voxel `(tile_org + corner)`; `tile_org`
//       is integral (tile * BRICK_ALLOC) and the clamp keeps `corner` in `[0, BRICK_ALLOC-1]`, so the
//       texel is always in-bounds and the ClampToEdge sampler never wraps. `inv_atlas == 1.0 /
//       atlas_dim` maps the integer texel to its normalized center uvw.
// M4: the atlas + sampler are RESOURCE PARAMETERS so the marcher's branch-ladder can pass this level's
// `BrickAtlas{N}`/`BrickSampler{N}` (HLSL supports resource params; it cannot dynamically index the N
// separate resources, so the per-level dispatch is the static ladder in the marcher). At N=1 the ladder
// passes `BrickAtlas`/`BrickSampler` (the level-0 / M2 resources), so this is byte-identical to M2.
float m2_corner(Texture3D<float> atlas, SamplerState atlas_smp,
                float3 tile_org, uint cx, uint cy, uint cz, float inv_atlas, float band_half) {
    float3 uvw = (float3(tile_org.x + (float)cx,
                         tile_org.y + (float)cy,
                         tile_org.z + (float)cz) + 0.5) * inv_atlas;
    float n = atlas.SampleLevel(atlas_smp, uvw, 0.0).r;
    return m2_decode(n, band_half);
}

// Floors an apron'd-grid coordinate to a low cell index with room for the +1 neighbour
// (clamped into 0..=BRICK_ALLOC-2). Mirror `boyko_sdf_math::brick::clamp_index`.
uint m2_clamp_index(float g) {
    if (g <= 0.0) {
        return 0u;
    }
    uint i = (uint)g;
    return (i >= M2_BRICK_ALLOC - 1u) ? (M2_BRICK_ALLOC - 2u) : i;
}

// Evaluates the JCGT cubic c3·t³ + c2·t² + c1·t + c0 (Horner, FMA-friendly) — mirror `cubic_eval`.
//
// SINGLE-SOURCED (A3): this body is MACHINE-GENERATED by `boyko_shaderdsl::brick::cubic_eval`, whose
// `f32` Eval backend IS the host `boyko_sdf_math::brick::cubic_eval`. The coefficient float4 is read
// as `c.x..c.w` (c.x = c0 ... c.w = c3). Do NOT hand-edit between the sentinels: re-run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. The `sdf_field_edsl_sync` test pins this body to the generator; a hand-edit fails CI.
// === GENERATED m2_cubic_eval BEGIN ===
float m2_cubic_eval(float4 c, float t) {
    float t0 = c.w * t;
    float t1 = t0 + c.z;
    float t2 = t1 * t;
    float t3 = t2 + c.y;
    float t4 = t3 * t;
    float t5 = t4 + c.x;
    return t5;
}
// === GENERATED m2_cubic_eval END ===

// Forms the JCGT-2022 cubic [c0,c1,c2,c3] whose root is the ray↔trilinear-isosurface crossing in
// ONE voxel cell. `s` holds the 8 corners in the `s_ijk ↔ x + 2y + 4z` order (x fastest):
// s[0]=s000,1=s100,2=s010,3=s110,4=s001,5=s101,6=s011,7=s111 — the SAME order `m2_corner` fetches
// and the trilinear blend uses. `a` = ro_local, `b` = rd_local in the cell's [0,1]³ frame. Mirror
// `boyko_sdf_math::brick::jcgt_cubic_coeffs` (the k-basis + the FMA chain must NOT be reordered).
//
// SINGLE-SOURCED (A3): this body is MACHINE-GENERATED by `boyko_shaderdsl::brick::jcgt_cubic_coeffs`,
// whose `f32` Eval backend IS the host `boyko_sdf_math::brick::jcgt_cubic_coeffs`. The corners are read
// as `s[0]..s[7]`, the ray frame as `a.x/a.y/a.z` / `b.x/b.y/b.z`, and the result is the `float4(c0, c1,
// c2, c3)` construct. Do NOT hand-edit between the sentinels: re-run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. The `sdf_field_edsl_sync` test pins this body to the generator; a hand-edit fails CI.
// === GENERATED m2_jcgt_cubic_coeffs BEGIN ===
float4 m2_jcgt_cubic_coeffs(float s[8], float3 a, float3 b) {
    float t0 = s[1] - s[0];
    float t1 = s[2] - s[0];
    float t2 = s[4] - s[0];
    float t3 = s[3] - s[1];
    float t4 = t3 - s[2];
    float t5 = t4 + s[0];
    float t6 = s[6] - s[2];
    float t7 = t6 - s[4];
    float t8 = t7 + s[0];
    float t9 = s[5] - s[1];
    float t10 = t9 - s[4];
    float t11 = t10 + s[0];
    float t12 = s[7] - s[3];
    float t13 = t12 - s[5];
    float t14 = t13 - s[6];
    float t15 = t14 + s[1];
    float t16 = t15 + s[2];
    float t17 = t16 + s[4];
    float t18 = t17 - s[0];
    float t19 = t0 * a.x;
    float t20 = s[0] + t19;
    float t21 = t1 * a.y;
    float t22 = t20 + t21;
    float t23 = t2 * a.z;
    float t24 = t22 + t23;
    float t25 = t5 * a.x;
    float t26 = t25 * a.y;
    float t27 = t24 + t26;
    float t28 = t8 * a.y;
    float t29 = t28 * a.z;
    float t30 = t27 + t29;
    float t31 = t11 * a.z;
    float t32 = t31 * a.x;
    float t33 = t30 + t32;
    float t34 = t18 * a.x;
    float t35 = t34 * a.y;
    float t36 = t35 * a.z;
    float t37 = t33 + t36;
    float t38 = t0 * b.x;
    float t39 = t1 * b.y;
    float t40 = t38 + t39;
    float t41 = t2 * b.z;
    float t42 = t40 + t41;
    float t43 = a.x * b.y;
    float t44 = a.y * b.x;
    float t45 = t43 + t44;
    float t46 = t5 * t45;
    float t47 = t42 + t46;
    float t48 = a.y * b.z;
    float t49 = a.z * b.y;
    float t50 = t48 + t49;
    float t51 = t8 * t50;
    float t52 = t47 + t51;
    float t53 = a.z * b.x;
    float t54 = a.x * b.z;
    float t55 = t53 + t54;
    float t56 = t11 * t55;
    float t57 = t52 + t56;
    float t58 = a.x * a.y;
    float t59 = t58 * b.z;
    float t60 = a.x * b.y;
    float t61 = t60 * a.z;
    float t62 = t59 + t61;
    float t63 = b.x * a.y;
    float t64 = t63 * a.z;
    float t65 = t62 + t64;
    float t66 = t18 * t65;
    float t67 = t57 + t66;
    float t68 = t5 * b.x;
    float t69 = t68 * b.y;
    float t70 = t8 * b.y;
    float t71 = t70 * b.z;
    float t72 = t69 + t71;
    float t73 = t11 * b.z;
    float t74 = t73 * b.x;
    float t75 = t72 + t74;
    float t76 = a.x * b.y;
    float t77 = t76 * b.z;
    float t78 = b.x * a.y;
    float t79 = t78 * b.z;
    float t80 = t77 + t79;
    float t81 = b.x * b.y;
    float t82 = t81 * a.z;
    float t83 = t80 + t82;
    float t84 = t18 * t83;
    float t85 = t75 + t84;
    float t86 = t18 * b.x;
    float t87 = t86 * b.y;
    float t88 = t87 * b.z;
    return float4(t37, t67, t85, t88);
}
// === GENERATED m2_jcgt_cubic_coeffs END ===

// Regula-falsi (false position) refinement of a sign-bracketed root in [lo, hi] — mirror
// `boyko_sdf_math::brick::regula_falsi`. FMA-only, bounded iterations.
//
// MACHINE-GENERATED by boyko_shaderdsl::emit::emit_hlsl_m2_regula_falsi() (Increment 4a —
// the first genuine RUNTIME `[loop]` / OpLoop). Do NOT hand-edit between the sentinels: run
//   cargo run -p boyko_shaderdsl --features emit --bin emit_field
// and re-splice. The `m2_regula_falsi_matches_edsl_emit` test pins this body to the
// generator; a hand-edit fails CI. (The 2 call sites in m2_marmitt_root stay UNCHANGED.)
// === GENERATED m2_regula_falsi BEGIN ===
float m2_regula_falsi(float4 c, float lo, float hi, float f_lo, float f_hi) {
    float mid = lo;
    [loop]
    for (uint i = 0u; i < M2_MARMITT_ITERS; ++i) {
        float denom = f_hi - f_lo;
        mid = (abs(denom) > 1.0e-30) ? (lo - f_lo * (hi - lo) / denom) : (0.5 * (lo + hi));
        float f_mid = m2_cubic_eval(c, mid);
        if (abs(f_mid) <= M2_CUBIC_ROOT_EPS || hi - lo <= M2_CUBIC_ROOT_EPS) {
            return mid;
        }
        if (f_lo * f_mid <= 0.0) {
            hi = mid;
            f_hi = f_mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    return mid;
}
// === GENERATED m2_regula_falsi END ===

// The Marmitt iterative root of the JCGT cubic in [t0, t1] — the FIRST sign crossing, or a negative
// sentinel (-1) when the cubic does not change sign. FMA-only, NO transcendentals, NO 1/c3 (robust
// to c3 → 0). Mirror `boyko_sdf_math::brick::marmitt_root` (returns -1 instead of None).
float m2_marmitt_root(float4 c, float t0, float t1) {
    if (t1 <= t0) {
        return -1.0;
    }
    // The interior extrema: roots of the derivative quadratic 3·c3·t² + 2·c2·t + c1, solved WITHOUT
    // dividing by c3 (a near-zero leading term collapses to the linear/constant case → no split).
    float qa = 3.0 * c.w;
    float qb = 2.0 * c.z;
    float qc = c.y;

    float e0 = t1;
    float e1 = t1;
    bool have0 = false;
    bool have1 = false;

    float disc = qb * qb - 4.0 * qa * qc;
    if (abs(qa) > 1.0e-30 && disc > 0.0) {
        float sq = sqrt(disc);
        float q = -0.5 * (qb + (qb >= 0.0 ? 1.0 : -1.0) * sq);
        float r0 = q / qa;
        float r1 = (abs(q) > 1.0e-30) ? (qc / q) : r0;
        if (r0 > r1) { float tmp = r0; r0 = r1; r1 = tmp; }
        if (r0 > t0 && r0 < t1) { e0 = r0; have0 = true; }
        if (r1 > t0 && r1 < t1) {
            if (have0) { e1 = r1; have1 = true; }
            else { e0 = r1; have0 = true; }
        }
    }

    // March the monotone sub-intervals left→right; refine the FIRST sign bracket.
    float lo = t0;
    float f_lo = m2_cubic_eval(c, lo);
    float splits[3];
    splits[0] = have0 ? e0 : t1;
    splits[1] = have1 ? e1 : t1;
    splits[2] = t1;
    [unroll]
    for (uint i = 0u; i < 3u; ++i) {
        float hi = splits[i];
        if (hi <= lo) {
            continue;
        }
        float f_hi = m2_cubic_eval(c, hi);
        if (f_lo == 0.0) {
            return lo;
        }
        if (f_lo * f_hi <= 0.0) {
            return m2_regula_falsi(c, lo, hi, f_lo, f_hi);
        }
        lo = hi;
        f_lo = f_hi;
        if (hi >= t1) {
            break;
        }
    }
    return -1.0;
}

// Clips the ray `p + rd·t` to the M2 brick at `cell_min` of size M2_BRICK_WORLD, returning the
// [t_enter, t_exit] span (in world `t`, measured from `p`). `t_exit < t_enter` means the ray misses
// the brick AABB. Standard slab intersection.
bool m2_brick_span(float3 p, float3 rd, float3 cell_min, float brick_world, out float t_enter, out float t_exit) {
    float tmin = 0.0;          // never march behind the current march point
    float tmax = 1.0e30;
    [unroll]
    for (uint a = 0u; a < 3u; ++a) {
        float lo = cell_min[a];
        float hi = lo + brick_world;
        if (abs(rd[a]) <= 1.0e-20) {
            // Parallel to this slab: a miss only if the origin is outside it.
            if (p[a] < lo || p[a] > hi) {
                t_enter = 1.0; t_exit = 0.0; // empty span
                return false;
            }
            continue;
        }
        float inv = 1.0 / rd[a];
        float t1 = (lo - p[a]) * inv;
        float t2 = (hi - p[a]) * inv;
        if (t1 > t2) { float tmp = t1; t1 = t2; t2 = tmp; }
        tmin = max(tmin, t1);
        tmax = min(tmax, t2);
    }
    t_enter = tmin;
    t_exit = tmax;
    return tmax > tmin;
}

// Marches `p + rd·t` through SURFACE brick `cell_min`'s interior voxel cells (3D-DDA), forming the
// JCGT cubic at the first cell whose 8 corners bracket a sign change and solving it for the in-cell
// crossing. Returns the world-space `t` of the FIRST hit (>= 0), or a negative sentinel (-1) when
// the ray clears the brick without crossing. `ro_v`/`rd_v` are the ray in INTERIOR-voxel units
// (world → voxel: (world - cell_min) / voxel_size). Mirror `boyko_sdf_math::brick::brick_cubic_hit`
// (the DDA + the cubic fetch order + the per-cell local-t solve are bit-for-bit the CPU oracle).
float m2_brick_cubic_hit(Texture3D<float> atlas, SamplerState atlas_smp,
                         float3 ro_v, float3 rd_v, float t_enter, float t_exit,
                         float3 tile_org, float inv_atlas, float band_half) {
    if (t_exit <= t_enter) {
        return -1.0;
    }
    const uint W = M2_BRICK_ALLOC;
    float t = t_enter;
    int   cell[3];
    int   step[3];
    float t_next[3];
    float t_delta[3];

    [unroll]
    for (uint axis = 0u; axis < 3u; ++axis) {
        // The apron'd-grid coordinate at entry (the +APRON-0.5 shift maps interior coords to the
        // apron'd grid, the SAME shift `atlas_uvw` / the corner fetch use).
        float g_entry = ro_v[axis] + rd_v[axis] * t + M2_APRON - 0.5 + M2_ATLAS_BIAS;
        int c0 = (int)m2_clamp_index(g_entry);
        cell[axis] = c0;
        if (rd_v[axis] > 0.0) {
            step[axis] = 1;
            float boundary = (float)(c0 + 1);
            t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
            t_delta[axis] = 1.0 / rd_v[axis];
        } else if (rd_v[axis] < 0.0) {
            step[axis] = -1;
            float boundary = (float)c0;
            t_next[axis] = t + (boundary - g_entry) / rd_v[axis];
            t_delta[axis] = -1.0 / rd_v[axis];
        } else {
            step[axis] = 0;
            t_next[axis] = 1.0e30;
            t_delta[axis] = 1.0e30;
        }
    }

    [loop]
    for (uint iter = 0u; iter < M2_MAX_CELLS; ++iter) {
        // The cell's low corner clamped so the +1 neighbour is in-bounds.
        uint cx = min((uint)max(cell[0], 0), W - 2u);
        uint cy = min((uint)max(cell[1], 0), W - 2u);
        uint cz = min((uint)max(cell[2], 0), W - 2u);

        // Fetch the 8 decoded corners in the s_ijk ↔ x + 2y + 4z order (NEAREST point-sample at the
        // texel center — the corner-fetch fix; the SAME decoded i8 the host `brick_cubic_hit` reads).
        float s[8];
        s[0] = m2_corner(atlas, atlas_smp, tile_org, cx,      cy,      cz,      inv_atlas, band_half); // s000
        s[1] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy,      cz,      inv_atlas, band_half); // s100
        s[2] = m2_corner(atlas, atlas_smp, tile_org, cx,      cy + 1u, cz,      inv_atlas, band_half); // s010
        s[3] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz,      inv_atlas, band_half); // s110
        s[4] = m2_corner(atlas, atlas_smp, tile_org, cx,      cy,      cz + 1u, inv_atlas, band_half); // s001
        s[5] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy,      cz + 1u, inv_atlas, band_half); // s101
        s[6] = m2_corner(atlas, atlas_smp, tile_org, cx,      cy + 1u, cz + 1u, inv_atlas, band_half); // s011
        s[7] = m2_corner(atlas, atlas_smp, tile_org, cx + 1u, cy + 1u, cz + 1u, inv_atlas, band_half); // s111

        // The t-span of THIS cell along the ray (clamped to the brick span).
        float t_cell_exit = min(min(min(t_next[0], t_next[1]), t_next[2]), t_exit);
        float seg_lo = max(t, t_enter);
        float seg_hi = min(t_cell_exit, t_exit);

        if (seg_hi > seg_lo) {
            // The ray in the cell's LOCAL [0,1]³ frame: the apron'd-grid coordinate minus the cell
            // low index gives the in-cell fraction; the direction is unchanged (a pure translation).
            float3 lo_g = float3(
                ro_v[0] + rd_v[0] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cx,
                ro_v[1] + rd_v[1] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cy,
                ro_v[2] + rd_v[2] * seg_lo + M2_APRON - 0.5 + M2_ATLAS_BIAS - (float)cz);
            float4 coeffs = m2_jcgt_cubic_coeffs(s, lo_g, rd_v);
            float local_t = m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo);
            if (local_t >= 0.0) {
                return seg_lo + local_t;
            }
        }

        // Advance the DDA to the next cell boundary; stop once past the brick exit.
        if (t_cell_exit >= t_exit) {
            break;
        }
        uint axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u
                  : ((t_next[1] <= t_next[2]) ? 1u : 2u);
        t = t_next[axis];
        cell[axis] += step[axis];
        t_next[axis] += t_delta[axis];
        if (step[axis] == 0 || cell[axis] < 0 || (uint)cell[axis] >= W - 1u) {
            break;
        }
    }
    return -1.0;
}

// The M2 SURFACE-brick step: given the march point `p`'s SURFACE M2 cell, sample the atlas + run
// the JCGT cubic for the EXACT crossing, then VALIDATE it analytically (the exact-CSG fallback).
// Returns the accepted world `t` of the hit (>= 0) via `out hit_t`, and `true`/`false` for
// hit/no-crossing. A cubic crossing is accepted ONLY when the SIGNED analytic refine CONVERGES onto
// the EXACT field (`abs(d) < EPS`); a grazing silhouette point or a stalled hard crease returns
// `false` → the caller's analytic fold (matching the OFF path exactly, no silhouette rim). The normal
// + shade stay ANALYTIC (decided by the caller's `sdf_normal`), so the surface is always validated
// analytically (C1). `ro`/`rd` are the WORLD ray; `t_world` is the current march `t` (p = ro+rd·t).
// M4: the per-level grid block (`lvl`) + the level's atlas/sampler are PARAMETERS (the marcher's
// branch-ladder passes `m2_levels[L]` + `BrickAtlas{L}`/`BrickSampler{L}`). At N=1 the caller passes
// `m2_levels[0]` + the M2 resources, so this is byte-identical to the pre-M4 M2 surface-hit.
bool m2_surface_hit(M4Level lvl, Texture3D<float> atlas, SamplerState atlas_smp,
                    float3 ro, float3 rd, float t_world, out float hit_t) {
    hit_t = t_world;
    float3 origin = lvl.origin_brick_world.xyz;
    float brick_world = lvl.origin_brick_world.w;
    uint3 dims = (uint3)lvl.dims_atlas_dim.xyz;
    float band_half = lvl.band_voxel_inv_atlas.x;
    float voxel_size = lvl.band_voxel_inv_atlas.y;
    float inv_atlas = lvl.band_voxel_inv_atlas.z;

    float3 p = ro + rd * t_world;
    // The M2 tile containing `p`. Outside the bounded M2 grid → no atlas tile (the caller folds the
    // analytic field). Test the float directly so a negative coord is caught before the uint cast.
    float3 rel = (p - origin) / brick_world;
    if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) {
        return false;
    }
    uint tx = (uint)rel.x;
    uint ty = (uint)rel.y;
    uint tz = (uint)rel.z;
    if (tx >= dims.x || ty >= dims.y || tz >= dims.z) {
        return false;
    }
    float3 cell_min = origin + float3((float)tx, (float)ty, (float)tz) * brick_world;
    // M5 (Decision 5): the tile is stored at its TOROIDAL slot, decoupling the world box-cell from
    // the atlas tile so a camera-follow scroll re-bakes only the revealed slab. The slot is
    // `(origin_cell + box) mod M2_GRID_DIM`, where `origin_cell = round(origin / brick_world)` is the
    // grid's integer cell snap (recomputed from the UBO `origin`/`brick_world` — NO new UBO field, so
    // the OFF UBO byte-identity is untouched). `(uint)(... + DIM) % DIM` reproduces the host's
    // `rem_euclid(DIM)` for the small non-negative `origin_cell + box` range these grids occupy; the
    // `+ DIM` bias keeps the small NEGATIVE origin_cell case (camera below the origin) on the positive
    // side of HLSL's truncating `%`. This is a stable per-grid PERMUTATION of the M4 box→box map; at a
    // grid where `origin_cell ≡ 0 (mod DIM)` it reduces to `tile * BRICK_ALLOC` (the OFF reduction).
    int3 origin_cell = (int3)round(origin / brick_world);
    int3 world_cell = origin_cell + int3((int)tx, (int)ty, (int)tz);
    // rem_euclid(DIM): bias by a multiple of DIM large enough to clear the negative magnitudes in play
    // (the grids are camera-local, |origin_cell| stays small), then truncating `%`.
    const int WRAP_BIAS = (int)(M2_GRID_DIM * 1024u); // a DIM-multiple swamping any camera-local cell
    uint3 slot = (uint3)((world_cell + WRAP_BIAS) % (int)M2_GRID_DIM);
    float3 tile_org = float3((float)(slot.x * M2_BRICK_ALLOC),
                             (float)(slot.y * M2_BRICK_ALLOC),
                             (float)(slot.z * M2_BRICK_ALLOC));

    // Clip the ray to the brick AABB, then convert to interior-voxel units (world → voxel:
    // (world - cell_min) / voxel_size). The DDA + cubic operate in voxel units.
    float t_enter, t_exit;
    if (!m2_brick_span(p, rd, cell_min, brick_world, t_enter, t_exit)) {
        return false;
    }
    float3 ro_v = (p - cell_min) / voxel_size;
    // rd is a unit world direction; in voxel units it scales by 1/voxel_size. Keep the world `t`
    // metric by dividing the direction (so cubic-local `t` is in WORLD units, matching the oracle's
    // ro/rd in interior-voxel units with the SAME world-t parameterization).
    float3 rd_v = rd / voxel_size;

    float local = m2_brick_cubic_hit(atlas, atlas_smp, ro_v, rd_v, t_enter, t_exit, tile_org, inv_atlas, band_half);
    if (local < 0.0) {
        return false; // the ray clears this brick without crossing — the caller marches on
    }

    // The candidate world `t` (local is measured from `p`, in world units). `local` is computed by
    // the NEAREST point-sampled cubic (`m2_corner` reads s10 directly), so the s10 sampler — and the
    // binding-10 combined descriptor — is GENUINELY referenced (no keep-alive hack needed).
    float cand_t = t_world + local;

    // ANALYTIC-RESIDUAL FALLBACK (the exact-CSG guarantee): a SIGNED, under-relaxed sphere-trace from
    // the cubic candidate onto the EXACT field decides BOTH whether this is a hit and where `hit_t`
    // lands, converging from EITHER side. The committed `hit_t` always satisfies `abs(sdf) < EPS`
    // (on-surface). The brick reconstruction's EPSILON_Q down-bias (scaled `2^L` per clip-map level)
    // lands the cubic candidate INSIDE the true surface (`d < 0`); committing the raw `cand_t` parked
    // the baked AO ~M2_CREASE_EPS deep and cratered (BUG-M2-CRATER). The signed step
    // `rt += M2_REFINE_RELAX * d` walks BACKWARD for `d < 0` (toward the surface) and forward for
    // `d > 0` — a forward-only step (`max(d, EPS)`) could never pull an inside hit back out. Accept on
    // `abs(d)` (not signed `d`) so an inside candidate is corrected, never committed. ONLY a
    // refine-CONVERGED candidate (`abs(d) < EPS`) is accepted; a grazing silhouette point (analytic
    // miss within the old crease band) or a hard crease where the refine stalls falls to `false` → the
    // caller's M1 analytic fold, which resolves the pixel EXACTLY as the OFF path. Removing the old
    // trailing crease-accept band (which accepted a NON-converged candidate within `M2_CREASE_EPS`)
    // erased the 1-2px silhouette rim where the brick hit but the analytic ray missed (BUG-M2-RIM).
    // Mirrors the host `host_m2_surface_hit` refine loop bit-for-bit.
    // The refine loop+tail (the analytic-residual signed sphere-trace) is MACHINE-GENERATED by
    // `boyko_shaderdsl::emit::emit_hlsl_m2_surface_hit_refine()` (Inc 4b.2) from the single-source
    // body `boyko_shaderdsl::surface::m2_surface_hit_refine_body` (reusing the proven runtime
    // `[loop]` + `brk` + `field_distance` `call1`, adding a real `OpTypeBool` return + an
    // `out float hit_t` write). The integer cell-addressing preamble above + the
    // `m2_brick_span`/`m2_brick_cubic_hit`/`select_level` call sites stay HAND-WRITTEN (framing b).
    // The host `host_m2_surface_hit` refine stays HAND-WRITTEN (firewall option B); the cmp-`.spv`
    // proves this span re-DXCs byte-identical (R1: `rt = rt + step` == the prior `rt += step`).
    // Re-emit + re-splice on drift: `cargo run -p boyko_shaderdsl --features emit --bin emit_field`.
    // === GENERATED m2_surface_hit_refine BEGIN ===
    float rt = cand_t;
    [loop]
    for (uint i = 0u; i < M2_REFINE_ITERS; ++i) {
        float d = field_distance(ro + rd * rt);
        if (abs(d) < EPS) {
            hit_t = rt;
            return true;
        }
        float step = M2_REFINE_RELAX * d;
        rt = rt + step;
        if (rt < 0.0 || rt > T_MAX) {
            break;
        }
    }
    return false;
    // === GENERATED m2_surface_hit_refine END ===
}

// SDF brick-atlas M4 (clip-map LOD): selects the FINEST enclosing clip-map level for world point `p`,
// or -1 when `p` is outside EVERY level (the caller then folds the analytic field, exactly as M2 does
// outside its single grid today). The levels are strictly concentric (level L's extent doubles), so
// the first-enclosing scan (level 0 = finest, nearest) returns the tightest LOD. The loop is bounded by
// `pc.brick_levels` (a runtime count <= BRICK_LEVELS); the `[unroll]` is compile-safe (BRICK_LEVELS).
//
// OFF/N=1 keystone: `pc.brick_levels == 1` loops ONCE over level 0 → returns 0 iff `p` is in the
// level-0 box (else -1), EXACTLY the M2 single-grid containment test. The marcher's branch-ladder then
// takes only the lvl==0 arm (the M2 resources), so `brick_levels == 1` is byte-identical to M2.
int select_level(float3 p) {
    [unroll]
    for (uint L = 0u; L < BRICK_LEVELS; ++L) {
        if (L >= pc.brick_levels) {
            break; // honor the runtime level count (a level >= brick_levels is not active)
        }
        float3 o = m2_levels[L].origin_brick_world.xyz;
        float bw = m2_levels[L].origin_brick_world.w;
        float3 hi = o + m2_levels[L].dims_atlas_dim.xyz * bw;
        if (all(p >= o) && all(p < hi)) {
            return (int)L;
        }
    }
    return -1; // outside all active levels → the caller folds the analytic field
}

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    if (idx >= count) {
        return;
    }

    // Resolve the runtime extent. At the golden invocation these equal 64, so every
    // downstream arithmetic reproduces the frozen ORTHO fixture.
    uint w = img_w();
    uint h = img_h();

    uint px = idx % w;
    uint py = idx / w;

    // PBR MVP-2 Phase 0: ray-gen extracted into the shared `generate_ray`
    // (`ray_gen.hlsli`). The arithmetic is CHARACTER-IDENTICAL to the prior inline
    // block (both ORTHO + PERSPECTIVE arms), so the hit point feeding the FROZEN field
    // is bit-unchanged (the GATE-1 tripwire + distance/depth golden prove it).
    float3 ro;
    float3 rd;
    generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);

    // I/O EDIT (1): the shared mesh depth for this pixel is now a SAMPLED-IMAGE fetch
    // of the rasterized D32_SFLOAT image (transitioned to SHADER_READ_ONLY) INSTEAD of
    // the packed `asfloat(Buf[DEPTH_BASE + idx])`. The value + interpretation are
    // identical: depth == clear (1.0) => no mesh; else the mesh's ray parameter is
    // depth * T_MAX (the ortho convention).
    float md = gDepth.Load(int3((int)px, (int)py, 0)).r;
    bool has_mesh = (md < DEPTH_CLEAR);          // strictly less than the far-plane clear
    float t_mesh = has_mesh ? (md * T_MAX) : 1.0e30; // a finite bound only when covered

    // --- Render P4b: the GATED coarse-cull prefix (Algorithm B). `coarse_enabled == 0`
    // leaves `t_seed = 0.0` and never touches `Tiles` — the OFF path is BYTE-IDENTICAL
    // to today's marcher (the 0%-gate). When enabled, read this pixel's tile bound:
    //   * EMPTY tile -> no SDF surface in the cone in front of the deepest mesh, but the
    //     pixel can still be MESH-covered -> composite mesh/background + return (D6); a
    //     blind background would erase the mesh -> golden regression.
    //   * else SEED the march at `near_t` (the proven-empty prefix, a conservative lower
    //     bound on every in-tile pixel's first hit -> never skips this pixel's surface).
    // The march loop + field eval below stay BYTE-UNTOUCHED. ---
    float t_seed = 0.0;
    if (pc.coarse_enabled != 0u) {
        uint tiles_w = (w + TILE_SIZE_FINE - 1u) / TILE_SIZE_FINE;
        uint tx = px / TILE_SIZE_FINE;
        uint ty = py / TILE_SIZE_FINE;
        TileBound tb = Tiles[ty * tiles_w + tx];
        if ((tb.flags & TILE_FLAG_EMPTY) != 0u) {
            // EMPTY fast-path: the marcher's else-if(has_mesh)/else arms with hit = false.
            // PBR MVP-2 (mask == 0 arms): write the base mesh/background color to gAlbedo
            // and gMaterial = (shadow = 1, ao = 1, mask = 0, 1). mask == 0 makes the resolve
            // PASS THE BASE THROUGH byte-identically (no PBR, no material fetch) — the
            // 0%-gate for mesh / background / empty. gNormal is neutral (unread when mask==0).
            float3 empty_color = has_mesh ? MESH_COLOR : BACKGROUND;
            gAlbedo[uint2(px, py)] = float4(clamp(empty_color, 0.0, 1.0), 1.0);
            gNormal[uint2(px, py)] = float4(0.5, 0.5, 0.0, 0.0);   // neutral oct, id = 0
            gMaterial[uint2(px, py)] = float4(1.0, 1.0, 0.0, 1.0); // shadow=1, ao=1, mask=0
            // Lighting L0b (C2): this EMPTY-tile early-return is a SEPARATE terminal exit
            // BEFORE the final block, so gViewT must be written here too — the `1.0e30`
            // sentinel (mask == 0, never read on a non-lit pixel). Omitting it would leave
            // an EMPTY pixel's gViewT lane unwritten this frame (the prior single-site plan's
            // bug). EXACTLY-ONCE: this thread returns immediately after.
            gViewT[uint2(px, py)] = 1.0e30;
            return;
        }
        t_seed = tb.near_t;
    }

    // Sphere-trace, BOUNDED by the mesh depth: as soon as the march parameter reaches
    // t_mesh the mesh is in front from here on, so the SDF cannot win. P4b seeds the
    // start at `t_seed` (0.0 when cull-off — byte-identical; else the tile's near_t).
    //
    // Render B1: Keinert over-relaxation (omega-gated). The ENTIRE over-relaxation block
    // is gated behind `if (omega > 1.0)`; the else-arm is the VERBATIM frozen `t += d`,
    // so at omega == 1.0 the live path is textually the pre-B1 plain sphere-trace (the
    // 0%-gate). The frozen ordering (top mesh-guard, probe, hit test, step, miss test) is
    // preserved exactly.
    float t = t_seed;
    float omega = pc.omega;          // [1.0, 1.99] host-clamped
    bool hit = false;
    float safe_t = 0.0;              // probe param remembered for an exact retreat
    float sor_prev = 0.0;           // previous probe's d
    float sor_step_prev = 0.0;      // previous over-relaxed step length
    // BUG-B1-HOLE-3 (Candidate C): EXHAUSTION flag. Set only if the fast loop runs
    // ALL MAX_IT iterations without ANY break — i.e. the ray neither converged
    // (`hit`), nor clearly left the scene (`t > T_MAX`), nor hit the mesh
    // (`t >= t_mesh`); it simply ran out of budget mid-field. This is the precise,
    // minimal re-march trigger (under-detecting it would reopen the hole; the flag is
    // unambiguous — it is true exactly when the `for` falls off the end). It starts
    // `true` and is cleared by EVERY in-loop `break`.
    bool exhausted = true;
    [loop]
    for (uint it = 0u; it < MAX_IT; ++it) {
        if (t >= t_mesh) {
            // The mesh occludes the SDF from this distance onward — stop marching.
            exhausted = false;       // mesh-occlusion termination — NOT budget exhaustion
            break;
        }
        float3 p = ro + rd * t;

        // --- SDF brick-atlas M1: the EMPTY-SPACE-SKIP prefix. `brick_enabled == 0` leaves
        // this block textually dead → the marcher is the EXACT pre-M1 analytic sphere-trace
        // (the 0%-gate). When enabled, read this point's pointer-grid cell:
        //   * EmptyOutside → step to the brick AABB exit (a plain, non-over-relaxed step)
        //     and CONTINUE, skipping the analytic fold. SOUND: the conservative classifier
        //     guarantees no surface within band_half of an EMPTY brick, so the boundary step
        //     never over-steps a surface (the next brick is Surface if a surface is near).
        //   * EmptyInside / Surface (and OUTSIDE the bounded grid) → fall through to the
        //     EXACT `sdf(p)` fold below. EmptyInside is the start-inside case the analytic
        //     negative-`d` handling already covers; outside-grid folds analytically (the
        //     grid is a near-field accelerator, never a correctness boundary).
        // The hit/normal stay ANALYTIC (C1): the skip only accelerates EMPTY traversal, so
        // the converged hit `t` equals the pure-analytic hit `t`. ---
        if (pc.brick_enabled != 0u) {
            // M4 clip-map LOD: pick the finest enclosing level for `p`, then read THAT level's pointer
            // grid + geometry via the STATIC branch-ladder. `lvl < 0` (outside every active level) →
            // skip the empty-skip and fold analytically (as M1 does outside its grid). At N=1
            // (`pc.brick_levels == 1`) only the lvl==0 arm runs (the push grid geometry + level-0's
            // PointerGrid) — byte-identical to the pre-M4 M1 empty-skip.
            int lvl = select_level(p);
            if (lvl >= 0) {
                float3 cell_min;
                uint cls;
                float bw;
                if (lvl == 0) {
                    // Level 0 reads the SAME push grid geometry the pre-M4 M1 skip used (`pc.grid_*`),
                    // so the N=1 path is byte-identical (the push + m2_levels[0] carry the same origin).
                    cls = brick_cell_class(PointerGrid, pc.grid_origin, pc.brick_world, pc.grid_dims, p, cell_min);
                    bw = pc.brick_world;
                } else if (lvl == 1) {
                    cls = brick_cell_class(PointerGrid1, m2_levels[1].origin_brick_world.xyz,
                                           m2_levels[1].origin_brick_world.w,
                                           (uint3)m2_levels[1].dims_atlas_dim.xyz, p, cell_min);
                    bw = m2_levels[1].origin_brick_world.w;
                } else {
                    cls = brick_cell_class(PointerGrid2, m2_levels[2].origin_brick_world.xyz,
                                           m2_levels[2].origin_brick_world.w,
                                           (uint3)m2_levels[2].dims_atlas_dim.xyz, p, cell_min);
                    bw = m2_levels[2].origin_brick_world.w;
                }
                if (cls == BRICK_EMPTY_OUTSIDE) {
                    t += dist_to_brick_exit(p, rd, cell_min, bw);
                    if (t > T_MAX) {
                        exhausted = false;   // clear-miss termination — NOT budget exhaustion
                        break;
                    }
                    continue;                // skip the analytic fold this step
                }
            }
        }

        // --- SDF brick-atlas M2: the trilinear SURFACE-brick path. `brick_trilinear == 0` leaves
        // this block textually dead → the marcher is the EXACT M1 behavior (the M2 0%-gate). When
        // enabled, inside a SURFACE brick (per the M2 grid lookup in `m2_surface_hit`) sample the
        // atlas + run the JCGT cubic for the EXACT ray↔isosurface crossing, validated analytically
        // (the exact-CSG residual fallback). A hit TERMINATES the march at the analytically-decided
        // `t` (hit/normal stay analytic — C1). No cubic crossing / a CSG-rounded divergence FALLS
        // THROUGH to the M1 analytic fold below (so a brick the cubic clears is still marched
        // exactly). INDEPENDENT of `brick_enabled` (the M1 empty-skip can be OFF here). ---
        if (pc.brick_trilinear != 0u) {
            // M4 clip-map LOD: pick the finest enclosing level for `p`, then dispatch the surface-hit
            // to THAT level's atlas/grid via the STATIC branch-ladder (HLSL can't dynamically index the
            // N separate resources). `lvl < 0` → `p` is outside every active level → skip the brick
            // block and fall to the analytic fold (exactly as M2 does outside its grid). At N=1
            // (`pc.brick_levels == 1`) `select_level` returns 0 iff in the level-0 box, and only the
            // lvl==0 arm runs (the M2 resources) — byte-identical to the pre-M4 M2 path.
            int lvl = select_level(p);
            if (lvl >= 0) {
                float m2_hit_t;
                bool m2_hit;
                if (lvl == 0) {
                    m2_hit = m2_surface_hit(m2_levels[0], BrickAtlas,  BrickSampler,  ro, rd, t, m2_hit_t);
                } else if (lvl == 1) {
                    m2_hit = m2_surface_hit(m2_levels[1], BrickAtlas1, BrickSampler1, ro, rd, t, m2_hit_t);
                } else {
                    m2_hit = m2_surface_hit(m2_levels[2], BrickAtlas2, BrickSampler2, ro, rd, t, m2_hit_t);
                }
                if (m2_hit) {
                    hit = true;
                    exhausted = false;       // M2 cubic+analytic-validated convergence
                    t = m2_hit_t;
                    break;
                }
            }
            // else: no level / no cubic crossing in this brick (or the refine cleared it) → fall through
            // to the analytic `sdf(p)` step, exactly as M1 marches a SURFACE brick.
        }

        float d = sdf(p);
        if (d < EPS) {
            hit = true;
            exhausted = false;       // converged — NOT budget exhaustion
            // B1 over-relaxation accept-refine (mirror the brick `m2_surface_hit` signed
            // refine). `d < EPS` is a ONE-SIDED upper bound: an over-relaxed step (`omega > 1`)
            // can jump from outside to DEEP inside in one stride, so the accepted `d` may be
            // large-NEGATIVE (the hit point sits ~δ below the surface). Committing that `t`
            // makes `sdf_soft_shadow` / `sdf_ao` sample from inside the field → shadow == ao == 0
            // → the resolve renders the surface BLACK. Settle the hit ONTO the surface with the
            // SAME signed, under-relaxed sphere-trace the brick path uses: `t += M2_REFINE_RELAX
            // * d` walks BACKWARD for `d < 0` (an overshot inside hit, back toward the surface)
            // and forward for `d > 0`; accept on `abs(d) < EPS` so an inside hit is corrected,
            // never committed. The plain arm (`omega == 1`) is sphere-traced from OUTSIDE so its
            // accept `d` is already in `[0, EPS)` — the first iteration's `abs(d) < EPS` accepts
            // immediately (the omega==1 t is byte-unchanged, the 0%-gate). Bounded by
            // M2_REFINE_ITERS; only the FINAL accept refines (the omega march speed is preserved).
            [loop]
            for (uint ri = 0u; ri < M2_REFINE_ITERS; ++ri) {
                float rd_ = sdf(ro + rd * t);
                if (abs(rd_) < EPS) {
                    break;
                }
                // Named `step` (no FMA contraction) so the host `step = M2_REFINE_RELAX * rd_;
                // t += step;` rounds bit-identically (two roundings: the multiply, then the add).
                float step = M2_REFINE_RELAX * rd_;
                t += step;
            }
            break;
        }
        if (omega > 1.0) {
            float step_len = d * omega;
            // sor_fail: the over-step taken last iter overshot the previous unbounding
            // sphere. Valid only for omega < 2 (host-clamped). Spheres must overlap or we
            // may have skipped a surface. Lipschitz-aware (BUG-B1-HOLE-1): IQ's smooth-min
            // is super-Lipschitz, so the guaranteed-empty radius at field value `f` is `f/L`,
            // not `f`. Two empty balls (radii sor_prev/L, d/L) cover the over-relaxed step
            // `sor_step_prev` iff `sor_prev + d >= L*sor_step_prev`; the retreat must fire
            // when that fails. Multiply the threshold by FIELD_LIPSCHITZ_L (defined in
            // sdf_field.hlsli) to keep the lower-bound invariant sound in blend bands.
            //
            // The `it > 0u` guard is LOAD-BEARING (do not remove): a sor-fail can only be
            // reached after at least one ACCEPTED over-relax step (it >= 1 ⟹ accepted >= 1),
            // which is what pre-pays the +1 retreat iteration in the budget proof below.
            if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) {
                // BUG-B1-HOLE-2: do NOT retreat to bare `safe_t` and re-probe. That re-evals
                // the field at safe_t (costing +2 iters vs a plain march), and on a ray
                // converging at the MAX_IT cliff the extra probe overflows the budget → a
                // missed-surface hole. Instead RESUME the plain march ONE certified step past
                // the safe point: `safe_t` is the exact probe param and `sor_prev` is the
                // exact field value sampled there, so `safe_t + sor_prev` is precisely where a
                // plain march lands after probing safe_t — reusing that eval (no re-probe). The
                // add is one same-sign FMA-free addition (both operands >= 0): no catastrophic
                // cancellation, unlike a `t - <correction>` subtraction form. Net cost is +1
                // iteration vs plain, pre-paid by the >= 1 accepted over-step (the it>0 guard).
                t = safe_t + sor_prev; // plain-resume one certified step past the safe probe
                omega = 1.0;           // permanent fall-to-plain for the rest of this ray
                continue;
            }
            safe_t = t;              // remember THIS probe point
            sor_prev = d;
            sor_step_prev = step_len;
            t += step_len;
        } else {
            t += d;                  // frozen plain arm — TEXTUALLY identical to the frozen loop
        }
        if (t > T_MAX) {
            exhausted = false;       // clear-miss termination — NOT budget exhaustion
            break;
        }
    }

    // BUG-B1-HOLE-3 (Candidate C): the PROVABLY-hole-free fallback re-march. The fast
    // over-relaxed pass above can fall BEHIND a plain march on a non-monotone field
    // (the `steps(omega) <= steps(1)` bound is genuinely violated and unbounded), so it
    // can exhaust the budget mid-field on a ray the FROZEN plain marcher would have hit.
    // If that happened (`exhausted` — ran all MAX_IT with no break, so it neither
    // converged nor left the scene nor hit the mesh), RE-MARCH from the ORIGINAL seed
    // with a plain omega = 1.0 sphere-trace and use ITS result. This second loop is the
    // EXACT frozen marcher body (no omega, no sor logic — `t += d`), so any surface the
    // frozen path hits within MAX_IT it hits here too. Hence B1 reports "no hit" only
    // where BOTH passes miss — i.e. exactly where the frozen marcher misses: B1's hit-set
    // is identical to the frozen hit-set with NO dependence on any step-count bound.
    //
    // At omega == 1.0 the fast pass IS the frozen plain loop, so on exhaustion this
    // re-march reproduces the identical frozen (hit = false) result — the omega == 1.0
    // OUTPUT is byte-unchanged (the 0%-gate). Over-detecting `exhausted` is harmless
    // (a clear-miss re-march just misses again); under-detecting would reopen a hole.
    if (exhausted) {
        t = t_seed;                  // re-seed from the SAME original seed the fast pass used
        hit = false;
        [loop]
        for (uint it2 = 0u; it2 < MAX_IT; ++it2) {
            if (t >= t_mesh) {
                break;               // mesh occludes from here on
            }
            float3 p = ro + rd * t;
            float d = sdf(p);
            if (d < EPS) {
                hit = true;
                break;
            }
            t += d;                  // frozen plain step
            if (t > T_MAX) {
                break;
            }
        }
    }

    // --- PBR MVP-2: the marcher WRITES G-BUFFER ATTRIBUTES; the full Cook-Torrance
    // shade moves to the fullscreen `deferred_pbr.comp` resolve.
    //   * gAlbedo  = RAW LINEAR base color. SDF hit: `materials[id].base_color.rgb`
    //                (the picked edit's material, NO Lambert/shadow/ao baked in). Mesh /
    //                background: the flat MESH_COLOR / BACKGROUND constant (mask == 0).
    //   * gNormal  = (oct.x, oct.y, matid_lo, matid_hi): the octahedral-encoded world
    //                normal in RG + the 16-bit picked material id packed into BA. SDF hit
    //                only; neutral on the mask == 0 arms (unread by the resolve there).
    //   * gMaterial= (shadow, ao, mask, 1): the A1 soft-shadow visibility (R), the A2 AO
    //                factor (G), and the SDF-lit selector (B). The resolve modulates the
    //                DIRECT term by shadow and the AMBIENT term by ao when mask == 1, and
    //                passes gAlbedo through unchanged when mask == 0 (mesh/bg 0%-gate).
    //
    // The shadow + ao marches stay here (in-register from the exact hit p/n against the
    // FROZEN field, unchanged). The material PICK (`pick_material_id`) and the SSBO
    // base-color fetch are CONSUMERS that never touch the field. Defaults below cover the
    // mask == 0 arms so every channel is written (gMaterial is never stale).
    float3 base = BACKGROUND;                      // gAlbedo default (mask == 0 background)
    float shadow = 1.0;                            // A1 visibility default (no occlusion)
    float ao = 1.0;                                // A2 AO default (unoccluded)
    float mask = 0.0;                             // NOT SDF-lit (resolve passes base through)
    float2 oct = float2(0.5, 0.5);                // neutral oct-normal (unread when mask==0)
    float2 id_ba = float2(0.0, 0.0);             // material id 0 (unread when mask==0)
    if (hit && t < t_mesh) {
        // The SDF surface is in FRONT of the mesh (or there is no mesh): emit PBR attrs.
        float3 p = ro + rd * t;
        float3 n = sdf_normal(p);                  // FROZEN field gradient (the hit normal)

        // ATTRIBUTE the hit to the nearest edit's material (hard-union nearest-surface),
        // then fetch its RAW LINEAR base color — gAlbedo carries NO lighting (the resolve
        // runs the full BRDF). Both are CONSUMERS; the field is untouched.
        uint mat_id = pick_material_id(p);
        base = Materials[mat_id].base_color.rgb;

        // Render A1/A2: the soft-shadow + AO marches, gated by `lighting_flags`. OFF
        // (`lighting_flags == 0`) leaves shadow == ao == 1.0 (a STRUCTURAL `if`, no march)
        // → R8 1.0, a no-op modulation in the resolve.
        if (pc.lighting_flags != 0u) {
            float3 light = normalize(pc.light_dir);
            if (pc.lighting_flags & LIGHTING_FLAG_SHADOWS) {
                shadow = sdf_soft_shadow(p, n, light);
            }
            if (pc.lighting_flags & LIGHTING_FLAG_AO) {
                ao = sdf_ao(p, n);
            }
        }

        mask = 1.0;                              // SDF-LIT (the resolve runs Cook-Torrance)
        oct = oct_encode(n);                       // octahedral normal -> gNormal.RG
        id_ba = pack_material_id_ba(mat_id);     // 16-bit material id -> gNormal.BA
    } else if (has_mesh) {
        base = MESH_COLOR;                         // mesh arm: mask = 0 (resolve pass-through)
    } else {
        base = BACKGROUND;                         // background arm: mask = 0 (pass-through)
    }

    // PBR MVP-2 WRITES. The resolve reads gNormal (oct + id), gAlbedo (raw base), and
    // gMaterial (shadow, ao, mask), fetches `materials[id]`, and runs Cook-Torrance.
    gAlbedo[uint2(px, py)] = float4(clamp(base, 0.0, 1.0), 1.0);
    gNormal[uint2(px, py)] = float4(oct.x, oct.y, id_ba.x, id_ba.y);
    gMaterial[uint2(px, py)] = float4(shadow, ao, mask, 1.0);
    // Lighting L0b (C2): the third + last terminal write site. The mesh + background arms
    // (mask == 0) and the SDF-lit arm (mask == 1) all fall through here. On the lit arm
    // store the REAL marched ray param `t` (the same `t` the SDF-hit arm's `p = ro + rd * t`
    // used, in scope here); on the mesh/background arm store the `1.0e30` sentinel (never
    // read on a non-lit pixel). `t` is the true world distance (rd is unit), so the resolve
    // reconstructs `P = ro + rd * t` exactly.
    gViewT[uint2(px, py)] = (mask == 1.0) ? t : 1.0e30;
}
