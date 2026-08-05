The researcher is still running. I have enough in-tree evidence to write the plan; I'll flag where external practice would change a decision.

---

# Architecture: VG R3 piece 2 of 4 — the occlusion-culling CAPABILITY and the VB raster SPLIT, inert

Status: **DESIGN, round 1.** Scope fixed by `docs/OPEN-QUESTIONS.md:167-212` ("RESOLVED 2026-08-03 — decomposed"), piece 2 verbatim: *"The capability and the raster split alone, inert — the second scope drawing nothing, proven byte-identical on the pins."*

## Goal

Land the two structures piece 3's occlusion test needs, with **no occlusion decision anywhere**:

1. **The capability** — "this instance participates in occlusion culling" — as an ECS fact, with the seam to boot-minted GPU objects named and anchored.
2. **The raster split** — `vb_raster` becomes `vb_raster_early` + `vb_raster_late`, with the pyramid build between them, the second scope drawing nothing.

Functional target: on every scene in the tree today, byte-identical output. Performance target: **zero** added cost on any frame with no `OcclusionCulling` entity (structural skip, not a branch); on an armed frame, one extra `vkCmdBeginRendering`/`EndRendering` pair, `N` zero-instance `vkCmdDrawIndexedIndirect`, `3N` binds/pushes, one extra 20 KiB×FIF buffer, and 2 derived barriers.

## Context and constraints

Affected: `boyko_render` (marker, gather lane, instance row), `boyko_app` (gather query, draw-list fold, `GpuSceneBundles` allocation + scene assembly), `boyko_rhi_vulkan` (`declare_vb_graph`, `record_vb`, `VbBarrierSink`).

**Invariants that must survive untouched:**

| invariant | anchor |
|---|---|
| `VB_VISIBLE_INSTANCE_ELEMS == INSTANCE_CAPACITY` (R2d-4 ⊇ / R2d-6 ⊆, both directions) | `crates/boyko_app/src/gpu_scene/mod.rs:258-264` |
| `first_instance: 0` in every record (`drawIndirectFirstInstance` is VK_FALSE) | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:962-976` |
| INVARIANT R2d-REGION-DEFINED (every dereferenced slot written this frame) | `crates/boyko_rhi_vulkan/shaders/vb_batch_cull.comp.hlsl:118-153` |
| INVARIANT R2d-REGION-DISJOINT (bases strictly ascending) | same file, `:101-116` |
| declare/record ORDER parity | `vb.rs:317-320`, `graph_bridge.rs:3084` |
| `hzb_pyramid` is ResId **last** in both `cfg` arms | `graph_bridge.rs:2922-2928`, `debug_assert_eq!` at `:3272-3275` |
| `multiDrawIndirect` is NOT enabled ⇒ `draw_count ∈ {0,1}` | `vb.rs:1422-1424` |
| `robustBufferAccess` is OFF | `gpu_scene/mod.rs:256-257` |

## Key decisions

### D1: The capability is a ZST opt-IN component, `OcclusionCulling`, per ENTITY

**What.** `#[derive(Component)] #[component(storage = "bitset")] pub struct OcclusionCulling;` in a new `crates/boyko_render/src/occlusion_marker.rs`, with `const _: () = assert!(size_of::<OcclusionCulling>() == 0);`.

**Why.** This is verbatim the `ShadowCaster` shape (`crates/boyko_render/src/csm_marker.rs:16-29`) — the tree's own instance of the standing rule (`feedback-capability-structural-not-flag`): presence is the datum, absence is a structural skip. `bitset` storage rather than `ShadowCaster`'s table storage because the consumer is a per-instance *test* over a dense ring, not an archetype filter that materialises rows — `LightEnabled` (`light.rs:251-253`) and `SnapInterpolation` (`snap_interpolation.rs:75-77`) already use it for exactly that shape.

**Opt-IN, not opt-out**, and the direction is the safe one: absence ⇒ never occlusion-culled ⇒ never wrongly vanished. An entity type added in two years by an author who never heard of this feature cannot be silently deleted from the frame. The cost — a game must mark its meshes — is paid by `#[require(...)]`, which this ECS already has (`light.rs:284`).

**Alternatives rejected.**
- *`NoOcclusionCulling` (opt-out)* — the default becomes "cull unless told otherwise", i.e. a new component's failure mode is *invisible geometry*. Rejected on the same asymmetry the tree already states for `MeshLocalBounds::UNKNOWN`: "absence of bounds is not evidence of invisibility" (`mesh_geometry_table.rs:171-173`).
- *A `Resource`-only knob* — cannot express per-object policy (a skybox, a first-person weapon, a UI proxy must never be culled), and would force the "cull everything" default this design rejects.
- *A runtime `bool` field on a component* — forbidden by the rule; `HzbConfig` already argues the same point for itself (`hzb_config.rs:16-22`).

**Trade-off.** A scene must opt in, so the feature is invisible until someone marks something. That is exactly why piece 2's gates need their own marked fixture (see G1) and why the 25 existing pins are blind to it (see §"What a pin cannot claim").

### D2: The seam — a per-frame FOLD at the host gather, producing one per-instance word and one frame-level count. Boot mints unconditionally.

This is the round-3 blocker: *"a capability that is a per-frame ECS fact gating objects minted at boot with no seam named between them."*

**The seam already exists in this tree, on this path, one function above the VB draw-list build**, and it has a name:

```
csm_armed = resolved_csm.csm_mode_word == 1 && casters.batch_count() > 0
                    ^^^ owner Resource knob        ^^^ structural: at least one entity HAS the capability
```
`crates/boyko_app/src/runner.rs:2009`.

The cascade pipeline and its descriptor-set layout are minted at boot, unconditionally. `ShadowCaster` presence is a per-frame ECS fact. They meet at a **per-frame conjunction computed at the host gather, threaded onto the scene struct, and read by declare and record alike** — never at the minting site. Piece 2 reproduces that seam exactly:

