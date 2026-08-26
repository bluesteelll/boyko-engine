# UI-PLAN-ANIMATION — animation columns, easing and the layout seam

**Campaign:** advanced UI/GUI for `boyko_ui` · **Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`)
**Date:** 2026-08-21 · **Status:** plan, pre-implementation
**Authority:** [`docs/UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) §5 (D9–D15), §3 (D5, D6, D10, D31), §10.4/§10.5, §12 R2.
**Evidence:** [`docs/UI-ADVANCED-RESEARCH-ANIMATION.md`](UI-ADVANCED-RESEARCH-ANIMATION.md).
**Siblings:** [`docs/UI-PLAN-SPRITES.md`](UI-PLAN-SPRITES.md) · [`docs/UI-PLAN-INTERACTION.md`](UI-PLAN-INTERACTION.md) · [`docs/UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md).

---

## 0 · How to read this

This is the ladder a developer walks. Each rung is independently landable and leaves the workspace
green. Each rung states **what lands**, **what gates it**, and a **RED MUTATION** — a one-line edit
that must turn the gate red. *A gate whose red nobody has seen is not a gate*: this project has
recorded five "dead datum" instances and a whole ladder of benches that did not exist, and every one
of them was found by running the red, never by reading the green.

**Decisions are numbered `AD<n>`** and carry a reason plus the alternatives rejected. Decisions
already made by the architecture (D9, D9b, D10–D15) are **cited, not restated**; only where this plan
*corrects* the architecture does it say so, in §1, at the source.

**What this plan owns**, and nothing else:

| Owned here | Owned elsewhere |
|---|---|
| The UI clock (`UiClock`) — the one delta source for every time-varying UI subsystem | `Time` itself (`boyko_ecs`) |
| `UiVisual` (the sink), the four `Tween*` channel columns, the fused tick (D9/D9b) | — |
| `EasingId`, the built-in family, the custom-curve LUT (D12) | — |
| `UiStateTint` + the `Changed<Interaction>` transition driver (D14) | The production of `Interaction` (`ui_focus_system`, ships today); dwell/tooltips (D22 → interaction plan) |
| The **definition** of the Tier-2 visual transform: its origin, its inheritance, its clip composition | The **gather** and `pack_ui_instance` themselves (D31/D5 → sprites plan) |
| FLIP (D11) and the Tier-3 relayout measurement (§10.4) | The per-root dirty-bit fix (deferred by D11) |
| The hit-test's visual fold | `collect_candidates`' structure (D17 → interaction plan) |
| The tier table's animation rows (D10) | The sprite flipbook (`UiSpriteCursor`, a Tier-1 **step** channel, not a tween — research §8.4) → sprites plan |

**No rung in this ladder touches a shader.** Stated explicitly because the campaign rule requires it:
opacity folds into the existing premultiply and offset/scale fold into `min_px`/`size_px`, all
CPU-side in `pack_ui_instance`. There is no new HLSL, no new `.spv`, no `*_edsl_sync`/`*_spv_sync`
pin and no `docs/SHADER-VARIANT-MANIFEST.md` row owed by animation. The one place a shader could sneak
in — `UiVisual.uv_shift` — is removed in **AM5** for exactly that reason. The D30 eDSL migration and
the D1 widening remain the sprites plan's rungs; this plan depends on neither.

---

## 1 · Amendments to the architecture

Six statements in the authority did not survive a read of the kernel. They are corrected here at
their source with the fact that forced each, per this project's standing rule that a diverged pair is
worse than a missing one. **Each must be folded back into `docs/UI-ADVANCED-ARCHITECTURE.md` in the
same commit that lands the rung which depends on it.**

### AM1 — `&mut UiVisual` bumps no tick, so D9b's own query defeats D10's enforcement

D9b specifies the fused tick as `Query<(&mut UiVisual, AnyOf<(&mut TweenTint, …)>)>`. D10 then says
*"A Tier-1/2 channel dirties the render gate by being written: the sink type is a term of
`ui_render_discovery`'s change set."*

`&mut T` **has no change tracking**:

> "Compared to `&mut T` (no change tracking), `Mut<T>` is the path that participates in change
> detection." — `boyko_ecs/src/ecs/core/iters/query/data/mut_.rs:12-14`

and the dense side is pinned the same way:

> "A `Mut<Dense>` write (deref) makes the row visible to `Changed<Dense>`; an untouched dense
> component is NOT `Changed` on an idle frame." — `boyko_ecs/tests/dense_d4_change_detection.rs:8-10`

**As written, D9b's tick would advance `UiVisual`'s bytes and never advance its tick**, so
`ui_render_discovery` would not bump the generation, the D6 gate would short-circuit, and **every
animation would render nothing** while every unit test on the tick's arithmetic stayed green. That is
the campaign's own R2 failure — animation silently doing nothing — reached through a different door.

**Correction:** the sink term is `Mut<UiVisual>`, written through `Mut::set_if_neq`
(`mut_.rs:84`), which is the crate's existing set-if-changed discipline (`widgets.rs:205`) and keeps a
value-preserving frame from bumping. The channel arms stay `&mut Tween*` — nothing reads
`Changed<Tween*>`, and paying a tick bump for a datum nobody filters on is the cost D9's reason 2 is
trying to avoid. **Gate: A1's red mutation #1.**

### AM2 — D9b's "a `UiVisual` row with no live channel is skipped" is false for an all-dense `AnyOf`

D9b leans on `AnyOf`'s ≥1-member predicate to skip resting rows, and this matters because it says so
itself: `UiVisual` **persists after a tween finishes**, so `UiVisual` rows accumulate to *every
element that has ever animated*.

For a **dense** member the predicate is vacuous:

```rust
fn matches_component_set(state: &Self::State, mask: &ComponentMask) -> bool {
    // Dense is signature-excluded … a dense include must NOT gate at the archetype level.
    if const { T::STORAGE_IS_DENSE } { return true; }
    mask.contains(state.id)
}
```
— `data/mut_.rs:239-247` (identical in `read.rs`). `AnyOf` folds those with `||`
(`data/anyof.rs:171-175`), so with four dense arms the OR-trim is `true` for every archetype. And the
per-row escape is deliberately absent:

> "`dense_row_passes` is NOT forwarded (AnyOf's ≥1-member semantics keep the default `true` — a row
> missing one OR-arm still yields `(…, None, …)`, never a skip)." — `data/anyof.rs:112-117`

**So a rested row is *visited*, not skipped, and yields `(None, None, None, None)`.** Two consequences,
both v1:

1. **The tick MUST `continue` on the all-`None` case before touching the sink.** Without it every
   element that has ever animated gets a `Mut<UiVisual>` deref every frame; `set_if_neq` saves the
   tick bump only because the value is unchanged — one `PartialEq` on 24 B per ever-animated element
   per frame, forever. This is D13's "a spring that never rests" leak arriving through the rested
   path instead.
2. **§10.5's bench measures the wrong axis as specified.** "N ∈ {8, 64, 512} animating rows" cannot
   see this cost at all. The bench needs a **second axis: the resting population** — see §4.

**The cost model, corrected.** `Mut<UiVisual>` is a dense include, so candidates are seeded from the
store's `arch_presence` (`state.rs:126-137`, `dense_store.rs:377`) — but seeding is **per archetype**,
and within a seeded archetype **every row is visited** and rejected by `dense_row_passes`
(`iter.rs:550-559`). Animated and un-animated panels share an archetype in any real UI. So:

> per frame ≈ (rows in archetypes hosting ≥1 `UiVisual`) × 1 sparse-map probe
> &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;+ (`UiVisual` rows) × 4 further probes (one per `AnyOf` arm, inside `fetch`).

That is the number §4/M2 must report, and it is not the number the architecture's §10.5 asks for.

### AM3 — D5's fold formula is correct only for a node's own background quad

D5 states the fold as `min_px = (rect.xy + visual.offset) * s` / `size_px = (rect.wh * visual.scale) * s`.

Two defects:

* **A node emits more than one quad.** Glyphs go through the same seam —
  `PackInput { text_uv: Some(uv), .. }` (`boyko_ui/src/text/emit.rs:25`), and the sprites plan's
  nine-slice adds nine more (D8d) *(and takes the image record away in exchange, so a
  nine-sliced imaged node is 10 quads rather than 11 — `UI-PLAN-SPRITES.md` **S-D12 (1)**, 2026-08-21;
  it does not change this argument, which needs only "more than one")*. Applying `rect.wh * scale` to a glyph scales each glyph **in
  place**: the letters grow, the word does not. A scaled label becomes overlapping mush.
* **The transform has no origin.** Scaling about the top-left is not what any UI means by "pop on
  hover". CSS's default is `transform-origin: 50% 50%`; Flutter, Unity uGUI and Godot all default to a
  centred pivot.

**Correction: the transform is affine, origin-relative, about the node's rect centre** — see
**AD3**, which states the normative form and the composition rule inheritance needs.

### AM4 — the Tier-2 transform **inherits**; the architecture never answered it

Research §10 Q2 names this "the biggest unresolved design question" and the architecture is silent.
Silence resolves to "does not inherit", and that answer makes the single most common UI animation —
a panel sliding in *with its contents* — impossible to express as one animation. See **AD4**.

### AM5 — `uv_shift` leaves `UiVisual` in v1

D9's struct carries `uv_shift: [f32; 2]` ("sprite-frame nudge"). **No v1 animation channel writes it**
— there is no `TweenUvShift`, and D9's own comment says the flipbook writes `frame`, not this. A field
that ships with no writer is the dead-datum class this project has recorded six times. Removing it
takes `UiVisual` from 32 B to 24 B and takes this plan's shader exposure to exactly zero.

**The deferral records its shape** so it is a decision, not a hole: a sprite-UV nudge channel is a
Tier-1 `TweenUvShift` dense column plus two `f32` on `UiVisual`, landing in whichever rung first needs
sub-frame sprite scrolling, and it costs no `UiInstance` byte because the sprites plan's `uv` field
(D1) already exists by then.

### AM6 — `Time` caches no real-delta `f32`, and the real delta is **unclamped**

`Time::delta_secs()` is a cached `f32` of the *virtual* delta (`time.rs:64-70`); the real side offers
only `real_delta() -> Duration` (`time.rs:80-83`). And `DEFAULT_MAX_DELTA = 250 ms` (`time.rs:23`)
clamps the **virtual** delta only — *"Raw delta of the current frame — unclamped, unscaled,
pause-blind."*

D15 makes the real delta the UI default. Taken literally, an alt-tab stall or a shader-compile hitch
delivers a 2-second real delta and **every running transition jumps straight to its end**, which reads
as a visual glitch on resume, not as an animation. **`UiClock` therefore applies its own clamp** — see
**AD1**. No kernel change; one `as_secs_f32()` per frame, not per row.

---

## 2 · Decisions

### AD1 — `UiClock` is a resource, not a `Res<Time>` read at each consumer

```rust
#[derive(Resource)]
pub struct UiClock {
    /// Clamped real delta, seconds. The default tween clock (D15).
    dt_real: f32,
    /// Clamped virtual delta, seconds — zero while `Time` is paused.
    dt_virtual: f32,
    /// UI-local hitch clamp applied to BOTH deltas. Default 100 ms (AM6).
    max_delta: f32,
}
```

Written once per frame by `ui_clock_tick` (a normal system, `Res<Time>` in, `ResMut<UiClock>` out),
read by every time-varying UI system.

**Reason.** (1) **One clamp, one place.** AM6's hazard is invisible until someone alt-tabs; putting
the clamp at each consumer means the third consumer forgets. (2) **One `Duration → f32` conversion
per frame** instead of one per consumer. (3) **It is the seam the siblings need.** The sprites plan's
flipbook and the interaction plan's `ScrollMomentum` are both time-varying and neither is a tween;
they read `UiClock`, not `Time`, so "the UI's clock" is one decision rather than three. (4) A per-row
`flags` bit selects `dt_virtual` — D15's opt-in — and selecting between two `f32` in a resource is a
`select`, not a branch on a `Duration`.

**Rejected:** (a) *each system reads `Res<Time>`* — three copies of the clamp decision and three
chances to pick the wrong delta; the pause-menu bug D15 exists to prevent is exactly a
wrong-delta-picked-once bug. (b) *a UI-local wall clock* — a second time source that disagrees with
the engine the first time anyone pauses (D15's own rejection). (c) *no clamp, trusting `Time`'s* —
`Time`'s clamp does not reach the real delta (AM6).

**The clamp value is a VALUES call** and is filed as Q1 in §7. 100 ms is the default because it is
below the shortest hitch a user perceives as a stall and above any frame time a shipping build
targets; it is not measured, and the plan says so.

### AD2 — `EasingId` is a `u8` with a reserved custom half

```rust
#[repr(transparent)]
pub struct EasingId(u8);
// 0 ..= 29   built-in: family * 3 + direction, families in RmlUi's declaration order
// 30..=127   reserved (a const-assert holds the boundary)
// 128..=255  custom: LUT index = id - 128, into the Resource-owned curve table (D12)
const _: () = assert!(EasingId::BUILTIN_COUNT <= 128);
```

**Reason.** The custom test is `id & 0x80` — one branchless bit, not a comparison against a
mutable table length. The built-in encoding `family * 3 + direction` makes "same curve, other
direction" arithmetic, which is what a reversing transition (D14) needs and what an authoring surface
(the Aether plan) wants to spell. 128 custom curves is more than any UI has ever had.

**Rejected:** (a) *an open `u16` index into one table holding both* — makes every built-in a table
load, which is D12's rejected option (b) applied to the common case; (b) *a separate `CustomEasingId`
type* — two types where the tween row must store one, so the row grows a discriminant to say which.

**`linear` in/out/in-out are three ids over one body.** Pinned, so the authoring surface stays
uniformly `family × direction` and no author has to know which family is degenerate.

**The built-in family is D12's set verbatim** (RmlUi's ten × three); this plan adds no curve and
removes none.

### AD3 — the visual transform is affine, origin-relative, and composes as a `(scale, translate)` pair

Normative. For a node with laid-out rect `r` (from `ComputedRect`), centre `c = r.xy + r.wh * 0.5`,
and `UiVisual { offset: o, scale: s }`, the node's **local** transform is the map
`p ↦ s ⊙ p + (c − s ⊙ c + o)`, i.e. the pair

```
S_local = s
T_local = c − s ⊙ c + o
```

Every quad the node emits — background, glyphs, and the sprites plan's nine sub-quads *(which stand in
for the node's image record rather than beside it — `UI-PLAN-SPRITES.md` **S-D12 (1)**, 2026-08-21)* —
is transformed by the **same** pair: `min' = S ⊙ min + T`, `size' = S ⊙ size`. For the node's own
background quad this reduces to D5's formula plus the centring term; for a glyph it is the
origin-relative form AM3 requires.

**Composition (what inheritance needs, AD4):** two axis-aligned scale-translate maps compose to a
third, so the DFS stack carries one pair and folds
`(S, T) ∘ (S_l, T_l) = (S ⊙ S_l, S ⊙ T_l + T)` — **four multiply-adds per node**, the same shape and
the same stack the gather already carries for the inherited clip and scroll offset (D31 point 3,
D19a).

**Opacity** is not part of the pair: it is a scalar that multiplies down the stack and folds into the
existing premultiply (D5), costing zero GPU bytes. **Tint** multiplies component-wise in straight
RGBA8 before the premultiply.

**Rejected:** (a) *top-left origin* — D5's literal formula; a hover pop drifts the element
down-right, which is why no shipped UI defaults to it; (b) *an authored per-node pivot* — a fifth
field on `UiVisual` for a case nothing in v1 asks for, and it does not compose more cheaply; the
shape is recorded and deferred; (c) *a full 2×3 matrix per node* — buys rotation, which D5 defers for
three independent reasons (instance clip, hit-test, shader), and doubles the stack fold.

### AD4 — the Tier-2 transform inherits, multiplicatively, on the gather's existing DFS

A parent's offset/scale/opacity/tint compose into every descendant's, folded on the gather's descent
(D31 point 3) by AD3's rule. **The inherited clip is transformed by the same accumulated pair at the
point of descent**, or a scaled subtree is clipped against a rect that did not move with it.

**Reason.** (1) "Panel slides in with its contents" is *the* canonical UI animation and without
inheritance it is N animations that must stay in phase. (2) The cost is four multiply-adds on a stack
tuple that already exists and is already being pushed for clip and scroll — this is the cheapest
place in the whole campaign to add a feature. (3) It matches CSS, Flutter, Godot and Unity, so no
author is surprised. (4) It keeps `ComputedRect`'s single-writer invariant (`widgets.rs:6-9`) intact —
nothing is written; the fold is read-side, at gather.

**Opacity inherits multiplicatively, and this is explicitly NOT CSS group opacity.** CSS composites a
subtree as one layer, so two overlapping children at group opacity 0.5 do not darken each other.
Multiplying per element does. **Group opacity is deferred with its reason**: it requires an offscreen
layer, which is a second render target and a second pass, which is categorically outside a one-draw
batcher (D2/§4.4). Authors who need it compose non-overlapping children. Recorded here so the
difference is a decision and not a bug report.

**Rejected:** (a) *no inheritance* — the architecture's silent default; makes the most common
animation inexpressible (AM4); (b) *inheritance as a propagation **pass** writing a computed
component* (research §6.5's "could fold into `ui_layout_apply`'s walk") — a second per-node durable
datum, a second writer to keep single, and a full tree walk on a frame where one leaf animates; the
gather already walks the tree, so the pass would be a second walk buying nothing.

### AD5 — the tick is one fused system with an all-`None` early-out; the write is `Mut::set_if_neq`

D9b's shape, with AM1 and AM2 applied:

```rust
pub fn ui_visual_tick(
    clock: Res<UiClock>,
    mut q: Query<(Mut<UiVisual>, AnyOf<(&mut TweenTint, &mut TweenOpacity,
                                       &mut TweenOffset, &mut TweenScale)>)>,
    mut done: ResMut<UiTweenScratch>,   // retained completion list; no per-frame alloc
) { … }
```

Per row: if all four arms are `None`, `continue` **before** touching the sink (AM2). Otherwise compose
the four channels into a stack-local `UiVisual`, then `sink.set_if_neq(composed)` (AM1). A channel
whose `elapsed >= duration` writes its final value and pushes its `(Entity, ComponentId)` onto the
retained completion list; **removal is deferred to `ui_tween_reap`**, an exclusive system running
immediately after, because a dense remove inside an iterating query is a structural op during
iteration.

**Reason for the deferred reap rather than `Commands`:** `Commands` would land the removal at the next
apply window, which is fine for a *removal* (the row's last write already happened) — but the reap
also has to run before `ui_transition_apply` can observe "no row present ⇒ at rest", and an exclusive
system in the same set gives that ordering explicitly instead of depending on where the command
buffer drains. It is the `ui_bar_apply` shape (`widgets.rs:110-166`) with a smaller body.

**`UiVisual` stays single-writer** — the crate's `ComputedRect` discipline (`widgets.rs:6-9`)
— and D9b's rejected alternatives (four systems; four systems plus four sinks; Model A; Model C; the
EnableTag) stand unchanged and are not re-argued here.

### AD6 — `UiVisual::default()` is the **identity**, hand-written, and const-asserted

```rust
impl Default for UiVisual {
    fn default() -> Self {
        UiVisual { tint_mul: 0xFFFF_FFFF, opacity: 1.0, offset_px: [0.0; 2], scale: [1.0; 2] }
    }
}
```

**Reason.** A derived `Default` gives `tint = 0` (transparent black), `opacity = 0`, `scale = [0,0]` —
an element that inserts a `UiVisual` and animates nothing becomes an invisible zero-sized node. This
is a two-line decision that costs one afternoon if it is discovered from a screenshot instead. The
`#[derive(Default)]` route must be **absent from the type**, not merely unused: see A1's red mutation
#3, and `boyko_render`'s own `default_mode_is_off` precedent — *two* routes into a default, neither
implying the other.

**And the absent case must agree with it:** a node with **no** `UiVisual` row folds by the identity,
which is arithmetically the same instance bytes as today. That equality is A4's disarmed gate.

### AD7 — the hit-test folds the same transform, behind an O(1) frame-level guard

`collect_candidates` (`focus.rs:204-257`) folds AD3's pair on its own DFS so a slid-in panel is
clickable **where it is drawn**. The per-node `UiVisual` probe is skipped wholesale when the frame
carries no visual at all:

```rust
let any_visual = world.dense_registry()
    .store(UiVisual::component_id())
    .is_some_and(|s| s.live_count() != 0);       // dense_store.rs:353, ecs_master.rs:590
```

**Reason.** Without the fold, an animating menu accepts clicks at its pre-animation position for the
whole animation — the class of bug that is reported as "the UI is haunted" and diagnosed in an
afternoon. With the fold and without the guard, this plan would add a probe per node per frame to the
pass D23 already names as the campaign's likely dominant cost, on every UI, including the 100 % of
UIs that animate nothing. `live_count()` is one load and makes that cost exactly zero.

**Rejected:** (a) *hit-test on `ComputedRect` only* — defensible for a 1.05 hover pop (2.5 % edge
error), indefensible for a −400 px slide; the fold cannot be per-channel; (b) *a `With<UiVisual>`
query probe per frame* — correct but pays an archetype walk where a `live_count()` load answers
exactly; (c) *stamp the folded rect into a component at gather time* — a second durable per-node
datum written by a render-side pass and read by an input-side pass, i.e. a cross-crate parallel data
system for a value that is four multiply-adds.

### AD8 — FLIP is a two-system idiom, not a component graph

D11's supported Tier-3 answer, made concrete: `ui_flip_capture` (exclusive, gated on a `FlipRequest`
scratch flag) snapshots `ComputedRect` for the marked entities into a retained buffer; the author's
layout write lands; layout runs **once**; `ui_flip_launch` (exclusive, `.after(ui_layout_apply)`)
reads the new `ComputedRect`, computes the `UiVisual` offset/scale that maps new onto old, inserts it
plus a `TweenOffset`/`TweenScale` to identity. Layout runs **twice** for an animation of any duration.

