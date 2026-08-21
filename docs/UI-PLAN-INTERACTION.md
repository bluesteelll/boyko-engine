# UI-PLAN-INTERACTION — drag, scroll, text input, keyboard navigation

**Campaign:** advanced UI/GUI for `boyko_ui` · **Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`)
**Date:** 2026-08-21 · **Status:** plan, pre-implementation, rev 1.

**Authority:** [`docs/UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) §6 (D16–D24), with
§3 (D7, D31, D32) and §10 (measurement obligations) binding this ladder from outside.
**Evidence:** [`docs/UI-ADVANCED-RESEARCH-INTERACTION.md`](UI-ADVANCED-RESEARCH-INTERACTION.md);
the other three research documents carry the sprite / animation / DSL halves.
**Siblings:** [`docs/UI-PLAN-SPRITES.md`](UI-PLAN-SPRITES.md),
[`docs/UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md),
[`docs/UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md). §4 names every place they meet this one; nothing they
own is re-specified here.

`graphify` CLI is not installed on this machine; orientation was Grep/Read. Every `path:line` below
was read in this worktree at this date.

---

## 0 · How to read this

This is the **ladder a developer walks**, not a design. The design is the architecture document; when
this plan diverges from it, the divergence is stated at its source as a numbered correction (§2) with
the defect it replaces — never silently substituted, because a plan that quietly disagrees with its
own authority is the doc-rot this project keeps recording.

* **Rungs are `I0…I11`.** Each is independently landable and each leaves the workspace green.
* Every rung states **Lands · Depends on · Default · Gates · RED MUTATION · Size**.
* **A gate whose red nobody has seen is not a gate.** Every rung names one concrete mutation of the
  shipped code and the exact test that must go red under it. This campaign has paid for that lesson
  five recorded times (`site.decode`, `LogSite.fields`, twelve unbuilt benches, `sample_shift=2`,
  `intern_site`), plus the sixth the architecture verified in §1 — and §1 below adds the **seventh
  and eighth**.
* Decisions are numbered **`ID<n>`** to keep them distinct from the architecture's `D<n>`, carry a
  reason, and name what was rejected.
* Where a number matters, the rung says **how it is measured** and §6 holds the obligation.

### Standing gate on every rung

Unchanged from the repo's plan register:

* `cargo clippy -p boyko-ui -p boyko-input -p boyko-rhi-vulkan -p boyko-app --all-targets -- -D warnings`
  clean. (**`-p`, never `--workspace`** — this worktree is disk-bound; an `os error 112` or a
  compiler ICE means `rm -rf target/debug/incremental`, not a code bug.)
* `cargo test -p <crate> --all-targets --no-fail-fast` — **`--no-fail-fast` is load-bearing**; without
  it one known-red target shadows every target ordered behind it.
* All 35 `goldens/PINS.toml` image hashes unchanged, unless the rung's **Default** row says which pin
  moves and why.
* The existing `boyko_ui` interaction corpus stays green: `p4_focus_hittest`, `p4_click_action`,
  `p4_input_seam`, `p4_schedule`, `p4_miri`, `p6a_button_dispatch`, `p3_equivalence`,
  `p3_round_trip`, `zero_alloc`, `world_scratch_zero_alloc`.
* Miri where the rung adds `unsafe`; every `unsafe` carries a `// SAFETY:` comment with concrete
  invariants.
* English in every artifact. Author-only commit; no `Co-Authored-By`.

### Standing invariants this plan may not break

* **G-STANDING-1 — `Interaction` stays set-if-changed.** `write_interactions` writes the column only
  on a genuine transition (`focus.rs:378-395`), which is what makes `Changed<Interaction>` a clean
  edge. The **animation** plan's D14 transition trigger is built on it. Any rung touching
  `write_interactions` re-runs the still-frame tick assertion.
* **G-STANDING-2 — layout is the single writer of computed geometry.** Nothing in this plan writes
  `ComputedRect`. `ComputedClip` gains a writer at **I5**, and that writer is the layout pass itself
  — never an interaction system.
* **G-STANDING-3 — the unconditional reset pass survives.** `write_interactions` deliberately visits
  **every** interactive node so a node occluded this frame is still reset to `None`
  (`interaction/components.rs:44-49`). Capture (I2) and early-out (I11) apply to *resolution* only.

---

## 1 · Verified in-tree state — what exists, what does not

Read in this worktree, 2026-08-21. This is the substrate the ladder stands on.

| Fact | Anchor |
|---|---|
| The interaction spine is one exclusive system, six steps: collect → blur → resolve → write → click → focus | `interaction/focus.rs:145-197` |
| `collect_candidates` is a DFS over `UiRoot`/`Children` carrying an inherited clip on the stack | `focus.rs:204-257` |
| Hover resolution is a **total** order `(StackIndex, paint_seq, Entity)` over a linear candidate scan | `focus.rs:301-330` |
| A candidate requires **both** `ComputedRect` **and** `Interaction`; per node the DFS probes 6 components | `focus.rs:221-234` |
| `PointerSlot` holds `pending_click`/`click_fired` only — **no owner, no press position** | `focus.rs:41-52` |
| `MAX_POINTERS = 1`; slot 0 is hardcoded at three sites | `focus.rs:37`, `focus.rs:471`, `dispatch.rs:44,53` |
| Keyboard: **Tab and Enter, nothing else**; no modifier is ever read | `focus.rs:543,556` |
| `Focusable { tab_index: u32 }` — unsigned, `#[repr(transparent)]` | `interaction/components.rs:63-69` |
| `ComputedClip` is **author-owned**; no system computes it, layout never reads or derives it | `components.rs:186-201` |
| `ScrollPosition` / `Overflow` / `ScrollExtent` / `Draggable` / `TextInput` — **none exist** | crate-wide grep |
| `RelativeCursorPosition` exists, opt-in, set-if-changed with a canonical leave value | `interaction/components.rs:28-42` |
| `UiTextBuffer { bytes: [u8; 247], len: u8 }` — the shipped inline-POD text buffer, tick-bearing | `binding/components.rs:67-79` |
| `shape_into` **streams** glyphs to a sink; retains no advances, emits **no quad for whitespace**, and `ShapedGlyph` carries **no byte index** | `text/shape.rs:48-50,56-121` |
| The query-filter vocabulary is `Added` / `Changed` / `With` / `Without` / `Or`. **`Removed<C>` does not exist** | `boyko_ecs/.../query/filter.rs:513,693,863,1253,1535` |
| The observer backbone does exist: `observe_on_remove`, `observe_entity_event`, `trigger`, `PropagationMode::{None,Up,Down}`, `ChildOfTraversal`, `propagate(bool)`, gated by the sticky `ArchetypeFlags::HAS_ENTITY_OBSERVER` bit | `observer_api.rs:170,322,449`; `observers/traversal.rs:25-40`; `observers/entity_store.rs:5-11` |
| `Time` is reachable from `boyko_ui` with **no new crate edge** (`boyko-ecs` is already a dependency); `delta_secs()` is clamped, speed-scaled and pause-aware | `boyko_ui/Cargo.toml:12`; `time/time.rs:63-75` |
| `LayoutScratch::relayout_count` is `#[cfg(test)]`-only | `layout.rs:168-171,190-194` |
| Hot reload deliberately never writes transient components — *"Transient components (`UiFocus`/`UiScroll`/`UiHover`, P4+) … are NEVER written, so they are preserved by omission"* | `reload/reconcile.rs:49-52` |

### The three dead data this plan inherits

The architecture's §1 found the sixth instance of this project's recurring **dead-datum** class (the
`last_seen_generation` render gate). Reading the interaction spine end-to-end for this plan found
**two more**, and both sit directly under rungs the architecture sequences early. They are stated
here rather than discovered at implementation time.

**Dead datum #7 — the blur/leave defence has no producer on Windows, so it never fires in a real
host.**
`RawInputEvent::{CursorEntered, CursorLeft, WindowFocus}` exist (`raw/event.rs:44-58`) and
`PhysicalInput::apply` honours them (`raw/queue.rs:302-313`). But:

* `window_proc` captures exactly `WM_KEYDOWN/UP`, `WM_SYSKEYDOWN/UP`, `WM_MOUSEMOVE`,
  `WM_L/R/M/XBUTTON{DOWN,UP}`, `WM_MOUSEWHEEL/HWHEEL` and `WM_INPUT`
  (`boyko_rhi_vulkan/src/window.rs:650-672`). `WM_MOUSELEAVE`, `WM_SETFOCUS` and `WM_KILLFOCUS` fall
  into the `_ => DefWindowProcW` arm;
* `win32::translate` has no arm that can produce any of the three (`boyko_input/src/win32.rs:190-209`);
* both levels **default `true` and persist across `begin_frame`** (`raw/queue.rs:186-200`);
* the only producers anywhere in the tree are tests (`boyko_ui/tests/p4_input_seam.rs:86-98`).

So `ui_focus_system`'s blur short-circuit at `focus.rs:161` — *the* defence D16 names against stuck
capture — is **structurally unreachable in production**. Alt-tabbing away from the window leaves the
last hovered node hovered forever, today, with no capture involved. **This is why I0 exists and why
it is first**: capture without a working release path is a permanent wedge, and the mitigation the
architecture names would be extending a defence that does not fire.

**Dead datum #8 — a `.ui`-authored click handler can never fire, because the capability it needs is
not in the vocabulary.**
`parse_and_insert` is a closed match over **20 arms / 21 component types**
(`text/dispatch.rs:83-214`): `UiLayout`, `UiSpacing`, `UiAlign`, `UiAbsolute`, `ContentSize`,
`UiText`, `ComputedRect`, `ComputedClip`, `StackIndex`, `UiRoot`, `Button`, `Bar`, `BarFill`,
`UiImage`, `UiGrid`, `UiAnchor`, `OnClick`, `OnHover`, `OnSubmit`, and one fused
`"BindText" | "BindValue"` arm. **`Interaction` and `Focusable` are both absent.**

