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

## The parts of this plan

**A plan is always split across several files.** This file is the index: it holds §0, the §3 ladder
gate, and §4–§9. The amendments, the decisions and the rungs live in the files below, moved on
2026-08-28 — **verbatim apart from 8 lines**, and those 8 are one class: a citation of the SIBLING
sprites plan, re-pointed at the part file that now holds its target and rewritten as an anchor link.
*(The sprites plan's own figure is **9**, stated there. Per plan rather than as one total, because a
single number hides which file carries what — and because the total, 17, collides numerically with an
unrelated 17 in the sprites index: the count of `crates/**` headers the split re-pointed. Different
populations, same digits.)*

**What the integrity check prints, before anyone runs it.** Reassembling this plan and diffing it
against the pre-split blob is the right check, and it does **not** come out empty — a reader who
inherits "no prose changed" and then sees eight differences concludes the split corrupted content:

```sh
# 2a10f3a4 is the split commit's parent — the last tree on which this plan was one file.
git show 2a10f3a4:docs/UI-PLAN-ANIMATION.md | sort > /tmp/pre.s
cat docs/UI-PLAN-ANIMATION.md docs/UI-PLAN-ANIMATION-DECISIONS.md docs/UI-PLAN-ANIMATION-A0.md \
    docs/UI-PLAN-ANIMATION-A1.md docs/UI-PLAN-ANIMATION-A2-A8.md | sort > /tmp/post.s
comm -23 /tmp/pre.s /tmp/post.s | wc -l                   # 8 — lines the split did not carry through
comm -23 /tmp/pre.s /tmp/post.s | grep -c UI-PLAN-SPRITES # 8 — every one of them cites the sibling
```

Both sides are **sorted** on purpose: the split reorders whole sections, so an ordered `diff` reports
the entire file and proves nothing. The reverse direction (`comm -13`) is **not** a clean count of
anything — it mixes those 8 rewritten counterparts with this index section, which is prose the split
added rather than moved.

| file | holds |
|---|---|
| [UI-PLAN-ANIMATION-DECISIONS.md](UI-PLAN-ANIMATION-DECISIONS.md) | §1 amendments `AM1`–`AM8` · §2 decisions `AD1`–`AD13` |
| [UI-PLAN-ANIMATION-A0.md](UI-PLAN-ANIMATION-A0.md) | §3 rung `A0` — the UI clock |
| [UI-PLAN-ANIMATION-A1.md](UI-PLAN-ANIMATION-A1.md) | §3 rung `A1` — the sink, the four channels, the fused tick |
| [UI-PLAN-ANIMATION-A2-A8.md](UI-PLAN-ANIMATION-A2-A8.md) | §3 rungs `A2`–`A8` |

Still in this file: §0 how to read · §3's unconditional gate and default-OFF ruling · §4 measurement
· §5 tests · §6 risks · §7 open questions · §8 dependencies · §9 deferred.

**Every ID, and the anchor that resolves it.** This campaign names amendments, decisions and rungs by
bare ID — `AD9`, `AM8`, `A4` — in running prose, and before the split those names resolved because
everything was one file. They are **not** rewritten as links at each of their ~500 mention sites, and
that is the decision, not an omission: a generated heading anchor encodes the *whole heading text*, so
re-wording one heading would break every link into it — the same failure the line numbers had, one
level up. The table below is the single place a heading's wording is written down twice. Re-word a
heading and repair **one row**, not the mention sites.

