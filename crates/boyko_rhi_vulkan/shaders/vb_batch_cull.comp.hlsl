// VG rungs R2c0/R2c/R2d-3/R2d-6 + VG R3 piece 3 steps P3-3/P3-4: the VisibilityBuffer cull compute
// pass (`vb_batch_cull.comp.hlsl`).
//
// # TWO PHASES, ONE MODULE (VG R3 piece 3 steps P3-3/P3-4, plan D5)
//
// Since P3-3 the host dispatches this module TWICE per armed-split frame and selects the body with
// the `pc.phase` push word: `VB_CULL_PHASE_EARLY` before the early raster, `VB_CULL_PHASE_LATE`
// after the depth pyramid this frame's raster fed.
//
// The fork was NOT deferred past P3-3, deliberately: that step is the one that records the second
// dispatch, and a second dispatch of a module with no fork re-runs the EARLY body — rewriting the
// survivor list and every record's `instanceCount` after the early raster has already fetched them,
// with the same numbers on a static scene and therefore invisibly to every golden.
//
// **Step P3-4 gives the fork its two BODIES.** The early phase partitions its frustum survivors
// into DRAWN NOW (`VbVisibleInstance`, unchanged) and DEFERRED (`VbLateVisible`, new); the late
// phase re-tests the deferred set against the pyramid this frame's early raster fed, compacts the
// survivors in place and writes `VbIndirectLate[i].instanceCount`. Both phases share ONE
// `occlusion_reject` leaf — the whole point of one module over a `-D` variant pair, because drift
// in the direction where the LATE test is stricter than the EARLY one deletes geometry.
//
// # ⚠️ P3-4 IS ARMED ONLY BY `pc.occ_flags`, WHICH THE HOST STILL PUSHES AS 0
//
// `defer` is `occ_armed && …`, so with `occ_flags == 0` it is identically FALSE and the early loop
// degrades to the pre-P3-4 loop EXACTLY — not to something merely similar. `n_defer` is then 0 for
// every batch, so the late phase's loop body never executes and it stores `instanceCount = 0`, the
// same word the host's `vb_indirect_late_upload` already seeded. No frame can move at this step;
// the arming commit is P3-6.
//
// # ⚠️ THE OBLIGATION P3-6 OWES THIS MODULE: `ARMED` MUST IMPLY `path_vb_occlusion_split()`
//
// `declare_vb_graph` declares `vb_batch_cull`'s `vb_late_visible` / `vb_late_count` WRITES under
// `occlusion_split` only. This module performs them under `pc.occ_flags & VB_CULL_OCC_ARMED`. The
// two stores are therefore gated on the ARMED bit rather than issued unconditionally, so that
// PERFORMED is a subset of DECLARED in the one direction that is safe: a declared write that does
// not happen costs a barrier nobody needed, while a performed write that was not declared is the
// undeclared-access class this campaign has already paid for. The host side of that containment —
// setting `VB_CULL_OCC_ARMED` exactly when `path_vb_occlusion_split()` holds — lands at P3-6, and
// this comment is the statement of what it owes. (`present/passes/vb.rs` already debug-asserts the
// weaker `ARMED ⇒ scene.hzb.is_some()`.)
//
// # ⚠️ THE PYRAMID IS READ POINT-SAMPLED, AND THAT IS A SOUNDNESS PROPERTY (plan D7)
//
// `gHzbPyramid` is a `Texture2D<float>` read with `.Load(int3(x, y, level))` — integer coordinates,
// an explicit mip, NO sampler anywhere in this piece. A bilinear blend of four min-reduced texels is
// a convex combination: it lies strictly between their min and max and therefore bounds the
// footprint from NEITHER side. Under reverse-Z the stored value must be <= every depth in the
// footprint, and a blend can be GREATER — which rejects something visible. False negatives are
// missing geometry, the one failure mode that is not recoverable. `tests/vb_batch_cull_spv_sync.rs`
// pins `OpTypeSampler == 0` and `OpImageSample* == 0` so the structural claim has an artifact-level
// gate that can go red.
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
//   * `VbLateVisible` @7 + `VbLateCount` @11 — VG R3 piece 3 step P3-4's THIRD list, and it is the
//     SECOND KIND again: region-addressed by the very same `VbBatchDesc` fields, holding the very
//     same GLOBAL instance indices. The early phase fills `[base, base + n_defer)` with the
//     instances it DEFERRED; the late phase compacts the survivors into `[base, base + n_keep)` and
//     writes `n_keep` into the late draw record. A SEPARATE buffer rather than two-ended packing
//     inside @6, for one decisive reason among several: two-ended packing would force
//     `vb_raster.vs.hlsl` to gain a DESCENDING index path and a third flags bit, and piece 3 is the
//     first piece whose change is supposed to move pixels — if the rasteriser's `.spv` also moved,
//     a pixel diff could not separate "the cull rejected wrongly" from "the VS indexes wrongly".
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
// **VG R3 piece 3 step P3-4 re-pins the same field once more, and for the same reason.** P3-2 bound
// FIVE more descriptors (@7..@11) while this module still declared seven, so DXC stripped all five
// and the set stayed `[0,1,2,3,4,5,6]`. This step is the one that LOADS them — the candidate list,
// the uniform block, the pyramid, the late records and the late counts — so all twelve must appear,
// each in its own named assertion. A set still reporting seven after this step would mean the
// occlusion leaf never reached the artifact, which renders a byte-identical image on every pinned
// scene. `OpAtomicIAdd` must STILL be exactly 1: a partition is not a compaction change.
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
// # `-D VB_CULL_DEBUG_PROBE=1` — THE OCCLUSION LEAF'S DIAGNOSTIC SINK (a SECOND artifact)
//
// `hzb_verdict_oracle_gate.rs`'s boundary corpus decides the verdict and NOTHING ELSE: it sees a
// partition, so a `depth_near` that disagrees with the oracle by one ULP is observable only as a
// failure on the exactly-equal arm, with no way to read the two values or their distance. That
// failure has now been MEASURED, so the leaf's own intermediates must become readable.
//
// This variant, and ONLY this variant, declares `VbCullDebug` @12 and writes an 8-word record per
// instance at every exit of `occlusion_reject` — the stage that fired, `depth_near`, `occ`, the
// selected level and the four tap coordinates.
//
// ⚠️ WHAT THE MEASUREMENT SAID, AND WHAT IT COST. Over 72 boundary probes on the shipping module the
// window rect, the selected level and all four taps were IDENTICAL to the oracle's on every probe,
// while `depth_near` differed on 6 of them by at most 1 ULP — in BOTH directions, i.e. a rounding
// difference and not a bias — and two probes therefore PARTITIONED differently. The leaf's only
// inexact step is the reciprocal, and Vulkan's precision appendix allows `OpFDiv` 2.5 ULP at 32-bit
// while Rust's divide is the IEEE 0.5-ULP one, so no amount of qualification on the fold could have
// closed it. THE DIVISION WAS THEREFORE REMOVED FROM THE DECISION (see step 6 of the leaf), and
// `depth_near` survives HERE ONLY — as a reported diagnostic, computed under this macro and nowhere
// else, so the shipping module cannot even spell the quantity that used to decide.
//
// ⚠️ THE BASE ARTIFACT CARRIES NO DIAGNOSTIC CODE, BY CONSTRUCTION AND NOT BY INSPECTION. Every
// probe-only statement — the sink, the two record writers, the slot, and `depth_near` itself — is
// inside `#ifdef VB_CULL_DEBUG_PROBE`, so with the macro undefined none of it is in the token stream
// the compiler is handed. That is stronger than "DXC will dead-code it": no inference about
// elimination is being made, which is the `frozen-base` discipline `deferred_pbr.hlsl`'s
// `TERMINATOR_WRAP` row states in the same words.
//
// The diagnostic step's ADDITIONS therefore could not move `vb_batch_cull.comp.spv`, and
// `tests/vb_batch_cull_spv_sync.rs`'s byte gate executed that claim. ⚠️ It does NOT follow that the
// base artifact is frozen: the DECISION change that the diagnostic measured — the division-free
// verdict below — is in both builds and moves both `.spv`. Byte-identity of the base is a claim
// about a step that adds only probe code, not a standing property of this file.
//
// ⚠️ WHY A `-D` VARIANT AND NOT A RUNTIME `occ_flags` BIT. A runtime-gated sink would have to
// DECLARE @12 in the shipping module. The engine's set layout has twelve bindings
// (`VB_CULL_LAYOUT_BINDINGS`), and a module declaring a binding its bound layout does not provide is
// invalid usage — `create_bind_group`'s own debug-assert, then a validation error, on EVERY engine
// frame. It would also move the census pins and add stores to `PERFORMED` that the framegraph never
// `DECLARED`, the class this campaign has already paid for.
//
// ⚠️ WHAT THE VARIANT CANNOT CLAIM. It is a DIFFERENT artifact, and since the verdict went
// division-free it is different in one more way: `depth_near` is a quantity the SHIPPING module does
// not compute at all, so the recorded value is a proxy for nothing in that module — it is a
// measurement of the FOLD the two builds share, and only the fold's text, its `precise` qualifiers
// and its operation order being identical between them makes it that. The gate does not assume even
// that: it dispatches BOTH modules over every probe and asserts their PARTITIONS agree, so a proxy
// that drifted is caught rather than believed.
//
// Compiled offline (hermetic build) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 vb_batch_cull.comp.hlsl \
//       -Fo vb_batch_cull.comp.spv
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 -D VB_CULL_DEBUG_PROBE=1 \
//       vb_batch_cull.comp.hlsl -Fo vb_batch_cull_debug.comp.spv

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
// plus an 8-byte pad @56, and left every HLSL mirror spelling `uint3 _pad` because the layout
// is identical either way. **Step P3-4 renames it HERE and only here** — this is the one module
// that READS the word, out of the `gVbInstances[base_instance + j]` fetch it already issues, so
// the flag costs zero extra device fetches. `vb_raster.vs.hlsl` and `vb_geom_fetch.hlsli` keep
// `uint3 _pad`: renaming there would re-DXC four `.spv` and move four census pins to buy a name
// in files that do not read the word.
struct VbInstanceRow {
    float4 r0;
    float4 r1;
    float4 r2;
    uint   mesh_id;  // 48
    uint   flags;    // 52 — bit 0 = VB_INST_FLAG_OCCLUSION_CULLING (was `_pad.x`)
    uint2  _pad;     // 56..64
};

