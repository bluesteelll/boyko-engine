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
  `let clamped = raw.min(self.max_delta);` at **`time.rs:200`**. *(`UI-PLAN-SPRITES.md` S-D17 and
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

## 3 · The rung ladder

**Unconditional gate on every rung:** `cargo clippy -p boyko-ui -p boyko-render --all-targets -- -D warnings`;
`cargo test -p boyko-ui -p boyko-render --all-targets --no-fail-fast`; Miri where new `unsafe` lands
(none is expected in this ladder — see §6 R3); every `// SAFETY:` present; author-only commit.
**`--no-fail-fast` is load-bearing** — one red target otherwise shadows every target behind it.

**Default OFF, and which rung turns something on.** Rungs **A0–A3** are structurally off: the plugin
is opt-in, no existing host adds it, and no component is inserted by anything that ships. *(2026-08-26
— **one qualification, added with A0b.** A0 now also migrates `ui_sprite_flipbook` off `Res<Time>`,
which changes a **shipping public signature** in `boyko_ui`. Structural off-ness is unaffected — no
`src/` plugin registers that system, so nothing that ships gains a running system — but the migration
is a breaking API change and it is not "nothing visible": a host or test that registers the flipbook
without inserting `UiClock` now **panics** at `get_param` (`res.rs:130`, `missing_resource_panic`),
loudly rather than silently. A0's leg 6 is what keeps the bytes identical.)* **A4** is
the first rung whose code runs on every packed node, and it is *arithmetically* off — the identity
fold (AD6) reproduces today's bytes exactly, which is A4's disarmed gate. **No rung in this ladder
changes an existing golden byte.** The first *new* golden is authored by the observer rung (D32,
sprites plan) once it has an animated hover to show, which is after A3 and A4 have both landed.

---

### A0 — the UI clock, and the one consumer that already exists — **size S → M** · *no cross-plan dependency*

*(rung rewritten 2026-08-26 at the A0 pre-build audit. **What S5 landed is not this rung**: `UiClock`,
`ui_clock_tick`, `UiAnimationPlugin` and `UiAnimationSet` have **zero** occurrences in the tree —
`grep -rn "UiClock\|ui_clock_tick\|UiAnimationPlugin\|UiAnimationSet\|dt_real\|dt_virtual"
--include=*.rs crates/` returns one hit and it is a doc comment (in `sprite.rs`, pre-A0b — unanchored for the same reason). What S5 landed is a
`pub const` and one inline `min`. See AM6's re-verification block and the ledger at the end of this
rung.)*

**Lands — A0a, the clock.**

* `UiClock` (AD1) — the three fields **plus the three accessors and the validated `set_max_delta`**
  the struct was missing — `ui_clock_tick`, `UiAnimationPlugin`, and the `UiAnimationSet` ordering
  set, following `UiWidgetsPlugin`'s registration idiom verbatim (~~`widgets.rs:267-289`~~ **`widgets.rs:267-290`**, re-verified
  live 2026-08-26 — the struck range stopped one line short of the `impl`'s own brace:
  `UiWidgetSet` decl at `:234-235`, `impl Plugin` at `:267`,
  `add_systems_cfg_in(CoreSchedule::Main, …)` at `:278`, block closing at `:290`).
* `UiClock::default()`'s `max_delta` **references** `sprite::UI_FALLBACK_MAX_DELTA` (AD9 (3)); no
  second `0.1` is written.

**Lands — A0b, the migration of the one consumer that already exists.**

* `ui_sprite_flipbook` takes `Res<UiClock>` instead of `Res<Time>` and reads **`dt_virtual`**
  (AD9 (1), (2)); its pre-A0b inline `min` *(deleted by A0b, and deliberately unanchored — a
  coordinate into deleted state resolves to whatever live line now occupies it)* is deleted; the
  clock paragraph of its doc comment *(pre-A0b, unanchored for the same reason; rewritten to AD9,
  and now that system's `# The clock (A0b …)` doc section, `sprite.rs:329-349`)* is rewritten to
  point at AD9 instead of at the fallback.
* The **four** registration sites — `flipbook_schedule` in
  `boyko_render/tests/ui_flipbook_gpu_golden.rs` and in `boyko_render/tests/ui_s5_sprite_sheet.rs`,
  `g5_3_the_churn_split_is_real`'s inline builder in that same S5 file, and `flipbook_only` in
  `boyko_ui/tests/ui_s6_authoring.rs` — insert `UiClock` and register `ui_clock_tick`
  **ordered ahead of the flipbook**. There is no `src/` registration site to fix: `grep -rn` finds
  none, and `boyko_ui`'s four plugins (`plugin.rs:84`, `widgets.rs:267`, `interaction/plugin.rs:157`,
  `profiling_overlay.rs:207`) do not mention the system.
  *(Landed 2026-08-26 — the four sites are named above by SYMBOL rather than by line, because
  the PRE-LANDING coordinates this bullet used to carry moved as the edits grew and now resolve
  to unrelated live text. Post-landing they sit at `ui_flipbook_gpu_golden.rs:270`,
  `ui_s5_sprite_sheet.rs:154` and `:622`, `ui_s6_authoring.rs:363`; the verb everywhere is
  `.after(<clock key>)`, so no existing `.before(discovery)` edge was touched. `boyko_ui` now
  has **five** plugins — `UiAnimationPlugin` at `animation.rs:400` is the fifth, and it is the
  only one that mentions the tick.)*

> **Why A0b is a rung item and not "a later rung".** *(2026-08-26.)* It belonged to **no rung in any
> ladder** — the sprites ladder is complete through S6, and the animation ladder's rungs A0–A8 never
> touch the flipbook; the replacement existed only as a promise inside two dependency tables
> (`UI-PLAN-ANIMATION.md` §8; `UI-PLAN-SPRITES.md` S-D17, `:3582`). Leaving it unowned means the
> crate carries **two UI delta sources at once** — `Res<Time>` + `UI_FALLBACK_MAX_DELTA` in
> `sprite.rs`, and `UiClock` — with no rung scheduled to collapse them and no gate that would notice,
> which is precisely the second-source-of-truth this plan's own §0 table exists to prevent. It is two
> lines of production code and four test registrations, and **its gate already exists and is green**
> (leg 6). The alternative considered and rejected: a separate micro-rung A0c. Rejected because the
> window between A0 and A0c is exactly the two-sources state, and because a rung whose entire content
> is "re-run someone else's gate" is not a rung.

**Gate — six legs. Each states what it does NOT prove, because on this branch two legs were written
believing a third covered them.**

1. **The tick ran, and `dt_real` is the real delta below the clamp.**
   `dt_real == Time::real_delta().as_secs_f32()` after a 16 ms frame.
   **Does NOT prove `dt_real` came from the real side.** MEASURED, not argued: under this leg's own
   precondition (unpaused, `relative_speed == 1.0`, raw ≤ both clamps) `advance_with` takes the
   integer-ns branch and assigns `delta = clamped = raw` (`time.rs:197-210`), which the kernel pins
   itself — `advance_with_default_path_is_integer_exact` (`time.rs:279-285`) asserts
   `t.delta() == raw`. So `delta_secs()` and `real_delta().as_secs_f32()` are the **same `f32`** here
   and this leg cannot tell them apart. Legs 2 and 4 are what separate the two sources. What leg 1
   *does* prove, and nothing else does: **the system ran at all.**
2. **Paused: `dt_virtual == 0.0` AND `dt_real > 0.0` on the same frame.** D15's whole reason, as a
   test, and the leg that catches both cross-wirings.
   **Does NOT prove `dt_virtual` is ever non-zero** — see leg 4, which exists because it does not.
3. **Hitch (AM6), BOTH deltas.** A 2 000 ms raw delta yields (a) `dt_real == max_delta`, not 2.0, and
   (b) `dt_virtual == max_delta`, not 0.25. Leg (b) is live rather than decorative because `Time`'s
   own 250 ms clamp lands first, so an unclamped `dt_virtual` reads **0.25** — a value that is
   neither the input nor the answer, and would otherwise look plausible. AD1 says the clamp applies
   to BOTH deltas; this is the only leg that says so too. Assert against `clock.max_delta()` itself,
   never a `0.1` literal — the `min` is taken against that very value, so the comparison is exact by
   construction.
4. **NEW — `dt_virtual` is positive and SCALED on an unpaused frame, and it is NOT `dt_real`.** With
   `set_relative_speed(0.5)` and an **80 ms** raw delta — deliberately below the 100 ms clamp, so
   this leg tests the SOURCE and the SCALING with the clamp out of the picture:
   `dt_virtual == Duration::from_millis(40).as_secs_f32()` **and**
   `dt_real == Duration::from_millis(80).as_secs_f32()`. Assert against the computed `Duration`s,
   never `0.04` / `0.08` literals — this project does not gamble a gate on a decimal literal's ULP.
   Note this is also the only leg that separates the two fields on an **unpaused** frame, which leg 1
   provably cannot.
   **Why this leg exists:** without it an implementation that hardwires `dt_virtual = 0.0`
   unconditionally passes legs 1, 2, 3 and 5 **green**, and so does one that drops
   `relative_speed` — and `dt_virtual` is the field AD9 makes every unflagged consumer read. The plan
   previously believed leg 2 covered this (A1's gate 7 is annotated *"(A0 leg 2, downstream)"*); it
   does not, and the hole would have surfaced one whole rung later. This is the campaign's headline
   class — the gate that cannot fail — found inside the gate written to establish the clock.
5. **Plugin containment, in the ACTING form, WITH its non-vacuity control.** ~~an identical
   registered schedule-label set and an identical resolved event policy~~ — **struck as an
   instrument that does not exist.** `App` exposes **no** getter for its resolved
   `EventUpdatePolicy` (private field, `app.rs:162`; setter only, `:490`) and **no** enumeration of
   registered schedule labels (`fixed_builder` private, `:145`); `CoreSchedule` is a **closed
   two-variant enum**, `Main` and `Fixed` (`app.rs:64`, *"New top-level slots are an engine change by
   design; no label map"*), so "identical label set" was a one-bit statement dressed as a set
   comparison. The borrowed file says this in its own header —
   `crates/boyko_render/tests/particle_containment.rs:6-8`: *"`App` exposes no accessor … so this
   file does not read those fields — it **acts**"*. **Leg 5 is therefore its two behavioural probes,
   verbatim** (fixed-schedule probe via `FixedTime::elapsed()`/`overstep()`; event-swap probe via a
   Main reader over 0-substep frames) — **and the third app that proves they can flip
   (`the_probes_are_not_vacuous`, `:234`) is copied WITH them.** The previous wording borrowed the
   probes without the control, which is the exact import the source file warns against: *"A gate that
   cannot fail is worse than no gate … a probe that silently returned 'clean' for every input would
   report containment forever."*
6. **A0b: the SHIPPED clock gate re-runs UNCHANGED, and the flipbook golden is byte-identical.**
   `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`
   (`crates/boyko_render/tests/ui_s5_sprite_sheet.rs:534`) is **not edited** — only its harness gains
   the resource and the tick — and its three legs (CLAMPED / PAUSE-AWARE / SCALED) stay green, which
   is the whole statement "the migration is behaviour-preserving". Plus `ui_flipbook_gpu_golden.rs`
   unchanged bytes. *An edit to G5-2 in the same change that migrates its subject voids the leg;
   if the migration cannot keep it green unedited, the field choice is wrong, not the test.*

**M4 is reported here** (§4): `dt_real` at a synthetic 2 000 ms delta, clamped **and** unclamped,
both numbers written into A0's landing note. Leg 3(a) is pass/fail; M4 is the pair of numbers, and
the two are not the same obligation. *(M4's second half — "the resulting tween `elapsed` delta" —
moves to A1, where a tween exists; see §4.)*

**RED MUTATIONS — nine, every one runnable AT THIS RUNG, and EVERY LEG OWNS ONE.** *(The
one-red-per-leg property is the whole repair: the struck text below left legs 1, 2 and 4 with no red
at all, which is how a leg that cannot fail survives a rung.)*

1. Source `dt_real` from `time.delta_secs()` ⇒ **leg 2 reds** (`dt_real == 0.0` while paused).
   *This replaces the struck mutation below and is the same defect it was aiming at.*
2. Source `dt_virtual` from `time.real_delta()` ⇒ **legs 2 and 4 both red** (`dt_virtual > 0` while
   paused; `dt_virtual` unscaled at half speed). Two independent legs, deliberately.
3. Delete the clamp on the `dt_real` line ⇒ **leg 3(a) reds** (`dt_real == 2.0`).
4. Delete the clamp on the `dt_virtual` line ⇒ **leg 3(b) reds** (`dt_virtual == 0.25`).
5. Register `ui_clock_tick` in `CoreSchedule::Fixed` instead of `Main` ⇒ **leg 5's BOTH probes
   flip** — the fixed clock advances, and the Main reader observes zero events over the 0-substep
   script, because `App::finish` resolves `event_policy_cfg: None` to `WaitForFixed` **iff** a Fixed
   schedule exists (`app.rs:591-593`). This is the red leg 4-as-written never had. It is distinct
   from the non-vacuity control shipped in leg 5: the control proves the probes can flip for *some*
   input, this mutation proves they flip for *this plugin*.
6. **A0b:** read `dt_real` instead of `dt_virtual` in the flipbook ⇒ **leg 6 reds twice** — G5-2 (b)
   PAUSE-AWARE (a paused game animates) and (c) SCALED (four 100 ms frames advance four, not two).
   *This is AD9 (1) as a test: the documented default was the wrong field for this consumer, and the
   red is the proof.*
7. Drop `ui_clock_tick` from `UiAnimationPlugin::build` ⇒ **leg 1 reds** (`dt_real == 0.0`, never
   written). Listed because leg 1's only unique claim is *"the system ran"*, and a claim with no red
   is the thing this rung is being repaired for.
8. Delete `.in_set(UiAnimationSet)` from `UiAnimationPlugin::build` ⇒ **leg 7 reds**
   (`a_consumer_after_the_set_observes_a_written_clock`: `dt_real == 0.0`). *(Added 2026-08-26 at
   the A0 verification, which MEASURED this mutation leaving `ui_a0_clock` at 7/7 and
   `boyko-ui --lib` at 20/20. `UiAnimationSet` is a doc-comment promise to downstream hosts, and a
   set with no members expands into no edges: every `.after_set(UiAnimationSet)` in the tree would
   silently become a no-op, with nothing red anywhere.)*
9. Replace `UiAnimationPlugin::build`'s insert-if-absent guard with an unconditional
   `insert_resource(UiClock::default())` ⇒ **leg 8 reds**
   (`a_host_configured_clock_survives_the_plugin`: `max_delta` reads 0.1, not the host's value).
   *(Added 2026-08-26 at the same verification, which MEASURED this mutation leaving the file at
   7/7. This is the escape hatch §7 Q1 leans on — "`set_max_delta` exists per host" — and a host
   that configures its clamp BEFORE `add_plugin` was losing it undetected.)*

~~**RED MUTATION.** Swap the default in the tween row's clock select (`dt_real` → `dt_virtual`) ⇒ leg
2's downstream assertion in A1 (a tween advances while `Time` is paused) reds.~~ **Struck 2026-08-26
— structurally unrunnable at this rung, which under the campaign's protocol meant A0 could not
close.** The tween row, its clock select and the assertion named all land at **A1**
(`UiVisual`, the four `Tween*`, `ui_visual_tick`; A1 gate 7). At A0 there is nothing to mutate and
nothing to red, yet the rung closed with *"Both must be run and the red observed before the rung
closes."* The only mutation that *was* runnable — deleting the clamp — covers `dt_real`'s clamp and
nothing else, leaving legs 1, 2 and 4 with **no red at all**. Replaced by the six above.

**Landing ledger — what A0 owed after S5, item by item.**

| Item | Status entering A0 |
|---|---|
| `UiClock` (AD1) — struct, three fields | **lands here.** Zero occurrences in the tree. |
| `UiClock` accessors + validated `set_max_delta` | **lands here.** Never specified; AD1 corrected 2026-08-26. |
| `ui_clock_tick` | **lands here.** Zero occurrences. |
| `UiAnimationPlugin` | **lands here.** Zero occurrences; `boyko_ui` has four plugins and none is it. |
| `UiAnimationSet` | **lands here.** Zero occurrences; `boyko_ui` has two `SystemSet`s, `UiBindSet` and `UiWidgetSet`. |
| The 100 ms clamp **value** | **already landed at S5**, as `pub const UI_FALLBACK_MAX_DELTA = 0.1` (`sprite.rs:320`) — public API. A0 **references** it (AD9 (3)); it does not restate it. |
| A clamp on a UI-consumed delta, at the one consumer | **already landed at S5** (`ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*), and gated three ways (G5-2). |
| `ui_sprite_flipbook` on `Res<UiClock>` | **lands here (A0b)** — it was in no ladder. |
| Retiring `UI_FALLBACK_MAX_DELTA` | **moves to a later rung** — whichever deletes the last reader; after A0b that reader is `UiClock::default()`. |
| The `flags` bit that makes `dt_real` reachable | **moves to A1**, and A1's Lands list must name it (AD9 (4)). |
| `dt_real`'s first actual consumer | **moves to A1** (the tween row). A0 lands the field with no reader — deliberately, and recorded here so it is not mistaken for a dead datum: its reader is one rung away and named. |
| M4's tween half | **moves to A1** (§4). |

**LANDING NOTE — A0 landed 2026-08-26 (A0a + A0b), worktree `D:/wt/ui`, branch `feat/ui-advanced`.**

*Landed set.* New module `crates/boyko_ui/src/animation.rs` — `UiClock` (three private `f32`,
`#[derive(Resource, Clone, Copy, Debug, PartialEq)]`), the three accessors, the validated
`set_max_delta` (out-of-line `#[cold] #[inline(never)]` panic, `Time::set_max_delta`'s idiom),
`Default` **referencing** `sprite::UI_FALLBACK_MAX_DELTA`, `ui_clock_tick`, `UiAnimationSet`,
`UiAnimationPlugin` (Main only, insert-if-absent — the `UiSafeArea` precedent). Registered in
`crates/boyko_ui/src/lib.rs` as `pub mod animation` and in the crate prelude.
**A0b:** `ui_sprite_flipbook` now takes `Res<UiClock>` and reads `dt_virtual()`; the inline `min`
is gone; the const, its doc, the system's `# The clock` section and the module's `# Ordering`
section are rewritten to AD9. Four schedule sites gained the resource and the tick, ordered ahead
of the flipbook: both `flipbook_schedule` helpers (`ui_s5_sprite_sheet.rs`,
`ui_flipbook_gpu_golden.rs`), `g5_3_the_churn_split_is_real`'s inline builder, and
`flipbook_only` (`ui_s6_authoring.rs`). No `src/` registration site existed and none was added.

*Documents in the landed set.* `docs/UI-PLAN-ANIMATION.md` (this rung), `docs/UI-PLAN-SPRITES.md`,
`docs/OPEN-QUESTIONS.md` and its `docs/ru/` mirror, and — **`docs/UI-PLAN-INTERACTION.md`**: ID12's
heading struck (it said *"on `Time`'s real delta"* while its own body chose `Time::delta_secs()`,
which is the VIRTUAL one) and the cross-plan clock row answered from AM7/AD9. *(Added 2026-08-26 at
the A0 verification. The edit itself was correct; it was simply absent from this record, and a
landed-set list that omits a file is how a reader concludes a change was never made.)*

*Gate.* `crates/boyko_ui/tests/ui_a0_clock.rs` — **10** tests, **identical names in debug and
release** (the count alone is not evidence: `running 6` has been equal over different sets on this
branch): `clock_tick_ran` · `clock_paused_advances_real_not_virtual` · `clock_clamps_a_hitch` ·
`clock_virtual_is_positive_clamped_and_scaled` · `plugin_adds_no_shared_schedule_surface` ·
`the_probes_are_not_vacuous` · `flipbook_reads_the_virtual_delta` ·
`a_consumer_after_the_set_observes_a_written_clock` · `the_ordering_probe_is_not_vacuous` ·
`a_host_configured_clock_survives_the_plugin`. Plus 5 unit tests in `animation::tests` (the
`Default`-references-the-const pin and four setter-validation legs) and the SHIPPED
`g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`, **re-run unedited and green** — leg 6's
whole statement.

*(The last three arrived 2026-08-26 at the A0 verification, which found two doc-comment contracts
with no red: `UiAnimationSet`'s membership and the insert-if-absent guard — reds 8 and 9. Leg 7
ships with its own non-vacuity control, `the_ordering_probe_is_not_vacuous`, which runs the same app
WITHOUT the `.after_set` edge and asserts the probe really does read a zero; without it the leg
would be green whether or not the edge did any work.)*

*Dead data removed at the same pass.* `UiAnimationPlugin::new()` — a `pub fn new() -> Self { Self }`
on a unit struct, with **zero callers anywhere** (the gate constructs the plugin by naming it).
Deleted rather than shipped, and the reason recorded at the type. ⚠️ **`UiBindingPlugin::new()` and
`UiWidgetsPlugin::new()` in the same crate are each exactly that same zero-caller constructor.**
They are pre-existing, A0 did not create them and does not touch them — recorded here so a later
reader sees a deliberate refusal to add a third copy rather than an inconsistency.
`UiClock::set_max_delta` is no longer test-only in the trivial sense either: leg 8 calls it in the
HOST shape the doc promises (configure, then `add_plugin`) and then reads the clamp back out of a
truncated hitch, so the setter is load-bearing for a gate rather than merely exercised by one.

*RED ledger — nine mutations, nine observed reds, every source restored byte-identically
(`cp` from a pre-mutation copy + `cmp`; SHA-256 re-checked).*

| # | Mutation | Predicted | OBSERVED |
|---|---|---|---|
| 1 | `dt_real` ← `time.delta_secs()` | leg 2 | leg 2 red — *"the REAL delta keeps advancing"* fired; legs 4 and 6 red too |
| 2 | `dt_virtual` ← `time.real_delta()` | legs 2 **and** 4 | both, as predicted: leg 2 `left: 0.016 / right: 0.0`; leg 4 `left: 0.08 / right: 0.04` |
| 3 | delete the `dt_real` clamp | leg 3(a) | leg 3(a) red, `left: 2.0 / right: 0.1` — the predicted 2.0 exactly |
| 4 | delete the `dt_virtual` clamp | leg 3(b) | leg 3(b) red, `left: 0.25 / right: 0.1` — `Time`'s own 250 ms showing through, the predicted plausible-looking wrong number |
| 5 | register on `CoreSchedule::Fixed` | leg 5, **both probes** | both flipped in one diff: `has_fixed_schedule: true` **and** `events_delivered: 0` vs `false` / `5` |
| 6 | flipbook reads `dt_real` | shipped G5-2 (b) **and** (c) | (b) red `left: 5 / right: 0`; (c) red `left: 4 / right: 2` — (c) reached by temporarily neutralising (b), since a `panic!` at (b) hides it; both restored byte-identically. `flipbook_reads_the_virtual_delta` red as well |
| 7 | drop `ui_clock_tick` from the plugin | leg 1 | leg 1 red, `left: 0.0 / right: 0.016` — the clock nobody wrote |
| 8 | delete `.in_set(UiAnimationSet)` | leg 7 | leg 7 red, `left: 0.0 / right: 0.016` — the downstream consumer's `.after_set` edge expanded to nothing and it ran ahead of the tick. Exit 101, `running 10`, 9 passed / 1 failed |
| 9 | unconditional `insert_resource(UiClock::default())` | leg 8 | leg 8 red, `left: 0.1 / right: 0.05` — the host's clamp replaced by the default, the predicted number exactly. Exit 101, `running 10`, 9 passed / 1 failed |

*M4 (§4), measured not argued.* At a synthetic 2 000 ms raw delta: `dt_real` **unclamped = 2.0 s**
(read off red 3's own assertion), `dt_real` **clamped = 0.1 s** (`== UiClock::max_delta()`, itself
`== UI_FALLBACK_MAX_DELTA`). Third number, not owed but measured by red 4 and worth the row:
`dt_virtual` with the UI clamp deleted reads **0.25 s** — `Time`'s clamp, four times the UI's.
So the UI clamp truncates the real delta by **20×** and the virtual delta by **2.5×** on that frame.

*Goldens.* All **ten** SHA-256 image pins re-run on the RTX 3060 with validation ON:
`ui_flipbook_gpu_golden` ×2, `ui_nine_slice_gpu_golden`, `ui_nine_slice_tiled_gpu_golden` ×2,
`ui_rect_gpu_golden`, `ui_rect_swapchain_golden`, `ui_sprite_gpu_golden`, `ui_text_gpu_golden`,
`ui_text_multiscale_gpu_golden`. **None moved; none re-blessed.** A clock rung must move no pixel,
and `dt_virtual` is `sprite.rs`'s pre-A0 arithmetic verbatim, so this is the expected result rather
than a lucky one.

> ⚠️ **The instrument this paragraph used to name is not the one that checks.** It read *"re-run
> with `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` (a skip is then a failure)"*. **MEASURED 2026-08-26 at the
> A0 verification: that variable is read in 3 of the 8 golden binaries** — `ui_flipbook_gpu_golden`,
> `ui_nine_slice_gpu_golden`, `ui_nine_slice_tiled_gpu_golden`, i.e. **5 of the 10 pins.** The other
> five (`ui_rect_gpu_golden`, `ui_rect_swapchain_golden`, `ui_sprite_gpu_golden`,
> `ui_text_gpu_golden`, `ui_text_multiscale_gpu_golden`) call `boot_or_skip`, which prints
> `SKIP <test>: …` to stderr and **returns `None` so the test exits 0** — the variable reaches none
> of them, and `ui_flipbook_gpu_golden`'s own header says so: *"`BOYKO_UI_GOLDEN_REQUIRE_DEVICE` is
> not a shared convention"*. **The check that actually distinguishes a device run from a vacuous one
> is ZERO `SKIP` LINES in the output**, which is what is asserted here: 0 across the 61-binary
> `boyko-render` run AND 0 across a dedicated serial re-run of all eight golden binaries
> (`--test-threads=1`, 11 device tests, every one `ok`, 1.4–2.7 s each — timings inconsistent with
> an early return). `git status` shows no image or hash artifact modified.

*Regression.* `-p boyko-ui --all-targets --no-fail-fast` → **323** passed / 0 failed over 49
binaries (320 + the three legs added at the verification); `-p boyko-render --lib --tests
--no-fail-fast` → 741 passed / 0 failed / 9 pre-existing ignores over 61 binaries, **0 `SKIP` lines**
(no device leg silently sat out). `ui_a0_clock` and `boyko-ui --lib` re-run in **both profiles with
NAMES compared, not counts** — `cargo test … -- --list` in debug and release, sorted and `diff`ed:
identical, 10 and 20. `ui_s5_sprite_sheet` 12/12, `ui_s6_authoring` 7/7. Root censuses green:
`engine_packages_census` (3), `goldens_pins_wellformed` (7), `gpu_blocking_reader_census` (2),
`internal_docs_anchors` (5), `trybuild_corpus_compiler_witness` (2), `vg_symbol_reachability` (16).
`cargo clippy -p boyko-ui -p boyko-render --all-targets -- -D warnings` green after `touch`
(15.4 s — not a sub-second false-fresh), and proven LIVE by injection: an unused local in
`ui_clock_tick` reds it with ``error: unused variable: `clippy_liveness_probe` `` at
`animation.rs:180:9`, **exit 101**; restored byte-identically (`cp` + `cmp`, SHA-256
`d50701920cc507c8ca4845ceb459c0f2257f164930725d5537c0da974db06b7f`) and re-run green.

*Findings this rung produced, recorded so no later rung re-discovers them.*

1. **The clamp gate caught a real defect before any red was applied.** The first clippy run reded on
   `Arc<Mutex<Option<Entity>>>` — the spawn-probe idiom the S5 harnesses use — because
   `ui_s5_sprite_sheet.rs` carries a file-scope `#![allow(clippy::disallowed_types)]` and the new
   file did not. Copying a harness idiom across files silently copies its **waiver requirement**.
   Resolved without an exception: `EcsMaster::run_system` returns the closure's own value, so the
   entity id comes back directly.
2. **"The four registration sites insert `UiClock`" under-counts if read as world builders.**
   `ui_s5_sprite_sheet.rs` builds five worlds and `g5_12` builds two of them inline; the resource
   was therefore inserted in the two `flipbook_schedule` helpers and the two inline builders — the
   four **schedule** sites — which covers every world by construction. Read as "the five
   `insert_resource(Time::default())` sites", the edit would have missed nothing in that file but
   would have had to be repeated per world; read as the four registration sites, it is exactly four
   edits. The rung's wording is right; this note pins which reading it is.
3. **A0b's own edits moved anchors this plan cites, and a PROSE MARKER is not a gate.**
   *(2026-08-26, the A0 verification.)* The UI plans are **structurally outside** `GATED_DOCS`
   (`FEATURE_MAP.md`, `SYSTEMS.md`, `ARCHITECTURE.md`, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`), so
   `internal_docs_anchors` reads none of them and every anchor here is hand-verified or nothing.
   Two distinct defects were found by that hand pass, and BOTH are the class that gate's own header
   denounces — *"a wrong anchor is worse than no anchor: it sends a reader to a plausible-looking
   but unrelated line"*:
   * **Coordinates into DELETED state.** `sprite.rs:306` was cited 11 times across four documents
     for the pre-A0b inline `min` that A0b deleted; `sprite.rs:306` is now a LIVE line
     (`/// # The clock (A0b — …)`), so the anchor resolved to unrelated, plausible-looking text and
     **only an italic prose marker separated them**. Same for `sprite.rs:292-301` / `:294-301`
     (the clock paragraph) and `sprite.rs:278`. All converted to prose naming the SYMBOL and no
     line — this campaign already ruled that a coordinate into deliberately-destroyed state is not
     an anchor. The pre-landing coordinates in the A0b Lands bullet went the same way.
   * **Coordinates into LIVE state that A0b's own +10-line harness edit shifted.**
     `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware` was cited at
     `ui_s5_sprite_sheet.rs:524` **four times across three documents** (this plan ×2,
     `OPEN-QUESTIONS.md`, and its `ru/` mirror); its `fn` is at **`:534`**, and `:524` lands on that
     same test's doc comment — the
     near-miss the gate's identity clause exists for. Its leg (c) range `:554-565` shifted to
     `:564-575` the same way. Re-pointed, because here a correct live target does exist.
     `animation.rs:228` (`impl Plugin for UiAnimationPlugin`, the convention the four sibling
     plugin anchors use) moved to `:231` when `UiAnimationPlugin::new()` was deleted, and is
     **`:402` as of 2026-08-28** (part 7, re-measured by content; the struct is at `:400`).
   Every remaining `.rs:N` in these documents that points into a file A0/A0b touched was resolved
   by hand afterwards: `sprite.rs:320` (`pub const UI_FALLBACK_MAX_DELTA`), `sprite.rs:329-349`
   (the AD9 clock section), `lib.rs:44` (`pub mod sprite`), `animation.rs:400`,
   `ui_s5_sprite_sheet.rs:154` / `:534` / `:564-575` / `:622`, `ui_flipbook_gpu_golden.rs:270`,
   `ui_s6_authoring.rs:363`. All land where they claim.

*Deviation from the rung as written:* none in substance. One mechanical addition — the plan named
four registration sites but not the ORDERING VERB; `.after(tick)` is used everywhere (rather than
`.before(flipbook)` on the tick) so the existing `.before(discovery)` edge is left untouched at
each site.

*Still open after A0, unchanged:* §7 Q1 (the 100 ms VALUES call — now answerable by editing one
`const` line, and `UiClock::set_max_delta` exists to override it per host, **the per-host route now
gated by leg 8** rather than merely asserted); the `flags` bit,
`dt_real`'s first production reader and M4b, all at **A1**; retiring `UI_FALLBACK_MAX_DELTA` at
whichever rung drops its last reader, which after A0b is `UiClock::default()` alone.

---

### A1 — the sink, the four channels, the fused tick — **size L** · *no cross-plan dependency*

*(rung amended 2026-08-27 at the A1 pre-build audit. **The storage kind of the sink was undecided at
the only rung that can decide it**, and four authorities gave three answers — D9 said dense
(`UI-ADVANCED-ARCHITECTURE.md:833`, now struck), AM2's cost model assumed dense, §6 R4 said table, the shipped
`gather.rs:121-122` said table and claimed this plan had been corrected to say so. It had not. The
ruling is **AM8 / AD10 — the sink is a TABLE component, the four channels stay dense** — and it is
stated in the Lands list below, which is the one place an implementer reads to write the derive.
Three further holes are closed with it: the composition base (**AD12**), the sink's equality
(**AD11**), and the storage lockstep at the pack-input macro (**AD13**). Gates and reds are rewritten
to A0's standard — every leg owns one.)*

*(**Ownership sweep, run 2026-08-27.** The rung id "A1" appears **zero** times outside this plan. The
one grep hit across the sprites, interaction, Aether, architecture, research and OPEN-QUESTIONS
corpus — `UI-PLAN-AETHER.md:139`, *"Decision A1: cross-construct …"* — is that plan's own DECISION
namespace colliding with this plan's RUNG namespace, not a reference to this rung. It is recorded
because a false positive in an ownership sweep is resolved by assuming coverage exists, and this
branch has already paid for the inverse error. The ARTEFACTS, unlike the rung, are known outside:
`crates/boyko_render/src/ui/gather.rs:121-122`, `crates/boyko_render/tests/ui_s0_discovery.rs:496` and
`crates/boyko_render/tests/ui_s0_measure.rs:245` all name `UiVisual`'s arrival, and the first states a
HARD CONSTRAINT on it that this rung's text did not carry until now.)*

**Lands — the sink.** `UiVisual`, **a TABLE component** (AM8 / **AD10**), 24 B (AM5), `#[repr(C)]` POD
`Copy`. Its derive list is deliberately short and each absence is a decision:

* **no `#[component(storage = "dense")]`** — AD10. A dense sink is invisible to
  `ui_render_discovery`'s `Or<…>` and every animation renders nothing (AM8, MEASURED).
* **no `#[derive(Default)]`** — AD6. A hand-written `impl Default` returning the new
  **`pub const UiVisual::IDENTITY`**, so gate 4 has two routes into the value rather than one.
* **no `#[derive(PartialEq)]`** — **AD11**. A hand-written **bytewise** `impl PartialEq`, because
  `set_if_neq` is the render gate's only throttle and `NaN != NaN` makes the derived form bump it on
  every frame forever (MEASURED: `[1,1,1]` derived vs `[0,0,0]` bytewise).

**Lands — the channels.** `TweenTint` / `TweenOpacity` / `TweenOffset` / `TweenScale`, all
`#[component(storage = "dense")]` `#[repr(C)]` POD `Copy` — **including the per-row `flags: u8`
whose bit 0 selects `dt_virtual` (D15's opt-in, AD1 reason (4), AD9 (4)); it was named once in this
plan and appeared in no landing list, and it is what makes `dt_real` reachable at all**. Dense is
safe here and nowhere else in this rung, for one stated reason: **nothing filters a `Tween*`.**
`AnyOf` forwards `HAS_DENSE` and `resolve_dense` to its arms (`data/anyof.rs:118`, `:137`); `Or` does
not (AM8). No `Tween*` is a member of `ui_pack_inputs!` and nothing reads `Changed<Tween*>`.

**Lands — four one-field wrapper bundles, one per channel.** *Not a style choice, and MEASURED one
file over:* dense storage SUPPRESSES the single-component `Bundle` impl the derive normally emits, so
`insert(TweenTint { .. })` is `error[E0277]: the trait bound TweenTint: Bundle is not satisfied` —
`crates/boyko_ui/src/sprite.rs:83-93` records exactly this for the crate's only existing dense
component, gated in `boyko_macros`' `component.rs` on `no_bundle || storage_bitset || storage_dense`.
A wrapper bundle is the only spelling that compiles. The companion fact is recorded at
`sprite.rs:103-112` and constrains this rung too: **`#[require(<a dense type>)]` PANICS at insert** —
the require pass resolves the required id's `ComponentPool` in the target archetype and a dense id
owns none — so no `#[require]` may point at a `Tween*`. (It may point at `UiVisual`, which is now a
table component; A1 does not use one, because the sink's insert is the helper's job, below.)

**Lands — the systems and the surface.** `ui_visual_tick` (AD5, **AD12** — the composition base is
`*sink`) · `ui_tween_reap` · `UiTweenScratch` (the retained completion list) · the public start/stop
helpers (`start_tween_tint(world, e, from, to, ms, easing)` and siblings), which **insert
`UiVisual::IDENTITY` if the entity has none** — stated because no rung said who inserts the sink, and
a channel whose sink is missing is a tween that ticks into nothing ·
`size_of`/`align_of`/`offset_of!` const-asserts on all five types, **plus the storage-kind
const-asserts of gate 11**.

Easing is **linear only** at this rung — `EasingId` exists as a field and the tick applies `t` — so
that A1's gates test the machinery and A2's gates test the curves, and a red at A2 cannot be blamed
on A1.

**Gate — eleven, and EVERY LEG OWNS A RED** (A0's standard — its red-mutation header reads *"nine, every one runnable AT THIS RUNG, and EVERY LEG OWNS ONE"*; A1 shipped three reds over eight
gates until 2026-08-27, and two of the three did not do what they said).
1. **Presence is running (C2/D9):** inserting `TweenTint` starts it; the reap removes it at
   completion; `DenseStore::live_count()` for each channel returns to **0** after the last tween ends.
2. **The tick bumps the sink's tick, on BOTH routes (AM1 + AM8).** A system running after
   `ui_visual_tick` with `Query<(), Changed<UiVisual>>` sees the row on an animating frame and does
   not on the frame after the reap — **and a second system with the discovery filter's own shape,
   `Query<(), Or<(Changed<C>, Changed<UiVisual>)>>`, sees exactly the same rows.**
   **`C` must be a component the fixture never writes** (a bare marker, or `ComputedRect` in a
   layout-stable fixture with the layout pass NOT registered). *Stated because getting it wrong makes
   this half a gate that cannot fail in the other direction:* if `C` is `Changed` on the frame under
   test, the `Or` is `true` through `C` and reports success whatever the sink's storage kind is. The
   arm exists only to make the filter an `Or` at all — a single-element `Or` would be a different
   type from the one `ui_pack_inputs!(changed)` expands to.
   *The second half is the storage gate, and it is why the first half alone was not one:* MEASURED,
   the two routes AGREE for a table sink (1 and 1) and DISAGREE for a dense one (1 and **0**), so a
   bare `Changed` is green under the storage error that makes every animation invisible. This is the
   only A1 gate that can see AD10, and A4 — where the symptom first appears — is a rung and a
   cross-plan dependency away.
3. **A rested element is silent (AM2):** an entity whose tween has completed keeps its `UiVisual` row
   and, on every subsequent still frame, is **not** `Changed<UiVisual>`. Assert with a live-row count
   of ≥1 and a changed-row count of 0 — the two together are what "rested but retained" means.
4. **The identity default, two routes (AD6):** `UiVisual::default()` equals `UiVisual::IDENTITY`,
   field by field, and `IDENTITY`'s four fields are additionally asserted against literals written
   into the test. ~~**and** `UiVisual` does not `#[derive(Default)]` (a compile-time absence, asserted
   the way `every_variant_states_its_own_answer_without_a_wildcard` asserts its own).~~ **Struck
   2026-08-27 — the named instrument does not do this and cannot be built.** That precedent
   (`crates/boyko_render/src/occlusion_config.rs:196`) is a WILDCARD-FREE MATCH OVER AN ENUM — *"adding
   a variant fails to COMPILE here … the compile error is the actual gate"* — and `UiVisual` is a
   struct with no variant set to exhaust. The property wanted was *"this `Default` was hand-written,
   not derived"*, and a derived and a hand-written `impl Default` are the SAME trait impl to the type
   system; no Rust test separates them (having both is E0119 in the implementation, not an assertion
   in a test). The closer precedent, `default_mode_is_off` (`occlusion_config.rs:155`), is a SECOND
   ROUTE into the value — which `UiVisual` now has, because AD6 lands `IDENTITY`.
5. **Arity-one per channel (D9 reason 3), on the property that is not already a kernel invariant:**
   `start_tween_tint` on an entity that already has one **replaces** `from`/`to`/`duration` and
   **restarts `elapsed` at 0**; `live_count()` does not grow. *(2026-08-27: the `live_count()` half
   alone was a tautology — every public insert route for a dense id goes through
   `DenseStore::insert_or_replace` (`commands/insert_command.rs:259`, `migration_helpers.rs:608`,
   `ecs_master/component_api.rs:470`), which *"overwrites the slot in place, keeping the slot VALUE
   assignment stable (no churn, the determinism contract)"* (`dense_store.rs:256-257`, verbatim), so
   `live_count()` — `column.count() - free.len()`, `:353` — is
   arithmetically incapable of moving and no implementation of `start_tween_tint` could have made it.
   The bare `DenseStore::insert` opens with `debug_assert!(!self.e2s.contains(…))` (`:186-190`) and is
   reachable only from fresh-entity paths (`clone/materialize.rs:893`, `spawn_batch_command.rs:592`).
   The replace-and-restart property is the one A3's reversing transition actually depends on, and it
   was asserted nowhere.)*
6. **Zero per-frame allocation** on the steady animating path — the crate's existing
   `zero_alloc.rs` / `p3_watch_zero_alloc.rs` harness shape, extended to the tick.
7. **Paused-clock leg (A0 leg 2, downstream):** with `Time` paused, a default-clock tween advances and
   a `virtual`-flagged tween does not. *(2026-08-26: the parenthetical is kept because this leg is
   genuinely downstream of A0 leg 2 — but it was ALSO being read as A0's coverage of `dt_virtual`,
   which A0 leg 2 does not provide. That hole is now closed at its own rung by **A0 leg 4**; this
   gate tests the `flags` SELECT, not the field's arithmetic. AD9 keeps `dt_real` the default of
   this lane, so gate 7's expectation is unchanged.)*
8. **Miri** over the tick + reap. ~~because dense insert/remove during a frame that also iterates the
   store is the one place this rung could be unsound.~~ *(2026-08-27: the struck reason names a hazard
   AD5's deferred reap **designs out**, so Miri passing over the shipped shape says nothing about the
   deferral — the classic shape of a gate whose subject stops being observable at the moment the rung
   succeeds. The live subjects, and what gate 8 must therefore exercise, are: (i) the retained
   `UiTweenScratch` reused across frames without a stale `(Entity, ComponentId)` surviving a despawn;
   (ii) a **remove-then-insert of the same channel on the same entity within one frame** — the reap
   frees the slot and a `start_tween_*` in the same frame reuses it, which is where a stale per-slot
   tick would leak (`dense_store.rs` re-stamps both ticks on a fresh insert, and that is the property
   under test); (iii) the tick's `AnyOf` fetch over four dense stores.)*
9. **The composition base (AD12) — NEW, and the first A1 gate that runs two channels.** A node whose
   `TweenOffset` finished at −400 px, then given a `TweenTint`: on the tint's first frame and every
   frame after, `UiVisual.offset_px[0]` is still **−400**. *(Gates 1–8 each exercise a single channel,
   so an identity-base implementation — which silently undoes every finished animation — passed all
   of them, and the defect would have surfaced first as an A3 transition resetting a slid-in panel.)*
10. **The sink's equality is idempotent (AD11) — NEW.** A *plateau* tween (`from == to`, and a
    duration long enough that the channel is still live on the frames under test — otherwise the reap
    fires, the row goes all-`None`, gate 3's `continue` takes over and gate 10 measures nothing) on a
    node whose `UiVisual` was inserted directly carrying one NaN field is **not** `Changed<UiVisual>`
    on any still frame after the first. MEASURED both ways: `[0,0,0]` bytewise, `[1,1,1]` derived. This is the release-side half of
    AM1's *"a tick that bumps every frame defeats the render gate as surely as one that never bumps"* —
    §5's `debug_assert!`s all compile out and the public helpers take author `from`/`to` values.
11. **The storage kinds are const-asserted (AD10) — NEW, and it is a compile-time gate.**
    `const _: () = assert!(!<UiVisual as Component>::STORAGE_IS_DENSE, "<rationale>")` plus
    `const _: () = assert!(<TweenTint as Component>::STORAGE_IS_DENSE)` and its three siblings — the
    `crates/boyko_render/src/occlusion_marker.rs:171-176` idiom verbatim, rationale string included.
    MEASURED at this audit: `STORAGE_IS_DENSE` is const-reachable and discriminating (`true` for a
    dense derive, `false` for a plain one), so this is a build error rather than a test.

**RED MUTATIONS — eleven, one per gate, each must be run and its red OBSERVED.**
1. Make `ui_tween_reap` skip the remove ⇒ **gate 1 reds** (`live_count()` never returns to 0).
   *(New 2026-08-27: the deferred reap is this rung's most novel machinery and nothing falsified it.)*
2. `Mut<UiVisual>` → `&mut UiVisual`, **as a PAIR of edits**: also replace `sink.set_if_neq(composed)`
   with `if *sink != composed { *sink = composed; }` ⇒ **gate 2 reds** (both halves).
   *(Corrected 2026-08-27. As a single edit it does not red gate 2 — it fails to COMPILE:
   `set_if_neq` is an inherent method on `Mut<'w, T>` (`data/mut_.rs:84`) and does not exist on
   `&mut T`, so the observation would be an E0599 proving only that a method name is spelled on one
   type and not another. The pair preserves the value-equality short-circuit so the mutation isolates
   the tick-bump axis and nothing else — which is the whole of AM1. MEASURED: `&mut` write ⇒ 0
   `Changed` rows; `Mut::set_if_neq` ⇒ 1.)*
3. **Delete the all-`None` `continue` AND replace `set_if_neq` with the plain `DerefMut` write**
   (`*sink = composed;`) ⇒ **gate 3 reds** (the rested element becomes `Changed` every frame).
   *(Corrected 2026-08-27. The original red — delete the `continue` alone — **cannot fire**, and
   AM2's own sentence says why: `set_if_neq` "saves the tick bump only because the value is
   unchanged". MEASURED, the mutated tick over still frames: `[0,0,0,0]` under a `*sink` base and
   `[0,0,0,0]` under an identity base. Either edit alone is silent — with the `continue` in place the
   deref write never reaches a rested row, and with `set_if_neq` in place the deleted `continue`
   writes nothing — so the red is the PAIR, which is exactly "AD5's two protections, both removed".
   The COST the `continue` saves is priced at A8 gate 3; the NaN containment it also provides is
   gate 10's subject under AD11.)*
4. Replace the hand-written `Default` with `#[derive(Default)]` ⇒ **gate 4 reds** (`opacity` 0.0 vs
   1.0, `scale` [0,0] vs [1,1]).
5. ~~Make `start_tween_tint` early-return when the channel is already present ⇒ **gate 5 reds**
   (`elapsed` does not restart and `from`/`to` are the old ones).~~ **UNWRITABLE — and gate 5's
   property is therefore recorded `NOT PROVED`.** *(Struck 2026-08-27 at the A1 remediation.
   `start_tween_tint`'s only world contact is `&mut Commands` (`crates/boyko_ui/src/animation.rs:876-877`,
   the `tween_helpers!` `$start` signature — `pub fn $start(` at `:876`, `cmds: &mut Commands` at
   `:877`; the anchor was `:730-742`, which was off by one at its START EDGE — it opened on a doc
   line — then `:799-807`, which was doc prose inside the same macro. Re-measured by CONTENT
   2026-08-28, part 7),
   and `Commands` declares **no** component or resource access at all —
   `crates/boyko_ecs/src/ecs/core/system/params/commands.rs:389-399`, whose own comment reads
   "SP1: `Commands` declares NO component / resource access". A helper that cannot read the world
   cannot branch on "the channel is already present", so this mutation cannot be written, let alone
   run. The mutation that SHIPPED in its place — make the restart resume at half phase — does red the
   test, but it edits the ONE insert path a fresh start and a restart share, so it falsifies the
   FRESH path and leaves the restart axis gate 5 exists for untouched. **No third substitute is
   offered, deliberately.** Replace-and-rewind is a KERNEL property of the only route out of the
   crate — `DenseStore::insert_or_replace` *"overwrites the slot in place, keeping the slot VALUE
   assignment stable (no churn, the determinism contract)"* (`dense_store.rs:256-257`) — so no
   implementation of `start_tween_tint` can produce a correct
   fresh insert and an incorrect restart. That is the same tautology this rung already recorded for
   the `live_count()` half, now shown to extend to `from`/`to`/`elapsed` as well.
   `restarting_a_channel_replaces_and_rewinds_it` stays as a REGRESSION assertion over the shipped
   behaviour and **must not be counted toward "every leg owns a red"**; an unprovable property stated
   as proved is worse than one stated as open.)*
   ⚠️ **The quotation in this strike was FABRICATED, and is corrected here and at the gate-5 note
   above (2026-08-27, third remediation round).** Both sites read *"overwrites the slot in place,
   keeping … the SAME slot"* under a `dense_store.rs:253-257` citation. **The string `the SAME slot`
   occurs nowhere in that range.** Lines 253-257 are the doc block quoted above; the phrase comes
   from `dense_store.rs:270`, a comment in the FUNCTION BODY — *"Present: drop the old value,
   overwrite in place at the SAME slot"* — so a paraphrase fusing two separate passages was
   presented inside quotation marks under a citation that contained only one of them. **The SUBSTANCE
   is unaffected: the slot assignment is stable either way, and gate 5's property is still a kernel
   tautology.** What failed is the evidence, not the conclusion — which is the harder failure to
   notice. *(Produced by a pass whose own subject was documentation accuracy. **Quote verbatim or do
   not use quotation marks.**)*
6. Put a `Vec::with_capacity(64)` behind a `black_box` in the tick body **or in the reap's loop
   body** ⇒ **gate 6 reds**. *(New 2026-08-27; corrected the same day, measured both ways.
   `Vec::new()` — the mutation this rung shipped with — is capacity-0 and never calls the allocator:
   MEASURED green, EXIT=0, so gate 6 could not see it. The `with_capacity` form additionally
   MEASURED green against the ORIGINAL `seeded_world()` window and red only after it was widened —
   that window ran two channels with zero completions, leaving the reap's loop body and the
   opacity/scale arms dead code. Reds after the widening: `baseline 6, pair 38` (reap loop),
   `baseline 6, pair 70` (`scale` arm).)*
7. **A0's struck red, adopted here — the rung it was deferred to.** Swap the default in the tween
   row's clock select (`dt_real` → `dt_virtual`) ⇒ **gate 7 reds** (a default-clock tween stops
   advancing while `Time` is paused). *(A0 struck this red as "structurally unrunnable at this rung"
   and pointed at "A1 gate 7"; A1's three reds never adopted it, so D15's `flags` bit and `dt_real`'s
   only reader had no red for a second rung running.)*
8. Make `ui_tween_reap` **drop the entry without clearing it from `UiTweenScratch`** ⇒ **gate 8
   reds** on leg (i): the next frame's reap replays a `(Entity, ComponentId)` for a row that is gone,
   and after a despawn + id reuse it removes a channel from an unrelated entity.
   *(New 2026-08-27, and deliberately NOT the "perform the removal inline in the tick" mutation an
   earlier draft of this line proposed. That one is not runnable and its stated mechanism is wrong:
   the removal needs `&mut EcsMaster` while the `Query` holds its borrow, so the mutation is a
   BORROW-CHECK error rather than a Miri red — the same failure mode as red #2's original single-edit
   form — and `dense_store.rs:186-190`'s `debug_assert!` guards insert-when-present, which is the
   START path, not a removal. **If an implementer finds the inline shape does compile, that is a
   finding and it escalates**, because AD5's reason for the deferral assumes it does not.)*
9. Base the composition on `UiVisual::default()` instead of `*sink` ⇒ **gate 9 reds**
   (`offset_px[0]` reads 0, not −400). *(New 2026-08-27, AD12.)*
10. Replace the hand-written bytewise `PartialEq` with `#[derive(PartialEq)]` ⇒ **gate 10 reds**
    (`[1,1,1]` instead of `[0,0,0]`). *(New 2026-08-27, AD11.)*
11. Add `#[component(storage = "dense")]` to `UiVisual` ⇒ **gate 11 reds at COMPILE TIME**.
    *(New 2026-08-27, AD10. **Substantively true and mechanically false as first written**, measured
    2026-08-27 at the A1 remediation: the line above also promised "**gate 2's `Or` half reds too**",
    and it cannot, because `error[E0277] … UiVisual: Bundle` prints FIRST and the crate does not
    compile — so gate 2 never runs. The `Or` half IS the best discriminator this rung has, and
    reaching it takes **three coordinated edits**: neutralize the const-assert, add a `Bundle`
    wrapper, change one fixture line. After those, leg 2 fails at its `Or` half while PASSING at its
    bare-`Changed` half, which is the A/B in one run — that is the observation the sentence was
    reaching for, and it is not a single mutation.)*
    **Second red, for the guard the remediation added at the OTHER end (`ui_pack_inputs!`):** add a
    DENSE component to `__ui_pack_inputs_list!` ⇒ `error[E0080]` naming it, traced through
    `crate::ui_pack_inputs!(assert_table)`. *Do NOT use "mark a listed member dense" as that red:
    MEASURED, `#[component(storage = "dense")]` on `StackIndex` produces two `error[E0277] …
    StackIndex: Bundle` at production insert sites in `boyko_ui`, `boyko-ui` fails, `boyko-render` is
    never checked, and the const-assert never evaluates — `grep -c "MUST be a TABLE component"` = 0.
    That is this same shadowing generalized across a crate boundary.* **And that is not a quirk of
    the chosen mutation — it is the assert's structural blind spot: flipping an EXISTING member is
    exactly the realistic regression, and the assert never evaluates for one that `boyko_ui` both
    defines and inserts directly. See the C7 block under the RED ledger for the full statement,
    including the measurement showing `UiVisual`'s own leg-11a const-assert (`components.rs:1117`)
    is what catches THIS member — the two guards are complementary, and only leg 11a covers that
    direction.**

**Measurement obligation — M4b (§4), carried here 2026-08-27 and SATISFIED 2026-08-27.** §4 moved
M4b to A1 on 2026-08-26 with the reason *"A0's text never carried the obligation at all"*, and A1's
text then did not carry it either — the same failure, at the receiving rung. It is a *reported
comparison*, not gate 7's pass/fail assertion on the same subject — §4 distinguishes the two
deliberately, and the sibling precedent for getting this axis wrong is AM2 (2), which found §10.5's
bench measuring the wrong axis as specified. ~~A1 does not close until M4b's PAIR of numbers is
written into this rung's landing note.~~ *(Struck: the pair is written, immediately below. Leaving a
live obligation standing next to its satisfied form is how this one got dropped twice.)*

**M4b is reported here** (§4), in A0's shape: a running tween's `elapsed` advance across ONE
synthetic 2 000 ms frame delta, clamped vs unclamped.

> **Clamped (`max_delta` 0.1 s): `dt_real` 0.1 s, tween `elapsed` advance 0.1 s.
> Unclamped (`max_delta` 1 000 s): `dt_real` 2 s, tween `elapsed` advance 2 s.**

The tween's advance tracks the clock's field **exactly** — that is the "visible" half M4 could not
show, and it sits against A0's already-recorded `dt_real` 2.0 s / 0.1 s pair (landing note above).
**20×**: unclamped, one alt-tab stall runs 2 s of animation in a single frame, so every transition
shorter than 2 s jumps straight to its end on resume instead of resuming mid-flight — which is the
user-visible thing §7 Q1's 100 ms is choosing between.

*Source and exact command* — `crates/boyko_ui/tests/ui_a1_tween.rs:1260`
(`#[test]` at `:1259`). The anchor read `:831`, then `:977`, then `:1117`, until 2026-08-28. ⚠️ **Two
of those three were stale as written and BOTH landed on plausible content, inside the doc block of
the same different test** — `a_zero_or_denormal_duration_snaps_to_the_endpoint` (`#[test]` `:1127`,
`fn` `:1128`): `:977` read *"Runs in BOTH profiles — every duration here is ACCEPTED"*, and `:1117`
now reads *"spelling that admits `+0.0` and refuses `-0.0`; `> 0.0` refuses both and"*. `:1117` was
correct when written on 2026-08-27 and was falsified the next day by part 7's own landing, +143.
That is the failure mode this document calls worse than landing out of range,
`m4b_the_clamp_is_visible_in_a_tweens_elapsed`:

```
cargo test -p boyko-ui --test ui_a1_tween m4b_the_clamp_is_visible_in_a_tweens_elapsed \
  -- --nocapture --test-threads=1
```

⇒ `test result: ok. 1 passed; 0 failed; … 10 filtered out`, EXIT=0 *(transcript as observed; the
debug binary has since grown to 12 names, so a re-run prints `11 filtered out` — the count is the
binary's, not this test's)*, and the four numbers printed
verbatim above. *(The test `assert_eq!`s all four, so a DIVERGENCE between the shipped behaviour and
this quoted pair is caught. **What is NOT mechanized is the pair reaching this prose** — and
deliberately so: a check for "does this paragraph contain a float" passes on any float, including a
wrong one, and would be another gate that cannot fail. This half stays documentary; the citation
anchor is what the repo's anchor discipline covers, and `docs/UI-PLAN-ANIMATION.md` is **not** in
`tests/internal_docs_anchors.rs`'s `GATED_DOCS` set, so even that is unenforced here — see the
landing note's "what is gated" row.)*

**LANDING NOTE — A1 landed 2026-08-27..28 in NINE parts, worktree `D:/wt/ui`, branch `feat/ui-advanced`.**
**Part 1 is the rung's code; parts 2–9 are one remediation each, one per adversarial pass, EIGHT of
them. None of the nine parts is committed.**
**Part 1 = the rung's code, commit `e7a16fd9`, which shipped with its red ledger and its adversarial
pass OWED and said so in its own message. Part 2 = the remediation both passes produced, written on
top of `e7a16fd9`. Part 3 = the remediation of PART 2, which a second adversarial pass produced —
three BLOCKING defects and six accuracy defects, one of them a behavioural REGRESSION part 2 itself
introduced.** They are recorded together because a landing note
split across a "we will do it later" is exactly the shape the M4b obligation just demonstrated: it was
dropped twice, each time at the boundary between the rung that owed it and the rung that received it.

> ⚠️ **This header read *"landed 2026-08-27 in three parts … None of the three parts is committed"*
> for six consecutive parts** — through 4, 5, 6, 7, 8 and into 9 — while the sections beneath it grew
> to nine. Every one of those parts ran an anchor sweep, and not one of them re-read the sentence
> that COUNTS them. **A landing note's own arithmetic is a claim like any other, and nothing was
> checking it** — the same shape as an anchor, one altitude up: a number written once, true once, and
> never re-derived. Corrected by part 9, 2026-08-28.

> ⚠️ **Part 3 is the part to read first, and its lesson is the campaign's.** Part 2 was itself a
> documentation-accuracy pass, run by a reviewer whose whole mandate was correcting false claims —
> and it shipped a behavioural regression (C1), a gate that could not fail (C2), a mandatory
> `#[allow]` rationale asserting a false impossibility (C3), and a **fabricated quotation** (C4).
> **A repair receives less scepticism than the original it repairs.** Nothing in part 2 was
> careless; every one of its defects survived because the repair itself was not treated as a new
> claim needing its own measurement.

*Landed set — part 1 (`e7a16fd9`).* `UiVisual` (the TABLE sink, AD10/AM8, with `IDENTITY` and the
hand-written `Default` and bytewise `PartialEq` of AD6/AD11) · the four DENSE channels `TweenTint` /
`TweenOpacity` / `TweenOffset` / `TweenScale` · the `start_tween_*` / `stop_tween_*` authoring
surface, macro-generated so four spellings are written once · `UiTweenScratch` (the retained
completion list, AD5) · `ui_visual_tick` (the fused four-`AnyOf` pass composing onto `*sink` — AD12 —
through `Mut::set_if_neq`) · `ui_tween_reap` (EXCLUSIVE, `.after(tick)`) · the `on_add` hook that
materializes the sink for a hand-inserted channel · `UiAnimationPlugin` extended from A0's one system
to three, with SET edges between them.

*Landed set — part 2 (the remediation, ten defects).* All in
`crates/boyko_ui/src/animation.rs` unless named otherwise; line anchors are as of this note and are
**not** anchor-gated (see "what is gated" below).

| Act | What | Site |
|---|---|---|
| F1 | `#[allow(clippy::type_complexity)]` + rationale on `ui_visual_tick` — the crate idiom, MEASURED at `e7a16fd9`: **19** `#[allow(clippy::type_complexity)]` sites workspace-wide (**20** with this one) and **0** `type` aliases for a `SystemParam`. ⚠️ **The RATIONALE this row first shipped was FALSE and is corrected below (C3)** — it claimed a `type` alias "loses the `SystemParam` impl", and that claim is refuted in one edit | ⚠️ **LIVE pointer — re-read by CONTENT every round, never offset.** `animation.rs:688-700`: the rationale comment `:688-699` and the `#[allow]` `:700`, with the query it suppresses at `:701` (`pub fn ui_visual_tick(`). It has read `:592-603`, then `:603` with the query at `:606`, then `:634`, then `:654`/`:657` (H6), and part 7's landing moved it a **fourth** time. See H6 |
| F9-a | A **release-active refusal** of a degenerate `duration_ms`, replacing a `debug_assert!` that compiled out. `#[cold] #[inline(never)] invalid_tween_duration` — the `layout.rs` `depth_clamped` precedent (refuse + degrade), NOT `UiClock::set_max_delta`'s panic, because this is the gameplay path and a panic in a UI system under the windowed runner hangs the window with no message. ⚠️ **Shipped as "non-finite / non-positive" with the predicate `is_finite() && > 0.0`, which was a REGRESSION (C1) and is corrected below**; the shipped predicate is `is_finite() && is_sign_positive()` | `animation.rs:177-303` — the WHOLE doc comment of `invalid_tween_duration`; `:304` is `#[cold]`, the first line of the item itself. ⚠️ The range read `:177-225` until 2026-08-27 and was **MIS-CUT AT ITS END**: `:225` ended a sentence but `:226` continued the SAME paragraph, which is this landing's own stated rule broken by the landing. It then read `:177-271` / `:272` until 2026-08-28, when part 7's census disclosure grew the same doc comment by 32 lines and mis-cut it at its end AGAIN — **the start `:177` has been right through all three spellings and only the END has ever been wrong.** Call site `:885-888` (the guard, the call `:886`, the `return`; the anchor first read `:739-742` **and was off by one at both edges**, then `:808-811`) |
| F9-b | `advance`'s completion test inverted to `if t < 1.0 { Some(t) } else { None }`, putting NaN on the COMPLETING side. MEASURED free: **14.002 vs 14.008 ns/row** (4096 nodes × 4 channels, release, floor of five process invocations). ⚠️ **This fix OVERLAPS the F9-a guard on the NaN member, and the overlap is what made C2's gate vacuous** — see the corrected red ledger | `animation.rs:583` (the test itself; its doc paragraph is `:551-555`. Was `:457`, then `:486`; re-measured by CONTENT 2026-08-28, part 7) |
| F9-c | The falsified "bounded damage" paragraph on `advance` | `animation.rs:557-562` (was `:439-450`, then `:474-479`, then `:494-499`; re-measured by CONTENT 2026-08-28, part 7) |
| F7-a | The generation-free scratch key's real safety argument — three measured facts, replacing one sentence that ruled out a window nothing was ever going to use | `animation.rs:438-468` (the `# Why the generation-free key is safe` block; `:469` is `#[derive(Resource, Default)]`. Was `:326-356`, then `:355-386`, then `:375-406`; re-measured by CONTENT 2026-08-28, part 7) |
| F7-b | `entity_ids_are_not_recycled_today` — the trip-wire under fact 2 | `crates/boyko_ui/tests/miri_a1_tween.rs:137-178` |
| F2-i | The ordering CONTRACT, stated at four sites | `animation.rs:60-66` (module, appended — A0's clock sentence at `:51-58` deliberately untouched, it is TRUE for `Res<UiClock>`), `animation.rs:657-687` (`# Ordering` on `ui_visual_tick`; `:688` begins the `#[allow]` rationale. Was `:531-555`, then `:560-590`), `animation.rs:350-362` (`UiAnimationSet`'s doc; `:363` is `#[derive]`, `:364` the struct. Was `:246-250`, then `:267-279`), `crates/boyko_render/src/ui/gather.rs:506-520` (the reciprocal, the only site that can name both; was `:486-500`) |
| F2-ii | The ordering GATE | `crates/boyko_render/tests/ui_a1_sink_reaches_discovery.rs` |
| F2-iii | `sprite.rs`'s `# Ordering` — the measurably false "a repaint one frame late" | `crates/boyko_ui/src/sprite.rs:36-63` |
| F3 | `miri_a1_tween.rs` leg (i): the queued pair names `TweenTint`, so the assertion now watches `TweenTint`; the "a fresh entity very likely reuses `a`'s id slot" comment deleted as measurably false; the survival half demoted to SMOKE and the DRAIN named as leg 8's gate | `miri_a1_tween.rs`, header row in `crates/boyko_ui/tests/ui_a1_tween.rs:18` |
| F4 | Gate 6's measured WINDOW widened — two cohorts (steady + completing), all four channels, warm/armed split, per-channel census. The old window ran two channels with ZERO completions, so the reap's loop body and the opacity/scale arms were dead code inside it | `crates/boyko_ui/tests/ui_a1_zero_alloc.rs` |
| F5 | The two byte-identical doc blocks a commit that TRIPLED the set left unchanged, plus the `.after_set` note | `animation.rs:350-362` (`UiAnimationSet`'s doc, 13 lines — `:363` is `#[derive(Clone, Copy, Debug)]`, so both ends hold), `:367-398` (`UiAnimationPlugin`'s doc, **32 lines**; `:399` is `#[derive(Default)]`) — ⚠️ **the second range's END was WRONG and had never been checked: the row said `:367-371`, "5 lines", which is the first paragraph only, and the doc runs on through `# Containment` and `# No `new()``. Re-measured by CONTENT at BOTH ENDS 2026-08-28, part 10.** Both prior sweeps re-aimed this pair's START and carried its END unexamined, which is the failure mode this campaign's own rule — *check BOTH ends of every range* — exists for, found on a row that carried a by-CONTENT annotation through three parts — both were `:238-250` / `:255-259`, then `:267-279` / `:284-288`, then `:287-299` / `:304-308`; re-measured by CONTENT 2026-08-28, part 7 — `sprite.rs:341-349` (was `:338-345`; **verified correct at both ends by CONTENT 2026-08-28** — `:341` is *"defects S5 measured and AD9 ruled on. Register"* and `:349` *"than animating on a stale zero."*, the `.after_set` note this row is about) |
| F6 | `ui_pack_inputs!(assert_table)` — one `const _: () = assert!(!STORAGE_IS_DENSE)` per list member, so the macro's TABLE-only promise is a compile error instead of prose | `gather.rs:147` (invocation), `:166-169` (arm doc), `:184-186` (arm), `:211-230` (applier), `:84-119` (the prose, rewritten into a pointer at the assert **plus** the coverage disclosure C7 required) |
| F10-d | `ui_a1_tween.rs`'s header table credited `the_or_arm_is_not_vacuous` as co-owner of leg 2's red; it is leg 2's harness CONTROL and no A1 mutation can reach it | `ui_a1_tween.rs:9-21` (the table), `:23-28` (the control note), `:30-46` (leg 5, added by the doc act) |
| F8 / F10-a/b/c / F4-e | This document (the M4b report above, mutations 5, 6 and 11, and A4's blocking precondition) | `docs/UI-PLAN-ANIMATION.md` |

*Landed set — part 3 (the remediation OF part 2; three blocking, six accuracy).* The second
adversarial pass reproduced every one of part 2's own hardest refutations and confirmed F2/F3/F4/F5/F6/F7
stand — **and then found that part 2 had introduced a behavioural regression, a gate that cannot fail,
a false mandatory rationale and a fabricated quotation.** Anchors are as of this note.

| Act | What | Site |
|---|---|---|
| **C1 — BLOCKING, a REGRESSION** | F9-a's predicate `is_finite() && duration_ms > 0.0` refused `+0.0`, because `0.0f32 > 0.0` is false. MEASURED, release: pre-guard a zero-duration tween SNAPPED to its endpoint; under the shipped guard it created **no row and no `UiVisual` at all** — a node authored with a zero duration silently never animates, reachable from the safe `pub` API (`duration * speed_multiplier` with a zero multiplier). Denormals were unaffected, which is what hid it. **Fixed on the INPUT, not the derived reciprocal:** `is_finite() && is_sign_positive()` — `+0.0` snaps, `-0.0` stays refused, and the check stays where the value enters (the stance `UiClock::set_max_delta` already takes in this file) | `animation.rs:885` (the predicate; was `:788`), `:848-866` (the `$start` helper's `# A degenerate `duration_ms` is REFUSED, in release too` section; was `:751-769`) and `:177-303` (`invalid_tween_duration`'s doc; was `:177-225`) — both doc blocks, rewritten to a per-shape measured table. All three re-measured by CONTENT 2026-08-28, part 7 |
| **C2 — BLOCKING, a gate that could not fail** | `a_degenerate_duration_creates_no_row`'s **NaN arm passed with the guard disabled**: the row completed, assigned `to = 1.0`, and the fixture's `1.0` IS `UiVisual::IDENTITY.opacity`, so both assertions held. The sink assertion carried no independent weight either — `unwrap_or(IDENTITY) == IDENTITY` cannot separate "no sink" from "sink == IDENTITY". **Fixed three ways:** the endpoint moved to `0.25` (so a completed tween and an absent sink are different readings), the row's absence is asserted BEFORE the first tick, and the sink is asserted `is_none()`. `-0.0` added — it had never been enumerated. Red re-run: the panic is now at the **first** iteration (`NaN`) | `ui_a1_tween.rs:786-788` (the gate — `#[cfg(not(debug_assertions))]`, `#[test]`, `fn a_degenerate_duration_creates_no_row`). ⚠️ This read `:763` until 2026-08-27, which is a **bare empty `///`** inside the doc block and not the gate at all; `:690-785` (its doc — the range read `:690-712`, which ended MID-SENTENCE, `:713` continuing it) |
| **C2 — the accepting side, NEW** | `a_zero_or_denormal_duration_snaps_to_the_endpoint` holds `+0.0` / `1e-40` / `f32::MIN_POSITIVE` on the accepting side. Runs in **both** profiles (every duration in it is accepted, so the `debug_assert!` is never reached) | `ui_a1_tween.rs:1128` (`#[test]` `:1127`, doc `:1098-1126`) — re-measured by CONTENT 2026-08-28, part 7; the row read `:845`, which is today a doc line of the over-ceiling gate |
| **C3 — BLOCKING, a false mandatory rationale** | F1's `#[allow]` justified itself with *"so it cannot be a `type` alias without losing the SystemParam impl"*. **Refuted in one edit:** the alias compiles, `-D warnings` EXIT=0, and the lib registers the system itself, so the impl survives — type aliases are transparent. The DECISION to suppress stands; the REASON did not, and this repo makes the rationale mandatory. Rewritten to the honest one (it hides the access set, and the alias must spell three lifetimes the inline signature elides). ⚠️ **That replacement rationale was MEASURED at `animation.rs` ONLY, and part 4 below retracts its propagation to the other three sites** — see D3 | `animation.rs:688-699` (the rationale; `#[allow]` at `:700`, the query at `:701` — re-measured by CONTENT 2026-08-28, part 7; the row read `:623-633` / `:634`, and `:592-603` before the part-4 landing moved it) **plus the three pre-existing copies of the same false sentence**: `layout.rs:81-87`, `text/measure.rs:54-64`, `gather.rs:522-532` (all three re-anchored and rewritten in part 4) |
| C4 — a FABRICATED QUOTATION | *"overwrites the slot in place, keeping … the SAME slot"* attributed to `dense_store.rs:253-257`. The phrase occurs nowhere in that range — it is from `:270`, a body comment. Corrected at both sites with the verbatim text | this document (mutation 5 and the gate-5 note) |
| C5 — a citation to prose | `schedule.rs:152` was cited as the half-open-window MECHANISM; it is a doc comment about gated-system dispatch that merely contains the notation. The exact sites are `:288` (`bump_change_tick`) and `:342` (`set_change_ticks`); the comparison is `change_detection/tick.rs:169-171`, consumed at `filter.rs:1205/1225/1493/1503`. **The semantics is real and IS measured by the gate — only the pointer was to prose** | `animation.rs:662-671` (the `Schedule::run` paragraph carrying the `:288` / `:342` sites and the `schedule.rs:152` disclaimer; was `:565-574`, re-measured by CONTENT 2026-08-28, part 7), `sprite.rs:41-48` (**verified correct at both ends by CONTENT 2026-08-28**), this document, `docs/OPEN-QUESTIONS.md` + its `docs/ru/` twin |
| C6 — an over-credited assertion | F2's gate said it sees the sink write on "every one of the 20 animating frames"; frame 1 is true through BOTH `Or` arms (the spawn stamps `ComputedRect`), so only **19** are attributable to the sink. Now asserted frame-by-frame over `[1..20]` — the SAME window the two controls prove inert — with frame 1 asserted separately and credited to neither arm. Strictly stronger than the retired `sum() == 20`, and it mirrors the reader-first arm's `&[0; 19]` | `ui_a1_sink_reaches_discovery.rs:220-238` |
| C7 — two over-claims about F6 | The const-assert's blind spot is structural, not a bad mutation, and it guards one of five `Or` lists. Both recorded; **the pass's own second claim needed correcting in turn** — flipping `UiVisual` to dense emits TWO errors, and the `E0080` comes from `components.rs:1117` (leg 11a's own guard), not from `gather.rs`'s `assert_table` | `gather.rs:95-119`, and the C7 block + "what is NOT proved" items 7-8 above |
| C8 — two anchors off by one at an edge | `animation.rs:730-742` opened on a doc line; F9-a's call site was `740-743` cited as `739-742`. Both re-anchored, along with **every other `file:line` in the part-2 table**, which the C1/C3 landings moved by 20-50 lines | this document |
| C9 — a heading contradicting its own table, found while closing C4-C8 | `invalid_tween_duration`'s doc read **"# The six refused shapes, MEASURED"** over a FIVE-row table, two lines above its own sentence "Four of those **five** are immortal". The taxonomy the same landing established is **six degenerate shapes, of which five are refused and four are immortal** (`ui_a1_tween.rs:748` states exactly that) — so the heading was the one place that said six *refused*. Corrected to five; a companion "the worst shape of the six" was disambiguated to "the worst of the five refused shapes", which was ambiguous rather than false. **Found by the pass that was closing part 3's OWN accuracy debt, in text part 3 had just written** | `animation.rs:185`, `ui_a1_tween.rs:712` |
| **C9 — AMENDED by part 4 (D4): the original was ambiguous, the REPLACEMENT was FALSE** | The disambiguation this row records did not merely narrow the superlative — it landed a new one, *"`-0.0` … is the worst of the five refused shapes"*, and that is refuted by arithmetic. Under the only criterion the render gate cares about — **how many frames bump `set_if_neq`** — `-0.0` yields `inv = -inf`, so `t = -inf` is CONSTANT from frame 1 and the sink bumps **zero** times after the first, tying it for LAST with `+inf` (`t = 0`) and `-inf` (`t = -0.0`). The worst is the negative finite shape `-100.0` (`inv = -10`), whose `t` diverges (−0.25, −0.5, −0.75, −1.0, −1.25) and which therefore bumps **every** frame. **The replacement was false because no criterion was named.** A superlative without a stated criterion is not "ambiguous" — it is unfalsifiable, and unfalsifiable is precisely what let a wrong one land unchallenged through two passes. Part 4 states the criterion at the site and deletes the false claim | `ui_a1_tween.rs:696-697` (criterion named), `:712-717` (false superlative deleted, replaced by the tie) |

**Anchor sweep, run for part 3.** Every `file:line` in the two tables above was re-read at its target
and checked by CONTENT, not by arithmetic. **Every one of the part-2 table's `animation.rs` anchors
had rotted** (the file grew ~50 lines in part 3 alone), and the F6 landing had silently moved four
more citations: `gather.rs:91-93` (three sites in this document, one in each `OPEN-QUESTIONS.md`, and
**one in `docs/UI-ADVANCED-ARCHITECTURE.md:854` that a three-file grep did not see**),
`gather.rs:96`, `gather.rs:84-89` and the M0-c red's `gather.rs:128-129` — the last of which had
drifted onto the member LIST and would have read as correct to anyone who opened it. **A rotted
anchor that lands on plausible content is worse than one that lands out of range.** Two historical
transcripts are deliberately left at their as-observed coordinates and annotated instead of
rewritten: F1's red-ledger `animation.rs:565:12` and M4b's `10 filtered out`. **A red ledger is a
transcript; re-coordinating one without re-running it is the C4 class.** *(Part 5 note: there is a
**third** transcript under the same rule — the `E0277 UiVisual: Bundle` at `animation.rs:457` cited
in `crates/boyko_render/src/ui/gather.rs:113` and again at this document's `:1696` (**part 6:** this
said `:1656`, a bullet in this very discussion — plausible content, 8 lines short; part 6's own
replacement `:1664` was then 7 lines short after part 7's edits above it, and `:1664` today is the
`E0080 … UiSpriteCursor` line of the SAME transcript block — plausible content a third time. ⚠️ **Part
10: `:1673` had rotted a FOURTH time and this time OUT of content — read `:1673` on the part-10 tree
and it was a BLANK line**, which is the one failure mode this chain had not yet produced and the only
one a reader notices unaided. ⚠️ **Part 11 re-read `:1673` and even that had rotted**: on the tree
part 11 inherited it was `the clippy masking below.`, with the nearest blank one line down. *A record of where a
stale anchor USED to point is an anchor, and it decays at the same rate as the pointer it documents —
which is why this sentence now names the tree it was read on.* Read at its target 2026-08-28
(part 11): `:1696`, `` `cargo check -p boyko-render --lib`: **two** errors, `E0277 UiVisual: Bundle` at ``.
Four values for one pointer across five parts. ⚠️ **And the disclaimer about `:1656` was itself
false — part 11 read it.** Part 10 wrote that `:1656`, the value H5 below quotes, *"is today the F2
ordering-axis red-ledger row"*. Read at `:1656` on the tree part 11 inherited: *"rather than being
restated here — **this note did not re-run them.** What it DOES pin is the part"* — prose about a
remediation note, in no ledger, with the F2 ordering-axis row nine lines further down. **A pass that re-aims an anchor and
describes the discarded value in the present tense has written a second anchor and checked
neither**). It was
observed in a MUTATED tree, which is why it does not resolve in the clean one. Part 5 added 20 lines
and part 6 a further 18 above `animation.rs:244`; do NOT therefore add 38. A transcript moves only by
being RE-RUN and a live pointer only by being RE-READ — part 6's H6 is the measurement of why.)*

*Documents in the landed set.* `docs/UI-PLAN-ANIMATION.md` (this rung), `docs/OPEN-QUESTIONS.md`
with its `docs/ru/` mirror, and — from part 3 — one line of `docs/UI-ADVANCED-ARCHITECTURE.md:854`,
the fifth copy of the `gather.rs:91-93` anchor. **`docs/UI-PLAN-ANIMATION.md` has no `docs/ru/` twin
and never had one** — that directory holds `OPEN-QUESTIONS.md` and `README.md` only, so the
same-commit twin rule binds on `OPEN-QUESTIONS.md` alone here. Part 3's twin edit is **5 hunks on
each side, in the same order**: item 1's mechanism pointer + the 19-of-20 attribution, item 2's NaN
and `+0.0` corrections, item 5's "two gates" → "three gates", item 5's new third bullet, and the
2026-08-21 entry's `gather.rs` anchor. Structural parity re-counted after the edit: `##` 78/78,
`###` 55/55, `> ⚠️` 4/4 on both sides.

*Gate — the names, not the counts, because equal counts over different SETS is this branch's measured
vacuity shape.* Enumerated with `-- --list | sort` and run unpiped with `$?` on the next line:

| Binary | Names | Result |
|---|---|---|
| `boyko-ui` `ui_a1_tween` (debug) | ~~11~~ **12** | `running 12` · `ok. 12 passed` |
| `boyko-ui` `ui_a1_tween` (release) | ~~12~~ **13** — the same 12 **plus** `a_degenerate_duration_creates_no_row`, which is `#[cfg(not(debug_assertions))]` because the debug build asserts | `running 13` · `ok. 13 passed` |
| `boyko-ui` `ui_a1_zero_alloc` | 1 | `running 1` · `ok. 1 passed`, debug **and** release. `--test-threads=1` is load-bearing: the file arms a process-global allocator |
| `boyko-ui` `miri_a1_tween` | 4 (was 3 — `entity_ids_are_not_recycled_today`) | `running 4` · `ok. 4 passed` |
| `boyko-render` `ui_a1_sink_reaches_discovery` | 3 (was 2 — `the_reader_must_be_ordered_after_the_tick_or_every_write_is_lost`) | `running 3` · `ok. 3 passed` |
| `boyko-render` `ui_s0_discovery` / `ui_s0_seam` | 2 / 6, unchanged | `ok. 2 passed` / `ok. 6 passed` |

> The two `ui_a1_tween` counts moved **+1 each** in the third remediation round (part 3 below):
> `a_zero_or_denormal_duration_snaps_to_the_endpoint` runs in BOTH profiles. **RE-ENUMERATED
> 2026-08-27 by NAMES, not counts** — `cargo test -q -p boyko-ui --test ui_a1_tween -- --list`,
> `grep ': test'`, `sort`, `diff`: debug **12**, release **13**, and the diff is exactly the one
> line `a_degenerate_duration_creates_no_row`. `miri_a1_tween` 4, `ui_a1_zero_alloc` 1 and
> `ui_a1_sink_reaches_discovery` 3 re-enumerated the same way and unchanged.

`cargo clippy --workspace --all-targets --keep-going -- -D warnings` ⇒ **EXIT=0**, 0 warnings,
0 errors, `Checking boyko-render` present, run after `cargo clean -p boyko-ui -p boyko-render`
because clippy is FALSE-FRESH on this tree.

*RED ledger.* A1's own eleven-gate ledger was produced by the red-ledger + adversarial passes run
2026-08-27 against `e7a16fd9`; its headline is **8 gates red-proven by an independent re-run, 14
claims corroborated, 2 diagnosed greens**, and its per-mutation artefacts live in that pass's record
rather than being restated here — **this note did not re-run them.** What it DOES pin is the part
that CHANGED: the remediation’s own reds, each applied, run unpiped with `$?` captured on the next
line, and restored by `cp` from a snapshot taken first with `cmp` verified afterwards.

| Subject | Mutation | OBSERVED |
|---|---|---|
| **F1 — the mandated linter gate** | drop the `#[allow]`; `cargo clean -p boyko-ui`; `cargo clippy --workspace --all-targets --keep-going -- -D warnings` | **EXIT=101**, `error: very complex type used. Consider factoring parts into type definitions` at `crates\boyko_ui\src\animation.rs:565:12`, and `Checking boyko-render` = **0**. Restored ⇒ EXIT=0, 0 warnings, 0 errors. *(Re-observed independently for this note, 2026-08-27, cache-matched with `cargo clean -p boyko-ui` before BOTH runs.* ⚠️ *`565:12` is the line AS OBSERVED, against the pre-C3 file. The file has moved under it repeatedly since. **RE-MEASURED BY CONTENT 2026-08-28** (part 8): the `#[allow(clippy::type_complexity)]` is at **`animation.rs:700`** and the query it covers — `mut q: Query<(` inside `ui_visual_tick` — begins at **`:703`**. Both are unique in the file (`grep -n` returns one line each). ⚠️ **This is a LIVE pointer, not a transcript coordinate, and it must be RE-READ every round rather than offset.** It read `:603`/`:606` from part 3, then `:672`/`:675` from part 6 — and part 6's own pair was **already 28 lines stale when part 7 landed the census on the same file**, `:672` reading *"So a write stamped in frame N carries frame N's tick, and a reader ordered"*: plausible content, the failure mode this very document calls worse than landing out of range. Part 6 was 31 lines stale when part 5 inherited it, then 51, and part 5's blanket "+20 for everything below `animation.rs:235`" doctrine would have produced `:623` — still wrong. An offset repairs a transcript, never a live pointer. **A live pointer inside a red-ledger row is the one thing in these tables that no sweep can skip, and three consecutive parts have now had to re-read this one.** **The row itself is NOT re-run at these lines** — a red-ledger artifact is a transcript, and rewriting its OBSERVED coordinates without re-running it would be the fabrication class C4 records.)* |
| F9 — the refusal | revert F9-a's `return` | ⚠️ **THIS ROW WAS MIS-ATTRIBUTED AND IS CORRECTED (C2).** As shipped it read **EXIT=101**, `"duration_ms = NaN: the helper REFUSED the tween…"`, `left: 1 right: 0`. **That artifact cannot come from this mutation.** Re-run of the declared mutation against the then-shipped gate produced the panic at the **`inf`** arm — the loop's SECOND iteration — because the NaN arm could not fail: F9-b's own inversion puts a NaN `t` on the completing side, so the row completed, assigned `to = 1.0`, and the fixture's endpoint `1.0` **is** `UiVisual::IDENTITY.opacity`, making both of that arm's assertions true with the guard disabled. The quoted string could only have come from reverting F9-a **and** F9-b together. **Both halves are fixed in part 3**, and the row's current OBSERVED is below |
| F9 — the inversion, ALONE | revert `t < 1.0` → `t >= 1.0` with a hand-built NaN row | **EXIT=101**, `"a NaN t lands on the COMPLETING side…"`, `left: 1 right: 0` |
| F2 — the ordering axis | (1) give the reader-first arm the after-edge · (2) take the edge off `schedule()`'s reader | **EXIT=101** both. (1) `left: [1×19] right: [0×19]` — unaffected by C6, the before-arm assertion did not change. (2) **RE-RUN 2026-08-27 against the C6-corrected gate**, because C6 changed that assertion's SHAPE and the old artifact (`left: 1 right: 20`, from the retired `after.iter().sum() == 20`) can no longer be printed: `ui_a1_sink_reaches_discovery.rs:227:5`, `left: [0×19] right: [1×19]`, with the run's own stdout `or=[1, 0×19]` for both arms. Source restored by `cp` from a snapshot taken first; `cmp` identical, SHA-256 `4b2cac6f…` identical |
| F4 — gate 6 | `Vec::with_capacity(64)` behind a `black_box`, in the reap loop · in the `scale` arm | **EXIT=101** both: `baseline 6, pair 38` · `baseline 6, pair 70`. **Both were first confirmed GREEN against the un-widened window** — that is what makes the widening a fix rather than a tidy-up |
| F3 — leg 8's drain | delete `done.clear()` | **EXIT=101**, `"drained in its own frame"`, `left: 1 right: 0` |
| **F6 — the storage guard** | add a DENSE component (`UiSpriteCursor`) to `__ui_pack_inputs_list!`; `cargo check -p boyko-render --lib` | **exit 101**, ``error[E0080]: evaluation panicked: ui_pack_inputs! member `:: boyko_ui :: components :: UiSpriteCursor` MUST be a TABLE component…`` at `gather.rs:199:27`, traced through `crate::ui_pack_inputs!(assert_table)` at `gather.rs:128`, alongside the pre-existing M0-c arity red `error[E0308]` at `gather.rs:351`. Source restored by `cp` from a pre-mutation snapshot, `cmp` identical, SHA-256 identical, `cargo check` back to EXIT=0. *(Re-observed independently for this note, 2026-08-27.)* ⚠️ *The three `gather.rs` coordinates above are **AS OBSERVED**, and the file has moved under them since. Today: `:199` is **blank** (content length 0) — the `assert!` this `E0080` names is at `gather.rs:218:27`, same column, +19 lines; `crate::ui_pack_inputs!(assert_table)` is at **`:147`**, not `:128` (`:128` is macro internals, `$crate::$apply! { $extra [`); and the M0-c arity destructuring is at **`:370-371`**, not `:351` (`:351` is `// UI-ADVANCED S5: the sheet table, read ONCE per gather — not per node, so` — **plausible content**, the worse of the two drift failure modes, and the one this rung's own anchor sweep named as worse than landing out of range). **This row is NOT re-run at those lines**, same rule as F1 above: a red ledger is a transcript, and re-coordinating one without re-running it is the C4 fabrication class. Verified by content 2026-08-27, part 4.* |
| F7-b — the trip-wire | **none, on purpose** | A KERNEL-PROPERTY gate: its red is the fact changing. Non-vacuity was shown instead by inverting the predicate — first batch `[EntityId(0..5)]`, second `[EntityId(6..11)]`, recycled `[]` |
| F5 | **none available, and none invented** | Doc-only. Its evidence is `git show e7a16fd9 -- crates/boyko_ui/src/animation.rs` showing both blocks byte-identical across the commit that tripled the set |

**⚠️ Do NOT use "mark a listed member dense" as F6's red.** MEASURED: `#[component(storage = "dense")]`
on `StackIndex` produces two `error[E0277] … StackIndex: Bundle` at production insert sites in
`boyko_ui` (`reload/reconcile.rs`, `text/dispatch.rs`), `boyko-ui` fails, `boyko-render` is therefore
never checked, and the const-assert never evaluates — `grep -c "MUST be a TABLE component"` = **0**.
That is red mutation 11's shadowing generalized across a crate boundary, and it is also the shape of
the clippy masking below.

**⚠️ And that generalizes into a limit on what F6's assert can be claimed to cover (C7, corrected
2026-08-27).** The blind spot is NOT "a badly chosen mutation" — it is structural, and it is the
REALISTIC regression:

* **A NEW dense member added to the list** ⇒ the assert fires reliably and by name
  (`error[E0080] … UiSpriteCursor MUST be a TABLE component`, reproduced above).
* **An EXISTING member flipped to dense** ⇒ the assert is **not what reports it**, whenever that
  member has a direct-insert site in `boyko_ui`: the `E0277 … : Bundle` errors arrive first,
  `boyko-ui` fails, `boyko-render` never compiles and the const never evaluates. The defining crate
  compiles first, so this ordering holds for every member `boyko_ui` both defines and inserts.
* **For `UiVisual` specifically the regression IS still caught loudly and by name — by a DIFFERENT
  guard.** MEASURED 2026-08-27, `#[component(storage = "dense")]` on `UiVisual`,
  `cargo check -p boyko-render --lib`: **two** errors, `E0277 UiVisual: Bundle` at
  `boyko_ui/src/animation.rs:457` **and** `error[E0080]` from **`boyko_ui/src/components.rs:1117`**,
  which is `UiVisual`'s OWN dedicated const-assert (A1 leg 11a) — not `gather.rs`'s `assert_table`.
  The two guards are complementary, not redundant, and only leg 11a covers this direction.
* **Coverage, stated instead of implied.** `ui_pack_inputs!(assert_table)` is invoked **exactly once**
  in the workspace (`gather.rs:147`). The survey enumerates **five** production `Or<..>` lists
  (`light_system.rs:989`, `layout.rs:88`, `layout.rs:118`, `text/measure.rs:61`, `gather.rs`'s own);
  the assert guards **one**. `gather.rs`'s closing *"this guard is to keep it that way"* over-claimed
  about the other four and has been rewritten (`gather.rs:100-119`).

*What is NOT proved, and is recorded as such rather than papered over.*

1. **Gate 5's property (a restart REPLACES and REWINDS) is `NOT PROVED`** — its mutation is
   unwritable and its substitute attacks the shared fresh-insert path. See red mutation 5 above.
2. **Red mutation 11's second clause is not a single mutation** — reaching gate 2's `Or` half takes
   three coordinated edits. See red mutation 11 above.
3. **The F2 ordering edge is a CONTRACT, not a wired edge.** Verified 2026-08-27: all **12**
   `add_system(ui_render_discovery)` sites in the tree are in `crates/boyko_render/tests/`; `src/`
   has **zero**. There is no production registration for an edge to be declared on, and `boyko_ui`
   could not declare it in any case (`crates/boyko_render/Cargo.toml:82` depends on `boyko-ui`; the
   reverse dependency does not exist). **A4 inherits this — see its blocking precondition.**
4. **The GPU golden suite was NOT run** at the remediation (device-needing leg, outside its mandate).
   `--all-targets` clippy compiles those targets, so no compile break is hiding there, but no image
   pin was re-taken.
5. **`tests/ignore_reasons_census.rs` does not exist on this branch** — it lives on
   `feat/multi-paradigm-render` and `fix/inherited-red-gates`. It is NOT RUN here by construction and
   must not be reported green.
6. **Miri:** `cargo miri test -p boyko-ui` exits 1 AT HEAD on three pre-existing
   `memory leaked … RawVec<InlandPoolId>` errors from `BundleColumnCache::resolve_and_cache`. Miri
   red/green on this branch is read off the `test result:` line and the FAILED names, never off `$?`.
   RE-MEASURED for this note, 2026-08-27: `cargo miri test -p boyko-ui --test miri_a1_tween` ⇒
   raw exit **1**, `running 4 tests`, `test result: ok. 4 passed; 0 failed`, and **4**
   `error: memory leaked` — every one a `RawVec<InlandPoolId>` from
   `BundleColumnCache::resolve_and_cache::<UiVisual>`. The count moved 3 → 4 with the test count
   3 → 4: one per spawning test, the documented pre-existing shape scaling, not a new class.
7. **F6's const-assert does not cover the regression a reader would assume it covers** — an EXISTING
   member flipped to dense, when that member has a direct-insert site in `boyko_ui`. It fires
   reliably only for a NEW dense member. And it guards **one** of the five production `Or<..>` lists
   the survey enumerates, not all five. Both limits are measured and written at `gather.rs:100-119`
   and in the C7 block above. *(Added 2026-08-27, third remediation round. **`UiVisual` itself is
   still covered — by leg 11a's own const-assert in `components.rs`, which is a different guard.**)*
8. **F9-a's guard and F9-b's inversion OVERLAP on the NaN member, and NaN is not gated by the guard
   alone.** MEASURED: under the shipped `advance`, a NaN duration is NOT immortal — `t` is NaN, the
   `t < 1.0` spelling puts it on the COMPLETING side, and the row completes on frame 1 and is reaped.
   Four of the five refused shapes are immortal; NaN is not one of them. The entry-point guard
   refuses it anyway, but **no gate here may claim NaN as coverage earned by the guard**. This is
   what made C2's pre-correction NaN arm incapable of failing, and it is why the gate now reads the
   row count BEFORE the first tick. *(Added 2026-08-27, third remediation round.)*

*The declined guard, priced, so a later rung does not re-litigate it from scratch.* Closing the
residual hand-inserted-`TweenXBundle` route in code means a per-row `inv_duration > 0.0` term inside
`advance`. MEASURED (4096 nodes × 4 channels, release, min-of-40×50 per process, floor across five
process invocations): **14.008 ns/row at HEAD · 14.002 with the `t < 1.0` inversion alone · 14.544
with the per-row guard added** — **+0.536 ns/row, +3.8 %**, forever, on every animating row. Declined
because the entry-point refusal costs zero per frame and is the crate's own `UiClock::set_max_delta`
precedent, and because the guard would defend one `pub` field on a route where `from`, `to` and
`elapsed` are equally undefended. *(⚠️ Instrument note, because it fooled the measurer first: a
single-variant spread across process invocations on this box is 14.0–22.9 ns. A one-shot read of
21.6 ns produced a confident and wrong "the inversion is a 35 % speedup". The only usable statistic
is the FLOOR across several process invocations, interleaved A/B/A/B.)*

*⚠️ The mandated linter gate was RED at `e7a16fd9`, and it masked FOUR crates.* Cache-matched A/B
(`cargo clean -p boyko-ui` before BOTH runs, one edit apart), measured twice by two independent
passes with identical results: **red** ⇒ EXIT=101 with only `boyko-input`, `boyko-threadpool`,
`boyko-ui` reaching a `Checking` line; **green** ⇒ EXIT=0, 0 warnings, 0 errors, with `boyko-render`,
`boyko-app`, `boyko_rhi_vulkan` and `aether-tests` additionally appearing. **Every clippy claim made
on this branch between `e7a16fd9` and this remediation was vacuum-green for those four crates.** The
masked set is four, and `boyko-ui` is NOT among them — it is the crate that errored, which is the
distinction that makes "one crate was hidden" the wrong summary. Filed for the owner in
`docs/OPEN-QUESTIONS.md`.

*What is anchor-gated, and what is not.* `tests/internal_docs_anchors.rs:231` gates exactly four
documents — `FEATURE_MAP.md`, `SYSTEMS.md`, `ARCHITECTURE.md`, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`.
**`docs/UI-PLAN-ANIMATION.md` is not one of them**, so **none** of the `file:line` citations in this
rung — including M4b's `ui_a1_tween.rs:1260` and every anchor in the table above — is mechanically
checked. They are correct as of 2026-08-27 and they will rot silently. *(A sibling lane measured 96 of
143 UI-plan anchors already stale, with nothing gating them.)* Adding the UI plans to `GATED_DOCS` is
a SCOPE call and is filed for the owner.

⚠️ **This paragraph's prediction was tested and CAME TRUE in under 24 hours, on its own example.**
M4b's anchor read `:1117` when the sentence above was written on 2026-08-27; the census landing the
next day moved that function by **+143**, and `:1117` became a doc line of a different test. It was
cited at three sites in this document and all three were stale in the same way. The coordinate above
is the value part 7 re-measured by content; it is not gated either, and the next landing will move
it again. **Part 7's sweep measured the standing exposure: 146 citations in these four documents
point into files the A1 landing changed** — see part 7 for the per-file breakdown and the method.

*Landed set — part 4 (the remediation OF part 3; one blocking, five accuracy).* The third
adversarial pass measured the part-2/part-3 gate rather than re-reading it, and found that **the
gate certified 52/52 by two prior passes was broken in BOTH directions** — and that part 3's own
accuracy repair had reproduced the defect class it was convened to fix.

| Act | What | Site |
|---|---|---|
| **D1 — BLOCKING, a gate broken in BOTH directions** | Gate 6's `armed()` took `.max()` on each side and compared two independently-sampled maxima with zero slack. MEASURED: both arms have a DETERMINISTIC per-frame cost of **6** allocations with byte-identical size histograms, the A1 pair's two systems contribute **ZERO** of them, and the sporadic `+1` (reaching `8` under load) is charged **always to a THREADPOOL WORKER** thread — it is `Schedule::run`'s executor, it appears with `noop` systems exactly as readily as with the pair, and **the fixture cannot quiet it**. Consequence, measured on the real binary in separate processes: clean code went **RED 3 in 60** isolated idle runs, *and* the gate's own documented red mutation (`Vec::with_capacity(64)` in `ui_visual_tick`, signal = 1) went **GREEN 2 in 60** runs under CPU load, because `base_max` absorbed the `+1` as noise. **`.max()` does not merely manufacture false reds; it destroys the true red.** Fixed to the **FLOOR** statistic on both sides — allocation noise is strictly ADDITIVE, so the minimum over the window IS the deterministic cost unless every frame is hit. ⚠️ **SUPERSEDED by E1 (part 5): the diagnosis above stands, but the fix named here took the floor along the FRAME axis and was itself blind to four of the seven mutations the widening exists to catch. See the E1 rows in part 5** | ⚠️ pre-E1 coordinates, kept as the record and NOT resolvable in the current file: `ui_a1_zero_alloc.rs:273-322` (the then-`armed()` — doc `:273-310`, body `:311-322`, `.min()` at `:321`), `:151-154` (`live_total`, since replaced by `live_per_channel`), `:349` / `:373` (call sites), `:22-38` (module header). Current coordinates are in the E1 rows |
| **D1 — the floor's PRECONDITION, made mechanical** | The floor covers the reap loop and the four `None` arms only while **every** armed frame carries a completion — which is exactly what the staggered `TINT`/`OPACITY`/`OFFSET`/`SCALE_MS` durations buy. That is a property of four constants, and a duration edit that moved one completion out of the window would have silently deleted the reap path from the gate's coverage and left it GREEN. Now asserted per-frame: `reaped == NODES` on all four. ⚠️ **SUPERSEDED by E1 (part 5): the sentence this row states is FALSE — the floor covers the reap LOOP but NOT the four `None` arms — and `reaped == NODES` is an AGGREGATE that cannot detect the difference. See the E1 rows in part 5** | ⚠️ pre-E1 coordinate, kept as the record: `ui_a1_zero_alloc.rs:375-388` |
| **D2 — the guard's scope, corrected; it over-claimed** | `invalid_tween_duration` closes **degenerate reciprocals** — `duration_ms` non-finite or not positively signed. It does **NOT** close "the immortal row class", and this rung retracts that reading. `elapsed` is an `f32` accumulating `+= dt`, so absorption gives it a HARD CEILING: **`524288 s` (`2^19`, 6.068 days)** at `dt = 1/60 s` and at 16 ms, **`2097152 s` (`2^21`, 24.273 days)** at the 100 ms `UiClock` clamp. A row completes only when `elapsed` reaches `duration_ms / 1000`, so every accepted duration STRICTLY above that ceiling — at 60 Hz, `> 5.24288e8` — is **genuinely immortal**, and `1e10`, `1e30` and `f32::MAX` are all on that side. ⚠️ **The operator is `>`, not `>=` — corrected 2026-08-27 (E3), and re-measured at every site rather than carried between them.** `5.24288e8` (bits `0x4dfa0000`) gives `inv_duration = 2^-19` exactly, so `t` at the ceiling is exactly `1.0` (bits `0x3f800000`), and `advance`'s `t < 1.0` puts exactly `1.0` on the COMPLETING side: **the boundary value itself completes.** The first genuinely never-completing duration is ONE ULP ABOVE, `5.24288032e8` (bits `0x4dfa0001`, `t = 0.99999994`). The `>=` spelling was written once and transported to FOUR sites — this row, `animation.rs`, and both `OPEN-QUESTIONS.md` twins — and was wrong at all four, while `ui_a1_tween.rs`'s own doc, which measured it independently, said "above" and was right. **An accepted `f32::MAX` is strictly WORSE for the A4 repaint skip than the REFUSED `+inf`**: it bumps `set_if_neq` on every frame where `+inf` bumps zero. The predicate is NOT changed here — the boundary is `dt`-dependent, so any constant is a policy choice, and that is a VALUES call filed for the owner | `animation.rs:212-292` (the `# What this predicate does NOT close, MEASURED` block, grown by the E3/E4 corrections and again by part 7's census disclosure; `:293` is the next heading, ``# `+0.0` and the denormals are ACCEPTED``), gate at `ui_a1_tween.rs:840-1096` (doc `:840-983`, `#[test]` `:984`, `fn` `:985-1096`). ⚠️ **Re-measured by CONTENT 2026-08-28 (part 7).** The block END read `:260` and the gate read `:840-953` (doc `:840-883`, `#[test]` `:884`, `fn` `:885-953`): the block START and the gate START were both still right, and every END and every interior coordinate was wrong — `:261` had become *"BITS, never by the printed digits."* and `:953` a bare `///`. **A range whose start is checked and whose end is not reads as verified and is not** |
| **D2 — ⚠️ REFUTED, the deletion half had no referent** | The ruling directed a deletion of an over-claim at `animation.rs:195-197`. **Those three lines are two table rows and a blank `///`** (`\| -0.0 \| -inf \| … \|`, `\| NaN \| NaN \| … \|`, `///`) — there is no prose there. The nearest prose (*"Four of those five are immortal…"*) is scoped to the five REFUSED shapes and is TRUE as written; the ruling itself says it stays. A grep of `immortal` / `closes the` / `never completes` across `boyko_ui/src`, `boyko_ui/tests` and `boyko_render/src` found **no sentence anywhere asserting the guard closes the class**. The over-claim being retracted lived in the READING, not in the code. Insertion landed; **nothing was cut** | — |
| **D3 — the borrowed measurement, DROPPED at three sites** | C3 (part 3) refuted a false rationale at `ui_visual_tick` and then propagated its replacement to three sites under the words *"identically-shaped query"*. **The four queries are four different shapes.** The property transferred — the alias's lifetime arity — is per-site: `ui_visual_tick` needs three, `ui_layout_discovery` and `ui_render_discovery` need two. The measurement is not load-bearing (it justifies an `#[allow]` the readability argument already justifies alone), and a measured claim with no gate behind it, re-verified at each site, is upkeep that buys nothing. **The repair is not "measure four times" — it is to stop claiming a measurement where none was taken.** Each of the three now carries an explicit no-measurement note | `gather.rs:522-532`, `layout.rs:81-87`, `text/measure.rs:54-64` (`animation.rs:688-699`, the one site that DID measure, untouched by D3 — re-measured by CONTENT 2026-08-28, part 7; the row read `:623-633`) |
| D4 | The `-0.0` superlative that part 3 landed while disambiguating another one. Criterion now named at the site; the false claim deleted | `ui_a1_tween.rs:696-697`, `:712-717`, and the C9 amendment row above |
| D5 | The five refusal arms are **TWO mechanisms plus one shape caught by both**, not five independent covers: `is_finite()` gates `NaN` and `+inf`; `is_sign_positive()` gates `-100.0` and `-0.0`; **`-inf` leaks under NEITHER**, so its arm has zero discriminating power against either natural weakening. Recorded so the next reader does not infer five covers from five rows | `ui_a1_tween.rs:767-785` |
| D6a | The `debug_assert!` message said "strictly positive" 40 lines below a doc that deliberately ACCEPTS `+0.0`. Now states "finite and POSITIVELY SIGNED", and says why | `animation.rs:307-314` (the `debug_assert!` and its message; re-measured by CONTENT 2026-08-28, part 7, where the row read `:255-262` — today two bare `///` lines bracketing doc prose) |
| D6b | F6's red-ledger row was the one copy a sweep left behind: the F6 **gate** row (this document `:1650`, re-anchored to `:147 / :166-169 / :184-186 / :211-230`) and the M0-c anchor in its other copy (`:822`, corrected to `gather.rs:370-371`) were BOTH fixed while the red-ledger row kept all three stale coordinates. **One row missed in a sweep that corrected its neighbours is the anchor-rot signature — the remedy is a census over all copies, not another pass.** Annotated as-observed, NOT re-coordinated, because a red ledger is a transcript | the F6 row in the RED ledger above |

*Gate — the D1 acceptance legs, run as separate processes with `--test-threads=1`, never piped,
`$?` captured on the next line.* The run counts are chosen so the leg can SEE the rate it is
certifying against: at the max-gate's measured idle false-red rate of 5 %, `P(0 failures in 200) ≈
3.5e-5`, whereas 40 runs would leave a 1-in-40 rate invisible 36 % of the time.

| Leg | Runs | Result |
|---|---|---|
| clean, idle | **200** | **0 failures** |
| clean, saturated (16 spinners, `nproc` = 16) | **100** | **0 failures** |
| clean, idle — re-run on the FINAL tree | **200** | **0 failures** |
| clean, saturated — re-run on the FINAL tree | **100** | **0 failures** |
| red 1 — `black_box(Vec::<u8>::with_capacity(64))` atop `ui_visual_tick` | 60 idle + 60 saturated | **120 / 120 RED** |
| red 2 — same expression first in `ui_tween_reap`'s `for` body | 40 | **40 / 40 RED** |
| red 3 — the NEW reap census, `SCALE_MS` moved out of the window | — | **RED**, `armed frame 3 reaped 0 channel rows, not 32` |

**600 clean runs, zero false reds; 160 red-mutation runs, 100 % red.** Red 2 reads `baseline floor 6,
pair floor 38` — `6 + 32` reaped rows on **every** armed frame, which is the part-2 widening's real
consequence reproduced independently rather than inherited.

*The name-set re-enumerated, because Act B-2 adds a test.* `ui_a1_tween` moves to **release 14 /
debug 13**; the release-only set is still exactly `a_degenerate_duration_creates_no_row` and the
debug-only set is empty. Re-run by NAMES with `-- --list | sort` + `diff`, not by counts. No document
pinned the old 13/12 numbers, so this is a re-run obligation discharged, not an anchor edit.

*⚠️ NEW FINDING — a SECOND gate in this crate has the same flaw, and it is not this rung's.* Release
run 4 of 8 failed at `crates/boyko_ui/tests/zero_alloc.rs:296` with `baseline 5, pair 6` — **the exact
`+1` signature diagnosed above.** That file is **untouched by this landing** (not among the 13
modified; the only edits reaching its code path are comment-only, and the one non-doc change is a
`debug_assert!` message string that compiles out in release). Measured standalone: **100 runs ⇒ 6 red
/ 94 green, 6 % false-red.** It is **worse than max/max** — line 296 compares a **single** frame
against a **single** frame with zero slack, one sample versus one sample. Per the ruling this is a
survey and was NOT fixed here, but it is no longer a suspicion: `cargo test -p boyko-ui --all-targets
--release` is ~6 % flaky today through no fault of this rung, and the same floor remedy applies.
`p4_bind_zero_alloc.rs` and `text_emit_zero_alloc.rs` are cited as the same established pattern and
were **not** measured. Filed for the owner.

*Out of scope, filed rather than done.* (1) **`Schedule::run`'s per-frame worker-thread allocation** —
the `+1`/`+2` is real engine behaviour on the parallel executor, on a path Principle 5 governs, and
it is not the A1 pair's; rung A1 is the wrong place to chase it. Numbers for whoever picks it up: 6
deterministic and all main-thread; the sporadic extra always off-main; reaches 8 under load; present
with `noop_normal`/`noop_exclusive` and no UI code at all (348 frames: `6×313, 7×34, 8×1`). (2) The
zero-alloc survey above. (3) **An upper bound on tween duration** — VALUES, owner, filed in
`docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin.

**The lesson this part owes, stated plainly because it is about this campaign's own passes.**

* **D1 — a gate certified twice was a coin being sampled.** Two prior passes reported 52/52 green.
  The gate was **5 % false-red at idle** and **3.3 % false-GREEN on its own documented red under
  load**, and neither pass ran the red mutation more than once. **A red mutation observed ONCE is
  not a measured red.** The part-2 widening was RIGHT and is retained — it is what converts the
  reap-loop regression from a sporadic signal into an every-frame one, which is exactly what makes
  a floor statistic sound. What was never measured is the widening's interaction with the
  **statistic**: `.max()` over two independently-noisy maxima, compared with zero slack, is
  maximally fragile precisely where the noise lives.
* **D3 — the accuracy-repair pass reproduced the defect class it was convened to fix.** A
  measurement taken at one site was transported to three others under a qualifier
  (*"identically-shaped"*) that does not hold. **A correction inherits the original's presumption of
  correctness and gets LESS scepticism than the thing it replaced.** Every claim names the
  measurement behind it *at its own site*, or says that none was taken.
* **D4 — a superlative with no stated criterion is unfalsifiable, not ambiguous**, and that is what
  let a wrong one land through the pass that was auditing superlatives.
* **D6b — a sweep that corrects a row's neighbours and misses the row is the anchor-rot signature.**
  The fix is a census over all copies, not another pass. The same pass wrote *"a rotted anchor that
  lands on plausible content is worse than one that lands out of range"* and then left three such
  anchors standing, one of them on plausible content.
* **⚠️ The false-red rate moved ~30× with ambient conditions that could not be controlled** (two
  early series of the uninstrumented shipped binary gave 59/60 and 77/80 red and never reproduced;
  the cause was not attributed and is not invented here). The operative conclusion is not the
  number — it is that **a run count cannot bound a rate that moves that much.** Prefer a statistic
  robust by construction over a certification budget.

*Landed set — part 5 (the remediation OF part 4; two blocking, four accuracy).* The fourth
adversarial pass found part 4's own repairs carrying the next defect **twice**: the replacement gate
statistic was stable and BLIND, and a boundary number part 4 stated was transported to four sites and
wrong at all four. **No production code changed in part 5, and that is measured rather than
asserted.** The two gate fixes (E1, E2) left `crates/boyko_ui/src/animation.rs` byte-identical to the
state part 5 inherited (`cmp` EXIT=0, sha256 `7b344ad4…`) — the file is one of the landing's 13
modified files, so that is identity against the inherited tree, **not** against `e7a16fd9`. E3 and E4
then corrected that same file's DOC COMMENTS: **38 changed lines, every one of them a `///` line,
zero non-comment changed lines** (sha256 now `c46e5702…`).

| Act | What | Site |
|---|---|---|
| **E1 — BLOCKING: the floor was taken along the WRONG AXIS, and it is part 4's own defect returning** | D1's floor is a `min` over the four ARMED FRAMES. The noise it removes is per-frame-RANDOM; the signal it destroys is per-frame-DETERMINISTIC; `min` over frames removes **both**. The staggered `TINT 140 / OPACITY 156 / OFFSET 172 / SCALE 188` durations mean **each channel's `None` arm executes on exactly ONE of the four armed frames**, so a floor over frames discards anything that does not raise all four. MEASURED against the frame-axis floor: `black_box(Vec::<u8>::with_capacity(64))` planted in the `scale` `None` arm went **GREEN 40/40**, in the `tint` arm **GREEN 25/25**, and a reap loop keyed to `TweenScale` **GREEN 25/25** — while the positive control (the same expression atop `ui_visual_tick`, 1 alloc EVERY frame) went **RED 30/30**. The instrument saw 1 alloc/frame and missed 32 allocs on one frame. Instrumenting `armed()` showed why: `counts = [6, 6, 6, 38]`, whose `min` is `6` — the baseline floor. **Fixed by taking the floor along the REPETITION axis**: `REPS = 8` independent rebuilds of world + schedule + warm-up, `floor[i] = min over repetitions of counts[r][i]` — which removes the executor noise and keeps per-arm resolution. `REPS = 8` is measured, not chosen: 200 repetitions per arm per frame index put the deterministic floor at **6 on every index of every arm in all 1600 samples**, longest CONSECUTIVE-noisy run 7 | ⚠️ **Anchors RE-MEASURED BY CONTENT 2026-08-28 (part 6)** — part 6's H1 landing added ~180 lines to this file and moved every one of them. As written they read `:151` / `:314-318` / `:319-335` / `:337-401` / `:402-446` (with the per-index `min` at `:443`) / `:181-188` / `:198` / `:489` / `:490` / `:537-547`; **`:443` had already drifted onto plausible content before part 6** — a doc line reading *"measured defect this gate has already shipped once."* ⚠️ **And RE-MEASURED BY CONTENT AGAIN 2026-08-28 (part 7), because part 7's own landing moved every one of them a second time** — as part 6 wrote them they read `:179` / `:383-387` / `:388-404` / `:406-474` / `:475-536` / `:531` / `:236` / `:237-243` / `:245-252` / `:253` / `:616` / `:617` / `:687-699`, and `:179` had drifted onto a doc line of `const REPS`'s own block (*"allocation would be counted several times over. The fixture holds FIVE"*), plausible content once more. ⚠️ **AND A THIRD TIME, 2026-08-28 (part 10) — the part-7 list above was stale at ALL FIFTEEN coordinates**, moved by part 9's own landing on this file, which no sweep re-opened because part 9 scoped itself to the file it moved *most*. As part 7 wrote them they read `:237` / `:486-490` / `:491-507` / `:509-581` / `:582-645` / `:640` / `:297` / `:298-304` / `:306-313` / `:314-321` / `:760` / `:761` / `:839-854`, and **eight of the thirteen land on plausible content**: `:237` is a doc line of `REPS`'s own block (*"The deterministic floor was **6 on every index of every arm in all 1600"*), `:297` is `const VIRTUAL_SPEED: f32 = 0.5;` — *a different `const`*, `:509` is `(0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()`, `:640` a bare `}`, `:760` a blank line, and **`:839` — cited as the per-index comparison loop — is `fn armed_once(`, the RIGHT SHAPE for a neighbouring row of this very list.** **Current, each read at its target 2026-08-28 (part 10): `ui_a1_zero_alloc.rs:255` (`const REPS: usize = 8;`), `:834-838` / `:839-855` (`armed_once` doc / body), `:857-929` / `:930-985` (`armed_floor` doc / body; **the per-index `min` at `:980`**, `*slot = (*slot).min(c);`), `:391` / `:392-398` (`live_per_channel`), `:400-407` / `:408-415` (`channels` doc / body, whose ordering is load-bearing), `:1105` / `:1106` (call sites), `:1171-1186` (the per-index comparison loop).** ⚠️ **This is the third consecutive part to re-measure this one list and the third to be invalidated by the next landing. Nothing checks it; see part 10's note on what the annotation now says.** |
| **E1 — the load-bearing sentence part 4 wrote was FALSE, and its census could not detect that** | D1 stated at two sites that *"The floor covers the reap loop and the four `None` arms only while **every** armed frame carries a completion"* (quoted with its own emphasis). The floor covers the reap LOOP — 32 rows every frame, which is why that mutation reds — and does **NOT** cover the four `None` arms. Its census could not catch the difference either: `reaped[i] == NODES` asserts only that the AGGREGATE is 32 per frame, satisfied by ANY assignment of channels to frames. **The precondition made mechanical was not the precondition the floor needs.** Replaced by a **4×4 channel-by-frame drop matrix** — `drops[i][c] == NODES` iff `c == i` — which pins WHICH channel completes on WHICH frame and is what gives each `None` arm an index of its own. Coverage is now stated as what it covers AND what it does not; the false sentence is gone from both sites (grep returns empty) | `ui_a1_zero_alloc.rs:486-505` (the rationale, now `armed_once`'s doc `:486-490`, plus the matrix loop `:497-505`), `:661-747` (the `# What the statistic covers, and what it does not` section; `:748` is the next heading, `# The non-vacuity census is the point`), `:1-101` (module header — the last `//!` line; `:102` is blank and `:103` begins the harness plumbing. `:22` is its `# The coverage claim is PINNED SOMEWHERE ELSE, and that is the point` heading). Re-measured by CONTENT 2026-08-28 (part 7); the row read `:502-525` / `:502-510` / `:511-525` / `:459-477` / `:479` / `:22-41`, of which only the `:22` end of the last was still right |
| **E1 — acceptance: seven red mutations, zero greens; clean code, zero reds** | Every run a separate process, `--test-threads=1`. `ui_visual_tick` +1/frame: **RED 30/30 idle, 60/60 saturated** (16 spinners) — the exact case where `.max()` went GREEN 2/60. `tint` / `opacity` / `offset` `None` arms: **RED 25/25** each. `scale` `None` arm: **RED 40/40 idle, 25/25 saturated**. Reap loop keyed: **RED 25/25**. Reap loop unkeyed: **RED 25/25**. Clean: **0 red in 150 idle**, **0 red in 80 saturated**. Every failure names the right index — the `scale` arm reports `pair floors [6, 6, 6, 38]`, which is the refuter's own `counts = [6,6,6,38]`, the vector the frame-axis floor collapsed to `6` and passed 40/40. Cost of `REPS = 8`: **54 → 70 ms per process** (in-test 0.00 → 0.02 s) | measured by the E1 landing |
| **E2 — the over-ceiling gate pinned ONE POINT while the disclosure covers an open-ended CLASS** | With `&& duration_ms < 1e20` added to the guard, `an_over_ceiling_duration_is_accepted_and_never_completes` **passed**, the whole `ui_a1_tween` binary passed 14/14, and `cargo test -p boyko-ui --all-targets --release` exited **0** over 52 targets. An upper bound WAS added and the gate that exists to notice it said nothing; its assertion message then ended *"If this reads 0, an upper bound was added"*, which was itself an over-claim — a bound above `1e10` HAD been added and the reading was still 1. It now ends *"an upper bound at or below {label} was added"* (`ui_a1_tween.rs:1012-1018`, the sentence itself at `:1013-1014` — ⚠️ **re-measured by CONTENT 2026-08-28, part 10; the row read `:913-916`, which is today the ```text block of K2's counterexample transformation, and this coordinate sat OUTSIDE the row's own by-CONTENT annotation, so no sweep had ever looked at it**). Fixed by looping the body over `OVER_CEILING_MS = [("1e10", 1e10), ("1e30", 1e30), ("f32::MAX", f32::MAX)]` with a fresh world per arm and the arm label on every assertion. **`f32::MAX` is what closes the class rather than sampling it**: it is the largest finite `f32`, so every finite upper bound refuses that arm wherever it is placed; `1e10` and `1e30` localize the bound. Three bounds applied, each RED at EXIT=101, each naming the correct arm: `< 5.24288e8` → `1e10`; `< 1e20` → `1e30`; `< 1e38` → `f32::MAX` | `ui_a1_tween.rs:840-1096` (doc `:840-983`, `#[test]` `:984`, `fn` `:985-1096`), the arm table at `:986-995` — the three-point doc list `:986-993` plus `OVER_CEILING_MS` `:994-995` (re-measured by CONTENT 2026-08-28, part 7; it read `:840-953` / `:840-883` / `:884` / `:885-953` / `:894-895`) |
| **E3 — a named boundary carried the wrong OPERATOR, and the value it names COMPLETES** | Part 4 wrote the immortality boundary as `duration_ms >= 5.24288e8` and transported it to **four** sites. Re-measured independently at each site (`rustc -O`, exact `f32`): `5.24288e8` is bits `0x4dfa0000` = exactly `524288000`, so `inv_duration = 1000.0/it` is bits `0x36000000` = exactly `2^-19`, and `t` at the `2^19` ceiling is `524288.0 × 2^-19` = exactly `1.0` (bits `0x3f800000`). `advance`'s `t < 1.0` puts exactly `1.0` on the **COMPLETING** side, so **the boundary value completes** and the operator must be `>`. The first genuinely never-completing duration is ONE ULP ABOVE: `5.24288032e8`, bits **`0x4dfa0001`**, `t = 0.99999994`. The 100 ms clamp behaves identically: `2.097152e9` (bits `0x4efa0000`) reaches exactly `1.0` at `2^21` and completes; `0x4efa0001` is the first that does not. Ceilings re-confirmed: `524288 s` in **24,986,955** frames at 1/60 s and 25,150,895 at 16 ms; `2097152 s` in 18,073,720 at 100 ms. **The one site that measured it independently was right and is left untouched**: `ui_a1_tween.rs:844` reads *"any `duration_ms` above `5.24288e8` yields a row that can never reach `t = 1.0`"* | `animation.rs:223-224` + the new `:228-239` measurement paragraph, this document's D2 row, `docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin |
| **E4 — the closed form named the value on the WRONG SIDE of the boundary it defines** | The reciprocal-overflow boundary was stated as bits `0x047a0001` (`2.9387360564219222e-36`), *"exactly `250.0 × f32::MIN_POSITIVE`"* — in a doc block whose own instruction is **"state it by BITS"**. Measured: `250.0 × f32::MIN_POSITIVE` is bits **`0x047a0000`** (`2.9387358770557188e-36`), ONE ULP BELOW, and it is the **LARGEST duration that still overflows**; the smallest with a finite reciprocal is `0x047a0001`. The bits were right and the closed form was off by one ULP. Corrected rather than deleted: both values are now given as bits AND decimal, each on its named side, and both decimals were checked to round-trip to their stated bits. The "cannot be named by a decimal" justification is also made precise — the two bracketing floats print `2.938736e-36` **at 7 significant figures** (`{:.6e}`); under shortest-round-trip they differ (`2.9387359e-36` vs `2.938736e-36`) | `animation.rs:263-272` (re-read by CONTENT 2026-08-28 — `:245-254` until part 6's opacity correction added 18 doc lines above it; `:263` is *"A second, benign boundary sits far below"*, `:272` is *"and it is benign: those SNAP."*), `docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin |
| **E5 — four anchor defects, two named by the pass and two found while fixing them** | (1) The C2 row's *"(the gate)"* pointed at `ui_a1_tween.rs:763`, a **bare empty `///`** inside a doc block — re-aimed at `:786-788`, the actual `#[cfg]`/`#[test]`/`fn`. (2) F9-a's `animation.rs:177-225` was **mis-cut at its end** (`:225` ended a sentence, `:226` continued the same paragraph) — re-cut to `:177-271`, the whole doc comment, `:272` being `#[cold]`. **Found while fixing those:** (3) the same C2 row's *"(its doc)"* `:690-712` ended **MID-SENTENCE**, `:713` continuing it — re-cut to `:690-785`. (4) **M4b's `ui_a1_tween.rs:977` was stale as written and landed on PLAUSIBLE CONTENT** — a doc line of a different test ("Runs in BOTH profiles — every duration here is ACCEPTED") — the failure mode this document names as worse than landing out of range; the real item is `#[test]` `:1116` / `fn` `:1117`, corrected at all **three** citation sites. E3/E4's edits also moved every `animation.rs` line ≥ 235 by **+20**, and each live anchor was re-resolved by CONTENT, not arithmetic | this document; `crates/boyko_render/src/ui/gather.rs` and the two `OPEN-QUESTIONS.md` twins for the shifted `animation.rs` anchors |
| **E6 — a SEPARATE, PRE-EXISTING flake, filed and deliberately NOT fixed** | The crate's pre-existing `zero_alloc` suite reds intermittently in release. Re-measured here: **5 red in 60** standalone `--test-threads=1` runs — `zero_alloc.rs:238` alone 4, `:296` alone 1, both 0. **A1 neither introduced nor fixed it, proven not asserted**: `zero_alloc.rs` is untouched by the landing (`git status` on that path is empty) and `layout.rs` — the code it measures — has **zero non-comment changed lines**. ⚠️ **The two sites do NOT share a statistic**, so they are recorded separately rather than one measurement being carried to the other: `:238` compares `warmed_idle_allocs` (max of 4) against `warmed_idle_allocs`, while `:296` compares a **single** frame against a **single** frame (`:286` / `:294`) — strictly weaker. Part 4 recorded the `:296` half at 6/100; the site mix and rate move with load, so it is a range, not a constant | filed in `docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin; the part-4 note at this document's *"a SECOND gate in this crate has the same flaw"* paragraph |

**The lesson part 5 owes.**

* **A measurement taken at one site is not a measurement at another — and part 4 said so, then did
  it.** E3 is the pure case: one number, four sites, wrong at all four, while the single site that
  measured it independently was right. D3's own rule — that every claim names the measurement
  behind it at its own site — was written in part 4 and broken in part 4.
* **A statistic can be stable and blind at once, and stability is the easier property to
  demonstrate.** The frame-axis floor passed 230 clean runs with zero false reds and missed four of
  seven planted regressions. **Certifying a gate's false-RED rate says nothing about its
  false-GREEN rate**; only planted mutations measure the second, and they must be planted at each
  site the gate claims to cover — not at one representative site.
* **An anchor is checked by resolving it and reading what is there.** Two of part 5's four anchor
  defects were invisible to the pass that named the other two, and one of them was a *proposed
  replacement* that was itself wrong. None of these are machine-checked: the UI plans are not in
  `internal_docs_anchors.rs`'s `GATED_DOCS`.

#### Part 6 — the fifth adversarial pass: both of part 5's gates had a hole, and both are closed

The fifth pass affirmed part 5's advance (the repetition-axis floor caught **7 of 7** planted
mutations where the frame-axis floor missed 4, and ran 120/120 clean under load) and then found one
defect **in each of the two gates part 5 landed**. Both are closed in code; five documentation
defects behind them are closed here.

| Act | What | Site |
|---|---|---|
| **H1 — BLOCKING: the alloc gate never executed the RESTED branch, the majority path** | `ui_visual_tick` has a **seventh** site the acceptance list omitted: the all-`None` `continue` arm. Dense storage keeps a reaped channel out of the archetype signature, so a node that has finished animating still matches the tick's query and is yielded all-`None` **on every frame, forever** — every at-rest animated node, every frame. The window never ran it: in the two-cohort fixture every node carried a channel for the whole window, and the completing cohort lost its last one at frame 3's reap, AFTER the last measured tick. **Proven, not inferred**: an alloc planted in that arm left the gate GREEN, and a `panic!` there left it printing `test … ok` in 0.02 s while the same panic hung three tests in `ui_a1_tween`. Fixed by a **third, RESTED cohort**, spawned and warmed in the same world both arms build (a cohort on one side only would recreate the "structurally different workloads" defect the floor exists to avoid) and then deliberately not restarted. Coverage is now proven by execution, not by argument: the all-`None` count is censused before the window on every repetition and asserted per arm after it. The floor vector is unchanged, `base = pair = [6, 6, 6, 6]`. **Acceptance: 8 planted mutations, RED 25/25 each, 0 green; clean code 0 red in 100 idle and 0 red in 60 under 32 spinners on 16 cores.** The rested arm's signal is `pair = [38, 38, 38, 38]` — `NODES` on every index — and each per-channel arm's is `38` on its own index alone | **Re-measured by CONTENT 2026-08-28 (part 10); every coordinate part 7 wrote here had gone stale under part 9's landing on this same file.** Current: `ui_a1_zero_alloc.rs:58-105` (header — part 7 replaced the *"Why a RESTED cohort, and why it is not optional"* heading with `# The five cohorts`, the rested cohort being one of the five; `:107` is the next heading, `# Why the window has FIVE frames and the fifth is empty`), `:332-344` / `:345-357` (`rested_rows` doc / body — it counts through the tick's OWN query shape, not through the stores, because "carries a sink and no channel" and "is yielded all-`None`" are the same set only while `AnyOf` does not filter the archetype), `:622` called at `:945` / `:966` (pre-window census — now a named witness, `witness_rested_cohort_takes_the_all_none_arm` — and the post-window `let post = rested_rows(&mut world);`), `:1108-1114` / `:1115-1129` (the per-arm assertion's lead-in comment and the two `assert_eq!`s, in the caller). As part 7 wrote them they read `:46-88` / `:89` / `:270-282` / `:283-295` / `:615-623` / `:626` / `:763-784`, and as part 6 wrote them `:39-63` / `:209-221` / `:222-234` / `:507` / `:517` / `:626-640`. ⚠️ **`:626` has now named THREE different subjects in three parts** — part 6's first line of the per-arm assertion, part 7's post-window census read, and today a line of the PRE-window census witness's assertion message. ⚠️ **And `:89`, cited as *"the next heading"*, is today `//!   `black_box(Vec::<u8>::with_capacity(64))` in that lane, GREEN 10/10.`** A coordinate can survive a landing, keep its shape, and stop meaning what it meant |
| **H1 — the arm-dependence the first spelling got wrong** | The post-window all-`None` count was first asserted as `2 * NODES` inside `armed_floor`, and went red on repetition 0 of the BASELINE arm reading `32`: the baseline runs `noop_normal`/`noop_exclusive`, so it never reaps and the completing cohort never joins the rested one there. The count is **arm-dependent**, and is now checked by the caller — the only place that knows which arm it asked for | `ui_a1_zero_alloc.rs:1115-1129` — the two `assert_eq!`s on `base_rested_after` / `pair_rested_after`, under the lead-in comment `:1108-1114` (**re-measured by CONTENT 2026-08-28, part 10**; part 7 wrote `:763-784`, which today opens on a bare `///` and closes on `b.add_system(ui_tween_reap).after(tick);` inside `build_pair` — the schedule builder, not an assertion; part 6 wrote `:626-640`) |
| **H2 — BLOCKING: the over-ceiling CLASS gate was refuted by a CLAMP** | The `f32::MAX` argument — *"the largest finite `f32`, so every upper bound strictly below it refuses that arm"* — is sound for `duration_ms < X` and says **nothing** about `duration_ms.min(X)`, the more natural way a rung would cap a duration. MEASURED: one line `let duration_ms = duration_ms.min(1e4);` after the guard left the gate green 14/14 and `boyko-ui --all-targets --release` green at 342 passed — while every accepted duration was then UNDER the ceiling, so the disclosure the gate exists to protect had become **false**, and the gate's own assertion message (*"`elapsed` can never reach 10000000 s"*) was false with it. **A clamp anywhere between 500 ms and `f32::MAX` — the true `5.24288e8` ms ceiling included — closed the class silently.** Fixed by asserting the **DATUM** rather than its consequences: the row's stored `inv_duration` must be bit-exactly `1000.0 / duration_ms` recomputed from the value passed IN. That is the one number the door produces from the duration, and it moves under clamp, floor, round, scale **and** refusal. Verified against six bounds — `.min(5.24288e8)`, `.min(1e4)`, `.min(5e1)`, `< 5.24288e8`, `< 1e20`, `< 1e38` — **all six RED at EXIT=101**, each naming the correct arm | `ui_a1_tween.rs:840-1096` (doc `:840-983`) / `:984` (`#[test]`) / `:985-1096` (`fn`), `:986-995` (the arm table), `:1038-1053` (the datum assertion, under its rationale comment `:1021-1032`), `:1041-1050` and `:1064-1072` (the two messages that were false, now scoped to *refusal-shaped* bounds and to the duration the row ACTUALLY HOLDS). Re-measured by CONTENT 2026-08-28 (part 7); the row read `:840-938` / `:939` / `:940-1043` / `:949` / `:983-1003` / `:969` / `:1014` |
| **H3 — a measured-wrong number at three sites, and it is this rung's signature defect FOUR passes running** | `3.673e-36` was stated as the opacity an accepted `f32::MAX` renders. **It matches no frame of any fixture.** Re-measured 2026-08-28 two independent ways that agree bit-for-bit — read back FROM THE ENGINE through the gate's own `f32::MAX` arm, and by exact `f32` simulation of `advance` — the E2 fixture (`0.0 -> 1.0`, `FRAME = 100 ms`) renders **`2.938736e-37`** (bits `0x02c80001`) at frame 1 and **`1.469368e-36`** (bits `0x03fa0001`) at frame 5. `3.673…` is the frame-5 value the NEIGHBOUR's endpoints (`0.0 -> 0.25`) would give — `3.67342e-37` — **and the stated exponent is off by a further factor of 10 from even that**. ⚠️ The likely mechanism is now recorded at the site: `2.938736e-37` shares all seven mantissa digits with the `2.938736e-36` one paragraph below, which is `inv_duration` — and not by coincidence, since `1000.0 / f32::MAX` is bits `0x047a0001`, the very pattern that paragraph names | `animation.rs:241-261`, `docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin |
| **H4 — anchor: the per-index `min`, already on plausible content before part 6 moved it** | The E1 row cited it at `ui_a1_zero_alloc.rs:443`; `:441` was the `min` and `:443` the loop's closing brace. H1's landing then added ~180 lines to that file, and today `:443` is a **doc line** — *"measured defect this gate has already shipped once."* — the plausible-content failure this document calls worse than out-of-range. The whole E1 site list is re-measured by content in that row; the `min` is at ~~**`:531`**~~ **`:980`** *(`*slot = (*slot).min(c);` — re-measured by CONTENT 2026-08-28, part 10)*. ⚠️ **This row is the campaign's own worked example of anchor rot and it had itself rotted TWICE, in a way no sweep could see: part 7 moved the `min` to `:640` and updated the E1 row but NOT this one, so for two parts the document carried two DIFFERENT answers to "where is the `min`" — `:531` here, `:640` there — and both were wrong by part 9. `:531` today is `start_all_four(&mut world, everyone.clone(), [1.0; 4], 0);`, `:640` a bare `}`. Even the historical quote no longer holds: `:443`, which this row says is a doc line, is today `[-400.0, 0.0],`.** **A document that repairs one citation of a fact and not its twin has not repaired the fact — it has forked it, and the fork is invisible to a per-row sweep** | E1 row, above |
| **H5 — anchor: a self-citation off by 8, onto a bullet in the same discussion** | The part-5 note said the third protected transcript is cited *"at this document's `:1656`"*. `:1656` is a bullet about `UiSpriteCursor`; the citation is 8 lines lower. Re-resolved by content and re-measured **after** part 6's own edits to this file rather than before them. ⚠️ **Part 10: this row's own supporting quote has expired.** `:1656` is no longer a bullet about `UiSpriteCursor` — the pointer it describes has since read `:1664`, then `:1673`, and part 10 found `:1673` BLANK and re-aimed it a fourth time (see the part-3 anchor-sweep paragraph's own note). **A row that records where a stale anchor USED to point is itself an anchor, and it rots on the same schedule as the thing it is documenting** — which is why part 10 stopped writing these as claims of currency and started writing them as dated transcripts | the part-3 anchor-sweep paragraph |
| **H6 — anchor: a LIVE pointer stale by 51 lines, which a blanket offset could not repair** | F1's row said the `#[allow]` is *"now `animation.rs:603`"* with the query at `:606`. Measured, they were `:654`/`:657` — already 31 lines stale when part 5 inherited them, and part 5's blanket "+20" doctrine would have given `:623`, still wrong. **An offset repairs a transcript; it never repairs a live pointer.** The row now distinguishes the two explicitly and says the live half must be RE-READ each round. ⚠️ **Part 7: it moved a FOURTH time, to `:700` / `:701`** — the census landing's doc additions to `animation.rs` put +46 above it. Four rounds, four different true values for one `#[allow]`; this single pointer is the campaign's clearest measurement of what an ungated anchor costs per round | F1 row |
| **H7 — the twins kept a conclusion the same paragraph falsifies** | `animation.rs` correctly says the two floats bracketing the reciprocal boundary *"both print `2.938736e-36` at 7 significant figures, so **no decimal at that width** can name it."* Both `OPEN-QUESTIONS.md` twins dropped the qualifier to *"so a decimal cannot name it"* — false, and refuted four lines above by the two 17-digit decimals that round-trip exactly. Re-verified 2026-08-28: at `{:.6e}` the two forms are equal; under shortest-round-trip they differ (`2.9387359e-36` vs `2.938736e-36`); both 17-digit decimals parse back to their stated bits. The qualifier is restored in both twins **and** the reason is stated: the limit is the WIDTH, not decimal | `docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin |
| **H8 — a stated coverage gap, recorded and deliberately NOT closed** | `ui_clock_tick` is uncoverable by the alloc gate **by construction** — it runs in both arms, so it raises `base[i]` and `pair[i]` equally and the subtraction cancels it. That was already documented. What was not: `ui_a1_zero_alloc.rs` is the **only** allocation-counting test in either crate that so much as mentions it, so `UiAnimationSet`'s third member has **no per-frame allocation gate anywhere in the workspace**. Stated at the site; not built, on instruction | `ui_a1_zero_alloc.rs:1050-1059` (the `ui_clock_tick` bullet; the two non-coverage bullets together are `:1047-1059`, and `:1044-1046` is their lead-in), inside the coverage section `:1001-1091` (`:1093` is the next heading, `# The non-vacuity census is the point`). ⚠️ **Re-measured by CONTENT at BOTH ENDS 2026-08-28, part 10** — part 7 wrote `:710-714` inside `:707-714` / `:704-706` inside `:661-747` / `:748`, and part 6 `:594-597` inside `:549-604`. **Part 7's own warning was *"a coordinate that moved from a doc bullet onto executable code"*, and every one of its five replacements has now done exactly that**: `:704`, `:707` and `:710` are lines of `witness_tint_only_sink_never_moves`'s assert messages and its closing `);`, `:661` is `steady > 0.15,`, and `:748` — cited as *"the next heading"* — is `tint_elapsed_of(world, c.completing[0]).is_none(),`. **The class survives being named in the row that names it** |
| **H9 — correctly not covered, correctly not claimed** | `ui_visual_sink_on_add` fires on channel insert, and every insert precedes the armed window. No gate claims it. Nothing to do | — |

**The coverage section now names BOTH reasons a cost is invisible, and says they are independent**
(`ui_a1_zero_alloc.rs:1001-1091`; `:1093` is the next heading — re-measured by CONTENT 2026-08-28,
**part 10**, where part 7 had it as `:661-747` / `:748` — today `steady > 0.15,` and
`tint_elapsed_of(world, c.completing[0]).is_none(),`, both inside witness functions — and part 6 as
`:549-604` / `:606`). Part 5 documented only the first:
**(1) the STATISTIC can absorb it** — a cost random in the frame index. H1 was the second:
**(2) the FIXTURE can never execute it** — a perfectly deterministic cost on a branch that never
runs. *That is not a resolution question and no statistic fixes it; only the world does.*
⚠️ **Part 7 removed the site-by-site enumeration from this section**, because a prose table is the
thing that failed four passes running: it now points at the pinned data in
`tests/ui_a1_source_census.rs` (pointer at `ui_a1_zero_alloc.rs:46`, re-measured by CONTENT 2026-08-28,
part 9; it read `:685`, a doc line of a different witness — **still true at part 10, the one
coordinate in this cluster that part 9 did re-measure**), keeps only the two stated
non-coverages as prose (`:1047-1059`), and carries the planted-mutation results as a table
(`:1061-1083`) — *both re-measured by CONTENT 2026-08-28, part 10; part 7 wrote `:707-714` and
`:716-738`, which today are executable assert lines and the doc heading of
`witness_completing_cohort_completes_one_channel_per_armed_frame`*. What the section claims about
coverage is therefore no longer checked by reading it.

**The lesson part 6 owes.**

* **A gate's coverage list is a claim about which BRANCHES the fixture reaches, and it is checkable
  by planting a `panic!` rather than a cost.** H1's branch had been listed as covered through three
  passes; one panic settled it in 0.02 s. A planted allocation asks "would the statistic see this?";
  a planted panic asks the prior question, "does this code run at all?" — and part 5's acceptance
  matrix never asked it.
* **Closing a class against one SHAPE of change is not closing the class.** H2's gate was built to
  notice an upper bound and did notice every *refusal*; a *clamp* produces the identical observables
  and inverts the disclosure. The fix was to stop asserting consequences and assert the **datum** —
  the one value the code under test derives from the input — because a datum moves under every
  transformation, and consequences do not.
* **Editing a file re-anchors every citation of it, including the ones you just wrote.** Part 6's
  own H3 correction added 18 lines to `animation.rs` and thereby falsified **five** live anchors —
  one of them the H6 fix, written correct and stale three edits later. They were re-resolved by
  content afterwards, not by adding 18. *(Part 5 shipped a blanket "+20" note for exactly this
  situation; H6 is the measurement of why that does not work.)*

⚠️ **Pre-existing anchor rot found while doing the above, MEASURED and deliberately NOT edited**
(these sit in part-1/part-4 rows outside part 6's remit, and each lands on plausible content):
this document's `:907` and `:1194` cited `UiAnimationPlugin` at `animation.rs:231` and `:1190` cited
`:228`, while `:228`/`:231` are doc lines about the `5.24288e8` boundary; `:1385` cited
*"`start_tween_tint`'s only world contact is `&mut Commands`"* at `animation.rs:799-807`, which is
doc prose inside the same macro; and the D6a row's `animation.rs:255-262` for the `debug_assert!`.
The first three were stale before part 6 touched anything; the last was stale by part 5's own +20.

✅ **All four CLOSED by part 7's sweep, 2026-08-28**, re-read by content and now `animation.rs:400`
(struct) / `:402` (`impl Plugin`), `:876-877` (`pub fn $start(` and `cmds: &mut Commands`) and
`:307-314` (the `debug_assert!` block).

⚠️ **And part 6's own version of this paragraph had rotted before part 7 read it.** As part 6 wrote
it, the note gave the true sites as `:386` / `:388`, `:848` / `:849` and `:293-300` — every one
moved by the census landing (+14, +28, +14) between the writing and the next morning. *A paragraph
whose whole subject is anchor rot is not exempt from it: a correction is an anchor like any other,
and it starts rotting the moment it is written.* This is why part 7 stopped recording rot as prose
and put the coverage claim it was guarding into `tests/ui_a1_source_census.rs` instead.

#### Part 7 — the sixth adversarial pass: three gates that sampled OUTPUTS are replaced by two that pin SOURCE, and the first anchor sweep driven by the LINE SHIFT rather than by the edit set

The sixth pass affirmed everything part 6 landed — the rested cohort executes (a planted `panic!`
hangs the binary at EXIT=124 where it printed `ok` in 0.02 s), nine mutations red 25/25 each, the
floor unchanged at `[6,6,6,6]`, 100/100 idle and 60/60 under 64 spinners on 16 cores, every `f32`
figure bit-reproducible, the EN/RU twins 1:1 — and then found **the same class at four new sites**.
Three blocking, and all three share one shape: **a gate that samples the OUTPUTS of a predicate
cannot see a change that leaves those outputs alone.**

*Provenance, because this campaign's own rule is that a claim names the measurement behind it at its
own site:* the K1-K3 numbers below are **the sixth pass's and the census landing's**, taken on this
worktree — each evasion applied with an editor, the gate run unpiped with `$?` read on the next
line, the file restored and `cmp`-proved. The **anchor sweep** and every count in it are part 7's
own, and the commands that produced them are named where they are used.

| Act | What | Site |
|---|---|---|
| **K1 — BLOCKING: the repair asserted a property of the DOOR while the disclosure is a property of the SYSTEM** | `invalid_tween_duration`'s disclosure says every accepted `duration_ms` above the ceiling *"yields a row that never completes, is never reaped, and bumps `set_if_neq` on EVERY frame"*. That is a claim about `advance`, not about the door. MEASURED: one line — `if t < 1.0 && *elapsed < 3_600.0` in `advance` — falsifies it verbatim, and **1427 tests were green over it** (`boyko-ui --all-targets --release` 342, debug 341, `boyko-render --lib --tests` 744, all EXIT=0). The datum assertion was untouched (the door still stores the exact reciprocal) and the live-row assertion was untouched (5 frames × 100 ms is four orders under the cap) | closed by `ADVANCE_PIN` in `crates/boyko_ui/tests/ui_a1_source_census.rs:1571`, checked by `the_termination_condition_is_pinned_to_the_disclosure` (`:3851`); disclosure and gate named at `animation.rs:277` and `:564-576`; the live-row message re-scoped at `ui_a1_tween.rs:1064-1072` *(the two census coordinates re-measured by CONTENT 2026-08-28, part 8: they read `:705` and `:1332`, written against the 1379-line lexer census. The AST rewrite took that file to 2267 lines and `:705` became `what: "Some — the channel is live",` — a `Path` field inside `SITES`, plausible content. The part-7 review that checked this pair reported it CORRECT, and it was, on the tree it measured; the same landing that made its own prose coordinates stale moved the file these point into.* ⚠️ ***And part 8's own re-aim was stale within one landing.** Part 9 took the census from 2267 to 3181 lines; re-measured by CONTENT 2026-08-28, part 9, `:1150` reads `name: ".is_none",` — a `Call` field — and `:2205` reads `visit::visit_item(self, i);`, inside the walker. **Plausible content at both, twice in a row, for the one pair this table has re-aimed most often.** True today: ~~`:1364` and `:3119`~~ — ⚠️ ***and stale again within one landing, a THIRD time.*** Part 10's code landing took the census from 3181 to 3913 lines; re-measured by CONTENT 2026-08-28, part 10, `:1364` reads `("ui_clock_tick", ".min"),` — a `CALL_SITES` row — and `:3119` reads `pinned.len(),`. **Plausible content at both, three times running, for the one pair this table has re-aimed most often. True today: `:1571` and `:3851`.**)* |
| **K2 — BLOCKING: the datum assertion's UNIVERSAL claim was false** | Its prose claimed the datum moves *"under every transformation — clamp, floor, round, scale"* and *"for any finite X"*. Those are CLASS claims resting on three point samples. MEASURED: `let duration_ms = if duration_ms > 5.0e8 && duration_ms <= 1.0e9 { 1.0e4 } else { duration_ms };` placed immediately after the guard preserves all three arms bit-exactly, so all three datum assertions pass, while every accepted duration in `(5.24288e8, 1e9]` — squarely inside the disclosed class — completes in 10 s. **Control**, so the assertion is not merely dead: the GLOBAL `.min(1e4)` still reds at the datum assertion, `ui_a1_tween.rs:1038-1053`, EXIT=101. Re-run on the same tree, `cargo test -p boyko-ui --release --test ui_a1_tween` ⇒ EXIT=0, 14 passed — the point-sampled gate green while the census is red | prose weakened to what it pins at `ui_a1_tween.rs:907` (`# ⚠️ What the datum assertion pins is THREE POINTS, not a class`); class closed by `START_PIN` (`ui_a1_source_census.rs:1609` — re-measured by CONTENT 2026-08-28 **three times**: part 8 moved it from `:733` — by then `why: "every armed frame — offset",` — to `:1188`; part 9's +914 lines moved it again, `:1188` then reading `note: "Time's RAW delta — unclamped, unscaled, pause-blind. A field read. It is what \`; and part 10's +732 moved it once more, `:1402` now reading `("ui_visual_tick", "TweenOpacity::component_id"),` — **plausible content three times running, and each re-aim was correct on the tree it was taken on**) |
| **K3 — BLOCKING for the coverage claim: the prose table was incomplete at FOUR sites** | With `black_box(Vec::<u8>::with_capacity(64))` planted one site at a time, release, `--test-threads=1`: the reap's **zero-iteration** path (GREEN 10/10 — the completing cohort put a completion on every armed frame *by construction*, so the widening that fixed "the reap loop never runs" **swapped a blind spot for its exact complement**), `advance`'s **virtual-clock** lane (GREEN 10/10 — every fixture row passed `flags = 0`), the **`set_if_neq` EQUAL** path (GREEN 20/20 — measured, not asserted: with the fixture's own steady tint parameters `lerp_rgba8` returns `0x00000000` on all 16 frames and does not leave 0 until frame 74, so a tint-only node takes that branch every armed frame), and the missing-store `else` | fixture now five cohorts and five armed frames; `EMPTY_REAP_FRAME` at `ui_a1_zero_alloc.rs:221`; the prose table is **deleted** and replaced by a pointer at `:46`. ⚠️ **Both re-measured by CONTENT 2026-08-28, part 9, and both had been stale on PLAUSIBLE CONTENT in a file THIS round did not move** — so no shift-driven sweep would have opened them: `:203` reads `const WARM_FRAMES: usize = 8;` (a different const in the same block) and `:685` a doc line of `witness_tint_only_sink_never_moves` (*"moves — so `composed == *sink` on every armed frame and the verb takes its"*). **A sweep scoped to the diff is necessary and not sufficient; a coordinate can rot in a file the current landing never touched** |
| **K3 — the fix is a census, not a fifth table** | The table lived in the fixture's doc for four adversarial passes and **each pass found a live branch missing from it**, because a branch nobody thought of is absent from the table *and* absent from the fixture and the two absences look identical. `ui_a1_source_census.rs` scans the bodies of `ui_visual_tick`, `ui_tween_reap` and their intra-file callees and compares the **ordered** control-flow lines against a pinned table: **25 control-flow lines, 37 paths — 34 covered, 3 explicitly not; 18 calls — 6 intra-file, 12 external.** Two further tests hold the boundary, so a branch cannot escape by being factored into a new local helper. ⚠️ **Those six numbers are part 7's TRANSCRIPT of a lexer census that no longer exists** and are left as observed, under the same rule as F1 and F6: parts 8 and 9 replaced the scanner with an AST walk and widened it twice. **Measured on today's tree by part 9** (`cargo test -p boyko-ui --test ui_a1_source_census -- --nocapture`, EXIT=0): **32 control-flow nodes, 50 paths — 42 covered by 7 executable witnesses, 3 by a named gate in another binary, 5 explicitly not; 31 callees — 6 intra-file, 25 external; 50 call sites; 22 definitions, 28 distinct bare names called; 12 pinned function-value references; 0 `impl Drop` in `src/`, 5 constructed types; 2 hook registrations, 1 on the walked file, 0 observers.** Two further tests became **eight** | `SITES` `:438`, `CALLS` `:1168`; `the_branch_set_of_the_animation_systems_is_pinned` `:3076`, `the_walked_set_is_closed_under_intra_file_calls` `:3247`, `every_callee_of_the_walked_region_is_enumerated` `:3659` — **all six re-measured by CONTENT 2026-08-28, part 10; part 9's values (`:352` / `:1082` / `:2511` / `:2680` / `:2984`) were correct on the 3181-line tree and every one of them is plausible content on the 3913-line one** — `:352` a doc line, `:1082` `kind: "for",`, `:2511` `let mut defs = Vec::new();`, `:2680` a doc line, `:2984` a bare `// guess.`. ⚠️ **This cell was stale in BOTH halves, and part 8 — which re-aimed K1's and K2's coordinates in the two rows above — never opened it.** As written it read `SITES` `:211`, `CALLS` `:594`, and **three test names that no longer exist**: `the_branch_set_of_the_animation_pair_is_pinned` `:1129`, `the_scanned_set_is_closed_under_intra_file_calls` `:1230`, `every_callee_of_the_pair_is_enumerated` `:1256` — `grep -c` returns **0** for each of the three names. All five coordinates landed on plausible content: `:211` a module-doc line (*"hook scan reads the same bodies as tokens for the same reason."*), `:594` a bare `],`, `:1129` `local: false,`, `:1230` a doc-string continuation, `:1256` `Call { name: "lerp1", …`. **A renamed test is the one rot a coordinate sweep cannot repair by re-reading a line** — the citing text is wrong, not just its number — and it survived a pass whose whole subject was this file |
| **K3 — disjointness, because the last two passes fell into exactly this trap** | The reap **loop body** signals `[38,38,38,38,6]` — silent on index 4 — and the reap's **zero-iteration** path signals `[6,6,6,6,7]` — silent on 0..=3. That disjointness is what proves the fifth frame is a genuinely new path and not a fifth copy of the first four | the mutation table at `ui_a1_zero_alloc.rs:716-738` |
| **K3(d) — recorded non-coverage, not a hole** | The missing-store `else` is unreachable from the shipped schedule: `done`'s only writer is the tick, which pushes `(e, C::component_id())` only from inside `if let Some(row) = <C>`, so the store demonstrably existed one system earlier. **GREEN 10/10 is the row's evidence**, not its failure | the census's own "not covered" column |

**What the two censuses buy, stated as the property and not as the mechanism.** Every earlier gate
on this rung asked *"does the observable still look right?"* — and a clamp, a sub-range rewrite and
a second termination condition all leave the observable looking right. The censuses ask the prior
question, *"is the source still the source the disclosure describes?"*, over the whole branch set
rather than at a sampled point. All four evasions above are **RED** under them, each applied with
`Edit`, run unpiped with `$?` on the next line, restored and `cmp`-proved.

**⚠️ Anchor sweep — driven by the SHIFT, and this is the change from every prior part.** Parts 3–6
each swept anchors and each shipped fresh rot, because every one of them enumerated *the anchors it
TOUCHED* and not *the anchors its landing MOVED*. Part 7 computed the per-file line shift of the
whole landing first (`git diff -U0 e7a16fd9 -- <path>`, hunk headers folded into context bands), and
took the set to verify from that:

* **146** `file:line` citations across the four documents point into a file this landing changed —
  this document **122**, the two `OPEN-QUESTIONS.md` twins **9** each, `UI-ADVANCED-ARCHITECTURE.md`
  **6**. By target: `animation.rs` 37, `gather.rs` 29, `ui_a1_tween.rs` 19, `sprite.rs` 17,
  `layout.rs` 13, `UI-ADVANCED-ARCHITECTURE.md` 10, `ui_a1_zero_alloc.rs` 9, `text/measure.rs` 5,
  this document 5, `miri_a1_tween.rs` 1, `ui_a1_sink_reaches_discovery.rs` 1. **130** sit in a band
  the landing shifted; **79** are ranges whose END sits in a shifted band.
* ⚠️ **A basename matcher is required, not a suffix matcher.** Counted by suffix the total is
  **149**, because `"…/tests/ui_a1_zero_alloc.rs".endswith("zero_alloc.rs")` is true and three
  citations of the **pre-existing, untouched** `crates/boyko_ui/tests/zero_alloc.rs` (E6's flake —
  `OPEN-QUESTIONS.md:3989` and its twin, this document's E6 row) get swept in. `git diff
  --name-only e7a16fd9 -- crates/boyko_ui/tests/zero_alloc.rs` is empty, so those three are not in
  scope at all. *The two files whose names are a suffix of each other are exactly the pair this
  campaign already had to keep apart by hand.*
* Every one was resolved by **reading the target line and quoting it**, never by adding an offset.
* The rot is concentrated exactly where the landing is: `animation.rs` (+270 at EOF), the two A1
  test binaries (+583 / +620 at EOF). `gather.rs`, `layout.rs`, `text/measure.rs` and the two
  `boyko_render` files carry citations that were **verified correct at both ends by content**.
* ⚠️ **One class was found and deliberately NOT edited: a doc anchor that MIRRORS a source anchor.**
  The five-way `Or<..>` survey — `light_system.rs:989`, `layout.rs:88`, `layout.rs:118`,
  `text/measure.rs:61`, `gather.rs`'s own — appears in this document, in both `OPEN-QUESTIONS.md`
  twins, **and in `gather.rs:96-97`, which is the source of truth the docs copy.** Measured:
  `layout.rs:88` is exact (the `#[allow]`, its ten-way `Or<(` at `:92`), `layout.rs:118` is 4 short
  of its 3-way `Or<` at `:122`, and `text/measure.rs:61` is 4 short of the `#[allow]` at `:65` and
  8 short of its `Or<` at `:69`. **Correcting the docs alone would DIVERGE them from the source
  comment**, which is a worse state than a uniformly imprecise one, and correcting the source is a
  code edit outside a documentary sweep. Filed, with the measured values, for whichever rung next
  edits `gather.rs`.

**The prediction this document made at its own `:1744` came true in under 24 hours.** That paragraph
named M4b's `ui_a1_tween.rs:1117` as its example of an unmechanised citation — *"correct as of
2026-08-27 and they will rot silently"* — and it rotted the next day, in the landing that wrote it,
by **+143**, onto a doc line of a different test. It was cited at three separate sites, all three
stale in the same way. **A document that can name the defect, name the example, and predict the
timing, and still ship the defect, is not short of understanding — it is short of a gate.**

**The lesson part 7 owes.**

* **A gate that samples outputs closes the shapes it was built against and nothing else.** K1 and K2
  are the same defect at two altitudes: the disclosure is a property of a SYSTEM, the datum is a
  property of one FUNCTION, and both were being checked by sampling a third thing downstream. The
  fix in both cases was to stop sampling and pin the source.
* **Widening a fixture can DELETE coverage.** Part 6's completing cohort put a completion on every
  armed frame, which is what made the reap loop reachable — and by construction made the
  zero-iteration path, *the ordinary frame of any real UI*, unreachable. **Nothing in the coverage
  claim could express that**, because a table records what is covered and never what covering it
  cost.
* **An anchor sweep must be driven by the diff, not by the edit set.** The prior four sweeps were
  each honest, each thorough over the wrong set. The set is computable — it is the shift map — and
  once you compute it the work is bounded and mechanical.
* **A correction is an anchor.** Part 6's paragraph *about* anchor rot listed five true sites; all
  five were false within a day. There is no altitude at which prose stops rotting; the only exit is
  a check that runs.

**Filed for the owner, unchanged in force from `:1744`:** the UI plans are not in
`internal_docs_anchors.rs`'s `GATED_DOCS`, so none of the 146 is machine-checked, and part 7's
sweep — like the four before it — is correct only until the next landing. Adding them is a SCOPE
call. Part 7 measured its price on both sides. **To gate: 146** citations in these four documents.
**Found stale and corrected by this sweep: 45 citation-instances over 36 distinct coordinates** —
38 in this document, 2 in each `OPEN-QUESTIONS.md` twin, 3 in `UI-ADVANCED-ARCHITECTURE.md`. That
is **31 % of the in-scope set wrong after one landing**, in a document whose previous four parts
each ran a sweep. Two more are re-pointed but keep their old coordinate on purpose, annotated as
**no longer resolving** (`UI-PLAN-ANIMATION.md:846` in both twins): a dangling anchor and a
re-aimed one are different facts, and only the first says the referent was deleted.

Everything above was verified by resolving the coordinate and reading the line, and the 139
coordinates part 7 *wrote* were re-resolved the same way after the last edit, because this
document's own history is that a correction written correct goes stale inside the same landing.

#### Part 8 — the seventh adversarial pass: the census stopped LEXING and started PARSING, and this section exists because that landing opened none

**Part 8 is named here retroactively, by part 9.** Its landing wrote its record in two places that
are not a landing note: the `(part 8)` parentheticals inside **part 7's** K1 and K2 rows above, and
the module doc of the file it rewrote. It opened no section of its own, so this note ran 7 → 9 over a
part that only prose referred to. **A record that annotates someone else's row instead of opening its
own is this ladder's signature defect one altitude up: the evidence exists, and it is not where a
reader of the landing note looks.**

*What it did*, as the tree records it rather than as part 9 re-measured it: `ui_a1_source_census.rs`
was rewritten from a regex-and-keyword **scanner** into a `syn::parse_file` **walk**, 1379 → 2267
lines. The three evasions that forced it are tabulated in the file's own module doc at
`crates/boyko_ui/tests/ui_a1_source_census.rs:25-29` — a method call the scanner keyed by bare
identifier, a `macro_rules!` body, and an `&&` — each of them live code that left the whole crate at
EXIT=0.

*What part 9 measured about it.* Part 8 re-aimed the census coordinates in the K1 and K2 rows and
**never opened K3**, whose cell named `SITES` `:211`, `CALLS` `:594` and **three test names its own
rewrite had renamed out of existence** (`grep -c` = 0 for each). Corrected in that row above, every
replacement read at its target. **A sweep scoped to "the rows I am editing" is the part-7 defect in
miniature** — and part 8 ran one while standing on the paragraph that names it.

#### Part 9 — the eighth adversarial pass, and the closing move: the claim is narrowed to what the instrument MEASURES, and the residue is named

The eighth pass returned **NOT CONVERGED** and it was right, but the interesting half is what it
affirmed. The AST walk had genuinely closed the **syntactic** class: `&&`, closures, match guards
(including one the refuter planted himself), macro invocations and qualified name collisions all
reproduce as red; all 15 recorded allocation sites reproduce with their per-index signatures; all
four termination evasions red; **40 idle + 20 under 48 spinners on 16 cores with zero false reds**.
Then it found **three BLOCKING holes and four items of debt**, and every one of the three shares a
shape the previous eight passes had never produced:

> **The census keys on SYNTAX AT A CALL SITE. Two of A1's live execution edges have no syntax at a
> call site at all** — drop glue, which the compiler inserts, and an `on_add` hook, which the kernel
> calls from an attribute one module over. Both were live in the shipped tree; both left the whole
> crate at **EXIT=0, 53 targets, 349 passed**; and the third hole let an "EXECUTABLE witness" be
> satisfied by *mentioning* the function's name.

| # | the hole | closed how |
|---|---|---|
| **P1** | `impl Drop for ZzDropCap` + `let _zz = ZzDropCap { … };` in the opacity arm — a **fourth termination condition**, same threshold as the `macro_rules!` form the census DOES catch. `Expr::Struct` matched no branch arm and no call arm; `walk_region` visits nine bodies so a top-level `impl` is never reached; `collect_defs` records the method as bare `drop` and nothing calls `.drop()` | `every_drop_impl_on_a_type_the_walked_region_constructs_is_walked` (`ui_a1_source_census.rs:2813`), over `const DROP_GLUE` (`:2796`) — a scan of **every** `.rs` under `src/` |
| **P2** | The witness neuter. One Edit — `witness_tint_only_sink_never_moves(&mut world, …)` → `let _ = witness_tint_only_sink_never_moves;` — left EXIT=0 with the witness defending **five** `SITES` paths executed **zero** times. The cause was a blanket `Expr::Path` arm in `Callees`, which made `reachable_from_tests` a **mention set, not a call graph**; `Cover::By`'s own doc claimed such a witness *"reds exactly as loudly as one that was deleted"*, and measured it did not | the blanket arm **deleted**; `FN_VALUES` (`:2294`), a 12-row allowlist for the genuine function-value sites, each with its reason; new test `every_function_value_reference_in_the_fixture_is_pinned` (`:2421`) |
| **P3** | `ui_visual_sink_on_add` — registered on every `Tween*` channel and already carrying an **unenumerated** live `if` — appeared in no census: `grep -n` for it in the census returned **zero** hits. A second `if` plus a heap allocation planted in it: EXIT=0, 349 passed | `const HOOKS` (`:2876`) + `every_hook_registered_on_the_walked_file_is_walked` (`:2891`), a **token** walk because the registration lives inside `macro_rules! tween_channel`; the hook added to `WALKED`; observers asserted **zero** so a first one reds |

**Act 1 — P2 closed, and the neuter is now LOUDER than the deletion.** The refuter's exact edit gives
**EXIT=101** and reds **twice**, once naming the `SITES` row whose witness went unreachable and once
naming the unpinned value reference. The plain deletion reds the same way, EXIT=101. `Cover::By`'s
doc claim is true for the first time.

**Act 2 — the two sizing measurements first, then P1 and P3 closed.** `impl Drop` in
`crates/boyko_ui/src/`: **0**. Types the walked region constructs: **5**. Six lines of scan, not a
subsystem — so both halves were built rather than one being waived. The refuter's exact `ZzDropCap`
placement does not compile (`E0499`: its `&mut row.elapsed` overlaps `advance`'s), so it was moved to
the end of the same opacity arm, where its `Drop` still caps `elapsed`; the census then reds naming
`ZzDropCap` in `constructed by the region` and `ZzDropCap::drop` as the missing `DROP_GLUE` body, and
the **whole-crate** comparator — the command that measured EXIT=0 / 349 passed for the refuter — goes
to **EXIT=101, 53 targets, 351 passed, 1 failed**. `HOOKS` reds on its own before the widening; after
it, the refuter's hook probe reds **three** tests. Both hook paths are `Cover::Elsewhere` and both
were measured rather than pointed at: deleting the guard reds `the_tick_composes_from_the_sink`
(the `#[test]` at `ui_a1_tween.rs:590`, failing at its `assert_eq!` on `sink_of(&world, e).offset_px[0]`,
`ui_a1_tween.rs:615`, `left: 0.0, right: -400.0` — AD12's panel jumping home); deleting the insert
reds `presence_is_running_and_the_reap_ends_it` (the `#[test]` at `:217`, panicking one frame down in
`sink_of`'s `expect("the node carries a UiVisual sink")`, `:196`).

**⚠️ What Act 2 could NOT close, and why it is not a fourth act.** The two edges it closed are the two
that have a *registration* — a `Drop` impl in this crate's `src/`, and a `#[component(on_add = …)]`
attribute naming a function in the walked file. **An execution edge with neither is still invisible,
and the scan is deliberately over-approximate only in the direction that fails closed** (`T::assoc(…)`
counts `T` as constructed). A `Drop` impl in another crate, one written inside a `macro_rules!` body,
or one on a type the region obtains from a function whose name says nothing about its return type
(`let g = make_guard();`) is out of reach — recorded as residue items 5 and 2 rather than claimed.
**That is the whole shape of the closing move: the instrument was widened where widening is cheap and
mechanical, and where it is not, the CLAIM came down to meet it.**

**Act 3 — the claim narrowed, the residue named.** Census 1 now claims *"every control-flow node
written **syntactically inside the WALKED bodies**"* — no longer *"every executable branch of the
A1/A0 systems and their intra-file callees"*, which was a claim about a **runtime reachability set**
being checked by a **syntactic walk of nine bodies in one file**. The gap is written down at the
instrument, as **THE RESIDUE**, `crates/boyko_ui/tests/ui_a1_source_census.rs:126-221` (`:126` is the
heading, `:221` its last line, `:222` blank — verified at both ends), and repeated below and in
`docs/OPEN-QUESTIONS.md` + its `docs/ru/` twin so the owner does not have to open a test file.

##### THE RESIDUE — the rung's stated limit, thirteen items

Each item names its reason; where a measurement was taken, it names it. **Items 1, 4, 6 and 7 are the
four the eighth pass raised as debt (its P4, P7, P5 and the `Cover::Elsewhere` half); each was
re-measured at the code landing rather than transcribed.**

| # | not measured | reason, and the measurement where one exists |
|---|---|---|
| 1 | **Branches inside a callee in another module or crate** | `CALLS` makes them *enumerable*, not *visible*. MEASURED: `pub(crate) fn zz_cap` in `components.rs`, called from the opacity arm, reds `every_callee_…` and `every_call_site_…` — and adding the two rows those failures ask for gives **EXIT=0, 352 passed, with the cap live**, because neither `Call::local` nor `Call::note` is checked against anything. **The answer to a new cross-module callee is a place to write a sentence** |
| 2 | **Trait impls selected by type** | `impl_type_name` reads the `Self` path's last segment; nothing resolves a receiver's type or a blanket impl. The drop scan is over-approximate in the fail-closed direction for exactly this reason |
| 3 | **Macro EXPANSIONS** | The invocation is always a node and the walk descends when the body parses as an expression list, but what a `macro_rules!` body *expands to* is not. `no_macro_in_the_walked_region_is_opaque` makes an unparseable body RED — fail-closed, not sighted |
| 4 | **`#[cfg]`-dead code is COUNTED** | MEASURED: a branch under `#[cfg(feature = "zz_never_a_real_feature")]`, compiling in **no** profile, reds the census. Fail-closed for hiding, which is the right direction, **at the price of a `SITES` row whose `why` cannot be falsified** — the shape `Cover` was rewritten to eliminate |
| 5 | **A `Drop` impl the scan cannot reach** | One in another crate, one inside a `macro_rules!` body, or one on a type obtained from `make_guard()`. The scan sees `Expr::Struct` and `T::assoc(…)`, and nothing else |
| 6 | **A coverage row pins an OBSERVABLE, not an EXECUTION COUNT** | MEASURED: `lerp_rgba8`'s `let mut shift = 0;` → `= 32;` drives the `while` body to zero iterations while the node key `shift < 32` does not move — a `let` is not a node — and census (10 passed) *and* allocation gate (2 passed) stay **EXIT=0**. One unrelated test elsewhere, `restarting_a_channel_replaces_and_rewinds_it`, does catch that particular neuter, so it is a coverage-column defect and not an escape. **The exposure is concentrated and now PRINTED rather than lore: `witness_steady_cohort_keeps_every_channel_live` is the sole defence of SEVENTEEN of the 50 paths and observes four liveness counts**; next are `completing` at 8 and `tint_only` at 7 |
| 7 | **`Cover::Elsewhere` is weaker than `Cover::By`** | It asserts the named `#[test]` exists in the named file; it does not assert that gate would fail if the path stopped executing. Both rows using it carry a hand-taken measurement of exactly that — the difference between a checked claim and an automated one |
| 8 | **The PATH decomposition of a node** | The walk sees that `if let Some(row) = tint` is one node; that it has a taken and a not-taken path is authored. What is mechanical is that no node may exist without a row and no row without a node |
| 9 | **Straight-line code that calls nothing** | `*elapsed += dt;` introduces no path and no callee. `CALL_SITES` closed the sixth pass's "a straight-line CALL to a callee already listed"; a straight-line assignment is not that |
| 10 | **`SITES` order** | Positional comparison: the four structurally identical `match advance(…)` are told apart ONLY by position. Swapping two channels' blocks reds — correct, their coverage rows differ by frame index — but the message says "edited", not "reordered" |
| 11 | **String CONTENT** | Every string literal is emptied to `""` before comparison, so a message that has become FALSE does not red. It has already bitten once — see the note on `ADVANCE_PIN` |
| 12 | **`macro_rules!` bodies as trees** | A body with `$` metavariables is not parseable Rust until expansion. `START_PIN` pins it as printed TOKENS — whitespace-, comment- and line-ending-immune, which raw bytes would not be — and the hook scan reads the same bodies as tokens for the same reason |
| 13 | **Anchor currency outside this crate** | See the P6 record below: the repository's anchor gate is green over a file set **disjoint from this landing** |

##### P6 — the anchor gate's green is about OTHER FILES, and this is the record

Re-measured for this record, on this tree, each number from the command quoted beside it.

* **What the gate checks.** `cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture`
  ⇒ **EXIT=0, 5 passed**, `ARCHITECTURE.md` 6 + `FEATURE_MAP.md` 222 +
  `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` 177 + `SYSTEMS.md` 330 = **735 anchor(s) checked, 0 stale**.
* **What it checks of this landing: nothing.** Line-numbered citations from any of those four
  documents into any of the landing's **ten** moved source files — `gather.rs`,
  `ui_a1_sink_reaches_discovery.rs`, `animation.rs`, `layout.rs`, `sprite.rs`, `text/measure.rs`,
  `miri_a1_tween.rs`, `ui_a1_tween.rs`, `ui_a1_zero_alloc.rs`, `ui_a1_source_census.rs` — measured by
  `grep -oE "<basename>:[0-9]+"` over the four documents: **0, 0, 0, 0, 0, 0, 0, 0, 0, 0**. The
  parenthesised `(N)` form cannot reach them either: no `**File:**` header in the four names any of
  the ten. The only contact of any kind is **two link-form path mentions** of
  `crates/boyko_ui/src/layout.rs` (`FEATURE_MAP.md:137`, `SYSTEMS.md:2162`), which are checked for
  **existence** and carry no line number.
* **And the gate is NOT vacuous — measured here, not transported.** `SYSTEMS.md:597`/`:599`'s two
  live anchors swapped (`enable_store.rs:206` ↔ `:299`) ⇒ **EXIT=101**, *"docs/SYSTEMS.md: 330
  anchor(s) checked, 2 stale"*, with both messages naming the symbol:
  ``…:299` does not define `test`; that line reads: pub(crate) fn swap_remove_bit(…)`` and
  ``…:206` does not define `swap_remove_bit`; that line reads: pub(crate) fn test(&self, row: usize) -> bool {``.
  Restored by inverse `Edit`; `cmp` EXIT=0 and SHA-256 identical to the pre-probe snapshot;
  `git status` does not list `docs/SYSTEMS.md`; the gate back to EXIT=0, 5 passed.

**So the gate's "735 anchors, 0 stale" is true and says nothing whatever about the sixteen files this
landing changed.** The recommendation is unchanged from part 7 and from `docs/OPEN-QUESTIONS.md`'s
2026-08-28 item — **bring the UI plans under a gate, but not by widening `GATED_DOCS` first**; that
item measured widening and found **405 of 541 flags are MISBINDINGS**, not rot, because this gate
binds an anchor to the nearest resolvable **path** mention and these plans cite bare basenames. The
owner-facing cost is filed there.

**And part 9 supplied its own evidence for why the class is not closed by the gate that exists.** The
only file this round moved is `ui_a1_source_census.rs` (2267 → 3181), and its citations rotted
*inside this document* in the three K rows above — twice for the same pair, and once onto three test
names that no longer exist. **Zero of that is visible to a green anchor gate, because the gate does
not read this document.**

##### ⚠️ A correction to part 7's own sweep doctrine: driven by the diff is NECESSARY and NOT SUFFICIENT

Part 7 replaced "sweep the anchors you TOUCHED" with "sweep the anchors your landing MOVED", and that
was the right correction — it is computable, bounded and mechanical. **Part 9 found the residual
class it does not cover.** While re-reading the K3 row, two of its coordinates into
`ui_a1_zero_alloc.rs` — a file **this round did not move at all** — were stale, and both had landed
on plausible content: `:203`, cited as `EMPTY_REAP_FRAME`, reads `const WARM_FRAMES: usize = 8;`, a
different `const` eighteen lines above it (true: `:221`); `:685`, cited as the pointer that replaced
the deleted prose table, reads a doc line of `witness_tint_only_sink_never_moves` (true: `:46`).

**A coordinate that goes stale in landing N and is missed there survives every later sweep**, because
each later sweep is scoped to its own diff and that file is not in it. The shift-driven scope is a
filter on *new* rot; it has no term for *inherited* rot, and inherited rot is the half that
accumulates. The eighth pass's `docs/**` sweep — 197 spans, 2 out of range — could not see these
either: **both are in range**, which is the failure mode this document has now called worse than
out-of-range at four separate sites and has still not gated. The honest scope is the union: *the
coordinates this landing moved* **plus** *a rotating re-read of the rest*, and only the first half is
mechanical.

##### Counts, and the gates

Every count printed by the census itself (`cargo test -p boyko-ui --test ui_a1_source_census --
--nocapture`, EXIT=0, **10 passed**):

**32 control-flow nodes** · **50 paths** — 42 by 7 executable witnesses, 3 by a named gate in another
binary, 5 explicitly not · **31 callees** — 6 intra-file, 25 external · **50 call sites** · **22
definitions**, 28 distinct bare names called · **12 pinned function-value references** · **0 `impl
Drop` in `src/`**, 5 constructed types, 0 drop bodies walked · **2 hook registrations**, 1 on the
walked file, **0 observers**.

Part 9 ran in two halves — the code landing, then this record — and the table says which half took
each number, because a transported measurement is what this ladder spent eight passes learning to
distrust.

| gate | result | taken at |
|---|---|---|
| `cargo test -p boyko-ui --all-targets --no-fail-fast` | EXIT=0 — 53 targets, **352 passed**, 0 failed | both halves, same figures |
| `cargo test -p boyko-ui --test ui_a1_source_census -- --nocapture` | EXIT=0 — **10 passed**, all counts above | both halves |
| `cargo test -p boyko-render --lib --tests --no-fail-fast` | EXIT=0 — 62 targets, **744 passed**, 0 failed | both halves |
| `cargo clippy --workspace --all-targets --keep-going -- -D warnings` | EXIT=0, **0 warnings, 0 errors** — with real `Checking boyko-ui` **and** `Checking boyko-render` lines after `touch`; 1m06s at the landing, 19.13s at the record (three files touched, not all) — not false-fresh either time | both halves |
| `cargo test -p boyko-engine --test internal_docs_anchors` | EXIT=0 — 5 passed, **735 anchors, 0 stale** | the record |
| `cargo test --release -p boyko-ui --all-targets --no-fail-fast` ×5 | EXIT=0 each — 53 targets, **353 passed**, 0 failed | the code landing; **not re-run for this record** |

**All of the following were taken at part 9's code landing and are NOT re-run for this record**, and
are marked so rather than absorbed into the table above. Stability: allocation gate **60/60** idle
green and **20/20** green under 48 spinners on 16 cores; census **60/60** idle and **25/25** under 20
spinners; the pre-existing `zero_alloc.rs:238`/`:296` flake (E6) fired in none of them. The
regression sweep was enumerated exhaustively from the coverage table rather than from the 15-site
prose record — **18 sites, one at a time, release, `--test-threads=1`: 17 RED, 1 recorded-GREEN**
(the missing-store `else`, whose recorded disposition is exactly that; the census reds in its place,
three tests). Baseline `[6,6,6,6,6]` throughout, with the four `None` arms landing on frames 0/1/2/3
respectively and the reap's zero-iteration on frame 4 alone — **the per-index resolution E1 exists
for, intact.** All four termination evasions still red, EXIT=101 each.

##### The eight passes in one paragraph, because this is the transferable part

**Rung A1 shipped code that was correct and a gate that was blind, and it took eight adversarial
passes to find out in how many ways.** The first four found the blindness on successive axes of the
**fixture**: a window with no completions in it, a floor taken along the frame axis instead of the
repetition axis, a cohort that never took the all-`None` arm, and a widening that deleted the
zero-iteration path by making every armed frame complete something. The fifth and sixth moved the
defence out of the fixture and into a **source census** — and the seventh showed that a
regex-and-keyword scanner cannot see Rust, with three constructs that were live code at EXIT=0: a
method call keyed by bare identifier, a `macro_rules!` body, and an `&&`. The AST walk closed that
class completely. Then the eighth found the last one: **two execution edges with no syntax at the
call site at all** — drop glue and a kernel-registered `on_add` hook — plus a witness that could be
neutered by mentioning its name. Each repair closed the named site and the next pass found a new one,
which is the actual lesson: **a gate's claim is a hypothesis about its own blind spots, and only an
adversary who wants to find one ever does.** What finally converged was not another widening. It was
**narrowing the claim to what the instrument measures and writing the difference down as a list** —
because the failure mode this ladder never once caught by inspection is the silent skip, and a
residue that is enumerated can at least be argued with.

*Landed set — part 10 (the remediation OF part 9; three code acts and the record's own U4).* The
**tenth** adversarial pass classified part 9's close as *"a construct the narrowed claim still
over-claims"* and raised five items. Three named execution edges or false positive assertions and
were closed by a code landing confined to **one file**,
`crates/boyko_ui/tests/ui_a1_source_census.rs` (**3181 → 3913 lines**; no tracked file was touched).
The fourth is this document's own anti-rot annotation and is closed here. The fifth is recorded as
residue rather than closed.

##### What the code landing closed, and what it could not close mechanically

| Act | The refutation | What shipped | What it does NOT close |
|---|---|---|---|
| **U1 — an operator-trait `impl` is a third execution edge with no syntax at the call site** | `Walk::visit_expr` matched `Expr::Binary` only for `And`/`Or`; `Index`, `Unary`, `Assign` and `AssignOp` matched no arm. A `Mul` impl carrying a fourth termination condition, invoked from the opacity arm, left **every headline count bit-identical** and the whole crate at EXIT=0 — **and it cost ZERO rows**, against two for the recorded cross-module escape and two for the factory escape | **Both** halves, because they are blind to different things. `OPERATOR_SITES` pins every operator EXPRESSION of the walked region — **43** of them, measured by the instrument with the table emptied (3 `ui_clock_tick`, 9 `advance`, 3 `lerp1`, 10 `lerp_rgba8`, 17 `ui_visual_tick`, 1 `ui_tween_reap`; 22 binary, 8 index, 10 assign, 3 unary, 3 assign-op) — in a table of its own, so `CALL_SITES` stays 50 rows of dispositions and the "drown the table" objection does not apply. `OPERATOR_IMPLS` + `operator_impls_in_crate` scan `boyko_ui/src` over **28** operator traits. Both probes reproduced first on the shipped tree and then red at EXIT=101 naming both tests | **An `impl` in another crate reached by token-identical source** — residue item 14. The expression pin is blind to an operand type changing under unchanged tokens; the impl scan is blind to a body next door. ⚠️ **The population figure once written here was WITHDRAWN by part 11 — see residue item 14 below, which supersedes it.** It read *"zero, because the region's operands are `f32`/`u32`/`u8`/`[f32;2]`"*; measured, the cross-crate population is **at least five**, and **four of them carry no operator syntax at the site at all** — two of the three auto-derefs plus both `for`-driven `Iterator` bodies — so the sentence named the wrong mechanism and not merely the wrong count. Residue item 14 below scopes the same measurement by sub-class |
| **U2 — the narrowed claim was still FALSE, and the compensation it claimed did not exist** | `Walk::visit_item` recorded a nested item as one node and **returned without descending**, while its comment said the contents *"are censused by the definition table and the closure assertion"*. Moving a `struct` + `impl` inside `ui_visual_tick`'s body reddened **exactly one** test, asking for two `<nested item>` rows; adding them gave EXIT=0 with the cap live and the `if` inside the operator body counted nowhere. **The compensation is call-gated, and there is no call** | **The mechanical option, because it was measured free**: zero `kind: "nested item"` rows exist in `SITES` and the comparison is ordered and element-wise, so the walk emits none either — descending costs nothing today. `visit_item` now calls `visit::visit_item(self, i)`; the false comment is deleted and replaced with the measurement that refutes it. The probe now reds **3** and the walk carries `<if> * rhs . 0 > 5.0` | Nothing here; Census 1's header claim now says *"written syntactically inside"* and means it |
| **U3 — a witness could be LAUNDERED through the allowlist** | `reachable_from_tests` added an edge whenever the `from` body merely *mentioned* `to` in value position. With `let _ = (witness, …)` plus one allowlist row the census printed *"defends 7 paths"* at EXIT=0 while the witness never ran — falsifying, in the positive, the document's sentence *"a witness that exists and is never called … reds exactly as loudly as one that was deleted."* | `ValueRef` + `NOT_AN_ARGUMENT` + `Callees::call_args`: all **12** rows must name the callee and the argument index, and no row may carry `NOT_AN_ARGUMENT`. The neuter alone now reds **2** tests including the coverage claim itself; the neuter *plus* the row that would have bought it back is **refused** by name | **The absolute sentence was NOT re-shipped in a narrowed form — it was deleted at all three sites.** Whether the receiver calls what it was handed is mechanical for **4 of 12** (`armed_floor`, `clock_floor`); the other **8** name a kernel verb whose body is in `boyko_ecs` and rest on prose. The test **prints the split** instead of implying uniformity |

**The scan's first run contradicted the brief that prescribed it.** The pass asserted `boyko_ui/src`
holds zero user operator `impl`s. It holds **three** — `PartialEq for UiTextBuffer`
(`src/binding/components.rs`), `PartialEq for UiVisual` (`src/components.rs`), `PartialOrd for
UiName` (`src/components.rs`) — and the middle one is AD11's bitwise sink equality, **five `&&` and
four `Index` operations**, and it is what `sink.set_if_neq(composed)` — the walked region's last
line — short-circuits on. It is reached by a CALL, not by an operator, so `OPERATOR_SITES` was never
the list that would find it and `CALLS`' `.set_if_neq` row is; what that row could not say and the
new one does is that the equality is written in *this* crate, one module over, and no census here
walks it. Its behavioural gate is `the_sinks_equality_is_idempotent_under_nan` in
`tests/ui_a1_tween.rs`, which makes it a **disposition, not a hole**. *A predicted-zero population
that measures three is the same defect class as a coverage table nobody planted a `panic!` into.*

##### U4 — the anti-rot annotation is UNCHECKED PROSE, and it was false at more sites than the pass found

The pass reported *"false at four sites — one of them the row whose whole subject is stale
anchors."* **Measured over the whole corpus rather than sampled: there are 41 `by CONTENT`
annotation sites across seven documents, and the false population is larger than four.** Each
coordinate below was re-resolved by opening the target and reading the line; nothing was offset.

**Cluster A — `ui_a1_zero_alloc.rs`, six sites, invalidated by part 9's own landing and re-opened by
no sweep.** Part 9 moved this file (the K3 fix replaced the prose table with a pointer and rebuilt
the fixture to five cohorts) and then scoped its sweep to the file it moved *most*.

| row | as written | true 2026-08-28 | what the stale coordinate reads today |
|---|---|---|---|
| **E1** (13 coordinates) | `:237` / `:486-490` / `:491-507` / `:509-581` / `:582-645` / `:640` / `:297` / `:298-304` / `:306-313` / `:314-321` / `:760` / `:761` / `:839-854` | `:255` / `:834-838` / `:839-855` / `:857-929` / `:930-985` / `:980` / `:391` / `:392-398` / `:400-407` / `:408-415` / `:1105` / `:1106` / `:1171-1186` | eight on plausible content — `:297` is `const VIRTUAL_SPEED: f32 = 0.5;` (*a different `const`*), `:509` `(0..NODES).map(\|_\| cmds.spawn(Node).id()).collect::<Vec<_>>()`, `:640` a bare `}`, `:760` blank, and **`:839`, cited as the comparison loop, is `fn armed_once(`** |
| **H1** | `:46-88` / `:89` / `:270-282` / `:283-295` / `:615-623` / `:626` / `:763-784` | `:58-105` / `:107` / `:332-344` / `:345-357` / `:622` called at `:945` / `:966` / `:1108-1129` | `:89`, cited as *"the next heading"*, is a `//!` continuation line; **`:626` now names its THIRD subject in three parts** |
| **H1 (arm-dependence)** | `:763-784` | `:1115-1129` | opens on a bare `///`, closes on `b.add_system(ui_tween_reap).after(tick);` — the schedule builder |
| **H4** | `:531` | `:980` | `start_all_four(&mut world, everyone.clone(), [1.0; 4], 0);` |
| **H8** | `:710-714` / `:707-714` / `:704-706` / `:661-747` / `:748` | `:1050-1059` / `:1047-1059` / `:1044-1046` / `:1001-1091` / `:1093` | all five now on executable code — `:661` is `steady > 0.15,`, `:748` is `tint_elapsed_of(world, c.completing[0]).is_none(),` |
| **the coverage paragraph + the part-7-removal paragraph** | `:661-747` / `:748` / `:707-714` / `:716-738` | `:1001-1091` / `:1093` / `:1047-1059` / `:1061-1083` | as above |

⚠️ **H4 is the row the pass meant by *"the row whose whole subject is stale anchors"*, and it had
rotted in a way no per-row sweep can see.** Part 7 moved the per-index `min` to `:640` and updated
the **E1** row but not **H4**, so for two parts this document carried two different answers to *where
is the `min`* — `:531` here, `:640` there — and part 9 invalidated both. **A document that repairs
one citation of a fact and not its twin has not repaired the fact; it has forked it.** The fork is
invisible to a sweep that walks rows, and visible only to one that walks *facts*.

⚠️ **And one site failed the rule this campaign wrote for itself — *check BOTH ends of every range*.**
F5's pair of doc-block anchors reads `animation.rs:350-362` *"(`UiAnimationSet`'s doc, 13 lines)"* and
`:367-371` *"(`UiAnimationPlugin`'s, 5 lines)"*. The first holds at both ends; **the second's END was
never checked and has been wrong through three parts** — `UiAnimationPlugin`'s doc runs `:367-398`,
**32 lines**, on through `# Containment` and `# No `new()``, and `:399` is `#[derive(Default)]`. The
row's "5 lines" is the first paragraph only. Every sweep re-aimed this pair's START and carried its
END unexamined, and a range whose start is right reads as verified. *Both ends, or it is not a range.*

**Cluster B — three cross-document coordinates, two of them cited four times each.**

⚠️ **Part 11 re-resolved all three by content and found the LIVE half correct and the OTHER half
rotted.** `:2857`, `:3105` and `:1696` all still hold, re-read at their targets 2026-08-28 (part 11)
and quoted below. What went false is the *description of the stale value* in each bullet — **and
those descriptions are anchors into this same moving document.** The ruling this section wrote was
applied to the pointer and not to the sentence saying where the pointer used to land, so the half
that was dated survived and the half that was not rotted on exactly the schedule the warning three
paragraphs down predicts. Both halves are dated transcripts now.

**Every reading below is quoted as CONTENT and dated to the tree it was taken on.** A quote does not
rot; only the line number attached to it does. That is not a stylistic preference — it is the one
form that survived this cluster, and the paragraph after the bullets shows what happened to the form
that did not.

* `crates/boyko_ui/benches/ui_animation.rs`, the dead-path citation. **True: `:2857`** — *"**Lands.**
  `crates/boyko_ui/benches/ui_animation.rs` + its `[[bench]]` entry in `Cargo.toml` —"* (re-read at
  its target as this landing's last action). It read `:663`, then `:2282`. **Part 10 recorded
  `:2282` as a blank line. Read at `:2282` on the tree part 11 inherited: prose** — *"defence out of
  the fixture and into a **source census** — and the seventh showed that a"*. Not a blank line, and
  not close to one.
* the D7 owner self-citation. **True: `:3105`**, the D7 row whose owner cell is verbatim
  *"`docs/UI-PLAN-SPRITES.md` (rung 1) or wherever it is sequenced"* (re-read at its target as this
  landing's last action). It read `:846` — **still blank, and the only value in this cluster that has
  never rotted, because a deleted referent has nothing to drift onto** — then `:2530`. **Part 10
  recorded `:2530` as the prose *"than deleted, because an implementer who builds the axis measures a
  flat line and reports it as"*. Read at `:2530` on the tree part 11 inherited: a different
  sentence** — *"*N+1* renders a strictly-moved value. The failure this catches is one frame of
  nothing at the head"* — with the prose part 10 quoted **exactly 200 lines below** the value part 10
  named.
* this document's own third protected transcript. **True: `:1696`** — `` `cargo check -p
  boyko-render --lib`: **two** errors, `E0277 UiVisual: Bundle` at `` (re-read at its target as this
  landing's last action). Four values across five parts for one pointer. **Part 10 recorded `:1673`
  as blank. Read at `:1673` on the tree part 11 inherited: `the clippy masking below.`** — with the
  nearest blank one line down. ⚠️ **And part 10's description of the value H5 quotes was false too**:
  it wrote that `:1656` *"is today the F2 ordering-axis red-ledger row"*, and read at `:1656` on the
  same tree it was *"rather than being restated here — **this note did not re-run them.** What it
  DOES pin is the part"* — prose about a remediation note, in no ledger, with the F2 row nine lines
  further down. **A FIFTH false description, and the one that shows the shape is not about stale
  values at all** — this one describes neither a stale nor a live coordinate but a *third* one,
  mentioned in passing, and it was wrong on both halves: the wrong line, and the wrong ledger row.

⚠️ **AND THIS LANDING DID IT AGAIN, TO ITS OWN READINGS, WHILE WRITING THEM DOWN.** Every reading in
the three bullets above was taken on the tree part 11 inherited and was **invalidated by the act of
recording it**: writing this cluster pushed the whole document down, so on the tree that ships,
`:2282` no longer holds the prose quoted above, `:2530` no longer holds *"*N+1* renders…"*, and
`:1673` no longer holds `the clippy masking below.` — each moved by this landing's own insertions.
**The readings above are true as dated and false as coordinates, which is exactly why they are
written as quotes with a tree attached and not as claims about a line.** The three LIVE pointers were
re-read at their targets **after the last content edit**, per this section's own rule; nothing else in
this cluster is a live pointer, by construction. *The eleventh pass set out to repair descriptions
that rot when the document moves, and could not write the repair without moving the document. The
only form that survives that is the one that does not name a line.*

⚠️ **The 200 is not a coincidence and it is the whole finding.** `:2506` → `:2706` is the ripple this
section's own warning measures for the *live* pointer, in four steps, inside one landing. The
*stale-value description* sat in the same bullet, was never re-read, and moved by the same 200 — which
is provable in exactly one of the three cases, because only one of the three descriptions has content
to match: on the tree part 11 inherited, part 10's D7 quote stood **200 lines below** the value part
10 named for it. The other two say **"is a blank line"**, and **a blank line cannot be re-resolved by
content, because it has none** — the +200 carries over to them by the same arithmetic and by nothing
stronger, which is stated here as the inference it is rather than as a reading. The repair
method this campaign prescribes for a rotted anchor — *open the target and read it* — is structurally
inapplicable to precisely the two descriptions that rotted. For those, a dated transcript is not the
better form; it is **the only available one**. *(On the tree part 11 inherited, `:2282 + 200` was blank, consistent with the
same ripple, and that is an inference from the one case that could be proved — not a reading.)*

⚠️ **The site count was wrong in both directions, and the reason is mechanical.** The bullets above
used to end *"Cited from `docs/OPEN-QUESTIONS.md`, its `docs/ru/` twin, and `UI-PLAN-SPRITES.md`"* —
**a count of DOCUMENTS**. Two of those documents carry the sentence **twice**, in unrelated sections
that were written by different sweeps. Enumerated by grep over the five documents, part 11:

**The carriers are named by document and section, NOT by line.** Listing 21 line numbers across five
documents that this very landing rewrites would have produced 21 fresh instances of the defect the
table exists to record — measured, not feared: every coordinate in the first draft of this table was
already wrong by the time the table was finished. *Where a table of anchors would itself be an anchor,
name the thing instead.*

| the false description | carrier sites |
|---|---|
| *"`:2282` is a blank line"* | **6** — this bullet; `OPEN-QUESTIONS.md` twice (the Cluster-B bullet and the dead-PATHS sentence) and its `docs/ru/` twin twice; `UI-PLAN-SPRITES.md`'s dead-PATHS paragraph |
| *"`:2530` is prose about an unrelated axis"* | **8** — this bullet; `OPEN-QUESTIONS.md` twice (the Cluster-B bullet and the D7 dead-owner row) and its `docs/ru/` twin twice; `UI-PLAN-AETHER.md`'s D7 dependency row; `UI-PLAN-SPRITES.md` twice (its §0 rung-1 row and its S6 heading note) |
| *"`:1673` is blank / OUT of content"* | **4** — this bullet; the part-5 protected-transcript note earlier in this document; `OPEN-QUESTIONS.md`'s Cluster-B bullet and its `docs/ru/` twin |
| *"`:663`, today a sentence about `UI_FALLBACK_MAX_DELTA`"* — **a FOURTH description, in no pass's list** | **1** — `UI-PLAN-SPRITES.md`'s dead-PATHS paragraph. Read at `:663` on the tree part 11 inherited: *"before the datum exists rather than after. §7 Q1's answer, when it comes, edits one line and both"*, with the nearest `UI_FALLBACK_MAX_DELTA` sentence three lines down. It carries the word **"today"**, the strongest currency claim of the five, and every pass missed it because **it names no stale coordinate to grep for** — it is the *discarded* value in a re-aim, described in passing, and nobody sweeps those |
| *"`:1656` is today the F2 ordering-axis red-ledger row"* — **a FIFTH, and it was wrong on both halves** | **2** — the part-5 protected-transcript note and this bullet, both repaired here. Read at `:1656` on the same tree: *"rather than being restated here — **this note did not re-run them.** What it DOES pin is the part"* — prose about a remediation note, with the F2 ordering-axis row nine lines further down. Part 10 wrote this sentence **while disclaiming the very quote it was correcting** |

**Twenty-one carriers of five false descriptions**, where part 10 wrote three and the eleventh pass
counted thirteen. Neither number was wrong by inattention: **13 is what you get by counting the
documents each bullet names, 3 is what you get by counting the coordinates, and 21 is what you get by
counting the sentences.** Only the third is the population that can be false. A grep for the
*coordinate* misses `UI-PLAN-SPRITES.md:1647` entirely, because that site names no stale coordinate
at all; a grep for the *document* misses the second copy in each `OPEN-QUESTIONS` twin.

⚠️ **The generalisation, and it is the reason this cluster kept re-opening.** Every pass treated
"the anchor" as the thing that rots and re-aimed it. But a re-aim writes **three** claims, not one:
the new coordinate, the old coordinate's content, and — in four of the cases above — a passing remark
about some *third* line. The first is the only one anybody re-reads, and it is the only one that has
never yet been found false. **The rot is not in the pointers this campaign maintains; it is in the
prose it writes ABOUT the pointers, which nothing has ever re-read even once.** Parts 5 through 10
each repaired a coordinate and each left a fresh description behind it, and the descriptions
outnumber the coordinates **21 to 3**.

⚠️ **All three of those replacement values were invalidated by THIS section, before it was
finished.** Inserting this landing note pushed everything below it down, so the `:2506` / `:2754`
first measured for the two cross-document coordinates became `:2686` / `:2934` — **and then
`:2696` / `:2944` when this very warning was added, and `:2706` / `:2954` by the F5 finding above
and this sentence** — **four invalidations inside one landing**, while the `:1682` first measured for
the transcript became `:1687`. All three were re-read at their targets *after the last edit* and are
correct as this part leaves the tree; the values in this paragraph's own history are left as the
successive readings they were. **The point is not the arithmetic — it is that a
document cannot record a coordinate into itself without moving it, so the re-read has to be the LAST
action of the landing, never a step inside it.** This is the same shape as part 7's H6 (*"an offset
repairs a transcript; it never repairs a live pointer"*), reached from the other direction.

**Cluster C — eight census coordinates, invalidated by THIS part's own code landing, within hours.**
`SITES` `:352`→**`:438`**, `CALLS` `:1082`→**`:1168`**, `ADVANCE_PIN` `:1364`→**`:1571`**,
`START_PIN` `:1402`→**`:1609`**, and the four test anchors `:2511`→**`:3076`**, `:2680`→**`:3247`**,
`:2984`→**`:3659`**, `:3119`→**`:3851`**. Every one of part 9's values was **correct on the tree part
9 read**, and every one is plausible content on the 3913-line tree — `:1364` is
`("ui_clock_tick", ".min"),`, `:1402` is `("ui_visual_tick", "TweenOpacity::component_id"),`, `:2511`
is `let mut defs = Vec::new();`. **The `ADVANCE_PIN` / termination-test pair has now been re-aimed by
parts 8, 9 and 10 and landed on plausible content each time.**

##### What the annotation now says, and why it is not made checkable here

The words *"re-measured by CONTENT"* asserted a property — *this coordinate holds the content it is
described as holding* — that **nothing checks, and that has an observed shelf life of one landing**.
Three of this campaign's own re-aims were invalidated by the very next part, one of them within the
same day. A false positive assertion cannot be repaired by a caveat elsewhere, so it is not repaired
by one here either.

**The ruling: the annotation stops asserting currency and starts stating a measurement with a date
and a tree.** Every site it appears on now reads as *what was read, when, and by which part* — a
transcript, under the same rule this document already applies to red ledgers — and the rows in
Cluster A additionally carry the sentence that nothing gates them. **It is not made mechanical here,
and the reason is measured, not asserted**: `docs/OPEN-QUESTIONS.md`'s open SCOPE item shows the
existing `internal_docs_anchors` gate widened to these five documents produces **405 misbindings out
of 541 flags**, and its sensitivity control — repointing a real-rot site to `gather.rs:999999` —
moved the stale count **92 → 92**. The prerequisite is the **594** bare-basename citations that must
become link form first, which is a document-surgery rung with an owner-facing budget, already filed.
**Until that lands, the honest form of this annotation is a dated transcript, not a claim of
currency.**

##### The residue is FIFTEEN items

Part 9 closed on thirteen. The code landing adds two, both with their populations measured rather
than predicted. Source of truth with each item's full reasoning:
`crates/boyko_ui/tests/ui_a1_source_census.rs:168-363` (the `# THE RESIDUE` section; `:168` is its
heading, `:363` its last line — *read at its target 2026-08-28, part 11. Part 10's record said
`:148-300`, correct on the 3913-line tree it was written against and stale within hours of part 11's
landing; on the 4134-line tree `:148` is `//! is the body ``sink.set_if_neq(composed)`` short-circuits
on. See` and `:300` is a line of item 14's own cross-crate `Iterator` table — **plausible content at
both ends of the range, which is Cluster C recurring for the third consecutive part**. Part 9's said
`:126-221`*). Items 1–13 are unchanged in substance; **item 9
had its stated REASON refuted and rewritten**, because it excused straight-line code as introducing
*"no path and no callee"* and an operator overload introduces both while looking like straight-line
code at the site. It now reads *"straight-line code that is neither a call nor an operator"*, and the
line is drawn by construct: call → `CALL_SITES`, operator → `OPERATOR_SITES`, and what remains is a
field read or a plain `let`.

| # | what the instrument does not measure | measured population |
|---|---|---|
| 1 | branches inside a callee in **another module or crate** — enumerating a callee makes its branches *enumerable*, not *visible* | the answer to a new one is a place to write a sentence; `PartialEq for UiVisual` is now a live instance of it |
| 2 | trait impls **selected by type** — nothing here resolves a receiver's type or a blanket impl | the drop scan is over-approximate in the fail-closed direction because it cannot resolve |
| 3 | **macro EXPANSIONS** — an unparseable body is a RED, not a silent skip | fail-closed, not sighted |
| 4 | **`#[cfg]`-dead code is COUNTED** — right direction, at the price of a row whose justification cannot be falsified | measured: a branch under a feature in no profile reds and demands a row |
| 5 | a **`Drop` impl the scan cannot reach** — another crate, a `macro_rules!` body, or a type from `make_guard()` | **0 `impl Drop` in `src/`** |
| 6 | a coverage row pins an **OBSERVABLE, not an EXECUTION COUNT** | the exposure is printed, not folklore: **one witness is the sole defence of 17 of the 50 paths** |
| 7 | `Cover::Elsewhere` asserts the named `#[test]` **exists**, not that it would fail | both rows using it carry a hand-taken measurement of exactly that |
| 8 | the **PATH decomposition** of a node is authored, not derived | mechanical: no node without a row, no row without a node |
| 9 | **straight-line code that is neither a call nor an operator** *(reason rewritten this part)* | a field read or a plain `let` |
| 10 | **row ORDER** — reordering reds, but the message says "edited" | positional comparison |
| 11 | **string CONTENT** — literals are emptied before comparison | has already bitten once |
| 12 | **`macro_rules!` bodies as trees** — pinned as printed TOKENS | whitespace-, comment- and line-ending-immune; never a tree |
| 13 | **anchor currency outside this crate** | the `GATED_DOCS` gate is green over **735 anchors** in four documents and cites **zero** line numbers into any of this landing's moved files |
| **14** | **a no-syntax `impl` that is in ANOTHER CRATE** — `lerp1(from: f32, …)` becoming `lerp1(from: Px, …)` leaves `from + (to - from) * t` printed identically while `+` starts calling `Px`'s `Add` | ⚠️ **This row's guarantee was WITHDRAWN by part 11.** It said *"cross-crate: zero, because the region's operands are `f32`/`u32`/`u8`/`[f32;2]`, all primitives — which is what makes a first non-primitive one red the expression pin."* The operand list is wrong and the population is **at least five**, all reached on every armed frame: three auto-derefs — `*sink` on `Mut<'w, UiVisual>` (`std::ops::Deref for Mut`, `boyko_ecs .../query/data/mut_.rs:123`), `clock.dt_real()`/`.dt_virtual()` on `Res<'_, UiClock>` (`res.rs:42`) and `done.done.push(…)` on `ResMut<'_, UiTweenScratch>` (`resmut.rs:53`) — and two cross-crate `Iterator` bodies driven by `for` (`q.iter_entities_mut()`, and `&done` through `impl IntoIterator for &Vec<T>`). **Two of the three derefs carry no operator syntax at the site at all**, so no widening of `OPERATOR_SITES` could ever have listed them: the withdrawn sentence named the wrong mechanism, not merely the wrong count. What holds, as a property rather than a promise: *`OPERATOR_SITES` reds when the printed TOKENS of an operator expression of the walked region change — that and nothing more.* **In-crate was PREDICTED zero and MEASURED three** |
| **15** | **the observer scan's SCOPE is `CARGO_MANIFEST_DIR/src`** — a registration from a sibling crate leaves *"0 observer registrations"* true and irrelevant | **zero, latent not live**: no crate outside `boyko_ecs` calls any `OBSERVER_VERB`, and every cross-crate `#[component(on_*)]` in the tree sits in `boyko_ecs` fixtures or `aether_lang` expander tests, none naming a `boyko_ui` path. The hook scan shares the scope but covers all four hook kinds within it |

**Items 1 and 6 remain the two a next pass should attack**, and item 14 is now item 1 reached through
an operator instead of through a call — the same answer, the same cost.

##### Headline counts after the code landing

**32 nodes / 50 paths / 31 callees (6 intra-file, 25 external) / 50 call sites / 43 operator
expressions / 3 operator impls over 28 traits / 22 definitions / 12 function-value refs (4
mechanical, 8 prose) / 0 `impl Drop` / 5 constructed types / 2 hook registrations / 0 observers.**
Census tests: **10 → 12**. *(Part 9's row above records 32/50/31/50/22/12/0/5/2/0 and "two further
tests became eight"; the four new numbers are the operator pair, the function-value split and the
test count.)* ⚠️ **Two printed counts move under the operator probes and are compared against
nothing** — definitions 22→23 and constructed types 5→6 — so "bit-identical" covers the *asserted*
counts only, and these two are named rather than folded in.

**Re-read from the instrument 2026-08-28 after part 11's landing** — `cargo test -p boyko-ui --test
ui_a1_source_census -- --nocapture`, EXIT=0, **13 passed**. Every figure above is bit-identical;
the two that moved are the ones part 11 added: **1 desugaring impl over 8 desugared traits**, and
census tests **12 → 13**. *(The whole point of the widening was that it moved no asserted count —
`OPERATOR_SITES` is still 43, `OPERATOR_IMPLS` still 3 over 28 traits, so every external citation of
those numbers stayed true and the landing created no doc-rot.)*

##### The ten passes in one paragraph, because this is the transferable part

*(Left as part 10 wrote it — it is accurate about passes 1–10 and a past part's summary is a
transcript, not a live claim. **An eleventh pass followed it**; its record is the section below.)*

**Rung A1 shipped code that was correct and a gate that was blind, and it took ten adversarial passes
to find out in how many ways.** The first four found the blindness on successive axes of the
**fixture** — a window with no completions, a floor along the frame axis instead of the repetition
axis, a cohort that never took the all-`None` arm, and a widening that deleted the zero-iteration
path by giving every armed frame a completion. The fifth and sixth moved the defence into a **source
census**, and the seventh showed a regex-and-keyword scanner cannot see Rust, with **three
constructs** live at EXIT=0: a method call keyed by bare identifier, a `macro_rules!` body, and an
`&&`. The AST walk closed that class. The eighth found **two execution edges with no syntax at the
call site** — drop glue and a kernel-registered `on_add` hook — plus a witness neuterable by
mentioning its name. The ninth narrowed the claim and enumerated the residue. **The tenth found a
third such edge — any operator-trait `impl`, which costs zero rows where the recorded escapes cost
two — and, worse, showed that the narrowed claim was still FALSE in two places: a nested item was
not walked while a comment claimed a compensation that is call-gated and does not exist, and one
allowlist row bought back "reachable" without buying back execution.** Each repair closed the named
site and the next pass found a new one, which is the actual lesson: *a gate's claim is a hypothesis
about its own blind spots, and only an adversary who wants to find one ever does.* What converged was
**narrowing the claim to the measurement and naming the residue** — and the last two passes added the
half that is easy to miss: **a narrowed claim still has to be TRUE.** A false positive assertion is
not repaired by a caveat elsewhere; it is repaired by deleting the assertion or by making it
checkable. That is why U3's sentence was deleted rather than softened, and why this part's own
annotation stopped claiming currency instead of claiming it more carefully.

##### Part 11 — the ELEVENTH pass, classified (c): *four positive assertions still false*

The mechanisms held. Everything the refuter re-derived from the instrument survived — the anchor gate
at **735 anchors / 0 stale**, the termination-evasion probe redding exactly its three named tests, the
allocation gate non-vacuous at pair floors `[134,134,134,134,102]` against a baseline of
`[6,6,6,6,6]`, and the function-value split printing **4 proved / 8 prose** exactly as claimed. What
was false was **prose asserted in the positive**, in four places, and three of them were closed by a
code landing again confined to `crates/boyko_ui/tests/ui_a1_source_census.rs` (**3913 → 4134 lines**;
no tracked file touched, `animation.rs` `cmp`-identical at **996** lines).

| # | the refutation | disposition |
|---|---|---|
| **W2** | **A FOURTH execution edge with no syntax at the call site: DESUGARING.** `for x in it` is a walked node; the `Iterator`/`IntoIterator` impl behind it is not. The same shape reaches `From` through `?`, `Display` through macro interpolation, `Deref` through `*`. The probe was an `impl Iterator for ZzIter` carrying a fourth termination condition, driven from the opacity arm | **Widened, because it was measured free.** `DESUGARED_TRAITS` (8) + `DESUGARED_IMPLS`, a **sibling table** served by the same `trait_impls_in_crate` scanner — not a widening of `OPERATOR_TRAITS`, so `OPERATOR_SITES` stays **43** and `OPERATOR_IMPLS` stays **3 over 28**, and every external citation of those numbers stayed true. Population measured with the table empty: **ONE**, `impl Write for UiTextBuffer`. `Iterator`, `IntoIterator`, `From`, `Try`, `FromResidual`, `Display`, `Debug` are each **zero** in `boyko_ui/src` |
| **W3** | **Residue item 14's guarantee was FALSE, and it is the item that exists to state it** | Withdrawn. See the row in the residue table above: the population is **at least five**, not zero, and two of the three derefs have no operator syntax at all |
| **W4** | **`OPERATOR_SITES`' design justification asserted the exact prediction the same file brags about refuting** — the narrow option *"starts EMPTY, because the crate has none"*, in the doc of the table shipped to fix that defect | Corrected at `census:1533`. The crate has **three**, the program prints it every run, and the narrow option starts empty for a *different* reason — none of `UiTextBuffer` / `UiVisual` / `UiName` is an operand of any operator expression in the walked region. The corrected reason makes the rejection **stronger**: the narrow filter would also drop `* sink`, the one live non-primitive operand, because its `impl` is next door |
| **W5** | **False "reads today" descriptions across four documents including both `OPEN-QUESTIONS` twins** | Closed in Cluster B above. Measured **21 carriers of FIVE false descriptions**, where the refuter counted 13 and part 10 wrote 3 |

⚠️ **The `ZzIter` probe's A/B is the decisive measurement of the whole ladder, and it says the escape
was real.** On the clean tree: EXIT=0, 53 targets, 354 passed. **With the probe live and its one
`SITES` row paid: EXIT=0, 53 targets, 354 passed — identical.** Proof the body actually ran: dropping
the cap to `0.001` reds **four behavioural tests**. And post-gate, probe live, `SITES` row still
paid: **EXIT=101, and the only red is the new gate**, naming `impl Iterator for ZzIter`. *Paying the
entire price the escape used to cost no longer buys the body.*

**So the honest description of this instrument is a LIST, not a closure.** Four members across four
passes — drop glue (8th), hook registration (8th), operator dispatch (10th), desugaring (11th) —
**and every one was found by a PROBE, never by construction.** Nothing derives the class from the
language. The costs make the point: drop glue and operator dispatch cost **zero rows**, desugaring
cost **zero rows for the body**. Read a green run as *"none of the four known no-syntax edges is
unaccounted for"*, never as *"there is no no-syntax edge"*. Recorded at `ui_a1_source_census.rs:151`.

⚠️ **A transferable hazard that is not about this rung at all: A KILLED AGENT LEAVES ITS PROBE
APPLIED, AND THE PROBE CAN BE INVISIBLE TO EVERY GATE.** Three instances in one day across two lanes.
One was a one-token swap the suite is green over *by construction*; one was a loud panic nothing ran
to see; and one — the `ZzIter` above — sat **between an `#[allow]` attribute and the function it was
written for, silently re-attaching the attribute to the probe struct**. `git status` showed nothing
(the probe was inside an already-modified file), the untracked list showed nothing, and a name-based
grep misses it unless you guess the name. **The check that caught all three was `git diff --numstat`
against a known baseline.** The corollary for this record: every figure in the reports written before
that removal was measured against a tree nobody re-measured, so part 11 re-read all of them from the
instrument rather than carrying one forward.

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

> ⚠️ **BLOCKING PRECONDITION, added 2026-08-27 at the A1 remediation — A4 is the rung that inherits a
> permanently frozen picture if the ordering edge does not exist by then.** A4 puts `UiVisual` into
> `ui_pack_inputs!`, which is what makes `ui_render_discovery` filter on `Changed<UiVisual>`. A1
> MEASURED what that costs without an edge: a reader ordered BEFORE `ui_visual_tick` sees the sink's
> write **1 time in 20 animating frames**, and the one hit is an out-of-schedule insert stamp — the
> repaint is LOST, not late, because `Changed`'s window `(last_run, this_run]` is half-open
> (`schedule.rs:288` `let this_run = world.bump_change_tick();` and `:342`
> `set_change_ticks(prev_this_run, this_run)`; the comparison is `change_detection/tick.rs:169-171`,
> consumed at `filter.rs:1205`, `:1225`, `:1493`, `:1503`. **Not `schedule.rs:152`** — that is a doc
> comment about gated-system dispatch which merely quotes the same notation, and it was cited as the
> mechanism here until 2026-08-27). The reader placed AFTER hits on all 20 frames, of which **19 are
> attributable to the sink**: frame 1 is true through both `Or` arms, because the spawn stamps the
> pack-input component in that frame too. **Verified 2026-08-27: NOTHING in production registers
> `ui_render_discovery`** — the only registrations in the whole tree are in
> `crates/boyko_render/tests/*`. So there is no production edge to declare, from either side, and
> `boyko_ui` cannot declare it in any case (`boyko_render` depends on `boyko_ui`, not the reverse).
> A1 therefore shipped the edge as a **stated contract at four sites** plus a gate over the contract
> (`crates/boyko_render/tests/ui_a1_sink_reaches_discovery.rs`,
> `the_reader_must_be_ordered_after_the_tick_or_every_write_is_lost`). **Before A4 lands, either a
> render-side plugin must register `ui_render_discovery` `.after_set(UiAnimationSet)`, or A4 must
> state in its own landing note that the contract is still prose.** Creating that plugin is a SCOPE
> call (which schedule, which set, interaction with `AaPlugin`/`Render3dPlugin`, and whether
> `.after_set` on a set no plugin registered is legal on this builder) and is filed for the owner in
> `docs/OPEN-QUESTIONS.md`; a remediation rung must not invent it.

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
| ~~**Bystander rows** (same archetype, no `UiVisual`)~~ | ~~**0 / 4096**~~ | ~~AM2's seed-per-archetype term (`iter.rs:550-559`)~~ |

⚠️ **The third axis is STRUCK, 2026-08-27 — under AD10 it is a bench axis that cannot vary.** It was
derived from AM2's dense-include cost model, and AM8 refused the storage kind that model assumes. A
**table** `Mut<UiVisual>` contributes an include bit (`data/mut_.rs:250-256`), so every row of every
candidate archetype HAS `UiVisual` and "same archetype, no `UiVisual`" is unconstructible — an entity
without the sink is in a DIFFERENT archetype and is never seeded. MEASURED: 2 sink rows + 2
marker-only bystanders ⇒ **2** visits. The cited `iter.rs:550-559` per-row `dense_row_passes`
rejection is real but belongs to the DENSE-include seeding path only. Left as a struck row rather
than deleted, because an implementer who builds the axis measures a flat line and reports it as
evidence.

**Gate.**
1. The bench builds and runs; the numbers land in §4's table below with the machine and toolchain
   recorded.
2. **D12's fork is decided by the number, not by argument.** The easing partition ships **iff** the
   built-in `match` is a material fraction of the tick at 512 animating rows. If it is not, the
   partition is **deleted from the deferred list with its measurement cited**, not left as a standing
   TODO. Shipping it unmeasured would be the "arithmetic instead of a measurement" failure D23 refuses
   elsewhere.
3. **The all-`None` early-out is priced:** the `resting = 4096, animating = 8` cell is run with and
   without the `continue`, and both numbers are recorded. ~~This is the second, quantitative proof of
   A1's red mutation #2.~~ **This is the ONLY proof of AM2 (1), 2026-08-27** — A1's red mutation #2
   pointed at a change-detection gate and could not fire (AD12), because AM2 (1) is a **cost** claim
   and gate 3 cannot observe cost. **Pricing it is not licence to delete it:** under
   `#[derive(PartialEq)]` the `continue` is also the only thing containing a NaN in the sink
   (AD5 (c), MEASURED `[0,0,0]` with vs `[1,1,1]` without), and under AD11's bytewise equality the two
   concerns are independent. If the number says the `continue` is free, it stays anyway and the
   measurement is recorded as the reason it *could* have gone — it is not deleted.

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
| **M2** | §10.5 — the tween tick cost, **on the axis AM2 found** | criterion, `boyko_ui/benches/ui_animation.rs` | animating × resting ~~× bystander~~, per A8's table; and the with/without-`continue` pair at `(8, 4096)`. *(The bystander axis is struck 2026-08-27 — unconstructible under AD10's table sink; see A8.)* | A8 |
| **M3** | AD7 — the hit-test fold costs zero when nothing animates | the probe counter the interaction plan builds for §10.8 (`get_component` calls per node per frame in the focus pass) | Same tree, ~~`UiVisual` live-count 0 vs 1~~ **no archetype carrying `UiVisual` vs one** *(2026-08-27: `live_count()` is a `DenseStore` verb and AD10 makes `UiVisual` a table component — `DenseRegistry::store` returns `None` for it, silently, forever; AD7's guard and this comparison are both restated on the archetype-level check)*. The zero case **must be unchanged** from the pre-A6 baseline | A6 |
| **M4** | AM6 — the UI clamp is doing something | `dt_real` at a synthetic 2 000 ms delta | Clamped vs unclamped, **both numbers written into the rung's landing note** | A0 |
| **M4b** | AM6 — and the clamp is doing something *visible* | the same synthetic delta, with one tween running | ~~the resulting tween `elapsed` delta~~ — moved here 2026-08-26: no tween exists at A0 (`UiVisual` and the four `Tween*` land at A1), so half of M4 was unmeasurable at the rung it was assigned to, and A0's text never carried the obligation at all — leg 3 is a pass/fail assertion, not a reported comparison. **The axis, stated 2026-08-27 because A1's text did not carry the obligation either:** a running tween's `elapsed` advance across ONE synthetic 2 000 ms frame delta, **clamped vs unclamped**, both numbers written into A1's landing note, against A0's already-recorded `dt_real` 2.0 s / 0.1 s pair. **SATISFIED 2026-08-27** — clamped `dt_real` 0.1 s / `elapsed` advance 0.1 s; unclamped `dt_real` 2 s / `elapsed` advance 2 s; reported at A1, source `crates/boyko_ui/tests/ui_a1_tween.rs:1260` (`#[test]` `:1259`; the anchor read `:1117` until part 7 re-measured it by content on 2026-08-28) | A1 |

**M2's results table** *(to be filled by A8 — empty until then, and the rung does not close while it
is empty)*:

| animating | resting | ns/frame | ns/frame, no early-out |
|---|---|---|---|
| 8 | 0 | — | — |
| 8 | 4096 | — | — |
| 64 | 512 | — | — |
| 512 | 4096 | — | — |

*(the `bystanders` column is struck 2026-08-27 with A8's third axis — see there.)*

---

## 5 · Mandatory tests, invariants and benches

**Unit.** `ui_visual_default_is_the_identity` + ~~`ui_visual_does_not_derive_default`~~
**`ui_visual_identity_const_matches_its_literals`** (two routes, neither implying the other — *renamed
2026-08-27: the struck name promises a compile-time absence no Rust test can assert, because a derived
and a hand-written `impl Default` are the same trait impl; AD6 lands `UiVisual::IDENTITY` so a real
second route exists*) · **`ui_visual_partial_eq_is_bytewise` (AD11 — the plateau-tween-over-NaN leg,
`[0,0,0]`)** · **`ui_visual_is_a_table_component` + `tween_channels_are_dense` (AD10, const-asserted,
the `occlusion_marker.rs:171-176` idiom)** · **`the_tick_composes_from_the_sink` (AD12 — a finished
`TweenOffset` survives a later `TweenTint`)** · **`restarting_a_channel_replaces_and_rewinds_it`
(A1 gate 5 — the property `live_count()` cannot see)** ·
`size_of`/`align_of`/`offset_of!` pins on `UiVisual` and all four
`Tween*` · `easing_endpoints_are_exact` (all 30) · `easing_midpoint_oracle` (all 30) ·
`easing_monotone_where_it_should_be` / `easing_overshoot_is_bounded` · `linear_three_ids_one_body` ·
`easing_custom_boundary_is_const_asserted` · `state_tint_array_is_three` ·
`clock_paused_advances_real_not_virtual` · `clock_clamps_a_hitch` **(both deltas — A0 leg 3)** ·
**`clock_virtual_is_positive_clamped_and_scaled` (A0 leg 4 — the leg that did not exist; without it
`dt_virtual ≡ 0.0` passes every other leg)** · **`clock_tick_ran`** (A0 leg 1, named for what it
actually proves) · ~~`plugin_changes_no_schedule_label_set`~~ **`plugin_adds_no_shared_schedule_surface`
+ `the_probes_are_not_vacuous`** *(renamed 2026-08-26: the struck name promises a comparison of
registered schedule-label sets, and nothing in the tree can produce one — `App` has no such
enumeration and `CoreSchedule` is a closed two-variant enum. The name was baking a missing accessor
into a test title; the pair replacing it is the acting form plus its control, per A0 leg 5)* ·
**`flipbook_reads_the_virtual_delta` (A0b / AD9 (1))**.

**Property.** Over random `(from, to, duration, easing, dt-sequence)`: the tween's value is **bounded
by `[min(from,to), max(from,to)]` for the 21 non-overshooting curves**; the value at `elapsed >=
duration` is **exactly `to`** (not `to ± ULP` — the endpoint is assigned, not interpolated, and that is
a decision worth a property); a row is removed exactly once; `live_count()` returns to its starting
value after every sequence. Over random trees: AD3's composition is associative
(`(A∘B)∘C == A∘(B∘C)`) to within one ULP, so the DFS fold order cannot matter.

**`debug_assert!`.** `inv_duration.is_finite() && inv_duration > 0.0` at insert (a zero duration is
the reciprocal-of-zero trap the `inv_duration` field creates) · `opacity` within `[0, 1]` at fold
time · `scale` finite and non-negative · `elapsed >= 0.0` · the fold's output `min_px`/`size_px`
finite before the instance write (mirroring `pack_ui_instance`'s existing finite assert) ·
**`tint_mul` and `offset_px` at the SINK, added 2026-08-27** — the list covered every field but the
two the public helpers take straight from an author.

⚠️ **None of these is the NaN defence, and §5 must not be read as one.** They all compile out in
release, and this branch has recorded that *a rule about RELEASE behaviour is ungatable in debug*.
The release-side answer is **AD11**: the sink's bytewise `PartialEq` makes `set_if_neq` idempotent
under NaN, so a NaN that reaches `UiVisual` is a wrong picture on one node rather than a render gate
disarmed for the whole UI on every frame forever. The `debug_assert!`s stay because they name the
authoring mistake at the site that made it.

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
`StackIndex`, `UiImage`, `UiNineSlice`, `UiSpriteSheet` (`crates/boyko_render/src/ui/gather.rs:129-135`).
*(Was "six" at `:76-86` — written before S5 and invalidated by the rung it was written for. This
document is not in the anchors census's `GATED_DOCS`, so nothing reddened.)* `UiText` and `Children` are
**not** members — `Children` is a separate traversal probe the gather pays outside the macro, and
`UiText` is not in the list at all — so two of R4's twelve were never there. The sprites plan's S5
adds **one**, not two: `UiSpriteSheet` alone, because `UiSpriteAnim` is author configuration the pack
never reads and `UiSpriteCursor` is the flipbook's private state. **The flat arity therefore runs
6 → 7 (S5) → 8 (`UiVisual`) → 9 (the interaction plan's scroll datum), against a ceiling of 12.**

**R4's real finding survives, gets sharper, and is now RULED ON — see AM8 / AD10 (2026-08-27).** Two
things this paragraph left open are closed there: the sink IS a table component (it was written as a
condition, *"if this plan intends …"*, and this plan does intend it), and the alternative it offers —
*"or each tween system bumps `UiRenderGeneration` at its own writer"* — **does not compile**:
`UiRenderGeneration` lives in `boyko_render` (`ui/pack.rs:883`), `ui_visual_tick` lands in
`boyko_ui::animation`, and the dependency runs `boyko-render → boyko-ui`
(`boyko_render/Cargo.toml:80-82`, which states the acyclicity as a rule) while `boyko-ui` names no render crate. There is no fork. **And the
mitigation this risk hands to the seam rung grows a third item: AD13**, the const-assert arm on
`ui_pack_inputs!` that turns a dense pack input into a build error — because prose at the macro
(`gather.rs:85-89`) is exactly the "gate that cannot fail" shape, and animation is the second
subsystem to walk into it. *(AD13 LANDED 2026-08-27 as remediation act F6; the prose now points at
the assert instead of standing alone. **What the assert does and does not cover is recorded at F6's
row and in "what is NOT proved" — it is not a blanket guarantee over every `Or` list.**)*

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
   *(2026-08-26 — **two corrections.** (a) This section's own heading promises its questions are
   "also to be filed in `docs/OPEN-QUESTIONS.md`"; **this one never was**, and the number has
   meanwhile **shipped** as `pub const UI_FALLBACK_MAX_DELTA = 0.1` (`sprite.rs:320`, public API) —
   a VALUES call answered by landing it. Now filed, dated 2026-08-26. (b) The answer edits **one**
   line, not two: AD9 (3) makes `UiClock::default()` reference that const rather than restate `0.1`,
   so the flipbook and the tweens cannot come to disagree about what a hitch is.)*
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
| **AD13 — the `storage` const-assert arm on `ui_pack_inputs!`** *(added 2026-08-27)* | `docs/UI-PLAN-SPRITES.md` (seam rung), or A4 if it lands first | **A4** | Nothing reds at A4; the guard against the defect AM8 found stays a doc comment, and the next subsystem to add a dense pack input gets a frozen picture with nothing saying so |
| **The `probes/node = 8.00` prose in `ui_s0_measure.rs:240`** *(added 2026-08-27)* | this plan, at A4 | **A4** | `ui_pack_inputs!(count)` is DERIVED, so `ui_s0_discovery.rs:499-500`'s `PROBES_PER_NODE = PACK_INPUTS + 1` moves by itself when `UiVisual` becomes the eighth member — but the hand-written **8.00** in the measurement harness's doc comment does not, and `:245` already names leg (b) as blocked on this plan |
| **`impl Default for PackInput` + the 11 literal conversions** | whichever of A4 / sprites' pack rung lands **first** | **A4** | Both plans churn the same 11 test sites |
| **D7 — the `.ui` registration table** | ~~`docs/UI-PLAN-SPRITES.md` (rung 1)~~ **NOBODY** — or wherever it is sequenced | **A3** (only for authoring `UiStateTint` in `.ui`) | A3 lands with `UiStateTint` inserted from Rust only; the `.ui` spelling follows. *(2026-08-26, `UI-PLAN-SPRITES.md` S-D20 (7) — **D7 has no owning document.** This row, `UI-PLAN-SPRITES.md` §0, `UI-PLAN-ANIMATION.md:846` and `UI-PLAN-INTERACTION.md:501` each name a different owner or none, and no rung in any ladder lands it. ⚠️ **2026-08-28, part 7: `UI-PLAN-ANIMATION.md:846` no longer resolves.** Measured — `grep -n 'D7' docs/UI-PLAN-ANIMATION.md` returns **this row and nothing else** that names a D7 owner, and `:846` is a blank line under `## 3 · The rung ladder`. The fourth naming site the sentence counts is gone, so the count *"each name a different owner"* is over THREE sites, not four. Left as written rather than silently re-pointed, because a dangling coordinate and a re-aimed one are different facts and only the first says the referent was deleted. SCOPE call filed in `docs/OPEN-QUESTIONS.md`.)* `UI-PLAN-SPRITES.md` §0 explicitly **Rejected** owning D7, so this cell pointed at a refusal. |
| **`collect_candidates`' post-D16/D17 shape** | `docs/UI-PLAN-INTERACTION.md` | **A6** | Merge conflict, not a design gap — see A6's coordination note |
| **The §10.8 probe counter** | `docs/UI-PLAN-INTERACTION.md` (D23) | **M3** | M3 falls back to a wall-clock A/B, which is weaker |

**What this plan exposes, that others depend on:**

| Exposed | Consumed by |
|---|---|
| `UiClock` — ~~the one UI delta source~~ **the one UI FRAME-DELTA source**, clamped, real/virtual (AD1) | Sprites plan (the `UiSpriteCursor` flipbook — **migrated by A0b, reading `dt_virtual`**); interaction plan (`ScrollMomentum`, `HoverDwell` — **neither exists in the tree; `grep -rn` returns nothing for either**, so this cell names planned consumers, not current ones). **Fallback, recorded because this row previously implied there was none:** the sprites plan landed first, so `ui_sprite_flipbook` takes `Res<Time>` and applies AD1's clamp itself at that one site as `UI_FALLBACK_MAX_DELTA = 0.1`; ~~the replacement swaps one `SystemParam` and deletes one `min`. It does **not** adopt the raw `real_delta()` AD1 rejects.~~ **The replacement is A0b, and the field is `dt_virtual` (AD9 (1)) — named here 2026-08-26 because neither document named it, and "not the RAW `real_delta()`" left CLAMPED `dt_real` — the documented default — squarely permitted. `dt_real` reds two legs of the shipped `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`: a paused game animates and `set_relative_speed` stops working. `dt_virtual` reproduces `ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*'s arithmetic exactly, so the promise "one `SystemParam` swapped, one `min` deleted" is true only of that field.** *(`UI-PLAN-SPRITES.md` **S-D17**, 2026-08-21 — the sprites plan had named the rejected option as its fallback, so the dependency read as satisfied here and as waived there; the 2026-08-26 landing correction that chose the VIRTUAL delta was applied to the sprites plan only, and the two owner-facing documents then diverged on the axis that decides whether a paused game keeps animating.)* **`UiClock` is not the crate's only clock**: `reload/system.rs:55` throttles the hot-reload file poll on a `std::time::Instant`, a wall clock consuming no frame delta and deliberately outside this rule. |
| `UiVisual` — the Tier-1/2 sink, **a TABLE component** (AM8/AD10), its **identity** default (AD6) and its **bytewise** `PartialEq` (AD11) | Sprites plan (the pack fold reads it; the sprite lane's future `uv_shift`, AM5). **The storage kind is part of the contract, not an implementation detail:** the pack-input macro's promise — *"adding a component to `ui_pack_inputs!` wires the discovery filter for free"* — holds for table components only (`gather.rs:85-89`), so a later change of `UiVisual`'s storage silently unwires the repaint. AD13's const-assert is what makes that a build error instead — **for a NEW member; for `UiVisual` itself the guard that fires is its own const-assert at `components.rs:1117`, see "what is NOT proved" item 7.** |
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
