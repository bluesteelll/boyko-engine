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
2. **`REPEAT` is what makes tiled nine-slice cheap (D8d), and it is also what makes it narrow.** A UV
   outside `[0,1]` wraps to the **whole texture**, not to a sheet frame — see S-D7.
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

### S-D7 — `Tile` nine-slice requires a whole-texture sprite; with a sheet it is a hard error

Tiled edges work by letting the UV run past `[0,1]` and letting `REPEAT` wrap. Under a sheet, `REPEAT`
wraps to the **whole sheet**, so a tiled edge would tile the neighbouring frames.

**Decision:** `UiNineSlice { mode: Tile }` on a node that also carries `UiSpriteSheet` is a
`debug_assert!` in dev and a **clamp to `Stretch`** in release, with the clamp counted in a diagnostic
counter so it is observable rather than silent.

**Reason.** The combination is not expressible under S-D4's sampler and cannot be made expressible
without per-sprite address modes. Failing loudly in dev and degrading visibly-but-safely in release is
the house pattern (`D15`-shaped release-present clamps in `PARTICLES-PLAN.md`).

**Rejected:** (a) emulate tiling with geometry (Unity's quad-per-tile explosion and its documented
16 250-quad cap — the exact thing D2 exists to avoid); (b) silently treat it as `Stretch` (a wrong
image with no trace).

### S-D8 — the default-OFF ladder, stated rung by rung

| Rung | What is new | Default | Which rung turns it on |
|---|---|---|---|
| S0 | gather, generation gate, host rung | gate ON (it can only *skip* work), host rung is a new binary nothing else runs | S0 itself, in its own binary |
| S1 | eDSL-generated shader | **byte-identical `.spv`** — nothing changes | never (it is a refactor) |
| S2 | 80 B record, `uv` field | every existing node packs `uv = (0,0,1,1)` and `flags` bits 3..31 zero | never (image pins must be identical) |
| S3 | `FLAG_TEXTURED`, set 1, UI sampler | `UiImage`'s default tint is **alpha 0** ⇒ an authored-but-untextured Image is invisible (`components.rs:454-466`) | S3's own new golden `ui_sprite_bindless` |
| S4 | `UiNineSlice` | absent ⇒ pack emits 1 record, exactly as S3 | S4's own golden `ui_nine_slice` |
| S5 | sheets + flipbook | `UiSpriteSheet` absent ⇒ `uv` comes from `UiImage`; `UiSpriteCursor` absent ⇒ no tick | S5's own golden `ui_flipbook` |
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

**Lockstep sites — all ten, in one commit:** the Rust struct; `UI_INSTANCE_SIZE`; the **ten**
`offset_of!` const-asserts; S-D2's `BINDLESS_TEXTURE_CAPACITY` const-assert; the `UiInstance` mirror in
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
| **G2-1** | Rust layout is exactly D1's | ten `offset_of!` asserts + `size_of == 80` + `align_of == 16` |
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

---

### S3 — the textured lane: set 1, the UI sampler, `UiImage` finally renders — **size L**

*Architecture D2, D3, D8e, plus S-D3 and S-D4.*

**Lands.**

1. **S-D3's `bind_descriptor_set_at`** (or nothing, if G3-0 says the cheap sampler route works and the
   set-1 bind is still needed — it always is, so this always lands).
2. **S-D4's UI sampler** — set 0 binding 3, `UiSamplerMode::{Smooth, Pixel}` chosen at `ui_setup`;
   plus `DescriptorKind::Sampler` + `BindGroupEntry::Sampler` **iff** G3-0 says so.
