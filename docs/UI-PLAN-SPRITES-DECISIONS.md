> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §3 decisions S-D2–S-D21. S-D1 is in §0, in the index.

## 3 · Decisions this plan adds

Each is numbered, carries a reason, and names what was rejected. **S-D1 is stated in §0**, where the
boundary it draws is the subject; S-D2 onward follow here.

### S-D2 — the `flags` bit budget is fixed here, once, with the assert beside it

```
bit 0   FLAG_BORDER_ANY      (exists)
bit 1   FLAG_CLIP_PRESENT    (exists)
bit 2   FLAG_TEXT            (exists)
bit 3   FLAG_TEXTURED        (S3 — the sprite lane)
bit 4   reserved: sampler index (S7's deferred per-sprite filter; see S-D4)
bits 5..19  free
bits 20..31 bindless slot — 12 bits, slots 0..4095, EXACTLY the table's range
```

```rust
const _: () = assert!(BINDLESS_TEXTURE_CAPACITY <= 1 << 12,
    "UiInstance.flags carries the bindless slot in bits 20..31");
```

**Reason.** D1 fixes the field list but not the bit assignment, and the assignment is the half a
second author can get wrong silently. The 12-bit field has **zero** headroom (D3 refuses a UI
reservation, so "raise the capacity" is the natural response to slot pressure, and a raised capacity
truncates the field and makes a UI quad sample a different texture). Bit 4 is reserved **now**,
unused, because S7's deferred per-sprite filter is the one extension that would otherwise want a
second widening.

**Rejected:** packing the slot at bits 3..14 adjacent to the flags (leaves the high bits free but puts
the widest field next to the one that grows, so every new flag risks the slot); a separate `u16 slot`
field (+4 B on every instance for a field 95 % of nodes do not use, and D1's 80 B has no room without
a tail pad).

### S-D3 — the set-1 bind needs a generic verb; it is added to `boyko_rhi`, not routed around

`bind_descriptor_set` binds at **set 0** by contract (`encoder.rs:174-186`). The offscreen golden
drives `record_ui_rects` through the trait; the on-screen path drives `present_blit.rs` concretely.
Binding the bindless set at set 1 is expressible today only on the concrete path.

**Decision: add `RhiCommandEncoder::bind_descriptor_set_at(set_index, group, pipeline)`** — an
additive default-no-op trait method with a Vulkan override, exactly the shape every other verb in that
trait already has (`#[cold] #[inline(never)]` default body, backend override). `bind_descriptor_set`
becomes `bind_descriptor_set_at(0, …)` and keeps its signature, so no existing call site moves.

**Reason.** The alternative leaves the two recorders structurally different at the exact place they
must agree, and the offscreen golden — the only one that runs without a display — would stop
exercising the sprite path. That is the "the gate could not fail" shape this project keeps recording.
The cost is one trait method and one Vulkan override.

**Rejected:** (a) *a Vulkan-only escape in `record_ui_rects`* — makes the generic recorder
non-generic and un-testable through the trait encoder. (b) *`UiPass` carries an optional raw
`VkDescriptorSet` and only the on-screen path binds it* — the offscreen golden then cannot draw a
sprite, so **S3's gate would be untestable on a device-less machine and unverifiable in CI**;
`M3-c` exists precisely because these two paths diverge.

### S-D4 — the UI owns its sprite sampler; the shared bindless sampler is not the UI's to choose

The bindless set's sampler is **immutable**, **trilinear**, **16× anisotropic**, **`REPEAT`**
(`bindless.rs:139-172`). It was chosen for tiled world material textures. It is the only sampler that
set offers, and it is baked into the layout for the layout's lifetime.

Three consequences, none of which the research or the architecture recorded:

1. **A pixel-art UI sprite cannot be `NEAREST`.** Under D2 the UI would inherit LINEAR forever.
   Under the rejected Model A the UI owns its atlas sampler and could pick either — so this is a real
   cost of D2 that must be paid rather than discovered.
2. ~~**`REPEAT` is what makes tiled nine-slice cheap (D8d), and it is also what makes it narrow.** A UV
   outside `[0,1]` wraps to the **whole texture**, not to a sheet frame — see S-D7.~~
   **`REPEAT` is not what makes tiled nine-slice cheap, and this consequence was a false lead that cost
   a decision (S-D7) and a gate (G4-5).** *(corrected 2026-08-21 at the S4 pre-build audit.)* The
   decision below gives the UI a `ClampToEdge` sampler in both modes — which was the right call and the
   S3 code says so at `resources.rs:307-309` — and therefore **`REPEAT` is not on the UI's sampling path
   at all**, so nothing downstream may reason from it. Tiling is a `frac` inside the sprite's own
   sub-rect (**S-D11**), which costs one shader instruction, wraps to a **sheet frame** rather than to
   the whole texture, and consequently has no narrowness to guard against.
3. **`mipmapMode = LINEAR`, `maxLod = 1000`.** A single-mip UI texture always resolves to level 0, so
   nothing is wrong today; a UI texture uploaded *with* mips would trilinear-blend across sheet frames
   under minification.

**Decision: the UI declares its own `SamplerState` at set 0, binding 3, and samples the bindless
texture with it** — `g_textures[NonUniformResourceIndex(slot)].Sample(g_ui_sampler, uv)`. The mode is
chosen once at `ui_setup` (`UiSamplerMode::{Smooth, Pixel}` → LINEAR/ClampToEdge vs NEAREST/ClampToEdge).
**Zero per-instance bytes, zero change to the shared bindless set.**

This needs one additive RHI change: `DescriptorKind::Sampler = 0` (`VK_DESCRIPTOR_TYPE_SAMPLER`) plus
a `BindGroupEntry::Sampler { sampler }` variant — the same additive shape `AccelerationStructure` and
`SampledImageAtGeneral` already took in those two enums.

**A cheaper route is tried first, and it is a measurement, not an argument.** Vulkan permits a
`COMBINED_IMAGE_SAMPLER` descriptor to be accessed as a plain sampler, which would let the UI's
existing binding-1 atlas sampler serve the bindless texture with **no RHI change at all**. Whether
this backend's descriptor write and this shader's SPIR-V actually satisfy that is a validation
question with a live local oracle: the UI goldens already assert **zero** validation messages. So
**G3-0** decides it, and whichever way it lands is recorded. If it lands green, the RHI change is
dropped and S3 shrinks; if it lands red, the additive variant ships and the message is quoted in the
commit.

**Rejected:** (a) *accept the shared sampler* — permanently forecloses pixel-art UI, and the ceiling
cannot later be lifted without editing a world-shared descriptor set, which is the one thing D3
refuses. (b) *add a second sampler to the bindless set layout* — a UI concern mutating a table shared
with every world material; the exact Principle-0 inversion D3 names. (c) *per-sprite sampler in v1* —
needs N samplers plus an index field; deferred to S7 with bit 4 already reserved for it.

### S-D5 — the first sprite texture is procedural, not a checked-in PNG

Every gate in S3–S5 that needs a texture builds it in Rust — an 8×8 RGBA8 checkerboard, a 3×3
nine-slice source, a 4×4 flipbook grid — and uploads it raw through the existing
`create_solid_color_texture`-shaped path, registering it into a `BindlessTextureTable`.

**Reason, three parts.** (1) `boyko_image` is a **decoder only**; there is no encoder in the tree, so a
checked-in PNG would have to be authored by hand outside the repo and could not be regenerated by
anything the repo owns. (2) A procedural texture is **bit-reproducible**, which is what an image pin
needs. (3) It removes an asset dependency from the campaign's critical path — the architecture's D32
notes the asset floor is genuinely zero and that the sprite leg has no slack; this decision gives it
slack.

