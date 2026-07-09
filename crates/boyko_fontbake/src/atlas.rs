//! T3 — atlas packing, the dense metrics table, and the `.bfont` format.
//!
//! Packs every glyph's expanded transition-band field into one MTSDF RGBA8
//! atlas via a skyline packer, builds the dense [`GlyphMetrics`] table
//! (planeBounds/atlasBounds over the **expanded** quad, so the AA band is never
//! clipped), the [`AtlasMeta`], and sorted cmap/kern tables, then serializes
//! everything to an in-house `.bfont` binary (no serde — POD blits behind a
//! small header). A thin reader round-trips it byte-identically.
//!
//! Inter-glyph spacing is ≥ `distance_range_texels / 2` (here
//! [`ATLAS_PADDING_TEXELS`]) to prevent bilinear neighbor bleed across packed
//! glyphs.

use std::sync::Arc;

use boyko_threadpool::ThreadPool;

use crate::constants::{
    ATLAS_PADDING_TEXELS, BFONT_MAGIC, DISTANCE_RANGE_TEXELS, EDGE_COLORING_SEED,
    GENERATOR_VERSION, PIXELS_PER_EM,
};
use crate::extract::{Glyph, extract_codepoint, face_metrics};
use crate::face::FontFace;
use crate::msdf::{GlyphField, generate_glyph_field};

/// The atlas distance-field encoding kind. Owner-locked to MTSDF for P5b; the
/// field is carried in [`AtlasMeta`] so a future MSDF re-bake is data-only.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasKind {
    /// 3-channel MSDF (RGB), fill-only. Range 4.
    Msdf = 0,
    /// 4-channel MTSDF (RGB MSDF + A true-SDF). Range 6. The P5b default.
    Mtsdf = 1,
}

impl AtlasKind {
    #[inline]
    fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(AtlasKind::Msdf),
            1 => Some(AtlasKind::Mtsdf),
            _ => None,
        }
    }
}

/// One glyph's render + layout metrics. POD, `#[repr(C)]`. planeBounds and
/// atlasBounds describe the **expanded** transition-band quad (§T3).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMetrics {
    /// Pen advance, em units.
    pub advance_em: f32,
    /// planeBounds: left, bottom, right, top — em, expanded quad, baseline-rel.
    pub plane: [f32; 4],
    /// atlasBounds: left, bottom, right, top — texels, expanded quad.
    pub atlas: [f32; 4],
}

const _: () = assert!(size_of::<GlyphMetrics>() == 36);

/// Per-atlas metadata. `distance_range_texels` and `pixels_per_em` are both
/// carried; bound by `range_em = distance_range_texels / pixels_per_em`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasMeta {
    /// pxrange in texels (the shader divisor). 6 for MTSDF.
    pub distance_range_texels: f32,
    /// Global rasterization scale (binds `range_em`).
    pub pixels_per_em: f32,
    /// Atlas width, texels.
    pub atlas_w: u32,
    /// Atlas height, texels.
    pub atlas_h: u32,
    /// Ascender, em.
    pub ascender_em: f32,
    /// Descender, em.
    pub descender_em: f32,
    /// Line gap, em.
    pub line_gap_em: f32,
    /// The distance-field kind.
    pub kind: AtlasKind,
}

/// A codepoint → glyph-slot mapping entry. Sorted by `codepoint` for binary
/// search; `cp < 128` resolves via a direct array fast path at load.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedCodepoint {
    /// The Unicode scalar value.
    pub codepoint: u32,
    /// The dense per-font glyph slot.
    pub slot: u16,
}

/// A kerning pair: `(left_slot, right_slot)` packed into one key, sorted for
/// binary search; the adjustment is em units quantized to i16 font units.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernPair {
    /// `(left_slot as u32) << 16 | right_slot as u32`.
    pub key: u32,
    /// Adjustment, font units (signed). Apply as `adjust / units_per_em`.
    pub adjust: i16,
}