| layer | what piece 2 does | precedent |
|---|---|---|
| ECS | `OcclusionCulling` on entities | `ShadowCaster`, `csm_marker.rs:25` |
| gather | a parallel `ScratchColumn<u32>` lane scattered in lock-step with `ring`, **fused into the primary scatter** (zero extra query walks) | `material_ids`, `mesh_draw.rs:1179-1185` |
| per-instance GPU datum | folded into `VbInstanceRow._pad[0]` by `build_vb_ring` | `VbBatchDesc::base_instance` occupying R2c0's reserved `pad`, `scene_types.rs:468-476` |
| frame-level predicate | `GBufferScene::vb_occlusion_instances: u32` (plain `u32`, threaded — this crate cannot depend on `boyko_render`) | `vb_classify_material_count`, `scene_types.rs:2909-2925` |
| the single source read by declare AND record | `GBufferScene::path_vb_occlusion_split()` | `vb_use_classified` (`scene_types.rs:2926-2947`), `vb_viewt` PRE-TAIL slot (`graph_bridge.rs:4021-4026`) |
| boot | `vb_indirect_late` minted **unconditionally**; no pipeline, no layout, no set is added at all | `vb_visible_instance` — "MANDATORY, deliberately NOT part of the R2c0 all-or-nothing arm", `scene_types.rs:2809-2812` |

**Why the P1-4/§9 "arm lives on the TARGETS" shape does NOT transfer here, and this is a deliberate divergence.** §9's shape works because the pyramid *is* a target: `HzbTargets` is `None` when disarmed, so presence and arming are one object. The split allocates **no new target**. Its one new resource, `vb_indirect_late`, belongs on `GpuSceneBundles` beside `vb_indirect`, not on `GBufferTargets`. And piece 1 §7 records the reason hanging it there would be a *bug*: `sync_gbuffer` short-circuits on `(extent, aa_arm)` alone (`targets.rs:7393-7399`), so a per-frame ECS fact carried on the targets **cannot survive a runtime flip at fixed extent** — precisely what a component being inserted mid-run is. The arm therefore belongs on the per-frame scene struct, where `vb_use_classified` already lives, and the resource it would have gated is minted unconditionally instead. **One predicate, computed once per frame, at one site.** There is no second predicate that could disagree with it, which is §9's actual requirement.

**Per-INSTANCE, not per-batch — and the per-batch fold would be UNSOUND.** `ShadowCaster` is folded to per-batch `casts_shadow`, with the documented consequence "a mesh with ANY caster instance casts with ALL its visible instances" (`runner.rs:1944-1952`). For shadow casting that over-approximation is safe. For occlusion it is not: an OR-fold would make an *unmarked* instance eligible for rejection because a sibling sharing its mesh was marked — the one error direction that deletes geometry. Per-instance costs **zero extra bytes and zero extra fetches**: `VbInstanceRow` is 64 B with `_pad: [u32; 3]` at offset 52 (`instance_model.rs:230-246`), in the same 16-byte lane as `mesh_id`, and `vb_batch_cull.comp.hlsl` already loads `gVbInstances[base_instance + j]` for every candidate. The flag arrives in a load the shader already issues.

**Trade-off.** `_pad`'s "unused, always zero" contract (`instance_model.rs:231`) and its unit pin (`:275`) change. That is a documented, deliberate edit — the same one `VbBatchDesc::pad → base_instance` made — not a silent reuse.

### D3: The split is gated on the CAPABILITY ALONE in piece 2; piece 4 AND-s in the config knob

`path_vb_occlusion_split() = path_is_vb() && resolved_render_path.mesh_leg && vb_occlusion_instances > 0`

**Why not also gate on a config knob now.** A split nobody can record is a split nobody can prove inert. Gating on a knob that piece 4 introduces would make every piece-2 gate vacuous by construction — the exact failure mode this campaign has now shipped three times. Gating on the capability makes the split **reachable today**, by a fixture that inserts the marker, so the byte-identity claim is a measurement rather than a hope.

**Why the `mesh_leg` conjunct is load-bearing, not defensive.** On a `VisibilityBuffer × Sdf` frame `vb_raster` is not declared at all (`graph_bridge.rs:3453-3467`); a late scope with no early scope would `LOAD_OP_LOAD` an image nothing wrote this frame. Same conjunct, same reason, as the HZB build's own (`graph_bridge.rs:3961-3969`).

**The predicate over-approximates in the harmless direction.** The count is folded during the *scatter*, so it counts instances that reached the ring; the runner then further skips batches whose mesh is not `Loaded` (`runner.rs:1977-1979`). A frame can therefore arm the split and have zero marked instances in the drawn set. Consequence: an armed empty scope. Never the reverse.

**Trade-off, stated.** Component presence alone changes the recorded pass structure before the feature does anything. A user who marks objects today pays a second empty scope for no benefit. Honest and bounded: no default world and no shipped example inserts the marker, so every golden and every example is untouched *structurally*.

### D4: "Drawing nothing" = a fully recorded late scope whose per-batch indirect records all carry `instanceCount = 0`

This is the charter's central mechanical question. Three candidates:

| candidate | byte-identity story | cost | leaves piece 3 a place for the verdict? |
|---|---|---|---|
| **A** — scope recorded, **zero draw commands** | provable (below) | 2 commands | **NO.** A verdict is a number a compute pass writes and a draw reads. With no draw and no record array there is nowhere to write one; piece 3 would add the buffer, the loop and the cull in one step. |
| **B** — scope recorded, `N` indirect draws against a **dedicated late record array** host-filled with `instanceCount = 0` | provable *and* measurable | 2 + 3N commands, one 20 KiB×FIF buffer | **YES.** Piece 3 changes one word's *producer* (host `0` → cull-written `k`) and the `base_instance` push. Nothing structural. |
| **C** — pass declared, not recorded | violates declare/record parity (`vb.rs:317-320`) | — | NO, and it is not a split. |

**Decision: B.** It is verbatim the discipline this campaign has already run twice and documented as deliberate: `vb_batch_cull.comp.hlsl:166-173` — *"EACH LEVEL SHIPPED INERT ONE RUNG BEFORE IT WAS ARMED, on purpose… Neither was a placeholder"* — `visible = true` at R2c0, `keep = true` at R2d-3, both replaced by real decisions one rung later. `instanceCount = 0` is the third instance of the same pattern, and it is the *conservative* constant (the previous two were the permissive one) because the late scope must draw nothing.

**Every dangerous detail of the late draw loop lands in piece 2, under review, while nothing depends on it**: `first_instance` staying 0, the `i < record_capacity_late` allocation-derived bound, the per-batch push at offset `GBUFFER_PUSH_BASE_INSTANCE_OFFSET`, and the visible-indirection bit.

**The late push carries the indirection bit CLEAR.** With `instanceCount = 0` no invocation exists, so neither the base nor the bit is read — but "harmless because a count is zero" is a weaker invariant than "the bit is clear", and a set bit over a region no pass wrote this frame is verbatim the residue hazard `R2d-REGION-DEFINED` (`vb_batch_cull.comp.hlsl:118-153`) exists to forbid. Piece 3 sets the bit in the same change that writes the region.

