# Architecture: VG R3 piece 2 of 4 — the occlusion-culling CAPABILITY and the VB raster SPLIT, inert

Status: **DESIGN, round 2.** Round 1 was `APPROVED_WITH_CHANGES`; the critique is preserved verbatim
at the end of this document as the record of how the design got here. Every blocker and major is
folded into the text above it — a reader gets the corrected design by reading top to bottom, and
must not reconstruct it by diffing against the critique.

Scope fixed by `docs/OPEN-QUESTIONS.md:167-212` ("RESOLVED 2026-08-03 — decomposed"), piece 2
verbatim: *"The capability and the raster split alone, inert — the second scope drawing nothing,
proven byte-identical on the pins."*

> **Anchors.** Every `file:line` below was re-verified against the tree at commit `9e80cd4`
> (2026-08-06). Where the round-1 text or the critique cited a line that has moved or never existed,
> the corrected anchor is used and the correction is called out at the point of use. Piece 1 lost a
> round to stale anchors (`VG-R3-P1-PYRAMID-PLAN.md` §12); treat every anchor here as *name + hint*
> and grep the name.

## Goal

Land the two structures piece 3's occlusion test needs, with **no occlusion decision anywhere**:

1. **The capability** — "this instance participates in occlusion culling" — as an ECS fact, with the
   seam to boot-minted GPU objects named and anchored.
2. **The raster split** — `vb_raster` becomes `vb_raster` (early) + `vb_raster_late`, with the
   poison+pyramid-build block between them, the second scope drawing nothing.

**Functional target:** on every scene in the tree today, byte-identical output.

**Cost, stated honestly** (this replaces round 1's "zero added cost (structural skip, not a branch)",
which was false in both halves — the design has neither a skip nor a branch on the unmarked path,
and the Boundary section already said "piece 2 adds cost and removes none"):

| frame shape | added cost |
|---|---|
| **any frame, marked or not** | one extra `u32` lane in the gather (`build_view().clear()` + one `push` per row + one `Option<&ZST>` probe per row), one extra sequential `u32` load in the ring build, one `u32` field on `GBufferScene`, one 20 KiB×FIF device buffer reserved at boot. **Zero extra passes over the query, zero binds, zero recorded commands. The uploaded ring bytes are byte-IDENTICAL on every scene that exists today**, because `flags == 0` is exactly what `_pad[0]` already carried. |
| **frame with `OcclusionCulling` on ≥1 ringed instance** | one `vb_indirect_late_upload` pass (⌈`draw_batches`/64⌉ `vkCmdUpdateBuffer` commands), one extra `vkCmdBeginRendering`/`EndRendering` pair, `draw_batches` zero-`instanceCount` `vkCmdDrawIndexedIndirect`, `3 × draw_batches` binds/pushes, and **3** derived barriers at the late scope's boundary (HZB off) — see G4 for the full per-configuration count. |

"Structural skip" is reserved for the **recording**, where it is true: with no marked instance the
late scope is not declared and not recorded at all.

## Context and constraints

**Affected:** `boyko_render` (marker, gather lane, instance row, caster gather), `boyko_app` (gather
query, draw-list fold, `GpuSceneBundles` allocation + scene assembly, tests), `boyko_rhi_vulkan`
(`declare_vb_graph`, `record_vb`, `VbBarrierSink`, `GBufferScene`, test fixtures).

**Invariants that must survive untouched:**

| invariant | anchor (re-verified) |
|---|---|
| `VB_VISIBLE_INSTANCE_ELEMS == INSTANCE_CAPACITY` (R2d-4 ⊇ / R2d-6 ⊆, both directions) | `crates/boyko_app/src/gpu_scene/mod.rs:258-264` |
| `first_instance: 0` in every record (`drawIndirectFirstInstance` is VK_FALSE) | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:976`, assert `:979-982` |
| INVARIANT R2d-REGION-DEFINED (every dereferenced slot written this frame) | `crates/boyko_rhi_vulkan/shaders/vb_batch_cull.comp.hlsl:118-153` |
| INVARIANT R2d-REGION-DISJOINT (bases strictly ascending) | same file, `:101-116` |
| declare/record ORDER parity | `vb.rs:317-320` (doc), `graph_bridge.rs:3084-3085` (doc) — ⚠️ **both are DOC COMMENTS, not asserts**; the only real order asserts in `declare_vb_graph` are the HZB trio at `graph_bridge.rs:4698-4726` |
| `hzb_pyramid` is ResId **last among images** in both `cfg` arms | `graph_bridge.rs:3264-3268`, `debug_assert_eq!` at `:3272-3276`; `VB_IMAGE_COUNT` = 21 (hwrt) / 15 at `:2924-2928` |
| `hzb_poison` is declared BEFORE every `hzb_build` | `graph_bridge.rs:4711-4717` |
| every `hzb_build` is declared BEFORE `hzb_dump` | `graph_bridge.rs:4698-4704` |
| `hzb_poison.is_some() == hzb_dump.is_some()` | `graph_bridge.rs:4722-4726` |
| `multiDrawIndirect` is NOT enabled ⇒ `draw_count ∈ {0,1}` | `vb.rs:1422-1424` |
| `robustBufferAccess` is OFF | `gpu_scene/mod.rs:256-257` |

**Two hard local constraints from `docs/VG-R3-TWO-PHASE-OCCLUSION-RESEARCH.md`, and what they
change** (round 1 did not know either):

1. **`vkCmdDrawIndexedIndirectCount` is not available, and it is not one line away.**
   `crates/boyko_rhi_vulkan/src/device.rs:615-618` states the reason verbatim: *"The `Count` variant
   is deliberately NOT loaded: it needs `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features`
   this device never chains."* A repo-wide case-insensitive grep for `IndirectCount` returns eight
   hits, **all comments or docs** — `device.rs:617`, `ffi.rs:3340`, and six in `docs/`. So this is
   not a fn-table line: it is a device-feature chain edit that changes device creation for *every*
   frame, on behalf of a piece whose whole claim is inertness. **Out of scope for pieces 2 and 3.**
   ⇒ niagara's "reuse one buffer, refill the count to 0" is structurally unavailable here, so **the
   late scope's record storage is a decision this plan makes explicitly** — see D4.
2. **The engine already sits in the per-entry-empty-draws configuration** that vkguide and pcwalton
   warn about: a fixed-length `VkDrawIndexedIndirectCommand` array where a culled entry gets
   `instanceCount = 0` (`scene_types.rs:462-464`, *"A culled batch gets `0` written over it"*), drawn
   with `vkCmdDrawIndexedIndirect`. It is a **PREFIX, not a mask** — the early loop is already bounded
   by a hoisted `draw_batches` local (`vb.rs:935-940`), not by the 1024 allocation. ⇒ the late loop
   **must use the same hoisted bound**, not `record_capacity_late`. That is a change from round 1 and
   it directly caps the cost the research could not quantify: the late scope records exactly as many
   empty draws as the early scope records real ones, never 1024.

**A third research finding, recorded for piece 3 rather than acted on here:** the standard HZB tap is
*not* point-sampled `textureLod`. It is either a `VK_SAMPLER_REDUCTION_MODE_MIN` sampler (niagara) or
four `textureLoad`s with a shader-side `min` (Bevy `occlusion_culling.wgsl`). **The second matches
this engine's `.Load`-only discipline in `hzb_build.comp.hlsl` and avoids a
`VK_EXT_sampler_filter_minmax` dependency, so it is the one piece 3 adopts.** Linear filtering of a
reduced pyramid is *wrong* — a bilinear blend of four reduced texels is a convex combination and is
therefore neither an upper nor a lower bound of the footprint, which under reverse-Z + `min` reduce
produces false negatives, i.e. missing geometry.

## Key decisions

### D1: The capability is a **table-storage** ZST opt-IN component, `OcclusionCulling`, per ENTITY, read non-filtering as `Option<&OcclusionCulling>`

**This reverses round 1's `storage = "bitset"`.** Round 1 picked the EnableTag mechanism while
arguing capability-PRESENCE semantics; that combination does not compile and its naive repair makes
G1 vacuously green. The tree answers the fork in one line, inside the very file round 1 cited as its
precedent — `crates/boyko_render/src/csm_caster.rs:173`:

```rust
q: Query<(&MeshHandle, &InstanceModelCol), (Enabled<RenderEnabled>, With<ShadowCaster>)>,
```

`With<ShadowCaster>` = **Axis-1, structural capability** (a table ZST, `csm_marker.rs:25-26`, no
`storage` attribute). `Enabled<RenderEnabled>` = **Axis-2, runtime on/off** (a bitset EnableTag).
Both in one query, one line apart. Occlusion-culling participation is a property of the object
*kind* — a skybox, a first-person weapon and a UI proxy never participate, and that is decided at
spawn, not toggled per frame. **Axis-1 ⇒ table storage.**

**What.** `#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)] pub struct
OcclusionCulling;` in a new `crates/boyko_render/src/occlusion_marker.rs`, with
`const _: () = assert!(size_of::<OcclusionCulling>() == 0);` — verbatim the `ShadowCaster` shape
including its const-assert (`csm_marker.rs:28-29`).

**Why table, not bitset — the real trade-off** (round 1's rationale was a false dichotomy and is
deleted: `Option<&T>` *is* the non-filtering per-row read for table storage, `option.rs:56`, so "not
an archetype filter" never argued for bitset):

| | table ZST (chosen) | bitset EnableTag (rejected) |
|---|---|---|
| non-filtering per-row read | `Option<&OcclusionCulling>` — `matches_component_set` is unconditionally true (`option.rs:96-101`), `aggregate_include` is a no-op (`:104-107`), so it never drops or reorders a row | only `IsEnabled<T>`; `Option<&T>` cannot resolve (no column, `data_is_enabled.rs:12-15`) and `With<T>` matches zero archetypes (bitset ids are stripped from every signature, `archetype.rs:314-324`, `archetype_master.rs:144-150`) |
| spawnable in a bundle | yes | **no** — `boyko_macros/src/component.rs:315` (`hooks.no_bundle \|\| hooks.storage_bitset \|\| hooks.storage_dense`) suppresses the `Bundle` impl, so `spawn((MeshBundle, OcclusionCulling))` and `insert` do not compile |
| `#[require(...)]` reachable | yes | **no** — a required ctor is `unsafe fn(dst: *mut u8)` (`required.rs:49`) writing into a pool a bitset id has none of; it reaches `migration_helpers.rs:725-728` and panics on `.expect("invariant: target hosts every required id")` |
| default for a never-touched entity | **absent** — genuinely no marker | **`false`** — every never-toggled row reads disabled, so "presence is the datum" is false in both halves |
| declares scheduler access | yes, a real read | **no** — `IsEnabled::init_access` is a documented no-op (`data_is_enabled.rs:170-176`) |
| **cost paid** | **a marked subset FRAGMENTS the mesh archetype in two, shortening per-archetype runs in the gather** | none (bitset ids never fragment) |

That last row is the one genuinely distinguishing cost, and it is accepted: the fragmentation is
bounded at 2× archetypes for the mesh family, the gather is already per-archetype chunked, and in the
steady state a game marks *all* its meshes, which collapses back to one archetype. `Enabled<T>` is
additionally disqualified outright — it **drops rows** (`filter_enable.rs:295-309`, stated at
`data_is_enabled.rs:16-17`), which is incompatible with the lock-step scatter this lane requires.

**`Has<OcclusionCulling>` does not exist anywhere in this plan, because it does not exist in this
kernel.** The single workspace-wide `Has<` hit is a doc comment about *resources* at
`crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs:11`; `HasRelation<R>`
(`ecs/core/iters/query/relation/filter.rs:40`) is an unrelated `QueryFilter`.

**Opt-IN, not opt-out**, and the direction is the safe one: absence ⇒ never occlusion-culled ⇒ never
wrongly vanished. An entity type added in two years by an author who never heard of this feature
cannot be silently deleted from the frame. The cost — a game must mark its meshes — is payable by
`#[require(...)]`, which under table storage actually works (`light.rs:284` is the in-tree shape).

**Alternatives rejected.**
- *`NoOcclusionCulling` (opt-out)* — the default becomes "cull unless told otherwise", i.e. a new
  component's failure mode is *invisible geometry*. Rejected on the asymmetry the tree already
  states for unknown bounds: *"Absence of bounds is not evidence of invisibility"*
  (`mesh_geometry_table.rs:171-174`).
- *A `Resource`-only knob* — cannot express per-object policy, and forces the "cull everything"
  default this design rejects.
- *A runtime `bool` field on a component* — forbidden by the standing rule; `hzb_config.rs:16-22`
  argues the identical point for itself (`enabled()` is a derived predicate, not stored state).

**Lifecycle, which round 1 never stated.** A table-storage insert triggers an archetype migration
applied at the next command flush, so an entity marked in system *S* is invisible to
`gather_mesh_draws` until that flush — **the split arms one frame late.** That is the safe direction
(one frame of extra draws, never missing geometry) and it is stated so no gate is authored against
an unstated frame. **Every fixture in this piece marks at SPAWN, inside the bundle** — never a later
`insert` — so "which frame does G2 read" has one answer: every rendered frame.

**Trade-off.** A scene must opt in, so the feature is invisible until someone marks something. That
is exactly why piece 2's gates need their own marked fixtures (G1, G2) and why the 25 existing pins
are blind to the split (see "What a pin cannot claim").

### D2: The seam — a per-frame FOLD at the host gather, producing one per-instance flags word and one frame-level count. Boot mints unconditionally.

This is the round-3 blocker: *"a capability that is a per-frame ECS fact gating objects minted at
boot with no seam named between them."*

**The seam already exists in this tree, on this path, one function above the VB draw-list build**,
and it has a name:

```
csm_armed = resolved_csm.csm_mode_word == 1 && casters.batch_count() > 0
                    ^^^ owner Resource knob        ^^^ structural: at least one entity HAS the capability
```
`crates/boyko_app/src/runner.rs:2009`.

The cascade pipeline and its descriptor-set layout are minted at boot, unconditionally.
`ShadowCaster` presence is a per-frame ECS fact. They meet at a **per-frame conjunction computed at
the host gather, threaded onto the scene struct, and read by declare and record alike** — never at
the minting site. Piece 2 reproduces that seam exactly:

| layer | what piece 2 does | precedent (re-verified) |
|---|---|---|
| ECS | `OcclusionCulling` on entities, table ZST | `ShadowCaster`, `csm_marker.rs:25-26` |
| gather | a parallel `ScratchColumn<u32>` **flags** lane scattered in lock-step with `ring`, **fused into the primary scatter** (zero extra query walks) | the `material_ids` lane's own fused shape: `build_view().clear()` at `mesh_draw.rs:646`, store at `:702`, inside the primary loop `:671-708`. ⚠️ **Round 1 cited `mesh_draw.rs:1179-1185` for this; that is the comment introducing `gather_material_tex_into`'s SECOND, non-fused walk** — the exact anchor collision the critique flagged. The non-fused walk is the *contrast*, not the precedent. |
| per-instance GPU datum | folded into `VbInstanceRow._pad[0]` → `flags` by `sync_vb_instance_ring` | `VbBatchDesc::base_instance` occupying R2c0's reserved `pad`, `scene_types.rs:468-476` |
| frame-level predicate | `GBufferScene::vb_occlusion_instances: u32` (a plain `u32` — this crate cannot depend on `boyko_render`) | `vb_classify_material_count`, a plain `u32` field at `scene_types.rs:2909-2925` |
| the single source read by declare AND record | `GBufferScene::path_vb_occlusion_split()`, a **derived predicate, not stored state** | `HzbConfig::enabled()` is derived from `mode != Off` rather than stored (`hzb_config.rs:16-22`, `:128`). ⚠️ note `vb_use_classified` (`scene_types.rs:2926-2947`) is a **FIELD**, not a method — round 1 cited it as the method precedent; the derived-predicate precedent is `HzbConfig::enabled`. |
| boot | `vb_indirect_late` minted **unconditionally**; no pipeline, no layout, no set is added at all | `vb_visible_instance` — "MANDATORY, deliberately NOT part of the R2c0 all-or-nothing arm", `scene_types.rs:2809-2812` |

**Why the P1-4/§9 "arm lives on the TARGETS" shape does NOT transfer here, and this is a deliberate
divergence.** §9's shape works because the pyramid *is* a target: `HzbTargets` is `None` when
disarmed, so presence and arming are one object. The split allocates **no new target**. Its one new
resource, `vb_indirect_late`, belongs on `GpuSceneBundles` beside `vb_indirect`, not on
`GBufferTargets`. And piece 1 §7 records the reason hanging it there would be a *bug*: `sync_gbuffer`
short-circuits on `(extent, aa_arm)` alone, so a per-frame ECS fact carried on the targets **cannot
survive a runtime flip at fixed extent** — precisely what a component appearing mid-run is. The arm
therefore belongs on the per-frame scene struct. **One predicate, computed once per frame, at one
site.** There is no second predicate that could disagree with it, which is §9's actual requirement.

**Per-INSTANCE, not per-batch — and the per-batch fold would be UNSOUND.** `ShadowCaster` is folded
to per-batch `casts_shadow` (`runner.rs:1968-1969`), with the documented consequence *"a mesh with
ANY caster instance casts with ALL its visible instances"* (`runner.rs:1943-1952`). For shadow
casting that over-approximation is safe. For occlusion it is not: an OR-fold would make an *unmarked*
instance eligible for rejection because a sibling sharing its mesh was marked — the one error
direction that deletes geometry. Per-instance costs **zero extra bytes and zero extra device
fetches**: `VbInstanceRow` is 64 B with `_pad: [u32; 3]` at offset 52 (`instance_model.rs:232`), in
the same 16-byte lane as `mesh_id` @48, and `vb_batch_cull.comp.hlsl` already loads
`gVbInstances[base_instance + j]` for every candidate. The flag arrives in a load the shader already
issues.

**The CSM caster gather shares the fold core, and this is stated rather than left to be discovered.**
`CsmCasterScratch` is a tuple struct wrapping the same type — `pub struct CsmCasterScratch(pub
MeshRenderScratch);` (`csm_caster.rs:88-89`) — and `gather_shadow_casters` calls the shared
`gather_mixed_into` through the closure at `csm_caster.rs:199`. So the caster gather runs the
occlusion fold on **its own scratch**, producing a second instance of the datum. Two consequences,
both handled:
- The caster query at `csm_caster.rs:173` **also gains `Option<&OcclusionCulling>`**, so the caster
  scratch's lane and count are *truthful* rather than a hard-coded `false`. `Option<&ZST>` is
  non-filtering and free; a lane that lies is worse than a lane that is redundant.
- The runner reads **the main scratch** (`runner.rs:2386`/`:2390` is the established shape for
  `any_non_default_material` / `any_textured_material`). `CsmCasterScratch.0` is `pub`, so
  `.0.occlusion_instances()` remains *reachable*; it is now merely redundant, never wrong. Making it
  structurally unreachable would mean privatising the tuple field, a wider edit than this piece
  earns. Recorded as a named, checkable diff condition (the D5 discipline): **if a future diff reads
  `occlusion_instances()` off anything other than the main `MeshRenderScratch`, that is a second
  predicate and it is a defect.**

**Per-frame reset — round 1 had none, and without it the split arms permanently.**
`MeshRenderScratch` is `#[derive(Resource)]` (`mesh_draw.rs:265-266`), i.e. it persists across
frames. Every sibling per-frame reduce in the *same function* is reset first, with the reason written
beside it — `mesh_draw.rs:609-612`, the first statement of `gather_mixed_into`:

> *"reset the per-frame PM pipeline-selection flag BEFORE the scatter recomputes it below — a
> persistent `Resource` field must not stay sticky-true after a material is removed."*

