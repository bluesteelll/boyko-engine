# Architecture: VG R3 piece 3 of 4 — the CULL INTEGRATION (the pyramid finally does something)

Status: **DESIGN, round 1.**

Scope fixed by `docs/OPEN-QUESTIONS.md` ("RESOLVED 2026-08-03 — decomposed"), piece 3 verbatim:
*the occlusion decision itself*. Pieces 1 and 2 are SHIPPED: the pyramid is built every armed frame
and read by nothing (`docs/VG-R3-P1-PYRAMID-PLAN.md`); the capability, the per-instance flag bit,
`vb_indirect_late` and a fully recorded LATE RASTER SCOPE that draws nothing exist
(`docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md`). The field survey is
`docs/VG-R3-TWO-PHASE-OCCLUSION-RESEARCH.md`, re-verified and extended for this round.

> **Anchors.** Every `file:line` below was re-verified against the working tree on 2026-08-06,
> AFTER piece 2 landed (which moved `graph_bridge.rs` and `vb.rs` substantially) and after
> `1977fe0` re-cut the framegraph's unwritten-read backstop. Piece 1 lost a round to stale anchors
> and piece 2's own critique repeats the warning: treat every anchor as **name + hint** and grep the
> name.

---

## Goal

Make the early raster draw LESS, and prove that what it stopped drawing was invisible.

Concretely, three things become true that are false today:

1. **The pyramid acquires a reader.** `vb_batch_cull.comp.hlsl` gains a per-instance occlusion test
   that is a statement-for-statement mirror of `boyko_render::hzb`'s
   `project_aabb → select_texels → occluder_depth → occlusion_verdict` chain.
2. **The two-phase partition becomes real.** The early cull splits each batch's frustum survivors
   into *drawn now* and *deferred*; the late cull re-tests the deferred set against the pyramid
   built from the early depth and writes the late records' `instanceCount`.
3. **The late scope draws.** Piece 2's two "PIECE 2 ONLY" tripwires are deleted deliberately, and
   the survivor-indirection bit is set on the late push.

**Functional target — the claim the gates are built around:**

> On any scene, the image produced with the cull ARMED is **byte-identical** to the image produced
> with the cull DISARMED, while the GPU reports a **nonzero rejection count** in the same run.

Neither half alone is a gate. Byte-identity alone is satisfied by a cull that rejects nothing (the
campaign has shipped that failure five times). A rejection count alone is satisfied by a cull that
deletes visible geometry. **The conjunction, measured in one sitting, is the gate.**

**Explicitly NOT a goal: a performance claim.** `docs/VG-DECIDABILITY-FLOOR.md`'s own measurement
(6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol) makes any delta under ~15 % undefendable
on this machine, and the committed corpus has no occlusion-dominated scene. Piece 3 adds one compute
dispatch (whose ~13.9 µs fixed cost VB-P1d measured as dispatch-intrinsic) and removes raster work
whose magnitude is scene-dependent. **No number is claimed. Piece 4 owns the knob; a perf case, if
one is ever made, needs a scene built for it.** For calibration only, and not as a target: the one
published two-phase timing table (Ubisoft, SIGGRAPH 2015, Xbox One 1080p) records the phase-2 draw
at **< 0.01 ms** — i.e. on a scene where the scheme works, the late scope is nearly free and is
therefore nearly indistinguishable from not existing. That is the regime this piece lands in.

---

## Context and constraints

### The six obligations piece 3 INHERITS, each written into the shipped code for this reader

| # | obligation | where it is written | discharged by |
|---|---|---|---|
| 1 | `vb_indirect_late`'s declared writer changes from `(TRANSFER, TRANSFER_WRITE)` to `(COMPUTE_SHADER, SHADER_WRITE)` | `scene_types.rs:3157-3159`; the guard that now catches the omission at `graph.rs:652-722` (P2-8, `1977fe0`) | **D8** |
| 2 | the late scope must declare `vb_instance_ring` and `vb_visible_instance` | `graph_bridge.rs:4049-4056` ("WHAT IS DELIBERATELY *NOT* DECLARED, and what piece 3 must add") | **D5 + D8** — with a SUBSTITUTION: the late scope binds `vb_late_visible`, not `vb_visible_instance` |
| 3 | two "PIECE 2 ONLY" tripwires deleted deliberately | `vb.rs:1224-1230` (`instance_count == 0`), `vb.rs:1802-1806` (indirection bit clear) | **D9** |
| 4 | the cross-frame WAR on the non-ringed pyramid | `graph_bridge.rs:3497-3512` ("PIECE 3 ADDS THE READER AND IS WHERE THIS MUST BE REVISITED") | **D2** |
| 5 | `hzb_dump`'s depth copy moves between the scopes, or both depths are dumped | `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` D6's hazard note; the copy at `vb.rs:3509-3541` | **D10** — both depths |
| 6 | the pyramid is read POINT-SAMPLED | `VG-R3-P1-PYRAMID-PLAN.md` §7 (`SAMPLED_IMAGE_FILTER_LINEAR` is not mandatory for `R32_SFLOAT`) | **D7** — discharged STRUCTURALLY: no `VkSampler` is created |

### Hard local constraints, each verified in this tree

| # | constraint | anchor |
|---|---|---|
| C1 | **`vkCmdDrawIndexedIndirectCount` is not in the device fn table**, and adding it is a `VkPhysicalDeviceVulkan12Features` chain edit, not a fn-table line | `device.rs:615-618` |
| C2 | **`multiDrawIndirect` is off** ⇒ `draw_count ∈ {0,1}`; the only GPU-writable knob per draw is `instanceCount` | `vb.rs:1817-1820` |
| C3 | **The compute push range is const-asserted ≤ 128 B** (the Vulkan-guaranteed floor) and `VB_BATCH_CULL_PUSH_BYTES` is already **104**. **24 bytes of headroom. A `float4x4` does not fit.** | `rhi_impl/mod.rs:212-232`, `compute.rs:1701` |
| C4 | **`robustBufferAccess` is OFF**; an out-of-bounds buffer read is silent corruption | `gpu_scene/mod.rs:256-257` |
| C5 | **One device queue, one queue family** — `get_device_queue(device, queue_family_index, 0, ..)`; no async compute exists | `device.rs:1160`, `:3261` |
| C6 | **The cull dispatches ONE LANE PER BATCH**, looping over that batch's instances serially | `vb_batch_cull.comp.hlsl:400-408`, `:441-466`; `groups` at `vb.rs:1329` |
| C7 | **`VB_VISIBLE_INSTANCE_ELEMS == INSTANCE_CAPACITY`** is an equality sound in BOTH directions (R2d-4 ⊇ / R2d-6 ⊆) | `gpu_scene/mod.rs:264-288` |
| C8 | **INVARIANT R2d-REGION-DEFINED**: every reader of `vb_visible_instance` must be bounded by the same `k` the cull stores into record word 1, **or the tail must be filled** | `vb_batch_cull.comp.hlsl:118-153`, esp. `:150-153` |
| C9 | **INVARIANT R2d-REGION-DISJOINT**: bases strictly ascending, regions pairwise disjoint, established on the HOST — which is why the region write needs no atomic | `vb_batch_cull.comp.hlsl:101-116` |
| C10 | **`hzb_arm` is a STORED field** on `GBufferTargets` and part of the recreate predicate, so the pyramid's presence cannot flip inside one targets generation | `targets.rs:672`, `:7718`, `:7781-7785` |
| C11 | **Dense components are pinned `ResidencyKind::Cpu`** — a dense column can never be GPU-resident | `boyko_macros/src/component.rs:544-546`; `docs/DENSE-COMPONENTS-PLAN.md:61` |
| C12 | **`GpuColumnManager` has ZERO production call sites** — every `create_column` caller is a `boyko_render` test | `gpu_column.rs:639-706` and its eight test callers |
| C13 | **The VB ring index is NOT stable across frames.** Archetype rows use `swap_remove` on despawn; a mesh leaving `Loaded` deletes a whole bucket and shifts every later `offsets[m]` | `archetype.rs:1019-1057`; `mesh_draw.rs:768-811`, `:780`; the warning at `mesh_draw.rs:1246-1249` |

### Invariants that must survive untouched

| invariant | anchor |
|---|---|
| `first_instance == 0` in every record (`drawIndirectFirstInstance` is VK_FALSE) | `vb.rs:1213-1223`, assert `:1220-1223` |
| R2d-REGION-DEFINED / R2d-DISJOINT on `vb_visible_instance` | C8 / C9 above |
| R2d-EXPORT-IS-GLOBAL — the VS exports the GLOBAL instance index into `vb_id` | `vb_raster.vs.hlsl:98-107`, `:209` |
| `hzb_poison` declared before every `hzb_build`; every `hzb_build` before `hzb_dump`; `poison.is_some() == dump.is_some()` | the trio in `declare_vb_graph`'s tail asserts |
| declare/record ORDER parity, and the recorder's gate is the declarator's VERBATIM | `vb.rs:1156-1161`, `:3420-3426` |
| the late scope's `renderArea` equals the early scope's | `vb.rs:1746-1754` |

---

## Key decisions

### D1 — The EARLY predicate: an HZB re-test against the pyramid AS THE PREVIOUS FRAME LEFT IT, with the CURRENT frame's transforms and view-projection. **Not** a stored per-object visibility bit.

**What.** The early cull, after its existing per-instance frustum test, applies the occlusion test to
every instance whose `VbInstanceRow.flags` bit 0 is set. The pyramid it reads is the one this frame's
`hzb_build` has *not yet* overwritten — i.e. frame N−1's content. An instance the test rejects is not
dropped: it is appended to the batch's **late candidate** list.

**Why not niagara's stored visibility bit — and this is a structural refutation, not a preference.**
The research records the fork as a genuine architectural disagreement between shipped, performance-
serious codebases (niagara and Granite store a bit; Nanite and Bevy re-test against the previous
pyramid). **The tree closes it for this engine:**

- A per-object bit is **durable per-entity data**, so Principle 0 puts it in the ECS's own storage —
  a component column or a dense component. Not a raw SSBO conjured beside it.
- **The only storage with a stable per-entity row is `Dense`, and `Dense` is force-pinned
  `ResidencyKind::Cpu`** (C11). A dense column cannot be GPU-resident, and the bit's producer is the
  GPU.
- **The only GPU-resident mechanism is `GpuColumn`, which requires TABLE storage** — whose rows
  `swap_remove` on despawn — **and has zero production call sites** (C12).
- Keying the bit by the VB ring index instead is wrong on its own terms: **the ring index is not
  stable across frames** (C13). A spawn, a despawn, an archetype migration or a mesh leaving
  `Loaded` renumbers it, and `mesh_draw.rs:1246-1249` already warns in those words about a
  *filtering* query silently renumbering the ring.
- Making it right would mean minting a stable per-instance GPU key and a durable buffer indexed by
  it, plus a GPU→CPU writeback or a new GPU-writable column mechanism. **That is a larger change than
  the cull itself.**
- Nanite's own justification for having no inter-frame list (continuous LOD + streaming make cluster
  IDs unstable) does not apply here — but the *conclusion* does, for a different reason: this
  engine's IDs are unstable because the gather rebuilds them, not because the geometry churns.

**What adopting the previous-pyramid predicate COSTS, stated because the research makes it precise.**
niagara's stored bit removes the cross-frame HZB read **entirely** — it builds the pyramid once per
frame, after the early render, and reads it only in that same frame's late cull. Choosing the
previous-pyramid predicate takes on the cross-frame hazard as a real obligation. **D2 discharges it,
and D2 exists because of this decision.**

**What it does NOT cost: a second pyramid build.** Bevy builds the pyramid twice per frame (early and
late). This design builds it **once**, between the two raster scopes, and reads it twice — stale by
the early phase, fresh by the late phase. A second build after the late scope would be pure loss:
**nothing in this engine consumes a pyramid of the final depth.** That is a hybrid of Bevy's
predicate and niagara's build schedule, and it is cheaper than either.

**Why the current view-projection against a stale pyramid is SOUND.** It is not a coherent query
about anything — the pyramid's texels belong to the previous camera. Correctness does not rest on it:

> **INVARIANT VG-P3-RECOVERY.** Every instance the early phase rejects is a member of that batch's
> late candidate list, and the late phase re-tests it against the pyramid built from THIS frame's
> early depth with THIS frame's view-projection.

So an early false-reject costs a late draw and nothing else. The early predicate is a *heuristic
about how much work the early pass saves*, and its only failure mode is doing more work. Nanite's
recorded weakness of the same class — "when turning the camera a slice of the screen near the edge
will have no previous-frame data to be culled against" — is a hit-rate statement, not a correctness
one, for exactly this reason.

**Why the LATE test is exactly sound**, which is the half that can delete geometry:

> The late test rejects instance *i* iff *i* is occluded by the set of instances drawn in the EARLY
> scope, at their current positions. Occlusion by a SUBSET of the scene implies occlusion by the
> scene. Therefore a late reject is genuinely invisible.

**Why not the PREVIOUS view-projection** (Bevy std-mesh / Nanite, which reproject): it needs a second
`float4x4` and a definition of "previous" that survives resize, path switches and the first frame,
for a difference that changes only the early phase's hit rate — a quantity this machine cannot
measure to better than ~15 %. The current view-projection is **already in hand**: the frustum planes
the cull is handed are extracted from the same 64 push bytes the raster VS reads
(`gpu_scene/mod.rs:6480-6484`), so one matrix serves both.

**Convergence, and why the boot clear in D2 is load-bearing rather than hygiene.** Let *E* be the
early-drawn set and *P(E)* the pyramid built from its depth. Next frame's early set is
`E' = {i : ¬occluded by P(E)}`. Starting from a pyramid of 0.0 — the reverse-Z far plane, which
rejects nothing — frame 1 has `E = ` all frustum survivors, so `P(E)` is the true depth and `E'` is
the visible set from frame 2 onward. **One frame to converge, with no special case in any shader.**
⚠️ **That convergence is also a gate hazard: on frame 1 the cull provably rejects NOTHING.** See
G-P3-A's frame-index requirement, which exists entirely because of this sentence.

**Trade-off.** The occlusion test runs TWICE per marked instance on a frame where the early phase
rejects it (once per phase), and once otherwise. It is ~8 corner projections plus 4 image loads. It
is paid per instance on a per-batch lane (C6), i.e. serially within a batch — an inherited property
of R2c0's dispatch shape, named in Boundary.

---

### D2 — The pyramid becomes a CROSS-FRAME resource: seed `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)`, plus a one-time boot clear to `0.0`

This is obligation 4, and `graph_bridge.rs:3497-3512` predicted the answer verbatim.

**Why `ResSync::undefined()` is not merely unsynchronised but WRONG the moment a reader exists.** A
first touch derives `oldLayout = UNDEFINED`, which **licenses the driver to discard the image
contents**. With the early cull reading the pyramid, frame N+1 would be reading an image the graph
has just told the driver it may throw away. The failure is content-dependent and motion-dependent —
verbatim the engine's recorded "wrong only in motion, stable when stopped" fingerprint.

**The chain, on an armed-split frame** (`vb_batch_cull` is declared at `graph_bridge.rs:3879`,
before `vb_raster`; the poison+build block moves between the scopes at `:4024-4029`):

```
vb_batch_cull  COMPUTE  SHADER_READ    <- frame N-1's content
hzb_poison     TRANSFER TRANSFER_WRITE (dump frames only)
hzb_build_*    COMPUTE  SHADER_WRITE / SHADER_READ
vb_cull_late   COMPUTE  SHADER_READ    <- this frame's content        (NEW, D4)
hzb_dump       TRANSFER TRANSFER_READ  (dump frames only)
```

- **Cross-frame RAW** (frame N's `hzb_build_{n-1}` write → frame N+1's `vb_batch_cull` read): carried
  by the seed. `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` is exactly
  `shadow_temporal_hist_read`'s shape at `graph_bridge.rs:3469-3476`.
- **Cross-frame WAR** (frame N's `vb_cull_late` read → frame N+1's `hzb_poison`/`hzb_build_0` write):
  subsumed. Frame N+1 derives an intra-frame WAR against its OWN `vb_batch_cull` read (COMPUTE,
  SHADER_READ → the write), and the Vulkan spec is explicit that a `vkCmdPipelineBarrier` recorded
  **outside a render pass instance** has a first synchronization scope that "includes all commands
  that occur earlier in **submission order**" — and submission order is defined across
  `vkQueueSubmit` calls, `VkSubmitInfo`s and command buffers. It therefore includes frame N's
  commands, because **there is exactly one queue** (C5).

**Frames-in-flight does not enter the argument, and that is the crux.** `FRAMES_IN_FLIGHT` exists to
let the HOST re-record command buffers and rewrite host-visible data without waiting. The pyramid is
never touched by the host. Its only producers and consumers are queue operations, ordered by
submission order and the barriers above. This is why niagara ships one `depthPyramid` image with
three frames in flight, and why Bevy ships one `ViewDepthPyramid` per view.