**Reason.** The endpoints are plain data — "record the first rect" is a component copy, not the
forced synchronous reflow the web pays (D11). Two exclusive systems reuse the crate's existing
discovery/apply cadence and introduce no component that outlives the launch frame.

**FLIP's precondition is restated as a rule, not a caveat** (D11): a channel that is *continuously
driven with no fixed endpoint* — a fling — is Tier-1/2 or it is not shipped. Any future Tier-3 channel
proposal must name its two endpoints.

---

## 3 · The rung ladder

**Unconditional gate on every rung:** `cargo clippy -p boyko-ui -p boyko-render --all-targets -- -D warnings`;
`cargo test -p boyko-ui -p boyko-render --all-targets --no-fail-fast`; Miri where new `unsafe` lands
(none is expected in this ladder — see §6 R3); every `// SAFETY:` present; author-only commit.
**`--no-fail-fast` is load-bearing** — one red target otherwise shadows every target behind it.

**Default OFF, and which rung turns something on.** Rungs **A0–A3** are structurally off: the plugin
is opt-in, no existing host adds it, and no component is inserted by anything that ships. **A4** is
the first rung whose code runs on every packed node, and it is *arithmetically* off — the identity
fold (AD6) reproduces today's bytes exactly, which is A4's disarmed gate. **No rung in this ladder
changes an existing golden byte.** The first *new* golden is authored by the observer rung (D32,
sprites plan) once it has an animated hover to show, which is after A3 and A4 have both landed.

