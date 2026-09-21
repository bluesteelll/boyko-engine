# Engine runtime as ECS - research

- **Date:** 2026-09-11
- **Scope:** every non-physics runtime subsystem: UI, render, host and I/O, world services (scene, assets, serialize, log, diag, time, state, kernel services, reflect), plus what the repository's plans already decided and how reference engines do it.
- **Companion design:** [ENGINE-RUNTIME-ECS-DESIGN.md](ENGINE-RUNTIME-ECS-DESIGN.md)
- **Sibling physics study:** [../physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md](../physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md), [../physics/PHYSICS-ECS-UNIFICATION-DESIGN.md](../physics/PHYSICS-ECS-UNIFICATION-DESIGN.md)
- **Status:** six read-only survey lenses (ui, render, host-io, world-services, plans, reference). Nothing was run: no cargo, no rustc, no benchmarks, no git state change.

## Code trees

The newest code of each subsystem lives in a different worktree, so each subsystem was read from its own tree.

| Letter | Path | Branch | Commit | Read for |
|---|---|---|---|---|
| J | `D:/wt/joltab` | `merge/ke16-into-ecsnative` | `d11962a9` | the kernel, ECS-native physics, the KE16 thread pool, and the newest render / RHI / app / input / scene / log / diag |
| U | `D:/wt/ui` | `feat/ui-advanced` | `615cda8f` | boyko_ui and the render-side UI (16 commits and ~11.4k lines of boyko_ui that J does not have) |
| R | `D:/wt/reflect` | `feat/reflection` | `0e0b4c68` | boyko_reflect (20 commits not in J) |
| M | `D:/claude/BoykoEngine` | `feat/multi-paradigm-render` | `dc35fae8` (HEAD when these files were written) | docs committed there: `docs/animation/`, `docs/physics/ADVANCED-PHYSICS-*`, `docs/render/TRANSPARENCY-*`, `docs/render/REFLECTIONS-*`; and the untracked `docs/memory/ALLOCATOR-*` |

## The owner's orders (verbatim in translation)

Orders 1-3 are given on 2026-09-10 and are copied from the sibling physics study's research file ([../physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md](../physics/PHYSICS-ECS-UNIFICATION-RESEARCH.md)). Order 4 was given on 2026-09-11 and widens the scope from physics to the whole runtime.

1. "There is not one reason to use an allocator other than ours. If something is missing, extend the memory library."
2. "Move ALL runtime data structures onto our system, and all arrays etc. onto the ECS."
3. "The point is not only to move everything onto our allocator, but to bring everything as close as possible to the ECS PARADIGM - in particular in PHYSICS - and to make ONE UNIFIED SYSTEM."
4. "Not only physics must be ECS, but all the runtime things too - that is, for example, the interface included, and so on."

## Provenance tags

- **[S]** source read.
- **[D]** official documentation.
- **[B]** blog post or talk (recorded, not relied on).
- Anything that could not be verified is labelled "not verified", never paraphrased from memory.
- In-tree facts are cited as `tree:path:line` with the quoted line; the tree letter is one of J, U, R, M below.

## How this file is laid out

Each lens report below is reproduced verbatim under its lens heading, followed by that lens's open list (one item per entry, text verbatim). Each report names the tree of every citation it makes.

---

# Lens 1 of 6: ui

## UI lens: boyko_ui checked datum by datum against the "ECS-native" claim

### 0. Trees, provenance, limits

- **U** = `D:/wt/ui` @ `615cda8f`. This is where boyko_ui **and the UI render side** were read. U also rewrote `boyko_render/src/ui/*` against merge-base `5ec1699f`: `gather.rs +541`, `upload.rs`, `pack.rs`, `resources.rs`. So J's `boyko_render/src/ui` is stale for UI. For example, J still has `pub fn host_upload_frame_from_world<F>(` at `J:crates/boyko_render/src/ui/upload.rs:255`, and U deleted it (`U:crates/boyko_render/src/ui/upload.rs:41` "had NO possible caller and is DELETED, not re-signed.").
- **J** = `D:/wt/joltab` @ `d11962a9`. Used for kernel facts only.
- **M** = `D:/claude/BoykoEngine`. Used for the allocator design and the host example.
- **R** was checked once: `R:crates/boyko_reflect/src/lib.rs:1` "`boyko_reflect` — build-gated runtime reflection for the EDITOR, absent from the". It is not a runtime option for the UI.
- No cargo, rustc or benchmark was run. Anything marked **derived** comes from reading only. No outside sources were used.

### 1. Verdict

1. **Durable per-entity data is genuinely ECS.** Widgets are entities. The tree is the kernel `ChildOf`/`Children` relation. Every layout input and output, style, interaction state, binding sink, world anchor and tween channel is a component (table, dense or bitset). I found no per-entity side store.
2. **The side system is transient and lives in the logic.**
   - 13 of the 20 UI systems are exclusive `fn(&mut EcsMaster)`.
   - Each one copies query results into its own Vec held in a Resource, keeps a hand-rolled change window or dirty flag, and walks the tree with random `get_component` probes.
   - Result: about 25 per-frame copy buffers and 5 separate copies of "the root set".
3. **Nothing hosts the core pipeline.**
   - Layout, text measure, world UI, flipbook, render discovery and upload have no plugin. They are "the host's responsibility", and no host registers them: `M:crates/boyko_app/examples/playground.rs:53` "`boyko_app` names no `boyko_ui`, and the".
   - The GPU upload runs outside the scheduler by design.
4. **Text never reaches the GPU in production:** `U:crates/boyko_render/src/ui/gather.rs:389` "`text_uv: None,`".
5. **Three private asset-handle tables** (fonts, sprite sheets, UI texture slots) sit beside the kernel's `Assets<T>`/`Handle<T>`.
6. **Kernel features needed: 7 (KF-A..KF-G), with no new storage kind.** Four blockers on U are already fixed on J (§8).

### 2. The claim, question by question

| Question | Answer | Evidence |
|---|---|---|
| Are widgets entities? | Yes | `U:crates/boyko_ui/src/lib.rs:3` "Widgets are entities; layout inputs/outputs are components; the tree is" |
| Is the tree a relation? | Yes: the kernel `ChildOf`/`Children`. But sibling order is not part of the relation, and consumers disagree about it (§7 B-5). | `J:crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:120` "`pub struct Children(Vec<Entity>);`"; same file `:98` "with `Vec::swap_remove` (O(1), the last child fills the gap), so the order is" |
| Are layout results components or a side buffer? | Final results are a component: `ComputedRect`, single writer. Intermediate results live in a frame-transient side arena, rebuilt per root. | `U:.../components.rs:167` "`pub struct ComputedRect {`"; `U:.../resources.rs:234` "`pub(crate) measured: Vec<MeasuredNode>,`" |
| Text runs and glyph quads? | The content (`UiTextBuffer`, 256 B inline) and the style (`UiText`) are components. Glyph quads are an unfilled Resource Vec. The only callers of `emit_glyphs` are tests. | `U:.../text/emit.rs:65` "`pub glyphs: Vec<GlyphInstance>,`"; `U:.../gather.rs:389` "`text_uv: None,`" |
| Style? | Components (`UiBackground`, `UiImage`, `UiNineSlice`, `UiText`). The asset references inside them are raw indices into private tables. | `U:.../components.rs:442` "`pub texture: u32,`"; `U:.../text/components.rs:51` "`pub font: FontId,`" |
| Dirty tracking: change detection or a private flag? | Change detection at the entry points, collapsed into private flags. Granularity is lost: any change relays all roots and repacks the whole UI. | `U:.../layout.rs:130` "`let inputs_changed = changed.iter().next().is_some();`"; `U:.../layout.rs:28` "dirty, apply re-lays-out ALL roots."; `U:.../binding/bind_system.rs:44` "`pub last_run: Tick,`"; `U:crates/boyko_render/src/ui/pack.rs:885` "`pub generation: u64,`" |

### 3. Data inventory

All paths are U-relative, `crates/boyko_ui/src` unless marked `render` (= `crates/boyko_render/src/ui`).

- **Ptr/grow**: no UI code holds a raw pointer or slice across a grow. The `mem::take` protocol moves Vec headers (`layout.rs:170` "`(s.dirty, mem::take(s))`"). The staging slice is used in the same call and never grows. There is no `par_iter` anywhere in the UI.
- **Lifetimes**: P = persistent, F = per frame, C = rebuilt on a change frame, L = load/reload, A = per asset (setup).

**A. Per-entity components** (persistent, per-entity)

| # | Datum | Evidence | Form | Writer | Reader |
|---|---|---|---|---|---|
| A1 | Layout inputs: `UiLayout` (:22), `UiSpacing` (:73), `UiAlign` (:119), `UiAbsolute` (:133), `ContentSize` (:152), `UiGrid` (:896), `UiAnchor` (:945), `StackIndex` (:184), `ComputedClip` (:192) | `components.rs:22` "`pub struct UiLayout {`" | table components | author / `.ui` / `ui!`; `ui_bar_apply` writes `UiLayout` (`widgets.rs:217`); text measure writes `ContentSize` (`text/measure.rs:91`) | layout, gather, focus |
| A2 | `ComputedRect` | `components.rs:167` | table, 16 B | `write_rect` only (`layout.rs:1689`) | gather, focus, measure, emit |
| A3 | Style: `UiBackground` (:219), `UiImage` (:440), `UiNineSlice` (:558), `UiText` | `text/components.rs:45` | table | author | gather, measure |
| A4 | Asset edges: `UiText.font` (u16), `UiImage.texture` (u32 bindless slot), `UiSpriteSheet.sheet` (u16) | `components.rs:678` "`pub sheet: u16,`" | raw index fields | author | gather, measure, emit |
| A5 | Identity: `UiRoot` (:258), `UiName` (:274, 64 B inline), `Button`, `Bar`, `BarFill`, `UiSourceOrder` | `components.rs:991` "`pub(crate) struct UiSourceOrder(pub u32);`" | table | spawn / reload | layout (roots), reload |
| A6 | Animation: `UiVisual` sink (table, 24 B); `Tween*` channels (dense); `UiSpriteAnim` (table); `UiSpriteCursor` (dense); `UiSpriteSheet.index` written every frame | `components.rs:1220` "`#[component(storage = "dense", on_add = crate::animation::ui_visual_sink_on_add)]`"; `components.rs:836` "`#[component(storage = "dense")]`"; `sprite.rs:416` "`sheet.set_if_neq(UiSpriteSheet {`" | table / dense | `ui_visual_tick`, `ui_sprite_flipbook` | `UiVisual`: **no production reader at U**, because it is absent from the pack list (`render/gather.rs:129-135`); rung A4 is pending |
| A7 | Interaction: `Interaction`, `RelativeCursorPosition`, `FocusPolicy`, `Focusable`, `OnClick`/`OnHover`/`OnSubmit`, plus 3 enable tags registered **by string** | `interaction/plugin.rs:54` "`pub const TAG_UI_HOVERED: &str = "boyko_ui::UiHovered";`" | table + enable bits | `ui_focus_system` | dispatch, styling |
| A8 | Binding: `BindText`/`BindValue` (edge `source: Entity`), `UiTextBuffer` (256 B), `UiValue` | `binding/components.rs:34` "`pub source: Entity,`" | table | author; `ui_bind_apply` | measure, bar, overlay |
| A9 | World UI: `UiWorldAnchor` (edge `EntityAnchor(Entity)`), `UiWorldProjection`, `UiPickable`, `UiWorldCulled`/`Hidden`/`Occluded` (bitset) | `world/components.rs:36` "`EntityAnchor(Entity),`"; `:222` "`#[component(storage = "bitset")]`" | table + bitset | project / pick / visibility | layout |

**B. Resources** (global singletons)

- **B1 `UiViewport`** (POD; host writes; layout/focus/pick read). Its `generation` field hand-rolls resource change detection: `resources.rs:25` "(resources have no `Changed` semantics)." `UiSafeArea` and `UiClock` (`animation.rs:98`) are also POD.
- **B2 `LayoutScratch`** (F/C; written and read by layout):
  - depth pools, 3 × 128 inner Vecs: `resources.rs:216` "`pub(crate) child_pool: Vec<Vec<Entity>>,`" (`:218`, `:226`; seeded at `:326` "`for _ in 0..MAX_LAYOUT_DEPTH {`");
  - flat arenas `measured`/`child_index`/`child_sizes` (`:234`, `:239`, `:247`);
  - a root cache (`:254` "`pub(crate) roots: Vec<Entity>,`");
  - flags.
- **B3 `UiTweenScratch`** (F; tick → reap): `animation.rs:473` "`done: Vec<(EntityId, ComponentId)>,`".
- **B4 `UiSheetTable`** (A): `sprite.rs:257` "`sheets: Vec<UiSheet>,`".
- **B5 `FontTable`/`FontEntry`** (A): `text/font.rs:139` "`fonts: Vec<FontEntry>,`", plus four Box fields (`:33`, `:36`, `:38`, `:41`) cloned from the decoded font: `:72` "`glyphs: font.glyphs.clone().into_boxed_slice(),`".
- **B6 `TextEmitScratch`** (F): no production writer or reader.
- **B7 `UiBindScratch`** (setup id list, plus per-frame widget/archetype lists, private tick, dirty): `bind_system.rs:38` "`pub dynamic_bound_ids: Vec<ComponentId>,`". **`UiBarScratch`** has the same shape: `widgets.rs:71` "`pub last_run: Tick,`".
- **B8 Pointer and focus state.**
  - `UiPointerState` is a fixed array, but it also holds one-frame message slots: `focus.rs:496` "`slot.click_fired = Some((origin, action));`".
  - `UiInputFocus` (`focus.rs:79` "`pub focused: Option<Entity>,`").
  - `UiInteractionConfig` caches the tag ids (`focus.rs:88`).
- **B9 `UiInteractionScratch`** (F): `focus.rs:117` "`candidates: Vec<Candidate>,`". It also holds `stack`, `focusables`, `hover_entered` (`:128`), `query_buf`, `arch_ids`, and `write_nodes` (`:136`), a second copy of the candidate list kept only to satisfy the borrow checker (`:353-355`).
- **B10 World UI resources.** `HoveredWorldEntity` (`world/components.rs:210`) plus a private mirror of its previous value (`visibility.rs:38` "`prev: HoveredWorldEntity,`"). `UiWorldScratch` (F), shared by three systems (`pick.rs:110`, `:117`).
- **B11 `UiHotReload`** (L/P): `reload/state.rs:110` "`pub(crate) doc_roots: SmallRoots,`" (spill `Vec`, `:31`), plus `last_report: UiParseReport` (strings, `:126`).
- **B12 Render.** `UiRenderGeneration` (u64). `UiRenderScratch` (`render/pack.rs:801` "`pub pack: Vec<UiInstance>,`") serves only the legacy path, which has no caller (`render/upload.rs:49` "**It is UNREACHED and it is now WRONG.**").
- **B13 Input.** `PhysicalInput` is read and cloned per frame (J `boyko_input/src/raw/queue.rs:167`; POD `BitSet256`, so the clone does not allocate). `ActionState<A>` is written by UI dispatch (`dispatch.rs:103` "`world.resource_mut::<ActionState<A>>().ui_press(index);`").

**C. System-owned state** (`UiUploadSystem`, C)

- A 1.72 MiB staging box allocated at initialize: `render/upload.rs:198` "`staging: Box<[UiInstance]>,`" and `:669` "`self.staging = vec![UI_INSTANCE_ZERO; UI_STAGING_ROWS].into_boxed_slice();`". It has a hard cap and truncation (`:411` "`let n = if overflowed {`").
- `node_buf` (`:203`), `keys` (`:206`), and `UiGatherScratch` (`render/gather.rs:284` roots, `:289` DFS stack).
- It is system-owned, not a Resource, because Phase 1 sees only a read-only `WorldView`: `render/gather.rs:41-42` "the gather runs against a read-only [`WorldView`], which / cannot project `&mut` to a resource".

**D. GPU side** (driver-owned)

- `RhiContext.ui` holds the pipeline, the MSDF atlas texture, sampler and UBO, the sprite set, and the ring: `render/resources.rs:244` "`slots: [UiRingSlot; FRAMES_IN_FLIGHT],`"; `render/gpu_column.rs:174` "`ui: Option<UiRenderResources>,`".
- There is one CPU setup Vec: `render/resources.rs:464`.

**E. Load and reload phase**

- The `.ui` source string: `plugin.rs:93` "`let src = std::fs::read_to_string(path).unwrap_or_default();`".
- The parsed tree, the report and the despawn plan.
- `UiTreeView`, a copy of 14 components per node: `reload/tree_view.rs:81` "`pub nodes: Vec<LiveNode>,`"; `:40` "`pub children: Vec<Entity>,`".
- Two `Arc<Mutex<..>>` result sinks: `reload/system.rs:167`.

**F. Statics**

- SPIR-V blobs (`render/mod.rs:137`, `:171`), which are compile-time.
- The kernel's bind accessor table: `J:crates/boyko_ecs/src/ecs/core/component/component_registry/serialize.rs:277` "`static BIND_ACCESSORS: [OnceLock<BindAccessor>; MAX_COMPONENTS] =`".

### 4. Logic inventory

**Exclusive systems (13)**

| System | Location | Registered by |
|---|---|---|
| `ui_layout_apply` | `layout.rs:156` | host |
| `ui_focus_system` | `focus.rs:145` | `UiInteractionPlugin`, `GameplaySet` (`interaction/plugin.rs:116` "`let focus = b.add_system(ui_focus_system).in_set(GameplaySet).key();`") |
| `ui_dispatch_system`, `ui_refreeze_fixed_snapshot` | — | `UiInteractionPlugin` |
| `ui_bind_discovery`, `ui_bind_apply` | — | `UiBindingPlugin` (`UiBindSet`) |
| `ui_bar_discovery`, `ui_bar_apply` | — | `UiWidgetsPlugin` (`UiWidgetSet`, after `UiBindSet`) |
| `ui_tween_reap` | — | `UiAnimationPlugin` (`UiAnimationSet`) |
| `ui_world_project_system`, `ui_world_pick_system`, `ui_world_visibility_system` | — | host |
| `ui_hot_reload_system` | — | `UiPlugin`, Main; runs `world.run_system` closures inside itself (`reload/system.rs:175`) |

**Function systems (7)**

- `ui_layout_discovery` (host), `ui_text_measure_system` (host; `text/measure.rs:24` "P5b stays HOST-DRIVEN").
- `ui_clock_tick` and `ui_visual_tick` (`UiAnimationSet`).
- `ui_sprite_flipbook` (host; `sprite.rs:38` "Register [`ui_sprite_flipbook`] `.before(ui_render_discovery)`").
- `ui_render_discovery` (host).
- `profiling_overlay_system` (Main).

**Outside the scheduler**

- **`UiUploadSystem`** is a manual `impl System`, GpuCompute and dispatcher-solo. It is driven by a host loop:
  - `render/upload.rs:66` "`let token = renderer.wait_frame_in_flight()?;`" and `:70` "`ecs.run_system_once(&mut sys);`";
  - an in-schedule run returns before upload: `:782` "proof: the swapchain `Renderer` is not yet an ECS resource, so".
  - Nothing in production constructs it; the only matches outside tests are docs and exports.
- **Glyph emission** has no production caller.
- **Startup closures** in `UiPlugin` (`plugin.rs:105`) and the **component hooks** (`ui_visual_sink_on_add`, `ui_sprite_anim_on_add`) run through kernel mechanisms and are ECS-native.

**Per-frame, not change-gated:**
- focus: a whole-tree DFS every frame, starting at `focus.rs:159` "`collect_candidates(world, &mut scratch);`";
- the visual tick over every `UiVisual` row, including rested ones (`animation.rs:714`);
- flipbook, world project and world pick;
- the discovery scans.

Everything else runs only on change frames.

### 5. Glued on the side

- **G1. The upload system.** A system-owned staging copy of the pack inputs (A1–A6 → `node_buf` → `keys` → `staging` → ring), driven by a host loop outside the schedule (§4).
- **G2. Five copies of one query result:**
  - `LayoutScratch.roots` (`resources.rs:254`);
  - `UiInteractionScratch.query_buf`;
  - `UiGatherScratch.roots` (`render/gather.rs:284`);
  - `UiWorldScratch.roots` (`pick.rs:110`);
  - `UiHotReload.doc_roots`.

  Layout refills its copy with the allocating verb: `layout.rs:215` "`let fresh = world.query_entities(&[UiRoot::component_id()]);`".
- **G3. Two copies of one traversal.** The candidate snapshot (focus, every frame) and the render gather (on change) are two code copies of one DFS. U's own doc claims otherwise: `render/gather.rs:21` "the hit-test's `paint_seq` are ONE traversal discipline rather than two".
- **G4. `UiTreeView`.** A per-reload copy of a hand-listed component set. Omissions drop silently: `reload/tree_view.rs:60` "but here is dead code that silently drops on every round trip and goes".
- **G5. Private asset tables.** `FontTable`, `UiSheetTable` and raw `UiImage.texture` slots, beside kernel `Assets<T>` (`J:crates/boyko_ecs/src/ecs/core/asset/assets.rs:200` "`pub struct Assets<T: AssetBacking> {`") and `Handle<T>` (`J:.../asset/handle.rs:44`).
- **G6. Hand-rolled change detection:**
  - `UiBindScratch.last_run` and `UiBarScratch.last_run` (both paired with a dirty flag);
  - `UiViewport.generation`;
  - `UiWorldHoverState.prev`;
  - `LayoutScratch.last_viewport_generation`;
  - the global `UiRenderGeneration`.
- **G7. Hand-rolled events:** `click_fired`, `pending_submit`, `hover_entered`, `UiTweenScratch.done`.
- **G8. The re-freeze.** A second freeze of the input snapshot, because the UI writes the input's resource from outside the input pipeline: `dispatch.rs:128` "`world.resource_mut::<ActionState<A>>().freeze_fixed_snapshot();`".

### 6. Kernel features needed

Each exists to remove a workaround listed above. I did **not** read the parallel physics study. The names are generic so they can be shared. Where the need matches one the physics physics-scene-math ledger group already named (`RaggedColumn`), I reuse that name.

- **KF-A `exclusive-change-window`.**
  - The problem: an exclusive body cannot see its own change window, and the direct query rejects change detection: `J:crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:818-819` "Change-detection / requires `Schedule` context".
  - The ticks already exist in the system's metadata: `J:.../system/exclusive_function_system.rs:235` "`self.meta.last_run = last_run;`". But the body only gets the world: `:210` "`(self.func)(world_ref);`".
  - This caused every discovery/apply pair and G6. Physics exclusive stages would hit the same thing.
- **KF-B `entity-query-item`** (a generation-carrying `Entity` as a query item, or an entity-lookup system parameter).
  - The problem: queries yield only a generation-free id (`J:.../iters/query/query.rs:357` "yielding `(EntityId, D::Item<'_>)`"), and `get_entity` exists only on `EcsMaster` (`J:.../ecs_master/entity_query_api.rs:26`).
  - Six UI docs give "no entity-yielding query" as the reason for being exclusive, for example `U:.../world/project.rs:201` "entity-yielding `QueryData`, so the cull-tag flip — which needs the ROOT".
- **KF-C `enable-write-in-query`** (set or clear a bitset/enable tag per row, with declared access).
  - Today it is only possible through `&mut EcsMaster` (`J:.../enable_tag_api.rs:88` "`pub fn enable<T: Component>(&mut self, entity: Entity) {`") or deferred Commands (`J:.../params/entity_commands.rs:220`).
  - This is part of why project, visibility, pick and focus are exclusive (`U:.../world/visibility.rs:56` "An EXCLUSIVE system (`&mut EcsMaster`): the EnableTag mutators are").
- **KF-D `removal-detection`.** There is no `Removed<C>` on either tree: `U:docs/UI-PLAN-INTERACTION.md:90` "**`Removed<C>` does not exist**", and a grep for `pub struct Removed` on J finds nothing. This causes §7 B-1. Physics body removal needs the same.
- **KF-E `dense-change-tick-api`** (the Gaia GK-2 item).
  - `J:.../ecs_master/component_api.rs:406` "`let Some(pool) = archetype.component_pools().get_pool(id) else {`" makes `any_changed_since` blind to dense storage, and J marks this "RED BY DESIGN; not fixed here." (`J:crates/boyko_ecs/tests/gk2_change_tick_api_dense_blindness.rs:2`).
  - Consequence: a HUD label bound to a dense component (physics solver state is dense) **never updates**. This is shared with physics and must use one name.
- **KF-F `resource-change-ticks`.** Resources have no change detection (`U:.../resources.rs:25`), which forced G6's counters.
- **KF-G `exclusive-structural-verbs`.**
  - An exclusive system has no immediate remove: `J:.../component_api.rs:130` "`pub(crate) fn dense_remove_and_fire(`".
  - So `ui_tween_reap` calls the raw dense-store remove, which skips hooks and "takes a bare id with NO liveness check" (`U:.../animation.rs:448`).
  - Its safety rests on "**This kernel does not recycle [`EntityId`]**" (`:454`).
- **Ergonomic, not blocking.**
  - Dense storage still suppresses the single-component Bundle (`J:crates/boyko_macros/src/component.rs:334` "`let bundle_items = if hooks.no_bundle || hooks.storage_bitset || hooks.storage_dense {`"), which forces the wrapper bundles.
- **Memory library** (from the allocator design, not ECS forms):
  - `FrameArena` with `mark()/rewind()` (`M:docs/memory/ALLOCATOR-DESIGN-SPACE.md:91` "`FrameArena` + `FrameVec<T>`, `FrameSlice<T>` … `mark()/rewind(mark)` LIFO"). This is the same need as the ledger's proposed `ScratchStack`, so I recommend not creating `ScratchStack`.
  - `TableSet` (`:94`) for build-once per-font tables.
  - `HeapString`/`LogRing` (`:93`) for diagnostics.
  - `ScratchColumn` already exists, but needs a registered id per column (`J:.../scratch/scratch_column.rs:54` "`component_id`'s layout MUST already be registered in the").

### 7. Defects found along the way

- **B-1 (important, derived, not reproduced): a despawned or de-styled node keeps drawing.**
  - The render discovery filter is `Changed<..>` terms only (`U:render/gather.rs:129-135`) and bumps only on change (`:538` "`if changed.iter().next().is_some() {`").
  - The upload skip then re-serves the previous count (`U:render/upload.rs:742` "`if generation == self.last_seen_generation[slot] {`"; `:746` "`instance_count: self.last_counts[slot],`").
  - So a despawn, or a `remove::<UiBackground>()`, that moves no other rect produces no repack. Examples: a root, an absolute or overlay child, a trailing flow child.
  - No test covers despawn or removal: a grep over `U:crates/boyko_render/tests/ui_*.rs` returns nothing.
  - Fix direction: KF-D.
- **B-2 (scope): no production text path.** U's docs claim text "RIDES the P5a instanced-quad path" (`U:.../lib.rs:68`), but production never sets `text_uv`: it is `None` at `U:render/gather.rs:389` and the only `emit_glyphs` callers are tests.
- **B-3 (informational): stale premises.**
  - "No entity-yielding query" is contradicted by U's own `U:.../animation.rs:713` "`q.iter_entities_mut()`". The real gap is KF-B.
  - `U:.../profiling_overlay.rs:39` says "`Option<Res<R>>` is not a `SystemParam`". That is false on J (`J:.../params/res.rs:160`).
  - The planned host rung wires the deleted function: `U:docs/UI-ADVANCED-ARCHITECTURE.md:570` "gather wired to `host_upload_frame_from_world`".
- **B-4 (latent): font atlas mismatch.**
  - `FontTable::load` accepts N fonts, but the GPU binds one atlas (`U:.../text/font.rs:133` "P5b binds a SINGLE resident font/atlas on the GPU").
  - `GlyphInstance` carries no atlas index (`U:.../text/emit.rs:27-38`), so FontId ≥ 1 would sample font 0's atlas.
- **B-5 (informational): three consumers, two sibling orders.**
  - Layout sorts children by id: `U:.../layout.rs:549` "`child_buf.sort_unstable_by_key(|e| e.id().0);`".
  - Gather and focus use the order of the `Children` slice (`U:render/gather.rs:447`; `U:.../focus.rs:251`), and that order changes on `swap_remove`.
  - Result: paint order among overlapping siblings can flip when an unrelated sibling is removed.
- **Also recorded by the interaction plan (not re-verified by me):**
  - The blur/leave guard has no Windows producer (`U:docs/UI-PLAN-INTERACTION.md:103`).
  - `Interaction` and `Focusable` are not in the `.ui` vocabulary (`:128`).

### 8. U versus J: what merging changes

J already fixes four U-era blockers. U's code still encodes the old constraints.

- **Dense terms inside `Or<..>`.**
  - Fixed on J: `J:crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:1885` "`const HAS_DENSE: bool = false $( || $F::HAS_DENSE )*;`", in the `Or` impl.
  - This removes the reason for `U:.../components.rs:1119` ("UiVisual MUST be a table component (AD10): a dense Changed<C> inside Or<..> was MEASURED") and for `U:render/gather.rs:147` "`crate::ui_pack_inputs!(assert_table);`". That guard now forbids something that works (not verified by running).
- **`#[require]` of a dense component.** Fixed on J: `J:crates/boyko_ecs/tests/ke11_require_poolless_storage_kind.rs:8` "* **DENSE** — `(b) build it`". The cursor could become `#[require]`d (`U:.../components.rs:737-738`).
- **Optional resource parameters.** `Option<Res<R>>` exists on J (`J:.../params/res.rs:160`).
- **Ad-hoc systems return a value.** `run_system` returns `Out` (`J:.../system_api.rs:111` "`pub fn run_system<F, M, Out>(&mut self, system: F) -> Out`"), which makes the `Arc<Mutex>` sinks unnecessary (not compiled).
- **The other direction:** J has `b49c45af` (the `.ui` vocabulary fix across four hand-written lists) and U does not.