/// The decoded RGBA8 atlas image (the GPU upload source).
#[derive(Clone, Debug)]
pub struct AtlasImage {
    /// Width, texels.
    pub width: u32,
    /// Height, texels.
    pub height: u32,
    /// `width * height * 4` bytes, row-major RGBA8.
    pub pixels: Vec<u8>,
}

/// The full baked font: the in-memory form of a `.bfont` before/after
/// serialization. The tester asserts against these fields directly.
#[derive(Clone, Debug)]
pub struct BakedFont {
    /// Per-atlas metadata.
    pub meta: AtlasMeta,
    /// Dense glyph metrics, indexed by glyph slot (slot 0 == `.notdef`).
    pub glyphs: Vec<GlyphMetrics>,
    /// Sorted codepoint → slot map.
    pub cmap: Vec<MappedCodepoint>,
    /// Sorted kerning pairs (may be empty).
    pub kern: Vec<KernPair>,
    /// The packed atlas image.
    pub atlas: AtlasImage,
}

/// A skyline packer column-height tracker. `heights[x]` is the lowest free y at
/// texel column `x`; a rect drops into the lowest valid slot (fontstash / stb
/// precedent), incremental-friendly for a future dynamic atlas.
struct Skyline {
    width: u32,
    heights: Vec<u32>,
}

impl Skyline {
    fn new(width: u32) -> Self {
        Self {
            width,
            heights: vec![0; width as usize],
        }
    }

    /// Finds the lowest y at which a `w × h` rect fits at some x, returning
    /// `(x, y)`, or `None` when it does not fit within `max_h`.
    fn find(&self, w: u32, h: u32, max_h: u32) -> Option<(u32, u32)> {
        if w > self.width {
            return None;
        }
        let mut best: Option<(u32, u32)> = None;
        let mut x = 0;
        while x + w <= self.width {
            // The rect's y is the max column height over [x, x+w).
            let mut y = 0;
            for col in x..x + w {
                y = y.max(self.heights[col as usize]);
            }
            if y + h <= max_h && best.is_none_or(|(_, by)| y < by) {
                best = Some((x, y));
            }
            x += 1;
        }
        best
    }

    /// Marks a placed rect, raising the skyline.
    fn place(&mut self, x: u32, w: u32, top: u32) {
        for col in x..x + w {
            self.heights[col as usize] = top;
        }
    }
}

