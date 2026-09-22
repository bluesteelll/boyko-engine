# UI Advanced — Interaction Research

> Campaign: advanced UI/GUI for `crates/boyko_ui` (sprites, animation, rich interactivity) and its
> integration into the Aether DSL. This document covers **interaction only**: hit-testing,
> dispatch, capture, drag-and-drop, scrolling, text input/IME, keyboard navigation, gestures,
> tooltips.
>
> Status: RESEARCH. No code is proposed as final here; §9 states a recommendation and the strongest
> argument against it. **No benchmark was run for this document** — every number below is either
> (a) read out of source, (b) arithmetic on struct layouts, or (c) explicitly labelled an estimate.

---

## 0. Method

Two passes:

1. **Read what exists.** `crates/boyko_ui` (~25 k lines) and the kernel mechanisms it could use were
   read directly. Every claim about this engine below carries a `file:line`.
2. **Read what the references do.** Bevy, Unity (both uGUI and UI Toolkit), Godot, browsers
   (WebRender + the Chromium compositor split + the DOM event spec), Dear ImGui — from their own
   sources or specifications, not from tutorials, wherever the detail was load-bearing. Links in
   §10.

The deliverable is comparative: §3 is the table, §4 the three candidate models, §5 the
mechanism-by-mechanism analysis, §9 the recommendation and its counter-argument.

---

## 1. What `boyko_ui` does today — measured by reading

### 1.1 The one interaction pass

`ui_focus_system` (`crates/boyko_ui/src/interaction/focus.rs:145`) is an **exclusive** system
(`&mut EcsMaster`) that runs every frame in `GameplaySet` and does the whole job in six steps:

| Step | Site | What it does |
|---|---|---|
| collect | `focus.rs:204` `collect_candidates` | `query_entities_buf(&[UiRoot])` → sort roots by `Entity::id` → DFS over `Children` pushing children in reverse → for every node carrying `Interaction`, push a `Candidate { entity, paint_seq, stack_index, rect, clip, block }` |
| blur | `focus.rs:161` | `!cursor_inside \|\| !window_focused` → reset everything, cancel pending clicks, clear focus, return |
| resolve | `focus.rs:301` `resolve_hovered` | linear scan of `candidates`; keep the max by the total order `(stack_index, paint_seq, entity.id)`; `point_in_rect` + `point_in_clip` |
| write | `focus.rs:340` `write_interactions` | **unconditional** pass over every candidate: hovered → `Pressed`/`Hovered`, everyone else → `None`; set-if-changed; EnableTag bits toggled only on genuine transitions; `RelativeCursorPosition` written on the hovered node |
| click | `focus.rs:464` `resolve_pointer` | stamp `(origin, action)` at press; fire `click_fired` at release iff release lands on the same origin |
| focus | `focus.rs:516` `update_focus` | rebuild `focusables` `(tab_index, paint_seq, entity)`, sort, Tab advances cyclically, Enter stamps `pending_submit` |

`ui_dispatch_system::<A>` (`interaction/dispatch.rs:42`) then lowers those edges into
`ActionState<A>` — click O(1), submit O(1), hover-enter O(changed) by draining
`UiInteractionScratch::hover_entered`. `ui_refreeze_fixed_snapshot` (`dispatch.rs:127`) carries the
UI edge into the Fixed schedule exactly once.

This is a **flat, single-target, non-propagating** model, and it is already the right skeleton. The
rest of this document is mostly about what has to be added around it.

### 1.2 What is present

* `Interaction { None, Hovered, Pressed }` — a tick-bearing column, written set-if-changed
  (`interaction/components.rs:17`).
* Three EnableTag bits `UiHovered` / `UiPressed` / `UiFocused` as the O(1) filter surface
  (`interaction/plugin.rs:54-58`), toggled transition-gated.
* `FocusPolicy { Block, Pass }` occlusion (`interaction/components.rs:51`).
* `Focusable { tab_index }` + linear cross-root tab order (`focus.rs:516`).
* `OnClick`/`OnHover`/`OnSubmit` as `#[repr(transparent)] u16` dense action indices with a
  `NO_ACTION = u16::MAX` sentinel (`interaction/action.rs`) — reflection-free, `.ui`-authorable, and
  crucially **no `Box<dyn Fn>` anywhere**.
* `RelativeCursorPosition` (opt-in, normalized to `(-0.5..0.5)²`) — the seam a slider would use.
* All scratch in one retained `Resource` (`UiInteractionScratch`, `focus.rs:114`) — clear-then-refill,
  allocation-free in steady state.
* World-space UI picking against 3D bounds with a CPU occlusion proxy (`world/pick.rs`).

### 1.3 What is absent — the gap list

| Capability | State today | Evidence |
|---|---|---|
| Drag & drop | **absent**. No press-origin motion tracking beyond `pending_click`; no drag threshold; no drop target | `focus.rs:464` `resolve_pointer` only stamps/fires |
| Scrolling | **absent**. `PhysicalInput.wheel` is accumulated by the input layer but `boyko_ui` never reads it | `grep wheel crates/boyko_ui` → 0 hits |
| Momentum / fling | absent | — |
| Computed clip | **absent**. `ComputedClip` is *author-owned*, never derived from an overflow policy | `components.rs:188-190`: "AUTHOR-OWNED in P1 (not computed); P1's overflow policy is 'allow overflow'" |
| Text input | **absent, and the seam is dead**. `RawInputEvent::Text(char)` exists (`boyko_input/src/raw/event.rs:29`), is **never produced** (the Win32 translator handles no `WM_CHAR`, `boyko_input/src/win32.rs`) and is **explicitly dropped** on ingest (`raw/queue.rs:302`: `RawInputEvent::Text(_) => {}`) | see §5.6 |
| Caret / selection | absent | text pipeline is measure+emit only (`src/text/`) |
| IME | **absent at every layer** — no `WM_IME_*` in the Win32 translator, no preedit state anywhere | `win32.rs` message list |
| Keyboard nav beyond Tab | absent. Tab-cycle only; no Shift+Tab, no arrow/directional, no tab groups, no focus-visible distinction | `focus.rs:568` `advance_focus` |
| Focus ring | the `UiFocused` bit exists; **nothing renders it** | `boyko_render/src/ui/pack.rs` reads only `UiBackground`/`ComputedClip`/`StackIndex` |
| Gestures | absent | — |
| Tooltips | absent (no hover timer; `hover_entered` is a pure edge with no dwell) | `focus.rs:393` |
| Hit-test under transforms | **structurally absent** — `boyko_ui` has no transform component at all. `ComputedRect` is an axis-aligned screen rect and both the hit-test and the clip test are axis-aligned `>=`/`<` compares | `focus.rs:271-287` |
| Multi-pointer / touch | shaped but not wired: `MAX_POINTERS = 1` with a fixed `[PointerSlot; MAX_POINTERS]` array | `focus.rs:38` |
| Propagation | absent by design (flat, single target) | §4 |

### 1.4 Two structural facts that shape every recommendation below

**(a) Clipping is free on the GPU side, and rectangular.** `UiInstance` carries a per-instance clip
AABB in physical px, flag-gated by `FLAG_CLIP_PRESENT`, and the whole UI is **one instanced draw**
(`boyko_render/src/ui/instance.rs:40-42`, `ui/draw.rs:74` sets one full-extent scissor and records
`draw(6, N, 0, 0)`). Consequence: a scroll container costs **zero** extra draw calls and zero state
changes — unlike every retained UI that breaks batches at a scissor boundary. Nested clips are an
AABB intersection at pack time. Non-rectangular / rotated clipping is *not* available and would need
a new instance field and an eDSL change.

**(b) The candidate record is one cache line, and the collect pass is random-access.**
`Candidate` (`focus.rs:97`) is `Entity` (16 B: `EntityId(usize)` + `u32` gen, align 8) + `paint_seq`
(4) + `stack_index` (4) + `ComputedRect` (16) + `Option<ComputedClip>` (20) + `bool` (1) = 61 B of
payload → **64 B natural size**. So a 1000-node UI scans 64 KB per frame, twice the 32 KB L1d.
Worse, `collect_candidates` does **five per-entity random-access `get_component` probes per node**
(`ComputedClip`, `ComputedRect`, `Interaction`, `StackIndex`, `FocusPolicy` — `focus.rs:221-234`)
plus a `Children` probe, every frame, whether or not anything moved. That — not the scan — is the
cost, and §5.1 addresses it.

---

## 2. The reference systems: who receives the pointer event, and what it costs

The question the task asks — *how does each system decide which element receives a pointer event,
and how does it stay O(1)-ish rather than walking the tree* — has an honest answer worth stating up
front: **none of them is O(1), and only one of the five is even sublinear in the general case.**
What they actually do is four things, in descending order of payoff:

1. **Capture** — while a pointer is owned by a widget, *no hit test runs at all*. This is the only
   genuine O(1) path, and every one of the five has it.
2. **Event-driven, not frame-driven** — browsers hit-test when an input event arrives; a still
   pointer over a still page costs nothing. (boyko_ui hit-tests unconditionally every frame.)
