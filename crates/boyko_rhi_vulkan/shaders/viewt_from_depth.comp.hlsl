// Multi-paradigm render-path plan, rung R3b (Deferred x Mesh -- the SDF leg fully off): the
// gViewT PRODUCER replacement for a mesh-only frame.
//
// The R3 audit (`sdf_gbuffer_composite.hlsl`, both terminal write sites) found the SDF marcher
// is the SOLE writer of the `gViewT` lane even for MESH-owned pixels: a mesh-covered,
// non-SDF-lit pixel gets `gViewT = t_mesh` (`= md * mesh_norm`, the mesh surface's own ray
// parameter) so the resolve's `P = ro + rd*view_t` reconstruction and SSAO's mesh/SDF
// `view_t` classification both see the REAL mesh surface. Under `Deferred x Mesh` the marcher
// is not dispatched at all (`GeometryLegs::Mesh` => no SDF leg), so nothing writes `gViewT` --
// this small full-screen pass is that replacement, reproducing the marcher's own conversion
// TOKEN-FOR-TOKEN (`sdf_gbuffer_composite.hlsl:1434-1440` + the terminal `gViewT` write at
// :1497), just without the SDF branch (mask is always 0 on this leg -- there is no SDF leg to
// win ownership):
//
//   float md = gDepth.Load(...).r;
//   gViewT[...] = (md < DEPTH_CLEAR) ? (md * mesh_norm) : VIEWT_BG;
//
// `mesh_norm` is NOT recomputed here from `camera_mode` (that would be a THIRD hand-written
// HLSL copy of the marcher's `mesh_norm` ternary, alongside `sdf_gbuffer_composite.hlsl` and
// `sdf_tile_cull.hlsl`) -- it arrives PRECOMPUTED host-side, via
// `boyko_render::gbuffer_depth::mesh_view_t_norm` (the single-sourced Rust mirror of that same
// ternary; see `boyko_rhi_vulkan::compute::ViewtFromDepthPush`), as a push-constant float. This
// shader therefore carries NO `camera_mode` branch at all.
//
// # Resources (dedicated 2-image bind-group; the smallest existing compute-pass precedent,
//   `ssao_atrous.comp.hlsl`'s 4-binding shared layout, minus the 2 bindings this pass does not
//   need -- no AO lanes, no camera UBO, since `mesh_norm` is a push constant not a UBO read)
//
//   binding 0 : Texture2D<float> (SAMPLED) -- the mesh depth (DEPTH-aspect view of the shared
//               D32_SFLOAT image, the SAME access the marcher declares at its own binding 1:
//               `.Load` / OpImageFetch, no sampler consumed).
//   binding 1 : RWTexture2D<float> (STORAGE, r32f) -- gViewT (WRITE; every dispatched pixel is
//               written exactly once -- this pass covers the WHOLE screen, unlike the marcher's
//               conditional early-return sites).
// A 12-byte `[[vk::push_constant]]` block (`ViewtFromDepthPush`): `img_w`/`img_h` (the bounds
// guard for the ceil(w/8)*ceil(h/8) dispatch grid) + `mesh_norm` (the host-precomputed
// normalizer above).
//
// Compiled offline (hermetic -- no SDK at `cargo build` time) with:
//   dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       viewt_from_depth.comp.hlsl -Fo viewt_from_depth.comp.spv

Texture2D<float> gDepth : register(t0);
[[vk::image_format("r32f")]] RWTexture2D<float> gViewT : register(u1);

struct ViewtFromDepthPush {
    uint  img_w;
    uint  img_h;
    float mesh_norm;
};
[[vk::push_constant]] ViewtFromDepthPush pc;

// The depth-clear sentinel (mirrors `sdf_gbuffer_composite.hlsl`'s `DEPTH_CLEAR` / the host
// `MESH_DEPTH_CLEAR`): a stored depth `>= DEPTH_CLEAR` means no mesh fragment rasterized here.
static const float DEPTH_CLEAR = 1.0;
// The mesh/SDF G-buffer background sentinel (mirrors the marcher's `gViewT` `1.0e30` sentinel /
// `ssao_atrous.comp.hlsl`'s own `VIEWT_BG` local).
static const float VIEWT_BG = 1.0e30;

// Full-screen, 8x8 tiles (a 2D dispatch -- the group grid is `ceil(img_w/8) x ceil(img_h/8)`, so
// an edge group can run threads past the real extent; the bounds guard below discards them).
[numthreads(8, 8, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    if (tid.x >= pc.img_w || tid.y >= pc.img_h) {
        return;
    }
    uint px = tid.x;
    uint py = tid.y;

    // The marcher's own mesh-depth decode (`sdf_gbuffer_composite.hlsl:1434-1440`), byte-for-byte:
    // a SAMPLED-IMAGE fetch, the SAME `< DEPTH_CLEAR` / `* mesh_norm` interpretation. `mesh_norm`
    // is the push-constant value already selected host-side by camera_mode -- no branch here.
    float md = gDepth.Load(int3((int)px, (int)py, 0)).r;
    bool has_mesh = (md < DEPTH_CLEAR);
    float t_mesh = has_mesh ? (md * pc.mesh_norm) : VIEWT_BG;

    gViewT[uint2(px, py)] = t_mesh;
}
