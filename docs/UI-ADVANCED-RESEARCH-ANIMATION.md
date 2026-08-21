# UI Advanced — Animation Research

**Campaign:** advanced UI/GUI for `boyko_ui` (sprites, animation, richer interactivity) + Aether integration.
**Scope of this document:** animation only — tweens, easing, timelines, state transitions, spring physics,
value storage and ticking, and the interaction between an animated value and a retained layout.
**Status:** research. No code is proposed as final here; the deliverable is a comparative analysis plus a
recommendation with its counter-argument.

---

## 0. Method, and what is claimed

Two kinds of statement appear below and they are kept apart deliberately.

* **Read** — a claim about `boyko_ui` / this engine, established by opening the file. Every one carries a
  `path:line`. Nothing about the existing UI was assumed.
* **Cited** — a claim about another engine, established from its source, its docs, or a primary write-up.
  Every one carries a link. Where a source was ambiguous or a fetch failed, that is said in place rather
  than papered over.

Where I could not establish something (e.g. exact per-frame costs in Godot's `Tween`), the document says
so instead of guessing.

---

## 1. What `boyko_ui` does today — established by reading

### 1.1 There is no animation, and no clock

`crates/boyko_ui/Cargo.toml` does **not** depend on anything that carries a clock, and a grep for `Time`
across `crates/boyko_ui/src` returns zero non-comment hits. Every `animat*` / `tween` / `eas*` match in the
crate is prose in a doc comment (e.g. `components.rs:6`, "a node animating only its size bumps only
`UiLayout`'s tick"). **`boyko_ui` has never read a delta time.** This is a greenfield subsystem inside a
completed one.

The engine *does* have the clock the design needs:
`crates/boyko_ecs/src/ecs/core/time/time.rs:38` — `Time`, with a hitch-clamped virtual delta
(`DEFAULT_MAX_DELTA = 250 ms`), `relative_speed`, `pause`, and a cached `delta_secs: f32`. It also carries
the real (unclamped, pause-blind) delta alongside — which matters, because a UI animation usually wants to
keep running while the game clock is paused (a pause menu that fades in on a paused `Time` never fades).
**That fork — virtual vs real clock for UI — is a decision the design must make explicitly**, and both
values are already on one resource, so it costs nothing but a field.

### 1.2 The relayout gate is global, and this is the single most important fact for animation

`crates/boyko_ui/src/layout.rs:84-134` — `ui_layout_discovery` is
`Query<(), Or<(Changed<UiLayout>, Changed<UiSpacing>, Changed<UiAlign>, Changed<UiAbsolute>,
Changed<ContentSize>, Changed<Children>, Changed<ChildOf>, Added<UiRoot>, Changed<UiAnchor>,
Changed<UiWorldProjection>)>>` reduced to **one boolean**:

```rust
scratch.dirty = inputs_changed || viewport_changed;   // layout.rs:128
```

and the crate's own doc block states the consequence (`layout.rs:24-31`):

> "When dirty, apply re-lays-out ALL roots. This relaxes the relayout granularity from per-dirty-root to
> all-roots — the only buildable option given the missing entity-yielding query".

So **one** node animating **one** layout-affecting value re-runs the whole multi-pass solver over **every
root, every frame, for the whole duration of the animation.** Today that costs nothing because nothing
animates. The moment animation ships, it becomes the steady state.

### 1.3 The engine has already met this problem once, on a much smaller scale

`crates/boyko_ui/src/widgets.rs:55` quantizes a bar's fill fraction to 1/10 000 before turning it into a
`Unit::Pct`, with the rationale written out:

> "so a mathematically-equal-but-not-bit-identical bound value (FP rounding of `current/max`) does NOT flip
> the `Pct` by an ULP and bump the fill's `Changed<UiLayout>` tick every frame (defeating the 0%-overhead
> steady state)."

And `widgets.rs:205` (`set_fill_pct_if_changed`) only acquires the `Mut` guard when the value differs.
This is the right instinct applied to a *value that changes rarely*. An animation is a value that changes
**every frame by design** — quantization cannot save it, because the changes are real. Section 5 is about
what does.

### 1.4 What the renderer can express

`crates/boyko_render/src/ui/instance.rs:35` — `UiInstance`, 64 B, `#[repr(C, align(16))]`, with a
compile-time `offset_of!` oracle per field:

| field | meaning |
| --- | --- |
| `min_px[2]`, `size_px[2]` | the rect, physical px, folded from `ComputedRect` |
| `clip[4]` | clip AABB |
| `corner_radius[4]` | per-corner radius — **aliased as the glyph UV rect when `FLAG_TEXT`** (`instance.rs:84`) |
| `color`, `border_color` | **premultiplied** RGBA8 (`premultiply_rgba8`, straight-in / premultiplied-out at pack) |
| `border_width`, `flags` | uniform border + bit flags |

There is **no per-instance transform and no per-instance opacity.** Everything positional comes from
`ComputedRect`, which only `ui_layout_apply` writes (a stated single-writer invariant). Consequently, as
the code stands today, *every* visual motion has to go through layout. That is precisely the constraint
that every system surveyed in §3 removed, and §5–§6 propose removing it here.

The header already anticipates the widening: *"A future textured/nine-slice rect (needing BOTH a radius
and a UV on one instance) retires this alias and widens `UiInstance` to 80 B — the recorded
deliberate-revisit trigger."* Sprites (this campaign) will trigger exactly that. **Animation should ride
the same widening rather than provoke a second one.**

### 1.5 A dead datum sits exactly where the design needs a live one

`crates/boyko_render/src/ui/pack.rs:196` defines `UiRenderGeneration { generation: u64 }` with a `bump()`,
documented in `ui/mod.rs:40` and `ui/upload.rs:6` as *"O(1) generation gate — short-circuits on
`gen == last_seen_generation`; a static frame does nothing."*

Grepping the workspace: `bump()` is **never called** on it, and `upload.rs` never reads `generation`
(`upload.rs` mentions it only in doc comments at lines 6, 65-68, 110, 119, 336). The only construction
sites are in `crates/boyko_render/tests/ui_pack_cpu.rs:339,349`.

**The repaint-damage gate is declared, documented, tested in isolation, and unwired.** This is the same
class of defect the diagnostics campaign catalogued as a "dead datum". It matters here because the
recommendation in §6 depends on there being a *repack-without-relayout* signal — and the signal already
exists in name. Wiring it is a prerequisite, not an extension.

### 1.6 The kernel facilities the design gets for free

| facility | where | why it matters for animation |
| --- | --- | --- |
| `Time` (virtual + real, clamped, pausable) | `boyko_ecs/.../time/time.rs:38` | the tick source; the virtual/real fork is a real design decision |
| dense (non-fragmenting) components | `boyko_ecs/.../component/dense/mod.rs` — "ONE contiguous column ... keyed by `EntityId`", tombstone + free-list, live slots never move | the correct storage kind for a channel that only some elements carry |
| `DenseQueryIter` / `dense_iter_mut` | `boyko_ecs/.../iters/query/query.rs:430,456` | bulk tick over the dense column |
| `IsEnabled<T>` / EnableTag bits | `boyko_ecs/.../iters/query/data_is_enabled.rs:123` | runtime on/off with, per `boyko_ecs/benches/enable_tags.rs:19`, "no structural-generation bump, no hook/observer fire, no deferred drain" |
| observers / hooks | `boyko_ecs/.../component/observers/` | a transition can be *started* by an observed change rather than polled |
| `Interaction` as a **tick-bearing, set-if-changed** column | `boyko_ui/src/interaction/components.rs:17` | see §7 — this is the engine's free equivalent of CSS's before-change/after-change style diff |
| Aether `machine` (Harel-lite: `initial`, `state`, `enter`/`exit`, `on EVENT => target`) | `crates/aether_lang/src/ast.rs:470-502` | a host for visual-state transitions that already exists |
| `FontId` dense-handle precedent (`components.rs:440` — "a DENSE `u32` handle ... NOT a string / `HashMap` key") | | the shape an `EasingId` / `ClipId` must copy |