// Bit 0 of `VbInstanceRow::flags`: "this instance's entity carries `OcclusionCulling`". Mirrors
// `boyko_render::occlusion_marker::VB_INST_FLAG_OCCLUSION_CULLING`, whose own const-asserts pin it
// to exactly one bit and to bit 0. Bits 1..31 are reserved and written zero by the host gather.
//
// An instance WITHOUT the bit is never occlusion-tested and therefore never deferred: the capability
// is structural (component presence), so its absence is a skip rather than a runtime `false`.
static const uint VB_INST_FLAG_OCCLUSION_CULLING = 1u;

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

// ==== VG R3 piece 3 step P3-4: the five bindings P3-2 bound and this step first LOADS. ====
//
// ⚠️ THE `t`/`u` SPACES SHARE ONE BINDING NUMBER SPACE HERE. This module spells its bindings as
// `: register(uN/tN)` rather than `[[vk::binding(N, 0)]]`, and DXC maps the register INDEX straight
// to the Vulkan binding — so `t7` and `u7` would be the SAME binding, aliasing silently with no
// validation message and no pixel change until something loaded the wrong buffer. The twelve indices
// are kept mutually exclusive BY HAND; the host half of the table is
// `boyko_app::gpu_scene::VB_CULL_LAYOUT_ENTRIES`, whose `vb_cull_layout_table_is_well_formed`
// const-assert makes a kind/index mismatch a BUILD error.

// binding 7: the per-instance EARLY-REJECT list, and after the late phase compacts it in place, the
// LATE SURVIVOR list (RW). Region-addressed by exactly the `VbBatchDesc` fields that address @6 —
// INVARIANT VG-P3-LATE-REGION below — and holding GLOBAL instance indices, the same encoding @6
// carries, so `vb_raster.vs.hlsl` reads it through the identical expression when the late scope
// binds it at the VS's own @11 (`vb_set0_late`).
RWStructuredBuffer<uint> VbLateVisible : register(u7);

// The cull's NON-push inputs — 96 bytes, per-FIF, written by `vkCmdUpdateBuffer` UNCONDITIONALLY
// (armed or not) inside the `vb_batch_cull` pass, and read by BOTH phases. Mirrors the host
// `present::scene_types::VbCullUniform`, whose own size const-assert pins the 96.
//
// It is a BUFFER rather than more push words because the shared COMPUTE push range is const-asserted
// at most 128 bytes and `VB_BATCH_CULL_PUSH_BYTES` is already 112: a `float4x4` does not fit.
//
// The matrix is FOUR `float4` members rather than one `float4x4` on purpose — a `float4x4` in a
// structured buffer carries a majorness decoration that a host-side byte layout cannot see, and the
// one thing this block must not get wrong is which of `pv[row][col]` / `pv[col][row]` it holds. It
// holds MATH ROWS, `clip = pv · world`, exactly what `boyko_render::hzb::project_aabb` takes; the
// host performs the single inversion out of the column-major push storage.
struct VbCullUniformGpu {
    float4 vp_row0;     //  0
    float4 vp_row1;     // 16
    float4 vp_row2;     // 32
    float4 vp_row3;     // 48
    uint2  src_extent;  // 64 — the pyramid's SOURCE extent (`present_extent`, NOT the client one)
    uint2  base_extent; // 72 — level 0's extent, `prev_pow2` per axis. PUSHED, never re-derived
    uint   levels;      // 80 — `HzbPlan::levels`; `level >= levels` is KEEP, never a clamp down
    uint   frame_index; // 84 — the engine frame this block describes (the record-order control)
    uint2  _pad;        // 88..96
};

// binding 8: that block, one element (read-only).
StructuredBuffer<VbCullUniformGpu> VbCullUni : register(t8);

// binding 9: the DEPTH PYRAMID, SAMPLED at `GENERAL` through a mip-complete view.
//
// `.Load(int3(x, y, level))` — POINT, integer coordinates, explicit mip, and there is no
// `SamplerState` in this module to get wrong. See the header's point-sampling section for why a
// filtered read of a min-reduced pyramid is unsound rather than merely imprecise.
//
// On a boot with no pyramid this binds `hzb_null`, a 1×1 single-mip image cleared to `0.0`. The
// safety argument for that is IN-RANGE BY ADDRESS and CONSERVATIVE BY VALUE, and it is deliberately
// NOT a reachability argument: DXC is free to lower a not-taken `? :` to an eager load plus an
// `OpSelect`, so "the load never issues" is not a property source code can claim. `hzb_pyramid_load`
// below masks every coordinate AND the level to 0 whenever `VB_CULL_OCC_ARMED` is clear, so the
// disarmed address is literally `(0, 0, 0)`; and `0.0` is the reverse-Z far plane, so even a value
// that did reach a verdict provably rejects nothing.
Texture2D<float> gHzbPyramid : register(t9);

// binding 10: the LATE draw records (write, phase 1 ONLY).
//
// The store sits under `pc.phase == VB_CULL_PHASE_LATE`, and `pc.phase` is a push constant — uniform
// across the dispatch. That is what makes the framegraph's ASYMMETRIC declarations sound rather than
// lucky: a compiler may hoist a not-taken LOAD, but it may not introduce a STORE the source does not
// perform, so `vb_batch_cull` declares no access on this buffer at all and `vb_cull_late` declares
// the write. Word 1 of each 20-byte record is the ONLY word any shader writes here, exactly as for
// `VbIndirect` @0.
RWByteAddressBuffer VbIndirectLate : register(u10);

// binding 11: per-batch `n_defer` — the early phase's deferral count — plus ONE reserved TAIL slot
// carrying the frame index the GPU actually observed in `VbCullUni` (RW).
//
// ⚠️ The count lives HERE and NOT in `VbIndirectLate[i].instanceCount`, and that is load-bearing in
// three ways at once: the late cull stays the record word's ONLY producer, a frame whose late cull
// did not run draws NOTHING rather than `n_defer` untested instances, and deleting the late cull
// becomes observable in an image instead of being a pixel-invisible redraw at identical depth.
RWStructuredBuffer<uint> VbLateCount : register(u11);

