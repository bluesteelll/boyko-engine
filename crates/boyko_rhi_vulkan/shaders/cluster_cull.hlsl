// Lighting L1: the clustered froxel light-cull compute pass (`cluster_cull.hlsl`).
//
// One invocation per froxel (`CLUSTER_COUNT` total, the default 16×9×24 = 3456). Each
// froxel thread:
//   (a) BUILDs its WORLD-space AABB by unprojecting the froxel's screen-tile corners at the
//       exp-Z slice's near/far view-z (the SHARED ray-gen `generate_ray` + the view-z→t map,
//       so the AABB encloses exactly the world points the resolve reconstructs as `P = ro +
//       rd * t` for the pixels in that froxel);
//   (b) CULLs each POINT/SPOT light's bounding sphere (center = world pos, r = range) vs the
//       AABB via `sqDistPointAABB <= r²`; DIRECTIONAL + SKY are GLOBAL (the resolve always
//       loops the no-`P` front block — they are NOT culled here);
//   (c) atomic-appends surviving light indices into the flat `LightIndexList` (one
//       `InterlockedAdd` claims a disjoint slice base — lock-free, no data race) and writes
//       this froxel's `{offset, count}` ClusterCell.
//
// Overflow (O2 — clamp-and-drop): a per-froxel count reaching `max_lights_per_cluster`, or a
// global index-list claim past `index_list_cap`, DROPS the extra light (the bump is clamped,
// never overflowing the slice). No UB; the cap is a documented limit.
//
// Cluster grid + index-list layout is the ONE source of truth in `light_table.hlsli`
// (`cluster_linear_index`, `cluster_z_slice`); the resolve reads it back with the SAME
// linearization. The cull reads the camera UBO + the light table but NOT `gViewT` (it is
// geometric — the per-pixel surface `t` is the resolve's concern).
//
// Compiled offline (hermetic build) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 cluster_cull.hlsl \
//       -Fo cluster_cull.comp.spv

// binding 0: the camera/extent UNIFORM block (byte-identical to the resolve/marcher Camera).
cbuffer Camera : register(b0) {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;   // xyz = forward basis (NORMALIZED, O1), w = tan(fovY/2)
    float4 cam_right;     // xyz = right basis, w = aspect (W/H)
    float4 cam_up;
};

// binding 1: the L0/L1 light table (word-indexed `[LightHeaderGpu || GpuLight[]]`). Read for
// the header counts + each point/spot light's world sphere.
StructuredBuffer<uint> LightBuf : register(t1);

// binding 2: the per-froxel ClusterCell grid (RW): {offset, count} into LightIndexList.
RWStructuredBuffer<uint2> ClusterGrid : register(u2);

// binding 3: the flat light-index list (RW): per-froxel index slices concatenated. The
// cull atomic-claims disjoint slices via the global counter (element 0 of LightIndexAlloc).
RWStructuredBuffer<uint> LightIndexList : register(u3);

// binding 4: the global slice-allocation counter (RW, one u32). `InterlockedAdd` on
// element 0 hands out disjoint LightIndexList ranges (lock-free).
RWStructuredBuffer<uint> LightIndexAlloc : register(u4);

// The cull push constants: the exp-Z near/far + the caps (the dims/scale come from the
// header, but the cull needs the raw near/far to build the slice view-z bounds, and the caps
// to clamp-and-drop). Mirrors the host `ClusterCullPush`.
struct ClusterCullPush {
    float z_near;                 // exp-Z near plane (slice 0 view-z)
    float z_far;                  // exp-Z far plane (slice dim_z view-z)
    uint  max_lights_per_cluster; // per-froxel cap (O2 clamp-and-drop)
    uint  index_list_cap;         // flat-list cap (O2 global clamp-and-drop)
};
[[vk::push_constant]] ClusterCullPush pc;

#include "ray_gen.hlsli"
#include "light_table.hlsli"

static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The exp-Z view-z at slice boundary `k` (k in [0, dim_z]): near * (far/near)^(k/dim_z).
// The MATCHING distribution the resolve's `cluster_z_slice` inverts.
float slice_view_z(uint k, uint dim_z) {
    return pc.z_near * pow(pc.z_far / pc.z_near, (float)k / (float)dim_z);
}

// Converts a view-space depth `view_z` to the world-ray parameter `t` for the ray (ro, rd),
// the inverse of the resolve's `view_z = dot(rd, cam_forward.xyz) * t` (PERSP) / `view_z = t`
// (ORTHO). O1: cam_forward.xyz is contractually NORMALIZED, so the PERSP divisor is the
// cosine between the pixel ray and the camera axis.
float view_z_to_t(float view_z, float3 rd) {
    if (camera_mode == RAYGEN_CAM_PERSPECTIVE) {
        float cos_axis = dot(rd, cam_forward.xyz);
        return view_z / max(cos_axis, 1e-4);
    }
    return view_z; // ORTHO: view-z is the ray parameter directly (rd = (0,0,-1))
}

