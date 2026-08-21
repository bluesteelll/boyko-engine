# UI-ADVANCED — Architecture

**Campaign:** advanced UI/GUI for `boyko_ui` — sprites, animation, richer interactivity — and its
integration into the Aether DSL as a first-class construct.
**Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`) · **Date:** 2026-08-21 · **Status:** design, pre-implementation, **revision 2**.
**Inputs:** `docs/UI-ADVANCED-RESEARCH-{SPRITES,ANIMATION,INTERACTION,DSL}.md`, plus a direct re-read
of `boyko_ui`, `boyko_render/src/ui/`, `boyko_ecs`'s dense/time/observer subsystems, and
`aether_lang`.

This document is a **design over a measured gap**, not a greenfield plan. `boyko_ui` is ~25 k lines
of shipped layout, text, binding, hot-reload and interaction. Everything below either consumes an
existing mechanism or says explicitly why a new one is required.

---

## 0 · How to read this

Every decision is numbered **D<n>**, carries a **reason**, and names the **alternatives rejected**.
Every claim about cost is either arithmetic (labelled *arithmetic*), a citation to a file (labelled
with `path:line`), or an explicit **measurement obligation** (§10) — this campaign does not accept
asserted numbers, and §10 is the list of numbers that must exist before the rung is called done.

Three of the research corpus's own recommendations are **corrected** here on the strength of kernel
facts established by reading (§2). They are called out rather than quietly dropped, because the
research documents remain the campaign's reference and a silent divergence between them and this
design is exactly the doc-rot this project keeps recording.

### Revision 2 — what changed and why, so the diff is not archaeology

Revision 1 was reviewed and four decisions did not survive contact with the tree. They are corrected
**at their source**, and each correction states the defect it replaces rather than quietly
substituting the right answer — the same discipline §2 applies to the research corpus.

| Changed | Was | Now |
|---|---|---|
| **D19 / D19a** (§6.4) | `ScrollPosition` offsetting children **at layout time**, written every frame by `ScrollMomentum` — which trips `ui_layout_discovery`'s single dirty bool and re-solves **every root** on every frame of every fling. The document's own cautionary tale (Bevy #22893), shipped. Its mitigation could not apply: FLIP needs two fixed endpoints and a fling has none. | Scroll is a **Tier-2 composite channel** folded during the gather's and the hit-test's existing DFS descents. Zero relayouts per fling — a pass/fail gate in §10.4. D10 gains the scroll row it never had; D11 gains FLIP's precondition. |
| **D31** (new, §3) | The campaign's central data path crossed a crate edge that does not exist and that both `Cargo.toml`s forbid; R2's only mitigation had no legal home, and D10's "bump the D6 generation" was not expressible from `boyko_ui` at all. | `boyko-ui` becomes a **production** dependency of `boyko_render` — the direction the tree's own layering rule names — and the canonical gather ships there. One spelling of the pack-input set feeds both the gather and D6b's discovery system. |
| **D7a–c** (§3) | Justified by the cost of *not* doing it, with a reuse claim (`register_bind_accessor`) that is true of the installation shape and false of the content. Unbounded, unmeasured, and first in the sequence. | Both sides counted; the "~15 components" figure corrected to 12 authored + 12 runtime-only (and the exclusion made a **safety** property); the real parts list enumerated; an escape hatch for the four irregulars; §10.9 as the rung's gate; **R4** as its risk. |
| **D14** (§5.6) | `Query<(&Interaction, &UiStateTint, &mut TweenTint), Changed<Interaction>>` presented as the mechanism — but `&mut TweenTint` matches only rows that already exist, so a button at rest was excluded from the query meant to start its transition. | The kernel gives the *diff* for free (unchanged, and still a strict improvement); the *start* is a structural insert, handled by the crate's existing discovery/exclusive-apply split. No lost first frame, and the reversing factor has a row to read. |

Seven smaller corrections are folded in at their sites: C3's conclusion restated inside D9 (two dense
stores are not co-indexed); **D9b** fixing one fused tick rather than four serialised writers of
`UiVisual`; the gather's per-node probe cost admitted in D5 and measured in §10.8; **D32** promoting
the observer from an open question to a v1 rung; D27's emitted path corrected
(`::boyko_ui::OnClick` does not resolve) plus the missing `aether-tests` dependency; the
`dense_iter`-versus-`Changed` kernel question attached as a condition on D9's reason 2; the
`BINDLESS_TEXTURE_CAPACITY <= 1 << 12` const-assert added to D1's lockstep list; and `UiSheet`'s
implicit padding spelled.

---

## 1 · The gap, in one table

Everything in this table was established by reading, not assumed. It is the compressed form of the
four research documents' inventories.

| Capability | State today | Anchor |
|---|---|---|
| Sprite rendering | `UiImage` is authorable in `ui!`, `.ui` and `ImageBundle`, and **renders nothing** — the pack never reads it; `PackInput` has no texture and no tint field | `components.rs:440`, `ui/pack.rs:21` |
| UI texture table | Does not exist. `UiImage.texture` is documented as a handle into the "(future) UI texture table" | `components.rs:441` |
| Second descriptor set on the UI pipeline | Not used. UI builds through the one-set `GraphicsPipelineDesc`; the 2-set builder exists and the UI never calls it | `ui/resources.rs:225`, `rhi_impl/device.rs:2112` |
| Animation | **Nothing.** No easing, no curve, no track, no time-varying driver. `boyko_ui` has never named the clock | crate-wide grep; `Time` at `time/time.rs:38` |
| Scroll | One doc comment naming a hypothetical `UiScroll`. Wheel is aggregated and unread | `reload/reconcile.rs:49`; `raw/queue.rs:186` |
| Drag | One doc comment ("drag handles"). `Interaction` has 3 variants; press **position** is never stamped | `interaction/components.rs:28`, `focus.rs:41` |
| Text input | `RawInputEvent::Text(char)` exists, is **never produced** by the Win32 translator, and is **explicitly discarded** on ingest with a test asserting it stays discarded | `raw/event.rs:29`, `raw/queue.rs:302,578` |
| Hover/press **visual** response | Bits produced, nothing consumes them for styling. No system maps `Interaction` onto any visual component | `focus.rs:385,416`; `UiBackground` is authored-only |
| Keyboard nav | Exactly two keys: Tab (no modifier read ⇒ no Shift+Tab) and Enter | `focus.rs:543,556` |
| Computed clipping | No system computes `ComputedClip`; layout never reads or derives it. Policy is "allow overflow" | `components.rs:188` |
| Multi-pointer | `MAX_POINTERS = 1`; every read hardcodes `slots[0]` | `focus.rs:37`, `dispatch.rs:44,53` |
| The O(1) render change gate | **A dead datum.** `last_seen_generation` is read by nobody, `bump()` is called by nobody outside its own unit test, and `pack_sort_upload` contains no gate | `ui/pack.rs:154`, `ui/upload.rs:162` — verified |
| A production host that draws UI | None. `boyko-ui` is a **dev-dependency** of `boyko_render`; `boyko_app` never draws UI | `boyko_render/Cargo.toml:113` |
| UI shader under the eDSL | No sentinels, no leaf, no manifest row, no re-DXC gate. The only pin is a const-generic byte **length** | `ui/mod.rs:122,129` |
| Aether `ui` construct | Does not exist. Nine constructs, nine parse arms, nine `keyword()` rows | `parse.rs:68`, `ast.rs:152` |

---

## 2 · Three corrections to the research corpus

These are the load-bearing findings from re-reading the kernel. Each invalidates a specific
recommendation in the research, and each is pinned to a file.

### C1 — dense storage and the EnableTag bit are **mutually exclusive on the contiguous path**

`UI-ADVANCED-RESEARCH-ANIMATION.md` §6.2 recommends **both**, and calls the pairing "the single most
important application of the engine's own rule": dense columns for storage, the EnableTag bit for
"a tween is running right now".

They cannot both be had on the fast path. `Query::dense_iter` / `dense_iter_mut` **const-reject an
enable-bearing filter at monomorphisation**:

> "The dense fast path strides the archetype-agnostic `DenseStore` column directly (no per-slot
> `(archetype, row)` context) and therefore cannot honor a per-row enable term keyed by
> `(archetype, row)`. Callers who want `Query<&mut Dense, Enabled<Tag>>` iteration use `iter_mut()`
> — the archetype-walking cursor whose per-row `filter_fetch` enforces the bit."
> — `boyko_ecs/src/ecs/core/iters/query/query.rs`, `assert_dense_iter_no_enable`

`Query<&mut Dense, Enabled<Tag>>` **is** sound and Miri-gated (`tests/dense_enable_query_miri.rs`),
but it is the *gather* path — precisely the path the dense column was chosen to avoid.

### C2 — dense insert/remove is **not** an archetype migration, so the problem the EnableTag solved does not exist here

The EnableTag was invoked because "a button hovered 500 times pays 1 000 archetype moves". For a
**dense** component that premise is false, and it is pinned by a test:

> "dense insert/remove does NOT change the entity's archetype id (no-migration)"
> — `boyko_ecs/tests/dense_d2_routing.rs`, gate list

Start/stop of a dense row is a free-list push/pop plus an observer fire — not a structural move.
**C1 and C2 together resolve into D9**: presence of the dense row *is* "running", removal *is*
"stopped", and no EnableTag is needed on the tween columns at all.

### C3 — "the write-back is a same-row write, no scatter" is **false as stated**

Research §6.2 claims the per-channel tick has "no scatter" because the write-back is a same-row
write into the sink component. It is not: the tween lives in a `DenseStore` keyed by `EntityId`, the
sink (`UiBackground`, `UiText`) lives in an archetype column. Writing across them is either a
random-access `get_component_mut` per row, or a mixed query that falls back to the archetype gather.
The contiguous SIMD-shaped loop the research describes **only exists if the tick writes into its own
storage.**

This is the finding that shapes the whole animation design (D8, D10): the tween's output lands in a
**dedicated visual sink** that the pack reads, never in the authored style components — which
simultaneously buys the tier enforcement (D10) that the research asked for and could not get from a
convention.

**And the sink does not dissolve the scatter — it relocates it.** `TweenTint` and `UiVisual` are two
independent `DenseStore`s, so the write-back is still a per-row lookup, dense→dense instead of
dense→archetype. C3's conclusion therefore holds against D9 as well as against the research, and D9
says so in full (§5.1) rather than leaving this section to read as if the sink answered it.

**And the honest consequence, recorded up front:** at a UI's realistic N (tens of concurrent
animations), the contiguity is worth nanoseconds. Research §9.2 says so itself. This design therefore
does **not** justify its storage shape by throughput — it justifies it by tier enforcement, by the
absence of a side store, and by C2. §10.5 is the bench that will say what the tick actually costs,
and the design is built so that the answer does not change the architecture.

---

## 3 · Cross-cutting decisions

These bind all four subsystems and must be settled before anything is written, because each is
cheap once and expensive twice.

### D1 — `UiInstance` widens 64 B → 80 B **once**, with the final field list fixed here

`corner_radius` is currently aliased as the glyph UV under `FLAG_TEXT` (`instance.rs:50-56`), and
the source names the exit condition verbatim. Sprites trigger it; animation would trigger it again.
The two must not widen it twice.

```
@0   min_px        [f32; 2]   // physical px, offset+scale folded at pack (D5)
@8   size_px       [f32; 2]
@16  clip          [f32; 4]   // AABB, physical px, FLAG_CLIP_PRESENT
@32  corner_radius [f32; 4]   // ALWAYS the radius — the alias is retired
@48  uv            [f32; 4]   // NEW: normalized (u0,v0,u1,v1) — glyphs AND sprites
@64  color         u32        // premultiplied fill / tint
@68  border_color  u32
@72  border_width  f32
@76  flags         u32        // bits 0..3 flags, bits 20..31 bindless slot
                              // 12 bits ⇒ slots 0..4095 — see the capacity assert below
                              // = 80 B, multiple of 16, no tail pad
```

**The 12-bit slot field has exactly zero headroom, so it is const-asserted against the table it
indexes.** `BINDLESS_TEXTURE_CAPACITY = 4096` (`boyko_rhi_vulkan/src/bindless.rs:72`) and the
allocator issues `1..capacity` (`boyko_render/src/bindless.rs:80-93`), so the maximum live slot is
4095 and it fits bits 20..31 *precisely*. D3 then commits the UI to sharing that table with world
materials and refuses any reservation — which makes "raise the capacity" the natural response to slot
pressure, and a raised capacity would **silently truncate** the field and make a UI quad sample a
different texture. Therefore, beside the field:

```rust
const _: () = assert!(BINDLESS_TEXTURE_CAPACITY <= 1 << 12,
    "UiInstance.flags carries the bindless slot in 12 bits (20..31)");
