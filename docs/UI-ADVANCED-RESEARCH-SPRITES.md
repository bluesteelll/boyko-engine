# UI sprites — research corpus (pre-design)

**Date:** 2026-08-21 · **Branch:** `feat/ui-advanced` · **Status:** research complete; feeds the UI-advanced architecture rung
**Scope:** sprites and 2D batching for `boyko_ui` — atlas packing, sheets vs individual textures, bindless vs atlas, nine-slice, how batching survives z-order and clipping, and how animated sheets are represented.
**Method:** six-implementation survey read from primary sources (Bevy `bevy_sprite_render` / `bevy_ui_render`, Unity UGUI + UI Toolkit, Godot 4 `RendererCanvasRenderRD`, Dear ImGui draw lists, RmlUi render interface, Mozilla WebRender) cross-checked against the in-tree render path, which was read in full first. Secondary sources are flagged. Every recommendation is grounded against Principle 0, the no-`dyn`/no-`Box`/no-`HashMap` hot-path ban, capability-by-component-presence, and eDSL-authored shaders.

---

## TL;DR — what the design must know

1. **This engine already has the thing every surveyed engine works around.** `boyko_render`'s UI path is **one pipeline, one `draw(6, N, 0, 0)`, one global z-sort**, with **clipping carried per-instance in the fragment shader** (`FLAG_CLIP_PRESENT`), not by scissor. Every other implementation surveyed breaks its draw on a texture change, and ImGui/RmlUi break it on a *clip* change too. Preserving "one draw, always" is the single most valuable property in the building, and sprites must not be the feature that spends it.
2. **The bindless table already exists, and `UiImage.texture` was already specified as a dense `u32` slot into it.** `boyko_rhi_vulkan::bindless` ships a 4096-entry `SAMPLED_IMAGE` array with `PARTIALLY_BOUND | UPDATE_AFTER_BIND`, a shared immutable sampler, a magenta error texture in every slot, and a fence-gated slot recycle; `TextureGpu` already carries `bindless_slot: u32`. So *atlas-vs-individual-texture stops being a renderer fork here and becomes an asset-packing choice.* That is not true in any of the six references.
3. **The 64 B `UiInstance` has a recorded expiry, and sprites are the trigger.** `corner_radius` is aliased as the glyph UV under `FLAG_TEXT`, and the source names the exit condition verbatim: a textured/nine-slice rect needing **both** a radius and a UV "retires this alias and widens `UiInstance` to 80 B — the recorded deliberate-revisit trigger" (`crates/boyko_render/src/ui/instance.rs:55`). Take the trigger; do not invent a fourth meaning for the same 16 bytes.
4. **Nine-slice: expand on the CPU into the pack scratch; do NOT widen every instance and do NOT add a second pipeline.** Bevy pays a whole extra pipeline + shader + 16 extra floats per vertex for slicing; Unity expands to ≤9 quads on the CPU. Here the pack scratch is *already* the sanctioned frame-transient buffer, all 9 sub-quads inherit the parent's `StackIndex` and consecutive append order (so the existing total-order sort keeps painter's order for free), and the hot 95% of plain rects pay **zero** bytes.
5. **Animation state is gameplay-observable and belongs in the ECS, not in a shader clock.** Godot exposes `frame` and an `animation_finished` signal; Unreal's flipbook exposes the keyframe; Aseprite's canon includes ping-pong and *per-frame* durations that have no closed form. A shader-side `frame = f(time)` is a datum no system can query — the parallel-data-system failure inverted. Split the per-element datum into a **cold track** (first/last/fps/mode) and a **hot cursor** (elapsed/frame), exactly as `components.rs:5` already prescribes for churn-split columns.
6. **Uniform-grid sheets need zero per-frame storage.** Bevy stores `Vec<URect>` per layout plus a `HashMap<AssetId, usize>` for reverse lookup; Godot stores a frame list per animation. A uniform grid makes the frame UV pure arithmetic from `(cols, rows, index)`. Only ragged/trimmed sheets need a table, and that table is one `Resource`-owned dense column keyed by `u16 sheet_id` — never a per-element owned array.
7. **⚠️ Three in-tree facts that invalidate assumptions a sprite design would otherwise make.** The documented "O(1) generation gate / 0%-when-static" **does not exist as a live mechanism**; the UI render path has **no non-test caller**; and `ui_setup` cannot boot without a font. Details in *§7 — live findings*.

---

## 1 · In-tree inventory (read, not assumed)