3. **A precomputed flat, z-sorted array** scanned front-to-back with early-out at the first blocker
   — O(N) worst case, but linear, prefetchable, and in practice terminating in a handful of
   iterations because UI is mostly opaque.
4. **Two-level bounds** — resolve a coarse container first (window / canvas / stacking context),
   then only its members. This is the only structure that is sublinear, and it is a *hierarchy of
   bounding boxes*, i.e. a one-level BVH.

### 2.1 Bevy — `bevy_picking` + the `bevy_ui` backend + observers

**Hit-test.** `bevy_ui`'s picking backend walks `UiStack` — a **precomputed flat array in render
order** — in reverse: `for uinodes in ui_stack.partition.iter().rev()` then
`for node_entity in uinodes.iter().rev()`. Per node it tests
`node.node.contains_point(*node.transform, *cursor_position)` **and**
`node.calculated_clip.is_none_or(|clip| clip.contains_point(*cursor_position))`. `Pickable`'s
`should_block_lower` stops the iteration; **a node without `Pickable` blocks by default**.
So: flat array, reverse paint order, early-out at the first opaque hit — the same shape as
`resolve_hovered` except that ours does not early-out (it scans all N and keeps the max).

**Merge and dispatch.** The backend only *produces hits*; `bevy_picking::hover` merges them.
That merge is where Bevy pays: `OverMap` is
`HashMap<PointerId, BTreeMap<PickLayer, DepthSortedHits>>`, the final `HoverMap` is
`HashMap<PointerId, EntityHashMap<HitData>>`, plus a `PreviousHoverMap` for change detection, and
hits are `sort_by_key(|(_, hit)| FloatOrd(hit.depth))` per layer per frame.

**Propagation.** Bevy 0.15+ dispatches `Pointer<E>` as an *observer trigger* that bubbles:
`#[entity_event(propagate = PointerTraversal, auto_propagate)]`, where `PointerTraversal` is
"send to the parent if it has one, otherwise the window entity", short-circuited by
`event.pointer().propagate == false`. `PointerEnter`/`PointerLeave` deliberately set
`propagate: false` and compute their own ancestor set from hover-state changes — the DOM
`mouseenter`/`mouseleave` rule rediscovered.

**Verdict for us.** The *hit-test* half is directly applicable and is essentially what we already do.
The *merge* half is a textbook Principle-0/1 violation for this engine: three levels of per-frame
hashing and a `BTreeMap` keyed by a float wrapper, to solve a problem (N heterogeneous picking
backends over M pointers) we do not have. The *propagation* half maps onto machinery this kernel
already owns — see §2.6.

### 2.2 Unity — two different answers in one engine

**uGUI (`EventSystem` + `GraphicRaycaster`).** Each `Canvas` has a raycaster; a raycaster iterates
that canvas's **registered `Graphic` list** (flat, per-canvas — not a tree walk), tests each against
the pointer, sorts the hits by `depth`, and emits `RaycastResult`s. Results across raycasters are
ordered by `sortOrderPriority` / `renderOrderPriority` (canvas sorting layer / render order) then
distance. So the coarse level is the Canvas and the fine level is a flat list — the two-level
structure of §2, item 4.

**Dispatch is a hierarchy walk with pooled lists.** `ExecuteEvents.ExecuteHierarchy` collects the
parent chain and executes the first handler that accepts:

```csharp
GetEventChain(root, s_InternalTransformList);
for (var i = 0; i < s_InternalTransformList.Count; i++) {
    var transform = s_InternalTransformList[i];
    if (Execute(transform.gameObject, eventData, callbackFunction))
        return transform.gameObject;
}
```

and `Execute` fetches handlers with `go.GetComponents(components)` from a `ListPool<Component>`,
casts each to `T`, and `try/catch`es around every invocation. That is: **per-event, per-ancestor,
per-handler dynamic type discovery, virtual dispatch, and exception handling**. It is the exact
opposite of everything this engine's principles ask for, and it is also *why* Unity UI has a
reputation for GC pressure and event cost.

**UI Toolkit (UIElements)** is Unity's second, newer answer, and it is the DOM model: `panel.Pick`
resolves a target by `pickingMode` (`Position` / `Ignore`), then dispatch runs three phases —
**trickle-down** (root → target's parent), **target**, **bubble-up** (target's parent → root) —
with every element on the path receiving the event twice. Materially identical to the browser, and
it carries the browser's cost: a computed propagation path allocated per event, and per-element
callback registries.

**Verdict for us.** uGUI's raycast shape (per-canvas flat list, cross-canvas priority) is sound and
mirrors our `(StackIndex, paint_seq)` total order. Its dispatch is unusable here. UI Toolkit's
three-phase model is the most expressive option on the table and is analysed as Model C in §4.

### 2.3 Godot — `Control` input

**Hit-test is a recursive tree walk with a matrix inverse per node.**
`Viewport::_gui_find_control_at_pos` (`scene/main/viewport.cpp:1855`):

```cpp
Transform2D matrix = p_xform * p_node->get_transform();
if (matrix.determinant() == 0.0f) return nullptr;
Control *c = Object::cast_to<Control>(p_node);
if (!c || !c->is_clipping_contents() || c->has_point(matrix.affine_inverse().xform(p_global))) {
    for (int i = p_node->get_child_count() - 1; i >= 0; i--) { ... recurse ... }
}
if (!c || c->get_mouse_filter_with_override() == Control::MOUSE_FILTER_IGNORE) return nullptr;
matrix.affine_invert();
if (!c->has_point(matrix.xform(p_global))) return nullptr;
return c;
```

Reverse child order, first hit wins, **one `affine_inverse()` per visited node**, and — note this —
the recursion only *descends into* a clipping node if the point is inside it, which is a genuine
subtree rejection. `gui_find_control` iterates the sorted roots back-to-front.

**Dispatch bubbles, transforming the event on the way up** (`viewport.cpp:1750`
`_gui_call_input`): walk `ci = ci->get_parent_item()`, call `_call_gui_input` on each `Control`
whose filter is not `IGNORE`, stop at `MOUSE_FILTER_STOP` (which also calls
`set_input_as_handled()`), and `ev = ev->xformed_by(ci->get_transform())` each hop.

The three filters map cleanly: `IGNORE` = not a candidate; `STOP` = consume and stop bubbling;
`PASS` = handle *and* keep bubbling. Our `FocusPolicy::{Block, Pass}` is a *strictly weaker* thing —
it controls occlusion during resolution, not propagation after it. Worth naming, because the names
collide and will confuse anyone coming from Godot.

**Verdict for us.** The per-node matrix inverse and the `Object*` pointer-chase are exactly what a
data-oriented engine must not copy; but the *policy* (three-valued filter, transform-aware
subtree rejection at clip nodes) is well-designed and worth stealing in a flat form.

### 2.4 Browsers — three separate mechanisms people conflate

**(a) The DOM event model** is capture → target → bubble, with a propagation path computed per
event, `stopPropagation`, and `stopImmediatePropagation`. Expressive; the cost is the per-node
listener registry.

**(b) The hit-test itself** — the interesting part. WebRender does **not** walk a tree. It builds a
`HitTestingScene` once per scene: `items: Vec<HitTestingItem>` where each item is
`{ rect, tag, animation_id, is_backface_visible, spatial_node_index, clip_node_id }`, plus a map of
`HitTestClipNode { region, spatial_node_index, parent }`. The query is a **flat reverse linear
scan** — `for item in self.scene.items.iter().rev()` — with two optimizations that matter:

* the transform inverse is **cached across consecutive items sharing a spatial node**
  (`if item.spatial_node_index != current_spatial_node_index { recompute point_in_layer }`) — i.e.
  the array is *sorted so that coherence pays*;
* clips are a **chained node list walked by id** (`while current_clip_node_id != ClipNodeId::NONE`),
  not an inline rect per item — so a subtree's shared clip is stored once and tested by index.

This is, structurally, the design a DOD engine would arrive at independently: an SoA-ish flat array,
indices instead of embedded data, and coherence-exploiting caching. It ships in Firefox.

**(c) The compositor split** is the real performance lesson. Chromium keeps a *simplified* hit-test
structure on the compositor thread, tracking **non-fast-scrollable regions** — areas that cannot be
scrolled without consulting the main thread. Scrolls that land outside those regions never touch
Blink at all ("fast scroll"); those inside are posted to the main thread ("slow scroll"). The
generalizable rule: **separate the input channels that need the full interaction graph from the ones
that do not.** A wheel over a scroll container needs to know *which container*, not which glyph.

### 2.5 Dear ImGui — the immediate-mode alternative, stated plainly

**How it decides.** Two levels, and the top one is *retained across frames*.
`FindHoveredWindowEx` (`imgui.cpp:6302`) is a reverse linear scan of `g.Windows` — a persistent,
z-ordered array — testing `window->OuterRectClipped.ContainsWithPad(pos, hit_padding)` and skipping
`!WasActive || Hidden`. Then per item, during submission, `ItemHoverable` (`imgui.cpp:5026`):

