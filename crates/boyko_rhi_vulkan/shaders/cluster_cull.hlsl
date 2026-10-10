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
// -D HIER=1 (VB-P1e rung H2, "dark infra"): compiles in a SECOND arm of `main` that replaces
// the flat one-thread-per-froxel scan with a two-level, gather-side hierarchical cull — a
// 256-lane workgroup first reduces its own froxels' AABBs into a group box (in groupshared
// memory), tests the light table against THAT once, records survivors as a groupshared
// bitmask, then re-runs today's exact per-froxel test over only the mask's set bits. Nothing
// in this engine selects the HIER pipeline by default (H3/H4 arm it behind an env knob). See
// docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md section 4 for the full derivation and D1-D11 for the
// design decisions the shape below encodes.
//
// VB-P1j (this rung): the BASE arm's `ClusterGrid[fi]` WRITE is now bounded by the buffer's own
// element count, not only by the live header's dims — see the base arm's own prologue below for
// the derivation. Both arms are therefore hard-bounded by the allocation; they get there by
// DIFFERENT routes (HIER: D11's pushed boot capacity; base: `GetDimensions`), which is stated
// where each one lives.
//
// Compiled offline (hermetic build) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 cluster_cull.hlsl \
//       -Fo cluster_cull.comp.spv
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 -D HIER=1 cluster_cull.hlsl \
//       -Fo cluster_cull_hier.comp.spv

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
#ifdef HIER
    // D11: two BOOT-snapshot words, minted once in `build_froxel_light_cull` from the SAME
    // `cluster_config.cluster_count()` binding that sizes `ClusterGrid` — never re-derived
    // from the live header, which `sync_cluster_light_gate` may move behind this dispatch.
    uint cluster_dims_packed;     // dim_x | dim_y<<8 | dim_z<<16   (the MAPPING)
    uint cluster_capacity;        // cluster_count() in FULL precision (the WRITE BOUND)
#endif
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
// a sphere (center, r) intersects the AABB iff this <= r^2.
//
// The sum is WRITTEN OUT and `precise`, not `dot()`, on purpose. Vulkan specifies OpFAdd /
// OpFSub / OpFMul as "Correctly rounded" (one legal fp32 result), but specifies OpDot only as
// "inherited from a formula", and the same appendix permits that formula to "be transformed
// using the mathematical associativity, commutativity, and distributivity of the operators
// involved". Two OpDot instructions in one module may therefore be lowered to different
// summation orders (or to different FMA-contracted forms) by the driver -- and DXC emits no
// Fma at all, so contraction is decided BELOW the .spv, where no byte- or disassembly-gate can
// see it. VB-P1e's coarse->fine enclosure proof needs the two call sites to evaluate the SAME
// function of their operands; correctly-rounded ops plus NoContraction (what `precise` emits)
// deliver exactly that, unconditionally. `precise` is on BOTH `d` and `sd` so that every node
// the monotonicity chain of the proof traverses -- the two OpFSub included -- is decorated;
// see docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md section 5 step 2. It also makes the GPU match the
// host oracle `golden_sq_dist_point_aabb` (goldens.rs:3491), which accumulates `s += d*d` in
// the identical ((dx^2+dy^2)+dz^2) order and never fuses.
float sq_dist_point_aabb(float3 c, float3 aabb_min, float3 aabb_max) {
    precise float3 d  = max(max(aabb_min - c, c - aabb_max), 0.0.xxx);
    precise float  sd = d.x * d.x + d.y * d.y + d.z * d.z;
    return sd;
}

