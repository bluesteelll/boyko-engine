# Architecture: VG R3 piece 3 of 4 — the CULL INTEGRATION (the pyramid finally does something)

Status: **DESIGN, round 3.** Rounds 1 and 2 were REJECTED. Round 2 closed all four of round 1's
blockers — the round-2 critique verifies that, and this round does not re-open round 1's material.
**Round 3 is scoped to what round 2 itself ADDED and got wrong**, plus the majors and minors the
round-2 critique raises. Both critiques are preserved verbatim at the end of this file.

Scope fixed by `docs/OPEN-QUESTIONS.md` ("RESOLVED 2026-08-03 — decomposed"), piece 3 verbatim:
*the occlusion decision itself*. Pieces 1 and 2 are SHIPPED: the pyramid is built every armed frame
and read by nothing (`docs/VG-R3-P1-PYRAMID-PLAN.md`); the capability, the per-instance flag bit,
`vb_indirect_late` and a fully recorded LATE RASTER SCOPE that draws nothing exist
(`docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md`). The field survey is
`docs/VG-R3-TWO-PHASE-OCCLUSION-RESEARCH.md`.

> **Anchors.** Every `file:line` below was re-verified against the working tree on 2026-08-07, at
> commit `9e80cd4`. Round 1's header claimed the same and was false; round 2's claimed the same and
> was **also false in seven places**, listed in the refutation table below and fixed at every site.
> Anchors are therefore **name + hint**: grep the name, always. Where a critique cited an anchor that
> does not exist, or that says the opposite of what it was cited for, this round says so at the site.

## What round 3 changed, and why

| # | change | forced by |
|---|---|---|
| 1 | **A failed pyramid clear returns `Err`, never `Ok(None)`.** Round 2's "degrade to `Ok(None)` and let the arming predicate follow" is refuted by three shipped release-live `.expect()`s and by the very lockstep assert round 2 cited as its safety net. | R2-B1 |
| 2 | **`hzb_null` gets its OWN unconditional boot helper**, owned by `GBufferTargets::create`, landing at **P3-1** (the step that MINTS the image) and NOT inside `HzbTargets::build`, whose first statement is the armed-only 0%-gate. It is **cleared to `0.0`** as well as transitioned, so the disarmed load is safe by VALUE as well as by ADDRESS. | R2-B2 |
| 3 | **F-M4 becomes TWO controls.** F-M4a (record the fill after `cmd_dispatch`) is deterministic in SUBMISSION order and reds. F-M4b (the real record-order defect) has its expected column set to **GREEN / undetermined** and is published either way. The two contradicting sentences about barrier visibility are reconciled. | R2-M1 |
| 4 | **The `OpDot == 0` census pin is DELETED as unimplementable.** Replaced by a source-level sentinel gate on the projection body plus an artifact-level `NoContraction` decoration count; module-wide `op_dot` is **re-measured, never asserted 0**. | R2-M2 |
| 5 | **The mixed fixture gets a computable occlusion PRECONDITION** (clause 0, in two forms — analytic and measured), concrete geometry, two mesh registrations, and `0 < Σ n_keep < Σ n_defer` in place of `Σ n_keep == 2`. | R2-M3, R2-M9 |
| 6 | **`BOYKO_VG_SCENE` is a new ORTHOGONAL scene selector.** `BOYKO_VG_OCC` keeps its shipped `== "1"` predicate, which is what makes `vb_occ_mixed_off` producible at all. | R2-M4 |
| 7 | **The five readback lists get an EMITTER** in `format_vb_cull_probe_line`, per-batch grouped in `vis=`'s own shape, with a format→parse round-trip unit test. | R2-M5 |
| 8 | **G-P3-G gains two EXECUTED controls** and an explicit "if neither emits, strike the credit" clause. | R2-M6 |
| 9 | **Controls A1 and B2 corrected.** A1 is GREEN on `vb_occ_mixed_keep` by construction; B2 becomes `keep += 1` (the over-count class it was added to catch), and `keep -= 1` is FORBIDDEN as a device-loss hazard. | R2-M7, R2-M8 |
| 10 | **`BindGroupEntry` has TWO exhaustive matches**, so P3-1 touches `rhi_impl/mod.rs` as well as `rhi_impl/device.rs`. | R2-MINOR |

### What round 3 REFUTES, with the anchor that refutes it

| claim | refutation | anchor |
|---|---|---|
| R2-B2: *"`goldens/PINS.toml` has 25 sections"* | It has **26**. The substantive half stands — only one sets `BOYKO_VG_HZB` — but the arithmetic is **25 of 26 disarmed today**, not 24, and the plan's own "26 pins except two" was wrong in the other direction. | `goldens/PINS.toml` — 26 top-level tables at `:25, 46, 67, 95, 123, 163, 207, 240, 271, 309, 342, 397, 429, 473, 520, 554, 598, 650, 702, 738, 778, 808, 831, 870, 912, 945`; `BOYKO_VG_HZB` only at `:339`, `BOYKO_VG_OCC` only at `:394` |
| R2-B2: *"in the shape the existing boot-layout seeder uses"*, implying one exists in `targets.rs` | **There is no unconditional boot-layout submit in `GBufferTargets::create` at all.** Both boot-clear helpers sit behind conditional wrappers, and `seed_boot_layouts` lives in another crate. So `hzb_null` needs a **new** helper, and it costs an eleventh `DeferredSets::build` parameter plus one drain line in each of the three error arms — a cost round 2 denied and this round pays out loud (D7, Boundary). | `targets.rs:6279` / `:6687` (the two helpers), `:7234` / `:7300` (their conditional wrappers), `boyko_app/src/gpu_scene/csm.rs:365` / `:394-435` |
| R2-B1: *"`gpu_scene/mod.rs:4005-4012` records that rung R2d-2 already paid for exactly that second-arm-bit pair"* | Wrong anchor. The R2d-2 pair is `vb_visible_instance` (UNCONDITIONAL) against `vb_mesh_bounds` (ARMED). The finding's conclusion is unaffected — only its citation. | `gpu_scene/mod.rs:6488-6492` vs `:6496-6501` |
| Plan C13's second anchor: *"`mesh_draw.rs:1246-1249` already warns in those words"* | **No such warning exists in that file.** The substance survives on a different anchor: the gather is "recomputed from scratch (never accumulated across frames)". | `boyko_render/src/mesh_draw.rs:657-666` |
| Plan D11: *"`MeshGeometryTable::register` writes geometry and bounds together, so the sentinel is invisible today"* | The stated MECHANISM is wrong, and wrong in the unsafe direction: *"A slot that is never registered keeps the [`MeshLocalBounds::UNKNOWN`] prefill"* — and `VB_GEOMETRY_RESERVED_SLOT` is exactly such a slot. What makes the sentinel rare today is that the gather EXCLUDES non-resolvable meshes; a mesh that IS registered but never received a VB geometry slot reaches the ring **carrying the sentinel**. The conclusion ("do not lean on it") is strengthened. | `mesh_geometry_table.rs:587-588`, `:600-612`; `mesh_draw.rs:536`, `:657-661` |
| Plan D8: *"each named in its own assertion the way @4/@5 are at `vb_batch_cull_spv_sync.rs:331-339`"* | `:331-339` is ONE **joint** assertion (`binding_set.contains(&4) && binding_set.contains(&5)`), not one per binding. The prescription stands; the precedent does not. It is a NEW shape and is now labelled as one. | `vb_batch_cull_spv_sync.rs:331-339` |
| Plan D2: *"exactly `shadow_temporal_hist_read`'s shape at `graph_bridge.rs:3469-3476`"* | The example is at **`graph_bridge.rs:901-909`**. | that |

### Anchors that MOVED since round 2, all fixed below

`vb_batch_cull` pass `graph_bridge.rs:3881`→**`:3881`** (fill shape `:3882-3886`) · `vb_raster_late` `:4057`→**`:4058`** · `hzb_dump` pass `:4971-4973` · dev-profile note `:5071-5076` · `record_vb_pass` **`:5172-5353`** · P2-8 guard assert `graph.rs:703-704`→**`:703-720`**, latch `:721-723`→**`:722`** · `compile` reseed `:591`→**`:588-594`** · seed comment quote `sync.rs:308-310`→**`:309-310`** · last-writer branch `:373-383`→**`:372-383`** · `BindGroupEntry` enum `boyko_rhi/src/device.rs:346-405`, doc `:343-345`, `StorageImageView` doc `:355-357` · Vulkan arms `rhi_impl/device.rs:520` / `:533` / `:541` (round 2's `:426` and `:537-541` were wrong), entry-count assert `:345-348`, single `vkUpdateDescriptorSets` `:641` · push const-assert `rhi_impl/mod.rs:227-232`, "no device-limit query" `:205`, `VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE` `:221` · size-assert `scene_types.rs:549-551` · `cmd_draw_indexed_indirect` (late) `vb.rs:1821-1826` · late-array capacity assert `vb.rs:1189-1192`, its reason `:1186` · `renderArea` assert `:1746-1753` · `first_instance` asserts `:1065-1068` AND `:1220-1223` · three `.expect()`s `vb.rs:3439` / `:3783` / `:3841` · cull-readback block `runner.rs:2601-2621` (round 2 wrote `:2591-2621`), return at `:2619` · `frame_index` decl `runner.rs:987`, increment `:2790` · `late_instances == 0` clause `vb_occ_split_gate.rs:592-604` · survivor-ascending clause `vb_inst_cull_corpus.rs:411-419` · non-vacuity `hzb_engine_pyramid_gate.rs:559-573` · `a_dropped_writer_…` `vb_barrier_stream_baseline.rs:4356-4421`, its debug twin `:4434-4443`, the 19-message header note `:18-26`.

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

> On the `vb_occ_mixed` fixture, the image produced with the cull ARMED is **byte-identical** to the
> image produced with it DISARMED, while the GPU reports a **nonzero deferral count** and, under
> FORCE-LATE, a **nonzero late-survivor count**, in the same run.

Neither half alone is a gate. Byte-identity alone is satisfied by a cull that rejects nothing (this
campaign has shipped that failure six times). A count alone is satisfied by a cull that deletes
visible geometry. **The conjunction, measured in one sitting, is the gate** — and D12 states exactly
which half of the conjunction each fixture can carry, because on a converged static scene the
*correct* late-survivor count is ZERO.

**Explicitly NOT a performance claim, with one measurement obligation.**
`docs/VG-DECIDABILITY-FLOOR.md`'s own numbers (6.3 / 14.3 / 4.7 / 13.5 % across four runs of one
protocol) make any delta under ~15 % undefendable on this machine, and the committed corpus has no
occlusion-dominated scene. **No number is claimed and no benchmark is pinned.** Round 1 stopped
there; round 2 does not, because the sixth lens is right that a mechanism shipped with *no*
measurement hands piece 4 an architecture whose first number may say the split costs more than it
saves. P3-8 therefore runs **FORCE_KEEP vs ARMED vs DISARMED with a zero control in one sitting** and
publishes the three numbers **as prose in the commit message** — not a pin, not a claim, just the
measurement piece 4 starts from. For calibration only: the one published two-phase timing table
(Ubisoft, SIGGRAPH 2015, Xbox One 1080p) records the phase-2 draw at **< 0.01 ms**, i.e. on a scene
where the scheme works the late scope is nearly indistinguishable from not existing. That is the
regime this piece lands in, and D12 explains why that is a *gate* problem before it is a perf one.

---

## Context and constraints

### The six obligations piece 3 INHERITS, each written into the shipped code for this reader

| # | obligation | where it is written | discharged by |
|---|---|---|---|
| 1 | `vb_indirect_late`'s declared writer changes from `(TRANSFER, TRANSFER_WRITE)` to `(COMPUTE_SHADER, SHADER_WRITE)` | `scene_types.rs:3204-3206`; the P2-8 provenance guard at `graph.rs:692-724` | **D8** — and the writer that changes is `vb_cull_late`, NOT `vb_batch_cull` (change 1) |
| 2 | the late scope must declare `vb_instance_ring` and `vb_visible_instance` | `graph_bridge.rs:4049-4056` ("WHAT IS DELIBERATELY *NOT* DECLARED, and what piece 3 must add") | **D5 + D8** — with a SUBSTITUTION: the late scope binds `vb_late_visible`, not `vb_visible_instance` |
| 3 | two "PIECE 2 ONLY" tripwires deleted deliberately | `vb.rs:1224-1230` (`instance_count == 0`), `vb.rs:1802-1806` (indirection bit clear) | **D9** |
| 4 | the pyramid's cross-frame hazard, **re-labelled**: it is a cross-frame RAW with the WAR subsumed, not a WAR | `graph_bridge.rs:3497-3507` — which **prescribes the OPPOSITE seed** and is rewritten by this piece | **D2** |
| 5 | `hzb_dump`'s depth copy moves between the scopes, or both depths are dumped | `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` D6's hazard note; the copy at `vb.rs:3509-3541`; `graph_bridge.rs:4993-4998` | **D10** — both depths |
| 6 | the pyramid is read POINT-SAMPLED | `VG-R3-P1-PYRAMID-PLAN.md` §7 | **D7** — discharged STRUCTURALLY: no `VkSampler` is created |

### Hard local constraints, each re-verified in this tree

