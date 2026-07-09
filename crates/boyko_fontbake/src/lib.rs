//! # boyko_fontbake
//!
//! The load-time MTSDF font baker for `boyko-engine` GUI text (Phase P5b,
//! rungs T0–T3). This crate is a **build/setup tool**, never on the engine
//! render hot path: it ingests a font file, extracts glyph outlines and
//! metrics, generates a multi-channel signed-distance-field (MTSDF) atlas
//! entirely in-house, packs the glyphs, and serializes everything to a
//! `.bfont` binary that the runtime loads with a thin POD reader.
//!
//! ## Layout
//!
//! - [`face`] — the in-house [`FontFace`]/[`OutlineSink`] traits (T0) plus a
//!   `ttf-parser` adapter ([`face::TtfFace`]). The engine depends on the
//!   **trait**, not the backend, so a future in-house parser swaps in with no
//!   call-site churn.
//! - [`extract`] — glyph outline (line/quad/cubic, em-normalized) + per-glyph
//!   and face metrics via [`FontFace`] (T1).
//! - [`msdf`] — the in-house MSDF generator: edge coloring, per-channel signed
//!   pseudo-distance, scanline sign-correction, and the mandatory
//!   error-correction pass (T2a/T2b/T2c).
//! - [`atlas`] — skyline packing, the dense [`atlas::GlyphMetrics`] table,
//!   [`atlas::AtlasMeta`], sorted cmap/kern, and the `.bfont` writer/reader
//!   (T3).
//!
//! ## Performance posture
//!
//! Everything here runs at load time and is then discarded. Transient `Vec`
//! scratch is the documented Principle-0 load-time exception; the only durable
//! artifacts are the `.bfont` binary and the atlas image. Per-texel MSDF
//! generation is dispatched on the engine threadpool ([`boyko_threadpool`])
//! by disjoint-output row partitioning (no shared mutable state, no atomics).
//!
//! ## In-house contract
//!
//! `ttf-parser` is used **only** for outline + metrics extraction, hidden
//! behind [`FontFace`]. All distance-field generation, coloring,
//! sign-correction, error-correction, and atlas packing are in-house — that is
//! the engine's value-add.

pub mod atlas;
pub mod constants;
pub mod extract;
pub mod face;
pub mod msdf;

pub use atlas::{
    AtlasImage, AtlasKind, AtlasMeta, BakedFont, GlyphMetrics, KernPair, MappedCodepoint,
    bake_font, read_bfont, write_bfont,
};
pub use extract::{Glyph, GlyphOutline, Segment};
pub use face::{BBox, FontFace, GlyphId, OutlineSink, TtfFace};
pub use msdf::{GlyphField, generate_glyph_field};
