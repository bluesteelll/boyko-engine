// Phase 6 rung 10 compute shader: SDF + MESH HYBRID COMPOSITE via a SHARED DEPTH
// buffer (the SDF-doc §15.1 seam, the Slice-S2 occlusion-acceptance core).
//
// This generalizes the rung-9 edit-list sphere-trace (`sdf_editlist.hlsl`) into a
// hybrid renderer: a REAL GPU-rasterized mesh's depth (written by the graphics
// pipeline into a D32_SFLOAT attachment, then copied into this buffer) BOUNDS the
// SDF sphere-trace march, so the mesh and the SDF OCCLUDE EACH OTHER correctly,
// composited into one image. The SDF field math + lighting + camera are FROZEN to
// rung 9 (sphere/box + union/sub/intersect + smooth-min, no shader-only
// primitives); the only new thing is the per-pixel mesh-depth read + the
// composite. The host-side single source of truth lives in `compute.rs`
// (`golden_composite_pixel` + the composite consts/const-asserts).
//
// # The packed-header buffer contract (one binding, a NEW depth region)
//
// Keeps the PROVEN single-binding compute layout verbatim: binding 0 (set 0) is
// one `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant.
// It extends the rung-9 word layout with a DEPTH region between the edit array and
// the pixel output (rung 9's layout/shader/consts are UNTOUCHED — this is a new
// file with its OWN base constants):
//
//   word 0                       : uint  edit_count        (the header count)
//   words [HEADER_BASE ..]       : MAX_SDF_EDITS * SdfEdit (the std430 edit array)
//   words [DEPTH_BASE ..]        : IMG_W * IMG_H * float   (the GPU mesh depth)
//   words [PIXEL_BASE ..]        : IMG_W * IMG_H * uint    (the packed-RGBA output)
//
// The CPU writes the header (edit_count + edits) before the dispatch; the GPU
// image→buffer copy writes the DEPTH region (the rasterized mesh's D32_SFLOAT
// depth, one float per pixel, after a depth-write → transfer → compute-read
// barrier chain); this shader READS the header + the depth region and WRITES only
// the pixel region. The push constant stays `uint count` = the PIXEL count.
//
// # std430 SdfEdit layout (identical to rung 9, mirrored host-side)
//
//   offset  0 : float4 center  (xyz = center/position, w unused)      [16 B]
//   offset 16 : float4 params  (xyz = radius / half-extents, w unused) [16 B]
//   offset 32 : uint   kind    (0 = SPHERE, 1 = BOX)                  [ 4 B]
//   offset 36 : uint   op      (0 = UNION, 1 = SUBTRACT, 2 = INTERSECT) [ 4 B]
//   offset 40 : float  smoothness (0 = hard op; > 0 = smooth blend k)  [ 4 B]
//   offset 44 : uint   _pad    (keeps size a 16-B multiple)           [ 4 B]
//   total: 48 bytes, 16-byte aligned.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_depth_composite.hlsl -Fo sdf_depth_composite.comp.spv
//
// # The deterministic camera / depth convention (the single source of truth)
//
// The camera + light are mirrored EXACTLY by the host-side golden in `compute.rs`
// (`golden_composite_pixel`). The SDF math here is the same single source of truth
// rung 9 froze (a future CPU physics evaluator reuses the SAME field math).
//
//   Image:   IMG_W x IMG_H pixels, pixel index = py * IMG_W + px.
//   Camera:  ORTHOGRAPHIC, looking down -Z at the origin (no perspective divide).
//            For pixel (px,py):
//              u =  ((px + 0.5) / IMG_W) * 2 - 1     // [-1, +1), +x right
//              v = -(((py + 0.5) / IMG_H) * 2 - 1)   // [-1, +1), +y up (flipped)
//            ray origin = (u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z)
//            ray dir    = (0, 0, -1)                 // straight down -Z
//   Depth:   the mesh is rasterized with an ORTHOGRAPHIC projection chosen so the
//            STORED depth equals the NORMALIZED ray parameter `t / T_MAX`, where
//            `t = CAM_Z - worldZ` (the distance from the camera plane). Near plane
//            worldZ = CAM_Z -> depth 0; far plane worldZ = CAM_Z - T_MAX -> depth 1.
//            Because the projection is orthographic, depth is EXACTLY linear in `t`
//            (no perspective divide), so for a fronto-parallel surface the mapping
//            is exact. The host MVP (in the rung-10 test) packs that projection;
//            here we invert it: `t_mesh = depth * T_MAX` (and `depth == clear`
//            (1.0) means "no mesh covered this pixel").
//   Light:   one hardcoded directional light from LIGHT_DIR; Lambert + ambient:
//              lit = BASE_COLOR * max(dot(N, L), 0) + BASE_COLOR * AMBIENT
//            clamped to [0,1] and packed as 0xAABBGGRR (R in the low byte).
//   Mesh:    where covered (and in front of the SDF), a FLAT MESH_COLOR constant.
//            (Reading the mesh's real rasterized albedo from a G-buffer is a
//            deferred S3 refinement — this rung proves DEPTH sharing, so the mesh
//            color is a constant mirrored host-side.)
//   Miss:    BACKGROUND color packed the same way.

