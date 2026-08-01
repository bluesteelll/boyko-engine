// VG rung R2c0: the per-BATCH draw-record cull compute pass (`vb_batch_cull.comp.hlsl`).
//
// One invocation per `DrawBatch`. Each thread:
//   (a) READS its batch's descriptor (`VbBatchDesc[i]` — the world AABB + the instance count);
//   (b) DECIDES visibility (rung R2c0: unconditionally VISIBLE — see "INERT BY CONSTRUCTION");
//   (c) WRITES `instanceCount` into word 1 of that batch's `VkDrawIndexedIndirectCommand`
//       record, which `vkCmdDrawIndexedIndirect` fetches this same frame;
//   (d) atomic-appends the surviving batch index into the flat `VbCullVisible` list via one
//       `InterlockedAdd` on `VbCullCount[0]` — the SAME lock-free bump + clamp-and-drop shape
//       `cluster_cull.hlsl`'s own `LightIndexAlloc` claim uses.
//
// # INERT BY CONSTRUCTION (rung R2c0)
//
// `visible` is the literal `true` on this compile. That is not a placeholder that "happens to
// be true today" — it is the rung's DELIVERABLE. `docs/VG-DECIDABILITY-FLOOR.md` measured this
// engine's GPU-timing floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of ONE protocol, so
// no later cull delta is defensible without a NULL CONTROL taken in the same sitting: the cull
// machinery present, dispatched, and provably changing nothing. This module is that control.
// `instanceCount` is therefore re-written with EXACTLY the value the host's `vkCmdUpdateBuffer`
// already placed there, and the frame stays byte-identical — which is the gate this rung is
// graded on (`scripts/golden.ps1`), not a timing delta.
//
// Because `visible` is a compile-time constant, DXC constant-folds the branch and DEAD-CODES
// `aabb_min`/`aabb_max` out of the module entirely. That is deliberate and it is CHECKED:
// `tests/vb_batch_cull_spv_sync.rs` pins `OpFOrdLessThan == 0` and `OpDot == 0`, so a compile
// that quietly acquired a comparison would be RED at the artifact.
//
// # Why the AABB fields exist NOW, unread
//
// The descriptor's LAYOUT is what rung R2c (the real camera cull) needs; only the DECISION is
// missing. Fixing the 32-byte record here means R2c edits the shader body and the host's AABB
// fill, and touches neither the descriptor-set layout, the buffer sizes, nor the graph — the
// churn that a "just the count for now" struct would have forced on the very next rung.
//
// Rung R2c0 fills both corners with +/- `VbBatchDesc::UNBOUNDED` (a large FINITE magnitude,
// never an infinity: a frustum plane's `dot(n, p) + d` against an infinite corner can produce a
// NaN, and a NaN compare picks the OTHER operand under `NMin`/`NMax` rather than propagating).
// The sentinel therefore reads as "unbounded box", the CONSERVATIVE value: a batch whose AABB
// was never filled survives every plane test. R2c's own error direction is the same one — a
// wasted draw, never a false cull.
//
// # first_instance / index_count are NOT touched here
//
// This pass writes word 1 of the record and nothing else. `index_count`, `first_index`,
// `vertex_offset` and `first_instance` stay exactly as the host's transfer fill left them —
// notably `first_instance == 0`, which MUST hold because `drawIndirectFirstInstance` is
// `VK_FALSE` on this device and the validation layers cannot read buffer CONTENTS. Writing it
// from a shader would put that invariant past the reach of the host-side assert that guards it.
//
// Compiled offline (hermetic build) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 vb_batch_cull.comp.hlsl \
//       -Fo vb_batch_cull.comp.spv

// The workgroup width. Mirrors the host's `VB_BATCH_CULL_LOCAL_SIZE_X` — the dispatch is
// `ceil(batch_count / 64)` groups, so the tail group runs partly out of range and is trimmed by
// the `i >= pc.batch_count` guard below.
static const uint LOCAL_SIZE_X = 64u;

// The `VkDrawIndexedIndirectCommand` byte stride. Mirrors the host's
// `boyko_rhi_vulkan::ffi::DRAW_INDEXED_INDIRECT_STRIDE`; Vulkan fixes this layout, so both
// spellings are pinned to the same specification rather than to each other.
static const uint DRAW_INDEXED_INDIRECT_STRIDE = 20u;