| Datum | Where | Shape |
|---|---|---|
| `ComputedRect` | `crates/boyko_ui/src/components.rs:167` | `#[repr(C)]` 16 B, "the ONLY geometry the renderer reads" |
| `ComputedClip` | `components.rs:192` | 16 B, author-owned, consumed as a per-instance AABB (not scissor) |
| `StackIndex` | `components.rs:184` | `#[repr(transparent)] u32`, author-owned painter's key |
| `UiBackground` | `components.rs:219` | 44 B: fill + border colors (straight RGBA8), per-corner radius, per-side border |
| `UiImage` | `components.rs:440` | 24 B: `texture: u32` ("dense handle into the (future) UI texture table"), `uv_min`, `uv_max`, `tint` — **authorable in `ui!` and `.ui`, but the pack never reads it: an Image node renders nothing today** |
| `UiInstance` (GPU) | `crates/boyko_render/src/ui/instance.rs:35` | `#[repr(C, align(16))]` **64 B**, std430, per-field `offset_of!` oracle |
| Descriptor set 0 | `crates/boyko_render/src/ui/resources.rs:131` | b0 = `StorageBuffer` ring (VERTEX\|FRAGMENT), b1 = `CombinedImageSampler` MSDF atlas (FRAGMENT), b2 = per-atlas `UniformBuffer` |
| Push block | `resources.rs:51` | `UiOrtho`, 16 B, VERTEX\|FRAGMENT |
| Pack / sort | `crates/boyko_render/src/ui/pack.rs` | `clear()+extend` into a reused `Resource` scratch; `sort_unstable_by_key((stack, append))` — a **total** key, so the unstable sort *is* the stable permutation with no merge buffer |
| Draw | `crates/boyko_render/src/ui/draw.rs` | exactly one `draw(6, N, 0, 0)` into an already-open `LoadOp::Load` scope |
| Bindless table | `crates/boyko_rhi_vulkan/src/bindless.rs:72` | `BINDLESS_TEXTURE_CAPACITY = 4096`, `PARTIALLY_BOUND \| UPDATE_AFTER_BIND`, immutable shared sampler at binding 1, slot 0 reserved (magenta error) |
| Bindless slot allocator | `crates/boyko_render/src/bindless.rs` | free-list `Vec<u32>` + **fence-gated** recycle; explicitly sanctioned as allocator-internal bookkeeping, not a side store |
| `TextureGpu` | `crates/boyko_render/src/texture.rs:535` | owns the image, **already carries `bindless_slot: u32`**, `mip_levels`; PNG decode via in-house `PngTextureLoader` |
| Skyline packer | `crates/boyko_fontbake/src/atlas.rs:138` | offline glyph packing, `ATLAS_PADDING_TEXELS` gutter, `MAX_ATLAS_DIM = 8192` |

**The property to protect.** Because the clip travels *in the instance record* rather than in pipeline state, boyko's UI has no clip-driven draw break. ImGui splits a draw command on any `ClipRect` **or** `TextureId` change (`ImDrawCmd`'s `ClipRect`/`TexRef`/`VtxOffset` are documented as deliberately contiguous "as they are compared together"); RmlUi's interface is literally `EnableScissorRegion` / `SetScissorRegion` around `RenderGeometry(geometry, translation, texture)` — one call per compiled geometry, per texture. Boyko is a generation ahead here and the sprite design must not regress it.

---

## 2 · The data model each implementation uses per sprite element, and where it lives

### Bevy — `bevy_sprite` / `bevy_ui_render`

*Per element (main world, ECS):*

```rust
pub struct Sprite {
    pub image: Handle<Image>,
    pub texture_atlas: Option<TextureAtlas>,
    pub color: Color,
    pub flip_x: bool, pub flip_y: bool,
    pub custom_size: Option<Vec2>,
    pub rect: Option<Rect>,
    pub image_mode: SpriteImageMode,   // Auto | Scale(..) | Sliced(TextureSlicer) | Tiled{..}
}
pub struct TextureAtlas { pub layout: Handle<TextureAtlasLayout>, pub index: usize }
pub struct TextureAtlasLayout { pub size: UVec2, pub textures: Vec<URect> }
pub struct TextureAtlasSources { pub texture_ids: HashMap<AssetId<Image>, usize> }
```

*Per element (render world, rebuilt every frame):*

```rust
pub struct ExtractedSprite { main_entity, render_entity, transform, color,
                             image_handle_id: AssetId<Image>, flip_x, flip_y,
                             kind: ExtractedSpriteKind }
pub enum ExtractedSpriteKind { Single { anchor, rect, scaling_mode, custom_size },
                               Slices { indices: Range<usize> } }   // into a side Vec<ExtractedSlice>
```

*GPU:* `SpriteInstance { i_model_transpose: [Vec4;3], i_color: [f32;4], i_uv_offset_scale: [f32;4] }` — **80 B**, instanced.
*Batching:* sort key is `FloatOrd(translation.z)`; a batch breaks on `image_handle_id` change. `SpriteBatch { image_handle_id, range }`.

