// Render P4b coarse-cull / tile pre-trace compute shader.
//
// A 1/8-res CONSERVATIVE cone-trace: one invocation per 8x8 fine-pixel tile emits a
// `TileBound{near_t, far_t, flags}` the fine marcher (`sdf_gbuffer_composite.hlsl`)
// reads to (a) early-out EMPTY tiles into the mesh/background composite and (b) seed
// `t = near_t` for non-empty tiles (skipping the proven-empty prefix). Algorithm A of
// docs/RENDER-P4-DESIGN.md (every decision pinned there). This shader is a strict
// FIELD-CONSUMER: it calls the frozen `field_distance` from `sdf_field.hlsli` and never
// edits the field math (the P4 determinism invariant).
//
// # Conservativeness (a hole = the worst bug — INVIOLABLE)
//
//   * D1 coarse_ray: the tile-center axis derived from the fine ray-gen's EXACT
//     arithmetic (`(px_c + 0.5) = tx*8 + 4.0`), so the host mirror `golden_tile_bound`
//     (compute.rs) and this shader emit identical ops.
//   * D2 ORTHO cone radius = sqrt(2)*(9/w)*HE (footprint-enclosing + 1 full pixel of
//     fp-ULP-safe margin).
//   * D3 PERSPECTIVE per-tile half-angle = max over the 4 corner OUTER-EDGE dirs of
//     acos(dot(d_center, d_corner)) + ALPHA_MARGIN.
//   * D4 cone-aware step `(d/L - r)/(1+tan(alpha_safe))` with the cone-entry rule:
//     when `d/L - r <= EPS_COARSE` RECORD near_t = t and STOP (do NOT over-step at
//     grazing). The `/L` corrects smin's super-Lipschitz under-report.
//   * D5 far_t = min(MAX over the 8x8 depth texels of depth->t, T_MAX); a cleared /
//     out-of-range texel -> T_MAX. MAX_IT_COARSE exhaustion -> NON-empty, near_t = 0.
//   * The proof (enclosure-with-margin + non-skipping step) is in the design.
//
// # The vocabulary set (set 0 — written ONCE at setup)
//
//   binding 0 : StructuredBuffer<uint>      (READ-ONLY) — the packed edit-list header
//               (the include contract: `Buf` MUST be in scope before the include).
//   binding 1 : Texture2D<float>            (SAMPLED)   — the mesh depth (D32_SFLOAT,
//               DEPTH-aspect), fetched per fine-pixel with `.Load` (OpImageFetch).
//   binding 6 : RWStructuredBuffer<TileBound> (STORAGE) — the per-tile output (u6).
//   binding 5 : cbuffer Camera              (UNIFORM)   — the 80-byte extent/camera
//               block (the SAME block the fine marcher reads, written once).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_tile_cull.hlsl -Fo sdf_tile_cull.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_tile_cull.comp.spv

StructuredBuffer<uint> Buf : register(t0); // binding 0: edit-list header (READ-ONLY)

// The shared determinism-frozen field gateway. INCLUDE CONTRACT: `Buf` (above) must be
// in scope first — the field eval reads the packed edit-list out of it. Brings in
// `field_distance(p)` + `FIELD_LIPSCHITZ_L`. Resolved relative to this .hlsl at DXC time.
#include "sdf_field.hlsli"

// binding 1: the mesh depth as a SAMPLED IMAGE (DEPTH-aspect view of the D32_SFLOAT
// rasterized image). `.Load(int3(px,py,0)).r` is an unfiltered fetch (no sampler).
Texture2D<float> gDepth : register(t1);

// One per-tile cull bound, byte-identical to the host `#[repr(C)] TileBound` (16 B,
// std430 scalar layout: near_t@0, far_t@4, flags@8, _pad@12). Mirrored host-side in
// compute.rs (the offsets are const-asserted there).
struct TileBound {
    float near_t;
    float far_t;
    uint  flags;
    uint  _pad;
};

// binding 6: the per-tile output. One element per tile (`tiles_w * tiles_h`), written
// disjointly by each invocation (race-free).
RWStructuredBuffer<TileBound> Tiles : register(u6);

// --- Mirrored constants (host: compute.rs / sdf_gbuffer_composite.hlsl) -------------
static const uint CAM_ORTHO       = 0u;
static const uint CAM_PERSPECTIVE = 1u;

