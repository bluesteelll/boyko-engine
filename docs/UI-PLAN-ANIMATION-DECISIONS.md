> **Part of [UI-PLAN-ANIMATION.md](UI-PLAN-ANIMATION.md)** — §1 amendments AM1–AM8, §2 decisions AD1–AD13.

## 1 · Amendments to the architecture

~~Six~~ ~~Seven~~ **Eight** statements in the authority did not survive a read of the kernel — or, for
**AM7**, a read of the **shipped tree**, which is the same failure through a later door: the
architecture named a default, three consumers were built, and none of them took it. They are
corrected here at their source with the fact that forced each, per this project's standing rule that a diverged pair is
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

~~**The cost model, corrected.** `Mut<UiVisual>` is a dense include, so candidates are seeded from the
store's `arch_presence` (`state.rs:126-137`, `dense_store.rs:377`) — but seeding is **per archetype**,
and within a seeded archetype **every row is visited** and rejected by `dense_row_passes`
(`iter.rs:550-559`). Animated and un-animated panels share an archetype in any real UI. So:~~

> ~~per frame ≈ (rows in archetypes hosting ≥1 `UiVisual`) × 1 sparse-map probe~~
> ~~+ (`UiVisual` rows) × 4 further probes (one per `AnyOf` arm, inside `fetch`).~~

**Struck 2026-08-27 at the A1 pre-build audit — the whole formula rests on a storage kind AM8 has
now refused.** The paragraph reads correctly *for a dense sink*, and a dense sink is exactly what
makes every animation invisible (**AM8**, MEASURED). Under the table sink AM8 mandates, `Mut<UiVisual>`
contributes an **include bit** — the dense early-return in `aggregate_include` is not taken
(`data/mut_.rs:250-256`) — so only archetypes that *carry* `UiVisual` are candidates and there is no
per-archetype bystander term at all. **The corrected model:**

> per frame ≈ (`UiVisual` rows) × 4 dense `slot_of` probes (one per `AnyOf` arm, inside `fetch`).
> The sink itself costs a **column offset**, not a probe.

MEASURED (A1 pre-build audit, leg L3b): a world of 2 sink rows plus 2 bystanders carrying only the
shared table marker — which under a *dense* sink would sit in the **same** archetype and be walked —
yields **2** visits under a table sink, because the bystanders are in a different archetype and are
never seeded. **Consequence for §4/A8: the bystander axis is structurally empty and is struck there
too.** Consequence (1) above — the all-`None` `continue` — **survives unchanged**: MEASURED, a rested
row is still visited and still yields `(None, None, None, None)` under a table sink (leg L3b, 2 visits
over 2 channel-less rows).

That is the number §4/M2 must report, and it is not the number the architecture's §10.5 asks for.

### AM3 — D5's fold formula is correct only for a node's own background quad

D5 states the fold as `min_px = (rect.xy + visual.offset) * s` / `size_px = (rect.wh * visual.scale) * s`.

Two defects:

* **A node emits more than one quad.** Glyphs go through the same seam —
  `PackInput { text_uv: Some(uv), .. }` (`boyko_ui/src/text/emit.rs:25`), and the sprites plan's
  nine-slice adds nine more (D8d) *(and takes the image record away in exchange, so a
  nine-sliced imaged node is 10 quads rather than 11 — [`UI-PLAN-SPRITES-DECISIONS.md` **S-D12 (1)**](UI-PLAN-SPRITES-DECISIONS.md#1-uinineslice-suppresses-the-sub-10-image-record--the-slices-are-the-image-sliced), 2026-08-21;
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

`Time::delta_secs()` is a cached `f32` of the *virtual* delta (~~`time.rs:64-70`~~ **`time.rs:67-71`**);
the real side offers only `real_delta() -> Duration` (~~`time.rs:80-83`~~ **`time.rs:79-83`** — the
struck range started one line below the sentence it quotes). And `DEFAULT_MAX_DELTA = 250 ms`
(`time.rs:23`) clamps the **virtual** delta only — *"Raw delta of the current frame — unclamped,
unscaled, pause-blind."*

D15 makes the real delta the UI default. Taken literally, an alt-tab stall or a shader-compile hitch
delivers a 2-second real delta and **every running transition jumps straight to its end**, which reads
as a visual glitch on resume, not as an animation. **`UiClock` therefore applies its own clamp** — see
**AD1**. No kernel change; one `as_secs_f32()` per frame, not per row.

**AM6 SURVIVES S5, re-verified 2026-08-26 at the A0 pre-build audit** *(recorded because an amendment
carrying a decision's authority over a fact that has changed is worse than no amendment, and S5
landed a clock-shaped thing that could plausibly have overtaken this one)*. `time/time.rs` is
untouched since `d8f53863` and both halves hold **now**, not only when AM6 was written:

* **No cached real-delta `f32`.** The struct carries `delta_secs: f32` (`time.rs:43`) — the *virtual*
  cache — and `real_delta: Duration` (`time.rs:47`). There is no `real_delta_secs` field and no
  `real_delta_secs()` method.
* **The real delta is unclamped, and the ordering is visible in one function.** `advance_with`
  assigns `self.real_delta = raw;` at `time.rs:197` and only *then* computes
  `let clamped = raw.min(self.max_delta);` at **`time.rs:200`**. *([`UI-PLAN-SPRITES-DECISIONS.md` S-D17](UI-PLAN-SPRITES-DECISIONS.md#s-d17--s5-carries-am6s-clamp-itself-and-the-seam-is-a-resource-read-not-a-world-call) and
  `:3018` cite this clamp as `:201`; the assignment cite `:197` is exact, the clamp cite is one line
  low. Cosmetic, and it is inside the 96-stale-anchor backlog already filed in
  `docs/OPEN-QUESTIONS.md`.)*

**What S5 landed is NOT this clock**, and the distinction is the whole of A0's remainder: it is a
`pub const UI_FALLBACK_MAX_DELTA: f32 = 0.1` (`crates/boyko_ui/src/sprite.rs:320`) plus one inline
`min` at one call site, inside `ui_sprite_flipbook` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*. No resource, no `SystemParam` wrapper, no system, no second
delta. The sprites plan calls it a fallback and says so at its own site.

### AM7 — D15's real-delta default is a **tween-lane** rule; every consumer without a per-row `flags` bit has already chosen virtual, twice in shipped code

*(added 2026-08-26 at the A0 pre-build audit.)*

D15 (`UI-ADVANCED-ARCHITECTURE.md:1159-1171`) says *"the UI clock is `Time`'s **real** delta by
default, virtual opt-in per row"*, and its reason is sharp and **correct for the case it names**: *"a
pause menu that fades in on a paused virtual clock never fades"*. That is a **tween** on a pause
menu, and D15's own opt-in mechanism is *"a `flags` bit per tween row"*.

**What did not survive:** the word *default*, applied outside the tween lane. Three consumers have
now been decided, and **all three chose the virtual delta** — two of them are shipped, gated code,
and the third is a sibling plan asking this one to confirm:

1. **`ui_sprite_flipbook` — SHIPPED (S5).** `let dt = time.delta_secs().min(UI_FALLBACK_MAX_DELTA);`
   *(the pre-A0b inline `min`, DELETED by A0b and deliberately unanchored — a coordinate into
   deleted state resolves to whatever live line now occupies it)*, with the refusal recorded in the
   source at that system's clock paragraph *(pre-A0b, unanchored for the same reason; rewritten to
   AD9, and now its `# The clock (A0b …)` doc section, `sprite.rs:329-349`)*: `real_delta()`
   is *"unclamped, unscaled, pause-blind — three defects, and a `min` fixes only the first: a paused
   game would keep animating and `set_relative_speed` would do nothing."* It is gated three ways by
   `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`
   (`crates/boyko_render/tests/ui_s5_sprite_sheet.rs:534`) — CLAMPED, PAUSE-AWARE, SCALED.
2. **`ParticleClock` — SHIPPED, a second subsystem clock reaching the same answer independently.**
   It advances from `Time::delta_secs()` *"which the engine has already clamped to `Time::max_delta`,
   scaled by `relative_speed`, and zeroed while paused — so pausing the game pauses particles and
   slow-motion slows them, both free"* (`crates/boyko_render/src/particle_clock.rs:4-8`).
3. **`ScrollMomentum` and the dwell timer — the interaction plan, planned virtual, and it asks.**
   `UI-PLAN-INTERACTION.md:453` picks `Time::delta_secs()` *"already `min(raw, max_delta)`-clamped,
   speed-scaled and pause-aware … A fling therefore pauses with the game and slows in slow motion,
   for free"*; `:967` says *"The dwell clock is `Time::delta_secs()`, so a paused game does not
   accumulate dwell."* Its dependency table (`:490`) states the ask outright: *"If the animation plan
   makes the UI clock virtual-by-default, momentum follows that one answer."*

`dt_real` therefore has **zero** consumers, shipped or planned, outside the tween row that has not
been built yet. **Correction: `dt_real` is the default of the lane D15 was reasoning about — the one
with the `flags` bit — and `dt_virtual` is the default everywhere else.** Both fields stay; AD1's
struct is unchanged; what changes is the sentence that says which one an unflagged consumer reads.
See **AD9**.

*(A defect found in passing, belonging to the interaction plan: `UI-PLAN-INTERACTION.md:439`'s
heading reads **"ID12 — momentum is UIKit's frame-rate-independent form, on `Time`'s real delta"**
while its own body six lines down picks `Time::delta_secs()`, the virtual one. A diverged pair inside
one decision, and the half a reader takes on trust is the heading. **Repaired in the same change
as this amendment** — the heading is struck and the body's "D15's default applied here" justification
replaced by AD9's rule; the DECISION it always made is unchanged, only its stated reason was wrong.)*

### AM8 — D9 declares `UiVisual` **dense**, and D10 makes it a term of `ui_render_discovery`'s `Or<…>`; those two cannot both be true

*(added 2026-08-27 at the A1 pre-build audit. This is the amendment A1 cannot land without — it is the
one statement in the authority whose failure mode is a **frozen picture with no panic, no error and no
failing assertion**.)*

D9 spelled the sink `#[component(storage = "dense")] pub struct UiVisual`
(`UI-ADVANCED-ARCHITECTURE.md:833-834` — **the attribute is struck there in the same change as this
amendment**). D10's tier table then says of the same type: *"the sink is a term of
`ui_render_discovery`'s `Or<…>` (D6b)"* (`:986`, `:987`).

**A dense `Changed<C>` inside `Or<..>` can never be true.** The `Or` `QueryFilter` impl
(`filter.rs:1813` macro, `1833` impl) overrides **none** of the dense hooks — `grep` over its whole
body, `filter.rs:1830-2035`, returns zero hits for `HAS_DENSE`, `HAS_DENSE_INCLUDE`, `resolve_dense`,
`dense_include_candidates` — so `HAS_DENSE` takes the trait default `false` (`filter.rs:155`), the
cursor's `if const { F::HAS_DENSE }` dense-resolution arm (`iter.rs:188`) is never emitted, the inner
term's store pointer stays the `init_fetch` NULL, and `Changed::filter_fetch`'s dense arm returns on
its first line, `if fetch.dense.is_null() { return false; }` (`filter.rs:1483-1485`).

**MEASURED at this audit, over AD5's *exact* write** (`Mut<sink>::set_if_neq`, one changed field),
with the discovery filter's own shape `Query<(), Or<(Changed<Marker>, Changed<Sink>)>>`:

| sink storage | bare `Changed<Sink>` | the same term inside `Or<..>` |
|---|---|---|
| **dense** | **1** | **0** |
| **table** | — | **1** |

The architecture already carries this rule — *"a DENSE component cannot be a repaint sink through
D6b"* (`UI-ADVANCED-ARCHITECTURE.md:998-999`) — and it was written on 2026-08-21 while amending **row 1's
`UiSpriteCursor`**. `UiVisual` stands in that same row and in the row below it, and D9's `dense`
attribute was never touched. The shipped tree had already ruled the other way and said so at the
macro that owns the promise: *"Animation adds `UiVisual` HERE (a table component — **the animation
plan's own text is corrected to say so**)"* — `crates/boyko_render/src/ui/gather.rs:121-122`. **It was
not.** `docs/OPEN-QUESTIONS.md`'s own 2026-08-21 entry says *"Both documents are amended"*; verified
false for this plan and for D9, and repaired in the same change as this amendment.

**There is no second option.** §6 R4 offers *"or each tween system bumps `UiRenderGeneration` at its
own writer"*. That is **structurally impossible**: `UiRenderGeneration` lives in `boyko_render`
(`crates/boyko_render/src/ui/pack.rs:883`), `ui_visual_tick` lands in `boyko_ui::animation` beside
`UiClock`, and `boyko-render` depends on `boyko-ui` (`boyko_render/Cargo.toml:80-82`, which states the acyclicity as a rule) while `boyko-ui`
names no render crate — its own manifest states the acyclicity as a rule. A `boyko_ui` system cannot
take `ResMut<UiRenderGeneration>` without inverting the crate graph.

**Correction: `UiVisual` is a TABLE component. The four `Tween*` stay dense.** The normative form,
the reason and the rejected alternatives are **AD10**; the storage kind is stated in A1's Lands list,
which is the one place an implementer reads to write the derive.

---

## 2 · Decisions

### AD1 — `UiClock` is a resource, not a `Res<Time>` read at each consumer

```rust
#[derive(Resource)]
pub struct UiClock {
    /// Clamped real delta, seconds. The default clock of the TWEEN lane (D15, AM7) —
    /// unscaled and pause-blind by construction, which is the point.
    dt_real: f32,
    /// Clamped virtual delta, seconds — zero while `Time` is paused, AND scaled by
    /// `Time::relative_speed`. The default everywhere D15's per-row `flags` bit does
    /// not exist (AM7 / AD9).
    dt_virtual: f32,
    /// UI-local hitch clamp applied to BOTH deltas. Default 100 ms (AM6), and the
    /// default is a REFERENCE to `sprite::UI_FALLBACK_MAX_DELTA`, not a second `0.1`
    /// (AD9 (3)).
    max_delta: f32,
}
```

*(2026-08-26 — **the corrected field docs, and the surface that was missing.** `dt_virtual`'s
contract named two of the three properties the SHIPPED gate already enforces for exactly this
quantity: scaling was absent, and
`g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`'s leg (c)
(`ui_s5_sprite_sheet.rs:564-575`) asserts it by name. A field contract silent about a property an
existing gate enforces invites an implementation that drops it. Separately, the struct as written had
three private fields and **no accessors**, so neither A0's own gate legs nor any consumer could read
it, and §7 Q1 filed the clamp as an owner VALUES call against a field with no setter — answering it
would have been a recompile. **A0 therefore also lands `dt_real()`, `dt_virtual()`, `max_delta()` and
a validated `set_max_delta(f32)`** that panics on non-finite or non-positive input, mirroring
`Time::set_max_delta`'s own validated-setter idiom (`time.rs:155-160`), which panics on zero.)*

Written once per frame by `ui_clock_tick` (a normal system, `Res<Time>` in, `ResMut<UiClock>` out),
read by every time-varying UI system.

**Reason.** (1) **One clamp, one place.** AM6's hazard is invisible until someone alt-tabs; putting
the clamp at each consumer means the third consumer forgets. (2) **One `Duration → f32` conversion
per frame** instead of one per consumer. (3) **It is the seam the siblings need.** ~~The sprites
plan's flipbook and the interaction plan's `ScrollMomentum` are both time-varying and neither is a
tween; they read `UiClock`, not `Time`, so "the UI's clock" is one decision rather than three.~~
**Struck as a premise, kept as a conclusion (2026-08-26, AM7).** The three consumers were not
waiting to be told: two SHIPPED on `Time::delta_secs()` while this row was being read as undecided
(`ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*; `particle_clock.rs:4-8`), and the third states its choice and asks this plan to
ratify it (`UI-PLAN-INTERACTION.md:453`, `:490`, `:967`). `ScrollMomentum` and `HoverDwell` do not
exist in the tree at all — `grep -rn` across `crates/` returns nothing for either — so this row was
reasoning about hypothetical consumers while the real ones were being built the other way. The
conclusion survives: one clock resource, one clamp, one `Duration → f32` — but the DEFAULT it hands
an unflagged consumer is `dt_virtual`, not `dt_real` (**AD9**). (4) A per-row
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
for the node's image record rather than beside it — [`UI-PLAN-SPRITES-DECISIONS.md` **S-D12 (1)**](UI-PLAN-SPRITES-DECISIONS.md#1-uinineslice-suppresses-the-sub-10-image-record--the-slices-are-the-image-sliced), 2026-08-21)* —
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

*(2026-08-27, the A1 pre-build audit — **three things this shape left unsaid, each of which decides a
behaviour**.*

*(a) **"compose the four channels into a stack-local `UiVisual`" never named the BASE**, and the two
readings are not interchangeable: from `UiVisual::default()` a node that finished a `TweenOffset` at
−400 px has its offset snapped back to 0 on the first frame of a subsequent `TweenTint` — the finished
animation silently undoes itself — while from `*sink` it is carried. **The base is `*sink`: see
AD12.** No A1 gate ran two channels, so an identity-base implementation passed all eight.*

*(b) **`set_if_neq` requires `UiVisual: PartialEq`, which A1's Lands list never stated**, and the
derived float equality is **not idempotent under NaN** — which turns AM1's own stated catastrophe
(*"a tick that bumps every frame defeats the render gate as surely as one that never bumps"*) into a
release-reachable one. **The equality is bytewise and hand-written: see AD11.***

*(c) **the all-`None` `continue` is load-bearing for more than cost.** AM2 (1) prices it — one
`PartialEq` on 24 B per ever-animated row per frame — and §4/M2 measures it. But MEASURED at this
audit, it is also what BOUNDS (b): with AD11's bytewise equality the two are independent, and
without it the `continue` is the only thing keeping a NaN-carrying rested row from bumping the render
gate on every frame forever — `[0, 0, 0]` with the `continue`, `[1, 1, 1]` without. A8 gate 3 prices
the `continue`; it is **not** a licence to delete it.)*

**The trace, row class by row class** *(added 2026-08-27 at the A1 pre-build audit, because every
defect this rung's audit found was a case one of these five rows answers and no prose did. Read with
AD10 — the sink is TABLE — and AD11/AD12.)*

| Row class | `AnyOf` yields | What the tick writes | Tick bumped? | What the render gate sees |
|---|---|---|---|---|
| **never animated** | *not visited at all* — no `UiVisual` row, and a table `Mut<UiVisual>` include admits only archetypes that carry it (MEASURED: 2 sink rows + 2 sink-less rows ⇒ **2** visits) | nothing | no | nothing. The node folds by the **identity** at pack (AD6) — byte-identical to today, which is A4's disarmed gate |
| **mid-tween** | ≥1 `Some` | `composed = *sink` with each live channel's own fields overwritten (**AD12**); `set_if_neq` writes it | **yes** — the value differs | `Changed<UiVisual>` → the `Or` term in `ui_pack_inputs!(changed)` fires → `generation.bump()` → the frame repacks. MEASURED end to end for a TABLE sink (1 row through the `Or`); **0 rows for a dense one**, which is AM8 |
| **channel just finished** (this frame) | that channel `Some`, `elapsed >= duration` | the endpoint **assigned exactly** (§5's property — `to`, not `to ± ULP`); the other fields carried from `*sink`; `(Entity, ComponentId)` pushed onto `UiTweenScratch` | **yes**, if the endpoint differs from last frame's value | one final bump. `ui_tween_reap` (exclusive, immediately after) then removes the dense row, so `ui_transition_apply` can read "no row present ⇒ at rest" the same frame (AD5) |
| **rested** (all channels reaped, sink retained) | `(None, None, None, None)` — **visited, not skipped** (AM2, MEASURED) | nothing: the all-`None` `continue` fires **before** the sink is touched | no | nothing. The generation holds and D6a's per-slot skip stays armed. *This is the row the sink persists for: its last value IS the resting appearance (AM2), so removing it would snap the element back* |
| **plateau or NaN-carrying, channel live** | ≥1 `Some`, composing to the value already held | `set_if_neq` compares **bytewise** (AD11) and writes nothing | **no** | nothing. Under `#[derive(PartialEq)]` instead: `NaN != NaN` ⇒ a write and a bump on **every** frame forever ⇒ `UiRenderGeneration` bumps every frame ⇒ the D6a skip is disarmed for the **whole UI**. MEASURED `[0,0,0]` vs `[1,1,1]` |

**Two things the trace makes visible that no gate did.** (1) The `continue` changes **nothing** the
render gate can see — under AD12's base an all-`None` row composes the value it already holds, so with
or without it the row is silent. That is why A1's red #2 could not fire and why the `continue`'s gate
is A8's cost pair, not gate 3. (2) The only row class that can bump the gate **forever** is the last
one, and the thing that stops it is an equality, not a `debug_assert!` — which is AD11.

**Storage, stated here because AD5 is where the query lives (AM8/AD10).** The sink term is a
**table** component and the four `AnyOf` arms are **dense**. That combination is the one that works:
`AnyOf` DOES forward the dense hooks — `HAS_DENSE` OR-folds over its arms (`data/anyof.rs:118`) and
`resolve_dense` is forwarded to each (`data/anyof.rs:137`) — so a dense channel arm resolves its
store and yields `Some`/`None` per row correctly (MEASURED, leg L3). `Or` forwards none of them, which
is why the SINK cannot be dense (AM8). Nothing reads `Changed<Tween*>`, and no `Tween*` appears in any
`Or` in this plan or in `ui_pack_inputs!`, so the dense arms are never filtered — that is the whole
condition under which dense storage is safe here.

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

**The second route is named, 2026-08-27.** `default_mode_is_off`'s precedent works because *two*
independent spellings produce the same value. `UiVisual` had only one until this amendment, so
**A1 also lands `pub const UiVisual::IDENTITY`**, and the hand-written `Default` returns it. Gate 4
then compares two routes that neither derives from the other — which is a gate that can fail, unlike
the "does not `#[derive(Default)]`" half A1 claimed and could not build (a derived and a hand-written
`impl Default` are the SAME trait impl to the type system; no Rust test separates them). See A1's
corrected gate 4.

**And the absent case must agree with it:** a node with **no** `UiVisual` row folds by the identity,
which is arithmetically the same instance bytes as today. That equality is A4's disarmed gate.

### AD7 — the hit-test folds the same transform, behind an O(1) frame-level guard

`collect_candidates` (`focus.rs:204-257`) folds AD3's pair on its own DFS so a slid-in panel is
clickable **where it is drawn**. The per-node `UiVisual` probe is skipped wholesale when the frame
carries no visual at all:

~~```rust~~
~~let any_visual = world.dense_registry()~~
~~    .store(UiVisual::component_id())~~
~~    .is_some_and(|s| s.live_count() != 0);       // dense_store.rs:353, ecs_master.rs:590~~
~~```~~

**Struck 2026-08-27 at the A1 pre-build audit — this guard does not survive AD10, and its failure is
silent in the worst direction.** `DenseRegistry::store` returns `None` for an id no dense store was
ever created for (`dense_registry.rs:133-136`), and a TABLE id never gets one. So with `UiVisual`
table the expression evaluates to `false` **forever**: no panic, no `debug_assert`, no compile error —
`any_visual` reads *"nothing animates"* on every frame of every UI, the fold never runs, and A6 ships
the exact "the UI is haunted" bug this decision exists to prevent. The mirror error is equally quiet:
if the guard had been kept and the sink made dense to suit it, **AM8**'s frozen picture arrives
instead.

**The guard becomes an archetype-level emptiness check** — the crate's own
`ui_layout_discovery`/`is_empty()` idiom, cited at `crates/boyko_render/src/ui/gather.rs:501-503` as
*"`is_empty()` is archetype-level only — the `ui_layout_discovery` precedent"* (re-measured by
CONTENT 2026-08-28, part 7; it read `:434-436`, which is today three lines of a `NineSliceMode`
match arm — plausible content again) — evaluated **once per frame**, not once per node, which is
the property AD7's reason actually needs. Its exact spelling is **A6's** to pick against
`collect_candidates`' real parameter list, and A6 must state it; what A1 owes is only that
`UiVisual` is a table component so an archetype-level answer exists at all. AD7's rejected (b) — *"a
`With<UiVisual>` query probe per frame … pays an archetype walk where a `live_count()` load answers
exactly"* — is **re-opened by this strike**: with no `live_count()` available, the archetype walk is
the cheapest correct answer, and it is O(matched archetypes) once per frame rather than O(nodes).

**Reason.** Without the fold, an animating menu accepts clicks at its pre-animation position for the
whole animation — the class of bug that is reported as "the UI is haunted" and diagnosed in an
afternoon. With the fold and without the guard, this plan would add a probe per node per frame to the
pass D23 already names as the campaign's likely dominant cost, on every UI, including the 100 % of
UIs that animate nothing. ~~`live_count()` is one load and makes that cost exactly zero.~~ **The
archetype-level check is one matched-list emptiness test per frame and makes that cost exactly zero**
(2026-08-27).

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

### AD9 — which field a consumer reads is decided by whether it carries D15's `flags` bit, and the clamp has exactly one definition

*(added 2026-08-26 at the A0 pre-build audit; implements **AM7**.)*

**(1) The read rule, normative and in one sentence.** *A consumer that carries D15's per-row `flags`
bit reads `dt_real` unless that bit says otherwise. A consumer with no `flags` bit reads
`dt_virtual`.* In v1 the only lane with the bit is the tween row (A1), so in v1 the rule reads:
**tweens are real by default and virtual per row; everything else is virtual, full stop.**

**Reason.** D15's argument is *"a pause menu that fades in on a paused virtual clock never fades"* —
true, and it is an argument about a **tween with an endpoint**, on a UI that is shown *because* the
game is paused. It is not an argument about a flipbook, a fling or a dwell timer: none of them has an
endpoint to be robbed of, and all three are *worse* on the real delta — they keep running under a
pause menu and they ignore slow-motion. Two of the three already shipped saying exactly that, in
their own source (AM7 (1), (2)). A default that no built consumer takes is not a default; it is a
trap for the fourth consumer, who will read the word *default* and take it.

**Rejected:** (a) *flip the clock to virtual outright and delete `dt_real`* — that throws away D15's
correct case along with its overreach, and the pause-menu fade is a real requirement arriving at A3;
(b) *leave the default real and let each consumer opt out* — that is the shape being repaired: an
opt-out is invisible at the call site and the third consumer forgets, which is the failure AD1's own
reason (1) rejects for the clamp; (c) *two resources, `UiRealClock` and `UiVirtualClock`* — the
selection is D15's per-ROW bit, so it must be a field select inside one row's tick, not a
`SystemParam` choice made once per system.

**(2) The consequence for `ui_sprite_flipbook`, stated so no rung has to rediscover it.** Under the
rule the migration is `dt = clock.dt_virtual()` — which is `time.delta_secs().min(max_delta)`: **the
same arithmetic, from the same source, against the same clamp value** as `ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)* did before
A0b. It is
therefore behaviour-preserving, which is what both plans promised (*"swaps one `SystemParam` and
deletes one `min`"*) **and could not have delivered under the struck default**: `dt_real` reds legs
(b) and (c) of the SHIPPED `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware` — a paused
game animates, and `set_relative_speed(0.5)` stops halving. Neither plan named the field; this
decision names it.

**(3) The clamp has exactly ONE definition in the crate.** `0.1` already ships as
`pub const UI_FALLBACK_MAX_DELTA` in `pub mod sprite` (`sprite.rs:320`, `lib.rs:44` — public API).
`UiClock::default()` **references** it; it does not restate `0.1`. No pin test is owed, because with
one definition there is no second datum to diverge — the campaign's "dead datum" lesson applied
before the datum exists rather than after. §7 Q1's answer, when it comes, edits one line and both
readers follow. *(The direction is deliberate: the const keeps its name and its site until the
flipbook's `min` is gone, so no public symbol moves in the same rung that changes a system signature.
Whichever rung deletes the last reader of `UI_FALLBACK_MAX_DELTA` moves the definition onto `UiClock`
and drops the const; A0b leaves exactly one reader, `UiClock::default()`.)*

**(4) A hole this decision exposes and does not fill.** D15's `flags` bit is named **once** in this
plan (AD1 reason (4)) and appears in **no rung's landing list** — A1 lands `TweenTint` /
`TweenOpacity` / `TweenOffset` / `TweenScale` without ever spelling their fields. Under this rule the
bit is what makes `dt_real` reachable at all, so **A1's Lands list must name the field**, or
`dt_real` ships with no reader and D15's opt-in becomes another dead datum. Recorded here rather than
silently assumed.

### AD10 — `UiVisual` is a **table** component; the four `Tween*` are **dense**

*(added 2026-08-27 at the A1 pre-build audit; implements **AM8**. This is the rung's headline decision:
A1 is the only rung that can make it, and three authorities gave three answers while A1's own Lands
list gave none.)*

```rust
#[repr(C)]
#[derive(Component, Clone, Copy, Debug)]   // NOT dense; NOT #[derive(Default)]; NOT #[derive(PartialEq)]
pub struct UiVisual { /* 24 B — AM5, AD6, AD11 */ }

#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
pub struct TweenTint { /* … and TweenOpacity / TweenOffset / TweenScale */ }
```

**Reason.**

1. **It is the only kind that reaches the render gate.** MEASURED (AM8's table): a dense sink written
   through AD5's exact `Mut::set_if_neq` is seen by a bare `Changed` (1 row) and **not** by the
   discovery filter's `Or` (0 rows); a table sink is seen (1 row). The failure is a frozen picture
   with nothing red.
2. **The alternative §6 R4 names is impossible, not merely worse.** `ui_visual_tick` lives in
   `boyko_ui`; `UiRenderGeneration` lives in `boyko_render` (`ui/pack.rs:883`); the dependency runs
   `boyko-render → boyko-ui` (`boyko_render/Cargo.toml:80-82`, which states the acyclicity as a rule) and `boyko-ui` names no render crate.
   There is no fork to weigh.
3. **The crate already ships this exact split, and it shipped for this exact reason.**
   `UiSpriteSheet` is a **table** component, is a member of `ui_pack_inputs!`, and takes the flipbook's
   per-frame `Mut::set_if_neq` write *because that write's tick is the repaint signal*
   (`crates/boyko_ui/src/components.rs:658-665`); `UiSpriteCursor` is **dense**, is deliberately
   **out** of the list, and holds the flipbook's private state (`components.rs:820-829`).
   `UiVisual` : `Tween*` :: `UiSpriteSheet` : `UiSpriteCursor` — same split, same reason, one rung
   later.
4. **The archetype cost is bounded and it is paid once.** `UiVisual` **persists after a tween
   finishes** (AM2), so a table sink costs exactly **one** archetype migration per element that ever
   animates, and none thereafter — while the four channels, which are inserted and reaped per
   animation, keep their churn out of the signature. The dense-storage argument
   (`crates/boyko_ui/src/components.rs:822-825`'s "per-frame churn") applies to the CHANNELS and is why they
   stay dense; it never applied to a sink that is never removed.

**Rejected:** (a) *dense sink + `ui_render_discovery` gains a nested `Or`* — nesting fixes the arity
ceiling (R4), not the dense blindness: the inner `Or` has the same defaulted `HAS_DENSE`.
(b) *dense sink + fix the kernel's `Or` impl* — filed as a kernel defect with the owner
(`docs/OPEN-QUESTIONS.md`, 2026-08-21, option 1) and explicitly a VALUES/SCOPE call; a rung does not
land on an unanswered kernel change, and the recommendation there is option (2).
(c) *dense sink + the tick bumps the generation itself* — reason 2: it does not compile.
(d) *four `Tween*` as table columns* — four archetype migrations per animation start and four more per
completion, on the pass that starts every hover; and nothing filters them, so dense costs nothing.

### AD11 — `UiVisual`'s `PartialEq` is **bytewise and hand-written**, never derived

*(added 2026-08-27 at the A1 pre-build audit.)*

```rust
impl PartialEq for UiVisual {
    /// Bitwise, NOT `f32`'s `PartialEq`: `set_if_neq` is the render gate's only
    /// throttle, and `NaN != NaN` makes the derived form bump it forever.
    fn eq(&self, o: &Self) -> bool {
        self.tint_mul == o.tint_mul
            && self.opacity.to_bits() == o.opacity.to_bits()
            && self.offset_px[0].to_bits() == o.offset_px[0].to_bits()
            && self.offset_px[1].to_bits() == o.offset_px[1].to_bits()
            && self.scale[0].to_bits() == o.scale[0].to_bits()
            && self.scale[1].to_bits() == o.scale[1].to_bits()
    }
}
```

**Reason.** AD5's whole render-gate contract is *"a value-preserving frame does not bump"*. Under the
derived equality that contract has an exception nothing states: `NaN != NaN`, so one NaN anywhere in
the sink makes `set_if_neq(THE SAME BYTES)` write and bump on **every** frame. One such row bumps
`UiRenderGeneration` — which is one global counter, bumped if **any** row changed — so the D6a
per-slot skip is disarmed for the **whole UI**, permanently. **MEASURED**, on the legitimate fixture
of a *plateau* tween (`from == to`, which an author writes whenever a transition targets the state it
is already in) over a sink carrying one NaN:

| `UiVisual`'s `PartialEq` | `Changed` rows, 3 still frames |
|---|---|
| **bytewise (this decision)** | `[0, 0, 0]` |
| **`#[derive(PartialEq)]`** | `[1, 1, 1]` |

A NaN is reachable in **release** without any kernel bug: §5's `debug_assert!`s on `inv_duration`,
`opacity`, `scale` and the fold output all compile out, and the public helpers take author `from`/`to`
values (`start_tween_offset(world, e, from, to, ms, easing)`) with no release-side filter.

**Rejected:** (a) *a release-time `is_finite()` guard on the composed value before the write* — six
float tests per animating row per frame, forever, to defend against a case the equality can answer for
free; and it must then decide what to substitute, which is a product question A1 has no answer to.
(b) *`#[derive(PartialEq)]` plus a `debug_assert!`* — debug-only, and this branch has already recorded
that "a rule about RELEASE behaviour is ungatable in debug". (c) *`#[derive(PartialEq, Eq)]` over a
`[u8; 24]` newtype* — the same bytes with a worse field API and a `Hash`/`Eq` surface nothing wants.

**The cost is not a regression:** six integer compares against six float compares, on a type that is
already `#[repr(C)]` POD `Copy`. It is priced by A8's tick bench like everything else.

*(± 0.0 note, so the trade is on the record: bytewise equality calls `+0.0` and `−0.0` different, so a
channel that lands on `−0.0` where `+0.0` stood costs **one** extra bump — one frame, not a state. The
derived form's NaN case costs **every** frame, forever. This is the direction the trade must run.)*

### AD12 — the fused tick composes from `*sink`, not from the identity

*(added 2026-08-27 at the A1 pre-build audit; closes a hole AD5 left open and A1's eight gates could
not see.)*

```rust
let mut composed = *sink;                       // NOT UiVisual::default()
if let Some(t) = tint     { composed.tint_mul  = eval(t); }
if let Some(o) = opacity  { composed.opacity   = eval(o); }
if let Some(f) = offset   { composed.offset_px = eval(f); }
if let Some(c) = scale    { composed.scale     = eval(c); }
sink.set_if_neq(composed);
```

**Reason.** The four channels own **disjoint** fields of the sink, so "compose" means *overwrite the
fields whose channel is live and carry the rest*. From the identity, a field whose channel has been
reaped is reset — MEASURED: a node whose `TweenOffset` finished at `−400 px` reads `off = 0` on the
first frame of a later `TweenTint` under an identity base and `−400` under `*sink`. That contradicts
`UiVisual`'s stated persistence (AM2), and it is the "UI is haunted" class again: a panel that slid in
and stayed jumps home the moment anything tints it.

**A consequence that must be stated, because it disarms a red.** Under this base an all-`None` row
composes **the value it already holds**, so `set_if_neq` writes nothing and bumps nothing — which is
exactly why A1's original red mutation #2 (*delete the `continue` ⇒ the rested element becomes
`Changed` every frame*) **cannot fire**. MEASURED, the mutated tick over still frames: `[0,0,0,0]`
under a `*sink` base, and `[0,0,0,0]` under an identity base too (the single normalizing write lands
on the tick's first frame, which gate 3's *"on every subsequent still frame"* excludes). The
`continue` is a **cost** decision (AM2 (1), priced at A8) and a **NaN-containment** decision
(AD5 (c)) — never a change-detection one — and A1's reds are corrected accordingly.

**Rejected:** (a) *identity base* — above. (b) *identity base plus a "sticky" flag per field* — four
bits and a second source of truth for a question `*sink` already answers. (c) *let each channel's reap
write its endpoint back into the sink* — the reap is exclusive and would become a second writer of
`UiVisual`, which AD5 forbids (`ComputedRect`'s single-writer discipline).

### AD13 — a **dense** member of `ui_pack_inputs!` is a BUILD ERROR, not a comment

*(added 2026-08-27 at the A1 pre-build audit. The decision belongs to whichever rung ships D6b — the
sprites plan's seam rung — but **animation is the subsystem that discovers it twice**, and A4 is the
rung it would bite.)*

`crates/boyko_render/src/ui/gather.rs:85-89` already states the rule in prose: *"adding a component to
`ui_pack_inputs!` wires the discovery filter for free" is true for TABLE components only. A DENSE
component added to this list would be read correctly by the gather and would be INVISIBLE to
`ui_render_discovery` — the frame would never repaint, with nothing saying so.* The macro's existing
lockstep does not cover it: **deleting** a member breaks `gather_ui_nodes`' destructuring at compile
time (the M0-c red, `gather.rs:370-371` — the one arity-locked destructuring of
`ui_pack_inputs!(read …)`; the anchor read `:128-129` until 2026-08-27, which the S5 and F6 landings
had left pointing at the member LIST rather than at the site that breaks), while **adding** a dense
one compiles clean and is silently dead.

**The mechanical form exists in the same workspace.**
`crates/boyko_render/src/occlusion_marker.rs:171-176` const-asserts
`!<C as Component>::STORAGE_IS_DENSE` with a rationale string, and MEASURED at this audit the const is
both reachable and discriminating (`true` for a dense derive, `false` for a plain one). So
`ui_pack_inputs!` grows a `storage` arm that expands the list into one `const _: () = assert!(...)`
per member, and the wrong storage kind becomes `error[E0080]` at the site that chose it.

**Reason.** The campaign's headline defect class is *the gate that cannot fail*; a prose warning at
the macro is exactly that shape, and this plan is the second subsystem to walk into it (the first was
`UiSpriteCursor`, S-D16 (1)). The assert costs nothing at runtime and cannot be forgotten, because
adding a member is what triggers it.

**Handed over, with what it needs:** the seam rung owes (i) this const-assert arm, (ii) R4's nested-`Or`
emission, and (iii) a test that **runs** `ui_render_discovery` rather than naming its type — a
type-level test sees neither the dense hole nor R4's `init_state` panic.

---

