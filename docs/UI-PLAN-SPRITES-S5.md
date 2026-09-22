> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §4 rung S5 and S5 · LANDED. Ladder gate: see the index.

### S5 — sprite sheets and the flipbook — **size M**

*Architecture D8a, D8b, D8c, §4.2.*

**Lands.**

1. **The sheet table** — one `Resource`-owned dense column keyed by a dense `u16 sheet_id`
   (the `FontId` handle discipline; never a `HashMap<name, sheet>`):

   ```rust
   #[repr(C)]
   pub struct UiSheet {
       slot: u32,          // bindless slot
       cols: u16, rows: u16,
       frame_count: u16,   // <= cols*rows; trailing cells may be unused
       _pad: [u8; 2],      // SPELLED — inset_uv needs 4-byte alignment
       inset_uv: [f32; 2], // half-texel inset against bilinear bleed
   }                       // 20 B, no tail pad
   ```

   **Plus the mint verb** *(added 2026-08-21 — S-D16 (3): the struct and the column were landed and
   the REGISTRATION was not, so §6 exposed a "sheet-id mint" no line of this rung created)*:
   `UiSheetTable::register(UiSheet) -> SheetId`, the `FontTable::load` verb
   (`text/font.rs:130-158`) — a setup-time push into a `Vec` inside a `#[derive(Resource)]` struct,
   returning the dense index. Setup-only; the table never grows in-frame.
2. `UiSpriteSheet { sheet: u16, index: u16 }` — 4 B, table. Presence ⇒ the pack derives `uv` from the
   sheet table by **pure arithmetic** from `(cols, rows, index)` instead of reading `UiImage`'s
   `uv_min`/`uv_max`, and takes the slot from `UiSheet` rather than from `UiImage`. **The
   substitution happens in the GATHER, into `UiImageInput`, and `UiImage` remains the capability**
   *(ruled 2026-08-21 — S-D16 (3): `ui_node_sub_codes` is the SOLE authority and its truth table plus
   `pack_ui_sub_record`'s two `.expect` preconditions are keyed on `input.image`; substituting in the
   gather keeps `pack.rs` free of every `boyko_ui` type and keeps `components.rs:520-525`'s
   `border_uv` sentence — "a fraction of the node's CURRENT `UiImage` UV sub-rect" — true, which is
   what makes item 7's "composes for free" a fact rather than a hope. A node with `UiSpriteSheet` and
   no `UiImage` emits its background alone, the S-D12 (3) row for `UiNineSlice` alone.)*. The table is
   read via ~~`WorldView::resource::<UiSheetTable>()` (`dispatcher_token.rs:284`)~~
   **`WorldView::try_resource::<UiSheetTable>()`** — once per gather, not per node, so it is **not**
   a probe and does not move the census. *(corrected 2026-08-26 at the landing — S-D19 (4): `:284`
   is the PANICKING verb, and the gather it sits in runs for every UI scene in the tree, including
   eight harnesses that build worlds by hand and insert no sheet table. An absent table is not an
   error; it means no sheet is registered, and every node then draws its `UiImage` exactly as it did
   at S4.)*
3. ~~`UiSpriteAnim { first: u16, last: u16, fps: f32, mode: u8, repeats: u8 }` — 12 B, table,
   **cold**~~ **`#[repr(C)] #[derive(Clone, Copy)] UiSpriteAnim { first: u16, last: u16, fps: f32,
   mode: SpriteAnimMode, repeats: u8, _pad: [u8; 2] }` — 12 B, align 4, table, cold** *(corrected
   2026-08-21 — S-D16 (2). Two defects in one line. The stated 12 B was reached through IMPLICIT
   tail padding, which `UiNineSlice::_pad`'s own doc calls "precisely what 'the padding is spelled'
   forbids leaving unwritten in a `#[repr(C)]` POD" — and the sibling item one row up spells its own.
   And `mode` is a raw `u8` on an AUTHORED component with four legal values, which is the shape
   S-D13 (4)(3) ruled against one rung earlier: the authored component keeps the typed enum, where
   the type system forbids the fifth value; only a CROSS-CRATE byte is `debug_assert!`ed, and this
   one never crosses — the flipbook and the component both live in `boyko_ui` and the pack never
   reads `mode`. `SpriteAnimMode` is `#[repr(u8)] { Forward, Reverse, PingPong, Once }`; no count
   const and no conversion site are minted. The size is pinned by `const _: () = assert!(size_of…)`,
   MEASURED not asserted — S-D12 (2).)*: author-written, never system-written.
4. ~~`UiSpriteCursor { elapsed: f32, frame: u16, dir: i8 }` — 8 B, **dense**: the only column the
   flipbook system writes per frame.~~ **`#[repr(C)] #[derive(Clone, Copy)] UiSpriteCursor
   { elapsed: f32, dir: i8, loops_done: u8, _pad: [u8; 2] }` — 8 B, align 4, dense: the flipbook's
   PRIVATE state, read by no other system and by no pack input.** *(corrected 2026-08-21 —
   S-D16 (2). `frame` was written by the flipbook and read by nobody — item 2's pack reads
   `UiSpriteSheet.index` — which is this campaign's dead-datum class at `:343`. `loops_done` replaces
   it because `UiSpriteAnim.repeats` had NO reader either: nothing in `{elapsed, frame, dir}` counts
   completed cycles, so `Once` and `repeats` were unimplementable and `repeats` was a second dead
   datum in the same pair. Padding spelled, size const-asserted, same reasons as item 3. The "only
   column written per frame" claim is struck: item 5 writes a table column too, and S-D16 (1)
   measured that it MUST.)*
5. `ui_sprite_flipbook` — one system over `(UiSpriteAnim, UiSpriteCursor, UiSpriteSheet)` advancing
   `elapsed`, flipping `dir` at the ends for `PingPong`, counting `loops_done` against `repeats`, and
   writing `index` **through `Mut<UiSpriteSheet>::set_if_neq`** *(added 2026-08-21 — S-D16 (1), and
   it is the rung's load-bearing verb. `&mut T` does not consult ticks (`write.rs:234`), and this
   tick IS the repaint signal: it is what bumps `UiRenderGeneration` through the discovery filter. It
   cannot instead be `Changed<UiSpriteCursor>`, because a dense `Changed<C>` inside `Or<..>` was
   MEASURED to never fire on this tree — see S-D16 (1)'s table. `set_if_neq` rather than a plain
   deref so a 12 fps flipbook does not bump the generation on the four frames in five where the index
   is unchanged.)*. The clock is `Res<Time>` plus S5's own clamp until `UiClock` lands — S-D17, and
   the field it lands on is `dt_virtual` ([`UI-PLAN-ANIMATION-DECISIONS.md` AD9](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition)), swapped by that plan's rung A0b.
