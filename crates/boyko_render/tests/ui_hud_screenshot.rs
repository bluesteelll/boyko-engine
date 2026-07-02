//! GUI P6b — the HUD-bound-to-ECS-`Health` screenshot harness + CPU invariants.
//!
//! # What this proves
//!
//! An on-screen HUD text node whose content is DATA-BOUND to an ECS `Health`
//! component, end-to-end through the SHIPPED P4/#27 binding chain
//! (`BindText{source}` -> `ui_bind_discovery`/`ui_bind_apply` -> `UiTextBuffer` ->
//! the live `emit_glyphs` MSDF emitter), rendered through the REUSED P5a/P5b render
//! path (`pack_ui_instance` text lane -> `ui_upload` -> `ui_handles` ->
//! `record_ui_rects`) into an offscreen target, read back, and written as a viewable
//! BMP the owner eyeballs. ZERO new engine/core code: the bind systems, the accessor
//! trampoline, the emitter, the pack lane, and the offscreen render+readback recipe
//! all pre-exist and are proven (`text_bind_emit.rs`, `ui_text_gpu_golden.rs`).
//!
//! The one genuinely-new authored asset is a synthetic, legible 7-segment-style digit
//! atlas (`hud_digit_font`) — each glyph cell a DISTINCT binary-median footprint so a
//! human reads "75/100", not an undifferentiated bar. Binary median (0 or 1 per texel)
//! removes all SDF anti-alias subtlety, so the GPU foreground texel is bit-exact.
//!
//! # Split: CPU tests run in-workflow; the GPU screenshot is `#[ignore]`d
//!
//! - `hud_binding_value`            — the bind chain formats the live value (6a).
//! - `hud_glyph_packing_golden`     — the bound text packs into glyph instances (6b).
//! - `hud_chars_map_to_distinct_cells` — per-digit cells are distinct UVs (6c, W1 guard).
//! - `p6b_hud_screenshot` (`#[ignore]`) — boots Vulkan, renders, asserts the blend
//!   texel, and writes the BMP. Owner-run on the RTX (Vulkan boot can hang headless).
//!
//! The owner screenshot command (one line, RTX 3060):
//!
//! ```text
//! RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu CARGO_BUILD_TARGET=x86_64-pc-windows-gnu \
//!   cargo test -p boyko_render --test ui_hud_screenshot \
//!   p6b_hud_screenshot -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Output image: `D:\claude\BoykoEngine\target\screenshots\p6b_hud.bmp`

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Component, Resource};

use boyko_fontbake::atlas::{
    AtlasImage, AtlasKind, AtlasMeta, BakedFont, GlyphMetrics, MappedCodepoint, bake_font,
};
use boyko_fontbake::face::TtfFace;

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, TemplateId, UiTextBuffer, NO_FIELD};
use boyko_ui::binding::Bindable;
use boyko_ui::components::{ComputedRect, StackIndex};
use boyko_ui::text::{emit_glyphs, FontId, FontTable, GlyphInstance, TextAlign, UiText};

use boyko_render::{pack_ui_instance, PackInput, UiInstance};

// ---------------------------------------------------------------------------
// Fixed screenshot geometry — the W2 blend texel DERIVES from these constants.
// ---------------------------------------------------------------------------

/// Offscreen screenshot width (logical == physical px; scale 1.0).
const W: u32 = 256;
/// Offscreen screenshot height.
const H: u32 = 128;
const TEXELS: usize = (W * H) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// First glyph pen origin (top-left of glyph 0's quad), logical px.
const X0: f32 = 16.0;
const Y0: f32 = 48.0;
/// Glyph quad size, logical px.
const GW: f32 = 24.0;
const GH: f32 = 32.0;
/// Pen advance per glyph, logical px.
const GADV: f32 = 28.0;

/// The W2 assert texel: a DEEP INTERIOR texel of glyph 0 (`'7'`)'s 2-texel-thick top
/// bar — NOT a naive geometric centre. It is derived by reproducing the exact FS
/// coverage path (`local_uv -> lerp -> bilinear atlas Sample -> median3 -> screen_px_range
/// coverage`) in [`coverage_at`] and picking a screen pixel whose bilinear sample lands
/// fully inside the solid bar (`sd == 1.0 => coverage == 1.0`). At `coverage == 1` the
/// premultiplied src (`fg * 1`) over the opaque CLEAR yields `FG_BYTES` exactly. The CPU
/// test `hud_fg_texel_is_fully_covered` re-derives this so a footprint/geometry change
/// that breaks the GPU assert fails IN-WORKFLOW without a device.
///
/// Glyph 0's quad is `x in [16, 40)`, `y in [48, 80)`. `'7'`'s top bar occupies atlas
/// rows 0..2 (screen `y in [48, ~58)`); `(28, 51)` is centred in that bar, columns deep
/// inside the lit run, so the bilinear sample is `1.0` on every channel.
const FG_TEXEL: (u32, u32) = (28, 51);
/// A background texel outside every glyph quad (no-bleed half of the assert).
const BG_TEXEL: (u32, u32) = (2, 2);

/// The offscreen CLEAR color (opaque so the premultiplied-over blend is deterministic
/// where `src == 0`). Same convention as `ui_text_gpu_golden`.
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
/// The glyph foreground (opaque RED) — STRAIGHT RGBA8; premultiply is identity at A=255.
const FG: u32 = 0xFF00_00FF; // byte0=R=FF, byte3=A=FF
const FG_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

// ---------------------------------------------------------------------------
// The synthetic legible-digit atlas (W1) — the one new authored asset.
// ---------------------------------------------------------------------------

/// Texels per glyph cell, each axis (8x8 binary-median block).
const CELL: u32 = 8;
/// Number of authored cells: `'0'..'9'` (10) + `'/'` (1).
const NCELLS: u32 = 11;
/// Atlas dimensions: cells laid out horizontally, one row tall.
const ATLAS_W: u32 = CELL * NCELLS; // 88
const ATLAS_H: u32 = CELL; // 8