/// Quantizes a `[0, 1]` float field value to u8 (nearest, saturating).
#[inline]
fn quantize(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// A glyph's generated field plus the slot/codepoint bookkeeping needed to
/// build the tables after packing.
struct PreparedGlyph {
    slot: u16,
    glyph: Glyph,
    field: Option<GlyphField>,
}

/// Bakes a font from a [`FontFace`] over the given codepoints into a
/// [`BakedFont`] (MTSDF). `pool` parallelizes per-texel field generation when
/// provided.
///
/// Slot 0 is always `.notdef`; the requested codepoints follow in sorted order.
/// Empty glyphs (e.g. space) carry advance-only metrics and no atlas entry.
pub fn bake_font(
    face: &dyn FontFace,
    codepoints: &[char],
    pool: Option<&Arc<ThreadPool>>,
) -> BakedFont {
    let fmetrics = face_metrics(face);

    // Build the slot order: .notdef first, then sorted unique codepoints.
    let mut sorted: Vec<char> = codepoints.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut prepared: Vec<PreparedGlyph> = Vec::with_capacity(sorted.len() + 1);
    let mut cmap: Vec<MappedCodepoint> = Vec::with_capacity(sorted.len());

    // Slot 0: .notdef (glyph id 0).
    let notdef = crate::extract::extract_glyph(face, crate::face::GlyphId(0));
    prepared.push(PreparedGlyph {
        slot: 0,
        glyph: notdef,
        field: None, // filled below
    });

    for (i, &cp) in sorted.iter().enumerate() {
        let slot = (i + 1) as u16;
        let glyph = extract_codepoint(face, cp);
        cmap.push(MappedCodepoint {
            codepoint: cp as u32,
            slot,
        });
        prepared.push(PreparedGlyph {
            slot,
            glyph,
            field: None,
        });
    }

    // Generate fields (the parallel-per-glyph-per-texel work). Each glyph's
    // field uses the disjoint-row parallel distance pass internally.
    for pg in &mut prepared {
        pg.field = generate_glyph_field(&pg.glyph.outline, pool);
    }

    // cmap is already sorted (sorted codepoints). Build kern over the slot pairs.
    let kern = build_kern(face, &prepared, fmetrics.units_per_em);

    // Pack the non-empty fields.
    pack_and_build(&prepared, &cmap, &kern, &fmetrics)
}

/// Builds the sorted kern table from the face over every present glyph pair.
fn build_kern(face: &dyn FontFace, prepared: &[PreparedGlyph], _upem: u16) -> Vec<KernPair> {
    let mut kern: Vec<KernPair> = Vec::new();
    for left in prepared {
        for right in prepared {
            let adjust = face.kerning(left.glyph.id, right.glyph.id);
            if adjust != 0 {
                let key = ((left.slot as u32) << 16) | right.slot as u32;
                kern.push(KernPair { key, adjust });
            }
        }
    }
    kern.sort_unstable_by_key(|k| k.key);
    kern
}

/// Packs the prepared fields into one atlas and assembles the [`BakedFont`].
fn pack_and_build(
    prepared: &[PreparedGlyph],
    cmap: &[MappedCodepoint],
    kern: &[KernPair],
    fmetrics: &crate::extract::FaceMetrics,
) -> BakedFont {
    let pad = ATLAS_PADDING_TEXELS;

    // Estimate atlas width: round up the total area to a square, power-of-two-ish.
    let total_area: u64 = prepared
        .iter()
        .filter_map(|p| p.field.as_ref())
        .map(|f| ((f.width + pad) as u64) * ((f.height + pad) as u64))
        .sum();
    let side = ((total_area as f64).sqrt().ceil() as u32 + pad).next_power_of_two();
    let atlas_w = side.max(64);
    // Height grows; cap generously, packer raises the skyline.
    let max_h = atlas_w * 8;

    // Pack tallest-first for density.
    let mut order: Vec<usize> = (0..prepared.len()).filter(|&i| prepared[i].field.is_some()).collect();
    order.sort_unstable_by(|&a, &b| {
        let ha = prepared[a].field.as_ref().map_or(0, |f| f.height);
        let hb = prepared[b].field.as_ref().map_or(0, |f| f.height);
        hb.cmp(&ha)
    });

    let mut skyline = Skyline::new(atlas_w);
    // Per-glyph (slot) placement: (x, y) of the field's lower-left in texels.
    let mut placement: Vec<Option<(u32, u32)>> = vec![None; prepared.len()];
    let mut used_h = 0u32;

    for &i in &order {
        let f = prepared[i].field.as_ref().expect("invariant: filtered to Some");
        let w = f.width + pad;
        let h = f.height + pad;
        let (x, y) = skyline
            .find(w, h, max_h)
            .unwrap_or((0, used_h)); // fallback append (max_h is generous)
        skyline.place(x, w, y + h);
        // Place the field at (x + pad/2, y + pad/2)-ish; keep pad on the low
        // side so the high-side padding is covered by the next rect's spacing.
        placement[i] = Some((x, y));
        used_h = used_h.max(y + h);
    }

    let atlas_h = used_h.max(1).next_power_of_two();

    // Blit fields into the RGBA8 atlas.
    let mut pixels = vec![0u8; (atlas_w * atlas_h * 4) as usize];
    for (i, pg) in prepared.iter().enumerate() {
        let (Some(f), Some((px, py))) = (pg.field.as_ref(), placement[i]) else {
            continue;
        };
        for y in 0..f.height {
            for x in 0..f.width {
                let src = ((y * f.width + x) * 4) as usize;
                let dx = px + x;
                let dy = py + y;
                let dst = ((dy * atlas_w + dx) * 4) as usize;
                pixels[dst] = quantize(f.texels[src]);
                pixels[dst + 1] = quantize(f.texels[src + 1]);
                pixels[dst + 2] = quantize(f.texels[src + 2]);
                pixels[dst + 3] = quantize(f.texels[src + 3]);
            }
        }
    }

    // Build the dense GlyphMetrics table.
    let mut glyphs: Vec<GlyphMetrics> = Vec::with_capacity(prepared.len());
    for (i, pg) in prepared.iter().enumerate() {
        let advance_em = pg.glyph.advance_em;
        match (pg.field.as_ref(), placement[i]) {
            (Some(f), Some((px, py))) => {
                // planeBounds (em, baseline-relative) describes the expanded
                // quad — the field's em extent from origin_em.
                let left = f.origin_em.x;
                let bottom = f.origin_em.y;
                let right = f.origin_em.x + f.width as f32 * f.texel_em;
                let top = f.origin_em.y + f.height as f32 * f.texel_em;
                // atlasBounds (texels), expanded quad in atlas space. Bottom/top
                // follow the atlas's y-down texel convention as stored.
                let a_left = px as f32;
                let a_bottom = py as f32;
                let a_right = (px + f.width) as f32;
                let a_top = (py + f.height) as f32;
                glyphs.push(GlyphMetrics {
                    advance_em,
                    plane: [left, bottom, right, top],
                    atlas: [a_left, a_bottom, a_right, a_top],
                });
            }
            _ => {
                // Empty glyph (space): advance only, zero-area quad.
                glyphs.push(GlyphMetrics {
                    advance_em,
                    plane: [0.0; 4],
                    atlas: [0.0; 4],
                });
            }
        }
    }

    let meta = AtlasMeta {
        distance_range_texels: DISTANCE_RANGE_TEXELS,
        pixels_per_em: PIXELS_PER_EM,
        atlas_w,
        atlas_h,
        ascender_em: fmetrics.ascender_em,
        descender_em: fmetrics.descender_em,
        line_gap_em: fmetrics.line_gap_em,
        kind: AtlasKind::Mtsdf,
    };

    BakedFont {
        meta,
        glyphs,
        cmap: cmap.to_vec(),
        kern: kern.to_vec(),
        atlas: AtlasImage {
            width: atlas_w,
            height: atlas_h,
            pixels,
        },
    }
}

// --- .bfont serialization -------------------------------------------------

/// A little-endian byte writer over a growable buffer (load-time scratch).
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }
    #[inline]
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
}