6. ~~`ui_pack_inputs!` gains the three components that affect the picture.~~ **`ui_pack_inputs!`
   gains exactly ONE component — `UiSpriteSheet`.** *(corrected 2026-08-21 — S-D16 (2)(3): only one
   of the three affects the picture. The pack never reads `UiSpriteAnim` (author configuration) and,
   after item 4, never reads `UiSpriteCursor` (flipbook-private). The gather probes every listed
   component on every visited node whether it is present or not, so the other two would have been
   dead probes charged to every node of every changed frame — and `UiSpriteCursor`, being dense,
   would additionally have sat in the `Or` as a term that CANNOT be true. Probe census **7.00 →
   8.00**, not 7.00 → 10.00; `Or` arity **6 → 7** against a ceiling of 12.)* The landing is
   lockstep — the component, the macro list, the arity-locked destructure, the `PackInput`
   construction, `UiImageInput`'s substitution site, `ui_s0_discovery`'s enum + `ALL` + `name()` +
   `mutate_pack_input` arm, and `ui_s0_measure`'s prose ladder row — and **G5-10 drives it**, because
   S4's identical line was gated by G4-7 and this table had no row for it.
7. **`NineSliceMode::Tile`, inherited from S4 by the 2026-08-21 audit ruling (S-D11).** S4 lands the
   `mode` field with one legal value; S5 widens the value set. What S5 owes, and why it is cheap
   *here* and was not cheap at S4: ~~the mechanism is `uv = sub_min + frac(t) * (sub_max - sub_min)`
   on the fragment shader's sprite branch, selected by a new `FLAG_TILED` bit out of the free bits
   5..19 (S-D2), with the tile count folded into `uv` at pack.~~ **the mechanism is
   `uv = sub_min + ui_tile_frac(t * tiles) * (sub_max - sub_min)`, with `FLAG_TILED` at bit 5 and the
   two 7-bit repeat counts at bits 6..=12 / 13..=19, and the counts DERIVED at pack from
   `border_px`/`border_uv`** *(both halves corrected 2026-08-21 — **S-D15**, and this is the rung's
   blocking finding. `t` is the 0..1 quad corner (`ui_rect.vs.hlsl:74`), so `frac(t) == t` for every
   covered fragment and the ruled expression was BIT-IDENTICAL to the `lerp` it said it replaced —
   `Tile` as specified rendered as `Stretch`, and M5-e compared two implementations that compute the
   same pixel. And "the tile count folded into `uv`" is a clause inherited from the mechanism S-D11
   RETIRED: `uv` is four floats all consumed as `sub_min`/`sub_max`, and pushing a count into them
   makes `frac` sweep N whole frames — reproducing, in the replacement, exactly the sheet bleed S-D7
   existed to guard, on the gate that forbids it. The count needs its own carrier and the free bits
   are it; the derivation needs no texture dimensions because the source extent cancels out of the
   border ratio.)*. That is **the same sub-rect arithmetic items 1–2 above already build** for sheet
   frames — and now literally so, since the cancellation makes the count identical under a frame and
   under a whole texture — so it is one shader edit at S5 (**one NEW eDSL leaf `ui_tile_uv` plus a
   new `frac` primitive the eDSL does not have, re-emit, re-DXC, ONE `SpirvBlob<N>` length, ZERO new
   manifest rows**) *(corrected 2026-08-21 — S-D15 (4): there is no `frac`/`floor`/`fract` anywhere
   in `boyko_shaderdsl/src/`, none of `ui.rs`'s six leaves touches the sprite `uv` (the line is in
   `main`, below the last sentinel), `ui_rect.vs.hlsl` has no sprite branch or flag span so only the
   FS blob moves — the manifest's own history records the VS byte-identical across the whole S3
   sprite landing — and `FLAG_TILED` is a runtime bit, not a `-D` variant, on two sources the
   manifest records as having NO `-D` axis. The existing two rows gain notes; the landing-history
   table gains an S5 row.)* instead of two at S4 and S5 — and S4 keeps
   the "pure CPU rung, no shader change" property that is D8d's whole argument for CPU expansion over
   Bevy's separate pipeline. **`frac` inside the sub-rect wraps to the sub-rect, which IS a sheet
   frame**, so `Tile` + `UiSpriteSheet` is the correct picture rather than S-D7's hard error — the
   guard and the clamp S4 was going to build are not built by anyone. ~~the diagnostic counter~~
   **The COUNTER, however, is built — by this rung, in the gather** *(corrected 2026-08-21 —
   S-D18 (1): G5-6 three rows below requires a clamp counter, so the rung both retired one for want
   of a home and demanded one two paragraphs later. S-D16 (3) puts the sheet arithmetic in the
   gather, which owns `UiGatherScratch` — the struct that already carries the unconditional `probes`
   counter for exactly this reason.)* as `UiGatherScratch::sheet_index_clamps`.
   **The bit budget is EXHAUSTED by this** — fifteen free bits, fifteen spent — and §6's exposure row
   says so, because the animation and interaction plans read it to decide whether they may take one.
   **S4's `border_uv` composes with this for free and needs no S5 edit** *(added 2026-08-21 —
   S-D12 (2))*: it is a fraction of the node's CURRENT sub-rect, so once item 2 makes that sub-rect a
   sheet frame, a nine-sliced sheet-framed node slices the frame rather than the atlas — the same
   "wrap and inset both belong to the sub-rect" property `frac` relies on. An absolute-UV inset would
   have had to be re-derived on every flipbook tick.
   Gates: **G5-7** the tiled golden `ui_nine_slice_tiled` (a tiled edge shows N repeats of the source's
   edge cell, not one clamped streak — the failure the retired mechanism would have shipped silently),
   and **G5-8** `Tile` over a sheet frame samples only within that frame's sub-rect (the assertion
   S-D7 could not make because it forbade the combination instead).

