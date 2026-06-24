// Render P1a GPU gate compute shader: sphere-trace an ORDERED SDF EDIT-LIST and
// STORE the marcher color into a STORAGE IMAGE through the multi-resource
// descriptor *vocabulary* set (the one genuinely-new P1a capability).
//
// This is a verbatim derivative of the rung-9 `sdf_editlist.hlsl` field eval +
// ray-gen + lighting: the field math (primitive distances, boolean ops,
// smooth-min, central-difference gradient), the orthographic camera, the
// directional Lambert+ambient light, and the deterministic scene constants are
// reused LINE-FOR-LINE so the host golden (`golden_editlist_pixel`) predicts the
// output unchanged. The ONLY differences vs rung 9 are the two bind points and
// the output sink:
//
//   * binding 0 (set 0) — `StructuredBuffer<uint>` (READ-ONLY): the edit-list
//     packed-header region (the existing `sdf_editlist` format: word 0 =
//     edit_count, then MAX_SDF_EDITS * SdfEdit). The shader only READS it, so it
//     is a plain `StructuredBuffer` (not `RWStructuredBuffer`); the host binds it
//     as a STORAGE_BUFFER descriptor (DescriptorKind::StorageBuffer) — a storage
//     buffer is read/write-capable, and reading through it is valid.
//   * binding 1 (set 0) — `RWTexture2D<float4>` (WRITE): the R8G8B8A8_UNORM
//     storage image (DescriptorKind::StorageImage). The marcher color, previously
//     PACKED into the buffer's pixel region (`Buf[PIXEL_BASE + idx]`), is instead
//     STORED to texel (px, py) as `float4(rgb, 1.0)`. The float->UNORM store
//     quantization vs the host `pack_rgba`'s `(x*255+0.5)` rounding is absorbed by
//     the test's +/-2/255 per-channel tolerance (same tolerance as rung 8/9).
//
// There is NO packed pixel region and NO depth region here — the buffer is just
// the edit-list header (EDITLIST_BUFFER_WORDS reuses the rung-9 layout; the shader
// never touches words >= PIXEL_BASE).
//
// # No push constant on the vocabulary path (review O1)
//
// The encoder's `push_constants` records against the device-shared compute pipeline
// layout, NOT a vocabulary pipeline's dedicated layout, so "P1a wires no push on the
// vocabulary path." This shader therefore takes NO push constant: the bound check
// uses the statically-known fixed extent (`IMG_W * IMG_H`) instead of a pushed
// `count`. The dispatch still covers exactly `ceil(IMG_W*IMG_H / 64)` groups, and
// the last group's tail invocations (idx >= IMG_W*IMG_H) early-out. (The vocabulary
// pipeline layout still declares a push range — declaring an unused range is valid
// Vulkan — but nothing records a push against it.)
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 sdf_editlist_storage_image.hlsl \
//       -Fo sdf_editlist_storage_image.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe sdf_editlist_storage_image.comp.spv

StructuredBuffer<uint>   Buf : register(t0); // binding 0: edit-list (READ-ONLY)
// binding 1: marcher output (WRITE). The `[[vk::image_format("rgba8")]]` qualifier
// pins the SPIR-V `OpTypeImage` format to `Rgba8` so it MATCHES the R8G8B8A8_UNORM
// image view — without it DXC defaults a typed `RWTexture2D<float4>` to `Rgba32f`,
// which the validation layer flags as a storage-image format mismatch (the store
// would produce undefined values across the whole image).
[[vk::image_format("rgba8")]]
RWTexture2D<float4>      Img : register(u1);

// The shared SDF field gateway (field consts/enums + `Edit`/`load_edit` + the
// primitive distances + boolean ops + smooth-min/-max + the edit-list `sdf` +
// `sdf_normal`). `Buf` (declared above) is the include contract precondition. This
// header also defines `FAR` and `GRAD_H`, so they are NOT redeclared below.
#include "sdf_field.hlsli"

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

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    // No push constant on the vocabulary path: bound by the statically-known extent.
    if (idx >= IMG_W * IMG_H) {
        return;
    }

    uint px = idx % IMG_W;
    uint py = idx / IMG_W;

    // Reconstruct the orthographic ray for this pixel (deterministic, rung-9 verbatim).
    float u =  (((float)px + 0.5) / (float)IMG_W) * 2.0 - 1.0;
    float v = -((((float)py + 0.5) / (float)IMG_H) * 2.0 - 1.0);
    float3 ro = float3(u * HALF_EXTENT, v * HALF_EXTENT, CAM_Z);
    float3 rd = float3(0.0, 0.0, -1.0);

    // Sphere-trace the folded edit-list field.
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

    // STORE the marcher color into the storage image (the one new P1a sink). The
    // R8G8B8A8_UNORM store quantizes `clamp(color,0,1)` to bytes; the host golden's
    // `pack_rgba` uses `(x*255+0.5)` rounding — the +/-2/255 tolerance absorbs the
    // <=1-LSB difference between the two quantizations (same tolerance as rung 8/9).
    Img[uint2(px, py)] = float4(clamp(color, 0.0, 1.0), 1.0);
}
