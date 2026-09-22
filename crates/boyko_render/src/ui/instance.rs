//! The std430 per-instance UI quad record (`UiInstance`) + the ortho push block
//! (`UiOrtho`) the GUI P5a render path uploads to the GPU.
//!
//! `UiInstance` is the POD record the shader's `StructuredBuffer<UiInstance>` reads
//! by `SV_InstanceID` (proven readable in BOTH the vertex and fragment stages off a
//! single VERTEX|FRAGMENT-visible descriptor by the Rung-0.5 GPU golden). It is
//! PACK-SCRATCH ONLY (GUI P5a Decision 6): the upload system materializes a reused
//! `Vec<UiInstance>`, stable-sorts it by `StackIndex`, and bulk-memcpys it into the
//! mapped per-frame ring slot — it is NOT an ECS column and NOT a per-chunk
//! `cast_slice` (the global z-sort forbids a per-chunk blit).

/// One instanced UI quad on the GPU — the std430 record the shader reads by
/// `SV_InstanceID`. `#[repr(C, align(16))]` POD, all-f32 (no f16 / no u8 packing,
/// so no 16-bit-storage device feature is needed). Physical px (the `scale_factor`
/// is folded at pack); PREMULTIPLIED color (authors write straight RGBA8 in
/// `UiBackground`; premultiplied at pack).
///
/// # std430 layout contract (the compile-time oracle)
///
/// Field order places the two `float2`s first (off 0, 8 — so the first `float4`
/// lands on a 16 B boundary at off 16), then the three `float4`s (off 16, 32, 48),
/// then four scalars (off 64..76). The total stride is **80 B** (UI-ADVANCED S2 /
/// architecture D1 — widened ONCE from 64 B when the `uv` field retired the
/// `corner_radius` text-lane alias), a multiple of 16, so the std430 array stride
/// is legal with NO internal padding and NO tail pad. The HLSL `struct UiInstance`
/// mirrors these offsets; the per-field `offset_of!` const-asserts below are the
/// build-time oracle that catches a Rust↔HLSL offset drift the size assert alone
/// would miss.
///
/// `align(16)` is forced explicitly so the Rust struct's alignment matches the
/// std430 array's 16 B stride alignment (a `[f32; 4]` field is only 4-aligned in
/// Rust, unlike an HLSL `float4`, so without `align(16)` the whole struct would be
/// 4-aligned — the per-field byte offsets stay identical either way, but the
/// explicit align documents and pins the array-stride alignment).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct UiInstance {
    /// Top-left corner, physical px.
    pub min_px: [f32; 2],
    /// Width + height, physical px.
    pub size_px: [f32; 2],
    /// Clip AABB `min.xy, max.xy`, physical px (valid iff `CLIP_PRESENT`). Shared by
    /// rect AND text nodes (text clips too).
    pub clip: [f32; 4],
    /// Per-corner radius `tl, tr, br, bl`, physical px — **ALWAYS the radius**.
    ///
    /// # The text-lane alias is RETIRED (UI-ADVANCED S2 / architecture D1)
    ///
    /// Until S2 this field was REINTERPRETED as the glyph UV under [`FLAG_TEXT`]
    /// (GUI P5b Decision T4-G), which kept the record at 64 B but forbade a node
    /// carrying BOTH a radius and a UV — the exact case sprites trigger (a rounded
    /// avatar, a nine-slice chip). The recorded deliberate-revisit fired: the UV now
    /// lives in its own [`uv`](UiInstance::uv) field, this field means ONE thing, and
    /// a `FLAG_TEXT` instance packs it ZERO (gate G2-5).
    pub corner_radius: [f32; 4],
    /// Normalized UV rect `(u0, v0, u1, v1)` in `[0, 1]` — glyphs AND (from S3)
    /// sprites. Written verbatim at pack (never scale-folded). A plain rect packs the
    /// identity `(0, 0, 1, 1)` and its shader branch never reads it (S-D8: the
    /// widening is default-OFF — every existing image is byte-identical).
    pub uv: [f32; 4],
    /// PREMULTIPLIED RGBA8 fill (`byte0=R .. byte3=A`).
    pub color: u32,
    /// PREMULTIPLIED-at-pack RGBA8 border color.
    pub border_color: u32,
    /// Uniform border width, physical px (P5a is uniform; per-side is deferred).
    pub border_width: f32,
    /// Bit flags under the S-D2 bit budget (fixed at S2, once — and **SPENT OUT at S5**):
    /// bit0 [`FLAG_BORDER_ANY`], bit1 [`FLAG_CLIP_PRESENT`], bit2 [`FLAG_TEXT`],
    /// bit3 [`FLAG_TEXTURED`] (S3's sprite lane), bit4 reserved for S7's deferred
    /// per-sprite sampler index, bit5 [`FLAG_TILED`] (S5's tiled nine-slice lane),
    /// bits [`UI_TILE_X_SHIFT`]`..=12` / [`UI_TILE_Y_SHIFT`]`..=19` the two
    /// [`UI_TILE_BITS`]-bit repeat counts, bits
    /// [`UI_SLOT_SHIFT`]`..32` the bindless slot — [`UI_SLOT_BITS`] bits, slots
    /// `0..=`[`UI_SLOT_MASK`], EXACTLY the table's range (the capacity
    /// const-assert below).
    ///
    /// **The budget is EXHAUSTED**: S-D2 left bits 5..19 free — fifteen — and S5's flag
    /// plus its two 7-bit fields are fifteen. Only bit 4 remains, and it is S7's. The next
    /// per-instance datum widens the record instead of taking a bit; the animation and
    /// interaction plans read §6's exposure row to learn that.
    ///
    /// At S2 every packed instance had bits 3..31 zero. **S3 moved that gate,
    /// deliberately**: an UNTEXTURED instance still has bits 3..31 zero, and a
    /// `FLAG_TEXTURED` one has bit 3 set plus its slot in the top 12. **S5 moves it
    /// again, and only for one lane**: a TILED nine-slice sub-quad additionally carries
    /// bit 5 and its two count fields. Every other record — background, glyph, whole-rect
    /// sprite, `Stretch` slice, and a `Tile` slice whose own counts are both `1` (every
    /// corner) — still leaves bits 4..19 zero, which is what keeps the six committed image
    /// pins identical (gate G5-11 / the S3 G3-5 successor).
    pub flags: u32,
}