**The premise is named so it can be re-derived when it stops holding: this argument is valid only
while the engine submits every pass to ONE queue, and only while every one of these barriers is
recorded OUTSIDE a render pass instance. The day async compute lands (Pillar A's Phase 3, not built),
submission order stops being a total order and the pyramid must be re-examined or ringed.**

**Why not ring the pyramid per FIF.** It would remove the argument's dependence on the single-queue
premise, at 2× the pyramid's VRAM (~2.8 MB at 1920×1080, ~5.6 MB ringed) and — the real cost — it
would double `HzbTargets::level_views`, `HzbTargets::sets` and every declared span, and it would make
`hzb_dump` ambiguous about which slot it copied. Rejected: the barrier argument is rigorous under a
premise that is *stated and checkable*, and the ring buys nothing else. Its one genuine benefit is
cross-frame overlap — a single image forces frame N+1's early cull to wait on frame N's pyramid
build — and that is a performance property this campaign cannot measure (see Goal).

**The boot clear.** `HzbTargets::build` (`targets.rs:1292-1330`) already issues a one-shot
`UNDEFINED → GENERAL` transition; it gains one `vkCmdClearColorImage` filling mips `[0, levels)` with
`0.0`. The usage bit it needs — `TRANSFER_DST` — is already on the pyramid (added at P1-8 for
`hzb_poison`). Three reasons, in order of weight:

1. It removes a read of uninitialised image data. Vulkan bounds image accesses unconditionally, so
   this is not a fault — but the VALUE is undefined, and a large undefined value under reverse-Z is a
   near occluder that rejects everything in its footprint.
2. `0.0` is the far plane, so an unbuilt pyramid **provably rejects nothing** — the invariant "the
   pyramid always holds a conservative lower bound over its footprint" becomes true from birth
   instead of from frame 2.
3. It makes convergence one frame (D1), which is what keeps a resize from producing a visible
   late-draw spike.

⚠️ **It does not weaken G8/G5.** `hzb_poison`'s `-1.0` is per-dump-frame and runs after the boot
clear; the poison argument (`VG-R3-P1-PYRAMID-PLAN.md` §14) is untouched.

**Trade-off, stated.** The seed change moves the derived barrier stream on **existing**
configurations: today's `vb_mesh_hzb` frame derives `UNDEFINED → GENERAL` at `hzb_build_0`'s first
write; it will derive a `GENERAL → GENERAL` WAW against `(COMPUTE, SHADER_WRITE)`. G-P3-F's U-rows
move, deliberately, and are re-pinned in the same commit with the reason in the commit message. This
is the ONE place piece 3 changes a stream on an unsplit frame, and P3-0 lands it alone.

---

### D3 — The LATE input is a COMPACTED early-reject list, in its OWN buffer `vb_late_visible`, which is ALSO the late survivor list

**Why a compacted list rather than re-scanning all instances.** niagara re-scans all N with a
different predicate, and that is the cheaper option where it works. **It is not expressible here**:
the early predicate reads the PREVIOUS pyramid, and by the time the late cull runs the pyramid has
been overwritten by this frame's build. The late phase cannot recompute what the early phase decided.
(niagara escapes this only because its early predicate is a stored bit, which D1 rejects.) So the
early verdict must be *stored*. Storing it as a compacted list of ring indices is strictly better
than a per-instance flag array: it is smaller, it is what the late phase actually iterates, and it
doubles as the output.

**Why a NEW buffer rather than two-ended packing inside `vb_visible_instance`.** Piece 2's D5 fixed
the *budget* (`|early| + |late| ≤ instance_count`, so one region suffices) and explicitly left the
*packing* to piece 3, while saying "the late scope gets no survivor list of its own, ever". **This
plan takes the budget and rejects the never**, for reasons that are about gate quality:

| | two-ended in `vb_visible_instance` | separate `vb_late_visible` (chosen) |
|---|---|---|
| VRAM | 0 new | +4 KiB × FIF = 8 KiB |
| descriptor bindings | 0 new | +1 on `vb_cull_layout`, +1 set (`vb_set0_late`, D5) |
| **`vb_raster.vs.hlsl`** | **must gain a DESCENDING index path and a third flags bit** — the late survivors compact toward the region's END, so the VS must read `visible[anchor − 1 − id]` | **byte-unchanged** |
| in-place compaction | reads `[base+count−1−j]`, writes `[base+count−1−keep]`; safe only by a non-obvious lemma | reads `[base+j]`, writes `[base+keep]`, `keep ≤ j`, and every later read is at a strictly LARGER index — **one line** |
| `vb_visible_instance`'s R2d invariants | a second reader indexing the region by something other than a compacted `SV_InstanceID` — permitted by C8 only if given a per-batch survivor count, i.e. exactly the case `vb_batch_cull.comp.hlsl:150-153` says "do not weaken" | **literally untouched** |
| `vb_cull_readback` probe | observes the LATE partition; the early partition is destroyed | observes BOTH, in separate regions |

**The decisive one is row 3.** Piece 3 is the first piece whose change is supposed to move pixels. If
`vb_raster.vs.hlsl` also changes, a pixel diff cannot separate "the cull rejected wrongly" from "the
VS indexes wrongly". **Keeping the rasteriser's `.spv` byte-identical means every pixel change is
attributable to the cull alone** — that is a gate-quality argument, not a convenience one, and it
buys the entire gate section its meaning. 8 KiB is the price.

**The region rule for the new buffer, stated in its own terms** (a fresh instance of the same shape,
not a weakening of C8):

> **INVARIANT VG-P3-LATE-REGION.** Batch `b` owns `[base_instance_b, base_instance_b + instance_count_b)`
> of `vb_late_visible` and writes nowhere else — the same host-established disjointness C9 gives
> `vb_visible_instance`, from the same `VbBatchDesc` fields. The EARLY phase writes
> `[base, base + n_defer)`; the LATE phase reads that prefix and writes `[base, base + n_keep)` with
> `n_keep ≤ n_defer`. The only dereferencing reader is the late raster's VS, bounded by
> `SV_InstanceID < instanceCount = n_keep`. No tail fill is required, for verbatim C8's reason.

`VB_LATE_VISIBLE_ELEMS == INSTANCE_CAPACITY`, const-asserted against `VB_VISIBLE_INSTANCE_ELEMS`, so
`vb_cull_batch_count_visible_clamp` (`vb.rs:236-244`) bounds BOTH lists with the one number it
already computes. **C7's equality is untouched in both directions** — the new buffer is a different
constant family, exactly as `vb_indirect_late` is.

**Overflow is impossible, which is a genuine divergence from the field.** niagara drops draws past
`TASK_WGLIMIT` and Nanite drops clusters past `MaxCandidateClusters`, surfacing as blinking geometry.
Here every list is exactly the size of the region it partitions (`n_defer + k ≤ instance_count`), so
there is no overflow policy to get wrong and no drop to make visible.

---

### D4 — The LATE dispatch is FIXED and HOST-SIZED: `batch_count` lanes, the SAME shape as the early one. No `vkCmdDispatchIndirect`, no `…Count`, no readback.

Because the cull dispatches **one lane per batch** (C6) and `batch_count` is a host number computed
once at `vb.rs:1021-1026`, the late dispatch's size is known on the host. The GPU-only quantity is
the *per-batch* candidate count, which the lane reads from its own record.

⇒ **Adding `vkCmdDrawIndexedIndirectCount` (C1) or `vkCmdDispatchIndirect` is OUT OF SCOPE and NOT
NEEDED.** Stated flatly so it cannot be re-litigated. Bevy's `atomicAdd(work_item_count); if (i % 64
== 0) atomicAdd(dispatch_x)` is the cheapest published way to size an indirect dispatch from an
append list — and it is machinery this design does not require, because the domain of the late
dispatch is batches, not survivors. The late scope records `batch_count` plain
`vkCmdDrawIndexedIndirect` calls whose `instanceCount` the late cull wrote — piece 2's structure,
with one word's producer changed, exactly as it promised.

**This is the design the constraint selects, not a workaround.** Of the two shapes that survive
without `…Count` — (a) a fixed-length record array with zero `instanceCount` for culled entries, and
(b) a dense per-batch prefix with a GPU-written survivor count — this engine already implements (b)
at instance granularity, and piece 3 replicates it for the late array. vkguide and pcwalton both warn
against (a); nobody has published a native-Vulkan measurement of its cost, and this design does not
need one.

**Where the candidate count lives.** The early phase writes it into `vb_indirect_late[b].instanceCount`
(word 1, offset 4, the same address arithmetic as `vb_batch_cull.comp.hlsl:483-485`). The late phase
reads it, refines it and writes it back. No third buffer, no counter, no atomic.

**The alternative rejected:** a separate per-batch `n_defer` array. It is one more buffer, one more
binding and one more region invariant, to avoid a read-modify-write of a word this pass already owns.

**Two dispatches, not one, and it is not a choice.** The late test needs the pyramid, which needs the
early depth, which needs the early raster, which needs the early cull's output. The chain is serial
by construction.

---

### D5 — ONE shader, ONE pipeline, ONE entry point, a UNIFORM branch on `pc.phase`; and the late raster scope binds a SECOND descriptor set differing in one entry

**The shader.** `vb_batch_cull.comp.hlsl` gains a `phase` push word. `phase == 0` is the early pass
(frustum + occlusion vs the stale pyramid + the two-way partition); `phase == 1` is the late pass
(read the candidates + occlusion vs the fresh pyramid + in-place compaction).

**Why one module and not a `-D` variant pair.** The shared part — `project_aabb`, `select_texels`,
`occluder_depth` and the verdict — is the part VG-P3-RECOVERY depends on being the SAME function in
both phases. Two artifacts means two implementations that can drift, and drift in the direction where
the late test is stricter than the early one is **geometry deletion**. This is verbatim
`hzb_build.comp.hlsl`'s own argument for a uniform `pc.base_level == 0` fork over two variants
(`VG-R3-P1-PYRAMID-PLAN.md` §8): the hard code exists exactly once. No
`docs/SHADER-VARIANT-MANIFEST.md` row is added, because that manifest registers `-D` variants only.

**Why the late raster gets `vb_set0_late` instead of a VS edit.** The VS reads
`visible_instances` at `[[vk::binding(11, 0)]]` (`vb_raster.vs.hlsl:167`) and resolves
`visible_instances[pc.base_instance + instance_id]` (`:201-203`). Binding `vb_late_visible` at that
slot for the late scope makes the *identical expression* read the late list at the *identical base*.
`vb_set0_late` is `vb_set0` with one entry changed, per FIF. The late scope already rebinds set 0
explicitly (`vb.rs:1774-1783`), so this is a one-token change at the bind site.

⇒ **`vb_raster.vs.hlsl`, `vb_raster.fs.hlsl`, `vb_geom_fetch.hlsli` and every raster `.spv` are
BYTE-UNCHANGED.** Only `vb_batch_cull.comp.hlsl` and its `.spv` move.

**How this discharges obligation 2, by substitution rather than by omission.** The late scope's VS
reads `vb_instance_ring` @0 and `vb_late_visible` @11. Both become real reads the moment
`instanceCount` is nonzero, and both are declared on `vb_raster_late` in the same change (D8).
`vb_visible_instance` is **not bound to the late scope at all** and therefore is not declared on it —
which is the R2d-3 rule applied correctly ("a bound descriptor is declared regardless"), not waived.
**A reviewer looking for `vb_visible_instance` on `vb_raster_late` should find its absence explained
here and in the code comment, not discover it.**

---

### D6 — The cull's new inputs travel in a per-FIF UNIFORM BUFFER, not push constants; the push grows by 8 bytes

C3 is decisive: 24 bytes of push headroom, and the occlusion test needs a `float4x4` (64 B) plus the
pyramid's source and base extents and level count.

**`VbCullUniform`, 96 bytes**, written by `vkCmdUpdateBuffer` and read as a `StructuredBuffer` — a
new binding on `vb_cull_layout`. Its write and its read are declared **inside the existing
`vb_batch_cull` pass**, as a `TRANSFER_WRITE` followed by a `COMPUTE SHADER_READ` — verbatim the
intra-pass shape the counter fill already uses (`graph_bridge.rs:3885-3886`,
*"the SAME intra-pass TRANSFER -> COMPUTE shape `light_cull` uses for `light_index_alloc`"*). **No
new pass.**

**Rejected alternatives.**
- *Raise the push range and probe `maxPushConstantsSize`.* Destroys `rhi_impl/mod.rs:204-205`'s stated
  property — "so no device-limit query is required" — for one matrix.
- *Drop the six planes and derive them in-shader from the view-projection.* Saves 96 B of push, but
  the planes are extracted host-side today (`frustum_planes_from_push_bytes`), and re-deriving them
  on the GPU changes their floating-point evaluation order, hence the frustum verdicts at the
  boundary, hence pixels — in a step whose whole claim is that only the OCCLUSION decision moves.
- *Widen `VbBatchDesc` with a per-batch copy of the matrix.* 64 B × 1024 batches of identical data.

**Push grows 104 → 112**: `phase: u32`, `occ_flags: u32`. Still inside the 128-byte floor, and
`COMPUTE_PUSH_CONSTANT_RANGE_BYTES`'s const-assert (`rhi_impl/mod.rs:227-232`) is the mechanical
gate on that.

**The matrix is uploaded in MATH-ROW form** — `pv[row][col]`, `clip = pv · world` — which is exactly
what `boyko_render::hzb::project_aabb` takes (`hzb.rs:687-692`, layout note at `:660-668`). The host
performs the one byte inversion from the column-major push buffer, at ONE site, so the shader's
`dot(row_r, world4)` and the oracle's `pv[r] · world` are the same four products in the same order.

---

### D7 — The pyramid is read with `.Load(int3(x, y, level))` through the texture's own mip-complete SAMPLED view. **No `VkSampler` is created anywhere in this piece.**

Obligation 6, discharged structurally rather than by discipline: `.Load` takes integer coordinates
and an explicit mip index. **There is no filter to get wrong, and no
`SAMPLED_IMAGE_FILTER_LINEAR` / `VK_EXT_sampler_filter_minmax` dependency to probe.**

The two shapes the field uses are a `VK_SAMPLER_REDUCTION_MODE_MIN` sampler with `VK_FILTER_LINEAR`
(niagara — legal because the reduction mode *replaces* the weighted average with a component-wise
minimum, per the Vulkan sampler chapter) and four `textureLoad`s with a shader-side `min` (Bevy,
whose own comment says it would use a min reduction if wgpu exposed one). **Piece 2 already fixed
Bevy's practice as this engine's** — it matches `hzb_build.comp.hlsl`'s existing `.Load`-only
discipline — **and D7 is that decision landing.**

Why a plain linear filter would be unsound, stated once: a reduced pyramid stores a *bound over a
footprint*, not a band-limited signal. A bilinear blend of four reduced texels is a convex
combination, so it lies strictly between their min and max — neither an upper nor a lower bound.
Under reverse-Z with a `min` reduce the stored value must be ≤ every depth in the footprint; a blend
can be *greater*, and therefore reject something visible. **False negatives are missing geometry, the
one failure mode that is not recoverable.**

Four loads, folded with the same `conservative_min` the oracle uses (`hzb.rs:503-510`: NaN on either
side → `-INFINITY`, else `if b < a { b } else { a }`).

**Two lemmas that make the loads provably in range** — so the design does not lean on Vulkan's
unconditional image bounding, whose "returns undefined values" is agreement-breaking even when it is
memory-safe (`VG-R3-P1-PYRAMID-PLAN.md` §3's corrected wording):

1. `texel_of(x) = (x · base) / source` is computed by the oracle in **u64** and by the shader in
   **u32**. They agree because the true product fits: `x < source ≤ MAX_HZB_EXTENT = 65536` and
   `base ≤ source`, so `x·base ≤ (source−1)·source ≤ 65535 · 65536 = 4 294 901 760 < 2³²`.
   **No `Int64` capability is requested.**
2. `tx = texel_of(px) >> level` is always `< level_extent(level)`. `base` is a power of two, so for
   `level < log2(base)`, `(base−1) >> level = (base >> level) − 1`; for `level ≥ log2(base)`,
   `level_extent = 1` and `tx = 0`.

**The `firstbithigh` trap, named because it is exactly this campaign's class of defect.** The oracle
defines `msb(0) := 0` (`hzb.rs:492`). HLSL's `firstbithigh(0)` returns `0xFFFFFFFF`. The selector
`level = max(msb(tx0 ^ tx1), msb(ty0 ^ ty1))` hits `0` whenever a rect fits in one texel — **the
common case**. The shader spells `v == 0u ? 0u : firstbithigh(v)`, and the corpus gate below contains
single-texel rects by construction.

**The engine's selector is already the refined form.** niagara and interplayoflight both compute
`ceil(log2(max_extent))` and then apply a one-level refinement when the rect provably fits in 2×2
texels at its actual alignment; the un-refined form is where Bevy #14042 went wrong (a bbox of
~29.36 × 30.06 px selected mip 4, the 2×2 footprint stopped covering the sphere, and visible clusters
were rejected — *geometry disappearing at certain distances only*). `boyko_render::hzb`'s
`msb(tx0 ^ tx1)` is alignment-aware by construction: it is the smallest level at which both ends of
the rect land in the same or adjacent texels, and it carries a committed coverage proptest
(`property_selected_texels_cover_the_rect`) plus a counterfactual test showing that clamping DOWN a
level stops covering. **The shader mirrors it exactly; no refinement step is added, and none is
needed.**

---

### D8 — The graph: ONE new pass (`vb_cull_late`), FOUR new declared accesses on the early cull, TWO on the late raster, and `vb_indirect_late`'s writer chain becomes four links

**`vb_indirect_late`'s chain** — obligation 1, stated precisely, because "the writer changes" is not
the same as "the writer is replaced":

```
vb_indirect_late_upload   TRANSFER  TRANSFER_WRITE   (host fill, all five words, instanceCount = 0)
vb_batch_cull             COMPUTE   SHADER_WRITE     (the early phase writes n_defer)   [NEW]
vb_cull_late              COMPUTE   SHADER_READ|WRITE(reads n_defer, writes n_keep)     [NEW]
vb_raster_late            DRAW_INDIRECT  INDIRECT_COMMAND_READ
```

The host fill **stays** — it carries `index_count`/`first_index`/`vertex_offset`/`first_instance`,
which no GPU pass produces, exactly as `vb_indirect_upload` does for the early array. What changes is
the **last** declared writer before the fetch, hence the `vb_raster_late` read's `src_stage`/
`src_access`, hence G-P3-F's S-rows. `graph.rs`'s P2-8 provenance guard stays satisfied throughout —
`vb_indirect_late` remains a bare `add_buffer` with an in-graph producer.

**New accesses on `vb_batch_cull`, ALL gated on `occlusion_split`** so an unsplit frame's declared
set — and therefore every existing golden's barrier stream — is **bit-unchanged**:

| resource | stage | access | layout |
|---|---|---|---|
| `vb_cull_uniform` | `TRANSFER` then `COMPUTE_SHADER` | `TRANSFER_WRITE` then `SHADER_READ` | — |
| `hzb_pyramid` | `COMPUTE_SHADER` | `SHADER_READ` | `GENERAL`, mips `[0, levels)` |
| `vb_late_visible` | `COMPUTE_SHADER` | `SHADER_WRITE` | — |
| `vb_indirect_late` | `COMPUTE_SHADER` | `SHADER_WRITE` | — |

The uniform is gated too: an unsplit frame's cull reads only the planes, which are pushed. One
predicate, both sites, and it is `scene.path_vb_occlusion_split()` verbatim.

**`vb_cull_late`**, declared immediately after the last `hzb_build_*` and before `vb_raster_late`:

| resource | stage | access |
|---|---|---|
| `vb_batch_desc` | `COMPUTE_SHADER` | `SHADER_READ` |
| `vb_instance_ring` | `COMPUTE_SHADER` | `SHADER_READ` |
| `vb_cull_uniform` | `COMPUTE_SHADER` | `SHADER_READ` |
| `hzb_pyramid` | `COMPUTE_SHADER` | `SHADER_READ`, `GENERAL`, mips `[0, levels)` |
| `vb_late_visible` | `COMPUTE_SHADER` | `SHADER_READ \| SHADER_WRITE` |
| `vb_indirect_late` | `COMPUTE_SHADER` | `SHADER_READ \| SHADER_WRITE` |

`vb_mesh_bounds` is not a tracked ResId in this graph (a boot-fixed host-coherent table), exactly as
today. It declares **no** access to `vb_indirect`, `vb_cull_visible` or `vb_cull_count` — the late
phase touches none of them, and over-declaring here would put a spurious WAR on `vb_indirect` *after*
`vb_raster` fetched it, which reads to the next maintainer as "the late cull rewrites the early
records". The asymmetry is safe in the direction `graph_bridge.rs:3994-3998` already names, and this
is the other direction, so it is stated rather than assumed: **the late phase's write set is
literally the `phase == 1` arm of one shader, and the shader's phase branch is the single source.**

**New accesses on `vb_raster_late`** — obligation 2:

| resource | stage | access |
|---|---|---|
| `vb_instance_ring` | `VERTEX_SHADER` | `SHADER_READ` |
| `vb_late_visible` | `VERTEX_SHADER` | `SHADER_READ` |

**The declared pass order on an armed-split frame**, with three new declare-order asserts joining the
five piece 2 added:

```
vb_indirect_upload → vb_indirect_late_upload → vb_batch_cull → vb_raster
  → hzb_poison? → hzb_build_* → hzb_dump_depth_early? → vb_cull_late → vb_raster_late
  → classify? → lit → … → hzb_dump?
