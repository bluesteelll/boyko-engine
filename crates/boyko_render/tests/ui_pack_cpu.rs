//! GUI P5a — CPU-side pack / ortho / z-sort / no-realloc unit + property coverage.
//!
//! These exercise the PURE half of P5a (Rungs 0–2 + the draw recorder's data carrier):
//! `pack_ui_instance`, `premultiply_rgba8`, `UiOrtho::for_extent`, the
//! `UiRenderScratch` stable z-sort, and the `UiInstance` byte view. They have NO GPU /
//! Arena / world dependency (the `PackInput` testable boundary), so they run under a
//! plain `cargo test` and under Miri.
//!
//! The end-to-end RECT GOLDEN (a rect landing at its ComputedRect on a real GPU image)
//! depends on the deferred Rungs 3–5 (`ui_setup` / `ui_upload` / swapchain wiring),
//! which do not exist in this FOUNDATION-ONLY commit — so the GPU image-diff goldens
//! are NOT writable here. The combination they would build on IS proven on the GPU by
//! the `ssbo_graphics_probe` (Rung 0.5) golden. These CPU tests pin the layout/pack/
//! ortho math that the deferred goldens will ultimately verify pixel-for-pixel.

use boyko_render::ui::{
    pack_ui_image_instance, pack_ui_instance, premultiply_rgba8, PackInput, UiImageInput,
    UiInstance, UiOrtho, UiRenderGeneration, UiRenderScratch, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT,
    FLAG_TEXT, FLAG_TEXTURED, UI_INSTANCE_SIZE, UI_SLOT_MASK, UI_SLOT_SHIFT,
};

// --- helpers ---------------------------------------------------------------

/// A baseline opaque-fill, no-border, no-clip, no-radius input at `(x,y,w,h)`.
fn plain_rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> PackInput {
    PackInput {
        rect: [x, y, w, h],
        color,
        border_color: 0,
        corner_radius: [0.0; 4],
        border_width: [0.0; 4],
        clip: None,
        text_uv: None,
        image: None,
        nine_slice: None,
    }
}

const OPAQUE_RED: u32 = 0xFF_00_00_FF; // byte0=R .. byte3=A
const OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

// --- premultiply -----------------------------------------------------------

#[test]
fn premultiply_opaque_is_identity() {
    // a == 255 ⇒ premultiply is a no-op (the (c*255 + 127)/255 rounds to c).
    assert_eq!(
        premultiply_rgba8(OPAQUE_RED),
        OPAQUE_RED,
        "opaque premultiply must be identity"
    );
    assert_eq!(
        premultiply_rgba8(OPAQUE_WHITE),
        OPAQUE_WHITE,
        "opaque white premultiply must be identity"
    );
}

#[test]
fn premultiply_zero_alpha_zeroes_rgb() {
    // a == 0 ⇒ every premultiplied color channel is 0, alpha stays 0.
    let straight = 0x00_AB_CD_EF & 0x00_FF_FF_FF; // a = 0
    let got = premultiply_rgba8(straight);
    assert_eq!(got, 0, "zero-alpha premultiply must zero rgb and keep a=0");
}

#[test]
fn premultiply_half_alpha_scales_rgb_rounded() {
    // R=255, G=255, B=255, A=128 ⇒ each channel = (255*128 + 127)/255 = 128.
    let straight = 0x80_FF_FF_FF; // a=0x80=128
    let got = premultiply_rgba8(straight);
    let r = got & 0xFF;
    let a = (got >> 24) & 0xFF;
    assert_eq!(r, 128, "premultiplied R at a=128 must round to 128, got {r}");
    assert_eq!(a, 128, "alpha must be preserved");
}

#[test]
fn premultiply_preserves_alpha_byte() {
    for a in 0u32..=255 {
        let straight = (a << 24) | 0x00_12_34_56;
        let got = premultiply_rgba8(straight);
        assert_eq!((got >> 24) & 0xFF, a, "alpha byte must be preserved for a={a}");
    }
}

// --- pack: scale folding ----------------------------------------------------

