> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §4 rung S4 and S4 · LANDED. Ladder gate: see the index.

### S4 — nine-slice: CPU expansion + the D4 emission contract — **size M**

*Architecture D8d and D4, plus ~~S-D7~~ **S-D11** (S-D7 is retired; see the audit ruling immediately
below).*

> **PRE-BUILD AUDIT RULING — 2026-08-21, before one line of S4 was written.** Three lenses read this
> rung against the tree S3 landed in. **Eleven findings were confirmed by direct verification**, five
> of them blocking; two claims were **refuted by measurement** and are recorded as such rather than
> acted on. The rung as originally written **could not be built**: three of its five gates had no
> constructible subject, one red mutation was unwritable, its two headline numbers (the record count
> and the staging budget) were undetermined or wrong, and the one item that would have made
> `UiNineSlice` visible to the renderer at all was missing from the list. Every sentence below is
> struck rather than deleted — the record of what was believed and why it was wrong is the point.
> The full ruling is the **S4 audit ledger** after the red mutations.
>
> **SECOND PRE-BUILD RULING — 2026-08-21, still before one line of S4 was written.** The implementer
> **refused to build the amended rung**, and was right; an adversarial pass confirmed the refusal and
> found more. Seven further findings, four of them blocking, are ruled by **S-D12**: the emission
> contract **occluded itself** (the sub-10 image covered the nine regions it was slicing, so G4-3
> could not fail and M4-b/M4-e could not fire), the **source-side UV split was stated nowhere** while
> a red presupposed it, a **nine-sliced node without `UiImage`** had no ruled behaviour and no gate,
> and M4-f's threshold was **a number asserted rather than computed**. The root of the first three is
> one sentence: S-D11 (3) ruled on the BACKGROUND record and generalized to the IMAGE record. Rows
> **13-19** of the ledger.
>
> **THIRD PRE-BUILD RULING — 2026-08-21, still before one line of S4 was written.** The implementer
> refused a **second** time and was right a second time; a second adversarial pass confirmed the
> refusal. Fourteen findings, two of them blocking, are ruled by **S-D13**: **M4-c could not be
> applied under S-D12's own truth table**, which left G4-2 — the row carrying S-D12 (1)'s headline
> claim — with **no red at all** (and G4-5 and G4-7 named by none either); and the `pack_sort_upload`
> loop that Lands item 2 and G4-1 both required the expansion to land in **has no caller anywhere in
> this workspace**, so half the gate could not be run and half the code could not be exercised. The
> **✅ BUILT AND LANDED 2026-08-21 — the third implementer built the S-D13-amended rung.** Ten
> further corrections were needed and are ruled as **S-D14**; two of them are the class all three
> pre-build rulings were hunting (M4-c2's ruled sub pair fires for the wrong reason and dies in a
> `.expect` before the property it mutates is asserted; M4-d's ruled bound is a red that cannot fire,
> because `sort_by_stack` rotates the buffer it names). Neither was visible to reading — both were
> found by applying the mutation and watching the wrong thing happen. The full record is the
> **S4 · LANDED** section after the audit ledger: the landed set file by file, the RED ledger with
> what each mutation actually did, the golden's ten-colour accounting, and the measured
> 6.00 → 7.00 probes/node/frame.
>
> other twelve are instruments that cannot be written as spelled — a derivation with no formula, a
> `StackIndex` that `UiInstance` does not carry, a `const` assert that is nightly-only on 1.97.1, a
> corner-sampling claim that is false under the default sampler, an upper bound that cannot fire on
> the scene it is attached to — plus M4-b's margin, which **S-D12 itself made stale** by changing
> G4-3's border one row above it. Rows **20-33** of the ledger.

**Lands.**

1. ~~`UiNineSlice { border_px: [f32;4], mode: u8 /* Stretch|Tile */, fill_center: bool }` — 20 B,
   `#[repr(C)]` POD, padding spelled.~~ ~~**`UiNineSlice { border_px: [f32;4], mode: u8, fill_center: bool, _pad: [u8; 2] }` — 20 B**~~
   **`UiNineSlice { border_px: [f32;4], border_uv: [f32;4], mode: u8, fill_center: bool, _pad: [u8; 2] }` — 36 B**,
   `#[repr(C)]` POD, `_pad` SPELLED, `mode` a `NineSliceMode` with exactly ONE legal value at S4
   (`Stretch = 0`), pinned by ~~a variant-count `const` assert~~ **the one-variant `const` match
   `const _: () = match NineSliceMode::Stretch { NineSliceMode::Stretch => () };`** *(respecified
   2026-08-21 — S-D13 (4)(4). MEASURED on rustc 1.97.1: `std::mem::variant_count` is `E0658`
   AND "not yet stable as a const fn", two errors on one line, so the prescribed spelling does not
   exist on this toolchain. The match spelling was measured green, and measured RED — `error[E0004]:
   non-exhaustive patterns` — the moment a second variant is added. **No outer braces:** the braced
   form emits `unused_braces`, which `-D warnings` turns into an error.)*; an out-of-range
   discriminant is rejected at pack ~~on the enum~~ **on the raw `u8` `PackInput` carries** *(S-D13
   (4)(3): an out-of-range discriminant of a one-variant enum can only be produced by `transmute`,
   which is UB and cannot be a gate. `PackInput` carries `nine_slice: Option<UiNineSliceInput>` whose
   `mode` is a raw `u8` `debug_assert!`ed at the pack boundary — the `UiImageInput.slot` precedent,
   `pack.rs:31-33`, `:212-217` — while the AUTHORED component keeps the typed `NineSliceMode`, where
   the type system already forbids the value.)*. **Both `border_px` and `border_uv` are `[l, t, r, b]`** —
   `PackInput::border_width`'s
   order (`pack.rs:55-57`), NOT `corner_radius`'s `tl, tr, br, bl`. **`border_uv` is the SOURCE inset in
   fractions of the node's current UV sub-rect**, `Default` = equal thirds; its validity domain and the
   release behaviour when it (or `border_px`) would invert are ruled in S-D12 (2).
   **`border_px`'s `Default` is `[0.0; 4]`, and the picture that produces is ACCEPTED rather than
   guarded** *(added 2026-08-21 — S-D13 (4)(5): it was unstated while `border_uv`'s was ruled. Under
   S-D12 (1) presence suppresses the image, so `UiNineSlice::default()` on an imaged node does not
   degrade to S3's sprite — every corner and edge has zero destination extent and only the centre
   sub-quad is visible, so the node renders **the middle ninth of its texture, zoomed to fill**.
   S-D12 (2)'s validity domain passes it: `0 + 0 > rect.w` is false, no shrink fires. It is the same
   shape as `UiImage`'s alpha-0 default tint — the null value of an authored component, visible
   immediately — and it must be STATED in the field's doc comment, because an unstated degenerate
   default is the datum an author discovers by acting on it.)*
   **and `fill_center`'s `Default` is `true`, ruled rather than inherited from `bool::default()`**
   *(added 2026-08-21 at LANDING — S-D14 (1). The field had no stated `Default` while the field one
   line above had just been given one, and `bool::default()` is `false`, which FALSIFIES the picture
   the ruling above describes: with `border_px = [0;4]` AND `fill_center = false` a defaulted
   `UiNineSlice` on an imaged node emits its background plus eight ZERO-EXTENT slices and renders
   nothing at all, the image having been suppressed and the only region with extent being the one
   that was skipped. `UiNineSlice` cannot `#[derive(Default)]` anyway — `border_uv`'s default is
   thirds, not zeros — so the impl is hand-written and this field's value in it was an unmade
   choice. Every record count in this rung's gates (the emission 10, G4-2's 12→13, G4-6's 20 480,
   G4-8's 14, M4-f's 410) silently assumes `true`.)*. Table (authored, cold).
   *(amended 2026-08-21: (a) the original field list is 20 B — **verified by compiling both spellings
   under rustc 1.97.1: size 20 / align 4 with and without `_pad`** — but those two bytes were IMPLICIT
   TAIL PADDING, which is exactly what "padding spelled" forbids; S5's own `UiSheet` at the next rung
   spells its `_pad` with a reason, and this one now matches its own prose. (b) `Tile` moves to S5 —
   S-D11. **(c) amended AGAIN the same day by S-D12 (2): with no source inset the nine SOURCE sub-rects
   came from nothing, and the texel size that Unity and Godot use for this is a datum the engine never
   records — `BindlessTextureTable::register` takes a bare `VkImageView` (`bindless.rs:287`) and the
   table stores no dimension map. Re-measured at 36 B / align 4; the two trailing bytes are implicit
   tail padding at the wider spelling too, so `_pad` stays. ZERO GPU bytes — the split resolves at pack
   into each sub-quad's `uv`, and `UiInstance` does not move.**)*
2. ~~The pack emits **9** sub-quads (8 with `fill_center == false`) into the **existing**
   `UiRenderScratch.pack` when the component is present, and **1** when it is absent.~~
   **The pack emits the node's background rect at sub 0 (unchanged from S3) PLUS 9 sub-quads at subs
   1..=9 (8 when `fill_center == false` — the centre, sub 5, is the one that is skipped), ~~in BOTH pack
   loops~~ **in `gather_into_staging` ONLY** *(struck 2026-08-21 — S-D13 (2). `pack_sort_upload`, the
   other loop, **has no caller anywhere in this workspace**: its only non-doc caller is
   `host_upload_frame` (`upload.rs:526`), whose only non-doc occurrence in the whole `crates/` tree is
   its own definition at `upload.rs:509` — every other hit is a doc comment, two of them describing
   the already-DELETED `host_upload_frame_from_world`. The name is not re-exported from
   `boyko_render/src/lib.rs` or `src/ui/mod.rs`; it is public API with zero in-workspace callers. It
   is the surviving half of the path S0 replaced with the two-phase seam. Expanding it would add
   untested code AND manufacture a gate that cannot be run — both of this campaign's headline
   classes in one edit. What S4 lands instead is a **loop-agnostic emitter**, and its shape is the
   PER-SUB MAPPING rather than an append: `ui_node_sub_codes(input, &mut [u32; N]) -> usize` (the
   sole authority on which subs exist) plus `pack_ui_sub_record(input, sub, scale) -> UiInstance`
   (a pure function of `(input, sub)`), with `emit_ui_node_records(input, scale, &mut Vec<_>)` as a
   thin wrapper looping the subs for callers that append. `gather_into_staging` drives the first
   two directly and G4-4 calls the wrapper into a `UiRenderScratch` the test owns
   *(shape corrected 2026-08-21 at LANDING — S-D14 (4): "one free function appending a node's
   records into a caller-supplied sink, which `gather_into_staging` calls into `staging[dst..]`" is
   not writable as spelled. That loop neither appends nor iterates per node — it builds the whole
   key lane, sorts it, then writes ONE record per sorted key BY INDEX into a fixed
   `Box<[UiInstance]>`, recovering the source from the key alone (`upload.rs:337-374`). There is no
   per-node block to hand to a sink and a fixed slice cannot be appended to. The per-sub mapping is
   what that `staging[dst] = …` already IS, it is what M4-c2 must mutate, and it keeps M4-a writable
   at the key push.)*. Whether the dead loop
   should be DELETED is public-API scope and is filed in `docs/OPEN-QUESTIONS.md`.)* — and SUPPRESSES
   the node's sub-10 image record, because the slices ARE that image (S-D12 (1);
   the full four-row truth table is stated there). Subs 1..=9 are ROW-MAJOR: TL, T, TR, L, C, R, BL, B, BR.
   A node carrying `UiNineSlice` but NO `UiImage` emits its background and nothing else (S-D12 (3)).**
   All nine inherit the parent's `StackIndex` and `ComputedClip` verbatim and take
   **consecutive** `append` indices — so the existing `(stack, append)` total-order sort keeps them
   contiguous and in painter's order **with no change to the sort**.
   *(amended 2026-08-21, two corrections. (a) **Replace vs add was undetermined** — D8d says the
   sub-quads replace the background record, D4 lists the background as a distinct preceding element,
   and `tests/ui_s0_seam.rs:245` records a third number. S-D11 rules ADD, with the reason: a nine-slice
   source is a FRAME with transparent regions, so the background rect is the surface it sits on, not
   redundant overdraw. `UI_RECORDS_PER_NODE = 11`. (b) **"the existing `UiRenderScratch.pack`" names
   the LEGACY loop only.** `UiRenderScratch` is reached solely through `pack_sort_upload`
   (`upload.rs:452-477`, documented at `:140-142` as the host/golden driver); the IN-SCHEDULE path the
   scheduler runs is `gather_into_staging`, which packs into `UiUploadSystem::staging` and never names
   `UiRenderScratch`. This rung's own S3 retrospective at §"Reconciliations" knows there are two loops
   with two append encodings; the item named one. ~~The expansion lands in BOTH.~~ **The expansion
   lands in `gather_into_staging` only — S-D13 (2): the loop this sub-amendment was written to
   include turned out to have no caller in the workspace, so "both" would have expanded dead code
   and specified a gate that cannot be run.** *(amended again 2026-08-21.)*)*