```cpp
if (g.HoveredWindow != window) return false;
if (!IsMouseHoveringRect(bb.Min, bb.Max)) return false;
if (g.HoveredId != 0 && g.HoveredId != id && !g.HoveredIdAllowOverlap) return false;
if (g.ActiveId != 0 && g.ActiveId != id && !g.ActiveIdAllowOverlap) ... return false;
```

So: *first item submitted at that point wins*, unless it opts into overlap — and the overlap path is
explicitly one frame behind:

```cpp
// AllowOverlap mode (rarely used) requires previous frame HoveredId to be null or to match.
if (item_flags & ImGuiItemFlags_AllowOverlap) {
    g.HoveredIdAllowOverlap = true;
    if (g.HoveredIdPreviousFrame != id) return false;
}
```

There is no z-order resolution among items at all — submission order *is* the hit order, and
correct front-to-back behaviour for overlapping widgets is bought with a frame of latency.

**What immediate mode buys.**

* **No retained per-element state and therefore no synchronization problem.** State lives in the
  caller's own variables; the UI cannot go stale relative to the model, because it is re-derived
  from the model every frame. This is genuinely the single biggest source of bugs it eliminates.
* **Hit-testing fuses into layout and submission** — one pass, no separate tree, no candidate
  array, no z-sort. Excellent cache behaviour for the common case.
* **Identity is a hash, not a pointer** — the `ImGuiID` stack. Duplicate-ID detection is
  "1 u32 compare per item. No O(log N) lookup whatsoever" (`imgui.cpp:5032`).
* **Input ownership is first-class and cheap.** `g.ActiveId` is exactly pointer capture; the
  `SetKeyOwner`/`TestKeyOwner` layer (`imgui.cpp:9548`) generalizes it to *every key*, so a widget
  that owns the mouse button also owns Escape. This is the best-designed piece of ImGui for our
  purposes and it is model-independent.

**What immediate mode costs, and why it is wrong for this engine.**

* **The tree is re-walked every frame in user code.** Not a problem at HUD scale; fatal at
  "10 000 rows" scale without manual clipping, and it makes *the application*, not the framework,
  responsible for the per-frame cost.
* **Layout is one-pass and causal.** You cannot size a container to its content and then align that
  content, because the content is not known when the container is emitted. Retained two-pass layout
  — which `boyko_ui` already has (`layout.rs`, 2340 lines of measure/arrange) — solves problems
  immediate mode structurally cannot.
* **The one-frame delay is not an implementation detail**, it is visible in the source above, and it
  interacts badly with animation and with anything that needs a stable pointer target.
* **It is architecturally opposite to Principle 0.** Immediate mode puts widget state in globals
  (`GImGui`) and application locals; this engine's inviolable rule is that durable per-element data
  lives in ECS columns. An immediate-mode UI is not "ECS-native with a different storage" — it is
  *the parallel data system glued on the side* that Principle 0 exists to forbid. Adopting it would
  be the `std::Vec` physics-mirror mistake (see `docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md`) at UI
  scale.

**Conclusion on immediate mode:** borrow three ideas — (i) `ActiveId`-style **input ownership**
generalized to channels, (ii) **hover dwell timers** as first-class state
(`HoverDelayNormal = 0.40f`, `HoverStationaryDelay = 0.15f`, `imgui.cpp:1584-1586`), (iii)
**`MouseDragThreshold = 6.0f`** as a real, named constant rather than an ad-hoc epsilon
(`imgui.cpp:1733`). Reject the model.

### 2.6 The mechanism this engine already has that the survey usually assumes you lack

`boyko_ecs` ships a complete **observer / custom-trigger** subsystem, and it is Bevy-observer-shaped
without Bevy's costs:

* `Trigger` with `const PROPAGATION: PropagationMode` = `None` / `Up` / `Down` and
  `type Traversal` (`observers/traversal.rs:26`), where `Toward<R>` bubbles along *any*
  `Relationship` and `ChildOfTraversal` is the parent hop.
* `propagate(bool)` to stop a walk, made re-entrancy-safe with a TLS `PropagateGuard`
  (`observers/propagate.rs`) rather than a world field — a deliberate Tree-Borrows decision.
* Entity-targeted observers stored as **real `unsafe fn` pointers** in a tagged union
  (`EntityRunner::{Lifecycle, Custom}`, `observers/entity_store.rs:61`) — no `Box<dyn Fn>`, and the
  fn-ptr is kept typed specifically so strict-provenance/Miri-TB stays sound.
* A **sticky `ArchetypeFlags::HAS_ENTITY_OBSERVER` gate**: "entities with no entity-targeted
  observer are ABSENT from the store and pay nothing" (`entity_store.rs:5-11`). Capability by
  structural absence, in the kernel, already.
* A dense `TriggerId` mint packed with lifecycle kinds into one `u32` `DispatchKey`
  (`observers/dispatch_key.rs`) — no `HashMap` on the dispatch path.
* API: `world.trigger::<E>(target, event)` (`ecs_master/observer_api.rs:449`), which needs
  `&mut EcsMaster` — and `ui_focus_system` is already exclusive.

**This is decisive for §4:** the "bubbling costs you a `dyn`-heavy listener registry" objection,
which is true of Unity uGUI and of the DOM, is **not true here**. Bubbling in this engine costs a
fn-pointer call per hop plus one `get_component::<ChildOf>` per hop, gated by a sticky archetype bit
that makes non-observed subtrees free.

---

## 3. Comparative table

| | Hit-test structure | Order key | Early-out | Dispatch | Capture | Per-frame/per-event allocation | Fits ECS+DOD? |
|---|---|---|---|---|---|---|---|
| **boyko_ui today** | flat `Vec<Candidate>` rebuilt every frame from a DFS with ~6 random `get_component` per node | `(StackIndex, paint_seq, Entity)` max | **no** (scans all N) | flat, single target, no propagation | **no** | none (retained scratch) | yes, but the collect pass is random-access |
| **Bevy** | flat `UiStack` reverse scan | render order, reverse | yes (`should_block_lower`) | observer triggers, bubble up `ChildOf`, `auto_propagate` | drag state in resources | `HashMap`+`BTreeMap`+`sort_by_key(FloatOrd)` per pointer per frame | hit-test yes; merge no |
| **Unity uGUI** | per-Canvas flat `Graphic` list; canvases prioritized | `depth`, then canvas sortOrder/renderOrder | partial (blocking objects) | `ExecuteHierarchy` up the transform chain, `GetComponents` per node, try/catch per handler | `pointerPress`/`pointerDrag` pinned on `PointerEventData` | pooled lists, still per-event reflection | no |
| **Unity UI Toolkit** | `panel.Pick` tree walk by `pickingMode` | tree order | yes | trickle-down / target / bubble-up (each element twice) | pointer capture API | propagation path per event | no |
| **Godot** | recursive DFS, reverse child order, `affine_inverse()` per node, subtree rejection at clip nodes | tree order + sorted roots | **yes**, first hit wins | bubble to parents, event transformed per hop, `MOUSE_FILTER_STOP` consumes | `gui.mouse_focus` | per-event `Ref<InputEvent>` xform allocs | no |
| **Browser / WebRender** | flat `Vec<HitTestingItem>` reverse scan; clips as an indexed chain; transform inverse cached per spatial node | display-list order, reverse | yes | capture/target/bubble | explicit + implicit (touch) pointer capture | scene built once, reused across frames via `Arc` | **hit-test: yes, closest match** |
| **Dear ImGui** | reverse scan of retained `g.Windows`, then rect test at submission | submission order | yes (first wins) | none (caller *is* the handler) | `ActiveId` + per-key ownership | none | model no; ownership yes |

Reading the table left to right: **everyone converged on the same hit-test shape** (a flat,
z-ordered array scanned front-to-back with early-out) and **diverged on dispatch**. The
divergence is not about performance, it is about who writes the handler.

---

## 4. Three candidate models for `boyko_ui`

### Model A — flat capability dispatch (today's model, completed)

One resolve pass produces one primary target per pointer. Every interaction behaviour is an ordinary
ECS system reading the components that mark its capability: `ui_drag_system` over entities with
`Draggable` + `UiPressed`, `ui_scroll_system` over `ScrollContainer` + a resolved wheel target,
`ui_text_input_system` over `TextInput` + `UiFocused`. No event travels anywhere; state is columns.

* **For:** maximally ECS-native; every system is a normal query over dense columns; no dispatch
  machinery at all; trivially parallel except for the exclusive resolve; capability is component
  presence by construction.
* **Against:** the *routing* question is unanswered. A wheel event lands on a `Label` inside a
  `ScrollView`; a click lands on an icon inside a `Button`. Somebody must decide the wheel belongs
  to the ancestor. Without propagation, that decision has to be made *at resolve time*, which means
  the resolve pass must know about channels.

### Model B — flat resolve + kernel-trigger bubbling