**The CLEAR-then-LOAD-then-STORE equivalence, *shown* rather than assumed** (round-1 blocker 7, `docs/VG-R3-HZB-PLAN.md:116-117`):

- Scope 1 (`vb_raster_early`): `LOAD_OP_CLEAR` on `vb_id` (`VB_ID_CLEAR`, `vb.rs:1285`) and `vb_depth` (`VB_DEPTH_CLEAR = 0.0`, `vb.rs:1297`), all `N` draws, `STORE_OP_STORE` on both.
- Scope 2 (`vb_raster_late`): `LOAD_OP_LOAD` on both, **same `renderArea`, same views**, zero fragments, `STORE_OP_STORE`.
- `LOAD_OP_LOAD` yields exactly what scope 1 stored; no draw writes; `STORE_OP_STORE` writes back the loaded contents. Final contents ≡ scope 1's. The argument needs no numerics, so it is not subject to the 8-bit golden floor.
- Two conditions the implementation must not break: both attachments are already in the required layouts at scope 2's start (scope 1 left `vb_id` in `COLOR_ATTACHMENT_OPTIMAL` and `vb_depth` in `DEPTH_ATTACHMENT_OPTIMAL`; the interposed HZB build touches only depth, and the graph derives the round trip — D6), and `renderArea` is identical (`full_area`, the same local).
- Not free on a TBDR (a restore/resolve round trip). This engine's targets are desktop IMR (`CLAUDE.md`: x86_64 Windows/Linux). Stated, not hidden.

### D5: The survivor list is **not** doubled, **not** widened, and **not** duplicated — the R2d-6 collision, answered structurally

**The collision.** The round-3 blocker: *"a fourth route by which the design disarms rung R2d-6 (doubling the survivor list breaks the very const-assert added in R2d-4 to prevent an out-of-bounds device read)"*. The assert is `crates/boyko_app/src/gpu_scene/mod.rs:258-264`, and it is an **equality**, sound in both directions:

- **⊇ (R2d-4)** — `vb_raster.vs.hlsl` selects between `visible_instances[base + id]` and `base + id` with a `? :`, and DXC may lower that to an *eager* load plus `OpSelect`, so the list must admit every index the ring does.
- **⊆ (R2d-6)** — `vb_cull_batch_count_visible_clamp` (`present/passes/vb.rs:194-202`) is the *only* thing bounding the cull's `gVbInstances[base_instance + j]` reads, and it clamps against the **survivor list's** element count (`visible_elems = size/4`, `vb.rs:933-934`). A list of `2N` therefore admits a batch whose ring rows run past the ring's `N`.

`robustBufferAccess` is OFF, so neither direction degrades to a zero read.

**How piece 2 avoids re-creating it — three claims, each structural.**

1. **The late scope gets no survivor list of its own, ever.** Not in piece 2, not in piece 3. The design that needed one was solving a problem that does not exist: it wanted a *third bucket* for HZB rejects. HZB rejects are **discarded**, not stored. The only two sets that need storage are *early-drawn* and *late-candidate*, and round-1 blocker 1 (`VG-R3-HZB-PLAN.md:93-96`) already fixed the late candidate set as `(frustum survivors) \ (early rasterized)`. Both are subsets of one batch's frustum survivors, they are disjoint, and their union is exactly that survivor set. So
   `|early| + |late| ≤ survivors ≤ instance_count`,
   and the existing region `[base_instance, base_instance + instance_count)` holds both — **two-ended, from opposite ends, at the size it already is.** `INSTANCE_CAPACITY` is unchanged, `VB_VISIBLE_INSTANCE_ELEMS` is unchanged, the assert is untouched, and `vb_cull_batch_count_visible_clamp` keeps bounding both directions with the same number.
2. **Piece 2 writes none of that partition.** It ships only the *record array* that a late draw fetches from. The partition is piece 3's, and this plan states the budget so piece 3 inherits a proof rather than a temptation.
3. **The one new allocation is a different buffer in a different const family**, and it gets its own drift guard modelled on `TEX_INSTANCE_CAPACITY` (`gpu_scene/mod.rs:223-238`, explicitly designed so "a future edit to either literal alone is now a BUILD ERROR"):
   ```rust
   const VB_INDIRECT_LATE_RECORDS: usize = 1024;
   const _: () = assert!(
       VB_INDIRECT_LATE_RECORDS == INSTANCE_CAPACITY,
       "the late record array's capacity must track the early one: both draw loops bound \
        themselves by `i < record_capacity` derived from their OWN allocation, and a late \
        array shorter than the early one silently drops the tail batches from the late scope."
   );
   ```
   It touches neither side of the R2d-6 equality.

**The review checkpoint this earns.** `gpu_scene/mod.rs:258-270` must appear in piece 2's diff **only as context**. If the R2d-6 assert or `VB_VISIBLE_INSTANCE_ELEMS` moves by one character, the piece has re-created the collision. I cannot express that as a test; I state it as a named, checkable diff condition.

### D6: The HZB build slot moves BETWEEN the scopes on an armed-split frame — one predicate, both sites

**What.** `declare_vb_graph` and `record_vb` each pick one of two slots for the `hzb_build_*` chain: today's (after the lit producer) when the split is unarmed, and immediately after `vb_raster_early` (before the classify chain) when it is armed.

**Why the move is required, and why it is required *now*.** Today `hzb_build` is declared at `graph_bridge.rs:3941` and recorded at `vb.rs:1958` — **after** the classify chain (`:1457`) and after the `lit` producer. In the target design the late raster must write `vb_id`/`vb_depth` *before* `vb_resolve`/`vb_shade` reads `vb_id`, or the late geometry is never shaded. So the armed order is
`vb_raster_early → hzb_build_* → vb_raster_late → classify → lit`.
That reorder must land somewhere. The only moment it is *provably* neutral is while the late scope draws nothing — which is this piece. Deferring it to piece 3 means shipping a graph reorder in the same step that arms a decision.

**Why one predicate picking a slot, rather than an unconditional move.** Exact precedent, same file, three functions down: the `vb_viewt` PRE-TAIL slot — *"ONE `scene.ssao.is_some()` predicate picks the slot at both declare and record (the accesses are IDENTICAL in both slots; only the position differs)"* (`graph_bridge.rs:4021-4026`). An unarmed frame then derives a barrier stream bit-identical to today's, which is what G4's baseline pin measures.

