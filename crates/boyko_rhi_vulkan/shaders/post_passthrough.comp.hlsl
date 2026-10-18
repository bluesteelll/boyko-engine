// post_passthrough.comp -- render wave PX1a (px1a/cut.md D-S4): the TRANSITIONAL kernel the
// `post_final` pass dispatches between the plumbing commit and the switch commit. It copies the
// frame's scene input (`taa_hist[fi]` under TAA, `lit_native` under SSAA, `lit[fi]` otherwise) into
// `display` unchanged, so the post seam's new passes, targets and barriers land with no pixel
// moving (GL-PX1a-S2's byte-identity sweep). The bindings and `numthreads` are post_final.comp's,
// so the same layout, set ring and dispatch serve both; the generated post_final.comp replaces this
// kernel when `lit` turns HDR, and this file, its .spv, its manifest row and its sync line are
// deleted then. Hand-written on purpose: a pass-through mode word in the generated body would move
// post_final.comp.spv and its stage census (the body stays stage-only).
//
// # Compile (offline + hermetic; the committed .spv is byte-gated by post_chain_spv_sync.rs)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 post_passthrough.comp.hlsl -Fo post_passthrough.comp.spv

[[vk::binding(0, 0)]] Texture2D<float4> gScene;
[[vk::binding(1, 0)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> gDisplay;
[[vk::binding(2, 0)]] cbuffer PostFinal {
    uint mode_words; // offset 0 (unread here)
    float exposure_ratio; // offset 4 (unread here)
    uint2 extent; // offset 8
    uint _r0; // offset 16
    uint _r1; // offset 20
    uint _r2; // offset 24
    uint _r3; // offset 28
};

[numthreads(8, 8, 1)]
void main(uint3 dtid : SV_DispatchThreadID) {
    if (any(dtid.xy >= extent)) {
        return;
    }
    gDisplay[dtid.xy] = float4(gScene.Load(int3(dtid.xy, 0)).rgb, 1.0);
}
