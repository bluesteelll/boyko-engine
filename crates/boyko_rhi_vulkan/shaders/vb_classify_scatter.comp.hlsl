// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
// unwired). The `scatter` compute pass ("The pipeline" step 4): one thread per composite pixel
// -- IDENTICAL sentinel/material-lookup prologue to `vb_classify_count.comp.hlsl` (see that
// file's doc) -- claims a slot in its material's `pixel_list` region
// (`InterlockedAdd(cursors[mat], 1)`, `cls_scatter`) and stores the pixel's LINEAR index
// (`py*w+px`) there. Runs AFTER `scan` has turned `cursors[mat]` into each material's region
// START (`vb_classify_scan.comp.hlsl`'s Phase 1) -- the SAME `offsets[mat]`-seeded cursor
// `scan` wrote, now walked forward by this pass's per-pixel atomics (P1-3: the framegraph must
// barrier `scan`'s writes visible to this pass's reads, verified in rung P2b -- this file's own
// correctness does not depend on HOW that barrier is emitted, only that it exists by the time
// this dispatch runs).
//
// # Bindings (Set 0 = `vb_layout0` only -- 1-set pipeline, `create_compute_pipeline`)
//
//   t1 : StructuredBuffer<PerInstanceMaterial> instance_materials
//   b2 : cbuffer Camera                                            (`img_w_raw`/`img_h_raw`)
//   t5 : Texture2D<uint2>                       gVbId
//   u7 : RWByteAddressBuffer                    gClassify           (via
//        `vb_classify_common.hlsli` -- `cls_scatter`/`cls_pixel_store`)
//
// Compiled offline: see `vb_classify_count.comp.hlsl`'s header for the dxc/spirv-val
// invocation (identical flags, this file's own name substituted).

#include "vb_pack.hlsli"

struct PerInstanceMaterial {
    float4 base_color;
    uint   id;
    uint3  _pad;
};
[[vk::binding(1, 0)]] StructuredBuffer<PerInstanceMaterial> instance_materials;

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
    uint slot = cls_scatter(mat);
    cls_pixel_store(slot, py * w + px, w, h);
}
