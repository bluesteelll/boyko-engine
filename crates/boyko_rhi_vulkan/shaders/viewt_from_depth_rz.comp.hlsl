// TAA-under-VB: the gViewT PRODUCER for the VisibilityBuffer x Mesh path — the REVERSE-Z
// sibling of `viewt_from_depth.comp.hlsl` (the Deferred x Mesh producer, which decodes the
// Deferred CUSTOM-LINEAR depth encode and therefore CANNOT be reused here).
//
// VB's `vb_depth` is standard HARDWARE reverse-Z (`forward_view_proj_rows`'s encode:
// `depth(view_z) = A + B / view_z`, GREATER test, 0.0 clear — see that fn's doc for the
// A/B derivation). The unchanged `taa_resolve.comp.hlsl` reconstructs the surface as
// `P = ro + rd * view_t` from its `generate_ray` family, so this pass converts the raster
// depth into that SAME ray parameterization:
//
//   view_z = B / (d - A)                      // the exact encode inverse
//   t      = view_z / dot(cam_forward, rd)    // sound because ro == cam_eye
//
// The decode is PROVEN IN-TREE: `sdf_forward_march.comp.hlsl`'s HAS_MESH variant performs the
// identical two lines to bound its march against the rasterized mesh surface (the
// `forward_view_z_coeffs` consumer). `A`/`B` arrive PRECOMPUTED host-side (the SAME
// single-sourced Rust `forward_view_z_coeffs(near, far)` that feeds the marcher) as push
// constants — no third hand-written copy of the encode.
//
// Ray consistency: the `Camera` cbuffer below is the SAME b5 camera-ring slot the TAA
// resolve's own `generate_ray` reads (`scene.camera_ring[fi]`), and `ray_gen.hlsli` is the
// SAME verbatim include — producer `t` and consumer `P = ro + rd*t` use bitwise-identical
// rays by construction (under C1's `RasterAndBasis` the ring basis may be SHEARED; both
// sides then see the same sheared family, which the C2 differential reprojection tolerates —
// `taa_resolve.comp.hlsl`'s binding-6 doc).
//
// # Resources (dedicated 3-binding bind-group; the `viewt_from_depth` 2-binding precedent
//   plus the camera UBO this reverse-Z ray parameterization needs)
//
//   binding 0 : Texture2D<float> (SAMPLED) — vb_depth (DEPTH-aspect view of the shared
//               D32_SFLOAT `ForwardTargets::depth[fi]`, `.Load` / OpImageFetch, no sampler).
//   binding 1 : RWTexture2D<float> (STORAGE, r32f) — gViewT (WRITE; every dispatched pixel
//               written exactly once — full-screen pass).
//   binding 2 : cbuffer Camera — the 80-byte b5 camera ring slot (byte-identical layout to
//               the marcher / resolve / SSAO / TAA-resolve `Camera`).
//
// A 16-byte `[[vk::push_constant]]` block (`ViewtFromDepthRzPush`): `img_w`/`img_h` (the
// bounds guard for the ceil(w/8) x ceil(h/8) dispatch grid) + `view_z_a`/`view_z_b` (the
// host-precomputed reverse-Z coefficients above).
//
// Compiled offline (hermetic — no SDK at `cargo build` time) with:
//   dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       viewt_from_depth_rz.comp.hlsl -Fo viewt_from_depth_rz.comp.spv

Texture2D<float> gDepth : register(t0);
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u1);

// The 80-byte camera block — byte-identical field layout to `taa_resolve.comp.hlsl`'s b6
// `Camera` (the SAME ring slot is bound here at binding 2).
cbuffer Camera : register(b2) {
    uint   count;        // total pixel count = img_w * img_h (UNREAD here; layout parity)
    uint   img_w_raw;    // runtime extent width  (UNREAD here; the push carries the extent)
    uint   img_h_raw;    // runtime extent height (UNREAD here)
    uint   camera_mode;  // RAYGEN_CAM_ORTHO | RAYGEN_CAM_PERSPECTIVE (see ray_gen.hlsli)
    float4 cam_eye;
    float4 cam_forward;  // xyz = forward basis, w = tan(fovY/2)
    float4 cam_right;    // xyz = right basis,   w = aspect (W/H)
    float4 cam_up;
};

// The shared camera ray-gen (VERBATIM include — the marcher / TAA-resolve precedent; the
// header takes the camera fields as plain parameters, composing after `cbuffer Camera`).
#include "ray_gen.hlsli"

struct ViewtFromDepthRzPush {
    uint  img_w;
    uint  img_h;
    float view_z_a; // forward_view_z_coeffs(near, far).a — the reverse-Z encode's A
    float view_z_b; // forward_view_z_coeffs(near, far).b — the reverse-Z encode's B
};
[[vk::push_constant]] ViewtFromDepthRzPush pc;

// The mesh/SDF G-buffer background sentinel (mirrors the marcher's `gViewT` `1.0e30` and
// `viewt_from_depth.comp.hlsl`'s own `VIEWT_BG`): background pixels reproject the
// point-at-infinity `(rd, 0)` inside the TAA resolve, exactly as under Deferred.
static const float VIEWT_BG = 1.0e30;

// Full-screen, 8x8 tiles (the `viewt_from_depth` dispatch shape: `ceil(img_w/8) x
// ceil(img_h/8)` groups; the bounds guard discards the edge-group overhang).
[numthreads(8, 8, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    if (tid.x >= pc.img_w || tid.y >= pc.img_h) {
        return;
    }
    uint px = tid.x;
    uint py = tid.y;

    // Reverse-Z occupancy: the depth CLEAR is 0.0 ("nothing drawn", farther than any real
    // fragment under GREATER), so any d > 0.0 is a rasterized mesh surface.
    float d = gDepth.Load(int3((int)px, (int)py, 0)).r;
    bool has_mesh = (d > 0.0);

    // The exact encode inverse + the ray reparameterization (the proven
    // `sdf_forward_march.comp.hlsl` HAS_MESH decode, token-for-token):
    //   view_z = B / (d - A);  t = view_z / dot(cam_forward, rd)   (ro == cam_eye).
    float3 ro;
    float3 rd;
    generate_ray(px, py, pc.img_w, pc.img_h, camera_mode,
                 cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
    float view_z = pc.view_z_b / (d - pc.view_z_a);
    float t_mesh = view_z / dot(cam_forward.xyz, rd);

    gViewT[uint2(px, py)] = has_mesh ? t_mesh : VIEWT_BG;
}
