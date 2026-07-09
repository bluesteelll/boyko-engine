//! The `.ui` text format: an in-house, serde-free indentation grammar that
//! lowers AT RUNTIME (via `Commands`) to the byte-identical entity tree the P2
//! `ui!` macro produces, plus the inverse serializer (P3).
//!
//! # Pipeline
//!
//! 1. [`parse_ui`] — one-pass, lookahead-free indentation parse into a
//!    transient [`ParsedTree`] + a recoverable [`UiParseReport`]
//!    ([`parser`], Decision 1 / 2 / 6).
//! 2. [`spawn_ui_tree`] — runtime lowering, the exact mirror of the macro's
//!    `lower_node` ([`lower`], Decision 12).
//! 3. [`serialize_ui`] — the canonical-text inverse ([`serialize`], Decision 16).
//!
//! Dispatch is reflection-free: a closed `match` over the 10-component builtin
//! vocabulary with type-directed leaf parsers ([`dispatch`], Decision 3 / 4).
//! No serde, no reflection, no `Any` / downcast, no external crate.

pub mod ast;
pub mod components;
pub(crate) mod dispatch;
pub mod emit;
pub mod font;
pub mod lower;
pub mod measure;
pub mod parser;
pub mod report;
pub mod serialize;
pub mod shape;
pub(crate) mod split;

pub use ast::{CompKind, ParsedComponent, ParsedNode, ParsedTree, UiNameStr};
pub use components::{FontId, TextAlign, UiText};
pub use emit::{default_font, emit_glyphs, emit_node, GlyphInstance, TextEmitScratch, TextNode};
pub use font::{FontEntry, FontTable, NOTDEF_SLOT};
pub use lower::spawn_ui_tree;
pub use measure::{measure_one, ui_text_measure_system};
pub use parser::parse_ui;
pub use report::{UiParseReport, UI_FORMAT_VERSION};
pub use serialize::serialize_ui;
pub use shape::{shape_into, ShapedExtent, ShapedGlyph};

// GUI P5b re-exports the baked POD metric/meta types + the `.bfont` reader so a host
// can load a font into the [`FontTable`] without naming the bake crate directly.
pub use boyko_fontbake::atlas::{AtlasKind, AtlasMeta, BakedFont, GlyphMetrics, read_bfont};
