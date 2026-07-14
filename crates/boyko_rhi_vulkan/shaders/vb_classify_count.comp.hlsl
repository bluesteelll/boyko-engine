// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
// unwired). The `count` compute pass ("The pipeline" step 2): one thread per composite pixel,
// `InterlockedAdd(counts[mat], 1)` for every mesh-covered (non-SENTINEL) pixel's material id —
// the first of the three classify-family full-screen passes (count -> scan -> scatter), D1's
// "full-screen per-material bins, not tiled" choice.
//
// # Sentinel handling
//
// Mirrors `vb_resolve.comp.hlsl`'s own sentinel gate: `vb_id_unpack(...).instance_id ==
// VB_ID_SENTINEL` (the sky background / an SDF-owned pixel `vb_raster` never covered) is
// skipped -- it contributes to no material's count.
//
// # Bindings (Set 0 = `vb_layout0` only -- a 1-set pipeline, `create_compute_pipeline`,
// plan P2-1: no dedicated `_vb1` helper)
//
//   t1 : StructuredBuffer<PerInstanceMaterial> instance_materials  (SAME shape as
//        `vb_resolve.comp.hlsl`'s own binding 1 -- reads only `.id`)
//   b2 : cbuffer Camera                                            (`img_w_raw`/`img_h_raw`)
//   t5 : Texture2D<uint2>                       gVbId               (the `vb_raster` output)
//   u7 : RWByteAddressBuffer                    gClassify           (via
//        `vb_classify_common.hlsli` -- `cls_count_add`)
//
// Compiled offline (hermetic build -- no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_classify_count.comp.hlsl -Fo vb_classify_count.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe vb_classify_count.comp.spv

#include "vb_pack.hlsli"

// binding 1: the per-instance material payload -- byte-identical shape to
// `vb_resolve.comp.hlsl`'s own declaration (this pass reads only `.id`).
struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;

// binding 2: the extent/camera UNIFORM block -- the SAME 80-byte shape every VB-family
// consumer declares (`vb_resolve.comp.hlsl`'s own binding 2).
[[vk::binding(2, 0)]] cbuffer Camera {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};

// binding 5: the `vb_id` raster output -- byte-identical declaration to
// `vb_resolve.comp.hlsl`'s own binding 5.
Texture2D<uint2> gVbId : register(t5);

#include "vb_classify_common.hlsli"

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;
    uint w = img_w_raw;
    uint h = img_h_raw;
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

    uint2 packed = gVbId.Load(int3((int)px, (int)py, 0));
    VbId id = vb_id_unpack(packed);
    if (id.instance_id == VB_ID_SENTINEL) {
        return;
    }

    uint mat = instance_materials[id.instance_id].id;
    cls_count_add(mat);
}
