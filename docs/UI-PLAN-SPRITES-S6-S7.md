> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §4 rungs S6 (and S6 · LANDED) and S7. Ladder gate: see the index.

### S6 — the `.ui` authoring landing for the sprite vocabulary — **size S**

~~*Behind **D7**, which is owned by [`UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md). The only rung here that
is.*~~ **STRUCK 2026-08-26 at the S6 pre-build audit — S-D20 (7): D7 has NO owning document.**
`UI-PLAN-AETHER.md:73` files D7 as its own **inbound** dependency and states *"D7 does not gate any
rung here"*; its ladder U0–U8 lands no registration table. [`UI-PLAN-ANIMATION.md` §8](UI-PLAN-ANIMATION.md#8--dependencies-stated-explicitly) *(pre-split `:3105`)* names
**this** file as the owner "(rung 1)" — the option §0 explicitly **Rejected**. *(Coordinate read at
its target 2026-08-28 by the tenth pass; it read `:846`, which no longer resolves, then `:2530`,
read on the part-10 tree as prose about an unrelated axis. **Part 11: that description had itself
rotted** — on the tree part 11 inherited `:2530` was a different sentence, with part 10's quote 200
lines below — and the "four sites" count is eight.)*
`UI-PLAN-INTERACTION.md:518-521` names no owner and declines to block *(re-aimed 2026-08-28 by the
tenth pass; `:501-504` is that plan's authorable-component table, which names no owner because it is
not about ownership)*. D7 exists only as
`UI-ADVANCED-ARCHITECTURE.md` §11 sequencing item 1, an architecture ladder no plan file claims.
**S6 is the only rung in the campaign behind it, and nobody is building it.** The fallback below is
therefore not a contingency — it is the path. Filed for the owner in `docs/OPEN-QUESTIONS.md`.

**Lands.** `UiNineSlice`, `UiSpriteSheet` and `UiSpriteAnim` join the `.ui` vocabulary table (three
authored components, ~~five landings each under today's hand-written path~~ **NINE landings each,
counted site-by-site against `UiSpacing` — S-D20 (6)**, one registration each under D7 — *and see
(6): "one registration" does not cover the `LiveNode` field, which a derive in a third module cannot
emit into a fixed struct*). ~~`ImageBundle` gains the optional members.~~ **STRUCK 2026-08-26 —
S-D20 (8): a bundle field cannot be optional on this kernel.** `crates/boyko_macros/src/bundle.rs`
contains **zero** occurrences of `Option` — `expand` collects every named field unconditionally and
emits one `T::component_id()` per field (`bundle.rs:46-80`), which `bundles.rs:50-53` restates in
prose. The buildable form is a SEPARATE bundle, and S5 already landed it:
`AnimatedSpriteBundle` (`bundles.rs:157-176`). `UiSpriteCursor` **deliberately does not opt in** — a
`.ui` file must not be able to ~~inject a running cursor into a live world~~ **NAME a runtime-state
component, or give one a value** *(narrowed 2026-08-26 — S-D20 (2): under the hook ruling a cursor
DOES appear beside an authored animation, at its `Default`, on every authoring path alike. The
property that survives is the one `parse_and_insert` can actually enforce — a name outside the
table — and it is the property G6-3 measures. The wider sentence and the narrower one are
indistinguishable to G6-3, which is why the narrowing is written rather than assumed)*, which is the
same structural-safety property `parse_and_insert` already claims for its closed `match`
(`text/dispatch.rs:5-8`) *(and D7a records it as an **extension** of that claim —
`UI-ADVANCED-ARCHITECTURE.md:397`, "Extending it:" — not as the claim itself)*.

⚠️ **The exclusion is right and it is INCOMPLETE as written** *(added 2026-08-21 at the S5 pre-build
audit)*. `ui_sprite_flipbook` matches `(UiSpriteAnim, UiSpriteCursor, UiSpriteSheet)` — all three —
and no rung gives an authored `UiSpriteAnim` a cursor. A `.ui` file spelling `flipbook:` therefore
produces a node the flipbook system never matches: it renders `index` forever and never ticks, with
no diagnostic. ~~**S5 lands `#[require(UiSpriteCursor)]` on `UiSpriteAnim`** — the kernel already has
the mechanism (`component_registry/required.rs`, the `#[require]` derive attribute), it is exactly
the "capability = presence, state supplied structurally" shape this campaign uses elsewhere, and it
keeps the cursor un-authorable while making it un-missable.~~ **STRUCK 2026-08-26 at the S5 landing
— S-D19 (5): `#[require]` whose target is a DENSE component PANICS at insert on this kernel** (the
require pass resolves the required id's `ComponentPool` in the target ARCHETYPE, and a dense id owns
none by construction — dense plan D0). Three S5 gates failed that way before the attribute came off,
and the panic names an expansion that never happened. Filed in `docs/OPEN-QUESTIONS.md`.

**What S5 landed instead: `AnimatedSpriteBundle`** (`boyko_ui/src/bundles.rs`) — the layout base,
the image, the sheet, the animation and the cursor in ONE spawn, so the pairing is structural at the
AUTHORING site rather than at the component. That is the buildable form of the same guarantee for a
hand-spawned node, and **G5-12** pins both halves (the bundle animates; a hand-spawned
`UiSpriteAnim` with no cursor is FROZEN, silently).

**It does NOT reach a `.ui` file, and S6 still owes the hole a fix.** ~~A parsed `flipbook:` goes
through `parse_and_insert`'s closed `match`, not through a Rust bundle literal~~ **STRUCK 2026-08-26
— S-D20 (8): `flipbook:` is not a `.ui` spelling at all.** It is Aether U5's PROP name
(`UI-PLAN-AETHER.md:70`, `:590`), and the Aether construct *"emits Rust, never `.ui` text"*
(`AETHER:73`) — so the `flipbook:` route is the one that CAN emit `AnimatedSpriteBundle`, and it is
the route that does **not** have this hole. `.ui` keys components on the Rust TYPE NAME
(`dispatch.rs:5-8`), so the authored spelling of this node is `UiSpriteAnim { first: …, last: …,
fps: … }`, and a literal `flipbook:` at component position falls to `other =>` as *"unknown
component"* (`dispatch.rs:220-222`). **The hole is real; it was described in the one vocabulary where
it does not apply** — and a gate written to the struck sentence would construct a node with no
`UiSpriteAnim` on it and assert `index` moved, failing for the wrong reason or, written loosely,
going vacuous.

**The ruling — neither (a) nor (b). `UiSpriteAnim` takes an `on_add` HOOK.** Recorded in full as
**S-D20 (1)**, with the probe that proved it: a TABLE component's `on_add` hook deferred-inserts the
DENSE `UiSpriteCursor` through a one-field `#[derive(Bundle)]` wrapper, and `has_component` reports
it present after the apply, with `dir: 1`. ONE landing, at the component, inherited by **every**
construction site — the `.ui` dispatch, the reconcile's insert branch, `ui!`, a hand-spawn,
`AnimatedSpriteBundle`, and U5's future `flipbook:` prop. ~~(a) have the dispatch insert a
`UiSpriteCursor` beside every parsed `UiSpriteAnim`~~ is **REFUSED**: it is three sites, not one (the
reconcile insert branch and the reconcile remove branch are the other two), and it falsifies the
`.ui` ≡ `ui!` ≡ hand-spawn invariant where no comparator can see it. ~~(b) wait for the kernel
defect to close and restore the `#[require]`~~ is **REFUSED**: "closes" is undefined — of the three
options filed in `docs/OPEN-QUESTIONS.md` only the first restores the attribute, the second leaves
*"the capability missing rather than fixed"*, the third is what S5 did, and the entry still reads
*"**What it blocks:** nothing today"*. **S6 therefore does NOT ship with this hole open.**

**No gate in either rung could see this**: G5-2 and G5-5 insert the components by hand, and
G6-1/G6-2/G6-3 test round-trip, reload and the exclusion diagnostic — none drives a tick on a PARSED
node. ~~**G6-4** stays as specified~~ **STRUCK 2026-08-26 — S-D20 (9): G6-4 had NO specification to
stay as.** `grep -rn "G6-4"` over the whole tree returned exactly this one line; the Gate table below
carried three rows and not four; and neither red mutation named it — so a builder working the rung
from its own table lands S6 green with the hole untouched, which is the failure this paragraph was
written to prevent. G6-4 is specified in the table below, and it is written so it **REDS TODAY**:
its first assertion is that the LOWERING report is clean, which fails right now because
`UiSpriteAnim` is not yet in the vocabulary; after the vocabulary lands and before the hook does, it
reds at the cursor-presence assertion instead. Each red names which half is missing.

**Gate.** *(rewritten 2026-08-26 at the S6 pre-build audit — S-D20 (2)–(5), (9), (10). Two of the
three original rows named an observable that does not depend on what the row exists to prove; the
fourth row had no specification at all; and one gate had to be added because the corpus it extends
was MEASURED blind. Every row below now names the value that moves when the thing is missing.)*

| # | Claim | How — and what the observable is |
|---|---|---|
| **G6-1** | Round trip, for the three new components | A CANONICAL `.ui` source (already in `serialize_ui`'s exact output form) carrying `UiLayout` + the three sprite components and **no `UiImage`** → parse → spawn → `serialize_ui` → **byte-identical to the INPUT**, not merely a fixed point. *(added 2026-08-26 — S-D20 (4). The existing corpus's `assert_serialize_fixed_point` (`p3_round_trip.rs:66-78`) compares `s1` to `s2`, and a component the serializer DROPS is dropped from both — so a fixed point cannot see a missing `serialize.rs` arm. MEASURED: `UiImage` parses, inserts, and is written by nothing; the corpus is green over it. The `UiImage` exclusion is a recorded departure, not a convenience — a realistic sprite node carries `UiImage`, and that node cannot round-trip today for a reason S6 does not create and does not fix.)* |
| **G6-2** | Hot reload carries an EDIT and a DELETION through, per component | Edit `UiNineSlice.border_px` in the file, reload, assert the LIVE value MOVED to the new one; then delete the component from the file, reload, assert it is ABSENT. **Landed as TWO tests, not one — S-D21 (3): applying M6-b showed the edit leg failing FIRST and hiding the delete leg, which is `--no-fail-fast`'s shadowing one level down. A third test was added for the animation leg (an edited `fps` reaches the world and the running cursor is NOT reset, because `on_add` does not re-fire on a re-insert).** *(re-pointed 2026-08-26 — S-D20 (5). The row said "hot reload **preserves** them", which is exactly what happens with **no reconcile arm at all**: `patch_node`'s own doc is "*Writes ONLY the closed text-owned set; transient components + `UiSourceOrder` are **preserved by omission***" (`reload/reconcile.rs:429-437`), and `patch_unit_struct`'s remove branch `(None, Some(_))` is reachable only for a component the patcher already tracks. The gate named the one outcome its own mutation cannot disturb.)* |
| **G6-3** | Runtime state is not NAMEABLE from text | A `.ui` file naming `UiSpriteCursor` produces an "unknown component" `UiParseReport` diagnostic at the right line and column — asserted on the **LOWERING** report, never the parse report. **The COLUMN is `body_col` (the first byte inside the component's `{`), not the name's own column — S-D21 (4): that is what `parse_and_insert` has in hand, so the diagnostic locates the component on the line but does not point at the offending NAME. Pinned at the measured value with the gap written down, rather than at the value a reader would guess.** *(added 2026-08-26 — S-D20 (3). `parse_ui` does not know component types; `parse_and_insert` does, and it runs inside `spawn_ui_tree`. The shared harness `spawn_dot_ui` asserts `tree.report.is_clean()` (the PARSE report) and then hands the lowering a `owned.report.clone()` that is dropped — so a lowering diagnostic is unobservable through it. A gate written against the parse report proves nothing about the closed match.)* |
| **G6-4** | An AUTHORED animation TICKS | *(specified 2026-08-26 — S-D20 (9); the rung's whole reason for existing, and it had no row.)* A `.ui` file spelling `UiSpriteAnim { … }` and `UiSpriteSheet { … }` on one node; ~~a registered `UiSheetTable`~~ **STRUCK at the build — S-D21 (5): `ui_sprite_flipbook` takes `Res<Time>` and a `Query` over the three components and never reads the table; the table is the RENDER gather's input. Inserting one would have been a dead datum dressed as a precondition, which is this campaign's own recorded defect class**; `ui_sprite_flipbook` in a real `Schedule` ~~before `ui_render_discovery`~~ **(ALONE — the order relative to discovery is S5's gate, and discovery lives in `boyko_render` while this rung's tests live in `boyko_ui`)**; `Time::advance_with` driven N frames — S5's own harness shape (`ui_s5_sprite_sheet.rs:140-190`). **Four assertions, in this order, so each red names its own half:** (1) the LOWERING report is clean — reds TODAY, because `UiSpriteAnim` is not in the vocabulary yet; (2) `UiSpriteAnim` and `UiSpriteSheet` are PRESENT on the spawned entity; (3) **`UiSpriteCursor` is PRESENT** — the hook's own observable, and the assertion the whole ruling turns on; (4) `UiSpriteSheet.index` MOVED. |
| **G6-5** | The three components are in **both** equivalence comparators | *(added 2026-08-26 — S-D20 (3).)* One row in `p3_common::presence_vector` **and** one in `p6a_equivalence`'s local `pres!` / `valeq!` lists. They are two independent hand lists (10 rows, and 10 + 5), and a component in one is not in the other. **MEASURED why this is a gate and not bookkeeping:** `button_widget_three_ways_equivalent` is GREEN today over a real divergence — its `.ui` arm authors `UiBackground { color: 0 }`, `UiBackground` has **no dispatch arm anywhere** (`grep -rn UiBackground crates/boyko_ui/src/text/ crates/boyko_ui/src/reload/` → nothing), the probe reports `UiLayout present = true, UiBackground present = false` on the `.ui` node while the `ui!` arm inserts it, and the test passes because neither comparator lists the name and the lowering report is a dropped clone. **Add the three arms, forget the two rows, and the gate reports green over a `.ui`-vs-`ui!` divergence.** |

**Red mutations.**

* **M6-a — add `UiSpriteCursor` to the vocabulary table.** G6-3 reds. *Proves the exclusion is
  enforced rather than documented — D7a calls this a safety property, and a safety property with no
  test is a sentence.* *(scope corrected 2026-08-26 — S-D20 (2): it proves the NAME half only. The
  narrowed property is what M6-a can red on; the wider sentence it used to be attached to is not
  reddened by any mutation, which is why the sentence was narrowed instead of kept.)*
* **M6-b — omit the ~~`TextStruct` impl~~ RECONCILE ARM for `UiNineSlice`** — the `TextStruct` impl,
  the `LiveNode` field, the snapshot read and the `patch_unit_struct::<UiNineSlice>` line **together**,
  because the `C: TextStruct` bound (`reconcile.rs:554-563`) makes them one unit.
  ~~G6-2 reds: the component silently disappears on reload.~~ **STRUCK 2026-08-26 — S-D20 (5): the
  red could not fire, in two independent ways.** (a) Omitting only the impl is
  `error[E0277]` at the call site — a compile error, and the protocol requires the predicted failure
  OBSERVED, not a build that never runs. (b) Omitting the arm does not make the component vanish; it
  makes it **STALE** — an edit to the file has no effect and a deletion does not remove. **G6-2 now
  reds on both of its legs**: the EDIT leg (the live value does not move) and the DELETE leg (the
  component survives). *This is the exact silent failure D7 exists to remove; seeing it red once is
  what makes the registration table's value concrete.*
* **M6-c — delete `#[component(on_add = …)]` from `UiSpriteAnim`.** *(new 2026-08-26 — S-D20 (1).)*
  G6-4 reds **twice**: at assertion (3), the cursor is absent, and at assertion (4), `index` never
  moves. *The ruling's own red. Without it the hook is a sentence, and the campaign has already
  recorded what a rung written around an unbuilt mechanism costs.*
* **M6-d — add the three components to `p3_common::presence_vector` but NOT to `p6a_equivalence`'s
  local lists.** *(new 2026-08-26 — S-D20 (3).)* G6-5 reds. *Two hand lists is one list too many to
  keep in a head; the MEASURED `UiBackground` divergence is what a forgotten row looks like when
  nothing checks.* **How G6-5 is made able to fail at all — S-D21 (2): a comparator ROW is not
  self-gating, because a row nothing exercises is indistinguishable from a missing row. G6-5
  therefore injects a divergence per list (a presence divergence and a value divergence) and asserts
  the comparator PANICS on it, through `catch_unwind` with the panic hook silenced. That is the
  "prove the gate live by injecting an error" discipline, moved inside the test.**
* **M6-e — revert `split_top_level`'s bracket-depth fix** (`b'(' | b'['` back to `b'('`).
  *(new 2026-08-26 at the build — S-D21 (6).)* G6-1 reds on both tests and G6-2's edit leg reds,
  with the mis-split visible in the diagnostics (`invalid value for field 'border_px'` followed by
  three `expected 'key: value'`). *The fix is a PRE-EXISTING bug S6 could not build around: the
  `[f32; 4]` leaves cannot parse without it, and neither could `UiImage`'s `[f32; 2]` UVs, which had
  been silently defaulting since GUI P6a.*

**Fallback if D7 slips (R4's residual risk) — and per S-D20 (7) it has not slipped, it is
unowned, so this IS the path.** The three components take hand-written arms — ~~a `.ui`
dispatch arm, a field parser, a `serialize.rs` arm, a `reload/reconcile.rs` `TextStruct` impl, and an
equivalence-gate row, each~~ **NINE each, S-D20 (6)**: a dispatch arm, a private field parser, a
`parse_<comp>_public` wrapper, a `write_<comp>` formatter, its emit block in `write_node`, a
`LiveNode` field, its snapshot read in `UiTreeView::build`, a `TextStruct` impl, and the
`patch_unit_struct::<C>` line — **plus two comparator rows** (G6-5) and, for the sprite trio, **four
new leaf value parsers** the dispatch does not have: `[f32; 4]`, `u16`, `NineSliceMode` and
`SpriteAnimMode`. S6 lands anyway, ~~fifteen~~ **about thirty** landings heavier and no worse than
the status quo. **The sprite ladder is never blocked behind D7.** S0–S5 do not touch the `.ui`
surface at all.

**Where G6-4's green STOPS, stated because it is not where a reader would assume** *(added 2026-08-26
— S-D20 (10))*. `UiPlugin::build` (`plugin.rs:85-115`) registers exactly ONE system
(`ui_hot_reload_system`, itself gated on a configured watch path) and inserts exactly ONE resource
(`UiHotReload`). It never adds `ui_sprite_flipbook` and never inserts a `UiSheetTable`. Every
registration of either in the tree is a TEST harness building its own `ScheduleBuilder`
(`ui_s5_sprite_sheet.rs:144`, `:188`, `:609`; `ui_flipbook_gpu_golden.rs:216`, `:262`). So a green
G6-4 proves the authored node ticks **in a schedule that has the flipbook**, and a `.ui`-authored
flipbook still animates nothing in any real application — because the SYSTEM and the TABLE are
missing from the production path too, not only the cursor. That follows from D32's deferral (§6) and
is not S6's regression; it is stated here so nobody reads G6-4's green as "sprites animate in the
app".

---

### S6 · LANDED 2026-08-26 — the landed set, the RED ledger, the goldens, and what the build found

*(First build, after the pre-build audit's S-D20 amendments. Every gate below was run with its exit
code seen UNPIPED and `running N` confirmed in BOTH profiles with the test NAMES compared, not the
counts; every red was APPLIED and its failure OBSERVED; every mutated source was restored and
verified byte-identical with `cmp` plus a SHA-256 diff against a pre-mutation snapshot. Six
corrections the build found are ruled in **S-D21**. Landed on the FALLBACK path, because S-D20 (7)
found D7 unowned — so the three components take hand-written landings, and the rung is not blocked
behind anything.)*

#### The landed set, file by file

| File | What landed |
|---|---|
| `crates/boyko_ui/src/sprite.rs` | **`ui_sprite_anim_on_add`** — the ruling. A `HookFn` that deferred-inserts `UiSpriteCursor::default()` through `SpriteCursorBundle`, a private one-field `#[derive(Bundle)]` wrapper (dense storage suppresses the single-component `Bundle` impl, so the bare type is `E0277` — the only spelling that compiles). Its doc carries why it is `on_add` and not `on_insert`, why the DEFERRED insert is stated rather than assumed, and why there is deliberately no `on_remove`. |
| `crates/boyko_ui/src/components.rs` | `#[component(on_add = crate::sprite::ui_sprite_anim_on_add)]` on `UiSpriteAnim` — **one landing, at the component, inherited by every construction site**. `UiSpriteAnim`'s "the cursor is NOT `#[require]`d — spawn the bundle" section rewritten to "the cursor arrives on its own"; `UiSpriteCursor::default`'s doc re-pointed at the hook (the `#[require(A => B)]` spelling it used to show has never existed). |
| `crates/boyko_ui/src/bundles.rs` | `AnimatedSpriteBundle`'s doc: it is now ERGONOMICS (one spawn into one archetype), not the requirement. Records that the hook replaces its own `cursor` field with an equal `Default` at the drain, and when that would stop being inert. |
| `crates/boyko_ui/src/text/split.rs` | **`split_top_level` is bracket-aware** — S-D21 (6), a pre-existing bug S6 could not build around. The "COPIED VERBATIM" header becomes "copied, then DIVERGED", with the measurement that bracketed `.ui` values had never parsed. |
| `crates/boyko_ui/src/text/dispatch.rs` | Three match arms (`UiNineSlice`, `UiSpriteSheet`, `UiSpriteAnim`) with the exclusion written at the site; three private field parsers; three `parse_*_public` wrappers for the reconcile; **four new leaf parsers** — `parse_u16`, `parse_f32_quad`, `parse_nine_slice_mode`, `parse_sprite_anim_mode` (S-D20 (6) counted these, and D7a's "the bodies already exist" is true of the fifteen and false of these four). |
| `crates/boyko_ui/src/text/serialize.rs` | `write_ui_nine_slice` / `write_ui_sprite_sheet` / `write_ui_sprite_anim`, their three emit blocks in `write_node` (appended AFTER the P1/P3 set, so a document carrying none of the three serializes byte-identically to what it did before), `write_f32_quad`, and the two mode-name tables. |
| `crates/boyko_ui/src/reload/tree_view.rs` | Three `LiveNode` fields + three snapshot reads. **The file S6's own Lands list never named** (S-D20 (6)): without these, the serializer arms and the `TextStruct` impls are unreachable code. |
| `crates/boyko_ui/src/reload/reconcile.rs` | Three `TextStruct` impls + three `patch_unit_struct::<C>` lines. `UiSpriteAnim::remove`'s doc carries the orphaned-cursor accounting and why the symmetric hook cannot exist. |
| `crates/boyko_ui/tests/ui_s6_authoring.rs` | **NEW** — G6-1 (two tests: the round trip against the AUTHORED bytes, and the negative that the materialized cursor never reaches the text), G6-2 (three: edit, delete, and the animation edit that must NOT reset a running cursor), G6-3, G6-4. Seven tests, device-free. |
| `crates/boyko_ui/tests/p6a_equivalence.rs` | Three `pres!` + three `valeq!` rows, and **G6-5** — the three-way equivalence plus four injected divergences asserted to be DETECTED (S-D21 (2)). |
| `crates/boyko_ui/tests/p3_common/mod.rs` | Three `presence_vector` rows + three `assert_same_values` rows — the OTHER hand list. |
| `crates/boyko_ui/tests/p3_dispatch.rs` | `DISPATCHABLE` `9 → 12` with the three new names, their minimal bodies and their presence arms. Its doc now says the list is a SUBSET and names what it does not walk, instead of implying a completeness it never had (the match has 22 arms). |
| `crates/boyko_render/tests/ui_s5_sprite_sheet.rs` | **G5-12 RE-POINTED.** Its leg 2 pinned the hazard S6 exists to close (*"a cursorless animation is FROZEN, silently"*) and was OBSERVED red at this landing — `index 3` where it demanded `0`. It now asserts the opposite and still reds under M6-c. `g5_4`'s `#[require]` comment and the module header's stale `running 10` (actual 12) corrected in passing. |

#### The RED ledger — five mutations, every failure OBSERVED

| Mutation | Predicted | OBSERVED |
|---|---|---|
| **M6-a** — add a `"UiSpriteCursor"` arm to the closed match | G6-3 reds | `g6_3_naming_the_cursor_from_text_is_an_unknown_component` FAILED at `assert!(!report.is_clean())`, panicking *"UiSpriteCursor must not be dispatchable"*. ~~5 passed / 1 failed.~~ **6 passed / 1 failed** — corrected at the S6 verification, which re-applied the mutation and found the recorded count arithmetically impossible against a 7-test binary. The row was written before G6-2's third test existed, i.e. **not against the landed source**. The reason it names is exact; the count was not re-taken after the gate grew. |
| **M6-b** — delete `patch_unit_struct::<UiNineSlice>` **and** `impl TextStruct for UiNineSlice` | G6-2 reds on BOTH legs | Both FAILED. The edit leg printed `left: Some([8.0; 4])` vs `right: Some([12.0; 4])` — **STALE, not absent**, exactly the outcome the struck row could not have seen. 5 passed / 2 failed. |
| **M6-c** — delete `#[component(on_add = …)]` from `UiSpriteAnim` | G6-4 reds twice | THREE tests FAILED in `boyko-ui`: `g6_4` at assertion (3) with `left: None` vs `right: Some(UiSpriteCursor { elapsed: 0.0, dir: 1, … })`, plus both other cursor observables. ~~Assertion (4)'s red was observed SEPARATELY, in `boyko-render`'s re-pointed G5-12, which fails on the frozen index.~~ **NOT OBSERVED — corrected at the S6 verification.** Under M6-c, G5-12 panics three lines EARLIER, at the `dense_contains(node, UiSpriteCursor::component_id())` assertion **S6 itself added**, so it never reaches the index check. Assertion (4) is shadowed in `boyko-ui` by assertion (3) and shadowed again in `boyko-render` by that presence assert: **no mutation in this ledger reaches it.** A live route does exist and was measured — neutering `ui_sprite_flipbook`'s `dt` guard reds `g6_4` **alone**, `left: Some(0)` vs `right: Some(3)`. The property is genuinely gated; the evidence this row claimed for it was not. 4 passed / 3 failed, and 11 passed / 1 failed. |
| **M6-d** — the three rows in `presence_vector` but NOT in the local `pres!`/`valeq!` | G6-5 reds | FAILED at the first local-list control: *"p6a_equivalence pres! row (UiNineSlice): the comparator did NOT report this divergence"*. 5 passed / 1 failed. |
| **M6-e** — revert the `split_top_level` bracket fix | G6-1 reds | Both G6-1 tests and G6-2's edit leg FAILED, with the mis-split visible: `(4, 29, "invalid value for field 'border_px'")` followed by three `expected 'key: value'`. 4 passed / 3 failed. |

**Restoration.** SHA-256 before and after, for all six mutated files
(`dispatch.rs`, `split.rs`, `components.rs`, `reconcile.rs`, `tree_view.rs`, `p6a_equivalence.rs`):
identical, verified by `cmp` per file and one `diff` over the two hash manifests.

#### The goldens — ten pins, none moved

All eight UI golden binaries run on the RTX 3060 with `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` armed (so a
device skip is a FAILURE, not a silent pass) and `--test-threads=1`, exit 0 each, no `SKIP` line:
`ui_rect_gpu_golden` (1), `ui_sprite_gpu_golden` (2), `ui_nine_slice_gpu_golden` (1),
`ui_nine_slice_tiled_gpu_golden` (2), `ui_flipbook_gpu_golden` (2), `ui_text_gpu_golden` (1),
`ui_text_multiscale_gpu_golden` (1), `ui_rect_swapchain_golden` (1). **S6 is an authoring rung and
moved no pixel**, which is what an authoring rung owes.

⚠️ **Recorded because it nearly produced a vacuous green:** these tests are NOT `#[ignore]`d. A first
attempt ran them with `-- --ignored`, which FILTERED THEM OUT and printed `running 0 tests`, exit 0,
for three binaries in a row. `running 0 tests` is a vacuous pass, and the flag that produced it is
the flag the ignored-suite protocol uses everywhere else in this repo.

#### Both profiles, by NAME

`ui_s6_authoring` — 7/7 in debug and release, same seven names. `p6a_equivalence` — 6/6, same six.
`p3_dispatch` — 5/5, same five. `ui_s4_nine_slice` is the standing counter-example the rule exists
for and still behaves that way: **6 in both, DIFFERENT sets** (`g4_5_…should panic` in debug,
`s_d12_2_a_negative_inset_degenerates_in_release_instead_of_inverting` in release).
`ui_s5_sprite_sheet` — 12/12, same names in both.

#### Regression

`cargo test -p boyko-ui --all-targets --no-fail-fast` exit 0 (40 targets). `cargo test -p
boyko-render --lib --tests --no-fail-fast` exit 0, `--lib` alone 539 passed. The named UI battery
(`ui_s0_discovery`, `ui_s0_seam`, `ui_rect_edsl_sync`, `ui_rect_spv_sync`, `ui_pack_cpu`,
`ui_no_realloc`, `ui_s4_nine_slice`, `ui_s5_sprite_sheet`) exit 0. `cargo clippy -p boyko-ui
--all-targets -- -D warnings` and the same for `boyko-render`, both touch-first and both exit 0
(4.76 s and 21.63 s — not false-fresh, and the gate was proven live by the `expect_fun_call` it
caught on the first run). Root censuses `engine_packages_census`, `goldens_pins_wellformed`,
`internal_docs_anchors`, `gpu_blocking_reader_census`, `vg_symbol_reachability` — exit 0.

#### What S6 did NOT close, restated so the next reader does not assume it did

* **D7 is still unowned** (S-D20 (7)). S6 landed on the fallback; the registration table has no
  builder and no plan file claims it.
* **Ten dispatchable components still have no serializer arm** (S-D20 (4)) — `UiText`, `Button`,
  `Bar`, `BarFill`, `UiImage`, `UiGrid`, `UiAnchor`, `OnClick`, `OnHover`, `OnSubmit`. S6's three are
  landed MORE completely than `UiImage`, the component they modify.
* **`UiBackground` has no dispatch arm at all**, and `button_widget_three_ways_equivalent` is still
  green over that divergence. G6-5 adds the instrument for the sprite trio; it does not retro-fit
  the button.
* **`ui_sprite_flipbook` and `UiSheetTable` are still absent from `UiPlugin::build`**, so a
  `.ui`-authored flipbook animates in a schedule that has the flipbook and nowhere else. That is
  D32's deferral, restated above under "Where G6-4's green STOPS".
* **`internal_docs_anchors` still does not gate this document** (S-D20 (11)) — every `file.rs:NN` in
  this landing block, including the ones just written, rots unnoticed.

---

### S7 — measurement-gated dispositions — **size S, may be dropped entirely**

Three items, each of which ships **only** if a number says so. Recording them here is what makes S3's
and S5's deferrals decisions rather than holes.

1. **Model A, the runtime atlas.** Ships only if §10.1's 64-slot leg regresses materially. The
   migration path is recorded now: `UiSheet.slot` re-points at the atlas texture, `uv` re-bases onto
   the tile, **and no component changes**. If it does not ship, the number is recorded and D2 stands.
2. **An archetype-shaped gather.** Ships only if §10.8's probe cost dominates the gather µs at
   N = 2048. Today's per-node `get_component` probes are the campaign's one unconditional per-node
   per-frame cost, and D23 independently names the same cost class as the likely dominant term in the
   interaction spine — so this is one decision serving two subsystems and should be taken with
   [`UI-PLAN-INTERACTION.md`](UI-PLAN-INTERACTION.md), not before it.
3. **Per-sprite filtering.** A 2-element sampler array at set 0 binding 3 plus `flags` **bit 4**
   (reserved at S-D2, cost zero) — ships only when a UI needs pixel-art and photographic sprites in
   one pass. Until then S-D4's one-mode-at-setup stands.

Also parked here, unchanged from the architecture: **opaque pre-pass / overdraw management** — noted
as the one lever left if fill rate ever dominates; no surveyed engine solves it in the batcher.

---