The *demo* sprite (D32's observer rung) may still be a checked-in PNG; the *gates* may not depend on
one.

**Rejected:** a checked-in PNG for the gates — makes every sprite test depend on the PNG decoder's
correctness as well as the sprite path's, and a decoder regression would red the sprite gates for the
wrong reason.

### S-D6 — the UI gets image pins, because it has none and S2 is the change that needs them

`goldens/PINS.toml` has **zero** UI rows and the four UI GPU goldens assert individual texels. A texel
assertion cannot see a UV that moved by a texel — which is exactly what D1's un-aliasing does to every
glyph.

**Decision: each UI GPU golden gains a SHA-256 of its full readback**, asserted in-test against a
constant in the test file, with `BOYKO_UI_GOLDEN_BLESS=1` printing the fresh hash and dumping a BMP
for a human to look at. The existing texel assertions stay — they say *what* is wrong; the hash says
*that* something is.

**Reason.** This is not tidiness. **M2-b** — swapping two fields in the HLSL mirror only — is the
mutation **R1** says nothing in the tree can currently see, and a full-image hash is the cheapest
thing that sees it. The pin is only as live as a machine with a GPU — since the 2026-08-21 ruling
re-pointed S0's observer at device-free Phase 1, that liveness comes from the owner-run windowed
leg (SR1's completion criterion), not from S0.

**Rejected:** (a) adding the UI goldens to `goldens/PINS.toml` — that file's discipline is a BMP dump
from a windowed `boyko_app` test driven by `scripts/golden.ps1`; the UI goldens are offscreen
readbacks in `boyko_render/tests` with a graceful device-less skip, and forcing them into that shape
would mean building a windowed UI dump test before the widening. (b) leaving the texel assertions
alone — the widening then lands with no gate that can fail on the shader half.

### S-D7 — ~~`Tile` nine-slice requires a whole-texture sprite; with a sheet it is a hard error~~ **RETIRED 2026-08-21 — the hazard was an artifact of a mechanism that will never ship (see S-D11)**

> **RETIRED at the S4 pre-build audit, 2026-08-21.** Every sentence below is preserved because the
> reasoning was sound *given its premise*, and the premise is the interesting part: the whole decision
> is downstream of "tiling = UV past `[0,1]` + `REPEAT`", and **S3 landed a UI sampler that is
> `ClampToEdge` in BOTH modes** (`resources.rs:310`, whose own comment names this rung). There is no
> `REPEAT` anywhere on the UI's sampling path — `ui_rect.fs.hlsl:198` samples
> `g_sprites[…].Sample(g_ui_sampler, uv)`, the UI's own sampler, never the bindless set's. So the
> mechanism does not exist, the sheet hazard it creates does not arise, and the clamp that guards it
> guards nothing. **S-D11 replaces it**: tiling is `frac` *within the sub-rect*, which is well-defined
> under a sheet and therefore needs no guard at all. The gate this decision spawned (`G4-5`) named
> `UiSpriteSheet`, a type S5 creates — so it was also a gate that could not fail. Both are gone.

~~Tiled edges work by letting the UV run past `[0,1]` and letting `REPEAT` wrap. Under a sheet, `REPEAT`
wraps to the **whole sheet**, so a tiled edge would tile the neighbouring frames.~~

~~**Decision:** `UiNineSlice { mode: Tile }` on a node that also carries `UiSpriteSheet` is a
`debug_assert!` in dev and a **clamp to `Stretch`** in release, with the clamp counted in a diagnostic
counter so it is observable rather than silent.~~

~~**Reason.** The combination is not expressible under S-D4's sampler and cannot be made expressible
without per-sprite address modes. Failing loudly in dev and degrading visibly-but-safely in release is
the house pattern (`D15`-shaped release-present clamps in `PARTICLES-PLAN.md`).~~

**Rejected (still rejected, and now for a second reason):** (a) emulate tiling with geometry (Unity's
quad-per-tile explosion and its documented 16 250-quad cap — the exact thing D2 exists to avoid);
(b) silently treat it as `Stretch` (a wrong image with no trace). Under S-D11 (a) is additionally
unnecessary, because the shader-side `frac` costs one instruction and zero records.

### S-D8 — the default-OFF ladder, stated rung by rung

| Rung | What is new | Default | Which rung turns it on |
|---|---|---|---|
| S0 | gather, generation gate, host rung | gate ON (it can only *skip* work), host rung is a new binary nothing else runs | S0 itself, in its own binary |
| S1 | eDSL-generated shader | **byte-identical `.spv`** — nothing changes | never (it is a refactor) |
| S2 | 80 B record, `uv` field | every existing node packs `uv = (0,0,1,1)` and `flags` bits 3..31 zero | never (image pins must be identical) |
| S3 | `FLAG_TEXTURED`, set 1, UI sampler | `UiImage`'s default tint is **alpha 0** ⇒ an authored-but-untextured Image is invisible (`components.rs:454-466`) | S3's own new golden `ui_sprite_bindless` |
| S4 | `UiNineSlice` (**`Stretch` only** — `Tile` moved to S5 by the 2026-08-21 audit, S-D11) | absent ⇒ pack emits 1 record (+ its image), **byte-identical to S3** | S4's own golden `ui_nine_slice` |
| S5 | sheets + flipbook **+ `NineSliceMode::Tile`** *(added 2026-08-21: `Tile` needs the sub-rect arithmetic S5 builds — S-D11)* | `UiSpriteSheet` absent ⇒ `uv` comes from `UiImage`; `UiSpriteCursor` absent ⇒ no tick; **`mode != Tile` ⇒ no `frac`, `FLAG_TILED` zero — and `mode == Tile` on a region whose two counts are both 1 (every corner) ALSO packs `FLAG_TILED` zero, so a tiled node's four corners are byte-identical to their `Stretch` records** *(added 2026-08-21 — S-D15: the flag is set only when a count exceeds 1, which is what lets G4-3's corner claim carry over unchanged)* | S5's own goldens `ui_flipbook` + `ui_nine_slice_tiled` |
| S6 | `.ui` vocabulary | authoring only; no runtime behaviour | — |

**Every rung's own goldens are the only images that change at that rung.** The S2 image pins
(S-D6) are the invariant carried forward from S2 to the end of the ladder.

### S-D9 — the two recorders are gated separately, on purpose

`record_ui_rects` (generic, offscreen) and `present_blit.rs` (concrete, on-screen) are two
implementations of one contract. Every rung that changes the recorder must gate **both**, and
**M3-c** is the mutation that proves the two gates are not one gate wearing two names.

**Reason.** The on-screen path is exercised only by `ui_rect_swapchain_golden` and by the (`#[ignore]`d
× 8) HUD screenshot; the offscreen path by three goldens. A change made in one and forgotten in the
other is invisible to the other's gate — and the on-screen path is the one a user sees.

### S-D10 — the eDSL generator owns the whole `.hlsl` for both UI stages, and `.spv` byte identity is S1's end condition

Following `emit_particles` / `emit_probe_gi`: the generator owns the entire file as a `format!`
template with the eDSL spans spliced between sentinels, because the skeleton (bindings, `VsOut`,
`[[vk::push_constant]]`, the struct mirror of `UiInstance`) carries numbers that must agree with host
`offset_of!` constants, and single-sourcing them is the point.

**S1's end condition is: the committed `.spv` are byte-identical after a re-DXC.** The `.hlsl` bytes
**will** change (the printer's formatting is not the hand author's).

**The fallback, recorded now so it is not a surprise:** if DXC's output moves at the same source
semantics — a plausible outcome of a whitespace or declaration-order change — S1 lands as a **single
recorded re-bless**: the four UI goldens are re-run on a device, the images compared by a human, the
`SpirvBlob<N>` lengths and the S-D6 hashes updated in one commit whose message says the `.spv` moved
and why. What is **not** acceptable is landing S1 without noticing which of the two happened.

### S-D11 — tiling is `frac` **inside the sub-rect**, it belongs to S5, and the nine sub-quads are **added to** the background rect, not substituted for it

*(added 2026-08-21 at the S4 pre-build audit — three findings with one root, so one decision)*

**(1) The mechanism.** S-D7 assumed tiling = "let the UV run past `[0,1]` and let `REPEAT` wrap". That
is not available and never was: S3 landed the UI's own sampler as `AddressMode::ClampToEdge` in
**both** `UiSamplerMode` variants (`resources.rs:310`), and the fragment shader samples the bindless
texture *through it* (`ui_rect.fs.hlsl:198`), so the bindless set's `REPEAT` sampler is not on the UI's
path at all. A UV of 1.5 under the landed pipeline reads the edge texel — a smear, not a tile.

**Decision: `Tile` is a fragment-side `frac` applied to the sprite's normalized position within its
own sub-rect**, selected by a new `FLAG_TILED` bit out of the free bits 5..19 (S-D2's budget), ~~with
the tile count folded into `uv` at pack exactly as S4 originally proposed. Concretely the sprite
branch computes `uv = sub_min + frac(t) * (sub_max - sub_min)` instead of
`uv = lerp(sub_min, sub_max, t)`.~~ **with the repeat count in `flags` bits 6..=19 and the sprite
branch computing `uv = sub_min + frac(t * tiles) * (sub_max - sub_min)`.** *(the mechanism's two
concrete clauses REFUTED and replaced 2026-08-21 at the S5 pre-build audit — **S-D15**. `t` is the
0..1 quad corner, so `frac(t) == t` on every covered fragment and the ruled expression was
bit-identical to the `lerp` it claimed to replace: `Tile` would have rendered as `Stretch`. And
folding the count into `uv` — a clause carried over from the `REPEAT` mechanism this decision
retired, where it worked because `REPEAT` wrapped at the texture boundary — makes `frac` sweep N
whole frames under the new wrap, reproducing the sheet bleed this decision claims to dissolve. The
DIRECTION of the decision survives intact and is vindicated: tiling really is one fragment-side
`frac` inside the sub-rect, it really does dissolve S-D7, and no sampler change is needed. Only the
two arithmetic clauses were wrong.)*

**Reason, and it is the interesting half: this DISSOLVES S-D7 rather than implementing it.** The sheet
hazard S-D7 designed a `debug_assert!` + release clamp + diagnostic counter around exists only because
`REPEAT` wraps to the *whole texture*. `frac` inside the sub-rect wraps to the **sub-rect**, which is
precisely a sheet frame. So a tiled nine-slice over a sheet frame is not a hard error — it is the
correct picture, for free. A guard, a clamp, a counter and a gate all disappear, and the thing they
were guarding becomes expressible. *(A counter that can only ever read zero is this campaign's
dead-datum class; S-D7's would have been one by construction.)*

**Rejected:** (a) *a second UI sampler with `REPEAT`, selected per draw* — needs either two pipelines
or a per-instance address-mode index; the latter IS S7's deferred per-sprite sampler lever (bit 4
reserved), and it is the wrong lever, because tiling under `frac` needs no sampler change at all.
(b) *`REPEAT` as the UI sampler's default* — `resources.rs:307-309` already refused this in S3, on the
grounds that S4's tiled nine-slice "does not get to set the default" for every glyph and every sprite;
that refusal stands and is now vindicated, since the caller it was protecting the default from does not
need the default changed.

**(2) The rung.** **`Tile` lands at S5, not S4.** Not because S4 cannot afford it, but because S4
cannot *gate* it: the mechanism is a **shader** change (an eDSL leaf, a re-emit, a re-DXC, ~~two
`SpirvBlob<N>` lengths, two manifest rows~~ **ONE `SpirvBlob<N>` length and zero new manifest
rows — plus a `frac` primitive the eDSL does not have and a NEW leaf, since none of `ui.rs`'s six
touches the sprite `uv`; corrected 2026-08-21 — S-D15 (4). The cost estimate was wrong in both
directions at once, and the argument it supports is unaffected: a shader rung is a shader rung.**)
and S4 is otherwise a pure CPU rung — that "no shader
change" property is D8d's entire argument for CPU expansion over Bevy's separate-pipeline strategy,
and it should not be spent on the half of `mode` nothing yet asks for. S5 already owns sprite sub-rect
arithmetic (`UiSheet.inset_uv`, the frame rect from `(cols, rows, index)`), which is the same
arithmetic `frac`-in-sub-rect needs, so the two are one shader edit at S5 and two at S4+S5.

**What S4 lands anyway, so S5 widens rather than re-specifies:** the `mode: u8` field exists at S4
with **exactly one legal value** (`NineSliceMode::Stretch = 0`), a `const` assert pinning the variant
count, and a rejection path for an out-of-range discriminant. The byte layout does not move at S5;
only the set of accepted values grows.

**(3) The record count — D4 and D8d disagreed, and D4 wins.** `UI-ADVANCED-ARCHITECTURE.md:620`
(D8d) says the nine sub-quads REPLACE the node's single record ("present ⇒ 9; absent ⇒ 1");
`:250` (D4) lists "background rect → nine-slice sub-quads → image" as *distinct* elements, i.e. the
sub-quads are ADDED. A third number is already written into the tree
(`tests/ui_s0_seam.rs:245`, "seven more sub-quads"). Three readings, three different values for
`UI_RECORDS_PER_NODE`, and the rung is unbuildable until one wins.

**Decision: ADD. The background rect is always sub 0 and keeps `UiBackground`'s colour, border and
corner radius verbatim, exactly as it packs today; the nine-slice sub-quads are pure textured rects
with zero radius and zero border.**

**Reason.** Not economy — *correctness*. A nine-slice source is a **frame**, and frames have
transparent regions (a rounded window chrome is the canonical case). Under REPLACE, a translucent
corner would composite against whatever is behind the entire UI instead of against the node's own
background — the background rect is not redundant overdraw, it is the surface the frame sits on. This
is also why Bevy, Godot and Unity all keep the node's own background beneath the slice. REPLACE has a
second cost that ADD does not pay at all: it would force S4 to decide how one node's `corner_radius`
and `border_width` distribute across nine sub-quads (does a 4 px radius mean 4 px on TL's *outer*
corner only? what happens to `border_width` on the shared interior edges?) — a real visual question
with no cheap answer, gated by nothing S4 has. Under ADD the sub-quads are uniform: one new pack
function, zero radius, zero border, the same shape `pack_ui_image_instance` already has.

**Consequence, stated so no gate has to guess:** `UI_RECORDS_PER_NODE = 11` — sub **0** the background
rect, subs **1..=9** the nine-slice regions in D4's TL..BR order (sub **5** is the centre, emitted iff
`fill_center == true`), sub **10** the image. ~~A nine-sliced node emits **10** records (9 with
`fill_center == false`), **11** with an image (10 without the centre).~~ **A nine-sliced node emits
**10** records (9 with `fill_center == false`) — the sub-10 image record is SUPPRESSED, because the
slices ARE the image. See S-D12 (1).** A node with no `UiNineSlice`
emits exactly what it emits today — 1, or 2 with an image — so S-D8's default-OFF row holds byte for
byte. `UI-ADVANCED-ARCHITECTURE.md:620-621` owes the same strike-and-correct; this plan is the
authority until it gets it.

*(corrected 2026-08-21 by **S-D12 (1)**. The struck sentence is where this decision over-reached: the
paragraph above it argues, correctly, that the **BACKGROUND** record must survive the slices — and
then silently generalizes the word "ADD" to the **IMAGE** record, which is a different question with
the opposite answer. Adding the image back on top of the slices paints the whole node rect at the
whole authored UV, **over** the nine regions that were just placed, and an opaque source under
`PREMULTIPLIED_ALPHA` replaces what is under it — so this arithmetic renders a plain stretched sprite
and G4-3 would have pinned it. The reason ADD gives — "the background is the surface the frame sits
on" — never mentions the image, and could not have, since it was written about the other record.)*

### S-D12 — a nine-sliced node's slices **are** its image: sub 10 is suppressed, the source split is an authored `border_uv`, and a slice with no texture is a structural skip

*(added 2026-08-21 — the **second** S4 pre-build ruling. S-D11 amended the rung; the implementer then
refused to build it and was right, and an adversarial pass confirmed the refusal and found more. Four
questions, one root: **S-D11 (3) ruled on the BACKGROUND record and generalized to the IMAGE record**,
and everything downstream — a golden that could not fail, two reds that could not fire, an
unspecified source split — follows from that one over-reach.)*

#### (1) `UiNineSlice` SUPPRESSES the sub-10 image record — the slices ARE the image, sliced

**Decision.** When a node carries **both** `UiNineSlice` and `UiImage`, the image is drawn **sliced**:
the nine sub-quads at subs 1..=9 are the whole of its rendering, and **sub 10 is not emitted**. A
nine-sliced node emits **10** records (9 when `fill_center == false`). `UI_RECORDS_PER_NODE = 11`
stays exactly as ruled, now explicitly as the **stride** of the `(node, sub)` code rather than as a
per-node emission count — the sub space is a fixed layout with a hole in it, which costs nothing
because the key push only pushes codes for records that exist and the decode is `append % 11`.

**The complete truth table, so no gate and no pack arm has to infer it:**

| `UiNineSlice` | `UiImage` | emits | subs |
|---|---|---|---|
| absent | absent | 1 | 0 |
| absent | **present** | 2 | 0, 10 |
| **present** | absent | **1** | 0 — see (3) |
| **present** | **present** | **10** (9 without the centre) | 0, 1..=9 |

**Reason.** Nine-slicing is a **rendering MODE of an image**, not a layer added on top of one. That is
what all three shipped implementations are: Unity's `Image` with `type = Sliced` draws the sprite as
nine quads *instead of* one; Godot's `NinePatchRect` draws its texture as nine patches; Bevy's
`ImageNode` under `NodeImageMode::Sliced` slices the same image. **None of them draws the image twice.**
What they all *do* keep underneath is the node's own background (Bevy's `BackgroundColor`, Godot's
`StyleBox`) — which is precisely the half S-D11 got right and must stay.

The tree makes the alternative not merely wasteful but self-cancelling, and every step was verified at
source rather than reported:

* the only texture a sub-quad can sample is the node's own `UiImage` — `UiNineSlice` carries no slot
  and no UV (Lands item 1), and `UiImage` is the sole texture datum on a node (`components.rs:440-449`);
* the image record is the **whole node rect at the whole authored sub-rect** — `pack_ui_image_instance`
  writes `input.rect` verbatim with `uv: image.uv` (`pack.rs:233-243`), pinned green by
  `ui_pack_cpu::sprite_record_mirrors_the_geometry_and_keeps_its_uv_unfolded` (`ui_pack_cpu.rs:415-416`);
* it paints **last** — both loops emit in ascending `append` (`upload.rs:364-374`, `:456-467`);
* an opaque source **replaces** the destination — `resources.rs:438` is `BlendState::PREMULTIPLIED_ALPHA`
  = `src + dst*(1-src_a)` (`boyko_rhi/src/enums.rs:907-914`), and the FS sprite branch returns
  `float4(t.rgb*t.a, t.a) * tint` (`ui_rect.fs.hlsl:195-209`), so an opaque texel under an opaque tint
  emits alpha 1.

So under ADD-as-ruled, on G4-3's own scene, the nine slices cover 9 216 px and sub 10 covers the same
9 216 px on top: **the golden would have blessed a plain stretched sprite**, M4-b would have moved
~~16×16~~ **16×24** corners to 32×32 *underneath it* *(number corrected 2026-08-21 — S-D13 (5)(1):
this paragraph computed from `[16,16,16,16]`, the border (2) below replaced with `[16,24,16,24]` in
the same ruling)*, and M4-e would have permuted source UVs *underneath it*. One
gate that cannot fail plus two reds that cannot fire — produced by the amendment written to remove
exactly that class.

**Rejected: keep ADD and make the source transparent so the slices show through.** It does not work
and the reason is worth recording, because it is the escape that looks like it should. A *fully*
transparent source hides the slices too — they sample the same texture. A *partially* transparent one
blesses a blend of stretched-over-sliced, in which a wrong slice is a wrong contribution to a
composite rather than a wrong region, and M4-b's ~~3 000~~ **2 560** px delta becomes a fraction of
itself scaled by `1 - src_a` *(number corrected 2026-08-21 — S-D13 (5)(1); same stale border)*.
Neither branch gives G4-3 a picture that means "slicing works". And a *different* texture
for the slices does not exist at S4: it would need a second slot, which is exactly the datum (1) is
about.

**Rejected: hand-pack nine `UiInstance`s in the gate and never construct a node.** That is the
self-gating defect this rung's own audit flagged one row later for G4-4 (`:1019` — a test that
re-implements the expansion policy gates itself). It was named as a defect once; it may not be adopted
as a fix now.

**The trap, named and answered: what about a node that wants a nine-sliced frame AND an unsliced
image?** It is not a node — it is **two**. A window chrome with an icon inside it is a nine-sliced
parent with an imaged child, which is how Unity (`Image` + child `Image`), Godot (`NinePatchRect` +
child `TextureRect`) and Bevy all express it, and this engine already has the hierarchy: the gather is
a DFS over `Children` and a child carries its own `StackIndex` and clip. So the case is expressible
today, with no new datum, no flag, and no fourth combination in the table above. This is also the
engine's own rule rather than an import — **capability is component presence**: `UiNineSlice`'s
presence *is* the statement "draw my image sliced", and an author who wants it unsliced removes the
component, exactly as an author who wants no image removes `UiImage`. A `draw_image_too: bool` on
`UiNineSlice` would be a runtime flag re-expressing what presence already says, and would put the
occluding record back behind a default.

**Consequences.** Record count 10 (9 without the centre), stride 11 unchanged, `UI_STAGING_ROWS`
derivation unchanged (see (4) for the threshold it moves), probe census unchanged (Lands item 6's
6.00 → 7.00 is about the pack-input **list**, not the record stream), S-D8's default-OFF row
**byte-identical** — the first two rows of the truth table are S3's behaviour verbatim. D4's order is
preserved and not weakened: the image term is simply absent when slicing is on, the way a rect-only
node has no image term.

#### (2) The source-side split is an authored `border_uv: [f32; 4]`, in fractions of the **current** sub-rect

**Decision.** `UiNineSlice` gains **`border_uv: [f32; 4]`** — the source inset, per side, as a
**fraction of the node's current UV sub-rect**, in the same side order as `border_px`. The component
becomes **36 B**. Equal thirds is its `Default`, not its rule.

**Both sides' orders are stated here because neither was, and a symmetric example cannot reveal one:**
`border_px` and `border_uv` are both **`[l, t, r, b]`**, matching `PackInput::border_width`'s
documented order (`pack.rs:55-57`) rather than `corner_radius`'s `tl, tr, br, bl`. Subs 1..=9 are
**row-major**: TL, T, TR, L, **C (sub 5)**, R, BL, B, BR.

**Reason.** The rule has to come from data the node carries, and the texture's texel size is not
merely out of reach at S4 — **the engine never records it at all**. `BindlessTextureTable::register`
takes a bare `VkImageView` (`boyko_render/src/bindless.rs:287`) and the table holds
`{ set, error_texture, allocator }` (`:217-221`) with no dimension map, and it is a `NonSendResource`
(`:223`) besides. `UiImage` is `{ texture, uv_min, uv_max, tint }` — no size (`components.rs:440-449`).
So Unity's and Godot's shape (border in **source texels**, engine supplies the size) is unavailable
here and stays unavailable until something introduces a texture-dimension column; it is not an S4
scoping problem. A normalized inset needs no dimensions by construction.

**Fractions of the sub-rect, not absolute UVs, and this is the load-bearing half.** At S5 the sub-rect
becomes a sheet frame that *changes every flipbook tick*; an absolute UV inset would be wrong on every
frame but one, while a fraction of the current sub-rect is frame-invariant and composes with S5's
`(cols, rows, index)` arithmetic for free. It is the same property S-D11 (1) found for `frac`: **wrap
and inset both belong to the sub-rect, because the sub-rect is what a frame is.**