### 9. Ledger cross-check

`scratchpad/ledger/ui-input.verified.json` was taken on tree J at `ca582e72`, one commit behind J's head, with no UI change since. It has 122 boyko_ui rows.

- **Missing U-only rows:** `animation.rs:473`, `sprite.rs:257`, `render/upload.rs:198/203/206/669`, `render/gather.rs:284/286/289`.
- **Stale rows:** its render rows name `UiHost::host_upload_frame_from_world`, which U deleted.
- **Duplicate naming:** it proposes `CsrColumn<T>` for the same need `physics-scene-math.verified.json` calls `RaggedColumn<T: Copy>`. Pick one.

### 10. Final table

Earlier-form refutations for all the scratch rows, stated once: component, dense-component, enable-state and relation fail because no entity owns the datum. Event fails because only the producing system reads it. Resource-column fails because the datum does not outlive the frame or phase.

| # | Datum or loop | Today | Natural ECS form (why not an earlier one) | Kernel feature | Cost on the hot path it touches |
|---|---|---|---|---|---|
| 1 | A1/A2/A3/A5 layout inputs, `ComputedRect`, style, markers | component | component, unchanged | — | none |
| 2 | A4 font / texture / sheet fields | raw index into private tables | **relation** entity→asset through `Handle<T>` carriers (component: the field is an edge, not a datum about the node) | none (`Assets<T>` exists) | `UiText` 12→16 B (`U:.../text/components.rs:58` asserts 12); `UiSpriteSheet` 4→12 B. One extra generation compare per resolve, only on change frames |
| 3 | B5 `FontTable` / `FontEntry` | Resource Vec + 4 Box | **resource-column**: `Assets<Font>` + build-once tables (a font is neither an entity nor an edge nor a message) | memory library: `TableSet`; `RaggedColumn` if fonts stream | per-glyph lookup identical |
| 4 | B4 `UiSheetTable` | Resource Vec | **resource-column**: `Assets<Sheet>` | none | one generation check per imaged node per change frame |
| 5 | `UiSourceOrder` / sibling order | reload-only component | **component**: the one sibling-order key, stamped by `ui!` as well | none | layout already sorts per node (`layout.rs:549`) |
| 6 | A6 tween channels, `UiSpriteCursor` | dense | **dense-component** (a component would migrate the archetype per animation, `components.rs:1247-1250`) | none | — |
| 7 | `UiSpriteSheet.index` per-frame write | table, forced by the pre-KE1 kernel | component remains valid; after the merge, re-decide by measurement | none after merge | writes only on a visible frame change (`set_if_neq`) |
| 8 | A7 `Interaction` + 3 string-named tags + `UiInteractionConfig` | component + dynamic tag ids | component, plus **enable-state** as typed bitset components (like `UiWorldCulled`) | none | same toggle cost; one Resource deleted |
| 9 | A8 bind `source` edge | Entity field | **relation** widget→source with a reverse collection (component: an Entity field cannot be walked backwards) | KF-E for dense sources | today scans all bound widgets per dirty frame (`bind_system.rs:130` "`for &widget in &widgets {`"); becomes changed sources × fan-out. One cold insert per source |
| 10 | A9 `EntityAnchor` | Entity in an enum | **relation** root→scene entity | none | visibility scan of all roots becomes a reverse lookup (cold path) |
| 11 | Paint order + effective clip | recomputed by 2 DFS copies (G3) | **component** (`ComputedPaintOrder`, effective clip) written by the one change-frame walk | none | focus's per-frame DFS (4–6 random probes per node, `focus.rs:221-234`) becomes a linear scan of interactive rows; change frames pay one `set_if_neq` per node |
| 12 | B1 viewport / safe area / clock | POD Resource + generation counter | **resource-column** (singleton, no heap) | KF-F | one tick compare, the same as today's |
| 13 | `UiRenderGeneration` | u64 Resource | **resource-column** | KF-D for removals (B-1) | adds an O(removed) term to discovery |
| 14 | `pending_click` | fixed array | **resource-column** | — | — |
| 15 | `click_fired` / `pending_submit` / `hover_entered` / `done` | one-frame slots and Vecs | **event** (component: not durable; enable-state: carries a payload) | none (`EcsMaster::send_event`, `J:.../event_api.rs:50`); same-frame delivery **not verified** | a handful per frame |
| 16 | `ui_press` + re-freeze (G8) | cross-subsystem resource write + a second freeze | **event** consumed by the input aggregator | none | removes one exclusive system per frame |
| 17 | `UiInputFocus` | `Option<Entity>` | **resource-column** (enable-state alone would need a scan to find the focused node) | — | — |
| 18 | `HoveredWorldEntity` + its prev copy | Resource + copy | **event** `HoverChanged` (enable-state: a toggle stamps no tick, `J:.../enable_tag_api.rs:80` "no structural-generation bump, no hook / observer fire") | none | rare |
| 19 | B2 depth pools | 3 × 128 Vec | **system-scratch** on `FrameArena` mark/rewind; bounds are known at level entry (`layout.rs:544-551`) | memory library | one contiguous region; same algorithm |
| 20 | B2 flat arenas | Resource Vec | **system-scratch** on `ScratchColumn` (three interleaved growing arrays rule out a bump arena) | id per column (open) | identical access |
| 21 | G2 root caches × 5 | Vec copies | none: a **query** over `With<UiRoot>` / a `UiDocument` marker | KF-B | removes one copy per system per frame |
| 22 | B9 candidates / `write_nodes` / focusables | per-frame Vecs | **system-scratch** (`FrameArena`) for the residual tab sort; the rest goes with row 11 | KF-B, KF-C | see row 11 |
| 23 | B7 bind/bar lists, ticks, dirty flags | Vec + private tick windows | none: change queries | KF-A | removes the pre-scan; per-row work unchanged |
| 24 | B10 world scratch | Vec | `bounds`: **system-scratch**; roots/pickables: a query | KF-B, KF-C | — |
| 25 | B6 glyph quads | Resource Vec, unfed | **system-scratch** inside the render pack | none | at most 247 B of text per node (`binding/components.rs:79`), change frames only |
| 26 | C upload staging / `node_buf` / `keys` | system Box + Vecs | **system-scratch**: `ScratchColumn` for staging (grows in place, which removes the cap and truncation), `FrameArena` for the rest | none (host: make the swapchain renderer and frame token resources) | same memcpy into the ring |
| 27 | B12 legacy render scratch | no caller | delete (owner scope question already filed) | — | — |
| 28 | D GPU objects and ring | on `RhiContext` | **out-of-scope: driver-owned**; the setup Vec becomes an array | — | — |
| 29 | E source / parse tree / report / plan | String / Vec / HashSet | **system-scratch** on a load arena; diagnostics on `HeapString`/`LogRing` | memory library (open) | load time only |
| 30 | E `UiTreeView` | per-reload copy | none: reconcile over a query + Commands | KF-B | load time only |
| 31 | E `Arc<Mutex>` sinks | heap cells | none: use `run_system`'s return value | none | — |
| 32 | B11 document roots | spill Vec | **component** marker `UiDocument` on each root | none | — |
| 33 | F SPIR-V blobs / tag names / bind accessor table | static | **out-of-scope: compile-time** / **kernel-internal** | — | — |
| 34 | `Children` storage | kernel `Vec<Entity>` | **kernel-internal** (storage study) | — | — |
| L1 | Layout discovery + apply | Function + exclusive, host-registered, relays all roots | exclusive stays (nested parent↔child writes) but dirties per root via an up-walk | KF-A, KF-B | relayout work proportional to dirty roots instead of all roots |
| L2 | Text measure / flipbook / clock / visual tick / overlay | function systems (the flipbook is host-registered) | unchanged; register them in a plugin | — | — |
| L3 | Glyph emission | tests only | ECS system inside the render gather | — | change frames only |
| L4 | Render discovery + upload | host loop outside the schedule | both on the schedule | KF-D; renderer as a resource | none per frame |
| L5 | Focus / dispatch / re-freeze | exclusive, every frame | function systems over the row-11 components | KF-B, KF-C | per-frame DFS becomes a scan |
| L6 | Bind / bar pairs | exclusive pairs | function systems | KF-A | — |
| L7 | Tween reap | exclusive, raw dense remove | Commands, or keep it exclusive | KF-G | only as many as completions per frame |
| L8 | World project / pick / visibility | exclusive, host-registered | function systems | KF-B, KF-C | — |
| L9 | Hot reload | exclusive + closures | exclusive, without the copy and the mutexes | KF-B | load time only |
| L10 | Component hooks | kernel hooks | unchanged; the cursor hook can become `#[require]` after KE11 | none | — |

No cost in this table was measured. Every direction is argued from access pattern and allocation count.

## Open list (lens ui)

