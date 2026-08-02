// VG rungs R2c0/R2c/R2d-3: the VisibilityBuffer cull compute pass (`vb_batch_cull.comp.hlsl`).
//
// One invocation per `DrawBatch`. There are TWO LEVELS, and they are different tests on different
// data:
//
//   LEVEL 1 (per BATCH, rung R2c) — `VbBatchDesc[i]`'s world AABB is the UNION over that batch's
//     instances. It is tested against the six pushed frustum planes; a batch wholly outside one
//     plane is certainly invisible in every instance, so the level-2 result for it is zero
//     survivors. This is the cheap early verdict, and it is the ONLY level that culls today.
//
//   LEVEL 2 (per INSTANCE, rung R2d) — the loop below walks this batch's `instance_count`
//     instances and compacts the survivors into `VbVisibleInstance`. Rung R2d-3 ships that loop
//     with its `keep` predicate HARDWIRED to `true`: the machinery — the region write, the
//     survivor count, the relocated counter bump — is present, dispatched, and provably changing
//     nothing OBSERVABLE IN THE IMAGE, because `k` then equals `d.instance_count`, the value the
//     host's transfer fill already wrote into the record and the value rung R2c already stores.
//     The pass's GPU TIME is a different matter and is NOT unchanged: the loop is real work —
//     `instance_count` dependent iterations, each a global store, run by ONE lane per batch. That
//     cost is deliberate. A null control that skipped the work would not be a control for the
//     armed version's cost, which is exactly the comparison the decidability floor demands. `keep`
//     is the ONE
//     expression the arming rung replaces (with the instance's own world box — its mesh's LOCAL
//     box from `gMeshBounds[mesh_id]` transformed by `gVbInstances[...]`'s affine — against the
//     same six planes). That is the R2c0-before-R2c discipline, repeated deliberately: the null
//     control `docs/VG-DECIDABILITY-FLOOR.md` demands has to be present in the MEASURED
//     configuration to be a control at all.
//
// Each thread therefore:
//   (a) READS its batch's descriptor (`VbBatchDesc[i]` — the union AABB, the instance count and,
//       since rung R2d-3, the `base_instance` that keys its region of the survivor list);
//   (b) DECIDES level-1 visibility against the six pushed frustum planes;
//   (c) WRITES its survivors into `VbVisibleInstance[base_instance ..]` (level 2);
//   (d) WRITES the survivor count `k` into word 1 of that batch's `VkDrawIndexedIndirectCommand`
//       record — gated by the level-1 verdict, so a level-1-culled batch still writes 0 exactly as
//       rung R2c does — which `vkCmdDrawIndexedIndirect` fetches this same frame;
//   (e) atomic-appends the surviving BATCH index into the flat `VbCullVisible` list via one
//       `InterlockedAdd` on `VbCullCount[0]` — the SAME lock-free bump + clamp-and-drop shape
//       `cluster_cull.hlsl`'s own `LightIndexAlloc` claim uses.
//
// # TWO visible lists now coexist, and they are NOT alternatives
//
//   * `VbCullVisible` @2 + `VbCullCount` @3 — the BATCH-level, bump-allocated list: a compacted
//     array of surviving batch INDICES plus its atomic length. Its only reader is the rung-R2c-tail
//     readback probe (`BOYKO_VB_CULL_READBACK`), which copies its prefix to the host so
//     `boyko_app/tests/vb_cull_offscreen.rs` can assert WHICH batches survived rather than only how
//     many. NO RENDER PASS CONSUMES IT, and none can until a merged multi-draw exists
//     (`multiDrawIndirect` plus a merged vertex/index arena, neither of which this device/engine
//     has). It is a probe, and it is kept because it is the only GPU-side evidence that this module
//     rejects anything at all.
//
//   * `VbVisibleInstance` @6 — the INSTANCE-level, REGION-addressed list (rung R2d-2 allocated it,
//     R2d-3 writes it): one `uint` global instance id per surviving instance, written at
//     `base_instance + k` rather than at a bump-allocated slot. Nothing reads it yet either; from
//     rung R2d-4 the raster's VS indexes it as `gVbVisibleInstance[base_instance + SV_InstanceID]`
//     to recover the instance a compacted draw is drawing.
//
// The two differ in KIND, not just in level: a bump allocation gives each writer an arbitrary slot
// (so the list must be read together with its count, and a clamp may drop entries), while a region
// write gives each batch a FIXED, private range (so a reader indexes straight into it with no
// count, and a dropped entry is not representable). The raster needs the second property.
//
// # INVARIANT R2d-REGION-DISJOINT — regions never overlap
//
// Batch `i` owns `[d.base_instance, d.base_instance + d.instance_count)` of `VbVisibleInstance` and
// writes nowhere else. Those regions are pairwise disjoint because the host's gather assigns
// `base_instance = running` BEFORE adding that mesh's count
// (`boyko_render/src/mesh_draw.rs:815-832`) and emits no batch with a zero count, so bases are
// STRICTLY ASCENDING and batch `i`'s region ends AT OR BEFORE batch `i+1`'s base.
//
// "At or before", not "exactly where": the list this pass walks is `scene.mesh_draw`, which the
// runner builds by SKIPPING any batch whose mesh asset is not `Loaded`
// (`boyko_app/src/runner.rs:1961-1963`), so it is a SUBSEQUENCE of the gather's own list and can
// carry gaps between one region's end and the next region's base. Disjointness survives — a
// subsequence of a strictly ascending sequence is strictly ascending — while contiguity does not,
// and only disjointness is load-bearing here. Two threads therefore never write the same slot,
// which is why this pass needs NO atomic, NO groupshared and NO barrier for the region write: the
// property is structural, established on the host, not enforced by synchronisation here.
//
// # INVARIANT R2d-REGION-DEFINED — every slot the rasterizer will dereference is written THIS frame
//
// `VbVisibleInstance` is DEVICE_LOCAL and NOTHING clears it: an unwritten slot holds undefined
// memory on frame 1 and a PREVIOUS frame's residue afterwards. Rung R2d-3 satisfies the invariant
// trivially — with `keep` hardwired, `k == d.instance_count`, so the loop writes the batch's WHOLE
// region every frame.
//
// ⚠️ THE ARMING RUNG MUST PRESERVE IT. Once `keep` is real, `k < d.instance_count` for a partly
// culled batch and the tail `[base + k, base + count)` goes unwritten. That is only safe as long as
// nothing dereferences it — the record's `instanceCount` is `k`, so the rasterizer reads exactly
// `[base, base + k)`. Any consumer that indexes the region by anything OTHER than a compacted
// `SV_InstanceID` (a per-batch scan, a persistent-thread reader, a debug dump of the whole buffer)
// must either fill the tail or be given a per-batch survivor count. Do not weaken this by adding a
// reader before the fill. A tail fill is deliberately NOT written this rung: with `keep` constant it
// is provably empty, so DXC would remove it, and the `.spv` census must reflect what the module
// really contains rather than what its author intended.
//
// # NO in-shader capacity guard on the region write — and that is deliberate
//
// The bound on `base_instance + instance_count` is enforced ENTIRELY by the host
// (`present/passes/vb.rs`'s `vb_cull_batch_count_visible_clamp`, derived from the survivor list's
// own ALLOCATION, never from a host capacity constant): the dispatch covers only the PREFIX of
// batches whose regions fit, and a clamped-away batch keeps the record the host's transfer fill
// wrote — i.e. exactly pre-R2d rendering for it. The clamp-and-drop discipline `VbCullVisible`
// carries would be WRONG here: dropping a region write would leave a slot unwritten while
// `instanceCount` still reported it, and the rasterizer would then dereference it. Dropping a whole
// batch on the host is the only correct direction, and it is what happens.
//
// # ARMED AT RUNG R2c — and this file shipped INERT first, on purpose
//
// Rung R2c0 shipped this module with `visible` as the literal `true`. That was not a placeholder:
// `docs/VG-DECIDABILITY-FLOOR.md` measured this box's GPU-timing floor at 6.3 / 14.3 / 4.7 / 13.5 %
// across four runs of ONE protocol, so no cull delta is defensible without a NULL CONTROL taken in
// the same sitting — the machinery present, dispatched, and provably changing nothing. Rung R2c
// then replaced that one line with the test below.
//
// Both states are pinned at the ARTIFACT by `tests/vb_batch_cull_spv_sync.rs`, which R2c re-pinned
// rather than deleted: R2c0 asserted `OpSelect == 0` / `OpDot == 0` (no decision at all), R2c
// asserts a real one while holding `OpAtomicIAdd == 1` across the change — the compaction claim
// must survive the arming untouched. R2d-3 re-pins the same census against ITS module and adds two
// fields: the module's DECLARED BINDING SET — whether DXC keeps or STRIPS the @4/@5 declarations
// while nothing loads from them is exactly what that field REPORTS, not something this comment
// knows — and `OpControlBarrier` (expected zero, since there is no groupshared and no barrier in
// the construction; see the disjointness invariant above — read off the module, not asserted here).
//
// ⚠️ A GOLDEN CANNOT SEE THE DIFFERENCE. Every pinned scene is entirely on-screen, so a cull that
// rejects nothing renders a byte-identical image to a correct one. The evidence that this module
// actually rejects is `crates/boyko_app/tests/vb_cull_offscreen.rs`, which reads the visible COUNT
// back off the GPU (measured: 2 batches in, 1 visible out) and goes RED at `visible=2` when the
// planes are disarmed.
//
// # The AABB fields, and why they shipped one rung before they were read
//
// Rung R2c0 shipped the 32-byte descriptor with its AABB fields present and dead (DXC dead-coded
// them out of that module, which the census pinned). Fixing the LAYOUT one rung early is what let
// R2c change the shader body and the host's AABB fill while touching neither the descriptor-set
// layout, the buffer sizes, nor the graph — the churn a "just the count for now" struct would have
// forced on the very next rung. The reserved `pad` word paid the same dividend at rung R2d-3: it
// became `base_instance` with the stride, the allocation and the chunked upload all unchanged.
//
// The corners are `+/- VbBatchDesc::UNBOUNDED` whenever the host could not compute real bounds
// (a mesh still streaming in, or the C0 zero-vertex sentinel). That magnitude is large but FINITE,
// never an infinity: `dot(n, p) + d` against an infinite corner can produce a NaN, and a NaN
// compare picks the OTHER operand under `NMin`/`NMax` rather than propagating. So an unfilled
// descriptor reads as "unbounded box" and survives every plane — the CONSERVATIVE direction, which
// is the same one the test below errs in: a wasted draw, never a false cull.
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