static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;

// binding 5: the extent/camera UNIFORM block (the SAME 80-byte block the fine marcher
// reads; written ONCE at setup). Field layout identical to `CompositePushConstants`.
cbuffer Camera : register(b5) {
    uint   count;        // total PIXEL count (img_w * img_h) — unused here (tile loop)
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;  // xyz = forward, w = tan(fovY/2)
    float4 cam_right;    // xyz = right,   w = aspect (W/H)
    float4 cam_up;
};

uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// --- Scene / march constants (mirrored host-side — identical to the fine marcher) ----
static const float CAM_Z        = 2.0;
static const float HALF_EXTENT  = 1.0;
static const float T_MAX        = 10.0;
static const float DEPTH_CLEAR  = 1.0;

// P4b coarse-cull tuning (mirrored host-side as compute.rs consts).
static const uint  TILE_SIZE       = 8u;
static const uint  MAX_IT_COARSE   = 64u;
static const float EPS_COARSE      = 0.001;
static const float ALPHA_MARGIN    = 1e-4;
static const uint  TILE_FLAG_EMPTY = 1u;

// The exact perspective ray direction for a (fractional) pixel whose `(px + 0.5)`
// sample is `sxp` and `(py + 0.5)` is `syp`, normalized — the SAME op sequence as the
// fine marcher's perspective ray-gen + `composite_ray` (the determinism boundary).
float3 persp_dir(float sxp, float syp, uint w, uint h, float tan_half_fov, float aspect) {
    float ndc_x =  (sxp / (float)w) * 2.0 - 1.0;
    float ndc_y = -((syp / (float)h) * 2.0 - 1.0);
    float3 dir = cam_forward.xyz
               + cam_right.xyz * (ndc_x * aspect * tan_half_fov)
               + cam_up.xyz    * (ndc_y * tan_half_fov);
    return normalize(dir);
}

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint w = img_w();
    uint h = img_h();
    uint tiles_w = (w + TILE_SIZE - 1u) / TILE_SIZE;
    uint tiles_h = (h + TILE_SIZE - 1u) / TILE_SIZE;
    uint tile_count = tiles_w * tiles_h;

    uint i = tid.x;
    if (i >= tile_count) {
        return;
    }
    uint tx = i % tiles_w;
    uint ty = i / tiles_w;

    // --- D1: the coarse ray through the tile's TRUE geometric center. `(px_c + 0.5)`
    // and `(py_c + 0.5)` are `tx*8 + 4.0` / `ty*8 + 4.0` (3.5 + 0.5 = 4.0 exact). ---
    float cx = (float)(tx * TILE_SIZE) + 4.0;
    float cy = (float)(ty * TILE_SIZE) + 4.0;

    float3 ro;
    float3 rd;
    float  r_const = 0.0;   // ORTHO constant cone radius (D2).
    float  tan_a   = 0.0;   // PERSPECTIVE cone slope tan(alpha_safe) (D3); ORTHO -> 0.
    if (camera_mode == CAM_PERSPECTIVE) {
        float tan_half_fov = cam_forward.w;
        float aspect       = cam_right.w;
        ro = cam_eye.xyz;
        rd = persp_dir(cx, cy, w, h, tan_half_fov, aspect);

        // D3: per-tile half-angle = max over the 4 corner OUTER-EDGE dirs of
        // acos(dot(d_center, d_corner)) + ALPHA_MARGIN. Outer edges: `(px + 0.5)` at
        // `tx*8 + 0.0` and `tx*8 + 8.0` (the footprint's outermost samples).
        float lo_x = (float)(tx * TILE_SIZE);
        float hi_x = (float)(tx * TILE_SIZE) + (float)TILE_SIZE;
        float lo_y = (float)(ty * TILE_SIZE);
        float hi_y = (float)(ty * TILE_SIZE) + (float)TILE_SIZE;
        float max_angle = 0.0;
        // The 4 corner directions, unrolled (no array of float3 — DXC friendliness).
        float3 c0 = persp_dir(lo_x, lo_y, w, h, tan_half_fov, aspect);
        float3 c1 = persp_dir(hi_x, lo_y, w, h, tan_half_fov, aspect);
        float3 c2 = persp_dir(lo_x, hi_y, w, h, tan_half_fov, aspect);
        float3 c3 = persp_dir(hi_x, hi_y, w, h, tan_half_fov, aspect);
        max_angle = max(max_angle, acos(clamp(dot(rd, c0), -1.0, 1.0)));
        max_angle = max(max_angle, acos(clamp(dot(rd, c1), -1.0, 1.0)));
        max_angle = max(max_angle, acos(clamp(dot(rd, c2), -1.0, 1.0)));
        max_angle = max(max_angle, acos(clamp(dot(rd, c3), -1.0, 1.0)));
        float alpha_safe = max_angle + ALPHA_MARGIN;
        tan_a = tan(alpha_safe);
    } else {
        // ORTHO: a constant-radius cylinder, parallel rays (D2). The radius uses the
        // LARGER world pixel pitch `min(w,h)` so a non-square ortho extent stays
        // conservative (byte-identical to `(9/w)` at the square golden where w == h).
        float u =  (cx / (float)w) * 2.0 - 1.0;
        float v = -((cy / (float)h) * 2.0 - 1.0);
        ro = float3(u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z);
        rd = float3(0.0, 0.0, -1.0);
        float min_wh = (float)min(w, h);
        r_const = 1.41421356 * (9.0 / min_wh) * HALF_EXTENT; // sqrt(2)*(9/min(w,h))*HE
    }

    // --- D5: far_t = min(MAX over the 8x8 depth texels of depth->t, T_MAX). A cleared
    // (>= DEPTH_CLEAR) or out-of-range texel decodes to T_MAX (conservative). ---
    float max_t_mesh = 0.0;
    [loop]
    for (uint dy = 0u; dy < TILE_SIZE; ++dy) {
        for (uint dx = 0u; dx < TILE_SIZE; ++dx) {
            uint px = tx * TILE_SIZE + dx;
            uint py = ty * TILE_SIZE + dy;
            float t_mesh;
            if (px >= w || py >= h) {
                t_mesh = T_MAX; // out-of-range (partial-edge tile): conservative T_MAX.
            } else {
                float md = gDepth.Load(int3((int)px, (int)py, 0)).r;
                t_mesh = (md < DEPTH_CLEAR) ? (md * T_MAX) : T_MAX;
            }
            max_t_mesh = max(max_t_mesh, t_mesh);
        }
    }
    float far_t = min(max_t_mesh, T_MAX);

    // --- D4: the cone-aware march. ---
    float t = 0.0;
    float near_t = 0.0;
    bool  entered = false;
    bool  exhausted = true; // cleared on a cone-entry / far_t / T_MAX break -> EMPTY/hit.
    [loop]
    for (uint it = 0u; it < MAX_IT_COARSE; ++it) {
        if (t >= far_t) {
            exhausted = false; // reached far_t without entering: EMPTY.
            break;
        }
        float3 p = ro + rd * t;
        float d = field_distance(p);
        float r = r_const + t * tan_a;            // ortho: r_const; persp: t*tan(alpha).
        float budget = d / FIELD_LIPSCHITZ_L - r;  // the cone clearance.
        if (budget <= EPS_COARSE) {
            near_t = t;                            // cone-entry: RECORD + STOP (D4).
            entered = true;
            exhausted = false;
            break;
        }
        t += budget / (1.0 + tan_a);               // ortho: /(1+0); persp: /(1+tan).
        if (t > T_MAX) {
            exhausted = false; // walked past T_MAX without entering: EMPTY.
            break;
        }
    }

    TileBound tb;
    tb._pad = 0u;
    if (entered) {
        // near_t in [0, far_t]; entered implies t >= 0 already.
        tb.near_t = clamp(near_t, 0.0, far_t);
        tb.far_t  = far_t;
        tb.flags  = 0u;
    } else if (exhausted) {
        // MAX_IT_COARSE exhaustion -> NON-empty, near_t = 0 (safe full-march fallback).
        tb.near_t = 0.0;
        tb.far_t  = far_t;
        tb.flags  = 0u;
    } else {
        // Reached far_t / T_MAX without cone-entry -> EMPTY (near_t = 0).
        tb.near_t = 0.0;
        tb.far_t  = far_t;
        tb.flags  = TILE_FLAG_EMPTY;
    }
    Tiles[i] = tb;
}
