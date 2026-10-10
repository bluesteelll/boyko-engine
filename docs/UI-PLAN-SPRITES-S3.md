> **Part of [UI-PLAN-SPRITES.md](UI-PLAN-SPRITES.md)** — §4 rung S3 and S3 · LANDED. Ladder gate: see the index.

### S3 — the textured lane: set 1, the UI sampler, `UiImage` finally renders — **size L**

*Architecture D2, D3, D8e, plus S-D3 and S-D4.*

**Lands.**

1. **S-D3's `bind_descriptor_set_at`** (or nothing, if G3-0 says the cheap sampler route works and the
   set-1 bind is still needed — it always is, so this always lands).
2. **S-D4's UI sampler** — set 0 binding 3, `UiSamplerMode::{Smooth, Pixel}` chosen at `ui_setup`;
   plus `DescriptorKind::Sampler` + `BindGroupEntry::Sampler` **iff** G3-0 says so. *(Landed
   2026-08-21: G3-0 came back GREEN, so the RHI change is NOT part of this rung. Binding 3 is a
   `COMBINED_IMAGE_SAMPLER` whose image half is the atlas texture and is never read — the sampler
   half alone is what the stage declares. `UiSamplerMode` ships intact; see the ledger's defect 5
   for why the literal "reuse the binding-1 atlas sampler" reading would have dropped it.)*
3. `ui_setup` gains `bindless: Option<&VulkanBindlessSet>` and `font: Option<&BakedFont>`; the
   pipeline is built through `create_graphics_pipeline_bindless(desc, set1_layout)` ~~when the
   bindless set is supplied and through the existing one-set path when it is not~~ **always —
   over the shared table's set-1 layout when one is supplied, and over a UI-owned fallback's when
   it is not** *(amended 2026-08-21: the one-set path is not legal, measured; ledger defect 1)* —
   **so a host with no bindless table still boots and still draws rects and text**.
4. **D8e — the font atlas binding becomes default-filled** with a 1×1 transparent texture when
   `font: None`. The same trick the bindless table already uses with its magenta error slot.
5. `FLAG_TEXTURED` (bit 3); the slot in bits 20..31; `PackInput` gains `image: Option<UiImageInput>`
   carrying `(slot, uv, tint)`.