/// The byte size of [`UiInstance`] (the std430 array stride) — 80 B since the
/// UI-ADVANCED S2 widening (architecture D1; 64 B before it).
pub const UI_INSTANCE_SIZE: usize = 80;

/// `UiInstance.flags` bit0 — the rect has a (uniform, P5a) border to draw. Set at
/// pack when `border_width > 0`; gates the fragment shader's border branch.
pub const FLAG_BORDER_ANY: u32 = 1 << 0;
/// `UiInstance.flags` bit1 — the rect carries a finite clip AABB in `clip`. Set at
/// pack from a present `ComputedClip`; gates the fragment shader's clip branch (a
/// sentinel-free uniform branch — unclipped rects never evaluate `clip_coverage`).
pub const FLAG_CLIP_PRESENT: u32 = 1 << 1;
/// `UiInstance.flags` bit2 — the instance is a GLYPH quad, not a rounded rect (GUI
/// P5b Decision T4-G). When set, the fragment shader reads the glyph's normalized
/// atlas UV rect from [`UiInstance::uv`] (its own field since the S2 widening — the
/// `corner_radius` alias is retired) and samples the MSDF atlas (`median` +
/// `screenPxRange` AA, premultiplied out) instead of evaluating the rounded-box
/// SDF. A uniform-per-instance branch, so the rect majority is unregressed.
pub const FLAG_TEXT: u32 = 1 << 2;
/// `UiInstance.flags` bit3 — the instance is a SPRITE quad: the fragment shader
/// samples the bindless texture at the slot in bits
/// [`UI_SLOT_SHIFT`]`..32` through the UI's OWN sampler (set 0, binding 3 — S-D4)
/// and modulates it by the premultiplied tint in [`UiInstance::color`]
/// (UI-ADVANCED S3, architecture D2). Mutually exclusive with [`FLAG_TEXT`]: a
/// glyph and a sprite are different quads, emitted separately (D4's per-node
/// emission contract), never one record wearing both flags.
pub const FLAG_TEXTURED: u32 = 1 << 3;
/// `UiInstance.flags` bit5 — the instance is a TILED sprite quad: the fragment shader wraps
/// the quad corner `(`[`UI_TILE_X_SHIFT`]`, `[`UI_TILE_Y_SHIFT`]`)` times inside the
/// record's own UV sub-rect (`frac`) instead of stretching it across (`lerp`) — UI-ADVANCED
/// S5, S-D15.
///
/// Set at pack ONLY when at least one of the two repeat counts exceeds `1`, which is what
/// makes a corner sub-quad (always `1×1`) pack BYTE-IDENTICALLY to its `Stretch` record.
/// Implies [`FLAG_TEXTURED`]: it is only ever set on a nine-slice sub-quad, and those are
/// emitted only for a node carrying both `UiNineSlice` and `UiImage`.
///
/// **The wrap is of the quad PARAMETER, not of the UV**, so the sample can never leave the
/// sub-rect for any count — which is what makes `Tile` over a sprite-sheet frame correct by
/// construction rather than a forbidden pair (the hazard the retired S-D7 guarded).
pub const FLAG_TILED: u32 = 1 << 5;

