# Architecture: GUI Phase P5b — Full MSDF Text Rendering (in-house generation, riding the P5a quad path)

## Goal

Render glyph text on the in-house Vulkan path as **crisp, resolution-independent, sharp-cornered** quads at any scale, using **in-house MSDF generation** (canonical Chlumsky pipeline incl. edge-coloring, scanline sign-correction, and the mandatory error-correction pass) and the **P5a instanced-quad render path** (one draw, premultiplied blend, `fwidth` AA). The engine's value-add is the offline/load-time MSDF generator + atlas baker; the runtime hot path adds exactly **one atlas sample + a 3-op `median()` + `screenPxRange`** per fragment over the existing P5a rounded-rect SDF cost.

**Performance targets:**
- **Runtime per-fragment**: +1 `texture()` sample (bilinear, RGBA8 MTSDF or R8 SDF) + `median(r,g,b)` (3 ALU) + `screenPxRange()` (≈5 ALU) over the P5a fill path. No marching, no loops. On par with `sdRoundedBox`. The rect majority is unregressed (the `FLAG_TEXT` branch is uniform-per-instance; proven by the A/B bench, §Metrics M5).
- **Runtime per-glyph CPU (layout/emit)**: `O(glyphs)` table arithmetic over the metrics table; **zero per-frame heap allocation** — emit/measure scratch is sized once at setup to the hard `UiTextBuffer::CAP` bound (§Decision T5-A) and asserted never to realloc. One `UiInstance` emitted per visible glyph — glyph quads and rect quads interleave in one z-sorted instance stream and one `draw(6,N,0,0)`.
- **Bake-time (load/setup, NOT hot path)**: MSDF generation is `O(atlas_texels × edges_per_glyph)` for distance, plus `O(atlas_texels)` for sign-correction and error-correction; embarrassingly parallel per texel on the engine threadpool. Transient `Vec` scratch is a Principle-0 legitimate exception (load-time, discarded). Produces one atlas image + one metrics table, both then ECS-resident.
- **Memory**: one atlas texture (ASCII+Latin-1 at 48px/em). **Atlas kind is an owner VALUES call tied to P7** (§Decision T0-D): MTSDF RGBA8 (≈512×512×4 = 1 MiB) if P7 diegetic HUD is committed; MSDF R8 (≈256 KiB) if fill-only. ECS-resident metrics table (≈224 glyphs × 48 B ≈ 11 KiB). No per-frame atlas writes (upload-once, all-FIF sample concurrently — the `SampledComposite` pattern).

---

## Context and constraints

- **Subsystems touched**: `boyko_ui` (new `UiText`/`FontId`/`TextAlign` components; the glyph-emitter + text-measure systems; the `ui!`/`.ui` authoring arms; the `FontTable` Resource); `boyko_render` (extend `UiInstance` with a text UV lane via the `corner_radius`→`uv` alias + `FLAG_TEXT`; the MSDF text FS branch; a second bind-group binding for the atlas + a third for the per-atlas uniform UBO; the glyph-quad emitter feeding the existing `UiNode` stream); `boyko_rhi`/`boyko_rhi_vulkan` (**one small owned RHI delta** — a `mip` control on `SamplerDesc`, §Decision T4-D — plus the per-atlas-uniform delivery via an existing `UniformBuffer` binding, §Decision T4-A); a **new in-house bake crate** `boyko_fontbake` for MSDF generation + atlas packing + font extraction (load-time tool, off the engine hot path).
- **Invariants preserved**:
  - **Principle 0** — no parallel data system. The glyph metrics table + atlas handle are **ECS-resident** (`Resource`-owned columns / a `!Send` GPU-asset owner on `RhiContext`, identical class to `UiRenderResources`). Glyphs are emitted into the **existing `UiNode`/`UiInstance` stream**. The only `std::Vec`/`HashMap` are (a) load-time bake scratch (discarded) and (b) the FFI atlas upload buffer (the documented GPU-contiguity exception).
  - **Principle 1/5** — no `Box<dyn>`/`HashMap`/`Vec::new()`/realloc on the per-frame emit/upload path; metrics lookup is an array indexed by a dense glyph slot (not a `HashMap`); emit/measure scratch is preallocated to a setup-time worst case bounded by `UiTextBuffer::CAP` (§Decision T5-A).
  - **P5a render contract** — text uses the **same single pipeline, single z-sort, single draw, premultiplied `src=ONE` blend**. The atlas sample is additive (a second bind-group binding at FRAGMENT, a proven `CombinedImageSampler` capability).
  - `RhiContext` stays `!Send + !Sync`; the atlas + per-atlas UBO + bind-groups are owned by it, Drop-wired (Decision 8 precedent). MF-7 handle re-resolution preserved.
  - Every `unsafe` carries `// SAFETY:`; the GPU golden (validation = zero messages + texel asserts) is the soundness/correctness oracle, per the P5a/Phase-5 precedent.
- **Out of scope** (seams listed in §Out of scope): complex-script shaping (HarfBuzz/bidi/ligatures/CJK), RTL/BiDi, color emoji, dynamic runtime glyph generation, world-space text (P7), subpixel/LCD AA, rich-text runs, text selection/editing/IME. **CFF/OTF-PS support is conditional on the font-fork choice** (§Out of scope, gated as a tracked seam if Option A is taken).

---

## Phased sub-rungs (each independently shippable + GPU-gated)

The decomposition follows the research framing — **bake-time first (T1–T3, pure CPU, no GPU), then the runtime path (T4–T6, GPU-gated)**. T1→T3 produce a **checked-in `.bfont` asset**, so T4 (the GPU shader) tests against a *known-good* field — isolating "is the field wrong" from "is the shader wrong". T4 ships a single-glyph render before any layout. T5 adds CPU emit/layout once the GPU path is proven. T6 is authoring + dogfood.

| Rung | Title | Ships | Gate |
|------|-------|-------|------|
| **T0** | Font-parsing fork decision + `FontFace` adapter trait + checked-in fixtures | The in-house `FontFace` trait + the chosen backend (§Font-parsing fork); checked-in `.ttf` (TrueType) **and** `.otf` (CFF) golden fixtures | Unit: extract outline+metrics for the checked-in TTF; outline byte-exact vs a golden segment list. (If Option A: the `.otf` fixture gates the documented-unsupported seam, not extraction.) |
| **T1** | Outline + metrics extraction | `boyko_fontbake::extract` — glyph outline (line/quad/cubic segments, em-normalized) + per-glyph advance/LSB/bbox + face metrics via the T0 `FontFace` | Unit: known glyph (`A`,`o`,`.`) segment counts + winding; metrics match reference values |
| **T2a** | Edge-coloring + per-channel pseudo-distance + winding | `boyko_fontbake::msdf` color/distance core — edge coloring (seeded, max-angle corner predicate, 0/1/N-corner handling) + per-channel signed pseudo-distance (line + quadratic Cardano + cubic multi-seed Newton) + raw winding sign + range mapping with the **pinned global pixels-per-em** binding | **CPU golden**: `A`/`o`/`.` + a smooth `O` (0-corner) + a teardrop median-reconstruct sharp; per-channel distances match a brute-force fine-sampled reference within tolerance |
| **T2b** | Scanline sign-correction + overlap preprocessing | The path-preprocessing (overlapping/self-intersecting contour combiner) + the **scanline sign-correction pass** that re-derives authoritative inside/outside and overrides the pseudo-distance sign | **CPU golden**: an overlapping-contour glyph (`8`, accented `é` with separate base+mark) has **zero inverted-interior texels** |
| **T2c** | MSDF error-correction pass (mandatory) | The msdfgen error-correction step: analytically predict bilinear interpolation between adjacent texels; where the median would introduce a spurious edge (sign flip) not in the true distance, collapse that texel's channels to the true single-channel distance | **CPU golden**: a glyph KNOWN to speckle pre-correction (thin-waist `e`/`g` bowl) median-reconstructs **clean** post-correction; pre-correction control asserts the speckle existed (so the pass is proven load-bearing) |
| **T3** | Atlas packing + glyph/UV/metrics table | `boyko_fontbake::atlas` — skyline packer → one atlas image + a POD `GlyphMetrics` table (planeBounds/atlasBounds over the **expanded transition-band quad**) + `AtlasMeta` (distanceRange texels, pixels-per-em, atlas size, line metrics, atlas_kind); serialized to a checked-in `.bfont` (in-house binary) | Unit: packed glyphs non-overlap + inter-glyph spacing ≥ pxrange/2; **field generated over the expanded bbox**, plane/atlasBounds describe the expanded quad; round-trip byte-identical; UV rects inside atlas bounds; **per-glyph plane↔atlas scale == global pixels-per-em (no drift)** |
| **T4** | Atlas RHI binding + per-atlas UBO + MSDF text shader on the P5a path | Extend `UiInstance` (`corner_radius`→`uv` alias + `FLAG_TEXT`); the MSDF text FS branch (`median` + `screenPxRange`, NaN-floored, premultiplied); **binding 1** (`CombinedImageSampler` @ FRAGMENT) + **binding 2** (`UniformBuffer` @ FRAGMENT, per-atlas pxRange+atlasSize) on the UI layout; the `UiAtlas` + `UiAtlasUniform` owned by `UiRenderResources` (so the grow path can re-bind, §Decision T4-C); the no-mip sampler (§Decision T4-D); the shared-VS `local_uv` varying (§Decision T4-B) | **GPU golden** (RTX 3060, validation = zero msgs): G-T4.1 crisp single glyph; G-T4.2 median preserves a corner vs an SDF control; G-T4.3 **multi-scale on TWO distinct-footprint glyphs** (16/48/96 px, AA band ~1–2 device px); G-T4.4 atlas UV correctness; G-T4.5 single `draw(6,N,0,0)`; G-T4.6 NaN-safe flat field; **G-T4.7 grow re-bind** (overflow N then sample correctly on the grown slot); **G-T4.8 minified no-corruption** (sample at a minified scale, no median corruption — proves no-mip) |
| **T5** | Shaping/layout + glyph-quad emitter + `UiText` + `ContentSize` measure | `UiText{font,size,color,align}` + content via existing `UiTextBuffer`; the Latin layout (advance/kerning/wrap/baseline) emitting `UiNode`s into the existing stream; the change-gated text-measure system writing `ContentSize` set-if-changed; **measure ordered before layout in the schedule** | **GPU golden**: G-T5.1 pen positions; G-T5.2 kerning tighter; G-T5.3 word-wrap at whitespace; G-T5.4 multiline baseline. Unit: measure→`ContentSize` matches shaped run; `Auto` text node hugs; **emit/measure scratch never reallocs** (counting allocator) |
| **T6** | Authoring (`ui!`/`.ui` `UiText`) + dogfood | `parse_ui_text` `.ui` arm + `ui!` `UiText{...}` lowering; a dogfood HUD label in `boyko_demo` (a `Health` readout via `BindText`→`UiTextBuffer`) | Integration: `.ui` ≡ `ui!` text-node archetypes; a bound label renders the live value, re-emits only on `Changed`; the demo HUD shows crisp text end-to-end |

