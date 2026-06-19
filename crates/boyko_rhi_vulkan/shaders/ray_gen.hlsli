// Shared camera ray-generation (`ray_gen.hlsli`) — PBR MVP-2 Phase 0 extraction.
//
// The ORTHO + PERSPECTIVE ray-gen was previously inlined in the marcher
// (`sdf_gbuffer_composite.hlsl`). It is extracted here VERBATIM so the marcher and
// the deferred PBR resolve (`deferred_pbr.hlsl`) — which reconstructs the per-pixel
// view direction for the BRDF — share ONE ray-gen and can never drift.
//
// # DETERMINISM (the marcher path)
//
// The arithmetic below is CHARACTER-IDENTICAL to the marcher's pre-extraction
// inline ray-gen (the `u`/`v`/`ro`/`rd` ORTHO block and the NDC/basis PERSPECTIVE
// block). Plain IEEE ops only (no fast-math, no `rsqrt`/`rcp`) so the marcher's hit
// point — which feeds the FROZEN field via the shading consumers — is bit-unchanged
// and the host golden (`composite_ray` in compute.rs) still predicts it. The
// extraction is value-preserving; the GATE-1 field tripwire + the distance/depth
// golden prove the marcher's field eval is undisturbed.
//
// # The camera parameter block
//
// Both including TUs declare the SAME 80-byte `cbuffer Camera` (count/img_w/img_h/
// camera_mode + 4 `float4` basis vectors). To stay binding-agnostic this header takes
// the resolved camera fields as PLAIN PARAMETERS rather than reading a global cbuffer,
// so it can be `#include`d after either TU's `cbuffer Camera` declaration. The
// including TU passes the cbuffer members straight through.

// Camera modes (mirror compute.rs CAM_MODE_ORTHO / CAM_MODE_PERSPECTIVE).
static const uint RAYGEN_CAM_ORTHO       = 0u;
static const uint RAYGEN_CAM_PERSPECTIVE = 1u;

// Deterministic ORTHO scene constants (mirrored host-side in compute.rs). The marcher
// previously declared these locally; they live here now since ray-gen owns them.
static const float RAYGEN_CAM_Z       = 2.0; // camera plane Z (ortho rays start here)
static const float RAYGEN_HALF_EXTENT = 1.0; // orthographic view half-extent (world units)

// Reconstructs the (ro, rd) ray for pixel (px, py) at extent (w, h) under the camera.
//
//   camera_mode  : RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE
//   cam_eye_xyz  : perspective eye world position (ORTHO ignores it)
//   cam_fwd      : float4 — xyz = forward basis, w = tan(fovY/2)   (PERSPECTIVE)
//   cam_right    : float4 — xyz = right basis,   w = aspect (W/H)   (PERSPECTIVE)
//   cam_up_xyz   : up basis                                          (PERSPECTIVE)
//
// `out ro` / `out rd` receive the ray origin + (PERSPECTIVE: normalized) direction.
void generate_ray(
    uint   px, uint py, uint w, uint h,
    uint   camera_mode,
    float3 cam_eye_xyz,
    float4 cam_fwd,
    float4 cam_right,
    float3 cam_up_xyz,
    out float3 ro,
    out float3 rd
) {
    if (camera_mode == RAYGEN_CAM_PERSPECTIVE) {
        // ADDITIVE perspective ray-gen. NDC in [-1,+1] (+x right, +y up, y flipped to
        // match the ortho convention); the direction is the camera basis combined with
        // the NDC scaled by the half-FOV tangent and aspect. Plain IEEE ops (no
        // rsqrt/rcp/fast-math) so a perspective scene is reproducible.
        float ndc_x =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float ndc_y = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        float tan_half_fov = cam_fwd.w;   // tan(fovY / 2)
        float aspect       = cam_right.w; // W / H
        float3 dir = cam_fwd.xyz
                   + cam_right.xyz * (ndc_x * aspect * tan_half_fov)
                   + cam_up_xyz    * (ndc_y * tan_half_fov);
        ro = cam_eye_xyz;
        rd = normalize(dir);
    } else {
        // Reconstruct the orthographic ray for this pixel (deterministic, golden-frozen).
        float u =  (((float)px + 0.5) / (float)w) * 2.0 - 1.0;
        float v = -((((float)py + 0.5) / (float)h) * 2.0 - 1.0);
        ro = float3(u * RAYGEN_HALF_EXTENT, v * RAYGEN_HALF_EXTENT, RAYGEN_CAM_Z);
        rd = float3(0.0, 0.0, -1.0);
    }
}