/// The LOW bit of the X repeat-count field inside `UiInstance.flags` (S-D15): bits
/// `6..=12`, [`UI_TILE_BITS`] wide, holding `1..=`[`UI_TILE_MAX`] repeats ACROSS the
/// record's UV sub-rect. Zero when [`FLAG_TILED`] is clear.
pub const UI_TILE_X_SHIFT: u32 = 6;
/// The LOW bit of the Y repeat-count field inside `UiInstance.flags` (S-D15): bits
/// `13..=19`, repeats DOWN the sub-rect. See [`UI_TILE_X_SHIFT`].
pub const UI_TILE_Y_SHIFT: u32 = 13;
/// The WIDTH in bits of each repeat-count field (S-D15).
///
/// **7, not 8 and not 6**, and the reason is measured against the use: a UI chrome edge
/// tiles an 8–32 px source over up to ~1000 px, i.e. tens of repeats. 6 bits (63) could
/// clip a long scrollbar track; 7 bits (127) cannot, and 8 would not fit beside the flag in
/// the fifteen bits S-D2 left.
pub const UI_TILE_BITS: u32 = 7;
/// Each repeat-count field's mask AFTER its shift (`0x7F`). Derived from [`UI_TILE_BITS`],
/// never spelled — the emitted HLSL derives its own `UI_TILE_MASK` from the same generator
/// input, so the two cannot drift into a quad tiling a different number of times.
pub const UI_TILE_MASK: u32 = (1u32 << UI_TILE_BITS) - 1;
/// The largest repeat count a field can hold (`127`) — the `min` the pack clamps a derived
/// count into. Equal to [`UI_TILE_MASK`] because the counts are stored verbatim (a count of
/// `0` is unreachable: the flag is set only when a count exceeds `1`).
pub const UI_TILE_MAX: u32 = UI_TILE_MASK;

/// The LOW bit of the bindless-slot field inside `UiInstance.flags` (S-D2).
///
/// The field sits at the TOP of the word on purpose: it is the widest field and the
/// flags are what grow, so a new flag can never risk the slot (S-D2's rejected
/// alternative packed it at bits 3..14, adjacent to the flags).
pub const UI_SLOT_SHIFT: u32 = 20;
/// The WIDTH in bits of the bindless-slot field inside `UiInstance.flags` (S-D2) —
/// 12 bits, slots `0..=4095`, EXACTLY
/// [`BINDLESS_TEXTURE_CAPACITY`](boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY)'s
/// range with ZERO headroom (the const-assert below is what makes that safe).
pub const UI_SLOT_BITS: u32 = 12;
/// The bindless-slot field's mask AFTER the [`UI_SLOT_SHIFT`] right-shift
/// (`0xFFF`). Derived from [`UI_SLOT_BITS`], never spelled — the emitted HLSL
/// derives its own `UI_SLOT_MASK` from the same generator input, so the two cannot
/// drift into a quad sampling a different texture.
pub const UI_SLOT_MASK: u32 = (1u32 << UI_SLOT_BITS) - 1;

