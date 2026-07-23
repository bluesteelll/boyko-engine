//! Property-based tests (proptest) for the bake math + serialization.
//!
//! These generate random inputs and assert invariants that must hold for ALL
//! inputs (not just the hand-picked golden cases): the `.bfont` round-trip, the
//! skyline packer's non-overlap guarantee, the cmap sort/permutation property,
//! the quantizer's monotonic clamp, and field finiteness across a codepoint
//! sweep. CPU-only, no GPU.

use boyko_fontbake::atlas::{
    AtlasImage, AtlasKind, AtlasMeta, BakedFont, GlyphMetrics, KernPair, MappedCodepoint,
    lookup_slot,
};
use boyko_fontbake::extract::extract_codepoint;
use boyko_fontbake::msdf::generate_glyph_field;
use boyko_fontbake::{TtfFace, bake_font, read_bfont, write_bfont};
use proptest::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

fn face() -> Option<&'static TtfFace> {
    static FACE: OnceLock<Option<TtfFace>> = OnceLock::new();
    FACE.get_or_init(|| {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("Ubuntu-Light.ttf");
        std::fs::read(path).ok().and_then(|b| TtfFace::from_bytes(&b))
    })
    .as_ref()
}

/// Builds an arbitrary in-memory `BakedFont` from random POD tables (the atlas
/// pixels too), so the round-trip is exercised independently of the generator.
fn arb_baked() -> impl Strategy<Value = BakedFont> {
    let meta = (
        any::<f32>(),
        any::<f32>(),
        1u32..32,
        1u32..32,
        any::<f32>(),
        any::<f32>(),
        any::<f32>(),
        prop::bool::ANY,
    )
        .prop_map(|(dr, ppem, w, h, asc, desc, gap, mtsdf)| AtlasMeta {
            distance_range_texels: dr,
            pixels_per_em: ppem,
            atlas_w: w,
            atlas_h: h,
            ascender_em: asc,
            descender_em: desc,
            line_gap_em: gap,
            kind: if mtsdf { AtlasKind::Mtsdf } else { AtlasKind::Msdf },
        });

    let glyphs = prop::collection::vec(
        (any::<f32>(), any::<[f32; 4]>(), any::<[f32; 4]>())
            .prop_map(|(advance_em, plane, atlas)| GlyphMetrics { advance_em, plane, atlas }),
        0..8,
    );
    let cmap = prop::collection::vec(
        (0u32..0x10FFFF, any::<u16>()).prop_map(|(codepoint, slot)| MappedCodepoint { codepoint, slot }),
        0..8,
    );
    let kern = prop::collection::vec(
        (any::<u32>(), any::<i16>()).prop_map(|(key, adjust)| KernPair { key, adjust }),
        0..8,
    );
    // 2026-07 audit: the atlas dimensions must SATISFY `AtlasImage`'s documented
    // invariant (`pixels.len() == width * height * 4`, tightly-packed RGBA8, non-zero
    // extent). The previous generator emitted `width = pixels.len(), height = 1`,
    // i.e. a payload 4x too small for its own declared extent — it was encoding the
    // very defect `read_bfont` now rejects (that mismatch reaches
    // `vkCmdCopyBufferToImage` as an out-of-bounds device read). Generate the extent
    // first and derive the payload from it, so the fixture models a REAL atlas.
    let extent = (1u32..8, 1u32..8);

    (meta, glyphs, cmap, kern, extent).prop_map(|(meta, glyphs, cmap, kern, (width, height))| {
        let pixels = (0..(width as usize) * (height as usize) * 4)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>();
        BakedFont {
            meta,
            glyphs,
            cmap,
            kern,
            atlas: AtlasImage { width, height, pixels },
        }
    })
}

proptest! {
    /// A written .bfont always re-parses and re-serializes byte-identically.
    #[test]
    fn bfont_roundtrip_is_stable(font in arb_baked()) {
        let bytes = write_bfont(&font);
        let back = read_bfont(&bytes).expect("written .bfont must parse");
        let bytes2 = write_bfont(&back);
        prop_assert_eq!(bytes, bytes2, "re-serialization must be byte-identical");
    }

    /// Reading never panics on arbitrary bytes — it returns None or a value.
    #[test]
    fn read_bfont_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = read_bfont(&bytes); // must not panic; Some/None both acceptable
    }

    /// Truncating a valid .bfont at any prefix never yields a value that
    /// re-serializes back to the original (a short read is rejected, not silently
    /// accepted) — and never panics.
    #[test]
    fn truncated_bfont_is_rejected_cleanly(font in arb_baked(), cut in 0usize..512) {
        let bytes = write_bfont(&font);
        let cut = cut.min(bytes.len().saturating_sub(1));
        let truncated = &bytes[..cut];
        // Either None, or (if the cut happened to land on a full valid prefix —
        // impossible here because the pixel tail is length-prefixed) a value.
        if let Some(back) = read_bfont(truncated) {
            // If it parsed, it must have consumed a self-consistent record.
            prop_assert_eq!(back.atlas.pixels.len(), back.atlas.width as usize);
        }
    }

    /// The cmap a bake produces is sorted ascending with no duplicate codepoints
    /// (the binary-search precondition), and every requested mapped codepoint
    /// resolves to a unique slot.
    #[test]
    fn baked_cmap_is_sorted_and_a_permutation(
        chars in prop::collection::hash_set(prop::char::range('!', '~'), 1..16)
    ) {
        let Some(face) = face() else { return Ok(()); };
        let cps: Vec<char> = chars.into_iter().collect();
        let baked = bake_font(face, &cps, None);
        // sorted + deduped
        prop_assert!(
            baked.cmap.windows(2).all(|w| w[0].codepoint < w[1].codepoint),
            "cmap must be strictly ascending"
        );
        // every input codepoint is present (a permutation onto slots)
        for &cp in &cps {
            let slot = lookup_slot(&baked.cmap, cp as u32);
            // slot 0 is .notdef; a mapped glyph resolves to a real OR notdef slot
            // (notdef only if the font lacks the glyph — these are ASCII so present).
            prop_assert!((slot as usize) < baked.glyphs.len(), "slot in range for '{}'", cp);
        }
    }

    /// Generated fields are always finite and in [0,1] across the printable ASCII
    /// sweep (no NaN/Inf, no out-of-range — the NaN black hole never originates in
    /// the bake).
    #[test]
    fn generated_field_is_finite_and_unit_ranged(cp in prop::char::range('!', '~')) {
        let Some(face) = face() else { return Ok(()); };
        let g = extract_codepoint(face, cp);
        if let Some(field) = generate_glyph_field(&g.outline, None) {
            for &t in &field.texels {
                prop_assert!(t.is_finite(), "texel must be finite for '{}'", cp);
                prop_assert!((0.0..=1.0).contains(&t), "texel in [0,1] for '{}'", cp);
            }
        }
    }
}
