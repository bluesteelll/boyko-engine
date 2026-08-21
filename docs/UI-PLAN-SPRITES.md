# UI-PLAN-SPRITES — sprites, atlas and the draw path

**Campaign:** advanced UI/GUI for `boyko_ui` · **Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`)
**Date:** 2026-08-21 · **Status:** plan, pre-implementation, revision 1
**Authority:** [`docs/UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) (revision 2) — every
`D<n>` below is that document's decision, unchanged unless this plan says so at the point of change.
**Evidence:** [`docs/UI-ADVANCED-RESEARCH-SPRITES.md`](UI-ADVANCED-RESEARCH-SPRITES.md).
**Siblings:** [`docs/UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md) ·
[`docs/UI-PLAN-INTERACTION.md`](UI-PLAN-INTERACTION.md) · [`docs/UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md).

This is the ladder a developer walks. It is not a design document: where a decision already exists in
the architecture it is cited, not restated. Where this plan **adds** a decision — because building the
thing surfaced something reading it could not — the decision is numbered `S-D<n>`, carries a reason,
and names what was rejected.

---

## 0 · What this plan owns, and what it does not

The architecture's §11 sequences eight rungs. This plan owns four of them and states its dependency
on the other four rather than duplicating them.

| §11 rung | Owner | This plan's relation |
|---|---|---|
| 1 · D7 `.ui` registration table | **`UI-PLAN-AETHER.md`** | **Dependency.** Blocks exactly ONE rung here (S6), and S6 carries a fallback so the sprite ladder is never blocked behind D7's risk **R4**. |
| 2 · D31 + D6 + D32 — seam, gate, observer | **this plan (S0)** | The gather feeds the pack; the pack is the draw path. Animation and interaction both **extend** what S0 spells — see §7. |
| 3 · D30 — the eDSL migration | **this plan (S1)** | |
| 4 · D1 — the 80 B instance | **this plan (S2)** | Both feature halves depend on it; neither may widen it twice. |
| 5 · Sprites — D2, D3, D8a–e, D4 | **this plan (S3–S5)** | |
| 6 · Animation — D9–D15 | `UI-PLAN-ANIMATION.md` | **Dependency at S5** (the flipbook clock) and **consumer of S0** (`UiVisual` joins the pack-input spelling). |
| 7 · Interactivity — D16–D24 | `UI-PLAN-INTERACTION.md` | **Consumer of S0** (the gather's DFS is the hit-test's traversal). |
| 8 · Aether `ui` | `UI-PLAN-AETHER.md` | **Consumer of S3–S5** (it can only name components that exist). |

**S-D1 — the boundary is drawn at the pack-input seam, not at the crate edge.** Everything that
decides *what the GPU sees* is here: the gather, the generation gate, the instance record, the shader,
the pipeline's descriptor sets, the two recorders, and the five sprite components. Everything that
decides *what writes those components* belongs to a sibling.

**Reason.** The alternative boundary — "sprites = the five sprite components, the draw path is
shared" — leaves the draw path unowned, and the draw path is where every one of this campaign's
lockstep failures lives (**R1**). One plan must be answerable for the 80 B record and both recorders.

**Rejected:** (a) *sprites owns only §11 rung 5*; the widening (rung 4) and the eDSL migration (rung 3)
then have no owner, and they exist **because** of sprites. (b) *sprites also owns D7*; D7 is a `.ui`
authoring-surface refactor whose consumers are all four subsystems, and putting it behind a feature
plan makes the feature plan's schedule the campaign's schedule.

---

## 1 · Verified in-tree facts

Read in this worktree, not assumed. Every rung below is built on these; a rung that contradicts one is
wrong.

| Fact | Where |
|---|---|
| `UiInstance` is `#[repr(C, align(16))]`, **64 B**, with **9** per-field `offset_of!` const-asserts | `boyko_render/src/ui/instance.rs:35,69,92-99` |
| `corner_radius` is REINTERPRETED as the glyph UV under `FLAG_TEXT`; the source names the exit condition verbatim — a rect needing both a radius and a UV "retires this alias and widens `UiInstance` to 80 B" | `instance.rs:44-56` |
| Three flag bits are used (`BORDER_ANY`, `CLIP_PRESENT`, `TEXT`); bits 3..31 are free | `instance.rs:73,77,84` |
| `PackInput` has **no** texture, **no** tint, **no** slot field; `text_uv: Option<[f32;4]>` is the only UV | `ui/pack.rs:21-46` |
| `UiRenderScratch.last_seen_generation` is a **single `u64`** and is **read by nobody**; `UiRenderGeneration::bump` has no production caller; `pack_sort_upload` contains no compare and repacks unconditionally | `pack.rs:141-199`, `upload.rs:153-180` |
| The seam order today is `gather_nodes(world, node_buf)` **then** `pack_sort_upload` — so a gate inside the pack still pays the whole world read | `upload.rs:255-274` |
| The fused seam predecessor (`host_upload_frame_from_world`, **DELETED 2026-08-21**) had **zero callers** outside its own doc comments — and the CAUSE, established after this table was first written: its parameter list demanded a live `WorldView` AND a `&mut RhiContext` at one call site, which the token's M1 discipline forbids by borrowck. All three call routes died in the compiler — in-schedule: **E0502** (view = `&self` of the token, `nonsend_resource_mut` = `&mut self`); host owning adapter: **E0277** (`System: Send + Sync + 'static` vs `RhiContext`'s raw pointers); host borrowing adapter: **E0521/E0505** (the `'static` bound) — and the shapes are exhaustive because `WorldView` has exactly ONE constructor (`DispatcherToken::world`) and `DispatcherToken::new` is `pub(crate)` with two mint sites (scheduler dispatch, `run_system_once`), both inside a `Send + Sync + 'static` system. Superseded by S0's **two-phase seam** (Phase 1 gather / Phase 2 upload, sequenced in one `run_dispatcher`) | `docs/OPEN-QUESTIONS.md` entry 2026-08-21 (RESOLVED); `upload.rs` |
| The UI pipeline is built through the **one-set** `create_graphics_pipeline`; set 0 has three bindings (StorageBuffer @0 VERTEX\|FRAGMENT, CombinedImageSampler @1 FRAGMENT, UniformBuffer @2 FRAGMENT) | `ui/resources.rs:188-243` |
| `create_graphics_pipeline_bindless(desc, set1_layout)` exists and is used by the textured gbuffer, Forward and particle paths | `boyko_rhi_vulkan/src/rhi_impl/device.rs:2112` |
| `BINDLESS_TEXTURE_CAPACITY = 4096`; the allocator issues `1..capacity`, so the maximum live slot is **4095** | `boyko_rhi_vulkan/src/bindless.rs:72`, `boyko_render/src/bindless.rs:80-93` |
| **The bindless set's sampler is ONE IMMUTABLE sampler: trilinear (LINEAR mag/min/mip), 16× anisotropic, `REPEAT` on all three axes, `maxLod = 1000`** | `boyko_rhi_vulkan/src/bindless.rs:139-172` |
| `BindlessTextureTable` is owned by `boyko_app::runner`, inserted as a NonSend resource. `RhiContext` does **not** own it and `ui_setup` never sees it | `boyko_app/src/runner.rs:212`, `boyko_render/src/gpu_column.rs:163-176` |
| **`boyko_rhi`'s generic encoder has NO set-index bind verb.** `bind_descriptor_set` binds at **set 0** and says so | `boyko_rhi/src/encoder.rs:174-186` |
| `DescriptorKind` has no `Sampler` variant; `BindGroupEntry` has no sampler-only variant | `boyko_rhi/src/enums.rs:717-745`, `boyko_rhi/src/device.rs:352-420` |
| There are **two** UI recorders: the `RhiApi`-generic `record_ui_rects` (offscreen golden) and the concrete inline recording in `present_blit.rs` (on-screen). They are separate code | `ui/draw.rs:41`, `present/passes/present_blit.rs:201-329` |
| `ui_setup` requires `&BakedFont` **unconditionally** — a sprite-only UI cannot boot | `gpu_column.rs:269-292` |
| The UI's own set-0 binding-1 sampler is bilinear + **ClampToEdge** + no-mip | `ui/resources.rs:494-496` |
| `ui_rect.{vs,fs}.hlsl` have **no** `// === GENERATED ===` sentinels, no eDSL leaf, no manifest row, no `*_edsl_sync` / `*_spv_sync` test. The only pin is `SpirvBlob<2368>` / `SpirvBlob<7060>` — a byte **length** | `ui/mod.rs:122,129`; `docs/SHADER-VARIANT-MANIFEST.md` |
| **`goldens/PINS.toml` contains ZERO UI rows.** All 32 pins are scene screenshots | `goldens/PINS.toml` |
| The four UI GPU goldens assert **individual texels**, not images, and skip gracefully on a device-less host | `ui_rect_gpu_golden.rs:19-39` |
| `ui_hud_screenshot.rs` is `#[ignore]`d **8** times | `boyko_render/tests/ui_hud_screenshot.rs` |
| `boyko-ui` is a **dev**-dependency of `boyko_render`, annotated TEST-ONLY; `boyko_app` has no `boyko-ui` edge at all | `boyko_render/Cargo.toml:106-118`, `boyko_app/Cargo.toml` |
| PNG assets exist in-tree (`boyko_app/assets/pbr_fixtures/…`) but `boyko_image` is a **decoder only** — there is no encoder | `boyko_render/src/loaders/png_texture.rs`; crate-wide grep |
| `create_solid_color_texture` builds a texture from raw bytes with no asset file | `boyko_render/src/bindless.rs:366` |

