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
cbuffer Camera : register(b5) {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
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
    float res = 1.0;
    float t = SHADOW_MINT;
    [loop]
    for (uint i = 0u; i < MAX_IT; ++i) {
        float d = field_distance(p + L * t);
        res = min(res, SHADOW_K * d / t);
        if (d < SHADOW_HIT_EPS) {
            return 0.0; // hit an occluder — fully shadowed
        }
        t += max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);
        if (t > T_MAX) {
            break;
        }
    }
    return clamp(res, 0.0, 1.0);
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
float dist_to_brick_exit(float3 p, float3 rd, float3 cell_min) {
    float bw = pc.brick_world;
    float exit = 1.0e30;
    [unroll]
    for (uint a = 0u; a < 3u; ++a) {
        float dir = rd[a];
        float lo = cell_min[a];
        float hi = lo + bw;
        // A near-axis-parallel component imposes no exit bound (the other axes do; the
        // clamp covers a fully-degenerate ray).
        if (abs(dir) <= BRICK_EXIT_EPS) {
            continue;
        }
        float inv = 1.0 / dir;
        float t_lo = (lo - p[a]) * inv;
        float t_hi = (hi - p[a]) * inv;
        float t_far = max(t_lo, t_hi); // the far-face crossing along this axis
        exit = min(exit, t_far);
    }
    // Progress guarantee: a degenerate / boundary-grazing exit must still advance.
    return (exit < BRICK_EXIT_EPS) ? BRICK_EXIT_EPS : exit;
}

// Reads the pointer-grid cell class containing world point `p`, returning its BrickClass
// (and `cell_min` via the out param) or BRICK_OUTSIDE_GRID when `p` is outside the bounded
// grid. Mirrors `boyko_sdf_math::brick`'s host_brick_cell index + bounds check exactly.
uint brick_cell_class(float3 p, out float3 cell_min) {
    float3 origin = pc.grid_origin;
    float bw = pc.brick_world;
    float3 rel = (p - origin) / bw;
    cell_min = origin; // default (overwritten on an in-grid hit; unread when OUTSIDE)
    // Outside on any axis (incl. negative rel) → no cell. Test the float directly so a
    // negative coordinate is caught before the uint cast wraps it.
    if (rel.x < 0.0 || rel.y < 0.0 || rel.z < 0.0) {
        return BRICK_OUTSIDE_GRID;
    }
    uint3 dims = pc.grid_dims;
    uint ix = (uint)rel.x;
    uint iy = (uint)rel.y;
    uint iz = (uint)rel.z;
    if (ix >= dims.x || iy >= dims.y || iz >= dims.z) {
        return BRICK_OUTSIDE_GRID;
    }
    uint idx = ix + iy * dims.x + iz * dims.x * dims.y;
    cell_min = origin + float3(ix, iy, iz) * bw;
    return PointerGrid[idx];
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
            float3 cell_min;
            uint cls = brick_cell_class(p, cell_min);
            if (cls == BRICK_EMPTY_OUTSIDE) {
                t += dist_to_brick_exit(p, rd, cell_min);
                if (t > T_MAX) {
                    exhausted = false;   // clear-miss termination — NOT budget exhaustion
                    break;
                }
                continue;                // skip the analytic fold this step
            }
        }

        float d = sdf(p);
        if (d < EPS) {
            hit = true;
            exhausted = false;       // converged — NOT budget exhaustion
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