**The barriers this derives, and they are new.** Between two consecutive passes both writing `vb_depth` at `(FRAG, DEPTH_STENCIL_ATTACHMENT_WRITE, DEPTH_ATTACHMENT_OPTIMAL)`, `transition` fires a WAW — the same auto-chaining the classify block documents (`graph_bridge.rs:3670-3674`). With HZB armed between them the depth chain becomes
`DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (into `hzb_build_0`) → `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` (into `vb_raster_late`),
both content-preserving (neither is a first touch from `UNDEFINED`), plus the `vb_id` WAW. **None of these is visible to the validation leg** — see the gates.

**Effect on the pyramid's cross-frame question (§11/§13), asked explicitly by the charter.** Piece 2 changes **when** the pyramid is written, not **who** writes it and not **who** reads it (still nobody). The `ResSync::undefined()` seed's soundness argument — "a single-buffered image written every frame is safe only while nothing reads it" — is untouched, and piece 3 still inherits it whole.

⚠️ **But one thing does change, and it is a hazard piece 3 inherits from this step.** On an armed-split frame the pyramid is built from the depth as of the *early* scope, while `BOYKO_HZB_DUMP` copies `vb_depth` at *frame end* (`vb.rs:3227`). In piece 2 those are equal because the late scope draws nothing, so G8 still holds — **which also means G8 cannot see the ordering** (see G5's honest limit). The moment piece 3 arms the late draws they diverge, and G8 will be comparing the pyramid against a depth it was not built from. **Piece 3 must move the dump's depth copy between the scopes, or dump both depths.** Recorded here so it is a requirement rather than a discovery.

### D7: The `+INFINITY` fixture vertex — located, and piece 2 does not touch it

The round-3 blocker: *"an `+INFINITY` fixture vertex reaching a second, unfenced host consumer on the shipped VB path."*

**The chain, anchored end to end:**

| stage | anchor | behaviour |
|---|---|---|
| origin | `crates/boyko_render/src/mesh_assets.rs:193-200` | `local_aabb` seeds `[f32::INFINITY; 3]` / `[f32::NEG_INFINITY; 3]` and **returns the seed** for an empty vertex slice (the C0 zero-vertex case). `build_mesh_gpu` only `debug_assert!`s non-empty — `:191-192` calls this "the release-mode backstop". |
| carrier | `crates/boyko_render/src/mesh.rs:179-186` | `MeshGpu::local_min` / `local_max` hold the inverted pair. CPU-only, never uploaded. |
| choke point, GPU side | `mesh_geometry_table.rs:219-230`, `:136` | `MeshLocalBounds::from_aabb` maps any non-finite / inverted box to `UNKNOWN` = `±1e30`, deliberately finite because "a NaN does NOT propagate through `NMin`/`NMax`". |
| host consumer #1, **fenced** | `csm_caster.rs:297-300` | `batch_world_aabb` returns `None` on `mn[i] > mx[i]` — `+INF > -INF` is true on every axis. |
| host consumer #2, **fenced** | `boyko_render/src/hzb.rs:698-709` | the S3 oracle's `screen_rect` short-circuits to `KeepReason::UnknownBounds` on `!(min <= max)`, deliberately spelled that way so a NaN also counts, "at the earliest possible point". |
| the shipped VB path's only raw reader | `boyko_app/src/runner.rs:1993-1997` | feeds `(mesh.local_min, mesh.local_max)` straight into `batch_world_aabb`, i.e. through consumer #1's fence. |

**The blocker was that the whole-feature design added a *third* raw reader of that pair on the shipped VB path without the `!(min <= max)` short-circuit** — round-1 blocker 2 (`VG-R3-HZB-PLAN.md:97-100`) names the same defect class and the same required fix: *"the sentinel must short-circuit to KEEP before any projection, structurally, at the shared entry point"*.

**Piece 2 does not touch it.** Piece 2 adds **no** host consumer of `MeshGpu::local_min`/`local_max`, and no host consumer of any vertex, AABB or projection. Its only new host-side data flow is a `u32` presence flag, which cannot be a float. `local_min`/`local_max` do not appear in piece 2's diff. **The obligation passes intact to piece 3**, which is where a screen-rect is first computed, and where the fence must be the *shared entry point* rather than a per-call-site check.

## Data structures

```rust
// ── crates/boyko_render/src/occlusion_marker.rs (NEW) ────────────────────────────────
/// The structural occlusion-culling capability. Presence = "this entity's instance may be
/// rejected by the HZB occlusion test"; ABSENCE = the instance is always drawn (a structural
/// skip, not a runtime flag). Opt-IN: the error direction of a missing marker is a wasted
/// draw, never vanished geometry.
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
#[component(storage = "bitset")]
pub struct OcclusionCulling;

const _: () = assert!(size_of::<OcclusionCulling>() == 0); // presence IS the datum

// ── crates/boyko_render/src/instance_model.rs (EDIT) ─────────────────────────────────
#[repr(C)]
pub struct VbInstanceRow {
    pub affine: [[f32; 4]; 3],   // 0..48  — byte-identical to InstanceModelCol::rows
    pub mesh_id: u32,            // 48     — geometry-table slot
    pub flags: u32,              // 52     — WAS `_pad[0]`. bit 0 = VB_INST_FLAG_OCCLUSION_CULLING.
                                 //          Same 16-byte lane as mesh_id: the cull's existing
                                 //          gVbInstances load already brings it in. Zero fetches.
    pub _pad: [u32; 2],          // 56..64 — still unused, still always zero
}
// existing offset/size const-asserts unchanged; ADD:
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, flags) == 52);

// ── crates/boyko_render/src/mesh_draw.rs (EDIT) ──────────────────────────────────────
pub struct MeshRenderScratch {
    // ... existing lanes ...
    /// Per-instance capability lane, scattered in LOCK-STEP with `ring`/`mesh_ids`
    /// (`occlusion.len() == ring.len()`, every slot written exactly once). Fused into the
    /// PRIMARY scatter — zero extra query walks, the `material_ids` shape, NOT
    /// `gather_material_tex_into`'s second pass.
    pub occlusion: ScratchColumn<u32>,
    /// Instances whose lane is non-zero, folded during the scatter. The frame-level
    /// structural conjunct: `> 0` ⇔ "the capability is present in this frame's ring".
    occlusion_instances: u32,
}

// ── crates/boyko_app/src/gpu_scene/mod.rs (EDIT) ─────────────────────────────────────
const VB_INDIRECT_LATE_RECORDS: usize = 1024;
const _: () = assert!(VB_INDIRECT_LATE_RECORDS == INSTANCE_CAPACITY, "…");
pub(crate) const VB_INDIRECT_LATE_BYTES: u64 =
    (VB_INDIRECT_LATE_RECORDS as u64) * DRAW_INDEXED_INDIRECT_STRIDE as u64;   // 20 KiB