```

```rust
debug_assert_eq!(vb_cull_late.is_some(), scene.path_vb_occlusion_split());
debug_assert!(vb_cull_late.is_none_or(|c| hzb_build.iter().flatten().all(|b| b.index() < c.index())),
    "invariant: the late cull reads the pyramid this frame's build wrote");
debug_assert!(vb_cull_late.is_none_or(|c| vb_raster_late.is_some_and(|l| c.index() < l.index())),
    "invariant: the late cull writes the count the late raster fetches");
```

---

### D9 — Arming: `path_vb_occlusion_split()` gains the pyramid as a conjunct, and the two tripwires are deleted with one replaced

```
path_vb_occlusion_split() = path_is_vb()
                         && resolved_render_path.mesh_leg
                         && vb_occlusion_instances > 0
                         && hzb.is_some()                     // NEW
```

`scene_types.rs:3540-3544`. **Why now and not in piece 2**: piece 2's split was inert, so a pyramid
was not needed and adding the conjunct would have made every piece-2 gate reachable only under HZB.
Piece 3's split *is* the occlusion test, and a late scope with no pyramid is a second scope that can
decide nothing. The predicate stays a DERIVED expression, never a stored bool
(`HzbConfig::enabled()`'s discipline).

⚠️ **Consequence that must land in the same commit.** `[vb_occ_split.env]`
(`goldens/PINS.toml:390-395`) sets `BOYKO_VG_OCC=1` and **not** `BOYKO_VG_HZB`. Adding the conjunct
would silently disarm the split on the pin whose whole purpose is to arm it, and G2's `scopes == 2`
would red for a reason unrelated to any defect. ⇒ `crates/boyko_app/tests/vb_mesh.rs` makes
`BOYKO_VG_OCC=1` **imply** the `HzbMode::Build` arm (the OCC branch at `:64`, the HZB branch at
`:240`), and `[vb_occ_split.env]` gains `BOYKO_VG_HZB = "1"` so the configuration is legible from the
pin file rather than only from the fixture.

**Obligation 3 — the two tripwires:**

- `vb.rs:1802-1806` (the indirection bit must be CLEAR): **deleted.** The late push now sets
  `VB_RASTER_FLAG_VISIBLE_INDIRECTION` (`vb.rs:68`), because the late list is an indirection list.
  The assert is replaced by the early scope's own shape (`vb.rs:1596-1602`): bit 1 never without
  bit 0.
- `vb.rs:1220-1230` (`instance_count == 0` on every host-filled record): **deleted as a tripwire and
  REPLACED by the same expression under a different, permanently true invariant** —

  ```rust
  debug_assert!(records.iter().all(|r| r.instance_count == 0),
      "invariant: the HOST seeds every late record with instanceCount = 0, so a frame in which \
       the late cull did not run draws nothing. The late cull is the ONLY producer of a nonzero \
       value in this array.");
  ```

  This is not a cosmetic edit. It is the safety property that makes a missing `vb_cull_late`
  dispatch a *blank* late scope rather than a scope drawing `n_defer` untested instances.

⚠️ **And the vacuity it creates, named rather than discovered.** `VbRecordProbe::late_instances`
(`vb.rs:115`) sums the HOST-written records (`vb.rs:1237-1240`) — which stay `0` forever. G2's
`late_instances == 0` clause therefore **stays green and stops meaning anything**. It is renamed
`late_seed_instances` with a new message, and **the GPU's real late count comes from the readback,
never from the probe** (Gates). Renaming rather than deleting keeps the host-seed property gated.

---

### D10 — The dump carries BOTH depths; the EARLY depth copy becomes its own declared pass between the scopes

Obligation 5. Piece 2 recorded the hazard: the pyramid is built from the depth as of the *early*
scope, while `BOYKO_HZB_DUMP` copies `vb_depth` at frame end (`vb.rs:3509-3541`). In piece 2 those
were equal because the late scope drew nothing — "**which also means G8 cannot see the ordering**".
The moment piece 3 arms the late draws they diverge, and G5 would compare the pyramid against a depth
it was not built from.

**"Move the copy" is the weaker of the two options and is rejected.** Moving it gives one depth and
therefore only a one-sided claim: *the pyramid equals the oracle over this depth*. Dumping both gives
a **two-sided** claim, which is what this campaign requires of a gate that is supposed to prove an
ordering:

- `build_pyramid(depth_early) == pyramid`, bit-exact — the pyramid WAS built from the early depth;
- and where `depth_early ≠ depth_final`, `build_pyramid(depth_final) ≠ pyramid` — it was NOT built
  from the final one.

The second clause is what a one-depth dump structurally cannot state.

**Shape.** `HzbDumpLayout` grows a second depth region. The header becomes
`[magic, source_w, source_h, levels, flags]` + the per-level extents
(`HZB_DUMP_HEADER_WORDS: 38 → 39`, 152 → 156 bytes), and **`HZB_DUMP_MAGIC` is bumped**
(`scene_types.rs:1415`) so a stale dump file cannot be silently decoded against the new offsets.
`flags` bit 0 = "the early-depth region is live", set iff the frame armed the split. The staging
grows by one depth image (1 MiB at 512²), on dump frames only.

- **New pass `hzb_dump_depth_early`**, declared iff `occlusion_split && hzb_dump_armed`, sited
  between the last `hzb_build_*` and `vb_raster_late`. It declares `vb_depth` at
  `(TRANSFER, TRANSFER_READ, TRANSFER_SRC_OPTIMAL, DEPTH aspect)`, so the graph derives the round trip
  out of `hzb_build_0`'s `SHADER_READ_ONLY_OPTIMAL` and back into `vb_raster_late`'s
  `DEPTH_ATTACHMENT_OPTIMAL`. Both preserving; neither may become a first touch.
- The existing end-of-frame `hzb_dump` pass is unchanged in position and gains nothing but a
  destination offset: on a split frame its depth copy lands in the **final** region.
- On an unsplit dump frame only the final region is written; `flags` bit 0 is clear and the host must
  not read the early region — which still carries the `0xFF`/NaN prefill the existing gate already
  treats as "the copy never ran".

---

### D11 — The occlusion test is a HAND-AUTHORED, statement-for-statement mirror of `boyko_render::hzb`, `precise`-guarded, gated by a GPU-vs-oracle VERDICT differential

**The oracle already exists and is complete.** `hzb.rs:836-859` is `occlusion_verdict`; the steps and
their exact short-circuit ORDER are:

1. `!(min <= max)` → `Keep(UnknownBounds)` — spelled that way so a NaN also lands here (`:701-709`);
2. per corner in index order 0..8 (`bit0→x, bit1→y, bit2→z`, `0` picks min): non-finite clip →
   `Keep(NonFinite)`; **then** `cw <= 0.0` → `Keep(BehindEye)`; then divide; then non-finite
   post-divide → `Keep(NonFinite)`; accumulate window min/max and `depth_near = max(z_ndc)` seeded
   `-INFINITY` (`:720-755`). **First offending corner returns — this is a short-circuit, not a fold.**
3. `floor` on BOTH ends, clamp to `[0, source−1]`, empty → `Keep(EmptyRect)` (`:764-773`);
4. `level = max(msb(tx0^tx1), msb(ty0^ty1))`; `level >= levels()` → `Keep(LevelUnavailable)`,
   **never clamped down** (`:790-807`); 4 texels via `containing_texel`;
5. `occ = conservative_min`-fold over the 4 texels seeded `+INFINITY` (`:817-824`);
6. **`REJECT iff rect.depth_near < occ`, strictly** (`:855`) — equality KEEPS, and `:854` states why:
   the soundness chain `occ ≤ D[p] ≤ d_i(p) ≤ depth_near` admits equality, so `<=` would delete a
   visible instance.

**Step 2 is where this engine is already ahead of the field, and the shader must not lose it.** The
research could not find a `w <= 0` rejection in Bevy's 8-corner projection; without one, a corner
behind the eye flips sign under the perspective divide and the min/max rect can invert or collapse,
selecting a fine mip over the wrong place — a silent **over-cull**. `boyko_render::hzb` returns
`Keep(BehindEye)` on the first such corner, and it has a committed test for the exactly-`w == 0`
case. niagara reaches the same outcome differently, by bailing out of `projectSphere` when the sphere
crosses the near plane, on the stated ground that "if the object intersects the camera plane, the
odds of it being culled are nil".

**The missing half is the world AABB**, which no single Rust function produces: the shader must
compose `arvo_transform`'s fold (`csm_caster.rs:291-300`, mirrored in-shader at
`vb_batch_cull.comp.hlsl:450-457`) with steps 1-6. The shader already computes that world AABB for
the frustum test — **the occlusion test reuses it, at zero additional cost.**

**Why hand-authored and not `boyko_shaderdsl`.** The eDSL owns numeric LEAVES so a nontrivial float
body is bit-exact across the boundary, and this body qualifies on its face. It is rejected here for
one reason and the reason is churn, not principle: `hzb.rs`'s projection chain is a shipped, heavily
pinned oracle with **26 committed tests** including the exact-integer-edge pin, the
`depth_near`-is-max-of-quotients pin and the strictness pin at `depth_near == occ`. Rewriting it as a
generic body over `f32`/`Emit` moves the file every existing HZB gate depends on, in the same step
that arms a decision. **Recorded as an Open Question**, because if the differential below ever shows
a disagreement that is not attributable to a stated cause, converting the leaf is the correct fix
rather than a tolerance.

**Why bit-exactness is not required but near-exactness is, in ONE direction.** The verdict is a
boolean; a 1-ULP difference in `depth_near` changes it only within one ULP of `depth_near == occ` —
and there the oracle KEEPS. A shader whose `depth_near` comes out one ULP LOW would REJECT there.
That is the geometry-deleting direction. Mitigations, in order:

- `precise` on the projection locals (`inv_w`, `z_ndc`, `x_win`, `y_win`), which forbids FMA
  contraction of `(cx * inv_w + 1.0) * half_w` and reciprocal substitution for `1.0 / cw`. This is
  the "FMA → `precise` MINIMALLY" practice the engine already records.
- The clamps use `max`/`min`, which lower to `NMax`/`NMin` — under which **a NaN operand is silently
  discarded rather than propagated**, the engine's recorded incident. They are reached only after
  both non-finite checks have returned, so no NaN can arrive. **Stated, because "unreachable" is the
  claim, not "handled".**
- The differential gate carries a **constructed boundary corpus** where `depth_near == occ` exactly,
  which is where the only dangerous disagreement can live. A random corpus never lands there.

---

## Data structures

```rust
// ── crates/boyko_rhi_vulkan/src/present/scene_types.rs (EDIT) ────────────────────────
/// The cull's non-push inputs. 96 B, 16-byte aligned, per-FIF, DEVICE_LOCAL, written by
/// `vkCmdUpdateBuffer` inside the `vb_batch_cull` pass and read by both phases.
/// It exists because `VB_BATCH_CULL_PUSH_BYTES` is 104 and the shared compute push range is
/// const-asserted <= 128 (`rhi_impl/mod.rs:227-232`): a float4x4 does not fit.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VbCullUniform {
    /// MATH-ROW form, `clip = view_proj * world` — verbatim what `hzb::project_aabb` takes
    /// (`hzb.rs:687-692`). The host performs the ONE byte inversion out of the column-major
    /// push buffer, at one site, so the shader's `dot(row_r, world4)` and the oracle's
    /// `pv[r] . world` are the same four products in the same order.
    pub view_proj_rows: [[f32; 4]; 4],   // 0..64
    /// The pyramid's SOURCE extent — `present_extent`, the same value the build pushed as
    /// `src_extent`, NOT the client extent (under armed SSAA they differ by 2x).
    pub src_extent: [u32; 2],            // 64..72
    /// Level-0 extent = `prev_pow2` per axis. PUSHED, never re-derived in the shader
    /// (P1-3's rule: a base-map disagreement must be a SHADER bug, never a math one).
    pub base_extent: [u32; 2],           // 72..80
    /// `HzbPlan::levels`. `level >= levels` is `Keep(LevelUnavailable)`, never a clamp down.
    pub levels: u32,                     // 80..84
    pub _pad: [u32; 3],                  // 84..96
}
const _: () = assert!(core::mem::size_of::<VbCullUniform>() == 96);

/// 104 -> 112. Still inside `VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE`; the const-assert at
/// `rhi_impl/mod.rs:227-232` is the mechanical gate.
#[repr(C)]
pub struct VbBatchCullPush {
    pub planes: [[f32; 4]; 6],   // 0..96   unchanged
    pub batch_count: u32,        // 96      unchanged
    pub visible_cap: u32,        // 100     unchanged
    pub phase: u32,              // 104     0 = early, 1 = late   NEW
    pub occ_flags: u32,          // 108     NEW — see below
}