| # | constraint | anchor |
|---|---|---|
| C1 | **`vkCmdDrawIndexedIndirectCount` is not in the device fn table**, and adding it is a `VkPhysicalDeviceVulkan12Features` chain edit | `device.rs:615-618` |
| C2 | **`multiDrawIndirect` is off** ⇒ `draw_count ∈ {0,1}`; the only GPU-writable knob per draw is `instanceCount` | `vb.rs:1817-1820` |
| C3 | **The shared compute push range is const-asserted ≤ 128 B** and `VB_BATCH_CULL_PUSH_BYTES` is **104**. **24 bytes of headroom. A `float4x4` does not fit.** | `rhi_impl/mod.rs:202-232`, `compute.rs:1701`, size-assert `scene_types.rs:549-551` |
| C4 | **`robustBufferAccess` is OFF**; an out-of-bounds buffer read is silent corruption, and a GPU-written count word is the ONLY bound on the late draw and on the VS dereference | `gpu_scene/mod.rs:280`, inside the R2d-6 doc `:264-281` |
| C5 | **One device queue, one queue family**; no async compute exists | `device.rs:1160`, `:3261` |
| C6 | **The cull dispatches ONE LANE PER BATCH**, looping over that batch's instances serially | `vb_batch_cull.comp.hlsl:400-408`, `:441-466`; `groups` at `vb.rs:1329` |
| C7 | **`VB_VISIBLE_INSTANCE_ELEMS == INSTANCE_CAPACITY`** is an equality sound in BOTH directions | `gpu_scene/mod.rs:264-294` (const-assert `:282-288`, prose `:290-293`) |
| C8 | **INVARIANT R2d-REGION-DEFINED**: every reader of `vb_visible_instance` must be bounded by the same `k` the cull stores into record word 1, **or the tail must be filled** | `vb_batch_cull.comp.hlsl:118-153`, esp. `:150-153` |
| C9 | **INVARIANT R2d-REGION-DISJOINT**: bases strictly ascending, regions pairwise disjoint, established on the HOST | `vb_batch_cull.comp.hlsl:101-116` (round 1 transposed the two invariant names) |
| C10 | **`hzb_arm` is a STORED field** and part of the recreate predicate, so the pyramid's presence cannot flip inside one targets generation. ⚠️ It is a **check**, not a mechanism that absorbs a failure — see C19/C20 | `targets.rs:672`, assigned `:7718`; lockstep assert `:7781-7785`; the shipped contract in words `:7714-7716` |
| C11 | **Dense components are pinned `ResidencyKind::Cpu`** | `boyko_macros/src/component.rs:544-546`; `docs/DENSE-COMPONENTS-PLAN.md:61` |
| C12 | **`GpuColumnManager` has ZERO production call sites** | `gpu_column.rs:639-706` and its eight test callers |
| C13 | **The VB ring index is NOT stable across frames** — the gather is rebuilt from scratch every frame, so a mesh leaving `Loaded` shifts every later slot. ⚠️ Round 2 cited `mesh_draw.rs:1246-1249` as "already warns in those words"; **no such warning exists**. The property is anchored on the gather's own doc instead | `archetype.rs:1019-1057`; `boyko_render/src/mesh_draw.rs:657-666` (*"recomputed from scratch (never accumulated across frames)"*), gather `:672-956` |
| C14 | **`HzbTargets::build` issues NO layout transition** — `create_texture` + views + sets only, no encoder, no barrier, no submit. The pyramid's only layout producer is the framegraph first touch. | `targets.rs:1252-1423`; the doc says so in words at `:1147-1149` and `:1202-1204` |
| C15 | **`BindGroupEntry::SampledImage` hard-writes `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL`**; `StorageImage`/`StorageImageView` write `GENERAL`. The kind implies the layout | `boyko_rhi/src/device.rs:343-345` (the contract in words), enum `:346-405`, `StorageImageView` doc `:355-357`; Vulkan arms `rhi_impl/device.rs:520` / `:533` / `:541` |
| C16 | **`occlusion_split && !batch_cull_armed` is REACHABLE** on a device without `storage_buffer_array_non_uniform_indexing` | `runner.rs:551`, `render_path_config.rs:952-954`, `runner.rs:607`/`:630-631`/`:2355-2356`, `vb.rs:964-969` vs `scene_types.rs:3587-3591` |
| C17 | **A pass's ENTIRE barrier set emits at ONE site.** There is no comment saying so; the mechanism is `graph.rs:621-623` + `:913-918` (`PassBarrierRange`) + `record.rs:50-56`. | those three |
| C18 | **The P2-8 provenance guard is first-touch and `is_write`-latching**: `debug_assert!(is_write \|\| self.res_written[..])` at `graph.rs:703-704`, latch at `:721-723`. A combined `SHADER_READ\|SHADER_WRITE` never tests the read half. | `graph.rs:692-724` |
| C19 | **`HzbTargets::build`'s ONLY `Ok(None)` is the 0%-gate, and it is the FIRST statement** — before any encoder, fence or barrier exists. Any helper folded into that function is **armed-only by construction**. The body contains no `create_command_encoder`, no `create_fence`, no `queue.submit`, no barrier | `targets.rs:1252-1257` (signature, `-> Result<Option<Self>, SwapchainError>`), `:1258-1262` (the gate), `:1252-1423` (the whole body) |
| C20 | **A silent `None` is a RELEASE PANIC, not a degrade.** Three release-live, per-frame, unconditional `.expect("invariant: scene.hzb armed => targets.hzb (sync_gbuffer's hzb_arm predicate)")` gate on the SCENE side, and the tree states the contract in prose: *"a create failure returns `Err` rather than a silent `None`"* | `vb.rs:3439`, `:3783`, `:3841`; prose `vb.rs:3740-3751` (quote at `:3746-3747`), echo `:1273` |
| C21 | **`GBufferScene::hzb` is the host PLAN, not the allocation.** Computed once per frame in the runner and threaded into the scene struct; **no runtime failure can flip it**, so no predicate derived from it can disarm anything in response to one | plan built `runner.rs:2304-2308`; field decl `gpu_scene/mod.rs:5697`, set `:6563`; "no image, no views, no passes" `:6560-6562` |
| C22 | **There is NO unconditional boot-layout submit in `GBufferTargets::create`.** Both boot-clear helpers are reached only through conditional wrappers, and the one unconditional precedent lives in another crate. A new helper is required and it cannot be free | `targets.rs:6279` (`boot_clear_taa_hist`, uncfg'd but called under `if scene.taa.is_some()` at `:7300`), `:6687` (`boot_clear_shadow_temporal_hist`, `#[cfg(feature = "hwrt")]` at `:6686`, called under a caps check at `:7234`); the unconditional precedent `boyko_app/src/gpu_scene/csm.rs:394-435`, called at `:365`, reason `:8-10` |
| C23 | **DXC inlines every helper into `%main`: exactly ONE `OpFunction` in the committed `vb_batch_cull.comp.spv`.** There is no artifact-level function range to scope a SPIR-V census to — not "unbuilt", *unreachable* | byte-scan of the committed `.spv`; the census's own flat token loop `vb_batch_cull_spv_sync.rs:213-260` |
| C24 | **Synchronization validation is NOT live on this machine, MEASURED.** A genuine missing RAW between two compute dispatches produced the unchanged 19-message baseline, no `SYNC-HAZARD-*`, and a byte-identical golden. The entire baseline is `vkCreate*`-time | `docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1642-1676` (table `:1652-1655`, conclusion `:1657-1659`); test header `vb_barrier_stream_baseline.rs:18-26` |
| C25 | **`record_vb_pass` records BARRIERS ONLY.** The dispatch is recorded by the caller with nothing in between, so "move the fill past the barrier" does **not** move it past the dispatch — which is what kills round 2's F-M4 prediction (D6) | `graph_bridge.rs:5172-5353` (sink + `record_pass` at `:5352`); `framegraph/record.rs:50-56`; the caller pair `vb.rs:1301` → `:1362` |
| C26 | **`format_vb_cull_probe_line` is the ONLY serializer of `VbCullReadback`**, its file write the only channel to the driver process, and the parser's `field()` PANICS on a missing key. It already emits three LIST-valued keys with per-batch grouping, so the shape piece 3 needs exists | `runner.rs:2924-2954` (emit `:2949-2953`, group join `:2952`), file write `:2613-2618`, grouping rationale `:2905-2916`; parser `vb_inst_cull_scene/mod.rs:565-599`, panic `:569-571` |
| C27 | **Batches bucket per `MeshHandle`.** Batch count is a mesh-REGISTRATION property of a fixture, never an instance-count property — `vb_mesh.rs` registers exactly ONE mesh today | `boyko_render/src/mesh_draw.rs:672-956` (count pass `:894-910`, prefix-sum + `DrawBatch` emission `:922-955`); `vb_mesh.rs:117-120` |
| C28 | **`select_texels` returns the ALIGNED expansion**, `containing_texel(t, level) = t >> level`, and `conservative_min` folds all four texels. With the reverse-Z clear at `0.0`, **one background texel anywhere in the footprint forces KEEP**. At 512×512 `base == source`, so a rect straddling the midline selects the top level and the footprint is the whole image | `hzb.rs:300-302`, `:790-808` (selector `:797`), `:817-825`; clear `vb.rs:52` (`VB_DEPTH_CLEAR = 0.0`) |
| C29 | **The framegraph exposes no per-pass access count.** `pass_access_count` is a private field and no accessor exists in the public surface, so a declarator-side access-count assert would need new framegraph-core API. Not available to this piece | `graph.rs:158` (private); public surface `:257-1254`, eighteen `pub fn`, none exposing it |

### Invariants that must survive untouched

| invariant | anchor |
|---|---|
| `first_instance == 0` in every record (`drawIndirectFirstInstance` is VK_FALSE) | asserts `vb.rs:1065-1068` (early) and `:1220-1223` (late) — **both survive**; only `:1224-1230` is deleted |
| R2d-REGION-DEFINED / R2d-DISJOINT on `vb_visible_instance` | C8 / C9 |
| R2d-EXPORT-IS-GLOBAL — the VS exports the GLOBAL instance index into `vb_id` | `vb_raster.vs.hlsl:98-107`, `:209` |
| `hzb_poison` before every `hzb_build`; every `hzb_build` before `hzb_dump`; `poison.is_some() == dump.is_some()` | the trio in `declare_vb_graph`'s tail asserts |
| declare/record ORDER parity, and the recorder's gate is the declarator's VERBATIM | `vb.rs:1155-1161` (the late-upload site, verified verbatim), `:3420-3426` (the `hzb_dump` site); the `batch_cull_armed` pair is `vb.rs:964-969` against its byte-identical twin at `graph_bridge.rs:3764-3769` |
| the late scope's `renderArea` equals the early scope's | `vb.rs:1746-1753` |
| the unknown-bounds sentinel guard is the OUTER branch of the per-instance loop | `vb_batch_cull.comp.hlsl:83-99` (the header states why, naming a prior critic; the load-bearing sentence is `:91-92`), shipped loop `:441-466`, guard line `:449`, frustum call `:458` |

---

## Key decisions

### D1 — The EARLY predicate: an HZB re-test against the pyramid AS THE PREVIOUS FRAME LEFT IT, with the CURRENT frame's transforms and view-projection. **Not** a stored per-object visibility bit.

**What.** The early cull, after its existing per-instance frustum test, applies the occlusion test to
every instance whose `VbInstanceRow.flags` bit 0 is set. The pyramid it reads is the one this frame's
`hzb_build` has *not yet* overwritten — i.e. frame N−1's content. An instance the test rejects is not
dropped: it is appended to the batch's **late candidate** list.

**Why not niagara's stored visibility bit — a structural refutation, not a preference.**
The research records the fork as a genuine disagreement between shipped codebases (niagara and
Granite store a bit; Nanite and Bevy re-test against the previous pyramid). **The tree closes it:**

- A per-object bit is **durable per-entity data**, so Principle 0 puts it in the ECS's own storage.
- **The only storage with a stable per-entity row is `Dense`, and `Dense` is force-pinned
  `ResidencyKind::Cpu`** (C11). The bit's producer is the GPU.
- **The only GPU-resident mechanism is `GpuColumn`, which requires TABLE storage** — whose rows
  `swap_remove` on despawn — **and has zero production call sites** (C12).
- Keying the bit by the VB ring index instead is wrong on its own terms: **the ring index is not
  stable across frames** (C13), and `mesh_draw.rs:1246-1249` already warns in those words.
- Making it right means minting a stable per-instance GPU key, a durable buffer indexed by it, and a
  GPU→CPU writeback or a new GPU-writable column mechanism. **Larger than the cull itself.**
- Nanite's own justification (continuous LOD + streaming make cluster IDs unstable) does not apply
  here — but the *conclusion* does, for a different reason: this engine's IDs are unstable because
  the gather rebuilds them.

**What the previous-pyramid predicate COSTS.** niagara's stored bit removes the cross-frame HZB read
entirely. Choosing this predicate takes on the cross-frame hazard as a real obligation. **D2
discharges it, and D2 exists because of this decision.**

**What it does NOT cost: a second pyramid build.** Bevy builds the pyramid twice per frame. This
design builds it **once**, between the two raster scopes, and reads it twice — stale by the early
phase, fresh by the late phase. A second build after the late scope would be pure loss: **nothing in
this engine consumes a pyramid of the final depth.** A hybrid of Bevy's predicate and niagara's build
schedule, cheaper than either.

**Why the current view-projection against a stale pyramid is SOUND.** It is not a coherent query
about anything — the pyramid's texels belong to the previous camera. Correctness does not rest on it:

> **INVARIANT VG-P3-RECOVERY.** Every instance the early phase rejects is a member of that batch's
> late candidate list, and the late phase re-tests it against the pyramid built from THIS frame's
> early depth with THIS frame's view-projection. The early phase therefore *partitions* the frustum
> survivors and never removes one.
>
> **Gated by** G-P3-B clause 2 (`k + n_defer ==` the CPU census's frustum-survivor count) **and**
> clause 2b (the early survivor list and the candidate list are DISJOINT and their union is exactly
> that survivor set). Round 1 stated this invariant and left it un-clauses; clause 2b is the fold.

So an early false-reject costs a late draw and nothing else. Nanite's recorded weakness of the same
class — "when turning the camera a slice of the screen near the edge will have no previous-frame data
to be culled against" — is a hit-rate statement, not a correctness one, for exactly this reason.

**Why the LATE test is exactly sound**, which is the half that can delete geometry:

> The late test rejects instance *i* iff *i* is occluded by the set of instances drawn in the EARLY
> scope, at their current positions. Occlusion by a SUBSET of the scene implies occlusion by the
> scene. Therefore a late reject is genuinely invisible.

**Why not the PREVIOUS view-projection** (Bevy std-mesh / Nanite, which reproject): it needs a second
`float4x4` and a definition of "previous" that survives resize, path switches and the first frame,
for a difference that changes only the early phase's hit rate — a quantity this machine cannot
measure to better than ~15 %. The current view-projection is **already in hand**: the frustum planes
are extracted from the same 64 push bytes the raster VS reads (`gpu_scene/mod.rs:6480-6484`).

**Convergence.** Let *E* be the early-drawn set and *P(E)* the pyramid built from its depth. Next
frame's early set is `E' = {i : ¬occluded by P(E)}`. Starting from a pyramid of `0.0` — the reverse-Z
far plane, which rejects nothing — frame 1 has `E =` all frustum survivors, so `P(E)` is the true
depth and `E'` is the visible set from frame 2 onward. **One frame to converge, with no special case
in any shader.** ⚠️ **On frame 1 the cull provably rejects NOTHING**, and from frame 2 the system sits
at a fixed point whose properties are the subject of **D12**. Both facts are gate hazards; both are
turned into gate *content* there.

**Trade-off.** The occlusion test runs TWICE per marked instance on a frame where the early phase
rejects it, once otherwise: ~8 corner projections plus 4 image loads, on a per-batch lane (C6), i.e.
serially within a batch — an inherited property of R2c0's dispatch shape, named in Boundary.

---

### D2 — The pyramid becomes a CROSS-FRAME resource: a BOOT CLEAR to `0.0` through a real encoder, and only then the seed `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)`

This is obligation 4. ⚠️ **`graph_bridge.rs:3497-3507` predicted the OBLIGATION, not the answer** —
round 1 claimed it "predicted the answer verbatim", and it does the opposite: it prescribes
`seeded_readers_at_layout(GENERAL, COMPUTE, SHADER_READ)`. That comment is **wrong** and is rewritten
in the same commit that changes the seed, at the site P3-0 already edits.

**Why a READER seed is wrong, and this is the argument round 1 never gave — the TWO RESIDUALS.**
The seed describes the state a frame *ends* in, and piece 3 produces two different endings:

| frame kind | the pyramid's LAST access | what next frame's FIRST access needs |
|---|---|---|
| armed-split | `vb_cull_late` COMPUTE `SHADER_READ` (or `hzb_dump` TRANSFER_READ) | next frame's `hzb_build_0` WRITE must WAR against it |
| HZB-armed, split OFF (`[vb_mesh_hzb]`) | `hzb_build_{n-1}` COMPUTE `SHADER_WRITE` | next frame's first read must RAW against it |

A reader seed makes the first case right and the second case **silently wrong**: the next frame's
early-cull read derives against a seed that says "already visible", which `sync.rs:308-310` names in
the engine's own words — *"the reader WAR seed would leave the read FREE/already-visible, which is
exactly the race"*. A **writer** seed makes the second case exactly right and the first case
*conservative*: the next frame's first write derives a WAW where a WAR would have sufficed. **Only
the writer form is conservative for both residuals**, and both residuals are reachable in the shipped
pin set. `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` is exactly
`shadow_temporal_hist_read`'s shape at **`graph_bridge.rs:901-909`** — round 2 cited `:3469-3476`,
which is not that call. The seed helper itself is `sync.rs:312-321`, and `seeded_readers_at_layout`,
the form the stale comment prescribes, is `:270-279`.

**Why `ResSync::undefined()` becomes WRONG the moment a reader exists.** A first touch derives
`oldLayout = UNDEFINED`, which **licenses the driver to discard the image contents**. Frame N+1 would
read an image the graph just told the driver it may throw away — content- and motion-dependent,
verbatim the engine's recorded "wrong only in motion, stable when stopped" fingerprint.

**The chain, on an armed-split frame** (`vb_batch_cull` at `graph_bridge.rs:3881`, `vb_raster` at
`:3949`, the poison+build block between the scopes at `:4024-4029`, `vb_raster_late` at `:4058`):

```
vb_batch_cull        COMPUTE  SHADER_READ    <- frame N-1's content
hzb_poison           TRANSFER TRANSFER_WRITE (dump frames only)
hzb_build_*          COMPUTE  SHADER_WRITE / SHADER_READ
hzb_dump_depth_early TRANSFER TRANSFER_READ  (dump+split frames only, D10 — depth, not pyramid)
vb_cull_late         COMPUTE  SHADER_READ    <- this frame's content        (NEW, D4)
hzb_dump             TRANSFER TRANSFER_READ  (dump frames only)
```

- **Cross-frame RAW** (frame N's `hzb_build_{n-1}` write → frame N+1's `vb_batch_cull` read): carried
  by the seed.
- **Cross-frame WAR** (frame N's `vb_cull_late` read → frame N+1's `hzb_poison`/`hzb_build_0` write):
  subsumed. Frame N+1 derives an intra-frame WAR against its OWN `vb_batch_cull` read, and the Vulkan
  spec is explicit that a `vkCmdPipelineBarrier` recorded **outside a render pass instance** has a
  first synchronization scope including all commands earlier in **submission order** — defined across
  `vkQueueSubmit` calls, `VkSubmitInfo`s and command buffers — which includes frame N's commands,
  because **there is exactly one queue** (C5).
- ⚠️ **The one residual, stated because round 1 left it unstated.** On a DUMP frame the pyramid's last
  access is `hzb_dump`'s `TRANSFER_READ`. No derived `srcStageMask` can ever name TRANSFER for the
  next frame's first write: `seeded_writer_at_layout` sets `visible_stages = 0` (`sync.rs:312-321`,
  the field at `:319`) and the last-writer branch at `sync.rs:372-383` sources from there. So a dump frame's read is unordered against the
  next frame's poison write. The exposure is **one frame, on a diagnostic path, and is strictly
  IMPROVED by D2** — today's `undefined()` seed gives `TOP_OF_PIPE` *plus* a licensed content
  discard. It is value-invisible on the static corpus because the next frame reproduces the same
  bytes. A both-halves seed would close it (`sync.rs:406-413` shows a first READ ORs `visible_stages`
  with no `transition` change); it is **not taken**, because the writer-only form is what the
  two-residual argument selects and a hybrid seed has no second consumer to justify it.

**Frames-in-flight does not enter the argument, and that is the crux.** `FRAMES_IN_FLIGHT` exists to
let the HOST re-record and rewrite host-visible data without waiting. The pyramid is never touched by
the host. Its only producers and consumers are queue operations, ordered by submission order and the
barriers above. This is why niagara ships one `depthPyramid` with three frames in flight, and Bevy
one `ViewDepthPyramid` per view.

**The premise is named so it can be re-derived when it stops holding: this argument is valid only
while the engine submits every pass to ONE queue, and only while every one of these barriers is
recorded OUTSIDE a render pass instance. The day async compute lands (Pillar A's Phase 3, not built),
submission order stops being a total order and the pyramid must be re-examined or ringed.**

**Why not ring the pyramid per FIF.** 2× VRAM (~2.8 → ~5.6 MB at 1920×1080) and — the real cost — it
doubles `HzbTargets::level_views`, `HzbTargets::sets` and every declared span, and makes `hzb_dump`
ambiguous about which slot it copied. Rejected: the barrier argument is rigorous under a premise that
is *stated and checkable*. Its one genuine benefit is cross-frame overlap, a performance property
this campaign cannot measure.

#### The boot clear, and why round 1's version was unbuildable

⚠️ **`HzbTargets::build` issues NO transition** (C14). It calls `create_texture`, a view loop and a
set loop — no encoder, no barrier, no submit; `VulkanTexture::create` takes no queue and cannot
transition. Round 1 asserted that `build` "already issues a one-shot `UNDEFINED → GENERAL`
transition" and hung the seed on it. It does not, and the plan contradicted itself two paragraphs
later by correctly naming the framegraph first touch as today's only producer. **Flipping the seed
without a real clear would emit `oldLayout = GENERAL` (`sync.rs:353` computes `layout_change =
false`; `:389-390` emits it) against an image genuinely in `UNDEFINED` —
VUID-VkImageMemoryBarrier-oldLayout-01197 — for the entire life of every `HzbTargets` generation,
i.e. after every resize, not once at boot.**

**The specified shape: `boot_clear_hzb_pyramid`, modelled statement-for-statement on
`boot_clear_shadow_temporal_hist` (`targets.rs:6687-6773`)** — whose final-barrier comment at
`:6740-6743` states the motive in the engine's own words: *"the GENERAL layout also satisfies the
… framegraph seed's GENERAL-layout assumption."* That is this decision, already shipped once.

```
create_command_encoder                                   (targets.rs:6692)
create_fence(false); on Err destroy the encoder, return  (:6693)
range = COLOR, mips [0, plan.levels), 1 layer
begin
  UNDEFINED -> TRANSFER_DST_OPTIMAL   TOP_OF_PIPE->TRANSFER, NONE->TRANSFER_WRITE   (:6716-6727)
  clear_color_image(pyramid, TransferDstOptimal, [0.0, 0, 0, 0], range)             (:6731-6738)
  TRANSFER_DST_OPTIMAL -> GENERAL     TRANSFER->COMPUTE_SHADER, TRANSFER_WRITE->SHADER_READ  (:6744-6755)
end; queue.submit(&encoder, &fence)  (:6759); wait_fence(u64::MAX)  (:6760)
teardown ladder: destroy_command_encoder + destroy_fence on EVERY path              (:6768-6771)
```

⚠️ **`hzb_null` is NOT transitioned here, and round 2 said it was.** This helper lives behind
`HzbTargets::build`, whose FIRST statement is the armed-only 0%-gate (C19), while `hzb_null` is
minted and bound on EVERY boot. Folding the two would leave `hzb_null` in `UNDEFINED` under a
descriptor recording `GENERAL` on 25 of the 26 committed pins. `hzb_null` gets its own
**unconditional** helper, specified in D7, landing at the step that mints it.

**Four reasons, and the FIRST is the one round 1 did not have:**

1. **The clear is what makes the seed's `GENERAL` claim true.** Without it the seed is not merely
   unhelpful, it is a lie the validation layer can (and at P3-6 will) name. The seed and the clear
   are one change and land in one commit.
2. It removes a read of uninitialised image data. Vulkan bounds image accesses unconditionally, so
   this is not a fault — but the VALUE is undefined, and a large undefined value under reverse-Z is a
   near occluder that rejects everything in its footprint.
3. `0.0` is the far plane, so an unbuilt pyramid **provably rejects nothing** — "the pyramid always
   holds a conservative lower bound over its footprint" becomes true from birth instead of frame 2.
4. It makes convergence one frame (D1), which keeps a resize from producing a late-draw spike.

**Degrade policy: a failed clear returns `Err`. Round 2 said `Ok(None)` and that is refuted by the
shipped code in the engine's own words.**

`build_and_clear_taa_hist` degrades to `None` on any encoder/submit/fence failure
(`targets.rs:6251-6273`, the `None` at `:6270`) because TAA-off is byte-identical. **The pyramid
cannot degrade that way, and not for the reason round 2 gave.** Round 2 said a second `Ok(None)`
producer would be absorbed by `hzb_arm`'s lockstep assert and by `path_vb_occlusion_split()`'s new
`hzb.is_some()` conjunct. Both claims are false:

- `hzb_arm` is captured from the SCENE (`targets.rs:7718`), and the lockstep assert
  (`:7781-7785`) compares it against the ALLOCATION. A second `Ok(None)` whose precondition is
  `scene.hzb == Some` makes those two disagree — **the assert cited as the safety net is the thing
  that FIRES.** The comment three lines above it already states the shipped contract:
  *"the stored bit and the allocation can only disagree if the build fails — which returns"*
  (`:7714-7716`).
- Release is worse than debug. `vb.rs:3439`, `:3783` and `:3841` are release-live, per-frame,
  unconditional `.expect("invariant: scene.hzb armed => targets.hzb …")` (C20). The degraded
  generation would panic **every frame, in every profile, at P3-0** — before D9's conjunct or D5's
  set selector exist.
- The conjunct disarms nothing even once it exists. `GBufferScene::hzb` is the host PLAN, computed
  once in the runner (C21); no runtime failure can change it. `hzb.is_some()` would still be `true`
  and the `GENERAL` seed would stand over a null image.

⇒ **A failed clear tears down through the existing reverse-acquisition ladder and returns `Err`
from `HzbTargets::build`** — the shape `targets.rs:1236-1251`'s own doc already describes ("Reverse-
acquisition draining on partial failure, in two stages") and `vb.rs:3746-3747` already relies on.
**`Ok(None)` stays reserved for the 0%-gate at `targets.rs:1258-1262`** and acquires no second
producer. The lockstep assert is the CHECK on that discipline, never a mechanism that absorbs a
failure.

**What that costs, stated rather than hidden.** A device on which a 2.8 MB clear submit fails now
fails the whole targets build — i.e. boot or resize returns `Err` and the caller's existing error
path runs. That is the same class as every other `create_*` failure in `GBufferTargets::create`, and
it is strictly better than the alternatives: a silent `None` is a guaranteed release panic (C20), and
"seed `GENERAL`, skip the clear" is VUID-VkImageMemoryBarrier-oldLayout-01197 for the life of the
generation.

**If a real disarm is ever wanted, say what it costs.** It is not a wording change. `hzb_arm`,
`path_vb_occlusion_split()`, D5's set selector and D2's graph seed would all have to move off
`scene.hzb` (the host plan) and onto an **allocation-backed** flag, plumbed from `GBufferTargets`
back into the scene the recorder reads. That is a second arm bit of exactly the kind rung R2d-2
already paid for once (`gpu_scene/mod.rs:6488-6492` unconditional vs `:6496-6501` armed), and it is
**larger than this piece's Boundary allows**. It is recorded in Open Questions, not taken.

⚠️ **Mandatory unit test, because a policy nobody executes is a comment.** A test forces the clear
to fail (an injected `create_fence`/`submit` error at the seam) and asserts the resulting
configuration is SELF-CONSISTENT: `HzbTargets::build` returns `Err`, no `GBufferTargets` is
constructed, and therefore no lockstep panic and no `GENERAL` seed over a null image is reachable.
Its control is the round-2 shape: make the same failure return `Ok(None)` and show the lockstep
assert at `targets.rs:7781-7785` fire.

⚠️ **It does not weaken G8/G5.** `hzb_poison`'s `-1.0` is per-dump-frame and runs after the boot
clear; the poison argument (`VG-R3-P1-PYRAMID-PLAN.md` §14) is untouched.

**Trade-off, stated.** The seed change moves the derived barrier stream on **existing**
configurations: today's `vb_mesh_hzb` frame derives `UNDEFINED → GENERAL` at `hzb_build_0`'s first
write; it will derive a `GENERAL → GENERAL` WAW against `(COMPUTE, SHADER_WRITE)`. G-P3-F's U-rows
move, deliberately, and are re-pinned in the same commit with the reason in the commit message. This
is the ONE place piece 3 changes a stream on an unsplit frame, and P3-0 lands it alone.

---

### D3 — The LATE input is a COMPACTED early-reject list in its OWN buffer `vb_late_visible`, and its COUNT lives in its OWN per-batch array `vb_late_count` — **not** in the draw record

**Why a compacted list rather than re-scanning all instances.** niagara re-scans all N with a
different predicate. **That is not expressible here**: the early predicate reads the PREVIOUS pyramid,
and by the time the late cull runs the pyramid has been overwritten by this frame's build. The late
phase cannot recompute what the early phase decided. (niagara escapes this only because its early
predicate is a stored bit, which D1 rejects.) So the early verdict must be *stored*, and a compacted
list of ring indices is strictly better than a per-instance flag array: smaller, it is what the late
phase iterates, and it doubles as the output.

**Why a NEW buffer rather than two-ended packing inside `vb_visible_instance`.** Piece 2's D5 fixed
the *budget* (`|early| + |late| ≤ instance_count`) and left the *packing* to piece 3 while saying
"the late scope gets no survivor list of its own, ever". **This plan takes the budget and rejects the
never**, for gate-quality reasons:

| | two-ended in `vb_visible_instance` | separate `vb_late_visible` (chosen) |
|---|---|---|
| VRAM | 0 new | `INSTANCE_CAPACITY` u32s × FIF — the same byte count as `vb_visible_instance` |
| descriptor bindings | 0 new | +1 on `vb_cull_layout`, +1 set (`vb_set0_late`, D5) |
| **`vb_raster.vs.hlsl`** | **must gain a DESCENDING index path and a third flags bit** (`visible[anchor − 1 − id]`) | **byte-unchanged** |
| in-place compaction | reads `[base+count−1−j]`, writes `[base+count−1−keep]`; safe only by a non-obvious lemma | reads `[base+j]`, writes `[base+keep]`, `keep ≤ j` — **one line** |
| `vb_visible_instance`'s R2d invariants | a second reader indexing the region by something other than a compacted `SV_InstanceID` — exactly the case `vb_batch_cull.comp.hlsl:150-153` says "do not weaken" | **literally untouched** |
| the readback probe | observes the LATE partition; the early partition is destroyed | observes BOTH, in separate regions |

**The decisive one is row 3.** Piece 3 is the first piece whose change is supposed to move pixels. If
`vb_raster.vs.hlsl` also changes, a pixel diff cannot separate "the cull rejected wrongly" from "the
VS indexes wrongly". **Keeping the rasteriser's `.spv` byte-identical means every pixel change is
attributable to the cull alone** — a gate-quality argument, not a convenience one.

#### ⚠️ The count does NOT go in `vb_indirect_late[b].instanceCount`. Round 1's D4 said it did, and that decision was the piece's deepest defect.

Round 1 had the early phase write `n_defer` into the late draw record and the late phase overwrite it
with `n_keep`. Three consequences, all fatal to the gates:

1. **D9's replacement assert became false.** "The late cull is the ONLY producer of a nonzero value
   in this array" is refuted by the early phase on the same page.
2. **The safety property inverted.** A frame whose late cull did not run would draw `n_defer`
   **untested** instances, not a blank scope — the opposite of what round 1 claimed the assert bought.
3. **No image-level control for "the late phase is load-bearing" could exist.** Deleting the late
   cull draws a SUPERSET of the correct set at identical depth, and a redraw at identical depth is
   pixel-invisible under `VK_COMPARE_OP_GREATER` with no `discard` and no `SV_Depth`
   (`vb_occ_split_gate.rs:51`). Every image gate stays green on a deleted late cull, forever.

⇒ **`vb_late_count`**, a per-batch `u32` array sized like `vb_indirect_late`'s record capacity plus
one reserved tail slot. The early phase writes `vb_late_count[b] = n_defer` and **does not touch the
record**. The late phase reads it, compacts, and writes `vb_indirect_late[b].instanceCount = n_keep`.
That restores all three properties: the host seed of `0` is the truth, a missing late cull draws
**nothing**, and A3 becomes a real image control on the FORCE-LATE fixture (Gates).

Round 1 rejected this exact array as "one more buffer, one more binding and one more region
invariant, to avoid a read-modify-write of a word this pass already owns". **The price was three
gates. One binding is cheaper.** The region invariant is trivial — the array is indexed by batch id
`b`, the same index `vb_indirect_late` uses, with no base arithmetic at all.

**Two properties the split buys that are not merely restorations:**

- **`vb_late_count`'s first in-graph touch is `vb_batch_cull`'s `SHADER_WRITE`**, so the P2-8
  provenance guard (C18) is **LIVE** on it: deleting that declaration reds a `debug_assert` in every
  dev-profile golden run (`graph_bridge.rs:5071-5076`). It is the only new buffer in this piece the
  guard can protect, and it partially replaces what piece 3 retires elsewhere (D8).
- **The reserved tail slot `vb_late_count[capacity]`** carries the frame index the GPU actually
  observed in `VbCullUniform`, written by batch lane 0 in phase 0. That is the only executable
  control for D6's record-order hazard (M4) — see D6.

**The region rule for `vb_late_visible`**, a fresh instance of the same shape, not a weakening of C8:

> **INVARIANT VG-P3-LATE-REGION.** Batch `b` owns `[base_instance_b, base_instance_b + instance_count_b)`
> of `vb_late_visible` and writes nowhere else — the same host-established disjointness C9 gives
> `vb_visible_instance`, from the same `VbBatchDesc` fields. The EARLY phase writes
> `[base, base + n_defer)`; the LATE phase reads that prefix and writes `[base, base + n_keep)` with
> `n_keep ≤ n_defer`. The only dereferencing reader is the late raster's VS, bounded by
> `SV_InstanceID < instanceCount = n_keep`. No tail fill is required, for verbatim C8's reason.

`VB_LATE_VISIBLE_ELEMS == INSTANCE_CAPACITY`, const-asserted against `VB_VISIBLE_INSTANCE_ELEMS`, so
`vb_cull_batch_count_visible_clamp` (`vb.rs:236-244`) bounds BOTH lists with the one number it
already computes. **C7's equality is untouched in both directions.** ⚠️ Per mechanical fact 2, the
late array's size is **asserted, never folded into the `.min()` clamp chain** — `vb.rs:1186-1192`
states why in words ("a late array SHORTER than the early one would silently drop the tail batches").
The runtime backstop is `debug_assert!(late_visible[fi].size / 4 >= visible_elems)`, matching the
existing assert at `:1189-1193`.

**Overflow is impossible, a genuine divergence from the field.** niagara drops draws past
`TASK_WGLIMIT` and Nanite drops clusters past `MaxCandidateClusters`, surfacing as blinking geometry.
Here every list is exactly the size of the region it partitions (`n_defer + k ≤ instance_count`), so
there is no overflow policy to get wrong and no drop to make visible.

---

### D4 — The LATE dispatch is FIXED and HOST-SIZED: `batch_count` lanes, the SAME shape as the early one. No `vkCmdDispatchIndirect`, no `…Count`, no readback in the loop.

Because the cull dispatches **one lane per batch** (C6) and `batch_count` is a host number computed
once at `vb.rs:1021-1026`, the late dispatch's size is known on the host. The GPU-only quantity is
the *per-batch* candidate count, which the lane reads from `vb_late_count[b]` (D3).

⇒ **Adding `vkCmdDrawIndexedIndirectCount` (C1) or `vkCmdDispatchIndirect` is OUT OF SCOPE and NOT
NEEDED.** Stated flatly so it cannot be re-litigated. Bevy's `atomicAdd(work_item_count); if (i % 64
== 0) atomicAdd(dispatch_x)` is the cheapest published way to size an indirect dispatch from an
append list — and it is machinery this design does not require, because the domain of the late
dispatch is BATCHES, not survivors. The late scope records `batch_count` plain
`vkCmdDrawIndexedIndirect` calls whose `instanceCount` the late cull wrote — piece 2's structure,
with one word's producer changed, exactly as it promised.

**This is the design the constraint selects, not a workaround.** Of the two shapes that survive
without `…Count` — (a) a fixed-length record array with zero `instanceCount` for culled entries, and
(b) a dense per-batch prefix with a GPU-written survivor count — this engine already implements (b)
at instance granularity, and piece 3 replicates it for the late array. vkguide and pcwalton both warn
against (a); nobody has published a native-Vulkan measurement of its cost, and this design does not
need one.

**Two dispatches, not one, and it is not a choice.** The late test needs the pyramid, which needs the
early depth, which needs the early raster, which needs the early cull's output. The chain is serial
by construction.

**What the host fill still owns.** `vb_indirect_late_upload` (`graph_bridge.rs:3862-3870`, recorded
`vb.rs:1179` → `:1253`) **stays**. It carries `index_count`/`first_index`/`vertex_offset`/
`first_instance` — which no GPU pass produces — and seeds `instanceCount = 0`. After D3's split that
seed is **load-bearing rather than decorative**: it is what makes a frame with no late cull dispatch
draw nothing.

---

### D5 — ONE shader, ONE pipeline, ONE entry point, a UNIFORM branch on `pc.phase`; the late raster binds a SECOND set differing in one entry; and the cull binds a SECOND set differing in one entry

**The shader.** `vb_batch_cull.comp.hlsl` gains a `phase` push word. `phase == 0` is the early pass
(frustum + occlusion vs the stale pyramid + the two-way partition); `phase == 1` is the late pass
(read the candidates + occlusion vs the fresh pyramid + in-place compaction).

**Why one module and not a `-D` variant pair.** The shared part — `project_aabb`, `select_texels`,
`occluder_depth` and the verdict — is the part VG-P3-RECOVERY depends on being the SAME function in
both phases. Two artifacts means two implementations that can drift, and drift in the direction where
the late test is stricter than the early one is **geometry deletion**. This is verbatim
`hzb_build.comp.hlsl`'s own argument for a uniform `pc.base_level == 0` fork over two variants
(`VG-R3-P1-PYRAMID-PLAN.md` §8). No `docs/SHADER-VARIANT-MANIFEST.md` row is added, because that
manifest registers `-D` variants only.

**⚠️ The rule that makes per-phase framegraph declarations sound: LOADS may be hoisted, STORES may
not.** `graph_bridge.rs:3987-3992` records that DXC is free to lower a `? :` to an eager load plus an
`OpSelect`, so a not-taken *read* may still issue. No such licence exists for a *store*: a compiler
may not introduce a write the source does not perform. Therefore:

- `vb_batch_cull` (phase 0) declares **no access at all** on `vb_indirect_late` — the only store to it
  sits inside `if (pc.phase == 1u)`, and `pc.phase` is a push constant, uniform across the dispatch.
- `vb_cull_late` (phase 1) declares **no access** on `vb_indirect`, `vb_cull_visible`,
  `vb_cull_count` or `vb_visible_instance`, for the same reason.
- Every *load* the module can issue in either phase IS declared on both passes — the pyramid, the
  uniform, the ring, the batch descriptors — regardless of which phase dereferences it.

This is the asymmetry `graph_bridge.rs:3994-3998` already names, applied in the direction that needs
a reason rather than assumed.

**Why the late raster gets `vb_set0_late` instead of a VS edit.** The VS reads `visible_instances` at
`[[vk::binding(11, 0)]]` (`vb_raster.vs.hlsl:167`) and resolves
`visible_instances[pc.base_instance + instance_id]` (`:201-203`). Binding `vb_late_visible` at that
slot for the late scope makes the *identical expression* read the late list at the *identical base*.
`vb_set0_late` is `vb_set0` with one entry changed, per FIF. The late scope already rebinds set 0
explicitly (`vb.rs:1774-1783`), so this is a one-token change at the bind site.

**Why the cull gets `vb_cull_set_hzb`, and why Open Question 4 is CLOSED rather than deferred.**
Binding the pyramid at cull slot @9 needs the pyramid's view, which does not exist when
`DeferredSets::build` runs. Round 1 proposed reordering `sync_gbuffer` so the HZB is built first, and
called it a statement move. **It is not** (M11, verified): `DeferredSets::build` is a 10-parameter fn
at `targets.rs:2800-2817` that owns `vb_cull_set` at `:4630`; `GBufferTargets::create` calls it at
`:7015-7026` and builds the HZB at `:7767`; `hzb_depth_ring` is selected at `:7763-7766` from
`targets.forward`/`targets.depth` — **struct fields that do not exist until the literal at `:7714`**;
and three error arms (`:7049`, `:7110`, `:7153`) would each gain a pyramid drain, refuting bullets 2
and 3 of the placement argument the code carries at `:7728-7741`.

⇒ **Neither the reorder nor a new RHI update-one-binding helper.** `DeferredSets::build` keeps its
signature and builds `vb_cull_set` binding **`hzb_null`** at @9. The HZB block at `:7767` then builds
**`vb_cull_set_hzb`** — a complete second set, per FIF, identical except that @9 binds the real
pyramid — through the existing `RhiDevice::create_bind_group`. The recorder picks between them on
`scene.hzb.is_some()`, which C10 pins for the whole targets generation. Cost: one extra
`VulkanBindGroup` per FIF, destroyed by `HzbTargets::destroy` in the same reverse-acquisition ladder
`:7739-7741` already describes. This is `vb_set0_late`'s shape applied a second time, and it touches
**zero** existing error arms.

⇒ **`vb_raster.vs.hlsl`, `vb_raster.fs.hlsl`, `vb_geom_fetch.hlsli` and every raster `.spv` are
BYTE-UNCHANGED.** Only `vb_batch_cull.comp.hlsl` and its `.spv` move.

**How this discharges obligation 2, by substitution rather than by omission.** The late scope's VS
reads `vb_instance_ring` @0 and `vb_late_visible` @11. Both become real reads the moment
`instanceCount` is nonzero, and both are declared on `vb_raster_late` in the same change (D8).
`vb_visible_instance` is **not bound to the late scope at all** and therefore is not declared on it —
the R2d-3 rule applied correctly ("a bound descriptor is declared regardless"), not waived. **A
reviewer looking for `vb_visible_instance` on `vb_raster_late` should find its absence explained here
and in the code comment, not discover it.** The comment at `graph_bridge.rs:4049-4056` is now false
and is rewritten in the same commit.

---

### D6 — The cull's new inputs travel in a per-FIF UNIFORM BUFFER written UNCONDITIONALLY at a NAMED record site; the push grows by 8 bytes

C3 is decisive: 24 bytes of push headroom, and the occlusion test needs a `float4x4` (64 B) plus the
pyramid's source and base extents, the level count and a frame index.

**`VbCullUniform`, 96 bytes**, written by `vkCmdUpdateBuffer` and read as a `StructuredBuffer` — a new
binding on `vb_cull_layout`. Its write and its read are declared **inside the existing
`vb_batch_cull` pass**, as a `TRANSFER_WRITE` followed by a `COMPUTE SHADER_READ` — verbatim the
intra-pass shape the counter fill already uses (`graph_bridge.rs:3885-3886`). **No new pass.**

#### ⚠️ The intra-pass edge works at exactly ONE record site, and round 1 did not name it

C17: a pass's **entire** barrier set — including the intra-pass `TRANSFER → COMPUTE` edge derived
from its second declared access — emits at **one** site, at the pass boundary. So TRANSFER work
belonging to a pass must be recorded **BEFORE** `record_vb_pass`, or the barrier precedes the write
it is supposed to order and the dispatch reads stale bytes.

- **The precedent, with its reason in the tree**: `vb.rs:1293` records `cmd_fill_buffer` for
  `vb_cull_count`, and `vb.rs:1301` calls `record_vb_pass` after it. The SAFETY comment at
  `:1298-1300` states exactly this: *"`record_vb_pass` records the graph's derived barriers for the
  `vb_batch_cull` pass into `cmd` — the TRANSFER→COMPUTE ordering of both the descriptor upload and
  this counter fill against the atomics below."*
- **The neighbouring pass does the opposite, correctly, and that is the trap.**
  `vb_indirect_late_upload` calls `record_vb_pass` at `vb.rs:1179` (its SAFETY comment at
  `:1176-1178`) and issues its `cmd_update_buffer` at `:1253-1259` (SAFETY at `:1242-1251`), i.e.
  barrier first. That is right for a pass whose barrier
  orders *the write itself* (a WAW/WAR flush must precede it) and wrong for a pass whose barrier
  orders an *intra-pass* edge. Two adjacent sites, opposite orders, both correct. ⚠️ **There is no
  comment in `vb.rs` naming this contrast** — round 1's critique cited one at `:1179`/`:1252` and no
  such text exists. Piece 3 writes it.

⇒ **The specified site: immediately before `record_vb_pass(vb_batch_cull, …)`, beside the counter
fill at `vb.rs:1293`, inside the same `unsafe` block, under a `// SAFETY:` comment that names C17.**

#### The controls — TWO of them, because the obvious one cannot red on this machine

With `FRAMES_IN_FLIGHT = 2`, a fill landing on the wrong side of the barrier makes the dispatch read
**frame N−2's** uniform — bit-identical on every static fixture, so every golden, every image gate
and every oracle differential stays GREEN. That is why `VbCullUniform` carries a **`frame_index`**
field and why phase 0's batch lane 0 stores it into `vb_late_count[VB_LATE_COUNT_FRAME_SLOT]` (D3):
the readback can then compare `gpu_observed_frame_index` against the host's.

⚠️ **Round 2 specified ONE control and predicted a deterministic red for it. That prediction is
refuted by the tree.** `record_vb_pass` records **barriers only** (C25), and `vb.rs:1301 → :1362`
has nothing between it and `cmd_dispatch`. Moving the fill after `record_vb_pass` therefore leaves
the `vkCmdUpdateBuffer` **still ahead of the dispatch in submission order**; it removes only the
derived `TRANSFER → COMPUTE` edge. And a real missing edge is **measured invisible on this
machine** (C24): 19 messages, no `SYNC-HAZARD`, byte-identical image. So the control round 2
specified is expected to go GREEN with the defect present — the precise failure mode this campaign
has paid for six times.

⇒ **Two controls, with honestly different expected columns.**

| # | corruption | expected | what it proves |
|---|---|---|---|
| **F-M4a** | record the `vkCmdUpdateBuffer` **after `cmd_dispatch`** (`vb.rs:1362`), not merely after `record_vb_pass` | **RED, deterministically**: `gpu_observed_frame_index == frame_index − FRAMES_IN_FLIGHT` on a static fixture, because the FIF's uniform still holds its previous write | the INSTRUMENT is live: the tail slot, the lane-0 store, the probe field and clause 8's third line are wired end-to-end and CAN red. It does **not** test the barrier |
| **F-M4b** | move the fill after `record_vb_pass` but keep it before `cmd_dispatch` — **the real record-order defect** | **GREEN / undetermined**, and the result is published either way | nothing, if green. See below |

**What an F-M4b green is allowed to mean, stated so it cannot be over-read.** It means *this driver,
on this run, did not reorder a TRANSFER write past a COMPUTE read of the same buffer in the same
command buffer*. It does **NOT** mean the barrier is present, and it does **NOT** mean the ordering
is correct: without the derived edge the Vulkan spec permits the two to overlap, and C24 measures
that neither validation nor any image gate here can tell the difference. A green is a statement
about the driver, not about the code.

⚠️ **Consequence, recorded rather than papered over: the intra-pass edge has NO executable red on
this machine.** G-P3-F cannot see it either — the graph derives the same barrier set regardless of
where the recorder places the fill, so the pinned stream is field-identical under the defect. What
defends it is (i) the `// SAFETY:` comment at the fill site naming C17 and the `vb.rs:1298-1300`
precedent, (ii) the contrast note piece 3 writes beside `vb.rs:1179`/`:1253`, and (iii) F-M4a, which
proves the instrument is not vacuous. **That is the whole of the coverage, and it is weaker than
round 2 claimed.**

**Rejected alternatives.**
- *Raise the push range and probe `maxPushConstantsSize`.* Destroys `rhi_impl/mod.rs:195-211`'s
  stated property — *"so no device-limit query is required"* (`:205`) — for one matrix.
- *Drop the six planes and derive them in-shader from the view-projection.* Saves 96 B of push, but
  the planes are extracted host-side today (`frustum_planes_from_push_bytes`, `gpu_scene/mod.rs:6480-6484`),
  and re-deriving them on the GPU changes their floating-point evaluation order, hence the frustum
  verdicts at the boundary, hence pixels — in a step whose whole claim is that only the OCCLUSION
  decision moves.
- *Widen `VbBatchDesc` with a per-batch copy of the matrix.* 64 B × batch capacity of identical data.

**Written UNCONDITIONALLY, on every frame the cull is recorded, armed or not.** Round 1 gated the fill
on `occlusion_split` while the shader's `level >= levels → KEEP` early-out — the guard that makes the
`hzb_null` load safe — reads `levels` from that buffer. On a disarmed boot that is unwritten
allocation contents. The fill is ~96 bytes in a pass that already records a fill; gating it saves
nothing and creates a per-field validity question the plan would have to answer for every field.

**Push grows 104 → 112**: `phase: u32`, `occ_flags: u32`. Still inside the 128-byte floor
(`VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE = 128` at `rhi_impl/mod.rs:221`); `rhi_impl/mod.rs:227-232`'s
const-assert is the mechanical gate, and `scene_types.rs:549-551`'s size-assert — whose message
literally names 104 — moves with it, as does `compute.rs:1701`.

**The matrix is uploaded in MATH-ROW form** — `pv[row][col]`, `clip = pv · world` — which is exactly
what `boyko_render::hzb::project_aabb` takes (`hzb.rs:687-692`, layout note at `:660-668`). The host
performs the one byte inversion from the column-major push buffer, at ONE site.

---

### D7 — The pyramid is read with `.Load(int3(x, y, level))` through a mip-complete SAMPLED view bound at `GENERAL`. **No `VkSampler` is created anywhere in this piece.**

Obligation 6, discharged structurally: `.Load` takes integer coordinates and an explicit mip index.
**There is no filter to get wrong, and no `SAMPLED_IMAGE_FILTER_LINEAR` /
`VK_EXT_sampler_filter_minmax` dependency to probe.**

The two shapes the field uses are a `VK_SAMPLER_REDUCTION_MODE_MIN` sampler with `VK_FILTER_LINEAR`
(niagara — legal because the reduction mode *replaces* the weighted average with a component-wise
minimum) and four `textureLoad`s with a shader-side `min` (Bevy, whose own comment says it would use
a min reduction if wgpu exposed one). **Piece 2 already fixed Bevy's practice as this engine's** — it
matches `hzb_build.comp.hlsl`'s existing `.Load`-only discipline — **and D7 is that decision landing.**

Why a plain linear filter would be unsound, stated once: a reduced pyramid stores a *bound over a
footprint*, not a band-limited signal. A bilinear blend of four reduced texels is a convex
combination, so it lies strictly between their min and max — neither an upper nor a lower bound.
Under reverse-Z with a `min` reduce the stored value must be ≤ every depth in the footprint; a blend
can be *greater*, and therefore reject something visible. **False negatives are missing geometry, the
one failure mode that is not recoverable.**

Four loads, folded with the same `conservative_min` the oracle uses (`hzb.rs:503-511`: NaN on either
side → `-INFINITY`, else `if b < a { b } else { a }`).

#### ⚠️ The descriptor layout: a new RHI entry variant, because the kind currently implies the layout

C15: `BindGroupEntry::SampledImage` hard-writes `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL`
(`rhi_impl/device.rs:541`; the `GENERAL` arms are `:520` and `:533`), and `boyko_rhi/src/device.rs:343-345` states the contract in those terms.
**The pyramid is `GENERAL` for life** (P1's shipped property, on which D2's whole seed argument
rests). Binding it as `SampledImage` records a layout the image is never in — a core-validation error
at the arming commit, where an unexplained message delta on G-P3-G is hardest to attribute.

⇒ **`BindGroupEntry::SampledImageAtGeneral { texture }`**: `DescriptorKind::SampledImage`,
`sampler: VkSampler::NULL`, `image_layout: VK_IMAGE_LAYOUT_GENERAL`. This is exactly the shape
`StorageImageView` already has relative to `StorageImage` (`boyko_rhi/src/device.rs:355-357`:
*"Descriptor-IDENTICAL … the same `GENERAL` image layout, the same NULL sampler"*), so the enum's
discipline — *the kind names the layout* — is extended, not broken. ⚠️ **The enum is NOT
`#[non_exhaustive]`** (`boyko_rhi/src/device.rs:346-405`) and has **two** exhaustive matches in
`boyko_rhi_vulkan` — the write arm at `rhi_impl/device.rs:501-610` and `bind_group_entry_kind` at
`rhi_impl/mod.rs:157-175`. A variant added to one only is an E0004, which is exactly what
`StorageImageView` required when it landed. Both arms are in P3-1.

**Rejected: bind `levels` per-mip storage-image views as a descriptor array.** `HzbTargets::level_views`
already exists and storage descriptors legally take `GENERAL`, so it needs no RHI change. But the
selected level is **per-instance**, not dynamically uniform, so indexing that array needs
`shaderStorageImageArrayNonUniformIndexing` — a device-feature dependency this piece has none of
elsewhere (C16 already shows what a feature-conditional VB path costs) — plus `MAX_HZB_LEVELS`
descriptors on a set that would then differ by boot. Rejected on the feature dependency.

#### ⚠️ `hzb_null`, and the boot transition round 2 put in a place it can never run

On an `HzbMode::Off` boot @9 binds `hzb_null`: a 1×1 `R32_SFLOAT` image with a single-mip SAMPLED
view, minted and bound **unconditionally**, on every VB boot.

**Why it needs a real transition, and why round 2's answer cannot work.** Round 2 folded
`hzb_null`'s `UNDEFINED → GENERAL` into `boot_clear_hzb_pyramid` — "one encoder, two images". That
helper lives behind `HzbTargets::build`, whose **first statement** is the armed-only 0%-gate
(C19: `targets.rs:1258-1262`, before any encoder exists). `hzb_null` is minted and bound
unconditionally. So on **25 of the 26 committed pins** — every one except `[vb_mesh_hzb]`, which is
the only section setting `BOYKO_VG_HZB` (`goldens/PINS.toml:339`) — a descriptor would record
`GENERAL` over an image still in `UNDEFINED`, at **every** `vb_batch_cull` dispatch, from the arming
commit onward. The escalation exists to make that binding legal and round 2 left it illegal on the
path that always runs.

⇒ **`boot_seed_hzb_null`, its OWN unconditional helper.** Shape: the `csm.rs::seed_boot_layouts`
shape (`boyko_app/src/gpu_scene/csm.rs:394-435`, called unconditionally at `:365`, its reason at
`:8-10`: a module that *statically references* a binding makes the descriptor's recorded layout a
validation obligation whether or not the load is dynamically reached) — one encoder + one fence +
one submit + one `wait_fence`, with the destroy ladder on **every** path, exactly as
`boot_clear_shadow_temporal_hist` spells it (`targets.rs:6692`, `:6693`, `:6759`, `:6760`,
`:6768-6771`).

```
create_texture(hzb_null)  1x1, R32_SFLOAT, 1 mip, usage = SAMPLED | TRANSFER_DST
create_command_encoder; create_fence(false)
begin
  UNDEFINED -> TRANSFER_DST_OPTIMAL   TOP_OF_PIPE->TRANSFER, NONE->TRANSFER_WRITE
  clear_color_image(hzb_null, TransferDstOptimal, [0.0, 0, 0, 0], 1 mip, 1 layer)
  TRANSFER_DST_OPTIMAL -> GENERAL     TRANSFER->COMPUTE_SHADER, TRANSFER_WRITE->SHADER_READ
end; submit; wait_fence(u64::MAX); destroy encoder + fence on EVERY path
```

**It is CLEARED, not merely transitioned, and that is not decoration.** A transition alone leaves the
texel's VALUE undefined, and the disarmed path does issue the load (see the in-range argument below —
a not-taken load may still be lowered eagerly). `0.0` is the reverse-Z far plane, so even if that
value reached a verdict it **provably rejects nothing**. The safety argument becomes two-layered —
in-range by ADDRESS *and* conservative by VALUE — instead of resting on the compiler's lowering of an
`OpSelect`. Cost: one `vkCmdClearColorImage` on a 4-byte image, inside a submit that already exists.