3. `ui_setup` gains `bindless: Option<&VulkanBindlessSet>` and `font: Option<&BakedFont>`; the
   pipeline is built through `create_graphics_pipeline_bindless(desc, set1_layout)` when the bindless
   set is supplied and through the existing one-set path when it is not — **so a host with no
   bindless table still boots and still draws rects and text**.
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
| **G3-4** | A bindless-less host still works | `ui_setup(bindless: None)` builds the one-set pipeline; the rect and text goldens pass. |
| **G3-5** | The slot field round-trips | CPU test: pack with slot ∈ {1, 4095}, read bits 20..31 back, assert equality; `debug_assert!(slot < BINDLESS_TEXTURE_CAPACITY)` on the pack path. |
| **G3-6** | Both recorders bind set 1 | The offscreen golden (`record_ui_rects`) **and** `ui_rect_swapchain_golden` extended with one textured quad. |

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

### S4 — nine-slice: CPU expansion + the D4 emission contract — **size M**

*Architecture D8d and D4, plus S-D7.*

**Lands.**

1. `UiNineSlice { border_px: [f32;4], mode: u8 /* Stretch|Tile */, fill_center: bool }` — 20 B,
   `#[repr(C)]` POD, padding spelled. Table (authored, cold).
2. The pack emits **9** sub-quads (8 with `fill_center == false`) into the **existing**
   `UiRenderScratch.pack` when the component is present, and **1** when it is absent. All nine inherit
   the parent's `StackIndex` and `ComputedClip` verbatim and take **consecutive** `append` indices —
   so the existing `(stack, append)` total-order sort keeps them contiguous and in painter's order
   **with no change to the sort**.
3. `Tile` mode folds the tile count into `uv` at pack (UVs run past 1.0 and `REPEAT` wraps), with
   S-D7's `debug_assert!` + release clamp when a sheet is also present, and a diagnostic counter for
   the clamp.
4. **D4's emission contract, pinned:** *background rect → nine-slice sub-quads (TL..BR) → image →
   glyphs → focus ring*, per node.
5. Layout is untouched — slicing is purely visual.

**Gate.**

| # | Claim | How |
|---|---|---|
| **G4-1** | The expansion is 9 (or 8), consecutive, inheriting | CPU unit test, no GPU: assert record count, that `append` is `k..k+9`, that all nine carry the parent's `StackIndex` and clip |
| **G4-2** | The emission order is D4's | A node with background + nine-slice + image + glyphs + focus ring: assert the `append` lane's order equals the contract, by name |
| **G4-3** | Slicing preserves corners | GPU golden `ui_nine_slice`: a 3×3 procedural source (S-D5) stretched to a 64×16 rect; the four corner regions are **unstretched** (their texel pattern matches the source's corners 1:1) and the edges are; image hash pinned |
| **G4-4** | The pack still never reallocates | `ui_no_realloc.rs` extended: the 9× expansion at N=1024 nodes must not grow the scratch after the first frame |
| **G4-5** | S-D7 is enforced | `Tile` + `UiSpriteSheet`: `debug_assert!` fires in dev; the release build clamps to `Stretch` and the counter increments |

**Red mutations.**

* **M4-a — give all nine sub-quads the same `append` index.** G4-1 reds, and the golden's paint order
  becomes order-of-iteration. *Proves the "the key is TOTAL because `append` is unique" claim is
  load-bearing rather than a description — an unstable sort over a non-total key is a real
  nondeterminism.*
* **M4-b — expand the corners proportionally instead of at fixed `border_px`.** G4-3 reds. *Proves the
  golden tests **slicing** rather than "a textured rect appeared" — the mutation that a texel-only
  assertion would survive.*
* **M4-c — swap the image and glyph emission order.** G4-2 reds. *Proves the contract is pinned. Note
  that this mutation is **invisible** to every image gate unless the glyph and the image overlap —
  which is why G4-2 asserts the order directly and not through a picture.*
* **M4-d — pre-`reserve` the 9× worst case at setup.** G4-4 stays green but the scratch's steady-state
  capacity grows 9× for a world with one nine-sliced node. *Not a red — recorded as the tempting
  wrong fix, because the scratch is a `Resource` and the growth is permanent.*

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

**Red mutations.**

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
| **S4** | D4's pinned emission order. | **Interaction**: the focus ring is the last quad of the contract, so a focused node's ring is never painted under its own glyphs. |

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
