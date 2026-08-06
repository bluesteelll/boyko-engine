// VG rungs R2c0/R2c/R2d-3/R2d-6: the VisibilityBuffer cull compute pass
// (`vb_batch_cull.comp.hlsl`).
//
// One invocation per `DrawBatch`. There are TWO LEVELS, and they are different tests on different
// data:
//
//   LEVEL 1 (per BATCH, rung R2c) — `VbBatchDesc[i]`'s world AABB is the UNION over that batch's
//     instances. It is tested against the six pushed frustum planes; a batch wholly outside one
//     plane is certainly invisible in every instance, so the level-2 result for it is zero
//     survivors. This is the cheap early verdict.
//
//   LEVEL 2 (per INSTANCE, rungs R2d-3 .. R2d-6) — the loop below walks this batch's
//     `instance_count` instances and compacts the survivors into `VbVisibleInstance`. Rung R2d-3
//     shipped that loop with its `keep` predicate HARDWIRED to `true`: the machinery — the region
//     write, the survivor count, the relocated counter bump — present, dispatched, and provably
//     changing nothing OBSERVABLE IN THE IMAGE, because `k` then equalled `d.instance_count`. That
//     was the R2c0-before-R2c discipline repeated deliberately: the null control
//     `docs/VG-DECIDABILITY-FLOOR.md` demands has to be present in the MEASURED configuration to
//     be a control at all, so the loop's real cost (`instance_count` dependent iterations, each a
//     global store, run by ONE lane per batch) was already being paid by the control.
//
//     **RUNG R2d-6 IS THE ARMING RUNG, AND IT REPLACES THAT ONE EXPRESSION.** `keep` is now the
//     instance's OWN world box against the same six planes: its mesh's LOCAL box from
//     `gMeshBounds[row.mesh_id]`, Arvo-transformed by `gVbInstances[base_instance + j]`'s affine.
//     Nothing else moved — not the loop shape, not the region write, not the record store, not the
//     counter bump. From this rung `k <= d.instance_count`, and the two are equal only for a batch
//     nothing rejects.
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
//     `base_instance + k` rather than at a bump-allocated slot. Since rung R2d-4 the raster's VS
//     indexes it as `visible_instances[base_instance + SV_InstanceID]` to recover the instance a
//     compacted draw is drawing — which is what makes rung R2d-6's compaction reach the image at
//     all.
//
// The two differ in KIND, not just in level: a bump allocation gives each writer an arbitrary slot
// (so the list must be read together with its count, and a clamp may drop entries), while a region
// write gives each batch a FIXED, private range (so a reader indexes straight into it with no
// count, and a dropped entry is not representable). The raster needs the second property.
//
// # THE CONSERVATIVE DIRECTION — derived, not asserted (rung R2d-6)
//
// The two errors this predicate can make are NOT symmetric. A false KEEP costs one wasted
// instance: it is rasterized, covers no pixel, and the frame is identical. A false REJECT DELETES
// GEOMETRY the camera can see. So every step of the level-2 test is biased the same way, and the
// bias is a property of the construction rather than a hope:
//
//   * the plane test rejects ONLY when the instance's own world box is WHOLLY in one plane's
//     negative half-space — an exact statement about that box, never an approximation of the
//     frustum (`aabb_outside_frustum` below derives it from `dist + radius`, the signed distance
//     of the box's FARTHEST corner). A box straddling two planes' outsides is reported VISIBLE;
//   * a NaN anywhere makes every `<` false, so the box survives all six planes => KEEP;
//   * UNKNOWN bounds => KEEP, tested BEFORE the transform (next section);
//   * a DEGENERATE affine (zero linear part) can only SHRINK the world box, and the one case where
//     that would invert the guarantee is the unknown-bounds sentinel, which the previous point
//     removes before the arithmetic runs.
//
// A rung that "tightens" any of these must re-derive this direction first.
//
// # ⚠️ UNKNOWN BOUNDS ARE TESTED BEFORE THE TRANSFORM, AND THE ORDER IS LOAD-BEARING
//
// `gMeshBounds[mesh_id]` holds the INVERTED sentinel `min = +1e30`, `max = -1e30`
// (`boyko_render::mesh_geometry_table::MeshLocalBounds::UNKNOWN`) for every slot that is not a
// registered mesh's real fold: a mesh still streaming in, the reserved slot, or the C0
// zero-vertex mesh. Absence of bounds is NOT evidence of invisibility, so it must read as KEEP —
// the obligation that type's own doc places on every consumer.
//
// Testing it AFTER the Arvo fold would INVERT the one-way guarantee, and a critic found exactly
// that inversion in an earlier draft of this rung. The sentinel is large-but-FINITE by
// construction (an infinite corner makes `dot(n, p) + d` a NaN, and a NaN picks the OTHER operand
// under `NMin`/`NMax` instead of propagating). Folded, it gives `lc = (1e30 + -1e30) * 0.5 = 0`
// and `lh = -1e30`; push THAT through a degenerate affine — a zero linear part — and every
// `wh[r] = dot(abs(row.xyz), lh)` is `0 * -1e30 = 0`. The "unbounded" box has collapsed to a POINT
// at the instance's translation, which the frustum test can then REJECT. The conservative
// direction survives only because the sentinel branch wins before any of that arithmetic is
// believed.
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
// # INVARIANT R2d-REGION-DEFINED — every slot a reader DEREFERENCES is written THIS frame
//
// `VbVisibleInstance` is DEVICE_LOCAL and NOTHING clears it: an unwritten slot holds undefined
// memory on frame 1 and a PREVIOUS frame's residue afterwards. Rung R2d-3 satisfied the invariant
// trivially — with `keep` hardwired, `k == d.instance_count`, so the loop wrote the batch's WHOLE
// region every frame.
//
// ⚠️ RUNG R2d-6 NO LONGER SATISFIES IT TRIVIALLY — AND NO TAIL FILL IS NEEDED. With `keep` real,
// `k < d.instance_count` for a partly culled batch and the tail `[base + k, base + count)` keeps
// last frame's residue. The invariant is stated over slots a reader DEREFERENCES, and EVERY reader
// of this buffer is bounded by the SAME `k` this pass stores into word 1 of the record:
//
//   * the RASTERIZER — `vb_raster.vs.hlsl`'s indirected arm reads
//     `visible_instances[pc.base_instance + SV_InstanceID]`, and `vkCmdDrawIndexedIndirect` takes
//     `instanceCount` from the record written below, so `SV_InstanceID < k` and the read stays
//     inside `[base, base + k)` — every one of them written this frame. No shift widens it:
//     `first_instance` is 0 in every record (see the section on that field below);
//   * the READBACK PROBE — `boyko_app::runner`'s `format_vb_cull_probe_line` decodes PER BATCH and
//     slices `[base, base + <that batch's record word>)`, i.e. the same `k` again, never a flat
//     prefix of the allocation;
//   * NOTHING ELSE. No other shader in `crates/boyko_rhi_vulkan/shaders` names this buffer.
//
// So a fill is not merely unnecessary, it is harmful: dead stores DXC may or may not eliminate
// would move the `.spv` census with no behavioural reason to.
//
// The case where the record word does NOT bound the written region is a DIFFERENT one, and the
// host already handles it: a batch CLAMPED AWAY from the dispatch (`vb_cull_batch_count_visible_clamp`,
// or the record/descriptor capacities) keeps the full `instanceCount` its transfer fill wrote while
// its region goes ENTIRELY unwritten. `record_vb`'s `i < batch_count` term clears that draw's
// visible-indirection bit, so its VS computes `base_instance + SV_InstanceID` literally and never
// touches this buffer.
//
// Any FUTURE consumer that indexes the region by anything OTHER than a compacted `SV_InstanceID`
// (a per-batch scan, a persistent-thread reader, a debug dump of the whole buffer) must either
// fill the tail or be given a per-batch survivor count. Do not weaken this by adding such a reader
// before the fill.
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
// # EACH LEVEL SHIPPED INERT ONE RUNG BEFORE IT WAS ARMED, on purpose
//
// Rung R2c0 shipped this module with `visible` as the literal `true`; rung R2d-3 shipped `keep` the
// same way. Neither was a placeholder: `docs/VG-DECIDABILITY-FLOOR.md` measured this box's
// GPU-timing floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of ONE protocol, so no cull delta
// is defensible without a NULL CONTROL taken in the same sitting — the machinery present,
// dispatched, and provably changing nothing. Rung R2c replaced the first of those literals with the
// level-1 test below; rung R2d-6 replaced the second with the level-2 test in the loop.
//
// Both states are pinned at the ARTIFACT by `tests/vb_batch_cull_spv_sync.rs`, which R2c re-pinned
// rather than deleted: R2c0 asserted `OpSelect == 0` / `OpDot == 0` (no decision at all), R2c
// asserts a real one while holding `OpAtomicIAdd == 1` across the change — the compaction claim
// must survive the arming untouched. R2d-3 re-pinned the same census against ITS module and added
// two fields — the module's DECLARED BINDING SET and `OpControlBarrier` (zero: there is no
// groupshared and no barrier in the construction; see the disjointness invariant above) — and
// R2d-6 re-pins every count again, against the ARMED module, for the same reason R2c did rather
// than deleting the pin.
//
// R2d-3 MEASURED that DXC STRIPS a declared-but-unloaded resource: its module's binding set was
// `[0,1,2,3,6]`, without the @4/@5 it declares. That measurement is what makes the same field the
// strongest single check on THIS rung — the armed module reads `gVbInstances` (@4) and
// `gMeshBounds` (@5), so both MUST reappear. A module still reporting `[0,1,2,3,6]` after the
// arming would mean the arming reads neither the instance rows nor the bounds, i.e. that it
// silently did nothing — precisely the failure an all-on-screen golden cannot see. `OpAtomicIAdd`
// must likewise still be exactly 1: the arming has no business touching the compaction.
//
// ⚠️ A GOLDEN CANNOT SEE THE DIFFERENCE. Every pinned scene is entirely on-screen, so a cull that
// rejects nothing renders a byte-identical image to a correct one. The evidence that this module
// actually rejects is read off the GPU instead: `crates/boyko_app/tests/vb_cull_offscreen.rs` for
// LEVEL 1 (2 batches in, 1 visible out; RED at `visible=2` when the planes are disarmed), and
// `vb_inst_cull_narrow.rs` + its `vb_inst_cull_wide.rs` control for LEVEL 2 (one instance INTERIOR
// to each of two batches off-screen; `inst=[2,2]` armed against `[3,3]` inert, with the wide
// framing's `[3,3]` unchanged across the arming so a cull that rejected VISIBLE geometry reds
// there rather than reading as success here).
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
// `boyko_render::instance_model::VbInstanceRow` (grep the type + its offset const-asserts:
// `affine` @0 as three interleaved `[linear_row.xyz | translation]` quads, `mesh_id` @48,
// `flags` @52, an 8-byte `_pad` @56) and the SAME HLSL spelling `vb_geom_fetch.hlsli` already
// uses, so the two mirrors of one host type cannot drift into different layouts.
//
// VG R3 piece 2 step P2-2 split the host's former 12-byte `_pad` @52 into a `flags` word @52
// plus an 8-byte pad @56. The HLSL spelling below is DELIBERATELY unchanged: `uint3 _pad` at
// offset 52 has the identical layout either way, so renaming it here would re-DXC four
// modules for zero layout benefit against a charter that keeps every `.spv` byte-frozen.
// What the rename would have bought is stated instead: word @52 is the per-instance FLAGS
// word, bit 0 = "this instance's entity carries `OcclusionCulling`", bits 1..31 reserved and
// written zero. NOTHING on the device reads it as of P2-2 — piece 3 renames the field and is
// the first code to load it, out of the `gVbInstances[base_instance + j]` fetch this shader
// already issues (so the flag costs zero extra device fetches).
struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;
    uint3  _pad;
};