// per-FIF, DEVICE_LOCAL, usage = INDIRECT_BUFFER | TRANSFER_DST.
// Minted UNCONDITIONALLY (the `vb_visible_instance` rule, scene_types.rs:2809-2812).

// ── crates/boyko_rhi_vulkan/src/present/scene_types.rs (EDIT) ────────────────────────
pub struct GBufferScene<'a> {
    // ...
    /// The per-FIF LATE indirect record array. `Some` on every VB boot; `None` degrades the
    /// late scope to "not recorded at all", exactly as `vb_indirect: None` degrades the early
    /// scope to the direct path it replaced.
    pub vb_indirect_late: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Instances in this frame's ring carrying `OcclusionCulling`. A plain `u32` because this
    /// crate cannot depend on `boyko_render` (the `vb_classify_material_count` boundary,
    /// scene_types.rs:2909-2925).
    pub vb_occlusion_instances: u32,
}

// ── crates/boyko_rhi_vulkan/src/present/graph_bridge.rs (EDIT) ───────────────────────
pub struct VbBarrierSink<'a> {
    pub(crate) images: [VkImage; VB_IMAGE_COUNT],          // UNCHANGED (15 / 21)
    #[cfg(not(feature = "hwrt"))] pub(crate) buffers: [VkBuffer; 14],  // 13 -> 14
    #[cfg(feature = "hwrt")]      pub(crate) buffers: [VkBuffer; 15],  // 14 -> 15
    // `vb_indirect_late` is appended LAST, after `vb_visible_instance`, in BOTH cfg arms —
    // the P1-5 rule, so every existing ResId is byte-unchanged and
    // `buffers[res.index() - VB_IMAGE_COUNT]` keeps indexing what it indexed.
}

pub struct VbFramePlan {
    pub(crate) vb_raster: Option<PassId>,        // RENAMED in prose to "early"; ident kept so the
                                                 // 20+ existing anchors do not churn. See W-1.
    pub(crate) vb_raster_late: Option<PassId>,   // Some iff path_vb_occlusion_split()
    pub(crate) hzb_build: [Option<PassId>; MAX_HZB_PASSES],  // unchanged; SLOT differs
}
```

## Public API

```rust
// boyko_render
pub struct OcclusionCulling;                       // the component
pub const VB_INST_FLAG_OCCLUSION_CULLING: u32 = 1; // bit 0 of VbInstanceRow::flags

impl MeshRenderScratch {
    /// Instances in this frame's ring carrying the capability. The structural conjunct of the
    /// split's arming predicate — the `CsmCasterScratch::batch_count()` shape.
    pub fn occlusion_instances(&self) -> u32;
}

// boyko_rhi_vulkan (pub(crate))
impl GBufferScene<'_> {
    /// THE single source of "this frame records TWO raster scopes", read by
    /// `declare_vb_graph` AND `record_vb` (W1 declare/record parity).
    pub(crate) fn path_vb_occlusion_split(&self) -> bool;
}
```

No `dyn`, no allocation, no new pipeline, no new descriptor-set layout, no new descriptor set, no shader edit.

## Algorithms for critical paths

**The gather fold** (once per frame, in the existing primary scatter, `mesh_draw.rs:661-716`).
Steps: for each query row, `occlusion[slot] = u32::from(has_marker)`; `occlusion_instances += that`.
Complexity O(instances), **fused** into the existing scatter — no second walk (contrast `gather_material_tex_into`, which the tree itself flags as an extra O(N) walk, `:1179-1185`). Cache: one extra sequential `u32` lane beside `mesh_ids`. Branching: `Has<OcclusionCulling>` lowers to a bitset probe; the fold is `+= bool as u32`, branchless. SIMD: the lane is a dense `u32` column; the count is a trivially vectorisable reduction. This lane and `mesh_ids` are the same shape, so if `mesh_ids` ever gets a SIMD scatter this comes free.

**`build_vb_ring`** (`mesh_draw.rs:485-500`): the existing zip over `ring`/`mesh_ids` gains a third lane; `flags = occlusion[i]`. Still one sequential pass, still one store per row, still 64 B written per instance.

**The late scope recording** (once per frame, only when armed): `begin_rendering` → bind pipeline/set/viewport/scissor → pass-wide push → per batch { 2-word push, bind VB, bind IB, `draw_indexed_indirect(late[fi], i*20, 1, 20)` } → `end_rendering`. O(batches). The rebinds are explicit rather than relying on state surviving `vkCmdEndRendering` and an interposed compute dispatch — 4 commands to remove a subtle dependence.

**GPU cost of a zero-`instanceCount` indirect draw.** `instanceCount == 0` produces zero vertex invocations by spec; the command processor still fetches the 20-byte record and sets up the draw. `draw_count = 1` is forced — `multiDrawIndirect` is off (`vb.rs:1422-1424`), so `vkCmdDrawIndexedIndirectCount` with a zero count buffer is not available as an alternative. ⚠️ **This is the one quantitative claim in the plan I have not measured and cannot measure from the source**; the researcher was asked exactly this. If measurement shows a non-trivial per-record fixed cost at the 1024 cap, the mitigation is already available and costs nothing structurally: record the late loop only over `[0, batch_count)` rather than all of `mesh_draw`, using the same hoisted local the early loop uses (`vb.rs:935-940`).

## Multithreading model

Single-threaded with respect to everything piece 2 adds. The ECS gather runs as one system on the scheduler with the access set already declared by `gather_mesh_draws`' `Query` signature; adding `Has<OcclusionCulling>` widens the read set by one component and introduces no new conflict (a ZST bitset read). Command recording is single-threaded into one `VkCommandBuffer` (`record_vb`). No atomics, no shared state, no `Send`/`Sync` change. Device-side: the late scope writes nothing, so there is no GPU race to reason about — the barriers D6 derives exist to be *correct in piece 3*, and G4 pins them now while the claim is cheap.

## Integration

**Modules touched (7 files, 2 new).**

| file | change |
|---|---|
| `crates/boyko_render/src/occlusion_marker.rs` | **NEW** — the marker + const-assert |
| `crates/boyko_render/src/lib.rs` | `pub mod occlusion_marker;` + re-export |
| `crates/boyko_render/src/instance_model.rs` | `_pad[0]` → `flags`; offset const-assert; `from_model_col` signature gains the flag; the `_pad == [0,0,0]` unit pin updated |
| `crates/boyko_render/src/mesh_draw.rs` | the `occlusion` lane, the fold, `build_vb_ring`, **and the query term in BOTH `gather_mesh_draws` variants** (`:1117` non-hwrt, `:1218` hwrt) |
| `crates/boyko_app/src/gpu_scene/mod.rs` | `vb_indirect_late` allocation + const-asserts + `scene()` wiring + `destroy` |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | two `GBufferScene` fields + `path_vb_occlusion_split()` |
| `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | `VbBarrierSink::buffers` +1, `vb_raster_late` declaration, the HZB slot pick, `VbFramePlan` |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | the late scope recording + the HZB record-slot pick |