/// A little-endian byte reader; every read is bounds-checked, returning `None`
/// on truncation (no panics on malformed input).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
}

/// Serializes a [`BakedFont`] to the in-house `.bfont` binary.
///
/// Layout: a magic + version + pinned-seed header (Decision T2-E), then the
/// `AtlasMeta`, then count-prefixed POD tables (glyphs, cmap, kern), then the
/// atlas pixels. Little-endian throughout. Round-trips byte-identically via
/// [`read_bfont`].
pub fn write_bfont(font: &BakedFont) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(BFONT_MAGIC);
    w.u32(GENERATOR_VERSION);
    w.u64(EDGE_COLORING_SEED);

    // AtlasMeta.
    let m = &font.meta;
    w.f32(m.distance_range_texels);
    w.f32(m.pixels_per_em);
    w.u32(m.atlas_w);
    w.u32(m.atlas_h);
    w.f32(m.ascender_em);
    w.f32(m.descender_em);
    w.f32(m.line_gap_em);
    w.u32(m.kind as u32);

    // Glyph table.
    w.u32(font.glyphs.len() as u32);
    for g in &font.glyphs {
        w.f32(g.advance_em);
        for v in g.plane {
            w.f32(v);
        }
        for v in g.atlas {
            w.f32(v);
        }
    }

    // cmap.
    w.u32(font.cmap.len() as u32);
    for c in &font.cmap {
        w.u32(c.codepoint);
        w.u16(c.slot);
    }

    // kern.
    w.u32(font.kern.len() as u32);
    for k in &font.kern {
        w.u32(k.key);
        w.i16(k.adjust);
    }

    // Atlas pixels.
    w.u32(font.atlas.width);
    w.u32(font.atlas.height);
    w.u32(font.atlas.pixels.len() as u32);
    w.bytes(&font.atlas.pixels);

    w.buf
}