/// The HUD string the bound `Health{75,100}` formats to under `TemplateId::Ratio`.
const HUD_STRING: &str = "75/100";

/// 8x8 binary footprint per cell (MSB == leftmost column), distinct per glyph so the
/// number is humanly legible. Cell order: index 0..=9 are digits `'0'..'9'`, index 10
/// is `'/'`. Each row is one `u8`; bit 0x80 is column 0. The shapes are a 7-segment
/// style with **2-texel-thick strokes** (so every lit segment has a fully-covered
/// interior, not a single-texel sliver). Binary median: a set bit is foreground
/// (`median = 1`), a clear bit is exterior (`median = 0`).
///
/// Stroke thickness matters: a single-texel stroke stretched over a 24x32-px quad at
/// `screen_px_range ~= 21` antialiases to PARTIAL coverage at every lit texel (bilinear
/// bleed from the adjacent off-texel), so it renders faint and no texel reaches
/// `coverage == 1`. Thick (>= 2-texel) strokes render their interior at full coverage
/// and give the W2 assert a provably solid `FG_TEXEL` (see `coverage_at`). The legibility
/// the W1 redesign promised therefore holds: bold, distinct, readable digits.
const DIGIT_BITS: [[u8; 8]; NCELLS as usize] = [
    // '0' — full ring, hollow centre.
    [0b0111_1110, 0b0111_1110, 0b0110_0110, 0b0110_0110, 0b0110_0110, 0b0110_0110, 0b0111_1110, 0b0111_1110],
    // '1' — thick centre stem with a serifed foot.
    [0b0001_1000, 0b0011_1000, 0b0001_1000, 0b0001_1000, 0b0001_1000, 0b0001_1000, 0b0011_1100, 0b0011_1100],
    // '2' — top, upper-right, middle, lower-left, bottom.
    [0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0111_1110, 0b0111_1110, 0b0110_0000, 0b0111_1110, 0b0111_1110],
    // '3' — top, right column, two crossbars, bottom.
    [0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0011_1110, 0b0011_1110, 0b0000_0110, 0b0111_1110, 0b0111_1110],
    // '4' — two verticals joined by the middle bar, right stem to bottom.
    [0b0110_0110, 0b0110_0110, 0b0110_0110, 0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0000_0110, 0b0000_0110],
    // '5' — top, upper-left, middle, lower-right, bottom.
    [0b0111_1110, 0b0111_1110, 0b0110_0000, 0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0111_1110, 0b0111_1110],
    // '6' — top, upper-left, middle, full lower ring, bottom.
    [0b0111_1110, 0b0111_1110, 0b0110_0000, 0b0111_1110, 0b0111_1110, 0b0110_0110, 0b0111_1110, 0b0111_1110],
    // '7' — full 2-row top bar, then a thick diagonal stem.
    [0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0000_1100, 0b0001_1000, 0b0011_0000, 0b0011_0000, 0b0011_0000],
    // '8' — full ring + middle bar.
    [0b0111_1110, 0b0111_1110, 0b0110_0110, 0b0111_1110, 0b0111_1110, 0b0110_0110, 0b0111_1110, 0b0111_1110],
    // '9' — full upper ring, middle, lower-right, bottom.
    [0b0111_1110, 0b0111_1110, 0b0110_0110, 0b0111_1110, 0b0111_1110, 0b0000_0110, 0b0111_1110, 0b0111_1110],
    // '/' — thick diagonal from bottom-left to top-right.
    [0b0000_0110, 0b0000_1100, 0b0000_1100, 0b0001_1000, 0b0001_1000, 0b0011_0000, 0b0011_0000, 0b0110_0000],
];

/// One MTSDF texel as RGBA8 from per-channel `[0,1]` distances. `.rgb` are the MSDF
/// channels the FS takes the `median` of; `.a` the single-channel SDF control (unused
/// by the FS). Cloned from `ui_text_gpu_golden::texel`.
fn texel(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(r), q(g), q(b), q(a)]
}

/// The cell index (0..NCELLS) for an ASCII char in `"0123456789/"`. Panics on any
/// other char — test-local; only the HUD string's chars are fed.
fn cell_index(c: char) -> usize {
    match c {
        '0'..='9' => (c as u8 - b'0') as usize,
        '/' => 10,
        other => panic!("hud_digit_font has no cell for {other:?}"),
    }
}

/// The font glyph SLOT for a cell index: slot 0 is the zero-advance `.notdef`, so the
/// real cells live at slots `1..=NCELLS`.
fn cell_slot(cell: usize) -> u16 {
    (cell + 1) as u16
}