3. ~~`Tile` mode folds the tile count into `uv` at pack (UVs run past 1.0 and `REPEAT` wraps), with
   S-D7's `debug_assert!` + release clamp when a sheet is also present, and a diagnostic counter for
   the clamp.~~ **MOVED TO S5 in its entirety (S-D11).**
   *(amended 2026-08-21 — the mechanism does not exist and its guard has no subject. **`REPEAT` is not
   on the UI's sampling path**: S3 landed the UI's own sampler as `AddressMode::ClampToEdge` in BOTH
   modes (`resources.rs:310`) and the fragment shader samples the bindless texture through it
   (`ui_rect.fs.hlsl:198`) — the comment at `resources.rs:307-309` names this rung by name and says it
   "does not get to set the default". As written, a `Tile` edge would render one clamped streak. And
   the guard's subject, `UiSpriteSheet`, has **zero occurrences in any `.rs` file in the tree** — S5's
   Lands item 2 creates it, so at S4 the `debug_assert!` cannot be written, the clamp guards an
   unconstructible combination, and the counter can only ever read zero. S-D11 replaces the mechanism
   with `frac` inside the sub-rect, which is well-defined under a sheet and therefore needs no guard,
   no clamp and no counter — and, separately, removes the need to thread a `&mut u64` through
   `pack_ui_instance`/`pack_ui_image_instance`, which are free functions with no receiver
   (`pack.rs:86`, `:204`) and had nowhere to put a counter.)*
