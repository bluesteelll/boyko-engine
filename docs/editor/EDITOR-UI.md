# U1–U6 — The substrate: what the editor is drawn with, what has never been drawn, and what it is driven by

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 5.**
**Revision 5** rewrote §4's reason 2 as history: the `serialize_ui` loss it rested on was repaired
2026-09-03, and the reason's own membership list had been wrong before that too — it named
`UiBackground`, which the `.ui` format has never accepted, and omitted `BarFill`, which it does.
Decisions `U1`–`U6`. Code claims are read at HEAD `59009f8a`; the ones revision 5 added are read in
the WORKING TREE at that HEAD, where the `.ui` vocabulary repair lives uncommitted.

| § | decision |
|---|---|
| §1 | `U1` — the substrate gap, and why it is rung `V0` |
| §2 | `U2` — the v1 widget set |
| §3 | `U3` — the panel layout, and the viewport |
| §4 | `U4` — editor chrome is Rust systems, not a document |
| §5 | `U5` — how a machine addresses a widget |
| §6 | **`U6` (new) — how the editor gets input at all, and `EK17`** |
| §7 | alternatives, priced |
| §8 | out of scope, and what this file could not establish |

---

## 1. The substrate gap — `U1`, and why it is rung `V0`

**Decision. The production window loop must draw one `boyko_ui` panel with validation ON, and the
gate must be shown capable of going RED. This is rung `V0`, and nothing in `V1`–`V8` is designed on
the UI until it is green.**

The campaign's single most load-bearing assumption has never been exercised. It is not one gap but
two, and revision 2 saw only the first.

### 1.1 Gap one: the UI is not rendered by the production host

Established three independent ways at this checkout.

1. **No production dependency.** `crates/boyko_app/Cargo.toml:17-42` lists `boyko-ecs`,
   `boyko-scene`, `boyko-render`, `boyko_rhi`, `boyko_rhi_vulkan`, `boyko-input`, `boyko-math`,
   `boyko_sdf_math`, `boyko-macros`, `boyko-diag`, `boyko-log`. There is no `boyko-ui`.
2. **`boyko_render`'s edge is TEST-ONLY.** `boyko-ui` appears at
   `crates/boyko_render/Cargo.toml:113`, inside `[dev-dependencies]` (which begins at `:78`), under a
   comment reading "GUI P6b screenshot test ONLY … Acyclic + TEST-ONLY".
3. **The production draw call has no UI parameter.** The only function taking `ui: Option<&UiPass<'_>>`
   is `Renderer::present_sampled`
   (`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:696,706`), and its only two callers in the
   whole tree are tests: `crates/boyko_render/tests/ui_rect_swapchain_golden.rs:639` and
   `crates/boyko_rhi_vulkan/tests/window_present_hybrid.rs:978`. What `run_windowed` calls is
   `render_gbuffer_frame` (`frame_driver.rs:821`), which has no `ui` argument.

*(Revision 2 said there was ONE test caller. There are two. Both are tests, so the conclusion stands;
the count was wrong and is corrected here and in the index.)*

### 1.2 Gap two: the plugin that installs the UI does not exist

Revision 2's `V0` listed "a real `UiPlugin` added by the host" as one of four deliverables. There is
no such plugin.

- `UiPlugin::build` returns immediately when its `path` is `None`, and with a path it reads a `.ui`
  file, adds ONE startup system that parses and spawns the tree, and registers ONE frame system,
  `ui_hot_reload_system` (`crates/boyko_ui/src/plugin.rs:84-127`). It registers **no layout, no
  measure, no hit-test.** It is the `.ui` document loader and hot-reloader, and nothing else.
- **Nothing in `crates/*/src` registers `ui_layout_discovery` or `ui_layout_apply`.** Every
  occurrence outside `crates/boyko_ui/src/layout.rs` is a doc comment. The only registration in the
  tree is a test harness: `crates/boyko_ui/tests/common/mod.rs:96-105` builds a schedule of
  `[ui_layout_discovery, ui_layout_apply]` with the apply pinned after the discovery.