// The workgroup width, used by `[numthreads]` below so this file has ONE spelling of it rather
// than a constant beside a literal that could drift from it. The host mirrors it as
// `VB_BATCH_CULL_LOCAL_SIZE_X` and dispatches `ceil(batch_count / width)` groups, so the tail group
// runs partly out of range and is trimmed by the `i >= pc.batch_count` guard below.
//
// The two spellings CANNOT be one symbol across the language boundary, so they are held together at
// the ARTIFACT: `tests/vb_batch_cull_spv_sync.rs` reads the compiled `LocalSize` out of the module
// and asserts it equals the host constant. A silent divergence would either leave tail batches
// unvisited (stale `instanceCount`) or over-dispatch — neither of which a golden would show on a
// scene whose batch count divides the width.
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
// explicit padding member. Mirrors the host `VbBatchDesc`
// (`present/scene_types.rs` — `aabb_min` @0, `instance_count` @12, `aabb_max` @16,
// `base_instance` @28).
struct VbBatchDescGpu {
    float3 aabb_min;       // world-space AABB min corner (`-UNBOUNDED` when host bounds are absent)
    uint   instance_count; // the `instanceCount` a VISIBLE batch draws, and this batch's region size
    float3 aabb_max;       // world-space AABB max corner (`+UNBOUNDED` when host bounds are absent)
    uint   base_instance;  // rung R2d-3: this batch's start in the instance ring AND in @6's region
};