**Uniform grids only (D8c).** Ragged/trimmed sheets and per-frame durations are deferred with their
shape recorded: a second sub-rect column and an optional run-length `frame_run: u8` column (Unreal's
`UPaperFlipbook` compression), both needing an asset-pipeline dependency that no in-tree asset
exercises.

**Dependency — the clock.** D15 (real vs virtual delta, per row) belongs to
[`UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md). This rung consumes whatever time source that plan
exposes. ~~**If the animation plan has not landed, S5 reads `Time`'s real delta directly and the seam
is one function** — `ui_frame_delta(world) -> f32` — which the animation plan later replaces in one
edit.~~ **If the animation plan has not landed, `ui_sprite_flipbook` takes `Res<Time>` as a
`SystemParam` and applies AM6's clamp itself at that one site —
~~`dt = time.real_delta().as_secs_f32().min(UI_FALLBACK_MAX_DELTA)`~~
**`dt = time.delta_secs().min(UI_FALLBACK_MAX_DELTA)`** with `UI_FALLBACK_MAX_DELTA = 0.1`, AD1's own
default *(corrected 2026-08-26 at the landing: S-D17 (1) names TWO defects of `real_delta()` in one
sentence — an alt-tab stall that skips whole cycles AND a paused game that keeps animating — and a
`min` fixes only the first. `delta_secs()` is already clamped, scaled and pause-aware, and AD1's
tighter clamp still applies on top of it. G5-2's clock test asserts all three properties, because a
remedy that covers one of two named defects and is silent about the other is the shape this campaign
keeps finding.)* — and the replacement is one parameter swapped for
`Res<UiClock>`'s **`dt_virtual`** and one clamp deleted.** *(2026-08-26, [`UI-PLAN-ANIMATION-DECISIONS.md` **AD9**](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition) **(1)/(2)** at the A0 pre-build audit — **the field is `dt_virtual`.** Neither document named one, and the animation plan's own AD1 called `dt_real` "the default": taking it reds two legs of this rung's SHIPPED `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware` — a paused game animates, and `set_relative_speed(0.5)` stops halving. `dt_virtual` is `time.delta_secs().min(max_delta)`, i.e. `ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*'s arithmetic verbatim, so "one parameter swapped, one clamp deleted" is true of that field and of no other. The swap itself is owned by animation rung **A0b** — it belonged to no rung in either ladder until then.)* *(both halves corrected 2026-08-21 — **S-D17**. The struck
value is the option [`UI-PLAN-ANIMATION-DECISIONS.md` AD1](UI-PLAN-ANIMATION-DECISIONS.md#ad1--uiclock-is-a-resource-not-a-restime-read-at-each-consumer) REJECTS by name, for this consumer by name: `Time`'s
`DEFAULT_MAX_DELTA` clamps the virtual delta only — `time.rs:197` assigns `real_delta = raw` BEFORE
the clamp at `:201`, and `real_delta()` is documented "unclamped, unscaled, pause-blind" — so an
alt-tab stall hands the flipbook a two-second delta that skips whole cycles and jumps `Once` to its
end, and a paused game keeps animating. And the struck SIGNATURE is uncallable: there is no `world`
handle inside a scheduled `Query`-bearing system (`WorldView` is minted only from a
`DispatcherToken`'s `&self` and is `!Send`/`!Sync`; a `&mut EcsMaster` would force the flipbook to be
an EXCLUSIVE system), so it matched neither the mechanism it fell back to nor the one it is replaced
by — both of which are resource reads.)* S5 is therefore **not blocked** on the animation plan; it is
only less configurable without it.

**Measurement.** *(added 2026-08-21 — S5 was, after S4's was written in, the LAST rung in this plan
with no measurement paragraph, while §5 assigns it leg 10.8(c). Every other rung that moves the
number carries the paragraph in its own text.)* §5 leg **10.8(c)**, the increment this rung owes:
the list holds six pack inputs today (`gather.rs:126-138`) and the census is `ui_pack_inputs!(count)
+ 1` = **7.00 probes/node/frame** (MEASURED on the S4 build). Item 6 adds ONE, giving **8.00
(+14.3 %)**, paid by every node of every changed frame whether or not it is a sprite.
⚠️ *(2026-08-28: **the anchor and the count were both stale, and independently.** The coordinate
read `:76-86`, which is doc-comment prose ABOUT the list, never the list — `__ui_pack_inputs_list!`
is at `:126-138`. And that list holds **seven** members today, not six: `ComputedRect`,
`UiBackground`, `ComputedClip`, `StackIndex`, `UiImage`, `UiNineSlice`, `UiSpriteSheet` — S5's
`UiSpriteSheet` landed after the S4 build this paragraph measured. The `7.00` above is a dated
measurement and is left as measured; the derived increment it feeds is therefore owed a
re-measurement before this rung reports leg 10.8(c). Filed in `docs/OPEN-QUESTIONS.md`.)* Report it in
both worlds at N ∈ {256, 2048}, on `ui_s0_measure`'s existing instrument, and **add the row to that
file's prose ladder in the same edit that moves the list** — the ladder's own doc requires it and
nothing machine-checks it. The number is arithmetic and is stated as such; the wall-clock half is
reported with the instrument's floor, per §5's closing rule. *(§5's "S5 owes 7.00 → 10.00 (its
three)" and §6's "SIX after S4 … 7.00" are corrected in place.)*

**What a node actually DRAWS under these rulings** *(added 2026-08-21 at the S5 pre-build audit. S4's
second ruling was self-cancelling — it fixed the defect it was asked about and created a new
gate-that-cannot-fail in the same sentence, and only an implementer's end-to-end trace caught it. So
the trace is written down here, before the code, for the three combinations this rung makes
expressible.)*

**(a) A plain sheet frame** — `UiImage` + `UiSpriteSheet{ sheet, index: 6 }`, no `UiNineSlice`.
Gather: 8 probes; `index < frame_count` (else clamp + `sheet_index_clamps`); `col = 2, row = 1`;
`UiImageInput{ slot: sheet.slot, uv: (0.53125, 0.28125, 0.71875, 0.46875), tint }`.
`ui_node_sub_codes` takes the `(None, Some(_))` row ⇒ **2 records**, `[0, UI_IMAGE_SUB]`. The image
record packs `FLAG_TEXTURED | slot<<20`, no `FLAG_TILED` (no `nine_slice` ⇒ no `mode`). The FS takes
the untiled side and reads only frame 6's texels. *G5-1's pixel is the uv constant; G5-5's is the
frame's identity, which is why all sixteen frames must be mutually distinct; G5-6's is a clamp on a
node whose `index` exceeds `frame_count`, observed in a counter rather than in a picture.*

**(b) A tiled nine-slice, no sheet** — `UiImage` + `UiNineSlice{ mode: Tile }` on G4-3's scene.
`ui_node_sub_codes` takes `(Some, Some)` ⇒ **10 records**, `[0, 1..=9]`; the whole-rect image record
is suppressed (S-D12 (1)). Pack derives `tiles = (4, 2)` from `border_px`/`border_uv`; the four
corners get `(1, 1)` and therefore **no `FLAG_TILED`**, the top/bottom edges `(4, 1)`, the left/right
edges `(1, 2)`, the centre `(4, 2)`. The FS wraps inside each region's own sub-rect. *The top edge's
64 px carries eight 8-px bands where `Stretch` carries two 32-px bands — G5-7's probe pair is a
pixel whose value is a function of `tiles_x`, which is the thing the row exists to prove, and M5-e
moves it because a UV past `[0,1]` clamps instead of wrapping.*

**(c) A nine-sliced sheet frame** — all three. The gather substitutes the frame rect into
`UiImageInput.uv` **first**, and `pack_ui_nine_slice_instance` then slices THAT by `border_uv`
fractions — so the nine sub-rects live inside frame 6, and `border_uv`'s landed doc sentence stays
true. `tiles` is unchanged from (b), because the sub-rect extent cancels out of S-D15 (3)'s ratio.
Still **10 records**. *G5-8's pixel is a colour drawn from frame 6's disjoint palette; under M5-e the
top edge's UV sweeps four frame-widths past `sub_min` and lands in frame 7's palette, which is what
makes the census red rather than merely different.*

**The one thing this trace does NOT make true, stated rather than gated away:** under
`UiSamplerMode::Smooth` the hardware's bilinear tap at a TILE SEAM straddles `sub_max → sub_min` and
therefore reads one texel outside the sub-rect. `UiSheet.inset_uv` cannot fix it — the inset is on
the frame's outer edge and the seam is interior. G5-7 and G5-8 run `Pixel`, where the artifact does
not exist; a per-sprite `REPEAT` sampler would fix it and is S7's deferred lever (bit 4).

**Gate.**

| # | Claim | How |
|---|---|---|
| **G5-1** | The frame UV is the stated arithmetic | CPU table test: `(cols=4, rows=4, index=6, inset_uv=(h,h))` → an exact hand-computed `uv` constant. Asserted against the constant, **not** against the implementation. **On S-D18 (4)'s 16×16 source, `h = 1/32` and the constant is `(0.53125, 0.28125, 0.71875, 0.46875)` — exact in binary FP.** **Plus a NON-SQUARE case `(cols=4, rows=2, frame_count=8)`** *(added 2026-08-21 — S-D18 (5): with `cols == rows` the decode `col = index % cols; row = index / cols` is bit-identical to the same expression with the two interchanged, so the standard sprite-sheet transpose passes both this constant and G5-5's hash. This is S4's dihedral-symmetry finding one axis over, and it costs one extra row.)* |
| **G5-2** | The four modes are exactly right at the turns | A deterministic tick harness at fixed `dt` over 3 cycles per mode; the `frame` sequence pinned as a **literal array** in the test. **`Time::advance_with(fixed)` between `Schedule::run`s is the harness** — it is `pub` (`time.rs:178`) and its `debug_assert!` forbids only calling it INSIDE a system body, so a driver-shaped test may. **`Once` and `repeats` are covered, since `UiSpriteCursor.loops_done` is what makes them expressible** (S-D16 (2)). |
| **G5-3** | The churn split is real | `Changed<UiSpriteAnim>` fires on an author retarget and **never** on a per-frame advance. **The per-frame advance it must stay silent through is a `Mut<UiSpriteSheet>::set_if_neq` write** *(clarified 2026-08-21 — S-D16 (1): the flipbook's tick-bearing per-frame write lands on `UiSpriteSheet`, not on the cursor, and D8a's stated benefit is precisely that `UiSpriteAnim` is untouched by it.)* **Second leg: at a frame rate where the index does NOT move between two ticks, `Changed<UiSpriteSheet>` must ALSO stay silent** — that is `set_if_neq`'s whole purpose and nothing else in the ladder reads it. |
| **G5-4** | The cursor is dense and does not migrate | Insert/remove `UiSpriteCursor`, assert the entity's archetype id is unchanged (`dense_d2_routing`'s property, re-asserted at this consumer). *(Constructible: `EcsMaster::entity_archetype_id` is `pub` at `entity_query_api.rs:35` and `dense_d2_routing.rs:302/316` already asserts exactly this shape.)* |
| **G5-5** | It animates on the GPU | Golden `ui_flipbook_frame3`: a 4×4 procedural grid (S-D5) at a fixed tick count; image hash pinned. **At `UiSamplerMode::Pixel`, on a 16×16 source (4×4 frames of 4×4 texels) whose SIXTEEN frames are mutually distinct, and honouring `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1`** *(added 2026-08-21 — S-D18 (2)(3)(4). The mode, because a which-frame claim under `Smooth`/LINEAR is a filter claim — the amendment G4-3 already took. The source size, because "4×4 grid" read as 4×4 TEXELS gives one-texel frames on which the half-texel inset collapses the frame extent to exactly ZERO. The distinctness, because a hash cannot see an off-by-one between two identical frames — the S4 lesson, one axis over. The env guard, because it is not a shared helper: `BOYKO_UI_GOLDEN_REQUIRE_DEVICE` occurs in exactly ONE file in the workspace and `boot_or_skip` exits 0.)* |
| **G5-6** | `frame_count < cols*rows` is honoured | `index >= frame_count` clamps to `frame_count - 1` and increments **`UiGatherScratch::sheet_index_clamps`** (trailing cells are never sampled). *(The counter's home is S-D18 (1)'s finding: the pack's five entry points are receiverless free functions with nowhere to put one — the reason the S4 ledger retired this very counter — but S-D16 (3) moves the sheet arithmetic into the gather, which owns the scratch that already carries `probes` unconditionally for exactly this reason. `frame_count == 0` ⇒ the sheet is treated as absent and the node emits no sprite record.)* CPU, device-free. |
| **G5-7** | `Tile` actually tiles | *(inherited from S4, 2026-08-21 — S-D11; respecified 2026-08-21 — S-D15 + S-D18 (2)(4).)* Golden `ui_nine_slice_tiled`, **at `UiSamplerMode::Pixel`**, on G4-3's scene verbatim (`rect 96×96`, `border_px = [16, 24, 16, 24]`, `border_uv` at its equal-thirds `Default`, opaque white tint) — for which S-D15 (3)'s derivation **computes** `tiles = (4, 2)` rather than asserting it — over a **6×6 source: nine 2×2 cells, each cell two distinct values** *(the 3×3 source CANNOT distinguish tiling: each region of it is exactly one uniform texel, and four repeats of a uniform texel are byte-identical to one stretched copy, so the blessed hash would be reproduced by a `Tile` that silently fell back to `Stretch` — the entire failure this row exists to catch, and the S4 amendment (a) recurring one axis over)*. The 64-px top edge shows **eight 8-px bands, not two 32-px bands**; assert NAMED PROBE COLUMNS as well as the hash, because the columns are what depend on the count — **`x = 36` and `x = 44` on the top edge, and `y = 46` and `y = 58` on the left edge, each pair DIFFERING under `Tile` and AGREEING under `Stretch`** (the top edge spans x 32..96 at `tiles_x = 4`, so each repeat is 16 px and each source texel 8 px; the left edge spans y 40..88 at `tiles_y = 2`, so each repeat is 24 px and each texel 12 px — under `Stretch` both probes of each pair fall in the first source texel). **The four corner regions must be byte-identical to THE SAME SCENE rendered at `NineSliceMode::Stretch`** — a second leg of this row, not a comparison with G4-3, whose source is 3×3 — since S-D15 leaves `FLAG_TILED` clear wherever both counts are 1. Honours `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1`. ~~**This is the gate S4 did not have** — its table tested `Stretch` only (G4-3) and the `Tile`+sheet clamp (G4-5), so half of a two-valued field would have landed ungated.~~ **This is the gate S4 did not have, and the true statement is stronger than the struck one: S4 gated no mode BEHAVIOUR at all.** *(corrected 2026-08-21 — the `Tile`+sheet G4-5 was STRUCK at the S4 pre-build audit as a gate that could not fail and never landed; what landed as G4-5 is a `#[should_panic]` on the raw-`u8` bound (`ui_s4_nine_slice.rs`, `#[cfg(debug_assertions)]`). And `mode` is ONE-valued at S4, not two: `UI_NINE_SLICE_MODE_COUNT = 1`.)* |
| **G5-8** | `Tile` under a sheet stays inside its frame | *(inherited, and it is the assertion S-D7 could not make because it FORBADE the combination.)* A tiled nine-slice on a node carrying `UiSpriteSheet`: every sampled texel lies within that frame's sub-rect — no neighbouring frame contributes. `frac`-in-sub-rect makes this true by construction; the gate is what proves the construction. **The instrument is a COLOUR-PALETTE census, because "which texel was sampled" is not directly observable in a readback** *(added 2026-08-21 — S-D18 (2): the row stated a property with no way to read it. Each of the sheet's sixteen frames gets a PALETTE DISJOINT from every other frame's — 16 frames × 36 texels, all 576 values distinct — and the assertion is that every non-background pixel of the readback belongs to frame 6's 36. That is decidable per pixel under NEAREST, it names the neighbour that would contribute if the wrap escaped, and it is the same "a colour census can confuse two subjects" discipline the S4 golden's own accounting earned.)* **At `UiSamplerMode::Pixel`, on a 24×24 sheet (4×4 frames of 6×6 texels) with `inset_uv = (0, 0)`, honouring `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1`** *(added 2026-08-21 — S-D18 (2)(4). NEAREST because "every sampled TEXEL" is decidable per texel only under NEAREST: under LINEAR the hardware tap at a tile seam straddles `sub_max → sub_min` and reaches outside the frame, and no inset can fix an interior seam — that limitation is recorded here rather than gated away. Zero inset because it protects against a bleed NEAREST does not have, and because it makes each frame exactly six texels per axis so each nine-slice region is exactly 2×2. The frame's NEIGHBOURS must be distinct, or M5-e's escape has nothing to land in.)* |
| **G5-9** | `inset_uv` is protecting something | *(added 2026-08-21 — S-D18 (2): the field's entire stated purpose is "half-texel inset against bilinear bleed", which is INERT under NEAREST, so on a `Pixel` row M5-b's second half cannot fire — the exact shape of a red that cannot fire.)* The G5-5 scene at **`UiSamplerMode::Smooth`**, asserting NAMED PROBES at the frame's outer edge rather than a hash: with the inset, the edge probe lies within frame 6's own colour range; without it, it carries a measurable contribution from frame 5 / frame 2. **Probes, not a hash and not an image statistic** — an 8-bit hash is blind to a sub-texel blend and image statistics lie about render changes; the number comes from the readback at a named texel. |
| **G5-10** | The macro landing is driven, not just written | *(added 2026-08-21 — S-D18 (8): S4's identical Lands line was gated by G4-7, and the landed ledger calls that edit "the omission that would have made the rung invisible"; this table had no row for it.)* `ui_s0_discovery`'s no-catch-all loop drives the new `PackInput` variant end to end — an author's runtime edit to `UiSpriteSheet` bumps `UiRenderGeneration` — and `PackInput::ALL.len() == ui_pack_inputs!(count)` holds (the assertion that turns "added to the macro but not to this test" into a red with a reason; the array's arity is hard-coded in its own TYPE, so it is one of the landings). CPU, device-free. |

**Red mutations.**

* **M5-e — implement `Tile` as a UV past `[0,1]` instead of `frac` in the sub-rect** *(inherited from
  S4's retired mechanism, 2026-08-21; re-armed 2026-08-21 — S-D15, because under the mechanism as
  S-D11 spelled it this red could not fire at all: `lerp(uv.xy, uv.zw, t)` and
  `uv.xy + frac(t)*(uv.zw - uv.xy)` compute the same fragment for every `t` in `[0,1)`, so the
  mutation changed no pixel and G5-7 could not distinguish the two. It fires against S-D15's
  `frac(t * tiles)`: the mutated form sweeps `tiles ×` the sub-rect extent, which leaves the frame.)*
  G5-7 reds with a clamped streak on every edge, and G5-8 reds under a sheet. *Proves S-D15's
  mechanism is load-bearing and re-runs, as a red, the exact thing S-D7 believed was the only option
  — the UI's sampler is `ClampToEdge` in both modes, so the retired mechanism does not even reach the
  sheet hazard it was designed around: it fails one step earlier.*
* ~~**M5-a — merge `UiSpriteAnim` and `UiSpriteCursor` into one component.**~~ **M5-a — merge
  `UiSpriteAnim` INTO `UiSpriteSheet`.** G5-3 reds — the change tick fires every frame the index
  moves. *(respecified 2026-08-21 — S-D18 (7): as spelled it was a red that could not fire. The
  merged target was the cursor, which the flipbook writes with `&mut` (no tick — `write.rs:234`), so
  the merged component would not be `Changed` either and G5-3 would have stayed green while proving
  nothing. `UiSpriteSheet` is the component the flipbook DOES tick-write, so merging the track into
  it makes `Changed<UiSpriteAnim>` fire exactly as D8a says it must not.)* *This is the mutation that
  makes D8a a measurement rather than a preference: the merged shape destroys `Changed<UiSpriteAnim>`
  as a signal, and nothing else in the ladder would notice.*
* ~~**M5-b — drop `inset_uv`.**~~ **M5-b — IGNORE `inset_uv` in the frame-UV derivation, leaving the
  field in place.** G5-1 reds (the constant moves), and **G5-9**'s `Smooth` frame-edge probe takes a
  measurable contribution from the neighbouring frame. *(respecified 2026-08-21 — S-D18 (2)(6):
  DELETING the field is a compile error, not a red — G5-1 names `inset_uv` as one of its four inputs
  — and the protocol requires the predicted failure OBSERVED. The second half moved off G5-5 because
  G5-5 runs `Pixel`, under which the inset is inert.)* *Proves the half-texel inset is protecting
  something.*
* **M5-c — make `UiSpriteCursor` a table component.** G5-4 reds. *Proves the storage claim is
  enforced.*
* **M5-d — flip `dir` one frame late at the `PingPong` turn.** G5-2's pinned array reds. *The classic
  flipbook off-by-one, and the reason G5-2 pins a literal sequence: an eyeball check of "it animates"
  cannot see it, and neither can an image golden at a single tick count.*
* **M5-f — swap `cols` and `rows` in the frame decode.** *(added 2026-08-21 — S-D18 (5).)* G5-1's
  NON-SQUARE row reds; its square row does not, and neither does G5-5's hash. *The characteristic
  sprite-sheet defect, and the reason G5-1 carries a second case: on a square grid the transposed
  decode is bit-identical to the correct one, so the whole gate table was blind to it.*
* **M5-g — write `UiSpriteSheet.index` through `&mut` instead of `Mut::set_if_neq`.** *(added
  2026-08-21 — S-D16 (1).)* G5-10's discovery leg still passes (the author's edit is a separate
  write), but the FLIPBOOK's own repaint stops: `UiRenderGeneration` never bumps on a tick, the D6a
  gate keeps skipping, and G5-5's golden shows frame 0 at every tick count. *This is the rung's
  quietest failure and the one the kernel measurement in S-D16 (1) exists to make impossible to walk
  into: it is a frozen picture with no error, no panic and no failing assertion anywhere else.*