- The crate states the ordering as **the host's responsibility**, twice: "Schedule them in that
  order, after all structural/prop-mutation systems" (`crates/boyko_ui/src/lib.rs:19-21`), and
  "`ui_text_measure_system` MUST be registered `.before(ui_layout_discovery)` … Like the layout pair,
  the ORDER is the host's responsibility (P5b ships the system, not an App schedule); a host that
  registers them out of order …" (`crates/boyko_ui/src/text/measure.rs:25-30`).
- The other plugins each cover a slice and none covers layout: `UiWidgetsPlugin`
  (`crates/boyko_ui/src/widgets.rs:257-267`, the `Bar` driver), `UiInteractionPlugin<A>` and
  `UiBindingPlugin` (`crates/boyko_ui/src/interaction/plugin.rs:64,147`), `ProfilingOverlayPlugin`
  (`crates/boyko_ui/src/profiling_overlay.rs:205-213`, which registers exactly one system and returns
  early with no `Profiler`).

So `V0`'s largest deliverable is not "add the existing plugin" — it is **author the plugin that does
not exist**, and the ordering contract it must encode currently lives in prose plus a test harness.

### 1.3 What already exists, and is more than the gap suggests

This is a WIRING rung, not a subsystem rung, and the distinction is what keeps it sized at M.

- The layout and interaction ENGINE is real and adequate: a flexbox-shaped solver
  (`crates/boyko_ui/src/layout.rs`, ~2340 lines) over `Row`/`Column`/`Overlay`/`Grid` and
  `Px`/`Pct`/`Stretch`/`Auto` (`crates/boyko_ui/src/units.rs`), `ComputedClip` clipping, MSDF text
  with a measure→layout feedback edge, a total-Z-order hit test with `FocusPolicy::Block` and
  cross-root tab order (`crates/boyko_ui/src/interaction/focus.rs`), and the `ChildOf`/`Children`
  hierarchy as the tree.
- `crates/boyko_render/src/ui/` is already a **production** module (not a dev-only one) whose own
  docs describe the intended sub-pass: "`LoadOp::Load` at the full swapchain extent in
  `present_sampled` and the `record_ui_rects` …" (`crates/boyko_render/src/ui/mod.rs:55`), with
  `pack_ui_instance` and the ring upload written against `present_sampled`'s contract
  (`crates/boyko_render/src/ui/upload.rs:31-33,181,197`).
- The GPU side has been proven — against an offscreen target and one swapchain golden. What has never
  run is the production frame's version of it.

### 1.4 What `V0` delivers, precisely

`EK8`, five pieces:

1. **`UiRuntimePlugin`, in `boyko_ui`** — the crate that owns the ordering contract, so the contract
   lives in code beside the prose rather than in a host that could get it wrong. It inserts
   `UiViewport`, `LayoutScratch::with_seeds()` and `FontTable`, and registers, in this order (the
   order the crate's own docs and its test harness already state):

   | step | system | why here |
   |---|---|---|
   | 1 | `ui_bar_discovery`, `ui_bar_apply` | the widget drivers must run BEFORE the layout discovery so the fill's `Unit::Pct` change is seen the same frame (`crates/boyko_ui/src/widgets.rs:250-253`) |
   | 2 | `ui_text_measure_system` | `.before(ui_layout_discovery)` so the same-frame relayout sees the new `ContentSize` (`crates/boyko_ui/src/text/measure.rs:25-30`) |
   | 3 | `ui_layout_discovery` | the change-detection half; sets the `dirty` flag (`crates/boyko_ui/src/lib.rs:11-14`) |
   | 4 | `ui_layout_apply` | exclusive; re-lays-out the root subtrees (`lib.rs:15-18`) |
   | 5 | `ui_bind_discovery`, `ui_bind_apply` | the data-binding pair |

   **Revision 4 moved the interaction pair OUT of this plugin**, into a second one:

   | plugin | systems | rung | why separate |
   |---|---|---|---|
   | `UiRuntimePlugin` | steps 1–5 above | `V0` | pure draw. Depends on no input resource, so `V0`'s harness needs no input plugin |
   | `UiInputPlugin` | `ui_focus_system` → `ui_command_dispatch` (`EK15`) | `V5` | `ui_focus_system` reads `world.resource::<PhysicalInput>()` and **panics if it is absent** (`crates/boyko_ui/src/interaction/focus.rs:145-151`). That resource is inserted only by `InputPlugin<A>` today, and by `EK17`'s `RawInputPlugin` after `V5` — see §6. Ordered after `ui_layout_apply`, because the hit-test reads `ComputedRect` |

   Revision 3 listed `ui_focus_system` as step 5 of `UiRuntimePlugin`, which would have made `V0` —
   the rung whose whole purpose is to prove the draw path — panic at boot in its own harness unless
   the harness quietly added a game's input plugin, at which point `V0` would have been proving
   something other than what it says. Splitting costs one plugin value and removes the coupling.

   `UiPlugin` (the `.ui` loader) is a SEPARATE, optional plugin and the editor does not use it (§4).

2. **Host viewport seeding.** `UiViewport { width, height, scale_factor, generation }` is documented
   as "set by the host (window/swapchain) on surface create and resize"
   (`crates/boyko_ui/src/resources.rs:19-37`), and every `UiRoot` is seeded from it
   (`components.rs:251-258`). The runner writes it at boot and on every resize, bumping `generation`
   so the layout's viewport-resize trigger fires.
3. **A `ui: Option<&UiPass>` parameter threaded into `render_gbuffer_frame`** (or a UI sub-pass
   recorded after the composite, whichever the implementer measures as the smaller diff — this is a
   wiring choice, not a design fork).
4. **An ECS→ring upload system** in `boyko_render::ui` over the existing `pack_ui_instance`.
5. **The production `boyko_app → boyko-ui` dependency edge.** Acyclic: `boyko-ui` names no render
   crate (its deps are ecs/macros/utils/input/fontbake/scene/math, stated at
   `crates/boyko_render/Cargo.toml:110-112`), so `boyko_app → boyko-ui` introduces no cycle.

This is shared with Gaia `G7`'s prerequisite, so it is paid once.

### 1.5 The hazard, and why the gate is validation-on with TWO red-first calibrations

The barrier/layout interaction between the G-buffer composite and a `LoadOp::Load` UI pass has never
been exercised on this path, and **the instrument that would catch a missing barrier is documented as
not live**: a deliberately removed barrier produces zero `SYNC-HAZARD` output and an unchanged
golden. So the obvious gate — a golden — cannot see the defect class this rung exists to find.

Hence:

- **Calibration A (the barrier).** Remove the UI pass's barrier and the `V0` run MUST go red. If it
  does not, the gate is declared non-measuring and the fallback is a `vkCmdPipelineBarrier` census
  over the recorded command stream.
- **Calibration B (the pipeline).** Delete `ui_layout_apply` from `UiRuntimePlugin` and the golden
  MUST change. A pipeline that never laid anything out must not be able to produce the expected
  image — this calibration exists because gap two is exactly the kind that produces a plausible frame
  from an incomplete pipeline (a panel with a default `ComputedRect` still draws *something*).
- The validation-message count is asserted zero **and printed**, so a run with the layer disabled
  cannot read as a pass.
- The UI pass's frame time is REPORTED, not gated: the machine is a workstation, not a bench rig, and
  the number's job is to give `V5` a baseline rather than to be a threshold nobody calibrated.

---

## 2. The v1 widget set — `U2`

**Decision. Three new widgets only: an editable text field, scrolling, and a tree.**

### 2.1 What ships today

Seven bundles — `UiNodeBundle`, `PanelBundle`, `ButtonBundle`, `LabelBundle`, `ImageBundle`,
`GridBundle`, `BarBundle` (`crates/boyko_ui/src/lib.rs` prelude, `bundles`) — plus the interaction
components `Interaction`, `Focusable`, `FocusPolicy`, `RelativeCursorPosition`. The entire widget
DRIVER module is the `Bar` (`crates/boyko_ui/src/widgets.rs`).

### 2.2 The three, each with the seam already open under it

| widget | why v1 needs it | what already exists |
|---|---|---|
| **editable text field** | to type a field value into the inspector — without it the inspector is read-only and `V5`'s chain cannot close | `RawInputEvent::Text(char)` is produced by the Win32 adapter and explicitly DISCARDED by `PhysicalInput` ("Text is for text fields, never gameplay"), with a test pinning the discard; keyboard focus, tab order and the Enter edge already exist (`focus.rs`); `UiTextBuffer` already exists as the bind target. The work is caret, selection and the edit commands — plus `EK12` for the commit edge (`EDITOR-COMMANDS.md` §9.4) |
| **scrolling** | to see more than a screenful of components in the inspector or entities in the hierarchy | the wheel is already accumulated in `PhysicalInput`; `ComputedClip` already clips. A grep for scroll/wheel over `crates/boyko_ui/src/` returns exactly ONE hit and it is a comment naming `UiScroll` as a future component (`crates/boyko_ui/src/reload/reconcile.rs:49`). The work is a scroll offset in the solver plus wheel routing |
| **tree** | the scene hierarchy panel | it is a `Children` walk plus a collapse marker; the hierarchy is already there and `UiTreeView`'s own walk (`crates/boyko_ui/src/reload/tree_view.rs:73-80`) is the shape |

Each is components plus systems over the shipped solver, hit-test and clip — Principle 0 working as
intended. The `MAX_POINTERS = 1` array and the deliberately-ignored `Text(char)` variant show the
seams were left open on purpose.

### 2.3 What is cut, and why cutting it is honest

Docking, splitters, drag-and-drop, context menus, dropdowns, sliders, checkboxes. Each is L; none
gates a v1 demonstration; and **growing the widget set IS the campaign rather than a prerequisite of
it**. A v1 that can select an entity, read its fields, type a new value and see the object move is a
v1 that exists and can be driven. A v1 with docking and no inspector is neither.

---

## 3. The panel layout, and the viewport — `U3`

**Decision. ONE fixed layout, built from the shipped units, with the viewport as a HOLE.**

```
+--------------------------------------------------------------+
| toolbar   (Row, Px 32)                                        |
+-------------------+--------------------------+---------------+
| hierarchy         |  viewport (the HOLE)      | inspector     |
| (Column, Px 260)  |  (Stretch)                | (Column,      |
|                   |                           |   Px 320)     |
+-------------------+--------------------------+---------------+
| console  (Row, Px 120)                                        |
+--------------------------------------------------------------+
```

Every unit in that diagram is shipped (`Px`, `Stretch`, `Row`, `Column`,
`crates/boyko_ui/src/units.rs`). No docking, no splitters, no saved layout — those need drag and a
persistence story, and both are v2.

**The viewport is a HOLE, not a render target.** The scene is drawn by the ordinary
`render_gbuffer_frame` path over the whole swapchain; the editor's UI is drawn OVER it by `EK8`'s
sub-pass, and the "viewport" is simply the rectangle the chrome does not cover. This is what avoids
needing a per-root `UiViewport` or a nested render target (Godot's `SubViewport`) in v1: there is one
viewport because there is one scene and one window, and the play world is a different process.

**The camera.** The viewport is navigated by `editor_camera_system` in `boyko_editor`, which calls
the identical pure `fly_step` (`crates/boyko_scene/src/camera.rs:895`, made `pub` by `EK13`) driven
by `Time::real_delta()` rather than the virtual delta — because the edit World's virtual clock is
paused and `fly_camera_system`'s own doc says the virtual delta is "ZERO while paused"
(`camera.rs:841-843,879-888`). The editor does not add `FlyCameraPlugin`. The full reasoning and the
`Res<Time>` census are in `EDITOR-BOUNDARY.md` §4.2, and `V5` carries the red-first gate.

**Input arbitration** between chrome and viewport is the existing one: `ui_focus_system`'s hit test
with `FocusPolicy::Block` decides whether the pointer is over chrome; when it is not, the viewport's
camera and the world pick see it. No new mechanism.

---

## 4. Editor chrome is Rust systems, not a document — `U4`

**Decision. The editor's panels are spawned and maintained by Rust systems in `boyko_editor`. They
are NOT `.ui` documents.**

Three independent reasons, any one sufficient:

1. **`.ui` is scheduled for deletion.** Gaia ruling `F6` migrates the format into Gaia's `ui` profile
   and DELETES it in the same campaign. Authoring the editor's chrome in a format with a scheduled
   end is buying a migration.
2. **`serialize_ui` WAS lossy and its gate WAS green over the loss — repaired 2026-09-03, and the
   reason no longer stands on its own.** ⚠️ Read as history; the two claims below are what this
   reason asserted when it was written, and both have moved.

   *What was true.* Until 2026-09-03 the writer emitted 8 of the 20 non-exempt components its own
   parser accepts — its whole component universe was the import list at
   `crates/boyko_ui/src/text/serialize.rs:21-23`, HEAD `59009f8a` — dropping every `UiText`,
   `UiImage`, `UiGrid`, `UiAnchor`, `Button`, `Bar`, `BarFill`, `OnClick`, `OnHover`, `OnSubmit`,
   `BindText` and `BindValue`. The round-trip gate stayed green because
   `serialize → parse → serialize` is byte-identical when both sides are equally lossy, and the
   ceiling was structural: `LiveNode` carried fields for exactly those 8.

   *This list was itself rotted, which is the point.* The version above named `UiBackground`, which
   the `.ui` format has never accepted (zero occurrences under `crates/boyko_ui/src/text/`; it is
   authorable only from `ui!` / Rust), and omitted `BarFill`, which it does. A document arguing that
   hand-maintained lists rot carried one.

   *What is true now* (in the working tree at HEAD `59009f8a`). The vocabulary is ONE declaration
   (`crates/boyko_ui/src/text/vocab.rs:163-205`) that generates the enum, the roster and each
   member's writer and reload policy; the writer's `write_attached`
   (`crates/boyko_ui/src/text/serialize.rs:111-230`) and the reload patcher's `patch_member`
   (`crates/boyko_ui/src/reload/reconcile.rs:488-561`) both match on it EXHAUSTIVELY, so a 22nd
   member is an `E0004` in all three consumers; `LiveNode` carries the whole vocabulary
   (`crates/boyko_ui/src/reload/tree_view.rs:48-87`); and two world-to-world gates
   (`tests/p3_world_round_trip.rs`, `tests/p3_reload_patch_vocabulary.rs`) catch an arm that exists
   but does nothing, which exhaustiveness cannot see.

   *What this reason rests on now.* Not much: "an editor that saved its own layout through it would
   silently destroy it" is no longer true of the writer. The residual loss is one documented format
   limitation — a bind `source` outside the document is written as a raw entity id and reloads at
   generation 0 (`serialize_ui`'s `write_bind_source`), which needs a grammar that can spell a
   generation. `U4` therefore rests on reasons 1 and 3, each of which the decision already declares
   sufficient on its own; this reason is kept because the campaign's other files cite it and because
   the measured history — a list that rotted for as long as nobody re-read it, under a green gate —
   is the evidence `EDITOR-COMMANDS.md` §7 uses to choose a coverage census over an equality check.
3. **The inspector cannot be a static document.** Its rows depend on the selected entity's archetype
   and on each component's `FieldTable`, both known only at runtime.

The chrome is therefore built with `Commands` at startup and reconciled by editor systems. `OnCommand`
(`EDITOR-COMMANDS.md` §9) is attached in Rust, where the `CommandId` is resolved from the registry
rather than parsed from text — which also removes the `.ui` action-name resolution hazard entirely.

*(The `.ui` hot-reload machinery is not wasted: it is the engine's working answer to reloading a
document into a live world while preserving runtime state, and it is what Gaia generalises. It is
simply not the editor's chrome authoring path. It carried the SAME defect in a third direction until
2026-09-03 — the reconcile patched 10 of the 21 members and ignored ten of the eleven it left out
(only `ComputedRect` was deliberate), so a `Button`'s action or a `UiText`'s colour edited in a live
file was silently ignored on a node that survived the reload — and it is now driven
from the same one declaration, `crates/boyko_ui/src/reload/reconcile.rs:488-561`.)*

---

## 5. How a machine addresses a widget — `U5`

**Decision. By `UiName`. Geometry is read from `ui.tree`. Activation is `ui.click(name)`, which goes
through `OnCommand` and the command layer. NEVER by synthesising a pointer event at a coordinate.**

### 5.1 Why not synthetic pointer events

Three reasons, in order of weight.

1. **Measured.** On OSWorld — 369 real tasks on a real OS — the best reported model reached 12.24 %
   against 72.36 % human, and the failure analysis attributes **over 75 % of failures to mouse-click
   inaccuracy**: "strong planning capabilities but weak execution precision". Performance dropped
   60–80 % when window positions, sizes or clutter changed. An editor whose only machine interface is
   synthesised clicks inherits that failure rate for reasons that have nothing to do with this
   engine's quality.
2. **It would break dev builds.** `RAW_QUEUE_CAP = 1024`
   (`crates/boyko_input/src/constants.rs:8`) with drop-oldest and a `debug_assert!(false)` on the
   first eviction: a burst-injecting agent panics every debug build. The drop-oldest policy would
   also make a recording unfaithful (`EDITOR-REPLAY.md` §4).
3. **It tests the wrong thing.** A synthetic click exercises hit-testing; an agent wants to exercise
   the COMMAND. Keeping the two apart means a hit-test regression does not masquerade as an agent
   failure.

Synthetic pointer events are KEPT for hit-test tests, which is what they are for, and `input.key` is
kept for the camera gate (which genuinely is testing input).

### 5.2 The mechanism, re-derived from `OnCommand`

`ui.click(name)`:

1. resolve `name` through `UiName` — a 60-byte inline, one-cache-line stable key
   (`crates/boyko_ui/src/components.rs:272-287`) — to an entity; a miss is `E1902`;
2. read `OnCommand` on that entity; its absence is a coded refusal, asserted by `V5`'s gate;
3. materialise the argument bytes from the node's `ArgSource` exactly as `ui_command_dispatch` does;
4. enqueue the SAME `CommandEnvelope`.

So the agent's click and the human's click are **the same call by construction**. Revision 2 claimed
this and had no carrier for it; `EDITOR-COMMANDS.md` §9 is the carrier, and §9.3 records the fact
that made it cheap — `resolve_pointer` already stamps the click ORIGIN regardless of whether the node
carries `OnClick` (`crates/boyko_ui/src/interaction/focus.rs:475-500`), so the click path needs no
change to `ui_focus_system` at all.

`ui.type(name, text)` writes the node's `UiTextBuffer` and fires its submit provenance; `ui.focus`
and `ui.scroll` are the same shape.

### 5.3 `ui.tree` — a SECOND view over an existing walk

An agent needs geometry to reason about what is where. `UiTreeView::build(world, roots)` already
walks down from document roots via `Children` and snapshots each node's `parent`, `children`, `name`,
`layout`, `spacing`, `align`, `absolute`, `content_size`, `stack_index`, `clip`
(`crates/boyko_ui/src/reload/tree_view.rs:33-80`). It deliberately does **not** carry `ComputedRect`
— "layout output, excluded from the patch set, so it is not snapshotted" (`:47-48`) — and that
exclusion was made for a reason: the reconcile must not write layout output back.

`ui.tree` is therefore a **second view over the same walk**, which ADDS `ComputedRect` and `UiName`
and omits the reload-only `UiSourceOrder`. The reconcile's view is untouched. The walk is a cold path
(`tree_view.rs` says so) and `ui.tree` is a command, so it runs in the exclusive window like any
other.

**This is the engine's structural advantage over the field's usual answer.** Figma — also
canvas-rendered — had to BUILD an accessibility tree: an internal cache with surgical updates, a
React "Mirror DOM" of invisible parallel elements positioned by affine transforms, bidirectional
selection sync, ARIA live regions, and work its own engineers describe as "never really finished".
This engine needs no parallel tree because the tree is already ECS data, and `ui.tree` is a read over
it. That is Principle 0 paying for itself on a requirement it was not written for.

---

## 6. How the editor gets input at all — `U6`, kernel item `EK17`

**Decision. `EK17` splits `boyko_input`'s non-generic half into `RawInputPlugin`
(`RawInputQueue` + `PhysicalInput` + `ingest_physical_input`); `InputPlugin<A>` keeps the
`ActionState<A>` half unchanged. The editor adds `RawInputPlugin` and no `Actionlike` at all.**

### 6.1 The gap, read from the tree

Revision 3 said the editor "does not register `UiInteractionPlugin<A>` at all, which is what removes
the single-`Actionlike` constraint from the editor's path". True — and it removes the editor's input
with it, which nothing in revision 3 noticed. Three facts, each read at this checkout:

1. `RawInputQueue` and `PhysicalInput` are inserted **only** by `InputPlugin<A>::build`
   (`crates/boyko_input/src/plugin.rs:110-140`), which is generic over the game's action enum.
2. The only system that calls `queue.begin_frame()` and drains the ring into the snapshot is
   `update_action_state<A: Actionlike>` (`crates/boyko_input/src/action/process.rs:51-77`) — the
   drain and the per-`A` action fold are one function.
3. `EnginePlugins` is not generic and carries neither (`crates/boyko_app/src/plugins.rs:96,374`).

So an editor App composed as `EnginePlugins + GameTypes + EditorPlugins` has no `PhysicalInput`
resource. `ui_focus_system` unwraps it (`focus.rs:145-151`) ⇒ **panic on the first frame**. Suppose
that were dodged: nothing calls `begin_frame`, so the 1024-slot ring
(`crates/boyko_input/src/constants.rs:8`) fills with mouse motion and trips its own
`debug_assert(false)` on the first eviction — every debug editor session dies within a few seconds of
moving the mouse. Suppose that too were dodged: `EK13`'s camera reads `Res<PhysicalInput>` and would
never see a key, and `V5`'s camera gate would have gone red naming `EK13` as the cause.

The alternative — give the editor its own `Actionlike` enum — is worse and is refused: `ACTION_NAMES`
is write-once and single-`A` (`crates/boyko_input/src/action/names.rs:22-42`), so an editor enum and
a game enum cannot coexist in one process, and `--play` would inherit whichever registered first.
`E2.1` already rejected that surface for the editor's commands; taking it back for the editor's
cursor would be the same mistake in a smaller font.

### 6.2 The split, and why it is behaviour-preserving

One shared `fn install_raw_input(app: &mut App)` called by both plugins, so there is one definition
and no dedup question:

| plugin | inserts | registers |
|---|---|---|
| `RawInputPlugin` (non-generic) | `RawInputQueue::with_capacity(RAW_QUEUE_CAP)`, `PhysicalInput` | `ingest_physical_input` — `queue.begin_frame()`, `physical.begin_frame()`, the drain loop |
| `InputPlugin<A>` (unchanged surface) | the above, plus `ActionState<A>`, `InputMap<A>` | `register_action_names::<A>()`; `clear_consumed_fixed_edges::<A>`; `process_action_state::<A>` — `process_actions` + `freeze_fixed_snapshot` — ordered AFTER the ingest and after the clear |

For a game this is the same systems in the same order doing the same work per frame; the split is
where the code lives, not what it does. That is the property the rung must gate, so `EK17` ships with
one behaviour test: a fixture that presses a key and reads `ActionState<A>` must produce the same
`just_pressed` / `pressed` / frozen-edge state before and after the split, including the 0-substep
sticky-edge case (`BUG-I4-C3`), which is the one the ordering exists for.

### 6.3 Which rung

`V5`, with one caveat stated rather than buried: if `V0`'s windowed harness ends up wanting a live
cursor (for a human to look at the panel while the golden is captured), it needs `RawInputPlugin`
too, and `EK17` moves to `V0`. It is an S-sized split either way and it is listed under `V5` because
that is the first rung that CANNOT ship without it. `V0`'s gate as written needs no input: it dumps a
frame and compares a golden.

### 6.4 What the editor still does not have, and is not getting in v1

No `Actionlike`, therefore no rebindable editor shortcuts and no `.keys` override file for the
editor. Editor key handling in v1 is direct `PhysicalInput` reads inside editor systems — which is
what `ui_focus_system` already does for Tab and Enter (`focus.rs:261`) and what `EK13`'s camera does
for WASD. When editor shortcuts want rebinding, the answer is a second `Actionlike` registry keyed
per World rather than a process-global write-once table — a kernel change, out of scope, recorded in
§7.

---

## 7. Alternatives, priced

| alternative | price | verdict |
|---|---|---|
| Build the editor UI with an external toolkit (Defold's route) | forfeits self-hosting, which is a requirement rather than a preference; and Defold's own retrospective records the cost — the editor "is often the main cause of discomfort", "feels very slow and unresponsive", and Clojure "is a huge barrier for others to contribute" | rejected |
| Grow docking, drag-drop, splitters, menus, dropdowns and sliders first | L each; none gates a demonstration; it is the campaign, not its prerequisite | rejected for v1 |
| Widen `UiTreeView` itself to carry `ComputedRect` | it is excluded by an explicit recorded decision made for a reason (the reconcile's patch set); a second view costs one struct and touches nothing | rejected |
| A per-root `UiViewport` so editor and game UI coexist in one World (Godot's `SubViewport`) | the general fix, and real work; unnecessary once play is a child process (`EDITOR-BOUNDARY.md` §1) | deferred |
| Treat `V0` as a task inside `V5` | it is the assumption everything else stands on; discovering it late costs the whole ladder — and revision 3's own finding (gap two) is evidence that it is bigger than it looks from outside | rejected |
| A golden-only `V0` gate | measured blind to this exact defect class (§1.5) | rejected |

---

## 8. Out of scope, and what this file could not establish

**Out of scope (recorded):** docking, splitters, drag-and-drop, context menus, dropdowns, sliders,
checkboxes; clipboard in the text field (an OS call the engine makes nowhere today); multi-pointer
and gamepad navigation of the editor UI (`MAX_POINTERS = 1` is a fixed array shaped for widening); a
saved or user-arrangeable panel layout (needs docking first); a per-root `UiViewport`; IME and
non-Latin text entry beyond what the MSDF atlas already bakes; theming.

**NOT established:**

- **Whether `boyko_ui` sustains an editor's node count at interactive rates.** No bench exists and
  none was run — the machine is a workstation under load and a number taken now is an artifact.
  Mitigation: `V0` REPORTS the UI pass's frame time so `V5` has a baseline rather than a first
  measurement taken too late to act on. The layout is change-gated with a genuine 0 %-overhead
  still-frame path, which is the right shape for density, but "right shape" is not a number.
- **Whether the ECS→ring upload cost is acceptable at editor densities.** Same reason, same
  mitigation.
- **Whether `render_gbuffer_frame` gaining a parameter or a separate post-composite sub-pass is the
  smaller diff.** A wiring choice for the implementer, to be decided by reading, not by this file.
- **Whether the text field's caret and selection can be expressed without a new per-node component**
  (probably not — expect a `UiTextEdit` component; it is a component, which is the right shape, but
  it is not written).
- **Whether `UiName`'s 60-byte inline is enough for the editor's naming scheme**
  (`"hierarchy/entity-42"`, `"inspector/Transform.translation.x"` — the second is 33 bytes, so
  probably yes, but a deeply nested path could exceed it and `V5` must state what happens then).