#[test]
fn pack_folds_scale_factor_into_lengths() {
    let input = plain_rect(10.0, 20.0, 30.0, 40.0, OPAQUE_RED);
    let inst = pack_ui_instance(&input, 2.0);
    assert_eq!(inst.min_px, [20.0, 40.0], "min_px must be logical * scale");
    assert_eq!(inst.size_px, [60.0, 80.0], "size_px must be logical * scale");
}

#[test]
fn pack_scale_one_is_passthrough_geometry() {
    let input = plain_rect(3.5, 7.25, 100.0, 50.0, OPAQUE_RED);
    let inst = pack_ui_instance(&input, 1.0);
    assert_eq!(inst.min_px, [3.5, 7.25]);
    assert_eq!(inst.size_px, [100.0, 50.0]);
}

#[test]
fn pack_folds_scale_into_corner_radius() {
    let mut input = plain_rect(0.0, 0.0, 50.0, 50.0, OPAQUE_RED);
    input.corner_radius = [2.0, 4.0, 6.0, 8.0];
    let inst = pack_ui_instance(&input, 3.0);
    assert_eq!(
        inst.corner_radius,
        [6.0, 12.0, 18.0, 24.0],
        "corner radii must be scaled per-corner"
    );
}

// --- pack: color premultiply at pack ---------------------------------------

#[test]
fn pack_premultiplies_fill_and_border_color() {
    let mut input = plain_rect(0.0, 0.0, 10.0, 10.0, 0x80_FF_FF_FF);
    input.border_color = 0x80_FF_00_00; // half-alpha red border
    input.border_width = [1.0; 4];
    let inst = pack_ui_instance(&input, 1.0);
    assert_eq!(
        inst.color,
        premultiply_rgba8(0x80_FF_FF_FF),
        "fill color must be premultiplied at pack"
    );
    assert_eq!(
        inst.border_color,
        premultiply_rgba8(0x80_FF_00_00),
        "border color must be premultiplied at pack"
    );
}

// --- pack: BORDER_ANY flag --------------------------------------------------

#[test]
fn pack_sets_border_any_when_width_positive() {
    let mut input = plain_rect(0.0, 0.0, 10.0, 10.0, OPAQUE_RED);
    input.border_width = [2.0; 4];
    let inst = pack_ui_instance(&input, 1.0);
    assert_ne!(inst.flags & FLAG_BORDER_ANY, 0, "positive border width sets BORDER_ANY");
    assert_eq!(inst.border_width, 2.0, "uniform border width recorded");
}

#[test]
fn pack_clears_border_any_when_width_zero() {
    let input = plain_rect(0.0, 0.0, 10.0, 10.0, OPAQUE_RED);
    let inst = pack_ui_instance(&input, 1.0);
    assert_eq!(inst.flags & FLAG_BORDER_ANY, 0, "zero border width clears BORDER_ANY");
    assert_eq!(inst.border_width, 0.0);
}

// --- pack: CLIP_PRESENT flag + clip AABB conversion ------------------------

#[test]
fn pack_unclipped_leaves_clip_zero_and_flag_clear() {
    let inst = pack_ui_instance(&plain_rect(0.0, 0.0, 10.0, 10.0, OPAQUE_RED), 1.0);
    assert_eq!(inst.flags & FLAG_CLIP_PRESENT, 0, "no clip ⇒ CLIP_PRESENT clear");
    assert_eq!(inst.clip, [0.0; 4], "unclipped clip stays zero (no sentinel)");
}

#[test]
fn pack_clip_present_converts_xywh_to_aabb_scaled() {
    let mut input = plain_rect(0.0, 0.0, 100.0, 100.0, OPAQUE_RED);
    // clip xywh (logical) = (10, 20, 30, 40) ⇒ AABB (10,20,40,60) ⇒ *2 = (20,40,80,120)
    input.clip = Some([10.0, 20.0, 30.0, 40.0]);
    let inst = pack_ui_instance(&input, 2.0);
    assert_ne!(inst.flags & FLAG_CLIP_PRESENT, 0, "clip present sets CLIP_PRESENT");
    assert_eq!(
        inst.clip,
        [20.0, 40.0, 80.0, 120.0],
        "clip must convert xywh→(min.xy,max.xy) and fold scale"
    );
}