**Rejected: (a) equal thirds as the RULE** (split the `uv` rect into three equal parts per axis). It is
exact for G4-3's 3×3 source, needs no new datum, and makes M4-b and M4-e fire — which is why it was
the implementer's only candidate. It is also simply wrong for the canonical case the feature exists to
serve: a 32×32 chrome with an 8 px border wants 1/4, and a 64×64 panel with a 6 px border wants 3/32.
A rule that is right only for sources whose cells happen to be thirds is a rule that will be
discovered wrong by an author, not by a gate. **It survives as the `Default`**, so the
zero-configuration case is exactly (a) and G4-3 needs no extra authoring — the generality costs the
author nothing until they need it.

**Rejected: (c) an authored texel border plus an authored source size** (`border_src_px` +
`source_px: [f32;2]`, Unity's shape with the author carrying the size). +8 B over (b) to store a datum
the author can get wrong and nothing can check — a second spelling of the texture's size, silently
stale the moment the texture is swapped. That is the dead/wrong-datum class this campaign keeps
finding, bought at a higher price than (b).

**Rejected: the degenerate identity** (source fractions = destination fractions). Recorded because it
is the one that compiles and looks plausible: it makes slicing an exact no-op — every sub-quad samples
the region it covers — so M4-b cannot fire and the whole rung is a 10× more expensive way to draw a
stretched sprite.

**Validity, and it is part of the decision rather than an implementation detail** — a split that
inverts produces a negative-extent UV rect, which is a wrong picture with no diagnosis:

* **Source:** `border_uv[0] + border_uv[2] < 1.0` and `border_uv[1] + border_uv[3] < 1.0`, each side in
  `[0, 1)`. `debug_assert!` at pack; in release the offending axis's two sides are scaled down
  proportionally so the sum is `< 1` — the centre source region degenerates to zero width rather than
  inverting. The house pattern S-D7 stated and this rung keeps: loud in dev, visibly-but-safely
  degraded in release.
* **Destination:** the twin case `border_px[0] + border_px[2] > rect.w` (a nine-sliced node scaled
  below its own border) gets the **same** proportional shrink, per axis, which is what Unity and Godot
  both do. It is not optional: a 96×96 chrome animated to 8×8 is an ordinary tween, and without the
  shrink its corners overlap and its edges invert.

> **AMENDED at the post-landing audit, 2026-08-21 — the domain has TWO edges and only one had a
> remedy.** The two bullets above rule the SUM edge, and the implementation matched them exactly:
> `split_axis`'s only guard was `sum > extent && sum > 0.0`. That guard **cannot fire for a negative
> side** — `-0.5 + 0.25 = -0.25` is under any positive extent — so the other edge of the same stated
> domain ("each side in `[0, 1)`", non-negative `border_px`) fell straight through it. MEASURED in
> `--release` with `border_uv = [-0.5, 0.25, 0.25, 0.25]` on the `UV = [0.25, 0.5, 0.75, 1.0]` scene:
> TL's `uv` came out `[0.25, 0.5, 0.0, 0.625]` — **`u1 < u0`, the negative-extent UV rect this whole
> section exists to forbid** — with the centre's u-extent wider than the entire `UiImage` sub-rect it
> is a fraction of, and `border_px = [-8.0, …]` giving TL a `size_px` of `[-8.0, 24.0]`.
>
> **Ruled: the CODE moves, not the ruling.** A negative inset is not a proportion of anything, so the
> proportional shrink is the wrong remedy for it; `split_axis` now **clamps each side at zero before**
> the shrink. Both edges then land on the one guarantee this section actually states — *degenerate,
> never invert*. Pinned by `ui_s4_nine_slice.rs`'s
> `s_d12_2_a_negative_inset_degenerates_in_release_instead_of_inverting`, which is
> `#[cfg(not(debug_assertions))]` because in a debug build the `debug_assert!` fires first and the
> split is never reached — **the ruled release behaviour was ungatable by construction until a
> release-only test existed, which is why the gap survived the landing audit's own green run.**
>
> *(Doc-rot footnote, because the shape is this campaign's recurring one: `pack.rs`'s field doc had
> generalized the true, narrow sentence in `boyko_ui`'s `UiNineSlice` — "an axis whose sides **sum to
> 1 or more**" — into "an **out-of-domain** axis is scaled down proportionally", which was false the
> day it was written. The twin pair diverged inside one rung, on the same day, and the false half was
> the one on the implementation's side.)*

**Size, MEASURED not asserted** (rustc 1.97.1, the precedent set by ledger row 10 — both spellings
compiled):

```
{ border_px: [f32;4], mode, fill_center, _pad: [u8;2] }                 size 20  align 4   (as ruled)
{ border_px: [f32;4], border_uv: [f32;4], mode, fill_center, _pad }     size 36  align 4   ← S-D12
{ border_px: [f32;4], border_uv: [f32;4], mode, fill_center }           size 36  align 4
offsets: border_px @0, border_uv @16, mode @32, fill_center @33, _pad @34
```

The two trailing bytes are **again** implicit tail padding when unspelled — the same finding as ledger
row 10, one field wider — so `_pad: [u8; 2]` stays spelled. **+16 B on a cold, authored, table-storage
component, and ZERO GPU bytes**: the split is resolved at pack into each sub-quad's `uv`, so
`UiInstance` does not move and D1's 80 B stands.

#### (3) A nine-sliced node with **no** `UiImage` emits its background and nothing else

**Decision.** `UiNineSlice` without `UiImage` emits **no sub-quads at all** — the node packs exactly
what it packs today, its background rect at sub 0. `UiNineSlice` alone is a **no-op**, not nine
invisible quads.

**Reason.** It is the engine's own structural-skip rule, and S3 already spells it one line into the
pack it would reuse: `let image = input.image?;` (`pack.rs:205`) — absence is the skip, never a flag
and never an empty record. Under (1) the slices sample the node's `UiImage`; with no image there is no
texture, no source rect, and nothing for nine quads to be. Emitting them anyway costs nine instances,
nine vertex-shader invocations and nine ring rows per node to draw nothing, and forces a "no texture"
branch into the slice pack that would have to invent a UV and a slot.

**It is also the correct fix for Lands item 7's release panic, and a better one than item 7 states.**
The rule is: **the key push is the sole authority on which subs exist**, and it pushes slice codes only
for `UiNineSlice.is_some() && UiImage.is_some()`. Every arm of the decode's `match append % 11` then
has its precondition established at the push, thirty lines above, and **no arm can fail for any of the
four component combinations** — rather than item 7's `match` having to carry an arm for a combination
the push can still emit. No `.expect` in that loop may be reachable by any authored component set;
that is now a gate (G4-8) and a red (M4-g), because item 7 fixed a release panic and the table had no
row that constructs the node which panics.

#### (4) M4-f's threshold is **410** — not 187, and not 373 either

**Decision.** `:1074`'s "the box overflows at 187 nine-sliced imaged nodes" is wrong twice over and
becomes **410**. Computed, not asserted:

| per-node emission | first node that overflows a 4 096-row box |
|---|---|
| 11 — ADD as S-D11 ruled it | 373 (11 × 372 = 4 092 ≤ 4 096 < 4 103) |
| **10 — S-D12 (1), imaged + centre** | **410** (10 × 409 = 4 090 ≤ 4 096 < 4 100) |
| 9 — `fill_center == false` | 456 (9 × 455 = 4 095 ≤ 4 096 < 4 104) |
| 2 — S3 today | 2 049 (2 × 2 048 = 4 096 **exactly**) |

**Reason.** `187 = ceil(2048 / 11)` — the **node** budget divided by the stride, where the arithmetic
called for the **row** budget. Correcting it under S-D11 gives 373; correcting it under (1), where a
nine-sliced node emits 10 records rather than 11, gives **410**, and the two corrections are recorded
together so the record shows which ruling moved the number. The red is unaffected either way: G4-6
drives `UI_MAX_NODES = 2048` nine-sliced imaged nodes = 20 480 records, five times over a 4 096-row
box. Only the explanatory number was wrong — but "a number asserted rather than measured" is a class
this campaign has been bitten by, and the last row of the table is why it matters here: **today's box
is not 2× the measurement scene, it is exactly 1×**, overflowing at node 2 049 with zero margin. That
was ledger row 12's finding and this table confirms it arithmetically.

**Consequence for Lands item 8: the derivation stays `UI_MAX_NODES * UI_RECORDS_PER_NODE` = 2 048 × 11
= 22 528 rows = 1.72 MiB**, even though the true worst case is now 10/node (20 480 rows, 1.56 MiB).
The 160 KiB of slack is deliberate: the constant is derived from the **stride**, so it cannot go stale
when a later rung adds a sub code, whereas a constant derived from today's maximum emission is a
number that must be re-audited every time the truth table in (1) gains a row. A budget that cannot
overflow within the stated node count is worth 160 KiB of host RAM — the same argument item 8 already
makes for paying 1.4 MiB over the old box.

#### In-tree comments that repeat the struck arithmetic

Three of them, corrected in the **same** commit as the code (the doc-rot-repair rule: a claim is swept
wherever it is repeated, not only where it was noticed):

* `pack.rs:177-185` — `UI_RECORDS_PER_NODE`'s doc, including the sentence ledger row 5 already
  refuted ("S4's nine-slice raises this constant; nothing else changes at that call site");
* `upload.rs:320-328` — "a node emits up to `UI_RECORDS_PER_NODE` records (its background rect, then
  its sprite quad)", which stops being the whole list at S4;
* `ui_s0_seam.rs:243-245` — "the one S4's nine-slice will extend by **seven more** sub-quads", the
  third reading ledger row 4 found in the tree. It is wrong under S-D12 as well (from an imaged node's
  2 records S4 adds eight and removes one, for 10), so it is corrected rather than left as the number
  that happens to survive.

### S-D13 — the ruling that had no red, the loop that has no caller, and ten sentences that could not be written as spelled

*(added 2026-08-21 — the **third** S4 pre-build ruling, and still before one line of S4 exists. S-D11
amended the rung; S-D12 amended the amendment; the implementer refused a **second** time and was right
a second time, and a second adversarial pass confirmed the refusal. Two blocking findings and twelve
amend-level ones. The pattern across all three rulings is worth naming once: **a ruling changes what
the rung EMITS and leaves the rung's INSTRUMENTS describing the old emission.** S-D11 ruled the record
count and left the image record occluding the slices; S-D12 ruled the occlusion and left M4-c
mutating a sequence it had just made an alternation, and left M4-b's margin computed from the border
it had just changed one row above. Both are the doc-rot-repair class this plan already warns about,
committed by the repair itself.)*

#### (1) M4-c cannot be applied, and G4-2 therefore has no red at all — it takes TWO mutations, not one

**The finding.** `:1331` reads "swap the image and the **LAST sub-quad (BR)** emission order. **G4-2**
reds." S-D12 (1)'s own truth table (`:427-432`) makes those two records **mutually exclusive**:
present/present emits subs 0, 1..=9 and no sub 10; absent/present emits subs 0, 10 and no sub-quads.
All three readings of "swap" fail, and each was checked at source:

* **Same node** — no node emits both records, so there is nothing to swap. The mutation is unwritable
  in the literal sense M4-c's own respecification used against its predecessor: "what would have been
  'observed' is that the mutation is unwritable, which is not a red" (`:1335-1336`).
* **A global code swap** (image → sub 9, BR → sub 10) — **`staged()` is byte-identical either way.**
  `gather_into_staging` sorts the key lane and packs in SORTED order (`upload.rs:364-374`), and within
  one node exactly one of {BR, image} exists, so relabelling the two codes cannot change any node's
  internal order; `pack_sort_upload` keys on `scratch.pack.len()` (`upload.rs:458-467`) and never sees
  the code at all. Cross-node interleaving is impossible because the maximum sub (10) is below the
  stride (11), so every code of node *k* precedes every code of node *k+1* at equal stack.
* **Cross-node** — G4-2's contract is D4's **per-node** order. Two nodes are ordered by
  `(stack, append)` whatever the sub codes are.

**And the mapping shows the hole is structural, not local.** Over all seven reds: M4-a→G4-1,
M4-b→G4-3, M4-c→G4-2, M4-d→G4-4, M4-e→G4-3, M4-f→G4-6, M4-g→G4-8. **G4-5 and G4-7 are named by no red
at all**, and G4-2 — which carries S-D12 (1)'s headline claim, the image record's ABSENCE — is named
by exactly one, which cannot be applied. This bullet has now died **twice**: it was respecified once
under S-D11, and S-D12's amendment list touched M4-b, M4-e, M4-f and M4-g and never it.

**Decision: M4-c becomes TWO mutations, and both are required, because they red different halves of
G4-2.**

* **M4-c1 — emit sub 10 as well on a nine-sliced imaged node** (i.e. S-D11's ADD, exactly the bug
  S-D12 (1) exists to forbid). It reds G4-2 on the record **count** — its two-node scene goes 12 → 13
  staged — and independently reds G4-3 on the hash, because sub 10 covers all 9 216 px under
  `PREMULTIPLIED_ALPHA` (`resources.rs:438`). It also reds G4-8, whose derived total goes 14 → 15.
* **M4-c2 — misassign the sub → record MAPPING**, concretely **swap the DECODE arms for sub 0 and
  sub 9** so arm 0 yields the BR slice and arm 9 yields the background. `staged()` comes back
  BR-first and background-LAST — D4's order inverted at both ends, at an unchanged record count, so
  it is a pure ORDER red. Without it, G4-2's **order** half stays unmutated even after M4-c1, which
  reds only on count and hash.
  **⚠️ The mutation must land in the DECODE (equivalently in the emitter's sub → region table), NOT
  in the key push, and this was measured rather than assumed** *(2026-08-21)*: the decode loop reads
  **nothing but the sorted key** — `staging[dst]` is a pure function of `(node, sub)`
  (`upload.rs:363-374`) — so pushing the SAME code set in a different order is normalized away by
  `keys.sort_unstable_by_key` and `staged()` comes back byte-identical. **A push-side spelling of
  this mutation is a red that cannot fire, which is the class it exists to catch.** Only three things
  can move the staged order: the pushed code SET (that is M4-c1), the sub → record mapping (this),
  or the sort key itself. *(A push-side spelling also has a second failure available: the loose first
  wording "background → 9, TL → 0" leaves BR at 9 as well, and two records sharing one code is
  M4-a's failure — duplication and loss, since `append` is the SOURCE ADDRESS — which reds G4-1, not
  G4-2.)* **This mutation also trips G4-1's contract-order clause**, which is expected and not a
  duplication: S-D12 moved G4-1 onto the same CONSEQUENCE in `staged()`, so the two rows overlap on
  order by construction and differ on what else they assert (G4-1: count, consecutiveness,
  inheritance; G4-2: the by-name kind sequence across two nodes and sub 10's absence).

**And a claim inside the ruling is corrected: `:1295`'s "otherwise pinned nowhere" is FALSE.** The
absence of sub 10 is **doubly** asserted — by G4-2 and by G4-8, whose derived total `1 + 2 + 1 + 10`
= 14 becomes 15 with one extra record — and, until M4-c1, **zero times mutated**. A property asserted
twice and mutated never is this campaign's headline shape, not a property pinned nowhere. The two
statements are opposite diagnoses and the wrong one was written down.

**Separately: G4-2's cited instrument is blind to the property it is cited for.** The row points at
"the shape `ui_s0_seam.rs:288-302` … by `FLAG_TEXTURED`". On a nine-sliced imaged node index 0 is the
untextured background and 1..=9 are nine textured slices; a wrongly-emitted sub 10 is a **tenth
textured record**, so a `FLAG_TEXTURED` prefix assertion sees an identical prefix with and without it —
only the LENGTH or the GEOMETRY separates them. That shape also hard-codes
`assert_eq!(staged.len(), 4, …)` (`ui_s0_seam.rs:289-293`), the literal G4-1 forbids one row above.
**G4-2 borrows the shape's stack-bracketing and its by-name reading, and takes its length from the
same derivation G4-1 and G4-8 use — never from that file's literal.**

#### (2) S4 does NOT expand the legacy loop — it has no caller in this workspace

**The finding, measured and re-verifiable in four greps.** `:1294` requires G4-1 to run "against BOTH
loops … and for `pack_sort_upload` via `UiRenderScratch`", and Lands item 2 requires the expansion
"in BOTH". `pack_sort_upload` takes `ctx: &mut RhiContext` and ends in `ctx.ui_upload(..)`
(`upload.rs:432-440`, `:474`), so it cannot be driven device-free — but that is the symptom. The
disease:

* `pack_sort_upload`'s only non-doc caller is `host_upload_frame` (`upload.rs:526`);
* `host_upload_frame`'s only non-doc occurrence in the entire `crates/` tree **is its own definition**
  at `upload.rs:509`. Every other hit is a doc comment, and two of those (`upload.rs:39`,
  `dispatcher_token.rs:531`) describe the already-DELETED `host_upload_frame_from_world`;
* the function name is not re-exported from `crates/boyko_render/src/lib.rs` or `src/ui/mod.rs` —
  it is reachable only as an inherent method on the re-exported `UiUploadSystem`, i.e. it is public
  API with zero in-workspace callers;
* the only mention of `UiUploadSystem` outside `boyko_render` is a doc comment
  (`boyko_ecs/src/ecs/core/system/dispatcher_token.rs:531`).

So the legacy loop is **reachable from nowhere in this workspace**. It is the surviving half of the
path **S0 already replaced** with the two-phase seam: its world-facing sibling
`host_upload_frame_from_world` was deleted at S0 for having no possible caller, and this half stayed.

**Decision: S4 does not expand `pack_sort_upload`.** Lands item 2's "in BOTH" is struck with this
reason, and G4-1's `pack_sort_upload` leg is deleted with this reason. Expanding an unreachable loop
adds untested code and manufactures a gate that cannot be run — both of this campaign's headline
classes, in one edit. The expansion lands in `gather_into_staging`, the loop the scheduler runs.

**Consequence for G4-4 and M4-d, ruled here so the next reader does not find a third contradiction.**
`UiRenderScratch` is production-filled only through `pack_sort_upload`, so after this ruling no
production path puts a nine-slice record into it. G4-4's amended instruction — "driving the expansion
through the production emitter rather than the test's own hand-rolled loop" (`:1297`) — is **still
writable and still means what it said**, because what S4 exposes is a **loop-agnostic emitter**: one
free function that appends a node's records into a caller-supplied sink, called by
`gather_into_staging` writing into `staging[dst..]` and by G4-4 writing into a `UiRenderScratch` the
test owns (which is exactly what `ui_no_realloc.rs` already owns today —
`UiRenderScratch::default()` allocates nothing, `pack.rs:278-287`). **What G4-4 may NOT do is call
`pack_sort_upload`**, and it does not need to.

**Filed, not decided — this is the owner's SCOPE call.** Should `host_upload_frame` +
`pack_sort_upload` be deleted outright? That removes public API. The four measurements above go to
`docs/OPEN-QUESTIONS.md` and its `docs/ru/` twin in the same commit. S4 is not blocked on the answer;
it is blocked on not pretending the loop is live.

#### (3) The derivation instruction G4-1 gives has no formula, and G4-8 already breaks it

`:1294` says derive the count from `UI_RECORDS_PER_NODE` and `fill_center`, "never a literal". But
S-D12 (1) **severed the stride from the emission**: the stride is 11 and the emissions are 10 / 9 /
2 / 1, and **no expression in (stride, `fill_center`) yields 2** for the imaged row.
`UI_RECORDS_PER_NODE - 1` gives 10 only by the accident that exactly one of {centre, image} is
dropped. There is no region constant in the tree — `UI_RECORDS_PER_NODE` (`pack.rs:185`) is the only
one. And **G4-8 one row later already breaks the instruction**, asserting "(1 + 2 + 1 + 10)" — four
literals (`:1301`).

**Decision: mint the constants the derivation needs**, beside `UI_RECORDS_PER_NODE` and in the same
edit:

```
UI_NINE_SLICE_REGIONS: u32 = 9;   // TL,T,TR,L,C,R,BL,B,BR — the sub space 1..=9
UI_NINE_SLICE_SUB_BASE: u32 = 1;  // the first slice's sub code
UI_IMAGE_SUB: u32 = 10;           // the image record's sub code
```

with `UI_RECORDS_PER_NODE = UI_IMAGE_SUB + 1` — so the stride is DERIVED from the largest sub code
and cannot drift from it, which is the one relation S-D12 (1)'s hole made non-obvious. **The rule
every gate then satisfies, stated once so a literal is a review finding rather than a matter of
taste:** a gate may write a literal only for the *component combination* it constructs (the truth
table is authored data, not arithmetic); every *record count* is an expression over the three
constants above plus `fill_center`. G4-8's total becomes
`1 + 2 + 1 + (1 + UI_NINE_SLICE_REGIONS)` and moves by itself when a sub code is added.

#### (4) Six instrument sentences that cannot be written as spelled

Each was verified at the site named; each gets its correction in the row it belongs to.

1. **G4-1 asserts a `StackIndex` that `UiInstance` does not carry.** Its complete field set is
   `min_px, size_px, clip, corner_radius, uv, color, border_color, border_width, flags`
   (`upload.rs:110-120`); `stack` lives on `UiNode` (`:99-101`), is pushed into the private key lane
   and is consumed as the sort key only. This is the SAME shape S-D12 struck from this very row for
   the `append` codes, one clause later in the same sentence. **Observable only as a consequence**,
   by stack-bracketing — a plain node at a lower stack and another at a higher one, so a slice that
   lost its parent's stack lands outside the block (`ui_s0_seam.rs:255-281` is the shape).
2. **G4-3 states no sampler mode, and "samples only its own source cell" is FALSE under the
   default.** `UiSamplerMode::Smooth` is `Filter::Linear` and is `#[default]`
   (`resources.rs:101-119`); the existing sprite golden runs Smooth as its primary leg
   (`ui_sprite_gpu_golden.rs:442-444`) and Pixel only as a reachability leg (`:473-474`). Magnifying
   a 3-texel axis 32× under Linear blends into the neighbouring cell past each cell's texel centre,
   so under the default the sentence is false for every pixel outside the inner half of a cell.
   **G4-3 runs `UiSamplerMode::Pixel`**, which makes the assertion true as written and keeps the
   golden's meaning ("this region came from that cell") independent of a filter kernel.
3. **G4-5's discriminant half is unwritable if `PackInput` carries the enum** — an out-of-range
   discriminant would need a `transmute` into a one-variant enum, which is instant UB and cannot be
   a gate. The working in-tree precedent is `UiImageInput.slot`: a raw `u32` validated by a
   `debug_assert!` at the pack boundary (`pack.rs:31-33`, `:212-217`). **`PackInput` carries
   `nine_slice: Option<UiNineSliceInput>` whose `mode` is a raw `u8`**, `debug_assert!`ed
   `< UI_NINE_SLICE_MODE_COUNT` at the pack; the typed `NineSliceMode` stays the AUTHORED component's
   field, where the type system already forbids the out-of-range value.
4. **G4-5's first half and Lands item 1 are unwritable as spelled too.** Both prescribe "a
   variant-count `const` assert" (`:1160`, `:1298`). **MEASURED on 1.97.1**: `std::mem::variant_count`
   is `E0658` (nightly-only, issue #73662) **and** "not yet stable as a const fn" — two errors, one
   line. The stable spelling, measured green and measured RED:
   `const _: () = match NineSliceMode::Stretch { NineSliceMode::Stretch => () };` compiles clean and
   gives `error[E0004]: non-exhaustive patterns: NineSliceMode::Tile not covered` the moment `Tile`
   is added. **Write it WITHOUT the outer braces** — measured: the braced form
   `const _: () = { match … };` emits `unused_braces`, which the project's own
   `clippy --all-targets -- -D warnings` gate turns into an error.
5. **`border_px`'s `Default` is unstated** while `border_uv`'s is ruled (`:1170`). Under S-D12 (1)
   presence SUPPRESSES the image, so `UiNineSlice::default()` on an imaged node does **not** degrade
   to S3's picture: with `border_px = [0;4]` every corner and edge has zero destination extent and
   only the centre sub-quad is visible, whose source is the middle third of each axis — the node
   renders **the middle ninth of its texture, zoomed to fill**. S-D12 (2)'s validity domain
   (`:540-551`) passes it as legal: `0 + 0 > rect.w` is false, so no shrink fires.
   **Decision: `border_px`'s `Default` is `[0.0; 4]` and the degenerate picture is ACCEPTED, not
   guarded.** It is the same shape as `UiImage`'s alpha-0 default tint (`pack.rs:186-190`): the
   zero-configuration value of an authored component is the null one, and the author sees the result
   immediately. A non-zero default would be a magic number no gate could justify, and a `debug_assert!`
   against zero would forbid the legal "slice the source but not the destination" case. **Stated in
   the field's doc comment**, because an unstated degenerate default is exactly the datum an author
   discovers by acting on it.
6. **M4-d's new upper bound cannot fire on the scene G4-4 rules.** `ui_no_realloc.rs` is `N = 4096`
   (`:102`, `:191`). Nine-sliced + imaged ⇒ `emitted = 40 960` and `2 × emitted = 81 920`; an 11×
   reserve is `4 096 × 11 = 45 056 < 81 920`, so `assert!(cap < 2 * emitted)` **passes**. It is
   structural, not a coincidence of this N: a reserve of 11/node can never exceed 2× an emission of
   10/node. **Decision: the upper bound lives on the file's EXISTING rect-only frame**
   (`ui_render_scratch_does_not_realloc_in_steady_state`, `:100-140`), where `emitted = N = 4 096`,
   `2 × emitted = 8 192` and the 45 056-row reserve overshoots it 5.5× — and M4-d is applied at the
   scratch's setup, which today allocates nothing at all (`UiRenderScratch::default()`,
   `pack.rs:278-287`). The nine-sliced frame G4-4 adds carries the count and consecutiveness half; it
   does not carry this line.

#### (5) Four claims that are true but stated wrongly, and one number that went stale in the last repair

1. **M4-b's margin is stale — and by the repair that fixed the row above it.** `:1325` computes from
   `border_px = [16,16,16,16]`, which S-D12 (2) replaced with `[16, 24, 16, 24]` **one row earlier in
   the same table**. Under the ruled border a correct corner is 16 × 24 = **384 px** (not 256), four
   correct corners are **1 536 px**, four equal-thirds corners are 32 × 32 × 4 = **4 096 px**, and the
   delta is **2 560 of 9 216** — not "~3 000". The red still fires; only the number was wrong. This is
   the doc-rot-repair class, committed by the ruling that repaired the sentence above it, and the
   same stale arithmetic is repeated twice inside S-D12 (1) itself (`:457`, `:465`) — all three sites
   are corrected in this edit, which is the point of the rule.
2. **G4-2 and §6 still name the struck noun.** `:1295` "assert the `append` lane's order" and `:1624`
   "extend the `append`-lane order assertion (G4-2's shape)" — the phrase S-D12 removed from G4-1 one
   row earlier as unobservable (`UiUploadSystem.keys` is private, `upload.rs:160`). G4-2 names the
   right instrument (`sys.staged()`) so it is writable, but the wording propagates the wrong claim
   into the Interaction plan, which inherits the obligation. Both become **"the STAGED order"**.
3. **G4-3 names no pack loop**, in a table whose preamble said every row states which one
   (`:1288-1290`). ~~It is the one row that does.~~ *(census corrected 2026-08-21 at LANDING —
   S-D14 (3): THREE of the eight rows drive no pack loop. G4-5 is a `const` match plus a CPU test at
   the pack BOUNDARY — `pack_ui_*`, not a loop — and G4-7 drives `ui_render_discovery` and the probe
   census. The preamble over-claimed and is narrowed there; the re-pointing of G4-3 below stands
   unchanged, because G4-3 is a PICTURE and a picture has to come from somewhere.)* The only nine-slice-shaped golden precedent hand-packs
   (`ui_sprite_gpu_golden.rs:171-176` builds its `UiInstance`s from `pack_ui_image_instance` with no
   node and no gather) — the construction S-D12 (1) explicitly rejects at `:470-473`. **G4-3 drives
   `gather_into_staging`** and uploads `sys.staged()`, so the picture it pins is the one the
   scheduler's own loop produced.
4. **The "does not compile" claim about G4-7 is imprecise, and the imprecision is measurable.**
   `:1300` says adding `UiNineSlice` to `ui_pack_inputs!` without a `PackInput` variant "does not
   compile". Two different edits, two different outcomes, both structural in the file as it stands:
   adding to the macro **and** to the test's `PackInput` enum + `ALL` **without match arms** gives
   `error[E0004]` **twice** — the enum has two exhaustive matches, `name()` and `mutate_pack_input`
   (`ui_s0_discovery.rs:195-227`, `:228-...`) and neither has a catch-all; adding to the macro
   **only** compiles and reds at RUNTIME on `assert_eq!(PackInput::ALL.len(),
   ui_pack_inputs!(count), …)` (`:265-274`) with a message naming the three places to add it. **Both
   are stated, and which edit each protects against is stated**: the compile error catches "declared
   but never driven", the runtime assert catches "added to the macro but not to this test". The
   property is gated either way — this is precision, not a hole.

#### (6) The escalation channel says RESOLVED while the rung is blocked a second time

`docs/OPEN-QUESTIONS.md:19` and its `docs/ru/` twin both open the S4 entry with "**RESOLVED**
2026-08-21 … S-D12", and neither records the two blockers found after that ruling. The twin is in
sync item-for-item, so whatever lands must land in both, in the same commit. **Both are amended in
this edit**: the S4 entry's status becomes RE-OPENED-then-RESOLVED with S-D13 named, and the
`host_upload_frame` SCOPE question of (2) is filed as a new item with its four measurements.

### S-D14 — the ten corrections landing found, and the two reds that could not fire as ruled

*(added 2026-08-21 — written DURING the build, not before it. S-D11 amended the rung, S-D12 amended
the amendment, S-D13 amended that, and two implementers refused in between; this one is different in
kind, because every item below was found by RUNNING something. Two of the ten are the class all three
pre-build rulings were hunting — **a red that cannot fire** — and neither was visible to reading:
M4-c2's ruled sub pair dies in a `.expect` before the property it mutates is ever asserted, and
M4-d's ruled bound is hidden from by a buffer swap. Both were found by applying the mutation and
watching the wrong thing happen.)*

1. **`fill_center` had no ruled `Default`, and `bool::default()` falsifies the picture S-D13 (4)(5)
   ruled one field earlier.** With `border_px = [0;4]` AND `fill_center = false` a defaulted
   `UiNineSlice` on an imaged node emits its background plus eight zero-extent slices and renders
   NOTHING — the image suppressed, the only region with extent skipped. **`true`**, stated in the
   field's doc, for the same reason the sibling field's default is stated. Every record count in the
   rung already assumed it.
2. **`UI_NINE_SLICE_MODE_COUNT` was named by a ruling and minted by no Lands item.** S-D13 (4)(3)
   prescribed the `mode` `debug_assert!` against it; the identifier occurred exactly once in the whole
   plan, in that ruling. It is minted in Lands item 7 beside the sub-space constants and BOUND to
   `NineSliceMode` by the gather's exhaustive conversion match, which is `error[E0004]` when S5 adds
   `Tile`. `UI_NINE_SLICE_CENTER_SUB` and `UI_MAX_SUBS_PER_NODE` are minted in the same edit for the
   same reason: both were spelled in prose and nowhere as names.
3. **The gate table's preamble over-claimed, and S-D13 (5)(3)'s census of it was stale when written.**
   THREE of the eight rows drive no pack loop, not one. Both sites narrowed.
4. **Lands item 2's emitter shape is not writable as spelled.** "One free function appending a node's
   records into a caller-supplied sink, which `gather_into_staging` calls into `staging[dst..]`" —
   that loop neither appends nor iterates per node. It builds the whole key lane, sorts it, and writes
   ONE record per sorted key BY INDEX into a fixed `Box<[UiInstance]>`. The writable shape is the
   PER-SUB MAPPING (`ui_node_sub_codes` + `pack_ui_sub_record`), with an append wrapper over it for
   callers that append — which is also the only shape in which M4-a stays writable at the key push and
   M4-c2 has a decode to mutate.
5. **The measurement paragraph's noun was off by the probe that is not a pack input** — six pack
   inputs + `Children` = 7.00 probes, not "7 pack inputs". The figure was right.
6. **G4-3 stated no TINT, and `UiImage`'s default tint is alpha 0.** The pack premultiplies it into
   every slice and the pipeline blends `PREMULTIPLIED_ALPHA`, so under the default M4-b's whole
   2 560 px margin and M4-e's entire premise are zero — two reds disarmed on the rung's only
   device-bound row, by a value nobody wrote down. `0xFF_FF_FF_FF`, stated in the row.
7. **G4-3 could pass without comparing anything.** Its harness returns early and exits 0 on a
   GPU-less or validation-less box — the `boot_*_or_skip` false-green CLAUDE.md documents for two
   other tests. The landed file honours **`BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1`**, under which a skip
   FAILS, and the rung was run with it set.
8. **G4-8's "no panic in either build profile" had no release invocation.** The ladder's unconditional
   gate is dev-profile only, and the debug leg is strictly weaker — it additionally has
   `debug_assert!` armed, so it says nothing about the `.expect`s release keeps, which is the very
   thing Lands item 7 exists to make unreachable. The row now names both invocations and their
   expected counts.
9. **M4-d's ruled bound is A RED THAT CANNOT FIRE, and the reason is a buffer swap nobody named.**
   `assert!(scratch.pack.capacity() < 2 * emitted)` was applied against a setup-time reserve and came
   back GREEN: `UiRenderScratch::sort_by_stack` ends in `core::mem::swap(&mut self.pack, gather)`, so
   the two buffers rotate every frame and the reserve sits in `scratch.pack` on even frames and in the
   caller's `gather` on odd ones. Measured: `pack 4 096 / gather 22 528`. The bound belongs on the
   PAIR. *(The ruling's magnitude was also computed from the wrong quantity — 11× of the test's N,
   which `UiRenderScratch::default()` cannot see, since it takes no N. The natural reserve is
   `UI_MAX_NODES * UI_RECORDS_PER_NODE` = 22 528. Both exceed `2 × 4 096`, so the mutation was fine
   and the number was not.)*
10. **M4-c2's ruled sub pair fires for the WRONG REASON.** Swapping the decode arms for sub 0 and
    sub 9 sends EVERY node's sub-0 record — nodes with no `UiNineSlice` included, and G4-2's scene
    contains one by construction — into an arm that resolves `input.nine_slice` and, under S-D12 (3)'s
    deliberate `.expect`, panics before any order assertion runs. The pair is **sub 1 (TL) and sub 9
    (BR)**: total on the sliced node, no other node's decode touched, count unchanged, and it reds
    G4-2 on the per-slice `min_px` the row already reads. OBSERVED exactly so.

**One more, found by the golden's own accounting rather than by a gate**, and recorded because it is
the accounting earning its keep: the scene's first background colour was pure blue, which is
byte-identical to the 3×3 source's TR cell. The "zero background pixels" assertion counted TR's 384 px
and reported a missing slice on a picture that was CORRECT. A colour census that can confuse two
subjects is an instrument defect, not a finding; the background is olive, chosen to collide with no
cell.

### S-D15 — `Tile` needs a per-instance REPEAT COUNT; `frac(t)` alone is the identity function

*(added 2026-08-21 at the S5 pre-build audit. S-D11 (1) retired a mechanism that could not work and
replaced it with one that does nothing. Both halves of its decision sentence are refuted below, and
the second half is refuted by the FIRST half's own arithmetic.)*

**(1) `uv = sub_min + frac(t) * (sub_max - sub_min)` is bit-identical to the `lerp` it replaces.**
~~S-D11 (1): "Concretely the sprite branch computes `uv = sub_min + frac(t) * (sub_max - sub_min)`
instead of `uv = lerp(sub_min, sub_max, t)`."~~ **REFUTED at source.** `t` is `input.local_uv`, which
the VS sets to `CORNERS[vid]` — *"0..1 within the quad"* (`ui_rect.vs.hlsl:74`) — and which the FS
already uses as the `lerp` parameter (`ui_rect.fs.hlsl:197`). For every `t` in `[0, 1)`,
`frac(t) == t`, so the two expressions agree on every fragment; they differ only at exactly
`t == 1.0`, which a pixel centre does not land on. **`Tile` as S-D11 spelled it renders as
`Stretch`**, and its own red (M5-e) compares two implementations that compute the same fragment.

**(2) "the tile count folded into `uv` at pack" cannot be done and would break the property S-D11
was built to obtain.** ~~That clause.~~ **STRUCK.** `UiInstance.uv` is four floats, `offset_of! == 48`,
documented *"Normalized UV rect `(u0, v0, u1, v1)` in `[0, 1]` … Written verbatim at pack"*
(`instance.rs:56-62`), and all four are consumed as `sub_min`/`sub_max`. Folding a count in means
`uv.zw = sub_min + N * extent`, and then `sub_min + frac(t) * (sub_max - sub_min)` sweeps N whole
frames — the sheet bleed S-D7 designed a guard around, reproduced by the mechanism that retired
S-D7, on the gate (G5-8) that exists to forbid it. The clause is inherited verbatim from the
mechanism S-D11 replaced: there, N in `uv` worked because `REPEAT` wrapped at the TEXTURE boundary.
Under `frac`-in-sub-rect it does not. **The repair rewrote the sentence and kept the clause that
belonged to the paragraph it deleted.**

**Decision: the repeat count is a per-instance integer PAIR and it rides `flags` bits 5..19.**

```
bit  5          FLAG_TILED          set iff tiles_x > 1 || tiles_y > 1
bits 6..=12     UI_TILE_X  (7 b)    repeats across the sub-rect, 1..=127
bits 13..=19    UI_TILE_Y  (7 b)    repeats down  the sub-rect, 1..=127
```

and the fragment computes

```
uv = sub_min + ui_tile_frac(local_uv * float2(tiles_x, tiles_y)) * (sub_max - sub_min)
```

on the `FLAG_TILED` side and the landed `lerp` on the other.

**Consequences, stated because each moves a number somewhere else.**

* **The bit budget is now EXHAUSTED.** Bits 5..19 were fifteen; `FLAG_TILED` + two 7-bit fields are
  fifteen. Bit 4 stays reserved for S7. §6's exposure row is amended, because the animation and
  interaction plans read it to decide whether they may take a bit — after S5 they may not, and the
  next per-instance datum widens the record instead. `instance.rs:69-78`'s prose and the
  `ui_flag_consts` generated span both move with this.
* **`FLAG_TILED` is set only when a count exceeds 1**, so a corner sub-quad (always `1×1`) packs
  BYTE-IDENTICALLY to its `Stretch` record. That is what lets G4-3's corner claim carry over
  unchanged and what makes S-D8's `mode != Tile ⇒ FLAG_TILED zero` row exact on both sides.
* **7 bits, not 8 and not 6:** a UI chrome edge tiles a 8–32 px source over up to ~1000 px, i.e.
  tens of repeats; 63 could clip a long scrollbar track and 127 cannot. The clamp is a `min` and it
  is ~~shares G5-6's diagnostic counter (S-D18)~~ **NOT counted** *(corrected 2026-08-26 at the
  landing: the tile clamp happens in `pack_ui_nine_slice_instance`, a receiverless free function —
  which is exactly the reason the S4 ledger retired this counter and the reason S-D18 (1) had to
  move the SHEET clamp's counter into the gather. Only the gather's arithmetic moved; the pack's
  did not, so the pack's clamp still has nowhere to put a counter.)*

**(3) The count is DERIVED, not authored, and it needs no texture dimensions.** The engine records a
texture's size nowhere — `components.rs:520-525` says so and `BindlessTextureTable` confirms it — so
the reference-engine derivation (`dest_px / source_px`) is unavailable. It is not needed. A
nine-slice already states its own source→destination scale twice: `border_px` is the corner's
destination size and `border_uv` is the same corner's source extent. Their ratio IS the scale, and
the sub-rect width cancels out of it:

```
tiles_x = round( (rect_w - bp_l - bp_r) * (bu_l + bu_r) / ((1 - bu_l - bu_r) * (bp_l + bp_r)) )
tiles_y = round( (rect_h - bp_t - bp_b) * (bu_t + bu_b) / ((1 - bu_t - bu_b) * (bp_t + bp_b)) )
```

both clamped into `1..=127`. It is dimensionless, it is computed from values
`pack_ui_nine_slice_instance` already holds (after S4's proportional shrink, so a shrunk border is
the one that counts), and **because the source extent cancels, it is identical under a sheet frame
and under a whole texture** — which is what makes item 7's "the same sub-rect arithmetic" claim true
for the first time. Degenerate inputs (`bu_l + bu_r == 0`, `bp_l + bp_r == 0`, a non-positive centre
source extent, or a non-finite result) yield `1`, i.e. `Stretch`; a zero-border nine-slice has no
scale to read and does not get to guess one.

**It also retires an asserted number.** G5-7's *"a destination whose edge spans 4 whole tiles"* had
no mechanism that could produce a 4. On G4-3's own landed scene — `rect 96×96`,
`border_px = [16, 24, 16, 24]`, `border_uv` at its equal-thirds `Default` — the formula gives
`tiles_x = 64 * (2/3) / ((1/3) * 32) = 4` and `tiles_y = 48 * (2/3) / ((1/3) * 48) = 2`, exactly. The
gate stops asserting 4 and starts computing it.

**Per region**, `tiles_x` applies to the centre COLUMN (regions T, C, B) and `tiles_y` to the centre
ROW (L, C, R); every other axis of every other region is `1`. Corners are `1×1` and therefore
untiled by construction.

**Rejected.** (a) *CPU expansion — one sub-quad per tile*, the pure-CPU route D8d otherwise prefers:
`ui_node_sub_codes` writes into a fixed `[u32; UI_MAX_SUBS_PER_NODE]` and the staging key is
`node * UI_RECORDS_PER_NODE + sub` (`upload.rs`), so an unbounded per-node emission does not merely
cost records — it destroys the `(node, sub)` code the sorted loop recovers each record's source
from. It is Unity's documented quad explosion and its 16 250-quad cap (research §, `Image.cs`
`GenerateTiledSprite`). (b) *carry the counts in a field the sprite branch does not read* —
`corner_radius` (16 dead bytes on every textured record), `border_width` or `border_color`: this is
exactly the `corner_radius`-as-UV alias S2 spent 16 B per record to retire, and reviving it one rung
later for one flag's worth of data would trade a permanent invariant ("this field means ONE thing")
for fifteen bits the budget already has. (c) *a `-D TILE` shader variant* — the manifest records
these two sources as having **no `-D` axis**, and a runtime flag costs one branch that is
uniform-per-instance exactly like `FLAG_TEXT` and `FLAG_TEXTURED` beside it.

**(4) The shader edit is ONE new eDSL leaf and ONE `SpirvBlob<N>`, not "two lengths, two manifest
rows".** ~~"one shader edit at S5 (eDSL leaf, re-emit, re-DXC, two `SpirvBlob<N>` lengths, two
manifest rows)"~~ **CORRECTED, three ways, each measured:**

* **The eDSL has no `frac`.** No `frac`, `floor` or `fract` occurs anywhere in
  `crates/boyko_shaderdsl/src/`. The leaf costs a `Cf` method, its `EvalCf` `f32` arm
  (`x - x.floor()`), its `Emit` printer arm, and a host oracle row in
  `crates/boyko_shaderdsl/tests/ui_leaves.rs`.
* **There is no existing leaf to extend.** `crates/boyko_shaderdsl/src/ui.rs` holds exactly six leaf
  bodies and none of them touches the sprite `uv`; the line to be replaced
  (`ui_rect.fs.hlsl:197`) sits in `main`, below the last `// === GENERATED … END ===` sentinel at
  `:159`. S5 mints a NEW leaf `ui_tile_uv(uv, local_uv, flags) -> float2`, a new spliced span, and a
  new `assert_span_is_the_body_of` row in `ui_rect_edsl_sync.rs`.
* **Only the FS blob moves.** `ui_rect.vs.hlsl` has no sprite branch and no `ui_flag_consts` span;
  the manifest's own landing history records the VS at `2408 → 2408, byte-identical` across the
  whole S3 sprite landing. One length (`UI_RECT_FS_SPV: SpirvBlob<8760>`), and **zero** new manifest
  rows — `FLAG_TILED` is a runtime bit, and the manifest's rule is one row per `-D` variant, of
  which these files have none. The two existing rows gain notes; the landing-history table gains an
  S5 row.

**And the whole sprite-`uv` computation moves INTO the new leaf**, so the template line becomes
`float2 uv = ui_tile_uv(inst.uv, input.local_uv, inst.flags);`. That is not tidiness: it closes a
real hole. `emit_ui.rs` owns the whole file as a `format!` template, `ui_rect_edsl_sync.rs` compares
only the sentinel spans and the six leaf bodies, and `ui_rect_spv_sync.rs` compares the committed
`.hlsl` to the committed `.spv` — so **nothing in the workspace compares the generator's `main`
template to the committed `main`**. An edit applied to one copy and not the other is green under
`cargo test` and is silently reverted by the next `emit_ui` run. Putting the mechanism inside a
sentinel span puts it under the gate that already exists.

### S-D16 — the flipbook writes `UiSpriteSheet.index`, and it MUST: a dense `Changed<C>` inside `Or<..>` is measurably DEAD

*(added 2026-08-21 at the S5 pre-build audit. The rung's Lands list contradicted itself; the
contradiction is resolved by a kernel measurement, not by a preference.)*

**(1) The contradiction.** Item 4 says `UiSpriteCursor` is *"the only column the flipbook system
writes per frame"*; item 5 says the same system writes `index`, which is a field of
`UiSpriteSheet` — item 2's `4 B, table`. Both cannot hold, and
`UI-ADVANCED-ARCHITECTURE.md:581`/`:583` repeat both halves verbatim, so neither document arbitrates.

**(2) The measurement that decides it.** Written and run against this tree at `b2318ac5` on rustc
1.97.1 (`boyko-ecs`, a three-frame schedule; the probe was deleted after reading):

| frame | `Query<(), Changed<DCursor>>` | `Query<(), Or<(Changed<TSheet>, Changed<DCursor>)>>` |
|---|---|---|
| 1 — insert | 1 | 1 |
| 2 — idle | 0 | 0 |
| 3 — **dense write through `Mut`** | **1** | **0** |

**A dense `Changed<C>` inside `Or<..>` can never be true.** The cause is structural and is in the
kernel, not in the test: `Changed<C>` supports dense fully (`HAS_DENSE`, `HAS_DENSE_INCLUDE`,
`resolve_dense`, the per-slot tick read — `filter.rs:1321-1390`), but the `Or<(..)>` `QueryFilter`
impl **overrides none of them** (`filter.rs:1834-2030` sets `IS_ARCHETYPAL`,
`NEEDS_CHANGE_DETECTION`, `CONTAINS_*` and nothing else), so `Or::HAS_DENSE` takes the trait default
`false`, `resolve_dense` is never called on the inner term, its `ChangedFetch.dense` stays the
`init_fetch` NULL, and `filter_fetch`'s first line is `if fetch.dense.is_null() { return false; }`
(`filter.rs:1483-1484`). Frame 1's `or = 1` comes from the TABLE arm.

**Consequence for this rung, and it is the decisive one.** `ui_render_discovery`'s filter is
`Query<(), ui_pack_inputs!(changed)>` — a flat `Or`. A dense `UiSpriteCursor` placed in that list
would be READ correctly by the gather (`WorldView::get_component_raw` routes dense ids to
`dense_get_raw`, `component_api.rs:199-205`) and would be INVISIBLE to the discovery filter, so the
generation would never bump on a flipbook tick, the D6a per-slot gate would keep skipping, and **the
sprite would render a frozen first frame with nothing saying so** — the exact failure
`gather.rs:66-70` records for the S3 case. The macro's own promise, *"adding a component to
`ui_pack_inputs!` wires the discovery filter for free"*, **is true for TABLE components only**, and
that sentence is amended where it lives.

**Decision, three parts.**

1. **The flipbook writes `UiSpriteSheet.index` through `Mut<UiSpriteSheet>`, and that tick IS the
   repaint signal.** It uses `set_if_neq` (`data.rs`, MUT4-gated), so a tick at 12 fps does not bump
   the generation on the ~4 frames out of 5 where the frame index does not change — the churn is
   proportional to visible change, not to frame rate. `&mut T` is FORBIDDEN at this site: it does not
   consult ticks (`write.rs:234`), and the repaint depends on the tick.
2. **`UiSpriteCursor` stays dense, loses `frame`, and gains `loops_done`.** `frame` under the ruling
   above is written and read by nobody — the dead-datum class this campaign names at `:343`. And
   `UiSpriteAnim.repeats` had no reader at all: nothing in `{ elapsed, frame, dir }` counts completed
   cycles, so `Once`/`repeats` could not be honoured and `repeats` was a second dead datum in the
   same pair. The cursor becomes
   `#[repr(C)] #[derive(Clone, Copy)] UiSpriteCursor { elapsed: f32, dir: i8, loops_done: u8, _pad: [u8; 2] }`
   — **8 B, align 4, padding SPELLED** (the `UiNineSlice::_pad` rule, `components.rs:582-586`: both
   S5 structs reached their stated sizes through IMPLICIT tail padding, which is what that rule
   forbids). `UiSpriteAnim` likewise becomes
   `#[repr(C)] #[derive(Clone, Copy)] UiSpriteAnim { first: u16, last: u16, fps: f32, mode: SpriteAnimMode, repeats: u8, _pad: [u8; 2] }`
   — **12 B, align 4**, with `mode` a `#[repr(u8)]` **typed enum** (`Forward|Reverse|PingPong|Once`)
   rather than a raw `u8`: S-D13 (4)(3) ruled that the AUTHORED component keeps the typed enum and
   only a CROSS-CRATE raw byte is `debug_assert!`ed, and `mode` never crosses — the flipbook and the
   component both live in `boyko_ui`, and the pack never sees it. No count const and no conversion
   site are minted for it. **All four sizes are MEASURED with a `const _: () = assert!(size_of…)`,
   not asserted in prose** (S-D12 (2)).
3. **`ui_pack_inputs!` gains exactly ONE component: `UiSpriteSheet`.** ~~"the three components that
   affect the picture"~~ **STRUCK — only one of the three affects the picture.** The pack derives
   `uv` from `(cols, rows, index)` and takes the slot from `UiSheet`; it never reads `UiSpriteAnim`
   (author configuration the flipbook consumes) and, after (2), never reads `UiSpriteCursor` (the
   flipbook's private state). The gather probes EVERY listed component on EVERY visited node — a
   probe that returns `None` is still a probe (`ui_s0_measure.rs:218-224`) — so listing the other
   two would have charged two dead probes to every node of every changed frame, and one of the two
   would additionally have sat in the `Or` as a term that cannot fire.

**Cascade, all of it downstream of "ONE, not three":** the probe census goes **7.00 → 8.00**
(+14.3 %), not 7.00 → 10.00; the `Or` arity goes **6 → 7** (ceiling 12; `UiVisual` makes 8 and the
interaction plan's scroll datum 9); `ui_s0_discovery.rs` gains **one** `PackInput` variant, one
`ALL` entry, one `name()` arm and one `mutate_pack_input` arm (three landings, not nine — and the
array's hard-coded arity in `const ALL: [PackInput; 6]` is one of them); `ui_s0_measure.rs`'s prose
ladder gains one row. §5's *"S5 owes 7.00 → 10.00 (its three)"* is corrected in place.

**(3) The sheet OVERRIDES `UiImage`; it does not replace it.** Item 2 never said whether a
sheet-bearing node still needs `UiImage`, and both answers cost something. **Ruled: `UiImage`
remains the capability**, and the GATHER — the one site that already flattens components into
`PackInput` — substitutes the sheet's slot and the computed frame rect into `UiImageInput`. Three
reasons, each a landed mechanism: (a) `ui_node_sub_codes` is documented *"the SOLE authority"* and
its truth table plus `pack_ui_sub_record`'s two `.expect` preconditions are keyed on `input.image` —
that is the machinery G4-8 and M4-g exist to protect, and S-D12 (3) ruled its shape one rung ago;
(b) `pack.rs` stays free of every `boyko_ui` type, so `UiImageInput` keeps its three fields and the
pack learns nothing about sheets; (c) it keeps `components.rs:520-525` TRUE — `border_uv` is *"a
FRACTION of the node's current `UiImage` UV sub-rect"*, and because the sheet writes the frame INTO
that sub-rect, item 7's "`border_uv` composes for free" is true rather than merely hoped. A node
carrying `UiSpriteSheet` and no `UiImage` therefore draws its background alone — the same structural
skip S-D12 (3) ruled for `UiNineSlice` alone, and it gets the same truth-table row.

**The gather reads the sheet table through `WorldView::resource::<UiSheetTable>()`**
(`dispatcher_token.rs:284`), which is a read-only projection the view already offers — so
`gather_ui_nodes`' signature does not change, and the resource read is once per gather, not per
node, and is therefore NOT a probe. **The table gets the mint verb item 1 omitted**, on the
`FontTable` precedent it cites but did not copy: `FontTable::load(&BakedFont) -> FontId`
(`text/font.rs:130-158`) is a setup-time push into a `Vec` inside a `#[derive(Resource)]` struct
returning the dense index. `UiSheetTable::register(UiSheet) -> SheetId` is the same verb; §6's
*"the `u16 sheet_id` dense-handle mint"* row names a surface that Lands item 1 otherwise never
creates.

### S-D17 — S5 carries AM6's clamp itself, and the seam is a resource read, not a `world` call

*(added 2026-08-21 at the S5 pre-build audit.)*

~~"If the animation plan has not landed, S5 reads `Time`'s real delta directly and the seam is one
function — `ui_frame_delta(world) -> f32`."~~ **Both halves struck.**

**(1) The value is the refuted one.** [`UI-PLAN-ANIMATION-DECISIONS.md` AD1](UI-PLAN-ANIMATION-DECISIONS.md#ad1--uiclock-is-a-resource-not-a-restime-read-at-each-consumer) rejects *"each system reads
`Res<Time>`"* by name, for this consumer by name (*"The sprites plan's flipbook and the interaction
plan's `ScrollMomentum` … read `UiClock`, not `Time`"*), and rejects *"no clamp, trusting `Time`'s"*
because AM6 measured that `Time`'s clamp does not reach the real delta. The kernel confirms it:
`time.rs:197` assigns `self.real_delta = raw` BEFORE the clamp at `:201`, and `real_delta()` is
documented *"unclamped, unscaled, pause-blind"*. An alt-tab stall hands the flipbook a two-second
delta, which skips whole cycles for `PingPong` and `Forward` and jumps `Once` to its end; a paused
game keeps animating.

**Decision: the fallback is `Res<Time>` PLUS AM6's clamp, spelled at the one site, with the constant
named `UI_FALLBACK_MAX_DELTA = 0.1` — AD1's own default — and a comment pointing at AD1 as the value
this line will be deleted in favour of.** S5 stays unblocked, but it does not adopt the option its
sibling rejected: it adopts the sibling's *conclusion* with the sibling's *number*, in one place, so
that the later replacement is a deletion rather than a behaviour change.

**(2) `ui_frame_delta(world)` is not callable from the system item 5 describes.** There is no `world`
handle inside a scheduled `Query`-bearing system: the only world-shaped read surface is `WorldView`,
minted solely from a `DispatcherToken`'s `&self` and `!Send`/`!Sync`, and a `&mut EcsMaster`
parameter would force `ui_sprite_flipbook` to be an EXCLUSIVE system. The in-tree spelling — and the
shape of the thing that replaces it (`Res<UiClock>`, AD1) — is a `SystemParam`. **The seam is
`Res<Time>` today and `Res<UiClock>`'s `dt_virtual` after the animation plan lands**, one parameter
swapped and one clamp deleted; the plan stops promising a function signature that neither the
fallback nor the replacement has. *(2026-08-26, [`UI-PLAN-ANIMATION-DECISIONS.md` **AD9**](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition) **(1)/(2)** at the A0 pre-build audit — **the field is `dt_virtual`.** Neither document named one, and the animation plan's own AD1 called `dt_real` "the default": taking it reds two legs of this rung's SHIPPED `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware` — a paused game animates, and `set_relative_speed(0.5)` stops halving. `dt_virtual` is `time.delta_secs().min(max_delta)`, i.e. `ui_sprite_flipbook`'s pre-A0b inline `min` *(DELETED by A0b; deliberately unanchored — a coordinate into deleted state resolves to whatever live line now occupies it)*'s arithmetic verbatim, so "one parameter swapped, one clamp deleted" is true of that field and of no other. The swap itself is owned by animation rung **A0b** — it belonged to no rung in either ladder until then.)*

### S-D18 — the S5 gate table: what each device row samples, and where the clamp counter lives

*(added 2026-08-21 at the S5 pre-build audit — the row-level corrections S-D15 and S-D16 force, plus
the two the S4 landing already paid for and this rung repeated.)*

**(1) The diagnostic counter has a home now, and it did not before.** G5-6 requires a counter for the
`index >= frame_count` clamp. The S4 ledger retired that counter for want of a home
(`:1644-1646`: the pack entry points *"are free functions with no receiver … and had nowhere to put
a counter"*, which is still true of all five of them), and S5's item 7 says in the same breath that
*"the diagnostic counter S4 was going to build are not built by anyone"* — one rung both retiring a
counter and requiring one. S-D16 (3) moves the sheet arithmetic into the GATHER, which owns
`UiGatherScratch` — the struct that already carries `probes`, deliberately not `#[cfg(test)]`
because *"a `#[cfg(test)]` counter cannot be read by the observer rung"* (`gather.rs:26-36`). The
clamp counter is `UiGatherScratch::sheet_index_clamps: u64`, unconditional, on the `probes`
precedent, exposed the way `UiUploadSystem::probes()` already exposes its sibling. *(Aside: the
struck citation `pack.rs:86`, `:204` in `:1645` is stale after S4's own landing —
`pack_ui_instance` is at `:141` and `pack_ui_image_instance` at `:296`; `:86` is mid-doc-comment and
`:204` is inside a `corner_radius` scaling. The claim survives; the anchors do not.)*

**(2) Every device row names its `UiSamplerMode`, because the sibling row it inherits from was
amended for exactly this one rung ago** (S-D13 (4)(2): G4-3 *"stated no mode, and its own assertion …
is FALSE under the default"*; `Smooth` is `Filter::Linear` and is `#[default]`,
`resources.rs:103-118`). G5-5, G5-7 and G5-8 all make which-texel-was-sampled claims. **G5-5, G5-7
and G5-8 run `UiSamplerMode::Pixel`**; the one claim that is *about* filtering — `inset_uv`'s
purpose, *"half-texel inset against bilinear bleed"* — gets its own `Smooth` row, **G5-9**, because
under NEAREST there is no tap to bleed and the field's entire effect is inert. M5-b's second half
moves onto G5-9.

**(3) A SKIP IS NOT A PASS, and it is NOT inherited.** `BOYKO_UI_GOLDEN_REQUIRE_DEVICE` occurs in
exactly one file in the workspace — `ui_nine_slice_gpu_golden.rs:586-589` — and
`tests/common/mod.rs` offers only `boot_or_skip`, which `eprintln!`s and exits 0. S5 has four
device-bound rows and two of its reds land only on them, so **each new golden file replicates the
guard**, exactly as S-D14 (7) ruled for S4's single row.

**(4) Every source must be able to SHOW what its row claims.** Two of the three sources as specified
cannot, and both are the S4 amendment (a) — *"a symmetric source makes region assignment
unobservable"* — recurring one axis over:

* **G5-7's 3×3 source cannot distinguish a tile from a stretch.** Each nine-slice region of a 3×3
  source under `border_uv`'s equal-thirds `Default` is EXACTLY ONE uniform texel; four repeats of a
  uniform texel and one stretched copy of it are the same solid block, byte-identical under NEAREST.
  The blessed hash would be reproduced exactly by a `Tile` that silently fell back to `Stretch` —
  which is the entire failure the row exists to catch. **G5-7's source becomes 6×6: nine 2×2 cells,
  each cell two distinct values.** Its top edge is then 64 px from a 2-texel source: stretched, two
  32-px bands; tiled ×4, eight 8-px bands. The row asserts NAMED PROBE COLUMNS as well as the hash,
  because the columns are what depend on the count.
* **`inset_uv` on a 4×4-texel source is degenerate.** S-D5's *"4×4 flipbook grid"* read as 4×4
  TEXELS gives 16 frames of one texel; a half-texel inset is `0.5/4 = 0.125` uv against a frame
  extent of `0.25`, so insetting both sides leaves an extent of **exactly zero** — `u0 == u1` at the
  texel centre. `frac`, `lerp` and the inset are then all no-ops and M5-b moves no pixel. **S-D5's
  flipbook source is 4×4 FRAMES of 4×4 TEXELS — a 16×16 RGBA8 grid** — so `inset_uv = (1/32, 1/32)`,
  the frame extent is `0.25 - 1/16 = 0.1875`, and G5-1's hand-computed constant at
  `(cols=4, rows=4, index=6)` is `uv = (0.53125, 0.28125, 0.71875, 0.46875)`, exact in binary FP.
  **All sixteen frames must be mutually distinct**, and specifically frames 5, 6 and 7 must differ,
  or an off-by-one in the decode is invisible to a hash.
* **G5-8's sheet source is 4×4 frames of 6×6 texels (24×24), with `inset_uv = (0, 0)`**, stated with
  its reason: G5-8 runs NEAREST, where the inset protects against nothing, and a zero inset makes
  each frame exactly 6 texels per axis so each nine-slice region is exactly 2×2 — which is what makes
  *"every sampled texel lies within that frame's sub-rect"* decidable per texel instead of per
  sub-texel blend.

**(5) The missing red is the characteristic sheet defect.** G5-1 and G5-5 both use a SQUARE grid, and
with `cols == rows` the natural decode `col = index % cols; row = index / cols` is bit-identical to
the same expression with `cols` and `rows` interchanged — so a transposed `(cols, rows)`, the
standard sprite-sheet bug, passes the hand-computed constant AND the pinned hash. **G5-1 additionally
carries a NON-SQUARE case (`cols = 4, rows = 2, frame_count = 8`)**, and **M5-f — swap `cols` and
`rows` in the frame decode** is added, reddening it. This is S4's own dihedral-symmetry finding
(`ui_nine_slice_gpu_golden.rs:22-31`) one axis over.

**(6) M5-b as spelled is a compile error, not a red.** *"drop `inset_uv`"* deletes a field G5-1 names
as one of its four inputs, so the target fails to BUILD rather than to assert, and the protocol
requires the predicted failure OBSERVED. **M5-b becomes "ignore `inset_uv` in the frame-UV
derivation", leaving the field in place** — and it now reds G5-1 (the constant moves) and G5-9 (the
`Smooth` probe at the frame edge takes its neighbour's contribution).

**(7) M5-a is verb-dependent and the verb is now pinned.** Merging `UiSpriteAnim` into
`UiSpriteCursor` reds G5-3 only if the merged component's write stamps a tick. Under S-D16 (1) the
flipbook's tick-bearing write is `Mut<UiSpriteSheet>::set_if_neq`, and the merge moves
`UiSpriteAnim`'s fields into a component the flipbook writes with `&mut` — which does NOT consult
ticks. **M5-a is restated as "merge `UiSpriteAnim` INTO `UiSpriteSheet`"**, the component the
flipbook already tick-writes: then `Changed<UiSpriteAnim>` becomes `Changed<UiSpriteSheet>`, fires
every frame the index moves, and G5-3's *"never on a per-frame advance"* half reds for the reason
D8a exists. As originally spelled it was a red that could not fire.

**(8) Item 6's landing is gated, because S4's identical line was.** G4-7 existed for the S4 macro
edit and the landed ledger calls that edit *"the omission that would have made the rung invisible"*.
S5's table had no row driving it. **G5-10** is added: the new `PackInput` variant is driven end to
end by `ui_s0_discovery`'s no-catch-all loop, and `PackInput::ALL.len() == ui_pack_inputs!(count)`
holds — the assertion that turns "added to the macro but not to the test" into a red with a reason.

### S-D19 — the six corrections BUILDING S5 found, and the one red that did not fire

*(added 2026-08-26 at the S5 landing. The pre-build audit and its check lens between them refuted
seventeen claims and amended sixteen rows; these six are the ones neither could have found without
running the code, and the last is the one the RED PROTOCOL found rather than the build.)*

**(1) The untiled arm keeps the `lerp` INTRINSIC, and "bit-identical" was the wrong word for what
S-D15 (1) refuted.** S-D15 (1) says `uv = sub_min + frac(t) * (sub_max - sub_min)` is *"bit-identical
to the `lerp` it replaces"*. It is not: HLSL `lerp` lowers to `OpExtInst GLSL.std.450 FMix`, whose
specified form is `x * (1 - t) + y * t`, while the decomposition spells `x + t * (y - x)`. They agree
to about one ULP, not bit-for-bit. **The refutation still lands** — an 8-bit golden cannot see a
1-ULP shader edit (`reference-golden-fp-resolution`), so `Tile`-as-spelled rendered as `Stretch`
either way — but the word matters DOWNSTREAM: the six committed UI image pins were blessed against
the intrinsic, and had the new leaf decomposed it, all six would have held by luck rather than by
construction. `Cf::vec2_lerp` therefore spells the intrinsic and its doc says why, and the leaf's
oracle table pins the untiled arm as hard as the tiled one.

**(2) The leaf costs THREE new eDSL facets, not one — and the third is a printer bug the leaf found.**
S-D15 (4) costed `frac` alone. Measured: `vec2_add` does not exist either (only `vec2_add_scalar`),
and no `float2` `lerp` exists (`lerp` is scalar-only on `FieldScalar`), so the leaf needs `vec2_frac`
AND `vec2_lerp`. The third is subtler. `Cf::named_uint` exists — but its Emit node is a `NamedLit`
typed `Float`, minted for a symbol whose only consumer is a bare `return` with no operand check, and
`and_u`/`shr_u` DO check their operands `Uint`. Spelling the tile constants as bare literals instead
would have put a SECOND copy of the S-D2 bit layout inside the leaf, beside the copy
`emit_hlsl_ui_flag_consts` generates from the layout — the drift class S-D10 exists to close. So
`named_uint_val` + `Node::NamedUint` were added. **And then the generator PANICKED**: `emit_ui_leaf`
passed `NO_NAMED_LITS` to the printer, because the six S1 leaves spelled only bare literals, and an
empty symbol table under a symbol node is an index-out-of-bounds AT GENERATION. Found by running the
generator, not by reading it.

**(3) "Only the FS blob moves" is true of the `.spv` and FALSE of the `.hlsl`.** S-D15 (4) says the
VS is untouched because it has no sprite branch and no `ui_flag_consts` span. Both halves are true,
and the conclusion still does not follow: the tile bits are described in the `UiInstance` MIRROR
span, which both stages carry. `ui_rect.vs.hlsl` gained one comment line and `ui_rect_edsl_sync`'s
VS half is what covers it. The `.spv` did stay byte-identical (2408 → 2408), exactly as the S3
landing recorded for the same reason.

**(4) The gather reads the sheet table through `try_resource`, not `resource`.** S-D16 (3) and Lands
item 2 both name `WorldView::resource::<UiSheetTable>()` and cite `dispatcher_token.rs:284` — the
PANICKING verb, whose own doc says *"Panics if no resource of type `R` has been inserted … Use
`try_resource` for the non-panicking variant."* The read is hoisted above the DFS in
`gather_ui_nodes`, which runs for every UI scene in the tree, and EIGHT in-tree harnesses build
worlds by hand and insert only what they need. Following the plan literally panics every one of them
at the first gather. An absent table is not an error — it means no sheet is registered — so the
gather takes `Option<&UiSheetTable>` and an absent table leaves every node's `UiImage` untouched,
which is the S-D12 (3) structural-skip shape the rung already uses. **The same ruling settles a
residual contradiction:** G5-6's parenthetical says a `frame_count == 0` sheet makes the node *"emit
no sprite record"*. It cannot — `ui_node_sub_codes` is the SOLE authority on a node's records (gate
G4-8) and the gather is not allowed a second opinion. Inert-and-fall-back is the behaviour, and
G5-1's second test pins all three ways to be inert.

**(5) `#[require(UiSpriteCursor)]` is UNBUILDABLE, and the reason is a kernel defect.** S-D18's
closing amendment adds it so an authored `flipbook:` cannot silently never tick. Applied, it PANICS
on every insert: the require pass resolves the required id's `ComponentPool` in the target
ARCHETYPE, and a dense id owns no per-archetype pool by construction (dense plan D0). Three S5 gates
failed this way. The panic even names an expansion that never happened, so its message points away
from the cause. Filed in `docs/OPEN-QUESTIONS.md` (+ the `ru/` twin, same edit). **The buildable
remedy is a BUNDLE**: `AnimatedSpriteBundle` carries the layout base, the image, the sheet, the
animation and the cursor in one spawn, so the pairing is structural at the AUTHORING site instead of
at the component. **G5-12** pins both halves — the bundle animates, and a hand-spawned
`UiSpriteAnim` with no cursor is FROZEN, silently.

**(6) ⚠️ A RED THAT DID NOT FIRE, and what it exposed.** **M5-j** — set `FLAG_TILED` unconditionally,
so a `1×1` corner stops being byte-identical to its `Stretch` record — left EVERY gate green on its
first run: G5-7, G5-8, the S4 nine-slice golden and all twelve CPU tests. Two causes, and both are
instructive:

* **The picture genuinely does not move.** `frac(local_uv * 1) == local_uv` for every covered
  fragment — S-D15 (1)'s own finding, one level down. No golden can see this mutation, ever.
* **G5-11's corner leg was a COMPARISON between two arms that share the mutated code.** It asserted
  `tile_record.flags == stretch_record.flags`, and `tile_flag_bits` is called on the `Stretch` path
  too — so the mutation moved both sides equally and the equality held. A gate that compares two
  outputs of the mutated function is not an instrument for mutations of that function.

The repair is to assert the ABSOLUTE property S-D15 states and the comparison only implies: a `1×1`
region's record carries NO tile bits at all — not the flag, not either count field. With that line
added, M5-j reds immediately (`flags & tile_mask == 0x2060`). **This is the campaign's
"gate that cannot fail" class caught by the protocol working**, and it is worth naming the shape:
*a relative assertion between two arms of the same function is blind to every change that is
symmetric across them* — which is most single-line changes to that function.

---

### S-D20 — the S6 pre-build audit: the cursor hole closes with a HOOK, and six of the rung's own sentences did not survive the tree

*(added 2026-08-26 at the S6 pre-build audit. Every claim below was run in this worktree on
`rustc 1.97.1`; the two probe tests were written, run, and deleted, and `git status --porcelain` was
empty afterwards. The rung as written could not have closed the hole it exists to close.)*

**(1) The ruling: `UiSpriteAnim` takes `#[component(on_add = …)]`, and the hook deferred-inserts
`UiSpriteCursor::default()` through a one-field `#[derive(Bundle)]` wrapper.** S6 stated two options
and chose neither; the option it builds is a third one neither names.

*MEASURED, because ruling for a mechanism without building it is this campaign's own recorded
failure — S5's `#[require]` and S4's `UiSpriteSheet` gate were both written around something that
did not work.* The probe:

* A hook receives `DeferredEcsMaster`, whose `commands()` handle carries `entity(e).insert::<B: Bundle>`
  (`component/hooks/deferred_master.rs:150`, `:242`). Structural change from a hook is **deferred**,
  and `boyko_ecs/tests/phase14a_hooks_deferred.rs:63-83` already pins that the deferred op IS applied
  at the outermost drain.
* A local TABLE component with `#[component(on_add = probe_on_add)]`, spawned through `Commands`,
  produced — after the apply — `has_component(e, UiSpriteCursor::component_id()) == true` with the
  value `UiSpriteCursor { elapsed: 0.0, dir: 1, loops_done: 0, _pad: [0, 0] }`. The `dir: +1`
  `PingPong` needs arrives on its own.
* **The bare type does NOT work, and this is the part a paper design would have got wrong:**
  `insert(UiSpriteCursor::default())` is `error[E0277]: the trait bound UiSpriteCursor: Bundle is not
  satisfied`. Dense plan D0 SUPPRESSES the single-component `Bundle` impl —
  `boyko_macros/src/component.rs:315`, where `hooks.storage_dense` joins `no_bundle` and
  `storage_bitset` in one gate. The one-field wrapper bundle is not a style choice; it is the only
  spelling that compiles, and `AnimatedSpriteBundle` is the standing proof that a multi-field
  `#[derive(Bundle)]` may carry a dense field.

**Why this reaches what `#[require]` could not.** The require pass fails on dense because it resolves
the required id's `ComponentPool` **in the target ARCHETYPE**, and a dense id owns none. The deferred
insert does not go that way: `InsertCommand` PARTITIONS the bundle's ids and routes the dense subset
off the table path — *"`is_dense(cid)` filters dense bundle ids out of the TABLE replace path (a
dense id has no archetype pool, so its table-flag-gated fire + `get_pool_mut` are wrong/absent)"*
(`commands/insert_command.rs:128-137`). The hook route reaches the one path that already learned the
partition, which is exactly what `docs/OPEN-QUESTIONS.md` records the require pass as not having
learned.

***Rejected — (a), the dispatch-side insert.*** Three defects, two fatal. **It is not one site:**
`parse_and_insert` is the SPAWN path only, and a survivor that GAINS `UiSpriteAnim` from a file edit
is patched by `patch_unit_struct`, whose insert branch is `TextStruct::insert`
(`reload/reconcile.rs:571-575`) — a second, independent construction site with the identical silent
outcome, one reload later. A `.ui` DELETION is a third (`C::remove` leaves an orphan dense row).
**It falsifies the campaign's headline invariant where nothing can see it:** `.ui` would insert a
component `ui!` does not, and `UiSpriteCursor` is excluded from the vocabulary by design, so it can
never be a comparator row — the same blindness (3) measures on `UiBackground`. And it is strictly
dominated by (1), which is fewer sites and keeps all three authoring paths identical.
*(The round-trip worry does NOT materialize, and the negative is recorded because the lens asked:
`serialize_ui` writes only from `LiveNode` (`serialize.rs:47-105`) and the cursor is not a `LiveNode`
field, so an auto-inserted cursor is never written back and G6-1's byte identity is untouched. The
equivalence invariant is the casualty, not the serializer.)*

***Rejected — (b), "wait for the kernel defect to close".*** "Closes" is undefined for S6.
`docs/OPEN-QUESTIONS.md` (2026-08-26, correctly filed in both languages) lists THREE options and only
the first restores the attribute; the second — refuse at compile time — *"leaves the capability
missing rather than fixed"*, and the third is what S5 did. The entry closes *"**What it blocks:**
nothing today"* and marks the first option a SCOPE call the owner has not taken. Option (b) would
condition S6's only reason for existing on an event two of the three filed resolutions never produce,
stacked on top of a D7 that (7) shows has no owner either. **Two orphans is not a schedule.**

***What (1) costs, stated rather than hidden.***
* The insert is **DEFERRED**. The cursor is present after the apply, not inside the window that
  spawned the anim. Nothing in this campaign reads a cursor at spawn time — but the pairing is
  structural, not instantaneous, and a future reader must be told which.
* `AnimatedSpriteBundle` places the cursor AND the hook fires on the anim's add, so the bundle's
  cursor is REPLACED by a fresh `Default` at the drain. On a spawn frame the two values are equal, so
  this is inert today; it would become a visible reset if the bundle ever spawned a non-default
  cursor. `on_add` (*newly* added — `hooks/mod.rs:67`) and never `on_insert` is what holds it to that
  one case.
* ~~An anim REMOVED from a `.ui` file leaves its dense cursor row behind. The symmetric `on_remove`
  hook closes it at the same one place. The row is 8 B and inert without the anim, so this is
  tidiness rather than correctness — but it is a landing and the rung counts it.~~
  ⚠️ **REFUTED BY MEASUREMENT AT THE BUILD — S-D21 (1). The symmetric hook is not tidiness; it is a
  HARD PANIC on every despawn of an animated node.** `on_remove` also fires on the per-component
  pass of a DESPAWN. The entity is still live at hook time (`w.is_alive(ctx.entity)` reads `true` —
  MEASURED, so a liveness guard does not help), and the deferred `RemoveCommand` it enqueues runs
  after the entity is gone: `RemoveCommand::apply: stale entity Entity { id: EntityId(0),
  generation: 0 }`. A `.ui` hot reload that deletes an animated node despawns it, so the "tidiness"
  landing would have crashed the exact workflow this rung exists to make work. **Also measured: a
  despawn already reclaims the dense row on its own** (`has_component` after despawn = `false`), so
  the hook buys nothing there even if it could run. S6 therefore lands `on_add` ALONE, and the real
  residue is narrower than the struck bullet claimed: an anim removed from a SURVIVING node leaves
  an 8 B row that is inert (the flipbook needs all three components) and self-healing (a re-added
  anim gets a fresh `Default` cursor — MEASURED). The kernel defect is filed in
  `docs/OPEN-QUESTIONS.md`.
* The exclusion property NARROWS — (2).

**(2) The exclusion property narrows, and G6-3 cannot tell the two apart.** S6 stated it as *"a `.ui`
file must not be able to inject a running cursor into a live world"* and cited G6-3 as its gate. G6-3
asserts only that the NAME `UiSpriteCursor` yields an "unknown component" diagnostic — that the
string is absent from the table. Under (1) a cursor DOES appear beside an authored animation, so the
wide sentence is false while G6-3 stays green; under the rejected (a) it is equally false and equally
invisible. The property that is both true and gated is **"a `.ui` file must not NAME a runtime-state
component, or give one a value"**: the cursor arrives at its `Default`, author-uncontrollable, on
every authoring path alike. The narrowing is written into the Lands paragraph rather than assumed,
because a property no gate can distinguish from its own negation is a sentence.

**(3) The equivalence corpus is structurally blind, and it is GREEN TODAY over a real divergence.**
MEASURED. `p6a_equivalence::button_widget_three_ways_equivalent` authors `UiBackground { color: 0 }`
in its `.ui` arm while the `ui!` and hand-spawn arms insert `UiBackground::default()`. There is **no
`UiBackground` dispatch arm anywhere** — `grep -rn UiBackground crates/boyko_ui/src/text/
crates/boyko_ui/src/reload/` returns nothing — and the probe reported `UiLayout present = true`,
`UiBackground present = false` on the `.ui` node. `cargo test -p boyko-ui --test p6a_equivalence`
prints `5 passed`, exit 0, unpiped. Two independent reasons, and closing either alone is not enough:
`spawn_dot_ui` asserts the PARSE report and hands the lowering `owned.report.clone()`, a clone that
is dropped (`p3_common/mod.rs:66-90`); and neither comparator lists the name — `presence_vector` is
10 hand rows, `p6a_equivalence`'s local `pres!`/`valeq!` are 10 and 5. **A generic set comparison is
constructible** (`EcsMaster::archetype_master()` is public), so the hand list is a choice. G6-5 and
M6-d exist because of this measurement.

**(4) G6-1's round trip is not achievable for a realistic sprite node, and the reason predates S6.**
MEASURED: a `.ui` source spelling `UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint:
4294967295 }` gives `UiImage present after parse = true`, and `serialize_ui` then emits the node's
`UiLayout` line **and nothing else** — `round-trip contains UiImage = false`. Cause: `write_node`
reads only `LiveNode`'s seven component fields (`tree_view.rs:49-56`) and `UiImage` is not among
them. A sprite node MUST carry `UiImage` — it is the capability, and the sheet only substitutes its
slot and UV. **This is a class, not an instance:** the dispatch has 19 component arms, `serialize_ui`
writes 8 of them (+`UiName` from the sigil, `ComputedRect` deliberately excluded), and the reconcile
patches the same 8 — so **TEN** existing components (`UiText`, `Button`, `Bar`, `BarFill`, `UiImage`,
`UiGrid`, `UiAnchor`, `OnClick`, `OnHover`, `OnSubmit`) already exhibit precisely the silent failure
D7 cites as its own justification, and D7c's pin — *"same round-trip bytes"* for all 19 — would
REPRODUCE the loss rather than fix it. S6 does not create this and does not fix it: its three
components would be landed MORE completely than `UiImage`, the component they modify. G6-1's fixture
is therefore `UiImage`-free with a comment naming why, and the gap is filed for the owner in
`docs/OPEN-QUESTIONS.md` as a SCOPE call, because closing it is +9 landings per component × ten.

**(5) M6-b's red could not fire, and G6-2 named the one outcome the mutation cannot disturb.** Both
halves are in the amended rows above; the shape is worth naming once more because it is the
campaign's most frequent defect: *the mutation was a compile error rather than a gate red, and the
property the gate asserted — "hot reload PRESERVES them" — is precisely what a component with no
reconcile arm does.* `patch_node` preserves by omission and there is no sweep that removes an
unlisted component, so the observable failure is STALE, never ABSENT. A gate written to observe a
disappearance observes nothing and passes.

**(6) "Five landings each" is NINE, and the file the list never names is the one that makes the other
two reachable.** Traced site-by-site against `UiSpacing`, the closest existing analogue (struct-form,
table, round-trips, hot-reloads): (1) the dispatch match arm `dispatch.rs:87`; (2) the private field
parser `parse_ui_spacing` `:371`; (3) the `pub(crate) parse_ui_spacing_public` wrapper `:276` —
mandatory, not decorative, because the reconcile lives in another module and (2) is private, and
exactly seven such wrappers exist, one per reconciled component; (4) the formatter `write_ui_spacing`
`serialize.rs:134`; (5) its emit block in `write_node` `:66-70`; (6) the `LiveNode` field
`reload/tree_view.rs:50`; (7) the snapshot read in `UiTreeView::build` `:99`; (8)
`impl TextStruct for UiSpacing` `reload/reconcile.rs:665`; (9) the
`patch_unit_struct::<UiSpacing>(…)` line `:445`. **`serialize_ui` reads ONLY `LiveNode`, and
`patch_unit_struct` takes `live_val` ONLY from `LiveNode`, so without (6) and (7) both (4) and (8)
are unreachable code** — and `reload/tree_view.rs` appears in neither S6's list nor
`UI-ADVANCED-ARCHITECTURE.md:371-376`, the D7 record S6 inherits the number from. `UI-PLAN-INTERACTION.md:590-594`
states the reconcile half correctly for its own components; the correction existed in a sibling and
did not propagate. Consequences: S6's fallback is ~30 landings, not 15, and D7's *"12 × 5 = 60"*
(`UI-ADVANCED-ARCHITECTURE.md:423`) is ~108 *(both coordinates re-measured by CONTENT 2026-08-28:
they read `:372-376` and `:403`, which are a blank line and a blank line; the D7 heading is `:371`
and the `12 × 5 = 60` figure — already struck to `12 × ~9 = ~108` there — is `:423`)*. Two further
costs neither document counts: **two**
comparator rows, not one (3); and **four new leaf value parsers** — the dispatch's fifteen leaves
(`parse_unit … parse_template_id`, `dispatch.rs:545-875`) include no `[f32; 4]`, no `u16`, and no
parser for either sprite enum, while `UiNineSlice.border_px` / `.border_uv` are `[f32; 4]`,
`UiSpriteSheet.sheet` / `.index` and `UiSpriteAnim.first` / `.last` are `u16`, and both `mode` fields
are typed enums. D7a's *"the bodies already exist"* (`UI-ADVANCED-ARCHITECTURE.md:414-421`) is true of
the fifteen and false of these four.

**(7) D7 has no owning document — three plans name three different owners and none of them builds
it.** The sweep is in the struck line at the head of this rung. Two consequences beyond the schedule.
First, S6 is invisible: `grep -rn "\bS6\b"` across the sibling plans, the architecture and the book
returns exactly one UI-campaign hit (`UI-PLAN-AETHER.md:576`), and it merely cites S6's exclusion in
passing — **nobody outside this document knows the rung exists**. Second, the architecture's own
R4-bounding pin binds S6 and cannot name it: §11 item 1 says *"§10.9 must be green — the generated
table reproducing all 19 existing components — before **rung 4** adds the twentieth"*
(`UI-ADVANCED-ARCHITECTURE.md:1772-1776`), but rung 4 is D1, the 80 B widening, which adds no
vocabulary member; the rung that adds the twentieth `.ui` NAME is S6, which §11 does not list.
Landing S6 on the fallback spends that pin and makes *"all **19** existing components"*
(`:1765`, `:452-457`) stale at 22. Recorded here and struck at both architecture sites; the ownership
question itself is a SCOPE call and goes to the owner.

**(8) Two Lands items were not expressible.** `flipbook:` is Aether U5's PROP name, not a `.ui`
spelling — struck in place above with the reason. `ImageBundle` cannot "gain optional members" —
`bundle.rs` has zero `Option`s and takes every field unconditionally, and the buildable form is the
separate bundle S5 already landed. *Both are the same shape: a sentence written in the vocabulary of
one surface and filed against another.*

**(9) G6-4 was a dead cross-reference.** *"**G6-4** stays as specified"* — and one `grep` over the
whole tree returns that line and nothing else. There was no prior G6-4 for it to stay as; the Gate
table had three rows; neither red mutation named it. **The rung's single reason for existing was
carried entirely by a forward reference to a specification that did not exist**, which is the class
this campaign has now hit at S4 (a gate whose subject a later rung created) and in the diagnostics
corpus (twelve benches nothing had built). Its specification and its red (M6-c) are above, and it is
written to RED TODAY rather than to be deferred with the fix.

**(11) NOTHING GATES THIS CAMPAIGN'S OWN CITATIONS, and two anchors in three do not land.**
Found while trying to certify the amendments above rather than trust them: the root gate
`tests/internal_docs_anchors.rs` — the one thing in the tree that checks a `file.rs:N` citation —
scans a hand list of **four** documents (`GATED_DOCS`, `:231`: `FEATURE_MAP.md`, `SYSTEMS.md`,
`ARCHITECTURE.md`, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`). **None of the five UI campaign documents is
on it.** PROVEN, not inferred: an anchor deliberately repointed to `insert_command.rs:999999` in
this file left the gate at `5 passed`, exit 0.

MEASURED by widening `GATED_DOCS` to the five UI documents for one run and restoring the file
byte-identically (`cmp`):

| Document | anchors checked | STALE |
|---|---|---|
| the four already gated | **735** | **0** |
| `UI-PLAN-SPRITES.md` | 118 | **80** |
| `UI-ADVANCED-ARCHITECTURE.md` | 7 | **7** |
| `UI-PLAN-AETHER.md` | 6 | **4** |
| `UI-PLAN-ANIMATION.md` | 4 | **3** |
| `UI-PLAN-INTERACTION.md` | 8 | **2** |
| **the five UI documents** | **143** | **96 (67%)** |

Plus **three dead PATHS**: `crates/boyko_ui/benches/ui_animation.rs` ([`UI-PLAN-ANIMATION-A2-A8.md` A8](UI-PLAN-ANIMATION-A2-A8.md#a8--the-measurement-rung-what-the-tick-actually-costs--size-s--depends-on-a1a5), pre-split `:2857`,
read at its target 2026-08-28 by the tenth pass — *"**Lands.** `crates/boyko_ui/benches/ui_animation.rs`
+ its `[[bench]]` entry in `Cargo.toml` —"*. It read `:663`, then `:2282`. The two
`OPEN-QUESTIONS.md` twins carry the same sentence and were re-aimed to `:2282` on 2026-08-28;
**this third copy was not, because that sweep's scope was "documents the landing edited" — and the
value it would have propagated was wrong anyway**. ⚠️ **Part 11 re-read both discarded values, and
BOTH descriptions of them were false — this line carried the only instance in the corpus that no
pass had ever listed.** It said `:663` is *"today a sentence about `UI_FALLBACK_MAX_DELTA`"*; read at
`:663` on the tree part 11 inherited, it was *"before the datum exists rather than after. §7 Q1's
answer, when it comes, edits one line and both"*, with the nearest `UI_FALLBACK_MAX_DELTA` sentence
three lines down. It said `:2282` is *"a BLANK line"*; read at `:2282` on the same tree, it was
*"defence out of the fixture and into a **source census** — and the seventh showed that a"*. **The
readings are quoted as CONTENT with a tree attached and not as claims about a line, because writing
them down moved the lines again.** **The `:663` description escaped five
sweeps because it names no stale coordinate to grep for** — it is the *discarded* value in a re-aim,
described in passing, and nobody sweeps those. It also carried the word *"today"*, which is the
strongest currency claim in the whole cluster)
and `crates/boyko_render/shaders/ui_rect` twice (`UI-PLAN-SPRITES.md:3067`, `:3470`).

**The difference between 0/735 and 96/143 is the gate, not the authors.** Every "MEASURED at
`file.rs:NN`" in this plan — the S-series' entire evidentiary apparatus, and every anchor written
into this amendment — rots the moment a source file moves, and nothing notices. This is the
campaign's "gate that cannot fail" at corpus scale, and it is why the audit lenses' line numbers
disagreed with the tree in several places. **Arming it is NOT done here**: it reds instantly on 96
pre-existing anchors, which is a repair rung with its own protocol and its own budget, not a line in
an S6 audit. Filed for the owner in `docs/OPEN-QUESTIONS.md`.

**(10) Where G6-4's green stops.** `UiPlugin::build` registers neither `ui_sprite_flipbook` nor a
`UiSheetTable`; every registration in the tree is a test harness. Stated at the end of the rung, so a
green G6-4 is not read as "sprites animate in the app". It follows from D32's deferral, not from S6.

---

### S-D21 — the six corrections BUILDING S6 found, and the pre-existing bug the rung could not build around

*(added 2026-08-26 at the S6 landing. Every claim was run in this worktree on `rustc 1.97.1`. The
pre-build audit's own ruling survived contact; two of its supporting sentences did not, and one of
them would have shipped a panic.)*

**(1) The symmetric `on_remove` hook is UNLANDABLE, and S-D20 (1) filed it as tidiness.** The full
measurement is written into the struck cost bullet above. The shape is worth naming here because it
is the audit's own defect class turned on the audit: **a cost was costed without being built.**
S-D20 (1) was scrupulous about MEASURING the `on_add` half — it says so in its own first line, and
cites S5's `#[require]` and S4's `UiSpriteSheet` gate as the reason — and then wrote the `on_remove`
half from symmetry. Symmetry is exactly what does not hold: `on_add` fires on one event and
`on_remove` fires on two, and the second one (despawn) hands the deferred queue an entity that will
be dead by the drain. **A probe of four lines found it; a paragraph of reasoning had endorsed it.**

**(2) A comparator ROW cannot gate itself, so G6-5 injects the divergence.** The rung asked for "one
row in `presence_vector` and one in `pres!`/`valeq!`". A row is not an observable: a list that names
`UiNineSlice` and is never handed two nodes that DISAGREE about `UiNineSlice` is exactly as green as
a list that does not name it. G6-5 as landed authors the sprite node three ways, compares it through
BOTH comparators, and then hands each comparator a pair that differs in exactly one sprite component
— once by presence, once by value — asserting through `catch_unwind` that the comparator PANICS.
M6-d reds it at the first local-list control. *This is the same move the campaign already makes at
the shell (inject an error to prove the gate is live); what is new is that it belongs INSIDE a test
whose subject is a hand-maintained list.*

**(3) G6-2's two legs are two TESTS.** Applying M6-b the first time, the EDIT leg failed and the
DELETE leg never ran — the first-failure shadowing that `--no-fail-fast` exists to stop between
targets, reproduced inside one target. The protocol requires the predicted failure OBSERVED, and
"the other leg would also have failed" is not an observation. Split, M6-b reds both, and both were
seen.

**(4) The unknown-component diagnostic's COLUMN is the BODY column, not the name's.** MEASURED: 20,
where a reader would guess 4. `parse_and_insert` receives `body_col` — the first byte inside the
component's `{` — and `extract_component_span` never hands the name's own column down. The gate pins
the measured value and the gap is written down rather than quietly accepted; giving the diagnostic
the name's column is a `split.rs`/`ast.rs` change no rung has asked for.

**(5) G6-4's "registered `UiSheetTable`" was a dead precondition.** `ui_sprite_flipbook`'s signature
is `Res<Time>` plus `Query<(&UiSpriteAnim, Mut<UiSpriteCursor>, Mut<UiSpriteSheet>)>`. It never
reads the table — the table is the RENDER gather's input, one crate over. Inserting one to satisfy
the rung's prose would have put a datum in the gate that nothing under test reads, which is
precisely the "dead datum" class this campaign has now recorded five times.

**(6) `split_top_level` was NOT bracket-aware, and bracketed `.ui` values had never parsed.** The
`[f32; 4]` leaves S-D20 (6) costed cannot exist without this fix, so it is S6's to make. What the
fix uncovered is older and wider:

* `crates/boyko_ui/src/text/split.rs`'s own doc claimed the P3 field list is *"provably free of
  `{`/`[`/quoted-comma values … locked by a rejection test"*. **Both halves are false.** GUI P6a
  added `UiImage`'s `uv_min`/`uv_max`, which are `[u, v]`; and `grep` finds no such rejection test
  anywhere in the tree. A claim, an instrument that does not exist, and a grammar that outgrew both.
* MEASURED consequence: `UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint: … }` split into
  `uv_min: [0` / `0]` / `uv_max: [1` / `1]`. `parse_f32_pair` rejected both UV fields, they kept
  their `Default`s, and FOUR recoverable errors went into the LOWERING report.
* **`p6a_equivalence::image_widget_three_ways_equivalent` is green over it, for two independent
  reasons that compose exactly as S-D20 (3) described for `UiBackground`**: the harness asserts the
  PARSE report and drops the lowering clone, so the four errors are unobservable; and the authored
  UVs happen to EQUAL `UiImage::default()`'s (`[0,0]` / `[1,1]`), so the mis-parse lands back on the
  right values. A test that meant to prove "the `.ui` path carries these UVs" proved that the
  defaults are `[0,0]`/`[1,1]`.
* The fix is one match arm: `(` and `[` open the same depth counter, `)` and `]` close it. The
  `boyko_input` copy is untouched — `.keys` has no bracketed values — so the file's "COPIED
  VERBATIM" header became "copied, then DIVERGED, and here is why".

---

