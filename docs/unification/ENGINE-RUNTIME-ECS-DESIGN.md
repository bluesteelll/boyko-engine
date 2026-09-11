# Engine runtime as ECS - design

- **Date:** 2026-09-11
- **Revision:** rev 2 after one critique pass
- **Research (surveys and the owner's orders):** [ENGINE-RUNTIME-ECS-RESEARCH.md](ENGINE-RUNTIME-ECS-RESEARCH.md)
- **Sibling physics study:** [../physics/PHYSICS-ECS-UNIFICATION-DESIGN.md](../physics/PHYSICS-ECS-UNIFICATION-DESIGN.md) (owns physics and kernel features K1-K7; this design consumes them and files proposed changes to them as inputs, section 13). The file exists in M at the time of writing; its later revisions (a pass-2 critique and a rev 3 patch appended there) are not reflected here.
- **Status:** design only. No gate was run and no timing was taken. The owner questions in section 14 (Q1-Q5) are open.

## Code trees

The newest code of each subsystem lives in a different worktree, so each subsystem was read from its own tree.

| Letter | Path | Branch | Commit | Read for |
|---|---|---|---|---|
| J | `D:/wt/joltab` | `merge/ke16-into-ecsnative` | `d11962a9` | the kernel, ECS-native physics, the KE16 thread pool, and the newest render / RHI / app / input / scene / log / diag |
| U | `D:/wt/ui` | `feat/ui-advanced` | `615cda8f` | boyko_ui and the render-side UI (16 commits and ~11.4k lines of boyko_ui that J does not have) |
| R | `D:/wt/reflect` | `feat/reflection` | `0e0b4c68` | boyko_reflect (20 commits not in J) |
| M | `D:/claude/BoykoEngine` | `feat/multi-paradigm-render` | `dc35fae8` (HEAD when these files were written) | docs committed there: `docs/animation/`, `docs/physics/ADVANCED-PHYSICS-*`, `docs/render/TRANSPARENCY-*`, `docs/render/REFLECTIONS-*`; and the untracked `docs/memory/ALLOCATOR-*` |

## Provenance tags

- **[S]** source read.
- **[D]** official documentation.
- **[B]** blog post or talk (recorded, not relied on).
- Anything that could not be verified is labelled "not verified", never paraphrased from memory.
- In-tree facts are cited as `tree:path:line` with the quoted line; the tree letter is one of J, U, R, M below.

## How this file is laid out

1. The rev 2 design, verbatim.
2. Critique log - pass 1: the critic's verdict and every finding verbatim, each followed by the action the architect took, quoted from rev 2's change table.
3. The critic's preserve list, verbatim.

---

# Architecture: one engine, one world. The ECS form of every non-physics runtime subsystem (Rev 2)

**Output format.** The role file asks for a patch from revision 2 onward; the orchestrator asked for the complete text. The complete text follows. The change table at the end quotes each removed rev-1 sentence verbatim, so any silent drop would show up there.

**Provenance.**
- **Trees.** Every citation names the tree it comes from.
  - J = `D:/wt/joltab` @ `d11962a9`: the kernel, render, RHI, app, input, scene, log and diag.
  - U = `D:/wt/ui` @ `615cda8f`: boyko_ui and the render-side UI.
  - R = `D:/wt/reflect`.
  - M = `D:/claude/BoykoEngine`: docs, and the physics study `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md` Rev 2. I did not check whether that file is committed.
- **Re-read for rev 2:**
  - physics §3 D14, §8 K-table, §12 rungs;
  - U `interaction/focus.rs`, `interaction/action.rs`, `interaction/plugin.rs`, `components.rs`, `shaders/ui_rect.fs.hlsl`, `boyko_fontbake/src/atlas.rs`, `animation.rs`;
  - J `mesh_draw.rs`, `relationship/collection.rs`, `hierarchy/mod.rs`, `entity_master.rs`, `entity.rs`, `scratch/scratch_column.rs`, `scratch/views.rs`, `params/res.rs`, `bindless.rs`, `boyko_serialize/src/save.rs`.
- **What I did.** Nothing was run and nothing was edited.
- **Research.** There was no `Agent` tool, so the reference lens stands in for the mandatory researcher pass.
- **Tags.**
  - `arith.` means worked out from field types, not checked with `size_of`.
  - `not verified` means the claim was not checked against the tree.
- **Ledger.** It was read, not waited on. Its UI rows are 16 commits stale against U.

---

## 0. Corrections and new findings (read these first)

| ID | Premise in a lens or in rev 1 | What the tree says | Consequence |
|---|---|---|---|
| X-1 | UI lens: "same-frame delivery not verified" for kernel events | `J:crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs:78-80` "The reader observes events that were sent in a previous frame and made visible by the most recent `update_events` call (next-frame visibility, Phase 6 Model B)." | Kernel events arrive next frame. Intra-frame UI messages go to triggers (X-7) or to fixed resource lanes (ED9). |
| X-2 | UI KF-B "no entity-yielding query" | `J:crates/boyko_ecs/src/ecs/core/system/params/entities.rs:1-2` "`Entities<'w>` — read-only `SystemParam` resolving an [`EntityId`] to a / live [`Entity`] handle." Glob for `params/entities.rs` on U returns nothing. | KF-B is not a kernel feature. It arrives with the U→J merge. |
| X-3 | Render and host lenses treat the gathers as ordinary Main systems | `J:crates/boyko_render/src/mesh_draw.rs:1255` `mesh_assets: NonSendRes<Assets<MeshGpu>>,`. Only POD is read: `:1291-1292` `let m = mesh_assets.try_get(MeshHandle(mesh_id))?;` / `Some((m.index_count, m.index_type))`. A NonSend param makes a system exclusive: `J:.../system/system_meta.rs:145-149` "Set by `NonSendRes` / `NonSendResMut::init_access` … resolves the system to `SystemKind::CpuExclusive`". The same param type appears on `asset_refcount.rs:92`, `:402`, and `csm_caster.rs:192`, `:453`. A sixth user, `sync_vb_instance_ring_system` (`mesh_draw.rs:1170` `mesh_assets: NonSendRes<Assets<MeshGpu>>,`), is a per-frame `run_system` today, and HO2 schedules it. | Six per-frame Main systems end up CpuExclusive only because POD mesh meta shares a NonSend table with device handles. This drives ED4. |
| X-4 | UI KF-G "exclusive-structural-verbs" | `J:.../commands/remove_command.rs:88` `world.dense_remove_and_fire(entity, C::component_id());` | KF-G is dropped. |
| X-5 | M playground: the edge between ui and render is test-only | `U:crates/boyko_render/Cargo.toml:82` `boyko-ui = { path = "../boyko_ui" }`. On J it is still dev-only (`J:crates/boyko_render/Cargo.toml:110-113` "Acyclic + TEST-ONLY"). | D31 (render → ui) is decided. |
| X-6 | UI lens: layout "exclusive stays" | `J:.../iters/query/query.rs:986` `pub fn get_mut(&mut self, entity: Entity) -> Option<D::Item<'_>> {`; `:896` `pub fn get(&self, entity: Entity)` | Layout can be a function system (ED8). |
| X-7 | UI lens: hand-rolled one-frame messages | `J:.../ecs_master/observer_api.rs:449` `pub fn trigger<E: Trigger>(&mut self, target: Entity, event: E) {`; `:444` "`E::PROPAGATION` ([`Up`](PropagationMode::Up) bubble or" | UI bubbling already has a kernel home. |
| X-8 | Services KF4: Children removal is "O(n)" | `J:.../relationship/collection.rs:100-101` `if let Some(idx) = self.as_slice().iter().position(\|&c\| c == e) {` / `self.swap_remove(idx);` | An ordered remove stays in the same complexity class (ED10). |
| X-9 | Tick cost on small rows | `J:crates/boyko_ecs/src/ecs/constants.rs:219` "stagger): `[pad \| data \| added_ticks \| changed_ticks]`, every *sub-region*" | A data-only query streams data bytes only. |
| X-10 | Host lens: `WindowHost` must stay outside the World | `U:crates/boyko_render/src/ui/upload.rs:782` "proof: the swapchain `Renderer` is not yet an ECS resource, so"; `J:.../system/dispatcher_token.rs:22` "the token is minted ONLY by the scheduler on the dispatcher-solo" | No kernel feature is needed for residency. EK11 is needed only for drop order. |
| **X-11 (new)** | U's tween reap: "This kernel does not recycle [`EntityId`]" (`U:crates/boyko_ui/src/animation.rs:454`) | J's allocator pops a recycled id first: `J:crates/boyko_ecs/src/ecs/core/entity/entity_master.rs:124-125` `pub(crate) fn allocate_entity(&mut self) -> Entity {` / `if let Some(id) = self.free_entity_ids.pop() {`. `:352` "Deallocates an entity, bumps its generation, and recycles its id." U's measurement (no recycling) was not re-run on J. | The reap's load-bearing fact is not guaranteed on J. UI9 therefore moves into the merge prerequisite (UI0). EK3 records carry the generation (W6). |
| **X-12 (new)** | Rev 1 EK15: a new "relation target storage" feature | Collections are already a pluggable trait with two impls: `J:.../relationship/collection.rs:80` `impl RelationshipSourceCollection for Vec<Entity> {` and `:158` `impl RelationshipSourceCollection for Exclusive {`. `Children` chooses its impl by an associated type: `J:.../hierarchy/mod.rs:197` `type Collection = Vec<Entity>;` | The ordered remove, the count-only target and K7-backed storage become three new impls of an existing trait (EK15a/b/c). Only the third needs K7. |
| **X-13 (new)** | Rev 1 B-4/UI7 assumed a new atlas binding | The UI record already carries a bindless slot: `U:crates/boyko_render/shaders/ui_rect.fs.hlsl:37` "bits 20..31 the bindless sprite slot"; `:66` `[[vk::binding(0, 1)]] Texture2D g_sprites[] : register(t0, space1);`. The atlas is a separate single binding: `:49` `[[vk::binding(1, 0)]] Texture2D    g_atlas         : register(t1);`. The bake writes one distance range, `U:crates/boyko_fontbake/src/atlas.rs:398` `distance_range_texels: DISTANCE_RANGE_TEXELS,`, but a loaded atlas reads it from the file: `:583` `distance_range_texels: r.f32()?,` | Per-font atlases can ride the existing bindless array with the 80 B stride unchanged, through an explicit eDSL rung (UI6s, W7). |
| **X-14 (new)** | Rev 1: EK1 is a rung of this design | Physics owns it: `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:790` "\| U1 \| K1 cohorts; migrate `scratch_ids.rs` and render lanes \|" | EK1's core is consumed from physics U1. This design adds only the `Default` impl. |
| **X-15 (new)** | Rev 1 ED6: one cursor pick per chunk, no per-row branch | The gather re-walks the query index-aligned: `J:crates/boyko_render/src/mesh_draw.rs:1315-1317` "Textured-PBR rung T6c: the parallel TEXTURED material-payload scatter — a SECOND, / index-aligned pass over the SAME query … / call-site shape), re-using the offsets `gather_mixed_into` just fixed". Also `:1225-1226` (prev ring). The VB rows read the lanes by position: `:532-533` `for ((model, &mesh_id), &flags) in` / `ring_slice.iter().zip(mesh_ids_slice.iter()).zip(inst_flags_slice.iter())` | Every re-walk must replay the partition (W4, ED6). The primary-lane readers inherit it. |
| **X-16 (new)** | Rev 1 ED8: focus scans `With<Focusable>` with a per-root order | Candidates are Interaction nodes: `U:crates/boyko_ui/src/interaction/focus.rs:224-225` `if let Some(rect) = world.get_component::<ComputedRect>(node).copied()` / `&& world.get_component::<Interaction>(node).is_some()`. The order is global: `:331` `(a.stack_index, a.paint_seq, a.entity.id().0) > (b.stack_index, b.paint_seq, b.entity.id().0)`; roots are walked in one sequence, `:214` `scratch.query_buf.sort_unstable_by_key(\|e\| e.id().0);`. The clip replaces rather than intersects: `:222` `let effective_clip = own_clip.or(inherited_clip);`. `ComputedClip` is author-owned: `U:crates/boyko_ui/src/components.rs:186-188` "Clip rectangle for overflow. COLD, OPT-IN. / … / AUTHOR-OWNED in P1 (not computed)". `StackIndex` is `:184` `pub struct StackIndex(pub u32);` | ED8 is redone around a global paint key and a replace-semantics effective clip (W1). |
| **X-17 (new)** | Rev 1 EK13: a new LIFO frame feature | The LIFO primitive exists: `J:crates/boyko_ecs/src/ecs/core/component/scratch/views.rs:208` `pub fn truncate(&mut self, new_len: usize) {`, with base stability at `:135` "needed (the base never moves — address-stable)". | EK13 is deleted. Mark/rewind is `len()` / `truncate(mark)` on a `Local<ScratchColumn<T>>` (W3). |

---

## 1. Goal

**Functional.**
- Every non-physics runtime datum has exactly one authoritative home, in one ECS form from the closed vocabulary. Wherever a per-entity form fits, the datum is owned by the entity it describes.
- Every runtime loop is an ECS system on the engine scheduler. Only the OS message pump stays host code. The profiler fold and the frame driver stay kernel-internal and outside the schedule (§6).
- The physics study owns physics and its kernel features K1–K7. This design meets it at transforms, colliders on render meshes, events, the SDF field, and shared kernel features. It uses physics's names and files proposed changes as inputs (§13), never as edits.

**Performance.**
- Unification is justified by the owner's orders, not by speed: the allocation A/B was null. So every rung must be argued not to regress the hot loop it touches.
- Where the ECS form removes work, the gain is claimed with arithmetic and gated by a microbench:
  - six exclusive gathers become Send;
  - one shadow-caster gather is deleted;
  - the per-frame UI DFS becomes a linear scan.

## 2. Context, constraints, target metrics

**Crates affected:**
- `boyko_ecs` (EK2–EK21, excluding EK13 and EK14);
- `boyko_app`: the runner shrinks to the pump;
- `boyko_render`, including the render feature RF1 (FIF mirrors);
- `boyko_rhi_vulkan`;
- `boyko_ui`, `boyko_input`, `boyko_scene`, `boyko_log`, `boyko_diag`;
- `boyko_reflect`: seam only.

**Invariants kept:**
- Principle 0.
- The capability-state model (`J:docs/CAPABILITY-STATE-MODEL.md:3` "owner-approved convention").
- Shaders are eDSL-generated.
- Dense storage is CPU-only (`J:docs/DENSE-COMPONENTS-PLAN.md:6`).
- GPU-resident archetypes are GPU-pure (PHASE5:283).
- The FIF count and frame latency are unchanged.
- Every physics invariant in the physics study's §2.

**Targets.** Each is a gate threshold.

| Metric | Today | Target |
|---|---|---|
| Host frame-loop steps that write the World outside a system (G-LOOP) | 7: F3, F7, F8, F10, F11, F12, F15 (host lens §1) | 0 |
| Per-frame `run_system` calls in the frame loop | 1 (`J:crates/boyko_app/src/runner.rs:1619` `app.world_mut().run_system(boyko_render::sync_vb_instance_ring_system);`) | 0 |
| Per-frame Main systems that are CpuExclusive only because of a NonSend POD read | 5 scheduled today, plus 1 that HO2 schedules (X-3) | 0 |
| Exclusive UI systems | 13 (U, UI lens §4) | 1 (`ui_bind_apply`, ED8) |
| Copies of one datum (window size 3, SDF scene 3, render path 2, frame counter 2, interners 2, type registries 2, UI root set 5) | 19 copies of 7 data | 7 |
| Steady-frame heap allocations in input, UI, render-prep and host, excluding the scheduler and pool | K0 baseline; code-derived ≥ 1 × 24 KB on VB frames (`J:.../filtered_access_set.rs:146` `bit_owners: Box::new([""; OWNERSHIP_SLOT_COUNT]),`) | 0 |
| Focus hit-test per frame | whole-tree DFS, 4–6 random probes per node (`U:.../focus.rs:221-234`) | one sequential scan over `Interaction` rows at 40 B/row (arith.: `ComputedRect` 16 + `ComputedPaintKey` 8 + `ComputedEffectiveClip` 16); ≤ 5 µs at 2,000 rows (measured, Q5) |
| Shadow-caster CPU gather | a second count/prefix/scatter plus a ring (`J:crates/boyko_render/src/csm_caster.rs:203`) | 0 extra passes |
| Render-schedule dispatch overhead | n/a | ≤ 2 µs/frame at ~20 dispatcher-solo systems (measured, Q5) |
| Engine-owned runtime structures outside an ECS form (G-FORM rows, non-physics) | K0 baseline | only shrinks |
| Resident commit added by new untracked kernel columns | K0 baseline | recorded per rung, not gated. The floor is 64 KiB of data region per untracked column (`J:.../scratch/scratch_column.rs:70-74`: a tracked pool commits "TWO tick sub-regions alongside the data at 64 KiB granularity"; "Untracked reserves those sub-regions and never commits them, so the cost is the data region alone"). Owned by `POOL-SUBGRANULAR-PACKING-PLAN.md:48` (as cited at `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:834`). |

---

## 3. Key decisions

### ED1. One world. No extracted render world. (unchanged)

**What.**
- Render reads the simulation world's columns directly.
- Per-frame render products live in resource-columns on kernel ScratchColumns.
- The GPU copy lives in driver-owned FIF memory, written by upload systems.

**Why.** See §7. In short:
- A second world adds at least one extra 48 B/instance copy plus an entity map. Bevy measured −25% from pair hashing (#17078 [S]).
- Its only benefit is pipelining sim(N+1) ∥ render(N): 7–29% in Bevy (#6503 [S], 5600x/RX 6600). That gain depends on a record share that is unmeasured here.
- It is a second data system, which order (3) forbids.

**Alternatives.**
- Bevy SubApp: rejected for the reasons above.
- Per-entity device-resident instance columns: structurally unavailable, because instance archetypes carry CPU columns and GPU archetypes must be GPU-pure (PHASE5:283).

**Trade-off.** No CPU pipelining. §7 gives the seam for later.

### ED2. The frame is three core schedules, `Fixed → Main → Render`. The host keeps only the OS pump.

**What.**
- `CoreSchedule::Render` (EK10) holds only dispatcher-solo systems: acquire, retire, asset upload, grows, FIF mirrors, record/submit/present, publish.
- The runner becomes `loop { pump → app.update() }`.

**Why.**
- Seven host steps mutate the World outside any system (`J:crates/boyko_app/src/host.rs:3` "[`WindowHost`] owns everything the OS/present side needs OUTSIDE the World:").
- Features are hand-ordered across 2,000 runner lines (`runner.rs:1012-3283`).
- A separate slot rather than a Main tail set, because:
  - (a) one driver branch skips it when headless;
  - (b) it is the seam for a later overlap (§7);
  - (c) it keeps dispatcher-solo nodes out of Main's worker waves.
- `J:.../app/app.rs:60-62` "New top-level slots are an engine change by design".

**Alternatives.**
- A Main tail set with `run_if`: rejected for (a)–(c).
- One exclusive `render_frame` mega-system: rejected, because plugins could not add upload or record steps.

**Trade-off (rewritten, per the critic's preserve item).**
- `FrameWriteToken` moves from a stack local into `NonSendResMut<FrameInFlight>` as an `Option`. It stays a witness. It is lifetime-free (`J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:1043-1044` `pub struct FrameWriteToken {` / `slot: usize,`), so it can be stored.
- Every mirror borrows `&FrameWriteToken` from the `Option`: no token, no write, and that is still a type-level proof. R5 consumes it by value (`Option::take`).
- What is lost is only the static guarantee that a token cannot outlive its frame. It is replaced by two debug_asserts: R0 finds `None` on entry, and teardown finds `None`.
- Order "uploads after the fence wait" is carried by the edge `after(render_acquire)` (G-GRAPH) plus `run_if(frame_acquired)`.

### ED3. One FIF-mirror protocol replaces the 22 hand-written upload calls. It is a render feature (RF1), not a kernel feature.

**What.**
- `upload_mirror::<M: FifMirror>` in `boyko_render`, monomorphised per mirror, no `dyn`.
- It copies `M::Source`'s bytes into slot `s` if and only if `!state.valid(s)` or the source changed after `state.uploaded[s]`. Resource ticks come from EK5.
- **The per-slot state is owned by the target:** `FifMirrorState` sits inside `M::Target`, the NonSend resource that owns the ring buffers. It is reached through `FifMirror::state`.
- **Invalidation:** every function that replaces a target buffer calls `state.invalidate(slot)` in the same function that retires the old buffer. That covers R3 `render_grow`, device re-create and swapchain re-create. A fresh ring is therefore always written, even when the source tick is unchanged (O1).
- **Scope.**
  - Resource-sourced mirrors: view, CSM, atlas, ray, denoise, temporal, TAA, motion-cam, lights, particle effects, instance rings, UI pack, SDF edits.
  - Asset-sourced device lanes (materials, mesh meta) do not use `FifMirror`. They upload per row on `Changed<T>` of the asset's dense value (Q1a), or through the `Assets<T>` counters (Q1b).

**Why.**
- Three copies of one protocol: `J:crates/boyko_app/src/host.rs:138` `pub(crate) light_uploaded_gen: [u64; FRAMES_IN_FLIGHT],`, `host.rs:147`, and `J:crates/boyko_render/src/material_table.rs:97` `rebind_pending: [bool; FRAMES_IN_FLIGHT],`.
- CSM/atlas UBOs are rewritten every frame (`runner.rs:1804` "UNCONDITIONAL every").
- **Why render and not kernel:** the kernel must stay graphics-pure (`J:.../system/dispatcher_token.rs:33` "names NO graphics type", per the render lens). The protocol's inputs are a kernel resource tick (EK5) and a render-owned buffer.

**Trade-off.** Mirrors whose source changes every frame pay one tick compare per frame.

### ED4. Every asset table splits into a Send meta lane and a NonSend device lane.

**What.**
- `index_count`, `index_type`, `geometry_slot`, local bounds and refcount move to a Send lane (`MeshMeta`, §5).
- `BoundBuffer` handles stay NonSend, indexed by the same slot.
- `Assets<Material>` gets the same split. Its GPU copy is a NonSend resource-column indexed by the material's slot (see render row 12).

**Why.** X-3: **six** per-frame systems become Send. These are `gather_mesh_draws`, `gather_shadow_casters`, `reduce_caster_bounds`, the two `asset_refcount` systems, and `sync_vb_instance_ring_system` (O5).

**Alternative rejected.** Resolving mesh meta in Render: the gather needs `index_count` in Main to size its buckets.

**Trade-off.** Two lanes per asset kind. The meta lane is written at install time.

**This decision holds under either answer to Q1.**

### ED5. Assets are entities. Recommended; put to the owner as Q1 because it reverses S1.

**What.**
- **Identity.** Asset identity is an `Entity`. `Handle<T>` stays an 8 B `{index u32, generation u32}` view of it.
- **Value and release.** The asset value is a column of a **K3 dense group with `RELEASE = Deferred`** on the asset entity (physics K3/D14, `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:478` "`RELEASE = Immediate | Deferred` (D14);"). Its slot never moves, so GPU tables index by slot.
  - A retired asset's slot enters `dying` with its bytes intact.
  - It returns to `free` only through the K6 release, called with a **horizon tick** (K6′, ED16). For assets the horizon is the gather tick of the last fence-completed frame.
  - This replaces rev 1's `Retiring{epoch}` component (W9).
- **Load state** is component presence: no value = loading; `AssetFailed`; `Pinned`.
- **Refcount** is a count-only relation, instance → asset (EK15b). A `CountOnly` target cannot enumerate its sources, so despawning an asset with count > 0 is converted into "retire when the count reaches 0". The kernel's target-despawn cascade is statically disabled for count-only targets (EK15b).
- **Change tracking.** `dirty_gen` / `install_epoch` / `free_epoch` become kernel `Changed`/`Added` on the dense value (EK6) plus the release visitor.
- **Slot 0 (O4).** Each GPU-indexed asset kind spawns its **slot-0 sentinel** before any user asset, in `StartupSet::Resources`: the pinned default material and the null texture. This preserves "the slot-0-never-retires invariant" (`J:crates/boyko_render/src/mesh_draw.rs:1279-1280`) and bindless's reserved slot 0 (`J:crates/boyko_render/src/bindless.rs:551` `assert_eq!(a.register(), None, "capacity 5 has exactly 4 real slots");`).
- **Save, prefab, clear (O3).**
  - `save_world` walks every archetype (`J:crates/boyko_serialize/src/save.rs:170` `for archetype in world.archetype_master().iter_archetypes() {`). Asset value columns are therefore classified **Ignore**.
  - An asset entity is saved as its `AssetPath` only.
  - Instance → asset relations are entity-mapped by `AssetPath` at load: resolve through the path index, or spawn a loading asset entity.
  - Prefab capture never captures asset entities; they are referenced, not owned.
  - World-clear utilities skip entities carrying `AssetRoot` unless asked.

**Why.** `Assets<T>` duplicates four kernel mechanisms:
- the entity allocator (`J:.../asset/handle.rs:44-49` `pub struct Handle<T> {` … `index: u32,` … `generation: u32,`);
- dense storage;
- change detection (`J:.../asset/assets.rs:209-211` `dirty_gen: u64,` / `free_epoch: u64,` / `install_epoch: u64,`);
- lifetime.

Assets as entities also make the fence-deferred slot release the same mechanism physics uses for mid-chain despawns (ED16).

**Alternative.** Q1(b): keep `Assets<T>` and deduplicate internally (EK17). This is fully specified.

**Trade-offs.**
- The `Handle<T>` construction path changes.
- Insertion is deferred to an apply window (`J:.../params/commands.rs:242` reserves atomically).
- `MeshGpu`/`TextureGpu` stay in NonSend device lanes.
- Assets now depend on physics K3/K6′ (the kernel-first order puts both before AS2).

### ED6. Per-entity instance data stays CPU columns. The draw-ordered ring stays. The caster partition folds into the one gather, and **every re-walk replays it**.

**What.**
- `InstanceModelCol` stays a table component.
- `MeshRenderScratch` gains per-batch lanes: `mesh_row`, `caster_count`, world AABB. The per-bucket cursor lane widens to `[u32; 2]`.
- **Count pass.** Per bucket: `count[b]` and `caster_count[b]`.
- **Prefix.** `base[b]`.
- **Scatter.** Cursors are initialised as `cur[b] = [base[b] + caster_count[b], base[b]]` (index 0 = non-caster, 1 = caster). Each row takes `slot = cur[b][is_caster as usize]; cur[b][is_caster as usize] += 1`. This is an indexed select with no branch.
- **Re-walks (X-15).** `gather_material_tex_into` (`mesh_draw.rs:1324`) and `gather_prev_ring_into` (hwrt) re-initialise `cur` from the same `(base, caster_count)` lanes. They walk the **same `q`**, which gains `Option<&ShadowCaster>` in its tuple. They obtain every slot through **one shared `#[inline]` function** `bucket_slot(&mut cur, mesh_id, is_caster) -> u32`.
  - Query order and the per-archetype presence are identical in every walk, so index alignment holds by construction. The single function means the rule cannot diverge between walks.
  - Readers of the primary lanes (VB rows at `mesh_draw.rs:532-533`, `material_ids`, `pair_out_slot`) inherit the order.
- `CsmCasterScratch` (`csm_caster.rs:90`) and `DrawListScratch` plus its transmute (`J:crates/boyko_app/src/gpu_scene/mod.rs:8132`) are deleted.

**Why.**
- Casters are a strict subset of the main gather (`csm_caster.rs:190` `(Enabled<RenderEnabled>, With<ShadowCaster>),` against `mesh_draw.rs:1253` `Enabled<RenderEnabled>,`), with no CPU culling in between.
- The change removes one count/prefix/scatter pass and one 48 B/caster ring.
- The AABB fold moves into the gather, which has already loaded the row.

**Correction to rev 1.** There *is* a per-row select, and it is branch-free. `Option<&ShadowCaster>` is resolved per row. Its value is constant per archetype, and the select indexes a two-element array.

**Alternatives.**
- A per-instance caster flag lane: rejected, because it adds 1 B/instance and a filtered shadow draw.
- Scattering into mapped FIF memory: rejected, because a bucketed scatter is hostile to write-combined memory (whether the rings are WC is not verified; O-6).
- A slot-indexed dense instance column: a perf fork, not a form fork (O-3).

**Trade-offs.**
- Instance order within a bucket changes. Opaque output is unaffected except where coplanar instances overlap at equal depth, where draw order picks the winner. The golden gate reports any such pixel instead of accepting it.
- Each re-walk pays one extra ZST presence read per row, from a per-archetype-constant column.

### ED7. Input: raw events are kernel events. `PhysicalInput` is folded once. UI is an input source, not a second writer. (unchanged)

**What.**
- `RawInput` is `#[event(swap = "every_frame")]` (EK7).
- Interim: the runner's drain calls `send_event`.
- Final: an OS-callback sink (EK7) writes from `WNDPROC` on the dispatcher thread inside `pump_events`. `InputRing` is then deleted.
- `input_fold_physical` runs once. `action_fold::<A>` runs once per `A`.

**Why.**
- Destructive consumption loses edges when several plugins read: `J:crates/boyko_input/src/action/process.rs:59-60` `queue.begin_frame();` / `physical.begin_frame();`.
- Text is dropped: `J:crates/boyko_input/src/raw/queue.rs:302` `RawInputEvent::Text(_) => {}`.

**Alternative rejected.** A global `EveryFrame` swap: physics S7 sends contact events from Fixed (`M:.../PHYSICS-ECS-UNIFICATION-DESIGN.md:405` "\| S7 \| `physics_events` \| … `EventWriter<…>` \|"). Those must keep `WaitForFixed`.

**Trade-off.** Kernel lanes refuse the newest event (`J:.../events/event_buffer.rs:62`). The sink therefore coalesces mouse motion (`window.rs:177-189`).

### ED8. UI: every system is on the scheduler, 13 exclusive systems become 1, and the per-frame DFS becomes a linear scan

**What.**
- Plugins register every UI system.
- Most become function systems over `Query::get`/`get_mut` (X-6), `Entities` (X-2) and `EntityCommands::enable/disable` (J `entity_commands.rs:220`/`:236`).
- **The paint walk (change frames only).** One DFS per change frame, triggered by any layout input, `Changed<StackIndex>`, `Changed<ComputedClip>`, or a `Children` change. Roots are walked in entity-id order (`focus.rs:214`), and children in `Children` order (EK15a). It writes two derived components on every laid-out node with `set_if_neq`:
  - **`ComputedPaintKey(u64)`** `= (StackIndex.0 as u64) << 32 | dfs_index`. Here `dfs_index` is a **global** pre-order counter across all roots (X-16), and `StackIndex` defaults to 0 when absent.
    - Among `Interaction` nodes, this key orders exactly like today's `(stack_index, paint_seq, entity)`. `paint_seq` counts candidates only, in DFS order, so it is monotone in `dfs_index`.
    - The `entity` tie-break is unreachable (keys are unique; debug_assert).
    - The render gather's stable sort by `StackIndex` over the DFS sequence equals a sort by this key. So one datum serves both consumers, with one writer (G3). Whether today's render DFS uses the same root order as focus is not verified; the UI4 gate compares both oracles.
  - **`ComputedEffectiveClip { min: [f32; 2], max: [f32; 2] }`** with **replace** semantics, matching today's `own_clip.or(inherited_clip)` (`focus.rs:222`).
    - "No clip" is stored as `[-INF, -INF, +INF, +INF]`, so the scan is branch-free.
    - `max = x + w` is computed once in f32 by the walk. That is the same f32 operation `point_in_clip` performs today (`focus.rs:283`), so results are bit-identical.
    - `ComputedClip` stays author-owned and untouched (O-1 answered).
- **The hit-test (every frame).** `Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>`, **filtered by the presence of `Interaction`** as today (`focus.rs:225`), not by `Focusable`. It does a branch-free argmax of the key among rows whose rect and clip contain the cursor, then the write pass.
  - `FocusPolicy` is read only by the occlusion debug_assert.
  - The tab-order sort for `Focusable` rows stays on a `Local` ScratchColumn.
- **Layout scratch.** Layout becomes a function system. Its three depth pools become three `Local<ScratchColumn<Entity>>` used as LIFO stacks: `mark = len()`, push the level's children, recurse, `truncate(mark)` (X-17). A level's children are held as an `(start, end)` index range, never as a slice across a push, so no borrow outlives a grow (Miri gate).
- **The residual exclusive system** is `ui_bind_apply`. Its source component ids are chosen at setup. It uses EK2 ticks. It runs in its own `UiBindSet`, **after `GameplaySet`**, so a HUD shows this frame's gameplay writes (O6).

**Why.**
- Focus runs a whole-tree DFS every frame (`U:crates/boyko_ui/src/interaction/focus.rs:159` `collect_candidates(world, &mut scratch);`).
- There are five root-set copies.
- Serial exclusive nodes block overlap with gameplay.

**Alternatives.**
- A per-root-unique paint order (rev 1): rejected, because it inverts cross-root hover (W1).
- An intersecting clip: rejected, because it changes behaviour against the oracle.
- A dynamic-access query param for bind: rejected, too large for one consumer.
- Bevy's taffy side tree: rejected, because it uses an `EntityHashMap` side store.

**Trade-off.**
- Change frames pay two `set_if_neq` per node: 8 B plus 16 B written, only where a value changed.
- Every laid-out node carries 24 B more (arith.).

### ED9. One-frame UI messages become triggers and a typed fixed resource lane

**What.**
- Click, submit and hover-enter/leave become `Trigger`s with `Up` propagation, raised through `Commands::trigger` (EK8).
- **UI action presses go into `UiActionPresses<A>`**, a resource-column generic over the one action set the UI dispatches to (W2).
  - `UiInteractionPlugin::<A>` inserts it. The hit-test system is `ui_hit_test::<A>`, monomorphised once, exactly as `ui_dispatch_system::<A>` is today (`U:crates/boyko_ui/src/interaction/plugin.rs:117-118` `.add_system(ui_dispatch_system::<A>)`).
  - `action_fold::<B>` reads `Option<Res<UiActionPresses<B>>>`. `Option<Res<R>>` is a SystemParam on J: `J:crates/boyko_ecs/src/ecs/core/system/params/res.rs:160` `unsafe impl<'a, R: Resource> SystemParam for Option<Res<'a, R>> {`.
  - For every `B ≠ A` the resource is absent, so a UI press never reaches another set.
  - This deletes `ui_press` and the second freeze.
- Tween completions become `EntityCommands::remove` (X-4).

**Why.**
- Kernel events are next-frame (X-1).
- Triggers exist and bubble (X-7).
- `OnClick` carries a raw index that is not typed by `A` (`U:crates/boyko_ui/src/interaction/action.rs:3-5` "Each carries a dense `Actionlike::index()` as a raw `u16` … NOT a generic `OnClick<A>`"). The routing must therefore be typed at the resource.

**Alternatives.**
- A non-generic `UiActionPresses` (rev 1): rejected, because it creates phantom presses across sets (W2).
- A same-frame event mode: rejected (rev 1 reasons stand).

**Trade-off.** Observers run on the applying thread. There are a handful per frame.

### ED10. Children keep insertion order. The ordered remove lands first, on today's storage.

**What.**
- **EK15a.** A new `RelationshipSourceCollection` impl, `Ordered` (a newtype over today's `Vec<Entity>`), whose `remove` is an order-preserving `Vec::remove`.
- `Children` switches its `type Collection` (`J:.../hierarchy/mod.rs:197`) to it.
- It has **no prerequisite**. The move of relation storage onto kernel columns is EK15c (after physics K7) and does not block any UI rung.
- Layout's id sort (`U:crates/boyko_ui/src/layout.rs:549` `child_buf.sort_unstable_by_key(|e| e.id().0);`) is deleted. B-5 closes.

**Cost (X-8).**
- Today's remove is an O(n) scan plus an O(1) swap.
- The ordered remove is the same scan plus an O(n − idx) `memmove`. That is the same class and about 2× the memory traffic in the worst case (read + write).
- Append stays amortised O(1).

### ED11. The device, present and scene GPU state become NonSend World residents (unchanged)

**What.**
- `Renderer`, `GBufferFrame`, `PresentChain`, `FrameInFlight`, and the per-feature GPU bundles become NonSend residents. The per-feature bundles are split out of `GpuSceneBundles` (`gpu_scene/mod.rs:924`): `ShadowGpu`, `ParticleGpu`, `DdgiGpu`, `UiGpu`, and so on.
- `VulkanContext` is owned by the World. The `device.rs:776` static becomes a non-owning pointer for the `'static` borrowers.
- EK11 ranks enforce today's order (`host.rs:67-80`).

**Why.**
- G1.
- The take-out/reinsert dance (`runner.rs:1289`), whose own price is stated at `runner.rs:1275`.

**Trade-off.** The `'static` borrow stays a documented lifetime extension whose SAFETY rests on EK11's order.

### ED12. One SDF scene field. A hand-off to physics, landed after physics U5b.

**What.** `SdfSceneField` is a resource-column maintained by `sdf_scene_sync` from `Changed`/`Added`/`Removed<SdfPrimitive>` (EK3). It is read by the physics narrowphase, the marcher mirror and the brick bake.

**Why.** There are three diverging stores today:
- `J:crates/boyko_physics/src/sdf_query.rs:46` `pub struct SdfField(SdfEditField);`
- `J:crates/boyko_app/src/gpu_scene/mod.rs:1036`
- `runner.rs:1384` "a post-boot `SdfPrimitive` spawn is silently ignored"

**Ownership (W8d).**
- Physics owns `SdfField` and ports `physics_narrowphase_sdf` in its rung U5b (`M:.../PHYSICS-ECS-UNIFICATION-DESIGN.md:795` "port `physics_narrowphase_sdf` and the coupled soft step's body reads").
- The unification is filed as an input to the physics study (§13): one resource of the same `[SdfEdit; 16]` layout (`sdf_query.rs:38-44`), with the name chosen by physics.
- RE9 is ordered **after physics U5b**, and it edits only the render and app side.

### ED13. Frame-graph arenas are system-scratch of `render_record`, lent through a storage trait

**What.**
- `FrameGraph<S: GraphStorage>`.
- `boyko_render` implements `S` over `Local` ScratchColumns.
- `boyko_rhi_vulkan` keeps a test-only `Vec` impl.

**Why.**
- The RHI crate cannot name the kernel (`J:crates/boyko_rhi_vulkan/Cargo.toml:54-73`).
- It keeps one storage kind (the rejected-bespoke-column ruling).

**Alternative rejected.** The `boyko_memory` split (M `ALLOCATOR-DESIGN-SPACE.md:84`; the design is at CHANGES REQUESTED, `:413`).

**Trade-offs.**
- Generic parameters on `render_gbuffer_frame`.
- **Resident floor (O2).** 18 lanes (`J:crates/boyko_rhi_vulkan/src/framegraph/graph.rs:126-205`) × a 64 KiB untracked data floor ≈ 1.1 MiB resident, where there were a few KB of `Vec` (arith.).
  - CPU work per frame is unchanged, so this is not a hot-loop regression.
  - It is recorded under the resident-commit metric and routed to `POOL-SUBGRANULAR-PACKING-PLAN.md:48`.
  - Repacking the 18 SoA lanes as AoS records would cut this to 3 columns, but it is a perf fork on the frame graph's walk. It is not taken without a measurement.

### ED14. Views: the view is a component on the camera, and "active" is enable-state (unchanged)

- `RenderView { view_uniform }` goes on each camera.
- `ActiveView` is an EnableTag; at most one is enabled (debug_assert).
- `ActiveCamera` and `Camera.is_active` (`J:crates/boyko_scene/src/camera.rs:113` `pub is_active: bool,`) are deleted.

### ED15 (new, C1). One kernel span primitive, `SegmentedColumn<T>` = physics K7, with the header held by the owner

**What.**
- **Name and prerequisite, exactly as physics states them:** `M:.../PHYSICS-ECS-UNIFICATION-DESIGN.md:473` "\| K7 \| Segmented dense column \| Per-entity variable-length ranges in a group bank \| SoftBody (Q2) \| animation bone arrays, UI text runs \| K3 \|". It is physics-owned, landed in physics rung S0. This design adopts the same position in the kernel order (after K3).
- **Its shape.** Filed to physics as an input (§13); this design does not edit physics.
  - **Element storage:** one untracked `ComponentPool` with an address-stable base, holding `T: Copy`.
  - **Owner key: none inside the primitive.** Each owner stores a `SpanRef<T> { ptr, len, class }` header wherever the owner lives:
    - in a K3 group column (physics soft bodies, animation bone arrays — hence K3 for that consumer);
    - in a table component (`Children` under EK15c);
    - in a resource.

    So a table-entity owner needs no dense handle, and a dense-slot owner needs no entity lookup.
  - **Allocation:** power-of-two size classes. A span of length `n` occupies a block of class `ceil(log2 n)`.
  - **Growth:** in place while `len < 2^class`. Past that, relocate: allocate the next class (LIFO free list first, frontier otherwise), copy `len` elements, free the old block to its class list, and rewrite the owner's header.
    - Amortised O(1) per append.
    - Total copy for `n` appends ≤ 2n elements.
    - This answers the 10k-children edge case.
  - **Holes:** a freed block goes to its class's LIFO free list (a `VmColumn<u32>`, like `dense_store.rs:118`'s `s2e`). **No compaction and no offset re-issue.** Offsets are stable until the owner itself grows or frees.
    - Bound: internal slack ≤ 2× per span.
    - Free blocks ≤ the per-class peak.
    - An exact-fit policy is not added until a measurement asks for it.
  - **Pointer validity:** the base never moves, so a `SpanRef` stays valid until its own span relocates or is freed. Both happen only under `&mut` of the owner (apply windows, relationship hooks, the owning system's write param).
- **Fixed-shape, write-once spans do not need K7.** Examples: `InputMap` bindings, font glyph tables, the asset path index. They are CSR on two EK1 ScratchColumns (offsets + elements), with no new feature.

**Why.** One name and one shape serve both needs:
- alloc/free of fixed spans (physics soft bodies);
- arbitrary growth at arbitrary owners (`Children`).

The earlier aliases (`RaggedColumn`, `CsrColumn`, `VmJagged`, AK-2 = KR-3) collapse into it.

**Alternatives.**
- A second primitive for growable collections: rejected, because it recreates the five-names problem (plans lens C-12).
- Compaction with an owner back-reference lane: rejected, because it needs random writes into owners and a per-span back-pointer, with no measured need.

**Trade-off.**
- Up to 2× slack per span.
- The physics consumer takes the same slack for fixed-shape spans unless physics chooses CSR for those instead. That is physics's call.

### ED16 (new, W9). One deferred-release capability, K6 with a caller-supplied horizon (K6′, a hand-off to physics)

**What.**
- Physics K3/D14 already defines deferred dense release: `M:.../PHYSICS-ECS-UNIFICATION-DESIGN.md:239-240` "The slot enters a `dying` list, not the free list" / "Release happens only through a kernel command, `Commands::release_dense_group::<G>()`".
- This design asks for **one** amendment: each `dying` entry is stamped with the world change tick of the apply window that removed it, and release takes a horizon, `release_dense_group::<G>(horizon: Tick)`. It frees exactly the dying slots stamped before `horizon` (wrap-aware compare).
  - Physics passes its S6 `this_run`, which is today's "release all dying" semantics.
  - Assets pass `FrameInFlight::completed_gather_tick()`: the tick at which the last fence-completed frame's gather ran.
- An `EcsMaster::release_dense_group_with::<G>(horizon, |slot| …)` visitor lets R1 free the NonSend device lane of each released slot in the same pass.

**Proof of the asset horizon.**
- A slot that died at tick `t` is named only by frames whose gather ran before `t`.
- Queue completion is in order, so once a completed frame's gather tick exceeds `t`, every earlier frame has completed.

**Cost.**
- +4 B per dying entry.
- One compare per dying slot per release call.

### ED17 (new, W6). The EK3 change log has a fixed two-frame horizon and a typed overflow

**What.**
- Records carry the full `Entity`, id plus generation (X-11).
- The log is truncated at driver step ② to records stamped after the start of the previous frame. That is two frames of retention, like the event double buffer.
- A reader whose `last_run` is older than the oldest retained record gets `RemovalWindow::Overflowed` instead of an iterator. The type forces the caller to handle it.
- An overflow emits one rate-limited diagnostic (the W0701 pattern), and the consumer runs its full-rebuild path. Every EK3 consumer already has one: UI full repack, light full rebuild, propagation full reseed, SDF full resync.

**Why.**
- Rev 1's minimum-cursor truncation never truncates while any reader is `run_if`-false (VB-only, hwrt-only, windowless). That is unbounded side growth.
- A fixed horizon bounds memory at 24 B × structural changes per two frames. A reader that skipped frames pays a rebuild it would need for correctness anyway.

**Trade-off.** A reader skipped for one frame or more after a structural change rebuilds once.

---

## 4. The entity model (MUST 1)

**Tags for "no per-entity form fits"** (each is a one-sentence refutation of component, dense, enable-state and relation):
- N1: exactly one per world.
- N2: a per-frame product in draw or bucket order.
- N3: produced and consumed inside one system run.
- N4: must exist before any World, on any thread, or in the panic path.
- N5: memory owned by the OS or the driver.
- N6: extent or value fixed at compile time.
- N7: measured spawn cost (`J:crates/boyko_render/src/particle.rs:5-6` "20k spawns/frame through `Commands` is 0.6–2 ms of CPU against ~2 µs of GPU emit").
- N8: no identity across frames.

"Event refuted" always means next-frame visibility (X-1) unless another reason is given.

### 4.1 Owning entity types

Only two rows changed from rev 1:
- **mesh, material, texture, font, sprite-sheet, animation-clip, UI document:** yes, asset entities (Q1a), with the value as a **K3 Deferred group column** (ED5).
- **animation instance:** unchanged (dense `AnimInstance`). The **pose bank is the animation plan's** decision (§13).

| Type | Entity? | Evidence or reason |
|---|---|---|
| widget | **yes** | `U:crates/boyko_ui/src/lib.rs:3` "Widgets are entities" |
| text-run | no: components `UiText` + `UiTextBuffer` on the widget | 1:1 with the widget; not a K7 consumer here, because `UiTextBuffer` is 256 B inline |
| glyph | no: system-scratch in the UI pack | N8, N7 |
| sprite (UI) | no: components on the widget | 1:1 |
| sprite (world) | yes, as a render-instance | — |
| mesh, material, texture, font, sprite-sheet, animation-clip, UI document | **yes, asset entities (Q1a)** | ED5 |
| light, camera | **yes** | — |
| render-instance | **the simulation entity itself** | ED1 |
| GPU resource | no | N5; passes are N8 (`J:docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md:164-165`) |
| window | no in v1: NonSend resource | N1; Q3 |
| input device | no in v1 | N1 (`CapturedMsg` carries no device id, `window.rs:89-107`) |
| action | no: resource-column `ActionState<A>` | N6; Q3 |
| animation instance | no: dense `AnimInstance` | M `ANIMATION-DESIGN-SPACE.md:197-198`; bones are never entities (`:620`) |
| log sink | no: kernel-internal statics | N4 (`J:docs/LOGGING-SYSTEM-PLAN.md:136`) |
| log target | **yes**: `LogTarget` projected into `CONTROL` | the `ProfilingScope` pattern |
| profiler zone | scope = entity; descriptor = static | `J:.../profiling/ecs_control.rs:264-265`; N4 |
| type descriptor | no: kernel-internal static | N4 (`R:crates/boyko_reflect/src/registry.rs:29`) |
| prefab template | **yes** (EK18) | `J:.../clone/prefab.rs:43` |
| particle | no: device-only buffers in NonSend `ParticleGpu` | N7, N5 |
| pair, contact, island | per the physics study | — |

### 4.2 Datum placement

Rows marked **(rev 2)** changed.

**UI** (U, `crates/boyko_ui/src` unless marked `render/`)

| # | Datum | Today | Form | Rung |
|---|---|---|---|---|
| 1 | layout inputs/outputs, style, markers | component | component (unchanged) | — |
| 2 | `UiText.font`, `UiImage.texture`, `UiSpriteSheet.sheet` | raw indices (`components.rs:442` `pub texture: u32,`) | **relation** widget → asset via `Handle<T>`; `UiText` 12 → 16 B, `UiSpriteSheet` 4 → 12 B (arith.) | UI7 |
| 3 **(rev 2)** | `FontTable` + 4 `Box` fields | Resource `Vec` (`text/font.rs:139`) | asset entity `Font`: `FontMetrics` group column (including the atlas bindless slot) + glyph table as **CSR on EK1 columns**, header `(offset, len)` in `FontMetrics` (write-once; no K7) | UI7 |
| 4 | `UiSheetTable` | Resource `Vec` (`sprite.rs:257`) | asset entity `SpriteSheet` | UI7 |
| 5 **(rev 2)** | sibling order | two orders (B-5) | **relation** order via the `Ordered` collection (EK15a) | UI3 |
| 6 | `Tween*`, `UiSpriteCursor` | dense | dense (unchanged) | — |
| 7 | `Interaction` + 3 string-named tags + config | component + dynamic tags | component + typed **enable-state**; config deleted | UI4 |
| 8 **(rev 2)** | bind `source: Entity` | field (`binding/components.rs:34`) | **relation** `BoundTo(source)` with reverse `Bindings`, on today's `Relationship` API (`J:.../relationship/mod.rs:215` `pub trait Relationship: Component + Sized {`) and its `Vec` collection; storage moves with EK15c | UI5 |
| 9 | `EntityAnchor(Entity)` | enum field | **relation** `AnchoredTo(entity)` | UI6 |
| 10 **(rev 2)** | paint order + effective clip | two DFS copies (G3) | **components** `ComputedPaintKey(u64)` + `ComputedEffectiveClip` (derived, one writer, ED8) | UI4 |
| 11 | `UiViewport` + `generation` | Resource; no production writer | resource-column derived from `WindowSurface` (EK5) | UI2 |
| 12 | `UiRenderGeneration` | u64 | deleted: `Changed ∪ Removed ∪ Toggled` (EK3) | UI8 |
| 13 | `UiPointerState` slots | fixed array | resource-column (N1), message slots removed | UI4 |
| 14 | `click_fired`, `pending_submit`, `hover_entered` | one-frame slots | **event** in its trigger form (ED9) | UI4 |
| 15 | tween `done` Vec (`animation.rs:473`) | Resource Vec | deleted: `EntityCommands::remove` | UI0 (via UI9) |
| 16 **(rev 2)** | `ui_press` + re-freeze | cross-subsystem write | **resource-column** `UiActionPresses<A>` (N1 per `A`; event refuted) | IN3 |
| 17 | `UiInputFocus` | `Option<Entity>` | resource-column | — |
| 18 | `HoveredWorldEntity` + prev mirror | Resource + copy | resource-column + triggers; mirror deleted | UI6 |
| 19 **(rev 2)** | layout depth pools (3 × 128 `Vec`) | Resource | **system-scratch**: three `Local<ScratchColumn<Entity>>` LIFO stacks with `truncate(mark)` (X-17) (N3) | UI3 |
| 20 | layout flat arenas | Resource Vec | system-scratch: `Local` ScratchColumns (N3) | UI3 |
| 21 | 5 root-set copies | Vec copies | deleted: `Query<…, With<UiRoot>>` | UI2 |
| 22 **(rev 2)** | focus candidates / `write_nodes` / focusables | per-frame Vecs | candidates and `write_nodes` deleted (row 10); tab-order sort on a `Local` ScratchColumn | UI4 |
| 23 | bind/bar lists, private ticks | Vec + windows | deleted: `Changed` + EK2 | UI5 |
| 24 | world-UI scratch bounds | Vec | system-scratch (N3) | UI6 |
| 25 | glyph quads (`text/emit.rs:65`) | unfed Vec | system-scratch in the pack (N8); wired (B-2) | UI8 |
| 26 | upload staging (1.72 MiB `Box`), `node_buf`, `keys` | system-owned (`render/upload.rs:198`, `:203`, `:206`) | **resource-column** `UiPackLanes` (written by the Main gather, read by the Render mirror); grows in place; no cap or truncation | UI8 |
| 27 | legacy `UiRenderScratch` | no caller (`render/upload.rs:49`) | deleted (Q4) | UI8 |
| 28 | UI GPU pipeline/atlas/ring | on `RhiContext` | out-of-scope: driver-owned; handles in NonSend `UiGpu` | UI8 |
| 29 **(rev 2)** | `.ui` source / parse tree / report / plan | String/Vec/HashSet | **system-scratch** of the reload system on EK1 columns: a `u8` arena with `(offset, len)` handles, and a sorted id column instead of the set; diagnostics → `boyko_log` | UI10 |
| 30 | `UiTreeView` | per-reload copy | deleted | UI10 |
| 31 | 2 `Arc<Mutex>` sinks (`reload/system.rs:167`) | heap cells | deleted: `run_system` returns `Out` (`J:.../system_api.rs:111`); whether `Out` admits `DespawnPlan` is not verified (O-4) | UI10 |
| 32 | `UiHotReload.doc_roots` + mtime | Resource + spill Vec | relation `FromDocument(asset)` + `AssetWatch` on the document asset (EK21) | UI10 |
| 33 | SPIR-V, tag names, bind accessor table | static | out-of-scope: compile-time / kernel-internal | — |
| 34 **(rev 2)** | `Children` | kernel `Vec<Entity>` | relation; order by EK15a now, storage on K7 by EK15c later | K-EK15a / K-EK15c |

**Render** (J)

| # | Datum | Today | Form | Rung |
|---|---|---|---|---|
| 1 | `InstanceModelCol` | component | component (unchanged) | — |
| 2 | `Gpu3dInstance` + sync | no consumer | deleted (Q4) | RE1 |
| 3 | `GpuTransform3D` | dense | dense (unchanged) | — |
| 4 | `PrevInstanceModelCol` | table, doc says dense | component; doc fixed | RE1 |
| 5 **(rev 2)** | `MeshRenderScratch` lanes | Resource of ScratchColumns | **resource-column** (N2) + per-batch `mesh_row` / `caster_count` / AABB lanes + a `[u32; 2]` cursor lane | RE2 |
| 6 | `CsmCasterScratch` | second gather | deleted (ED6) | RE2 |
| 7 | `DrawListScratch` + transmute | host Vec | deleted | RE2 |
| 8 | `LightTableStaging` | ScratchColumn Resource | unchanged | — |
| 9 **(rev 2)** | per-slot upload generations | host arrays + NonSend | `FifMirrorState` **inside each mirror's target** (ED3, RF1) | RE4 |
| 10 | `LightSeedState` | closure-captured | deleted (EK4) | RE5 |
| 11 | `LightTableDirty` | bool channel | deleted (EK3) | RE5 |
| 12 **(rev 2)** | `MaterialTable` GPU mirror | NonSend | **resource-column**: NonSend, indexed by the material asset slot; VRAM is driver-owned (N5); gated by `Changed` on the material value (Q1a). **Not counted as removed in G-FORM**: the row keeps its count, and only its index space and gate change (W5) | RE6 |
| 13 | `Assets<MeshGpu>` / `Assets<TextureGpu>` | NonSend store | split (ED4) | AS1 |
| 14 **(rev 2)** | orphan queues | NonSend `Vec<(T,u64)>` | **K3 deferred release with a horizon** (ED16) (Q1a) / EK17 (Q1b) | AS2 |
| 15 | `RetiredGpuBuffers` | NonSend Vec | NonSend resource on an EK12 owning column | RE7 |
| 16 **(rev 2)** | bindless / geometry slot allocators | second index spaces (`bindless.rs:73-74`) | deleted: slot = asset dense slot; slot 0 = the pinned sentinel (ED5); **texture slots < 4096**, because the UI record packs 12 bits (`ui_rect.fs.hlsl:37`) and a load past it fails with a log | AS3 |
| 17 **(rev 2)** | `MeshGeometryTable` meta/bounds | host-visible mirror | **resource-column**: NonSend, indexed by mesh asset slot; driver-owned VRAM (N5) | RE6 |
| 18 **(rev 2)** | `GpuColumnManager.meta` | Vec (`gpu_column.rs:532`) | kernel-internal: Stage-5 fold into `PoolBacking::Device` (EK16, narrowed) | K-EK16 |
| 19 | `GpuSceneBundles` | host field | NonSend resources per feature plugin | HO4 |
| 20 | `Renderer`, `GBufferFrame`, `Swapchain`, `Surface`, `Window` | host | NonSend resources (N5 for OS/driver objects) | HO4 |
| 21 | FrameGraph SoA arenas | `Vec` | system-scratch of `render_record` (ED13) | RE8 |
| 22 | `resolved_render_path` host copy | duplicate | deleted | HO1 |
| 23 | `frame_index` | stack local | `Time.frame_count` | HO1 |
| 24 | TAA / jitter / motion-cam writes | host code | writers become Main systems | HO2 |
| 25 | 22 upload call sites | runner | FIF mirror systems (RF1) | RE4 |
| 26 **(rev 2)** | SDF: three stores | three stores | one resource-column, named by physics (ED12, hand-off) | RE9 |
| 27 **(rev 2)** | particle pools | device-only, host bundle | NonSend `ParticleGpu` resident; VRAM driver-owned (N5, N7); **no form change beyond residency**, moved in HO4 | HO4 |
| 28 | particle emit/effect scratch | ScratchColumn Resource | unchanged | — |
| 29 | `retire_scratch` | host Vec | system-scratch of `render_retire` | HO4 |
| 30 | `ViewUniform` | Resource | component `RenderView` (ED14) | SC3 |

**Host, window, input** (J)

| Datum | Today | Form | Rung |
|---|---|---|---|
| `InputRing` (`window.rs:121`) | Box behind `GWLP_USERDATA` | **event** `RawInput` via EK7's OS sink | IN4 |
| `RawInputQueue` (`queue.rs:33`) | destructive ring | **event** `RawInput` | IN1 |
| `PhysicalInput` | Resource | resource-column (N1; persistent levels) | — |
| `ActionState<A>` + 4 `Box<[f32]>` | Resource | resource-column on EK1 columns sized at build | IN2 |
| `InputMap<A>` CSR **(rev 2)** | 2 Boxes | resource-column: **CSR on two EK1 columns** (write-once at build; rebind is cold and rebuilds). No K7 | IN2 |
| `ACTION_NAMES` (`names.rs:42`) | process `OnceLock` | resource-column `ActionNames` | IN2 |
| `RebindSession<A>` | user struct | component on the listening widget + `EventReader<RawInput>` | IN1 |
| window size ×3 | three copies | one resource-column `WindowSurface` (EK5) | HO3 |
| boot commits (`host.rs:102-131`) | host fields | resource-column `RenderCommit` | HO1 |
| `VulkanContext` + `SINGLETON` | process static | NonSend resource owned by the World | HO6 |
| `DebugMessengerState` Box | driver holds `p_user_data` | one-row EK1 column (address-stable); messages → `boyko_log` | HO6 |
| `Renderer`/`Swapchain` handle Vecs | struct fields | EK1 columns sized at (re)create; handles N5 | HO4 |
| `frames_run`, `last`, `frame_index` | runner locals | `Time.frame_count`; `last` deleted | HO1 |
| `HostFrameStats`, `RenderEpoch`, `AppExit` | Resources | resource-column; writers move to `render_publish` | HO4 |
| Escape fallback | host logic | deleted: a default quit action | IN1 |
| diagnostics drivers | runner locals | resource-column; presence = armed | HO5 |
| `WindowDesc` + `BOYKO_*` reads | closure / env | resource-column `LaunchConfig` | HO1 |
| `FrameWriteToken` | typed local | NonSend `FrameInFlight` (ED2) | HO4 |
| `TimerResolutionGuard` | RAII local | out-of-scope: OS-owned | — |

**Scene, assets, services** (J; R for reflect)

| Datum | Today | Form | Rung |
|---|---|---|---|
| `Transform`, `GlobalTransform`, `Visibility`, `Name` | component | unchanged | — |
| `RenderEnabled`, `ProfilingScopeEnabled` | enable-state | unchanged; readers use `Toggled` (EK3) | — |
| propagation `stack` / `dirty` | Resource Vec | system-scratch `Local` | SC1 |
| propagation `detached` | observer → Vec | read via `Removed<ChildOf>` (EK3) | SC1 |
| propagation `last_run`, observer flag | Resource fields | deleted | SC1 |
| `SetRenderEnabledById` | custom command | deleted (`Entities` + `disable`) | SC2 |
| `Camera.is_active`, `ActiveCamera` | bool + Resource | enable-state `ActiveView` | SC3 |
| name interner (`identity.rs:81`) | `Mutex<HashMap>` | kernel-internal StrInterner (EK20) | SC4 |
| `MeshHandle` / `MaterialHandle` + 4 hooks | raw index | Q1a: **relation** instance → asset (count-only target, EK15b) + a derived `u32`/`u16` slot component the pack reads (same bytes); Q1b: unchanged | AS4 |
| `MeshRefGen`/`MaterialRefGen` + validate | per-frame net | deleted under Q1a | AS4 |
| `RefcountDeltas` | hook → Vec | deleted (relation count) | AS4 |
| `Assets<T>` value / `slot_word` / `free` / `live` / `refcount` / `pinned` / dirty counters **(rev 2)** | parallel kernel | Q1a: **K3 Deferred group column** / kernel-internal slot map / relation count / `Pinned` ZST / kernel change detection | AS5 |
| `AssetStaging<A>` | NonSend Vec | component `Staged<A::Cpu>` on the asset entity | AS5 |
| `AssetPaths<A>` **(rev 2)** | NonSend `PathIndex` | component `AssetPath(hash)` + resource-column sorted hash → `Entity` index on one EK1 column (write-once per load batch, re-sorted cold) | AS5 |
| `DeferredFree` **(rev 2)** | Resource Vec | deleted: K3 `dying` + K6′ horizon (ED16) | AS2 |
| `ASSET_LAYOUTS` (`backing.rs:87`) | `Mutex<HashMap>` | kernel-internal via `TypeIntern` | SC4 |
| save/load plans, `LoadEntityMap` **(rev 2)** | Vecs | system-scratch on EK1 columns held by a `SerializeScratch` the caller passes | SC5 |
| `Prefab` | off-world image | template entities (EK18) | SC6 |
| log/diag statics, `REGISTRY`, `ARM_MASK` | `.bss` | kernel-internal (N4/N6) | — |
| `LogRing`, `LogStats`, drain | resource + system | unchanged | — |
| log target levels | env | component on `LogTarget` entities | LG1 |
| `Profiler` + fold | outside the schedule | unchanged (`profiling/plugin.rs:20-21`) | — |
| GPU `WindowReducer` (`reduce.rs:59`) | runner local | resource-column: the `Profiler`'s GPU channel | LG2 |
| `TelemetryStream` | `.bss`, no caller | Q4 | LG2 |
| `Time`, `FixedTime`, `State<S>`, records | Resources | unchanged | — |
| `REFLECT` table | static | kernel-internal; enable-bit source via EK19 | SC5 |

**Animation** (M design; not implemented; rev 2 changes the ownership statement only)

| Datum | Form |
|---|---|
| `AnimInstance` | dense component (AN1, kept) |
| pose bank | **owned by the animation plan**: a resource-owned ScratchColumn bank (AN1, `M:docs/animation/ANIMATION-DESIGN-SPACE.md:175-177`). Input filed (§13): if K7 lands with ED15's header-by-owner shape, the bank is a K7 consumer (AK-2 = KR-3). No decision is taken here. |
| clip data | asset entity (Q1a); blob form per the animation plan |
| skinning palette upload | a FIF mirror (RF1) |

---

## 5. Data structures

```rust
// ── Render schedule state (boyko_render; NonSend) ─────────────────────────────
pub struct FrameInFlight {                 // written by render_acquire, token taken by render_record
    token: Option<FrameWriteToken>,        // None ⇔ minimized / no acquire this frame
    gather_tick: [Tick; FRAMES_IN_FLIGHT], // tick at which RenderPrepareSet's gather ran for the frame in slot s
    completed: Option<usize>,              // slot whose fence R0 last waited on
}                                          // size not verified (FrameWriteToken holds `slot: usize`)

#[repr(C)]
pub struct FifMirrorState<const N: usize> { // owned BY THE TARGET (ED3); N = 2 → 9 B + pad (arith.)
    uploaded: [Tick; N],
    valid: u8,                              // bit s ⇔ slot s holds bytes of `uploaded[s]`; cleared on buffer replace
}

pub trait FifMirror: 'static {             // boyko_render feature RF1; monomorphised, no dyn
    type Source: Resource;                 // #[resource(ticks)] (EK5)
    type Target: NonSendResource;          // owns the per-slot rings (driver memory, N5) + FifMirrorState
    fn bytes(src: &Self::Source) -> &[u8];
    fn state(t: &mut Self::Target) -> &mut FifMirrorState<FRAMES_IN_FLIGHT>;
    fn slot_dst<'a>(t: &'a mut Self::Target, tok: &FrameWriteToken) -> &'a mut MappedRange;
}

// ── Asset meta lane (ED4): K3 Deferred group column on the mesh asset entity (Q1a) ─
#[repr(C)]                                  // 36 B (arith.); read per MESH id
pub struct MeshMeta {
    index_count: u32,                       // DEAD = 0 ⇒ an inert draw
    geometry_slot: u32,                     // = the asset's dense slot
    index_type: u8, _pad: [u8; 3],
    aabb_min: [f32; 3], aabb_max: [f32; 3],
}
// Device lane: NonSend resource-column `MeshGpuLane { rows: OwningColumn<MeshGpu> }` (EK12), same slot.

// ── MeshRenderScratch additions (untracked, one K1 cohort) ────────────────────
// per batch: mesh_row: u32, caster_count: u32, aabb: [f32; 6], cursor: [u32; 2]
// fn bucket_slot(cur: &mut [[u32; 2]], mesh_id: u32, is_caster: bool) -> u32   // the ONE slot rule (ED6)

// ── UI ────────────────────────────────────────────────────────────────────────
#[repr(transparent)] pub struct ComputedPaintKey(u64);   // (StackIndex << 32) | global dfs_index; unique (debug_assert)
#[repr(C)] pub struct ComputedEffectiveClip { min: [f32; 2], max: [f32; 2] }  // 16 B; none = ±INF; REPLACE semantics
pub struct UiActionPresses<A: Actionlike> { pressed: BitSet256, _a: PhantomData<A> } // 32 B; cleared by ui_hit_test::<A>
pub struct UiPackLanes {                                  // resource-column, untracked
    instances: ScratchColumn<UiInstance>,
    keys: ScratchColumn<[u32; 2]>,                         // key = ComputedPaintKey order
    glyphs: ScratchColumn<GlyphInstance>,                  // atlas = font's bindless slot in flags bits 20..31 (UI6s)
}

// ── Window / host ─────────────────────────────────────────────────────────────
#[repr(C)] pub struct WindowSurface { extent: [u32; 2], scale: f32, _pad: u32, safe_area: [f32; 4] } // 32 B (arith.)
pub struct RenderCommit { composite_extent: [u32; 2], native_extent: [u32; 2], ssaa_scale: u32, path: ResolvedRenderPath }

// ── Camera ────────────────────────────────────────────────────────────────────
pub struct RenderView { uniform: ViewUniform }
#[component(storage = "bitset")] pub struct ActiveView;

// ── Kernel (boyko_ecs) ─────────────────────────────────────────────────────────
// EK3 change log (ED17): per opted-in ComponentId, kernel-internal untracked column of
#[repr(C)] struct ChangeRecord {
    id: EntityId,        // 8 B: EntityId wraps the usize id (entity_master.rs:141-142), arith.
    generation: u32,     // X-11: recycled ids must not alias
    tick: Tick,          // 4 B
    kind: u8,            // Removed | Despawned | Toggled
    _pad: [u8; 7],
}                        // 24 B (arith.)
// Written only in apply windows or under &mut EcsMaster. Truncated at driver step ② to the 2-frame horizon.

// K7 (physics-owned, ED15 shape filed to physics):
#[repr(C)] pub struct SpanRef<T> { ptr: NonNull<T>, len: u32, class: u8, _pad: [u8; 3] }   // 16 B; Copy
pub struct SegmentedColumn<T: Copy> {    // untracked ComponentPool (address-stable) + 25 class free lists (VmColumn<u32>)
    elems: ComponentPool, free: [VmColumn<u32>; 25], frontier: u32,
}
// EK15c: `impl RelationshipSourceCollection for Segmented` holds a SpanRef<Entity> (16 B vs Vec's 24 B header).

// K6′ (physics-owned amendment): dying entries become (slot: u32, died: Tick) = 8 B.
```

**Hot/cold split.**
- `MeshMeta` is read per mesh id; `MeshGpu` is cold on the CPU.
- The hit-test scan reads `ComputedRect` 16 + `ComputedPaintKey` 8 + `ComputedEffectiveClip` 16 = 40 B/row (arith.) and writes `Interaction` with `set_if_neq`.

**Drop order.** EK11 ranks, following `host.rs:67-80`:
1. PresentChain;
2. feature GPU bundles + `GBufferFrame` + `Renderer`;
3. UI/particle GPU;
4. asset device lanes (after a final `release_dense_group_with` at horizon = MAX following the device idle);
5. `RetiredGpuBuffers` drain;
6. `RhiContext`/`GpuDevice`;
7. `VulkanContext`;
8. log shutdown.

`FrameInFlight.token` must be `None` at teardown.

**Send/Sync of `SpanRef`-bearing components.** `unsafe impl Send + Sync`, with this SAFETY: the pointee is written only under `&mut` of the owner, in apply windows or relationship hooks; the base is address-stable; readers hold `&` components only in waves with no concurrent writer (declared access of the owning component id).

---

## 6. The system graph (MUST 2)

```
host (boyko_app runner, os-owned loop):
  loop { if !PresentWindow::pump(world)  → RawInput lane (EK7 OS sink; interim: drain+send_event) { break }
         app.update();  if AppExit { break } }
App::update_with_delta (kernel frame driver, outside any schedule by design):
  ⓪ profiler fold  ① Time  ② check_ticks (+ EK3 log truncation to the 2-frame horizon, EK5 resource ticks)
  ③ per-type event swap (EK7)  ④ Fixed × N  ⑤ Main  ⑥ Render (EK10; skipped when absent)
```

The pump and the fold stay outside the schedule, with evidence: `app.rs:709-720` (the swap) and `app.rs:680-690` (the fold).

**Fixed** (physics-owned):

| Set | Systems |
|---|---|
| `FixedSet::Physics` | the physics study's S1…S7 |
| `FixedSet::SceneSync` | rigid → `Transform` |
| `FixedSet::Snapshot` | `pack_gpu_transforms` |

**Main:**

| Set | Systems | Kind | Concurrency |
|---|---|---|---|
| `InputSet` | `input_fold_physical` → `ui_hit_test::<A>` (reads last frame's rect/key/clip; writes `Interaction`, `UiPointerState`, `UiActionPresses<A>`; `Commands::trigger`) → `action_fold::<B>` (one per `B`; reads `Option<Res<UiActionPresses<B>>>`) → rebind listeners | function | `action_fold` instances in parallel |
| `GameplaySet` ∥ `UiLogicSet` | user systems ∥ `ui_bar_apply`, `ui_clock_tick`, `ui_visual_tick`, `ui_tween_tick`, `ui_sprite_flipbook`, state charts | function | UI logic touches only UI components, so it runs parallel with gameplay |
| `UiBindSet` **(rev 2)** | `ui_bind_apply` (exclusive, EK2) | exclusive, serial | after `GameplaySet` and `UiLogicSet`: the HUD shows this frame's writes |
| `CameraSet::Resolve` | `propagate_transforms`, `resolve_active_view`, `visibility_sync` | function | per-root gang after physics K5b |
| `UiLayoutSet` | `ui_text_measure` → `ui_layout` → `ui_paint_walk`; `ui_world_project`, `ui_world_pick`, `ui_world_visibility` | function | parallel with non-UI systems |
| `RenderPrepareSet` | `sync_instance_model_cols` → `gather_mesh_draws` (+ caster partition, AABB, batch lanes, replayed re-walks; stamps `FrameInFlight.gather_tick` via a Send `GatherTick` resource copied in R0); `sync_vb_instance_ring` (`run_if(VB)`); lights; particles; resolves; `taa_advance`, `jitter_advance`, `motion_cam_advance`; `ui_render_gather`; `sdf_scene_sync` (after physics U5b) | all Send after ED4 | the gathers write disjoint resource-columns, so they form a parallel wave |
| `LogSet` | `log_drain_system` | function | — |

**Render** (dispatcher-solo, serial):

| # | System | Replaces |
|---|---|---|
| R0 | `render_acquire`: fence wait, then swapchain acquire/recreate → `FrameInFlight`; copies `GatherTick` into `gather_tick[slot]`; `None` when minimized | F6 |
| R1 | `render_retire` (exclusive): `release_dense_group_with::<AssetGroup>(completed_gather_tick, device-lane free)` per asset kind; `RetiredGpuBuffers` drain; `Local` scratch | F7 |
| R2 | `render_asset_upload::<Mesh/Texture/Material>` (per-frame drain), geometry backfill, material seed | boot one-shots + D-e |
| R3 | `render_grow` (grow + `FifMirrorState::invalidate` for every replaced buffer + descriptor repoint) | F8, F9 |
| R4 | `upload_mirror::<M>` × ~17 | F10, F13 uploads |
| R5 | `render_record`: declare the graph, record one primary command buffer including the UI pass, submit, present; takes the token | F13 |
| R6 | `render_publish` | F3, F15 |
| R7 | diagnostics, `run_if(resource_exists::<HostDump>)` | F14 |

- R1 onward run under `run_if(frame_acquired)`.
- Recording stays one system (R5): the frame graph solves barriers over the whole pass list, and Vulkan command pools are externally synchronised.
- Driver-side: `vkQueueSubmit`/present inside R5, and the debug-messenger callback.
- Parallel secondary command buffers are a later rung.

**Boot** (EK9): `StartupSet::{Device, Caps, Commit, Resources (slot-0 sentinels, ED5), Pipelines, User, PostUser}`.

**Teardown:** EK11.

---

## 7. The render-world question, decided on this kernel (MUST 3) — unchanged

**Per-instance CPU bytes per frame today** (arith., J):

| Step | Bytes |
|---|---|
| `sync_instance_model_cols` | reads `GlobalTransform` (64 B), writes 48 B |
| gather | reads 48 + 4 + 2 B, scatters 48 B |
| upload | `memcpy` 48 B |
| **Total** | ≈ 310 B/instance |

**Second world, retained (Bevy 0.15+):**
- adds ≥ 48 B read + 48 B write per synced instance;
- adds a `MainEntity ↔ RenderEntity` resolve (one `resolve_point`, 3–4 dependent loads; `query.rs:986-987`);
- adds observer-driven spawn/despawn sync;
- net ≥ +96 B/instance/frame on top of today's gather and upload.

It buys only sim(N+1) ∥ record(N): Bevy's 7–29% [S] was measured where record time is large. The R-schedule share here is unmeasured.

**Device-resident per-entity columns:** blocked (ED1).

**Decision:** one world. Latency is unchanged.

**The seam for a later overlap.**
- Render reads only resource-columns produced by `RenderPrepareSet` plus NonSend GPU state.
- Double-buffering those per FIF slot, plus a kernel overlapped schedule, gives pipelining without a second world.
- **Trigger:** Render-schedule CPU ≥ 30% of frame time on the owner's target scene, measured by SystemSpan zones.

---

## 8. Hot-path layout costs (MUST 4)

**Kernel storage facts used:**
- table columns are `base + row·stride`, with ticks in separate sub-regions (X-9);
- dense slots never move (`J:.../dense/dense_store.rs:7`);
- a point lookup is 3–4 dependent loads;
- enable toggles are tickless (`J:.../enable_tag_api.rs:79-81`);
- untracked ScratchColumns commit their data region only, at 64 KiB granularity (`scratch_column.rs:70-74`).

| Loop | Today | ECS form | Bytes / pattern (arith.) | Verdict |
|---|---|---|---|---|
| UI layout (change frames) | exclusive; random probes; relays all roots | function system; `Query::get`; dirty roots by up-walk; LIFO `Local` stacks with `truncate` | same probes per node; work ∝ dirty roots; +64 KiB resident per `Local` column | **equal per node, less total** |
| Paint walk (change frames) | 2 DFS copies | one DFS writing `ComputedPaintKey` + `ComputedEffectiveClip` via `set_if_neq` | +8 B + 16 B written per changed node | small cost, change frames only |
| Focus / pick (every frame) | whole-tree DFS, 4–6 random probes/node | linear scan of `Interaction` rows: rect + key + clip; branch-free argmax | 40 B/row sequential; 2,000 rows ≈ 80 KB → L2, prefetch-friendly | **better** |
| Text emission | none in production (B-2) | in `ui_render_gather`; glyph lookup in the font's CSR (ASCII ≈ 3 KB → L1) | sequential pushes | new work, bounded by changed text |
| UI pack | whole repack, 1.72 MiB capped staging | same repack, sorted by `ComputedPaintKey`; lanes grow in place | identical bytes | equal |
| Instance pack + gather | CpuExclusive; two gathers | Send; partition in one scatter; replayed re-walks; AABB from the loaded row | −48 B/caster scatter, −1 count/prefix pass; +1 ZST presence read per row in each re-walk; +4 B/bucket cursor; AABB ~12 FMA/instance from L1 | **better**, and parallel-capable |
| Upload (instances) | host `memcpy` | `upload_mirror` | identical | equal, +1 tick compare |
| Upload (CSM/atlas UBOs) | unconditional (336 + 1296 B/frame) | tick-gated | 0 B on unchanged frames | better |
| Culling (CSM caster bounds) | reads the caster ring | reads the caster sub-ranges | one ring fewer | better |
| Draw-list build | host `Vec` + transmute | recorder walks batch lanes | O(batches) | equal |
| Frame-graph arenas | `Vec` | lent `Local` ScratchColumns | same CPU work; +~1.1 MiB resident (ED13) | equal CPU; resident cost recorded |
| Transform propagation | exclusive; O(spatial) seeding | function system; `Changed` + `Removed<ChildOf>` | O(archetypes + changed) | **better** |
| Input fold | destructive pop; N resets | one lane read, one fold, per-A CSR walk | N−1 resets removed | better, and correct |
| Children iteration | `Vec` slice | EK15a: same `Vec`; EK15c: `SpanRef` (ptr + len) | identical access | equal |

**Where the claim fails today, and what closes it:** NonSend POD meta → ED4; removal/toggle observation → EK3; the exclusive tick window → EK2; resource ticks → EK5; registry-free ids → physics K1.

---

## 9. Kernel features, ordered and deduplicated (MUST 5)

"Owner" marks who lands the feature. Physics-owned features are consumed, and changes to them are filed as inputs (§13).

| # | Feature | What | Aliases merged | Consumers here | Owner |
|---|---|---|---|---|---|
| EK1 **(rev 2)** | **= physics K1 storage cohorts**, plus `impl Default for ScratchColumn<T>` (a width-1 cohort) so `Local<ScratchColumn<T>>` works | K1: "`ScratchCohort::reserve(width)`: registry-free scratch pools" (`M:…:466`). The LIFO frame is the existing `truncate` (X-17) | KF-scratch-column-for-type; registry-free id; id bands (C-8); **EK13 folded in** | every render lane, UI scratch/pack, input arrays, host scratch, propagation, save/load, CSR tables | **physics U1**; `Default` here |
| EK2 | Exclusive system context | `(last_run, this_run)` + `Local<S>` for exclusive bodies | UI KF-A; services KF3 | `ui_bind_apply` | here |
| EK3 **(rev 2)** | Structural change log | opt-in per component; 24 B `ChangeRecord` with generation; `Removed<T>`/`Toggled<T>` returning `RemovalWindow::{Complete, Overflowed}`; **2-frame horizon** (ED17); 0% when unread | UI KF-D; services KF1+KF2; render KF-structural-change-observation | UI discovery, lights, propagation, SDF sync, profiling projection, visibility consumers | here |
| EK4 | Enable initial polarity | `default_enabled` | KF-enable-initial-polarity | `LightEnabled` | here |
| EK5 | Resource change ticks (opt-in) | `#[resource(ticks)]` | UI KF-F; host K3 | `WindowSurface`, FIF mirrors, `UiViewport` | here |
| EK6 | Dense change-tick API | dense-aware `any_changed_since` | GK-2; UI KF-E | `ui_bind_apply`; asset value `Changed` (Q1a) | here |
| EK7 | Per-type event swap + OS sink | `#[event(swap = …)]`; `OsEventSink<E>` | host K1+K2 | `RawInput`; physics S7 keeps `WaitForFixed` | here (**phys needs**) |
| EK8 | Deferred trigger verb | `Commands::trigger` | — | UI | here |
| EK9 | Ordered startup stages | startup `ScheduleBuilder` | KF-startup-schedule; host K4 | boot | here |
| EK10 | `CoreSchedule::Render` | third slot | — | §6 | here |
| EK11 | Ordered NonSend teardown | rank-ordered eviction | host K5 | ED11 | here |
| EK12 | Owning scratch column | non-`Copy` column with drop glue | allocator `DropColumn` | `RetiredGpuBuffers`, device lanes | here |
| ~~EK13~~ | **deleted (W3)** | mark/rewind = `ScratchBuildView::truncate` (`views.rs:208`) on `Local` columns | allocator `FrameArena`; ledger `ScratchStack`, `LoadArena` | — | — |
| EK14 **(rev 2)** | **= physics K7** "Segmented dense column", prerequisite K3 (`M:…:473`); ED15's shape filed as input | pow2 classes; header held by the owner; relocate-on-grow; LIFO class free lists; no compaction | `RaggedColumn`, `CsrColumn`, `VmJagged`, AK-2 = KR-3 | EK15c only (fixed spans use CSR on EK1) | **physics S0** |
| EK15a **(rev 2)** | Ordered collection | `impl RelationshipSourceCollection for Ordered` (order-preserving remove); `Children` switches | services KF4 (order half) | `Children` (B-5) | here, **no prerequisite** |
| EK15b **(rev 2)** | Count-only collection | `CountOnly` impl: `add`/`remove` = ±1, no iteration; the target-despawn cascade is statically disabled; despawning with count > 0 becomes retire-at-0 | KF-relation-count | asset refcount (Q1a) | here, no prerequisite |
| EK15c **(rev 2)** | Relation storage on K7 | `Segmented` impl holding `SpanRef<Entity>` | KF-relation-collection-on-kernel-storage | `Children`, `Bindings`, every one-to-many target | here, after K7 |
| EK16 **(rev 2)** | Device-column meta fold (audit Stage 5), kernel only | fold `GpuColumnManager.meta` into `PoolBacking::Device` | KF-device-column-residency (kernel half only) | `GpuColumnManager` (test-only in production today) | here, last |
| RF1 **(new, render)** | FIF-mirror protocol | `FifMirror` + `upload_mirror` in `boyko_render` | KF-device-column-residency (FIF arm) | ED3 | render, not kernel |
| EK17 | Asset store dedup (Q1b only) | Retiring rows, lane split, `iter_mut` | — | — | here |
| EK18 | Default-excluded prefab entities | marker that matches no query unless named | services KF7 | `Prefab` | here (**phys needs**) |
| EK19 | Enable bits in the serialize/prefab/reflect seams | — | services KF9 | save/load | here (**phys needs**: `Simulated`/`Kinematic`) |
| EK20 | One StrInterner | — | services KF6 | `Name`, tags, log | here |
| EK21 | Kernel asset hot reload | — | services KF8 | `.ui`, meshes, textures | here |
| K6′ **(new)** | Horizon on the K6 release | dying entries stamped with a tick; `release_dense_group::<G>(horizon: Tick)` + `_with` visitor | rev 1 `Retiring{epoch}`, KF-assets-adopt-retiring (Q1a) | assets (ED16) | **physics U2** (input filed) |

**Consumed from physics without change:**
- K2 (untracked dense): candidate `GpuTransform3D` (O-7).
- K3 (dense groups): asset values (ED5).
- K4 (dense `par_iter`): tweens, `pack_gpu_transforms`, animation.
- K5a/K5b: propagation, culling.

**Dropped with evidence:**
- KF-B (X-2).
- KF-C (`EntityCommands::enable/disable` exist).
- KF-G (X-4).
- KF5 (the relation plus EK3 remove the hook queues).
- A same-frame event mode (ED9).
- `resource_scope`.
- `HeapVec` destinations.
- **EK13** (X-17).
- **the `Retiring{epoch}` component** (ED16).
- **the EK16 asset and resource arms**: those lanes are NonSend resource-columns (render rows 12, 17, 27); no form change is credited.

**Order.** Owner rule: finish the kernel first, then go by dependency and breadth.

1. **Physics-owned, in physics's rung order:** K1 (U1) → K2 + K3 + K6 [+ K6′] (U2) → K4 (U3) → K5a/b (P1/P2) → K7 (S0).
2. **This design's, interleaved by dependency:** EK15a → EK15b → EK2 → EK5 → EK3 → EK4 → EK6 → EK7 → EK8 → EK9 → EK10 → EK11 → EK12. The EK1 `Default` follows K1. EK15c follows K7. Then EK16 → EK18 → EK19 → EK20 → EK21. EK17 only if Q1 = (b).
3. **Subsystem rungs** start after the kernel set their prerequisites name. Red-first bug rungs may be pulled forward once their prerequisite has landed.

---

## 10. Public API (signatures only)

```rust
// boyko_ecs
pub enum CoreSchedule { Main, Fixed, Render }                                          // EK10
pub enum RemovalWindow<'a> { Complete(RemovedIter<'a>), Overflowed }                   // EK3 (ED17)
pub struct Removed<'s, T: Component>; impl Removed<'_, T> { pub fn read(&mut self) -> RemovalWindow<'_>; }
pub struct Toggled<'s, T: Component>; /* same shape */
// RemovedIter yields Entity (id + generation)
// #[component(storage = "bitset", default_enabled)]                                    // EK4
// #[resource(ticks)] ⇒ impl<R> Res<'_, R> { pub fn is_changed(&self) -> bool; pub fn last_changed(&self) -> Tick; } // EK5
// #[event(swap = "every_frame" | "wait_for_fixed")]                                     // EK7
#[derive(Clone, Copy)] pub struct OsEventSink<E: Event> { /* buffer ptr + dispatcher lane */ }
impl EcsMaster { pub fn os_event_sink<E: Event>(&self) -> OsEventSink<E>; }
impl<E: Event> OsEventSink<E> {
    /// # Safety: dispatcher thread; not inside Schedule::run or update_events; World alive.
    pub unsafe fn send(&self, e: E) -> Result<(), EcsError>;
}
impl Commands<'_, '_> { pub fn trigger<E: Trigger>(&mut self, target: Entity, event: E); }  // EK8
pub struct ExclusiveCtx { pub last_run: Tick, pub this_run: Tick }                          // EK2
pub enum StartupSet { Device, Caps, Commit, Resources, Pipelines, User, PostUser }            // EK9
pub trait NonSendTeardown: NonSendResource { const RANK: u16; }                              // EK11
impl<T: Copy + 'static> Default for ScratchColumn<T>;                                        // EK1 addition (after K1)
pub struct OwningColumn<T: 'static>;                                                         // EK12
pub struct Ordered(/* Vec<Entity> until EK15c */); impl RelationshipSourceCollection for Ordered;   // EK15a
pub struct CountOnly(u32); impl RelationshipSourceCollection for CountOnly;                         // EK15b
//   + `const ITERABLE: bool` on the trait (default true; CountOnly = false) gating the despawn cascade at compile time
pub struct Segmented(SpanRef<Entity>); impl RelationshipSourceCollection for Segmented;              // EK15c
// physics-owned, shape filed (ED15 / ED16):
pub struct SegmentedColumn<T: Copy>;                                                         // K7
impl<T: Copy> SegmentedColumn<T> {
    pub fn alloc(&mut self, len: u32) -> SpanRef<T>;
    pub fn push(&mut self, span: &mut SpanRef<T>, v: T);          // may relocate `span`
    pub fn remove_ordered(&mut self, span: &mut SpanRef<T>, idx: u32);
    pub fn free(&mut self, span: SpanRef<T>);
}
impl Commands<'_, '_> { pub fn release_dense_group<G: DenseGroup>(&mut self, horizon: Tick); }      // K6′
impl EcsMaster { pub fn release_dense_group_with<G: DenseGroup>(&mut self, horizon: Tick, f: impl FnMut(u32)); }
// #[component(prefab)] default-excluded marker                                                // EK18

// boyko_render (RF1)
pub trait FifMirror { /* §5 */ }
pub fn upload_mirror<M: FifMirror>(src: Res<M::Source>, dst: NonSendMut<M::Target>, frame: NonSend<FrameInFlight>);
pub fn render_acquire(/* NonSendMut<Renderer>, NonSendMut<PresentChain>, NonSendMut<FrameInFlight>, Res<GatherTick> */);
pub fn render_retire(world: &mut EcsMaster, ctx: ExclusiveCtx);                                // R1
pub fn render_record(/* NonSendMut<Renderer>, NonSendMut<FrameInFlight>, Res<MeshRenderScratch>, Res<UiPackLanes>, … */);
pub fn frame_acquired(f: NonSend<FrameInFlight>) -> bool;

// boyko_ui
pub fn ui_hit_test<A: Actionlike>(/* Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>,
    Res<PhysicalInput>, ResMut<UiPointerState>, ResMut<UiActionPresses<A>>, Commands */);

// boyko_rhi_vulkan
pub trait GraphStorage { /* arena lanes by index; no dyn */ }
pub struct FrameGraph<S: GraphStorage>;
pub unsafe fn render_gbuffer_frame<S: GraphStorage>(/* …, graph: &mut FrameGraph<S>, ui: Option<&UiPass<'_>> */);
```

There is no `dyn` on any per-frame path.

---

## 11. Multithreading model

- **Model:** multi-reader/multi-writer, partitioned by declared access. No `Mutex`/`RwLock`.
- **Shared in Main:** Send resource-columns (`MeshRenderScratch`, `UiPackLanes`, `MeshMeta` group column, `PhysicalInput`, `UiActionPresses<A>`, `GatherTick`) and components. Each has exactly one writer system per set (G-GRAPH).
- **Dispatcher-only:** every NonSend resident, all Render systems, and `OsEventSink` writes.
- **Synchronisation points:**
  - apply windows: commands, triggers, EK3 writes, relation hooks, K7 relocations, K6′ stamps;
  - the event swap;
  - the fence wait;
  - Main → Render.
- **New atomics:** none.
- **Race-freedom.**
  - The gathers write distinct `ResMut` targets.
  - The paint walk is the single writer of the two derived UI components.
  - FIF writes target the fence-released slot (R0 → R4 edge).
  - A K7 relocation happens only under `&mut` of the owner in an apply window, where no reader wave is live.
  - The asset release horizon is proven in ED16.
- **Send/Sync.**
  - ScratchColumn is Send.
  - `FrameInFlight`, `Renderer` and the device lanes are NonSend.
  - `OsEventSink` is Copy but not Send.
  - `SpanRef`-bearing components are `unsafe impl Send + Sync` (§5 SAFETY).

---

## 12. Migration rungs and gates (MUST 6)

### Gates on every rung

- **G-FORM** (`tests/ecs_form_census.rs`).
  - Scans non-test `crates/*/src` fields and statics typed `Vec|VecDeque|Box<|String|HashMap|HashSet|Arc<Mutex|OnceLock<Mutex`.
  - Checks against `tests/ecs_form_ledger.toml`, where each row is `{form, rung, tag}`.
  - Fails on a new site, a stale row, or a total different from the recorded count (anti-vacuity).
  - **A row may change only its `form` label when a rung changes its index space or its gate without removing storage.** Such a row is not counted as removed (W5).
- **G-LOOP.** Runner loop census; the allowlist only shrinks, 7 → 0.
- **G-GRAPH.** A headless `EnginePlugins` app. Checks every §6 system, set and edge, including `UiBindSet` after `GameplaySet`. Main exclusive systems must equal `{ui_bind_apply}`. Each resource-column has one writer.
- **G-ALLOC.** A counting allocator, steady frames 10..20. Budget only decreases, and the data path is 0. Anti-vacuity: UI nodes > 0 and instances > 0.
- **G-RES (new, record-only).** Committed bytes of kernel ScratchColumns after boot, recorded per rung (O2). This is not a pass/fail gate.
- **G-PERF.** Needs Q5 consent; otherwise the measurement is recorded as owed.

### Rungs

| Rung | Content | Prerequisite | Extra gate |
|---|---|---|---|
| K0 | Land G-FORM, G-LOOP, G-GRAPH at today's counts on the merged tree; G-ALLOC and G-RES baselines | — | counts recorded |
| **UI0 (rev 2)** | **Prerequisite owned by the UI lane / orchestrator, not a rung of this design.** Merge U onto J. Inputs filed (§13): re-argue AD10 reason 1, AD13 and `assert_table` (`U:render/gather.rs:147`) against J's `Or` fix (`J:.../filter.rs:1885`); **fold UI9 in** (X-11). Re-take the ledger UI rows. | — | as consumed here: U's UI tests green on J; an `EntityId`-recycling stress test proves the reap never removes from the wrong entity |
| K-EK* | one rung per feature, in §9 order; physics-owned ones are consumed at their physics rung | per §9 | per §17; trybuild or proptest; consumer-free anti-vacuity |
| IN1 | `RawInput` event; fold once; `RebindSession` on events; default quit; WM_CHAR/SETFOCUS/KILLFOCUS/MOUSELEAVE | EK7 | **red-first**: two `InputPlugin`s lose `just_pressed`; `Text` readable; G-FORM −1 |
| IN2 | `ActionState`/`InputMap`/`ACTION_NAMES` onto EK1 columns (CSR) and a Resource | K1 + EK1 `Default` | G-FORM −6; bit microbench unchanged |
| **IN3 (rev 2)** | `UiActionPresses<A>`; `ui_hit_test::<A>`; delete `ui_press` and the re-freeze | UI0 | a UI press visible in the same frame's `ActionState<A>`; **two-plugin gate**: `FlyAction` + `GameAction`, a button bound to `GameAction` index k, a click ⇒ `FlyAction` index k not pressed |
| IN4 | `OsEventSink` in WNDPROC; delete `InputRing` | EK7 | G-FORM −1; lane-full coalescing test |
| HO1 | `RenderCommit`, `LaunchConfig`, `Time.frame_count` | EK9 | G-FORM −6; SSAA test |
| HO2 | TAA/jitter/motion-cam as Main systems; VB ring scheduled (reads `MeshMeta` after AS1) | EK10, AS1 | G-LOOP −2; the 24 KB/frame allocation gone (G-ALLOC); G-GRAPH: `sync_vb_instance_ring` not exclusive |
| HO3 | `WindowSurface` (EK5); `UiViewport` derived | EK5 | red-first: no production writer of `UiViewport` |
| HO4 | Render schedule R0–R6; `WindowHost` → NonSend residents, including `ParticleGpu`; `FrameInFlight` | EK10, EK11 | G-LOOP → 0; G-GRAPH; minimized-window test; token debug_asserts |
| HO5 | diagnostics drivers → resources | HO4 | `BOYKO_HOST_DUMP` golden |
| HO6 | `VulkanContext` World-owned; debug → `boyko_log` | EK11 | teardown-order test |
| RE1 | delete `Gpu3dInstance` + `Render3dPlugin` (Q4); doc fixes | Q4 | census of `Gpu3dInstance` = 0 |
| **RE2 (rev 2)** | caster partition + batch lanes + `bucket_slot` shared by all walks; delete `CsmCasterScratch`, `DrawListScratch` | AS1 | **alignment proptest** (below); shadow **and** textured-GBuffer goldens byte-identical, except that coplanar equal-depth pixels are reported, never silently accepted; G-PERF gather microbench |
| **RE4 (rev 2)** | FIF mirrors (RF1) for ~17 uploads; delete G3 | EK5, HO4 | G-FORM −3; "unchanged CSM ⇒ 0 B uploaded"; **"grow with unchanged source ⇒ the fresh ring is written"** (O1) |
| RE5 | delete light seed and `LightTableDirty` | EK3, EK4 | red-first: a light toggled or removed after boot updates the table |
| **RE6 (rev 2)** | material and mesh-meta device lanes indexed by asset slot | AS3 | material edit after boot reaches the GPU (`material_table.rs:41`); G-FORM row label change only |
| RE7 | `RetiredGpuBuffers` on EK12 | EK12 | G-FORM −1 |
| RE8 | `FrameGraph<S>` with lent columns | K1 + EK1 `Default` | G-FORM −N; frame-graph tests on both impls; G-RES delta recorded |
| **RE9 (rev 2)** | render/app side of the one SDF field | **physics U5b** + physics's acceptance of the input; EK3 | red-first: a post-boot `SdfPrimitive` reaches physics and render |
| ~~RE10~~ | **deleted**: particle pools move with HO4, no form change | — | — |
| **AS1 (rev 2)** | ED4 lane split; **six** systems become Send | — | G-GRAPH: the six are not exclusive |
| **AS2–AS5 (rev 2)** | Q1(a): AS2 asset value as a K3 Deferred group + R1 release at the gather-tick horizon (K6′) → AS3 slot identity + slot-0 sentinels + the 4096 texture ceiling → AS4 count-only relation (EK15b) → AS5 staging/paths components + save/prefab/clear classification | Q1, K3, K6′, EK6, EK15b | slot-reuse proptest (a stale `Handle` is rejected; no slot is released before `completed_gather_tick` exceeds its death tick); first user asset gets slot ≥ 1; save/load round-trip keeps instance → asset links by `AssetPath`; G-FORM −12 |
| UI2 | plugins host everything (Q2); root caches → queries; viewport derived | UI0, HO3 | G-GRAPH; G-FORM −5 |
| **UI3 (rev 2)** | ordered `Children`; layout function system with `Local` LIFO columns | **EK15a, K1 + EK1 `Default`** | red-first B-5; Miri: index-range levels across pushes; G-PERF layout |
| **UI4 (rev 2)** | paint walk (`ComputedPaintKey`, `ComputedEffectiveClip`); `Interaction`-filtered scan; triggers; typed tags | EK8 | **oracle equivalence** against the old focus DFS **and** the old render DFS, on a corpus with overlapping roots, `StackIndex` ties, nested clips, and nodes with `Interaction` but no `Focusable` (same hovered entity, same pack order); G-PERF hit-test |
| UI5 | `BoundTo` relation (today's API); bar function system; bind on EK2 in `UiBindSet` | EK2, EK6 | a HUD bound to a dense component updates; the HUD shows the same frame's gameplay write |
| UI6 | world-UI function systems; hover triggers | EK3 | cull toggle repacks |
| **UI6s (new)** | **eDSL rung** (W7): the `ui_screen_px_range` leaf and the TEXT branch sample `g_sprites[slot]` (slot = flags bits 20..31) with `atlas_size` from the texture's dimensions; `g_atlas`/`g_atlas_ubo` retired; `px_range` stays one constant, and the font asset load **rejects** an atlas whose `distance_range_texels` differs from the bake constant (`atlas.rs:398` vs `:583`) with a logged error; stride stays 80 B | UI0 | extend `boyko_shaderdsl`, re-emit, re-splice; `ui_rect_edsl_sync` and `*_spv_sync` green; a `SHADER-VARIANT-MANIFEST.md` row; **no hand edit** (the census of GENERATED spans is unchanged except the re-emitted ones) |
| UI7 | fonts, sheets, textures become assets/handles | AS series or Q1(b), UI6s | B-4: FontId ≥ 1 samples its own atlas (a two-font golden) |
| UI8 | UI gather in Main + mirror in Render; glyphs live; legacy deleted | EK3, HO4, RE4 | red-first B-1 (despawn / `remove::<UiBackground>()` stops drawing the same frame); B-2 (text reaches the GPU) |
| ~~UI9~~ | **folded into UI0** (X-11) | — | — |
| UI10 | hot reload via EK21; delete `UiTreeView` and the `Arc<Mutex>` sinks; parse scratch on EK1 columns | EK21, K1 | G-FORM −4; round-trip keeps every component |
| SC1 | propagation function system | EK3, K1 | detach test; G-PERF seeding |
| SC2 | `visibility_sync` via `Entities` | — | G-FORM −1 |
| SC3 | `RenderView` + `ActiveView` | — | ≤ 1 active; uniform identical |
| SC4 | StrInterner; `ASSET_LAYOUTS` → `TypeIntern` | EK20 | G-FORM −2 |
| SC5 | serialize/reflect enable bits; save/load scratch on EK1 | EK19 | red-first: save/load keeps `Simulated`/`RenderEnabled` |
| SC6 | prefab entities | EK18 | instantiate keeps dense and enable memberships |
| LG1 | log targets as entities | — | env-level parity |
| LG2 | GPU zones → `Profiler`; `TelemetryStream` per Q4 | Q4 | G-FORM −3 |
| K-EK15c | `Children`/`Bindings` onto `Segmented` | K7 | proptest against a `Vec` model under random link/unlink/despawn; the 10k-append test records total copies ≤ 2n; Miri on `SpanRef` reads across a sibling span's relocation |

**RE2 alignment proptest.** Random archetypes with and without `ShadowCaster`, `MaterialHandle`, `PrevInstanceModelCol` and `GpuTransform3D`, mixed into the same mesh buckets. A test-only lane tags each scattered row with its entity. Assert, for every `i`:
- `ring[i]`, `material_ids[i]`, `material_tex[i]`, `prev[i]` (hwrt) and `vb_ring[i]` name the same entity;
- the caster prefix length equals `caster_count[b]`;
- the per-bucket multiset is unchanged against the pre-partition gather.

---

## 13. Respecting what exists (MUST 7)

**UI plans (U).**
- Kept: D31, AD10 (until the UI lane re-decides), U3, U4, U6, U8, U9, U10 (→ EK18), U11, U15.
- U6 is strengthened: fonts now join the one bindless table (UI6s).
- U5 is resolved as an asset (Q1a) or an EK1 resource-column (Q1b).

**Render plans (J, M).** Kept: R2 (passes are not entities), R3, R5, R6 (particles are not entities, N7), R7/R8. Reflections: probes become `RenderView`-bearing entities. RENDER-GRAPH-API's `Box<dyn RenderPassNode>` is not adopted.

**Capability-state model.** `LightEnabled`'s axis-2 gap is closed by EK4. `Camera.is_active` is removed.

**eDSL.** UI6s is the only shader change, and it goes through the generator.

**Audit.** Stage 5 = EK16 (kernel half). Stage 0's registry-free id = physics K1.

**Allocator (M, untracked, CHANGES REQUESTED).**
- `FrameArena` mark/rewind = the existing `truncate`.
- `DropColumn` = EK12.
- `HeapVec` and `boyko_memory` are rejected.

**Hand-offs to sibling plans (W8, C1, W9).** Each item is filed as an input. None is decided here, and the order column says which lands first.

| Owning plan | Datum or feature | Proposed input | Ordering |
|---|---|---|---|
| Physics study | K7 (= EK14) | ED15's shape: header held by the owner, pow2 classes, relocate-on-grow, LIFO class free lists, no compaction; prerequisite stays K3 for group-bank consumers | EK15c waits for physics S0; no UI rung waits |
| Physics study | K6 → K6′ | dying entries stamped with a tick; `release_dense_group::<G>(horizon)` + `_with` visitor; physics passes `this_run` (today's semantics) | AS2 waits for physics U2 |
| Physics study | `SdfField` | one field for physics + render, same `[SdfEdit; 16]` layout, named by physics | RE9 after physics U5b (`:795`) |
| Physics study | K1 | a `Default` for `ScratchColumn<T>` via a width-1 cohort | lands after U1 |
| Physics study | event swap | S7's events keep `WaitForFixed` under EK7's per-type policy | EK7 before physics E1 |
| Animation plan (AN1, RUNG-1) | pose bank | if K7 lands with ED15's shape, the bank may be a K7 consumer (AK-2 = KR-3). RUNG-1 §2.2's tick-page premise is stale on J (`scratch_column.rs:15-16` "UNTRACKED mode so the two tick sub-regions are reserved but never committed"). | animation's call |
| Transparency plan (R11) | `MaterialXGpu` second SSBO | could be a second NonSend lane indexed by the material asset slot (ED4/AS3), gated by the same `Changed` | transparency's call |
| UI lane | U → J merge (UI0) | re-argue AD10 reason 1, AD13, `assert_table`; fold the reap onto `EntityCommands::remove` (X-11) | before every UI rung here |

---

## 14. Owner questions (values/scope) vs architecture decisions (MUST 8)

**Owner questions:**
- **Q1 — Assets as entities.** This reverses S1 (2026-07-10, "assets are not entities").
  - (a) **Recommended.** Assets as entities. Consequences, stated before you answer:
    - assets depend on physics K3/K6′;
    - `Handle<T>` minting changes;
    - save files store asset entities as `AssetPath` only, and links are re-mapped by path;
    - prefab capture skips asset entities;
    - world-clear utilities skip `AssetRoot` unless asked;
    - despawning a referenced asset retires it when its count reaches 0.
  - (b) Keep `Assets<T>` and deduplicate internally (EK17).
  - ED4 lands under both.
- **Q2 — `EnginePlugins` composes the UI by default?** (a) Default on, **recommended**. (b) Opt-in `UiPlugins`.
- **Q3 — Multiplicity in v1.** (a) One window, one active view, one `ActionState<A>` per `A`, **recommended**. (b) Window and player as entities now.
- **Q4 — Delete dead paths** (`Gpu3dInstance` + `Render3dPlugin`, `TelemetryStream`, legacy UI scratch). (a) Delete, **recommended**. (b) Keep as allowed G-FORM rows.
- **Q5 — Timing permission** for G-PERF, the dispatch overhead and the §7 record share.

**Decided here, with the basis:**

| Decision | Basis |
|---|---|
| One world (ED1) | §7 |
| Render slot (ED2) | — |
| FIF protocol as a render feature (ED3) | kernel graphics-purity |
| Lane split (ED4) | X-3 |
| Ring kept; partition replayed (ED6) | X-15, write-combined pattern |
| Input as events (ED7) | edge-loss defect |
| UI function systems; global paint key (ED8) | X-6, X-16 |
| Triggers + typed presses (ED9) | X-1, W2 |
| Ordered `Children` on today's storage (ED10) | X-8, X-12 |
| World-owned device (ED11) | — |
| Lent FrameGraph storage (ED13) | — |
| Views (ED14) | capability ruling |
| One span primitive (ED15) | C1 |
| One deferred release (ED16) | W9 |
| 2-frame EK3 horizon (ED17) | W6 |
| Resource-owned columns are an ECS form | rank 6 of the vocabulary |
| `.bss` statics stay | N4/N6 |
| Particles stay non-entities | N7 |
| No GPU-resident dense | ED1 |
| `VmColumn` stays `pub(crate)` | K1 is its public face |
| Whole-UI repack kept | O-2 |
| One font distance range (UI6s) | keeps the 80 B stride, no new binding |

---

## 15. What this design does NOT change (MUST 9)

- **Physics:** everything in the physics study, including its K-feature definitions. Changes are inputs only (§13).
- **The kernel's storage kinds** and the `[pad | data | ticks]` layout.
- **Solver math, FMA, determinism.**
- **Shaders** except UI6s.
- **The frame-graph algorithm and pass list,** the FIF count, latency, and CPU/GPU overlap.
- **`InstanceModelCol` layout, the 48 B stride, the draw-ordered ring,** and the 80 B `UiInstance` stride.
- **Particles on the GPU; DDGI, HW-RT, VB geometry.**
- **The UI whole-repack policy and `UiVisual` storage.**
- **The profiler fold placement, the log/diag `.bss`, the reflect static table.**
- **`Time`/`FixedTime`/`State` and `state_chart!`.**
- **Editor selection.**
- **The OS pump and WNDPROC.**
- **Root order in the UI** (entity id, `focus.rs:214`).
- **`ComputedClip`'s author-owned semantics.**

## 16. Integration

- **boyko_ecs:** EK2–EK12, EK15a/b/c, EK16–EK21; `CoreSchedule::Render`; the EK3 hook in `remove_command.rs` / `set_enable_bit`; the event policy; the serialize seam. Physics-owned K1/K3/K6′/K7 are consumed.
- **boyko_app:** the runner becomes pump + update; `host.rs` fields become residents; `plugins.rs` gains `UiPlugins` (Q2) and render registration; `light_gate.rs` and `particle_gate.rs` are deleted.
- **boyko_render:**
  - `mesh_draw.rs`: partition, `bucket_slot`, re-walk replay;
  - `csm_caster.rs`: deleted;
  - `upload.rs` → RF1;
  - `material_table.rs`, `mesh_assets.rs`, `texture.rs`: lanes;
  - `light_system.rs`;
  - `ui/*` from U.
- **boyko_shaderdsl + `ui_rect.fs.hlsl`:** UI6s, re-emitted.
- **boyko_rhi_vulkan:** `FrameGraph<S>`, `render_gbuffer_frame` parameters, the WNDPROC sink, new message producers.
- **boyko_ui:** §4 UI rows; `ui_hit_test::<A>`; the paint walk.
- **boyko_fontbake:** none. The load-time check lives in the font asset loader.
- **boyko_input:** `RawInput`, the fold, EK1 CSR.
- **boyko_scene:** propagation, visibility, views, `asset_refs.rs` (Q1).
- **boyko_physics:** none from this design (§13 hand-offs).
- **boyko_log / boyko_diag:** `LogTarget`; the GPU channel.
- **Compatibility:** every new column is a ComponentPool; dense slots never move; `compact()` is forbidden on GPU-indexed stores (debug_assert).

## 17. Validation

**Unit and integration tests:**
- EK3:
  - `Removed<T>` sees a despawn and a remove in the same frame, with the correct generation;
  - a reader skipped for 2 frames gets `Overflowed` and one diagnostic;
  - no records are produced when nothing reads.
- EK4, EK5, EK7: as in rev 1.
- EK15a: an ordered remove keeps relative order.
- EK15b:
  - counts match relations after random link/unlink;
  - despawning a target with count > 0 retires it at count 0;
  - a trybuild fixture rejects `linked_spawn` on a `CountOnly` target.
- K6′ (at physics U2): `this_run` reproduces today's release set exactly.
- RF1: invalidation on grow.
- G-GRAPH, G-LOOP, G-FORM (with anti-vacuity), G-RES recorded.

**Proptests:**
- The EK3 log against a model under random structural ops and reader skips: exact within the horizon, `Overflowed` beyond it, bounded memory.
- EK15a/EK15c against a `Vec` model.
- UI: `ComputedPaintKey` is unique and the hit result equals the focus-DFS oracle; the pack order equals the render-DFS oracle.
- The caster partition alignment (RE2).
- K7: random alloc/push/remove/free against a `Vec<Vec<T>>` model; slack ≤ 2× per span; free-list reuse.

**Miri (Tree Borrows):**
- `OsEventSink::send`;
- `FrameGraph<S>` lent views;
- layout LIFO levels as index ranges across pushes;
- `SpanRef` reads while a different span relocates;
- the K6′ release visitor.

**Benchmarks (Q5):** the gather with and without partition and replay; the hit-test scan against the DFS; layout dirty-root against all-root; propagation seeding; Render dispatch overhead; §7 record share.

**debug_asserts:**
- `FrameInFlight.token` is `Some` in R1–R5, `None` at R0 entry and at teardown;
- `FifMirrorState` ticks are monotonic per valid slot;
- at most one `ActiveView`;
- `ComputedPaintKey` is unique;
- no `compact()` on a GPU-indexed store;
- EK3 writes happen only under `&mut EcsMaster`;
- asset slot < 2^32; texture slot < 4096;
- the slot-0 sentinel is at slot 0;
- the `OsEventSink` thread is the dispatcher;
- an occlusion `Block` hit is top-most.

**Edge cases:**
- a minimized window;
- 0 roots / 0 instances;
- a full lane;
- tick wrap (`check_ticks` clamps EK3, EK5, K6′ stamps and `gather_tick`);
- `EntityId` recycling (X-11, UI0);
- asset slot reuse before the fence;
- 10k children appended to one parent (amortised O(1), total copy ≤ 2n) and then despawned (O(n²) worst case, the same class as today's scan, about 2× traffic; timed by a stress test);
- a mixed caster/non-caster bucket;
- overlapping UI roots with equal `StackIndex`;
- an `Interaction` node without `Focusable`;
- a font atlas with a foreign distance range (load rejected).

## 18. Open questions (not decisions)

- **O-1:** closed (X-16). `ComputedClip` is author-owned; a new `ComputedEffectiveClip` is added.
- **O-2:** per-node cached GPU records vs whole repack. A perf fork, needs Q5.
- **O-3:** slot-indexed dense instance column with VS indirection. Needs a spike.
- **O-4:** whether `run_system`'s `Out` admits `DespawnPlan`/`UiParseReport`. Not verified.
- **O-5:** closed by ED17.
- **O-6:** whether the instance and FIF rings are write-combined. Not verified.
- **O-7:** whether `GpuTransform3D`'s ticks are read (a K2 candidate).
- **O-8 (new):** whether today's render-gather DFS uses the same root order as focus (`focus.rs:214`). The UI4 gate reports any difference as a latent G3 defect; not verified.
- **O-9 (new):** whether physics accepts ED15's K7 shape and K6′. Until it does, EK15c and AS2 are blocked, and nothing else is.
- **O-10 (new):** EK3's two-frame horizon is a value chosen by analogy to the event double buffer. If G-RES or the overflow diagnostic shows frequent rebuilds, a per-component horizon is the knob. Not measured.

---

## Change table (rev 1 → rev 2)

| Finding | Action | Where in rev 2 | Rev-1 text removed (verbatim excerpt) |
|---|---|---|---|
| **C1** EK14 ≠ physics K7; cannot hold `Children` | **FIX.** One primitive adopted under physics's name and prerequisite (K7, prereq K3). Header held by the owner (no key inside the primitive), pow2 classes, relocate-on-grow with ≤ 2n copies, LIFO class free lists, no compaction. Filed to physics as an input. `Children` order decoupled onto today's storage (EK15a, no prerequisite) and count-only split out (EK15b); K7 storage for relations is EK15c. Fixed-shape tables use CSR on EK1. | ED10, ED15, §9 EK14/EK15a/b/c, §10, §12 K-EK15c, §13 | "per-key variable-length spans: element column + (offset, len); key = dense slot or resource index"; "collections on EK14 storage with **order-preserving** remove (X-8); a `CountOnly` target"; "EK14 (with physics K7) → EK15" |
| **W1** focus scan does not reproduce today's order | **FIX.** Filter on `Interaction`; one global `ComputedPaintKey(u64)` = (StackIndex, global DFS index); new `ComputedEffectiveClip` with replace semantics; 40 B/row recomputed; oracle gate against both DFSs. O-1 closed. | X-16, ED8, §2, §5, §8, §12 UI4, §18 | "`ComputedPaintOrder(u32)` and the effective clip"; "`Query<(&ComputedRect, &ComputedPaintOrder, &clip), With<Focusable>>`"; "unique per root (debug_assert)"; "≈ 36–40 B/row" |
| **W2** non-generic `UiActionPresses` → phantom presses | **FIX.** `UiActionPresses<A>`, `ui_hit_test::<A>`; folds read `Option<Res<UiActionPresses<B>>>` (`res.rs:160`); two-plugin gate in IN3. | ED9, §5, §6, §10, §12 IN3 | "`UiActionPresses { pressed: BitSet256 }`"; "pub struct UiActionPresses { pressed: BitSet256 }           // resource-column, 32 B, cleared by ui_hit_test" |
| **W3** EK13 unusable from function systems | **FIX.** EK13 deleted. LIFO is the existing `ScratchBuildView::truncate` (`views.rs:208`) on `Local` columns; UI rows 19/29 and UI3's prerequisites updated. | X-17, ED8, §4 rows 19/29, §9, §10, §12 UI3/UI10 | "impl EcsMaster { pub fn scratch_frame<T: Copy>(&mut self) -> ScratchFrame<'_, T>; }"; "system-scratch: `Local` on EK13 frames (N3)"; UI3 prerequisite "EK15, EK13" |
| **W4** partition not replayed by re-walks | **FIX.** One `bucket_slot` function shared by all walks; `Option<&ShadowCaster>` in the one query; alignment proptest; textured golden added. Rev 1's "no per-row branch" corrected to a branch-free per-row select. | X-15, ED6, §5, §8, §12 RE2 | "so the choice is one cursor pick per archetype chunk, with no per-row branch" |
| **W5** EK16 partly a render wrapper; asset arm contradicts CPU-only dense | **FIX.** EK16 narrowed to the kernel Stage-5 fold. `FifMirror` is render feature RF1. Material and mesh device lanes are stated as NonSend resource-columns indexed by slot; G-FORM gives no removal credit; RE10 deleted into HO4. | ED3, §4 render 12/17/27, §9 EK16/RF1, §12 G-FORM/RE6/RE10 | "asset/resource arm (Stage 5 fold of `GpuColumnManager.meta`) + the FIF-mirror protocol"; "asset device lane (EK16 asset arm)"; "device-resident resource-column (EK16 resource arm)" |
| **W6** EK3 retention contradicts the bounded-memory proptest | **FIX.** Two-frame horizon; typed `Overflowed` plus diagnostic plus consumer rebuild; records carry the generation (X-11). O-5 closed. | ED17, X-11, §5, §9 EK3, §10, §17 | "Truncated at driver step ③ to min(reader cursors)"; "`ChangeRecord { entity: EntityId /*8 B*/, tick: Tick /*4 B*/, kind: u8, _pad: [u8;3] }  // 16 B`" |
| **W7** eDSL cost of B-4/UI7 unstated | **FIX.** Explicit rung UI6s: re-emit via `boyko_shaderdsl`; sample the existing bindless array via flags bits 20..31; retire `g_atlas`; one `px_range` enforced at font load; stride unchanged; sync gates plus a manifest row. | X-13, §5, §12 UI6s/UI7, §13, §15 | "If that shader is eDSL-owned, the change is made by re-emitting it (not verified which)." |
| **W8** rungs re-decide sibling plans | **FIX.** Hand-off table: animation pose bank, transparency `MaterialXGpu`, UI0 (UI lane-owned, now a prerequisite), RE9 ordered after physics U5b. | ED12, §4 animation, §12 UI0/RE9, §13 | "replaces the resource-owned ScratchColumn bank"; "Its `MaterialXGpu` SSBO becomes one more device lane of the material asset (EK16), not a second mirror."; "Physics renames its resource." |
| **W9** asset release horizon duplicates K3 | **FIX.** `Retiring{epoch}` deleted. Assets use K3 Deferred plus K6′ (release with a tick horizon), filed to physics; the horizon proof is in ED16. | ED5, ED16, §4 render 14/scene rows, §9 K6′, §12 AS2 | "Load state is component presence: no value = loading, `AssetFailed`, `Retiring { epoch }`, `Pinned`."; "component `Retiring{epoch}` on the asset (Q1a)" |
| O1 mirror state placement / invalidation | **FIX.** State owned by the target, with a `valid` mask; every buffer replace invalidates; grow test. | ED3, §5, §6 R3, §12 RE4 | "Per-slot state is `[Tick; FRAMES_IN_FLIGHT]` inside the mirror's resource-column."; "st: Local<FifMirrorState<FRAMES_IN_FLIGHT>>" |
| O2 resident floor uncounted | **ACCEPT + FIX.** ~1.1 MiB for the frame graph stated; 64 KiB/column floor; G-RES record-only metric; routed to POOL-SUBGRANULAR. AoS repack left as an unmeasured fork. | §2, ED13, §8, §12 G-RES | "| Draw-list build | … | equal; transmute deleted |" (unchanged row); rev 1 had no resident metric — nothing removed |
| O3 Q1(a) save/prefab/clear consequences | **FIX.** Classification stated in ED5 and in Q1's option text. | ED5, §14 Q1 | "It deletes four duplicated kernel mechanisms, and hot reload and serialize come free." |
| O4 slot-0 invariants | **FIX.** Slot-0 sentinels in `StartupSet::Resources`; texture slot ceiling 4096 (12-bit field). | ED5, §4 render 16, §12 AS3, §17 | "deleted: slot = asset dense slot; descriptor array sized to the high-water ceiling, grow = cold re-create" |
| O5 sixth NonSend system | **FIX.** Six systems named; HO2 depends on AS1. | X-3, ED4, §2, §12 AS1/HO2 | "**Five per-frame Main systems are CpuExclusive**"; "G-GRAPH: the 5 systems are no longer exclusive" |
| O6 `ui_bind_apply` in a parallel set | **FIX.** Own `UiBindSet` after `GameplaySet`/`UiLogicSet`; gated. | ED8, §6, §12 UI5 | "| `GameplaySet` ∥ `UiLogicSet` | user systems ∥ …; `ui_bind_apply` (exclusive, EK2) |" |
| New X-11 (recycling on J) | added; UI9 folded into UI0 | §0, §12 | "| UI9 | tween reap via Commands | UI0 | …" |
| New X-12, X-14 | EK15 re-based on the existing collection trait; EK1 consumed from physics U1 | §0, §9 | "| EK1 | **Storage cohorts + typed scratch** | … | **yes (K1)** |" (now "= physics K1") |
| ED2 trade-off (critic preserve) | rewritten to state what survives | ED2 | "Uploads after the fence wait go from a type-level proof to a schedule-edge proof" |

**Preserved as the critic asked:**
- ED1 with §7's numbers and the ≥ 30% trigger.
- X-3/ED4.
- ED6's partition itself.
- ED2's token as a witness, now stated more strongly.
- ED9's triggers.
- The pump and fold staying outside the schedule.
- X-4 and X-8.
- ED13's lent storage.
- G-FORM's anti-vacuity and the red-first rungs.
- Q1–Q5.

**Files relevant to this revision:**
- `D:\claude\BoykoEngine\docs\physics\PHYSICS-ECS-UNIFICATION-DESIGN.md` (K1/K3/K6/K7 at :466-473, D14 at :236-262, U5b at :795)
- `D:\wt\ui\crates\boyko_ui\src\interaction\focus.rs`
- `D:\wt\ui\crates\boyko_ui\src\interaction\action.rs`
- `D:\wt\ui\crates\boyko_ui\src\interaction\plugin.rs`
- `D:\wt\ui\crates\boyko_render\shaders\ui_rect.fs.hlsl`
- `D:\wt\joltab\crates\boyko_render\src\mesh_draw.rs`
- `D:\wt\joltab\crates\boyko_ecs\src\ecs\core\relationship\collection.rs`
- `D:\wt\joltab\crates\boyko_ecs\src\ecs\core\entity\entity_master.rs`
- `D:\wt\joltab\crates\boyko_ecs\src\ecs\core\component\scratch\views.rs`

---

# Critique log - pass 1

Critic: `architecture-critic`, one pass over rev 1. The verdict and findings are quoted verbatim. Each action is quoted verbatim from the rev 2 change table above (columns "Action" and "Where in rev 2"); the removed rev-1 text is in that table.

## Verdict

CHANGES REQUESTED. One critical remark (C1) and nine important ones (W1-W9). The one-world decision, the render-schedule shape, the NonSend lane split and the trigger-based UI messaging hold up against the code. The critical remark is about the kernel's variable-length-span primitive: the design defines it differently from the physics study, and it cannot hold Children as written. Because the owner's order is 'finish the kernel first', that primitive would be built wrong first. Trees checked: J @ d11962a9 for the kernel, render, RHI and app; U @ 615cda8f for the UI (every UI claim in the design was read from U, and I found none checked against J by mistake); M for the physics study, PHYSICS-ECS-UNIFICATION-DESIGN.md. Nothing was run.

## Blocking findings

### C1

**Finding (verbatim):**

C1. EK14 (the segmented column) is not the same thing as physics K7, and as defined it cannot hold EK15's Children. WHERE: section 9, rows EK14 and EK15, and the kernel order line. WHAT: the design says EK14 '= physics K7' and defines it as 'per-key variable-length spans: element column + (offset, len); key = dense slot or resource index'. It places EK14 before EK15 and before anything from physics. Physics defines K7 differently and gives it a prerequisite: M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:473 '| K7 | Segmented dense column | Per-entity variable-length ranges in a group bank | SoftBody (Q2) | animation bone arrays, UI text runs | K3 |'. K3 is physics' dense-group feature (:468). EK15 then puts relation collections on EK14, but the owner of a Children collection is a table-stored parent, not a dense slot or a resource index: J:crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120 'pub struct Children(Vec<Entity>);'. Its collection grows at arbitrary owners: J:crates/boyko_ecs/src/ecs/core/relationship/collection.rs:89-92 'fn add(&mut self, e: Entity) -> bool { ... self.push(e);' is O(1) amortized. The span consumers the physics study names (SoftBody particles, bone arrays) are fixed-shape once created. EK14 as written says nothing about how a span in the middle of the column grows, whether and how holes are compacted, how offsets are re-issued, or how a table-entity owner is keyed. CONSEQUENCE: K-EK14 is the first shared primitive on the kernel-first path, and it has only two ways to go. (a) It is built to physics' K7 shape: then K-EK15 and UI3 cannot use it, and ordered Children, the bind reverse index and the asset refcount relation all stall behind a second primitive. That recreates the five-names-for-one-shape problem of plans lens C-12, which the brief orders collapsed ('same kernel feature names where the need is the same'). (b) It is built to this design's shape: then physics K7 (which depends on K3) and EK14 diverge silently under one name. Either way a UI list that appends 10k children to one parent, a stated edge case in section 17, has no specified growth cost. CONFIDENCE: CONFIRMED (both definitions and both dependency chains quoted above). WHAT IS NEEDED: one span primitive agreed with the physics study. State its owner key (a dense slot, a table entity via a dense handle, or a resource index), its growth and relocation rule for a span that is not at the end of the column, its hole and compaction policy, and its dependency on K3 if any. Then put EK14 and EK15 in an order both studies accept, and state K7's name and prerequisites exactly as physics states them.

**Architect's action (rev 2 change table, row "C1 EK14 ≠ physics K7; cannot hold `Children`"):** **FIX.** One primitive adopted under physics's name and prerequisite (K7, prereq K3). Header held by the owner (no key inside the primitive), pow2 classes, relocate-on-grow with ≤ 2n copies, LIFO class free lists, no compaction. Filed to physics as an input. `Children` order decoupled onto today's storage (EK15a, no prerequisite) and count-only split out (EK15b); K7 storage for relations is EK15c. Fixed-shape tables use CSR on EK1.

**Where in rev 2:** ED10, ED15, §9 EK14/EK15a/b/c, §10, §12 K-EK15c, §13

## Non-blocking findings

### W1

**Finding (verbatim):**

W1 [IMPORTANT]. The focus hit-test scan in section 8 and the ComputedPaintOrder spec in section 5 do not reproduce today's ordering. The design scans 'Query<(&ComputedRect, &ComputedPaintOrder, &clip), With<Focusable>>' and takes an argmax of paint order, with ComputedPaintOrder 'unique per root (debug_assert)'. Today on U, the candidates are every node carrying Interaction, not every Focusable: U:crates/boyko_ui/src/interaction/focus.rs:224-225 'if let Some(rect) = world.get_component::<ComputedRect>(node).copied() && world.get_component::<Interaction>(node).is_some()'. The order is a global key across roots, with StackIndex first: :331 '(a.stack_index, a.paint_seq, a.entity.id().0) > (b.stack_index, b.paint_seq, b.entity.id().0)'. paint_seq is global because the roots are sorted and walked in one sequence (:214). CONSEQUENCE: implemented as written, (1) a button that has Interaction and OnClick but no Focusable can never be hovered or clicked, and (2) where two roots overlap (a HUD and a modal), a per-root-unique order ties or inverts, so the wrong root wins the hover. The UI4 oracle-equivalence gate would fail at implementation time. CONFIDENCE: CONFIRMED. NEEDED: make ComputedPaintOrder a global total-order key that folds in StackIndex and the root order, filter on Interaction, and recompute the bytes per row with StackIndex and FocusPolicy included. This also answers O-1: ComputedClip is author-owned, not computed (U:crates/boyko_ui/src/components.rs:186-189 'Clip rectangle for overflow. COLD, OPT-IN. / AUTHOR-OWNED in P1 (not computed)'). Today's effective clip REPLACES the inherited one rather than intersecting it (focus.rs:221-222 'let effective_clip = own_clip.or(inherited_clip);'). So a new derived component is required, and its semantics (replace or intersect) must match the oracle.

**Architect's action (rev 2 change table, row "W1 focus scan does not reproduce today's order"):** **FIX.** Filter on `Interaction`; one global `ComputedPaintKey(u64)` = (StackIndex, global DFS index); new `ComputedEffectiveClip` with replace semantics; 40 B/row recomputed; oracle gate against both DFSs. O-1 closed.

**Where in rev 2:** X-16, ED8, §2, §5, §8, §12 UI4, §18

### W2

**Finding (verbatim):**

W2 [IMPORTANT]. A non-generic UiActionPresses makes UI presses fire in every action set. The design has 'pub struct UiActionPresses { pressed: BitSet256 }' (section 5), read by 'action_fold::<A> (one per A)' (section 6). But OnClick carries a raw, untyped index: U:crates/boyko_ui/src/interaction/action.rs:3-5 'Each carries a dense Actionlike::index() as a raw u16 ... NOT a generic OnClick<A>'. Today the press is delivered only to the A of the one plugin that registered dispatch: U:crates/boyko_ui/src/interaction/plugin.rs:118 '.add_system(ui_dispatch_system::<A>)'. CONSEQUENCE: with FlyCameraPlugin (J:crates/boyko_app/src/fly.rs:113 'app.add_plugin(InputPlugin::<FlyAction>::new(fly_default_map()));') plus a game's InputPlugin<GameAction> — exactly the multi-plugin case ED7 exists to fix — clicking a button bound to GameAction index k also presses FlyAction index k. That is a new cross-set phantom press. CONFIDENCE: CONFIRMED (types and registration quoted). NEEDED: key the presses by action set, either UiActionPresses<A> or a single declared UI action type, and add a two-InputPlugin gate to IN3.

**Architect's action (rev 2 change table, row "W2 non-generic `UiActionPresses` → phantom presses"):** **FIX.** `UiActionPresses<A>`, `ui_hit_test::<A>`; folds read `Option<Res<UiActionPresses<B>>>` (`res.rs:160`); two-plugin gate in IN3.

**Where in rev 2:** ED9, §5, §6, §10, §12 IN3

### W3

**Finding (verbatim):**

W3 [IMPORTANT]. EK13's API cannot serve the function systems it is listed for. Section 10 gives 'impl EcsMaster { pub fn scratch_frame<T: Copy>(&mut self) -> ScratchFrame<'_, T>; }'. But UI row 19 ('system-scratch: Local on EK13 frames'), rung UI3 ('layout function system with Local scratch', prerequisite EK13) and ED8 make layout a function system, which has no &mut EcsMaster. CONSEQUENCE: K-EK13 ships an exclusive-only API, and UI3 either keeps layout exclusive (the section 2 target of 13 exclusive UI systems down to 1 is missed) or falls back to plain EK1 columns (EK13 has no consumer in UI). CONFIDENCE: CONFIRMED, as an internal inconsistency of the plan. NEEDED: either a SystemParam or Local form of the LIFO frame, or remove EK13 from the UI rows and prerequisites.

**Architect's action (rev 2 change table, row "W3 EK13 unusable from function systems"):** **FIX.** EK13 deleted. LIFO is the existing `ScratchBuildView::truncate` (`views.rs:208`) on `Local` columns; UI rows 19/29 and UI3's prerequisites updated.

**Where in rev 2:** X-17, ED8, §4 rows 19/29, §9, §10, §12 UI3/UI10

### W4

**Finding (verbatim):**

W4 [IMPORTANT]. The ED6 caster partition must be replayed by the index-aligned re-walks, and nothing gates that. The partition itself is sound. Casters are a subset of the main gather: J:crates/boyko_render/src/csm_caster.rs:190 '(Enabled<RenderEnabled>, With<ShadowCaster>),' against J:crates/boyko_render/src/mesh_draw.rs:1253 'Enabled<RenderEnabled>,', and there is no CPU culling in between. But the gather has secondary passes that re-walk the same query and reuse the primary scatter's offsets: :1316-1319 'a SECOND, index-aligned pass over the SAME query ... re-using the offsets gather_mixed_into just fixed' (material_tex), plus the hwrt prev ring (:1225-1226). The file warns that renumbering is silent: :1247-1248 'a filtered gather would silently RENUMBER the ring'. Those re-walk queries carry no ShadowCaster term, so they cannot choose the front or back cursor. CONSEQUENCE: as soon as a bucket mixes casters and non-casters, material_tex[i] and prev[i] describe a different entity from ring[i]. On a textured scene that means wrong textures, and under the temporal denoiser wrong motion vectors, while the shadow golden (the RE2 gate) stays byte-identical. CONFIDENCE: CONFIRMED that the walks are separate and lack the term. NEEDED: every re-walk replays the per-bucket two-cursor split (for example Option<&ShadowCaster> in its tuple, or a recorded per-archetype decision), plus a proptest that ring, material_tex, prev and material_ids stay index-aligned on mixed buckets.

**Architect's action (rev 2 change table, row "W4 partition not replayed by re-walks"):** **FIX.** One `bucket_slot` function shared by all walks; `Option<&ShadowCaster>` in the one query; alignment proptest; textured golden added. Rev 1's "no per-row branch" corrected to a branch-free per-row select.

**Where in rev 2:** X-15, ED6, §5, §8, §12 RE2

### W5

**Finding (verbatim):**

W5 [IMPORTANT]. EK16 is partly a render-local wrapper, and its asset arm contradicts 'dense is CPU-only'. Its 'FIF-mirror protocol' content is the FifMirror trait and the upload_mirror system, which section 10 places in boyko_render. The kernel must stay graphics-pure (J dispatcher_token.rs:33 per the render lens), so that half is not a kernel feature. The asset arm, under Q1a, gives the material value (a dense component) a 'device-resident' gpu lane. The design keeps both 'Dense storage is CPU-only (J:docs/DENSE-COMPONENTS-PLAN.md:6)' and 'No GPU-resident dense storage (ED1)'. So the material GPU lane can only be a NonSend resource-column indexed by slot. That is MaterialTable (G5) under a new name, yet the design claims the mirror is removed. The particle 'resource arm' is likewise today's BoundBuffers renamed. CONSEQUENCE: RE6/RE10 would count G-FORM rows as moved when no form changed, and K-EK16 has no kernel content to review. CONFIDENCE: CONFIRMED from the plan's own invariants. NEEDED: split EK16 into its kernel part (the Stage-5 fold of GpuColumnManager.meta into PoolBacking::Device) and a render feature (FifMirror). State plainly that the material and mesh device lanes are resource-columns indexed by dense slot.

**Architect's action (rev 2 change table, row "W5 EK16 partly a render wrapper; asset arm contradicts CPU-only dense"):** **FIX.** EK16 narrowed to the kernel Stage-5 fold. `FifMirror` is render feature RF1. Material and mesh device lanes are stated as NonSend resource-columns indexed by slot; G-FORM gives no removal credit; RE10 deleted into HO4.

**Where in rev 2:** ED3, §4 render 12/17/27, §9 EK16/RF1, §12 G-FORM/RE6/RE10

### W6

**Finding (verbatim):**

W6 [IMPORTANT]. The EK3 retention contract (left open as O-5) contradicts section 17's own proptest. Truncation to 'min(reader cursors)' with any reader that stays run_if-false for a whole session means the log never truncates. The design itself creates such readers: VB-only, hwrt-only and window-gated systems, and UI discovery. Yet section 17 promises 'no loss within the retention window, and bounded memory'. CONSEQUENCE: on a Forward-path session with UI churn, a Removed or Toggled reader registered by a VB-only system holds 16 B per structural change forever. That is exactly the unbounded side growth G-ALLOC is meant to forbid. CONFIDENCE: PLAUSIBLE; the shape follows from the proposed rule, and no reader census was done. NEEDED: decide the policy (a horizon cap with a declared loss signal such as the W0701 pattern, or cursors that advance even for skipped readers) before K-EK3. Also state whether ChangeRecord's generation-free EntityId can alias a recycled id within the window.

**Architect's action (rev 2 change table, row "W6 EK3 retention contradicts the bounded-memory proptest"):** **FIX.** Two-frame horizon; typed `Overflowed` plus diagnostic plus consumer rebuild; records carry the generation (X-11). O-5 closed.

**Where in rev 2:** ED17, X-11, §5, §9 EK3, §10, §17

### W7

**Finding (verbatim):**

W7 [IMPORTANT]. The eDSL cost of B-4 and UI7 is unstated, and it is not optional. The UI shader IS owned by the eDSL: U:crates/boyko_render/shaders/ui_rect.fs.hlsl:22 '// === GENERATED ui_instance_mirror BEGIN ==='. The atlas is a single hand-declared binding: :49 '[[vk::binding(1, 0)]] Texture2D    g_atlas         : register(t1);'. A GENERATED leaf reads a single atlas uniform: :149 'float2 unit_range = g_atlas_ubo.px_range / g_atlas_ubo.atlas_size;'. Per-font atlases therefore mean: a new field in the generated UiInstance mirror, a change to the ui_screen_px_range leaf in boyko_shaderdsl, new descriptor-layout declarations, a re-DXC, the *_edsl_sync and *_spv_sync gates, and a SHADER-VARIANT-MANIFEST row. CONSEQUENCE: UI7's gate ('FontId >= 1 samples its own atlas') cannot go green without an unplanned shader rung. Editing the HLSL by hand would break the generated-only rule. CONFIDENCE: CONFIRMED. NEEDED: an explicit eDSL rung before UI7, with those gates.

**Architect's action (rev 2 change table, row "W7 eDSL cost of B-4/UI7 unstated"):** **FIX.** Explicit rung UI6s: re-emit via `boyko_shaderdsl`; sample the existing bindless array via flags bits 20..31; retire `g_atlas`; one `px_range` enforced at font load; stride unchanged; sync gates plus a manifest row.

**Where in rev 2:** X-13, §5, §12 UI6s/UI7, §13, §15

### W8

**Finding (verbatim):**

W8 [IMPORTANT]. Several rungs re-decide decisions that sibling plans own. (a) Animation table, section 4: the pose bank 'replaces the resource-owned ScratchColumn bank', which overturns AN1 (M:docs/animation/ANIMATION-DESIGN-SPACE.md:175-177). Section 13 says the animation plan is only 'flagged, not edited'. (b) Transparency: 'MaterialXGpu SSBO becomes one more device lane of the material asset (EK16)' overturns TRANSPARENCY R11's second SSBO. (c) UI0 'Merge U onto J. Re-argue AD10 ... AD13' makes a UI-lane-owned merge and its storage rulings into a rung of this design. (d) RE9 renames physics' SdfField while the physics study's rung U5b ports physics_narrowphase_sdf (M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:795). CONSEQUENCE: two plans each authorise a different edit to the same datum, and whichever lands second breaks the other's gate. CONFIDENCE: CONFIRMED. NEEDED: turn each into a hand-off. Name the owning plan, file the proposed change as an input to it, and order RE9 relative to physics U5b.

**Architect's action (rev 2 change table, row "W8 rungs re-decide sibling plans"):** **FIX.** Hand-off table: animation pose bank, transparency `MaterialXGpu`, UI0 (UI lane-owned, now a prerequisite), RE9 ordered after physics U5b.

**Where in rev 2:** ED12, §4 animation, §12 UI0/RE9, §13

### W9

**Finding (verbatim):**

W9 [IMPORTANT]. The asset release horizon duplicates physics K3's deferred release. ED5 and AS2 introduce a Retiring{epoch} component and an AS-series slot-reuse guard so that a dense slot is not reused until a fence epoch passes. Physics K3 already defines deferred dense release for the same shape (M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:478 'RELEASE = Immediate | Deferred (D14)'; :486-487, the dying and free invariants). The only difference is the horizon: a fixed step for physics, a fence epoch for assets. CONSEQUENCE: two kernel mechanisms for 'do not reuse a dense slot before horizon H', against the brief's rule of one feature per need. CONFIDENCE: CONFIRMED. NEEDED: one deferred-release capability with a caller-supplied horizon, or a written reason why the two cannot merge.

**Architect's action (rev 2 change table, row "W9 asset release horizon duplicates K3"):** **FIX.** `Retiring{epoch}` deleted. Assets use K3 Deferred plus K6′ (release with a tick horizon), filed to physics; the horizon proof is in ED16.

**Where in rev 2:** ED5, ED16, §4 render 14/scene rows, §9 K6′, §12 AS2

### O1

**Finding (verbatim):**

O1. ED3's per-slot mirror state is placed in two different places, and invalidation on target recreation is missing. ED3 says the state is 'inside the mirror's resource-column', while section 10 has 'st: Local<FifMirrorState<FRAMES_IN_FLIGHT>>'. Neither says what happens when R3 render_grow or a device re-create replaces a target buffer while the source's tick is unchanged: the gate would skip the upload into a fresh, uninitialised ring. It is probably benign today because grows coincide with source changes, but make the state target-owned or generation-checked.

**Architect's action (rev 2 change table, row "O1 mirror state placement / invalidation"):** **FIX.** State owned by the target, with a `valid` mask; every buffer replace invalidates; grow test.

**Where in rev 2:** ED3, §5, §6 R3, §12 RE4

### O2

**Finding (verbatim):**

O2. The resident-memory floor of EK1 columns is not counted. An untracked ScratchColumn commits its data region at 64 KiB granularity (J:crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:70-73 'commits TWO tick sub-regions alongside the data at 64 KiB granularity ... a 192 KiB resident floor per column'; untracked leaves only the data region). FrameGraph has 18 release lanes (J:crates/boyko_rhi_vulkan/src/framegraph/graph.rs:126-205) at caps of 48/32/192 entries, so ED13 alone moves a few KB of Vec into about 1.1 MiB resident. The same applies to ActionState's four arrays and every UI Local. This is not a hot-loop regression, but the section 8 'equal' verdicts should say so, and it should be routed to the plan that owns the floor (physics section 13 cites POOL-SUBGRANULAR-PACKING-PLAN.md:48).

**Architect's action (rev 2 change table, row "O2 resident floor uncounted"):** **ACCEPT + FIX.** ~1.1 MiB for the frame graph stated; 64 KiB/column floor; G-RES record-only metric; routed to POOL-SUBGRANULAR. AoS repack left as an unmeasured fork.

**Where in rev 2:** §2, ED13, §8, §12 G-RES

### O3

**Finding (verbatim):**

O3. Q1(a) consequences are understated ('hot reload and serialize come free'). save_world walks every archetype (J:crates/boyko_serialize/src/save.rs:170 'for archetype in world.archetype_master().iter_archetypes() {'), so asset entities and instance-to-asset relation targets would enter save files. The dense value types need a classification (Ignore, or entity-mapping by AssetPath). Say how save/load, prefab capture and 'despawn all' treat asset entities before the owner answers Q1.

**Architect's action (rev 2 change table, row "O3 Q1(a) save/prefab/clear consequences"):** **FIX.** Classification stated in ED5 and in Q1's option text.

**Where in rev 2:** ED5, §14 Q1

### O4

**Finding (verbatim):**

O4. AS3 'slot = asset dense slot' must preserve the bindless reserved slot 0 (J:crates/boyko_render/src/bindless.rs:551 'capacity 5 has exactly 4 real slots') and the pinned default material at slot 0 (J mesh_draw.rs:1279 'the slot-0-never-retires invariant'). A fresh dense store hands slot 0 to whatever is inserted first.

**Architect's action (rev 2 change table, row "O4 slot-0 invariants"):** **FIX.** Slot-0 sentinels in `StartupSet::Resources`; texture slot ceiling 4096 (12-bit field).

**Where in rev 2:** ED5, §4 render 16, §12 AS3, §17

### O5

**Finding (verbatim):**

O5. The AS1 gate names 5 NonSend systems. A sixth, sync_vb_instance_ring_system, reads NonSendRes<Assets<MeshGpu>> for geometry_slot (J:crates/boyko_render/src/mesh_draw.rs:1170 and :535-536 'mesh_assets.get_by_index(mesh_id).map_or(VB_GEOMETRY_RESERVED_SLOT, |m| m.geometry_slot)'). HO2 schedules it in RenderPrepareSet, so it must read MeshMeta (or disappear under AS3) or it stays CpuExclusive. G-GRAPH's exclusive allowlist would catch it, but add it to the text.

**Architect's action (rev 2 change table, row "O5 sixth NonSend system"):** **FIX.** Six systems named; HO2 depends on AS1.

**Where in rev 2:** X-3, ED4, §2, §12 AS1/HO2

### O6

**Finding (verbatim):**

O6. In section 6, 'GameplaySet || UiLogicSet' contains ui_bind_apply, which is exclusive, so that set cannot overlap anything. It also leaves unstated whether bind runs after the gameplay writes it displays; if not, the HUD lags one frame nondeterministically. State the edge.

**Architect's action (rev 2 change table, row "O6 `ui_bind_apply` in a parallel set"):** **FIX.** Own `UiBindSet` after `GameplaySet`/`UiLogicSet`; gated.

**Where in rev 2:** ED8, §6, §12 UI5

---

# Critic's preserve list

1. ED1 (one world) is costed on THIS kernel, not by analogy: section 7's 310 B/instance, the at-least +96 B/instance an extract copy would add, and the one-world overlap seam gated on a measured Render-schedule share of at least 30%. Keep this, and keep the trigger condition.
2. X-3 and ED4 are a real, verified finding. NonSendRes<Assets<MeshGpu>> at J:crates/boyko_render/src/mesh_draw.rs:1255, csm_caster.rs:192 and :453, and asset_refcount.rs:92 and :402 makes universal access, hence SystemKind::CpuExclusive (J system_kind.rs:11 'CpuExclusive is resolved from access().is_universal()'), even though the gather reads only POD (:1291-1292). The Send meta lane holds under either answer to Q1.
3. ED6 partition validity. Casters are a strict subset of the main gather (csm_caster.rs:190 against mesh_draw.rs:1253), there is no CPU culling in between, and ShadowCaster presence is constant per archetype. That makes the second count/prefix/scatter pass genuinely deletable. Keep the partition; fix only the re-walks (W4).
4. ED2 keeps FrameWriteToken as a witness. The token is lifetime-free (J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:1043-1044 'pub struct FrameWriteToken { slot: usize, }'), so Option<FrameWriteToken> in NonSend FrameInFlight is storable. Every mirror borrowing &FrameWriteToken still cannot write before acquire, and R5 consumes the token by value, so the type-level proof survives better than the design's own trade-off paragraph admits.
5. ED9, triggers rather than a same-frame event mode: grounded in the verified next-frame event visibility (J event_reader.rs:78-79), Up propagation (J observer_api.rs:442-449) and the between-wave apply window (J schedule.rs:650-679). No new kernel event mode is introduced.
6. The pump and the profiler fold stay outside the schedule, with evidence (J app.rs:680-690 on the fold; app.rs:709-720 on the swap).
7. The corrections X-4 (J remove_command.rs:87-88 dense_remove_and_fire) and X-8 (J collection.rs:98-101, the swap_remove position scan) are verified. KF-G was correctly dropped.
8. ED13 lends ScratchColumns through a storage trait rather than exposing VmReservation or adding a boyko_memory primitive. This respects the owner's rejection of a bespoke column (K2).
9. G-FORM's anti-vacuity total count, and the red-first rungs (IN1 edge loss, B-1, B-5).
10. The owner questions Q1-Q5 are genuinely values or scope. Q1 reverses an owner-approved direction (S1) and ships a fully specified fallback, EK17.

---

# Critique log - pass 2 (2026-09-11)

Critic: `architecture-critic`, pass 2, scoped to the rev 1 → rev 2 delta plus the decided options (`ENGINE-RUNTIME-ECS-DECISIONS.md`: Q1 (a), Q2 (b), Q3 (b), Q4 (a), Q5 on the owner's word). The verdict, every finding, every decided-option impact and every confirmed fix are quoted verbatim from the critic's output. Each architect action is quoted verbatim from the rev 3 changelog (columns "Action" and "Section"), which is part of the rev 3 patch below. Pass-2 finding ids (B1-B5, N1-N8, O1-O3) are a new series: pass 2's O1-O3 are not pass 1's O1-O6.

## Verdict

CHANGES REQUESTED (pass 2, scoped to the rev 1 → rev 2 delta plus the decided options). Most pass-1 findings are resolved against the code: W1 for reachable nodes, and W2, W3, W4, W5, W7, W8, O1-O6. The C1 fix holds as a filing: there is one primitive under physics's name and prerequisite, the header is held by the owner, and growth, holes and compaction are specified. Rev 3 of the physics design did not touch K7; it appears only at M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:313, :448, :473 and :844. Five blocking items remain:
- B1, B2: the W9 fix (assets on K3 Deferred plus K6′) was never re-checked against physics rev 3. Group columns carry no change ticks, and the release API it amends is gone.
- B3: a new defect in ED16's data structure frees an asset's GPU memory while the GPU is still reading it.
- B4: Q3 makes IN2 impossible without a kernel feature that §9 does not list.
- B5: Q2 makes G-GRAPH, the gate on every rung, and R5's signature impossible to satisfy as written.
Trees read: J @ d11962a9 (kernel, render, RHI, input, app); U @ 615cda8f (UI only); M (docs, and the physics rev 2 plus the rev-3 patch). No cargo and no timing were run.

## Blocking findings

### B1

**Finding (verbatim):**

B1 - ED5/ED3/RE6/EK6: asset change tracking relies on change ticks that physics K3 group columns do not have. The W9 fix is wrong against physics rev 3. WHERE: ED5 'Change tracking. `dirty_gen` / `install_epoch` / `free_epoch` become kernel `Changed`/`Added` on the dense value (EK6)'; ED3 'upload per row on `Changed<T>` of the asset's dense value (Q1a)'; §4 render row 12 'gated by `Changed` on the material value (Q1a)'; §9 EK6 consumer 'asset value `Changed` (Q1a)'; RE6's gate. PHYSICS: M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:467 '| K2 | Untracked dense, compile-time sound | See the table below | All group columns |'; rev-3 :1566 '// one untracked ComponentPool per column id (K1 contiguous ids, K2 no tick pages); DEAD-initialised'; :1695 '`Changed<T>` dense | `filter.rs:1492` | same' (const-assert TICKS_TRACKED). Writes go only through `DenseGroupMut` views (:1726 `pub fn view<T: GroupColumn<Group = G>>(&self) -> TypedDenseView<'_, T>;`), and those stamp nothing. Rev 3 also allows membership changes only through anchor transitions (:1662 'Membership changes only through binder transitions (D13)'; :1663 '`insert::<T>`/`remove::<T>`/`Bundle` const-assert `!IS_GROUP_COLUMN`'). So 'Load state is component presence: no value = loading' cannot be expressed: the value column exists with DEAD bytes from the moment the anchor attaches. CONSEQUENCE: `Changed<Material value>` fails to compile, so RE6's red-first 'material edit after boot reaches the GPU' cannot go green. An implementer who falls back to per-frame re-upload pays every asset lane every frame. One who reads ticks through an id-based path hits physics B1's release-build fault on uncommitted tick memory. CONFIDENCE: CONFIRMED (plan text against the physics rev-3 text). NEEDED: pick a change signal that group columns can carry: a per-group dirty lane written by the install/edit path, an edit event, or a physics input asking for a tick-stamping group variant. Then state: the anchor component of each asset kind; which system writes the value, through `DenseGroupMut<G>`; and how 'loading' is represented (DEAD value or no anchor).

**Architect's action (rev 3 changelog, row "B1 group columns carry no ticks"):** FIX. The kernel group edit log EK6g (opt-in, own node, beside `DenseGroupStore`). Anchors `MeshAsset`/`MaterialAsset`/`TextureAsset` at mint. Writers named per kind through the `AssetWrite` facade. Loading = DEAD + `AssetReady` disabled. Null descriptor on anchor. Q1's `Changed<T>` premise refuted for the three group kinds

**Where in rev 3:** X-18, ED3, ED5, §4.2, §5, §6, §9, §10, RE6, AS2

### B2

**Finding (verbatim):**

B2 - ED16/K6′ amends an API that physics rev 3 deleted, and its physics horizon is wrong on this kernel. PHYSICS rev 3: :1459 'No physics system issues a command. `Commands::release_dense_group` is **deleted**.'; :1655 '`DenseGroupMut<G>::release_dying()` ... `EcsMaster::release_dense_group::<G>()` exists for chain-less worlds and is refused once a chain head is registered'; :1483 'That call panics (cold) if any `DenseGroupMut<G>` has ever been initialised in the world'. DESIGN: §10 `impl Commands<'_, '_> { pub fn release_dense_group<G: DenseGroup>(&mut self, horizon: Tick); }` and `impl EcsMaster { pub fn release_dense_group_with<G>(&mut self, horizon: Tick, f: impl FnMut(u32)); }`; R1 is `render_retire(world: &mut EcsMaster, ctx)`. CONSEQUENCE 1: by B1 the asset value is written through `DenseGroupMut<G>`, which sets `chain_head_registered` (:1570). R1's `EcsMaster` release then panics at the first asset retire. CONSEQUENCE 2: ED16's 'Physics passes its S6 `this_run`, which is today's "release all dying" semantics' is wrong twice. First, physics now releases at the head of S1. Second, on this kernel apply-window structural stamps land at `this_run + 1`: J:crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:426 `let _apply_window_tick = world.bump_change_tick();`, :430 'apply-window tick must be exactly one past the frame-start this_run'. So a horizon of `this_run` keeps dying at S1 every slot removed in a Fixed apply window earlier in the same run. That is physics's B2 ghost body, reopened. §17's 'K6′ (at physics U2): `this_run` reproduces today's release set exactly' is false. CONFIDENCE: CONFIRMED. NEEDED: re-file K6′ against rev 3's shape: a horizon-plus-visitor form of `release_dying` on `DenseGroupMut<G>`, or a per-group opt-out of the exclusive-release refusal. Make R1 a Render function system that holds `DenseGroupMut<G>` and `NonSendMut<device lane>`. State that physics's horizon is 'all' (MAX), not `this_run`.

**Architect's action (rev 3 changelog, row "B2 K6′ amends a deleted API; wrong physics horizon"):** FIX. Re-filed on `DenseGroupMut` (`DeferredStamped`, `release_dying_before`, `release_dying_with`, teardown-token form). Physics horizon = "all" (`release_dying()` unchanged). R1 = function systems

**Where in rev 3:** X-19, ED16, §6, §9, §10, §13

### B3

**Finding (verbatim):**

B3 - ED16/§5/§6 R0-R1: the asset release horizon is read after R0 has overwritten it, so assets are freed while the GPU still reads them (new defect). WHERE: §5 `gather_tick: [Tick; FRAMES_IN_FLIGHT], // tick at which RenderPrepareSet's gather ran for the frame in slot s` and `completed: Option<usize>, // slot whose fence R0 last waited on`; R0 'fence wait … copies `GatherTick` into `gather_tick[slot]`'; R1 releases at `completed_gather_tick`. CODE: the fence R0 waits on is the slot the new frame reuses. J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:321 `let fence = self.frames[self.frame_index].in_flight;`; :333 `Ok(FrameWriteToken { slot: self.frame_index })`. So `completed == slot`, and `gather_tick[completed]` already holds frame N's own gather tick g_N when R1 reads it. The correct value is g_{N-2}. CONSEQUENCE: an asset whose last reference dropped between frame N−1's gather and frame N's gather is the ordinary despawn case. Its death stamp t satisfies g_{N-1} < t < g_N, so R1 of frame N releases it, and ED16's visitor frees its VkBuffer/VkImage, while frame N−1 is still executing and names that slot. That is a GPU use-after-free under FIF=2: validation errors or device loss. The AS2 proptest ('no slot is released before `completed_gather_tick` exceeds its death tick') would pass, because it tests the wrong tick. CONFIDENCE: CONFIRMED (plan text against frame_driver.rs). NEEDED: R0 must latch the completed frame's gather tick before it overwrites the slot's entry, for example with a dedicated field. The proptest must model FIF-slot reuse. Note: the proof itself is sound on this kernel's coarse ticks, because all Main systems share one `this_run` and apply windows stamp `this_run + 1` (schedule.rs:331, :426). Only the read order is wrong.

**Architect's action (rev 3 changelog, row "B3 horizon read after overwrite"):** FIX. `submitted_gather` written only on submit; `completed_gather` latched by R0 after the fence wait. Proof restated for any stamp ≤ g. FIF-slot-reuse proptest with a mutation check

**Where in rev 3:** X-20, X-25, ED16, §5, §6, §17

### B4

**Finding (verbatim):**

B4 - Q3 (decided): IN2/IN3/ED7 need `ActionState<A>`/`InputMap<A>` as components on player entities, which needs a kernel feature missing from §9. CODE: J:crates/boyko_macros/src/component.rs:1050 '`#[derive(Component)]` never handles generics (`input.generics` is not read anywhere in this module)'; :369-372 `impl ... Component for #name { ... static ID: ::std::sync::OnceLock<...>`. J:crates/boyko_input/src/action/state.rs:432-434 'NOT `#[derive(Resource)]`: the derive caches the id in a `static` inside the generic `resource_id()` body, which collapses every `A` onto one id (rust#22991)'. The kernel's TypeId-keyed mint exists for resources only (J:crates/boyko_ecs/src/ecs/core/resources/resource_type_registry.rs:94 `pub fn resource_id_for<T: Resource>() -> ResourceId {`); components have only `register_new<T>` (component_registry/mod.rs:920). CONSEQUENCE: IN2 as written ('onto EK1 columns (CSR) and a Resource') contradicts Q3(b). The obvious rewrite, a hand-written `impl<A> Component for ActionState<A>` with a static id, gives `ActionState<FlyAction>` and `ActionState<GameAction>` one ComponentId and one column. The fly camera (J:crates/boyko_app/src/fly.rs:78 `fn quit_on_action(actions: Res<ActionState<FlyAction>>, ...)`) would read the game's bits. The per-player form also turns the 4 `Box<[f32]>` fields into per-entity Box G-FORM rows. A per-player InputMap as 2 EK1 columns costs 2 × 64 KiB resident per player per action set. CONFIDENCE: CONFIRMED. NEEDED: a TypeId-keyed component id mint (with the derive-installed tables) in the kernel-first list, or a non-generic route (for example concrete per-set components emitted by `#[derive(Actionlike)]`). Also re-cut ActionState storage (fixed arrays bounded by BitSet256's 256, or K7) and InputMap storage (one shared per-A CSR bank with a per-player header).

**Architect's action (rev 3 changelog, row "B4 no generic component mint"):** FIX. EK22 `component_id_for` over `TypeIntern`, derive accepts generics. `ActionState` fixed `A::Lanes`. `InputMap` = profile + shared per-A bank. Rev 2's `Staged<A::Cpu>` had the same defect

**Where in rev 3:** X-21, ED20, §5, §9, IN2

### B5

**Finding (verbatim):**

B5 - Q2 (decided, opt-in `UiPlugins`): G-GRAPH (the gate on every rung, landed at K0) and R5's signature cannot be satisfied as written. WHERE: §12 G-GRAPH 'A headless `EnginePlugins` app. Checks every §6 system, set and edge, including `UiBindSet` after `GameplaySet`. Main exclusive systems must equal `{ui_bind_apply}`'; §10 `render_record(/* NonSendMut<Renderer>, NonSendMut<FrameInFlight>, Res<MeshRenderScratch>, Res<UiPackLanes>, … */)`; G-ALLOC anti-vacuity 'UI nodes > 0'. CONSEQUENCE: with UI opt-in, EnginePlugins registers no `ui_bind_apply`, so G-GRAPH is red by construction. `UiPackLanes` is inserted only by UiPlugins, so in every UI-less app, now the default, R5's required `Res` fails validation and no frame is recorded or presented. G-ALLOC cannot reach its anti-vacuity condition without UiPlugins. CONFIDENCE: CONFIRMED (plan text against the decision). NEEDED (mechanical): run G-GRAPH as two apps: EnginePlugins alone, asserting no UI system and an empty exclusive set; and EnginePlugins plus UiPlugins, asserting `{ui_bind_apply}` and the UI edges. Make R5's UI inputs optional or contributed by the UI plugin. Compose UiPlugins in the G-ALLOC app.

**Architect's action (rev 3 changelog, row "B5 G-GRAPH / R5 / G-ALLOC vs opt-in UI"):** FIX. G-GRAPH as two apps. R5's UI inputs are `Option`. G-ALLOC composes `UiPlugins`

**Where in rev 3:** ED19, §6, §10, §12

## Non-blocking findings

### N1

**Finding (verbatim):**

N1 - ED17: the overflow test as worded fires on ordinary reads. Line 475: 'A reader whose `last_run` is older than the oldest retained record gets `RemovalWindow::Overflowed`'. Take a reader that runs every frame, last_run = T, with the log holding a record stamped T+1 (Main's apply windows stamp `this_run + 1`, schedule.rs:426). T is older than the oldest retained record, so the read returns Overflowed. CONSEQUENCE: implemented literally, every consumer (UI repack, light rebuild, propagation, SDF resync) runs its full rebuild after any structural change, which defeats EK3. §17's same-frame test catches it, so it is not blocking. Also: the horizon counts frames, so a Fixed-schedule reader overflows every time the frame rate exceeds twice the fixed rate. None is listed today. CONFIDENCE: CONFIRMED (text). NEEDED: overflow iff last_run is before the truncation watermark (the horizon start, or the newest dropped record); state the rule for Fixed readers.

**Architect's action (rev 3 changelog, row "N1 overflow fires on ordinary reads"):** FIX. Exact watermark rule over the inclusive window. Fixed-reader retention rule

**Where in rev 3:** X-30, ED17, §17

### N2

**Finding (verbatim):**

N2 - EK15b: 'despawning an asset with count > 0 becomes retire when the count reaches 0' has no mechanism. Hooks cannot veto a despawn: J:crates/boyko_ecs/src/ecs/core/relationship/mod.rs:24-27 'A relationship-maintenance hook's only world handle is `DeferredEcsMaster`, which exposes NO `&mut`-into-storage method'. With the cascade statically disabled, a user `despawn(asset)` goes ahead. Instances keep the FK and the derived slot component. After the K6′ release and slot reuse they draw the next tenant's mesh or material. CONFIDENCE: CONFIRMED (API), PLAUSIBLE (frequency). NEEDED: a refusal or redirect in the despawn command path, which runs under `&mut EcsMaster`, for a CountOnly target with count > 0; or an asset-only retire API plus a debug refusal.

**Architect's action (rev 3 changelog, row "N2 no despawn-veto mechanism"):** FIX. Redirect in `delete_entity_core` via `COUNTED_TARGET`; `DESPAWN_AT_ZERO`; sentinel refusal

**Where in rev 3:** X-22, EK15b, ED5, AS4

### N3

**Finding (verbatim):**

N3 - ED5 counts only instance → asset references. Asset → asset references are raw slots. J:crates/boyko_render/src/material_table.rs:2-3 'widened the CPU authority element from a bare `MaterialGpu` to `Material { gpu, textures }`'; :515 '`mat.textures.albedo = slot`'. The UI6s font atlas slot and sprite-sheet → texture have the same shape. CONSEQUENCE: under Q1(a), a texture named only by a material, or unlinked by its last widget while a material still names it, reaches count 0. It retires, R1's visitor frees its VkImage, and the bindless slot is reused. The material then samples a destroyed or foreign image. CONFIDENCE: CONFIRMED (field shape), PLAUSIBLE (trigger). NEEDED: make asset → asset links counted relations, or pin their targets.

**Architect's action (rev 3 changelog, row "N3 asset → asset raw slots"):** FIX. Role-typed counted relations `TexRef<R>` (EK22); derived slots via `asset_slot_sync`

**Where in rev 3:** X-23, ED5, §4.2, AS4

### N4

**Finding (verbatim):**

N4 - C1 residual, EK15c: X-12 says K7-backed relations are 'a new impl of an existing trait' that 'touches no hook body' (J collection.rs:6-7). That is false for `Segmented`. `fn add(&mut self, e: Entity) -> bool;` (collection.rs:41), `with_capacity` and `clear` have no access to the SegmentedColumn, and a `SpanRef` has no Drop to free its span when the parent despawns. CONSEQUENCE: K-EK15c needs a trait change (a context parameter) and an explicit free path; without the free, span blocks leak on every parent despawn and the frontier grows under churn. The Send/Sync SAFETY in §5 also needs one SegmentedColumn per owning component id or group. Otherwise two write params on different owner types race on the shared free lists and frontier. Late rung; no UI rung waits. CONFIDENCE: CONFIRMED (trait signature).

**Architect's action (rev 3 changelog, row "N4 K7 relations need a trait context and a free path"):** FIX (lands at K-EK15c). `type Ctx`; derive-installed `release_fn`; one column per owner type

**Where in rev 3:** ED15, EK15c, §5, K-EK15c

### N5

**Finding (verbatim):**

N5 - C1 residual, the K7-in-group filing: rev-3 `release_dying` 'writes each column's `const DEAD: Self` into every dying slot' (:1454) and has no per-slot visitor. A `SpanRef` header in a group column (soft bodies, bone arrays) is overwritten without `free`. CONSEQUENCE: each soft-body despawn leaks its span, and the bank grows without bound under churn. NEEDED: file K7 with a DEAD `SpanRef` and the same release visitor K6′ asks for. CONFIDENCE: CONFIRMED (text).

**Architect's action (rev 3 changelog, row "N5 K7 span leak on group release"):** FIX (input). `SpanRef::DEAD`; `release_dying_with`

**Where in rev 3:** ED15, ED16, §13

### N6

**Finding (verbatim):**

N6 - Anchor-mask capacity against Q1(a). Rev 3 :1364 '`anchor_mask: u8` … At most 8 anchored groups per process; registration asserts this'. ED5 makes one K3 group per asset kind: 7 kinds (§4.1) plus PhysicsBody = 8, the cap. A ninth group, such as a soft-body bank under physics Q2(a) or animation skeleton instances (physics K3 row :468), panics at boot registration. CONFIDENCE: CONFIRMED (arithmetic), PLAUSIBLE (that a ninth group arrives). NEEDED: state how many groups the design consumes; merge kinds, or file a wider mask as a physics input.

**Architect's action (rev 3 changelog, row "N6 anchor cap"):** FIX. Groups only for GPU-indexed kinds: 3, plus physics = 4 of 8. Font, sheet, clip and doc are table kinds

**Where in rev 3:** X-24, ED5, §4.1, §13

### N7

**Finding (verbatim):**

N7 - W1 residual: detached-but-alive nodes. Today's candidates are chosen by reachability from `UiRoot` (U:crates/boyko_ui/src/interaction/focus.rs:219-256). The new scan filters only on component presence, and the paint walk writes keys only on the nodes it reaches. A widget whose `ChildOf` was removed, or a subtree whose root lost `UiRoot`, keeps a stale `ComputedRect`, `ComputedPaintKey` and `ComputedEffectiveClip`. It stays hoverable and clickable at its old position. UI4's oracle corpus has no detached case, so the gate stays green. CONFIDENCE: CONFIRMED (divergence by construction), PLAUSIBLE (how common the pattern is). NEEDED: invalidate the keys of unreached nodes, for example a walk generation in the key or a reset driven by `Removed<ChildOf>`/`Removed<UiRoot>`; add detached subtrees to the corpus.

**Architect's action (rev 3 changelog, row "N7 detached-but-alive nodes"):** FIX, stronger than asked. `PaintGen` + `ui_paint_sweep` cover `despawn_without_children`, which removal signals miss. Pack skips `UNREACHED`. Corpus extended

**Where in rev 3:** X-26, ED8, §4.2, §8, UI4

### N8

**Finding (verbatim):**

N8 - RF1 lists 'view' as a resource-sourced mirror, but ED14 moves `RenderView` onto the camera component. `FifMirror::Source: Resource` cannot source a component. Q3 makes this worse (see impacts). NEEDED: a component-sourced arm, or a derived resource written by `resolve_active_view`. CONFIDENCE: CONFIRMED (plan-internal).

**Architect's action (rev 3 changelog, row "N8 view mirror sourced from a component"):** FIX. Derived `ViewUniforms` resource

**Where in rev 3:** ED3, ED14, §4.2, RE4, SC3

### O1

**Finding (verbatim):**

O1 - `UiActionPresses<A>` needs a hand-written `Resource` impl through `resource_id_for`, following the `ActionState` precedent (state.rs:432-437). It also needs `PhantomData<fn() -> A>` (map.rs:139-142), or Send/Sync depend on A.

**Architect's action (rev 3 changelog, row "O1 `UiActionPresses` id and Send"):** FIX. Now a generic component (EK22) with `PhantomData<fn() -> A>`

**Where in rev 3:** ED9, §5

### O2

**Finding (verbatim):**

O2 - `FifMirrorState.uploaded` ticks are missing from §17's `check_ticks` clamp list. After a source has been unchanged longer than the wrap horizon, a compare against an unclamped stored tick can skip a real change.

**Architect's action (rev 3 changelog, row "O2 mirror ticks not clamped"):** FIX by `TickEpoch` invalidation (the kernel cannot reach render state). The same applies to `FrameInFlight`; rev 2's `gather_tick` clamp claim corrected

**Where in rev 3:** X-29, ED3, §5, §17

### O3

**Finding (verbatim):**

O3 - EK6's id-based bind path (`ui_bind_apply` source ids are chosen at setup) must refuse untracked or group ids. It should get a row in physics rev 3's tick-consumer census (:1335), or a bound K2 column faults in release builds.

**Architect's action (rev 3 changelog, row "O3 id-based bind path on group/untracked ids"):** FIX. Setup refusal; census row filed

**Where in rev 3:** ED8, EK6, §13

## Decided-option impacts

**Impacts (verbatim, in the critic's order):**

1. Q2 (opt-in UiPlugins), G-GRAPH: must run as two apps (EnginePlugins alone: no UI system, empty exclusive set; plus UiPlugins: `{ui_bind_apply}`, `UiBindSet` after `GameplaySet`). Blocking, B5.
2. Q2, R5 `render_record`: the required `Res<UiPackLanes>` becomes optional or contributed by the UI plugin. UiPlugins must also register the `UiGpu` NonSend resident, UI pipeline creation in `StartupSet::Pipelines`, the EK11 rank-3 teardown entry and `upload_mirror::<UiPack>`. Blocking, B5.
3. Q2, G-ALLOC: the anti-vacuity app ('UI nodes > 0') must compose UiPlugins. UI2's 'plugins host everything (Q2)', HO3's `UiViewport` deriver and UI8's wiring all go through UiPlugins; `boyko_demo` and the playground add it. Note that `UiPlugin` (singular, the `.ui` loader) already exists at U:crates/boyko_ui/src/plugin.rs:35.
4. Q2, §6 graph: `ui_hit_test::<A>`, `UiLogicSet`, `UiBindSet`, `UiLayoutSet` and `ui_render_gather` exist only with UiPlugins. The input crate must expose named sub-sets of `InputSet` so the UI can order itself between `input_fold_physical` and `action_fold::<A>` without input naming UI. `action_fold`'s `Option<Res<UiActionPresses<B>>>` is already opt-in-safe. Nothing becomes unsound.
5. Q2, D31 (render → ui Cargo edge): every app still links boyko_ui. This costs compile time and binary size, not per-frame work. Open question: should the render-side UI live behind the opt-in?
6. Q3 (window and player entities), IN2: `ActionState<A>`, `InputMap<A>` and the 4 `Box<[f32]>` arrays move to player components. This needs a generic component id mint (blocking, B4). ActionState becomes fixed arrays (≤ 256 actions) or K7 spans; InputMap becomes a shared per-A CSR bank with a per-player (offset, len) header, not 2 EK1 columns per player. `ACTION_NAMES` stays a per-A resource (N6).
7. Q3, ED7/§6: `action_fold::<A>` becomes a query over players (cost proportional to players × action sets). The cursor fields of `PhysicalInput` (`cursor_pos`, `cursor_inside`, `window_focused`, read at U focus.rs:151 and :174-177) move to the window entity; key state stays a resource, because `CapturedMsg` carries no device id.
8. Q3, IN3/ED9: which player receives a UI press is undefined. If `action_fold` merges `UiActionPresses<A>` into every player's `ActionState<A>`, W2's phantom press returns across players. This needs a window/pointer → player link, and IN3 needs a two-player gate (a click for player 1 does not press player 2's action k).
9. Q3, HO3/§4/§5: `WindowSurface` becomes a component on the window entity, so EK5 is no longer needed for it; EK5 stays for the FIF mirrors. `UiViewport`, `UiPointerState` and `HoveredWorldEntity` become per-window (via a root → window relation). `ui_hit_test`'s `ResMut<UiPointerState>` becomes a per-window query. The G-FORM row 'window size ×3 → one resource-column' is relabelled; the 19 → 7 target is unchanged.
10. Q3, HO4/ED2/ED11: the kernel has no NonSend components (no match for them in J boyko_ecs), so `PresentChain`, `FrameInFlight`, `Swapchain`, `Surface` and `Window` stay NonSend singletons. The design must state that v1 has exactly one window entity (a `PrimaryWindow` marker; `Query::single` exists at J query.rs:1122) and refuses a second, until a per-window swapchain rung exists. `FrameWriteToken` stays device-global.
11. Q3, EK7/IN4: `RawInput` must carry the window `Entity` (from per-HWND userdata) once a second window exists. v1 can rely on the cardinality assert.
12. Q3, ED14/SC3: 'at most one `ActiveView` (debug_assert)' and SC3's '≤ 1 active' gate become per window or per player (split screen), or v1 states one view with a reason. The RF1 view mirror must be component-sourced (N8).
13. Q3, consumers: `RebindSession<A>` needs a target player. The default quit action (replacing the Escape fallback) belongs to no player. Every reader of `Res<ActionState<…>>` changes: 9 files on J, including J:crates/boyko_app/src/fly.rs:78 and the aether_lang code generator (J:crates/aether_lang/src/expand.rs:1683 `actions: ::boyko_ecs::ecs::core::system::Res<ActionState>`).
14. Q1 (assets are entities) plus physics rev 3: every asset kind needs an anchor component and a `DenseGroupMut` writer (B1); the anchor cap is at its limit (N6); release goes through the rev-3 K6 shape (B2). Q4: no impact beyond RE1 and LG2 as written.

**Architect's actions (rev 3 changelog rows, verbatim):**

- **Q2: G-GRAPH, R5, G-ALLOC, `InputSet` sub-sets, demo/playground, D31:** FIX. ED19; D31 decided as no feature (where in rev 3: ED19, §6, §12, §18 O-13)
- **Q3: IN2, ED7, IN3 routing, HO3/HO4 windows, EK7/IN4, SC3, consumers, quit action:** FIX. ED18: window and player entities, v1 one window, `PointerPlayer`, `EngineAction` on the `DefaultPlayer`, readers migrated (where in rev 3: ED7, ED9, ED14, ED18, §4, §5, §6, §12)
- **Q1 + physics rev 3: anchors, writers, release:** FIX (see B1, B2, N6) (where in rev 3: ED5, ED16)
- **Q4:** no change beyond RE1/LG2 (where in rev 3: —)

**Rev 3 changelog rows for defects the architect found beyond the critique (verbatim):**

- **New: `EcsMaster::clear()` resets group stores:** FIX. `clear_gameplay()`; `clear()` legal only at teardown (where in rev 3: X-28, ED5, AS5)
- **New: teardown release refused by `chain_head_registered`:** FIX. Teardown-token form (where in rev 3: ED16, §5 drop order)
- **New: two `InputPlugin`s race to spawn a player:** FIX. Build-time spawn (where in rev 3: ED18, IN2)
- **New: a loading texture's descriptor is unwritten:** FIX. Null descriptor on anchor; R2 orders texture before material (where in rev 3: ED5, §6 R2, AS3)
- **New: EK17 obsolete under decided Q1:** deleted (where in rev 3: §9)
- **New: edit-log window semantics:** adopted the kernel's inclusive at-least-once window (`mut_.rs:54`) (where in rev 3: X-30, EK6g)

## Pass-1 fixes confirmed by the critic (verbatim)

- C1: the filing is FIXED, with residuals N4 and N5. There is one primitive under physics's exact name and prerequisite (K7, prerequisite K3, :473). The header is held by the owner, so no owner key lives inside the primitive. Growth is by power-of-two size classes with ≤ 2n total copies. Holes go to LIFO class free lists with no compaction. The ordered `Children` is decoupled onto today's storage (EK15a has no prerequisite; verified: trait at J collection.rs:29-78, `type Collection = Vec<Entity>` at hierarchy/mod.rs:197, clone denies `Children` at clone/cloner.rs:29). Rev 3 left K7 unchanged.
- W1: RESOLVED for reachable nodes (residual N7). The global `ComputedPaintKey = StackIndex<<32 | dfs_index` reproduces `(stack_index, paint_seq, entity)`. `stack_index` is the node's own `StackIndex` defaulting to 0 (focus.rs:227-230). `paint_seq` counts Interaction-plus-rect nodes in DFS pre-order (:224-243). Roots are walked in id order (:214). The clip replaces (:222). The stored f32 `x + w` equals `point_in_clip`'s compare at :283.
- W2: RESOLVED. `Option<Res<R>>` exists (J res.rs:160), and `ui_hit_test::<A>` mirrors `ui_dispatch_system::<A>`.
- W3: RESOLVED. `truncate` (views.rs:208) and an in-place `push` on an address-stable base (:141-156); holding index ranges across pushes is correct.
- W4: RESOLVED. Both walks iterate the same `q` (mesh_draw.rs:1296 and :1330); one `bucket_slot` function; the alignment proptest covers ring, material_ids, material_tex, prev and vb_ring.
- W5, W8: RESOLVED (EK16 narrowed, RF1 in render, no G-FORM credit; hand-off table).
- W6: RESOLVED in policy (fixed horizon, generation in records, typed overflow). The wording of the overflow test is N1.
- W7: RESOLVED. The px_range leaf is GENERATED (U ui_rect.fs.hlsl:143-153), and the sprite path already uses `NonUniformResourceIndex` on `g_sprites` (:229), so the text branch follows the same shape through the eDSL.
- W9: the approach (one deferred-release capability with a caller horizon) is correct. The filing is stale against physics rev 3 (B1, B2).
- O1-O6: RESOLVED. For O3, rev 3 simplifies the plan: group columns are excluded from save by construction (:1704). For O4, the first anchored insert appends to slot 0 when the free list is empty. `UiBindSet` already exists on U (boyko_ui/src/lib.rs:101).
- X-11, X-13, X-15, X-16, X-17 re-verified at the cited lines. ED16's horizon proof is sound on this kernel's tick model: all Main systems share `this_run` (schedule.rs:331), and apply-window stamps are `this_run + 1` (:426). Only the read order is wrong (B3).

---

# Rev 3 patch

## How to read rev 3 - precedence and provenance

- **Rev 2 stands except where the patch below names a section.** Each patch block (`## P-…`) names the section it edits, quotes the removed rev 2 text verbatim and gives the added text. A section the patch does not name is unchanged rev 2 text.
- **The decisions file is the authority for Q1-Q5.** `M:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md` records Q1 (a) assets are entities, Q2 (b) UI is opt-in through `UiPlugins`, Q3 (b) window and player are entities now, Q4 (a) the dead paths are deleted, Q5 timings only on the owner's word. Where §14's option texts or any rev 2 recommendation disagrees with that file, the file wins. The patch's P-§14 block records the one premise of the Q1 decision text that physics rev 3 refutes (asset change detection is not `Changed<T>` for the group kinds); the decision itself stands.
- **Order of reading:** rev 2 body (above the pass-1 log), then this patch on top of it. The pass-1 and pass-2 critique logs are the record of why; they are not design text.
- **Provenance of the text below.** It is the `architect`'s output, verbatim. The architect emitted it in two consecutive messages because of the output limit. The first ends mid-cell in the AS2 gate row of P-§12; the second opens with a row marked `**AS2 (rev 3)** *(continued)*` that resumes the same cell. The two are joined by one line break and nothing else, so that one table row is visibly split in two. Nothing was edited, reordered or dropped.
- The patch names physics rev 3 lines by `M:…PHYSICS…:<line>`, meaning `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md`.

---

# Rev 3 patch: ENGINE-RUNTIME-ECS-DESIGN.md (rev 2 → rev 3)

## How to read rev 3

- This patch applies to the rev 2 text in `M:docs/unification/ENGINE-RUNTIME-ECS-DESIGN.md`. Each block gives the section, the **removed** text quoted verbatim, the **added** text, and the sections whose invariants the change rests on. Sections not named here are unchanged.
- **Trees:** J = `D:/wt/joltab` @ `d11962a9`; U = `D:/wt/ui` @ `615cda8f` (all UI claims come from U); M = docs. The physics design was read as rev 2 plus its rev-3 patch: D13 at :1360-1417, D14 at :1449-1486, K3/K6 at :1643-1667, the API at :1715-1732, and U2 at :1975.
- **Newly read for rev 3:**
  - J: `schedule.rs:320-432`, `frame_driver.rs:290-367`, `material_table.rs:70-281`, `material.rs:140-199`, `entity_api.rs:795-848`, `relationship/mod.rs:1-352`, `deferred_master.rs`, `resource_type_registry.rs`, `component.rs:340-399`, `state.rs:42-100` and `:380-442`, `actionlike.rs`, boyko_macros `actionlike.rs`, `mut_.rs`, and every crate `Cargo.toml`;
  - U: `focus.rs:145-180`, `sprite.rs:234`/`:255`, and a grep of `ui_rect.fs.hlsl`.
- Nothing was run and nothing was edited. I have no shell. I did not re-run the critic's "9 reader files" count; IN2 names the readers the migration must cover.
- **Q1–Q4 are decided** (`M:docs/unification/ENGINE-RUNTIME-ECS-DECISIONS.md`). Rev 3 re-cuts the design to them. Q2 and Q3 go against rev 2's recommendations.

---

## P-header

**Removed (verbatim):**
> - **Revision:** rev 2 after one critique pass

**Added:**
> - **Revision:** rev 3 after two critique passes (rev 3 is the patch appended after the pass-2 log)

**Removed (verbatim):**
> The file exists in M at the time of writing; its later revisions (a pass-2 critique and a rev 3 patch appended there) are not reflected here.

**Added:**
> Rev 3 of this design is checked against physics rev 2 plus its rev-3 patch (D13 binder/anchors, D14 release at the chain head, K3 `StorageKind::Group`, K6 `release_dying`).

**Removed (verbatim):**
> - **Status:** design only. No gate was run and no timing was taken. The owner questions in section 14 (Q1-Q5) are open.

**Added:**
> - **Status:** design only. No gate was run and no timing was taken. Q1–Q5 were decided under the owner's delegation (`ENGINE-RUNTIME-ECS-DECISIONS.md`): Q1 (a), Q2 (b), Q3 (b), Q4 (a), Q5 on the owner's word.

---

## P-§0: corrections table (new rows)

**Added (after X-17):**

| ID | Premise | What the tree / sibling says | Consequence |
|---|---|---|---|
| X-18 (B1) | Rev 2 ED5: the asset value is a K3 column with kernel `Changed`/`Added` | `M:…PHYSICS…:1566` "one untracked ComponentPool per column id (K1 contiguous ids, K2 no tick pages); DEAD-initialised"; `:1695` "`Changed<T>` dense \| `filter.rs:1492` \| same" (const-assert); writes go only through `:1726` `pub fn view<T: GroupColumn<Group = G>>(&self) -> TypedDenseView<'_, T>;`; `:1662` "Membership changes only through binder transitions (D13)" | Group columns cannot carry `Changed<T>`, and the value exists (DEAD) from the anchor on. The change signal becomes EK6g; loading = DEAD + `AssetReady` disabled (ED5). |
| X-19 (B2) | Rev 2 ED16 amends `Commands::release_dense_group` | `:1459` "`Commands::release_dense_group` is **deleted**"; `:1483` "That call panics (cold) if any `DenseGroupMut<G>` has ever been initialised in the world" | K6′ is re-filed on `DenseGroupMut<G>`. R1 becomes function systems (ED16). |
| X-20 (B3) | Rev 2 R0 copies the gather tick into the slot it waited on | `J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:321` `let fence = self.frames[self.frame_index].in_flight;` … `:333` `Ok(FrameWriteToken { slot: self.frame_index })`. The waited slot is the slot the new frame reuses. Out-of-date returns without a submit (`:351-354`) | The completed frame's gather tick must be latched before anything overwrites it, and written only on a successful submit (§5 `FrameInFlight`). |
| X-21 (B4) | Rev 2 assumed generic components work (`ActionState<A>` under Q3, and rev 2's own `Staged<A::Cpu>`) | `J:crates/boyko_macros/src/component.rs:372-373` `static ID: ::std::sync::OnceLock<…>` inside `component_id()`; `J:crates/boyko_input/src/action/state.rs:432-434` "the derive caches the id in a `static` inside the generic `resource_id()` body, which collapses every `A` onto one id (rust#22991)". The TypeId mint exists only for resources: `J:crates/boyko_ecs/src/ecs/core/resources/resource_type_registry.rs:94` `pub fn resource_id_for<T: Resource>() -> ResourceId {` | New kernel feature EK22 (ED20). Rev 2's `Staged<A::Cpu>` (AS5) had the same latent defect. |
| X-22 (N2) | "despawning with count > 0 becomes retire-at-0" | Hooks cannot veto (`J:…/relationship/mod.rs:24-27`). Every despawn path funnels into one core: `J:…/ecs_master/entity_api.rs:811-812` `pub fn delete_entity(&mut self, entity: Entity) -> bool {` / `let result = self.delete_entity_core(entity);` and `:843` `self.delete_entity_core(entity)` (`DespawnCommand::apply` reaches it, `:813-816`) | The redirect goes into `delete_entity_core` (EK15b). |
| X-23 (N3) | Only instance → asset references are counted | `J:crates/boyko_render/src/material.rs:144-147` `pub struct MaterialTextures {` … `pub albedo: u32,` (raw bindless slots; 5 roles, `:147-158`) | Asset → asset links become role-typed counted relations `TexRef<R>` (ED5, EK22). |
| X-24 (N6) | All 7 asset kinds need a K3 group | Sprite-sheet UVs are computed on the CPU: `U:crates/boyko_ui/src/sprite.rs:234` `pub fn frame_uv(&self, index: u16) -> [f32; 4] {`. `ui_rect.fs.hlsl` names "sheet" only in a comment (`:160`). The font atlas is sampled through the texture's bindless slot (X-13). Clips are CPU-sampled (§4 animation) | Only mesh, material and texture are indexed by their own slot on the GPU. That is 3 groups, and with `PhysicsBody` 4 of the 8 anchors. |
| X-25 | ED16's proof assumed Main apply windows stamp `this_run + 1` | True (`J:…/schedule/schedule.rs:426-432`). The state pass runs before any system at `this_run` (`:408-413` "Reuses the frame-start `this_run` (no second bump)") | A removal in a state transition stamps g, not g+1. ED16's proof is restated for any stamp ≤ g. |
| X-26 (N7) | Detaching is visible as `Removed<ChildOf>`/`Removed<UiRoot>` | `J:…/entity_api.rs:824-826` "the children survive — each keeps a now-**dangling** [`ChildOf`] pointing at the freed parent" | Removal signals miss this case. N7 needs a reachability sweep (ED8). |
| X-27 | Facades are derived SystemParams | No `derive(SystemParam)` exists in J (grep over `crates`). Hand-written precedent: `J:…/params/res.rs:160` | `AssetRead`/`AssetWrite` are hand-written `unsafe impl SystemParam`. |
| X-28 (new) | ED5: world-clear skips `AssetRoot` | `M:…PHYSICS…:1371` "`EcsMaster::clear` resets every group store." | A clear that skipped asset entities would leave them with no slot. See ED5's clear rule. |
| X-29 (new) | Rev 2 §17: `check_ticks` clamps `gather_tick` | `FrameInFlight` and `FifMirrorState` are render-owned NonSend state, which the graphics-pure kernel cannot reach | That rev-2 claim was false. Render-owned ticks re-base when `TickEpoch` changes (O2, §5). |
| X-30 (new) | `Mut<T>` window semantics | `J:…/iters/query/data/mut_.rs:54` `.is_newer_than(Tick::new(self.last_run.get().wrapping_sub(1)), self.this_run)`; writes stamp `this_run` (`:105`) | The kernel window is inclusive `[last_run, this_run]`, at least once. EK6g and the N1 rule adopt the same window. |

Depends on: nothing.

---

## P-§2: targets

**Removed (verbatim):**
> | Exclusive UI systems | 13 (U, UI lens §4) | 1 (`ui_bind_apply`, ED8) |

**Added:**
> | Exclusive UI systems | 13 (U, UI lens §4) | 1 in an app composing `UiPlugins` (`ui_bind_apply`, ED8); 0 in an app without it (Q2b) |

Depends on: ED19.

---

## P-ED3: FIF-mirror scope (N8, B1)

**Removed (verbatim):**
> - Resource-sourced mirrors: view, CSM, atlas, ray, denoise, temporal, TAA, motion-cam, lights, particle effects, instance rings, UI pack, SDF edits.
> - Asset-sourced device lanes (materials, mesh meta) do not use `FifMirror`. They upload per row on `Changed<T>` of the asset's dense value (Q1a), or through the `Assets<T>` counters (Q1b).

**Added:**
> - Resource-sourced mirrors: view (source `ViewUniforms`, a derived resource-column written by `resolve_active_view`, one row per window, one in v1; N8), CSM, atlas, ray, denoise, temporal, TAA, motion-cam, lights, particle effects, instance rings, UI pack (only with `UiPlugins`), SDF edits.
> - Asset device lanes (material rows, mesh meta, texture descriptors) do not use `FifMirror`. R2 uploads the slots named by the group's **edit log** (EK6g) in the upload system's window. It uploads the current bytes, so a duplicate or post-reuse record is idempotent. On `EditWindow::Overflowed` or a lane grow it re-stages the whole table (today's path, `J:crates/boyko_render/src/material_table.rs:255-271`).
> - **Tick wrap (O2).** `FifMirrorState` stores `check_epoch`. `upload_mirror` reads `TickEpoch`, the world's last `check_ticks` tick (stored at `schedule.rs:353`), and clears `valid` on a change. So every mirror re-uploads once per clamp pass (~100 days at 60 FPS, `schedule.rs:341-343`). No compare against an unclamped tick can skip a change.

Depends on: EK6g (§9), ED14, §5.

---

## P-ED5: assets are entities (B1, N2, N3, N6, X-28)

**Removed (verbatim):**
> ### ED5. Assets are entities. Recommended; put to the owner as Q1 because it reverses S1.

**Added:**
> ### ED5. Assets are entities (Q1a, decided)

**Removed (verbatim):**
> - **Value and release.** The asset value is a column of a **K3 dense group with `RELEASE = Deferred`** on the asset entity (physics K3/D14, `M:docs/physics/PHYSICS-ECS-UNIFICATION-DESIGN.md:478` "`RELEASE = Immediate | Deferred` (D14);"). Its slot never moves, so GPU tables index by slot.
>   - A retired asset's slot enters `dying` with its bytes intact.
>   - It returns to `free` only through the K6 release, called with a **horizon tick** (K6′, ED16). For assets the horizon is the gather tick of the last fence-completed frame.
>   - This replaces rev 1's `Retiring{epoch}` component (W9).
> - **Load state** is component presence: no value = loading; `AssetFailed`; `Pinned`.
> - **Refcount** is a count-only relation, instance → asset (EK15b). A `CountOnly` target cannot enumerate its sources, so despawning an asset with count > 0 is converted into "retire when the count reaches 0". The kernel's target-despawn cascade is statically disabled for count-only targets (EK15b).
> - **Change tracking.** `dirty_gen` / `install_epoch` / `free_epoch` become kernel `Changed`/`Added` on the dense value (EK6) plus the release visitor.

**Added:**
> - **Which kinds are K3 groups (N6, X-24).** Only the kinds whose own slot indexes a GPU table:
>   - `MeshGroup`: `MeshMeta`, whose `geometry_slot` indexes the mesh meta lane;
>   - `MaterialGroup`: `Material { gpu, textures }`, indexing the material table row;
>   - `TextureGroup`: `TextureMeta`, indexing the bindless slot.
>
>   Each has `RELEASE = DeferredStamped` (K6′, ED16) and `EDIT_LOG = true` (EK6g). The group marker types are private to `boyko_render`, so only its facades can name `DenseGroupMut` over them.
> - **Table kinds.** Font, sprite sheet, animation clip and UI document are asset entities whose value is an ordinary **table component**: `FontMetrics` (with a glyph CSR header), `SpriteSheet`, `AnimClip`, `UiDocument`. No GPU table is indexed by their slot, so they need neither slot stability nor a fence horizon. They keep ordinary `Changed<T>`, and they despawn immediately.
> - **Anchor budget:** 3 groups plus `PhysicsBody` = 4 of the 8 that `anchor_mask: u8` allows (`M:…PHYSICS…:1364`). Four remain for a soft-body bank, animation skeleton instances and others.
> - **Anchor and slot.** Each group is anchored on a ZST table component: `#[component(anchor_group = MeshGroup)] MeshAsset`, and likewise `MaterialAsset` and `TextureAsset`.
>   - The anchor is attached at handle mint, in the asset entity's spawn, so the binder's transition gives the slot in that spawn's apply window. The value is DEAD until installed (`:1458`, `:1667`).
>   - A loading asset therefore holds a slot. The 4096 texture ceiling (AS3) counts loading textures.
> - **Value writers.** Only these systems hold `DenseGroupMut<G>`:
>
>   | Kind | Install | Edit | Retire |
>   |---|---|---|---|
>   | mesh | R2 `render_asset_upload::<Mesh>`, after the device buffers exist | EK21 re-stage → R2 | R1 `render_retire::<Mesh>` |
>   | texture | R2 `render_asset_upload::<Texture>`, after the VkImage exists | EK21 re-stage → R2 | R1 `render_retire::<Texture>` |
>   | material | `asset_install::<Material>` (Main `AssetSet`) | the `AssetWrite<Material>` facade, from any Main system; `asset_slot_sync` for texture slots | R1 `render_retire::<Material>` |
>
>   `AssetWrite<K>` is a hand-written SystemParam (X-27) over `DenseGroupMut<K::Group>`, `GroupEditsMut<K::Group>` and `GroupSlot` reads. Its `get_mut(handle)` marks the edit log on access, so no write path skips the log.
> - **Load state.** Loading = DEAD bytes plus `AssetReady` (an EnableTag) disabled. The value writer enables `AssetReady`. `AssetFailed` and `Pinned` are ZSTs.
> - **DEAD values:**
>   - `MeshMeta::DEAD = { index_count: 0, geometry_slot: VB_GEOMETRY_RESERVED_SLOT, … }`: an inert draw, matching today's fallback (`J:crates/boyko_render/src/mesh_draw.rs:535-536` `map_or(VB_GEOMETRY_RESERVED_SLOT, |m| m.geometry_slot)`);
>   - `Material::DEAD` = the slot-0 default material bytes, so a loading material draws as the default;
>   - `TextureMeta::DEAD` = zero extent.
> - **Texture descriptor validity.** R2 writes the null-image descriptor into every newly anchored texture slot (`Added<TextureAsset>`) before any material row of the same R2 is uploaded. R1's release visitor re-nulls a released slot. So no recorded command reads an unwritten bindless descriptor, including a material or UI record that names a still-loading texture.
> - **Change signal (B1).** Group columns are untracked by construction (X-18), so the three groups use the kernel group edit log (EK6g). Table kinds use `Changed<T>`.
>   - Consumers: R2's material upload (per row), transparency's `MaterialXGpu` lane (§13), and EK21 feedback.
>   - This refutes the premise, in the Q1 decision text, that "asset change detection becomes ordinary `Changed<T>`", for the three GPU-indexed kinds. It does not overturn Q1: that decision's overturn gate is the handle-resolution microbench (AS2).
> - **Release.** `DeferredStamped` plus K6′ at R1's horizon (ED16). The dying bytes stay intact until the release.
> - **Refcount (N2, N3).** Every reference to an asset is a count-only relation (EK15b). Asset → asset links are role-typed:
>   - instance → mesh, instance → material;
>   - widget → font / sprite sheet;
>   - material → texture, one relation per role: `TexRef<Albedo | Normal | MetalRough | Ao | Emissive>` (X-23);
>   - font → atlas texture (`TexRef<FontAtlas>`), sheet → texture (`TexRef<SheetImage>`), widget → texture (`TexRef<UiImageTex>`).
>
>   `TexRef<R>`/`TexUsers<R>` are generic components (EK22). One target component exists per role, because J's relation pair round-trips one source type per target type (`J:…/relationship/mod.rs:293` `type Source: Relationship<Target = Self>;`).
> - **Refcount rules:**
>   - Derived slots (the instance's `MeshSlot`/`MaterialSlot`, the material value's `textures.*`) are written by `asset_slot_sync` from the target's `GroupSlot`, on `Changed` of the relation component.
>   - When the sum of counts over an unpinned asset's counted targets reaches 0, the asset despawns (EK15b `DESPAWN_AT_ZERO`).
>   - A despawn request on an asset whose count is > 0 unpins it instead (EK15b redirect).
>   - A slot-0 sentinel (`AssetSentinel`) refuses despawn.
>   - A `Handle<T>` is not a reference: a stale handle is rejected by its generation.

**Removed (verbatim):**
>   - `save_world` walks every archetype (`J:crates/boyko_serialize/src/save.rs:170` `for archetype in world.archetype_master().iter_archetypes() {`). Asset value columns are therefore classified **Ignore**.

**Added:**
>   - `save_world` walks every archetype (`J:crates/boyko_serialize/src/save.rs:170` `for archetype in world.archetype_master().iter_archetypes() {`).
>     - Group value columns are excluded by construction: they are not in `dense_ids()` (`M:…PHYSICS…:1704`).
>     - The table kinds' value components, the anchor ZSTs and `AssetReady` are classified **Ignore**.
>     - `insert::<T>`/`Bundle` const-assert `!IS_GROUP_COLUMN` (`:1663`), so load and prefab paths reach the value only through `Staged<A::Cpu>` and the install systems.

**Removed (verbatim):**
>   - World-clear utilities skip entities carrying `AssetRoot` unless asked.

**Added:**
>   - **Clear (X-28).** `clear_gameplay()` (boyko_scene) despawns every entity without `AssetRoot` through the ordinary despawn path. Asset group stores are untouched.
>   - `EcsMaster::clear()` resets every group store (`:1371`). For a world with asset groups it is therefore legal only at teardown, after `release_dense_group_at_teardown` (EK11 rank 4). A debug assert checks that no asset device lane still holds a resource.

**Removed (verbatim):**
> - Assets now depend on physics K3/K6′ (the kernel-first order puts both before AS2).

**Added:**
> - Assets depend on physics K3 (U2), on the K6′ input being accepted (ED16), and on EK6g, EK15b and EK22. The kernel-first order puts all of them before AS2.
> - One loading asset holds one DEAD slot.

Depends on: ED16, §5, §9 (EK6g, EK15b, EK22, K6′), §12 AS2–AS5.

---

## P-ED7: input (Q3b)

**Removed (verbatim):**
> - `input_fold_physical` runs once. `action_fold::<A>` runs once per `A`.

**Added:**
> - `input_fold_physical` runs once. It writes key and button state to the `PhysicalInput` resource. Cursor position, inside and focus go to the primary window's `WindowCursor`, and resize/DPI to its `WindowSurface` (ED18). `CapturedMsg` carries no device id, so key state stays one resource.
> - `action_fold::<A>` runs once per `A`, as a query over players: `(&InputMap<A>, &mut ActionState<A>, Option<&UiActionPresses<A>>)` plus `Res<InputBindings<A>>` and `Res<PhysicalInput>`. Cost ∝ players × bindings. With 1 player it costs what today's fold does.
> - Every player folds the shared `PhysicalInput`. Players differ by `InputMap` profile (a split keyboard). Per-device routing is not in v1.

Depends on: ED18.

---

## P-ED8: UI (N7, Q2b, Q3b)

**Removed (verbatim):**
> - Plugins register every UI system.

**Added:**
> - `UiPlugins` (opt-in, ED19) registers every UI system. An app without it has no UI system and no UI resource.

**Added (after the bullet `  - `ComputedClip` stays author-owned and untouched (O-1 answered).`):**
> - **Reachability (N7, X-26).** Detached-but-alive nodes must stop being hit and packed. Removal signals miss `despawn_without_children`'s dangling `ChildOf` (X-26), so reachability is a walk generation:
>   - `ComputedPaintKey` defaults to `UNREACHED = 0`; real keys start at `dfs_index = 1`. `ComputedEffectiveClip` defaults to `EMPTY = [+INF, +INF, -INF, -INF]`, which contains no point. `Interaction` requires both components, so a never-attached node is born unreachable.
>   - Each change-frame walk increments a `Local` `walk_gen` and writes `PaintGen(walk_gen)` (4 B, not in the scan row) on every node it reaches.
>   - `ui_paint_sweep`, right after `ui_paint_walk`, runs change frames only. It is one sequential pass over `(&PaintGen, &mut ComputedPaintKey, &mut ComputedEffectiveClip)`: rows whose gen ≠ `walk_gen` get `UNREACHED`/`EMPTY` through `set_if_neq`. That covers `remove::<ChildOf>`, `remove::<UiRoot>`, a parent despawned without children, and never-attached nodes.
>   - The hit-test's containment test excludes `EMPTY` with no branch. `ui_render_gather` skips `key == UNREACHED` rows (one predictable branch).
>   - The paint walk is already O(reachable nodes) per change frame, so the sweep keeps the class. The every-frame scan stays at 40 B/row.

**Removed (verbatim):**
> - **The hit-test (every frame).** `Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>`, **filtered by the presence of `Interaction`** as today (`focus.rs:225`), not by `Focusable`. It does a branch-free argmax of the key among rows whose rect and clip contain the cursor, then the write pass.

**Added:**
> - **The hit-test (every frame).** For the one window (ED18; `Query::single`), read `WindowCursor`, `UiViewport` and `UiPointerState` from the window entity. Then run `Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>`, **filtered by the presence of `Interaction`** as today (`focus.rs:225`), not by `Focusable`. It does a branch-free argmax of the key among rows whose rect and clip contain the cursor, then the write pass.
>   - A press goes to the player named by the window's `PointerPlayer` (ED9).
>   - v1 has one window, so the scan filters nothing. The multi-window rung adds a per-node window key.

**Removed (verbatim):**
> - **The residual exclusive system** is `ui_bind_apply`. Its source component ids are chosen at setup. It uses EK2 ticks. It runs in its own `UiBindSet`, **after `GameplaySet`**, so a HUD shows this frame's gameplay writes (O6).

**Added:**
> - **The residual exclusive system** is `ui_bind_apply`, which exists only with `UiPlugins`.
>   - Its source component ids are chosen at setup. Setup **refuses** `StorageKind::Group` ids and untracked dense ids with a setup error (O3), so the id-based tick read never touches uncommitted tick memory.
>   - It uses EK2 ticks.
>   - It runs in its own `UiBindSet` (existing: `U:crates/boyko_ui/src/lib.rs:101`), **after `GameplaySet`**, so a HUD shows this frame's gameplay writes (O6).

Depends on: ED18, ED19, §6, §12 UI4.

---

## P-ED9: typed presses per player (Q3b, O1)

**Removed (verbatim):**
> - **UI action presses go into `UiActionPresses<A>`**, a resource-column generic over the one action set the UI dispatches to (W2).
>   - `UiInteractionPlugin::<A>` inserts it. The hit-test system is `ui_hit_test::<A>`, monomorphised once, exactly as `ui_dispatch_system::<A>` is today (`U:crates/boyko_ui/src/interaction/plugin.rs:117-118` `.add_system(ui_dispatch_system::<A>)`).
>   - `action_fold::<B>` reads `Option<Res<UiActionPresses<B>>>`. `Option<Res<R>>` is a SystemParam on J: `J:crates/boyko_ecs/src/ecs/core/system/params/res.rs:160` `unsafe impl<'a, R: Resource> SystemParam for Option<Res<'a, R>> {`.
>   - For every `B ≠ A` the resource is absent, so a UI press never reaches another set.

**Added:**
> - **UI action presses go into `UiActionPresses<A>`**, a **component on the player entity**. It is generic over the one action set the UI dispatches to (W2). It is minted by EK22 and carries `PhantomData<fn() -> A>`, so Send/Sync do not depend on `A` (the `J:…/action/map.rs:139-142` precedent; O1).
>   - `ui_hit_test::<A>` is monomorphised once, as `ui_dispatch_system::<A>` is today (`U:crates/boyko_ui/src/interaction/plugin.rs:117-118`).
>   - It clears the presses of every player that carries the component (few rows).
>   - It writes the one player named by the hit window's `PointerPlayer` relation, through `Query::get_mut`. If that player lacks the component, a cold `Commands` insert carries the press. The insert is applied in the window before `action_fold` (the successor waits for the apply window), so the press lands in the same frame.
>   - `action_fold::<B>` reads `Option<&UiActionPresses<B>>` per player. For every `B ≠ A` the component does not exist, and for every player other than the pointer player the bits are clear. So a UI press reaches neither another set nor another player.
>   - **Routing.** `PointerPlayer` is a Relationship window → player, with target `PointerWindows` (Vec collection, `LINKED_DESPAWN = false`). The host sets it to the `DefaultPlayer` at window spawn. A window with no `PointerPlayer` drops presses, with one rate-limited diagnostic.

Depends on: ED18, ED20, §12 IN3.

---

## P-ED14: views (Q3b, N8)

**Removed (verbatim):**
> - `ActiveView` is an EnableTag; at most one is enabled (debug_assert).

**Added:**
> - `ActiveView` is an EnableTag. At most one is enabled **per window**. v1 has one window (ED18), so at most one exists in the world (debug_assert). Split-screen views per player are not in v1.
> - `resolve_active_view` writes `ViewUniforms`: a derived Send resource-column, `#[resource(ticks)]`, one row per window, written only when the bytes differ. It is the RF1 view mirror's source (N8).

Depends on: ED3, ED18.

---

## P-ED15: K7 residuals (N4, N5)

**Removed (verbatim):**
>     - in a K3 group column (physics soft bodies, animation bone arrays — hence K3 for that consumer);
>     - in a table component (`Children` under EK15c);

**Added:**
>     - In a K3 group column (physics soft bodies, animation bone arrays; hence K3 for that consumer).
>       - `SpanRef::DEAD = { ptr: dangling, len: 0, class: u8::MAX }`, and `free(DEAD)` is a no-op.
>       - A group holding `SpanRef` columns releases with `release_dying_with(visit)` (K6′), which frees each dying slot's span **before** the DEAD overwrite (N5). Rev-3 `release_dying` has no visitor (`M:…PHYSICS…:1453-1457`), so without this each despawn would leak its span.
>     - In a table component (`Children` under EK15c).
>       - The `RelationshipSourceCollection` trait gains a context parameter.
>       - The structural core frees the span on remove, despawn and clear, through a derive-installed `release_fn` (EK15c, N4). `SpanRef` has no Drop.
>     - **One `SegmentedColumn` per owning component type or per group column**, so free lists and frontiers are never shared between two owners' write params (N4).

Depends on: ED16 (K6′), §9 EK15c, §5 Send/Sync.

---

## P-ED16: whole section replaced (B2, B3, X-25)

**Removed (entire rev-2 ED16, verbatim):**
> ### ED16 (new, W9). One deferred-release capability, K6 with a caller-supplied horizon (K6′, a hand-off to physics)
>
> **What.**
> - Physics K3/D14 already defines deferred dense release: `M:.../PHYSICS-ECS-UNIFICATION-DESIGN.md:239-240` "The slot enters a `dying` list, not the free list" / "Release happens only through a kernel command, `Commands::release_dense_group::<G>()`".
> - This design asks for **one** amendment: each `dying` entry is stamped with the world change tick of the apply window that removed it, and release takes a horizon, `release_dense_group::<G>(horizon: Tick)`. It frees exactly the dying slots stamped before `horizon` (wrap-aware compare).
>   - Physics passes its S6 `this_run`, which is today's "release all dying" semantics.
>   - Assets pass `FrameInFlight::completed_gather_tick()`: the tick at which the last fence-completed frame's gather ran.
> - An `EcsMaster::release_dense_group_with::<G>(horizon, |slot| …)` visitor lets R1 free the NonSend device lane of each released slot in the same pass.
>
> **Proof of the asset horizon.**
> - A slot that died at tick `t` is named only by frames whose gather ran before `t`.
> - Queue completion is in order, so once a completed frame's gather tick exceeds `t`, every earlier frame has completed.
>
> **Cost.**
> - +4 B per dying entry.
> - One compare per dying slot per release call.

**Added:**
> ### ED16 (rev 3, B2/B3). One deferred-release capability: rev-3 K6 plus a stamped-horizon form on `DenseGroupMut` (K6′, re-filed against physics rev 3)
>
> **What physics rev 3 has.**
> - Release is a step inside the chain head's body: `DenseGroupMut<G>::release_dying()` (`M:…PHYSICS…:1725`).
> - No command exists (`:1459`).
> - The exclusive `EcsMaster::release_dense_group` is refused once a `DenseGroupMut<G>` has been initialised (`:1483`, `:1655`; flag `chain_head_registered`, `:1570`).
>
> **K6′, the input to physics.** Four additive items. None of them changes a byte, or a line of code, of what `PhysicsBody` does.
> 1. `RELEASE = Immediate | Deferred | DeferredStamped`.
>    - Only `DeferredStamped` groups carry `died: VmColumn<Tick>`, parallel to `Recycle::dying` (`:1572`).
>    - The binder's removal transition writes `world.current_tick()` into it. That is `this_run + 1` in an apply window (`schedule.rs:426`) and `this_run` in a state transition (X-25).
> 2. `DenseGroupMut<G>::release_dying_before(horizon: Tick, visit: impl FnMut(u32)) -> u32`. It const-asserts `G::RELEASE == DeferredStamped`.
>    - It releases exactly the dying entries whose `died` is strictly older than `horizon`, using the kernel's wrap-aware compare against `this_run`.
>    - It calls `visit(slot)` before that slot's DEAD fill, and appends released slots to `free` in dying order, so LIFO reuse stays deterministic.
>    - It compacts the retained entries in place, keeping their order.
>    - It returns `len()`.
> 3. `DenseGroupMut<G>::release_dying_with(visit) -> u32`: `release_dying()` plus the visitor, for groups holding K7 `SpanRef` columns (N5).
> 4. `EcsMaster::release_dense_group_at_teardown::<G>(&TeardownToken, visit)`.
>    - It releases every dying entry, whatever `chain_head_registered` says.
>    - `TeardownToken` is minted only by EK11's teardown driver, which runs when no schedule runs. So the hazard the rev-3 refusal guards against (an exclusive release between S1 and S5, while S2's pairs still name the slots, `:1483`) cannot occur.
>
> **Physics's horizon is "all".** S1 keeps calling `release_dying()` unchanged. Rev 2's "physics passes its S6 `this_run`" is withdrawn. It was wrong twice: physics releases at the head of S1 now, and a `this_run` horizon would keep in `dying` every slot removed in an earlier Fixed apply window of the same run, stamped `this_run + 1`, which is physics's B2 ghost.
>
> **Assets.** R1 is three function systems, `render_retire::<K>` for K ∈ {Mesh, Material, Texture}. Each holds `DenseGroupMut<K::Group>`, `NonSendMut<K::DeviceLane>` and `NonSend<FrameInFlight>`, and calls `release_dying_before(frame.completed_gather, |slot| lane.free(slot))`.
> - Mesh: free the geometry range.
> - Texture: destroy the VkImage and re-null the descriptor.
> - Material: no-op; the row is dead.
>
> **The horizon (B3).** `FrameInFlight` (§5):
> - R5 writes `submitted_gather[s] = GatherTick`, and only after a successful `vkQueueSubmit` on slot s.
> - R0 latches `completed_gather = submitted_gather[s]` right after the fence wait on s (`frame_driver.rs:321`). This comes before any code of the current frame can write `submitted_gather[s]`.
> - Frames that are skipped (minimized, or out-of-date, `frame_driver.rs:351-354`) submit nothing and write nothing.
>
> **Proof.** Let g_k be frame k's Main `this_run`, the gather tick. All Main systems share it (`schedule.rs:331`).
> 1. A removal stamped t ≤ g_k was stamped before Main(k) started. The state pass at g_k precedes every system (X-25), and ticks are monotone. So frame k's gather could name slot s only if t > g_k.
> 2. Let c = `completed_gather` at R1(N). It is the gather tick of the frame last submitted from the slot R0(N) waited on, and that frame is complete.
> 3. Suppose frame k names s and s is released, so t < c. Then g_k < t < c. Gather order is submission order (one Main per frame, frames submitted in order), so frame k was submitted before the completed frame. By in-order queue completion, frame k is complete.
> 4. Frames that were never submitted name nothing on the GPU. The current frame N gathered at g_N > c > t, so it does not name s either. ∎
>
> The proof holds for any FIF and across skipped frames. At FIF = 2 a slot is reused about 2 frames after its death. That is one frame later than the tighter g_{N−1} bound; the simpler proof is taken, because an extra frame of dying slots costs only address space.
>
> **Cost.**
> - +4 B per dying entry of `DeferredStamped` groups only.
> - One compare per dying entry per R1 run.
> - Physics: 0.
>
> **Rejected:**
> - A `this_run` horizon (B2).
> - An exclusive R1 on `EcsMaster`: it panics per `:1483`.
> - A per-group opt-out of the refusal: the flag would stop guarding physics's chain against an exclusive release.
> - `g_{N−2}` read from the overwritten slot: that was rev 2's defect (B3).

Depends on: §5 `FrameInFlight`, §6 R0/R1/R5, §9 K6′, §11, §12 AS2, §13, §17.

---

## P-ED17: overflow rule (N1)

**Removed (verbatim):**
> - A reader whose `last_run` is older than the oldest retained record gets `RemovalWindow::Overflowed` instead of an iterator. The type forces the caller to handle it.

**Added:**
> - **Overflow is exact.** Each log keeps `dropped_newest`, the tick of the newest record ever truncated. A reader's window is the kernel's inclusive `[last_run, this_run]` (X-30).
>   - It gets `RemovalWindow::Overflowed` if and only if `dropped_newest` falls in that window, that is, a record it would have read is gone. Otherwise it gets `Complete`.
>   - So a reader that runs every frame never overflows. That includes records stamped `this_run + 1` by Main's apply windows (N1's example: dropped records are stamped before the previous frame's start, which is ≤ `last_run`).
>   - The type forces the caller to handle `Overflowed`.
> - **Fixed readers.** At schedule build every EK3/EK6g reader param registers its schedule. For a log with a Fixed-schedule reader, the truncation cut is `min(start of the previous frame, this_run of the Fixed schedule's latest run)`.
>   - A Fixed reader that ran in the latest Fixed run therefore never overflows, at any ratio of frame rate to fixed rate.
>   - Retention is bounded by one fixed step plus one frame of wall time.
>   - No Fixed reader exists in this design today. The rule is the kernel's contract.

**Removed (verbatim):**
> **Trade-off.** A reader skipped for one frame or more after a structural change rebuilds once.

**Added:**
> **Trade-off.** A reader whose skipped span loses a record to truncation rebuilds once. A skip with no dropped record in its window costs nothing.

Depends on: §9 EK3/EK6g, §17.

---

## P-§3: new decisions ED18–ED20 (added after ED17)

**Added:**

> ### ED18 (new, Q3b). Window and player are entities. OS handles stay NonSend singletons. v1 has exactly one window.
>
> **What.**
> - **Window entity.** The host spawns it in `StartupSet::Device`, after the OS window exists (HO3).
>   - `Window` + `PrimaryWindow` markers.
>   - `WindowDesc`: title, requested extent, mode. It moves out of `LaunchConfig`.
>   - `WindowSurface`: extent, scale, safe area.
>   - `WindowCursor { pos: [f64; 2], inside, focused }`. These are the fields today read from `PhysicalInput` (`U:crates/boyko_ui/src/interaction/focus.rs:151` `let cursor_active = physical.cursor_inside && physical.window_focused;`; `:174-177`).
>   - `PointerPlayer(player)`.
>   - With `UiPlugins`, also `UiViewport`, `UiPointerState`, `UiKeyboardFocus` (rev 2's `UiInputFocus`) and `HoveredWorldEntity`.
> - **Where the types live.** `boyko_input::{window, player}`. That is the lowest crate every consumer already depends on:
>   - `boyko_scene` → `boyko_input` (`J:crates/boyko_scene/Cargo.toml:34`);
>   - `boyko_ui` → `boyko_input` (`J:crates/boyko_ui/Cargo.toml:23`);
>   - `boyko_input` depends only on ecs, utils and macros (`J:crates/boyko_input/Cargo.toml:13-15`).
>
>   `boyko_render` gains a normal, acyclic edge to `boyko_input`.
> - **Player entity.**
>   - A `Player` marker; `DefaultPlayer` on the one the engine creates.
>   - Per action set: `ActionState<A>` and `InputMap<A> { profile }`. With UI, also `UiActionPresses<A>`.
>   - `InputPlugin::<A>::build` spawns the `DefaultPlayer` at build time if the world has none, then inserts the two components. Build holds `&mut World`, so two `InputPlugin`s cannot race to spawn two players.
>   - The default quit action, which replaces the Escape fallback, is an `EngineAction` set on the `DefaultPlayer`.
> - **Stay NonSend singletons** (N5; the kernel has no NonSend components): `PresentChain`, `FrameInFlight`, `Swapchain`, `Surface`, the OS `Window`, `Renderer`. `FrameWriteToken` is device-global. `RenderCommit` stays a device-global resource, because the swapchain is a singleton.
> - **v1 cardinality: exactly one `Window` entity.**
>   - `window_fold` release-asserts a count ≤ 1, through a cold message naming the multi-window rung. `Query::single` (`J:…/iters/query/query.rs:1122`) is the access form.
>   - `RawInput` carries no window id; every event belongs to the primary window.
>   - The multi-window rung is not in this design. It adds `window: Entity` to `RawInput` (the WNDPROC sink resolves HWND → entity through `GWLP_USERDATA`), a per-window swapchain, `ViewTarget(window)` on cameras, and a per-node window key in the hit-test.
>
> **Why.** Decided under Q3(b). Per-window and per-player data are touched once per frame per instance, so the per-frame cost equals the singleton form. The tie-break is the unification goal (DECISIONS Q3).
>
> **Cost (arith.).**
> - `action_fold`: players × Σ bindings, the same as today for one player.
> - Window data: one row.
> - `ActionState<A>`: 224 + 24·COUNT B per player per set, with no `Box` (ED20).
>
> **Trade-off.**
> - Every `Res<ActionState<…>>` reader becomes `Query<&ActionState<A>, With<DefaultPlayer>>` plus `.single()`. The readers include `J:crates/boyko_app/src/fly.rs:78` and the aether_lang generator (`J:crates/aether_lang/src/expand.rs:1683`).
> - `RebindSession<A>` gains `player: Entity`.
> - `.keys` persistence writes the `DefaultPlayer`'s profile.
>
> ### ED19 (new, Q2b). `UiPlugins` composes the UI; `EnginePlugins` does not
>
> **What.**
> - `boyko_app::UiPlugins<A: Actionlike>` composes:
>   - the boyko_ui core: layout, paint walk + sweep, logic, bind, `UiBindSet`;
>   - `UiInteractionPlugin::<A>`, i.e. `ui_hit_test::<A>`;
>   - the `.ui` loader `UiPlugin` (`U:crates/boyko_ui/src/plugin.rs:35`);
>   - `boyko_render::ui::UiRenderPlugin`: the `UiGpu` NonSend resident, UI pipelines in `StartupSet::Pipelines`, the EK11 rank-3 teardown entry, `ui_render_gather` in `RenderPrepareSet`, `upload_mirror::<UiPack>` in R4, `UiPackLanes`, and the `UiViewport` deriver on the window entity.
> - `boyko_input` exposes the named sub-sets `InputSet::{Physical, Sources, Actions, Listeners}`, so the UI orders `ui_hit_test::<A>` in `Sources` without input naming the UI.
> - `render_record` takes the UI as optional inputs: `Option<Res<UiPackLanes>>` + `Option<NonSend<UiGpu>>` → `ui: Option<&UiPass<'_>>`. `render_gbuffer_frame` already takes that `Option` (§10). The cost is one branch per frame, and a UI-less app still records and presents.
> - `boyko_demo` and the playground compose `UiPlugins`.
> - **D31 stays an unconditional Cargo edge, with no feature flag.** The per-frame cost is 0 without `UiPlugins`, because no system is registered. Code reachable only from `UiRenderPlugin::build` is unreferenced from `main`, and rustc passes `--gc-sections` (GNU) and `/OPT:REF` (MSVC) unless `-C link-dead-code` is set, so that code does not reach the image or L1i (not verified against this workspace's linker config). A feature would add a build leg to save compile time only.
>
> **Alternative rejected.** A record-contributor registry, where the UI registers a callback into R5. It adds an fn-pointer indirection and breaks "recording stays one system" (§6).
>
> ### ED20 (new, B4). Generic components get a TypeId-keyed id mint (EK22)
>
> **What.**
> - `component_id_for::<T>(install: fn(u32)) -> ComponentId`, over a `TypeIntern<TypeId, { COMPONENT_SLOT_COUNT * 2 }>`. This is the primitive behind `resource_id_for` (`J:…/resource_type_registry.rs:70`, `:94`).
> - On first sight it mints via `register_new::<T>()` and runs `install(raw)`, the same derive-installed tables as the non-generic body: hooks, storage, require, clone, relationship, residency, serialize (`J:crates/boyko_macros/src/component.rs:374-393`). They are monomorphised per `T`, because the install fn holds no `static`.
> - `#[derive(Component)]` on a generic type emits this body in place of `static ID: OnceLock` (`:372-373`).
> - A generic component with no explicit stable serialize name is classified save-Ignore.
>
> **Consumers:** `ActionState<A>`, `InputMap<A>`, `UiActionPresses<A>`, `TexRef<R>`/`TexUsers<R>`, and rev 2's `Staged<A::Cpu>`.
>
> **Cost.** The hit path is one hash plus one acquire load (`resource_type_registry.rs:67-69`), where the non-generic path is one acquire load. It is paid only on uncached paths (command constructors, `get_component::<T>`). Query and param state cache the id at init.
>
> **ActionState storage.** The `Actionlike` derive already emits `COUNT` (`J:crates/boyko_macros/src/actionlike.rs:79-80`, asserted ≤ 256 at `:112-116`). It additionally emits `type Lanes = ActionLanes<{COUNT}>`, so `ActionState<A>` is `{ 7 × BitSet256, lanes: A::Lanes }`: exact fixed arrays with no `Box`.
>
> **Alternative rejected.** Concrete per-set component types emitted by `#[derive(Actionlike)]`. It has zero lookup cost but covers only input, so `Staged<A::Cpu>` and `TexRef<R>` would need a second route: two mechanisms for one need.

Depends on: §5, §6, §9 (EK22), §10, §12 (IN2, IN3, HO3, UI2).

---

## P-§4.1

**Removed (verbatim):**
> - **mesh, material, texture, font, sprite-sheet, animation-clip, UI document:** yes, asset entities (Q1a), with the value as a **K3 Deferred group column** (ED5).

**Added:**
> - **mesh, material, texture:** asset entities whose value is a column of a **K3 `DeferredStamped` group** with an edit log (ED5).
> - **font, sprite-sheet, animation-clip, UI document:** asset entities whose value is a **table component** (ED5, X-24).
> - **window, player:** entities (Q3b, ED18).

**Removed (verbatim):**
> | mesh, material, texture, font, sprite-sheet, animation-clip, UI document | **yes, asset entities (Q1a)** | ED5 |

**Added:**
> | mesh, material, texture | **yes, asset entities; group value** | ED5 |
> | font, sprite-sheet, animation-clip, UI document | **yes, asset entities; table value** | ED5, X-24 |

**Removed (verbatim):**
> | window | no in v1: NonSend resource | N1; Q3 |

**Added:**
> | window | **yes** (Q3b): portable components on the window entity; the OS, surface and swapchain handles stay NonSend singletons (N5); v1 has exactly one | ED18 |
> | player | **yes** (Q3b): `ActionState<A>`, `InputMap<A>`, `UiActionPresses<A>` | ED18 |

**Removed (verbatim):**
> | action | no: resource-column `ActionState<A>` | N6; Q3 |

**Added:**
> | action | no: an index into the player's `ActionState<A>` lanes | N6 |

---

## P-§4.2: datum rows (Q1–Q3, B1, N3, N8)

UI rows. Each row below replaces the rev-2 row with the same number. The removed text is the rev-2 row, verbatim:

> | 2 | `UiText.font`, `UiImage.texture`, `UiSpriteSheet.sheet` | raw indices (`components.rs:442` `pub texture: u32,`) | **relation** widget → asset via `Handle<T>`; `UiText` 12 → 16 B, `UiSpriteSheet` 4 → 12 B (arith.) | UI7 |
> | 3 **(rev 2)** | `FontTable` + 4 `Box` fields | Resource `Vec` (`text/font.rs:139`) | asset entity `Font`: `FontMetrics` group column (including the atlas bindless slot) + glyph table as **CSR on EK1 columns**, header `(offset, len)` in `FontMetrics` (write-once; no K7) | UI7 |
> | 4 | `UiSheetTable` | Resource `Vec` (`sprite.rs:257`) | asset entity `SpriteSheet` | UI7 |
> | 11 | `UiViewport` + `generation` | Resource; no production writer | resource-column derived from `WindowSurface` (EK5) | UI2 |
> | 13 | `UiPointerState` slots | fixed array | resource-column (N1), message slots removed | UI4 |
> | 16 **(rev 2)** | `ui_press` + re-freeze | cross-subsystem write | **resource-column** `UiActionPresses<A>` (N1 per `A`; event refuted) | IN3 |
> | 17 | `UiInputFocus` | `Option<Entity>` | resource-column | — |
> | 18 | `HoveredWorldEntity` + prev mirror | Resource + copy | resource-column + triggers; mirror deleted | UI6 |

**Added:**
> | 2 **(rev 3)** | `UiText.font`, `UiImage.texture`, `UiSpriteSheet.sheet` | raw indices (`components.rs:442` `pub texture: u32,`) | **counted relations** widget → asset (`FontRef`, `TexRef<UiImageTex>`, `SheetRef`; EK15b, EK22), plus derived slot bytes the pack reads; `UiText` 12 → 16 B, `UiSpriteSheet` 4 → 12 B (arith.) | UI7 |
> | 3 **(rev 3)** | `FontTable` + 4 `Box` fields | Resource `Vec` (`text/font.rs:139`) | asset entity `Font`: **table component** `FontMetrics` with glyph CSR header `(offset, len)` into one shared `FontGlyphBank` (CSR on two EK1 columns, write-once per load); atlas = **counted** `TexRef<FontAtlas>` to a texture asset (N3) | UI7 |
> | 4 **(rev 3)** | `UiSheetTable` | Resource `Vec` (`sprite.rs:257`) | asset entity with **table component** `SpriteSheet` (UVs stay CPU-side, `sprite.rs:234`) + counted `TexRef<SheetImage>` | UI7 |
> | 11 **(rev 3)** | `UiViewport` + `generation` | Resource; no production writer | **component on the window entity**, derived from `Changed<WindowSurface>` by `UiPlugins` | UI2 |
> | 13 **(rev 3)** | `UiPointerState` slots | fixed array | **component on the window entity**; message slots removed | UI4 |
> | 16 **(rev 3)** | `ui_press` + re-freeze | cross-subsystem write | **component** `UiActionPresses<A>` on the pointer player (EK22; ED9) | IN3 |
> | 17 **(rev 3)** | `UiInputFocus` | `Option<Entity>` | **component** `UiKeyboardFocus` on the window entity | UI4 |
> | 18 **(rev 3)** | `HoveredWorldEntity` + prev mirror | Resource + copy | **component on the window entity** + triggers; mirror deleted | UI6 |
> | 35 **(new, N7)** | reachability | implicit in the DFS | component `PaintGen(u32)` (walk-written) + `ui_paint_sweep` | UI4 |

Render rows. Removed (verbatim):
> | 12 **(rev 2)** | `MaterialTable` GPU mirror | NonSend | **resource-column**: NonSend, indexed by the material asset slot; VRAM is driver-owned (N5); gated by `Changed` on the material value (Q1a). **Not counted as removed in G-FORM**: the row keeps its count, and only its index space and gate change (W5) | RE6 |
> | 14 **(rev 2)** | orphan queues | NonSend `Vec<(T,u64)>` | **K3 deferred release with a horizon** (ED16) (Q1a) / EK17 (Q1b) | AS2 |
> | 30 | `ViewUniform` | Resource | component `RenderView` (ED14) | SC3 |

**Added:**
> | 12 **(rev 3)** | `MaterialTable` GPU mirror | NonSend | **resource-column**: NonSend, indexed by the material asset slot; VRAM is driver-owned (N5); gated by the `MaterialGroup` **edit log** (EK6g), per row; the whole table is re-staged only on `Overflowed` or grow. **Not counted as removed in G-FORM** (W5) | RE6 |
> | 14 **(rev 3)** | orphan queues | NonSend `Vec<(T,u64)>` | **K3 `DeferredStamped` + K6′ `release_dying_before`** in R1 (ED16) | AS2 |
> | 30 **(rev 3)** | `ViewUniform` | Resource | component `RenderView` (ED14) + derived resource-column `ViewUniforms` (one row per window) as the RF1 source (N8) | SC3 |

Host rows. Removed (verbatim):
> | `PhysicalInput` | Resource | resource-column (N1; persistent levels) | — |
> | `ActionState<A>` + 4 `Box<[f32]>` | Resource | resource-column on EK1 columns sized at build | IN2 |
> | `InputMap<A>` CSR **(rev 2)** | 2 Boxes | resource-column: **CSR on two EK1 columns** (write-once at build; rebind is cold and rebuilds). No K7 | IN2 |
> | `RebindSession<A>` | user struct | component on the listening widget + `EventReader<RawInput>` | IN1 |
> | window size ×3 | three copies | one resource-column `WindowSurface` (EK5) | HO3 |
> | Escape fallback | host logic | deleted: a default quit action | IN1 |
> | `WindowDesc` + `BOYKO_*` reads | closure / env | resource-column `LaunchConfig` | HO1 |

**Added:**
> | `PhysicalInput` **(rev 3)** | Resource | resource-column: keys and buttons only (N1, no device id); cursor fields → `WindowCursor` | IN1 |
> | `ActionState<A>` + 4 `Box<[f32]>` **(rev 3)** | Resource | **component per player**, fixed `A::Lanes` arrays, no `Box` (EK22, ED20) | IN2 |
> | `InputMap<A>` CSR **(rev 3)** | 2 Boxes | **component** `InputMap<A> { profile: u16 }` per player + per-A resource `InputBindings<A>`: one CSR bank of profiles on two EK1 columns (rebind appends a profile, cold; the bank is compacted at rebind). 2 columns per A regardless of player count | IN2 |
> | `RebindSession<A>` **(rev 3)** | user struct | component on the listening widget, `player: Entity`, + `EventReader<RawInput>` | IN1 |
> | window size ×3 **(rev 3)** | three copies | one **component** `WindowSurface` on the window entity (G-FORM label relabelled; the 19 → 7 target unchanged) | HO3 |
> | Escape fallback **(rev 3)** | host logic | deleted: `EngineAction::Quit` on the `DefaultPlayer` | IN1 |
> | `WindowDesc` + `BOYKO_*` reads **(rev 3)** | closure / env | `WindowDesc` → component on the window entity; `BOYKO_*` → resource-column `LaunchConfig` | HO1 / HO3 |
> | cursor pos / inside / focused **(new)** | `PhysicalInput` fields | component `WindowCursor` on the window entity | HO3 |

Scene rows. Removed (verbatim):
> | `MeshHandle` / `MaterialHandle` + 4 hooks | raw index | Q1a: **relation** instance → asset (count-only target, EK15b) + a derived `u32`/`u16` slot component the pack reads (same bytes); Q1b: unchanged | AS4 |
> | `Assets<T>` value / `slot_word` / `free` / `live` / `refcount` / `pinned` / dirty counters **(rev 2)** | parallel kernel | Q1a: **K3 Deferred group column** / kernel-internal slot map / relation count / `Pinned` ZST / kernel change detection | AS5 |
> | `AssetStaging<A>` | NonSend Vec | component `Staged<A::Cpu>` on the asset entity | AS5 |
> | `DeferredFree` **(rev 2)** | Resource Vec | deleted: K3 `dying` + K6′ horizon (ED16) | AS2 |

**Added:**
> | `MeshHandle` / `MaterialHandle` + 4 hooks **(rev 3)** | raw index | **counted relation** instance → asset (EK15b) + a derived `u32`/`u16` slot component written by `asset_slot_sync` from `GroupSlot` (same bytes) | AS4 |
> | `Assets<T>` value / `slot_word` / `free` / `live` / `refcount` / `pinned` / dirty counters **(rev 3)** | parallel kernel | mesh/material/texture: **K3 `DeferredStamped` group column** / group slot map / relation count / `Pinned` ZST / **group edit log (EK6g)**; font/sheet/clip/doc: **table component** / `Changed<T>` | AS2 / AS5 |
> | `AssetStaging<A>` **(rev 3)** | NonSend Vec | generic component `Staged<A::Cpu>` on the asset entity (EK22) | AS5 |
> | `DeferredFree` **(rev 3)** | Resource Vec | deleted: K3 `dying` + `died` stamps + K6′ (ED16) | AS2 |

Depends on: ED5, ED14, ED18, ED20.

---

## P-§5: data structures

**Removed (verbatim):**
```
pub struct FrameInFlight {                 // written by render_acquire, token taken by render_record
    token: Option<FrameWriteToken>,        // None ⇔ minimized / no acquire this frame
    gather_tick: [Tick; FRAMES_IN_FLIGHT], // tick at which RenderPrepareSet's gather ran for the frame in slot s
    completed: Option<usize>,              // slot whose fence R0 last waited on
}                                          // size not verified (FrameWriteToken holds `slot: usize`)
```
**Added:**
```rust
pub struct FrameInFlight {                        // NonSend; dispatcher-only
    token: Option<FrameWriteToken>,               // None ⇔ minimized / no acquire this frame
    submitted_gather: [Tick; FRAMES_IN_FLIGHT],   // GatherTick of the frame last SUBMITTED from slot s;
                                                  //   written ONLY by R5 after a successful vkQueueSubmit
    completed_gather: Tick,                       // latched by R0 immediately after the fence wait on slot s,
                                                  //   = submitted_gather[s], BEFORE anything of this frame can write it;
                                                  //   = gather tick of the newest frame known complete (ED16 horizon)
    tick_epoch: Tick,                             // TickEpoch last seen; on change, submitted_gather[*] and
                                                  //   completed_gather are re-based to the clamp floor (conservative:
                                                  //   delays release by ≤ FRAMES_IN_FLIGHT frames) — X-29
}   // boot: every tick = the boot tick ⇒ nothing is released before a frame completes.
    // Size not verified (FrameWriteToken holds `slot: usize`).
```

**Removed (verbatim):**
```
    uploaded: [Tick; N],
    valid: u8,                              // bit s ⇔ slot s holds bytes of `uploaded[s]`; cleared on buffer replace
}
```
**Added:**
```rust
    uploaded: [Tick; N],
    check_epoch: Tick,                      // TickEpoch at the last upload; a change clears `valid` (O2, X-29)
    valid: u8,                              // bit s ⇔ slot s holds bytes of `uploaded[s]`; cleared on buffer replace
}                                           // N = 2 → 13 B + pad (arith.)
```

**Removed (verbatim):**
```
// ── Asset meta lane (ED4): K3 Deferred group column on the mesh asset entity (Q1a) ─
#[repr(C)]                                  // 36 B (arith.); read per MESH id
pub struct MeshMeta {
    index_count: u32,                       // DEAD = 0 ⇒ an inert draw
    geometry_slot: u32,                     // = the asset's dense slot
```
**Added:**
```rust
// ── Asset meta lane (ED4): MeshGroup (K3 DeferredStamped, EDIT_LOG) column; anchor = MeshAsset (ZST) ─
#[repr(C)]                                  // 36 B (arith.); read per MESH id
pub struct MeshMeta {
    index_count: u32,                       // DEAD = 0 ⇒ an inert draw
    geometry_slot: u32,                     // = the asset's group slot once installed; DEAD = VB_GEOMETRY_RESERVED_SLOT
```
```rust
// Anchors (table ZSTs): MeshAsset, MaterialAsset, TextureAsset; plus AssetRoot, AssetSentinel, Pinned, AssetFailed (ZST),
// AssetReady (#[component(storage = "bitset")], disabled at mint, enabled by the value writer).
```

**Removed (verbatim):**
```
pub struct UiActionPresses<A: Actionlike> { pressed: BitSet256, _a: PhantomData<A> } // 32 B; cleared by ui_hit_test::<A>
```
**Added:**
```rust
pub struct UiActionPresses<A: Actionlike> { pressed: BitSet256, _a: PhantomData<fn() -> A> } // component on a player (EK22);
                                                                                             // 32 B; cleared by ui_hit_test::<A>
#[repr(transparent)] pub struct PaintGen(u32);            // walk generation (N7); not in the hit-test scan row
// ComputedPaintKey::UNREACHED = 0 (default); ComputedEffectiveClip::EMPTY = [+INF, +INF, -INF, -INF] (default)
```

**Removed (verbatim):**
```
#[repr(C)] pub struct WindowSurface { extent: [u32; 2], scale: f32, _pad: u32, safe_area: [f32; 4] } // 32 B (arith.)
```
**Added:**
```rust
// ── Window / player entities (boyko_input::{window, player}; ED18) ────────────
#[repr(C)] pub struct WindowSurface { extent: [u32; 2], scale: f32, _pad: u32, safe_area: [f32; 4] } // component; 32 B (arith.)
#[repr(C)] pub struct WindowCursor { pos: [f64; 2], inside: u8, focused: u8, _pad: [u8; 6] }       // component; 24 B (arith.)
pub struct Window; pub struct PrimaryWindow; pub struct Player; pub struct DefaultPlayer;           // markers
pub struct PointerPlayer(Entity);  // Relationship window → player; target PointerWindows (Vec, no linked despawn)
pub struct WindowDesc { /* title, requested extent, mode */ }                                       // component

pub struct ActionState<A: Actionlike> {   // component per player (EK22). 224 B + A::Lanes (arith.)
    pressed, just_pressed, just_released, consumed,
    fixed_pressed, fixed_just_pressed, fixed_just_released: BitSet256,
    lanes: A::Lanes,                      // = ActionLanes<{COUNT}>, emitted by #[derive(Actionlike)]
}
pub struct ActionLanes<const N: usize> {  // 24·N B; replaces the 4 Box<[f32]> (state.rs:61-67)
    button_value: [f32; N], axis2: [[f32; 2]; N], fixed_value: [f32; N], fixed_axis2: [[f32; 2]; N],
}
pub struct InputMap<A: Actionlike> { profile: u16, _a: PhantomData<fn() -> A> }  // component per player
pub struct InputBindings<A: Actionlike> {    // resource (resource_id_for); CSR bank shared by all players of A
    offsets: ScratchColumn<u32>,             // (profiles × (COUNT + 1))
    bindings: ScratchColumn<Binding>,
}
```

**Removed (verbatim):**
```
// K6′ (physics-owned amendment): dying entries become (slot: u32, died: Tick) = 8 B.
```
**Added:**
```rust
// K6′ (physics-owned; input re-filed against rev 3): RELEASE = DeferredStamped only. Recycle gains
//   died: VmColumn<Tick>, parallel to `dying` (+4 B per dying entry). Deferred groups (PhysicsBody) unchanged.
// K7 input: SpanRef::DEAD = { ptr: dangling, len: 0, class: u8::MAX }; free(DEAD) is a no-op (N5).

// EK6g group edit log — kernel-internal, BESIDE (not inside) DenseGroupStore; one per EDIT_LOG group, own access node:
#[repr(C)] struct EditRecord { slot: u32, tick: Tick }   // 8 B (arith.); appended in non-decreasing tick order
//   records: untracked column of EditRecord; last_marked: VmColumn<Tick> indexed by slot (dedup per writer run);
//   dropped_newest: Tick (N1 watermark); has_fixed_reader: bool (ED17).
```

**Removed (verbatim):**
> 4. asset device lanes (after a final `release_dense_group_with` at horizon = MAX following the device idle);

**Added:**
> 4. asset device lanes. First the device idle, then `release_dense_group_at_teardown::<G>(&TeardownToken, lane-free visitor)` per asset group (K6′ item 4), then the lanes drop.

**Removed (verbatim):**
> **Send/Sync of `SpanRef`-bearing components.** `unsafe impl Send + Sync`, with this SAFETY: the pointee is written only under `&mut` of the owner, in apply windows or relationship hooks; the base is address-stable; readers hold `&` components only in waves with no concurrent writer (declared access of the owning component id).

**Added:**
> **Send/Sync of `SpanRef`-bearing components.** `unsafe impl Send + Sync`, with this SAFETY:
> - the pointee is written only under `&mut` of the owner, in apply windows or in command applies;
> - the base is address-stable;
> - readers hold `&` components only in waves with no concurrent writer (declared access of the owning component id);
> - **each owner type (or group column) has its own `SegmentedColumn`**, so two write params on different owner types never share a free list or a frontier (N4).

Depends on: ED16, ED17, ED18, ED20, §9.

---

## P-§6: system graph

**Removed (verbatim):**
> | `InputSet` | `input_fold_physical` → `ui_hit_test::<A>` (reads last frame's rect/key/clip; writes `Interaction`, `UiPointerState`, `UiActionPresses<A>`; `Commands::trigger`) → `action_fold::<B>` (one per `B`; reads `Option<Res<UiActionPresses<B>>>`) → rebind listeners | function | `action_fold` instances in parallel |

**Added:**
> | `InputSet::Physical` → `Sources` → `Actions` → `Listeners` **(rev 3)** | `input_fold_physical` + `window_fold` (keys/buttons → `PhysicalInput`; cursor/focus/resize/DPI → the primary window's `WindowCursor`/`WindowSurface`) → `ui_hit_test::<A>` (**UiPlugins only**; reads last frame's rect/key/clip and the window's cursor; writes `Interaction`, the window's `UiPointerState`, the pointer player's `UiActionPresses<A>`; `Commands::trigger`) → `action_fold::<B>` (one per `B`; a query over players; reads `Option<&UiActionPresses<B>>`) → rebind listeners | function | `action_fold` instances in parallel |

**Removed (verbatim):**
> | `UiBindSet` **(rev 2)** | `ui_bind_apply` (exclusive, EK2) | exclusive, serial | after `GameplaySet` and `UiLogicSet`: the HUD shows this frame's writes |

**Added:**
> | `UiBindSet` **(rev 3, UiPlugins only)** | `ui_bind_apply` (exclusive, EK2; refuses group/untracked ids at setup) | exclusive, serial | after `GameplaySet` and `UiLogicSet`: the HUD shows this frame's writes |
> | `AssetSet` **(new)** | loaders; `asset_install::<Material>` (`Staged` → value via `AssetWrite<Material>`; marks the edit log; enables `AssetReady`); `asset_slot_sync` (derived slot bytes on `Changed` of `MeshRef`/`MaterialRef`/`TexRef<R>`, from the target's `GroupSlot`) | function | after `GameplaySet`, before `RenderPrepareSet`; writers of one group serialise on `DenseGroupMut<G>` |

**Removed (verbatim):**
> | `UiLayoutSet` | `ui_text_measure` → `ui_layout` → `ui_paint_walk`; `ui_world_project`, `ui_world_pick`, `ui_world_visibility` | function | parallel with non-UI systems |

**Added:**
> | `UiLayoutSet` **(UiPlugins only)** | `ui_text_measure` → `ui_layout` → `ui_paint_walk` → `ui_paint_sweep` (N7); `ui_viewport_derive`; `ui_world_project`, `ui_world_pick`, `ui_world_visibility` | function | parallel with non-UI systems |

**Removed (verbatim, fragment of the `RenderPrepareSet` row):**
> stamps `FrameInFlight.gather_tick` via a Send `GatherTick` resource copied in R0

**Added:**
> writes the Send `GatherTick` resource (its `this_run`); R5 copies it into `FrameInFlight.submitted_gather[slot]` after a successful submit. `ui_render_gather` is UiPlugins-only and skips `UNREACHED` rows

**Removed (verbatim):**
> | R0 | `render_acquire`: fence wait, then swapchain acquire/recreate → `FrameInFlight`; copies `GatherTick` into `gather_tick[slot]`; `None` when minimized | F6 |
> | R1 | `render_retire` (exclusive): `release_dense_group_with::<AssetGroup>(completed_gather_tick, device-lane free)` per asset kind; `RetiredGpuBuffers` drain; `Local` scratch | F7 |
> | R2 | `render_asset_upload::<Mesh/Texture/Material>` (per-frame drain), geometry backfill, material seed | boot one-shots + D-e |

**Added:**
> | R0 | `render_acquire`: fence wait on slot s; **then** `completed_gather = submitted_gather[s]` (latch, B3); `TickEpoch` re-base; swapchain acquire/recreate → `FrameInFlight`; `None` when minimized | F6 |
> | R1 | `render_retire::<Mesh>`, `::<Material>`, `::<Texture>` (**function systems**: `DenseGroupMut<K::Group>` + `NonSendMut<K::DeviceLane>` + `NonSend<FrameInFlight>`; `release_dying_before(completed_gather, lane.free)`); `retired_buffers_drain` with `Local` scratch | F7 |
> | R2 | in this order: `render_asset_upload::<Mesh>`, then `::<Texture>` (null descriptor for every newly anchored slot; install values via `DenseGroupMut`; mark the edit log; enable `AssetReady`), then `::<Material>` (per-row upload from the `MaterialGroup` edit window; whole table on `Overflowed`/grow); geometry backfill; material seed | boot one-shots + D-e |

**Removed (verbatim):**
> | R5 | `render_record`: declare the graph, record one primary command buffer including the UI pass, submit, present; takes the token | F13 |

**Added:**
> | R5 | `render_record`: declare the graph, record one primary command buffer including the UI pass **iff** `Option<Res<UiPackLanes>>` and `Option<NonSend<UiGpu>>` are both present, submit, present; takes the token; on a successful submit writes `submitted_gather[slot] = GatherTick` | F13 |

Depends on: ED8, ED9, ED16, ED18, ED19.

---

## P-§8: layout table (N7, Q3b)

**Removed (verbatim):**
> | Paint walk (change frames) | 2 DFS copies | one DFS writing `ComputedPaintKey` + `ComputedEffectiveClip` via `set_if_neq` | +8 B + 16 B written per changed node | small cost, change frames only |
> | Input fold | destructive pop; N resets | one lane read, one fold, per-A CSR walk | N−1 resets removed | better, and correct |

**Added:**
> | Paint walk (change frames) | 2 DFS copies | one DFS writing `ComputedPaintKey` + `ComputedEffectiveClip` via `set_if_neq` and `PaintGen`; then one sweep | +8 B + 16 B written per changed node, +4 B `PaintGen` per reached node; the sweep reads 4 B/node sequentially and writes only newly unreached rows | small cost, change frames only; same class as the walk |
> | Input fold | destructive pop; N resets | one lane read, one fold per player, CSR walk of that player's profile | N−1 resets removed; ×players (1 in the default app) | better, and correct |

---

## P-§9: kernel features

**Removed (verbatim):**
> | EK5 | Resource change ticks (opt-in) | `#[resource(ticks)]` | UI KF-F; host K3 | `WindowSurface`, FIF mirrors, `UiViewport` | here |
> | EK6 | Dense change-tick API | dense-aware `any_changed_since` | GK-2; UI KF-E | `ui_bind_apply`; asset value `Changed` (Q1a) | here |

**Added:**
> | EK5 **(rev 3)** | Resource change ticks (opt-in) + `TickEpoch` | `#[resource(ticks)]`; a read-only param `TickEpoch` = the world's last `check_ticks` tick | UI KF-F; host K3 | FIF mirror sources (incl. `ViewUniforms`, N8); render-owned tick re-basing (O2). `WindowSurface`/`UiViewport` are components now (Q3b) | here |
> | EK6 **(rev 3)** | Dense change-tick API | dense-aware `any_changed_since` for table and **tracked** dense ids; `StorageKind::Group` and untracked ids are refused at param setup (O3) | GK-2 (table/dense half); UI KF-E | `ui_bind_apply` only | here; the census row is filed to physics (§13) |
> | EK6g **(new, B1)** | Group edit log | opt-in per K3 group (`const EDIT_LOG: bool`); `GroupEditsMut<G>::mark(slot)` deduplicates per writer run; `GroupEdits<G>::read() -> EditWindow::{Complete, Overflowed}` over the inclusive `[last_run, this_run]` window (X-30); retention and exact overflow per ED17; its own access node; lives beside `DenseGroupStore`, changes no physics struct | GK-2 (group half) | `MeshGroup`, `MaterialGroup`, `TextureGroup`; R2 material upload; `MaterialXGpu` (§13); EK21 | here, after K3 (physics U2) |
> | EK22 **(new, B4)** | Generic component id mint | `component_id_for::<T>(install)` over `TypeIntern` (ED20); `#[derive(Component)]` accepts generics | the `resource_id_for` pattern | `ActionState<A>`, `InputMap<A>`, `UiActionPresses<A>`, `TexRef<R>`/`TexUsers<R>`, `Staged<A::Cpu>` | here, no prerequisite |

**Removed (verbatim):**
> | EK15b **(rev 2)** | Count-only collection | `CountOnly` impl: `add`/`remove` = ±1, no iteration; the target-despawn cascade is statically disabled; despawning with count > 0 becomes retire-at-0 | KF-relation-count | asset refcount (Q1a) | here, no prerequisite |
> | EK15c **(rev 2)** | Relation storage on K7 | `Segmented` impl holding `SpanRef<Entity>` | KF-relation-collection-on-kernel-storage | `Children`, `Bindings`, every one-to-many target | here, after K7 |

**Added:**
> | EK15b **(rev 3)** | Count-only collection + keep-alive | `CountOnly` impl: `add`/`remove` = ±1, no iteration, `const ITERABLE = false` (the cascade is statically disabled). Target option `const DESPAWN_AT_ZERO`: the unlink command apply, under `&mut EcsMaster`, enqueues a despawn when the sum over the entity's counted targets reaches 0 and the entity has no `Pinned`. **Despawn redirect (N2):** `delete_entity_core` (X-22), for archetypes with the new `ArchetypeFlags::COUNTED_TARGET` bit (one of the free bits 13–15, `M:…PHYSICS…:1408`), sums the counted targets on the cold path. If the sum is > 0 it removes `Pinned` and returns `DespawnOutcome::Deferred`. An `AssetSentinel` refuses despawn with a cold log | KF-relation-count | every asset reference, including asset → asset roles (N3) | here; the `TexRef<R>` consumers need EK22 |
> | EK15c **(rev 3)** | Relation storage on K7 | `Segmented` impl holding `SpanRef<Entity>`. **Trait change (N4):** `RelationshipSourceCollection` gains `type Ctx` (`()` for Vec/Exclusive/Ordered/CountOnly; the owner type's `SegmentedColumn<Entity>` for `Segmented`), passed `&mut` by the generic command applies and fetched by raw projection disjoint from the target's table bytes (the `apply_via_raw_twin` discipline, `relationship/mod.rs:29-31`). **Free path:** a derive-installed `release_fn` (EK22's install table) that the structural core calls on remove, despawn and clear. One `SegmentedColumn` per owning component type | KF-relation-collection-on-kernel-storage | `Children`, `Bindings`, every one-to-many target | here, after K7 and the K7 input (N5) |

**Removed (verbatim):**
> | EK17 | Asset store dedup (Q1b only) | Retiring rows, lane split, `iter_mut` | — | — | here |

**Added:**
> | ~~EK17~~ | **deleted**: Q1 = (a) is decided | — | — | — | — |

**Removed (verbatim):**
> | K6′ **(new)** | Horizon on the K6 release | dying entries stamped with a tick; `release_dense_group::<G>(horizon: Tick)` + `_with` visitor | rev 1 `Retiring{epoch}`, KF-assets-adopt-retiring (Q1a) | assets (ED16) | **physics U2** (input filed) |

**Added:**
> | K6′ **(rev 3, re-filed)** | Stamped horizon on `release_dying` | `RELEASE = DeferredStamped` (`died` stamps); `DenseGroupMut::release_dying_before(horizon, visit)`; `release_dying_with(visit)` (N5); `EcsMaster::release_dense_group_at_teardown(&TeardownToken, visit)`. Physics's `release_dying()` is unchanged (horizon "all") | rev 1 `Retiring{epoch}` | assets (ED16), K7 group banks | **physics** (input filed; lands with or after U2) |

**Removed (verbatim):**
> - K3 (dense groups): asset values (ED5).

**Added:**
> - K3 (dense groups) and the D13 binder: three asset groups (ED5), consuming 4 of 8 anchors together with `PhysicsBody`. Group columns are consumed as rev 3 defines them: untracked, written only through `DenseGroupMut`, and excluded from save.

**Removed (verbatim):**
> 1. **Physics-owned, in physics's rung order:** K1 (U1) → K2 + K3 + K6 [+ K6′] (U2) → K4 (U3) → K5a/b (P1/P2) → K7 (S0).
> 2. **This design's, interleaved by dependency:** EK15a → EK15b → EK2 → EK5 → EK3 → EK4 → EK6 → EK7 → EK8 → EK9 → EK10 → EK11 → EK12. The EK1 `Default` follows K1. EK15c follows K7. Then EK16 → EK18 → EK19 → EK20 → EK21. EK17 only if Q1 = (b).

**Added:**
> 1. **Physics-owned, in physics's rung order:** K1 (U1) → K2 + K3 + K6 (U2) [+ K6′ once accepted] → K4 (U3) → K5a/b (P1/P2) → K7 (S0) [+ the K7 input: DEAD `SpanRef`, release visitor].
> 2. **This design's, interleaved by dependency:** EK22 → EK15a → EK15b → EK2 → EK5 → EK3 → EK4 → EK6 → EK7 → EK8 → EK9 → EK10 → EK11 → EK12.
>    - The EK1 `Default` follows K1.
>    - EK6g follows K3.
>    - EK15c follows K7 and its input.
>    - Then EK16 → EK18 → EK19 → EK20 → EK21.

Depends on: ED5, ED15, ED16, ED20, §13.

---

## P-§10: public API

**Removed (verbatim):**
```
pub struct CountOnly(u32); impl RelationshipSourceCollection for CountOnly;                         // EK15b
//   + `const ITERABLE: bool` on the trait (default true; CountOnly = false) gating the despawn cascade at compile time
pub struct Segmented(SpanRef<Entity>); impl RelationshipSourceCollection for Segmented;              // EK15c
```
**Added:**
```rust
pub struct CountOnly(u32); impl RelationshipSourceCollection for CountOnly;                         // EK15b
//   + `const ITERABLE: bool` (default true; CountOnly = false); RelationshipTarget gains `const DESPAWN_AT_ZERO: bool = false`
pub enum DespawnOutcome { Despawned, Deferred /* counted target > 0: unpinned instead (N2) */, Refused /* sentinel */ }
pub struct Segmented(SpanRef<Entity>); impl RelationshipSourceCollection for Segmented;              // EK15c
//   RelationshipSourceCollection gains `type Ctx;` and `ctx: &mut Self::Ctx` on add/remove/with_capacity/clear (N4)
pub fn component_id_for<T: Component>(install: fn(u32)) -> ComponentId;                            // EK22
pub struct GroupEditsMut<'w, G: DenseGroup>; impl<G: DenseGroup> GroupEditsMut<'_, G> { pub fn mark(&mut self, slot: u32); } // EK6g
pub struct GroupEdits<'w, G: DenseGroup>;    impl<G: DenseGroup> GroupEdits<'_, G> { pub fn read(&self) -> EditWindow<'_>; }
pub enum EditWindow<'a> { Complete(EditIter<'a> /* yields slot: u32 */), Overflowed }
pub struct TickEpoch;                                                                                 // EK5 (read-only param)
```

**Removed (verbatim):**
```
impl Commands<'_, '_> { pub fn release_dense_group<G: DenseGroup>(&mut self, horizon: Tick); }      // K6′
impl EcsMaster { pub fn release_dense_group_with<G: DenseGroup>(&mut self, horizon: Tick, f: impl FnMut(u32)); }
```
**Added:**
```rust
// physics-owned K6′ (input, ED16); release_dying() itself is unchanged
impl<G: DenseGroup> DenseGroupMut<'_, G> {
    pub fn release_dying_before(&mut self, horizon: Tick, visit: impl FnMut(u32)) -> u32; // G::RELEASE == DeferredStamped
    pub fn release_dying_with(&mut self, visit: impl FnMut(u32)) -> u32;                  // visitor before DEAD fill (N5)
}
impl EcsMaster { pub fn release_dense_group_at_teardown<G: DenseGroup>(&mut self, t: &TeardownToken, visit: impl FnMut(u32)); }
```

**Removed (verbatim):**
```
pub fn render_acquire(/* NonSendMut<Renderer>, NonSendMut<PresentChain>, NonSendMut<FrameInFlight>, Res<GatherTick> */);
pub fn render_retire(world: &mut EcsMaster, ctx: ExclusiveCtx);                                // R1
pub fn render_record(/* NonSendMut<Renderer>, NonSendMut<FrameInFlight>, Res<MeshRenderScratch>, Res<UiPackLanes>, … */);
```
**Added:**
```rust
pub fn render_acquire(/* NonSendMut<Renderer>, NonSendMut<PresentChain>, NonSendMut<FrameInFlight>, TickEpoch */);
pub fn render_retire<K: GpuAssetKind>(group: DenseGroupMut<K::Group>, lane: NonSendMut<K::DeviceLane>,
                                      frame: NonSend<FrameInFlight>);                          // R1 ×3
pub fn render_record(/* NonSendMut<Renderer>, NonSendMut<FrameInFlight>, Res<MeshRenderScratch>, Res<GatherTick>,
                        Option<Res<UiPackLanes>>, Option<NonSend<UiGpu>>, … */);               // UI optional (Q2b)
pub struct AssetRead<'w, K: GpuAssetKind>;   // hand-written SystemParam: DenseColumn<K::Value> + GroupSlot reads
pub struct AssetWrite<'w, K: GpuAssetKind>;  // hand-written SystemParam: DenseGroupMut + GroupEditsMut + GroupSlot reads
impl<K: GpuAssetKind> AssetWrite<'_, K> { pub fn get_mut(&mut self, h: Handle<K>) -> Option<&mut K::Value>; } // marks
```

**Removed (verbatim):**
```
pub fn ui_hit_test<A: Actionlike>(/* Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>,
    Res<PhysicalInput>, ResMut<UiPointerState>, ResMut<UiActionPresses<A>>, Commands */);
```
**Added:**
```rust
pub fn ui_hit_test<A: Actionlike>(/* Query<(&ComputedRect, &ComputedPaintKey, &ComputedEffectiveClip, &mut Interaction)>,
    Res<PhysicalInput>, Query<(&WindowCursor, &UiViewport, &mut UiPointerState, &PointerPlayer), With<PrimaryWindow>>,
    Query<&mut UiActionPresses<A>, With<Player>>, Commands */);
// boyko_input
pub enum InputSet { Physical, Sources, Actions, Listeners }
pub fn action_fold<A: Actionlike>(/* Query<(&InputMap<A>, &mut ActionState<A>, Option<&UiActionPresses<A>>), With<Player>>,
    Res<InputBindings<A>>, Res<PhysicalInput> */);
// boyko_app
pub struct UiPlugins<A: Actionlike>;   // ED19; not part of EnginePlugins
```

---

## P-§11: multithreading

**Removed (verbatim):**
> - **Shared in Main:** Send resource-columns (`MeshRenderScratch`, `UiPackLanes`, `MeshMeta` group column, `PhysicalInput`, `UiActionPresses<A>`, `GatherTick`) and components. Each has exactly one writer system per set (G-GRAPH).

**Added:**
> - **Shared in Main:** Send resource-columns (`MeshRenderScratch`, `UiPackLanes`, `PhysicalInput`, `InputBindings<A>`, `GatherTick`, `ViewUniforms`), the asset group columns, and components (`UiActionPresses<A>` and `ActionState<A>` on players; window components). Each has exactly one writer system per set (G-GRAPH).
> - **Group edit log:** one access node per group. `GroupEditsMut` declares a write on it and `GroupEdits` a read, so the scheduler serialises them. No atomics. Appends are in non-decreasing tick order because each marker uses its own `this_run`, and ticks are monotone across schedules.

**Removed (verbatim):**
> - The asset release horizon is proven in ED16.

**Added:**
> - The asset release horizon is proven in ED16. `completed_gather` is written only by R0, after a fence wait, and `submitted_gather` only by R5, after a submit. Both are dispatcher-only, inside NonSend state.
> - R1's `DenseGroupMut` and Main's asset writers run in different schedules. Within Main, the writers of one group serialise on the group's column and recycle nodes (`M:…PHYSICS…:1474`).

---

## P-§12: gates and rungs

**Removed (verbatim):**
> - **G-GRAPH.** A headless `EnginePlugins` app. Checks every §6 system, set and edge, including `UiBindSet` after `GameplaySet`. Main exclusive systems must equal `{ui_bind_apply}`. Each resource-column has one writer.

**Added:**
> - **G-GRAPH (rev 3, B5).** Two headless apps; each resource-column has one writer in both.
>   - **(a) `EnginePlugins` alone.** Every non-UI §6 system, set and edge is present. No system from `boyko_ui` or `boyko_render::ui` is registered. There is no `UiPackLanes` resource. The Main exclusive set is empty. R5 validates with its `Option` UI params absent.
>   - **(b) `EnginePlugins` + `UiPlugins::<TestAction>`.** The Main exclusive set is `{ui_bind_apply}`. `UiBindSet` runs after `GameplaySet` and `UiLogicSet`. `ui_hit_test` is in `InputSet::Sources`. `upload_mirror::<UiPack>` is in R4. `UiGpu` is in the EK11 rank-3 list.

**Removed (verbatim):**
> - **G-ALLOC.** A counting allocator, steady frames 10..20. Budget only decreases, and the data path is 0. Anti-vacuity: UI nodes > 0 and instances > 0.

**Added:**
> - **G-ALLOC.** A counting allocator, steady frames 10..20, in an app composing **`EnginePlugins` + `UiPlugins`** (Q2b). Budget only decreases, and the data path is 0. Anti-vacuity: UI nodes > 0 and instances > 0.

Rung rows. Each row below replaces the rev-2 row with the same name. Removed (verbatim):
> | IN2 | `ActionState`/`InputMap`/`ACTION_NAMES` onto EK1 columns (CSR) and a Resource | K1 + EK1 `Default` | G-FORM −6; bit microbench unchanged |
> | **IN3 (rev 2)** | `UiActionPresses<A>`; `ui_hit_test::<A>`; delete `ui_press` and the re-freeze | UI0 | a UI press visible in the same frame's `ActionState<A>`; **two-plugin gate**: `FlyAction` + `GameAction`, a button bound to `GameAction` index k, a click ⇒ `FlyAction` index k not pressed |
> | HO3 | `WindowSurface` (EK5); `UiViewport` derived | EK5 | red-first: no production writer of `UiViewport` |
> | HO4 | Render schedule R0–R6; `WindowHost` → NonSend residents, including `ParticleGpu`; `FrameInFlight` | EK10, EK11 | G-LOOP → 0; G-GRAPH; minimized-window test; token debug_asserts |
> | **RE4 (rev 2)** | FIF mirrors (RF1) for ~17 uploads; delete G3 | EK5, HO4 | G-FORM −3; "unchanged CSM ⇒ 0 B uploaded"; **"grow with unchanged source ⇒ the fresh ring is written"** (O1) |
> | **RE6 (rev 2)** | material and mesh-meta device lanes indexed by asset slot | AS3 | material edit after boot reaches the GPU (`material_table.rs:41`); G-FORM row label change only |
> | **AS2–AS5 (rev 2)** | Q1(a): AS2 asset value as a K3 Deferred group + R1 release at the gather-tick horizon (K6′) → AS3 slot identity + slot-0 sentinels + the 4096 texture ceiling → AS4 count-only relation (EK15b) → AS5 staging/paths components + save/prefab/clear classification | Q1, K3, K6′, EK6, EK15b | slot-reuse proptest (a stale `Handle` is rejected; no slot is released before `completed_gather_tick` exceeds its death tick); first user asset gets slot ≥ 1; save/load round-trip keeps instance → asset links by `AssetPath`; G-FORM −12 |
> | UI2 | plugins host everything (Q2); root caches → queries; viewport derived | UI0, HO3 | G-GRAPH; G-FORM −5 |
> | **UI4 (rev 2)** | paint walk (`ComputedPaintKey`, `ComputedEffectiveClip`); `Interaction`-filtered scan; triggers; typed tags | EK8 | **oracle equivalence** against the old focus DFS **and** the old render DFS, on a corpus with overlapping roots, `StackIndex` ties, nested clips, and nodes with `Interaction` but no `Focusable` (same hovered entity, same pack order); G-PERF hit-test |
> | UI7 | fonts, sheets, textures become assets/handles | AS series or Q1(b), UI6s | B-4: FontId ≥ 1 samples its own atlas (a two-font golden) |
> | SC3 | `RenderView` + `ActiveView` | — | ≤ 1 active; uniform identical |
> | K-EK15c | `Children`/`Bindings` onto `Segmented` | K7 | proptest against a `Vec` model under random link/unlink/despawn; the 10k-append test records total copies ≤ 2n; Miri on `SpanRef` reads across a sibling span's relocation |

**Added:**
> | **IN2 (rev 3)** | Player entities (ED18): `ActionState<A>` component with fixed `A::Lanes` (the Actionlike derive emits `Lanes`); `InputMap<A> { profile }` + the per-A `InputBindings<A>` CSR bank; `ActionNames<A>` resource; `action_fold::<A>` over players; `DefaultPlayer` spawned at `InputPlugin` build; every `Res<ActionState<…>>` reader migrated (`fly.rs:78`, `expand.rs:1683`, the input fold, the UI dispatch); `RebindSession<A>.player`; `.keys` per profile | K1 + EK1 `Default`, EK22 | G-FORM −6; bit microbench unchanged; two `InputPlugin`s ⇒ exactly one `DefaultPlayer`; `ActionState<FlyAction>` and `ActionState<GameAction>` have distinct ComponentIds and columns (the B4 red-first); a second player on a split-keyboard profile does not see the first player's keys |
> | **IN3 (rev 3)** | `UiActionPresses<A>` component on players; `PointerPlayer` routing; `ui_hit_test::<A>`; delete `ui_press` and the re-freeze | UI0, IN2, HO3, EK22 | a UI press visible in the same frame's `ActionState<A>` (including the cold first-press insert); **two-plugin gate** as rev 2; **two-player gate:** P1 and P2 both carry `ActionState<GameAction>`, the window's `PointerPlayer` = P1, and a click on a button bound to k ⇒ P1's k pressed and P2's k not pressed; re-pointing to P2 flips it |
> | **HO3 (rev 3)** | Primary window entity (ED18): `Window`/`PrimaryWindow`/`WindowDesc`/`WindowSurface`/`WindowCursor`/`PointerPlayer`; `window_fold` from `RawInput` resize/DPI/cursor/focus events; one-window assert | IN1, EK9 | red-first: no production writer of `UiViewport` (closed by UI2's deriver); a second `Window` spawn panics at the next `window_fold` naming the multi-window rung; headless app: no window entity, no panic; G-FORM label relabel |
> | HO4 **(rev 3)** | Render schedule R0–R7; `WindowHost` → NonSend residents, including `ParticleGpu`; `FrameInFlight` with the latch fields | EK10, EK11 | G-LOOP → 0; G-GRAPH (both apps); minimized-window test (no `submitted_gather` write on a skipped frame); out-of-date test (same); token debug_asserts |
> | **RE4 (rev 3)** | FIF mirrors (RF1) for ~17 uploads; delete G3; the view mirror sources today's `ViewUniform` until SC3, then `ViewUniforms` (N8) | EK5, HO4 | G-FORM −3; "unchanged CSM ⇒ 0 B uploaded"; "grow with unchanged source ⇒ the fresh ring is written" (O1); **"a forced `TickEpoch` change ⇒ every mirror uploads once"** (O2) |
> | **RE6 (rev 3)** | material and mesh-meta device lanes indexed by asset slot; the material upload driven by the edit log | AS3, EK6g | red-first: a material edit after boot reaches the GPU (`material_table.rs:41`); a frame with no edit stages 0 B; a frame with k edits stages k × `size_of::<MaterialGpu>()`; a reader skipped past the horizon with an edit in the gap stages the whole table once; G-FORM label change only |
> | **AS2 (rev 3)** | Asset entities; `MeshGroup`/`MaterialGroup`/`TextureGroup` (`DeferredStamped`, `EDIT_LOG`) anchored on `MeshAsset`/`MaterialAsset`/`TextureAsset` at mint; DEAD = loading + `AssetReady`; `AssetRead`/`AssetWrite` facades; R1 `render_retire::<K>` at `completed_gather`; `Handle<T>` minted from the entity | K3 (physics U2), K6′ accepted, EK6g | **FIF-slot-reuse model proptest** (§17); a stale `Handle` is rejected; physics's debug reconciliation (Σ anchored rows == `live_count
> | **AS2 (rev 3)** *(continued)* | … | … | … (the gate cell, continued) `live_count` after every window, `M:…PHYSICS…:1398`) holds for all three asset groups; **Q1 overturn gate:** the handle → slot resolution microbench within its band (G-PERF, owed under Q5) |
> | **AS3 (rev 3)** | slot identity (slot = group slot); slot-0 sentinels (`AssetSentinel`, pinned) spawned in `StartupSet::Resources` before any user asset; the 4096 texture ceiling, loading textures included; null descriptor on anchor | AS2 | first user asset gets slot ≥ 1; a despawn of a sentinel is refused; no recorded command reads an unwritten descriptor (validation-ON run, device leg) |
> | **AS4 (rev 3)** | counted relations for every asset reference, including the asset → asset roles `TexRef<R>` (N3); `asset_slot_sync`; despawn redirect; `DESPAWN_AT_ZERO` | EK15b, EK22 | counts equal relations under random link/unlink/despawn; **N2 red-first:** `despawn(asset)` with count > 0 leaves the asset alive and unpinned, and it dies when the last instance unlinks; **N3 red-first:** a texture named only by a material survives the unlink of its last widget; the redirect fires on every path (command, direct, `despawn_without_children`, hierarchy cascade) |
> | **AS5 (rev 3)** | `Staged<A::Cpu>` (generic, EK22) and `AssetPath` components; the four table kinds as asset entities; save/prefab/clear classification; `clear_gameplay()` | AS4, EK22 | save/load round-trip keeps instance → asset links by `AssetPath`; group columns are absent from the save file; `clear_gameplay()` keeps assets; `EcsMaster::clear()` on a world with live asset lanes trips the debug assert; G-FORM −12 |
> | **UI2 (rev 3)** | `UiPlugins` (ED19) hosts every UI system and the render-side UI; root caches → queries; `UiViewport` derived on the window entity; `InputSet` sub-sets; `boyko_demo` and the playground compose `UiPlugins` | UI0, HO3 | G-GRAPH apps (a) and (b); G-FORM −5; a UI-less app records and presents (R5 with the `Option` params absent) |
> | **UI4 (rev 3)** | paint walk + `PaintGen` + `ui_paint_sweep`; `Interaction`-filtered scan per window; triggers; typed tags; the pack skips `UNREACHED` | EK8 | **oracle equivalence** against the old focus DFS **and** the old render DFS, on a corpus with overlapping roots, `StackIndex` ties, nested clips, nodes with `Interaction` but no `Focusable`, **and detached subtrees**: `remove::<ChildOf>`, `remove::<UiRoot>`, a parent despawned with `despawn_without_children`, and a never-attached spawn. Detached nodes are neither hovered nor packed, and a re-attached node is hit again; G-PERF hit-test |
> | **UI7 (rev 3)** | fonts, sheets and textures become assets and handles (table kinds for font and sheet; `TexRef<FontAtlas>`, `TexRef<SheetImage>`) | AS series, UI6s | B-4: FontId ≥ 1 samples its own atlas (a two-font golden) |
> | **SC3 (rev 3)** | `RenderView` + `ActiveView`; `ViewUniforms` derived | — | ≤ 1 active per window (v1: ≤ 1 in the world); uniform identical; the `ViewUniforms` row equals the active camera's `RenderView` bytes |
> | **K-EK15c (rev 3)** | `Children`/`Bindings` onto `Segmented`; `type Ctx` on the collection trait; `release_fn` free path | K7 + the K7 input (N5) | proptest against a `Vec` model under random link/unlink/despawn/clear, with **frontier and free-list totals bounded by live spans** (no leak under churn); the 10k-append test records total copies ≤ 2n; Miri on `SpanRef` reads across a sibling span's relocation; two owner types written in one wave share no free list |

**Added (new rungs):**
> | **K-EK22 (new)** | generic component mint; the derive accepts generics | — | two instantiations get distinct ids and columns; hooks, storage and require installs run once per instantiation; a generic relation (`TexRef<R>`) links and unlinks with correct counts; a generic component without a serialize name is save-Ignore; the uncached `T::component_id()` cost is recorded (G-PERF, owed) |
> | **K-EK6g (new)** | group edit log | K3 (physics U2) | mark dedup within one run; inclusive-window at-least-once semantics against the `Mut<T>` oracle (`mut_.rs:54`); exact overflow (§17); a const-assert rejects `mark` on a group without `EDIT_LOG`; the Fixed-reader retention rule; consumer-free anti-vacuity |

Depends on: ED5, ED8, ED9, ED16–ED20, §17.

---

## P-§13: hand-offs

**Removed (verbatim):**
> | Physics study | K7 (= EK14) | ED15's shape: header held by the owner, pow2 classes, relocate-on-grow, LIFO class free lists, no compaction; prerequisite stays K3 for group-bank consumers | EK15c waits for physics S0; no UI rung waits |
> | Physics study | K6 → K6′ | dying entries stamped with a tick; `release_dense_group::<G>(horizon)` + `_with` visitor; physics passes `this_run` (today's semantics) | AS2 waits for physics U2 |

**Added:**
> | Physics study | K7 (= EK14) | ED15's shape, plus `SpanRef::DEAD` and the rule that a group holding `SpanRef` columns releases through `release_dying_with(visit)` (N5); one `SegmentedColumn` per group column | EK15c waits for physics S0 and this input; no UI rung waits |
> | Physics study | K6 → K6′ (re-filed against rev 3) | `RELEASE = DeferredStamped` with `died` stamps; `DenseGroupMut::release_dying_before(horizon, visit)`; `release_dying_with(visit)`; `EcsMaster::release_dense_group_at_teardown(&TeardownToken, visit)`. **Physics keeps `release_dying()` unchanged (horizon "all").** No byte of `PhysicsBody` changes | AS2 waits for acceptance and physics U2 |
> | Physics study | tick-consumer census (`M:…PHYSICS…:1335`) | a row for EK6's id-based `any_changed_since`: the route refuses `StorageKind::Group` and untracked ids at param setup (O3) | K-EK6 lands with the row |
> | Physics study | anchor budget | this design consumes 3 anchors (Mesh, Material, Texture); with `PhysicsBody`, 4 of the 8 are used (N6). No wider mask is requested | informational |
> | Physics study | `EcsMaster::clear()` | legal for worlds with asset groups only at teardown; `clear_gameplay()` is the gameplay reset (X-28) | informational |

**Removed (verbatim):**
> | Transparency plan (R11) | `MaterialXGpu` second SSBO | could be a second NonSend lane indexed by the material asset slot (ED4/AS3), gated by the same `Changed` | transparency's call |

**Added:**
> | Transparency plan (R11) | `MaterialXGpu` second SSBO | could be a second NonSend lane indexed by the material asset slot (ED4/AS3), fed by a second reader of the `MaterialGroup` edit log (EK6g; multiple readers need no coordination) | transparency's call |

---

## P-§14: owner questions

**Added (before `**Owner questions:**`):**
> **Decided 2026-09-11** (`ENGINE-RUNTIME-ECS-DECISIONS.md`): Q1 (a), Q2 (b), Q3 (b), Q4 (a), Q5 on the owner's word. The option texts below are kept as the record of what was put to the owner. Rev 3 re-cuts §3–§17 to the decided options (ED5, ED18, ED19).
>
> One premise in the Q1 decision text is refuted by physics rev 3: "asset change detection becomes ordinary `Changed<T>`". Group columns carry no ticks (X-18). For mesh, material and texture the signal is the group edit log (EK6g); the four table kinds keep `Changed<T>`. The decision itself stands: its overturn gate is the handle-resolution microbench (AS2), which this does not touch.

---

## P-§16: integration

**Removed (verbatim):**
> - **boyko_ecs:** EK2–EK12, EK15a/b/c, EK16–EK21; `CoreSchedule::Render`; the EK3 hook in `remove_command.rs` / `set_enable_bit`; the event policy; the serialize seam. Physics-owned K1/K3/K6′/K7 are consumed.

**Added:**
> - **boyko_ecs:**
>   - EK2–EK12, EK6g, EK15a/b/c, EK16, EK18–EK22; `CoreSchedule::Render`.
>   - The EK3 hook in `remove_command.rs` / `set_enable_bit`.
>   - The despawn redirect in `entity_api.rs` `delete_entity_core`, and `ArchetypeFlags::COUNTED_TARGET`.
>   - `component_id_for` next to `resource_type_registry.rs`.
>   - The event policy; the serialize seam.
>   - Physics-owned K1/K3/K6/K7 are consumed; the K6′ and K7 inputs are filed.
> - **boyko_macros:** `component.rs` accepts generics (EK22); `actionlike.rs` emits `type Lanes`.

**Removed (verbatim):**
> - **boyko_app:** the runner becomes pump + update; `host.rs` fields become residents; `plugins.rs` gains `UiPlugins` (Q2) and render registration; `light_gate.rs` and `particle_gate.rs` are deleted.

**Added:**
> - **boyko_app:**
>   - The runner becomes pump + update; `host.rs` fields become residents.
>   - `plugins.rs` defines `UiPlugins<A>`, which is not part of `EnginePlugins` (Q2b), and the render registration.
>   - The host spawns the primary window entity and sets its `PointerPlayer`.
>   - `fly.rs` reads `ActionState<FlyAction>` from the `DefaultPlayer`.
>   - `light_gate.rs` and `particle_gate.rs` are deleted.

**Removed (verbatim):**
> - **boyko_input:** `RawInput`, the fold, EK1 CSR.

**Added:**
> - **boyko_input:** `RawInput`; the fold over players; `InputSet` sub-sets; `window` and `player` modules (ED18); `ActionState<A>`/`InputMap<A>` as components; the `InputBindings<A>` bank.
> - **boyko_render:** a normal dependency on `boyko_input`; `render_retire::<K>`; the facades; the edit-log-driven material upload; `ViewUniforms`; `UiRenderPlugin`.
> - **aether_lang:** `expand.rs:1683` emits the `DefaultPlayer` query in place of `Res<ActionState>`.

---

## P-§17: validation

**Removed (verbatim):**
>   - a reader skipped for 2 frames gets `Overflowed` and one diagnostic;

**Added:**
>   - A reader that runs every frame never gets `Overflowed`, including with records stamped `this_run + 1` (N1's example).
>   - A skipped reader gets `Overflowed` and one diagnostic if and only if a truncated record fell in its window.
>   - A Fixed reader at a frame-to-fixed rate ratio of 4 never overflows (ED17 rule).

**Removed (verbatim):**
>   - despawning a target with count > 0 retires it at count 0;

**Added:**
>   - A despawn request on a target with count > 0 returns `Deferred` and unpins it, on every despawn path; the target dies at count 0.
>   - A sentinel despawn is refused.
>   - The sum across role-typed targets gates the despawn: an asset used as albedo by one material and as a UI image by one widget dies only after both unlink.

**Removed (verbatim):**
> - K6′ (at physics U2): `this_run` reproduces today's release set exactly.

**Added:**
> - **K6′:**
>   - `release_dying()` is byte-identical to rev 3's.
>   - `release_dying_before(h)` releases exactly {dying : `died` strictly older than h}, appends them to `free` in dying order, and keeps retained entries in order.
>   - Both visitor forms call the visitor once per slot, before the DEAD fill.
>   - `release_dense_group_at_teardown` without a token does not compile, and with a token releases every dying entry.
> - **FIF-slot-reuse horizon model (B3), proptest.**
>   - FIF ∈ {2, 3}. Removals are drawn at random in Fixed apply windows, the state pass (stamp g), Main apply windows before and after the gather (g + 1), and Render apply windows. Frames are skipped at random (minimized, out-of-date, no submit).
>   - The model tracks each submitted frame's named set, and completion as "the frame last submitted from slot s is complete at R0's wait on s".
>   - Assert: no slot released at R1(N) is in the named set of any incomplete frame.
>   - Anti-vacuity: ≥ 1 release per 10 frames on average.
>   - **Mutation:** rev 2's read order (`gather_tick[slot]` read after R0's overwrite) makes the model red.
> - **EK22:** distinct ids per instantiation; installs run per instantiation.
> - **EK6g:** see K-EK6g.
> - **UI (N7):** the detached-subtree corpus (UI4).
> - **G-GRAPH:** both apps (B5).

**Removed (verbatim):**
> - UI: `ComputedPaintKey` is unique and the hit result equals the focus-DFS oracle; the pack order equals the render-DFS oracle.

**Added:**
> - UI: `ComputedPaintKey` is unique among reached nodes, and the hit result equals the focus-DFS oracle. The pack order equals the render-DFS oracle. Under random detach and re-attach, unreached nodes are neither hit nor packed.

**Removed (verbatim):**
> - at most one `ActiveView`;

**Added:**
> - at most one `ActiveView` per window (v1: in the world);
> - at most one `Window` entity (a release assert in `window_fold`);
> - exactly one `DefaultPlayer`;
> - `completed_gather` is non-decreasing (outside a `TickEpoch` re-base);
> - `submitted_gather[s]` is written only after a successful submit;
> - edit-log appends are tick-non-decreasing;
> - no `UNREACHED` row is packed;

**Removed (verbatim):**
> - tick wrap (`check_ticks` clamps EK3, EK5, K6′ stamps and `gather_tick`);

**Added:**
> - tick wrap: `check_ticks` clamps kernel-owned ticks, namely the EK3 and EK6g records and watermarks, EK5, and K6′ `died`. Render-owned ticks (`FifMirrorState`, `FrameInFlight`) re-base when `TickEpoch` changes (X-29). Rev 2's claim that `check_ticks` clamps `gather_tick` was false.

**Added (edge cases):**
> - a second `InputPlugin` (one `DefaultPlayer`);
> - a window whose `PointerPlayer` was despawned (presses dropped, one diagnostic);
> - a headless app (no window, no UI);
> - a texture referenced by a material while still loading (null descriptor);
> - a material edit, then the slot released and reused in the same edit window (idempotent upload);
> - a minimize longer than the tick wrap horizon (re-base);
> - a `SpanRef` group slot released (span freed before DEAD).

---

## P-§18: open questions

**Removed (verbatim):**
> - **O-9 (new):** whether physics accepts ED15's K7 shape and K6′. Until it does, EK15c and AS2 are blocked, and nothing else is.

**Added:**
> - **O-9 (rev 3):** whether physics accepts ED15's K7 shape plus the N5 input, and the re-filed K6′ (four additive items, zero bytes for `PhysicsBody`). Until it does, EK15c and AS2–AS5 are blocked, and nothing else is.
> - **O-11 (new):** when to land the multi-window rung: per-window swapchain, `RawInput.window`, `ViewTarget`, and a per-node window key in the hit-test. v1 refuses a second window.
> - **O-12 (new):** per-device input routing for local multiplayer. It needs a device id in `CapturedMsg` (`window.rs:89-107`). v1 players share key state.
> - **O-13 (new), not verified:** whether the release profile's linker config keeps section GC on (ED19's D31 answer rests on it).

---

## Responses without a section edit

- **B3's value.** I agree with the critic that the correct horizon is g_{N−2}. ED16 records that the tighter g_{N−1} bound was considered and not taken.
- **The critic's "9 files" count** for `Res<ActionState<…>>` readers was not re-verified. IN2's gate is compiler-driven (every reader fails to compile until migrated), so the count does not affect correctness.

---

## Changelog: rev 2 → rev 3

| Finding or impact | Action | Section |
|---|---|---|
| B1 group columns carry no ticks | FIX. The kernel group edit log EK6g (opt-in, own node, beside `DenseGroupStore`). Anchors `MeshAsset`/`MaterialAsset`/`TextureAsset` at mint. Writers named per kind through the `AssetWrite` facade. Loading = DEAD + `AssetReady` disabled. Null descriptor on anchor. Q1's `Changed<T>` premise refuted for the three group kinds | X-18, ED3, ED5, §4.2, §5, §6, §9, §10, RE6, AS2 |
| B2 K6′ amends a deleted API; wrong physics horizon | FIX. Re-filed on `DenseGroupMut` (`DeferredStamped`, `release_dying_before`, `release_dying_with`, teardown-token form). Physics horizon = "all" (`release_dying()` unchanged). R1 = function systems | X-19, ED16, §6, §9, §10, §13 |
| B3 horizon read after overwrite | FIX. `submitted_gather` written only on submit; `completed_gather` latched by R0 after the fence wait. Proof restated for any stamp ≤ g. FIF-slot-reuse proptest with a mutation check | X-20, X-25, ED16, §5, §6, §17 |
| B4 no generic component mint | FIX. EK22 `component_id_for` over `TypeIntern`, derive accepts generics. `ActionState` fixed `A::Lanes`. `InputMap` = profile + shared per-A bank. Rev 2's `Staged<A::Cpu>` had the same defect | X-21, ED20, §5, §9, IN2 |
| B5 G-GRAPH / R5 / G-ALLOC vs opt-in UI | FIX. G-GRAPH as two apps. R5's UI inputs are `Option`. G-ALLOC composes `UiPlugins` | ED19, §6, §10, §12 |
| N1 overflow fires on ordinary reads | FIX. Exact watermark rule over the inclusive window. Fixed-reader retention rule | X-30, ED17, §17 |
| N2 no despawn-veto mechanism | FIX. Redirect in `delete_entity_core` via `COUNTED_TARGET`; `DESPAWN_AT_ZERO`; sentinel refusal | X-22, EK15b, ED5, AS4 |
| N3 asset → asset raw slots | FIX. Role-typed counted relations `TexRef<R>` (EK22); derived slots via `asset_slot_sync` | X-23, ED5, §4.2, AS4 |
| N4 K7 relations need a trait context and a free path | FIX (lands at K-EK15c). `type Ctx`; derive-installed `release_fn`; one column per owner type | ED15, EK15c, §5, K-EK15c |
| N5 K7 span leak on group release | FIX (input). `SpanRef::DEAD`; `release_dying_with` | ED15, ED16, §13 |
| N6 anchor cap | FIX. Groups only for GPU-indexed kinds: 3, plus physics = 4 of 8. Font, sheet, clip and doc are table kinds | X-24, ED5, §4.1, §13 |
| N7 detached-but-alive nodes | FIX, stronger than asked. `PaintGen` + `ui_paint_sweep` cover `despawn_without_children`, which removal signals miss. Pack skips `UNREACHED`. Corpus extended | X-26, ED8, §4.2, §8, UI4 |
| N8 view mirror sourced from a component | FIX. Derived `ViewUniforms` resource | ED3, ED14, §4.2, RE4, SC3 |
| O1 `UiActionPresses` id and Send | FIX. Now a generic component (EK22) with `PhantomData<fn() -> A>` | ED9, §5 |
| O2 mirror ticks not clamped | FIX by `TickEpoch` invalidation (the kernel cannot reach render state). The same applies to `FrameInFlight`; rev 2's `gather_tick` clamp claim corrected | X-29, ED3, §5, §17 |
| O3 id-based bind path on group/untracked ids | FIX. Setup refusal; census row filed | ED8, EK6, §13 |
| Q2: G-GRAPH, R5, G-ALLOC, `InputSet` sub-sets, demo/playground, D31 | FIX. ED19; D31 decided as no feature | ED19, §6, §12, §18 O-13 |
| Q3: IN2, ED7, IN3 routing, HO3/HO4 windows, EK7/IN4, SC3, consumers, quit action | FIX. ED18: window and player entities, v1 one window, `PointerPlayer`, `EngineAction` on the `DefaultPlayer`, readers migrated | ED7, ED9, ED14, ED18, §4, §5, §6, §12 |
| Q1 + physics rev 3: anchors, writers, release | FIX (see B1, B2, N6) | ED5, ED16 |
| Q4 | no change beyond RE1/LG2 | — |
| New: `EcsMaster::clear()` resets group stores | FIX. `clear_gameplay()`; `clear()` legal only at teardown | X-28, ED5, AS5 |
| New: teardown release refused by `chain_head_registered` | FIX. Teardown-token form | ED16, §5 drop order |
| New: two `InputPlugin`s race to spawn a player | FIX. Build-time spawn | ED18, IN2 |
| New: a loading texture's descriptor is unwritten | FIX. Null descriptor on anchor; R2 orders texture before material | ED5, §6 R2, AS3 |
| New: EK17 obsolete under decided Q1 | deleted | §9 |
| New: edit-log window semantics | adopted the kernel's inclusive at-least-once window (`mut_.rs:54`) | X-30, EK6g |

**Files relevant to this revision:**
- `D:\claude\BoykoEngine\docs\unification\ENGINE-RUNTIME-ECS-DESIGN.md`, the target of this patch.
- `D:\claude\BoykoEngine\docs\unification\ENGINE-RUNTIME-ECS-DECISIONS.md`
- `D:\claude\BoykoEngine\docs\physics\PHYSICS-ECS-UNIFICATION-DESIGN.md`: rev-3 blocks at :1360-1486 and :1643-1732.
- `D:\wt\joltab\crates\boyko_rhi_vulkan\src\present\frame_driver.rs`
- `D:\wt\joltab\crates\boyko_ecs\src\ecs\core\schedule\schedule.rs`
- `D:\wt\joltab\crates\boyko_ecs\src\ecs\core\ecs_master\entity_api.rs`
- `D:\wt\joltab\crates\boyko_macros\src\component.rs`
- `D:\wt\joltab\crates\boyko_input\src\action\state.rs`
- `D:\wt\joltab\crates\boyko_render\src\material.rs`
- `D:\wt\joltab\crates\boyko_render\src\material_table.rs`
- `D:\wt\ui\crates\boyko_ui\src\interaction\focus.rs`