// binding 1: the per-batch descriptors, transfer-filled by the host each frame (read-only).
StructuredBuffer<VbBatchDescGpu> VbBatchDesc : register(t1);

// binding 2: the compacted visible-BATCH list (RW). Written here; read by the rung-R2c-tail
// readback probe (`BOYKO_VB_CULL_READBACK`), which copies its prefix to the host so a test can
// assert WHICH batches survived rather than only how many. No RENDER pass consumes it — see this
// file's "TWO visible lists" section for why it stays and what distinguishes it from @6.
RWStructuredBuffer<uint> VbCullVisible : register(u2);

// binding 3: the visible-batch counter (RW, one u32 at element 0). Transfer-zeroed by the host
// each frame ahead of the TRANSFER -> COMPUTE barrier the graph derives.
RWStructuredBuffer<uint> VbCullCount : register(u3);

// One per-instance row of the VB instance ring. 64 bytes, mirroring the host
// `boyko_render::instance_model::VbInstanceRow` (`instance_model.rs:221-233` + its offset
// const-asserts: `affine` @0 as three interleaved `[linear_row.xyz | translation]` quads,
// `mesh_id` @48, a 12-byte `_pad` @52) and the SAME HLSL spelling `vb_geom_fetch.hlsli:44-50`
// already uses, so the two mirrors of one host type cannot drift into different layouts.
struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;
    uint3  _pad;
};