| ID | resolves to |
|---|---|
| `AM1` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am1--mut-uivisual-bumps-no-tick-so-d9bs-own-query-defeats-d10s-enforcement) |
| `AM2` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am2--d9bs-a-uivisual-row-with-no-live-channel-is-skipped-is-false-for-an-all-dense-anyof) |
| `AM3` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am3--d5s-fold-formula-is-correct-only-for-a-nodes-own-background-quad) |
| `AM4` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am4--the-tier-2-transform-inherits-the-architecture-never-answered-it) |
| `AM5` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am5--uv_shift-leaves-uivisual-in-v1) |
| `AM6` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am6--time-caches-no-real-delta-f32-and-the-real-delta-is-unclamped) |
| `AM7` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am7--d15s-real-delta-default-is-a-tween-lane-rule-every-consumer-without-a-per-row-flags-bit-has-already-chosen-virtual-twice-in-shipped-code) |
| `AM8` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#am8--d9-declares-uivisual-dense-and-d10-makes-it-a-term-of-ui_render_discoverys-or-those-two-cannot-both-be-true) |
| `AD1` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad1--uiclock-is-a-resource-not-a-restime-read-at-each-consumer) |
| `AD2` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad2--easingid-is-a-u8-with-a-reserved-custom-half) |
| `AD3` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad3--the-visual-transform-is-affine-origin-relative-and-composes-as-a-scale-translate-pair) |
| `AD4` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad4--the-tier-2-transform-inherits-multiplicatively-on-the-gathers-existing-dfs) |
| `AD5` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad5--the-tick-is-one-fused-system-with-an-all-none-early-out-the-write-is-mutset_if_neq) |
| `AD6` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad6--uivisualdefault-is-the-identity-hand-written-and-const-asserted) |
| `AD7` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad7--the-hit-test-folds-the-same-transform-behind-an-o1-frame-level-guard) |
| `AD8` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad8--flip-is-a-two-system-idiom-not-a-component-graph) |
| `AD9` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition) |
| `AD10` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad10--uivisual-is-a-table-component-the-four-tween-are-dense) |
| `AD11` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad11--uivisuals-partialeq-is-bytewise-and-hand-written-never-derived) |
| `AD12` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad12--the-fused-tick-composes-from-sink-not-from-the-identity) |
| `AD13` | [DECISIONS](UI-PLAN-ANIMATION-DECISIONS.md#ad13--a-dense-member-of-ui_pack_inputs-is-a-build-error-not-a-comment) |
| `A0` | [A0](UI-PLAN-ANIMATION-A0.md#a0--the-ui-clock-and-the-one-consumer-that-already-exists--size-s--m--no-cross-plan-dependency) |
| `A1` | [A1](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) |
| `A2` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a2--easing--size-m--depends-on-a1) |
| `A3` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a3--interaction-transitions--size-m--depends-on-a1-a2) |
| `A4` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a4--the-pack-fold--size-m--depends-on-a1-depends-on-the-sprites-plans-seam-rung-d31-gather--d6-gate) |
| `A5` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a5--inheritance-on-the-gathers-dfs--size-m--depends-on-a4) |
| `A6` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a6--the-hit-test-fold--size-s--depends-on-a5-coordinates-with-the-interaction-plan) |
| `A7` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a7--flip-and-the-tier-3-measurement--size-m--depends-on-a1-a4) |
| `A8` | [A2-A8](UI-PLAN-ANIMATION-A2-A8.md#a8--the-measurement-rung-what-the-tick-actually-costs--size-s--depends-on-a1a5) |

⚠️ **Sub-item locators like `AD9 (3)` stop at the section.** The numbered items inside an amendment or
decision are list items, not headings, so nothing finer than the section has an anchor. `A1` is a
**PARTIAL** exception, and the nameable list is short: of its eleven landing parts, **five are headings
— parts 6, 7, 8, 9 and 11**; **parts 1–5 and 10 are not**, so the **104** times A1's own prose names
one of those six resolve no finer than the rung anchor. Two sub-sections are headings: **`U4`** and
**`P6`**. **`Cluster A` / `B` / `C` are bold paragraph leads, not headings** — a citation of one lands
on the `U4` heading they sit inside, which is what `docs/OPEN-QUESTIONS.md`'s "Cluster B" link already
does. Derived 2026-08-28 with `grep -nE '^#{1,6} ' docs/UI-PLAN-ANIMATION-A1.md`; the 104 with
`tr '\n' ' ' < docs/UI-PLAN-ANIMATION-A1.md | grep -ohiE '\bparts? +(1[0-2]|[1-9])\b' | grep -icE ' (1|2|3|4|5|10)$'`.
⚠️ **The `tr` is what makes the command agree with the number.** Drop it — match line by line, with a
single literal space — and the same pipeline prints **102**, because A1 is hard-wrapped and `part 10`
straddles a line break twice (`UI-PLAN-ANIMATION-A1.md:397-398` and `:1215-1216`), which no per-line
matcher can see. 104 is the count of mentions; 102 is the count the file's wrapping lets a line-based
grep reach, and the gap is an artefact of the wrap, not of the tree.

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
pre-build audit — [`UI-PLAN-SPRITES-DECISIONS.md` **S-D16**](UI-PLAN-SPRITES-DECISIONS.md#s-d16--the-flipbook-writes-uispritesheetindex-and-it-must-a-dense-changedc-inside-or-is-measurably-dead) **(3)** and its [§6 exposure row](UI-PLAN-SPRITES.md#6--what-this-plan-exposes-to-its-siblings))*. The landed
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
| **D7 — the `.ui` registration table** | ~~`docs/UI-PLAN-SPRITES.md` (rung 1)~~ **NOBODY** — or wherever it is sequenced | **A3** (only for authoring `UiStateTint` in `.ui`) | A3 lands with `UiStateTint` inserted from Rust only; the `.ui` spelling follows. *(2026-08-26, [`UI-PLAN-SPRITES-DECISIONS.md` S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) (7) — **D7 has no owning document.** This row, `UI-PLAN-SPRITES.md` §0, `UI-PLAN-ANIMATION.md:846` and `UI-PLAN-INTERACTION.md:501` each name a different owner or none, and no rung in any ladder lands it. ⚠️ **2026-08-28, part 7: `UI-PLAN-ANIMATION.md:846` no longer resolves.** Measured — `grep -n 'D7' docs/UI-PLAN-ANIMATION.md` returns **this row and nothing else** that names a D7 owner, and `:846` is a blank line under `## 3 · The rung ladder`. The fourth naming site the sentence counts is gone, so the count *"each name a different owner"* is over THREE sites, not four. Left as written rather than silently re-pointed, because a dangling coordinate and a re-aimed one are different facts and only the first says the referent was deleted. SCOPE call filed in `docs/OPEN-QUESTIONS.md`.)* [`UI-PLAN-SPRITES.md` §0](UI-PLAN-SPRITES.md#0--what-this-plan-owns-and-what-it-does-not) explicitly **Rejected** owning D7, so this cell pointed at a refusal. |
| **`collect_candidates`' post-D16/D17 shape** | `docs/UI-PLAN-INTERACTION.md` | **A6** | Merge conflict, not a design gap — see A6's coordination note |
| **The §10.8 probe counter** | `docs/UI-PLAN-INTERACTION.md` (D23) | **M3** | M3 falls back to a wall-clock A/B, which is weaker |

**What this plan exposes, that others depend on:**

| Exposed | Consumed by |
|---|---|
| `UiClock` — ~~the one UI delta source~~ **the one UI FRAME-DELTA source**, clamped, real/virtual (AD1) | Sprites plan (the `UiSpriteCursor` flipbook — **migrated by A0b, reading `dt_virtual`**); interaction plan (`ScrollMomentum`, `HoverDwell` — **neither exists in the tree; `grep -rn` returns nothing for either**, so this cell names planned consumers, not current ones). **Fallback, recorded because this row previously implied there was none:** the sprites plan landed first, so `ui_sprite_flipbook` takes `Res<Time>` and applies AD1's clamp itself at that one site as `UI_FALLBACK_MAX_DELTA = 0.1`; ~~the replacement swaps one `SystemParam` and deletes one `min`. It does **not** adopt the raw `real_delta()` AD1 rejects.~~ **The replacement is A0b, and the field is `dt_virtual` (AD9 (1)) — named here 2026-08-26 because neither document named it, and "not the RAW `real_delta()`" left CLAMPED `dt_real` — the documented default — squarely permitted. `dt_real` reds two legs of the shipped `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`: a paused game animates and `set_relative_speed` stops working. `dt_virtual` reproduces `ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*'s arithmetic exactly, so the promise "one `SystemParam` swapped, one `min` deleted" is true only of that field.** *([`UI-PLAN-SPRITES-DECISIONS.md` **S-D17**](UI-PLAN-SPRITES-DECISIONS.md#s-d17--s5-carries-am6s-clamp-itself-and-the-seam-is-a-resource-read-not-a-world-call), 2026-08-21 — the sprites plan had named the rejected option as its fallback, so the dependency read as satisfied here and as waived there; the 2026-08-26 landing correction that chose the VIRTUAL delta was applied to the sprites plan only, and the two owner-facing documents then diverged on the axis that decides whether a paused game keeps animating.)* **`UiClock` is not the crate's only clock**: `reload/system.rs:55` throttles the hot-reload file poll on a `std::time::Instant`, a wall clock consuming no frame delta and deliberately outside this rule. |
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
