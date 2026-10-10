// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
// unwired). The `scan` compute pass ("The pipeline" step 3): a SINGLE workgroup performing two
// chained exclusive prefix sums over the LIVE `[0, material_count)` prefix of the M-arrays
// (`present_material_count`, plan D2 -- the frame's distinct material ids, NOT
// `MaterialTable::capacity_rows()`), in ONE dispatch (`vkCmdDispatch(1, 1, 1)`):
//
//   Phase 1: counts[mat] -> offsets[mat] (exclusive prefix sum); cursors[mat] = offsets[mat]
//            (the `scatter` pass's per-material write cursor, seeded to its region's start).
//   Phase 2: gc[mat] = ceil(counts[mat] / 64) -> gbase[mat] (exclusive prefix sum over `gc`);
//            group_to_mat[gbase[mat] .. gbase[mat]+gc[mat}) = mat (grid-stride fill, ONE
//            thread per LIVE material writes its own contiguous group range).
//
// `total_groups` (the running Phase-2 carry after the loop, `gs_carry`) is NOT written back
// anywhere this rung -- `vb_shade`'s over-dispatch (`G + present_material_count`, plan D2) is
// provably >= `total_groups` by construction (P1-1/D2), so the host never needs to read it
// back this rung.
//
// P1-1 (baked in, NOT this file's job): the SENTINEL tail past `total_groups` in
// `group_to_mat` is the `fill` pass's job (`vkCmdFillBuffer`, `0xFFFFFFFF`) -- this pass only
// OVERWRITES `[0, total_groups)`, never touches the tail.
//
// # Single-workgroup multi-block scan
//
// `SCAN_BLOCK` (256) threads process the (<= `VB_MAX_MATERIAL_ROWS`) live material rows in
// 256-wide blocks; each block's own Hillis-Steele INCLUSIVE scan (in-place, register-staged so
// no thread reads a value a sibling in the SAME step already advanced -- the two-barrier
// sandwich each step below) is chained to the next block via the `gs_carry` groupshared
// scalar (a classic single-pass multi-block scan). HLSL has no function pointers, so the two
// phases are two near-identical unrolled loops rather than one parameterized helper (the SAME
// "duplicated, not shared" idiom `forward_opaque.fs.hlsl`'s own doc documents for its pure
// helpers).
//
// # Bindings (Set 0 = `vb_layout0` only -- 1-set pipeline, `create_compute_pipeline`)
//
//   u7 : RWByteAddressBuffer gClassify   (via `vb_classify_common.hlsli`)
//
// No Camera/instance/image binding -- this pass never touches `w`/`h`/pixels (only the
// `w`/`h`-INDEPENDENT M-array + `group_to_mat` regions).
//
// Compiled offline: see `vb_classify_count.comp.hlsl`'s header for the dxc/spirv-val
// invocation (identical flags, this file's own name substituted).

#include "vb_classify_common.hlsli"

static const uint SCAN_BLOCK = 256u;

[[vk::push_constant]] struct PushConstants {
    // `present_material_count` (plan D2) -- the frame's LIVE distinct material-id count. A
    // LOOP BOUND only: never participates in any `gClassify` OFFSET computation (see
    // `vb_classify_common.hlsli`'s sync-pin doc -- offsets are fixed, `VB_MAX_MATERIAL_ROWS`-
    // sized, so `vb_shade` needs no matching push-constant field to agree on them).
    uint material_count;
} pc;

groupshared uint gs_scan[SCAN_BLOCK];
groupshared uint gs_carry;

[numthreads(SCAN_BLOCK, 1, 1)]
void main(uint3 tid : SV_GroupThreadID) {
    uint local = tid.x;

    // --- Phase 1: counts -> offsets (exclusive prefix sum); cursors seeded = offsets. ---
    if (local == 0u) {
        gs_carry = 0u;
    }
    GroupMemoryBarrierWithGroupSync();

    for (uint block_start = 0u; block_start < pc.material_count; block_start += SCAN_BLOCK) {
        uint mat = block_start + local;
        uint v = (mat < pc.material_count) ? cls_count(mat) : 0u;
        gs_scan[local] = v;
        GroupMemoryBarrierWithGroupSync();

        // In-place Hillis-Steele INCLUSIVE scan: read own + neighbor into registers before any
        // thread in this step overwrites `gs_scan`, so no thread ever reads a value a sibling
        // in the SAME step already advanced (the two-barrier sandwich makes the
        // read-all-then-write-all ordering explicit).
        for (uint offset = 1u; offset < SCAN_BLOCK; offset <<= 1u) {
            uint own = gs_scan[local];
            uint add = (local >= offset) ? gs_scan[local - offset] : 0u;
            GroupMemoryBarrierWithGroupSync();
            if (local >= offset) {
                gs_scan[local] = own + add;
            }
            GroupMemoryBarrierWithGroupSync();
        }

        uint inclusive = gs_scan[local];
        uint exclusive_in_block = inclusive - v;
        if (mat < pc.material_count) {
            uint global_exclusive = gs_carry + exclusive_in_block;
            cls_offset_store(mat, global_exclusive);
            cls_cursor_store(mat, global_exclusive);
        }
        GroupMemoryBarrierWithGroupSync();
        if (local == SCAN_BLOCK - 1u) {
            gs_carry += inclusive;
        }
        GroupMemoryBarrierWithGroupSync();
    }

    // --- Phase 2: gc = ceil(counts/64) -> gbase (exclusive prefix sum); fill group_to_mat. ---
    if (local == 0u) {
        gs_carry = 0u;
    }
    GroupMemoryBarrierWithGroupSync();

    for (uint block_start2 = 0u; block_start2 < pc.material_count; block_start2 += SCAN_BLOCK) {
        uint mat = block_start2 + local;
        uint gc = (mat < pc.material_count) ? ((cls_count(mat) + 63u) / 64u) : 0u;
        gs_scan[local] = gc;
        GroupMemoryBarrierWithGroupSync();

        for (uint offset = 1u; offset < SCAN_BLOCK; offset <<= 1u) {
            uint own = gs_scan[local];
            uint add = (local >= offset) ? gs_scan[local - offset] : 0u;
            GroupMemoryBarrierWithGroupSync();
            if (local >= offset) {
                gs_scan[local] = own + add;
            }
            GroupMemoryBarrierWithGroupSync();
        }

        uint inclusive2 = gs_scan[local];
        uint exclusive_in_block2 = inclusive2 - gc;
        if (mat < pc.material_count) {
            uint gbase = gs_carry + exclusive_in_block2;
            cls_gbase_store(mat, gbase);
            for (uint k = 0u; k < gc; ++k) {
                cls_g2m_store(gbase + k, mat);
            }
        }
        GroupMemoryBarrierWithGroupSync();
        if (local == SCAN_BLOCK - 1u) {
            gs_carry += inclusive2;
        }
        GroupMemoryBarrierWithGroupSync();
    }
}