// --- UiInstance byte view (no-bytemuck POD) --------------------------------

#[test]
fn instance_slice_as_bytes_has_exact_length() {
    let insts = [
        pack_ui_instance(&plain_rect(0.0, 0.0, 1.0, 1.0, OPAQUE_RED), 1.0),
        pack_ui_instance(&plain_rect(2.0, 2.0, 1.0, 1.0, OPAQUE_WHITE), 1.0),
        pack_ui_instance(&plain_rect(4.0, 4.0, 1.0, 1.0, OPAQUE_RED), 1.0),
    ];
    let bytes = UiInstance::slice_as_bytes(&insts);
    assert_eq!(
        bytes.len(),
        3 * UI_INSTANCE_SIZE,
        "byte view length must be N * UI_INSTANCE_SIZE"
    );
}

#[test]
fn instance_byte_view_roundtrips_first_field() {
    let inst = pack_ui_instance(&plain_rect(12.0, 34.0, 5.0, 6.0, OPAQUE_RED), 1.0);
    let one = [inst];
    let bytes = UiInstance::slice_as_bytes(&one);
    // min_px @ offset 0 is two f32 LE; reconstruct the first.
    let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(x, 12.0, "byte view min_px.x must match");
    assert_eq!(y, 34.0, "byte view min_px.y must match");
}

#[test]
fn instance_empty_slice_is_zero_bytes() {
    let bytes = UiInstance::slice_as_bytes(&[]);
    assert_eq!(bytes.len(), 0, "empty instance slice yields zero bytes");
}

/// UI-ADVANCED S2 gate G2-4: the byte view is sound AND correctly laid out at the
/// widened 80 B stride — `uv` reads back at offset 48 and `flags` at 76, straight out
/// of the raw byte image (this file runs under Miri, so the view's provenance is
/// exercised over the new size, not just its length).
#[test]
fn instance_byte_view_roundtrips_uv_and_flags_at_80b_offsets() {
    let mut input = plain_rect(1.0, 2.0, 3.0, 4.0, OPAQUE_RED);
    input.text_uv = Some([0.25, 0.5, 0.75, 1.0]);
    let inst = pack_ui_instance(&input, 1.0);
    let one = [inst];
    let bytes = UiInstance::slice_as_bytes(&one);
    assert_eq!(bytes.len(), UI_INSTANCE_SIZE, "one record is exactly the 80 B stride");

    let f32_at = |off: usize| {
        f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    };
    // uv @ 48 — the four normalized components, verbatim.
    assert_eq!(f32_at(48), 0.25, "uv.u0 must sit at byte offset 48");
    assert_eq!(f32_at(52), 0.5, "uv.v0 must sit at byte offset 52");
    assert_eq!(f32_at(56), 0.75, "uv.u1 must sit at byte offset 56");
    assert_eq!(f32_at(60), 1.0, "uv.v1 must sit at byte offset 60");
    // flags @ 76 — the FLAG_TEXT bit, read from the raw bytes.
    let flags = u32::from_le_bytes([bytes[76], bytes[77], bytes[78], bytes[79]]);
    assert_eq!(flags, FLAG_TEXT, "flags must sit at byte offset 76 (FLAG_TEXT only)");
}

// --- UI-ADVANCED S2: the un-aliasing (D1) ----------------------------------

/// Gate G2-5: the text lane REALLY migrated — a `FLAG_TEXT` instance carries the UV in
/// `uv` and ZERO in `corner_radius` (the retire is complete, not additive: a shader
/// still reading the old alias field would see zeros, not a stale copy of the UV).
#[test]
fn text_lane_writes_uv_and_zeroes_corner_radius() {
    let mut input = plain_rect(0.0, 0.0, 10.0, 10.0, OPAQUE_RED);
    input.text_uv = Some([0.125, 0.25, 0.5, 0.875]);
    let inst = pack_ui_instance(&input, 2.0);
    assert_ne!(inst.flags & FLAG_TEXT, 0, "text_uv sets FLAG_TEXT");
    assert_eq!(
        inst.uv,
        [0.125, 0.25, 0.5, 0.875],
        "the glyph UV must land in `uv`, verbatim (never scale-folded)"
    );
    assert_eq!(
        inst.corner_radius,
        [0.0; 4],
        "a glyph's corner_radius must be ZERO — the alias is retired, not shadowed"
    );
}