// S-D2 (UI-ADVANCED S2): the 12-bit bindless-slot field in `flags` bits 20..31 has
// EXACTLY zero headroom over the live table capacity — and D3 refuses a UI slot
// reservation, so "raise the capacity" is the natural response to slot pressure,
// and a raised capacity would SILENTLY truncate the field and make a UI quad
// sample a different texture. This assert is against the LIVE constant the
// allocator uses (mutation M2-c: a copy of it here would keep passing).
const _: () = assert!(
    boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY <= 1 << UI_SLOT_BITS,
    "UiInstance.flags carries the bindless slot in bits 20..31"
);
// The slot field ends exactly at the top of the word: a shift/width pair that
// overhung would silently drop the high slot bits on every sprite above 2047.
const _: () = assert!(
    UI_SLOT_SHIFT + UI_SLOT_BITS == 32,
    "the bindless-slot field must end at bit 31 (S-D2's bit budget)"
);
// Bit 4 is RESERVED for S7's deferred per-sprite sampler index, so the slot field must not
// reach down into it.
const _: () = assert!(
    FLAG_TEXTURED.trailing_zeros() < UI_SLOT_SHIFT - 1,
    "bit 4 stays reserved between the flags and the slot field"
);
// S-D15's bit budget, made mechanical. The three relations below say, together, that the
// flag and the two count fields EXACTLY fill S-D2's fifteen free bits (5..=19), do not
// overlap each other, do not collide with S7's reserved bit 4, and stop below the slot
// field. The budget is spent out; a rung that wants a bit widens the record instead, and
// this is the line that tells it so.
const _: () = assert!(
    FLAG_TILED.trailing_zeros() == FLAG_TEXTURED.trailing_zeros() + 2,
    "FLAG_TILED sits at bit 5, one above S7's reserved bit 4"
);
const _: () = assert!(
    UI_TILE_X_SHIFT == FLAG_TILED.trailing_zeros() + 1,
    "the X repeat-count field starts immediately above FLAG_TILED"
);
const _: () = assert!(
    UI_TILE_Y_SHIFT == UI_TILE_X_SHIFT + UI_TILE_BITS,
    "the two repeat-count fields are adjacent and do not overlap"
);
const _: () = assert!(
    UI_TILE_Y_SHIFT + UI_TILE_BITS == UI_SLOT_SHIFT,
    "the tile fields end exactly where the bindless-slot field begins — the S-D2 budget is \
     EXHAUSTED (fifteen free bits, fifteen spent)"
);

// --- std430 layout oracle (compile-time). The size/align pin the array stride;
//     the per-field `offset_of!` asserts pin every field's byte offset against the
//     HLSL `struct UiInstance` (catching a swapped/shifted field a size match
//     would not). ---
const _: () = assert!(size_of::<UiInstance>() == UI_INSTANCE_SIZE);
const _: () = assert!(align_of::<UiInstance>() == 16);
const _: () = assert!(core::mem::offset_of!(UiInstance, min_px) == 0);
const _: () = assert!(core::mem::offset_of!(UiInstance, size_px) == 8);
const _: () = assert!(core::mem::offset_of!(UiInstance, clip) == 16);
const _: () = assert!(core::mem::offset_of!(UiInstance, corner_radius) == 32);
const _: () = assert!(core::mem::offset_of!(UiInstance, uv) == 48);
const _: () = assert!(core::mem::offset_of!(UiInstance, color) == 64);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_color) == 68);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_width) == 72);
const _: () = assert!(core::mem::offset_of!(UiInstance, flags) == 76);

impl UiInstance {
    /// Re-views a packed `&[UiInstance]` as the contiguous `&[u8]` the upload
    /// memcpys into the mapped ring slot — the no-bytemuck POD view (GUI P5a uses a
    /// hand-rolled view, not a `Pod`/`cast_slice` dependency).
    ///
    /// The returned slice borrows `instances` and MUST NOT outlive it; the upload
    /// path uses it only for the immediate `memcpy`.
    #[inline]
    pub fn slice_as_bytes(instances: &[UiInstance]) -> &[u8] {
        // SAFETY: `UiInstance` is `#[repr(C, align(16))]` all-POD (f32/u32), with no
        // padding (const-asserted 80 B / 16-align / per-field offsets above), so the
        // byte image of `instances` is a valid initialized `[u8]` of exactly
        // `len * UI_INSTANCE_SIZE` bytes. The `&[UiInstance]` borrow keeps the
        // backing alive for the returned slice's lifetime; the slice is read-only.
        unsafe {
            core::slice::from_raw_parts(
                instances.as_ptr().cast::<u8>(),
                instances.len() * UI_INSTANCE_SIZE,
            )
        }
    }
}