`occlusion_instances` gets `self.occlusion_instances = 0;` in that same opening block, and
`any_textured_material`'s reset at `:964` is the second sibling. Round 1's cited model —
`CsmCasterScratch::batch_count()` (`csm_caster.rs:94-97`) — is a *derived* count over a column
cleared per gather (`mesh_draw.rs:803`), structurally incapable of going sticky; a bare scalar is
not. The regression test is a two-gather test modelled on `mesh_draw.rs:1864-1884`, not the
single-gather count check, which is green with or without the reset.

**Who clears the lane.** The `inst_flags` column is a `ScratchColumn<u32>` and gets
`build_view().clear()` in the same block as its siblings — `ring` at `mesh_draw.rs:627`, `mesh_ids`
at `:636`, `material_ids` at `:646` — via `ScratchBuildView::clear()`
(`crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:69`). Without it a shrinking ring leaves
stale non-zero tail entries that `sync_vb_instance_ring`'s zip never reads (it is bounded by
`ring.len()`) but that the fold would already have counted. That is a *separable* defect from the
scalar reset and both are specified.

**Trade-off.** `_pad`'s "unused, always zero" contract (`instance_model.rs:231`) and its unit pin
(`:275`) change. That is a documented, deliberate edit — the same one `VbBatchDesc::pad →
base_instance` made — not a silent reuse.

### D3: The split is gated on the CAPABILITY ALONE in piece 2; piece 4 AND-s in the config knob

```
path_vb_occlusion_split() = path_is_vb() && resolved_render_path.mesh_leg && vb_occlusion_instances > 0
```

A **derived predicate**, not a stored flag — the `HzbConfig::enabled()` discipline
(`hzb_config.rs:16-22`, `:128`): a second stored bool is a second thing that can disagree.

**Why not also gate on a config knob now.** A split nobody can record is a split nobody can prove
inert. Gating on a knob that piece 4 introduces would make every piece-2 gate vacuous by
construction — the exact failure mode this campaign has now shipped five times. Gating on the
capability makes the split **reachable today**, by a fixture that marks at spawn, so the
byte-identity claim is a measurement rather than a hope.

**Why the `mesh_leg` conjunct is load-bearing, not defensive.** On a `VisibilityBuffer × Sdf` frame
`vb_raster` is not declared at all — `graph_bridge.rs:3453-3459` states it (*"a `VisibilityBuffer ×
Sdf` (mesh-less) frame skips BOTH"*), `mesh_leg` is bound at `:3460`, the branch is at `:3499`. A
late scope with no early scope would `LOAD_OP_LOAD` an image nothing wrote this frame. Same conjunct,
same reason, as the HZB build's own (`graph_bridge.rs:3972`).

**Why `path_is_vb()` is kept even though it is redundant inside the VB declarator.** Both
`declare_vb_graph` and `record_vb` are only reached on the VB arm (`graph_bridge.rs:706`, the `3 =>`
dispatch), so inside them the first conjunct is always true. It is kept so the method is *correct at
any call site* — a predicate that is only sound in one caller is a trap for the next reader.

**The predicate over-approximates in the harmless direction.** The count is folded during the
*scatter*, so it counts instances that reached the ring; the runner then further skips batches whose
mesh is not `Loaded` (`runner.rs:1977-1979`). A frame can therefore arm the split and have zero
marked instances in the drawn set. Consequence: an armed empty scope. Never the reverse.

**Trade-off, stated.** Component presence alone changes the recorded pass structure before the
feature does anything. A user who marks objects today pays a second empty scope for no benefit.
Honest and bounded: no default world and no shipped example inserts the marker, so every golden and
every example is untouched *structurally*.

### D4: "Drawing nothing" = a fully recorded late scope, fed by its OWN record array, whose per-batch records all carry `instanceCount = 0`

This is the charter's central mechanical question, and the research settles half of it before the
candidates are even compared: **`vkCmdDrawIndexedIndirectCount` is unavailable and out of scope**
(see Context), so niagara's "one buffer, refill the count to 0" is not on the menu. The remaining
question is *whether the late scope gets its own array or shares `vb_indirect`* — and sharing is
unsound here for a reason that has nothing to do with inertness: the early scope needs
`instanceCount = early_k` and the late scope needs `instanceCount = late_k` **in the same command
buffer**, so a shared array would have to be rewritten between the scopes, i.e. a transfer or compute
write racing the early scope's still-in-flight indirect fetches. That is verbatim the hazard the
research records against niagara's reuse (*"legal only because of `stageBarrier(DRAW_INDIRECT →
TRANSFER)` before `vkCmdFillBuffer`. Drop it and you overwrite records the previous draw is still
fetching"*), and it would have to be paid every frame forever to save 20 KiB × FIF. **Nanite makes
the same call — "separate per pass".**

⇒ **Decision: a dedicated `vb_indirect_late` array**, host-filled with `instanceCount = 0`.

| candidate | byte-identity story | cost | leaves piece 3 a place for the verdict? |
|---|---|---|---|
| **A** — scope recorded, **zero draw commands** | provable | 2 commands | **NO.** A verdict is a number a compute pass writes and a draw reads. With no draw and no record array there is nowhere to write one; piece 3 would add the buffer, the loop and the cull in one step. |
| **B** — scope recorded, `draw_batches` indirect draws against a **dedicated late record array** host-filled with `instanceCount = 0` | provable *and* measurable | 2 + 3·`draw_batches` commands, one 20 KiB×FIF buffer | **YES.** Piece 3 changes one word's *producer* (host `0` → cull-written `k`) and sets the indirection bit. |
| **C** — pass declared, not recorded | violates declare/record parity (`vb.rs:1069-1076` forbids it by name) | — | NO, and it is not a split. |
| **D** — share `vb_indirect`, rewrite between scopes | unsound (above) | a per-frame `DRAW_INDIRECT → TRANSFER` barrier | irrelevant |

**Decision: B.** It is verbatim the discipline this campaign has already run twice and documented as
deliberate: `vb_batch_cull.comp.hlsl:166-173` — *"EACH LEVEL SHIPPED INERT ONE RUNG BEFORE IT WAS
ARMED, on purpose… Neither was a placeholder"* — `visible = true` at R2c0, `keep = true` at R2d-3,
both replaced by real decisions one rung later. `instanceCount = 0` is the third instance of the same
pattern, and it is the *conservative* constant (the previous two were the permissive one) because the
late scope must draw nothing.

**The records are REAL except for one word.** Each late record carries the early record's true
`index_count` / `first_index` / `vertex_offset` / `first_instance: 0`; only `instance_count` is the
inert constant. An all-zero record would be a placeholder, and piece 3 would then be adding structure
rather than flipping a producer.

**The late loop is bounded by the SAME hoisted local as the early loop** — `vb.rs:935-940`'s
`draw_batches` (`mesh_draw.len()` min `record_capacity` min `desc_capacity` min
`vb_cull_batch_count_visible_clamp`) — **not** by `record_capacity_late`. This is the round-1
correction the research forced: the array is a prefix, not a mask, so the late scope records exactly
as many empty draws as the early scope records real ones. `record_capacity_late` survives only as a
`debug_assert!(record_capacity_late >= draw_batches)` and as D5's allocation drift guard.

#### The late array's WRITE is a DECLARED pass. This is the round-1 blocker, and it is the difference between "draws nothing" and "draws whatever was in freshly allocated device memory".

The early array is filled by **`vkCmdUpdateBuffer`** — an inline transfer, `vb.rs:990-998`, from a
stack `[VkDrawIndexedIndirectCommand; 64]` in 64-record chunks — and that write is *declared*:
`graph_bridge.rs:3520` is `g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_TRANSFER_BIT,
VK_ACCESS_TRANSFER_WRITE_BIT);` inside its own `vb_indirect_upload` pass (`:3518-3529`, gated at
`:3517`), and the read is declared on the raster at `:3613-3619`
(`DRAW_INDIRECT` / `INDIRECT_COMMAND_READ`).

Omitting either half is not a wasted barrier, it is a **missing** one. The declarator says so twice,
in two different comments — round 1 (and the critique) quoted a *splice* of them as if it were one
sentence; here they are separately and correctly anchored:
- `graph_bridge.rs:3513-3514`: *"the TRANSFER -> DRAW_INDIRECT dependency the upload actually needs
  would be derived nowhere -- a missing barrier, not a wasted one."*
- `graph_bridge.rs:3553-3554`: *"an access the recorder performs but the declarator omits is a
  barrier derived nowhere, and a buffer hazard is invisible to goldens, to the validation layers and
  to `robustBufferAccess` (off on this device)."*

With no declared writer, `sync.rs:380-383` takes the first-touch arm — `(TOP_OF_PIPE, 0)` — an
execution-only edge that makes the update neither available nor visible. Frame 1, split armed,
freshly allocated DEVICE_LOCAL memory: `instanceCount` is arbitrary, `firstInstance` may be nonzero
with `drawIndirectFirstInstance` VK_FALSE, `robustBufferAccess` OFF — and the scope this whole piece
claims draws nothing, draws. **The compile-time backstop cannot catch it**: `graph.rs:641` / `:955`
is `!is_image || is_write || res_written[ri]`, so a *buffer* read with no producer is waved through,
and both sites sit under `#[cfg(debug_assertions)]` (blocks at `:587-588` / `:951-952`; the backing
`res_written` field is itself `#[cfg(debug_assertions)]` at `:185-186`).

**Therefore:**

| what | value |
|---|---|
| declaring pass | **`vb_indirect_late_upload`**, a new `Option<PassId>` on `VbPassPlan` |
| its predicate | **exactly `path_vb_occlusion_split()`** — the same single source, so declare and record cannot disagree. It is deliberately NOT folded into `vb_indirect_upload`, whose gate is `scene.vb_indirect.is_some()` (`graph_bridge.rs:3517`); reconciling two different gates on one pass is how the W1 single-predicate rule gets broken. |
| where declared | immediately after `vb_indirect_upload` (`graph_bridge.rs:3518-3529`), i.e. **before** `vb_raster` and therefore before `vb_raster_late` |
| its access | `g.buffer_access(vb_indirect_late, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT)` |
| where recorded | immediately after the early fill block (`vb.rs:942-999`), reusing the same stack record array and the same `CHUNK = 64` `cmd_update_buffer` loop, with `instance_count` forced to `0` |
| the late raster's read | `g.buffer_access(vb_indirect_late, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT, VK_ACCESS_INDIRECT_COMMAND_READ_BIT)` on `vb_raster_late`, mirroring `:3613-3619` |
| barriers derived at the late scope boundary, HZB off | **3**, not 2: `vb_id` WAW, `vb_depth` WAW, and `vb_indirect_late` TRANSFER_WRITE → DRAW_INDIRECT |

⚠️ **A count of 3 is NOT sufficient evidence, and G4 is specified accordingly.** The
read-declared/write-undeclared variant *also* yields 3, differing only in `src_stage = TOP_OF_PIPE`,
`src_access = 0`. A gate that asserts the count goes **red on the correct implementation and green on
the defective one**. G4 therefore asserts the barrier's **fields**. See G4.

**Note for piece 3.** The framing "piece 3 changes one word's producer" must not be read as "piece 3
adds no graph edges". Piece 3 replaces the host `vkCmdUpdateBuffer` producer with a *compute* writer,
which changes `vb_indirect_late`'s declared write from `(TRANSFER, TRANSFER_WRITE)` to
`(COMPUTE_SHADER, SHADER_WRITE)` and requires `BufferUsage::STORAGE` plus a `vb_cull_layout` slot.
The usage bit is minted now (below); the access change is piece 3's and is named here so it is
inherited as a requirement rather than discovered as a bug.

**The late push carries the indirection bit CLEAR.** The per-batch push at `vb.rs:1407-1414` is two
words — `base_instance` at `GBUFFER_PUSH_BASE_INSTANCE_OFFSET` (= 80, `scene_types.rs:325`) and a
flags word at `FLAGS_OFFSET` (= 84, `vb.rs:1330`) — and bit 1 of that flags word is what selects the
survivor indirection in `vb_raster.vs.hlsl:194-196`
(`((pc.use_model_matrix & 2u) != 0u) ? visible_instances[base + id] : (base + id)`). With
`instanceCount = 0` no invocation exists, so neither the base nor the bit is read — but "harmless
because a count is zero" is a weaker invariant than "the bit is clear", and a set bit over a region
no pass wrote this frame is verbatim the residue hazard `R2d-REGION-DEFINED`
(`vb_batch_cull.comp.hlsl:118-153`) exists to forbid. Piece 3 sets the bit in the same change that
writes the region.

**The CLEAR-then-LOAD-then-STORE equivalence, *shown* rather than assumed:**

- Scope 1 (`vb_raster`, early): `LOAD_OP_CLEAR` on `vb_id` (`VB_ID_CLEAR = [0xFFFF_FFFF, 0, 0, 0]`,
  `vb.rs:58`; load_op at `:1285`, clear value `:1287`) and on `vb_depth`
  (`VB_DEPTH_CLEAR = 0.0`, `vb.rs:52`; load_op `:1297`, value `:1300`), all `draw_batches` draws,
  `STORE_OP_STORE` on both.
- Scope 2 (`vb_raster_late`): `LOAD_OP_LOAD` on both, **same `renderArea`, same views**, zero
  fragments, `STORE_OP_STORE`.
- `LOAD_OP_LOAD` yields exactly what scope 1 stored; no draw writes; `STORE_OP_STORE` writes back the
  loaded contents. Final contents ≡ scope 1's. The argument needs no numerics, so it is not subject
  to the 8-bit golden floor.
- Two conditions the implementation must not break: both attachments are in the required layouts at
  scope 2's start (scope 1 left `vb_id` in `COLOR_ATTACHMENT_OPTIMAL` and `vb_depth` in
  `DEPTH_ATTACHMENT_OPTIMAL`; the interposed poison+build block touches depth as
  `SHADER_READ_ONLY_OPTIMAL` and the graph derives the round trip — D6), and `renderArea` is
  identical (`full_area`, the same local).
- Not free on a TBDR (a restore/resolve round trip). This engine's targets are desktop IMR
  (`CLAUDE.md`: x86_64 Windows/Linux). Stated, not hidden.

### D5: The survivor list is **not** doubled, **not** widened, and **not** duplicated — the R2d-6 collision, answered structurally

**The collision.** The round-3 blocker: *"a fourth route by which the design disarms rung R2d-6
(doubling the survivor list breaks the very const-assert added in R2d-4 to prevent an out-of-bounds
device read)"*. The assert is `crates/boyko_app/src/gpu_scene/mod.rs:258-264`, and it is an
**equality**, sound in both directions:

- **⊇ (R2d-4)** — `vb_raster.vs.hlsl:194-196` selects between `visible_instances[base + id]` and
  `base + id` with a `? :`, and DXC may lower that to an *eager* load plus `OpSelect`, so the list
  must admit every index the ring does.
- **⊆ (R2d-6)** — `vb_cull_batch_count_visible_clamp` (`vb.rs:194-202`) is the *only* thing bounding
  the cull's `gVbInstances[base_instance + j]` reads, and it clamps against the **survivor list's**
  element count (`visible_elems = size / 4`, `vb.rs:933-934`). A list of `2N` therefore admits a
  batch whose ring rows run past the ring's `N`.

`robustBufferAccess` is OFF (`gpu_scene/mod.rs:256-257`), so neither direction degrades to a zero
read.

**How piece 2 avoids re-creating it — three claims, each structural.**

1. **The late scope gets no survivor list of its own, ever.** Not in piece 2, not in piece 3. The
   design that needed one was solving a problem that does not exist: it wanted a *third bucket* for
   HZB rejects. HZB rejects are **discarded**, not stored. The only two sets that need storage are
   *early-drawn* and *late-candidate*, and round-1 blocker 1 (`VG-R3-HZB-PLAN.md:93-96`) already
   fixed the late candidate set as `(frustum survivors) \ (early rasterized)`. Both are subsets of
   one batch's frustum survivors, they are disjoint, and their union is exactly that survivor set. So
   `|early| + |late| ≤ survivors ≤ instance_count`,
   and the existing region `[base_instance, base_instance + instance_count)` holds both — **two-ended,
   from opposite ends, at the size it already is.** `INSTANCE_CAPACITY` is unchanged,
   `VB_VISIBLE_INSTANCE_ELEMS` is unchanged, the assert is untouched, and
   `vb_cull_batch_count_visible_clamp` keeps bounding both directions with the same number.
2. **Piece 2 writes none of that partition.** It ships only the *record array* that a late draw
   fetches from. The partition is piece 3's, and this plan states the budget so piece 3 inherits a
   proof rather than a temptation.
3. **The one new allocation is a different buffer in a different const family**, and it gets its own
   drift guard modelled on `TEX_INSTANCE_CAPACITY` (`gpu_scene/mod.rs:223-238`, explicitly designed
   so *"a future edit to either literal alone is now a BUILD ERROR"*):
   ```rust
   const VB_INDIRECT_LATE_RECORDS: usize = 1024;
   const _: () = assert!(
       VB_INDIRECT_LATE_RECORDS == INSTANCE_CAPACITY,
       "the late record array's capacity must track the early one: both loops bound themselves by \
        the SAME hoisted `draw_batches`, which is min-ed against EACH array's own derived \
        record_capacity, and a late array shorter than the early one silently drops the tail \
        batches from the late scope."
   );
   ```
   It touches neither side of the R2d-6 equality.

**Usage bits.** `vb_indirect_late` is minted with the **same** flag set as `vb_indirect`
(`gpu_scene/mod.rs:3757-3760`: `BufferUsage::INDIRECT | TRANSFER_DST | STORAGE | TRANSFER_SRC`).
⚠️ Two corrections to round 1 and to the critique: the flag is named **`INDIRECT`**, not
`INDIRECT_BUFFER` (`crates/boyko_rhi/src/enums.rs:37`); and `STORAGE` (`:35`) is **enabling, not
redundant** — piece 3's cull writes `instanceCount` through a descriptor
(`vb_batch_cull.comp.hlsl:253` `RWByteAddressBuffer VbIndirect : register(u0)`, store at `:473-475`),
which is not a transfer. `rhi_impl/device.rs:50-55` already ORs both TRANSFER bits for every
DeviceLocal buffer, so those two are symmetry rather than necessity. Minting STORAGE now is legal and
inert on a buffer nothing binds.

**The review checkpoint this earns.** `gpu_scene/mod.rs:258-270` must appear in piece 2's diff **only
as context**. If the R2d-6 assert or `VB_VISIBLE_INSTANCE_ELEMS` moves by one character, the piece
has re-created the collision. I cannot express that as a test; it is a named, checkable diff
condition.

### D6: The **poison + pyramid-build BLOCK** moves BETWEEN the scopes on an armed-split frame — one predicate, both sites, and the block moves whole

**What.** `declare_vb_graph` and `record_vb` each pick one of two slots for the
**`[hzb_poison, hzb_build_0 .. hzb_build_{n-1}]`** block: today's (after the lit producer) when the
split is unarmed, and immediately after the early `vb_raster` (before the classify chain) when it is
armed.

⚠️ **`hzb_poison` is part of the moving unit, and round 1 omitted it entirely.** It is a separate
declared pass — `graph_bridge.rs:3926-3939` (`g.add_pass("hzb_poison")` at `:3928`, its
`hzb_pyramid` access `TRANSFER` / `TRANSFER_WRITE` / `VK_IMAGE_LAYOUT_GENERAL` over
`hzb_mips(0, levels)` at `:3929-3935`) — declared immediately ahead of the build chain
(header comment `:3941`, code `:3970-4014`, predicate `scene.hzb.filter(|_| mesh_leg)` at `:3972`),
and recorded immediately ahead of the dispatches (poison comment `vb.rs:1956-1973`, gate `:1974`,
`record_vb_pass` `:1985`, block ends `:2019`; build comment `:2021-2039`, gate `:2040`, per-pass
`record_vb_pass` `:2077`, loop `:2067-2154`).

