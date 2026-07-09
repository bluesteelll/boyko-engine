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
// # P0a — resolution-as-dispatch-dim + an additive perspective camera mode
//
// The image extent (`img_w`/`img_h`) and the camera mode are no longer compile-time
// constants: they arrive via the push-constant block. With `img_w == img_h == 64` and
// `camera_mode == CAM_ORTHO` the marcher reproduces the golden 64×64 ORTHOGRAPHIC
// fixture BIT-EXACT (same extent → same `u`/`v` → same ray → same pixels → the
// rung-8..11 goldens are unchanged). `img_w == 0` (or `img_h == 0`) falls back to the
// legacy 64 default so an all-zero push tail is safe (the 0%-gate).
//
// The ORTHOGRAPHIC ray-gen arithmetic is the golden-frozen path: the PERSPECTIVE
// branch lives ENTIRELY inside `if (camera_mode == CAM_PERSPECTIVE) { ... }` and never
// touches the ortho `u`/`v`/`ro`/`rd` computation. The SDF field eval (`sdf`/`smin`/
// `combine`/normal) is BYTE-IDENTICAL — only ray GENERATION + the extent source change;
// perspective ray-gen feeds points into the SAME deterministic field eval (plain IEEE
// ops, no fast math / rsqrt / reordered FMA) so a perspective scene is reproducible.
//
// # The deterministic camera / depth convention (the single source of truth)
//
// The camera + light are mirrored EXACTLY by the host-side golden in `compute.rs`
// (`golden_composite_pixel`). The SDF math here is the same single source of truth
// rung 9 froze (a future CPU physics evaluator reuses the SAME field math).
//
//   Image:   img_w x img_h pixels (runtime extent, P0a), pixel index = py*img_w + px.
//   Camera (ORTHO, golden-frozen): looking down -Z at the origin (no perspective divide).
//            For pixel (px,py):
//              u =  ((px + 0.5) / img_w) * 2 - 1     // [-1, +1), +x right
//              v = -(((py + 0.5) / img_h) * 2 - 1)   // [-1, +1), +y up (flipped)
//            ray origin = (u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z)
//            ray dir    = (0, 0, -1)                 // straight down -Z
//   Camera (PERSPECTIVE, P0a additive): eye = cam_eye.xyz; per-pixel NDC in [-1,+1]
//            maps to a ray dir = normalize(forward + right*ndc_x*aspect*tan(fovY/2)
//            + up*ndc_y*tan(fovY/2)). Selected by camera_mode == 1; the ortho path is
//            untouched.
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

// The shared SDF field gateway (field consts/enums + `Edit`/`load_edit` + the
// primitive distances + boolean ops + smooth-min/-max + the edit-list `sdf` +
// `sdf_normal`). `Buf` (declared above) is the include contract precondition. This
// header also defines `FAR` and `GRAD_H`, so they are NOT redeclared below; and it
// defines `MAX_SDF_EDITS`/`SDF_EDIT_WORDS`/`HEADER_BASE`, so `DEPTH_BASE` and
// `pixel_base()` (which derive from them) are positioned AFTER this include.
#include "sdf_field.hlsli"

// Camera modes selected by `pc.camera_mode`. ORTHO is the golden-frozen path; the
// PERSPECTIVE branch is strictly additive (P0a part 2). Mirrored host-side in
// compute.rs as `CAM_MODE_ORTHO` / `CAM_MODE_PERSPECTIVE`.
static const uint CAM_ORTHO       = 0u;
static const uint CAM_PERSPECTIVE = 1u;

// The legacy fixture extent the golden invocation reproduces. Used as the fallback
// when `img_w`/`img_h` are zero (an all-zero push tail), and as the host const-assert
// anchor (`SDF_IMG_W`/`SDF_IMG_H` in compute.rs both equal this).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;