// binding 4: the per-instance rows (read-only) — the affine and the `mesh_id` the ARMING rung needs
// to build an instance's world box. Bound since rung R2d-2 and declared here since R2d-3; while
// `keep` is hardwired the module loads nothing from it, so DXC may drop it from the compiled
// binding set. That is legal (a descriptor no shader dereferences is still written by the host) and
// is what the census field records rather than assumes.
StructuredBuffer<VbInstanceRow> gVbInstances : register(t4);

// One mesh's LOCAL-space AABB, keyed by `mesh_id`. 32 bytes, mirroring the host
// `boyko_render::mesh_geometry_table::MeshLocalBounds` (`mesh_geometry_table.rs:175-199` + its
// offset const-asserts: `min` @0, `_p0` @12, `max` @16, `_p1` @28 — `float3`-then-`uint` twice, so
// each half is a whole std430 lane and no trailing pad member is needed).
//
// `bmin`/`bmax` rather than the host's `min`/`max`: those are HLSL intrinsic names, and the LAYOUT
// is what must mirror exactly, not the identifiers. An INVERTED row (`any(bmin > bmax)`) is the
// "bounds unknown" sentinel — a slot for a mesh that never registered, or one whose fold was
// degenerate — and a consumer must read it as KEEP, never as "empty, therefore cull": absence of
// bounds is not evidence of invisibility (that type's own doc states the obligation).
struct MeshLocalBounds {
    float3 bmin;
    uint   _p0;
    float3 bmax;
    uint   _p1;
};

// binding 5: the per-mesh local bounds table (read-only), one row per `mesh_id`. Same
// declared-but-unread status as @4 this rung, for the same reason.
StructuredBuffer<MeshLocalBounds> gMeshBounds : register(t5);

// binding 6: the per-INSTANCE survivor list (RW). Batch `i` writes ONLY
// `[base_instance, base_instance + k)`; see INVARIANT R2d-REGION-DISJOINT (why no synchronisation
// is needed) and INVARIANT R2d-REGION-DEFINED (what the arming rung owes this buffer).
RWStructuredBuffer<uint> VbVisibleInstance : register(u6);

// The number of camera-frustum planes. Fixed order: left, right, bottom, top, near, far — the
// SAME order `boyko_render::frustum::frustum_planes_from_view_proj` emits.
static const uint FRUSTUM_PLANE_COUNT = 6u;

// The batch-cull push constants. Mirrors the host `VbBatchCullPush` (104 bytes).
struct VbBatchCullPush {
    // Rung R2c: the six frustum planes as (a, b, c, d), inside => a*x + b*y + c*z + d >= 0.
    // EXTRACTED ON THE HOST from the same 64 push bytes the raster's VS reads as `view_proj`, and
    // pushed here rather than re-derived: one extraction, two consumers, so a disagreement between
    // this shader and its host oracle is a shader bug rather than a math bug. UNNORMALISED — the
    // sign of a comparison against zero is scale-invariant, and normalising would introduce a
    // division that a degenerate row turns into a NaN.
    float4 planes[6];
    uint batch_count;  // number of live `DrawBatch` records this frame (the range guard)
    uint visible_cap;  // element capacity of `VbCullVisible` (the clamp-and-drop bound)
};
[[vk::push_constant]] VbBatchCullPush pc;

// Rung R2c: the conservative rejection test — true iff the world AABB is WHOLLY in the negative
// half-space of at least one plane, and therefore certainly invisible.
//
// For each plane, `dist` is the centre's signed distance and `radius` is the box's extent projected
// onto the plane normal, so `dist + radius` is the signed distance of the box's FARTHEST corner
// along that normal. Still negative => all eight corners are outside. That is an EXACT statement
// about the box, not an approximation.
//
// The converse is deliberately not computed: a box straddling two planes' outsides without being
// wholly outside either is reported VISIBLE. A false reject would delete geometry from the frame; a
// false keep costs one wasted draw. Do not "tighten" this without re-deriving that guarantee.
//
// A NaN anywhere makes every comparison false, so the box reads VISIBLE — the same safe direction.
//
// Rung R2d takes this same fn for its per-INSTANCE test: an instance's world box is its mesh's
// local box (`gMeshBounds[mesh_id]`) Arvo-transformed by its affine, and the plane test on it is
// identical. One test, two granularities.
bool aabb_outside_frustum(float3 mn, float3 mx) {
    const float3 c = (mn + mx) * 0.5;
    const float3 h = (mx - mn) * 0.5;
    for (uint p = 0u; p < FRUSTUM_PLANE_COUNT; ++p) {
        const float4 pl = pc.planes[p];
        const float dist = dot(pl.xyz, c) + pl.w;
        const float radius = dot(abs(pl.xyz), h);
        if (dist + radius < 0.0) {
            return true;
        }
    }
    return false;
}