The resolve pass fires `world.trigger::<UiPointerDown>(target, ev)` with
`PROPAGATION = Up` / `Traversal = ChildOfTraversal`; author code attaches entity observers; an
ancestor sees the event unless a descendant called `propagate(false)`. This is Bevy's model built on
machinery this kernel already ships (§2.6).

* **For:** composability without a `dyn` registry (fn-ptr runners); the "unhandled by the child,
  handled by the parent" idiom every UI author expects; zero cost for subtrees with no observers
  (sticky archetype bit).
* **Against:** observers run **inline inside an exclusive system**, mutating the world mid-walk —
  the ordering and re-entrancy hazards are real and the kernel's own docs are full of the fixes
  (`OBS-FIRE-LOOP`, `PropagateGuard`, W9/F2/F3). It also re-introduces "logic lives in callbacks"
  as the idiomatic path, which is exactly what `OnClick(u16)` was designed to avoid
  (`interaction/action.rs:1-9`).

### Model C — retained handler tree with capture + target + bubble

The full DOM / UI Toolkit model: three phases, per-node handler lists, a computed propagation path
per event.

* **For:** the most expressive; the model every web/app developer already knows; capture phase
  enables clean modal/overlay interception ("a modal eats all pointer events below it" is one line).
* **Against:** **per-node handler lists are the Principle 0 violation.** A `Vec<Handler>` or a
  `HashMap<EventType, Vec<Box<dyn Fn>>>` per node is a parallel data system glued on the side, plus
  `Box<dyn>` on the hot path, plus per-event path allocation. It could be made ECS-native (handlers
  as entity-targeted observers, path as a scratch buffer) — at which point it *is* Model B plus a
  capture phase, and the capture phase alone does not justify the second walk.

---

## 5. Mechanism by mechanism

Each subsection: what the references do → what would fight this engine → the ECS-native shape.

### 5.1 Hit-testing: the resolve pass

**The references converged (§3): flat, z-ordered, reverse scan, early-out.** We have the flat array
and the total order; we lack the early-out and we pay a random-access collect.

Three concrete improvements, in descending confidence:

**(i) Early-out at the first blocking hit.** `resolve_hovered` (`focus.rs:301`) scans all N keeping
the max. If `candidates` were built in *descending* total order instead of ascending paint order, the
first hit that is `FocusPolicy::Block` terminates the scan — Bevy, Godot, WebRender and ImGui all do
this. Expected effect: for an opaque HUD, the scan terminates in single-digit iterations instead of
N. (Estimate; the *worst* case is unchanged.)

Caveat that must be respected: `write_interactions` (`focus.rs:340`) deliberately performs an
**unconditional** pass over every candidate so a node occluded this frame is still reset to `None`.
Early-out applies to *resolution*, never to the reset — this is already documented at
`interaction/components.rs:46-49` and is a correctness invariant, not an optimization target.

**(ii) Split the candidate record; index the clips.** The scan reads only `rect` and `clip`. Today
it strides 64 B/node (§1.4b). WebRender's answer is exactly the right one: keep the rect inline and
the **clip as an index into a small table**, because clips are shared by whole subtrees. ECS-native
shape:

```text
UiInteractionScratch {
    hit_x: Vec<f32>, hit_y: Vec<f32>, hit_w: Vec<f32>, hit_h: Vec<f32>,  // 16 B/node, SIMD-ready
    hit_key: Vec<u64>,        // (stack_index << 32) | paint_seq — the sort key, one compare
    hit_clip: Vec<u16>,       // index into clip_rects; u16::MAX = unclipped
    hit_flags: Vec<u8>,       // Block, focusable, ...
    hit_entity: Vec<Entity>,  // touched only for the winner
    clip_rects: Vec<[f32; 4]>,// deduplicated per frame; a handful of entries, L1-resident
}
```

The hot loop then touches 16 B/node instead of 64 B — a 1000-node UI's scan working set drops from
~64 KB to ~16 KB, i.e. from "misses L1" to "fits L1" (arithmetic, not a measurement). And the four
parallel `f32` arrays are AVX2-shaped: eight rects per compare, producing a mask, with `hit_key`
consulted only for set lanes. This is the AVX2-baseline SoA layout Principle 6 asks for, and it is
the one place in the interaction system where SIMD is honestly applicable.

**(iii) Do not rebuild the candidate set when nothing changed.** The collect pass is the real cost
(§1.4b: ~6 random-access probes × N × every frame). `ComputedRect` is written set-if-changed by
layout (`components.rs:167`), so `Changed<ComputedRect>`, `Changed<ComputedClip>`,
`Added/Removed<Interaction>`, hierarchy edits and `UiViewport.generation` form a complete dirty
term. A clean still frame then costs **one scan of a cached SoA array** and zero component probes.
This is the browser lesson (§2.4c) in ECS form: the interaction graph is only rebuilt when the
interaction graph changes.

**Transforms.** Godot inverse-transforms the point per node; WebRender caches the inverse per
spatial node because consecutive items share one. `boyko_ui` has **no UI transform at all** today, so
the axis-aligned compare is currently exact and free. If the sprites/animation half of this campaign
introduces a rotating/scaling UI node (a spinner, a card flip, a scale-on-hover button), the
WebRender shape is the one to copy and **not** the Godot one: a small per-frame table of distinct
2D affines with an index per node (`hit_xform: Vec<u16>`), the inverse computed once per table
entry, and the point transformed once per distinct transform rather than once per node. A per-node
`Mat3` inverse in the hot loop would be the mistake.

Note the render-side coupling: the GPU clip is an **axis-aligned AABB** in `UiInstance`
(`ui/instance.rs:40`). A rotated node inside a clip would hit-test against a rotated rect but be
*drawn* clipped by an AABB. Either the transform feature is restricted to unclipped subtrees, or
`UiInstance` grows a clip transform — an eDSL change and a `.spv` re-bless, per the shader rules.

### 5.2 Capture / input ownership — the single largest lever

Every reference has it, under a different name:

| System | Name | Semantics |
|---|---|---|
| Browser | `setPointerCapture` / implicit capture | "the target captures all subsequent pointer events as if they were occurring over the capturing target"; `pointerover/enter/leave/out` do not fire while set. For touch and pen it is **automatic on `pointerdown`** and released on `pointerup`/`pointercancel` |
| ImGui | `g.ActiveId` (+ `SetKeyOwner`/`TestKeyOwner`) | one owner per input; generalized per key so the active widget owns Escape too |
| Godot | `gui.mouse_focus` | the pressed control keeps receiving motion |
| Unity uGUI | `PointerEventData.pointerPress` / `pointerDrag` | pinned at press, drag events go there regardless of what is under the cursor |
| Bevy | drag state in the picking plugin | `PointerDragStart`/`Drag`/`DragEnd` target the press origin |

**Why it matters here specifically:** while a pointer is captured, the resolve pass is *not needed*.
Dragging a slider, scrolling with a held scrollbar thumb, and selecting text are precisely the
high-frame-rate interactions, and they are exactly the ones where the target is already known. This
converts the most performance-sensitive interaction from "N-candidate scan every frame" to "one
component read". It is the only true O(1) answer in the whole survey.

**We are one field away from it.** `PointerSlot` (`focus.rs:42`) already stores
`pending_click: Option<(Entity, u16)>` — a press-origin stamp. Capture is that field promoted to a
first-class owner:

```text
PointerSlot {
    owner: Option<Entity>,      // captured target; None = resolve normally
    owned_channels: u8,         // bitmask: Move | Wheel | Keys — the ImGui SetKeyOwner idea
    pending_click: Option<(Entity, u16)>,
    click_fired: Option<(Entity, u16)>,
}
```

with the resolve pass short-circuiting when `owner.is_some()`, and blur/despawn releasing it (the
existing `blur_reset` at `focus.rs:588` is already the right site — it already cancels pending
clicks, so it becomes "cancel captures" too). **Generation safety is already solved** the same way
`resolve_pointer` solves it: `Entity` equality includes the generation, so a recycled slot cannot
masquerade as the owner (`focus.rs:487-490` documents this exact argument).

**Named risk:** capture is the classic source of stuck-input bugs (`OpenSeadragon#1962` is a
long-running WebKit implicit-capture bug). The engine's existing defence — the unconditional
blur reset on `!cursor_inside || !window_focused` (Decision 12, `focus.rs:161`) — must extend to
capture, and a captured entity that despawns must release, or the UI wedges permanently. This is the
same "no stale set bit" class the world-pick module already reasons about explicitly
(`world/pick.rs:13-21`).

### 5.3 Propagation: bubbling vs flat dispatch

**What the references buy with bubbling.** Exactly one thing that a flat model cannot express
cheaply: *"the child did not handle it; let the ancestor decide"*, without the ancestor enumerating
its descendants. That idiom shows up in four real places: a click on a `Button`'s `Label` child, a
wheel over anything inside a `ScrollView`, a key press inside a focused panel, and a modal overlay
swallowing everything below it.

**What it costs, per system.** Unity uGUI: `GetComponents` + cast + try/catch per ancestor per
event. DOM/UI Toolkit: a materialized path and two traversals. Godot: a `Ref<InputEvent>` transformed
per hop. Bevy: an observer trigger per hop. **This engine:** a fn-pointer call and one
`get_component::<ChildOf>` per hop, skipped entirely for archetypes that never registered an
observer.