`Button` is a bare ZST marker and **nothing** turns it into an interactive node — verified: no system
in the crate reads it (`widgets.rs` has only the bar systems), and the P6a gate test hand-inserts
`Interaction` before pressing (`tests/p6a_button_dispatch.rs:39-55`). Therefore a `.ui` file can
author `OnClick(3)` on a `Button` and the node can never become `Hovered` or `Pressed`, so the
handler is unreachable. The same holds for `Focusable`: the tab order is built from
`scratch.candidates` (`focus.rs:527-533`), and a candidate requires an `Interaction` column
(`focus.rs:224`) — so **`Focusable` without `Interaction` is silently unfocusable**, in code as well
as in `.ui`.

This blocks two things outside this plan: D32's observer rung ("one `.ui` file, one animated hover")
and the whole Aether `on click:` lowering (D27), which emits exactly this `OnClick(u16)`. **I1 fixes
it**, and it is the dependency the Aether plan must name.

**And a third, pre-existing, which is not this plan's to fix but must not be assumed away.**
`serialize_ui` writes **8** things — `UiLayout`, `UiSpacing`, `UiAlign`, `UiAbsolute`, `ContentSize`,
`StackIndex`, `ComputedClip`, `UiRoot` (`text/serialize.rs:31-104`) — bounded by `LiveNode`'s
snapshot set (`reload/tree_view.rs:33-57`). So the `.ui` round trip is **already lossy for 12 of the
20 parse arms**, `OnClick` among them. The architecture's §10.9 gate ("identical `serialize_ui` bytes
for all 19 components") is keyed to a count that does not match the tree — the arms number **20**, the
types **21**, and the serialized set **8**. Recounted here because a gate keyed to an uncounted number
is the failure mode this register exists to prevent; the correction belongs to whoever lands D7.

---

## 2 · Corrections to the architecture, made at their source

Five of §6's decisions do not survive contact with the tree as written. Each is corrected here with
the defect it replaces.

### C-I1 — D16's stuck-capture defence extends a mechanism that does not fire

**Was:** *"The engine's existing unconditional blur reset on `!cursor_inside || !window_focused`
(`focus.rs:161`) **must extend to capture**."*
**Now:** that reset is unreachable in a real windowed host (dead datum #7 above). The defence must be
**built** before it can be extended. **I0** builds it; **I2** then extends it. The architecture's
sequencing note — *"capture first (largest effect, smallest change)"* — is corrected to *capture
second*.

### C-I2 — the caret cannot binary-search "the existing shaped run", because no such run is retained

**Was (D20):** *"Caret placement from a click is a binary search over cluster advances in the existing
shaped run (`text/shape.rs`, `measure.rs` already hold the advances)."*
**Measured:** `shape_into` takes `sink: impl FnMut(ShapedGlyph)` and streams — it retains nothing
(`shape.rs:61,102-107`). `ShapedGlyph` is `{ rect: [f32;4], uv: [f32;4] }` with **no byte index**
(`shape.rs:28-32`). And whitespace **emits no quad at all** — *"a space has no atlas entry"*
(`shape.rs:48-50`) — so even a retained quad list could not place a caret inside a run of spaces or
at end-of-line.
**Now:** **I8** adds `shape_carets_into(…, sink: impl FnMut(CaretStop))` where
`CaretStop { byte: u32, pen_x: f32, line: u16 }`, emitted for **every** char including whitespace and
including the one-past-the-end stop per line, built on the **same** `next_line` / `line_advance` /
kerning code `shape_into` uses. That shared core is what makes the caret x byte-identical to the
emitter's pen — the identical property `measure` already gets the identical way
(`text/measure.rs:12-15`), and it is gated the same way.

### C-I3 — D20's new inline text buffer already exists, and a second one would be a parallel data system

**Was:** *"The buffer is a **fixed inline POD**, following the shipped `UiName` precedent … at a
larger cap."*
**Measured:** `UiTextBuffer { bytes: [u8; 247], len: u8 }` (`binding/components.rs:67-79`) **is** that
component. It is tick-bearing, `Changed`-gated by `ui_text_measure_system` (`measure.rs:5-7`), read by
the glyph emitter, and already round-tripped through data binding.
**Now:** text editing mutates `UiTextBuffer` **in place**. `TextInput` carries **no buffer**; the
capability is the co-presence `TextInput` + `UiTextBuffer`. Everything downstream — re-measure →
`ContentSize` → relayout → re-emit — is machinery that exists and is tested. A second buffer would
need its own measure, its own emit and its own reconcile arm: a subsystem-local data model glued on
the side, which is the Principle-0 violation this campaign exists to avoid.
**Consequence:** `TextInput.cap` is dropped. `UiTextBuffer::CAP` is the one number; `TextInput` keeps
`max_len: u16` as a **per-field clamp asserted `<= UiTextBuffer::CAP`**, never a second spelling of
the cap.

### C-I4 — D23's dirty term `Removed<Interaction>` is not expressible in this kernel

**Was (D23 item 1):** *"`Changed<ComputedRect>` + `Changed<ComputedClip>` + `Added`/`Removed<Interaction>`
+ hierarchy edits + `UiViewport.generation` is a complete dirty term."*
**Measured:** the filter vocabulary is `Added`, `Changed`, `With`, `Without`, `Or`
(`query/filter.rs:513,693,863,1253,1535`). **There is no `Removed<C>` anywhere in `boyko_ecs`.**
**Now:** the removal leg is an `observe_on_remove::<Interaction>` observer
(`observer_api.rs:170`) bumping a `UiCandidateGeneration` counter, which the discovery system
compares — the same shape `UiViewport.generation` already uses (`resources.rs:34-36`). **I11** ships
it that way. Written as specified, the gate would have compiled to a term that silently never fires,
and a stale candidate array is invisible until an entity is clicked that no longer exists.

### C-I5 — the `[Entity; 16]` ancestor snapshot pays per node for a walk that happens per event

**Was (D17):** *"recording a **bounded ancestor snapshot** (`[Entity; 16]`, a fixed array in the
retained scratch) makes routing a ≤16-step array walk."*
**Arithmetic:** `Candidate` is 64 B today (`focus.rs:95-107`). A per-candidate `[Entity; 16]` is
**+128 B**, i.e. 64 B → 192 B, tripling the very working set D23 item 3 proposes to shrink from 64 B
to 16 B. At N = 1000 that is 64 KB → 192 KB — out of L2 on the machines this targets.
**And the walk it avoids does not happen per node.** Routing resolves **one** channel to **one**
container: at most once per wheel event and once per press. Walking `ChildOf` from the hovered node,
bounded to `MAX_ROUTE_DEPTH = 16` hops, is O(depth) **per event**, not per frame.
**Now:** **I6** routes by a bounded `ChildOf` walk from the resolved target. The snapshot is
**deferred behind §M4**, which measures hops per event at depth 4 / 8 / 16; it ships only if that
number is material. Shipping it unmeasured would be the "arithmetic instead of a measurement" failure
the architecture refuses elsewhere.

---

## 3 · Decisions

### ID1 — the release path is a producer problem in two other crates, and it lands alone

`WM_MOUSELEAVE` is not delivered unsolicited: it must be armed with `TrackMouseEvent(TME_LEAVE)` on
each `WM_MOUSEMOVE` after a leave. `WM_SETFOCUS`/`WM_KILLFOCUS` are delivered unconditionally.

The edit crosses two crates and **keeps the existing layering**: `boyko_rhi_vulkan` stays free of any
`boyko_input` dependency (`window.rs:26-36` states the rule), so `window_proc` captures the three
messages as ordinary `CapturedMsg::Raw` triples and the *translation* stays at the edge, where
`ingest_captured` already lives (`boyko_app/src/runner.rs:971-979`).

`CursorEntered` has no dedicated message: it is derived as **the first `WM_MOUSEMOVE` after a
leave**, which is also the moment `TrackMouseEvent` must be re-armed — one state bit on the window,
not a new event source.

**Rejected:** (a) *synthesise the levels in `boyko_app` from cursor bounds* — a second source of
truth for a fact the OS already reports, and wrong for an occluded window; (b) *drop the levels and
reset on every frame with no button held* — breaks click-drag-off, the exact case
`p6a_button_dispatch::drag_off_then_release` pins.

### ID2 — `Interaction` is authorable in **bare** form; its `.ui` value is always `None`

`Interaction` is runtime state written every frame. Authoring it must express **capability**, never
state. So the `.ui` form is bare (`Interaction`, the `UiRoot`/`Button` precedent at
`text/dispatch.rs:125-140`), it always spawns `Interaction::None`, and `serialize_ui` emits the bare
form **regardless of the live runtime value**.

**Reason:** a struct form `Interaction { Hovered }` would make the `.ui` round trip depend on where
the mouse was when the file was written — a non-deterministic golden, and the `p3_round_trip` corpus
would flake on cursor position. Bare form makes the round trip a function of the document alone.

`Focusable` takes the **struct** form `Focusable { tab_index: 3 }`, not a tuple: `StackIndex` is
documented as the *only* tuple newtype in the format (`text/dispatch.rs:118`, Decision 15), and
`Focusable` is a named-field struct, so the struct form disturbs nothing.