/// S2's rect-lane default (S-D8): a plain rect packs the identity UV `(0,0,1,1)` —
/// the constant every pre-S2 node now carries, unread by its shader branch.
#[test]
fn rect_lane_packs_identity_uv() {
    let mut input = plain_rect(0.0, 0.0, 10.0, 10.0, OPAQUE_RED);
    input.corner_radius = [3.0; 4];
    let inst = pack_ui_instance(&input, 1.0);
    assert_eq!(inst.flags & FLAG_TEXT, 0, "a rect is not a glyph");
    assert_eq!(inst.uv, [0.0, 0.0, 1.0, 1.0], "a rect packs the identity UV");
    assert_eq!(inst.corner_radius, [3.0; 4], "the radius stays the radius");
}

/// Gate G2-6: the reserved bits are actually zero — at S2 every packed instance uses
/// only bits 0..2 (`flags & 0xFFFF_FFF8 == 0`). Bits 3..31 are S-D2's budget (bit 3
/// FLAG_TEXTURED at S3, bit 4 reserved, 5..19 free, 20..31 the bindless slot) and
/// nothing may set them before the rung that owns them.
#[test]
fn packed_flags_reserved_bits_are_zero() {
    // Every lane the pack has: plain, bordered+rounded, clipped, and a clipped glyph.
    let plain = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    let mut bordered = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    bordered.border_width = [2.0; 4];
    bordered.border_color = OPAQUE_WHITE;
    bordered.corner_radius = [1.0; 4];
    let mut clipped = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    clipped.clip = Some([1.0, 1.0, 4.0, 4.0]);
    let mut glyph = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    glyph.text_uv = Some([0.0, 0.0, 1.0, 1.0]);
    glyph.clip = Some([1.0, 1.0, 4.0, 4.0]);

    for (label, input) in [
        ("plain", plain),
        ("bordered+rounded", bordered),
        ("clipped", clipped),
        ("clipped glyph", glyph),
    ] {
        let inst = pack_ui_instance(&input, 1.0);
        assert_eq!(
            inst.flags & 0xFFFF_FFF8,
            0,
            "{label}: bits 3..31 of flags must be zero at S2 (got {:#010x})",
            inst.flags
        );
    }
}

// --- UI-ADVANCED S3: the sprite lane (D2, S-D2, S-D8) ----------------------

/// A `PackInput` for a node carrying a `UiImage` at `slot` with `tint`, full-texture UV.
fn sprite_node(slot: u32, tint: u32) -> PackInput {
    let mut input = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    input.image = Some(UiImageInput {
        slot,
        uv: [0.0, 0.0, 1.0, 1.0],
        tint,
    });
    input
}

/// Gate G3-5: the 12-bit slot field ROUND-TRIPS. Packing slot `s` and reading `flags`
/// bits 20..31 back must yield `s` for both ends of the live table's range — the
/// property no offset assert can state, because the field is a bit range inside one word
/// and nothing but arithmetic connects the two ends of it.
#[test]
fn sprite_slot_round_trips_through_the_flags_bit_field() {
    for slot in [1u32, 2, 1234, UI_SLOT_MASK] {
        let inst = pack_ui_image_instance(&sprite_node(slot, OPAQUE_WHITE), 1.0)
            .expect("a node carrying UiImage emits a sprite record");
        assert_ne!(inst.flags & FLAG_TEXTURED, 0, "slot {slot}: FLAG_TEXTURED is set");
        assert_eq!(
            (inst.flags >> UI_SLOT_SHIFT) & UI_SLOT_MASK,
            slot,
            "slot {slot} must survive the pack into flags bits {UI_SLOT_SHIFT}..32 (got {:#010x})",
            inst.flags
        );
    }
    // The top of the field IS the top of the live table's range, with zero headroom —
    // S-D2's whole reason for the `BINDLESS_TEXTURE_CAPACITY` const-assert beside the
    // struct. If the capacity ever exceeds the field, this equality is the first thing
    // that stops being true.
    assert_eq!(
        UI_SLOT_MASK + 1,
        boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY,
        "the slot field's range and the bindless table's capacity are the SAME number"
    );
}