/// Deserializes a `.bfont` binary back into a [`BakedFont`].
///
/// Returns `None` on a bad magic, an unknown version/kind, or truncation. No
/// panics on malformed input (every read is bounds-checked).
pub fn read_bfont(bytes: &[u8]) -> Option<BakedFont> {
    let mut r = Reader::new(bytes);
    if r.u32()? != BFONT_MAGIC {
        return None;
    }
    let version = r.u32()?;
    if version != GENERATOR_VERSION {
        return None;
    }
    let _seed = r.u64()?;

    let meta = AtlasMeta {
        distance_range_texels: r.f32()?,
        pixels_per_em: r.f32()?,
        atlas_w: r.u32()?,
        atlas_h: r.u32()?,
        ascender_em: r.f32()?,
        descender_em: r.f32()?,
        line_gap_em: r.f32()?,
        kind: AtlasKind::from_u32(r.u32()?)?,
    };

    let glyph_count = r.u32()? as usize;
    let mut glyphs = Vec::with_capacity(glyph_count);
    for _ in 0..glyph_count {
        let advance_em = r.f32()?;
        let plane = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let atlas = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        glyphs.push(GlyphMetrics {
            advance_em,
            plane,
            atlas,
        });
    }

    let cmap_count = r.u32()? as usize;
    let mut cmap = Vec::with_capacity(cmap_count);
    for _ in 0..cmap_count {
        let codepoint = r.u32()?;
        let slot = r.u16()?;
        cmap.push(MappedCodepoint { codepoint, slot });
    }

    let kern_count = r.u32()? as usize;
    let mut kern = Vec::with_capacity(kern_count);
    for _ in 0..kern_count {
        let key = r.u32()?;
        let adjust = r.i16()?;
        kern.push(KernPair { key, adjust });
    }

    let width = r.u32()?;
    let height = r.u32()?;
    let px_len = r.u32()? as usize;
    let pixels = r.take(px_len)?.to_vec();

    Some(BakedFont {
        meta,
        glyphs,
        cmap,
        kern,
        atlas: AtlasImage {
            width,
            height,
            pixels,
        },
    })
}

