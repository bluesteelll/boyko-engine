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
| **G4-2** | The emission order is D4's | ~~A node with background + nine-slice + image + glyphs + focus ring~~ ~~**A node with background + nine-slice + image**~~ **TWO nodes, because S-D12 (1) made the last two terms alternatives: one with background + image (subs 0, 10) and one with background + nine-slice + image (subs 0, 1..=9, and NO sub 10 — the gate asserts the image record's ABSENCE, which is the whole of ruling (1) and is ~~otherwise pinned nowhere~~ **also pinned by G4-8's derived total, and until S-D13 was mutated by NOTHING** *(corrected 2026-08-21 — S-D13 (1): "pinned nowhere" and "asserted twice, mutated never" are opposite diagnoses and the wrong one was written down. `1 + 2 + 1 + 10 = 14` becomes 15 with one extra record, so G4-8 sees it too; what was missing was a RED, which M4-c1 now is.)*)** *(amended 2026-08-21 — S-D12 (1))* *(amended 2026-08-21: glyphs are not per-node sub-records — `gather.rs:272` hard-codes `text_uv: None` and `pack.rs:208` forbids one record being both — and `FocusRing` has zero occurrences in `crates/`; it is Interaction's I9. As written this gate's subject was unconstructible)*: **drives `gather_into_staging`**; assert the ~~`append` lane's~~ **STAGED** order equals the contract, **by name**, off `sys.staged()` — the shape `ui_s0_seam.rs:288-302` already uses to assert staged records by `FLAG_TEXTURED` *(two corrections 2026-08-21 — S-D13 (5)(2) and (1). **(a)** "the `append` lane's order" is the noun S-D12 struck from G4-1 one row earlier as unobservable (`UiUploadSystem.keys` is private, `upload.rs:160`); the instrument named here is right, only the wording propagated the wrong claim — and it propagated it into §6 and thence into the Interaction plan. **(b) The cited shape is BLIND to the property this row is cited for.** On a nine-sliced imaged node index 0 is the untextured background and 1..=9 are nine textured slices, so a wrongly-emitted sub 10 is a TENTH TEXTURED record and a `FLAG_TEXTURED` prefix assertion is identical with and without it — only LENGTH or GEOMETRY separates them. That shape also hard-codes `assert_eq!(staged.len(), 4, …)` (`ui_s0_seam.rs:289-293`), the literal G4-1 forbids one row above. **G4-2 borrows the shape's stack-bracketing and its by-name reading, identifies each slice by its own `uv`/`min_px` as G4-1 does, and takes its LENGTH from the same derivation G4-1 and G4-8 use — never from that file's literal.**)* |
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
  canonical gather hard-codes `text_uv: None` (`gather.rs:272`), and `pack.rs:208`'s `debug_assert!`
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
| **S0** | **`ui_pack_inputs!`** — the single spelling of the pack-input set. Adding a visual component to it wires the discovery filter **and** the gather read list together, or fails to compile. **The list holds SIX after S4 added `UiNineSlice`** (S4 Lands item 6), so the derived census is `ui_pack_inputs!(count) + 1` = **7.00 probes/node/frame**. | **Animation** adds `UiVisual`. **Interaction** adds its scroll datum. Neither may add a component to the gather without adding it here — and neither may write the count down a second time: `ui_s0_discovery`'s length assertion and `ui_s0_measure`'s report both DERIVE it, because S3 already turned a hand-written `5 * 5` into a red with nothing wrong. |
| **S0** | **`gather_ui_nodes`** — the DFS over `UiRoot`/`Children` carrying the inherited clip on its stack. Its pre-order **is** paint order. | **Interaction**: this DFS and `collect_candidates` are the same traversal; D19a's traversal-folded scroll offset rides this stack. |
| **S0** | `UiRenderGeneration` + the per-slot gate, hoisted ahead of the gather. | **Animation**: an animating frame bumps the generation every frame and the gate cannot help it — §10.3 reports that number unchanged, so the animation plan inherits an honest baseline rather than a claim. |
| **S0** | The **two-phase seam** (`gather_into_staging` / `upload_staging`, G0-5-pinned signatures) + the host drive protocol (`stage_frame` → `run_system_once` → `take_frame_output`). The `boyko_app` UI rung (D32's floor) is **deferred** to the Renderer-as-ECS-resource rung (2026-08-21 ruling) — the observer S0 ships is Phase 1, device-free. | **All three.** The device-free observer makes the seam's behaviour falsifiable on any machine; the human-visible floor arrives with the windowed rung, and until then **R1**/**R2**'s visual half rests on the owner-run windowed leg. |
| **S2** | `UiInstance` at 80 B with `uv` and S-D2's bit map. **Bits 5..19 are free; bit 4 is reserved.** | **Animation**: D5 folds the visual transform at pack and costs **zero** GPU bytes, so animation needs none of these bits. If it ever does, it takes bits 5..19 and says so here. |
| **S3** | `FLAG_TEXTURED`, the bindless slot lane, the UI sampler binding, font-optional boot. | **Aether**: the `ui` construct's `image` / `sheet` vocabulary can only name what S3–S5 built. |
| **S5** | The `u16 sheet_id` dense-handle mint. | **Aether**: the sheet-id mint is the natural thing for the construct to own at expand time (research §11 item 6). |
| **S4** | D4's pinned emission order — ~~**over the three terms S4 emits (background → nine-slice TL..BR → image)**~~ **over the terms S4 emits: background → EITHER the nine-slice sub-quads (row-major TL..BR) OR the image**, with `UI_RECORDS_PER_NODE = 11` as the sub-record **stride** (the maximum emission is 10) *(amended 2026-08-21: S4 pins what it emits; the contract's last two terms are pinned by their own rungs, because at S4 neither exists — see the S4 audit ledger. **Amended again the same day — S-D12 (1): the nine-slice and image terms are ALTERNATIVES, since `UiNineSlice` suppresses the image record it slices.** A sibling adding a term takes a free sub code and raises the stride; it does not renumber these.)*. | **Interaction**: the focus ring is the last quad of the contract, so a focused node's ring is never painted under its own glyphs — **but S4 does not gate that, because `FocusRing` has zero occurrences in `crates/` and I9 is the rung that emits it. Interaction inherits the obligation to extend the ~~`append`-lane~~ **STAGED** order assertion (G4-2's shape) when it lands the ring, and to raise `UI_RECORDS_PER_NODE` for it — ~~raise~~ **it takes the next free sub code and `UI_RECORDS_PER_NODE` follows, because S-D13 (3) derives the stride as `UI_IMAGE_SUB + 1` rather than authoring it**.** *(both corrected 2026-08-21 — S-D13 (5)(2) and (3). "`append` lane" is the noun S-D12 struck from G4-1 as unobservable — `UiUploadSystem.keys` is private (`upload.rs:160`) — and this sentence was propagating it into the sibling plan, which is where a struck claim does the most damage: Interaction would have written a gate against a lane it cannot read.)* Likewise the glyph term: D4 itself records that glyph order "is decided purely by the order the host appends them", so it is a HOST APPEND DISCIPLINE, not a property of this lane. |

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

### SR5 — the nine-slice expansion multiplies the instance count by up to ~~9~~ **10**

A UI whose panels are all nine-sliced pays ~~9×~~ **10×** the ring traffic and the sort. At the
2 048-node figure §10.2 uses, that is 160 KB → ~~1.44 MB~~ **1.6 MB** per frame.

*(corrected 2026-08-21 — S-D12 (1). A nine-sliced imaged node emits **10** records (background + nine
regions) where a plain rect emits 1, so the multiplier against the §10.2 baseline is 10, not 9. It is
**5×** rather than 10× against an already-imaged node, whose S3 cost is 2 records. The ruling did not
raise this risk — it made it countable: under the ADD arithmetic the same node emitted 11 records, one
of which was invisible under the other ten.)*

*Mitigation:* G4-4 pins that the scratch does not reallocate, and M4-d records the tempting wrong fix.
The real answer if it ever bites is `fill_center = false` (**9 records**) and authoring fewer sliced
panels — neither is a renderer change, which is why nothing is built for it now.

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