Two of these were not in the architecture or the research and change decisions below: the **immutable
anisotropic REPEAT sampler** (S-D4) and the **absent set-index bind verb** (S-D3).

---

## 2 · Invariants this ladder may not break

1. **One pipeline, one `draw(6, N, 0, 0)`, one global z-sort, clip in the instance record.** No batch
   list, no per-texture rebind, no second pipeline. This is the crate's single best property and
   sprites are not permitted to spend it (D2, research §6).
2. **`ComputedRect` keeps its single-writer invariant.** The pack *reads*; it never writes geometry
   (D5, `widgets.rs:6-9`).
3. **Capability is component presence.** No sprite flag on a general component; absence is the
   structural skip.
4. **No side store.** The sheet table is a `Resource`-owned dense column; the cursor is a dense
   component. No `Vec`/`HashMap` keyed by element (Principle 0).
5. **Zero steady-state allocation in the pack.** `clear()` + `extend` into the reused scratch; the
   nine-slice expansion writes into that same scratch and must not change this (`ui_no_realloc.rs`).
6. **Every `unsafe` carries a `// SAFETY:` comment with concrete invariants.**
7. **Shaders are eDSL-authored and re-spliced between sentinels from S1 onward.** Before S1 the UI
   shaders are outside that rule; after S1 they are inside it and a hand edit is a red test.

---

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
| S5 | sheets + flipbook **+ `NineSliceMode::Tile`** *(added 2026-08-21: `Tile` needs the sub-rect arithmetic S5 builds — S-D11)* | `UiSpriteSheet` absent ⇒ `uv` comes from `UiImage`; `UiSpriteCursor` absent ⇒ no tick; **`mode != Tile` ⇒ no `frac`, `FLAG_TILED` zero** | S5's own goldens `ui_flipbook` + `ui_nine_slice_tiled` |
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
own sub-rect**, selected by a new `FLAG_TILED` bit out of the free bits 5..19 (S-D2's budget), with
the tile count folded into `uv` at pack exactly as S4 originally proposed. Concretely the sprite
branch computes `uv = sub_min + frac(t) * (sub_max - sub_min)` instead of `uv = lerp(sub_min, sub_max, t)`.

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
cannot *gate* it: the mechanism is a **shader** change (an eDSL leaf, a re-emit, a re-DXC, two
`SpirvBlob<N>` lengths, two manifest rows) and S4 is otherwise a pure CPU rung — that "no shader
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
`fill_center == true`), sub **10** the image. A nine-sliced node emits **10** records (9 with
`fill_center == false`), **11** with an image (10 without the centre). A node with no `UiNineSlice`
emits exactly what it emits today — 1, or 2 with an image — so S-D8's default-OFF row holds byte for
byte. `UI-ADVANCED-ARCHITECTURE.md:620-621` owes the same strike-and-correct; this plan is the
authority until it gets it.

---

## 4 · The rung ladder

**Unconditional gate on every rung:** `cargo clippy -p <crate> --all-targets -- -D warnings`;
`cargo test -p <crate> --all-targets --no-fail-fast` for every crate touched, plus
`cargo test --workspace --all-targets --no-fail-fast` before the rung is called done; Miri where new
`unsafe` lands; **the `*_spv_sync` tests run locally with `dxc` present and the result reported — a
SKIP is not a pass**; all 32 `goldens/PINS.toml` hashes unchanged (no rung here touches a scene
render); author-only commit.

> Disk discipline in this worktree: build with `-p <crate>`, never `--workspace`, except for the
> final pre-commit sweep. An `os error 112` or a compiler ICE is the disk —
> `rm -rf target/debug/incremental`.

---

### S0 — the seam, the gate, the observer — **size L**

*Architecture D31 + D6a + D6b + D32. This is the first landable rung.*

**Lands.**

1. `boyko-ui` moves from `[dev-dependencies]` to `[dependencies]` in `boyko_render/Cargo.toml`; the
   `TEST-ONLY` annotation at `:106-118` is rewritten to record the promotion and its reason (the
   layering rule at `boyko_render/Cargo.toml:7-13` names this crate as the home of per-entity GPU
   data paths).
2. **`ui_pack_inputs!`** — a `macro_rules!` in `boyko_render::ui` that expands to **both** the
   `Or<(Changed<…>, …)>` filter type of `ui_render_discovery` **and** the gather's per-node read list,
   from **one** spelling. At S0 the list is the four that exist today:
   `ComputedRect`, `UiBackground`, `ComputedClip`, `StackIndex`. Animation adds `UiVisual` here;
   sprites add four here at S3–S5; interaction adds its scroll datum here.
3. **`boyko_render::ui::gather_ui_nodes`** — the canonical gather, a DFS over `UiRoot`/`Children`
   carrying the inherited clip on its stack, mirroring `collect_candidates` (`focus.rs:204-257`)
   line for line. Its pre-order **is** the paint order D4 pins, so the renderer's z-order and the
   hit-test's `paint_seq` are one traversal rather than two that must be kept in agreement.
4. **`ui_render_discovery`** — one normal system whose `Query<(), ui_pack_inputs!(changed)>` bumps
   `UiRenderGeneration` once per changed frame. One site, not fifteen.
5. **The two-phase seam** (the architect's 2026-08-21 WorldView ruling — sequence, never fuse,
   inside one `UiUploadSystem::run_dispatcher`, mirroring the shipped `GpuSystem` ordering):
   **Phase 1 (shared borrow)** — the per-slot `[u64; FRAMES_IN_FLIGHT]` generation gate, hoisted
   AHEAD of the gather (D6a; a static frame costs one `u64` compare and **zero** component probes —
   the structural skip), then `gather_into_staging(&mut self, view: &WorldView<'_>) -> usize`
   (gather + pack + z-sort into the system-owned staging `Box`, no `!Send` type in the signature,
   device-free, unit-testable with a bare `EcsMaster`), the view dropped at the phase's closing
   brace; **Phase 2 (exclusive borrow)** — `token.nonsend_resource_mut::<RhiContext>()`, then
   `upload_staging(rhi, packed, ortho, token)` (no `WorldView` in the signature, so the fusion
   cannot be re-written). `self.staging` is a preallocated `Box<[UiInstance]>` sized at
   `initialize`, never grown in the frame loop (the Principle-0 named legitimate exception: the
   staging mirror for a GPU-contiguity write; durable data stays in ECS columns). The fused
   predecessor `host_upload_frame_from_world` is **DELETED, not re-signed** — its parameter list
   WAS the defect (see the fact table).
6. Two **diagnostic** counters (not `#[cfg(test)]` — the §10.4 `relayout_count` lesson): probes/frame
   in `gather_ui_nodes` and repacks/frame in `pack_sort_upload`. The two-phase seam carries its own
   pack census on the system (`UiUploadSystem::repacks`), because Phase 1 reads the world through a
   read-only `WorldView` that cannot project `&mut` to a `Resource`.