**⚠️ The two `gather_mesh_draws` variants are a trap.** They are separate `#[cfg]` functions and the doc explicitly requires *"the ring / mesh-id / material-id / pair lanes are byte-identical to the non-hwrt gather (the OFF path never diverges)"* (`mesh_draw.rs:1205-1207`). Both must gain the term identically, and `cargo check --workspace --features hwrt` is not optional for this step.

**⚠️ `VbBarrierSink::buffers` grows in both `cfg` arms** and `vb_indirect_late` must be appended LAST (after `vb_visible_instance`), or `buffers[b.res.index() - VB_IMAGE_COUNT]` (`graph_bridge.rs:3041`) re-indexes every existing buffer. The P1-5 precedent for images (`:2922-2923`, "so every existing ResId is byte-unchanged") applies verbatim.

**No change** to: `vb_batch_cull.comp.hlsl`, `vb_raster.{vs,fs}.hlsl`, any `.spv`, `vb_cull_layout`, `vb_layout0`, `HzbTargets`, `HzbConfig`, `GBufferTargets`, `boyko_render::hzb`, `gpu_scene/mod.rs:258-270`.

## Implementation plan

Each step builds green and commits alone.

- **P2-1 — the marker, read by nothing.** `occlusion_marker.rs` + export + const-assert + the three unit tests. Zero call sites. (The `hzb_config.rs` P1-1 shape: the knob before the machinery.)
- **P2-2 — the gather lane and the per-instance flag, read by nothing.** The `occlusion` column, the fused scatter, the `occlusion_instances` fold, `VbInstanceRow::flags`, `build_vb_ring`, both `gather_mesh_draws` variants. The uploaded ring bytes change; no shader reads them. Gates: lock-step, the flag word, `_pad[1..2]` still zero, 25/25 goldens (**blind** — see below), `--features hwrt` check.
- **P2-3 — the frame predicate and the late buffer, read by nothing.** `vb_occlusion_instances` threaded from the runner; `path_vb_occlusion_split()`; `vb_indirect_late` allocated + wired + destroyed + const-asserted; the `VbBarrierSink` slot appended. Zero readers of the predicate; zero writers of the buffer.
- **P2-4 — the BASELINE barrier-stream pin, on the unmodified declarator.** The P1-5a C1 discipline (`VG-R3-P1-PYRAMID-PLAN.md:517-520`): *"Authoring them after the change would certify the new behaviour."* Pins the VB graph's derived image/buffer barrier stream field-by-field, unsplit, with and without HZB armed. Must be green before P2-5 exists.
- **P2-5 — the split, ATOMICALLY.** `declare_vb_graph`'s `vb_raster_late` + the HZB slot pick; `record_vb`'s late scope + its HZB slot pick; the late record fill (`instanceCount = 0`). Declaration and recording cannot be split without a red intermediate (declare/record parity) — the P1-5a C2 lesson.
- **P2-6 — the gates.** G1..G5 below, including the marked fixture and the new pin.
- **P2-7 — the corruptions, EXECUTED, and the record of what the pins cannot claim.** Each gate shown red on a deliberate defect, results tabulated in the plan document the way P1-3 §8 tabulates its two.

## Metrics and validation — THE GATES

Every gate below carries the corruption that turns it red. *"Can this gate fail?"* is asked first, per the charter.

### ⚠️ What a golden pin CAN and CANNOT claim about this change

**It CAN claim** that the pixels a given scene produces are unchanged.

**It CANNOT claim:**

1. **That the changed code ran.** §13 is the worked example: `hzb_engine_pyramid_gate` reported *0 mismatches over all 349 525 texels* — and the measurement beside it showed **89.3 % of the pyramid was `0.0`, levels 6..9 entirely so**, i.e. *"a pyramid image that a driver zero-filled and NOBODY WROTE would match the oracle at every one of those texels"* (`VG-R3-P1-PYRAMID-PLAN.md:607-651`). The green was nearly meaningless until §14 replaced the coverage argument with a `-1.0` poison.
2. **Anything at all about the split, on the 25 pins as they stand.** No pin, no example and no default world inserts `OcclusionCulling`, so `path_vb_occlusion_split()` is **false on every one of them** and the late scope is never recorded. A green 25/25 on this piece is evidence about the *untouched* path — that P2-2's ring-byte change and P2-3's new field and buffer perturb nothing. **It is not evidence about the split.** This must be said in the commit message, or the next reader will read 25/25 as "the split is proven inert". This is the same class of blindness as §13's, arriving from the other direction: there the scene could not reach the property, here the scene cannot reach the code.
3. **Sub-ULP agreement.** The 8-bit floor is blind between roughly 2⁻²⁰ and 2⁻¹⁶ relative (`reference-golden-fp-resolution`). Irrelevant here — D4's equivalence is exact and non-numeric — but stated so it is not silently assumed away.

### G1 — the split pin equals the unsplit pin *(the load-bearing image gate)*

A new pin `vb_occ_split`: the `[vb_mesh]` scene, verbatim, with `OcclusionCulling` inserted on every mesh entity. Its `sha256_software` must be **the same literal** as `[vb_mesh]`'s. A cross-pin equality a reviewer reads in one glance, and one that cannot be blessed wrong — a blessing that differs is a red.

- **Red control (executed):** force the late scope's `load_op` to `VK_ATTACHMENT_LOAD_OP_CLEAR`. The frame then presents only what the late scope drew — nothing — and the pin goes red while `vb_mesh` stays green. This is the direct proof of D4's CLEAR-then-LOAD equivalence, in the direction that matters.
- **Second control (executed, and expected NOT to fire — record the result either way):** set one late record's `instanceCount = 1`. That re-draws one batch's instance 0 with identical transform and identical depth; under `GREATER` reverse-Z the depth test *fails* on equal depth, so nothing is written and the image is byte-identical. **A gate that cannot see a duplicated draw is a fact worth publishing**, and the corruption that *does* fire is `instanceCount = 1` with the `base_instance` push perturbed by `+1` (a different instance drawn late, at a different place). Both must be run; reporting only the one that fires would be the third vacuous gate.