#ifdef VB_CULL_DEBUG_PROBE
// ==== `-D VB_CULL_DEBUG_PROBE=1` ONLY: binding 12, the occlusion leaf's diagnostic sink. ====
//
// Present in NO other build of this file. See the header's section on the variant for why it is a
// separate artifact rather than a runtime-gated store in the shipping one.
//
// @12 is the first free index in the SHARED `t`/`u` number space this module spells its bindings in
// (@0..@11 are taken; the header's binding-space warning is why the check is by index and not by
// space). `hzb_verdict_oracle_gate.rs` builds its OWN thirteen-binding layout for this variant; the
// ENGINE never creates a set for it, so the base layout's twelve stand.
RWStructuredBuffer<uint> VbCullDebug : register(u12);

// One record per INSTANCE SLOT, 8 words:
//
//   0  the STAGE at which `occlusion_reject` exited (the `VB_DBG_STAGE_*` values below)
//   1  `asuint(depth_near)` — the max over the corners folded SO FAR, so it is meaningful at every
//      stage, not only at the verdict. ⚠️ REPORTED, NOT DECIDING: the verdict is division-free since
//      the ULP measurement (step 6), and this word exists in NO other build of this file
//   2  `asuint(occ)`        — the conservative min over the taps
//   3  the selected level
//   4  tap x0, 5 tap x1, 6 tap y0, 7 tap y1 — the SHIFTED texel coordinates the four taps use
//
// Words that the exiting stage never computed hold `VB_DBG_UNSET`, which is read as a BIT PATTERN by
// the gate: `0xFFFFFFFF` is a quiet NaN, so a reader that forgot to check the stage sees a NaN
// rather than a plausible depth.
static const uint VB_DBG_RECORD_WORDS = 8u;
static const uint VB_DBG_UNSET = 0xFFFFFFFFu;

// The exits, in the leaf's own source order. 1..6 mirror `boyko_render::hzb::KeepReason` one for one
// (2 and 4 are its single `NonFinite`, split by WHICH of the two finiteness guards fired, which the
// oracle cannot distinguish and which is exactly the sort of thing this sink exists to show); 7 means
// the strict comparison was REACHED, whichever way it went.
static const uint VB_DBG_STAGE_UNORDERED_BOX    = 1u;
static const uint VB_DBG_STAGE_CLIP_NON_FINITE  = 2u;
static const uint VB_DBG_STAGE_BEHIND_EYE       = 3u;
static const uint VB_DBG_STAGE_NDC_NON_FINITE   = 4u;
static const uint VB_DBG_STAGE_EMPTY_RECT       = 5u;
static const uint VB_DBG_STAGE_LEVEL_UNAVAIL    = 6u;
static const uint VB_DBG_STAGE_VERDICT          = 7u;

// The record slot the NEXT `occlusion_reject` call writes — the GLOBAL instance index, set by each
// call site immediately before the call.
//
// A `static` global (per-invocation Private storage), rather than a parameter, so that
// `occlusion_reject`'s SIGNATURE is the same text in both builds: the two artifacts must differ in
// what they store, never in what they are asked to compute.
static uint g_vb_dbg_slot = 0u;

// An exit BEFORE the verdict: the stage and whatever `depth_near` had accumulated.
void vb_dbg_bailout(uint stage, float depth_near) {
    const uint o = g_vb_dbg_slot * VB_DBG_RECORD_WORDS;
    VbCullDebug[o + 0u] = stage;
    VbCullDebug[o + 1u] = asuint(depth_near);
    VbCullDebug[o + 2u] = VB_DBG_UNSET;
    VbCullDebug[o + 3u] = VB_DBG_UNSET;
    VbCullDebug[o + 4u] = VB_DBG_UNSET;
    VbCullDebug[o + 5u] = VB_DBG_UNSET;
    VbCullDebug[o + 6u] = VB_DBG_UNSET;
    VbCullDebug[o + 7u] = VB_DBG_UNSET;
}

// The verdict exit: every field is defined, and `occ` is stored BESIDE `depth_near` because the two
// were the operands of the one comparison that can delete geometry — and because their DISTANCE is
// what the diagnostic step measured. `occ` still decides (against each corner's own `cz`/`cw`);
// `depth_near` no longer does.
void vb_dbg_verdict(float depth_near, float occ, uint level, uint2 tx, uint2 ty) {
    const uint o = g_vb_dbg_slot * VB_DBG_RECORD_WORDS;
    VbCullDebug[o + 0u] = VB_DBG_STAGE_VERDICT;
    VbCullDebug[o + 1u] = asuint(depth_near);
    VbCullDebug[o + 2u] = asuint(occ);
    VbCullDebug[o + 3u] = level;
    VbCullDebug[o + 4u] = tx.x;
    VbCullDebug[o + 5u] = tx.y;
    VbCullDebug[o + 6u] = ty.x;
    VbCullDebug[o + 7u] = ty.y;
}
#endif // VB_CULL_DEBUG_PROBE

// The number of camera-frustum planes. Fixed order: left, right, bottom, top, near, far — the
// SAME order `boyko_render::frustum::frustum_planes_from_view_proj` emits.
static const uint FRUSTUM_PLANE_COUNT = 6u;

// `+INFINITY` and `-INFINITY` by BIT PATTERN, the `hzb_build.comp.hlsl` idiom: `1.#INF` parsing is
// implementation-dependent, `asfloat` of the exact word is not.
//
// `+INFINITY` seeds `occ` (the `min` identity) and the window-rect minima; `-INFINITY` seeds the
// window-rect maxima and `depth_near`, and is `conservative_min`'s answer on a NaN — "unknown depth
// is infinitely far", the only reading under which the verdict can never be `Reject`.
static const float HZB_POS_INF = asfloat(0x7F800000u);
static const float HZB_NEG_INF = asfloat(0xFF800000u);

// VG R3 piece 3 step P3-3 (docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md, decision D5): the two values
// `pc.phase` may take. ONE module, ONE pipeline, ONE entry point, a UNIFORM branch — not a `-D`
// variant pair, for verbatim the reason `hzb_build.comp.hlsl` forks on `pc.base_level == 0`: the
// occlusion leaf must be the SAME function in both phases, and two artifacts are two
// implementations that can drift. Drift in the direction where the LATE test is stricter than the
// EARLY one deletes geometry from the frame.
static const uint VB_CULL_PHASE_EARLY = 0u;
static const uint VB_CULL_PHASE_LATE  = 1u;

// VG R3 piece 3 (plan D6): the bits of `pc.occ_flags`. Mirrors
// `present::scene_types::{VB_CULL_OCC_ARMED, VB_CULL_OCC_FORCE_LATE, VB_CULL_OCC_FORCE_KEEP}`, which
// the host folds into ONE word at ONE site so the declarator, the recorder and this module read the
// same number.
//
//   * ARMED      — the pyramid exists and the split is armed. CLEAR ⇒ `defer` is identically false
//                  ⇒ this module is the pre-P3-4 cull, statement for statement.
//   * FORCE_LATE — the CONTROL: defer every MARKED instance whatever the verdict says. It exists
//                  because on a converged static scene the CORRECT late-survivor count is zero, so
//                  an unforced fixture cannot carry the "the late scope actually rasterises" claim.
//   * FORCE_KEEP — the OFF SWITCH: defer nothing, while the whole partition machinery still runs.
//                  The zero control for a timing triple.
//
// FORCE_LATE and FORCE_KEEP are opposite controls; the host debug-asserts they are never both set,
// because the resolution would otherwise be "whichever branch the shader tests first". This module
// tests FORCE_KEEP in the guard and FORCE_LATE in the value, so with both set FORCE_KEEP would win —
// stated so the host assert is understood as pinning a choice rather than preventing a crash.
static const uint VB_CULL_OCC_ARMED      = 1u << 0;
static const uint VB_CULL_OCC_FORCE_LATE = 1u << 1;
static const uint VB_CULL_OCC_FORCE_KEEP = 1u << 2;