**Round 1's two anchors pointed the implementer away from it, and that is how the pass got dropped:**
`vb.rs:1958` is the **third line of the poison block's own comment**, 119 lines above the build's
first `record_vb_pass` at `:2077`; and the declare anchor `graph_bridge.rs:3941-4014` begins *after*
the poison block. The corrected ranges are **`graph_bridge.rs:3901-4014`** and **`vb.rs:1956-2155`**.

**Why the block must move whole.** `PassId` is strictly monotonic in declare order
(`graph.rs:441-451`) and `compile()` does not reorder, so moving the build alone into the
`:3665-3677` gap would put `build.index() < poison.index()` — the exact negation of the
`debug_assert!` at `graph_bridge.rs:4711-4717`, whose comment at `:4706-4710` predicts the failure in
words: *"the dump would then read `-1.0` everywhere and G8 would red claiming 'the build never ran',
which is a gate reporting the wrong defect."* Moving the block whole preserves all three declare-order
asserts: `poison < build` (`:4711-4717`), `build < dump` (`:4698-4704`, the dump is declared far
later at `:4652+`, so moving the block *earlier* strengthens it), and
`poison.is_some() == dump.is_some()` (`:4722-4726`, unaffected).

⚠️ **The configuration is not hypothetical.** G5 runs a marked scene under `HzbConfig::Build` with
`BOYKO_HZB_DUMP` — armed-split **and** armed-poison in the same frame, by construction. The golden
and gate runs are **dev profile** (`scripts/golden.ps1:180`/`:193` carry no `--release`;
`goldens/PINS.toml:759` relies on it: *"The pin run itself exercises every new declare/record parity
`debug_assert` (dev-profile build)"*), so the assert is live there. In a release binary
(`Cargo.toml` declares only `[profile.bench]`, so `debug-assertions` are off) the assert is compiled
out and the clear would run *after* the dispatches, reddening
`hzb_engine_pyramid_gate.rs:507-517`'s clause 1 at every texel. **Both profiles are named because
they fail differently, and neither failure is silent.**

**Why the move is required, and why it is required *now*.** In the target design the late raster must
write `vb_id`/`vb_depth` *before* `vb_resolve`/`vb_shade` reads `vb_id`, or the late geometry is never
shaded. So the armed order is
`vb_raster (early) → hzb_poison → hzb_build_* → vb_raster_late → classify → lit`.
That reorder must land somewhere. The only moment it is *provably* neutral is while the late scope
draws nothing — which is this piece. Deferring it to piece 3 means shipping a graph reorder in the
same step that arms a decision.

**Why one predicate picking a slot, rather than an unconditional move.** Exact precedent, same file:
the `vb_viewt` PRE-TAIL slot — *"ONE `scene.ssao.is_some()` predicate picks the slot at both declare
and record (the accesses are IDENTICAL in both slots; only the position differs)"*
(`graph_bridge.rs:4023-4025`). An unarmed frame then derives a barrier stream bit-identical to
today's, which is what P2-4's baseline pin measures.

**The barriers this derives between the scopes, and they are new.** Between two consecutive passes
both writing `vb_depth` at `(FRAG, DEPTH_STENCIL_ATTACHMENT_WRITE, DEPTH_ATTACHMENT_OPTIMAL)`,
`transition` fires a WAW — the same auto-chaining the classify block documents
(`graph_bridge.rs:3670-3674`). With the poison+build block armed between them the depth chain becomes
`DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (into `hzb_build_0`, whose `vb_depth` access is
`graph_bridge.rs:3986-3992`) → `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` (into
`vb_raster_late`), both content-preserving (neither is a first touch from `UNDEFINED`; a first touch
here would *discard* the early scope's depth), plus the `vb_id` WAW, plus `vb_indirect_late`'s
TRANSFER → DRAW_INDIRECT.

#### ⚠️ The move RE-SOURCES every downstream `vb_depth` reader. This is a modelling obligation, not a defect.

The declarator states the old assumption in words at `graph_bridge.rs:4652-4654`: *"on every armed
frame that is `SHADER_READ_ONLY_OPTIMAL`, since `hzb_build_0` itself reads it there."* After the
move, on an armed-split frame, the last toucher of `vb_depth` is `vb_raster_late` at
`DEPTH_ATTACHMENT_OPTIMAL` with a pending write, so every later reader's transition changes
*character* — from the "already SHADER_READ_ONLY, execution-only" arm to a real RAW flush
(`FRAG` / `DEPTH_STENCIL_ATTACHMENT_WRITE` → its own stage) plus a layout transition. Affected
readers, all four re-verified:

| reader | anchor | changes on an armed-split frame |
|---|---|---|
| `hzb_dump` | `graph_bridge.rs:4652-4658` | yes — **and this is G5's own path**, so the row is mandatory in G4 |
| `vb_viewt` PRE-TAIL (SSAO on) | `:4028-4034` | yes |
| `vb_viewt` LATE (SSAO off) | `:4537-4543` | yes |
| `sdf_forward_march` mesh arm (VB × Both) | `:4480-4487` | yes |

Every derived barrier is **sound in both slots** — a RAW flush plus a preserving layout transition is
strictly stronger than an execution-only edge. What changes is the *model*, so:
- `graph_bridge.rs:4652-4654`'s comment is **falsified by this piece and must be edited** in P2-5.
- G4 pins the derived `vb_depth` stream per **configuration**, not "with and without HZB".
- The `vb_viewt` precedent D6 leans on for the *slot-picking idiom* does **not** transfer to the
  *re-sourcing* question: both `vb_viewt` slots sit after `vb_raster` with no depth writer between
  them, so its move re-sources nothing. The poison+build block's move crosses a new writer. Stated so
  the precedent is not over-claimed.

**Effect on the pyramid's cross-frame question (§11/§13), asked explicitly by the charter.** Piece 2
changes **when** the pyramid is written, not **who** writes it and not **who** reads it (still
nobody). The `ResSync::undefined()` seed's soundness argument — "a single-buffered image written
every frame is safe only while nothing reads it" — is untouched, and piece 3 still inherits it whole.

⚠️ **One hazard piece 3 inherits from this step.** On an armed-split frame the pyramid is built from
the depth as of the *early* scope, while `BOYKO_HZB_DUMP` copies `vb_depth` at *frame end* (gate
`vb.rs:3227`, the depth `cmd_copy_image_to_buffer` itself at `:3327-3336`; round 1 cited `:3227` for
"the copy", which is the gate). In piece 2 those are equal because the late scope draws nothing, so
G8 still holds — **which also means G8 cannot see the ordering** (see G5's honest limit). The moment
piece 3 arms the late draws they diverge, and G8 would be comparing the pyramid against a depth it
was not built from. **Piece 3 must move the dump's depth copy between the scopes, or dump both
depths.** Recorded here so it is a requirement rather than a discovery.

### D7: The `+INFINITY` fixture vertex — located, and piece 2 does not touch it

The round-3 blocker: *"an `+INFINITY` fixture vertex reaching a second, unfenced host consumer on the
shipped VB path."*

**The chain, anchored end to end. ⚠️ Two of round 1's five rows named functions that do not exist;
both are corrected here, and two missing consumers are added, because piece 3 inherits this list as
its "complete set of raw readers" and an incomplete list is how a fence gets missed.**

| stage | anchor (re-verified) | behaviour |
|---|---|---|
| origin | `crates/boyko_render/src/mesh_assets.rs:193-203` | `local_aabb` seeds `[f32::INFINITY; 3]` / `[f32::NEG_INFINITY; 3]` (`:194-195`) and **returns the seed** for an empty vertex slice (`:202`, the C0 zero-vertex case). `build_mesh_gpu` only `debug_assert!`s non-empty — `:191-192` calls this *"the release-mode backstop"*. |
| carrier | `crates/boyko_render/src/mesh.rs:184`, `:186` (doc `:179-183`) | `MeshGpu::local_min` / `local_max` hold the inverted pair. CPU-only, never uploaded. |
| choke point, GPU side | **`MeshLocalBounds::new`, `mesh_geometry_table.rs:236`** (doc `:213-234`), `UNKNOWN` at `:206-211`, `MESH_BOUNDS_UNKNOWN_COORD = 1e30` at `:136` | maps any non-finite / inverted box to `UNKNOWN` = `±1e30`, deliberately finite because *"a NaN does NOT propagate through `NMin`/`NMax`"*. ⚠️ **`MeshLocalBounds::from_aabb`, which round 1 named, does not exist** — a repo-wide grep returns zero hits. |
| host consumer #1, **fenced** | `csm_caster.rs:297-300` | `batch_world_aabb` returns `None` on `mn[i] > mx[i]` — `+INF > -INF` on every axis. |
| host consumer #2, **fenced** | **`boyko_render/src/hzb.rs:687-692`, short-circuit at `:704-709`** | `project_aabb` returns `Err(KeepReason::UnknownBounds)` on `!(min <= max)`, spelled that way (comment `:701`) so a NaN also counts, at the earliest possible point. ⚠️ **`screen_rect`, which round 1 named, does not exist**; `ScreenRect` is the *type* (`hzb.rs:627`), produced by `project_aabb` and consumed by `select_texels` (`:790`). |
| host consumer #3, **fenced** — MISSING from round 1 | `csm_caster.rs:328-390`, fence at **`:348-350`** (a *different* block from #1's), called at `:439-450` | `reduce_bounds_into` `continue`s on the same `mn[i] > mx[i]` test. Wired in production through the CSM plugin. |
| GPU-side producer #2 — MISSING from round 1 | `gpu_upload.rs:228-237` (the two bounds args at `:235-236`) → `MeshGeometryTable::register` → `MeshLocalBounds::new` at `mesh_geometry_table.rs:680` | the streaming backfill path into the choke point. |
| the shipped VB path's only raw reader | `boyko_app/src/runner.rs:1993-1997` | feeds `(mesh.local_min, mesh.local_max)` straight into `batch_world_aabb`, i.e. through consumer #1's fence. |

**The blocker was that the whole-feature design added a *fourth* raw reader of that pair on the
shipped VB path without the `!(min <= max)` short-circuit** — round-1 blocker 2
(`VG-R3-HZB-PLAN.md:97-100`) names the same defect class and the same required fix: *"the sentinel
must short-circuit to KEEP before any projection, structurally, at the shared entry point"*.

**Piece 2 does not touch it.** Piece 2 adds **no** host consumer of `MeshGpu::local_min`/`local_max`,
and no host consumer of any vertex, AABB or projection. Its only new host-side data flow is a `u32`
presence flag, which cannot be a float. `local_min`/`local_max` do not appear in piece 2's diff.
**The obligation passes intact to piece 3**, which is where a screen rect is first computed, and where
the fence must be the *shared entry point* (`project_aabb`) rather than a per-call-site check.

## Data structures

```rust
// ── crates/boyko_render/src/occlusion_marker.rs (NEW) ────────────────────────────────
/// The structural occlusion-culling capability. Presence = "this entity's instance MAY be
/// rejected by the HZB occlusion test"; ABSENCE = the instance is always drawn. Axis-1
/// (structural capability), verbatim the `ShadowCaster` shape — TABLE storage, no
/// `#[component(storage = ...)]` attribute. Opt-IN: the error direction of a missing marker
/// is a wasted draw, never vanished geometry.
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct OcclusionCulling;

const _: () = assert!(size_of::<OcclusionCulling>() == 0); // presence IS the datum

/// Bit 0 of `VbInstanceRow::flags`. Bits 1..31 are reserved and written zero — the lane is a
/// WORD, not a bool, so piece 3 adds a bit rather than a column.
pub const VB_INST_FLAG_OCCLUSION_CULLING: u32 = 1 << 0;

// ── crates/boyko_render/src/instance_model.rs (EDIT) ─────────────────────────────────
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VbInstanceRow {
    pub affine: [[f32; 4]; 3],   // 0..48  — byte-identical to InstanceModelCol::rows
    pub mesh_id: u32,            // 48     — geometry-table slot
    pub flags: u32,              // 52     — WAS `_pad[0]`. bit 0 = VB_INST_FLAG_OCCLUSION_CULLING.
                                 //          Same 16-byte lane as mesh_id: the cull's existing
                                 //          gVbInstances load already brings it in. Zero fetches.
    pub _pad: [u32; 2],          // 56..64 — still unused, still always zero
}
// The five existing const-asserts (`instance_model.rs:240-249`) are unchanged. ADD two, because
// no assert currently pins `_pad`'s offset at all:
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, flags) == 52);
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, _pad) == 56);

// ── crates/boyko_render/src/mesh_draw.rs (EDIT) ──────────────────────────────────────
pub struct MeshRenderScratch {
    // ... existing lanes ...
    /// Per-instance FLAGS lane, scattered in LOCK-STEP with `ring`/`mesh_ids`
    /// (`inst_flags.len() == ring.len()`, every slot written exactly once). Fused into the
    /// PRIMARY scatter — the `material_ids` shape (clear `:646`, store `:702`, loop
    /// `:671-708`), NOT `gather_material_tex_into`'s second walk.
    pub inst_flags: ScratchColumn<u32>,
    /// Instances whose lane has bit 0 set, folded during the scatter. The frame-level
    /// structural conjunct: `> 0` <=> "the capability is present in THIS frame's ring".
    /// RESET to 0 in `gather_mixed_into`'s opening block beside `mesh_draw.rs:612`.
    occlusion_instances: u32,
}

// ── crates/boyko_app/src/gpu_scene/mod.rs (EDIT) ─────────────────────────────────────
const VB_INDIRECT_LATE_RECORDS: usize = 1024;
const _: () = assert!(VB_INDIRECT_LATE_RECORDS == INSTANCE_CAPACITY, "…");   // D5 §3
pub(crate) const VB_INDIRECT_LATE_BYTES: u64 =
    (VB_INDIRECT_LATE_RECORDS as u64) * u64::from(DRAW_INDEXED_INDIRECT_STRIDE);   // 20 KiB
// per-FIF, DEVICE_LOCAL, usage = the SAME set as `vb_indirect` (`:3757-3760`):
//   BufferUsage::INDIRECT | TRANSFER_DST | STORAGE | TRANSFER_SRC
// Minted UNCONDITIONALLY (the `vb_visible_instance` rule, scene_types.rs:2809-2812).
// Destroyed beside `vb_indirect` (`:7042-7044`).

// ── crates/boyko_rhi_vulkan/src/present/scene_types.rs (EDIT) ────────────────────────
pub struct GBufferScene<'a> {                                   // defined at :1607
    // ...
    /// The per-FIF LATE indirect record array. `Some` on every VB boot (minted
    /// unconditionally); `None` only in the four hand-written test literals, exactly as
    /// `vb_indirect` is. It is `.expect()`ed under `path_vb_occlusion_split()` and is NOT a
    /// conjunct of that predicate — a dead conjunct is what `scene_types.rs:2809-2812` exists
    /// to avoid.
    pub vb_indirect_late: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Instances in this frame's ring carrying `OcclusionCulling`. A plain `u32` because this
    /// crate cannot depend on `boyko_render` (the `vb_classify_material_count` boundary,
    /// `scene_types.rs:2909-2925`).
    pub vb_occlusion_instances: u32,
}

// ── crates/boyko_rhi_vulkan/src/present/graph_bridge.rs (EDIT) ───────────────────────
#[cfg(feature = "hwrt")]      const VB_BUFFER_COUNT: usize = 15;   // was a bare literal 14
#[cfg(not(feature = "hwrt"))] const VB_BUFFER_COUNT: usize = 14;   // was a bare literal 13

pub struct VbBarrierSink<'a> {
    pub(crate) images: [VkImage; VB_IMAGE_COUNT],       // UNCHANGED (:2884; 15 / 21 at :2924-2928)
    pub(crate) buffers: [VkBuffer; VB_BUFFER_COUNT],    // :2906 / :2910, literals -> the const
    // `vb_indirect_late` is appended LAST, after `vb_visible_instance`, in BOTH cfg arms —
    // the P1-5 rule, so every existing ResId is byte-unchanged and
    // `buffers[b.res.index() - VB_IMAGE_COUNT]` (`:3041`) keeps indexing what it indexed.
    // NOTE: `vb_indirect_late` is `.expect()`ed in the sink array, NOT placeholder-backed
    // (`:4872-4892` is the placeholder family) — it is mandatory on every VB boot, and a
    // placeholder would silently name a live wrong buffer with no VUID.
}

pub struct VbPassPlan {                                  // :2630 — NOT `VbFramePlan` (round-1 typo)
    pub(crate) vb_raster: Option<PassId>,                // :2674 — the EARLY scope. Ident kept; see OQ4.
    pub(crate) vb_raster_late: Option<PassId>,           // NEW — Some iff path_vb_occlusion_split()
    pub(crate) vb_indirect_upload: Option<PassId>,       // :2680 — unchanged
    pub(crate) vb_indirect_late_upload: Option<PassId>,  // NEW — Some iff path_vb_occlusion_split()
    pub(crate) hzb_poison: Option<PassId>,               // existing; SLOT differs (D6)
    pub(crate) hzb_build: [Option<PassId>; MAX_HZB_PASSES], // :2708 — unchanged; SLOT differs (D6)
}

// ── the G2 probe (NEW, crates/boyko_rhi_vulkan/src/present/passes/vb.rs) ─────────────
/// Recorder-authored counts. ⚠️ There is NO existing scalar writeback route out of `record_vb`
/// — the critique's F-14 named `vb.rs:3134-3205`, which is a `vkCmdCopyImageToBuffer` of
/// `vb_id`, not a counter. This struct is new, and its numbers ORIGINATE IN THE RECORDER.
#[derive(Default, Clone, Copy, Debug)]
pub struct VbRecordProbe {
    pub scopes: u32,          // raster scopes actually recorded this frame: 1 or 2
    pub late_draws: u32,      // vkCmdDrawIndexedIndirect calls issued in the late scope
    pub late_instances: u32,  // sum of instanceCount over the late records written
}
// Threaded as `Option<&mut VbRecordProbe>`, a `&mut` PARAMETER on `record_vb` (whose receiver
// is `&self`, `vb.rs:349-365`) and on `render_gbuffer_frame` (`frame_driver.rs:806`, params to
// `:820`; forwarded at `:918`). No buffer, no barrier, no fence — the counts are known on the
// host at record time. `None` on every non-gate frame: zero cost.
```

## Public API

```rust
// boyko_render
pub struct OcclusionCulling;                            // the marker (table ZST)
pub const VB_INST_FLAG_OCCLUSION_CULLING: u32 = 1 << 0; // bit 0 of VbInstanceRow::flags

impl MeshRenderScratch {
    /// Instances in THIS frame's ring carrying the capability. The structural conjunct of the
    /// split's arming predicate. Read off the MAIN scratch only — `CsmCasterScratch.0` also
    /// carries one and it is redundant, never authoritative (D2).
    pub fn occlusion_instances(&self) -> u32;
}

// boyko_rhi_vulkan — `pub`, not `pub(crate)`: `GBufferScene` is `pub` and a `pub(crate)` method
// on it would be dead_code under `-D warnings` for the one commit in which it has no reader.
impl GBufferScene<'_> {
    /// THE single source of "this frame records TWO raster scopes", read by `declare_vb_graph`
    /// AND `record_vb` (declare/record parity). Derived, never stored.
    pub fn path_vb_occlusion_split(&self) -> bool;
}