```

**Reason:** one field, one meaning. The text lane migrates to the real `uv` in the same commit, so
`corner_radius` stops carrying two meanings and a rounded avatar / nine-slice chip / tinted bordered
icon becomes representable.

**Rejected:** (a) *Godot's union* — honest about the two variants, but it forbids a node that is
both rounded and textured, which is the exact case that triggered the widening. (b) *Keep 64 B and
alias a third meaning* — the option that leaves no trace of the choice. (c) *96 B with an explicit
offset/scale pair* (research §6.5) — unnecessary, see D5.

**Cost, arithmetic:** at 2 048 nodes, ring traffic per frame 128 KB → 160 KB, touched twice by the
sort gather. Measured obligation §10.2.

**Lockstep sites — all seven, in one commit:** the Rust struct; the nine `offset_of!` const-asserts
(ten after `uv`); the `BINDLESS_TEXTURE_CAPACITY <= 1 << 12` const-assert above; `ui_rect.vs.hlsl`
and `ui_rect.fs.hlsl`; the two `SpirvBlob<N>` byte-length pins (`ui/mod.rs:122,129`);
`pack_ui_instance`; the Miri byte-view test. See **R1** — this is the campaign's
highest-consequence, lowest-observability change.

### D2 — sprites use **bindless set 1**, not an atlas and not batch-by-texture

`UiImage.texture` **is** `TextureGpu.bindless_slot`. The UI pipeline is built through the existing
`create_graphics_pipeline_bindless(desc, set1_layout)` (`rhi_impl/device.rs:2112`); the fragment
shader samples `g_textures[NonUniformResourceIndex(slot)]`.

**Reason:** zero new mechanisms. The 4096-entry table, the free-list allocator, the fence-gated slot
recycle, the magenta error slot, and the boot gate (`bindless_capable` is already a boot requirement)
all exist and are already used by the raster path. The crate's single best property — **one pipeline,
one `draw(6, N, 0, 0)`, one global z-sort, clipping carried per-instance rather than as pipeline
state** — survives untouched. Every one of the six engines surveyed breaks its draw on a texture
change; ImGui and RmlUi break it on a *clip* change too. Sprites must not be the feature that spends
that.

**Rejected:** (a) *Runtime UI atlas* — needs an allocator **with eviction** (a new mechanism and a
side-store hazard), forbids `REPEAT` so tiled nine-slice becomes geometry, needs per-tile padding and
a per-tile mip policy, caps sprite size at `MAX_ATLAS_DIM`, and turns a resize into a repack stall.
(b) *Batch by texture* — spends the one-draw property; `UiFramePlan` becomes a variable-length batch
list and the draw count becomes content-dependent.

**The strongest argument against D2, recorded because it is substantially right:** WebRender — the
one team whose entire job is drawing UI, and the only one that reasoned about this fork in public —
chose atlases, and its author's stated reason ("the worst part is driver bugs") is a portability
argument this engine does not share. But three of its reasons do transfer: UI textures are small,
few and long-lived (the atlas profile); the 4095 slots are shared with world materials; and
`NonUniformResourceIndex` is **the first non-uniform thing in this shader**, whose design note is
explicit that the text branch is "a uniform-per-instance branch, so the rect majority is
unregressed". §10.1 is the gate that must answer this before the rung closes.

**Why D2 is still the right start: reversibility.** `UiImage { texture, uv_min, uv_max }` describes
an atlas tile and a bindless slot *equally well*. If §10.1 finds the divergence cost real, Model A is
reachable **without changing one component**. That asymmetry is the whole argument.

### D3 — no bindless slot reservation for the UI; slot pressure is an **asset-time** concern

The UI registers textures through the same `BindlessSlotAllocator` as everything else.

**Reason:** a second table would itself be the Principle-0 violation the research names. A hard
partition strands slots in whichever half is idle. Slot pressure has a correct answer that costs
nothing at runtime — pack the icon set into one texture **offline**, reusing the existing skyline
packer (`boyko_fontbake/src/atlas.rs:138`), and spend one slot. Atlasing becomes an asset-packing
choice rather than a renderer fork, which is precisely what bindless buys.

**Rejected:** a reserved UI sub-range (strands slots, and nothing else in the engine reserves
anything today, so it would be a new policy with one consumer).

### D4 — in-node paint order is a **pinned emission contract**, not a new sort field

Today text quads carry their node's `StackIndex` verbatim (`text/emit.rs:123`), so a label and its
own background share a key and their order is decided purely by the order the host appends them.
Sprites add a third kind of quad into the same ambiguity.

The sort key `(StackIndex, append_order)` is already **total** — `append_order` is unique — so the
sort is not ambiguous; the *emission* is. The fix is to specify emission order and pin it:

> **background rect → nine-slice sub-quads (TL..BR) → image → glyphs → focus ring**, per node.

**Reason:** cheapest correct answer. It adds no instance byte, no key-lane width, and no sort cost.
`StackIndex` remains available for an author who needs to override.

**Rejected:** a third sort key / sub-order byte — pays a permanent widening of the key lane to
express something the append order already encodes correctly once the order is stated.

**Consequence made explicit:** `StackIndex` is read by **both** the renderer's sort and
`ui_focus_system`'s hover resolution (`focus.rs:301`). Therefore **`StackIndex` is never an animation
channel** (D10's Tier table excludes it). A hover z-lift is an authored `StackIndex` write, and the
fact that the lifted card also hit-tests on top is *correct*, not a hazard — but it must be an
author's deliberate act, never a side effect of a visual tween.

### D5 — the Tier-2 visual transform is **folded at pack** and costs **zero** GPU bytes

Research §6.5 asks for `offset_px[2]` + `scale[2]` on `UiInstance` (+16 B on every instance). Not
needed. The pack already folds `scale_factor` into every length (`pack_ui_instance`), so it folds the
visual transform in the same multiply:

```
min_px  = (rect.xy + visual.offset) * s
size_px = (rect.wh * visual.scale)  * s
color   = premultiply(straight) * visual.opacity     // opacity: zero bytes, zero shader change
```

**Reason:** `ComputedRect` keeps its single-writer invariant (`widgets.rs:6-9`) untouched — the pack
*reads* two components and folds them; it never writes geometry. Opacity rides the existing
premultiply.

**"The 95 % of nodes that animate nothing pay nothing" is true of GPU bytes and of tick cost, and
false of the gather — so the exception is stated here rather than left to be discovered.** Reading
`UiVisual` at gather time is a random-access `get_component` probe **per node per frame**, and the
sprite lane adds four more (`UiImage`, `UiNineSlice`, `UiSpriteSheet`, `UiSpriteCursor`). That is the
same cost class D23 identifies as the likely dominant term in the interaction spine — *"~6
random-access `get_component` probes per node per frame"* (`focus.rs:221-234`; verified:
`ComputedClip`, `ComputedRect`, `Interaction`, `StackIndex`, `FocusPolicy`, `Children`) — and it is
the one cost this campaign adds unconditionally to every node of every frame. Two things follow, both
now specified: the D6 generation compare is hoisted **ahead of the gather** so a static frame pays
zero probes (D6a), and the gather's per-node probe cost is a **measurement obligation** (§10.8),
because it is otherwise the largest number in the campaign that nothing in §10 was watching.

**Rejected:** (a) instance fields (+16 B on every instance for a field most never use);
(b) writing `ComputedRect` from the animation system (breaks the single-writer invariant and re-enters
the relayout path).

**Deferred with a reason:** *rotation*. It cannot fold into a min/size pair, it invalidates the
axis-aligned per-instance clip (`instance.rs:40`), and it invalidates the axis-aligned hit-test
(`focus.rs:271-287`). It is a coherent later rung — instance clip transform + eDSL change + `.spv`
re-bless + a WebRender-shaped per-frame transform table for the hit-test — and pulling it into v1
would triple the surface of three subsystems at once.

### D6 — the repack generation gate is **implemented and made per-slot**, or the claim is deleted

Today the module doc states "**O(1) generation gate** — short-circuits on
`gen == last_seen_generation` … a static frame does nothing" (`ui/upload.rs:6`) and "the
0%-when-static guarantee" (`ui/pack.rs:193`). Verified: `pack_sort_upload` (`upload.rs:162`) contains
no compare. It unconditionally `clear()`s, repacks every node, re-sorts and re-memcpys.

The gate is **implemented**, and the specification is fixed while the record is being touched:

```rust
pub struct UiRenderScratch {
    /// One per frame-in-flight. A change must reach EVERY ring slot before a skip
    /// is legal; a single scalar cannot express that and would serve a stale frame
    /// from the second slot.
    last_seen_generation: [u64; FRAMES_IN_FLIGHT],
    ...
}
```

**Two wiring facts the original record left implicit, and both are load-bearing:**

**D6a — the compare goes *before* the gather, not before the pack.** Today's seam runs
`gather_nodes(world, node_buf)` and *then* `pack_sort_upload`
(`ui/upload.rs:255,272` — `host_upload_frame_from_world`). A gate that sits inside `pack_sort_upload`
still pays the whole world read: one `get_component` probe per pack-input component per node per
frame. That is the **only** cost this campaign unconditionally adds to every node of every frame
(D5 and §4.1 add four more probes per node for the sprite lane), and it is the cost D23 independently
identifies as the likely dominant term in the sibling subsystem. So the generation compare is hoisted
to the top of `host_upload_frame_from_world`, ahead of the gather: a static frame then costs **one
`u64` compare and zero component probes**, which is what the module doc has always claimed. §10.8 is
the number.

**D6b — the bump is a render-side discovery system, because the generation is a render-side
`Resource`.** `UiRenderGeneration` lives in `boyko_render` (`ui/pack.rs:196`) and every writer of a
pack input lives in `boyko_ui`. That crossing is settled by **D31**, and with D31 in place the bump is
not scattered across the writers at all — it is one system, `ui_render_discovery`, shaped exactly like
`ui_layout_discovery`: a normal system whose `Query<(), Or<(Changed<…>, …)>>` over the pack-input set
bumps the counter once per changed frame. One site, not fifteen, and no writer has to remember.

**Reason:** anything built on this path inherits the belief that a static UI is free. Sprites make
each instance more expensive and animation guarantees a change most frames, so the gate's *absence*
would be attributed to this campaign's features. Leaving the doc and the hole together is exactly how
the recurring "dead datum" defect hides a regression — and this one is the **sixth** recorded instance
of that class in this project.

**And the honest half:** the gate cannot help an animated frame. §10.3 must report *both* numbers —
repacks avoided on a static frame, and the unchanged full cost of an animating frame.

**Rejected:** (a) leave both the claim and the hole (the option with no trace); (b) delete the claim
and skip the gate — defensible, but the gate is ~15 lines and a static HUD is the common case.

### D7 — the `.ui` component vocabulary becomes a **registration table**, before any new component is added

`parse_and_insert` (`text/dispatch.rs:71`) is a hand-written closed `match` over 19 type-names.
Today, **every new component costs five hand-written landings**: a `.ui` dispatch arm, a `.ui` field
parser, a `serialize.rs` arm (or `.ui` round-trip breaks), a `reload/reconcile.rs` `TextStruct` impl
(or hot reload silently drops it), and a row in the equivalence gate. The Aether construct makes it
six.

**Decision:** convert the vocabulary into a registration table installed by the derive. One table,
spelling and dispatch together.

#### D7a — the corrected arithmetic, counted on **both** sides

The original record justified D7 by the cost of *not* doing it and stopped there. Both halves are
counted here, because D7 is sequenced first and gates the whole campaign.

**The "fifteen components" figure was wrong, and correcting it is itself a decision.** Most of what
this campaign adds is **runtime-written state**, and runtime state is deliberately **not
`.ui`-authorable**:

| Class | Components | `.ui` landings each |
|---|---|---|
| **Authored** — the author declares them, they round-trip, hot reload must preserve them | `UiNineSlice`, `UiSpriteSheet`, `UiSpriteAnim`, `UiStateTint`, `Overflow`, `ScrollPosition`, `Draggable`, `DropTarget`, `TextInput`, `FocusNeighbors`, `FocusGroup`, `Tooltip` — **12** | 5 |
| **Runtime-only** — a system adds/removes them; an author never writes one | `UiVisual`, `TweenTint`, `TweenOpacity`, `TweenOffset`, `TweenScale`, `UiSpriteCursor`, `ScrollMomentum`, `ScrollExtent`, `DragActive`, `TextCursor`, `TextPreedit`, `HoverDwell` — **12** | **0** |

That the second column is empty is **not** an omission — it is the same structural-safety property
`parse_and_insert`'s doc already claims for the closed `match`: *"a `.ui` file can ONLY construct UI
components — a structural safety property for untrusted/hand-edited text"* (`dispatch.rs:5-8`).
Extending it: a `.ui` file must not be able to inject a running tween row, a `DragActive`, or a
`TextPreedit` into a live world. The vocabulary table's membership is therefore **opt-in per
component**, and runtime state does not opt in.

So the cost of *not* doing D7 is **12 × 5 = 60** hand-written landings, not 75.

**And the cost of doing it, which the original record did not state at all.** The claim that the table
is installed *"exactly as `register_bind_accessor` already installs a `ComponentId`-keyed fn-pointer
table"* is true of the **installation mechanism** and false of the **content**, and the difference is
the whole estimate. `Bindable` (`binding/bindable.rs:23-46`) is **read-only** — `fmt_field`,
`value_field`, `field_id`. It has no parse side and no write side. The path D7 replaces states its own
shape verbatim:

> "Per-field value parsing is TYPE-DIRECTED by the destination field (Decision 4): there is NO
> standalone 'parse a value' function." — `text/dispatch.rs:12-14`

and gives the reason: `Stretch(f)` is `Unit::Stretch` only in a `Unit` field and `AlignCross::Stretch`
only in an `AlignCross` field. A derive-generated table is therefore a **new type-directed
parse/serialize codegen framework**, and this is its full parts list:

* a new `UiField` trait — `parse(&str) -> Option<Self>` plus `write(&self, &mut String)` — and one
  impl per destination field type: `Unit`, `AlignCross`, `AlignMain`, `LayoutType`, `PositionType`,
  `AnchorEdge`, `TextAlign`, `FontId`, `f32`, `u32`, `u8`, `bool`, `[f32; 2]`, `ComponentId`,
  `TemplateId`. **The bodies already exist** as the free leaf fns at `dispatch.rs:545-875`
  (`parse_unit`, `parse_layout_type`, `parse_align_main`, `parse_align_cross`, `parse_position_type`,
  `parse_anchor_edge`, `parse_font_id`, `parse_text_align`, `parse_f32`, `parse_u32`, `parse_u8`,
  `parse_bool`, `parse_f32_pair`, `parse_component_id`, `parse_template_id`) — so the impls are
  delegations, not new parsing logic. **This is the reuse D7 actually has**, and it is real; it is
  simply not `Bindable`.
* `UiParseReport` line/column threading into the generated body, so a bad field is still a recoverable
  per-field error at the right span rather than a silent `Default`.
* the derive itself, emitting three outputs from one spelling: the parse arm, the `serialize.rs`
  writer, and the `reload/reconcile.rs` `TextStruct` impl.

#### D7b — the escape hatch, because four of the nineteen are irregular and always will be

The existing `match` contains cases a field-wise derive cannot express, and pretending otherwise is
how this rung would overrun:

* `StackIndex` is **tuple-only** — `StackIndex { .. }` is a hard error (`dispatch.rs:118-125`);
* `UiRoot`/`Button`/`Bar`/`BarFill` are **bare markers** — a body is an error;
* `BindText`/`BindValue` return `BindParse<C>` and can **defer the whole insert to pass 2** when
  `source` is a `#name` (`dispatch.rs:47-62`) — a two-pass control-flow shape, not a field parse;
* `OnClick`/`OnHover`/`OnSubmit` route through `resolve_action_name` (`boyko_input`) and fall back to
  the `NO_ACTION` sentinel.

`#[derive(UiVocab)]` therefore takes `#[ui_vocab(manual)]`, which registers the type in the table and
keeps the hand-written arm as its parse fn. **The derive covers the regular case; the four irregulars
keep the code they already have.** A table that could not express them would be a table nobody could
finish.

#### D7c — the pin that bounds the rung: reproduce the nineteen before adding the twentieth

D7 lands as a **refactor with a byte-level pin**, never as a new feature: the generated table must
reproduce the existing hand-written behaviour for all 19 components — same worlds, same
`UiParseReport` diagnostics, same round-trip bytes — **before any new component is added**. §10.9 is
that gate, and it is what makes the rung's end condition observable instead of a judgement call. If
some component cannot be reproduced, it takes `#[ui_vocab(manual)]` and the rung still closes.

**Reason D7 keeps its place at the head of §11 even after the honest count:** 60 hand-written landings
with a *silent* failure mode (hot reload drops the component; the round trip loses it) against one
framework whose parsing bodies already exist and whose end condition is pinned. The ordering argument
was never that D7 is cheap — it is that its cost is paid once and the alternative's is paid twelve
times, and that doing it afterwards means writing the 60 landings and then deleting them.

**Rejected:** hand-writing the arms (linear, and each missed landing is a silent data loss);
deferring the table to a later rung (pays the cost first, then removes it); a table that also admits
runtime-state components (widens the `.ui` attack surface for zero authoring value).

**Risk:** this rung is the campaign's largest single unknown and is tracked as **R4** (§12).

### D31 — the canonical gather lives in `boyko_render`, which takes `boyko-ui` as a **production** dependency

This is the campaign's central data path and the original record never named the crate edge it
crosses. It is named and decided here, because **every** other decision that reads a UI component from
the renderer — sprites (D2), the `UiVisual` fold (D5), the generation bump (D6), the tier
enforcement's actual teeth (D10), R2's whole mitigation — is downstream of it.

**The measured state of the edge:**

* every new visual datum (`UiVisual`, `UiImage`, `UiNineSlice`, `UiSpriteSheet`, `UiSpriteCursor`)
  lives in `boyko_ui`;
* the type that must read them (`PackInput` / `UiNode`, `ui/pack.rs:21`, `ui/upload.rs:71`) lives in
  `boyko_render` and is deliberately `boyko_ui`-agnostic — *"so the pack is driven by a host-owned
  `Query` without this crate naming the query types"*;