pub const VB_CULL_OCC_ARMED: u32      = 1 << 0;  // the pyramid exists and the split is armed
pub const VB_CULL_OCC_FORCE_LATE: u32 = 1 << 1;  // the CONTROL: early rejects every marked instance
pub const VB_CULL_OCC_FORCE_KEEP: u32 = 1 << 2;  // the NULL CONTROL: early rejects nothing

pub struct GBufferScene<'a> {
    // ... existing ...
    /// The per-FIF early-reject / late-survivor list. `INSTANCE_CAPACITY` u32s, region-addressed
    /// by the SAME `VbBatchDesc.base_instance` / `.instance_count` that address
    /// `vb_visible_instance` — see INVARIANT VG-P3-LATE-REGION. Minted unconditionally on every
    /// VB boot (the `vb_visible_instance` rule); `.expect()`ed under
    /// `path_vb_occlusion_split()`, never a conjunct of it.
    pub vb_late_visible: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// The per-FIF `VbCullUniform`. Same minting rule.
    pub vb_cull_uniform: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// `occ_flags`, folded ONCE on the host so declare, record and shader read one number.
    pub vb_occ_flags: u32,
}

// ── crates/boyko_app/src/gpu_scene/mod.rs (EDIT) ─────────────────────────────────────
const VB_LATE_VISIBLE_ELEMS: usize = INSTANCE_CAPACITY;
const _: () = assert!(
    VB_LATE_VISIBLE_ELEMS == VB_VISIBLE_INSTANCE_ELEMS,
    "the late candidate/survivor list must hold every index the early survivor list can: both are \
     addressed by the SAME VbBatchDesc region and bounded by the SAME \
     vb_cull_batch_count_visible_clamp, which is computed from ONE element count."
);
pub(crate) const VB_LATE_VISIBLE_BYTES: u64 = (VB_LATE_VISIBLE_ELEMS as u64) * 4;   // 4 KiB
// Usage = STORAGE | TRANSFER_DST | TRANSFER_SRC (the readback probe copies it).
// ⚠️ `gpu_scene/mod.rs:264-288`'s R2d-6 equality appears in this diff as CONTEXT ONLY. If it or
// `VB_VISIBLE_INSTANCE_ELEMS` moves by one character, piece 3 has re-created the R2d-6 collision.

// ── crates/boyko_rhi_vulkan/shaders/vb_batch_cull.comp.hlsl (EDIT) ───────────────────
// The three HLSL mirrors of VbInstanceRow still spell offsets 52..64 as `uint3 _pad`
// (`vb_batch_cull.comp.hlsl:294-300`, `vb_raster.vs.hlsl:151-157`, `vb_geom_fetch.hlsli:57-63`);
// piece 2 corrected only the COMMENTS. Piece 3 renames the field in the ONE module that reads it:
struct VbInstanceRow {
    float4 r0; float4 r1; float4 r2;
    uint   mesh_id;      // 48
    uint   flags;        // 52  bit 0 = VB_INST_FLAG_OCCLUSION_CULLING  (was _pad.x)
    uint2  _pad;         // 56..64
};
// The other two keep `_pad` and their comments now say so. Renaming there would touch an .hlsli
// four modules include, re-DXC four .spv and move four census pins, to buy a name in a file that
// does not read the word.

// ── new bindings on vb_cull_layout: 7 -> 10 ──────────────────────────────────────────
// @7  StorageBuffer  RWStructuredBuffer<uint>  VbLateVisible      (RW)
// @8  StorageBuffer  StructuredBuffer<VbCullUniform> VbCullUni    (read)
// @9  SampledImage   Texture2D<float>          gHzbPyramid        (read, GENERAL, mip-complete)
// @10 StorageBuffer  RWByteAddressBuffer       VbIndirectLate     (RW)
// (@9's view is `HzbTargets::pyramid`'s own [0, levels) texture view — P1 gave the image SAMPLED
//  usage for exactly this. On an `HzbMode::Off` boot it binds `hzb_null` — see below.)

// ── crates/boyko_rhi_vulkan/src/present/targets.rs (EDIT) ────────────────────────────
pub(crate) struct GBufferTargets {
    // ...
    /// A 1x1 R32_SFLOAT single-mip image with a SAMPLED view, minted UNCONDITIONALLY, bound at
    /// `vb_cull_layout` @9 on every boot where `hzb` is `None`. The module DECLARES the pyramid
    /// binding in both phases (D5: one module, one pipeline), so a valid image must sit there —
    /// and the load is gated off by `VB_CULL_OCC_ARMED`, so it is never dereferenced.
    /// The `HzbTargets::sets` padding rule, lifted to an image ("a real view of a real mip that
    /// no store reaches"); `descriptorBindingPartiallyBound` is NOT relied upon.
    pub(crate) hzb_null: VulkanTexture,
    /// `vb_set0` with ONE entry changed: @11 binds `vb_late_visible` instead of
    /// `vb_visible_instance`. Bound by the LATE raster scope only.
    pub(crate) vb_set0_late: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
}
```

---

## Public API

```rust
// boyko_rhi_vulkan
pub struct VbCullUniform { /* pub fields, above */ }
pub const VB_CULL_OCC_ARMED: u32;
pub const VB_CULL_OCC_FORCE_LATE: u32;
pub const VB_CULL_OCC_FORCE_KEEP: u32;

impl GBufferScene<'_> {
    /// UNCHANGED SIGNATURE; gains the `hzb.is_some()` conjunct (D9).
    pub fn path_vb_occlusion_split(&self) -> bool;
}

/// The recorder-authored probe. `late_instances` -> `late_seed_instances` (D9): the field sums the
/// HOST seed, which is permanently 0. The GPU's real late count comes from `VbCullReadback`.
pub struct VbRecordProbe {
    pub scopes: u32,
    pub late_draws: u32,
    pub late_seed_instances: u32,
    /// Late cull dispatches recorded this frame: 0 or 1. Counted AT the dispatch, never derived
    /// from the arming predicate — the difference between a gate and a tautology.
    pub late_cull_dispatches: u32,
}

// boyko_app — the readback probe's decoded payload gains two regions.
pub struct VbCullReadback {
    // ... existing: count, visible[], indirect[], visible_instance[] ...
    /// `vb_late_visible` in full, and `vb_indirect_late`'s records. Together with the existing
    /// early regions these give a host test the COMPLETE per-batch partition the GPU computed.
    pub late_visible: Vec<u32>,
    pub indirect_late: Vec<VkDrawIndexedIndirectCommand>,
}
```

No `dyn`, no allocation in the frame loop, no new pipeline **layout family** beyond the widened
`vb_cull_layout`, no new sampler, no device-feature change.

---

## Algorithms for critical paths

### A1 — `occlusion_reject(world_min, world_max)`, the shared leaf (both phases)

| # | step | cost | notes |
|---|---|---|---|
| 1 | `any(mn > mx)` → KEEP | 3 cmp | catches the `MESH_BOUNDS_UNKNOWN_COORD` (`1e30`) sentinel and any NaN; the same short-circuit the frustum arm already has at `vb_batch_cull.comp.hlsl:449` |
| 2 | 8 corners × (4 `dot4` + finite checks + 1 divide + 2 mad) | ~200 flop | **short-circuits on the first offending corner**, in the oracle's order: finite-clip, then `w <= 0`, then divide, then finite-post-divide |
| 3 | `floor` both ends, clamp to `[0, src−1]`, empty → KEEP | 4 flop | `floor` on the UPPER end too — `ceil(hi)−1` drops an exactly-integer edge column and produces a FALSE REJECT (`hzb.rs:757-763`) |
| 4 | `texel_of` ×4 (u32, D7 lemma 1), `msb(x^y)` with the zero guard, `level >= levels` → KEEP | ~10 int | |
| 5 | 4 × `gHzbPyramid.Load(int3(tx, ty, level))`, folded with `conservative_min` | 4 loads | the 4 texels are 2×2 adjacent at `level`; on a scene with locality they share a cache line |
| 6 | `depth_near < occ` → REJECT | 1 cmp | STRICT; equality KEEPS |

- **Complexity:** O(1) per instance, O(instances) per batch lane.
- **Cache:** the world AABB is already in registers from the frustum test (reused, not recomputed).
  The 4 pyramid loads are the only new memory traffic and they are 2×2-adjacent at a level chosen so
  the rect spans ≤ 2 texels per axis — the tightest footprint the selector admits.
- **Branching:** six early-out returns, all of which KEEP. Every one is the conservative direction,
  so a mispredict costs cycles and never geometry. The hot path (a fully-visible on-screen box) takes
  none of them.
- **SIMD:** the 8-corner loop is 8 independent `dot4`s over one matrix — the shape a compiler
  vectorises without help. It is NOT unrolled by hand; `[unroll]` is left off, and the `.spv` census
  records the loop's shape so a future change to it is visible (the P1-3 lesson: an attribute's
  effect is a measurement, not an inference).

### A2 — the EARLY phase, per batch lane

```
for j in 0 .. instance_count:
    g   = base_instance + j
    row = gVbInstances[g];  b = gMeshBounds[row.mesh_id]
    (wc, wh) = arvo_fold(row, b)                       // UNCHANGED, lines 450-457
    if aabb_outside_frustum(wc-wh, wc+wh): continue    // UNCHANGED — neither list
    defer = (occ_flags & ARMED)
         && (row.flags & OCCLUSION_CULLING)
         && !(occ_flags & FORCE_KEEP)
         && ( (occ_flags & FORCE_LATE) || occlusion_reject(wc-wh, wc+wh) )
    if defer: VbLateVisible[base_instance + n_defer] = g;  n_defer += 1
    else:     VbVisibleInstance[base_instance + k]   = g;  k       += 1
VbIndirect    .Store(i*20 + 4, visible ? k       : 0)   // UNCHANGED shape
VbIndirectLate.Store(i*20 + 4, visible ? n_defer : 0)   // NEW
// the batch-level InterlockedAdd on VbCullCount/VbCullVisible: UNCHANGED, early arm only.
```

- `k + n_defer ≤ frustum survivors ≤ instance_count`, so both writes stay inside the batch's own
  region. **The budget piece 2's D5 proved, used.**
- No atomic, no `groupshared`, no barrier — region disjointness is host-established (C9), unchanged.
- **Branchless where it matters:** `defer` is a chain of `&&` over uniform-ish flags plus one data
  predicate; the two appends differ only in the base pointer and the cursor.

### A3 — the LATE phase, per batch lane

```
n_defer = VbIndirectLate.Load(i*20 + 4)
keep = 0
for j in 0 .. n_defer:
    g   = VbLateVisible[base_instance + j]              // sequential, ascending
    row = gVbInstances[g];  b = gMeshBounds[row.mesh_id]
    (wc, wh) = arvo_fold(row, b)
    if !occlusion_reject(wc-wh, wc+wh):
        VbLateVisible[base_instance + keep] = g;  keep += 1
VbIndirectLate.Store(i*20 + 4, keep)
```

> **LEMMA (in-place compaction is race-free with no scratch).** At step `j` the lane reads index
> `base+j` and writes index `base+keep` with `keep ≤ j`. Every later read is at `base+j'` with
> `j' > j ≥ keep`. Therefore no write can clobber a slot a later read will consume. One lane per
> batch, so there is no cross-lane question at all.

- The candidate list is written in ascending `j` order by A2, so `g` is ascending and the
  `gVbInstances[g]` gather is a monotone strided walk, not a random one.
- No frustum test is repeated: a candidate passed it in A2 by construction.
- **The world AABB is recomputed rather than stored.** Storing it would be 24 B × 1024 = 24 KiB of
  extra traffic to save ~20 flops per candidate. Rejected on the traffic.

### A4 — the late raster scope (piece 2's, with three words changed)

`begin_rendering(LOAD/LOAD/STORE)` → bind pipeline → **bind `vb_set0_late[fi]`** → pass-wide push →
per batch { push `[base_instance, base_flags | VISIBLE_INDIRECTION]`, bind VB/IB,
`cmd_draw_indexed_indirect(late[fi], i*20, 1, 20)` } → `end_rendering`. O(`batch_count`).

The VS expression, the base and the `vb_id` export are **unchanged**: `vb_late_visible` stores GLOBAL
instance indices exactly as `vb_visible_instance` does, so R2d-EXPORT-IS-GLOBAL holds verbatim and
every downstream consumer of `vb_id` sees the same encoding it sees for an early-drawn instance.

---

## Multithreading model

**Host:** single-threaded with respect to everything piece 3 adds. `record_vb(&self, …)` means every
buffer write in the recorder is a *command*, never a host mutation; the uniform's contents are built
in a stack local and handed to `vkCmdUpdateBuffer`. `VbRecordProbe` remains the one host mutation and
remains an explicit `&mut` parameter. No new `Resource`, no atomics, no `Send`/`Sync` change.

**ECS:** unchanged. `Option<&OcclusionCulling>` is already in both gather variants and already
declares a shared read.

**Device — the only place with a real concurrency argument:**

| pair | hazard | resolved by |
|---|---|---|
| early cull write `vb_visible_instance` → early raster VS read | RAW | declared today (`graph_bridge.rs:3999-4005`), unchanged |
| early cull write `vb_late_visible` → late cull read | RAW | both declared on their passes; the graph derives COMPUTE→COMPUTE |
| early cull write `vb_indirect_late` → late cull RW → late raster fetch | RAW, RAW | the four-link chain in D8 |
| early raster depth write → `hzb_build_0` depth read | RAW + layout | derived today (piece 2's D6 move) |
| `hzb_build_*` pyramid write → late cull pyramid read | RAW | new; both declared, both `GENERAL`, no transition |
| **frame N `hzb_build` write → frame N+1 early cull read** | **cross-frame RAW** | **D2's seed** |
| **frame N late cull pyramid read → frame N+1 pyramid write** | **cross-frame WAR** | subsumed by frame N+1's intra-frame WAR barrier, because a barrier recorded outside a render pass orders against all commands earlier in SUBMISSION ORDER and **there is one queue** (C5) |
| late cull writes `vb_late_visible` while the EARLY raster is still reading `vb_visible_instance` | none — different buffers | this is the hazard the two-ended alternative would have had to barrier |

**Within one lane** there is no synchronisation at all: the compaction lemma (A3) is a
single-threaded argument, and the region disjointness (C9 / VG-P3-LATE-REGION) is host-established.
**No atomic is added by this piece.** The module's single `OpAtomicIAdd` (the early phase's
batch-level append) is unchanged and its census pin stays at 1.

---

## Integration

**26 files touched, 3 new.** Piece 2's list was short by five and cost a round; this one is
enumerated to the test-fixture level.

### `boyko_rhi_vulkan`