**The honest distinction the survey exposes:** three of those four idioms are not *propagation*
problems at all, they are **routing** problems, and routing is resolvable at hit-test time:

* click on a Button's Label — solved by not making the Label a candidate. Capability is component
  presence: a Label with no `Interaction` column is not in `candidates` at all
  (`focus.rs:224`), so the Button *is* the hit. **This already works today.**
* wheel over a ScrollView's content — solved by resolving the wheel channel against the nearest
  ancestor carrying `ScrollContainer`, walking the ancestor chain the DFS already knows.
* keys inside a focused panel — solved by focus, not by pointer propagation.

Only the modal-overlay case genuinely wants a phase (and a `FocusPolicy::Block` full-screen node
already handles the pointer half of it).

**Recommendation (see §9):** **flat routing as the spine, kernel-trigger bubbling as an opt-in
escape hatch.** The default path emits no events and allocates nothing; an author who wants
composable interception registers an entity observer for an `Up`-propagating trigger and pays only
then. Refusing bubbling *entirely* would push authors toward the one thing we most want to avoid —
storing `Box<dyn Fn>` callbacks in their own side tables to fake it.

**Naming hazard to fix:** `FocusPolicy::{Block, Pass}` reads exactly like Godot's
`MOUSE_FILTER_{STOP, PASS}` but means something different (occlusion during resolution, not
propagation after it). If any propagation lands, these two axes must be separately named, or every
user coming from Godot writes the wrong thing.

### 5.4 Drag and drop

**Four models.**

| | Payload | Target discovery | Cancel |
|---|---|---|---|
| Godot | `get_drag_data()` returns a `Variant`, `can_drop_data()`/`drop_data()` on the target; a drag *preview* Control is excluded from hit-testing (`viewport.cpp:1891`) | hit-test each frame, ask the target and then its ancestors | Esc / release outside |
| Unity uGUI | `PointerEventData.pointerDrag` pinned at press; `IBeginDrag/IDrag/IEndDrag` on the source, `IDropHandler` on whatever is under the pointer at release | pinned source + live hit-test | — |
| HTML5 DnD | `DataTransfer`, string MIME payloads; the notorious "must `preventDefault` on `dragover` to allow a drop" | live hit-test, `dragenter`/`dragover`/`dragleave` | `dragend` |
| ImGui | typed payload copied into a fixed context buffer; `BeginDragDropSource` requires `g.ActiveId == source_id` (`imgui.cpp:15083`) and the source is *excluded from hover* while dragging (`imgui.cpp:5065`) | `BeginDragDropTarget` at submission | release |

**Common invariants worth keeping:** a drag threshold before a press becomes a drag (ImGui:
`MouseDragThreshold = 6.0f`, `imgui.cpp:1733`; Unity: `EventSystem.pixelDragThreshold`); the source
is excluded from its own hit-test while dragging (Godot **and** ImGui do this explicitly); the drag
preview follows the pointer and must not be pickable.

**What fights Principle 0.** Every one of these stores the payload in a subsystem-owned box: a
`Variant`, a `DataTransfer`, an untyped context buffer. A `Box<dyn Any>` drag payload — or a
`HashMap<DragId, Payload>` side store — is the violation.

**ECS-native shape.** The payload is an entity. Drag state is columns:

* `Draggable { threshold_px: f32, channels: u8 }` — presence = capability.
* `DragActive { origin: Entity, grab_offset: [f32; 2], started_at: [f32; 2] }` — added by the drag
  system when the threshold is exceeded, removed on drop. **Presence is the "am I dragging" bit**;
  no boolean flag (Capability rule).
* The **payload is whatever components the dragged entity already carries** — an inventory item
  drags because it *is* an `ItemStack` entity. No serialization, no type erasure, no `Any`. A
  `DropTarget { accepts_mask: u32 }` and a `ComponentId`-based accept test (or, better, an
  `EnableTag` for the accepted class) replaces MIME strings; the mask is checked against the dragged
  entity's archetype in O(1).
* The drag *preview* is a normal UI entity spawned with `StackIndex = u32::MAX` and **without**
  `Interaction` — so it is structurally absent from the candidate set, which is the ECS-native way
  to say "the preview is not pickable" (Godot needs an explicit ancestor test for this,
  `viewport.cpp:1891`).
* Capture (§5.2) owns the pointer for the drag's whole life, so the drag costs **zero** hit-tests
  per frame; the *drop target* still needs one hit-test per frame — or, cheaper, only when the
  pointer moves more than a threshold.

### 5.5 Scrolling: momentum and clipping

**Clipping.** This engine's clip is per-instance and free (§1.4a) — a real advantage over every
reference, all of which break batches or push scissor state. What is missing is that `ComputedClip`
is **author-owned, never computed** (`components.rs:188-190`). A scroll container needs
`ComputedClip` *derived* from an overflow policy and intersected down the subtree, exactly as Bevy
derives `CalculatedClip` during rendering and as the layout pass here already propagates rects.
Concretely: an `Overflow { x: Visible|Clip|Scroll, y: ... }` component read by `ui_layout_apply`,
which then writes `ComputedClip` set-if-changed for the subtree. That is a layout-pass change, not
an interaction change, but interaction depends on it because `point_in_clip` (`focus.rs:280`) is
already the correct hit-test gate and will start doing real work the moment clips are computed.

**Scroll offset.** Bevy's `ScrollPosition` is the right shape: a component on the container whose
value offsets the children's positions at layout time, clamped to content size. ECS-native and
already how `bar_fill_system` drives a child's size from a parent's `UiValue`.

**Momentum.** The formula everyone actually uses is exponential velocity decay, and UIKit's is the
most precisely specified: `decelerationRate` "indicates how much the scroll velocity will change
after one millisecond" — `normal = 0.998`, `fast = 0.99` — so `v(t) = v₀ · rate^(t_ms)`. That is
frame-rate independent by construction:

```text
v *= rate.powf(dt_secs * 1000.0);
offset += v * dt_secs;
if v.abs() < STOP_EPS { remove ScrollMomentum; }   // capability disappears structurally
```

Chromium/Edge tune the same idea with a custom curve and add *fling boosting* (successive flings
compound velocity). Android's `Scroller` uses a friction constant plus a deceleration-rate exponent
rather than a per-ms rate — equivalent family, different parameterization.

**ECS-native shape:** `ScrollMomentum { vel: [f32; 2] }` **added** when a drag/fling releases with
velocity and **removed** when it decays below epsilon — so the per-frame integrate system queries an
archetype that is *empty* when nothing is coasting. That is capability-by-presence doing real work:
a still UI runs the momentum system over zero rows.

**The routing question this raises:** which container consumes the wheel? The Chromium lesson
(§2.4c) is that this is a *separate, cheaper* question than full hit-testing — the answer is a
container, not a leaf. Concretely: resolve the wheel channel by walking up from the hovered node to
the nearest ancestor carrying `ScrollContainer` with a non-clamped axis. The DFS in
`collect_candidates` already knows every node's parent chain; recording a bounded ancestor snapshot
(depth ≤ 16, a fixed array — no `Vec` per node) makes this a ≤16-step array walk with no tree
traversal. Godot needed a special case for exactly this and added `force_pass_scroll_events`
(`viewport.cpp:1775`) — evidence that "the wheel routes differently from the click" is a real,
recurring requirement, not a nicety.

### 5.6 Text input, carets, selection, IME

**This is the largest genuinely-missing piece, and it is missing at the OS layer, not just in the
UI.**

The seam is a **dead datum** in the exact sense this project has catalogued before: `RawInputEvent`
declares `Text(char)` with the comment "*A logical character produced under the active layout — text
fields only... Kept distinct from `Key` so IME/composition stays out of the physical-binding path*"
(`boyko_input/src/raw/event.rs:26-29`) — a correct design decision — but

* the Win32 translator produces no `WM_CHAR` (`boyko_input/src/win32.rs` handles `WM_KEYDOWN/UP`,
  `WM_SYSKEYDOWN/UP`, mouse, wheel, `WM_INPUT`, `WM_SIZE`, `WM_PAINT`, `WM_QUIT` — and nothing else),
  and
* the ingest explicitly discards it: `RawInputEvent::Text(_) => {}` (`raw/queue.rs:302`).

So the variant exists, is unreachable from the OS, and is a no-op if it ever arrives. **Nothing in
the engine can currently receive a typed character.** Any text-input plan must start here, and the
plan should treat "the seam exists" as *false* until a test drives a `WM_CHAR` end-to-end.

**Why a game engine cannot skip IME.** East-Asian input goes through a composition (preedit) phase:
the user types keys, an IME shows a candidate window, and only on commit does text appear. Without
IME support the field silently receives the raw Latin keystrokes. This engine owns its Win32
`WndProc` (`crates/boyko_app/src/runner.rs:974` calls `translate_win32(msg, wparam, lparam)`), so
there is no winit to inherit it from — the two options are:

* **IMM32** — handle `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` (read `GCS_COMPSTR` for the
  in-progress string, `GCS_RESULTSTR` for the committed one via `ImmGetCompositionStringW`) /
  `WM_IME_ENDCOMPOSITION`, and position the candidate window with `ImmSetCompositionWindow`
  (`CFS_POINT`, default style) so it tracks the caret. Microsoft's own game guidance
  (*Using an Input Method Editor in a Game*, and DXUT's `CDXUTIMEEditBox`) is the IMM32 route.
  Simpler; adequate; what most engines ship.
* **TSF (Text Services Framework)** — richer (inline reconversion, handwriting, better Win10+
  behaviour), substantially more work, and requires implementing COM interfaces.

**Recommendation: IMM32, with the preedit modelled as engine data.** The winit `Ime` enum
(`Enabled` / `Preedit(String, Option<(usize, usize)>)` / `Commit(String)` / `Disabled`) is the right
*shape* to copy even though we are not using winit — it is the minimal cross-platform vocabulary,
and it makes the Linux path later a translation rather than a redesign. Note winit's own rule:
"during the preedit phase the window will NOT get `KeyboardInput` events" and IME is off by default
because it is wrong for gameplay — which maps exactly onto capability-by-presence here: **IME is
enabled iff a focused entity carries `TextInput`**, and the focus system calls
`ImmAssociateContext`/`set_ime_allowed`-equivalent on the `UiFocused` transition it already
computes (`focus.rs:547-550`).

**Storage — the Principle 0 question.** A text field's buffer, caret, selection anchor, and preedit
are durable per-element data. `String` per widget in a side map is the violation. ECS-native shape:

* `TextInput { cap: u16, flags: u16 }` — capability marker (presence = "this is an editable
  field").
* The **buffer as a dense component**, not a `String`: the codebase already has the precedent in
  `UiName` — a fixed inline `[u8; 60]` + `len`, one cache line, POD `Copy`, "no interner and no
  global string table — Principle 1/5" (`components.rs:264-274`). A single-line field is the same
  pattern at a larger cap; a multi-line field is the case for a `Resource`-owned rope column
  (`docs/DENSE-COMPONENTS-PLAN.md` describes exactly the dense-storage kind this wants), never a
  per-widget `String`.
* `TextCursor { anchor: u32, head: u32, affinity: u8 }` — selection is an anchor/head pair
  (empty selection = caret), which is what every editor uses and what makes shift-click and
  drag-select the same code path.
* `TextPreedit { bytes: [u8; N], len: u16, cursor: (u16, u16) }` — **added** while composing and
  **removed** on commit. Structural presence again: the renderer draws the underline only for
  entities that have the column, so there is no "is_composing" flag to get stale.

**Caret placement from a click** needs a hit-test *inside* shaped text — a `x → byte index` query
against the shaped run. The existing pipeline (`src/text/shape.rs`, `measure.rs`) has the advance
data; the query is a binary search over cluster advances, and the two correctness traps are
well-known: **never split a grapheme cluster** (a caret between the base and the combining mark is a
bug), and **caret affinity at a soft line break** (the same byte index is both end-of-line-N and
start-of-line-N+1 — hence the `affinity` byte above). Selection *rendering* is a set of rects per
line, which is already exactly what the instanced quad path draws.

### 5.7 Keyboard navigation and focus rings

**Today:** Tab cycles a linearly sorted list `(tab_index, paint_seq, Entity)` (`focus.rs:516-566`).
No Shift+Tab, no groups, no arrows.

**References:**

* **Bevy** split this into a crate (`bevy_input_focus`): `InputFocus` resource, `TabIndex` where
  "index ≥ 0 means tabbable, order by index; index < 0 means not sequentially focusable but still
  directly focusable", and `TabGroup { order }` so tabbables must be descendants of a group. That is
  a strictly better model than a flat list, and it is a two-key sort — cheap.
* **Unity** does *directional* navigation with a scoring function worth quoting because it is small
  and it works (`Selectable.FindSelectable`):

  ```csharp
  Vector3 myVector = sel.transform.TransformPoint(selCenter) - pos;
  float dot = Vector3.Dot(dir, myVector);
  if (dot <= 0) continue;
  float score = dot / myVector.sqrMagnitude;
  if (score > maxScore) { maxScore = score; bestPick = sel; }
  ```

  i.e. *maximize (alignment with the direction) / (distance²)*, starting from the point on the
  current rect's **edge** in that direction, not its center. For a gamepad/arrow-key UI this is the
  whole algorithm.
* **Godot** offers explicit `focus_neighbor_*` overrides plus an automatic fallback — the lesson
  being that automatic directional navigation always needs a manual override escape hatch.
* **Browsers** separate `:focus` from `:focus-visible` — the ring is shown for keyboard-driven
  focus and hidden for mouse-driven focus. This is a real UX requirement, not decoration.

**ECS-native shape.** `Focusable { tab_index }` grows into `Focusable { tab_index: i32 }` (negative =
directly-focusable-only, per Bevy) plus an optional `FocusGroup { order: u32 }` on ancestors, and
directional navigation is the Unity scoring loop over the **same SoA rect arrays the hit-test
already builds** (§5.1) — one pass, no tree walk, no extra storage. `FocusNeighbors { up, down,
left, right: Entity }` is the opt-in override component (absent = automatic).

**The focus ring** should be `UiFocused` (the EnableTag that already exists) driving a *render*
concern. Two candidate implementations: (a) an extra `UiInstance` emitted for focused nodes with an
outset rect and a border — no shader change at all, since border + corner radius already exist; or
(b) a `flags` bit and an eDSL change. **(a) is strictly preferable** — it needs zero shader work,
zero `.spv` re-bless, and it composes with the existing z-sort. `:focus-visible` becomes a bit set
by the focus system when focus moved *by keyboard* — one boolean in `UiInputFocus`, not a component.

### 5.8 Gestures

Two competing designs, and the difference is philosophical.

* **UIKit / Android: recognizer state machines with an explicit dependency graph.** Each recognizer
  is `Possible → Began → Changed → Ended/Failed/Cancelled`, and conflicts are resolved by declared
  dependencies (`requireGestureRecognizerToFail:`). Deterministic; verbose; every new pair of
  gestures may need a new declaration.
* **Flutter: the `GestureArena`.** Every recognizer interested in a pointer joins an arena; "at any
  time, a recognizer can eliminate itself and leave the arena. If there's only one recognizer left,
  that recognizer wins. At any time, a recognizer can declare itself the winner, causing all the
  remaining recognizers to lose." `sweep()` forces resolution after `PointerUp` so passive
  recognizers cannot deadlock the app; `hold()`/`release()` let a recognizer (double-tap) keep the
  arena open past the up event.

The arena is the more elegant answer to *tap-vs-drag-vs-long-press on the same element*, which is the
only conflict a game UI actually has at first. But an arena is a per-pointer dynamic set of
participants — in a `dyn`-free engine that means a fixed-size participant array per pointer slot and
a `u8` bitmask of recognizer kinds, not a `Vec<Box<dyn GestureArenaMember>>`.

**Recommendation: defer the arena; ship a fixed recognizer ladder first.** With `MAX_POINTERS = 1`
and a mouse, the entire conflict set is {click, drag, long-press, double-click} and it resolves with
two thresholds and two timers — the ImGui constants are the right starting values
(`MouseDragThreshold = 6.0f`, `HoverStationaryDelay = 0.15f`, `HoverDelayNormal = 0.40f`). Encode it
as a `PointerGesture` state enum on the pointer slot, not per element. Revisit the arena when touch
lands, because that is when the conflict set becomes open-ended (pinch vs pan vs two-finger scroll)
and a hard-coded ladder stops scaling — and note that the arena is *also* the natural place to
express "the ScrollView wins the vertical drag, the Slider wins the horizontal one", which the
ladder cannot express at all.

### 5.9 Tooltips

The mechanism is a **dwell timer plus a stationarity test**, and ImGui states both constants
explicitly (`imgui.cpp:1584-1586`): `HoverStationaryDelay = 0.15f` ("time required to consider mouse
stationary"), `HoverDelayShort = 0.15f`, `HoverDelayNormal = 0.40f`. The stationarity test is the
part naive implementations miss: a pointer sweeping across a toolbar should not fire six tooltips.

`hover_entered` (`focus.rs:128`) gives the enter edge but no dwell. ECS-native shape:
`Tooltip { text: ..., delay_ms: u16 }` as the capability component; a `HoverDwell { ms: u16 }`
column **added on hover-enter and removed on hover-exit** so the accumulate system iterates only
currently-hovered nodes (typically one row); the tooltip entity itself spawned with
`StackIndex = u32::MAX` and no `Interaction` — structurally unpickable, same trick as the drag
preview (§5.4).

### 5.10 Multi-pointer / touch

`MAX_POINTERS = 1` with a fixed `[PointerSlot; MAX_POINTERS]` array (`focus.rs:38-62`) is already the
right shape — the constant is the only thing that changes. Two things to fix *before* it changes,
because they are cheap now and invasive later:

1. `resolve_pointer` hardcodes `slots[0]` (`focus.rs:471`) and `ui_dispatch_system` reads
   `slots[0].click_fired` (`dispatch.rs:44`). Both should loop.
2. The browser rule is worth adopting verbatim: **touch gets implicit capture on down** ("for touch
   and stylus inputs, pointer capture is automatically set on the target element whenever there is a
   `pointerdown` event", released on up/cancel). This is not a nicety — without it, a finger that
   drags off a button re-targets mid-gesture and every touch UI feels broken. §5.2's `owner` field
   is the mechanism; touch just sets it unconditionally.

---

## 6. Aether DSL integration

The Aether plan (`docs/AETHER-LANG-PLAN.md`) already names `ui!` as its precedent ("*the scene
construct is `ui!`'s proven shape generalized to 3D render objects*", §3.7) and its `scene` grammar
is `node_head` + `node_body` with a `children: [...]` prop and an `EXPR` fallback for arbitrary
component literals. A `ui` construct is therefore a sibling of `scene`, not a new mechanism, and
interaction slots into `node_body` as props. Four requirements fall out of this research:

1. **Interaction props are component presence, not callbacks.** `on click: Action::Confirm` must
   lower to `OnClick(Action::Confirm.index())` — a `u16` resolved at expansion time — never to a
   closure. The existing `interaction/action.rs` design ("*NOT a generic `OnClick<A>`... an integer
   is the reflection-free common denominator*") is what makes this possible and is exactly why it
   was designed that way. `on hover` / `on submit` follow.
2. **Capability props emit marker components.** `draggable`, `scroll y`, `focusable(3)`,
   `tooltip: "..."` are bare keywords or single-value props that emit `Draggable`, `Overflow`,
   `Focusable { tab_index: 3 }`, `Tooltip` — presence, not booleans. A prop that is *absent* must
   emit *nothing*, so the archetype genuinely lacks the column (this is the difference between
   capability-by-presence and a struct of flags, and the DSL is where it is easiest to get wrong).
3. **Cross-construct references must reach the action enum.** The plan's `AetherCtx` already carries
   cross-construct names (a `system` names a sibling for `after`, a `scene` names a sibling
   `material`). `on click: Confirm` resolving against a sibling-declared action enum is the same
   machinery; a misspelled action must be a compile error with a did-you-mean, **never** the
   `NO_ACTION` sentinel silently — the sentinel exists for the `.ui` hot-reload path where a
   compile-time error is impossible, and blurring the two would turn a typo into a dead button.
4. **`ui` and `.ui` must stay equivalent.** The `p3_equivalence` / `p6a_equivalence` test pairs
   already pin macro-vs-text equivalence; every interaction prop added to Aether needs the same
   pin, or the two authoring paths drift.

Open DSL question for the owner: does an Aether `ui` construct *replace* `ui!` or lower to it? The
plan's principle 2 ("Builds ON `boyko_macros`, never bypasses it") argues for lowering.

---

## 7. Patterns that are standard elsewhere and must not be copied here

| Pattern | Where it is standard | Why it fights this engine | ECS-native replacement |
|---|---|---|---|
| `Box<dyn Fn>` / interface-based handlers per node | Unity uGUI (`IPointerClickHandler`), DOM listeners, most Rust UI crates | `dyn` + `Box` on the hot path (Principle 1); a parallel per-node data store (Principle 0) | dense `u16` action index (`OnClick(u16)`), already shipped; entity observers with fn-ptr runners for the escape hatch |
| `HashMap<PointerId, …>` merge maps | `bevy_picking::hover` (`OverMap`, `HoverMap`, `PreviousHoverMap`) | `HashMap` is clippy-banned here (`clippy.toml` `disallowed-types`); per-frame hashing for a fixed, tiny pointer count | fixed `[PointerSlot; MAX_POINTERS]` array — **already the design** (`focus.rs:55`) |
| Per-event propagation path allocation | DOM, UI Toolkit, Unity `GetEventChain` | per-event `Vec`; `Vec::new()` on the hot path | bounded ancestor snapshot (`[Entity; 16]`) in the retained scratch resource |
| Per-node matrix inverse in the hit loop | Godot `_gui_find_control_at_pos` | one `affine_inverse()` per visited node per frame | per-frame transform table + index (WebRender `spatial_node_index`), inverse computed once per distinct transform |
| Widget state in a widget struct | ImGui (`GImGui` globals), retained toolkits (`self.scroll_offset`) | the Principle 0 violation by definition — a subsystem-local data model | components: `ScrollPosition`, `ScrollMomentum`, `DragActive`, `TextCursor` |
| `String` / `Variant` / `Box<dyn Any>` payloads | HTML5 `DataTransfer`, Godot `get_drag_data`, ImGui payload buffer | heap + type erasure per interaction | the payload **is** the dragged entity; acceptance by `ComponentId`/EnableTag mask |
| A boolean `is_dragging` / `is_composing` flag | nearly everywhere | runtime flag where structural absence is available | presence of `DragActive` / `TextPreedit`; runtime on/off is the EnableTag bit |
| Immediate-mode re-submission | Dear ImGui | application-owned per-frame cost; one-pass layout; state outside the ECS | retained tree + ECS columns (what we have); borrow only `ActiveId`-style ownership and the dwell/threshold constants |
| Hand-editing HLSL for a focus ring / rounded clip | most engines | the eDSL owns the UI shader; `.spv` is byte-gated | reuse the existing border+radius instance fields, or extend `boyko_shaderdsl` and re-splice between sentinels |

---

## 8. Open questions for the owner (also to be filed in `docs/OPEN-QUESTIONS.md`)

1. **Scope of IME.** IMM32 (simpler, adequate, Windows-only for now) or TSF (richer, COM, much more
   work)? And is East-Asian text input in scope for this campaign at all, or is Latin-only text entry
   the deliverable with IME deferred? This is a VALUES/SCOPE call, not a perf fork.
2. **UI transforms.** Do sprites/animation require rotated or scaled UI nodes? If yes, the
   axis-aligned GPU clip (`UiInstance.clip`, `ui/instance.rs:40`) becomes wrong inside clipped
   subtrees and needs an eDSL change + `.spv` re-bless. If no, the hit-test stays a pure AABB compare
   and this whole class of complexity disappears.
3. **Touch.** Is `MAX_POINTERS > 1` in scope? It changes the gesture recommendation (§5.8: ladder vs
   arena) and makes implicit capture mandatory rather than optional.
4. **Bubbling.** Should author-facing interaction *ever* be callback-shaped (entity observers on
   `Up`-propagating triggers), or must every interaction stay a component + system? §9 recommends
   "opt-in escape hatch"; the owner may prefer "never", which is defensible and simpler.

---

## 9. Recommendation

**Adopt Model A′ = flat SoA resolve + first-class capture + capability routing, with kernel-trigger
bubbling as an opt-in escape hatch.**

Concretely, in the order the value lands:

1. **Capture** (`PointerSlot.owner` + `owned_channels`). The only O(1) answer in the survey; turns
   drag, scrollbar-thumb, and text-selection from N-scans-per-frame into zero. Smallest change,
   largest effect, and the field is already half-present as `pending_click`.
2. **Dirty-gated candidate rebuild.** The collect pass (~6 random-access component probes × N ×
   every frame) is the actual per-frame cost, and `Changed<ComputedRect>` + hierarchy edits +
   `UiViewport.generation` is a complete dirty term. A still frame should cost one array scan.
3. **SoA candidate layout + early-out + indexed clip table.** WebRender's shape, AVX2-friendly,
   arithmetically ~16 KB instead of ~64 KB of scan working set per 1000 nodes.
4. **Capability routing on the bounded ancestor snapshot** — the wheel goes to the nearest
   `ScrollContainer`, the drop to the nearest `DropTarget`. A ≤16-entry array walk, no tree
   traversal, no event dispatch. This answers three of the four idioms that motivate bubbling
   (§5.3).
5. **Bubbling only where an author asks for it**, on the kernel's existing `Trigger` +
   `PropagationMode::Up` + `ChildOfTraversal` machinery — fn-ptr runners, sticky archetype gate,
   `propagate(false)` to stop. Zero cost when unused.
6. Everything else as components: `DragActive`, `ScrollPosition`, `ScrollMomentum`, `TextCursor`,
   `TextPreedit`, `HoverDwell`, `FocusGroup` — each **added and removed** so that its system's query
   is empty when the interaction is not happening.

Why this and not the alternatives: Model C's per-node handler tree is a Principle 0 violation with a
poor exchange rate (it buys a capture phase we can approximate with a `Block` overlay). Model B as
the *default* would make callbacks the idiomatic path and undo the deliberate reflection-free
`OnClick(u16)` design. Model A alone leaves routing unanswered, and routing is the thing that
actually breaks (Godot needed `force_pass_scroll_events` for exactly this).

### The strongest argument against this recommendation

**Routing-instead-of-bubbling makes the target of an event depend on which components happen to
exist on which ancestors, decided inside an exclusive system the author cannot get between.**

Under full propagation, an author interposes behaviour by attaching a handler — locally, at the node
they care about, without knowing what the resolve pass thinks. Under capability routing, an ancestor
that wants to see an event it was not routed must *acquire a component*, and a node that wants to
stop an event that was routed past it has no mechanism at all. "Handle it in the parent unless the
child consumed it" — the single most common composition idiom in every retained UI ever shipped —
is not expressible in the default path; it is only expressible by opting into the escape hatch,
which means the escape hatch will become the common path, and then we will have paid for two
mechanisms and standardized on the one we called exceptional.

Every retained UI in the survey — DOM, UI Toolkit, Godot, Unity uGUI, and Bevy in its most recent
redesign — independently chose full propagation. Five independent convergences is weak evidence
about performance and *strong* evidence about expressiveness: they did not all choose it by
accident, and app UI's composition needs are not obviously narrower than a game HUD's once the HUD
grows an inventory grid, a dialogue tree, and a settings menu with nested scroll views.

And the failure is **not local**. The resolve pass's output shape under this recommendation is *one
primary target per pointer plus a bounded ancestor snapshot*; under propagation it is *a path*.
Every state machine keyed on "the target" (drag, scroll, tooltip, gesture) would have to be re-keyed
on "the path". Discovering the need late means rewriting the interaction spine, not extending it.

**A second, narrower counter-argument, aimed at item 3:** the SoA/SIMD hit-test may be optimizing
the wrong half. A game HUD plausibly has N in the low hundreds, where a 64 B AoS scan is already
microseconds; the measured cost is far more likely to be the random-access component probes in
`collect_candidates`, which SIMD does not touch — item 2 addresses that and item 3 may be
unmeasurable on top of it. This is not a reason to reject the design (the SoA layout is also what
directional focus navigation in §5.7 wants), but item 3 should be **gated behind a measurement**,
not shipped on the strength of the arithmetic in §1.4b. This project has a documented history of
gates that could not fail and of numbers that were not measurable; a 64 KB-vs-16 KB argument is a
hypothesis until a bench with a realistic N says otherwise.

---

## 10. Sources

**This repository** (branch `feat/ui-advanced`, worktree `D:/wt/ui`)

* `crates/boyko_ui/src/interaction/{focus.rs, dispatch.rs, action.rs, components.rs, plugin.rs}`
* `crates/boyko_ui/src/components.rs`, `src/layout.rs`, `src/world/pick.rs`, `src/resources.rs`
* `crates/boyko_input/src/raw/{event.rs, queue.rs}`, `crates/boyko_input/src/win32.rs`
* `crates/boyko_app/src/runner.rs`
* `crates/boyko_render/src/ui/{instance.rs, pack.rs, draw.rs}`
* `crates/boyko_ecs/src/ecs/core/component/observers/{traversal.rs, trigger.rs, propagate.rs, entity_store.rs, dispatch_key.rs}`
* `crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs`
* `docs/AETHER-LANG-PLAN.md`, `docs/DENSE-COMPONENTS-PLAN.md`, `docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md`

**Bevy**

* [`bevy_ui/src/picking_backend.rs`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_ui/src/picking_backend.rs)
* [`bevy_picking/src/hover.rs`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_picking/src/hover.rs)
* [`bevy_picking/src/events.rs`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_picking/src/events.rs)
* [`bevy_picking/src/backend.rs`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_picking/src/backend.rs) · [`HitData` docs](https://docs.rs/bevy/latest/bevy/picking/backend/struct.HitData.html)
* [`bevy_input_focus` — tab navigation](https://docs.rs/bevy/latest/bevy/input_focus/tab_navigation/index.html) · [PR #16795](https://github.com/bevyengine/bevy/pull/16795)
* [`ScrollPosition`](https://docs.rs/bevy/latest/bevy/ui/struct.ScrollPosition.html) · [PR #20093 improved UI scrolling](https://github.com/bevyengine/bevy/pull/20093)
* [`bevy_mod_picking` (the predecessor)](https://github.com/aevyrie/bevy_mod_picking)

**Unity**

* [`ExecuteEvents.cs` (uGUI)](https://github.com/Pinkuburu/Unity-Technologies-ui/blob/master/UnityEngine.UI/EventSystem/ExecuteEvents.cs)
* [`Selectable.cs` — `FindSelectable`](https://github.com/liuqiaosz/Unity/blob/master/UGUI%E6%BA%90%E4%BB%A3%E7%A0%81/UnityEngine.UI/UI/Core/Selectable.cs)
* [uGUI raycasting system overview](https://deepwiki.com/Unity-Technologies/uGUI/2.2-raycasting-system) · [event system overview](https://deepwiki.com/Unity-Technologies/uGUI/2.1-event-system)
* [`GraphicRaycaster.Raycast`](https://docs.unity3d.com/2019.1/Documentation/ScriptReference/UI.GraphicRaycaster.Raycast.html) · [`EventSystem.pixelDragThreshold`](https://docs.unity3d.com/550/Documentation/ScriptReference/EventSystems.EventSystem-pixelDragThreshold.html)
* [UI Toolkit — Dispatch events (trickle-down / target / bubble-up)](https://docs.unity3d.com/Manual/UIE-Events-Dispatching.html) · [Handling events / pickingMode](https://docs.unity3d.com/6000.4/Documentation/Manual/UIE-Events-Handling.html)

**Godot**

* [`scene/main/viewport.cpp`](https://github.com/godotengine/godot/blob/master/scene/main/viewport.cpp) — `gui_find_control` (l. 1827), `_gui_find_control_at_pos` (l. 1855), `_gui_call_input` (l. 1750), `force_pass_scroll_events` (l. 1775)
* [Proposal #3613 — renaming the mouse filter modes](https://github.com/godotengine/godot-proposals/issues/3613) (the STOP/PASS/IGNORE semantics discussion)

**Browsers**

* [WebRender `hit_test.rs`](https://github.com/servo/webrender/blob/main/webrender/src/hit_test.rs) · [docs](https://doc.servo.org/webrender/hit_test/index.html) · [`CLIPPING_AND_POSITIONING.md`](https://github.com/servo/webrender/blob/main/webrender/doc/CLIPPING_AND_POSITIONING.md)
* [Chromium — Compositor Thread Architecture](https://www.chromium.org/developers/design-documents/compositor-thread-architecture/) · [Compositor (Touch) Hit Testing](https://www.chromium.org/developers/design-documents/compositor-hit-testing/)
* [W3C Pointer Events (implicit capture, retargeting)](https://www.w3.org/TR/pointerevents2/) · [MDN `setPointerCapture`](https://developer.mozilla.org/en-US/docs/Web/API/Element/setPointerCapture)
* [Inside look at modern web browser, part 4 (input)](https://developer.chrome.com/blog/inside-browser-part4)

**Dear ImGui**

* [`imgui.cpp`](https://github.com/ocornut/imgui/blob/master/imgui.cpp) — `ItemHoverable` (l. 5026), `FindHoveredWindowEx` (l. 6302), `BeginDragDropSource` (l. 15065), style constants (l. 1584-1586, 1733), key-ownership section (l. 9548)
* [Widget system overview](https://deepwiki.com/ocornut/imgui/2.3-widget-system)

**Flutter**

* [`gestures/arena.dart`](https://github.com/flutter/flutter/blob/main/packages/flutter/lib/src/gestures/arena.dart) · [`GestureArenaManager`](https://api.flutter.dev/flutter/gestures/GestureArenaManager-class.html) · [`sweep`](https://api.flutter.dev/flutter/gestures/GestureArenaManager/sweep.html) · [Taps, drags, and other gestures](https://docs.flutter.dev/ui/interactivity/gestures)

**Text input / IME / scrolling physics**

* [Processing the `WM_IME_COMPOSITION` Message](https://learn.microsoft.com/en-us/windows/win32/intl/processing-the-wm-ime-composition-message) · [Status, Composition, and Candidates Windows](https://learn.microsoft.com/en-us/windows/win32/intl/status--composition--and-candidates-windows) · [Using an Input Method Editor in a Game](https://learn.microsoft.com/en-us/windows/win32/dxtecharts/using-an-input-method-editor-in-a-game)
* [winit `Ime` event](https://docs.rs/winit/latest/winit/event/enum.Ime.html) · [the commit that added it](https://github.com/rust-windowing/winit/commit/f04fa5d54f4ec10cdb6d084deeb79d3e6d27ae67)
* [`UIScrollView.decelerationRate`](https://developer.apple.com/documentation/uikit/uiscrollview/decelerationrate) · [Deceleration mechanics of UIScrollView](https://medium.com/@esskeetit/scrolling-mechanics-of-uiscrollview-142adee1142c)
* [Scrolling personality improvements in Microsoft Edge](https://blogs.windows.com/msedgedev/2020/04/02/scrolling-personality-improvements/) · [Smooth scrolling in Chrome 49](https://developer.chrome.com/blog/smooth-scrolling-in-chrome-49)
