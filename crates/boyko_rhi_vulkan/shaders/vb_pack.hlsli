// Multi-paradigm render-path plan, rung R8 (Decision 9 / plan §F): the Visibility-Buffer
// id-channel pack/unpack convention shared by `vb_raster.fs.hlsl` (the writer) and
// `vb_geom_fetch.hlsli` (the reader).
//
// `vb_id` (R32G32_UINT):
//   R = instance_id = base_instance + SV_InstanceID  -- the key that ALSO addresses the
//       instance SSBO (`VbInstanceRow`), the per-instance material ring, and — via the
//       instance row's own `mesh_id` lane — the Decision-0 geometry table.
//   G = triangle_id  = the RAW rasterizer-provided `SV_PrimitiveID` (a system value, no VS
//       export). The in-mesh triangle is recovered downstream as `raw % tri_count`
//       (`vb_geom_fetch.hlsli`), which is correct under EITHER possible `SV_PrimitiveID`
//       per-instance semantics (Decision 9) — every instance of one `DrawBatch` shares one
//       index buffer, so `tri_count` is the same modulus regardless.
//
// `VB_ID_SENTINEL` marks a pixel the mesh raster leg never covered (the SDF leg's own hit, or
// the sky background) — host mirror: `boyko_render::render_path_config::VB_ID_SENTINEL`. A
// real per-frame `instance_id` never reaches `0xFFFFFFFF` (the instance ring is bounded far
// below `u32::MAX`), so the two domains cannot collide.
static const uint VB_ID_SENTINEL = 0xFFFFFFFFu;

/// One `vb_id` texel, unpacked.
struct VbId {
    uint instance_id;
    uint raw_prim_id;
};

VbId vb_id_unpack(uint2 packed) {
    VbId id;
    id.instance_id = packed.x;
    id.raw_prim_id = packed.y;
    return id;
}

uint2 vb_id_pack(uint instance_id, uint raw_prim_id) {
    return uint2(instance_id, raw_prim_id);
}