// The batch-cull push constants. Mirrors the host `VbBatchCullPush` (112 bytes since P3-3).
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
    // VG R3 piece 3 step P3-3: `VB_CULL_PHASE_EARLY` or `VB_CULL_PHASE_LATE`. A PUSH constant, so
    // the fork below is uniform across the dispatch — which is what makes the two passes' framegraph
    // declarations sound: a compiler may lower a not-taken `? :` to an eager LOAD plus an
    // `OpSelect`, but it may not introduce a STORE the source does not perform.
    uint phase;
    // VG R3 piece 3 step P3-3 (plan D6): the occlusion decision's arming word — bit 0 ARMED, bit 1
    // FORCE_LATE, bit 2 FORCE_KEEP; the host folds it ONCE (`GBufferScene::vb_occ_flags`). Read by
    // this module since step P3-4, in exactly three places: the early phase's `defer` guard, the
    // early phase's two list stores, and `hzb_pyramid_load`'s disarmed address mask.
    //
    // ⚠️ THE HOST STILL PUSHES 0 AT P3-4, and that is this step's whole inertness claim. `defer` is
    // then identically false, so the early loop is the pre-P3-4 loop statement for statement, no
    // list store issues, and every pyramid load address is masked to `(0, 0, 0)`. The arming commit
    // is P3-6.
    uint occ_flags;
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

// VG R3 piece 3 step P3-4: ONE instance's world AABB, as a centre + half-extent pair.
//
// The SAME six `dot()` calls rung R2d-6 shipped inline in the early loop, hoisted VERBATIM into a
// function so BOTH phases evaluate one implementation. That is decision D5's own argument applied
// one level below the occlusion leaf: two copies of the fold are two implementations that can
// drift, and a late fold tighter than the early one deletes geometry from the frame.
//
// The Arvo abs-matrix fold — `wc[r] = dot(row[r].xyz, lc) + row[r].w`,
// `wh[r] = dot(abs(row[r].xyz), lh)` — is the same fold `boyko_render::csm_caster::arvo_transform`
// performs on the host. ⚠️ These six `dot()`s are DELIBERATELY left unqualified: plan D11 adds
// `precise` to the PROJECTION leaf only, so that the module's `NoContraction` count is a
// measurement of that one change rather than of this one too. The frustum verdict they feed is
// unchanged from rung R2d-6 and no golden may move because of them.
//
// The caller must have already cleared the UNKNOWN-BOUNDS sentinel: this function believes `b`.
void arvo_world_box(VbInstanceRow row, MeshLocalBounds b, out float3 wc, out float3 wh) {
    const float3 lc = (b.bmin + b.bmax) * 0.5;
    const float3 lh = (b.bmax - b.bmin) * 0.5;
    wc = float3(dot(row.r0.xyz, lc) + row.r0.w,
                dot(row.r1.xyz, lc) + row.r1.w,
                dot(row.r2.xyz, lc) + row.r2.w);
    wh = float3(dot(abs(row.r0.xyz), lh),
                dot(abs(row.r1.xyz), lh),
                dot(abs(row.r2.xyz), lh));
}

// ==== VG R3 piece 3 step P3-4 (plan D11 / A1): THE OCCLUSION LEAF. ====
//
// A statement-for-statement mirror of `boyko_render::hzb`'s
// `project_aabb → select_texels → occluder_depth → occlusion_verdict` chain, in that chain's own
// SHORT-CIRCUIT ORDER. `crates/boyko_app/tests/hzb_verdict_oracle_gate.rs` is the differential that
// decides the mirror against the oracle with no engine involved — the P1-7 discipline: prove the
// shader against the oracle BEFORE any engine frame depends on it, so a real disagreement is
// characterised instead of chased through a renderer.
//
// # THE CONSERVATIVE DIRECTION, again and for a different predicate
//
// Six early-out returns, and EVERY ONE of them is `false` = KEEP. A false keep costs one wasted
// draw; a false reject deletes geometry the camera can see. The hot path — a fully visible on-screen
// box — takes none of them.
//
// # ⚠️ THE DECISION IS DIVISION-FREE, AND THAT IS A SPEC-LEVEL NECESSITY (not an optimisation)
//
// The verdict used to be `max_i (cz_i / cw_i) < occ`, folded into a single `depth_near`. The
// diagnostic variant MEASURED that quotient against `boyko_render::hzb`'s over 72 boundary probes:
// the rect, the level and all four taps agreed EXACTLY, `depth_near` differed on 6 probes by at most
// 1 ULP in BOTH directions, and 2 probes partitioned differently because of it. Vulkan's precision
// appendix specifies `OpFAdd`/`OpFSub`/`OpFMul` as correctly rounded but allows `OpFDiv` 2.5 ULP at
// 32-bit, while Rust's `/` is the IEEE 0.5-ULP one — and `precise` emits `NoContraction`, which
// constrains CONTRACTION and REASSOCIATION and says NOTHING about a division's ULP allowance. So
// bit-agreement through the quotient is closed BY THE SPECIFICATION, at any price.
//
// Every corner reaching the verdict has `cw_i > 0` (the `cw <= 0.0` early-out is what buys it), so
//
//     max_i (cz_i / cw_i) < occ    ⟺    for all i:  cz_i  <  occ · cw_i
//
// — multiplying a strict inequality through by a POSITIVE denominator, which is an equivalence over
// the reals and not an approximation of one. The right-hand form spends ONE correctly-rounded
// multiply per corner, so the host and this module evaluate the SAME function of the same bits and
// agree BY CONSTRUCTION rather than within a tolerance. It is also cheaper than the divide.
//
// ⚠️ IT IS A UNIVERSAL TEST OVER ALL EIGHT CORNERS, NEVER A TEST ON THE ARGMAX. Reducing to "the
// nearest corner" first would need a comparison of two quotients, i.e. exactly the rounding question
// this reformulation exists to remove: two corners within a rounding of each other could be ordered
// differently on the two sides, and the corner not chosen is then never tested. `for all i` has no
// selection step to get wrong.
//
// The divide SURVIVES for the window rect (`x_win`/`y_win`) and for the `z_ndc` finiteness guard,
// and is deliberately left there: the rect is what the measurement found already agreeing exactly on
// every probe, and both directions of a finiteness guard are KEEP.

// `boyko_render::hzb::conservative_min`, EXACTLY: `-INFINITY` on a NaN operand, else the smaller.
//
// ⚠️ NOT `min(a, b)`. `min` lowers to `NMin`, under which a NaN operand is silently DISCARDED — the
// other operand is taken — instead of propagating; this repository has an incident on exactly that
// (`clamp(NaN, 0, 1)` came out `0`, a black pixel). Here the swallowed NaN would become an occluder
// depth taken from whichever texel happened to be finite, and the pyramid's "conservative lower
// bound over the footprint" claim would be false. `hzb_build.comp.hlsl` refuses the intrinsic in the
// same words for the same reason.
//
// The ±0 TIE this shape has been MEASURED to produce is provably outside the verdict: P1 §10 found a
// driver fusing `b < a ? b : a` into a hardware `min` whose tie-break returns `-0.0` regardless of
// operand order. The verdict's `occ · cw_i` carries the zero's sign into the product and nowhere
// else, and IEEE compares `-0.0` and `+0.0` as EQUAL — so `cz_i < -0.0` and `cz_i < +0.0` answer
// identically for every `cz_i`, and the one known GPU/oracle disagreement in this chain cannot reach
// this predicate. (Under the earlier quotient form the same conclusion held through
// `depth_near < occ`; the division-free form does not weaken it.)
float hzb_conservative_min(float a, float b) {
    if (isnan(a) || isnan(b)) {
        return HZB_NEG_INF;
    }
    return b < a ? b : a;
}

// `boyko_render::hzb::msb`, whose contract is `msb(0) := 0`.
//
// ⚠️ THE `firstbithigh(0)` TRAP, named because it is this campaign's class of defect. HLSL's
// `firstbithigh(0)` is `0xFFFFFFFF`, and the selector `max(msb(tx0 ^ tx1), msb(ty0 ^ ty1))` is an
// UNSIGNED max — so a single un-guarded axis wins outright. `tx0 ^ tx1 == 0` means both rect corners
// land in the SAME texel on that axis, which is the COMMON case, and level `0` is exactly the right
// answer for it. Dropping the guard on one axis alone therefore forces `level >= levels` (KEEP) on
// most instances, i.e. a cull that rejects nothing — the failure no golden can see.
// `hzb_verdict_oracle_gate.rs`'s control D3 fires it on a `1 × 1` layout, where single-texel rects
// are unconditional.
//
// ⚠️ A SECOND hazard, named because no artifact gate can see it either: `firstbithigh` is one of the
// few HLSL intrinsics whose DXIL and SPIR-V lowerings have historically disagreed about which end
// the returned position counts from. The oracle's `msb` is `31 - leading_zeros`, i.e. the MSB's
// index from the LSB, which is what SPIR-V's `FindUMsb` produces. A lowering that returned
// `31 - msb` instead would make `level` enormous for every multi-texel rect, `level >= levels` would
// fire, and the cull would reject NOTHING — the CONSERVATIVE direction, so it cannot delete
// geometry, but it would silently disarm the whole feature behind a byte-identical golden. The
// differential is what decides it: every `Reject` case in every corpus goes red.
uint hzb_msb(uint v) {
    return v == 0u ? 0u : (uint)firstbithigh(v);
}

