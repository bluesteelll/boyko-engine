> **Part of [UI-PLAN-ANIMATION.md](UI-PLAN-ANIMATION.md)** — §3 rung A0. Ladder gate: see the index.

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
> ([`UI-PLAN-ANIMATION.md` §8](UI-PLAN-ANIMATION.md#8--dependencies-stated-explicitly); [`UI-PLAN-SPRITES-DECISIONS.md` S-D17](UI-PLAN-SPRITES-DECISIONS.md#s-d17--s5-carries-am6s-clamp-itself-and-the-seam-is-a-resource-read-not-a-world-call), `:3582`). Leaving it unowned means the
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