* **M5-h — clamp an out-of-range sheet index to `frame_count` instead of `frame_count - 1`, and
  (separately) drop the counter increment.** *(added 2026-08-26 at the landing: **G5-6 was named by
  no mutation at all**, and the protocol wants the failure OBSERVED.)* (a) reds G5-6's UV leg — the
  node samples a TRAILING cell of a partly-filled grid, which holds nothing; (b) reds its counter
  leg. *The clamp is not otherwise observable: a clamped node draws a real frame, so no picture and
  no UV assertion can tell "the author asked for frame 13 of a 12-frame sheet" from "the author
  asked for frame 11".*
* **M5-i — add `UiSpriteSheet` to `ui_pack_inputs!` but NOT to `PackInput::ALL`.** *(added
  2026-08-26 at the landing, for the same gap: **G5-10 was named by no mutation**.)* Reds
  `ALL.len() == ui_pack_inputs!(count)` with its own reason.
* **M5-j — set `FLAG_TILED` unconditionally, so a `1×1` corner stops being byte-identical to its
  `Stretch` record.** *(added 2026-08-26 at the landing.)* ⚠️ **It did NOT fire on its first run** —
  see **S-D19 (6)**, which is the finding, not the mutation. After G5-11 gained the ABSOLUTE
  assertion it reds immediately. *The two causes are both worth knowing: no golden can EVER see this
  mutation (`frac(local_uv * 1) == local_uv`), and the gate that was supposed to was a COMPARISON
  between two arms that share the mutated function.*