// Expands `aabb_min`/`aabb_max` to include the world point `ro + rd * t`.
void expand_aabb(inout float3 aabb_min, inout float3 aabb_max, float3 ro, float3 rd, float t) {
    float3 p = ro + rd * t;
    aabb_min = min(aabb_min, p);
    aabb_max = max(aabb_max, p);
}

// Squared distance from a point to an AABB (0 inside). The canonical clustered-cull test:
// a sphere (center, r) intersects the AABB iff this <= r².
float sq_dist_point_aabb(float3 c, float3 aabb_min, float3 aabb_max) {
    float3 d = max(max(aabb_min - c, c - aabb_max), 0.0.xxx);
    return dot(d, d);
}

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    ClusterParams cp = load_cluster_params(LightBuf);
    uint cluster_count = cp.dim_x * cp.dim_y * cp.dim_z;
    uint fi = tid.x;
    if (fi >= cluster_count) {
        return;
    }

    // Delinearize the flat froxel index back to (x, y, z) — the inverse of
    // `cluster_linear_index` ((y*dim_x + x)*dim_z + z, Z innermost).
    uint z = fi % cp.dim_z;
    uint xy = fi / cp.dim_z;
    uint x = xy % cp.dim_x;
    uint y = xy / cp.dim_x;

    uint w = img_w();
    uint h = img_h();

    // Build the froxel's WORLD-space AABB: unproject the 4 screen-tile corners at the slice's
    // near + far view-z (8 world points). The tile (x,y) covers pixels
    // [x*w/dim_x .. (x+1)*w/dim_x) — sample the inclusive corner pixels so the AABB strictly
    // encloses every pixel ray in the froxel (a conservative over-estimate is sound for a
    // light cull: it can only KEEP a light, never falsely drop one).
    uint px0 = (x * w) / cp.dim_x;
    uint py0 = (y * h) / cp.dim_y;
    uint px1 = ((x + 1u) * w) / cp.dim_x;
    uint py1 = ((y + 1u) * h) / cp.dim_y;
    if (px1 > 0u) { px1 -= 1u; }
    if (py1 > 0u) { py1 -= 1u; }
    px1 = max(px1, px0);
    py1 = max(py1, py0);

    float vz_near = slice_view_z(z, cp.dim_z);
    float vz_far = slice_view_z(z + 1u, cp.dim_z);

    float3 aabb_min = (1.0e30).xxx;
    float3 aabb_max = (-1.0e30).xxx;
    uint2 corners[4] = { uint2(px0, py0), uint2(px1, py0), uint2(px0, py1), uint2(px1, py1) };
    [unroll]
    for (uint ci = 0u; ci < 4u; ++ci) {
        float3 ro, rd;
        generate_ray(corners[ci].x, corners[ci].y, w, h, camera_mode,
                     cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
        expand_aabb(aabb_min, aabb_max, ro, rd, view_z_to_t(vz_near, rd));
        expand_aabb(aabb_min, aabb_max, ro, rd, view_z_to_t(vz_far, rd));
    }

    // Cull the POINT/SPOT block [l0a_count .. light_count) against this froxel's AABB. The
    // surviving indices are buffered locally, then a SINGLE InterlockedAdd claims a disjoint
    // slice base (lock-free) and the disjoint writes scatter the indices into LightIndexList.
    LightHeader hd = load_light_header(LightBuf);
    uint local[256]; // MAX_LIGHTS_PER_CLUSTER worst case (the per-froxel cap)
    uint nlocal = 0u;
    for (uint i = hd.l0a_count; i < hd.light_count; ++i) {
        LightElem L = load_light(LightBuf, i);
        uint k = light_kind(L); // mask off the bit-16 shadow flag + bits-17..21 atlas slot (light_table.hlsli)
        if (k != LIGHT_KIND_POINT && k != LIGHT_KIND_SPOT) {
            continue;
        }
        float r = L.range;
        if (sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r * r) {
            // O2 clamp-and-drop: stop appending at the per-froxel cap (no overflow of `local`).
            if (nlocal < pc.max_lights_per_cluster && nlocal < 256u) {
                local[nlocal] = i;
                nlocal += 1u;
            }
        }
    }

    // Claim a disjoint slice of the flat list (lock-free global bump). O2 global clamp: if the
    // claim runs past `index_list_cap` the overflow tail is dropped (count is trimmed to the
    // fitting prefix), never writing out of bounds.
    uint offset = 0u;
    uint write_count = 0u;
    if (nlocal > 0u) {
        InterlockedAdd(LightIndexAlloc[0], nlocal, offset);
        if (offset >= pc.index_list_cap) {
            write_count = 0u;            // the whole slice fell past the cap — drop all
        } else {
            write_count = min(nlocal, pc.index_list_cap - offset);
            for (uint k = 0u; k < write_count; ++k) {
                LightIndexList[offset + k] = local[k];
            }
        }
    }

    ClusterGrid[fi] = uint2(offset, write_count);
}