Bevy **UI** is a different renderer again: a non-instanced vertex stream, `UiVertex { position:[f32;3], uv:[f32;2], color:[f32;4], flags:u32, radius:[[f32;4];2], border:[f32;4], size:[f32;2], point:[f32;2] }` — that is **~104 B per vertex × 4 = ~416 B per quad**, against boyko's 64 B *per quad*. `UiBatch { range, image }` breaks on image id (with the white default folding into any batch). `z_order` is the stack index plus fixed sub-offsets (`BACKGROUND_COLOR = 0.0`, `IMAGE = 0.04`, `TEXT = 0.06`). Nine-slice is a **separate pipeline** (`ui_texture_slice_pipeline.rs` + `ui_texture_slice.wesl`) whose instance carries four normalized `[f32;4]`s — `slices`, `border`, `repeat`, `atlas` — and does the slicing in the fragment shader on one quad.

### Unity — UGUI and UI Toolkit

*UGUI, per element:* a `MonoBehaviour` `Image` holding `m_Sprite`, `m_Type` (`Simple|Sliced|Tiled|Filled`), `m_FillCenter`, `pixelsPerUnit`; geometry is regenerated into a `CanvasRenderer` mesh. **Nine-slice is CPU quad expansion**: `for (x = 0..3) for (y = 0..3) { if (!m_FillCenter && x==1 && y==1) continue; ... }` — up to 9 quads / 36 vertices. Tiled expands to one quad per tile, capped at 16 250 quads, and the docs recommend `TextureWrapMode.Repeat` with an unpacked sprite specifically "to prevent the generation of additional geometry" — i.e. *the atlas is what forces the geometry explosion*.
*Batch key:* material + texture, per canvas, further constrained by sibling overlap analysis; Sprite Atlas exists to collapse the texture axis of that key.
*UI Toolkit* (the newer stack) went the other way: one pre-allocated vertex buffer per panel acting as a heap allocator, plus a runtime **dynamic atlas**; overflowing the buffer "fragments batching, increases draw calls".

### Godot 4 — `RendererCanvasRenderRD`

*GPU record, per instance,* `renderer_canvas_render_rd.h`:

```cpp
struct InstanceData {
    float world[6];
    float ninepatch_pixel_size[2];
    union {                              // rect variant vs primitive variant
      struct { float modulation[4]; float ninepatch_margins[4];
               float dst_rect[4]; float src_rect[4]; float pad[2]; };
      struct { float points[6]; float uvs[6]; uint32_t colors[6]; };
    };
    uint32_t flags; uint32_t instance_uniforms_ofs; uint32_t lights[4];
};                                        // static-asserted 128 B
```