---

### S5 · LANDED 2026-08-26 — the landed set, the RED ledger, the goldens, and what the build found

*(First build, after the pre-build audit's S-D15..S-D18 amendments and the check lens's row-level
corrections. Every gate below was run with its exit code seen UNPIPED and `running N` confirmed in
BOTH profiles; every red was APPLIED and its failure OBSERVED; every mutated source was restored and
verified byte-identical with `cmp` against a pre-mutation snapshot. Six corrections landing found are
ruled in **S-D19**.)*

#### The landed set, file by file

| File | What landed |
|---|---|
| `crates/boyko_shaderdsl/src/cf.rs` | THREE new `Cf` facets, not the one S-D15 (4) costed: `vec2_frac` (the wrap), `vec2_lerp` (the untiled arm, spelled as the `FMix` INTRINSIC — see S-D19 (1)), and `named_uint_val` (a `uint` symbol that types as `Uint`, which the existing `named_uint` does not — S-D19 (2)). Eval arms for all three. |
| `crates/boyko_shaderdsl/src/emit/{mod,cf}.rs` | `Node::{Vec2Frac, Vec2Lerp, NamedUint}` + their type-table, inline-leaf and printer arms. |
| `crates/boyko_shaderdsl/src/ui.rs` | The SEVENTH leaf `ui_tile_uv_body`, plus the four tile-bit generator inputs (`UI_TILE_FLAG_BIT`/`X_SHIFT`/`Y_SHIFT`/`BITS`) and three `const _` budget relations. |
| `crates/boyko_shaderdsl/src/emit/shaders.rs` | `emit_hlsl_ui_tile_uv`; `UiInstanceLayout` gains four tile fields; the mirror and `ui_flag_consts` spans emit them. **And `emit_ui_leaf` now feeds the interned symbol table to the printer** — it was `NO_NAMED_LITS` while the six S1 leaves spelled only bare literals, and an empty table under a symbol node is an index-out-of-bounds panic AT GENERATION (S-D19 (2)). |
| `crates/boyko_shaderdsl/src/bin/emit_ui.rs` | The layout literals; the seventh `leaf(..)`; the template's sprite line becomes `ui_tile_uv(inst.uv, input.local_uv, inst.flags)`; four `const _` asserts binding the layout to `boyko_shaderdsl::ui`'s copy. |
| `crates/boyko_render/shaders/ui_rect.{vs,fs}.hlsl` + `.spv` | RE-EMITTED and re-DXC'd with the frozen recipe. FS `8760 → 9120`; **VS `2408 → 2408`, byte-identical** — but its `.hlsl` DID move by one comment line, because the tile bits land in the SHARED mirror span (S-D19 (3)). |
| `crates/boyko_render/src/ui/instance.rs` | `FLAG_TILED` (bit 5), `UI_TILE_X_SHIFT`/`Y_SHIFT`/`BITS`/`MASK`/`MAX`, and FOUR `const _` relations that together say the S-D2 budget is EXHAUSTED (fifteen free, fifteen spent, ending exactly at the slot field). |
| `crates/boyko_render/src/ui/pack.rs` | `UI_NINE_SLICE_MODE_COUNT` `1 → 2` + `UI_NINE_SLICE_MODE_TILE`; **`ui_nine_slice_tiles_axis`** (S-D15 (3)'s ratio, with its four degenerate arms) and `ui_nine_slice_tiles`; `tile_flag_bits`; the per-region application inside `pack_ui_nine_slice_instance` (X on the centre column, Y on the centre row, `1` elsewhere). |
| `crates/boyko_render/src/ui/gather.rs` | `UiSpriteSheet` — **and it alone** — added to `__ui_pack_inputs_list!`; the read tuple widened; `sheet_frame` (the substitution, through `try_resource` — S-D19 (4)); `UiGatherScratch::sheet_index_clamps`; the `NineSliceMode::Tile` arm of the one exhaustive conversion, reached via `E0004` exactly as S4 designed. The macro's own doc gains the S-D16 (1) narrowing: "wires the discovery filter for free" is TRUE FOR TABLE COMPONENTS ONLY. |
| `crates/boyko_render/src/ui/upload.rs` | `UiUploadSystem::sheet_index_clamps()`, on `probes()`'s precedent. |
| `crates/boyko_render/src/ui/mod.rs`, `src/lib.rs` | The new surface re-exported; `SpirvBlob<8760>` → `<9120>` with the re-bless recorded in its doc. |
| `crates/boyko_ui/src/components.rs` | `NineSliceMode::Tile`; `UiSpriteSheet` (4 B, table, `PartialEq` for `set_if_neq`); `SpriteAnimMode`; `UiSpriteAnim` (12 B, table, cold, spelled `_pad`); `UiSpriteCursor` (8 B, **dense**, spelled `_pad`, hand-written `Default` with `dir: 1`). All four sizes MEASURED by `const _: () = assert!` — plus the two-variant brace-less `const` match that now pins "exactly two modes". |
| `crates/boyko_ui/src/sprite.rs` | **NEW** — `SheetId`, `UiSheet` (20 B) + `frame_uv`, `UiSheetTable` + `register` (the mint S-D16 (3) found missing), `UI_FALLBACK_MAX_DELTA`, and `ui_sprite_flipbook` with its mode/repeat semantics. |
| `crates/boyko_ui/src/bundles.rs` | **`AnimatedSpriteBundle`** — the cursor pairing made structural at the AUTHORING site, because `#[require]` cannot make it structural at the component (S-D19 (5), a kernel defect filed in `OPEN-QUESTIONS.md`). |
| `crates/boyko_render/tests/ui_s5_sprite_sheet.rs` | **NEW** — G5-1 (two tests), G5-2 (two), G5-3, G5-4 (two), G5-6 (two), G5-11 (two), G5-12. Twelve device-free tests, driving `gather_into_staging` and a real `Schedule`. |
| `crates/boyko_render/tests/ui_flipbook_gpu_golden.rs` | **NEW** — G5-5 (two tick counts, two hashes) and G5-9 (named `Smooth` probes). |
| `crates/boyko_render/tests/ui_nine_slice_tiled_gpu_golden.rs` | **NEW** — G5-7 (four named probes + corner byte-identity vs `Stretch` + a 19-colour census + a hash) and G5-8 (the 577-value palette census + a hash). |
| `crates/boyko_shaderdsl/tests/ui_leaves.rs` | The `ui_tile_uv` Eval table (BOTH arms, plus a 6×64-point containment sweep over every count the field can hold) and its literal span pin. |
| `crates/boyko_render/tests/ui_s0_discovery.rs` | G5-10: the seventh `PackInput` variant, `ALL`, `name()` and `mutate_pack_input` arm. |
| `crates/boyko_render/tests/ui_rect_edsl_sync.rs` | The `ui_tile_uv` span row, the layout's four tile fields, and a THIRD pin — `ui_tile_bit_layout_matches_the_host` — because the leaf carries its own copy of the bit layout and no existing gate could see it drift. |
| `crates/boyko_render/tests/ui_s0_measure.rs` | The §10.8(c) ladder row: 7 pack inputs + `Children` = **8.00 probes/node/frame** (+14.3 %). |
| `docs/SHADER-VARIANT-MANIFEST.md` | The two existing rows gain the tiled lane and the seventh leaf; the landing-history table gains its S5 row. **ZERO new variant rows** — `FLAG_TILED` is a runtime bit. |
| `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` | Two anchors re-pointed `ui/mod.rs:96 → :97`; the `SpirvBlob` doc edit moved `FRAMES_IN_FLIGHT`, and `internal_docs_anchors` caught it — the same instrument, the same file, one rung later. |
| `docs/OPEN-QUESTIONS.md` + `docs/ru/OPEN-QUESTIONS.md` | The `#[require]`-on-dense kernel defect, both sides, same edit. |

#### The RED ledger — nine mutations, nine observations

| Red | What was mutated | What was OBSERVED |
|---|---|---|
| **M5-b** | `UiSheet::frame_uv` ignores `inset_uv` | G5-1 reds (`uv` becomes the un-inset `[0.5, 0.25, 0.75, 0.5]`) **and** G5-9's `Smooth` left-edge probe reads `[109, 97, 153, 255]` — between frame 6's `[114, 104, 150]` and frame 5's `[103, 90, 157]`, i.e. the ~48 % neighbour contribution the row's own arithmetic predicts, to the byte |
| **M5-f** | `cols`/`rows` swapped in the decode | G5-1's NON-SQUARE row reds with exactly the hand-computed transposed value `[0.03125, 0.78125, 0.46875, 0.96875]`; the SQUARE row does not, and neither does G5-5's hash — S-D18 (5)'s finding, confirmed by measurement |
| **M5-d** | `PingPong` flips `dir` one frame late | G5-2's literal array reds: `[1,2,3,3,2,1,0,0,1,2,3,3]` — the endpoint repeated at both turns |
| **M5-g** | `index` written through `bypass_change_detection` | G5-3's first leg reds (`Changed<UiSpriteSheet>` count 0 on an advance frame) **and** G5-5's golden reds with the FROZEN picture — no panic, no error, just frame 0 at every tick count. The harness dispatches the upload EVERY tick precisely so this can fire |
| **M5-c** | `UiSpriteCursor` made a table component | G5-4 alone reds, at the `dense_contains` assertion |
| **M5-a** | the flipbook tick-writes `UiSpriteAnim` too | G5-3's D8a leg reds (`Changed<UiSpriteAnim>` fires on a per-frame advance). *Applied in the check lens's source-only form rather than as a literal merge: S-D18 (7)'s "merge `UiSpriteAnim` INTO `UiSpriteSheet`" DELETES a type three gates name, so the target fails to BUILD rather than to assert — the very shape S-D18 (6) struck for M5-b two bullets earlier. A per-frame tick-write on the animation track has the identical observable consequence and is one line.* |
| **M5-h** | (a) clamp to `frame_count`; (b) drop the counter | (a) G5-6 reds sampling frame 12 (`[0.0, 0.75, 0.25, 1.0]`); (b) G5-6 reds with `clamps == 0`. Added because G5-6 was named by NO mutation |
| **M5-i** | `PackInput::ALL` left at 6 while the macro says 7 | G5-10 reds with its own reason. Added for the same gap |
| **M5-j** | `FLAG_TILED` set unconditionally | ⚠️ **DID NOT FIRE on the first attempt — see S-D19 (6).** After the gate was repaired it reds with `flags & tile_mask == 0x2060` |

**G5-12 has no red mutation, and the reason is that it is one.** Its subject is a `Bundle` field
list, and every mutation of a field list is a compile error at the construction site rather than an
assertion failure (S-D18 (6)'s shape). Its SECOND leg is the standing red: a hand-spawned
`UiSpriteAnim` with no cursor is asserted FROZEN, which is the hazard the bundle exists to remove,
observed on every run rather than once.

#### The goldens

Four new SHA-256 image pins, taking the campaign's total from six to ten. Each was blessed on this
box (RTX 3060 Laptop, validation on), **LOOKED AT**, and its every distinct colour accounted for:

* `ui_flipbook_frame3` `0fd69179…` and `ui_flipbook_frame7` `c948b989…` — **2 colours each**: the
  clear ground (7 168 px = 128² − 96²) and the frame's own (9 216 px = 96²), with a bounding box
  measured at cols 16..111 / rows 16..111. The two frame colours read back as `(0x51,0x3E,0xAB)` and
  `(0x7D,0x76,0x8F)`, which are `frame_rgba(3)` and `frame_rgba(7)` EXACTLY.
* `ui_nine_slice_tiled` `9dc817b6…` — **19 colours**: the clear ground and all eighteen source
  values, every one present. The rendered top edge is `ddddeeee` repeated FOUR times — eight 8-px
  bands where `Stretch` carries two 32-px bands — and the left edge shows its two source rows twice
  over 48 px. The corners show two texels each, unrepeated.
* `ui_tiled_sheet` `766f7997…` — **37 colours**: the clear ground and ALL 36 of frame 6's texels,
  none of any other frame's.

**The six pre-existing pins did NOT move**, and that is the shader edit's own evidence: the untiled
arm still spells the same `lerp(uv.xy, uv.zw, t)` intrinsic on the same operands, so an untiled
sprite's pixel is IDENTICAL rather than equal-to-within-a-ULP (which an 8-bit golden could not tell
apart either way — `reference-golden-fp-resolution`).

#### Both profiles

`ui_s5_sprite_sheet` reports **`running 12 tests`** in debug AND release, and unlike
`ui_s4_nine_slice` the two SETS are the same twelve: no S5 sentence is profile-gated, because none
of them is about a `debug_assert!`. The release leg is still run, because the rung packs into a
`flags` word through shifts and masks and release is where an overflow would be silent.

---