### G2 — the split RAN *(the gate G1 structurally cannot be)*

G1 is satisfied by not splitting at all. So the recorder's own decision is measured: `record_vb` reports `{ vb_scopes, vb_late_draws, vb_late_instances }` out through the existing probe route (env → host driver → `render_gbuffer_frame` parameter → recorder, which `vb_id_readback` already uses and P1-6 documents at `VG-R3-P1-PYRAMID-PLAN.md:620-624`), and the gate asserts `vb_scopes == 2, vb_late_draws == <batch count>, vb_late_instances == 0` on the marked scene and `vb_scopes == 1` on the unmarked one.

⚠️ **The number must ORIGINATE IN THE RECORDER.** A host that re-derives `scopes` from `vb_occlusion_instances` agrees with itself no matter what the recorder did — §14's own objection to a dump header that re-derives its extent (`:707-709`). This is the difference between a gate and a tautology.

- **Red control:** force `path_vb_occlusion_split()` to `false`. `vb_scopes` reports 1 on the marked scene; the gate reds *while G1 stays green* — which is precisely the pair of outcomes that proves G1 needs G2.
- **Honest limit:** it proves the host *recorded* the scope, not that the GPU *executed* it. For a scope with zero draws there is no observable consequence of execution, so no gate in this repository can close that gap; the nearest independent evidence is G3.

### G3 — validation, armed vs unarmed, message-for-message

The P1-2 / P1-4 leg (*"19 messages armed and 19 unarmed, identical after handle normalisation"*, `:343-347`), run on the marked and unmarked scenes. This is the leg that sees the new scope's *legality*: the `LOAD_OP_LOAD` against the layout the graph left, `vb_indirect_late`'s usage bits, the second `DRAW_INDIRECT` access, the second `begin/endRendering` bracket.

- **Red control:** drop `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT` from `vb_indirect_late`. Validation errors immediately.
- ⚠️ **The limit, and it is severe.** `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION` **does not appear anywhere in this repository** — I grepped for `SYNCHRONIZATION_VALIDATION` / `ValidationFeature` / `sync_validation` and found nothing. So **sync-val is off, and a MISSING BARRIER between the two scopes is invisible to this leg.** It is also invisible to G1 (the late scope writes nothing) and to G2 (a host count). The *only* gate that can see it is G4. Say this in the commit, or a future reader will take a green validation leg as barrier evidence. Independently: the goldens run with `BOYKO_DISABLE_VALIDATION=1` (`goldens/PINS.toml:42`), so the golden legs see no validation at all.

### G4 — the derived barrier stream, split and unsplit *(the only gate that can see a missing barrier)*

Extends P2-4's baseline. On a synthetic VB declaration at a real extent, asserted field-by-field:

- **unsplit, HZB off** — the stream is *bit-identical to P2-4's pre-change baseline*. Nothing about the split leaks into the unarmed path.
- **unsplit, HZB armed** — identical to today's, including `compile_derives_the_hzb_build_chain_at_a_real_extent`'s three barriers at `levels = 10` (`:571-575`).
- **split, HZB off** — exactly two new barriers between the scopes: `vb_id` WAW at `(COLOR_ATTACHMENT_OUTPUT, COLOR_ATTACHMENT_WRITE, COLOR_ATTACHMENT_OPTIMAL)` and `vb_depth` WAW at `(FRAG, DEPTH_STENCIL_ATTACHMENT_WRITE, DEPTH_ATTACHMENT_OPTIMAL)`. No layout change on either.
- **split, HZB armed** — the depth round trip `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL`, neither transition from `UNDEFINED` (a first touch here would *discard* the early scope's depth — round-1 blocker 4's failure, `VG-R3-HZB-PLAN.md:105-108`), plus the `vb_id` WAW, plus the pyramid's own three.

- **Red controls (two, both executed):** (a) delete `vb_raster_late`'s `vb_depth` `image_access` — the round-trip transitions vanish and the late scope would `LOAD_OP_LOAD` an image in `SHADER_READ_ONLY_OPTIMAL`; (b) declare the late pass's `vb_id` access with `VK_IMAGE_LAYOUT_UNDEFINED` — a first touch appears where a preserving transition must be.
- **Authoring order is mandatory:** the baseline (P2-4) precedes the machine (P2-5). P1-5a's C1/C2 exists because *"Authoring them after the change would certify the new behaviour"* (`:517-520`).

### G5 — G8 under the split

`hzb_engine_pyramid_gate` re-run on the marked scene with `HzbConfig::Build`, keeping its `-1.0` poison and all five non-vacuity clauses (`:707-713`).

- **What it proves:** the HZB slot move did not hand `hzb_build_0` a wrong or untransitioned image — the pyramid the engine builds from the *earlier* slot is still bit-exact against `boyko_render::hzb` over the dumped depth.
- ⚠️ **What it structurally CANNOT prove, in piece 2:** that the *ordering* is right. The late scope draws nothing, so early-depth and end-of-frame depth are the same bytes, and a pyramid built at either slot agrees. The ordering's real gate is piece 3's, and piece 3 must first move the dump's depth copy between the scopes (D6's carried-forward hazard). Naming this here is what stops a green G5 from being read as ordering evidence.

### Mandatory unit tests

- `size_of::<OcclusionCulling>() == 0` (const-assert, the `csm_marker.rs:29` shape).
- `occlusion.len() == ring.len()` after a gather with a hole (a retired mesh) — the `mesh_ids` lock-step test's shape (`mesh_draw.rs:1757-1758`).
- `occlusion_instances()` counts exactly the marked instances that reached the ring, across a gather that skips a non-`Loaded` bucket.
- `VbInstanceRow::from_model_col` sets `flags` and leaves `_pad == [0, 0]` (the existing `:268-276` test, updated rather than deleted).
- `path_vb_occlusion_split()` is false on every non-VB path, on `VB × Sdf` (no `mesh_leg`), and at `vb_occlusion_instances == 0`.
- Both `gather_mesh_draws` variants produce byte-identical `ring`/`mesh_ids`/`material_ids`/`occlusion` lanes (the "OFF path never diverges" contract, `:1205-1207`).

### `debug_assert!` invariants

