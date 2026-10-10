> **Part of [UI-PLAN-ANIMATION.md](UI-PLAN-ANIMATION.md)** — §3 rungs A2–A8. Ladder gate: see the index.

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