---

### A0 — the UI clock — **size S** · *no cross-plan dependency*

**Lands.** `UiClock` (AD1) + `ui_clock_tick` + `UiAnimationPlugin` + the `UiAnimationSet` ordering set,
following `UiWidgetsPlugin`'s registration idiom verbatim (`widgets.rs:267-289`). Nothing visible.

**Gate.**
1. `dt_real == Time::real_delta().as_secs_f32()` exactly, for deltas below the clamp.
2. **Paused-clock leg:** `Time::pause()` ⇒ `dt_virtual == 0.0` **and** `dt_real > 0.0` on the same
   frame. This is D15's whole reason, as a test.
3. **Hitch leg (AM6):** a 2 000 ms real delta yields `dt_real == max_delta`, not 2.0.
4. Plugin containment: an app with and without `UiAnimationPlugin` has an identical registered
   schedule-label set and an identical resolved event policy — the particles plan's `D17` containment
   test, borrowed because it is the cheap way to prove a plugin adds no shared-schedule surface.

**RED MUTATION.** Swap the default in the tween row's clock select (`dt_real` → `dt_virtual`) ⇒ leg 2's
downstream assertion in A1 (a tween advances while `Time` is paused) reds. **And, at this rung
alone:** delete the clamp in `ui_clock_tick` ⇒ leg 3 reds. *Both must be run and the red observed
before the rung closes.*