- Every late record has `first_instance == 0` (mirrors `vb.rs:979-982`).
- `plan.vb_raster_late.is_some() == scene.path_vb_occlusion_split()` at both the declare and the record site — declare/record parity, stated as an assert rather than a convention.
- `vb_late_instances == 0` in piece 2, with a message naming piece 3 as the step that removes it. This assert is a **tripwire that must be deleted deliberately**, not a check that quietly stops holding.
- The late scope's `renderArea` equals the early scope's (D4's equivalence depends on it).

## Boundary — what piece 2 does NOT do

The review rounds got value from this list, so it is explicit and it is long.

**No occlusion DECISION of any kind.** No screen rect, no `depth_near`, no `occ`, no `depth_near < occ`, no `KeepReason`, no call into `boyko_render::hzb`'s `screen_rect` / `select_texels` / `occluder_depth` / `occlusion_verdict`.

**No shader edit.** `vb_batch_cull.comp.hlsl`, `vb_raster.vs.hlsl`, `vb_raster.fs.hlsl` and every `.spv` are byte-unchanged. `VbInstanceRow::flags` is written and read by nothing on the device — the R2d-2/R2d-3 rule: *"the descriptor arrives before its consumer so the consumer rung changes only shader code"* (`scene_types.rs:2814-2816`).

**No late cull pass.** No second compute dispatch, no `vb_cull_layout` widening, no new descriptor-set layout, no new descriptor set, no new pipeline. Piece 2 mints **zero** boot objects beyond one buffer.

**No survivor-list change.** `VB_VISIBLE_INSTANCE_ELEMS`, `INSTANCE_CAPACITY`, `vb_cull_batch_count_visible_clamp` and the R2d-6 const-assert (`gpu_scene/mod.rs:258-264`) appear in the diff as context only.

**No two-ended region partition.** The budget proof is stated (D5) so piece 3 inherits it; the packing is piece 3's.

**No `prev_view_proj`, no "visible last frame" bit, no prev-ring work.** Round-1 blocker 6 (`VG-R3-HZB-PLAN.md:112-115`) records that `prev_ring` / `gather_prev_ring_into` / `upload_prev_instance_models` are all `#[cfg(feature = "hwrt")]` with no plugin adding the column. Untouched.

**No arming config.** `HzbConfig` gains no variant — `hzb_config.rs:53-59` fixes it at two, permanently, and names the consumer knob as pieces 3/4's. No `OcclusionConfig` is introduced.

**No `MeshGpu::local_min` / `local_max` consumer.** D7.

**No change to `first_instance`, to `drawIndirectFirstInstance`, or to `multiDrawIndirect`.**

**No perf claim.** None is measurable in this tree (`OPEN-QUESTIONS.md:260-279`); the Sponza fixture is not built. Piece 2 adds cost and removes none.

**No fix for the 19 outstanding validation messages.** Owner-deferred (`OPEN-QUESTIONS.md:157-163`), option (b).

## Open questions

1. **VALUES — the default policy.** Should the engine's own mesh bundle eventually `#[require(OcclusionCulling)]`, making "cull everything" the default and marking the exceptions? Piece 2 ships opt-in (the conservative direction, and reversible). Flipping it changes what a naive user sees on screen, so it is the owner's, not mine. Blocks nothing.
2. **MEASUREMENT — the cost of a zero-`instanceCount` indirect draw** at the 1024-record cap, on this device, with `multiDrawIndirect` off. The researcher was asked; if the answer is "non-trivial fixed cost per record", the mitigation is already specified (bound the late loop by the same hoisted `batch_count` the early loop uses) and costs nothing structurally. Would settle it: a timestamp-query A/B of `N` zero-instance indirect draws against zero draws, in the same sitting, against the `VG-DECIDABILITY-FLOOR` protocol — and per that document's own finding (6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol) a delta under ~15 % is not defensible.
3. **`vb_indirect_late`'s ResId and the `with_capacity` budget.** Appending it grows `VbBarrierSink::buffers` to 14/15 and the VB graph's resource count by one. `frame_driver.rs:188` is `with_capacity(16, 16, 64)` and the deferred declarator already mints 33 resources, so `state` already regrows on frame 1 (`VG-R3-P1-PYRAMID-PLAN.md:504-506`) — this is a `Vec` capacity hint, not a cap, so I expect no blocker. **I have not read the VB declarator's own `with_capacity` call and cannot confirm it from what I read.** Settled by one read at P2-3.
4. **Naming.** I keep the `vb_raster` identifier for the early pass rather than renaming to `vb_raster_early`, because 20+ anchors across four files and several docs cite it by name and the R9 plan's §0 already rejected a rename on exactly this ground (*"OpSource/.spv churn on a frozen, pinned blob"*). Prose says "early". If the critic prefers the rename, it is mechanical but it is churn in a step whose whole claim is that nothing changed.
5. **The `vb_occ_split` pin's blessing.** The pin's `sha256_software` must equal `[vb_mesh]`'s *literal*, which means it is blessed by copy rather than by run. That is deliberate — a pin blessed from its own run would certify whatever it produced — but it does mean the harness must not "helpfully" re-bless it. Needs confirming against `golden.ps1`'s blessing path, which I did not read.
6. **Research pending.** The two-phase-structure survey (does UE5 Nanite / niagara / Bevy 0.16 give the late pass a separate indirect buffer or reuse one; how the disarmed case avoids the second pass; and whether anyone reports measured cost for zero-count indirect draws) has not returned. If it shows a materially different late-pass structure, D4's *choice of B over A* is the decision that moves — the seam (D2), the collision answer (D5) and the slot move (D6) are all argued from in-tree invariants and do not depend on it.

---

**Files an implementer starts from:** `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\present\passes\vb.rs` (the raster loop, `:1264-1449`; the HZB record block, `:1958-2130`), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\present\graph_bridge.rs` (`:3608-3664` the raster declaration, `:3941-4014` the HZB slot, `:2884-2928` the sink), `D:\claude\BoykoEngine\crates\boyko_app\src\gpu_scene\mod.rs` (`:145-270` the capacity family and the R2d-6 assert), `D:\claude\BoykoEngine\crates\boyko_render\src\mesh_draw.rs` (`:1117-1300` the two gathers), `D:\claude\BoykoEngine\crates\boyko_render\src\instance_model.rs` (`:221-260` `VbInstanceRow`), `D:\claude\BoykoEngine\crates\boyko_render\src\csm_marker.rs` (the marker template), `D:\claude\BoykoEngine\crates\boyko_app\src\runner.rs` (`:1944-2009` the fold and the `csm_armed` seam).
agentId: a5c3d827cc02ccd0b (use SendMessage with to: 'a5c3d827cc02ccd0b', summary: '<5-10 word recap>' to continue this agent)
<usage>subagent_tokens: 247299
tool_uses: 54
duration_ms: 960242</usage>

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