/// The pixel→NDC ortho transform pushed as a VERTEX-stage push constant (16 B). A
/// mat-free vec2 multiply-add in the vertex shader: `ndc = pos_px * scale +
/// translate`.
///
/// `#[repr(C)]` POD (`scale` @0, `translate` @8, 16 B).
///
/// # Canonical convention (the ONE source of truth)
///
/// `scale = (2/w, +2/h)`, `translate = (-1, -1)`, so:
/// - `(0, 0)` (top-left pixel)     → NDC `(-1, -1)`
/// - `(w, h)` (bottom-right pixel) → NDC `(+1, +1)`
///
/// A POSITIVE y scale gives a top-left pixel origin in Vulkan's y-DOWN NDC. This is
/// the convention the code implements and the only one this type expresses.
///
/// # Plan deviation (recorded)
///
/// GUI P5a plan A2 specified the GL-style `scale = (2/w, **-2/h**)`,
/// `translate = (-1, **+1**)` (negative y, bottom-left-style). The **Rung-0.5 GPU
/// oracle overrides A2**: that GL formula lands pixel row 0 at the framebuffer
/// *bottom* on the in-house Vulkan path (`ssbo_graphics_probe`, RTX 3060,
/// validation clean). The positive-y convention above is therefore canonical; the
/// `-2/h, +1` form is the **rejected** one — do not reintroduce it.
///
/// The denominator (`w`, `h`) is the EXTENT THE UI PASS RENDERS INTO (the swapchain
/// `VkExtent2D`), per GUI P5a Decision 9 / A2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiOrtho {
    /// Per-axis NDC scale `(2/w, +2/h)` — positive y for the top-left origin.
    pub scale: [f32; 2],
    /// NDC translation `(-1, -1)` — maps the top-left pixel `(0,0)` to NDC `(-1,-1)`.
    pub translate: [f32; 2],
}

const _: () = assert!(size_of::<UiOrtho>() == 16);

impl UiOrtho {
    /// The pixel→NDC ortho for a `(width, height)` physical-px render target,
    /// top-left origin (Vulkan y-down NDC; positive y scale). `(0,0)` maps to NDC
    /// `(-1,-1)` (top-left texel) and `(width,height)` to `(+1,+1)` (bottom-right).
    ///
    /// `width`/`height` MUST be the extent of the image the UI pass renders into
    /// (the swapchain `VkExtent2D`), so a rect at the bottom-right corner lands at
    /// the bottom-right texel of that same image (Decision 9 contract).
    #[inline]
    pub fn for_extent(width: u32, height: u32) -> UiOrtho {
        debug_assert!(width > 0 && height > 0, "invariant: UI ortho extent is non-zero");
        UiOrtho {
            scale: [2.0 / width as f32, 2.0 / height as f32],
            translate: [-1.0, -1.0],
        }
    }

    /// Re-views this 16-byte POD as the `&[u8]` the draw recorder pushes as a
    /// VERTEX-stage push constant.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `UiOrtho` is `#[repr(C)]` POD (f32 only, 16 B, no padding —
        // const-asserted), so its byte image is a valid `[u8; 16]`; the `&self`
        // borrow keeps it alive for the returned slice; the slice is read-only.
        unsafe { core::slice::from_raw_parts((self as *const UiOrtho).cast::<u8>(), size_of::<UiOrtho>()) }
    }
}

/// Premultiplies a STRAIGHT RGBA8 color (`byte0=R .. byte3=A`) — author space — into
/// the PREMULTIPLIED RGBA8 the shader expects (`rgb *= a`), keeping the byte order.
///
/// Premultiplied alpha (RmlUi/WebRender) composes AA edges + nested clips correctly
/// under the engine's `src=ONE` blend, where straight alpha would fringe. Rounded
/// with `+ 127` before the `/ 255` divide so the conversion is symmetric.
#[inline]
pub const fn premultiply_rgba8(straight: u32) -> u32 {
    let r = straight & 0xFF;
    let g = (straight >> 8) & 0xFF;
    let b = (straight >> 16) & 0xFF;
    let a = (straight >> 24) & 0xFF;
    // (channel * a + 127) / 255 — rounded, exact for a == 255 (identity) and a == 0.
    let pr = (r * a + 127) / 255;
    let pg = (g * a + 127) / 255;
    let pb = (b * a + 127) / 255;
    pr | (pg << 8) | (pb << 16) | (a << 24)
}