[numthreads(LOCAL_SIZE_X, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    const uint i = tid.x;
    // The tail group's out-of-range lanes. Every buffer access below is behind this guard, so no
    // lane can touch a record past the live batch count — `robustBufferAccess` is OFF on this
    // device, so an unguarded lane would be a real out-of-bounds device write.
    if (i >= pc.batch_count) {
        return;
    }

    const VbBatchDescGpu d = VbBatchDesc[i];

    // LEVEL 1, armed at rung R2c. Rung R2c0 shipped this line as the literal `true` — the null
    // control `docs/VG-DECIDABILITY-FLOOR.md` demands — and this is the one line that rung promised
    // would change. A batch whose descriptor was never filled carries the `VbBatchDesc::UNBOUNDED`
    // corners, which survive every plane, so an unfilled descriptor degrades to "keep" rather than
    // to "cull".
    const bool visible = !aabb_outside_frustum(d.aabb_min, d.aabb_max);

    // LEVEL 2, rung R2d-3: compact this batch's survivors into its OWN region of the survivor list.
    // `k` is the compaction cursor and, after the loop, the survivor COUNT.
    //
    // ⚠️ `keep` is HARDWIRED `true` this rung — the ONE expression the arming rung replaces (with
    // the instance's own world box against the same six planes, via `gVbInstances[base + j]`'s
    // affine and `gMeshBounds[mesh_id]`'s local box). With it constant, `k == d.instance_count`,
    // so the record store below reproduces rung R2c's value EXACTLY and the whole region is
    // rewritten every frame (INVARIANT R2d-REGION-DEFINED). No `groupshared`, no barrier and no
    // atomic: the region is this thread's alone (INVARIANT R2d-REGION-DISJOINT).
    //
    // The write is unguarded by design — the region's fit is a HOST precondition (see this file's
    // "NO in-shader capacity guard" section), and a clamp here would corrupt rather than protect.
    uint k = 0u;
    for (uint j = 0u; j < d.instance_count; ++j) {
        const bool keep = true;
        if (keep) {
            VbVisibleInstance[d.base_instance + k] = d.base_instance + j;
            ++k;
        }
    }

    // Word 1 of the record: the SURVIVOR COUNT, still gated by the level-1 verdict. A culled batch
    // writes 0, which draws nothing while leaving the rest of the record — and therefore the host's
    // `first_instance == 0` invariant — untouched.
    //
    // Storing `k` rather than `d.instance_count` is the rung's substitution, and it is inert by
    // construction: with `keep` hardwired the two are equal, so this stores the same word rung R2c
    // stores for every batch, visible or not. The `visible ?` gate is NOT folded into `keep` on
    // purpose — folding it would leave a level-1-culled batch's region unwritten, which is exactly
    // the tail the region invariant above forbids anyone to create silently.
    VbIndirect.Store(
        i * DRAW_INDEXED_INDIRECT_STRIDE + INSTANCE_COUNT_OFFSET,
        visible ? k : 0u);

    // Claim a slot in the compacted BATCH list (lock-free global bump), AFTER the loop so the
    // condition can see `k`. CLAMP-AND-DROP, the same overflow discipline `cluster_cull.hlsl`'s
    // global claim carries: a slot past the capacity drops the entry rather than writing out of
    // bounds. The counter still counts it, so a future reader comparing `count` against
    // `visible_cap` can SEE that the list was trimmed instead of silently reading a short list as
    // complete.
    //
    // `k > 0u` is the forward-looking half of the gate — once `keep` is real it makes this the list
    // of batches with SURVIVORS rather than of batches whose union box passed. It changes nothing
    // today: the host's gather emits no batch with a zero instance count
    // (`boyko_render/src/mesh_draw.rs:815-832` — `counts[m] == 0` collapses to `resolved == None`
    // and pushes no `DrawBatch`), so `k > 0` holds for every dispatched lane and the effective gate
    // is still `visible`, which is what `vb_cull_offscreen.rs` measures (2 batches in, 1 out).
    if (visible && k > 0u) {
        uint slot = 0u;
        InterlockedAdd(VbCullCount[0], 1u, slot);
        if (slot < pc.visible_cap) {
            VbCullVisible[slot] = i;
        }
    }
}