pub struct VbRecordProbe { /* pub fields, see above */ }
```

No `dyn`, no allocation in the frame loop, no new pipeline, no new descriptor-set layout, no new
descriptor set, no shader **code** edit (three HLSL *comments* are corrected — see Integration).

## Algorithms for critical paths

**The gather fold** — once per frame, inside the existing primary scatter (`mesh_draw.rs:671-708`).

- Steps: reset `occlusion_instances = 0` and `inst_flags.build_view().clear()` in the opening block
  (beside `:612` and `:646`); per query row compute `let occ = marker.is_some();`, push
  `u32::from(occ) * VB_INST_FLAG_OCCLUSION_CULLING` into `inst_flags`, and fold
  `occlusion_instances += u32::from(occ)`.
- Complexity: O(instances), **fused** — no second walk, in deliberate contrast to
  `gather_material_tex_into`, which the tree itself flags as an extra O(N) pass
  (`mesh_draw.rs:1179-1185`).
- Cache: one extra sequential `u32` lane beside `mesh_ids`; the marker probe is a per-*archetype*
  `bool` resolved once (`option.rs:96-101`) plus a per-row `Some(&T)` at a dangling base
  (`component_pool.rs:3454-3457`) — no column touch, because the column is zero-width.
- Branching: `Option<&ZST>` lowers to a per-archetype constant; `+= bool as u32` is branchless.
- SIMD: the lane is a dense `u32` column and the count is a trivially vectorisable reduction. It has
  the same shape as `mesh_ids`, so a future SIMD scatter for one covers the other.

**`sync_vb_instance_ring`** (`mesh_draw.rs:483-498` — the real name; round 1 called it
`build_vb_ring`, which does not exist; the public system wrapper is `sync_vb_instance_ring_system`
at `:1047-1053`): the existing zip over `ring`/`mesh_ids` (`:493`) gains a third lane;
`flags = inst_flags[i]`. Still one sequential pass, one store per row, 64 B per instance.

**The late indirect fill** — once per frame, only when armed, recorded immediately after the early
fill (`vb.rs:942-999`). Same stack `[VkDrawIndexedIndirectCommand; 64]`, same `CHUNK = 64`
`cmd_update_buffer` loop (`:990-998`), same per-batch record construction, with `instance_count`
forced to `0` and `first_instance` staying `0`. ⌈`draw_batches`/64⌉ commands, ≤ 1280 bytes inline
each (well inside the 65536 `vkCmdUpdateBuffer` limit).

**The late scope recording** — once per frame, only when armed: `begin_rendering` (LOAD/LOAD/STORE)
→ bind pipeline / set / viewport / scissor → pass-wide push → per batch { 2-word push
(`base_instance`, flags **with bit 1 clear**), bind VB, bind IB,
`cmd_draw_indexed_indirect(late[fi], i * DRAW_INDEXED_INDIRECT_STRIDE, 1, stride)` } →
`end_rendering`. O(`draw_batches`). The rebinds are explicit rather than relying on state surviving
`vkCmdEndRendering` and interposed compute dispatches — 4 commands to remove a subtle dependence.

**GPU cost of a zero-`instanceCount` indirect draw.** `instanceCount == 0` produces zero vertex
invocations by spec (`Vulkan-Docs/chapters/drawing.adoc`: `drawCount` "can: be zero"; no valid-usage
statement forbids zero `instanceCount`); the command processor still fetches the 20-byte record and
sets up the draw. `draw_count = 1` is forced — `multiDrawIndirect` is off (`vb.rs:1422-1424`).
⚠️ **This is the one quantitative claim in the plan that is unmeasured, and the research confirms
nobody has published it**: the nearest figures are about per-*entry* empty draws inside an MDI array
(vkguide, pcwalton/Bevy #17211), and the only Dawn number (~3 ms → ~10 µs) is a WebGPU validation-shim
artifact that does not transfer to native Vulkan. The mitigation is already in the design and costs
nothing structurally: **the late loop is bounded by `draw_batches`, not by the 1024 cap**, so the
worst case is "one empty indirect draw per batch the frame already draws". Ubisoft's 2015 table
records phase-2 draw at **< 0.01 ms** on Xbox One at 1080p — the regime where "recorded with a
near-zero count" and "not recorded" are hard to tell apart, which is exactly where piece 2 sits by
construction. See OQ2 for what would settle it and why a small delta is not defensible here.

## Multithreading model

Single-threaded with respect to everything piece 2 adds.

- **ECS.** The gather runs as one system on the scheduler. Under **table** storage,
  `Option<&OcclusionCulling>` **does** declare a real read of the component id, widening
  `gather_mesh_draws`' access set by one shared read — and the query tuple *is* the access contract
  the scheduler derives from (`mesh_draw.rs:1210-1214` / `:1112-1115` say so in both variants). A
  shared read conflicts with nothing else in the frame, since no system in the tree writes
  `OcclusionCulling`. ⚠️ Round 1 asserted this same sentence under *bitset* storage, where it was
  false in the reassuring direction: `IsEnabled::init_access` is a documented no-op
  (`data_is_enabled.rs:170-176`), so a bitset read declares **zero** access and the toggling system
  and the gather would be invisible to conflict detection (sound only because toggles route through
  the command flush). The D1 reversal is what makes the claim true.
- **Command recording.** Single-threaded into one `VkCommandBuffer` (`record_vb(&self, …)`,
  `vb.rs:349-365`). Because the receiver is `&self`, every buffer write in this function is a
  *command* (`cmd_update_buffer` / `cmd_fill_buffer`), never a host mutation — the late fill obeys
  that without exception. The G2 probe is the one host mutation and it is an explicit `&mut`
  parameter, not interior mutability.
- **Device.** The late scope writes nothing, so there is no GPU race to reason about. The barriers
  D4 and D6 derive exist to be *correct in piece 3*; G4 pins them now, while the claim is cheap.
- No atomics, no shared state, no `Send`/`Sync` change, no new `Resource`.

## Integration

**Modules touched — 15 files, 1 new.** Round 1 said "(7 files, 2 new)" over an 8-row table with one
NEW row; both numbers were wrong and five production/test files were missing, three of which break
the build the moment `gather_mixed_into`'s item tuple widens.

| file | change | step |
|---|---|---|
| `crates/boyko_render/src/occlusion_marker.rs` | **NEW** — the marker + const-asserts + `VB_INST_FLAG_OCCLUSION_CULLING` | P2-1 |
| `crates/boyko_render/src/lib.rs` | `pub mod occlusion_marker;` + re-export | P2-1 |
| `crates/boyko_render/src/instance_model.rs` | `_pad[0]` → `flags`; two new offset const-asserts; `from_model_col` (`:258`) gains the flags arg; the `_pad == [0,0,0]` pin (`:267-276`, assert `:275`) updated to `_pad == [0,0]` + a flags assert | P2-2 |
| `crates/boyko_render/src/mesh_draw.rs` | the `inst_flags` lane + its clear; the fold + its reset; `gather_mixed_into`'s item tuple (`:605-607`) widens to 5; `sync_vb_instance_ring` (`:483-498`); the query term in **both** `gather_mesh_draws` variants (`:1117` non-hwrt, `:1218` hwrt) | P2-2 |
| `crates/boyko_render/src/csm_caster.rs` | **PRODUCTION, and it breaks without this edit** — the closure at `:199` closes `\|(h, col)\| (h.0, col, None, PerInstanceMaterial::default())` into the shared core (`:14` names the reuse as a deliberate contract). The caster query at `:173` gains `Option<&OcclusionCulling>` and the closure supplies the real value (D2). Test call sites `:562` and `:674` follow. | P2-2 |
| `crates/boyko_app/tests/zero_alloc.rs` | the Principle-5 gate `frame_helpers_allocate_zero_after_warmup` (`#[test]` `:100`, fn `:101`) calls `gather_mixed_into` twice (`:157` warmup, `:178` measured) — ⚠️ round 1's anchor `:139` had **MOVED**. Both closures widen. **This gate must stay green: the lane is a `ScratchColumn` reused across frames, so the steady state allocates zero.** | P2-2 |
| `crates/boyko_app/tests/structural_zero_substep.rs` | `gather_mixed_into` call at `:56-61` (round 1 said `:59`) widens | P2-2 |
| `crates/boyko_rhi_vulkan/shaders/vb_batch_cull.comp.hlsl` | **COMMENT ONLY** (`:280-283`, the `_pad` @52 mirror claim) | P2-2 |
| `crates/boyko_rhi_vulkan/shaders/vb_raster.vs.hlsl` | **COMMENT ONLY** (`:140-143`) | P2-2 |
| `crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli` | **COMMENT ONLY** above the `uint3 _pad;` declaration (`:44-50`) | P2-2 |
| `crates/boyko_app/src/gpu_scene/mod.rs` | `vb_indirect_late` allocation (`:3744-3764`'s shape) + the two consts + the drift assert + `scene()` wiring (`:5472-5643`) + destroy (`:7042-7044`'s block) | P2-3 |
| `crates/boyko_app/src/runner.rs` | threads `scratch.occlusion_instances()` into the sole `.scene(` call (`:2364`), beside `any_non_default_material` / `any_textured_material` (`:2386`/`:2390`); owns the G2 probe | P2-3 / P2-6 |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | two `GBufferScene` fields (struct at `:1607`) + `path_vb_occlusion_split()` | P2-3 |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | ⚠️ **`GBufferScene` has NO `Default` impl anywhere in `crates/`** and all four literals are exhaustive with no `..` rest — `:2265`, `:3366`, `:8390`, `:9904`. All four must gain both fields. ⚠️ this file is in **`tests/`**, not `src/present/`. | P2-3 |
| `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | `VB_BUFFER_COUNT` + the sink assert; the sink array (both cfg arms, `:2905-2910`, `:4854-4950`); `vb_indirect_late_upload` + `vb_raster_late` declarations; the poison+build slot pick; `VbPassPlan` (`:2630`, construction `:4730-4771`); **the falsified comment at `:4652-4654`**; the pass-chain doc at `:3084-3085` | P2-3 / P2-5 |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | the late fill; the late scope; the poison+build record-slot pick; the `VbRecordProbe` parameter; the pass-chain doc at `:317-320` | P2-5 / P2-6 |
| `crates/boyko_app/tests/vg_density_census.rs` | `VB_PINS: [&str; 14]` (`:55-79`) → 15, in the SAME commit as the new pin — see M5 note below | P2-6 |

**⚠️ The two `gather_mesh_draws` variants are a trap.** They are separate `#[cfg]` functions and the
doc explicitly requires *"the ring / mesh-id / material-id / pair lanes are byte-identical to the
non-hwrt gather (the OFF path never diverges)"* (`mesh_draw.rs:1205-1207`). Both must gain the term
identically, and `cargo check --workspace --features hwrt` is not optional for P2-2.

**⚠️ `VbBarrierSink::buffers` grows in both `cfg` arms** (13 → 14 non-hwrt, 14 → 15 hwrt) and
`vb_indirect_late` must be appended **LAST**, after `vb_visible_instance`, or
`buffers[b.res.index() - VB_IMAGE_COUNT]` (`:3041`) re-indexes every existing buffer. Appending a
*buffer* cannot disturb the "pyramid last" invariant, which is asserted over **images** only
(`:3272-3276`). The buffer side is currently **less** guarded on both axes: there is no
`VB_BUFFER_COUNT` anywhere in the repo (the lengths are hand-written literals), and every buffer sink
slot resolves to a valid handle — the placeholder family at `:4872-4892` maps `None` to
`scene.light_table.buffer` — so a mis-indexed *buffer* barrier names a **live wrong buffer with no
VUID**, where the image array would hold `VkImage::NULL`. Hence the new const and its
`debug_assert_eq!`, mirroring the image side.

**⚠️ The new pin reds a test the plan must name.** `vg_density_census.rs:187` —
`the_a_domain_is_exactly_the_vb_pins_that_were_measured` — parses `goldens/PINS.toml` (`:188-199`),
filters `s.starts_with("vb")` (`:196`) and asserts set-equality against `VB_PINS` (`:202-208`). It is
**not** `#[ignore]`d, so it runs under a plain `cargo test -p boyko-app`. Adding `vb_occ_split` makes
`found` 15 against a 14-element list and the step reds. `VB_PINS` is bumped in the same commit. That
file's own `:57-64` records why this list exists and that it already caught exactly this omission
once in this campaign — ⚠️ the critique cited that note as `PINS.toml:57-64`; it is
`vg_density_census.rs:57-64` (PINS.toml:57-64 is the `[grand_showcase_2mat]` block). **Read
`vg_density_census.rs:57-64` before authoring the bump**: if `vg_density_census_gate` (`:330`,
`#[ignore]`d, live-GPU) also requires a measured density row for the new name, that measurement is
part of P2-6 rather than an afterthought.

**On the three HLSL comment edits.** `vb_batch_cull.comp.hlsl:280-283` documents *"a 12-byte `_pad`
@52 … so the two mirrors of one host type cannot drift"*; `vb_raster.vs.hlsl:140-143` says the same;
`vb_geom_fetch.hlsli:44-50` declares `uint3 _pad;` and is `#include`d by four modules
(`vb_geo.comp.hlsl:118`, `vb_resolve.comp.hlsl:85`, `vb_shade.comp.hlsl:90`,
`vb_shade_split.comp.hlsl:137`). All three go stale the moment P2-2 lands, and the victim is piece
3's author, who must read `row.flags` in the file that tells him those bytes are unused.
**Decision: the COMMENTS are corrected in P2-2; the HLSL field is NOT renamed until piece 3.** A
`uint3 _pad` at offset 52 has the correct *layout* either way, so the rename buys nothing here and
would be a real edit to an `.hlsli` included by four shaders, each of which must then be re-DXC'd and
re-pinned — against a charter that says every `.spv` is byte-unchanged. The comment edit is expected
to be `.spv`-neutral (the frozen `dxc` recipes carry no `-Zi`/`-Qembed_debug`), and **the
`*_spv_sync` re-DXC tests are the gate for that expectation, not an assumption**: if any of them
reds on a comment-only edit, the edit is reverted and the finding — that the frozen recipes are not
comment-neutral — is recorded in this document, which is itself worth more than the comment.

**No change** to: `vb_batch_cull.comp.hlsl`'s code, `vb_raster.{vs,fs}.hlsl`'s code, any `.spv`,
`vb_cull_layout`, `vb_layout0`, `HzbTargets`, `HzbConfig`, `GBufferTargets`, `boyko_render::hzb`,
`gpu_scene/mod.rs:258-270`, `device.rs`'s fn table or feature chain.

## Implementation plan

Each step builds green and commits alone. Round 1 claimed this of P2-2 and P2-3 and it was false only
because its file list was short by five; with the list in Integration corrected, both land green.

- **P2-0 — the sync-validation LIVENESS probe. No code change; one measured run; it decides what two
  later gates mean.** Round 1 asserted *"`SYNCHRONIZATION_VALIDATION` does not appear anywhere in
  this repository"* and that is **false**: `crates/boyko_rhi_vulkan/src/device.rs:2152` is
  `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`, packed into `VkValidationFeaturesExt`
  at `:2153-2160` and chained as the instance `p_next` head at `:2187-2193`; `ffi.rs:1665`/`:1670`
  define both; `scripts/golden.ps1:167` sets `BOYKO_ENABLE_VALIDATION=1` for every `-ValidationOn`
  pin and `runner.rs:213` reads it; `crates/boyko_render/tests/sync_validation.rs:47-54` calls it
  *"the AUTHORITATIVE oracle"*. **But it degrades SILENTLY** when `VK_EXT_validation_features` is
  absent — `device.rs:2107-2111`, *"Its absence downgrades to plain validation rather than crashing
  on an unrecognized chained struct"*, arm at `:2119-2122` — and the published 19-message baseline
  (`OPEN-QUESTIONS.md:144-151`) is entirely `vkCreate*`-time entries, so **nothing in the tree
  establishes the feature was ever live on this device.**
  *The probe:* delete `hzb_build_0`'s `vb_depth` `image_access` (`graph_bridge.rs:3986-3992`), run
  `scripts\golden.ps1 -Pin vb_mesh_hzb -ValidationOn`, record whether a `SYNC-HAZARD-*` message
  appears, then revert.
  - ✅ **a SYNC-HAZARD appears** ⇒ sync-val is live; **G3 is the leg that can see a missing barrier**,
    and G4 is corroboration.
  - ❌ **no SYNC-HAZARD** ⇒ the extension is absent on this device; G3 sees only *static* legality
    (usage bits, layouts, the second `begin/endRendering` bracket) and **G4 plus the declarator's own
    `debug_assert`s are the only barrier evidence**. The commit message must then say "the extension
    is absent on this device", not "sync-val does not exist in the repo".
  - **Can it fail?** It has two outcomes and both are informative; the *failure* mode is running it
    and not recording the answer.

- **P2-1 — the marker, read by nothing.** `occlusion_marker.rs` + export + const-asserts + the unit
  tests. Zero call sites. (The `hzb_config.rs` P1-1 shape: the knob before the machinery.)

- **P2-2 — the gather lane and the per-instance flag, read by nothing on the device.** The
  `inst_flags` column + its clear, the fused scatter, the `occlusion_instances` fold **and its
  reset**, `VbInstanceRow::flags`, `sync_vb_instance_ring`, both `gather_mesh_draws` variants, the
  widened `gather_mixed_into` tuple **and all five call sites it breaks** (`csm_caster.rs:199`
  production, `:562`, `:674`, `zero_alloc.rs:157`/`:178`, `structural_zero_substep.rs:56-61`), the
  caster query term, and the three HLSL comment corrections.
  Gates: lock-step, the flag word, `_pad == [0,0]`, the two-gather reset regression, 25/25 goldens
  (**blind — see below**), `--features hwrt` check, `zero_alloc` green, `*_spv_sync` green.
  ⚠️ Round 1 said "the uploaded ring bytes change" — **false**: nothing in the tree marks anything at
  P2-2, so `flags == 0 == the old _pad[0]` on every existing scene and the ring bytes are identical.

- **P2-3 — the frame predicate, the late buffer and the sink slot, read by nothing.**
  `vb_occlusion_instances` threaded from the runner; `path_vb_occlusion_split()` (`pub`, so no
  `dead_code` in the commit where it has no reader); `vb_indirect_late` allocated + wired + destroyed
  + drift-asserted; `VB_BUFFER_COUNT` introduced **as the array length** (so it is used, not dead) +
  the sink assert; the slot appended last in both cfg arms; the four `window_present_gbuffer.rs`
  literals. Zero readers of the predicate; zero writers of the buffer; zero recorded commands change.

- **P2-4 — the BASELINE barrier-stream pin, on the UNMODIFIED declarator, over the FULL matrix.** The
  P1-5a C1 discipline (`VG-R3-P1-PYRAMID-PLAN.md:517-520`): *"Authoring them after the change would
  certify the new behaviour."* Must be green before P2-5 exists. The matrix is G4's eight rows minus
  the four split rows — i.e. the four unsplit rows, each authored now.

- **P2-5 — the split, ATOMICALLY.** `declare_vb_graph`'s `vb_indirect_late_upload` + `vb_raster_late`
  + the poison+build slot pick + the new declare asserts; `record_vb`'s late fill + late scope + its
  own slot pick; the falsified comment at `graph_bridge.rs:4652-4654`; the pass-chain docs at
  `graph_bridge.rs:3084-3085` and `vb.rs:317-320`. Declaration and recording **cannot** be split
  without a red intermediate (declare/record parity, `vb.rs:1069-1076` forbids the declared-but-
  unrecorded state by name) — the P1-5a C2 lesson. Neither can the poison and the build move
  separately (D6).