| file | change | step |
|---|---|---|
| `shaders/vb_batch_cull.comp.hlsl` | the `flags` rename; the `VbCullUniform` / pyramid / late-list / late-record bindings; `phase` + `occ_flags`; the occlusion leaf (A1); the two-way partition (A2); the late phase (A3) | P3-3 |
| `shaders/vb_batch_cull.comp.spv` | re-DXC under the frozen recipe; **no new `-D` variant, no `SHADER-VARIANT-MANIFEST.md` row** | P3-3 |
| `src/compute.rs` | `VB_BATCH_CULL_PUSH_BYTES` 104 → 112; `VB_CULL_UNIFORM_BYTES`; the binding-count constant | P3-3 |
| `src/present/scene_types.rs` | `VbCullUniform`; `VbBatchCullPush` +2 fields + its size assert; the three `VB_CULL_OCC_*` consts; `GBufferScene` +3 fields; `path_vb_occlusion_split()` +conjunct; `HzbDumpLayout` + the early-depth region, `HZB_DUMP_HEADER_WORDS` 38 → 39, `HZB_DUMP_MAGIC` bump; `VbCullReadbackLayout` +2 regions | P3-1/2/5/6 |
| `src/present/graph_bridge.rs` | `vb_late_visible` + `vb_cull_uniform` ResIds appended LAST in both `cfg` arms; `VB_BUFFER_COUNT` 14→16 / 15→17 + the sink assert; the sink arrays (both arms); the pyramid **seed**; the early cull's four new gated accesses; the `vb_cull_late` pass; `hzb_dump_depth_early`; `vb_raster_late`'s two new accesses; `VbPassPlan` +2; the three new declare-order asserts; **the "WHAT IS DELIBERATELY NOT DECLARED" comment at `:4049-4056` is now false and must be rewritten** | P3-0/2/4/5 |
| `src/present/passes/vb.rs` | the uniform fill + push widening; the late cull dispatch; the two tripwire deletions (one replaced); the `vb_set0_late` bind; the indirection bit; `VbRecordProbe` rename + `late_cull_dispatches`; the dump's two depth copies; the readback's two new regions | P3-2/4/5/6 |
| `src/present/targets.rs` | `hzb_null` (1×1, unconditional); `vb_cull_set` 7 → 10 entries with the null fallback; `vb_set0_late`; **the `HzbTargets` build must precede the `vb_cull_set` build** (today: sets at `:4630-4667`, HZB at `:7722+`), which adds the HZB to that block's error-path teardown ladder; the pyramid's **boot clear to 0.0** in `HzbTargets::build` (`:1292-1330`) | P3-0/1 |
| `tests/vb_barrier_stream_baseline.rs` | G-P3-F: the four U-rows re-pinned for the seed change (at P3-0, alone); four new S-rows | P3-0, P3-6 |
| `tests/vb_batch_cull_spv_sync.rs` | the census: binding set 7 → 10, push 104 → 112, the new opcode counts, `OpAtomicIAdd == 1` **unchanged**, and a new pin that no `OpTypeSampler` / `OpImageSampleExplicitLod` exists (D7, artifact-level) | P3-3 |
| `tests/window_present_gbuffer.rs` | the FOUR exhaustive `GBufferScene` literals (piece 2's anchors `:2265`, `:3366`, `:8390`, `:9904` — **re-grep**) gain three fields each | P3-1 |
| `tests/vb_indirect_barrier_chain.rs` | verify: the chain gains a link on split frames | P3-6 |
| `tests/framegraph_gbuffer_equiv.rs` | verify only — the VB path has a PRIVATE ResId space and this file exercises the deferred declarator | — |

### `boyko_app`

| file | change | step |
|---|---|---|
| `src/gpu_scene/mod.rs` | `vb_late_visible` + `vb_cull_uniform` allocation, consts, drift asserts, `scene()` wiring, destroy; `vb_cull_layout` 7 → 10 entries; `vb_occ_flags` fold | P3-1 |
| `src/runner.rs` | build `VbCullUniform` from the same 64 push bytes the planes come from (the ONE byte inversion); the `BOYKO_VG_OCC_FORCE` knob → `occ_flags`; the readback decode | P3-2/6 |
| `src/hzb_dump.rs` | the two-depth layout + header version; the decode | P3-5 |
| `src/hzb_plan.rs` | verify — it already calls `HzbLayout::new`, and `base_extent`/`levels` come from there | P3-2 |
| `tests/vb_mesh.rs` | `BOYKO_VG_OCC=1` implies the `HzbMode::Build` arm (D9) | P3-4 |
| `tests/hzb_engine_pyramid_gate.rs` | G-P3-E: the driver gains the two-depth clauses; G8's unmarked leg unchanged | P3-5 |
| `tests/vb_occ_split_gate.rs` | the probe rename; `late_cull_dispatches`; the header's "what it cannot claim" list is rewritten | P3-4/6 |
| `tests/vg_density_census.rs` | `VB_PINS` gains the new pin names, in the SAME commit as the pins | P3-6 |
| `tests/vb_inst_cull_scene/mod.rs` | the shared readback fixture decodes the two new regions | P3-6 |
| `tests/vb_occ_hidden.rs` | **NEW** — the occlusion fixture + its gates (G-P3-A / B / C) | P3-6 |
| `tests/vg_occ_verdict_census.rs` | **NEW** — the CPU census, the `vg_cull_granularity_census.rs` shape | P3-6 |
| `tests/hzb_verdict_oracle_gate.rs` | **NEW** — the GPU-vs-oracle verdict differential (G-P3-D) | P3-3 |

### `boyko_render`, `goldens`, `docs`

| file | change |
|---|---|
| `src/hzb.rs` | **no functional change.** If the census needs `msb`, it is exported; nothing else moves. |
| `src/occlusion_marker.rs` | doc only — the marker's meaning goes from "may be rejected" to "is tested". |
| `goldens/PINS.toml` | `[vb_occ_split.env]` gains `BOYKO_VG_HZB`; three new pins (`vb_occ_hidden`, `vb_occ_hidden_off`, `vb_occ_force_late`) with the pre-filled-and-verified pattern |
| `docs/SHADER-VARIANT-MANIFEST.md` | **no row** — stated so its absence is a decision |
| `docs/OPEN-QUESTIONS.md` | piece 3 status |

**No change** to: `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl`, `vb_geom_fetch.hlsli`, `hzb_build.comp.hlsl`
or any `.spv` other than the cull's; `device.rs`'s fn table or feature chain;
`gpu_scene/mod.rs:264-288` (the R2d-6 equality — **context only**); `boyko_render::hzb`'s algorithms.

---

## Implementation plan — each step builds green and commits alone

- **P3-0 — the boot clear and the seed, alone.** `HzbTargets::build` clears the pyramid to `0.0`;
  `add_image_mipped`'s seed becomes `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)`.
  **Nothing reads the pyramid yet**, so this step's whole content is a barrier-stream change on
  `vb_mesh_hzb`, and the G-P3-F U-rows are re-pinned here, in isolation, with the reason.
  Gates: 25/25 goldens + `vb_occ_split`, U-rows re-pinned, G8/G5 green (the poison still dominates
  the clear).
  *Why first:* it is the one change that moves an existing stream, and it must not be entangled with
  a change that moves pixels.

- **P3-1 — the two buffers and the widened layout, read by nothing.** `vb_late_visible`,
  `vb_cull_uniform`, `hzb_null`, the const-asserts, the `vb_cull_layout` 7 → 10 widening, the
  `vb_cull_set` 10 entries, `vb_set0_late`, the `sync_gbuffer` reordering, the four
  `window_present_gbuffer.rs` literals. The SHADER still declares 7 bindings — legal in this
  direction and stated so at `vb.rs:1336-1339`: *"a WRITTEN descriptor a shader never loads from is
  never dereferenced, so the bound set may legally exceed what the module declares."*
  Gates: 25/25, validation armed-vs-unarmed message-for-message (the leg that sees an illegal image
  view or a wrong descriptor type), `vb_occ_split` green.

- **P3-2 — the graph and the recorder, still inert.** The ResIds, `VB_BUFFER_COUNT`, the sink arrays,
  the four gated accesses on `vb_batch_cull`, the `vb_cull_late` pass DECLARED AND RECORDED
  ATOMICALLY (declare/record parity forbids splitting them), the uniform fill, the push widening,
  `phase`/`occ_flags` pushed with `occ_flags = 0`. **The shader ignores both words**, so the frame is
  byte-identical. Gates: 25/25 + `vb_occ_split`, G-P3-F S-rows authored, the new declare asserts live.

- **P3-3 — the shader, and the differential that proves it before the engine ever runs it.** The
  occlusion leaf, both phases, the partition, the compaction. **Armed only by `occ_flags`, which the
  host still pushes as 0.** Landed together with `tests/hzb_verdict_oracle_gate.rs` (G-P3-D) — the
  gate that compares the SHADER's verdict against `boyko_render::hzb::occlusion_verdict` with no
  engine involved, the `hzb_build_oracle_gate.rs` shape. Gates: 25/25 (the frame cannot change —
  `occ_flags == 0`), G-P3-D green over three corpora, the `.spv` census re-pinned with its
  corruptions executed.
  *Why the differential lands with the shader and not after:* P1-7 proved a shader against an oracle
  before any engine frame depended on it, and that is what let a real disagreement (the ±0 tie) be
  characterised instead of chased through a renderer.

- **P3-4 — ARM IT.** `path_vb_occlusion_split()` gains the conjunct; `vb_mesh.rs` makes OCC imply
  HZB; `[vb_occ_split.env]` gains `BOYKO_VG_HZB`; the host sets `VB_CULL_OCC_ARMED`; the two
  tripwires are deleted (one replaced); the indirection bit is set; `vb_set0_late` is bound.
  **This is the first commit whose frame can change**, and it is deliberately the smallest one that
  can. Gates: the full G-P3 set.

- **P3-5 — the two-depth dump.** `HzbDumpLayout`, the magic bump, `hzb_dump_depth_early`, G-P3-E's
  two-sided clauses. Separate because it changes a file format and a gate, not behaviour.

- **P3-6 — the gates and the corpus.** `vb_occ_hidden`, the CPU census, the readback regions, the
  `VB_PINS` bump, and the corruption table — **including the controls that do NOT fire**, since
  reporting only the ones that fire is how a vacuous gate ships.

---

## Gates

> **"Can this gate fail?" is asked first.** This campaign has shipped and then caught: a validation
> switch that enabled no layer, a tile assertion whose two sides were the same expression, a barrier
> count pinned to the defective value, and a barrier-stream pin that is green on the production
> defect because it is a replica. Every gate below states what it CANNOT claim and carries a control
> that has been shown red.

### The central question: which gate separates "rejected" from "broke"?

**Neither an image nor a count. The CONJUNCTION, measured in one sitting:**

| | image byte-identical to the disarmed run | GPU-reported rejection count |
|---|---|---|
| the cull rejected correctly | **yes** | **> 0** |
| the cull rejected nothing (vacuous) | yes | 0 |
| the cull deleted visible geometry | **no** | > 0 |
| the cull drew everything twice | yes (reverse-Z, identical depth) | 0 |

The middle row is why the count must be a hard `assert`, never a report — `VG-R3-P1-PYRAMID-PLAN.md`
§13 is the worked example of a green comparison over a field of zeros. The third row is why the image
must be compared against a **disarmed run of the same scene**, not against a blessed constant.

**The preconditions of the identity claim, stated because they are real:** it holds for OPAQUE
geometry, on a frame where the late pass actually ran, with no temporal history dependence in the
captured frame. All three hold for the `vb_mesh`-family fixtures (opaque VB raster, dumps taken with
`BOYKO_SHADOW_DENOISE=none`), and the third is why G-P3-A pins the capture frame rather than
inheriting it.

⚠️ **THE FRAME-INDEX TRAP, and it would make G-P3-A vacuous by construction.** D2's boot clear makes
the pyramid all-zeros at birth, and D1's convergence argument says the early phase therefore rejects
**nothing on frame 1**. A fixture that captures the first rendered frame would compare a cull that
did nothing against a cull that was off, get byte-identity, and prove nothing. **Every occlusion
fixture must render at least three frames before capture and ASSERT the captured frame index**, and
the frame-1 capture is one of G-P3-B's red controls. This is the same shape as Bevy #17736, whose
defect "only manifests on the second and later frames of a stable scene — exactly the shape a golden
pin can miss".

### G-P3-A — the ARMED image equals the DISARMED image, on a scene with guaranteed occlusion

**`vb_occ_hidden`** (new fixture): one large near occluder covering the framebuffer's centre, `M ≥ 8`
small spheres wholly behind it, and `≥ 2` spheres outside its silhouette. Two pins from one binary:
`vb_occ_hidden` (`BOYKO_VG_OCC=1`) and `vb_occ_hidden_off` (unarmed). Their `sha256_software` and
`sha256_hwrt` must be the **same literals**, guarded by the existing cross-pin machinery
(`vg_density_census.rs`'s `the_pins_declared_byte_identical_actually_agree`) so a `-Bless` cannot
silently redefine the gate.

- **Why byte-identity is the right claim:** a rejected instance never writes `vb_id`; a drawn-but-
  z-failed instance also never writes `vb_id`. The occluder is strictly in front, so there are no
  depth ties and no ordering question. The two runs must agree exactly.
- **What it CANNOT claim:** that anything was rejected. Satisfied by a cull that rejects nothing, and
  — per the frame-index trap — GUARANTEED to be satisfied that way if the capture is frame 1. That is
  G-P3-B's job, and the pair is the evidence.
- **Controls, all to be EXECUTED:**

| # | corruption | expected |
|---|---|---|
| A1 | invert the verdict (`depth_near > occ`) | **RED** — the occluder itself is rejected, the hidden spheres are drawn, the image changes grossly |
| A2 | `<=` instead of `<` in the verdict | **RED or GREEN, and the answer is a finding.** The oracle's strictness pin exists because equality must KEEP; if this scene cannot reach equality the control does not fire, and **that is reported**, with the boundary case left to G-P3-D's constructed corpus |
| A3 | delete the late cull dispatch entirely | **RED** — every deferred instance vanishes (the host seed of `0` draws nothing). The control proving the late phase is load-bearing |
| A4 | run with `FORCE_LATE` | **GREEN** — a different execution reaching the same pixels. See G-P3-C |

### G-P3-B — the GPU rejected something, and it rejected EXACTLY what the oracle says

The load-bearing gate, and it reuses machinery that already exists: `BOYKO_VB_CULL_READBACK` +
`BOYKO_HZB_DUMP` armed **in the same frame**, in the `vb_inst_cull_corpus.rs` worker/driver shape.

The host then has: the instance ring and the mesh bounds (it built them), the view-projection (it
pushed it), the pyramid (dumped), the early depth (dumped, D10) and the GPU's complete partition
(readback: `visible_instance[]`, `late_visible[]`, `indirect[]`, `indirect_late[]`).

It asserts, per batch:

1. `Σ n_defer > 0` — **the non-vacuity clause, an assert and not a report.**
2. `k + n_defer ==` the frustum-survivor count the existing CPU census computes. Piece 3 removes
   nobody from the union; it only partitions it. **A cull that "rejects" by dropping instances
   outright fails here.**
3. For every index in `late_visible[base .. base+n_keep)`:
   `occlusion_verdict(layout, dumped_pyramid, view_proj, world_aabb) == Keep(_)`.
4. For every deferred index NOT in the kept prefix: `... == Reject`.
5. `n_keep == indirect_late[b].instanceCount`.
6. The captured frame index is ≥ 3.

Clauses 3 and 4 are the **oracle equivalence** — an independent implementation of the same predicate
over the same numbers, which is the standard `vb_inst_cull_corpus.rs` already sets for the frustum
arm (*"a disagreement here is a FINDING … It must be reported, never 'fixed' by editing the
expectation"*).

- **What it CANNOT claim:** anything about the EARLY phase's verdicts, because the pyramid the early
  phase read was overwritten before the dump. The early phase is unfalsifiable by construction and it
  does not need to be falsified — VG-P3-RECOVERY makes it a heuristic. **Stated so the gap is a
  design property rather than a hole.** See Open Question 7.
- **Controls:**

| # | corruption | expected |
|---|---|---|
| B1 | perturb one texel of the pyramid the HOST uses before running the oracle | **RED on clause 3 or 4** — proves the comparison is live and not comparing a thing to itself |
| B2 | `take(1)` in the late compaction loop | RED on clauses 2 and 5 |
| B3 | capture frame 1 instead of frame ≥ 3 | **RED on clauses 1 and 6** — this is the frame-index trap, executed |
| B4 | run on a scene with no occlusion | **RED on clause 1** — the non-vacuity clause fires, which is why it is asserted |
| B5 | drop the `firstbithigh(0)` guard | RED — single-texel rects select level `0xFFFFFFFF` → `LevelUnavailable` → everything KEEPs → clause 4 fails |

### G-P3-C — FORCE-LATE: the late scope actually rasterises, and the ordering is real

`BOYKO_VG_OCC_FORCE=late` sets `VB_CULL_OCC_FORCE_LATE`, so the early phase defers **every** marked
instance regardless of the pyramid. The pin `vb_occ_force_late` must be byte-identical to
`vb_occ_hidden_off`.

This is the control that makes three otherwise-unreachable properties reachable **on a static scene**:

1. **The late raster path produces correct pixels.** Every marked instance is drawn by the late
   scope, through `vb_set0_late`, through the indirection bit, with a GPU-written `instanceCount`.
2. **The ordering.** With FORCE-LATE the early depth contains only the unmarked instances, so
   `depth_early ≠ depth_final` at many texels **by construction**, and G-P3-E's two-sided clause is
   non-vacuous.
3. **`late_draws` and `late_cull_dispatches`** are exercised at `draw_batches ≥ 2` — the multi-batch
   late scope piece 2 recorded as *"piece 3's first gate"*.

`VB_CULL_OCC_FORCE_KEEP` is the mirror: the early phase defers nothing, which is exactly today's
behaviour and is the null control the decidability-floor protocol asks for in the same sitting. It is
also niagara's `O` key by another name — the field's own answer to Q6 is a cull-OFF toggle in the
shipping binary.

**Why runtime bits and not a `#[cfg]` or a moving camera:** a gate reachable only by a moving camera
or a debug build is a gate that rots. These are push-constant bits, exercised by committed pins, on
the same binary every other pin uses.

⚠️ **What FORCE-LATE cannot claim:** that the *unforced* early phase defers anything. That is
G-P3-B clause 1.

### G-P3-D — the SHADER's verdict equals `boyko_render::hzb`'s, with no engine involved

`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs`, the `hzb_build_oracle_gate.rs` shape: its own
pyramid image, its own uploaded instance rows and bounds, its own dispatch, its own readback. It
compares the shader's partition against `occlusion_verdict` over three corpora:

1. **The oracle's own fixtures** — the `7×3` anchor, the `8×16` exact-integer edge, `1×1`, `511×1023`,
   `1920×1080`. These already carry the boundary properties the oracle's 26 tests pin, and four of
   them are non-power-of-two — the configuration Bevy ships its whole feature as *experimental* for
   ("precision issues with non-power-of-two framebuffer sizes, occasionally misclassifying small
   meshes as occluded"). This engine's `prev_pow2` level-0 makes the mapping non-identity, and
   **that mapping is where the field's bugs live**; corpus 1 is where it is tested.
2. **A random corpus** — ≥ 100 000 (matrix, AABB) pairs from the committed xorshift, including
   AABBs that straddle the near plane, sit wholly behind the eye, are off-screen, and carry the
   `±1e30` unknown-bounds sentinel. Each `KeepReason` class must be **observed at least once**, and
   the observation counts are printed — a corpus that never reaches `BehindEye` proves nothing about
   it.
3. **A constructed boundary corpus** — cases with `depth_near == occ` **exactly**, built by planting
   a pyramid texel equal to a corner's computed `z_ndc`. This is the only place the `<` vs `<=`
   difference is decidable, and it is the one difference that deletes geometry.

- **What it CANNOT claim:** that the ENGINE's cull reads the right pyramid, the right ring, the right
  matrix or the right extent — it builds its own everything. That is G-P3-B's job. This is verbatim
  the G3/G8 division piece 1 established.
- **Controls:** (D1) `ceil(hi)-1` instead of `floor(hi)` — must red on corpus 1's exact-integer-edge
  extent; (D2) clamp `level` down to `levels-1` instead of KEEPing — must red (the oracle's own
  `keep_case_level_unavailable_never_clamps_down` shows this is a FALSE REJECT); (D3) drop `precise`
  from the projection and report whether anything moves — **a control whose null result is itself the
  finding**, since it measures whether this device's compiler contracts the mad at all.

### G-P3-E — G5 under a DRAWING late scope, two-sided

Extends piece 2's `hzb_engine_pyramid_equals_the_oracle_occ` with the D10 payload:

1. `build_pyramid(depth_early) == pyramid`, bit-exact, all five non-vacuity clauses intact;
2. `depth_early ≠ depth_final` at ≥ 1 texel — **asserted**, and under FORCE-LATE it is guaranteed;
3. `build_pyramid(depth_final) ≠ pyramid` — the pyramid was NOT built from the final depth.

Clause 3 is the ordering proof piece 2 could not make. **Control:** move the poison+build block back
after the late scope — clauses 1 and 3 both red.

### G-P3-F — the derived barrier stream, per configuration, FIELD by FIELD

Extends `vb_barrier_stream_baseline.rs`. Piece 2's round 1 learned this the hard way: a barrier COUNT
is the assertion that certifies the defect it exists to catch. Fields, not counts.

New rows: the pyramid's cross-frame seed on the four U-rows (P3-0, in isolation); on the S-rows,
`vb_late_visible`'s COMPUTE→COMPUTE RAW, `vb_indirect_late`'s four-link chain
(`TRANSFER→COMPUTE→COMPUTE→DRAW_INDIRECT`), the pyramid's `hzb_build`→`vb_cull_late` RAW at `GENERAL`
with **no layout change**, and `vb_raster_late`'s two new VERTEX reads.

- **Controls:** (F1) delete `vb_cull_late`'s `vb_indirect_late` write declaration → the
  `vb_raster_late` fetch's `src` reverts to TRANSFER while the count stays the same — **the control
  that proves fields, not counts**; (F2) delete the pyramid read on `vb_cull_late` → the RAW against
  the build vanishes; (F3) declare `vb_raster_late`'s `vb_late_visible` read at the wrong stage
  (FRAGMENT) → the derived edge moves.
- ⚠️ **What it CANNOT claim:** that `declare_vb_graph` writes this shape. It is a hand-written
  REPLICA, exactly as `framegraph_gbuffer_equiv.rs:2405-2412` says of itself. The gap is closed by
  the production `debug_assert`s (D8) which run in every dev-profile golden, plus
  `VbRecordProbe::late_cull_dispatches`, which is the only number in this piece that originates in
  the real recorder.

### G-P3-G — validation, armed vs unarmed, message for message

The P1-2 / P1-4 / G3 leg. It is the leg that sees: the SampledImage descriptor's type and layout
against a mip-complete view, `vb_late_visible`'s usage bits, `hzb_null`'s legality, the widened push
range against `maxPushConstantsSize`, and the second descriptor set.

⚠️ **Its limit is MEASURED, not assumed.** Piece 2's P2-0 established that synchronization validation
is **not live on this machine** — `VK_EXT_validation_features` is absent and the instance chain
degrades silently; a real barrier was deleted and produced 19 messages, no `SYNC-HAZARD` and a
byte-identical image. **So a missing barrier is invisible to G-P3-G, invisible to G-P3-A, and
invisible to the probe.** G-P3-F and the production asserts are the only barrier evidence, and that
sentence belongs in the commit message.

### Mandatory unit tests

- `VbCullUniform` size/offset const-asserts; `VB_LATE_VISIBLE_ELEMS == VB_VISIBLE_INSTANCE_ELEMS`.
- `VB_BATCH_CULL_PUSH_BYTES == 112` and `COMPUTE_PUSH_CONSTANT_RANGE_BYTES <= 128` (the existing
  const-assert, verified still live).
- The math-row inversion: `view_proj_rows` built from the push bytes equals the matrix
  `frustum_planes_from_push_bytes` was derived from — one test, both consumers.
- `path_vb_occlusion_split()` false with `hzb: None` and true only with all four conjuncts.
- `HzbDumpLayout`: `total_bytes`, both depth offsets, `header_words`, and that the OLD magic fails to
  decode.
- The compaction lemma as a host model: a proptest over `(n_defer, keep-mask)` asserting the in-place
  compaction preserves order and never reads a clobbered slot.
- `vb_cull_batch_count_visible_clamp` unchanged (its existing tests at `vb.rs:4098-4239` must not
  move).

### `debug_assert!` invariants

- `plan.vb_cull_late.is_some() == scene.path_vb_occlusion_split()`, at both declare and record.
- The three declare-order asserts (D8).
- Every late record's `first_instance == 0` (host side), and the host seed is `0` (D9's replacement).
- The late push carries bit 1 only with bit 0 (`vb.rs:1596-1602`'s shape, applied to the late scope).
- `n_defer + k <= instance_count` — as a host-side check on the READBACK, since the shader cannot
  assert.
- `scene.vb_occ_flags & VB_CULL_OCC_ARMED != 0` implies `scene.hzb.is_some()`.
- `VB_CULL_OCC_FORCE_LATE` and `VB_CULL_OCC_FORCE_KEEP` are never both set.

---

## Boundary — what piece 3 does NOT do

**No config knob.** `HzbConfig` gains no variant; no `OcclusionConfig` is introduced. Arming is the
capability (piece 2's `OcclusionCulling`) AND the pyramid. **Piece 4 owns the owner-facing knob**, and
until then the feature is reachable only by marking entities.

**No `vkCmdDrawIndexedIndirectCount`, no `vkCmdDispatchIndirect`, no device-feature change.** D4
explains why neither is needed, not merely why neither is affordable.

**No change to `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl` or `vb_geom_fetch.hlsli`** — and this is a
gate-quality commitment (D3 / D5), not an accident.

**No change to `vb_visible_instance`, `VB_VISIBLE_INSTANCE_ELEMS`, `INSTANCE_CAPACITY`,
`vb_cull_batch_count_visible_clamp` or the R2d-6 const-assert.** They appear in the diff as context
only; if any moves by one character the piece has re-created the R2d-6 collision.

**No per-instance dispatch.** The cull remains **one lane per batch with a serial inner loop** (C6).
For a batch with 1000 instances one lane performs 1000 projections. That is a shipped property of
R2c0 / R2d-6, not a regression, and changing it means re-deriving the region-write scheme that
currently needs no atomic. **Named because it is the first thing to look at if a measurement ever
shows the cull dominating.**

**No previous-frame reprojection**, no `prev_view_proj`, no previous transforms, no
`#[cfg(feature = "hwrt")]` prev-ring work. D1 states why, and the cost is early-phase hit rate only.

**No second pyramid build.** Bevy builds two per frame; nothing here consumes a pyramid of the final
depth, so one build serves both phases (D1).

**No occlusion for shadow cascades, for the SDF leg, or for any non-VB path.** `path_is_vb()` and
`mesh_leg` remain conjuncts.

**No mesh-level or cluster-level (meshlet) occlusion.** The cull unit stays the INSTANCE. The
research's granularity caveat is discharged for this rung (R2d moved the engine to per-instance) and
finer granularity is the R4+ ladder.

**No overflow policy**, because overflow is unreachable (D3). niagara and Nanite both drop and
surface it as blinking geometry; this design cannot reach that state.

**No perf claim, and no benchmark pin.** Stated twice on purpose.

**No fix for the 19 outstanding validation messages.** Owner-deferred.

---

## Open questions

1. **DESIGN, for the critic.** Should the projection leaf become a `boyko_shaderdsl` leaf rather than
   a hand-authored mirror (D11)? It would buy bit-exactness by construction; it costs a rewrite of
   `hzb.rs`'s projection chain as a generic body while keeping its 26 pins green, in the same
   campaign that arms a decision. My call is **hand-author now, gated by G-P3-D**, and convert only
   if G-P3-D shows a disagreement not attributable to a stated cause. If the critic disagrees, this
   is the fork to argue.

2. **VALUES, owner.** `VB_CULL_OCC_FORCE_LATE` / `_FORCE_KEEP` are shipped as production push bits
   driven by an env knob. They cost two bits and make four properties gateable on a static scene
   (G-P3-C). The alternative is `#[cfg(debug_assertions)]`, which would make the controls unreachable
   in the release leg CI runs. My call is **production bits** — it is also what niagara ships — but
   it is a values call because it puts a debug affordance in shipping code.

3. **MEASUREMENT, deferred.** The early phase's hit rate under current-view-proj-vs-stale-pyramid
   (D1) versus previous-view-proj. Settling it needs an occlusion-heavy scene with a moving camera
   and a rejection-count readback over N frames. It changes no pixel and no correctness property, so
   it is not a blocker; it is the first thing to measure if piece 4 ever wants a perf case.

4. **VERIFY AT P3-1.** The `sync_gbuffer` reordering (HzbTargets before `vb_cull_set`). The set block
   at `targets.rs:4606-4667` documents itself as "the NEW TERMINAL fallible set … its error path
   tears down every prior set"; the HZB is built far later at `:7722+`. I have specified moving the
   HZB build UP rather than the set block DOWN, because the HZB's own error path already returns
   before any set exists. **If the HZB build turns out to depend on something constructed between
   those points, this becomes a real design question and must come back here rather than be patched
   at the call site.**

5. **VERIFY AT P3-1.** Whether `hzb_null` is needed at all, or whether the device's
   `descriptorBindingPartiallyBound` (enabled for the bindless set) can be extended to this binding.
   `hzb_null` is 4 bytes and needs no feature, so it is the specified answer; the alternative is
   recorded only so a reviewer does not raise it as an omission.

6. **VERIFY AT P3-6.** Whether `vg_density_census_gate` requires a measured density row for each new
   pin, or whether the set-equality bump suffices. Piece 2 left the same question open and its answer
   is in `vg_density_census.rs:57-64`; read it before authoring the bump.

7. **A limit I cannot close.** Nothing in this repository can observe the EARLY phase's verdicts,
   because the pyramid it read is overwritten before any dump. G-P3-B clause 1 observes only that the
   early phase deferred *something*. If a future defect makes the early phase defer everything (or
   nothing) while the late phase compensates, every gate here stays green and only a perf measurement
   would notice. **Closing it would mean dumping the pyramid TWICE per frame — a third staging region
   and a second dump pass — deliberately not proposed, and recorded here so it is a known gap rather
   than a discovered one.**

---

# ROUND 1 CRITIQUE — five lenses, every finding adversarially refuted

**25 findings survived a skeptic; 17 were killed. Verdict: REJECTED.**

Not for the design — several of its load-bearing decisions are sound and well-defended. It is
rejected because five claims are false against the tree, and because **the piece's two headline
gates are green in exactly the states they exist to catch.** That is the SIXTH instance of this
campaign's signature defect, and the first to arrive in a piece whose change is supposed to move
pixels — where byte-identity is no longer the claim and a vacuous gate has nowhere else to be caught.

⚠️ **Every line number below is as-of-the-critique.** Re-anchor at use.

# ROUND-1 CRITIQUE — docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md

## 1. VERDICT

**REJECTED**

Not for the design. D1's convergence argument, D4's word reuse, D5's one-module/two-phase split and D11's mirrored leaf are all sound and mostly well-defended. It is rejected because five separate load-bearing claims are false against the tree, and because **the piece's two headline gates — G-P3-A's byte-identity pair and G-P3-B's oracle clauses — are green in exactly the states they exist to catch.** That is the sixth instance of this campaign's signature defect, arriving in the first piece whose change is supposed to move pixels. Round 2 on the document, then implement.

---

## 2. BLOCKERS

### B1 — G-P3-B's readback is recorded BEFORE `vb_cull_late`; clause 5 is green exactly when the late phase rejects nothing

`crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:3921` (declared between `vb_batch_cull`:3879 and `vb_raster`:3949) + `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:1368-1446` (recorded inside the `batch_cull_armed` block, copies at :1444) vs plan `:499`, `:530`, `:972`, `:973`, `:1140`, `:1146`, `:1292`.

Verified this session. `vb_cull_readback` is one pass, declared and recorded before `vb_raster`; `vb_cull_late` lands after the last `hzb_build_*`, i.e. long after. The copy therefore captures `vb_indirect_late[b].instanceCount == n_defer` (the EARLY write, plan `:882`) and the **uncompacted candidate list**. Clause 5 becomes `n_keep == n_defer` — RED when the late phase works, GREEN when it rejects nothing. Clauses 3/4 partition the candidate list by a count the buffer never held. D3's comparison-table row 6 (`:284`, "observes BOTH, in separate regions") is refuted by the pass's position, not by the buffer choice.

Amplification the plan must confront: clause 2 and the `:1292` debug_assert need `n_defer`, observable **only before** `vb_cull_late`; clauses 3/5 need `n_keep`, observable **only after**. **No single copy point serves all five clauses.**

**PLAN EDIT:**
- Rewrite D3 row 6 and G-P3-B `:1136-1147` to specify **two snapshots**: keep the existing pre-late copy for `n_defer`/candidate list, add a second readback pass declared after `vb_cull_late` for `n_keep`/kept prefix (or persist `n_defer` into a separate per-batch word so one post-late copy suffices).
- Add `vb_cull_readback` to the `graph_bridge.rs` change row at `:972` with its new `(TRANSFER, TRANSFER_READ)` accesses named — `:973` currently assigns the regions to `vb.rs` alone, which would ship undeclared copies (the P2-7 class).
- Account for the cost in G-P3-F: a TRANSFER read between `vb_cull_late`'s COMPUTE write and `vb_raster_late`'s DRAW_INDIRECT fetch re-sources that fetch to `COMPUTE→TRANSFER` + `TRANSFER→DRAW_INDIRECT`, verbatim the re-sourcing `graph_bridge.rs:3928-3940` already documents for `vb_indirect`. D8's four-link chain becomes five under the probe; S-rows must be pinned **per-configuration**.

### B2 — G-P3-B's data pairing cannot be produced: the cull readback returns on the first presented frame, and the probe line carries no frame index

`crates/boyko_app/src/runner.rs:2601-2621` (no settle, bare `return`, outside the `:2763` exit conjunction) + `:2924`/`:2949-2953` (no frame counter) + `crates/boyko_app/src/hzb_dump.rs:45,51` (30+3 frames) + `crates/boyko_app/tests/vb_inst_cull_scene/mod.rs:646-650` vs plan `:1130-1131`, `:1147`, `:1164`.

Verified this session. The block is `if vb_cull_probe && presented_ok { wait_idle; if let Some(rb) = read_vb_cull(s) { …; return; } }` — no frame gate, and `read_vb_cull` returns `Some` whenever the staging is mapped (persistently mapped at creation, `memory.rs:305`). Arming `BOYKO_VB_CULL_READBACK` and `BOYKO_HZB_DUMP` together exits at frame 1: the cull file holds frame 1, the pyramid file is **never written**. The named driver's own comment already records this ("a second armed capture would silently produce no file") and its `env_remove` list omits `BOYKO_HZB_DUMP`. On that frame-1 payload clause 1 (`Σ n_defer > 0`) is FALSE by the plan's own D1 (`:173-176`), clauses 3/4 have no pyramid, and clause 6 asserts a number no code emits.

**PLAN EDIT:** budget the work explicitly in the `runner.rs` row (`:986`) and in P3-4: give the cull readback the settle→request→drain shape its three siblings use, fold it into the `runner.rs:2763` exit conjunction, and add the engine's `frame_index` (`runner.rs:987`) to `format_vb_cull_probe_line` — the parser at `vb_inst_cull_scene/mod.rs:565-573` is key-driven, so a new field is free. Until the probe carries a frame index, clause 6 and control B3 are unimplementable as written. Do **not** resolve this by running the two captures in separate processes: the readback payload is still frame 1, so clause 1 reds for an instrument reason and the only way to green it is to relax it.

### B3 — At the fixed point the late scope draws ZERO: control A3 is green, G-P3-E clause 2 is a hard red on its own fixture

plan `:1231` (G-P3-E clause 2), `:1125` (A3), refuted by `:882`/`:903`; corroborated by `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:4995-4996`, `crates/boyko_app/tests/hzb_engine_pyramid_gate.rs:189-224`, `:563`, `crates/boyko_app/src/host_dump.rs:26`.

A rejected instance writes no depth, so `depth(E_2) == depth(E_1)` bit-for-bit and `P_prev == P_cur` from frame 2. The early phase defers iff `occlusion_reject(P_prev)`; the late phase keeps iff `!occlusion_reject(P_cur)`. Same predicate, same bytes ⇒ **`n_keep == 0` on every converged frame**, and every capture is frame 31. Consequences: clause 3 never demonstrated live; A3 ("delete the late cull → RED") is GREEN twice over — the deferred set is the occluded set, and per `:882` the early phase has already written `n_defer` into the record, so deleting the late cull draws them all at identical depth, which `vb_occ_split_gate.rs:50` measured as pixel-invisible; control B2 (`take(1)`) cannot fire either. G-P3-E clause 2 (`depth_early != depth_final`, ASSERTED) reds with no defect on the unforced fixture — and reds under FORCE_LATE too, because `setup_scene(marked=true)` marks all five spheres and empties the early depth, tripping the shipped non-vacuity clause at `:563`.

**PLAN EDIT:**
- Bind G-P3-E and A3 to a **mixed-marking** FORCE_LATE fixture (unmarked filler that populates the early depth + marked instances that go late); specify it, because none exists.
- State plainly at `:1116-1118` and `:1231` that the unforced pin proves the **early partition only**.
- Add `Σ n_keep > 0` as an assert wherever the late phase is claimed exercised; it will red on `vb_occ_hidden`, and that red **is** the measurement.
- Restate A3's expectation off the image and onto `VbRecordProbe::late_cull_dispatches == 1` + G-P3-B clause 5, and record in the corruption table that **no image-level control for "the late phase is load-bearing" exists under D4's word reuse** — as a stated limit.

### B4 — A1/A2 move the unknown-bounds test AFTER the Arvo fold, the exact inversion the shader header records a critic already catching

plan `:848` (A1 step 1) and `:874` (A2's hoisted frustum `continue`) vs `crates/boyko_rhi_vulkan/shaders/vb_batch_cull.comp.hlsl:83-99` and `:448-459`.

Verified this session. The shipped shape is `bool keep = true; if (!any(b.bmin > b.bmax)) { …fold…; keep = !aabb_outside_frustum(…); }` — the LOCAL guard is the OUTER branch, and the header states why in words, naming a prior critic who caught this exact inversion. The plan never mentions `bmin`/`bmax` (2 hits in 94 KB, both world-space), makes D11 `:638` and A1 step 1 the normative home of the sentinel test in WORLD space, and A2 places the frustum `continue` at `:874` **before** `defer`.

Two failures, one broader than the finding as filed:
- Sentinel + **any normal affine**: `lh = -1e30` ⇒ inverted world box ⇒ `radius = dot(abs(pl.xyz), h)` large negative ⇒ `dist + radius < 0` on the first plane ⇒ `continue`. Shipped code KEEPS; A2 **deletes**. Every unknown-bounds instance vanishes from both lists, so VG-P3-RECOVERY (`:147-152`) does not cover it.
- Sentinel + **zero linear part** (`Transform::from_scale` is an unguarded public `const fn`, `boyko_scene/src/transform.rs:92`): `mn == mx == wc`, so A1's `any(mn > mx)` is FALSE and the collapsed point is trivially occluded.

A1 step 1's two supporting claims are also false: `vb_batch_cull.comp.hlsl:449` is a LOCAL test, not "the same short-circuit"; and `any(mn > mx)` does **not** catch NaN — which is why the oracle spells it `!(min <= max)` (`hzb.rs:701-703`).

**PLAN EDIT:** state in D11/A1/A2/A3 that the local guard `!any(b.bmin > b.bmax)` remains the OUTER branch, so unknown bounds ⇒ `keep = true, defer = false` in phase 0 and `keep` in phase 1 — never occlusion-tested, never frustum-tested, always drawn by the EARLY scope. Delete the sentinel/NaN claim at `:848` and the false `:449` equivalence. Add a G-P3-D corpus row carrying the sentinel with (a) a normal affine and (b) an exactly-zero linear part, plus a red control that hoists the guard and shows the instance vanish — and note that G-P3-D models only the occlusion verdict, so the frustum-side deletion needs a named gate or an explicit out-of-scope statement.

### B5 — P3-0 deletes the pyramid's only `UNDEFINED → GENERAL` producer and supplies no replacement; the step is unbuildable as literally written

plan `:236-237`, `:974` (false premise) vs `crates/boyko_rhi_vulkan/src/present/targets.rs:1252-1423` (verified this session: `create_texture`, the view loop, struct assembly — **no encoder, no barrier, no submit**), `:1147-1149`, `:1202-1204` (the tree says the opposite in words), `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:3508-3512`, `crates/boyko_rhi_vulkan/src/framegraph/sync.rs:353,389-390`.

Two lenses reached this independently (one at BLOCKER, one at MAJOR); the facts are identical. `HzbTargets::build` issues no transition — `VulkanTexture::create` cannot (`texture.rs:206-211,287`: no queue, `initial_layout = UNDEFINED`). The pyramid's only layout producer is the framegraph first touch derived from `ResSync::undefined()`. Flip the seed to `seeded_writer_at_layout(GENERAL, …)` and `sync.rs:353` computes `layout_change = false`, `:389` emits `old_layout = GENERAL` — VUID-VkImageMemoryBarrier-oldLayout-01197 — on an image genuinely in UNDEFINED, **for the entire life of every `HzbTargets` generation** (build runs per generation, `targets.rs:7767`, i.e. every resize, not once at boot). The plan contradicts itself: `:253-255` correctly names the graph first touch as today's only producer — the transition `:236` attributes to `build`.

P3-0's gate set cannot see it: goldens (measured blind), G-P3-F U-rows re-pinned to whatever the implementation produces, G8/G5 dumping a pyramid the build overwrote. The validation leg — the only thing that could see a layout mismatch — first appears at P3-1.

**PLAN EDIT:** re-specify P3-0 as **clear-then-seed**, modelled on the two in-file precedents the plan does not cite: `boot_clear_shadow_temporal_hist` (`targets.rs:6687-6773`, whose final-barrier comment at `:6743` says the GENERAL layout exists precisely "to satisfy the framegraph seed's GENERAL-layout assumption") and `build_taa_hist_ring` (`targets.rs:6236-6249`, `:6279-6366`). That is encoder + fence + `UNDEFINED→TRANSFER_DST` + `vkCmdClearColorImage` over `[0, levels)` + `→GENERAL` + submit + `wait_fence` + a teardown/`wait_idle` ladder — not "one `vkCmdClearColorImage`". Add a **fourth reason, first**: the clear is what makes the seed's GENERAL claim true, so the seed is unsound without it. State the degrade policy (unlike TAA, a failed clear cannot degrade to "no clear" while the GENERAL seed stands — it must disarm the HZB or fail the build), and resolve the interaction with Open Question 4's `sync_gbuffer` reorder there rather than in OQ4.

---

## 3. MAJORS

**M1 — the early cull writes `n_defer` into the late draw record, so D9's assert message is false.** plan `:335-337`/`:475`/`:882` vs `:579`, `:583-584`, `:1125`; `vb.rs:1224-1230` asserts over the HOST-local `records` array. "The late cull is the ONLY producer of a nonzero value in this array" is refuted by D4 on the same page; `:583-584`'s "safety property" is precisely inverted — a missing late dispatch draws `n_defer` **untested** instances, not a blank scope. Delete `:583-584`, restate the assert message to the only true claim (the host seed), and note that the replacement assert is structurally blind to the GPU-written word. Caveat for the A3 corruption: under a dev-profile golden it may panic on `:1288`'s declare/record parity assert and be recorded as "RED" for an unrelated reason.

**M2 — `graph_bridge.rs:3502-3507` prescribes the OPPOSITE seed, and the plan cites it as corroboration.** plan `:187` ("predicted the answer verbatim") and `:65` vs the comment's `seeded_readers_at_layout(GENERAL, COMPUTE, SHADER_READ)`. The plan's seed is correct; the comment is not, and `sync.rs:305-311` says so in the engine's own words ("the reader WAR seed would leave the read FREE/already-visible, which is exactly the race"). Reword `:187` to "predicted the obligation, not the answer"; relabel obligation-table row 4 from "the cross-frame WAR" to "the cross-frame RAW (WAR subsumed)"; **add `graph_bridge.rs:3497-3507` to the P3-0 edit list** — the plan already commissions the sibling rewrite at `:4049-4056` (`:972`), and this is the same discipline at the site P3-0 actually edits. State the two-residual argument (armed frame ends on a read, build-only frame ends on a write; only the writer form is conservative for both) — that is the real justification and the plan does not give it.

**M3 — declaring `vb_late_visible` as combined `SHADER_READ|SHADER_WRITE` on `vb_cull_late` opts it out of the guard 1977fe0 installed.** `graph.rs:694,703-704,721-723` vs plan `:507`. `is_write ⇒` the read half is never tested and `res_written` latches, so deleting the early cull's write declaration (`:493`, one gated line in the closure that already omitted one in P2-7) is silent at `vb_cull_late`, silent at `vb_raster_late`, invisible to goldens, validation, the probe, and G-P3-F (a replica, by the plan's own `:1252-1253`). Split it read-then-write on `vb_late_visible` **only** — `vb_indirect_late` is already latched by `vb_indirect_late_upload`'s TRANSFER_WRITE (`graph_bridge.rs:3862-3870`), so the extra call there is inert. Note the cost: the split yields a second self-WAR execution-only edge, a NEW G-P3-F row. Also **state the declarator** (`add_buffer`, not `add_buffer_seeded`) — after 1977fe0 that spelling IS the provenance claim, and the plan never says it. And name what replaces the guard's ability to re-catch the P2-7 corruption on `vb_indirect_late`, which piece 3 retires by adding `vb_batch_cull`'s write at `:494`.

**M4 — the uniform's intra-pass `TRANSFER→COMPUTE` edge works at exactly one record site, unnamed.** `record.rs:50-56` + `graph.rs:621-624` (a pass's ENTIRE barrier set emits at ONE boundary) vs `vb.rs:1293→1301` (the correct precedent, with the reason in its SAFETY comment at `:1298-1301`) vs `vb.rs:1179/:1252` (the neighbouring pass inverts the visible order). Plan `:973`/`:1036` say only "the uniform fill". Land it after `record_vb_pass` and the barrier precedes the write; with `FRAMES_IN_FLIGHT = 2` the dispatch reads **frame N-2's view-projection**, bit-identical on every static fixture, so every gate stays green and only motion breaks it. Name the site in D6 and P3-2 ("immediately before `record_vb_pass(vb_batch_cull, …)`, beside the counter fill at `vb.rs:1293`") with the reason, and execute the corruption, reporting the GREEN null result.

**M5 — the projection uses `dot()`, which this repo has already rejected in writing for host-oracle agreement.** plan `:410`, `:705-706` vs `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl:127-141`: Vulkan specifies `OpDot` only as "inherited from a formula", and permits that formula to be transformed by associativity — the comment's remedy is at `:142-143` (`precise` on every node of a written-out sum). The oracle is an explicit left-fold (`hzb.rs:726`). `precise` emits `NoContraction`, which forbids contraction, not reassociation, so D11's list at `:680-682` does not cover it; control D3 (`:1222-1224`) mutates contraction only. `depth_near = max(cz * inv_w)` is downstream, and `:677` names 1-ULP-LOW as the geometry-deleting direction. D11's own title (`:633`, "statement-for-statement mirror") contradicts `:410`. **Delete `dot()` from the projection**, spell the fold explicitly with `precise` on every node, cite `cluster_cull.hlsl:127-141` as the governing precedent, and add a D4 control that swaps the fold for `dot()` and reports whether the differential moves — null result recorded in P1 §10's narrowed shape. (Credit: on the ±0 divergence the plan is right — `depth_near < occ` compares −0.0 and +0.0 equal, so P1 §10's measured tie-order divergence provably cannot reach this verdict. Say so explicitly.)

**M6 — G-P3-F's F1 control is mis-specified; its stated outcome is unreachable.** plan `:1247-1249` vs `:474-477`/`:494`, `sync.rs:373-383`, `:400-405`, and the already-pinned identical shape at `tests/vb_indirect_barrier_chain.rs:107-109` ("the last writer of `vb_indirect` is COMPUTE, not TRANSFER"). Deleting `vb_cull_late`'s `vb_indirect_late` write leaves `vb_batch_cull` as the last writer, so `vb_raster_late`'s src is field-identical and the COUNT drops by one — the opposite of "src reverts to TRANSFER, count stays the same". The correct fields-not-counts control on this chain (matching `a_dropped_writer_keeps_every_count_and_moves_only_fields`) is to **drop the WRITE BIT** of `vb_cull_late`'s RW, keeping `SHADER_READ`: all counts preserved, `vb_raster_late`'s `src_access` moves `SHADER_WRITE → 0`. Note also that the finding's alternative (delete the EARLY write) does **not** preserve total count — that access emits a WAW against the TRANSFER upload.

**M7 — controls B1 and B5 depend on scene properties nothing asserts.** plan `:1162`, `:1166` vs `:1106`; `vb_mesh.rs:221` + `hzb_plan.rs:34-41` (512×512 ⇒ level 0 is 1 texel/pixel); `hzb.rs:790-801,817-824,837-859`. B5 fires only on a sub-texel rect, which "M ≥ 8 small spheres" does not guarantee; the plan's own testability argument for the guard (`:453`) points at G-P3-D's corpus, not this fixture. B1 fires only if the perturbed texel is one of the four sampled for a deferred instance and crosses its `depth_near`. B1 and B5 are the **only** controls touching clauses 3 and 4. Two corrections: B5's trigger is per-axis (unsigned `max` at `hzb.rs:797` makes `firstbithigh(0) = 0xFFFFFFFF` win), and its red lands on **clause 3**, not clause 4 (with the guard dropped everything KEEPs, so clause 4 is vacuously true). Move B5 to G-P3-D (corpus 1's 1×1 layout makes it unconditional); respecify B1 against the texel the oracle's own `select_texels` reports for a NAMED deferred instance, in the direction that crosses its `depth_near`, asserting exactly that instance's clause flips.

**M8 — G-P3-B clause 5 is a tautology as phrased, and it is the only cover for the OVER-count class.** plan `:300`, `:903`, `:828-834`, `:1146`; precedent `runner.rs:2914-2915,2941-2947` and `vb_inst_cull_corpus.rs:379-386`. The host's only source for `n_keep` is the word it read. Worked miss: deferred `[e0,e1,e2]`, keeps `{e0,e2}`, a cursor bug writes `keep = 3` — region reads `[e0,e2,e2]` (slot 2 is residue), clause 3 green, clause 4 green (`{e1}` is Reject), clause 5 tautologically green, G-P3-A blind by `:1083`. **Define `n_keep` independently** as the count of oracle-Keep verdicts over the deferred set (clause 4 already requires that set), and add the strictly-ascending / no-duplicate clause on `late_visible` that the frustum arm already has at `vb_inst_cull_corpus.rs:404-421`. Also correct B2's prediction at `:1163`: clause 5 cannot go red there.

**M9 — `crates/boyko_app/src/vb_probe_dump.rs` is absent from the Integration list.** `:129`, `:158` break on D9's rename (hard E0609); `write_probe` at `:144-168` is the **only** serializer of `VbRecordProbe`, so with the file untouched `late_cull_dispatches` — plan `:1255`'s "the only number in this piece that originates in the real recorder" — never reaches a test. Add the file to P3-4/P3-6 with three named edits (the field access, the emitted key + a new `late_cull_dispatches = {}` line, the eprintln label) and run the control: delete `p.late_cull_dispatches += 1` and show `vb_occ_split_gate.rs` reds. Note that `field()` at `vb_occ_split_gate.rs:414-417` PANICS on a missing key, so "never emitted" and "never incremented" red differently — say which is expected. The `schema_version` bump is hygiene, not a defect (three existing defenses).

**M10 — G-P3-A's baseline moves two variables, and the one-variable control the plan already designed gets no pin.** plan `:1104-1112`, `:1004`, `:1184-1187`; `gpu_scene/mod.rs:6562` ("None on the default `HzbMode::Off` — no image, no views, no passes"); `vb_mesh.rs:240-242`. `vb_occ_hidden_off` differs from `vb_occ_hidden` by the cull, the pyramid's entire existence, **and** the late scope/dispatch/descriptor-set/render-pass bracket. `VB_CULL_OCC_FORCE_KEEP` is a shipped production shader branch that **no gate in the plan ever sets** — its only stated use is a null control for a measurement the plan declines to make. Add `vb_occ_hidden_keep` (`BOYKO_VG_OCC=1 BOYKO_VG_OCC_FORCE=keep`), pin it equal to `vb_occ_hidden`, and make it the byte-identity baseline. Write `BOYKO_VG_HZB="1"` into all four `[*.env]` blocks. Register all four names in `VB_PINS` (`vg_density_census.rs:59`) **and** the relations in `DECLARED_IDENTICAL_PINS` (`:238`) — the plan cites the cross-pin guard at `:1109-1110` but its change row at `:992` names only `VB_PINS`.

**M11 — OQ4's `sync_gbuffer` reorder is not a statement move.** `targets.rs:2800-2817` (`DeferredSets::build`, 10 params, no extent/HZB) owns `vb_cull_set` at `:4630`; `GBufferTargets::create` calls it at `:7015` and builds the HZB at `:7767`. Binding the pyramid at @9 needs an **eleventh parameter**, `hzb_depth_ring` must be rewritten to take the `forward`/`core` LOCALS instead of `targets.*` (the struct does not exist until `:7690`), and **three** error arms (`:7049`, `:7110`, `:7153`) gain a pyramid drain — which refutes bullets 2 and 3 of the placement argument the code carries at `:7732-7745`, un-rebutted by the plan. OQ4's escape hatch does not fire on its own terms (scene/extent are `create` params; `forward`/`core` both precede `:7015`). Rewrite the targets.rs row and OQ4 to the real shape, enumerate the three arms, rebut the placement bullets one by one — or cost the alternative (leave the HZB last, write @9 in a second update) honestly: `RhiDevice::create_bind_group` writes the whole set once (`rhi_impl/device.rs:329`) and the RHI exposes no update-one-binding entry point, so it needs a new helper.

---

## 4. MINORS WORTH DOING

- **`hzb_null` "is never dereferenced" (plan `:790`) is refuted by the engine's own recorded argument** at `graph_bridge.rs:3987-3992` ("DXC is free to lower the `?:` to an eager load plus an `OpSelect` … that is why the shader header carries an in-range argument for the not-taken address"), and by `hzb_build.comp.hlsl:478-481` ("'No tap is issued' has to be STRUCTURAL"). Plan `:889` advertises the gate as "Branchless where it matters". Replace with an in-range argument: clamp the not-taken load's coords **and level to 0 unconditionally** — not derived from `levels`, because `:496-497` deliberately leaves `VbCullUniform` unwritten on exactly the boots where `hzb_null` is bound. No pixel moves (`defer` is false under both lowerings), so this is a claim defect.
- **`hzb_null` is never transitioned.** `texture.rs:287` leaves it UNDEFINED; the @9 descriptor declares a layout; D5 makes the module statically use the binding, so it becomes a core-validation error at P3-4 — the arming commit, where an unexplained message delta on G-P3-G is hardest to attribute. Precedents: `csm.rs:351-365,394-417` (`seed_boot_layouts`, which transitions the 1×1×1 dummy for exactly this reason) and `targets.rs:6303-6347`. Fold into B5's boot helper. **Related and worth a separate look:** `:779` declares @9 as `(read, GENERAL, mip-complete)`, but `BindGroupEntry::SampledImage` hard-writes `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL` at `rhi_impl/device.rs:541`, contradicting `:1244`'s "at GENERAL with no layout change" for the real pyramid.
- **`vb_cull_layout` is 7 → 11, not 7 → 10.** `gpu_scene/mod.rs:3919-3972` declares seven entries; the plan adds four (`:777-780`) and says "10" in six places (`:776`, `:974`, `:976`, `:985`, `:1027`, `:970`). The normative binding table is correct; only the derived count is wrong, and `device.rs:345-348`'s debug_assert + core validation intercept it. State it once, derived, and const-assert the layout array length against the new binding-count constant. Keep the secondary ask: name @7/@8/@9/@10 each in its own census assertion the way @4/@5 are at `vb_batch_cull_spv_sync.rs:331-339` — DXC provably strips declared-but-unloaded resources, so which survive in each phase is a **measurement**, not a prediction.
- **The header-drift guard cannot fire on D10's change.** `hzb_engine_pyramid_gate.rs:133-134`: `(HZB_DUMP_HEADER_WORDS - 4) / 2` stays 17 across 38→39 by truncation, while every `word(bytes, 4 + 2*k)` read at `:426`/`:437` shifts. The mis-decode reds loudly on the tail-zero loop (`:436-445`), so it cannot ship — but it reds naming the wrong defect. Export `HZB_DUMP_HEADER_SCALAR_WORDS` that both `HZB_DUMP_HEADER_WORDS` and the gate's offsets derive from, and add `:426`/`:437` plus the hardcoded `152` at `:455` to the P3-5 list.
- **D2 leaves the cross-frame WAR against `hzb_dump`'s TRANSFER_READ unstated.** `:203` names it as the last pyramid access on a dump frame; the WAR bullet at `:209-215` covers only the COMPUTE reader, and no derived `srcStageMask` can ever name TRANSFER (`sync.rs:313-321` seeds `visible_stages = 0`; `:376-379` takes it from there). The exposure is one frame, diagnostic-only, **pre-existing and strictly improved** by D2 (today's `undefined()` seed gives TOP_OF_PIPE + a licensed content discard), and value-invisible on the static corpus. One paragraph stating all of that closes it; a both-halves seed is cheap if wanted, since `sync.rs:406-413` shows a first READ ORs `visible_stages` with no `transition` change.
- **Three of four `window_present_gbuffer.rs` anchors are stale** (`:977` cites 2265/3366/8390/9904; live are 2265/3376/8410/9934), and they were already stale in the plan's own commit — `git show e7531c7` yields exactly the cited quadruple, and both shifting commits are ancestors of 97e4905. That falsifies the header at `:12-13` ("Every `file:line` below was re-verified … on 2026-08-06"). Re-anchor, and either re-verify the header's claim or downgrade it to `:15`'s honest "name + hint; grep the name".
- **D9's deletion range at `:573` is one token too wide** (`vb.rs:1220-1230` swallows the `first_instance == 0` assert at `:1220-1223`). `:64` and `:91` carry the correct range and `:1290` requires the assert to survive, so it is editorial — fix the number.
- **Piece-3 additions that no clause names:** `VG-P3-RECOVERY` (`:147-152`) is stated as an invariant but no gate asserts it directly; clause 2 covers it only if the census is the frustum-survivor count. Say so explicitly rather than leaving the reader to derive it.

---

## 5. MECHANICAL FACTS THE REFUTATIONS ESTABLISHED — do not re-derive

**Allocation / sizing**
1. `BoundBuffer::size` is the **requested** size verbatim (`memory.rs:60-61`, `:305`, `:538`); the driver's `reqs.size` feeds only the suballocator (`:277`, `:516`) and is never stored. No alignment round-up, no usage-flag pad can make two identically-sized buffers differ at runtime. The const-assert **is** what the runtime path reads (`gpu_scene/mod.rs:290-293` says so in prose).
2. Do **not** fold `vb_late_visible`'s size into the `.min()` clamp chain. `vb.rs:1183-1193`: "it is asserted, not min-ed, because a late array SHORTER than the early one would silently drop the tail batches." Add the runtime backstop instead: `debug_assert!(late_visible[fi].size / 4 >= visible_elems)`, matching `vb.rs:1188-1193`.
3. `MAX_BIND_GROUP_BINDINGS = 24` (`targets.rs:205`), so an 11-entry `vb_cull_layout` is legal. `device.rs:345-348` debug-asserts `entries.len() == layout.entry_count` — **debug only**; release leaves an unwritten binding silent.

**Framegraph / sync**
4. `FrameGraph::compile` refills the per-(ResId, mip) state arena from `res_seed` **every frame** (`graph.rs:591`, `:994`). The derived barrier stream carries **zero** cross-frame state — an "arming-transition frame" pin would be bit-identical to the steady-state pin and cannot fail. Do not add one.
5. A barrier's src is the **last declared writer**: `sync.rs:373-383` (flush branch wins over visible), `:400-405` (every write overwrites the pending flush). A **read** clears the flush and ORs `visible_*` (`:406-413`).
6. `graph.rs:694`/`:703-704`/`:721-723`: the P2-8 provenance guard tests `is_write || res_written`, and a combined `SHADER_READ|SHADER_WRITE` is `is_write` — the read half is never tested, and the access latches. It is a **first-touch** guard; once any earlier pass declared a write on that buffer it can never fire again.
7. `record.rs:50-56` + `graph.rs:621-624`: a pass's **entire** barrier set (including intra-pass edges derived from its second declared access) emits at **one** site. TRANSFER work belonging to a pass must be recorded **before** `record_vb_pass` (precedent + reason: `vb.rs:1293→1301`, SAFETY at `:1298-1301`).
8. `sync.rs:305-311` states the seed rule in the engine's own words: a reader seed "would leave the read FREE/already-visible, which is exactly the race."

**Boot-layout precedents (three, all in files piece 3 edits)**
9. `targets.rs:6687-6773` (`boot_clear_shadow_temporal_hist`; the `:6743` comment states the framegraph-seed motive), `targets.rs:6236-6249`/`:6279-6366` (`build_taa_hist_ring`, incl. the "NOT a boot-only one-shot — `sync_gbuffer`'s resize path rebuilds targets" note), `csm.rs:351-365`/`:394-417` (`seed_boot_layouts`, which transitions the 1×1×1 dummy because the resolve `.spv` statically references it on the OFF path). `VulkanTexture::create` takes no queue and cannot transition (`texture.rs:206-211`, `:287`).

**Depth / verdict semantics**
10. `VK_COMPARE_OP_GREATER`, strict, depth-write ON (`device.rs:1806-1813`, `scene_types.rs:2722`); `vb_raster.fs.hlsl:3` — no `SV_Depth`, no `discard`, no UAV. A redraw at identical depth is **pixel-invisible** (`vb_occ_split_gate.rs:50`), and the committed fixtures contain no coincident geometry, so the early/late reorder is byte-safe on them. `vb_mesh.rs:127-136` already constrains marking "all five or none" for exactly this reason.
11. `hzb.rs:855` is strict `<` with equality KEEPing. `hzb.rs:698-709` guards with `!(min <= max)` — NaN-aware; `any(mn > mx)` is not.
12. `cluster_cull.hlsl:127-141` is this repo's own reasoned rejection of `dot()` for host-oracle agreement (`OpDot` is "inherited from a formula", transformable by associativity); the implemented remedy is `:142-143`. `precise` ⇒ `NoContraction` ⇒ forbids contraction, **not** reassociation.

**Probes / gates / harness**
13. `vb_cull_readback` is a **mid-frame** copy declared before `vb_raster` and recorded immediately after the cull dispatch (`graph_bridge.rs:3921-3947`, `vb.rs:1362→1444`). `n_defer` is observable there; `n_keep` is not.
14. `runner.rs:2601-2621`: the readback fires on the first **presented** frame, has no settle, and `return`s from **outside** the `:2763` exit conjunction — whose own comment names that hazard. The probe line (`:2924`, `:2949-2953`) carries **no frame index**.
15. `vb_inst_cull_scene/mod.rs:646-650`: one capture per process is a deliberate convention, and its `env_remove` list omits `BOYKO_HZB_DUMP`. Same at `vb_occ_split_gate.rs:449-451`, `hzb_engine_pyramid_gate.rs:495-497`.
16. `vb_probe_dump.rs::write_probe` (`:144-168`) is the **only** serializer of `VbRecordProbe`; `vb_occ_split_gate.rs:414-417` `field()` **panics** on a missing key.
17. Goldens run **dev profile** (`graph_bridge.rs:5071-5072`) — every `debug_assert` is live in a golden run. `[vb_mesh_hzb]` (`PINS.toml:335-341`) sets `BOYKO_VG_HZB=1` with **no** `BOYKO_VG_OCC`, so any assert equating `hzb_build` presence with `vb_cull_late` presence panics on a correct configuration.
18. `vg_density_census.rs`: `VB_PINS` (`:59`) and `DECLARED_IDENTICAL_PINS` (`:238`) are **separate** arrays; a new byte-identical pair must land in both.
19. `mesh_leg == false` ⇒ `MeshGeometryTableSlot(None)` ⇒ `vb_mesh_bounds: None` ⇒ `batch_cull_armed == false` (`mesh_geometry_table.rs:19-27`, `runner.rs:2352-2356`, `vb.rs:964-969`) — the early cull is not recorded at all. `vb_occlusion_instances` and the `OCCLUSION_CULLING` flag lane are written as one branchless pair (`mesh_draw.rs:804-806`), so `== 0` iff no ring slot carries the bit; the over-approximation is one-directional (`scene_types.rs:3172-3175`). **But** `render_path_config.rs:952-954` + `runner.rs:607-631` make `occlusion_split && !batch_cull_armed` reachable on a device without `storage_buffer_array_non_uniform_indexing` — under the plan's record-parity mandate that hits a `.expect()` on the absent `vb_cull_set`/pipeline (`targets.rs:4617-4645`). Add the conjunct at the dispatch site.
20. `VbCullReadbackLayout` **exists** (`vb.rs:260`, impl `:272`, ctor `:328-355`) and lives in `boyko_rhi_vulkan`, not `scene_types.rs` and not `boyko_app`; the layout is deliberately two-sided (`gpu_scene/mod.rs:197-199` ↔ `vb.rs:318-320`). The plan's `:971` row files it under the wrong file but `:973` carries the same work correctly.
21. `MeshGeometryTable::register` (`mesh_geometry_table.rs:577-589`) writes geometry and bounds together, and an unregistered slot resolves to the zero-triangle `VB_GEOMETRY_RESERVED_SLOT` (`mesh_draw.rs:513-517`). That coupling makes B4's sentinel case invisible **today** — it is incidental and guaranteed by nothing. Do not lean on it.

---

## 6. WHAT A SIXTH LENS WOULD HAVE FOUND

Five lenses audited claims, gates and controls. The unexamined axis is **the plan's own step ladder and its costs**. Six things nobody looked at:

1. **Only P3-0 gets a "green alone" argument.** The plan lands seven steps and states the gate set for each, but never asserts that each intermediate state is *self-consistently correct*. P3-1 widens the layout to 11 entries while "the SHADER still declares 7 bindings" (`:1028-1030`) — that is an intentional descriptor/module mismatch living in the tree for two commits, and `device.rs:345`'s debug_assert plus core validation both have opinions about it. P3-2 declares and records `vb_cull_late` against a module that has no phase-1 arm yet. Each step needs one sentence saying which gates it is expected to *move* and why the tree is shippable at that point — the discipline P3-0 gets and the other six do not.

2. **`VbCullUniform` has no write discipline, and the plan's only structural guard reads it on the frames it declines to write.** `:496-497` gates the fill off on unsplit frames ("an unsplit frame's cull reads only the planes, which are pushed"), while `:851`'s `level >= levels → KEEP` early-out — the guard that makes the `hzb_null` load safe — reads `levels` from that buffer. On a disarmed boot that is unwritten allocation contents. The fix is one unconditional `vkCmdUpdateBuffer` of ~100 bytes in a pass that already records one; the plan should either do that or state, per-field, which uniform fields are read on which frames.

3. **Nobody costed the allocations against the 25 goldens.** `vb_late_visible` is a **full duplicate** of the survivor array (`VB_LATE_VISIBLE_ELEMS == VB_VISIBLE_INSTANCE_ELEMS`, per FIF), `vb_cull_uniform` is per FIF, and `hzb_null` is a real `VkImage` minted on **every** `HzbMode::Off` VB boot — which is every golden. The plan's cost sections discuss barriers and dispatches and never once state bytes. On a campaign whose memory index records a disk-to-zero incident and a VRAM-deferred rung, "how much does the inert configuration now allocate" is a one-line answer the plan should contain.

4. **There is no disarm story.** Every prior piece shipped inert-by-default with a named off switch. Piece 3 arms at P3-4 behind `path_vb_occlusion_split()`, whose only production input is `OcclusionCulling` markers on scene instances. If the arming turns out wrong after the campaign moves on, the owner's only lever is un-marking geometry — there is no `HzbMode`-style config field for the split. State the production arming route and the disarm route explicitly, and say whether `VB_CULL_OCC_FORCE_KEEP` is a debug knob or the supported off switch (it is currently shipped shader surface that no gate sets — see M10).

5. **The no-perf-claim stance freezes the design before the only number that matters.** `:44-52` and `:1337` remove perf from scope twice, and the piece adds a second cull dispatch, a second raster scope, a second descriptor set and a per-frame uniform update to every armed frame. The campaign's own decidability floor is ~15% and its own memory records that per-batch culling rejected **zero** on the corpus because the granularity, not the test, was the bottleneck. Shipping the mechanism with no measurement at all risks piece 4 inheriting a two-phase architecture whose first measurement says the split costs more than it saves. Minimum ask: run FORCE_KEEP vs armed vs disarmed in one sitting with a zero control, and **publish the three numbers as prose** — not a pin, not a claim, just the measurement, so piece 4 starts from a number instead of a hope.

6. **Piece 3 retires a guard and does not name the replacement.** Adding `vb_batch_cull`'s `SHADER_WRITE` on `vb_indirect_late` (`:494`) means the P2-8 provenance guard can never again catch the P2-7-class omission on that buffer — it is first-touch only (fact 6). P2-7 measured all four gates green on that defect. The plan should say, in one line, what covers `vb_indirect_late`'s provenance after piece 3, or record that nothing does.