* `boyko_ui` names **no** render crate, and its `Cargo.toml` records the reason three times
  (`Cargo.toml:22-40`: *"so boyko-ui takes NO render dependency"*, *"Acyclic"*, *"no production cycle
  is introduced"*);
* `boyko_render`'s `boyko-ui` line is a **dev**-dependency annotated *"GUI P6b screenshot test ONLY"*
  / *"TEST-ONLY"* (`boyko_render/Cargo.toml:106-118`);
* and no gather exists anywhere: `host_upload_frame_from_world` has **zero callers**, and every
  `PackInput` in the tree is a hand-written literal inside a test. Verified.

**Decision: `boyko-ui` moves from `[dev-dependencies]` to `[dependencies]` in `boyko_render`, and the
canonical gather ships as `boyko_render::ui::gather_ui_nodes` — a crate-provided function, not an
example closure.**

**Reason, and it is the tree's own layering rule rather than a preference.** `boyko_render`'s
`Cargo.toml:7-13` states the rule verbatim: *"boyko_render (the data bridge) and boyko_app (the host)
are the only two crates that name both the graphics RHI and the ECS core; boyko_app **must not define
per-entity GPU data paths** — those belong in boyko_render"*. A gather from ECS columns into
`Vec<UiNode>` **is** a per-entity GPU data path. The rule does not merely permit the choice; it names
it.

**The direction is the one the tree already declares legal.** The three `boyko_ui` comments are not
retired by this — they are what *makes it legal*, and they become load-bearing in the new direction:
`boyko_ui` still names no render crate, so the edge stays acyclic and one-way. Exactly **one** comment
changes: the `TEST-ONLY` annotation on `boyko_render`'s dev-dep line, which is rewritten to record the
promotion and its reason (the P6b screenshot test keeps working; it simply stops being the only
consumer).

**Rejected — (a) the gather in `boyko_ui`.** Requires the render dependency that crate refuses in
three places, and drags `boyko_rhi_vulkan` into every consumer of the layout solver. **(b) the gather
in `boyko_app`.** `boyko_app` already depends on `boyko-render` and would need a new `boyko-ui` edge —
and it is refused *by name* by the layering rule quoted above. **(c) a fourth crate
(`boyko_ui_render`).** Retires no comment and is genuinely clean, but it adds a graph node whose whole
content is one function, and it would put the pack (`boyko_render`) and the pack's only feeder in
different crates for no invariant that the promotion does not already preserve.

**What the decision buys immediately, beyond R2:**

1. **The generation bump becomes expressible** (D6b) — `ui_render_discovery` can name
   `Changed<UiVisual>` because the crate now sees the type.
2. **One list, three consumers.** The pack-input set is spelled **once**, in a `macro_rules!` that
   expands to *both* the `Or<(Changed<…>, …)>` filter type of `ui_render_discovery` *and* the
   gather's per-node read list. Adding a visual component to the list wires the gate and the gather
   together, or fails to compile. This is D7's move — *one table, spelling and dispatch together* —
   applied to the render seam, and it is the only form of R2's gate that cannot rot: a completeness
   test asserting "the gather reads every pack-input component" is checking a list against itself
   unless the two are generated from one spelling.
3. **The gather becomes a DFS over `UiRoot`/`Children`**, carrying the inherited clip *and* the
   inherited scroll offset on its stack — mirroring `collect_candidates` (`focus.rs:204-257`)
   line-for-line. That DFS is what D19 needs, and its pre-order **is** the paint order the hit-test
   already uses for `paint_seq`, so D4's emission contract and the interaction spine's z-order are
   the same traversal rather than two orders that must be kept in agreement.

**The cost, recorded honestly:** every consumer of `boyko_render` now compiles `boyko_ui` (~25 k
lines). `boyko_app` already depends on `boyko-render`, so the host build gains it; `aether-tests`
gains it transitively (see D27's dependency note). This is a compile-time cost, not a runtime one —
the gather is a plain function nobody has to call.

### D32 — a minimal **observer** is a v1 deliverable, not an open question

§12's R1 and R2 both terminate in the same sentence — *the campaign has no non-test observer* — and
§9 nonetheless commits v1 to ~24 components across five subsystems plus an eDSL migration plus a new
Aether construct. A plan cannot make observability its top two risks and then leave the observer to a
question at the end of the document.

**Decision: v1 ships a minimal `boyko_app` UI rung** — a `UiPlugin` registration, the D31 canonical
gather wired to `host_upload_frame_from_world`, one checked-in `.ui` file, and one checked-in sprite
sheet. It is a *rung*, not a showcase: one panel, one sprite, one animated hover, one scroll
container.

**The asset floor is genuinely zero and that is part of the cost.** Verified in this worktree: there
is **no** `.ui` file, **no** `.bfont`, and **no** sprite anywhere in the repository. So v1 creates the
first of each. The text leg is the one with slack: D8e makes the font atlas default-filled, so if the
`.bfont` bake slips, the rung still boots and draws its sprite and its panel — a font-less demo is
degraded, not blocked. There is no equivalent slack for the sprite sheet; it is the thing being
demonstrated.

**Reason.** This is the boyko_app host campaign's recorded lesson applied before it is re-learned:
*host render rungs need a golden-**independent** visual regression, because owner-eval caught three
bugs there and the autogates caught zero*. Everything that makes this campaign risky is invisible
without a host — the GPU goldens skip gracefully on a device-less machine
(`ui_rect_gpu_golden.rs:36-39`), `ui_hud_screenshot.rs` is `#[ignore]`d eight times, and the only pin
on the committed `.spv` is a byte length. A minimal rung converts R1 and R2 from *tracked* risks into
*observable* ones; without it their mitigations are unfalsifiable.

**It is sequenced early, not last** (§11 rung 2), because an observer delivered after the thing it
observes has observed nothing.

**Not an open question, and §13 is corrected accordingly.** Whether the campaign has an observer is an
observability call, which this document decides. What remains for the owner is the *scope above the
floor* — whether a richer demo scene is also wanted — and that is what §13 Q5 now asks.

---

## 4 · Sprites

### 4.1 The per-element data model

Capability is component **presence** throughout; every component is `#[repr(C)]` POD `Copy`.

| Component | Size | Meaning | Storage |
|---|---|---|---|
| `UiImage` *(exists, 24 B)* | 24 B | `texture` **reinterpreted as the bindless slot**; presence ⇒ textured lane | table (authored, cold) |
| `UiNineSlice` | 20 B | `border_px: [f32;4]`, `mode: u8` (`Stretch`; `Tile` lands at S5), `fill_center: bool`, `_pad: [u8;2]` — presence ⇒ pack emits 9 sub-quads (8 without the centre) **in addition to** the node's background | table (authored, cold) |
| `UiSpriteSheet` | 4 B | `sheet: u16`, `index: u16` — presence ⇒ pack derives `uv` from the sheet table instead of reading `uv_min`/`uv_max` | table (authored, cold) |
| `UiSpriteAnim` | 12 B | the **cold track**: `first: u16, last: u16, fps: f32, mode: u8` (`Forward`\|`Reverse`\|`PingPong`\|`Once`), `repeats: u8` — author-written, never system-written | table (authored, cold) |
| `UiSpriteCursor` | 8 B | the **hot cursor**: `elapsed: f32, frame: u16, dir: i8` — the only column the flipbook system writes per frame | **dense** |

**D8a — the cold-track / hot-cursor split.** `components.rs:5` already prescribes exactly this churn
split ("a node animating only its size bumps only `UiLayout`'s tick"). Keeping the track cold
preserves `Changed<UiSpriteAnim>` as a meaningful signal — "the author retargeted the animation" —
which a merged component would destroy.

**D8b — animation state is ECS data, not a shader clock.** A shader-side `frame = f(time)` is a datum
no system can query: no `animation_finished`, no "is this explosion done", no save/restore. Godot
exposes `frame` and an `animation_finished` signal; Unreal's flipbook exposes the keyframe. A shader
clock is the parallel-data-system failure **inverted** — the datum exists nowhere the ECS can see it.

**Rejected for the sheet table:** a `HashMap<name, sheet>` (banned, and mechanically blocked by
`clippy.toml`); a per-element owned frame array (Bevy stores `Vec<URect>` per layout plus a
`HashMap<AssetId, usize>` reverse map — both banned shapes here).

### 4.2 The sheet table

One `Resource`-owned dense column keyed by a dense `u16 sheet_id` — the `FontId` handle discipline
(`components.rs:440`: "a DENSE `u32` handle … NOT a string / `HashMap` key"):

```rust
#[repr(C)]
pub struct UiSheet {
    slot: u32,          // bindless slot
    cols: u16, rows: u16,
    frame_count: u16,   // ≤ cols*rows; trailing cells may be unused
    _pad: [u8; 2],      // SPELLED, not implicit — `inset_uv` needs 4-byte alignment
    inset_uv: [f32; 2], // half-texel inset against bilinear bleed
}                       // 20 B, no tail pad
```

The `_pad` is written out rather than left implicit because every other `#[repr(C)]` POD in this
document and in the crate spells its padding (`TweenTint._pad`), and because the crate's
`offset_of!` const-assert habit only catches a layout change if the layout is stated. The field is
also renamed `pad_uv` → `inset_uv`: sitting next to a `_pad` byte-padding field, "pad" meant two
different things in one struct.

**D8c — uniform grids only in v1; ragged sheets deferred.** A uniform grid makes the frame UV **pure
arithmetic** from `(cols, rows, index)` and needs *zero* per-frame storage. Ragged/trimmed sheets need
a real sub-rect table, and that is a second column with an asset-pipeline dependency for a case no
in-tree asset exercises (there is **no** checked-in `.ui` file and **no** checked-in sprite in the
repo). Deferred with the shape recorded, not with the question open.

**Variable per-frame durations** (Aseprite's canon) are covered at integer resolution by an optional
run-length `frame_run: u8` column — Unreal's `UPaperFlipbook` compression — rather than a per-frame
float table. Also deferred; uniform fps is v1.

### 4.3 Nine-slice — CPU expansion into the pack scratch

**D8d.** ~~`UiNineSlice` present ⇒ the pack emits **9 sub-quads** into the existing pack scratch;
absent ⇒ it emits 1.~~ **`UiNineSlice` present ⇒ the pack emits the node's background rect (unchanged)
PLUS 9 sub-quads (8 when `fill_center == false`) into the pack scratch; absent ⇒ it emits 1, exactly as
before.**

*(corrected 2026-08-21 at the S4 pre-build audit — see `UI-PLAN-SPRITES.md` **S-D11**. As written this
sentence said the sub-quads REPLACE the node's record, while **D4** at `:250` lists "background rect →
nine-slice sub-quads → image" as distinct elements, i.e. ADD; a third number had already reached the
tree at `crates/boyko_render/tests/ui_s0_seam.rs:245`. Three readings, three different values for
`UI_RECORDS_PER_NODE`, and the rung was unbuildable until one won. **ADD wins, and the reason is
correctness rather than economy:** a nine-slice source is a **frame**, and frames have transparent
regions — under REPLACE a translucent corner would composite against whatever is behind the entire UI
instead of against the node's own background, so the background rect is not redundant overdraw but the
surface the frame sits on. It is also why Bevy, Godot and Unity all keep the node's background beneath
the slice. REPLACE carries a second cost ADD does not pay at all: it would force a decision about how
one node's `corner_radius` and `border_width` distribute across nine sub-quads, which under ADD simply
does not arise — the sub-quads are uniform textured rects with zero radius and zero border, the shape
`pack_ui_image_instance` already has.)*

**Reason:** all three shipped strategies were compared and only this one keeps the crate's asset.
Bevy pays a **separate pipeline** plus shader plus 16 extra floats — disqualified outright, it breaks
the one-draw property. Godot carries `ninepatch_margins[4]` + `ninepatch_pixel_size[2]` in **every**
`InstanceData` — 24 B taxing the 95 % of nodes that are plain rects. Unity expands on the CPU, and it
is *cheaper here than in Unity* because:

* `UiRenderScratch.pack` is **already** the sanctioned frame-transient buffer — the 9 sub-quads need
  no new storage;
* all 9 inherit the parent's `StackIndex` and get **consecutive** `append` indices, so the existing
  total-order sort keeps them contiguous and in painter's order with **no change to the sort**;
* they inherit the parent's `ComputedClip` verbatim, so per-instance clipping composes for free;
* ~~radius and border stay meaningful on the sub-quads (they are ordinary rects), where a single-quad
  shader route would have to reconcile radius, border and slice math in one branch.~~ **the sub-quads
  need no radius and no border at all — they are uniform textured rects, the shape
  `pack_ui_image_instance` already emits — so the reconciliation a single-quad shader route would owe
  between radius, border and slice math simply never arises here.**

  *(corrected 2026-08-21 at the S4 pre-build audit. The struck clause claimed the opposite of the
  amendment fourteen lines above it, which settled that the sub-quads carry zero radius and zero
  border. It survived the D8d correction because that edit was made by quoting the sentence it
  replaced, and this one is in the supporting list rather than the claim — the doc-rot-repair hazard
  this project has measured: a repair that reads the sentence it is fixing and not the paragraph that
  argues for it. The list item is load-bearing prose: it is one of the three reasons CPU expansion is
  chosen over Bevy's separate pipeline.)*

Layout is untouched — slicing is purely visual.

**Tiled** (rather than stretched) sides and center: ~~this is where D2 pays again — with a real
per-texture sampler, `REPEAT` handles it in one quad.~~ **the mechanism is a fragment-side `frac`
inside the sprite's sub-rect, and it lands at S5, not S4.** Under an atlas the CPU alternative is
Unity's quad-per-tile explosion and its documented 16 250-quad cap, which is still the reason not to
expand tiles on the CPU.

*(corrected 2026-08-21 at the S4 pre-build audit — see `UI-PLAN-SPRITES.md` **S-D11 (1)**. `REPEAT`
is not available and never was: `crates/boyko_render/src/ui/resources.rs:310` sets
`AddressMode::ClampToEdge` **unconditionally, in both `UiSamplerMode` variants**, and the fragment
shader samples the UI's own sampler rather than the bindless set's. A `Tile` edge written against
`REPEAT` would have rendered a clamped streak. This false lead cost a decision (**S-D7**, since
retired) and a gate (**G4-5**) before anyone read the sampler.*

*The correction pays for itself: `frac` wraps to the **sub-rect**, whereas `REPEAT` wraps to the
whole texture. The sheet hazard S-D7 existed to forbid — a tiled nine-slice whose repeat runs off its
frame and into its neighbours — cannot arise, because the sub-rect **is** the frame. A `debug_assert!`,
a release clamp, a diagnostic counter and a gate all dissolve, and the combination they forbade
becomes the correct picture. That counter could only ever have read zero: the dead-datum class, by
construction.)*

### 4.4 Batching

Unchanged, and that is the point. One pipeline, one `draw(6, N, 0, 0)`, one global z-sort, the clip
in the instance record. The texture identity moves **into the record** as a bindless slot, following
the same doctrine the clip already follows — so z-order costs nothing, because there is only ever one
draw to order within.

**Two consequences accepted explicitly:** there is no opaque/blended split and no depth buffer (a
depth-based opaque pre-pass would be a second pass and a second sort — noted as the one lever left if
fill rate ever dominates), and **overdraw is unmanaged** (every surveyed engine has this too; none
solves it in the batcher).

### 4.5 `ui_setup`'s mandatory font

**D8e.** `ui_setup` requires a `&BakedFont` unconditionally (`gpu_column.rs:269`), so a sprite-only UI
cannot boot. The font atlas binding becomes **default-filled** with a 1×1 transparent texture when no
font is supplied — the same trick the bindless table already uses with its magenta error slot in
every slot. Zero new mechanism.

---

## 5 · Animation

This is where Principle 0 bites hardest, and where the research's own recommendation needed the three
corrections of §2.

### 5.1 The shape: a sink the pack reads, and channel rows that write it

**D9 — `UiVisual` is the sink; presence of a tween row is "running"; there is no EnableTag on tween
columns.**

```rust
/// The composed visual override block — the ONLY thing an animation writes, and the
/// only animation datum the pack reads. Structurally incapable of being a layout
/// input, because it is a different type from `UiLayout` and appears in no term of
/// `ui_layout_discovery`'s `Or<…>` set.
#[repr(C)]
#[component(storage = "dense")]
pub struct UiVisual {
    tint_mul:  u32,       // straight RGBA8, multiplied into color at pack
    opacity:   f32,       // folded into the premultiply — zero GPU bytes
    offset_px: [f32; 2],  // folded into min_px at pack
    scale:     [f32; 2],  // folded into size_px at pack
    uv_shift:  [f32; 2],  // sprite-frame nudge; the flipbook writes `frame`, not this
}                          // 32 B
```

Per-channel tween rows, all `#[component(storage = "dense")]`, one per animatable channel:
`TweenTint`, `TweenOpacity`, `TweenOffset`, `TweenScale`.

```rust
#[repr(C)]
#[component(storage = "dense")]
pub struct TweenTint {
    from: u32, to: u32,           // straight RGBA8
    elapsed: f32,
    inv_duration: f32,            // reciprocal — the tick has no divide
    easing: EasingId,             // #[repr(u8)] dense handle
    flags: u8,                    // loop | ping-pong | reversing
    _pad: [u8; 2],
}                                  // 20 B
```

**Reason, in the order the value lands:**

1. **Presence = running (C2).** Dense insert/remove is pinned as **no archetype migration**
   (`dense_d2_routing.rs`), so the churn argument that motivated the EnableTag does not apply. A
   button hovered 500 times pays 1 000 free-list operations, not 1 000 archetype moves.
2. **Which frees the contiguous path (C1).** Because no EnableTag rides the column,
   `dense_iter_mut`'s const-reject never fires, and the contiguous stride stays *available* — for a
   future measured need, not as a v1 justification. **With one condition attached now, so it is not
   discovered later:** the kernel note this reason leans on carries an open question of its own —
   *"whether the dense fast path also silently skips a `Changed`/`Added` per-row filter is a
   pre-existing, separate question — intentionally NOT widened here"*
   (`query/query.rs:112-114`). D9 rejects Model A partly *because* it destroys per-channel change
   detection, so the two properties must not be combined on a tween column until that question is
   closed: **a `dense_iter` tick may not carry a `Changed`/`Added` filter** — it would be silently
   ignored, and the tick would run over rows it believes it filtered. The contiguous stride is
   available for an *unfiltered* full-column pass and nothing else until the kernel answers.
3. **One row per channel per element is exactly complete, not a compromise.** CSS Transitions'
   uniqueness invariant — *"the element does not have a running transition for the property"*, and
   *"there is never both a running transition and a completed transition for the same property and
   element"* — is precisely the statement that arity-one per channel is the whole model. That
   convergence is the strongest single argument for the per-channel shape, and it dissolves the
   arity question that would otherwise force either a fixed-arity array or an arena.
4. **`UiVisual` as the sink is what makes the tier split *structural*** — see D10.

**What the sink does NOT do, stated because §2/C3 would otherwise read as if it did.** C3 killed the
research's "same-row write, no scatter" claim. Routing the write into `UiVisual` does **not** revive
it. `TweenTint` and `UiVisual` are two *independent* `DenseStore`s — separate free lists, separate
slot orders, membership keyed by `EntityId` through a sparse map (`dense_store.rs:5-8`) — so
`TweenTint` row *i* and `UiVisual` row *j* for one entity are **not co-indexed**. The write-back is
still a per-row lookup; it is merely dense→dense instead of dense→archetype. Reason 2's "the
contiguous stride stays available" is therefore true of the **read** side only. C3's conclusion stands
in full: the sink design buys tier enforcement (D10) and the absence of a side store — it does not buy
contiguity on the write, and §10.5 is the bench that will price what remains.

#### D9b — **one** fused tick system, not four per-channel ones

The four channel columns write **one** component. The scheduler's conflict graph keys on
`ComponentId`, not on field, so four systems taking `&mut UiVisual` would serialise against each other
outright — four sequential passes with the parallelism claim spent on a component none of them
contend for field-wise. Left unstated, that is the kind of thing an implementer settles by accident.

**Decision: one system, one writer of `UiVisual`, one pass:**

```rust
Query<(
    &mut UiVisual,
    AnyOf<(&mut TweenTint, &mut TweenOpacity, &mut TweenOffset, &mut TweenScale)>,
)>
```

`AnyOf` carries a **≥1-member OR predicate that runs per row** (`query/data.rs:136`), so a `UiVisual`
row with no live channel is skipped — which matters because `UiVisual` **persists after a tween
finishes** (its final value *is* the resting appearance; removing the row would snap the element
back), so `UiVisual` rows accumulate to "every element that has ever animated" while live tween rows
do not. And `Option<&Dense>` / `AnyOf` are explicitly non-filtering for the dense **seed**
(`data.rs:227-228`), so the candidate archetypes come from the `&mut UiVisual` include term — the
shape is expressible on this kernel as written.

**This makes `UiVisual` single-writer**, matching the crate's existing discipline for `ComputedRect`
(*"there is a SINGLE `ComputedRect` writer — the layout pass — no write-write race"*,
`widgets.rs:6-9`) rather than inventing a second convention.

**Consequence for D12, stated rather than left to collide:** a fused `AnyOf` pass forfeits the
per-channel contiguous stride that a per-column tick would have had. Per C3 and §9.2 of the research
that contiguity is worth nanoseconds at a UI's N, and the write side never had it anyway (above) — so
the trade is one the design already priced. D12's partition-by-easing consequently applies **within**
each channel's arm of the fused pass, and it is gated on §10.5 rather than required in v1: at tens of
rows the partition's setup exceeds what it saves, and shipping it unmeasured would be exactly the
"arithmetic instead of a measurement" failure D23 refuses elsewhere in this document.

**Rejected — four systems** (serialise on `ComponentId`, and the composition order of four
independent writers into one struct becomes schedule-order-dependent);
**rejected — four systems plus four sink components** (a `UiVisualTint` / `UiVisualOffset` / … split
would restore per-system disjointness, at the price of four random-access probes per node in the
gather instead of one — paying the campaign's one unconditional per-node cost four times over, see
D6a and §10.8, to parallelise a loop measured in nanoseconds).

**Rejected — Model A, the per-element animator object** (`UiAnimator` holding `Vec<Tween>` or
`[Option<Box<dyn Curve>>; N]`). This is what every tutorial and every ported design will suggest, so
it is named rather than omitted: a `Vec` of tracks inside a component is a per-element parallel data
system — the exact shape that produced the O11-SP4 colored-solve race. It also puts `Box`/`dyn` on a
per-frame path, makes the component non-POD (breaking the crate's uniform `#[repr(C)] Copy`
discipline), and destroys per-channel change detection.

**Rejected — Model C, the central `Resource`-owned arena** (`cc::AnimationHost` / LitMotion shape).
Principle 0 permits it explicitly, and it is genuinely better for *unbounded arity per element* and
for group-keyed sequences. It is **held in reserve** and adopted only if a concrete feature demands
arity the channel columns cannot express. Against it today: the write-back is a real scatter
(`get_component_mut` per row), there is no free per-channel change detection, and it re-implements
identity/liveness/iteration inside a resource.

**Rejected — the EnableTag on tween columns** (research §6.2). See C1/C2: it buys nothing that
presence does not already buy on dense storage, and it costs the contiguous path.

### 5.2 D10 — the channel tier table, enforced by the type system

This is the decision that determines whether UI animation costs microseconds or milliseconds. It is
WPF's `AffectsMeasure`/`AffectsArrange`/`AffectsRender` as a compile-time partition instead of runtime
property metadata.

| Tier | Channels | Sink | Dirties |
|---|---|---|---|
| **1 — paint** | tint, opacity, `UiText.color`, sprite frame/UV | `UiVisual`, `UiSpriteCursor` | **repack only** — the sink is a term of `ui_render_discovery`'s `Or<…>` (D6b) |
| **2 — composite** | offset x/y, scale x/y | `UiVisual` | **repack only** — folded at pack (D5) |
| **2 — composite** | **scroll offset x/y** (D19) | `ScrollPosition` | **repack only** — folded during the gather's DFS descent (D19), never at layout time |
| **3 — layout** | width, height, padding, gaps, `UiText.size_px`, `Unit::Pct` | `UiLayout`, `ContentSize` | **relayout** — allowed, documented expensive, steered to FLIP (D11) |

**The table is exhaustive over the channels v1 ships, and the scroll row is why that sentence is
here.** Scroll is the highest-frame-rate interaction a UI has, and the original record placed it on a
layout-time sink without giving it a row — which is precisely how a tier discipline gets breached by a
feature the tier table does not list. Any channel added later without a row in this table is a defect
by construction.

**No tick bumps anything.** A Tier-1/2 channel dirties the render gate by *being written*: the sink
type is a term of `ui_render_discovery`'s change set (D6b), which is one render-side system, so a
`boyko_ui` tick never names a `boyko_render` resource and cannot forget to.

**The enforcement is structural, not documentary.** A Tier-1/Tier-2 tween writes `UiVisual`, and
`UiVisual` appears in **no term** of `ui_layout_discovery`'s ten-way `Or<…>` (`layout.rs:84-134`). It
is therefore *impossible* for a Tier-1/2 animation to trigger a relayout — not "discouraged", not
"by convention", but unrepresentable, because the tick system's signature names a type the layout
query does not.

This is the ECS-native form of Chromium's "only transform and opacity composite" and React Native's
native driver refusing layout properties — three independent systems converged on this partition,
which is the strongest evidence available that it is intrinsic to the problem. Bevy, which lacks it,
has the defect in production: issue #22893, a scrollbar thumb writing its layout fields every frame
and forcing taffy to re-run; the fix was **to stop writing the layout component**.

**Types enforce; conventions erode.** That is the honest justification, and it is a design-discipline
argument, not a benchmark — labelled as such per research §9.2's fair objection.

### 5.3 D11 — layout-affecting animation: allowed, expensive, and steered to FLIP

`ui_layout_discovery` collapses ten `Changed`/`Added` terms into **one bool** and `ui_layout_apply`
then re-lays-out **every cached root** (`layout.rs:126,190-198`). Verified by reading. So one animated
`UiLayout.width` re-solves the entire screen every frame for the animation's duration.

v1 does **not** fix the granularity. Instead:

* Tier-3 animation is **allowed** and documented as expensive.
* The engine ships **FLIP** (First-Last-Invert-Play) as the supported idiom for animating a size or
  position: record `ComputedRect`, apply the final state, let layout run **once**, write a `UiVisual`
  offset/scale that maps the new rect onto the old one, then tween that to identity. Layout runs
  exactly **twice** for an animation of any duration, and every intervening frame is Tier-2.
* `boyko_ui` implements FLIP **better than the web can**, because the two endpoint rects are already
  plain data — "record the first rect" is a component copy, not a forced synchronous reflow.

**FLIP's precondition, stated because one v1 feature does not meet it.** FLIP needs **two fixed
endpoint rects**: a first, a last, and a finite duration to interpolate between them. A **fling has
neither endpoint** — the offset is driven by a decaying velocity toward a bound that the user can
re-throw at any frame — so FLIP is not "expensive here", it is **not expressible** here. That is not a
gap in D11; it is the reason continuous scroll must not be a Tier-3 channel at all, and D19 puts it in
Tier 2 for exactly this reason. Any future channel that is *continuously driven with no fixed
endpoint* inherits the same conclusion: it is Tier-1/2 or it is not shipped.

**Why the granularity fix is not in v1.** Research §5.4 shows it is buildable — stamp a
`UiRootIndex(u8)` at spawn/reparent and `Query<&UiRootIndex, Or<…>>` becomes expressible, reducing
`dirty: bool` to a `dirty_roots: u64` — and `layout.rs:12-31` explains that per-root dirtying was
abandoned only because a change-detecting query cannot yield `Entity` (it *can* yield data). But with
nothing animating, the current design is provably free, and FLIP removes the steady-state case
entirely. **Measure first** (§10.4); build it when the number says so.

### 5.4 D12 — easing by column partition; no `dyn`, no per-row branch

`Box<dyn Fn(f32) -> f32>` is banned and would be an indirect call per row per frame besides.

**Decision:** the built-in family is a `#[repr(u8)] EasingId` over RmlUi's de-facto-standard
10 × 3 set (`back`, `bounce`, `circular`, `cubic`, `elastic`, `exponential`, `linear`, `quadratic`,
`quartic`, `quintic` × `in`/`out`/`in-out`). The tick **partitions the column by easing id** and runs
a monomorphic straight-line polynomial per run — one branch per *run*, not per row. The particles P2
stage already established partition-a-column-by-a-small-class-key in this repository.

Custom/authored curves (including CSS-`linear()`-style baked springs) live in a `Resource`-owned LUT
table indexed by a dense `EasingId` — the `FontId` handle pattern again.

**Rejected:** (a) a `match` on the easing id inside the hot loop (defeats vectorisation); (b) LUT for
*everything* (quantises the curve, visible on a long slow large-amplitude tween, and costs a table
load per row); (c) analytic `cubic-bezier` by Newton-Raphson (4–8 iterations per row — the browsers'
approach, far too expensive for the common path).

### 5.5 D13 — springs are **deferred**, and the reason is a conditional the campaign must honour

A spring's entire distinctive benefit is **velocity-preserving retargeting**: a flick across a button
produces continuous motion rather than a restart. If the campaign does not ship that, a spring is a
worse tween and should be baked to a LUT easing exactly as the web had to.

v1 ships tweens plus **the reversing shortening factor** (D14), which is the tween-domain answer to
the same problem. Springs land as a separate column — never a tagged union with a per-row branch,
since the two have different fields — **if and only if** a measured feature needs velocity
continuity. When they land they use Ryan Juckett's **closed form** (four coefficients computed once
per `(dt, ω, ζ)` preset per frame; unconditionally stable; four multiply-adds per row against
loop-invariant scalars), not semi-implicit Euler.

**A mandatory clause, recorded now because it is the single most likely place this design leaks a
permanent per-frame cost:** a spring approaches its target asymptotically and **never arrives**. The
rest test — `|x − target| < ε && |v| < ε ⇒ remove the row` — is a correctness requirement, not an
optimisation. Without it every spring the UI ever started keeps ticking and keeps writing its sink —
which, under D6b, is exactly what `ui_render_discovery` watches, so the generation advances every
frame forever and the UI never returns to a still frame. Note that the sink write is
*set-if-changed*-shaped only if the tick makes it so: an asymptotic approach writes a **different**
float every frame, so no set-if-changed gate can save a spring from itself. The rest test is the
only thing that can.

### 5.6 D14 — the transition trigger is `Changed<Interaction>`, with the reversing shortening factor

CSS defines a transition as starting when the computed value differs between the before-change and
after-change style. Every surveyed implementation pays real machinery for that diff: Chromium keeps
two style objects, RmlUi narrows the trigger to class/pseudo-class changes to make it cheap, WPF
hooks the property system, Flutter makes the user call `forward()` by hand.

**This engine gets the *diff* for free.** `Interaction` is already a tick-bearing column written
*set-if-changed* — its module doc says so: *"Written set-if-changed, so a still frame bumps no tick"*
(`interaction/components.rs:17`). `Changed<Interaction>` **is** the before/after diff, computed by
the kernel, at zero marginal cost, with per-row granularity. No system surveyed has this for free, and
that much is a strict improvement rather than a compromise.

**It does not get the *start* for free, and the original record's query could not start a
transition.** The shape first written here was

```rust
Query<(&Interaction, &UiStateTint, &mut TweenTint), Changed<Interaction>>   // WRONG
```

and it retargets a transition that is already running — it cannot begin one. `&mut TweenTint` matches
only entities that **already carry the column**, and D9 fixes that presence of the row *is* "running"
and removal *is* "stopped" (that is what buys "500 hovers = 1 000 free-list operations, not 1 000
archetype moves"). A button **at rest** — the state every transition starts from — carries no row and
is therefore excluded from the query meant to start its transition. Starting a tween is a
**structural insert**, not a mutable borrow, and this design does not get to have it both ways.

**The correct shape is the crate's existing answer to "a structural/cross-entity write that still
needs a `Changed` window": the discovery/apply split.** `ui_bar_discovery`/`ui_bar_apply`
(`widgets.rs:12-19,110-166`) and `ui_layout_discovery`/`ui_layout_apply` both solve exactly this, for
exactly the reason recorded at `layout.rs:20-21` — *"an exclusive body has no tick window for
`Changed`"*:

* **`ui_transition_discovery`** — a normal scheduled system, so SystemParams supply the tick window.
  `Query<(), Changed<Interaction>>`; `iter().next().is_some()` sets `dirty` in the transition scratch,
  alongside `last_run`. A still frame leaves it false and apply early-returns: the 0%-gate, unchanged.
* **`ui_transition_apply`** — **exclusive**. It walks the cached `UiStateTint` candidate list
  (`query_entities_buf` into retained scratch, refreshed on a set change — the `ui_bar_apply` pattern
  verbatim), gates each entity with
  `get_component_changed_tick(e, Interaction::component_id())` + `is_newer_than(last_run, this_run)`
  — the *same* per-row change signal, re-checked by hand, which is precisely what `ui_bar_apply`
  already does — reads the running `TweenTint` **if present**, and inserts or overwrites the row
  **immediately**.

**Why the exclusive form is required and not merely tidy.** The alternative —
`Option<&mut TweenTint>` plus a `Commands` insert — makes every hover-enter a **deferred** structural
op that lands at the next apply window. Two consequences, both v1:

1. the **first frame of every transition is lost**, so a 120 ms hover is 120 ms of tween preceded by
   one frame of nothing — the exact class of defect D14's closing paragraph says nobody can name and
   everybody feels;
2. the **reversing shortening factor has nothing to read**. It is specified below as scaling the new
   duration by the fraction the old row had traversed, from that row's `elapsed`/`inv_duration`. A
   reversal arriving while the start is still queued finds no row and silently takes the full
   duration — a correctness hole that appears only under fast cursor flicks, which is the one input
   pattern the factor exists to handle.

The exclusive apply has neither, because the insert is visible to the next entity in the same loop.
Its cost is a dirty-gated exclusive pass over a cached candidate list — the shape this crate already
runs twice per frame.

The state→value declaration is an array indexed by the enum, never a map (the repo's own stated rule):

```rust
#[repr(C)]
pub struct UiStateTint {
    by_state: [u32; 3],   // indexed by `Interaction as usize` — None | Hovered | Pressed
    duration_ms: u16,
    easing: EasingId,
    flags: u8,
}
```

**The reversing shortening factor is in the runtime, not left to callers.** When a
`Changed<Interaction>` arrives while a `TweenTint` is already running, the new tween's duration is
scaled by the fraction the old one had traversed — a half-completed hover-in that reverses takes half
the time back, not the full time. The row already holds `elapsed` and `inv_duration`, so it is three
lines. It is specified here because it is the kind of thing that is never added later: nobody can name
what feels wrong, they only know the button feels bad when the cursor flicks across it.

**`Disabled` stays out of the state array.** Per the standing capability rule it is the
`Focusable`/`Interaction` EnableTag bit, not a fourth `Interaction` variant, so the table stays at 3
and the disabled *appearance* is a separate component.

### 5.7 D15 — the UI clock is `Time`'s **real** delta by default, virtual opt-in per row

`boyko_ui` has never named a clock, so this is a new cross-crate coupling that must be justified once
and reused everywhere. `Time` (`time/time.rs:38`) carries both a virtual delta (hitch-clamped,
`relative_speed`-scaled, **zero while paused**) and a real delta (unclamped, unscaled, pause-blind).

**Default real**, because a pause menu that fades in on a paused virtual clock never fades — the
canonical UI animation is the one that must run *while the game is paused*. A `flags` bit per tween
row selects the virtual clock for the diegetic cases (a world-space health bar that should slow with
bullet-time).

**Rejected:** virtual-by-default (breaks pause menus, the single most common UI animation);
a UI-local clock (a second time source that disagrees with the engine the first time anyone pauses).

### 5.8 Timelines — clips are assets, players are cursors

Recorded so the shape is fixed even though it is deferred (§9): a multi-channel, multi-keyframe
timeline must **not** be copied per element. The clip is a `Resource`-owned immutable contiguous
keyframe table addressed by a dense `ClipId`, stored **channel-major** so a channel's evaluation is a
contiguous scan; the player is a per-element POD `{ clip: ClipId, time: f32, speed: f32, flags: u8 }`,
~16 B — the entire per-element cost of playing an arbitrarily complex timeline. Sequencing is a
**group key over contiguous rows** (`cc`'s group ids), never a nested object graph: LeanTween's
49 963 ms sequence start is an O(n²) defect in exactly that structure, and it is the cautionary number
in the corpus.

---

## 6 · Interactivity

Built **on** the existing `Interaction`/`Focusable`/pointer machinery, not beside it. The existing
`ui_focus_system` six-step spine (collect → blur → resolve → write → click → focus) is the right
skeleton and is kept.

### 6.1 D16 — pointer **capture** is the spine change, and it is one field

Every reference system has it under a different name (browser `setPointerCapture`, ImGui `ActiveId`,
Godot `gui.mouse_focus`, Unity `pointerPress`/`pointerDrag`, Bevy's drag state). It is **the only
genuine O(1) path in the entire survey**: while a pointer is captured, *no hit test runs at all*.

Dragging a slider, scrolling with a held thumb, and selecting text are precisely the high-frame-rate
interactions, and they are exactly the ones where the target is already known.

`PointerSlot` (`focus.rs:41`) already stores `pending_click: Option<(Entity, u16)>` — a press-origin
stamp. Capture is that field promoted, plus the press **position** the crate does not currently record
at all:

```rust
pub struct PointerSlot {
    owner: Option<Entity>,          // captured target; None ⇒ resolve normally
    owned_channels: u8,             // bitmask: Move | Wheel | Keys (the ImGui SetKeyOwner idea)
    press_pos: [f32; 2],            // NEW — stamped at press; no drag delta is computable without it
    pending_click: Option<(Entity, u16)>,
    click_fired: Option<(Entity, u16)>,
}
```

**Generation safety is already solved** the way `resolve_pointer` solves it: `Entity` equality
includes the generation, so a recycled slot cannot masquerade as the owner (`focus.rs:487-490`
documents exactly this argument).

**Named risk, with its defence:** capture is the classic source of stuck-input bugs. The engine's
existing unconditional blur reset on `!cursor_inside || !window_focused` (`focus.rs:161`) **must
extend to capture**, and a captured entity that despawns must release — or the UI wedges permanently.

### 6.2 D17 — routing by capability on a bounded ancestor snapshot; bubbling is an **opt-in escape hatch**

The four idioms that motivate full propagation are, on inspection, three *routing* problems and one
genuine phase problem:

* *click on a Button's Label* — already solved: a Label with no `Interaction` column is structurally
  absent from `candidates` (`focus.rs:224`), so the Button **is** the hit. Capability by presence,
  working today.
* *wheel over a ScrollView's content* — routing: resolve the wheel channel against the nearest
  ancestor carrying `Overflow` with a non-clamped axis.
* *keys inside a focused panel* — focus, not pointer propagation.
* *modal overlay swallowing everything* — a full-screen `FocusPolicy::Block` node already handles the
  pointer half.

The DFS in `collect_candidates` already knows every node's parent chain; recording a **bounded
ancestor snapshot** (`[Entity; 16]`, a fixed array in the retained scratch — never a `Vec` per node)
makes routing a ≤16-step array walk with no tree traversal and no event dispatch.

**Bubbling is available where an author asks for it**, on the kernel's *existing* observer machinery:
`Trigger` with `PROPAGATION = Up` and `Traversal = ChildOfTraversal`, `propagate(false)` to stop,
fn-pointer runners (no `Box<dyn Fn>`), and a **sticky `ArchetypeFlags::HAS_ENTITY_OBSERVER` gate** so
subtrees with no observer pay nothing (`observers/entity_store.rs:5-11`). Zero cost when unused.

**Reason for the default:** making callbacks the idiomatic path would undo the deliberate
reflection-free `OnClick(u16)` design, which is what makes handlers survive a round trip through a
`.ui` file — a shipped feature this campaign must not break. But refusing bubbling *entirely* would
push authors to fake it with their own side tables of closures, which is the worst outcome.

**The strongest argument against D17, recorded because it may be right:** every retained UI in the
survey — DOM, UI Toolkit, Godot, Unity uGUI, and Bevy in its most recent redesign — independently
chose full propagation. Five convergences is weak evidence about performance and **strong** evidence
about expressiveness. "Handle it in the parent unless the child consumed it" is not expressible in the
default path, so the escape hatch may become the common path, and then we will have paid for two
mechanisms and standardised on the one we called exceptional. And the failure is **not local**: under
D17 the resolve pass's output is *one target plus an ancestor snapshot*; under propagation it is *a
path*, and every state machine keyed on "the target" (drag, scroll, tooltip, gesture) would have to be
re-keyed on "the path". **Discovering this late means rewriting the interaction spine, not extending
it.** This is **R3**.

**Naming hazard to fix in the same commit:** `FocusPolicy::{Block, Pass}` reads exactly like Godot's
`MOUSE_FILTER_{STOP, PASS}` but means something different — occlusion during *resolution*, not
propagation *after* it. Once any propagation exists the two axes must be separately named, or every
user coming from Godot writes the wrong thing.

### 6.3 Drag

**D18.** The payload **is** the dragged entity. Every reference stores it in a subsystem-owned box —
a Godot `Variant`, an HTML5 `DataTransfer`, ImGui's untyped context buffer — and a `Box<dyn Any>` or
a `HashMap<DragId, Payload>` side store is the Principle-0 violation.

* `Draggable { threshold_px: f32, channels: u8 }` — presence = capability. Threshold default 6.0 px
  (ImGui's `MouseDragThreshold`, a real named constant rather than an ad-hoc epsilon).
* `DragActive { origin: Entity, grab_offset: [f32;2], started_at: [f32;2] }` — **added** when the
  threshold is exceeded, **removed** on drop. Presence *is* "am I dragging"; there is no
  `is_dragging` bool.
* `DropTarget { accepts_mask: u32 }` — acceptance is a `ComponentId`/EnableTag mask checked against
  the dragged entity's archetype in O(1). No MIME strings, no serialisation, no type erasure.
* The drag **preview** is an ordinary UI entity spawned with `StackIndex = u32::MAX` and **without**
  `Interaction` — structurally absent from the candidate set. This is the ECS-native way to say "the
  preview is not pickable"; Godot needs an explicit ancestor test for the same thing
  (`viewport.cpp:1891`).
* Capture (D16) owns the pointer for the drag's whole life, so the drag itself costs **zero**
  hit-tests per frame.
* `Interaction` gains **no** `Dragged` variant — dragging is `DragActive`'s presence, and adding a
  fourth variant would widen the `UiStateTint` array and the state table for a state that is
  structurally available.

### 6.4 Scroll

**D19 — `ComputedClip` becomes computed, by the layout pass, from an `Overflow` policy.**

This is the substrate scroll has been missing: today no system computes `ComputedClip`, every write
site is authoring, and the layout pass never reads or derives it (`components.rs:188`, verified).

* `Overflow { x: Visible|Clip|Scroll, y: … }` — read by `ui_layout_apply`, which writes
  `ComputedClip` **set-if-changed** for the subtree, intersecting down. Layout remains the single
  writer of computed geometry. `Overflow` **is** a layout input and joins `ui_layout_discovery`'s
  `Or<…>` set — correctly, because changing an overflow policy genuinely changes the solve.
* `ScrollExtent { max: [f32;2] }` — written by layout beside `ComputedClip`: content extent minus
  viewport extent, the clamp bound. It is a layout **output**, so it introduces no cycle, and it means
  the scroll clamp never needs to ask layout anything at run time.
* `ScrollPosition { offset: [f32;2] }` on the container. **It is a Tier-2 composite channel, folded
  during the gather's DFS descent — it is never a layout input, and `ui_layout_discovery` does not
  name it.** See D19a below; this is the decision the original record got wrong.
* `ScrollMomentum { vel: [f32;2] }` — **added** on fling release, **removed** when it decays below
  epsilon. The integrate system's query is *empty* when nothing is coasting; a still UI runs it over
  zero rows. That is capability-by-presence doing real work.

Momentum uses UIKit's frame-rate-independent formulation — `decelerationRate` is defined as the
velocity change after one millisecond, `normal = 0.998` — so `v *= rate.powf(dt_secs * 1000.0)`.

#### D19a — the scroll offset is folded at **traversal**, never applied at layout time

**The defect this replaces, recorded rather than quietly fixed.** The original record put
`ScrollPosition` "on the container, offsetting children **at layout time**". For that to have any
visible effect, a `ScrollPosition` write must reach `ui_layout_discovery` — and that system collapses
ten `Changed`/`Added` terms into **one bool** (`layout.rs:118-127`), after which `ui_layout_apply`
re-lays-out **every cached root** (`layout.rs:189-197`: `for i in 0..scratch.roots.len() { …
layout_root(…) }`). `ScrollMomentum` writes the offset **every frame while coasting**. So as
specified, every frame of every fling re-solved every root on the screen — which is, verbatim, the
Bevy issue #22893 defect §5.2 cites as this document's own cautionary tale: *a scrollbar thumb writing
its layout fields every frame and forcing taffy to re-run; the fix was to stop writing the layout
component.* And the mitigation §5.3 offers could not have applied: **FLIP needs two fixed endpoints
and a fling has none** (see D11's precondition note).

**The invariant that fixes it:** `ComputedRect` and `ComputedClip` are stored in **unscrolled layout
space**. The ancestor scroll accumulation is folded by each consumer that maps to screen space, during
the descent it already performs. There are exactly **two** such consumers and **both already DFS with
an inherited value on the stack**:

| Consumer | Descent it already has | Change |
|---|---|---|
| The canonical gather (D31) | DFS over `UiRoot`/`Children`, carrying the inherited clip | the stack tuple carries `(clip, scroll_accum)`; a node's packed `min_px` gets `+ scroll_accum`, and its clip likewise |
| `collect_candidates` (`focus.rs:204-257`) | DFS over `Children`, `stack.push((child, effective_clip))` | the same one-field widening; the candidate's `rect` and `clip` are folded at push |

**Reason this is the right shape and not merely the cheap one.** The scroll offset is a *uniform
translation of a subtree*, which is the definition of a composite-tier channel — it is what Chromium
means by scrolling on the compositor and what §5.2 already cites as "only transform and opacity
composite". Folding it at traversal costs **zero** per-node storage and **zero** relayouts per fling
frame.

**And zero extra probes in the common case, by one gating rule that must be stated or it will not
happen.** Naively, folding needs a `get_component::<ScrollPosition>` probe on every node — which
would hand back part of what D6a and §10.8 are fighting for. It does not, because a scrolling node
**always** carries a `ComputedClip` (an axis set to `Scroll` implies `Clip`, and D19 makes layout the
writer of both), and both DFS descents **already read `ComputedClip` on every node**
(`focus.rs:221`). So the rule is: **probe `ScrollPosition` only where the `ComputedClip` read
returned `Some`.** A UI with no scroll container has no computed clips, so it pays nothing at all;
a UI with one pays one extra probe on the handful of clipped nodes. Layout's single-writer
invariant over computed geometry is untouched — nothing writes `ComputedRect` but the layout pass —
which is the same property D5 protects for the Tier-2 visual transform, obtained the same way.

**`ScrollPosition` joins exactly two change sets, and neither is layout's.** It is a term of
**`ui_render_discovery`**'s `Or<…>` (D6b) — without which a coasting fling would repack nothing and
the screen would not move — and a term of **D23's** candidate-rebuild dirty set — without which the
hit-test would keep testing against pre-scroll rects. It is **not** a term of
`ui_layout_discovery`'s `Or<…>`, and that omission is the decision, not an oversight; a future editor
adding it there re-introduces exactly this defect.

**The honest cost, which is not zero.** A coasting frame *does* dirty the hit-test, so the candidate
array is rebuilt — one DFS with its probes over the interactive nodes, per fling frame. That is O(N)
probes where the original specification was O(N) *layout solves*, and it is the same cost the
hit-test already pays on any frame anything moves. It is priced in §10.4.

**Rejected — per-root layout dirty bits as the answer** (D11's deferred item). They would reduce a
fling from "every root" to "one root" — still a full root re-solve on every frame of every fling, for
a translation that changes no size. Wrong tier, not merely a slower one.

**Rejected — a `ComputedScrolledRect` mirror column** written per frame by a propagation system: it is
a second computed-geometry column with a second writer, O(subtree) writes per coasting frame, and it
would put the campaign back in the business of maintaining two rects that must agree.

**Clipping stays free.** Because the clip travels in the instance record rather than as scissor state,
a scroll container costs **zero** extra draw calls and zero state changes; nested clips are an AABB
intersection at pack time. Every reference breaks batches at a scissor boundary. This property is why
D19 is cheap here and expensive everywhere else.

**Wheel routing:** `PhysicalInput.wheel` is already aggregated (`raw/queue.rs:186,289-297`) and
`ui_focus_system` already reads `PhysicalInput` (`focus.rs:26`) — it simply never touches `.wheel`.
Routing is D17's ancestor-snapshot walk to the nearest `Overflow` with a non-clamped axis. Godot
needed a special case (`force_pass_scroll_events`) for exactly this, which is evidence that "the wheel
routes differently from the click" is a recurring requirement, not a nicety.

### 6.5 Text input

**D20 — the character stream is a separate UI-facing ring; `PhysicalInput` is not widened.**

This is the deepest change in the campaign and it starts one crate down. Verified state: `WM_CHAR` is
**not handled** by the Win32 translator; `RawInputEvent::Text(_) => {}` discards on ingest
(`raw/queue.rs:302`); and a test at `queue.rs:578-580` **asserts that it stays discarded**, with the
rationale *"Text is for text fields only — never gameplay; the physical snapshot must not carry it."*

That rationale is **correct**, and this design honours it rather than re-blessing the test:

* `boyko_input`'s Win32 translator gains `WM_CHAR` → `RawInputEvent::Text(char)` (it is currently
  produced by nothing, so the variant is unreachable).
* Ingest routes `Text` into a **new bounded per-frame character ring** — *not* into `PhysicalInput`.
  `PhysicalInput` is a LEVEL+EDGE snapshot and a character **stream** is not; making it a field would
  force ordering and overflow decisions onto a type that has neither.
* **The existing test keeps passing unchanged**, because text still never reaches the physical
  snapshot. The I1-ABI test on the raw types (`raw/keycode.rs:425`) is likewise untouched — no raw
  type changes shape.

Storage, ECS-native:

* `TextInput { cap: u16, flags: u16 }` — presence = "this is an editable field".
* The buffer is a **fixed inline POD**, following the shipped `UiName` precedent (`[u8; 60]` + `len`,
  one cache line, `Copy`, "no interner and no global string table", `components.rs:264-274`) at a
  larger cap. Never a `String` in a side map.
* `TextCursor { anchor: u32, head: u32, affinity: u8 }` — selection is anchor/head (empty = caret),
  which makes shift-click and drag-select the same code path. `affinity` exists because the same byte
  index is both end-of-line-N and start-of-line-N+1 at a soft break.
* Caret placement from a click is a binary search over cluster advances in the existing shaped run
  (`text/shape.rs`, `measure.rs` already hold the advances). **Never split a grapheme cluster** — a
  caret between a base and its combining mark is a bug, not an edge case.

**IME is deferred, and the shape is recorded so the deferral is not a hole.** `TextPreedit { bytes,
len, cursor }` **added** while composing and **removed** on commit — structural presence, so there is
no `is_composing` flag to go stale. When it lands the route is **IMM32**
(`WM_IME_STARTCOMPOSITION`/`COMPOSITION`/`ENDCOMPOSITION`, `ImmGetCompositionStringW`,
`ImmSetCompositionWindow` to track the caret), not TSF — simpler, adequate, and what Microsoft's own
game guidance recommends. The winit `Ime` vocabulary (`Enabled`/`Preedit`/`Commit`/`Disabled`) is the
right *shape* to copy even though we do not use winit, because it makes the Linux path later a
translation rather than a redesign. IME is enabled **iff** a focused entity carries `TextInput`.

### 6.6 Keyboard navigation

**D21.** `update_focus` reads exactly two keys today (`focus.rs:543,556`), verified. v1 adds:

* **Shift+Tab** — reverse. The modifier is simply not read today.
* **Arrow keys** — directional navigation by Unity's scoring function, which is small and works:
  maximise `dot(dir, v) / |v|²` from the point on the current rect's **edge** in that direction, not
  its centre. It runs over the same rect arrays the hit-test already builds — one pass, no tree walk,
  no extra storage.
* **Escape** — blur / cancel (and, once capture exists, release).
* **Space** — activates the focused node (the `OnSubmit` path Enter already uses).
* `Focusable { tab_index: i32 }` where **negative = directly-focusable-only, not sequentially**
  (Bevy's `bevy_input_focus` model), plus an optional `FocusGroup { order: u32 }` on ancestors.
* `FocusNeighbors { up, down, left, right: Entity }` — the opt-in override. Godot's lesson is that
  automatic directional navigation **always** needs a manual escape hatch.

**The focus ring emits an extra `UiInstance`** — an outset rect with a border — rather than a shader
flag. Zero shader work, zero `.spv` re-bless, and it composes with the existing z-sort (D4 puts it
last in the node's emission order). `:focus-visible` (ring shown for keyboard focus, hidden for mouse
focus) is one bool on `UiInputFocus`, not a component.

### 6.7 Hover feedback, dwell and tooltips

**D22.** `write_interactions` produces `UiHovered`/`UiPressed`/`UiFocused` and *nothing consumes them
for styling* — a `ButtonBundle` is visually inert on hover today (verified: no system anywhere maps
`Interaction` onto `UiBackground`, `UiText.color`, `UiLayout` or `UiImage.tint`).

D14 closes this: `UiStateTint` + `TweenTint` + `UiVisual` is the hover/press response, and it is an
*animation*, not a snap. Bevy's canonical button example snaps between three constants and Bevy has no
transition primitive at all — this engine gets the smooth version from machinery it already has.

Tooltips need a **dwell timer plus a stationarity test** — the part naive implementations miss, and
why a pointer sweeping a toolbar must not fire six tooltips. ImGui's constants are the starting
values: `HoverStationaryDelay = 0.15`, `HoverDelayNormal = 0.40`. `HoverDwell { ms: u16 }` is **added
on hover-enter and removed on hover-exit**, so the accumulate system iterates typically one row. The
tooltip entity spawns with `StackIndex = u32::MAX` and no `Interaction` — the same structural
unpickability trick as the drag preview.

### 6.8 D23 — hit-test optimisation is **gated behind a measurement**, not shipped on arithmetic

The interaction research proposes an SoA candidate layout (16 B/node instead of 64 B/node, AVX2-shaped)
and argues from arithmetic: a 1 000-node scan drops from ~64 KB to ~16 KB of working set.

**That is a hypothesis, and this project has a documented history of gates that could not fail and of
numbers that were not measurable.** The research's own second counter-argument is correct: at a HUD's
realistic N the 64 B AoS scan is already microseconds, and the *actual* per-frame cost is far more
likely the **~6 random-access `get_component` probes per node per frame** in `collect_candidates`
(`focus.rs:221-234`), which SIMD does not touch.

So the order is fixed by expected effect, not by elegance:

1. **Dirty-gated candidate rebuild** (do this): `Changed<ComputedRect>` + `Changed<ComputedClip>` +
   `Added`/`Removed<Interaction>` + hierarchy edits + `UiViewport.generation` is a complete dirty
   term. A still frame then costs one scan of a cached array and **zero** component probes. This is
   the browser lesson — hit-test on event, not on frame — in ECS form.
2. **Early-out at the first blocking hit** (do this): build candidates in *descending* total order so
   the first `FocusPolicy::Block` hit terminates the scan. **Caveat that is a correctness invariant,
   not an optimisation target:** `write_interactions` performs a deliberate *unconditional* pass so a
   node occluded this frame is still reset to `None` (`interaction/components.rs:46-49`). Early-out
   applies to **resolution only**, never to the reset.
3. **SoA + indexed clip table** (measure first, §10.6). It is also what D21's directional navigation
   wants, so it may earn its place there rather than here.

### 6.9 Multi-pointer

**D24 — `MAX_POINTERS` stays 1 in v1, but the two hardcoded `slots[0]` reads are removed now.**
`resolve_pointer` (`focus.rs:471`) and `ui_dispatch_system` (`dispatch.rs:44,53`) both hardcode slot
0; both become loops. This is cheap now and invasive later, and the array shape is already correct.

When touch lands, the browser rule is adopted verbatim: **touch gets implicit capture on down**,
released on up/cancel. Without it a finger that drags off a button re-targets mid-gesture and every
touch UI feels broken — this is not a nicety.

**Gestures: a fixed recognizer ladder, not an arena.** With one pointer the whole conflict set is
{click, drag, long-press, double-click} and it resolves with two thresholds and two timers, encoded as
a `PointerGesture` state enum **on the pointer slot**, not per element. Flutter's `GestureArena` is the
more elegant answer and is the right thing **when touch lands**, because that is when the conflict set
becomes open-ended (pinch vs pan vs two-finger scroll) and the ladder stops scaling — and the arena is
also the natural place to express "the ScrollView wins the vertical drag, the Slider wins the
horizontal one", which the ladder cannot express at all.

---

## 7 · The Aether `ui` construct

**The hard constraint:** `aether_lang` depends on **no engine crate**. It emits token paths only
(`quote!(::boyko_ecs::…)`) and it **cannot resolve names** — a proc macro sees a `TokenStream` with
spans, not types, not `use` statements, and procedural macros are additionally unhygienic, so every
emitted path must be absolute.

### 7.1 D25 — `ui` is the tenth construct, and it follows `scene` exactly

`Construct` gains a tenth variant; `parse.rs`'s dispatch gains a tenth arm; `keyword()` gains a tenth
row. The lowering follows `scene_fn` (`expand.rs:1383`) verbatim in shape:

* one **demand-driven** spawn fn — a `ui` block with no bindings compresses to `(commands)` alone,
  exactly as a scene with no mesh lets does;
* `__aether_`-prefixed params, because a user binding named `commands` shadowed the param in `scene`
  and produced an E0599 on a user-token-free span — measured, and the fix was to make the collision
  unrepresentable rather than diagnosable;
* nesting via `children: [ … ]` lowering to `add_child`, mirroring `emit_node`'s recursion —
  hierarchy is driven by `ChildOf` insertion; user code never writes `Children`, and neither does
  Aether;
* bare component expressions as the `extras` fallback (`SceneNode.extras` is already documented as
  "the `ui!` fallback"), each becoming one `.insert(EXPR)`.

**Two requirements a UI tree has that a scene does not**, and how each is met:

1. **Every node MUST carry a `UiLayout`** (the `ui!` macro is a compile error without one) and roots
   need `UiRoot` plus a host-supplied `UiViewport`. The construct enforces this at expansion with a
   spanned diagnostic on the node head, which is strictly better than `ui!`'s error because the span
   points at the offending node rather than the macro.
2. **`#name` is load-bearing twice** — it declares a `let` handle *and* it is forward-referenced by
   `BindText.source`.

### 7.2 D26 — `#name` resolution reuses `AetherCtx`; forward references lower to a deferred insert tail

Name resolution already exists in **three incompatible forms**: `ui!`'s compile-time dup table,
`.ui`'s two-pass runtime fixup list, and Aether's `AetherCtx` symbol table. **Picking one rather than
inventing a fourth is the decision.** `AetherCtx` is chosen — it is the construct's native mechanism,
it already does cross-construct resolution with did-you-mean at edit distance ≤ 2, and it produces
spanned diagnostics.

Emission keeps `ui!`'s node order (spawn+insert per node, pre-order) and handles forward references
with a **deferred-insert tail**: any component expression naming a `#name` not yet bound is emitted
after the whole tree. This is semantically what `.ui`'s two-pass fixup already does, performed at
compile time instead of at load time.

**Rejected:** (a) *spawn-all-first, then insert, then link* — makes forward references trivially work,
but diverges from `ui!`'s emission shape for every node rather than only the forward-referencing ones,
and the equivalence gate pins the resulting world, not the token shape, so the divergence would be
unpinned; (b) a fourth resolution mechanism.

### 7.3 D27 — handlers lower to `OnClick(u16)` via **token re-spelling**, and the action enum is named by the construct

This is the crux the campaign turns on, and the answer is that **no new handler mechanism is needed**.

`OnClick`/`OnHover`/`OnSubmit` are `#[repr(transparent)] u16` carrying a dense `Actionlike::index()`,
resolved at authoring time — *"an integer is the reflection-free common denominator"*
(`interaction/action.rs:1-9`). This is Iced's Elm-message model in its strongest possible form, and it
is why handlers survive a round trip through a `.ui` file.

The construct names its action enum, and the handler re-spells tokens for **rustc** to resolve (M1 —
the same trick `bsn!` uses and the same trick `ui!` already uses):

```
ui hud(actions = GameAction) {
    #start_btn {
        UiLayout { .. },
        UiBackground { .. },
        on click:  Confirm,
        on hover:  FocusStart,
        on submit: Confirm,
        children: [ ... ]
    }
}
```

lowers to

```rust
.insert(::boyko_ui::interaction::OnClick(
    <GameAction as ::boyko_input::Actionlike>::index(GameAction::Confirm) as u16
))
```

**The path matters and the obvious spelling is wrong.** `::boyko_ui::OnClick` does **not** resolve:
`boyko_ui`'s `lib.rs` re-exports at the root only `ui`, `Bindable`, `UiPlugin` and the `text` entry
points (`lib.rs:50-68`); `OnClick`/`OnHover`/`OnSubmit` are re-exported by `pub mod interaction`
(`interaction/mod.rs:18`) and again inside `pub mod prelude` (`lib.rs:89`) — and a prelude is useless
to a proc macro, which must emit an absolute path because procedural macros are unhygienic and see no
`use` statements. `::boyko_ui::interaction::OnClick` is the canonical absolute path (the fully
explicit `::boyko_ui::interaction::action::OnClick` also resolves). The other half of the line is
already correct: `Actionlike` **is** a root re-export (`boyko_input/src/lib.rs:45`) and
`fn index(self) -> usize` (`actionlike.rs:50`) takes the variant by value exactly as written.

**And the gate that proves it must exist, or D27 is an assertion.** `aether-tests` is where the R4
anti-drift discipline lives — its `Cargo.toml` states the rule for `material` verbatim: *"the day
`Material::new` gains a parameter, the A5 tests stop compiling here, in-repo, instead of in a user's
game"*. That crate currently has **no** `boyko-ui` dependency (its dev-deps are
aether / ecs / macros / render / app / scene / math), so a `ui` construct would emit `boyko_ui` paths
that **nothing in the repository compiles** — the exact drift R4 exists to prevent, and the reason the
wrong path above survived into this document at all. `boyko-ui = { path = "../boyko_ui" }` joins
`aether-tests`'s dev-dependencies in the same rung that adds the construct, with its blast radius
recorded beside the existing `boyko-render` / `boyko-app` notes. Under **D31** `boyko-render` already
pulls `boyko_ui` transitively, so the edge adds compile time only for the *names*, not for the code.

**Why this is the right answer:**

* **`aether_lang` resolves nothing.** It re-spells `Confirm` into `GameAction::Confirm` and emits an
  absolute trait path. The constraint is honoured exactly.
* **A misspelled action is a real compile error** with a real span (variant not found on the enum),
  which is what the interaction research demanded: *a typo must never silently become `NO_ACTION`.*
  The `NO_ACTION` sentinel exists for the `.ui` hot-reload path where a compile-time error is
  impossible, and blurring the two would turn a typo into a dead button.
* **Zero runtime cost and zero new mechanism** — the same 2-byte POD component the other two surfaces
  produce, so the equivalence gate extends to a third leg rather than a second gate being invented.

**Rejected — inline closure handlers** (`on click: |…| { … }`). SwiftUI, Compose, Dioxus, Leptos, QML
and now `bsn!` all put the handler at the call site, and that near-unanimity is a real ergonomic
argument: the action model costs a variant, a registration, a system, and a name — so a button's
behaviour lives three files from the button. But a closure is a per-element `Box<dyn Fn>` on durable
data (Principle 0 *and* 1) and, decisively, **it is not serialisable** — a `.ui` file cannot contain a
closure, and hot reload is a shipped feature this campaign must not break. The tax is deliberate, and
Iced makes the same trade on purpose.

**Rejected — the codegen compromise, deferred rather than refused.** An Aether `ui` block that accepts
an inline handler *body* and generates the `Actionlike` variant + the system + the registration +
`OnClick(<index>)` would give call-site locality with the *same* POD runtime — and `machine` already
proves the pattern (it generates a flat enum plus a drain-and-act fn plus its registration from a
declaration). It is the right long-term answer and it is **deferred**, because it must synthesise an
enum that coexists with the user's own action enum, and that interaction is a language design question
that should not be settled in the same rung that introduces the construct.

### 7.4 D28 — Aether emits data, never behaviour; styling resolves at author time

Every capability prop emits a **component, or nothing**. `draggable`, `scroll y`, `focusable(3)`,
`tooltip: "…"` emit `Draggable`, `Overflow`, `Focusable { tab_index: 3 }`, `Tooltip`. **A prop that is
absent emits nothing**, so the archetype genuinely lacks the column — this is the difference between
capability-by-presence and a struct of flags, and the DSL is where it is easiest to get wrong.

Animation follows **QML's `Behavior on <property>`** shape — the single best declarative-animation
precedent in the survey — attaching the animation to the property **as data**:

```
states { hovered { tint: 0xFF3355FF, in: 120ms ease_out } }
```

lowers to `UiStateTint` + `TweenTint` **inserts**, with **no new runtime**. This mirrors the existing
lowering discipline exactly: `material` lowers to a builder fn, `scene` to a spawn fn, `component` to
a type — a style block lowering to a component set is the same move.

Styling is `bsn!`'s **patch** model (resolved to concrete component bytes before the entity is live)
plus Flecs's **`with` scope** (pure author-time factoring, zero runtime representation). **Refused:** a
USS-style runtime selector cascade (a parallel matcher over the tree — Principle 0) and Flecs's runtime
`IsA` value inheritance (a pointer chase per read, against dense columns built precisely to avoid one).

**Note for the implementer:** `Construct::Material` is `Box`ed because `MaterialDef` holds seven
`syn::Expr` slots and `syn::Expr` is a ~200-byte enum — inline, that one variant would have made every
`Construct` 952 bytes. `UiDef` will carry a node **tree** of expressions and is strictly larger, so it
is `Box`ed from the start for the same reason. `SceneNode` already `Box`es its `AtPose::Verbatim` on
this exact rationale.

### 7.5 D29 — dynamic lists are `UiRepeat`, not a macro loop and not a virtual tree

Recorded because it is the obvious wrong move and it is **deferred** (§9). A macro cannot know a
runtime length, so a `for` loop inside the macro produces a fixed node count at compile time — the
wrong answer to the question. And a diffed view tree is a third representation whose only purpose is
to be diffed, when `reconcile_ui` already diffs a parsed tree against the live world and
`BindText`/`BindValue` already do fine-grained property updates with no diff at all.

The ECS-native shape: a container carries `UiRepeat { source, template, key_field }` where the
template is a **disabled prefab subtree that is itself an entity**, never a `Vec<Node>`. One system
compares the source column's row count against `Children` length, spawns/despawns the delta, and
stamps each instance with `UiListKey(u64)` — reorders match by `UiListKey` exactly as `reconcile_ui`
matches by `UiName`, so there is **one identity discipline in the crate, not two**.

---

## 8 · The shader

**D30 — the UI shader migrates into `boyko_shaderdsl`, in the same rung that widens the instance.**

Verified state: `ui_rect.vs.hlsl` / `ui_rect.fs.hlsl` have **no `// === GENERATED … ===` sentinels**,
there is **no `ui` leaf** in `boyko_shaderdsl/src/`, **no row** in
`docs/SHADER-VARIANT-MANIFEST.md`, and **no `ui_*_spv_sync` / `ui_*_edsl_sync` test**. The only pin on
the committed `.spv` is the const-generic byte **length** (`SpirvBlob<2368>` / `SpirvBlob<7060>`,
`ui/mod.rs:122,129`) — which catches a size change but **not a re-compile drift at the same size**,
and does not prove the `.spv` matches the `.hlsl` beside it.

So the workspace's own rule — *HLSL the eDSL owns is generated, never hand-edited; committed `.spv`
are byte-gated* — **does not currently bind this file**, and nothing would notice a `.spv` that
stopped matching its source.

Both sprites and animation change the fragment shader. The options are exactly three, and only one
leaves a trace of the choice:

* **(a) Migrate into the eDSL** — the rule's intent, and the place where the per-corner rounded-box
  SDF, the MSDF median, and the new nine-slice/frame-UV arithmetic get an `f32` **host oracle**. This
  is a real scope item, not a formality: it means *bringing the UI leaves into the eDSL for the first
  time*.
* (b) Record a reasoned exemption plus, at minimum, a manifest row and a re-DXC gate.
* (c) Edit it a third time in silence.

**(a) is chosen.** The reason is that the campaign adds genuinely error-prone shader arithmetic
(nine-slice sub-rect math, frame-UV derivation, the un-aliased `uv`) to a file that today has *no gate
proving its binary matches its source*, and the eDSL's host oracle is exactly the instrument for that
class of arithmetic. (c) is named so it is on the record as an option that was refused.

**Required in the same rung:** sentinels + leaf + a `ui_rect_edsl_sync` test + a `ui_rect_spv_sync`
re-DXC test + a row per variant in `docs/SHADER-VARIANT-MANIFEST.md`.

---

## 9 · v1 versus deferred, and where the line is

**The line is drawn at: everything needed for one coherent, demonstrable advanced UI, with every
deferred item's *shape* recorded so the deferral is a decision and not a hole.**

### v1

| Area | In |
|---|---|
| Vocabulary | **D7 registration table first** — before any new component, landing as a pinned refactor (D7c) with the runtime-state components deliberately excluded (D7a) |
| Seam | **D31** — `boyko-ui` promoted to a production dependency of `boyko_render`; the canonical gather ships as `boyko_render::ui::gather_ui_nodes`, a DFS folding clip **and** scroll |
| Observer | **D32** — a minimal `boyko_app` UI rung: one `.ui` file, one sprite sheet, one animated hover, one scroll container |
| Sprites | `UiImage` consumed by pack; bindless set 1 (D2); 80 B instance (D1); nine-slice CPU expansion (D8d); uniform-grid sheets + flipbook; font-optional boot (D8e) |
| Animation | `UiVisual` sink + `TweenTint`/`TweenOpacity`/`TweenOffset`/`TweenScale` (D9) driven by **one fused tick** (D9b); the tier table enforced structurally, scroll row included (D10); `Changed<Interaction>` transitions via the discovery/exclusive-apply split **with** the reversing shortening factor (D14); real-clock default (D15); FLIP as the supported Tier-3 idiom, with its endpoint precondition stated (D11) |
| Interactivity | capture + press position (D16); capability routing on the ancestor snapshot (D17); drag (D18); computed `ComputedClip` + `ScrollExtent` from `Overflow`, with the scroll offset folded at **traversal** (D19/D19a) + momentum; single-line text input + the input-layer seam (D20); Shift+Tab, arrows, Escape, Space, focus groups, focus ring (D21); hover/press visual response + dwell + tooltips (D22); dirty-gated candidate rebuild + early-out (D23 items 1–2); slot-loop de-hardcoding (D24) |
| Render | the D6 generation gate, **wired, per-slot, and hoisted ahead of the gather**; `ui_render_discovery` as its single bump site (D6b) |
| Shader | UI leaves into the eDSL + sentinels + both sync gates + manifest rows (D30) |
| DSL | the Aether `ui` construct with `on click: Action` handlers (D25–D28) |

### Deferred, each with its reason

| Deferred | Why the line is here |
|---|---|
| **Springs** (D13) | Their only distinctive benefit is velocity-preserving retargeting. v1 ships the reversing shortening factor, which answers the same UX problem for tweens. Ship springs when a feature needs velocity continuity — otherwise they are a worse tween with a mandatory rest test. |
| **IME** (D20) | A VALUES/SCOPE call, and a large one (IMM32 vs TSF). The *shape* (`TextPreedit` added/removed) is fixed so the deferral does not become a redesign. Latin-only entry is the v1 deliverable. |
| **Multi-line text / rope** | The inline-POD buffer covers single-line. Multi-line wants a `Resource`-owned rope column — a different storage decision that no in-tree consumer needs yet. |
| **Multi-touch + gesture arena** (D24) | `MAX_POINTERS = 1` is already the right *shape*; only the constant changes. The arena is the correct answer **when touch lands**, because that is when the conflict set becomes open-ended. Building it now would be a mechanism with one hard-coded consumer. |
| **Rotation / non-AABB clip** (D5) | It invalidates the per-instance AABB clip **and** the axis-aligned hit-test **and** needs an eDSL change and a `.spv` re-bless — three subsystems at once, for a visual effect nothing has asked for. |
| **Timeline clips + players** (§5.8) | The shape is fixed (clip-as-asset + cursor-as-component + group-key sequencing). Nothing in v1 needs multi-channel keyframes; per-channel tweens cover hover, fade, slide and flipbook. |
| **`UiRepeat` dynamic lists** (D29) | The shape is fixed. It depends on the reconciler's identity discipline being extended to list keys, which is its own rung. |
| **Ragged / trimmed sheets, per-frame durations** (D8c) | Uniform grids need *zero* per-frame storage; ragged needs a table plus an asset-pipeline dependency, and **no sprite asset exists in the repo at all**. |
| **Runtime UI atlas (Model A)** (D2) | Reachable **without changing one component** if §10.1 says the divergence cost is real. That reversibility is why B is safe to start with. |
| **Per-root layout dirty bits** (D11) | Buildable today via `Query<&UiRootIndex, Or<…>>`. With nothing animating the current design is provably free, and FLIP removes the steady-state case. **Measure before building** (§10.4). |
| **Easing partition by column** (D12) | The shape is fixed and the reasons against a per-row `match` stand. v1 runs the fused tick (D9b) with the easing branch per row; the partition ships **iff** §10.5 shows the tick is material. Shipping it unmeasured would be the "arithmetic instead of a measurement" failure this document refuses elsewhere. |
| **SoA hit-test + SIMD** (D23 item 3) | The arithmetic is a hypothesis; the probes are the likely cost. Gated on §10.6. |
| **Inline handler bodies in Aether** (D27) | The right long-term answer, but it must synthesise an enum coexisting with the user's own — a language design question that should not ride in the construct's introducing rung. |
| **Opaque pre-pass / overdraw** (§4.4) | Noted as the one lever left if fill rate dominates. No reference solves it in the batcher. |

---

## 10 · Measurement obligations

Every cost claim above that is not arithmetic lands here. **None of these numbers exists yet.** The
project's recorded failure mode is a gate that could not fail and a number that was not measurable, so
each entry names the instrument and the discriminating comparison — not just "benchmark it".

| # | Claim under test | Instrument | Discriminating comparison |
|---|---|---|---|
| **10.1** | D2's `NonUniformResourceIndex` divergence is affordable | GPU timestamp around the UI pass | One frame of N textured quads at **1 / 8 / 64 distinct slots** vs **all quads on one slot**. If the 64-slot case regresses materially against 1-slot, Model A is reachable with no component change. |
| **10.2** | D1's widening is affordable | ring bytes/frame (exact arithmetic: 128 KB → 160 KB at N=2048) **plus** wall-clock pack+sort | Same scene, 64 B vs 80 B record. The bytes are arithmetic; the pack+sort time is not. |
| **10.3** | D6's gate does what its doc says — **and what it cannot do** | a repack counter | Static frames: repacks before vs after. **And** an animating frame, reported unchanged, so the doc never again claims more than the mechanism delivers. |
| **10.4** | D11 — whether per-root dirty bits are needed — **and D19a's tier claim, which is a pass/fail gate, not a number** | `LayoutScratch.relayout_count` **promoted from `#[cfg(test)]` to a diagnostic counter**; plus the candidate-rebuild counter for the scroll leg | Roots re-laid-out per frame: static; one Tier-1 animation; one Tier-3 animation; the same animation via FLIP; **and a scroll container coasting under `ScrollMomentum` for ≥30 frames**. If FLIP drives Tier-3 to two relayouts total, the granularity fix stays deferred. **The coasting leg is a GATE: `relayout_count` MUST be 0 across the whole fling.** A non-zero value means the scroll offset has re-entered the layout path — the exact defect D19a exists to remove, and the one the original record shipped. The same leg reports the candidate rebuilds it *does* cost (D19a's honest cost), so the trade is on the record as two numbers rather than one claim. |
| **10.5** | D9's tween tick cost, honestly | criterion | N ∈ {8, 64, 512} animating rows. **This is expected to be small** — the design does not depend on it, and §2/C3 says so up front. |
| **10.6** | D23 item 3 — SoA vs the probes | criterion over the focus pass | N ∈ {100, 1000}: today's AoS+probes vs dirty-gated-only vs dirty-gated+SoA. Item 3 ships **only** if it beats item 1 alone. |
| **10.7** | The eDSL migration is faithful | `ui_rect_edsl_sync` + `ui_rect_spv_sync` | Byte identity of re-emitted HLSL and re-DXC'd `.spv`. This gate does not exist today in **any** form. |
| **10.8** | **The gather** — the one cost this campaign adds to every node of every frame (D5, D6a, D31) | a probe counter in `gather_ui_nodes` **plus** wall-clock over the gather alone, separated from pack+sort (which §10.2 owns) | Probes/node/frame and gather µs at N ∈ {256, 2048}, in four states: **(a)** today's baseline (rect-only reads); **(b)** + `UiVisual`; **(c)** + the four sprite components; **(d)** a **static** frame with the D6 compare hoisted ahead of the gather — which must be **zero probes**, or D6a is not wired where it claims to be. This is the number §10 previously omitted entirely, and it is the one that decides whether the sprite lane needs an archetype-shaped gather rather than per-node probes. |
| **10.9** | **D7 reproduces the hand-written vocabulary before it extends it** (D7c) | the existing `.ui` round-trip + hot-reload equivalence corpus, re-run against the generated table | For all **19** existing components: identical spawned worlds, identical `UiParseReport` diagnostics (message, line, column), identical `serialize_ui` bytes. A component that cannot be reproduced takes `#[ui_vocab(manual)]` and is listed. **The rung does not close on judgement**, and no new component is added until this is green — which is what bounds R4. |

---

## 11 · Sequencing

Ordered by dependency and by the cost of doing it late.

1. **D7 — the `.ui` registration table.** First, because it converts a linear per-component cost into
   a constant one *before* the 12 authored components are added (D7a). Doing it after means writing
   ~60 landings and deleting them. It lands as a **pinned refactor**: **§10.9 must be green — the
   generated table reproducing all 19 existing components — before rung 4 adds the twentieth.** That
   pin is what bounds **R4**.
2. **D31 + D6 + D32 — the seam, the gate, and the observer.** These three are one rung because each
   is worthless without the others: D31 promotes `boyko-ui` to a production dependency of
   `boyko_render` and ships `gather_ui_nodes`; D6 wires the per-slot generation gate **hoisted ahead
   of that gather**, with `ui_render_discovery` as its single bump site; D32 stands up the minimal
   `boyko_app` rung that actually calls them. **Everything sequenced after this point is visible to a
   human**, which is the entire content of R1's and R2's mitigations. §10.8's baseline leg (a) and
   static leg (d) run here.
3. **D30 — the eDSL migration + both sync gates** (§10.7). Before D1, per R1: the re-DXC gate must
   exist *before* the shader is edited, or nothing notices a `.spv` that stops matching its source
   while the shader is being changed repeatedly.
4. **D1 — widen `UiInstance` to 80 B**, all seven lockstep sites in one commit, with the field list
   from §3 fixed. Both feature halves depend on it and neither may widen it again. §10.2 runs here.
5. **Sprites** (D2, D3, D8a–e) — the first consumer of the new `uv` field and set 1. §10.1 and
   §10.8's legs (b)/(c) run here.
6. **Animation** (D9–D15) — `UiVisual` + the fused tick (D9b) + transitions via the discovery/apply
   split (D14). §10.5 and §10.4's static / Tier-1 / Tier-3 / FLIP legs run here.
7. **Interactivity** (D16–D24) — capture first (largest effect, smallest change), then D19's computed
   `ComputedClip`/`ScrollExtent` (scroll's substrate) and D19a's traversal fold, then D20's
   input-layer seam, then the rest. §10.6 runs here, and so does **§10.4's coasting-scroll gate** —
   the one leg of §10.4 that is pass/fail rather than informational.
8. **Aether `ui`** (D25–D28) — **last**, deliberately. Aether cannot usefully name what does not
   exist, and Aether's own rule ("one table, spelling and dispatch together") means a construct whose
   vocabulary is still moving will churn its own diagnostics corpus, every diagnostic of which is a
   trybuild golden. `aether-tests` gains its `boyko-ui` dev-dependency in this rung (D27), or the
   emitted paths have no in-repo compiler.

---

## 12 · The four biggest risks

### R1 — The instance widening is a six-site lockstep change on a path with no live host and no binary gate

D1 must land, in one commit and consistently: the Rust struct; **ten** `offset_of!` const-asserts; two
HLSL mirrors; two `SpirvBlob<N>` byte-length pins; `pack_ui_instance`; and the Miri byte-view test.

What makes this the highest-consequence item is not the change — it is the **observability**:

* there is **no production host that draws any UI** (`boyko-ui` is a *dev-dependency* of
  `boyko_render` and of nothing else; `boyko_app` never draws UI), so the only consumers are tests;
* the GPU goldens **skip gracefully on a device-less host** (`ui_rect_gpu_golden.rs:36-39`) — a green
  CI run may have exercised nothing;
* `ui_hud_screenshot.rs` is `#[ignore]`d eight times (owner-run);
* and the **only** pin on the committed `.spv` is a byte **length**, which cannot see a re-compile
  drift at the same size.

So the shader half of a wrong widening is invisible to every automated gate that exists today.

*Mitigation, and it is why the sequencing puts them first:* **D30 before D1** — the re-DXC gate must
exist before the shader is edited. Plus the recorded lesson from the boyko_app host campaign: host
render rungs need a golden-**independent** visual regression, because owner-eval caught three bugs
there that the autogates caught zero of. That lesson is now a decision rather than an aspiration —
**D32** makes the minimal host rung a v1 deliverable sequenced at rung 2, *ahead of* the widening it
has to observe.

### R2 — The tier discipline can be silently absent, because the pack is fed by a host-supplied closure

D10's whole enforcement rests on the pack reading `UiVisual`. But the pack does not gather nodes —
**the host does**: `host_upload_frame_from_world(world, node_buf, gather_nodes, …)` takes
`F: FnOnce(WorldView, &mut Vec<UiNode>)` (`ui/upload.rs:255`). Nothing in the crate forces that
closure to read `UiVisual`, `UiImage`, or `UiNineSlice`.

A host that does not opt in gets a UI where **animation silently does nothing** and **sprites silently
render nothing** — which is *exactly* the state `UiImage` is in today, for exactly this reason. This
campaign would be re-creating the defect it exists to fix, one layer up, and the "dead datum" class
already has **five** recorded instances in this project (`site.decode`, `LogSite.fields`, twelve
unbuilt benches, `sample_shift=2`, `intern_site`) plus the sixth verified here in §1.

*Mitigation — and the crate edge it crosses, which the first draft of this risk did not name.* The
mitigation "ship the canonical gather as a crate-provided function" presumed a home for that function
and there was none: `boyko_ui` names no render crate, and `boyko_render`'s `boyko-ui` line is a
dev-dependency annotated *TEST-ONLY*. **D31 settles it** — `boyko-ui` becomes a production dependency
of `boyko_render`, the gather ships as `boyko_render::ui::gather_ui_nodes`, and the host's job becomes
"call this" rather than "write one". The rot-proof half is D31's point 2: the pack-input set is spelled
**once**, in a macro that expands to both `ui_render_discovery`'s `Or<(Changed<…>, …)>` and the
gather's read list, so a new visual component either joins both or fails to compile. A completeness
test that merely asserts "the gather reads every pack-input component" against a hand-kept list is
checking a list against itself — the shape this project has recorded five times as a dead datum.
**D32** then supplies the observer that would notice if it broke anyway.

### R3 — Capability routing may be the wrong spine, and the failure is not local

D17 chooses routing over propagation as the default. The counter-evidence is that **five independent
retained UIs** — DOM, Unity UI Toolkit, Godot, Unity uGUI, and Bevy in its most recent redesign —
converged on full propagation. That is weak evidence about performance and **strong** evidence about
expressiveness.

The specific failure: "handle it in the parent unless the child consumed it" — the most common
composition idiom in every retained UI ever shipped — is **not expressible in the default path**. It is
expressible only through the opt-in observer escape hatch, which means the escape hatch may become the
common path, and then the campaign has paid for two mechanisms and standardised on the one it called
exceptional.

And the blast radius is structural, not local. Under D17 the resolve pass emits *one target plus a
bounded ancestor snapshot*; under propagation it emits *a path*. Every state machine keyed on "the
target" — drag, scroll, tooltip, gesture — would have to be re-keyed on "the path". **Discovering this
late means rewriting the interaction spine, not extending it.**

*Mitigation:* build the opt-in bubbling hatch (D17) **in v1, not later**, so the question is
answerable by use rather than by argument; and keep the resolve pass's output type in **one** place so
that widening "target" to "path" is a single edit rather than a survey. Track adoption: if the hatch
becomes the common path in practice, that is the measurement saying to invert the default.

### R4 — D7 is the campaign's first rung and its largest unestimated item, and everything is behind it

D7 is sequenced first and gates sprites, animation, interactivity and the Aether construct. Its
original justification counted only the cost of *not* doing it, and its claimed reuse —
*"exactly as `register_bind_accessor` already installs a `ComponentId`-keyed fn-pointer table"* — is
true of the installation mechanism and **false of the content**: `Bindable` is read-only
(`bindable.rs:23-46`), and the path D7 replaces is type-directed by the destination field with **no
standalone value parser** (`dispatch.rs:12-14`). What D7 actually builds is a new type-directed
parse/serialize codegen framework (D7a), and a framework at the head of a campaign is where schedules
go to die.

*Mitigation, in three parts, all now in the plan.* **(1)** The parts list is enumerated (D7a) rather
than gestured at, and the honest reuse is named: the ~15 leaf parsers already exist as free fns at
`dispatch.rs:545-875`, so the field impls are delegations, not new parsing logic. **(2)** The
irregulars have a declared escape hatch (D7b, `#[ui_vocab(manual)]`) — a derive that had to express
`StackIndex`'s tuple-only form, the `BindParse<C>` two-pass deferral and `resolve_action_name` would
have no end. **(3)** The rung's end condition is a **gate, not a judgement**: §10.9 requires the
generated table to reproduce all 19 existing components — worlds, diagnostics and round-trip bytes —
before the twentieth is added.

*The residual risk, stated because the mitigation does not remove it:* if §10.9 stays red, the whole
campaign is blocked behind a refactor. The fallback is explicit — components move to
`#[ui_vocab(manual)]` until §10.9 is green, and in the limit D7 degenerates to today's hand-written
match with a table wrapper, which is strictly no worse than the status quo and unblocks rung 2.

---

## 13 · Open questions for the owner (VALUES / SCOPE — also to be filed in `docs/OPEN-QUESTIONS.md`)

These are not perf or architecture forks (those are decided above, with numbers or with reasons). They
are scope and values calls.

1. **IME scope.** Is East-Asian text input in scope for this campaign, or is Latin-only entry the
   deliverable with IME deferred (this design assumes the latter)? If in scope: IMM32 or TSF?
2. **Touch.** Is `MAX_POINTERS > 1` in scope? It flips D24's gesture recommendation from ladder to
   arena and makes implicit capture mandatory rather than optional.
3. **Bubbling.** D17 recommends "opt-in escape hatch". The owner may prefer **"never"** — defensible,
   simpler, and it makes R3 a closed question rather than a tracked one.
4. **`.ui` capability parity.** BSN accepted a permanent code-vs-asset gap. `.ui` structurally cannot
   host a Rust expression. Accepting the same gap here is legitimate **but must be a decision**,
   because the equivalence gate is currently written as if the two surfaces are equal.
5. **How much demo above the floor.** Not *whether* the campaign has an observer — that is an
   observability call and **D32 decides it**: the minimal `boyko_app` rung plus its two first-in-repo
   assets is a v1 deliverable, sequenced at rung 2, because R1 and R2 are otherwise unfalsifiable and
   this document may not name observability its top two risks and then leave the observer to a
   question. What is left for the owner is scope *above* that floor: is a richer showcase scene
   wanted inside this campaign, or does the minimal rung stand until a game needs more?

---

## 14 · Sources

**In-tree, read for this document** (worktree `D:/wt/ui`, branch `feat/ui-advanced`):
`crates/boyko_ui/src/{components,layout,widgets,bundles,resources,plugin}.rs` ·
`crates/boyko_ui/src/interaction/{focus,dispatch,action,components,plugin}.rs` ·
`crates/boyko_ui/src/text/dispatch.rs` · `crates/boyko_ui/src/reload/reconcile.rs` ·
`crates/boyko_ui/src/binding/{components,bindable}.rs` · `crates/boyko_ui/Cargo.toml` ·
`crates/boyko_render/src/ui/{instance,pack,upload,resources,mod}.rs` ·
`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:2112` ·
`crates/boyko_render/Cargo.toml` (the layering rule at `:7-13`; the dev-dep block at `:106-118`) ·
`crates/boyko_rhi_vulkan/src/bindless.rs:72` · `crates/boyko_render/src/bindless.rs:80-93` ·
`crates/aether_tests/Cargo.toml` (the R4 anti-drift + blast-radius discipline) ·
`crates/boyko_ui/src/lib.rs` (root re-exports vs `prelude`) · `crates/boyko_input/src/lib.rs:45` ·
`crates/boyko_input/src/action/actionlike.rs:50` ·
`crates/boyko_ecs/src/ecs/core/time/time.rs` ·
`crates/boyko_ecs/src/ecs/core/iters/query/query.rs` (`dense_iter`, `assert_dense_iter_no_enable`,
and the `Changed`-under-`dense_iter` open question at `:112-114`) ·
`crates/boyko_ecs/src/ecs/core/iters/query/data.rs` (`AnyOf`'s ≥1 predicate at `:136`; the dense-seed
note at `:227-228`) ·
`crates/boyko_ecs/src/ecs/core/component/dense/{mod,dense_store}.rs` ·
`crates/boyko_ecs/tests/{dense_d2_routing,dense_d4_change_detection,dense_enable_query_miri}.rs` ·
`crates/boyko_input/src/action/{actionlike,names}.rs` · `crates/boyko_input/src/raw/queue.rs` ·
`crates/aether_lang/src/{parse,ast,expand}.rs` (`scene_fn`, `emit_node`, `spawn_call`)

**Research corpus:** `docs/UI-ADVANCED-RESEARCH-{SPRITES,ANIMATION,INTERACTION,DSL}.md` — which carry
the full external citation lists (Bevy, Unity uGUI + UI Toolkit, Godot 4, Dear ImGui, RmlUi,
WebRender/Chromium, Flutter, WPF, CSS Transitions, React Native, LitMotion/PrimeTween/DOTween,
Iced/Xilem/Dioxus/Leptos, Flecs, QML, Aseprite/Unreal flipbooks, Juckett springs, UIKit scroll physics)
rather than duplicating them here.