**Rationale for the T2 split:** the critic correctly identified that the original T2 collapsed three distinct canonical passes (distance, sign-correction, error-correction) into "range mapping → done". Splitting them into T2a/T2b/T2c makes each a named rung with its own CPU golden, so a generator regression in any one pass is caught in isolation, and the error-correction pass — the single most common reason in-house MSDF "looks generated but is subtly wrong" — cannot be silently skipped.

---

## Component + data model (all ECS-native / engine storage)

### New ECS components (`boyko_ui/src/text/components.rs`)

```rust
/// Text style for a node. AUTHOR-OWNED, OPT-IN. The CONTENT is the existing
/// `UiTextBuffer` (P4 sink, tick-bearing); UiText carries STYLE only, so a
/// content-only change bumps only UiTextBuffer's tick and a style-only change bumps
/// only UiText's tick (independent churn columns — Principle 2 hot/cold split).
/// 16 B, #[repr(C)], POD Copy.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiText {
    pub color: u32,        // STRAIGHT RGBA8 authored; premultiplied at emit (P5a convention)
    pub size_px: f32,      // logical px font size (em); scale_factor folded at emit
    pub font: FontId,      // dense u16 index into the loaded-font table (NOT a string/HashMap)
    pub align: TextAlign,  // u8: Left|Center|Right (line alignment within the rect)
    pub _pad: u8,
}
// Default: opaque white, 16 px, font 0, Left. A node with UiText + a non-empty
// UiTextBuffer renders text; absent UiText ⇒ no text (rect-only, P5a unchanged).

/// Dense font handle — a u16 index into the FontTable resource (Principle 1).
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontId(pub u16);

#[repr(u8)]
pub enum TextAlign { Left = 0, Center = 1, Right = 2 }
```

**Why a separate `UiText` (style) vs reusing `UiTextBuffer` (content):** content and style are independent churn axes (a value-bound label changes content every frame but style never). `UiTextBuffer` is already tick-bearing and `Changed`-gateable; `UiText` is the style sibling, mirroring how `UiBackground` is the style sibling to `ComputedRect`. `font` is a `u16` dense index, never a string or `HashMap` — Principle 1.

### Font + atlas + metrics — ECS-resident (Principle 0), NOT a side store

```rust
/// CPU-side per-font metadata: glyph metrics + line metrics + cmap + kerning. A
/// Resource-owned column (engine storage), NOT a HashMap side store. Loaded once at
/// setup from a .bfont asset.
pub struct FontTable {
    fonts: Box<[FontEntry]>,           // dense, indexed by FontId.0 (setup-time alloc, never grows in-frame)
}
pub struct FontEntry {
    glyphs: Box<[GlyphMetrics]>,       // dense, indexed by a per-font glyph slot
    cmap: CodepointMap,                // codepoint→glyph-slot: sorted [(u32 cp, u16 slot)] + binary search; cp<128 direct-array fast path
    kern: KernTable,                   // sorted [(u32 left_right_packed, i16 adjust)] + binary search (or empty)
    meta: AtlasMeta,
    atlas_asset: AtlasAssetId,         // dense index into the GPU atlas owner
}

/// One glyph's render+layout metrics — POD. ~48 B. planeBounds/atlasBounds describe
/// the EXPANDED transition-band quad (tight bbox + distanceRange/2 + ε on all sides;
/// §T3), NOT the tight silhouette, so the AA band is never clipped.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GlyphMetrics {
    pub advance_em: f32,               // pen advance (em units)
    pub plane: [f32; 4],               // planeBounds left,bottom,right,top (em, EXPANDED quad rel. baseline)
    pub atlas: [f32; 4],               // atlasBounds left,bottom,right,top (TEXELS, EXPANDED quad)
}

/// Per-FONT atlas metadata. distance_range_texels + pixels_per_em are BOTH carried:
/// they are bound by `range_em = distance_range_texels / pixels_per_em` (§T2a/§T3).
/// pixels_per_em is GLOBAL across all glyphs of this atlas (one uniform rasterization
/// scale), which is what makes the shader's single screenPxRange uniform valid.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AtlasMeta {
    pub distance_range_texels: f32,    // pxrange in TEXELS (4 for MSDF, 6 for MTSDF)
    pub pixels_per_em: f32,            // GLOBAL rasterization scale (binds range_em)
    pub atlas_w: u32,
    pub atlas_h: u32,
    pub ascender_em: f32,
    pub descender_em: f32,
    pub line_gap_em: f32,
    pub kind: AtlasKind,               // u32: Msdf | Mtsdf
}
```

`CodepointMap`/`KernTable` are **sorted POD slices + binary search**, never `HashMap` — matching the engine's serialization `LoadEntityMap` precedent (the sorted-Vec+binary-search rewrite after the DoS bug). Lookup is `O(log glyphs)`; for ASCII a 128-entry direct array fast-path (`cp < 128`) makes the common case `O(1)`.

### GPU atlas asset + per-atlas uniform — owned by `UiRenderResources` (the grow-path fix, §Decision T4-C)

```rust
/// One loaded MSDF atlas on the GPU: an upload-once SAMPLED texture + its no-mip
/// sampler + the per-atlas uniform UBO (pxRange, atlasSize). Owned as a FIELD ON
/// `UiRenderResources` (NOT on RhiContext directly) so `create_slot`/`grow_slot` can
/// re-bind binding 1 + binding 2 when a ring grow rebuilds the bind-group. Upload-once,
/// all-FIF sample concurrently, NO per-frame barrier (the SampledComposite pattern).
struct UiAtlas {
    texture: VulkanTexture,            // SAMPLED|TRANSFER_DST, ShaderReadOnlyOptimal after upload
    sampler: VulkanSampler,            // Linear/Linear, ClampToEdge, NO mips (Decision T4-D)
    uniform: BoundBuffer,              // host-visible UBO holding UiAtlasUniform (16 B), written once at setup
}

/// The per-atlas FRAGMENT uniform: distanceRange (texels) + atlas size (texels).
/// 16 B std140-compatible (the UBO scalar/vec2 layout the FS reads). Written once at
/// atlas upload; immutable thereafter (one atlas → one constant pxRange/size).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UiAtlasUniform {
    pub px_range: f32,                 // = AtlasMeta.distance_range_texels
    pub _pad0: f32,
    pub atlas_size: [f32; 2],          // (atlas_w, atlas_h) as f32
}
const _: () = assert!(size_of::<UiAtlasUniform>() == 16);
```