7. **The observer — re-pointed at Phase 1 alone** (the 2026-08-21 ruling): a device-free test on a
   bare `EcsMaster`, no graphics type in sight — one hardcoded panel, discovery + the two-phase
   dispatch, the staged records asserted **by value** (scale folding, z-order, packed count) in
   `tests/ui_s0_seam.rs`. Cheaper than a windowed rung AND unit-testable. The windowed `boyko_app`
   UI rung (D32's floor) is deferred to the rung that makes the swapchain `Renderer` an ECS
   resource — until then the host drives the seam via
   `stage_frame(token, ortho)` → `run_system_once` → `take_frame_output()`.

**No instance change. No shader change. No `.spv` change.**

**Gate.**

| # | Claim | How |
|---|---|---|
| **G0-1** | The gather reads every pack-input component, and cannot drift | Compile-level: `ui_pack_inputs!` is the only spelling. Plus a behavioural test — a world with one node, mutating each pack-input component in turn, asserting the generation bumps **exactly once** per mutation and **zero** times on an unrelated component. |
| **G0-2** | The structural skip is Phase 1's contract | Asserted on the **COMMAND CENSUS**, not a timing delta: 10 consecutive static dispatches of the two-phase `run_dispatcher` record **zero** on both census counters — zero component probes, zero packs — i.e. zero recorded work on an unchanged generation (`ui_s0_seam.rs::g0_2_static_frames_record_zero_census`). |
| **G0-3** | The packed-count return crosses the seam | Phase 1's contract: mutate one node once ⇒ the next dispatch packs **exactly once**, `gather_into_staging`'s packed-count is observable off the system (the count AND the repacked row carrying the new value — not a stale re-serve), and the dispatch after that skips again (`ui_s0_seam.rs::g0_3_one_mutation_one_repack_count_returned`). |
| **G0-4** | The DFS carries the inherited clip and its pre-order is paint order | A three-level tree with a clip at the middle level: the leaf's packed `clip` is the ancestor's, and the emitted `append` order equals `collect_candidates`'s `paint_seq` for the same tree. |
| **G0-5** | **THE SEAM GATE: the fusion is unrepresentable** | Two halves. (1) Signature pins: Phase 1's signature names **no** `!Send`/graphics type, Phase 2's names **no** world type — pinned by fn-pointer coercions that stop compiling if either signature grows the other phase's type (`ui_s0_seam.rs::g0_5_seam_signatures_do_not_cross`). (2) A trybuild fixture where **re-fusing them fails to compile**: one call site holding Phase 1's `WorldView` live across Phase 2's `&mut RhiContext` projection is E0502 (`tests/ui_s0_seam_fusion/refused_refusion.rs`, blessed `.stderr`). That makes the fusion unrepeatable rather than fixed. |

**Red mutations.**

* **M0-a — hoist Phase 1's braces (delete the view drop).** The 2026-08-21 ruling re-specified this
  as "⇒ E0502 AT COMPILE TIME — a build failure, stronger than a runtime red; pins the brace as
  load-bearing". **That claim is REFUTED BY THE COMPILER** (probed the same day, ledger in the
  landing report and `docs/OPEN-QUESTIONS.md`): under NLL the view's borrow ends at its **last
  use** (`gather_into_staging(&view)`), so the hoisted-brace form **compiles clean — exit 0, no
  E0502, no warning**. What the brace actually is: scope hygiene against a future edit that HOLDS
  the view across Phase 2 — and the compile-time tripwire on *that* shape is real and demonstrated:
  it is M0-b below and G0-5's trybuild fixture, both of which red. The brace's comment in
  `upload.rs` records exactly this so the brace is not deleted as decorative. *(The pre-ruling
  M0-a — collapse the per-slot gate to a scalar ⇒ a skip serves the sibling slot's stale ring — is
  retired as a plan mutation; its rationale is pinned at the gate's field doc in `upload.rs`.)*

  **Disposition (orchestrator, same day): M0-a is RETIRED as a mutation.** Its premise is false and
  its property is covered twice over — at compile time by M0-b (the borrow crossing the seam) and
  structurally by G0-5's re-fusion fixture. A mutation kept alive after its predicted red is proven
  unfireable would be this plan's own gate-that-cannot-fail class, curated rather than caught.
* **M0-b — read the generation from the view across Phase 2 instead of from self.** ⇒ **E0502**,
  demonstrated 2026-08-21 (ledger): `cannot borrow 'token' as mutable because it is also borrowed
  as immutable` — the view minted in Phase 1, the `&mut` projection at Phase 2's head, the
  view-read after it named as "immutable borrow later used here". *Pins that the gate's RESULT
  crosses the seam — the packed count, a copied `u64` — never its borrow.* *(The pre-ruling M0-b —
  move the compare into `pack_sort_upload`, splitting the two counters — is retired as a plan
  mutation; the two-counter split it argued for is now G0-2's census design, asserted on both
  halves.)*
* **M0-c — delete `ComputedClip` from `ui_pack_inputs!`.** The build fails at the gather. *A
  completeness test that checks a hand-kept list against the gather is checking a list against itself;
  only one spelling can fail to compile.*
* **M0-d — make `ui_render_discovery` bump unconditionally.** G0-2 reds (`repacks == 10`). *Proves the
  discovery filter is doing work rather than the counter being incidentally zero.*

**Measurement.** §10.8 legs **(a)** baseline probes/node/frame and gather µs at N ∈ {256, 2048}, and
**(d)** the static frame with the compare hoisted — which must read **zero probes** or D6a is not
wired where it claims to be. §10.3: repacks avoided on a static frame **and** the unchanged full cost
of a changing frame, both reported, so the module doc never again claims more than the mechanism
delivers. **Both previously-blocked legs are re-pointed at `run_dispatcher` as the bracket** (the
two-phase seam, driven through `run_system_once` — headless; `ui_s0_measure.rs`). **Scope note:**
Phase 2's upload cost is DRAW-adjacent host+transfer on the windowed device — compare only within
one scene, and name the GPU zone id before quoting a number; that half is the owner-run windowed
leg, not the headless harness. *Landed 2026-08-21 (this box, debug profile, Instant/QPC ~0.1 µs
floor): leg (d) static dispatch median 0.2 µs @ N=256 / 0.4 µs @ N=2048, probes = 0 asserted,
repacks avoided 100/100; §10.3 changed-frame full cost median 231.7 µs @ N=256 / 2250.3 µs @
N=2048 — the gate does not reduce it, and now the number says so.*

---

### S1 — the UI shader into the eDSL; both sync gates; manifest rows — **size M**

*Architecture D30. Sequenced before S2 per **R1**: the re-DXC gate must exist before the shader is
edited, or nothing notices a `.spv` that stops matching its source while the shader is being changed
repeatedly.*

**Lands.**

1. `boyko_shaderdsl/src/ui.rs` — the leaves, each one generic `C: Cf` body instantiated over `f32`
   (the host oracle) and `Emit` (the HLSL printer):
   `ui_sd_rounded_box` (the per-corner Quilez/Bevy SDF), `ui_clip_coverage`, `ui_median3`,
   `ui_screen_px_range`, `ui_premultiplied_over` (the border-over-fill composite),
   `ui_unpack_rgba8`.
2. `boyko_shaderdsl/src/bin/emit_ui.rs` — owns **both whole files** (S-D10), with the `UiInstance`
   field offsets, `UI_INSTANCE_SIZE`, and the three flag-bit constants as **generator inputs**, so no
   shader ever spells a byte offset that a host `offset_of!` also spells.
3. Sentinels in `ui_rect.vs.hlsl` / `ui_rect.fs.hlsl`.
4. `crates/boyko_render/tests/ui_rect_edsl_sync.rs` and `ui_rect_spv_sync.rs` — in `boyko_render`,
   because that is where the shaders live (`boyko_render/shaders/`), following the
   `particle_edsl_sync.rs` plumbing verbatim (`shaders_dir()`, LF normalization, `find_dxc()`,
   graceful SKIP with an `eprintln`).
5. Two rows in `docs/SHADER-VARIANT-MANIFEST.md` — `ui_rect.vs.spv` and `ui_rect.fs.spv`, both
   **no-`-D`-axis** rows, stated as such, with the frozen dxc recipe from each source's header
   (`-T vs_6_0` / `-T ps_6_0`, `-fspv-target-env=vulkan1.3`).

**Gate.**

| # | Claim | How |
|---|---|---|
| **G1-1** | Every generated span **is** the printer's output, and is inside the right function | `ui_rect_{vs,fs}_matches_edsl_emit` |
| **G1-2** | Each committed `.spv` **is** the re-DXC of its own source under the frozen recipe | `ui_rect_{vs,fs}_spv_byte_identical` — SKIPs without dxc, **and the skip is reported** (`PARTICLES-PLAN.md` F15) |
| **G1-3** | The `f32` oracle agrees with the emitted math | `ui_sd_rounded_box::<f32>` against a table of `(p, half_size, r)` points including all four quadrants and the degenerate `r == 0`; `ui_median3::<f32>` against the three orderings |
| **G1-4** | The `.spv` did not move | `SpirvBlob<2368>` / `SpirvBlob<7060>` unchanged |
| **G1-5** | The four UI goldens are unchanged | S-D6's hashes (landed at S2) do not exist yet at S1 — so at S1 the existing **texel** assertions plus G1-4's byte lengths are the gate, and this weakness is why S-D6 lands with S2 and not later |

**Red mutations.**

* **M1-a — change the `1e-5` `fwidth` floor to `1e-4` in the leaf and re-emit.** G1-2 reds (`.spv`
  bytes move). *Proves the byte gate is live and not a length check.*
* **M1-b — hand-edit one character INSIDE a sentinel span without re-emitting.** G1-1 reds. *Proves
  the eDSL owns the span.*
* **M1-c — hand-edit one character OUTSIDE every sentinel, in the skeleton.** G1-1 stays **green** and
  G1-2 reds. *This is the mutation that says why both tests exist: neither alone covers the file.*
* **M1-d — remove `dxc` from `PATH`.** G1-2 **SKIPs**. *This is not a defect; it is the rung's
  vacuum condition, and the rung is not called done on a skipped run. The mutation exists so that the
  skip is seen at least once by the person who has to report it.*

**Measurement.** §10.7 — byte identity of the re-emitted HLSL and the re-DXC'd `.spv`. This gate does
not exist today in **any** form.

**Honest note.** S1 delivers no user-visible change and is the rung most likely to be skipped under
pressure. It is sequenced third and not last because S2 is a six-site lockstep edit to a file that
today has *no gate proving its binary matches its source*.

---

### S2 — `UiInstance` 64 B → 80 B, all ten sites, one commit — **size M**

*Architecture D1. No feature lands here. This rung exists so that neither feature half widens the
record twice.*

**Lands.** The field list from D1, verbatim:

```
@0   min_px        [f32; 2]
@8   size_px       [f32; 2]
@16  clip          [f32; 4]
@32  corner_radius [f32; 4]   // ALWAYS the radius — the alias is retired
@48  uv            [f32; 4]   // NEW: normalized (u0,v0,u1,v1) — glyphs AND sprites
@64  color         u32
@68  border_color  u32
@72  border_width  f32
@76  flags         u32        // S-D2's bit assignment
                              // = 80 B, multiple of 16, no tail pad
```

**Lockstep sites — all ten, in one commit:** the Rust struct; `UI_INSTANCE_SIZE`; the ~~**ten**~~
**nine per-field** `offset_of!` const-asserts *(amended 2026-08-21 at landing: D1's field list has
NINE fields, so nine per-field asserts — plus the `size_of == 80` and `align_of == 16` pins beside
them. The "ten" carried forward the fact table's counting convention, which called today's eight
per-field asserts "9"; the same off-by-one, twice)*; S-D2's `BINDLESS_TEXTURE_CAPACITY` const-assert;
the `UiInstance` mirror in
`ui_rect.vs.hlsl`; the mirror in `ui_rect.fs.hlsl` (both now via `emit_ui.rs` — the offsets are
generator inputs, so this is one edit, not two, which is S1's dividend); the two `SpirvBlob<N>` byte
lengths; `pack_ui_instance`'s text branch (writes `uv`, leaves `corner_radius` zero); the Miri
byte-view test.

**Also lands: S-D6's four image hashes**, blessed on the 64 B build **first**, in a preparatory commit,
so "identical" has a referent. The two-commit protocol is the rung:

* **commit A** — add the SHA-256 assertion to each of the four UI goldens, blessed against the
  **current 64 B** build, with the BMP dumped and looked at;
* **commit B** — the widening, which must reproduce all four hashes.

**Gate.**

| # | Claim | How |
|---|---|---|
| **G2-1** | Rust layout is exactly D1's | ~~ten~~ **nine** `offset_of!` asserts *(one per field — the 2026-08-21 count amendment above)* + `size_of == 80` + `align_of == 16` |
| **G2-2** | The 12-bit slot field cannot silently truncate | S-D2's `BINDLESS_TEXTURE_CAPACITY <= 1 << 12` const-assert |
| **G2-3** | The widening is pixel-invisible | The four UI goldens reproduce commit A's hashes exactly |
| **G2-4** | The byte view is sound at the new size | the Miri byte-view test over an 80 B record |
| **G2-5** | The text lane really migrated | a CPU test asserting a `FLAG_TEXT` instance now carries the UV in `uv` and **zero** in `corner_radius` |
| **G2-6** | The reserved bits are actually zero | a CPU test asserting `flags & 0xFFFF_FFF8 == 0` for every packed instance at S2 |

**Red mutations.**

* **M2-a — swap `uv` and `corner_radius` in the *Rust* struct only.** G2-1 reds.
* **M2-b — swap them in the *HLSL* mirror only.** G2-1 stays **green**; G2-3 reds. *This is the
  mutation **R1** says nothing in the tree can currently see. It is visible only because S1 put the
  `.spv` under a byte gate and commit A put the image under a hash. If M2-b does not red, the rung's
  gate is decorative and the rung is not done.*
* **M2-c — raise `BINDLESS_TEXTURE_CAPACITY` to 8192.** G2-2 reds. *Proves the assert is against the
  live constant and not a copy of it.*
* **M2-d — leave the text lane writing `corner_radius`.** G2-5 reds and G2-3 reds (the glyph samples
  `uv == (0,0,0,0)`). *Proves the un-aliasing is complete rather than additive.*

**Measurement.** §10.2 — ring traffic 128 KB → 160 KB at N = 2048 (arithmetic, stated as such) **plus**
wall-clock pack+sort at N ∈ {256, 2048}, same scene, 64 B vs 80 B, criterion, median of the two builds.
The bytes are arithmetic; the time is not, and only the time can say whether the sort's gather (which
touches every record twice) notices.

*Landed 2026-08-21 (this box, RTX 3060 host, criterion 0.5 bench profile,
`benches/ui_pack_sort.rs` — one deterministic scene, 16 stack strata, every 4th node clipped, half
rounded): arithmetic 2048 × 64 B = 128 KiB → 2048 × 80 B = 160 KiB (+25 % bytes, touched twice by
the sort gather). Wall-clock medians 64 B → 80 B: **5.62 µs → 5.71 µs @ N=256 (+4.3 %)**,
**49.12 µs → 51.16 µs @ N=2048 (+4.7 %)** — the sort's gather notices the widening at a fifth of
the byte growth; the pack is compute-, not bandwidth-, bound at these N. Affordable; D1 stands.*

*Landing notes, 2026-08-21, recorded per S-D10/SR1 (the reconciliations the rung surfaced):*

* *The `.spv` MOVED, deliberately — this rung is the shader-surface edit S1's gates exist for.
  `ui_rect.vs.spv` 2368 → 2408 B (the mirror gains the `uv` member the VS declares and never
  reads); `ui_rect.fs.spv` 7060 → 7136 B (the mirror + the `FLAG_TEXT` branch reading `inst.uv`
  instead of the retired alias — the rung's whole semantic delta). The generated-HLSL diff was read
  before the re-bless and contains exactly those two changes; the reason is recorded at both
  `SpirvBlob<N>` pins (`src/ui/mod.rs`).*
* *Commit A blessed the four S-D6 hashes on the 64 B build (BMPs dumped and looked at); commit B
  reproduced all four EXACTLY on the 80 B build — G2-3 held with zero re-blessing.*
* *S-D6 refinement for the swapchain golden: its "full readback" is the WSI-owned frame, whose
  extent and byte order the driver decides (this box clamps the requested 64×64 window to
  **120×64 BGRA**), so that pin carries `(extent, is_bgra)` qualifiers beside the hash and is
  asserted only when the live shape matches the blessed one — a mismatched WSI shape prints a loud
  NOTE rather than silently passing-as-checked. The three offscreen goldens pin fixed 64×64 /
  128×128 RGBA frames and carry the bare hash, as written.*
* *`UI_STAGING_ROWS`' doc arithmetic followed the stride (4096 × 80 B = 320 KiB — the S0 seam's
  staging box, untouched otherwise).*
* *An ELEVENTH lockstep site the ten-site list missed, found by the unconditional full-suite gate
  (`cargo test -p boyko-render --all-targets --no-fail-fast`), not by the enumerated ten:
  `ui_hud_screenshot.rs::hud_glyph_packing_golden` — a device-free test in the (otherwise
  `#[ignore]`d ×8) HUD binary that PINNED the retired contract verbatim ("the GPU pack lane
  carries that same UV into the `corner_radius` alias"). Its pin migrated with the field
  (`inst.uv == expected`, `corner_radius == [0;4]`). The lesson is the fact table's own: a
  grep-shaped site census misses a consumer that names the CONTRACT rather than the field.*

---

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
   re-spliced, both `.spv` re-DXC'd and re-committed, the two `SpirvBlob<N>` lengths updated, the two
   `SHADER-VARIANT-MANIFEST.md` rows updated (still no `-D` axis), and `ui_rect_{vs,fs}_spv_sync`
   re-run with dxc present.
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

**Lands.**

1. ~~`UiNineSlice { border_px: [f32;4], mode: u8 /* Stretch|Tile */, fill_center: bool }` — 20 B,
   `#[repr(C)]` POD, padding spelled.~~ **`UiNineSlice { border_px: [f32;4], mode: u8, fill_center: bool, _pad: [u8; 2] }` — 20 B,
   `#[repr(C)]` POD, `_pad` SPELLED, `mode` a `NineSliceMode` with exactly ONE legal value at S4
   (`Stretch = 0`), pinned by a variant-count `const` assert; an out-of-range discriminant is
   rejected at pack.** Table (authored, cold).
   *(amended 2026-08-21: (a) the original field list is 20 B — **verified by compiling both spellings
   under rustc 1.97.1: size 20 / align 4 with and without `_pad`** — but those two bytes were IMPLICIT
   TAIL PADDING, which is exactly what "padding spelled" forbids; S5's own `UiSheet` at the next rung
   spells its `_pad` with a reason, and this one now matches its own prose. (b) `Tile` moves to S5 —
   S-D11.)*
2. ~~The pack emits **9** sub-quads (8 with `fill_center == false`) into the **existing**
   `UiRenderScratch.pack` when the component is present, and **1** when it is absent.~~
   **The pack emits the node's background rect at sub 0 (unchanged from S3) PLUS 9 sub-quads at subs
   1..=9 (8 when `fill_center == false` — the centre, sub 5, is the one that is skipped), in BOTH pack
   loops.** All nine inherit the parent's `StackIndex` and `ComputedClip` verbatim and take
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
   with two append encodings; the item named one. The expansion lands in BOTH.)*
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
   glyphs → focus ring*, per node~~ **at S4, over the terms S4 emits: *background rect → nine-slice
   sub-quads (TL..BR, centre at sub 5) → image*, per node.** The two remaining terms of D4's full
   contract keep their home in D4 and are pinned at the rungs that emit them: **glyphs are not
   per-node sub-records at all** — the canonical gather hard-codes `text_uv: None` (`gather.rs:272`)
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
8. **`UI_STAGING_ROWS` is re-derived from a stated node budget, and its doc comment is corrected**
   *(added 2026-08-21)*. The in-schedule path packs into a FIXED `Box<[UiInstance]>` of
   `UI_STAGING_ROWS = 4096` (`upload.rs:107`, `:571`) whose overflow arm is `debug_assert!(false, …)` —
   a debug **panic** — then a release `truncate` of the emission TAIL with `staging_overflows` bumped
   (`upload.rs:346-359`). Its doc claims "2× the plan's own N = 2048 measurement scene": **that was
   already false at S3** (2 records/node × 2048 nodes = 4096 = exactly 1×), and at stride 11 the box
   overflows at 187 nine-sliced nodes. Replace with
   `UI_MAX_NODES: usize = 2048; UI_STAGING_ROWS: usize = UI_MAX_NODES * UI_RECORDS_PER_NODE as usize`
   — 22 528 rows × 80 B = **1.72 MiB**, one host allocation at `initialize`, never grown, never walked
   beyond the live prefix. **Reason for paying it rather than sizing for a "typical" mix:** a box sized
   for a typical composition overflows as a function of *what the scene contains*, which is precisely
   the composition-dependent silent truncation the clamp exists to make loud. A constant that cannot
   overflow within the stated node budget is worth 1.4 MiB of host RAM. *(The GPU ring needs no change:
   `UiRingSlot` is grow-only pow2 on overflow — `resources.rs:190-203` — so the CPU box is the sole
   hard cap.)*

**Gate.**

*(the whole table was re-pointed 2026-08-21 — three of the five rows named a subject that cannot be
constructed at S4; each row below states **which pack loop it drives**, because the rung has two and
S3's ledger already recorded that naming one leaves the other ungated)*

| # | Claim | How |
|---|---|---|
| **G4-1** | The expansion is 9 (or 8) sub-quads **in addition to** the background rect, consecutive, inheriting *(S-D11 — "in addition to" is the ruled reading of a contradiction, not a restatement)* | ~~CPU unit test, no GPU: assert record count, that `append` is `k..k+9`~~ **CPU unit test, no GPU, run against BOTH loops: for `gather_into_staging` via `sys.staged()` on a bare `EcsMaster` (the device-free precedent is `ui_s0_seam.rs:251`, whose own doc anticipates this rung), and for `pack_sort_upload` via `UiRenderScratch`. Assert the record count DERIVED from `UI_RECORDS_PER_NODE` and `fill_center` — never a literal — that the sub-quads occupy consecutive `append` codes `base+1..=base+9` (`base+5` absent iff `fill_center == false`)**, that all nine carry the parent's `StackIndex` and clip |
| **G4-2** | The emission order is D4's | ~~A node with background + nine-slice + image + glyphs + focus ring~~ **A node with background + nine-slice + image** *(amended 2026-08-21: glyphs are not per-node sub-records — `gather.rs:272` hard-codes `text_uv: None` and `pack.rs:208` forbids one record being both — and `FocusRing` has zero occurrences in `crates/`; it is Interaction's I9. As written this gate's subject was unconstructible)*: assert the `append` lane's order equals the contract, **by name**, off `sys.staged()` — the shape `ui_s0_seam.rs:288-302` already uses to assert staged records by `FLAG_TEXTURED` |
| **G4-3** | Slicing preserves corners | GPU golden `ui_nine_slice`: a 3×3 procedural source (S-D5) ~~stretched to a 64×16 rect~~ **whose nine cells carry NINE DISTINCT values, stretched to a 96×96 rect at `border_px = [16,16,16,16]`** *(amended 2026-08-21, two reasons. (a) A **symmetric** source makes region assignment unobservable: the natural corner=A/edge=B/centre=C source is invariant under the full dihedral group, so all 24 corner permutations hash identically — and the existing S-D5 checkerboard is itself invariant under 180° rotation and transpose (`ui_sprite_gpu_golden.rs:117-124`). Nine distinct values make every region individually visible, which is what M4-e needs. (b) At 64×16 from a 3×3 source, a correct corner is **one destination pixel**, so "matches the source's corners 1:1" degenerates to a single-texel assertion that is exact only because a 1-px quad's centre lands at u = 1/6 — any half-texel convention error blends instead of failing. 96×96 at 16 px borders gives every corner real width.)*; the four corner regions are **unstretched** and the edges are; image hash pinned |
| **G4-4** | The pack still never reallocates | ~~`ui_no_realloc.rs` extended: the 9× expansion at N=1024 nodes must not grow the scratch after the first frame~~ **`ui_no_realloc.rs` extended at its own `N = 4096`** *(amended 2026-08-21: there is no N=1024 configuration in that file — it runs `N = 4096` at `:102` and `:191` and `WARM_N = 2048` / `STEADY_N = 16` at `:148-149`; the gate named a scene that does not exist)*, **driving the expansion through the production emitter rather than the test's own hand-rolled loop** *(its `build_frame` at `:83-93` calls `pack_ui_instance` directly and pushes keys by hand, so "extending it" would re-implement the expansion policy inside the test and gate the test against itself — S4 exposes the expansion as a callable seam and G4-4 calls it)*. The steady-state half of this file is sound as it stands and needs no work: a 3-frame warm-up whose allocations are excluded (`:108-110`), capacities captured (`:112-114`) and asserted byte-stable in an armed window (`:118-134`) |
| **G4-5** | ~~S-D7 is enforced~~ **`mode` has exactly one legal value at S4** | ~~`Tile` + `UiSpriteSheet`: `debug_assert!` fires in dev; the release build clamps to `Stretch` and the counter increments~~ **A `const` assert on the variant count plus a CPU test that an out-of-range `mode` discriminant is rejected at pack.** *(amended 2026-08-21 — **this was a gate that could not fail.** Its subject `UiSpriteSheet` has **zero occurrences in any `.rs` file in the tree**; S5's Lands item 2 creates it. A gate whose subject a later rung introduces cannot be written, therefore cannot fail — the exact class S3's M3-e already exhibited, caught here before the code rather than after. S-D7 is retired (S-D11) and the tiling half moves to S5; what remains at S4 is the half that HAS a subject: `mode` is a one-variant enum and the rung says so mechanically, so S5 widens the value set instead of re-specifying the field.)* |
| **G4-6** | The staging box holds the stated node budget | *(added 2026-08-21 — nothing in the original table looked at the box that actually truncates.)* Drive `gather_into_staging` with `UI_MAX_NODES` nine-sliced, imaged nodes: `sys.staged()` equals the derived emission count, `staging_overflows == 0`, and no `debug_assert!` fires. The original G4-4 could not see this: it drives `UiRenderScratch`, a growable `Vec`, while production packs into a fixed `Box` that **clamps** rather than grows |
| **G4-7** | `UiNineSlice` reaches the renderer at all | *(added 2026-08-21 with Lands item 6.)* The `ui_s0_discovery` shape: mutate `UiNineSlice` on a live node and assert `UiRenderGeneration` bumps **exactly once**; assert the derived probe census is `ui_pack_inputs!(count) + 1` per node per frame. Without the macro edit the component is invisible to both halves and the frame silently does not repaint |

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
  amended 96×96 destination with 16 px borders a correct corner is 16×16 px and a proportional one is
  32×32, so four corners move ~3 000 of 9 216 px — far above any 8-bit hash threshold, unlike the ~1-ULP
  shader edits an 8-bit golden genuinely cannot see.)*
* **M4-c — ~~swap the image and glyph emission order~~ swap the image and the LAST sub-quad (BR)
  emission order.** G4-2 reds. *(respecified 2026-08-21: **the original mutation could not be applied
  at all.** There is no site that emits a glyph and an image into one per-node lane to swap — the
  canonical gather hard-codes `text_uv: None` (`gather.rs:272`), and `pack.rs:208`'s `debug_assert!`
  forbids one record being both. What would have been "observed" is that the mutation is unwritable,
  which is not a red. The swap of image against the last sub-quad is writable, fires the same gate, and
  tests the same property.)* *Proves the contract is pinned. Note that this mutation is **invisible**
  to every image gate unless the two quads overlap — which is why G4-2 asserts the order directly and
  not through a picture.*
* **M4-d — pre-`reserve` the 11× worst case at setup.** ~~G4-4 stays green but the scratch's
  steady-state capacity grows 9× for a world with one nine-sliced node. *Not a red — recorded as the
  tempting wrong fix, because the scratch is a `Resource` and the growth is permanent.*~~
  **CONFIRMED green-as-written, and therefore UPGRADED to a real red** *(2026-08-21)*: `ui_no_realloc.rs`
  asserts capacity **stability**, not magnitude — the warm-up check is a LOWER bound (`cap >= N`), the
  armed window compares against the warmed value, and a setup-time reserve is set once and allocates
  nothing inside the window, so all three pass. A mutation no gate can see is not a mutation. **G4-4
  gains an UPPER bound** (`assert!(scratch.pack.capacity() < 2 * emitted)`) — one line — and M4-d becomes
  a red like the others. *The tempting wrong fix is still worth recording as such: the scratch is a
  `Resource` and the growth is permanent.*
* **M4-e — permute which source region a sub-quad samples** *(added 2026-08-21)*: swap the TL and TR
  sub-quads' source UV sub-rects while leaving their **destination** rects correct. G4-3 reds. *Proves
  the golden sees **region assignment**, which the original four mutations left entirely uncovered:
  G4-1 asserts count/consecutiveness/inheritance (blind to UVs), G4-2 asserts record KIND order, G4-4
  is capacity, G4-5 is the enum's value set. Only the picture can see this, and only if the source
  breaks symmetry — which is why G4-3 now requires nine distinct cell values.*
* **M4-f — leave `UI_STAGING_ROWS` at 4096** *(added 2026-08-21)*. G4-6 reds: the box overflows at 187
  nine-sliced imaged nodes, `debug_assert!` fires in the test build, and in release the frame is
  silently truncated at the tail with `staging_overflows` bumped. *Proves item 8's constant is
  load-bearing rather than tidy — and that the gate looks at the box production actually packs into,
  not at the growable `Vec` the legacy loop uses.*

**Measurement.** *(added 2026-08-21 — S4 carried no measurement paragraph, and §5 assigns leg 10.8(c)
to "S3–S5". Every other rung states its obligation in the rung; S4 and S5 were the only two without
one.)* **§10.8 leg (c), next increment.** S3 established that the gather cost is the LIST getting
longer, not component presence — a probe returning `None` is still a probe — and landed at 5 pack
inputs + `Children` = **6.00** probes/node/frame. S4's `UiNineSlice` makes it 7 pack inputs' worth:
**6.00 → 7.00 (+16.7 %)**, paid by every node of every changed frame whether or not it is nine-sliced.
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
| 3 | G4-2 asserts a five-term order on one node's `append` lane | **SUBJECT UNCONSTRUCTIBLE.** Glyphs: `gather.rs:272` hard-codes `text_uv: None`; `pack.rs:208` forbids one record being both glyph and sprite. Focus ring: zero occurrences in `crates/`, and §6 assigns it to Interaction I9. Narrowed to the three terms S4 emits; M4-c respecified. |
| 4 | The record count is determined | **CONTRADICTION, three readings.** D8d `:620` = replace (9/1); D4 `:250` = add; `ui_s0_seam.rs:245` = "seven more". Ruled ADD by S-D11, with the reason (a nine-slice source is a translucent FRAME; the background is the surface it sits on). `UI_RECORDS_PER_NODE = 11`. |
| 5 | "S4's nine-slice raises `UI_RECORDS_PER_NODE` and nothing else at that site" | **FALSE, and the same sentence is already in the tree** at `pack.rs:183-184`. The key push is hard-coded to two (`upload.rs:337-343`) and the decode is binary with an `.expect` that **panics in release** for a nine-sliced node without `UiImage` (`upload.rs:367-373` + `pack.rs:205`). Lands item 7. |
| 6 | G4-4 gates the expansion against reallocation | **WRONG INSTRUMENT, WRONG N, AND SELF-GATING.** It drives `UiRenderScratch` (legacy-only, reached solely via `pack_sort_upload`), at N=4096 not 1024, through a `build_frame` that hand-rolls the emission. Production packs into a fixed 4096-row `Box` that **clamps**, not grows. Re-pointed; G4-6 added; M4-f added. |
| 7 | S4's Lands list is complete | **`ui_pack_inputs!` was missing.** The macro's own doc says "sprites add the rest at S4–S5"; §6 forbids bypassing it; S5's item 6 wires only its own three. Without it `UiNineSlice` is invisible to gather and discovery both. Lands item 6, gate G4-7. |
| 8 | M4-a proves an unstable sort over a non-total key is nondeterministic | **REFUTED BY MEASUREMENT.** `sort_unstable_by_key` over true nine-key tie blocks at n = 9…1800 returned byte-identical output on every repeat and across fresh processes (rustc 1.97.1). The mutation fires; its rationale is rewritten — `append` is the SOURCE ADDRESS, so the real failure is duplication and loss. |
| 9 | M4-d "stays green — not a red" | **CONFIRMED green, and that is the defect.** Upgraded to a red by one upper-bound assert. |
| 10 | `UiNineSlice { … }` is "20 B, padding spelled" | **HALF TRUE — verified by compiling both spellings**: 20 B / align 4 with and without `_pad`. The two bytes were implicit TAIL padding, which is what the prose forbids. `_pad: [u8; 2]` added. |
| 11 | "Layout is untouched" | **CONFIRMED, and structurally so** — `boyko_ui` takes no render dependency and never names `UiInstance` or the pack. Not a defect. |
| 12 | `UI_STAGING_ROWS`'s doc: "2× the plan's own N = 2048 scene" | **ALREADY FALSE AT S3** (2 × 2048 = 4096 = exactly 1×). Corrected and re-derived in Lands item 8. |

---

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

2. `UiSpriteSheet { sheet: u16, index: u16 }` — 4 B, table. Presence ⇒ the pack derives `uv` from the
   sheet table by **pure arithmetic** from `(cols, rows, index)` instead of reading `UiImage`'s
   `uv_min`/`uv_max`, and takes the slot from `UiSheet` rather than from `UiImage`.
3. `UiSpriteAnim { first: u16, last: u16, fps: f32, mode: u8, repeats: u8 }` — 12 B, table, **cold**:
   author-written, never system-written.
4. `UiSpriteCursor { elapsed: f32, frame: u16, dir: i8 }` — 8 B, **dense**: the only column the
   flipbook system writes per frame.
5. `ui_sprite_flipbook` — one system over `(UiSpriteAnim, UiSpriteCursor, UiSpriteSheet)` advancing
   `elapsed`, flipping `dir` at the ends for `PingPong`, and writing `index`.
6. `ui_pack_inputs!` gains the three components that affect the picture.
7. **`NineSliceMode::Tile`, inherited from S4 by the 2026-08-21 audit ruling (S-D11).** S4 lands the
   `mode` field with one legal value; S5 widens the value set. What S5 owes, and why it is cheap
   *here* and was not cheap at S4: the mechanism is `uv = sub_min + frac(t) * (sub_max - sub_min)` on
   the fragment shader's sprite branch, selected by a new `FLAG_TILED` bit out of the free bits 5..19
   (S-D2), with the tile count folded into `uv` at pack. That is **the same sub-rect arithmetic items
   1–2 above already build** for sheet frames, so it is one shader edit at S5 (eDSL leaf, re-emit,
   re-DXC, two `SpirvBlob<N>` lengths, two manifest rows) instead of two at S4 and S5 — and S4 keeps
   the "pure CPU rung, no shader change" property that is D8d's whole argument for CPU expansion over
   Bevy's separate pipeline. **`frac` inside the sub-rect wraps to the sub-rect, which IS a sheet
   frame**, so `Tile` + `UiSpriteSheet` is the correct picture rather than S-D7's hard error — the
   guard, the clamp and the diagnostic counter S4 was going to build are not built by anyone.
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
exposes. **If the animation plan has not landed, S5 reads `Time`'s real delta directly and the seam is
one function** — `ui_frame_delta(world) -> f32` — which the animation plan later replaces in one edit.
S5 is therefore **not blocked** on the animation plan; it is only less configurable without it.

**Gate.**

| # | Claim | How |
|---|---|---|
| **G5-1** | The frame UV is the stated arithmetic | CPU table test: `(cols=4, rows=4, index=6, inset_uv=(h,h))` → an exact hand-computed `uv` constant. Asserted against the constant, **not** against the implementation. |
| **G5-2** | The four modes are exactly right at the turns | A deterministic tick harness at fixed `dt` over 3 cycles per mode; the `frame` sequence pinned as a **literal array** in the test. |
| **G5-3** | The churn split is real | `Changed<UiSpriteAnim>` fires on an author retarget and **never** on a per-frame cursor advance. |
| **G5-4** | The cursor is dense and does not migrate | Insert/remove `UiSpriteCursor`, assert the entity's archetype id is unchanged (`dense_d2_routing`'s property, re-asserted at this consumer). |
| **G5-5** | It animates on the GPU | Golden `ui_flipbook_frame3`: a 4×4 procedural grid (S-D5) at a fixed tick count; image hash pinned. |
| **G5-6** | `frame_count < cols*rows` is honoured | `index >= frame_count` clamps and increments a diagnostic counter (trailing cells are never sampled). |
| **G5-7** | `Tile` actually tiles | *(inherited from S4, 2026-08-21 — S-D11.)* Golden `ui_nine_slice_tiled`: the same nine-distinct-value 3×3 source as G4-3, at a destination whose edge spans **4 whole tiles**; the edge shows four repeats of the source's edge cell, not one clamped streak. **This is the gate S4 did not have** — its table tested `Stretch` only (G4-3) and the `Tile`+sheet clamp (G4-5), so half of a two-valued field would have landed ungated. |
| **G5-8** | `Tile` under a sheet stays inside its frame | *(inherited, and it is the assertion S-D7 could not make because it FORBADE the combination.)* A tiled nine-slice on a node carrying `UiSpriteSheet`: every sampled texel lies within that frame's sub-rect — no neighbouring frame contributes. `frac`-in-sub-rect makes this true by construction; the gate is what proves the construction. |

**Red mutations.**

* **M5-e — implement `Tile` as a UV past `[0,1]` instead of `frac` in the sub-rect** *(inherited from
  S4's retired mechanism, 2026-08-21)*. G5-7 reds with a clamped streak on every edge, and G5-8 reds
  under a sheet. *Proves S-D11's mechanism is load-bearing and re-runs, as a red, the exact thing S-D7
  believed was the only option — the UI's sampler is `ClampToEdge` in both modes, so the retired
  mechanism does not even reach the sheet hazard it was designed around: it fails one step earlier.*
* **M5-a — merge `UiSpriteAnim` and `UiSpriteCursor` into one component.** G5-3 reds — the change tick
  fires every frame. *This is the mutation that makes D8a a measurement rather than a preference: the
  merged shape destroys `Changed<UiSpriteAnim>` as a signal, and nothing else in the ladder would
  notice.*
* **M5-b — drop `inset_uv`.** G5-1 reds, and G5-5's sampled frame-edge texel takes the neighbouring
  frame's colour under LINEAR filtering. *Proves the half-texel inset is protecting something.*
* **M5-c — make `UiSpriteCursor` a table component.** G5-4 reds. *Proves the storage claim is
  enforced.*
* **M5-d — flip `dir` one frame late at the `PingPong` turn.** G5-2's pinned array reds. *The classic
  flipbook off-by-one, and the reason G5-2 pins a literal sequence: an eyeball check of "it animates"
  cannot see it, and neither can an image golden at a single tick count.*

---

### S6 — the `.ui` authoring landing for the sprite vocabulary — **size S**

*Behind **D7**, which is owned by [`UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md). The only rung here that is.*

**Lands.** `UiNineSlice`, `UiSpriteSheet` and `UiSpriteAnim` join the `.ui` vocabulary table (three
authored components, five landings each under today's hand-written path, one registration each under
D7). `ImageBundle` gains the optional members. `UiSpriteCursor` **deliberately does not opt in** — a
`.ui` file must not be able to inject a running cursor into a live world, which is the same
structural-safety property `parse_and_insert` already claims for its closed `match`
(`text/dispatch.rs:5-8`).

**Gate.**

| # | Claim | How |
|---|---|---|
| **G6-1** | Round trip | parse → spawn → `serialize_ui` → identical bytes, for all three |
| **G6-2** | Hot reload preserves them | the `TextStruct` reconcile path, per component |
| **G6-3** | Runtime state is not authorable | a `.ui` file naming `UiSpriteCursor` produces an "unknown component" `UiParseReport` diagnostic at the right line and column |

**Red mutations.**

* **M6-a — add `UiSpriteCursor` to the vocabulary table.** G6-3 reds. *Proves the exclusion is
  enforced rather than documented — D7a calls this a safety property, and a safety property with no
  test is a sentence.*
* **M6-b — omit the `TextStruct` impl for `UiNineSlice`.** G6-2 reds: the component silently
  disappears on reload. *This is the exact silent failure D7 exists to remove; seeing it red once is
  what makes the registration table's value concrete.*

**Fallback if D7 slips (R4's residual risk).** The three components take hand-written arms — a `.ui`
dispatch arm, a field parser, a `serialize.rs` arm, a `reload/reconcile.rs` `TextStruct` impl, and an
equivalence-gate row, each — and S6 lands anyway, fifteen landings heavier and no worse than the status
quo. **The sprite ladder is never blocked behind D7.** S0–S5 do not touch the `.ui` surface at all.

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

## 5 · Measurement obligations owned by this plan

Every number is named with its instrument and its **discriminating comparison**. None of them exists
today. The recorded failure mode this table exists against is a gate that could not fail and a number
that was not measurable.

| # | Claim under test | Instrument | Discriminating comparison | Rung |
|---|---|---|---|---|
| **10.1** | D2's `NonUniformResourceIndex` divergence is affordable | GPU timestamp around the UI pass | N textured quads at **1 / 8 / 64** distinct slots vs **all on one slot**, N ∈ {256, 2048} | S3 |
| **10.2** | D1's widening is affordable | ring bytes/frame (arithmetic: 128 KB → 160 KB at N=2048) **plus** criterion over pack+sort | same scene, 64 B vs 80 B build | S2 |
| **10.3** | The D6 gate does what its doc says — **and what it cannot do** | the repack counter | static frames: repacks before vs after; **and** a changing frame, reported **unchanged** | S0 |
| **10.7** | The eDSL migration is faithful | `ui_rect_edsl_sync` + `ui_rect_spv_sync` | byte identity of re-emitted HLSL and re-DXC'd `.spv` | S1 |
| **10.8** | **The gather** — the one cost this campaign adds to every node of every frame | a probe counter in `gather_ui_nodes` **plus** wall-clock over the gather alone, separated from pack+sort | probes/node/frame and gather µs at N ∈ {256, 2048} in four states: **(a)** today's rect-only baseline; **(b)** + `UiVisual`; **(c)** + the sprite components; **(d)** a **static** frame with the D6 compare hoisted — which must be **zero probes** | (a),(d) S0 · (b),(c) S3–S5 |

*(clarified 2026-08-21 at the S4 audit: leg **(c)** is an INCREMENT PER RUNG, not one number at the
end — S3 landed 5.00 → 6.00, S4 owes 6.00 → 7.00 (`UiNineSlice`), S5 owes 7.00 → 10.00 (its three).
S4 and S5 were the only rungs in this plan with no measurement paragraph of their own; S4's is now
written into the rung, and S5 inherits the same obligation.)*

§10.4, §10.5, §10.6 and §10.9 belong to the sibling plans and are not restated here.

**Where a rung reports a number, it reports the instrument's own resolution too.** The particles
campaign's recorded lesson stands: a delta smaller than the instrument's floor is not a small effect,
it is no measurement — and the floor of a GPU zone is not a constant across sessions.

---

## 6 · What this plan exposes to its siblings

Named explicitly, because three other plans build on these and a silent change here is a silent break
there.

| Exposed at | Surface | Consumer |
|---|---|---|
| **S0** | **`ui_pack_inputs!`** — the single spelling of the pack-input set. Adding a visual component to it wires the discovery filter **and** the gather read list together, or fails to compile. | **Animation** adds `UiVisual`. **Interaction** adds its scroll datum. Neither may add a component to the gather without adding it here. |
| **S0** | **`gather_ui_nodes`** — the DFS over `UiRoot`/`Children` carrying the inherited clip on its stack. Its pre-order **is** paint order. | **Interaction**: this DFS and `collect_candidates` are the same traversal; D19a's traversal-folded scroll offset rides this stack. |
| **S0** | `UiRenderGeneration` + the per-slot gate, hoisted ahead of the gather. | **Animation**: an animating frame bumps the generation every frame and the gate cannot help it — §10.3 reports that number unchanged, so the animation plan inherits an honest baseline rather than a claim. |
| **S0** | The **two-phase seam** (`gather_into_staging` / `upload_staging`, G0-5-pinned signatures) + the host drive protocol (`stage_frame` → `run_system_once` → `take_frame_output`). The `boyko_app` UI rung (D32's floor) is **deferred** to the Renderer-as-ECS-resource rung (2026-08-21 ruling) — the observer S0 ships is Phase 1, device-free. | **All three.** The device-free observer makes the seam's behaviour falsifiable on any machine; the human-visible floor arrives with the windowed rung, and until then **R1**/**R2**'s visual half rests on the owner-run windowed leg. |
| **S2** | `UiInstance` at 80 B with `uv` and S-D2's bit map. **Bits 5..19 are free; bit 4 is reserved.** | **Animation**: D5 folds the visual transform at pack and costs **zero** GPU bytes, so animation needs none of these bits. If it ever does, it takes bits 5..19 and says so here. |
| **S3** | `FLAG_TEXTURED`, the bindless slot lane, the UI sampler binding, font-optional boot. | **Aether**: the `ui` construct's `image` / `sheet` vocabulary can only name what S3–S5 built. |
| **S5** | The `u16 sheet_id` dense-handle mint. | **Aether**: the sheet-id mint is the natural thing for the construct to own at expand time (research §11 item 6). |
| **S4** | D4's pinned emission order — **over the three terms S4 emits (background → nine-slice TL..BR → image), with `UI_RECORDS_PER_NODE = 11` as the sub-record stride** *(amended 2026-08-21: S4 pins what it emits; the contract's last two terms are pinned by their own rungs, because at S4 neither exists — see the S4 audit ledger)*. | **Interaction**: the focus ring is the last quad of the contract, so a focused node's ring is never painted under its own glyphs — **but S4 does not gate that, because `FocusRing` has zero occurrences in `crates/` and I9 is the rung that emits it. Interaction inherits the obligation to extend the `append`-lane order assertion (G4-2's shape) when it lands the ring, and to raise `UI_RECORDS_PER_NODE` for it.** Likewise the glyph term: D4 itself records that glyph order "is decided purely by the order the host appends them", so it is a HOST APPEND DISCIPLINE, not a property of this lane. |

**And what this plan needs from them:**

| Needed | From | Blocks | Fallback if it is late |
|---|---|---|---|
| **D7's registration table** | `UI-PLAN-AETHER.md` | **S6 only** | S6 lands with fifteen hand-written landings; S0–S5 are unaffected |
| **The UI clock (D15)** | `UI-PLAN-ANIMATION.md` | nothing | S5 reads `Time`'s real delta through a one-function seam the animation plan later replaces |
| **`UiVisual`** in `ui_pack_inputs!` | `UI-PLAN-ANIMATION.md` | nothing | S0's macro is already shaped to take it; §10.8 leg (b) simply does not run until it exists |
| **The `paint_seq` agreement** | `UI-PLAN-INTERACTION.md` | G0-4 | G0-4 asserts the gather's pre-order against `collect_candidates` as it exists **today**; if the interaction plan changes that traversal, G0-4 is its gate too |

---

## 7 · Risks

### SR1 — the widening is a ten-site lockstep edit on a path with no live host and no binary gate

This is the architecture's **R1**, restated because it is this plan's risk and its whole ordering
exists to answer it. Today: no production host draws any UI; the GPU goldens **skip gracefully** on a
device-less host, so a green CI run may have exercised nothing; `ui_hud_screenshot.rs` is `#[ignore]`d
eight times; and the only pin on the committed `.spv` is a byte **length**, which cannot see a
re-compile drift at the same size.

*Mitigation, and it is the ladder itself:* **S0 before S1 before S2.** The observer exists before the
gate; the gate exists before the edit. **M2-b** is the mutation that says whether the mitigation
worked, and if M2-b does not red, S2 is not done. *Status 2026-08-21: S0's seam landed (two-phase,
observer + G0-2/G0-3/G0-5 green, measurement legs run), so the sequencing argument holds and* ***S2
is unblocked as written*** *(S1 first, per the ladder). The observer's half of this mitigation is
device-free; the visual half rides the completion criterion below.*

*Residual:* every gate here needs a GPU. On a device-less machine S2's gate is vacuous and reports
green. **The rung's completion criterion therefore includes "run on the RTX 3060 with the four hashes
reported", not "CI is green".**

### SR2 — S1 may not achieve `.spv` byte identity, and the fallback weakens the gate it was built for

If the generator's formatting shifts DXC's output, S1 lands as a re-bless (S-D10) — and a re-bless is
exactly the operation the gate exists to prevent being casual. The mitigation is that it happens
**once**, before any semantic change, with the four UI goldens compared by a human in the same commit;
after that the gate is live for every subsequent edit, which is the state S2 needs.

*What would make this worse and must not happen:* re-blessing during S3, when the shader is also
changing semantically. If S1's identity fails, the re-bless commit contains **nothing else**.

### SR3 — `NonUniformResourceIndex` is the first non-uniform thing in this shader

The design note on the text branch is explicit that it is "a uniform-per-instance branch, so the rect
majority is unregressed". A bindless sample is not. On some hardware a divergent descriptor index
becomes a waterfall loop **per quad**, on a pass that is otherwise trivially uniform.

*Mitigation:* §10.1 is a gate, not a note, and Model A stays reachable **without changing one
component** — which is the entire argument for starting with D2 rather than a claim that D2 wins.

*Counter-evidence, recorded because it is substantially right:* WebRender — the one team whose whole
job is drawing UI, and the only one that reasoned about this fork in public — chose atlases. Three of
its reasons transfer even though the driver-bug one does not: UI textures are small, few and
long-lived; the 4095 slots are shared with world materials; and the divergence is real. §10.1 is what
turns that from an argument into a number.

### SR4 — the slot budget is shared and nothing reserves anything

D3 refuses a UI reservation, so a UI that registers 500 icons individually steals 500 slots from the
scene. The correct answer costs nothing at runtime — pack the icon set offline with the existing
skyline packer (`boyko_fontbake/src/atlas.rs:138`) and spend one slot — but nothing enforces it.

*Mitigation:* a diagnostic counter of UI-held slots, reported by the observer rung, and S-D2's
const-assert so that the *response* to slot pressure (raising the capacity) cannot silently truncate
the field. Neither prevents exhaustion; both make it visible before it is a magenta screen.

### SR5 — the nine-slice expansion multiplies the instance count by up to 9

A UI whose panels are all nine-sliced pays 9× the ring traffic and 9× the sort. At the 2 048-node
figure §10.2 uses, that is 160 KB → 1.44 MB per frame.

*Mitigation:* G4-4 pins that the scratch does not reallocate, and M4-d records the tempting wrong fix.
The real answer if it ever bites is `fill_center = false` (8 quads) and authoring fewer sliced panels —
neither is a renderer change, which is why nothing is built for it now.

---

## 8 · Open questions for the owner (VALUES / SCOPE — also to be filed in `docs/OPEN-QUESTIONS.md`)

These are not perf or architecture forks; those are decided above with numbers or with reasons.

1. **`UiSamplerMode` at boot, or per-sprite from the start?** S-D4 ships one mode chosen at
   `ui_setup` and reserves bit 4 for the per-sprite extension. A UI that must mix pixel-art icons and
   photographic images in one pass needs the extension on day one, and that is a product call, not a
   perf one.
2. **How much demo above D32's floor.** The architecture's §13 Q5, unchanged: the minimal
   `boyko_app` rung is a v1 deliverable (S0), but whether a richer showcase scene belongs inside this
   campaign is scope.
3. **Is a checked-in sprite asset wanted at all?** S-D5 makes every *gate* procedural. The *demo*
   sprite could stay procedural too — which would mean the repo still contains no UI sprite, and the
   first real one arrives with a game.

---

## 9 · Sources

**In-tree, read for this plan** (worktree `D:/wt/ui`, branch `feat/ui-advanced`) —
`crates/boyko_render/src/ui/{instance,pack,upload,plan,draw,resources,mod}.rs` ·
`crates/boyko_render/src/gpu_column.rs` (`ui_setup`, `ui_upload`, `RhiContext`) ·
`crates/boyko_render/src/bindless.rs` · `crates/boyko_render/src/texture.rs` ·
`crates/boyko_render/shaders/ui_rect.{vs,fs}.hlsl` ·
`crates/boyko_render/tests/{ui_rect_gpu_golden,ui_pack_cpu,ui_no_realloc}.rs` ·
`crates/boyko_render/Cargo.toml` · `crates/boyko_ui/{Cargo.toml,src/components.rs}` ·
`crates/boyko_rhi_vulkan/src/bindless.rs` (the capacity **and** the shared sampler) ·
`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` (`create_graphics_pipeline_bindless`) ·
`crates/boyko_rhi_vulkan/src/present/{scene_types.rs (UiPass), passes/present_blit.rs, passes/gbuffer.rs}` ·
`crates/boyko_rhi/src/{encoder,device,enums}.rs` · `crates/boyko_app/{Cargo.toml,src/runner.rs}` ·
`crates/boyko_shaderdsl/src/{particle.rs,bin/emit_particles.rs}` (the leaf + generator idiom) ·
`crates/boyko_rhi_vulkan/tests/particle_edsl_sync.rs` (the sync-test plumbing) ·
`goldens/PINS.toml` · `docs/SHADER-VARIANT-MANIFEST.md`.

**Design authority:** `docs/UI-ADVANCED-ARCHITECTURE.md` rev 2 §3 (D1–D7, D31, D32), §4 (D8a–e),
§8 (D30), §9, §10, §11, §12.
**Evidence:** `docs/UI-ADVANCED-RESEARCH-SPRITES.md` (six-implementation survey; §7's live findings;
§10's argument against the recommendation), which carries the external citation list rather than
duplicating it here.
**Register shape:** `docs/PARTICLES-PLAN.md` (rung ladder, gate/red-mutation discipline, the F15
"a skipped `*_spv_sync` is not a pass" rule).