// The P0a push-constant block. Field offsets are pinned host-side by const-asserts
// (`COMPOSITE_PC_*_OFFSET` in compute.rs) so a host/shader desync is a build error.
// `count` stays at offset 0 (the legacy 4-byte field). Vector camera params use
// `float4` (16-byte slots) to avoid HLSL push-constant `float3` packing surprises;
// the ORTHO path ignores every camera field.
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
struct PushConstants {
    uint   count;
    uint   img_w;
    uint   img_h;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};
[[vk::push_constant]] PushConstants pc;

// Resolves the runtime extent, falling back to the legacy 64×64 fixture when a field
// is zero (so an all-zero push tail reproduces the golden — the 0%-gate).
uint img_w() { return (pc.img_w != 0u) ? pc.img_w : IMG_W_DEFAULT; }
uint img_h() { return (pc.img_h != 0u) ? pc.img_h : IMG_H_DEFAULT; }

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

// Sphere-trace tuning (the §S2 march budget; identical to rung 9).
static const float EPS    = 0.001;  // hit threshold on |sdf|
static const float T_MAX  = 10.0;   // miss distance bound (= depth-1.0 far plane)
static const uint  MAX_IT = 128u;   // max march steps per ray (the §S2 ceiling)

// The depth value the depth attachment was CLEARED to (the far plane, 1.0). A
// pixel whose stored depth is >= this sentinel had NO mesh fragment rasterized.
static const float DEPTH_CLEAR = 1.0;

// --- The depth/pixel region bases (derive from the header's field consts) -----
//
// `MAX_SDF_EDITS`/`SDF_EDIT_WORDS`/`HEADER_BASE` come from `sdf_field.hlsli`. The
// header is IDENTICAL to rung 9 up to the edit array; then a NEW depth region,
// then the pixel output. These MUST match the host-side rung-10 composite
// constants in `compute.rs` (pinned there by const-asserts).
static const uint DEPTH_BASE = HEADER_BASE + MAX_SDF_EDITS * SDF_EDIT_WORDS; // 4 + 192 = 196
// PIXEL_BASE now scales with the runtime extent: the depth region is `img_w*img_h`
// f32s, then the pixel region. At the golden 64×64 extent this is 196 + 4096 = 4292
// (unchanged). The host computes the matching offsets from the same extent.
uint pixel_base() { return DEPTH_BASE + img_w() * img_h(); }

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

    // Resolve the runtime extent (P0a part 1). At the golden invocation these equal
    // 64, so every downstream arithmetic reproduces the frozen ORTHO fixture.
    uint w = img_w();
    uint h = img_h();

    uint px = idx % w;
    uint py = idx / w;

    float3 ro;
    float3 rd;
    if (pc.camera_mode == CAM_PERSPECTIVE) {
        // P0a part 2: ADDITIVE perspective ray-gen, strictly inside this branch — the
        // ORTHO arithmetic below is byte-untouched. NDC in [-1,+1] (+x right, +y up,
        // y flipped to match the ortho convention); the ray direction is the camera
        // basis combined with the NDC scaled by the half-FOV tangent and aspect. Plain
        // IEEE ops (no rsqrt/rcp/fast-math) so a perspective scene is reproducible.
        float ndc_x =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float ndc_y = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        float tan_half_fov = pc.cam_forward.w; // tan(fovY / 2)
        float aspect       = pc.cam_right.w;   // W / H
        float3 dir = pc.cam_forward.xyz
                   + pc.cam_right.xyz * (ndc_x * aspect * tan_half_fov)
                   + pc.cam_up.xyz    * (ndc_y * tan_half_fov);
        ro = pc.cam_eye.xyz;
        rd = normalize(dir);
    } else {
        // Reconstruct the orthographic ray for this pixel (deterministic, golden-frozen).
        float u =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float v = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        ro = float3(u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z);
        rd = float3(0.0, 0.0, -1.0);
    }

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

    Buf[pixel_base() + idx] = pack_rgba(color);
}