RWStructuredBuffer<uint> Buf : register(u0);

struct PushConstants {
    uint count; // total PIXEL count = IMG_W * IMG_H (NOT the buffer word count)
};
[[vk::push_constant]] PushConstants pc;

// --- Deterministic scene constants (mirrored host-side in compute.rs) ---------
static const uint  IMG_W = 64u;
static const uint  IMG_H = 64u;

static const float CAM_Z       = 2.0;   // camera plane Z (rays start here)
static const float HALF_EXTENT = 1.0;   // orthographic view half-extent in world units

static const float3 LIGHT_DIR  = float3(0.0, 0.0, 1.0); // points toward +Z (at the camera)
static const float3 BASE_COLOR = float3(0.8, 0.3, 0.2); // the SDF surface albedo
static const float  AMBIENT    = 0.1;

static const float3 BACKGROUND = float3(0.05, 0.05, 0.1);  // miss color
// The flat mesh albedo — a green clearly distinct from both the SDF lit color
// (warm orange/red) and the background (dark blue). Mirrored host-side.
static const float3 MESH_COLOR = float3(0.15, 0.65, 0.25);

// Sphere-trace tuning (the §S2 march budget; identical to rung 9).
static const float EPS    = 0.001;  // hit threshold on |sdf|
static const float T_MAX  = 10.0;   // miss distance bound (= depth-1.0 far plane)
static const uint  MAX_IT = 128u;   // max march steps per ray (the §S2 ceiling)
static const float GRAD_H = 0.0005; // central-difference half-step for the normal
static const float FAR    = 1.0e9;  // the "empty field" sentinel before the first edit

// The depth value the depth attachment was CLEARED to (the far plane, 1.0). A
// pixel whose stored depth is >= this sentinel had NO mesh fragment rasterized.
static const float DEPTH_CLEAR = 1.0;

// --- The edit-list + depth packed-header contract (mirrored host-side) --------
//
// IDENTICAL header to rung 9 up to the edit array; then a NEW depth region, then
// the pixel output. These MUST match the host-side rung-10 composite constants in
// `compute.rs` (pinned there by const-asserts).
static const uint MAX_SDF_EDITS  = 16u;
static const uint SDF_EDIT_WORDS = 12u;       // size_of::<SdfEdit>() / 4
static const uint HEADER_BASE    = 4u;        // edit array word offset (count padded to 16 B)
static const uint DEPTH_BASE     = HEADER_BASE + MAX_SDF_EDITS * SDF_EDIT_WORDS; // 4 + 192 = 196
static const uint PIXEL_BASE     = DEPTH_BASE + IMG_W * IMG_H;                    // 196 + 4096 = 4292

// Primitive kinds.
static const uint KIND_SPHERE = 0u;
static const uint KIND_BOX    = 1u;

// Boolean ops.
static const uint OP_UNION     = 0u;
static const uint OP_SUBTRACT  = 1u;
static const uint OP_INTERSECT = 2u;

// One decoded edit (the in-register form of the packed std430 element).
struct Edit {
    float3 center;
    float3 params;     // radius (sphere) or half-extents (box)
    uint   kind;
    uint   op;
    float  smoothness;
};

// Reads `asfloat`/`asuint` of the i-th packed edit out of the header region.
Edit load_edit(uint i) {
    uint base = HEADER_BASE + i * SDF_EDIT_WORDS;
    Edit e;
    e.center     = float3(asfloat(Buf[base + 0u]), asfloat(Buf[base + 1u]), asfloat(Buf[base + 2u]));
    // word base+3 = center.w (unused)
    e.params     = float3(asfloat(Buf[base + 4u]), asfloat(Buf[base + 5u]), asfloat(Buf[base + 6u]));
    // word base+7 = params.w (unused)
    e.kind       = Buf[base + 8u];
    e.op         = Buf[base + 9u];
    e.smoothness = asfloat(Buf[base + 10u]);
    // word base+11 = _pad (unused)
    return e;
}