The atlas binding is **added to the UI bind-group layout as binding 1** (`CombinedImageSampler` @ FRAGMENT) and the per-atlas uniform as **binding 2** (`UniformBuffer` @ FRAGMENT) alongside the existing binding-0 storage buffer (verified: `BindGroupLayoutDesc.entries` is a slice, `MAX_BIND_GROUP_BINDINGS` caps it, `DescriptorKind::UniformBuffer` + `DescriptorKind::CombinedImageSampler` + `BindGroupEntry::CombinedImage` all exist and are GPU-proven). **Ownership on `UiRenderResources` (not `RhiContext`) is load-bearing** — see Decision T4-C.

---

## The MSDF generation algorithm (the in-house core, T1–T3)

This is the offline/load-time value-add. All of it is pure scalar math over transient `Vec` scratch (legitimate Principle-0 exception — load-time, discarded), parallelizable per texel on the engine threadpool. **This section now matches the canonical Chlumsky pipeline; the three previously-collapsed passes are each named and gated.**

### T1 — Outline extraction (via the T0 `FontFace`)
1. `face.glyph_index(cp)` → glyph id; `.notdef` (slot 0) fallback for missing.
2. `face.outline(gid, &mut sink)` collects contours; each contour is a closed list of **edge segments**: `Line{p0,p1}`, `Quad{p0,c,p1}`, `Cubic{p0,c0,c1,p1}` in em-normalized coords (divide by `unitsPerEm`).
3. Record per-glyph `advance_em`, LSB, bbox; face `unitsPerEm/ascender/descender/lineGap`.

### T2a — Edge-coloring + per-channel pseudo-distance + raw winding