**Call site, and the cost round 2 denied.** `boot_seed_hzb_null(ctx)` is called from
`GBufferTargets::create` **immediately before the `DeferredSets::build` call at `targets.rs:7015-7026`**,
because `DeferredSets::build` is what creates `vb_cull_set` (`:4630`, entries `:4656-4667`) and @9
must be bound there. Two consequences, both paid explicitly:

1. **`DeferredSets::build` gains an ELEVENTH parameter** — `hzb_null: &VulkanTexture`. ⚠️ This does
   **not** revive the reorder M11 refuted: that refutation was about the **pyramid**, whose view does
   not exist until `hzb_depth_ring` is selected at `targets.rs:7763-7766` from `targets.forward` /
   `targets.depth`, struct fields that do not exist until the literal at `:7628-7720`. `hzb_null` is
   1×1, takes no extent, depends on no struct field, and is constructible before any of it.
2. **The three error arms at `targets.rs:7049`, `:7110`, `:7153` each gain ONE `destroy_texture(hzb_null)`
   line.** The Boundary's round-2 claim "no edit to the three error arms" is **withdrawn**; what
   survives is the narrower and still-true "no pyramid drain in those arms, no rewrite of
   `hzb_depth_ring`, no `sync_gbuffer` reorder". C22 is why no cheaper site exists: there is **no**
   unconditional boot-layout submit already in `GBufferTargets::create` to fold into — both existing
   boot-clear helpers sit behind conditional wrappers (`:7234`, `:7300`).

**It lands at P3-1**, the step that MINTS `hzb_null` — not at P3-0, which is one commit earlier and
where the image does not exist. Round 2's step ladder said "transitioned by P3-0's helper"; that is
corrected in the implementation plan.

