// Phase 6 rung 9 compute shader: sphere-trace an ORDERED SDF EDIT-LIST (multi-
// primitive CSG).
//
// Generalizes the rung-8 single hardcoded sphere (`sdf_spheretrace.hlsl`) into
// the SDF-edits model (SDF doc §2-§3): the scene is an ORDERED list of edits,
// each a primitive (SPHERE or BOX) combined into the accumulated field by a
// boolean op (union / subtraction / intersection), optionally smoothed by a
// polynomial smooth-min/-max blend (`smoothness > 0`). This stays the ANALYTIC
// base (the field is folded per pixel each march step) — NO grid cache / brick
// atlas / clipmap / BVH (SDF doc §5-§8, all deferred).
//
// # The packed-header buffer contract (why one binding, not two)
//
// The proven Slice-0 compute path is a FIXED single-binding layout: binding 0
// (set 0) is one `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count`
// push constant (the `ComputeLayouts` shared by the device + the per-encoder
// descriptor set). Adding a second storage binding would mean reshaping the
// device-shared compute descriptor-set layout, the per-encoder descriptor pool,
// and `bind_storage_buffer`/`dispatch` — disproportionate, and it would also
// alter every existing rung-1 compute test that rides the same shared layout.
//
// The plan explicitly sanctions the alternative: PACK the edit-list as a HEADER
// region at the front of the single output buffer. So the one buffer is:
//
//   word 0                : uint edit_count        (the header count)
//   words [HEADER_BASE ..]: MAX_SDF_EDITS * SdfEdit (the std430 edit array)
//   words [PIXEL_BASE  ..]: IMG_W * IMG_H * uint    (the packed-RGBA output)
//
// The CPU writes the header (edit_count + edits) before the dispatch; the shader
// READS the header (never writes it) and WRITES only the pixel region. The push
// constant stays `uint count` = the PIXEL count (so the dispatch group count is
// unchanged: ceil(pixels / 64)).
//
// # std430 SdfEdit layout (the CPU<->GPU contract, mirrored host-side)
//
// `SdfEdit` is laid out so the Rust `#[repr(C, align(16))]` struct and this
// std430 structured-buffer element are byte-identical (no std430 vs repr(C)
// padding surprise). All members are scalar `uint`/`float` or `float4`:
//
//   offset  0 : float4 center  (xyz = center/position, w unused)   [16 B]
//   offset 16 : float4 params  (xyz = radius / half-extents, w unused) [16 B]
//   offset 32 : uint   kind    (0 = SPHERE, 1 = BOX)               [ 4 B]
//   offset 36 : uint   op      (0 = UNION, 1 = SUBTRACT, 2 = INTERSECT) [ 4 B]
//   offset 40 : float  smoothness (0 = hard op; > 0 = smooth blend k) [ 4 B]
//   offset 44 : uint   _pad    (keeps size a 16-B multiple)        [ 4 B]
//   total: 48 bytes, 16-byte aligned.
//
// Storing `center` as a `float4` (not `float3`) makes the following `float4
// params` start at offset 16 WITHOUT std430 inserting padding the Rust side
// would have to mirror — the two layouts are then trivially identical.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_editlist.hlsl -Fo sdf_editlist.comp.spv
//
// # The deterministic camera / light (the single source of truth)
//
// The camera + light are mirrored EXACTLY by the host-side golden in
// `compute.rs` (`golden_editlist_*`). The SDF math here (the primitive distance
// functions + boolean ops + smooth-min + central-difference gradient) is the
// single source of truth the host golden reproduces line-for-line (a future CPU
// physics evaluator will reuse the SAME field math).
//
//   Image:   IMG_W x IMG_H pixels, pixel index = py * IMG_W + px.
//   Camera:  ORTHOGRAPHIC, looking down -Z at the origin (deterministic, no
//            perspective divide). For pixel (px,py):
//              u =  ((px + 0.5) / IMG_W) * 2 - 1     // [-1, +1), +x right
//              v = -(((py + 0.5) / IMG_H) * 2 - 1)   // [-1, +1), +y up (flipped)
//            ray origin = (u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z)
//            ray dir    = (0, 0, -1)                 // straight down -Z
//   Light:   one hardcoded directional light from LIGHT_DIR; Lambert + ambient:
//              lit = BASE_COLOR * max(dot(N, L), 0) + BASE_COLOR * AMBIENT
//            clamped to [0,1] and packed as 0xAABBGGRR (R in the low byte).
//   Miss:    BACKGROUND color packed the same way.

RWStructuredBuffer<uint> Buf : register(u0);

// The shared SDF field gateway (field consts/enums + `Edit`/`load_edit` + the
// primitive distances + boolean ops + smooth-min/-max + the edit-list `sdf` +
// `sdf_normal`). `Buf` (declared above) is the include contract precondition. This
// header also defines `FAR` and `GRAD_H`, so they are NOT redeclared below; and it
// defines `MAX_SDF_EDITS`/`SDF_EDIT_WORDS`/`HEADER_BASE`, so `PIXEL_BASE` (which
// derives from them) is positioned AFTER this include.
#include "sdf_field.hlsli"

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
static const float3 BASE_COLOR = float3(0.8, 0.3, 0.2); // the surface albedo
static const float  AMBIENT    = 0.1;

static const float3 BACKGROUND = float3(0.05, 0.05, 0.1); // miss color

// Sphere-trace tuning (the §S2 march budget, scaled to the small edit list).
static const float EPS    = 0.001;  // hit threshold on |sdf|
static const float T_MAX  = 10.0;   // miss distance bound
static const uint  MAX_IT = 128u;   // max march steps per ray (the §S2 ceiling)

// --- The edit-list pixel-output base (derives from the header's field consts) -
//
// `MAX_SDF_EDITS`/`SDF_EDIT_WORDS`/`HEADER_BASE` come from `sdf_field.hlsli`; the
// header is `edit_count` (1 word) padded up to 4 words so the edit array starts
// 16-byte aligned, then the edit array, then the pixel output. This MUST match the
// host-side constants in `compute.rs`.
static const uint PIXEL_BASE = HEADER_BASE + MAX_SDF_EDITS * SDF_EDIT_WORDS; // 4 + 192 = 196

// Packs a linear [0,1] RGB into 0xAABBGGRR (alpha forced to 0xFF), matching the
// host-side `pack_rgba` golden (compared with a small +/-2/255 tolerance, NOT
// bit-exact, to absorb GPU FMA/rounding under one LSB).
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
    // iteration ceiling. The field is now the folded edit-list, not one sphere.
    float t = 0.0;
    bool hit = false;
    [loop]
    for (uint it = 0u; it < MAX_IT; ++it) {
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
    if (hit) {
        float3 p = ro + rd * t;
        float3 n = sdf_normal(p);
        float3 l = normalize(LIGHT_DIR);
        float ndotl = max(dot(n, l), 0.0);
        color = BASE_COLOR * ndotl + BASE_COLOR * AMBIENT;
    } else {
        color = BACKGROUND;
    }

    Buf[PIXEL_BASE + idx] = pack_rgba(color);
}
