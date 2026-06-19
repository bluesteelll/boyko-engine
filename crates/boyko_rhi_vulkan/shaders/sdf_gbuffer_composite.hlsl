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
// Additively (unconsumed in P1b, the MRT G-buffer foundation):
//   * binding 3 — `gNormal[px,py] = float4(sdf_normal(p)*0.5+0.5, 1)` on an SDF hit,
//     else a neutral 0.5 (the world normal encoded to UNORM).
//   * binding 4 — `gMaterial[px,py]` a constant material id/params slot.
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
//   binding 2 : RWTexture2D<float4> (STORAGE, rgba8) — albedo (the marcher color).
//   binding 3 : RWTexture2D<float4> (STORAGE, rgba8) — world normal (additive).
//   binding 4 : RWTexture2D<float4> (STORAGE, rgba8) — material (additive constant).
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
//
// # The push constant (Render P4b + B1 — pushed against THIS pipeline's OWN dedicated
//   layout via `push_compute_constants`, NOT the shared `push_constants`)
//
// An 8-byte `[[vk::push_constant]]` block: `uint coarse_enabled` (offset 0) gates the
// coarse cull and `float omega` (offset 4) carries the Render B1 over-relaxation factor.
// `coarse_enabled == 0` (cull-off) keeps the marcher byte-identical to today; `!= 0`
// reads binding 6 and either early-outs an EMPTY tile (mesh/background composite) or
// seeds `t = near_t`. `omega == 1.0` keeps the sphere-trace TEXTUALLY the frozen plain
// loop (the 0%-gate); `> 1.0` enables Keinert over-relaxation with an exact-retreat
// safeguard. 8 bytes fits the declared 80-byte COMPOSITE push range.
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
// `CAM_MODE_ORTHO` / `CAM_MODE_PERSPECTIVE`.
static const uint CAM_ORTHO       = 0u;
static const uint CAM_PERSPECTIVE = 1u;

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

// Render P4b: a push constant on the fine pipeline (the ONLY push — the camera is the
// UBO @ b5). `coarse_enabled == 0` keeps the OFF path BYTE-IDENTICAL to today's marcher
// (the 0%-gate anchor); `!= 0` reads the tile's `TileBound` and culls.
//
// Render B1: a second 4-byte field `omega` carries the Keinert over-relaxation factor,
// host-clamped to [1.0, 1.99]. At `omega == 1.0` the marcher's live path is TEXTUALLY
// the frozen plain sphere-trace (see the gated loop below — the 0%-gate). 8 bytes fits
// the declared 80-byte COMPOSITE push range, so pipeline creation is unchanged.
[[vk::push_constant]] struct PushConstants {
    uint  coarse_enabled;   // offset 0 (unchanged)
    float omega;            // offset 4 — Keinert over-relaxation factor, host-clamped [1.0, 1.99]
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

static const float CAM_Z       = 2.0;   // camera plane Z (rays start here)
static const float HALF_EXTENT = 1.0;   // orthographic view half-extent in world units

static const float3 LIGHT_DIR  = float3(0.0, 0.0, 1.0); // points toward +Z (at the camera)
static const float3 BASE_COLOR = float3(0.8, 0.3, 0.2); // the SDF surface albedo
static const float  AMBIENT    = 0.1;

static const float3 BACKGROUND = float3(0.05, 0.05, 0.1);  // miss color
// The flat mesh albedo — a green clearly distinct from both the SDF lit color
// (warm orange/red) and the background (dark blue). Mirrored host-side.
static const float3 MESH_COLOR = float3(0.15, 0.65, 0.25);

// The constant material slot written into gMaterial (additive, unconsumed in P1b).
static const float4 MATERIAL_CONST = float4(0.0, 0.0, 0.0, 1.0);

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

    float3 ro;
    float3 rd;
    if (camera_mode == CAM_PERSPECTIVE) {
        // ADDITIVE perspective ray-gen, strictly inside this branch — the ORTHO
        // arithmetic below is byte-untouched. NDC in [-1,+1] (+x right, +y up, y
        // flipped to match the ortho convention); the ray direction is the camera
        // basis combined with the NDC scaled by the half-FOV tangent and aspect. Plain
        // IEEE ops (no rsqrt/rcp/fast-math) so a perspective scene is reproducible.
        float ndc_x =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float ndc_y = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        float tan_half_fov = cam_forward.w; // tan(fovY / 2)
        float aspect       = cam_right.w;   // W / H
        float3 dir = cam_forward.xyz
                   + cam_right.xyz * (ndc_x * aspect * tan_half_fov)
                   + cam_up.xyz    * (ndc_y * tan_half_fov);
        ro = cam_eye.xyz;
        rd = normalize(dir);
    } else {
        // Reconstruct the orthographic ray for this pixel (deterministic, golden-frozen).
        float u =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float v = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        ro = float3(u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z);
        rd = float3(0.0, 0.0, -1.0);
    }

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
            float3 empty_color = has_mesh ? MESH_COLOR : BACKGROUND;
            gAlbedo[uint2(px, py)] = float4(clamp(empty_color, 0.0, 1.0), 1.0);
            gNormal[uint2(px, py)] = float4(0.5, 0.5, 0.5, 1.0); // neutral world normal
            gMaterial[uint2(px, py)] = MATERIAL_CONST;
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

    float3 color;
    float3 normal_enc = float3(0.5, 0.5, 0.5); // neutral world normal (UNORM-encoded 0)
    if (hit && t < t_mesh) {
        // The SDF surface is in FRONT of the mesh (or there is no mesh): light it.
        float3 p = ro + rd * t;
        float3 n = sdf_normal(p);
        float3 l = normalize(LIGHT_DIR);
        float ndotl = max(dot(n, l), 0.0);
        color = BASE_COLOR * ndotl + BASE_COLOR * AMBIENT;
        normal_enc = n * 0.5 + 0.5; // world normal encoded into [0,1] for the G-buffer
    } else if (has_mesh) {
        // No nearer SDF surface, but the mesh covered this pixel — flat mesh color.
        color = MESH_COLOR;
    } else {
        color = BACKGROUND;
    }

    // I/O EDIT (2): STORE the marcher color into the ALBEDO storage image (the host
    // golden `golden_composite_pixel_ex` predicts this within +/-2/255) INSTEAD of the
    // packed `Buf[pixel_base() + idx] = pack_rgba(color)`.
    gAlbedo[uint2(px, py)] = float4(clamp(color, 0.0, 1.0), 1.0);
    // Additive MRT targets (unconsumed in P1b).
    gNormal[uint2(px, py)] = float4(clamp(normal_enc, 0.0, 1.0), 1.0);
    gMaterial[uint2(px, py)] = MATERIAL_CONST;
}