/// Builds the synthetic legible-digit MTSDF [`BakedFont`] (W1). Each cell carries the
/// `DIGIT_BITS` footprint as a binary-median block: a set bit => `texel(1,1,1,1)`
/// (`median = 1`, full coverage), a clear bit => `texel(0,0,0,0)` (`median = 0`,
/// exterior). Slot 0 is a zero-advance `.notdef`; the digits `'0'..'9'` and `'/'`
/// occupy slots `1..=11` mapped contiguously across the 11 horizontal cells.
fn hud_digit_font() -> BakedFont {
    let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    let lit = texel(1.0, 1.0, 1.0, 1.0);
    // off-texels are already zero (exterior, median 0).
    for (cell, bits) in DIGIT_BITS.iter().enumerate() {
        let cell_x0 = cell as u32 * CELL;
        for (row, &row_bits) in bits.iter().enumerate() {
            let y = row as u32;
            for col in 0..CELL {
                // MSB (0x80) is column 0.
                let bit = (row_bits >> (7 - col)) & 1;
                if bit == 1 {
                    let x = cell_x0 + col;
                    let i = ((y * ATLAS_W + x) * 4) as usize;
                    pixels[i..i + 4].copy_from_slice(&lit);
                }
            }
        }
    }
    debug_assert_eq!(
        pixels.len(),
        (ATLAS_W * ATLAS_H * 4) as usize,
        "invariant: the atlas image is ATLAS_W*ATLAS_H*4 bytes"
    );

    // Glyph metrics: slot 0 == zero-advance .notdef; slots 1..=11 map to the 11 cells.
    // atlas bounds are in TEXELS, [left, bottom, right, top] (bottom == larger texel-Y).
    let mut glyphs = vec![GlyphMetrics {
        advance_em: 0.0,
        plane: [0.0; 4],
        atlas: [0.0; 4],
    }];
    for cell in 0..NCELLS as usize {
        let left = (cell as u32 * CELL) as f32;
        let right = left + CELL as f32;
        glyphs.push(GlyphMetrics {
            advance_em: 0.6,
            // plane unused by the GPU harness (it places quads directly); the emitter
            // (CPU test 6b) shapes from this — a simple full-em box keeps it legible.
            plane: [0.0, 0.0, 0.6, 0.8],
            atlas: [left, CELL as f32, right, 0.0], // [left, bottom, right, top]
        });
    }

    // cmap: '0'..'9' + '/', each to its slot. Sorted by codepoint ('/' == 0x2F < '0').
    let mut cmap = vec![MappedCodepoint {
        codepoint: '/' as u32,
        slot: cell_slot(10),
    }];
    for d in 0u32..10 {
        cmap.push(MappedCodepoint {
            codepoint: '0' as u32 + d,
            slot: cell_slot(d as usize),
        });
    }
    cmap.sort_unstable_by_key(|m| m.codepoint);

    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: ATLAS_W,
            atlas_h: ATLAS_H,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs,
        cmap,
        kern: Vec::new(),
        atlas: AtlasImage {
            width: ATLAS_W,
            height: ATLAS_H,
            pixels,
        },
    }
}

/// The atlas-cell metrics for an ASCII char in `"0123456789/"` (panics otherwise —
/// test-local). Resolves the cell -> slot -> [`GlyphMetrics`] through the same slot
/// mapping `hud_digit_font` authored.
fn glyph_for(c: char, font: &BakedFont) -> &GlyphMetrics {
    let slot = cell_slot(cell_index(c)) as usize;
    &font.glyphs[slot]
}

/// A normalized UV rect for a glyph cell `(left, top, right, bottom)` in `[0,1]`,
/// matched to `shape::quad_uv`'s ordering: atlas `[left, bottom, right, top]` texels ->
/// `(left/aw, top/ah, right/aw, bottom/ah)`. Cloned from `ui_text_gpu_golden::cell_uv`.
fn cell_uv(g: &GlyphMetrics) -> [f32; 4] {
    let aw = ATLAS_W as f32;
    let ah = ATLAS_H as f32;
    [
        g.atlas[0] / aw, // left
        g.atlas[3] / ah, // top (smaller texel-Y -> v=0)
        g.atlas[2] / aw, // right
        g.atlas[1] / ah, // bottom (larger texel-Y -> v=1)
    ]
}

/// One glyph quad at logical-px `(x, y, w, h)` sampling atlas cell `uv` with `color`
/// — the SAME `pack_ui_instance` text lane the emitter feeds. Cloned from
/// `ui_text_gpu_golden::glyph_quad`.
fn glyph_quad(x: f32, y: f32, w: f32, h: f32, color: u32, uv: [f32; 4]) -> UiInstance {
    pack_ui_instance(
        &PackInput {
            rect: [x, y, w, h],
            color,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip: None,
            text_uv: Some(uv),
        },
        1.0,
    )
}

/// Builds the per-char glyph quads for `text` at the fixed HUD pen geometry, each
/// sampling its distinct atlas cell. The W2 `FG_TEXEL` lands inside glyph 0's quad by
/// construction.
fn hud_glyph_quads(text: &str, font: &BakedFont) -> Vec<UiInstance> {
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            let x = X0 + i as f32 * GADV;
            glyph_quad(x, Y0, GW, GH, FG, cell_uv(glyph_for(c, font)))
        })
        .collect()
}