**Why the load is in range, restated because round 1 got the ARGUMENT wrong.** Round 1 justified
`hzb_null` with "it is never dereferenced". That is refuted by the engine's own recorded reasoning at
`graph_bridge.rs:3987-3992` (*"DXC is free to lower the `? :` to an eager load plus an `OpSelect`, in
which case the fetch happens and its result is DISCARDED"*) and by `hzb_build.comp.hlsl:478-481`
(*"'No tap is issued' has to be structural, not a property of an evaluation rule"*). ⇒ the
justification is an **in-range** argument, not a reachability one: the four load coordinates and the
level are **clamped to 0 unconditionally** on the disarmed path, so the address is `(0,0,0)`, which is
in range for a 1×1 single-mip image. The clamp is **not** derived from `uni.levels` — and that matters
even though D6 now writes the uniform unconditionally, because a structural bound must not depend on a
value another decision could later gate off. `descriptorBindingPartiallyBound` is NOT relied upon.

**Two lemmas that make the loads provably in range** — so the design does not lean on Vulkan's
unconditional image bounding, whose "returns undefined values" is agreement-breaking even when it is
memory-safe:

1. `texel_of(x) = (x · base) / source` is computed by the oracle in **u64** and by the shader in
   **u32**. They agree because the true product fits: `x < source ≤ MAX_HZB_EXTENT = 65536` and
   `base ≤ source`, so `x·base ≤ 65535 · 65536 = 4 294 901 760 < 2³²`. **No `Int64` capability is
   requested.**
2. `tx = texel_of(px) >> level` is always `< level_extent(level)`. `base` is a power of two, so for
   `level < log2(base)`, `(base−1) >> level = (base >> level) − 1`; for `level ≥ log2(base)`,
   `level_extent = 1` and `tx = 0`.

**The `firstbithigh` trap, named because it is exactly this campaign's class of defect.** The oracle
defines `msb(0) := 0` (`hzb.rs:492-494`, doc at `:488`). HLSL's `firstbithigh(0)` returns
`0xFFFFFFFF`. The selector `level = max(msb(tx0 ^ tx1), msb(ty0 ^ ty1))` (`hzb.rs:797`, an **unsigned**
`max`, so a single un-guarded axis wins) hits `0` whenever a rect fits in one texel — **the common
case**. The shader spells `v == 0u ? 0u : firstbithigh(v)`, and G-P3-D's corpus 1 contains a `1×1`
layout where single-texel rects are unconditional.

**The engine's selector is already the refined form.** niagara and interplayoflight compute
`ceil(log2(max_extent))` then apply a one-level refinement; the un-refined form is where Bevy #14042
went wrong (a ~29.36 × 30.06 px bbox selected mip 4, the 2×2 footprint stopped covering the sphere,
visible clusters were rejected — *geometry disappearing at certain distances only*).
`boyko_render::hzb`'s `msb(tx0 ^ tx1)` is alignment-aware by construction and carries a committed
coverage proptest (`property_selected_texels_cover_the_rect`, `hzb.rs:1606-1607`) plus a
counterfactual (`two_texels_per_axis_stop_covering_below_the_selected_level`, `:1692-1693`). **The
shader mirrors it exactly; no refinement step is added, and none is needed.**

---

### D8 — The graph: TWO new compute passes' worth of declarations, ONE new readback pass, ONE new dump pass, and `vb_indirect_late`'s writer chain becomes four links

**`vb_indirect_late`'s chain** — obligation 1, stated precisely:

```
vb_indirect_late_upload   TRANSFER       TRANSFER_WRITE          (host fill, all five words, instanceCount = 0)
vb_cull_late              COMPUTE        SHADER_WRITE            (n_keep)                      [NEW]
vb_raster_late            DRAW_INDIRECT  INDIRECT_COMMAND_READ
vb_cull_readback_late     TRANSFER       TRANSFER_READ           (probe frames only)           [NEW]
```

⚠️ Note what is NOT in that chain: `vb_batch_cull`. Round 1 put the early phase's `SHADER_WRITE`
there; D3 removed it. The declared writer that changes from `(TRANSFER, TRANSFER_WRITE)` to
`(COMPUTE_SHADER, SHADER_WRITE)` — obligation 1's literal text — is `vb_cull_late`.

**⚠️ What piece 3 retires, and what does not replace it.** The P2-8 provenance guard is first-touch
(C18) and `vb_indirect_late_upload`'s `TRANSFER_WRITE` is already its first touch, so the guard
**cannot fire on `vb_indirect_late` today either**. What piece 3 retires is the guard's ability to
catch the *deletion* of that upload declaration — the exact P2-7 defect, measured green on all four
gates — because after this change the next declaration in the list is `vb_cull_late`'s write, and a
write is never tested. **Nothing in this repository replaces that coverage.** The bounded fix is the
one P2-7 already prescribed (`is_write || res_written || res_seeded` for both kinds, plus the 14-site
`add_buffer` audit); it is a framegraph-core change and is **not** taken here. It stays in
`docs/OPEN-QUESTIONS.md` with P2-7's measurement, and this paragraph is the one-line answer the sixth
lens asked for: **after piece 3, `vb_indirect_late`'s provenance is covered by nothing.** The partial
consolation is `vb_late_count`, whose first touch IS a compute write and on which the guard is live
(D3).

**Declarator spelling, which after `1977fe0` *is* the provenance claim:** `vb_late_visible`,
`vb_late_count` and `vb_cull_uniform` are all declared with **`add_buffer`**, not
`add_buffer_seeded` — each has an in-graph producer every frame it is read.

**New accesses on `vb_batch_cull`, ALL gated on `occlusion_split`** so an unsplit frame's declared
set — and every existing golden's barrier stream — is **bit-unchanged**:

| resource | stage | access |
|---|---|---|
| `vb_cull_uniform` | `TRANSFER` then `COMPUTE_SHADER` | `TRANSFER_WRITE` then `SHADER_READ` |
| `hzb_pyramid` | `COMPUTE_SHADER` | `SHADER_READ`, `GENERAL`, mips `[0, levels)` |
| `vb_late_visible` | `COMPUTE_SHADER` | `SHADER_WRITE` |
| `vb_late_count` | `COMPUTE_SHADER` | `SHADER_WRITE` |

⚠️ The uniform's pair is the ONE exception to the gating: it is declared and written on **every**
frame the cull runs (D6). One predicate for the other three, both sites, and it is
`scene.path_vb_occlusion_split()` verbatim.

**`vb_cull_late`**, declared immediately after the last `hzb_build_*` and before `vb_raster_late`:

| resource | stage | access |
|---|---|---|
| `vb_batch_desc` | `COMPUTE_SHADER` | `SHADER_READ` |
| `vb_instance_ring` | `COMPUTE_SHADER` | `SHADER_READ` |
| `vb_cull_uniform` | `COMPUTE_SHADER` | `SHADER_READ` |
| `hzb_pyramid` | `COMPUTE_SHADER` | `SHADER_READ`, `GENERAL`, mips `[0, levels)` |
| `vb_late_count` | `COMPUTE_SHADER` | `SHADER_READ` |
| `vb_late_visible` | `COMPUTE_SHADER` | `SHADER_READ`, **then a SECOND call** `SHADER_WRITE` |
| `vb_indirect_late` | `COMPUTE_SHADER` | `SHADER_WRITE` |

⚠️ **`vb_late_visible` is declared as TWO calls, read then write, never as a combined
`SHADER_READ|SHADER_WRITE`.** C18: a combined access is `is_write`, so the guard never tests the read
half and the access latches (`graph.rs:692-724`; the assert at `:703-720`, the latch at `:722`). Under a combined declaration, deleting `vb_batch_cull`'s
`vb_late_visible` write (`one gated line in the closure that already omitted one in P2-7`) would be
silent at `vb_cull_late`, silent at `vb_raster_late`, and invisible to goldens, validation, the probe
and G-P3-F — which is a hand-written replica by its own admission. Split read-then-write, the read is
a genuine first-touch test. **Cost, stated:** the split derives a second self-WAR execution-only edge
on that pass, which is a NEW G-P3-F row rather than a hidden one. `vb_indirect_late` needs no split
(the upload already latched it, and the extra call would be inert).

`vb_mesh_bounds` is not a tracked ResId in this graph (a boot-fixed host-coherent table), exactly as
today.

**New accesses on `vb_raster_late`** — obligation 2:

| resource | stage | access |
|---|---|---|
| `vb_instance_ring` | `VERTEX_SHADER` | `SHADER_READ` |
| `vb_late_visible` | `VERTEX_SHADER` | `SHADER_READ` |

**New accesses on the EXISTING `vb_cull_readback`** (`graph_bridge.rs:3921-3947`), gated on
`occlusion_split && vb_cull_readback.is_some()`:

| resource | stage | access |
|---|---|---|
| `vb_late_visible` | `TRANSFER` | `TRANSFER_READ` — the **candidate list, before compaction** |
| `vb_late_count` | `TRANSFER` | `TRANSFER_READ` |

⚠️ Round 1 assigned these copies to `vb.rs` alone and never declared them — shipping undeclared
copies, the P2-7 class. They are declared here.

**`vb_cull_readback_late`, a NEW pass declared AFTER `vb_raster_late`**, gated identically:

| resource | stage | access |
|---|---|---|
| `vb_late_visible` | `TRANSFER` | `TRANSFER_READ` — the **compacted prefix** |
| `vb_late_count` | `TRANSFER` | `TRANSFER_READ` — for the no-clobber clause |
| `vb_indirect_late` | `TRANSFER` | `TRANSFER_READ` — `n_keep` |

**Why AFTER the late raster, and this refutes the cost the critique predicted.** B1 assumed the second
snapshot would sit between `vb_cull_late`'s COMPUTE write and `vb_raster_late`'s DRAW_INDIRECT fetch,
re-sourcing that fetch exactly as `graph_bridge.rs:3921-3947` documents for `vb_indirect`. Sited
**after** the raster, it does not: `vb_raster_late` only READS these buffers, so the bytes are
identical either way, and the first three links of the chain above are **field-identical with and
without the probe**. The probe still appends one edge, so **G-P3-F pins per configuration anyway**
(PROBE-OFF is the normative row set; PROBE-ON is a second, smaller set) — but the shipping chain the
gate certifies is the shipping chain, not a perturbed one.

**The declared pass order on an armed-split probe frame**, with four new declare-order asserts joining
the five piece 2 added:

```
vb_indirect_upload → vb_indirect_late_upload → vb_batch_cull → vb_cull_readback?
  → vb_raster → hzb_poison? → hzb_build_* → hzb_dump_depth_early? → vb_cull_late
  → vb_raster_late → vb_cull_readback_late? → classify? → lit → … → hzb_dump?
```

```rust
debug_assert_eq!(vb_cull_late.is_some(), scene.path_vb_occlusion_split());
debug_assert!(vb_cull_late.is_none_or(|c| hzb_build.iter().flatten().all(|b| b.index() < c.index())),
    "invariant: the late cull reads the pyramid this frame's build wrote");
debug_assert!(vb_cull_late.is_none_or(|c| vb_raster_late.is_some_and(|l| c.index() < l.index())),
    "invariant: the late cull writes the count the late raster fetches");
debug_assert!(vb_cull_readback_late.is_none_or(|r| vb_raster_late.is_some_and(|l| l.index() < r.index())),
    "invariant: the post-late snapshot must not re-source the indirect fetch (D8)");
```

⚠️ **None of these equates `hzb_build`'s presence with `vb_cull_late`'s** — mechanical fact 17:
`[vb_mesh_hzb]` (`goldens/PINS.toml:309-333`, env `:335-340`, the variable itself at `:339`) sets `BOYKO_VG_HZB=1` with **no** `BOYKO_VG_OCC` — and it is the **only** one of the 26 committed pins that does, and goldens run
the dev profile (`graph_bridge.rs:5071-5076`), so such an assert would
panic on a correct configuration.

---

### D9 — Arming: `path_vb_occlusion_split()` gains TWO conjuncts, and the two tripwires are deleted with one replaced

```
path_vb_occlusion_split() = path_is_vb()
                         && resolved_render_path.mesh_leg
                         && vb_occlusion_instances > 0
                         && hzb.is_some()                     // NEW
                         && vb_mesh_bounds.is_some()          // NEW — C16
```

`scene_types.rs:3539-3544`. The predicate stays a DERIVED expression, never a stored bool
(`HzbConfig::enabled()`'s discipline).

- **`hzb.is_some()` — why now and not in piece 2**: piece 2's split was inert, so a pyramid was not
  needed and the conjunct would have made every piece-2 gate reachable only under HZB. Piece 3's
  split *is* the occlusion test, and a late scope with no pyramid is a second scope that can decide
  nothing.
- **`vb_mesh_bounds.is_some()` — C16.** `batch_cull_armed` (`vb.rs:964-969`) carries this term and
  `path_vb_occlusion_split()` does not, so on a device without
  `storage_buffer_array_non_uniform_indexing` (`render_path_config.rs:952-954`, `runner.rs:551`,
  `:607`, `:630-631`, `:2355-2356`) the split arms while the cull is not recorded at all. Under this
  plan's declare/record parity mandate that reaches a `.expect()` on an absent `vb_cull_set`. The
  conjunct removes the state. On THIS machine the feature is present, so the golden impact is nil —
  which is exactly why it must be a conjunct and not a comment.

⚠️ **Consequence that must land in the same commit.** `[vb_occ_split.env]`
(`goldens/PINS.toml:390-395`) sets `BOYKO_VG_OCC="1"` at `:394` and **not** `BOYKO_VG_HZB`. Adding the
conjunct would silently disarm the split on the pin whose whole purpose is to arm it, and G2's
`scopes == 2` would red for a reason unrelated to any defect. ⇒ `crates/boyko_app/tests/vb_mesh.rs`
makes `BOYKO_VG_OCC` **imply** the `HzbMode::Build` arm (the const at `:64`, its read at `:135`, the
HZB branch at `:240-242`), and `BOYKO_VG_HZB = "1"` is written into `[vb_occ_split.env]` **and into
three of the four new `[*.env]` blocks**, so the configuration is legible from the pin file rather
than only from the fixture.

#### ⚠️ `BOYKO_VG_OCC` cannot also be the scene selector, and round 2 made it one

Round 2 specified the mixed fixture behind `BOYKO_VG_OCC=mixed`. **That selects nothing**: the read
at `vb_mesh.rs:135` is `std::env::var(ENV_OCC).is_ok_and(|v| v == "1")`, so `"mixed"` is simply
FALSE, and the scene itself is an unconditional five-sphere loop (`:139-157`) over one registered
mesh (`:117-120`). One variable was carrying scene shape, marking, and — after this decision — the
pyramid's existence. The four-pin equality round 2 wrote was therefore unsatisfiable, and its
`vb_occ_mixed_off` row rendered the existing `[vb_mesh_hzb]` configuration instead of the mixed scene.

⇒ **Three ORTHOGONAL variables, one meaning each:**

| variable | predicate | meaning |
|---|---|---|
| `BOYKO_VG_SCENE` | `== "mixed"` | **NEW.** Selects the scene SHAPE: two registered meshes, eight instances, the geometry G-P3-A specifies. Absent ⇒ today's five-sphere scene, byte-unchanged |
| `BOYKO_VG_OCC` | `== "1"` — **UNCHANGED**, `vb_mesh.rs:64`/`:135` | Marks. On the five-sphere scene: all five (the shipped meaning). On the mixed scene: the fixture's committed 6-of-8 pattern |
| `BOYKO_VG_HZB` | `== "1"` — unchanged, `:240-242` | Builds the pyramid. Implied by `BOYKO_VG_OCC` after this decision |

`BOYKO_VG_OCC_FORCE ∈ {unset, keep, late}` is the fourth, and it selects the push bits only.

**Why an orthogonal selector rather than widening the `OCC` predicate**: widening it would make the
one variable that means "mark" also mean "different geometry", so the `off` baseline could not exist
at all — and `vb_occ_mixed_keep`, the ONE-VARIABLE baseline the whole M10 answer rests on, needs the
scene held fixed while exactly one push bit moves. A selector that conflates scene with marking
cannot express that.

⚠️ **`vb_mesh.rs:127-134`'s "ALL FIVE spheres, or none" constraint is scoped, not deleted.** It
continues to govern the five-sphere scene verbatim. The mixed scene is the first fixture with PARTIAL
marking, and it therefore carries the reorder-safety argument itself, in a comment beside it: its
marked instances are strictly in front of or strictly behind the filler, never coplanar and never
interpenetrating, so the early/late reorder cannot change a depth tie — and the byte-identity pin IS
the check on that claim (G-P3-A).

**Obligation 3 — the two tripwires:**

- `vb.rs:1802-1806` (the indirection bit must be CLEAR): **deleted.** The late push now sets
  `VB_RASTER_FLAG_VISIBLE_INDIRECTION` (`vb.rs:68`), because the late list is an indirection list.
  The assert is replaced by the early scope's own shape (`vb.rs:1596-1602`): bit 1 never without
  bit 0.
- **`vb.rs:1224-1230` ONLY** (`instance_count == 0`): deleted as a tripwire and **REPLACED by the same
  expression under a permanently true invariant**. ⚠️ Round 1 wrote the range as `1220-1230`, which
  swallows the `first_instance == 0` assert at `:1220-1223` that `:1290`'s own invariant list requires
  to SURVIVE. The two asserts are adjacent and separate; only the second moves.

  ```rust
  debug_assert!(records.iter().all(|r| r.instance_count == 0),
      "invariant: the HOST seeds every late record with instanceCount = 0, and the LATE CULL is \
       the only producer of a nonzero value in this array (D3 moved the early phase's n_defer to \
       vb_late_count for exactly this reason). A frame in which the late cull did not run \
       therefore draws NOTHING.");
  ```

  This is not a cosmetic edit. **After D3 the message is true**, and it is the safety property that
  makes a missing `vb_cull_late` dispatch a *blank* late scope. ⚠️ **What it still cannot see:** it
  runs over the HOST-local `records` array (`vb.rs:1203-1218`), so it is structurally blind to the
  GPU-written word. The GPU's value is gated by G-P3-B clause 5, never by this assert.

⚠️ **And the vacuity D9 creates, named rather than discovered.** `VbRecordProbe::late_instances`
(struct `vb.rs:104-116`, field `:115`) sums the HOST-written records (`vb.rs:1237-1240`) — which stay `0`
forever. `vb_occ_split_gate.rs`'s `late_instances == 0` clause (`:592-604`) therefore **stays green
and stops meaning anything**. It is renamed `late_seed_instances` with a new message, and **the GPU's
real late count comes from the readback, never from the probe**. Renaming rather than deleting keeps
the host-seed property gated.

⇒ **`crates/boyko_app/src/vb_probe_dump.rs` is in the diff** (M9): the rename breaks `:129` and `:158`
with a hard E0609, and `write_probe` (`:144-168`) is the **only** serializer of `VbRecordProbe`, so a
new probe field that does not land there never reaches a test. Four named edits: the `finish` eprintln
field (`finish` at `:121-135`, the `eprintln!` at `:123-132`, the field access at `:129`), the emitted
key at `:158`, a new `late_cull_dispatches = {}` line beside it, and the `schema_version` bump at
`:153`. ⚠️ `vb_occ_split_gate.rs`'s `field()` (`:398-418`) **panics** on a missing key (`:414-417`), so
"never emitted" and "never incremented" red **differently** — the corruption table says which is
expected for which control.

---

### D10 — The dump carries BOTH depths and a FRAME INDEX; the EARLY depth copy becomes its own declared pass between the scopes

Obligation 5. Piece 2 recorded the hazard: the pyramid is built from the depth as of the *early*
scope, while `BOYKO_HZB_DUMP` copies `vb_depth` at frame end (`vb.rs:3509-3541`). In piece 2 those
were equal because the late scope drew nothing — `graph_bridge.rs:4995-4996` says so in words:
*"the late scope draws nothing — so gate G8 still holds AND is blind to the ordering."* The moment
piece 3 arms the late draws they can diverge, and G5 would compare the pyramid against a depth it was
not built from.

**"Move the copy" is the weaker of the two options and is rejected.** Moving it gives one depth and
therefore only a one-sided claim. Dumping both gives a **two-sided** claim:

- `build_pyramid(depth_early) == pyramid`, bit-exact — the pyramid WAS built from the early depth;
- and where `depth_early ≠ depth_final`, `build_pyramid(depth_final) ≠ pyramid` — it was NOT built
  from the final one.

The second clause is what a one-depth dump structurally cannot state. ⚠️ **And per D12 it is only
reachable on the FORCE-LATE fixture** — on a converged unforced frame the two depths are equal by
theorem, which is why G-P3-E is split in two.

**Shape.** `HzbDumpLayout` grows a second depth region. The header becomes
`[magic, source_w, source_h, levels, flags, frame_index]` + the per-level extents, and
**`HZB_DUMP_MAGIC` is bumped** (`scene_types.rs:1417`) so a stale dump file cannot be silently decoded
against the new offsets. `flags` bit 0 = "the early-depth region is live", set iff the frame armed the
split. `frame_index` is the engine frame the capture came from — the other half of B2's pairing check.
The staging grows by one depth image on dump frames only.

⚠️ **The header-drift guard cannot fire on this change, and that is a defect in the guard.**
`hzb_engine_pyramid_gate.rs:133-134` asserts `(HZB_DUMP_HEADER_WORDS - 4) / 2 == MAX_HZB_LEVELS`.
`HZB_DUMP_HEADER_WORDS` is `4 + 2 * MAX_HZB_LEVELS` (`scene_types.rs:1424`, = 38 with
`MAX_HZB_LEVELS = 17` at `:1362`). Going to 6 scalar words gives `(40 - 4) / 2 = 18 ≠ 17` and reds —
but going to any ODD count truncates and passes silently. ⇒ **export
`HZB_DUMP_HEADER_SCALAR_WORDS`** (today 4, becomes 6), define
`HZB_DUMP_HEADER_WORDS = HZB_DUMP_HEADER_SCALAR_WORDS + 2 * MAX_HZB_LEVELS`, and derive the gate's
offsets from it. The sites that move with it: `hzb_engine_pyramid_gate.rs:133-134`, the
`word(bytes, 4 + 2*k)` reads at `:426` and `:437`, the hardcoded `152` in the size message at `:455`,
and `HZB_DUMP_HEADER_BYTES` (`scene_types.rs:1430`). The tail-zero loop at `:436-445` would red
loudly on a mis-decode, so a wrong offset cannot ship — **but it would red naming the wrong defect**,
which is why the derivation, not the loudness, is the fix.

- **New pass `hzb_dump_depth_early`**, declared iff `occlusion_split && hzb_dump_armed`, sited between
  the last `hzb_build_*` and `vb_cull_late`. It declares `vb_depth` at
  `(TRANSFER, TRANSFER_READ, TRANSFER_SRC_OPTIMAL, DEPTH aspect)`, so the graph derives the round trip
  out of `hzb_build_0`'s `SHADER_READ_ONLY_OPTIMAL` and back into `vb_raster_late`'s
  `DEPTH_ATTACHMENT_OPTIMAL`. Both preserving; neither may become a first touch.
- The existing end-of-frame `hzb_dump` pass is unchanged in position and gains a destination offset:
  on a split frame its depth copy lands in the **final** region.
- On an unsplit dump frame only the final region is written; `flags` bit 0 is clear and the host must
  not read the early region — which still carries the `0xFF`/NaN prefill the existing gate already
  treats as "the copy never ran".

---

### D11 — The occlusion test is a HAND-AUTHORED, statement-for-statement mirror of `boyko_render::hzb`, with an EXPLICIT `precise` fold and the LOCAL sentinel guard as the OUTER branch

**The oracle already exists and is complete.** `hzb.rs:837-860` is `occlusion_verdict`; the steps and
their exact short-circuit ORDER are:

1. `!(min <= max)` → `Keep(UnknownBounds)` — spelled that way so a NaN also lands here
   (`hzb.rs:698-709`, reason at `:701-703`);
2. per corner in index order 0..8 (`bit0→x, bit1→y, bit2→z`, `0` picks min): non-finite clip →
   `Keep(NonFinite)`; **then** `cw <= 0.0` → `Keep(BehindEye)`; then divide; then non-finite
   post-divide → `Keep(NonFinite)`; accumulate window min/max and `depth_near = max(z_ndc)` seeded
   `-INFINITY` (`:720-755`). **First offending corner returns — a short-circuit, not a fold.**
3. `floor` on BOTH ends, clamp to `[0, source−1]`, empty → `Keep(EmptyRect)` (`:757-773`);
4. `level = max(msb(tx0^tx1), msb(ty0^ty1))` (`:790-808`, unsigned max at `:797`);
   `level >= levels()` → `Keep(LevelUnavailable)`, **never clamped down**; 4 texels via
   `containing_texel` (`:300-302`);
5. `occ = conservative_min`-fold over the 4 texels seeded `+INFINITY` (`:817-825`);
6. **`REJECT iff rect.depth_near < occ`, strictly** (`:855`) — equality KEEPS, and `:854` states why:
   the soundness chain `occ ≤ D[p] ≤ d_i(p) ≤ depth_near` admits equality, so `<=` would delete a
   visible instance.

**Step 2 is where this engine is already ahead of the field, and the shader must not lose it.** The
research could not find a `w <= 0` rejection in Bevy's 8-corner projection; without one, a corner
behind the eye flips sign under the perspective divide and the min/max rect can invert or collapse,
selecting a fine mip over the wrong place — a silent **over-cull**. `boyko_render::hzb` returns
`Keep(BehindEye)` on the first such corner and has a committed test for the exactly-`w == 0` case
(`keep_case_behind_the_eye`, `hzb.rs:1831-1832`).

#### ⚠️ The LOCAL sentinel guard is the OUTER branch. Round 1 inverted it; the shipped header records a critic catching exactly that inversion.

`vb_batch_cull.comp.hlsl:83-99` is a 17-line header whose title is *"UNKNOWN BOUNDS ARE TESTED BEFORE
THE TRANSFORM, AND THE ORDER IS LOAD-BEARING"*, and whose body says *"Testing it AFTER the Arvo fold
would INVERT the one-way guarantee, and a critic found exactly that inversion in an earlier draft of
this rung."* The shipped loop (`:441-466`) is:

```hlsl
bool keep = true;                              // :448 — seeded KEEP
if (!any(b.bmin > b.bmax)) {                   // :449 — the LOCAL sentinel guard, OUTER
    ...arvo fold...                            // :450-457
    keep = !aabb_outside_frustum(wc - wh, wc + wh);   // :458
}
```

Round 1's A1 step 1 made a **world-space** `any(mn > mx)` the normative home of the sentinel test and
placed the frustum `continue` before the `defer` computation. Two failures:

- **Sentinel + any normal affine.** `MeshLocalBounds::UNKNOWN` is `min = +1e30, max = -1e30`, so
  `lh = -1e30`; folded through a normal affine the world box is INVERTED, giving
  `radius = dot(abs(pl.xyz), h)` large NEGATIVE, so `dist + radius < 0` on the very first plane and
  the instance is frustum-REJECTED. Shipped code KEEPs; round 1 **deletes**. And because it is
  deleted before the partition, it lands in *neither* list — VG-P3-RECOVERY does not cover it.
- **Sentinel + exactly-zero linear part** (`Transform::from_scale` is an unguarded public `const fn`,
  `boyko_scene/src/transform.rs:92`): the header spells this case out — every
  `wh[r] = dot(abs(row.xyz), lh)` is `0 * -1e30 = 0`, so `mn == mx == wc`, round 1's `any(mn > mx)` is
  FALSE, and the collapsed point is trivially occluded by anything.

Round 1's two supporting claims are also false and are deleted: `vb_batch_cull.comp.hlsl:449` is the
LOCAL guard, **not** "the same short-circuit" as a world-space test (the frustum call is at `:458`);
and `any(mn > mx)` does **not** catch NaN, which is precisely why the oracle spells it
`!(min <= max)`.

⇒ **The normative shape, both phases:**

```
phase 0:  keep = true; defer = false;
          if (!any(b.bmin > b.bmax)) {            // OUTER, verbatim the shipped line
              (wc, wh) = arvo_fold(row, b);
              keep = !aabb_outside_frustum(wc - wh, wc + wh);
              if (keep && (occ_flags & ARMED) && (row.flags & OCCLUSION_CULLING)
                       && !(occ_flags & FORCE_KEEP)) {
                  defer = (occ_flags & FORCE_LATE) || occlusion_reject(wc - wh, wc + wh);
              }
          }
          // unknown bounds  =>  keep = true, defer = false  =>  drawn by the EARLY scope,
          // never frustum-tested, never occlusion-tested.

phase 1:  keep = true;
          if (!any(b.bmin > b.bmax)) {            // the SAME statement, spelled again
              (wc, wh) = arvo_fold(row, b);
              keep = !occlusion_reject(wc - wh, wc + wh);
          }
```

The phase-1 guard is **unreachable by construction** — the candidate list only ever holds instances
that passed the phase-0 guard, and `vb_mesh_bounds` is host-coherent and does not change between the
two dispatches of one frame. It is spelled anyway, because D5's whole argument is that the two phases
run the SAME statements, and an "unreachable so omitted" branch is the first place a future edit makes
them differ. The oracle's own world-space `!(min <= max)` (step 1) **stays** inside
`occlusion_reject`: it is a different guard, catching a degenerate or NaN *world* box, and dropping it
would diverge from the oracle.

⚠️ **The sentinel's reachability, with the MECHANISM corrected — round 2's was wrong, and wrong in
the unsafe direction.** Round 2 wrote that `MeshGeometryTable::register` "writes geometry and bounds
together", so an unregistered slot is harmless. The tree says the opposite in its own words:
*"A slot that is never registered keeps the [`MeshLocalBounds::UNKNOWN`] prefill"*
(`mesh_geometry_table.rs:587-588`, fn at `:600-612`) — and `VB_GEOMETRY_RESERVED_SLOT` is **exactly
such a slot**. What actually keeps the sentinel rare today is a different mechanism: the gather
**EXCLUDES non-resolvable meshes entirely** (`boyko_render/src/mesh_draw.rs:657-661`), so their
instances never reach the ring. But a mesh that IS registered and never received a VB geometry slot
resolves to `VB_GEOMETRY_RESERVED_SLOT` (`mesh_draw.rs:536`, doc `:500-515`) and **does** reach the
ring, carrying the `UNKNOWN` prefill. ⇒ **the sentinel is reachable in principle on the shipped
engine, not merely "invisible today by accident".** That strengthens the case for the outer guard
rather than weakening it, and the case is gated in G-P3-D corpus 4, which uploads its own rows and
bounds.

#### Why hand-authored and not `boyko_shaderdsl`

The eDSL owns numeric LEAVES so a nontrivial float body is bit-exact across the boundary, and this
body qualifies on its face. It is rejected here for one reason and the reason is churn, not principle:
`hzb.rs`'s projection chain is a shipped, heavily pinned oracle with **26 committed tests**
(`hzb.rs:937-2114`), including `anchor_pixel_rect_at_an_exact_integer_edge` (`:1190-1191`),
`depth_near_is_the_max_of_the_quotients_not_the_quotient_of_the_extremes` (`:1400-1401`) and the
`depth_near == occ` strictness assertion inside `anchor_verdicts` (message at `:1250`). Rewriting it
as a generic body over `f32`/`Emit` moves the file every existing HZB gate depends on, in the same
step that arms a decision. **Recorded as an Open Question**, because if G-P3-D ever shows a
disagreement not attributable to a stated cause, converting the leaf is the correct fix rather than a
tolerance.

#### ⚠️ `dot()` is FORBIDDEN in the projection. This repo has already rejected it, in writing, for exactly this reason.

`cluster_cull.hlsl:124-140` is this engine's own reasoned rejection (the load-bearing sentences at `:127-131`): *"Vulkan specifies OpFAdd /
OpFSub / OpFMul as 'Correctly rounded' … but specifies OpDot only as 'inherited from a formula', and
the same appendix permits that formula to 'be transformed using the mathematical associativity,
commutativity, and distributivity of the operators involved'."* The implemented remedy is at `:141-145`: the sum is **written out** with `precise` on **every node**
(`precise float3 d`, `precise float sd`). ⚠️ **`vb_batch_cull.comp.hlsl` carries no `precise`
qualifier today** — every one of its eight `dot()` calls (`:391-392` in the plane test, `:452-457` in
the Arvo fold) is unqualified. Piece 3 adds `precise` to the projection leaf ONLY; the eight existing
calls are UNCHANGED, which is what makes the census arithmetic below a re-measurement rather than a
prediction. The oracle's projection is an
explicit left-fold (`hzb.rs:726`: `r[0]*p[0] + r[1]*p[1] + r[2]*p[2] + r[3]`).

Round 1 specified `dot(row_r, world4)` and claimed it gives "the same four products in the same
order". It does not, and the distinction is not academic: `precise` emits `NoContraction`, which
forbids **contraction**, not **reassociation** — so round 1's mitigation list did not cover this at
all, and its control D3 (drop `precise`) mutates only contraction. `depth_near = max(cz * inv_w)` is
downstream of the sum, and a `depth_near` one ULP LOW is the geometry-deleting direction.

⇒ **The projection spells the four products and three adds explicitly, with `precise` on every
node**, and cites `cluster_cull.hlsl:124-145` as the governing precedent. G-P3-D carries control D5,
which swaps the fold for `dot()` and **reports whether the differential moves** — a null result is
itself the finding, recorded in the narrowed shape P1 §10 established.

#### ⚠️ There is NO artifact-level gate for "the projection contains no `dot()`". Round 2 specified one and it is unimplementable.

Round 2 pinned `OpDot == 0`. Three independent reasons it cannot be built:

1. **The census counts `OpDot` MODULE-WIDE**, in a flat token loop over the whole disassembly
   (`vb_batch_cull_spv_sync.rs:213-260`, the arm at `:230`), pinned `op_dot: 10` at `:356`.
2. **The calls it would have to exclude are in code this plan declares UNCHANGED** — the eight
   `dot()`s at `vb_batch_cull.comp.hlsl:391-392` and `:452-457`.
3. **Scoping the census to a function range is UNREACHABLE on this artifact.** A byte-scan of the
   committed `vb_batch_cull.comp.spv` finds exactly **one** `OpFunction` header: DXC inlines every
   helper into `%main` (C23). There is no artifact-level function boundary to scope to — this is not
   "not built yet", it is not constructible.

⇒ **Three replacements, each with a stated limit and a control:**

| gate | what it asserts | what it CANNOT claim | control |
|---|---|---|---|
| **SOURCE-level sentinel gate** (new, in `vb_batch_cull_spv_sync.rs`): read `vb_batch_cull.comp.hlsl`, take the text between `// === PROJECTION FOLD BEGIN ===` and `// === PROJECTION FOLD END ===`, assert the substring `dot(` does not occur in it and that it contains exactly four `precise` product terms and three `precise` adds | that the SOURCE of the projection is the written-out fold | **nothing about the compiled artifact**; a `dot()` written outside the sentinels is invisible; the sentinel comments themselves are unpinned text a refactor can move | insert `dot(` between the sentinels → RED. Delete a sentinel → RED (the extractor must fail loudly on a missing marker, not silently match empty) |
| **`NoContraction` decoration count** (new artifact-level pin): count `OpDecorate <target> NoContraction` in the same flat scan the binding pin already uses (`:250-254` is the existing `OpDecorate … Binding` selector, the shape to copy), pin the MEASURED value | that `precise` survived DXC on the number of nodes it was written on | it cannot distinguish WHICH nodes carry it; it counts, it does not locate | control D6 (drop `precise` from the projection locals) must move this number. If it does not, `precise` is not reaching the artifact and **that is the finding** |
| **module-wide `op_dot`, RE-MEASURED** | that the module's total `OpDot` count is what was measured at the commit | ⚠️ **it is NOT asserted to be 0 and must never be.** It is expected to MOVE, because D5 puts both phases' inlined copies of `arvo_fold` in one module | none — a re-measured pin has no red by construction, which is exactly why it is not credited as coverage for this property |

⚠️ **The live `assert!(actual.op_dot > 0 && actual.op_ford_less_than > 0, …)` at
`vb_batch_cull_spv_sync.rs:411-419` is UNTOUCHED and stays green.** It asserts the module still
carries a real plane test; it is not a `dot()`-in-the-projection gate and must not be repurposed into
one.

✅ **The no-sampler pin, by contrast, IS implementable and is kept.** `OpTypeSampler` and
`OpImageSample*` do not appear in the committed module today (`vb_batch_cull_spv_sync.rs` asserts
nothing about them yet, so this is a NEW pin), and both are countable in the same flat token scan.
Its control: add a `SamplerState` and one `.SampleLevel` → RED. That is D7's structural claim
("no `VkSampler` is created anywhere in this piece") given an artifact-level gate that can fail.

**Why bit-exactness is not required but near-exactness is, in ONE direction.** The verdict is a
boolean; a 1-ULP difference in `depth_near` changes it only within one ULP of `depth_near == occ` —
and there the oracle KEEPS. A shader whose `depth_near` came out one ULP LOW would REJECT there.
Mitigations, in order:

- The explicit `precise` fold above (reassociation AND contraction).
- `precise` on the projection locals (`inv_w`, `z_ndc`, `x_win`, `y_win`), which additionally forbids
  reciprocal substitution for `1.0 / cw`.
- The clamps use `max`/`min`, which lower to `NMax`/`NMin` — under which **a NaN operand is silently
  discarded rather than propagated**, the engine's recorded incident. They are reached only after both
  non-finite checks have returned, so no NaN can arrive. **Stated, because "unreachable" is the claim,
  not "handled".** (the shader header states the same NaN/`NMin` hazard in its own words at
  `vb_batch_cull.comp.hlsl:91-92`)
- G-P3-D's **constructed boundary corpus**, where `depth_near == occ` exactly.

✅ **One thing round 1 got right and it is worth keeping explicit:** P1 §10's measured ±0 tie-order
divergence **provably cannot reach this verdict**. `depth_near < occ` compares `−0.0` and `+0.0` as
EQUAL, and equality KEEPS. The one known GPU/oracle disagreement in the HZB chain is structurally
outside the predicate.

---

### D12 — THE FIXED POINT: on a converged static scene the late scope draws ZERO, and that is CORRECT. Every gate is built around it.

This decision has no code. It is the theorem the round-1 gates violated, and it is stated here because
four gates and three controls depend on it being written down.

> **THEOREM (VG-P3-FIXED-POINT).** On a static scene with a static camera, from frame 2 onward:
> `depth(E_N) == depth(E_{N−1})` bit-for-bit, `P_prev == P_cur` bit-for-bit, and therefore
> **`n_keep == 0` for every batch.**

*Proof.* An instance the early phase rejects is, by the late test's own soundness argument, occluded
by the early-drawn set — so every one of its fragments fails `VK_COMPARE_OP_GREATER` and it writes no
depth. Whether it is drawn or not, `depth(E)` is the same. Frame 1 rejects nothing (D1's boot clear),
so `depth(E_1)` is the full depth and `P_1 = P(depth(E_1))` is the true pyramid. Frame 2's early phase
rejects the occluded set, `depth(E_2) == depth(E_1)`, hence `P_2 == P_1`. The late phase on frame 2
tests each candidate against `P_2`, which is the same bytes the early phase rejected it against, with
the same view-projection and the same function (D5: one module). Same input, same function, same
verdict ⇒ every candidate is rejected ⇒ `n_keep == 0`. Induction gives `E_{N+1} = E_N`. ∎

**This is the correct behaviour of a two-phase cull, not a defect.** The late phase exists to catch
the cases where the previous frame's prediction was wrong — camera motion, object motion,
disocclusion. A static scene has none. Ubisoft's `< 0.01 ms` phase-2 number is the same statement
measured.

**What it destroys, and what round 1 shipped because it did not state it:**

| round 1 gate/control | under the theorem |
|---|---|
| G-P3-E clause 2 `depth_early ≠ depth_final`, **asserted** | **hard RED with no defect present** on the unforced fixture |
| control A3 "delete the late cull → RED" | **GREEN twice over**: `n_keep` is already 0, and under round 1's word reuse a missing late cull drew the deferred set at identical depth, which is pixel-invisible |
| control B2 `take(1)` in the late compaction | cannot fire — the loop keeps nothing to truncate |
| G-P3-C under FORCE_LATE with the all-marked fixture | the early depth is EMPTY, tripping the shipped non-vacuity clause at `hzb_engine_pyramid_gate.rs:559-573` (≥2 distinct depths, ≥1 `> 0.0`) |

**What it BUYS, and this is the round-2 reversal.** Because `P_prev == P_cur` on a converged frame,
**the dumped pyramid IS the pyramid the early phase read.** The early phase's verdicts, which round 1
declared permanently unfalsifiable (its Open Question 7), become observable:

> **G-P3-B clause 7 (phase agreement).** On a converged static frame, the oracle's verdict over the
> DUMPED pyramid must be `Reject` for **every** candidate — equivalently `Σ|K_b| == 0`, where `K_b`
> is the host-computed kept set. Any drift between what the early phase decided and what the same
> predicate says over the same bytes shows up here, in either direction.

That clause is the one place a wrong early matrix, a wrong extent, a wrong level selection or a
divergent phase branch becomes visible on a static scene, and it has a showable red (control B6).
Round 1's OQ7 is therefore **narrowed, not closed**: the early phase is unfalsifiable only on frames
where `P_prev ≠ P_cur`, i.e. under motion or under FORCE-LATE.

**What it requires of the fixtures.** Two regimes, two fixtures' worth of arming, from ONE scene:

| regime | early depth | `n_defer` | `n_keep` | what it can gate |
|---|---|---|---|---|
| unforced, converged | everything visible | > 0 | **== 0** | the early partition, phase agreement, byte-identity |
| FORCE_LATE, mixed marking | the UNMARKED filler only | == all marked | **> 0** | the late raster, the ordering, `depth_early ≠ depth_final` |

⇒ **`vb_occ_mixed` is specified in Gates and does not exist today.** The unforced regime alone cannot
exercise the late phase at all, and no amount of gate wording changes that.

⚠️ **The stated limit, recorded rather than papered.** With D3's count split, deleting the late cull
dispatch makes the late scope draw nothing, so **A3 IS an image control on the FORCE-LATE pin.** But
on the **unforced** pin no image-level control for "the late phase is load-bearing" exists or can
exist — the late phase correctly contributes zero pixels there. **On the unforced pin, "the late phase
works" is not a claim any image can carry; it is carried by G-P3-B clauses 5-7 and by the FORCE-LATE
pin, and by nothing else.**

---

## Data structures

```rust
// ── crates/boyko_rhi_vulkan/src/present/scene_types.rs (EDIT) ────────────────────────
/// The cull's non-push inputs. 96 B, 16-byte aligned, per-FIF, DEVICE_LOCAL, written by
/// `vkCmdUpdateBuffer` UNCONDITIONALLY inside the `vb_batch_cull` pass (D6) and read by both
/// phases. It exists because `VB_BATCH_CULL_PUSH_BYTES` is 104 and the shared compute push range
/// is const-asserted <= 128 (`rhi_impl/mod.rs:227-232`, the 128 itself at `:221`): a float4x4 does not fit.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VbCullUniform {
    /// MATH-ROW form, `clip = view_proj * world` — verbatim what `hzb::project_aabb` takes
    /// (`hzb.rs:687-692`). The host performs the ONE byte inversion out of the column-major
    /// push buffer, at one site. The shader spells the four products and three adds EXPLICITLY
    /// with `precise` on every node — never `dot()` (D11, `cluster_cull.hlsl:127-143`).
    pub view_proj_rows: [[f32; 4]; 4],   // 0..64
    /// The pyramid's SOURCE extent — `present_extent`, the same value the build pushed as
    /// `src_extent`, NOT the client extent (under armed SSAA they differ by 2x).
    pub src_extent: [u32; 2],            // 64..72
    /// Level-0 extent = `prev_pow2` per axis. PUSHED, never re-derived in the shader
    /// (P1-3's rule: a base-map disagreement must be a SHADER bug, never a math one).
    pub base_extent: [u32; 2],           // 72..80
    /// `HzbPlan::levels`. `level >= levels` is `Keep(LevelUnavailable)`, never a clamp down.
    pub levels: u32,                     // 80..84
    /// The engine frame this uniform describes. Phase 0's batch lane 0 stores it into
    /// `vb_late_count[VB_LATE_COUNT_FRAME_SLOT]`; the readback asserts it equals the host's
    /// frame index. This is the ONLY executable control for D6's record-order hazard, which is
    /// otherwise invisible on every static fixture (it would read frame N-2's matrix).
    pub frame_index: u32,                // 84..88
    pub _pad: [u32; 2],                  // 88..96
}
const _: () = assert!(core::mem::size_of::<VbCullUniform>() == 96);

/// 104 -> 112. Still inside `VULKAN_MIN_MAX_PUSH_CONSTANTS_SIZE`; `rhi_impl/mod.rs:227-232` is
/// the mechanical gate and `scene_types.rs:549-551`'s size-assert message moves with it.
#[repr(C)]
pub struct VbBatchCullPush {
    pub planes: [[f32; 4]; 6],   // 0..96   unchanged
    pub batch_count: u32,        // 96      unchanged
    pub visible_cap: u32,        // 100     unchanged
    pub phase: u32,              // 104     0 = early, 1 = late   NEW
    pub occ_flags: u32,          // 108     NEW — see below
}

pub const VB_CULL_OCC_ARMED: u32      = 1 << 0;  // the pyramid exists and the split is armed
pub const VB_CULL_OCC_FORCE_LATE: u32 = 1 << 1;  // the CONTROL: early defers every marked instance
pub const VB_CULL_OCC_FORCE_KEEP: u32 = 1 << 2;  // the OFF SWITCH: early defers nothing

pub struct GBufferScene<'a> {
    // ... existing ...
    /// The per-FIF early-reject / late-survivor list. `INSTANCE_CAPACITY` u32s, region-addressed
    /// by the SAME `VbBatchDesc.base_instance` / `.instance_count` that address
    /// `vb_visible_instance` — see INVARIANT VG-P3-LATE-REGION. Minted unconditionally on every
    /// VB boot (the `vb_visible_instance` rule); `.expect()`ed under `path_vb_occlusion_split()`,
    /// never a conjunct of it.
    pub vb_late_visible: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Per-batch `n_defer`, plus ONE reserved tail slot carrying the frame index the GPU observed.
    /// Written by the EARLY phase, read by the late phase, NEVER written by it (a readback clause
    /// gates that). It exists so `vb_indirect_late[b].instanceCount` has exactly one producer —
    /// see D3 for the three gates the alternative destroyed.
    pub vb_late_count: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
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
pub(crate) const VB_LATE_VISIBLE_BYTES: u64 = (VB_LATE_VISIBLE_ELEMS as u64) * 4;

/// One u32 per late DRAW RECORD, plus the frame-index slot. Sized off the same record capacity
/// `vb_indirect_late` uses, so the two arrays cannot disagree about how many batches exist.
pub(crate) const VB_LATE_COUNT_FRAME_SLOT: usize = VB_INDIRECT_LATE_RECORDS;
pub(crate) const VB_LATE_COUNT_ELEMS: usize = VB_INDIRECT_LATE_RECORDS + 1;
pub(crate) const VB_LATE_COUNT_BYTES: u64 = (VB_LATE_COUNT_ELEMS as u64) * 4;
// Usage on both = STORAGE | TRANSFER_DST | TRANSFER_SRC (the readback probe copies them).
// ⚠️ `gpu_scene/mod.rs:264-294`'s R2d-6 equality appears in this diff as CONTEXT ONLY. If it or
// `VB_VISIBLE_INSTANCE_ELEMS` moves by one character, piece 3 has re-created the R2d-6 collision.
// ⚠️ Mechanical fact 2: neither size is folded into the `.min()` clamp chain (`vb.rs:1186-1192`).

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

// ── vb_cull_layout: 7 -> 12 ENTRIES, bindings @0..@11 ────────────────────────────────
// ⚠️ The MODULE spells bindings as `: register(uN/tN)`, NOT `[[vk::binding(N,0)]]` — and the
// register index IS the Vulkan binding here (u0/t1/u2/u3/t4/t5/u6 -> 0..6), so the t/u spaces
// are kept mutually exclusive BY HAND. A new binding that reuses an index across spaces would
// alias silently. The seven that exist are declared at `vb_batch_cull.comp.hlsl:253` (u0
// VbIndirect), `:267` (t1 VbBatchDesc), `:273` (u2 VbCullVisible), `:277` (u3 VbCullCount),
// `:313` (t4 gVbInstances), `:341` (t5 gMeshBounds), `:346` (u6 VbVisibleInstance); the host
// layout is `gpu_scene/mod.rs:3919-3973`. New:
// u7   StorageBuffer  RWStructuredBuffer<uint>          VbLateVisible   (RW)
// t8   StorageBuffer  StructuredBuffer<VbCullUniform>   VbCullUni       (read)
// t9   SampledImage   Texture2D<float>                  gHzbPyramid     (read, GENERAL, mip-complete)
// u10  StorageBuffer  RWByteAddressBuffer               VbIndirectLate  (write, phase 1 only)
// u11  StorageBuffer  RWStructuredBuffer<uint>          VbLateCount     (RW)
// The count is DERIVED from one constant and const-asserted against the entry array's length —
// round 1 wrote "10" in six places against a normative table of four additions on a base of seven.
// `MAX_BIND_GROUP_BINDINGS = 24` (`boyko_rhi/src/device.rs:69`), so 12 is legal;
// `rhi_impl/device.rs:345-348` debug-asserts the entry/layout count match (DEBUG ONLY — release
// leaves an unwritten binding silent, which is why the const-assert is the real gate).

// ── crates/boyko_rhi_vulkan/src/present/targets.rs (EDIT) ────────────────────────────
pub(crate) struct GBufferTargets {
    // ...
    /// A 1x1 `R32_SFLOAT` single-mip image, usage `SAMPLED | TRANSFER_DST`, with a SAMPLED view.
    /// Minted UNCONDITIONALLY and bound at `vb_cull_layout` @9 by `DeferredSets::build` on every
    /// boot. The module DECLARES the pyramid binding in both phases (D5), and a not-taken load may
    /// still ISSUE (`graph_bridge.rs:3987-3992`), so the safety argument is IN-RANGE by address AND
    /// CONSERVATIVE by value: the disarmed path clamps coords and level to 0 unconditionally, and
    /// the texel holds `0.0` — the reverse-Z far plane, which rejects nothing. Created, cleared and
    /// transitioned `UNDEFINED -> GENERAL` by `boot_seed_hzb_null`, its OWN unconditional helper
    /// (D7), NOT by the armed-only `boot_clear_hzb_pyramid` — `VulkanTexture::create` cannot
    /// transition, and `HzbTargets::build`'s first statement is the 0%-gate (C19).
    pub(crate) hzb_null: VulkanTexture,
    /// `vb_set0` with ONE entry changed: @11 binds `vb_late_visible` instead of
    /// `vb_visible_instance`. Bound by the LATE raster scope only.
    pub(crate) vb_set0_late: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
}

pub(crate) struct HzbTargets {
    // ...
    /// `vb_cull_set` with ONE entry changed: @9 binds the real pyramid instead of `hzb_null`.
    /// Built HERE (`GBufferTargets::create:7767`) rather than in `DeferredSets::build:4630`,
    /// because the pyramid's view does not exist at that point and the reorder is not a statement
    /// move (D5 / M11). Destroyed by `HzbTargets::destroy`, first in reverse acquisition.
    pub(crate) vb_cull_set_hzb: [VulkanBindGroup; FRAMES_IN_FLIGHT],
}
```

**Bytes, because the sixth lens is right that no cost section stated any.** Per FIF:
`vb_late_visible` = `INSTANCE_CAPACITY × 4` (the same byte count as `vb_visible_instance`),
`vb_late_count` = `(records + 1) × 4` (one fifth of `vb_indirect_late` plus 4 B),
`vb_cull_uniform` = 96 B. Plus one 1×1 `R32_SFLOAT` image (`hzb_null`) whose suballocation
granularity dominates its 4-byte payload, one extra `VulkanBindGroup` per FIF (`vb_set0_late`), and
one more on HZB boots (`vb_cull_set_hzb`). **On a disarmed `HzbMode::Off` boot — which is 25 of the
26 committed pins today (`[vb_mesh_hzb]` is the only section setting `BOYKO_VG_HZB`,
`goldens/PINS.toml:339`), and 24 of 26 after P3-6 adds it to `[vb_occ_split]` — the added device
memory is under 32 KiB plus one 1×1 image**, and no new pass is declared or recorded. One extra
boot-time submit + fence wait per targets generation (`boot_seed_hzb_null`), on every boot; the
pyramid's own clear submit is armed-only.

---

## Public API

```rust
// boyko_rhi — ONE new enum variant (D7 / C15)
pub enum BindGroupEntry<'a, A: RhiApi> {
    // ... existing StorageImage / StorageImageView / SampledImage / ... ...
    /// Descriptor-IDENTICAL to [`Self::SampledImage`] except that it records
    /// `ImageLayout::General` and carries no sampler. For an image the engine keeps in
    /// `GENERAL` for life and reads with `.Load` — the HZB pyramid. Mirrors the
    /// `StorageImage` -> `StorageImageView` relation already in this enum.
    /// ⚠️ The enum is NOT `#[non_exhaustive]` and has TWO exhaustive matches in `boyko_rhi_vulkan`
    /// (`rhi_impl/device.rs:501-610`, `rhi_impl/mod.rs:157-175`); both are in P3-1.
    SampledImageAtGeneral { texture: &'a A::Texture },
}

// boyko_rhi_vulkan
pub struct VbCullUniform { /* pub fields, above */ }
pub const VB_CULL_OCC_ARMED: u32;
pub const VB_CULL_OCC_FORCE_LATE: u32;
pub const VB_CULL_OCC_FORCE_KEEP: u32;
pub const HZB_DUMP_HEADER_SCALAR_WORDS: usize;   // 4 -> 6; HZB_DUMP_HEADER_WORDS derives from it

/// ⚠️ Signature CHANGE, and the one this piece does not get for free: `DeferredSets::build`
/// (`targets.rs:2800-2817`) goes from 10 parameters to 11, gaining `hzb_null: &VulkanTexture`
/// so `vb_cull_set` (`:4630`, entries `:4656-4667`) can bind @9 on every boot. This is NOT the
/// reorder M11 refuted — `hzb_null` takes no extent and depends on no `GBufferTargets` field.
/// The three error arms at `targets.rs:7049`, `:7110`, `:7153` each gain one destroy line.

impl GBufferScene<'_> {
    /// UNCHANGED SIGNATURE; gains `hzb.is_some()` and `vb_mesh_bounds.is_some()` (D9). The shipped body (`scene_types.rs:3539-3544`) has exactly THREE conjuncts today: `path_is_vb() && resolved_render_path.mesh_leg && vb_occlusion_instances > 0`.
    pub fn path_vb_occlusion_split(&self) -> bool;
}

/// The recorder-authored probe. `late_instances` -> `late_seed_instances` (D9): the field sums the
/// HOST seed, which is permanently 0. The GPU's real counts come from `VbCullReadback`.
pub struct VbRecordProbe {
    pub scopes: u32,
    pub late_draws: u32,
    pub late_seed_instances: u32,
    /// Late cull dispatches recorded this frame: 0 or 1. Counted AT the `cmd_dispatch`, never
    /// derived from the arming predicate — the difference between a gate and a tautology. It is
    /// the only number in this piece that originates in the real recorder, and it reaches a test
    /// only because `vb_probe_dump.rs::write_probe` emits it (D9).
    pub late_cull_dispatches: u32,
}

// boyko_app — the readback payload becomes TWO snapshots (D8 / B1).
/// ⚠️ `VbCullReadbackLayout` lives in `boyko_rhi_vulkan` (`vb.rs:260-308`, ctor `:322-356`), NOT in
/// `scene_types.rs` and NOT in `boyko_app`; it is deliberately two-sided against `boyko_app`'s
/// `VB_CULL_READBACK_BYTES` (`vb.rs:310-321`). Round 1's integration table filed it wrongly.
pub struct VbCullReadback {
    // ... existing: count, visible[], indirect[], visible_instance[] ...
    /// PRE-late snapshot: the candidate list as the EARLY phase wrote it, and `n_defer` per batch.
    /// The only place clause 4's domain is observable — the late phase compacts in place.
    pub late_candidates: Vec<u32>,
    pub late_count_pre: Vec<u32>,
    /// POST-late snapshot (declared AFTER `vb_raster_late`, so the shipping barrier chain is
    /// unperturbed): the compacted prefix, `n_defer` again (the no-clobber clause), and the
    /// records whose `instanceCount` the late cull wrote.
    pub late_survivors: Vec<u32>,
    pub late_count_post: Vec<u32>,
    pub indirect_late: Vec<VkDrawIndexedIndirectCommand>,
    /// The engine frame the capture came from, and the frame index the GPU read out of
    /// `VbCullUniform`. Equality of the two is control F-M4; equality with the dump header's
    /// `frame_index` is the B2 pairing check.
    pub frame_index: u32,
    pub gpu_observed_frame_index: u32,
}
```

No `dyn`, no allocation in the frame loop, no new pipeline **layout family** beyond the widened
`vb_cull_layout`, no new sampler, no device-feature change. The `Vec`s above are decoded host-side in
a test harness, outside any frame loop.

---

## Algorithms for critical paths

### A1 — `occlusion_reject(world_min, world_max)`, the shared leaf (both phases)

Called ONLY from inside the outer `!any(b.bmin > b.bmax)` guard (D11), so an unknown-bounds instance
never reaches it.

| # | step | cost | notes |
|---|---|---|---|
| 1 | `!(mn <= mx)` → KEEP | 3 cmp | the ORACLE's world-space guard (`hzb.rs:698-709`), spelled `!(a <= b)` so a NaN lands here too. ⚠️ It is NOT the sentinel guard and NOT `any(mn > mx)` — round 1 conflated all three |
| 2 | 8 corners × (4 explicit `precise` products + 3 `precise` adds + finite checks + 1 divide + 2 mad) | ~200 flop | **short-circuits on the first offending corner**, in the oracle's order: finite-clip, then `w <= 0`, then divide, then finite-post-divide. **No `dot()`** (D11) |
| 3 | `floor` both ends, clamp to `[0, src−1]`, empty → KEEP | 4 flop | `floor` on the UPPER end too — `ceil(hi)−1` drops an exactly-integer edge column and produces a FALSE REJECT (`hzb.rs:757-763`) |
| 4 | `texel_of` ×4 (u32, D7 lemma 1), `msb(x^y)` with the zero guard, `level >= levels` → KEEP | ~10 int | the guard is per-axis; the unsigned `max` at `hzb.rs:797` lets one un-guarded axis win |
| 5 | 4 × `gHzbPyramid.Load(int3(tx, ty, level))`, folded with `conservative_min` | 4 loads | 2×2 adjacent at `level`; on a scene with locality they share a cache line |
| 6 | `depth_near < occ` → REJECT | 1 cmp | STRICT; equality KEEPS |

- **Complexity:** O(1) per instance, O(instances) per batch lane.
- **Cache:** the world AABB is already in registers from the frustum test (reused, not recomputed).
  The 4 pyramid loads are the only new memory traffic, at a level chosen so the rect spans ≤ 2 texels
  per axis — the tightest footprint the selector admits.
- **Branching:** six early-out returns, all of which KEEP. Every one is the conservative direction, so
  a mispredict costs cycles and never geometry. The hot path (a fully-visible on-screen box) takes
  none of them.
- **SIMD:** the 8-corner loop is 8 independent 4-term folds over one matrix. It is NOT unrolled by
  hand; `[unroll]` is left off, and the `.spv` census records the loop's shape so a future change is
  visible (the P1-3 lesson: an attribute's effect is a measurement, not an inference). ⚠️ `precise`
  constrains the *arithmetic*, not the vector width — the products may still be lowered to component
  ops; what it forbids is reassociating the sum.

### A2 — the EARLY phase (`pc.phase == 0`), per batch lane

```
if (i == 0u) VbLateCount[FRAME_SLOT] = uni.frame_index;   // D6's control, once per dispatch
k = 0; n_defer = 0
for j in 0 .. instance_count:
    g   = base_instance + j
    row = gVbInstances[g];  b = gMeshBounds[row.mesh_id]
    keep = true; defer = false
    if (!any(b.bmin > b.bmax)):                       // OUTER — UNCHANGED, :449
        (wc, wh) = arvo_fold(row, b)                  // UNCHANGED, :450-457
        keep = !aabb_outside_frustum(wc-wh, wc+wh)    // UNCHANGED, :458
        if keep && (occ_flags & ARMED) && (row.flags & OCCLUSION_CULLING) && !(occ_flags & FORCE_KEEP):
            defer = (occ_flags & FORCE_LATE) || occlusion_reject(wc-wh, wc+wh)
    if !keep: continue                                // neither list — frustum, as today
    if defer: VbLateVisible[base_instance + n_defer] = g;  n_defer += 1
    else:     VbVisibleInstance[base_instance + k]   = g;  k       += 1
VbIndirect  .Store(i*STRIDE + INSTANCE_COUNT_OFFSET, visible ? k : 0)   // UNCHANGED, :483-485
VbLateCount[i] = visible ? n_defer : 0                                  // NEW — NOT the record
// the batch-level InterlockedAdd on VbCullCount/VbCullVisible: UNCHANGED.
```

- `k + n_defer ≤ frustum survivors ≤ instance_count`, so both writes stay inside the batch's own
  region. **The budget piece 2's D5 proved, used.**
- No atomic beyond the existing one, no `groupshared`, no barrier — region disjointness is
  host-established (C9), unchanged.
- **`VbIndirectLate` is not touched in this phase.** Its only store sits under `pc.phase == 1u`, and a
  compiler may not introduce a store the source does not perform (D5) — which is what makes the
  framegraph declaration asymmetry sound rather than lucky.

### A3 — the LATE phase (`pc.phase == 1`), per batch lane

```
n_defer = VbLateCount[i]                              // read, never written in this phase
keep = 0
for j in 0 .. n_defer:
    g   = VbLateVisible[base_instance + j]            // sequential, ascending
    row = gVbInstances[g];  b = gMeshBounds[row.mesh_id]
    survive = true
    if (!any(b.bmin > b.bmax)):                       // the SAME statement; unreachable here (D11)
        (wc, wh) = arvo_fold(row, b)
        survive = !occlusion_reject(wc-wh, wc+wh)
    if survive: VbLateVisible[base_instance + keep] = g;  keep += 1
VbIndirectLate.Store(i*STRIDE + INSTANCE_COUNT_OFFSET, keep)
```

> **LEMMA (in-place compaction is race-free with no scratch).** At step `j` the lane reads index
> `base+j` and writes index `base+keep` with `keep ≤ j`. Every later read is at `base+j'` with
> `j' > j ≥ keep`. Therefore no write can clobber a slot a later read will consume. One lane per
> batch, so there is no cross-lane question at all.
>
> ⚠️ **Corollary the readback depends on:** after compaction the region holds
> `kept[0..keep)` followed by the ORIGINAL entries at `[keep, n_defer)` — a multiset that is NOT the
> candidate set. **The candidate list is recoverable only from the PRE-late snapshot**, which is why
> D8 declares two.

- The candidate list is written in ascending `j` order by A2, so `g` is ascending and the
  `gVbInstances[g]` gather is a monotone strided walk, not a random one.
- No frustum test is repeated: a candidate passed it in A2 by construction.
- **The world AABB is recomputed rather than stored.** Storing it would be 24 B per candidate of extra
  traffic to save ~20 flops. Rejected on the traffic.
- **Order is preserved**, and G-P3-B clause 5 asserts the kept prefix equals the oracle's kept
  SUBSEQUENCE elementwise — not just its length (M8).

### A4 — the late raster scope (piece 2's, with three words changed)

`begin_rendering(LOAD/LOAD/STORE)` → bind pipeline → **bind `vb_set0_late[fi]`** → pass-wide push →
per batch { push `[base_instance, base_flags | VISIBLE_INDIRECTION]`, bind VB/IB,
`cmd_draw_indexed_indirect(late[fi], i*20, 1, 20)` } → `end_rendering`. O(`batch_count`).

The VS expression, the base and the `vb_id` export are **unchanged**: `vb_late_visible` stores GLOBAL
instance indices exactly as `vb_visible_instance` does, so R2d-EXPORT-IS-GLOBAL (`vb_raster.vs.hlsl:98-107`,
`:209`) holds verbatim and every downstream consumer of `vb_id` sees the same encoding.

### A5 — the readback's host adjudication (the gate's algorithm, stated because its ORDER matters)

```
per batch b in 0..batch_count:
    C_b   = late_candidates[base_b .. base_b + late_count_pre[b])        // PRE snapshot
    K_b   = [c for c in C_b if occlusion_verdict(layout, dumped_pyramid, vp, aabb(c)) is Keep]
    S_b   = late_survivors[base_b .. base_b + indirect_late[b].instanceCount)   // POST snapshot
    assert late_count_post[b] == late_count_pre[b]                       // no-clobber
    assert S_b == K_b                                                    // elementwise, ordered
    assert indirect_late[b].instanceCount == K_b.len()                   // count, INDEPENDENTLY derived
```

`K_b` is computed from the CANDIDATE list and the DUMPED pyramid, never from the count the GPU wrote.
That is the whole of M8's fix: round 1's clause 5 compared the GPU's number against itself, and its
worked miss (deferred `[e0,e1,e2]`, keeps `{e0,e2}`, a cursor bug writing `keep = 3` so the region
reads `[e0,e2,e2]`) passed every clause. Elementwise equality against an ordered oracle subsequence
rejects it; so does the strictly-ascending / no-duplicate clause the frustum arm already carries at
`vb_inst_cull_corpus.rs:411-419`.

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
| early cull write `vb_late_visible` → late cull read | RAW | both declared; the read is a SEPARATE declaration from the write (D8/M3) so the provenance guard can still test it |
| early cull write `vb_late_count` → late cull read | RAW | both declared; first touch is the write, so the P2-8 guard is LIVE here |
| late cull write `vb_indirect_late` → late raster fetch | RAW | the four-link chain in D8 |
| late cull read+write `vb_late_visible` (same pass) | self-WAR | an execution-only edge the split declaration derives; a NEW G-P3-F row rather than a hidden one |
| early raster depth write → `hzb_build_0` depth read | RAW + layout | derived today (piece 2's D6 move) |
| `hzb_build_*` pyramid write → late cull pyramid read | RAW | new; both declared, both `GENERAL`, no transition — and `SampledImageAtGeneral` (D7) is what makes the DESCRIPTOR agree with that |
| **frame N `hzb_build` write → frame N+1 early cull read** | **cross-frame RAW** | **D2's writer seed**, whose GENERAL claim the boot clear makes true |
| **frame N late cull pyramid read → frame N+1 pyramid write** | **cross-frame WAR** | subsumed by frame N+1's intra-frame WAR barrier: a barrier recorded outside a render pass orders against all commands earlier in SUBMISSION ORDER and **there is one queue** (C5) |
| **frame N `hzb_dump` TRANSFER_READ → frame N+1 poison write** | **cross-frame WAR, UNORDERED** | ⚠️ **not resolved** — `seeded_writer_at_layout` sets `visible_stages = 0` (`sync.rs:319`). One frame, diagnostic path, strictly better than today's `undefined()` seed, value-invisible on a static corpus. See D2's residual paragraph |
| readback TRANSFER reads → the late raster's reads of the same buffers | RAR | none needed; the post-late snapshot is sited after the raster precisely so no WAR/RAW is introduced into the shipping chain |

**Within one lane** there is no synchronisation at all: the compaction lemma (A3) is a
single-threaded argument, and region disjointness (C9 / VG-P3-LATE-REGION) is host-established.
**No atomic is added by this piece.** The module's single `OpAtomicIAdd` (the early phase's
batch-level append) is unchanged and its census pin stays at 1
(`vb_batch_cull_spv_sync.rs:362`; the module's single `InterlockedAdd` is `vb_batch_cull.comp.hlsl:504`, and the live `assert!(op_dot > 0 && op_ford_less_than > 0, …)` at `vb_batch_cull_spv_sync.rs:411-419` is untouched).

---

## Integration

**31 files touched, 4 new.** Piece 2's list was short by five and cost a round; round 1's was short by
three (`vb_probe_dump.rs`, the readback declarator, `DECLARED_IDENTICAL_PINS`). This one is enumerated
to the test-fixture level.

### `boyko_rhi` / `boyko_rhi_vulkan`

| file | change | step |
|---|---|---|
| `boyko_rhi/src/device.rs` | `BindGroupEntry::SampledImageAtGeneral` (D7 / C15); the enum is `:346-405` and the layout contract at `:343-345` gains its row | P3-1 |
| `rhi_impl/device.rs` | the new variant's arm inside the ONE exhaustive `match *entry` at `:501-610`, writing `GENERAL` + NULL sampler — beside the existing `:520`/`:533`/`:541` arms | P3-1 |
| `rhi_impl/mod.rs` | ⚠️ **absent from rounds 1 and 2.** `bind_group_entry_kind` (`:157-175`) is a SECOND exhaustive match over `BindGroupEntry`, and the enum is not `#[non_exhaustive]` — without this arm P3-1 is an E0004. `StorageImageView`, the shape D7 copies, needed both arms too. The push const-assert at `:227-232` is verified still live (the 128 is at `:221`) | P3-1 |
| `shaders/vb_batch_cull.comp.hlsl` | the `flags` rename; five new bindings in `register(uN/tN)` form (u7/t8/t9/u10/u11); `phase` + `occ_flags`; the occlusion leaf (A1) as an EXPLICIT `precise` fold between `// === PROJECTION FOLD BEGIN/END ===` sentinels — the file carries **no `precise` today**; the two-way partition (A2) under the UNCHANGED outer guard at `:449`; the late phase (A3). The eight existing `dot()`s at `:391-392`/`:452-457` are UNTOUCHED | P3-4 |
| `shaders/vb_batch_cull.comp.spv` | re-DXC under the frozen recipe; **no new `-D` variant, no `SHADER-VARIANT-MANIFEST.md` row** | P3-4 |
| `src/compute.rs` | `VB_BATCH_CULL_PUSH_BYTES` 104 → 112 (`:1701`); `VB_CULL_UNIFORM_BYTES`; the binding-count constant | P3-4 |
| `src/present/scene_types.rs` | `VbCullUniform`; `VbBatchCullPush` +2 fields and the `:547-550` size-assert message; the three `VB_CULL_OCC_*` consts; `GBufferScene` +4 fields; `path_vb_occlusion_split()` +2 conjuncts (`:3539-3544`); `HzbDumpLayout` + the early-depth region; `HZB_DUMP_HEADER_SCALAR_WORDS` (`:1422`, `:1430`); `HZB_DUMP_MAGIC` bump (`:1415`); `VbCullReadbackLayout`'s new regions | P3-2/3/6/7 |
| `src/present/graph_bridge.rs` | the pyramid **seed** at `:3508-3512` **and the rewrite of the comment at `:3497-3507`, which prescribes the opposite**; `vb_late_visible` / `vb_late_count` / `vb_cull_uniform` ResIds appended LAST in both `cfg` arms; `VB_BUFFER_COUNT` 15→18 / 14→17 (`:2992`, `:2995`) + the sink assert (`:3630-3635`); the sink arrays; the early cull's four new accesses; `vb_cull_late`; `vb_cull_readback`'s two new accesses (`:3921-3947`); `vb_cull_readback_late`; `hzb_dump_depth_early`; `vb_raster_late`'s two new accesses; `VbPassPlan` +3; the four new declare-order asserts; **the "WHAT IS DELIBERATELY NOT DECLARED" comment at `:4049-4056` is now false and must be rewritten**; the pass-order anchors this piece depends on are `vb_batch_cull` `:3881`, `vb_raster` `:3949`, poison+build `:4024-4029`, `vb_raster_late` `:4058`, `hzb_dump` `:4971-4973`, `record_vb_pass` `:5172-5353` | P3-0/3/7 |
| `src/present/passes/vb.rs` | the uniform fill **immediately before `record_vb_pass` at `:1293`/`:1301`, under a SAFETY comment naming C17, and the note that `:1179`/`:1253` inverts it correctly**; the push widening; the late cull dispatch + `late_cull_dispatches`; the tripwire deletions (`:1802-1806`; `:1224-1230` **only** — `:1220-1223` survives); the `vb_set0_late` / `vb_cull_set_hzb` binds; the indirection bit; `VbRecordProbe` rename; the dump's two depth copies (`:3509-3541`); the two readback snapshots' copies; the `late_visible`/`late_count` size backstops beside `:1189-1193` | P3-3/6/7 |
| `src/present/targets.rs` | `hzb_null` (1×1, `SAMPLED\|TRANSFER_DST`, unconditional) + **`boot_seed_hzb_null`**, its own unconditional clear+transition helper in the `csm.rs:394-435` shape; **`DeferredSets::build` (`:2800-2817`) gains an ELEVENTH parameter** and the three error arms at `:7049`/`:7110`/`:7153` each gain one `hzb_null` destroy; `vb_cull_set` 7 → 12 entries with `hzb_null` at @9 (`:4630`, entries `:4656-4667`); `vb_set0_late`; `vb_cull_set_hzb` built in the HZB block at `:7767` and destroyed by `HzbTargets::destroy` (`:1428-1446`, reverse acquisition); **`boot_clear_hzb_pyramid`** modelled on `:6687-6773`, clearing the pyramid to `0.0` over mips `[0, levels)`, **returning `Err` on failure** (D2); **no `sync_gbuffer` reorder, no `hzb_depth_ring` rewrite** | P3-0/1/2 |
| `tests/vb_barrier_stream_baseline.rs` | G-P3-F: the U-rows re-pinned for the seed change (at P3-0, alone); the new S-rows, **per configuration** (PROBE-OFF normative, PROBE-ON additional); the controls F1/F2/F3 | P3-0, P3-8 |
| `tests/vb_batch_cull_spv_sync.rs` | the census: binding set 7 → 12 with **@7/@8/@9/@10/@11 each named in its OWN assertion** — ⚠️ a NEW shape, not a copied one: `:331-339` is a single JOINT check (`contains(&4) && contains(&5)`), so round 2's "the way @4/@5 are" cited a precedent that is not one. `OpAtomicIAdd == 1` **unchanged** (`:362`; the module has exactly one `InterlockedAdd`, `vb_batch_cull.comp.hlsl:504`); a NEW `OpTypeSampler == 0` / `OpImageSample* == 0` pin (D7); a NEW `NoContraction` decoration count, MEASURED (copy the `OpDecorate … Binding` selector shape at `:250-254`); the source-level `// === PROJECTION FOLD ===` sentinel gate (D11). ⚠️ **`op_dot` is RE-MEASURED, never asserted 0** — it is module-wide (`:213-260`, arm `:230`, pin `:356`) and the live `assert!(op_dot > 0 …)` at `:411-419` is untouched | P3-4 |
| `tests/window_present_gbuffer.rs` | the FOUR exhaustive `GBufferScene` literals gain four fields each. ⚠️ **Live anchors are `:2265`, `:3376`, `:8410`, `:9934`** — round 1 cited `3366/8390/9904`, which were already stale in its own commit, and filed the file under `boyko_app` | P3-2 |
| `tests/vb_indirect_barrier_chain.rs` | verify: the chain gains links on split frames. The `src_stage == COMPUTE` assertion is at `:104-108` | P3-8 |
| `tests/framegraph_gbuffer_equiv.rs` | verify only — the VB path has a PRIVATE ResId space | — |

### `boyko_app`

| file | change | step |
|---|---|---|
| `src/gpu_scene/mod.rs` | `vb_late_visible` / `vb_late_count` / `vb_cull_uniform` allocation, consts, drift asserts, `scene()` wiring, destroy; `vb_cull_layout` 7 → 12 entries (`:3919-3973`); **the `occ_flags` fold lives HERE**, beside `vb_cull_planes` (`:6480-6484`), reading `BOYKO_VG_OCC_FORCE` ONCE at boot — never per frame, and never in `runner.rs`; the readback decode (`:6732-6806`) gains both snapshots; the size two-sided assert at `:6747-6756` gains the two new buffers | P3-2/3/5 |
| `src/runner.rs` | **the B2 rewrite**: the cull readback gets settle→request→drain (`hzb_dump.rs:45`/`:51` are the shared constants; the sibling call sites are `:967`, `:2317-2320`, `:2718-2721`, `:2722-2727`), the bare early return at **`:2601-2621`** (round 2 wrote `:2591-2621`) is DELETED with its `return` at `:2619`, its work folds into the `:2763-2770` exit conjunction, and `frame_index` (declared `:987`, incremented `:2790`) reaches the probe line. **`format_vb_cull_probe_line` (`:2924-2954`) gains SIX keys**: `frame=`, `gpu_frame=`, and the four per-batch-grouped lists + `late_ic=` (M5) — in `vis=`'s own `groups.join("\|")` shape (`:2952`), for the reason `:2905-2916` gives (batch regions need not be contiguous). ⚠️ **`BOYKO_VG_OCC` / `BOYKO_VG_OCC_FORCE` are NOT read here** — round 2 put the knob in this row and no such read exists in this file | P3-5 |
| `src/vb_probe_dump.rs` | ⚠️ **absent from round 1's list.** `late_instances` → `late_seed_instances` at `:129` and `:158` (hard E0609); a new `late_cull_dispatches = {}` line in `write_probe` (`:144-168`) and in the `finish` eprintln (`:127-129`); the `schema_version` bump | P3-6 |
| `src/hzb_dump.rs` | the two-depth layout, the header rework, `frame_index`, the decode. `SETTLE_FRAMES`/`DRAIN_FRAMES` at `:45`/`:51` become the SHARED gate the cull readback also uses | P3-5/7 |
| `src/gpu_scene/csm.rs` | verify only — `seed_boot_layouts` (`:394-435`) is the PRECEDENT for `hzb_null`'s transition, not a site that changes | — |
| `src/hzb_plan.rs` | verify — it already calls `HzbLayout::new`; `base_extent`/`levels` come from there | P3-3 |
| `tests/vb_mesh.rs` | `BOYKO_VG_OCC` implies the `HzbMode::Build` arm (`:64`/`:135`, branch `:240-242`); **a NEW orthogonal `BOYKO_VG_SCENE == "mixed"` selector** (D9) building the mixed scene — **TWO registered meshes** (today there is exactly one, `:117-120`) and eight instances with the committed 6-of-8 marking pattern, replacing the unconditional five-sphere loop (`:139-157`) only under that selector. `:127-134`'s "all five or none" is SCOPED to the five-sphere scene, not deleted, and the mixed scene carries its own reorder-safety argument beside it. 512×512 unchanged (`:221`) | P3-6/8 |
| `tests/hzb_engine_pyramid_gate.rs` | G-P3-E: the two-depth clauses; the header-offset derivation at `:133-134`, `:426`, `:437`, `:455`; G8's unmarked leg unchanged | P3-7 |
| `tests/vb_occ_split_gate.rs` | the probe rename (`:593-604`); `late_cull_dispatches`; the header's "what it cannot claim" list (`:33-44`) is rewritten around D12 | P3-6/8 |
| `tests/vg_density_census.rs` | `VB_PINS` (`:59-91`) gains four names **and `DECLARED_IDENTICAL_PINS` (`:238-250`) gains the three identity relations** — they are separate arrays (fact 18) and round 1 named only the first. No density row is required (`:75-77` answers that) | P3-8 |
| `tests/vb_inst_cull_scene/mod.rs` | the shared readback fixture decodes both snapshots; the parser at `:565-573` is key-driven so the new fields are free | P3-5 |
| `tests/vb_occ_mixed.rs` | **NEW** — the mixed-marking fixture's gates (G-P3-A / B / C) | P3-8 |
| `tests/vg_occ_verdict_census.rs` | **NEW** — the CPU census, the `vg_cull_granularity_census.rs` shape | P3-8 |
| `tests/hzb_verdict_oracle_gate.rs` | **NEW** — the GPU-vs-oracle verdict differential (G-P3-D) | P3-4 |

### `boyko_render`, `goldens`, `docs`

| file | change |
|---|---|
| `src/hzb.rs` | **no functional change.** If the census needs `msb` (`:492-494`), it is exported; nothing else moves. |
| `src/occlusion_marker.rs` | doc only — the marker's meaning goes from "may be rejected" to "is tested". |
| `goldens/PINS.toml` | `[vb_occ_split.env]` (`:390-395`) gains `BOYKO_VG_HZB` beside its `BOYKO_VG_OCC="1"` at `:394`; **four** new pins (`vb_occ_mixed_off`, `vb_occ_mixed`, `vb_occ_mixed_keep`, `vb_occ_mixed_late`). ⚠️ The file has **26** top-level pin sections today and exactly ONE sets `BOYKO_VG_HZB` (`:339`) — round 2's critique said 25, and the plan's own "26 except two" was wrong the other way |
| `docs/SHADER-VARIANT-MANIFEST.md` | **no row** — stated so its absence is a decision |
| `docs/OPEN-QUESTIONS.md` | piece 3 status; and the `vb_indirect_late` provenance gap D8 records as covered by nothing |

**No change** to: `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl`, `vb_geom_fetch.hlsli`, `hzb_build.comp.hlsl`
or any `.spv` other than the cull's; `device.rs`'s fn table or feature chain (`vkCmdDrawIndexedIndirectCount`
stays unloaded, `:615-619`); `gpu_scene/mod.rs:264-294` (the R2d-6 equality — **context only**);
`boyko_render::hzb`'s algorithms; `hzb_depth_ring`'s selection at `targets.rs:7763-7766`; and no
`sync_gbuffer` reorder.

⚠️ **Two round-2 "no change" claims are WITHDRAWN, because D7 costs them:** `DeferredSets::build`'s
signature (10 → 11 parameters) and the three `GBufferTargets::create` error arms (`:7049`, `:7110`,
`:7153`, one destroy line each). M11's refutation is unaffected — it was about the PYRAMID, whose
view depends on struct fields that do not exist until `targets.rs:7628-7720`; `hzb_null` depends on
none of them (D7).

---

## Implementation plan — each step builds green, commits alone, and states what it MOVES

Round 1 gave only P3-0 a "why the tree is shippable here" argument. Every step now carries one, plus
the gates it is expected to move.

- **P3-0 — the pyramid's boot clear and the seed, alone.** `boot_clear_hzb_pyramid` (D2's ladder)
  clears the pyramid to `0.0` over mips `[0, levels)` and lands it in `GENERAL`, **returning `Err` on
  any encoder/fence/submit failure**; `add_image_mipped`'s seed becomes
  `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` at `graph_bridge.rs:3508-3512`;
  the comment at `:3497-3507`, which prescribes the opposite (quote at `:3502-3504`), is rewritten
  with the two-residual argument.
  *Shippable because:* nothing reads the pyramid yet, so the whole content is a barrier-stream change
  on `[vb_mesh_hzb]` — the ONE committed pin that builds a pyramid — and the clear makes the seed's
  `GENERAL` claim true from the first frame of every targets generation.
  *Moves:* G-P3-F's U-rows, re-pinned here in isolation with the reason in the commit message.
  *Gates:* 26/26 golden pins + `vb_occ_split`; U-rows re-pinned; G8/G5 green (the poison still
  dominates the clear); **the degrade unit test (D2) with its control executed** — force the clear to
  fail, assert `Err`; then make it return `Ok(None)` and show the lockstep assert at
  `targets.rs:7781-7785` fire; validation armed-vs-unarmed message-for-message — **this leg is the
  one that would have caught round 1's version**, and it runs here rather than at P3-1.
  *Why first:* it is the one change that moves an existing stream, and it must not be entangled with a
  change that moves pixels.
  ⚠️ **`hzb_null` is NOT in this step.** Round 2 said "transitioned by P3-0's helper"; the image does
  not exist until P3-1, and its transition cannot live in an armed-only builder (C19 / D7).

- **P3-1 — the RHI variant, and `hzb_null` MINTED, CLEARED AND TRANSITIONED, read by nothing.**
  `BindGroupEntry::SampledImageAtGeneral` **plus BOTH exhaustive match arms** —
  `rhi_impl/device.rs:501-610` and `rhi_impl/mod.rs:157-175`; the enum is not `#[non_exhaustive]`
  (`boyko_rhi/src/device.rs:346-405`), so one arm alone is an E0004. `hzb_null` minted
  unconditionally at `SAMPLED | TRANSFER_DST` and put through **`boot_seed_hzb_null`**, its own
  unconditional helper (D7): clear to `0.0`, `UNDEFINED → GENERAL`, teardown on every path.
  `DeferredSets::build` gains its eleventh parameter and the three error arms at `targets.rs:7049`,
  `:7110`, `:7153` each gain one destroy line.
  *Shippable because:* no existing entry changes kind, and `hzb_null` is bound by nothing yet — the
  layout widening is P3-2.
  *Moves:* nothing observable. *Gates:* 26/26, `cargo clippy` on the new public enum variant, and
  **`-ValidationOn` on a DISARMED pin** — the leg that would see a 1×1 image left in `UNDEFINED`, run
  on the configuration that actually exercises it rather than on the one pin in 26 that does not.
  *Why the transition lands here and not at P3-0:* this is the step that MINTS the image.

- **P3-2 — the three buffers and the widened layout, read by nothing.** `vb_late_visible`,
  `vb_late_count`, `vb_cull_uniform`, the const-asserts and the two size backstops, `vb_cull_layout`
  7 → 12 (`gpu_scene/mod.rs:3919-3973`), `vb_cull_set` with `hzb_null` at @9 (`targets.rs:4630`,
  entries `:4656-4667`), `vb_cull_set_hzb` in the HZB block at `:7767`, `vb_set0_late`, the four
  `window_present_gbuffer.rs` literals at `:2265`/`:3376`/`:8410`/`:9934`.
  *Shippable because:* the SHADER still declares 7 bindings, and that direction is legal and stated at
  `vb.rs:1330-1341` — *"a WRITTEN descriptor a shader never loads from is never dereferenced, so the
  bound set may legally exceed what the module declares"* (`:1338-1339`). ⚠️ The reverse would not be:
  `rhi_impl/device.rs:345-348` debug-asserts `entries.len() == layout.entry_count`, which is why the
  set and the layout move in ONE commit and the shader lags rather than leads.
  *Moves:* nothing observable. *Gates:* 26/26, validation armed-vs-unarmed (the leg that sees an
  illegal view, a wrong descriptor type or a layout mismatch), `vb_occ_split` green.

- **P3-3 — the graph and the recorder, still inert.** The ResIds, `VB_BUFFER_COUNT` (`:2992`/`:2995`),
  the sink arrays, the new accesses on `vb_batch_cull` (`graph_bridge.rs:3881`), `vb_cull_late`
  **DECLARED AND RECORDED ATOMICALLY** (declare/record parity forbids splitting them — the pattern is
  `vb.rs:964-969` against its byte-identical twin at `graph_bridge.rs:3764-3769`),
  `vb_cull_readback`'s two new accesses (`:3921-3947`), `vb_cull_readback_late`, the uniform fill at
  the named site (`vb.rs:1293`, immediately before `record_vb_pass` at `:1301`), the push widening,
  `phase`/`occ_flags` pushed with `occ_flags = 0`.
  *Shippable because:* the shader ignores both new push words, so the frame is byte-identical; the
  late cull dispatches against a module with no phase-1 arm, which is a dispatch that reads its
  bindings and writes `VbIndirectLate` nowhere — ⚠️ **and that is the one thing to check by hand at
  this step**: with no `if (pc.phase == 1u)` yet, phase 1 would re-run phase 0's body and rewrite the
  early lists. ⇒ **the `phase` fork lands HERE as a bare `if (pc.phase != 0u) return;`**, one line, so
  the inert late dispatch is a no-op by construction rather than by luck.
  *Moves:* G-P3-F's S-rows (authored here, PROBE-OFF and PROBE-ON). *Gates:* 26/26 + `vb_occ_split`,
  the four declare asserts live in every dev-profile golden (`graph_bridge.rs:5071-5076`).

- **P3-4 — the shader, and the differential that proves it before the engine ever runs it.** The
  occlusion leaf as an explicit `precise` fold between the `// === PROJECTION FOLD BEGIN/END ===`
  sentinels, both phases under the UNCHANGED outer guard (`vb_batch_cull.comp.hlsl:449`), the
  partition, the compaction. **Armed only by `occ_flags`, which the host still pushes as 0.** Landed
  together with `tests/hzb_verdict_oracle_gate.rs` (G-P3-D) — the gate that compares the SHADER's
  partition against `boyko_render::hzb::occlusion_verdict` with no engine involved, in the
  `hzb_build_oracle_gate.rs` shape.
  *Shippable because:* `occ_flags == 0` makes `defer` identically false, so A2 degrades to today's
  loop and the frame cannot change.
  *Moves:* the `.spv` census — the binding set (five new, each in its own assertion), the RE-MEASURED
  `op_dot`, the new `NoContraction` count, the new no-sampler pin, and the source-level sentinel gate.
  *Gates:* 26/26; G-P3-D green over four corpora with its controls executed; **and `-ValidationOn`,
  armed vs unarmed, message for message.** ⚠️ **This step's gate list named none in round 2, and it is
  the step that first puts @9 in the module's STATIC access set** — the point at which `hzb_null`'s
  recorded descriptor layout becomes a validation obligation whether or not the load is dynamically
  reached (`csm.rs:8-10`'s stated reason). It runs on a DISARMED pin, where `hzb_null` is what is
  bound.
  *Why the differential lands with the shader and not after:* P1-7 proved a shader against an oracle
  before any engine frame depended on it, and that is what let a real disagreement (the ±0 tie) be
  characterised instead of chased through a renderer.

- **P3-5 — the probe plumbing AND ITS EMITTER, on the INERT payload.** The cull readback's
  settle→request→drain conversion, the deletion of the bare early return at `runner.rs:2601-2621`
  (its `return` at `:2619`), the fold into the `:2763-2770` exit conjunction, both snapshots' regions
  in `VbCullReadbackLayout` (`vb.rs:260`, ctor `:322-356`) and the decode at
  `gpu_scene/mod.rs:6732-6806`.
  ⚠️ **AND the six new keys in `format_vb_cull_probe_line` (`runner.rs:2924-2954`)** — `frame=`,
  `gpu_frame=`, `late_cnt_pre=`, `late_cnt_post=`, `late_cand=`, `late_surv=`, `late_ic=` — in
  `vis=`'s own per-batch `groups.join("|")` shape (`:2952`), sized by `late_count_pre[b]` on the pre
  side and `indirect_late[b].instanceCount` on the post side, because batch regions need not be
  contiguous (`:2905-2916`). **Round 1's M9 was "a probe field with no serializer"; round 2 fixed it
  on `vb_probe_dump.rs` and reproduced it on this channel.** Without these keys, G-P3-B clauses 3-7
  and A5's adjudication have no data and the parser's `field()` panics naming the wrong defect
  (`vb_inst_cull_scene/mod.rs:569-571`).
  *Shippable because:* the payload is still the inert partition (`n_defer == 0` everywhere), so the
  existing `vb_inst_cull_corpus.rs` clauses are unchanged and green — this step is measured by them,
  not by the new clauses.
  *Moves:* the probe line's field set, and the exit behaviour of every `BOYKO_VB_CULL_READBACK` run.
  *Gates:* the existing cull-readback corpus tests; **a format→parse ROUND-TRIP unit test** over all
  six keys with ragged, non-contiguous, and empty per-batch groups — the one gate that can red on a
  grouping bug without a GPU; **plus the new pairing check run for the first time**:
  `BOYKO_VB_CULL_READBACK` and `BOYKO_HZB_DUMP` armed together in ONE process must now produce BOTH
  files, with equal `frame_index`. That is B2's defect, fixed and demonstrated before anything depends
  on it. ⚠️ The `env_remove` lists differ today — `vb_occ_split_gate.rs:449-451` removes
  `BOYKO_HZB_DUMP`, `vb_inst_cull_scene/mod.rs:649-650` does not — and the new driver must remove
  neither.

- **P3-6 — ARM IT.** The host sets `VB_CULL_OCC_ARMED`; `path_vb_occlusion_split()` gains its two
  conjuncts (`scene_types.rs:3539-3544`, three conjuncts today); `vb_mesh.rs` makes OCC imply HZB
  (`:64`/`:135`/`:240-242`); `[vb_occ_split.env]` gains `BOYKO_VG_HZB` (`goldens/PINS.toml:390-395`);
  the two tripwires are deleted — `vb.rs:1802-1806` and **`:1224-1230` ONLY** (`:1220-1223` and
  `:1065-1068` survive); the indirection bit is set; `vb_set0_late` and `vb_cull_set_hzb` are bound;
  `VbRecordProbe` is renamed (struct `vb.rs:104-116`, field `:115`) and `late_cull_dispatches` is
  counted AT the dispatch; `vb_probe_dump.rs` emits it (`write_probe` `:144-168`, key beside `:158`,
  eprintln `:123-132`, `schema_version` `:153`).
  *Shippable because:* it is the smallest commit whose frame can change, and every mechanism it arms
  landed green in isolation.
  *Moves:* **pixels, in principle.** On the committed corpus it must move none — `[vb_occ_split]`'s
  hash is the same literal as `[vb_mesh]` and `[vb_mesh_hzb]`, and it must stay that.
  *Gates:* the full G-P3 set except G-P3-E (needs P3-7) and the mixed fixture (P3-8).

- **P3-7 — the two-depth dump.** `HzbDumpLayout`, `HZB_DUMP_HEADER_SCALAR_WORDS`
  (`scene_types.rs:1424`, bytes `:1430`), the magic bump (`:1415`), `frame_index` in the header —
  stamped by the RECORDER inside the copy frame's command buffer (`vb.rs:3488-3507`), not by the host
  (`hzb_dump.rs:188-219`, the verbatim write at `:201`) — `hzb_dump_depth_early`, and G-P3-E's split
  clauses. The gate's derived offsets move with it: `hzb_engine_pyramid_gate.rs:133-134`, `:426`,
  `:437`, the hardcoded `152` at `:455`, and the tail-zero loop at `:434-445`.
  *Shippable because:* it changes a file format and a gate, not behaviour; the magic bump makes a
  stale file fail loudly instead of decoding against the new offsets.
  *Moves:* G8/G5's decode; the dump's byte size. *Gates:* G-P3-E on both regimes, G8's unmarked leg
  unchanged, and the OLD-magic-fails-to-decode unit test.

- **P3-8 — the fixture, the pins, the gates, and the three numbers.** `vb_occ_mixed` behind the NEW
  `BOYKO_VG_SCENE=mixed` selector with its two mesh registrations and partial marking, the four pins,
  `VB_PINS` (`vg_density_census.rs:59-91`, 15 entries today) **and** `DECLARED_IDENTICAL_PINS`
  (`:238-250`) — separate arrays, guarded by `the_pins_declared_byte_identical_actually_agree`
  (`:270-295`) — the CPU census with its analytic fixture precondition, the corruption table
  **including the controls that do NOT fire**, since reporting only the ones that fire is how a
  vacuous gate ships, and the FORCE_KEEP / ARMED / DISARMED timing triple with a zero control, in one
  sitting, published as prose in the commit message.
  *Shippable because:* it adds a scene and tests, and touches no shipped path.
  *Moves:* nothing in the engine. *Gates:* all of G-P3.

---

## Gates

> **"Can this gate fail?" is asked first.** This campaign has shipped and then caught: a validation
> switch that enabled no layer, a tile assertion whose two sides were the same expression, a barrier
> count pinned to the defective value, a barrier-stream pin green on the production defect because it
> is a replica, and — round 1 — two headline gates green in exactly the states they existed to catch.
> **Every gate below states what it CANNOT claim and carries at least one control that has been shown
> RED. Where no control is possible, that is written down instead of worked around.**

### The central question: which gate separates "rejected" from "broke"?

**Neither an image nor a count. The CONJUNCTION, measured in one sitting:**

| | image byte-identical to the disarmed run | GPU-reported counts |
|---|---|---|
| the cull partitioned correctly | **yes** | `n_defer > 0`; `n_keep == 0` unforced, `> 0` forced |
| the cull deferred nothing (vacuous) | yes | `n_defer == 0` |
| the cull deleted visible geometry | **no** | `n_defer > 0` |
| the late cull never ran | **no**, under FORCE-LATE (D3's count split) | `n_keep` word never written |

Row 2 is why the deferral count must be a hard `assert`, never a report —
`VG-R3-P1-PYRAMID-PLAN.md` §13 is the worked example of a green comparison over a field of zeros.
Row 3 is why the image must be compared against a **disarmed run of the same scene**, not a blessed
constant. **Row 4 is new in round 2**: under round 1's word reuse it read "yes / unobservable", and
that single cell is what made the piece's deepest control vacuous.

**The preconditions of the identity claim, stated because they are real:** it holds for OPAQUE
geometry, on a converged frame, with no temporal history dependence in the captured frame, **and only
if the fixture contains no coincident geometry** — a redraw at identical depth is pixel-invisible
under `VK_COMPARE_OP_GREATER` with depth-write ON, no `SV_Depth` and no `discard`
(`vb_occ_split_gate.rs:51`), but geometry at *equal* depth reordered between the scopes is not.
`vb_mesh.rs:127-135` already constrains marking "all five or none" for exactly this reason.
**`vb_occ_mixed` breaks that constraint deliberately and must therefore carry the argument itself:**
its marked instances are strictly in front of or strictly behind the filler, never coplanar and never
interpenetrating, and **the byte-identity pin IS the check** — a depth tie would red it.

### ⚠️ THE FRAME-INDEX TRAP, and the pairing that round 1 could not produce

D1's boot clear makes the pyramid all-zeros at birth, so **on frame 1 the cull provably defers
NOTHING**. A fixture capturing the first rendered frame would compare a cull that did nothing against
a cull that was off, get byte-identity, and prove nothing.

Round 1 asserted a frame index that no code emits, and specified a capture that could not exist:
`runner.rs:2601-2621` returns on the **first presented frame** (the `return` at `:2619`) with no
settle and from **outside** the `:2763-2770` exit conjunction, so arming `BOYKO_VB_CULL_READBACK`
beside `BOYKO_HZB_DUMP` exits at frame 1 — cull file written, pyramid file **never**. On that payload
clause 1 is FALSE by D1's own convergence argument.

⇒ P3-5 gives the cull readback the sibling shape (`SETTLE_FRAMES = 30` at `hzb_dump.rs:45`,
`DRAIN_FRAMES = 3` at `:51`), folds it into the exit conjunction, and puts `frame_index`
(`runner.rs:987`, incremented `:2790`) on the probe line **and** in the dump header (D10). ⚠️ The
drain relation is `>= DRAIN_FRAMES`, never `==`: `DRAIN_FRAMES = 3 > FRAMES_IN_FLIGHT = 2`, the dump
advances only on `presented == true` (`hzb_dump.rs:153-155`), and `frame_index` increments on every
loop iteration including recreate-skips. The gate then asserts:

```
probe.frame_index == dump_header.frame_index          // ONE frame, ONE process
probe.frame_index >= 3                                // converged (D1: fixed point from frame 2)
probe.gpu_observed_frame_index == probe.frame_index   // the instrument, control F-M4a (D6)
```

⚠️ **Running the two captures in separate processes does NOT fix this** and must not be attempted:
the readback payload would still be frame 1, so clause 1 would red for an instrument reason and the
only way to green it would be to relax it. A windowed boot owns the device singleton and the window
(`vb_inst_cull_scene/mod.rs:634-636`), so "in the same run" can only ever mean one *sitting*. This is
Bevy #17736's shape — a defect that "only manifests on the second and later frames of a stable scene,
exactly the shape a golden pin can miss".

### G-P3-A — the ARMED image equals the DISARMED image, on a scene with guaranteed occlusion

#### The fixture, specified — because round 2 named "a slab" and "a smaller object" and nothing else

**`vb_occ_mixed`**, selected by the NEW orthogonal `BOYKO_VG_SCENE=mixed` (D9). 512×512
(`vb_mesh.rs:221`), the shipped camera, static. `prev_pow2(512) = 512`, so `base == source`, the
level-0 texel grid IS the pixel grid, and `texel_of` is the identity.

**Two mesh registrations** — because batches bucket per `MeshHandle`
(`boyko_render/src/mesh_draw.rs:672-956`) and `vb_mesh.rs:117-120` registers exactly ONE mesh today,
so `draw_batches >= 2` is delivered by registration or by nothing:

| mesh | instances | role |
|---|---|---|
| **A** — the existing sphere (`vb_mesh.rs:117-120`) | 1 unmarked (the far filler), 2 marked-HIDDEN, 1 marked-VISIBLE | 4 |
| **B** — a NEW unit-cube "slab" | 1 unmarked (**the occluder**), 2 marked-HIDDEN, 1 marked-VISIBLE | 4 |

Eight instances, **six marked** (4 hidden + 2 visible), **two batches**, both of which defer under
FORCE-LATE — so the late path's per-batch state at `i > 0` is exercised, which is the debt
`vb_occ_split_gate.rs:43-44` records as *"piece 3's first gate"*. `draw_batches == 2` is **asserted
from the probe's `[host]` table** (`vb_probe_dump.rs:161`), in the shape `MULTI_BATCHES` is derived
and asserted at `vb_occ_split_gate.rs:90-93` / `:541-546`.

**Placement:** the slab's projected rect covers pixels `[64, 448)²` — its depth is the nearest in the
scene. The far filler sits outside that rect at a different depth, so the early depth carries **≥2
distinct values and ≥1 texel `> 0.0`**, which is what `hzb_engine_pyramid_gate.rs:559-573`'s SHIPPED
non-vacuity clauses require (`:561-567` distinct, `:568-573` coverage). The two marked-VISIBLE
instances project wholly OUTSIDE `[64, 448)²`.

#### ⚠️ FIXTURE PRECONDITION VG-P3-MIXED-OCCLUDES — because "wholly behind the silhouette" is necessary and NOT sufficient

`select_texels` returns the **ALIGNED** expansion, `containing_texel(t, level) = t >> level`
(`hzb.rs:300-302`, `:790-808`), and `conservative_min` folds all four texels (`:817-825`). With the
reverse-Z clear at `0.0` (`vb.rs:52`), **one background texel anywhere in the footprint forces KEEP**
(C28). At 512×512 a rect merely straddling x=256 selects level 8 and its footprint is the whole
image. Round 2 stated the property as "wholly behind the slab's silhouette" and asserted
`Σ n_keep == 2` off it; both would red a correct engine on a fixture that is one transform away from
structurally-cannot-defer.

**The design rule that makes it hold:** each marked-HIDDEN instance's projected rect must lie wholly
inside ONE `2^(L+1)`-aligned block of width `2^(L+1)` that is itself wholly inside the slab's rect.
Then `msb(tx0 ^ tx1) <= L`, the 2×2 footprint is exactly that block, and every texel in it belongs to
the slab. Concretely: the four hidden instances go inside the 128-aligned blocks
`[128,256)×[128,256)`, `[256,384)×[128,256)`, `[128,256)×[256,384)`, `[256,384)×[256,384)`, all
strictly inside `[64,448)²`. `MIXED_MAX_LEVEL = 6` is committed as a constant.

**It is ASSERTED, in two forms, and both are textually distinct from clause 1 so a fixture error can
never be mistaken for a cull defect:**

| form | where | what it computes | what it CANNOT claim |
|---|---|---|---|
| **analytic** (no GPU) | `tests/vg_occ_verdict_census.rs` | from the same `vp` and AABBs the engine uploads: `project_aabb` → `select_texels` for every marked instance; asserts `sel.level <= MIXED_MAX_LEVEL`, that each selected texel's covered pixel block ⊆ the slab's rect, and `rect_i.depth_near < rect_slab.depth_near` (the slab is strictly in front). For the two VISIBLE ones: rect disjoint from the slab's | that the ENGINE produced that pyramid — it is pure host arithmetic |
| **measured** | **clause 0 of G-P3-B** | over the DUMPED pyramid: `occlusion_verdict(...)` is `Reject` for every intended-hidden instance and `Keep(NotOccluded)` for every intended-visible one | nothing about the GPU's partition — it is the oracle over the dump, and it is what clause 4/5 then compare the GPU against |

Both red with the message **"FIXTURE PRECONDITION — the mixed scene's geometry does not produce the
intended occlusion at this framebuffer size; this is a FIXTURE error, not an engine defect."**
Control: nudge one hidden instance's transform so its rect straddles a 128 boundary → the analytic
form reds on `sel.level <= MIXED_MAX_LEVEL` **before any GPU runs**. The plan names this hazard class
at Bevy #14042 in D7 and round 2 then failed to carry it into the fixture; this is the carry.

#### The four pins

Four pins from one binary. ⚠️ **`vb_occ_mixed_off` carries `BOYKO_VG_HZB="1"` and NOT
`BOYKO_VG_OCC`** — that choice is stated because it decides how many variables the baseline moves:

| pin | env (beyond `BOYKO_VG_SCENE=mixed`) | draws early | draws late |
|---|---|---|---|
| `vb_occ_mixed_off` | `BOYKO_VG_HZB=1` | all 8, one scope | — |
| `vb_occ_mixed_keep` | `BOYKO_VG_HZB=1 BOYKO_VG_OCC=1 BOYKO_VG_OCC_FORCE=keep` | all 8 | 0 (2 records, both `instanceCount = 0`) |
| `vb_occ_mixed` | `BOYKO_VG_HZB=1 BOYKO_VG_OCC=1` | 4 (2 filler + 2 visible marked) | 0 |
| `vb_occ_mixed_late` | `BOYKO_VG_HZB=1 BOYKO_VG_OCC=1 BOYKO_VG_OCC_FORCE=late` | 2 (filler) | 2 |

⚠️ Round 2's table put `BOYKO_VG_OCC=mixed` in three rows and `—` in the `off` row. Neither works:
the read at `vb_mesh.rs:135` is `is_ok_and(|v| v == "1")`, so `"mixed"` selects nothing, and an empty
`off` row renders the existing five-sphere scene rather than the mixed one (D9). It also asserted
`BOYKO_VG_HZB` in all four rows while simultaneously counting "the pyramid's entire existence" among
`off`'s differences — a self-contradiction this table resolves by choosing.

**The variable ladder, which is the point of having four pins:**

- `off` → `keep`: **four variables** — the split predicate, the late scope's bracket, the late
  dispatch, and the second/third descriptor sets. A difference here is a **PLUMBING** defect.
- `keep` → `mixed`: **one push-constant bit**. Same pyramid, same scopes, same dispatches, same sets;
  `defer` goes from identically-false to computed. A difference here is a **DECISION** defect.
- `keep` → `late`: **one push-constant bit**, the other one.
- The pyramid's existence is **not** a variable in this family — `off` builds it too. That it is
  byte-neutral is already pinned by `[vb_mesh]` vs `[vb_mesh_hzb]`.

Round 1 shipped `VB_CULL_OCC_FORCE_KEEP` as production shader surface that no gate ever set; the
`keep` pin is what makes it a supported switch (Boundary).

All four `sha256_software` / `sha256_hwrt` must be the **same literals**, guarded by
`vg_density_census.rs`'s `the_pins_declared_byte_identical_actually_agree` (`:270-295`) so a `-Bless`
cannot silently redefine the gate. ⚠️ That requires entries in **both** `VB_PINS` (`:59-91`, 15
entries today) **and** `DECLARED_IDENTICAL_PINS` (`:238-250`) — separate arrays; round 1 named only
the first. No density row is required (`:75-77` answers that).

- **Why byte-identity is the right claim:** a rejected instance never writes `vb_id`; a
  drawn-but-z-failed instance also never writes `vb_id`. The occluder is strictly in front, so there
  are no depth ties and no ordering question.
- **What it CANNOT claim:** that anything was deferred (that is G-P3-B clause 1); that the late phase
  did anything on the three unforced pins (D12 — it correctly does nothing there); and — per the
  frame-index trap — it is GUARANTEED to be satisfied vacuously if the capture is frame 1.
- **Controls, all to be EXECUTED, including the two that must NOT fire:**

| # | corruption | expected |
|---|---|---|
| A1 | invert the verdict (`depth_near > occ`) | **RED on `vb_occ_mixed` and `vb_occ_mixed_late`. GREEN on `vb_occ_mixed_keep`, and that green is EXPECTED** — `FORCE_KEEP` short-circuits the `&& !(occ_flags & FORCE_KEEP)` guard, so no inverted instruction executes. ⚠️ Round 2 predicted RED on all three and gave the reason "the occluder itself is deferred", which **cannot happen**: the occluder is the unmarked slab and the flag is set only on marked entities (`mesh_draw.rs:804-805`). The true mechanism is the other half — the 2 marked-VISIBLE instances are deferred, then dropped by the inverted late test, and **vanish** |
| A2 | `<=` instead of `<` in the verdict | **RED or GREEN, and the answer is a finding.** If this scene cannot reach equality the control does not fire, and **that is reported**, with the boundary case left to G-P3-D's constructed corpus |
| A3 | delete the late cull's `cmd_dispatch` **only** (leave the pass declared and recorded) | **RED on `vb_occ_mixed_late`** — the record's `instanceCount` stays at the host seed `0`, so the 2 late instances vanish. Real because under FORCE_LATE the early cull writes no marked global into `vb_visible_instance`, so late-scope residue cannot coincidentally equal the survivor globals. ⚠️ **GREEN on the three unforced pins, by D12, and that green is expected.** ⚠️ The dispatch call is deleted, not the pass: deleting the pass trips the declare/record parity assert and would be recorded as "RED" for an unrelated reason |
| A4 | force the early phase to defer nothing (`FORCE_KEEP`) | **GREEN** — that is the `vb_occ_mixed_keep` pin, and its green is a claim, not an absence of one |

### G-P3-B — the GPU deferred something, and it partitioned EXACTLY what the oracle says

The load-bearing gate. `BOYKO_VB_CULL_READBACK` + `BOYKO_HZB_DUMP` armed **in the same frame of the
same process** (the trap section), in the `vb_inst_cull_corpus.rs` worker/driver shape.

The host then has: the instance ring and the mesh bounds (it built them), the view-projection (it
pushed it), the pyramid and both depths (dumped), and the GPU's complete partition across **two**
snapshots (D8) — **which reach it only because P3-5 gives them an emitter** (`runner.rs:2924-2954`;
without it every clause below reads a key that does not exist and `field()` panics naming the wrong
defect, `vb_inst_cull_scene/mod.rs:569-571`). It asserts, per batch, in A5's order:

0. **FIXTURE PRECONDITION VG-P3-MIXED-OCCLUDES**, measured form (G-P3-A): over the DUMPED pyramid,
   `occlusion_verdict` is `Reject` for every intended-hidden instance and `Keep(NotOccluded)` for
   every intended-visible one. Its failure message says **FIXTURE**, not defect. **This runs FIRST**,
   because every clause below is meaningless on a scene that cannot occlude.
1. `Σ n_defer > 0` — **the non-vacuity clause, an assert and not a report.**
2. `k + n_defer ==` the frustum-survivor count the CPU census computes. **A cull that "defers" by
   dropping instances outright fails here.**
   **2b.** the early survivor list and the candidate list are **disjoint**, and their union is exactly
   that survivor set. ⇒ **this pair is the gate for INVARIANT VG-P3-RECOVERY**, which round 1 stated
   and left unasserted.
3. every index in the candidate list is a frustum survivor, and the list is strictly ascending with no
   duplicates — the shape the frustum arm already carries at `vb_inst_cull_corpus.rs:411-419`
   (*"a repeat means the compaction cursor did not advance and one instance is drawn twice while
   another is dropped"*).
4. `K_b` = the ordered subsequence of candidates the oracle KEEPs against the **dumped** pyramid.
5. `late_survivors[base .. base + instanceCount) == K_b` **elementwise**, and
   `indirect_late[b].instanceCount == K_b.len()`. ⚠️ `K_b` is derived from the candidate list, never
   from the count the GPU wrote — round 1's clause 5 compared the GPU's number against itself (M8).
6. `late_count_post == late_count_pre` — the late phase does not clobber the early count.
7. **Phase agreement (D12):** on the unforced pin, `Σ|K_b| == 0`. On `vb_occ_mixed_late`,
   **`0 < Σ|K_b| < Σ n_defer`** — the two-sided derived form, not a hard-coded `== 2`. The upper
   bound is what makes it a real claim: it says the late test rejected SOMETHING as well as keeping
   something, and it cannot be satisfied by a late phase that keeps everything.
8. The frame-index triple from the trap section.

Clauses 4/5 are the **oracle equivalence** — an independent implementation of the same predicate over
the same numbers, the standard `vb_inst_cull_corpus.rs` already sets for the frustum arm (*"a
disagreement here is a FINDING … It must be reported, never 'fixed' by editing the expectation"*,
`:19-21`).

- **What it CANNOT claim:** anything about the shipping barrier chain — it runs with the probe armed,
  which appends a TRANSFER read to three buffers (D8). PROBE-OFF is G-P3-F's job. And it cannot
  observe the early phase's verdicts **under motion**: clause 7 works only because D12 makes
  `P_prev == P_cur` on a converged static frame. See Open Question 6.
- **Controls:**

| # | corruption | expected |
|---|---|---|
| B0 | nudge one hidden instance's transform so its rect straddles a 128 boundary | **RED on clause 0**, with the FIXTURE message — and, in the analytic form, **before any GPU runs** |
| B1 | perturb, in the HOST's copy of the pyramid before running the oracle, exactly the texel `select_texels` reports for a NAMED deferred instance, in the direction that crosses its `depth_near` | **RED on clause 4/5 for that instance**. ⚠️ Round 1 said "perturb one texel"; on this fixture a random texel need not be one of the four sampled for any candidate, so the control could silently not fire (M7) |
| B2 | **`keep += 1`** at the end of the late compaction — an OVER-count, which is the class clause 5's independently-derived `K_b` was added to catch (M8's worked miss reads region residue) | **RED on clause 5's elementwise equality AND on its length half**, on `vb_occ_mixed_late` and on `vb_occ_mixed` alike: at the fixed point `K_b` is empty while `instanceCount` is 1, and `late_survivors[base+0]` holds the original candidate (A3's corollary), so a valid-looking global index is drawn while `K_b.len() == 0`. ⚠️ **Round 2 specified `keep -= 1` and that is BOTH mislabelled and dangerous.** It is an UNDER-count, so it does not exercise the class clause 5 exists for; and `keep` is a raw `uint` (`vb_batch_cull.comp.hlsl:441`, stored at `:483-485`), the record word is the only bound on the draw, and `robustBufferAccess` is OFF (C4) — a decrement at `keep == 0`, which D12 **guarantees** on the converged unforced regime, yields `0xFFFFFFFF` instances: a **TDR, not a red.** `keep -= 1` is forbidden |
| B2-bound | — | ⚠️ `keep += 1` is in bounds only because this fixture's eight instances sit far below `INSTANCE_CAPACITY`; `base + n_defer` stays inside the allocation. It must **not** be generalised to a full ring, where the last batch's `base + n_defer` can be `INSTANCE_CAPACITY` and the same C4 argument applies |
| B3 | capture frame 1 (patch `SETTLE_FRAMES` to 0) | **RED on clauses 1 and 8** — the frame-index trap, executed |
| B4 | run on `vb_mesh` (no occlusion) | **RED on clause 1** — the non-vacuity clause fires, which is why it is asserted |
| B5 | make the early phase's occlusion test read `base_extent` off by one | **RED on clause 7** with clause 2 still green — the early phase is falsifiable on a converged frame, and this is the demonstration |
| B6 | delete the late phase's write to `vb_indirect_late` | RED on clause 5 (`instanceCount` stays 0 while `K_b` is nonempty) on `vb_occ_mixed_late` |
| **F-M4a** | record the uniform's `vkCmdUpdateBuffer` **after `cmd_dispatch`** (`vb.rs:1362`) — deterministic in SUBMISSION order | **RED on clause 8's third line**: `gpu_observed_frame_index == frame_index − FRAMES_IN_FLIGHT`. It proves the instrument is live; it does **not** test the barrier (D6) |
| **F-M4b** | move the fill after `record_vb_pass` but keep it before the dispatch — the REAL record-order defect | **GREEN / undetermined, published either way.** C25: `record_vb_pass` records barriers only, so submission order still orders the write before the read; C24: a real missing edge is measured invisible here. A green means "this driver did not reorder", never "the barrier is present" |

⚠️ **Round 1's control B5 ("drop the `firstbithigh(0)` guard") is MOVED to G-P3-D and its prediction
corrected.** It fires only on a sub-texel rect, which "6 small marked instances" does not guarantee;
its trigger is **per-axis** (the unsigned `max` at `hzb.rs:797` lets one un-guarded axis win); and with
the guard dropped everything KEEPs, so the red lands on **clause 7**, never on a "nothing was
rejected" clause — round 1 predicted the wrong clause on the wrong gate.

### G-P3-C — FORCE-LATE: the late scope actually rasterises, and the ordering is real

`BOYKO_VG_OCC_FORCE=late` sets `VB_CULL_OCC_FORCE_LATE`, so the early phase defers **every** marked
instance regardless of the pyramid. The pin `vb_occ_mixed_late` must be byte-identical to
`vb_occ_mixed_off`.

**This is the ONLY regime in which three properties are reachable on a static scene**, and D12 is why:

1. **The late raster path produces correct pixels.** The 6 marked instances are drawn (or correctly
   rejected) by the late scope, through `vb_set0_late`, through the indirection bit, with a
   GPU-written `instanceCount`. The 2 marked-VISIBLE ones survive the late test, so
   **`0 < Σ n_keep < Σ n_defer`** — asserted in that two-sided form (clause 7), never as a hard
   `== 2`, because the count is a property of the geometry and the geometry is what clause 0 pins.
2. **The ordering.** The early depth contains only the unmarked filler, so `depth_early ≠ depth_final`
   **by construction**, and G-P3-E's two-sided clause is non-vacuous.
3. **`late_draws` and `late_cull_dispatches`** are exercised at `draw_batches == 2` — asserted from
   the probe's `[host]` table (`vb_probe_dump.rs:161`) in `vb_occ_split_gate.rs:90-93`/`:541-546`'s
   shape, and delivered by the fixture's TWO mesh registrations (G-P3-A), because batches bucket per
   `MeshHandle` (C27) and nothing about instance counts can produce a second batch. This is the debt
   `vb_occ_split_gate.rs:43-44` records as *"piece 3's first gate"*.

⚠️ **Why the fixture must be MIXED, and why round 1's all-marked FORCE-LATE would have red on a
correct engine.** `vb_mesh.rs:127-134` marks all five spheres or none. With every instance marked,
FORCE-LATE empties the early depth entirely — every texel is the reverse-Z far plane `0.0` — which
trips the SHIPPED non-vacuity clauses at `hzb_engine_pyramid_gate.rs:559-573`. The unmarked filler
exists precisely to populate the early depth. **No fixture in the tree has this shape;
`vb_occ_mixed` is specified in G-P3-A and must be built.**

`VB_CULL_OCC_FORCE_KEEP` is the mirror: the early phase defers nothing, which is exactly today's
behaviour. It is the null control the decidability-floor protocol asks for in the same sitting, the
one-variable baseline of G-P3-A, **and the supported off switch until piece 4** (Boundary). It is also
niagara's `O` key by another name — the field's own answer is a cull-OFF toggle in the shipping
binary.

**Why runtime bits and not a `#[cfg]` or a moving camera:** a gate reachable only by a moving camera
or a debug build is a gate that rots. These are push-constant bits, exercised by committed pins, on
the same binary every other pin uses.

- **What FORCE-LATE cannot claim:** that the *unforced* early phase defers anything (G-P3-B clause 1),
  or that the unforced late phase does anything (D12 — it correctly does not).
- **Controls:** A3 above (its RED lands here and only here); and **C1** — set `FORCE_LATE` and
  `FORCE_KEEP` together, which must trip the `debug_assert` forbidding it rather than silently
  resolving.

### G-P3-D — the SHADER's verdict equals `boyko_render::hzb`'s, with no engine involved

`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs`, the `hzb_build_oracle_gate.rs` shape: its own
pyramid image, its own uploaded instance rows and mesh bounds, its own dispatch of the REAL
`vb_batch_cull` module, its own readback. Because it dispatches the real module with real rows, it
observes the **whole partition** — `vb_visible_instance` vs `vb_late_visible` — not merely the
occlusion verdict, which is what lets corpus 4 gate the frustum side.

Four corpora:

1. **The oracle's own fixtures** — the `7×3` anchor, the `8×16` exact-integer edge, `1×1`, `511×1023`,
   `1920×1080`. These carry the boundary properties the oracle's 26 tests pin, and four are
   non-power-of-two — the configuration Bevy ships its whole feature as *experimental* for
   ("precision issues with non-power-of-two framebuffer sizes, occasionally misclassifying small
   meshes as occluded"). This engine's `prev_pow2` level-0 makes the mapping non-identity, and **that
   mapping is where the field's bugs live**.
2. **A random corpus** — ≥ 100 000 (matrix, AABB) pairs from the committed xorshift, including AABBs
   that straddle the near plane, sit wholly behind the eye, and are off-screen. Each `KeepReason`
   class must be **observed at least once**, and the observation counts are printed — a corpus that
   never reaches `BehindEye` proves nothing about it.
3. **A constructed boundary corpus** — cases with `depth_near == occ` **exactly**, built by planting a
   pyramid texel equal to a corner's computed `z_ndc`. This is the only place the `<` vs `<=`
   difference is decidable, and it is the one difference that deletes geometry.
4. **The SENTINEL corpus (new in round 2, and B4's fold).** `MeshLocalBounds::UNKNOWN`
   (`min = +1e30, max = −1e30`) paired with (a) a normal affine and (b) an **exactly zero linear part**
   (`Transform::from_scale(Vec3::ZERO)`, an unguarded public `const fn` at
   `boyko_scene/src/transform.rs:92`). **Both must land in the EARLY survivor list** — never deferred,
   never dropped. ⚠️ **Reachability, with the mechanism corrected (D11).**
   `mesh_geometry_table.rs:587-588` states that *"A slot that is never registered keeps the
   [`MeshLocalBounds::UNKNOWN`] prefill"*, and `VB_GEOMETRY_RESERVED_SLOT` is exactly such a slot
   (`mesh_draw.rs:536`, doc `:500-515`). What keeps the sentinel rare on the committed corpus is that
   the gather EXCLUDES non-resolvable meshes entirely (`mesh_draw.rs:657-661`) — **not** that
   `register` couples bounds to geometry, which is what round 2 claimed. A mesh that IS registered and
   never received a VB geometry slot reaches the ring carrying the sentinel. ⇒ the case is
   **reachable in principle on the shipped engine**, guaranteed absent by nothing, and gated here,
   where the harness uploads its own bounds.

- **What it CANNOT claim:** that the ENGINE's cull reads the right pyramid, the right ring, the right
  matrix or the right extent — it builds its own everything. That is G-P3-B's job. This is verbatim
  the G3/G8 division piece 1 established.
- **Controls:**

| # | corruption | expected |
|---|---|---|
| D1 | `ceil(hi)-1` instead of `floor(hi)` | RED on corpus 1's exact-integer-edge extent (`hzb.rs:1190-1191`) |
| D2 | clamp `level` down to `levels-1` instead of KEEPing | RED — `keep_case_level_unavailable_never_clamps_down` (`hzb.rs:1936-1937`) shows this is a FALSE REJECT |
| D3 | drop the `firstbithigh(0)` guard on ONE axis | RED on corpus 1's `1×1` layout, where single-texel rects are unconditional and the unsigned `max` (`hzb.rs:797`) lets the un-guarded axis win. **Moved here from G-P3-B (M7): on an engine fixture the trigger is not guaranteed; on a `1×1` layout it is** |
| D4 | **hoist the sentinel guard after the Arvo fold** (round 1's shape) | **RED on corpus 4** — case (a) is frustum-deleted, case (b) is occlusion-deleted. The control that proves B4 was a real defect and not a style note |
| D5 | swap the explicit `precise` fold for `dot()` (D11 / M5) | **report whether the differential moves. A null result IS the finding** — it measures whether this driver reassociates `OpDot`, in the narrowed shape P1 §10 established. Do not "fix" a null result by keeping `dot()`: the precedent at `cluster_cull.hlsl:124-140` is about what the spec PERMITS, not what this driver did today. ⚠️ There is **no artifact-level pin** behind this control — the `OpDot == 0` census round 2 specified is unimplementable (D11), so what backs it is the source-level sentinel gate plus this differential, and nothing else |
| D6 | drop `precise` from the projection locals | report whether the differential moves — it measures contraction on this device. ⚠️ **This one DOES have an artifact-level pin**: the `NoContraction` decoration count (D11) must move. If it does not, `precise` is not reaching the artifact and that, not the differential, is the finding |

### G-P3-E — G5 under a DRAWING late scope, two-sided, SPLIT BY REGIME

Extends piece 2's `hzb_engine_pyramid_equals_the_oracle_occ` with the D10 payload. ⚠️ **Split in two,
because D12 makes a single clause set self-contradictory:**

**On `vb_occ_mixed_late` (FORCE-LATE):**
1. `build_pyramid(depth_early) == pyramid`, bit-exact, all five shipped non-vacuity clauses intact —
   including `:561-567`, which the unmarked filler exists to satisfy;
2. `depth_early ≠ depth_final` at ≥ 1 texel — **asserted**, and guaranteed by construction;
3. `build_pyramid(depth_final) ≠ pyramid` — the pyramid was NOT built from the final depth.

**On `vb_occ_mixed` (unforced, converged):**
1. `build_pyramid(depth_early) == pyramid`, as above;
2'. `depth_early == depth_final`, **byte-for-byte, asserted** — the fixed point (D12) as a positive,
   falsifiable claim rather than an embarrassment. If the late scope ever draws a pixel on a converged
   static frame, this reds.

Clause 3 is the ordering proof piece 2 could not make. ⚠️ Round 1 asserted clause 2 on the unforced
fixture, where it is a **hard red with no defect present**.

- **What it CANNOT claim:** the ordering, on the unforced pin. Clause 2' is a different claim, not a
  weaker version of clause 2.
- **Controls:** (E1) move the poison+build block back after the late scope — clauses 1 and 3 both red;
  (E2) swap the two depth regions in the dump writer — clause 1 reds on the FORCE-LATE pin and clause
  2' stays green on the unforced one, which is the pair that proves the two regions are not
  interchangeable.

### G-P3-F — the derived barrier stream, PER CONFIGURATION, FIELD by FIELD

Extends `vb_barrier_stream_baseline.rs`. Piece 2's round 1 learned this the hard way: a barrier COUNT
is the assertion that certifies the defect it exists to catch. Fields, not counts —
`a_dropped_writer_keeps_every_count_and_moves_only_fields`
(`vb_barrier_stream_baseline.rs:4356-4421`, the `#[cfg(not(debug_assertions))]` at `:4356`) is the
shape, and its debug counterpart `the_dropped_survivor_write_now_trips_the_framegraph_guard`
(`:4434-4443`, `#[should_panic(expected = "reads UNWRITTEN transient buffer")]` at `:4436`) covers the
other leg.

**Configurations pinned:** U-rows (unsplit, the pyramid seed change, P3-0); S-rows **PROBE-OFF** — the
shipping chain, normative; S-rows **PROBE-ON** — the same plus the readback's appended TRANSFER reads.
Round 1 pinned one set and would have certified a perturbed chain as the shipping one.

New rows: the pyramid's cross-frame seed on the U-rows; `vb_late_visible`'s COMPUTE→COMPUTE RAW **and
its self-WAR execution-only edge** (the cost of D8's split declaration, pinned rather than hidden);
`vb_late_count`'s COMPUTE→COMPUTE RAW; `vb_indirect_late`'s chain
(`TRANSFER→COMPUTE→DRAW_INDIRECT`, and `→TRANSFER` only under PROBE-ON); the pyramid's
`hzb_build`→`vb_cull_late` RAW at `GENERAL` with **no layout change**; `vb_raster_late`'s two new
VERTEX reads.

- **Controls:**
  - **F1** — declare `vb_cull_late`'s `vb_indirect_late` write at the WRONG STAGE (`TRANSFER` instead
    of `COMPUTE_SHADER`). Every count is preserved; `vb_raster_late`'s `src_stage` moves
    `COMPUTE_SHADER → TRANSFER` and `src_access` moves `SHADER_WRITE → TRANSFER_WRITE`. ⚠️ **Round 1's
    F1 ("delete the write declaration → src reverts to TRANSFER, count stays the same") is
    unreachable**: `sync.rs:373-383`/`:400-405` make the src the LAST DECLARED WRITER, so deleting it
    leaves the upload as that writer and the COUNT DROPS — the opposite of the stated outcome, and
    `vb_indirect_barrier_chain.rs:104-108` already pins the working shape.
  - **F2** — delete the pyramid read on `vb_cull_late` → the RAW against the build vanishes.
  - **F3** — declare `vb_raster_late`'s `vb_late_visible` read at `FRAGMENT` → the derived edge moves
    stage with no count change.
  - **F4** — delete `vb_batch_cull`'s `vb_late_count` write declaration → **the P2-8 provenance guard
    fires** in every dev-profile golden (`graph.rs:692-724`, the assert at `:703-720`, the latch at
    `:722`), because that buffer's first touch is that write. This is the one new buffer the guard
    protects, and F4 demonstrates it.
- ⚠️ **What it CANNOT claim:** that `declare_vb_graph` writes this shape. It is a hand-written
  REPLICA, exactly as `framegraph_gbuffer_equiv.rs` says of itself, and **P2-7 measured that a missing
  barrier in the production declarator is green on all four gates**. The gap is closed only partially,
  by the production `debug_assert`s (D8) which run in every dev-profile golden
  (`graph_bridge.rs:5071-5076`), by F4's guard, and by `VbRecordProbe::late_cull_dispatches`, which is
  the only number in this piece that originates in the real recorder. **`vb_indirect_late`'s
  provenance is covered by nothing** (D8) — that sentence belongs in the commit message.

### G-P3-G — validation, armed vs unarmed, message for message

The P1-2 / P1-4 / G3 leg. It is the leg **credited** with seeing: the `SampledImageAtGeneral`
descriptor's type and recorded layout against a mip-complete view of an image in `GENERAL` (C15),
`hzb_null`'s layout after `boot_seed_hzb_null` (D7), `vb_late_visible` / `vb_late_count` usage bits,
the widened push range against `maxPushConstantsSize`, the 12-entry set against the 12-entry layout,
and the second and third descriptor sets.

It runs at **P3-0** (the seed/clear commit), again at **P3-1** on a DISARMED pin — the step that
mints `hzb_null` — and again at **P3-4**, the step that first puts @9 in the module's STATIC access
set. Round 1 deferred it to P3-1 and would have shipped a
VUID-VkImageMemoryBarrier-oldLayout-01197 through a step whose gate set could not see it; round 2
listed no validation leg on P3-4 at all, which is the step where `hzb_null`'s recorded layout first
becomes an obligation.

#### ⚠️ It is the one gate in this piece with no demonstrated red, and that is now a control, not a footnote

The section header of this document requires every gate to carry at least one control that has been
shown RED. Round 2 exempted this one silently while making it the sole claimed coverage for the
descriptor-layout class. Two facts make the exemption untenable:

- **Synchronization validation is measured NOT live here** (C24): a genuine missing RAW produced
  19 messages, no `SYNC-HAZARD`, and a byte-identical image.
- **The entire 19-message baseline is `vkCreate*`-time** (`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1657-1659`).
  **No record-time validation message has ever been observed from this leg on this machine** — and
  BOTH classes G-P3-G is credited with are record-time.

⇒ **Two controls, to be EXECUTED at P3-4, with the result published either way:**

| # | corruption | expected |
|---|---|---|
| **G1** | bind `vb_cull_set_hzb` with a plain `BindGroupEntry::SampledImage` entry against the `GENERAL` pyramid — i.e. record `SHADER_READ_ONLY_OPTIMAL` over an image that is never in it. `-ValidationOn`, record the message delta **verbatim** | **UNDETERMINED.** This is a descriptor-layout mismatch checked at record/submit time, and no record-time message has ever been seen here |
| **G2** | delete `boot_seed_hzb_null`'s transition, leaving `hzb_null` in `UNDEFINED` under a descriptor recording `GENERAL`, on a DISARMED pin. `-ValidationOn` | **UNDETERMINED**, same reason |

**If either emits, G-P3-G's credit is real and the message class is recorded for future use.**

⚠️ **If NEITHER emits, that is the finding and the plan pays it in full:**

1. Strike the credit — the phrases "a core-validation error at the arming commit" (D7) and "this is
   the check round 1 would have failed" are **deleted**, not softened.
2. `SampledImageAtGeneral` is **kept anyway**, on the enum-discipline argument alone (*the kind names
   the layout*, D7): a descriptor that records a layout the image is never in is wrong whether or not
   a layer says so, and the variant costs one enum arm in each of two matches.
3. Write the statement the gate section demands: **"the descriptor-layout class has NO gate that can
   be shown red in this repository. Its coverage is the type system — one variant, two exhaustive
   matches, an E0004 if either is missed — and code review."** That sentence goes in the commit
   message.

**What it CANNOT claim, in any outcome:** anything about barriers. C24 settles that — a missing
barrier is invisible to G-P3-G, invisible to G-P3-A and invisible to the probe. **G-P3-F and the
production `debug_assert`s are the only barrier evidence**, and G-P3-F is a hand-written replica by
its own admission.

### Mandatory unit tests

- `VbCullUniform` size/offset const-asserts; `VB_LATE_VISIBLE_ELEMS == VB_VISIBLE_INSTANCE_ELEMS`;
  `VB_LATE_COUNT_ELEMS == VB_INDIRECT_LATE_RECORDS + 1`.
- `VB_BATCH_CULL_PUSH_BYTES == 112`, the `rhi_impl/mod.rs:227-232` const-assert verified still live,
  and `scene_types.rs:549-551`'s message updated off 104.
- `vb_cull_layout`'s entry-array length const-asserted against the derived binding count (the "7 → 12,
  not 7 → 10" defect, mechanically).
- The math-row inversion: `view_proj_rows` built from the push bytes equals the matrix
  `frustum_planes_from_push_bytes` (`gpu_scene/mod.rs:6480-6484`) was derived from — one test, both
  consumers.
- `path_vb_occlusion_split()` false with `hzb: None`, false with `vb_mesh_bounds: None`, true only
  with all five conjuncts.
- `HzbDumpLayout`: `total_bytes`, both depth offsets, `HZB_DUMP_HEADER_SCALAR_WORDS`-derived offsets,
  and that the OLD magic fails to decode.
- The compaction lemma as a host model: a proptest over `(n_defer, keep-mask)` asserting the in-place
  compaction preserves ORDER, never reads a clobbered slot, and leaves `[keep, n_defer)` holding the
  original entries (A3's corollary — the property the two-snapshot design rests on).
- `vb_cull_batch_count_visible_clamp` unchanged (its tests at `vb.rs:4096-4241` must not move).
- **The degrade policy (D2), with its control.** Force `boot_clear_hzb_pyramid` to fail; assert
  `HzbTargets::build` returns `Err` and that no `GBufferTargets` is constructed — hence no lockstep
  panic (`targets.rs:7781-7785`) and no `GENERAL` seed over a null image is reachable. Control: make
  the same failure return `Ok(None)` and show the lockstep assert fire.
- **`boot_seed_hzb_null`'s teardown ladder (D7):** encoder and fence are destroyed on **every** path,
  including the `Err` paths, and `hzb_null` itself is destroyed by each of the three
  `GBufferTargets::create` error arms (`targets.rs:7049`, `:7110`, `:7153`).
- **The probe-line round trip (M5):** `format_vb_cull_probe_line` → `parse_probe_line` over all six
  new keys, with **ragged, non-contiguous, and EMPTY per-batch groups** — the case `vis=`'s own
  comment (`runner.rs:2905-2916`) says the format must survive. This is the one gate for the emitter
  that can red without a GPU.
- **The source-level projection sentinel gate (D11):** the extractor must FAIL LOUDLY on a missing
  `// === PROJECTION FOLD BEGIN/END ===` marker rather than silently match an empty range — a gate
  that asserts over nothing is the failure mode this campaign has shipped six times.
- **`VbCullReadbackLayout`'s two new regions:** offsets and `total_bytes`, two-sided against
  `boyko_app`'s constants in the shape `gpu_scene/mod.rs:6747-6756` already uses
  (`VbCullReadbackLayout` is `pub(crate)` in `vb.rs:260`, ctor `:322-356` — NOT in `scene_types.rs`).

### `debug_assert!` invariants

- `plan.vb_cull_late.is_some() == scene.path_vb_occlusion_split()`, at both declare and record.
- The four declare-order asserts (D8) — none of which equates `hzb_build`'s presence with
  `vb_cull_late`'s (fact 17).
- Every late record's `first_instance == 0` (`vb.rs:1220-1223`, **survives**), and the host seed is `0`
  (D9's replacement, whose message is now true).
- The late push carries bit 1 only with bit 0 (`vb.rs:1596-1602`'s shape, applied to the late scope).
- `n_defer + k <= instance_count` — a HOST-side check on the READBACK, since the shader cannot assert.
- `late_visible[fi].size / 4 >= visible_elems` and the same for `late_count` (mechanical fact 2;
  asserted, never min-ed).
- `scene.vb_occ_flags & VB_CULL_OCC_ARMED != 0` implies `scene.hzb.is_some()`.
- `VB_CULL_OCC_FORCE_LATE` and `VB_CULL_OCC_FORCE_KEEP` are never both set (control C1).
- `VB_LATE_COUNT_FRAME_SLOT == VB_INDIRECT_LATE_RECORDS` is a fixed CONSTANT, never `batch_count`.
  The size backstop must be `>= VB_INDIRECT_LATE_RECORDS + 1`; a `batch_count + 1` bound would green a
  real out-of-bounds on the shipped multi-batch corpus.
- The uniform fill is recorded BEFORE `record_vb_pass` (C17/C25). ⚠️ There is **no runtime assert
  that can check this** — `pass_access_count` is private and no per-pass accessor exists (C29) — so
  it is carried by the `// SAFETY:` comment and by control F-M4a, and by nothing else. Stated here
  rather than listed as an invariant the code enforces.

---

## Boundary — what piece 3 does NOT do

**No config knob, and the disarm route is NAMED.** `HzbConfig` gains no variant; no `OcclusionConfig`
is introduced. **Arming** is `OcclusionCulling` markers on scene instances AND the pyramid AND the
mesh-bounds table (D9). **Disarming**, until piece 4, is `BOYKO_VG_OCC_FORCE=keep` — a production
push bit whose shader branch is exercised by a committed pin (`vb_occ_mixed_keep`), which is what
makes it a supported switch rather than shipped surface nobody tests — or removing the markers.
**Piece 4 owns the owner-facing config field.** Round 1 shipped `FORCE_KEEP` with no pin and no
stated role; the sixth lens was right to call that a missing disarm story.

⚠️ **And it is a DISARM of the DECISION, not of the allocation.** `hzb_null`, the three buffers, the
widened layout and the second/third descriptor sets are minted on every VB boot regardless
(Data structures). There is no route in piece 3 that un-allocates them, and — per D2 — no route by
which a runtime failure disarms anything, because `GBufferScene::hzb` is a host plan computed once
(C21). A real allocation-backed disarm is the larger change D2 costs out and Open Question 8 records.

**No `vkCmdDrawIndexedIndirectCount`, no `vkCmdDispatchIndirect`, no device-feature change.** D4
explains why neither is needed, not merely why neither is affordable.

**No change to `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl` or `vb_geom_fetch.hlsli`** — a gate-quality
commitment (D3 / D5), not an accident.

**No change to `vb_visible_instance`, `VB_VISIBLE_INSTANCE_ELEMS`, `INSTANCE_CAPACITY`,
`vb_cull_batch_count_visible_clamp` or the R2d-6 const-assert.** They appear in the diff as context
only; if any moves by one character the piece has re-created the R2d-6 collision.

**No `sync_gbuffer` reorder** (D5 / M11), no rewrite of `hzb_depth_ring`'s selection
(`targets.rs:7763-7766`), and no new RHI update-one-binding helper —
`RhiDevice::create_bind_group` writes the whole set once (`rhi_impl/device.rs:318-648`, the single
`vkUpdateDescriptorSets` at `:641`), so a second set is cheaper than a partial update.

⚠️ **Two round-2 Boundary claims are WITHDRAWN, because D7's `hzb_null` costs them and pretending
otherwise is how a plan ships a surprise:** `DeferredSets::build` **does** gain an eleventh parameter
(`targets.rs:2800-2817`), and the three `GBufferTargets::create` error arms (`:7049`, `:7110`,
`:7153`) **do** each gain one `hzb_null` destroy line. M11's refutation is untouched — it concerned
the PYRAMID, whose view depends on struct fields that do not exist until `targets.rs:7628-7720`, and
`hzb_null` depends on none of them. What remains true is the narrow form: no reorder, no
`hzb_depth_ring` rewrite, no pyramid drain in those arms.

**No fix for the framegraph provenance guard's buffer exemption.** Piece 3 makes
`vb_indirect_late`'s provenance uncovered (D8) and does not close it; P2-7's prescribed fix
(`is_write || res_written || res_seeded` + the 14-site audit) stays in `docs/OPEN-QUESTIONS.md`.

**No per-instance dispatch.** The cull remains **one lane per batch with a serial inner loop** (C6).
For a batch with 1000 instances one lane performs 1000 projections. That is a shipped property of
R2c0 / R2d-6, not a regression, and changing it means re-deriving the region-write scheme that
currently needs no atomic. **Named because it is the first thing to look at if a measurement ever
shows the cull dominating** — and P3-8 now produces the first such measurement.

**No previous-frame reprojection**, no `prev_view_proj`, no previous transforms. D1 states why; the
cost is early-phase hit rate only.

**No second pyramid build.** Bevy builds two per frame; nothing here consumes a pyramid of the final
depth, so one build serves both phases (D1).

**No occlusion for shadow cascades, for the SDF leg, or for any non-VB path.** `path_is_vb()` and
`mesh_leg` remain conjuncts.

**No mesh-level or cluster-level (meshlet) occlusion.** The cull unit stays the INSTANCE. The
research's granularity caveat is discharged for this rung (R2d moved the engine to per-instance) and
finer granularity is the R4+ ladder.

**No overflow policy**, because overflow is unreachable (D3).

**No perf CLAIM and no benchmark pin — but three numbers are measured and published as prose** (Goal,
P3-8). Round 1 removed perf from scope twice and measured nothing; that is how piece 4 inherits an
architecture whose first number is a surprise.

**No fix for the 19 outstanding validation messages.** Owner-deferred.

---

## Open questions

1. **DESIGN, for the critic.** Should the projection leaf become a `boyko_shaderdsl` leaf rather than
   a hand-authored mirror (D11)? It would buy bit-exactness by construction; it costs a rewrite of
   `hzb.rs`'s projection chain as a generic body while keeping its 26 pins green, in the same campaign
   that arms a decision. My call is **hand-author now, gated by G-P3-D**, and convert only if G-P3-D
   shows a disagreement not attributable to a stated cause. Round 2 strengthens the fallback: control
   D5's null result is recorded either way, so the evidence for converting accumulates whether or not
   a defect appears.

2. **VALUES, owner.** `VB_CULL_OCC_FORCE_LATE` / `_FORCE_KEEP` ship as production push bits driven by
   an env knob, and `FORCE_KEEP` is now the named disarm route (Boundary). They cost two bits and make
   five properties gateable on a static scene. The alternative is `#[cfg(debug_assertions)]`, which
   would make the controls unreachable in release-leg CI **and would leave the piece with no off
   switch at all until piece 4**. My call is **production bits** — it is also what niagara ships — but
   it is a values call because it puts a debug affordance in shipping code.

3. **MEASUREMENT, now scheduled rather than deferred.** P3-8 runs FORCE_KEEP vs ARMED vs DISARMED with
   a zero control in one sitting and publishes the three numbers as prose. What remains open is the
   *hit-rate* question — current-view-proj-vs-stale-pyramid (D1) versus previous-view-proj — which
   needs an occlusion-heavy scene with a moving camera and a rejection count over N frames. It changes
   no pixel and no correctness property.

4. **VERIFY AT P3-6.** Whether `vg_density_census_gate` requires a measured density row for each new
   pin. `vg_density_census.rs:75-77` answers it for `vb_occ_split` (*"It needs no density row … the
   pin list is (a)'s domain and nothing else"*), and the four new pins are the same kind — but the
   answer is stated for one pin, not as a rule. Read it before authoring the bump. ⚠️ Round 1 filed
   this as an open question against `:57-64`, which is the `VB_PINS` doc, not the answer.

5. **VERIFY AT P3-2.** `hzb_null`'s necessity given that the device already enables
   `descriptorBindingPartiallyBound` for the bindless set. `hzb_null` is 4 bytes, needs no feature and
   is the specified answer; the alternative is recorded only so a reviewer does not raise it as an
   omission. ⚠️ Note that partial binding would NOT remove the layout problem C15 names — that is
   `SampledImageAtGeneral`'s job either way.

6. **A limit that is NARROWED, not closed.** Round 1 declared the early phase's verdicts permanently
   unobservable. D12 refutes that **for converged static frames**: `P_prev == P_cur` bit-for-bit
   there, so the dumped pyramid is the one the early phase read, and G-P3-B clause 7 gates it with a
   showable red (control B5). What remains unobservable is the early phase **under motion or under
   FORCE-LATE**, where the two pyramids genuinely differ and only one is dumped. Closing that would
   mean dumping the pyramid TWICE per frame — a third staging region and a second dump pass —
   deliberately not proposed, and recorded here so it is a known gap rather than a discovered one.

7. **A limit with NO control, stated because the checklist demands it.** Nothing in this repository can
   see a missing barrier in the production **declarator** — P2-7 measured all four gates green on
   exactly that defect, and G-P3-F is a replica by its own admission. Piece 3 adds `vb_late_count`,
   whose first touch is a compute write and on which the provenance guard IS live (control F4), and
   simultaneously retires the guard's reach over `vb_indirect_late` (D8). **Net coverage of the
   declared-reader-with-no-declared-writer class is unchanged: one buffer gained, one lost.** No gate
   in this piece closes it, and the fix is a framegraph-core change P2-7 already specified.

8. **DEFERRED, and costed rather than waved at.** An ALLOCATION-BACKED arm bit — so a runtime
   pyramid failure could disarm the split instead of failing the build (D2). It requires moving
   `hzb_arm`, `path_vb_occlusion_split()`, D5's set selector and D2's graph seed off `scene.hzb` (the
   host plan, C21) and onto a flag plumbed back from `GBufferTargets` into the scene the recorder
   reads — a second arm-bit pair of exactly the kind rung R2d-2 already paid for once
   (`gpu_scene/mod.rs:6488-6492` vs `:6496-6501`). **Larger than this piece's Boundary allows.**
   Until it exists, `Err` is the correct answer and `Ok(None)` stays a single-producer gate.

9. **A limit with NO control, and it is the one the round-2 critique named as the largest residual
   risk.** The intra-pass `TRANSFER → COMPUTE` edge for `VbCullUniform` (D6) has **no executable red
   on this machine**: F-M4b leaves submission order intact, sync-validation is measured dead (C24),
   G-P3-F cannot see it (the derived stream is identical under the defect), and no image gate can.
   F-M4a proves only that the INSTRUMENT is live. ⇒ the production surface F-M4 justifies
   (`VbCullUniform.frame_index`, the frame slot, the lane-0 store, two probe fields) is defended by a
   control that tests something adjacent to the defect, and **that is written into the commit message
   rather than discovered later.** The honest alternative — a framegraph-core assert that a pass's
   TRANSFER work precedes its barrier emission — needs the per-pass accessor C29 says does not exist,
   and is not taken here.

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
10. `VK_COMPARE_OP_GREATER`, strict, depth-write ON (`device.rs:1806-1813`, `scene_types.rs:2769`); `vb_raster.fs.hlsl:3` — no `SV_Depth`, no `discard`, no UAV. A redraw at identical depth is **pixel-invisible** (`vb_occ_split_gate.rs:50`), and the committed fixtures contain no coincident geometry, so the early/late reorder is byte-safe on them. `vb_mesh.rs:127-136` already constrains marking "all five or none" for exactly this reason.
11. `hzb.rs:855` is strict `<` with equality KEEPing. `hzb.rs:698-709` guards with `!(min <= max)` — NaN-aware; `any(mn > mx)` is not.
12. `cluster_cull.hlsl:127-141` is this repo's own reasoned rejection of `dot()` for host-oracle agreement (`OpDot` is "inherited from a formula", transformable by associativity); the implemented remedy is `:142-143`. `precise` ⇒ `NoContraction` ⇒ forbids contraction, **not** reassociation.

**Probes / gates / harness**
13. `vb_cull_readback` is a **mid-frame** copy declared before `vb_raster` and recorded immediately after the cull dispatch (`graph_bridge.rs:3921-3947`, `vb.rs:1362→1444`). `n_defer` is observable there; `n_keep` is not.
14. `runner.rs:2601-2621`: the readback fires on the first **presented** frame, has no settle, and `return`s from **outside** the `:2763` exit conjunction — whose own comment names that hazard. The probe line (`:2924`, `:2949-2953`) carries **no frame index**.
15. `vb_inst_cull_scene/mod.rs:646-650`: one capture per process is a deliberate convention, and its `env_remove` list omits `BOYKO_HZB_DUMP`. Same at `vb_occ_split_gate.rs:449-451`, `hzb_engine_pyramid_gate.rs:495-497`.
16. `vb_probe_dump.rs::write_probe` (`:144-168`) is the **only** serializer of `VbRecordProbe`; `vb_occ_split_gate.rs:414-417` `field()` **panics** on a missing key.
17. Goldens run **dev profile** (`graph_bridge.rs:5071-5072`) — every `debug_assert` is live in a golden run. `[vb_mesh_hzb]` (`PINS.toml:335-341`) sets `BOYKO_VG_HZB=1` with **no** `BOYKO_VG_OCC`, so any assert equating `hzb_build` presence with `vb_cull_late` presence panics on a correct configuration.
18. `vg_density_census.rs`: `VB_PINS` (`:59`) and `DECLARED_IDENTICAL_PINS` (`:238`) are **separate** arrays; a new byte-identical pair must land in both.
19. `mesh_leg == false` ⇒ `MeshGeometryTableSlot(None)` ⇒ `vb_mesh_bounds: None` ⇒ `batch_cull_armed == false` (`mesh_geometry_table.rs:19-27`, `runner.rs:2352-2356`, `vb.rs:964-969`) — the early cull is not recorded at all. `vb_occlusion_instances` and the `OCCLUSION_CULLING` flag lane are written as one branchless pair (`mesh_draw.rs:804-806`), so `== 0` iff no ring slot carries the bit; the over-approximation is one-directional (`scene_types.rs:3219-3222`). **But** `render_path_config.rs:952-954` + `runner.rs:607-631` make `occlusion_split && !batch_cull_armed` reachable on a device without `storage_buffer_array_non_uniform_indexing` — under the plan's record-parity mandate that hits a `.expect()` on the absent `vb_cull_set`/pipeline (`targets.rs:4617-4645`). Add the conjunct at the dispatch site.
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

---

# ROUND 2 CRITIQUE — three lenses, scoped to what round 2 CHANGED

**Round 2 DID close round 1.** All four of its blockers were attacked and survived: the two-snapshot
design is sound and the cost genuinely is avoided by the siting; the settle/drain triple is anchored
by a shader-written word no host latch can move; the word-reuse reversal really does give the late
record one producer and really does buy an image-level red; the sentinel guard is restored
character-exact against the shipped shader.

**Round 3 is required because both remaining blockers live in material round 2 ADDED** — the degrade
policy and the sampled-image escalation. Round 2 fixed what it was told to fix and opened two new
holes in the same commit. That is the shape to watch: the count is falling (4 blockers → 2) and the
NEW material is where they now are.

⚠️ Line numbers are as-of-this-critique. Re-anchor at use.

# VERDICT

**REJECTED**

Round 2 **did close round 1**. All four of round 1's blockers were attacked and survived attack: B1's two-snapshot design is sound and the cost genuinely is avoided by the siting (refuted); B2's settle/request/drain triple is not true-by-construction and is anchored by a shader-written word no host latch can move (refuted); B3's word-reuse reversal really does give the late record one producer and really does buy an image-level red (confirmed by A3's mechanism, refuted as pixel-invisible); B4's guard is restored character-exact (`vb_batch_cull.comp.hlsl:448-458` matches D11 `:989-995` and both normative shapes `:1018-1034`). Round 1's M9 — "a probe field that never reaches a serializer" — was fixed on `vb_probe_dump.rs` as promised.

Round 3 is required because **both remaining blockers live in material round 2 ADDED**: the new degrade policy (D2) and the new `SampledImageAtGeneral` escalation (D7/C15). Round 2 fixed what it was told to fix and opened two new holes in the same commit.

---

# BLOCKERS

## BLOCKER 1 — D2's degrade policy is refuted by the shipped code in the engine's own words

**Anchor:** `docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md:330-337` vs `crates/boyko_rhi_vulkan/src/present/targets.rs:7714-7718`, `:7780-7784`, and `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:3746-3747`, `:3837-3841`.

Re-anchored this session. `:7718` is `hzb_arm: scene.hzb.is_some()`; `:7780-7784` is `debug_assert_eq!(targets.hzb_arm, targets.hzb.is_some(), "invariant: the stored HZB arm and the pyramid allocation move in lockstep")`. A second `Ok(None)` producer whose precondition is `scene.hzb == Some` makes those two disagree — the assert the plan cites as the thing that "keeps the graph, the sets and the arming predicate consistent" is the thing that **fires**. The comment three lines above it states the shipped contract: *"the stored bit and the allocation can only disagree if the build fails — which returns."*

Release is worse than debug. `vb.rs:3746-3747` reads *"a create failure returns `Err` rather than a silent `None`"*, and `vb.rs:3837-3841` is a release-live, per-frame, unconditional `.expect("invariant: scene.hzb armed => targets.hzb")`. So the degraded generation panics every frame in **every profile**, at **P3-0** — before D9's conjunct or D5's selector exist. `GBufferScene::hzb` is the host `Option<HzbPlan>` (`scene_types.rs:3136`), computed once in the runner: no runtime failure can change it, so D9's new `hzb.is_some()` conjunct disarms nothing and the `GENERAL` seed stands over `VkImage::NULL`. The plan's closing sentence at `:337` — "No path exists in which the seed is `GENERAL` and the clear did not run" — is false as written.

**Plan-text change.** Replace `:332-337` with the `Err` branch round 1 also offered:

> ⇒ a failed clear tears down through the existing ladder and returns **`Err`** from `HzbTargets::build` — the shape `targets.rs:7715-7717` already names and `vb.rs:3746-3747` already relies on. `Ok(None)` stays reserved for the 0%-gate at `targets.rs:1258-1261`. The lockstep assert (`targets.rs:7781-7785`) is the check on that, not a mechanism that absorbs a failure.

Delete the clauses crediting D9's conjunct and C10 with disarming the split. Add to *Mandatory unit tests* (`:2050-2067`): force the clear to fail and assert the configuration is self-consistent (no lockstep panic, no `GENERAL` seed over a null image). If a real disarm is wanted instead, the plan must say that `hzb_arm`, `path_vb_occlusion_split()`, the set selector and the graph seed all move onto an allocation-backed flag — which is a larger change than this piece's Boundary allows, and `gpu_scene/mod.rs:4005-4012` records that rung R2d-2 already paid for exactly that second-arm-bit pair.

## BLOCKER 2 — `hzb_null`'s `UNDEFINED → GENERAL` is unreachable on the majority configuration

**Anchor:** `docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md:313-316` and `:1565` vs `crates/boyko_rhi_vulkan/src/present/targets.rs:1258-1261`.

Re-anchored this session. `:313-316` gives `boot_clear_hzb_pyramid` as `hzb_null`'s only transition owner ("one encoder, two images"), and that helper lives behind `HzbTargets::build`, whose **first statement** is the 0%-gate `let Some(plan) = scene.hzb else { return Ok(None); }` — before any encoder, fence or barrier exists. `hzb_null` is minted and bound at `vb_cull_set` @9 **unconditionally** (`:1288-1294`, file row `:1565`). So on an `HzbMode::Off` boot — every committed pin but one — @9 records `GENERAL` over an image that is still `UNDEFINED`, at every `vb_batch_cull` dispatch, from P3-4 onward. The escalation exists to make that descriptor legal and leaves it illegal on the path that always takes it. The plan's own criterion at `:313-316` convicts it; the precedent it cites (`csm.rs:394` `seed_boot_layouts`, called unconditionally at `csm.rs:365`) is an unconditional boot helper, not a branch of an armed-only builder.

**Plan-text change.** Give `hzb_null` its own unconditional boot transition owned by `GBufferTargets::create` in the `seed_boot_layouts` shape (one encoder + fence + submit + wait, teardown on every path), separate from `boot_clear_hzb_pyramid`'s armed-only clear. Name the call site in D2 and in the `:1565` row; state the usage bits (`SAMPLED` is required for the @9 view); land it at **P3-1** (the step that mints `hzb_null`), not P3-0 — `:1628-1632` currently says "transitioned by P3-0's helper" one commit before the image exists. Add the `-ValidationOn` leg to **P3-4's** gate list (`:1660-1670`): P3-4 is the step that first puts @9 in the module's static access set, and its gate list currently names none. Correct `:2036` ("`hzb_null`'s layout after D2's boot transition") — on the unarmed leg that transition never happened — and correct the pin arithmetic to "every committed pin but one" (`goldens/PINS.toml` has 25 sections; only `[vb_mesh_hzb]:335-341` sets `BOYKO_VG_HZB`).

---

# MAJORS

**M1 — F-M4, the piece's only executable record-order control, predicts a deterministic red for a race that is measured invisible on this machine.** `:575-582`, `:1196-1197`, `:1870`. Three lenses reached this independently. `record_vb_pass` (`graph_bridge.rs:5172`) records barriers only; `passes/vb.rs:1301 → :1362` has no barrier between it and `cmd_dispatch`, so the corruption leaves the `vkCmdUpdateBuffer` **still ahead of the dispatch in submission order** and only moves the derived TRANSFER→COMPUTE edge. `vb_barrier_stream_baseline.rs:20-24` records that a genuine RAW between adjacent compute dispatches, deleted, produced 19 messages, no SYNC-HAZARD and a byte-identical image. The plan asserts at `:2047` that a missing barrier is *invisible to the probe* and at `:1772` that this missing barrier reds a probe clause. **Fix:** respecify the corruption to be deterministic in submission order (record the update **after** `cmd_dispatch`) and pin `gpu_observed_frame_index == frame_index − FRAMES_IN_FLIGHT` against that; keep the barrier-side move as a second control whose expected column reads GREEN/undetermined, publish its result either way, and state what a green is allowed to mean (in particular: not "the barrier is present"). Reconcile `:2047` with `:1772`.

**M2 — the `OpDot == 0` census pin is unimplementable, so M5's explicit-precise-fold fix ships with no artifact-level gate.** `:1567`, restated `:1668`. `vb_batch_cull_spv_sync.rs:230` counts `OpDot` **module-wide** in a flat token loop, pinned `op_dot: 10` at `:356`, and `:410-412` carries a live `assert!(actual.op_dot > 0 …)` the plan does not touch. The eight `dot()` calls are in code the plan declares UNCHANGED (`vb_batch_cull.comp.hlsl:391-392`, `:452-457`). **New measurement:** a byte-scan of the committed `.spv` for the 5-word `OpFunction` header finds exactly **one** — DXC inlines every helper into `%main` — so scoping a census to a function range is unreachable on this artifact, not merely unbuilt. **Fix:** replace with a source-level gate on the projection helper's text plus a pinned count of `NoContraction` decorations, and correct `:1567`/`:1668` to say module-wide `op_dot` is **re-measured**, not asserted 0.

**M3 — the mixed fixture's occlusion property is stated too weakly to hold, and `Σ n_keep == 2` reds a correct engine.** `:1785-1786`, `:1888`. `select_texels` returns `containing_texel(t, level) = t >> level` (`hzb.rs:300-302`, `:790-808`) — the **aligned** expansion, up to ~2 rect-widths — and `conservative_min` folds all four (`:817-825`); with reverse-Z clear `0.0` (`vb.rs:52`) one background texel forces KEEP. At 512×512 `base == source`, so a rect straddling x=256 selects level 8 and the footprint is the whole image. "Wholly behind the slab's silhouette" is necessary, not sufficient. **Fix:** state the requirement in pixels (silhouette ⊇ rect expanded to the level-`msb(tx0^tx1)`-aligned 2×2 footprint) and assert it as a **fixture precondition** in `vg_occ_verdict_census.rs`, computed from the same `project_aabb`/`select_texels` the oracle uses, so a structurally-cannot-defer fixture reds as a fixture error textually distinct from clause 1. Turn `:1888` into the two-sided derived form `0 < Σ n_keep < Σ n_defer`, with clause 5 carrying survivor identity. Also supply concrete geometry (slab mesh, transforms, near/far). The plan already names this hazard class at `:686` (Bevy #14042) and then does not carry it into the fixture.

**M4 — `vb_occ_mixed_off`, the baseline of the four-pin identity set, cannot be produced.** `:1792`, with `:1788` vs `:1803-1804`. `vb_mesh.rs:135` reads `BOYKO_VG_OCC` as `is_ok_and(|v| v == "1")` and the scene is an unconditional five-sphere loop (`:137-157`): the variable is both scene selector and marker switch, and D9 (`:856`) makes it imply the HZB arm. With env `—` the pin renders the existing `[vb_mesh_hzb]` configuration, so the four-pin equality at `:1797-1798` is unsatisfiable. The row is also self-contradictory: `:1788` puts `BOYKO_VG_HZB="1"` in all four blocks while `:1803` counts "the pyramid's entire existence" among `off`'s differences. **Fix:** mint an orthogonal scene selector (`BOYKO_VG_SCENE=mixed`) so `off` is *mixed scene, no markers, no HZB*; restate the env table; and say explicitly whether `off` carries `BOYKO_VG_HZB`, which decides whether the contrast is four variables or five. `vb_occ_mixed_keep` — the pin the M10 argument actually rests on (`:1802`) — is producible and unaffected.

**M5 — the two new readback snapshots have no emitter: round 1's M9 reproduced on the cull probe channel.** `:1577` (the `runner.rs` row budgets only the two scalars), P3-5 `:1674-1677` (stops at the decode). `format_vb_cull_probe_line` (`runner.rs:2924-2954`) is the only serializer of `VbCullReadback` and `runner.rs:2613-2618` the only channel to the driver process, so `late_candidates`, `late_count_pre`, `late_survivors`, `late_count_post` and `indirect_late` are decoded and dropped — G-P3-B clauses 3–7 and A5's adjudication have no data, and `field()` panics naming the wrong defect (`vb_inst_cull_scene/mod.rs:569-571`). The plan states the governing rule itself at `:892-894` and applies it to one probe only. **Fix:** add the five `key=[…]` groups to the `runner.rs` row and to P3-5's gate list, **with per-batch grouping** (`base:members`, sized by `late_count_pre[b]` pre and `indirect_late[b].instanceCount` post) — `vis=`'s own comment at `runner.rs:2905-2918` records that batch regions need not be contiguous — plus a format→parse round-trip unit test.

**M6 — G-P3-G is the only gate exempted from the section's own "carries at least one control that has been shown RED" rule (`:1723-1724`), and it is the sole claimed coverage for the descriptor-layout class.** `:2032-2048`. Every other gate has a corruption table (`:1819`, `:1862`, `:1913`, `:1955`, `:1986`, `:2010`). `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1657-1659` records that the entire 19-message baseline is `vkCreate*`-time — no record-time message has ever been observed from this leg — while both classes G-P3-G is credited with are record-time. **Fix:** add and **execute** control G1 (bind `vb_cull_set_hzb` with a plain `SampledImage` entry against the GENERAL pyramid; `-ValidationOn`; record the delta verbatim), plus a second control for `hzb_null`'s post-boot layout. If neither emits, that is the finding: strike the "core-validation error"/"the check round 1 would have failed" credits at `:637-638` and `:2034-2035`, keep the variant on the enum-discipline argument at `:640-644`, and write the no-control-possible statement `:1723-1724` requires.

**M7 — control A1's expected outcome is wrong on one of the three pins it names, and its rationale clause is false.** `:1821`. Under `FORCE_KEEP` the guard `&& !(occ_flags & FORCE_KEEP)` (`:1023`, `:1430`) short-circuits, `n_defer == 0` propagates (`:1436`, `:1457`), and inverting the verdict changes no executed instruction — `vb_occ_mixed_keep` is GREEN. "The occluder itself is deferred" cannot happen: the occluder is the unmarked filler and `mesh_draw.rs:805` sets the flag only on marked entities. **Fix:** RED on `vb_occ_mixed` and `vb_occ_mixed_late`, GREEN on `vb_occ_mixed_keep` recorded as an expected non-firing control (A3 at `:1823` already shows the form); replace the rationale with "the 2 visible marked instances are deferred, then dropped, and vanish."

**M8 — control B2 is mislabeled and is a device-loss hazard on the pins where it cannot fire.** `:1865`. `keep -= 1` is an under-count; M8's miss (`:1506`, `:2293`) is an over-count, the class that reads region residue — so clause 5's independently-derived `K_b`, added precisely to catch M8, gets no control exercising its class. And with `uint k = 0u` stored raw (`vb_batch_cull.comp.hlsl:441`, `:483-485`), the record word the only bound on the draw, and `robustBufferAccess` OFF (`gpu_scene/mod.rs:280`), a decrement at `keep == 0` — guaranteed on the converged unforced regime by D12 — yields `0xFFFFFFFF` instances: a TDR, not a red. **Fix:** specify `keep += 1`, restrict the control to `vb_occ_mixed_late`, and state why it must not be run unforced.

**M9 — `draw_batches >= 2` (`:1891`) is delivered by nothing the fixture states.** Batches bucket per `MeshHandle` (`mesh_draw.rs:935-952`); `:1782-1786` names "a slab" and "a smaller object" without a registration, and `vb_mesh.rs:118-119` registers exactly one mesh today. `draw_batches` appears exactly once in the document — as the claim. If the fixture ships single-batch, the debt `vb_occ_split_gate.rs:43-44` records as "piece 3's first gate" stays open silently, and the late path's per-batch state at `i > 0` gets no coverage at all. **Fix:** state the registrations and assert `draw_batches == 2` from the probe's `[host]` table, the way `MULTI_BATCHES` is derived and asserted at `vb_occ_split_gate.rs:90-93`, `:541-546`.

**MINOR — the file list is short by one.** `BindGroupEntry` has **two** exhaustive matches in `boyko_rhi_vulkan` — the write arm at `rhi_impl/device.rs:605` and `bind_group_entry_kind` at `rhi_impl/mod.rs:157-175` — and the enum is not `#[non_exhaustive]` (`boyko_rhi/src/device.rs:346`), so P3-1 as listed is an E0004. `StorageImageView`, the shape D7 says it copies, required both arms when it landed. Add the row; correct `:1628-1629`'s "its Vulkan arm" to name both.

---

# MECHANICAL FACTS ESTABLISHED

Anchored, re-verified this session. These are transferable beyond this piece.

1. **`HzbTargets::build`'s only `Ok(None)` is the 0%-gate**, and it is the *first statement* — before any encoder, fence or barrier (`targets.rs:1258-1261`). Any helper folded into that function is armed-only by construction.
2. **The lockstep pair is `hzb_arm` (from the scene) vs `hzb` (the allocation)** (`targets.rs:7718`, `:7780-7784`), and three shipped `.expect()`s gate on the *scene* side (`vb.rs:3432-3439`, `:3779-3783`, `:3837-3841`). A silent `None` is a release panic, not a degrade. `vb.rs:3746-3747` states this contract in prose.
3. **`GBufferScene::hzb` is the host PLAN, not the allocation** (`scene_types.rs:3136`), computed once in the runner (`gpu_scene/mod.rs:5690-5697`). No runtime failure can flip it, so no predicate derived from it can disarm anything in response to one.
4. **`boot_clear_taa_hist` (`targets.rs:6279`) is an UNCONDITIONAL twin** of the `hwrt`-gated `boot_clear_shadow_temporal_hist`; the tree cross-references the cfg difference itself at `:6277` and `:6236-6238`. The `#[cfg]` is an item attribute, not part of the copied shape.
5. **`csm.rs::seed_boot_layouts` (`crates/boyko_app/src/gpu_scene/csm.rs:394`, called unconditionally at `:365`) is the precedent for an unconditional boot layout transition** — no cfg gates in that file.
6. **DXC inlines every helper into `%main`: exactly one `OpFunction` in the committed `vb_batch_cull.comp.spv`.** There is no artifact-level function range to scope a SPIR-V census to, on this or any comparable module.
7. **A real RAW between adjacent compute dispatches, deleted, was invisible on this machine** — unchanged 19-message baseline, no SYNC-HAZARD, byte-identical golden (`vb_barrier_stream_baseline.rs:20-24`). And **the entire 19-message baseline is `vkCreate*`-time** (`VG-R3-P2 plan:1657-1659`): no record-time validation message has ever been observed from the `-ValidationOn` leg here.
8. **`record_vb_pass` records barriers only**; the dispatch is recorded by the caller with nothing between (`framegraph/record.rs:50-56`, `passes/vb.rs:1301 → :1362`). "Move the fill past the barrier" does not move it past the dispatch.
9. **`select_texels` returns the ALIGNED expansion** `containing_texel(t, level) = t >> level` (`hzb.rs:300-302`, `:790-808`); `conservative_min` folds all four (`:817-825`); reverse-Z clear is `0.0` (`vb.rs:52`). One background texel in the footprint forces KEEP. At 512×512 `base == source`, so a rect straddling the midline selects the top level and the footprint is the whole image.
10. **Batches bucket per `MeshHandle`** (`mesh_draw.rs:935-952`): batch count is a mesh-registration property of the fixture, not an instance-count property.
11. **`format_vb_cull_probe_line` (`runner.rs:2924-2954`) is the only serializer of `VbCullReadback`**, its file (`runner.rs:2613-2618`) the only channel to the driver process, and `parse_probe_line`'s `field()` panics on a missing key (`vb_inst_cull_scene/mod.rs:569-571`). **`vis=` regions need not be contiguous** (`runner.rs:2905-2918`), so any list emit needs per-batch grouping.
12. **A windowed boot owns the device singleton and the window** (`vb_inst_cull_scene/mod.rs:634-636`): two framings or two camera paths cannot be rendered by one process. "In the same run" can only ever mean one *sitting*.
13. **`DRAIN_FRAMES = 3 > FRAMES_IN_FLIGHT = 2`** (`hzb_dump.rs:47-51`, `present/mod.rs:82`), and the dump advances only on `presented == true` (`hzb_dump.rs:152-155`) while `frame_index` increments on every iteration including recreate-skips (`runner.rs:2549-2553`, `:2790`). The drain relation is `>= DRAIN_FRAMES`, never `==`.
14. **The dump header's `frame_index` is stamped by the RECORDER** inside the copy frame's command buffer (`passes/vb.rs:3488-3507`); `HzbDump::finish` writes mapped bytes verbatim with no host header write (`hzb_dump.rs:183-201`).
15. **`VbCullReadbackLayout` is `pub(crate)` in `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:260`**, not in `scene_types.rs`.
16. **`VB_LATE_COUNT_FRAME_SLOT == VB_INDIRECT_LATE_RECORDS` is a fixed constant, not `batch_count`** (`:1249`, `:1421`). The size backstop must be `>= VB_INDIRECT_LATE_RECORDS + 1`; a `batch_count + 1` bound would green a real OOB on the shipped 7-batch corpus.
17. **`robustBufferAccess` is OFF** (`gpu_scene/mod.rs:280`) and the GPU-written record word is the only bound on the late draw and on the VS dereference. A `uint` under-decrement of a zero count is a device loss, not an assertion failure.
18. **The framegraph provenance guard runs inside `compile()` on the PRODUCTION declarator** (`graph.rs:692-724`), and `vb_late_count`'s first in-graph touch is a write — so the guard is live on it (control F4). But **`pass_access_count` is private** (`graph.rs:158`) and no per-pass accessor exists in the public surface (`graph.rs:1189-1254`): a declarator-side access-count assert requires new framegraph-core API.
19. **Debug asserts run in golden runs** — dev-profile builds, `golden.ps1` carries no `--release` (`graph_bridge.rs:5071-5072`).
20. **Under `FORCE_LATE` the early cull writes no marked global into `vb_visible_instance`**, so late-scope residue cannot coincidentally equal the survivor globals — which is what makes A3's image-level red real.

---

# IS THIS SAFE TO IMPLEMENT?

Not yet, but it is close, and the gap is narrower than round 1's. Round 2's *design* is sound where it matters most: the B3 reversal is the real thing — one GPU producer for the late record, a host seed of 0, and therefore a missing late cull that draws nothing and reds an image — and it converts the piece's headline claim from a byte-identity assertion into a falsifiable one. B1's two-snapshot shape, B2's frame-index triple and B4's restored guard all survived direct attack. What must go back is a small, bounded set: two boot-time decisions round 2 added without checking them against `HzbTargets::build`'s shape (both fixable in a few sentences, and one of them is a one-line change to the shipped-code shape the tree already prescribes), one missing serializer row, one unproducible pin, one unimplementable census pin, and a control table whose predictions are wrong on three rows. None of that is a redesign; all of it is text.

**The single largest residual risk is that this piece's gates are calibrated against a machine where synchronization is unobservable, and round 2 responded to that by making a *stronger* claim rather than a hedged one.** F-M4 is the concentration point: it is the only executable control for a record-order defect, its corruption leaves submission order intact, and the campaign has *measured* that a real missing barrier on this hardware produces no message and no pixel. If it greens — which is the likely outcome — the piece ships believing it has record-order coverage it does not have, and the new production surface justified by that control (`VbCullUniform.frame_index`, the frame slot, the lane-0 store, two probe fields) becomes untested shipped surface. Every other finding is a gate that reds loudly on a correct engine or a step that fails to compile; this is the one that can go green while the defect is present, which is precisely the failure mode that has now cost this campaign six vacuous gates.
