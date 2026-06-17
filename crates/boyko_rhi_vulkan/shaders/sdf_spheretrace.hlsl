// Phase 6 rung 8 compute shader: sphere-trace ONE analytic SDF primitive.
//
// The first real "SDF on screen" thread (Slice S2, the analytic edit-list +
// sphere-trace layer of the SDF doc, §3-§4 — NOT the grid-cache / brick-atlas /
// clipmap / BVH layers, which are deferred). One hardcoded analytic sphere is
// sphere-traced per pixel; the lit RGBA is packed into a u32 and written to a
// storage buffer that the CPU reads back for a golden diff.
//
// Reuses the proven rung-1 compute + storage-BUFFER contract verbatim (NO new
// descriptor plumbing): binding 0 (set 0) is the SAME single RWStructuredBuffer
// <uint> at COMPUTE, and the SAME 4-byte push constant (`uint count`) the
// `write_pattern` / `transform_add` shaders use. Everything else (camera,
// sphere, light) is HARDCODED in this shader so the fixed Slice-0 pipeline
// layout (one storage binding + a 4-byte push range) is unchanged.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_spheretrace.hlsl -Fo sdf_spheretrace.comp.spv
//
// # The deterministic camera / sphere / light (the single source of truth)
//
// These constants are mirrored EXACTLY by the host-side golden in
// `compute.rs` (`golden_sdf_*`) so the center-pixel (HIT) and corner-pixel
// (MISS) colors are computed CPU-side without re-running the GPU. The SDF math
// here (`sdf_sphere` + central-difference gradient) is kept in the documentable
// single-source-of-truth form the plan calls for (physics will later evaluate
// the SAME field on the CPU).
//
//   Image:   IMG_W x IMG_H pixels, pixel index = py * IMG_W + px.
//   Camera:  ORTHOGRAPHIC, looking down -Z at the origin (deterministic, no
//            perspective divide). For pixel (px,py):
//              u =  ((px + 0.5) / IMG_W) * 2 - 1     // [-1, +1), +x right
//              v = -(((py + 0.5) / IMG_H) * 2 - 1)   // [-1, +1), +y up (flipped)
//            ray origin = (u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z)
//            ray dir    = (0, 0, -1)                 // straight down -Z
//   Sphere:  center (0,0,0), radius SPHERE_RADIUS;  sdf(p) = length(p) - r.
//   Light:   one hardcoded directional light from LIGHT_DIR; Lambert + ambient:
//              lit = BASE_COLOR * max(dot(N, L), 0) + BASE_COLOR * AMBIENT
//            clamped to [0,1] and packed as 0xAABBGGRR (R in the low byte).
//   Miss:    BACKGROUND color packed the same way.

RWStructuredBuffer<uint> Output : register(u0);

struct PushConstants {
    uint count; // total pixel count = IMG_W * IMG_H
};
[[vk::push_constant]] PushConstants pc;

// --- Deterministic scene constants (mirrored host-side in compute.rs) ---------
static const uint  IMG_W = 64u;
static const uint  IMG_H = 64u;

static const float CAM_Z       = 2.0;   // camera plane Z (rays start here)
static const float HALF_EXTENT = 1.0;   // orthographic view half-extent in world units

static const float3 SPHERE_CENTER = float3(0.0, 0.0, 0.0);
static const float  SPHERE_RADIUS = 0.5;

static const float3 LIGHT_DIR  = float3(0.0, 0.0, 1.0); // points toward +Z (at the camera)
static const float3 BASE_COLOR = float3(0.8, 0.3, 0.2); // the sphere's albedo
static const float  AMBIENT    = 0.1;

static const float3 BACKGROUND = float3(0.05, 0.05, 0.1); // miss color

// Sphere-trace tuning (the §S2 march budget, scaled down to one primitive).
static const float EPS    = 0.001;  // hit threshold on |sdf|
static const float T_MAX  = 10.0;   // miss distance bound
static const uint  MAX_IT = 128u;   // max march steps per ray (the §S2 ceiling)
static const float GRAD_H = 0.0005;  // central-difference half-step for the normal

// The analytic field: distance to one sphere. This is the single source of
// truth for the SDF math (the physics CPU evaluator will mirror it later).
float sdf_sphere(float3 p) {
    return length(p - SPHERE_CENTER) - SPHERE_RADIUS;
}

// Surface normal via central differences of `sdf_sphere` (the gradient).
float3 sdf_normal(float3 p) {
    float2 e = float2(GRAD_H, 0.0);
    float3 n = float3(
        sdf_sphere(p + e.xyy) - sdf_sphere(p - e.xyy),
        sdf_sphere(p + e.yxy) - sdf_sphere(p - e.yxy),
        sdf_sphere(p + e.yyx) - sdf_sphere(p - e.yyx));
    return normalize(n);
}

// Packs a linear [0,1] RGB into 0xAABBGGRR (alpha forced to 0xFF), matching the
// host-side `pack_rgba` golden. The readback is compared against that golden with a
// small +/-2/255 per-channel tolerance (NOT bit-exact): GPU FMA/rounding can differ
// from the scalar host math by under one LSB, while any wrong color misses by ~100+.
uint pack_rgba(float3 c) {
    float3 cl = clamp(c, 0.0, 1.0);
    uint r = (uint)(cl.r * 255.0 + 0.5);
    uint g = (uint)(cl.g * 255.0 + 0.5);
    uint b = (uint)(cl.b * 255.0 + 0.5);
    return (0xFFu << 24) | (b << 16) | (g << 8) | r;
}

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    if (idx >= pc.count) {
        return;
    }

    uint px = idx % IMG_W;
    uint py = idx / IMG_W;

    // Reconstruct the orthographic ray for this pixel (deterministic).
    float u =  (((float)px + 0.5) / (float)IMG_W) * 2.0 - 1.0;
    float v = -((((float)py + 0.5) / (float)IMG_H) * 2.0 - 1.0);
    float3 ro = float3(u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z);
    float3 rd = float3(0.0, 0.0, -1.0);

    // Sphere-trace: advance by the field distance until a hit, a miss, or the
    // iteration ceiling.
    float t = 0.0;
    bool hit = false;
    [loop]
    for (uint it = 0u; it < MAX_IT; ++it) {
        float3 p = ro + rd * t;
        float d = sdf_sphere(p);
        if (d < EPS) {
            hit = true;
            break;
        }
        t += d;
        if (t > T_MAX) {
            break;
        }
    }

    float3 color;
    if (hit) {
        float3 p = ro + rd * t;
        float3 n = sdf_normal(p);
        float3 l = normalize(LIGHT_DIR);
        float ndotl = max(dot(n, l), 0.0);
        color = BASE_COLOR * ndotl + BASE_COLOR * AMBIENT;
    } else {
        color = BACKGROUND;
    }

    Output[idx] = pack_rgba(color);
}