**Rejected:** *`Button` implies `Interaction`.* Capability is component **presence**, never inferred
from a marker — inferring it is precisely the flag-shaped anti-pattern this engine refuses, and it
would make a decorative non-clickable button unexpressible.

### ID3 — capture is one field on the slot, and its release is three unconditional paths

```rust
pub struct PointerSlot {
    owner: Option<Entity>,        // NEW — captured target; None ⇒ resolve normally
    owned_channels: u8,           // NEW — Move | Wheel | Keys (the ImGui SetKeyOwner idea)
    press_pos: [f32; 2],          // NEW — stamped at press; no drag delta is computable without it
    pending_click: Option<(Entity, u16)>,
    click_fired: Option<(Entity, u16)>,
}
```

Release is unconditional on **all three** of: blur/leave (I0's now-live path), the owner losing its
`Interaction` column or its generation (the `get_component_raw` re-validation `dispatch.rs:70-73`
already performs), and an explicit release call. Generation safety needs nothing new — `Entity`
equality includes the generation, the argument `focus.rs:487-490` already documents.

**Reason:** while a pointer is captured *no hit test runs at all* — the only genuine O(1) result in
the entire reference survey, and it covers exactly the high-frame-rate interactions (slider drag,
thumb scroll, text selection).

**Rejected:** *a `Captured` component on the target.* It is per-**pointer** state, not per-element:
with `MAX_POINTERS > 1` two pointers capturing the same entity would need two rows, and the array
shape on the slot is already correct.

### ID4 — the resolve pass gets **one** named output type, today, before anything consumes it

```rust
pub struct PointerResolve {
    pub target: Option<Entity>,
    pub captured: bool,
}
```

Every downstream state machine (drag, scroll, tooltip, gesture) keys on `PointerResolve`, never on a
bare `Option<Entity>`.

**Reason:** this is R3's mitigation made mechanical. If the bubbling hatch (I3) becomes the common
path and the default must invert from *one target* to *a path*, widening this struct is **one edit**
rather than a survey of every consumer. It costs nothing now and it is not expressible later.

### ID5 — bubbling ships at I3, before its first consumer, on the kernel's own observer machinery

`UiPointerEvent { kind, target, pos }` as a `Trigger` with `PROPAGATION = PropagationMode::Up` and
`Traversal = ChildOfTraversal`; `propagate(false)` stops it. Fn-pointer runners, no `Box<dyn Fn>`, and
the sticky `ArchetypeFlags::HAS_ENTITY_OBSERVER` bit means a subtree with no observer pays **nothing**
(`observers/entity_store.rs:5-11`).

**Reason (R3, verbatim):** the hatch must exist *before* drag/scroll/text, so the question "did the
escape hatch become the common path?" is answerable **by use** rather than by argument. Landing it
last would deliver a mechanism nothing had the chance to adopt.

**Naming hazard fixed in the same commit:** `FocusPolicy::{Block, Pass}` reads like Godot's
`MOUSE_FILTER_{STOP, PASS}` and means something different — occlusion during *resolution*, not
propagation *after* it. Once any propagation exists the doc comment must separate the two axes by
name, or every author arriving from Godot writes the wrong one.

### ID6 — the drag payload **is** the dragged entity; `Interaction` gains no variant

`Draggable { threshold_px: f32, channels: u8 }` (presence = capability; default 6.0 px, ImGui's
named `MouseDragThreshold`, not an ad-hoc epsilon) · `DragActive { origin, grab_offset, started_at }`
**added** past the threshold and **removed** on drop · `DropTarget { accepts_mask: u32 }` checked in
O(1) against the dragged entity's archetype.

**Reason:** every reference stores the payload in a subsystem-owned box (Godot `Variant`, HTML5
`DataTransfer`, ImGui's untyped context buffer). A `Box<dyn Any>` or a `HashMap<DragId, Payload>` side
store is the Principle-0 violation; the entity *is* the handle and the ECS *is* the payload store.

**Rejected:** *an `Interaction::Dragged` variant.* Dragging is `DragActive`'s presence. A fourth
variant would widen the animation plan's `UiStateTint` array and the state table for a state that is
structurally available for free.

### ID7 — the scroll offset is folded at **traversal**, and joins exactly two change sets, neither of them layout's

This is the architecture's D19a, carried verbatim because it is the decision the original record got
wrong and the one this ladder is most likely to regress. `ComputedRect`/`ComputedClip` stay in
**unscrolled layout space**; the two DFS descents that map to screen space fold the inherited scroll
on the stack tuple they already carry — `collect_candidates` (`focus.rs:204-257`) here, and the
canonical gather in the sibling render plan.

`ScrollPosition` is a term of `ui_render_discovery`'s `Or<…>` (or the frame would not move) and of
I11's candidate-rebuild dirty set (or the hit-test would test pre-scroll rects). **It is not a term of
`ui_layout_discovery`'s `Or<…>`**, and that omission is the decision: `ui_layout_discovery` collapses
its terms into one bool (`layout.rs:126-129`) after which `ui_layout_apply` re-solves **every** cached
root (`layout.rs:189-197`), so a `ScrollMomentum` write every coasting frame would re-solve the whole
screen per fling frame. §M2 is the pass/fail gate; a future editor adding the term re-introduces
exactly this defect.

**Zero extra probes in the common case, by one gating rule that must be stated or it will not
happen:** a scrolling node **always** carries a `ComputedClip` (an axis set to `Scroll` implies
`Clip`, and I5 makes layout the writer of both), and both descents **already read `ComputedClip` on
every node** (`focus.rs:221`). So: **probe `ScrollPosition` only where the `ComputedClip` read
returned `Some`.** A UI with no scroll container pays nothing at all.

### ID8 — `WM_CHAR` is UTF-16, so surrogate pairing is a decision, not an implementation detail

A non-BMP character arrives as **two** `WM_CHAR` messages — a high surrogate then a low surrogate.
`RawInputEvent::Text(char)` cannot hold either alone, and `win32::translate(msg, wparam, lparam) ->
Option<RawInputEvent>` is a **pure function** whose purity is what makes the whole translator
unit-testable off-Windows (`boyko_input/tests/i6_win32_translate.rs`).

**Decision:** the pairing state is a parameter, not hidden state:
`win32::translate_char(wparam: usize, pending_high: &mut Option<u16>) -> Option<RawInputEvent>`.
`ingest_captured` (`boyko_app/src/runner.rs:971`) owns the one `Option<u16>`. Purity preserved;
testable off-Windows; one `pending_high` per window, cleared on focus loss.

**And control characters are filtered at the translator, not at the field.** `WM_CHAR` delivers
`\r` (0x0D), `\b` (0x08), `\t` (0x09) and Escape (0x1B) as "text". They are **not** text: backspace
and Enter are already `RawInputEvent::Key` edges, and a field that inserts `\b` as a character is the
classic first bug of every hand-rolled text input. `translate_char` emits `None` for
`c < 0x20 || c == 0x7F`, with `\n` deferred to the multi-line rung.

### ID9 — the character stream is a bounded UI-facing ring, and `PhysicalInput` is not widened

`PhysicalInput` is a LEVEL+EDGE snapshot; a character **stream** is neither. `UiTextInputQueue`
(a `Resource`: `chars: [char; 64]`, `len: u8`, `dropped: u16`) is filled at ingest and drained by the
edit system, cleared each frame like `RawInputQueue::begin_frame`.

**Reason:** the existing test `physical_text_event_is_ignored_by_snapshot` (`raw/queue.rs:571-580`)
asserts, with a stated rationale — *"Text is for text fields only — never gameplay; the physical
snapshot must not carry it"* — that `Text` never reaches `PhysicalInput`. That rationale is correct.
This design **honours the test unchanged rather than re-blessing it**: text still never reaches the
physical snapshot. Re-blessing a correct test to make a feature fit is how a gate stops being one.

`dropped` is counted, not silently discarded, following `RawInputQueue::dropped` (`raw/queue.rs:140`).

### ID10 — `Focusable.tab_index` widens `u32 → i32`, and `Focusable` alone becomes sufficient for the tab order

Two changes, one commit:

1. `tab_index: i32`, where **negative = directly focusable but not sequentially** (Bevy's
   `bevy_input_focus` model). The sort key widens from `(u32, u32, u32)` to `(i32, u32, u32)`
   (`focus.rs:531-535`); negatives are filtered out of the Tab cycle, not sorted to the front.
2. **`Focusable` without `Interaction` becomes focusable.** Today `focusables` is derived from
   `scratch.candidates` (`focus.rs:527-533`) and a candidate requires an `Interaction` column
   (`focus.rs:224`), so a keyboard-only widget is silently absent from the tab order. The DFS gains a
   second acceptance leg: a node with `Focusable` and `ComputedRect` enters a `focusables` array
   directly, whether or not it is a pointer candidate.

**Reason for (2):** it is the same defect class as dead datum #8 one level down — a capability
component that does nothing because a *different* capability is missing. And `tab_index < 0` is
unusable without it: "directly focusable, not sequentially" describes exactly the node that has no
reason to carry `Interaction`.

**Rejected:** *keeping `u32` and reserving `u32::MAX` as the not-sequential sentinel.* A sentinel
inside the value range is a second meaning for one field — the alias the sprite plan's D1 is paying
to retire from `UiInstance`.

### ID11 — the focus ring, the caret and the selection are ordinary `UiInstance` records; **no shader work in this plan**

The ring is an outset rect with a border, emitted last in the node's paint order (the architecture's
D4 emission contract: *background → **either** nine-slice **or** image → glyphs → focus ring*
*(the middle two terms became alternatives 2026-08-21 — `UI-PLAN-SPRITES.md` **S-D12 (1)**:
`UiNineSlice` suppresses the image record it slices, because the slices ARE that image. The ring's
position — last — is untouched, and so is everything this plan asserts about it.)*). The caret is a
1–2 px rect; the selection is one rect per line. All three ride the existing pipeline, the existing
per-instance clip and the existing z-sort.

**So no rung in this plan touches HLSL, `boyko_shaderdsl`, or any `.spv`.** That is stated as a
positive claim so a future rung cannot drift into hand-editing a shader: the moment any of these
needs a *shader* expression — a blinking caret animated in the fragment shader, a non-AABB clip, a
rotated ring — it stops being an interaction rung and acquires the full shader ladder: an eDSL leaf,
the `// === GENERATED <name> BEGIN/END ===` sentinels, `*_edsl_sync` **and** `*_spv_sync` re-DXC
pins, and a row in [`docs/SHADER-VARIANT-MANIFEST.md`](SHADER-VARIANT-MANIFEST.md) — and it may not
land before the sprite plan's D30 eDSL migration, per R1.

**Caret blink is therefore CPU-side**: a `Time`-driven visibility bit that toggles the caret
instance's presence. `:focus-visible` is one bool on `UiInputFocus`, not a component and not a flag
bit.

### ID12 — momentum is UIKit's frame-rate-independent form, on `Time`'s real delta

`ScrollMomentum { vel: [f32; 2] }` **added** on fling release, **removed** below `STOP_EPS`, so the
integrate system's query is **empty** when nothing is coasting.

```
v *= DECEL_RATE.powf(dt_secs * 1000.0);   // DECEL_RATE = 0.998 (UIKit `normal`)
offset += v * dt_secs;
if v.length() < STOP_EPS { remove ScrollMomentum; }
```

`decelerationRate` is defined as the velocity change after **one millisecond**, which is what makes
`powf(dt_ms)` frame-rate independent by construction rather than by tuning.

**Clock:** `Time::delta_secs()` — already `min(raw, max_delta)`-clamped, speed-scaled and pause-aware
(`time/time.rs:63-75`, `:193-207`). A fling therefore pauses with the game and slows in slow motion,
for free, and needs no subsystem clock. This is the animation plan's D15 default (real delta,
virtual opt-in per row) applied here; if that plan makes the UI clock virtual-by-default, momentum
follows it rather than keeping a second answer.