/// Resolves a codepoint to a glyph slot via binary search over the sorted cmap
/// (the load-time loader's lookup, mirroring the runtime's). The runtime keeps a
/// 128-entry direct array for the ASCII fast path
/// ([`crate::constants::ASCII_FAST_PATH_LIMIT`]); the baker uses the uniform
/// binary search since it is not hot. Returns `0` (`.notdef`) when unmapped.
pub fn lookup_slot(cmap: &[MappedCodepoint], cp: u32) -> u16 {
    debug_assert!(
        cmap.windows(2).all(|w| w[0].codepoint < w[1].codepoint),
        "invariant: cmap must be sorted+deduped for binary search"
    );
    match cmap.binary_search_by_key(&cp, |e| e.codepoint) {
        Ok(i) => cmap[i].slot,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_endpoints_and_midpoint() {
        assert_eq!(quantize(0.0), 0, "0.0 ⇒ 0");
        assert_eq!(quantize(1.0), 255, "1.0 ⇒ 255");
        assert_eq!(quantize(0.5), 128, "0.5 rounds to 128 (nearest)");
    }

    #[test]
    fn quantize_saturates_out_of_range() {
        assert_eq!(quantize(-1.0), 0, "negative clamps to 0");
        assert_eq!(quantize(2.0), 255, "above 1 clamps to 255");
    }

    #[test]
    fn atlas_kind_roundtrips_through_u32() {
        assert_eq!(AtlasKind::from_u32(0), Some(AtlasKind::Msdf));
        assert_eq!(AtlasKind::from_u32(1), Some(AtlasKind::Mtsdf));
        assert_eq!(AtlasKind::from_u32(2), None, "unknown kind rejected");
        assert_eq!(AtlasKind::Mtsdf as u32, 1);
    }

    #[test]
    fn skyline_first_fit_drops_to_origin() {
        let sky = Skyline::new(64);
        let pos = sky.find(10, 10, 256).expect("fits in an empty skyline");
        assert_eq!(pos, (0, 0), "first rect lands at the origin");
    }

    #[test]
    fn skyline_rejects_too_wide() {
        let sky = Skyline::new(8);
        assert!(sky.find(16, 4, 256).is_none(), "a rect wider than the atlas does not fit");
    }

    #[test]
    fn skyline_raises_and_packs_second_rect() {
        let mut sky = Skyline::new(64);
        let (x0, y0) = sky.find(20, 10, 256).unwrap();
        sky.place(x0, 20, y0 + 10);
        let (x1, y1) = sky.find(20, 10, 256).unwrap();
        // The second rect must not overlap the first: either to the right, or
        // stacked above where the skyline was raised.
        let overlap_x = x0 < x1 + 20 && x1 < x0 + 20;
        let overlap_y = y0 < y1 + 10 && y1 < y0 + 10;
        assert!(!(overlap_x && overlap_y), "second placement must not overlap the first");
    }

    #[test]
    fn skyline_respects_max_height() {
        let mut sky = Skyline::new(16);
        sky.place(0, 16, 100); // raise the whole row to 100
        assert!(sky.find(16, 10, 105).is_none(), "no room under a tight max_h");
        assert!(sky.find(16, 10, 256).is_some(), "fits under a generous max_h");
    }

    #[test]
    fn writer_reader_roundtrip_primitives() {
        let mut w = Writer::new();
        w.u32(0x1234_5678);
        w.u64(0xDEAD_BEEF_CAFE_F00D);
        w.f32(core::f32::consts::PI);
        w.i16(-12345);
        w.u16(54321);
        let buf = w.buf;
        let mut r = Reader::new(&buf);
        assert_eq!(r.u32(), Some(0x1234_5678));
        assert_eq!(r.u64(), Some(0xDEAD_BEEF_CAFE_F00D));
        assert_eq!(r.f32(), Some(core::f32::consts::PI));
        assert_eq!(r.i16(), Some(-12345));
        assert_eq!(r.u16(), Some(54321));
    }

    #[test]
    fn reader_take_returns_none_on_truncation() {
        let buf = [0u8; 2];
        let mut r = Reader::new(&buf);
        assert!(r.u32().is_none(), "reading 4 bytes from a 2-byte buffer fails cleanly");
    }

    #[test]
    fn reader_take_advances_position() {
        let buf = [1u8, 2, 3, 4, 5, 6];
        let mut r = Reader::new(&buf);
        assert_eq!(r.take(2), Some(&[1u8, 2][..]));
        assert_eq!(r.take(2), Some(&[3u8, 4][..]));
        assert_eq!(r.take(3), None, "only 2 bytes remain");
    }

    #[test]
    fn lookup_slot_finds_present_and_misses_absent() {
        let cmap = vec![
            MappedCodepoint { codepoint: 'A' as u32, slot: 4 },
            MappedCodepoint { codepoint: 'a' as u32, slot: 7 },
            MappedCodepoint { codepoint: 'z' as u32, slot: 9 },
        ];
        assert_eq!(lookup_slot(&cmap, 'A' as u32), 4, "present low codepoint");
        assert_eq!(lookup_slot(&cmap, 'z' as u32), 9, "present high codepoint");
        assert_eq!(lookup_slot(&cmap, 'B' as u32), 0, "absent ⇒ .notdef slot 0");
    }

    #[test]
    fn lookup_slot_empty_cmap_returns_notdef() {
        assert_eq!(lookup_slot(&[], 'A' as u32), 0, "empty cmap ⇒ slot 0");
    }

    #[test]
    fn glyph_metrics_is_pod_sized() {
        // The compile-time const-assert already pins this; mirror it as a runtime
        // check so the intent is visible in the test inventory.
        assert_eq!(size_of::<GlyphMetrics>(), 36);
    }
}