1. The parallel PHYSICS study was not read: KF-A..KF-G are proposed as generic names and must be reconciled with its names, especially KF-E dense-change-tick-api (Gaia GK-2), which binds a HUD to dense physics state, and KF-D removal-detection.
2. B-1 (despawned or de-styled UI node keeps drawing: render discovery sees only Changed<..> and the per-slot generation skip re-serves last_counts) is derived from reading U:crates/boyko_render/src/ui/gather.rs:129-135,538 and upload.rs:742-746; it was not reproduced (no cargo), and no test covers despawn/remove.
3. Owner SCOPE call: nothing hosts boyko_ui (boyko_app names no boyko_ui per M playground.rs:53). Layout, text measure, world UI, flipbook, render discovery and UiUploadSystem have no plugin and run only in tests, so every per-frame cost argument here is about code that no shipping binary runs.
4. Converting one-frame UI messages (click_fired, pending_submit, hover_entered, tween completions, ui_press) to kernel events needs a check that an event sent by an exclusive system is readable by a later system in the SAME frame under the EventUpdatePolicy; not verified.
5. ScratchColumn requires a registered ComponentId per column (J scratch_column.rs:54); decide whether UI scratch mints ids or waits for the id-free public column the physics-scene-math ledger proposed.
6. Memory-library naming: the ledger's ScratchStack is the allocator design's FrameArena mark/rewind (M ALLOCATOR-DESIGN-SPACE.md:91); the ledger's CsrColumn equals physics-scene-math's RaggedColumn; the ledger's LoadArena has no counterpart in the allocator design's classes. Pick one name per need.
7. After merging U into J: re-decide the table-vs-dense choices forced by the pre-KE1 kernel (UiSpriteSheet.index as the repaint write, the UiVisual storage const-assert, gather.rs:147 assert_table, which now forbids something J's filter.rs:1885 fixes), and replace the ui_sprite_anim_on_add hook with #[require] per KE11. Neither was run.
8. ui_tween_reap removes dense rows by generation-free EntityId with no liveness check and no hooks (U animation.rs:448); its safety depends on the kernel not recycling EntityId (U animation.rs:454). It must move to a hook-firing, liveness-checked verb (KF-G) before recycling lands.
9. Per-node cached GPU records (a ragged dense UiRecords for incremental repack) vs today's whole-UI repack on any change: a design fork that needs a measurement, not argued further here.
10. Ledger gaps to patch (ui-input group at J ca582e72): no rows for U-only animation.rs:473, sprite.rs:257, render/upload.rs:198/203/206/669, render/gather.rs:284/286/289; its render UI rows describe UiHost::host_upload_frame_from_world, which U deleted.

---

# Lens 2 of 6: render

# Render lens: ECS data forms and where render logic runs (boyko_render and its host, tree J)

## 0. Provenance and scope

- **Trees read.** J = `D:/wt/joltab` @ `d11962a9` (render, RHI, app, scene, kernel). U = `D:/wt/ui` @ `615cda8f`, read only for the render-side UI seam. M = `D:/claude/BoykoEngine`, for the design docs. I ran no cargo and no benchmarks, and changed no git state.
- **The ledger is current for this lens.** `render.ecsform.json` in the scratchpad ledger was taken at J `ca582e72`. I checked two things:
  - `git merge-base --is-ancestor ca582e72 d11962a9` → `ANCESTOR`.
  - `git diff --stat ca582e72 d11962a9 -- crates/boyko_render crates/boyko_rhi crates/boyko_rhi_vulkan crates/boyko_app crates/boyko_scene` → 0 lines.

  So its 169 heap-container rows (form counts: system-scratch 40, resource-column 30, kernel-internal 83, enable-state 8, …) describe J as it is now.
- **What this report adds to the ledger.** I adopt the ledger's forms for those rows and do not repeat them. This report adds three things the ledger does not cover:
  - the non-heap data;
  - the paths by which data reaches the GPU;
  - where the logic runs.
- **Ledger UI rows are stale.** The ledger's `ui-input.json` was also taken on J, so it is 16 commits behind U.
- **One M doc is not committed.** `git status` on M shows `?? docs/memory/`, so the `ALLOCATOR-*` design I cite below is untracked.

## 1. How ECS data reaches the GPU today — 12 paths, one unused

1. **Per-entity model affine.**
   - `GlobalTransform` is packed into the component `InstanceModelCol` by a Main system: J:crates/boyko_render/src/instance_model.rs:111 `mut q: Query<(&GlobalTransform, &mut InstanceModelCol), Enabled<RenderEnabled>>,`
   - It is then bucketed into a Resource-owned ScratchColumn ring: J:crates/boyko_render/src/mesh_draw.rs:289 `pub ring: ScratchColumn<InstanceModelCol>,`
   - The runner then memcpys it, **outside the scheduler**, into a host-visible per-frame-in-flight (FIF) buffer: J:crates/boyko_app/src/runner.rs:1673 `upload_instance_models(&token, &host.gpu.instance_rings[s], scratch);` via J:crates/boyko_render/src/upload.rs:212 `let bytes: &[u8] = bytemuck::cast_slice(scratch.ring.as_read_slice());`
   - Net: two CPU copies per instance per frame after the pack.
2. **Interpolated bodies.** A dense component is packed in the Fixed schedule (J:crates/boyko_render/src/gpu_transform3d.rs:84 `#[component(storage = "dense")]`; J:crates/boyko_app/src/plugins.rs:781 `add_gpu_transform_pack(b).in_set(FixedSet::Snapshot);`). It then goes to `pair_ring`/`pair_out_slot`, the runner uploads (`upload_pair_ring`), and a GPU interp compute writes into the shared ring.
3. **Per-instance material lanes** (`material_ids`, `material_tex`) come from the same gather. The runner uploads them only when a flag gates them in (runner.rs:1693 `if scratch.any_non_default_material() {`).
4. **VB instance rows.** A system rebuilt and run by the runner every frame (§2.2 step 7) produces them, and the runner uploads them (`upload_vb_instance_rows`).
5. **Lights.**
   - Light components are folded by `collect_lights` (Main) into a byte ScratchColumn: J:crates/boyko_render/src/light_system.rs:90 `scratch: ScratchColumn<u8>,`
   - The runner uploads per slot behind a host-side generation gate: runner.rs:1786 `let light_upload = if light_upload_due(&mut host.light_uploaded_gen, s, generation)`
   - The recorder then copies staging into the device table.
6. **Materials** go through an explicit GPU mirror: J:crates/boyko_render/src/material_table.rs:67-68 "The device-resident [`MaterialGpu`] SSBO + its per-in-flight staging ring — the GPU / mirror of the world's [`Assets<Material>`] CPU authority". Edits after boot do not reach the GPU: material_table.rs:41 "At this rung (A1) no caller ever mutates a material after boot".
7. **Meshes and textures.** `Assets<MeshGpu>` and `Assets<TextureGpu>` rows own their device buffers, so there is no mirror. They are uploaded **once at boot**: runner.rs:830 "a BOOT ONE-SHOT, not a per-frame system (keeps the frame loop unchanged):", runner.rs:835 `app.world_mut().run_system(upload_material_assets);`
8. **Config Resources and camera.** `ViewUniform`, `ResolvedCsm`, `ResolvedShadowAtlas`, `ResolvedRayShadow`, `ResolvedShadowDenoise`, `ResolvedTemporalShadow`, `ResolvedTaa` and `MotionCam` are memcpy'd by the runner into per-slot uniform (UBO) rings. The CSM and atlas UBOs are written **unconditionally** every frame (runner.rs:1804 "The CSM cascade UBO into slot `s` — UNCONDITIONAL every").
9. **SDF.** `SdfPrimitive` components are gathered once after boot (runner.rs:661 `app.world_mut().run_system(collect_sdf_edits);`) and uploaded once (runner.rs:1442 `upload_sdf_edit_list(&token, host.gpu.edit_list(), staging.edits());`). Post-boot spawns are ignored (runner.rs:1384 "boot-static — so a post-boot `SdfPrimitive` spawn is silently ignored").
10. **Particles.** Emitter and effect scratch lanes are Resources on ScratchColumns (J:crates/boyko_render/src/particle_system.rs:213 `rows: ScratchColumn<EffectParamsGpu>,`), uploaded by the runner. Per-particle state exists only on the device, inside a host-owned bundle: J:crates/boyko_app/src/gpu_scene/particle.rs:227 `pub(crate) struct ParticleGpuBundle {`, :241 `particle: BoundBuffer,`
11. **Phase-5 device-resident columns** — the kernel seam (J:crates/boyko_ecs/src/ecs/memory/component_pool.rs:86 "Device-memory backing (Phase 5 fill)") and `GpuColumnManager::create_column` (J:crates/boyko_render/src/gpu_column.rs:639 `pub fn create_column(`) — are **not used by the production renderer**. `grep "\.create_column("` over `*/src` returns nothing; the only callers are `boyko_render/tests/{drop_teardown,gpu_system,grow_stale,lifecycle,multi_column}.rs`. `boyko_app` only inserts the context (runner.rs:239 `.insert_non_send_resource(RhiContext::from_shared(ctx));`).
12. **`Gpu3dInstance`** is packed every frame by a scheduled system (J:crates/boyko_render/src/render3d_plugin.rs:38 `b.add_system(sync_gpu_3d_instances);`, composed at plugins.rs:431 `app.add_plugin(Render3dPlugin);`). Nothing in any `*/src` consumes it: a grep for `Gpu3dInstance` finds only its own module, its pack, and doc comments in the scene and physics bundles saying it is *not* a bundle field.

**What is already good and should be kept:**
- Per-frame lanes live on ScratchColumn, not `Vec` (mesh_draw.rs:266-267 `#[derive(Resource)]` / `pub struct MeshRenderScratch {`).
- The interpolation pair is the first dense component.
- Capabilities are expressed as component presence (`ShadowCaster`, `OcclusionCulling`, `CastsPunctualShadow`).
- `Assets<MeshGpu>` owns its buffers, with no mirror.
- The particle effect table is indexed by the asset row: particle_system.rs:210 "The baked rows, INDEX-ADDRESSED: `rows[i]` is `Assets<ParticleEffect>` row `i`, so an".

## 2. Where render logic runs

### 2.1 On the scheduler (J)

**Main schedule**, registered at plugins.rs:675-767:
- `sync_instance_model_cols` (plugins.rs:676 `let pack = b.add_system(sync_instance_model_cols).key();`)
- `sync_prev_instance_model_cols` (hwrt only)
- `gather_shadow_casters` (:684), `reduce_caster_bounds`
- `sync_csm_light_gate`, `sync_punctual_light_gate`, `sync_ssao_light_gate`, `sync_sv0_light_gate`, `sync_cluster_light_gate`
- `snap_apply`
- `gather_mesh_draws` (:766 `b.add_system(gather_mesh_draws).after(pack).after(snap);`)

**Also on Main, via plugins:**
- Lighting: `collect_lights`, `light_reconcile`, `select_lighting_cull`, and an exclusive seed closure.
- Policy and resolve systems: CSM, shadow atlas, ray, shadow-denoise, AA/TAA, SSAO.
- Particles: tick, pack, refcount.
- Asset refcount: `apply_refcount_deltas`, `validate_asset_refs`.
- `sync_gpu_3d_instances` — no production consumer (§1 path 12).

**Fixed schedule:** `pack_gpu_transforms`.

**Only two core schedules exist:** J:crates/boyko_ecs/src/ecs/core/app/app.rs:67 `Main,` and :71 `Fixed,`

### 2.2 Outside the scheduler — the host frame loop

`frame_loop` in J:crates/boyko_app/src/runner.rs:1012-3283 holds the whole per-frame render path after `app.update_with_delta`. Inside that range there are 22 upload call sites (21 distinct fns) and 57 `resource::<`/`try_resource::<` World reads. Steps:

1. Epoch publish, then the ECS frame: runner.rs:1232 `*app.world_mut().resource_mut::<RenderEpoch>() = RenderEpoch(host.renderer.submission_epoch());`, :1233 `app.update_with_delta(dt);`
2. Fence wait and token: :1249 `let token = match host.renderer.wait_frame_in_flight() {`
3. Fence-gated frees: :1264 `retire_deferred_frees(`, which takes a host-owned `Vec`: J:crates/boyko_render/src/asset_refcount.rs:556 `scratch: &mut Vec<FreeEntry>,`
4. Material/instance grow, done by taking NonSend resources out of the World and reinserting them. The code says why: :1289 "cannot split a live `&World` borrow across Send/NonSend storage, so". It also names the price: :1275 "`NonSendResources::insert`/`remove` heap-(de)allocate the". The take-out itself is at :1298 `.remove_non_send_resource::<RetiredGpuBuffers>()`.
5. Descriptor repoint (:1341-1370) and the one-shot SDF upload (:1418-1446).
6. **Resource logic run as host code:** :1540 `taa_state.mark_reset();`, :1542 `taa_reset_flag = taa_state.advance();`, :1552 `advance_jitter(jitter, taa_armed_now);`, :1604 `Some(app.world_mut().resource_mut::<boyko_render::MotionCamState>().advance(cur))`
7. **A system rebuilt every frame:** :1619 `app.world_mut().run_system(boyko_render::sync_vb_instance_ring_system);` `run_system` builds a fresh system on each call (J:crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs:116 `let mut sys = F::into_system(system);`), and its `initialize` is idempotent only once state exists (J:crates/boyko_ecs/src/ecs/core/system/function_system.rs:188 `if self.state.is_some() {`). So a VB boot pays build + first-initialize every frame. Whether that allocates is not verified.
8. **The draw list is a derived side table built from the gather:**
   - :1623 `let mut draws = host.draw_scratch.take();`
   - :1978 `let Some(mesh) = mesh_assets.try_get(MeshHandle(b.mesh_id)) else {`
   - :1981 `draws.push(GBufferMeshDraw {`
   - `casts_shadow` is merged against the caster scratch at :1959 `let casters = world.resource::<CsmCasterScratch>();`
   - The world AABB is folded from :1963 `let instance_ring = scratch.ring.as_read_slice();`
9. The ~20 uploads (:1655-1940), the arming predicates (:2010, :2034), the push matrices (:2046-2112), the particle inputs (:2137-2223) and the config reads (:2243-2503).
10. Scene assembly (:2514 `let scene = host.gpu.scene(`), then record + present (:2640 `host.renderer.render_gbuffer_frame(`).
11. Publish: :3268 `*app.world_mut().resource_mut::<WindowInfo>() = WindowInfo {`, stats at :3273-3279, and the frame counter, which is a stack local (:1068 `let mut frame_index: u32 = 0;`, :3281 `frame_index = frame_index.wrapping_add(1);`).

**Boot one-shots outside any schedule:**
- `collect_sdf_edits` (:661)
- `upload_*_assets` (:835-837)
- `backfill_vb_geometry_slots` (:846)
- `MaterialTable::boot_seed`, done by taking a Resource out and reinserting it (:858-870)

They are outside startup because startup is an unordered push-order list (app.rs:175 `startup: Vec<StartupSystem>,`). SdfPlugin's own doc explains the consequence ("Startup systems drain in PUSH order in `App::finish`").

### 2.3 Precedents for moving this onto the scheduler

- **The kernel already has the pieces:**
  - dispatcher-only access to NonSend resources (J:crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs:22 "the token is minted ONLY by the scheduler on the dispatcher-solo");
  - run conditions (J:crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:831 `pub fn run_if<C, M>(self, condition: C) -> Self`);
  - scheduled GPU compute (`GpuSystem`, J:crates/boyko_render/src/lib.rs:35-36 "The system resolves its target / column indirectly by `(archetype, component)` (MF-7)").
- **The UI upload on U is already a scheduled GPU-compute system,** and it names the gap in its own doc:
  - U:crates/boyko_render/src/ui/upload.rs:177 "The UI upload system (Rung 4 + UI-ADVANCED S0): a `GpuSystem`-shaped"
  - :61 "# Driving the seam from a host (until the `Renderer` is an ECS resource)"
- **The frame token is plain data:** J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:1043-1044 `pub struct FrameWriteToken {` / `slot: usize,`

## 3. Structures glued on the side

Each of these is subsystem-owned, lives outside the World or duplicates World data, and is fed by copies.

- **G1 — the whole render device state lives outside the World.**
  - J:crates/boyko_app/src/host.rs:3 "[`WindowHost`] owns everything the OS/present side needs OUTSIDE the World:"
  - host.rs:83 `pub(crate) renderer: Renderer<'static>,`, :87 `pub(crate) frame: GBufferFrame,`, :90 `pub(crate) gpu: GpuSceneBundles,`
  - J:crates/boyko_app/src/gpu_scene/mod.rs:924 `pub(crate) struct GpuSceneBundles {` holds about 100 fields: pipelines, layouts, FIF rings (:931 `pub(crate) instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT],`, :1119 `pub(crate) light_staging: [BoundBuffer; FRAMES_IN_FLIGHT],`), per-slot capacity (:974 `instance_capacity: [u32; FRAMES_IN_FLIGHT],`), the particle bundle (:950), the brick clipmap (:1038) and DDGI.
- **G2 — the draw list.**
  - host.rs:92 `pub(crate) draw_scratch: DrawListScratch,`
  - gpu_scene/mod.rs:8111 `buf: Vec<GBufferMeshDraw<'static>>,`, re-lifetimed by :8132 `core::mem::transmute::<Vec<GBufferMeshDraw<'static>>, Vec<GBufferMeshDraw<'a>>>(v)`
  - It is a per-frame copy of `MeshRenderScratch.batches` plus mesh buffers, caster flags and AABBs, built outside the scheduler.
- **G3 — per-slot upload generations kept on the host, in three copies of one protocol.**
  - host.rs:138 `pub(crate) light_uploaded_gen: [u64; FRAMES_IN_FLIGHT],`
  - host.rs:147 `pub(crate) particle_effects_uploaded_gen: [u64; FRAMES_IN_FLIGHT],`
  - Two twin functions: J:crates/boyko_app/src/light_gate.rs:29 `pub fn light_upload_due<const N: usize>(` and particle_gate.rs:46 `pub fn particle_effects_upload_due<const N: usize>(`
  - A third variant inside `MaterialTable`: material_table.rs:97 `rebind_pending: [bool; FRAMES_IN_FLIGHT],`
- **G4 — two copies of one datum:** the render path is both a host field and a World Resource. runner.rs:559 `host.resolved_render_path = resolved_render_path;`, runner.rs:568 `app.world_mut().insert_resource(resolved_render_path);`
- **G5 — `MaterialTable`,** the explicit GPU mirror of `Assets<Material>` (§1 path 6).
- **G6 — the light seed state is captured in a closure, not stored in the World.**
  - J:crates/boyko_render/src/light_system.rs:698 "closure-captured cross-frame data, not an ECS `Resource` — but `ScratchColumn`"
  - Its eight cached systems return fresh `Vec`s (:729 `q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()`). Registration: J:crates/boyko_render/src/light_plugin.rs:122 `let mut seed_state = light_seed_state();`
- **G7 — host-held scratch:** `retire_scratch` (host.rs:96 `pub(crate) retire_scratch: Vec<FreeEntry>,`). The ledger row agrees.
- **G8 — three stores for the SDF scene, and they diverge.**
  - Render gathers `SdfPrimitive` into a staging resource once (J:crates/boyko_render/src/sdf_edit.rs:62 `pub struct SdfEditStaging {`).
  - Physics keeps its own Resource that the caller fills separately: J:crates/boyko_physics/src/sdf_query.rs:46 `pub struct SdfField(SdfEditField);` and plugin.rs:324 "the caller fills it with the same edit list the GPU".
  - The brick clipmap is baked from an **empty** field: gpu_scene/mod.rs:1036 "The brick clip-map baked from the EMPTY edit field — the valid"; mod.rs:1806 `let clipmap = BrickClipmap::create(ctx, &field, [0.0, 0.0, 0.0])`
- **G9 — the frame graph arenas are `std::Vec`s, chosen so the RHI crate would not need the kernel.**
  - J:crates/boyko_rhi_vulkan/src/framegraph/graph.rs:12-13 "This sidesteps making / `boyko_ecs`'s `pub(crate)` `VmReservation` public"
  - The arenas: graph.rs:126 `res_is_image: Vec<bool>,` … :205. Owner: frame_driver.rs:197 `let mut frame_graph = crate::framegraph::FrameGraph::with_capacity(48, 32, 192);`
  - The RHI crate has no kernel dependency: J:crates/boyko_rhi_vulkan/Cargo.toml:54-55 `[dependencies]` / `boyko_rhi = { path = "../boyko_rhi" }`, and its dependency list (:54-73) has no boyko_ecs.
- **G10 — the retire/orphan queues:** `OrphanedMeshGpu`, `OrphanedTextureGpu` and `RetiredGpuBuffers`.
  - J:crates/boyko_render/src/mesh_assets.rs:712 `orphans: Vec<(MeshGpu, u64)>,`
  - texture.rs:928 `orphans: Vec<(TextureGpu, u64)>,`
  - retired_gpu_buffers.rs:53 `entries: Vec<RetiredBuffer>,`

  Forms as in the ledger (`KF-assets-adopt-retiring`, `KF-owning-scratch-column`).
- **G11 — second index spaces beside the asset row index.**
  - J:crates/boyko_render/src/bindless.rs:73-74 `free_slots: Vec<u32>,` / `retiring_slots: Vec<(u32, u64)>,`
  - mesh_geometry_table.rs:445 `alloc: BindlessSlotAllocator,`
  - The asset side stores the mapping: mesh.rs:170 `pub geometry_slot: u32,`, texture.rs:541 `pub bindless_slot: u32,`

## 4. Duplications and dead paths — ECS storage, but not one system

- **D-a: two per-entity model columns.** `Gpu3dInstance` (§1 path 12) duplicates `InstanceModelCol` (instance_model.rs:58 `pub struct InstanceModelCol {`) and is consumed by nothing. Render3dPlugin's own doc hands the draw to "the consuming renderer" (render3d_plugin.rs:28-29), and no such consumer exists.
- **D-b: the caster gather duplicates the main gather.**
  - J:crates/boyko_render/src/csm_caster.rs:90 `pub struct CsmCasterScratch(pub MeshRenderScratch);` runs a second count/prefix/scatter (:203 `scratch.0.gather_mixed_into(`) over a subset of the same rows (:190 `(Enabled<RenderEnabled>, With<ShadowCaster>),`).
  - In production only its batch list (runner) and its ring (csm_caster.rs:465 `scratch.ring(),`, in `reduce_caster_bounds`) are read.
  - The main gather already folds a zero-sized-type (ZST) capability per row at no cost: mesh_draw.rs:345-346 "the marker read is one `Option<&ZST>` probe per row, which resolves to a / per-ARCHETYPE constant."
- **D-c: `LightTableDirty` is a hand-made change channel.**
  - J:crates/boyko_render/src/light.rs:267 `pub struct LightTableDirty(pub bool);` with the reason at :258 "A bitset toggle ([`LightEnabled`]) bumps no `Changed` tick, and a removed /"
  - It exists because the kernel has no change detection on enable tags (J:crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:972 "Added/Changed are not supported on bitset enable tags (no tick") and no removal query: grep for `RemovedComponents|Removed<` in boyko_ecs finds nothing.
  - Separately, `LightEnabled` starts disabled (light.rs:244 "A never-toggled row reads DISABLED (the bitset default)."), which is why the seed of G6 exists.
- **D-d: doc rot found while reading.**
  - `collect_sdf_edits` is said to be a startup system (sdf_edit.rs:8 "runs ONCE in the STARTUP schedule"; :111 "registered in the startup schedule by [`SdfPlugin`]"), but the plugin only inserts the staging resource (:152 `app.insert_resource(SdfEditStaging::default());`).
  - `PrevInstanceModelCol` is called dense (instance_model.rs:133 "so it lives in a dense `ComponentPool`"), but its derive has no dense storage (:158 `#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]`), unlike gpu_transform3d.rs:84.
- **D-e: assets loaded after boot never reach the GPU through the host.** The only drain is the boot one-shot (runner.rs:830).

## 5. The M design docs against these forms

- **Transparency** (M:docs/render/TRANSPARENCY-DESIGN-SPACE.md):
  - **Right forms:** markers as ZST components (:71 "**`Translucent`** — a ZST component; presence = …"), and a third gather as a newtype over the mesh scratch (:118 `TranslucentRenderScratch(MeshRenderScratch)   // the CsmCasterScratch newtype shape`).
  - **It adds a second GPU mirror** of `Assets<Material>` (:87 "transparency parameters are a **second SSBO** indexed by the same 16-bit `MaterialId`"). That is the G5 pattern twice, and it inherits the G2/G3/§2.2 host paths.
  - **It adds a second authority, as a check:** :79 "A debug system asserts marker ⇔ route each".
  - The translucent gather partitions rows with `Without<Translucent>` rather than overlapping them, so it is not D-b's kind of duplication.
- **Reflections** (M:docs/render/REFLECTIONS-DESIGN-SPACE.md):
  - **Right forms:** probes as components (:106 `ReflectionProbe {`), environments as a Resource-owned column (:57 "One row per environment in an append-only, resource-owned column (the `ScratchColumn`-class"), probe order sorted inside the table rather than a side array (:148 "array was considered and rejected as a second store for data the table already holds").
  - **Two data structures it introduces are not placed yet:**
    - a parallel `ProbeGpu` SSBO filled at gather (:118-121) — another mirror;
    - a `Captured { every: u16 frames }` source (:112) that needs per-probe cross-frame state and a per-view seam (:986 "the per-view seam (MPR R11) for captured probes and planar"). Today the renderer is single-view (one `ViewUniform` Resource).
- **Allocator** (M:docs/memory/ALLOCATOR-DESIGN-SPACE.md, untracked) sends render persistent lists to allocator primitives: :337 "render/rhi persistent lists (`bindless.rs`, `retired_gpu_buffers.rs`, `gpu_column.rs:532`, FrameGraph SoA lanes) | `HeapVec` / `VmColumn`". That conflicts with the rejected-bespoke-column ruling and with the ledger. Here I follow the ledger: ComponentPool-backed forms.

## 6. Raw pointers or slices held across a grow

- `GBufferMeshDraw` holds `&BoundBuffer` borrowed from `Assets<MeshGpu>` rows for the duration of the render call only. The `'static` park holds no element (gpu_scene/mod.rs:8125 debug-assert empty). Asset rows are ComponentPool-backed and address-stable.
- FIF mapped pointers are re-read from the current buffer every frame (`&host.gpu.instance_rings[s]`). A grow retires the old buffer through `RetiredGpuBuffers` and repoints descriptor sets via `rebind_pending` (runner.rs:1336-1370). Nothing caches a pointer across a grow.
- `LightTableStaging` never grows after construction.
- The frame graph holds `u16` indices only.
- The Phase-5 path re-resolves by `(archetype, component)` on every dispatch (lib.rs:35-36).
- **No violation found.** The one unsafe relabel is the G2 transmute. It is sound as written, and goes away under the form proposed in row 7.

## 7. Kernel features needed

Names are aligned with the ledger so the physics study can match them.

1. **`KF-device-column-residency`** (the existing Phase-5 seam, Stage 5), extended with two arms:
   - **Resource / asset-store arm:** `Assets<Material>.gpu`, mesh geometry meta and bounds, particle pools.
   - **FIF-mirror arm:** a per-slot uploaded change tick replaces G3 and the 22 hand-written upload calls.

   Evidence the seam is dead in production: §1 path 11. Constraint: boyko_ecs stays graphics-pure (dispatcher_token.rs:33 "`DispatcherToken` is generic over [`NonSendResource`] and names NO graphics type"), so the kernel holds only the opaque handle and render mints it.
2. **`KF-enable-initial-polarity`** (ledger) — deletes G6.
3. **`KF-assets-adopt-retiring`** (ledger) — the orphan queues, plus slot = asset row index (G11).
4. **`KF-owning-scratch-column`** (ledger) — `RetiredGpuBuffers`.
5. **`KF-assets-iter-mut`** (ledger) — the upload drains.
6. **`KF-startup-schedule`** (new) — startup systems built through `ScheduleBuilder`, with sets and edges, replacing the push-order `Vec<StartupSystem>`. This lets the boot one-shots be scheduled.
7. **`KF-structural-change-observation`** (candidate) — change ticks for enable-tag toggles and removals. Needed only if an `on_remove` hook cannot write an event lane (not verified).
8. **No kernel feature is needed for moving the render phase onto the scheduler.** NonSend params, `run_if` and GPU-compute systems all exist. An optional `CoreSchedule::Render` is an architect call.

## 8. Table

Abbreviations: C = component, DC = dense-component, ES = enable-state, R = relation, EV = event, RC = resource-column, SS = system-scratch. "✗" = does not fit, with the reason.

| # | datum or loop | today (quotes in §) | natural ECS form (why earlier forms fail) | kernel feature | cost argument for the hot path it touches |
|---|---|---|---|---|---|
| 1 | `InstanceModelCol` | C, Main pack (§1.1) | **C** — keep | none | unchanged |
| 2 | `Gpu3dInstance` + `sync_gpu_3d_instances` | C + scheduled pack, no production consumer (§1.12, D-a) | delete; row 1 is the single per-entity model column (C fits, but this is a duplicate) | none | removes one scheduled system; it matches 0 archetypes in production, so only dispatch cost goes away |
| 3 | `GpuTransform3D` | DC, Fixed (§1.2) | **DC** — keep (C✗: the GPU interp reads one contiguous pair buffer) | none | unchanged |
| 4 | `PrevInstanceModelCol` | table C; doc says dense (D-d) | **C**; fix the doc, or choose DC if the prev ring is ever uploaded from the column | none | unchanged |
| 5 | `MeshRenderScratch` lanes (ring, batches, mesh_ids, pairs, materials, flags, vb_ring) | Resource of ScratchColumns, frame lifetime; writers: gather (Main) + runner `run_system`; readers: runner uploads | **RC** (frame lifetime). C/DC✗: rows are in mesh-bucketed draw order, not per entity. ES✗: not a bool. R✗: no edge. EV✗: several consumers in the frame, and the storage persists across frames | none | keep the CPU ring — the host reads it back (runner.rs:1963); do not fold it into host-visible memory (write-combined reads: not verified) |
| 6 | `CsmCasterScratch` | second full gather (D-b) | a per-instance caster flag lane in row 5 + one bounds fold (C✗: a per-frame product, same as row 5) | none | −1 O(casters) count/prefix/scatter pass and one ring; +1 per-archetype-constant ZST probe per row (mesh_draw.rs:345-346) |
| 7 | `DrawListScratch` `Vec<GBufferMeshDraw<'a>>` | host field, transmute, built outside the scheduler (G2) | **RC**: `casts_shadow` and `world_aabb` lanes on row 5, computed in the gather; the recorder resolves buffers by mesh row index, which makes the element `Copy`. C✗ per batch; ES/R✗; EV✗ it is a table, not a message | none | O(batches), tens to hundreds; one indexed load per batch moves from runner to recorder; the transmute goes away |
| 8 | `LightTableStaging` + `LightTableGeneration` | Resource, `ScratchColumn<u8>` (§1.5) | **RC** — keep. C/DC✗: one packed header+rows table for all lights. ES/R/EV✗ | none | unchanged; rebuilt only on change |
| 9 | per-slot upload generations (light, particle effects, material `seen_gen`/`rebind_pending`) | host fields + NonSend (G3) | **RC** on the render-device Resource, one protocol. C✗ not per entity; ES✗ a per-slot u64; R✗; EV✗ persists across frames | `KF-device-column-residency` (FIF arm) | one compare per mirror per frame, same as today |
| 10 | `LightSeedState` | closure-captured exclusive system + 8 cached systems returning `Vec` (G6) | **ES** — `LightEnabled` starts enabled; the seed is deleted. C✗: the datum is a per-light bool | `KF-enable-initial-polarity` | removes four cached runs and one exclusive serialisation point per frame (light_system.rs:871 "The steady-state pass is NOT free: it runs four CACHED") |
| 11 | `LightTableDirty` | Resource bool as a hand-made change channel (D-c) | **EV** sent by the `on_remove` hook and the enable setter. C/DC✗ not per entity; ES✗ global; R✗ | `KF-structural-change-observation` only if hooks cannot send events (not verified) | one event-lane read per frame instead of one bool read |
| 12 | `MaterialTable` | NonSend GPU mirror of `Assets<Material>` (G5) | **RC**: the `gpu` lane of the `Assets<Material>` store, device-resident. C✗: asset rows are not entities. ES/R/EV✗ | `KF-device-column-residency` (asset arm) | cold: boot seed, rare grow; GPU reads unchanged |
| 13 | `Assets<MeshGpu>` / `Assets<TextureGpu>` | RC (ComponentPool + drop glue) | **RC** — keep | none | unchanged |
| 14 | `OrphanedMeshGpu` / `OrphanedTextureGpu` | NonSend `Vec<(T, u64)>` (G10) | **RC**: Retiring rows of the same `Assets<T>` (ledger) | `KF-assets-adopt-retiring` | cold: fill-reject only |
| 15 | `RetiredGpuBuffers` | NonSend `Vec` (G10) | **RC**: an owning column (ledger) | `KF-owning-scratch-column` | cold: grow only |
| 16 | bindless and geometry slot allocators + `geometry_slot` / `bindless_slot` | second index spaces (G11) | the relation asset→slot is total and 1:1, so it collapses into identity: slot = asset row (the particle effect precedent). This is RC indexing, not a stored R | `KF-assets-adopt-retiring` (fence-gated reuse) | cold; GPU still reads `meta[slot]` |
| 17 | `MeshGeometryTable` meta/bounds buffers | NonSend host-visible mirror | **RC**: device-resident lanes of `Assets<MeshGpu>` | `KF-device-column-residency` (asset arm) | cold: register/backfill |
| 18 | `GpuColumnManager.meta` | `Vec`; the path is test-only in production | kernel-internal (the device arm of the pool backing), Stage 5 | `KF-device-column-residency` (existing seam) | not on the production frame path |
| 19 | `GpuSceneBundles` (pipelines, layouts, FIF rings, TLAS, MV, particle, brick, DDGI) | host struct outside the World (G1) | **RC**: a NonSend singleton Resource. The VkBuffer/VkImage memory itself is out of scope: driver-owned. C/DC✗ no entity; ES/R✗; EV✗ persistent | none (the `RhiContext` NonSend precedent) | deref through the NonSend slab, `#[inline]` (runner.rs:1273) instead of a struct field — about the same |
| 20 | `GBufferFrame` / `Renderer` / `Swapchain` / `Surface` / `Window` | host-owned (G1) | **RC** (NonSend handle records) + an explicit teardown system in host.rs:67-80 order. Window, surface and images: out of scope, OS- or driver-owned | none | same |
| 21 | `FrameGraph` SoA arenas + cached pass plans | `std::Vec` in boyko_rhi_vulkan, rebuilt per frame (G9) | **SS** of the record system. C…RC✗: produced and consumed inside one call | none in the kernel; crate direction is an open item | same arena sizes; holds indices only |
| 22 | `resolved_render_path` host field | two copies (G4) | the World Resource only (C/ES/R/EV✗ singleton) | none | one `Res` deref per frame |
| 23 | `frame_index` | stack local in the frame loop | **RC** (a frame-counter Resource) | none | one load |
| 24 | `TaaState` / `JitterState` / `MotionCamState` writes | Resources written by host code (§2.2 step 6) | keep the Resources; the writers become systems | none | same arithmetic |
| 25 | 22 FIF upload call sites | runner, outside the scheduler | systems in a render set; the target memory is out of scope (driver-owned) | `KF-device-column-residency` (FIF arm), optional | same memcpy bytes; a change-tick gate can skip the unconditional CSM + atlas UBOs (336 + 1296 B/frame) |
| 26 | SDF scene: staging + physics `SdfField` + empty-field clipmap | three stores (G8) | **RC**: one `SdfEditField` derived by one system from `SdfPrimitive` components (C = the authority), read by physics, the marcher upload and the brick bake | none (coordinate with the physics study) | 768 B; the physics AVX2 kernel keeps the same `[SdfEdit; 16]` layout (sdf_query.rs:38-44) |
| 27 | particle pools (particle / render / alive / dead / counters) | device-only buffers in a host bundle | **RC**, device-resident. C✗: particles are not entities; DC✗: no owning entity | `KF-device-column-residency` (Resource arm) | the host never touches rows |
| 28 | `ParticleEmitScratch` / `ParticleEffectScratch` | RC on ScratchColumn | **RC** — keep | none | unchanged |
| 29 | `retire_scratch` `Vec<FreeEntry>` | host field (G7) | **SS** (ledger) | none | same |
| L1 | Main/Fixed render systems (§2.1) | scheduled | keep | none | — |
| L2 | runner per-frame steps 2–11 (§2.2) | host loop outside the scheduler | systems in a render set at the end of Main: dispatcher thread, NonSend params, `run_if` for a minimised window, ordered after the gathers; the fence token held in a NonSend frame Resource (U precedent) | none required; optional `CoreSchedule::Render` | adds per-system dispatch overhead (not measured); removes the grow-frame NonSend box alloc/dealloc (runner.rs:1275) and the per-frame `run_system` rebuild (L3); the loops themselves are unchanged |
| L3 | `sync_vb_instance_ring_system` via `run_system` every frame | rebuilt each frame | scheduled after `gather_mesh_draws`, `run_if(VB)` | none | removes a per-frame system build + initialize |
| L4 | boot one-shots (SDF gather, asset uploads, backfill, `boot_seed`) | `run_system` after `finish` | a startup schedule with sets | `KF-startup-schedule` | boot only |
| L5 | asset upload after boot | missing (D-e) | a per-frame drain system over `AssetStaging` | `KF-assets-iter-mut` | 0 on an idle frame (one empty check) |
| L6 | recording (`render_gbuffer_frame`, `declare_frame_graph` graph_bridge.rs:1243) | called by the runner | the last system of the render set, `NonSendResMut<Renderer>` | none | identical |
| L7 | `WindowInfo` / `HostFrameStats` publish | runner | a post-present system | none | three integer stores |

## 9. Hot-path summary

- **Nothing proposed touches the physics step.**
- **Render hot loops:** the gather is O(N) over two passes, the uploads are O(N) memcpy, and the draw list is O(batches).
  - Rows 6 and 7 cut work.
  - Rows 1–5 and 8 keep their loops byte-identical.
  - L2 moves unchanged code under the scheduler. Its only new cost is system dispatch, which is **unmeasured**. Per the owner's standing order it must be measured with the owner's consent before anyone claims it is neutral.

## Open list (lens render)

1. Render phase placement: a set at the end of Main with run_if + NonSend params (no kernel change) vs a new CoreSchedule::Render — architect call; the dispatch overhead of ~20 extra scheduled systems per frame is unmeasured (J:crates/boyko_ecs/src/ecs/core/app/app.rs:67 `Main,` / :71 `Fixed,` are the only schedules).
2. FrameGraph arenas cannot use ScratchColumn because boyko_rhi_vulkan has no boyko_ecs dependency (J:crates/boyko_rhi_vulkan/Cargo.toml:54-73; graph.rs:12-13 'This sidesteps making `boyko_ecs`'s `pub(crate)` `VmReservation` public'). Options: lend ScratchColumn build views from boyko_render, a storage trait in boyko_rhi, or the boyko_memory crate split in M:docs/memory/ALLOCATOR-DESIGN-SPACE.md §2.0 (which keeps scratch_column.rs in boyko_ecs, so it does not solve this by itself).
3. Not verified: whether an on_remove hook can write an event lane. If not, LightTableDirty needs KF-structural-change-observation (the kernel has no Changed on bitset enable tags — J:crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:972 — and no removal query).
4. Not verified: whether the HostVisibleCoherent instance rings are write-combined. The runner reads the CPU ring back (J:crates/boyko_app/src/runner.rs:1963), which is the reason given for keeping the ScratchColumn staging copy.
5. MeshHandle as a fragmenting relation (archetype per mesh, which would make the count/prefix/scatter unnecessary and the column device-backable) vs a plain component: not measured; fragmentation would split every co-located column (GlobalTransform, physics) per mesh.
6. Bindless/geometry slot = asset row index needs a policy for descriptor-array capacity vs the unbounded Assets<T> high_water — owner/architect call.
7. Assets<Material> rows are AoS `Material { gpu, textures }` (the transparency design adds `x`). A device-resident `gpu` lane needs the asset store to split lanes — this is the scope of KF-device-column-residency's asset arm.
8. Gpu3dInstance + Render3dPlugin: delete or keep (no production consumer on J; composed at J:crates/boyko_app/src/plugins.rs:431) — scope call.
9. M:docs/memory/ is untracked (`?? docs/memory/`); the ALLOCATOR-DESIGN-SPACE rows cited (:322, :337) are not committed and conflict with the ledger's ComponentPool-backed forms and with the rejected-bespoke-column ruling.
10. SDF scene has three stores (render SdfEditStaging, physics SdfField, empty-field brick clipmap) — must be unified together with the physics study (J:crates/boyko_physics/src/sdf_query.rs:46; J:crates/boyko_app/src/gpu_scene/mod.rs:1036).
11. Asset streaming after boot is unwired: AssetStaging is drained only by the boot one-shot (J:crates/boyko_app/src/runner.rs:830). Is a per-frame drain system in scope?
12. Doc rot to repair: sdf_edit.rs:8 and :111 claim collect_sdf_edits is a startup system (the plugin only inserts the staging, :152); instance_model.rs:133 claims PrevInstanceModelCol is dense, but its derive at :158 is table storage.
13. The reflections design's Captured probes need per-view render data and per-probe capture state; no form is designed yet, and the renderer is single-view today (one ViewUniform Resource) — M:docs/render/REFLECTIONS-DESIGN-SPACE.md:112, :986.
14. The ledger's plan_conflicts (UI pack Vecs: FrameVec vs dense component vs OK-by-design; GpuColumnManager.meta: Stage 5 vs MEMORY-SYSTEM-AUDIT) remain unresolved; this report follows the ledger/Stage 5 side.
15. The ledger's UI rows were taken on J and are 16 commits stale against U; the render-side UI scratch on U (U:crates/boyko_render/src/ui/pack.rs:801 `pub pack: Vec<UiInstance>,`, U:.../ui/upload.rs:203 `node_buf: Vec<UiNode>,`) is left to the UI lens.

---

# Lens 3 of 6: host-io

# Host and I/O lens: boyko_app, boyko_rhi/_vulkan outside the render passes, boyko_input

## 0. Inputs, trees, method

- **Primary tree J** = `D:/wt/joltab` @ `d11962a9`. **U** = `D:/wt/ui` @ `615cda8f`: I used it only for the input→UI edge. **M** = `D:/claude/BoykoEngine`: I used it only for the playground example. That example is **untracked in M** (`git status`: `?? crates/boyko_app/examples/playground.rs`) and exists on neither J nor U. `docs/memory/` is also **untracked** in M (`?? docs/memory/`), so the allocator vocabulary I borrow from it (`FrameArena`, `FrameVec`) is uncommitted text.
- **Ledger.** The files exist: `ledger/app-demo.verified.json`, `rhi.verified.json`, `ui-input.verified.json`. They were taken on J@`ca582e72`. `git merge-base --is-ancestor ca582e72 d11962a9` is true, and `git diff --stat ca582e72 d11962a9 -- crates/boyko_app crates/boyko_input crates/boyko_rhi crates/boyko_rhi_vulkan` is empty. So **every ledger row in this lens is current**. The ui-input group is J-based, so its boyko_ui rows are 16 commits stale, as the brief warned. That is not my lens.
- **No cargo and no benchmarks were run.** Every cost statement below comes either from the code's structure or from a figure the kernel documents about itself. Each one is labelled. graphify was queried once, returned off-target results, and I fell back to Grep/Read.

## 1. The frame as it actually runs, and where each piece of logic executes

The runner closure is installed at `J:crates/boyko_app/src/plugins.rs:803` `app.set_runner(Box::new(move |app: &mut App| {`. It owns the whole lifecycle.

**Boot.** All of these are host code, and none is a scheduled system:

1. The device singleton. `J:crates/boyko_app/src/runner.rs:164` `let ctx = match VulkanContext::boot_singleton(config) {`. The owning pointer is a process static: `J:crates/boyko_rhi_vulkan/src/device.rs:776` `static SINGLETON: AtomicPtr<VulkanContext> = AtomicPtr::new(ptr::null_mut());`.
2. The window chain. `runner.rs:174` `let mut host = match WindowHost::boot(ctx, &desc) {`.
3. About two dozen World residents, inserted by the runner rather than by plugins (`runner.rs:238-351`, `:568`, `:587`, `:630`).
4. The render-path resolve, called directly. `runner.rs:547` `boyko_render::resolve_render_path(&render_path_cfg, render_path_consumers, render_path_caps);`. The plugin registers no system for it. `plugins.rs:526-527` "registers NO per-frame system (Decision 1 … `boyko_app::runner` calls `resolve_render_path` directly at boot".
5. `app.finish()` at `runner.rs:638`. Startup systems are a push-ordered list with no sets: `J:crates/boyko_ecs/src/ecs/core/app/app.rs:175` `startup: Vec<StartupSystem>,`.
6. Seven post-`finish()` one-shots that the runner orders by hand:
   - `runner.rs:661` `app.world_mut().run_system(collect_sdf_edits);`
   - pipeline builds (`:676`, `:688-773`, `:814`)
   - `runner.rs:835-837` `app.world_mut().run_system(upload_material_assets);` …
   - `:846` backfill
   - `:862-869` `MaterialTable::boot_seed`, via a Resource take-out and reinsert.

**Frame loop.** `runner.rs:1175-1282`. Only step F4 runs on the engine scheduler.

| # | step | citation | executes as |
|---|---|---|---|
| F1 | OS pump | `runner.rs:1177` `if !host.window.pump_events() {` | host code (OS message loop) |
| F2 | drain `InputRing` → translate → `RawInputQueue`, or an inline Escape scan | `runner.rs:1203-1204` `host.window` / `.drain_input(\|captured\| ingest_captured(&mut *queue, captured));`; fallback `:1205-1220` | host code writing the World |
| F3 | publish the fence clock | `runner.rs:1232` `*app.world_mut().resource_mut::<RenderEpoch>() = RenderEpoch(host.renderer.submission_epoch());` | host code writing the World |
| F4 | `update_with_delta` = Time → gated event swap → Fixed×N → Main | `runner.rs:1233`; `app.rs:709-744` | **the only scheduled logic** |
| F5 | AppExit check; skip when minimized | `runner.rs:1236`, `:1242` | host code |
| F6 | fence wait → `FrameWriteToken` | `runner.rs:1249` `let token = match host.renderer.wait_frame_in_flight() {` | host code (driver wait) |
| F7 | deferred-free drain | `runner.rs:1264` `retire_deferred_frees(` (a plain fn over `&mut EcsMaster`) | host code writing the World |
| F8 | GPU-mirror grow, via NonSend take-out and reinsert | `runner.rs:1296-1299` `.remove_non_send_resource::<RetiredGpuBuffers>()` | host code writing the World |
| F9 | material/TLAS repoint | `runner.rs:1341-1369` | host code |
| F10 | SDF edit-list one-shot | `runner.rs:1418-1446` | host code writing the World |
| F11 | **TAA / jitter / motion-camera state advance** | `runner.rs:1537` `if let Some(taa_state) = app.world_mut().try_resource_mut::<TaaState>() {`; `:1552` `advance_jitter(jitter, taa_armed_now);`; `:1604` `…resource_mut::<boyko_render::MotionCamState>().advance(cur))` | **ECS Resource state transitions done by host code** |
| F12 | VB instance ring, rebuilt as a one-shot every frame | `runner.rs:1619` `app.world_mut().run_system(boyko_render::sync_vb_instance_ring_system);` | one-shot system, rebuilt per frame |
| F13 | uploads, draw-list build, scene assembly, record+submit | `runner.rs:1655-2661` (render at `:2640` `host.renderer.render_gbuffer_frame(`) | host code |
| F14 | env-gated readback and probe drivers | `runner.rs:2663-3261` | host code; inserts Resources on completion (`:3214-3216`) |
| F15 | post-present publish | `runner.rs:3268` `*app.world_mut().resource_mut::<WindowInfo>() = WindowInfo {`; `:3274` `stats.frames += 1;`; `:3281` `frame_index = frame_index.wrapping_add(1);` | host code writing the World |

**Teardown** is hand-ordered, 1-3 then 4:

- `runner.rs:3483` `unsafe { destroy_host_gpu_chain(host, ctx) };`
- evictions at `:3496-3577`
- the singleton destroy at `:900` `unsafe { VulkanContext::destroy_singleton() };`
- the log shutdown at `:928` `let _ = boyko_log::lifecycle::shutdown();`

**Net.** Of everything the loop does per frame, only F4 is on the scheduler. F3, F7, F8, F10, F11, F12 and F15 mutate World state from outside any system.

## 2. Ownership partition

- **OS-owned (out of scope):**
  - HWND, HINSTANCE, the window-class table, the thread message queue and the WNDPROC dispatch
  - raw-input device registration (`window.rs:390-396`)
  - the timer period held by `TimerResolutionGuard` (`plugins.rs:820`)
  - the environment variables
  - the `Instant` clock
  - the `.keys` file (`J:crates/boyko_input/src/plugin.rs:100` `let Ok(src) = std::fs::read_to_string(path) else {`)
- **Driver-owned (out of scope):**
  - the loader module, `VkInstance`/`VkDevice`/`VkQueue`
  - `VkSurfaceKHR`, `VkSwapchainKHR` and its `VkImage`s
  - fences, semaphores, command pools and buffers
  - the bytes of every mapped host-visible staging ring
  - the debug-messenger callback invocation
- **Engine-owned but kept outside the ECS.** This is what the orders target:
  - `InputRing` (Box) and the `Window` struct's own fields
  - the `VulkanContext` Box and its `device_name: String`, `device_fns: Box<DeviceFns>`, `debug_state: Option<Box<DebugMessengerState>>`, `host_pool`/`device_pool: RefCell<BlockPool<…>>` (`device.rs:677`, `:698`, `:688`, `:728`, `:737`)
  - all of `WindowHost` (`J:crates/boyko_app/src/host.rs:81` `pub(crate) struct WindowHost {`), including the handle arrays inside `Renderer`/`Swapchain`
  - the runner's stack locals (the probes, `frame_index`, `last`, the `vb_zone_*` counters, `WindowReducer`)
  - the `ACTION_NAMES` static
  - the diag `OnceSite` latches (a log-lens concern)
- **Already ECS residents** (the runner inserts them):
  - Resources: `WindowInfo`, `HostFrameStats`, `RenderEpoch`, `AppExit`, `DdgiCaps`, `RayCaps`, `ResolvedRenderPath`
  - NonSend residents: `GpuDevice`, `RhiContext`, `Assets<MeshGpu>`, `MaterialTable`, `RetiredGpuBuffers`, `BindlessTextureTable`, `Assets<TextureGpu>`, `AssetStaging<A>`, `AssetPaths<A>`
  - input Resources: `RawInputQueue`, `PhysicalInput`, `ActionState<A>`, `InputMap<A>` (`plugin.rs:119-124`)
  - the fly-camera component `FlyCamera` (`J:crates/boyko_scene/src/camera.rs:882`)
- **Layering constraint that governs every RHI row.** `boyko_rhi_vulkan` depends only on `boyko_rhi`, `boyko_sdf_math`, `boyko-diag` and `boyko-log`; `boyko_rhi` depends only on `boyko-utils` and `boyko-log` (their `[dependencies]` sections). Neither can name `boyko_ecs`. Any ECS residency for window, swapchain or device state must therefore happen by wrapping at `boyko_app`, and the memory placement needs a World-free leaf memory primitive (the ledger's `new_primitives`, `rhi.verified.json`).

## 3. Structures glued on the side, with evidence

### H-1. Two input rings in series, and the ECS one is a single-consumer destructive queue

- **The first ring.** It is engine-owned and lives outside the World.
  - `J:crates/boyko_rhi_vulkan/src/window.rs:121` `struct InputRing {`
  - It is boxed and handed to the OS: `:365` `let input_ring = Box::into_raw(Box::new(InputRing::with_capacity(INPUT_RING_CAP)));` and `:372` `os::SetWindowLongPtrW(hwnd, os::GWLP_USERDATA, input_ring as isize);`
  - Its capacity is fixed, so it never grows, and the raw pointer the WNDPROC holds stays stable.
  - It coalesces raw-mouse deltas at push (`:179-189`).
- **The second ring.** The runner re-translates and re-pushes into it.
  - `J:crates/boyko_input/src/raw/queue.rs:33` `pub struct RawInputQueue {` over `:35` `buf: Box<[RawInputEvent]>,`
  - Both rings are 1024 entries (`window.rs:59`, `constants.rs:8` `pub const RAW_QUEUE_CAP: usize = 1024;`).
- **Consumption is destructive.**
  - `queue.rs:103` `pub fn pop(&mut self) -> Option<RawInputEvent> {`
  - `J:crates/boyko_input/src/action/process.rs:64-66` `while let Some(ev) = queue.pop() {` / `physical.apply(&ev);`
- **Three concrete consequences.**
  1. **Multi-`InputPlugin` edge loss.** This is derived from the code and has not been run.
     - Each `InputPlugin<A>` registers its own `update_action_state::<A>` (`plugin.rs:139`), and that function resets the shared snapshot first: `process.rs:59-60` `queue.begin_frame();` / `physical.begin_frame();`. The reset clears the edges (`queue.rs:230-231` `self.keys_just_pressed = BitSet256::new();`).
     - With `FlyCameraPlugin` (`J:crates/boyko_app/src/fly.rs:113` `app.add_plugin(InputPlugin::<FlyAction>::new(fly_default_map()));`) plus a game's own `InputPlugin<GameAction>`, the second ingest drains an empty queue and wipes the `just_pressed`/`just_released` edges the first one folded. That action set, and every later `PhysicalInput` edge reader, sees no presses. Held levels survive.
     - No test adds two `InputPlugin`s (a grep for `InputPlugin::<` over the J workspace finds only `fly.rs:113`).
  2. **`Text` events can never reach a consumer.**
     - The only drain drops them: `queue.rs:302` `RawInputEvent::Text(_) => {}`.
     - The native capture list has no `WM_CHAR` anyway: `window.rs:651-654` `os::WM_KEYDOWN | os::WM_KEYUP | … | os::WM_MOUSEWHEEL | os::WM_MOUSEHWHEEL =>`. For the same reason `CursorLeft` and `WindowFocus` are never produced natively.
  3. **`RebindSession::feed(&RawInputEvent, …)` (`action/rebind.rs:89`) has no raw stream to read.** The ingest consumes it before any user system runs.
- **The UI reads input by cloning the snapshot out of the World** (U: `crates/boyko_ui/src/interaction/focus.rs:150` and `world/pick.rs:235`, both `let physical = world.resource::<PhysicalInput>().clone();`).
- **The UI also writes the action state from outside the ingest** (U: `interaction/dispatch.rs:103` `world.resource_mut::<ActionState<A>>().ui_press(index);`, and `:128` `…freeze_fixed_snapshot();`).

### H-2. The window size lives in three places, and the two ECS copies never meet

- **`Window.width/height`**, a host cache (`window.rs:232-233`), refreshed by `refresh_size` (`:438`).
- **`WindowInfo`**, written by the runner every frame (`runner.rs:3268`) and read by no engine system: `J:crates/boyko_app/src/window_info.rs:13-15` "so no v1 engine system consumes this". Its only reader is a test (`boyko_app/tests/room_smoke.rs`).
- **`UiViewport`** (U: `crates/boyko_ui/src/resources.rs:19-20` "Screen-space root seed. Set by the host (window/swapchain) on surface create and resize.").
  - No production writer exists. The only write is in a test: U `layout.rs:2112` `world.resource_mut::<UiViewport>().generation = 1;`.
  - `boyko_app` names no `boyko_ui` on J or on U: grepping `boyko_app/Cargo.toml` and `boyko_app/src` for `boyko.ui` returns nothing on both trees.
- **Three copies of one datum**: one written but unread, one read but unwritten, and a hand-rolled `generation` field standing in for change detection. That last point is the K3 gap below.

### H-3. Boot commitments held as host fields, mirrored into the World or overriding it

- **The fields:**
  - `host.rs:102` `pub(crate) composite_extent: (u32, u32),`
  - `:110` `pub(crate) ssaa_armed: bool,`
  - `:116` `pub(crate) ssaa_scale: u32,`
  - `:120` `pub(crate) native_extent: (u32, u32),`
  - `:131` `pub(crate) resolved_render_path: ResolvedRenderPath,`
- **The same value is stored twice.** `runner.rs:559` `host.resolved_render_path = resolved_render_path;`, then `:568` `app.world_mut().insert_resource(resolved_render_path);`. Per-frame readers take the host copy (`:1618`, `:1681`, `:2083`, `:2570`).
- **SSAA is a host lock over ECS state.** `runner.rs:647-648` `if host.ssaa_armed {` / `app.world_mut().insert_resource(AaConfig { mode: AaMode::Ssaa });`, and each frame `:2454` `let aa_mode = if host.ssaa_armed {`. The result: `ResolvedAa` is not authoritative. The host is.
- **Consequence:** the composite extent and `taa_supported()` are invisible to systems. That is the only reason the TAA, jitter and motion-camera state transitions (F11) are runner code rather than Main systems.

### H-4. Per-frame logic outside the scheduler costs more than the scheduled form

- **The one-shot is rebuilt every frame.** `run_system` rebuilds the system on each call: `J:crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs:90` "rebuilds the system on every call (≈ 1 µs cold init + ≤ 30 ns …". That is the kernel's own figure; I did not measure it.
- **Each rebuild does at least one 24 KB heap allocation.** Init constructs `J:crates/boyko_ecs/src/ecs/core/system/function_system.rs:223` `let mut access_set = FilteredAccessSet::new();`, which allocates `J:…/system/filtered_access_set.rs:146` `bit_owners: Box::new([""; OWNERSHIP_SLOT_COUNT]),`, 24 KB per its own doc at `:115`. So F12 performs **at least one 24 KB allocation and free on every VisibilityBuffer frame**. This is derived from the code, not counted.
- **The ledger cannot see this.** It is a field/container scan, and the allocation is kernel-side, triggered by the host.
- **A scheduled system would be cheaper**, because it is built once. NonSend-param systems are already schedulable: `J:…/system/system_meta.rs:145-149` "Set by `NonSendRes` / `NonSendResMut::init_access` … resolves the system to `SystemKind::CpuExclusive`".
- **Grow frames pay Box free and reallocation.** The take-out/reinsert pattern exists because the host holds half the GPU mirror and the World the other half:
  - `runner.rs:1286-1290` "the safe resource facade cannot split a live `&World` borrow across Send/NonSend storage, so `MaterialTable`/`RetiredGpuBuffers` are taken out (owned)"
  - `:1323` `host.gpu.grow_instance_family_if_needed(` takes `&mut retired`, a World resident
  - A system taking both as params needs no take-out. `run_cached_system` also exists (`system_api.rs:139`).

### H-5. Frame counter and clock duplicated on the runner stack

- **`frame_index` duplicates `HostFrameStats.frames`.** `runner.rs:1068` `let mut frame_index: u32 = 0;` is incremented only at `:3281`, right after `:3274` `stats.frames += 1;`, and both sit behind the same `continue` and `return` paths. So `frame_index == HostFrameStats.frames as u32` at every loop top. `Time` has no frame counter (`J:crates/boyko_ecs/src/ecs/core/time/time.rs:30-39`).
- **`last` duplicates the App's own clock.** `runner.rs:1064` `let mut last = Instant::now();` versus `app.rs:171` `last_instant: Option<Instant>,`, which `App::update` uses (`app.rs:770-775`).

### H-6. The boot sequence is hand-ordered around `finish()` because Startup has no ordering

- `runner.rs:655` "a plugin-registered startup gather would race (run BEFORE) the user's later `setup` and see zero primitives."
- `runner.rs:520-526` records the other trap: "a STARTUP SYSTEM that sets `vb_sdf_mesh_shadow`/`_ao` is ALREADY TOO LATE".
- The root cause is the push-ordered `Vec<StartupSystem>` (`app.rs:175`; push at `app.rs:514`).

### H-7. Teardown is hand-ordered eviction of device-owning residents

- `runner.rs:3496` `drop(app.world_mut().remove_non_send_resource::<RhiContext>());` through `:3577`, followed by the post-run debug-assert list (`:905-912`).
- The documented leak class: `runner.rs:3549` "A let-chain TAKES the first resource unconditionally, and if the second `remove` is `None` the chain's body never runs".

### H-8. The windowed engine has no UI

- The windowed entry point takes no UI. `J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:821-836` (`pub unsafe fn render_gbuffer_frame(` … `vb_record_probe: Option<&mut VbRecordProbe>,`) has no UI parameter.
- Only the test-side path has one: `:696-706` `pub unsafe fn present_sampled(` … `ui: Option<&UiPass<'_>>,`.
- `EnginePlugins::build` composes no `UiPlugin` (`plugins.rs:379-824`).
- The untracked playground confirms the effect: M `crates/boyko_app/examples/playground.rs:47-49` "the ECS UI stack exists and is complete, but it **cannot reach the windowed screen today** — `boyko_app` names no `boyko_ui`".

### H-9. Validation messages bypass the engine's log

- `J:crates/boyko_rhi_vulkan/src/debug.rs:114` `eprintln!("[vk-validation] {}", msg.to_string_lossy());`
- The counters (`:97` `state.errors.fetch_add(1, Ordering::Release);`) are read only by tests and one example (`boyko_render/examples/orbit_cube_window.rs:653`), never by the host.

### H-10. A process-global static stands in for an ECS table

- `J:crates/boyko_input/src/action/names.rs:42` `static ACTION_NAMES: OnceLock<Box<[(&'static str, u16)]>> = OnceLock::new();`
- It is first-writer-wins per process (`names.rs:23-29`).
- Its reader, U `crates/boyko_ui/src/text/dispatch.rs:879` `match resolve_action_name(token) {`, runs under `parse_and_insert(…, cmds: &mut Commands, …)` (U `dispatch.rs:64-68`). That is a system context which could take a `Res` instead.

### H-11. Diagnostics drivers are runner stack locals

- `runner.rs:1020-1044`, for example `let mut dump = crate::host_dump::HostDump::from_env(host.swapchain.format());`, plus the zone counters at `:1088-1160`.
- The host already inserts results into the World on completion (`runner.rs:3216` `app.world_mut().insert_resource(readback);`). So the pattern is half-ECS.

### H-12. The playground indexes entities from a Resource (M, untracked)

- M `playground.rs:330` `balls: [Option<Entity>; MAX_BALLS],` and `:336` `props: [Option<Entity>; MAX_PROPS],` inside `#[derive(Resource)] struct Playground` (`:315-316`).
- These are entity sets that marker components plus a query express natively.

### H-13. Nobody in the engine uses the kernel event system

- Grepping `src/` on J and on U for `EventWriter<`, `EventReader<` and `impl Event for`, outside `boyko_ecs`, tests and aether, finds only `boyko_macros/src/state_chart/emit.rs`. `#[event]` appears only in aether.
- This matters for H-1's fix. Input would be the first production event consumer, and the pause and 0-substep hazard (K2) is the gating kernel issue.

## 4. Kernel features this lens needs

The names are provisional. The physics study's feature names were not available to me; align them where the need is the same.

### K1 `external-event-writer`

- **Purpose:** an OS-callback producer that writes a kernel event lane directly, from the dispatcher thread, outside `Schedule::run`/`update_events`.
- **What already exists:**
  - An FFI or main-thread send path: `J:…/system/params/event_writer.rs:115` "use EcsMaster::events().send_event::<E>(...) for main-thread / FFI callers".
  - The accessor: `J:…/ecs_master/ecs_master.rs:607` `pub fn events(&self) -> &EventDispatcher {`.
  - Heap-stable buffers: `J:…/params/event_reader.rs:53` `buffer_ptr: NonNull<EventBuffer<E>>,`.
- **What is missing:** the WNDPROC has only `GWLP_USERDATA` and no `&EcsMaster`. It needs a Copy handle (buffer pointer plus dispatcher lane) obtainable once from `&EcsMaster`.
- **Only needed to delete `InputRing`.** Until it lands, the runner's drain can already `send_event` into the lane with no new feature.
- **Coalescing has to move.** Lane overflow refuses the newest event (`J:…/events/event_buffer.rs:73-99` "a write lane was full, so the send was refused"), whereas `InputRing` drops the oldest. So the sink must keep the raw-mouse coalescing from `window.rs:177-189`.

### K2 `per-type-event-swap-policy`

- **Why the event form fails today:**
  - `EnginePlugins` always creates a Fixed schedule: `plugins.rs:779` `app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {`.
  - That selects `app.rs:593` `None if self.fixed_builder.is_some() => EventUpdatePolicy::WaitForFixed,`.
  - Under that policy the swap is gated: `app.rs:713-715` `if self.event_policy == EventUpdatePolicy::EveryFrame` / `\|\| self.fixed.is_none()` / `\|\| self.fixed_steps_since_swap > 0`.
  - `EventConfig` has no per-type knob: `J:…/events/event_config.rs:16-21` holds only `thread_count` and `capacity_per_lane`.
- **Effect on input sent as events:**
  - Every 0-substep frame adds a frame of latency. At 64 Hz fixed (`app.rs:69-70`) and a 144 Hz render, about 56% of frames are 0-substep. That is arithmetic, not a measurement.
  - Under pause, input freezes: `app.rs:81-83` "a paused [`Time`] yields 0 substeps every frame, so the swap is held INDEFINITELY — starving ALL readers … (a paused menu sending UI events".
- **Why not just switch globally.** A global `EveryFrame` (`app.rs:490` `set_event_update_policy`) is harmless today, because nothing reads events in Fixed (H-13). It would break Fixed-produced events such as the physics study's likely contact events. That is why the policy must be per type.

### K3 `resource-change-ticks` (opt-in per Resource type)

- **What the kernel has:** `ResMut` is a bare `&mut`. `J:…/params/resmut.rs:42` `pub struct ResMut<'w, R: Resource>(pub(crate) &'w mut R);`.
- **Hand-rolled substitutes:**
  - U `resources.rs:23-25` "The host bumps `generation` on every resize … (resources have no `Changed` semantics)"
  - `host.rs:138` `pub(crate) light_uploaded_gen: [u64; FRAMES_IN_FLIGHT],` against `LightTableGeneration`
  - `host.rs:147` `particle_effects_uploaded_gen`
- **Hot-path cost:** opt-in, the same way `ComponentPool` has tracked and untracked modes (`J:…/component/scratch/scratch_column.rs:14-19`). An untracked Resource pays nothing. A tracked one pays one tick store per mutable deref, which replaces today's hand bump of one `u64`. That is cost-neutral by construction.

### K4 `ordered-startup-stages`

- **What it is:** set-ordered Startup, or boot-commit and post-startup stages, so that caps resolve, the render-path resolve and the seven post-`finish()` one-shots (H-6) become systems with named edges.
- **Cost:** boot only; zero per frame.

### K5 `ordered-nonsend-teardown`

- **What it is:** dependency-ordered eviction of device-owning NonSend residents (H-7).
- **What it would allow:** the device context could also become a World resident instead of the `device.rs:776` static, which today exists because the `&'static` holders live outside the World (`host.rs:83` `pub(crate) renderer: Renderer<'static>,`).
- **Cost:** shutdown only.

### No kernel feature needed for these

- **The present tail (F6-F15) becomes one exclusive, dispatcher-pinned system at Main's tail.**
  - `WindowHost` is `'static`, so it is a valid `NonSendResource`.
  - The fence proof `FrameWriteToken` stays a local inside that one system. It is consumed by value at `runner.rs:2641`. Splitting the tail into several systems would force the token into a runtime `Option` Resource and lose the type-level proof, so do not split it.
- **F11 and F12 become ordinary Main systems** once H-3's fields are a Resource.
- **The `RenderEpoch` publish (F3) moves to the tail system, post-submit.** The value is identical: nothing else bumps `submission_epoch` between the submit and the next frame's head.
- **The OS pump (F1/F2) stays host code, and deliberately so.** Events sent inside Main become visible only after the next frame's swap (`app.rs:709-720`). Pumping inside Main would therefore add one frame of latency always. Pumping stays pre-`update_with_delta` and writes ECS storage (K1, K2).

## 5. Table

Refutation shorthand. Each tag is one sentence, and each row cites the tags that apply plus its own evidence.

- **¬C1:** exactly one per World, with no entity it describes. There is one window (`runner.rs:174`, `host.rs:154` `pub(crate) window: Window,`), one device (`device.rs:776`) and one raw source (`CapturedMsg` carries no device id, `window.rs:89-107`).
- **¬D:** a dense component is a storage mode of a component, and there is no component here to store densely.
- **¬E:** not a per-entity boolean.
- **¬R:** not an edge between entities.
- **¬Ev-state:** state that persists across frames and is read as state, not a transient message.
- **¬RC-1sys:** transient and read by exactly one system within one frame, so there is no table for others to read.

| datum or loop | today | natural ECS form | kernel feature needed | cost argument for the hot path it touches |
|---|---|---|---|---|
| `InputRing` `window.rs:121` | engine Box behind `GWLP_USERDATA` | **event** (merged with the next row); ¬C1 ¬D ¬E ¬R | K1 to delete it; the runner-drain interim needs none | Per OS message: one lane `send_one` (~5 ns per `event_writer.rs:95-96`, a doc figure) instead of ring push + pop + translate + push + pop. Coalescing moves to the sink. |
| `RawInputQueue` `queue.rs:33` | Resource, destructive single-consumer ring | **event** `struct RawInput { ev: RawInputEvent }` (`#[event]` needs named fields: `boyko_macros/src/event.rs:43`); ¬C1 ¬D ¬E ¬R | K2 | The fold reads a lane slice rather than popping. Multiple readers (per-`A` ingest, rebind, text) become free. Fixes H-1's edge loss and `Text` drop. |
| `PhysicalInput` `queue.rs:167` | Resource, inline POD | **resource-column** (already); ¬C1; ¬Ev-state (`queue.rs:236` "`cursor_pos` and the held levels persist") | none | Fold it once in one non-generic system instead of per `A` (`process.rs:59-66`), removing N-1 redundant resets and drains. UI may borrow it via `Res` instead of cloning (U `focus.rs:150`). |
| `ActionState<A>` `state.rs:42` | Resource plus 4×`Box<[f32]>` (`:61-67`) | **resource-column**; ¬C1 in v1 (one per `A`, `plugin.rs:121`); ¬D ¬E ¬R ¬Ev-state | none (memory lens: inline arrays or kernel columns) | Unchanged bit tests; the value arrays leave the system heap. |
| `InputMap<A>` `map.rs:132` | Resource, CSR in two Boxes | **resource-column** (CSR on kernel columns; ledger `CsrColumn`); ¬C1 ¬D ¬E ¬R ¬Ev-state | none | Sequential `bindings_at` walk unchanged; rebind stays cold (`map.rs:241-254`). |
| `ACTION_NAMES` `names.rs:42` | process static `OnceLock` | **resource-column** (`ActionNames` Resource); ¬C1 ¬D ¬E ¬R ¬Ev-state | none | Load-time only; removes the first-`A`-wins process coupling. |
| `RebindSession<A>` `rebind.rs:37` | user-owned struct | **component** on the widget entity that is listening, read with `EventReader<RawInput>` | K2 (through the event row) | Cold UI path. |
| `Window` fields (`class_name: Vec<u16>` `window.rs:230`, size cache, ring pointer) | inside `WindowHost` | **resource** (NonSend singleton) for the engine fields; HWND/HINSTANCE/atom are out-of-scope:os-owned; ¬C1 (multi-window is an owner scope call) ¬D ¬E ¬R ¬Ev-state | K5 (ordered eviction before the surface) | Only the pump touches it, once per frame. |
| Window size ×3 (H-2) | host cache + `WindowInfo` (unread) + `UiViewport` (unwritten) | **one resource-column** (surface size, scale, change tick); ¬C1 ¬D ¬E ¬R ¬Ev-state | K3 (replaces `generation`) | One store per frame post-present (today's `runner.rs:3268`); UI relayout keyed on the tick. |
| Boot commits (H-3) | `WindowHost` fields, duplicated/overriding ECS | **resource-column** (a boot-commit Resource; `ResolvedRenderPath` already exists); ¬C1 ¬D ¬E ¬R ¬Ev-state | K4 (to write it in a boot stage) | Read-only per frame. Turns the AA lock into the `ResolvedAa` resolve and unlocks F11 as systems. |
| `light_uploaded_gen` / `particle_effects_uploaded_gen` `host.rs:138/147` | host arrays indexed by FIF slot | **resource-column** on the World-resident GPU mirror (precedent: `J:crates/boyko_render/src/material_table.rs:97` `rebind_pending: [bool; FRAMES_IN_FLIGHT],`); ¬C1 (per slot, not per entity) ¬D ¬E ¬R ¬Ev-state | K3 optional | Two compares per frame either way. |
| `retire_scratch: Vec<FreeEntry>` `host.rs:96` | host Vec, highwater | **system-scratch** (ScratchColumn or FrameArena; the ledger agrees); ¬RC-1sys | none (`ScratchColumn` exists: `scratch_column.rs:43`) | Zero steady-state either way; leaves the system heap. |
| `DrawListScratch` `gpu_scene/mod.rs:8109` | host Vec relabelled to `'static` | **system-scratch** (FrameArena `FrameVec`); render lens decides whether it disappears | none | Per-frame gathered copy of `MeshRenderScratch.batches`; handed to the render lens. |
| `GpuSceneBundles` `host.rs:90` | host field; siblings are World residents | render lens; host note: one owner, not split (H-4 dance) | none | Removes Box free and reallocation on grow frames (`runner.rs:1296-1333`). |
| `Renderer`/`Swapchain` handle arrays (`frame_driver.rs:48` `render_finished: Vec<VkSemaphore>`, `present/swapchain.rs:64` `images: Vec<VkImage>`) | fields of host structs | **resource** (NonSend present chain) for the holders; the handles are out-of-scope:driver-owned; ¬C1 ¬D ¬E ¬R ¬Ev-state | K5 | Read per frame by the present path only; memory lens (InlineVec). |
| `VulkanContext` + `SINGLETON` `device.rs:666/776` | process static, `&'static` fiction | **resource** (NonSend World resident); ¬C1 ¬D ¬E ¬R ¬Ev-state | K5 (plus root-provenance handles for its borrowers) | Read-only after boot; zero hot-path change. Blocked today by the Cargo layering and the `'static` holders. |
| `DebugMessengerState` + `eprintln` `device.rs:688`, `debug.rs:114` | Box handed to the driver; stderr side channel | messages → the engine log (`boyko_log`), not an ECS store; counters → **resource-column** if the host ever reads them; Box stays address-stable (driver holds `p_user_data`) | none | Validation builds only. |
| `frame_index`, `frames_run`, `last` `runner.rs:1063-1068` | runner locals duplicating `HostFrameStats.frames` and `App::last_instant` | frame count → **resource-column** (a field of the kernel `Time`); `last` → **kernel-internal** (`app.rs:171`) | none | One increment per frame. |
| `HostFrameStats`, `RenderEpoch`, `AppExit` | Resources | **resource-column** (already) | none | Writers move into the tail system; values identical. |
| inline Escape fallback `runner.rs:1205-1220` | host logic duplicating `quit_on_action` | delete once the raw ingest is unconditional (a default quit action) | K2 | Removes a per-frame branch plus a translate. |
| diagnostics drivers `runner.rs:1020-1160` | runner locals | **resource-column**, presence = armed (the capability-by-presence ruling; precedent `runner.rs:3216`); ¬C1 ¬D ¬E ¬R | none | Steady path unchanged (absence = one test). |
| `WindowDesc` via closure capture `plugins.rs:797-803`, scattered `BOYKO_*` env reads | closure capture and ad-hoc env reads | **resource-column** (a `WindowConfig`/`LaunchConfig` parsed once in build); the env itself is os-owned | none | Boot only. |
| `Playground` entity arrays (M `playground.rs:330/336`) | Resource holding `[Option<Entity>; N]` | **component** (marker + spawn sequence) + query | none | N ≤ `MAX_BALLS`; an O(N) minimum over a handful of rows. |
| `FrameWriteToken` `runner.rs:1249` | typed local | out-of-scope:compile-time (a type-level proof) | none | Keep it local inside one tail system. |
| `TimerResolutionGuard` `plugins.rs:820` | RAII local | out-of-scope:os-owned | none | none |
| **L: OS pump + drain** `runner.rs:1177-1221` | host code pre-update | host code by necessity (os-owned loop, must precede the swap); writes kernel events | K1, K2 | Moving it into Main would add one frame of latency always (`app.rs:709-720`). |
| **L: present tail** F6-F15 `runner.rs:1249-3281` | host code | **one exclusive system at Main's tail** (NonSend params, `system_meta.rs:145-149`) | none | Already serial on the dispatcher; one node dispatch (≤ 30 ns, `system_api.rs:90-91`, a doc figure). CPU/GPU overlap preserved (`runner.rs:1009`). |
| **L: TAA/jitter/motion-camera advance** `runner.rs:1535-1608` | host code over Resources | **Main systems** | none (after the boot-commit row) | Serial runner code becomes parallel-capable nodes: cost ≤ today. |
| **L: VB ring** `runner.rs:1619` | per-frame `run_system` | **Main system** after `gather_mesh_draws`, run condition `path == VB` | none | Removes ≥ one 24 KB Box plus a ≈1 µs init per VB frame (code-derived, not measured). |
| **L: boot one-shots and resolve** `runner.rs:454-869` | host code around `finish()` | **boot-stage systems** with named edges | K4 | Boot only. |
| **L: teardown** `runner.rs:3477-3581` | hand-ordered | kernel-ordered eviction | K5 | Shutdown only. |
| **L: UI record** | absent (H-8) | UI pass as part of the frame the tail system records | none known from this lens (render + UI lenses decide) | not assessable here |

## 6. Ledger cross-check

- **Class correction.** `rhi.verified.json` classes `present/surface.rs:238` (`present_mode_supported`) as "F perframe". Its callers are `present/swapchain.rs:297` and `:312`, inside `fn build` (`:243`), reached from `Swapchain::new` (`:144`) and `recreate` (`:193`). It runs per swapchain (re)creation, not per frame.
- **Blind spot.** The ledger's field/container scan cannot see a kernel allocation that host control flow triggers: H-4's per-frame 24 KB `FilteredAccessSet` via `run_system`. Only a heap census on a VisibilityBuffer boot counts it.

## Open list (lens host-io)

1. Confirm with a tester run (not done: no cargo allowed) that two InputPlugin<A> of different A lose just_pressed edges: the second update_action_state::<A> calls physical.begin_frame() on an already-drained queue (J:crates/boyko_input/src/action/process.rs:59-66). No test covers two InputPlugins.
2. Measure on a VisibilityBuffer boot whether the per-frame run_system(sync_vb_instance_ring_system) at J:crates/boyko_app/src/runner.rs:1619 really allocates the 24 KB FilteredAccessSet (J:crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs:146) every frame. This is code-derived only; the owner said the machine is not quiet.
3. Event swap policy: add a per-type swap policy (K2) or set EveryFrame globally in EnginePlugins? A global EveryFrame is safe today because no production system reads events in Fixed, but it conflicts with Fixed-produced physics events. The physics study's event plans are needed to decide.
4. Device ownership: keep the process-singleton static (J:crates/boyko_rhi_vulkan/src/device.rs:776), or make the World own the VulkanContext as a NonSend resident? The latter needs K5 plus root-provenance handles for the Renderer<'static>/Swapchain<'static>/GpuDevice borrowers. This is an owner/architect call, and the Cargo layering (rhi_vulkan cannot name boyko_ecs) constrains it.
5. Owner values or scope calls: per-player ActionState as a component (local multiplayer), and Window as an entity/component (multi-window). Both are Resources today because v1 has exactly one of each.
6. Who owns wiring the UI to the windowed screen? render_gbuffer_frame has no ui parameter (J:crates/boyko_rhi_vulkan/src/present/frame_driver.rs:821-836), boyko_app names no boyko_ui on J or U, and a layering rule forbids boyko_ui and boyko_render from naming each other outside tests (M playground.rs:55-57). The render and UI lenses must decide.
7. Align the provisional kernel-feature names (external-event-writer, per-type-event-swap-policy, resource-change-ticks, ordered-startup-stages, ordered-nonsend-teardown) with the physics study's names. Its feature list was not available to this lens.
8. The native capture list (J:crates/boyko_rhi_vulkan/src/window.rs:651-654) has no WM_CHAR, WM_SETFOCUS/KILLFOCUS or WM_MOUSELEAVE, so Text, WindowFocus and CursorLeft never arrive. Is this in scope for the UI text-field work?
9. Correct the ledger row rhi.verified.json present/surface.rs:238 from 'F perframe' to per-swapchain-(re)create (the callers are in Swapchain::build via new/recreate).

---

# Lens 4 of 6: world-services

# World services as one ECS system: data, logic and side structures

Scope: `boyko_scene`, the kernel asset system, `boyko_serialize` and the kernel serialize seam, `boyko_log`, `boyko_diag` and the profiler (kernel store plus host reducers), time and state machines, the kernel services (events, commands, hooks/observers, relations, change detection, EnableTag), and `boyko_reflect`. Read-only work: no cargo, no rustc, no benchmarks. Nothing below was run; every claim comes from a source read or a grep.

**Trees, checked with `git log -1` and `git branch --show-current`:**
- **J** = `D:/wt/joltab` on `merge/ke16-into-ecsnative` @ `d11962a9`.
- **U** = `D:/wt/ui` on `feat/ui-advanced` @ `615cda8f`.
- **R** = `D:/wt/reflect` on `feat/reflection` @ `0e0b4c68`.
- **M** = `D:/claude/BoykoEngine`, read only for `docs/memory/ALLOCATOR-DESIGN-SPACE.md`.

**Ledger.** I read `ecs-services`, `physics-scene-math`, `pool-utils-log`, `codec-tools` and `app-demo` (`*.verified.json`). They were taken at J `ca582e72`. That commit is an ancestor of `d11962a9`, one commit behind, and `git diff --stat ca582e72 d11962a9` over every path in this lens is empty. So the ledger rows for this lens are current. Its `boyko_ui` rows are not: they come from J, not U. For UI I read U directly and only for the hot-reload service.

---

## 1. Summary

1. **The kernel event system has no production user in any engine crate.** `EventWriter|EventReader|send_event` gives 0 hits in the `src/` of `boyko_scene`, `boyko_render`, `boyko_physics`, `boyko_input` and `boyko_app` on J, and 0 in `boyko_ui/src` on U. Instead, subsystems pass messages through their own `Vec` queues inside a Resource:
   - `RefcountDeltas`, `DeferredFree`, `TransformPropagationScratch.detached` on J;
   - `RawInputQueue` (a `Box<[RawInputEvent]>` ring).

   There are four structural reasons (section 2). The strongest: events are bounded and lossy (`J:crates/boyko_ecs/src/ecs/core/events/event_config.rs:19` `/// Maximum events per lane per frame before \`EventBufferFull\` is returned.`), and hooks cannot send events directly.

2. **`Assets<T>` is a parallel data system inside the kernel.**
   - It is a second entity allocator: `Handle<T>` is `index: u32` + `generation: u32`, which is `Entity`'s shape.
   - It is a second dense store; its own doc says it reuses "the identical recipe `DenseStore` already ships".
   - It is a second change-detection system (`dirty_gen`, `install_epoch`, `free_epoch`, a `dirty` bitmap).
   - It is a second reference-lifetime system (a `refcount` column fed by hook→`Vec` deltas).

   Its natural form is **assets as entities**:
   - the value becomes a dense component (live slots never move, so it can index the GPU tables);
   - load state becomes component presence;
   - the refcount becomes a relation;
   - dirtiness becomes kernel ticks.

3. **Entity→asset references are raw indices, not relations.** `MeshHandle(pub u32)` / `MaterialHandle(pub u16)` (`J:crates/boyko_scene/src/render_caps.rs:143,158`). Four hooks push ±1 into a `Vec` that a render system drains. The user-level Relations API has 0 users outside the kernel's own hierarchy.

4. **Enable toggles are tickless.** `J:crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:79-80` `/// Enables the flag \`T\` on \`entity\` (Decision D3). O(1) warm: no migration,` / `/// no structural-generation bump, no hook / observer fire, no deferred`. So every consumer that must react rolls its own dirty flag:
   - lights (`LightTableDirty`);
   - the profiler, which can only project at the next fold;
   - `visibility_sync`, which needs a custom command.

   The enable state is also absent from all three world-service seams: serialize, prefab and reflect (section 3.C).

5. **Several loops run outside the scheduler.**
   - Justified: the profiler fold, the frame driver (time, event swap, fixed loop) and the fence-gated asset retire drain. Each has a stated ordering reason.
   - Not justified by a structural necessity: the boot-only asset uploads via `run_system`, and the GPU zone reducer. That reducer is a second profiler store with `Vec<f64>` per zone, kept as a local variable of the runner.

6. **One subsystem runs a private asset service.** UI hot reload is a UI-owned mtime poll on U. The kernel has no asset hot reload: a grep for `hot reload|reload|file_watch` over J `crates/` finds hits only in `boyko_ui`.

7. **Some parts are already exemplary and should be preserved:**
   - `ProfilingScope` is an entity with a table capability and an EnableTag on/off, projected once per fold, with no mirror.
   - `LogRing` is resource-owned `VmColumn`s.
   - `state_chart!` compiles onto `State<S>` + events + systems.
   - `Transform`/`GlobalTransform`, `Visibility`/`RenderEnabled` and `ChildOf`/`Children` are native ECS forms.

---

## 2. Are the kernel services used uniformly?

Line counts from `grep -rEc` over each crate's `src/`, comments included, so treat them as coarse. Scene, render, physics, input and app are on J; UI is on U.

| service | scene | render | physics | input | app | ui (U) |
|---|---|---|---|---|---|---|
| events (`EventWriter\|EventReader\|send_event`) | 0 | 0 | 0 | 0 | 0 | 0 |
| user-level relations (`derive(Relationship` / `impl Relationship for` / `RelationshipTarget for`) | 0 | 0 | 0 | 0 | 0 | 0 |
| hooks/observers (`on_insert =`, `observe_on_`…) | 4 | 1 | 0 | 0 | 0 | 0 |
| `Changed<`/`Added<` | 9 | 20 | 7 | 0 | 0 | 60 |
| EnableTag (`bitset`/`Enabled<`/`.enable::<`) | 8 | 64 | 21 | 0 | 0 | 16 |
| `dirty` (hand-rolled flags or generations) | 51 | 104 | 3 | 0 | 12 | — |

Why subsystems skip the event system (each point from source):
- **Bounded and lossy:** see the `event_config.rs:19` quote in section 1. The deferred path swallows the failure: `J:crates/boyko_ecs/src/ecs/core/commands/send_event_command.rs:10-11` `//! Failures from the inner \`send_event\` call (an unregistered event type` / `//! or a full lane) are swallowed`. A dropped refcount `-1` is a leak, so `RefcountDeltas` cannot ride this path.
- **Hooks and observers have no `send_event`.** The `DeferredEcsMaster` surface is `get_component` (`deferred_master.rs:83`), `resource`/`resource_mut` (`:95`, `:105`), `commands` (`:150`) and entity commands. So the hooks push into Resource `Vec`s: `J:crates/boyko_scene/src/render_caps.rs:285` `if let Some(deltas) = dm.resource_mut::<RefcountDeltas>() {`.
- **Lifetime is at most two frames.** The swap runs in the frame driver: `J:crates/boyko_ecs/src/ecs/core/app/app.rs:718` `self.world.update_events();`. `DeferredFree` needs retention measured in fence epochs, so it is not event-shaped at all.
- **Ceremony.** `J:crates/boyko_ecs/src/ecs/core/events/event.rs:20-23` requires `type Participants` + `type Parameters`.

Other uniformity findings:
- **A duplicate TypeId registry.** `J:crates/boyko_ecs/src/ecs/core/asset/backing.rs:87` `static ASSET_LAYOUTS: OnceLock<Mutex<HashMap<TypeId, ComponentId>>> = OnceLock::new();` sits beside the kernel's lock-free `TypeIntern`, which was built for exactly this case: `J:crates/boyko_utils/src/type_intern/mod.rs:17` `//! Per Principle 0 the capability belongs in the kernel once, not as four per-crate adapters:`.
- **A duplicate string interner.** `J:crates/boyko_scene/src/identity.rs:81` `static INTERNER: OnceLock<Mutex<InternerState>> = OnceLock::new();` sits beside the kernel's `J:crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs:155` `static TAG_NAMES: OnceLock<Mutex<HashMap<Box<str>, TagId>>> = OnceLock::new();`.
- **A duplicate system tick horizon** (section 3.A, `last_run`).

---

## 3. Inventory by area

### A. `boyko_scene` (J)

| datum | type / owner | form today | element | lifetime / scope | writer → reader | pointer held across a grow? |
|---|---|---|---|---|---|---|
| `Transform` | component | component (40 B, `transform.rs:58`) | POD | persistent, per entity | gameplay, `fly/orbit_camera_system` → `propagate_transforms` | no |
| `GlobalTransform` | component | component | `Affine3A` | persistent, per entity | `propagate_transforms` only (`transform.rs:15` `the propagation pass is its sole writer.`) → camera, render pack, lights | no; read by copy, `propagation.rs:543` `let global = unsafe { *(raw as *const GlobalTransform) };` |
| `TransformPropagationScratch.stack` | Resource field | `Vec<(Entity,u32)>` (`propagation.rs:120`) | Copy | per frame, global | propagate only | no; `mem::take` then put back (`:250`, `:381`) |
| `.dirty` | Resource field | `Vec<Entity>` (`:124`) | Copy | per frame | `collect_dirty` → seeding | no |
| `.detached` | Resource field | `Vec<Entity>` (`:133`) | Copy | per event, until next run | `child_of_on_remove` observer (`:661` `scratch.detached.push(ctx.entity);`) → propagate | no |
| `.last_run` | Resource field | `Tick` (`:146`) | — | persistent | propagate | — |
| `.detach_observer_installed` | Resource field | `OnceLock<()>` (`:151`) | — | per world | `ensure_detach_observer` | — |
| `Camera.is_active` | component field | `bool` flag (`camera.rs:113` `pub is_active: bool,`) | — | per entity | user → `resolve_active_camera` | — |
| `ActiveCamera` | Resource | `Option<Entity>` (`camera.rs:240`) | — | global | user → `resolve_active_camera` | — |
| `ViewUniform` | Resource | singleton (`camera.rs:253`) | GPU POD | per frame, global | `resolve_active_camera` → renderer | — |
| `Name` | component | `u32` id (`identity.rs:56`) | Copy | per entity | spawn → readers | — |
| name interner | process static | `Mutex<HashMap<&'static str,u32>>` + `Vec<&'static str>` + `Box::leak` (`identity.rs:72-81`, `:121`) | str | process | `intern`/`resolve` (cold) | the leaked strs are immortal |
| `MeshHandle` / `MaterialHandle` | component | raw `u32` / `u16` index (`render_caps.rs:143`, `:158`) | Copy | per entity | user → pack (`gpu3d_system.rs:46` reads `&MaterialHandle`) | — |
| `MeshRefGen` / `MaterialRefGen` | component | `u32` generation lane (`render_caps.rs:197`, `:209`) | Copy | per entity | `apply_refcount_deltas` → `validate_asset_refs` | — |
| `Visibility` | component | `u8` (`render_caps.rs:226`) | Copy | per entity | user → `visibility_sync` | — |
| `RenderEnabled` | enable-state | bitset (`render_caps.rs:250` `#[component(storage = "bitset")]`) | bit | per entity | `visibility_sync` command → pack filter | — |
| `RefcountDeltas.deltas` | Resource field | `Vec<RefDelta>` (`asset_refs.rs:99`) | Copy | per frame | four hooks → `apply_refcount_deltas` | no |
| `DeferredFree.entries` | Resource field | `Vec<FreeEntry>` (`asset_refs.rs:149`) | Copy | multi-frame (fence) | `apply_refcount_deltas` → `retire_deferred_frees` | no |
| `WindowHost.retire_scratch` | host struct field | `Vec<FreeEntry>` (`J:crates/boyko_app/src/host.rs:96`) | Copy | per frame | `drain_ready` → retire loop | no |
| `FixedSet` / `CameraSet` | set enums | compile-time | — | — | — | — |

**Where the logic runs:**
- `propagate_transforms` is an exclusive system in `Main`, in `CameraSet::Resolve` (`camera_plugin.rs:69` `let propagate = b.add_system(propagate_transforms).in_set(CameraSet::Resolve).key();`).
- `resolve_active_camera` and `visibility_sync` are systems ordered `.after(propagate)` (`camera_plugin.rs:70-73`).
- The detach observer runs in the apply window.
- Everything in `boyko_scene` is on the scheduler.

**Side structures and kernel gaps found here:**
- `last_run` duplicates `SystemMeta::last_run`, because an exclusive body cannot see its own horizon. `J:crates/boyko_ecs/src/ecs/core/system/exclusive_function_system.rs:14-15` `//! 1. **Signature** — the body is \`FnMut(&mut EcsMaster) -> ()\`, not a` / `//!    \`SystemParamFunction\`. There is no param tuple, no per-param state,`.
- `detached` is a hand-rolled removal log. The kernel has no removal detection: a grep for `RemovedComponents|Removed<` over `boyko_ecs/src` returns 0.
- The dirty scan is O(spatial entities) because `read_changed_tick` is `pub(crate)` (`propagation.rs:40-41` `requires a public` / `per-archetype changed-tick-column accessor on the kernel`).
- **A stale claim in `visibility_sync`.** `J:crates/boyko_scene/src/visibility_sync.rs:47-48` says `yields \`(EntityId, _)\`; there is no \`QueryData for Entity\` and no` / `world-resolving \`SystemParam\`)`. J has had that param since `01a4436e` (2026-08-30): `J:crates/boyko_ecs/src/ecs/core/system/params/entities.rs:1` `//! \`Entities<'w>\` — read-only \`SystemParam\` resolving an [\`EntityId\`] to a`. So the custom `SetRenderEnabledById` (`visibility_sync.rs:84`) is avoidable: use `Entities` + `EntityCommands::disable` (`entity_commands.rs:236`).

### B. The kernel asset system (J), with its render and app glue

| datum | form today | lifetime / scope | writer → reader | notes |
|---|---|---|---|---|
| `Assets<T>.col` | standalone `ComponentPool` in a Resource or NonSendResource (`assets.rs:201`) | per asset | `add`/`fill` → `get`/`get_by_index` (render tables) | GPU tables index it by row (`get_by_index`, `high_water`) |
| `.slot_word` | `VmColumn<u32>` {generation, state} (`:202`) | per asset | `reserve`/`fill`/`fail`/`retire` | the generation duplicates `Entity`'s |
| `.refcount` | `VmColumn<u32>` (`:203`) | per asset | `inc_ref`/`dec_ref` from `apply_refcount_deltas` | the lifetime duplicates relations |
| `.live` / `.pinned` / `.dirty` | `LiveBitmap` (a `Vec<u64>` inside) (`:204`, `:206-207`) | per asset | internal | duplicates dense `live` and change ticks |
| `.free` | `Vec<u32>` (`:205`) | per asset | internal | duplicates the entity free list |
| `dirty_gen`, `free_epoch`, `install_epoch` | `u64` (`:209-211`) | global per `T` | `get_mut` (`:502` `self.dirty_gen += 1;`) → `material_table.rs`, `tlas.rs:251`, `asset_refcount.rs:406` | table-wide, hand-rolled change detection |
| `AssetStaging<A>.queue` | `Vec<Staged<A>>` in a NonSendResource (`staging.rs:58`) | per asset, until upload | `AssetServer::load` → `upload_*_assets` | per `staging.rs:53`, the `Vec` is a placeholder for an MPSC channel |
| `AssetPaths<A>.index` | `PathIndex` (`VmColumn`) in a NonSendResource (`paths.rs:55`) | persistent | `load` | — |
| `AssetServer` | zero-sized Resource (`server.rs:56`) | — | — | `load` is a plain call |
| `ASSET_LAYOUTS` | `Mutex<HashMap<TypeId,ComponentId>>` (`backing.rs:87`) | process | `Assets::with_reserved` | see section 2 |

**Where the logic runs:**
- `upload_material_assets`, `upload_mesh_assets` and `upload_texture_assets` are **boot one-shots run from the runner**, not scheduled systems: `J:crates/boyko_app/src/runner.rs:835` `app.world_mut().run_system(upload_material_assets);`, and `:830` `a BOOT ONE-SHOT, not a per-frame system`.
- `retire_deferred_frees` is a **host step outside the scheduler**, placed after the fence wait: `runner.rs:1264-1268` `retire_deferred_frees(` / `app.world_mut(),` / `ctx,` / `host.renderer.submission_epoch(),` / `&mut host.retire_scratch,`. That placement is justified: it must follow `wait_frame_in_flight` (`:1258-1263`), which is not a schedule point.
- `apply_refcount_deltas` and `validate_asset_refs` are scheduled systems (`AssetRefcountPlugin`, `plugins.rs:439`).
- **Nothing calls `AssetServer::load` in production** (`runner.rs:327-329` `No scene calls \`load\``).

**Stale docs:**
- `J:crates/boyko_scene/src/asset_refs.rs:143-144` `\`Assets::dec_ref\`-returned \`RetireTicket\`; nothing drains it until F6's` / `fence-gated \`retire_deferred_frees\` lands.` That function exists and runs every frame (`runner.rs:1264`).
- `J:crates/boyko_ecs/src/ecs/core/asset/mod.rs:38-39` `rungs stores only a 16-bit index (no generation)`. `MeshHandle` is a `u32` and generation lanes exist (`render_caps.rs:197`).

### C. Serialize (kernel seam and `boyko_serialize`, J)

- **Logic.** `save_world(world: &EcsMaster, …, out: &mut Vec<u8>)` (`save.rs:159-162`) and `load_world` are cold calls outside the scheduler. That is appropriate for a whole-world snapshot.
- **Transients (build once, one call's lifetime):**
  - `ColumnPlan.via_fn_bytes` / `ArchetypePlan.columns` / `DenseStorePlan.{s2e,data}` (`save.rs:74`, `:84`, `:112`, `:116`);
  - `LoadEntityMap.entries: Vec<(u64, Entity)>` (`J:crates/boyko_ecs/src/ecs/core/serialize/mod.rs:288`);
  - `SaveCursor.out: &mut Vec<u8>` (`mod.rs:81`).
- **Pointers across a grow.** Pass 1 captures column base pointers and Pass 2 reuses them. This is sound because no structural operation runs between the passes (`save.rs:14-17`).
- **Enable state is not persisted.** Save skips any id without a pool: `save.rs:190-192` `let pool = match archetype.component_pools().get_pool(component_id) {` / `Some(p) => p,` / `None => continue,`. Bitset tags have no pool.
- **Prefab drops the same memberships.** `J:crates/boyko_ecs/src/ecs/core/clone/prefab.rs:43` `//! * Dense / bitset (EnableTag) memberships are **not** captured`.
- **Reflect lacks the same view.** `R:crates/boyko_reflect/src/ecs.rs:296` `// ── source 3: bitset presence — EG3 ──…` is deliberately absent.

The enable state is second-class in all three world services.

- **`Prefab` is a frozen mini-world outside the ECS.** `prefab.rs:281` `nodes: Vec<PrefabNode>,`, `:284` `components: Vec<PrefabComponent>,`, `:287` `blob: RawBlob,` (realloc on grow, addressed by offsets, not pointers).

### D. Log (`boyko_log` below the kernel, plus the kernel seam; J)

- **Producer substrate: `.bss` statics, no allocator.**
  - `lane.rs:174` `static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = …`
  - `target.rs:265` `static CONTROL: [AtomicU8; MAX_TARGETS] = …`
  - `rate.rs:73` `static RATE`, `once_sites.rs:67` `static SITES`
  - `sink/ecs.rs:181` `static ECS_HANDOFF: HandoffRing = HandoffRing::new();`

  Written from any thread, the panic hook and the sink thread, before any world exists.
- **Kernel seam, already ECS-native.** `LogRing { lines: VmColumn<LogLine>, arena: VmColumn<u8>, … }` (`J:crates/boyko_ecs/src/ecs/core/log/ring.rs:103-107`) and `LogStats` are Resources.
- **Logic.** `log_drain_system` is a scheduled system in `Main`, in `LogSet` (`log/plugin.rs:56` `b.add_system(log_drain_system).in_set(LogSet);`).
- **Target-level authoring.** Levels are set by the host from an env parse (`J:crates/boyko_app/src/plugins.rs:249` `boyko_log::target::set_target_level(id, level);`). This could adopt the profiling-scope projection pattern (section E) so that a console or menu toggles an entity.

### E. Diag and profiler (J)

- **Producer substrate: `.bss` statics.**
  - `J:crates/boyko_diag/src/sample.rs:337` `static LANES: [ZoneLane; LANE_COUNT as usize] = …`
  - `profiling_abi.rs:146` `static ARM_MASK: ArmMask = …`
  - `profiling_abi.rs:291` `static REGISTRY: [core::sync::atomic::AtomicPtr<ZoneDesc>; ZONE_ID_SPACE] =`
  - `loss.rs:403` `static CELLS`
- **Store.** `Profiler` is a Resource over a process-lifetime reservation that is deliberately leaked: `J:crates/boyko_ecs/src/ecs/core/profiling/store.rs:19-20` `…then deliberately \`mem::forget\`-ed, so *"never freed"* is` / `structural rather than asserted. **This is the one deliberate leak in the engine**`. **Worker lanes hold pointers into it forever** (`:14-17`).
- **Logic.** The fold runs outside the schedule, by design: `J:crates/boyko_ecs/src/ecs/core/profiling/plugin.rs:20-21` `/// The fold is **not** a system. It runs at the top of \`App::update_with_delta\`, which is the` / `/// single funnel…`. The call site is `app.rs:690` `crate::ecs::core::profiling::fold_frame(&mut self.world);`.
- **Exemplary.** Scope on/off is an entity + EnableTag, projected by a query: `J:crates/boyko_ecs/src/ecs/core/profiling/ecs_control.rs:264-265` `for scope in world` / `.query::<&ProfilingScope, Enabled<ProfilingScopeEnabled>>()`. Its own doc also records the tickless-toggle gap (`:24-28`).
- **Side structure: a second store for GPU zones.**
  - `J:crates/boyko_app/src/profiling/reduce.rs:59` `dur_ns: Vec<f64>,` (with `:61`, `:63`), owned by a runner-local `vb_zone_reducer` (`runner.rs:1139-1140` `let mut vb_zone_reducer = … WindowReducer::new(`).
  - The kernel store admits it: `ecs_control.rs:290` `the host's GPU channel folds` / `into the artifact reducer, not into this \`Profiler\``.
- **`TelemetryStream` has no production caller.** Its `.bss` `BUFFERS` (`stream.rs:117`) and process-static counters (`:120-135`) are exercised only by `boyko_app/tests/profiling_telemetry_stream.rs`. The stated reason for staying off the World, `flush_on_panic`, does not exist (`stream.rs:34` `MEASURED while writing this: **\`flush_on_panic\` does not exist in the tree.**`).

### F. Time and state (J)

- **Time.** `Time` and `FixedTime` are Resources (`time.rs:38`, `fixed_time.rs:38`). The frame driver advances them outside the schedule: `app.rs:698` `self.world.resource_mut::<Time>().advance_with(raw);` and `:730` `let steps = fixed_advance(&mut self.world, |w| {`. That is kernel frame-driver work, correctly outside.
- **State.** `State<S>` and `NextState<S>` are Resources (`next_state.rs:19-25`). `StateTransitionRecord<S>` holds `transition: Option<Transition<S>>` (`transition_record.rs:62`). It is a one-slot, per-frame message, i.e. an event already in its degenerate form: `next_state.rs:30` `/// Calling \`set\` twice in one frame keeps only the final value`.
- **`state_chart!` is ECS-native** (`J:crates/boyko_macros/src/state_chart/mod.rs:3-6` `…one system per leaf, and` / `` `run_if(in_state(leaf))` registrations… `` / `no parallel data structure`). No `StateScoped`-style cleanup exists (0 grep hits).

### G. Kernel service internals (J)

- **The ECS's own storage.** `CommandQueue.bytes`, `EventDispatcher.slots/lanes`, `Resources.slots`, observer stores, the hook tables and the `EnableStore` pages are all kernel-internal. The allocator campaign already schedules them (M `docs/memory/ALLOCATOR-DESIGN-SPACE.md:332-336`).
- **`Children(Vec<Entity>)`** (`hierarchy/mod.rs:120`) is a relation. Its collection removal is O(n): `J:crates/boyko_ecs/src/ecs/core/relationship/collection.rs:100` `if let Some(idx) = self.as_slice().iter().position(|&c| c == e) {`.
- **Relation suppress flags** (`relationship/mod.rs:90`, `:152`) are thread_locals. They are correct as kernel-internal state.

### H. `boyko_reflect` (R)

- **Registry.** `R:crates/boyko_reflect/src/registry.rs:29-30` `static REFLECT: [OnceLock<&'static TypeInfo>; MAX_COMPONENTS] =` / `[const { OnceLock::new() }; MAX_COMPONENTS];`. It is the sibling of the kernel's `LAYOUTS`/`HOOKS`/`SERIALIZE` tables: process-global and written by derive registration before any world exists.
- **Descriptors** are `&'static` values baked at compile time.
- **The ECS glue** is `&EcsMaster`-only and zero-alloc (`ecs.rs:10-11`). No side store.

### I. UI hot reload (U, as evidence of an asset-service gap)

- `U:crates/boyko_ui/src/reload/state.rs:106` `pub struct UiHotReload {` holds `last_mtime: Option<SystemTime>`, `pending`, `last_poll` and `poll_interval` (`:112-120`).
- The watch is an exclusive system that polls `std::fs::metadata` (`reload/system.rs:46`, `:123`). Its exclusivity rationale is `reload/system.rs:4-5` `…READ live components (no entity-yielding \`Query\` on` / `this engine)`. That is true on U, which lacks `params/entities.rs`, and stale against J.

---

## 4. Side structures ("glued on the side")

1. `Assets<T>`: a second allocator, dense store, change-detection system and lifetime system (`assets.rs:200-216`).
2. `RefcountDeltas` + four hooks: a hook→system queue beside both events and relations (`asset_refs.rs:98-100`, `render_caps.rs:278-349`).
3. `DeferredFree` + `WindowHost.retire_scratch`: per-asset state kept in a global queue plus a host-parked `Vec`.
4. `TransformPropagationScratch.detached` (a removal log) and `.last_run` (a system-horizon mirror).
5. `Prefab`: a frozen off-world entity image that drops dense and enable memberships.
6. The runner-local `WindowReducer`: a second profiler store for GPU zones.
7. The name `INTERNER` beside `TAG_NAMES`; `ASSET_LAYOUTS` beside `TypeIntern`.
8. `SetRenderEnabledById` (a workaround for a gap that no longer exists) and `SetLightEnabledById` + `LightTableDirty` (a workaround for tickless enable). The render one is cross-evidence only: `J:crates/boyko_render/src/light_system.rs:942-943` `/// Like the immediate path, it marks [\`LightTableDirty\`] in the same apply because the bit` / `/// flip is tickless (Decision 2).`
9. `UiHotReload` (U): a subsystem-private asset watch.
10. `RawInputQueue` `Box<[RawInputEvent]>` (`J:crates/boyko_input/src/raw/queue.rs:35`): another hand-rolled event ring. Input lens; cross-evidence only.

---

## 5. Kernel features the forms need

Names follow the allocator design (M `ALLOCATOR-DESIGN-SPACE.md:88-94`: FrameArena, HeapVec, DropColumn) and the ledger (`public id-free VmColumn`, StrInterner). Wherever the physics study hits the same need (a removal log, an exclusive-system horizon, id-free scratch), the name should be shared.

| # | feature | evidence that the plain forms fail | users in this lens |
|---|---|---|---|
| KF1 | **Removal detection** (`Removed<T>` filter or a per-component removal log) | 0 hits; propagation rolls its own (`propagation.rs:125-133`) | propagation detach, asset `Retiring`, render mirrors |
| KF2 | **Change tick on enable toggle** (a `Toggled<T>` filter) | `enable_tag_api.rs:79-80`; `ecs_control.rs:24-28`; `light_system.rs:942-943` | `visibility_sync`→pack, lights, profiling projection |
| KF3 | **Tick horizon for exclusive systems** (and `Local` for exclusive bodies) | `exclusive_function_system.rs:14-15`; `propagation.rs:134-146` | propagate, UI reload, retire |
| KF4 | **Count-only relation target collection** | `collection.rs:100` O(n) remove × thousands of instances per mesh | entity→asset relation |
| KF5 | **Lossless event lane + `send_event` from hooks/observers** (needed only if a hook→system queue survives KF4 and the asset-entity move) | `event_config.rs:19`; `send_event_command.rs:10-11`; `deferred_master.rs:83-150` | leftover hook queues |
| KF6 | **StrInterner** (one kernel interner) | `identity.rs:72-81` vs `tags.rs:155` | `Name`, tag names, log dynamic targets |
| KF7 | **Default query exclusion** (so prefab templates can live in the world) | `prefab.rs:22-29`, `:43`; only `Disabled<T>` exists (`filter_enable.rs:327`) | `Prefab` |
| KF8 | **Kernel asset hot reload** (watch → change on the asset entity) | reload exists only in `boyko_ui` (grep) | UI `.ui` documents, meshes, textures |
| KF9 | **EnableStore bits in the serialize, prefab and reflect seams** | `save.rs:190-192`; `prefab.rs:43`; `R ecs.rs:296` | save/load, prefab, inspector |
| KF10 | **Public id-free scratch column / FrameArena** (the ledger's "public id-free typed column"; `ScratchColumn` today needs a registered `ComponentId`) | ledger NP; `ScratchColumn` is `ComponentPool`-backed (`scratch/mod.rs:3`) | propagate scratch, retire scratch, save/load plans |
| KF11 | **"Never compact" contract for GPU-indexed dense stores** (a policy, not a type) | `dense_store.rs:6-7` `live slots never move`; `compact()` exists (`:607`) | asset dense components |

---

## 6. Final table

Forms are picked from the closed vocabulary. Where the form is not "component", the "¬X" notes say why each earlier form fails.

| datum or loop | today | natural ECS form | kernel feature | cost argument for the hot path it touches |
|---|---|---|---|---|
| `Transform`, `GlobalTransform`, `Visibility`, `Name` (id) | component | **component** | — | unchanged |
| `RenderEnabled`, `ProfilingScopeEnabled` | enable-state | **enable-state** | KF2, so readers stop polling | unchanged |
| `propagate_transforms` | exclusive system, `Main` / `CameraSet::Resolve` | ECS system (stays exclusive: same column, different rows) | KF3; public changed-tick column for a streaming scan | the streaming scan is O(archetypes + changed) instead of today's O(spatial entities) — strictly cheaper |
| `stack`, `dirty` | `Vec` in a Resource | **system-scratch** (¬component/dense: not durable per entity; ¬enable/relation: not a boolean or an edge; ¬event: not a message; ¬resource-column: cleared every run) | KF10 | the same contiguous push/pop; no regression |
| `detached` | observer → `Vec` | **event** — a removal message (¬component: the row no longer has `ChildOf`; ¬enable: nothing to toggle; ¬relation: the edge is gone) | KF1 | written only on a detach; zero cost otherwise, the same as the observer path today |
| `last_run`, `detach_observer_installed` | Resource fields | **kernel-internal** (`SystemMeta`) | KF3 | none |
| `Camera.is_active` | `bool` in a component | **enable-state** (¬component: the capability ruling forbids a flag inside a component; ¬dense: no bulk buffer) | — | a handful of cameras; the `Enabled<>` filter is branch-free |
| `ActiveCamera` | `Option<Entity>` Resource | **relation**: an Exclusive 1:1 edge from a view/target entity to its camera (¬component/enable: cannot express "at most one" without a policy fallback) | a view entity (none exists; interim: keep the Resource) | one lookup per frame |
| `ViewUniform` | Resource | **component** on each camera (a derived per-camera datum; makes multi-view expressible) | — | one extra entity read per frame for the active view |
| name interner | process `Mutex<HashMap>` | **kernel-internal** (pre-world, cross-world, cold) | KF6 | setup only |
| `MeshHandle` / `MaterialHandle` | raw `u32`/`u16` + 4 hooks | **relation** instance→asset entity (¬component: a bare FK has no reverse index and no despawn hook, which is exactly what the refcount needs); keep a derived `u32`/`u16` row lane as a component | asset entities, KF4 | the pack still reads the `u32`/`u16` lane (`gpu3d_system.rs:46`): **unchanged**; link/unlink O(1) with KF4 (with a `Vec` collection it would be O(refs), a regression) |
| `MeshRefGen` / `MaterialRefGen` + `validate_asset_refs` | generation lanes + a per-frame net | removed; the asset `Entity`'s generation is the check | asset entities | removes one per-frame system |
| `RefcountDeltas` | `Vec` in a Resource | removed by the relation (else **event**, which needs KF5) | KF4 (else KF5) | removes one per-frame drain |
| `Assets<T>.col` | standalone pool in a Resource | **dense-component** on the asset entity (¬component: table rows move on swap-remove, and the GPU tables index by row) | KF11 | `get_by_index` becomes a dense slot index: the same O(1) |
| `slot_word` generation / `free` / `live` / `live_count` / `high_water` | `VmColumn`s, `Vec`, bitmaps | **kernel-internal** (entity allocator + dense bookkeeping) | — | the same work, done once instead of twice |
| `slot_word` state (Loading/Failed/Retiring) | packed bits | **component** presence: no `T` = loading; `AssetFailed` marker; `Retiring{retire_frame}` | — | cold transitions |
| `refcount` | `VmColumn<u32>` | **relation** (reverse-collection count) | KF4 | O(1) |
| `pinned` | bitmap | **component** (ZST marker; a permanent capability, so not a toggle) | — | none |
| `dirty` / `dirty_gen` / `install_epoch` / `free_epoch` | table-wide counters | **kernel-internal** change detection: `Changed<T>` on dense slots (dense stores keep ticks, `dense_store.rs:258-259`) | KF1 | the GPU mirror gates become `Changed<T>` queries; per-row, so no coarser |
| `AssetStaging<A>` | `Vec` in a NonSendResource | **component** `Staged<A::Cpu>` on the asset entity (per-asset, removed on upload) | — (single-thread consumer) | cold load path |
| `AssetPaths<A>` | `PathIndex` NonSendResource | **component** (path hash) + **resource-column** hash→`Entity` index | — | load-time only |
| `DeferredFree` | `Vec` in a Resource | **component** `Retiring{retire_frame}` (¬enable: carries data; ¬relation/event: multi-frame per-asset state) | KF1 | the drain becomes a query over the few `Retiring` rows; zero on the golden path |
| `retire_deferred_frees` | host step after the fence | **loop outside the scheduler (justified)**: it must follow `wait_frame_in_flight` | — | unchanged |
| `retire_scratch` | host `Vec` | **system-scratch** | KF10 | unchanged |
| `upload_*_assets` | boot one-shot via `run_system` | ECS systems in an upload set (startup + per frame once `load` is live) | — | an empty-queue check per frame |
| device handles inside `MeshGpu`/`TextureGpu` | inside `Assets<T>` rows | **out-of-scope: driver-owned** handles, kept in a NonSend **resource-column** indexed by dense slot | — | the same indexing |
| `ASSET_LAYOUTS` | `Mutex<HashMap>` | **kernel-internal** via `TypeIntern` | — (already exists) | lock-free lookup |
| `save_world`/`load_world` + their plans, `LoadEntityMap` | cold calls, `Vec`s | calls stay; transients become **system-scratch** | KF10, KF9 | cold |
| `Prefab` | an off-world image | **new-kernel-feature: world-resident prefab entities** (¬component: template entities would match every live query without an exclusion mechanism; ¬enable: `Disabled<T>` is opt-in per query, not a default) | KF7 | instantiate becomes `clone_subtree` (which re-materializes dense data) |
| log lanes / `CONTROL` / rate / once / handoff statics | `.bss` | **kernel-internal** (pre-world, any thread, panic path; one `.bss` load per site) | — | the emission site must stay one load + one branch |
| `LogRing`, `LogStats`, `log_drain_system` | resource-column + system | **resource-column** / ECS system | — | unchanged |
| log target levels | env → `set_target_level` | **component** on a log-target entity, projected into `CONTROL` once per frame (the `ProfilingScope` pattern) | — | the emission site is unchanged; one projection per frame |
| diag lanes, `REGISTRY`, `ARM_MASK`, `CLOCK`, loss cells | `.bss` | **kernel-internal** | — | unchanged |
| `Profiler` + fold at the top of `update_with_delta` | Resource over a leaked reservation; loop outside the schedule | **resource-column** + **loop outside the scheduler (justified**: the instrument stays outside its own number) | — | unchanged |
| GPU `WindowReducer` (`Vec<f64>` per zone) | runner local | **resource-column**: the `Profiler`'s GPU channel | — | off-frame, bench only |
| `TelemetryStream` buffers / counters | `.bss` + process statics | **kernel-internal**; the counters could move into the `Profiler` drop counters | — | no production caller today |
| `Time`, `FixedTime`; the frame driver | Resources; outside the schedule | **resource-column** (singletons) + **kernel-internal** driver | — | unchanged |
| `State<S>`, `NextState<S>` | Resources | **resource-column** | — | unchanged |
| `StateTransitionRecord<S>` | `Option` in a Resource | **event** (single slot; already the degenerate lane, at most one per frame per S) | — | a run condition reads one load; keep this storage |
| `Children`, relation flags | `Vec<Entity>`, thread_local | **relation** (collection on `HeapVec`/EntityListPool) / **kernel-internal** | — | O(1) push, swap-remove |
| `REFLECT` table (R) | static `[OnceLock<&TypeInfo>; MAX_COMPONENTS]` | **kernel-internal** (¬component: no entity per component type; ¬resource-column: must be readable without a world); `TypeInfo` is **out-of-scope: compile-time** | KF9 (source 3) | editor-only, cold |
| `UiHotReload` + `ui_hot_reload_system` (U) | UI Resource + exclusive poll | **component** watch state on the UI-document asset entity + a kernel watch system | KF8, KF3 | one throttled `metadata()` per interval, as today |
| `STILL_FRAME_COMPOSES`, `FixedSet`/`CameraSet`, `HasLoaders::LOADERS` | debug static / enums / const table | **out-of-scope: test-only / compile-time** | — | — |

Outside-engine comparisons were not fetched. The resemblance to flecs components-as-entities and prefabs, and to Bevy's `RemovedComponents` and assets-as-entities, is labelled **not verified** and none of the forms above depend on it.

## Open list (lens world-services)

1. Assets-as-entities changes the public Handle<T> API and every Assets<T> consumer in boyko_render/boyko_app. It is an architecture fork: should the architect take it, or is the owner to confirm?
2. Name persistence is not verified: no SerPod impl for NameId was found. If Name demotes to SerializeViaFn with serialize_fn None (the S1 boundary, J:crates/boyko_serialize/src/save.rs:21-29), Name is saved as a zero-length region. If it is saved raw, the u32 id is meaningless in another process (interner is process-global, J:crates/boyko_scene/src/identity.rs:81).
3. ViewUniform as a per-camera component (multi-view) versus keeping the singleton Resource: this depends on whether split-screen or multi-view is in scope.
4. Should the MeshGpu/TextureGpu device-handle table stay a NonSend resource-column indexed by dense slot, or should the kernel gain NonSend component access? The Component trait has no Send bound (J:crates/boyko_ecs/src/ecs/core/component/component.rs:37).
5. Is a world-resident prefab with default query exclusion (KF7) wanted, or is the off-world Prefab accepted as a cold template exception? Today it silently drops dense and EnableTag memberships (J:crates/boyko_ecs/src/ecs/core/clone/prefab.rs:43).
6. KF5 (a lossless event lane plus send_event from hooks) is needed only if any hook-to-system queue survives the relation move. Should it be dropped from the list once KF4 and assets-as-entities are accepted?
7. Kernel feature names (removal detection, exclusive tick horizon, id-free scratch column) must be reconciled with the parallel physics study's names. No physics study output was found in the scratchpad.
8. TelemetryStream has no production caller on J (only boyko_app/tests/profiling_telemetry_stream.rs). Is it intended to be wired, or deleted?
9. Stale in-tree claims to repair (not fixed here): J:crates/boyko_scene/src/visibility_sync.rs:47-48 (the Entities param exists since 01a4436e); J:crates/boyko_scene/src/asset_refs.rs:143-144 ('nothing drains it'); J:crates/boyko_ecs/src/ecs/core/asset/mod.rs:38-39 ('16-bit index (no generation)'); U:crates/boyko_ui/src/reload/system.rs:4-5 (true on U, stale against J).

---

# Lens 5 of 6: plans

# Lens plans: what the repository has already decided about the non-physics subsystems as ECS

## 0. Scope, trees, method, caveats

- **Trees read.** J = `D:/wt/joltab` @ `d11962a9` (kernel, render, RHI, app, input, scene, log, diag). U = `D:/wt/ui` @ `615cda8f`: all UI plans and UI code. R = `D:/wt/reflect` @ `0e0b4c68`: one decision. M = `D:/claude/BoykoEngine` for docs/animation, docs/render/TRANSPARENCY-* and REFLECTIONS-*, docs/memory/ALLOCATOR-*, and docs/physics/ADVANCED-* (read only for kernel-request names).
- **Rules followed.** Read-only. No cargo, no benchmarks. graphify was tried once and returned an off-target subgraph, so I fell back to Grep/Read.
- **Ledger.** The scratchpad ledger holds 12 `*.verified.json` files and 2 `*.ecsform.json` files, taken at `ca582e72`, which is J HEAD~1 (`git rev-list --count ca582e72..d11962a9` = 1). Its boyko_ui rows predate U's 16 commits.
  - Heap-typed fields added on U since merge-base `5ec1699f`, which the ledger cannot contain:
    - U:crates/boyko_ui/src/sprite.rs:257 `sheets: Vec<UiSheet>,`
    - U:crates/boyko_ui/src/animation.rs:473 `done: Vec<(EntityId, ComponentId)>,`
    - U:crates/boyko_render/src/ui/gather.rs:284 `pub roots: Vec<Entity>,` (and :286, :289)
    - U:crates/boyko_render/src/ui/upload.rs:203 `node_buf: Vec<UiNode>,` (and :198 `staging: Box<[UiInstance]>`, :206)
    - U:crates/boyko_render/src/ui/pack.rs:808 `pub keys: Vec<(u32, u32)>,`
  - The render ECS-form pass already exists in `render.ecsform.json`, with 7 kernel features and 6 plan conflicts. I cite it where it overlaps and do not redo it.

---

## 1. Decision register

Columns: what was decided · who decided it (owner ruling / plan / measurement) · status · quote.

### 1.1 Kernel and cross-cutting (the rungs any "everything else" design inherits)

| # | Decision | By | Status | Citation |
|---|---|---|---|---|
| K1 | Scope of the principle. Durable world data goes in ECS storage: per-entity data in `ComponentPool`, non-entity bulk in a Resource that owns a `ComponentPool`. | Audit plan, owner-mandated | Partly executed | J:docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md:6 "`ComponentPool` (per-entity columns) or `Resource`-owned `ComponentPool` host columns (non-entity bulk) — not a parallel `std::Vec`/`HashMap` keyed by a subsystem-minted id." · J:docs/OPEN-QUESTIONS.md:5059 "**STAGES 0 AND 1' RAN; STAGE 4 DID NOT.**" |
| K2 | A bespoke column primitive is rejected; `ComponentPool` is the primitive. | Owner question, answered by the audit | Shipped as `ScratchColumn` | ARCH-AUDIT:19 "A bespoke column re-fragments the engine into two data systems and re-opens the SP4 gap. For non-entity bulk: a `Resource` that OWNS a `ComponentPool` host column." · J:crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:43 "pub struct ScratchColumn<T: Copy> {" |
| K3 | Stage 0 planned a registry-free, synthetic-id scratch column with uncommitted tick regions. | Plan | **Diverged.** The shipped constructor needs a registered id. Untracked ticks landed on J only (commit `0cb082bb`). M has no `UNTRACKED` in the file. | ARCH-AUDIT:27 "`ComponentPool::new_scratch(layout, reserve_rows)` (synthetic-id, registry-free, tick sub-regions reserved-uncommitted)" · scratch_column.rs:88 "pub fn new(component_id: ComponentId, reserve_rows: usize) -> Self {" · :15-16 "UNTRACKED mode so the two tick sub-regions are reserved but never committed." |
| K4 | Legit-keep list. | Audit | Stale in one item: J boyko_input has no `HashMap` today (grep finds only doc prose). | ARCH-AUDIT:47 "FFI/GPU/OS-contiguity buffers; input flat-buffer Resources; SoftBody inner Vecs (L2 exception); the one input HashMap (rust#22991-forced)." |
| K5 | Dense storage is a third `StorageKind` holding one global column. It is always CPU-resident. | Plan | D0–D4 shipped; Stage P (physics) not | J:docs/DENSE-COMPONENTS-PLAN.md:6 "Non-goals: relationships; paged SparseMap; GPU-resident dense (dense is ALWAYS `ResidencyKind::Cpu`)." |
| K6 | Dense × enable: `dense_iter*` refuses an enable-bearing filter at compile time; SnapInterpolation moves to an EnableTag. | Owner directive (foundation-first) + plan | Shipped | J:docs/DENSE-ENABLE-QUERY-PLAN.md:86 "`const { assert!(!F::CONTAINS_ENABLE_TERM, …) }`" · J:crates/boyko_ecs/src/ecs/core/iters/query/query.rs:120 "pub const fn assert_dense_iter_no_enable" · J:crates/boyko_render/src/snap_interpolation.rs:76 `#[component(storage = "bitset")]` |
| K7 | Capability model: presence is Axis 1, the EnableTag bit is Axis 2, enum metadata is never the gate. | **Owner-approved convention** | Normative. Conformance: Render conforms, GUI conforms, Lighting planned. | J:docs/CAPABILITY-STATE-MODEL.md:3 "**Status:** owner-approved convention (this session). Normative for EVERY subsystem" · :67 "`LightEnabled` bit … **PLANNED (Axis-2 gap)**" · :102-103 "hierarchical `InheritedVisibility` propagation + `IslandSleep` off the `Resource` side-store = deferred later phases." |
| K8 | `par_iter` refuses dense storage. The owner lifted R-DENSE but sequenced the kernel fix after physics Stage 4. | Owner rulings AB-7 and 2026-09-09 | KE15 unowned | J:docs/OPEN-QUESTIONS.md:363 "Owner: *"that there is no parallelism is just wrong."*" · :5223 "**`par_iter`'s dense rejection waits for Stage 4 to finish.**" · J:docs/aether-v2/KERNEL-BACKLOG.md:54 "Give the parallel chunk runner a world cell." |
| K9 | `Or` dense blindness (KE1) is fixed on J. | Aether R0 | **Landed on J only.** Commit `01a4436e` is not an ancestor of U; U's `impl_or_filter_tuple` mentions dense nowhere. | KERNEL-BACKLOG:40 "✅ **LANDED (R0, 2026-08-29)**" · J:crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:1885 "const HAS_DENSE: bool = false $( \|\| $F::HAS_DENSE )*;" |
| K10 | KE13: `QueryView::get` ignores a dense `With`/`Without`. | R0 census | Unowned; red tests exist but are `#[ignore]`d | KERNEL-BACKLOG:52 "`get` returns `Some` for a non-member and **disagrees with `iter()` on the same view**" |
| K11 | `VmColumn` is not public. | KE12, deferred | Shipped as `pub(crate)` | KERNEL-BACKLOG:51 "*(deferred, measure first)* `VmColumn` promotion to pub" · J:crates/boyko_ecs/src/ecs/memory/vm_column.rs:80 "pub(crate) struct VmColumn<T: Copy> {" |
| K12 | GPU residency: a `PoolBacking` enum, `NonSendResource`, GPU-resident archetypes must be GPU-pure, and CPU systems may not touch GPU bytes. | Plan (Phase 4/5) | Shipped (Phase 5 archived COMPLETED) | J:docs/RENDER-PHYSICS-GPU-PLAN.md:91 "**Forbidden: a CPU system touching GPU-resident bytes**" · :159 "`ComponentPool` gains a `PoolBacking` ENUM" · J:docs/archive/PHASE5-GPUCOLUMN-PLAN.md:283 "tighten to **`saw_gpu && saw_non_gpu`**" · J:crates/boyko_ecs/src/ecs/memory/component_pool.rs:93 "Device(Box<DeviceColumn>)," |
| K13 | No mechanical gate exists on `Vec` as a durable side store. | Owner question | **Open** | OPEN-QUESTIONS:5090-5094 "**Should `Vec`-as-a-durable-side-store get a mechanical gate?** … a census over `Resource`-held structs" |

### 1.2 Render (J; transparency and reflections from M)

| # | Decision | By | Status | Citation |
|---|---|---|---|---|
| R1 | `GpuColumnManager` keeps a `Vec` side table of column metadata. | Phase 5 plan | Shipped. Audit Stage 5 (fold it into `PoolBacking::Device`) is planned and not executed. | PHASE5:28-29 "Side map `GpuColumnMeta` (SoA `Vec` indexed by `Slot.index`, NOT a HashMap)." · J:crates/boyko_render/src/gpu_column.rs:532 "meta: Vec<GpuColumnMeta>," · ARCH-AUDIT:31 "fold `GpuColumnManager.meta` into per-column archetype metadata (`PoolBacking::Device` arm). Cold; lowest urgency." |
| R2 | Frame graph: a NonSend resource. Passes are **not** entities. | Plan D1, critic-confirmed | Shipped (Pillars A and B). **The as-built substrate is `Vec`, not `VmReservation`.** | J:docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md:161-165 "owning **index-based SoA arenas on one `VmReservation`** … Passes are NOT ECS entities (transient, rebuilt every frame, no entity identity, walked backward — wrong access pattern for archetype iteration)" · :95-96 "**D-A2 Substrate = build-time preallocated `Vec`s** … not `VmReservation` (which stays `pub(crate)` in `boyko_ecs`)" |
| R3 | Interpolation pair `GpuTransform3D` is a dense component. `Gpu3dInstance` is the dense column that also serves as the GPU buffer. | Plan | Shipped | FRAME-GRAPH:244 "to a kernel **dense non-fragmenting component** `GpuTransform3D { curr, prev }`" · J:docs/archive/STD-LIB-PLAN.md:353 "`Gpu3dInstance` **is** the dense component column AND the GPU vertex/instance buffer (no parallel `std::Vec` mirror)" |
| R4 | Public render graph: `Box<dyn RenderPassNode>` nodes in a World NonSend registry. Pool and `res_table` stay host-side. | Converged plan | Not verified as shipped: `struct RenderNodeRegistry` has no match in J's crates | J:docs/RENDER-GRAPH-API-PLAN.md:40 "registered ONCE at setup as `Box<dyn RenderPassNode>`" · :189-190 "a **World NonSend resource** `RenderNodeRegistry`" · :199 "Pool / `res_table` / facade handle tables stay host-side (WindowHost)." |
| R5 | Render config lives in Resources. The VB geometry store is the device side of `Assets<MeshGpu>`. | Plan | Shipped | J:docs/MULTI-PARADIGM-RENDER-PLAN.md:27 "config = ECS Resources; all GPU data stays in existing VM-native mirrors. The C1 geometry store is **not a side table**" |
| R6 | Particles are not entities; emitters are. Particle state lives only on the GPU, with ScratchColumn staging. | Plan, grounded on a measurement | Shipped (P0 onward) | J:crates/boyko_render/src/particle.rs:5-6 "20k spawns/frame through `Commands` is 0.6–2 ms of CPU against ~2 µs of GPU emit, so a particle is never an `Entity`" · J:docs/PARTICLES-PLAN.md:223 "per-particle state is a GPU-contiguity buffer (the sanctioned exception) with **no CPU mirror**. CPU staging is `ScratchColumn`, never `std::Vec`." |
| R7 | DDGI: probe classification is a declared GPU buffer, not a host `Vec<bool>`. | Plan | Shipped | J:docs/RENDER-SDFDDGI-PLAN.md:82-84 "a dedicated GPU classification buffer (1 byte/probe), a declared FFI-GPU exception like the atlas — NOT a host `std::Vec<bool>`" |
| R8 | Acceleration structures are RHI-owned and derived from ECS columns. | Plan | Partly shipped (HW-RT behind a feature) | J:docs/RENDER-HYBRID-RAY-SYSTEM-DESIGN.md:127 "the AS is an **RHI-owned GPU resource derived from ECS columns**" |
| R9 | Mesh draw scratch is on ScratchColumn lanes. | Code | Shipped on J. MEMORY-SYSTEM-AUDIT:133 still describes it as "Vec<u32> ×3 lanes", which is stale. | J:crates/boyko_render/src/mesh_draw.rs:446 "counts: ScratchColumn::new(u32_id, u32_rows)," |
| R10 | Frame-transient scratch rebuilt from components is **OK-by-design** as `Vec`. | Memory audit | Standing verdict | J:docs/MEMORY-SYSTEM-AUDIT.md:152 "frame-transient clear+refill scratch rebuilt from ECS components (BroadphaseGrid CSR, MeshRenderScratch, UiRenderScratch, UiWorldScratch, TransformPropagationScratch, FrameGraph arenas)" |
| R11 | Transparency: ZST markers rather than EnableTags; ScratchColumn gather lanes; a separate `MaterialXGpu` SSBO; UI text outside the frame graph. | M architect design, critiqued once | Design | M:docs/render/TRANSPARENCY-DESIGN-SPACE.md:124-125 "Every lane is a `ScratchColumn` … — a `ComponentPool`, never a `Vec`)" · :73 "Both are ordinary components, not `EnableTag`s" · :902 "UI / MSDF text stays outside the frame graph" |
| R12 | Reflections: environments are append-only asset columns; probes are components that become light-table rows; atlases use the FFI exception; a key array is rejected. | M design, pass 2 | Design | M:docs/render/REFLECTIONS-DESIGN-SPACE.md:37-41 "**Data** is ECS-native throughout … atlases are the FFI/GPU-contiguity exception class" · :147-148 "a Resource-owned key array was considered and rejected as a second store for data the table already holds" · :154-155 "owned by a `ReflectionTargets` struct in `present/targets.rs` beside `HzbTargets`." |

### 1.3 UI (U; plans dated 2026-08-21)

Commits on U show S0–S6 and A0–A1 shipped. The interaction and Aether plans are pre-implementation.

| # | Decision | By | Status | Citation |
|---|---|---|---|---|
| U1 | UI crate rule: props and outputs are ECS columns; per-frame scratch is a Vec in a Resource. | Catalogue | Shipped | J:docs/SYSTEMS.md:2202-2203 "No parallel data system — props/outputs are ECS columns, per-frame scratch is a `Resource`-owned buffer (reset every frame)." |
| U2 | Animation sink `UiVisual` is a table component. The four `Tween*` channels are dense, and presence means "running". | Plan AD10 + a measurement | Shipped (A1) | U:docs/UI-ADVANCED-ARCHITECTURE.md:854-855 "**`UiVisual` is a TABLE component; the four `Tween*` stay dense, because nothing filters them**" · U:docs/UI-PLAN-ANIMATION-DECISIONS.md:653-656 "a dense sink … is seen by a bare `Changed` (1 row) and **not** by the discovery filter's `Or` (0 rows)" |
| U3 | Rejected: a per-element animator holding a `Vec` of tracks. Held in reserve: a Resource-owned arena. | Plan | Design rule | UI-ADVANCED:963-964 "a `Vec` of tracks inside a component is a per-element parallel data system" · :968-971 "Principle 0 permits it explicitly … It is **held in reserve**" |
| U4 | Animation state is ECS data, not a shader clock. | Plan D8b | Shipped (flipbook) | UI-ADVANCED:632 "**D8b — animation state is ECS data, not a shader clock.**" |
| U5 | Sheet table: "Resource-owned dense column". | Plan | **Shipped as a `Vec` inside a Resource** (the FontTable precedent) | UI-ADVANCED:643 "One `Resource`-owned dense column keyed by a dense `u16 sheet_id`" · U:crates/boyko_ui/src/sprite.rs:257 "sheets: Vec<UiSheet>," · U:crates/boyko_ui/src/text/font.rs:139 "fonts: Vec<FontEntry>," |
| U6 | Sprites use the shared bindless table; a second table is refused. | Plan D2/D3 | Shipped (S2–S3) | UI-ADVANCED:232 "a second table would itself be the Principle-0 violation the research names" |
| U7 | The canonical gather lives in `boyko_render`, which gains a production dependency on `boyko-ui`. | Plan D31 | Shipped (U gather.rs) | UI-ADVANCED:514-516 "the canonical gather ships as `boyko_render::ui::gather_ui_nodes`" |
| U8 | Drag payload is the dragged entity. A `HashMap` side store is refused. | Plan D18 | Plan | UI-ADVANCED:1304-1306 "a `Box<dyn Any>` or a `HashMap<DragId, Payload>` side store is the Principle-0 violation" |
| U9 | Text editing mutates `UiTextBuffer` in place; the char stream is a fixed-array Resource. | Plan C-I3 / ID9 | Plan | U:docs/UI-PLAN-INTERACTION.md:194-195 "a subsystem-local data model glued on the side, which is the Principle-0 violation this campaign exists to avoid" · :383-384 "`UiTextInputQueue` (a `Resource`: `chars: [char; 64]`, `len: u8`, `dropped: u16`)" |
| U10 | Dynamic lists use `UiRepeat` with an entity template. | Plan D29 | Deferred | UI-ADVANCED:1714-1715 "the template is a **disabled prefab subtree that is itself an entity**, never a `Vec<Node>`" |
| U11 | No runtime style cascade. | Plan D28 | Plan | UI-ADVANCED:1696-1697 "**Refused:** a USS-style runtime selector cascade (a parallel matcher over the tree — Principle 0)" |
| U12 | Timelines: clip data in a Resource-owned channel-major table; the player is a per-element POD. Multi-line text would use a Resource rope column. | Plan | Deferred | UI-ADVANCED:1211-1213 "The clip is a `Resource`-owned immutable contiguous keyframe table" · :1789 "Multi-line wants a `Resource`-owned rope column" |
| U13 | A dense member of the pack-input list is a build error. | Plan AD13 | Owed to the seam rung | UI-PLAN-ANIMATION-DECISIONS:789-790 "the wrong storage kind becomes `error[E0080]` at the site that chose it" |
| U14 | Kernel gaps the UI routed around: no `Removed<C>` filter; dense suppresses the single-component Bundle impl; `#[require(dense)]` panics. | Measurement | Worked around | UI-PLAN-INTERACTION:205 "**There is no `Removed<C>` anywhere in `boyko_ecs`.**" · U:docs/UI-PLAN-ANIMATION-A1.md:45-46 "dense storage SUPPRESSES the single-component `Bundle` impl" · :50-51 "**`#[require(<a dense type>)]` PANICS at insert**" |
| U15 | `.ui` migrates to the Gaia `ui` profile, and the old format is deleted. | **Owner ruling F6** | Planned (Gaia G7) | J:docs/OPEN-QUESTIONS.md:392 "**Migrate `.ui` to the Gaia `ui` profile and DELETE the old format in the same campaign.**" |

### 1.4 Input, app/host, assets/Gaia, diagnostics, editor, reflection

| # | Decision | By | Status | Citation |
|---|---|---|---|---|
| I1 | Action state is arrays indexed by action id inside Resources (`Box<[f32]>`, one heap allocation at setup). | Plan | Shipped | J:docs/archive/INPUT-SYSTEM-PLAN.md:268 "**One-time setup heap** for the four value arrays (`Box<[f32]>` / `Box<[[f32;2]]>`) is allocated in `InputPlugin::build` (cold path), never per frame" |
| A1 | Host state stays outside the World; the device is a leaked singleton. | Converged plan | Shipped | J:docs/APP-HOST-PLAN.md:327-328 "`WindowHost` never enters the World or crosses threads" · J:crates/boyko_app/src/host.rs:96 "pub(crate) retire_scratch: Vec<FreeEntry>," |
| S1 | Asset store: VM-native, resource-owned, keyed by dense slot id; assets are **not** entities, and components carry the ids. | Owner-approved direction | Design locked 2026-07-10; rungs partly shipped | J:docs/ASSET-STREAMING-PLAN.md:42 "**Shared store (S1):** VM-native, resource-owned, keyed by a dense slot id." · :67 "free:       Vec<u32>,        // LIFO free-list (DenseStore-sanctioned)" |
| S2 | Teardown queues for `!Send` GPU objects are `Vec`s. | Plan | Shipped per plan (code not re-read) | ASSET:377 "old buffers ride a NEW `RetiredGpuBuffers` (`!Send Vec` teardown queue mirroring `OrphanedMeshGpu`" |
| S3 | Gaia data-table rows **are entities** (Table storage). | Delegated ruling F2 | Ruled; critique open | OPEN-QUESTIONS:396 "**(a) rows are entities — AMENDED**: `StorageKind::Table` (not dense)" · :299-300 "twenty small tables of 50 rows × 40 B cost **~5 MiB resident for 40 KB of payload (~128× overhead)**" |
| S4 | GK-1 (a world-owned `LoadEntityMap`) violates Principle 0; the named fix is a `VmColumn` path index. | Adversarial pass | Recorded, not repaired | OPEN-QUESTIONS:319-321 "converts the exception into exactly the durable parallel data system Principle 0 forbids. **The in-tree answer is one line away** … `PathIndex { entries: VmColumn<PathEntry>`" |
| S5 | Streaming ships everything from the start. | **Owner ruling F5** | Planned | OPEN-QUESTIONS:391 "**EVERYTHING FROM THE START — option (b)**" |
| L1 | Diagnostics: tables with a compile-time extent live in `.bss`; run-time extents use `VmReservation`; `CONTROL` cannot be an ECS column. | Plan (S12) | Shipped | J:docs/LOGGING-SYSTEM-PLAN.md:130 "**Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒ `VmReservation`**" · :136 "`CONTROL` cannot be an ECS column at all (there is provably no `World` before `boot()`, inside a driver callback, or inside a panic hook)" |
| L2 | The profiler's durable store is a Resource on VM storage; the transport rings are kernel-internal. | Plan | Shipped | J:docs/PROFILING-SYSTEM-PLAN.md:121 "**the durable store is an ECS `Resource`, on kernel VM-native storage**" |
| E1 | Editor selection is a Resource-owned dense array, not components; a parallel document model is rejected. | Design rev 4 | Design | J:docs/editor/EDITOR-BOUNDARY.md:182-183 "a `Resource`-owned dense array of `StableId`s — Principle 0 permits a `Resource`-owned column, and this is one" · J:docs/editor/EDITOR-DOCUMENT.md:214 "it is a parallel data system — Principle 0 forbids it" |
| RF1 | Reflection descriptors are `&'static` in a `[OnceLock; MAX_COMPONENTS]` table. | Plan | Shipped on R | R:docs/REFLECTION-PLAN-CORE.md:159 "`REFLECT` is `[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]`" |

### 1.5 Animation and allocator (M)

| # | Decision | By | Status | Citation |
|---|---|---|---|---|
| AN1 | Pose bank is a resource-owned ScratchColumn, not a DenseStore. `AnimInstance` is dense; state the machine touches uses table components. | M design, after critic pass 1 | Design | M:docs/animation/ANIMATION-DESIGN-SPACE.md:175-177 "a `DenseStore` maps one entity to one fixed-stride slot … and a bone run per entity is a shape the kernel does not have" · :197-198 "the R-DENSE refusal there (`:103`) is exactly why the machine-facing state is NOT dense." |
| AN2 | Bones are never entities. | Design D13 | Design | :620 "Bones are never entities at runtime (no "exposed skeleton")" |
| AN3 | Hand-reserved id band for animation's scratch columns, and a 64 B clip-blob chunk to dodge tick pages. | RUNG-1 plan | Plan | M:docs/animation/RUNG-1-PLAN.md:147 "**Animation takes `460..=477`**" · :192-195 "**`grow_rows` commits all three sub-regions in lockstep**" · :206 "**Decision: `ClipBlobChunk = [u8; 64]`**" |
| M1 | Allocator: replace each type by lifetime class, using a new `boyko_memory` crate. Heap-class `HeapVec` relocates on growth. UI frame lanes go to `FrameVec`; `Children`, `SparseMap` and render lists go to `HeapVec`. | Architect rev 1 | **CHANGES REQUESTED**: blockers C1 (Tree Borrows aliasing) and C2 (`ScopeArena` collision) | M:docs/memory/ALLOCATOR-DESIGN-SPACE.md:70 "The new `HeapVec<T>` **relocates on growth** exactly like `Vec`" · :84 "`boyko_memory` — new crate containing `vm.rs`, `vm_column.rs`" · :322 "UI frame lanes … `FrameVec` on the `FrameArena`" · :318 "`Children(HeapVec<Entity>)`" · :413 "**Verdict of pass 1: CHANGES REQUESTED**" |

---

## 2. Conflicts between plans

**C-1. The UI's storage rulings rest on a kernel defect that J has since fixed.**
- On U, AD10 reason 1, AD13 and S-D16 all stand on `Or` being blind to dense storage (UI-PLAN-ANIMATION-DECISIONS:653-656, :789-790).
- J fixed `Or` in `01a4436e` (filter.rs:1885). That commit is not an ancestor of U.
- Gaia had the same kind of ban and deleted it on the same fact. See KERNEL-BACKLOG:40, GB-9: "the ban is DELETED with a record".
- When U is merged onto J:
  - AD10 reason 1 becomes false. Reasons 2–4 still stand: the crate edge, the precedent, and one migration per element.
  - AD13's build error would then forbid a shape that has become legal.
  - The `ui_render_discovery` behaviour changes silently.

**C-2. Whether the memory reservation may be exposed.** Three plans justified keeping data outside VM or ECS storage because `VmReservation` is `pub(crate)`:
- Frame graph as built: FRAME-GRAPH:95-96.
- Logging: LOGGING:134 "a std-only zero-dep `boyko_diag` cannot host it".
- Profiling: PROFILING:119.

Three other positions exist on the same fact:
- KE12 defers making `VmColumn` public (KERNEL-BACKLOG:51).
- The allocator design lifts it into `boyko_memory` (ALLOCATOR:84).
- The allocator critique wants it sealed (ALLOCATOR:461 "mark them `#[doc(hidden)]` or seal them").

Additionally, the ledger's render pass refutes the idea of a public `VmColumn` in favour of resource-owned ScratchColumns. Whichever position wins either removes or keeps the "forced" reason behind the first three plans.

**C-3. `GpuColumnManager.meta`.**
- The memory audit calls it OK-by-design (MEMORY-SYSTEM-AUDIT:124).
- Audit Stage 5 says fold it into `PoolBacking::Device` (ARCH-AUDIT:31).
- The code still holds a `Vec` (gpu_column.rs:532).
- The ledger follows Stage 5.

**C-4. Four positions on UI/render per-frame scratch.**
1. A Vec in a Resource is OK-by-design (SYSTEMS.md:2203; MEMORY-SYSTEM-AUDIT:137, :152).
2. Use `FrameVec` on a `FrameArena` (ALLOCATOR:322).
3. Use ScratchColumn lanes. Mesh draw already does this on J (mesh_draw.rs:446), and transparency mandates it: "a `ComponentPool`, never a `Vec`" (TRANSPARENCY:124-125).
4. The ledger's ECS form: `UiInstance` as a dense component on the widget, plus a ScratchColumn key lane.

As a result, render itself now runs two conventions side by side: mesh draw uses ScratchColumns (J) while UI pack/upload use `Vec` (U pack.rs:808, upload.rs:203).

**C-5. The allocator's Heap class against Principle 0 and the audit's C4.**
- The audit says the real win is address stability: ARCH-AUDIT:38 "the real win is ADDRESS-STABILITY-on-grow".
- The allocator moves `Children`, `SparseMap` and render lists onto `HeapVec`, which relocates (ALLOCATOR:70, :318, :323, :337).
- Its `DropColumn` and the ledger's `KF-owning-scratch-column` are the same capability in two homes (ledger plan_conflict 6).

**C-6. Animation's reason for table components is weaker than it was.**
- It keeps machine-facing state in table components because of R-DENSE (ANIMATION:197-198).
- The owner lifted R-DENSE (OPEN-QUESTIONS:363). Only the parallel half is still missing, and it waits on KE15.

**C-7. Animation RUNG-1 §2.2 is based on the M tree's state.**
- The 64 B clip-chunk fork exists to avoid 8 B/row of tick pages (RUNG-1:192-206).
- On J, ScratchColumn is untracked and its tick regions are never committed (scratch_column.rs:15-16, commit `0cb082bb`, absent from M).
- On J the premise is false. Cache-line alignment of clip data is the remaining argument.

**C-8. The id-band scheme was not what Stage 0 planned.**
- Stage 0 promised a registry-free synthetic id (ARCH-AUDIT:27).
- The shipped ScratchColumn needs a registered `ComponentId` (scratch_column.rs:88).
- So each crate hand-reserves a band out of the shared 512-id budget: physics 478–511, animation 460–477 (RUNG-1:147).
- No kernel registry manages these bands.

**C-9. Dense is CPU-only, which contradicts the CLAUDE.md framing.**
- Dense storage is always CPU-resident (DENSE-COMPONENTS:6), and GPU-resident archetypes must be GPU-pure (PHASE5:283).
- CLAUDE.md principle 0 names dense components for "GPU instances".
- So every GPU instance datum exists twice: once in a CPU dense column, once in an upload ring or SSBO. No plan covers GPU-resident dense storage.

**C-10. Asset teardown queues.**
- The asset plan chose `!Send` `Vec` queues because `BoundBuffer` is `!Send` (ASSET:327, :377).
- The ledger routes them to Retiring rows of `Assets<T>` and to an owning resource column.

**C-11. The sheet table's wording and its code disagree.**
- The plan says "Resource-owned dense column" (UI-ADVANCED:643).
- The code is a `Vec` in a Resource (sprite.rs:257).
- The audit's form would be a Resource that owns a `ComponentPool` (ARCH-AUDIT:19).

**C-12. Five names for one shape: a variable-length span per owner.**
- Animation pose bank: ANIMATION:14-19.
- Physics D-8 row bank: M:docs/physics/ADVANCED-PHYSICS-DESIGN-SPACE.md:808-809 "a resource-owned ROW BANK with per-body ranges".
- Ledger `RaggedColumn` (physics-scene-math), `CsrColumn` (ui-input), and `VmJagged` (ecs-schedule).
- KR-3 and AK-2 already ask for one ratified primitive (ADVANCED-PHYSICS:1144 "= animation AK-2").

---

## 3. Where plans keep runtime data outside ECS forms on purpose, and their stated reasons

Each reason below is one the design must either answer or overturn with evidence.

| Site | Form | Stated reason | Standing under the owner's orders |
|---|---|---|---|
| Frame-graph passes (FRAME-GRAPH:164-165) | NonSend arena, `Vec` as built | "transient, rebuilt every frame, no entity identity, walked backward — wrong access pattern for archetype iteration"; the `Vec` choice because `VmReservation` is `pub(crate)` | The access-pattern argument concerns entities, not storage. The pub(crate) argument dissolves if C-2 resolves toward a public reservation. |
| `WindowHost`, and the render-graph pool / `res_table` / facade tables (APP-HOST:327-328; RENDER-GRAPH-API:199) | Host struct outside the World | Runner-thread confinement, drop-order teardown before the device is destroyed, and the Renderer is a local of `run_windowed` (RENDER-GRAPH-API:184 "`Renderer` is a local of `run_windowed`") | Order (4) covers "ALL runtime things". The design must say whether NonSend Resources plus an ordered-eviction contract can carry this; the plan's own eviction discipline already exists (APP-HOST:103-115). |
| Particles (particle.rs:5-6) | GPU-only buffer; emitters are entities | A measurement: 0.6–2 ms of CPU for 20k spawns | Measured; the same number backs physics D-8. It stands unless the spawn path is re-measured. |
| Logging `CONTROL` and other `.bss` tables (LOGGING:130, :136) | Statics | No World exists before boot, in a driver callback, or in a panic hook; extents are compile-time constants | This is a structural reason. Open: whether `.bss` counts as "our system" under order (2). |
| Driver allocations (ALLOCATOR:27 `ffi-driver`) | Foreign | "routing them would force a thread-safe heap for a foreign caller" | Matches the narrowing that memory the OS or driver owns stays outside. |
| GPU atlases and probe buffers (SDFDDGI:82-84; REFLECTIONS:154-155) | RHI-owned | GPU contiguity | The brief's narrowing: staging is library-ownable; only the driver-owned VRAM stays out. |
| Asset store (ASSET:42) | Resource-owned `ComponentPool`, not entities | Assets are shared by id; components carry the id | This is the audit's own "Resource owns a ComponentPool" form. |
| Asset teardown queues (ASSET:377) | `!Send` `Vec` | `BoundBuffer` is `!Send` | The ledger proposes an owning NonSend resource column. |
| UI frame scratch (SYSTEMS.md:2203; MEMORY-SYSTEM-AUDIT:152) | `Vec` in a Resource | Clear-and-refill, zero allocation, rebuilt from components | See C-4. The audit's reason is address stability, which does not matter if no pointer outlives a push. |
| UI sheet and font tables (sprite.rs:257, font.rs:139) | `Vec` in a Resource | "Setup-time alloc; never grows in-frame" | The rule is shape-based, not growth-based; no reason is given against a `ComponentPool`. |
| UI Model C arena (UI-ADVANCED:968-971) and editor `Selection` (EDITOR-BOUNDARY:182-183) | Resource-owned column | "Principle 0 permits"; for the editor: selecting would migrate the entity and change save-file block order | Order (3) asks for things to be "as close as possible" to the ECS paradigm. The design must say whether a Resource-owned column is the ECS form or a fallback. The editor's migration and serialization reason is concrete. |
| Animation pose bank (ANIMATION:175-177) | Resource-owned ScratchColumn bank | "a bone run per entity is a shape the kernel does not have" | A kernel gap, shared with physics D-8. This is the case for a first-class kernel feature (C-12). |
| Render GPU column metadata (PHASE5:28-29) | Side `Vec` | A resolve cost of about 3 ns; no HashMap | Stage 5 already overturns it on paper. |
| Input value arrays (INPUT:268) | `Box<[f32]>` in a Resource | A closed enum sized once at setup | This is the "Resource-owned bulk" form, with a `Box` backing. |
| Reflection descriptors (REFLECTION-PLAN-CORE:159) | Static | Per-type metadata | Kernel-internal. The ledger's macros pass reaches the same verdict. |

---

## 4. Kernel features the lens plans already name (to align names with the physics study)

1. **ScratchColumn ratified as the resource-owned column for durable data.** AK-2 (ANIMATION:569) is the same item as KR-3 (ADVANCED-PHYSICS:1144). Its non-`Copy` twin: the ledger's `KF-owning-scratch-column` and the allocator's `DropColumn`.
2. **Per-owner span bank.** C-12 lists five names for it today.
3. **KE15: give the parallel chunk runner a world cell**, for dense storage and `Related` joins (KERNEL-BACKLOG:54). This blocks dense storage for render and UI in parallel systems as much as for physics.
4. **KE13, KE14, KE11**: dense point lookups and dense `#[require]`.
5. **Tick regions left uncommitted for scratch.** Shipped on J (`0cb082bb`).
6. **Visibility of `VmColumn` / `VmReservation`**: KE12 vs `boyko_memory` (C-2).
7. **EnableTag initial polarity**: the ledger's `KF-enable-initial-polarity`, citing J light.rs:244.
8. **Entity datum in `Query`** (KR-1), and the parallel entity-yielding chunk walk (KE9).
9. **In-scope barrier after KE16**: AK-9 is the same item as KR-5. Ruled to wait for KE16 to close (OPEN-QUESTIONS:5212).
10. **Two gaps no plan has requested yet:** a `Removed<C>` filter (UI worked around it with an observer plus a generation counter), and a kernel-managed registry for synthetic id bands (C-8).

## Open list (lens plans)

1. OWNER: Under orders (3)/(4), does a Resource-owned column count as an ECS form or only a fallback? Five plans say 'Principle 0 permits a Resource-owned column/arena': UI Model C (U:UI-ADVANCED-ARCHITECTURE.md:968-971), the editor Selection (J:EDITOR-BOUNDARY.md:182-183), the UI sheet/font tables, the asset store (J:ASSET-STREAMING-PLAN.md:42), and input's action arrays.
2. OWNER/ORCHESTRATOR: U's AD10 reason 1, AD13 and S-D16 stand on Or-dense blindness, which J fixed in 01a4436e (J filter.rs:1885). These must be re-argued when U is merged onto J; the Gaia precedent GB-9 deleted its ban on the same fact.
3. ARCHITECTURE: Resolve VmReservation/VmColumn visibility: KE12 deferral vs the boyko_memory crate (ALLOCATOR:84) vs sealing (ALLOCATOR:461 O6) vs ScratchColumn-only (ledger). The frame-graph Vec substrate (FRAME-GRAPH:95-96) and the logging/profiling .bss statics cite this visibility as their forced reason.
4. ARCHITECTURE: Pick one of four positions for UI/render per-frame scratch: Vec in a Resource (SYSTEMS.md:2203), FrameVec (ALLOCATOR:322), ScratchColumn lanes (J mesh_draw.rs:446; TRANSPARENCY:124), or a dense component on the widget (ledger). Render already runs two of these side by side.
5. ARCHITECTURE: Name ONE kernel primitive for per-owner variable-length spans, shared with the physics study (anim pose bank, physics D-8 row bank, ledger RaggedColumn / CsrColumn / VmJagged; AK-2 = KR-3).
6. ARCHITECTURE: Synthetic ScratchColumn ids are hand-reserved per crate out of the 512 budget (physics 478-511, anim 460-477). Stage 0 promised a registry-free id (ARCH-AUDIT:27). Decide whether a kernel-managed band registry is needed.
7. OWNER: Does order (4) extend to WindowHost and the render-graph pool/res_table, which are deliberately outside the World (APP-HOST-PLAN.md:327-328; RENDER-GRAPH-API-PLAN.md:199)? Does order (2) cover compile-time-extent .bss statics (LOGGING-SYSTEM-PLAN.md:130)?
8. OWNER: Particles-not-entities rests on a measurement (J particle.rs:5-6: 0.6-2 ms per 20k spawns). Is it accepted as an exception under order (4), or re-measured?
9. ARCHITECTURE: Dense is CPU-only (DENSE-COMPONENTS-PLAN.md:6) and GPU archetypes must be GPU-pure, so GPU instance data exists twice (a CPU dense column plus an upload ring). Is GPU-resident dense storage in scope?
10. ARCHITECTURE: GpuColumnManager.meta: Stage 5 fold (ARCH-AUDIT:31) vs OK-by-design (MEMORY-SYSTEM-AUDIT:124). The code still holds a Vec (J gpu_column.rs:532).
11. ORCHESTRATOR: Allocator rev 2 must reconcile its HeapVec destinations (Children, SparseMap, render lists; ALLOCATOR:318/323/337) with ECS forms and with the audit's address-stability argument (ARCH-AUDIT:38).
12. ORCHESTRATOR: Animation RUNG-1 §2.2 (64 B clip chunks to avoid tick pages) is stale against J's untracked ScratchColumn (0cb082bb). Animation's table-vs-dense choice cites R-DENSE, which AB-7 lifted.
13. NOT VERIFIED: whether RENDER-GRAPH-API-PLAN (RenderNodeRegistry) shipped. There is no struct of that name in J's crates.
14. OWNER (open since 2026-09-08): a mechanical census gate on Vec used as a durable side store (OPEN-QUESTIONS.md:5090-5094). This applies to UI/render, not only physics.
15. UNOWNED kernel defects that any subsystem using dense filters meets: KE13 (QueryView::get ignores a dense With/Without), KE14 (retained-dense insert path), KE15 (par_iter over dense; ruled to wait for physics Stage 4).

---

# Lens 6 of 6: reference

# Research: how ECS-native engines make UI, render and the rest of the runtime part of the ECS - and what it costs

Provenance tags: [S] source read, [D] official doc, [B] blog/talk/forum (recorded, not relied on). In-tree citations use `tree:path:line "quote"`, with J = D:/wt/joltab, U = D:/wt/ui, R = D:/wt/reflect, M = D:/claude/BoykoEngine. graphify was not run: this role has no shell, and project memory records that graphify is not installed. Orientation was done with Grep/Read.

Freshness: Bevy `main` is `version = "0.20.0-dev"` [S bevy Cargo.toml]; the latest docs.rs release is 0.19.1 [D]. On `main`, extraction has moved into a new `bevy_extract` crate and is generalised from "render world" to "sub world" (`SyncToSubWorld`, `MainEntity`) [S bevy_extract/src/sync_world.rs]. Where a release note is older, the version is named.

Ledger status: I read what exists in `scratchpad/ledger/*.json` without waiting. The ledger was taken on J at commit **ca582e72**, not d11962a9 (`ledger/ui-input.json:3-4 "tree": "D:/wt/joltab", "commit": "ca582e72"`), so it may lag J too, and its UI rows lag U. The drift can be measured: the ledger row `layout.rs` line **211** `let fresh = world.query_entities(&[UiRoot::component_id()]);` sits at `U:crates/boyko_ui/src/layout.rs:215` on U. The file sets also differ: J has `crates/boyko_ui/src/text/vocab.rs` and U has no such file (Glob on both trees). The destination vocabulary the ledger uses, which this report reuses so the names stay compatible with the physics study, is: `ScratchColumn`, `Resource+VmColumn`, `NEW:ScratchStack<T>`, `NEW:VmSlice<T: Copy>`, `NEW:ScratchOut<T>`, `NEW:VmQueue<T>`, `NEW:LoadArena`, `NEW:InlineVec<T,N>`, `NEW:InlineStr<N>`, `NEW:DiagArena` (ledger render.json / ui-input.json / app-demo.json). M:docs/memory/ALLOCATOR-DESIGN-SPACE.md adds `HeapVec<T>` and `DropColumn<T>`.

## TL;DR
- **No surveyed engine runs its whole runtime on one world's storage.** Bevy puts UI nodes, input state and assets in the ECS, but it keeps layout in a taffy side tree behind an `EntityHashMap`, and render data in a second world copied at a sync point. flecs puts its own metadata (components, systems) on entities, but its reference renderer copies components into `ecs_vec` side buffers that it clears every frame. Unity DOTS renders from chunks and leaves UI in GameObjects. The Machinery's scene is an ECS but its UI is immediate-mode. Godot and Unreal mirror state into "servers" or "scene proxies".
- **The second render world exists for CPU pipelining, and Bevy has paid for it repeatedly.** The measured win is 7-29% mean frame-time improvement from pipelining (PR #6503, Ryzen 5600x + RX 6600). The measured costs:
  - Clearing every frame caused "archetype moves" and "workarounds such as moving storage outside of the ECS" (0.15 notes).
  - It blocked "more things becoming entities" (#12144).
  - The first retained-world PR lost ~30% (#14449).
  - Entity-pair hashing in extraction cost bevymark 25% (#17078, fixed on M4 Max: -20.2% median frame time).
  Prior in-tree research already concluded "Don't build a render world now; leave a double-buffer/fence seam" (M:docs/RENDER-PHYSICS-GPU-RESEARCH.md:189).
- **The pattern that recurs wherever render data is ECS-native is a persistent GPU buffer plus change-driven delta upload**, with no per-frame copy. Unity Hybrid V2 (`chunk.DidChange<T>`, "Huge performance benefit", no number) did this, and so did Bevy 0.16 (retained data; Caldera 33.55 -> 10.16 ms, rig not stated). The engine already has the kernel half of this (Phase-5 device-resident `ComponentPool`s) and a UI analogue (an O(1) generation gate).
- **Per-chunk aggregates are an ECS-native way to make render cheap, but not for UI.** Unity's `ChunkWorldRenderBounds` and Mass `ChunkFragment`s do per-chunk culling. UI is painter-ordered across archetypes, and the tree already records that "the global z-sort forbids a per-chunk blit" (J:crates/boyko_render/src/ui/instance.rs:10).
- **In-tree, UI (on U) is already entities + components + systems.** The remaining non-ECS runtime is:
  - host-owned render state (`WindowHost`, `GpuSceneBundles`, `Renderer`: "the `Renderer` is not yet an ECS resource");
  - `Vec`-backed Resource scratch everywhere (UI layout, UI render pack, focus/pick, asset staging);
  - an `Arc<Mutex>` probe in UI hot reload;
  - a pervasive `mem::take` "borrow protocol" that works around a missing kernel capability.

## Approaches in state-of-the-art engines

### Bevy (main 0.20.0-dev; release 0.19.1)

**UI: entities, with a side-store layout engine.**
- **Approach**: UI nodes are entities with `Node` / `ComputedNode` / `UiGlobalTransform` components. Layout runs in taffy, held in a Resource that is not ECS storage [S bevy_ui/src/layout/ui_surface.rs].
- **Data structures**: `#[derive(Resource)] pub struct UiSurface { root_entity_to_viewport_node: EntityHashMap<NodeId>, entity_to_taffy: EntityHashMap<LayoutNode>, taffy: UiTree<NodeMeasure>, taffy_children_scratch: Vec<NodeId>, ... }`. `UiTree<T>(TaffyTree<T>)` carries `unsafe impl Send/Sync` with the justification "Taffy Tree becomes thread unsafe when you use the calc feature, which we do not implement" [S].
- **Sync algorithm**:
  - `ui_layout_system` takes `ResMut<UiSurface>`, `Query<(), Added<Node>>`, `Ref<Node>`, and `RemovedComponents<Children>` / `<Node>` / `<FixedNode>`.
  - Changed nodes are upserted when `computed_target.is_changed() || node.is_changed() || content_size.is_changed() || rem_size_changed || em_size.is_changed()`.
  - Removals are pushed with `ui_surface.remove_entities(removed_nodes.read()...)`.
  - It then calls `compute_layout` per root and writes back to `ComputedNode` / `UiGlobalTransform`, guarded by `if node.size != layout_size` so unchanged values do not trip change detection [S bevy_ui/src/layout/mod.rs; excerpted by the fetch tool].
  - Whether taffy's own cache avoids work on unchanged subtrees was not verified.
- **Stack**: `UiStack { partition: Vec<Range<usize>>, uinodes: Vec<Entity> }` is a Resource rebuilt by `ui_stack_system` from `Local<Vec<..>>` / `Local<EntityHashMap<usize>>` scratch "every frame with no gating condition" [S bevy_ui/src/stack.rs, per the fetch tool].
- **An alternative Bevy does not take**: taffy's low-level API lets its algorithms run over any storage that implements `TraversePartialTree` / `LayoutPartialTree` / `CacheTree` / `RoundTree` ("Any type that implements [`LayoutPartialTree`] can be laid out using Taffy's algorithms") [S taffy/src/tree/traits.rs]. Bevy still wraps `TaffyTree` [S ui_surface.rs].
- **Design stance**: "bevy_ui will be a retained mode UI. Performance is a partial motivator for this, but ease of managing widget state in complex applications is the larger motivation" and "ECS-powered: bevy_ui uses the same data structures and tools for logic as the rest of your game" [B hackmd.io/@bevy/HkjcMkJFC, Alice; undated].
- **Published UI cost**: PR #14064 (merged 2024-07-08): "bevy's ui layout system could takes a long time"; caching the default UI camera gave ~10%; "most of the time is spent in the `round_ties_up`". No rig or ms were stated [S PR page].

**UI render: extracted into the render world each frame.**
- `ExtractedUiNodes { uinodes: MainEntityHashMap<(Entity, EntityIndexMap<ExtractedUiNode>)>, changed: MainEntityHashSet }`.
- `UiMeta { vertices: RawBufferVec<UiVertex>, indices: RawBufferVec<u32>, .. }` is cleared at the start of `prepare_uinodes`.
- Nodes are sorted by `FloatOrd(z_order)` in `queue_uinodes` and batched by consecutive image. There are 15 extract sets (`ExtractBackgrounds`, `ExtractText`, ...) [S bevy_ui_render/src/lib.rs].

**Render world: a second world plus an extract step.**
- **Mechanism** (on main):
  - "Schedule in which data from the main world is 'extracted' into the sub world."
  - `MainWorld` is "The simulation World of the application, stored as a resource. This resource is only available during ExtractSchedule".
  - `ScratchMainWorld` is "A 'scratch' world used to avoid allocating new worlds every frame".
  - `extract()` does `core::mem::replace(main_world, scratch_world.0)`, inserts `MainWorld(inserted_world)`, runs `ExtractSchedule`, then swaps back ("move the app world back, as if nothing happened") [S bevy_extract/src/extract_plugin.rs].
- **Why** (0.6):
  - "We can now start running the main app logic for the next frame, while rendering the current frame."
  - "Pipelining requires drawing hard lines between 'app logic' and 'render logic', with a fixed synchronization point (which we call the 'extract' step)."
  - "The goal is to keep this step as quick as possible, as it is the one piece of logic that cannot run in parallel." [D bevy.org/news/bevy-0-6]
- **Pipelining**: `PipelinedRenderingPlugin` "moves rendering into a different thread, so that the Nth frame's rendering can be run at the same time as the N + 1 frame's simulation". The SubApp is shuttled across two channels [S pipelined_rendering.rs].
  - Measured in PR #6503 on Windows 11, Ryzen 5600x, RX 6600: many_lights 29.35%, many_foxes 27.01%, 3d_scene 25.79%, many_animated_sprites 13.97%, many_cubes 11.97%, many_sprites 7.14% mean improvement. Caveat: "run on an older version of main". With pipelining disabled the PR showed "mostly regressions" [S PR #6503].
- **Retained render world (0.15, PR #15320, merged 2024-09-30)**:
  - Reasons: "The clearing process itself had overhead. 'Table' ECS storage could be expensive to rebuild every frame relative to alternatives, due to 'archetype moves'. As a result, we employed many workarounds such as moving storage outside of the ECS. Full resyncs every frame meant re-doing work that didn't need redoing." [D bevy-0-15].
  - Sync: `SyncToRenderWorld` (now `SyncToSubWorld`) marks entities for mirroring; the pair is linked by `RenderEntity` / `MainEntity` components. `PendingSyncEntity { records: Vec<EntityRecord<L>> }` is fed by an observer on `On<Remove<SyncToSubWorld<L>>>` [S sync_world.rs].
  - Trigger: #12144 (2024-02-26): "There are plans for the ecs (mainly involving Relations) that hinge on more things becoming entities" - the clear-every-frame render world blocked that [S].
  - First attempt (#14449, closed): "~30% loss", mostly "spawn (~3ms) and despawn (~3ms) operations of ephemeral entity". Stated drawback: "component dependencies on other main world entities must also be synchronized to the rendering world, which undoubtedly increases the user's mental burden" [S].
  - Regression after merge: bevymark -25% because `ExtractedSprites` moved to `FixedHasher` over `(Entity, MainEntity)`. The fix (#17078, 2025-01-01, M4 Max, `bevymark --waves 100 --per-wave 1000`) gave -20.2% median frame time and -49.7% median `extract_sprites` [S].
- **GPU-driven 0.16**: "a retained render world, which allows the CPU to avoid processing and uploading data that hasn't changed since the last frame". Caldera (127,515 objects): 0.15 33.55 ms -> 0.16 10.16 ms, credited to all 0.16 optimisations together; rig not stated in the fetched text [D bevy-0-16].

**Text.**
- Uses parley on main: `FontCx`, `LayoutCx`, `ScaleCx` Resources [S bevy_text/src/lib.rs].
- `FontAtlasSet(HashMap<FontAtlasKey, Vec<FontAtlas>>)` is a Resource; the key exists "to allow an `f32` font size to be used as a key in a `HashMap`" [S font_atlas_set.rs].
- `TextPipeline { sections_buffer: Vec<..>, text_buffer: String }` is a Resource. Per-entity shaping output is the `ComputedTextBlock` component [S pipeline.rs].

**Input.** `#[derive(Resource)] pub struct ButtonInput<T> { pressed: HashSet<T>, just_pressed: HashSet<T>, just_released: HashSet<T> }` [S button_input.rs]. `keyboard_input_system(ResMut<ButtonInput<KeyCode>>, ResMut<ButtonInput<Key>>, MessageReader<KeyboardInput>, ...)`, where `KeyboardInput` derives `Message` [S keyboard.rs].

**Assets.**
- **Storage**: `Assets<A>` is a Resource with `dense_storage: DenseAssetStorage<A>` (`storage: Vec<Entry<A>>`, generational), `hash_map: HashMap<Uuid, A>`, and `queued_events: Vec<AssetEvent<A>>`. `AssetMut`'s `Drop` pushes `Modified` [S assets.rs].
- **Assets-as-entities** is a standing goal, not shipped:
  - #11266 (2024-01-09, closed as duplicate of #23094). Benefit: "Change detection for assets would just be regular component change detection". Costs: "Adding assets is no longer immediate. It requires calling `apply_deferred`", "weird" mixed-type asset entities, and assets without their managing systems.
  - Discussion #18414 (2025-03): "If your asset internally holds Arcs which themselves hold asset handles, those asset handles cannot be entity-mapped later"; the design "can no longer load assets from a thread"; both authors agree it needs remote entity reservation first.
  - #23094 (opened 2026-02-21): open, "Staffed (Approved)", no design doc and no PRs yet [S].

**One-shot systems.** `run_system_once(..) -> Result<Out, RunSystemError>`, documented as "not an efficient method of running systems and it's meant to be used as a utility for testing and/or diagnostics" [S bevy_ecs/src/system/system.rs]. `World::resource_scope` appears in the 0.19.1 docs.rs table of contents; its signature was not fetched (truncated).

- **Trade-offs**: UI logic, input, assets and render entities all use one API. But the storage is not one storage: taffy side tree, `EntityHashMap` / `MainEntityHashMap` indirections, a second world, and `HashSet` input state.

### flecs (master)
- **Everything-as-entity metadata**:
  - "many API constructs are implemented using entities" [S docs/Manual.md]
  - "In Flecs this is done by making each component is its own unique entity"; "Singletons are implemented as components that are added to themselves" [S docs/EntitiesComponents.md]
  - "Systems are queries + a function that can be ran manually or get scheduled as part of a pipeline" [S docs/Systems.md]
- **Scheduling**: builtin phases OnLoad ... PreStore/OnStore. "When a pipeline sees a read for a component for which commands could have been inserted, a sync point is inserted before the system that reads." "The scheduler runs each multithreaded system on all threads, and divides the number of matched entities across the threads." [S docs/Systems.md]
- **Observers**:
  - "queries that are combined with a callback"
  - "Observers are always executed when the operation that triggered the observer happens, on the thread where the operation is executed"
  - "a basic event queue is always going to outperform observers"; "systems are much more efficient to run, and have more predictable performance" [S docs/ObserversManual.md]
- **Modules**: "A good practice to employ with modules is to split them up into components.* modules and systems.* modules ... you can simply swap one physics implementation with another without changing application code." [S docs/DesignWithFlecs.md]
- **Storage traits**:
  - "Sparse components are stored outside of tables, which means they do not have to be moved"; they "are also guaranteed to have stable pointers"; they "trade in query speed for component add/remove speed".
  - "The `DontFragment` trait uses the same sparse storage as the `Sparse` trait, but does not fragment tables" [S docs/ComponentTraits.md].
- **Change detection**: per-table counters, one per component, plus one for add/remove. Queries cache a copy and compare. "counters are only tracked for tables matched with queries that use change detection" [D docs/Queries.md via a search-result excerpt only; the full section could not be fetched because it was truncated - treat as not verified word-for-word].
- **Rendering (reference module flecs-hub/flecs-systems-sokol, geometry.c on master)**:
  - One system, `ECS_SYSTEM(world, SokolPopulateGeometry, EcsPreStore, Geometry, [in] GeometryQuery)`.
  - It iterates the query and copies components into side buffers: `ecs_vec_init_t(a, &result->transforms_data, mat4, 0)` and `colors_data`. These are cleared every frame (`ecs_vec_clear(&buffers->transforms_data)`) with no `ecs_query_changed`, then uploaded (`sg_make_buffer`) [S].
  - The Flecs 3.1 release post reportedly says the geometry module "now uses groups for broad-phase visibility tests and change detection" [B, search snippet only; Medium returned 403]. The master geometry.c I fetched does not use `group_by`. This contradiction is unresolved.
- **UI**:
  - flecs-hub/flecs-components-gui defines `EcsCanvas` (title, size, camera, lights, fog), `EcsText { char *value }`, `EcsFontSize`, `EcsAlign`, `EcsPadding` - canvas and text only, no widget set [S].
  - The Explorer is a "Web-based UI for monitoring Flecs applications", external to the process, over REST (flecs-hub repo description) [S].
  - An "ECS UI framework with mouse events and CSS-like style sheets" by Mertens appears only in search snippets pointing to X [B, not verified].
- **Trade-offs**: engine metadata is uniformly entities. The shipped renderer is "renderer as a system" in the same world (no second world), but still copies into rebuilt side buffers.

### Unity DOTS / Entities Graphics
- **Approach**: "Entities Graphics acts as a bridge between ECS for Unity and Unity's existing rendering architecture". Baking turns `MeshRenderer` + `MeshFilter` into `RenderMesh` and `Transform` into `LocalToWorld` [D entities.graphics 1.4 overview].
- **Algorithm** (Hybrid Renderer V2, 0.4.0, 2020-03-13):
  - "GPU persistent data model. ComputeBuffer to store persistent data on GPU side. Use `chunk.DidChange<T>` to delta update only changed data. Huge performance benefit."
  - "Hybrid Renderer and culling no longer use hash maps ... Chunk components and chunk/forEach jobs are used instead."
  - "Batch setup and update now runs in parallel Burst jobs." [D Hybrid Renderer 0.5 changelog]
  - No number is published.
- **Data structures**:
  - `ChunkWorldRenderBounds : IComponentData` is "the combined world-space bounds for every entity inside the chunk. Entities Graphics uses it for visibility culling at the chunk level" [D API 1.3].
  - A chunk component "stores values per chunk instead of per entity" [D entities 1.3].
- **Constraints**: "Entities Graphics does not support multiple Worlds"; it requires an SRP; on URP "only Forward+ rendering path is supported" [D requirements 1.3].
- **UI**:
  - The official ECS package list is Entities, Unity Physics, Havok, Netcode for Entities, Entities Graphics - no UI, audio or animation [D entities 1.2 ecs-packages].
  - Unity staff (etienne_unity, 2020-09-29): "If you're using DOTS on top of UnityEngine, you can use UI Toolkit like any other UnityEngine dependency. It won't use DOTS rendering." [B forum].
  - No official rationale for keeping UI out of ECS was found.
- **Direction**: "ECS for All", Dec 2025 status (Eric Dziurzynski): "Every GameObject will have the ability to have ECS component data attached to it"; "EntityId will represent both GameObjects and Entities" [B official forum post].
- **Trade-offs**: render data is chunk-native with per-chunk culling and persistent GPU data. UI and most tooling stay GameObject-side.

### The Machinery (Our Machinery, 2017-2021; archive at ruby0x1.github.io/machinery_blog_archive)
- **Render**:
  - "Scenes in The Machinery are built using an Entity-Component-System (ECS), where components are written as plugins."
  - Rendering goes through `tm_ci_render_i` / `tm_ci_shader_i`; per-frame CPU work is 8 steps, and "All of the above steps, except 1, 2 and 6 runs in parallel for each component plugin" [B Persson, 2018-10-23].
  - Where render data lives and how change propagates are not stated in that post.
- **UI**: "We've decided to go with an immediate mode rather than a retained mode model for the UI". Draw functions write into primitive vertex/index buffers each frame: "Draw everything with a single draw call." [B Gray, 2017-07-17]. No numbers.
- **Syncing with stateful externals (PhysX)** [B Gray, 2019-06-08]:
  - Brute force "should be the default go-to solution for most change tracking problems. It should be fast enough for most scenarios".
  - Callbacks: "we don't know what the callback might do and how it might interact with other threads".
  - Hierarchical dirty flags: "the update cost will just be O(log N) rather than O(N)".
  - Per-component "Changing" tags: "risk of a blow-up in the number of entity types".
- **Creation graphs** [B Persson, 2019-04-04]: a node-graph framework separate from component storage. "CPU Image" lives in The Truth, "GPU Image" in video memory. Caching uses validity hashing.

### Counter-example 1: Godot (not ECS)
- "nodes are just interfaces to the actual data being processed inside servers"; "Godot uses plenty of data-oriented optimizations for physics, rendering, audio, etc."; "most games are generally just in the hundreds of objects at most" [B Linietsky, 2021-02-26].
- Servers "are low-level APIs to control rendering, physics, sound, etc."; RIDs "are opaque handles to the server implementation"; the scene system is optional [D docs using_servers]. No numbers.
- **What it gains**: nodes with inheritance and signals for authoring; data-oriented servers behind opaque IDs. This is exactly the "subsystem glued on the side with its own store" shape that principle 0 forbids.

### Counter-example 2: Unreal (not ECS; Mass as an island)
- "FPrimitiveSceneProxy: Renderer version of UPrimitiveComponent, mirrors UPrimitiveComponent state for the rendering thread"; "FScene: Renderer version of the UWorld"; "Game thread code that changes a component's properties must call MarkRenderStateDirty()" [D Graphics Programming Overview].
- "The entire renderer operates in its own thread that is a frame or two behind the game thread"; "The game thread can never touch the members of memory of an FPrimitiveSceneProxy after it is created and registered" [D Threaded Rendering]. This is structurally Bevy's render world, without an ECS.
- Mass: "MassEntity is a framework for data-oriented calculations"; "A ChunkFragment is a Fragment associated with a Chunk"; "Processors are stateless classes" [D]. Mass Representation switches per LOD between a high-res Actor, a low-res Actor, ISM, or none [D Mass Gameplay]. So ECS is one island in an actor engine.

## Comparative table

| Aspect | Bevy (0.20-dev) | flecs | Unity DOTS | The Machinery | Godot / Unreal |
|---|---|---|---|---|---|
| UI widgets | entities + components | only canvas/text components; Explorer is external web | GameObject UI Toolkit, "won't use DOTS rendering" | IMGUI, rebuilt per frame | nodes / UObjects |
| UI layout storage | taffy tree in `UiSurface` Resource + `EntityHashMap` | n/a | n/a | n/a | engine-internal |
| Render data home | second (sub) world, retained since 0.15 | same world; renderer is a PreStore system copying into `ecs_vec` | chunks + persistent GPU buffer | component plugins -> render graph | servers (RID) / scene proxies (FScene) |
| Change propagation to render | tick change detection, `SyncToSubWorld` + observer on Remove | per-table counters (search excerpt) | `chunk.DidChange<T>` | brute force by default; callbacks for add/remove | `MarkRenderStateDirty`, render commands |
| Per-frame copy | extract step (serial) | full re-copy, `ecs_vec_clear` | delta only | not stated | proxy updates only |
| Engine metadata as entities | components/queries partly; relations pushed for it | yes (components, systems, singletons) | no | no | no |
| Input | `ButtonInput` Resource (`HashSet`) + Messages | flecs-components-input module (not read) | not surveyed | not surveyed | engine |
| Assets | `Assets<A>` Resource; assets-as-entities open (#23094) | not surveyed | baking | The Truth | resources |
| Published cost | pipelining +7-29% (5600x/RX 6600); retain -30% then fixed; hash -25% (fix on M4 Max) | none found | "Huge" (no number) | none | none |

## Key algorithms and techniques
1. **Entity per widget + hierarchy relation.** Bevy and boyko U both do this: `U:crates/boyko_ui/src/lib.rs:3 "//! Widgets are entities; layout inputs/outputs are components; the tree is"`, `:4 "//! `ChildOf`/`Children` (Phase 19); layout is two systems over the ECS. There is"`.
2. **Side store + entity-to-foreign-ID map** (Bevy `UiSurface`, `MainEntityHashMap`; Godot RIDs; Unreal proxies). Its alternative is running the algorithm over ECS storage through traits (taffy low-level API). U already does layout in-house over ECS columns plus Resource scratch.
3. **Second world + sync point** (Bevy extract; Unreal game/render thread). It buys sim(N+1) || render(N) and costs a serial extract, two copies, an entity mapping, and a frame of latency ("a frame or two behind").
4. **Same-world gather -> reused scratch -> upload** (flecs sokol; boyko UI). `J:crates/boyko_render/src/ui/instance.rs:7-10 "PACK-SCRATCH ONLY (GUI P5a Decision 6): the upload system materializes a reused / `Vec<UiInstance>`, stable-sorts it by `StackIndex`, and bulk-memcpys it into the / mapped per-frame ring slot — it is NOT an ECS column and NOT a per-chunk / `cast_slice` (the global z-sort forbids a per-chunk blit)."`
5. **Persistent GPU data + delta upload** (Unity DidChange; Bevy 0.16). The in-tree kernel analogue is `J:crates/boyko_render/src/gpu_column.rs:3-4 "//! Phase 5 Wave B mints REAL device-local component pools: [`GpuColumnManager`] / //! allocates a `DeviceLocal` (VRAM) buffer through the [`boyko_rhi`] registry,"`.
6. **Chunk-level aggregate components** (Unity `ChunkWorldRenderBounds`, Mass `ChunkFragment`) for culling or LOD at chunk granularity.
7. **O(1) global change gate** instead of an O(N) scan (boyko UI): `J:crates/boyko_render/src/ui/pack.rs:193-194 "/// `if gen == scratch.last_seen_generation { return; }` — the 0%-when-static / /// guarantee is an O(1) compare, not an O(N) Changed scan."` This is the brute-force-with-a-dirty-flag end of The Machinery's taxonomy.
8. **Module split components.* / systems.*** (flecs) so an implementation can be swapped behind stable components.
9. **Immediate-mode UI** (The Machinery): no retained widget state at all. The alternative to UI-as-ECS.

## Pitfalls and mistakes
- **Entity-keyed hashing on the render path**: -25% bevymark (#17078). Principle 1 already bans `HashMap` on the hot path.
- **Clear-and-rebuild worlds**: archetype-move overhead, storage pushed "outside of the ECS", and it blocked entity-ization (#12144). The retained fix first cost ~30% (#14449).
- **Callback / observer sync**: breaks reasoning under parallel scheduling (Machinery), and is slower than an event queue (flecs docs).
- **Unconditional per-frame rebuilds**: Bevy `UiStack` (no gate); flecs sokol `ecs_vec_clear` plus full copy. No cost is published for either.
- **Assets-as-entities obstacles**: deferred insertion, `Arc`-held handles not entity-mappable, no loading from threads, remote entity reservation required (#11266, #18414).
- **Per-chunk blit versus global order**: UI z-order crosses archetypes (`J:.../ui/instance.rs:10`).
- **Tick cost on tiny rows**: `J:crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:17-19 "//! That is not a micro-optimisation: a tracked column pays 8 B/row of ticks on / //! top of the datum and a 192 KiB resident floor, so the unread ticks were / //! two thirds of this type's storage cost across the solver's live columns."` UI nodes and draw records are small rows, so this ratio applies.
- **Pipelining's price when absent**: 7-29% mean frame time in Bevy's measurement. That is the number the "no second world" position has to own.

## Relevant academic works
- No peer-reviewed work on ECS-native UI or render-world design was found in this pass; the evidence base is engine source, official docs and author posts.
- The only paper already cited in-tree for the storage side is Berger/Zorn/McKinley, OOPSLA 2002 ("the two winners were *regions*"), in `M:docs/memory/ALLOCATOR-DESIGN-SPACE.md:64`. Not re-verified here.

## Applicability to boyko-engine

**In-tree state (non-ECS pieces the evidence maps onto):**
- **Render host state is outside the ECS**:
  - `J:crates/boyko_app/src/host.rs:81 "pub(crate) struct WindowHost {"`, `:83 "pub(crate) renderer: Renderer<'static>,"`, `:90 "pub(crate) gpu: GpuSceneBundles,"`, `:96 "pub(crate) retire_scratch: Vec<FreeEntry>,"`
  - `J:crates/boyko_app/src/gpu_scene/mod.rs:920 "/// Owned by the host (`WindowHost.gpu`), created once at boot, destroyed"`
  - `J:crates/boyko_render/src/ui/upload.rs:46-48 "capability still missing for the in-schedule upload is the swapchain `Renderer` / slot index + in-flight fence — the `Renderer` is not yet an ECS resource — so the / host drives the path through `host_upload_frame_from_world` until an ECS-resident"`
- **The render read path is already a same-world projection, not a copy into a second world**: `J:.../ui/upload.rs:37-39 "seam (#31) gathers the visible nodes from a [`DispatcherToken::world`] / [`WorldView`] (a read-only ECS / projection, #30) FIRST, ends that borrow..."`. This matches flecs' renderer-as-system shape and the prior conclusion `M:docs/RENDER-PHYSICS-GPU-RESEARCH.md:185-189 "- **Extract largely COLLAPSES.** ... (flecs-style renderer-as-system, / not a separate World). **Don't build a render world now; leave a double-buffer/fence seam** if display-rate"`.
- **UI render scratch is `Vec` in a Resource**: `J:crates/boyko_render/src/ui/pack.rs:143 "pub pack: Vec<UiInstance>,"`, `:150 "pub keys: Vec<(u32, u32)>,"`.
- **UI layout scratch is `Vec<Vec<..>>` in a Resource**: `U:crates/boyko_ui/src/resources.rs:216 "pub(crate) child_pool: Vec<Vec<Entity>>,"`. The ledger destination is `NEW:ScratchStack<Entity>`.
- **Focus scratch**: `U:crates/boyko_ui/src/interaction/focus.rs:117 "candidates: Vec<Candidate>,"`.
- **Font data**: `U:crates/boyko_ui/src/text/font.rs:139 "fonts: Vec<FontEntry>,"`, `:33 "glyphs: Box<[GlyphMetrics]>,"`.
- **The `mem::take` borrow protocol recurs across the UI.** It appears in layout, focus, pick, visibility, project, widgets, binding, animation and dispatch:
  - `U:crates/boyko_ui/src/layout.rs:146 "/// Uses the `mem::take` borrow protocol: the scratch buffers are moved onto the"`, `:170 "(s.dirty, mem::take(s))"`
  - `U:crates/boyko_ui/src/interaction/focus.rs:155 "let mut scratch = mem::take(world.resource_mut::<UiInteractionScratch>());"`
  - Bevy's analogue is `World::resource_scope` (present in the 0.19.1 docs table of contents; semantics not fetched).
- **Hot reload uses `Arc<Mutex>` probes**: `U:crates/boyko_ui/src/reload/system.rs:167 "let plan_sink: Arc<Mutex<DespawnPlan>> = Arc::new(Mutex::new(DespawnPlan::default()));"`, justified at `:23-24 "It exists solely to satisfy / // the `Sync` bound of the one-shot system closure"`. Yet the kernel's one-shot already returns a value: `U:crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs:111 "pub fn run_system<F, M, Out>(&mut self, system: F) -> Out"`. Whether `Out` can carry `DespawnPlan` / `UiParseReport` under that bound was not verified.
- **Assets are already on kernel storage**, which is further than Bevy: `J:crates/boyko_ecs/src/ecs/core/asset/assets.rs:200-202 "pub struct Assets<T: AssetBacking> {" / "col: ComponentPool," / "slot_word: VmColumn<u32>,"`. Heap remnants: `:205 "free: Vec<u32>,"` and `J:crates/boyko_ecs/src/ecs/core/asset/staging.rs:58 "queue: Vec<Staged<A>>,"`. They are inserted as non-send resources: `J:crates/boyko_app/src/runner.rs:242 ".insert_non_send_resource(Assets::<MeshGpu>::default());"`.
- **Input is already a fixed ring + bitsets**, not `HashSet`s: `J:crates/boyko_input/src/raw/queue.rs:11 "/// Fixed-capacity SPSC ring buffer of raw input events (plan §5.4)."`, `:35 "buf: Box<[RawInputEvent]>,"`; `J:crates/boyko_input/src/action/state.rs:45 "pressed: BitSet256,"`.
- **Reflection metadata is a process static keyed by `ComponentId`**, not component data (compare flecs components-as-entities): `R:crates/boyko_reflect/src/registry.rs:29 "static REFLECT: [OnceLock<&'static TypeInfo>; MAX_COMPONENTS] ="`.
- **Kernel features the patterns would lean on already exist**:
  - dense non-fragmenting storage: `J:crates/boyko_ecs/src/ecs/core/component/dense/dense_store.rs:4-7 "//! One `DenseStore` per dense `ComponentId` holds every instance of that type / //! across all archetypes in ONE contiguous column. ... / //! swap-remove, so **live slots never move**"` (flecs `DontFragment` is the closest analogue)
  - observers: `J:crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs:143 "pub fn observe_on_add<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {"`
  - hooks: `J:crates/boyko_ecs/src/ecs/core/component/hooks/builder.rs:81 "pub fn on_add(mut self, f: HookFn) -> Self {"`
  - log ring: `J:crates/boyko_ecs/src/ecs/core/log/ring.rs:103 "pub struct LogRing {"`
  - `ScratchColumn`: `J:.../scratch/scratch_column.rs:43 "pub struct ScratchColumn<T: Copy> {"`

**Patterns with evidence that they fit the stated constraints (for the architect to accept or reject):**
- **Fit**:
  - Retained entity-per-widget UI (Bevy's stance; already in U).
  - Persistent GPU columns + delta upload (Unity V2, Bevy 0.16; already Phase 5).
  - A cheap global dirty gate before any scan (boyko UI generation; The Machinery's brute-force default).
  - Chunk/aggregate components for culling world-space render entities (Unity, Mass) - but not for painter-ordered UI (`instance.rs:10`).
  - flecs' components.* / systems.* module split, which is already expressed as "capability = presence of a component".
  - Running algorithms over ECS storage via traits rather than a side tree (taffy low-level API; U already does this in-house).
- **Evidence against copying**:
  - A second world copied each frame. Its documented purpose is CPU pipelining (0.6). Its documented costs are serial extract, archetype moves, storage pushed outside the ECS, blocked entity-ization, entity-pair hashing and mental burden. With device-resident columns the copy has nothing to carry for GPU-resident archetypes (M:RENDER-PHYSICS-GPU-RESEARCH.md:185-187). By construction it is a second data system, which order (3) "ONE UNIFIED SYSTEM" rules out.
  - Entity -> foreign-ID `HashMap`s (`UiSurface`, `MainEntityHashMap`): Principle 1, plus the -25% measurement.
  - `HashSet` input state (Bevy): the tree already beats it with `BitSet256`.
  - Observers as the per-frame sync path: flecs' own docs rank them below an event queue.
  - Immediate-mode UI (The Machinery): viable, but it is the opposite of order (4).
- **Adaptation note**: whatever replaces host-owned `WindowHost` / `GpuSceneBundles` / `Renderer` state must keep the `!Send` single-thread discipline already modelled by `NonSendResource` (`J:crates/boyko_app/src/runner.rs:239 ".insert_non_send_resource(RhiContext::from_shared(ctx));"`). That is the kernel feature Bevy lacks and works around with a whole SubApp.

## Open list (lens reference)

1. Pipelining trade-off: without a second world, the engine forgoes Bevy-measured sim(N+1)||render(N) gains of 7-29% mean frame time (PR #6503, Ryzen 5600x + RX 6600, older Bevy main). Is a double-buffer/fence seam, as the M doc recommends, enough, and on which columns would it sit?
2. Host render state outside the ECS: WindowHost / GpuSceneBundles / Renderer (J:boyko_app/src/host.rs:81-96; gpu_scene/mod.rs:920-924; upload.rs:46-48). Which of these become NonSendResources or Resource-owned columns, and does the in-schedule UI upload then replace host_upload_frame_from_world?
3. The mem::take borrow protocol recurs across boyko_ui on U (layout, focus, pick, visibility, project, widgets, binding, animation, dispatch). Is a kernel resource-scope / borrow-split capability (Bevy has World::resource_scope; its semantics were not fetched here) the first-class feature that removes it, and is it shared with the physics study's needs?
4. UI hot reload uses Arc<Mutex> probes (U:reload/system.rs:167-168) although run_system returns Out (U:system_api.rs:111). Whether the Out bound admits DespawnPlan/UiParseReport was not verified.
5. Tick overhead for small UI/render rows: 8 B/row + a 192 KiB floor per tracked column (J:scratch_column.rs:17-19). Which UI components need Changed/Added ticks, and which could be untracked or dense?
6. Should engine metadata (reflection TypeInfo, currently a static OnceLock array, R:registry.rs:29) become component data on component entities, as in flecs? No evidence about cost was found either way.
7. flecs change-detection details (per-table counters) come from a search-result excerpt of docs/Queries.md; the full section was not fetched (truncation). Not verified word-for-word.
8. Contradiction not resolved: a snippet of the Flecs 3.1 blog says the sokol geometry module uses query groups for visibility and change detection, but the master geometry.c fetched here has no group_by and no ecs_query_changed.
9. Bevy 0.16 Caldera numbers (33.55 -> 10.16 ms) have no rig in the fetched text; the Unity Hybrid V2 'Huge performance benefit' has no number; Bevy PR #14064's ~10% has no rig or ms.
10. No official Unity rationale for keeping UI outside ECS was found beyond the 2020 staff forum statement that UI Toolkit is used 'like any other UnityEngine dependency' and 'won't use DOTS rendering'.
11. Ledger staleness: the ledger was taken on J @ ca582e72 (not d11962a9) and lags U's boyko_ui by 16 commits. Line drift was observed (layout.rs 211 vs 215), and the file sets differ (J has text/vocab.rs, U does not). UI rows must be re-mapped onto U before they are used.
12. Whether taffy's internal cache makes Bevy's per-root compute_layout incremental for unchanged subtrees was not verified.