- **P2-6 — the gates.** G1..G5, the fixtures, the `VB_PINS` bump, the cross-pin hash guard, the
  `VbRecordProbe` route.

- **P2-7 — the corruptions, EXECUTED, and the record of what the pins cannot claim.** Every control
  below run, results tabulated in this document the way P1-3 §8 tabulates its two — **including the
  controls that do NOT fire**, since reporting only the ones that fire is how a vacuous gate ships.

## Metrics and validation — THE GATES

*"Can this gate fail?"* is asked first, per the charter. This campaign has now shipped five vacuous
gates; every gate below carries an executed corruption and an explicit statement of what it cannot
claim.

### ⚠️ What a golden pin CAN and CANNOT claim about this change

**It CAN claim** that the pixels a given scene produces are unchanged.

**It CANNOT claim:**

1. **That the changed code ran.** §13 is the worked example: `hzb_engine_pyramid_gate` reported *0
   mismatches over all 349 525 texels* — and the measurement beside it showed **89.3 % of the pyramid
   was `0.0`, levels 6..9 entirely so**, i.e. *"a pyramid image that a driver zero-filled and NOBODY
   WROTE would match the oracle at every one of those texels"*
   (`VG-R3-P1-PYRAMID-PLAN.md:607-651`). The green was nearly meaningless until §14 replaced the
   coverage argument with a `-1.0` poison (`HZB_PYRAMID_POISON`, `scene_types.rs:1454`).
2. **Anything at all about the split, on the 25 pins as they stand.** No existing pin, example or
   default world marks anything, so `path_vb_occlusion_split()` is **false on every one of them** and
   the late scope is never recorded. A green 25/25 on this piece is evidence about the *untouched*
   path — that P2-2's lane and P2-3's new field and buffer perturb nothing. **It is not evidence
   about the split.** This must be said in the commit message, or the next reader will read 25/25 as
   "the split is proven inert". Same class of blindness as §13's, arriving from the other direction:
   there the scene could not reach the property, here the scene cannot reach the code.
3. **Sub-ULP agreement.** The 8-bit floor is blind between roughly 2⁻²⁰ and 2⁻¹⁶ relative
   (`reference-golden-fp-resolution`). Irrelevant here — D4's equivalence is exact and non-numeric —
   but stated so it is not silently assumed away.

### G1 — the split pin equals the unsplit pin *(the load-bearing image gate)*

A new pin `vb_occ_split`: the `[vb_mesh]` scene, verbatim, with `OcclusionCulling` **in the spawn
bundle of all five spheres**. Its `sha256_software` **and** `sha256_hwrt` must be the same literals as
`[vb_mesh]`'s (`goldens/PINS.toml:300`/`:301` — ⚠️ round 1's anchor `:281-284` had **MOVED**; the
`[vb_mesh]` block is now `:271-307`, `test_binary` `:295`, `test_name` `:296`). It arms through the
`[vb_mesh_hzb]` route, which needs **no new test binary**: that pin (`:309-341`) reuses
`test_binary = "vb_mesh"` (`:327`) / `test_name = "vb_mesh_screenshot_dump"` (`:328`) and arms via
`[vb_mesh_hzb.env]` (`:335`) with `BOYKO_VG_HZB = "1"` (`:339`), read **inside the fixture** at
`crates/boyko_app/tests/vb_mesh.rs:198-200`. A `BOYKO_VG_OCC = "1"` branch takes the identical route.

**Why all five and not a strict subset.** A subset would split the mesh family into two archetypes,
and the gather walks archetypes in order — so the *ring order* would change, and with it the order
two instances writing the same pixel at exactly equal depth resolve. That risk is small and it is
avoidable: marking all five keeps one archetype and therefore the exact ring order, so the pin's
byte-identity is evidence about the **recording path**, which is what it exists to prove. The
mixed-archetype case is exercised by G2's `vb_occ_multi` fixture, where the gate is a count rather
than a hash and an order change cannot produce a false red.

⚠️ **"Cannot be blessed wrong" — round 1's claim — is FALSE, and the harness prints the destroying
advice.** `scripts/golden.ps1:245-259` overwrites `sha256_*` with whatever the run produced (the
write is `Set-Content` at `:259`, inside `if ($Bless)` at `:240`), and `:283` prints *"re-run with
-Bless"* on the very mismatch the pin exists to report. Nothing parses `PINS.toml` to check that two
pins agree. The tree states this failure for the identical construction one pin above —
`PINS.toml:315-318`, *"a lone re-bless here would silently convert the gate from 'inert' to
'whatever it does now'"*. What IS true (and closes OQ5) is that the harness cannot re-bless
*unhelpfully*: the sole `PINS.toml` write is under `-Bless`, a missing key throws only there
(`:258`), and on the check path an absent or `PENDING` hash exits 2 (`:266`/`:270`).
**⇒ G1 ships with a guard**, a plain `#[test]` that parses `goldens/PINS.toml` and asserts
`vb_occ_split.sha256_software == vb_mesh.sha256_software`, the same for `sha256_hwrt`, and the same
for the existing `vb_mesh_hzb`/`vb_mesh` pair. The machinery already exists —
`crates/boyko_app/tests/vg_density_census.rs:188-199` reads and parses that file — so this is a ~10
line extension, and it is what makes the equality survive a `-Bless`.

**Controls — three, all executed, all reported:**

| # | corruption | expected | why |
|---|---|---|---|
| C1 | force the late scope's `load_op` to `VK_ATTACHMENT_LOAD_OP_CLEAR` | **RED** (`vb_mesh` stays green) | the frame then presents only what the late scope drew — nothing. This is the direct proof of D4's CLEAR-then-LOAD equivalence, in the direction that matters. |
| C2 | set one late record's `instanceCount = 1`, nothing else | **GREEN — and that is the point** | under `GREATER` reverse-Z, re-drawing an instance the early scope already drew, at identical transform and identical depth, writes nothing. **A gate that cannot see a duplicated late draw is a fact worth publishing.** |
| C3 | `instanceCount = 1` **and** the late scope's pass-wide view push translated by a visible amount | **RED** | the late-drawn geometry lands where the early scope left `VB_ID_CLEAR` / depth `0.0`; any fragment with depth > 0 passes. This is the control that proves a late DRAW reaches the framebuffer. |

⚠️ **The critique's proposed control — `instanceCount = 1` with `base_instance` pushed `+1` — is
REFUTED as vacuous, and no fixture rescues it.** The critique argued it fails only because
`[vb_mesh]`'s five spheres share one batch, and prescribed a multi-mesh, subset-marked fixture as the
repair. That repair does not work, for a structural reason: **in piece 2 the EARLY scope draws every
batch's every instance** — marking affects only the *late* candidate set, and the late scope draws
nothing regardless. So any instance the late scope could name is already on screen at exactly that
depth, and `GREATER` rejects the redraw. `+1` is vacuous **by construction, on every possible piece-2
fixture**, not by fixture accident. C3 replaces it with a perturbation the early scope did *not*
already write, which is the only class that can fire.

**What G1 cannot claim:** that the split happened at all. It is satisfied by not splitting. That is
G2's job, and the pair (G1 green + G2 red under the same corruption) is the evidence, not either
alone.

### G2 — the split RAN *(the gate G1 structurally cannot be)*

`record_vb` reports `VbRecordProbe { scopes, late_draws, late_instances }` through a `&mut`
parameter, and the gate asserts `scopes == 2 && late_draws == draw_batches && late_instances == 0` on
a marked scene, `scopes == 1` on an unmarked one.

⚠️ **The route does NOT exist today and must be built; the critique's F-14 is wrong on this point.**
F-14 stated *"recorder writes it (`vb.rs:3134-3195`)"* — that block is a
`vkCmdCopyImageToBuffer` of the `vb_id` image into host-visible staging (`:3134-3205`), and all
decoding happens host-side in `crates/boyko_app/src/vg_census_dump.rs`. **There is no scalar
writeback out of `record_vb` at all.** The only recorder-*authored* datum in the whole function is
the HZB dump header (`cmd_update_buffer`, `vb.rs:3294-3302`). F-14's *conclusion* survives — a `&mut`
parameter works despite the `&self` receiver — but the plan must budget for building the route, not
for reusing one.

Chosen shape: a `&mut` parameter, not a device buffer. The counts are known on the host at record
time, so a buffer would add an allocation, a barrier, a fence wait and a decode for a number that is
already in a register. `Option<&mut VbRecordProbe>` costs nothing when `None`.

⚠️ **The number must ORIGINATE IN THE RECORDER.** A host that re-derives `scopes` from
`vb_occlusion_instances` agrees with itself no matter what the recorder did — §14's own objection to
a dump header that re-derives its extent. That is the difference between a gate and a tautology.

**Two fixtures, because one of them cannot falsify the per-batch clause:**

- `vb_occ_split` (G1's, 1 `DrawBatch` — its five spheres share one `MeshHandle`,
  `vb_mesh.rs:14-22`): `late_draws == 1`. ⚠️ **This clause is satisfiable by a hard-coded draw or a
  `take(1)`**, so on this fixture alone the per-batch loop, the `i * DRAW_INDEXED_INDIRECT_STRIDE`
  offset (`vb.rs:1432`) and the `draw_batches` bound are all unfalsifiable.
- **`vb_occ_multi` — a NEW fixture with ≥ 2 registered meshes and a STRICT SUBSET marked**, so
  `draw_batches ≥ 2`, `late_draws == draw_batches`, the offset is evaluated at `i > 0`, and the
  mixed-archetype gather path runs. ⚠️ round 1's cited template `sv0_scene/mod.rs:276-303`
  **registers zero meshes** — it takes one `MeshHandle` parameter and spawns five instances of it.
  Follow instead a fixture that genuinely registers two distinct meshes:
  `crates/boyko_app/tests/textured_smoke.rs:110`/`:119` (floor + sphere) or
  `pbr_showcase.rs:98`/`:107`.
  **`vb_occ_multi` is deliberately NOT a golden pin.** Adding one would mean a new `VB_PINS` name, a
  density-census row and a blessing ceremony to buy a second byte-identity claim, when G2's counts
  plus G3 and G4 already cover what the multi-batch case adds. **The limit this leaves is stated
  plainly: no golden covers a multi-BATCH late scope. That is piece 3's first gate.**

**Controls:**
- **Red control (primary):** force `path_vb_occlusion_split()` to `false`. `scopes` reports 1 on the
  marked scene; G2 reds *while G1 stays green* — precisely the pair that proves G1 needs G2.
- **Red control for the `late_instances == 0` clause:** G1's C2 (`instanceCount = 1`). G1 stays green
  by construction and **G2 reds**. Nothing extra has to be executed; the cross-reference is the
  point, because otherwise that clause has no corruption of its own.
- **Red control for the `late_draws` clause:** on `vb_occ_multi`, `take(1)` in the late loop.

**Honest limit:** it proves the host *recorded* the scope, not that the GPU *executed* it. For a
scope with zero draws there is no observable consequence of execution, so no gate in this repository
can close that gap; the nearest independent evidence is G3.

### G3 — validation, armed vs unarmed, message-for-message

The P1-2 / P1-4 leg (*"19 messages armed and 19 unarmed, identical after handle normalisation"*),
run on the marked and unmarked scenes. This is the leg that sees the new scope's *legality*: the
`LOAD_OP_LOAD` against the layout the graph left, `vb_indirect_late`'s usage bits, the second
`DRAW_INDIRECT` access, the second `begin/endRendering` bracket, and the `vkCmdUpdateBuffer` alignment
and size rules on the late fill.

- **Red control:** drop `BufferUsage::INDIRECT` (⚠️ that is the flag's real name —
  `crates/boyko_rhi/src/enums.rs:37`; round 1 wrote `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`) from
  `vb_indirect_late`. Validation errors immediately.
- **Second red control:** declare `vb_raster_late`'s `vb_id` access with
  `VK_IMAGE_LAYOUT_UNDEFINED`. The graph then emits a first-touch transition where a preserving one
  must be, and the layout mismatch is a static VUID the layers report with or without sync-val.
- ⚠️ **Whether this leg can see a MISSING BARRIER is decided by P2-0, not assumed.** If sync-val is
  live, G3 is the strongest barrier gate in the tree and G4 is corroboration. If it is not, G3 sees
  only static legality — and then a missing barrier is invisible to G3, invisible to G1 (the late
  scope writes nothing), and invisible to G2 (a host count), leaving G4 and the declarator's
  `debug_assert`s alone. **Whichever answer P2-0 returns goes in the commit message**, because a
  green validation leg silently read as barrier evidence is exactly how this campaign has shipped
  vacuous gates before. Independently: the goldens themselves run with `BOYKO_DISABLE_VALIDATION=1`
  (`goldens/PINS.toml:42`), so the *golden* legs see no validation at all — only the explicit
  `-ValidationOn` runs do.

### G4 — the derived barrier stream, per CONFIGURATION, asserted FIELD-BY-FIELD

Extends P2-4's baseline. ⚠️ **Round 1 specified this gate as a barrier COUNT ("exactly two new
barriers"), and a count is exactly the assertion that certifies the defect it exists to catch**: the
read-declared/write-undeclared variant of D4 yields the *same* count and differs only in
`src_stage` / `src_access`. As specified in round 1 this gate went **red on the correct
implementation and green on the defective one**. It now asserts fields.

**The matrix.** Round 1 pinned "with and without HZB armed", which misses every reader the D6 slot
move re-sources. Eight configurations, four authored at P2-4 on the unmodified declarator and four at
P2-5:

| # | split | HZB | dump | SSAO / path | what it exists to catch |
|---|---|---|---|---|---|
| U1 | off | off | off | off / VB×Mesh | the shipping baseline — nothing about the split leaks into the unarmed path |
| U2 | off | armed | off | off / VB×Mesh | today's `vb_mesh_hzb` shape, incl. the three barriers at `levels = 10` |
| U3 | off | armed | **on** | off / VB×Mesh | **G5's own path** — `hzb_dump`'s `vb_depth` source, and `hzb_poison`'s `UNDEFINED → GENERAL` first touch |
| U4 | off | armed | off | **on** / **VB×Both** | `vb_viewt` PRE-TAIL (`:4028-4034`) and `sdf_forward_march`'s mesh arm (`:4480-4487`) |
| S1 | **on** | off | off | off / VB×Mesh | the three new barriers at the late boundary |
| S2 | **on** | armed | off | off / VB×Mesh | the depth round trip across the moved poison+build block |
| S3 | **on** | armed | **on** | off / VB×Mesh | **mandatory** — `hzb_dump` is re-sourced by the move, and G5 runs here |
| S4 | **on** | armed | off | **on** / **VB×Both** | the other three re-sourced readers |

**S1's three barriers at the `vb_raster_late` boundary, field by field:**

| resource | src_stage | src_access | dst_stage | dst_access | layout |
|---|---|---|---|---|---|
| `vb_id` | `COLOR_ATTACHMENT_OUTPUT` | `COLOR_ATTACHMENT_WRITE` | `COLOR_ATTACHMENT_OUTPUT` | `COLOR_ATTACHMENT_WRITE` | `COLOR_ATTACHMENT_OPTIMAL` → same (WAW, **no layout change, NOT from UNDEFINED**) |
| `vb_depth` | `FRAG` | `DEPTH_STENCIL_ATTACHMENT_WRITE` | `FRAG` | `DEPTH_STENCIL_ATTACHMENT_WRITE` | `DEPTH_ATTACHMENT_OPTIMAL` → same (WAW) |
| `vb_indirect_late` | **`VK_PIPELINE_STAGE_TRANSFER_BIT`** | **`VK_ACCESS_TRANSFER_WRITE_BIT`** | **`VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT`** | **`VK_ACCESS_INDIRECT_COMMAND_READ_BIT`** | n/a (buffer) |

**S2 additionally** asserts the depth round trip `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL`
(into `hzb_build_0`) → `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` (into `vb_raster_late`),
**neither transition from `UNDEFINED`** — a first touch here would *discard* the early scope's depth,
which is round-1 blocker 4's failure (`VG-R3-HZB-PLAN.md:105-108`) — plus the pyramid's own three,
unchanged in content and moved in position.