6. The fragment shader's textured branch, **through the eDSL** — `emit_ui.rs` re-emitted, both `.hlsl`
   re-spliced, both `.spv` re-DXC'd and re-committed, ~~the two `SpirvBlob<N>` lengths updated~~
   **ONE `SpirvBlob<N>` length updated** *(corrected 2026-08-21 at the S5 audit, by MEASUREMENT of
   what S3 actually landed: the FS went `7136 → 8760` and the VS did **not** move — `2408 → 2408`,
   byte-identical, "DXC's output is measurably indifferent" to the comment-only edit inside the
   shared mirror (`ui/mod.rs:154-157`, and the manifest's own landing-history row). The prescription
   said two and one moved; the same over-count was about to be repeated verbatim in S5's item 7.)*,
   the two `SHADER-VARIANT-MANIFEST.md` rows updated (still no `-D` axis), and
   `ui_rect_{vs,fs}_spv_sync` re-run with dxc present.
7. Both recorders bind set 1: `record_ui_rects` via `bind_descriptor_set_at(1, …)`, `present_blit.rs`
   via its existing `cmd_bind_descriptor_sets` with `first_set = 1` — the shape `gbuffer.rs:1081-1095`
   already uses.
8. `gather_ui_nodes` reads `UiImage`; `ui_pack_inputs!` gains it, so `ui_render_discovery` sees
   `Changed<UiImage>` for free.

**Default OFF.** `UiImage`'s default is texture 0 with a **fully transparent** tint
(`components.rs:454-466`), chosen originally so an authored-but-untextured Image "never flashes a
white box when P5a lands". That default is now load-bearing: an existing world gains no pixels. The
S2 image hashes must be unchanged by this rung.

**Gate.**

| # | Claim | How |
|---|---|---|
| **G3-0** | The sampler route decision (S-D4) | Sample the bindless texture with a sampler taken from the existing `COMBINED_IMAGE_SAMPLER` binding. **Validation messenger == 0 messages** ⇒ cheap route, no RHI change; any message ⇒ the additive `DescriptorKind::Sampler` variant ships. The message text is quoted in the commit either way. |
| **G3-1** | A sprite renders | New GPU golden `ui_sprite_bindless`: an 8×8 procedural checkerboard (S-D5) registered into a `BindlessTextureTable`, drawn as one textured quad. Four decisive texels (two light, two dark, at known UVs), validation clean, and the S-D6 image hash pinned. |
| **G3-2** | The untextured majority pays nothing | The four S2 image hashes unchanged. |
| **G3-3** | A sprite-only UI boots | `ui_setup(font: None, bindless: Some(..))` succeeds and `ui_sprite_bindless` still draws. |
| **G3-4** | A bindless-less host still works | ~~`ui_setup(bindless: None)` builds the one-set pipeline~~ **`ui_setup(bindless: None)` builds a 2-set pipeline over a UI-OWNED fallback set 1** *(amended 2026-08-21 at landing — see the defect ledger below; the one-set pipeline is not legal)*; the rect and text goldens pass. |
| **G3-5** | The slot field round-trips | CPU test: pack with slot ∈ {1, 4095}, read bits 20..31 back, assert equality; `debug_assert!(slot < BINDLESS_TEXTURE_CAPACITY)` on the pack path. |
| **G3-6** | Both recorders bind set 1 | The offscreen golden (`record_ui_rects`) **and** ~~`ui_rect_swapchain_golden` extended with one textured quad~~ **`ui_rect_swapchain_golden` unchanged** *(amended 2026-08-21: the extension is unnecessary and would have moved an S2 pin — see the ledger)*. |
| **G3-7** | Divergent descriptor indices are correct | *(added 2026-08-21: **M3-b named a G3-7 that did not exist** — the S3 gate table stopped at G3-6 and the 64-slot leg was specified only as a MEASUREMENT.)* `ui_sprite_divergence.rs`: 256 dense 4×4 quads over 64 distinct slots, every quad asserted to read back its own slot's colour. |

**Red mutations.**

* **M3-a — write the slot into bits 16..27 instead of 20..31.** The shader reads a different slot;
  slot 0 is the reserved **magenta error texture**, so G3-1's texels go magenta. *Proves the field the
  pack writes is the field the shader reads, which no offset assert can say.*
* **M3-b — drop `NonUniformResourceIndex`.** Run G3-7's 64-slot leg: validation reports a non-uniform
  index error, or the sampled texel is wrong for all but one quad. *Proves the qualifier is
  load-bearing and not decorative — and it is the first non-uniform thing in this shader.*
* **M3-c — bind set 1 in `record_ui_rects` but NOT in `present_blit.rs`.** The offscreen golden stays
  **green**; `ui_rect_swapchain_golden` reds. *This is S-D9's mutation: two recorders, two gates, and
  the on-screen one is the one a user sees.*
* **M3-d — remove the default-fill and call `ui_setup(font: None)`.** Boot fails. *Proves G3-3 is not
  vacuous — a `None` that silently falls back to a real font would pass G3-3 while D8e was absent.*
* **M3-e — give `UiImage::default()` an opaque tint.** G3-2 reds (every world with an authored Image
  gains a white box). *Proves the default-OFF claim is enforced by a gate rather than by a comment.*

**Measurement — §10.1, the decision that keeps Model A reachable.**

| Leg | Scene | Instrument |
|---|---|---|
| baseline | N textured quads, **all on one slot** | GPU timestamp around the UI pass |
| 8-way | the same N quads over **8** distinct slots | same |
| 64-way | the same N quads over **64** distinct slots | same |

N ∈ {256, 2048}. If the 64-slot leg regresses materially against the 1-slot leg, **Model A (a runtime
atlas) is reachable without changing one component** — `UiImage { texture, uv_min, uv_max }` describes
an atlas tile and a bindless slot equally well. That asymmetry is why D2 is safe to start with, and
this measurement is what makes the deferral honest rather than hopeful. Recorded in S7 either way.

Also §10.8 legs **(b)** and **(c)**: the gather's probes/node/frame with `UiVisual` (if the animation
plan has landed it) and with the sprite components.

---

### S3 · LANDED 2026-08-21 — measurements, defect ledger, reconciliations

**Everything below was run on this box (RTX 3060 Laptop, validation layer ON, `dxc` = the pinned
VulkanSDK 1.4.350.0). No gate here reports a SKIP.**

#### §10.1 — the divergence measurement, and the instrument that had to be fixed first

`crates/boyko_render/tests/ui_sprite_divergence.rs`. Scene: N dense 4×4-px sprite quads over
1 / 8 / 64 distinct bindless slots, GPU-timestamped (`TopOfPipe`/`BottomOfPipe`) around the UI pass.

*The first run was NOT a measurement and said so by being physically impossible:* 10240 / 13312 /
11264 ns for 1 / 8 / 64 slots — every value an exact multiple of **1024 ns** (the device's timestamp
lattice), and the 8-slot leg SLOWER than the 64-slot one, which no divergence cost can be. A single
sub-15 µs pass was being measured with a ruler whose smallest mark was a tenth of the thing measured.
Fix: bracket **64 passes** per timestamp pair and divide, putting the lattice at ~16 ns/pass.

| N | 1 slot | 8 slots | 64 slots |
|---|---|---|---|
| 256 | 592.0 ns | 576.0 ns (−2.7 %) | 576.0 ns (−2.7 %) |
| 2048 | 4400.0 ns | 4368.0 ns (−0.7 %) | 4496.0 ns (+2.2 %) |

**Verdict: no measurable divergence cost.** Every delta is 1–6 steps of the 16 ns/pass resolution and
the sign is not monotone in slot count. **D2 stands; Model A is not needed.** The deferral is now
honest with a number behind it, exactly as SR3 required.

#### The corroborating result: M3-b does not red on this hardware, and §10.1 says why

Dropping `NonUniformResourceIndex` from the eDSL leaf, re-emitting and re-DXC-ing (`.spv` 8760 →
8680 B) left **every** sprite gate green — including G3-7's 256 quads over 64 slots. That is not a
weak test: the descriptor index is `nointerpolation`, i.e. **per instance**, and this rasterizer does
not pack one warp from two primitives, so the index is wave-uniform by construction here. The same
fact explains §10.1's flat table — two independent observations with one cause, which is what makes
either believable. The qualifier stays: a non-uniform index without it is UB by the Vulkan spec, and
another driver may pack warps differently. What *is* live on it is the byte gate — `ui_rect_spv_sync`
sees its removal (80 bytes) even when no pixel does.

#### §10.8 leg (c) — and the leg's own first answer being to the wrong question

`ui_s0_measure.rs::measure_gather_with_sprite_components` first reported `probes/node = 6.00` for
BOTH an imaged and an image-less world, with wall-clock medians disagreeing in sign between N=256 and
N=2048. Correct answer, wrong question: the gather probes every pack input on every visited node — a
probe returning `None` is still a probe — so **presence changes what the pack emits, not what the
gather reads**. The S3 gather cost is the LIST getting longer, measured against the pre-S3 build:

* before S3: 4 pack inputs + `Children` = **5.00** probes/node/frame
* after S3: 5 pack inputs + `Children` = **6.00** probes/node/frame (**+20 %**), paid by every node
  of every changed frame whether or not it is a sprite.

Gather wall-clock, image-less vs imaged (median of 100): 92.5 → 94.7 µs @ N=256, 737.8 → 768.9 µs @
N=2048 — the probe-hit/probe-miss difference, under this instrument's noise at both N.
Leg **(b)** (`UiVisual`) does not run: the animation plan has not landed it, and a leg measuring a
component that does not exist would be measuring nothing.

#### The RED ledger — what each mutation actually did

| # | Predicted | Observed |
|---|---|---|
| **M3-a** | slot into bits 16..27 ⇒ G3-1's texels go magenta | **RED, twice over.** The literal mutation (`UI_SLOT_SHIFT = 16`) does not even compile — S3's new `UI_SLOT_SHIFT + UI_SLOT_BITS == 32` const-assert catches it, which is stronger than the predicted GPU red. The semantically-equivalent skew (the pack site writes `<< 16` while the shader still reads bits 20..31) reaches the GPU and gives **exactly** the predicted `[255, 0, 255, 255]` — slot 0's reserved magenta error texture — plus a second, device-free red in `ui_pack_cpu`. |
| **M3-b** | drop `NonUniformResourceIndex` ⇒ the 64-slot leg reds | **NOT REPRODUCIBLE on this box.** See above: per-instance ⇒ wave-uniform here. Recorded, not curated away. |
| **M3-c** | offscreen green, `ui_rect_swapchain_golden` red | **RED, and stronger than written.** No textured quad was needed: because `ui_rect.fs` statically uses set 1, the on-screen recorder's set-1 bind is load-bearing for a PLAIN RECT draw. `VUID-vkCmdDraw-None-08600`, four times, on the unmodified S2 scene — while both offscreen goldens stayed green. This is why G3-6 above no longer asks for the scene change: the gate exists without it, and changing the scene would have moved an S2 image pin for nothing. |
| **M3-d** | remove D8e's default fill ⇒ boot fails | **RED.** `ui_setup(font: None)` → `MalformedAsset("MTSDF atlas extent is zero (VkImageCreateInfo requires w > 0 && h > 0)")`. G3-3 is not vacuous. |
| **M3-e** | opaque `UiImage::default()` tint ⇒ **G3-2 reds** | **RED — but NOT at G3-2, which CANNOT fire.** All four S2 goldens stayed green, because not one of them constructs a `UiImage`; an opaque default tint is invisible to them. The gate that caught it is a CPU test this rung added (`ui_pack_cpu::default_image_tint_packs_a_fully_transparent_sprite`). **As specified, M3-e named a red that no gate could produce** — the plan's own "gate that cannot fail" shape, and it would have been curated rather than caught had the mutation not been run. |

#### The defect ledger — where the rung as written could not be built

1. **G3-4's one-set pipeline is not legal, and this was measured, not argued.** `ui_rect.fs`
   STATICALLY uses set 1 (its sprite branch is reachable code), so building it against a one-set
   layout produces, verbatim, on this box: `vkCreateGraphicsPipelines(): … uses descriptor
   [Set 0, Binding 3, variable "g_ui_sampler"] but the binding was not declared in the
   VkPipelineLayoutCreateInfo::pSetLayouts[0]` and the same for `[Set 1, Binding 0, variable
   "g_sprites"]` (both `VUID-VkGraphicsPipelineCreateInfo-layout-07988`), then
   `vkCmdDraw(): … statically uses descriptor set 1, but … The set (1) is out of bounds for the
   number of sets bound (1)` (`VUID-vkCmdDraw-None-08600`) — on a **plain rect** draw. **Amendment:**
   `bindless: None` builds the SAME 2-set pipeline over a UI-owned fallback set 1 (one `SAMPLED_IMAGE`
   descriptor holding a 1×1 transparent texture), so G3-4's actual property — *a host with no bindless
   table still boots and still draws rects and text* — holds, with still exactly ONE `.spv` and no
   `-D` axis (item 6 intact). Rejected alternative: a `-D` variant, which item 6 forbids and which
   would have doubled the byte-gate surface.
2. **M3-b named a gate (G3-7) the plan never wrote.** Promoted to a real gate — see the table above.
3. **M3-e named a red that cannot fire** (G3-2 has no `UiImage` in any of its four scenes) — see the
   RED ledger.
4. **A missed lockstep consumer of `ui_pack_inputs!`, found by the unconditional suite run** and not
   by the enumerated Lands list — the S2 eleventh-site lesson, repeating: `ui_s0_discovery.rs` pinned
   the probe census as the LITERAL `5 * 5`, i.e. it wrote the pack-input list's length down a second
   time. The fifth input turned that into a red with nothing wrong. Fixed at the root: `ui_pack_inputs!`
   gained a `count` arm and the census derives from it. **A second, quieter instance in the same file:**
   G0-1 drove a hand-written four-name list of pack inputs, so after S3 it claimed "each pack-input
   mutation" while checking four of five — silently. Both are now derived from the macro, and the
   name list carries a length assert against `ui_pack_inputs!(count)` so the next rung's new input
   reds *with a reason* instead of under-covering.
5. **S-D4's `UiSamplerMode` and G3-0's "cheap route" pull in opposite directions as written.** G3-0
   green was specified to mean "the RHI change is dropped", but the cheap route as literally described
   (reuse the binding-1 ATLAS sampler) would also drop `UiSamplerMode` — the text's own reason for
   S-D4 — because the MSDF atlas needs LINEAR and a pixel-art UI needs NEAREST from one descriptor.
   **Resolution:** the UI declares its own sampler at set 0 binding 3 exactly as S-D4 decided, but
   backs it with a `COMBINED_IMAGE_SAMPLER` whose image half is the atlas texture and is never read.
   That is G3-0's mechanism (a shader-declared plain `SamplerState` served by a combined descriptor —
   which Vulkan's own validation names as legal: *"Possible VkDescriptorType that could be used are:
   VK_DESCRIPTOR_TYPE_SAMPLER or VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER"*), so **G3-0 is GREEN and
   the additive `DescriptorKind::Sampler` / `BindGroupEntry::Sampler` RHI change is dropped** — while
   `UiSamplerMode::{Smooth, Pixel}` ships intact and `Pixel` is gated by its own GPU test.

#### Reconciliations recorded at landing

* **The `.spv` moved on ONE stage only.** `ui_rect.fs.spv` 7136 → 8760 B (the set-1 array, the
  binding-3 sampler, the `FLAG_TEXTURED`/`UI_SLOT_*` constants, the sprite branch). `ui_rect.vs.spv`
  2408 → **2408, byte-identical**: the VS's only S3 edit is a comment inside the shared struct mirror,
  and DXC's output is measurably indifferent to it. The generated-HLSL diff was read before the
  re-bless.
* **The four S2 image pins reproduced EXACTLY** (G3-2), on the unmodified scenes, after the pipeline
  became 2-set and every UI draw gained a set-1 bind. The sprite golden is a NEW fifth pin, blessed
  here and looked at: eight distinct colours, all accounted for, including the one-pixel LINEAR blend
  seam at each checker-block boundary.
* **A node carrying `UiImage` emits TWO records** (its background rect, then its sprite quad — D4's
  per-node order), so the pack is no longer 1:1 with the gather. The two pack loops encode the append
  key differently ON PURPOSE and each says why: `pack_sort_upload` uses the running RECORD index
  (because `sort_by_stack` gathers `pack[idx]` by it) and `gather_into_staging` uses the
  `node * UI_RECORDS_PER_NODE + sub` CODE (because it packs directly in sorted order and must find
  each record's SOURCE from its key). Both are unique and strictly increasing in emission order, so
  both yield the same painter's order. ~~S4's nine-slice raises `UI_RECORDS_PER_NODE` and nothing else
  at that site.~~ **S4's nine-slice raises `UI_RECORDS_PER_NODE` AND rewrites two things at that site**
  *(corrected 2026-08-21 at the S4 pre-build audit)*: the key push is hard-coded to at most two
  sub-records (`upload.rs:337-343`) and must become a loop, and the pack dispatch is BINARY
  (`if append.is_multiple_of(UI_RECORDS_PER_NODE) { … } else { pack_ui_image_instance(..).expect(…) }`,
  `upload.rs:367-373`) and must become a full `match` over the sub code — because with the stride raised
  every sub-quad key takes the `else` arm, and on a nine-sliced node **without** `UiImage` that `.expect`
  panics in release as well as debug. **The same false sentence is in the tree** at `pack.rs:183-184`
  and is corrected in the same edit.
* **S-D3's `bind_descriptor_set_at` landed as specified**, `bind_descriptor_set` became its
  `set_index == 0` case, and no existing call site moved. `VulkanBindlessSet::as_bind_group()` is the
  new (non-owning, loudly documented) view that lets the shared table be bound through the generic
  verb — without it the offscreen golden could not have drawn a sprite at all, which is S-D3's whole
  argument.

---