---

## 4 · Dependencies on the sibling plans

Named explicitly, in both directions, because the Aether plan depends on what this one exposes.

### On [`UI-PLAN-SPRITES.md`](UI-PLAN-SPRITES.md)

| What | Why | Blocks |
|---|---|---|
| **D31 — `gather_ui_nodes` shipping as a crate function in `boyko_render`** | The focus ring (I9), the caret and the selection rects (I8) are *emitted*, and the emission site is the gather. Until the gather is crate-provided, they would be emitted by a host-supplied closure — R2 verbatim, one layer down. | The **visual** half of I8 and I9. Their state/logic half is independent and lands regardless. |
| **D32 — the minimal `boyko_app` UI rung** | Every rung here is invisible to automated gates without a host: the GPU goldens skip on a device-less machine (`ui_rect_gpu_golden.rs:36-39`) and `ui_hud_screenshot.rs` is `#[ignore]`d eight times. | Owner-eval confirmation of I6 (does a fling *feel* right), I8 (caret placement) and I9 (ring legibility). Not the landing of any rung. |
| **D1 — the 80 B `UiInstance`** | **Nothing here needs it.** No interaction rung adds an instance field; the ring/caret/selection are ordinary records. Stated so nobody sequences this plan behind the widening. | Nothing. |

### On [`UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md)

| What | Why | Blocks |
|---|---|---|
| **`UiVisual` + `TweenTint` + the D14 transition trigger** | I10's tooltip fade and the hover/press *visual* response are animations. This plan ships the **timing and state** half only — `HoverDwell` presence/removal, `DragActive` presence, `Interaction` edges — and never defines `UiVisual`. | The visible half of I10. `HoverDwell`'s timer and its gates land without it. |
| **G-STANDING-1 (owed by this plan to that one)** | D14 keys on `Changed<Interaction>`. Every rung here that touches `write_interactions` must keep the set-if-changed discipline or the transition trigger fires on still frames. | Nothing, if honoured. Everything, if not. |
| **The clock default (D15)** | ID12's momentum uses `Time::delta_secs()`. If the animation plan makes the UI clock virtual-by-default, momentum follows that one answer. | Nothing — a one-line follow. |

### On [`UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md) — and what it depends on here

The Aether plan is sequenced **last** in the architecture (§11 rung 8) precisely because it cannot
usefully name what does not exist. What this plan owes it:

**1. The vocabulary this plan adds, and the count.** Aether's `ui` construct and the `.ui` format
share one component vocabulary (D7's registration table). This plan adds **eight authorable
components**:

| Component | Rung | Form |
|---|---|---|
| `Interaction` | I1 | bare (ID2) |
| `Focusable { tab_index: i32 }` | I1 / I9 | struct |
| `Draggable { threshold_px, channels }` | I4 | struct |
| `DropTarget { accepts_mask }` | I4 | struct |
| `Overflow { x, y }` | I5 | struct |
| `TextInput { max_len, flags }` | I8 | struct |
| `FocusGroup { order }` | I9 | struct |
| `FocusNeighbors { up, down, left, right }` | I9 | **struct, entity-valued — see below** |

If D7's registration table has landed, each is one derive. If it has not, each is the five
hand-written landings D7 exists to collapse — a `.ui` dispatch arm, a field parser, a `serialize_ui`
arm plus a `LiveNode` field, a reconcile diff arm, and a row in the equivalence gate. **This plan
does not block on D7**; it states the cost of landing without it, and the fallback is the status quo.

**2. The exclusion list, which is a safety property and not a convenience.** These are runtime state
and must **never** enter the vocabulary: `DragActive`, `ScrollPosition`, `ScrollMomentum`,
`ScrollExtent`, `TextCursor`, `TextPreedit`, `HoverDwell`, and everything on `UiPointerState`.
Authoring them would let a document assert a state the systems own, and a hot reload would then fight
the runtime for it. The format already blesses this shape: *"Transient components
(`UiFocus`/`UiScroll`/`UiHover`, P4+) … are NEVER written, so they are preserved by omission"*
(`reload/reconcile.rs:49-52`).

**3. `FocusNeighbors` is not expressible in `.ui` and needs Aether's `#name` machinery.** Its fields
are `Entity`, and `.ui` has no entity references at all. So it is **code- and Aether-only**, and the
mechanism it needs is exactly D26's `#name` resolution with the forward-reference deferred-insert
tail. That is a concrete requirement on the Aether plan from this one, and the only one.

**4. Dead datum #8 is the Aether plan's blocker, and I1 is its fix.** D27 lowers `on click: Action`
to `OnClick(u16)` — the component that today cannot fire from a document, because the capability that
reaches it is not authorable. **Until I1 lands, every Aether `on click:` handler is unreachable**, and
its acceptance test would be asserting on a path that has never run.

---

## 5 · The rung ladder

Ordered by dependency and by the cost of doing it late. **I0 is the first landable rung.**

| # | Rung | Size | Depends on |
|---|---|---|---|
| **I0** | The release path — the three window levels get producers | S | — |
| **I1** | The authorable interactive node — `Interaction` + `Focusable` in the vocabulary | S | — |
| **I2** | Pointer capture, press position, the one resolve type, the slot loops | M | I0 |
| **I3** | The opt-in bubbling hatch | S | I2 |
| **I4** | Drag | M | I2, I3 |
| **I5** | `Overflow` → computed `ComputedClip` + `ScrollExtent` (the layout substrate) | M | — |
| **I6** | Scroll — traversal fold, wheel routing, momentum | L | I2, I5 |
| **I7** | The character seam — `WM_CHAR` → `Text` → the UI char ring | S | I0 (same files) |
| **I8** | Text editing — caret, selection, editing ops, caret hit-test | L | I2, I7 |
| **I9** | Keyboard navigation + focus ring | M | I1 |
| **I10** | Hover dwell + tooltips | S | I2 |
| **I11** | Hit-test dirty gate + early-out | M | I1, I5, I6, I9 |

---

### I0 — the release path: the three window levels that have no producer — **size S**

**Lands.** `boyko_rhi_vulkan/src/window.rs`: `WM_MOUSELEAVE`, `WM_SETFOCUS`, `WM_KILLFOCUS` added to
`window_proc`'s capture arm (`:650-655`); a `TrackMouseEvent(TME_LEAVE)` arm-on-`WM_MOUSEMOVE`
with a one-bit `tracking` state on `InputRing`; the `TRACKMOUSEEVENT` struct + `TrackMouseEvent`
declaration in `ffi.rs`. `boyko_input/src/win32.rs`: three `translate` arms →
`CursorLeft` / `WindowFocus(true)` / `WindowFocus(false)`, plus `CursorEntered` derived from the
first `WM_MOUSEMOVE` after a leave. No new crate edge: the translation stays at the
`boyko_app::ingest_captured` edge exactly as today.

**Depends on.** Nothing. This is the first landable rung.

**Default.** No flag and no opt-in — it repairs two levels that are *falsely `true`* today
(`raw/queue.rs:186-200`). Goldens byte-identical (no render change). The behaviour change is
observable only when the window loses focus or the cursor leaves, which no test and no golden
currently exercises.

**Gates.**
1. `win32::translate` unit tests, one per message, in the existing off-Windows pure-function corpus
   (`boyko_input/tests/i6_win32_translate.rs`) — `WM_KILLFOCUS ⇒ WindowFocus(false)` etc.
2. `ingest_captured` bridging test in `runner.rs`'s own `#[cfg(test)] mod tests`
   (`:3609-3640` is the shape), asserting the three `CapturedMsg::Raw` triples reach the queue as the
   three `RawInputEvent`s, and that the cursor-entered derivation fires **once** per leave→move pair
   and not on every move.
3. An end-to-end `boyko_ui` test: hover a node, apply `WindowFocus(false)`, run one frame, assert
   every interactive node is `Interaction::None`, every EnableTag bit is clear, `pending_click` is
   `None` and `UiInputFocus.focused` is `None`. This test **passes today** against the synthetic
   event — its value is that after this rung the event has a real producer, which gate 2 proves.
4. `TrackMouseEvent` re-arm assertion: two leave→enter cycles both deliver `WM_MOUSELEAVE`
   (a single-arm implementation passes the first cycle and silently fails every one after it).

**RED MUTATION.** Delete the `WM_KILLFOCUS` arm from `window_proc`'s capture list. Gate 2 goes red
(`WindowFocus(false)` never reaches the queue). Independently: delete the re-arm call after the
derived `CursorEntered` — gate 4 goes red on the second cycle while gate 2 stays green, which is the
point of splitting them.

**Why first.** Capture (I2) without a working release is a permanent wedge, and today the release path
is unreachable in production (dead datum #7). This rung is also small, self-contained, and touches
files no other rung in this plan rewrites except I7 — which is why I7 is its natural pair.

---

### I1 — the authorable interactive node — **size S**

**Lands.** `Interaction` (bare, ID2) and `Focusable` (struct) added to `parse_and_insert`
(`text/dispatch.rs`); two `LiveNode` fields + two snapshot reads (`reload/tree_view.rs:49-56`); two
reconcile diff arms; two `serialize_ui` arms in the fixed canonical order
(`text/serialize.rs:65-100`); two rows in the `p3_equivalence` corpus; a doc note on `Button`
recording that it is a marker and that `Interaction` is the capability.

**Depends on.** Nothing. Cheaper if D7's registration table has landed (two derives instead of ten
landings); **does not block on it**.

**Default.** Additive vocabulary — every existing `.ui` document parses to a byte-identical world.
The new arms are reachable only from a document that names them. Goldens byte-identical.

**Gates.**
1. `p3_round_trip`: a document with `Interaction` + `Focusable { tab_index: 2 }` round-trips to
   byte-identical text, **with the cursor over the node and the node `Hovered`** — the assertion that
   ID2's bare form actually decouples the serialized text from the runtime state.
2. `p3_equivalence`: the `ui!` macro form and the `.ui` text form spawn identical worlds for both.
3. The dead-datum-#8 gate, end to end: parse a `.ui` document containing `Button` + `OnClick(1)` +
   `Interaction`, run focus → dispatch over a synthetic press+release, assert the action fires.
4. Its negative twin: the **same** document with the `Interaction` line removed fires **nothing** —
   this is the assertion that pins capability-by-presence rather than by marker inference.
5. `p3_malformed`: `Interaction { Hovered }` and `Interaction(1)` are recoverable errors with the
   exact message, line and column the report contract requires.

**RED MUTATION.** Make the `Button` arm insert `Interaction` alongside the marker (the "obvious
convenience"). Gate 4 goes red — the action fires from a document that never asked for it. That is
the mutation worth having, because inferring capability from a marker is the change a future
contributor will propose.

---

### I2 — pointer capture + press position + one resolve type + the slot loops — **size M**

**Lands.** ID3's three new `PointerSlot` fields; ID4's `PointerResolve`; the resolve short-circuit
when `owner.is_some()`; release on blur (I0's now-live path), on owner invalidation, and on explicit
call; `press_pos` stamped at press; the `slots[0]` hardcodes at `focus.rs:471` and `dispatch.rs:44,53`
replaced by loops over `MAX_POINTERS` (which stays **1**); a `resolve_scans` counter on
`UiInteractionScratch`, promoted **out of** `#[cfg(test)]` as a diagnostic (the `relayout_count`
precedent §M2 also needs).

**Depends on.** I0 — the blur release is one of the three unconditional release paths and it must be
able to fire.

**Default.** Capture is **taken by nobody** at this rung: no system calls `capture()`. Behaviour is
byte-identical and the existing p4 corpus is the regression gate. I4 and I6 and I8 are the first
callers.

**Gates.**
1. **The O(1) claim, measured, not asserted:** with capture held across 60 frames, `resolve_scans`
   reads **0**. Without capture, it reads `candidates.len()` per frame. §M1.
2. Drag-off-and-release: press inside, move outside, release. With capture the target stays the
   origin for the whole gesture; `p6a_button_dispatch::drag_off_then_release` still fires nothing on
   release-outside (the click contract is unchanged — capture changes *targeting*, not *firing*).
3. **The wedge tests, all three release paths, one test each:** despawn the captured entity mid-drag;
   remove its `Interaction` column mid-drag; deliver `WindowFocus(false)` mid-drag. Each must leave
   `owner == None` and every node `Interaction::None` on the next frame.
4. `press_pos` is stamped at press and is stable across held frames (a delta computed against it must
   be monotone under a monotone cursor path).
5. `zero_alloc`: the capture path allocates nothing — the counting-allocator harness the crate
   already uses.
6. G-STANDING-3: with capture held, the unconditional reset pass still visits every node and still
   resets the non-target ones.

**RED MUTATION.** Delete the despawn release. Gate 3's first leg goes red: the slot keeps a dead
`Entity`, the resolve short-circuits to it forever, and every subsequent click is swallowed. This is
the exact stuck-input failure the reference survey names (`OpenSeadragon#1962`), so it is the one
whose red must be seen.

---

### I3 — the opt-in bubbling hatch — **size S**

**Lands.** `UiPointerEvent` as a `Trigger` with `PROPAGATION = Up` / `Traversal = ChildOfTraversal`;
fired from `ui_focus_system` at the transition sites it already computes; `propagate(false)` as the
consume signal; the `FocusPolicy` doc-comment axis separation (ID5).

**Depends on.** I2 (the event carries `PointerResolve`).

**Default.** Structurally off: with no entity observer registered anywhere, the sticky
`ArchetypeFlags::HAS_ENTITY_OBSERVER` bit is unset on every archetype and the fire loop is skipped
entirely (`observers/entity_store.rs:5-11`). Zero cost when unused, and that is a measured gate below,
not a claim.

**Gates.**
1. A parent observer receives a child's press; `propagate(false)` in the child's observer stops it.
2. Depth-bounded: a 20-deep chain with an observer only at the root fires exactly once at the root.
3. **The zero-cost gate:** a UI with **no** observers registered runs the focus pass with a
   fire-loop-entry counter reading **0**, and the still-frame allocation count is unchanged from I2.
4. Adoption instrument: the observer-registration count is exposed as a diagnostic, so R3's question
   ("did the hatch become the common path?") is answerable from data at the end of the campaign
   rather than from argument.

**RED MUTATION.** Change `PROPAGATION` from `Up` to `None`. Gate 1 goes red (the parent never
receives it). Independently: register one observer on an unrelated entity and re-run gate 3 — it must
**stay** at 0 fire-loop entries for the observer-free subtree, which is what proves the gate is
measuring the sticky archetype bit and not merely the global absence of observers.

---

### I4 — drag — **size M**

**Lands.** ID6's `Draggable` / `DragActive` / `DropTarget`; the threshold transition (press →
`press_pos` delta exceeds `threshold_px` → insert `DragActive` + take capture); drop resolution
against `DropTarget.accepts_mask`; the drag **preview** as an ordinary UI entity with
`StackIndex = u32::MAX` and **no** `Interaction` column.

**Depends on.** I2 (capture owns the pointer for the drag's life, so the drag costs **zero**
hit-tests per frame), I3 (a drop handler on an ancestor is the first real consumer of the hatch).

**Default.** Structural: no `Draggable` component anywhere ⇒ no drag system does anything. `Draggable`
enters the vocabulary at this rung (§4). Goldens byte-identical.

**Gates.**
1. Sub-threshold move does **not** start a drag and still fires the click on release — the assertion
   that separates a click from a 3 px drag.
2. Threshold crossed ⇒ `DragActive` inserted exactly once; released ⇒ removed exactly once; no
   `is_dragging` bool exists anywhere in the diff (a grep assertion in the test's doc comment is not
   a test — this is asserted structurally by `has_component`).
3. The preview is **not** pickable: with the preview under the cursor, the resolved target is the
   node beneath it. This is the ECS-native unpickability claim, and it must be exercised, because
   Godot needs an explicit ancestor test for the same thing.
4. Drop onto a non-accepting `DropTarget` is a no-op and still removes `DragActive`.
5. `zero_alloc` over a 100-frame drag.

**RED MUTATION.** Give the preview entity an `Interaction` column. Gate 3 goes red: the preview
becomes the resolved target and every drop lands on the thing being dragged.

---

### I5 — `Overflow` → computed `ComputedClip` + `ScrollExtent` — **size M**

**Lands.** `Overflow { x: Visible|Clip|Scroll, y: … }` read by `ui_layout_apply`, which writes
`ComputedClip` **set-if-changed** for the subtree, intersecting down; `ScrollExtent { max: [f32;2] }`
written beside it (content extent minus viewport extent — a layout **output**, so no cycle, and the
scroll clamp never asks layout anything at run time); `Overflow` joins `ui_layout_discovery`'s
`Or<…>` set (correctly — an overflow policy change genuinely changes the solve).

**Depends on.** Nothing in this plan. Landable in parallel with I0–I4.

**Default.** Structural: no `Overflow` component ⇒ layout writes no `ComputedClip` and behaviour is
byte-identical. **The ownership change is the hazard**: `ComputedClip` goes from author-owned
(`components.rs:186-190`) to *conditionally* layout-owned, and a scene that hand-authored a
`ComputedClip` must be left untouched. That is gate 3.

**Gates.**
1. A node with `Overflow { y: Clip }` gets a `ComputedClip` equal to its content box; its
   out-of-bounds child is not hovered (`point_in_clip` at `focus.rs:280` starts doing real work).
2. Nested clips intersect: the inner clip is the AABB intersection, not a replacement.
3. **The ownership gate:** a node with a hand-authored `ComputedClip` and **no** `Overflow` keeps its
   authored value byte-for-byte across N frames of layout. Layout writes clips only where `Overflow`
   is present.
4. Set-if-changed: a still frame with an `Overflow` node bumps **no** `ComputedClip` tick.
5. `ScrollExtent` is zero when content fits and equals `content − viewport` when it does not, on both
   axes independently.
6. `Overflow` round-trips through `.ui` (§4's vocabulary row) and through hot reload.

**RED MUTATION.** Make the clip writer unconditional (write `ComputedClip` for every node, not only
`Overflow`-bearing ones). Gate 3 goes red — the authored clip is overwritten — and this is precisely
the change a contributor makes when "why is the clip only sometimes computed?" looks like a bug.

---

### I6 — scroll: the traversal fold, wheel routing, momentum — **size L**

**Lands.** `ScrollPosition { offset: [f32;2] }` on the container, clamped to `ScrollExtent`; the
traversal fold in `collect_candidates` — the stack tuple widens to `(entity, clip, scroll_accum)` and
a candidate's `rect`/`clip` are folded at push, gated by ID7's `ComputedClip.is_some()` probe rule;
wheel routing via ID5's bounded `ChildOf` walk (C-I5) from the resolved target to the nearest
`Overflow` with a non-clamped axis; `ScrollMomentum` per ID12; `ScrollPosition` joining
`ui_render_discovery`'s `Or<…>` and I11's dirty set, and **not** `ui_layout_discovery`'s.

**Depends on.** I5 (the substrate), I2 (a thumb drag takes capture). The **render-side** half of the
fold belongs to the sibling sprites plan's D31 gather; this rung ships the hit-test half and the
component set, and states that the gather fold lands with the gather.

**Default.** Structural: no `Overflow { … Scroll }` anywhere ⇒ no `ScrollPosition`, no momentum
system rows, no fold (the `scroll_accum` stays 0 and the probe rule never probes). Goldens
byte-identical.

**Gates.**
1. **§M2 — the coasting gate, pass/fail, not informational.** A container coasting under
   `ScrollMomentum` for ≥ 30 frames: `relayout_count` **MUST be 0 across the whole fling**. A non-zero
   value means the scroll offset has re-entered the layout path — the exact defect ID7 exists to
   remove and the one the architecture's first draft shipped. The same leg **reports** the candidate
   rebuilds it does cost, so the trade is two numbers on the record rather than one claim.
2. The probe rule: a UI with **no** clipped nodes performs **zero** `ScrollPosition` probes in
   `collect_candidates` (counter-asserted), and a UI with one scroll container probes only its
   clipped nodes.
3. Hit-testing follows the scroll: after scrolling by 50 px, the node under the cursor is the one the
   *scrolled* content places there, and `ComputedRect` is **unchanged** (unscrolled layout space).
4. Clamp: `ScrollPosition` never exceeds `ScrollExtent.max` and never goes below zero, at any
   momentum magnitude, including one large enough to overshoot in a single frame.
5. Frame-rate independence: the same fling integrated at 30 fps and at 240 fps ends within
   `STOP_EPS` × 2 of the same offset. This is the assertion that `powf(dt_ms)` is doing its job; a
   naive `v *= 0.998` per frame fails it by a wide margin.
6. `ScrollMomentum` is removed below `STOP_EPS` and the integrate system then iterates **zero** rows.
7. Wheel routing: the wheel over a nested non-scrollable child reaches the nearest scrollable
   ancestor; a fully-clamped axis passes the wheel further up.
8. Nested scroll containers each accumulate only their own ancestors' offsets.

**RED MUTATION.** Add `Changed<ScrollPosition>` to `ui_layout_discovery`'s `Or<…>` set — the
"obviously missing" term. Gate 1 goes red immediately: `relayout_count` becomes `roots.len()` per
fling frame. This is the mutation that must be seen red, because it is the shipped defect the
architecture's revision 2 corrected, and it looks like a fix.

---

### I7 — the character seam: `WM_CHAR` → `RawInputEvent::Text` → the UI char ring — **size S**

**Lands.** `WM_CHAR` added to `window_proc`'s capture arm; ID8's
`win32::translate_char(wparam, &mut pending_high)` with surrogate pairing and control-character
filtering; the `pending_high` cell owned by `ingest_captured` and cleared on focus loss; ID9's
`UiTextInputQueue` resource filled at ingest, drained and cleared per frame.

**Depends on.** I0 only in the sense that it edits the same two files and should ride the same review;
functionally independent and separately landable.

**Note the thing that is already done:** `TranslateMessage` is **already** called in the pump
(`boyko_rhi_vulkan/src/window.rs:491`), so Windows is already synthesising `WM_CHAR` and delivering it
to `window_proc`, where the `_ => DefWindowProcW` arm discards it. This rung is one capture arm and
one translate function — not a new pump, and not a new message loop.

**Default.** The queue exists and nothing drains it but the (not-yet-existing) edit system. No
`TextInput` component exists until I8, so no behaviour changes. Goldens byte-identical.

**Gates.**
1. **`physical_text_event_is_ignored_by_snapshot` (`raw/queue.rs:571-580`) stays green, unmodified.**
   Text still never reaches `PhysicalInput`. If this rung needs that test re-blessed, the design is
   wrong.
2. `translate_char` unit tests, off-Windows, in the pure-function corpus: `'a'` → `Text('a')`; a
   surrogate **pair** → one `Text` with the correct non-BMP `char`; a **lone** high surrogate →
   `None` with `pending_high` set; a lone **low** surrogate → `None` with `pending_high` cleared (the
   malformed case, which must not panic and must not resurrect a stale high surrogate).
3. Control filtering: `\r` (0x0D), `\b` (0x08), `\t` (0x09), Escape (0x1B) and 0x7F all produce
   `None`. Each is its own assertion, because each is a separately-plausible omission.
4. Ring bounds: 200 characters into a 64-slot ring keeps the newest, counts `dropped`, and never
   panics; `dropped` is readable, not silent.
5. `pending_high` is cleared on `WindowFocus(false)` — a surrogate half must not survive an alt-tab
   and pair with the next session's first character.
6. End-to-end through `ingest_captured`: a `CapturedMsg::Raw { msg: WM_CHAR, … }` reaches
   `UiTextInputQueue`.

**RED MUTATION.** Remove the control-character filter. Gate 3 goes red for all five codes at once —
and this is the mutation to keep, because "just pass the char through" is the shortest implementation
and it is the one that produces a field that inserts a literal backspace.

---

### I8 — text editing: caret, selection, editing ops, caret hit-test — **size L**

**Lands.** `TextInput { max_len: u16, flags: u16 }` (C-I3: **no buffer**; the capability is
`TextInput` + `UiTextBuffer` co-presence, and `max_len` is asserted `<= UiTextBuffer::CAP`);
`TextCursor { anchor: u32, head: u32, affinity: u8 }` (empty selection = caret, so shift-click and
drag-select are one code path); the edit system draining `UiTextInputQueue` plus the Backspace /
Delete / Home / End / arrow `Key` edges; C-I2's `shape_carets_into` + `CaretStop`; caret placement
from a click by binary search over `CaretStop`s; caret and selection rect emission;
`TextPreedit`'s **shape only**, recorded, not built.

**Depends on.** I7 (the character stream), I2 (drag-select takes capture — without it a selection
drag re-targets the moment the cursor leaves the field).

**Default.** Structural: no `TextInput` component exists in any scene, in any golden, or in any
`.ui` file until one is authored. Goldens byte-identical. `TextInput` and its `.ui` arm land here
(§4); `TextCursor` and `TextPreedit` are runtime-only and are on §4's exclusion list.

**Gates.**
1. **C-I2's identity gate:** for a fixed string, font and size, `CaretStop[i].pen_x` equals the
   emitter's `ShapedGlyph[i].rect.x` for every non-whitespace `i` — the property that comes from the
   shared core, asserted rather than assumed. Plus the two things `shape_into` cannot express: a
   caret stop exists **inside** a run of spaces, and a stop exists at the one-past-the-end position
   of every line.
2. **Never split a grapheme cluster:** clicking mid-way through a base + combining-mark pair places
   the caret on a cluster boundary, never between the base and its mark. A caret between them is a
   bug, not an edge case.
3. Affinity: at a soft wrap, the same byte index resolves to end-of-line-N or start-of-line-N+1
   according to `affinity`, and both are reachable (Home from line N+1 vs End from line N).
4. Editing ops over UTF-8: inserting and deleting multi-byte characters keeps `UiTextBuffer` a valid
   UTF-8 prefix at **every** intermediate step — the `as_str` `from_utf8_unchecked` precondition
   (`binding/components.rs:83-88`) is a soundness invariant, so this is a Miri test, not only a unit
   test.
5. `max_len` clamps insertion; `UiTextBuffer::CAP` clamps it absolutely; a `max_len > CAP` is caught
   by the debug assert and clamped in release (the `UiName::new` truncation discipline).
6. The re-measure chain fires: editing bumps `Changed<UiTextBuffer>`, `ui_text_measure_system`
   rewrites `ContentSize`, layout re-solves the field. This is the whole reason C-I3 reuses the
   buffer, so it is gated.
7. Selection rects: an N-line selection emits N rects; an empty selection emits none and emits one
   caret.
8. `zero_alloc`: typing 100 characters allocates **zero** after warm-up. §M6.

**RED MUTATION.** Give `TextInput` its own `bytes: [u8; N]` buffer and have the edit system write
*that* instead of `UiTextBuffer`. Gate 6 goes red — the field accepts keystrokes and never re-measures
or re-renders, because nothing downstream reads the new buffer. That is C-I3's whole argument, made
falsifiable.

---

### I9 — keyboard navigation + focus ring — **size M**

**Lands.** ID10's `tab_index: i32` widening and the `Focusable`-without-`Interaction` fix;
**Shift+Tab** (the modifier is simply not read today — `focus.rs:543`); **arrow** navigation by
Unity's scoring function — maximise `dot(dir, v) / |v|²` from the point on the current rect's **edge**
in that direction, not its centre, over the same rect arrays the hit-test already builds;
**Escape** (blur / cancel / release capture); **Space** (activates the focused node through the
existing `OnSubmit` path); `FocusGroup { order: u32 }`; `FocusNeighbors` as the opt-in override
(Godot's lesson: automatic directional navigation **always** needs a manual escape hatch); the focus
ring as an outset `UiInstance` emitted last in the node's paint order; `:focus-visible` as one bool on
`UiInputFocus`.

**Depends on.** I1 (`Focusable` must be authorable, or nothing in a `.ui` file can demonstrate this).

**Default.** Shift+Tab / arrows / Escape / Space are **behaviour**, not a component, so they are on
from the moment they land — but they are keys nothing currently binds, so no existing test or golden
observes them. **The focus ring is opt-in** (`FocusRing` on the node, absent by default): it emits an
extra instance and therefore *would* move pixels. Goldens byte-identical while no node carries it;
the rung that deliberately turns it on is the sprites plan's **D32 observer rung**, and it moves that
rung's own new pin only.

**Gates.**
1. Shift+Tab reverses the same total order Tab advances, and the pair is cyclic in both directions.
2. `tab_index < 0` is skipped by Tab and reachable by a direct focus call — both halves, because only
   one of them is the new behaviour.
3. **The ID10(2) gate:** a node with `Focusable` and **no** `Interaction` is reachable by Tab. This
   currently fails by construction and is the defect's red.
4. Arrow scoring: a three-node cross layout picks the geometrically correct neighbour in each of the
   four directions, and a node behind the current one (`dot <= 0`) is never picked.
5. `FocusNeighbors` overrides the scoring when present; absent, the scoring runs.
6. `FocusGroup` scopes the cycle: Tab inside a group stays inside it until the group is exhausted.
7. Escape releases capture (the I2 interaction) **and** blurs, in that order, in one frame.
8. `:focus-visible` is set by keyboard focus movement and cleared by pointer focus — both directions.
9. The ring emits **exactly one** extra instance, last in the node's emission order (D4's contract).

**RED MUTATION.** Sort negative `tab_index` values into the cycle instead of filtering them out (the
natural reading of "widen to `i32`"). Gate 2's first half goes red: a `-1` node becomes the first Tab
stop, which is the opposite of its meaning.

---

### I10 — hover dwell + tooltips — **size S**

**Lands.** `HoverDwell { ms: u16 }` **added** on hover-enter and **removed** on hover-exit, so the
accumulate system iterates typically one row; the **stationarity** test (the part naive
implementations miss, and why a pointer sweeping a toolbar must not fire six tooltips); ImGui's
constants as the starting values — `HoverStationaryDelay = 0.15`, `HoverDelayNormal = 0.40`; the
tooltip entity spawned with `StackIndex = u32::MAX` and **no** `Interaction` (the same structural
unpickability as I4's drag preview).

**Depends on.** I2 (hover-enter/exit edges). The tooltip's **fade** belongs to the animation plan; this
rung ships presence, timing and placement only.

**Default.** Structural: no `Tooltip` component ⇒ nothing spawns. Goldens byte-identical.

**Gates.**
1. A pointer sweeping across five hoverable nodes at speed fires **zero** tooltips — the stationarity
   assertion, which a pure dwell timer fails.
2. A stationary hover past `HoverDelayNormal` fires exactly one.
3. `HoverDwell` is present only between enter and exit; the accumulate system iterates zero rows on a
   still, unhovered frame.
4. The tooltip is not pickable and does not itself keep the dwell alive (otherwise a tooltip under
   the cursor is a self-sustaining hover).
5. The dwell clock is `Time::delta_secs()`, so a paused game does not accumulate dwell.

**RED MUTATION.** Drop the stationarity test and keep only the dwell timer. Gate 1 goes red: the sweep
fires a tooltip on every node whose dwell happened to cross the threshold.

---

### I11 — the hit-test dirty gate + early-out — **size M**

**Lands.** D23 items 1 and 2, in that order. **Item 1:** a dirty-gated candidate rebuild — the term
set is `Changed<ComputedRect>` + `Changed<ComputedClip>` + `Added<Interaction>` +
`Or<(Changed<Children>, Changed<ChildOf>)>` + `UiViewport.generation` + `Changed<ScrollPosition>`
(ID7) + **C-I4's `observe_on_remove::<Interaction>` observer** bumping a `UiCandidateGeneration`,
because `Removed<C>` does not exist. A still frame then costs one scan of a cached array and **zero**
component probes. **Item 2:** candidates built in descending total order so the first
`FocusPolicy::Block` hit terminates the resolve scan.

D23 **item 3** (SoA + SIMD) is **not** in this plan. It is gated on §M5 and ships only if it beats
item 1 alone — and it may earn its place at I9's directional navigation instead, which wants the same
arrays.

**Depends on.** Everything that adds a dirty term: I1 (`Interaction` authorability changes when
columns appear), I5 (`ComputedClip` becomes computed), I6 (`ScrollPosition`), I9 (`Focusable` enters
the candidate/focusable derivation). Landing this before them means writing the term set twice.

**Default.** Pure optimisation — identical resolved targets on every frame. The regression gate is the
**entire** existing p4 corpus plus every gate added by I0–I10, re-run unchanged.

**Gates.**
1. **§M5:** a still frame performs **zero** `get_component` probes in `collect_candidates`
   (counter-asserted), at N ∈ {100, 1000}. A non-zero count means the gate is not where it claims.
2. Each dirty term, one test each, all seven: move a rect; change a clip; add an `Interaction`;
   **remove** an `Interaction`; reparent a node; bump the viewport generation; scroll. Each must
   rebuild; the removal leg is C-I4's observer and is the one that would silently never fire.
3. **G-STANDING-3:** early-out applies to *resolution* only. `write_interactions` still performs its
   unconditional pass, so a node occluded by a `Block` node this frame is still reset to `None`
   (`interaction/components.rs:44-49`). This is a correctness invariant, not an optimisation target,
   and it has its own test.
4. Determinism: the resolved target is identical, frame for frame, to the ungated implementation over
   a 200-frame recorded cursor path with spawns, despawns, reparents and scrolls interleaved.
5. Early-out order: the candidate array is in descending total order, and the scan terminates at the
   first `Block` hit — asserted by a scan-length counter, not by timing.

**RED MUTATION.** Replace C-I4's `observe_on_remove` bump with nothing (the state a literal reading of
D23's `Removed<Interaction>` would produce once it failed to compile and was "simplified away"). Gate
2's fourth leg goes red: a despawned or de-interacted node stays in the cached candidate array and
keeps winning the hit test.

---

## 6 · Measurement obligations

**None of these numbers exists yet.** Each names its instrument and the discriminating comparison —
not "benchmark it". Two of them are **pass/fail gates**, not informational, and are marked so.

| # | Claim under test | Instrument | Discriminating comparison | Rung |
|---|---|---|---|---|
| **M1** | Capture is genuinely O(1) — *"while a pointer is captured, no hit test runs at all"* | `UiInteractionScratch::resolve_scans`, promoted out of `#[cfg(test)]` | 60 frames of a held drag: captured must read **0** scans/frame; uncaptured reads `candidates.len()`/frame. **PASS/FAIL: a non-zero captured count means the short-circuit is not where it claims.** | I2 |
| **M2** | ID7's tier claim — the scroll offset never re-enters the layout path | `LayoutScratch::relayout_count`, promoted out of `#[cfg(test)]` (the architecture's §10.4 promotes the same counter); plus the candidate-rebuild counter | A container coasting under `ScrollMomentum` for ≥ 30 frames. **PASS/FAIL: `relayout_count` MUST be 0 across the whole fling.** The same leg **reports** the candidate rebuilds it does cost, so the honest trade is two numbers. | I6 |
| **M3** | What computed clips cost the descent | probe counter in `collect_candidates` | probes/node/frame at 0, 1 and 8 scroll containers, N ∈ {256, 2048}. The 0-container leg must equal the pre-I5 baseline exactly — that is ID7's probe rule, measured. Feeds the sprites plan's §10.8, which owns the gather's half of the same number. | I5, I6 |
| **M4** | Whether D17's ancestor snapshot is worth +128 B/candidate (C-I5) | hop counter on the routing walk | `ChildOf` hops **per event** at tree depth 4 / 8 / 16, alongside events/frame during a scroll and during a drag. The snapshot ships **only** if hops × events is material against the 64 B → 192 B candidate growth. | I6 |
| **M5** | The dirty gate does what D23 item 1 claims | probe counter in `collect_candidates` | Still frame: probes before vs after, N ∈ {100, 1000}. **Must be 0 after.** Then item 1 alone vs item 1 + SoA (criterion) — SoA ships only if it beats item 1 alone, which is the architecture's §10.6 condition. | I11 |
| **M6** | The text path allocates nothing | the crate's counting-allocator harness (`zero_alloc.rs`) | 100 characters typed into a warm field: **0** allocations. And the ring's `dropped` counter is non-zero under a 200-character burst, so the bound is exercised rather than assumed. | I7, I8 |

**How M1, M2 and M5 avoid being gates that cannot fail.** Each has a named RED MUTATION in its rung
that drives it red, and each counter is a **diagnostic** promoted out of `#[cfg(test)]` — so it reads
the same number in the shipped build that the test reads. The recorded failure mode this guards
against is precise: on the diagnostics campaign, twelve benches in a gate table were reported as
green and **none of them existed**.

---

## 7 · Default-OFF map

Every new capability is **structurally absent** by default — component presence, never a flag, never
an `is_enabled` bool. The complete list, and the one exception:

| Rung | New surface | Off by | Goldens |
|---|---|---|---|
| I0 | — (repairs two falsely-`true` levels) | n/a | byte-identical |
| I1 | `Interaction`, `Focusable` in the vocabulary | additive arms; existing documents unchanged | byte-identical |
| I2 | capture | nothing calls `capture()` at this rung | byte-identical |
| I3 | bubbling | no observer registered ⇒ the sticky archetype bit is unset ⇒ the fire loop is skipped | byte-identical |
| I4 | drag | no `Draggable` anywhere | byte-identical |
| I5 | computed clip | no `Overflow` anywhere | byte-identical |
| I6 | scroll | no `Overflow { … Scroll }` anywhere | byte-identical |
| I7 | the char ring | the queue exists; nothing drains it until I8 | byte-identical |
| I8 | text editing | no `TextInput` anywhere | byte-identical |
| I9 | keys | **behaviour, not a component** — but they are keys nothing binds today | byte-identical |
| I9 | **the focus ring** | **`FocusRing` component, absent by default** | byte-identical *while absent*; **the sprites plan's D32 observer rung is the one that deliberately turns it on**, and it moves only that rung's own new pin |
| I10 | tooltips | no `Tooltip` anywhere | byte-identical |
| I11 | the dirty gate | pure optimisation; identical resolved targets | byte-identical |

**The one thing that is not off by default is I9's key set**, and it is called out rather than hidden:
Shift+Tab, arrows, Escape and Space begin working the moment they land. They are safe because nothing
in the tree binds them today and no golden is keyboard-driven — but a host that reads
`PhysicalInput` for gameplay on those keys now shares them with the UI, which is exactly what
`owned_channels` (ID3) exists to arbitrate. **The arbitration is not implemented at I9** — it is the
`Keys` bit on the capture mask, and a host that needs it takes capture. Recorded here so it is a
decision, not a surprise.

---

## 8 · Risks

### IR1 — R3 lands on this plan, and the blast radius is the whole spine

The architecture's R3 (routing vs propagation) is *this plan's* risk, because every state machine
here keys on "the target": drag, scroll, tooltip, gesture. Five independent retained UIs converged on
full propagation — weak evidence about performance, **strong** evidence about expressiveness — and
"handle it in the parent unless the child consumed it" is not expressible in the default path.
*Mitigation, and it is why I3 is early:* the hatch exists before its first consumer, ID4 keeps the
resolve output in **one** named type so widening *target* → *path* is one edit, and I3's gate 4
instruments adoption so the question is answered by data at the end of the campaign. *Residual:* if
adoption says invert the default, that is a rewrite of I2–I6, not an extension.

### IR2 — I8 is the largest rung and its correctness is invisible to every automated gate that exists

Caret placement, cluster boundaries and affinity are *visual* correctness. The GPU goldens skip on a
device-less machine (`ui_rect_gpu_golden.rs:36-39`) and `ui_hud_screenshot.rs` is `#[ignore]`d eight
times, so "green CI" may have exercised nothing.
*Mitigation:* every I8 gate is a **CPU-side** assertion on `CaretStop` values and on `UiTextBuffer`
bytes — numbers, not pixels — plus a Miri leg on the UTF-8 invariant, plus the sprites plan's D32 host
rung for owner-eval. The recorded lesson from the `boyko_app` host campaign is exact: owner-eval
caught three bugs there and the autogates caught **zero**.

### IR3 — I0 and I7 edit two crates this plan does not own

`boyko_rhi_vulkan/src/window.rs` and `boyko_input/src/win32.rs` are shared with the render and input
campaigns. The `window_proc` capture arm is a single `match` that four messages join at once
(`WM_MOUSELEAVE`, `WM_SETFOCUS`, `WM_KILLFOCUS`, `WM_CHAR`).
*Mitigation:* I0 and I7 are deliberately **separate rungs on the same files** so each has its own red
mutation and its own review, and both keep the existing layering rule (`window.rs:26-36`) unchanged:
`boyko_rhi_vulkan` gains no `boyko_input` dependency and every translation stays at the
`ingest_captured` edge. *Residual:* a concurrent edit to `window_proc` from another campaign conflicts
textually; it is a merge, not a design problem.

### IR4 — this plan adds eight components to a vocabulary whose table may not exist yet

If D7 has not landed, each of §4's eight is five hand-written landings — forty landings, plus the
diagnostics corpus rows.
*Mitigation:* the ladder does **not** block on D7. The two components the campaign cannot proceed
without (`Interaction`, `Focusable`) land at I1 by hand, which is ten landings and unblocks the Aether
plan immediately. The remaining six ride whichever surface exists when their rung arrives. *Residual:*
landing them by hand and then migrating them to the table is duplicated work — bounded, and cheaper
than blocking eleven rungs behind a framework.

---

## 9 · Open questions for the owner (VALUES / SCOPE — also to be filed in `docs/OPEN-QUESTIONS.md`)

Perf and architecture forks are decided above, with reasons or with measurement obligations. These are
scope and values calls.

1. **IME.** This plan assumes Latin-only entry for v1 and records `TextPreedit`'s *shape* so the
   deferral is a decision and not a hole (the architecture's D20 route is IMM32, not TSF). Confirm, or
   pull IME into I8 — it is a large addition and it changes I8's size from L to XL.
2. **The key set that I9 turns on globally.** Shift+Tab, arrows, Escape and Space begin working the
   moment I9 lands, and a host reading those keys for gameplay now shares them with the UI (§7). Is
   the `owned_channels` `Keys` arbitration wanted **in** I9, or is "a host that needs it takes
   capture" the accepted answer for v1?
3. **Bubbling.** The architecture's D17 recommends "opt-in escape hatch" and this plan lands it at I3.
   The owner may prefer **never** — defensible, simpler, and it makes IR1 a closed question rather
   than a tracked one. It would delete I3 and shrink I4.
4. **Multi-line text.** I8 ships single-line. Multi-line wants a `Resource`-owned rope column — a
   different storage decision that no in-tree consumer needs yet. Confirm the line is here.
5. **The `.ui` capability gap (dead datum #8) is being closed by I1 for `Interaction` and
   `Focusable`, but the format's round trip is already lossy for 12 of its 20 parse arms** (§1). Is
   closing that a goal of this campaign, or does it stay with whoever lands D7? This plan assumes the
   latter and only adds its own two to the serialized set.

---

## 10 · Sources

**In-tree, read for this document** (worktree `D:/wt/ui`, branch `feat/ui-advanced`, 2026-08-21):
`crates/boyko_ui/src/interaction/{focus,dispatch,action,components,plugin,mod}.rs` ·
`crates/boyko_ui/src/{components,layout,widgets,bundles,plugin,resources}.rs` ·
`crates/boyko_ui/src/text/{dispatch,shape,measure,serialize}.rs` ·
`crates/boyko_ui/src/reload/{reconcile,tree_view}.rs` ·
`crates/boyko_ui/src/binding/components.rs` · `crates/boyko_ui/Cargo.toml` ·
`crates/boyko_ui/tests/{p4_common/mod,p4_focus_hittest,p4_input_seam,p6a_button_dispatch}.rs` ·
`crates/boyko_input/src/raw/{event,queue,keycode}.rs` · `crates/boyko_input/src/win32.rs` ·
`crates/boyko_rhi_vulkan/src/window.rs` (the pump at `:455-495`, `window_proc` at `:611-700`,
`CapturedMsg` at `:89-107`) · `crates/boyko_app/src/runner.rs` (`ingest_captured` at `:961-980`) ·
`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` (the complete filter vocabulary) ·
`crates/boyko_ecs/src/ecs/core/component/observers/{trigger,traversal,propagate,entity_store}.rs` ·
`crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs` ·
`crates/boyko_ecs/src/ecs/core/time/time.rs`

**Campaign documents:** [`UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) (the authority)
· [`UI-ADVANCED-RESEARCH-INTERACTION.md`](UI-ADVANCED-RESEARCH-INTERACTION.md) (the evidence, which
carries the full external citation list — browser pointer capture, ImGui `ActiveId`/`SetKeyOwner`,
Godot `gui.mouse_focus` and `force_pass_scroll_events`, Unity `pointerPress`/`Selectable.FindSelectable`,
Bevy `bevy_input_focus`/`bevy_picking`, UIKit `decelerationRate`, Chromium scroll routing, Flutter
`GestureArena`, winit `Ime`, IMM32/DXUT — rather than duplicating it here) ·
[`PARTICLES-PLAN.md`](PARTICLES-PLAN.md) (the register format this plan follows).