// --- Primitive distance functions (IQ; the frozen rung-9 primitive set) -------

// Sphere: distance to a sphere centered at `c` with radius `r`.
float sd_sphere(float3 p, float3 c, float r) {
    return length(p - c) - r;
}

// Box: distance to an axis-aligned box centered at `c` with half-extents `h`
// (the standard IQ exact box SDF).
float sd_box(float3 p, float3 c, float3 h) {
    float3 q = abs(p - c) - h;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// One edit's primitive distance at `p`.
float edit_distance(Edit e, float3 p) {
    if (e.kind == KIND_BOX) {
        return sd_box(p, e.center, e.params);
    }
    return sd_sphere(p, e.center, e.params.x);
}

// --- Boolean ops + polynomial smooth-min/-max (IQ) ----------------------------

// Polynomial smooth-min (IQ `smin`): a soft union with blend radius `k`.
float smin(float a, float b, float k) {
    float hh = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return lerp(b, a, hh) - k * hh * (1.0 - hh);
}

// Polynomial smooth-max: the De Morgan dual of `smin`.
float smax(float a, float b, float k) {
    return -smin(-a, -b, k);
}

// Combine the accumulated field distance `acc` with one edit's distance `d` under
// the edit's boolean op, hard (`k <= 0`) or smooth (`k > 0`).
float combine(float acc, float d, uint op, float k) {
    if (op == OP_SUBTRACT) {
        return (k > 0.0) ? smax(acc, -d, k) : max(acc, -d);
    } else if (op == OP_INTERSECT) {
        return (k > 0.0) ? smax(acc, d, k) : max(acc, d);
    }
    return (k > 0.0) ? smin(acc, d, k) : min(acc, d);
}

// --- The edit-list field (the single source of truth, identical to rung 9) ----
float sdf(float3 p) {
    uint n = min(Buf[0], MAX_SDF_EDITS); // word 0 = edit_count (clamped to capacity)
    float acc = FAR;
    [loop]
    for (uint i = 0u; i < n; ++i) {
        Edit e = load_edit(i);
        float d = edit_distance(e, p);
        if (i == 0u) {
            acc = d;
        } else {
            acc = combine(acc, d, e.op, e.smoothness);
        }
    }
    return acc;
}

// Surface normal via central differences of `sdf` (the gradient of the WHOLE
// edit-list field).
float3 sdf_normal(float3 p) {
    float2 e = float2(GRAD_H, 0.0);
    float3 n = float3(
        sdf(p + e.xyy) - sdf(p - e.xyy),
        sdf(p + e.yxy) - sdf(p - e.yxy),
        sdf(p + e.yyx) - sdf(p - e.yyx));
    return normalize(n);
}

// Packs a linear [0,1] RGB into 0xAABBGGRR (alpha forced to 0xFF), matching the
// host-side `pack_rgba` golden (compared with a small +/-2/255 tolerance).
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

    // The shared mesh depth for this pixel (written by the GPU image->buffer copy
    // of the rasterized D32_SFLOAT attachment). depth == clear (1.0) => no mesh.
    // Otherwise the mesh's ray parameter is depth * T_MAX (the ortho convention).
    float md = asfloat(Buf[DEPTH_BASE + idx]);
    bool has_mesh = (md < DEPTH_CLEAR);          // strictly less than the far-plane clear
    float t_mesh = has_mesh ? (md * T_MAX) : 1.0e30; // a finite bound only when covered

    // Sphere-trace, BOUNDED by the mesh depth: as soon as the march parameter
    // reaches t_mesh the mesh is in front from here on, so the SDF cannot win.
    float t = 0.0;
    bool hit = false;
    [loop]
    for (uint it = 0u; it < MAX_IT; ++it) {
        if (t >= t_mesh) {
            // The mesh occludes the SDF from this distance onward — stop marching.
            break;
        }
        float3 p = ro + rd * t;
        float d = sdf(p);
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
    if (hit && t < t_mesh) {
        // The SDF surface is in FRONT of the mesh (or there is no mesh): light it.
        float3 p = ro + rd * t;
        float3 n = sdf_normal(p);
        float3 l = normalize(LIGHT_DIR);
        float ndotl = max(dot(n, l), 0.0);
        color = BASE_COLOR * ndotl + BASE_COLOR * AMBIENT;
    } else if (has_mesh) {
        // No nearer SDF surface, but the mesh covered this pixel — flat mesh color.
        color = MESH_COLOR;
    } else {
        color = BACKGROUND;
    }

    Buf[PIXEL_BASE + idx] = pack_rgba(color);
}