/// A bilinear sample of the atlas `pixels` at normalized `(u, v)`, returning the MTSDF
/// `median3(r,g,b)`. Reproduces the GPU's `Filter::Linear` atlas read with the same
/// pixel-centre convention (`texel i centre at (i + 0.5) / dim`) and the same RGBA8 ->
/// `[0,1]` decode, then the FS `median3`. Off-atlas taps clamp to `0.0` (exterior), which
/// matches the binary-median atlas's behaviour for these in-cell coordinates (the cells'
/// solid runs never touch the atlas border).
fn atlas_median_bilinear(pixels: &[u8], u: f32, v: f32) -> f32 {
    let aw = ATLAS_W as i32;
    let ah = ATLAS_H as i32;
    let fx = u * ATLAS_W as f32 - 0.5;
    let fy = v * ATLAS_H as f32 - 0.5;
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let chan = |xi: i32, yi: i32, c: usize| -> f32 {
        if xi < 0 || xi >= aw || yi < 0 || yi >= ah {
            return 0.0;
        }
        let i = ((yi * aw + xi) * 4) as usize + c;
        pixels[i] as f32 / 255.0
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let sample = |c: usize| {
        let a = lerp(chan(x0, y0, c), chan(x0 + 1, y0, c), tx);
        let b = lerp(chan(x0, y0 + 1, c), chan(x0 + 1, y0 + 1, c), tx);
        lerp(a, b, ty)
    };
    let (r, g, b) = (sample(0), sample(1), sample(2));
    // FS median3: max(min(r,g), min(max(r,g), b)).
    (r.min(g)).max((r.max(g)).min(b))
}

/// The FS coverage for glyph cell `cell` (a `cell_index`) at SCREEN pixel `(px, py)`,
/// reproduced exactly: pixel-centre `local_uv` over the fixed quad geometry, `lerp` into
/// the cell's UV rect, a bilinear atlas `median`, then `clamp(screen_px_range*(sd-0.5) +
/// 0.5, 0, 1)`. `screen_px_range` is the analytic `0.5 * dot(px_range/atlas_size,
/// 1/fwidth(uv))` with `fwidth(uv)` constant over a glyph quad (the affine quad->atlas
/// map): `du/dpx = (right-left)/GW`, `dv/dpy = (bottom-top)/GH`. This is the IN-WORKFLOW
/// guard for the GPU-only W2 assert — a footprint/geometry regression that drops
/// `FG_TEXEL` below full coverage fails here without a device.
fn coverage_at(font: &BakedFont, cell: usize, glyph_x0: f32, px: u32, py: u32) -> f32 {
    let uv_rect = cell_uv(&font.glyphs[cell_slot(cell) as usize]);
    let local_u = (px as f32 + 0.5 - glyph_x0) / GW;
    let local_v = (py as f32 + 0.5 - Y0) / GH;
    let u = uv_rect[0] + (uv_rect[2] - uv_rect[0]) * local_u;
    let v = uv_rect[1] + (uv_rect[3] - uv_rect[1]) * local_v;
    let sd = atlas_median_bilinear(&font.atlas.pixels, u, v);

    let px_range = font.meta.distance_range_texels;
    let unit_range_x = px_range / ATLAS_W as f32;
    let unit_range_y = px_range / ATLAS_H as f32;
    let du_dpx = (uv_rect[2] - uv_rect[0]) / GW; // == fwidth(u) over one screen px
    let dv_dpy = (uv_rect[3] - uv_rect[1]) / GH;
    let screen_tex_sz_x = 1.0 / du_dpx;
    let screen_tex_sz_y = 1.0 / dv_dpy;
    let screen_px_range =
        (0.5 * (unit_range_x * screen_tex_sz_x + unit_range_y * screen_tex_sz_y)).max(1.0);

    (screen_px_range * (sd - 0.5) + 0.5).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// No-dep BMP writer (Decision 4) — 32bpp BGRA, top-down (negative biHeight).
// ---------------------------------------------------------------------------

/// Writes `rgba` (`w*h*4` tightly-packed R8G8B8A8) as a dependency-free 32bpp BGRA BMP.
/// Top-down via a NEGATIVE `biHeight`, so the in-memory top-left texel is the image
/// top-left — NO row flip anywhere. The single channel swap (RGBA -> BGRA) is here ONLY.
fn write_bmp(path: &Path, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(
        rgba.len(),
        (w * h * 4) as usize,
        "invariant: BMP body is w*h*4 bytes"
    );
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &buf)
}

/// The screenshot output path under the workspace target dir.
fn screenshot_path() -> PathBuf {
    // CARGO_MANIFEST_DIR is .../crates/boyko_render; the workspace target is ../../target.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("screenshots")
        .join("p6b_hud.bmp")
}

// ---------------------------------------------------------------------------
// The bound HUD world (cloned from text_bind_emit.rs) — Health + the bind schedule.
// ---------------------------------------------------------------------------

/// The bindable HUD source — identical to the shipped `text_bind_emit` `Health`
/// (field 0 = current, field 1 = max). `#[repr(C)]` for stable field offsets the
/// `BindAccessor` fn-ptr trampoline reads.
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

/// Pending source mutations applied in-schedule (so the write lands on a tick the next
/// bind discovery's change window covers). Mirrors `text_bind_emit::MutQueue`.
#[derive(Resource, Default)]
struct MutQueue {
    pending: Vec<(Entity, Health)>,
}

#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<MutQueue>().pending);
    for (e, h) in pending {
        if let Some(mut g) = world.get_component_mut::<Health>(e) {
            *g = h;
        }
    }
}

struct BindWorld {
    world: EcsMaster,
    schedule: Schedule,
}

impl BindWorld {
    fn new() -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();

        let mut scratch = UiBindScratch::default();
        Health::register_bind_accessor();
        scratch.register_bound_id(Health::component_id());
        world.insert_resource(scratch);
        world.insert_resource(MutQueue::default());

        let mut builder = ScheduleBuilder::new(pool);
        let mutate = builder.add_system(mutator_system).key();
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
        let mut schedule = builder.build(&mut world);

        // Advance past Tick::ZERO before any source spawn (first-frame Added note).
        schedule.run(&mut world);
        schedule.run(&mut world);

        Self { world, schedule }
    }

    fn spawn_source(&mut self, current: f32, max: f32) -> Entity {
        self.spawn_with(move |cmds| cmds.spawn(Health { current, max }).id())
    }

    fn spawn_label(&mut self, source: Entity, template: TemplateId) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BindText {
                source,
                comp: Health::component_id(),
                field: 0,
                field2: if template == TemplateId::Ratio { 1 } else { NO_FIELD },
                template,
            });
            ec.insert(UiTextBuffer::default());
            ec.insert(UiText {
                color: FG,
                size_px: 18.0,
                font: FontId(0),
                align: TextAlign::Left,
                _pad: 0,
            });
            ec.id()
        })
    }

    fn spawn_with<F>(&mut self, f: F) -> Entity
    where
        F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
    {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        let f = Mutex::new(Some(f));
        self.world.run_system(move |mut cmds: Commands| {
            let f = f.lock().unwrap().take().expect("spawn closure runs once");
            let e = f(&mut cmds);
            *probe.lock().unwrap() = Some(e);
        });
        sink.lock().unwrap().expect("spawned handle")
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn set_health(&mut self, e: Entity, current: f32, max: f32) {
        self.world
            .resource_mut::<MutQueue>()
            .pending
            .push((e, Health { current, max }));
    }

    fn text_of(&self, e: Entity) -> String {
        self.world
            .get_component::<UiTextBuffer>(e)
            .map(|b| b.as_str().to_string())
            .unwrap_or_default()
    }

    fn ui_text_of(&self, e: Entity) -> UiText {
        *self
            .world
            .get_component::<UiText>(e)
            .expect("label has UiText style")
    }
}