*Batch key:* `Batch { start, instance_count, tex_info, modulate, msdf_pix_range, clip, material, command_type, shader_variant, use_lighting, use_msdf, use_lcd, has_blend, ... }`, where `TextureState { RID texture; uint32_t other /* filter|repeat|is_data|linear_colors */ }`.
Two facts matter for us: Godot **carries nine-patch margins in the instance record** (`ninepatch_margins` + `ninepatch_pixel_size`, 40 % of the rect variant's payload), and it **unions** the rect and primitive variants rather than aliasing one field with two meanings — the honest version of what boyko currently does with `corner_radius`.

### Dear ImGui — draw lists

`ImDrawVert { pos, uv, col }` = **20 B**; `ImDrawCmd { ClipRect, TexRef, VtxOffset, IdxOffset, ElemCount, UserCallback, ... }` where "1 command = 1 GPU draw call". A new command is emitted on a clip-rect change, a texture change, or a callback; `ImDrawListSplitter` exists purely to fake z-order by drawing into channels and flattening. This is a **rebuild-everything-every-frame immediate-mode** model: there is no per-element durable datum at all. It is the wrong shape for a retained ECS UI and is included as the contrast case.

### RmlUi

`CompileGeometry(Span<const Vertex>, Span<const int>) -> CompiledGeometryHandle` retained across frames; then per frame `RenderGeometry(geometry, translation, texture)` — **texture is a per-draw argument**, so one draw per textured element unless the backend batches behind the interface. Clipping is `EnableScissorRegion` / `SetScissorRegion`, i.e. pipeline state, with a clip-mask escape hatch for transformed clips.

### WebRender

The most directly comparable design, and the one that reasoned about our exact fork out loud. Instance data is "a simple ivec4 which packs a bunch of 32 bits and 16 bits offsets and flags (primitive ID, transform ID, source pattern ID, source mask ID, etc.)", with rectangles in world space plus "another rectangle in texture space, containing the texture coordinates of the source image in a texture atlas". Batching walks backwards through open batches and stops at the first **overlapping** primitive, so paint order is never violated. Clipping is tiered: axis-aligned clips move vertices in the vertex shader; non-axis-aligned clips are computed per-primitive into alpha; fully general clips become mask textures in an atlas. Glyph and image atlases were deliberately **separated** because they use different shaders and "can never be used in the same rendering batches" — separating them reduced the number of textures per class and therefore batch breaks.

---

## 3 · Atlas packing — and why

Jylänki's survey (*A Thousand Ways to Pack the Bin*) is still the reference: **MAXRECTS packs best overall; SKYLINE wins for online packing into a single bin and is faster; GUILLOTINE is asymptotically faster and slightly worse; SHELF only if implementation simplicity dominates.** The in-tree glyph baker already uses skyline (`atlas.rs:138`) — the right choice for its offline single-bin job.

Three separate motivations get conflated under "atlas", and they must be separated before choosing:

| Motivation | Does it survive bindless? | Note |
|---|---|---|
| Collapse the draw-call-per-texture axis | **No — bindless removes the need entirely** | This is 90 % of why UGUI/Bevy/Godot atlas |
| Reduce descriptor/slot pressure | Yes, still real | 4095 slots shared with world textures |
| Improve sampler cache locality for many tiny quads | Yes, still real | Glyph-class workloads |
| Reduce file/IO count and per-image allocation overhead | Yes, still real | An **asset-pipeline** concern |

And three costs that never go away:

- **Mipmaps bleed across tile boundaries** unless every tile is padded and the mip chain is built per-tile; the in-tree packer already carries `ATLAS_PADDING_TEXELS` for exactly the bilinear case.
- **No wrap/`REPEAT`.** This is the one that bites UI hardest: tiled nine-slice edges and tiled backgrounds are exactly the case atlases cannot express, which is why Unity's own docs tell you to *leave the sprite out of the atlas* to get `TextureWrapMode.Repeat`.
- **A runtime atlas needs an allocator with eviction** (WebRender's whole `guillotiere`/`etagere` line of work exists because of this).

**Conclusion for this engine:** atlasing should be an **offline/asset-time** transform, reusing the existing skyline packer, chosen per-asset-set for slot pressure and locality — not a renderer requirement and not a runtime allocator. A runtime atlas allocator would be a new side data structure with an eviction map; the only sanctioned precedent for that shape is `BindlessSlotAllocator`'s bounded free-list, explicitly documented as allocator-internal.

---

## 4 · Sprite sheets vs individual textures, bindless vs atlas

The strongest primary evidence on the fork is WebRender's, and it is *against* bindless — for WebRender's constraints:

> "it's much easier to draw a UI in a single draw call when you have access to bindless textures, but the worst part is driver bugs"

…with Android/OpenGL cited, where "even using texture arrays ended up too fancy". That is a portability argument, and **boyko does not have WebRender's portability constraint**: `create_bindless_texture_set` asserts `device_caps().bindless_capable` and the engine **boot-fail-fasts** on it (`bindless.rs:202`). The cost was paid once, for the raster path; the UI would amortize it.

The comparison, held to this engine's constraints:

| | **Atlas** (one descriptor) | **Bindless** (one slot per texture) | **Batch-by-texture** (the mainstream) |
|---|---|---|---|
| Draw calls | 1 | 1 | O(distinct textures interleaved in z) |
| Fragment cost | 1 static descriptor | `NonUniformResourceIndex` load per quad | 1 static descriptor per batch |
| `REPEAT` / tiling | ✗ (emulate with geometry) | ✓ (real sampler) | ✓ |
| Mips | needs per-tile padding + care | ✓ native, `TextureGpu.mip_levels` already there | ✓ |
| Max sprite size | bounded by `MAX_ATLAS_DIM` | unbounded | unbounded |
| New mechanisms needed | runtime allocator + eviction + padding | **zero** (table, allocator, recycle all exist) | batch list + per-batch rebind + `UiFramePlan` becomes a range list |
| Slot budget | n/a | 4095, shared with world textures | n/a |
| Principle-0 risk | a runtime allocator + eviction map is a new side store | `UiImage.texture` = `TextureGpu.bindless_slot`; **no new store at all** | a per-frame batch `Vec` + a texture→batch map |

**Sheets vs individual textures becomes orthogonal under bindless.** A sheet is then just "one bindless slot whose UV sub-rect is chosen per element" — which is precisely what `UiImage`'s existing `uv_min`/`uv_max` already express. That is the same shape as Bevy's `TextureAtlas { layout, index }` minus the `Handle` and the `Vec<URect>` indirection, and minus the `HashMap<AssetId, usize>` reverse map, which is a banned type here.

---

## 5 · Nine-slice / scale-9

Three shipped strategies, and they differ on where the 9 rectangles are computed:

| | Where | Cost to non-sliced elements | One draw? |
|---|---|---|---|
| **Unity UGUI** | CPU, ≤9 quads / 36 verts into the canvas mesh | none | yes (same material) |
| **Bevy UI** | GPU, 1 quad, dedicated pipeline + shader, instance carries `slices/border/repeat/atlas` (16 floats) | none (separate pipeline) | **no — a pipeline switch** |
| **Godot** | GPU, 1 quad, `ninepatch_margins[4]` + `ninepatch_pixel_size[2]` in **every** `InstanceData` | 24 B on every instance | yes |

For boyko the Bevy route is disqualified outright: a second pipeline breaks the one-draw property that is the crate's chief asset. The Godot route taxes the 95 % of nodes that are plain rects. **The Unity route is the fit**, and it is *cheaper* here than it is in Unity, because:

- the pack scratch (`UiRenderScratch.pack`, a `Resource`) is already the frame-transient buffer, so the 9 sub-quads need **no new storage**;
- all 9 inherit the parent's `StackIndex` and get consecutive `append` indices, so the existing `(stack, append)` **total-order** sort keeps them contiguous and in painter's order with no change to the sort;
- they inherit the parent's `ComputedClip` verbatim, so per-instance clipping composes for free;
- corner-radius and border stay meaningful on the sub-quads (they are ordinary rects), where a single-quad shader route would have to reconcile radius, border and slice math in one branch.

The capability is a component: `UiNineSlice { border_px: [f32;4], ... }` present ⇒ the pack emits 9 records; absent ⇒ it emits 1. Structural absence, not a flag — and the layout pass is untouched, since slicing is purely visual.

**Tiled** (as opposed to stretched) sides/center is where bindless pays again: with a real per-texture sampler, `REPEAT` handles it in one quad; with an atlas, it is Unity's quad-per-tile explosion and its 16 250-quad cap.

---

## 6 · How batching survives z-order and clipping

Every reference makes the same admission in different words: **alpha-blended UI must be painted in order, so any per-draw state that changes mid-order costs a draw call.** Bevy sorts sprites by `FloatOrd(z)` and starts a new batch on an image change; Bevy UI does the same on `UiBatch.image`; Unity's canvas batching is constrained by overlap analysis; WebRender explicitly walks back through open batches and stops at the first overlapping primitive; ImGui splits on clip **or** texture and offers `ImDrawListSplitter` channels as a manual z-order escape.

Boyko's answer is structurally different and better: **move the per-draw state into the per-instance record.** The clip already lives there (`FLAG_CLIP_PRESENT` + a physical-px AABB with an AA band, so a clipped edge is as crisp as a rect edge). Sprites should follow the same doctrine — the *texture identity* moves into the record as a bindless slot — and then z-order costs nothing, because there is only ever one draw to order within.

Two consequences the architecture rung must accept:

- **No opaque/blended split, no depth buffer.** WebRender renders opaque primitives front-to-back with z-write, then blended ones. Boyko's UI pass is a single `LoadOp::Load` blended pass with a full-extent viewport; a depth-based opaque pass would be a second pass and a second sort. Out of scope; note it as the one lever left on the table if fill rate ever dominates.
- **Overdraw is unmanaged.** With one sorted draw and no occlusion, a full-screen sprite behind a full-screen panel is paid twice. Every reference has this problem too (Unity's UI overdraw is a standing field complaint); none of them solves it in the batcher.

---

## 7 · Live findings from reading the in-tree path (⚠️ these invalidate assumptions)

1. **The "O(1) generation gate / 0 % when static" does not exist.** `UiRenderGeneration::bump` is called from **no production site** (only `crates/boyko_render/tests/ui_pack_cpu.rs`), and `UiRenderScratch.last_seen_generation` is **never read anywhere**. The guarantee is asserted in three doc blocks (`ui/mod.rs:13`, `ui/pack.rs:154,193`, `ui/upload.rs:6`) and implemented nowhere — `pack_sort_upload` repacks every node unconditionally. A sprite-animation design must not lean on it, and should not be blamed for the repack cost it does not cause.
2. **The specified gate could not be correct even if wired.** With `FRAMES_IN_FLIGHT = 2` and one ring slot per frame-in-flight, a change must reach **both** slots before a skip is legal. A single scalar `last_seen_generation` cannot express that; the gate has to be per-slot (`[u64; FRAMES_IN_FLIGHT]`) or the second slot serves a stale frame. This is a defect in the *specification*, not only in the wiring.
3. **No non-test caller.** `host_upload_frame` / `host_upload_frame_from_world` / `ui_pass` / `ui_setup` are referenced only from `boyko_render`'s own tests. `boyko_app` does not drive the UI pass. Sprites therefore have unusual freedom to change `UiInstance`'s layout — and unusual exposure, because only goldens will notice a regression.
4. **`ui_setup` requires a `&BakedFont` unconditionally** (`gpu_column.rs:269`). A sprite-only UI cannot boot without a font asset. If sprites become a first-class construct, the atlas/texture bindings need to become optional or default-filled (the bindless table already has the magenta error texture in every slot — the same trick applies).
5. **`UiImage` is authorable but inert.** It parses in `.ui` (`text/dispatch.rs:157,437`), it has a bundle (`ImageBundle`), and the pack never reads it — by design, with the transparent-tint default chosen so it "never flashes a white box when P5a lands". The default is right; the gap is exactly this campaign's subject.

---

## 8 · Three candidate models for boyko, compared

### Model A — runtime UI atlas, one descriptor
One growable UI atlas texture; sprites are packed into it at load/runtime; `UiImage` carries a normalized UV rect; the shader keeps its single `CombinedImageSampler`.
**For:** the smallest shader change; one descriptor; best sampler locality for many small icons; the same shape as the shipped MSDF text lane, so text and sprites could share a branch.
**Against:** needs a runtime allocator with eviction (a new mechanism, and a side-store hazard); no `REPEAT`, so tiled nine-slice becomes geometry; mip bleed needs per-tile padding and a per-tile mip policy; a large sprite (full-screen background, a video frame) does not fit; and the atlas becomes a resize/repack stall.

### Model B — bindless slot per sprite, still one draw (**recommended**)
`UiImage.texture` **is** `TextureGpu.bindless_slot`. The UI pipeline gains the existing bindless set at set 1 — the same layout object the textured raster path and `vb_shade_tex.comp` already share — and the fragment shader samples `g_textures[NonUniformResourceIndex(slot)]`. Atlasing becomes an optional asset-time packing, not a renderer concern.
**For:** zero new mechanisms — the table, the free-list allocator, the fence-gated recycle, the error texture, the boot gate, and even the pipeline verb (`VulkanContext::create_graphics_pipeline_bindless`, already used by the g-buffer pass) all exist; one draw preserved with no batch key at all; real mips, real `REPEAT`, unbounded sprite size; the per-element datum is a dense `u32`, not a handle; `UiImage`'s doc comment already specified this ("dense `u32` handle into the (future) UI texture table") — and the table already exists, so building a *second* UI-only texture table would itself be the Principle-0 violation.
**Against:** see §10.

### Model C — batch by texture (the mainstream shape)
Sort by `(z, texture)`, break the draw on a texture change, emit a batch list.
**For:** what every surveyed engine does; no device feature required; unbounded texture count.
**Against:** it spends the crate's best property. `UiFramePlan` (a POD carrying one `instance_count`) becomes a variable-length batch list; the recorder gains a per-batch rebind; and the draw count becomes content-dependent and unpredictable, which is the cliff Bevy documents for sprites at different z with different images. Recommended **only** as the fallback if `bindless_capable` ever stops being a boot requirement.

---

## 9 · Recommended shape (ECS-native), in detail

**Record.** Take the recorded revisit trigger and widen `UiInstance` 64 B → **80 B**, retiring the `corner_radius`/UV alias so a textured node can carry both:

- `@64 uv: [f32; 4]` — normalized sub-rect `(u0, v0, u1, v1)`, used by glyphs **and** sprites (the text lane un-aliases too, so one field has one meaning again);
- the bindless slot rides `flags`' high bits (4096 slots = 12 bits; `flags` uses 3 today), keeping the record at exactly 80 B — a multiple of 16, no tail pad, per-field `offset_of!` oracle extended in the same commit;
- `color` becomes the premultiplied **tint** for a textured node (the Bevy/Godot convention: `Sprite.color`, `modulation`).

Godot's union is the alternative to bit-stuffing and is worth the architect's consideration; what must **not** survive is a third meaning for `corner_radius`.

**Components (capability = presence).**

- `UiImage` (exists, 24 B) — `texture` reinterpreted as the bindless slot; presence ⇒ textured lane.
- `UiSpriteSheet { sheet: u16, index: u16 }` — 4 B; presence ⇒ the pack derives `uv` from the sheet table instead of reading `uv_min`/`uv_max`. Uniform grid ⇒ pure arithmetic, no per-frame table.
- `UiNineSlice { border_px: [f32;4], mode: u8 (Stretch|Tile), .. }` — presence ⇒ the pack emits 9 records instead of 1.
- `UiSpriteAnim { first: u16, last: u16, fps: f32, mode: u8 (Forward|Reverse|PingPong|Once), repeats: u8 }` — the **cold track**, written by the author, never by a system.
- `UiSpriteCursor { elapsed: f32, frame: u16, dir: i8 }` — the **hot cursor**, the only column the animation system writes each frame; the churn split is the one `components.rs:5` already prescribes, and it keeps `Changed<UiSpriteAnim>` meaningful as "the author retargeted the animation".

**Sheet table.** One `Resource`-owned dense column keyed by `u16 sheet_id`: `{ slot: u32, cols: u16, rows: u16, frame_count: u16, pad_uv: [f32;2] }` for uniform grids, plus an optional ragged sub-rect column for trimmed/packed sheets. Never a `HashMap<name, sheet>` on the hot path; the name→id mint is a load-time concern with the existing `TypeIntern`/`FontId` dense-handle precedent.

**Animation system.** One system over `(UiSpriteAnim, UiSpriteCursor, UiSpriteSheet)` advancing `elapsed`, resolving ping-pong by flipping `dir` at the ends, and writing `index`. Aseprite's canon — `forward | reverse | pingpong | pingpong_reverse`, `repeat`, and **per-frame durations in ms** — is the superset to design the enum against; Godot's `SpriteFrames` (fps `speed` + per-frame *relative* duration + `loop`) and Unreal's `UPaperFlipbook` (`FramesPerSecond` + `KeyFrames[{ sprite, frame_run }]`, a run-length encoding) are the two shipped compressions of it. **Recommendation: uniform fps + optional run-length `frame_run` column**, which covers Aseprite's variable durations at integer resolution with no per-frame float table.

**Shader.** The fragment leaf must be authored through `boyko_shaderdsl` and spliced between sentinels. Note that `ui_rect.fs.hlsl` / `ui_rect.vs.hlsl` are currently **hand-written** and eDSL-free (no `// === GENERATED` sentinels, offline `dxc` recipe in the header, byte-gated `.spv`) — so "extend the eDSL" here means *bringing the UI leaves into the eDSL for the first time*, not editing generated output. That is a real scope item, not a formality, and the `f32` host oracle earns its keep on the nine-slice and frame-UV arithmetic.

---

## 10 · The strongest argument AGAINST the recommendation

**UI is the exact workload where bindless wins least and costs most, and the one team that reasoned about this in public chose the atlas.** WebRender's author states it plainly — bindless makes the single-draw-call UI easy, "but the worst part is driver bugs" — and WebRender shipped atlases instead, on a codebase whose entire job is drawing UI. The reasons transfer further than the Android caveat does:

1. **UI textures are small, few, and long-lived** — the profile atlases are best at, and the profile where a per-quad divergent descriptor load buys nothing. A `NonUniformResourceIndex` index defeats descriptor hoisting and, on some hardware, becomes a waterfall loop *per quad*, on a pass that is otherwise a trivially uniform branch. The shader's current design note is explicit that the text branch is "a uniform-per-instance branch, so the rect majority is unregressed" — a bindless sample is the first thing in this shader that is *not* uniform.
2. **The 4095-slot budget is shared with the world's material textures.** A UI that registers 500 icons individually steals 500 slots from the scene, and there is no per-consumer reservation today. An atlas costs exactly one slot no matter how many icons.
3. **The widening taxes the 95 %.** Every plain rect grows 64 B → 80 B: at 2 048 nodes, 128 KB → 160 KB of ring traffic per frame, touched twice by the sort gather, for a field most instances do not use. Godot pays 128 B and Bevy UI pays ~416 B per quad, so we would still be the leanest — but "still leanest" is not "free", and the atlas route could have kept 64 B by reusing the existing UV alias with no new bytes at all.
4. **The counter-argument's own premise is unproven here.** The claim "one draw always" is worth protecting rests on a path that no application currently drives (§7.3). Nobody has measured a boyko UI frame with sprites in it. Choosing the more invasive option on an unmeasured path is exactly the mistake this codebase keeps recording.

**Why the recommendation still stands:** the atlas route's savings are one-time and its costs are permanent mechanisms — a runtime allocator, an eviction policy, per-tile padding and mip policy, a `REPEAT` emulation, and a hard ceiling on sprite size — while bindless's cost is a device feature the engine **already requires at boot** and a table that already exists, is already fence-gated, and already stores the slot on `TextureGpu`. The honest disposition: **prototype Model B, and make the slot-budget and `NonUniformResourceIndex` costs a measured gate before the rung is called done** — a UI frame with N textured quads at 1, 8 and 64 distinct slots, against the same frame with all quads on one slot. If the divergence cost is real, Model A remains reachable *without changing the component model at all*, because `UiImage.texture` + `uv` describes an atlas tile and a bindless slot equally well. That reversibility is the reason to start with B.

---

## 11 · Open questions for the architecture rung

1. **Slot budget policy** — does the UI get a reserved sub-range of the 4096 bindless slots, or does it compete? Nothing today reserves anything.
2. **Union vs bit-stuffing** for the widened `UiInstance` (Godot unions; WebRender bit-stuffs; the crate's `flags` already bit-stuffs).
3. **Does the text lane migrate to the real `uv` field** in the same commit? It should — one field, one meaning — but it re-blesses every text golden.
4. **Per-slot generation gate** — fix the specified gate to `[u64; FRAMES_IN_FLIGHT]` while the record is being touched, or leave the whole gate for a separate rung? (It is dead either way today.)
5. **`ui_setup`'s mandatory font** — a sprite-only UI must be able to boot.
6. **Aether construct** — `Construct` today is `component | tag | bundle | event | system | plugin | machine | material | scene` (`crates/aether_lang/src/ast.rs:152`). A `sprite`/`sheet` construct would be the tenth; the sheet table's dense `u16 sheet_id` mint is the natural thing for it to own at expand time.
7. **Overdraw / opaque pre-pass** — noted and deliberately deferred (§6).

---

## Sources

**Primary source (read directly).**
- Bevy — [`bevy_sprite/src/sprite.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_sprite/src/sprite.rs), [`bevy_sprite_render/src/render/mod.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_sprite_render/src/render/mod.rs), [`bevy_image/src/texture_atlas.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_image/src/texture_atlas.rs), [`bevy_ui_render/src/lib.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ui_render/src/lib.rs), [`bevy_ui_render/src/ui_texture_slice_pipeline.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_ui_render/src/ui_texture_slice_pipeline.rs), [`examples/2d/sprite_animation.rs`](https://raw.githubusercontent.com/bevyengine/bevy/main/examples/2d/sprite_animation.rs)
- Godot 4 — [`servers/rendering/renderer_rd/renderer_canvas_render_rd.h`](https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/renderer_canvas_render_rd.h); [canvas batching PR #92797](https://github.com/godotengine/godot/pull/92797)
- Unity UGUI — [`UnityEngine.UI/Core/Image.cs`](https://github.com/MooseYang/UGUI/blob/master/UnityEngine.UI/Core/Image.cs) (`GenerateSlicedSprite` / `GenerateTiledSprite`)
- RmlUi — [`Include/RmlUi/Core/RenderInterface.h`](https://raw.githubusercontent.com/mikke89/RmlUi/master/Include/RmlUi/Core/RenderInterface.h)
- WebRender — [nical, *GUIs on the GPU* (notes)](https://nical.github.io/drafts/gui-gpu-notes.html); [*Improving texture atlas allocation in WebRender*](https://mozillagfx.wordpress.com/2021/02/04/improving-texture-atlas-allocation-in-webrender/); [*Eight million pixels and counting*](https://nical.github.io/posts/etagere.html)

**Secondary / documentation.**
- Dear ImGui — [`ImDrawCmd` reference](https://docs.rs/imgui-sys/latest/imgui_sys/struct.ImDrawCmd.html); [issue #2591 (VtxOffset / 64k meshes)](https://github.com/ocornut/imgui/issues/2591)
- Bevy docs — [`TextureSlicer`](https://docs.rs/bevy/latest/bevy/ui/prelude/struct.TextureSlicer.html), [`NodeImageMode`](https://docs.rs/bevy/latest/bevy/ui/widget/enum.NodeImageMode.html), [`ExtractedSprite`](https://docs.rs/bevy/latest/bevy/sprite/struct.ExtractedSprite.html); [Sprite Batching PR #3060](https://github.com/bevyengine/bevy/pull/3060)
- Unity — [9-slicing manual](https://docs.unity3d.com/6000.1/Documentation/Manual/sprite/9-slice/9-slicing.html), [`UI.Image.Type.Sliced`](https://docs.unity3d.com/550/Documentation/ScriptReference/UI.Image.Type.Sliced.html), [UI Toolkit performance](https://docs.unity3d.com/6000.4/Documentation/Manual/best-practice-guides/ui-toolkit-for-advanced-unity-developers/optimizing-performance.html)
- Godot — [`SpriteFrames`](https://docs.godotengine.org/en/stable/classes/class_spriteframes.html), [`AnimatedSprite2D`](https://docs.godotengine.org/en/stable/classes/class_animatedsprite2d.html), [`CanvasItem`](https://docs.godotengine.org/en/stable/classes/class_canvasitem.html)
- Unreal — [`UPaperFlipbook`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Plugins/Paper2D/UPaperFlipbook), [`PaperFlipbookKeyFrame`](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/PaperFlipbookKeyFrame?application_version=4.27)
- Aseprite — [tags docs](https://www.aseprite.org/docs/tags/), [exporting docs](https://www.aseprite.org/docs/exporting/), [Tag API](https://www.aseprite.org/api/tag)
- Packing — Jukka Jylänki, [*A Thousand Ways to Pack the Bin*](https://m.moam.info/a-thousand-ways-to-pack-the-bin-jukka-jylanki_6479d6d7097c4770028bb82b.html)
- Bindless — [MJP, *Bindless Texturing for Deferred Rendering and Decals*](https://mynameismjp.wordpress.com/2016/03/25/bindless-texturing-for-deferred-rendering-and-decals/); [ktstephano, *Bindless Textures*](https://ktstephano.github.io/rendering/opengl/bindless)

**In-tree (this worktree, `feat/ui-advanced`).**
`crates/boyko_ui/src/components.rs` · `crates/boyko_ui/src/bundles.rs` · `crates/boyko_ui/src/text/dispatch.rs` · `crates/boyko_render/src/ui/{mod,instance,pack,plan,draw,upload,resources}.rs` · `crates/boyko_render/shaders/ui_rect.{vs,fs}.hlsl` · `crates/boyko_render/src/bindless.rs` · `crates/boyko_render/src/texture.rs` · `crates/boyko_rhi_vulkan/src/bindless.rs` · `crates/boyko_fontbake/src/atlas.rs` · `crates/aether_lang/src/ast.rs`