**(a) Path preprocessing + normalize + edge-color** (per contour):
- `normalize()`: split a single-edge or fully-smooth contour so three colors can be placed; fix orientation/winding.
- `EdgeColor` is an RGB bitmask (`R=1,G=2,B=4`), invariant **every edge lights ≥2 channels** (YELLOW=R+G, MAGENTA=R+B, CYAN=G+B).
- **Corner predicate (corrected to match msdfgen `edge-coloring.h`):** a vertex is a **corner** when the turn angle between the incoming and outgoing edge directions **exceeds the configured max-angle** (`maxAngle`, canonical default ≈ 3.0 rad). Concretely, with `cross = aDir.x*bDir.y − aDir.y*bDir.x` and `dot = aDir·bDir`, it is a corner when `dot <= 0 || |cross| > crossThreshold` where `crossThreshold = sin(maxAngle)` — i.e. `maxAngle` is the *upper* (nearly-straight) cutoff: joins flatter than `maxAngle` are NOT corners. (The original plan's polarity reading was wrong; this restatement is verified against `edge-coloring.h`.)
- **Seeded switch:** a deterministic seeded pseudo-random `switchColor()` at each corner rotates the bitmask so the **two edges meeting at the corner share exactly one channel** (the shared channel keeps the sharp median intersection; the others extrapolate straight), AND breaks degenerate same-color adjacencies. The seed is a **pinned bake-config constant** (§Decision T2-E — determinism).
- **Explicit topology cases (no longer "in passing"):**
  - **0-corner (smooth loop, e.g. `O`, `o`)**: one color cannot represent the ring — the contour is split (msdfgen's smooth-loop split) so the median reconstructs a clean ring. Gated by the T2a smooth-`O` golden.
  - **1-corner (teardrop, e.g. a serif terminal)**: split-in-thirds handling so the single tip is preserved. Gated by the teardrop golden.
  - **N-corner**: the standard per-corner switch walk.

**(b) Per-texel, per-channel signed pseudo-distance** (embarrassingly parallel — one work item per texel on the threadpool):
- For each channel `c ∈ {R,G,B}`: over edges whose color includes `c`, compute the **signed pseudo-distance** (perpendicular/extrapolated, NOT clamped to endpoints — this straightens corners), keep the min unsigned.
  - **Line**: closed-form point-segment perpendicular distance.
  - **Quadratic** `B(t)=(1−t)²P₀+2(1−t)tC+t²P₁`: `d/dt|B(t)−p|²=0` is a **cubic in t** — solve via Cardano, also test endpoints, take min.
  - **Cubic** (CFF/OTF only): the nearest-point problem is a **quintic**. Strategy (specified, not "Newton from seeds"): seed Newton from a **fixed small set of evenly-spaced t-values in [0,1]** (msdfgen's fixed search count, default 4), refine each by Newton on the squared-distance derivative, **also test both endpoints**, take the global min. Multiple seeds avoid the S-shaped-cubic local-minimum nick. **Gated** by a CFF-glyph golden: cubic-segment distances match a brute-force fine-sampled reference within tolerance. (If Option A / no-CFF is taken, this path is unreachable; the rung asserts no cubic segments are produced and the golden is N/A — see §Font-parsing fork.)
- **Raw winding sign** from contour winding (nonzero rule). **This is provisional** — it is overridden by the T2b scanline pass (the original plan treated bare winding as authoritative; it is not).
- **MTSDF 4th channel (A)** (only when `kind == Mtsdf`): the true single-channel SDF — `min` over ALL edges regardless of color — generated with the **identical `range_em` mapping and 0.5 zero-crossing** as the rgb channels (§Decision T2-F), so P7 effects key off the same edge as the fill.

**(c) Range mapping — the pinned global pixels-per-em binding (the named "most common MSDF bug" fix):**
- All glyphs are rasterized at **one uniform `pixels_per_em`** (the atlas em size). Therefore `texels_per_em` is **global**, and `range_em = distance_range_texels / pixels_per_em`.
- `value = signed_distance_em / range_em + 0.5`, clamped `[0,1]`.
- Both `distance_range_texels` AND `pixels_per_em` are carried into `AtlasMeta`. The shader's `screenPxRange()` uses `distance_range_texels` directly because the uniform pixels-per-em guarantees the same texel count spans the transition for every glyph.
- **Bake `distance_range_texels = 4` for MSDF, `6` for MTSDF.**
- **Gate (T3 unit):** assert that **every glyph's plane↔atlas extent ratio equals the global `pixels_per_em`** (no per-glyph scale drift). G-T4.3 runs on **two distinct-footprint glyphs** to catch drift the shader cannot correct.

### T2b — Scanline sign-correction + overlap preprocessing (the inverted-interior fix)

The critic correctly flagged that bare nonzero-winding point-in-polygon is the **known-insufficient** sign source. Two halves, both explicit:
1. **Overlap preprocessing**: resolve overlapping/self-intersecting contours into a consistent winding before per-texel distance — adopt msdfgen's overlapping-contour distance combiner (combine per-contour distances respecting union winding) rather than assuming pre-unioned outlines. (The original plan's "self-intersections handled" claim is **dropped** until this step exists — it now exists as this named rung.)
2. **Scanline sign-correction pass**: after distance generation, for each scanline re-derive the **authoritative** inside/outside (a horizontal ray-cast nonzero/even-odd intersection count against the true outline) and **override** the pseudo-distance sign where it disagrees. This reconciles the unclamped-extrapolated pseudo-distance (which can yield the wrong sign near corners/overlaps) with true insideness.
- **Gate (T2b CPU golden):** an overlapping-contour glyph (`8`, or an accented `é` with separate base+mark contours) has **zero inverted-interior texels** (every interior texel median-reconstructs inside).

### T2c — MSDF error-correction pass (mandatory — the speckle fix)

The critic correctly flagged this as **entirely absent** and the #1 reason naive in-house MSDF is subtly wrong. The shader median is downstream of the bake and **cannot recover an inconsistent field**.
- The pass (msdfgen `msdf-error-correction`): for each texel, **analytically predict the result of bilinear interpolation toward each neighbor**; where the median would introduce a **spurious edge** (a sign flip in `median(rgb)` not present in the true distance — a stray bright/dark speck on a smooth run), **collapse that texel's channels to the true single-channel distance** (the `.a` SDF value, or a freshly computed true distance).
- **Gate (T2c CPU golden):** a glyph KNOWN to artifact pre-correction (thin-waist `e`/`g` bowl, near-parallel channel transition) median-reconstructs **clean** post-correction; a **pre-correction control** asserts the speckle existed (proving the pass is load-bearing, not a no-op).

### T3 — Atlas packing + table (with the expanded transition-band quad)

- **Skyline packer** (`stb_rect_pack`/fontstash precedent): track the upper contour, drop each glyph rect into the lowest valid slot; glyphs sorted by height first for density. Incremental-friendly (the future dynamic-atlas seam).
- **The field is generated over an EXPANDED region** = tight glyph bbox + `(distance_range_texels/2 + ε)` texels on **all four sides** (the critic's fix: padding is not just inter-glyph spacing — the transition band must be *represented* in the field, and the UI quad must *cover* it, or edges read hard-cut at large scale). **Both `planeBounds` (em) and `atlasBounds` (texels) describe this expanded quad**, so the emitted UI quad covers the full AA band.
- **Inter-glyph spacing ≥ distance_range_texels/2** between packed rects (prevents bilinear neighbor bleed); outer atlas border too.
- Emit: one atlas image (RGBA8 MTSDF / R8 SDF per `kind`) + the dense `GlyphMetrics[]` + `AtlasMeta` + the `cmap` (sorted) + `kern` (sorted).
- Serialize to a checked-in **`.bfont`** in-house binary (mirrors the engine's "codegen binary, not serde" serialization decision): a small header + the POD tables blitted + the atlas pixels. Round-trip byte-identical (gate).
- **Edge cases**: empty glyph (space — no outline, advance only, NO atlas entry); single-contour smooth (one color → split per T2a); `.notdef` always slot 0; glyph entirely outside its planeBounds (degenerate — emit zero-area, skip at emit).

---

## The text shader + how it extends the P5a `ui_rect` pipeline (T4)

### `UiInstance` extension — a text lane + `FLAG_TEXT` (single pipeline, Decision T4-G)

The current `UiInstance` is **64 B with no free lane** (verified: instance.rs — all 8 fields occupied; only a `FLAG_TEXT` flag bit is free). Text needs a **UV rect** (atlasBounds normalized → `[f32;4]`). Two ratified facts force the layout:

1. **A rect node never sets a UV; a text (glyph) node never sets `corner_radius`/`border`.** These are **mutually exclusive by `FLAG_TEXT`** — so the text UV can **alias `corner_radius` (offset 32, `[f32;4]`, verified instance.rs:74)**, reinterpreted in the FS when `FLAG_TEXT` is set. `UiInstance` stays **64 B, the std430 oracle unchanged, one pipeline, one draw**.
2. `clip` (off 16) is shared (text clips too); `color` is shared (premultiplied fg); `min_px`/`size_px` are the glyph quad (planeBounds × size + pen). `pxrange`/`atlasSize` are **per-atlas uniforms** delivered via the binding-2 UBO (§Decision T4-A), NOT per-instance — all glyphs of one atlas share them.

**Decision T4-G (chosen): alias `corner_radius`→`uv` under `FLAG_TEXT`, single pipeline.** Preserves every P5a invariant (one z-sort, one draw, premultiplied blend, the 64 B std430 oracle). **Alternatives rejected**: a sibling text struct + second pipeline + second draw (breaks single-z-sort — text and rects must interleave by `StackIndex`, doubles draw calls); widen `UiInstance` to 80 B (wastes 16 B/rect for a field rects never use). **Forward-looking trigger (critic-recorded):** a future **textured/image-fill/nine-slice rect** would need `corner_radius` AND a UV on the same instance, which would retire this alias and force the 80 B widen. This is recorded as the explicit deliberate-revisit trigger (debug-asserted: non-text instances never read the uv alias) — accepted for P5b because no current rect feature needs both.

The HLSL `struct UiInstance` mirrors the same 64 B + a `// when FLAG_TEXT: corner_radius reinterpreted as uv(left,top,right,bottom) normalized` comment + the `FLAG_TEXT` constant; the per-field offset oracle pins it. **Rust-side `corner_radius` stays `[f32;4]`** — the reinterpret is value-only in the shader, so the `offset_of!` oracle and the Miri-TB byte-view are unchanged (only the `slice_as_bytes` extension, if any, gets Miri).

### Decision T4-B: the shared VS gains a `local_uv` varying (explicit, oracle-pinned)

The shipped `VsOut` (ui_rect.vs.hlsl) forwards `pos_px (TEXCOORD0)`, `local_px (TEXCOORD1)`, `inst_index (INSTANCE)` — there is **no `local_uv`**. The VS already computes `corner = CORNERS[vid]` (the 0..1 quad corner). **T4a forwards `corner` as `local_uv : TEXCOORDn` for BOTH branches** (a one-line VS add). The rect branch ignores it; the text branch uses `lerp(inst.uv.xy, inst.uv.zw, in.local_uv)`. The new varying + the `FLAG_TEXT` constant are pinned in the **same compile-time oracle block** that const-asserts the 64 B / per-field offsets. No Rust-struct change (the Miri-TB note stays accurate).

### Decision T4-A: per-atlas uniform delivery — a UBO at binding 2, NOT a push constant (the CRITICAL fix)

The critic correctly established (verified rhi_impl.rs:1332 + device.rs `GraphicsPipelineDesc`) that **the shipped UI graphics push range is hard-coded `VK_SHADER_STAGE_VERTEX_BIT`** with **no stage knob** on `GraphicsPipelineDesc` (only `push_constant_bytes`), and the recorder pushes VERTEX-only. The original plan's "extend the push block or a small FRAGMENT push range" is **materially false as a free change** — a FRAGMENT push constant would require widening the range to `VERTEX|FRAGMENT`, adding a stage field to `GraphicsPipelineDesc`, a second `cmd_push_constants` in the recorder, and a backend change to the hard-coded stage.

**Decision (chosen): deliver `pxRange` + `atlasSize` via a per-atlas `UniformBuffer` at set0/binding2 (FRAGMENT-visible).** This requires **no push-constant change at all** — `DescriptorKind::UniformBuffer` and `BindGroupEntry` for a uniform buffer already exist; the UBO is written once at atlas upload and is immutable; binding 2 is additive to the layout exactly like binding 1. The recorder's existing VERTEX-only push SAFETY comment **stays truthful and unchanged** (we add zero push bytes). The binding-2 UBO is owned by `UiAtlas` (so the grow path re-binds it, §Decision T4-C).
- **Alternatives rejected:** (a) widen the push range to `VERTEX|FRAGMENT` — touches `GraphicsPipelineDesc` (a stage field), the recorder (a second push call), and the backend's hard-coded `VK_SHADER_STAGE_VERTEX_BIT`; a real RHI surface delta with more code and more Miri/golden surface than a UBO, for no benefit (the values are per-atlas-constant, not per-draw-dynamic, so a UBO is the natural home). (b) fold `pxRange`/`atlasSize` into a reserved SSBO lane — touches the std430 oracle per-instance for a value that is per-atlas-constant; wasteful and oracle-churning.

### Decision T4-C: `UiAtlas` + the per-atlas UBO are owned by `UiRenderResources`, so the grow path re-binds (the grow-hole fix)

The critic correctly found (verified resources.rs:344-371, grow_slot:378-402) that **the per-FIF bind-group is rebuilt on every ring grow** via `create_slot`, which writes the descriptor set **once** with **only `BindGroupEntry::StorageBuffer`**, and `create_slot`/`grow_slot` are methods on `UiRenderResources` with **no handle to an atlas parked on `RhiContext`**. Parking the atlas on `RhiContext` (original plan) would make a grown bind-group **incomplete** for the now-two/three-binding layout (a validation error). MF-7 is *handle re-resolution*, not *descriptor-set completeness on grow*.

**Decision (chosen): own `UiAtlas` (texture + sampler + the binding-2 UBO) as a field on `UiRenderResources`** (alongside `slots`, `layout`, `pipeline`). Then `create_slot`/`grow_slot` write **all three** `BindGroupEntry`s (StorageBuffer @0, CombinedImage @1, UniformBuffer @2) — a grown slot is complete. The atlas teardown rides `UiRenderResources::destroy` (Decision-8 Drop wiring) in reverse order (slots → atlas → pipeline → layout → modules). `create`/`upload` are extended to take/hold the atlas; `create_slot` signature gains `&UiAtlas` (or reads `self.atlas` in `grow_slot`).
- **Gate:** **G-T4.7** grows a ring (push overflow N) and re-samples the atlas correctly on the grown slot — closing the previously-unproven two/three-binding grow hole.

### Decision T4-D: no-mip sampler — a small owned `SamplerDesc` delta (the median-corruption guard)

The critic correctly found (verified device.rs:62-71) that `SamplerDesc` exposes **only** `mag_filter`/`min_filter`/`address_mode` — **no mip/LOD field** — so the repeatedly-asserted "NO mips" requirement is **currently unexpressible** and would be correct only by backend accident.

**Decision (chosen): add a `mip` control to `SamplerDesc`** — a minimal owned RHI delta:
```rust
#[repr(C)]
pub struct SamplerDesc {
    pub mag_filter: Filter,
    pub min_filter: Filter,
    pub address_mode: AddressMode,
    pub mip: MipMode,                  // NEW: None (maxLod=0, no mip) | Linear (future); P5b uses None
}
#[repr(C)] pub enum MipMode { None = 0, /* future: Nearest, Linear */ }
impl Default for SamplerDesc { /* mip: MipMode::None (the existing no-mip behavior, now explicit) */ }
```
The Vulkan backend's `create_sampler` maps `MipMode::None` → `mipmapMode = NEAREST`, `maxLod = 0.0`, `minLod = 0.0` (no mipmapping). This is a **declared, gate-backed** guarantee, not a backend accident. The existing rung-5 default behavior is preserved (the default was already nearest/clamp; we now name the no-mip intent). The atlas sampler is **Linear/Linear mag/min, ClampToEdge, `MipMode::None`** (bilinear, no mips).
- **Alternative rejected:** rely on the backend default — the critic's exact concern: "correct by accident", silently corrupts the median if a future backend change enables mips, with no desc-level way to disable it.
- **Gate:** **G-T4.8** samples the atlas at a minified scale and asserts no median corruption (the property the no-mip rule protects).

### Decision T4-E: single-atlas / single-font contract for P5b (the multi-font fix)

The critic correctly found that `FontTable` is **multi-font** (`fonts: Box<[FontEntry]>`, each with its own `meta`/atlas), but the render path binds **one** atlas with **per-atlas** `pxRange`/`atlasSize` — so two fonts in two atlases **cannot interleave in the single z-sorted draw** the core invariant requires (the binding-2 uniform would be wrong for one of them).

**Decision (chosen): P5b ships a SINGLE resident atlas/font.** `FontId` is reserved (the data model supports many; the runtime binds one). This is **stated explicitly as the P5b contract**, not left as an open question. Multi-font is a **named seam** with its render cost documented: when it lands, either (a) `pxRange`/`atlasSize` move to **per-instance lanes** (or a small per-atlas table the FS indexes by an atlas-id carried in `flags`), so distinct-atlas glyphs still interleave in one z-sorted draw, or (b) a **texture-array atlas** indexed by a glyph's atlas slot. The `.bfont` format + `AtlasMeta.kind` already carry per-font metadata, so the multi-font swap is additive. **This determines that the per-atlas-UBO design (T4-A) is valid for P5b** (one atlas → one constant uniform).

### The MSDF text fragment branch (canonical Chlumsky, adapted to the engine HLSL)

Added to the existing `ui_rect.fs` as a `FLAG_TEXT` branch (one VS, one pipeline — Decision T4-G):

```glsl
float median(float r, float g, float b) { return max(min(r,g), min(max(r,g), b)); }
// pxRange + atlasSize from the binding-2 per-atlas UBO (FRAGMENT-visible).
float screenPxRange(float2 uv) {
    float2 unitRange   = atlasUbo.px_range / atlasUbo.atlas_size;
    float2 screenTexSz = 1.0 / fwidth(uv);
    return max(0.5 * dot(unitRange, screenTexSz), 1.0); // NaN/Inf floor (fwidth==0 → Inf)
}
// --- FLAG_TEXT branch ---
float2 uv  = lerp(inst.uv.xy, inst.uv.zw, in.local_uv);  // local_uv: the VS quad corner (Decision T4-B)
float4 msd = atlas.Sample(linearClampNoMip, uv);         // RGBA8 MTSDF (or .rrr for R8 SDF)
float  sd  = median(msd.r, msd.g, msd.b);
float  cov = clamp(screenPxRange(uv) * (sd - 0.5) + 0.5, 0.0, 1.0);
if (inst.flags & CLIP_PRESENT) cov *= clip_coverage(in.pos_px, inst.clip, fwidth(in.pos_px));
out.rgba = premul_rgb_a(inst.color) * cov;               // PREMULTIPLIED (matches P5a src=ONE)
```

**Correctness pins (all golden-gated):**
- `screenPxRange()` floored at `1.0`; `px_range`/`atlas_size` come from the binding-2 UBO carrying the **baked** `distance_range_texels` (gated by the multi-scale crispness golden on **two** glyphs).
- **NaN guard**: flat regions give `fwidth→0 ⇒ screenPxRange→Inf`; `max(...,1.0)` + final `clamp` contain it (G-T4.6).
- **No mips** on the atlas sampler (Decision T4-D; G-T4.8).
- **Premultiplied output** → identical compositing to P5a rects; no new blend state.
- **MTSDF effects** (P7, only when `kind == Mtsdf`): `.a` (true SDF, same 0.5 zero-crossing as rgb — Decision T2-F) drives an `smoothstep` at an offset isovalue for outline/glow/shadow, while `median(.rgb)` drives the sharp fill — one sampled channel, no extra texture. Deferred to P7; the `.a` channel is the baked seam.

### Decision T4-F: NO new RHI *texture/binding* surface (restated honestly)

The CombinedImageSampler atlas binding + UniformBuffer binding are **additive and proven** (`create_texture`/`create_sampler`/`BindGroupEntry::CombinedImage`/`DescriptorKind::UniformBuffer` all exist, GPU-proven by `graphics_sample`). The atlas is upload-once (`SampledComposite`: all FIF sample concurrently, no per-frame barrier). **However, the original blanket "no new RHI surface" claim is RETRACTED**: there is **one** small owned RHI delta — the `SamplerDesc.mip` field (Decision T4-D). The fragment-uniform delivery needs **no** push-constant/pipeline change (Decision T4-A routes it through an existing UBO binding). Net RHI surface delta: **one field on `SamplerDesc`** + its backend mapping.

---

## THE FONT-PARSING DEPENDENCY FORK (decision-ready for the owner)

> **This is the ONE genuine VALUES/SCOPE fork in all of P5b. It is self-contained here for an owner decision before T1. It affects OUTLINE + METRICS EXTRACTION ONLY — MSDF generation (T2a/b/c), atlas packing (T3), the shader + render path (T4), shaping/layout (T5), and authoring (T6) are 100% in-house regardless of this choice.** Font parsing is **load-time, boring, fully-specified, and carries ZERO runtime/perf relevance** — none of the engine's hot-path principles (zero-alloc, lock-free, SIMD, cache) bind it. The engine's perf value-add is the MSDF generator + atlas + render, not re-parsing a binary font format. The only axis that matters here is the engine's in-house / no-external-deps philosophy vs correctness on the real-world font long tail.

**What must be extracted (all options must deliver this):** glyph outlines (`glyf` quadratic, or CFF/CFF2 cubic charstrings); `cmap` (codepoint→glyph, formats 4 + 12); metrics (`head` unitsPerEm/bbox, `hmtx` advance/LSB, `hhea` ascender/descender/lineGap); kerning (`kern` and/or `GPOS`).

| Option | Outlines | CFF/OTF-PS | Deps | Unsafe | Alloc | Effort | In-house purity |
|--------|----------|------------|------|--------|-------|--------|-----------------|
| **A — in-house `glyf`-only TTF parser** | TrueType quad + composite | **NO** (CFF = ~+1–1.5k LOC charstring VM) | none | yours | scratch only | ~1.5–2.5k LOC / days | **maximal** |
| **B — `ttf-parser` behind a `FontFace` adapter** | `glyf` + CFF + AAT | **yes** | **none** (zero transitive) | **`#![forbid(unsafe_code)]`** | **zero** | ~hours | high (zero-dep, load-time, unsafe-free) |
| **C — `fontdue`** | via ttf-parser + raster | yes | ttf-parser + alloc | safe | yes (allocates) | low | low (bundles an unwanted CPU rasterizer) |
| **D — `ab_glyph`/`rusttype`** | outline + raster | yes | several | mixed | yes | low | low (no advantage over B) |

**C and D are disqualified outright** for P5b: both bundle a CPU bitmap rasterizer the engine does not want (we generate MSDF, not bitmaps — the owner rejected bitmap), and both add allocation surface. The real fork is **A vs B**.

### My recommendation: **Option B — `ttf-parser` behind a thin in-house `FontFace` trait** (the T0 deliverable), with A as a defensible owner override.

Reasoning, grounded in the engine's own stated philosophy and precedents:
1. **Load-time only — the engine hot path NEVER calls it.** Extraction feeds the T2a generator at bake/load and is discarded. It cannot violate Principle 0/1 on the *runtime* path (there is no runtime path through it). This is categorically unlike the in-house-physics / in-house-RHI decisions, which were *hot-path, perf-load-bearing* — there the in-house work bought measurable perf. Here it buys **zero perf**.
2. **`ttf-parser` is the closest any external crate gets to "as if in-house"**: **zero dependencies** (no transitive tree to audit), **`#![forbid(unsafe_code)]`** (no `unsafe` surface added — relevant given the engine's Miri-TB soundness discipline), **zero heap allocations**, `no_std`, MIT/Apache. It is effectively an unsafe-free reference implementation of Option A.
3. **Correctness long-tail**: an in-house `glyf`-only parser **silently fails on CFF/OTF-PS fonts** (most Adobe fonts, many Google Fonts) until a charstring VM is written. `ttf-parser` handles CFF/CFF2/AAT and the real-world cmap/composite/variable long tail.
4. **The `FontFace` trait wall keeps the engine API in-house** and leaves the door open to swap in a future in-house parser with **zero call-site churn** — the dependency-isolation pattern the engine already uses for its RHI.

**Option A is a legitimate owner override** (the kind of VALUES/SCOPE call the owner reserves — cf. fully-in-house physics): write the `glyf`-only TTF parser and **document that PostScript/CFF `.otf` fonts are unsupported until a charstring VM is added**. It buys *purity only* (zero perf) and carries real correctness tail-risk. If the owner takes A: T0 ships the in-house parser behind the *same* `FontFace` trait, T1 is unchanged, the **cubic pseudo-distance path (T2a) is unreachable** (no CFF cubics — the cubic golden becomes N/A and the rung asserts no cubic segments are produced), and **"CFF/OTF-PS unsupported until a charstring-VM rung" is a tracked entry in §Out of scope** (critic's request — visible at the roadmap level, not only inline).

**Either way, T0 delivers the in-house `FontFace` trait** — the engine code depends on the trait, not on the chosen backend — **plus the checked-in golden fixtures**: a concrete `.ttf` (TrueType, e.g. a libre DejaVu/Roboto subset) for T0/T1/T2 goldens, AND a `.otf` (CFF, e.g. a libre Source-family subset) that gates the cubic path under Option B (or the documented-unsupported seam under Option A).

```rust
/// In-house font-extraction surface. The engine depends on THIS, not on the backend.
/// Load-time only — never on the render hot path. The chosen backend (ttf-parser,
/// recommended; or an in-house glyf parser, owner override) implements it.
pub trait FontFace {
    fn units_per_em(&self) -> u16;
    fn ascender(&self) -> i16;
    fn descender(&self) -> i16;
    fn line_gap(&self) -> i16;
    fn glyph_index(&self, cp: char) -> Option<GlyphId>;
    fn advance(&self, g: GlyphId) -> u16;
    fn left_side_bearing(&self, g: GlyphId) -> i16;
    fn outline(&self, g: GlyphId, sink: &mut dyn OutlineSink) -> Option<BBox>;
    fn kerning(&self, left: GlyphId, right: GlyphId) -> i16;
}
/// move_to/line_to/quad_to/curve_to/close — the T2a generator's edge collector
/// implements this directly (no intermediate Vec where avoidable).
pub trait OutlineSink { /* ... */ }
```

---

## Multithreading model

- **Bake (T1–T3)**: per-texel work is embarrassingly parallel — dispatched on the **engine threadpool** (Phase-9 Chase-Lev work-stealing). Each texel is an independent work item reading the shared (immutable during generation) edge list + the seeded coloring (pinned, §Decision T2-E), writing its own disjoint output texel. The T2b scanline pass and T2c error-correction pass also partition disjointly (per-scanline / per-texel). No shared mutable state → data-race-free by partitioning (no atomics; output texels disjoint). Load-time only.
- **Runtime emit (T5)**: the glyph emitter runs in the **same host-driven `host_upload_frame` path** as the P5a rect pack (§Decision T5-B). Single-threaded GPU touch: `RhiContext` (`!Send + !Sync`), atlas + bind-groups touched only on the dispatcher thread. Glyph `UiNode`s are appended to the **same reused scratch** the rects use (sized to the `UiTextBuffer::CAP` bound, §Decision T5-A), then the **same single sort + memcpy + one draw**. No new sync, no atomics, no `Mutex`/`RwLock`/`RefCell` (Principle 4).
- **Atlas lifecycle**: upload-once at setup (one staged `TRANSFER_DST` copy → barrier to `SHADER_READ_ONLY_OPTIMAL`; the binding-2 UBO written once), then immutable; all FIF sample concurrently with **no per-frame barrier** (`SampledComposite`). The atlas outlives every submission (the `BindGroupEntry` caller contract). `UiAtlas` is `!Send`-owned inside `UiRenderResources` (itself owned by `!Send RhiContext`), Drop-wired.
- **Send/Sync**: `UiText`/`FontId`/`GlyphMetrics`/`AtlasMeta` are POD `Send+Sync` (ECS columns / Resource). `FontTable` is `Send+Sync` (CPU data). `UiAtlas` is `!Send` (backend handles), correctly owned by `!Send UiRenderResources`. **Data-race freedom**: no datum reachable from two threads at runtime; bake parallelism is disjoint-output partitioning. ∎

---

## Integration

- **`boyko_ui`**: new `UiText`/`FontId`/`TextAlign` components (`text/components.rs`); reuse `UiTextBuffer` (content, CAP=247 — verified components.rs:79) + `ContentSize` (measure seam, verified components.rs:144-157 / layout.rs:545-551 as the leaf intrinsic-size fallback when `relative_count == 0`); new `ui_text_measure_system` (writes `ContentSize` set-if-changed, `Changed<UiTextBuffer>|Changed<UiText>`-gated, **scheduled before the layout system** so the same-frame relayout sees the new `ContentSize` — verified layout.rs:89 lists `Changed<ContentSize>` as a relayout trigger) and `ui_text_emit` (folds glyphs into the `UiNode` stream); the `FontTable` Resource; `parse_ui_text` `.ui` arm + `ui!` lowering.
- **`boyko_render`**: extend `UiInstance` (the `corner_radius`→`uv` alias under `FLAG_TEXT`, a new `FLAG_TEXT` bit — re-assert the std430 offset oracle; size stays 64 B); the MSDF text FS branch + the shared-VS `local_uv` varying + recompiled `.spv` (new `SpirvBlob<N>` byte-length is a compile-time check); extend the `UiRenderResources` bind-group layout with **binding 1** (`CombinedImageSampler` @ FRAGMENT) + **binding 2** (`UniformBuffer` @ FRAGMENT) + the `UiAtlas` (texture+sampler+UBO) owner **on `UiRenderResources`** so `create_slot`/`grow_slot` write all three bind-group entries (Decision T4-C); `ui_setup` gains the atlas upload (staged copy + barrier + UBO write); `UiRenderResources::create`/`destroy`/`create_slot`/`grow_slot` extended; the glyph emitter feeds the existing `pack_sort_upload`.
- **`boyko_rhi`/`boyko_rhi_vulkan`**: **one owned delta** — add `SamplerDesc.mip: MipMode` (Decision T4-D) + its Vulkan `create_sampler` mapping (`MipMode::None` → no mipmapping, maxLod 0). No push-constant/pipeline change (the fragment uniform rides the binding-2 UBO, Decision T4-A). The `UiPass`/present carry the atlas + UBO bind-group (part of the re-resolved UI bind-group, so existing MF-7 `ui_handles` re-resolution covers it — same per-FIF bind-group as the storage buffer).
- **New crate `boyko_fontbake`** (load-time tool, off the engine hot path): `face` (T0) / `extract` (T1) / `msdf` (T2a/b/c) / `atlas` (T3) / the `FontFace` trait + chosen backend. Depends on the threadpool for parallel generation; transient `Vec` scratch is the documented Principle-0 load-time exception. Produces `.bfont`; the engine loads `.bfont` (a thin POD reader, no bake deps at runtime).
- **`boyko_demo`**: a dogfood HUD label (T6) reading `Health` via the existing `BindText`.

### Decision T5-A: emit/measure scratch sizing — bounded by `UiTextBuffer::CAP` (the alloc-free fix)

The critic correctly flagged that "zero steady-state allocation" was unreconciled with the hard content bound. `UiTextBuffer::CAP = 247` bytes (verified components.rs:79), so a single text node holds **≤247 glyphs** (ASCII; fewer for multi-byte UTF-8 — each emitted glyph consumes ≥1 source byte, so glyph-count ≤ byte-count ≤ 247). The reused emit/measure scratch is sized **once at setup** to the worst case `247 glyphs × FRAMES_IN_FLIGHT` (per-node), or to a setup-time policy cap across all visible text nodes for the shared instance scratch (the same grow-only ring discipline the rects already use — it grows pow2 on overflow at setup-class cost, never per-frame). **Wrap → multi-line** is bounded too: one 247-byte buffer produces at most 247 glyphs across however many lines wrap creates (lines partition the same source bytes), so line-count adds no glyphs. The counting-allocator bench (§Metrics) asserts the emit/measure path **never reallocs in steady state**; a grow-on-overflow only fires when total visible glyphs exceed the ring, which is the same pow2 setup-class grow P5a rects already pay (not a per-frame alloc).

### Decision T5-B: emit driver — ride the host-driven `host_upload_frame` (matches P5a's shipped reality)

P5a ships the host-driven `host_upload_frame` path (the in-schedule dispatcher-solo upload-system world-projection is a documented P5a Rung-4 architectural gap, deferred). T5 rides the **host-driven path** to unblock the T6 dogfood, with the in-schedule path as the **same future seam P5a already flagged** (no new seam introduced). The glyph emit folds into the existing rect pack→sort→memcpy→draw.

---

## Implementation plan (for the developer)

1. **T0** — `boyko_fontbake/src/face.rs`: the `FontFace`/`OutlineSink` traits + the chosen backend (ttf-parser adapter, or in-house glyf parser per the owner fork). Check in the `.ttf` + `.otf` golden fixtures. Unit: extract the checked-in TTF.
2. **T1** — `boyko_fontbake/src/extract.rs`: glyph outline (em-normalized line/quad/cubic) + per-glyph + face metrics via `FontFace`. Unit: segment/winding goldens.
3. **T2a** — `boyko_fontbake/src/msdf/color.rs` + `distance.rs`: edge-coloring (seeded switch, corrected max-angle corner predicate, 0/1/N-corner) + per-channel signed pseudo-distance (line closed-form, quad Cardano, cubic multi-seed Newton) + raw winding + the pinned-pixels-per-em range mapping; threadpool per-texel dispatch. CPU goldens: `A`/`o`/`.`/smooth-`O`/teardrop + cubic-vs-brute-force (Option B).
4. **T2b** — `boyko_fontbake/src/msdf/sign.rs`: overlap-combiner preprocessing + scanline sign-correction overriding the pseudo-distance sign. CPU golden: `8`/`é` zero inverted interior.
5. **T2c** — `boyko_fontbake/src/msdf/error_correct.rs`: the analytic-bilinear error-correction pass (collapse offending texels to single-channel). CPU golden: thin-waist `e`/`g` clean post-correction + pre-correction control proving the speckle existed.
6. **T3** — `boyko_fontbake/src/atlas.rs`: skyline packer (expanded transition-band quad, spacing ≥ pxrange/2) + `GlyphMetrics`/`AtlasMeta`(distance_range_texels+pixels_per_em+kind)/`cmap`/`kern` + `.bfont` writer; `boyko_render` `.bfont` reader. Unit: packing/round-trip/UV/no-scale-drift.
7. **T4a** — `boyko_render/src/ui/instance.rs`: add `FLAG_TEXT`, document the `corner_radius`→`uv` alias, re-assert offsets (64 B). `boyko_render/src/ui/resources.rs`: add binding 1 (`CombinedImageSampler` @ FRAGMENT) + binding 2 (`UniformBuffer` @ FRAGMENT) to the layout; add the `UiAtlas` field (texture+sampler+UBO); extend `create`/`create_slot`/`grow_slot`/`destroy` to write all three entries + the atlas upload (staged copy + barrier + UBO write). `boyko_rhi`: add `SamplerDesc.mip: MipMode` + backend mapping (Decision T4-D). `boyko_render/shaders/ui_rect.vs.hlsl`: forward `corner` as `local_uv` for both branches; pin the new varying + `FLAG_TEXT` in the oracle block.
8. **T4b** — `boyko_render/shaders/ui_rect.fs.hlsl`: the `FLAG_TEXT` median branch (`median`+`screenPxRange` NaN-floored, premultiplied; read `pxRange`/`atlasSize` from the binding-2 UBO); dxc → `.spv`, `spirv-val` clean, new `SpirvBlob<N>`. GPU goldens G-T4.1–.8 against the checked-in `.bfont`.
9. **T5a** — `boyko_ui/src/text/layout.rs`: Latin shaping (advance/kerning/wrap/baseline) → glyph quads as `UiNode`s into the existing stream; the `ui_text_emit` integration; scratch sized per Decision T5-A.
10. **T5b** — `boyko_ui/src/text/measure.rs`: `ui_text_measure_system` writing `ContentSize` set-if-changed, change-gated, **scheduled before layout**. Unit + the layout-hug golden; GPU goldens G-T5.1–.4.
11. **T6** — `boyko_ui/src/text/dispatch.rs`: `"UiText" =>` arm + `parse_ui_text`; `ui!` `UiText{...}` lowering; `boyko_demo` HUD label dogfood. Integration goldens.

---

## Metrics and validation

- **Unit**: `UiInstance` offset oracle (text lane alias, 64 B) + the new `local_uv` varying + `FLAG_TEXT` in the same oracle block; `GlyphMetrics`/`UiText`/`UiAtlasUniform`(16 B) size const-asserts; outline segments (T1); MSDF per-channel distance vs brute-force + median reconstruct + corner + smooth-`O` + teardrop (T2a); cubic-vs-brute-force on a CFF glyph (T2a, Option B); overlap zero-inverted-interior (T2b); error-correction clean + pre-correction control (T2c); skyline non-overlap + expanded-quad + **no per-glyph scale drift** + `.bfont` round-trip + cmap/kern binary search (T3); measure→`ContentSize` + kerning/wrap arithmetic + scratch-never-reallocs (T5).
- **Property**: MSDF generation never produces NaN/Inf in a field texel; the cmap is a permutation of the input codepoints; shaping pen advance is monotonic; emit never produces NaN `UiInstance` (finite-assert); MTSDF `.a` ≈ `median(rgb)` ≈ 0.5 at the true edge within tolerance (Decision T2-F).
- **GPU goldens**: 
  - **G-T4.1** crisp single glyph (interior=fg, exterior=bg, premultiplied).
  - **G-T4.2** median preserves a corner vs a single-channel-SDF control (the defining MSDF property).
  - **G-T4.3** multi-scale crispness on **TWO distinct-footprint glyphs** at 16/48/96 px (AA band ~1–2 device px each — proves `screenPxRange` + the binding-2 pxRange + no per-glyph drift).
  - **G-T4.4** atlas UV (two distinct glyphs render distinctly — proves the UV lane + alias offset).
  - **G-T4.5** single `draw(6,N,0,0)` (the counting `RhiCommandEncoder`, P5a G7 mechanism — text+rects = one draw).
  - **G-T4.6** NaN-safe flat field (solid fg, no NaN black hole).
  - **G-T4.7** grow re-bind (overflow N, then sample correctly on the grown slot — proves Decision T4-C).
  - **G-T4.8** minified-scale sample, no median corruption (proves the no-mip sampler, Decision T4-D).
  - **G-T5.1–.4** pen positions / kerning tighter / word-wrap at whitespace / multiline baseline.
  - **T6 integration**: `.ui` ≡ `ui!` text node archetypes; a `BindText`→`UiTextBuffer`→`UiText` label renders the live `Health` value, re-emits only on `Changed<UiTextBuffer>` (0% when static); the demo HUD crisp end-to-end.
- **Benchmarks** (criterion, `bench.ps1` median-of-N): 
  - glyph emit+sort+memcpy throughput at N=100/1k/5k glyphs, **counting allocator asserts ZERO steady-state allocation** (Decision T5-A bound).
  - static-frame change gate ≈ O(1).
  - **M5 (the Principle-1 gate, critic-added): an A/B rect-only bench BEFORE vs AFTER the `FLAG_TEXT` branch + atlas/UBO bindings land** (same N rects, no glyphs) — asserts the rect path is within noise (the `FLAG_TEXT` branch must be free for the rect majority, not just cheap for glyphs; the always-present-but-unsampled atlas binding must be free for rect instances). Text-fragment cost is measured **on the shared draw**, not in isolation.
  - MSDF bake time per glyph (load-time, informational).
- **`debug_assert!`**: `FLAG_TEXT` instances have finite `uv` ⊂ `[0,1]`; non-text instances never read the `uv` alias; `pxRange ≥ 1`; atlas in `ShaderReadOnlyOptimal` before first sample; `glyph_slot < glyphs.len()`; emit never splits a UTF-8 char; `FontId.0 < fonts.len()`; the per-atlas UBO is written before the first sample; the no-mip sampler is `MipMode::None`.
- **Miri-TB**: the atlas upload + bind-group(×3 entries) + the per-atlas UBO write + the `UiInstance` byte-view extensions (the P5a/Phase-14a lesson — Miri-TB caught soundness bugs multiple review rounds approved). The `corner_radius`→`uv` reinterpret is a *value* reinterpret in the shader (no Rust transmute), so Rust-side it stays a normal `[f32;4]` field; the `slice_as_bytes` view still gets Miri.

### Decision T2-E: bake determinism (the golden-stability fix)

The edge-coloring seed is a **pinned bake-config constant**, and the CPU reference goldens are generated with that seed **on the scalar (non-SIMD) reference path** with a defined float-eval order. The byte-compare tolerance band is sized to absorb **only last-ULP float differences**, not algorithmic ones — so a real regression cannot hide under the tolerance (relevant given the GNU/codegen toolchain note: cross-machine float-eval drift is a real flakiness risk). Documented in the `.bfont` header (seed + generator version).

### Decision T2-F: MTSDF `.a` shares the rgb mapping (the effects-alignment fix)

When `kind == Mtsdf`, the `.a` true-SDF channel uses the **identical `range_em` mapping and 0.5 zero-crossing** as the rgb channels (same `pixels_per_em`, same `distance_range_texels`), so at the true edge `median(rgb) ≈ .a ≈ 0.5`. A unit check asserts this within tolerance so P7 outline/glow effects key off the **same** edge as the fill (otherwise the effect isovalue drifts from the fill edge).

### Decision T0-D: atlas kind (MTSDF range-6 vs MSDF range-4) — an owner VALUES call tied to P7

MTSDF range-6 (RGBA8, 4 channels) is **4× the atlas memory** of R8 SDF and bakes a `.a` channel the **P5b fill path never samples** (the effects branch is P7). Baking MTSDF now is justified **only if P7 diegetic HUD is committed** (bake once, `.a` is the seam). If P7 is speculative, **default to MSDF range-4** (smaller, fill-only matches the +1-sample budget) and re-bake when P7 lands — the `.bfont` `AtlasMeta.kind` field carries the kind, so the swap is **data-only** (no code change). This is surfaced as an **explicit owner decision** (not a leaning), correctly routed as VALUES/SCOPE because it hinges on whether P7 ships, not on a perf calculation.

---

## Out of scope (seams)

- **Complex-script shaping** (HarfBuzz: bidi, ligatures, contextual forms, CJK) — Latin-first; the `FontFace`/emit seam is where a future shaper plugs in.
- **RTL/BiDi**, **color emoji** (COLR/CBDT), **subpixel/LCD AA**, **rich-text runs** (per-span font/color/size), **text selection/editing/IME**.
- **Multi-font / multi-atlas in one draw** — P5b ships a single resident atlas/font (Decision T4-E); the named seam is per-instance pxRange/atlasSize lanes (or a texture-array atlas), with `AtlasMeta.kind` + `.bfont` already carrying per-font metadata.
- **CFF/OTF-PS support** — **if Option A (in-house `glyf`-only parser) is taken**, PostScript/CFF `.otf` fonts are **unsupported until a charstring-VM rung** (a tracked roadmap seam, gated by the checked-in `.otf` fixture; the cubic pseudo-distance path is unreachable until then). **If Option B (`ttf-parser`) is taken**, CFF/CFF2 is supported from T0 and this is not a gap.
- **Dynamic runtime glyph atlas** (CJK / arbitrary user fonts via runtime MSDF generation into an incremental skyline atlas) — T2/T3 are written incremental-friendly (the skyline packer + per-glyph generation are the seam); P5b ships a **precomputed Latin `.bfont`** (matches "preallocate at setup").
- **World-space / diegetic text** — **P7**: projects the text root to screen, then rides **this exact instanced path** (optional depth-test); MTSDF `.a` (Decision T2-F) enables outline/glow effects. `FLAG_TEXT` + atlas + MTSDF `.a` are the seams.
- **MTSDF effects** (outline/glow/drop-shadow via `.a`) — the atlas is baked MTSDF (range 6) iff Decision T0-D selects it; the effect FS branch is a small P7 follow-up.

---

## Open questions (for the owner — all genuinely VALUES/SCOPE, no technical forks left)

1. **The font-parsing fork** (§ above) — `ttf-parser` behind `FontFace` (recommended) vs in-house `glyf`-only (owner override, no CFF). **Owner VALUES decision before T1.** (All technical consequences are now pinned for both arms — cubic path, fixtures, the Out-of-scope seam.)
2. **Atlas kind** (Decision T0-D) — **MTSDF range-6 iff P7 diegetic HUD is committed; else MSDF range-4 with a data-only re-bake when P7 lands.** Owner decision hinging on P7 commitment.

(The three originally-open *technical* questions — UV-lane aliasing, the fragment-uniform delivery channel, and the emit driver — are now **decided** in Decisions T4-G, T4-A, and T5-B respectively, with the textured-rect alias-retirement trigger recorded.)

---

## Changes from review

**Critical fixes (all three resolved):**

- **C1 — MSDF error-correction pass was entirely absent.** Added **T2c** as a named, independently-gated rung: the msdfgen analytic-bilinear error-correction pass (predict bilinear interpolation toward each neighbor; collapse texels where the median would introduce a spurious edge to single-channel). Added the algorithm description (§T2c), the CPU golden (thin-waist `e`/`g` clean post-correction + a **pre-correction control proving the speckle existed**), and the implementation step (`msdf/error_correct.rs`).
- **C2 — sign correctness rested on bare winding.** Split out **T2b**: (1) overlap/self-intersection **preprocessing** (msdfgen overlapping-contour distance combiner) and (2) a **scanline sign-correction pass** that re-derives authoritative inside/outside and overrides the pseudo-distance sign. **Dropped the unsupported "self-intersections handled" claim**; the raw winding in T2a is now explicitly labeled provisional, overridden by T2b. Added the `8`/`é` zero-inverted-interior CPU golden.
- **C3 (two critics) — range/pxrange conflation + push-constant delivery (the keystone).** (a) Pinned the **global uniform `pixels_per_em`** binding: `range_em = distance_range_texels / pixels_per_em`; both carried in `AtlasMeta`; added the no-per-glyph-scale-drift unit gate and made **G-T4.3 run on two distinct-footprint glyphs**. (b) **Retracted the false "no new RHI surface"** for fragment-uniform delivery: replaced the impossible FRAGMENT push constant with a **per-atlas `UniformBuffer` at binding 2** (Decision T4-A) — verified the push range is hard-coded VERTEX-only (rhi_impl.rs:1332) with no stage knob; the UBO needs no push/pipeline change.

**Major fixes:**

- **Edge-coloring corner predicate corrected** to match `edge-coloring.h` (max-angle cutoff, `crossThreshold = sin(maxAngle)`, polarity fixed); added the **seeded switch** and **explicit 0/1/N-corner handling** with goldens (smooth-`O`, teardrop).
- **Cubic Bézier solver specified** concretely (fixed multi-seed Newton + endpoints + global min) with a CFF-glyph brute-force golden; tied to the font-fork (unreachable/N-A under Option A).
- **Atlas padding → expanded transition-band quad**: the field is generated over `bbox + distance_range_texels/2 + ε`; **both planeBounds and atlasBounds describe the expanded quad** so the AA band is never clipped; inter-glyph spacing is the separate ≥ pxrange/2 rule.
- **Per-atlas-uniform delivery committed to ONE channel** (binding-2 UBO, Decision T4-A) — no "or".
- **Multi-font contract committed** (Decision T4-E): P5b ships a single resident atlas/font; multi-font is a named seam (per-instance lanes or texture-array). Resolves the "interleave in one draw breaks with >1 atlas" finding.
- **VS `local_uv` varying made explicit** (Decision T4-B): forward `corner` for both branches, pinned in the offset oracle block; no Rust-struct change.
- **Grow-path atlas re-bind closed** (Decision T4-C): `UiAtlas` (+UBO) owned by **`UiRenderResources`** (not `RhiContext`) so `create_slot`/`grow_slot` write all three bind-group entries; added G-T4.7 grow re-bind golden. Verified the grow rebuild path (resources.rs:344-402).
- **No-mip sampler grounded** (Decision T4-D): added `SamplerDesc.mip: MipMode` (a small owned RHI delta) since the current desc (device.rs:62-71) has no mip field; backend maps `None`→maxLod 0; added G-T4.8 minified-no-corruption golden.
- **Emit/measure scratch bounded** (Decision T5-A): sized at setup to the verified `UiTextBuffer::CAP = 247` glyph bound (components.rs:79); wrap/multi-line bounded by the same source bytes; counting-allocator bench asserts no steady-state realloc.

**Minor fixes:**

- **MTSDF `.a` shares the rgb 0.5 zero-crossing/mapping** (Decision T2-F) with a unit check, so P7 effects key off the fill edge.
- **Bake determinism** (Decision T2-E): pinned coloring seed + scalar reference path + ULP-only tolerance, documented in the `.bfont` header.
- **A/B rect-only regression bench** (Metrics M5): proves the `FLAG_TEXT` branch + atlas binding are free for the rect majority on the shared draw.
- **`corner_radius`→`uv` alias-retirement trigger recorded** (textured/image-fill/nine-slice rect) so the 64 B decision is revisited deliberately.
- **Measure scheduled before layout** made explicit (verified layout.rs:89 `Changed<ContentSize>` relayout trigger + layout.rs:545-551 leaf seam).
- **Checked-in `.ttf` + `.otf` golden fixtures** named in T0; **CFF-unsupported-under-Option-A promoted to a tracked §Out-of-scope seam** (not only inline).
- **Atlas kind surfaced as an explicit owner VALUES call** (Decision T0-D) tied to P7 commitment, data-only re-bake via `AtlasMeta.kind`.

**Preserved verbatim (critic-affirmed strengths):** the entire font-parsing dependency fork section (A-vs-B table, load-time-only framing, `FontFace` trait wall, Option-B recommendation with Option-A as a documented owner override behind the same trait); the ECS-native data model (dense `FontId`, sorted-slice cmap/kern, ECS/`RhiContext`-resident atlas); the `ContentSize`→`Auto` chain; the corner_radius→uv alias soundness for P5b; FULL MSDF with shader `median` + corner preservation; the single-pipeline/single-z-sort/single-draw P5a render contract.