// ---------------------------------------------------------------------------
// CPU invariant tests (run in-workflow; NOT ignored).
// ---------------------------------------------------------------------------

/// (6a) The bind chain formats the live `Health` value and tracks/holds it across
/// frames — the HUD's data path is correct on the CPU, no GPU.
#[test]
fn hud_binding_value() {
    let mut bw = BindWorld::new();
    let source = bw.spawn_source(75.0, 100.0);
    let label = bw.spawn_label(source, TemplateId::Ratio);

    bw.run();
    assert_eq!(bw.text_of(label), "75/100", "the bound HUD carries the live value");

    bw.set_health(source, 50.0, 100.0);
    bw.run();
    assert_eq!(bw.text_of(label), "50/100", "the HUD re-emits the changed value");

    bw.run();
    assert_eq!(bw.text_of(label), "50/100", "a still frame holds the last value");
}

/// (6b) The bound text packs into glyph instances through the REAL emitter + the GPU
/// pack lane: one glyph per visible char, the label color, strictly monotonic pen
/// advance, and each packed instance carries its cell's `text_uv` rect.
#[test]
fn hud_glyph_packing_golden() {
    let mut bw = BindWorld::new();
    let source = bw.spawn_source(75.0, 100.0);
    let label = bw.spawn_label(source, TemplateId::Ratio);
    bw.run();
    let content = bw.text_of(label);
    assert_eq!(content, HUD_STRING, "the HUD string is the bound ratio");

    let font = hud_digit_font();
    let mut fonts = FontTable::new();
    fonts.load(&font);

    let style = bw.ui_text_of(label);
    let rect = ComputedRect { x: X0, y: Y0, w: 0.0, h: 0.0 };
    let mut out: Vec<GlyphInstance> = Vec::new();
    emit_glyphs(&style, &rect, &content, StackIndex(0), None, &fonts, &mut out);

    assert_eq!(out.len(), HUD_STRING.chars().count(), "one glyph per visible char");
    assert!(out.iter().all(|g| g.color == FG), "each glyph carries the label color");
    assert!(
        out.windows(2).all(|w| w[1].rect[0] > w[0].rect[0]),
        "glyphs pen-advance left-to-right"
    );

    // The link the module header promises: the REAL emitter's shaped per-glyph UV must
    // equal the cell UV the GPU pack lane samples. This ties the proven bind/emit path to
    // the rendered quad — a divergence between the shaped UV and the GPU UV (the exact
    // "does the bound text reach the screen correctly" failure) is caught here.
    for (i, c) in content.chars().enumerate() {
        let expected = cell_uv(glyph_for(c, &font));
        let shaped = out[i].uv;
        assert!(
            shaped
                .iter()
                .zip(expected.iter())
                .all(|(a, b)| (a - b).abs() <= 1e-6),
            "char {c:?}: emitter UV {shaped:?} must match the atlas cell UV {expected:?}"
        );

        // The GPU pack lane carries that same UV verbatim into the `corner_radius` alias.
        let inst = glyph_quad(X0 + i as f32 * GADV, Y0, GW, GH, FG, expected);
        assert_eq!(inst.corner_radius, expected, "char {c:?} packs its atlas cell UV");
        assert_eq!(inst.size_px, [GW, GH], "char {c:?} packs the fixed glyph quad size");
    }
}

/// (6c) W1 guard: the six chars of `"75/100"` map to atlas cells whose UV rects make
/// the digits humanly distinguishable — distinct chars => distinct UVs; the two `'0'`s
/// share one. Proves per-digit distinctness WITHOUT a GPU (the "undifferentiated bar"
/// failure guard).
#[test]
fn hud_chars_map_to_distinct_cells() {
    let font = hud_digit_font();
    let uv_of = |c: char| cell_uv(glyph_for(c, &font));

    // The five distinct chars in "75/100": '7','5','/','1','0'.
    let distinct = ['7', '5', '/', '1', '0'];
    for i in 0..distinct.len() {
        for j in (i + 1)..distinct.len() {
            assert_ne!(
                uv_of(distinct[i]),
                uv_of(distinct[j]),
                "{:?} and {:?} must sample DISTINCT atlas cells",
                distinct[i],
                distinct[j]
            );
        }
    }

    // The two '0's in "75/100" share one cell (identical UVs).
    let zeros: Vec<[f32; 4]> = HUD_STRING.chars().filter(|&c| c == '0').map(uv_of).collect();
    assert_eq!(zeros.len(), 2, "\"75/100\" has two '0's");
    assert_eq!(zeros[0], zeros[1], "both '0's sample the same atlas cell");

    // The cells are non-degenerate (left < right) and within [0,1].
    for c in HUD_STRING.chars() {
        let uv = uv_of(c);
        assert!(uv[0] < uv[2], "cell {c:?} has positive width");
        assert!(uv.iter().all(|&v| (0.0..=1.0).contains(&v)), "cell {c:?} UV within [0,1]");
    }
}