/// Gate G2-6's S3 SUCCESSOR: the reserved bits are still zero on BOTH lanes.
///
/// S2 proved every packed instance had bits 3..31 zero. S3 sets bit 3 and bits 20..31 —
/// deliberately, on the SPRITE record only. So the claim splits: a background record is
/// unchanged (bits 3..31 still zero, which is what keeps the S2 image pins identical),
/// and a sprite record uses ONLY bit 3 plus its slot field, leaving bit 4 (S7's reserved
/// per-sprite sampler index) and bits 5..19 zero.
///
/// **UI-ADVANCED S5 spent bits 5..19 (`FLAG_TILED` + the two 7-bit repeat counts), and this
/// test's claim is UNCHANGED — deliberately.** Those bits are set only on a nine-slice
/// SUB-QUAD whose own repeat count exceeds 1, and this file's subject is
/// `pack_ui_image_instance`, the WHOLE-RECT sprite record, which is emitted only for a node
/// with no `UiNineSlice` and therefore never carries them. The tiled lane's own bit census
/// is `ui_s5_sprite_sheet`'s G5-11, which asserts the same property from the other side: a
/// `1x1` region carries NO tile bits at all.
#[test]
fn packed_flags_use_only_the_bits_their_lane_owns() {
    let input = sprite_node(0x0AB, OPAQUE_WHITE);

    // The BACKGROUND record of a node that also carries a sprite: unchanged from S2.
    let background = pack_ui_instance(&input, 1.0);
    assert_eq!(
        background.flags & 0xFFFF_FFF8,
        0,
        "a background record must not gain a bit because its node carries a UiImage \
         (got {:#010x}) — this is what keeps the S2 image pins identical",
        background.flags
    );

    // The SPRITE record: bit 3 + the slot field, and NOTHING between them.
    let sprite = pack_ui_image_instance(&input, 1.0).expect("the node carries a UiImage");
    let between = sprite.flags & !(FLAG_TEXTURED | (UI_SLOT_MASK << UI_SLOT_SHIFT));
    assert_eq!(
        between, 0,
        "a sprite record must use only FLAG_TEXTURED and the slot field — bit 4 is \
         RESERVED for S7's per-sprite sampler index and bits 5..19 are free (got {:#010x})",
        sprite.flags
    );
}

/// S-D8's default-OFF row for this rung, at the pack: `UiImage::default()`'s fully
/// TRANSPARENT tint premultiplies to an all-zero color, so the sprite record contributes
/// nothing under the `src=ONE` premultiplied blend. This is the CPU half of gate G3-2 —
/// the half that runs on a device-less host, and the one red mutation M3-e trips.
#[test]
fn default_image_tint_packs_a_fully_transparent_sprite() {
    let default_tint = boyko_ui::components::UiImage::default().tint;
    assert_eq!(default_tint, 0, "the authored default is a transparent tint");
    let inst = pack_ui_image_instance(&sprite_node(1, default_tint), 1.0)
        .expect("presence of UiImage is what emits the record, not its tint");
    assert_eq!(
        inst.color, 0,
        "a transparent tint premultiplies to ZERO, so the sprite adds no pixels: \
         src == 0 leaves dst untouched under premultiplied blending"
    );
}