// `boyko_render::hzb::HzbAxis::texel_of`: the level-0 texel containing SOURCE pixel `x`.
//
// The oracle computes `(x · base) / source` in u64; this computes it in u32, and the two agree
// because the true product FITS: `x < source <= MAX_HZB_EXTENT = 65536` and `base <= source`, so
// `x · base <= 65535 · 65536 = 4 294 901 760 < 2^32`. No `Int64` capability is requested, and the
// bound is a property of the layout rather than of any value this module is handed.
uint hzb_texel_of(uint x, uint base, uint source) {
    return (x * base) / source;
}

// ONE pyramid tap, and the ONLY place this module addresses `gHzbPyramid`.
//
// ⚠️ THE MASK IS THE DISARMED-PATH SAFETY ARGUMENT, AND IT IS STRUCTURAL. With `VB_CULL_OCC_ARMED`
// clear, @9 binds `hzb_null` — 1 × 1, single mip — and every coordinate AND the level are masked to
// 0, so the address is literally `(0, 0, 0)`: in range for that image whether or not any branch above
// is taken, and whether or not DXC hoists the load. The mask is derived from the PUSH WORD and
// deliberately NOT from `uni.levels`: a structural bound must not depend on a value another decision
// could later gate off. `descriptorBindingPartiallyBound` is not relied upon and
// `robustBufferAccess` is OFF, so an out-of-range image read would be undefined VALUES rather than a
// fault — agreement-breaking even when it is memory-safe.
//
// On an ARMED frame the mask is all-ones and the address is the real one, in range by two lemmas:
// `texel_of` above bounds `tx < base`, and `tx >> level < level_extent(level)` because `base` is a
// power of two — for `level < log2(base)`, `(base − 1) >> level = (base >> level) − 1`; for
// `level >= log2(base)` the level extent is the clamped `1` and `tx >> level` is 0.
float hzb_pyramid_load(uint tx, uint ty, uint level) {
    const uint m = (pc.occ_flags & VB_CULL_OCC_ARMED) != 0u ? 0xFFFFFFFFu : 0u;
    return gHzbPyramid.Load(int3(int(tx & m), int(ty & m), int(level & m)));
}