/// (6d) The IN-WORKFLOW guard for the GPU-only W2 assert: reproduce the exact FS coverage
/// path on the CPU ([`coverage_at`]) and prove that `FG_TEXEL` is FULLY covered (so the
/// `#[ignore]`d GPU assert `at(FG_TEXEL) == FG_BYTES` cannot silently regress without a
/// device) and `BG_TEXEL` is exterior. Glyph 0 of `"75/100"` is `'7'`; its cell index is
/// `cell_index('7')`; its quad origin is `X0`.
#[test]
fn hud_fg_texel_is_fully_covered() {
    let font = hud_digit_font();
    let glyph0 = cell_index(HUD_STRING.chars().next().expect("HUD string is non-empty"));
    assert_eq!(glyph0, cell_index('7'), "glyph 0 of \"75/100\" is '7'");

    let cov_fg = coverage_at(&font, glyph0, X0, FG_TEXEL.0, FG_TEXEL.1);
    assert!(
        cov_fg >= 0.999,
        "FG_TEXEL {FG_TEXEL:?} must be a fully-covered interior of '7' (coverage = {cov_fg}); \
         the GPU W2 assert reads FG_BYTES only at coverage == 1"
    );

    // BG_TEXEL is outside every glyph quad (`y in [48, 80)`), so no cell is sampled there.
    assert!(
        BG_TEXEL.1 < Y0 as u32,
        "BG_TEXEL {BG_TEXEL:?} must be outside the glyph row so it keeps the CLEAR background"
    );

    // A texel just outside the lit bar (above the quad top edge) must NOT be covered —
    // confirms the coverage model is not trivially saturated everywhere.
    let cov_outside = coverage_at(&font, glyph0, X0, FG_TEXEL.0, (Y0 as u32).saturating_sub(2));
    assert!(
        cov_outside <= 0.001,
        "a texel above the glyph quad must be exterior (coverage = {cov_outside})"
    );
}

// ---------------------------------------------------------------------------
// REAL-MSDF variant (W3) — a baked-from-a-real-font atlas so the "/" is smooth.
//
// The synthetic atlas above is BINARY coverage (median 0 or 1 per texel): a
// diagonal stroke therefore staircases. A real MTSDF atlas baked from an outline
// font encodes a continuous signed distance, so the FS coverage anti-aliases the
// "/" diagonal and the digit curves. This variant bakes such an atlas in-process
// from a checked-in fixture font, drives the SAME bound HUD ("75/100"), shapes it
// through the SAME live emitter (`emit_glyphs`) + GPU pack lane, and writes a
// SECOND, larger, legible BMP. It shares ZERO state with the synthetic goldens
// above (different atlas, different geometry, no shared texel asserts).
// ---------------------------------------------------------------------------

/// Render em size for the MSDF HUD, logical px. Noticeably larger than the
/// synthetic 32-px glyph quad so the anti-aliased "/" diagonal is clearly visible.
const MSDF_SIZE_PX: f32 = 48.0;
/// MSDF offscreen width — wider than the synthetic 256 so "75/100" at 48 px fits
/// with margin and the smoothness reads.
const MSDF_W: u32 = 512;
/// MSDF offscreen height.
const MSDF_H: u32 = 192;
/// MSDF readback byte count (`MSDF_W * MSDF_H * 4`).
const MSDF_SIZE: u64 = (MSDF_W as u64) * (MSDF_H as u64) * 4;
/// The text pen origin (top-left of the first glyph's layout box), logical px.
/// Vertically centred-ish in the taller target so the baseline sits mid-frame.
const MSDF_X0: f32 = 24.0;
const MSDF_Y0: f32 = 64.0;

/// The fixture font baked into the MSDF atlas (clean digits + a "/"; OFL).
const MSDF_FIXTURE: &str = "Ubuntu-Light.ttf";

/// The codepoints the MSDF atlas covers — the HUD digits and the slash.
const MSDF_GLYPHS: &[char] = &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '/'];

/// Loads the checked-in `boyko_fontbake` fixture font bytes. The path is resolved
/// from THIS crate's manifest dir (`crates/boyko_render`) up to the fontbake
/// crate's `fixtures/` — the fixture lives with the baker, not this crate.
fn msdf_fixture_bytes() -> std::io::Result<Vec<u8>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("..")
        .join("boyko_fontbake")
        .join("fixtures")
        .join(MSDF_FIXTURE);
    std::fs::read(path)
}

/// Bakes a REAL MTSDF [`BakedFont`] from the fixture font over [`MSDF_GLYPHS`] via
/// the public `boyko_fontbake` bake API (`TtfFace::from_bytes` -> `bake_font`). No
/// threadpool is passed (load-time, single test glyph set); the field generation
/// runs serially.
fn msdf_baked_font() -> BakedFont {
    let bytes = msdf_fixture_bytes().expect("read the checked-in Ubuntu-Light.ttf fixture");
    let face = TtfFace::from_bytes(&bytes).expect("parse the fixture font");
    bake_font(&face, MSDF_GLYPHS, None)
}

/// Shapes `text` through the REAL emitter against the baked MSDF font, then packs
/// each shaped glyph into a GPU [`UiInstance`] via the SAME `pack_ui_instance` text
/// lane the production path uses. The emitter places each quad at the glyph's true
/// plane bounds (sized to `MSDF_SIZE_PX`) and carries its true atlas UV, so the
/// continuous-distance "/" renders smooth — no synthetic cell stretching.
fn msdf_hud_instances(text: &str, font: &BakedFont) -> Vec<UiInstance> {
    let mut fonts = FontTable::new();
    fonts.load(font);

    let style = UiText {
        color: FG,
        size_px: MSDF_SIZE_PX,
        font: FontId(0),
        align: TextAlign::Left,
        _pad: 0,
    };
    let rect = ComputedRect {
        x: MSDF_X0,
        y: MSDF_Y0,
        w: 0.0,
        h: 0.0,
    };
    let mut shaped: Vec<GlyphInstance> = Vec::new();
    emit_glyphs(&style, &rect, text, StackIndex(0), None, &fonts, &mut shaped);

    shaped
        .iter()
        .map(|g| {
            pack_ui_instance(
                &PackInput {
                    rect: g.rect,
                    color: g.color,
                    border_color: 0,
                    corner_radius: [0.0; 4],
                    border_width: [0.0; 4],
                    clip: None,
                    text_uv: Some(g.uv),
                },
                1.0,
            )
        })
        .collect()
}

/// The MSDF screenshot output path under the workspace target dir.
fn msdf_screenshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("screenshots")
        .join("p6b_hud_msdf.bmp")
}