4. **D4's emission contract, pinned** — ~~*background rect → nine-slice sub-quads (TL..BR) → image →
   glyphs → focus ring*, per node~~ ~~**at S4, over the terms S4 emits: *background rect → nine-slice
   sub-quads (TL..BR, centre at sub 5) → image*, per node.**~~ **at S4, over the terms S4 emits:
   *background rect → EITHER the nine-slice sub-quads (row-major TL..BR, centre at sub 5) OR the
   image*, per node** *(amended again 2026-08-21 — S-D12 (1): the two terms are ALTERNATIVES, not a
   sequence. D4's ORDER is untouched; the image term is simply absent when slicing is on, the way a
   rect-only node has no image term.)*. The two remaining terms of D4's full
   contract keep their home in D4 and are pinned at the rungs that emit them: **glyphs are not
   per-node sub-records at all** — the canonical gather hard-codes `text_uv: None` (`gather.rs:389`
   — re-measured by CONTENT 2026-08-28; the four citations in this document all read `:272`, which is
   `scratch: &mut UiGatherScratch,` in the pre-A1 tree and a borrowck comment in today's. `grep -n
   'text_uv' crates/boyko_render/src/ui/gather.rs` returns exactly one line, `389`)
   and every glyph in the tree is a separate `UiNode` row a host appends (D4's own preamble concedes
   their order "is decided purely by the order the host appends them"), and `pack.rs:208`'s
   `debug_assert!` forbids one record being both glyph and sprite — while the **focus ring is
   Interaction's I9** (`UI-PLAN-INTERACTION.md:884`, opt-in `FocusRing`; **zero occurrences in
   `crates/`**), and §6 of this plan already assigns it there.
   *(amended 2026-08-21: as written, two of the five terms could not be emitted, so G4-2's subject was
   unconstructible and M4-c was unwritable.)*
5. Layout is untouched — slicing is purely visual. *(verified 2026-08-21 and it is TRUE and structural,
   not a promise: `boyko_ui` takes **no render dependency** (`boyko_ui/Cargo.toml`, whose own comment
   states it) and never names `UiInstance` or the pack, so nothing in layout can read a record count.)*
6. **`ui_pack_inputs!` gains `UiNineSlice`** *(added 2026-08-21 — the omission that would have made the
   whole rung invisible)*. The single component list at `gather.rs:72-82` drives BOTH the gather's
   per-node read tuple AND `ui_render_discovery`'s `Changed<..>` filter; the macro's own doc says
   "sprites add the rest at **S4**–S5", and §6 names it as the surface neither sibling plan may bypass.
   Without this edit the gather cannot read the component at all, and an author's runtime edit to a
   nine-slice would never bump `UiRenderGeneration` — the frame would not repaint. This is S3 defect 4
   repeating one rung later, and it was in no Lands list: S5's item 6 wires *its* three components,
   never this one. The derived probe census (`ui_pack_inputs!(count)`) moves 6 → 7 per node per frame.
7. **`gather_into_staging`'s sub-record decode becomes a full match, and its key-push loop a loop**
   *(added 2026-08-21 — "S4's nine-slice raises `UI_RECORDS_PER_NODE` and nothing else changes at that
   call site" is FALSE, and the same false claim sits in the tree at `pack.rs:183-184` and must be
   corrected in the same edit)*. Two concrete blockers at `upload.rs:337-373`: the key push is
   hard-coded to at most two sub-records (`base`, then conditionally `base + 1`), and the pack dispatch
   is BINARY — `if append.is_multiple_of(UI_RECORDS_PER_NODE) { pack_ui_instance(..) } else {
   pack_ui_image_instance(..).expect("invariant: a sub-record key is emitted only for a node carrying
   UiImage") }`. With the stride raised, every sub-quad key falls into the `else` arm, and on a
   nine-sliced node **without** `UiImage` that `.expect` **panics in release as well as debug**
   (`pack_ui_image_instance` opens `let image = input.image?;`, `pack.rs:205`). The decode must become
   `match append % UI_RECORDS_PER_NODE` over {background, sub-quad 0..=8, image}.
   **The rule that makes every arm of that match total (S-D12 (3)): the KEY PUSH is the sole authority
   on which subs exist.** It pushes sub 0 always, subs 1..=9 only when `UiNineSlice` AND `UiImage` are
   both present (sub 5 additionally gated on `fill_center`), and sub 10 only when `UiImage` is present
   and `UiNineSlice` is ABSENT. Every decode arm's precondition is then established at the push thirty
   lines above, and **no `.expect` in that loop is reachable for any of the four component
   combinations** — which is what item 7 was actually for, and what the gate table had no row to check
   until G4-8. *(added 2026-08-21 by S-D12 (3): item 7 as written fixed the panic by widening the
   match, leaving the push free to emit a key whose arm still had to cope. The push is the cheaper and
   the checkable place.)*
   **The sub space gets NAMED CONSTANTS in this same edit, and the stride is derived from them**
   *(added 2026-08-21 — S-D13 (3))*:
   `UI_NINE_SLICE_REGIONS: u32 = 9` (the sub space `1..=9`, row-major TL..BR),
   `UI_NINE_SLICE_SUB_BASE: u32 = 1`, `UI_IMAGE_SUB: u32 = 10`,
   `UI_RECORDS_PER_NODE = UI_IMAGE_SUB + 1`, **`UI_NINE_SLICE_CENTER_SUB = UI_NINE_SLICE_SUB_BASE + 4`**
   and **`UI_NINE_SLICE_MODE_COUNT: u8 = 1`** *(the last two added 2026-08-21 at LANDING — S-D14 (2).
   `UI_NINE_SLICE_MODE_COUNT` is the bound S-D13 (4)(3) prescribed the `mode` `debug_assert!` to
   compare against, and it occurred exactly ONCE in the whole plan — in that ruling — while no Lands
   item created it: G4-5's discriminant half named an instrument the rung did not mint. It cannot be
   derived (`variant_count` is nightly-only here and the enum lives in the crate this module is
   deliberately type-free of), so it is BOUND to `NineSliceMode` by the exhaustive conversion match
   in the gather — the one site that narrows the authored enum to the raw byte — which is
   `error[E0004]` when S5 adds `Tile` and walks the author to this constant. The centre's sub code
   was likewise spelled `base + 4` in prose and nowhere as a name.)*
   Also minted: **`UI_MAX_SUBS_PER_NODE = 1 + UI_NINE_SLICE_REGIONS`**, the size of the sub-code
   scratch the push fills — the EMISSION maximum, deliberately not the stride, so the hole in the
   sub space stays visible at the one place a buffer is sized by it. Reason: S-D12 (1) **severed the stride (11) from the
   emission** (10 / 9 / 2 / 1), and G4-1's instruction "derive the count from `UI_RECORDS_PER_NODE`
   and `fill_center`, never a literal" has **no formula** afterwards — no expression in
   (stride, `fill_center`) yields 2 for the imaged row, and `UI_RECORDS_PER_NODE - 1` gives 10 only
   by the accident that exactly one of {centre, image} is dropped. `UI_RECORDS_PER_NODE`
   (`pack.rs:185`) is the only region constant in the tree, and G4-8 one row into the gate table
   already breaks the instruction with four literals. Deriving the stride from the largest sub code
   also pins the one relation the hole in the sub space made non-obvious. **The rule every gate then
   satisfies:** a literal is allowed for the *component combination* a gate constructs (the truth
   table is authored data); every *record count* is an expression over these constants and
   `fill_center`.
8. **`UI_STAGING_ROWS` is re-derived from a stated node budget, and its doc comment is corrected**
   *(added 2026-08-21)*. The in-schedule path packs into a FIXED `Box<[UiInstance]>` of
   `UI_STAGING_ROWS = 4096` (`upload.rs:107`, `:571`) whose overflow arm is `debug_assert!(false, …)` —
   a debug **panic** — then a release `truncate` of the emission TAIL with `staging_overflows` bumped
   (`upload.rs:346-359`). Its doc claims "2× the plan's own N = 2048 measurement scene": **that was
   already false at S3** (2 records/node × 2048 nodes = 4096 = exactly 1× — it overflows at node 2 049,
   with zero margin), and ~~at stride 11 the box overflows at 187 nine-sliced nodes~~ **it overflows at
   the 410th nine-sliced imaged node** *(corrected 2026-08-21 — S-D12 (4); 187 was the NODE budget
   divided by the stride where the ROW budget was called for, and a nine-sliced node emits 10 records
   rather than 11 under S-D12 (1))*. Replace with
   `UI_MAX_NODES: usize = 2048; UI_STAGING_ROWS: usize = UI_MAX_NODES * UI_RECORDS_PER_NODE as usize`
   — 22 528 rows × 80 B = **1.72 MiB**, one host allocation at `initialize`, never grown, never walked
   beyond the live prefix. **Reason for paying it rather than sizing for a "typical" mix:** a box sized
   for a typical composition overflows as a function of *what the scene contains*, which is precisely
   the composition-dependent silent truncation the clamp exists to make loud. A constant that cannot
   overflow within the stated node budget is worth 1.4 MiB of host RAM. *(The GPU ring needs no change:
   `UiRingSlot` is grow-only pow2 on overflow — `resources.rs:190-203` — so the CPU box is the sole
   hard cap.)* **The derivation stays on the STRIDE (11) even though S-D12 (1) puts the true worst case
   at 10 records/node (20 480 rows, 1.56 MiB): the 160 KiB of slack buys a constant that cannot go
   stale when a later rung adds a sub code, where a constant derived from today's maximum emission
   would have to be re-audited every time S-D12 (1)'s truth table gains a row.**

**Gate.**

*(the whole table was re-pointed 2026-08-21 — three of the five rows named a subject that cannot be
constructed at S4; each row below that DRIVES a pack loop states **which one**, because the rung had
two and S3's ledger already recorded that naming one leaves the other ungated. **Amended at LANDING
— S-D14 (3): the original clause said every row states which pack loop it drives, and by the time the
table reached eight rows THREE of them drove none — G4-3 was the one S-D13 (5)(3) noticed and
re-pointed, but G4-5 is a `const` match plus a CPU test at the pack BOUNDARY (`pack_ui_*`, not a
loop) and G4-7 drives `ui_render_discovery` and the probe census. The preamble over-claimed and
S-D13's "ONE row" census was stale before it was written.)*

| # | Claim | How |
|---|---|---|
| **G4-1** | The expansion is 9 (or 8) sub-quads **in addition to** the background rect, consecutive, inheriting *(S-D11 — "in addition to" is the ruled reading of a contradiction, not a restatement)* | ~~CPU unit test, no GPU: assert record count, that `append` is `k..k+9`~~ **CPU unit test, no GPU, run against ~~BOTH loops:~~ `gather_into_staging` via `sys.staged()` on a bare `EcsMaster` (the device-free precedent is `ui_s0_seam.rs:251`, whose own doc anticipates this rung)~~, and for `pack_sort_upload` via `UiRenderScratch`~~** *(second leg DELETED 2026-08-21 — S-D13 (2): `pack_sort_upload` has **no caller anywhere in this workspace** — its only non-doc caller is `host_upload_frame`, whose only non-doc occurrence in `crates/` is its own definition at `upload.rs:509` — and it takes `&mut RhiContext` and ends in `ctx.ui_upload(..)`, so it cannot be driven device-free either. A leg on a loop nothing calls is a gate that cannot be run over code nothing exercises.)*. **Assert the record count DERIVED from ~~`UI_RECORDS_PER_NODE` and `fill_center` — never a literal —~~ the minted sub-space constants (`UI_NINE_SLICE_REGIONS`, `UI_NINE_SLICE_SUB_BASE`, `UI_IMAGE_SUB`) and `fill_center` — never a literal for a COUNT, though the component combination a case constructs is authored data and may be written out** *(amended 2026-08-21 — S-D13 (3): as spelled the instruction had no formula. S-D12 (1) severed the stride from the emission, and no expression in (`UI_RECORDS_PER_NODE`, `fill_center`) yields 2 for the imaged row; G4-8 one row below already broke the instruction with four literals. Lands item 7 mints the constants the derivation needs and derives the stride from the largest sub code.)* ~~that the sub-quads occupy consecutive `append` codes `base+1..=base+9` (`base+5` absent iff `fill_center == false`)~~ that the sub-quads arrive CONSECUTIVE AND IN CONTRACT ORDER in `sys.staged()` — the SORTED output, identified by record kind and by each slice's own `uv`/`min_px`, never by an `append` code**, that all nine carry the parent's ~~`StackIndex` and~~ **clip — and the parent's stack observed as a CONSEQUENCE, by bracketing the nine-sliced node between a lower-stack and a higher-stack plain node (`ui_s0_seam.rs:255-281` is the shape) so a slice that lost its stack lands outside the block** *(amended 2026-08-21 — S-D13 (4)(1): `UiInstance` does not carry a stack. Its complete field set is `min_px, size_px, clip, corner_radius, uv, color, border_color, border_width, flags` (`upload.rs:110-120`); `stack` lives on `UiNode` (`:99-101`), is pushed into the PRIVATE key lane and consumed as the sort key only. This is the same shape S-D12 struck from this very row for the `append` codes, one clause later in the same sentence — the amendment fixed the codes and left the stack.)*. *(amended 2026-08-21 — S-D12; the struck clause is wrong on one loop and unobservable on the other. **`pack_sort_upload`'s `append` is the RUNNING RECORD INDEX, not the `(node, sub)` code** — `upload.rs:449-455` says so in the source and the loop at `:458-467` uses `scratch.pack.len()` — so `base+1..=base+9` is simply not that loop's encoding and the stride there is the emitted count, not `UI_RECORDS_PER_NODE`. On `gather_into_staging`, which DOES use the code, the key lane `UiUploadSystem.keys` is PRIVATE (`upload.rs:160`; the public surface is `staged()`/`probes()`/`repacks()`/`staging_overflows()`, `:266-290`), so the codes are unobservable. **Asserting the consequence is not a workaround, it is the stronger gate:** M4-a's real failure is record duplication and loss (the ledger's own row 8 — `append` is the SOURCE ADDRESS), and a test that read the key lane before the sort would go green on exactly that. No accessor is added — widening a production type's surface would have bought the weaker assertion.)* |
| **G4-2** | The emission order is D4's | ~~A node with background + nine-slice + image + glyphs + focus ring~~ ~~**A node with background + nine-slice + image**~~ **TWO nodes, because S-D12 (1) made the last two terms alternatives: one with background + image (subs 0, 10) and one with background + nine-slice + image (subs 0, 1..=9, and NO sub 10 — the gate asserts the image record's ABSENCE, which is the whole of ruling (1) and is ~~otherwise pinned nowhere~~ **also pinned by G4-8's derived total, and until S-D13 was mutated by NOTHING** *(corrected 2026-08-21 — S-D13 (1): "pinned nowhere" and "asserted twice, mutated never" are opposite diagnoses and the wrong one was written down. `1 + 2 + 1 + 10 = 14` becomes 15 with one extra record, so G4-8 sees it too; what was missing was a RED, which M4-c1 now is.)*)** *(amended 2026-08-21 — S-D12 (1))* *(amended 2026-08-21: glyphs are not per-node sub-records — `gather.rs:389` hard-codes `text_uv: None` and `pack.rs:208` forbids one record being both — and `FocusRing` has zero occurrences in `crates/`; it is Interaction's I9. As written this gate's subject was unconstructible)*: **drives `gather_into_staging`**; assert the ~~`append` lane's~~ **STAGED** order equals the contract, **by name**, off `sys.staged()` — the shape `ui_s0_seam.rs:288-302` already uses to assert staged records by `FLAG_TEXTURED` *(two corrections 2026-08-21 — S-D13 (5)(2) and (1). **(a)** "the `append` lane's order" is the noun S-D12 struck from G4-1 one row earlier as unobservable (`UiUploadSystem.keys` is private, `upload.rs:160`); the instrument named here is right, only the wording propagated the wrong claim — and it propagated it into §6 and thence into the Interaction plan. **(b) The cited shape is BLIND to the property this row is cited for.** On a nine-sliced imaged node index 0 is the untextured background and 1..=9 are nine textured slices, so a wrongly-emitted sub 10 is a TENTH TEXTURED record and a `FLAG_TEXTURED` prefix assertion is identical with and without it — only LENGTH or GEOMETRY separates them. That shape also hard-codes `assert_eq!(staged.len(), 4, …)` (`ui_s0_seam.rs:289-293`), the literal G4-1 forbids one row above. **G4-2 borrows the shape's stack-bracketing and its by-name reading, identifies each slice by its own `uv`/`min_px` as G4-1 does, and takes its LENGTH from the same derivation G4-1 and G4-8 use — never from that file's literal.**)* |
| **G4-3** | Slicing preserves corners | GPU golden `ui_nine_slice`, **driving `gather_into_staging` and uploading `sys.staged()`** *(added 2026-08-21 — S-D13 (5)(3): this was the ONE row in a table whose preamble requires every row to name its pack loop that named none. The only nine-slice-shaped golden precedent hand-packs its `UiInstance`s with no node and no gather — `ui_sprite_gpu_golden.rs:171-176` — which is the construction S-D12 (1) rejects at `:470-473` as self-gating. The picture this row pins must be the one the scheduler's own loop produced.)*, **at `UiSamplerMode::Pixel`** *(added 2026-08-21 — S-D13 (4)(2): the row stated no mode, and its own assertion "samples only its own source cell" is FALSE under the default. `Smooth` is `Filter::Linear` and is `#[default]` (`resources.rs:101-119`), and the existing sprite golden runs Smooth as its primary leg (`ui_sprite_gpu_golden.rs:442-444`) with Pixel only as a reachability leg (`:473-474`). Magnifying a 3-texel axis 32× under Linear blends into the neighbouring cell past each cell's texel centre, so the sentence is false for every pixel outside the inner half of a cell. `Pixel` makes it true as written and keeps the golden's meaning — "this region came from that cell" — independent of a filter kernel.)*: a 3×3 procedural source (S-D5) ~~stretched to a 64×16 rect~~ **whose nine cells carry NINE DISTINCT values, stretched to a 96×96 rect at `border_px = [16,16,16,16]`** *(amended 2026-08-21, two reasons. (a) A **symmetric** source makes region assignment unobservable: the natural corner=A/edge=B/centre=C source is invariant under the full dihedral group, so all 24 corner permutations hash identically — and the existing S-D5 checkerboard is itself invariant under 180° rotation and transpose (`ui_sprite_gpu_golden.rs:117-124`). Nine distinct values make every region individually visible, which is what M4-e needs. (b) At 64×16 from a 3×3 source, a correct corner is **one destination pixel**, so "matches the source's corners 1:1" degenerates to a single-texel assertion that is exact only because a 1-px quad's centre lands at u = 1/6 — any half-texel convention error blends instead of failing. 96×96 at 16 px borders gives every corner real width.)*; ~~the four corner regions are **unstretched** and the edges are~~ **each corner region is exactly `border_px` in size (NOT a fraction of the rect) and samples only its own source cell, while the edges and centre stretch; the sub-10 image record is ABSENT (S-D12 (1)) — the gate now has a picture in which slicing is what is on screen rather than what is underneath it**; image hash pinned. **`UiImage.tint = 0xFF_FF_FF_FF`, stated because the DEFAULT disarms both of this row's reds** *(added 2026-08-21 at LANDING — S-D14 (6): `UiImage::default()` is `tint: 0`, documented "FULLY TRANSPARENT tint (alpha 0) — an invisible node" (`components.rs:454-466`); the pack premultiplies it into every slice's `color` and the pipeline blends `PREMULTIPLIED_ALPHA`, so an alpha-0 slice contributes NOTHING to the hash. M4-b's whole 2 560 px margin and M4-e's entire premise would then be zero, on the rung's only device-bound row. An unstated parameter whose default disarms a red is this campaign's own headline defect, so it is written down rather than left to the builder's care.)* **A SKIP IS NOT A PASS: the row's harness returns early and exits 0 on a GPU-less or validation-less box (the `boot_*_or_skip` false-green the project's own CLAUDE.md documents), so the landed file honours `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1`, under which a skip FAILS** *(added 2026-08-21 at LANDING — S-D14 (7): this is the only device-bound row in the rung and it carries three of the reds; "G4-3 ran green" could not otherwise be distinguished from "G4-3 never compared the picture".)* **`border_px = [16, 24, 16, 24]`, deliberately ASYMMETRIC** *(amended again 2026-08-21 — S-D12 (2). `[16,16,16,16]` makes the SIDE ORDER unobservable: `[l,t,r,b]` and `[t,l,b,r]` hash identically, which is the amendment's own reason (a) — a symmetry that hides an assignment — one axis over from where it caught it. 16 and 24 are both far above the 1-px degeneracy reason (b) warns about, and the centre stays positive at 64×48. The SOURCE stays 3×3 with nine distinct values, and `border_uv` takes its `Default` of equal thirds, so the golden authors no new field.)* |
| **G4-4** | The pack still never reallocates | ~~`ui_no_realloc.rs` extended: the 9× expansion at N=1024 nodes must not grow the scratch after the first frame~~ **`ui_no_realloc.rs` extended at its own `N = 4096`** *(amended 2026-08-21: there is no N=1024 configuration in that file — it runs `N = 4096` at `:102` and `:191` and `WARM_N = 2048` / `STEADY_N = 16` at `:148-149`; the gate named a scene that does not exist)*, **driving the expansion through the production emitter rather than the test's own hand-rolled loop** *(its `build_frame` at `:83-93` calls `pack_ui_instance` directly and pushes keys by hand, so "extending it" would re-implement the expansion policy inside the test and gate the test against itself — S4 exposes the expansion as a callable seam and G4-4 calls it)*. The steady-state half of this file is sound as it stands and needs no work: a 3-frame warm-up whose allocations are excluded (`:108-110`), capacities captured (`:112-114`) and asserted byte-stable in an armed window (`:118-134`). **The emitter it calls is the loop-agnostic one Lands item 2 lands, NOT `pack_sort_upload`** *(clarified 2026-08-21 — S-D13 (2): after that ruling no production path fills a `UiRenderScratch` with nine-slice records, so "the production emitter" means the free function `gather_into_staging` also calls, appending into a caller-supplied sink. `ui_no_realloc.rs` already owns its own scratch — `UiRenderScratch::default()` allocates nothing, `pack.rs:278-287` — so this costs the file nothing.)*. **M4-d's upper bound lives on this file's EXISTING rect-only `N = 4096` frame (`ui_render_scratch_does_not_realloc_in_steady_state`, `:100-140`), not on the nine-sliced frame — and it is written on the PAIR of rotating buffers, `max(scratch.pack.capacity(), gather.capacity()) < 2 * emitted`, never on `scratch.pack` alone** *(the spelling corrected 2026-08-21 at LANDING, by MEASUREMENT — S-D14 (9). `assert!(scratch.pack.capacity() < 2 * emitted)` as prescribed is **a red that cannot fire**: `UiRenderScratch::sort_by_stack` ends in `core::mem::swap(&mut self.pack, gather)` (`pack.rs:317`), so the two buffers ROTATE every frame and a reserve made once at `UiRenderScratch::default()` sits in `scratch.pack` on even frames and in the caller's `gather` on odd ones. M4-d was applied, the mutation compiled, the frame ran — and the assert read `pack 4 096` with the **22 528-row reserve parked in `gather`**, green. The reserve's magnitude is also stated here rather than left to the reader: the natural setup-time reserve is `UI_MAX_NODES * UI_RECORDS_PER_NODE` = 22 528, the quantity `UI_STAGING_ROWS` is derived from — the ruling's "an 11× reserve is 4 096 × 11" computed 11× of `ui_no_realloc.rs`'s N, which `UiRenderScratch::default()` cannot see, since it takes no N. Either magnitude exceeds `2 × 4 096`, so the number was wrong and the mutation was not; the SPELLING of the bound was.)* *(ruled 2026-08-21 — S-D13 (4)(6): on the nine-sliced frame it cannot fire. `emitted = 4096 × 10 = 40 960`, `2 × emitted = 81 920`, and an 11× reserve is `4 096 × 11 = 45 056 < 81 920`, so the assert PASSES. Structural rather than a coincidence of this N: a reserve of 11/node can never exceed 2× an emission of 10/node. On the rect-only frame `emitted = 4 096`, `2 × emitted = 8 192`, and the same reserve overshoots 5.5×. The nine-sliced frame carries the count and consecutiveness half; it does not carry this line.)* |
| **G4-5** | ~~S-D7 is enforced~~ **`mode` has exactly one legal value at S4** | ~~`Tile` + `UiSpriteSheet`: `debug_assert!` fires in dev; the release build clamps to `Stretch` and the counter increments~~ ~~**A `const` assert on the variant count plus a CPU test that an out-of-range `mode` discriminant is rejected at pack.**~~ **The one-variant `const` match `const _: () = match NineSliceMode::Stretch { NineSliceMode::Stretch => () };` (no outer braces), plus a CPU test that an out-of-range `mode` value in `PackInput`'s raw `u8` is rejected at pack** *(respecified 2026-08-21 — S-D13 (4)(3)+(4). **Both halves were unwritable as spelled.** (a) MEASURED on rustc 1.97.1: `std::mem::variant_count` gives `E0658` (nightly-only, issue #73662) AND "not yet stable as a const fn" — the prescribed assert does not exist on this toolchain. The match spelling was measured green and measured RED (`error[E0004]: non-exhaustive patterns: NineSliceMode::Tile not covered`) the moment a second variant is added; the BRACED form additionally emits `unused_braces`, an error under the project's `-D warnings` gate. (b) An out-of-range discriminant of a ONE-VARIANT enum can only be produced by `transmute` — instant UB, which cannot be a gate. `PackInput` therefore carries the mode as a raw `u8` `debug_assert!`ed at the pack boundary, the exact `UiImageInput.slot` precedent (`pack.rs:31-33`, `:212-217`), while the AUTHORED component keeps the typed enum where the type system already forbids the value. **Note this row is still named by NO red mutation** (S-D13 (1)'s mapping): the `const` match's red is the `Tile` variant S5 adds, which is a compile-time red belonging to S5's arrival rather than an S4 mutation, and the discriminant half is reddened by feeding the pack an out-of-range `u8` directly.)* *(amended 2026-08-21 — **this was a gate that could not fail.** Its subject `UiSpriteSheet` has **zero occurrences in any `.rs` file in the tree**; S5's Lands item 2 creates it. A gate whose subject a later rung introduces cannot be written, therefore cannot fail — the exact class S3's M3-e already exhibited, caught here before the code rather than after. S-D7 is retired (S-D11) and the tiling half moves to S5; what remains at S4 is the half that HAS a subject: `mode` is a one-variant enum and the rung says so mechanically, so S5 widens the value set instead of re-specifying the field.)* |
| **G4-6** | The staging box holds the stated node budget | *(added 2026-08-21 — nothing in the original table looked at the box that actually truncates.)* Drive `gather_into_staging` with `UI_MAX_NODES` nine-sliced, imaged nodes: `sys.staged()` equals the derived emission count, `staging_overflows == 0`, and no `debug_assert!` fires. The original G4-4 could not see this: it drives `UiRenderScratch`, a growable `Vec`, while production packs into a fixed `Box` that **clamps** rather than grows |
| **G4-7** | `UiNineSlice` reaches the renderer at all | *(added 2026-08-21 with Lands item 6.)* The `ui_s0_discovery` shape: mutate `UiNineSlice` on a live node and assert `UiRenderGeneration` bumps **exactly once**; assert the derived probe census is `ui_pack_inputs!(count) + 1` per node per frame. Without the macro edit the component is invisible to both halves and the frame silently does not repaint. **The instrument this row points at is the `PackInput` ENUM at `ui_s0_discovery.rs:183-227`, not a `usize`** *(clarified 2026-08-21: that file's `mutate_pack_input` used to take a `usize` and end in `_ =>`, so a sixth input fell into the catch-all, re-inserted `UiImage`, bumped the generation and was reported as covered — a gate measured green over an unwired input. Commit `50a724ac` made it an exhaustiveness-checked enum, so adding `UiNineSlice` to `ui_pack_inputs!` WITHOUT giving it a variant and a mutation arm ~~**does not compile**~~ **is caught — but by WHICH of the two mechanisms depends on which half of the edit is made, and the row must say so** *(made precise 2026-08-21 — S-D13 (5)(4). Two edits, two outcomes, both structural in the file as it stands. **Adding to the macro AND to the test's `PackInput` enum + `ALL`, without match arms** → `error[E0004]` **twice**, because that enum has two exhaustive matches (`name()` and `mutate_pack_input`, `ui_s0_discovery.rs:195-227` onward) and neither carries a catch-all — this is the compile-time half, and it protects against "declared but never driven". **Adding to the macro ONLY** → compiles, and reds at RUNTIME on `assert_eq!(PackInput::ALL.len(), ui_pack_inputs!(count), …)` (`:265-274`), whose message names the three places to add it — this protects against "added to the macro but not to this test". The property is gated either way; the original sentence named one mechanism for both edits, which is precision lost, not a hole.)*. The repair is already in the tree; this row names the enum so the property is not re-lost.)* **This row is named by no red mutation** (S-D13 (1)): its red is the macro edit itself being omitted, which the two mechanisms above catch at build time rather than at gate time |
| **G4-8** | No component combination can panic the decode, and `UiNineSlice` alone is a no-op | *(added 2026-08-21 — S-D12 (3). Lands item 7 fixes a `.expect` that **panics in release**, and no row in this table constructed the node that panics: a gate for a crash nobody drives.)* CPU unit test on `gather_into_staging`: drive all FOUR rows of S-D12 (1)'s truth table in one world — bare node, imaged, nine-sliced-only, nine-sliced+imaged — and assert `sys.staged().len()` equals the DERIVED total ~~(1 + 2 + 1 + 10)~~ **`1 + 2 + 1 + (1 + UI_NINE_SLICE_REGIONS)` = 14** with no panic in either build profile *(respelled 2026-08-21 — S-D13 (3): "(1 + 2 + 1 + 10)" is four literals, and it sat one row below G4-1's "never a literal" instruction — the instruction that had no formula to offer. With the sub-space constants minted in Lands item 7 the total moves by itself when a sub code is added.)*. The nine-sliced-only node contributes **exactly one** record, its background. **"Either profile" is TWO invocations and the row names both** — `cargo test -p boyko-render --test ui_s4_nine_slice` (`running 6 tests`) and the same with `--release` (`running 5 tests`; G4-5's `should_panic` half is `#[cfg(debug_assertions)]` because it gates a `debug_assert!`, so its absence there is the expected count and not a vacuous filter) *(added 2026-08-21 at LANDING — S-D14 (8): the rung ladder's unconditional gate is dev-profile only, and the debug run is the strictly WEAKER leg — it additionally has `debug_assert!` armed, so passing it says nothing about the `.expect`s the release build keeps, which is the very thing Lands item 7 exists to make unreachable.)*. **This row is the second pin on sub 10's absence** — the extra record M4-c1 emits takes the total to 15 — which is why `:1295`'s "otherwise pinned nowhere" is corrected there |

**Red mutations.**

* **M4-a — give all nine sub-quads the same `append` index.** G4-1 reds. ~~and the golden's paint order
  becomes order-of-iteration. *Proves the "the key is TOTAL because `append` is unique" claim is
  load-bearing rather than a description — an unstable sort over a non-total key is a real
  nondeterminism.*~~ ***The mutation FIRES; its stated rationale is wrong on BOTH halves, and both were
  checked rather than assumed (2026-08-21).*** *(1) `append` is not a tie-break — it is the record's
  **SOURCE ADDRESS** in both loops. `sort_by_stack` gathers `self.pack[idx]` by it (`pack.rs:315`), so
  nine equal keys emit `pack[first]` NINE TIMES and **drop the other eight records**;
  `gather_into_staging` decodes the source as `node_buf[append / UI_RECORDS_PER_NODE]` and the sub-kind
  as `append % …` (`upload.rs:366-373`), so nine equal keys all resolve to the same node and the same
  sub. The observed failure is **record duplication and loss — a wrong picture, not a shuffled one**.
  (2) There is no nondeterminism to prove: `sort_unstable_by_key` is a deterministic pure function.
  **MEASURED under rustc 1.97.1** over true tie blocks of nine identical keys at n = 9 / 27 / 72 / 576 /
  1800 — the within-tie order IS permuted (at n ≥ 72 the block comes back scrambled), but **identically
  on every repeat and across fresh processes**. A golden blessed after this mutation would stay green
  run after run. What the codebase's comments actually claim (`pack.rs:257-259`, `upload.rs:316-318`)
  is "unstable result == stable result" — an equality of orderings — and that is the property this
  mutation should be said to prove.*
* **M4-b — expand the corners proportionally instead of at fixed `border_px`.** G4-3 reds. *Proves the
  golden tests **slicing** rather than "a textured rect appeared" — the mutation that a texel-only
  assertion would survive.* *(margin confirmed 2026-08-21, and it is large, not marginal: at the
  amended 96×96 destination with ~~16 px borders a correct corner is 16×16 px~~ **`border_px = [16, 24, 16, 24]`
  a correct corner is 16 × 24 = 384 px** and a proportional one is
  32×32, so four corners move ~~~3 000~~ **2 560** of 9 216 px *(1 536 correct vs 4 096 equal-thirds)* — far above any 8-bit hash threshold, unlike the ~1-ULP
  shader edits an 8-bit golden genuinely cannot see.)* ***RECOMPUTED 2026-08-21 — S-D13 (5)(1). The
  struck numbers were computed from `[16,16,16,16]`, the border S-D12 (2) replaced with
  `[16,24,16,24]` ONE ROW ABOVE THIS ONE in the same ruling: the repair changed the number and left
  every site that computed from it — this bullet and, inside S-D12 (1) itself, `:457` and `:465`.
  That is the doc-rot-repair class this plan warns about, committed by the repair. The red is
  unaffected; only the margin was wrong, and it is still an order of magnitude above any hash
  threshold.*** **That margin was computed against the SLICES, and
  until S-D12 (1) the slices were not what the golden saw: the sub-10 image covered all 9 216 px on top
  of them, so the moved pixels moved under an opaque sprite and the hash did not shift by one byte.
  Suppressing sub 10 is what makes this mutation a red rather than a description of one** *(2026-08-21)*.
* **M4-c — ~~swap the image and glyph emission order~~ ~~swap the image and the LAST sub-quad (BR)
  emission order~~ SPLIT INTO M4-c1 AND M4-c2 (below).** *(respecified 2026-08-21: **the original mutation could not be applied
  at all.** There is no site that emits a glyph and an image into one per-node lane to swap — the
  canonical gather hard-codes `text_uv: None` (`gather.rs:389`), and `pack.rs:208`'s `debug_assert!`
  forbids one record being both. What would have been "observed" is that the mutation is unwritable,
  which is not a red. The swap of image against the last sub-quad is writable, fires the same gate, and
  tests the same property.)*
  ***STRUCK AGAIN 2026-08-21 — S-D13 (1). THE RESPECIFIED MUTATION IS UNWRITABLE FOR THE SAME REASON
  THE FIRST ONE WAS, and by the hand of the ruling that respecified it.*** S-D12 (1)'s truth table
  (`:427-432`) made the image record and the BR sub-quad **mutually exclusive** — present/present
  emits subs 0, 1..=9 and no sub 10; absent/present emits subs 0, 10 and no sub-quads — so all three
  readings of "swap" fail, each checked at source: **same node** — no node emits both, nothing to
  swap; **a global code swap** (image → 9, BR → 10) — `staged()` is BYTE-IDENTICAL, because
  `gather_into_staging` packs in SORTED order (`upload.rs:364-374`) and within one node exactly one
  of the two exists, `pack_sort_upload` keys on `scratch.pack.len()` and never sees the code
  (`:458-467`), and max sub (10) < stride (11) forbids cross-node interleaving; **cross-node** —
  G4-2's contract is D4's PER-NODE order, and two nodes are ordered by `(stack, append)` regardless.
  **G4-2 was therefore a gate with no applicable red at all**, and it is the row carrying S-D12 (1)'s
  headline claim. *Note the shape: this bullet has now died twice, and S-D12's amendment list touched
  M4-b, M4-e, M4-f and M4-g and never it.* *Proves the contract is pinned. Note that a pure order
  mutation is **invisible**
  to every image gate unless the two quads overlap — which is why G4-2 asserts the order directly and
  not through a picture.*
* **M4-c1 — emit sub 10 as well on a nine-sliced imaged node** *(added 2026-08-21 — S-D13 (1))*, i.e.
  S-D11's ADD, exactly the bug S-D12 (1) exists to forbid. **G4-2 reds on the record COUNT** (its
  two-node scene goes 12 → 13 staged), **G4-3 reds on the hash** (sub 10 covers all 9 216 px under
  `PREMULTIPLIED_ALPHA`, `resources.rs:438`), and **G4-8 reds on its derived total** (14 → 15).
  *Proves that S-D12 (1)'s suppression is enforced rather than merely written down — the ruling was
  asserted by two gates and mutated by none, which is this campaign's dead-gate shape one step
  removed.*
* **M4-c2 — misassign the sub → record MAPPING: swap the DECODE arms for the FIRST and LAST SLICE,
  sub 1 (TL) and sub 9 (BR)** *(added 2026-08-21 — S-D13 (1); the PAIR corrected 2026-08-21 at
  LANDING — S-D14 (10))*. **G4-2 reds on ORDER**: the sliced node's block comes back BR-first and
  TL-LAST, the contract order inverted at both ends, at an unchanged record count — a pure order
  red, where M4-c1 reds only on count and hash. **OBSERVED**: `slice TL: destination origin — left
  [90.0, 92.0], right [10.0, 20.0]`, with G4-6 and G4-8 staying green.
  **⚠️ The ruled pair — sub 0 and sub 9 — is a red that fires for the WRONG REASON, and the
  distinction is not pedantry: the rung protocol requires the PREDICTED failure to be the OBSERVED
  one.** Sub 0 is pushed for EVERY node (Lands item 7: "It pushes sub 0 always"), so swapping arm 0
  into the BR-slice arm sends every node's sub-0 record — including nodes with no `UiNineSlice` —
  into a decode arm that must resolve `input.nine_slice` and, under S-D12 (3)'s deliberate `.expect`,
  PANICS. G4-2's own scene contains such a node by construction (node A carries an image and no
  nine-slice, which is what sub 10's absence needs to be contrasted against), and the sorted pack
  reaches it first, so the test dies before any order assertion runs. Swapping two SLICE arms is
  total on the nine-sliced node, touches no other node's decode, leaves the count unchanged, and
  reds G4-2 on the per-slice `min_px` its own row already reads. It stays distinct from M4-e, which
  leaves the DESTINATION correct and permutes only the source UV.
  *(The item's own parenthetical "background → 9, TL → 0" additionally leaves BR at 9, so two
  records share one code — which is M4-a's failure, duplication and loss, not an order failure.)*
  **⚠️ It must land in the DECODE (or the emitter's sub → region table), never in the key push —
  MEASURED at source:** the decode loop reads nothing but the sorted key, `staging[dst]` being a pure
  function of `(node, sub)` (`upload.rs:363-374`), so pushing the same code SET in a different order
  is normalized away by the sort and `staged()` comes back byte-identical. A push-side spelling of
  this mutation is **a red that cannot fire** — the class it exists to catch. Exactly three things
  move the staged order: the pushed code set (M4-c1), the sub → record mapping (this), the sort key.
  *This also trips G4-1's contract-order clause, by construction: S-D12 moved G4-1 onto the same
  consequence in `staged()`, so the rows overlap on order and differ elsewhere.*
* **M4-d — pre-`reserve` the 11× worst case at setup.** ~~G4-4 stays green but the scratch's
  steady-state capacity grows 9× for a world with one nine-sliced node. *Not a red — recorded as the
  tempting wrong fix, because the scratch is a `Resource` and the growth is permanent.*~~
  **CONFIRMED green-as-written, and therefore UPGRADED to a real red** *(2026-08-21)*: `ui_no_realloc.rs`
  asserts capacity **stability**, not magnitude — the warm-up check is a LOWER bound (`cap >= N`), the
  armed window compares against the warmed value, and a setup-time reserve is set once and allocates
  nothing inside the window, so all three pass. A mutation no gate can see is not a mutation. **G4-4
  gains an UPPER bound** (`assert!(scratch.pack.capacity() < 2 * emitted)`) — one line — and M4-d becomes
  a red like the others. **That upper bound goes on `ui_no_realloc.rs`'s EXISTING rect-only `N = 4096`
  frame (`:100-140`), it is written on the PAIR of rotating buffers (see G4-4's row — on
  `scratch.pack` alone it is a red that cannot fire, MEASURED at landing), and the mutation is
  applied at the scratch's setup — which today allocates nothing at all
  (`UiRenderScratch::default()`, `pack.rs:278-287`)** *(ruled 2026-08-21 — S-D13
  (4)(6): on the nine-sliced frame G4-4 adds, the bound **cannot fire**. `emitted = 4 096 × 10 =
  40 960`, `2 × emitted = 81 920`, and an 11× reserve is `4 096 × 11 = 45 056 < 81 920`, so the
  assert passes — structurally, since a reserve of 11/node can never exceed 2× an emission of
  10/node. On the rect-only frame `emitted = 4 096`, `2 × emitted = 8 192`, and the same reserve
  overshoots it 5.5×. A red that fires only on the frame nobody attached it to is the same defect
  this bullet was upgraded to fix.)* *The tempting wrong fix is still worth recording as such: the scratch is a
  `Resource` and the growth is permanent.*
* **M4-e — permute which source region a sub-quad samples** *(added 2026-08-21)*: swap the TL and TR
  sub-quads' source UV sub-rects while leaving their **destination** rects correct. G4-3 reds. *Proves
  the golden sees **region assignment**, which the original four mutations left entirely uncovered:
  G4-1 asserts count/consecutiveness/inheritance (blind to UVs), G4-2 asserts record KIND order, G4-4
  is capacity, G4-5 is the enum's value set. Only the picture can see this, and only if the source
  breaks symmetry — which is why G4-3 now requires nine distinct cell values.* **It also presupposed a
  source-UV rule that was stated nowhere — "swap the TL and TR sub-quads' source UV sub-rects" needs
  the sub-rects to be derivable at all — and it moved only occluded geometry until sub 10 was
  suppressed. S-D12 (2) supplies the rule (`border_uv`, fractions of the sub-rect) and S-D12 (1)
  uncovers the geometry; both were required before this mutation could fire** *(2026-08-21)*.
* **M4-g — push the nine sub-quad keys on `UiNineSlice` presence alone, ignoring `UiImage`**
  *(added 2026-08-21 — S-D12 (3))*. G4-8 reds. Against Lands item 7's decode as originally specified it
  is worse than a red: the `.expect` **panics in release**. Against the ruled key push it emits nine
  records that have no texture to sample. *Proves that the key push, not the decode's `match`, is what
  makes every arm total — and that the ruling "`UiNineSlice` alone is a no-op" is enforced rather than
  merely written down.*
* **M4-f — leave `UI_STAGING_ROWS` at 4096** *(added 2026-08-21)*. G4-6 reds: the box overflows at ~~187~~
  **410** *(corrected 2026-08-21 — S-D12 (4). `187 = ceil(2048/11)` divided the NODE budget by the
  stride where the ROW budget was called for; correcting only that gives 373, and S-D12 (1)'s 10
  records/node gives **410** — `10 × 409 = 4 090 ≤ 4 096 < 4 100`. Computed, not asserted. The red is
  unaffected: G4-6 drives 2 048 nodes = 20 480 records, five times over the box.)*
  nine-sliced imaged nodes, `debug_assert!` fires in the test build, and in release the frame is
  silently truncated at the tail with `staging_overflows` bumped. *Proves item 8's constant is
  load-bearing rather than tidy — and that the gate looks at the box production actually packs into,
  not at the growable `Vec` the legacy loop uses.*

**Measurement.** *(added 2026-08-21 — S4 carried no measurement paragraph, and §5 assigns leg 10.8(c)
to "S3–S5". Every other rung states its obligation in the rung; S4 and S5 were the only two without
one.)* **§10.8 leg (c), next increment.** S3 established that the gather cost is the LIST getting
longer, not component presence — a probe returning `None` is still a probe — and landed at 5 pack
inputs + `Children` = **6.00** probes/node/frame. S4's `UiNineSlice` makes it **6 pack inputs + `Children`**:
**6.00 → 7.00 (+16.7 %)**, paid by every node of every changed frame whether or not it is nine-sliced
*(noun corrected 2026-08-21 at LANDING — S-D14 (5): "7 pack inputs' worth" is off by the probe that
is not a pack input. The list holds six after this rung and the census is
`ui_pack_inputs!(count) + 1`, the `+ 1` being the `Children` traversal read. The figure 7.00 is
right; a reader taking the noun literally writes 8.)*.
The instrument exists and already derives its per-node figure from `ui_pack_inputs!(count)`
(`ui_s0_measure.rs:241-248`), so the leg is a one-line extension. **Report the instrument's own
resolution with the number** (§5's standing rule). ⚠️ **One trap in that harness, found in the audit:**
`ui_s0_measure.rs:276` asserts `sys.staged().len() == n` — record count equated with NODE count. It
holds today only because that scene is rect-only. A leg-(c) scene containing nine-sliced nodes reds it
**with nothing wrong** — the same false-red shape S3 recorded when `ui_s0_discovery` wrote the
pack-input list's length down a second time. Convert it to a derived expression in the same edit that
moves `UI_RECORDS_PER_NODE`.

#### The S4 audit ledger — what the rung claimed, and what the tree said

*(2026-08-21, before any S4 code. Verified by reading the named sites and, where a claim was about
behaviour rather than text, by compiling and running a probe.)*

| # | The rung's claim | Verdict |
|---|---|---|
| 1 | `Tile` works because "UVs run past 1.0 and `REPEAT` wraps" | **REFUTED at source.** `resources.rs:310` is `ClampToEdge` in both `UiSamplerMode` variants and `ui_rect.fs.hlsl:198` samples through it. S3's own comment at `resources.rs:307-309` names S4 as the caller that would want `REPEAT` and denies it the default. `Tile` moves to S5 on a new mechanism (S-D11). |
| 2 | G4-5 gates S-D7 by constructing `Tile` + `UiSpriteSheet` | **GATE THAT CANNOT FAIL.** `UiSpriteSheet` has zero `.rs` occurrences tree-wide; S5 creates it. Retired with S-D7; G4-5 re-pointed at the enum's value set. |
| 3 | G4-2 asserts a five-term order on one node's `append` lane | **SUBJECT UNCONSTRUCTIBLE.** Glyphs: `gather.rs:389` hard-codes `text_uv: None`; `pack.rs:208` forbids one record being both glyph and sprite. Focus ring: zero occurrences in `crates/`, and §6 assigns it to Interaction I9. Narrowed to the three terms S4 emits; M4-c respecified. |
| 4 | The record count is determined | **CONTRADICTION, three readings.** D8d `:620` = replace (9/1); D4 `:250` = add; `ui_s0_seam.rs:245` = "seven more". Ruled ADD by S-D11, with the reason (a nine-slice source is a translucent FRAME; the background is the surface it sits on). `UI_RECORDS_PER_NODE = 11`. |
| 5 | "S4's nine-slice raises `UI_RECORDS_PER_NODE` and nothing else at that site" | **FALSE, and the same sentence is already in the tree** at `pack.rs:183-184`. The key push is hard-coded to two (`upload.rs:337-343`) and the decode is binary with an `.expect` that **panics in release** for a nine-sliced node without `UiImage` (`upload.rs:367-373` + `pack.rs:205`). Lands item 7. |
| 6 | G4-4 gates the expansion against reallocation | **WRONG INSTRUMENT, WRONG N, AND SELF-GATING.** It drives `UiRenderScratch` (legacy-only, reached solely via `pack_sort_upload`), at N=4096 not 1024, through a `build_frame` that hand-rolls the emission. Production packs into a fixed 4096-row `Box` that **clamps**, not grows. Re-pointed; G4-6 added; M4-f added. |
| 7 | S4's Lands list is complete | **`ui_pack_inputs!` was missing.** The macro's own doc says "sprites add the rest at S4–S5"; §6 forbids bypassing it; S5's item 6 wires only its own three. Without it `UiNineSlice` is invisible to gather and discovery both. Lands item 6, gate G4-7. |
| 8 | M4-a proves an unstable sort over a non-total key is nondeterministic | **REFUTED BY MEASUREMENT.** `sort_unstable_by_key` over true nine-key tie blocks at n = 9…1800 returned byte-identical output on every repeat and across fresh processes (rustc 1.97.1). The mutation fires; its rationale is rewritten — `append` is the SOURCE ADDRESS, so the real failure is duplication and loss. |
| 9 | M4-d "stays green — not a red" | **CONFIRMED green, and that is the defect.** Upgraded to a red by one upper-bound assert. |
| 10 | `UiNineSlice { … }` is "20 B, padding spelled" | **HALF TRUE — verified by compiling both spellings**: 20 B / align 4 with and without `_pad`. The two bytes were implicit TAIL padding, which is what the prose forbids. `_pad: [u8; 2]` added. |
| 11 | "Layout is untouched" | **CONFIRMED, and structurally so** — `boyko_ui` takes no render dependency and never names `UiInstance` or the pack. Not a defect. |
| 12 | `UI_STAGING_ROWS`'s doc: "2× the plan's own N = 2048 scene" | **ALREADY FALSE AT S3** (2 × 2048 = 4096 = exactly 1×). Corrected and re-derived in Lands item 8. |

##### Second pass — what the AMENDED rung claimed, and what the tree said

*(2026-08-21, still before any S4 code. The implementer refused to build the rung as amended above; an
adversarial pass confirmed the refusal and found more. Ruled by **S-D12**. The lesson the first pass
should have drawn from its own row 4: **ruling a contradiction is not the same as ruling every record
the contradiction touched.**)*

| # | The AMENDED rung's claim | Verdict |
|---|---|---|
| 13 | S-D11 (3): a nine-sliced imaged node emits 11 records, sub 10 the image | **SELF-CANCELLING PICTURE.** Four facts verified at source (the slices' only texture is the node's `UiImage`; the image record is the whole rect at the whole UV, `pack.rs:233-243`, pinned by `ui_pack_cpu.rs:415-416`; it paints last, `upload.rs:364-374`/`:456-467`; `PREMULTIPLIED_ALPHA` lets an opaque source replace, `resources.rs:438`) make sub 10 cover the nine regions exactly. **G4-3 would have pinned a stretched sprite; M4-b and M4-e could not fire.** Ruled SUPPRESS — S-D12 (1). The root: S-D11 (3) argued about the BACKGROUND record and generalized "ADD" to the IMAGE record. |
| 14 | The nine SOURCE sub-rects are derivable at S4 | **THE RULE WAS STATED NOWHERE**, and M4-e presupposed it while G4-3 pinned a 3×3 source without saying why 3×3. Worse than "unreachable at S4": **the engine never records a texture's size at all** — `BindlessTextureTable::register` takes a bare `VkImageView` (`bindless.rs:287`), the table holds no dimension map (`:217-221`), and `UiImage` has no size field. Unity's and Godot's texel-border shape is therefore unavailable, not deferred. Ruled: authored `border_uv`, fractions of the sub-rect — S-D12 (2). Component 20 B → 36 B, GPU bytes unchanged. |
| 15 | Lands item 7 disposes of the release panic | **THE FIX WAS SOUND AND UNGATED**, and it fixed the panic at the wrong end. No row in the amended table constructs a nine-sliced node without `UiImage` — the node that panics — so item 7 repaired a crash nothing drives. Ruled: the KEY PUSH is the sole authority on which subs exist, `UiNineSlice` alone is a structural no-op (S-D12 (3)), gated by **G4-8** and reddened by **M4-g**. |
| 16 | M4-f: "the box overflows at 187 nine-sliced imaged nodes" | **A NUMBER ASSERTED RATHER THAN COMPUTED** — the class this ledger opens with. `187 = ceil(2048/11)`, the NODE budget over the stride. 373 under S-D11, **410** under S-D12 (1). S-D12 (4). |
| 17 | G4-1 asserts consecutive `append` codes `base+1..=base+9` on BOTH loops | **WRONG ON ONE LOOP, UNOBSERVABLE ON THE OTHER.** `pack_sort_upload`'s `append` is the RUNNING RECORD INDEX (`upload.rs:449-455`, `:458-467`), not the `(node, sub)` code — so the claim is not that loop's encoding. On `gather_into_staging`, `UiUploadSystem.keys` is private (`:160`) and the public surface is `staged()`/`probes()`/`repacks()`/`staging_overflows()` (`:266-290`). Re-pointed at the CONSEQUENCE in `staged()`, which is the stronger assertion: it is what M4-a's duplication-and-loss actually breaks. No accessor added. |
| 18 | G4-3's `border_px = [16,16,16,16]` | **THE SAME BLINDNESS THE AMENDMENT ITSELF FOUND, ONE AXIS OVER.** Reason (a) of that amendment removed a symmetric SOURCE because it hid region assignment; the symmetric DESTINATION border hides SIDE ORDER — `[l,t,r,b]` and `[t,l,b,r]` hash identically. And no site stated the side order at all. Ruled `[l,t,r,b]` matching `PackInput::border_width` (`pack.rs:55-57`), G4-3's destination border made asymmetric `[16,24,16,24]`. |
| 19 | G4-7's `mutate_pack_input` gates the discovery half | **ALREADY REPAIRED IN THE TREE** (`50a724ac`) and worth recording as a near miss: it took a `usize` and ended in `_ =>`, so a sixth pack input fell into the catch-all, re-inserted `UiImage` and was reported as covered — **measured green over an unwired input**. Now an exhaustiveness-checked `PackInput` enum, so omitting a variant does not compile. G4-7 re-worded to name the enum. Not a defect of this rung; a defect this rung would have inherited. |

##### Third pass — what the TWICE-AMENDED rung claimed, and what the tree said

*(2026-08-21, still before any S4 code. The implementer refused a SECOND time; a second adversarial
pass confirmed the refusal. Ruled by **S-D13**. The lesson the second pass should have drawn from its
own row 13: **a ruling that changes what the rung EMITS leaves every instrument describing the old
emission, and the ruling's own amendment list is written from the sentence it noticed, not from the
paragraph that computed off it.** S-D12 changed G4-3's border in one row and left M4-b's margin —
and two of its own sentences — computing from the old one.)*

| # | The TWICE-AMENDED rung's claim | Verdict |
|---|---|---|
| 20 | M4-c: "swap the image and the LAST sub-quad (BR) emission order" | **UNWRITABLE — for the same reason its predecessor was, by the hand of the ruling that respecified it.** S-D12 (1)'s truth table makes the two records mutually exclusive. Same node: nothing to swap. Global code swap: `staged()` is BYTE-IDENTICAL — the sort normalizes, within one node exactly one of {BR, image} exists, `pack_sort_upload` never sees the code (`upload.rs:458-467`), and max sub 10 < stride 11 blocks cross-node interleaving. Cross-node: the contract is per-node. **G4-2 — the row carrying S-D12 (1)'s headline claim — had NO applicable red.** Split into M4-c1 (emit sub 10 as well: count 12→13, hash, and G4-8's total 14→15) + M4-c2 (swap the DECODE arms for sub 0 and sub 9). **And M4-c2's first spelling was itself a red that could not fire, caught by the same measurement:** the decode reads nothing but the sorted key — `staging[dst]` is a pure function of `(node, sub)` (`upload.rs:363-374`) — so a PUSH-side code swap is normalized away and `staged()` is byte-identical. The mutation has to move the sub → record MAPPING. S-D13 (1). |
| 21 | Every gate row is named by a red | **G4-5 AND G4-7 ARE NAMED BY NONE**, and G4-2's only one could not be applied (row 20). Full mapping: M4-a→G4-1, M4-b→G4-3, M4-c→G4-2, M4-d→G4-4, M4-e→G4-3, M4-f→G4-6, M4-g→G4-8. Recorded at both rows: G4-5's reds are compile-time (the `Tile` variant, S5's arrival) plus an out-of-range `u8` fed to the pack; G4-7's is the macro edit omitted, caught at build time. S-D13 (1). |
| 22 | `:1295` — the image record's absence "is otherwise pinned nowhere" | **FALSE, AND THE OPPOSITE DIAGNOSIS.** It is pinned TWICE (G4-2 and G4-8's derived total, which goes 14→15 with one extra record) and, until M4-c1, mutated ZERO times. "Pinned nowhere" and "asserted twice, mutated never" call for different fixes; the wrong one was written inside the ruling that created the property. S-D13 (1). |
| 23 | G4-2's instrument: "the shape `ui_s0_seam.rs:288-302` … by `FLAG_TEXTURED`" | **BLIND TO THE PROPERTY IT IS CITED FOR.** On a nine-sliced imaged node index 0 is the untextured background and 1..=9 are nine textured slices, so a wrongly-emitted sub 10 is a TENTH TEXTURED record — the `FLAG_TEXTURED` prefix is identical with and without it. Only LENGTH or GEOMETRY separates them. The cited shape also hard-codes `assert_eq!(staged.len(), 4, …)` (`:289-293`), the literal G4-1 forbids one row above. Re-pointed at length-by-derivation + per-slice `uv`/`min_px`. S-D13 (1). |
| 24 | The expansion lands "in BOTH pack loops", and G4-1 runs a `pack_sort_upload` leg | **THE SECOND LOOP HAS NO CALLER IN THIS WORKSPACE.** `pack_sort_upload`'s only non-doc caller is `host_upload_frame` (`upload.rs:526`); `host_upload_frame`'s only non-doc occurrence in all of `crates/` is its own definition (`:509`) — every other hit is a doc comment, two describing the DELETED `host_upload_frame_from_world`; the name is not re-exported from `lib.rs` or `ui/mod.rs`; the only mention of `UiUploadSystem` outside `boyko_render` is a doc comment (`dispatcher_token.rs:531`). It is the surviving half of the path S0 replaced. Ruled: **S4 expands `gather_into_staging` only**, via a loop-agnostic emitter both callers share. Deletion of the dead loop is public-API SCOPE and is FILED, not decided. S-D13 (2). |
| 25 | G4-1: "assert the record count DERIVED from `UI_RECORDS_PER_NODE` and `fill_center` — never a literal" | **THERE IS NO SUCH FORMULA.** S-D12 (1) severed stride (11) from emission (10/9/2/1); no expression in (stride, `fill_center`) yields 2 for the imaged row, and `UI_RECORDS_PER_NODE - 1` gives 10 only by the accident that exactly one of {centre, image} is dropped. `UI_RECORDS_PER_NODE` (`pack.rs:185`) is the tree's only region constant — **and G4-8 one row later already breaks the instruction with four literals**. Ruled: mint `UI_NINE_SLICE_REGIONS` / `UI_NINE_SLICE_SUB_BASE` / `UI_IMAGE_SUB`, derive the stride from the largest sub code, and state the rule literals must satisfy. S-D13 (3). |
| 26 | G4-1: "all nine carry the parent's `StackIndex` and clip" | **`UiInstance` DOES NOT CARRY A STACK.** Field set: `min_px, size_px, clip, corner_radius, uv, color, border_color, border_width, flags` (`upload.rs:110-120`); `stack` is on `UiNode` (`:99-101`), pushed into the private key lane, consumed as the sort key only. **The same shape S-D12 struck from this row for the `append` codes, one clause later in the same sentence.** Re-pointed at stack-BRACKETING (`ui_s0_seam.rs:255-281`). S-D13 (4)(1). |
| 27 | G4-3: "each corner … samples only its own source cell" | **FALSE UNDER THE DEFAULT SAMPLER, and the row named no mode.** `UiSamplerMode::Smooth` is `Filter::Linear` and `#[default]` (`resources.rs:101-119`); the existing sprite golden runs Smooth as its primary leg (`ui_sprite_gpu_golden.rs:442-444`), Pixel as a reachability leg (`:473-474`). Magnifying a 3-texel axis 32× under Linear blends past each cell's texel centre. Ruled `Pixel`, which makes the sentence true as written. S-D13 (4)(2). |
| 28 | G4-5: "an out-of-range `mode` discriminant is rejected at pack" | **UNWRITABLE IF `PackInput` CARRIES THE ENUM** — producing one needs a `transmute` into a ONE-variant enum, instant UB, which cannot be a gate. In-tree precedent: `UiImageInput.slot`, a raw `u32` with a `debug_assert!` at the pack boundary (`pack.rs:31-33`, `:212-217`). Ruled: raw `u8` in `PackInput`, typed enum on the authored component. S-D13 (4)(3). |
| 29 | G4-5 / Lands item 1: "a variant-count `const` assert" | **DOES NOT EXIST ON THIS TOOLCHAIN — MEASURED.** `std::mem::variant_count` on rustc 1.97.1 is `E0658` (nightly-only, issue #73662) *and* "not yet stable as a const fn": two errors, one line. Stable spelling measured green and measured RED (`E0004` on adding `Tile`): `const _: () = match NineSliceMode::Stretch { NineSliceMode::Stretch => () };` — **without braces**, since the braced form emits `unused_braces`, an error under the project's `-D warnings` gate. S-D13 (4)(4). |
| 30 | Lands item 1 rules `border_uv`'s `Default` | **`border_px`'s WAS UNSTATED**, and under S-D12 (1) it does not degrade to S3's picture: presence suppresses the image, so `[0;4]` leaves every corner and edge at zero destination extent and the node renders **the middle ninth of its texture, zoomed to fill**. S-D12 (2)'s validity domain passes it (`0 + 0 > rect.w` is false). Ruled `[0.0; 4]`, degenerate picture ACCEPTED (the `UiImage` alpha-0 default-tint shape), and STATED in the field's doc. S-D13 (4)(5). |
| 31 | M4-d's new upper bound `assert!(cap < 2 * emitted)` reds on G4-4's scene | **CANNOT FIRE THERE.** `ui_no_realloc.rs` is `N = 4096` (`:102`, `:191`); nine-sliced + imaged ⇒ `emitted = 40 960`, `2 × emitted = 81 920`, an 11× reserve is `45 056 < 81 920` → PASSES. Structural: a reserve of 11/node can never exceed 2× an emission of 10/node. Ruled onto the file's existing rect-only frame, where `2 × emitted = 8 192` and the reserve overshoots 5.5×. S-D13 (4)(6). |
| 32 | M4-b's margin: "16×16 corners … ~3 000 of 9 216 px" | **STALE BY THE HAND OF S-D12 ITSELF**, which changed G4-3's border to `[16,24,16,24]` ONE ROW ABOVE and left this bullet — and two of its own sentences (`:457`, `:465`) — computing from `[16,16,16,16]`. Correct: a corner is 16 × 24 = **384 px**, four correct corners **1 536**, four equal-thirds corners **4 096**, delta **2 560 of 9 216**. The red is unaffected; the doc-rot-repair class is the finding. S-D13 (5)(1). |
| 33 | G4-2 / §6: "the `append` lane's order", and G4-7: "does not compile" | **TWO STRUCK-NOUN PROPAGATIONS AND ONE IMPRECISION.** `append` lane is what S-D12 removed from G4-1 as unobservable (`keys` private, `upload.rs:160`), and §6 was carrying it into the Interaction plan — where a struck claim does the most damage, since I9 would write a gate against a lane it cannot read. Both become "the STAGED order". G4-7: two edits, two outcomes, **both structural** — macro + enum + `ALL` without arms ⇒ `E0004` **twice** (`name()` and `mutate_pack_input` both exhaustive, no catch-all); macro only ⇒ compiles, reds at RUNTIME on `assert_eq!(ALL.len(), ui_pack_inputs!(count), …)` (`ui_s0_discovery.rs:265-274`). Gated either way — precision, not a hole. Also: G4-3 was the ONE row naming no pack loop in a table whose preamble requires it. S-D13 (5)(2)(3)(4). |

---

### S4 · LANDED 2026-08-21 — the landed set, the RED ledger, the golden, and what the build found

*(Third implementer, first build. The two previous refusals were correct and are recorded above; this
pass ran the rung. Every gate below was run with its exit code seen UNPIPED, every red was APPLIED and
its failure OBSERVED, and every mutated source was restored and verified byte-identical with `cmp`
against a pre-mutation snapshot. Ten corrections landing found are ruled in **S-D14**.)*

#### The landed set, file by file

| File | What landed |
|---|---|
| `crates/boyko_ui/src/components.rs` | `NineSliceMode` (`#[repr(u8)]`, one variant, pinned by the brace-less one-variant `const` match) and `UiNineSlice { border_px, border_uv, mode, fill_center, _pad }` — **36 B / align 4, MEASURED by the `const _: () = assert!` that compiled**. `Default` = `[0.0;4]` / equal thirds / `Stretch` / **`fill_center: true`** (S-D14 (1)), with the degenerate zero-inset picture stated in the field's own doc. |
| `crates/boyko_render/src/ui/pack.rs` | `UiNineSliceInput` (raw `u8` mode) and the `nine_slice` field on `PackInput`; the sub-space constants `UI_NINE_SLICE_REGIONS` / `_SUB_BASE` / `_CENTER_SUB` / `UI_IMAGE_SUB` / `UI_MAX_SUBS_PER_NODE` / `UI_NINE_SLICE_MODE_COUNT` with `UI_RECORDS_PER_NODE = UI_IMAGE_SUB + 1` and three `const _` relations; `split_axis`; `pack_ui_nine_slice_instance`; **`ui_node_sub_codes`** (the sole authority — S-D12 (1)'s truth table as code), **`pack_ui_sub_record`** (the decode), **`emit_ui_node_records`** (the append wrapper G4-4 drives). `UI_RECORDS_PER_NODE`'s doc rewritten — it repeated the claim ledger row 5 refuted. |
| `crates/boyko_render/src/ui/upload.rs` | `UI_MAX_NODES = 2048` and `UI_STAGING_ROWS = UI_MAX_NODES * UI_RECORDS_PER_NODE` (22 528 rows, 1.72 MiB) replacing the bare `4096` whose doc claimed 2× the measurement scene; the key push becomes a loop over `ui_node_sub_codes`; the decode's binary `if` becomes `pack_ui_sub_record(.., append % UI_RECORDS_PER_NODE, ..)`. The "background rect, then its sprite quad" comment corrected. |
| `crates/boyko_render/src/ui/gather.rs` | `UiNineSlice` added to `__ui_pack_inputs_list!` (Lands item 6 — the omission that would have made the rung invisible), the read tuple widened, and the **one** exhaustive `NineSliceMode → u8` conversion, which is the `E0004` that binds `UI_NINE_SLICE_MODE_COUNT` to the enum. |
| `crates/boyko_render/src/ui/mod.rs`, `src/lib.rs` | The new surface re-exported. |
| `crates/boyko_render/tests/ui_s4_nine_slice.rs` | **NEW** — G4-1 (two cases), G4-2, G4-5, G4-6, G4-8. Device-free, driving `gather_into_staging` through `run_system_once`; every count an expression over the minted constants; the nine destination and nine source rects authored BY HAND rather than recomputed with the pack's own formula. |
| `crates/boyko_render/tests/ui_nine_slice_gpu_golden.rs` | **NEW** — G4-3. Drives the scheduler's own pack and uploads `sys.staged()`; `Pixel`; 3×3 nine-distinct-cell source; `border_px = [16,24,16,24]`; opaque white tint; nine region probes, four boundary probes, a full colour census, and the S-D6 image pin. |
| `crates/boyko_render/tests/ui_no_realloc.rs` | G4-4: the nine-sliced frame through `emit_ui_node_records` (not the file's hand-rolled loop), plus M4-d's upper bound on the **pair** of rotating buffers on the existing rect-only frame. |
| `crates/boyko_render/tests/ui_s0_discovery.rs` | G4-7: the sixth `PackInput` variant, `ALL`, `name()` and `mutate_pack_input` arm. |
| `crates/boyko_render/tests/ui_s0_seam.rs`, `ui_s0_measure.rs` | The two in-tree comments S-D12 listed, corrected; and `assert_eq!(staged().len(), n)` given its reason (rect-only ⇒ one record per node) so a leg-(c) scene with sprites does not red it with nothing wrong. |
| 11 other files | `nine_slice: None` at every `PackInput` literal — the lockstep the S2 widening's SR1 anticipated. No packed byte moves. |
| `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` | Two anchors re-pointed at `ui/mod.rs:96`; the re-export edit moved `FRAMES_IN_FLIGHT`, and `internal_docs_anchors` caught it. |

#### The RED ledger — what was applied, and what was OBSERVED

| Red | Applied at | Gate(s) that fired | What was OBSERVED |
|---|---|---|---|
| **M4-a** — all nine sub-quads share one append index | the key push | G4-1 (both cases), G4-2 | `slice T: destination origin — left [20.0, 40.0], right [52.0, 40.0]`: every slice resolved to TL, the other eight LOST. **Duplication and loss, not a shuffle** — exactly what the bullet's own correction predicted, `append` being the SOURCE ADDRESS. G4-6 and G4-8 stayed green (the record COUNT is unchanged), which is the mapping M4-a→G4-1 holding. |
| **M4-b** — corners proportional instead of `border_px` | `pack_ui_nine_slice_instance`'s destination split | G4-3 | `…and the NEXT pixel is already the CENTRE — left [255,0,0,255], right [255,0,255,255]`: pixel (32,40) came back TL red because a proportional corner is 32×32. |
| **M4-c1** — emit sub 10 as well on a sliced imaged node | `ui_node_sub_codes` (+ the scratch bound it overflows) | G4-2, G4-8, G4-3, and incidentally G4-1 and G4-6 | **The recorded numbers, exactly**: G4-2 `13` vs `12`; G4-8 `15` vs `14`; G4-3 `11` vs `10` staged. |
| **M4-c2** — swap the decode arms for sub 1 (TL) and sub 9 (BR) | `pack_ui_sub_record` | G4-2, G4-1 | `slice TL: destination origin — left [90.0, 92.0], right [10.0, 20.0]`: the block comes back BR-first, at an UNCHANGED count (G4-6, G4-8 green). **A pure order red — and it required correcting the ruled pair; see S-D14 (10).** |
| **M4-d** — pre-`reserve` the worst case at scratch setup | `UiRenderScratch::default()` | G4-4 | First application: **GREEN — the red did not fire.** `pack 4 096 / gather 22 528`: the swap in `sort_by_stack` had parked the reserve in the other buffer. With the bound moved onto the pair: `22528 rows of capacity … for a frame that emitted 4096 records`. **S-D14 (9).** |
| **M4-e** — permute TL/TR source UV, destination correct | `pack_ui_nine_slice_instance`'s source column | G4-3 | `region TL samples its own source cell — left [0,0,255,255], right [255,0,0,255]`: TL's destination showing TR's blue. |
| **M4-f** — leave `UI_STAGING_ROWS` at 4096 | the constant | G4-6 **alone** | `UiUploadSystem staging box overflow: gather emitted 20480 records into a 4096-row box`. Every other gate green — the mapping M4-f→G4-6 holding exactly. |
| **M4-g** — push slice codes on `UiNineSlice` presence alone | `ui_node_sub_codes` | G4-8 **alone** | The decode's `.expect` fired on the sliced-but-imageless node: *"a nine-slice sub code is emitted only for a node carrying BOTH …"*. Worse than a red, as the bullet says — and that node is the one no gate constructed before G4-8 existed. |
| **G4-5's instrument** — disarm the `mode` `debug_assert!` | `pack_ui_nine_slice_instance` | G4-5 | `test did not panic as expected`. Recorded because a `#[should_panic]` gate whose subject is deleted is the quietest of all vacuous passes. |

**Byte-identical restoration.** `crates/boyko_render/src/ui/pack.rs`
`c5223005384fb255b3b38ef3a0c7d6964f490f1835ee63d7b073ce9e936e41e4` and
`crates/boyko_render/src/ui/upload.rs`
`bc33a82ad60393fc84edb42569a429f4fca5f6b872fc80fab81bfae6bf8db161` — the same SHA-256 before the
first mutation and after the last restore, with `cmp` clean after every individual one.

**The two reds not in the M4-* list, and why they are here.** G4-7's red is the macro edit itself
being omitted, and BOTH of S-D13 (5)(4)'s mechanisms were observed in the order that edit is naturally
made: adding `UiNineSlice` to `ui_pack_inputs!` and running `ui_s0_discovery` gave the RUNTIME red
(*"this test drives 5 pack inputs but `ui_pack_inputs!` declares 6 — add the new one as a `PackInput`
variant, to `PackInput::ALL`, and to `mutate_pack_input`"*); adding the variant and `ALL` without the
arms gave **`error[E0004]` twice**, at `name()` and at `mutate_pack_input`, neither carrying a
catch-all. The precision S-D13 (5)(4) insisted on is real and was measured.

#### The golden — every colour accounted for

`ui_nine_slice_gpu_golden` blessed on an RTX 3060 Laptop GPU with validation ON, and **LOOKED AT**.
An independent census of the dumped BMP found **exactly ten distinct colours and no eleventh**:

| Colour | px | Why |
|---|---|---|
| CLEAR `#112233` | 7 168 | `128² − 96²` — the target minus the node's rect, exactly |
| C magenta | 3 072 | 64 × 48 |
| T green, B violet | 1 536 each | 64 × 24 |
| L yellow, R cyan | 768 each | 16 × 48 |
| TL red, TR blue, BL orange, BR grey | 384 each | 16 × 24 |
| the node's OLIVE background | **0** | the nine regions tile the rect and every slice is opaque |

Total 16 384 = 128². The column widths (16, 64, 16) and row heights (24, 48, 24) are the authored
`border_px = [16, 24, 16, 24]` read back off the picture, which is what "the corners are preserved"
means when it is measured rather than asserted. There is no blend seam — `Pixel` is NEAREST and the
regions tile on integer pixel boundaries — which is exactly why this pin, unlike S3's, has no
one-pixel-seam colours to explain.

**The five existing pins did NOT move**, and that is the S4 duty discharged: four S2 pins
(`ui_rect_gpu_golden`, `ui_rect_swapchain_golden`, `ui_text_gpu_golden`,
`ui_text_multiscale_gpu_golden`) plus S3's `ui_sprite_gpu_golden`, all green in the full run. They
could not have moved: S4 changes what is emitted only for a node carrying `UiNineSlice`, and no other
scene in the tree has one.

#### Gates, unpiped

| Gate | Command | Result |
|---|---|---|
| G4-1, G4-2, G4-5, G4-6, G4-8 | `cargo test -p boyko-render --test ui_s4_nine_slice` | `running 6 tests` · `6 passed` · exit 0 |
| G4-8's release leg | the same with `--release` | `running 5 tests` · `5 passed` · exit 0 |
| G4-3 | `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1 cargo test -p boyko-render --test ui_nine_slice_gpu_golden -- --test-threads=1` | `running 1 test` · `1 passed` · exit 0 |
| G4-4 | `cargo test -p boyko-render --test ui_no_realloc` | `running 4 tests` · `4 passed` · exit 0 |
| G4-7 | `cargo test -p boyko-render --test ui_s0_discovery` | `running 2 tests` · `2 passed` · exit 0 |
| Regression | `cargo test -p boyko-render --lib --tests --no-fail-fast -- --test-threads=1` | **58 targets, 0 failed**, exit 0 |
| Regression | `cargo test -p boyko-ui --lib --tests --no-fail-fast` | **47 targets, 0 failed**, exit 0 |
| Lint | `cargo clippy -p boyko-render -p boyko-ui --all-targets -- -D warnings` (touch-first) | exit 0 |
| Censuses | `engine_packages_census`, `goldens_pins_wellformed`, `internal_docs_anchors`, `gpu_blocking_reader_census` | 3+7+5+2 passed, exit 0 each *(the docs census RED first, on two anchors the re-export edit had moved — repaired, re-run green)* |
| Downstream | `cargo check -p boyko-app -p boyko-engine --all-targets` | exit 0 |

#### The measurement obligation, discharged

**§10.8 leg (c), MEASURED** — `cargo test -p boyko-render --test ui_s0_measure -- --ignored
--test-threads=1 --nocapture`, `running 3 tests`, exit 0, on this box:

```
the LIST cost: 6 pack inputs + Children = 7 probes/node/frame
  N=256:  probes/frame=1792   probes/node=7.00   gather min/median/max = 104.2/105.7/108.2 us
  N=2048: probes/frame=14336  probes/node=7.00   gather min/median/max = 835.9/844.4/921.0 us
```

**6.00 → 7.00 probes/node/frame (+16.7 %)**, the predicted figure, paid by every node of every
changed frame whether or not it is nine-sliced — S3's finding that the cost is the LIST getting
longer, not component presence, holds one rung on: the leg-(a) and leg-(c) worlds differ only in
probe-hit versus probe-miss and report the same 7.00. **Instrument resolution, per §5's standing
rule: `std::time::Instant` (QPC on Windows), ~0.1 µs floor** — which is three to four orders below
the gather times above, so the µs figures are signal and not floor. The static-dispatch leg still
reads `0.10/0.20/0.90 µs` with `probes = 0` and `repacks avoided = 100/100`: S4 costs the STATIC
frame nothing, because the D6a gate still returns before one component is probed.

#### Deviations from the rung as written, each with its reason

1. **M4-c2's sub pair** is 1↔9, not 0↔9 — S-D14 (10). The ruled pair panics before the property it
   mutates is asserted.
2. **M4-d's bound** is on `max(pack, gather)`, not on `scratch.pack` — S-D14 (9), measured.
3. **`UI_MAX_SUBS_PER_NODE`, `UI_NINE_SLICE_CENTER_SUB`, `UI_NINE_SLICE_MODE_COUNT`** minted beyond
   the three the ruling listed — S-D14 (2). The third was already required by a `debug_assert!` the
   ruling prescribed.
4. **`ui_s0_measure.rs`'s `staged().len() == n`** became `n * RECORDS_PER_RECT_ONLY_NODE` with the
   truth-table row named, rather than a derivation over the sub-space constants: a rect-only node
   emits one record and no expression over those constants yields 1 without pretending to. The
   *reason* is what the site was missing, and the reason is now there.
5. **`emit_ui_node_records` is a wrapper, not the primitive** — S-D14 (4).
6. **Applying M4-c1 also required widening the sub-code scratch** by one, since the array is sized to
   the EMISSION maximum. Two lines, one mutation, restored together.
7. **`docs/FEATURE_MAP.md` / `docs/SYSTEMS.md` were not extended.** Neither tracks the UI pack lane at
   all — `UiImage`, `ui/pack.rs`, `UI_RECORDS_PER_NODE` and `ui_pack_inputs!` have zero occurrences in
   either — so S0-S3 did not register there and S4 registering alone would create an obligation the
   campaign has never carried. Named here rather than done silently.

---