// Byte offset of `instanceCount` inside that record: it is the SECOND `uint` member
// (`indexCount`, `instanceCount`, `firstIndex`, `vertexOffset`, `firstInstance`).
static const uint INSTANCE_COUNT_OFFSET = 4u;

// binding 0: this frame's indirect draw records (RW). Written at word 1 of each 20-byte record
// and read back the same frame by `vkCmdDrawIndexedIndirect` — the COMPUTE -> DRAW_INDIRECT
// dependency the framegraph derives.
RWByteAddressBuffer VbIndirect : register(u0);

// A batch's cull inputs. 32 bytes, `float3`-then-`uint` twice so the two 16-byte halves need no
// explicit padding member. Mirrors the host `VbBatchDesc`.
struct VbBatchDescGpu {
    float3 aabb_min;       // world-space AABB min corner (rung R2c0: -`VbBatchDesc::UNBOUNDED`)
    uint   instance_count; // the `instanceCount` a VISIBLE batch draws
    float3 aabb_max;       // world-space AABB max corner (rung R2c0: +`VbBatchDesc::UNBOUNDED`)
    uint   pad;            // reserved (rung R2c: the plane-set / batch-flags word)
};

// binding 1: the per-batch descriptors, transfer-filled by the host each frame (read-only).
StructuredBuffer<VbBatchDescGpu> VbBatchDesc : register(t1);

// binding 2: the compacted visible-batch list (RW). WRITTEN AND UNREAD at rung R2c0 — it is the
// compaction half of what R2 exists to de-risk, and nothing consumes it until a rung that can
// issue a merged multi-draw (which needs `multiDrawIndirect` + a merged vertex/index arena,
// neither of which exists on this device today).
RWStructuredBuffer<uint> VbCullVisible : register(u2);

// binding 3: the visible-batch counter (RW, one u32 at element 0). Transfer-zeroed by the host
// each frame ahead of the TRANSFER -> COMPUTE barrier the graph derives.
RWStructuredBuffer<uint> VbCullCount : register(u3);

// The batch-cull push constants. Mirrors the host `VbBatchCullPush` (8 bytes).
struct VbBatchCullPush {
    uint batch_count;  // number of live `DrawBatch` records this frame (the range guard)
    uint visible_cap;  // element capacity of `VbCullVisible` (the clamp-and-drop bound)
};
[[vk::push_constant]] VbBatchCullPush pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    const uint i = tid.x;
    // The tail group's out-of-range lanes. Every buffer access below is behind this guard, so no
    // lane can touch a record past the live batch count — `robustBufferAccess` is OFF on this
    // device, so an unguarded lane would be a real out-of-bounds device write.
    if (i >= pc.batch_count) {
        return;
    }

    const VbBatchDescGpu d = VbBatchDesc[i];

    // Rung R2c0: THE decision, and it is deliberately constant. Rung R2c replaces this line with
    // the conservative AABB-vs-frustum test over `d.aabb_min`/`d.aabb_max` — reject only when the
    // whole box sits in one plane's negative half-space, so a surviving batch can never be a
    // false cull. Nothing else in this file changes.
    const bool visible = true;

    // Word 1 of the record. A culled batch writes 0, which draws nothing while leaving the rest
    // of the record — and therefore the host's `first_instance == 0` invariant — untouched.
    VbIndirect.Store(
        i * DRAW_INDEXED_INDIRECT_STRIDE + INSTANCE_COUNT_OFFSET,
        visible ? d.instance_count : 0u);

    // Claim a slot in the compacted list (lock-free global bump). CLAMP-AND-DROP, the same
    // overflow discipline `cluster_cull.hlsl`'s global claim carries: a slot past the capacity
    // drops the entry rather than writing out of bounds. The counter still counts it, so a
    // future reader comparing `count` against `visible_cap` can SEE that the list was trimmed
    // instead of silently reading a short list as complete.
    if (visible) {
        uint slot = 0u;
        InterlockedAdd(VbCullCount[0], 1u, slot);
        if (slot < pc.visible_cap) {
            VbCullVisible[slot] = i;
        }
    }
}