// ---------------------------------------------------------------------------
// The GPU screenshot test (#[ignore]) — owner-run on the RTX.
// ---------------------------------------------------------------------------

#[cfg(not(miri))]
mod gpu {
    use super::*;

    use boyko_rhi::{
        BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
        ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
        RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
        StoreOp, TextureDesc, TextureDimension,
    };
    use boyko_render::{record_ui_rects, RhiContext, UiOrtho};

    use common::{assert_validation_clean, boot_or_skip};

    /// The CLEAR color as the RGBA floats `begin_rendering` takes.
    fn floats(bytes: [u8; 4]) -> [f32; 4] {
        [
            bytes[0] as f32 / 255.0,
            bytes[1] as f32 / 255.0,
            bytes[2] as f32 / 255.0,
            bytes[3] as f32 / 255.0,
        ]
    }

    /// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
    fn texel_base(x: u32, y: u32) -> usize {
        ((y * W + x) * 4) as usize
    }

    /// Renders the bound-HUD glyph row through the reused `RhiContext` UI capability +
    /// `record_ui_rects` recorder into an offscreen target, returning the readback.
    /// Cloned from `ui_text_gpu_golden::render_text_golden` with the HUD font + quads.
    fn render_hud(rhi: &mut RhiContext, instances: &[UiInstance]) -> Vec<u8> {
        let font = hud_digit_font();
        render_glyphs(rhi, instances, &font, W, H)
    }

    /// Renders `instances` against `font` into a `w * h` offscreen R8G8B8A8 target,
    /// returning the `w * h * 4`-byte readback. The single render recipe shared by the
    /// synthetic ([`render_hud`]) and the real-MSDF screenshots; the only differences
    /// between them are the atlas, the glyph geometry, and the target extent — all
    /// passed in here, so the GPU path itself is identical and proven once.
    fn render_glyphs(
        rhi: &mut RhiContext,
        instances: &[UiInstance],
        font: &BakedFont,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        let size: u64 = (w as u64) * (h as u64) * 4;

        rhi.ui_setup(
            Format::R8G8B8A8Unorm,
            boyko_render::ui_rect_vs_spirv(),
            boyko_render::ui_rect_fs_spirv(),
            4,
            font,
        )
        .expect("ui_setup (UI pipeline + atlas upload + per-FIF rings)");

        let ortho = UiOrtho::for_extent(w, h);
        // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
        // submitted against them, so slot 0 is free to host-write unfenced.
        let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
        let plan = rhi
            .ui_upload(instances, ortho, token)
            .expect("ui_upload (memcpy into the FIF ring + POD UiFramePlan)");
        debug_assert_eq!(
            plan.instance_count as usize,
            instances.len(),
            "invariant: every HUD glyph instance uploaded"
        );

        let (pipeline, bind_group) = rhi
            .ui_handles(plan.frame_index)
            .expect("ui_handles after ui_setup");

        let device = rhi.context();
        let queue = device.rhi_queue();

        let output = device
            .create_texture(&TextureDesc {
                width: w,
                height: h,
                depth: 1,
                format: Format::R8G8B8A8Unorm,
                dimension: TextureDimension::D2,
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
                array_layers: 1,
            })
            .expect("offscreen output texture");

        let staging = device
            .create_buffer(&BufferDesc {
                size,
                usage: BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("host-visible readback staging buffer");

        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("command encoder");
        let full = RenderArea {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };

        encoder.begin().expect("begin");

        encoder.image_barrier(&ImageBarrierDesc {
            texture: &output,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::ColorAttachmentOptimal,
            range: ImageSubresourceRange::COLOR,
        });

        // CLEAR pass: paint the opaque background, then close it so the UI pass opens
        // its own LoadOp::Load scope.
        let clear_attachment = [RenderingAttachment {
            texture: &output,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: floats(CLEAR_BYTES),
        }];
        encoder.begin_rendering(&RenderingDesc {
            render_area: full,
            colors: &clear_attachment,
            depth: None,
        });
        encoder.end_rendering();

        // UI pass: a fresh LoadOp::Load scope; the shared recorder records the
        // instanced glyph draw.
        let ui_attachment = [RenderingAttachment {
            texture: &output,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Load,
            store_op: StoreOp::Store,
            clear_color: [0.0; 4],
        }];
        encoder.begin_rendering(&RenderingDesc {
            render_area: full,
            colors: &ui_attachment,
            depth: None,
        });
        // SAFETY: recording is open inside a `begin_rendering(LoadOp::Load)` scope whose
        // single color attachment's format (`R8G8B8A8Unorm`) equals the UI pipeline's
        // `color_formats[0]`, at `full`; `pipeline`/`bind_group` are the live
        // current-frame (MF-7) UI handles whose ring holds `plan.instance_count` valid
        // records uploaded for `plan.frame_index` above; the pipeline declares the UI
        // bind-group layout (binding 0 SSBO, binding 1 atlas, binding 2 UBO) and a
        // 16-byte VERTEX push range.
        unsafe {
            record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group);
        }
        encoder.end_rendering();

        encoder.image_barrier(&ImageBarrierDesc {
            texture: &output,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::TRANSFER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::TRANSFER_READ,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::TransferSrcOptimal,
            range: ImageSubresourceRange::COLOR,
        });
        let regions = [BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: w,
            image_extent_h: h,
            image_extent_d: 1,
        }];
        encoder.copy_image_to_buffer(&output, ImageLayout::TransferSrcOptimal, &staging, &regions);

        encoder.end().expect("end");

        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");

        let dst_ptr = device
            .buffer_mapped_ptr(&staging)
            .expect("host-visible staging buffer is mapped");
        let mut out = vec![0u8; size as usize];
        // SAFETY: `dst_ptr` points to `size` mapped host-coherent bytes; the fence wait
        // above ordered this read after the draw + copy completed; `out` is a distinct,
        // non-overlapping allocation of `size` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), size as usize);
        }

