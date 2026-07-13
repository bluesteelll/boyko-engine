// Multi-paradigm render-path plan, rung R8: the VisibilityBuffer mesh raster FRAGMENT shader.
// Writes ONLY `SV_Target0 = uint2(global_instance_id, raw SV_PrimitiveID)` (Decision 9 / plan
// §F) into the `vb_id` `R32G32_UINT` color attachment — NO `SV_Depth`, NO `discard`, NO UAV, so
// hardware early-Z (against the reverse-Z `depth` image `vb_raster.vs.hlsl` writes) stays live,
// the SAME Decision-4 early-Z-clean contract every other raster FS in this codebase follows.
//
// `global_instance_id` arrives as `vb_raster.vs.hlsl`'s flat `IID` interpolant (Decision 9: no
// FS-side `SV_InstanceID` read); `raw_prim_id` is the rasterizer-provided `SV_PrimitiveID`
// system value — a pixel-shader-only input needing no VS export at all. The in-mesh triangle
// index is recovered downstream, in the compute fetch (`vb_geom_fetch.hlsli`), as
// `raw_prim_id % gMeshMeta[mesh_id].tri_count` — correct under either `SV_PrimitiveID`
// per-instance semantics (Decision 9), since every instance of one `DrawBatch` shares one index
// buffer / `tri_count`.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 vb_raster.fs.hlsl -Fo vb_raster.fs.spv

struct PsIn {
    float4 position : SV_Position;
    nointerpolation uint instance_id : IID;
};

uint2 main(PsIn input, uint raw_prim_id : SV_PrimitiveID) : SV_Target0 {
    return uint2(input.instance_id, raw_prim_id);
}