---

## 2. The question, stated precisely

"Object per element or columns of values" is really **three** separable questions, and most of the
literature conflates them. Keeping them apart is what makes the comparison in §3 legible.

**(a) Where does the animation STATE live?**
An object hanging off the element (`element.animator`, `Box<dyn Tween>`, `AnimationController`), versus a
row in a column that the element merely indexes into.

**(b) What DRIVES the value?**
*Sampled* — the value is a pure function `f(t)` of elapsed time, seekable, restartable, frame-pacing
independent by construction. Versus *integrated* — the value carries `(x, v)` forward, is not seekable,
and is frame-rate dependent unless integrated exactly.

**(c) What does a changed value DIRTY?**
Layout, paint, or nothing but the composite. This is where the two-order-of-magnitude difference lives,
and it is **orthogonal** to (a) and (b) — which is the single most important thing this research found,
and the basis of the counter-argument in §9.

---

## 3. The survey

### 3.1 Summary table

| system | (a) state shape | (b) driver | (c) layout interaction |
| --- | --- | --- | --- |
| **CSS / Chromium compositor** | central `AnimationHost` list, keyed by `ElementId`; animation is *not* owned by the element | sampled (keyframes + timing function); springs must be **baked** to `linear()` | hard three-way split: composited (transform/opacity) → damage only; paint-only; layout-affecting → full pipeline |
| **Flutter** | one `AnimationController` object per animation, one `Ticker` each, listener lists | sampled (`Simulation`; also has a real spring `Simulation`) | `markNeedsPaint` vs `markNeedsLayout`; `RepaintBoundary` / relayout boundary bound the propagation |
| **WPF** | per-`DependencyProperty` animation storage on the element | sampled (`Timeline`/`Clock`) | **per-property metadata flags** `AffectsMeasure` / `AffectsArrange` / `AffectsRender` — the classification is *data on the property* |
| **Bevy** | `AnimationClip` = `HashMap<AnimationTargetId, Vec<VariableCurve>>`, `VariableCurve(pub Box<dyn AnimationCurve>)`; evaluators are trait objects on a blend stack | sampled | applies into ECS components; `bevy_ui` has **no** transition system, and a per-frame write to a `Node` field re-runs taffy (issue #22893) |
| **Godot** | `AnimationMixer` with a resolved `TrackCache`; `Tween` is a `RefCounted` object composed of per-property `Tweener` objects | sampled; Penner easings via lookup tables | `AnimationPlayer` writes properties; layout consequences are per-property |
| **Unity Animator** | `Playables` graph objects; `Animator` per object | sampled | writes properties; UI layout rebuild is separately dirty-tracked |
| **DOTween** | **one heap object per tween**, pooled and recycled | sampled | none (writes properties) |
| **PrimeTween** | pooled internals behind a `struct` handle | sampled | none |
| **LitMotion** | **`NativeArray<MotionData>` SoA + `ManagedMotionData[]`; `MotionHandle` = index + version; Burst job over the array** | sampled | none |
| **RmlUi** | per-element animation list in a retained CSS-like tree | sampled (10 easing families) | transitions fire on class/pseudo-class change; animated properties apply to the element's local style |
| **React Native** | `Animated.Value` graph; optional "native driver" | sampled | native driver **refuses** any layout property — only `transform`/`opacity` |

### 3.2 Bevy — an ECS engine that nonetheless chose the OOP shape

Bevy is the most instructive comparison because it is the closest architectural relative and it still went
the other way. Its animation *data* is:

```rust
pub struct VariableCurve(pub Box<dyn AnimationCurve>);
// AnimationClip::curves() -> &HashMap<AnimationTargetId, Vec<VariableCurve>, NoOpHash>
```

([`VariableCurve`](https://docs.rs/bevy/latest/bevy/animation/struct.VariableCurve.html),
[`AnimationClip`](https://docs.rs/bevy/latest/bevy/prelude/struct.AnimationClip.html))

Both of those types are **banned by name** in this repository — `Box<dyn Trait>` and `HashMap` are on the
hot-path prohibition list in `CLAUDE.md`, and `clippy.toml`'s `disallowed-types` fails the build on the
latter. That is not an argument that Bevy is wrong; it is an argument that **Bevy's animation crate cannot
be ported, only re-derived.**

Bevy's own reform is worth reading as evidence of where this shape hurts.
[PR #16484 "AnimatedField and Rework Evaluators"](https://github.com/bevyengine/bevy/pull/16484) removed
the `Reflect`-based downcast from the evaluator path in favour of a bespoke `Downcast` trait, and replaced
per-property marker structs with an `animated_field!(TextFont::font_size)` macro whose evaluator identity
is a `(Component TypeId, field index)` pair. The direction of travel — *identify an animated channel by a
small POD key rather than by a type-erased object* — is exactly the direction this document recommends
going further in. Bevy stopped at "fewer trait objects"; nothing forced it to stop there except the
`Reflect`-shaped asset format it had to keep loading.

Notably, `bevy_ui` has **no transition primitive at all**. The canonical button example is a system with
`Changed<Interaction>` that snaps `BackgroundColor` between three constants
([bevy/examples/ui/button.rs](https://github.com/bevyengine/bevy/blob/main/examples/ui/button.rs)). So the
largest Rust ECS UI does not solve the hover→pressed problem; it leaves it to the user.

And it has the layout defect this document warns about, in production:
[issue #22893](https://github.com/bevyengine/bevy/issues/22893) — the `bevy_ui_widgets` scrollbar wrote its
thumb's `left/right/top/bottom/width/height` every frame regardless of change, which forced
`ui_layout_system` to re-run taffy every frame. The fix was to **stop writing the layout component**, not
to make layout faster. That is the same disease `boyko_ui` will contract the day a `UiLayout` field is
tweened, amplified by the all-roots granularity of §1.2.

### 3.3 The Unity tween lineage — the one place the OOP→DOD migration is actually measured

This is the only corner of the literature with a clean before/after, because three libraries animate the
same properties in the same engine with three different storage shapes.

**DOTween** allocates a class instance per tween, pooled and recycled to blunt GC pressure
([DOTween docs](https://dotween.demigiant.com/documentation.php)).

**PrimeTween** keeps the object internally but pools everything and hands the user a `struct` handle, so
the public API never allocates.

**LitMotion** removes the object entirely. Per its
[architecture](https://deepwiki.com/annulusgames/LitMotion): a `NativeArray<MotionData<T,TOptions>>` for
the Burst-compatible state plus a parallel `ManagedMotionData[]` for callbacks — an explicit SoA split —
with `MotionHandle` as an **index + version** validated in O(1) through a sparse set, and a
`MotionUpdateJob<T,TOptions>` that interpolates every live motion in one Burst-compiled parallel pass.
Write-back to targets happens afterwards on the main thread through adapters. This is, structurally, the
same design this document arrives at for `boyko_ui` — reached independently, in a different language,
against the same constraint (no per-item object, one contiguous pass).

The numbers, from PrimeTween's own
[benchmark discussion](https://github.com/KyryloKuzyk/PrimeTween/discussions/10) (Unity 2022.3.9, M1
MacBook Pro, IL2CPP, 100 000 iterations per test):

| benchmark (ms) | DOTween | LeanTween | UnityTweens | PrimeTween |
| --- | --- | --- | --- | --- |
| Animation start | 33.54 | 15.00 | 43.18 | **5.76** |
| Position animation | 8.91 | 12.54 | 7.88 | **4.34** |
| Custom animation | 4.93 | 4.45 | 4.78 | **3.28** |
| Sequence of 3 tweens | 9.36 | 9.49 | — | **2.83** |
| Sequence start | 45.59 | **49 963.62** | — | **8.83** |

| GC per operation | DOTween | LeanTween | UnityTweens | PrimeTween |
| --- | --- | --- | --- | --- |
| Animation | 734 B | 292 B | 878 B | **0 B** |
| Delay | 584 B | 146 B | 944 B | **0 B** |
| Sequence | 2 846 B | 877 B | — | **0 B** |

Three honest readings of this table, all of which the recommendation has to survive:

1. **The headline win is allocation, not throughput.** 734 B → 0 B is a garbage-collector argument. Rust
   has no garbage collector. That column of the win **does not transfer to this engine at all** — a Rust
   object-per-tween design would allocate once and drop once, with no collection pause.
2. **The throughput win is real but small at realistic N.** Steady-state update ("Position animation") is
   8.91 → 4.34 ms per 100 000 iterations, ~2×. At a HUD's realistic 30 concurrent animations that is
   nanoseconds. The 5-8× numbers are all *start* costs, i.e. allocation again.
3. **The author's own caveats cut against the DOD case.** The same discussion records that in one
   synthetic test PrimeTween is **1.29× slower** than DOTween, and that on older Android hardware DOTween
   won outright — with the note that synthetic benchmarks "may not reflect real-world game scenarios with
   30-40 simultaneous tweens".

LeanTween's 49 963 ms sequence start is the useful outlier: it is an **O(n²) sequence startup**, which is
an algorithmic defect in the *timeline* structure, not in the per-tween storage. It is a reminder that the
timeline/sequence data structure is a separate design problem from the tick storage, and the one where
naive designs actually explode.

### 3.4 Flutter — object per animation, with an explicitly bounded blast radius

Flutter is the cleanest instance of the OOP shape done *well*. Each `AnimationController` owns a `Ticker`;
all tickers register a transient frame callback with the singleton `SchedulerBinding` and therefore
[tick in unison off one frame timestamp](https://docs.flutter.dev/ui/animations/overview). Values propagate
through listener lists.

What makes it survive is (c), not (a): an animation whose value only affects painting calls
`markNeedsPaint`, and the framework "walks the render object's parents, marking each dirty until it hits a
repaint boundary"
([`RepaintBoundary`](https://api.flutter.dev/flutter/widgets/RepaintBoundary-class.html)). Layout dirtying
is a *different* call (`markNeedsLayout`) with its own boundary. Flutter's whole animation performance
story is "make sure your animation only calls `markNeedsPaint`".

**The lesson for this engine is not Flutter's storage shape — it is that Flutter has two dirty channels
and `boyko_ui` has one.**

### 3.5 WPF — the classification as *data on the property*

WPF is 20 years old and got the part everyone else hard-codes right: `FrameworkPropertyMetadata` carries
`AffectsMeasure`, `AffectsArrange`, and `AffectsRender` flags, and the property system routes
invalidation accordingly when an effective value changes
([Framework property metadata](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/properties/framework-property-metadata)).

That is exactly the shape §6 recommends: a **static, per-channel classification** that says which dirty
signal a write to this channel raises. WPF puts it in per-property metadata because its properties are
runtime objects; this engine can put it in a `const` table indexed by a `#[repr(u8)]` channel id, which is
strictly better — it is resolved at compile time and costs a byte.

### 3.6 CSS / the Chromium compositor — the most optimised instance of this problem

Three primary facts, each load-bearing.

**Only `transform` and `opacity` composite.** David Baron's
[Running animations on the compositor thread](https://dbaron.org/log/20150916-compositor-animations)
gives the mechanism in three steps: *"(1) force the element to have an active layer, (2) set the future
values of transform or opacity (in the form of timing functions and keyframes) on the layer instead of
just the current value, and (3) let the compositor update the transform or opacity over time."*

Read step (2) carefully — it is the whole architectural insight. To hand an animation to another agent,
you must be able to hand it the **entire future of the value as a declarative function of time**. An
imperative per-frame callback cannot be handed over. This is why (b) sampled-vs-integrated is not a
stylistic choice: *sampled animations are relocatable; integrated ones are not.*

Baron also names the limitation that matters most here: an element that simultaneously animates `height`,
`width`, `top`, `right`, `bottom`, or `left` **loses** the compositor transform animation. One
layout-affecting channel poisons the whole element's fast path — the same all-or-nothing failure mode
`boyko_ui`'s single global `dirty` flag has, at element granularity instead of process granularity.

**Damage vs invalidation.** Chromium's
[How cc works](https://chromium.googlesource.com/chromium/src/+/master/docs/how_cc_works.md) distinguishes
*invalidation* (content changed → must re-raster) from *damage* (visible region changed → must re-composite).
Transform and opacity changes produce damage without invalidation. It also explains why property trees
replaced the layer hierarchy: *"the update is O(interesting nodes) instead of O(layers)."* — the same
argument as replacing `boyko_ui`'s all-roots relayout with per-root or per-subtree dirtying.

**The animation registry is central, not per-element.**
[cc/animation](https://chromium.googlesource.com/chromium/src/+/HEAD/cc/animation/README.md): *"The
`AnimationHost` has a list of currently ticking Animations ... which it iterates through whenever it
receives a tick call from the client (along with a corresponding input time)."* `ElementAnimations` is
created per target `ElementId` and *shared* by every animation targeting it. The hierarchy is
`Animation → KeyframeEffect → KeyframeModel`, where a `KeyframeModel` describes one property's animation.
`KeyframeModel`s that must start together share a **group id**.

So the most-optimised UI animation engine in existence stores animations in a **central, bulk-ticked list
keyed by a stable element handle** — not as an object owned by the element. It is still a graph of heap
objects (C++, per-animation `unique_ptr`s), so it is not the SoA shape; but the *ownership* topology is
already the ECS one.

**Springs must become tweens.** Because only declarative time-functions can be handed off, CSS gained
[`linear()`](https://developer.chrome.com/docs/css-ui/css-linear-easing-function): you sample a real spring
equation into dozens of points and hand the piecewise-linear result to the easing slot
([Chrome for Developers](https://developer.chrome.com/docs/css-ui/css-linear-easing-function),
[Smashing Magazine](https://www.smashingmagazine.com/2023/09/path-css-easing-linear-function/)). The web
chose to **destroy the spring's statefulness** in exchange for relocatability. That trade is worth naming
because this engine does not have to make it — it has no thread boundary between the ticker and the
consumer — and therefore can keep real springs. (What it loses by keeping them: seekability, and
determinism under a variable frame rate unless the integration is exact. See §8.)

**Retargeting is specified, and everyone gets it wrong.** The CSS Transitions spec's
["Starting of transitions"](https://www.w3.org/TR/css-transitions-1/) is worth reading in full for two
details:

* the **uniqueness invariant** — a transition starts only if *"the element does not have a running
  transition for the property"*, and the spec maintains *"the invariant that there is never both a running
  transition and a completed transition for the same property and element"*;
* the **reversing shortening factor** — when a running transition is retargeted before it finishes, the new
  transition's duration is scaled by the fraction the old one had traversed, so a half-completed
  hover-in that reverses takes half the time to come back, not the full time.

The uniqueness invariant is a gift: **it is what makes "one component per animated channel" a complete
model rather than a limitation** (§6.2). The reversing shortening factor is the thing a naive
implementation omits and that makes hover feedback feel wrong when the cursor flicks across a button.

### 3.7 React Native — the same rule, rediscovered independently

React Native's `useNativeDriver` ships the animation to the UI thread at start time, and the documented
restriction is *"you can only animate non-layout properties: things like `transform` and `opacity` will
work, but Flexbox and position properties will not"*
([RN Animations](https://reactnative.dev/docs/animations)). Same rule, same reason, arrived at from a
different starting point. Three independent systems (Chromium, React Native, and by construction Flutter's
paint/layout split) landing on the identical partition is the strongest evidence in this document that the
partition is intrinsic to the problem and not an artefact of any one architecture.

### 3.8 RmlUi — the closest game-engine analogue

RmlUi is a retained-mode CSS-like UI for games and is already this repo's cited precedent for
premultiplied alpha. Its
[animations/transitions](https://mikke89.github.io/RmlUiDoc/pages/rcss/animations_transitions_transforms.html)
model:

* `@keyframes` + an `animation` shorthand; ten easing families (`back`, `bounce`, `circular`, `cubic`,
  `elastic`, `exponential`, `linear`, `quadratic`, `quartic`, `quintic`, `sine`) × `-in`/`-out`/`-in-out`;
* **transitions fire only on class / pseudo-class change** — *"in RCSS, they only apply when a class or
  pseudo-class is added to or removed from an element"*. A deliberate narrowing of CSS's
  any-computed-value-change rule, which makes the trigger cheap and unambiguous;
* animations write the element's **local style**, and the docs warn against mixing RML style attributes
  and animations on the same element.

Notably the docs contain **no** statement about layout cost for animating `width`/`padding` — the example
in the docs animates `padding-left` on `:hover`. RmlUi is a system that did *not* build the tier split, and
that is a data point too: it is possible to ship without it, at a cost the docs do not quantify.

### 3.9 Godot — two systems, two shapes

`AnimationMixer` resolves each track's `NodePath` to an `Object*` **once** into a `TrackCache` rebuilt by
`_animation_set_cache_update()`, so the per-frame path is a cached pointer write rather than a path walk
([Godot animation system](https://deepwiki.com/godotengine/godot/4.7-animation-system)). That caching step
is the OOP world's expensive substitute for what an ECS gets free: in an ECS, "the resolved location of the
animated value" *is* the row, and it is stable by construction.

`Tween` is the opposite shape: a `RefCounted` object processed by `SceneTree`, composed of individual
`Tweener` objects (`PropertyTweener`, `IntervalTweener`, `CallbackTweener`, `MethodTweener`) — **an object
per interpolated property**, which is the DOTween shape. Godot's easing uses Penner equations via lookup
tables, which is a data point for §8.3.

I could not find a primary statement of Godot's per-frame `Tween` object churn cost; the DeepWiki page
explicitly does not cover allocation patterns. Treat any claim about it as unestablished.

---

## 4. What the migrations actually reported

Collecting §3.3, §3.2 and §3.6 into the answer the brief asks for:

**Who moved from object-per-element to columns, and what did they report?**

* **LitMotion (Unity, tweens)** — moved all the way to SoA `NativeArray` + Burst job + index/version
  handles. Reported: zero allocation on tween creation, and "2 to 20 times faster than other tween
  libraries" (the repo's own claim; the readable benchmark charts are images, and the numeric tables are
  not machine-readable in the README — treat the multiplier as the author's claim, and PrimeTween's
  independently reproducible table in §3.3 as the measured evidence).
* **PrimeTween (Unity, tweens)** — moved to pooled internals behind struct handles. Reported: 0 B GC,
  ~2-5× faster starts, ~2× faster steady update, **and** one synthetic case 1.29× *slower* than DOTween
  plus a loss on older Android. Publishing the loss is what makes this the trustworthy source in the set.
* **Chromium (CSS)** — moved animation ownership out of layers into a central `AnimationHost` list keyed
  by `ElementId`, alongside the layer-hierarchy → property-tree move whose reported benefit is
  *"O(interesting nodes) instead of O(layers)"*.
* **Bevy (ECS)** — moved partway: from `Reflect`-driven type-erased property access to a
  `(TypeId, field index)` evaluator key, keeping `Box<dyn AnimationCurve>` for the curve itself.
  [PR #16484](https://github.com/bevyengine/bevy/pull/16484) contains **no benchmark numbers** — the stated
  motivation is boilerplate and coupling, not throughput. Do not cite it as a performance result.

**The honest aggregate:** the reported wins from the storage-shape migration are (i) allocation/GC, which
does not apply to Rust, and (ii) throughput at N in the tens of thousands, which a UI does not reach. The
reported wins from the *dirty-classification* work (Chromium's damage/invalidation, Flutter's
paint/layout split, React Native's native driver, Bevy's #22893 fix) are the ones measured in whole frames.
**The tier split is where the performance is. The storage shape is where the architecture is.** §9
returns to this.

---

## 5. The retained-layout question

### 5.1 The rule everyone converged on

> An animated channel that cannot change any other element's size or position may be applied after layout.
> Everything else must go through layout.

Chromium enforces it by refusing to composite anything but transform/opacity. React Native enforces it by
refusing the native driver for layout properties. Flutter enforces it by having two `markNeeds*` calls.
WPF enforces it by putting `AffectsMeasure`/`AffectsArrange`/`AffectsRender` on each property's metadata.
CSS's spec-level consequence is that transform, opacity and colour are the only channels that never
participate in layout.

### 5.2 How the good ones animate a *size* without re-running layout: FLIP

When the animation genuinely is a layout change (a list reorders, a card expands), the technique is
**FLIP — First, Last, Invert, Play**
([Paul Lewis, 2015](https://aerotwist.com/blog/flip-your-animations/),
[CSS-Tricks](https://css-tricks.com/animating-layouts-with-the-flip-technique/)):

1. **First** — record the element's current rect.
2. **Last** — apply the final state and let layout run **once**, recording the new rect.
3. **Invert** — apply a transform that maps the new rect back onto the old one, so nothing visibly moved.
4. **Play** — animate that transform to identity.

Layout runs exactly **twice** (at the two endpoints) for an animation of any duration, and every
intervening frame is a Tier-2 transform animation. This converts a Tier-3 animation into a Tier-2 one.
`boyko_ui` can implement FLIP *better than the web can*, because it already has the two endpoint rects as
plain data: `ComputedRect` before and after. The "record the first rect" step is a component copy, not a
forced synchronous reflow.

FLIP is the concrete, engine-native answer to the brief's question "does an animating size re-run layout
every frame, and how do the good ones avoid that?" — **the good ones do not animate size at all; they
animate a transform between two settled layouts.**

### 5.3 Where `boyko_ui` stands relative to that rule

`boyko_ui` today has **exactly one dirty channel**, it is **global**, and it is **all-roots**. Every
channel is therefore Tier-3 by construction, including colour — because there is no way to say "repack,
don't relayout" (the gate that would say it is the dead `UiRenderGeneration` of §1.5).

Concretely, if animation ships without the tier split:

* animating a button's `UiBackground.color` on hover → `Changed<UiBackground>` is *not* in the discovery
  `Or<…>` set, so it would (correctly) not relayout — **but** nothing repacks either, because the pack gate
  is unwired. Today the colour change would either never reach the GPU or force an unconditional repack of
  every node every frame, depending on how the upload is finally wired. **This must be decided by the
  animation design, because animation is the first subsystem that changes a paint-only property
  continuously.**
* animating a panel's `UiLayout.height` for a slide-down → `Changed<UiLayout>` → `dirty` →
  **`ui_layout_apply` re-lays out every root, every frame, for the duration**.
* animating `UiText.size_px` → the measure system writes `ContentSize` → `Changed<ContentSize>` →
  same all-roots relayout. (Text size is unavoidably Tier-3; it changes intrinsic content size.)

### 5.4 A concrete, buildable fix for the all-roots granularity

`layout.rs:12-31` explains that per-root dirtying was abandoned because *"a change-detecting query cannot
yield the `Entity` handles the up-walk needs"* — `QueryData` supports `&T`/`&mut T`/`Ref`/`Mut`/`Option`/
`AnyOf`/tuples/`()` but not `Entity`.

That limitation blocks yielding an **entity**. It does not block yielding **data**. Stamping every node
with a small POD `UiRootIndex(u8)` at spawn/reparent makes

```rust
Query<&UiRootIndex, Or<(Changed<UiLayout>, Changed<UiSpacing>, …)>>
```

expressible today, and reduces the global `dirty: bool` to a `dirty_roots: u64` bitmask (8 roots is
already `LayoutScratch`'s `SEED_ROOTS`; 64 is generous). Apply then relays only the marked roots.

This is not strictly an animation feature, but animation is what converts the all-roots relayout from a
latent inefficiency into the steady state, so it belongs in this campaign's dependency set. It should be
measured before it is built — with nothing animating, the current design is provably free.

---

## 6. Three candidate models for `boyko_ui`

### 6.1 Model A — object per element (`UiAnimator` component owning a track list)

The DOTween / Godot-`Tween` / Flutter shape, transliterated: a component holding
`Vec<Tween>` or `[Option<Box<dyn Curve>>; N]`, plus a system that walks each element's list.

**Named as a Principle-0 violation.** A `Vec` of tracks inside a component is a per-element parallel data
system — the exact shape `CLAUDE.md` calls a subsystem "glued on the side", and the exact shape that
produced the O11-SP4 colored-solve race. It additionally puts `Box`/`dyn` on a per-frame path (Principle 1), makes the
per-element track count unbounded and the component non-POD/non-`Copy` (breaking the crate's uniform
`#[repr(C)] Copy` discipline in `components.rs`), and destroys per-channel change detection — one write to
the animator bumps one tick covering every channel it owns.

**The ECS-native replacement is Models B and C.** Model A is listed only so the campaign can point at it
and say "not this", because it is what every tutorial and every ported design will suggest.

### 6.2 Model B — one component per animated CHANNEL (recommended core)

One small POD component per animatable channel, e.g.

```rust
#[repr(C)]
#[derive(Component, Clone, Copy)]
pub struct TweenTint {          // 20 B
    from: u32,                  // straight RGBA8
    to:   u32,
    elapsed: f32,
    inv_duration: f32,          // reciprocal so the tick has no divide
    easing: EasingId,           // #[repr(u8)] dense handle
    flags: u8,                  // loop / ping-pong / reversing
    _pad: [u8; 2],
}
```

with siblings `TweenOpacity`, `TweenOffset` (2×f32), `TweenScale`, `TweenRadius`, `TweenSize`, and the
spring variants of §8.

**Why one per channel and not one generic row with a channel id:** because the ECS *is* the index. A
channel-tagged generic row needs a per-entity multiplicity the archetype model does not give (an entity
holds at most one instance of a component type), so a generic `UiTween { channel, … }` immediately forces
either a fixed-arity array (wasteful and capped) or an external arena (that is Model C). Splitting by
channel makes the arity question disappear — **and CSS's uniqueness invariant (§3.6) says the arity you
need per channel is exactly one.** "There is never both a running transition and a completed transition
for the same property and element" is precisely the statement that one component per channel per element
is *complete*, not a compromise. That convergence is the strongest single argument for Model B.

**Storage kind: dense.** These columns are sparse across the UI (most nodes animate nothing, and the set
that does changes constantly), so as ordinary archetype columns they would fragment the UI's archetype set
badly — a node with `{TweenTint}`, one with `{TweenTint, TweenOffset}` and one with `{TweenOffset}` are
three archetypes. `DenseStore` exists for exactly this: *"ONE contiguous column ... keyed by `EntityId`",
tombstone + free-list, live slots never move* (`dense/mod.rs`), with `dense_iter_mut` for the bulk tick.

**Capability vs state, per the standing rule.** The rule "capability = component presence; runtime on/off
= EnableTag bit" resolves the churn problem cleanly:

* **presence** of `TweenTint` = "this element is *tint-animatable*" — authored, structural, set once at
  spawn by the bundle / the Aether `style` block. A node that will never animate its tint never carries
  the column, and never pays a byte or an iteration.
* **the EnableTag bit** = "a tween is *running right now*". Starting and stopping is then an O(1) bit
  flip with, per `boyko_ecs/benches/enable_tags.rs:19`, "no structural-generation bump, no hook/observer
  fire, no deferred drain" — no archetype migration on every hover of every button.

Without that split, a button hovered 500 times pays 1 000 archetype moves. With it, it pays 1 000 bit
flips. **This is the single most important application of the engine's own rule in this design.**

**The tick.** One system per channel (or one system with several dense queries), each a flat loop over a
contiguous column:

```
t = clamp(elapsed * inv_duration, 0, 1)
u = ease(easing, t)                       // partitioned by easing id — §8.3
sink = lerp(from, to, u)                  // writes the CHANNEL's own sink column
```

No branch on element type, no pointer chase, no `dyn`, SIMD-shaped. The write-back is a **same-row write**
into the sink component — no scatter, and the ECS's own change detection then does the dirty-signalling
for free.

**Cost:** N component types instead of one. Each needs a registration, an Aether spelling, a serialize
seam, and a row in whatever manifest the campaign keeps. That is real, and §9 counts it.

### 6.3 Model C — a central animation arena (`Resource`-owned SoA columns)

The `cc::AnimationHost` / LitMotion shape: one `Resource` owning parallel columns
(`entity[]`, `channel[]`, `from[]`, `to[]`, `elapsed[]`, `inv_duration[]`, `easing[]`, `flags[]`), a
free-list, and handles as **index + version** (LitMotion's `MotionHandle`, and the engine's own `Slot`
in `boyko_utils`). One linear tick over the whole arena; write-back by entity.

Principle 0 permits this explicitly — *"`Resource`-owned columns"* is named as legitimate storage, and
`LayoutScratch` is already a precedent for a `Resource` holding engine-owned buffers.

**Strengths over B:** unbounded arity per element (a timeline with 12 simultaneous channels is 12 rows,
not 12 component types); starting/stopping is a free-list push/pop with no ECS involvement at all;
sequences/timelines are naturally expressed as a group id over contiguous rows — which is exactly what
`cc` does with `KeyframeModel` group ids, and exactly what LeanTween's O(n²) sequence start (§3.3) shows
you get wrong if you build sequences as nested objects.

**Weaknesses vs B:** the write-back is a **scatter** — a random-access `get_component_mut(entity)` per row,
which is the cache-hostile half of the loop and the part B avoids entirely. There is no free per-channel
change detection, so the arena must raise the dirty signals itself. And it re-implements a slice of the
ECS (identity, liveness, iteration) inside a resource — permitted, but it is the shape Principle 0 warns
about, and it must be justified by a capability B genuinely cannot provide.

### 6.4 The channel classification — the load-bearing table

Independent of B vs C, **this is the decision that determines whether UI animation costs microseconds or
milliseconds.** It is WPF's `AffectsMeasure`/`AffectsArrange`/`AffectsRender` as a `const` table indexed by
a `#[repr(u8)]` channel id.

| tier | channels | what a write dirties | implementation |
| --- | --- | --- | --- |
| **1 — paint** | `color`, `border_color`, `border_width`, `corner_radius`, `UiText.color`, `UiImage.tint`, `UiImage.uv_*` (sprite frames), opacity | **repack only** | write the sink component; bump `UiRenderGeneration`. Must **not** appear in the layout discovery `Or<…>` set. |
| **2 — composite** | offset x/y, scale x/y, (later) rotation | **repack only** | needs a **new per-instance visual transform** on `UiInstance` (§6.5). Layout stays the single `ComputedRect` writer. |
| **3 — layout** | `width`, `height`, padding, gaps, `UiText.size_px`, `Unit::Pct` fills | **relayout** | `Changed<UiLayout>` / `Changed<ContentSize>`. Allowed, documented as expensive, and steered toward FLIP (§5.2). |

The table must be *enforced*, not documented: a Tier-1 or Tier-2 tween writes a sink component that is
**structurally incapable** of being a layout input, because it is a different component from `UiLayout`.
That is the ECS-native form of Chromium's "only these two properties composite" — and it is enforced by
the type system rather than by a shader-side branch or a runtime check.

### 6.5 What the renderer must gain

**A per-instance visual transform.** Tier 2 does not exist without it. `UiInstance` is 64 B with a
documented widening trigger already scheduled for sprites; adding `offset_px[2]` + `scale[2]` (16 B) lands
it at 80 B — the same 80 B the header already names. The pack folds
`rect = ComputedRect ⊗ VisualTransform`, so `ComputedRect` keeps its single writer and the layout
invariant survives untouched.

*Open question for the architect:* whether the transform should be inherited down the UI tree (a
parent fading and sliding takes its children with it — the CSS/Flutter behaviour, requiring a propagation
pass) or strictly per-node (cheaper, but a "panel slides in" animation has to be applied to every child).
Inheritance is a hierarchy walk, which `ui_layout_apply` already performs; folding it into the existing
walk is likely cheaper than a second pass. **This is the biggest unresolved design question in the
animation track.**

**Opacity without a new field.** Pack already premultiplies (`premultiply_rgba8`, `instance.rs`). A
per-node `UiOpacity(f32)` column can be folded at pack — `premultiply(straight) × opacity` — costing
**zero GPU bytes** and no shader change. Strongly preferred over widening the record for opacity.

**The repack gate, wired.** `UiRenderGeneration::bump()` must actually be called by every Tier-1/Tier-2
writer, and `ui_upload` must actually read it. Until then there is no "repack without relayout" path and
the whole tier split is theoretical. See §1.5.

### 6.6 Timelines, clips and sequences — keep them out of the element

A timeline (multi-channel, multi-keyframe, possibly shared across many elements) must **not** be copied
per element. The ECS-native shape:

* **the clip is an asset** — a `Resource`-owned, immutable, contiguous keyframe table, addressed by a dense
  `ClipId` (the `FontId` precedent, `components.rs:440`: *"a DENSE `u32` handle ... NOT a string /
  `HashMap` key"*). Keyframes are POD, sorted by time, stored channel-major so a channel's evaluation is a
  contiguous scan.
* **the player is a cursor** — a per-element POD component `{ clip: ClipId, time: f32, speed: f32,
  flags: u8 }`, ~16 B. That is the entire per-element cost of playing an arbitrarily complex timeline.
* **sequencing is a group key, not a linked structure** — `cc`'s group ids, not LeanTween's nested objects
  (whose O(n²) start cost is the cautionary number in §3.3).

This also answers "does a timeline need Model C?" — no. A *player* is one component; only *ad-hoc,
simultaneous, unbounded-arity* tweens need an arena.

---

## 7. State transitions (hover → pressed) — where this engine can beat the field

### 7.1 The trigger is already free

CSS defines a transition as starting when the computed value differs between the **before-change style**
and the **after-change style** at a style change event
([css-transitions-1](https://www.w3.org/TR/css-transitions-1/)). Every implementation surveyed pays real
machinery for that diff: Chromium keeps two style objects, RmlUi narrows the trigger to class/pseudo-class
changes to make it cheap, WPF hooks the property system's effective-value change, Flutter requires the user
to call `controller.forward()` by hand.

`boyko_ui` gets it for nothing. `Interaction` (`interaction/components.rs:17`) is already a **tick-bearing
column written set-if-changed** — the module doc says so explicitly: *"Written set-if-changed, so a still
frame bumps no tick"*. Therefore:

```rust
Query<(&Interaction, &UiStateStyle, &mut TweenTint), Changed<Interaction>>
```

**is** the before-change/after-change diff, computed by the kernel, at zero marginal cost, with per-row
granularity. No system surveyed here has this for free. It is the clearest case in the campaign where the
ECS-native shape is not a compromise but a strict improvement.

### 7.2 The state→value table is an array, not a map

The declaration "hovered means tint `0xFF...`" must not be a `HashMap<State, Style>` (banned, and
mechanically blocked by `clippy.toml`). It is a POD component indexed by the enum:

```rust
#[repr(C)]
#[derive(Component, Clone, Copy)]
pub struct UiStateTint {
    /// Indexed by `Interaction as usize` — None / Hovered / Pressed.
    by_state: [u32; 3],
    duration_ms: u16,
    easing: EasingId,
    flags: u8,
}
```

This is the repo's own stated rule ("use an array indexed by `ComponentId` instead" of a `HashMap`)
applied to a three-valued enum. `Interaction` is `#[repr(u8)]` with three variants, so the index is a
cast. A `Disabled` state, if added later, is a fourth slot — and per the standing capability rule,
"disabled" should be the `Focusable`/`Interaction` EnableTag bit rather than a fourth enum variant, which
means the table stays at 3 and the disabled *appearance* is a separate component. **That is a decision to
put to the architect, not to settle here.**

### 7.3 Retargeting must be in the runtime

The reversing shortening factor (§3.6) is the detail that separates good hover feedback from bad. When a
`Changed<Interaction>` arrives while a `TweenTint` is already running, the correct action is **not** to
restart from the current constant with the full duration; it is to retarget with the duration scaled by
the traversed fraction. The `TweenTint` row already holds `elapsed` and `inv_duration`, so this is three
lines in the transition system — but only if it is specified. It is the kind of thing that is never added
later because nobody can name what feels wrong.

For springs, retargeting is where springs earn their existence: a spring **carries its velocity through a
target change**, so a flick across a button produces continuous motion rather than a restart. That is the
argument for keeping real springs (§8) rather than baking everything to `linear()` as the web had to.

### 7.4 Aether

The `machine` construct already exists (`aether_lang/src/ast.rs:470-502`: `initial`, nested `state`,
`enter`/`exit` handlers with the `system` param grammar, `on EVENT … => target`), and visual state
transitions are a state machine. Two routes:

* **(a) reuse `machine`** — the visual state is a machine, `enter` starts the tween. Costs no new
  construct, but forces every styled button to declare a machine, and puts the *values* in handler bodies
  rather than in data.
* **(b) a `style` / `widget` construct** with a `states { hovered { tint: …, in: 120ms ease_out } }` block
  that lowers to `UiStateTint` + `TweenTint` **inserts**, with **no new runtime**. This mirrors the
  existing lowering discipline exactly: `material` lowers to a builder fn, `scene` to a spawn fn, and
  `component` to a type — a `style` lowering to a component set is the same move.

**(b) is recommended.** The reversing shortening factor and the tick belong in the runtime, and the DSL
should emit data, not behaviour. Note also that `Construct::Material` is already `Box`ed because
`MaterialDef` carries seven `syn::Expr` slots (`ast.rs:172-176`); a `style` construct with a state block
will be at least as large and should be `Box`ed from the start for the same reason.

---

## 8. The driver: sampled tweens vs integrated springs

### 8.1 They are different data, not different settings

| | tween | spring |
| --- | --- | --- |
| state | `from`, `to`, `elapsed`, `duration` | `x`, `v`, `target`, `ω`, `ζ` |
| seekable | yes (`f(t)`) | no |
| deterministic under variable frame rate | yes by construction | only with exact integration |
| relocatable to another thread/process | yes (this is why CSS composites them) | no |
| behaviour when retargeted mid-flight | restarts (needs the reversing factor to feel right) | **carries velocity through** — the reason springs exist |
| terminates | at `t = duration` | asymptotically — **needs an explicit rest test** |

They should be **separate columns**, not a tagged union with a branch in the loop. Two tight
homogeneous loops beat one loop with a per-row branch, and the two have different fields anyway.

### 8.2 Springs: use the closed form, not Euler

Ryan Juckett's [Damped Springs](https://www.ryanjuckett.com/damped-springs/) derives the exact solution for
the over-, critically- and under-damped cases and reduces the per-step update to:

```
newPos = posPosCoef*oldPos + posVelCoef*oldVel
newVel = velPosCoef*oldPos + velVelCoef*oldVel
```

with the four coefficients computed **once per `(dt, ω, ζ)` triple** — the exponentials and trigonometry
live entirely in that setup, not in the loop. It is *unconditionally stable*: because it solves the ODE
analytically, no timestep causes divergence, unlike explicit Euler
([Gaffer on Games, Integration Basics](https://gafferongames.com/post/integration_basics/) for the
contrast).

For an SoA column this is the ideal shape. UI springs come from a handful of authored presets, so the
distinct `(ω, ζ)` set per frame is tiny: compute the four coefficients once per preset per frame into a
small stack array, then the inner loop over the spring column is **four multiply-adds per row against
loop-invariant scalars** — no transcendental, no branch, perfectly vectorizable. Partition the column by
preset id and each run is a straight-line SIMD kernel.

The alternative — semi-implicit (symplectic) Euler, as react-spring uses — is cheaper per step to set up
and stable enough in practice, but drifts in frequency and is frame-rate dependent unless substepped. The
closed form costs one `exp`/`sin`/`cos` triple *per preset per frame* and removes the entire class of
problem. **Recommend the closed form.**

**The rest test is mandatory, not an optimisation.** A spring approaches its target asymptotically and
never arrives. Without `|x − target| < ε && |v| < ε → disable the row`, every spring the UI has ever
started keeps ticking forever, keeps writing its sink, and keeps bumping the damage generation — the UI
never returns to a still frame. This is the animation-domain form of the crate's own set-if-changed
discipline (`widgets.rs:205`), and it is the single most likely place for this design to leak a permanent
per-frame cost.

### 8.3 Easing

`Box<dyn Fn(f32) -> f32>` is banned and would be one indirect call per row per frame besides. Four options:

1. **`#[repr(u8)]` easing id + `match` in the loop.** One branch per row. Cheap, exact, but the branch is
   inside the hot loop and defeats vectorization.
2. **Partition the column by easing id, then a monomorphic body per run.** One branch per *run*, and each
   run is a straight-line polynomial — SIMD-able. The particles P2 stage already established the
   partition-a-column-by-a-small-class-key pattern in this repository.
3. **Precomputed LUT + linear interpolation.** Branchless and uniform; Godot uses Penner easings via
   lookup tables. Costs a table load per row (cache pressure) and quantizes the curve — visible on a long,
   slow, large-amplitude tween.
4. **Analytic `cubic-bezier(p1,p2)` by Newton-Raphson**, the browsers' approach. Exact and fully general,
   but 4-8 iterations per row is far too expensive for the common path.

**Recommendation: (2) for the built-in family, with a LUT table (3) reserved for authored custom curves.**
Custom curves — including CSS-`linear()`-style baked springs (§3.6), which are a genuinely useful authoring
convenience — live in a `Resource`-owned table indexed by a dense `EasingId`, the `FontId` handle pattern.
The built-in family should be RmlUi's ten × three (`back`, `bounce`, `circular`, `cubic`, `elastic`,
`exponential`, `linear`, `quadratic`, `quartic`, `quintic` × `in`/`out`/`in-out`); it is the de facto
standard set and every authoring tool emits it.

### 8.4 Sprite flipbooks are not tweens

Sprite-frame animation (this campaign's other half) animates `UiImage.uv_min`/`uv_max` by a **step**
function of time, not a lerp — interpolating between two atlas sub-rects is meaningless and produces a
smeared frame. It needs its own Tier-1 column:

```rust
#[repr(C)]
pub struct SpriteAnim { first_frame: u16, frame_count: u16, fps: u16, elapsed: f32, flags: u8 }
```

with the atlas rect table in a `Resource`-owned column indexed by frame. Worth stating explicitly because
"it's just an animated value, use a tween" is the obvious wrong move, and `UiImage` is Tier-1, so a
flipbook must not touch layout at all. (Note `components.rs:440`'s existing caveat: *"the P5a pack path
does NOT yet consume it, so an Image renders nothing until a P5a follow-up learns `UiImage`."*)

---

## 9. Recommendation, and the strongest argument against it

### 9.1 Recommendation

**Model B — one dense, POD, per-channel component per animatable channel — plus the §6.4 tier table
enforced structurally, plus §6.5's per-instance visual transform, plus the closed-form spring as a
separate column.** Model C (the central arena) is held in reserve and adopted only if a concrete feature
demands unbounded per-element animation arity that channel columns cannot express. Timelines are clips
(assets) + players (cursors), never per-element copies.

Ranked by the size of the effect:

1. **The tier split (§6.4) and the visual transform (§6.5).** This is where whole frames are. Every system
   surveyed has it; `boyko_ui` has one global dirty flag and no transform lane.
2. **Wiring `UiRenderGeneration` (§1.5).** A prerequisite. The paint-damage path is named but does not
   exist.
3. **Capability-by-presence + EnableTag-for-running (§6.2).** Turns every animation start/stop from an
   archetype migration into a bit flip. Directly the engine's own rule.
4. **`Changed<Interaction>` as the transition trigger (§7.1), with the reversing shortening factor (§7.3).**
   Free correctness the surveyed systems pay for.
5. **The dense per-channel column (§6.2) and the closed-form spring (§8.2).** Correct, clean, Principle-0
   conformant — and, honestly, the part with the smallest measurable payoff.
6. **The Aether `style` construct lowering to components with no new runtime (§7.4).**

### 9.2 The strongest argument against this recommendation

*Stated as strongly as I can make it, because it is substantially right.*

**The storage-shape half of this recommendation is architecture, not performance, and the evidence
presented for it does not transfer.**

Every measured migration in §3.3 reports the same two wins, and this engine collects neither:

* **The allocation win is a GC win.** 734 B → 0 B per tween start is about *collection pauses*, and Rust
  has no collector. A `Box<Tween>` in Rust allocates once and drops once; a `Vec<Tween>` with reserved
  capacity allocates *never* on the steady path. PrimeTween's biggest headline number is simply not a
  number that exists in this language.
* **The throughput win appears at N a UI never reaches.** The steady-state update advantage is ~2× at
  100 000 iterations. A HUD runs 30 concurrent animations. Two times a number that is already
  microseconds is not a reason to add N component types, N Aether spellings, N serialize seams, and N
  manifest rows. And PrimeTween's own author publishes the cases where the DOD design **loses** — 1.29×
  slower in one synthetic test, and beaten outright by object-per-tween DOTween on older Android.

Worse, the argument is internally separable in a way that undercuts it: **the tier split — which is where
the actual 100× lives — is completely orthogonal to the storage shape.** Model A could implement §6.4
perfectly well: an object-per-element animator that happens to write only Tier-1 sinks gets exactly the
same frame-time result. So the honest decomposition is "the part that matters is free of the part being
argued for", and a critic can fairly say the column model is being justified by a benefit it does not
produce.

The defence is a *design-discipline* argument, not a measured one, and it should be labelled as such: a
per-channel component **cannot** write a layout input, because it is a different type from `UiLayout` and
the tick system's signature says so. An opaque per-element animator object *can* write anything, and one
day will — the tier discipline degrades to a convention that a future contributor breaks silently, and
the failure mode is exactly Bevy's #22893 (a widget quietly writing a layout field every frame,
discovered only with a `track_location` build). Types enforce; conventions erode. But that is a bet about
future maintenance, not a benchmark, and it should not be dressed up as one.

**A second, narrower objection.** The per-channel model multiplies component types, and each is
archetype-fragmenting. `TweenTint` + `TweenOffset` + `TweenScale` + `TweenOpacity` + `SpringOffset` over a
UI whose nodes animate different subsets produces a combinatorial archetype spray, and the query cost of
walking a fragmented archetype set can exceed the tick cost of the rows themselves. §6.2 answers this by
mandating **dense** storage — but that answer is load-bearing, not incidental: **if these channels are
built as ordinary archetype columns, Model B is worse than Model C, and possibly worse than Model A.**
That is a concrete, checkable failure condition and it should be written into the plan as a gate, not left
as advice.

**A third.** Splitting tween and spring columns doubles the system count and the Aether surface for a
feature (springs) whose entire distinctive benefit is velocity-preserving retargeting (§7.3, §8.1). If the
campaign does not actually ship velocity-preserving retargeting, springs should be **baked to `linear()`-
style LUT easings** exactly as the web did, and the spring column should not exist at all.

---

## 10. Open questions for the architect / owner

1. **Virtual or real clock for UI animation?** `Time` carries both (`time.rs:38`). A pause menu that fades
   in on a paused virtual clock never fades. Likely answer: real by default, virtual opt-in per channel —
   but it is a VALUES call.
2. **Does the Tier-2 visual transform inherit down the UI tree?** (§6.5.) The biggest unresolved design
   question. Inheritance matches CSS/Flutter and makes "panel slides in with its contents" one animation;
   it costs a propagation pass, which could fold into `ui_layout_apply`'s existing walk.
3. **Per-root dirty bits (§5.4)** — build now, or measure first? It is buildable today via
   `Query<&UiRootIndex, Or<…>>`, and animation is what makes it matter.
4. **Is `Disabled` a fourth `Interaction` variant or an EnableTag bit?** (§7.2.) Affects the state-table
   arity and the standing capability rule.
5. **`style` construct vs reusing `machine` in Aether** (§7.4). Recommended (b), but it is a language
   surface decision.
6. **`UiInstance` 64 → 80 B**: sprites already trigger it. Animation should ride the same widening, and
   the two campaigns must agree on the final field list **before** either widens it, or the `offset_of!`
   oracle gets re-blessed twice.

---

## 11. Sources

**Primary engine sources read in this repository** (worktree `D:/wt/ui`, branch `feat/ui-advanced`):
`crates/boyko_ui/src/layout.rs` (12-31, 84-134), `crates/boyko_ui/src/components.rs` (22, 152, 167, 219, 440),
`crates/boyko_ui/src/widgets.rs` (55, 174, 205), `crates/boyko_ui/src/interaction/components.rs` (17),
`crates/boyko_ui/src/resources.rs`, `crates/boyko_ui/src/binding/components.rs`,
`crates/boyko_ui/src/text/components.rs` (45),
`crates/boyko_render/src/ui/instance.rs` (35, 69, 84), `crates/boyko_render/src/ui/pack.rs` (196),
`crates/boyko_render/src/ui/upload.rs`, `crates/boyko_ecs/src/ecs/core/time/time.rs` (38),
`crates/boyko_ecs/src/ecs/core/component/dense/mod.rs`,
`crates/boyko_ecs/src/ecs/core/iters/query/query.rs` (430, 456),
`crates/boyko_ecs/src/ecs/core/iters/query/data_is_enabled.rs` (123),
`crates/boyko_ecs/benches/enable_tags.rs` (19), `crates/aether_lang/src/ast.rs` (149-230, 470-502).

**External:**

- [dbaron — Running animations on the compositor thread](https://dbaron.org/log/20150916-compositor-animations)
- [Chromium — How cc works](https://chromium.googlesource.com/chromium/src/+/master/docs/how_cc_works.md)
- [Chromium — cc/animation README](https://chromium.googlesource.com/chromium/src/+/HEAD/cc/animation/README.md)
- [W3C — CSS Transitions Level 1](https://www.w3.org/TR/css-transitions-1/)
- [Chrome for Developers — the `linear()` easing function](https://developer.chrome.com/docs/css-ui/css-linear-easing-function)
- [Smashing Magazine — The path to awesome CSS easing with `linear()`](https://www.smashingmagazine.com/2023/09/path-css-easing-linear-function/)
- [Paul Lewis — FLIP your animations](https://aerotwist.com/blog/flip-your-animations/) · [CSS-Tricks — Animating layouts with the FLIP technique](https://css-tricks.com/animating-layouts-with-the-flip-technique/)
- [web.dev — Animations and performance](https://web.dev/articles/animations-and-performance)
- [Bevy — `VariableCurve`](https://docs.rs/bevy/latest/bevy/animation/struct.VariableCurve.html) · [`AnimationClip`](https://docs.rs/bevy/latest/bevy/prelude/struct.AnimationClip.html) · [`animation_curves`](https://docs.rs/bevy/latest/bevy/animation/animation_curves/index.html)
- [Bevy PR #16484 — AnimatedField and Rework Evaluators](https://github.com/bevyengine/bevy/pull/16484)
- [Bevy issue #22893 — scrollbar thumb update triggers node relayout every frame](https://github.com/bevyengine/bevy/issues/22893) · [bevy/examples/ui/button.rs](https://github.com/bevyengine/bevy/blob/main/examples/ui/button.rs)
- [PrimeTween — performance comparison of Unity tween libraries](https://github.com/KyryloKuzyk/PrimeTween/discussions/10) · [PrimeTween](https://github.com/KyryloKuzyk/PrimeTween)
- [LitMotion architecture (DeepWiki)](https://deepwiki.com/annulusgames/LitMotion) · [LitMotion](https://github.com/annulusgames/LitMotion)
- [DOTween documentation](https://dotween.demigiant.com/documentation.php)
- [Godot animation system (DeepWiki)](https://deepwiki.com/godotengine/godot/4.7-animation-system)
- [Flutter — Animations API overview](https://docs.flutter.dev/ui/animations/overview) · [`RepaintBoundary`](https://api.flutter.dev/flutter/widgets/RepaintBoundary-class.html)
- [WPF — Framework property metadata](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/properties/framework-property-metadata)
- [RmlUi — Animations, transitions, and transforms](https://mikke89.github.io/RmlUiDoc/pages/rcss/animations_transitions_transforms.html)
- [React Native — Animations (`useNativeDriver`)](https://reactnative.dev/docs/animations)
- [Ryan Juckett — Damped Springs](https://www.ryanjuckett.com/damped-springs/) · [Gaffer on Games — Integration Basics](https://gafferongames.com/post/integration_basics/)