        // Teardown the transient offscreen resources (the fence ordered their last use).
        // SAFETY: each was created on `device`, its GPU work completed (fence-waited),
        // and each is destroyed exactly once here.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
            device.destroy_buffer(staging);
            device.destroy_texture(output);
        }

        out
    }

    /// The owner-eval screenshot: boots Vulkan, renders the bound HUD, asserts the W2
    /// blend texel (fg over opaque CLEAR) + a no-bleed background texel + zero
    /// validation messages, then writes the BMP. `#[ignore]`d — Vulkan boot can hang a
    /// headless run; the orchestrator runs it on the RTX (see the module header).
    #[test]
    #[ignore = "boots Vulkan on the GPU; owner-run on the RTX (see module header)"]
    fn p6b_hud_screenshot() {
        let Some(ctx) = boot_or_skip("p6b_hud_screenshot") else {
            return;
        };
        println!("Vulkan device (validation on): {}", ctx.device_name());
        if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise -
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

        // Build the bound HUD string through the SHIPPED bind chain, then the glyph row.
        let mut bw = BindWorld::new();
        let source = bw.spawn_source(75.0, 100.0);
        let label = bw.spawn_label(source, TemplateId::Ratio);
        bw.run();
        let content = bw.text_of(label);
        assert_eq!(content, HUD_STRING, "the HUD renders the bound ratio");

        let font = hud_digit_font();
        let instances = hud_glyph_quads(&content, &font);

        let mut rhi = RhiContext::new(ctx);
        let out = render_hud(&mut rhi, &instances);
        debug_assert_eq!(out.len(), SIZE as usize, "readback is W*H*4 bytes");

        let at = |x: u32, y: u32| -> [u8; 4] {
            let b = texel_base(x, y);
            [out[b], out[b + 1], out[b + 2], out[b + 3]]
        };

        // W2: a deep interior texel of glyph '7's 2-texel-thick top bar is fg over the
        // opaque CLEAR (bilinear sd == 1 => coverage == 1; premultiply identity at A=255).
        // `hud_fg_texel_is_fully_covered` re-derives this coverage on the CPU in-workflow.
        assert_eq!(
            at(FG_TEXEL.0, FG_TEXEL.1),
            FG_BYTES,
            "W2: glyph '7' top-bar interior must be foreground: got {:02x?}",
            at(FG_TEXEL.0, FG_TEXEL.1)
        );
        // No-bleed: a texel outside every glyph quad keeps the CLEAR background.
        assert_eq!(
            at(BG_TEXEL.0, BG_TEXEL.1),
            CLEAR_BYTES,
            "no-bleed: a texel outside every glyph must keep the CLEAR background: got {:02x?}",
            at(BG_TEXEL.0, BG_TEXEL.1)
        );

        assert_validation_clean(rhi.context());

        let path = screenshot_path();
        write_bmp(&path, &out, W, H).expect("write the screenshot BMP");
        println!("P6b HUD screenshot written: {}", path.display());

        rhi.destroy_all();
        drop(rhi);
    }

    /// The REAL-MSDF screenshot (W3): bakes an MTSDF atlas in-process from the
    /// `Ubuntu-Light.ttf` fixture over `"0123456789/"`, drives the SAME bound HUD
    /// ("75/100") through the shipped bind chain, shapes it through the live
    /// `emit_glyphs` emitter against the baked font, packs each shaped glyph through
    /// the GPU text lane, renders at a legible 48 px into a 512x192 target, and
    /// writes a SECOND BMP. Unlike the synthetic variant, the atlas encodes a
    /// continuous signed distance, so the FS anti-aliases the "/" diagonal and the
    /// digit curves — the owner sees SMOOTH text. No golden texel asserts (real MSDF
    /// coverage is continuous and font-dependent); the proof is the eyeballed image +
    /// a zero-validation-message GPU run. `#[ignore]`d for the same Vulkan-boot reason.
    #[test]
    #[ignore = "boots Vulkan on the GPU; owner-run on the RTX (see module header)"]
    fn p6b_hud_screenshot_msdf() {
        let Some(ctx) = boot_or_skip("p6b_hud_screenshot_msdf") else {
            return;
        };
        println!("Vulkan device (validation on): {}", ctx.device_name());
        if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise -
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

        // Build the bound HUD string through the SHIPPED bind chain (same as synthetic).
        let mut bw = BindWorld::new();
        let source = bw.spawn_source(75.0, 100.0);
        let label = bw.spawn_label(source, TemplateId::Ratio);
        bw.run();
        let content = bw.text_of(label);
        assert_eq!(content, HUD_STRING, "the HUD renders the bound ratio");

        // Bake a REAL MTSDF atlas in-process, then shape + pack through the live path.
        let font = msdf_baked_font();
        assert_eq!(
            font.meta.kind,
            AtlasKind::Mtsdf,
            "the baked atlas is MTSDF (continuous distance => smooth '/')"
        );
        let instances = msdf_hud_instances(&content, &font);
        assert_eq!(
            instances.len(),
            content.chars().count(),
            "one packed glyph instance per visible char of the bound HUD"
        );

        let mut rhi = RhiContext::new(ctx);
        let out = render_glyphs(&mut rhi, &instances, &font, MSDF_W, MSDF_H);
        debug_assert_eq!(
            out.len(),
            MSDF_SIZE as usize,
            "readback is MSDF_W*MSDF_H*4 bytes"
        );

        assert_validation_clean(rhi.context());

        let path = msdf_screenshot_path();
        write_bmp(&path, &out, MSDF_W, MSDF_H).expect("write the MSDF screenshot BMP");
        let abs = std::fs::canonicalize(&path).unwrap_or(path);
        println!("P6b HUD MSDF screenshot written: {}", abs.display());

        rhi.destroy_all();
        drop(rhi);
    }
}