**S3 additionally** asserts that `hzb_dump`'s `vb_depth` transition has changed from the
execution-only arm to a real RAW flush (`FRAG` / `DEPTH_STENCIL_ATTACHMENT_WRITE` → `TRANSFER` /
`TRANSFER_READ`) with a `DEPTH_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL` transition, **and that this
is the asserted-correct value**, not a regression — `graph_bridge.rs:4652-4654`'s comment
(*"on every armed frame that is `SHADER_READ_ONLY_OPTIMAL`, since `hzb_build_0` itself reads it
there"*) is falsified by this piece and is edited in P2-5. S4 asserts the same character change for
`vb_viewt` PRE-TAIL and `sdf_forward_march`'s mesh arm.

**Red controls — five, all executed:**

| # | corruption | expected |
|---|---|---|
| R1 | delete `vb_indirect_late_upload`'s `buffer_access` | **RED on the FIELDS while the COUNT stays 3** (`TOP_OF_PIPE` / `0`). This is the control that proves the field assertion is load-bearing and that a count-based gate would have passed. |
| R2 | delete `vb_raster_late`'s `buffer_access(DRAW_INDIRECT, INDIRECT_COMMAND_READ)` | RED, count drops to 2 |
| R3 | delete `vb_raster_late`'s `vb_depth` `image_access` | RED — the round-trip transitions vanish and the late scope would `LOAD_OP_LOAD` an image left in `SHADER_READ_ONLY_OPTIMAL` |
| R4 | declare `vb_raster_late`'s `vb_id` access with `VK_IMAGE_LAYOUT_UNDEFINED` | RED — a first touch appears where a preserving transition must be |
| R5 | move `hzb_build` **alone**, leaving `hzb_poison` at the old slot | RED, **and it must be shown reddening twice**: the `debug_assert!` at `graph_bridge.rs:4711-4717` fires in dev profile (which is what the golden runs use — `golden.ps1:180`/`:193` carry no `--release`, and `PINS.toml:759` relies on it), and in a release binary the clear runs after the dispatches and `hzb_engine_pyramid_gate.rs:507-517`'s clause 1 reds at every texel. |

**Authoring order is mandatory:** the baseline (P2-4) precedes the machine (P2-5). P1-5a's C1/C2
exists because *"Authoring them after the change would certify the new behaviour."*

⚠️ **What G4 CANNOT claim, and it is a real limit.** G4 is a **synthetic-declaration pin**: a
hand-written replica of the declarator, because `declare_vb_graph` is `pub(crate)` on a `Renderer` no
test constructs (its only references are the definition at `graph_bridge.rs:3093` and the `3 =>`
dispatch arm at `:706`; every test hit is a doc-comment mention). The tree's existing framegraph pin
says this about itself, verbatim: *"**Nothing about `declare_deferred_graph` ITSELF.** This is a
hand-written REPLICA of it"* (`crates/boyko_rhi_vulkan/tests/framegraph_gbuffer_equiv.rs:2405-2412`).
So G4 proves the framegraph *derives* the right stream from a declaration shaped like the one
`declare_vb_graph` writes — **not that `declare_vb_graph` writes that shape.**

**What closes that gap** is not another pin; it is asserts that run inside production, in the
dev-profile builds every golden run uses:

```rust
// in declare_vb_graph, all NEW in P2-5:
debug_assert_eq!(vb_raster_late.is_some(),          scene.path_vb_occlusion_split());
debug_assert_eq!(vb_indirect_late_upload.is_some(), scene.path_vb_occlusion_split());
debug_assert!(vb_indirect_late_upload.is_none_or(|u| vb_raster_late.is_some_and(|l| u.index() < l.index())),
    "invariant: the late indirect upload is declared before the late raster reads it");
debug_assert!(vb_raster_late.is_none_or(|l| hzb_build.iter().flatten().all(|b| b.index() < l.index())),
    "invariant: on an armed split the pyramid build precedes the late raster");
debug_assert!(vb_raster_late.is_none_or(|l| vb_raster.is_some_and(|e| e.index() < l.index())),
    "invariant: the early raster precedes the late raster");
debug_assert_eq!(vb_indirect_late.index() + 1 - VB_IMAGE_COUNT, VB_BUFFER_COUNT,
    "invariant: vb_indirect_late is the LAST buffer ResId"); // mirrors the image assert at :3272-3276
```

plus **G2's recorder-originated `scopes`**, which is the only number in this piece that comes from the
real recorder rather than a replica. The three together — replica pin, production asserts, recorder
count — are the evidence; none of them alone is.

### G5 — G8 under the split, **ADDED to piece 1's G8, not replacing it**

⚠️ **Round 1 said "re-run `hzb_engine_pyramid_gate` on the marked scene", and that would have
CONVERTED the gate rather than extended it.** The file has one `setup` (`:155-218`), one worker
(`#[test]` `:226`, `#[ignore]` `:227`, `fn hzb_engine_pyramid_dump` `:228`) with `HzbMode::Build`
hardcoded (`:245-249`), and one driver (`#[test]` `:412`, `fn hzb_engine_pyramid_equals_the_oracle`
`:414`) that spawns the worker by name (`const WORKER` `:91`, `run_dump_worker` `:377`, args with
`--exact` at `:385`). The only thing "re-run on the marked scene" can change is `setup` — which
**deletes the unmarked leg**. That leg is the configuration every shipping frame and all 25 pins
take, and it is the one whose `hzb_build` slot piece 2 newly makes *conditional*. Nothing else covers
it: `hzb_build_spv_sync` is a byte gate, `hzb_build_oracle_gate` is structurally blind by its own
header (`:21`, *"The ONLY thing it shares with the engine is…"*), G4 is synthetic, and the parity
asserts cover `vb_raster_late`, not the HZB slot.

**⇒ G5 is a SECOND worker + driver pair in the same file**: `WORKER_OCC`,
`hzb_engine_pyramid_dump_occ` (the same `setup` plus the marker in the spawn bundle) and
`hzb_engine_pyramid_equals_the_oracle_occ`, writing to a distinct dump path. No new test binary is
needed — the driver already selects by name with `--exact`. **Both pairs must be green in the same
sitting**, and the plan demands both legs at G2, G3 and G4 already; G5 was the only gate that did
not, and now does.

- **What it proves:** the slot move did not hand `hzb_build_0` a wrong or untransitioned image — the
  pyramid the engine builds from the *earlier* slot is still bit-exact against `boyko_render::hzb`
  over the dumped depth, with the `-1.0` poison (`HZB_PYRAMID_POISON`, `scene_types.rs:1454`;
  `poison_bits` at `:485`) and all five non-vacuity clauses intact.
- **Red control:** R5 above (move the build without the poison) reds clause 1 at every texel in
  release, and fires the declare assert in dev.
- ⚠️ **What it structurally CANNOT prove, in piece 2:** that the *ordering* is right. The late scope
  draws nothing, so early-depth and end-of-frame depth are the same bytes, and a pyramid built at
  either slot agrees. The ordering's real gate is piece 3's, and piece 3 must first move the dump's
  depth copy (`vb.rs:3327-3336`, gate `:3227`) between the scopes or dump both depths (D6). Naming
  this here is what stops a green G5 from being read as ordering evidence.

### Mandatory unit tests

- `size_of::<OcclusionCulling>() == 0` (const-assert, the `csm_marker.rs:28-29` shape).
- `inst_flags.len() == ring.len()` after a gather with a hole (a retired mesh) — the `mesh_ids`
  lock-step test's shape (`mesh_draw.rs:1757-1758`).
- **Two-gather reset:** a marked gather followed by an all-unmarked gather **on the same reused
  scratch** leaves `occlusion_instances() == 0`. Modelled on
  `any_non_default_material_flag_resets_per_gather` (`mesh_draw.rs:1864-1884`). ⚠️ A single-gather
  count check is green with or without the reset and is **not** a substitute.
- **Lane-clear regression:** a long gather followed by a short one leaves no stale tail — assert
  `inst_flags.len() == ring.len()` and that the fold's count matches a recount over
  `inst_flags[..ring.len()]`.
- `occlusion_instances()` counts exactly the marked instances that reached the ring, across a gather
  that skips a non-`Loaded` bucket.
- `VbInstanceRow::from_model_col` sets `flags` from its new argument and leaves `_pad == [0, 0]` —
  the existing `:267-276` test updated rather than deleted, and the `:275` assert narrowed.
- `path_vb_occlusion_split()` is false on every non-VB path, on `VB × Sdf` (no `mesh_leg`), and at
  `vb_occlusion_instances == 0`; true only when all three conjuncts hold.
- Both `gather_mesh_draws` variants produce byte-identical `ring` / `mesh_ids` / `material_ids` /
  `inst_flags` lanes (the *"the OFF path never diverges"* contract, `mesh_draw.rs:1205-1207`).
- **The cross-pin hash guard** (G1): `vb_occ_split.sha256_{software,hwrt} == vb_mesh.sha256_*`, and
  the existing `vb_mesh_hzb` == `vb_mesh` pair, parsed from `goldens/PINS.toml` — the
  `vg_density_census.rs:188-199` machinery.
- `VB_PINS.len() == 15` and the set-equality test green (`vg_density_census.rs:187`).

### `debug_assert!` invariants

- Every late record has `first_instance == 0` (mirrors `vb.rs:979-982`).
- `plan.vb_raster_late.is_some() == scene.path_vb_occlusion_split()` and the same for
  `vb_indirect_late_upload`, **at both the declare and the record site** — declare/record parity as an
  assert rather than a convention. ⚠️ Note the tree's idiom for a *conditional* pass unwrap is
  `.expect("invariant: …")` (`vb.rs:1469-1471`), not a bare `debug_assert` — the late scope's plan
  fields are `.expect()`ed under the predicate, and the parity asserts are additional.
- `record_capacity_late >= draw_batches` (the late loop is bounded by `draw_batches`, D4).
- The late scope's `renderArea` equals the early scope's (D4's equivalence depends on it).
- `vb_late_instances == 0` in piece 2, with a message naming piece 3 as the step that removes it.
  **A tripwire that must be deleted deliberately**, not a check that quietly stops holding.
- The declare-order asserts listed under G4.

## Boundary — what piece 2 does NOT do

The review rounds got value from this list, so it is explicit and it is long.

**No occlusion DECISION of any kind.** No screen rect, no `depth_near`, no `occ`, no
`depth_near < occ`, no `KeepReason`, no call into `boyko_render::hzb`'s `project_aabb` /
`select_texels` / `occluder_depth` / `occlusion_verdict`.

**No shader CODE edit.** `vb_batch_cull.comp.hlsl`, `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl`,
`vb_geom_fetch.hlsli` and every `.spv` are byte-unchanged apart from three **comment** corrections
whose `.spv`-neutrality is gated by the `*_spv_sync` re-DXC tests (Integration). `VbInstanceRow::flags`
is written by the host and read by nothing on the device — the R2d-2/R2d-3 rule: *"the descriptor
arrives before its consumer so the consumer rung changes only shader code"*
(`scene_types.rs:2814-2816`).

**No late cull pass.** No second compute dispatch, no `vb_cull_layout` widening, no new
descriptor-set layout, no new descriptor set, no new pipeline. Piece 2 mints **zero** boot objects
beyond one buffer.

**No `vkCmdDrawIndexedIndirectCount`, and no device-feature change.** Adding it would require chaining
`drawIndirectCount` in `VkPhysicalDeviceVulkan12Features`, which `device.rs:615-618` records as
deliberately not done. Out of scope for pieces 2 and 3.

**No survivor-list change.** `VB_VISIBLE_INSTANCE_ELEMS`, `INSTANCE_CAPACITY`,
`vb_cull_batch_count_visible_clamp` and the R2d-6 const-assert (`gpu_scene/mod.rs:258-264`) appear in
the diff as context only.

**No two-ended region partition.** The budget proof is stated (D5) so piece 3 inherits it; the packing
is piece 3's.

**No HZB tap.** The research's corrected tap shape — four `.Load`s plus a shader-side `min`, matching
`hzb_build.comp.hlsl`'s existing discipline, **not** a `REDUCTION_MODE_MIN` sampler and **not** a
linear filter — is recorded in Context as piece 3's inherited constraint, not implemented here.

**No `prev_view_proj`, no "visible last frame" bit, no prev-ring work.** Round-1 blocker 6
(`VG-R3-HZB-PLAN.md:112-115`) records that `prev_ring` / `gather_prev_ring_into` /
`upload_prev_instance_models` are all `#[cfg(feature = "hwrt")]` with no plugin adding the column.
Untouched. (The research's open question 1 — stored visibility bit vs HZB re-test with previous
transforms — is therefore still open and belongs to piece 3.)

**No arming config.** `HzbConfig` gains no variant — `hzb_config.rs:53-59` fixes it at two,
permanently, and names the consumer knob as pieces 3/4's. No `OcclusionConfig` is introduced.

**No `MeshGpu::local_min` / `local_max` consumer.** D7.

**No change to `first_instance`, to `drawIndirectFirstInstance`, or to `multiDrawIndirect`.**

**No perf claim.** None is measurable in this tree (`OPEN-QUESTIONS.md:260-279`); the Sponza fixture
is not built. Piece 2 adds cost and removes none, and the Cost table in the Goal is a *description*,
not a measurement.

**No fix for the 19 outstanding validation messages.** Owner-deferred
(`OPEN-QUESTIONS.md:157-163`), option (b).

## Open questions

1. **VALUES — the default policy.** Should the engine's own mesh bundle eventually
   `#[require(OcclusionCulling)]` — which the D1 reversal makes *possible*, since a required ctor
   needs a real column — making "cull everything" the default and marking the exceptions? Piece 2
   ships opt-in (conservative and reversible). Flipping it changes what a naive user sees on screen,
   so it is the owner's call. Blocks nothing.
2. **MEASUREMENT — the cost of a zero-`instanceCount` indirect draw**, per record, on this device,
   with `multiDrawIndirect` off. The research found **no published number** for native Vulkan. The
   design already bounds the exposure (the late loop is `draw_batches`, not 1024). Settling it would
   take a timestamp-query A/B of `N` zero-instance indirect draws against zero draws, in the same
   sitting, under the `VG-DECIDABILITY-FLOOR` protocol — and per that document's own finding
   (6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol) **a delta under ~15 % is not
   defensible**. It would be a synthetic microbenchmark, never a pin.
3. **Does `vg_density_census_gate` (`vg_density_census.rs:330`, `#[ignore]`d, live-GPU) require a
   measured density row for `vb_occ_split`, or does the set-equality bump alone suffice?** Settled by
   reading `vg_density_census.rs:57-64` at P2-6. If a row is required, `vb_occ_split`'s is
   `[vb_mesh]`'s by construction (same scene, same geometry) and the pre-fill-and-verify pattern at
   `PINS.toml:353-358` / `:477-483` is the shape.
4. **Naming.** The `vb_raster` identifier is kept for the early pass rather than renamed to
   `vb_raster_early`: 20+ anchors across four files and several docs cite it by name, the pass name
   string reaches `OpSource`-adjacent debug paths, and the R9 plan's §0 already rejected a rename on
   exactly this ground. Prose says "early". If the reviewer prefers the rename it is mechanical, but
   it is churn in a step whose whole claim is that nothing changed.
5. **Whether the frozen `dxc` recipes are comment-neutral.** Asserted (no `-Zi`/`-Qembed_debug`) and
   **gated** by the `*_spv_sync` re-DXC tests at P2-2 rather than assumed. If they red, the three
   comment corrections are reverted and the finding is recorded here.
6. **`vb_indirect_late`'s ResId and the graph's capacity budget — CLOSED, no blocker.**
   `FrameGraph::with_capacity(16, 16, 64)` (`frame_driver.rs:188`) is a **hint**: `graph.rs:245-248`
   states *"exceeding a cap in `reset`-then-declare only regrows the `Vec` (cold), it is never a
   correctness issue"*, and the only real bound is `u16::MAX` (`:414-417`). The VB path already
   declares 28 resources non-hwrt (15 images + 13 buffers) / 35 under hwrt; one more is noise. Round
   1 left this open with a "I have not read" hedge; it is read and closed.
7. **`golden.ps1`'s blessing path — CLOSED.** It cannot re-bless without `-Bless` (sole write:
   `Set-Content` at `:259` inside `if ($Bless)` at `:240`; check path exits 2 on an absent or
   `PENDING` hash, `:266`/`:270`). What was missing is the *guard* against a human re-blessing, and
   G1 now ships it.

---

**Files an implementer starts from** (all anchors re-verified at `9e80cd4`):
`D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\present\passes\vb.rs` — the early indirect fill
`:942-999` (`cmd_update_buffer` at `:990-998`), the raster scope `:1264-1449` (draw loop `:1371-1447`,
`record_capacity` `:909-911`, `draw_batches` min-chain `:935-940`), the **poison** record block
`:1956-2019` and the **build** record block `:2021-2155`, the dump `:3207-3384` (depth copy
`:3327-3336`), `record_vb`'s signature `:349-365`.
`D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\present\graph_bridge.rs` — `vb_indirect_upload`
`:3517-3529`, `vb_raster` `:3608-3664` (the gap after it `:3665-3677`), the **poison** declaration
`:3926-3939` and the build chain `:3941-4014`, the sink `:2860-2911` + `:4854-4950`, `VbPassPlan`
`:2630` / construction `:4730-4771`, the declare-order asserts `:4698-4726`, the falsified `hzb_dump`
comment `:4652-4658`.
`D:\claude\BoykoEngine\crates\boyko_app\src\gpu_scene\mod.rs` — the capacity family `:145-278`, the
R2d-6 assert `:258-264`, `vb_indirect` `:3744-3764`, `scene()` `:5472-5643`, destroy `:7042-7044`.
`D:\claude\BoykoEngine\crates\boyko_render\src\mesh_draw.rs` — `gather_mixed_into` `:601-724` (resets
`:609-612`, primary loop `:671-708`), `sync_vb_instance_ring` `:483-498`, the two gathers `:1117` /
`:1218`.
`D:\claude\BoykoEngine\crates\boyko_render\src\instance_model.rs` — `VbInstanceRow` `:221-249`, the
pin `:267-276`.
`D:\claude\BoykoEngine\crates\boyko_render\src\csm_marker.rs` — the marker template `:25-29`.
`D:\claude\BoykoEngine\crates\boyko_render\src\csm_caster.rs` — the Axis-1/Axis-2 query `:171-176`,
the shared-core closure `:199`, the wrapper `:88-89`.
`D:\claude\BoykoEngine\crates\boyko_app\src\runner.rs` — the `csm_armed` seam `:2009`, the fold
`:1943-1997`, the `.scene(` call `:2364`, the scratch scalars `:2386`/`:2390`.

---

# ROUND 1 CRITIQUE — five lenses, every finding adversarially refuted

**28 findings survived a skeptic; 14 were killed.** Two of the three blockers are **gates that as
written would certify the very defect they exist to catch** — B2's G4 goes RED on the correct
implementation and GREEN on the defective one. That is the fourth and fifth instance of this class
in the campaign, and the reason "can this gate fail?" is now the FIRST question asked of a gate.

⚠️ **Every line number below is as-of-the-critique.** Re-anchor at use. That exact trap produced a
self-contradiction in piece 1 (`VG-R3-P1-PYRAMID-PLAN.md` §12), where a fact list captured at one
commit was applied at a later one and two of its own instructions then conflicted.

# VG R3 PIECE 2 — ARCHITECTURE CRITIQUE, ROUND 1 CLOSE

## (1) VERDICT

**APPROVED_WITH_CHANGES**

The load-bearing decisions survived: D2's seam (per-frame conjunction on the scene struct, the `csm_armed` shape), D4's candidate B, D5's two-ended budget, D6's slot-move-by-one-predicate, D7's "piece 2 does not touch it". None was refuted. What must change is **one Key Decision (D1, a reversal, not a wording fix)** and **three gate specifications that as written certify the defects they exist to catch**. The plan must not enter P2-1 until B1–B3 land in the text.

---

## (2) BLOCKERS

### B1 — D1 picks the EnableTag mechanism while arguing capability-PRESENCE semantics; the code it specifies does not compile, and the nearest naive repair makes G1 silently vacuous

`:39-43`, `:192`, `:289`, `:299`, `:350`, `:434`.

`#[component(storage = "bitset")]` IS the EnableTag backend in this kernel. Three independent consequences, each verified:

- **Unbuildable, twice.** `Has<T>` does not exist as a query term (`crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs:11` is a doc comment about *resources*; the only real `Has` is `HasRelation<R>`). And `crates/boyko_macros/src/component.rs:315` — `if hooks.no_bundle || hooks.storage_bitset || hooks.storage_dense` — suppresses the `Bundle` impl, so G1's fixture (`:350`, "`OcclusionCulling` **inserted** on every mesh entity") does not compile either.
- **Semantics inverted.** A bitset id is dropped from every archetype signature, has no `ComponentPool`, and every never-toggled row reads `false`. So `:41`'s "presence is the datum, absence is a structural skip" is false in both halves, `:16`'s "structural skip, not a branch" is unreachable, and `:43`'s `#[require(...)]` mitigation cannot work (a require writes bytes through a ctor into a column that does not exist; `light.rs:284` is `#[require(Transform, GlobalTransform)]`, both table components).
- **The naive repair is the campaign's fourth vacuous gate.** Patch the build error with `With<>` or `Option<&T>` and every row reads absent ⇒ `occlusion_instances == 0` ⇒ `path_vb_occlusion_split()` false ⇒ the late scope is never recorded ⇒ **G1 returns byte-identical because nothing happened**, which `:356` itself names as the thing G1 cannot distinguish.

The tree already answered this exact fork in one line, inside the file the plan cites as its precedent: `crates/boyko_render/src/csm_caster.rs:173` — `Query<(&MeshHandle, &InstanceModelCol), (Enabled<RenderEnabled>, With<ShadowCaster>)>`. `With<ShadowCaster>` = structural capability (table ZST); `Enabled<RenderEnabled>` = runtime on/off (bitset). A grep of the plan for `EnableTag|IsEnabled|Enabled<|enable::<` returns **zero** hits — the fork was answered invisibly.

**Plan text changes:** name the axis and match storage to it.
- Recommended: **table storage**, verbatim the `ShadowCaster` shape (`csm_marker.rs:25-26`, no `storage` attribute), read non-filtering as `Option<&OcclusionCulling>` at `:289`/`:299`. Rewrite `:41`'s rationale (see M-a). Confirm `Option<&ZST>` resolves against a zero-width column — it does: `component_pool.rs:3412-3473` (Phase 22 D6) pins `stride == 0` construction and `pool.get_typed::<ZstTag>(i).is_some()`.
- If bitset is kept deliberately: rewrite D1 as an **enable bit**, not presence; spell the read `IsEnabled<OcclusionCulling>`; state the arming verb is `EntityCommands::enable::<T>()` (`entity_commands.rs:220`, deferred, chainable — this exists and G1 can use it); state the seed policy, because a `LightSeedState`-shaped seeder is otherwise required (`light.rs:243-248`, `light_system.rs:851`); withdraw `:43` and open question 1.
- Either way: delete `Has<OcclusionCulling>` from `:289` and `:299`, and correct `:299`'s multithreading claim — see F-9.

---

### B2 — `vb_indirect_late` gets an indirect fetch with no declared TRANSFER write, and G4 is specified to pin the missing barrier's absence

`:16`, `:228`, `:254-259`, `:313`, `:328`, `:330`, `:377`.

P2-5 fills `vb_indirect_late[fi]` by `vkCmdUpdateBuffer` (the buffer is DEVICE_LOCAL | TRANSFER_DST, so the fill can only be a transfer op) and fetches it with `vkCmdDrawIndexedIndirect` in the same command buffer. **No pass anywhere in the plan declares `buffer_access(vb_indirect_late, TRANSFER, TRANSFER_WRITE)`**: `:328` says P2-3 has "zero writers of the buffer"; `:313`'s graph_bridge list names no upload pass; `:254-259`'s `VbFramePlan` carries no upload `PassId` — and the recorder *requires* one (`vb.rs:947` does `record_vb_pass(plan.vb_indirect_upload.expect(..))`).

The early array's symmetric pair is `graph_bridge.rs:3518-3520` (TRANSFER_WRITE in its own `vb_indirect_upload` pass) + `:3613-3619` (DRAW_INDIRECT / INDIRECT_COMMAND_READ on `vb_raster`). The declarator states the consequence of omitting either, verbatim, at `:3510-3516`: *"an access the recorder performs but the declarator omits is a barrier derived nowhere — a missing barrier, not a wasted one."*

With no declared writer, `sync.rs:351-383` takes the first-touch arm: `src_stage = TOP_OF_PIPE, src_access = 0` — an execution-only edge that makes the update neither available nor visible. Frame 1, split armed, freshly allocated DEVICE_LOCAL memory: `instanceCount` is arbitrary, `firstInstance` may be nonzero (`drawIndirectFirstInstance` is VK_FALSE, `robustBufferAccess` OFF at `gpu_scene/mod.rs:256-257`), and the scope the whole piece claims "draws nothing" draws — a charter violation, not merely a defect. The compile-time backstop cannot help: `graph.rs:641`/`:955` is `!is_image || is_write || res_written[ri]` — a **buffer** read with no producer is waved through, and only in debug.

**G4 is pinned to the defective number, twice**: `:16` ("2 derived barriers") and `:377` ("exactly two new barriers between the scopes"). Three is the correct count once the write is declared. So G4 as specified goes **red on the correct implementation and green on the defective one** — and `:369`/`:371` designate G4 the only gate that can see this class.

**Plan text changes:**
1. Name the declaring pass and its predicate: add `vb_indirect_late_upload` to `VbFramePlan`, gated on exactly `path_vb_occlusion_split()`, declared **before** `vb_raster_late` (extending `vb_indirect_upload` instead requires reconciling its `scene.vb_indirect.is_some()` gate with `path_vb_occlusion_split()` under the W1 single-predicate rule).
2. Declare `g.buffer_access(vb_indirect_late, DRAW_INDIRECT, INDIRECT_COMMAND_READ)` on `vb_raster_late`, mirroring `:3613-3619`.
3. Fix the counts at `:16` and `:377` from 2 to **3**, and add the late fill's `vkCmdUpdateBuffer` to `:16`'s armed-frame command list.
4. **A count is not sufficient.** The read-declared/write-undeclared variant also yields three, differing only in `src_stage = TOP_OF_PIPE, src_access = 0`. G4 must assert the barrier's **fields**: `src_stage == VK_PIPELINE_STAGE_TRANSFER_BIT`, `src_access == VK_ACCESS_TRANSFER_WRITE_BIT`, `dst_stage == VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT`, `dst_access == VK_ACCESS_INDIRECT_COMMAND_READ_BIT`.
5. Red control: delete the upload access; the TRANSFER→DRAW_INDIRECT barrier must vanish from the pinned stream.

Note for piece 3: `:100` sells piece 3 as changing "one word's producer (host 0 → cull-written k)". Piece 3 would inherit a graph whose late indirect buffer has no declared producer and, by that framing, would not add one — at which point `instanceCount` is a live nonzero `k` and the missing dependency is hot every frame.

---

### B3 — D6 moves `hzb_build_*` and never mentions `hzb_poison`, which is asserted to precede it; the plan's own record anchor points into the poison block

`:148`, `:150`, `:313-314`, `:320`, `:387`, `:443`.

`hzb_poison` is a separate declared pass at `graph_bridge.rs:3926-3939`, immediately ahead of the build chain (`:3941`), recorded at `vb.rs:1974-2019` immediately ahead of the dispatches (`:2021`), and the declarator carries `debug_assert!(poison.index() < build.index())` at `:4711-4717` whose comment at `:4706-4710` predicts this failure in words: *"the dump would then read `-1.0` everywhere and G8 would red claiming 'the build never ran', which is a gate reporting the wrong defect."*

`grep -i poison` over the plan returns only `:344` and `:385`, both about the **−1.0 value**, never the pass. It is in neither the change list nor the "No change" list. Worse, the plan's anchors point the implementer *away* from it: `:443`'s declare anchor `graph_bridge.rs:3941-4014` begins **after** the poison block, and `:150`'s claim that `hzb_build` is "recorded at `vb.rs:1958`" lands on the **third line of the poison block's own comment** — the build's record block starts at `vb.rs:2021`. That is how the pass got dropped.

PassId is strictly monotonic declare order (`graph.rs:441-451`) and `compile()` does not reorder, so moving the build into the `:3664..:3678` gap puts `build.index() < poison.index()` — the exact negation of the assert. The configuration is not hypothetical: G5 (`:385`) is a marked scene under `HzbConfig::Build` with `BOYKO_HZB_DUMP`, i.e. armed-split **and** armed-poison in the same frame, by construction. Debug: the assert fires. Release (no `[profile.*] debug-assertions` override exists in the tree): the clear runs after the dispatches and `hzb_engine_pyramid_gate.rs:507-513` clause 1 reds at every texel. Either way `:387`'s claim that G5 proves the slot move was clean is **false as specified**.

**Plan text changes:** D6 must define the moving unit as `[hzb_poison, hzb_build_0..pass_count)` at **both** declare and record; correct `:150`'s record anchor to `vb.rs:2021` and `:443`'s ranges to `graph_bridge.rs:3901-4014` and `vb.rs:1956-2130`; add `graph_bridge.rs:4711-4717` (and the `build < dump` assert at `:4698-4704`) to the "Invariants that must survive untouched" table at `:24-33`; state the profile G5 runs under, because in release neither assert exists; add a G4 case "split + HZB armed + dump" pinning the poison's UNDEFINED→GENERAL first touch at the early slot. Red control: move the build alone; show the assert fire in debug and clause 1 fire in release.

---

## (3) MAJORs

**M1 — `:369`'s sync-val premise is false, and G3's true capability is unknown.** `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT` is at `crates/boyko_rhi_vulkan/src/device.rs:2152`, packed into `VkValidationFeaturesExt` (`:2153-2160`) and chained as the instance `p_next` head (`:2187-2193`) whenever validation is on and `VK_EXT_validation_features` is present. `golden.ps1:167` → `runner.rs:213` satisfies the first conjunct on every `-ValidationOn` pin. All three of the plan's own grep terms hit. But `:2110-2111` degrades **silently** when the extension is absent, and the published 19-message baseline (`OPEN-QUESTIONS.md:144-151`) is entirely `vkCreate*`-time entries — nothing establishes the feature was ever live. *Change:* delete `:369`'s three sentences; move the "sees a missing barrier" label from G4 (`:371`) to G3; re-label G4 honestly as a **synthetic-declaration pin** (a hand-written replica — `declare_vb_graph` is `pub(crate)` on a `Renderer` no test constructs, and `framegraph_gbuffer_equiv.rs:2405-2412` says so about itself). *Settle first, before trusting any green:* delete `hzb_build_0`'s `vb_depth` `image_access` (`graph_bridge.rs:3986-3992`), run `scripts\golden.ps1 -Pin vb_mesh_hzb -ValidationOn`, record whether a `SYNC-HAZARD-*` appears. If not, the limitation to write in the commit is "the extension is absent on this device", not "sync-val does not exist in the repo".

**M2 — `occlusion_instances` has no per-frame reset, so the arming predicate is sticky-true for the process.** `:220` declares it a scalar on `MeshRenderScratch`, which is `#[derive(Resource)]` (`mesh_draw.rs:265-266`); `:288` specifies only `occlusion_instances += that`; `grep -i "reset|sticky"` over the plan = 0 hits. Every sibling per-frame reduce in the *same function* is reset first, with the reason written down: `mesh_draw.rs:609-612`, the first statement of `gather_mixed_into` — *"a persistent `Resource` field must not stay sticky-true after a material is removed"*. This falsifies `:219`'s own contract ("`> 0` ⇔ present in **this** frame's ring") and D3's per-frame reasoning at `:89`. The per-instance lane is re-scattered, so no geometry is lost — bounded damage is a permanently-armed split. Note `:272` cites `CsmCasterScratch::batch_count()` as its model, and that is a **derived** count over a per-frame-cleared column (`csm_caster.rs:95-97`) — structurally incapable of going sticky. *Change:* specify `self.occlusion_instances = 0;` beside `mesh_draw.rs:612`, and add the two-gather regression test to `:392-397`, modelled on the sibling at `mesh_draw.rs:1864-1884`. The plan's single-gather count check at `:394` is green with or without the reset.

**M3 — the only proposed fixture has exactly ONE `DrawBatch`, so every per-batch clause is unfalsifiable.** `:350` clones `[vb_mesh]` verbatim; its five spheres share one `MeshHandle` (`vb_mesh.rs:15-22`, `PINS.toml:281-284`) ⇒ `batch_count == 1`. So `:357`'s `vb_late_draws == <batch count>` collapses to `== 1`, satisfied by a hard-coded draw or `take(1)`; the `i * DRAW_INDEXED_INDIRECT_STRIDE` offset (`vb.rs:1432`) is only ever evaluated at `i == 0`. No test in `crates/boyko_app/tests` registers more than one mesh, so the implementer will not stumble into a multi-batch case. This directly refutes `:105` ("Every dangerous detail of the late draw loop lands in piece 2, under review"). **Second harm:** `:353`'s `base_instance + 1` control also does not fire on this fixture — all five instances share one batch with `base_instance == 0`, so `+1` re-draws instance 1 at its own position, bit-identical depth, `GREATER` fails on equal depth. G1 is then left with only the `LOAD_OP_CLEAR` control as evidence it can see a late **draw**. *Change:* author `vb_occ_split` with ≥2 registered meshes (new authoring; follow `crates/boyko_app/tests/sv0_scene/mod.rs:276-303`), place the second mesh where the early scope does not already cover it, and mark a **strict subset** of instances. Drop the `i < record_capacity_late` claim from the fixture's job — that bound only binds above 1024 batches; its guard is D5's const-assert.

**M4 — G1's "cannot be blessed wrong" is false; the harness prints the destroying advice.** `golden.ps1:246-259` overwrites `sha256_*` with whatever the run produced, and `:283` prints *"re-run with -Bless"* on the very mismatch the pin exists to report. Nothing parses PINS.toml to check two pins agree — `sha256_software` occurs only in `golden.ps1` and `PINS.toml`. The tree states this failure for the identical construction one pin above: `PINS.toml:315-318`, *"a lone re-bless here would silently convert the gate from 'inert' to 'whatever it does now'"*. *Change:* strike "cannot be blessed wrong" from `:350`; add a `#[test]` guard asserting `vb_occ_split.sha256_software == vb_mesh.sha256_software` (and `sha256_hwrt`, and the existing `vb_mesh_hzb`/`vb_mesh` pair). The machinery exists — `crates/boyko_app/tests/vg_density_census.rs:186-208` already reads and parses `goldens/PINS.toml`; the guard is a ~10-line extension. Also close open question 5 (see F-6).

**M5 — the new pin reds `the_a_domain_is_exactly_the_vb_pins_that_were_measured`, a test the plan never names.** `vg_density_census.rs:55` is `VB_PINS: [&str; 14]`; `:196` filters `s.starts_with("vb")`; `:200-207` asserts set-equality. `vb_occ_split` matches, so `found` becomes 15 and the test fails — not `#[ignore]`d, so it runs under plain `cargo test -p boyko-app`. P2-6 as written commits a red step, contradicting `:324`. `PINS.toml:57-64` records that this test already caught exactly this omission once in this campaign. *Change:* add `vg_density_census.rs` to the Integration table and bump `VB_PINS` to 15 in the same commit as the pin. The pin itself needs no new test binary — `[vb_mesh_hzb]` (`PINS.toml:309-341`) reuses `test_binary = "vb_mesh"` / `test_name = "vb_mesh_screenshot_dump"` and arms via an env knob read **inside the fixture** (`vb_mesh.rs:198-200` reads `BOYKO_VG_HZB` and calls `app.insert_resource(HzbConfig{..})`); a `BOYKO_VG_OCC=1` branch takes the identical route.

**M6 — P2-2 and P2-3 cannot "build green and commit alone"; the Integration table is missing ~5 files, one of them production.** P2-2 widens `gather_mixed_into`'s item tuple (`mesh_draw.rs:605-607`), which breaks: **`crates/boyko_render/src/csm_caster.rs:199` (production**, closing `|(h, col)| (h.0, col, None, PerInstanceMaterial::default())` into the shared core, a reuse `csm_caster.rs:14` names as a deliberate contract), `crates/boyko_app/tests/zero_alloc.rs:139` (the Principle-5 gate `frame_helpers_allocate_zero_after_warmup`), `crates/boyko_app/tests/structural_zero_substep.rs:59`, and `csm_caster.rs:562`/`:674`. P2-3 adds two `pub` fields to `GBufferScene`, which has **no `Default` impl anywhere in `crates/`** and four exhaustive literals in `window_present_gbuffer.rs` (`:2265`, `:3366`, `:8390-8437`, `:9904`). Also absent: `crates/boyko_app/src/runner.rs`, the sole `.scene(` caller and the only place `MeshRenderScratch::occlusion_instances()` can reach `GBufferScene` (the precedent scalars `any_non_default_material`/`any_textured_material` are read off the scratch there at `:2386`/`:2390`; `scene()` at `gpu_scene/mod.rs:5472-5542` takes no scratch). *Change:* extend the table to ~12 files, fix the "(7 files, 2 new)" header (the table already has 8 rows / 1 NEW), name `csm_caster.rs` in P2-2 **and state what occlusion value a caster row supplies** — D2's seam table never mentions that the CSM gather shares the core and would run the fold on its own scratch.

**M7 — G5 as written REPLACES piece 1's G8; the unsplit pyramid path loses its only engine-level gate.** `hzb_engine_pyramid_gate.rs` has one `setup` (`:155-218`) and one worker (`:228`), with `HzbMode::Build` already hardcoded (`:245-249`), so the only thing `:385` ("re-run on the marked scene") can change is `setup` — converting the gate. The unmarked case is the one every shipping frame and all 25 pins take, and piece 2 newly makes its `hzb_build` slot **conditional**. Nothing else covers it: `hzb_build_spv_sync` is a byte gate; G3's oracle is structurally blind by its own header (`hzb_build_oracle_gate.rs:3-6`); G4 is synthetic; `:402`'s parity assert covers `vb_raster_late`, not the HZB slot. Note the plan demands both legs at G2 (`:357`) and G3 (`:366`) and G4 (`:375-378`) — only G5 does not. *Change:* state that G5 is an **additional** marked worker+driver pair (a second `#[test] #[ignore]` fn in the same file — the driver selects the worker by name with `--exact` at `:385`, so no new binary is needed, only a distinct output path), and require both green in the same sitting.

**M8 — the HZB slot move re-sources every downstream `vb_depth` reader; G4's "split, HZB armed" enumeration is incomplete, and the reader it silently changes is `hzb_dump` — G5's own path.** `graph_bridge.rs:4652-4658` states the assumption in words: *"on every armed frame that is SHADER_READ_ONLY_OPTIMAL, since `hzb_build_0` itself reads it there."* After the move, `vb_raster_late` is the last toucher at `DEPTH_ATTACHMENT_OPTIMAL` with a pending write, so `hzb_dump`'s transition changes character — today the `visible_stages != 0` arm (COMPUTE, `src_access 0`, execution-only), after the move the `flush_access != 0` arm (a real RAW flush FRAG/DEPTH_STENCIL_ATTACHMENT_WRITE → TRANSFER). Same for `vb_viewt` pre-tail (`:4026-4034`), `sdf_forward_march`'s mesh arm (`:4480-4487`) and `vb_viewt` late (`:4535-4543`). The `vb_viewt` precedent D6 leans on does **not** transfer here: both of `vb_viewt`'s slots sit after `vb_raster` with no depth writer between, so its move re-sources nothing; `hzb_build_0`'s move crosses a new writer. Every derived barrier is sound in both slots — this is a modelling gap, not a correctness defect. *Change:* enumerate the derived `vb_depth` stream per **configuration** — {split} × {HZB} × {SSAO} × {VB×Mesh / VB×Both} × {hzb_dump} — with the `hzb_dump` row mandatory because G5 runs on it; author P2-4's baseline over the same matrix, not just "with and without HZB armed"; and add `graph_bridge.rs:4652-4658` to `:313`'s edit list, since piece 2 falsifies that comment.

---

## (4) MINORs worth doing

- **M-a — `:41`'s bitset-vs-table rationale does not distinguish the two.** "the consumer is a per-instance *test*… not an archetype filter that materialises rows" is a false dichotomy: `Option<&T>` is the non-filtering per-row read for table storage (`option.rs:56`; `data_is_enabled.rs:3-4` calls `IsEnabled<T>` "the order-preserving twin of `Option<&T>`"). `With<ShadowCaster>` is filtering because *that gather wants only casters* (`csm_caster.rs:168-170`), not because table storage forces it. Both cited bitset precedents are Axis-2 toggles in the tree's own words (`light.rs:230` section header; `snap_interpolation.rs:79-83`, a one-frame bit that disables itself). Replace with the real trade-off once B1 is decided. The one genuinely distinguishing cost — a table ZST on a subset fragments the mesh archetype and shortens per-archetype runs — appears nowhere in the plan.
- **M-b — `:228` mints `vb_indirect_late` without `STORAGE`, so D4's "Nothing structural" (`:100`) is false about the buffer piece 2 itself mints.** The early array carries STORAGE for precisely the reason piece 3 will need it (`gpu_scene/mod.rs:3747-3759`); `create_buffer` ORs only the TRANSFER bits (`device.rs:50-54`); the write is descriptor-bound, not a transfer (`vb_batch_cull.comp.hlsl:253` `RWByteAddressBuffer VbIndirect : register(u0)`, store at `:473-475`). Mint it with `BufferUsage::STORAGE` now — legal and inert on a buffer nothing binds — and note that piece 3 adds the binding and the `vb_cull_layout` slot. (Unlike the R2d-5 `TRANSFER_SRC` case the plan cites elsewhere, this bit is *enabling*, not redundant.)
- **M-c — delete `:234-237`'s degradation sentence.** "`None` degrades the late scope to 'not recorded at all', exactly as `vb_indirect: None` degrades the early scope" is a false analogy: `vb_indirect: None` keeps `vb_raster` recorded and swaps only the draw call (`vb.rs:1428-1446`, gate at `graph_bridge.rs:3517`). The state is unreachable (`:73`/`:229` mint unconditionally), so this is not a live defect — but the sentence positively licenses a nested `if let Some(late) = …` gate around the late scope, which is the declared-but-unrecorded failure `graph_bridge.rs:3510-3516` and `vb.rs:1069-1076` forbid by name. Replace with the `vb_visible_instance` wording: mandatory, `.expect()`ed under `path_vb_occlusion_split()`. Do **not** add `vb_indirect_late.is_some()` to the predicate — that is a dead conjunct of exactly the kind `scene_types.rs:2809-2812` exists to avoid.
- **M-d — two of D7's five rows name functions that do not exist.** `screen_rect` (`:176`, repeated at `:410`) — the fence is in `pub fn project_aabb`, `hzb.rs:687`, short-circuit at `:704-709`. `MeshLocalBounds::from_aabb` (`:174`) — a repo-wide grep returns only this plan; the real choke point is `MeshLocalBounds::new`, `mesh_geometry_table.rs:236` (doc at `:219-234` is the text the plan paraphrases). Two consumers are also missing from the "complete list" piece 3 inherits: `gpu_upload.rs:235-236` (into `MeshGeometryTable::register` → `new`) and `csm_caster.rs:448` (into `reduce_bounds_into`, fenced at `:348-350` — a *different* block from the `:297-300` one the table cites — and wired in production at `plugins.rs:338`). D7's conclusion survives; the enumeration does not.
- **M-e — mirror the image-side sink assert on the buffer side.** `graph_bridge.rs:3272-3276` guards the image order with `debug_assert_eq!(hzb_pyramid.index() + 1, VB_IMAGE_COUNT)`; there is no `VB_BUFFER_COUNT` anywhere and the `[VkBuffer; 13]`/`[14]` lengths are hand-written literals. The buffer side is *less* guarded on both axes: every buffer sink slot resolves to a valid handle (placeholder-backed at `:4874-4892`), so a mis-indexed buffer barrier names a live wrong buffer with **no VUID**, whereas the image array holds `VkImage::NULL` on unarmed slots. `:318` states the rule in prose and adds no guard. Add `VB_BUFFER_COUNT` + the assert in both `cfg` arms.
- **M-f — close open questions 3 and 5 with anchors** (see F-6, F-7); delete the "which I did not read" hedges.
- **M-g — the three HLSL `_pad` @52 mirrors go stale the moment P2-2 lands.** `vb_batch_cull.comp.hlsl:280-283` documents "a 12-byte `_pad` @52 … so the two mirrors of one host type cannot drift"; `vb_raster.vs.hlsl:140-143` says the same; `vb_geom_fetch.hlsli:44-50` declares the field (and is `#include`d by three more modules, none listed in `:320`). The plan's own cited precedent — `VbBatchDesc::pad → base_instance` — updated **both** sides (`scene_types.rs:468-476` and `vb_batch_cull.comp.hlsl:263`). Decide in the Boundary section: update the comments in P2-2 (comment-only, `.spv` byte-unchanged — the frozen dxc recipes carry no `-Zi`/`-Qembed_debug`, and the `*_spv_sync` tests prove it) or record why they stay stale until piece 3. The victim is piece 3's author, who must read `row.flags` in the file that tells him those bytes are unused.
- **M-h — `:16`'s "zero added cost (structural skip, not a branch)" contradicts `:428`'s "Piece 2 adds cost and removes none".** The design has neither a skip nor a branch: it has an unconditional per-row write plus a fill pass plus a reservation. Replace with the measured shape — "one extra `u32` lane (fill + scatter store + a per-instance probe) and one extra sequential load in the ring build; zero extra passes over the query, zero binds, zero recorded commands, and byte-identical uploaded ring bytes since `flags == 0 == the old _pad[0]`". Reserve "structural skip" for the *recording*, where it is true. In the same edit, correct `:327`'s "The uploaded ring bytes change" — false for every scene that exists when P2-2 commits.
- **M-i — two names in the Data-structures block do not exist:** `VbFramePlan` → **`VbPassPlan`** (defined `graph_bridge.rs:2630`, constructed `:4730`, stored `frame_driver.rs:84`); `build_vb_ring` → **`MeshRenderScratch::sync_vb_instance_ring`** (`mesh_draw.rs:483-498`, so `:291`'s `:485-500` is also misaligned).
- **M-j — G2's red control (`:361`) does not cover its own `vb_late_instances == 0` clause.** Under `:336` every gate carries the corruption that turns it red; that clause's corruption lives in a different gate's bullet (`:353`, `instanceCount = 1` — G1 stays green by construction, G2 reds). Add the cross-reference; nothing new has to be executed.
- **M-k — `:69`'s anchor for the fused `material_ids` precedent points at the line documenting the NON-fused pass.** `mesh_draw.rs:1179-1185` is the comment introducing `gather_material_tex_into`'s second walk — which is what `:289` correctly cites it as. The fused `material_ids` scatter is the primary loop (`mesh_draw.rs:644-650` fill, `:702` store). One of the two citations must move.

---

## (5) MECHANICAL FACTS THE REFUTATIONS ESTABLISHED — DO NOT RE-DERIVE

> **⚠️ Every line number below is as-of-2026-08-06 and must be re-anchored at the moment of use.** This exact trap already cost this campaign a contradiction in piece 1, and it is live in this plan right now: `:150` cites `vb.rs:1958` for the HZB record block and lands inside the *poison* block's comment (B3), and `:443` cites `graph_bridge.rs:3941-4014` for "the HZB slot" and excludes the poison pass entirely. Treat every anchor as a *name plus a hint*, and grep the name.

**F-1 — sync-val IS wired.** `device.rs:2152` = `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`; `ffi.rs:1665`/`:1670` define it; chained as `p_next` head at `:2187-2193` under `enable_validation && VK_EXT_validation_features present` (`:2110-2122`). `golden.ps1:167` → `runner.rs:213`. `crates/boyko_render/tests/sync_validation.rs:47-54` calls it "the AUTHORITATIVE oracle". It degrades **silently** if the extension is absent — liveness is unproven, not disproven.

**F-2 — `Has<T>` does not exist.** One occurrence workspace-wide: a doc comment about resources at `common_conditions.rs:11`. `HasRelation<R>` (`relation/filter.rs:40`) is unrelated. For a bitset id, `Option<&T>` cannot resolve (no column, `data_is_enabled.rs:12-15`), `With<T>` matches zero archetypes (bitset ids are dropped from every signature, `archetype.rs:314-324`, `archetype_master.rs:144-150`), and `Enabled<T>` **drops rows** (incompatible with the lock-step invariant). The only non-filtering bitset read is `IsEnabled<T>`, which reads the BIT and defaults false.

**F-3 — bitset suppresses `Bundle`.** `component.rs:315`: `if hooks.no_bundle || hooks.storage_bitset || hooks.storage_dense { … }`. The insertion verb is `EntityCommands::enable::<T>()` — deferred, chainable, ordinary system (`entity_commands.rs:220`), used at `structural_zero_substep.rs:96` and `bundles_s6_integration.rs:106-107`. **Not** exclusive `&mut EcsMaster`, and **not** `#[require]` (a required ctor writes bytes into a pool a bitset id does not have: `required.rs:49`, `migration_helpers.rs:725-728`).

**F-4 — `vb_indirect: None` does NOT skip a scope.** `vb.rs:1428-1446` keeps `begin_rendering`/`end_rendering` and swaps `cmd_draw_indexed_indirect` → `cmd_draw_indexed`; only the `vb_indirect_upload` *pass* is gated (`graph_bridge.rs:3517`). And `vb_indirect` cannot be `None` on a VB boot: `gpu_scene/mod.rs:1164` is a plain array, `:3763` `.expect()`s the create, `:6393` wires unconditional `Some`. The four `None` literals are all `window_present_gbuffer.rs` fixtures that never resolve VB.

**F-5 — the framegraph's unwritten-resource guard is IMAGE-ONLY and DEBUG-ONLY.** `graph.rs:641` and `:955`: `!is_image || is_write || res_written[ri]`, under `#[cfg(debug_assertions)]`. A buffer read with no producer is waved through in every build.

**F-6 — `golden.ps1` cannot "helpfully" re-bless (OQ5 answered).** The sole PINS.toml write is `Set-Content` at `:259`, inside `if ($Bless)` at `:240`; no other script writes the file. A missing key throws **only under `-Bless`** (`:258`); on the check path an absent or `PENDING` hash exits 2 (`:266`). The pre-fill-and-verify pattern is established at `PINS.toml:355-358` and `:479-483`. Seed **both** legs (`[vb_mesh]` carries `sha256_software` and `sha256_hwrt` as identical literals). What is missing is the *guard* — see M4.

**F-7 — `FrameGraph::with_capacity` is a hint, and the VB declarator has none (OQ3 answered).** One production construction site: `frame_driver.rs:188`, `with_capacity(16, 16, 64)`, on the single `Renderer::frame_graph` field (`:63`); every declarator does `g.reset()` into it (`graph_bridge.rs:3101-3102`). `graph.rs:245-248`: *"exceeding a cap in `reset`-then-declare only regrows the `Vec` (cold), it is never a correctness issue."* Only bound is `u16::MAX` (`:414-417`). The VB path already declares 28 resources non-hwrt (15 images + 13 buffers) / 35 under hwrt.

**F-8 — appending a buffer cannot disturb the "pyramid last" invariant.** `graph_bridge.rs:3272-3276` asserts last among **IMAGES** only; buffers index `res.index() - VB_IMAGE_COUNT` (`:3041`). Append after `vb_visible_instance` in both `cfg` arms.

**F-9 — a bitset read declares NO scheduler access.** `data_is_enabled.rs:170-173`: `init_access` is a no-op, "structural — a bitset id has no `ComponentPool`". So `:299`'s "widens the read set by one component" is false under bitset (it widens it by zero) and true-but-differently under table (`Option<&T>` declares a real read). Meanwhile `mesh_draw.rs:1210-1214` records that the query tuple **is** what the scheduler reads to derive access.

**F-10 — the pins run dev profile; CI runs both.** `golden.ps1:180`/`:193` carry no `--release`; `.github/workflows/ci.yml:52-54` is `matrix: profile: [debug, release]` with `:64` the debug leg. `PINS.toml:759` relies on this: *"The pin run itself exercises every new declare/record parity `debug_assert` (dev-profile build)."* The release **binary** still compiles asserts out (`Cargo.toml` declares only `[profile.bench]`) — which is exactly the branch that makes B3 dangerous.

**F-11 — `GBufferScene` has no `Default`.** `grep "for GBufferScene"` over `crates/` returns nothing; all four `window_present_gbuffer.rs` sites are exhaustive literals with no `..` rest.

**F-12 — a zero-width table component is a supported, pinned case.** `component_pool.rs:3412-3473` (Phase 22 D6) pins `stride == 0` construction, add/`swap_remove`, and `get_typed::<ZstTag>(i).is_some()`.

**F-13 — the conditional-pass unwrap idiom is `expect`, not `debug_assert`.** `vb.rs:1457` + `:1469-1471` (`plan.vb_classify_fill.expect("invariant: mesh_leg && vb_use_classified => …")`), not `:1265`'s unconditional `plan.vb_raster.expect(..)`.

**F-14 — the G2 probe route exists end to end and is outbound.** env → `VgCensusDump::from_env()` (`runner.rs:961-963`) → `c.request(..)` (`:1480-1483`) → `render_gbuffer_frame` parameter (`frame_driver.rs:820`, `:918`) → recorder writes it (`vb.rs:3134-3195`) → host reads after the fence (`runner.rs:2653-2675`). `record_vb(&self, ..)` does not block it: a `&mut` **parameter**, or `cmd_update_buffer` into a host-visible probe from inside the `&self` body (the idiom is at `vb.rs:990-998`), both work.

**F-15 — the plan's D4 record shape is already specified, contra a plausible misreading.** `:100`'s "changes ONE WORD'S producer … Nothing structural" fixes that the late records carry the early records' real `index_count`/`first_index`/`vertex_offset`; only `instance_count` is the inert constant. `:401`'s assert mirrors the early fill at `vb.rs:955-982`. Do not implement an all-zero record.

**F-16 — no perf claim is measurable here, by owner decision.** `OPEN-QUESTIONS.md:269`: *"no occlusion speed-up can be demonstrated on any content in this tree"*; `:277-279` keeps the perf fixture **out** of the density corpus deliberately. OQ2's A/B is a synthetic microbenchmark, never a pin.

---

## (6) WHAT A SIXTH LENS WOULD HAVE FIND

All five lenses read the plan as a *graph/gate* artifact. None read it as an **ECS-lifecycle** artifact. Four concrete gaps, none covered above:

1. **The marker's arrival is DEFERRED, and the plan never says when it becomes visible to the gather.** `EntityCommands::enable::<T>()` (F-3) applies at a command-flush sync point, and under table storage an `insert` triggers an archetype migration with the same deferral. So an entity marked in system *S* is invisible to `gather_mesh_draws` until the flush — the split arms **one frame late**. Combined with M2's missing reset, the observable behaviour is: arms one frame after the marker appears, never disarms. Neither half is stated, and G2's `vb_scopes == 1` on the unmarked scene will be authored against whichever frame the implementer happens to sample. The plan needs a sentence fixing the marker's write site relative to the gather in the schedule, and G2 needs to say which frame it reads.

2. **`:299`'s multithreading paragraph is unsupported under either storage, and the mechanism it names does not exist.** Per F-9 a bitset read declares *no* access, so "widens the read set by one component and introduces no new conflict" describes a mechanism that never runs; a system toggling the bit and the gather reading it are invisible to the scheduler's conflict detection (sound only because toggles route through the command flush). Under table storage the sentence becomes true but for a different reason. Given `mesh_draw.rs:1210-1214` explicitly frames the query tuple as the scheduler's access contract, this paragraph is the one place the plan asserts a *parallel-scheduler* property, and it is wrong in the direction that reads as reassurance.

3. **Nobody asked who CLEARS the new `ScratchColumn<u32>`.** `:213-217` asserts `occlusion.len() == ring.len()`, "every slot written exactly once", and `:393` tests it "after a gather with a hole (a retired mesh)". But the plan never names the reserve/clear step for the column — only the scalar's missing reset was caught (M2). If the column is reserved but not truncated per gather, a shrinking ring leaves stale non-zero tail entries that `sync_vb_instance_ring`'s zip never reads (bounded by `ring.len()`) but that `occlusion_instances` would have already counted. The two defects are separable and only one is currently on the list.

4. **`CsmCasterScratch` wraps a `MeshRenderScratch` and shares `gather_mixed_into`, so the CSM caster gather runs the occlusion fold on its own scratch.** Harmless today (the runner reads the main scratch's count) but entirely unstated in D2's seam table — and it is exactly the kind of "second instance of the datum" that becomes a second predicate the moment someone reads the wrong scratch. State it, or make `occlusion_instances()` structurally unavailable on the caster wrapper.

A sixth lens would also have caught **M-k** (the `:69`/`:289` anchor collision, where the same line range is cited both as the fused precedent and as the non-fused contrast) — a small thing, but it is the same class of error as `:150` pointing into the poison block, and that one is a BLOCKER.

---

# P2-0 RUN — the sync-validation liveness probe, EXECUTED and INCONCLUSIVE

Round 2 made P2-0 the first step and said the meaning of two gates hangs on it: until the probe runs,
whether **G3** (validation) or **G4** (the synthetic declaration pin) is the gate that can see a
missing barrier is undetermined. It has now been run, and the answer is neither yet — **because the
probe as specified cannot produce the hazard it looks for.**

## What was executed

`hzb_build_0`'s declared `image_access(vb_depth, COMPUTE, SHADER_READ, SHADER_READ_ONLY_OPTIMAL,
DEPTH)` was deleted while the dispatch that reads it stayed. `scripts\golden.ps1 -Pin vb_mesh_hzb
-ValidationOn`, against a restored baseline measured in the same session and the same build.

| | occurrences | DISTINCT messages |
|---|---|---|
| restored (×2) | **19**, 19 | 11 |
| probe | **29** | **11 — set-identical to the baseline** |

`Compare-Object` over the normalised message set returns **empty**: the probe produced ten more
*instances* of message classes the baseline already had, and **not one new class**. No
`SYNC-HAZARD-*`. No image-layout error either.

## ⚠️ Why that is not "sync-validation is absent"

**`vb_depth` has SIX declared access sites, not one** (`graph_bridge.rs:3628, 3987, 4029, 4482, 4538,
4660`). `vb_viewt`, `sdf_forward_march`'s mesh arm and the `hzb_dump` copy declare reads of the same
image, and on the `vb_mesh_hzb` configuration at least one of them is armed. So deleting
`hzb_build_0`'s declaration **removes neither the transition nor the dependency** — a sibling
declaration still carries both. The image is still in `SHADER_READ_ONLY_OPTIMAL` when the dispatch
reads it, and the raster's write is still ordered before it.

The probe therefore tested nothing. Its negative result is a property of the fixture, not of the
device, and the critique's proposed wording — "if not, the limitation to write in the commit is *the
extension is absent on this device*" — would have recorded a **false** limitation from it.

The 10 extra occurrences are unexplained and are NOT evidence of the hazard: they are additional
instances of the same `vkCreate*`-time classes, which arrive before any frame is recorded.

## What a decisive probe requires

One of:

1. **Remove EVERY declared reader** of `vb_depth` in the measured configuration, not one — six sites,
   and the count must be re-derived per configuration since some are conditionally armed; or
2. **Probe a resource with exactly one declared reader.** The pyramid qualifies on an unarmed-dump
   frame: `hzb_build_p`'s read of mip `d-1` (`graph_bridge.rs:3997`) is the only declared read of that
   mip, so deleting it leaves a genuine RAW with no sibling to cover it. This is the cheaper probe and
   it is the one to run.

Until one of those runs, **the plan must keep saying the question is open** rather than resolving it
in either direction. What IS established, and matches the round-1 critique's own observation: the
19-message baseline is **entirely `vkCreate*`-time**, so nothing in it demonstrates that
synchronization validation was ever live on a recorded frame.

## P2-0 RESOLVED — probe 2, and the answer is that validation cannot see it

The decisive probe was the second one named above: `hzb_build_p`'s read of mip `d-1`
(`graph_bridge.rs:3997`), which is the **only** declared read of that mip and therefore has no
sibling to cover it. Deleted while the dispatch that reads it stayed.

Pass 0 writes mip 5. Pass 1 reads mip 5. With the declaration gone the graph derives no dependency
between them — and the per-mip state P1-5a shipped is what guarantees that: pass 1's own write of
mips `[6, 10)` is tracked separately and cannot accidentally cover mip 5.

| | validation messages | `SYNC-HAZARD-*` | golden image |
|---|---|---|---|
| baseline (×2, same build) | 19 | — | byte-identical |
| **probe 2 — a REAL missing barrier** | **19** | **none** | **byte-identical** |

**Synchronization validation is NOT live on this machine.** The feature bit is requested in the code,
but the instance chain degrades silently when `VK_EXT_validation_features` is absent, and the entire
19-message baseline is `vkCreate*`-time — nothing in it was ever produced by a recorded frame.

### What this settles, and it settles it in the direction that costs more work

**G4 — the synthetic declaration pin — is the ONLY gate that can see a missing barrier here. G3
cannot.** Round 2 said the meaning of both gates hung on this and refused to pick; the measurement
picks, and it picks the harder one. G4's field-level assertions are therefore not belt-and-braces:
they are the sole coverage, and B2's red control (delete the upload access, watch the count stay 3
while the fields become `TOP_OF_PIPE`/`0`) is the only thing standing between piece 2 and a shipped
missing barrier.

### The sharpest line in the whole measurement

**A genuine missing barrier changed no pixel and emitted no message.** Both gates a reader would
reach for first — the golden pin and the validation leg — returned exactly what they return when
everything is correct. That is the concrete, executed form of the claim this campaign has been
repeating since piece 1 §5: *a golden pin cannot see a redundant or a missing barrier.* It is no
longer an argument. It is a table.