/// The sprite record is the SECOND record of its node, not a replacement: it takes the
/// node's geometry verbatim (scale-folded like a rect), its own UV verbatim (NEVER
/// scale-folded, exactly like the glyph UV), and packs no radius or border.
#[test]
fn sprite_record_mirrors_the_geometry_and_keeps_its_uv_unfolded() {
    let mut input = plain_rect(4.0, 6.0, 10.0, 20.0, OPAQUE_RED);
    input.corner_radius = [3.0; 4];
    input.clip = Some([0.0, 0.0, 5.0, 5.0]);
    input.image = Some(UiImageInput {
        slot: 7,
        uv: [0.25, 0.5, 0.75, 1.0],
        tint: OPAQUE_WHITE,
    });

    let background = pack_ui_instance(&input, 2.0);
    let sprite = pack_ui_image_instance(&input, 2.0).expect("the node carries a UiImage");

    assert_eq!(sprite.min_px, background.min_px, "same quad as its background");
    assert_eq!(sprite.size_px, background.size_px, "same quad as its background");
    assert_eq!(sprite.clip, background.clip, "the node's clip applies to both records");
    assert_ne!(
        sprite.flags & FLAG_CLIP_PRESENT,
        0,
        "the sprite carries the clip FLAG too, or the shader would not read the AABB"
    );
    assert_eq!(
        sprite.uv,
        [0.25, 0.5, 0.75, 1.0],
        "the sprite UV is written verbatim — a scale-folded UV would sample the wrong texels"
    );
    assert_eq!(
        sprite.corner_radius,
        [0.0; 4],
        "a sprite packs no radius (a rounded sprite is nine-slice's job, S4)"
    );
    assert_eq!(sprite.border_width, 0.0, "a sprite packs no border");
    assert_eq!(background.uv, [0.0, 0.0, 1.0, 1.0], "the background keeps its identity UV");
}

/// Absence is the structural skip: no `UiImage` ⇒ no sprite record at all, so an
/// image-less world's record stream is byte-identical to S2's (gate G3-2's premise).
#[test]
fn a_node_without_an_image_emits_no_sprite_record() {
    let input = plain_rect(0.0, 0.0, 8.0, 8.0, OPAQUE_RED);
    assert!(
        pack_ui_image_instance(&input, 1.0).is_none(),
        "capability is component presence — absence emits nothing, it does not emit a \
         disabled record"
    );
}

// --- UiOrtho: pixel→NDC, top-left origin, G11 corners ----------------------

/// Applies the ortho exactly as the vertex shader does: ndc = px*scale + translate.
fn apply_ortho(o: &UiOrtho, px: [f32; 2]) -> [f32; 2] {
    [px[0] * o.scale[0] + o.translate[0], px[1] * o.scale[1] + o.translate[1]]
}

#[test]
fn ortho_top_left_pixel_maps_to_ndc_minus_one() {
    let o = UiOrtho::for_extent(800, 600);
    let ndc = apply_ortho(&o, [0.0, 0.0]);
    assert_eq!(ndc, [-1.0, -1.0], "(0,0) must map to NDC (-1,-1) (top-left, y-down)");
}

#[test]
fn ortho_bottom_right_pixel_maps_to_ndc_plus_one_g11() {
    // G11 contract: the rect at the bottom-right corner lands at the bottom-right
    // texel — (w,h) → NDC (+1,+1) of the SAME image the UI pass renders into.
    let (w, h) = (1280u32, 720u32);
    let o = UiOrtho::for_extent(w, h);
    let ndc = apply_ortho(&o, [w as f32, h as f32]);
    assert_eq!(ndc, [1.0, 1.0], "(w,h) must map to NDC (+1,+1) (bottom-right)");
}

#[test]
fn ortho_center_pixel_maps_to_ndc_origin() {
    let (w, h) = (640u32, 480u32);
    let o = UiOrtho::for_extent(w, h);
    let ndc = apply_ortho(&o, [w as f32 / 2.0, h as f32 / 2.0]);
    assert_eq!(ndc, [0.0, 0.0], "the center pixel must map to NDC origin");
}

#[test]
fn ortho_uses_positive_y_scale() {
    // The canonical (non-GL) convention: positive y scale for the top-left origin.
    let o = UiOrtho::for_extent(100, 200);
    assert!(o.scale[1] > 0.0, "y scale must be POSITIVE (top-left, y-down NDC)");
    assert_eq!(o.scale, [2.0 / 100.0, 2.0 / 200.0]);
    assert_eq!(o.translate, [-1.0, -1.0]);
}