// === HIER (VB-P1e H2) — the group-uniform hierarchical arm's private declarations ==========
// #ifdef-only: none of this exists in the base (no `-D`) compile, so the base module is
// physically unperturbed by the seam (D5). `HIER_TPG`/`HIER_MASK_WORDS` are `#define`, not
// `static const uint`, because the `#if`/`#error` guards below need preprocessor constants.
#ifdef HIER
#define HIER_TPG        256u        // host mirror: ClusterConfig::hier_group_threads()
#define HIER_MASK_WORDS 32u         // MAX_LIGHTS / 32  (D6, pinned EQUAL)
#define HIER_MASK_BITS  (HIER_MASK_WORDS * 32u)
#define HIER_FMAX       asfloat(0x7F7FFFFFu)   // +FLT_MAX exactly, by bit pattern (D8, section 5 B)
#if (HIER_MASK_WORDS) > 32
#error "HIER_MASK_WORDS > 32: gs_summary is a SINGLE uint, one bit per mask word"
#endif
#if (HIER_MASK_WORDS) > (HIER_TPG)
#error "HIER_MASK_WORDS > HIER_TPG: phase 1 inits exactly one mask word per lane"
#endif
#if (HIER_TPG) != 256u
#error "HIER_TPG != 256: D9's radix-16 fold hardcodes 16 folding lanes x 16 entries == 256"
#endif
// D9: six SCALAR arrays, not two float3 arrays — workgroup storage carries no explicit
// layout under this recipe (no VK_KHR_workgroup_memory_explicit_layout), so a float3's
// stride is driver-chosen and undeterminable from the .spv; a float has none. Also makes
// lane-indexed access 4-byte-strided (bank-conflict-free). Footprint: 6*256*4 + 32*4 + 4 =
// 6 276 B, exact by construction.
groupshared float gs_min_x[HIER_TPG], gs_min_y[HIER_TPG], gs_min_z[HIER_TPG];
groupshared float gs_max_x[HIER_TPG], gs_max_y[HIER_TPG], gs_max_z[HIER_TPG];
groupshared uint  gs_mask[HIER_MASK_WORDS];
groupshared uint  gs_summary;       // bit j <=> gs_mask[j] != 0
// [numthreads] is driven by the SAME `HIER_TPG` macro the #error guard above pins to 256,
// not a second, independently-typed literal — a `-D`-only edit that widens/narrows the
// group can no longer leave the dispatch's actual LocalSize silently un-updated (P1-1,
// VB-P1e H2 adversarial review): the two were previously two free-standing literals with no
// relationship the compiler enforced. DXC macro-substitutes attribute arguments before
// parsing them, so `HIER_TPG` (`256u`) is legal here; re-verified byte-identical against the
// committed `cluster_cull_hier.comp.spv` before this landed.
[numthreads(HIER_TPG, 1, 1)]
#else
[numthreads(64, 1, 1)]
#endif
// The 3-parameter signature is SHARED (D5): DXC dead-strips the unused `gid`/`lane` SV
// parameters from the base (no `-D`) compile, so the base module is byte-identical whether
// or not they are declared here.
void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex) {
#ifdef HIER
    // ==== HIER arm (VB-P1e H2) =============================================================
    // D8's mandatory review checkpoint: every lane, valid or not, reaches every
    // GroupMemoryBarrierWithGroupSync() below. The out-of-range condition is the `valid` BOOL
    // computed here, never control flow across a barrier — an early `return` is UB and
    // typically a device hang. `valid` is the ONLY predicate gating phases 0/1/5/6 (Rev 4
    // deleted a second `contrib` predicate that let a lane skip the fold yet still run its
    // own fine test — see docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md section 4/5).

    // Phase -1 — the group-uniform prologue (before phase 0, before any barrier). D7's
    // mask-capacity clamp: the coarse groupshared WRITE and BOTH device reads share the ONE
    // bound `ps_n` (`j < ps_n <= ps_room <= HIER_MASK_BITS == MAX_LIGHTS`, D6's equality pin).
    // Sourced from header words 0/2 (l0a_count/light_count), NOT word 3 (point_spot_count) —
    // the base arm's range is [l0a_count, light_count), and the span must come from the SAME
    // two words or D4's byte-identity breaks.
    LightHeader hd = load_light_header(LightBuf);
    uint ps_begin = hd.l0a_count;
    uint ps_room  = (ps_begin < HIER_MASK_BITS) ? (HIER_MASK_BITS - ps_begin) : 0u;
    uint ps_total = (hd.light_count > ps_begin) ? (hd.light_count - ps_begin) : 0u;
    uint ps_n     = min(ps_total, ps_room);

    // D3/D11's thread-to-froxel map: every dim comes from the BOOT push, never the live
    // header. `capacity` is D11's pushed full-precision `cluster_count()` — the hard
    // device-write bound, and the only term naming ClusterGrid's real size.
    uint bdx = pc.cluster_dims_packed & 0xFFu;
    uint bdy = (pc.cluster_dims_packed >>  8) & 0xFFu;
    uint bdz = (pc.cluster_dims_packed >> 16) & 0xFFu;
    uint capacity = pc.cluster_capacity;
    uint gps   = max(1u, (bdx * bdy + 255u) / 256u);   // max(1) => OpUDiv can never divide by 0
    uint slice = gid.x / gps;
    uint s     = (gid.x % gps) * 256u + lane;
    uint x = (bdx != 0u) ? (s % bdx) : 0u;             // % 0 is UB on a degenerate header
    uint y = (bdx != 0u) ? (s / bdx) : 0u;
    uint z = slice;
    uint fi = cluster_linear_index(x, y, z, bdx, bdz); // pure uint arithmetic, no memory touched
    bool valid = (s < bdx * bdy) && (slice < bdz) && (fi < capacity);

    // Phase 0 — unchanged froxel AABB build (only when `valid`): the same 4 corners x
    // {near, far} unprojection as the base arm, over (x,y,z)/bdx/bdy/bdz instead of
    // (x,y,z)/cp.dim_x/cp.dim_y/cp.dim_z (D4 scope clause (b): value-identical, both are the
    // same 8-bit fields of the same encoding). An invalid lane's box stays the (+1e30,-1e30)
    // "nothing yet" initializer, which is also D8's `!valid` identity element.
    float3 aabb_min = (1.0e30).xxx;
    float3 aabb_max = (-1.0e30).xxx;
    if (valid) {
        uint w = img_w();
        uint h = img_h();
        uint px0 = (x * w) / bdx;
        uint py0 = (y * h) / bdy;
        uint px1 = ((x + 1u) * w) / bdx;
        uint py1 = ((y + 1u) * h) / bdy;
        if (px1 > 0u) { px1 -= 1u; }
        if (py1 > 0u) { py1 -= 1u; }
        px1 = max(px1, px0);
        py1 = max(py1, py0);

        float vz_near = slice_view_z(z, bdz);
        float vz_far  = slice_view_z(z + 1u, bdz);

        uint2 corners[4] = { uint2(px0, py0), uint2(px1, py0), uint2(px0, py1), uint2(px1, py1) };
        [unroll]
        for (uint ci = 0u; ci < 4u; ++ci) {
            float3 ro, rd;
            generate_ray(corners[ci].x, corners[ci].y, w, h, camera_mode,
                         cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro, rd);
            expand_aabb(aabb_min, aabb_max, ro, rd, view_z_to_t(vz_near, rd));
            expand_aabb(aabb_min, aabb_max, ro, rd, view_z_to_t(vz_far, rd));
        }
    }

    // Phase 1 — the substitution (D8's table, verbatim). The ABSORBING element is
    // +/-FLT_MAX (HIER_FMAX, exact by bit pattern -- no decimal literal has to be trusted to
    // round to it). The FINITENESS THRESHOLD below is 1e30 and is a DIFFERENT constant with a
    // different job: it classifies the lane, and it is what makes the !valid identity a true
    // identity. Do NOT unify the two -- section 5 Case B derives why +/-1e30 as the absorbing
    // element inverts enclosure for a finite light centre with |c| > 1e30.
    //
    // Branch order is LOAD-BEARING: `finite` reads aabb_min/aabb_max, which for an `!valid`
    // lane are still the (1e30,-1e30) initializers above (phase 0 never ran), and those
    // satisfy `abs(v) <= 1e30` — so `finite` is true for them too, and the `!valid` branch
    // must be tested FIRST or an invalid lane would take the absorbing (wrong) row instead of
    // the identity (correct) row.
    bool finite = all(abs(aabb_min) <= 1.0e30) && all(abs(aabb_max) <= 1.0e30);
    float3 store_min, store_max;
    if (!valid)          { store_min = ( 1.0e30).xxx;    store_max = (-1.0e30).xxx;    }  // identity
    else if (!finite)    { store_min = (-HIER_FMAX).xxx; store_max = ( HIER_FMAX).xxx; }  // ABSORBING
    else                 { store_min = aabb_min;         store_max = aabb_max;         }
    gs_min_x[lane] = store_min.x;
    gs_min_y[lane] = store_min.y;
    gs_min_z[lane] = store_min.z;
    gs_max_x[lane] = store_max.x;
    gs_max_y[lane] = store_max.y;
    gs_max_z[lane] = store_max.z;
    if (lane < HIER_MASK_WORDS) { gs_mask[lane] = 0u; }
    if (lane == 0u)             { gs_summary   = 0u; }
    GroupMemoryBarrierWithGroupSync();                              // B1

    // Phase 2 — D9's radix-16 in-place fold. Lanes [0,16) each serially fold the 16 entries
    // gs[l + 16k], k = 0..15, into gs[l]. Race-free in place: every active writer has l < 16,
    // every read address l + 16k for k >= 1 is >= 16, and k = 0 is the writer's own slot.
    if (lane < 16u) {
        float mnx = gs_min_x[lane], mny = gs_min_y[lane], mnz = gs_min_z[lane];
        float mxx = gs_max_x[lane], mxy = gs_max_y[lane], mxz = gs_max_z[lane];
        for (uint k = 1u; k < 16u; ++k) {
            uint idx = lane + 16u * k;
            mnx = min(mnx, gs_min_x[idx]);
            mny = min(mny, gs_min_y[idx]);
            mnz = min(mnz, gs_min_z[idx]);
            mxx = max(mxx, gs_max_x[idx]);
            mxy = max(mxy, gs_max_y[idx]);
            mxz = max(mxz, gs_max_z[idx]);
        }
        gs_min_x[lane] = mnx; gs_min_y[lane] = mny; gs_min_z[lane] = mnz;
        gs_max_x[lane] = mxx; gs_max_y[lane] = mxy; gs_max_z[lane] = mxz;
    }
    GroupMemoryBarrierWithGroupSync();                              // B2

    // Phase 3 — EVERY lane folds gs[0..16) into registers: 16 group-uniform broadcast reads,
    // no write, so no third barrier is needed (B2 already published, nobody writes
    // afterwards). This is what makes coarse_min/coarse_max group-uniform (D8 review item 6,
    // section 5 Setup premise) — the proof needs every lane testing against the SAME box.
    float3 coarse_min = float3(gs_min_x[0], gs_min_y[0], gs_min_z[0]);
    float3 coarse_max = float3(gs_max_x[0], gs_max_y[0], gs_max_z[0]);
    for (uint fw = 1u; fw < 16u; ++fw) {
        coarse_min = min(coarse_min, float3(gs_min_x[fw], gs_min_y[fw], gs_min_z[fw]));
        coarse_max = max(coarse_max, float3(gs_max_x[fw], gs_max_y[fw], gs_max_z[fw]));
    }

    // Phase 4 — coarse scan. ALL 256 lanes participate, `valid` or not: the coarse light scan
    // is striped across every lane regardless of validity, so idle lanes are productive here
    // (D3 trade-off 1). `sq_dist_point_aabb` is the SAME shared function phase 5/6 call — D10
    // — so both levels evaluate one function F, which is section 5's Step 0 premise.
    for (uint j = lane; j < ps_n; j += HIER_TPG) {
        LightElem CL = load_light(LightBuf, ps_begin + j);
        uint ck = light_kind(CL);
        if (ck != LIGHT_KIND_POINT && ck != LIGHT_KIND_SPOT) { continue; }
        float cr = CL.range;
        if (sq_dist_point_aabb(CL.pos, coarse_min, coarse_max) <= cr * cr) {
            // j < ps_n <= ps_room <= HIER_MASK_BITS == HIER_MASK_WORDS*32 => (j>>5) < HIER_MASK_WORDS
            // — the write bound is a syntactic consequence of the loop condition, no clamp
            // needed at the write site itself (D7's rejected-alternatives note).
            InterlockedOr(gs_mask[j >> 5], 1u << (j & 31u));
            InterlockedOr(gs_summary, 1u << (j >> 5));
        }
    }
    GroupMemoryBarrierWithGroupSync();                              // B3

    // Phase 5 (fine walk) + phase 6 (claim + scatter), `valid` only, no barrier between them.
    // Phase 5's inner test and phase 6's tail are TOKEN-IDENTICAL to the base arm's (D4), so
    // this is what makes D4's byte-identity a construction rather than a hope. `j >= ps_n` is
    // D7's SAME bound phase 4 wrote under, re-checked before the reconstruction `i = ps_begin
    // + j` — the one clamp both the groupshared write and both device reads share.
    if (valid) {
        uint local[256]; // MAX_LIGHTS_PER_CLUSTER worst case (the per-froxel cap)
        uint nlocal = 0u;
        uint summary = gs_summary;
        while (summary != 0u) {
            uint mw = firstbitlow(summary);
            summary &= ~(1u << mw);
            uint bits = gs_mask[mw];
            while (bits != 0u) {
                uint mb = firstbitlow(bits);
                bits &= ~(1u << mb);
                uint j = (mw << 5) | mb;
                if (j >= ps_n) { continue; }        // D7: the SAME bound phase 4 wrote under
                uint i = ps_begin + j;
                LightElem L = load_light(LightBuf, i);
                uint k = light_kind(L);
                if (k != LIGHT_KIND_POINT && k != LIGHT_KIND_SPOT) { continue; }
                float r = L.range;
                if (sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r * r) {
                    // O2 clamp-and-drop: stop appending at the per-froxel cap.
                    if (nlocal < pc.max_lights_per_cluster && nlocal < 256u) {
                        local[nlocal] = i;
                        nlocal += 1u;
                    }
                }
            }
        }

        // Claim a disjoint slice of the flat list (lock-free global bump). O2 global clamp:
        // if the claim runs past `index_list_cap` the overflow tail is dropped, never writing
        // out of bounds. `fi < pc.cluster_capacity` (folded into `valid`) is what makes this
        // ClusterGrid write in-bounds (D11) — the total bound this rung must not lose.
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
#else
    // ==== base arm =========================================================================
    // The froxel geometry, the light test and the claim/scatter tail below are token-for-token
    // what they have always been (D4's byte-identity claim is about phase 5's inner test and
    // phase 6's tail, both untouched). The ONE thing VB-P1j changes is the write bound in this
    // prologue.
    ClusterParams cp = load_cluster_params(LightBuf);

    // VB-P1j — the WRITE bound. `ClusterGrid` is SIZED once, at boot, from
    // `ClusterConfig::cluster_count()` (`gpu_scene/mod.rs`'s `build_froxel_light_cull`), and is
    // never re-allocated. `cp.dim_*` come from the LIVE light-table header, which
    // `sync_cluster_light_gate` (`light.rs`) republishes from the LIVE `ClusterConfig` Resource
    // every frame. A post-boot `ClusterConfig` edit therefore moves the live dims out from under
    // a buffer that keeps its boot size, and the host still dispatches the BOOT froxel count
    // rounded up to this arm's 64-wide group. Bounding only on `dim_x*dim_y*dim_z` let this arm
    // write `min(64*ceil(boot_cc/64), live_cc)` cells into a `boot_cc`-cell buffer — measured at
    // 16 cells / 128 bytes past the end for boot 16x9x23 vs live 16x9x24, silent because
    // `robustBufferAccess` is OFF and no GPU-assisted validation runs.
    //
    // `GetDimensions` reports the BOUND DESCRIPTOR's own element count (SPIR-V `OpArrayLength`
    // on the runtime array) — the allocation itself, not a host-side mirror of it — so this
    // bound cannot drift from the buffer even if a push word or a boot snapshot were wrong. That
    // is why this arm does NOT take the HIER arm's route (D11's pushed `cluster_capacity`); the
    // two are provably equal on every correct boot, and the asymmetry is deliberate: the pushed
    // word buys the HIER arm a group-uniform `capacity` its thread map already needed, while
    // this arm needs no new push word, no new pipeline-layout range and no host change at all.
    //
    // Cost: two instructions (`OpArrayLength` + `OpUMin`) once per thread, outside the light
    // loop. Output: IDENTICAL whenever boot dims == live dims (every shipping configuration),
    // because `grid_capacity == cluster_count` makes the `min` a no-op — that is what keeps
    // every existing golden byte-identical.
    uint grid_capacity, grid_stride;
    ClusterGrid.GetDimensions(grid_capacity, grid_stride);
    uint cluster_count = min(cp.dim_x * cp.dim_y * cp.dim_z, grid_capacity);
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
#endif
}