---

### A1 — the sink, the four channels, the fused tick — **size L** · *no cross-plan dependency*

**Lands.** `UiVisual` (24 B, AM5, AD6) · `TweenTint` / `TweenOpacity` / `TweenOffset` / `TweenScale`,
all `#[component(storage = "dense")]` `#[repr(C)]` POD `Copy` · `ui_visual_tick` (AD5) ·
`ui_tween_reap` · `UiTweenScratch` (the retained completion list) · the public start/stop helpers
(`start_tween_tint(world, e, from, to, ms, easing)` and siblings) · `size_of`/`align_of`/`offset_of!`
const-asserts on all five types.

Easing is **linear only** at this rung — `EasingId` exists as a field and the tick applies `t` — so
that A1's gates test the machinery and A2's gates test the curves, and a red at A2 cannot be blamed
on A1.

**Gate.**
1. **Presence is running (C2/D9):** inserting `TweenTint` starts it; the reap removes it at
   completion; `DenseStore::live_count()` for each channel returns to **0** after the last tween ends.
2. **The tick bumps the sink's tick (AM1):** a system running after `ui_visual_tick` with
   `Query<(), Changed<UiVisual>>` sees the row on an animating frame **and does not** on the frame
   after the reap.
3. **A rested element is silent (AM2):** an entity whose tween has completed keeps its `UiVisual` row
   and, on every subsequent still frame, is **not** `Changed<UiVisual>`. Assert with a live-row count
   of ≥1 and a changed-row count of 0 — the two together are what "rested but retained" means.
4. **The identity default (AD6):** `UiVisual::default()` equals the hand-written identity, field by
   field, **and** `UiVisual` does not `#[derive(Default)]` (a compile-time absence, asserted the way
   `every_variant_states_its_own_answer_without_a_wildcard` asserts its own).
5. **Arity-one per channel (D9 reason 3):** starting a `TweenTint` on an entity that already has one
   overwrites rather than stacking; `live_count()` does not grow.
6. **Zero per-frame allocation** on the steady animating path — the crate's existing
   `zero_alloc.rs` / `p3_watch_zero_alloc.rs` harness shape, extended to the tick.
7. **Paused-clock leg (A0 leg 2, downstream):** with `Time` paused, a default-clock tween advances and
   a `virtual`-flagged tween does not.
8. **Miri** over the tick + reap, because dense insert/remove during a frame that also iterates the
   store is the one place this rung could be unsound.

**RED MUTATIONS — three, each must be run.**
1. `Mut<UiVisual>` → `&mut UiVisual` ⇒ **gate 2 reds.** *This is the amendment's proof; without seeing
   this red, AM1 is an assertion.*
2. Delete the all-`None` `continue` ⇒ **gate 3 reds** (the rested element becomes `Changed` every
   frame).
3. Replace the hand-written `Default` with `#[derive(Default)]` ⇒ **gate 4 reds** on both halves.

---

### A2 — easing — **size M** · *depends on A1*