#[test]
fn ortho_extent_distinct_from_nominal_lands_bottom_right() {
    // Decision 9: when the render target extent differs from a nominal viewport, the
    // ortho uses the EXTENT THE UI PASS RENDERS INTO. A rect at that extent's corner
    // still lands at its bottom-right texel.
    let o = UiOrtho::for_extent(333, 777);
    let ndc = apply_ortho(&o, [333.0, 777.0]);
    assert!(
        (ndc[0] - 1.0).abs() < 1e-5 && (ndc[1] - 1.0).abs() < 1e-5,
        "the target-extent corner must map to (+1,+1): got {ndc:?}"
    );
}

#[test]
fn ortho_as_bytes_is_sixteen_bytes() {
    let o = UiOrtho::for_extent(10, 10);
    assert_eq!(o.as_bytes().len(), 16, "the ortho push block is 16 bytes");
}

// --- z-sort: painter's order (StackIndex ascending, stable tie-break) -------

/// Packs `n` distinguishable rects into the scratch and fills the key lane with the
/// given `(stack, append_idx)` tuples, then sorts. Returns the sorted min_px.x lane
/// (each rect is given a distinct x so the gather permutation is observable).
fn sort_with_keys(stacks: &[u32]) -> Vec<f32> {
    let mut scratch = UiRenderScratch::default();
    for (i, _) in stacks.iter().enumerate() {
        // Distinct x = append index so we can read back the gather permutation.
        scratch
            .pack
            .push(pack_ui_instance(&plain_rect(i as f32, 0.0, 1.0, 1.0, OPAQUE_RED), 1.0));
        scratch.keys.push((stacks[i], i as u32));
    }
    let mut gather = Vec::new();
    scratch.sort_by_stack(&mut gather);
    scratch.pack.iter().map(|inst| inst.min_px[0]).collect()
}

#[test]
fn sort_orders_by_stack_index_ascending() {
    // Append order: x=0 stack=2, x=1 stack=0, x=2 stack=1.
    // Sorted by stack asc: stack0(x=1), stack1(x=2), stack2(x=0).
    let xs = sort_with_keys(&[2, 0, 1]);
    assert_eq!(xs, vec![1.0, 2.0, 0.0], "rects must end in StackIndex-ascending order");
}

#[test]
fn sort_is_stable_within_equal_stack() {
    // Equal stack ⇒ append order preserved (the painter's-order tie-break — top-most
    // == last-appended at the same z draws last and wins overlap).
    let xs = sort_with_keys(&[5, 5, 5, 5]);
    assert_eq!(xs, vec![0.0, 1.0, 2.0, 3.0], "equal StackIndex preserves append order");
}

#[test]
fn sort_empty_is_noop() {
    let mut scratch = UiRenderScratch::default();
    let mut gather = Vec::new();
    scratch.sort_by_stack(&mut gather);
    assert_eq!(scratch.pack.len(), 0, "sorting an empty pack is a no-op");
}

#[test]
fn sort_single_element_unchanged() {
    let xs = sort_with_keys(&[42]);
    assert_eq!(xs, vec![0.0], "single-element sort is unchanged");
}

#[test]
fn sort_preserves_count_no_drop_or_dup() {
    // A permutation must neither drop nor duplicate (the gather is a bijection).
    let stacks = [3u32, 1, 4, 1, 5, 9, 2, 6];
    let xs = sort_with_keys(&stacks);
    assert_eq!(xs.len(), stacks.len(), "the sort must preserve the element count");
    let mut seen = xs.clone();
    seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        seen,
        (0..stacks.len()).map(|i| i as f32).collect::<Vec<_>>(),
        "every original record must appear exactly once (no drop/dup)"
    );
}

// --- generation change gate -------------------------------------------------

#[test]
fn generation_bump_increments() {
    let mut g = UiRenderGeneration::default();
    assert_eq!(g.generation, 0, "default generation starts at 0");
    g.bump();
    assert_eq!(g.generation, 1, "bump increments the generation");
    g.bump();
    assert_eq!(g.generation, 2);
}

#[test]
fn generation_bump_wraps_at_u64_max() {
    let mut g = UiRenderGeneration { generation: u64::MAX };
    g.bump();
    assert_eq!(g.generation, 0, "bump wraps (wrapping_add) at u64::MAX");
}