// binding 4: the per-instance rows (read-only) — the affine and the `mesh_id` the level-2 test
// builds an instance's world box from. Bound since rung R2d-2, declared since R2d-3, and LOADED
// since rung R2d-6: the row at `d.base_instance + j` is this batch's `j`-th instance, the same
// addressing `vb_raster.vs.hlsl` uses for the same ring.
//
// `base_instance + j` is in range for every lane this pass dispatches, and it is the SAME bound the
// region write below already relies on: the host's `vb_cull_batch_count_visible_clamp` dispatches
// only the PREFIX of batches whose `[base, base + count)` fits the survivor list, and that list is
// allocated with one `uint` per instance-ring row (`INSTANCE_CAPACITY` in both). So a batch that
// clears the clamp for @6 clears it for this buffer too. `robustBufferAccess` is OFF on this
// device — that host clamp is the guarantee, not a hardware clamp.
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

// binding 5: the per-mesh local bounds table (read-only), one row per `mesh_id`. Loaded since rung
// R2d-6, like @4.
//
// `gMeshBounds[row.mesh_id]` is in range for every row this pass reads: `mesh_id` is a SLOT index
// into the bindless geometry table (the same number `vb_geom_fetch.hlsli` uses to pick
// `gMeshVerts[mesh_id]`), and `MeshGeometryTable::new` sizes this buffer at
// `set.capacity() * MESH_LOCAL_BOUNDS_BYTES` — one row per slot — with every row prefilled to the
// inverted sentinel before any mesh registers. `robustBufferAccess` is OFF on this device, so that
// sizing is the guarantee, not a clamp.
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
// Rung R2d-6 calls this same fn for the per-INSTANCE test: an instance's world box is its mesh's
// local box (`gMeshBounds[mesh_id]`) Arvo-transformed by its affine, and the plane test on it is
// identical — reused VERBATIM rather than transcribed, so the two granularities cannot drift into
// two different tests. One test, two granularities; the host oracle
// (`boyko_render::frustum::aabb_outside_frustum`) is the third spelling and the only one this
// module is compared against.
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

    // LEVEL 2, ARMED at rung R2d-6: compact this batch's survivors into its OWN region of the
    // survivor list. `k` is the compaction cursor and, after the loop, the survivor COUNT.
    //
    // `keep` is the ONE expression this rung replaced (rung R2d-3 shipped it as the literal
    // `true`). It is now the instance's own world box against the same six planes:
    //
    //   1. the row `gVbInstances[base + j]` — the 3x4 row-major affine as three
    //      `[linear_row.xyz | translation]` quads, plus the `mesh_id` lane;
    //   2. that mesh's LOCAL box `gMeshBounds[mesh_id]`;
    //   3. the UNKNOWN-BOUNDS arm, BEFORE any transform — `keep` starts `true` and only a KNOWN
    //      box may lower it, so the inverted sentinel survives without its (meaningless) fold ever
    //      being believed. See this file's "UNKNOWN BOUNDS ARE TESTED BEFORE THE TRANSFORM"
    //      section for the degenerate-affine collapse that makes the ORDER load-bearing;
    //   4. otherwise the Arvo abs-matrix fold — `wc[r] = dot(row[r].xyz, lc) + row[r].w`,
    //      `wh[r] = dot(abs(row[r].xyz), lh)` — which is the SAME fold
    //      `boyko_render::csm_caster::arvo_transform` performs on the host, and then the same
    //      plane test level 1 just ran on the union box.
    //
    // No `groupshared`, no barrier and no atomic: the region is this thread's alone (INVARIANT
    // R2d-REGION-DISJOINT). The write is unguarded by design — the region's fit is a HOST
    // precondition (see this file's "NO in-shader capacity guard" section), and a clamp here would
    // corrupt rather than protect.
    uint k = 0u;
    for (uint j = 0u; j < d.instance_count; ++j) {
        const uint g = d.base_instance + j;
        const VbInstanceRow row = gVbInstances[g];
        const MeshLocalBounds b = gMeshBounds[row.mesh_id];
        // Seeded KEEP, and only a known box may lower it: the conservative direction is the
        // DEFAULT of this predicate rather than a case it remembers to handle.
        bool keep = true;
        if (!any(b.bmin > b.bmax)) {
            const float3 lc = (b.bmin + b.bmax) * 0.5;
            const float3 lh = (b.bmax - b.bmin) * 0.5;
            const float3 wc = float3(dot(row.r0.xyz, lc) + row.r0.w,
                                     dot(row.r1.xyz, lc) + row.r1.w,
                                     dot(row.r2.xyz, lc) + row.r2.w);
            const float3 wh = float3(dot(abs(row.r0.xyz), lh),
                                     dot(abs(row.r1.xyz), lh),
                                     dot(abs(row.r2.xyz), lh));
            keep = !aabb_outside_frustum(wc - wh, wc + wh);
        }
        if (keep) {
            // The STORED value is the GLOBAL instance index, never the compacted slot `k` — see
            // `vb_raster.vs.hlsl`'s INVARIANT R2d-EXPORT-IS-GLOBAL for everything keyed on it.
            VbVisibleInstance[d.base_instance + k] = g;
            ++k;
        }
    }

    // Word 1 of the record: the SURVIVOR COUNT, still gated by the level-1 verdict. A culled batch
    // writes 0, which draws nothing while leaving the rest of the record — and therefore the host's
    // `first_instance == 0` invariant — untouched.
    //
    // Storing `k` rather than `d.instance_count` was rung R2d-3's substitution, inert while `keep`
    // was constant. At rung R2d-6 it is the OBSERVABLE: `k` is now the number of instances of this
    // batch that survived their own frustum test, and it is the word `vkCmdDrawIndexedIndirect`
    // fetches — so it is both what the frame draws and what bounds every read of the survivor
    // region (INVARIANT R2d-REGION-DEFINED).
    //
    // The `visible ?` gate is NOT folded into `keep` on purpose. Folding it would skip the level-2
    // loop for a level-1-culled batch, and while that batch's record reads 0 either way, the two
    // states differ for anything that later learns to read the region: skipping leaves it holding
    // an arbitrary earlier frame's survivors rather than this frame's (empty, by the union
    // implication) answer.
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
    // `k > 0u` was the forward-looking half of the gate, and rung R2d-6 is what makes it live: this
    // is now the list of batches with SURVIVORS, not of batches whose union box passed. Before the
    // arming the two coincided — the host's gather emits no batch with a zero instance count
    // (`boyko_render/src/mesh_draw.rs:815-832` — `counts[m] == 0` collapses to `resolved == None`
    // and pushes no `DrawBatch`), so `k > 0` held for every dispatched lane. Armed, a batch whose
    // union box straddles the frustum while EVERY member fails its own test now drops out of this
    // count. So `visible` is a number to be MEASURED per scene from this rung on, never carried
    // over from an inert run — the corpus gate states its own derivation rather than assuming it.
    if (visible && k > 0u) {
        uint slot = 0u;
        InterlockedAdd(VbCullCount[0], 1u, slot);
        if (slot < pc.visible_cap) {
            VbCullVisible[slot] = i;
        }
    }
}