**Lands.** `EasingId` (AD2) · the 30 built-in curves as monomorphic `fn(f32) -> f32` leaves selected by
one `match` per row (D12's shipping form for v1; the partition is deferred to A8) · `UiEasingTable`,
the `Resource`-owned LUT column for custom curves (D12), with the dense-handle discipline
`FontId` already sets (`components.rs:440`).

**Gate.**
1. **Endpoints:** `f(0.0) == 0.0 && f(1.0) == 1.0` for all 30, exactly.
2. **A midpoint oracle table** — `f(0.25) / f(0.5) / f(0.75)` for all 30, against values computed
   independently from the closed forms and written into the test as literals.
3. **Monotonicity** over 1 024 samples for the 21 non-overshooting curves; **bounded overshoot** for
   `back` (≤ 1.10 / ≥ −0.10) and `elastic`, and `bounce`'s known plateau count, for the other 9.
4. **`linear` × 3 ids ⇒ one body** (AD2), asserted by function-pointer identity.
5. **Custom half:** `EasingId(128)` resolves to LUT index 0; `EasingId(127)` is a built-in-range id and
   is rejected by the const-assert boundary; a LUT miss falls back to `linear` and is counted, never
   panics (the crate's `.keys` graceful-fallback discipline).
6. **No allocation and no `dyn`** on the evaluation path — the ban is mechanical via
   `clippy.toml`, but assert `size_of::<EasingId>() == 1`.

**RED MUTATION.** Swap `In` and `Out` for one family (e.g. `cubic`).

**Why the midpoint table is the gate and the endpoints are not.** Under that mutation
`f(0) == 0` and `f(1) == 1` **stay green for every family** — every easing curve in the set passes
through both endpoints regardless of direction. A gate built on endpoints alone would be structurally
incapable of seeing the most likely authoring error in the whole rung. Gate 2 reds; gate 1 does not.
*The red must be run against gate 1 as well, and its staying green recorded* — that is the observation
this rung is really pinning.

---

### A3 — `Interaction` transitions — **size M** · *depends on A1, A2*

**Lands.** `UiStateTint` (D14's array-indexed-by-enum, 3 states) · `ui_transition_discovery` (normal
system, `Query<(), Changed<Interaction>>` → a `dirty` bool + `last_run` in `UiTransitionScratch`) ·
`ui_transition_apply` (**exclusive**, the `ui_bar_apply` pattern verbatim: `query_entities_buf` into
retained scratch refreshed on a set change, per-entity
`get_component_changed_tick(e, Interaction::component_id())` +
`is_newer_than(last_run, this_run)`, read the running `TweenTint` if present, insert-or-overwrite
**immediately**) · the reversing shortening factor · ordering:
`.after_set(GameplaySet)` so `ui_focus_system`'s `Interaction` write is visible the same frame, and
`ui_visual_tick` `.after(ui_transition_apply)` so a fresh tween ticks its first delta the same frame.

**Gate.**
1. **No lost first frame:** hover-enter on frame *N* ⇒ frame *N* renders the `from` value and frame
   *N+1* renders a strictly-moved value. The failure this catches is one frame of nothing at the head
   of every transition.
2. **The reversing factor:** enter, run to 50 % of the duration, leave ⇒ the return tween's duration is
   50 % ± one frame of the configured duration, read from the row's `elapsed`/`inv_duration`.
3. **The 0 %-gate:** a still frame leaves `dirty == false`, apply early-returns, and the exclusive pass
   performs **zero** `query_entities_buf` calls and zero allocations.
4. **Per-row granularity:** with 100 `UiStateTint` nodes and one hovered, exactly one `TweenTint` row
   is inserted.
5. **`Disabled` is not a fourth state:** the state array stays `[u32; 3]`, asserted by `size_of`, and
   a `Focusable`-disabled node produces no `Interaction` edge (the capability rule; the disabled
   *appearance* is the interaction plan's separate component).

**RED MUTATIONS — two.**
1. Replace the exclusive apply with `Option<&mut TweenTint>` + a `Commands` insert — **the shape the
   architecture's revision-1 record specified** ⇒ **gates 1 and 2 both red** (the start lands a frame
   late, and the reversal finds no row to read `elapsed` from). *This red is the proof of D14's own
   correction and must be run.*
2. Delete the shortening factor ⇒ gate 2 reds.

---

### A4 — the pack fold — **size M** · *depends on A1; **depends on the sprites plan's seam rung** (D31 gather + D6 gate)*

**Lands.** `PackInput` gains the folded visual inputs · `pack_ui_instance` applies AD3's affine to
`min_px`/`size_px`, multiplies `tint_mul` into `color` in straight space before the premultiply, and
folds `opacity` into the premultiply · `gather_ui_nodes` reads `UiVisual` and supplies it for the
node's background quad **and for every glyph quad the node emits** (AM3) · `impl Default for PackInput`
and the conversion of the **11** construction literals in `crates/boyko_render/tests/` to
`..Default::default()` tail form.

> **The `Default` conversion is a shared prerequisite, not this rung's property.** The sprites plan's
> pack rung adds its own `PackInput` fields and hits the same 11 sites. **Whichever of the two lands
> first owns the conversion; the other inherits it and adds fields at zero sites.** If the sprites
> plan lands first, delete this bullet from A4 rather than doing it twice.

**Gate.**
1. **DISARMED BYTE-IDENTITY — the rung's headline.** Every existing `ui_pack_cpu.rs` case produces a
   **byte-identical `UiInstance`** after the fold lands, because no fixture carries a `UiVisual` and
   the absent case is the identity (AD6). This corpus is device-independent and always runs, which
   makes it the real gate; the GPU goldens (`ui_rect_gpu_golden`, `ui_text_gpu_golden`,
   `ui_rect_swapchain_golden`, `ui_text_multiscale_gpu_golden`) **skip gracefully on a device-less
   host** (`ui_rect_gpu_golden.rs:36-39`) and must therefore be **run locally with a device and the
   result reported in the commit**, the discipline the particles plan applies to `*_spv_sync`.
2. **Armed identity:** a node carrying `UiVisual::default()` produces the same bytes as a node
   carrying none. Two routes to the same instance, neither implying the other.
3. **Centre origin (AM3/AD3):** a node at `(100,100,50,50)` with `scale = [2,2]` packs to
   `min = (75,75)`, `size = (100,100)` — centre preserved. A top-left fold gives `min = (100,100)`.
4. **Glyph sub-quads (AM3):** a label with three glyphs at `scale = [2,2]` produces three quads whose
   **spacing** doubles, not merely their size. Assert the gap between quad 0's right edge and quad 1's
   left edge.
5. **Opacity rides the premultiply:** `opacity = 0.5` on an opaque red gives the same instance bytes
   as an authored `alpha = 128` red, exactly (this is the property that makes opacity cost zero GPU
   bytes, so it is worth pinning rather than assuming).
6. **Tint multiplies in straight space, before premultiply**, not after — a specific ordering that is
   invisible on opaque colours and wrong on translucent ones. Assert with `alpha = 128`.

**RED MUTATIONS — three.**
1. Make the absent case fold `scale = [0,0]` instead of `[1,1]` ⇒ **gate 1 reds across the whole
   corpus.** *This is the disarmed proof: it demonstrates the byte-identity gate can fail, which is
   the only thing that makes its green mean anything.*
2. Fold about the top-left ⇒ gate 3 reds, gate 1 stays green.
3. Apply `size *= scale` per glyph without the origin-relative translate ⇒ gate 4 reds, gate 3 stays
   green.

---

### A5 — inheritance on the gather's DFS — **size M** · *depends on A4*

**Lands.** The `(S, T, opacity, tint)` accumulator on the gather's existing DFS stack (AD3/AD4) · the
inherited-clip transform · the `UiVisual` probe hoisted so a subtree with no visual anywhere pays the
stack push and nothing else.

**Gate.**
1. **Offset inherits:** a parent at `offset = [10, 0]` moves its child's packed `min` by 10, in
   addition to whatever the child's own offset is.
2. **Scale composes about each node's own centre:** parent `scale = [2,2]`, child `scale = [2,2]` ⇒ the
   child's packed size is 4×, and its position is where the composition puts it — asserted against a
   hand-computed expected value, not against the implementation.
3. **Opacity multiplies:** parent 0.5 × child 0.5 ⇒ 0.25.
4. **The clip follows the transform:** a parent with a clip rect and `offset = [0, 50]` clips its
   children at the **moved** rect. This is the one leg that is invisible until a real panel animates
   with real overflow.
5. **Zero-visual subtree costs nothing extra** — the probe counter (§4/M3) is unchanged from A4's
   baseline for a tree with no `UiVisual` anywhere.

**RED MUTATION.** Drop the clip from the accumulated transform (leave `clip` as the raw inherited
value) ⇒ **gate 4 reds; gates 1–3 stay green.** *That asymmetry is the point: the three obvious legs
cannot see the defect this rung is most likely to ship.*

---

### A6 — the hit-test fold — **size S** · *depends on A5; **coordinates with the interaction plan***

> **Coordination, stated because it is a merge hazard, not a design one.** This rung edits
> `focus.rs::collect_candidates`, which `docs/UI-PLAN-INTERACTION.md`'s capture/routing rungs (D16/D17)
> restructure. **A6 lands either strictly before that plan's spine rung or strictly after it, never
> concurrently.** If interaction's spine lands first, A6's fold goes into the restructured DFS and this
> rung shrinks; if A6 lands first, the interaction plan inherits a stack tuple with one more member.
> Either order is fine; the overlap is not.

**Lands.** AD7's fold and its `live_count()` guard.

**Gate.**
1. **Clicks follow the pixels:** a panel at `offset = [-400, 0]` hit-tests at its drawn position, not
   at its laid-out one.
2. **Scale narrows the target correspondingly** — a node at `scale = [0.5, 0.5]` rejects a point that
   is inside its laid-out rect and outside its drawn one.
3. **Zero-cost when nothing animates:** with no `UiVisual` row in the world, the focus pass performs
   **zero** additional `get_component` probes against A5's baseline (§4/M3's counter).
4. **Guard correctness at the boundary:** insert one `UiVisual` anywhere ⇒ the guard flips and the
   fold runs for the whole tree (the guard is global by design; assert it, so nobody later "optimises"
   it into a per-subtree test that is wrong).

**RED MUTATIONS — two.** (1) Delete the fold ⇒ gate 1 reds. (2) Delete the `live_count()` guard ⇒
gate 3 reds while gates 1, 2 and 4 stay green.

---

### A7 — FLIP and the Tier-3 measurement — **size M** · *depends on A1, A4*

**Lands.** AD8's `ui_flip_capture` / `ui_flip_launch` + the `FlipRequest` scratch · and the
**instrument this campaign has owed since D11 was written**: `LayoutScratch::relayout_count` promoted
from `#[cfg(test)]` (`resources.rs:268-271`) to an always-compiled diagnostic counter, which is
§10.4's named instrument and today **does not exist in a release build**.

**Gate — §10.4's animation legs, reported as numbers, with one pass/fail among them.**

| Leg | Reported | Pass/fail |
|---|---|---|
| Static frame | roots re-laid-out per frame | must be **0** |
| One Tier-1 animation (a tint tween) | roots re-laid-out per frame | must be **0** — *this is D10's structural claim, as a measurement* |
| One Tier-2 animation (an offset tween) | roots re-laid-out per frame | must be **0** |
| One Tier-3 animation (a raw `UiLayout.width` tween) | roots re-laid-out per frame | **reported, not fixed** — expected `= root count`, every frame, for the duration |
| The same size change via FLIP | **total** roots re-laid-out over the whole animation | must be **exactly 2 × root count** |

The Tier-3 row is deliberately not a gate: D11 allows it and documents it as expensive, and turning
its number into a failure would be this plan quietly adopting the per-root granularity fix D11
defers.

**RED MUTATIONS — two.** (1) Give `ui_flip_launch` an identity inverse (skip the invert step) ⇒ the
"the element does not jump on the launch frame" assertion reds, and the FLIP relayout count stays 2 —
*so the count alone is not the gate, and the rung says so*. (2) Make `TweenOffset` write `UiLayout`
instead of `UiVisual` ⇒ the Tier-2 leg reds. **That second mutation is the only executable proof that
D10's "structurally impossible" claim is real**, and it is worth running once precisely because the
claim is that it should not compile — if it does compile, D10's enforcement is documentary and this
rung has found it.

---

### A8 — the measurement rung: what the tick actually costs — **size S** · *depends on A1–A5*

**Lands.** `crates/boyko_ui/benches/ui_animation.rs` + its `[[bench]]` entry in `Cargo.toml` —
**`boyko_ui` has no `benches/` directory today**, so this rung creates one · and the recorded numbers,
written back **into this document**, not into a commit message.

> **The L10 lesson is the reason this is a rung and not a footnote.** Eleven diagnostics ladder rungs
> reported themselves gated against a table of **twelve benchmarks, none of which existed**. A bench
> named in a plan is not a measurement. This rung closes only when the binary builds, runs, and its
> output is pasted below.

**The axes — and the second one is AM2's correction.**

| Axis | Values | Why |
|---|---|---|
| Animating rows | 8 / 64 / 512 | §10.5 as specified |
| **Resting rows** (`UiVisual` present, all channels absent) | **0 / 512 / 4096** | AM2 — the cost the specified bench is structurally blind to |
| **Bystander rows** (same archetype, no `UiVisual`) | **0 / 4096** | AM2's seed-per-archetype term (`iter.rs:550-559`) |

**Gate.**
1. The bench builds and runs; the numbers land in §4's table below with the machine and toolchain
   recorded.
2. **D12's fork is decided by the number, not by argument.** The easing partition ships **iff** the
   built-in `match` is a material fraction of the tick at 512 animating rows. If it is not, the
   partition is **deleted from the deferred list with its measurement cited**, not left as a standing
   TODO. Shipping it unmeasured would be the "arithmetic instead of a measurement" failure D23 refuses
   elsewhere.
3. **The all-`None` early-out is priced:** the `resting = 4096, animating = 8` cell is run with and
   without the `continue`, and both numbers are recorded. This is the second, quantitative proof of
   A1's red mutation #2.

**RED MUTATION.** Not applicable in the usual sense — a bench has no green to falsify. **The
substitute gate:** the bench must, on first run, show a **monotone** increase with the resting axis.
If resting rows are free, the instrument is not measuring what AM2 says it measures and the bench
itself is wrong — the particles campaign's "the instrument cannot see its subject" check, applied
before the number is believed.

---

## 4 · Measurement obligations

Only what this plan owes. §10.1/§10.2/§10.3/§10.6/§10.7/§10.8/§10.9 belong to the sprites and
interaction plans.

| # | Claim under test | Instrument | Discriminating comparison | Rung |
|---|---|---|---|---|
| **M1** | §10.4 — the tier table is real, and FLIP bounds Tier-3 | `LayoutScratch::relayout_count`, promoted out of `#[cfg(test)]` | The five legs in A7's table. Tier-1/2 **must be 0**; FLIP **must be exactly 2 × roots**; raw Tier-3 is reported | A7 |
| **M2** | §10.5 — the tween tick cost, **on the axis AM2 found** | criterion, `boyko_ui/benches/ui_animation.rs` | animating × resting × bystander, per A8's table; and the with/without-`continue` pair at `(8, 4096, 0)` | A8 |
| **M3** | AD7 — the hit-test fold costs zero when nothing animates | the probe counter the interaction plan builds for §10.8 (`get_component` calls per node per frame in the focus pass) | Same tree, `UiVisual` live-count 0 vs 1. The zero case **must be unchanged** from the pre-A6 baseline | A6 |
| **M4** | AM6 — the UI clamp is doing something | `dt_real` at a synthetic 2 000 ms delta | Clamped vs unclamped, and the resulting tween `elapsed` delta | A0 |

**M2's results table** *(to be filled by A8 — empty until then, and the rung does not close while it
is empty)*:

| animating | resting | bystanders | ns/frame | ns/frame, no early-out |
|---|---|---|---|---|
| 8 | 0 | 0 | — | — |
| 8 | 4096 | 0 | — | — |
| 64 | 512 | 4096 | — | — |
| 512 | 4096 | 4096 | — | — |

---

## 5 · Mandatory tests, invariants and benches

**Unit.** `ui_visual_default_is_the_identity` + `ui_visual_does_not_derive_default` (two routes,
neither implying the other) · `size_of`/`align_of`/`offset_of!` pins on `UiVisual` and all four
`Tween*` · `easing_endpoints_are_exact` (all 30) · `easing_midpoint_oracle` (all 30) ·
`easing_monotone_where_it_should_be` / `easing_overshoot_is_bounded` · `linear_three_ids_one_body` ·
`easing_custom_boundary_is_const_asserted` · `state_tint_array_is_three` ·
`clock_paused_advances_real_not_virtual` · `clock_clamps_a_hitch` ·
`plugin_changes_no_schedule_label_set`.

**Property.** Over random `(from, to, duration, easing, dt-sequence)`: the tween's value is **bounded
by `[min(from,to), max(from,to)]` for the 21 non-overshooting curves**; the value at `elapsed >=
duration` is **exactly `to`** (not `to ± ULP` — the endpoint is assigned, not interpolated, and that is
a decision worth a property); a row is removed exactly once; `live_count()` returns to its starting
value after every sequence. Over random trees: AD3's composition is associative
(`(A∘B)∘C == A∘(B∘C)`) to within one ULP, so the DFS fold order cannot matter.