// THE VERDICT: `true` iff this world AABB is provably behind the depth already in the pyramid over
// its WHOLE screen rect. `false` is KEEP, and every guard answers `false`.
//
// Called ONLY from inside the outer `!any(b.bmin > b.bmax)` guard, so an unknown-bounds instance
// never reaches it. Its own step-1 world-space guard is a DIFFERENT test — see there.
bool occlusion_reject(VbCullUniformGpu uni, float3 mn, float3 mx) {
    // ---- STEP 1 — the ORACLE's own world-space guard (`hzb.rs`, `project_aabb`'s first statement).
    //
    // ⚠️ NOT the unknown-bounds sentinel guard (that is the caller's, on the LOCAL box, and its
    // position is load-bearing — see this file's own section on it), and ⚠️ NOT `any(mn > mx)`.
    // Spelled `!(mn <= mx)` so that a NaN coordinate — which is neither `<=` nor `>` — lands HERE, at
    // the earliest possible point, instead of travelling into the projection to be caught by a later
    // guard that may not exist. A degenerate `view_proj` would fold an inverted box into a perfectly
    // plausible small rect.
    if (!all(mn <= mx)) {
#ifdef VB_CULL_DEBUG_PROBE
        // `depth_near` does not exist yet at this exit; the record carries its SEED, which is what
        // the fold would have started from.
        vb_dbg_bailout(VB_DBG_STAGE_UNORDERED_BOX, HZB_NEG_INF);
#endif
        return false;
    }

    const float half_w = (float)uni.src_extent.x * 0.5;
    const float half_h = (float)uni.src_extent.y * 0.5;

    float x_lo = HZB_POS_INF;
    float x_hi = HZB_NEG_INF;
    float y_lo = HZB_POS_INF;
    float y_hi = HZB_NEG_INF;

    // THE VERDICT'S OPERANDS, one pair per corner, kept UNDIVIDED. `occ` is not known until step 5
    // (it depends on the rect these same corners produce), and the division-free predicate is a
    // statement about EVERY corner rather than about a pre-reduced maximum — see the leaf's header —
    // so the eight pairs are carried instead of folded. They are the fold's own `precise` values,
    // stored verbatim: a copy cannot re-round what `NoContraction` already fixed.
    //
    // ⚠️ SEEDED `(0, 0)`, and that is a STRUCTURAL safety argument rather than a reachability one —
    // the same distinction `hzb_pyramid_load`'s disarmed mask is written for. A `(0, 0)` slot makes
    // step 6's test `0 < occ · 0`, i.e. `0 < 0` for a finite `occ` and `0 < NaN` for an infinite
    // one: FALSE either way, so an unwritten slot forces KEEP. The loop does in fact fill all eight
    // before any read — every guard inside it RETURNS — but "unreachable" is not what the
    // conservative direction should rest on, and `boyko_render::hzb::project_aabb` seeds its mirror
    // of this array identically.
    float corner_cz[8] = { 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0 };
    float corner_cw[8] = { 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0 };

#ifdef VB_CULL_DEBUG_PROBE
    // ⚠️ DIAGNOSTIC ONLY, AND ONLY IN THIS BUILD. `depth_near` is the quotient the verdict USED to
    // fold; it is kept because the gate's census measures its distance from the oracle's, and it is
    // `#ifdef`-ed because the shipping module must not merely *not use* it — it must not COMPUTE it,
    // so that "the divide does not decide" is a property of the token stream rather than an
    // inference about dead-code elimination.
    float depth_near = HZB_NEG_INF;
#endif

    // ---- STEP 2 — the eight corners, in the oracle's index order (`bit0→x, bit1→y, bit2→z`, `0`
    // picks min) and with the oracle's guard order: finite-clip, then `w <= 0`, then the divide, then
    // finite-post-divide. **The first offending corner RETURNS** — a short-circuit, not a fold.
    //
    // The `cw <= 0.0` rejection is where this engine is ahead of the field and it must not be lost:
    // without it a corner behind the eye flips sign under the perspective divide, the min/max rect
    // inverts or collapses, and a fine mip is selected over the wrong place — a silent OVER-cull.
    //
    // NOT `[unroll]`ed: an attribute's effect is a measurement, not an inference (the P1-3 lesson),
    // and the `.spv` census records the loop's shape so a future change is visible.
    for (uint corner = 0u; corner < 8u; ++corner) {
        const float3 p = float3((corner & 1u) == 0u ? mn.x : mx.x,
                                (corner & 2u) == 0u ? mn.y : mx.y,
                                (corner & 4u) == 0u ? mn.z : mx.z);

        // ⚠️ `dot()` IS FORBIDDEN HERE, and this repository has already rejected it in writing for
        // exactly this reason (`cluster_cull.hlsl`'s `sq_dist_point_aabb`): Vulkan specifies OpFAdd /
        // OpFSub / OpFMul as "Correctly rounded", but specifies OpDot only as "inherited from a
        // formula", and permits that formula to be transformed using associativity, commutativity
        // and distributivity. `cz` and `cw` are the verdict's own operands — the predicate is
        // `cz_i < occ · cw_i` — and a `cz` one ULP LOW is the geometry-deleting direction. This is
        // the sum whose exactness the whole division-free reformulation rests on: it is what lets
        // "host and shader agree by construction" be a claim about ALL of the arithmetic that
        // decides, rather than about the last multiply alone.
        //
        // So the sum is WRITTEN OUT, mirroring the oracle's explicit left fold
        // (`r[0]*p[0] + r[1]*p[1] + r[2]*p[2] + r[3]`) term for term, with `precise` on every node —
        // which emits `NoContraction` and therefore forbids FMA contraction as well.
        //
        // The two sentinel comments below are READ BY A TEST: `vb_batch_cull_spv_sync.rs` extracts
        // the text between them and asserts it contains no `dot(`. Moving or deleting a sentinel is
        // a hard failure there, not a silent empty match.
        // === PROJECTION FOLD BEGIN ===
        precise float cx = uni.vp_row0.x * p.x + uni.vp_row0.y * p.y + uni.vp_row0.z * p.z + uni.vp_row0.w;
        precise float cy = uni.vp_row1.x * p.x + uni.vp_row1.y * p.y + uni.vp_row1.z * p.z + uni.vp_row1.w;
        precise float cz = uni.vp_row2.x * p.x + uni.vp_row2.y * p.y + uni.vp_row2.z * p.z + uni.vp_row2.w;
        precise float cw = uni.vp_row3.x * p.x + uni.vp_row3.y * p.y + uni.vp_row3.z * p.z + uni.vp_row3.w;
        // === PROJECTION FOLD END ===

        if (!(isfinite(cx) && isfinite(cy) && isfinite(cz) && isfinite(cw))) {
#ifdef VB_CULL_DEBUG_PROBE
            vb_dbg_bailout(VB_DBG_STAGE_CLIP_NON_FINITE, depth_near);
#endif
            return false;
        }
        if (cw <= 0.0) {
#ifdef VB_CULL_DEBUG_PROBE
            vb_dbg_bailout(VB_DBG_STAGE_BEHIND_EYE, depth_near);
#endif
            return false;
        }

        // `precise` on the post-divide locals too — but ⚠️ NOT because it constrains the DIVIDE.
        // `precise` emits `NoContraction`, which forbids FMA contraction and reassociation of the
        // expressions feeding these locals; it says NOTHING about `OpFDiv`'s accuracy, which
        // Vulkan's precision appendix permits to be 2.5 ULP at 32-bit (the same appendix that
        // specifies `OpFAdd`/`OpFSub`/`OpFMul` as correctly rounded). An earlier draft of this
        // comment claimed `precise` forbids a reciprocal estimate here; it does not, and a 2.5-ULP
        // divide is a CONFORMING implementation of this line. That is exactly why the VERDICT no
        // longer flows through it: what remains downstream of the quotient is the window rect —
        // MEASURED identical to the oracle's on all 72 boundary probes — and two finiteness guards,
        // both of whose outcomes are KEEP.
        precise float inv_w = 1.0 / cw;
        precise float z_ndc = cz * inv_w;
        // POSITIVE viewport height, no flip: `+Y` NDC is `+Y` window, verbatim the oracle.
        precise float x_win = (cx * inv_w + 1.0) * half_w;
        precise float y_win = (cy * inv_w + 1.0) * half_h;
        // Repeated AFTER the divide: a finite `clip` over a tiny `w` still overflows to infinity.
        if (!(isfinite(x_win) && isfinite(y_win) && isfinite(z_ndc))) {
#ifdef VB_CULL_DEBUG_PROBE
            vb_dbg_bailout(VB_DBG_STAGE_NDC_NON_FINITE, depth_near);
#endif
            return false;
        }

        // ⚠️ `min`/`max` here lower to `NMin`/`NMax`, under which a NaN operand is silently
        // discarded. They are reached ONLY after both finiteness checks have returned, so no NaN can
        // arrive — the claim is UNREACHABLE, not HANDLED, and it is written down because those are
        // different claims. `hzb_verdict_oracle_gate.rs`'s random corpus asserts that the NonFinite
        // class is actually observed, so the guards above are not vacuous.
        x_lo = min(x_lo, x_win);
        x_hi = max(x_hi, x_win);
        y_lo = min(y_lo, y_win);
        y_hi = max(y_hi, y_win);
        // The verdict's operands, UNDIVIDED and unreduced. `corner` is the loop's own induction
        // variable over a constant trip count, so every one of the eight slots is written exactly
        // once before step 6 reads it — on the only path that reaches step 6 at all, since each
        // guard above RETURNS.
        corner_cz[corner] = cz;
        corner_cw[corner] = cw;
#ifdef VB_CULL_DEBUG_PROBE
        depth_near = max(depth_near, z_ndc);
#endif
    }

    // ---- STEP 3 — the pixel rect: `floor` on BOTH ends, clamped to `[0, source − 1]`.
    //
    // ⚠️ `floor(hi)`, never `ceil(hi) - 1`. A pixel `i` covers `[i, i+1)`, so a span ending exactly
    // on a boundary still TOUCHES pixel `floor(hi)`; `ceil(6.0) - 1 = 5` drops a column the bound
    // covers, and a footprint missing the column where the instance is visible is a FALSE REJECT.
    // The two forms differ only on an exactly-integer edge, which is why the differential carries an
    // extent built to land on one.
    const float x_last = (float)(uni.src_extent.x - 1u);
    const float y_last = (float)(uni.src_extent.y - 1u);
    const float px0 = max(floor(x_lo), 0.0);
    const float px1 = min(floor(x_hi), x_last);
    const float py0 = max(floor(y_lo), 0.0);
    const float py1 = min(floor(y_hi), y_last);
    // Covers "entirely off-screen" (the clamp crosses the bounds over) and any inversion.
    if (px1 < px0 || py1 < py0) {
#ifdef VB_CULL_DEBUG_PROBE
        vb_dbg_bailout(VB_DBG_STAGE_EMPTY_RECT, depth_near);
#endif
        return false;
    }

    // ---- STEP 4 — the coarsest level at which the rect spans at most TWO texels per axis.
    //
    // `level >= levels` is KEEP and NEVER a clamp down to `levels - 1`: a finer level samples a
    // strict SUBSET of the rect's footprint, so `occ` could only come out too large and reject a
    // visible instance. On a disarmed boot `uni.levels` is 0, so this is the early-out that fires for
    // every selected level — which is what makes the `hzb_null` tap unreachable in source order, on
    // top of (never instead of) `hzb_pyramid_load`'s structural mask.
    //
    // `msb(tx0 ^ tx1)` is ALIGNMENT-AWARE by construction, not `ceil(log2(extent))` plus a
    // refinement: the un-refined form is where Bevy #14042 went wrong — a ~29 × 30 px bbox selected
    // mip 4, its 2×2 footprint stopped covering the sphere, and visible clusters were rejected at
    // certain distances only. No refinement step is added here and none is needed.
    const uint tx0 = hzb_texel_of((uint)px0, uni.base_extent.x, uni.src_extent.x);
    const uint tx1 = hzb_texel_of((uint)px1, uni.base_extent.x, uni.src_extent.x);
    const uint ty0 = hzb_texel_of((uint)py0, uni.base_extent.y, uni.src_extent.y);
    const uint ty1 = hzb_texel_of((uint)py1, uni.base_extent.y, uni.src_extent.y);
    const uint level = max(hzb_msb(tx0 ^ tx1), hzb_msb(ty0 ^ ty1));
    if (level >= uni.levels) {
#ifdef VB_CULL_DEBUG_PROBE
        vb_dbg_bailout(VB_DBG_STAGE_LEVEL_UNAVAIL, depth_near);
#endif
        return false;
    }
    // `containing_texel(t, level) = t >> level`. The oracle's `level >= 32 ⇒ 0` arm is not spelled
    // because it is unreachable here: `tx`/`ty` are `< base <= MAX_HZB_EXTENT = 65536`, so
    // `msb(tx0 ^ tx1) <= 15` and `level <= 15` whatever `uni.levels` says.
    const uint2 tx = uint2(tx0 >> level, tx1 >> level);
    const uint2 ty = uint2(ty0 >> level, ty1 >> level);

    // ---- STEP 5 — `occ`: the conservative min over the (up to four, duplicates idempotent) selected
    // texels, seeded `+INFINITY`, folded in the oracle's own row-major order.
    float occ = HZB_POS_INF;
    occ = hzb_conservative_min(occ, hzb_pyramid_load(tx.x, ty.x, level));
    occ = hzb_conservative_min(occ, hzb_pyramid_load(tx.y, ty.x, level));
    occ = hzb_conservative_min(occ, hzb_pyramid_load(tx.x, ty.y, level));
    occ = hzb_conservative_min(occ, hzb_pyramid_load(tx.y, ty.y, level));

    // ---- STEP 6 — the verdict. **STRICT `<`. EQUALITY KEEPS. NO DIVISION.**
    //
    // `for all i: cz_i < occ · cw_i`, which for `cw_i > 0` is `max_i (cz_i / cw_i) < occ` — see the
    // leaf's header for why the multiplied form is the one that can agree with the oracle at all.
    //
    // STRICTNESS: the soundness chain `occ <= D[p] <= d_i(p) <= depth_near` admits equality, so a
    // corner meeting `occ · cw_i` EXACTLY is a legitimate visible case and `<=` here would delete
    // it. This is the one comparison in the whole piece that can remove geometry, and it is the one
    // line the differential's constructed boundary corpus exists to decide.
    //
    // Written `!(cz < bound)` and not `cz >= bound`: a NaN is neither, and `!(NaN < x)` is TRUE, so
    // an impossible NaN exits KEEP. Impossible, because both finiteness guards have already returned
    // and `occ` is `hzb_conservative_min`'s NaN-free result — stated as UNREACHABLE, not HANDLED.
    //
    // The FIRST corner that is not strictly behind the occluder returns, exactly as step 2's guards
    // do: a short-circuit, not a fold.
#ifdef VB_CULL_DEBUG_PROBE
    // BEFORE the comparison, so the record is written whichever way the verdict goes and a REJECT is
    // as readable as a KEEP.
    vb_dbg_verdict(depth_near, occ, level, tx, ty);
#endif
    for (uint c = 0u; c < 8u; ++c) {
        // ONE `OpFMul` — "Correctly rounded" in Vulkan's precision appendix, and the same IEEE
        // product `boyko_render::hzb`'s `ScreenRect::behind_occluder` computes from the same bits.
        // `precise` so that no neighbouring arithmetic may contract or reassociate the one operation
        // the decision now rests on.
        precise float bound = occ * corner_cw[c];
        if (!(corner_cz[c] < bound)) {
            return false;
        }
    }
    return true;
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

    // ==== VG R3 piece 3 steps P3-3/P3-4 (plan D5): THE PHASE FORK. ====
    //
    // ⚠️ THE FORK AND THE HOST'S SECOND DISPATCH WERE ONE CHANGE, and separating them was the defect
    // it exists to foreclose. Step P3-3 records a `vb_cull_late` dispatch of this same module with
    // `pc.phase == VB_CULL_PHASE_LATE`. Without a fork, that dispatch would re-run the EARLY body —
    // re-testing every instance against the frustum and REWRITING `VbVisibleInstance` and every
    // record's `instanceCount` AFTER the early raster had already fetched them. On today's static
    // corpus it would write the same numbers and be invisible to every golden, which is exactly the
    // class of silent defect this campaign has shipped six times.
    //
    // Sited AFTER the tail-lane guard because both phases need it: the late phase is dispatched over
    // the SAME `batch_count` lanes (plan D4 — the late dispatch is FIXED and HOST-SIZED; the GPU-only
    // quantity is the per-batch candidate count, not the lane count), so the guard is shared.
    //
    // `pc.phase` is a PUSH constant, so this branch is UNIFORM across the dispatch. That is what
    // makes the two passes' asymmetric framegraph declarations sound rather than lucky: every LOAD
    // either phase can issue is declared on BOTH passes, while `VbIndirectLate`'s store is declared
    // on `vb_cull_late` only and `VbIndirect` / `VbCullVisible` / `VbCullCount` /
    // `VbVisibleInstance`'s stores on `vb_batch_cull` only. A compiler may hoist a not-taken load; it
    // may not introduce a store the source does not perform.

    // Both phases read the SAME uniform block, once per lane, before the fork — one load, and the
    // `vb_cull_uniform` COMPUTE `SHADER_READ` the graph declares on both passes is discharged by
    // this statement whichever way the fork goes.
    const VbCullUniformGpu uni = VbCullUni[0];
    const VbBatchDescGpu d = VbBatchDesc[i];

    // The occlusion decision's three arming bits, unpacked once. `occ_armed` gates BOTH of the early
    // phase's new stores as well as the `defer` computation — see the header's section on the
    // obligation P3-6 owes this module for why PERFORMED must stay a subset of DECLARED.
    const bool occ_armed = (pc.occ_flags & VB_CULL_OCC_ARMED) != 0u;
    const bool occ_force_late = (pc.occ_flags & VB_CULL_OCC_FORCE_LATE) != 0u;
    const bool occ_force_keep = (pc.occ_flags & VB_CULL_OCC_FORCE_KEEP) != 0u;

    // ==== VG R3 piece 3 step P3-4 (plan A3): THE LATE PHASE. ====
    //
    // Re-test this batch's early-DEFERRED candidates against the pyramid THIS frame's early raster
    // fed, compact the survivors in place, and store the count the late `vkCmdDrawIndexedIndirect`
    // fetches. This is the half of the two-phase split that can DELETE geometry, and it is exactly
    // sound for one reason: it rejects instance `i` iff `i` is occluded by the set of instances the
    // EARLY scope drew, at their current positions. Occlusion by a SUBSET of the scene implies
    // occlusion by the scene, so a late reject is genuinely invisible.
    //
    // > LEMMA (in-place compaction is race-free with no scratch). At step `j` the lane reads index
    // > `base + j` and writes index `base + keep` with `keep <= j`. Every later read is at
    // > `base + j'` with `j' > j >= keep`, so no write can clobber a slot a later read will consume.
    // > ONE LANE PER BATCH, so there is no cross-lane question at all — no atomic, no groupshared,
    // > no barrier.
    //
    // ⚠️ COROLLARY THE READBACK DEPENDS ON: after compaction the region holds `kept[0..keep)`
    // followed by the ORIGINAL entries at `[keep, n_defer)` — a multiset that is NOT the candidate
    // set. The candidate list is recoverable only from the PRE-late snapshot, which is why the
    // framegraph declares two readback passes rather than one.
    //
    // `n_defer` is read under `occ_armed` for the containment reason above: on a frame where the
    // early phase performed no `VbLateCount` store (disarmed), the word holds an earlier frame's
    // residue or undefined allocation contents (`robustBufferAccess` is OFF and nothing clears this
    // buffer), and compacting against it would be a read of garbage followed by a draw of it.
    if (pc.phase == VB_CULL_PHASE_LATE) {
        const uint n_defer = occ_armed ? VbLateCount[i] : 0u;
        uint n_keep = 0u;
        for (uint j = 0u; j < n_defer; ++j) {
            // Ascending `j`, and the early phase wrote the candidates in ascending order, so `g` is
            // ascending too and the `gVbInstances[g]` gather is a monotone strided walk rather than
            // a random one.
            const uint g = VbLateVisible[d.base_instance + j];
            const VbInstanceRow row = gVbInstances[g];
            const MeshLocalBounds b = gMeshBounds[row.mesh_id];
            // The SAME outer sentinel guard the early phase spells, and it is UNREACHABLE here by
            // construction: the candidate list only ever holds instances that passed it, and
            // `gMeshBounds` is host-coherent between the two dispatches of one frame. It is spelled
            // anyway, because D5's whole argument is that the two phases run the SAME statements and
            // an "unreachable so omitted" branch is the first place a future edit makes them differ.
            bool survive = true;
            if (!any(b.bmin > b.bmax)) {
                float3 wc;
                float3 wh;
                arvo_world_box(row, b, wc, wh);
                // No frustum test is repeated: a candidate passed it in the early phase by
                // construction. The world AABB is RECOMPUTED rather than stored — 24 B per candidate
                // of extra traffic to save ~20 flops is the wrong trade.
#ifdef VB_CULL_DEBUG_PROBE
                // The record is keyed by GLOBAL instance index in both phases, so a late dispatch
                // OVERWRITES the early record for the same instance — which is the reading wanted:
                // the last dispatch to test an instance is the one whose numbers are on file.
                g_vb_dbg_slot = g;
#endif
                survive = !occlusion_reject(uni, wc - wh, wc + wh);
            }
            if (survive) {
                VbLateVisible[d.base_instance + n_keep] = g;
                ++n_keep;
            }
        }
        // The ONLY producer of a nonzero `instanceCount` in the late record array. The host's
        // `vb_indirect_late_upload` seeds this word to 0 and writes the four words no GPU pass
        // produces; that seed is load-bearing rather than decorative, because it is what makes a
        // frame whose late cull did not run draw NOTHING.
        VbIndirectLate.Store(
            i * DRAW_INDEXED_INDIRECT_STRIDE + INSTANCE_COUNT_OFFSET,
            n_keep);
        return;
    }

    // ==== Everything below is the EARLY phase (`pc.phase == VB_CULL_PHASE_EARLY`). ====
    //
    // The frame index the GPU actually observed in `VbCullUni`, stamped once per dispatch by batch
    // lane 0 into `VbLateCount`'s reserved TAIL slot. It is the ONLY executable control for the
    // record-order hazard the uniform fill carries: with two frames in flight, a fill landing on the
    // wrong side of the barrier makes this dispatch read frame N−2's uniform, which is bit-identical
    // on every static fixture and therefore invisible to every image gate and every oracle
    // differential.
    //
    // ⚠️ THE SLOT INDEX IS DERIVED FROM THE BUFFER, NOT MIRRORED FROM THE HOST. `VbLateCount` is
    // allocated as one `uint` per LATE DRAW RECORD plus exactly one reserved tail slot — a relation
    // `boyko_app::gpu_scene`'s own const-assert pins across the record stride — so the reserved slot
    // IS the last element, and `GetDimensions` reads that off the descriptor's range. Hard-coding the
    // host's `VB_LATE_COUNT_FRAME_SLOT` here would put a private capacity constant of another crate
    // into this file with nothing holding the two spellings together; the module's other two
    // mirrored constants (`LOCAL_SIZE_X`, `DRAW_INDEXED_INDIRECT_STRIDE`) each have such a gate and
    // this one would not.
    if (occ_armed && i == 0u) {
        uint late_count_elems = 0u;
        uint late_count_stride = 0u;
        VbLateCount.GetDimensions(late_count_elems, late_count_stride);
        VbLateCount[late_count_elems - 1u] = uni.frame_index;
    }

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
    //
    // # VG R3 piece 3 step P3-4 (plan A2 / D1): THE LOOP BECOMES A TWO-WAY PARTITION
    //
    // The frustum survivors split into DRAWN NOW (`VbVisibleInstance`, cursor `k`) and DEFERRED
    // (`VbLateVisible`, cursor `n_defer`). ⚠️ AN EARLY REJECT REMOVES NOTHING:
    //
    // > INVARIANT VG-P3-RECOVERY. Every instance the early phase defers is a member of this batch's
    // > late candidate list, and the late phase re-tests it against the pyramid built from THIS
    // > frame's early depth with THIS frame's view-projection. The early phase therefore PARTITIONS
    // > the frustum survivors and never removes one.
    //
    // That is what makes the early predicate's own unsoundness harmless. The pyramid it reads is the
    // one this frame's build has NOT yet overwritten — frame N−1's content, whose texels belong to
    // the PREVIOUS camera — so testing it with the CURRENT view-projection is not a coherent query
    // about anything. It does not need to be: an early false-reject costs a late draw and nothing
    // else. (Nanite records the same weakness as a HIT-RATE statement for exactly this reason.)
    //
    // `k + n_defer <= frustum survivors <= d.instance_count`, so BOTH region writes stay inside this
    // batch's own range — the budget piece 2's D5 proved, used. `VbLateVisible` is addressed by the
    // SAME `base_instance` / `instance_count` pair as `VbVisibleInstance`:
    //
    // > INVARIANT VG-P3-LATE-REGION. Batch `b` owns
    // > `[base_instance_b, base_instance_b + instance_count_b)` of `VbLateVisible` and writes nowhere
    // > else — the same host-established disjointness R2d-REGION-DISJOINT gives `VbVisibleInstance`,
    // > from the same `VbBatchDesc` fields. The EARLY phase writes `[base, base + n_defer)`; the LATE
    // > phase reads that prefix and writes `[base, base + n_keep)` with `n_keep <= n_defer`. The only
    // > dereferencing reader is the late raster's VS, bounded by
    // > `SV_InstanceID < instanceCount = n_keep`. No tail fill is required, for verbatim
    // > R2d-REGION-DEFINED's reason.
    //
    // Overflow is not a policy question here and that is a genuine divergence from the field: niagara
    // drops draws past `TASK_WGLIMIT` and Nanite drops clusters past `MaxCandidateClusters`, both
    // surfacing as blinking geometry. Every list here is exactly the size of the region it partitions.
    uint k = 0u;
    uint n_defer = 0u;
    for (uint j = 0u; j < d.instance_count; ++j) {
        const uint g = d.base_instance + j;
        const VbInstanceRow row = gVbInstances[g];
        const MeshLocalBounds b = gMeshBounds[row.mesh_id];
        // Seeded KEEP, and only a known box may lower it: the conservative direction is the
        // DEFAULT of this predicate rather than a case it remembers to handle.
        bool keep = true;
        // Seeded NOT-DEFERRED, and only an ARMED, MARKED, frustum-surviving, KNOWN-bounds instance
        // may raise it.
        bool defer = false;
        if (!any(b.bmin > b.bmax)) {
            float3 wc;
            float3 wh;
            arvo_world_box(row, b, wc, wh);
            keep = !aabb_outside_frustum(wc - wh, wc + wh);
            // ⚠️ THE OCCLUSION TEST IS INSIDE THE SENTINEL GUARD, not after it. An UNKNOWN-BOUNDS
            // instance is drawn by the EARLY scope, never frustum-tested and never occlusion-tested
            // — the ONE reading under which absence of bounds is not treated as evidence of
            // invisibility. Hoisting this out would put the sentinel's inverted fold through the
            // projection, where a zero linear part collapses the "unbounded" box to a POINT at the
            // instance's translation that anything occludes.
            //
            // FORCE_KEEP is tested in the GUARD and FORCE_LATE in the VALUE, so the machinery still
            // runs under FORCE_KEEP (it is a zero control, not a bypass) while FORCE_LATE defers
            // every marked survivor whatever the verdict says.
            if (keep && occ_armed && !occ_force_keep
                && (row.flags & VB_INST_FLAG_OCCLUSION_CULLING) != 0u) {
#ifdef VB_CULL_DEBUG_PROBE
                g_vb_dbg_slot = g;
#endif
                defer = occ_force_late || occlusion_reject(uni, wc - wh, wc + wh);
            }
        }
        if (!keep) {
            // Frustum-rejected: NEITHER list, exactly as before this step. VG-P3-RECOVERY covers the
            // deferred set, not this one — and it does not need to, because a box wholly in one
            // plane's negative half-space is certainly invisible.
            continue;
        }
        if (defer) {
            VbLateVisible[d.base_instance + n_defer] = g;
            ++n_defer;
        } else {
            // The STORED value is the GLOBAL instance index, never the compacted slot `k` — see
            // `vb_raster.vs.hlsl`'s INVARIANT R2d-EXPORT-IS-GLOBAL for everything keyed on it.
            // `VbLateVisible` stores the same encoding, which is what lets `vb_set0_late` swap one
            // descriptor and leave the VS byte-unchanged.
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

    // VG R3 piece 3 step P3-4 (plan A2 / D3): this batch's deferral count, in its OWN array and
    // ⚠️ NOT in `VbIndirectLate[i].instanceCount`. That distinction is the deepest correction round 1
    // of the plan needed: had the early phase written the late RECORD, the late cull would have
    // stopped being that word's only producer, a frame whose late cull did not run would have drawn
    // `n_defer` UNTESTED instances instead of nothing, and "the late phase is load-bearing" would
    // have had no image-level control at all — deleting the late cull draws a SUPERSET of the correct
    // set at identical depth, which is pixel-invisible under `VK_COMPARE_OP_GREATER` with no
    // `discard` and no `SV_Depth`.
    //
    // The `visible ?` gate mirrors the record store's, term for term: a level-1-culled batch draws
    // nothing early and must defer nothing late, or the late phase would compact candidates for a
    // batch whose union box is wholly outside the frustum.
    //
    // Gated on `occ_armed` — see the header. The late phase reads this word under the same bit, so
    // the pair is consistent in both states rather than only in the armed one.
    if (occ_armed) {
        VbLateCount[i] = visible ? n_defer : 0u;
    }

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
