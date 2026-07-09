// GUI P5a RUNG 0.5 (de-risk) vertex shader: a vertexless unit quad whose corner
// positions come from SV_VertexID and whose per-instance transform is read from a
// StructuredBuffer<RungInstance> by SV_InstanceID — IN THE VERTEX STAGE.
//
// This proves the NEVER-BEFORE-EXERCISED combination: a GRAPHICS pipeline binding a
// STORAGE buffer (set 0, binding 0) visible at VERTEX|FRAGMENT, indexed by
// SV_InstanceID. Every prior SSBO bind in this engine is on the COMPUTE bind point;
// this isolates a backend stage-flag / descriptor-type mismatch BEFORE the full SDF
// + blend complexity of the UI pipeline.
//
// The record is read in BOTH stages: the VS reads min_px/size_px to place the quad
// (so a VS-stage SSBO read is genuinely exercised), the FS reads color. The quad is
// transformed into NDC by a 2D ortho passed as a push constant, so the offscreen
// golden can place the rect at a known sub-region and assert exact texels.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 ssbo_quad.vs.hlsl -Fo ssbo_quad.vs.spv

// The per-instance record. Mirrors the Rust `RungInstance` (#[repr(C)], std430):
//   min_px : float2  @ 0
//   size_px: float2  @ 8
//   color  : float4  @ 16
// std430 array stride = 32 (float4 forces 16-align; 32 % 16 == 0, no tail pad).
struct RungInstance {
    float2 min_px;
    float2 size_px;
    float4 color;
};

// set 0, binding 0: the per-instance STORAGE buffer, read in the VERTEX stage.
[[vk::binding(0, 0)]] StructuredBuffer<RungInstance> g_instances : register(t0);

// A 2D ortho (pixel -> NDC, top-left origin via the negative-y scale). 16 bytes.
struct Ortho {
    float2 scale;
    float2 translate;
};
[[vk::push_constant]] Ortho g_ortho;

// The six unit-quad corners (two triangles) generated from SV_VertexID.
static const float2 CORNERS[6] = {
    float2(0.0, 0.0),
    float2(1.0, 0.0),
    float2(0.0, 1.0),
    float2(0.0, 1.0),
    float2(1.0, 0.0),
    float2(1.0, 1.0),
};

struct VsOut {
    float4 position : SV_Position;
    nointerpolation uint inst_index : INSTANCE;
};

VsOut main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    RungInstance inst = g_instances[iid];          // SSBO read in the VERTEX stage
    float2 pos_px = inst.min_px + CORNERS[vid] * inst.size_px;

    VsOut o;
    o.position = float4(pos_px * g_ortho.scale + g_ortho.translate, 0.0, 1.0);
    o.inst_index = iid;
    return o;
}