**`debug_assert!`.** `inv_duration.is_finite() && inv_duration > 0.0` at insert (a zero duration is
the reciprocal-of-zero trap the `inv_duration` field creates) · `opacity` within `[0, 1]` at fold
time · `scale` finite and non-negative · `elapsed >= 0.0` · the fold's output `min_px`/`size_px`
finite before the instance write (mirroring `pack_ui_instance`'s existing finite assert).

**Benches.** `ui_animation.rs`: the tick over A8's three axes; the fold arithmetic in isolation
(`pack_ui_instance` with and without a visual); the transition apply at 100 / 1 000 `UiStateTint`
nodes with one hovered.

---

## 6 · Risks

**R1 — A4 depends on a rung this plan does not own, and that rung is where the campaign's observability
lives.** A0–A3 land without the seam and are fully gated by CPU tests inside `boyko_ui`, which is why
the ladder is ordered this way. But **nothing in A0–A3 is visible on a screen**, and A4 cannot land
until the sprites plan has shipped D31's gather and D6's gate. If that rung slips, this plan
accumulates four landed rungs whose only proof is unit tests — the exact state the architecture's R1
and R2 call the campaign's top two risks. *Mitigation:* A0–A3's gates are written so that each one
would red under a real defect (§3's red mutations), not so that they merely pass; and A4's headline
gate is a **disarmed byte-identity** over the always-running CPU corpus, which is the strongest
observation available without a host.

**R2 — AM1 and AM2 are corrections to a document that has already been reviewed once.** Both are
derived from reading the kernel, and both are the kind of claim that is easy to state and hard to
disprove from the outside. *Mitigation:* neither is accepted on my reading. A1's red mutations #1 and
#2 **are** the proofs — they convert both amendments from arguments into observed reds, and if either
red does not appear, the amendment is wrong and this plan is wrong with it. That is the intended
failure mode.

**R3 — this ladder writes no `unsafe`, which is a claim, not a fact, until A1 lands.** The tick is
safe-Rust over a `Query`; the reap is an exclusive system using `EcsMaster`'s safe API; the fold is
arithmetic. If an implementer finds a place that seems to need `unsafe`, that is a signal the shape
has drifted from AD5, and it escalates rather than getting a `// SAFETY:` comment written for it.

**R4 — `Or` filter arity caps at 12, and D6b's macro has no bound.** Verified: in-range
`impl_or_filter_tuple!` invocations stop at 12 (`filter.rs:2029-2057`); arity 13–24 are
`impl_or_filter_tuple_too_large!` stubs whose bodies **panic** (`filter.rs:2161-2180`, message
*"Or\<F\> has too many QueryFilter elements. boyko-engine supports up to arity 12. Split your …"*).
The stub **type-checks**, so an oversized `Or` compiles cleanly and dies at first-frame
`init_state` — not at build time.

~~`ui_render_discovery`'s pack-input set (D31 point 2) already names `ComputedRect`, `UiBackground`,
`ComputedClip`, `StackIndex`, `UiText`, `UiVisual`, `UiImage`, `UiNineSlice`, `UiSpriteSheet`,
`UiSpriteCursor`, `ScrollPosition`, `Children` — **twelve, before this plan's channels or the
interaction plan's components are counted.**~~ `Or` **is** `OrComposable`, so nesting is legal
(`filter.rs:2420-2424`), and the fix is either `Or<(Or<(…)>, Or<(…)>)>` or N discovery systems ORing
into one scratch bool.

**The projection is corrected, and the ceiling is further off than R4 feared** *(2026-08-21 at the S5
pre-build audit — `UI-PLAN-SPRITES.md` **S-D16 (3)** and its §6 exposure row)*. The landed
`__ui_pack_inputs_list!` holds **seven**: `ComputedRect`, `UiBackground`, `ComputedClip`,
`StackIndex`, `UiImage`, `UiNineSlice`, `UiSpriteSheet` (`crates/boyko_render/src/ui/gather.rs:96`).
*(Was "six" at `:76-86` — written before S5 and invalidated by the rung it was written for. This
document is not in the anchors census's `GATED_DOCS`, so nothing reddened.)* `UiText` and `Children` are
**not** members — `Children` is a separate traversal probe the gather pays outside the macro, and
`UiText` is not in the list at all — so two of R4's twelve were never there. The sprites plan's S5
adds **one**, not two: `UiSpriteSheet` alone, because `UiSpriteAnim` is author configuration the pack
never reads and `UiSpriteCursor` is the flipbook's private state. **The flat arity therefore runs
6 → 7 (S5) → 8 (`UiVisual`) → 9 (the interaction plan's scroll datum), against a ceiling of 12.**

**R4's real finding survives and gets sharper, though — and it is now a HARD constraint rather than a
budget worry.** ⚠️ A **dense** `Changed<C>` inside `Or<..>` can never be true, MEASURED on this tree:
the `Or` `QueryFilter` impl overrides none of the dense hooks (`HAS_DENSE`, `HAS_DENSE_INCLUDE`,
`resolve_dense`, `dense_include_candidates` — `filter.rs:1834-2030`), so the inner term's store
pointer stays the `init_fetch` NULL and `filter_fetch` returns `false` on its first line
(`filter.rs:1483-1484`), while the same `Changed<C>` used BARE observes the row. **`UiVisual` must
therefore be a table component if this plan intends `Changed<UiVisual>` to drive the repack**, or
each tween system bumps `UiRenderGeneration` at its own writer. The mitigation R4 hands to the seam
rung — *"the D31 macro must emit the nested form unconditionally, and the seam rung needs a test that
RUNS the discovery system"* — is still owed and is still correct; a runtime test would also have
caught the dense hole, which a type-level test cannot.

**This risk is recorded here because animation is the subsystem that discovers it** — `UiVisual` is
the thirteenth term — but **the fix belongs to whichever rung ships D6b** (the sprites plan's seam
rung). *Mitigation to hand over:* the D31 macro must emit the nested form unconditionally, and the
seam rung needs a test that **runs** the discovery system, not one that merely names its type — a
type-level test cannot see a panic that lives in `init_state`.

---

## 7 · Open questions for the owner (VALUES / SCOPE — also to be filed in `docs/OPEN-QUESTIONS.md`)

1. **The UI hitch clamp (AD1/AM6).** 100 ms is proposed and unmeasured. It trades "a transition
   survives a stall" against "a transition falls behind a slow frame". Accept 100 ms, or name another?
2. **Group opacity (AD4).** Multiplicative per-element opacity is what v1 ships. CSS group opacity —
   overlapping children not darkening each other — needs an offscreen layer and therefore a second
   pass, which is outside the one-draw batcher. Accept the difference permanently, or is it a later
   campaign?
3. **Springs (D13), reconfirmed against this ladder.** A3's reversing factor is the tween-domain answer
   to retargeting. Is there a named v1 feature that needs *velocity* continuity? If not, springs stay
   deferred and this plan does not build the column.
4. **The transform pivot (AD3 rejected (b)).** v1 fixes the pivot at the rect centre. An authored
   per-node pivot is one `[f32; 2]` on `UiVisual` (24 B → 32 B). Wanted now, or when something asks?

---

## 8 · Dependencies, stated explicitly

| Depends on | From | Needed by | If it does not land |
|---|---|---|---|
| **D31 — `boyko_render::ui::gather_ui_nodes`** and the macro-spelled pack-input set | `docs/UI-PLAN-SPRITES.md` (seam rung) | **A4, A5** | A0–A3 still land; nothing animates on screen (R1) |
| **D6 / D6b — the per-slot generation gate + `ui_render_discovery` as its single bump site** | `docs/UI-PLAN-SPRITES.md` (seam rung) | **A4** | AM1's `Mut<UiVisual>` write has nothing watching it; animation is invisible even after A4 |
| **The `Or` arity fix in D6b's macro (R4)** | `docs/UI-PLAN-SPRITES.md` (seam rung) | **A4** | First-frame panic once `UiVisual` becomes the 13th term |
| **`impl Default for PackInput` + the 11 literal conversions** | whichever of A4 / sprites' pack rung lands **first** | **A4** | Both plans churn the same 11 test sites |
| **D7 — the `.ui` registration table** | `docs/UI-PLAN-SPRITES.md` (rung 1) or wherever it is sequenced | **A3** (only for authoring `UiStateTint` in `.ui`) | A3 lands with `UiStateTint` inserted from Rust only; the `.ui` spelling follows |
| **`collect_candidates`' post-D16/D17 shape** | `docs/UI-PLAN-INTERACTION.md` | **A6** | Merge conflict, not a design gap — see A6's coordination note |
| **The §10.8 probe counter** | `docs/UI-PLAN-INTERACTION.md` (D23) | **M3** | M3 falls back to a wall-clock A/B, which is weaker |

**What this plan exposes, that others depend on:**

| Exposed | Consumed by |
|---|---|
| `UiClock` — the one UI delta source, clamped, real/virtual (AD1) | Sprites plan (the `UiSpriteCursor` flipbook); interaction plan (`ScrollMomentum`, `HoverDwell`). **Fallback, recorded because this row previously implied there was none:** if this plan lands second, the sprites plan's `ui_sprite_flipbook` takes `Res<Time>` and applies **AM6's clamp itself at that one site**, using AD1's own `100 ms` as `UI_FALLBACK_MAX_DELTA`; the replacement swaps one `SystemParam` and deletes one `min`. It does **not** adopt the raw `real_delta()` AD1 rejects. *(`UI-PLAN-SPRITES.md` **S-D17**, 2026-08-21 — the sprites plan had named the rejected option as its fallback, so the dependency read as satisfied here and as waived there.)* |
| `UiVisual` — the Tier-1/2 sink, and its **identity** default (AD6) | Sprites plan (the pack fold reads it; the sprite lane's future `uv_shift`, AM5) |
| AD3's affine + AD4's composition rule | Sprites plan (nine-slice sub-quads must use the origin-relative form, AM3); interaction plan (A6's hit-test fold) |
| `EasingId` + the built-in family + the LUT table (AD2) | **Aether plan** — the `ui` construct's easing spelling is `family-direction` name → `EasingId`, a closed 30-name set plus a custom handle |
| `UiStateTint`'s field list and its 3-state array (D14/A3) | **Aether plan** — the `on hover:` / `on press:` styling surface lowers to this component and nothing else |
| The tier table's animation rows (D10) | **Aether plan** — an authorable channel must have a tier row, or it is a defect by construction (D10) |

**The Aether dependency, named as the task requires.** `docs/UI-PLAN-AETHER.md` is sequenced **last**
in the architecture (§11 rung 8) precisely because it can only name what exists. From this plan it
needs exactly three stable surfaces — `EasingId`'s name set, `UiStateTint`'s field list, and the tier
table — and **all three are frozen by the end of A3.** A4–A8 change no authorable spelling. So the
Aether construct's animation half is unblocked after **A3**, not after A8.

---

## 9 · Deferred, each with its reason

| Deferred | Why the line is here | Shape recorded |
|---|---|---|
| **Springs** (D13) | Only distinctive benefit is velocity-preserving retargeting; A3's reversing factor answers the same UX problem. §7 Q3 | Separate dense column, Juckett closed form, **mandatory** rest test |
| **The easing partition** (D12) | Ships **iff** A8's number says the `match` is material — decided by measurement, deleted if not | Partition-by-`EasingId` within each channel arm, the particles P2 pattern |
| **Timeline clips + players** (§5.8) | Nothing in v1 needs multi-channel keyframes | Channel-major `Resource` clip table + a 16 B per-element cursor; group-key sequencing, never a nested graph |
| **Per-root layout dirty bits** (D11) | With nothing animating the current design is provably free, and FLIP removes the steady-state case. A7's M1 is the number that would reopen it | `UiRootIndex(u8)` + `Query<&UiRootIndex, Or<…>>`; `dirty: bool` → `dirty_roots: u64` |
| **`TweenUvShift` / `UiVisual.uv_shift`** (AM5) | No v1 channel writes it; a field with no writer is a dead datum | Tier-1 dense column + two `f32` on `UiVisual`, after the sprites plan's `uv` field exists |
| **Group opacity** (AD4) | Needs an offscreen layer ⇒ a second pass ⇒ outside the one-draw batcher. §7 Q2 | — |
| **Authored transform pivot** (AD3) | One `[f32;2]`, 24 B → 32 B, for a case nothing asks for. §7 Q4 | — |
| **Rotation** (D5) | Invalidates the per-instance AABB clip **and** the axis-aligned hit-test **and** needs a `.spv` re-bless — three subsystems for an effect nothing asked for | D5's recorded shape |
